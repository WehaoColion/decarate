# v0.0.1 - Run persisted-operation and capacity scenarios against the production core.
$ErrorActionPreference='Stop'
$scenarioRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match((Get-Content (Join-Path $scenarioRoot 'app/build.gradle') -Raw),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $scenarioRoot "release_artifacts/verification/v$version/canvas_scenarios"
New-Item -ItemType Directory -Force -Path (Join-Path $evidence 'src') | Out-Null
$source=Join-Path $scenarioRoot 'native/gridtimer_native/src/android_canvas.rs'
$sourceHash=(Get-FileHash -LiteralPath $source).Hash
Copy-Item -LiteralPath $source -Destination (Join-Path $evidence 'src/android_canvas.rs') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'canvas_scenarios.rs') -Destination (Join-Path $evidence 'src/main.rs') -Force
Copy-Item -LiteralPath (Join-Path $scenarioRoot 'native/gridtimer_native/Cargo.lock') -Destination (Join-Path $evidence 'Cargo.lock') -Force
@'
[package]
name = "tenfold_canvas_scenarios"
version = "0.0.1"
edition = "2021"
[dependencies]
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.145"
sha2 = "0.10.8"
rand = "0.8.5"
jni = "0.21.1"
'@ | Set-Content -LiteralPath (Join-Path $evidence 'Cargo.toml') -Encoding utf8
$previous=@{}
foreach($key in @('RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR','CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER','LIB')) { $previous[$key]=[Environment]::GetEnvironmentVariable($key,'Process') }
try {
    $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
    $env:CARGO_TARGET_DIR='C:\gt\gridtimer-build\canvas-performance'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
    $env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    & cargo run --offline --manifest-path (Join-Path $evidence 'Cargo.toml') 1> (Join-Path $evidence 'result.json') 2> (Join-Path $evidence 'run.log')
    if($LASTEXITCODE -ne 0) {Get-Content (Join-Path $evidence 'run.log') -Tail 35; throw 'Canvas scenario verification failed'}
} finally { foreach($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key,$previous[$key],'Process') } }
if((Get-FileHash -LiteralPath $source).Hash -ne $sourceHash) {throw 'Production source changed during scenario verification'}
$result=Get-Content (Join-Path $evidence 'result.json') -Raw | ConvertFrom-Json
if(!$result.passed -or $result.scenarios.Count -ne 9) {throw 'Incomplete canvas scenarios'}
$result | Add-Member -NotePropertyName sourceSha256 -NotePropertyValue $sourceHash
$result | Add-Member -NotePropertyName completed -NotePropertyValue ((Get-Date).ToUniversalTime().ToString('o'))
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
Write-Output "Nine canvas persistence and capacity scenarios passed: $version"
