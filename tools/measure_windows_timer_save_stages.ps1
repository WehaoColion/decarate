[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$CandidateTestExe,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [int]$TimeoutSeconds = 1800,
    [ValidateSet(0, 1, 8, 32, 68)][int]$ContentMiB = 68,
    [switch]$RequireControlledStages
)
$ErrorActionPreference = 'Stop'

function Get-MeasurementMemorySnapshot {
    $snapshot = [ordered]@{
        queryStartedAtUtc = [DateTime]::UtcNow.ToString('o'); capturedAtUtc = $null
        source = 'Win32_OperatingSystem; KiB multiplied by 1024'
        totalVisibleMemoryBytes = $null; freePhysicalMemoryBytes = $null
        available = $false; error = $null
    }
    try {
        # Exactly once before launch and once after exit, never inside the run.
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
    } catch { $snapshot.error = $_.Exception.Message }
    $snapshot.capturedAtUtc = [DateTime]::UtcNow.ToString('o')
    return $snapshot
}

function Assert-Metric($Object, [string]$Name) {
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name] -or $null -eq $Object.$Name) {
        throw ('Missing diagnostic metric: ' + $Name)
    }
    $value = $Object.$Name
    $type = [Type]::GetTypeCode($value.GetType())
    if ($type -notin @([TypeCode]::Byte, [TypeCode]::SByte, [TypeCode]::Int16, [TypeCode]::UInt16,
        [TypeCode]::Int32, [TypeCode]::UInt32, [TypeCode]::Int64, [TypeCode]::UInt64,
        [TypeCode]::Single, [TypeCode]::Double, [TypeCode]::Decimal) -or
        [double]$value -lt 0 -or [double]::IsNaN([double]$value) -or [double]::IsInfinity([double]$value)) {
        throw ('Diagnostic metric must be finite and nonnegative: ' + $Name)
    }
}

function Get-Percentiles($Values) {
    $ordered = @($Values | Sort-Object)
    if ($ordered.Count -ne 30) { throw 'Each diagnostic action requires exactly 30 samples.' }
    return [ordered]@{ p50 = $ordered[14]; p95 = $ordered[28]; maximum = $ordered[29] }
}

$benchmarkName = 'tests::timer_action_performance_tests::timer_action_end_to_end_release_benchmark'
$testPath = (Resolve-Path -LiteralPath $CandidateTestExe).Path
if ($testPath -match '[\\/]release_artifacts[\\/]current[\\/]' -or [IO.Path]::GetExtension($testPath) -ne '.exe') {
    throw 'Use an isolated candidate release TEST executable, never the installed client.'
}
if ($TimeoutSeconds -lt 30) { throw 'TimeoutSeconds must be at least 30.' }
$executableSha = (Get-FileHash -LiteralPath $testPath -Algorithm SHA256).Hash
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputRoot) {
    if (@(Get-ChildItem -LiteralPath $outputRoot -Force).Count -ne 0) {
        throw 'OutputDirectory must be new or empty; previous evidence is retained.'
    }
} else { New-Item -ItemType Directory -Path $outputRoot | Out-Null }
$runIdentity = [Guid]::NewGuid().ToString('N')
$fixtureParent = Join-Path ([IO.Path]::GetTempPath()) ('timer_latency_synthetic_' + $runIdentity)
New-Item -ItemType Directory -Path $fixtureParent | Out-Null
$label = 'candidate_save_stages_' + $ContentMiB + 'mib'
$fixtureRoot = Join-Path $fixtureParent $label
$manifestPath = Join-Path $outputRoot 'timer_save_stages_manifest.json'
$manifest = [ordered]@{
    formatVersion = 1; runId = $runIdentity; diagnosticOnly = $true; hasBaseline = $false
    profile = 'Candidate-only synthetic full production frame pump; diagnostic save stages; not comparative acceptance or OS input/display measurement'
    startedAtUtc = [DateTime]::UtcNow.ToString('o'); testName = $benchmarkName
    runnerSha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
    executable = @{ path = $testPath; sha256 = $executableSha }
    caseMiB = $ContentMiB; operations = @{ start = 30; pause = 30 }
    requireControlledSessionStages = [bool]$RequireControlledStages
    fixtureParent = $fixtureParent; fixtureRoot = $fixtureRoot
    memoryTelemetry = 'One CIM query before launch and one after exit; no query while the test is running; failures remain null with an error.'
    stageInterpretation = 'Five disjoint outer stages are bounded by totalMicros; three mirror subtotals are nested. Optional fourteen controlled-session subtotals are mutually disjoint and bounded by controlledSessionMicros. Never add nested subtotals to their parents. Durations are integer microseconds.'
    cleanup = 'This runner never deletes fixtures. The existing successful core probe owns its own fixture cleanup; failure fixtures are retained.'
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
$info = [Diagnostics.ProcessStartInfo]::new()
$info.FileName = $testPath
$info.Arguments = '"' + $benchmarkName + '" --exact --ignored --nocapture --test-threads=1'
$info.UseShellExecute = $false; $info.CreateNoWindow = $true
$info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
$info.WorkingDirectory = [IO.Path]::GetDirectoryName($testPath)
$info.EnvironmentVariables['GRIDTIMER_TIMER_BENCHMARK_ROOT'] = $fixtureRoot
$info.EnvironmentVariables['GRIDTIMER_TIMER_BENCHMARK_MIB'] = [string]$ContentMiB
$info.EnvironmentVariables['GRIDTIMER_TIMER_BENCHMARK_VARIANT'] = 'candidate'
$info.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $fixtureRoot 'local_app_data'
$process = [Diagnostics.Process]::new(); $process.StartInfo = $info
Write-Output ('Starting candidate-only save-stage diagnostic, case ' + $ContentMiB + ', 30 starts and 30 pauses')
$memoryBeforeStart = Get-MeasurementMemorySnapshot
if (-not $process.Start()) { throw 'Unable to start candidate diagnostic.' }
$ownedPid = $process.Id; $ownedStart = $process.StartTime.ToUniversalTime()
$stdoutTask = $process.StandardOutput.ReadToEndAsync(); $stderrTask = $process.StandardError.ReadToEndAsync()
$clock = [Diagnostics.Stopwatch]::StartNew(); $nextProgress = 15; $timedOut = $false
while (-not $process.WaitForExit(250)) {
    if ($clock.Elapsed.TotalSeconds -ge $nextProgress) {
        Write-Output ($label + ' elapsed ' + [int]$clock.Elapsed.TotalSeconds + ' s')
        $nextProgress += 15
    }
    if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
        $process.Refresh()
        if ($process.Id -ne $ownedPid -or $process.StartTime.ToUniversalTime() -ne $ownedStart -or $process.MainModule.FileName -ne $testPath) {
            throw 'Owned diagnostic process identity changed; no process was stopped.'
        }
        $process.Kill(); $process.WaitForExit(); $timedOut = $true
        break
    }
}
$clock.Stop()
$memoryAfterExit = Get-MeasurementMemorySnapshot
$stdout = $stdoutTask.GetAwaiter().GetResult(); $stderr = $stderrTask.GetAwaiter().GetResult()
$stdoutPath = Join-Path $outputRoot ($label + '.stdout.log')
$stderrPath = Join-Path $outputRoot ($label + '.stderr.log')
$stdout | Set-Content -LiteralPath $stdoutPath -Encoding UTF8
$stderr | Set-Content -LiteralPath $stderrPath -Encoding UTF8
$result = [ordered]@{
    diagnosticOnly = $true; variant = 'candidate'; caseMiB = $ContentMiB
    executable = $testPath; executableSha256 = $executableSha; pid = $ownedPid
    startedAtUtc = $ownedStart.ToString('o'); completedAtUtc = [DateTime]::UtcNow.ToString('o')
    exitCode = $process.ExitCode; elapsedSeconds = $clock.Elapsed.TotalSeconds; timedOut = $timedOut
    memoryBeforeStart = $memoryBeforeStart; memoryAfterExit = $memoryAfterExit
    stdoutSha256 = (Get-FileHash -LiteralPath $stdoutPath -Algorithm SHA256).Hash
    stderrSha256 = (Get-FileHash -LiteralPath $stderrPath -Algorithm SHA256).Hash
}
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputRoot ($label + '.process.json')) -Encoding UTF8
$exitCode = $process.ExitCode
$process.Dispose()
if ($timedOut -or $exitCode -ne 0) { throw 'Candidate diagnostic failed; logs and remaining isolated fixture are retained.' }
if ((Get-FileHash -LiteralPath $testPath -Algorithm SHA256).Hash -ne $executableSha) { throw 'Candidate executable changed during measurement.' }
$samples = @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_LATENCY_SAMPLE ' } | ForEach-Object {
    ($_ -replace '^TIMER_LATENCY_SAMPLE ', '') | ConvertFrom-Json
})
# Persist complete raw records before validation; invalid diagnostic evidence is still evidence.
ConvertTo-Json -InputObject $samples -Depth 15 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_save_stages_samples.json') -Encoding UTF8
$fixtures = @($stdout -split "`r?`n" | Where-Object { $_ -match '^TIMER_LATENCY_FIXTURE ' } | ForEach-Object {
    ($_ -replace '^TIMER_LATENCY_FIXTURE ', '') | ConvertFrom-Json
})
if ($samples.Count -ne 60 -or $fixtures.Count -ne 1 -or $fixtures[0].variant -ne 'candidate' -or
    $fixtures[0].caseMiB -ne $ContentMiB -or $fixtures[0].syntheticOnly -ne $true -or $fixtures[0].signedOut -ne $true) {
    throw 'Diagnostic requires exactly 60 samples and one signed-out synthetic candidate fixture.'
}
$stageNames = @('controlledSessionMicros', 'evidenceRefreshMicros', 'privacyMirrorsMicros', 'readableMirrorMicros',
    'privacyFinishMicros', 'totalMicros', 'mirrorPrepareMicros', 'mirrorEvidenceMicros', 'mirrorFileMicros')
$controlledStageNames = @(
    'controlledOpenAndFullIntegrityMicros', 'controlledSchemaForeignKeysAndOwnersMicros',
    'controlledHistoryAuditMicros', 'controlledIndependentEvidenceMicros', 'controlledParentSelectionMicros',
    'controlledProtectedSyncMicros', 'controlledPrivacyPrepareMicros', 'controlledIncomingAnalysisMicros',
    'controlledDestructiveDropAndPrivacyCommitMicros', 'controlledInsertReadbackVerifyMicros',
    'controlledHistoryPruneMicros', 'controlledParentMirrorRecheckMicros',
    'controlledSqliteCommitMicros', 'controlledConnectionCloseMicros'
)
$hasControlledStages = $null -ne $samples[0].saveStagesMicros -and
    $null -ne $samples[0].saveStagesMicros.PSObject.Properties[$controlledStageNames[0]]
if ($RequireControlledStages -and -not $hasControlledStages) {
    throw 'This diagnostic requires the fourteen controlled-session stages; the executable only emitted the legacy outer stages.'
}
if ($hasControlledStages) { $stageNames += $controlledStageNames }
$orderedTimes = @('acceptedMicros', 'submittedMicros', 'workerStartedMicros', 'transformCompletedMicros',
    'saveCompletedMicros', 'receiptReadyMicros', 'receiptAppliedMicros', 'committedFrameMicros')
$segments = @('prepareMainThreadMicros', 'workerQueueMicros', 'workerTransformMicros', 'durableSaveMicros',
    'workerPostprocessMicros', 'receiptDeliveryAndApplyMicros')
$otherMetrics = @('dispatchReturnMicros', 'feedbackFrameMicros', 'committedFrameWorkMicros', 'peakFrameMicros')
foreach ($sample in $samples) {
    if ($sample.variant -ne 'candidate' -or $sample.caseMiB -ne $ContentMiB -or $sample.action -notin @('start', 'pause')) {
        throw 'Invalid diagnostic sample identity.'
    }
    foreach ($metric in ($orderedTimes + $segments + $otherMetrics + @('sample', 'actionId', 'stateBytes', 'journalBytes'))) { Assert-Metric $sample $metric }
    foreach ($stage in $stageNames) { Assert-Metric $sample.saveStagesMicros $stage }
    $presentControlled = @($controlledStageNames | Where-Object {
        $null -ne $sample.saveStagesMicros.PSObject.Properties[$_]
    }).Count
    if (($hasControlledStages -and $presentControlled -ne $controlledStageNames.Count) -or
        (-not $hasControlledStages -and $presentControlled -ne 0)) {
        throw 'The controlled-session stage set must be complete and consistent across all sixty samples.'
    }
    for ($index = 1; $index -lt $orderedTimes.Count; $index++) {
        if ($sample.($orderedTimes[$index]) -lt $sample.($orderedTimes[$index - 1])) { throw 'Diagnostic timestamp order regressed.' }
    }
    for ($index = 0; $index -lt $segments.Count; $index++) {
        $difference = [double]$sample.($orderedTimes[$index + 1]) - [double]$sample.($orderedTimes[$index])
        if ([Math]::Abs($difference - [double]$sample.($segments[$index])) -gt 1) {
            throw ('Segment does not match timestamp difference: ' + $segments[$index])
        }
    }
    $stages = $sample.saveStagesMicros
    $outer = [double]$stages.controlledSessionMicros + $stages.evidenceRefreshMicros + $stages.privacyMirrorsMicros + $stages.readableMirrorMicros + $stages.privacyFinishMicros
    $inner = [double]$stages.mirrorPrepareMicros + $stages.mirrorEvidenceMicros + $stages.mirrorFileMicros
    if ($hasControlledStages) {
        $controlledSum = [double]0
        foreach ($stage in $controlledStageNames) { $controlledSum += [double]$stages.$stage }
        if ($controlledSum -gt $stages.controlledSessionMicros) {
            throw 'Controlled-session child stages exceed their enclosing stage; intervals may overlap or use a different clock.'
        }
    }
    if ($outer -gt $stages.totalMicros -or $inner -gt $stages.totalMicros -or $stages.totalMicros -gt $sample.durableSaveMicros -or
        $sample.acceptedMicros -gt $sample.dispatchReturnMicros -or $sample.dispatchReturnMicros -gt $sample.feedbackFrameMicros -or
        $sample.feedbackFrameMicros -gt $sample.committedFrameMicros) {
        throw 'Save stage nesting, duration bound or first-frame timestamp validation failed.'
    }
}
if (@($samples | Select-Object -ExpandProperty actionId -Unique).Count -ne 60) { throw 'Each action must have a unique actionId.' }
$summary = foreach ($action in @('start', 'pause')) {
    $group = @($samples | Where-Object action -eq $action)
    if ($group.Count -ne 30 -or (@($group.sample | Sort-Object) -join ',') -ne ((0..29) -join ',')) { throw ('Missing or duplicate sample index for ' + $action) }
    $entry = [ordered]@{ diagnosticOnly = $true; variant = 'candidate'; caseMiB = $ContentMiB; action = $action; samples = 30; unit = 'microseconds'; hasControlledSessionStages = $hasControlledStages; saveStagesMicros = [ordered]@{}; timingMicros = [ordered]@{} }
    foreach ($stage in $stageNames) {
        $entry.saveStagesMicros[$stage] = Get-Percentiles @($group | ForEach-Object { [double]$_.saveStagesMicros.$stage })
    }
    foreach ($metric in ($orderedTimes + $segments + $otherMetrics)) {
        $entry.timingMicros[$metric] = Get-Percentiles @($group | ForEach-Object { [double]$_.$metric })
    }
    $entry
}
@($summary) | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $outputRoot 'timer_save_stages_summary.json') -Encoding UTF8
$manifest.completedAtUtc = [DateTime]::UtcNow.ToString('o')
$manifest.validatedSamples = 60
$manifest.hasControlledSessionStages = $hasControlledStages
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
foreach ($entry in $summary) {
    foreach ($stage in $stageNames) {
        $values = $entry.saveStagesMicros[$stage]
        Write-Output ('{0} {1}: P50={2} us; P95={3} us; max={4} us' -f $entry.action, $stage, $values.p50, $values.p95, $values.maximum)
    }
}
Write-Output ('Candidate diagnostic evidence, not baseline acceptance: ' + $outputRoot)
