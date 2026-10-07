# v0.0.1 - Verify task Agent authorization and save-confirmation gates with isolated JUnit mutations.
[CmdletBinding()]
param([string]$Version = '2.23.2.17')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android Agent mutation version' }
$sourcePath = Join-Path $root 'native/gridtimer_native/src/sourcegen/android_ai_workflow.rs'
$sourceRelativePath = 'native/gridtimer_native/src/sourcegen/android_ai_workflow.rs'
$evidence = Join-Path $root "release_artifacts/verification/v$Version/agent_mutation"
$utf8 = [Text.UTF8Encoding]::new($false)
$sourceBefore = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$sourceText = [IO.File]::ReadAllText($sourcePath)
$sourceHashBefore = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($sourceText))).ToLowerInvariant()
function Get-EmbeddedKotlin([string]$Text, [string]$Name) {
    $match = [regex]::Match($Text, '(?s)pub const ' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if (!$match.Success) { throw "Rust-owned Kotlin literal is missing: $Name" }
    $match.Groups['body'].Value.Replace("`r`n", "`n")
}
function Get-Dependency([string]$Coordinate, [string]$Version, [string]$Name) {
    $cache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
    $matches = @(Get-ChildItem -LiteralPath (Join-Path $cache ($Coordinate + '/' + $Version)) -File -Recurse | Where-Object Name -eq $Name)
    if ($matches.Count -ne 1) { throw "Installed mutation-test dependency is missing or ambiguous: $Name" }
    $matches[0].FullName
}
function Replace-Unique([string]$Text, [string]$Before, [string]$After) {
    if ([regex]::Matches($Text, [regex]::Escape($Before)).Count -ne 1) { throw 'Agent mutation anchor is missing or ambiguous' }
    $Text.Replace($Before, $After)
}
$contents = Get-EmbeddedKotlin $sourceText 'CONTENTS'
$tests = Get-EmbeddedKotlin $sourceText 'TEST_CONTENTS'
$testNames = @([regex]::Matches($tests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$requiredTests = @('agentRequiresExplicitAuthorizationConfiguredModelAndReadableDocuments','agentDuplicateOrStaleWorkspaceCannotContinueOrSave','agentOnlySavesACompletedDraftAfterUserConfirmation','agentDraftContextIncludesWorkspaceQuestionConfigurationAndSelectedPageContent','agentScopeExcludesDeletedEncryptedAndOutOfFolderPages')
if (!$testNames.Count -or @($requiredTests | Where-Object { $_ -notin $testNames }).Count) { throw 'Agent authorization or durable-save business tests are missing' }
$agentCompiler = @(
    (Get-Dependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-Dependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-Dependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-Dependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-Dependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-Dependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-Dependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$agentRuntime = @(
    (Get-Dependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-Dependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-Dependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-Dependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$java = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
if (!(Test-Path -LiteralPath $java -PathType Leaf)) { throw 'Configured Java 17 runtime is unavailable' }
$cases = @(
    @{ name='baseline'; mutation=''; requiredFailure='' },
    @{ name='cosmeticCopy'; mutation='copy'; requiredFailure='' },
    @{ name='removedAuthorization'; mutation='authorization'; requiredFailure='agentRequiresExplicitAuthorizationConfiguredModelAndReadableDocuments' },
    @{ name='removedSaveConfirmation'; mutation='confirmation'; requiredFailure='agentOnlySavesACompletedDraftAfterUserConfirmation' }
)
$results = @()
foreach ($case in $cases) {
    $caseDir = Join-Path $evidence $case.name
    New-Item -ItemType Directory -Force -Path $caseDir | Out-Null
    $helper = $contents
    $mutatedSourceText = $sourceText
    switch ($case.mutation) {
        'copy' {
            $copyBefore = @'
                            Text(
                                "我已核对上方完整资料、接收方和模型，并授权本次发送。",
                                modifier = Modifier.clickable { agentPreviewAcknowledged = showSendingContent },
                                style = MaterialTheme.typography.bodySmall
                            )
'@
            $copyAfter = @'
                            Text(
                                "我已查看本次范围、接收方和模型，确认开始。",
                                modifier = Modifier.clickable { agentPreviewAcknowledged = showSendingContent },
                                style = MaterialTheme.typography.bodyMedium
                            )
'@
            $mutatedSourceText = Replace-Unique $mutatedSourceText $copyBefore.TrimEnd("`r", "`n") $copyAfter.TrimEnd("`r", "`n")
            if ($mutatedSourceText -ceq $sourceText) { throw 'Cosmetic mutation did not alter the production UI source' }
        }
        'authorization' {
            $helper = Replace-Unique $helper 'closed || pending != null || !authorized || !configured || question.isBlank() || documentCount !in 1..30' 'closed || pending != null || !configured || question.isBlank() || documentCount !in 1..30'
        }
        'confirmation' {
            $helper = Replace-Unique $helper '!closed && identityCurrent && contextCurrent && hasDraft && userConfirmed' '!closed && identityCurrent && contextCurrent && hasDraft'
        }
    }
    $helperFile = Join-Path $caseDir 'KnowledgeAiRequestBoundary.kt'
    $testFile = Join-Path $caseDir 'KnowledgeAiRequestBoundaryTest.kt'
    $configurationStub = "internal data class AiConfiguration(val apiKey: String, val baseUrl: String, val model: String)`n"
    $helper = Replace-Unique $helper "package com.ofairyo.gridtimer.ui`n" "package com.ofairyo.gridtimer.ui`n$configurationStub"
    [IO.File]::WriteAllText($helperFile, $helper, $utf8)
    [IO.File]::WriteAllText($testFile, $tests, $utf8)
    $classes = Join-Path $caseDir 'classes'
    $compileLog = Join-Path $caseDir 'compile.log'
    $testLog = Join-Path $caseDir 'tests.log'
    & $java -Xmx384m -cp ($agentCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name android_agent_mutation -classpath ($agentRuntime -join ';') -d $classes $helperFile $testFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Agent mutation did not compile: $($case.name)" }
    & $java -Xmx256m -cp ($classes + ';' + ($agentRuntime -join ';')) org.junit.runner.JUnitCore 'com.ofairyo.gridtimer.ui.KnowledgeAiRequestBoundaryTest' *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = [string]::IsNullOrEmpty([string]$case.requiredFailure)
    $observedFailure = !$expectedPass -and $log.Contains($case.requiredFailure + '(com.ofairyo.gridtimer.ui.KnowledgeAiRequestBoundaryTest)') -and $log.Contains('java.lang.AssertionError')
    $allTestsObserved = if ($expectedPass) { $log.Contains("OK ($($testNames.Count) tests)") } else { $log.Contains("Tests run: $($testNames.Count),") }
    $passed = $compileExit -eq 0 -and $allTestsObserved -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $observedFailure })
    $results += [ordered]@{ name=$case.name; compileExit=$compileExit; testExit=$testExit; expectedPass=$expectedPass; requiredFailureTest=$case.requiredFailure; requiredFailureObserved=$observedFailure; allTestsObserved=$allTestsObserved; passed=$passed; helperSha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($helper))).ToLowerInvariant(); testsSha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($tests))).ToLowerInvariant(); sourceMutationSha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($mutatedSourceText))).ToLowerInvariant() }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected Agent mutation result: $($case.name)" }
    Write-Output "$($case.name) verified: $($testNames.Count) tests, compile $compileExit, test $testExit"
}
$sourceAfter = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($sourceBefore -cne $sourceAfter -or $sourceHashBefore -cne [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes([IO.File]::ReadAllText($sourcePath)))).ToLowerInvariant()) { throw 'Production Rust source changed during isolated mutation verification' }
$receipt = [ordered]@{ passed=$true; version=$Version; sourcePath=$sourceRelativePath; sourceSha256=$sourceBefore; testCount=$testNames.Count; testNames=$testNames; cases=$results; productionUnchanged=$true; noNetworkRequests=$true; hostOnly=$true; scope='Android AI Agent request authorization, context-bound save confirmation, and UI copy/style mutation only. Does not assert real provider or device execution.'; checkedAt=[DateTimeOffset]::Now.ToString('o') }
[IO.File]::WriteAllText((Join-Path $evidence 'receipt.json'), ($receipt | ConvertTo-Json -Depth 12), $utf8)
Write-Output "Android $Version task Agent mutation acceptance completed"
