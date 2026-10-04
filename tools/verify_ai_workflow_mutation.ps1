# v0.0.5 - Verify direct questions never read sources and knowledge requires them.
# v0.0.4 - Verify keyboard requests wait for a focused and resumed window.
# v0.0.3 - Verify the exact busy failure envelope without bypassing identity checks.
# v0.0.2 - Count actual tests and kill the pending-dispatch cancellation mutation.
# v0.0.1 - Verify Android AI authorization and unbound failure state with real JVM models.
[CmdletBinding()]
param(
    [string]$Version = '2.23.2.5',
    [switch]$RebindUnchangedBodies
)
$ErrorActionPreference = 'Stop'
$workflowRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android mutation evidence version' }
$workflowEvidence = Join-Path $workflowRoot "release_artifacts/verification/v$Version/ai_workflow_mutation"
$workflowSources = @(
    @{ path='native/gridtimer_native/src/sourcegen/android_ai_workflow.rs'; production='CONTENTS'; tests='TEST_CONTENTS' },
    @{ path='native/gridtimer_native/src/sourcegen/android_sync_failure.rs'; production='HELPER_CONTENTS'; tests='TEST_CONTENTS' }
)
$workflowUtf8 = [Text.UTF8Encoding]::new($false)
function Get-WorkflowLiteral([string]$Path, [string]$Name) {
    $raw = [IO.File]::ReadAllText($Path)
    $marker = [regex]::Match($raw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if (!$marker.Success) { throw "Missing actual Rust literal $Name in $Path" }
    $from = $marker.Index + $marker.Length
    $to = $raw.IndexOf('"' + $marker.Groups['hash'].Value + ';', $from)
    if ($to -lt $from) { throw "Missing Rust literal terminator: $Name" }
    return $raw.Substring($from, $to - $from).Replace("`r`n", "`n")
}
function Get-WorkflowTextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Write-WorkflowJson([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 15), $workflowUtf8)
}
$workflowInputs = foreach ($definition in $workflowSources) {
    $absolute = Join-Path $workflowRoot $definition.path
    $production = Get-WorkflowLiteral $absolute $definition.production
    $tests = Get-WorkflowLiteral $absolute $definition.tests
    [pscustomobject]@{
        path=$definition.path; absolutePath=$absolute
        sha256Before=(Get-FileHash -LiteralPath $absolute).Hash.ToLowerInvariant()
        production=$production; tests=$tests
        productionLiteral=$definition.production; testsLiteral=$definition.tests
        productionSha256=(Get-WorkflowTextHash $production); testsSha256=(Get-WorkflowTextHash $tests)
    }
}
$workflowKnowledgeCount = ([regex]'@Test\s+fun ').Matches($workflowInputs[0].tests).Count
$workflowSyncCount = ([regex]'@Test\s+fun ').Matches($workflowInputs[1].tests).Count
$workflowTestCount = $workflowKnowledgeCount + $workflowSyncCount
$workflowCaseNames = @('baseline', 'cosmetic', 'removed_authorization', 'removed_pending_dispatch', 'removed_sync_boundary', 'restored')
if ($workflowKnowledgeCount -lt 1 -or $workflowSyncCount -lt 1) { throw 'Both actual AI and sync boundary suites must contain tests' }
if ([version]$Version -ge [version]'2.23.2.4') {
    if ($workflowKnowledgeCount -lt 10) { throw 'Keyboard request business state tests are missing' }
    $workflowCaseNames = @('baseline', 'cosmetic', 'removed_authorization', 'removed_pending_dispatch', 'removed_keyboard_focus', 'removed_sync_boundary', 'restored')
}

if ([version]$Version -ge [version]'2.23.2.5') {
    if ($workflowKnowledgeCount -lt 14) { throw 'Direct and knowledge mode business state tests are missing' }
    $workflowCaseNames = @('baseline', 'cosmetic', 'removed_authorization', 'removed_pending_dispatch', 'removed_keyboard_focus', 'removed_direct_source_isolation', 'removed_knowledge_source_requirement', 'removed_sync_boundary', 'restored')
}

if ($RebindUnchangedBodies) {
    $priorReceiptPath = Join-Path $workflowEvidence 'receipt.json'
    $priorReceipt = Get-Content -LiteralPath $priorReceiptPath -Raw | ConvertFrom-Json
    if (!$priorReceipt.passed -or !$priorReceipt.productionUnchanged -or $priorReceipt.tests -ne $workflowTestCount -or $priorReceipt.knowledgeTests -ne $workflowKnowledgeCount -or $priorReceipt.syncFailureTests -ne $workflowSyncCount -or $priorReceipt.cases.Count -ne $workflowCaseNames.Count -or @($priorReceipt.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0}).Count) { throw 'No complete passed mutation receipt to rebind' }
    foreach ($input in $workflowInputs) {
        $prior = @($priorReceipt.sourceFiles | Where-Object path -eq $input.path)
        if ($prior.Count -ne 1 -or $prior[0].productionSha256 -ne $input.productionSha256 -or $prior[0].testsSha256 -ne $input.testsSha256) { throw "Actual tested controller or tests changed: $($input.path)" }
    }
    Write-WorkflowJson (Join-Path $workflowEvidence 'final_source_identity.json') ([ordered]@{
        passed=$true; testedControllerAndTestsUnchanged=$true; version=$Version
        receiptSha256=(Get-FileHash -LiteralPath $priorReceiptPath).Hash.ToLowerInvariant()
        sourceFiles=@($workflowInputs | ForEach-Object {[ordered]@{path=$_.path;sha256=$_.sha256Before;productionSha256=$_.productionSha256;testsSha256=$_.testsSha256}})
        reason='Only the surrounding Rust renderer changed; production helpers and their executed JVM tests are byte-identical after newline normalization.'
        checkedAt=[DateTimeOffset]::Now.ToString('o')
    })
    Write-Output 'Final source identity verified against the executed production helpers and tests'
    return
}

$workflowPrior = Get-Content -LiteralPath (Join-Path $workflowRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$workflowCompiled = Join-Path $workflowRoot 'app/build/tmp/kotlin-classes/release'
$workflowModelFiles = @(
    'com/ofairyo/gridtimer/data/SyncAccountSession.class',
    'com/ofairyo/gridtimer/data/SyncNetworkResult.class',
    'com/ofairyo/gridtimer/data/WorkspaceIdentityRebindContract.class',
    'com/ofairyo/gridtimer/data/ModelsKt.class'
)
$workflowModels = foreach ($relative in $workflowModelFiles) {
    $absolute = Join-Path $workflowCompiled $relative
    if (!(Test-Path -LiteralPath $absolute -PathType Leaf)) { throw "Real compiled release model is missing: $relative" }
    [ordered]@{path=$absolute;sha256=(Get-FileHash -LiteralPath $absolute).Hash.ToLowerInvariant()}
}
$workflowCompiler = @($workflowPrior.compiler)
foreach ($entry in $workflowCompiler) { if (!(Test-Path -LiteralPath $entry -PathType Leaf)) { throw "Installed compiler dependency missing: $entry" } }
$workflowRuntime = @($workflowCompiled) + @($workflowPrior.runtime | ForEach-Object {
    if ($_ -match '^(.*?)\\app\\build\\(.*)$') { Join-Path $workflowRoot ('app/build/' + $Matches[2]) } else { $_ }
} | Where-Object { (Test-Path -LiteralPath $_ -PathType Leaf) -and $_.EndsWith('.jar') })
if (!($workflowRuntime | Where-Object { $_ -match 'junit-4\.13\.2\.jar$' })) { throw 'JUnit 4.13.2 dependency is missing' }
$workflowJava = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
if (!(Test-Path -LiteralPath $workflowJava -PathType Leaf)) { throw 'Java 17 runtime is missing' }
$workflowTestClasses = @('com.ofairyo.gridtimer.ui.KnowledgeAiRequestBoundaryTest', 'com.ofairyo.gridtimer.data.UnboundSyncFailureTest')
$workflowResults = @()
foreach ($caseName in $workflowCaseNames) {
    $directory = Join-Path $workflowEvidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $knowledge = $workflowInputs[0].production
    $sync = $workflowInputs[1].production
    $failureTest = ''
    if ($caseName -eq 'cosmetic') {
        $knowledge = $knowledge.Replace('AI 请求未完成，请检查连接后重试。', '请求暂未完成，请稍后再试。')
        $sync = $sync.Replace('本次同步未完成，请稍后重试；本机数据已保留', '同步暂未完成，本机内容仍已保留。')
        if ($knowledge -ceq $workflowInputs[0].production -or $sync -ceq $workflowInputs[1].production) { throw 'Both actual helper wording changes must occur' }
    } elseif ($caseName -eq 'removed_authorization') {
        $anchor = ' || !authorized'
        if ([regex]::Matches($knowledge, [regex]::Escape($anchor)).Count -ne 1) { throw 'Knowledge authorization guard is not unique' }
        $knowledge = $knowledge.Replace($anchor, '')
        $failureTest = 'noAuthorizationOrInputCannotSend'
    } elseif ($caseName -eq 'removed_pending_dispatch') {
        $anchor = ' && generation == ticket'
        if ([regex]::Matches($knowledge, [regex]::Escape($anchor)).Count -ne 1) { throw 'Pending dispatch generation guard is not unique' }
        $knowledge = $knowledge.Replace($anchor, '')
        $failureTest = 'cancelledQueueCannotStartTransport'
    } elseif ($caseName -eq 'removed_keyboard_focus') {
        $anchor = ' || !windowFocused'
        if ([regex]::Matches($knowledge, [regex]::Escape($anchor)).Count -ne 1) { throw 'Keyboard window focus guard is not unique' }
        $knowledge = $knowledge.Replace($anchor, '')
        $failureTest = 'keyboardWaitsForFocusedResumedWindow'
    } elseif ($caseName -eq 'removed_direct_source_isolation') {
        $anchor = 'if (mode == KnowledgeAiMode.DIRECT) emptyList() else loadSources()'
        if ([regex]::Matches($knowledge, [regex]::Escape($anchor)).Count -ne 1) { throw 'Direct source isolation guard is not unique' }
        $knowledge = $knowledge.Replace($anchor, 'loadSources()')
        $failureTest = 'directModeDoesNotReadOrIncludeKnowledgeSources'
    } elseif ($caseName -eq 'removed_knowledge_source_requirement') {
        $anchor = '(mode == KnowledgeAiMode.KNOWLEDGE && sourceCount <= 0) ||'
        if ([regex]::Matches($knowledge, [regex]::Escape($anchor)).Count -ne 1) { throw 'Knowledge source requirement is not unique' }
        $knowledge = $knowledge.Replace($anchor, '')
        $failureTest = 'switchingBackDoesNotReviveOldModeOrBypassKnowledgeSources'
    } elseif ($caseName -eq 'removed_sync_boundary') {
        $anchor = 'if (!pure && !busy) return null'
        if ([regex]::Matches($sync, [regex]::Escape($anchor)).Count -ne 1) { throw 'Pure sync failure state guard is not unique' }
        $sync = $sync.Replace($anchor, '')
        $failureTest = 'missingResponseAndSuccessfulResponseCannotUseFailurePath'
    }
    $caseSources = @(
        @{ name='KnowledgeAiRequestBoundary.kt'; body=$knowledge },
        @{ name='KnowledgeAiRequestBoundaryTest.kt'; body=$workflowInputs[0].tests },
        @{ name='UnboundSyncFailure.kt'; body=("package com.ofairyo.gridtimer.data`n" + $sync) },
        @{ name='UnboundSyncFailureTest.kt'; body=$workflowInputs[1].tests }
    ) | ForEach-Object {
        $file = Join-Path $directory $_.name
        [IO.File]::WriteAllText($file, $_.body, $workflowUtf8)
        $file
    }
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $workflowJava -Xmx512m -cp ($workflowCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$workflowCompiled" -classpath ($workflowRuntime -join ';') -d $classes @caseSources *> (Join-Path $directory 'compile.log')
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath (Join-Path $directory 'compile.log') -Tail 35; throw "Mutation compilation failed, not an accepted business failure: $caseName" }
    & $workflowJava -Xmx512m -cp ($classes + ';' + ($workflowRuntime -join ';')) org.junit.runner.JUnitCore @workflowTestClasses *> (Join-Path $directory 'tests.log')
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $expectedPass = $caseName -in @('baseline', 'cosmetic', 'restored')
    $passed = if ($expectedPass) {
        $testExit -eq 0 -and $log.Contains("OK ($workflowTestCount tests)")
    } else {
        $testExit -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains($failureTest) -and $log.Contains("Tests run: $workflowTestCount,")
    }
    $workflowResults += [ordered]@{name=$caseName;compileExit=$compileExit;testExit=$testExit;expectedPass=$expectedPass;requiredFailureTest=$failureTest;passed=$passed;knowledgeSourceSha256=(Get-WorkflowTextHash $knowledge);syncSourceSha256=(Get-WorkflowTextHash $sync)}
    if (!$passed) { Get-Content -LiteralPath (Join-Path $directory 'tests.log') -Tail 50; throw "Unexpected mutation test result: $caseName" }
    Write-Output "$caseName verified: $workflowTestCount tests, compile $compileExit, test $testExit"
}
$workflowFinalSources = foreach ($input in $workflowInputs) {
    $after = (Get-FileHash -LiteralPath $input.absolutePath).Hash.ToLowerInvariant()
    if ($after -ne $input.sha256Before) { throw "Production source changed during mutation checks: $($input.path)" }
    [ordered]@{path=$input.path;sha256Before=$input.sha256Before;sha256After=$after;productionLiteral=$input.productionLiteral;testsLiteral=$input.testsLiteral;productionSha256=$input.productionSha256;testsSha256=$input.testsSha256}
}
foreach ($model in $workflowModels) {
    if ((Get-FileHash -LiteralPath $model.path).Hash.ToLowerInvariant() -ne $model.sha256) { throw 'Compiled model changed during JVM mutation verification' }
}
Write-WorkflowJson (Join-Path $workflowEvidence 'receipt.json') ([ordered]@{
    passed=$true;version=$Version;tests=$workflowTestCount;knowledgeTests=$workflowKnowledgeCount;syncFailureTests=$workflowSyncCount;productionUnchanged=$true
    hostOnly=$true;networkRequests=0;noDeviceOperations=$true;sourceFiles=$workflowFinalSources;cases=$workflowResults
    compiledModels=$workflowModels;scope='Actual Rust-owned helpers and tests with real release model classes; no production source mutation or model stubs.'
    checkedAt=[DateTimeOffset]::Now.ToString('o')
})
Write-Output "Android $Version AI workflow and sync boundary mutation acceptance completed"
