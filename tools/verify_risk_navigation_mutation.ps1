# v2.22.48 - Prove cosmetic tolerance and account-boundary test sensitivity.
$ErrorActionPreference = 'Stop'
$project = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$source = Join-Path $project 'native/gridtimer_native/src/desktop/navigation.rs'
$evidence = Join-Path $project 'release_artifacts/verification/v2.22.48-risk-navigation'
$original = [System.IO.File]::ReadAllBytes($source)
$content = [System.Text.Encoding]::UTF8.GetString($original)
$utf8 = [System.Text.UTF8Encoding]::new($false)
$beforeHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
$result = [ordered]@{ cosmeticPassed = $false; missingAccountGuardDetected = $false; restored = $false; sourceSha256 = $beforeHash }
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$env:LIB = 'C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
$arguments = @('run', 'stable-x86_64-pc-windows-msvc', 'cargo', 'test', '--manifest-path', (Join-Path $project 'native/gridtimer_native/Cargo.toml'), '--locked', '--offline', '--features', 'desktop', '--target', 'x86_64-pc-windows-msvc', '--bin', 'timer_windows_client')
try {
    $cosmetic = $content.Replace('返回便签列表', '回到便签列表')
    if ($cosmetic -eq $content) { throw 'Cosmetic marker missing' }
    [System.IO.File]::WriteAllText($source, $cosmetic, $utf8)
    & rustup @arguments 'navigation_' '--' '--nocapture' *> (Join-Path $evidence 'mutation_cosmetic.log')
    $result.cosmeticPassed = $LASTEXITCODE -eq 0
    if (-not $result.cosmeticPassed) { throw 'Cosmetic-only change failed a business-state test' }
    $guard = 'origin.workspace == self.background_job_workspace_fingerprint()'
    if (-not $content.Contains($guard)) { throw 'Account boundary marker missing' }
    [System.IO.File]::WriteAllText($source, $content.Replace($guard, 'true'), $utf8)
    & rustup @arguments 'navigation_timer_history_return_does_not_reopen_another_accounts_detail' '--' '--nocapture' *> (Join-Path $evidence 'mutation_guard.log')
    $log = [System.IO.File]::ReadAllText((Join-Path $evidence 'mutation_guard.log'))
    $result.missingAccountGuardDetected = $LASTEXITCODE -ne 0 -and $log.Contains('test result: FAILED') -and $log.Contains('assertion failed')
    if (-not $result.missingAccountGuardDetected) { throw 'Removed account guard was not detected by a test assertion' }
}
finally {
    [System.IO.File]::WriteAllBytes($source, $original)
    $result.restored = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -eq $beforeHash
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'mutation_result.json') -Encoding utf8
    if (-not $result.restored) { throw 'Source restoration did not match original bytes' }
}
$result | ConvertTo-Json
exit 0
