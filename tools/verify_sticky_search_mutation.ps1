# v0.0.1 - Verify sticky search state and length boundaries with isolated Kotlin/JUnit mutations.
[CmdletBinding()]
param([string]$Version = '2.23.2.22')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android mutation version' }
$sourceRelative = 'native/gridtimer_native/src/sourcegen/android_note_collection_recovery.rs'
$sourcePath = Join-Path $root $sourceRelative
$templatePath = Join-Path $root 'native/gridtimer_native/src/sourcegen/kotlin_sources.rs'
$evidence = Join-Path $root "release_artifacts/verification/v$Version/sticky_search_mutation"
$utf8 = [Text.UTF8Encoding]::new($false)
function TextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Literal([string]$Text, [string]$Name) {
    $match = [regex]::Match($Text, '(?s)(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if (!$match.Success) { throw "Rust-owned Kotlin literal is missing: $Name" }
    $match.Groups['body'].Value.Replace("\r\n", "\n")
}
function ReplaceUnique([string]$Value, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Value, [regex]::Escape($Anchor)).Count -ne 1) { throw "Mutation anchor is not unique: $Anchor" }
    $Value.Replace($Anchor, $Replacement)
}
function Dependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $cache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
    $files = @(Get-ChildItem -LiteralPath (Join-Path $cache "$Coordinate/$DependencyVersion") -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
$sourceHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$templateHash = (Get-FileHash -LiteralPath $templatePath -Algorithm SHA256).Hash.ToLowerInvariant()
$source = [IO.File]::ReadAllText($sourcePath)
$helperOriginal = Literal $source 'HELPER_CONTENTS'
$tests = Literal $source 'TEST_CONTENTS'
$uiOriginal = Literal $source 'STICKY_PRESET_ROW_WITH_SEARCH'
$testNames = @([regex]::Matches($tests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$required = @('stickySearchCannotHideAnActiveQuery', 'stickySearchRejectsOverflowWithoutChangingExistingLimit')
if (@($required | Where-Object { $_ -notin $testNames }).Count -or $testNames.Count -ne @($testNames | Sort-Object -Unique).Count) { throw 'Required distinct sticky search state tests are missing' }
foreach ($method in @('visible', 'toggleExpanded', 'normalize')) {
    if (!$uiOriginal.Contains("StickyNoteSearchPolicy.$method(")) { throw "Actual UI is not using the tested search policy: $method" }
}
# Extract the actual production model declaration, without a model or logic stub.
$modelMatch = [regex]::Match([IO.File]::ReadAllText($templatePath), '(?s)internal data class NativeNoteCollectionSection\(.*?\r?\n\)')
if (!$modelMatch.Success) { throw 'Production note section model declaration is missing' }
$model = "package com.ofairyo.gridtimer.core" + [Environment]::NewLine + [Environment]::NewLine + $modelMatch.Value
$compiler = @(
    (Dependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Dependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Dependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Dependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Dependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Dependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Dependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$runtime = @(
    (Dependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Dependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Dependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Dependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$java = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
if (!(Test-Path -LiteralPath $java -PathType Leaf)) { throw 'Configured Java 17 runtime is unavailable' }
$testClass = 'com.ofairyo.gridtimer.core.NoteCollectionRecoveryTest'
$results = @()
foreach ($case in @('baseline', 'cosmetic', 'removed_active_query_guard', 'restored')) {
    $directory = Join-Path $evidence $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $helper = $helperOriginal
    $ui = $uiOriginal
    $requiredFailure = ''
    if ($case -eq 'cosmetic') {
        $ui = ReplaceUnique $ui 'text = "搜索",' 'text = "查找便签",'
        $ui = ReplaceUnique $ui 'shape = RoundedCornerShape(18.dp)' 'shape = RoundedCornerShape(20.dp)'
    } elseif ($case -eq 'removed_active_query_guard') {
        $helper = ReplaceUnique $helper 'expanded || query.isNotEmpty()' 'expanded'
        $requiredFailure = 'stickySearchCannotHideAnActiveQuery'
    }
    $helperFile = Join-Path $directory 'NoteCollectionPartition.kt'
    $testFile = Join-Path $directory 'NoteCollectionRecoveryTest.kt'
    $modelFile = Join-Path $directory 'NativeNoteCollectionSection.kt'
    [IO.File]::WriteAllText($helperFile, $helper, $utf8)
    [IO.File]::WriteAllText($testFile, $tests, $utf8)
    [IO.File]::WriteAllText($modelFile, $model, $utf8)
    [IO.File]::WriteAllText((Join-Path $directory 'StickyNotePresetRow.kt'), $ui, $utf8)
    $classes = Join-Path $directory 'classes'
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $java -Xmx384m -cp ($compiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name sticky_search_mutation -classpath ($runtime -join ';') -d $classes $helperFile $testFile $modelFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Mutation failed compilation: $case" }
    & $java -Xmx256m -cp ($classes + ';' + ($runtime -join ';')) org.junit.runner.JUnitCore $testClass *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = !$requiredFailure
    $observedFailure = !$expectedPass -and $log.Contains($requiredFailure + '(' + $testClass + ')') -and $log.Contains('java.lang.AssertionError')
    $allTestsObserved = if ($expectedPass) { $log.Contains("OK ($($testNames.Count) tests)") } else { $log.Contains("Tests run: $($testNames.Count),") }
    $passed = $allTestsObserved -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $observedFailure })
    $results += [ordered]@{ name=$case; passed=$passed; compileExit=$compileExit; testExit=$testExit; expectedPass=[bool]$expectedPass; requiredFailureTest=$requiredFailure; requiredFailureObserved=$observedFailure; allTestsObserved=$allTestsObserved; helperSha256=(TextHash $helper); testsSha256=(TextHash $tests); uiSha256=(TextHash $ui) }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 30; throw "Unexpected business mutation result: $case" }
    Write-Output "$case verified: $($testNames.Count) tests, compile $compileExit, test $testExit"
}
$sourceAfter = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
$templateAfter = (Get-FileHash -LiteralPath $templatePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($sourceHash -cne $sourceAfter -or $templateHash -cne $templateAfter) { throw 'Production Rust source changed during isolated mutation verification' }
$receipt = [ordered]@{
    passed=$true; version=$Version; sourcePath=$sourceRelative; sourceSha256Before=$sourceHash; sourceSha256After=$sourceAfter; templateSha256=$templateHash
    verifierSha256=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    tests=$testNames.Count; testNames=$testNames; cases=$results; productionUnchanged=$true; hostOnly=$true; noNetworkRequests=$true
    scope='Production sticky-search state/length policy and full note recovery domain suite. Cosmetic source variants are evidence only; no Compose, device, or provider execution is asserted.'
    checkedAt=[DateTimeOffset]::Now.ToString('o')
}
[IO.File]::WriteAllText((Join-Path $evidence 'receipt.json'), ($receipt | ConvertTo-Json -Depth 10), $utf8)
Write-Output "Android $Version sticky search mutation acceptance completed"
