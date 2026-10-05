# v0.0.1 - Verify finance scope isolation and bounded previews using real Kotlin/JUnit mutations.
[CmdletBinding()]
param([string]$Version = '')
$ErrorActionPreference = 'Stop'
$financeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) {
    $financeGradle = [IO.File]::ReadAllText((Join-Path $financeRoot 'app/build.gradle'))
    $Version = [regex]::Match($financeGradle, "versionName '([^']+)'").Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android finance mutation version' }
$financeSourcePath = 'native/gridtimer_native/src/sourcegen/android_finance_workspace.rs'
$financeSource = Join-Path $financeRoot $financeSourcePath
$financeVerifierPath = 'tools/verify_finance_workspace_mutation.ps1'
$financeEvidence = Join-Path $financeRoot "release_artifacts/verification/v$Version/finance_workspace_mutation"
$financeUtf8 = [Text.UTF8Encoding]::new($false)
function Get-FinanceTextHash([string]$Value) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value))).ToLowerInvariant()
}
function Get-FinanceLiteral([string]$Name) {
    $raw = [IO.File]::ReadAllText($financeSource)
    $markers = [regex]::Matches($raw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if ($markers.Count -ne 1) { throw "Actual Rust finance literal is missing or ambiguous: $Name" }
    $marker = $markers[0]
    $start = $marker.Index + $marker.Length
    $end = $raw.IndexOf('"' + $marker.Groups['hash'].Value + ';', $start)
    if ($end -lt $start) { throw "Actual Rust finance literal terminator is missing: $Name" }
    $raw.Substring($start, $end - $start).Replace("`r`n", "`n")
}
function Replace-FinanceAnchor([string]$Value, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Value, [regex]::Escape($Anchor)).Count -ne 1) { throw "Finance mutation anchor is not unique: $Anchor" }
    $Value.Replace($Anchor, $Replacement)
}
function Get-FinanceDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $path = Join-Path $financeCache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $path -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed finance verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
$financeSourceHash = (Get-FileHash -LiteralPath $financeSource -Algorithm SHA256).Hash.ToLowerInvariant()
$financeVerifierHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$financePolicy = Get-FinanceLiteral 'POLICY'
$financeComponents = Get-FinanceLiteral 'COMPONENTS'
$financeWorkspace = Get-FinanceLiteral 'WORKSPACE'
$financeTests = Get-FinanceLiteral 'TEST_CONTENTS'
$financeTestNames = @([regex]::Matches($financeTests, '@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object { $_.Groups[1].Value })
$financeRequiredTests = @('newMonthClosesOldDetailAndReturnsToOverview','newWorkspaceDoesNotInheritFinancialDetail','switchingTaskClearsThePreviousDialog','reviewCountUsesAllFourExplicitFlags','alertPreviewIsBoundedWithoutChangingTheTotal')
if ($financeTestNames.Count -ne 5 -or @($financeTestNames | Sort-Object -Unique).Count -ne 5 -or @($financeRequiredTests | Where-Object { $_ -notin $financeTestNames }).Count) { throw 'Actual finance suite must contain the five distinct state and boundary tests' }
$financeCache = if ($env:GRADLE_USER_HOME) { Join-Path $env:GRADLE_USER_HOME 'caches/modules-2/files-2.1' } else { Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1' }
$financeCompiler = @(
    (Get-FinanceDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-FinanceDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-FinanceDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-FinanceDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-FinanceDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-FinanceDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-FinanceDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$financeRuntime = @(
    (Get-FinanceDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-FinanceDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-FinanceDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-FinanceDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$financeJava = 'C:/tools/java/jdk-17.0.18+8/bin/java.exe'
$financeDependencies = @(@($financeCompiler + $financeRuntime + $financeJava | Sort-Object -Unique) | ForEach-Object {
    if (!(Test-Path -LiteralPath $_ -PathType Leaf)) { throw "Installed finance verification dependency is missing: $_" }
    [ordered]@{ path=$_; sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$financeTestClass = 'com.ofairyo.gridtimer.ui.FinanceWorkspacePolicyTest'
$financeResults = @()
foreach ($caseName in @('baseline','cosmetic','removed_workspace_guard','removed_month_guard','removed_preview_limit','restored')) {
    $directory = Join-Path $financeEvidence $caseName
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $policy = $financePolicy
    $components = $financeComponents
    $requiredFailure = ''
    switch ($caseName) {
        'cosmetic' {
            $policy = Replace-FinanceAnchor $policy 'OVERVIEW("总览")' 'OVERVIEW("概览")'
            $components = Replace-FinanceAnchor $components 'Column(modifier = modifier.padding(vertical = 4.dp),' 'Column(modifier = modifier.padding(vertical = 6.dp),'
        }
        'removed_workspace_guard' {
            $policy = Replace-FinanceAnchor $policy 'workspaceKey == workspace && monthKey == month' 'monthKey == month'
            $requiredFailure = 'newWorkspaceDoesNotInheritFinancialDetail'
        }
        'removed_month_guard' {
            $policy = Replace-FinanceAnchor $policy 'workspaceKey == workspace && monthKey == month' 'workspaceKey == workspace'
            $requiredFailure = 'newMonthClosesOldDetailAndReturnsToOverview'
        }
        'removed_preview_limit' {
            $policy = Replace-FinanceAnchor $policy 'total.coerceIn(0, 2)' 'total.coerceAtLeast(0)'
            $requiredFailure = 'alertPreviewIsBoundedWithoutChangingTheTotal'
        }
    }
    # Pure policy is compiled as owned by Rust. Exported component copies prove
    # the cosmetic change; this runner does not pretend to execute Compose UI.
    $helper = "package com.ofairyo.gridtimer.ui`n" + $policy
    $helperFile = Join-Path $directory 'FinanceWorkspacePolicy.kt'
    $testFile = Join-Path $directory 'FinanceWorkspacePolicyTest.kt'
    [IO.File]::WriteAllText($helperFile, $helper, $financeUtf8)
    [IO.File]::WriteAllText($testFile, $financeTests, $financeUtf8)
    [IO.File]::WriteAllText((Join-Path $directory 'FinanceWorkspaceComponents.kt'), $components, $financeUtf8)
    [IO.File]::WriteAllText((Join-Path $directory 'FinanceWorkspace.kt'), $financeWorkspace, $financeUtf8)
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $financeJava -Xmx384m -cp ($financeCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name finance_workspace_mutation -classpath ($financeRuntime -join ';') -d $classes $helperFile $testFile *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Finance mutation failed to compile, not an accepted state failure: $caseName" }
    & $financeJava -Xmx256m -cp ($classes + ';' + ($financeRuntime -join ';')) org.junit.runner.JUnitCore $financeTestClass *> $testLog
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText($testLog)
    $expectedPass = $caseName -in @('baseline','cosmetic','restored')
    $requiredFailureObserved = !$expectedPass -and $log.Contains($requiredFailure + '(' + $financeTestClass + ')') -and $log.Contains('java.lang.AssertionError')
    $allTests = if ($expectedPass) { $log.Contains('OK (5 tests)') } else { $log.Contains('Tests run: 5,') }
    $passed = $compileExit -eq 0 -and $allTests -and $(if ($expectedPass) { $testExit -eq 0 } else { $testExit -ne 0 -and $requiredFailureObserved })
    $financeResults += [ordered]@{
        name=$caseName; compileExit=$compileExit; testExit=$testExit; expectedPass=$expectedPass
        requiredFailureTest=$requiredFailure; requiredFailureObserved=$requiredFailureObserved; allBusinessTestsObserved=$allTests; passed=$passed
        helperSha256=(Get-FinanceTextHash $helper); policySha256=(Get-FinanceTextHash $policy); testsSha256=(Get-FinanceTextHash $financeTests)
        componentsSha256=(Get-FinanceTextHash $components); workspaceSha256=(Get-FinanceTextHash $financeWorkspace)
        compileLogSha256=(Get-FileHash -LiteralPath $compileLog -Algorithm SHA256).Hash.ToLowerInvariant()
        testLogSha256=(Get-FileHash -LiteralPath $testLog -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected finance state mutation result: $caseName" }
    Write-Output "$caseName verified: 5 state tests, compile $compileExit, test $testExit"
}
$financeAfterHash = (Get-FileHash -LiteralPath $financeSource -Algorithm SHA256).Hash.ToLowerInvariant()
if ($financeAfterHash -cne $financeSourceHash -or (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $financeVerifierHash) { throw 'Production finance source or verifier changed during verification' }
foreach ($dependency in $financeDependencies) {
    if ((Get-FileHash -LiteralPath $dependency.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $dependency.sha256) { throw 'Installed Kotlin/JUnit dependency changed during verification' }
}
[IO.File]::WriteAllText((Join-Path $financeEvidence 'receipt.json'), ([ordered]@{
    passed=$true; version=$Version; productionUnchanged=$true; sourcePath=$financeSourcePath
    sourceSha256Before=$financeSourceHash; sourceSha256After=$financeAfterHash
    verifierPath=$financeVerifierPath; verifierSha256=$financeVerifierHash
    policySha256=(Get-FinanceTextHash $financePolicy); testsSha256=(Get-FinanceTextHash $financeTests)
    helperSha256=(Get-FinanceTextHash ("package com.ofairyo.gridtimer.ui`n"+$financePolicy))
    componentsSha256=(Get-FinanceTextHash $financeComponents); workspaceSha256=(Get-FinanceTextHash $financeWorkspace)
    tests=5; testNames=$financeTestNames; cases=$financeResults; dependencies=$financeDependencies
    hostOnly=$true; networkRequests=0; noDeviceOperations=$true
    scope='Actual Rust-owned finance navigation policy and five state/boundary tests compiled with installed Kotlin and real JUnit. Cosmetic components are exported only; Compose layout, phone input and financial writes are not executed here.'
    checkedAt=[DateTimeOffset]::Now.ToString('o')
} | ConvertTo-Json -Depth 15), $financeUtf8)
Write-Output "Android $Version finance workspace mutation acceptance completed"
