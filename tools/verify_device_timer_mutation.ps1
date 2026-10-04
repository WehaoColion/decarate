# v2.22.49 - Check cosmetic tolerance and device timer state protection.
$ErrorActionPreference = 'Stop'
$timerProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$timerEvidence = Join-Path $timerProject 'release_artifacts/verification/v2.22.49-device-timers'
$timerClient = Join-Path $timerProject 'native/gridtimer_native/src/bin/timer_windows_client.rs'
$timerRuntime = Join-Path $timerProject 'native/gridtimer_native/src/timer_sync.rs'
$clientBytes = [IO.File]::ReadAllBytes($timerClient)
$runtimeBytes = [IO.File]::ReadAllBytes($timerRuntime)
$clientText = [Text.Encoding]::UTF8.GetString($clientBytes)
$runtimeText = [Text.Encoding]::UTF8.GetString($runtimeBytes)
$timerEncoding = [Text.UTF8Encoding]::new($false)
function Write-TimerSource([string]$Path, [byte[]]$Bytes) {
    $absolute = [IO.Path]::GetFullPath($Path)
    if (-not $absolute.StartsWith($timerProject + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Source path escaped project' }
    $temporary = $absolute + '.timer-mutating.tmp'
    if (Test-Path -LiteralPath $temporary) { throw 'A mutation source replacement is already pending' }
    [IO.File]::WriteAllBytes($temporary, $Bytes)
    [IO.File]::Move($temporary, $absolute, $true)
}
$clientHash = (Get-FileHash -LiteralPath $timerClient -Algorithm SHA256).Hash
$runtimeHash = (Get-FileHash -LiteralPath $timerRuntime -Algorithm SHA256).Hash
$timerResult = [ordered]@{passed=$false;cosmeticPassed=$false;missingStateGuardDetected=$false;restored=$false;clientSourceSha256=$clientHash;runtimeSourceSha256=$runtimeHash}
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$env:LIB = 'C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
$timerArguments = @('run','stable-x86_64-pc-windows-msvc','cargo','test','--manifest-path',(Join-Path $timerProject 'native/gridtimer_native/Cargo.toml'),'--locked','--offline','--features','desktop','--target','x86_64-pc-windows-msvc','--lib','--bin','timer_windows_client','--no-fail-fast','device_timer_sync','--','--nocapture')
try {
    $cosmetic = $clientText.Replace('replace_state_with_source(next_json, "同步完成", state_source)', 'replace_state_with_source(next_json, "已同步", state_source)')
    if ($cosmetic -eq $clientText) { throw 'Cosmetic source anchor missing' }
    Write-TimerSource $timerClient ($timerEncoding.GetBytes($cosmetic))
    & rustup @timerArguments *> (Join-Path $timerEvidence 'mutation_cosmetic.log')
    $timerResult.cosmeticPassed = $LASTEXITCODE -eq 0
    if (-not $timerResult.cosmeticPassed) { throw 'Cosmetic edit changed a business-state test' }
    Write-TimerSource $timerClient $clientBytes
    $guard = '.and_then(|id| previous_slots.get(&id).copied());'
    if (-not $runtimeText.Contains($guard)) { throw 'Local timer guard anchor missing' }
    $mutated = $runtimeText.Replace($guard, '.and_then(|_id| None::<&Value>);')
    Write-TimerSource $timerRuntime ($timerEncoding.GetBytes($mutated))
    & rustup @timerArguments *> (Join-Path $timerEvidence 'mutation_guard.log')
    $guardExit = $LASTEXITCODE
    $guardLog = [IO.File]::ReadAllText((Join-Path $timerEvidence 'mutation_guard.log'))
    $timerResult.missingStateGuardDetected = $guardExit -ne 0 -and ([regex]::Matches($guardLog, 'test result: FAILED').Count -eq 2) -and $guardLog.Contains('panicked at') -and -not $guardLog.Contains('could not compile')
    if (-not $timerResult.missingStateGuardDetected) { throw 'Both shared and desktop state tests must reject the removed guard' }
} finally {
    Write-TimerSource $timerClient $clientBytes
    Write-TimerSource $timerRuntime $runtimeBytes
    $timerResult.restored = (Get-FileHash -LiteralPath $timerClient -Algorithm SHA256).Hash -eq $clientHash -and (Get-FileHash -LiteralPath $timerRuntime -Algorithm SHA256).Hash -eq $runtimeHash
    $timerResult.passed = $timerResult.cosmeticPassed -and $timerResult.missingStateGuardDetected -and $timerResult.restored
    $timerResult | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $timerEvidence 'device_timer_mutation.json') -Encoding utf8
    if (-not $timerResult.restored) { throw 'Source restoration mismatch' }
}
$timerResult | ConvertTo-Json
exit 0
