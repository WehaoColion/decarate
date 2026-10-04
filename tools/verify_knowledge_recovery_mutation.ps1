# v2.23.2 - Verify source-complete knowledge grouping and show-all state recovery.
param(
    [string]$Version,
    [switch]$NativeOnly,
    [switch]$CosmeticOnly
)
$ErrorActionPreference = 'Stop'
$knowledgeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = [regex]::Match([IO.File]::ReadAllText((Join-Path $knowledgeRoot 'app/build.gradle')), "versionName '([^']+)'").Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+(?:\.\d+)?$') { throw 'Invalid Android evidence version' }
$knowledgeEvidence = Join-Path $knowledgeRoot "release_artifacts/verification/v$Version/knowledge_recovery_mutation"
$knowledgeEmitter = Join-Path $knowledgeRoot 'native/gridtimer_native/src/sourcegen/android_note_collection_recovery.rs'
$filterEmitter = Join-Path $knowledgeRoot 'native/gridtimer_native/src/sourcegen/android_knowledge_filters.rs'
$sourceHashes = [ordered]@{}
$sourceHashes[$knowledgeEmitter] = (Get-FileHash -LiteralPath $knowledgeEmitter -Algorithm SHA256).Hash

function Get-KnowledgeLiteral([string]$SourcePath, [string]$Name) {
    $literalRaw = [IO.File]::ReadAllText($SourcePath)
    $literalMarker = [regex]::Match($literalRaw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if (!$literalMarker.Success) { throw "Missing production source literal $Name in $SourcePath" }
    $literalStart = $literalMarker.Index + $literalMarker.Length
    $literalEnd = $literalRaw.IndexOf('"' + $literalMarker.Groups['hash'].Value + ';', $literalStart)
    if ($literalEnd -lt 0) { throw "Missing production source literal end for $Name" }
    return $literalRaw.Substring($literalStart, $literalEnd - $literalStart)
}

$knowledgeProduction = Get-KnowledgeLiteral $knowledgeEmitter 'HELPER_CONTENTS'
$knowledgeTests = Get-KnowledgeLiteral $knowledgeEmitter 'TEST_CONTENTS'
$testClasses = @('com.ofairyo.gridtimer.core.NoteCollectionRecoveryTest')
$testCount = ([regex]'@Test fun ').Matches($knowledgeTests).Count
$filterProduction = $null
$filterTests = $null
$filterQueryFunction = $null
$controlsProduction = $null
if (Test-Path -LiteralPath $filterEmitter -PathType Leaf) {
    $sourceHashes[$filterEmitter] = (Get-FileHash -LiteralPath $filterEmitter -Algorithm SHA256).Hash
    $controlsProduction = Get-KnowledgeLiteral $filterEmitter 'CONTROLS'
}
if ([string]::IsNullOrWhiteSpace($controlsProduction)) { throw 'Actual knowledge controls source must be ready for the UI wording mutation' }
if (!$NativeOnly) {
    if (!(Test-Path -LiteralPath $filterEmitter -PathType Leaf)) { throw 'Knowledge filter state emitter must be ready before full mutation verification' }
    $sourceHashes[$filterEmitter] = (Get-FileHash -LiteralPath $filterEmitter -Algorithm SHA256).Hash
    $filterProduction = Get-KnowledgeLiteral $filterEmitter 'CONTENTS'
    $filterTests = Get-KnowledgeLiteral $filterEmitter 'TEST_CONTENTS'
    $testClasses += 'com.ofairyo.gridtimer.ui.KnowledgeFilterStateTest'
    $testCount += ([regex]'@Test fun ').Matches($filterTests).Count
    # The previous compiled release has a private query matcher. Compile the
    # exact source function with the visibility change applied by this renderer.
    # Its remaining dependencies use the real compiled release, with no stubs.
    $filterQueryEmitter = Join-Path $knowledgeRoot 'native/gridtimer_native/src/sourcegen/kotlin_sources.rs'
    $sourceHashes[$filterQueryEmitter] = (Get-FileHash -LiteralPath $filterQueryEmitter -Algorithm SHA256).Hash
    $filterQueryRaw = [IO.File]::ReadAllText($filterQueryEmitter)
    $filterQueryStart = $filterQueryRaw.IndexOf('private fun noteMatchesQuery(')
    $filterQueryEnd = $filterQueryRaw.IndexOf('internal fun NoteEntry.matchesQuickFilter(', $filterQueryStart)
    if ($filterQueryStart -lt 0 -or $filterQueryEnd -le $filterQueryStart) { throw 'Production query matching extraction boundary is missing' }
    $filterQueryFunction = "package com.ofairyo.gridtimer.ui`nimport com.ofairyo.gridtimer.data.*`n" + $filterQueryRaw.Substring($filterQueryStart, $filterQueryEnd - $filterQueryStart).Replace('private fun noteMatchesQuery(', 'internal fun noteMatchesQuery(')
}

$knowledgePrior = Get-Content -LiteralPath (Join-Path $knowledgeRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$knowledgeCompiled = Join-Path $knowledgeRoot 'app/build/tmp/kotlin-classes/release'
if (!(Test-Path -LiteralPath (Join-Path $knowledgeCompiled 'com/ofairyo/gridtimer/core/NativeNoteCollectionSection.class'))) {
    throw 'Compiled release note model is required; no model or production logic stubs are used'
}
$knowledgeCompiler = @($knowledgePrior.compiler)
foreach ($compilerEntry in $knowledgeCompiler) {
    if (!(Test-Path -LiteralPath $compilerEntry -PathType Leaf)) { throw "Installed Kotlin compiler dependency missing: $compilerEntry" }
}
$knowledgeRuntime = @($knowledgeCompiled) + @($knowledgePrior.runtime | ForEach-Object {
    if ($_ -match '^(.*?)\\app\\build\\(.*)$') { Join-Path $knowledgeRoot ('app/build/' + $Matches[2]) }
    else { $_ }
} | Where-Object { (Test-Path -LiteralPath $_ -PathType Leaf) -and $_.EndsWith('.jar') })
$knowledgeAndroidRuntime = @(& rg --files (Join-Path $env:USERPROFILE '.gradle/caches/transforms-4') -g android.jar)
if ($knowledgeAndroidRuntime.Count -ne 1) { throw 'Resolve the current Gradle mockable Android jar before knowledge JVM checks' }
$knowledgeRuntime += $knowledgeAndroidRuntime[0]
if (!($knowledgeRuntime | Where-Object { $_ -match 'junit-4\.13\.2\.jar$' })) { throw 'JUnit runtime dependency missing' }
$knowledgeJava = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
if (!(Test-Path -LiteralPath $knowledgeJava -PathType Leaf)) { throw 'Release Java 17 runtime is missing' }
$knowledgeUtf8 = [Text.UTF8Encoding]::new($false)
$knowledgeCases = @('baseline', 'cosmetic_only', 'removed_native_coverage_guard')
if (!$NativeOnly) { $knowledgeCases += 'removed_show_all_reset' }
if ($CosmeticOnly) { $knowledgeCases = @('cosmetic_only') }
$knowledgeResults = @()

foreach ($knowledgeCase in $knowledgeCases) {
    $caseDirectory = Join-Path $knowledgeEvidence $knowledgeCase
    New-Item -ItemType Directory -Force -Path $caseDirectory | Out-Null
    $knowledgeCaseProduction = $knowledgeProduction
    $filterCaseProduction = $filterProduction
    $caseControlsProduction = $controlsProduction
    $caseUiMutation = $null
    if ($knowledgeCase -eq 'cosmetic_only') {
        # Keep this tool ASCII so Windows PowerShell 5.1 cannot misdecode labels.
        $originalLabel = -join @([char]0x663e, [char]0x793a, [char]0x5168, [char]0x90e8)
        $mutatedLabel = -join @([char]0x67e5, [char]0x770b, [char]0x5168, [char]0x90e8)
        $cosmeticAnchor = 'Text("' + $originalLabel + '")'
        if (!$caseControlsProduction.Contains($cosmeticAnchor)) { throw 'Actual UI control wording mutation anchor is missing' }
        $caseControlsProduction = $caseControlsProduction.Replace($cosmeticAnchor, 'Text("' + $mutatedLabel + '")')
        if ($caseControlsProduction -ceq $controlsProduction) { throw 'Actual UI wording mutation did not change the controls source' }
        $caseUiMutation = [ordered]@{ originalLabel = $originalLabel; mutatedLabel = $mutatedLabel; actualSourceLiteral = 'android_knowledge_filters.rs CONTROLS'; sameBusinessHelperBytes = $knowledgeCaseProduction -ceq $knowledgeProduction -and $filterCaseProduction -ceq $filterProduction }
    }
    if ($knowledgeCase -eq 'removed_native_coverage_guard') {
        $coverageAnchor = 'offset == values.size && covered == count'
        if (!$knowledgeCaseProduction.Contains($coverageAnchor)) { throw 'Native source coverage guard is missing' }
        $knowledgeCaseProduction = $knowledgeCaseProduction.Replace($coverageAnchor, 'offset == values.size')
    }
    if ($knowledgeCase -eq 'removed_show_all_reset') {
        $resetAnchor = 'query = "", folderId = null, quickFilter = NoteQuickFilter.ALL, trashMode = false'
        if (!$filterCaseProduction.Contains($resetAnchor)) { throw 'Production show-all reset mutation anchor is missing' }
        $filterCaseProduction = $filterCaseProduction.Replace($resetAnchor, 'query = query, folderId = null, quickFilter = NoteQuickFilter.ALL, trashMode = false')
    }
    $knowledgeSourcePath = Join-Path $caseDirectory 'NoteCollectionPartition.kt'
    $knowledgeTestPath = Join-Path $caseDirectory 'NoteCollectionRecoveryTest.kt'
    [IO.File]::WriteAllText($knowledgeSourcePath, $knowledgeCaseProduction, $knowledgeUtf8)
    [IO.File]::WriteAllText($knowledgeTestPath, $knowledgeTests, $knowledgeUtf8)
    # Keep the exact UI fragment beside the business fixtures. The business tests
    # intentionally do not assert labels or styles; release compilation checks UI.
    $controlsPath = Join-Path $caseDirectory 'KnowledgeCollectionControls.kt'
    $controlsOriginalPath = Join-Path $caseDirectory 'KnowledgeCollectionControls_original.kt'
    [IO.File]::WriteAllText($controlsPath, $caseControlsProduction, $knowledgeUtf8)
    [IO.File]::WriteAllText($controlsOriginalPath, $controlsProduction, $knowledgeUtf8)
    if ($null -ne $caseUiMutation) {
        $caseUiMutation.originalSha256 = (Get-FileHash -LiteralPath $controlsOriginalPath -Algorithm SHA256).Hash
        $caseUiMutation.mutatedSha256 = (Get-FileHash -LiteralPath $controlsPath -Algorithm SHA256).Hash
    }
    $caseSources = @($knowledgeSourcePath, $knowledgeTestPath)
    if (!$NativeOnly) {
        $filterSourcePath = Join-Path $caseDirectory 'KnowledgeFilterState.kt'
        $filterTestPath = Join-Path $caseDirectory 'KnowledgeFilterStateTest.kt'
        $filterQueryPath = Join-Path $caseDirectory 'KnowledgeQueryMatcher.kt'
        [IO.File]::WriteAllText($filterSourcePath, $filterCaseProduction, $knowledgeUtf8)
        [IO.File]::WriteAllText($filterTestPath, $filterTests, $knowledgeUtf8)
        [IO.File]::WriteAllText($filterQueryPath, $filterQueryFunction, $knowledgeUtf8)
        $caseSources += @($filterSourcePath, $filterTestPath, $filterQueryPath)
    }
    $caseClasses = Join-Path $caseDirectory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $knowledgeJava -Xmx768m -cp ($knowledgeCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$knowledgeCompiled" -classpath ($knowledgeRuntime -join ';') -d $caseClasses @caseSources *> (Join-Path $caseDirectory 'compile.log')
    if ($LASTEXITCODE -ne 0) {
        Get-Content -LiteralPath (Join-Path $caseDirectory 'compile.log') -Tail 35
        throw "Knowledge mutation fixture compilation failed: $knowledgeCase"
    }
    & $knowledgeJava -Xmx512m -cp ($caseClasses + ';' + ($knowledgeRuntime -join ';')) org.junit.runner.JUnitCore @testClasses *> (Join-Path $caseDirectory 'tests.log')
    $caseExitCode = $LASTEXITCODE
    $caseLog = [IO.File]::ReadAllText((Join-Path $caseDirectory 'tests.log'))
    $caseExpectedPass = $knowledgeCase -in @('baseline', 'cosmetic_only')
    $caseFailureTest = switch ($knowledgeCase) {
        'removed_native_coverage_guard' { 'emptyFailedOrPartialNativeResultsCannotHideSourcePages' }
        'removed_show_all_reset' { 'showAllAtomicallyRestoresExistingDocumentsAndKeepsSortChoice' }
        default { '' }
    }
    $casePassed = if ($caseExpectedPass) {
        $caseExitCode -eq 0 -and $caseLog.Contains("OK ($testCount tests)")
    } else {
        $caseExitCode -ne 0 -and $caseLog.Contains('java.lang.AssertionError') -and $caseLog.Contains($caseFailureTest)
    }
    $knowledgeResults += [ordered]@{ case = $knowledgeCase; exitCode = $caseExitCode; expectedPass = $caseExpectedPass; passed = $casePassed; requiredFailureTest = $caseFailureTest; uiMutation = $caseUiMutation }
    if (!$casePassed) {
        Get-Content -LiteralPath (Join-Path $caseDirectory 'tests.log') -Tail 45
        throw "Unexpected knowledge mutation result: $knowledgeCase"
    }
    Write-Output "$knowledgeCase verified"
}
foreach ($sourcePath in $sourceHashes.Keys) {
    if ((Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash -cne $sourceHashes[$sourcePath]) { throw "Production source changed during mutation verification: $sourcePath" }
}
$knowledgeResultName = if ($CosmeticOnly) { 'cosmetic_mutation_result.json' } elseif ($NativeOnly) { 'native_mutation_result.json' } else { 'mutation_result.json' }
[ordered]@{
    passed = $true
    version = $Version
    tests = $testCount
    nativeOnly = [bool]$NativeOnly
    productionUnchanged = $true
    sourceHashes = $sourceHashes
    compiledModels = $knowledgeCompiled
    cases = $knowledgeResults
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $knowledgeEvidence $knowledgeResultName) -Encoding utf8
if ($CosmeticOnly -and !$NativeOnly) {
    $fullResultPath = Join-Path $knowledgeEvidence 'mutation_result.json'
    if (Test-Path -LiteralPath $fullResultPath -PathType Leaf) {
        $fullResult = Get-Content -LiteralPath $fullResultPath -Raw | ConvertFrom-Json
        $mergeSafe = $fullResult.passed -and $fullResult.version -eq $Version -and $fullResult.tests -eq $testCount
        foreach ($sourceProperty in $fullResult.sourceHashes.PSObject.Properties) {
            if (!$sourceHashes.Contains($sourceProperty.Name) -or $sourceHashes[$sourceProperty.Name] -cne $sourceProperty.Value) { $mergeSafe = $false }
        }
        if ($mergeSafe) {
            $fullResult.cases = @($fullResult.cases | ForEach-Object {
                if ($_.case -eq 'cosmetic_only') { $knowledgeResults[0] } else { $_ }
            })
            $fullResult | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $fullResultPath -Encoding utf8
        }
    }
}
