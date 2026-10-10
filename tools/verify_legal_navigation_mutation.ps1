# v0.0.1 - Verify stale note-selection rejection using generated production logic.
param([string]$Version)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) { $Version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'app/build.gradle')), "versionName '([^']+)'").Groups[1].Value }
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android version' }
$evidence = Join-Path $root "release_artifacts/verification/v$Version/legal_navigation_mutation"
$emitter = Join-Path $root 'native/gridtimer_native/src/sourcegen/android_legal_integration.rs'
$uiPath = Join-Path $root 'native/gridtimer_native/src/sourcegen/legal_risk_ui_source.rs'
$emitterHash = (Get-FileHash -LiteralPath $emitter).Hash
$uiHash = (Get-FileHash -LiteralPath $uiPath).Hash
$raw = [IO.File]::ReadAllText($emitter)
function Get-Literal([string]$Name) {
    $match = [regex]::Match($raw, '(?s)pub const '+$Name+':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if (!$match.Success) { throw "Missing production literal: $Name" }
    return $match.Groups['body'].Value
}
$helper = Get-Literal 'HELPER_CONTENTS'
$tests = Get-Literal 'TEST_CONTENTS'
$testCount = [regex]::Matches($tests, '@Test\s+fun ').Count
if ($testCount -lt 5) { throw 'Selection boundary tests missing' }
$cache = Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1'
function Get-NavigationDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $directory = Join-Path $cache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $directory -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed navigation verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
$compiler = @(
    (Get-NavigationDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-NavigationDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-NavigationDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-NavigationDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-NavigationDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-NavigationDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-NavigationDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$runtime = @(
    (Get-NavigationDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-NavigationDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-NavigationDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-NavigationDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$java = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$utf8 = [Text.UTF8Encoding]::new($false)
$results = @()
foreach ($case in @('baseline','cosmetic_only','removed_selection_freshness','legacy_live_selection')) {
    $directory = Join-Path $evidence $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $production = $helper
    $ui = [IO.File]::ReadAllText($uiPath)
    if ($case -eq 'cosmetic_only') {
        $label = 'Text("'+(-join @([char]0x6253,[char]0x5f00,[char]0x6240,[char]0x5c5e,[char]0x8bb0,[char]0x5f55))+'")'
        if (!$ui.Contains($label)) { throw 'Actual report button label missing' }
        $ui = $ui.Replace($label, 'Text("Open original record")')
        # Keep actual UI wording change as evidence. Domain tests intentionally
        # ignore labels; formal release compilation separately verifies the UI.
        [IO.File]::WriteAllText((Join-Path $directory 'report_ui_wording.rs'), $ui, $utf8)
    }
    if ($case -eq 'removed_selection_freshness') {
        $guard = 'selectedId == currentId &&'
        if (!$production.Contains($guard)) { throw 'Production freshness guard missing' }
        $production = $production.Replace($guard, '')
    }
    if ($case -eq 'legacy_live_selection') {
        $guard = 'selectedId != null && selectedId == currentId && resolvedId != selectedId'
        if (!$production.Contains($guard)) { throw 'Selection snapshot policy missing' }
        $production = $production.Replace($guard, 'currentId != null && resolvedId == null')
    }
    $helperFile = Join-Path $directory 'NoteSelectionCheck.kt'
    $testFile = Join-Path $directory 'NoteSelectionCheckTest.kt'
    [IO.File]::WriteAllText($helperFile, $production, $utf8)
    [IO.File]::WriteAllText($testFile, $tests, $utf8)
    $classes = Join-Path $directory ('classes_'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $java -Xmx512m -cp ($compiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -classpath ($runtime -join ';') -d $classes $helperFile $testFile *> (Join-Path $directory 'compile.log')
    if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath (Join-Path $directory 'compile.log') -Tail 20; throw "Mutation compilation failed: $case" }
    & $java -Xmx256m -cp ($classes+';'+($runtime -join ';')) org.junit.runner.JUnitCore com.ofairyo.gridtimer.ui.NoteSelectionCheckTest *> (Join-Path $directory 'tests.log')
    $exitCode = $LASTEXITCODE
    $log = [IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $expectedPass = $case -in @('baseline','cosmetic_only')
    $requiredFailure = if ($case -eq 'legacy_live_selection') { 'staleEmptyCheckDoesNotClearOpenedRecord' } else { 'staleMissingCheckDoesNotClearDifferentRecord' }
    $passed = if ($expectedPass) { $exitCode -eq 0 -and $log.Contains("OK ($testCount tests)") } else { $exitCode -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains($requiredFailure) }
    $results += [ordered]@{case=$case;passed=$passed;expectedPass=$expectedPass;exitCode=$exitCode;tests=$testCount;helperSha256=(Get-FileHash -LiteralPath $helperFile).Hash.ToLowerInvariant();uiWordingChanged=($case -eq 'cosmetic_only')}
    if (!$passed) { Get-Content -LiteralPath (Join-Path $directory 'tests.log') -Tail 25; throw "Unexpected mutation result: $case" }
    Write-Output "$case verified"
}
if ((Get-FileHash -LiteralPath $emitter).Hash -cne $emitterHash -or (Get-FileHash -LiteralPath $uiPath).Hash -cne $uiHash) { throw 'Production changed during mutation verification' }
[ordered]@{passed=$true;version=$Version;tests=$testCount;productionUnchanged=$true;sourceSha256=$emitterHash.ToLowerInvariant();cases=$results;hostOnly=$true;deviceVerified=$false} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'receipt.json') -Encoding utf8
