# v1.0.3.1 - Share the Windows-only Rust formatting scope with the release packager.
# v2.22.39 - Check build-volume headroom before Windows tests and packaging.
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('doctor', 'check', 'test', 'package', 'verify')]
    [string]$Command = 'doctor'
)

$ErrorActionPreference = 'Stop'
$toolchain = 'stable-x86_64-pc-windows-msvc'
$target = 'x86_64-pc-windows-msvc'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$nativeRoot = Join-Path $projectRoot 'native\gridtimer_native'
$manifestPath = Join-Path $projectRoot 'native\gridtimer_native\Cargo.toml'
$lockPath = Join-Path $projectRoot 'native\gridtimer_native\Cargo.lock'
$rustupCommand = Get-Command rustup -ErrorAction Stop

if ([string]::IsNullOrWhiteSpace($env:CARGO_HOME)) {
    $env:CARGO_HOME = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo'
}
if ([string]::IsNullOrWhiteSpace($env:GRIDTIMER_WINDOWS_TEST_TARGET_DIR)) {
    $env:GRIDTIMER_WINDOWS_TEST_TARGET_DIR = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        Join-Path $nativeRoot 'target'
    } elseif ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $nativeRoot $env:CARGO_TARGET_DIR))
    }
}

function Resolve-ProjectCachePath {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $projectRoot $Path))
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($Arguments -join ' ')"
    }
}

function Invoke-MsvcCargo {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    Assert-WindowsBuildSpace
    Invoke-Checked -FilePath $rustupCommand.Source -Arguments (@('run', $toolchain, 'cargo') + $Arguments)
}

function Get-WindowsBuildSpace {
    # These are free-space floors, not predictions of final build size. Keep
    # every cache in place and let explicit per-project overrides choose drives.
    $buildFloor = switch ($Command) {
        'check' { 4GB }
        'verify' { 2GB }
        default { 12GB }
    }
    $cargoTarget = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        Join-Path $nativeRoot 'target'
    } elseif ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $nativeRoot $env:CARGO_TARGET_DIR))
    }
    $requiredPaths = @(
        @{ Label = 'Cargo'; Path = $cargoTarget; Minimum = $buildFloor },
        @{ Label = 'Temporary files'; Path = [System.IO.Path]::GetTempPath(); Minimum = 2GB }
    )
    if ($Command -in @('doctor', 'test', 'package')) {
        $requiredPaths += @{ Label = 'Windows tests'; Path = (Resolve-ProjectCachePath $env:GRIDTIMER_WINDOWS_TEST_TARGET_DIR); Minimum = $buildFloor }
    }
    if ($Command -in @('doctor', 'package')) {
        $releaseTarget = if ([string]::IsNullOrWhiteSpace($env:GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR)) {
            'C:\gt\gridtimer-build\windows-release'
        } else {
            Resolve-ProjectCachePath $env:GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR
        }
        $requiredPaths += @{ Label = 'Windows release'; Path = $releaseTarget; Minimum = $buildFloor }
        $requiredPaths += @{ Label = 'Release publication'; Path = $projectRoot; Minimum = 1GB }
    }
    $volumes = @{}
    foreach ($requirement in $requiredPaths) {
        $root = [System.IO.Path]::GetPathRoot([System.IO.Path]::GetFullPath($requirement.Path))
        if (-not $volumes.ContainsKey($root)) {
            $drive = [System.IO.DriveInfo]::new($root)
            if (-not $drive.IsReady) {
                throw "Build volume is not ready: $root"
            }
            $volumes[$root] = [pscustomobject]@{
                Volume = $root
                AvailableBytes = $drive.AvailableFreeSpace
                MinimumBytes = [long]0
                Roles = [System.Collections.Generic.List[string]]::new()
            }
        }
        $entry = $volumes[$root]
        $entry.MinimumBytes = [Math]::Max($entry.MinimumBytes, [long]$requirement.Minimum)
        $entry.Roles.Add($requirement.Label)
    }
    $volumes.Values | Sort-Object Volume
}

function Assert-WindowsBuildSpace {
    foreach ($volume in Get-WindowsBuildSpace) {
        if ($volume.AvailableBytes -lt $volume.MinimumBytes) {
            throw ('Insufficient free build space on {0}: {1:N1} GiB available; at least {2:N0} GiB required for {3}. Free space manually or choose another drive with CARGO_TARGET_DIR / GRIDTIMER_WINDOWS_TEST_TARGET_DIR / GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR. No cache was deleted.' -f
                $volume.Volume, ($volume.AvailableBytes / 1GB), ($volume.MinimumBytes / 1GB), ($volume.Roles -join ', '))
        }
    }
}

function Resolve-WindowsBuildEnvironment {
    $toolchains = & $rustupCommand.Source toolchain list
    if ($LASTEXITCODE -ne 0 -or -not ($toolchains | Where-Object { $_ -match '^stable-x86_64-pc-windows-msvc(?:\s|$)' })) {
        throw "Required Rust toolchain is not installed: $toolchain"
    }

    $rustcPath = (& $rustupCommand.Source which --toolchain $toolchain rustc).Trim()
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $rustcPath -PathType Leaf)) {
        throw "rustup could not resolve rustc for $toolchain"
    }
    $rustcVersion = (& $rustcPath --version).Trim()
    if ($LASTEXITCODE -ne 0 -or $rustcVersion -notmatch '^rustc\s+(\d+)\.(\d+)\.(\d+)') {
        throw "Could not parse rustc version: $rustcVersion"
    }
    $resolvedVersion = [version]::new([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
    if ($resolvedVersion -lt [version]'1.95.0') {
        throw "Rust 1.95.0 or newer is required; found $rustcVersion"
    }

    $toolchainRoot = Split-Path -Parent (Split-Path -Parent $rustcPath)
    $linker = Join-Path $toolchainRoot "lib\rustlib\$target\bin\rust-lld.exe"
    if (-not (Test-Path -LiteralPath $linker -PathType Leaf)) {
        throw "MSVC linker is missing: $linker"
    }

    $xwinRoots = @()
    if (-not [string]::IsNullOrWhiteSpace($env:GRIDTIMER_XWIN_ROOT)) {
        $xwinRoots += $env:GRIDTIMER_XWIN_ROOT
    }
    $xwinRoots += 'C:\tools\xwin'
    $xwinRoot = $xwinRoots | Where-Object {
        Test-Path -LiteralPath (Join-Path $_ 'crt\lib\x86_64') -PathType Container
    } | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($xwinRoot)) {
        throw 'xwin MSVC libraries were not found. Set GRIDTIMER_XWIN_ROOT to the xwin directory.'
    }
    $libraryPaths = @(
        (Join-Path $xwinRoot 'crt\lib\x86_64'),
        (Join-Path $xwinRoot 'sdk\lib\ucrt\x86_64'),
        (Join-Path $xwinRoot 'sdk\lib\um\x86_64')
    )
    foreach ($libraryPath in $libraryPaths) {
        if (-not (Test-Path -LiteralPath $libraryPath -PathType Container)) {
            throw "Required MSVC library directory is missing: $libraryPath"
        }
    }
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
        throw 'Cargo.toml or Cargo.lock is missing from native/gridtimer_native.'
    }

    $env:RUSTUP_TOOLCHAIN = $toolchain
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker
    $env:LIB = $libraryPaths -join ';'
    [pscustomobject]@{
        Rustc = $rustcVersion
        Linker = $linker
        Xwin = $xwinRoot
    }
}

function Invoke-Doctor {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        throw 'The Windows build entrypoint can only run on Windows.'
    }
    $environment = Resolve-WindowsBuildEnvironment
    $gitState = (& git -C $projectRoot status --porcelain --untracked-files=all 2>$null | Measure-Object).Count
    [pscustomobject]@{
        Project = $projectRoot
        Toolchain = $toolchain
        Rustc = $environment.Rustc
        Target = $target
        Linker = $environment.Linker
        Xwin = $environment.Xwin
        CargoHome = if ([System.IO.Path]::IsPathRooted($env:CARGO_HOME)) {
            [System.IO.Path]::GetFullPath($env:CARGO_HOME)
        } else {
            [System.IO.Path]::GetFullPath((Join-Path $nativeRoot $env:CARGO_HOME))
        }
        CargoTarget = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
            Join-Path $nativeRoot 'target'
        } elseif ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
            [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
        } else {
            [System.IO.Path]::GetFullPath((Join-Path $nativeRoot $env:CARGO_TARGET_DIR))
        }
        WindowsTestTarget = Resolve-ProjectCachePath $env:GRIDTIMER_WINDOWS_TEST_TARGET_DIR
        WindowsReleaseTarget = if ([string]::IsNullOrWhiteSpace($env:GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR)) {
            'C:\gt\gridtimer-build\windows-release'
        } else {
            Resolve-ProjectCachePath $env:GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR
        }
        GitStatusItems = $gitState
    } | Format-List
    Get-WindowsBuildSpace | Select-Object Volume,
        @{ Name = 'AvailableGiB'; Expression = { [Math]::Round($_.AvailableBytes / 1GB, 1) } },
        @{ Name = 'MinimumGiB'; Expression = { $_.MinimumBytes / 1GB } },
        @{ Name = 'BuildRoles'; Expression = { $_.Roles -join ', ' } } | Format-Table -AutoSize
}

Push-Location $nativeRoot
try {
    Invoke-Doctor
    switch ($Command) {
        'doctor' { }
        'check' {
            Invoke-MsvcCargo -Arguments @(
                'run', '--locked', '--offline', '--manifest-path', $manifestPath,
                '--target', $target, '--bin', 'gridtimer_packager', '--', 'format-windows'
            )
            Invoke-MsvcCargo -Arguments @(
                'check', '--locked', '--offline', '--features', 'desktop',
                '--target', $target, '--lib',
                '--bin', 'timer_windows_client', '--bin', 'timer_sync_server',
                '--bin', 'timer_sync_launcher', '--bin', 'tenrate_desktop_launcher',
                '--bin', 'gridtimer_packager'
            )
        }
        'test' {
            $testTargetDirectory = Resolve-ProjectCachePath $env:GRIDTIMER_WINDOWS_TEST_TARGET_DIR
            foreach ($binary in @('timer_windows_client', 'timer_sync_launcher', 'gridtimer_packager', 'tenrate_desktop_launcher')) {
                Invoke-MsvcCargo -Arguments @(
                    'test', '--locked', '--offline', '--features', 'desktop',
                    '--target', $target, '--target-dir', $testTargetDirectory,
                    '--bin', $binary, '--no-fail-fast'
                )
            }
            Invoke-MsvcCargo -Arguments @(
                'test', '--locked', '--offline', '--features', 'desktop',
                '--target', $target, '--target-dir', $testTargetDirectory, '--lib', '--no-fail-fast'
            )
        }
        'package' {
            Invoke-MsvcCargo -Arguments @(
                'run', '--locked', '--offline', '--manifest-path', $manifestPath,
                '--target', $target, '--bin', 'gridtimer_packager', '--',
                'build', '--windows-only', '--configuration', 'Release'
            )
        }
        'verify' {
            Invoke-MsvcCargo -Arguments @(
                'run', '--locked', '--offline', '--manifest-path', $manifestPath,
                '--target', $target, '--bin', 'gridtimer_packager', '--',
                'finish', '--windows-only', '--validate-only'
            )
        }
    }
}
finally {
    Pop-Location
}
