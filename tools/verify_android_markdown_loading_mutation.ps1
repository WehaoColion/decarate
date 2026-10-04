# v2.23.2.7 - Verify offline document identity, main-frame authorization and formula readiness.
[CmdletBinding()]
param([string]$Version = '2.23.2.7')
$ErrorActionPreference = 'Stop'
$loadingRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android loading mutation version' }
$loadingSourcePath = 'native/gridtimer_native/src/sourcegen/android_ai_answer_ui.rs'
$loadingSource = Join-Path $loadingRoot $loadingSourcePath
$loadingEvidence = Join-Path $loadingRoot "release_artifacts/verification/v$Version/markdown_loading_mutation"
$loadingUtf8 = [Text.UTF8Encoding]::new($false)

function Get-LoadingLiteral([string]$Path, [string]$Name) {
    $raw = [IO.File]::ReadAllText($Path)
    $marker = [regex]::Match($raw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if (!$marker.Success) { throw "Actual Rust literal is missing: $Name" }
    $from = $marker.Index + $marker.Length
    $to = $raw.IndexOf('"' + $marker.Groups['hash'].Value + ';', $from)
    if ($to -lt $from) { throw "Rust literal terminator is missing: $Name" }
    $raw.Substring($from, $to - $from).Replace("`r`n", "`n")
}
function Get-LoadingTextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Replace-LoadingGuard([string]$Value, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Value, [regex]::Escape($Anchor)).Count -ne 1) { throw 'Actual loading guard is not unique' }
    $Value.Replace($Anchor, $Replacement)
}

$loadingSourceHash = (Get-FileHash -LiteralPath $loadingSource -Algorithm SHA256).Hash.ToLowerInvariant()
$loadingVerifierHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$loadingHelper = Get-LoadingLiteral $loadingSource 'BOUNDARY_CONTENTS'
$loadingTests = Get-LoadingLiteral $loadingSource 'TEST_CONTENTS'
$loadingUi = Get-LoadingLiteral $loadingSource 'CONTENTS'
$loadingTestNames = @([regex]::Matches($loadingTests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$loadingRequiredTests = @('networkAndActiveDocumentsCannotLoad','onlyExactCurrentMainDocumentCanLoadOffline','plainMarkdownNeedsItsDocumentAndBodyWithoutMathBootstrap','incompleteFormulaOrForeignDocumentCannotBecomeReady','explicitFailureCanRetryWithoutAcceptingOldCompletion','disposedOrSupersededPageCannotResizeCurrentAnswer','staleWidthAndInvalidHeightCannotHideAnswer')
if ($loadingTestNames.Count -lt 7 -or @($loadingRequiredTests | Where-Object { $_ -notin $loadingTestNames }).Count) { throw 'Actual loading boundary business tests are incomplete' }

$loadingInventory = Get-Content -LiteralPath (Join-Path $loadingRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$loadingCompiler = @($loadingInventory.compiler)
$loadingRuntime = @($loadingInventory.runtime | Where-Object { $_ -match '(kotlin-stdlib-1\.9\.24|junit-4\.13\.2|hamcrest-core-1\.3|annotations-23\.0\.0)\.jar$' } | Sort-Object -Unique)
if ($loadingCompiler.Count -lt 1 -or $loadingRuntime.Count -ne 4) { throw 'Installed Kotlin/JUnit compiler inventory is incomplete' }
$loadingJava = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$loadingDependencies = @(@($loadingCompiler + $loadingRuntime + $loadingJava | Sort-Object -Unique) | ForEach-Object {
    if (!(Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Installed verification dependency is missing: $_" }
    [ordered]@{ path=$_; sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$loadingTestClass = 'com.ofairyo.gridtimer.ui.AndroidMarkdownRenderBoundaryTest'
$loadingCases = @('baseline','cosmetic','removed_document_identity','removed_main_frame_authorization','removed_formula_completion','restored')
$loadingResults = @()
foreach ($caseName in $loadingCases) {
    $directory = Join-Path $loadingEvidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $helper = $loadingHelper
    $ui = $loadingUi
    $failureTest = ''
    switch ($caseName) {
        'cosmetic' { $ui = Replace-LoadingGuard $ui '排版未完成，已保留原文。' '内容已保留，可以重新排版。' }
        'removed_document_identity' {
            $helper = Replace-LoadingGuard $helper 'androidMarkdownDocumentTrusted(documentUrl) && url == documentUrl' 'androidMarkdownDocumentTrusted(documentUrl)'
            $failureTest = 'onlyExactCurrentMainDocumentCanLoadOffline'
        }
        'removed_main_frame_authorization' {
            $helper = Replace-LoadingGuard $helper '(mainFrame && androidMarkdownDocumentAllowed(url, documentUrl))' '(androidMarkdownDocumentAllowed(url, documentUrl))'
            $failureTest = 'onlyExactCurrentMainDocumentCanLoadOffline'
        }
        'removed_formula_completion' {
            $helper = Replace-LoadingGuard $helper 'scriptReady && renderedCount.toLong() + errorCount.toLong() == formulaCount.toLong()' 'true'
            $failureTest = 'incompleteFormulaOrForeignDocumentCannotBecomeReady'
        }
    }
    $helperFile = Join-Path $directory 'AndroidMarkdownRenderBoundary.kt'
    $testFile = Join-Path $directory 'AndroidMarkdownRenderBoundaryTest.kt'
    [IO.File]::WriteAllText($helperFile, $helper, $loadingUtf8)
    [IO.File]::WriteAllText($testFile, $loadingTests, $loadingUtf8)
    # The UI copy is exported as evidence. The host suite executes only the actual
    # pure helper and tests; AndroidView rendering is covered separately on device.
    [IO.File]::WriteAllText((Join-Path $directory 'AndroidRenderedMarkdown.kt'), $ui, $loadingUtf8)
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $loadingJava -Xmx384m -cp ($loadingCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name markdown_loading_mutation -classpath ($loadingRuntime -join ';') -d $classes $helperFile $testFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Loading mutation failed to compile, not an accepted business failure: $caseName" }
    & $loadingJava -Xmx256m -cp ($classes + ';' + ($loadingRuntime -join ';')) org.junit.runner.JUnitCore $loadingTestClass *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = $caseName -in @('baseline','cosmetic','restored')
    $requiredFailureObserved = !$expectedPass -and $log -match ([regex]::Escape($failureTest + '(' + $loadingTestClass + ')')) -and $log.Contains('java.lang.AssertionError')
    $allBusinessTestsObserved = if ($expectedPass) { $log.Contains("OK ($($loadingTestNames.Count) tests)") } else { $log.Contains("Tests run: $($loadingTestNames.Count),") }
    $passed = $compileExit -eq 0 -and $allBusinessTestsObserved -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $requiredFailureObserved })
    $loadingResults += [ordered]@{
        name=$caseName; compileExit=$compileExit; testExit=$testExit; expectedPass=$expectedPass
        requiredFailureTest=$failureTest; requiredFailureObserved=$requiredFailureObserved
        allBusinessTestsObserved=$allBusinessTestsObserved; passed=$passed
        helperSha256=(Get-LoadingTextHash $helper); uiSha256=(Get-LoadingTextHash $ui); testsSha256=(Get-LoadingTextHash $loadingTests)
        compileLogSha256=(Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant()
        testLogSha256=(Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 50; throw "Unexpected loading mutation result: $caseName" }
    Write-Output "$caseName verified: $($loadingTestNames.Count) business tests, compile $compileExit, test $testExit"
}
$loadingAfterHash = (Get-FileHash -LiteralPath $loadingSource -Algorithm SHA256).Hash.ToLowerInvariant()
if ($loadingAfterHash -cne $loadingSourceHash -or (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $loadingVerifierHash) { throw 'Production source or verifier changed during loading mutation verification' }
foreach ($dependency in $loadingDependencies) {
    if ((Get-FileHash -LiteralPath $dependency.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $dependency.sha256) { throw 'Installed compiler/runtime dependency changed during verification' }
}
[IO.File]::WriteAllText((Join-Path $loadingEvidence 'receipt.json'), ([ordered]@{
    passed=$true; version=$Version; productionUnchanged=$true; sourcePath=$loadingSourcePath
    sourceSha256Before=$loadingSourceHash; sourceSha256After=$loadingAfterHash
    verifierPath='tools/verify_android_markdown_loading_mutation.ps1'; verifierSha256=$loadingVerifierHash
    helperLiteral='BOUNDARY_CONTENTS'; testsLiteral='TEST_CONTENTS'; uiLiteral='CONTENTS'
    helperSha256=(Get-LoadingTextHash $loadingHelper); testsSha256=(Get-LoadingTextHash $loadingTests); uiSha256=(Get-LoadingTextHash $loadingUi)
    tests=$loadingTestNames.Count; testNames=$loadingTestNames; cases=$loadingResults; dependencies=$loadingDependencies
    hostOnly=$true; networkRequests=0; noDeviceOperations=$true
    scope='Actual Rust-owned loading helper and tests, isolated JVM compilation. Cosmetic UI copy is exported; this receipt does not assert AndroidView or WebView execution.'
    checkedAt=[DateTimeOffset]::Now.ToString('o')
} | ConvertTo-Json -Depth 15), $loadingUtf8)
Write-Output "Android $Version offline document loading mutation acceptance completed"
