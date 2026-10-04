[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BaselineTestExe,
    [Parameter(Mandatory = $true)][string]$CandidateTestExe,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [int]$TimeoutSeconds = 1800,
    [int[]]$ContentMiB = @(1, 68)
)
$ErrorActionPreference = 'Stop'

function Get-MeasurementMemorySnapshot {
    $snapshot = [ordered]@{
        queryStartedAtUtc = [DateTime]::UtcNow.ToString('o')
        capturedAtUtc = $null
        source = 'Win32_OperatingSystem; KiB multiplied by 1024'
        totalVisibleMemoryBytes = $null
        freePhysicalMemoryBytes = $null
        available = $false
        error = $null
    }
    try {
        # One query only, outside the child measurement interval; never poll CIM.
        $memory = Get-CimInstance -ClassName Win32_OperatingSystem -Property TotalVisibleMemorySize, FreePhysicalMemory -ErrorAction Stop
        if ($null -eq $memory -or $null -eq $memory.TotalVisibleMemorySize -or $null -eq $memory.FreePhysicalMemory) {
            throw 'Operating-system memory counters were unavailable.'
        }
        $totalKiB = [uint64]$memory.TotalVisibleMemorySize
        $freeKiB = [uint64]$memory.FreePhysicalMemory
        if ($totalKiB -eq 0 -or $freeKiB -gt $totalKiB -or $totalKiB -gt ([uint64]::MaxValue / 1024)) {
            throw 'Operating-system memory counters were outside their valid range.'
        }
        $snapshot.totalVisibleMemoryBytes = $totalKiB * [uint64]1024
        $snapshot.freePhysicalMemoryBytes = $freeKiB * [uint64]1024
        $snapshot.available = $true
    } catch {
        # Missing telemetry stays null; it must never masquerade as zero memory.
        $snapshot.error = $_.Exception.Message
    }
    $snapshot.capturedAtUtc = [DateTime]::UtcNow.ToString('o')
    return $snapshot
}
$benchmarkName = 'tests::timer_collision_performance_tests::timer_autosave_collision_release_benchmark'
$baselinePath = (Resolve-Path -LiteralPath $BaselineTestExe).Path
$candidatePath = (Resolve-Path -LiteralPath $CandidateTestExe).Path
foreach ($executable in @($baselinePath, $candidatePath)) {
    if ($executable -match '[\\/]release_artifacts[\\/]current[\\/]' -or
        [IO.Path]::GetExtension($executable) -ne '.exe') {
        throw 'Use isolated Rust release test executables, never the installed client.'
    }
}
if ($ContentMiB | Where-Object { $_ -notin @(1, 68) }) {
    throw 'Supported synthetic collision cases are 1 MiB content and the 68 MiB history case.'
}
if ($TimeoutSeconds -lt 30) { throw 'TimeoutSeconds must be at least 30.' }
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputRoot) {
    if (@(Get-ChildItem -LiteralPath $outputRoot -Force).Count -ne 0) {
        throw 'OutputDirectory must be new or empty; prior measurement evidence is retained.'
    }
} else {
    New-Item -ItemType Directory -Path $outputRoot | Out-Null
}
$runIdentity = [Guid]::NewGuid().ToString('N')
$fixtureParent = Join-Path ([IO.Path]::GetTempPath()) ('timer_latency_synthetic_' + $runIdentity)
New-Item -ItemType Directory -Path $fixtureParent | Out-Null
$manifest = [ordered]@{
    runId = $runIdentity
    runnerSha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
    memoryTelemetry = 'One CIM snapshot before launch and one after exit; failed telemetry is null with an error; no query while child is running.'
    startedAtUtc = [DateTime]::UtcNow.ToString('o')
    testName = $benchmarkName
    profile = 'release; real autosave on isolated SQLite; synthetic production frame pump and workspace rendering; at least 100ms explicit write-lock gate; rejected first clicks and subsequent explicit retries measured separately'
    operationsPerCase = @{ start = 30; pause = 30 }
    baseline = @{ path = $baselinePath; sha256 = (Get-FileHash -LiteralPath $baselinePath -Algorithm SHA256).Hash }
    candidate = @{ path = $candidatePath; sha256 = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash }
    fixtureParent = $fixtureParent
    cases = @($ContentMiB)
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_collision_manifest.json') -Encoding UTF8
$allSamples = [Collections.Generic.List[object]]::new()
foreach ($caseMiB in $ContentMiB) {
    # Alternate order to reduce a systematic first-run cache advantage.
    $variants = if ($caseMiB -in @(1, 32)) { @('candidate', 'baseline') } else { @('baseline', 'candidate') }
    foreach ($variant in $variants) {
        $testPath = if ($variant -eq 'baseline') { $baselinePath } else { $candidatePath }
        $caseName = if ($caseMiB -eq 68) { 'history_68mib' } else { 'content_' + $caseMiB + 'mib' }
        $label = $variant + '_' + $caseName
        $fixtureRoot = Join-Path $fixtureParent $label
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $testPath
        $info.Arguments = '"' + $benchmarkName + '" --exact --ignored --nocapture --test-threads=1'
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.WorkingDirectory = [IO.Path]::GetDirectoryName($testPath)
        $info.EnvironmentVariables['GRIDTIMER_TIMER_BENCHMARK_ROOT'] = $fixtureRoot
        $info.EnvironmentVariables['GRIDTIMER_TIMER_BENCHMARK_MIB'] = [string]$caseMiB
        $info.EnvironmentVariables['GRIDTIMER_TIMER_BENCHMARK_VARIANT'] = $variant
        $info.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $fixtureRoot 'local_app_data'
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $info
        Write-Output ('Starting ' + $label + ', 30 starts and 30 pauses')
        $memoryBeforeStart = Get-MeasurementMemorySnapshot
        if (-not $process.Start()) { throw ('Unable to launch ' + $label) }
        $ownedPid = $process.Id
        $ownedStart = $process.StartTime.ToUniversalTime()
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $nextProgress = 15
        $timedOut = $false
        while (-not $process.WaitForExit(250)) {
            if ($clock.Elapsed.TotalSeconds -ge $nextProgress) {
                Write-Output ($label + ' elapsed ' + [int]$clock.Elapsed.TotalSeconds + ' s')
                $nextProgress += 15
            }
            if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                # Only the exact process created above can be stopped on timeout.
                $process.Refresh()
                if ($process.Id -ne $ownedPid -or $process.StartTime.ToUniversalTime() -ne $ownedStart -or
                    $process.MainModule.FileName -ne $testPath) {
                    throw 'Owned benchmark process identity changed; no process was stopped.'
                }
                $process.Kill()
                $process.WaitForExit()
                $timedOut = $true
                break
            }
        }
        $clock.Stop()
        $memoryAfterExit = Get-MeasurementMemorySnapshot
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $stdout | Set-Content -LiteralPath (Join-Path $outputRoot ($label + '.stdout.log')) -Encoding UTF8
        $stderr | Set-Content -LiteralPath (Join-Path $outputRoot ($label + '.stderr.log')) -Encoding UTF8
        $result = [ordered]@{ variant = $variant; case = $caseName; exitCode = $process.ExitCode; elapsedSeconds = $clock.Elapsed.TotalSeconds; timedOut = $timedOut; memoryBeforeStart = $memoryBeforeStart; memoryAfterExit = $memoryAfterExit }
        $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot ($label + '.process.json')) -Encoding UTF8
        if ($timedOut -or $process.ExitCode -ne 0) { throw ('Benchmark failed: ' + $label + '; evidence and isolated fixture retained.') }
        $samples = @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_COLLISION_SAMPLE ' } | ForEach-Object {
            ($_ -replace '^TIMER_COLLISION_SAMPLE ', '') | ConvertFrom-Json
        })
        if ($stdout -notmatch '(?m)^TIMER_COLLISION_REOPEN_VERIFIED sessions=30 running=0 signedOut=true') {
            throw ('Persisted reopen verification is missing: ' + $label)
        }
        if ($samples.Count -ne 60) { throw ('Expected exactly 60 samples for ' + $label + ', got ' + $samples.Count) }
        foreach ($sample in $samples) {
            if ($sample.variant -ne $variant -or $sample.caseMiB -ne $caseMiB -or
                $sample.firstDispatchAccepted -isnot [bool]) {
                throw ('Invalid collision sample identity or accepted flag: ' + $label)
            }
            $expectedDispatchCount = if ($sample.firstDispatchAccepted) { 1 } else { 2 }
            if ($sample.dispatchCount -ne $expectedDispatchCount -or
                (($null -ne $sample.retryDispatchMicros) -eq $sample.firstDispatchAccepted)) {
                throw ('Rejected click and retry accounting disagree: ' + $label)
            }
            $allSamples.Add($sample)
        }
        $allSamples | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_collision_samples.json') -Encoding UTF8
        Write-Output ('Completed ' + $label)
        $process.Dispose()
    }
}
$metrics = @(
    'firstDispatchReturnMicros', 'firstFrameMicros', 'predecessorGateReleasedMicros',
    'predecessorObservedFrameMicros', 'retryDispatchMicros', 'retryFirstFrameMicros',
    'firstClickCommittedMicros', 'retryClickCommittedMicros',
    'initialClickThroughExplicitRetryMicros', 'acceptedMicros', 'committedFrameWorkMicros',
    'acceptedToSubmittedMicros', 'workerQueueMicros', 'workerTransformMicros',
    'durableSaveMicros', 'workerPostprocessMicros', 'receiptDeliveryAndApplyMicros',
    'acceptedClickSaveCompletedMicros', 'acceptedClickReceiptAppliedMicros'
)
$summary = foreach ($group in ($allSamples | Group-Object -Property variant, caseMiB, action)) {
    $item = $group.Group[0]
    $accepted = @($group.Group | Where-Object { $_.firstDispatchAccepted }).Count
    $retried = @($group.Group | Where-Object { $null -ne $_.retryDispatchMicros }).Count
    $entry = [ordered]@{
        variant = $item.variant; caseMiB = $item.caseMiB; action = $item.action
        samples = $group.Count; firstAcceptedCount = $accepted
        firstRejectedCount = $group.Count - $accepted; explicitRetryCount = $retried
        countBasis = 'Observed firstDispatchAccepted and retry fields; no acceptance outcome inferred from executable label.'
        queueTimingBasis = 'acceptedToSubmitted includes the remaining predecessor lock/save wait for queued intentions; fixed lock gate is reported separately.'
    }
    if ($group.Count -ne 30) { throw ('Expected 30 samples in group ' + $group.Name) }
    if (@($group.Group | Select-Object -ExpandProperty sample -Unique).Count -ne 30) {
        throw ('Duplicate sample ordinal in group ' + $group.Name)
    }
    foreach ($metric in $metrics) {
        $values = @($group.Group | Where-Object { $null -ne $_.$metric } | ForEach-Object { [double]$_.$metric } | Sort-Object)
        $entry[$metric] = if ($values.Count -eq 0) {
            @{ observationCount = 0; p50 = $null; p95 = $null; maximum = $null }
        } else {
            @{
                observationCount = $values.Count
                p50 = $values[[Math]::Ceiling($values.Count * 0.50) - 1]
                p95 = $values[[Math]::Ceiling($values.Count * 0.95) - 1]
                maximum = $values[-1]
            }
        }
    }
    $entry
}
@($summary) | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_collision_summary.json') -Encoding UTF8
$manifest.completedAtUtc = [DateTime]::UtcNow.ToString('o')
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_collision_manifest.json') -Encoding UTF8
Write-Output ('Timer collision evidence: ' + $outputRoot)
