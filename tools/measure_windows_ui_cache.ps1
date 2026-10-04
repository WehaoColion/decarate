#requires -Version 7.0
# Directly run the actual compiled client test executable; never builds or starts a GUI.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$TestExecutable,
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [ValidateRange(30, 600)][int]$TimeoutSeconds = 180
)
$ErrorActionPreference = 'Stop'
$uiCacheProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$uiCacheEvidence = [IO.Path]::GetFullPath($EvidenceRoot)
$uiCacheVerification = Join-Path $uiCacheProject 'release_artifacts/verification'
$uiCacheExecutable = (Resolve-Path -LiteralPath $TestExecutable).Path
if (-not $uiCacheEvidence.StartsWith($uiCacheVerification.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Evidence must stay under this project verification directory.'
}
if ([IO.Path]::GetFileName($uiCacheExecutable) -notmatch '^timer_windows_client[-_].*\.exe$') {
    throw 'An explicit hashed client test executable is required.'
}
if (Test-Path -LiteralPath $uiCacheEvidence) { throw 'Keep prior evidence; select a fresh output directory.' }
New-Item -ItemType Directory -Path $uiCacheEvidence | Out-Null
$uiCacheProfile = Join-Path $uiCacheEvidence 'startup_synthetic_ui_cache'
New-Item -ItemType Directory -Path $uiCacheProfile | Out-Null
$uiCacheReport = Join-Path $uiCacheEvidence 'read_cache_performance.json'
$uiCacheSourceFiles = @(
    'native/gridtimer_native/src/desktop/knowledge_read_cache.rs',
    'native/gridtimer_native/src/desktop/knowledge_experience.rs',
    'native/gridtimer_native/src/desktop/knowledge_database_ui.rs',
    'native/gridtimer_native/src/desktop/knowledge_workspace.rs',
    'native/gridtimer_native/src/desktop/knowledge_ui_cache_tests.rs',
    'native/gridtimer_native/src/desktop/knowledge_ui_cache_benchmark.rs',
    'native/gridtimer_native/src/desktop/performance_tests.rs',
    'native/gridtimer_native/src/bin/timer_windows_client.rs',
    'native/gridtimer_native/src/product_identity.rs',
    'native/gridtimer_native/Cargo.lock'
)
$uiCacheHashesBefore = [ordered]@{}
foreach ($relative in $uiCacheSourceFiles) {
    $uiCacheHashesBefore[$relative] = (Get-FileHash -LiteralPath (Join-Path $uiCacheProject $relative) -Algorithm SHA256).Hash.ToLowerInvariant()
}
$uiCacheExecutableSha = (Get-FileHash -LiteralPath $uiCacheExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
function Invoke-UiCacheTest([string]$Name, [string[]]$ArgumentVector) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $uiCacheExecutable
    foreach ($argument in $ArgumentVector) { $start.ArgumentList.Add($argument) }
    $start.WorkingDirectory = $uiCacheProject
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['LOCALAPPDATA'] = $uiCacheProfile
    $start.Environment['DESKTOP_UI_CACHE_PERFORMANCE_OUTPUT'] = $uiCacheReport
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $began = [DateTime]::UtcNow
    $watch = [Diagnostics.Stopwatch]::StartNew()
    if (-not $process.Start()) { throw 'Cannot start the explicit client test executable.' }
    $ownedPid = $process.Id
    $ownedStart = $process.StartTime.ToUniversalTime()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $progressAt = 15
    $timedOut = $false
    while (-not $process.WaitForExit(250)) {
        if ($watch.Elapsed.TotalSeconds -ge $progressAt) {
            Write-Host "$Name elapsed $([int]$watch.Elapsed.TotalSeconds)s"
            $progressAt += 15
        }
        if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            $process.Refresh()
            if ($process.Id -ne $ownedPid -or $process.StartTime.ToUniversalTime() -ne $ownedStart) {
                throw 'Owned test process identity changed; no process terminated.'
            }
            $process.Kill($true)
            $process.WaitForExit()
            $timedOut = $true
            break
        }
    }
    $out = $stdout.GetAwaiter().GetResult()
    $err = $stderr.GetAwaiter().GetResult()
    $prefix = Join-Path $uiCacheEvidence $Name
    [IO.File]::WriteAllText($prefix + '.stdout.log', $out, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($prefix + '.stderr.log', $err, [Text.UTF8Encoding]::new($false))
    $result = [ordered]@{
        executable = $uiCacheExecutable; executableSha256 = $uiCacheExecutableSha
        arguments = $ArgumentVector; pid = $ownedPid; startedAtUtc = $began.ToString('o')
        endedAtUtc = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $watch.Elapsed.TotalSeconds
        exitCode = $process.ExitCode; timedOut = $timedOut
        stdoutSha256 = (Get-FileHash -LiteralPath ($prefix + '.stdout.log') -Algorithm SHA256).Hash.ToLowerInvariant()
        stderrSha256 = (Get-FileHash -LiteralPath ($prefix + '.stderr.log') -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    [IO.File]::WriteAllText($prefix + '.process.json', ($result | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
    $process.Dispose()
    if ($result.timedOut -or $result.exitCode -ne 0) { throw "The actual client test failed: $Name" }
    return [pscustomobject]@{ metadata = $result; stdout = $out }
}
$uiCacheList = Invoke-UiCacheTest 'test_inventory' @('--list')
$uiCacheBenchmark = 'tests::desktop_performance_probe::knowledge_ui_cache_performance_benchmark'
$uiCacheBoundaryTests = @(
    'tests::knowledge_quick_navigation_cache_tracks_query_history_notes_and_workspace',
    'tests::knowledge_quick_navigation_never_exposes_encrypted_or_deleted_page_titles',
    'tests::knowledge_database_frame_cache_preserves_order_query_and_snapshot_boundaries'
)
foreach ($test in @($uiCacheBenchmark) + $uiCacheBoundaryTests) {
    if ($uiCacheList.stdout -notmatch ([regex]::Escape($test) + ': test')) {
        throw "The supplied executable does not contain the actual business test: $test"
    }
}
$uiCacheRuns = @()
foreach ($test in $uiCacheBoundaryTests) {
    $label = $test.Split('::')[-1]
    $run = Invoke-UiCacheTest $label @($test, '--exact', '--test-threads=1', '--nocapture')
    if ($run.stdout -notmatch '1 passed; 0 failed') { throw "Business test did not execute: $test" }
    $uiCacheRuns += $run.metadata
}
$uiCacheMeasured = Invoke-UiCacheTest 'read_cache_performance' @($uiCacheBenchmark, '--exact', '--ignored', '--test-threads=1', '--nocapture')
if ($uiCacheMeasured.stdout -notmatch '1 passed; 0 failed' -or -not (Test-Path -LiteralPath $uiCacheReport -PathType Leaf)) {
    throw 'The benchmark did not produce a successful report.'
}
$uiCachePerformance = Get-Content -LiteralPath $uiCacheReport -Raw | ConvertFrom-Json
if ($uiCachePerformance.profile -ne 'release-test') { throw 'Quantified acceptance requires the actual release-profile test executable.' }
$uiCacheHashesAfter = [ordered]@{}
foreach ($relative in $uiCacheSourceFiles) {
    $after = (Get-FileHash -LiteralPath (Join-Path $uiCacheProject $relative) -Algorithm SHA256).Hash.ToLowerInvariant()
    $uiCacheHashesAfter[$relative] = $after
    if ($after -ne $uiCacheHashesBefore[$relative]) { throw "Product source changed while measuring: $relative" }
}
if ((Get-FileHash -LiteralPath $uiCacheExecutable -Algorithm SHA256).Hash.ToLowerInvariant() -ne $uiCacheExecutableSha) {
    throw 'The measured executable changed during the run.'
}
$uiCacheFinal = [ordered]@{
    format = 'windows_ui_cache_acceptance_v1'; completedAtUtc = [DateTime]::UtcNow.ToString('o')
    executable = $uiCacheExecutable; executableSha256 = $uiCacheExecutableSha
    profile = 'release'; sourceUnchanged = $true
    sourceFilesBefore = $uiCacheHashesBefore; sourceFilesAfter = $uiCacheHashesAfter
    boundaryTests = $uiCacheRuns; benchmarkProcess = $uiCacheMeasured.metadata
    performanceReport = $uiCacheReport
    performanceReportSha256 = (Get-FileHash -LiteralPath $uiCacheReport -Algorithm SHA256).Hash.ToLowerInvariant()
    performance = $uiCachePerformance
}
[IO.File]::WriteAllText((Join-Path $uiCacheEvidence 'ui_cache_acceptance.json'), ($uiCacheFinal | ConvertTo-Json -Depth 25), [Text.UTF8Encoding]::new($false))
Write-Output ($uiCacheFinal | ConvertTo-Json -Depth 25)
