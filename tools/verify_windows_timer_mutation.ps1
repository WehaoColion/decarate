#requires -Version 7.0
# Windows 1.1.0.3: only build the immutable, isolated copies from preparation.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$MutationManifest,
    [Parameter(Mandatory = $true)][string]$TargetDirectory,
    [string]$OutputDirectory,
    [string]$TargetTriple = 'x86_64-pc-windows-msvc',
    [string]$Toolchain = $env:RUSTUP_TOOLCHAIN,
    [ValidateSet('release', 'test')][string]$Profile = 'release',
    [int]$BuildTimeoutSeconds = 3600,
    [int]$TestTimeoutSeconds = 180,
    [string]$PauseFile,
    [switch]$Resume
)
$ErrorActionPreference = 'Stop'
$Profile = $Profile.ToLowerInvariant()
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationRoot = [IO.Path]::GetFullPath((Join-Path $repo 'release_artifacts/verification'))
$manifestPath = (Resolve-Path -LiteralPath $MutationManifest).Path
$preparedRoot = Split-Path -Parent $manifestPath
$targetRoot = [IO.Path]::GetFullPath($TargetDirectory)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $preparedRoot 'execution' }
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
function Is-ChildPath([string]$Path, [string]$Parent) {
    return $Path.StartsWith($Parent.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}
if (-not (Is-ChildPath $preparedRoot $verificationRoot) -or -not (Is-ChildPath $outputRoot $verificationRoot)) {
    throw 'Prepared copies and execution evidence must remain inside repository verification artifacts.'
}
if ($targetRoot.TrimEnd('\', '/') -eq $preparedRoot.TrimEnd('\', '/') -or
    (Is-ChildPath $targetRoot $preparedRoot) -or (Is-ChildPath $preparedRoot $targetRoot)) {
    throw 'Use one external shared Cargo target, separate from the prepared source copies.'
}
if ($BuildTimeoutSeconds -lt 60 -or $TestTimeoutSeconds -lt 10) { throw 'Timeout is too short.' }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.formatVersion -ne 2 -or -not $manifest.unmodified) { throw 'Regenerate isolated copies with prepare_windows_timer_mutations.py format 2.' }
if ((Test-Path -LiteralPath $outputRoot) -and -not $Resume) { throw 'Existing evidence is retained. Use -Resume only with identical inputs.' }
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
New-Item -ItemType Directory -Path $targetRoot -Force | Out-Null
if (-not $PauseFile) { $PauseFile = Join-Path $outputRoot 'pause_requested' }
$PauseFile = [IO.Path]::GetFullPath($PauseFile)
$cargo = (Get-Command cargo -CommandType Application).Source
$rustc = (Get-Command rustc -CommandType Application).Source
$scriptSha = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
$manifestSha = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash
$childEnvironment = [ordered]@{
    CARGO_TARGET_DIR = $targetRoot; CARGO_INCREMENTAL = '0'
    LOCALAPPDATA = (Join-Path $outputRoot 'local_app_data')
}
if ($Toolchain) { $childEnvironment.RUSTUP_TOOLCHAIN = $Toolchain }
foreach ($name in @('LIB', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER')) {
    $childEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
}
$linkerName = 'CARGO_TARGET_' + $TargetTriple.ToUpperInvariant().Replace('-', '_') + '_LINKER'
$childEnvironment[$linkerName] = [Environment]::GetEnvironmentVariable($linkerName)
foreach ($entry in (Get-ChildItem Env: | Where-Object { $_.Name -match '^CARGO_PROFILE_' } | Sort-Object Name)) {
    $childEnvironment[$entry.Name] = $entry.Value
}
$buildWorkspace = Join-Path $preparedRoot 'build_workspace'
$buildPackage = Join-Path $buildWorkspace 'native/gridtimer_native'
$activeSourcePath = Join-Path $preparedRoot 'build_workspace_active.json'
$libraryGuardPath = Join-Path $outputRoot 'first_library_rebuild_guard.json'
$profileDirectory = if ($Profile -eq 'release') { 'release' } else { 'debug' }
$expectedArtifactRoot = [IO.Path]::GetFullPath((Join-Path $targetRoot ($TargetTriple + '/' + $profileDirectory + '/deps')))
$buildArguments = @('test')
if ($Profile -eq 'release') {
    $buildArguments += '--release'
} else {
    # Match the ordinary test gate in gridtimer_packager::windows_release_command_args.
    $buildArguments += @('--config', 'profile.test.package.gridtimer_native.debug=0')
}
$buildArguments += @('--locked', '--offline', '--features', 'desktop', '--bin', 'timer_windows_client', '--no-run', '--message-format=json', '--target', $TargetTriple, '--manifest-path', (Join-Path $buildPackage 'Cargo.toml'))
$profileSelection = [ordered]@{
    effectiveProfile = $Profile; defaultProfile = 'release'
    overridesPreparedReleaseBuildProfile = ($Profile -eq 'test')
    preparedManifestModified = $false
    purpose = 'Business-state mutation verification; performance measurements retain their separate release executables.'
}
function Write-Json([string]$Path, $Object) {
    $temporary = $Path + '.writing'
    $Object | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $temporary -Encoding utf8
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}
function Sha-Text([string]$Text) {
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text)))
}
function Invoke-OwnedProcess([string]$Executable, [string[]]$ArgumentVector, [string]$WorkingDirectory, [string]$LogPrefix, [int]$Timeout) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    foreach ($argument in $ArgumentVector) { $start.ArgumentList.Add($argument) }
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($entry in $childEnvironment.GetEnumerator()) {
        if ($null -ne $entry.Value) { $start.Environment[$entry.Key] = [string]$entry.Value }
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $started = [DateTime]::UtcNow
    if (-not $process.Start()) { throw ('Cannot start ' + $Executable) }
    $ownedPid = $process.Id
    $ownedStart = $process.StartTime.ToUniversalTime()
    $outTask = $process.StandardOutput.ReadToEndAsync()
    $errTask = $process.StandardError.ReadToEndAsync()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $nextProgress = 15
    $timedOut = $false
    while (-not $process.WaitForExit(250)) {
        if ($clock.Elapsed.TotalSeconds -ge $nextProgress) {
            Write-Host ((Split-Path -Leaf $LogPrefix) + ' elapsed ' + [int]$clock.Elapsed.TotalSeconds + 's')
            $nextProgress += 15
        }
        if ($clock.Elapsed.TotalSeconds -ge $Timeout) {
            $process.Refresh()
            if ($process.Id -ne $ownedPid -or $process.StartTime.ToUniversalTime() -ne $ownedStart -or
                $process.MainModule.FileName -ne $Executable) {
                throw 'Owned process identity changed; no process was terminated.'
            }
            # Only descendants of this runner-owned Cargo/test process are stopped.
            $process.Kill($true)
            $process.WaitForExit()
            $timedOut = $true
            break
        }
    }
    $stdout = $outTask.GetAwaiter().GetResult()
    $stderr = $errTask.GetAwaiter().GetResult()
    $stdout | Set-Content -LiteralPath ($LogPrefix + '.stdout.log') -Encoding utf8
    $stderr | Set-Content -LiteralPath ($LogPrefix + '.stderr.log') -Encoding utf8
    $result = [ordered]@{
        executable = $Executable; arguments = $ArgumentVector; workingDirectory = $WorkingDirectory
        pid = $ownedPid; startedAtUtc = $started.ToString('o'); completedAtUtc = [DateTime]::UtcNow.ToString('o')
        exitCode = $process.ExitCode; timedOut = $timedOut; elapsedSeconds = $clock.Elapsed.TotalSeconds
        stdoutSha256 = (Get-FileHash -LiteralPath ($LogPrefix + '.stdout.log') -Algorithm SHA256).Hash
        stderrSha256 = (Get-FileHash -LiteralPath ($LogPrefix + '.stderr.log') -Algorithm SHA256).Hash
    }
    Write-Json ($LogPrefix + '.process.json') $result
    $process.Dispose()
    return [pscustomobject]@{ metadata = $result; stdout = $stdout; stderr = $stderr }
}
function Assert-Inputs($Variant) {
    $package = [IO.Path]::GetFullPath($Variant.packageDirectory)
    $variantRoot = Split-Path -Parent (Split-Path -Parent $package)
    if (-not (Is-ChildPath $package $preparedRoot) -or $package -eq (Join-Path $repo 'native/gridtimer_native')) {
        throw 'Build package must be an isolated prepared copy.'
    }
    foreach ($entry in $Variant.inputFiles.PSObject.Properties) {
        $path = [IO.Path]::GetFullPath((Join-Path $variantRoot $entry.Name))
        if (-not (Is-ChildPath $path $variantRoot) -or -not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw ('Missing or escaping prepared input: ' + $entry.Name)
        }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
            throw ('Prepared input hash changed: ' + $path)
        }
    }
    return $package
}
function Assert-ProductInputs {
    foreach ($entry in $manifest.sourceFiles.PSObject.Properties) {
        $path = [IO.Path]::GetFullPath((Join-Path $repo $entry.Name))
        if (-not (Is-ChildPath $path $repo) -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
            throw ('Product source no longer matches this prepared run: ' + $entry.Name)
        }
    }
}
function Assert-ProcessEvidence([string]$Prefix, [string]$ExpectedMetadataHash) {
    $metadataPath = $Prefix + '.process.json'
    if (-not (Test-Path -LiteralPath $metadataPath) -or
        (Get-FileHash -LiteralPath $metadataPath -Algorithm SHA256).Hash -ne $ExpectedMetadataHash) {
        throw ('Process metadata hash changed: ' + $Prefix)
    }
    $record = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
    foreach ($stream in @('stdout', 'stderr')) {
        $log = $Prefix + '.' + $stream + '.log'
        $property = $stream + 'Sha256'
        if (-not (Test-Path -LiteralPath $log) -or (Get-FileHash -LiteralPath $log -Algorithm SHA256).Hash -ne $record.$property) {
            throw ('Process output hash changed: ' + $log)
        }
    }
}
function Assert-BuildWorkspace($Variant) {
    foreach ($entry in $Variant.inputFiles.PSObject.Properties) {
        $path = [IO.Path]::GetFullPath((Join-Path $buildWorkspace $entry.Name))
        if (-not (Is-ChildPath $path $buildWorkspace) -or -not (Test-Path -LiteralPath $path -PathType Leaf) -or
            (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
            throw ('Active build workspace input mismatch: ' + $entry.Name)
        }
    }
}
function Select-BuildVariant($Variant) {
    $cleanPackage = Assert-Inputs $manifest.unmodified
    $cleanRoot = Split-Path -Parent (Split-Path -Parent $cleanPackage)
    if (-not (Test-Path -LiteralPath $buildWorkspace)) {
        if (Test-Path -LiteralPath $activeSourcePath) { throw 'Build workspace is missing but its active record exists.' }
        New-Item -ItemType Directory -Path $buildWorkspace | Out-Null
        foreach ($entry in $manifest.unmodified.inputFiles.PSObject.Properties) {
            $destination = Join-Path $buildWorkspace $entry.Name
            New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
            Copy-Item -LiteralPath (Join-Path $cleanRoot $entry.Name) -Destination $destination
        }
        Assert-BuildWorkspace $manifest.unmodified
        Write-Json $activeSourcePath ([ordered]@{ fingerprint = $fingerprint; variant = 'unmodified' })
    }
    if (-not (Test-Path -LiteralPath $activeSourcePath)) { throw 'Build workspace has no verified active-source record; do not overwrite unknown files.' }
    $active = Get-Content -LiteralPath $activeSourcePath -Raw | ConvertFrom-Json
    if ($active.fingerprint -ne $fingerprint) { throw 'Active build workspace belongs to different prepared inputs, profile, runner, or toolchain. Do not reuse it across profiles.' }
    $known = @($manifest.unmodified) + @($manifest.variants)
    $previous = @($known | Where-Object { $_.name -eq $active.variant })
    if ($previous.Count -ne 1) { throw 'Unknown active source variant.' }
    Assert-BuildWorkspace $previous[0]
    if ($previous[0].name -ne $Variant.name) {
        if ($previous[0].changedFile) {
            $relative = 'native/gridtimer_native/' + $previous[0].changedFile
            Copy-Item -LiteralPath (Join-Path $cleanRoot $relative) -Destination (Join-Path $buildWorkspace $relative)
        }
        if ($Variant.changedFile) {
            $isolated = Assert-Inputs $Variant
            Copy-Item -LiteralPath (Join-Path $isolated $Variant.changedFile) -Destination (Join-Path $buildPackage $Variant.changedFile)
        }
        Assert-BuildWorkspace $Variant
        Write-Json $activeSourcePath ([ordered]@{ fingerprint = $fingerprint; variant = $Variant.name })
    }
    return $buildPackage
}
function Prepare-FirstLibraryBuild {
    $libraryPath = Join-Path $buildPackage 'src/lib.rs'
    $expectedSha = $manifest.unmodified.inputFiles.'native/gridtimer_native/src/lib.rs'
    if (-not $expectedSha -or -not (Is-ChildPath $libraryPath $buildWorkspace)) {
        throw 'Cannot establish the isolated library source identity.'
    }
    if (Test-Path -LiteralPath $libraryGuardPath) {
        $guard = Get-Content -LiteralPath $libraryGuardPath -Raw | ConvertFrom-Json
        if ($guard.fingerprint -ne $fingerprint -or $guard.libraryPath -ne $libraryPath -or
            $guard.beforeSha256 -ne $expectedSha -or $guard.afterSha256 -ne $expectedSha -or
            (Get-FileHash -LiteralPath $libraryPath -Algorithm SHA256).Hash -ne $expectedSha) {
            throw 'The first-library-build protection belongs to different inputs.'
        }
        # Resume preserves a previous build attempt and never invalidates a verified executable.
        return
    }
    $beforeSha = (Get-FileHash -LiteralPath $libraryPath -Algorithm SHA256).Hash
    if ($beforeSha -ne $expectedSha) { throw 'Isolated library bytes changed before the first build.' }
    $beforeTime = [IO.File]::GetLastWriteTimeUtc($libraryPath)
    [IO.File]::SetLastWriteTimeUtc($libraryPath, [DateTime]::UtcNow)
    $afterSha = (Get-FileHash -LiteralPath $libraryPath -Algorithm SHA256).Hash
    if ($afterSha -ne $beforeSha) { throw 'The library timestamp refresh changed file bytes.' }
    Write-Json $libraryGuardPath ([ordered]@{
        fingerprint = $fingerprint; libraryPath = $libraryPath
        reason = 'A shared Cargo target may contain the baseline package normal rlib. A fresh lib-test executable does not prove that normal rlib was rebuilt. Refresh only the isolated src/lib.rs timestamp before the first unmodified client-test build.'
        beforeSha256 = $beforeSha; afterSha256 = $afterSha
        beforeLastWriteTimeUtc = $beforeTime.ToString('o')
        afterLastWriteTimeUtc = [IO.File]::GetLastWriteTimeUtc($libraryPath).ToString('o')
        recordedAtUtc = [DateTime]::UtcNow.ToString('o')
    })
}
# Reading versions is not a build. The version log is part of resume identity.
$preflight = Join-Path $outputRoot ('preflight_' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $preflight | Out-Null
$cargoVersion = Invoke-OwnedProcess $cargo @('--version') $preparedRoot (Join-Path $preflight 'cargo_version') 30
$rustVersion = Invoke-OwnedProcess $rustc @('--version', '--verbose') $preparedRoot (Join-Path $preflight 'rustc_version') 30
if ($cargoVersion.metadata.exitCode -ne 0 -or $rustVersion.metadata.exitCode -ne 0) { throw 'Cannot establish toolchain identity.' }
$configPaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$ancestor = [IO.DirectoryInfo]::new($buildPackage)
while ($null -ne $ancestor) {
    foreach ($name in @('config', 'config.toml')) { [void]$configPaths.Add((Join-Path $ancestor.FullName ('.cargo/' + $name))) }
    $ancestor = $ancestor.Parent
}
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
foreach ($name in @('config', 'config.toml')) { [void]$configPaths.Add((Join-Path $cargoHome $name)) }
$configHashes = [ordered]@{}
foreach ($path in ($configPaths | Sort-Object)) {
    $configHashes[$path] = if (Test-Path -LiteralPath $path -PathType Leaf) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash } else { $null }
}
$identity = [ordered]@{
    manifestSha256 = $manifestSha; runnerSha256 = $scriptSha; targetTriple = $TargetTriple
    cargoVersion = $cargoVersion.stdout.Trim(); rustcVersion = $rustVersion.stdout.Trim()
    childEnvironment = $childEnvironment; cargoConfigFiles = $configHashes; sourceFiles = $manifest.sourceFiles
    buildPackage = $buildPackage
    profileSelection = $profileSelection; effectiveBuildArguments = $buildArguments
    expectedArtifactRoot = $expectedArtifactRoot
}
$fingerprint = Sha-Text ($identity | ConvertTo-Json -Depth 30 -Compress)
$runManifestPath = Join-Path $outputRoot 'run_manifest.json'
if (Test-Path -LiteralPath $runManifestPath) {
    $oldRun = Get-Content -LiteralPath $runManifestPath -Raw | ConvertFrom-Json
    if (-not $Resume -or $oldRun.fingerprint -ne $fingerprint) { throw 'Resume inputs/profile/build arguments/toolchain/runner do not match the existing evidence.' }
}
$run = [ordered]@{ fingerprint = $fingerprint; inputs = $identity; status = 'running'; updatedAtUtc = [DateTime]::UtcNow.ToString('o') }
Write-Json $runManifestPath $run
function Should-Pause {
    if (-not (Test-Path -LiteralPath $PauseFile)) { return $false }
    $run.status = 'paused'; $run.updatedAtUtc = [DateTime]::UtcNow.ToString('o')
    Write-Json $runManifestPath $run
    Write-Host ('Paused between phases. Remove only your pause marker, then use -Resume: ' + $PauseFile)
    return $true
}
try {
    Assert-ProductInputs
    foreach ($variant in @($manifest.unmodified) + @($manifest.variants)) {
        Assert-Inputs $variant | Out-Null
        if (Should-Pause) { return }
        $package = Select-BuildVariant $variant
        $variantEvidence = Join-Path $outputRoot $variant.name
        New-Item -ItemType Directory -Path $variantEvidence -Force | Out-Null
        $buildResultPath = Join-Path $variantEvidence 'build_result.json'
        $executableSuffix = if ($Profile -eq 'release') { '_release_tests.exe' } else { '_test_profile_tests.exe' }
        $savedExecutable = Join-Path $variantEvidence ($variant.name + $executableSuffix)
        $buildVerified = $false
        if ($Resume -and (Test-Path -LiteralPath $buildResultPath)) {
            $oldBuild = Get-Content -LiteralPath $buildResultPath -Raw | ConvertFrom-Json
            if ($oldBuild.fingerprint -eq $fingerprint -and $oldBuild.profile -eq $Profile -and $oldBuild.verified -and
                ($oldBuild.effectiveBuildArguments | ConvertTo-Json -Compress) -eq ($buildArguments | ConvertTo-Json -Compress) -and
                (Is-ChildPath $oldBuild.artifact $expectedArtifactRoot) -and
                (Test-Path -LiteralPath $savedExecutable) -and
                (Get-FileHash -LiteralPath $savedExecutable -Algorithm SHA256).Hash -eq $oldBuild.executableSha256) {
                if (-not (Is-ChildPath $oldBuild.processLogPrefix $variantEvidence)) { throw 'Saved build log path escaped its evidence directory.' }
                Assert-ProcessEvidence $oldBuild.processLogPrefix $oldBuild.processMetadataSha256
                if (-not (Test-Path -LiteralPath $libraryGuardPath) -or
                    (Get-FileHash -LiteralPath $libraryGuardPath -Algorithm SHA256).Hash -ne $oldBuild.firstLibraryRebuildGuardSha256) {
                    throw 'Saved first-library-build protection evidence changed.'
                }
                $buildVerified = $true
            } else { throw ('Saved build evidence does not match: ' + $variant.name) }
        }
        if (-not $buildVerified) {
            if (Should-Pause) { return }
            if ($variant.name -eq 'unmodified') { Prepare-FirstLibraryBuild }
            $buildLogPrefix = Join-Path $variantEvidence ('build_' + [Guid]::NewGuid().ToString('N'))
            $build = Invoke-OwnedProcess $cargo $buildArguments $package $buildLogPrefix $BuildTimeoutSeconds
            $artifact = $null
            $artifactProfile = $null
            foreach ($line in ($build.stdout -split "`r?`n")) {
                if (-not $line.StartsWith('{')) { continue }
                try { $message = $line | ConvertFrom-Json } catch { continue }
                if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'timer_windows_client' -and
                    $message.profile.test -and $message.executable) {
                    $artifact = [IO.Path]::GetFullPath($message.executable)
                    $artifactProfile = $message.profile
                }
            }
            if ($build.metadata.timedOut -or $build.metadata.exitCode -ne 0 -or -not $artifact -or
                -not (Is-ChildPath $artifact $expectedArtifactRoot) -or -not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
                throw ('Compilation did not successfully produce the exact test artifact: ' + $variant.name)
            }
            Assert-Inputs $variant | Out-Null
            Assert-BuildWorkspace $variant
            Copy-Item -LiteralPath $artifact -Destination $savedExecutable
            Write-Json $buildResultPath ([ordered]@{
                fingerprint = $fingerprint; verified = $true; sourcePackage = $package
                profile = $Profile; profileSelection = $profileSelection
                effectiveBuildArguments = $buildArguments; preparedBuildArguments = $variant.buildArguments
                expectedArtifactRoot = $expectedArtifactRoot; compilerArtifactProfile = $artifactProfile
                exitCode = $build.metadata.exitCode; artifact = $artifact; savedExecutable = $savedExecutable
                executableSha256 = (Get-FileHash -LiteralPath $savedExecutable -Algorithm SHA256).Hash
                processLogPrefix = $buildLogPrefix
                processMetadataSha256 = (Get-FileHash -LiteralPath ($buildLogPrefix + '.process.json') -Algorithm SHA256).Hash
                firstLibraryRebuildGuardSha256 = (Get-FileHash -LiteralPath $libraryGuardPath -Algorithm SHA256).Hash
            })
        }
        $exeSha = (Get-FileHash -LiteralPath $savedExecutable -Algorithm SHA256).Hash
        $testIndex = 0
        foreach ($test in $variant.exactTests) {
            $testIndex += 1
            $testStem = Join-Path $variantEvidence ('test_' + $testIndex.ToString('00'))
            $testResultPath = $testStem + '.result.json'
            if ($Resume -and (Test-Path -LiteralPath $testResultPath)) {
                $previous = Get-Content -LiteralPath $testResultPath -Raw | ConvertFrom-Json
                if ($previous.fingerprint -ne $fingerprint -or $previous.profile -ne $Profile -or $previous.executableSha256 -ne $exeSha -or
                    $previous.exactTest -ne $test -or $previous.expectedOutcome -ne $variant.expectedTestOutcome -or
                    -not (Is-ChildPath $previous.processLogPrefix $variantEvidence)) {
                    throw ('Completed test evidence does not match: ' + $testResultPath)
                }
                Assert-ProcessEvidence $previous.processLogPrefix $previous.processMetadataSha256
                if ($previous.verified) {
                    Write-Host ('Reusing verified ' + $variant.name + ' ' + $test)
                    continue
                }
                # Keep failed attempts as evidence; they are rerun, never reused as success.
                Copy-Item -LiteralPath $testResultPath -Destination ($testStem + '_failed_' + [Guid]::NewGuid().ToString('N') + '.result.json')
            }
            if (Should-Pause) { return }
            $prefix = $testStem + '_attempt_' + [Guid]::NewGuid().ToString('N')
            $result = Invoke-OwnedProcess $savedExecutable @($test, '--exact', '--nocapture', '--test-threads=1') $package $prefix $TestTimeoutSeconds
            $output = $result.stdout + "`n" + $result.stderr
            $pass = -not $result.metadata.timedOut -and $result.metadata.exitCode -eq 0 -and
                $output -match 'running 1 test' -and $output -match 'test result: ok\. 1 passed; 0 failed'
            $assertionFailed = -not $result.metadata.timedOut -and $result.metadata.exitCode -eq 101 -and
                $output -match 'running 1 test' -and $output -match 'test result: FAILED\. 0 passed; 1 failed' -and
                $output -match '(?s)panicked at.*assertion[^\r\n]*failed'
            $verified = if ($variant.expectedTestOutcome -eq 'pass') { $pass } else { $assertionFailed }
            Write-Json $testResultPath ([ordered]@{
                fingerprint = $fingerprint; profile = $Profile; verified = $verified; exactTest = $test
                expectedOutcome = $variant.expectedTestOutcome; executableSha256 = $exeSha
                exitCode = $result.metadata.exitCode; timedOut = $result.metadata.timedOut
                rustAssertionFailed = $assertionFailed; ordinaryPass = $pass
                processLogPrefix = $prefix
                processMetadataSha256 = (Get-FileHash -LiteralPath ($prefix + '.process.json') -Algorithm SHA256).Hash
            })
            if (-not $verified) { throw ('Unexpected business mutation outcome: ' + $variant.name + ' ' + $test) }
            Assert-ProductInputs
        }
    }
    Assert-ProductInputs
    $run.status = 'completed'; $run.updatedAtUtc = [DateTime]::UtcNow.ToString('o')
    Write-Json $runManifestPath $run
    Write-Host ('All isolated timer mutations matched expected outcomes: ' + $outputRoot)
} catch {
    $run.status = 'failed'; $run.error = $_.Exception.Message; $run.updatedAtUtc = [DateTime]::UtcNow.ToString('o')
    Write-Json $runManifestPath $run
    throw
}
