# v0.0.1 - Compile isolated copies of the real canvas transaction code and tests.
$ErrorActionPreference = 'Stop'
$canvasRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$canvasSource = Join-Path $canvasRoot 'native/gridtimer_native/src/android_canvas.rs'
$canvasTests = Join-Path $canvasRoot 'native/gridtimer_native/src/android_canvas_tests.rs'
$canvasVersion = [regex]::Match((Get-Content (Join-Path $canvasRoot 'app/build.gradle') -Raw), "versionName '([^']+)'").Groups[1].Value
$canvasEvidence = Join-Path $canvasRoot "release_artifacts/verification/v$canvasVersion/canvas_mutation"
$canvasSourceHash = (Get-FileHash -LiteralPath $canvasSource).Hash
$canvasTestsHash = (Get-FileHash -LiteralPath $canvasTests).Hash
$canvasOriginal = [IO.File]::ReadAllText($canvasSource)
$canvasTestCount = ([regex]'#\[test\]').Matches([IO.File]::ReadAllText($canvasTests)).Count
$canvasUtf8 = [Text.UTF8Encoding]::new($false)
$canvasEnv = @{}
foreach($key in @('RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR','CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER','LIB')) { $canvasEnv[$key] = [Environment]::GetEnvironmentVariable($key,'Process') }
$results = @()
try {
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    $env:CARGO_TARGET_DIR = 'C:\gt\gridtimer-build\sourcegen'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
    $env:LIB = 'C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    foreach($case in @('baseline','wording_only','removed_stale_write_guard')) {
        $directory = Join-Path $canvasEvidence $case
        New-Item -ItemType Directory -Force -Path (Join-Path $directory 'src') | Out-Null
        $source = $canvasOriginal
        if($case -eq 'wording_only') { $source = $source.Replace('知识画布','灵感画布') }
        if($case -eq 'removed_stale_write_guard') {
            $guard = 'if file_digest(path)? != fingerprint {'
            if(!$source.Contains($guard)) { throw 'Stale-write guard not found' }
            $source = $source.Replace($guard,'if false {')
        }
        [IO.File]::WriteAllText((Join-Path $directory 'src/android_canvas.rs'),$source,$canvasUtf8)
        Copy-Item -LiteralPath $canvasTests -Destination (Join-Path $directory 'src/android_canvas_tests.rs') -Force
        [IO.File]::WriteAllText((Join-Path $directory 'src/lib.rs'),'mod android_canvas;',$canvasUtf8)
        $manifest = @'
[package]
name = "tenfold_canvas_acceptance"
version = "0.0.1"
edition = "2021"
[dependencies]
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.145"
sha2 = "0.10.8"
rand = "0.8.5"
jni = "0.21.1"
'@
        [IO.File]::WriteAllText((Join-Path $directory 'Cargo.toml'),$manifest,$canvasUtf8)
        Copy-Item -LiteralPath (Join-Path $canvasRoot 'native/gridtimer_native/Cargo.lock') -Destination (Join-Path $directory 'Cargo.lock') -Force
        & cargo test --manifest-path (Join-Path $directory 'Cargo.toml') --offline --lib -- --test-threads=1 *> (Join-Path $directory 'tests.log')
        $code = $LASTEXITCODE
        $log = [IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
        $expectedPass = $case -ne 'removed_stale_write_guard'
        $passed = if($expectedPass) { $code -eq 0 -and $log.Contains("$canvasTestCount passed; 0 failed") } else { $code -ne 0 -and $log.Contains('assertion failed') -and $log.Contains('another_editor_cannot_overwrite_a_newer_saved_revision ... FAILED') }
        $results += [ordered]@{case=$case;expectedPass=$expectedPass;exitCode=$code;passed=$passed}
        if(!$passed) { Get-Content (Join-Path $directory 'tests.log') -Tail 45; throw "Unexpected canvas mutation result: $case" }
        Write-Output "$case verified"
    }
} finally {
    foreach($key in $canvasEnv.Keys) { [Environment]::SetEnvironmentVariable($key,$canvasEnv[$key],'Process') }
}
if((Get-FileHash -LiteralPath $canvasSource).Hash -ne $canvasSourceHash -or (Get-FileHash -LiteralPath $canvasTests).Hash -ne $canvasTestsHash) { throw 'Production source changed during isolated verification' }
[ordered]@{passed=$true;sourceSha256=$canvasSourceHash;testsSha256=$canvasTestsHash;tests=$canvasTestCount;productionUnchanged=$true;cases=$results} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $canvasEvidence 'mutation_result.json') -Encoding utf8
