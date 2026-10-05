# v2.23.2.11 - Gate finance scope isolation, preview mutations and packaged navigation policy.
# v2.23.2.10 - Gate document caret and knowledge header mutations and packaged editor policies.
# v2.23.2.9 - Bind legal workflow authorization mutations and generated actions to the signed APK.
# v2.23.2.8 - Gate legal request lifecycle tests, settlement mutation and packaged boundary.
# v2.23.2.7 - Gate current-document loading, formula completion and actual loading mutations.
# v2.23.2.6 - Verify offline formula assets, safe rendering and renderer boundaries.
# v2.23.2.5 - Require direct AI isolation, knowledge source guards and native bridge.
# v2.23.2.4 - Require keyboard focus state, mutation and packaged helper acceptance.
# v2.23.2.3 - Require real AI workflow state and mutation acceptance.
# v0.0.7 - Gate AI connection state, authorization mutation and packaged native bridge.
# v0.0.6 - Require exact-money and save-queue business and mutation acceptance.
# v0.0.5 - Permit the explicitly requested Android 2.23.2 transition.
# v0.0.4 - Verify a published APK after later Windows-only source changes.
# v0.0.3 - Verify that the Android update-history classes reach the signed APK.
# v0.0.2 - Permit the requested Android 2.23 to 2.23.1 transition while retaining Windows.
# v0.0.1 - Verify and publish the signed note editor release while retaining Windows.
[CmdletBinding()]
param([ValidateSet('publish','verify')][string]$Mode='verify')
$ErrorActionPreference='Stop'
$releaseRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$gradle=[IO.File]::ReadAllText((Join-Path $releaseRoot 'app/build.gradle'))
$releaseVersion=[regex]::Match($gradle,"versionName '([^']+)'").Groups[1].Value
$releaseCode=[int][regex]::Match($gradle,'versionCode (\d+)').Groups[1].Value
$evidence=Join-Path $releaseRoot "release_artifacts/verification/v$releaseVersion"
$name="tenfold_v$releaseVersion.apk"
$apk=Join-Path $releaseRoot "app/build/outputs/apk/release/$name"
$manifestPath=Join-Path $releaseRoot 'release_artifacts/current/release_manifest.json'
$beforeText=[IO.File]::ReadAllText($manifestPath)
$manifest=$beforeText | ConvertFrom-Json
$build=Get-Content (Join-Path $evidence 'offline_build_acceptance.json') -Raw | ConvertFrom-Json
if(!$build.passed -or $build.mode -ne 'build' -or $build.version -ne $releaseVersion -or $build.editingTests -lt 1){throw 'Formal Android build acceptance missing'}
$provenance=Get-Content (Join-Path $evidence 'android_source_provenance.json') -Raw | ConvertFrom-Json
if($provenance.sha256 -ne $build.sourceSnapshotSha256){throw 'Source snapshot mismatch'}
$aiAcceptance=$null
$mathAcceptance=$null
$legalAcceptance=$null
$financeAcceptance=$null
$legalWorkflowAcceptance=$null
if($releaseVersion -eq '2.23.2'){
    if($build.knowledgeRecoveryTests.NoteCollectionRecoveryTest -ne 6 -or $build.knowledgeRecoveryTests.KnowledgeFilterStateTest -ne 4){throw 'Knowledge recovery business acceptance missing'}
    $mutation=Get-Content -LiteralPath (Join-Path $evidence 'knowledge_recovery_mutation/mutation_result.json') -Raw | ConvertFrom-Json
    if(!$mutation.passed -or $mutation.tests -ne 10 -or !$mutation.productionUnchanged -or @($mutation.cases | Where-Object {!$_.passed}).Count -ne 0 -or $mutation.cases.Count -ne 4){throw 'Knowledge recovery mutation acceptance missing'}
}
if($releaseVersion -eq '2.23.2.1'){
    if($build.knowledgeRecoveryTests.NoteSaveQueueTest -ne 4 -or $build.knowledgeRecoveryTests.FinanceMoneyTest -ne 5){throw 'Save queue or exact money business acceptance missing'}
    $queue=Get-Content -LiteralPath (Join-Path $evidence 'note_save_queue_mutation/mutation_result.json') -Raw | ConvertFrom-Json
    $identity=Get-Content -LiteralPath (Join-Path $evidence 'note_save_queue_mutation/final_tested_body_identity.json') -Raw | ConvertFrom-Json
    $queueHash=(Get-FileHash -LiteralPath (Join-Path $releaseRoot 'native/gridtimer_native/src/sourcegen/android_note_save_queue.rs')).Hash
    if(!$queue.passed -or !$queue.productionUnchanged -or $queue.tests -ne 4 -or $queue.cases.Count -ne 5 -or @($queue.cases|Where-Object {!$_.passed}).Count -or !$identity.passed -or !$identity.testedControllerAndTestsUnchanged -or $identity.finalSourceSha256 -ne $queueHash){throw 'Save queue mutation or final source acceptance missing'}
    $money=Get-Content -LiteralPath (Join-Path $releaseRoot 'release_artifacts/verification/android_v2.23.2.1/money_precision_mutation/result.json') -Raw | ConvertFrom-Json
    $moneyHash=(Get-FileHash -LiteralPath (Join-Path $releaseRoot 'native/gridtimer_native/src/finance_money.rs')).Hash
    if($money.sourceHashAfterWhitespaceFormat -ne $moneyHash -or $money.results.Count -ne 3 -or @($money.results|Where-Object {$_.compileExit -ne 0}).Count -or $money.results[0].testExit -ne 0 -or $money.results[1].testExit -ne 0 -or $money.results[2].testExit -eq 0){throw 'Exact money mutation acceptance missing'}
    $guard=Get-Content -LiteralPath (Join-Path $evidence 'finance_guard_mutation/receipt.json') -Raw | ConvertFrom-Json
    $guardHash=(Get-FileHash -LiteralPath (Join-Path $releaseRoot 'native/gridtimer_native/src/finance_precision_guard.rs')).Hash
    if($guard.production_source_sha256_before -ne $guardHash -or $guard.production_source_sha256_after -ne $guardHash -or $guard.cases.Count -ne 4 -or @($guard.cases|Where-Object {!$_.pass}).Count){throw 'Precision persistence guard mutation acceptance missing'}
}
if([version]$releaseVersion -ge [version]'2.23.2.2'){
    if($build.aiConnectionTests -lt 7){throw 'AI connection business acceptance missing'}
    [xml]$aiTests=Get-Content -LiteralPath (Join-Path $evidence 'AiConnectionControllerTest.xml') -Raw
    if([int]$aiTests.testsuite.tests -ne [int]$build.aiConnectionTests -or [int]$aiTests.testsuite.failures -ne 0 -or [int]$aiTests.testsuite.errors -ne 0 -or [int]$aiTests.testsuite.skipped -ne 0){throw 'AI connection business tests are incomplete or failed'}
    $aiMutation=Get-Content -LiteralPath (Join-Path $evidence 'ai_connection_mutation/receipt.json') -Raw | ConvertFrom-Json
    $aiSourcePath='native/gridtimer_native/src/ai_client.rs'
    $aiSource=@($provenance.files | Where-Object path -eq $aiSourcePath)
    $aiCosmetic=@($aiMutation.cases | Where-Object kind -eq 'cosmetic')
    $aiAuthorization=@($aiMutation.cases | Where-Object kind -eq 'authorization_guard_removed')
    if(!$aiMutation.passed -or $aiMutation.sourcePath -ne $aiSourcePath -or $aiSource.Count -ne 1 -or $aiMutation.sourceSha256Before -ne $aiSource[0].sha256 -or $aiMutation.sourceSha256After -ne $aiSource[0].sha256 -or @($aiMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0}).Count -ne 0 -or $aiCosmetic.Count -ne 1 -or $aiCosmetic[0].testExit -ne 0 -or $aiAuthorization.Count -ne 1 -or $null -eq $aiAuthorization[0].testExit -or $aiAuthorization[0].testExit -eq 0){throw 'AI authorization mutation acceptance missing or unrelated to the built source'}
    if ([version]$releaseVersion -ge [version]'2.23.2.3') {
        $completedMutation=@($aiMutation.cases | Where-Object kind -eq 'completion_guard_removed')
        if ($completedMutation.Count -ne 1 -or $completedMutation[0].compileExit -ne 0 -or $null -eq $completedMutation[0].testExit -or $completedMutation[0].testExit -eq 0 -or !$completedMutation[0].passed) { throw 'Incomplete response completion mutation acceptance' }
    }
    $aiAcceptance=[ordered]@{businessTests=[int]$build.aiConnectionTests;mutationPassed=$true;mutationSourceSha256=$aiSource[0].sha256}
    if ([version]$releaseVersion -ge [version]'2.23.2.5') {
        foreach($expectedNativeCase in @(@{kind='authorization_guard_removed';test='connection_probe_invalid_configuration_sends_nothing'},@{kind='completion_guard_removed';test='response_answer_requires_completion_even_when_partial_text_is_available'})){
            $criticalCase=@($aiMutation.cases | Where-Object kind -eq $expectedNativeCase.kind)
            if($criticalCase.Count -ne 1 -or $criticalCase[0].requiredFailureTest -ne $expectedNativeCase.test -or !$criticalCase[0].requiredFailureObserved){throw ('Native mutation failure is not tied to required business test: '+$expectedNativeCase.kind)}
        }
        $directMutation=@($aiMutation.cases | Where-Object kind -eq 'direct_mode_guard_removed')
        if($directMutation.Count -ne 1 -or $directMutation[0].requiredFailureTest -ne 'android_direct_query_accepts_zero_sources_without_local_answer' -or !$directMutation[0].requiredFailureObserved -or !$directMutation[0].passed -or $directMutation[0].compileExit -ne 0 -or $null -eq $directMutation[0].testExit -or $directMutation[0].testExit -eq 0){throw 'Native direct AI source isolation mutation missing'}
        $aiAcceptance['directModeMutationPassed']=$true
    }
}
if ([version]$releaseVersion -ge [version]'2.23.2.3') {
    if ($build.aiWorkflowTests.KnowledgeAiRequestBoundaryTest -lt 6 -or $build.aiWorkflowTests.UnboundSyncFailureTest -ne 9) { throw 'AI request or sync failure business acceptance missing' }
    if ([version]$releaseVersion -ge [version]'2.23.2.4' -and $build.aiWorkflowTests.KnowledgeAiRequestBoundaryTest -lt 10) { throw 'Keyboard request business acceptance missing' }
    if ([version]$releaseVersion -ge [version]'2.23.2.5' -and $build.aiWorkflowTests.KnowledgeAiRequestBoundaryTest -lt 14) { throw 'AI mode state business acceptance missing' }
    foreach ($suite in @('KnowledgeAiRequestBoundaryTest','UnboundSyncFailureTest')) {
        [xml]$suiteResult = Get-Content -LiteralPath (Join-Path $evidence ($suite+'.xml')) -Raw
        if ([int]$suiteResult.testsuite.tests -ne [int]$build.aiWorkflowTests.$suite -or [int]$suiteResult.testsuite.failures -ne 0 -or [int]$suiteResult.testsuite.errors -ne 0 -or [int]$suiteResult.testsuite.skipped -ne 0) { throw ('AI workflow tests failed: '+$suite) }
    }
    $workflowReceiptPath = Join-Path $evidence 'ai_workflow_mutation/receipt.json'
    $workflowMutation = Get-Content -LiteralPath $workflowReceiptPath -Raw | ConvertFrom-Json
    $workflowIdentity = Get-Content -LiteralPath (Join-Path $evidence 'ai_workflow_mutation/final_source_identity.json') -Raw | ConvertFrom-Json
    if (!$workflowMutation.passed -or !$workflowMutation.productionUnchanged -or $workflowMutation.tests -ne ([int]$build.aiWorkflowTests.KnowledgeAiRequestBoundaryTest+[int]$build.aiWorkflowTests.UnboundSyncFailureTest) -or !$workflowIdentity.testedControllerAndTestsUnchanged -or $workflowIdentity.receiptSha256 -ne (Get-FileHash -LiteralPath $workflowReceiptPath).Hash -or @($workflowMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0}).Count -ne 0) { throw 'AI workflow mutation or final body identity failed' }
    foreach ($caseName in @('baseline','cosmetic','restored')) {
        $case=@($workflowMutation.cases | Where-Object name -eq $caseName)
        if ($case.Count -ne 1 -or $case[0].testExit -ne 0) { throw ('AI workflow cosmetic or baseline acceptance missing: '+$caseName) }
    }
    foreach ($caseName in @('removed_authorization','removed_sync_boundary','removed_pending_dispatch')) {
        $case=@($workflowMutation.cases | Where-Object name -eq $caseName)
        if ($case.Count -ne 1 -or $null -eq $case[0].testExit -or $case[0].testExit -eq 0) { throw ('AI workflow critical mutation was not detected: '+$caseName) }
    }
    if ([version]$releaseVersion -ge [version]'2.23.2.4') {
        $keyboardMutation=@($workflowMutation.cases | Where-Object name -eq 'removed_keyboard_focus')
        if ($keyboardMutation.Count -ne 1 -or $keyboardMutation[0].requiredFailureTest -ne 'keyboardWaitsForFocusedResumedWindow' -or $null -eq $keyboardMutation[0].testExit -or $keyboardMutation[0].testExit -eq 0 -or !$keyboardMutation[0].passed) { throw 'Keyboard focus mutation was not detected by the required business state test' }
        $aiAcceptance['keyboardFocusMutationPassed']=$true
    }
    if ([version]$releaseVersion -ge [version]'2.23.2.5') {
        foreach ($modeCase in @(@{name='removed_direct_source_isolation';test='directModeDoesNotReadOrIncludeKnowledgeSources'},@{name='removed_knowledge_source_requirement';test='switchingBackDoesNotReviveOldModeOrBypassKnowledgeSources'})) {
            $modeMutation=@($workflowMutation.cases | Where-Object name -eq $modeCase.name)
            if($modeMutation.Count -ne 1 -or $modeMutation[0].requiredFailureTest -ne $modeCase.test -or !$modeMutation[0].passed -or $null -eq $modeMutation[0].testExit -or $modeMutation[0].testExit -eq 0){throw ('AI mode mutation was not caught: '+$modeCase.name)}
        }
        $aiAcceptance['modeWorkflowMutationPassed']=$true
    }
    foreach ($sourceFile in $workflowIdentity.sourceFiles) {
        $record=@($provenance.files | Where-Object path -eq $sourceFile.path)
        if ($record.Count -ne 1 -or $record[0].sha256 -ne $sourceFile.sha256) { throw ('AI workflow mutation is unrelated to final source: '+$sourceFile.path) }
    }
    $aiAcceptance['workflowTests']=$build.aiWorkflowTests
    $aiAcceptance['workflowMutationPassed']=$true
}
if([version]$releaseVersion -ge [version]'2.23.2.6'){
    $mathSourcePath='native/gridtimer_native/src/android_answer_render.rs'
    $mathSource=@($provenance.files | Where-Object path -eq $mathSourcePath)
    $mathVerifier=@($provenance.files | Where-Object path -eq 'tools/verify_android_math_mutation.ps1')
    $mathMutation=Get-Content -LiteralPath (Join-Path $evidence 'math_render_mutation/receipt.json') -Raw | ConvertFrom-Json
    if($mathSource.Count -ne 1 -or $mathVerifier.Count -ne 1 -or !$mathMutation.passed -or !$mathMutation.productionUnchanged -or $mathMutation.version -ne $releaseVersion -or $mathMutation.sourcePath -ne $mathSourcePath -or $mathMutation.sourceSha256Before -cne $mathSource[0].sha256 -or $mathMutation.sourceSha256After -cne $mathSource[0].sha256 -or $mathMutation.verifierSha256 -cne $mathVerifier[0].sha256 -or $mathMutation.tests -lt 5 -or $mathMutation.tests -ne $build.mathNativeTests -or $mathMutation.cases.Count -ne 4 -or @($mathMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0 -or !$_.allBusinessTestsObserved}).Count){throw 'Offline formula mutation acceptance does not bind the built renderer'}
    $mathExpectedKinds=@{baseline='baseline';cosmetic='cosmetic';missing_raw_html_guard='raw_html_guard_removed';restored='restored'}
    foreach($caseName in $mathExpectedKinds.Keys){
        $case=@($mathMutation.cases | Where-Object name -eq $caseName)
        if($case.Count -ne 1 -or $case[0].kind -ne $mathExpectedKinds[$caseName] -or $null -eq $case[0].testExit){throw ('Formula mutation case is missing: '+$caseName)}
        $testLogPath=Join-Path $evidence ('math_render_mutation/'+$caseName+'_tests.log')
        if((Get-FileHash -LiteralPath $testLogPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $case[0].testLogSha256){throw ('Formula mutation log changed: '+$caseName)}
        $testLog=Get-Content -LiteralPath $testLogPath -Raw
        if($caseName -eq 'missing_raw_html_guard'){
            $required='android_math_render_rejects_answer_html_and_external_resources'
            if($case[0].testExit -eq 0 -or !$case[0].requiredFailureObserved -or $case[0].requiredFailureTest -ne $required -or $testLog -notmatch ('(?m)^test android_answer_render::tests::'+$required+' \.\.\. FAILED\s*$')){throw 'Removing the formula answer HTML guard did not fail the required business test'}
        }else{
            if($case[0].testExit -ne 0){throw ('Formula baseline or cosmetic case failed: '+$caseName)}
            foreach($testName in $build.mathNativeTestNames){
                if($testLog -notmatch ('(?m)^test android_answer_render::tests::'+[regex]::Escape($testName)+' \.\.\. ok\s*$')){throw ('Formula business test is absent from mutation log: '+$testName)}
            }
        }
    }
    $mathUiExpected=[int]$build.aiWorkflowTests.AndroidMarkdownRenderBoundaryTest
    [xml]$mathUiTests=Get-Content -LiteralPath (Join-Path $evidence 'AndroidMarkdownRenderBoundaryTest.xml') -Raw
    if($mathUiExpected -lt 3 -or [int]$mathUiTests.testsuite.tests -ne $mathUiExpected -or [int]$mathUiTests.testsuite.failures -ne 0 -or [int]$mathUiTests.testsuite.errors -ne 0 -or [int]$mathUiTests.testsuite.skipped -ne 0){throw 'Formula reader boundary business tests are incomplete'}
    foreach($testName in @('networkAndActiveDocumentsCannotLoad','disposedOrSupersededPageCannotResizeCurrentAnswer','staleWidthAndInvalidHeightCannotHideAnswer')){
        if(@($mathUiTests.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1){throw ('Formula reader boundary test is missing: '+$testName)}
    }
    $mathAssets=[ordered]@{}
    foreach($asset in @('katex_v0.18.7.min.js','katex_v0.18.7_embedded.css','katex_manifest.json','katex_LICENSE.txt')){
        $relative='native/gridtimer_native/src/desktop/assets/'+$asset
        $record=@($provenance.files | Where-Object path -eq $relative)
        $path=Join-Path $releaseRoot $relative
        if($record.Count -ne 1 -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $record[0].sha256){throw ('Offline formula asset changed or was not frozen: '+$asset)}
        $mathAssets[$asset]=[ordered]@{sha256=$record[0].sha256;bytes=(Get-Item -LiteralPath $path).Length}
    }
    $mathAssetManifest=Get-Content -LiteralPath (Join-Path $releaseRoot 'native/gridtimer_native/src/desktop/assets/katex_manifest.json') -Raw | ConvertFrom-Json
    if($mathAssetManifest.version -ne '0.18.7'){throw 'Unexpected offline formula engine version'}
    foreach($asset in @('katex_v0.18.7.min.js','katex_v0.18.7_embedded.css')){
        $record=@($mathAssetManifest.files | Where-Object name -eq $asset)
        if($record.Count -ne 1 -or $record[0].sha256 -cne $mathAssets[$asset].sha256 -or $record[0].bytes -ne $mathAssets[$asset].bytes){throw ('Offline formula asset differs from its resource manifest: '+$asset)}
    }
    $mathAcceptance=[ordered]@{nativeBusinessTests=[int]$build.mathNativeTests;readerBoundaryTests=$mathUiExpected;mutationPassed=$true;rendererSourceSha256=$mathSource[0].sha256;offlineEngineVersion=$mathAssetManifest.version;offlineAssets=$mathAssets}
}
if([version]$releaseVersion -ge [version]'2.23.2.7'){
    if($build.mathNativeTests -lt 6 -or 'android_math_render_accepts_parenthesized_whitespace_without_treating_currency_as_math' -notin $build.mathNativeTestNames){throw 'Parenthesized formula and currency boundary acceptance is missing'}
    $loadingSourcePath='native/gridtimer_native/src/sourcegen/android_ai_answer_ui.rs'
    $loadingVerifierPath='tools/verify_android_markdown_loading_mutation.ps1'
    $loadingSource=@($provenance.files | Where-Object path -eq $loadingSourcePath)
    $loadingVerifier=@($provenance.files | Where-Object path -eq $loadingVerifierPath)
    $loadingMutation=Get-Content -LiteralPath (Join-Path $evidence 'markdown_loading_mutation/receipt.json') -Raw | ConvertFrom-Json
    if($loadingSource.Count -ne 1 -or $loadingVerifier.Count -ne 1 -or !$loadingMutation.passed -or !$loadingMutation.productionUnchanged -or $loadingMutation.version -ne $releaseVersion -or $loadingMutation.sourcePath -ne $loadingSourcePath -or $loadingMutation.sourceSha256Before -cne $loadingSource[0].sha256 -or $loadingMutation.sourceSha256After -cne $loadingSource[0].sha256 -or $loadingMutation.verifierPath -ne $loadingVerifierPath -or $loadingMutation.verifierSha256 -cne $loadingVerifier[0].sha256 -or $loadingMutation.tests -lt 7 -or $loadingMutation.tests -ne $mathUiExpected -or $loadingMutation.cases.Count -ne 6 -or @($loadingMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0 -or !$_.allBusinessTestsObserved}).Count){throw 'Loading mutation acceptance does not bind the final Android renderer'}
    $loadingActual=Get-Content -LiteralPath (Join-Path $releaseRoot $loadingSourcePath) -Raw
    foreach($literal in @(@{name='BOUNDARY_CONTENTS';hash=$loadingMutation.helperSha256},@{name='TEST_CONTENTS';hash=$loadingMutation.testsSha256},@{name='CONTENTS';hash=$loadingMutation.uiSha256})){
        $body=[regex]::Match($loadingActual, '(?s)pub const '+[regex]::Escape($literal.name)+':\s*&str\s*=\s*r(?<hashes>#+)"(?<body>.*?)"\k<hashes>;')
        if(!$body.Success){throw ('Tested Android loading literal is missing: '+$literal.name)}
        $bytes=[Text.Encoding]::UTF8.GetBytes($body.Groups['body'].Value.Replace("`r`n", "`n"))
        if([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant() -cne $literal.hash){throw ('Actual Android loading body changed after mutation verification: '+$literal.name)}
    }
    foreach($testName in $loadingMutation.testNames){
        if(@($mathUiTests.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1){throw ('Executed loading business test is missing from the formal suite: '+$testName)}
    }
    $loadingExpectedFailures=@{removed_document_identity='onlyExactCurrentMainDocumentCanLoadOffline';removed_main_frame_authorization='onlyExactCurrentMainDocumentCanLoadOffline';removed_formula_completion='incompleteFormulaOrForeignDocumentCannotBecomeReady'}
    foreach($caseName in @('baseline','cosmetic','removed_document_identity','removed_main_frame_authorization','removed_formula_completion','restored')){
        $case=@($loadingMutation.cases | Where-Object name -eq $caseName)
        if($case.Count -ne 1 -or $null -eq $case[0].testExit -or $case[0].testsSha256 -cne $loadingMutation.testsSha256){throw ('Loading mutation case is missing or tests changed: '+$caseName)}
        $compileLog=Join-Path $evidence ('markdown_loading_mutation/'+$caseName+'/compile.log')
        $testLog=Join-Path $evidence ('markdown_loading_mutation/'+$caseName+'/tests.log')
        if((Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant() -cne $case[0].compileLogSha256 -or (Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant() -cne $case[0].testLogSha256){throw ('Loading mutation evidence changed: '+$caseName)}
        $log=Get-Content -LiteralPath $testLog -Raw
        if($caseName -in @('baseline','cosmetic','restored')){
            if(!$case[0].expectedPass -or $case[0].testExit -ne 0 -or !$log.Contains("OK ($($loadingMutation.tests) tests)") -or $case[0].helperSha256 -cne $loadingMutation.helperSha256){throw ('Loading baseline, cosmetic or restored case failed: '+$caseName)}
            if(($caseName -eq 'cosmetic' -and $case[0].uiSha256 -ceq $loadingMutation.uiSha256) -or ($caseName -ne 'cosmetic' -and $case[0].uiSha256 -cne $loadingMutation.uiSha256)){throw 'Loading cosmetic UI variant or restored UI identity is invalid'}
        }else{
            $required=$loadingExpectedFailures[$caseName]
            if($case[0].expectedPass -or $case[0].testExit -eq 0 -or !$case[0].requiredFailureObserved -or $case[0].requiredFailureTest -ne $required -or !$log.Contains($required+'(com.ofairyo.gridtimer.ui.AndroidMarkdownRenderBoundaryTest)') -or !$log.Contains('java.lang.AssertionError') -or !$log.Contains("Tests run: $($loadingMutation.tests),") -or $case[0].helperSha256 -ceq $loadingMutation.helperSha256 -or $case[0].uiSha256 -cne $loadingMutation.uiSha256){throw ('Removing the loading guard did not fail its required business test: '+$caseName)}
        }
    }
    $mathAcceptance['loadingMutationPassed']=$true
    $mathAcceptance['currentDocumentBoundaryTests']=[int]$loadingMutation.tests
    $mathAcceptance['loadingSourceSha256']=$loadingSource[0].sha256
    $mathAcceptance['loadingVerifierSha256']=$loadingVerifier[0].sha256
}
if([version]$releaseVersion -ge [version]'2.23.2.8'){
    $legalSourcePath='native/gridtimer_native/src/sourcegen/legal_risk_ui_source.rs'
    $legalVerifierPath='tools/verify_legal_lifecycle_mutation.ps1'
    $legalSource=@($provenance.files | Where-Object path -eq $legalSourcePath)
    $legalVerifier=@($provenance.files | Where-Object path -eq $legalVerifierPath)
    $legalMutation=Get-Content -LiteralPath (Join-Path $evidence 'legal_lifecycle_mutation/receipt.json') -Raw | ConvertFrom-Json
    [xml]$legalTests=Get-Content -LiteralPath (Join-Path $evidence 'LegalSendReadyTest.xml') -Raw
    if($build.aiWorkflowTests.LegalSendReadyTest -ne 13 -or [int]$legalTests.testsuite.tests -ne 13 -or [int]$legalTests.testsuite.failures -ne 0 -or [int]$legalTests.testsuite.errors -ne 0 -or [int]$legalTests.testsuite.skipped -ne 0){throw 'Legal lifecycle formal business tests are incomplete or failed'}
    if($legalSource.Count -ne 1 -or $legalVerifier.Count -ne 1 -or !$legalMutation.passed -or !$legalMutation.productionUnchanged -or $legalMutation.version -ne $releaseVersion -or $legalMutation.sourcePath -ne $legalSourcePath -or $legalMutation.sourceSha256Before -cne $legalSource[0].sha256 -or $legalMutation.sourceSha256After -cne $legalSource[0].sha256 -or $legalMutation.verifierPath -ne $legalVerifierPath -or $legalMutation.verifierSha256 -cne $legalVerifier[0].sha256 -or $legalMutation.tests -ne 13 -or $legalMutation.cases.Count -ne 4 -or @($legalMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0 -or !$_.allBusinessTestsObserved}).Count){throw 'Legal lifecycle mutation acceptance does not bind the final Android source'}
    $legalActual=Get-Content -LiteralPath (Join-Path $releaseRoot $legalSourcePath) -Raw
    $legalUi=[regex]::Match($legalActual, '(?s)pub const CONTENTS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    $legalTestBody=[regex]::Match($legalActual, '(?s)pub const TEST_CONTENTS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if(!$legalUi.Success -or !$legalTestBody.Success){throw 'Legal Rust-owned UI or test literal is missing'}
    $legalUiText=$legalUi.Groups['body'].Value.Replace("`r`n","`n")
    $legalStart=$legalUiText.IndexOf('internal fun legalSendReady(')
    $legalEnd=$legalUiText.IndexOf('/** Separate from the finance overview pager.')
    if($legalStart -lt 0 -or $legalEnd -le $legalStart){throw 'Legal request helper boundaries are missing'}
    $legalHelperText="package com.ofairyo.gridtimer.ui`n`n"+$legalUiText.Substring($legalStart,$legalEnd-$legalStart)
    foreach($literal in @(@{text=$legalUiText;hash=$legalMutation.uiSha256},@{text=$legalTestBody.Groups['body'].Value.Replace("`r`n","`n");hash=$legalMutation.testsSha256},@{text=$legalHelperText;hash=$legalMutation.helperSha256})){
        $actual=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($literal.text))).ToLowerInvariant()
        if($actual -cne $literal.hash){throw 'Legal lifecycle mutation compiled a different helper, UI or test body'}
    }
    foreach($testName in $legalMutation.testNames){
        if(@($legalTests.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1){throw ('Legal lifecycle test absent from formal build: '+$testName)}
    }
    foreach($caseName in @('baseline','cosmetic','finish_generation_guard','restored')){
        $case=@($legalMutation.cases | Where-Object name -eq $caseName)
        if($case.Count -ne 1 -or $null -eq $case[0].testExit -or $case[0].testsSha256 -cne $legalMutation.testsSha256){throw ('Legal mutation case is missing or tests changed: '+$caseName)}
        $compileLog=Join-Path $evidence ('legal_lifecycle_mutation/'+$caseName+'/compile.log')
        $testLog=Join-Path $evidence ('legal_lifecycle_mutation/'+$caseName+'/tests.log')
        if((Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant() -cne $case[0].compileLogSha256 -or (Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant() -cne $case[0].testLogSha256){throw ('Legal mutation evidence changed: '+$caseName)}
        $log=Get-Content -LiteralPath $testLog -Raw
        if($caseName -eq 'finish_generation_guard'){
            $required='invalidationDuringSuspendedIoStillClearsBusyInFinally'
            if($case[0].expectedPass -or $case[0].testExit -eq 0 -or !$case[0].requiredFailureObserved -or $case[0].requiredFailureTest -ne $required -or !$log.Contains($required+'(com.ofairyo.gridtimer.ui.LegalSendReadyTest)') -or !$log.Contains('java.lang.AssertionError') -or !$log.Contains('Tests run: 13,') -or $case[0].helperSha256 -ceq $legalMutation.helperSha256 -or $case[0].uiSha256 -cne $legalMutation.uiSha256){throw 'The invalidated request settlement mutation did not fail its required business test'}
        }else{
            if(!$case[0].expectedPass -or $case[0].testExit -ne 0 -or !$log.Contains('OK (13 tests)') -or $case[0].helperSha256 -cne $legalMutation.helperSha256){throw ('Legal baseline, cosmetic or restored case failed: '+$caseName)}
            if(($caseName -eq 'cosmetic' -and $case[0].uiSha256 -ceq $legalMutation.uiSha256) -or ($caseName -ne 'cosmetic' -and $case[0].uiSha256 -cne $legalMutation.uiSha256)){throw 'Legal cosmetic variant or restored UI identity is invalid'}
        }
    }
    $legalAcceptance=[ordered]@{businessTests=13;mutationPassed=$true;sourceSha256=$legalSource[0].sha256;verifierSha256=$legalVerifier[0].sha256;hostOnly=$true;deviceVerified=$false}
}
if([version]$releaseVersion -ge [version]'2.23.2.9'){
    $workflowSourcePath='native/gridtimer_native/src/sourcegen/android_legal_workflow.rs'
    $workflowVerifierPath='tools/verify_legal_workflow_mutation.ps1'
    $workflowBaseSourcePath='native/gridtimer_native/src/sourcegen/legal_risk_ui_source.rs'
    $workflowSource=@($provenance.files | Where-Object path -eq $workflowSourcePath)
    $workflowVerifier=@($provenance.files | Where-Object path -eq $workflowVerifierPath)
    $workflowBaseSource=@($provenance.files | Where-Object path -eq $workflowBaseSourcePath)
    $workflowMutation=Get-Content -LiteralPath (Join-Path $evidence 'legal_workflow_mutation/receipt.json') -Raw | ConvertFrom-Json
    $workflowActual=Get-Content -LiteralPath (Join-Path $releaseRoot $workflowSourcePath) -Raw
    $workflowHelperLiteral=[regex]::Match($workflowActual,'(?s)pub const HELPERS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    $workflowTestLiteral=[regex]::Match($workflowActual,'(?s)pub const TEST_CONTENTS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if(!$workflowHelperLiteral.Success -or !$workflowTestLiteral.Success){throw 'Rust-owned legal workflow helper or tests are missing'}
    $workflowHelperText=$workflowHelperLiteral.Groups['body'].Value.Replace("`r`n","`n")
    $workflowTestText=$workflowTestLiteral.Groups['body'].Value.Replace("`r`n","`n")
    $workflowTestNames=@([regex]::Matches($workflowTestText,'@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object {$_.Groups[1].Value})
    [xml]$workflowTests=Get-Content -LiteralPath (Join-Path $evidence 'LegalAnalysisWorkflowTest.xml') -Raw
    if($workflowTestNames.Count -lt 1 -or $build.aiWorkflowTests.LegalAnalysisWorkflowTest -ne $workflowTestNames.Count -or [int]$workflowTests.testsuite.tests -ne $workflowTestNames.Count -or [int]$workflowTests.testsuite.failures -ne 0 -or [int]$workflowTests.testsuite.errors -ne 0 -or [int]$workflowTests.testsuite.skipped -ne 0){throw 'Legal workflow formal business tests are incomplete or failed'}
    if($workflowSource.Count -ne 1 -or $workflowVerifier.Count -ne 1 -or $workflowBaseSource.Count -ne 1 -or !$workflowMutation.passed -or !$workflowMutation.productionUnchanged -or $workflowMutation.version -ne $releaseVersion -or $workflowMutation.sourcePath -ne $workflowSourcePath -or $workflowMutation.sourceSha256Before -cne $workflowSource[0].sha256 -or $workflowMutation.sourceSha256After -cne $workflowSource[0].sha256 -or $workflowMutation.verifierPath -ne $workflowVerifierPath -or $workflowMutation.verifierSha256 -cne $workflowVerifier[0].sha256 -or $workflowMutation.baseSourcePath -ne $workflowBaseSourcePath -or $workflowMutation.baseSourceSha256Before -cne $workflowBaseSource[0].sha256 -or $workflowMutation.baseSourceSha256After -cne $workflowBaseSource[0].sha256 -or $workflowMutation.tests -ne $workflowTestNames.Count -or $workflowMutation.cases.Count -ne 6 -or @($workflowMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0 -or !$_.allBusinessTestsObserved}).Count){throw 'Legal workflow mutation is unrelated to the frozen source or business tests'}
    $workflowBaseHelperText=$legalUiText.Substring($legalStart,$legalEnd-$legalStart)
    $workflowCompiledHelperText=$legalHelperText+$workflowHelperText
    $workflowCompiledTestText="package com.ofairyo.gridtimer.ui`nimport org.junit.Test`nimport org.junit.Assert.assertTrue`nimport org.junit.Assert.assertFalse`n"+$workflowTestText
    foreach($literal in @(@{text=$workflowHelperText;hash=$workflowMutation.workflowHelperSha256},@{text=$workflowTestText;hash=$workflowMutation.testsSha256},@{text=$workflowBaseHelperText;hash=$workflowMutation.baseHelperSha256},@{text=$workflowCompiledHelperText;hash=$workflowMutation.helperSha256},@{text=$workflowCompiledTestText;hash=$workflowMutation.compiledTestSha256})){
        $actual=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($literal.text))).ToLowerInvariant()
        if($actual -cne $literal.hash){throw 'Legal workflow mutation compiled a different Rust-owned helper or test body'}
    }
    if($workflowMutation.testNames.Count -ne $workflowTestNames.Count){throw 'Legal workflow mutation test inventory differs from the actual Rust-owned suite'}
    foreach($testName in $workflowTestNames){
        if(@($workflowTests.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1 -or $testName -notin $workflowMutation.testNames){throw ('Legal workflow business test is missing: '+$testName)}
    }
    foreach($caseName in @('baseline','cosmetic','removedSendAuthorization','removedConfigurationBinding','removedReadinessGuard','restored')){
        $case=@($workflowMutation.cases | Where-Object name -eq $caseName)
        if($case.Count -ne 1 -or $null -eq $case[0].testExit -or $case[0].testsSha256 -cne $workflowMutation.testsSha256 -or $case[0].compiledTestSha256 -cne $workflowMutation.compiledTestSha256){throw ('Legal workflow mutation case or original test body is missing: '+$caseName)}
        $compileLog=Join-Path $evidence ('legal_workflow_mutation/'+$caseName+'/compile.log')
        $testLog=Join-Path $evidence ('legal_workflow_mutation/'+$caseName+'/tests.log')
        if((Get-FileHash -LiteralPath $compileLog).Hash.ToLowerInvariant() -cne $case[0].compileLogSha256 -or (Get-FileHash -LiteralPath $testLog).Hash.ToLowerInvariant() -cne $case[0].testLogSha256){throw ('Legal workflow mutation evidence changed: '+$caseName)}
        $log=Get-Content -LiteralPath $testLog -Raw
        if($caseName -in @('baseline','cosmetic','restored')){
            if(!$case[0].expectedPass -or $case[0].testExit -ne 0 -or !$log.Contains("OK ($($workflowTestNames.Count) tests)")){throw ('Legal workflow baseline, cosmetic or restored case failed: '+$caseName)}
            if(($caseName -ne 'cosmetic' -and $case[0].helperSha256 -cne $workflowMutation.helperSha256) -or ($caseName -eq 'cosmetic' -and $case[0].helperSha256 -ceq $workflowMutation.helperSha256)){throw 'Legal workflow cosmetic or restored helper identity is invalid'}
        }else{
            $required=$case[0].requiredFailureTest
            if($case[0].expectedPass -or $case[0].testExit -eq 0 -or !$case[0].requiredFailureObserved -or $required -notin $workflowTestNames -or !$log.Contains($required+'(com.ofairyo.gridtimer.ui.LegalAnalysisWorkflowTest)') -or !$log.Contains('java.lang.AssertionError') -or !$log.Contains("Tests run: $($workflowTestNames.Count),") -or $case[0].helperSha256 -ceq $workflowMutation.helperSha256){throw ('Removing a legal send authorization guard did not fail a business test: '+$caseName)}
            $requiredTest=@{removedSendAuthorization='requestOwnershipRequiresFreshBoundSingleUseAuthorization';removedConfigurationBinding='changedScanEndpointModelOrKeyCannotReuseConsent';removedReadinessGuard='busyOrStalePreviewCannotBeConfirmed'}[$caseName]
            if($required -ne $requiredTest){throw ('Legal authorization mutation did not exercise its required production boundary: '+$caseName)}
        }
    }
    $workflowIntegration=Get-Content -LiteralPath (Join-Path $evidence 'legal_workflow_generated_acceptance.json') -Raw | ConvertFrom-Json
    if(!$workflowIntegration.passed -or !$workflowIntegration.finalTransformConnected -or !$workflowIntegration.isolatedEntryPresent -or !$workflowIntegration.primaryActionPresent -or !$workflowIntegration.explicitConfirmationPresent -or !$workflowIntegration.sendBoundaryPresent -or $workflowIntegration.modelRequestSites -ne 1 -or $workflowIntegration.businessTests -ne $workflowTestNames.Count -or !$build.legalWorkflowIntegration.passed){throw 'Generated legal actions are not connected to the tested request boundary'}
    $workflowGeneratedPaths=@('app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/GridTimerScreen.kt','app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/LegalRiskScreen.kt')
    if($workflowIntegration.generatedFiles.Count -ne $workflowGeneratedPaths.Count){throw 'Generated legal workflow file inventory is incomplete'}
    foreach($generatedPath in $workflowGeneratedPaths){
        $generatedFile=@($workflowIntegration.generatedFiles | Where-Object path -eq $generatedPath)
        $builtGeneratedFile=@($build.legalWorkflowIntegration.generatedFiles | Where-Object path -eq $generatedPath)
        if($generatedFile.Count -ne 1 -or $builtGeneratedFile.Count -ne 1 -or $generatedFile[0].sha256 -cne $builtGeneratedFile[0].sha256 -or (Get-FileHash -LiteralPath (Join-Path $releaseRoot $generatedPath)).Hash.ToLowerInvariant() -cne $generatedFile[0].sha256){throw 'Generated legal workflow source changed after the formal build'}
    }
    $legalWorkflowAcceptance=[ordered]@{businessTests=$workflowTestNames.Count;mutationPassed=$true;sourceSha256=$workflowSource[0].sha256;verifierSha256=$workflowVerifier[0].sha256;baseSourceSha256=$workflowBaseSource[0].sha256;explicitConfirmationVerified=$true;generatedSourceConnected=$true;hostOnly=$true;deviceVerified=$false}
}
if([version]$releaseVersion -ge [version]'2.23.2.10'){
    if(!$build.documentIntegration.passed -or !$build.documentIntegration.policyConnectedToRealEditor -or !$build.documentIntegration.workspaceAndNoteFocusLifetime -or !$build.documentIntegration.compactHeaderPresent -or $build.aiWorkflowTests.DocumentCaretPolicyTest -ne 15){throw 'Generated document integration or caret domain acceptance missing'}
    foreach($entry in $build.documentIntegration.generatedFiles){if((Get-FileHash -LiteralPath (Join-Path $releaseRoot $entry.path)).Hash.ToLowerInvariant() -cne $entry.sha256){throw 'Generated document integration changed after build'}}
    foreach($definition in @(
        @{folder='document_caret_mutation';source='native/gridtimer_native/src/sourcegen/android_document_caret.rs';verifier='tools/verify_document_caret_mutation.ps1';tests=15;cases=5;critical=@{removed_layout_identity='equalLengthReplacementDoesNotUseAnOldLayout';removed_visible_block_guard='visibleBlockNeverJumpsToItsTop'}},
        @{folder='knowledge_header_mutation';source='native/gridtimer_native/src/sourcegen/android_knowledge_header.rs';verifier='tools/verify_knowledge_header_mutation.ps1';tests=2;cases=4;critical=@{missing_header_usage_guard='refuses_missing_duplicate_or_unreconciled_template_anchors'}}
    )){
        $receiptPath=Join-Path $evidence ($definition.folder+'/receipt.json')
        $record=Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
        $sourceEntry=@($provenance.files | Where-Object path -eq $definition.source)
        $verifierEntry=@($provenance.files | Where-Object path -eq $definition.verifier)
        if(!$record.passed -or !$record.productionUnchanged -or $record.version -ne $releaseVersion -or $sourceEntry.Count -ne 1 -or $verifierEntry.Count -ne 1 -or $record.sourcePath -ne $definition.source -or $record.sourceSha256Before -cne $sourceEntry[0].sha256 -or $record.sourceSha256After -cne $sourceEntry[0].sha256 -or $record.verifierPath -ne $definition.verifier -or $record.verifierSha256 -cne $verifierEntry[0].sha256 -or $record.tests -ne $definition.tests -or $record.cases.Count -ne $definition.cases -or @($record.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0 -or !$_.allBusinessTestsObserved}).Count){throw ('Document mutation acceptance does not bind the built source: '+$definition.folder)}
        foreach($case in $record.cases){
            if($null -eq $case.testExit){throw 'Document mutation has no actual test result'}
            if($definition.critical.ContainsKey($case.name)){
                if($case.testExit -eq 0 -or !$case.requiredFailureObserved -or !$case.requiredFailureTest.EndsWith($definition.critical[$case.name])){throw 'Document mutation failed to reject the required state guard removal'}
            } elseif($case.name -notin @('baseline','cosmetic','restored') -or $case.testExit -ne 0){throw 'Document cosmetic or baseline business acceptance failed'}
        }
        if($definition.folder -eq 'knowledge_header_mutation'){
            foreach($case in $record.cases){
                foreach($log in @(@{suffix='_compile.jsonl';hash=$case.compileJsonSha256},@{suffix='_compile.log';hash=$case.compileLogSha256},@{suffix='_tests.log';hash=$case.testLogSha256})){
                    if((Get-FileHash -LiteralPath (Join-Path $evidence ($definition.folder+'/'+$case.name+$log.suffix))).Hash.ToLowerInvariant() -cne $log.hash){throw 'Header mutation evidence changed'}
                }
            }
        }
        if($definition.folder -eq 'document_caret_mutation'){
            foreach($case in $record.cases){
                foreach($log in @(@{name='compile';hash=$case.compileLogSha256},@{name='tests';hash=$case.testLogSha256})){
                    if((Get-FileHash -LiteralPath (Join-Path $evidence ($definition.folder+'/'+$case.name+'/'+$log.name+'.log'))).Hash.ToLowerInvariant() -cne $log.hash){throw 'Caret mutation evidence changed'}
                }
            }
        }
    }
}
if([version]$releaseVersion -ge [version]'2.23.2.11'){
    $financeSourcePath='native/gridtimer_native/src/sourcegen/android_finance_workspace.rs'
    $financeVerifierPath='tools/verify_finance_workspace_mutation.ps1'
    $financeSource=@($provenance.files | Where-Object path -eq $financeSourcePath)
    $financeVerifier=@($provenance.files | Where-Object path -eq $financeVerifierPath)
    $financeMutation=Get-Content -LiteralPath (Join-Path $evidence 'finance_workspace_mutation/receipt.json') -Raw | ConvertFrom-Json
    $financeActual=Get-Content -LiteralPath (Join-Path $releaseRoot $financeSourcePath) -Raw
    $financeBodies=@{}
    foreach($name in @('POLICY','TEST_CONTENTS','WORKSPACE','COMPONENTS')){
        $literal=[regex]::Match($financeActual,'(?s)(?:pub\s+)?const\s+'+$name+':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
        if(!$literal.Success){throw ('Rust-owned finance literal is missing: '+$name)}
        $financeBodies[$name]=$literal.Groups['body'].Value.Replace("`r`n","`n")
    }
    $financeTestNames=@([regex]::Matches($financeBodies.TEST_CONTENTS,'@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object {$_.Groups[1].Value})
    $financeRequiredTests=@('newMonthClosesOldDetailAndReturnsToOverview','newWorkspaceDoesNotInheritFinancialDetail','switchingTaskClearsThePreviousDialog','reviewCountUsesAllFourExplicitFlags','alertPreviewIsBoundedWithoutChangingTheTotal')
    [xml]$financeTests=Get-Content -LiteralPath (Join-Path $evidence 'FinanceWorkspacePolicyTest.xml') -Raw
    if($financeTestNames.Count -ne 5 -or @($financeTestNames | Sort-Object -Unique).Count -ne 5 -or @($financeRequiredTests | Where-Object {$_ -notin $financeTestNames}).Count -or $build.financeWorkspaceTests -ne 5 -or [int]$financeTests.testsuite.tests -ne 5 -or [int]$financeTests.testsuite.failures -ne 0 -or [int]$financeTests.testsuite.errors -ne 0 -or [int]$financeTests.testsuite.skipped -ne 0){throw 'Finance navigation formal state tests are incomplete or failed'}
    if($financeSource.Count -ne 1 -or $financeVerifier.Count -ne 1 -or !$financeMutation.passed -or !$financeMutation.productionUnchanged -or $financeMutation.version -ne $releaseVersion -or $financeMutation.sourcePath -ne $financeSourcePath -or $financeMutation.sourceSha256Before -cne $financeSource[0].sha256 -or $financeMutation.sourceSha256After -cne $financeSource[0].sha256 -or $financeMutation.verifierPath -ne $financeVerifierPath -or $financeMutation.verifierSha256 -cne $financeVerifier[0].sha256 -or $financeMutation.tests -ne 5 -or $financeMutation.cases.Count -ne 6 -or @($financeMutation.cases | Where-Object {!$_.passed -or $_.compileExit -ne 0 -or !$_.allBusinessTestsObserved}).Count){throw 'Finance mutation is unrelated to the frozen source or state tests'}
    foreach($literal in @(
        @{text=$financeBodies.POLICY;hash=$financeMutation.policySha256},
        @{text=$financeBodies.TEST_CONTENTS;hash=$financeMutation.testsSha256},
        @{text=$financeBodies.WORKSPACE;hash=$financeMutation.workspaceSha256},
        @{text=$financeBodies.COMPONENTS;hash=$financeMutation.componentsSha256},
        @{text=("package com.ofairyo.gridtimer.ui`n"+$financeBodies.POLICY);hash=$financeMutation.helperSha256}
    )){
        $actual=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($literal.text))).ToLowerInvariant()
        if($actual -cne $literal.hash){throw 'Finance mutation compiled a different Rust-owned policy or test body'}
    }
    if($financeMutation.testNames.Count -ne 5){throw 'Finance mutation test inventory is incomplete'}
    foreach($testName in $financeRequiredTests){if(@($financeTests.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1 -or $testName -notin $financeMutation.testNames){throw ('Finance state test is absent from formal build or mutation: '+$testName)}}
    $financeCritical=@{removed_workspace_guard='newWorkspaceDoesNotInheritFinancialDetail';removed_month_guard='newMonthClosesOldDetailAndReturnsToOverview';removed_preview_limit='alertPreviewIsBoundedWithoutChangingTheTotal'}
    foreach($caseName in @('baseline','cosmetic','removed_workspace_guard','removed_month_guard','removed_preview_limit','restored')){
        $case=@($financeMutation.cases | Where-Object name -eq $caseName)
        if($case.Count -ne 1 -or $null -eq $case[0].testExit -or $case[0].testsSha256 -cne $financeMutation.testsSha256 -or $case[0].workspaceSha256 -cne $financeMutation.workspaceSha256){throw ('Finance mutation case or original test body is missing: '+$caseName)}
        $compileLog=Join-Path $evidence ('finance_workspace_mutation/'+$caseName+'/compile.log')
        $testLog=Join-Path $evidence ('finance_workspace_mutation/'+$caseName+'/tests.log')
        if((Get-FileHash -LiteralPath $compileLog).Hash.ToLowerInvariant() -cne $case[0].compileLogSha256 -or (Get-FileHash -LiteralPath $testLog).Hash.ToLowerInvariant() -cne $case[0].testLogSha256){throw ('Finance mutation evidence changed: '+$caseName)}
        $log=Get-Content -LiteralPath $testLog -Raw
        if($financeCritical.ContainsKey($caseName)){
            $required=$financeCritical[$caseName]
            if($case[0].expectedPass -or $case[0].testExit -eq 0 -or !$case[0].requiredFailureObserved -or $case[0].requiredFailureTest -ne $required -or !$log.Contains($required+'(com.ofairyo.gridtimer.ui.FinanceWorkspacePolicyTest)') -or !$log.Contains('java.lang.AssertionError') -or !$log.Contains('Tests run: 5,') -or $case[0].helperSha256 -ceq $financeMutation.helperSha256 -or $case[0].componentsSha256 -cne $financeMutation.componentsSha256){throw ('Removing a finance state guard did not fail its required test: '+$caseName)}
        }else{
            if(!$case[0].expectedPass -or $case[0].testExit -ne 0 -or !$log.Contains('OK (5 tests)')){throw ('Finance baseline, cosmetic or restored state acceptance failed: '+$caseName)}
            if($caseName -eq 'cosmetic'){
                if($case[0].helperSha256 -ceq $financeMutation.helperSha256 -or $case[0].componentsSha256 -ceq $financeMutation.componentsSha256){throw 'Finance cosmetic variant did not change both label and style'}
            }elseif($case[0].helperSha256 -cne $financeMutation.helperSha256 -or $case[0].componentsSha256 -cne $financeMutation.componentsSha256){throw 'Finance baseline or restored policy identity is invalid'}
        }
    }
    $financeIntegration=Get-Content -LiteralPath (Join-Path $evidence 'finance_workspace_generated_acceptance.json') -Raw | ConvertFrom-Json
    if(!$financeIntegration.passed -or !$financeIntegration.finalTransformConnected -or !$financeIntegration.policyConnectedToRealPanel -or !$financeIntegration.workspaceAndMonthLifetime -or !$financeIntegration.allDetailSectionsPresent -or !$financeIntegration.legalWorkflowEntryPreserved -or $financeIntegration.businessTests -ne 5 -or !$build.financeWorkspaceIntegration.passed){throw 'Generated finance actions are not connected to the tested navigation policy'}
    $financeGeneratedPaths=@('app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/FinanceRiskV2Panel.kt','app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/GridTimerScreen.kt')
    if($financeIntegration.generatedFiles.Count -ne 2){throw 'Generated finance workspace inventory is incomplete'}
    foreach($generatedPath in $financeGeneratedPaths){
        $generatedFile=@($financeIntegration.generatedFiles | Where-Object path -eq $generatedPath)
        $builtGeneratedFile=@($build.financeWorkspaceIntegration.generatedFiles | Where-Object path -eq $generatedPath)
        if($generatedFile.Count -ne 1 -or $builtGeneratedFile.Count -ne 1 -or $generatedFile[0].sha256 -cne $builtGeneratedFile[0].sha256 -or (Get-FileHash -LiteralPath (Join-Path $releaseRoot $generatedPath)).Hash.ToLowerInvariant() -cne $generatedFile[0].sha256){throw 'Generated finance workspace changed after the formal build'}
    }
    $financeAcceptance=[ordered]@{businessTests=5;mutationPassed=$true;sourceSha256=$financeSource[0].sha256;verifierSha256=$financeVerifier[0].sha256;generatedSourceConnected=$true;legalWorkflowEntryPreserved=$true;hostOnly=$true;deviceVerified=$false}
}
$hash=(Get-FileHash -LiteralPath $apk).Hash.ToLowerInvariant()
if($hash -ne $build.apkSha256){throw 'APK changed after verification'}
$publishedApk=@($manifest.files | Where-Object role -eq 'apk')
$alreadyPublished=$Mode -eq 'verify' -and $manifest.androidRelease.version -eq $releaseVersion -and $manifest.androidRelease.sha256 -eq $hash -and $publishedApk.Count -eq 1 -and $publishedApk[0].file_name -eq $name -and $publishedApk[0].sha256 -eq $hash
if($alreadyPublished -and $manifest.androidRelease.sourceSnapshotSha256 -ne $build.sourceSnapshotSha256){throw 'Published source snapshot mismatch'}
if(!$alreadyPublished){
    foreach($file in $provenance.files){
        if((Get-FileHash -LiteralPath (Join-Path $releaseRoot $file.path)).Hash.ToLowerInvariant() -ne $file.sha256){throw "Source changed: $($file.path)"}
    }
}
function Assert-ReleasePath([string]$Path){
    $resolved=[IO.Path]::GetFullPath($Path)
    if(!$resolved.StartsWith($releaseRoot+'\',[StringComparison]::OrdinalIgnoreCase)){throw "Path outside project: $resolved"}
    return $resolved
}
$windows=@($manifest.files | Where-Object role -ne 'apk' | ForEach-Object {
    $relative=if($_.role -eq 'cloudflared'){'release_artifacts/current/tools/'+$_.file_name}else{'release_artifacts/current/'+$_.file_name}
    $path=Join-Path $releaseRoot $relative
    if((Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ne $_.sha256){throw "Retained Windows artifact mismatch: $relative"}
    [ordered]@{path=$path;sha256=$_.sha256;modified=(Get-Item -LiteralPath $path).LastWriteTimeUtc.Ticks}
})
$desktopEntry=Join-Path $releaseRoot 'release_artifacts/desktop_entry'
if(!(Test-Path -LiteralPath (Join-Path $desktopEntry 'TenRate_Desktop_Launcher.exe') -PathType Leaf)){throw 'Retained stable Windows desktop launcher is missing'}
$windows+=@(Get-ChildItem -LiteralPath $desktopEntry -File | ForEach-Object {
    [ordered]@{path=$_.FullName;sha256=(Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant();modified=$_.LastWriteTimeUtc.Ticks}
})
$windowsVersion=$manifest.version
$oldJava=$env:JAVA_HOME
$oldPath=$env:PATH
try {
    $env:JAVA_HOME='C:\tools\java\jdk-17.0.18+8'
    $env:PATH="$env:JAVA_HOME\bin;$env:PATH"
    $sdk='C:\tools\android-sdk\build-tools\35.0.0'
    & (Join-Path $sdk 'apksigner.bat') verify --verbose --print-certs $apk *> (Join-Path $evidence 'apk_signature.txt')
    if($LASTEXITCODE -ne 0){throw 'Release signature verification failed'}
    $signature=Get-Content (Join-Path $evidence 'apk_signature.txt') -Raw
    $certificate=[regex]::Match($signature,'Signer #1 certificate SHA-256 digest: ([a-f0-9]+)').Groups[1].Value
    $previousManifest=$manifest
    $previousEntry=@($previousManifest.files | Where-Object role -eq 'apk')[0]
    $previousName=$previousEntry.file_name
    $previousCandidates=@((Join-Path $releaseRoot ('release_artifacts/current/'+$previousName)),(Join-Path $releaseRoot ('old_apks/'+[IO.Path]::GetFileNameWithoutExtension($previousName)+"_current_before_v$releaseVersion.apk")),(Join-Path $releaseRoot ('release_artifacts/verification/v'+$previousManifest.androidRelease.version+'/'+$previousName)))
    $previousApk=$previousCandidates | Where-Object {Test-Path -LiteralPath $_ -PathType Leaf} | Select-Object -First 1
    if(!$previousApk -or (Get-FileHash -LiteralPath $previousApk).Hash.ToLowerInvariant() -ne $previousEntry.sha256){throw 'Previous published APK is unavailable or changed'}
    & (Join-Path $sdk 'apksigner.bat') verify --verbose --print-certs $previousApk *> (Join-Path $evidence 'previous_apk_signature.txt')
    if($LASTEXITCODE -ne 0){throw 'Previous release signature verification failed'}
    $oldSignature=Get-Content (Join-Path $evidence 'previous_apk_signature.txt') -Raw
    if(!$certificate -or $certificate -ne [regex]::Match($oldSignature,'Signer #1 certificate SHA-256 digest: ([a-f0-9]+)').Groups[1].Value){throw 'Upgrade certificate changed'}
    Push-Location (Split-Path $apk -Parent)
    try {
        & (Join-Path $sdk 'aapt.exe') dump badging $name *> (Join-Path $evidence 'apk_badging.txt')
        if($LASTEXITCODE -ne 0){throw 'APK metadata read failed'}
        & (Join-Path $sdk 'zipalign.exe') -c -P 16 -v 4 $name *> (Join-Path $evidence 'apk_alignment.txt')
        if($LASTEXITCODE -ne 0){throw 'APK alignment failed'}
    } finally { Pop-Location }
    $badging=Get-Content (Join-Path $evidence 'apk_badging.txt') -Raw
    if(!$badging.Contains("package: name='com.ofairyo.gridtimer' versionCode='$releaseCode' versionName='$releaseVersion'") -or $badging.Contains('application-debuggable')){throw 'Wrong release package/version or debug flag'}
    $readelf='C:\tools\android-sdk\ndk\27.1.12297006\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-readelf.exe'
    $zip=[IO.Compression.ZipFile]::OpenRead($apk)
    $abis=@('arm64-v8a','armeabi-v7a','x86_64')
    $dexNames=@{'KnowledgeCanvasNative'=$false;'KnowledgeCanvasModel'=$false;'KnowledgeCanvasScreenKt'=$false;'SmartisanNoteUiKt'=$false;'GridTimerScreenKt'=$false;'MyAccountScreenKt'=$false;'AndroidUpdateHistoryDataKt'=$false;'KnowledgeFilterState'=$false;'KnowledgeFilterStateKt'=$false;'NoteCollectionPartitionKt'=$false}
    if($aiAcceptance){
        $dexNames['AiConnectionController']=$false
        if([version]$releaseVersion -ge [version]'2.23.2.3'){$dexNames['KnowledgeAiRequestBoundary']=$false}
        if([version]$releaseVersion -ge [version]'2.23.2.4'){$dexNames['KnowledgeKeyboardRequest']=$false}
        if([version]$releaseVersion -ge [version]'2.23.2.5'){$dexNames['KnowledgeAiMode']=$false}
    }
    if($mathAcceptance){
        $dexNames['AndroidRenderedMarkdownKt']=$false
        $dexNames['AndroidMarkdownRenderBoundary']=$false
        $mathEmbeddedAssets=@{}
        foreach($asset in @('katex_v0.18.7.min.js','katex_v0.18.7_embedded.css')){
            $mathEmbeddedAssets[$asset]=[Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes((Join-Path $releaseRoot ('native/gridtimer_native/src/desktop/assets/'+$asset))))
        }
    }
    if($legalAcceptance){$dexNames['LegalScanRequestBoundary']=$false;$dexNames['LegalRiskScreenKt']=$false}
    if($legalWorkflowAcceptance){$dexNames['LegalSendConsent']=$false;$dexNames['LegalPrimaryAction']=$false;$dexNames['LegalActionState']=$false}
    if([version]$releaseVersion -ge [version]'2.23.2.10'){$dexNames['DocumentCaretPolicyKt']=$false;$dexNames['DocumentCaretVisibilityKt']=$false;$dexNames['DocumentCaretVisibility']=$false;$dexNames['NoteDocumentEditorKt']=$false}
    if($financeAcceptance){$dexNames['FinanceRiskV2PanelKt']=$false;$dexNames['FinanceWorkspaceRoute']=$false;$dexNames['FinanceWorkspacePage']=$false;$dexNames['FinanceWorkspaceDetail']=$false}
    try {
        foreach($abi in $abis){
            $entry=$zip.GetEntry("lib/$abi/libgridtimer_native.so")
            if(!$entry){throw "Missing ABI $abi"}
            $directory=Join-Path $evidence ('native_symbols/'+$abi)
            New-Item -ItemType Directory -Force -Path $directory | Out-Null
            $library=Join-Path $directory 'libgridtimer_native.so'
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$library,$true)
            & $readelf --dyn-syms --wide $library *> (Join-Path $directory 'symbols.txt')
            if($LASTEXITCODE -ne 0){throw 'Native symbol read failed'}
            $symbols=Get-Content (Join-Path $directory 'symbols.txt') -Raw
            foreach($method in @('open','command','close')){if(!$symbols.Contains('Java_com_ofairyo_gridtimer_ui_KnowledgeCanvasNative_'+$method)){throw "Canvas JNI method missing: $abi $method"}}
            if($aiAcceptance -and !$symbols.Contains('Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeTestAiConnection')){throw "AI connection JNI method missing: $abi"}
            if([version]$releaseVersion -ge [version]'2.23.2.5' -and !$symbols.Contains('Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeCompleteAndroidAiQuery')){throw "Android AI question JNI method missing: $abi"}
            if($mathAcceptance){
                if(!$symbols.Contains('Java_com_ofairyo_gridtimer_core_NativeOptimizerBridge_nativeRenderAndroidAiAnswerHtml')){throw "Android formula renderer JNI method missing: $abi"}
                $nativeContents=[Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($library))
                foreach($asset in $mathEmbeddedAssets.Keys){if(!$nativeContents.Contains($mathEmbeddedAssets[$asset])){throw "Exact offline formula asset is missing from packaged native library: $abi $asset"}}
            }
        }
        foreach($entry in $zip.Entries | Where-Object FullName -like '*.dex'){
            $stream=$entry.Open(); $memory=[IO.MemoryStream]::new()
            try { $stream.CopyTo($memory); $text=[Text.Encoding]::Latin1.GetString($memory.ToArray()); foreach($className in @($dexNames.Keys)){$package=if($className -eq 'NoteCollectionPartitionKt'){'core'}else{'ui'}; if($text.Contains('Lcom/ofairyo/gridtimer/'+$package+'/'+$className+';')){$dexNames[$className]=$true}} }
            finally { $stream.Dispose(); $memory.Dispose() }
        }
    } finally { $zip.Dispose() }
    if(@($dexNames.Values | Where-Object {!$_}).Count){throw 'Required UI classes are missing from the packaged DEX'}
    if($aiAcceptance){$aiAcceptance['nativeBridgeAbis']=$abis; $aiAcceptance['packagedController']='com.ofairyo.gridtimer.ui.AiConnectionController'}
    if($mathAcceptance){$mathAcceptance['nativeBridgeAbis']=$abis; $mathAcceptance['exactOfflineAssetsEmbeddedInAllAbis']=$true; $mathAcceptance['packagedRenderer']='com.ofairyo.gridtimer.ui.AndroidRenderedMarkdownKt'}
} finally { $env:JAVA_HOME=$oldJava; $env:PATH=$oldPath }
$folders=[ordered]@{root=$releaseRoot;APK=(Join-Path $releaseRoot 'APK');current=(Join-Path $releaseRoot 'release_artifacts/current');build=(Split-Path $apk -Parent)}
if($Mode -eq 'publish'){
    $previous=$manifest.androidRelease.version
    $parts=$previous.Split('.')
    if($parts.Count -eq 4){
        $next=($parts[0..2] -join '.')+'.'+([int]$parts[3]+1)
    } elseif($parts.Count -eq 3) {
        $next=$previous+'.1'
    } elseif($previous -eq '2.23') {
        $next='2.23.0.1'
    } else {
        throw 'Unexpected Android release version format'
    }
    $explicitVersionTransition=(($previous -eq '2.22.49.13' -and $releaseVersion -eq '2.23') -or ($previous -eq '2.23' -and $releaseVersion -eq '2.23.1') -or ($previous -eq '2.23.1.3' -and $releaseVersion -eq '2.23.2'))
    if($previous -ne $releaseVersion -and $next -ne $releaseVersion -and !$explicitVersionTransition){throw 'Unexpected Android release version change'}
    if($releaseCode -ne ([int]$manifest.androidRelease.versionCode+1)){throw 'Android versionCode must increase by one'}
    foreach($role in @('root','APK')){
        $target=Assert-ReleasePath (Join-Path $folders[$role] $name)
        if((Test-Path -LiteralPath $target) -and (Get-FileHash -LiteralPath $target).Hash.ToLowerInvariant() -ne $hash){throw "Same-version package differs: $target"}
        Copy-Item -LiteralPath $apk -Destination $target -Force
    }
    $apkEntry=@($manifest.files | Where-Object role -eq 'apk')
    if($apkEntry.Count -ne 1){throw 'Expected exactly one APK manifest entry'}
    $oldCurrent=Assert-ReleasePath (Join-Path $folders.current $apkEntry[0].file_name)
    $newCurrent=Assert-ReleasePath (Join-Path $folders.current $name)
    $archive=Assert-ReleasePath (Join-Path $releaseRoot 'old_apks')
    New-Item -ItemType Directory -Force -Path $archive | Out-Null
    $oldArchive=Assert-ReleasePath (Join-Path $archive ([IO.Path]::GetFileNameWithoutExtension($oldCurrent)+"_current_before_v$releaseVersion.apk"))
    $oldCurrentHash=$apkEntry[0].sha256
    $apkEntry[0].file_name=$name; $apkEntry[0].size=(Get-Item -LiteralPath $apk).Length; $apkEntry[0].sha256=$hash
    $manifest.androidRelease=[ordered]@{version=$releaseVersion;versionCode=$releaseCode;androidOnly=$true;verification="release_artifacts/verification/v$releaseVersion";sourceSnapshotSha256=$build.sourceSnapshotSha256;sha256=$hash;createdAt=(Get-Date).ToUniversalTime().ToString('o')}
    # Stage outside current: the stable entry accepts exactly the declared files.
    $pending=Assert-ReleasePath (Join-Path $evidence 'release_manifest.pending.json')
    [IO.File]::WriteAllText($pending,($manifest | ConvertTo-Json -Depth 30),[Text.UTF8Encoding]::new($false))
    $normalizedRoot=[IO.Path]::GetFullPath((Join-Path $releaseRoot 'release_artifacts')).Replace('/','\').ToLowerInvariant().TrimEnd('\')
    $mutexHash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($normalizedRoot))).ToLowerInvariant()
    $releaseMutex=[Threading.Mutex]::new($false,('Local\TenRate.ReleaseArtifacts.'+$mutexHash))
    $ownsMutex=$false; $movedOld=$false; $copiedNew=$false
    function Restore-AndroidPublication {
        if($copiedNew -and (Test-Path -LiteralPath $newCurrent)){
            if((Get-FileHash -LiteralPath $newCurrent).Hash.ToLowerInvariant() -ne $hash){throw 'Cannot roll back a concurrently changed APK'}
            Remove-Item -LiteralPath $newCurrent -Force
        }
        if($movedOld){
            if((Get-FileHash -LiteralPath $oldArchive).Hash.ToLowerInvariant() -ne $oldCurrentHash){throw 'Cannot restore an altered previous APK'}
            Move-Item -LiteralPath $oldArchive -Destination $oldCurrent
        }
        [IO.File]::WriteAllText($manifestPath,$beforeText,[Text.UTF8Encoding]::new($false))
    }
    try {
        try {$ownsMutex=$releaseMutex.WaitOne(30000)} catch [Threading.AbandonedMutexException] {$ownsMutex=$true}
        if(!$ownsMutex){throw 'Another release operation is active'}
        if(Test-Path -LiteralPath (Join-Path $releaseRoot 'release_artifacts/.release-transaction-v1.json')){throw 'Resolve the existing release transaction before publishing Android'}
        if([IO.File]::ReadAllText($manifestPath) -ne $beforeText){throw 'Release manifest changed concurrently'}
        try {
            if($oldCurrent -ne $newCurrent){
                if((Get-FileHash -LiteralPath $oldCurrent).Hash.ToLowerInvariant() -ne $oldCurrentHash -or (Test-Path -LiteralPath $oldArchive)){throw 'Previous Android release cannot be archived safely'}
                if(Test-Path -LiteralPath $newCurrent){throw 'Unexpected candidate APK in current'}
                Move-Item -LiteralPath $oldCurrent -Destination $oldArchive
                $movedOld=$true
                Copy-Item -LiteralPath $apk -Destination $newCurrent
                $copiedNew=$true
            } elseif((Get-FileHash -LiteralPath $newCurrent).Hash.ToLowerInvariant() -ne $hash){throw 'Same-version current APK differs'}
            Move-Item -LiteralPath $pending -Destination $manifestPath -Force
        } catch {Restore-AndroidPublication; throw}
    } finally {if($ownsMutex){$releaseMutex.ReleaseMutex()}; $releaseMutex.Dispose()}
    $launcher=Join-Path $releaseRoot 'release_artifacts/desktop_entry/TenRate_Desktop_Launcher.exe'
    $process=Start-Process -FilePath $launcher -ArgumentList @('--sync-supervisor','--resolve-only') -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $evidence 'windows_entry_resolution.log') -RedirectStandardError (Join-Path $evidence 'windows_entry_resolution_errors.log')
    if($process.ExitCode -ne 0){
        $releaseMutex=[Threading.Mutex]::new($false,('Local\TenRate.ReleaseArtifacts.'+$mutexHash)); $ownsMutex=$false
        try {
            try {$ownsMutex=$releaseMutex.WaitOne(30000)} catch [Threading.AbandonedMutexException] {$ownsMutex=$true}
            if(!$ownsMutex){throw 'Cannot acquire the release mutex for rollback'}
            Restore-AndroidPublication
        } finally {if($ownsMutex){$releaseMutex.ReleaseMutex()}; $releaseMutex.Dispose()}
        throw 'Windows entry rejected the updated manifest; previous exact release set restored'
    }
    foreach($role in $folders.Keys){
        foreach($old in Get-ChildItem -LiteralPath $folders[$role] -File -Filter '*.apk'){
            if($old.Name -eq $name){continue}
            if($old.Name -notmatch '^(tenfold|grid_timer_app)_v'){throw "Unrecognized APK: $($old.Name)"}
            $source=Assert-ReleasePath $old.FullName
            $destination=Assert-ReleasePath (Join-Path $archive ($old.BaseName+"_${role}_before_v$releaseVersion.apk"))
            if(Test-Path -LiteralPath $destination){throw "Archive destination already exists: $destination"}
            $oldHash=(Get-FileHash -LiteralPath $source).Hash
            Move-Item -LiteralPath $source -Destination $destination
            if((Get-FileHash -LiteralPath $destination).Hash -ne $oldHash){throw 'Archived APK hash mismatch'}
        }
    }
}
if($Mode -eq 'verify' -and $manifest.androidRelease.version -ne $releaseVersion){
    $unexpected=@(Get-ChildItem -LiteralPath (Join-Path $releaseRoot 'app') -Recurse -File | Where-Object {$_.Extension -eq '.aab' -or ($_.Extension -eq '.apk' -and $_.FullName -ne $apk)})
    if($unexpected.Count){throw 'Non-delivery Android packages remain in app build outputs'}
    $receipt=[ordered]@{passed=$true;version=$releaseVersion;versionCode=$releaseCode;sha256=$hash;certificateSha256=$certificate;upgradeCertificateUnchanged=$true;sourceSnapshotSha256=$build.sourceSnapshotSha256;alignment16KVerified=$true;nativeAbis=$abis;packagedUiClasses=$dexNames;formalApkOnly=$true;noDeviceOperations=$true;windowsVersion=$windowsVersion;windowsArtifactsUnchanged=$true;completed=(Get-Date).ToUniversalTime().ToString('o')}
    if($aiAcceptance){$receipt['aiConnectionAcceptance']=$aiAcceptance}
    if($mathAcceptance){$receipt['mathRenderingAcceptance']=$mathAcceptance}
    if($legalAcceptance){$receipt['legalLifecycleAcceptance']=$legalAcceptance}
    if($legalWorkflowAcceptance){$receipt['legalWorkflowAcceptance']=$legalWorkflowAcceptance}
    if($financeAcceptance){$receipt['financeWorkspaceAcceptance']=$financeAcceptance}
    $receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $evidence 'pre_publish_package_check.json') -Encoding utf8
    [pscustomobject]$receipt | Select-Object passed,version,versionCode,sha256
    return
}
$published=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if($published.version -ne $windowsVersion -or $published.androidRelease.version -ne $releaseVersion -or $published.androidRelease.sha256 -ne $hash){throw 'Published version or digest mismatch'}
$copies=foreach($role in $folders.Keys){
    $path=Join-Path $folders[$role] $name
    if((Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ne $hash){throw "Published APK mismatch: $role"}
    [ordered]@{role=$role;path=$path;sha256=$hash;bytes=(Get-Item -LiteralPath $path).Length}
}
foreach($file in $windows){if((Get-FileHash -LiteralPath $file.path).Hash.ToLowerInvariant() -ne $file.sha256 -or (Get-Item -LiteralPath $file.path).LastWriteTimeUtc.Ticks -ne $file.modified){throw 'A retained Windows file changed'}}
$unexpected=@(Get-ChildItem -LiteralPath (Join-Path $releaseRoot 'app') -Recurse -File | Where-Object {$_.Extension -eq '.aab' -or ($_.Extension -eq '.apk' -and $_.FullName -ne $apk)})
if($unexpected.Count){throw 'Non-delivery Android packages remain in app build outputs'}
$receipt=[ordered]@{passed=$true;version=$releaseVersion;versionCode=$releaseCode;sha256=$hash;certificateSha256=$certificate;upgradeCertificateUnchanged=$true;sourceSnapshotSha256=$build.sourceSnapshotSha256;alignment16KVerified=$true;nativeAbis=$abis;packagedUiClasses=$dexNames;formalApkOnly=$true;noDeviceOperations=!(Test-Path -LiteralPath (Join-Path $evidence 'device_installation.json'));windowsVersion=$windowsVersion;windowsArtifactsUnchanged=$true;files=$copies;completed=(Get-Date).ToUniversalTime().ToString('o')}
if($aiAcceptance){$receipt['aiConnectionAcceptance']=$aiAcceptance}
if($mathAcceptance){$receipt['mathRenderingAcceptance']=$mathAcceptance}
if($legalAcceptance){$receipt['legalLifecycleAcceptance']=$legalAcceptance}
if($legalWorkflowAcceptance){$receipt['legalWorkflowAcceptance']=$legalWorkflowAcceptance}
if($financeAcceptance){$receipt['financeWorkspaceAcceptance']=$financeAcceptance}
$receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $evidence $(if($Mode -eq 'publish'){'delivery_receipt.json'}else{'post_publish_package_check.json'})) -Encoding utf8
[pscustomobject]$receipt | Select-Object passed,version,versionCode,sha256
