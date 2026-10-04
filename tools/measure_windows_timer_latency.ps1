[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BaselineTestExe,
    [Parameter(Mandatory = $true)][string]$CandidateTestExe,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [int]$TimeoutSeconds = 1800,
    [switch]$Native,
    [int[]]$ContentMiB = @(0, 1, 8, 32, 68)
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

function Assert-RequiredFiniteMetric($Sample, [string]$Metric) {
    $property = $Sample.PSObject.Properties[$Metric]
    if ($null -eq $property -or $null -eq $property.Value) {
        throw ('Required timer metric is missing: ' + $Metric)
    }
    $value = $property.Value
    $typeCode = [Type]::GetTypeCode($value.GetType())
    if ($typeCode -notin @([TypeCode]::Byte, [TypeCode]::SByte, [TypeCode]::Int16, [TypeCode]::UInt16,
        [TypeCode]::Int32, [TypeCode]::UInt32, [TypeCode]::Int64, [TypeCode]::UInt64,
        [TypeCode]::Single, [TypeCode]::Double, [TypeCode]::Decimal)) {
        throw ('Required timer metric is not a number: ' + $Metric)
    }
    if ($value -lt 0 -or
        ($value -is [double] -and ([double]::IsNaN($value) -or [double]::IsInfinity($value))) -or
        ($value -is [single] -and ([single]::IsNaN($value) -or [single]::IsInfinity($value)))) {
        throw ('Required timer metric must be finite and nonnegative: ' + $Metric)
    }
}
$benchmarkName = if ($Native) { 'tests::timer_native_latency_tests::timer_native_window_end_to_end_benchmark' } else { 'tests::timer_action_performance_tests::timer_action_end_to_end_release_benchmark' }
if ($Native) { $ContentMiB = @(68) }
$baselinePath = (Resolve-Path -LiteralPath $BaselineTestExe).Path
$candidatePath = (Resolve-Path -LiteralPath $CandidateTestExe).Path
foreach ($executable in @($baselinePath, $candidatePath)) {
    if ($executable -match '[\\/]release_artifacts[\\/]current[\\/]' -or
        [IO.Path]::GetExtension($executable) -ne '.exe') {
        throw 'Use isolated Rust release test executables, never the installed client.'
    }
}
if ($ContentMiB | Where-Object { $_ -notin @(0, 1, 8, 32, 68) }) {
    throw 'Supported synthetic cases are 0, 1, 8, 32, and 68 MiB (history case).'
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
    profile = if ($Native) { 'release; production eframe update; native GPU viewport; programmatic dispatch; screenshot callback upper bounds; no OS mouse or monitor scanout measurement' } else { 'release; synthetic; production frame pump and workspace rendering; excludes native input delivery and display presentation' }
    operationsPerCase = @{ start = 30; pause = 30 }
    baseline = @{ path = $baselinePath; sha256 = (Get-FileHash -LiteralPath $baselinePath -Algorithm SHA256).Hash }
    candidate = @{ path = $candidatePath; sha256 = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash }
    fixtureParent = $fixtureParent
    cases = @($ContentMiB)
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_latency_manifest.json') -Encoding UTF8
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
        $nativeOutput = Join-Path $outputRoot ($label + '_native')
        if ($Native) { $info.EnvironmentVariables['GRIDTIMER_TIMER_NATIVE_OUTPUT'] = $nativeOutput }
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
        $samples = if ($Native) {
            $completion = Get-Content -LiteralPath (Join-Path $nativeOutput 'native_timer_completion.json') -Raw | ConvertFrom-Json
            $reopened = Get-Content -LiteralPath (Join-Path $nativeOutput 'native_timer_reopen.json') -Raw | ConvertFrom-Json
            if (-not $completion.completed -or -not $reopened.verified) { throw ('Native frame or reopen verification failed: ' + $label) }
            @(Get-Content -LiteralPath (Join-Path $nativeOutput 'native_timer_samples.json') -Raw | ConvertFrom-Json)
        } else {
            @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_LATENCY_SAMPLE ' } | ForEach-Object {
                ($_ -replace '^TIMER_LATENCY_SAMPLE ', '') | ConvertFrom-Json
            })
        }
        if ($samples.Count -ne 60) { throw ('Expected exactly 60 samples for ' + $label + ', got ' + $samples.Count) }
        foreach ($sample in $samples) { $allSamples.Add($sample) }
        $allSamples | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_latency_samples.json') -Encoding UTF8
        Write-Output ('Completed ' + $label)
        $process.Dispose()
    }
}
$metrics = if ($Native) { @('dispatchReturnMicros', 'acceptedMicros', 'feedbackUpdateReturnMicros', 'feedbackRenderedUpperBoundMicros', 'saveCompletedMicros', 'receiptAppliedMicros', 'confirmedUpdateReturnMicros', 'confirmedRenderedUpperBoundMicros', 'confirmedFrameWorkMicros') } else { @('dispatchReturnMicros', 'acceptedMicros', 'feedbackFrameMicros', 'saveCompletedMicros', 'receiptAppliedMicros', 'committedFrameMicros', 'committedFrameWorkMicros', 'peakFrameMicros') }
$metrics += @('submittedMicros', 'workerStartedMicros', 'transformCompletedMicros', 'receiptReadyMicros', 'prepareMainThreadMicros', 'workerQueueMicros', 'workerTransformMicros', 'durableSaveMicros', 'workerPostprocessMicros', 'receiptDeliveryAndApplyMicros')
# Validate all required fields before any numeric coercion can turn null into zero.
foreach ($sample in $allSamples) {
    foreach ($metric in ($metrics + @('sample', 'actionId', 'caseMiB', 'stateBytes', 'journalBytes'))) {
        Assert-RequiredFiniteMetric $sample $metric
    }
}
$summary = foreach ($group in ($allSamples | Group-Object -Property variant, caseMiB, action)) {
    $item = $group.Group[0]
    $entry = [ordered]@{ variant = $item.variant; caseMiB = $item.caseMiB; action = $item.action; samples = $group.Count }
    if ($group.Count -ne 30) { throw ('Expected 30 samples in group ' + $group.Name) }
    foreach ($metric in $metrics) {
        $values = @($group.Group | ForEach-Object { [double]$_.$metric } | Sort-Object)
        $entry[$metric] = @{
            p50 = $values[[Math]::Ceiling($values.Count * 0.50) - 1]
            p95 = $values[[Math]::Ceiling($values.Count * 0.95) - 1]
            maximum = $values[-1]
        }
    }
    $entry
}
@($summary) | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_latency_summary.json') -Encoding UTF8
$manifest.completedAtUtc = [DateTime]::UtcNow.ToString('o')
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_latency_manifest.json') -Encoding UTF8
Write-Output ('Timer latency evidence: ' + $outputRoot)
