# v0.0.1 - Compare the preserved previous core with the production optimized core.
$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match((Get-Content (Join-Path $taskRoot 'app/build.gradle') -Raw),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $taskRoot "release_artifacts/verification/v$version"
$previous=@{}
foreach($key in @('RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR','CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER','LIB')){$previous[$key]=[Environment]::GetEnvironmentVariable($key,'Process')}
$results=@()
try {
    $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
    $env:CARGO_TARGET_DIR='C:\gt\gridtimer-build\canvas-performance'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path $env:USERPROFILE '.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
    $env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    foreach($case in @('before','after')) {
        $directory=Join-Path $evidence "canvas_performance/$case"
        New-Item -ItemType Directory -Force -Path (Join-Path $directory 'src') | Out-Null
        $source=if($case -eq 'before'){Join-Path $evidence 'before/android_canvas.rs'}else{Join-Path $taskRoot 'native/gridtimer_native/src/android_canvas.rs'}
        Copy-Item -LiteralPath $source -Destination (Join-Path $directory 'src/android_canvas.rs') -Force
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'canvas_transfer_benchmark.rs') -Destination (Join-Path $directory 'src/main.rs') -Force
        Copy-Item -LiteralPath (Join-Path $taskRoot 'native/gridtimer_native/Cargo.lock') -Destination (Join-Path $directory 'Cargo.lock') -Force
        @'
[package]
name = "tenfold_canvas_transfer_benchmark"
version = "0.0.1"
edition = "2021"
[features]
optimized = []
[dependencies]
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.145"
sha2 = "0.10.8"
rand = "0.8.5"
jni = "0.21.1"
'@ | Set-Content -LiteralPath (Join-Path $directory 'Cargo.toml') -Encoding utf8
        $arguments=@('run','--offline','--release','--manifest-path',(Join-Path $directory 'Cargo.toml'))
        if($case -eq 'after'){$arguments+=@('--features','optimized')}
        $arguments+=@('--',(Join-Path $directory 'fixture'))
        & cargo @arguments 1> (Join-Path $directory 'result.json') 2> (Join-Path $directory 'run.log')
        if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'run.log') -Tail 25; throw "Benchmark failed: $case"}
        $result=Get-Content (Join-Path $directory 'result.json') -Raw | ConvertFrom-Json
        if(!$result.passed -or $result.panSamples.Count -ne 30){throw 'Incomplete benchmark'}
        $times=@($result.panSamples.elapsedMicros | Sort-Object)
        $results += [pscustomobject]@{case=$case;sourceSha256=(Get-FileHash -LiteralPath $source).Hash;responseBytes=($result.panSamples.responseBytes | Measure-Object -Average).Average;allocationCalls=($result.panSamples.allocationCalls | Measure-Object -Average).Average;allocatedBytes=($result.panSamples.allocatedBytes | Measure-Object -Average).Average;medianMicros=$times[15];p95Micros=$times[28];open=$result.open}
        Write-Output "$case canvas benchmark passed"
    }
} finally {foreach($key in $previous.Keys){[Environment]::SetEnvironmentVariable($key,$previous[$key],'Process')}}
[pscustomobject]@{passed=$true;hostOnly=$true;version=$version;nodes=1000;edges=4000;iterations=30;results=$results;completed=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $evidence 'canvas_performance/comparison.json') -Encoding utf8
$results | Select-Object case,responseBytes,allocationCalls,allocatedBytes,medianMicros,p95Micros | Format-Table
