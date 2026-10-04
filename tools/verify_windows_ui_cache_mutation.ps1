#requires -Version 7.0
# Windows knowledge read caches: compile only the prepared immutable copies, sequentially.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$MutationManifest,
    [string]$OutputDirectory,
    [int]$BuildTimeoutSeconds = 3600,
    [int]$TestTimeoutSeconds = 180,
    [switch]$Resume
)
$ErrorActionPreference = 'Stop'
$uiCachePreviousJobs = $env:CARGO_BUILD_JOBS
$uiCachePreviousIncremental = $env:CARGO_INCREMENTAL
$uiCachePreviousToolchain = $env:RUSTUP_TOOLCHAIN
try {
    $env:CARGO_BUILD_JOBS = '2'
    $env:CARGO_INCREMENTAL = '0'
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    $uiCacheArguments = @{
        MutationManifest = $MutationManifest
        TargetDirectory = 'C:/gt/gridtimer-build/windows-release'
        Profile = 'release'
        BuildTimeoutSeconds = $BuildTimeoutSeconds
        TestTimeoutSeconds = $TestTimeoutSeconds
    }
    if ($OutputDirectory) { $uiCacheArguments.OutputDirectory = $OutputDirectory }
    if ($Resume) { $uiCacheArguments.Resume = $true }
    & (Join-Path $PSScriptRoot 'verify_windows_timer_mutation.ps1') @uiCacheArguments
} finally {
    $env:CARGO_BUILD_JOBS = $uiCachePreviousJobs
    $env:CARGO_INCREMENTAL = $uiCachePreviousIncremental
    $env:RUSTUP_TOOLCHAIN = $uiCachePreviousToolchain
}
