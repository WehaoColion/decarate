# v0.0.1 - Verify Windows build-space checks without compiling or changing caches.
$ErrorActionPreference = 'Stop'
$source = Join-Path $PSScriptRoot 'windows.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw ($parseErrors -join [Environment]::NewLine) }
foreach ($name in @('Resolve-ProjectCachePath', 'Get-WindowsBuildSpace', 'Assert-WindowsBuildSpace')) {
    $definition = $ast.Find({ param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name
    }, $true)
    if ($null -eq $definition) { throw "Missing preflight function: $name" }
    . ([scriptblock]::Create($definition.Extent.Text))
}
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$nativeRoot = Join-Path $projectRoot 'native\gridtimer_native'
$previousEnvironment = @{}
foreach ($name in @('CARGO_TARGET_DIR', 'GRIDTIMER_WINDOWS_TEST_TARGET_DIR', 'GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR')) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:CARGO_TARGET_DIR = 'target-preflight-test'
    $env:GRIDTIMER_WINDOWS_TEST_TARGET_DIR = 'test-preflight-test'
    $env:GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR = 'release-preflight-test'
    foreach ($case in @(@('doctor', 12GB), @('test', 12GB), @('package', 12GB), @('check', 4GB), @('verify', 2GB))) {
        $Command = $case[0]
        $volumes = @(Get-WindowsBuildSpace)
        $buildVolume = $volumes | Where-Object { $_.Roles.Contains('Cargo') } | Select-Object -First 1
        if ($null -eq $buildVolume -or $buildVolume.MinimumBytes -ne $case[1]) {
            throw "Incorrect free-space floor for $Command"
        }
        if ($buildVolume.AvailableBytes -lt 0 -or [string]::IsNullOrWhiteSpace($buildVolume.Volume)) {
            throw "Invalid disk-space result for $Command"
        }
    }
    if (Test-Path -LiteralPath (Join-Path $projectRoot 'test-preflight-test')) {
        throw 'Preflight unexpectedly created its test target'
    }
    $realSpaceFunction = (Get-Item Function:Get-WindowsBuildSpace).ScriptBlock
    try {
        function Get-WindowsBuildSpace {
            [pscustomobject]@{ Volume = 'test-volume'; AvailableBytes = 1GB; MinimumBytes = 12GB; Roles = @('Windows tests') }
        }
        $rejected = $false
        try { Assert-WindowsBuildSpace } catch {
            if ($_.Exception.Message -notlike 'Insufficient free build space*') { throw }
            $rejected = $true
        }
        if (-not $rejected) { throw 'Low disk space must stop the build before Cargo runs' }
        function Get-WindowsBuildSpace {
            [pscustomobject]@{ Volume = 'test-volume'; AvailableBytes = 12GB; MinimumBytes = 12GB; Roles = @('Windows tests') }
        }
        Assert-WindowsBuildSpace
    }
    finally { Set-Item Function:Get-WindowsBuildSpace $realSpaceFunction }
    Write-Output 'Windows preflight tests passed: five command floors, read-only paths, low-space rejection, exact-floor acceptance.'
}
finally {
    foreach ($entry in $previousEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process')
    }
}
