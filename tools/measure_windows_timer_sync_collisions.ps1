[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BaselineTestExe,
    [Parameter(Mandatory = $true)][string]$CandidateTestExe,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [Parameter(Mandatory = $true)][string]$SourceProvenance,
    [Parameter(Mandatory = $true)][string]$BuildProvenance,
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
$metrics = @(
    'firstDispatchReturnMicros', 'handlerEnteredMicros', 'firstFrameMicros', 'syncObservedFrameMicros', 'syncSettledMicros',
    'firstClickCommittedMicros', 'acceptedMicros', 'committedFrameWorkMicros', 'peakFrameMicros',
    'acceptedToSubmittedMicros', 'workerQueueMicros', 'workerTransformMicros', 'durableSaveMicros',
    'workerPostprocessMicros', 'receiptDeliveryAndApplyMicros',
    'acceptedClickSaveCompletedMicros', 'acceptedClickReceiptAppliedMicros'
)
$benchmarkName = 'tests::timer_sync_collision_performance_tests::timer_sync_receipt_collision_release_benchmark'
$baselinePath = (Resolve-Path -LiteralPath $BaselineTestExe).Path
$candidatePath = (Resolve-Path -LiteralPath $CandidateTestExe).Path
foreach ($executable in @($baselinePath, $candidatePath)) {
    if ($executable -match '[\\/]release_artifacts[\\/]current[\\/]' -or
        [IO.Path]::GetExtension($executable) -ne '.exe') {
        throw 'Use isolated Rust release test executables, never the installed client.'
    }
}
if (-not $ContentMiB.Count -or @($ContentMiB | Select-Object -Unique).Count -ne $ContentMiB.Count -or
    ($ContentMiB | Where-Object { $_ -notin @(1, 68) })) {
    throw 'Supported synthetic collision cases are 1 MiB content and the 68 MiB history case.'
}
if ($TimeoutSeconds -lt 30) { throw 'TimeoutSeconds must be at least 30.' }
$sourceProvenancePath = (Resolve-Path -LiteralPath $SourceProvenance).Path
$buildProvenancePath = (Resolve-Path -LiteralPath $BuildProvenance).Path
$python = (Get-Command python -CommandType Application).Source
$validation = & $python -B (Join-Path $PSScriptRoot 'record_windows_timer_sync_builds.py') --verify-build $buildProvenancePath
if ($LASTEXITCODE -ne 0) { throw 'Supplemental build/source provenance failed verification.' }
$verified = ($validation -join "`n") | ConvertFrom-Json
if ($verified.verified -ne $true) { throw 'Supplemental build/source provenance was not verified.' }
$sourceProof = Get-Content -LiteralPath $sourceProvenancePath -Raw | ConvertFrom-Json
$buildProof = Get-Content -LiteralPath $buildProvenancePath -Raw | ConvertFrom-Json
$sourceProofSha = (Get-FileHash -LiteralPath $sourceProvenancePath -Algorithm SHA256).Hash
$buildProofSha = (Get-FileHash -LiteralPath $buildProvenancePath -Algorithm SHA256).Hash
if ($sourceProof.exactTest -ne $benchmarkName -or $buildProof.exactTest -ne $benchmarkName -or
    [IO.Path]::GetFullPath($buildProof.sourceProvenance.path) -ne $sourceProvenancePath -or
    $buildProof.sourceProvenance.sha256 -ne $sourceProofSha -or $verified.buildProvenanceSha256 -ne $buildProofSha) {
    throw 'Supplemental source and build proofs are not the same input set.'
}
foreach ($variant in @('baseline', 'candidate')) {
    $executable = if ($variant -eq 'baseline') { $baselinePath } else { $candidatePath }
    if ([IO.Path]::GetFullPath($buildProof.binaries.$variant.path) -ne $executable -or
        (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash -ne $buildProof.binaries.$variant.sha256) {
        throw ('Supplemental executable is not bound to the source proof: ' + $variant)
    }
}
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
    profile = 'release supplemental; real synthetic SyncTaskResult queued before programmatic click; click starts before full production frame pump; all new network dispatch disabled only by cfg(test) instrumentation; no OS mouse or native GPU measurement'
    operationsPerCase = @{ start = 30; pause = 30 }
    baseline = @{ path = $baselinePath; sha256 = (Get-FileHash -LiteralPath $baselinePath -Algorithm SHA256).Hash }
    candidate = @{ path = $candidatePath; sha256 = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash }
    supplemental = @{
        sourceProvenance = @{ path = $sourceProvenancePath; sha256 = $sourceProofSha }
        buildProvenance = @{ path = $buildProvenancePath; sha256 = $buildProofSha }
        productionAndIsolatedSourceIdentities = $buildProof.sources
        sharedProbeSha256 = $buildProof.sharedProbeSha256; gateSha256 = $buildProof.gateSha256
        distinction = 'Independent supplemental binaries; never pooled with core/native/autosave binaries. Production source identity is established through the source and patch proof.'
    }
    fixtureParent = $fixtureParent
    cases = @($ContentMiB)
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_sync_collision_manifest.json') -Encoding UTF8
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
        $result = [ordered]@{ variant = $variant; case = $caseName; executable = $testPath; executableSha256 = $buildProof.binaries.$variant.sha256; pid = $ownedPid; startedAtUtc = $ownedStart.ToString('o'); completedAtUtc = [DateTime]::UtcNow.ToString('o'); exitCode = $process.ExitCode; elapsedSeconds = $clock.Elapsed.TotalSeconds; timedOut = $timedOut; memoryBeforeStart = $memoryBeforeStart; memoryAfterExit = $memoryAfterExit }
        $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot ($label + '.process.json')) -Encoding UTF8
        if ($timedOut -or $process.ExitCode -ne 0) { throw ('Benchmark failed: ' + $label + '; evidence and isolated fixture retained.') }
        $samples = @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_SYNC_COLLISION_SAMPLE ' } | ForEach-Object {
            ($_ -replace '^TIMER_SYNC_COLLISION_SAMPLE ', '') | ConvertFrom-Json
        })
        $fixtures = @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_SYNC_COLLISION_FIXTURE ' } | ForEach-Object { ($_ -replace '^TIMER_SYNC_COLLISION_FIXTURE ', '') | ConvertFrom-Json })
        $reopened = @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_SYNC_COLLISION_REOPEN_VERIFIED ' } | ForEach-Object { ($_ -replace '^TIMER_SYNC_COLLISION_REOPEN_VERIFIED ', '') | ConvertFrom-Json })
        if ($fixtures.Count -ne 1 -or $fixtures[0].variant -ne $variant -or $fixtures[0].caseMiB -ne $caseMiB -or
            $fixtures[0].syntheticOnly -ne $true -or $fixtures[0].boundSyntheticAccount -ne $true -or $fixtures[0].networkDispatchBlocked -ne $true -or
            $reopened.Count -ne 1 -or $reopened[0].sessions -ne 30 -or $reopened[0].running -ne 0 -or $reopened[0].boundSyntheticAccount -ne $true -or
            $reopened[0].blockedMediaDispatches -ne 60 -or $reopened[0].blockedRevocationDispatches -ne 0) {
            throw ('Supplemental fixture/reopen verification is missing: ' + $label)
        }
        if ($samples.Count -ne 60) { throw ('Expected exactly 60 samples for ' + $label + ', got ' + $samples.Count) }
        foreach ($field in @('stateBytes', 'journalBytes')) { Assert-RequiredFiniteMetric $fixtures[0] $field }
        foreach ($field in @('blockedSyncDispatches', 'blockedMediaDispatches', 'blockedLegalDispatches', 'blockedRevocationDispatches')) {
            Assert-RequiredFiniteMetric $reopened[0] $field
        }
        foreach ($sample in $samples) {
            if ($sample.variant -ne $variant -or $sample.caseMiB -ne $caseMiB -or $sample.action -notin @('start', 'pause') -or
                $sample.firstDispatchAccepted -isnot [bool] -or -not $sample.firstDispatchAccepted -or $sample.dispatchCount -ne 1 -or
                $sample.firstPendingFeedbackVisible -isnot [bool] -or $sample.queuedBehindSync -isnot [bool] -or
                $sample.syncMergedTitlePreserved -isnot [bool] -or -not $sample.syncMergedTitlePreserved -or
                $sample.finalRunningStateVerified -isnot [bool] -or -not $sample.finalRunningStateVerified) {
                throw ('Invalid sync-receipt collision identity, one-click acceptance or saved-state assertions: ' + $label)
            }
            foreach ($metric in ($metrics + @('sample', 'caseMiB', 'dispatchCount'))) { Assert-RequiredFiniteMetric $sample $metric }
            $allSamples.Add($sample)
        }
        $allSamples | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_sync_collision_samples.json') -Encoding UTF8
        Write-Output ('Completed ' + $label)
        $process.Dispose()
    }
}
$summary = foreach ($group in ($allSamples | Group-Object -Property variant, caseMiB, action)) {
    $item = $group.Group[0]
    $accepted = @($group.Group | Where-Object { $_.firstDispatchAccepted }).Count
    $queued = @($group.Group | Where-Object { $_.queuedBehindSync }).Count
    $visible = @($group.Group | Where-Object { $_.firstPendingFeedbackVisible }).Count
    $entry = [ordered]@{
        variant = $item.variant; caseMiB = $item.caseMiB; action = $item.action
        samples = $group.Count; firstAcceptedCount = $accepted
        firstRejectedCount = $group.Count - $accepted; explicitRetryCount = 0
        queuedBehindSyncCount = $queued; firstPendingFeedbackVisibleCount = $visible
        countBasis = 'Observed firstDispatchAccepted; exactly one dispatch per action; no retry.'
        queueTimingBasis = 'Click begins before production pump. handlerEntered includes synchronous pump work. syncObservedFrame is the first observed remote-title frame, not a dedicated durable receipt timestamp.'
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
@($summary) | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_sync_collision_summary.json') -Encoding UTF8
$manifest.completedAtUtc = [DateTime]::UtcNow.ToString('o')
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_sync_collision_manifest.json') -Encoding UTF8
Write-Output ('Supplemental sync-receipt collision evidence: ' + $outputRoot)
