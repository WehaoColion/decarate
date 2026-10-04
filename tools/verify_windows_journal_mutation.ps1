# v2.22.51 - Verify that cosmetic edits pass and content-independent cache keys fail.
$ErrorActionPreference='Stop'
$auditRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$auditEvidence=Join-Path $auditRoot 'release_artifacts/verification/v2.22.51-windows-audit'
$auditSource=Join-Path $auditRoot 'native/gridtimer_native/src/desktop_state_store.rs'
$auditBytes=[IO.File]::ReadAllBytes($auditSource)
$auditText=[Text.Encoding]::UTF8.GetString($auditBytes)
$auditEncoding=[Text.UTF8Encoding]::new($false)
$auditHash=(Get-FileHash -LiteralPath $auditSource -Algorithm SHA256).Hash
function Write-JournalMutation([byte[]]$Bytes) {
    $temporary=$auditSource+'.audit-mutation.tmp'
    if(Test-Path -LiteralPath $temporary){throw 'Mutation already pending'}
    [IO.File]::WriteAllBytes($temporary,$Bytes)
    [IO.File]::Move($temporary,$auditSource,$true)
}
$env:CARGO_INCREMENTAL='0'
$env:CARGO_BUILD_JOBS='2'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
$auditArguments=@('run','stable-x86_64-pc-windows-msvc','cargo','test','--manifest-path',(Join-Path $auditRoot 'native/gridtimer_native/Cargo.toml'),'--locked','--offline','--features','desktop','--target','x86_64-pc-windows-msvc','--lib')
$auditResult=[ordered]@{passed=$false;cosmeticPassed=$false;contentKeyGuardDetected=$false;restored=$false;sourceSha256=$auditHash}
try {
    $cosmetic=$auditText.Replace('snapshot content or semantic metadata diverged','snapshot content does not match its metadata')
    if($cosmetic -eq $auditText){throw 'Cosmetic anchor missing'}
    Write-JournalMutation ($auditEncoding.GetBytes($cosmetic))
    & rustup @auditArguments journal_audit -- --nocapture --test-threads=1 *> (Join-Path $auditEvidence 'mutation_cosmetic.log')
    $auditResult.cosmeticPassed=$LASTEXITCODE -eq 0
    if(-not $auditResult.cosmeticPassed){throw 'Cosmetic mutation changed business behavior'}
    $mutant=$auditText.Replace('let key = (sha256_hex(raw.as_bytes()), compatibility_now);','let key = ("0".repeat(64), compatibility_now);')
    if($mutant -eq $auditText){throw 'Content key anchor missing'}
    Write-JournalMutation ($auditEncoding.GetBytes($mutant))
    & rustup @auditArguments journal_audit_changed_bytes_cannot_reuse_previous_semantics -- --nocapture --test-threads=1 *> (Join-Path $auditEvidence 'mutation_guard.log')
    $auditMutationExit=$LASTEXITCODE
    $auditMutationLog=[IO.File]::ReadAllText((Join-Path $auditEvidence 'mutation_guard.log'))
    $auditResult.contentKeyGuardDetected=$auditMutationExit -ne 0 -and $auditMutationLog.Contains('0 passed; 1 failed') -and $auditMutationLog.Contains('panicked at') -and -not $auditMutationLog.Contains('could not compile')
    if(-not $auditResult.contentKeyGuardDetected){throw 'A content-independent cache key escaped the integrity test'}
} finally {
    Write-JournalMutation $auditBytes
    $auditResult.restored=(Get-FileHash -LiteralPath $auditSource -Algorithm SHA256).Hash -eq $auditHash
    $auditResult.passed=$auditResult.cosmeticPassed -and $auditResult.contentKeyGuardDetected -and $auditResult.restored
    $auditResult | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $auditEvidence 'mutation_result.json') -Encoding utf8
    if(-not $auditResult.restored){throw 'Journal source bytes were not restored'}
}
$auditResult | ConvertTo-Json
