# v2.23.2.10 - Verify the actual source generator rejects ambiguous header edits.
[CmdletBinding()]
param([string]$Version = '2.23.2.10')
$ErrorActionPreference = 'Stop'
$headerRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android header mutation version' }
$headerRelativeSource = 'native/gridtimer_native/src/sourcegen/android_knowledge_header.rs'
$headerSourcePath = Join-Path $headerRoot $headerRelativeSource
$headerReceiptDirectory = Join-Path $headerRoot "release_artifacts/verification/v$Version/knowledge_header_mutation"
New-Item -ItemType Directory -Force -Path $headerReceiptDirectory | Out-Null
$headerOriginalBytes = [IO.File]::ReadAllBytes($headerSourcePath)
$headerOriginalText = [Text.Encoding]::UTF8.GetString($headerOriginalBytes)
$headerSourceHash = (Get-FileHash -LiteralPath $headerSourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$headerVerifierHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$headerUtf8 = [Text.UTF8Encoding]::new($false)
$headerTestModule = 'android_note_list_performance::android_knowledge_header::tests'
$headerTestNames = @([regex]::Matches($headerOriginalText, '(?m)#\[test\]\s*fn\s+(?<name>[A-Za-z0-9_]+)\s*\(') | ForEach-Object { $_.Groups['name'].Value })
$headerRequiredTests = @('refuses_missing_duplicate_or_unreconciled_template_anchors', 'refuses_ambiguous_helper_removal_boundaries')
if ($headerTestNames.Count -ne 2 -or @($headerRequiredTests | Where-Object { $_ -notin $headerTestNames }).Count -ne 0) {
    throw 'Actual source-generator failure-path tests are missing or unexpected'
}
$headerEnvironmentNames = @('RUSTUP_TOOLCHAIN', 'LIB', 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER', 'CARGO_TARGET_DIR', 'CARGO_INCREMENTAL')
$headerOldEnvironment = @{}
foreach ($name in $headerEnvironmentNames) { $headerOldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }

function Replace-HeaderMutationAnchor([string]$Value, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Value, [regex]::Escape($Anchor)).Count -ne 1) { throw 'Header mutation anchor is missing or ambiguous' }
    $Value.Replace($Anchor, $Replacement)
}
function Write-HeaderMutationBytes([byte[]]$Bytes) {
    $temporary = $headerSourcePath + '.header-test-mutation.tmp'
    if (-not ([IO.Path]::GetFullPath($temporary)).StartsWith($headerRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Header mutation target is outside the project'
    }
    [IO.File]::WriteAllBytes($temporary, $Bytes)
    [IO.File]::Move($temporary, $headerSourcePath, $true)
}
function Get-HeaderEvidenceHash([string]$Path) {
    if (Test-Path -LiteralPath $Path -PathType Leaf) { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
    return $null
}
function Invoke-HeaderMutationCase([string]$Name, [string]$Kind, [byte[]]$Bytes, [bool]$ExpectFailure, [string]$RequiredFailure = '') {
    Write-HeaderMutationBytes $Bytes
    $compileJson = Join-Path $headerReceiptDirectory ($Name + '_compile.jsonl')
    $compileLog = Join-Path $headerReceiptDirectory ($Name + '_compile.log')
    $testLog = Join-Path $headerReceiptDirectory ($Name + '_tests.log')
    [IO.File]::WriteAllText($testLog, '', $headerUtf8)
    # The explicit MSVC host toolchain matches android.ps1's source-generator cache.
    & $headerCargo test --manifest-path (Join-Path $headerRoot 'native/gridtimer_native/Cargo.toml') --locked --offline --bin gridtimer_sourcegen --no-run --message-format=json 1> $compileJson 2> $compileLog
    $compileExit = $LASTEXITCODE
    $testExit = $null
    $artifact = $null
    $requiredObserved = $false
    $allTestsObserved = $false
    if ($compileExit -eq 0) {
        $artifact = Get-Content -LiteralPath $compileJson | ForEach-Object {
            try {
                $item = $_ | ConvertFrom-Json
                if ($item.reason -eq 'compiler-artifact' -and $item.executable -and $item.target.name -eq 'gridtimer_sourcegen' -and $item.profile.test) { $item.executable }
            } catch { }
        } | Select-Object -Last 1
        if (!$artifact -or !(Test-Path -LiteralPath $artifact -PathType Leaf)) { throw "No actual source-generator test executable for $Name" }
        & $artifact $headerTestModule '--test-threads=1' *> $testLog
        $testExit = $LASTEXITCODE
        $testText = [IO.File]::ReadAllText($testLog)
        $allTestsObserved = $true
        foreach ($testName in $headerTestNames) {
            $testLine = '(?m)^test ' + [regex]::Escape($headerTestModule + '::' + $testName) + ' \.\.\. (ok|FAILED)\s*$'
            if ($testText -notmatch $testLine) { $allTestsObserved = $false }
        }
        if ($RequiredFailure) {
            $failureLine = '(?m)^test ' + [regex]::Escape($headerTestModule + '::' + $RequiredFailure) + ' \.\.\. FAILED\s*$'
            $requiredObserved = $testText -match $failureLine -and $testText.Contains('test result: FAILED') -and $testText.Contains('assertion failed')
        }
    }
    $passed = $compileExit -eq 0 -and $allTestsObserved -and $null -ne $testExit -and $(if ($ExpectFailure) { $testExit -ne 0 -and $requiredObserved } else { $testExit -eq 0 })
    Write-Output "Header mutation $Name compile=$compileExit test=$testExit passed=$passed" | Write-Host
    return [ordered]@{
        name = $Name; kind = $Kind; compileExit = $compileExit; testExit = $testExit
        expectedPass = !$ExpectFailure; requiredFailureTest = $RequiredFailure; requiredFailureObserved = [bool]$requiredObserved
        allBusinessTestsObserved = [bool]$allTestsObserved; passed = [bool]$passed
        sourceSha256 = (Get-HeaderEvidenceHash $headerSourcePath)
        compileJsonSha256 = (Get-HeaderEvidenceHash $compileJson); compileLogSha256 = (Get-HeaderEvidenceHash $compileLog)
        testLogSha256 = (Get-HeaderEvidenceHash $testLog); executableSha256 = $(if ($artifact) { Get-HeaderEvidenceHash $artifact } else { $null })
    }
}

$headerCases = @()
$headerFailure = $null
try {
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    $headerRustc = (& rustup which --toolchain $env:RUSTUP_TOOLCHAIN rustc).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Rust toolchain is unavailable' }
    $headerToolchain = Split-Path (Split-Path $headerRustc)
    $headerCargo = Join-Path $headerToolchain 'bin/cargo.exe'
    $env:LIB = 'C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $headerToolchain 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
    $env:CARGO_TARGET_DIR = 'C:\gt\gridtimer-build\sourcegen'
    $env:CARGO_INCREMENTAL = '0'

    # Mutate the real module in the complete Cargo crate; no substitute module is compiled.
    $headerCosmetic = Replace-HeaderMutationAnchor $headerOriginalText 'modifier = Modifier.fillMaxWidth().padding(14.dp),' 'modifier = Modifier.fillMaxWidth().padding(15.dp),'
    $headerGuard = '    if source.matches("FlowusWorkspaceHeader(").count() != 2 {' + [char]10 + '        return Err("unexpected document workspace header usage".into());' + [char]10 + '    }'
    if ($headerOriginalText.Contains("`r`n")) { $headerGuard = $headerGuard.Replace("`n", "`r`n") }
    $headerMissingGuard = Replace-HeaderMutationAnchor $headerOriginalText $headerGuard ''
    $specifications = @(
        @{ name = 'baseline'; kind = 'baseline'; bytes = $headerOriginalBytes; fail = $false; required = '' },
        @{ name = 'cosmetic'; kind = 'rendered_padding_only'; bytes = $headerUtf8.GetBytes($headerCosmetic); fail = $false; required = '' },
        @{ name = 'missing_header_usage_guard'; kind = 'header_usage_guard_removed'; bytes = $headerUtf8.GetBytes($headerMissingGuard); fail = $true; required = $headerRequiredTests[0] },
        @{ name = 'restored'; kind = 'restored'; bytes = $headerOriginalBytes; fail = $false; required = '' }
    )
    foreach ($specification in $specifications) {
        $result = Invoke-HeaderMutationCase $specification.name $specification.kind $specification.bytes $specification.fail $specification.required
        $headerCases += $result
        if (!$result.passed) { throw "Unexpected source-generator mutation result: $($specification.name)" }
    }
} catch {
    $headerFailure = $_.Exception.Message
} finally {
    Write-HeaderMutationBytes $headerOriginalBytes
    foreach ($name in $headerEnvironmentNames) { [Environment]::SetEnvironmentVariable($name, $headerOldEnvironment[$name], 'Process') }
}
$headerAfterHash = (Get-FileHash -LiteralPath $headerSourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$headerVerifierAfterHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$headerPassed = !$headerFailure -and $headerSourceHash -ceq $headerAfterHash -and $headerVerifierHash -ceq $headerVerifierAfterHash -and $headerCases.Count -eq 4 -and @($headerCases | Where-Object { !$_.passed }).Count -eq 0
$headerReceipt = [ordered]@{
    passed = [bool]$headerPassed; version = $Version; sourcePath = $headerRelativeSource
    sourceSha256Before = $headerSourceHash; sourceSha256After = $headerAfterHash; productionUnchanged = ($headerSourceHash -ceq $headerAfterHash)
    verifierPath = 'tools/verify_knowledge_header_mutation.ps1'; verifierSha256 = $headerVerifierHash; verifierSha256After = $headerVerifierAfterHash
    tests = $headerTestNames.Count; testModule = $headerTestModule; testNames = $headerTestNames; cases = $headerCases
    cargoTarget = 'x86_64-pc-windows-msvc'; targetSelection = 'Explicit MSVC toolchain host without --target, matching android.ps1'; cargoTargetDirectory = 'C:\gt\gridtimer-build\sourcegen'
    hostOnly = $true; networkRequests = 0; noDeviceOperations = $true; error = $headerFailure
    scope = 'Actual Rust source-generator failure paths in the complete Cargo crate. New rendered padding changes must pass; deleting the header usage guard must fail its required test. No Compose or device acceptance is claimed.'
    checkedAt = [DateTimeOffset]::Now.ToString('o')
}
[IO.File]::WriteAllText((Join-Path $headerReceiptDirectory 'receipt.json'), ($headerReceipt | ConvertTo-Json -Depth 12), $headerUtf8)
if (!$headerPassed) { throw "Android knowledge header mutation acceptance failed: $headerFailure" }
Write-Output "Android $Version knowledge header mutation acceptance completed"
