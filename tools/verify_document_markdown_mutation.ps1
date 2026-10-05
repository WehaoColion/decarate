# v0.0.2 - Recognize JUnit string comparison assertion failures for the named mutation test.
# v0.0.1 - Execute Rust-owned document Markdown projection and separator mutations.
[CmdletBinding()]
param([string]$Version = '')
$ErrorActionPreference = 'Stop'
$markdownRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) {
    $gradle = [IO.File]::ReadAllText((Join-Path $markdownRoot 'app/build.gradle'))
    $Version = [regex]::Match($gradle, "versionName '([^']+)'").Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android Markdown mutation version' }
$sourcePath = 'native/gridtimer_native/src/sourcegen/android_document_markdown.rs'
$templatePath = 'native/gridtimer_native/src/sourcegen/kotlin_sources.rs'
$verifierPath = 'tools/verify_document_markdown_mutation.ps1'
$source = Join-Path $markdownRoot $sourcePath
$template = Join-Path $markdownRoot $templatePath
$evidence = Join-Path $markdownRoot "release_artifacts/verification/v$Version/document_markdown_mutation"
$utf8 = [Text.UTF8Encoding]::new($false)
function Get-MarkdownTextHash([string]$Text) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text.Replace("`r`n", "`n")))).ToLowerInvariant()
}
function Get-MarkdownLiteral([string]$Name) {
    $raw = [IO.File]::ReadAllText($source)
    $matches = [regex]::Matches($raw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if ($matches.Count -ne 1) { throw "Actual Rust Markdown literal is missing or ambiguous: $Name" }
    $marker = $matches[0]
    $start = $marker.Index + $marker.Length
    $end = $raw.IndexOf('"' + $marker.Groups['hash'].Value + ';', $start)
    if ($end -lt $start) { throw "Rust Markdown literal terminator is missing: $Name" }
    $raw.Substring($start, $end - $start).Replace("`r`n", "`n")
}
function Replace-MarkdownAnchor([string]$Text, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Text, [regex]::Escape($Anchor)).Count -ne 1) { throw "Markdown mutation anchor is missing or ambiguous: $Anchor" }
    $Text.Replace($Anchor, $Replacement)
}
function Get-MarkdownDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $directory = Join-Path $cache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $directory -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed Markdown verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
function Invoke-MarkdownJava([string[]]$Arguments, [string]$Log, [int]$TimeoutSeconds) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $java
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = $utf8
    $info.StandardErrorEncoding = $utf8
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $started = $false
    try {
        if (!$process.Start()) { throw 'Cannot start the owned Markdown verification JVM' }
        $started = $true
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while (!$process.WaitForExit(15000)) {
            Write-Host ("Markdown verification JVM {0}: {1}s" -f $process.Id, [int]$clock.Elapsed.TotalSeconds)
            if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                $process.Kill($true)
                $process.WaitForExit(5000) | Out-Null
                [IO.File]::WriteAllText($Log, $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult(), $utf8)
                throw "Owned Markdown verification JVM exceeded ${TimeoutSeconds}s"
            }
        }
        [IO.File]::WriteAllText($Log, $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult(), $utf8)
        return $process.ExitCode
    } finally {
        if ($started -and !$process.HasExited) { $process.Kill($true); $process.WaitForExit(5000) | Out-Null }
        $process.Dispose()
    }
}
$sourceHash = (Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant()
$templateHash = (Get-FileHash -LiteralPath $template).Hash.ToLowerInvariant()
$verifierHash = (Get-FileHash -LiteralPath $PSCommandPath).Hash.ToLowerInvariant()
$helper = Get-MarkdownLiteral 'HELPERS'
$tests = Get-MarkdownLiteral 'TEST_CONTENTS'
$oldJoin = Get-MarkdownLiteral 'OLD_JOIN'
$newJoin = Get-MarkdownLiteral 'NEW_JOIN'
$templateText = [IO.File]::ReadAllText($template).Replace("`r`n", "`n")
$previewMarker = 'internal fun documentMarkdownPreviewText('
if ([regex]::Matches($templateText, [regex]::Escape($previewMarker)).Count -ne 1) { throw 'Actual Markdown preview function is missing or ambiguous' }
$previewStart = $templateText.IndexOf($previewMarker, [StringComparison]::Ordinal)
$previewEnd = $templateText.IndexOf("`n}`n", $previewStart, [StringComparison]::Ordinal)
if ($previewEnd -lt $previewStart) { throw 'Actual Markdown preview function boundary is missing' }
$preview = Replace-MarkdownAnchor ($templateText.Substring($previewStart, $previewEnd - $previewStart + 2)) $oldJoin $newJoin
$testNames = @([regex]::Matches($tests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$requiredTests = @('independentParagraphsHaveBlankLineBoundaries', 'splitBacktickFencePreservesCodeLinesAndBlankLines', 'actualPreviewUsesLiveDraftAndKeepsOriginalBlocks', 'actualPreviewRetainsEligibilityAndEmptyDraftRules')
if ($testNames.Count -lt 1 -or @($testNames | Sort-Object -Unique).Count -ne $testNames.Count -or @($requiredTests | Where-Object { $_ -notin $testNames }).Count) { throw 'Actual Markdown projection suite omits a required separator or preview boundary' }
$provenancePath = Join-Path $markdownRoot "release_artifacts/verification/v$Version/android_source_provenance.json"
$sourceSnapshot = $null
if (Test-Path -LiteralPath $provenancePath -PathType Leaf) {
    $provenance = Get-Content -LiteralPath $provenancePath -Raw | ConvertFrom-Json
    foreach ($entry in @(@{path=$sourcePath;hash=$sourceHash}, @{path=$templatePath;hash=$templateHash}, @{path=$verifierPath;hash=$verifierHash})) {
        $record = @($provenance.files | Where-Object path -eq $entry.path)
        if ($record.Count -ne 1 -or $record[0].sha256 -cne $entry.hash) { throw ('Markdown mutation differs from the frozen build input: ' + $entry.path) }
    }
    $sourceSnapshot = $provenance.sha256
}
$cache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
$compiler = @(
    (Get-MarkdownDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-MarkdownDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-MarkdownDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-MarkdownDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-MarkdownDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-MarkdownDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-MarkdownDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$runtime = @(
    (Get-MarkdownDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-MarkdownDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-MarkdownDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-MarkdownDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$java = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
$dependencies = @(@($compiler + $runtime + $java | Sort-Object -Unique) | ForEach-Object {
    if (!(Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Installed Markdown verification dependency is missing: $_" }
    [ordered]@{path=$_;sha256=(Get-FileHash -LiteralPath $_).Hash.ToLowerInvariant()}
})
# These value-only host types provide the same fields/copy semantics used by the
# real projection. They do not emulate Compose layout, focus or Android rendering.
$textRangeStub = "package androidx.compose.ui.text`ndata class TextRange(val start: Int, val end: Int = start)`n"
$textFieldStub = "package androidx.compose.ui.text.input`nimport androidx.compose.ui.text.TextRange`ndata class TextFieldValue(val text: String, val selection: TextRange = TextRange(0), val composition: TextRange? = null)`n"
$noteStub = @'
package com.ofairyo.gridtimer.data
enum class NoteBlockType { TEXT, IMAGE, HEADING, CHECKLIST, QUOTE, CODE, DIVIDER }
data class NoteBlock(val id: String = "", val type: NoteBlockType = NoteBlockType.TEXT, val text: String = "")
data class NoteDocument(val markdownEnabled: Boolean = false, val richTextEnabled: Boolean = false, val blocks: List<NoteBlock> = emptyList())
'@
$imports = "package com.ofairyo.gridtimer.ui`nimport androidx.compose.ui.text.input.TextFieldValue`nimport com.ofairyo.gridtimer.data.NoteBlock`nimport com.ofairyo.gridtimer.data.NoteBlockType`nimport com.ofairyo.gridtimer.data.NoteDocument`n"
$program = $imports + $preview + "`n" + $helper
$testClass = 'com.ofairyo.gridtimer.ui.DocumentMarkdownBlocksTest'
$results = @()
foreach ($caseName in @('baseline', 'cosmetic', 'removed_paragraph_separator', 'removed_fence_continuation', 'restored')) {
    $directory = Join-Path $evidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $caseHelper = $helper
    $failureTest = ''
    switch ($caseName) {
        'cosmetic' {
            $caseHelper = Replace-MarkdownAnchor $caseHelper '/** Preview-only projection. Never normalizes or writes the editor''s block text. */' '/** Read-only Markdown preview; original editor blocks remain unchanged. */'
        }
        'removed_paragraph_separator' {
            $caseHelper = Replace-MarkdownAnchor $caseHelper 'append(if (context.continuesWith(text)) "\n" else "\n\n")' 'append("\n")'
            $failureTest = 'independentParagraphsHaveBlankLineBoundaries'
        }
        'removed_fence_continuation' {
            $fenceGuards = [regex]::Matches($caseHelper, '(?m)^\s*if \(fence != null[^\r\n]*\) return true$')
            if ($fenceGuards.Count -ne 1) { throw 'The actual fence continuation guard is missing or ambiguous' }
            $caseHelper = Replace-MarkdownAnchor $caseHelper $fenceGuards[0].Value ''
            $failureTest = 'splitBacktickFencePreservesCodeLinesAndBlankLines'
        }
    }
    $caseProgram = $imports + $preview + "`n" + $caseHelper
    $files = @(
        @{name='DocumentMarkdownBlocks.kt';text=$caseProgram}, @{name='DocumentMarkdownBlocksTest.kt';text=$tests},
        @{name='TextRange.kt';text=$textRangeStub}, @{name='TextFieldValue.kt';text=$textFieldStub}, @{name='NoteDocument.kt';text=$noteStub}
    )
    $sourceFiles = @($files | ForEach-Object {
        $path = Join-Path $directory $_.name
        [IO.File]::WriteAllText($path, $_.text, $utf8)
        $path
    })
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    $compileExit = Invoke-MarkdownJava (@('-Xmx384m', '-XX:ActiveProcessorCount=2', '-cp', ($compiler -join ';'), 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect', '-jvm-target', '17', '-module-name', 'document_markdown_mutation', '-classpath', ($runtime -join ';'), '-d', $classes) + $sourceFiles) $compileLog 90
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Markdown mutation failed to compile, not a business assertion failure: $caseName" }
    $testExit = Invoke-MarkdownJava @('-Xmx256m', '-XX:ActiveProcessorCount=2', '-cp', ($classes + ';' + ($runtime -join ';')), 'org.junit.runner.JUnitCore', $testClass) $testLog 30
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = $caseName -in @('baseline', 'cosmetic', 'restored')
    $requiredFailurePattern = '(?m)^\d+\) ' + [regex]::Escape($failureTest + '(' + $testClass + ')') + '\r?\n(?:java\.lang\.AssertionError|org\.junit\.ComparisonFailure)(?::|\r?$)'
    $requiredFailureObserved = !$expectedPass -and [regex]::IsMatch($log, $requiredFailurePattern)
    $allTests = if ($expectedPass) { $log.Contains("OK ($($testNames.Count) tests)") } else { $log.Contains("Tests run: $($testNames.Count),") }
    $passed = $compileExit -eq 0 -and $allTests -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $requiredFailureObserved })
    $results += [ordered]@{
        name=$caseName;compileExit=$compileExit;testExit=$testExit;expectedPass=$expectedPass;passed=$passed
        requiredFailureTest=$failureTest;requiredFailureObserved=$requiredFailureObserved;allBusinessTestsObserved=$allTests
        helperSha256=(Get-MarkdownTextHash $caseProgram);helperLiteralSha256=(Get-MarkdownTextHash $caseHelper);testsSha256=(Get-MarkdownTextHash $tests)
        compileLogSha256=(Get-FileHash -LiteralPath $compileLog).Hash.ToLowerInvariant();testLogSha256=(Get-FileHash -LiteralPath $testLog).Hash.ToLowerInvariant()
    }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected Markdown mutation result: $caseName" }
    Write-Output "$caseName verified: $($testNames.Count) projection tests, compile $compileExit, test $testExit"
}
$sourceAfter = (Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant()
$templateAfter = (Get-FileHash -LiteralPath $template).Hash.ToLowerInvariant()
if ($sourceAfter -cne $sourceHash -or $templateAfter -cne $templateHash -or (Get-FileHash -LiteralPath $PSCommandPath).Hash.ToLowerInvariant() -cne $verifierHash) { throw 'Production projection, template or verifier changed during mutation verification' }
foreach ($dependency in $dependencies) {
    if ((Get-FileHash -LiteralPath $dependency.path).Hash.ToLowerInvariant() -cne $dependency.sha256) { throw 'Installed Markdown compiler or JUnit dependency changed' }
}
$receipt = [ordered]@{
    passed=$true;version=$Version;productionUnchanged=$true;sourceSnapshotSha256=$sourceSnapshot
    sourcePath=$sourcePath;sourceSha256Before=$sourceHash;sourceSha256After=$sourceAfter
    templatePath=$templatePath;templateSha256Before=$templateHash;templateSha256After=$templateAfter
    verifierPath=$verifierPath;verifierSha256=$verifierHash;helperLiteralSha256=(Get-MarkdownTextHash $helper)
    helperSha256=(Get-MarkdownTextHash $program);previewSha256=(Get-MarkdownTextHash $preview);testsSha256=(Get-MarkdownTextHash $tests)
    hostTypeSha256=(Get-MarkdownTextHash ($textRangeStub + $textFieldStub + $noteStub))
    tests=$testNames.Count;testNames=$testNames;cases=$results;dependencies=$dependencies;hostOnly=$true;deviceVerified=$false;networkRequests=0;noDeviceOperations=$true
    scope='Actual Rust-owned projection and preview function, executed with original JUnit tests and value-only host types. This does not verify Compose layout, Android WebView rendering or phone input.'
    checkedAt=[DateTimeOffset]::Now.ToString('o')
}
[IO.File]::WriteAllText((Join-Path $evidence 'receipt.json'), ($receipt | ConvertTo-Json -Depth 15), $utf8)
Write-Output "Android $Version document Markdown mutation acceptance completed"
