# Windows 1.1.0.2: mutate behavioral boundaries and restore the exact source bytes.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$mutationRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$mutationSource = Join-Path $mutationRoot 'native/gridtimer_native/src/desktop'
$mutationEvidence = Join-Path $mutationRoot 'release_artifacts/verification/windows_v1.1.0.2/mutations'
New-Item -ItemType Directory -Force -Path $mutationEvidence | Out-Null
$mutationFiles = @('legal_risk.rs', 'legal_background.rs', 'startup_performance.rs')
$mutationOriginal = @{}
foreach ($name in $mutationFiles) { $mutationOriginal[$name] = [IO.File]::ReadAllBytes((Join-Path $mutationSource $name)) }
$mutationUtf8 = [Text.UTF8Encoding]::new($false)
$env:CARGO_TARGET_DIR = 'C:\gt\gridtimer-build\windows-release'
$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$env:LIB = 'C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
function Restore-MutationSources {
    foreach ($name in $mutationFiles) { [IO.File]::WriteAllBytes((Join-Path $mutationSource $name), $mutationOriginal[$name]) }
}
function Run-Mutation([string]$Label, [string]$Name, [string]$From, [string]$To, [string]$Test, [bool]$ShouldPass) {
    Restore-MutationSources
    $source = $mutationUtf8.GetString($mutationOriginal[$Name])
    if (-not $source.Contains($From)) { throw "Mutation anchor missing: $Label" }
    $source = $source.Replace($From, $To)
    [IO.File]::WriteAllText((Join-Path $mutationSource $Name), $source, $mutationUtf8)
    $log = Join-Path $mutationEvidence ($Label + '.log')
    & cargo test --manifest-path (Join-Path $mutationRoot 'native/gridtimer_native/Cargo.toml') --locked --offline --target x86_64-pc-windows-msvc --features desktop --bin timer_windows_client $Test -- --exact --test-threads=1 *> $log
    $exitCode = $LASTEXITCODE
    $output = [IO.File]::ReadAllText($log)
    if ($output -notmatch 'running 1 test' -or $output -match 'could not compile') { throw "Mutation did not reach the business test: $Label" }
    if ($ShouldPass -and ($exitCode -ne 0 -or $output -notmatch '1 passed; 0 failed')) { throw "Cosmetic mutation failed: $Label" }
    if (-not $ShouldPass -and ($exitCode -eq 0 -or $output -notmatch '0 passed; 1 failed')) { throw "Critical guard mutation escaped: $Label" }
    [pscustomobject]@{ name = $Label; test = $Test; exitCode = $exitCode; expectedPass = $ShouldPass; verified = $true }
}
$mutationResults = @()
try {
    $mutationResults += Run-Mutation 'wording' 'legal_risk.rs' '发送并分析' '确认发送并分析' 'tests::desktop_performance_probe::legal_send_dispatch_requires_configuration_and_matching_digest' $true
    $mutationResults += Run-Mutation 'send_authorization' 'legal_risk.rs' 'if !legal_can_send(' 'if false && !legal_can_send(' 'tests::desktop_performance_probe::legal_send_dispatch_requires_configuration_and_matching_digest' $false
    $mutationResults += Run-Mutation 'loading_write_boundary' 'startup_performance.rs' 'Ok(workspace) if workspace.workspace_persistence_ready =>' 'Ok(workspace) if true =>' 'tests::startup_unverified_receipt_requires_explicit_read_only_recovery' $false
    $mutationResults += Run-Mutation 'stale_preparation' 'legal_background.rs' 'outcome.version != self.data_version' 'false' 'tests::desktop_performance_probe::legal_preparation_drops_changed_cancelled_and_other_workspace_results' $false
} finally {
    Restore-MutationSources
    foreach ($name in $mutationFiles) {
        if ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([IO.File]::ReadAllBytes((Join-Path $mutationSource $name)))) -ne [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($mutationOriginal[$name]))) { throw "Source restore mismatch: $name" }
    }
}
$mutationResults | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $mutationEvidence 'mutation_results.json') -Encoding utf8
Write-Output 'All behavioral mutations matched expectations; original source bytes restored.'
