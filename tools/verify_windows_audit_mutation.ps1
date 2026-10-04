# v2.22.51 - Prove cosmetic tolerance and editor/reminder state guards.
$ErrorActionPreference = 'Stop'
$auditProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$auditEvidence = Join-Path $auditProject 'release_artifacts/verification/v2.22.51-windows-audit'
$auditSource = Join-Path $auditProject 'native/gridtimer_native/src/bin/timer_windows_client.rs'
$auditBytes = [IO.File]::ReadAllBytes($auditSource)
$auditText = [Text.Encoding]::UTF8.GetString($auditBytes)
$auditEncoding = [Text.UTF8Encoding]::new($false)
$auditHash = (Get-FileHash -LiteralPath $auditSource -Algorithm SHA256).Hash
function Write-AuditSource([byte[]]$Bytes) {
    $temporary = $auditSource + '.audit-mutation.tmp'
    if (Test-Path -LiteralPath $temporary) { throw 'Mutation file already pending' }
    [IO.File]::WriteAllBytes($temporary, $Bytes)
    [IO.File]::Move($temporary, $auditSource, $true)
}
$auditResult = [ordered]@{passed=$false;cosmeticPassed=$false;removedGuardsDetected=$false;restored=$false;sourceSha256=$auditHash}
$env:CARGO_INCREMENTAL='0'
$env:CARGO_BUILD_JOBS='2'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
$auditArguments=@('run','stable-x86_64-pc-windows-msvc','cargo','test','--manifest-path',(Join-Path $auditProject 'native/gridtimer_native/Cargo.toml'),'--locked','--offline','--features','desktop','--target','x86_64-pc-windows-msvc','--bin','timer_windows_client','windows_audit','--','--nocapture','--test-threads=2')
try {
    $cosmetic=$auditText.Replace('replace_state_with_source(next_json, "同步完成", state_source)','replace_state_with_source(next_json, "已同步", state_source)')
    if($cosmetic -eq $auditText){throw 'Cosmetic anchor missing'}
    Write-AuditSource ($auditEncoding.GetBytes($cosmetic))
    & rustup @auditArguments *> (Join-Path $auditEvidence 'windows_mutation_cosmetic.log')
    $auditResult.cosmeticPassed=$LASTEXITCODE -eq 0
    if(-not $auditResult.cosmeticPassed){throw 'Cosmetic mutation failed business tests'}
    $mutant=$auditText.Replace('if unchanged {','if false && unchanged {').Replace('self.rebase_synced_interval_bell_markers(&previous_data, timer_now);','// Removed by the isolated regression mutation.')
    if($mutant -eq $auditText -or -not $auditText.Contains('if unchanged {') -or -not $auditText.Contains('self.rebase_synced_interval_bell_markers(&previous_data, timer_now);')) {throw 'State guard anchor missing'}
    Write-AuditSource ($auditEncoding.GetBytes($mutant))
    & rustup @auditArguments *> (Join-Path $auditEvidence 'windows_mutation_guards.log')
    $mutationExit=$LASTEXITCODE
    $mutationLog=[IO.File]::ReadAllText((Join-Path $auditEvidence 'windows_mutation_guards.log'))
    $auditResult.removedGuardsDetected=$mutationExit -ne 0 -and $mutationLog.Contains('2 passed; 5 failed') -and $mutationLog.Contains('panicked at') -and -not $mutationLog.Contains('could not compile')
    if(-not $auditResult.removedGuardsDetected){throw 'Removed guards did not fail the five editor and reminder guard cases'}
} finally {
    Write-AuditSource $auditBytes
    $auditResult.restored=(Get-FileHash -LiteralPath $auditSource -Algorithm SHA256).Hash -eq $auditHash
    $auditResult.passed=$auditResult.cosmeticPassed -and $auditResult.removedGuardsDetected -and $auditResult.restored
    $auditResult | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $auditEvidence 'windows_mutation_result.json') -Encoding utf8
    if(-not $auditResult.restored){throw 'Original source bytes were not restored'}
}
$auditResult | ConvertTo-Json
