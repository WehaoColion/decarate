# v0.0.1 - Verify legal request authorization and workflow domain states with real JUnit mutations.
[CmdletBinding()]
param([string]$Version = '')
$ErrorActionPreference = 'Stop'
$workflowRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) {
    $gradle = [IO.File]::ReadAllText((Join-Path $workflowRoot 'app/build.gradle'))
    $Version = [regex]::Match($gradle, "versionName '([^']+)'").Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android workflow mutation version' }
$workflowSourcePath = 'native/gridtimer_native/src/sourcegen/android_legal_workflow.rs'
$workflowSource = Join-Path $workflowRoot $workflowSourcePath
$workflowBaseSourcePath = 'native/gridtimer_native/src/sourcegen/legal_risk_ui_source.rs'
$workflowBaseSource = Join-Path $workflowRoot $workflowBaseSourcePath
$workflowEvidence = Join-Path $workflowRoot "release_artifacts/verification/v$Version/legal_workflow_mutation"
$workflowUtf8 = [Text.UTF8Encoding]::new($false)
function Get-WorkflowLiteral([string]$Source, [string]$Name) {
    $raw = [IO.File]::ReadAllText($Source)
    $literal = [regex]::Match($raw, '(?s)pub const ' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if (!$literal.Success) { throw "Actual Rust literal is missing: $Name" }
    $literal.Groups['body'].Value.Replace("`r`n", "`n")
}
function Get-WorkflowTextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Get-WorkflowDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $path = Join-Path $workflowCache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $path -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
function Replace-WorkflowGuard([string]$Source, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Source, [regex]::Escape($Anchor)).Count -ne 1) { throw 'Actual production authorization mutation anchor is missing or ambiguous' }
    $Source.Replace($Anchor, $Replacement)
}
$workflowBefore = (Get-FileHash -LiteralPath $workflowSource -Algorithm SHA256).Hash.ToLowerInvariant()
$workflowBaseBefore = (Get-FileHash -LiteralPath $workflowBaseSource -Algorithm SHA256).Hash.ToLowerInvariant()
$workflowVerifierHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$workflowHelperLiteral = Get-WorkflowLiteral $workflowSource 'HELPERS'
$workflowTests = Get-WorkflowLiteral $workflowSource 'TEST_CONTENTS'
$workflowBaseUi = Get-WorkflowLiteral $workflowBaseSource 'CONTENTS'
$workflowStart = $workflowBaseUi.IndexOf('internal fun legalSendReady(')
$workflowEnd = $workflowBaseUi.IndexOf('/** Separate from the finance overview pager.')
if ($workflowStart -lt 0 -or $workflowEnd -le $workflowStart) { throw 'Actual legal request helper boundaries are missing' }
$workflowBaseHelper = $workflowBaseUi.Substring($workflowStart, $workflowEnd - $workflowStart)
$workflowHelper = "package com.ofairyo.gridtimer.ui`n`n" + $workflowBaseHelper + $workflowHelperLiteral
$workflowTestFile = "package com.ofairyo.gridtimer.ui`nimport org.junit.Test`nimport org.junit.Assert.assertTrue`nimport org.junit.Assert.assertFalse`n" + $workflowTests
$workflowTestNames = @([regex]::Matches($workflowTests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$workflowRequiredTests = @('requestOwnershipRequiresFreshBoundSingleUseAuthorization','changedScanEndpointModelOrKeyCannotReuseConsent','busyOrStalePreviewCannotBeConfirmed','concurrentConfirmationCanConsumeAuthorizationOnlyOnce','unavailableDomainStatesCannotConsumeSendAuthorization')
if (!$workflowTestNames.Count -or @($workflowRequiredTests | Where-Object { $_ -notin $workflowTestNames }).Count) { throw 'Actual legal workflow authorization business tests are incomplete' }
$workflowCache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
$workflowCompiler = @(
    (Get-WorkflowDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-WorkflowDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-WorkflowDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-WorkflowDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-WorkflowDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-WorkflowDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-WorkflowDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$workflowRuntime = @(
    (Get-WorkflowDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-WorkflowDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-WorkflowDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-WorkflowDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$workflowJava = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
$workflowDependencies = @(@($workflowCompiler + $workflowRuntime + $workflowJava | Sort-Object -Unique) | ForEach-Object {
    if (!(Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Installed verification dependency is missing: $_" }
    [ordered]@{ path=$_; sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$workflowTestClass = 'com.ofairyo.gridtimer.ui.LegalAnalysisWorkflowTest'
$workflowResults = @()
foreach ($caseName in @('baseline','cosmetic','removedSendAuthorization','removedConfigurationBinding','removedReadinessGuard','restored')) {
    $directory = Join-Path $workflowEvidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $helper = $workflowHelper
    $requiredFailure = ''
    switch ($caseName) {
        'cosmetic' {
            $helper = Replace-WorkflowGuard $helper '"发送并开始分析"' '"开始发送 AI 分析"'
            $helper = Replace-WorkflowGuard $helper '"正在整理本机资料"' '"资料整理中"'
            $helper = Replace-WorkflowGuard $helper '"已有 $evidenceCount 条可读资料，点击后核对接收方并确认发送。"' '"可发送 $evidenceCount 条，先核对服务和内容。"'
        }
        'removedSendAuthorization' {
            $helper = Replace-WorkflowGuard $helper 'if (!consume(scanId, baseUrl, model, apiKey, ready)) return null' '// Authorization deliberately removed in this isolated mutation.'
            $requiredFailure = 'requestOwnershipRequiresFreshBoundSingleUseAuthorization'
        }
        'removedConfigurationBinding' {
            $anchor = "this.scanId != scanId || this.baseUrl != baseUrl ||`n            this.model != model || this.apiKey != apiKey"
            $helper = Replace-WorkflowGuard $helper $anchor 'false'
            $requiredFailure = 'changedScanEndpointModelOrKeyCannotReuseConsent'
        }
        'removedReadinessGuard' {
            $helper = Replace-WorkflowGuard $helper 'consumed || !ready ||' 'consumed ||'
            $requiredFailure = 'busyOrStalePreviewCannotBeConfirmed'
        }
    }
    $helperFile = Join-Path $directory 'LegalWorkflowAuthorization.kt'
    $testFile = Join-Path $directory 'LegalAnalysisWorkflowTest.kt'
    [IO.File]::WriteAllText($helperFile, $helper, $workflowUtf8)
    [IO.File]::WriteAllText($testFile, $workflowTestFile, $workflowUtf8)
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $workflowJava -Xmx384m -cp ($workflowCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name legal_workflow_mutation -classpath ($workflowRuntime -join ';') -d $classes $helperFile $testFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Legal workflow mutation failed to compile, not an accepted business failure: $caseName" }
    & $workflowJava -Xmx256m -cp ($classes + ';' + ($workflowRuntime -join ';')) org.junit.runner.JUnitCore $workflowTestClass *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = !$requiredFailure
    $observed = !$expectedPass -and $log.Contains($requiredFailure + '(' + $workflowTestClass + ')') -and $log.Contains('java.lang.AssertionError')
    $allTests = if ($expectedPass) { $log.Contains("OK ($($workflowTestNames.Count) tests)") } else { $log.Contains("Tests run: $($workflowTestNames.Count),") }
    $passed = $compileExit -eq 0 -and $allTests -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $observed })
    $workflowResults += [ordered]@{ name=$caseName; compileExit=$compileExit; testExit=$testExit; expectedPass=$expectedPass; requiredFailureTest=$requiredFailure; requiredFailureObserved=$observed; allBusinessTestsObserved=$allTests; passed=$passed; helperSha256=(Get-WorkflowTextHash $helper); testsSha256=(Get-WorkflowTextHash $workflowTests); compiledTestSha256=(Get-WorkflowTextHash $workflowTestFile); compileLogSha256=(Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant(); testLogSha256=(Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant() }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected legal workflow authorization mutation result: $caseName" }
    Write-Output "$caseName verified: $($workflowTestNames.Count) business tests, compile $compileExit, test $testExit"
}
$workflowAfter = (Get-FileHash -LiteralPath $workflowSource -Algorithm SHA256).Hash.ToLowerInvariant()
$workflowBaseAfter = (Get-FileHash -LiteralPath $workflowBaseSource -Algorithm SHA256).Hash.ToLowerInvariant()
if ($workflowBefore -cne $workflowAfter -or $workflowBaseBefore -cne $workflowBaseAfter -or (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $workflowVerifierHash) { throw 'Production source or verifier changed during legal workflow mutation verification' }
foreach ($dependency in $workflowDependencies) {
    if ((Get-FileHash -LiteralPath $dependency.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $dependency.sha256) { throw 'Installed compiler/runtime dependency changed during verification' }
}
[IO.File]::WriteAllText((Join-Path $workflowEvidence 'receipt.json'), ([ordered]@{ passed=$true; version=$Version; productionUnchanged=$true; sourcePath=$workflowSourcePath; sourceSha256Before=$workflowBefore; sourceSha256After=$workflowAfter; baseSourcePath=$workflowBaseSourcePath; baseSourceSha256Before=$workflowBaseBefore; baseSourceSha256After=$workflowBaseAfter; verifierPath='tools/verify_legal_workflow_mutation.ps1'; verifierSha256=$workflowVerifierHash; helperSha256=(Get-WorkflowTextHash $workflowHelper); workflowHelperSha256=(Get-WorkflowTextHash $workflowHelperLiteral); baseHelperSha256=(Get-WorkflowTextHash $workflowBaseHelper); testsSha256=(Get-WorkflowTextHash $workflowTests); compiledTestSha256=(Get-WorkflowTextHash $workflowTestFile); tests=$workflowTestNames.Count; testNames=$workflowTestNames; cases=$workflowResults; dependencies=$workflowDependencies; hostOnly=$true; networkRequests=0; noDeviceOperations=$true; scope='Actual Rust-owned legal workflow policy and single-use authorization request boundary. JUnit domain failures must compile successfully. Does not assert device UI or provider execution.'; checkedAt=[DateTimeOffset]::Now.ToString('o') } | ConvertTo-Json -Depth 15), $workflowUtf8)
Write-Output "Android $Version legal workflow mutation acceptance completed"
