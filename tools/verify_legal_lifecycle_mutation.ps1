# v0.0.1 - Verify legal scan settlement using Rust-owned helpers and real coroutine tests.
[CmdletBinding()]
param([string]$Version = '')
$ErrorActionPreference = 'Stop'
$legalRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) {
    $gradle = [IO.File]::ReadAllText((Join-Path $legalRoot 'app/build.gradle'))
    $Version = [regex]::Match($gradle, "versionName '([^']+)'").Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android legal mutation version' }
$legalSourcePath = 'native/gridtimer_native/src/sourcegen/legal_risk_ui_source.rs'
$legalSource = Join-Path $legalRoot $legalSourcePath
$legalEvidence = Join-Path $legalRoot "release_artifacts/verification/v$Version/legal_lifecycle_mutation"
$legalUtf8 = [Text.UTF8Encoding]::new($false)
function Get-LegalLiteral([string]$Name) {
    $raw = [IO.File]::ReadAllText($legalSource)
    $literal = [regex]::Match($raw, '(?s)pub const ' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if (!$literal.Success) { throw "Actual Rust literal is missing: $Name" }
    $literal.Groups['body'].Value.Replace("`r`n", "`n")
}
function Get-LegalTextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Get-LegalDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $path = Join-Path $legalCache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $path -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
$legalBefore = (Get-FileHash -LiteralPath $legalSource -Algorithm SHA256).Hash.ToLowerInvariant()
$legalVerifierHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$legalUi = Get-LegalLiteral 'CONTENTS'
$legalTests = Get-LegalLiteral 'TEST_CONTENTS'
# Only the pure send predicate and request boundary are compiled. The source
# remains Rust-owned; no generated Android file or production file is edited.
$legalStart = $legalUi.IndexOf('internal fun legalSendReady(')
$legalEnd = $legalUi.IndexOf('/** Separate from the finance overview pager.')
if ($legalStart -lt 0 -or $legalEnd -le $legalStart) { throw 'Actual legal request helper boundaries are missing' }
$legalHelper = "package com.ofairyo.gridtimer.ui`n`n" + $legalUi.Substring($legalStart, $legalEnd - $legalStart)
$legalTestNames = @([regex]::Matches($legalTests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$legalRequiredTests = @('invalidatedAnalysisSettlesWithoutPublishingAndAllowsRetry','invalidatedPreparationCannotOverlapItsReplacement','invalidationDuringSuspendedIoStillClearsBusyInFinally','oldWorkspaceCannotReleaseNewWorkspaceRequest')
if ($legalTestNames.Count -ne 13 -or @($legalRequiredTests | Where-Object { $_ -notin $legalTestNames }).Count) { throw 'Actual legal lifecycle business tests are incomplete' }
$legalCache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
$legalCompiler = @(
    (Get-LegalDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-LegalDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-LegalDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-LegalDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-LegalDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-LegalDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-LegalDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$legalRuntime = @(
    (Get-LegalDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-LegalDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-LegalDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-LegalDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar'),
    (Get-LegalDependency 'org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm' '1.8.1' 'kotlinx-coroutines-core-jvm-1.8.1.jar')
)
$legalJava = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
$legalDependencies = @(@($legalCompiler + $legalRuntime + $legalJava | Sort-Object -Unique) | ForEach-Object {
    if (!(Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Installed verification dependency is missing: $_" }
    [ordered]@{ path=$_; sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$legalTestClass = 'com.ofairyo.gridtimer.ui.LegalSendReadyTest'
$legalResults = @()
foreach ($caseName in @('baseline','cosmetic','finish_generation_guard','restored')) {
    $directory = Join-Path $legalEvidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $helper = $legalHelper
    $ui = $legalUi
    $requiredFailure = ''
    if ($caseName -eq 'cosmetic') {
        $anchor = 'Text("法律风险线索", style ='
        if (!$ui.Contains($anchor)) { throw 'Actual legal UI cosmetic anchor is missing' }
        $ui = $ui.Replace($anchor, 'Text("查看法律风险线索", style =')
    } elseif ($caseName -eq 'finish_generation_guard') {
        $anchor = 'if (pending != ticket) return false'
        if ([regex]::Matches($helper, [regex]::Escape($anchor)).Count -ne 1) { throw 'Actual legal request settlement guard is not unique' }
        $helper = $helper.Replace($anchor, 'if (generation != ticket) return false')
        $requiredFailure = 'invalidationDuringSuspendedIoStillClearsBusyInFinally'
    }
    $helperFile = Join-Path $directory 'LegalRequestBoundary.kt'
    $testFile = Join-Path $directory 'LegalSendReadyTest.kt'
    [IO.File]::WriteAllText($helperFile, $helper, $legalUtf8)
    [IO.File]::WriteAllText($testFile, $legalTests, $legalUtf8)
    # The cosmetic UI copy is evidence only, not a claim of device UI execution.
    [IO.File]::WriteAllText((Join-Path $directory 'LegalRiskScreen.kt'), $ui, $legalUtf8)
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $legalJava -Xmx384m -cp ($legalCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name legal_lifecycle_mutation -classpath ($legalRuntime -join ';') -d $classes $helperFile $testFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Legal mutation failed to compile, not an accepted business failure: $caseName" }
    & $legalJava -Xmx256m -cp ($classes + ';' + ($legalRuntime -join ';')) org.junit.runner.JUnitCore $legalTestClass *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = $caseName -ne 'finish_generation_guard'
    $observed = !$expectedPass -and $log.Contains($requiredFailure + '(' + $legalTestClass + ')') -and $log.Contains('java.lang.AssertionError')
    $allTests = if ($expectedPass) { $log.Contains("OK ($($legalTestNames.Count) tests)") } else { $log.Contains("Tests run: $($legalTestNames.Count),") }
    $passed = $compileExit -eq 0 -and $allTests -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $observed })
    $legalResults += [ordered]@{ name=$caseName; compileExit=$compileExit; testExit=$testExit; expectedPass=$expectedPass; requiredFailureTest=$requiredFailure; requiredFailureObserved=$observed; allBusinessTestsObserved=$allTests; passed=$passed; helperSha256=(Get-LegalTextHash $helper); uiSha256=(Get-LegalTextHash $ui); testsSha256=(Get-LegalTextHash $legalTests); compileLogSha256=(Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant(); testLogSha256=(Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant() }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected legal lifecycle mutation result: $caseName" }
    Write-Output "$caseName verified: $($legalTestNames.Count) business tests, compile $compileExit, test $testExit"
}
$legalAfter = (Get-FileHash -LiteralPath $legalSource -Algorithm SHA256).Hash.ToLowerInvariant()
if ($legalBefore -cne $legalAfter -or (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $legalVerifierHash) { throw 'Production source or verifier changed during legal mutation verification' }
foreach ($dependency in $legalDependencies) {
    if ((Get-FileHash -LiteralPath $dependency.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $dependency.sha256) { throw 'Installed compiler/runtime dependency changed during verification' }
}
[IO.File]::WriteAllText((Join-Path $legalEvidence 'receipt.json'), ([ordered]@{ passed=$true; version=$Version; productionUnchanged=$true; sourcePath=$legalSourcePath; sourceSha256Before=$legalBefore; sourceSha256After=$legalAfter; verifierPath='tools/verify_legal_lifecycle_mutation.ps1'; verifierSha256=$legalVerifierHash; helperSha256=(Get-LegalTextHash $legalHelper); testsSha256=(Get-LegalTextHash $legalTests); uiSha256=(Get-LegalTextHash $legalUi); tests=$legalTestNames.Count; testNames=$legalTestNames; cases=$legalResults; dependencies=$legalDependencies; hostOnly=$true; networkRequests=0; noDeviceOperations=$true; scope='Actual Rust-owned legal request helper and tests, including suspended coroutine IO. Cosmetic UI copy is exported; this receipt does not assert device UI execution.'; checkedAt=[DateTimeOffset]::Now.ToString('o') } | ConvertTo-Json -Depth 15), $legalUtf8)
Write-Output "Android $Version legal lifecycle mutation acceptance completed"
