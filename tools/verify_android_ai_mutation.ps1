# v2.23.2.5 - Verify Android direct question isolation and native response guards.
[CmdletBinding()]
param([string]$Version='2.23.2.5',[switch]$VerifyExistingFailureIdentity)
$ErrorActionPreference='Stop'
$mutationRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if($Version -notmatch '^\d+\.\d+\.\d+\.\d+$'){throw 'Invalid release evidence version'}
$sourcePath=Join-Path $mutationRoot 'native/gridtimer_native/src/ai_client.rs'
$receiptDir=Join-Path $mutationRoot "release_artifacts/verification/v$Version/ai_connection_mutation"
New-Item -ItemType Directory -Path $receiptDir -Force | Out-Null
if($VerifyExistingFailureIdentity){
    $receiptPath=Join-Path $receiptDir 'receipt.json'
    $receipt=Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    $sourceHash=(Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if(!$receipt.passed -or !$receipt.productionUnchanged -or $receipt.sourceSha256Before -cne $sourceHash -or $receipt.sourceSha256After -cne $sourceHash -or $receipt.cases.Count -ne 6 -or @($receipt.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0}).Count){throw 'Executed native mutation receipt does not bind the current source'}
    $requiredCases=@(
        @{name='missing_key_guard';test='connection_probe_invalid_configuration_sends_nothing'},
        @{name='missing_completion_guard';test='response_answer_requires_completion_even_when_partial_text_is_available'},
        @{name='missing_direct_mode_guard';test='android_direct_query_accepts_zero_sources_without_local_answer'}
    )
    foreach($definition in $requiredCases){
        $case=@($receipt.cases | Where-Object name -eq $definition.name)
        $logPath=Join-Path $receiptDir ($definition.name+'_tests.log')
        $log=Get-Content -LiteralPath $logPath -Raw
        if($case.Count -ne 1 -or $null -eq $case[0].testExit -or $case[0].testExit -eq 0 -or $log -notmatch ('(?m)^test ai_client::tests::'+[regex]::Escape($definition.test)+' \.\.\. FAILED')){throw ('Required native business test did not reject mutation: '+$definition.name)}
        $case[0].requiredFailureTest=$definition.test
        $case[0].requiredFailureObserved=$true
        $case[0] | Add-Member -NotePropertyName testLogSha256 -NotePropertyValue (Get-FileHash -LiteralPath $logPath -Algorithm SHA256).Hash.ToLowerInvariant() -Force
    }
    $rawPath=Join-Path $receiptDir 'raw_run_receipt.json'
    if(!(Test-Path -LiteralPath $rawPath)){Copy-Item -LiteralPath $receiptPath -Destination $rawPath}
    $receipt | Add-Member -NotePropertyName failureIdentityValidatedAt -NotePropertyValue ([DateTimeOffset]::Now.ToString('o')) -Force
    $receipt | Add-Member -NotePropertyName failureIdentityScope -NotePropertyValue 'Verified exact failing business tests in the executed mutation logs; no test rerun or source modification.' -Force
    $receipt | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $receiptPath -Encoding utf8
    Write-Output 'Executed native failure identities verified against current source'
    return
}
$original=[IO.File]::ReadAllBytes($sourcePath)
$originalText=[Text.Encoding]::UTF8.GetString($original).Replace("`r`n","`n")
$before=(Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-msvc'
$rustc=(& rustup which rustc).Trim()
$env:LIB='C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=Join-Path (Split-Path (Split-Path $rustc)) 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$env:CARGO_TARGET_DIR='C:\gt\gridtimer-build\sourcegen'
$env:CARGO_INCREMENTAL='0'
function Write-SourceBytes([byte[]]$Bytes){
    $temporary=$sourcePath+'.ai-test-mutation.tmp'
    if(-not ([IO.Path]::GetFullPath($temporary)).StartsWith($mutationRoot+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Source mutation outside project'}
    [IO.File]::WriteAllBytes($temporary,$Bytes)
    [IO.File]::Move($temporary,$sourcePath,$true)
}
function Invoke-Case([string]$Name,[string]$Kind,[string]$Text,[bool]$ExpectFailure,[string]$RequiredFailure=''){
    Write-SourceBytes ([Text.UTF8Encoding]::new($false).GetBytes($Text))
    $compileLog=Join-Path $receiptDir ($Name+'_compile.jsonl')
    & cargo test --manifest-path (Join-Path $mutationRoot 'native/gridtimer_native/Cargo.toml') --locked --offline --lib --no-run --message-format=json 1> $compileLog 2> (Join-Path $receiptDir ($Name+'_compile.log'))
    $compileExit=$LASTEXITCODE
    $testExit=$null
    $requiredFailed=$false
    if($compileExit -eq 0){
        $artifact=Get-Content -LiteralPath $compileLog | ForEach-Object {
            try{$item=$_ | ConvertFrom-Json;if($item.reason -eq 'compiler-artifact' -and $item.executable -and $item.target.name -eq 'gridtimer_native'){$item.executable}}catch{}
        } | Select-Object -Last 1
        if(-not $artifact){throw "No compiled native test executable for $Name"}
        $testLog=Join-Path $receiptDir ($Name+'_tests.log')
        & $artifact 'ai_client::tests' '--test-threads=1' *> $testLog
        $testExit=$LASTEXITCODE
        if($RequiredFailure){$requiredFailed=(Get-Content -LiteralPath $testLog -Raw) -match ('(?m)^test ai_client::tests::'+[regex]::Escape($RequiredFailure)+' \.\.\. FAILED')}
    }
    $passed=$compileExit -eq 0 -and $null -ne $testExit -and (($ExpectFailure -and $testExit -ne 0 -and (!$RequiredFailure -or $requiredFailed)) -or (!$ExpectFailure -and $testExit -eq 0))
    Write-Host "$Name compile=$compileExit test=$testExit passed=$passed"
    return [ordered]@{name=$Name;kind=$Kind;compileExit=$compileExit;testExit=$testExit;requiredFailureTest=$RequiredFailure;requiredFailureObserved=$requiredFailed;passed=$passed}
}
$cases=@()
try{
    $cases+=Invoke-Case 'baseline' 'baseline' $originalText $false
    $cosmetic=$originalText.Replace('连接测试通过，已验证结构化输出和测试图片识别','测试完成，结构化输出与图片识别均通过')
    if($cosmetic -ceq $originalText){throw 'Cosmetic marker missing'}
    $cases+=Invoke-Case 'cosmetic' 'cosmetic' $cosmetic $false
    $guard='    if api_key.is_empty() {'+[char]10+'        result.message = "请先填写 AI API Key".to_string();'+[char]10+'        return result;'+[char]10+'    }'
    if(-not $originalText.Contains($guard)){throw 'Probe empty-key guard missing'}
    $cases+=Invoke-Case 'missing_key_guard' 'authorization_guard_removed' ($originalText.Replace($guard,'    // Test mutation: remove empty-key guard.')) $true 'connection_probe_invalid_configuration_sends_nothing'
    $completionGuard='    if value.get("status").and_then(Value::as_str) != Some("completed") {'
    if(-not $originalText.Contains($completionGuard)){throw 'Completion guard missing'}
    $cases+=Invoke-Case 'missing_completion_guard' 'completion_guard_removed' ($originalText.Replace($completionGuard,'    if false { // Test mutation: remove completion guard.')) $true 'response_answer_requires_completion_even_when_partial_text_is_available'
    $directGuard='if mode == AndroidAiQueryMode::Direct {'
    if([regex]::Matches($originalText,[regex]::Escape($directGuard)).Count -ne 1){throw 'Direct question source-isolation guard is not unique'}
    $cases+=Invoke-Case 'missing_direct_mode_guard' 'direct_mode_guard_removed' ($originalText.Replace($directGuard,'if false { // Test mutation: remove direct source isolation.')) $true 'android_direct_query_accepts_zero_sources_without_local_answer'
}finally{Write-SourceBytes $original}
try{$cases+=Invoke-Case 'restored' 'restored' $originalText $false}finally{Write-SourceBytes $original}
$after=(Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$passed=$before -ceq $after -and $cases.Count -eq 6 -and @($cases | Where-Object {-not $_.passed}).Count -eq 0
$receipt=[ordered]@{passed=$passed;version=$Version;sourcePath='native/gridtimer_native/src/ai_client.rs';sourceSha256Before=$before;sourceSha256After=$after;productionUnchanged=($before -ceq $after);cases=$cases;networkRequests=0;noDeviceOperations=$true;checkedAt=[DateTimeOffset]::Now.ToString('o')}
$receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $receiptDir 'receipt.json') -Encoding utf8
if(-not $passed){throw 'Native Android AI mutation acceptance failed'}
Write-Output "Native Android AI mutation acceptance completed: $Version"
