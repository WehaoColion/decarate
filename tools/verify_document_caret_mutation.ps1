# v0.0.1 - Verify Rust-owned document caret policy with real Kotlin/JUnit mutations.
[CmdletBinding()]
param([string]$Version = '')
$ErrorActionPreference = 'Stop'
$caretRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) {
    $caretGradle = [IO.File]::ReadAllText((Join-Path $caretRoot 'app/build.gradle'))
    $Version = [regex]::Match($caretGradle, "versionName '([^']+)'").Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android caret mutation version' }
$caretSourcePath = 'native/gridtimer_native/src/sourcegen/android_document_caret.rs'
$caretSource = Join-Path $caretRoot $caretSourcePath
$caretTemplatePath = 'native/gridtimer_native/src/sourcegen/kotlin_sources.rs'
$caretTemplateSource = Join-Path $caretRoot $caretTemplatePath
$caretVerifierPath = 'tools/verify_document_caret_mutation.ps1'
$caretEvidence = Join-Path $caretRoot "release_artifacts/verification/v$Version/document_caret_mutation"
$caretUtf8 = [Text.UTF8Encoding]::new($false)
function Get-CaretTextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Get-CaretLiteral([string]$Name) {
    $raw = [IO.File]::ReadAllText($caretSource)
    $marker = [regex]::Match($raw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if (!$marker.Success) { throw "Actual Rust literal is missing: $Name" }
    $start = $marker.Index + $marker.Length
    $end = $raw.IndexOf('"' + $marker.Groups['hash'].Value + ';', $start)
    if ($end -lt $start) { throw "Actual Rust literal terminator is missing: $Name" }
    $raw.Substring($start, $end - $start).Replace("`r`n", "`n")
}
function Replace-CaretAnchor([string]$Value, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Value, [regex]::Escape($Anchor)).Count -ne 1) { throw "Caret mutation anchor is not unique: $Anchor" }
    $Value.Replace($Anchor, $Replacement)
}
function Get-CaretDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $path = Join-Path $caretCache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $path -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed caret verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
$caretSourceHash = (Get-FileHash -LiteralPath $caretSource -Algorithm SHA256).Hash.ToLowerInvariant()
$caretTemplateHash = (Get-FileHash -LiteralPath $caretTemplateSource -Algorithm SHA256).Hash.ToLowerInvariant()
$caretVerifierHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$caretHelper = Get-CaretLiteral 'POLICY_CONTENTS'
$caretUi = Get-CaretLiteral 'UI_CONTENTS'
$caretTests = Get-CaretLiteral 'TEST_CONTENTS'
$caretTestNames = @([regex]::Matches($caretTests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$caretRequiredTests = @('newlineWaitsForTheMatchingTextLayout','equalLengthReplacementDoesNotUseAnOldLayout','visibleBlockNeverJumpsToItsTop','consecutiveLinesStayWithinTheAvailableHeight')
if ($caretTestNames.Count -ne 15 -or @($caretTestNames | Sort-Object -Unique).Count -ne 15 -or @($caretRequiredTests | Where-Object { $_ -notin $caretTestNames }).Count) { throw 'Actual caret domain suite must contain all 15 distinct tests' }
# The cosmetic case changes the actual editor template's toolbar label and color.
# Its exported UI copy is evidence only; host JUnit executes the pure policy and tests.
$caretTemplateRaw = [IO.File]::ReadAllText($caretTemplateSource)
$caretTemplateMarker = [regex]::Match($caretTemplateRaw, 'path:\s*"com/ofairyo/gridtimer/ui/NoteDocumentEditor\.kt",\s*contents:\s*r(?<hash>#+)"')
if (!$caretTemplateMarker.Success) { throw 'Actual document editor template is missing' }
$caretTemplateStart = $caretTemplateMarker.Index + $caretTemplateMarker.Length
$caretTemplateEnd = $caretTemplateRaw.IndexOf('"' + $caretTemplateMarker.Groups['hash'].Value + ',', $caretTemplateStart)
if ($caretTemplateEnd -lt $caretTemplateStart) { throw 'Actual document editor template terminator is missing' }
$caretEditorTemplate = $caretTemplateRaw.Substring($caretTemplateStart, $caretTemplateEnd - $caretTemplateStart).Replace("`r`n", "`n")
$caretCache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
$caretCompiler = @(
    (Get-CaretDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-CaretDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-CaretDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-CaretDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-CaretDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-CaretDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-CaretDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$caretRuntime = @(
    (Get-CaretDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-CaretDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-CaretDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-CaretDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$caretJava = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
$caretDependencies = @(@($caretCompiler + $caretRuntime + $caretJava | Sort-Object -Unique) | ForEach-Object {
    if (!(Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Installed caret verification dependency is missing: $_" }
    [ordered]@{ path=$_; sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$caretTestClass = 'com.ofairyo.gridtimer.ui.DocumentCaretPolicyTest'
$caretResults = @()
foreach ($caseName in @('baseline','cosmetic','removed_layout_identity','removed_visible_block_guard','restored')) {
    $directory = Join-Path $caretEvidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $helper = $caretHelper
    $editor = $caretEditorTemplate
    $requiredFailure = ''
    switch ($caseName) {
        'cosmetic' {
            $editor = Replace-CaretAnchor $editor 'DocumentToolbarAction("撤销", "↶", accent, enabled = canUndo, onClick = onUndo)' 'DocumentToolbarAction("撤回编辑", "↶", accent, enabled = canUndo, onClick = onUndo)'
            $editor = Replace-CaretAnchor $editor 'color = if (enabled) accent else accent.copy(alpha = 0.26f),' 'color = if (enabled) accent else accent.copy(alpha = 0.32f),'
        }
        'removed_layout_identity' {
            $helper = Replace-CaretAnchor $helper ' || text != layoutText' ''
            $requiredFailure = 'equalLengthReplacementDoesNotUseAnOldLayout'
        }
        'removed_visible_block_guard' {
            $helper = Replace-CaretAnchor $helper ' || alreadyVisible' ''
            $requiredFailure = 'visibleBlockNeverJumpsToItsTop'
        }
    }
    $helperFile = Join-Path $directory 'DocumentCaretPolicy.kt'
    $testFile = Join-Path $directory 'DocumentCaretPolicyTest.kt'
    [IO.File]::WriteAllText($helperFile, $helper, $caretUtf8)
    [IO.File]::WriteAllText($testFile, $caretTests, $caretUtf8)
    [IO.File]::WriteAllText((Join-Path $directory 'DocumentCaretVisibility.kt'), $caretUi, $caretUtf8)
    [IO.File]::WriteAllText((Join-Path $directory 'NoteDocumentEditor_template.kt'), $editor, $caretUtf8)
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $caretJava -Xmx384m -cp ($caretCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name document_caret_mutation -classpath ($caretRuntime -join ';') -d $classes $helperFile $testFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Caret mutation failed to compile, not an accepted business failure: $caseName" }
    & $caretJava -Xmx256m -cp ($classes + ';' + ($caretRuntime -join ';')) org.junit.runner.JUnitCore $caretTestClass *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = $caseName -in @('baseline','cosmetic','restored')
    $requiredFailureObserved = !$expectedPass -and $log.Contains($requiredFailure + '(' + $caretTestClass + ')') -and $log.Contains('java.lang.AssertionError')
    $allTests = if ($expectedPass) { $log.Contains("OK ($($caretTestNames.Count) tests)") } else { $log.Contains("Tests run: $($caretTestNames.Count),") }
    $passed = $compileExit -eq 0 -and $allTests -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $requiredFailureObserved })
    $caretResults += [ordered]@{
        name=$caseName; compileExit=$compileExit; testExit=$testExit; expectedPass=$expectedPass
        requiredFailureTest=$requiredFailure; requiredFailureObserved=$requiredFailureObserved; allBusinessTestsObserved=$allTests; passed=$passed
        helperSha256=(Get-CaretTextHash $helper); testsSha256=(Get-CaretTextHash $caretTests)
        uiSha256=(Get-CaretTextHash $caretUi); editorTemplateSha256=(Get-CaretTextHash $editor)
        compileLogSha256=(Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant()
        testLogSha256=(Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected caret policy mutation result: $caseName" }
    Write-Output "$caseName verified: $($caretTestNames.Count) domain tests, compile $compileExit, test $testExit"
}
$caretAfterHash = (Get-FileHash -LiteralPath $caretSource -Algorithm SHA256).Hash.ToLowerInvariant()
if ($caretAfterHash -cne $caretSourceHash -or (Get-FileHash -LiteralPath $caretTemplateSource -Algorithm SHA256).Hash.ToLowerInvariant() -cne $caretTemplateHash -or (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $caretVerifierHash) { throw 'Production source, editor template or caret verifier changed during verification' }
foreach ($dependency in $caretDependencies) {
    if ((Get-FileHash -LiteralPath $dependency.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $dependency.sha256) { throw 'Installed Kotlin/JUnit dependency changed during verification' }
}
[IO.File]::WriteAllText((Join-Path $caretEvidence 'receipt.json'), ([ordered]@{
    passed=$true; version=$Version; productionUnchanged=$true; sourcePath=$caretSourcePath
    sourceSha256Before=$caretSourceHash; sourceSha256After=$caretAfterHash
    templateSourcePath=$caretTemplatePath; templateSourceSha256=$caretTemplateHash
    verifierPath=$caretVerifierPath; verifierSha256=$caretVerifierHash
    helperLiteral='POLICY_CONTENTS'; testsLiteral='TEST_CONTENTS'; uiLiteral='UI_CONTENTS'
    helperSha256=(Get-CaretTextHash $caretHelper); testsSha256=(Get-CaretTextHash $caretTests)
    uiSha256=(Get-CaretTextHash $caretUi); editorTemplateSha256=(Get-CaretTextHash $caretEditorTemplate)
    tests=$caretTestNames.Count; testNames=$caretTestNames; cases=$caretResults; dependencies=$caretDependencies
    hostOnly=$true; networkRequests=0; noDeviceOperations=$true
    scope='Actual Rust-owned document caret policy and 15 domain tests compiled with installed Kotlin and real JUnit. Cosmetic editor label/color copies are exported; this receipt does not assert Compose UI, IME or phone execution.'
    checkedAt=[DateTimeOffset]::Now.ToString('o')
} | ConvertTo-Json -Depth 15), $caretUtf8)
Write-Output "Android $Version document caret mutation acceptance completed"
