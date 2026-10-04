# v2.23.2.6 - Verify offline formula rendering and rejection of answer HTML.
[CmdletBinding()]
param([string]$Version='2.23.2.6')
$ErrorActionPreference='Stop'
$mathRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if($Version -notmatch '^\d+\.\d+\.\d+\.\d+$'){throw 'Invalid release evidence version'}
$mathRelativeSource='native/gridtimer_native/src/android_answer_render.rs'
$mathSourcePath=Join-Path $mathRoot $mathRelativeSource
$mathReceiptDir=Join-Path $mathRoot "release_artifacts/verification/v$Version/math_render_mutation"
New-Item -ItemType Directory -Force -Path $mathReceiptDir | Out-Null
$mathOriginal=[IO.File]::ReadAllBytes($mathSourcePath)
$mathOriginalText=[Text.Encoding]::UTF8.GetString($mathOriginal).Replace("`r`n","`n")
$mathBefore=(Get-FileHash -LiteralPath $mathSourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$mathTestNames=@(([regex]'(?m)#\[test\]\s*fn\s+(?<name>[A-Za-z0-9_]+)\s*\(').Matches($mathOriginalText) | ForEach-Object { $_.Groups['name'].Value })
if($mathTestNames.Count -lt 5){throw 'Renderer business test coverage is missing'}
$mathRequiredFailure='android_math_render_rejects_answer_html_and_external_resources'
if($mathRequiredFailure -notin $mathTestNames){throw 'Renderer answer HTML safety test is missing'}
$mathEnvironmentNames=@('RUSTUP_TOOLCHAIN','LIB','CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER','CARGO_TARGET_DIR','CARGO_INCREMENTAL')
$mathOldEnvironment=@{}
foreach($name in $mathEnvironmentNames){$mathOldEnvironment[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
function Write-MathSourceBytes([byte[]]$Bytes){
    $temporary=$mathSourcePath+'.math-test-mutation.tmp'
    if(-not ([IO.Path]::GetFullPath($temporary)).StartsWith($mathRoot+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Renderer mutation target is outside the project'}
    [IO.File]::WriteAllBytes($temporary,$Bytes)
    [IO.File]::Move($temporary,$mathSourcePath,$true)
}
function Invoke-MathCase([string]$Name,[string]$Kind,[string]$Text,[bool]$ExpectFailure,[string]$RequiredFailure=''){
    Write-MathSourceBytes ([Text.UTF8Encoding]::new($false).GetBytes($Text))
    $compileLog=Join-Path $mathReceiptDir ($Name+'_compile.jsonl')
    & $mathCargo test --manifest-path (Join-Path $mathRoot 'native/gridtimer_native/Cargo.toml') --locked --offline --lib --no-run --message-format=json 1> $compileLog 2> (Join-Path $mathReceiptDir ($Name+'_compile.log'))
    $compileExit=$LASTEXITCODE
    $testExit=$null
    $requiredObserved=$false
    $allTestsObserved=$false
    $testLogSha256=$null
    if($compileExit -eq 0){
        $artifact=Get-Content -LiteralPath $compileLog | ForEach-Object {
            try{$item=$_ | ConvertFrom-Json;if($item.reason -eq 'compiler-artifact' -and $item.executable -and $item.target.name -eq 'gridtimer_native'){$item.executable}}catch{}
        } | Select-Object -Last 1
        if(!$artifact){throw "No compiled renderer test executable for $Name"}
        $testLog=Join-Path $mathReceiptDir ($Name+'_tests.log')
        & $artifact 'android_answer_render::tests' '--test-threads=1' *> $testLog
        $testExit=$LASTEXITCODE
        $testText=Get-Content -LiteralPath $testLog -Raw
        $testLogSha256=(Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant()
        $allTestsObserved=$true
        foreach($testName in $mathTestNames){
            if($testText -notmatch ('(?m)^test android_answer_render::tests::'+[regex]::Escape($testName)+' \.\.\. (ok|FAILED)\s*$')){$allTestsObserved=$false}
        }
        if($RequiredFailure){$requiredObserved=$testText -match ('(?m)^test android_answer_render::tests::'+[regex]::Escape($RequiredFailure)+' \.\.\. FAILED\s*$')}
    }
    $passed=$compileExit -eq 0 -and $allTestsObserved -and $null -ne $testExit -and (($ExpectFailure -and $testExit -ne 0 -and $requiredObserved) -or (!$ExpectFailure -and $testExit -eq 0))
    Write-Host "Renderer mutation $Name compile=$compileExit test=$testExit passed=$passed"
    return [ordered]@{name=$Name;kind=$Kind;compileExit=$compileExit;testExit=$testExit;requiredFailureTest=$RequiredFailure;requiredFailureObserved=$requiredObserved;allBusinessTestsObserved=$allTestsObserved;testLogSha256=$testLogSha256;passed=$passed}
}
$mathCases=@()
try{
    $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
    $mathRustc=(& rustup which --toolchain $env:RUSTUP_TOOLCHAIN rustc).Trim()
    if($LASTEXITCODE -ne 0){throw 'Rust toolchain is unavailable'}
    $mathToolchain=Split-Path (Split-Path $mathRustc)
    $mathCargo=Join-Path $mathToolchain 'bin/cargo.exe'
    $env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path $mathToolchain 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
    $env:CARGO_TARGET_DIR='C:\gt\gridtimer-build\sourcegen'
    $env:CARGO_INCREMENTAL='0'
    $mathCases+=Invoke-MathCase 'baseline' 'baseline' $mathOriginalText $false
    $cosmeticBefore='color:#24272a;font:16px/1.75'
    $cosmeticAfter='color:#26313a;font:16px/1.75'
    if([regex]::Matches($mathOriginalText,[regex]::Escape($cosmeticBefore)).Count -ne 1){throw 'Unique renderer cosmetic mutation marker is missing'}
    $mathCases+=Invoke-MathCase 'cosmetic' 'cosmetic' ($mathOriginalText.Replace($cosmeticBefore,$cosmeticAfter)) $false
    $rawHtmlGuard='        Event::Html(text) | Event::InlineHtml(text) => {'+[char]10+'            text_with_formulas(&text, &prefix, &formulas)'+[char]10+'        }'
    $androidBodyMarker='fn render_answer_body(content: &str) -> String {'
    $windowsBodyMarker='pub fn render_windows_knowledge_math_body(content: &str) -> Option<String> {'
    if([regex]::Matches($mathOriginalText,[regex]::Escape($androidBodyMarker)).Count -ne 1 -or [regex]::Matches($mathOriginalText,[regex]::Escape($windowsBodyMarker)).Count -ne 1){throw 'Renderer mutation function boundaries are missing or ambiguous'}
    $androidBodyStart=$mathOriginalText.IndexOf($androidBodyMarker)
    $windowsBodyStart=$mathOriginalText.IndexOf($windowsBodyMarker)
    if($windowsBodyStart -le $androidBodyStart){throw 'Renderer mutation function boundaries are out of order'}
    $androidBody=$mathOriginalText.Substring($androidBodyStart,$windowsBodyStart-$androidBodyStart)
    if([regex]::Matches($androidBody,[regex]::Escape($rawHtmlGuard)).Count -ne 1){throw 'Unique Android renderer raw HTML guard is missing'}
    $guardPosition=$androidBodyStart+$androidBody.IndexOf($rawHtmlGuard)
    $removedGuard='        Event::Html(text) | Event::InlineHtml(text) => vec![Event::Html(text)],'
    $androidGuardRemoved=$mathOriginalText.Substring(0,$guardPosition)+$removedGuard+$mathOriginalText.Substring($guardPosition+$rawHtmlGuard.Length)
    $mathCases+=Invoke-MathCase 'missing_raw_html_guard' 'raw_html_guard_removed' $androidGuardRemoved $true $mathRequiredFailure
}finally{
    Write-MathSourceBytes $mathOriginal
    foreach($name in $mathEnvironmentNames){[Environment]::SetEnvironmentVariable($name,$mathOldEnvironment[$name],'Process')}
}
# Run the restored source with the same explicit toolchain and environment.
try{
    $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
    $env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path $mathToolchain 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
    $env:CARGO_TARGET_DIR='C:\gt\gridtimer-build\sourcegen'
    $env:CARGO_INCREMENTAL='0'
    $mathCases+=Invoke-MathCase 'restored' 'restored' $mathOriginalText $false
}finally{
    Write-MathSourceBytes $mathOriginal
    foreach($name in $mathEnvironmentNames){[Environment]::SetEnvironmentVariable($name,$mathOldEnvironment[$name],'Process')}
}
$mathAfter=(Get-FileHash -LiteralPath $mathSourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$mathPassed=$mathBefore -ceq $mathAfter -and $mathCases.Count -eq 4 -and @($mathCases | Where-Object {!$_.passed}).Count -eq 0
$mathReceipt=[ordered]@{passed=$mathPassed;version=$Version;sourcePath=$mathRelativeSource;sourceSha256Before=$mathBefore;sourceSha256After=$mathAfter;productionUnchanged=($mathBefore -ceq $mathAfter);verifierPath='tools/verify_android_math_mutation.ps1';verifierSha256=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant();mutationTarget='render_answer_body';tests=$mathTestNames.Count;testNames=$mathTestNames;cases=$mathCases;networkRequests=0;noDeviceOperations=$true;checkedAt=[DateTimeOffset]::Now.ToString('o')}
[IO.File]::WriteAllText((Join-Path $mathReceiptDir 'receipt.json'),($mathReceipt | ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
if(!$mathPassed){throw 'Android formula renderer mutation acceptance failed'}
Write-Output "Android formula renderer mutation acceptance completed: $Version"
