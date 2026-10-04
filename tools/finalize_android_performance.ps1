# v0.0.1 - Bind performance evidence, formal test results and published package hashes.
$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match([IO.File]::ReadAllText((Join-Path $taskRoot 'app/build.gradle')),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $taskRoot "release_artifacts/verification/v$version"
function Read-Evidence([string]$Name){Get-Content -LiteralPath (Join-Path $evidence $Name) -Raw | ConvertFrom-Json}
$build=Read-Evidence 'offline_build_acceptance.json'
$delivery=Read-Evidence 'delivery_receipt.json'
$package=Read-Evidence 'post_publish_package_check.json'
foreach($record in @($build,$delivery,$package)) {
    if(!$record.passed -or $record.version -ne $version){throw 'Incomplete formal release evidence'}
}
if($build.apkSha256 -ne $package.sha256 -or $delivery.sha256 -ne $package.sha256){throw 'Package changed after testing'}
$canvas=Read-Evidence 'canvas_performance/comparison.json'
$notes=Read-Evidence 'note_performance/acceptance.json'
$scope=Read-Evidence 'note_performance/final_source_scope.json'
if(!$canvas.passed -or !$notes.passed -or !$scope.passed -or !$scope.measuredFunctionsUnchanged){throw 'Performance evidence incomplete'}
if(($canvas.results | Where-Object case -eq 'after').sourceSha256 -ne (Get-FileHash -LiteralPath (Join-Path $taskRoot 'native/gridtimer_native/src/android_canvas.rs')).Hash){throw 'Canvas benchmark differs from final core'}
if($scope.currentEmitterSha256 -ne (Get-FileHash -LiteralPath (Join-Path $taskRoot 'native/gridtimer_native/src/sourcegen/android_note_list_performance.rs')).Hash){throw 'Note query scope differs from final source'}
$suites=@()
foreach($name in @('editing_tests.xml','canvas_tests.xml','note_list_tests.xml')) {
    [xml]$xml=Get-Content -LiteralPath (Join-Path $evidence $name) -Raw
    $suite=$xml.testsuite
    if([int]$suite.tests -lt 1 -or [int]$suite.failures -ne 0 -or [int]$suite.errors -ne 0 -or [int]$suite.skipped -ne 0){throw "Failed JVM suite: $name"}
    $suites += [pscustomobject]@{name=$suite.name;tests=[int]$suite.tests;failures=0;errors=0}
}
$rust=@()
foreach($name in @('native_tests.log','sourcegen_tests.log','packager_tests.log')) {
    $log=[IO.File]::ReadAllText((Join-Path $evidence $name))
    $taskRustMatches=[regex]::Matches($log,'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;')
    if(!$taskRustMatches.Count){throw "Missing successful Rust tests: $name"}
    $match=$taskRustMatches[$taskRustMatches.Count-1]
    $rust += [pscustomobject]@{name=$name;passed=[int]$match.Groups[1].Value;ignored=[int]$match.Groups[2].Value}
}
[xml]$lint=Get-Content -LiteralPath (Join-Path $evidence 'lint-results-release.xml') -Raw
$lintErrors=@($lint.issues.issue | Where-Object {$_.severity -in @('Error','Fatal')}).Count
if($lintErrors){throw 'Release lint errors remain'}
$mutations=@()
foreach($name in @('canvas_mutation/mutation_result.json','canvas_ui_mutation/mutation_result.json','note_performance/acceptance.json')) {
    $record=Read-Evidence $name
    if(!$record.passed -or @($record.cases | Where-Object {!$_.passed}).Count){throw "Mutation check failed: $name"}
    $mutations += [pscustomobject]@{evidence=$name;cases=$record.cases}
}
$scenarios=Read-Evidence 'canvas_scenarios/result.json'
if(!$scenarios.passed -or $scenarios.scenarios.Count -ne 9){throw 'Canvas persistence scenarios incomplete'}
$receipt=[pscustomobject]@{passed=$true;version=$version;versionCode=$delivery.versionCode;sha256=$delivery.sha256;sourceSnapshotSha256=$build.sourceSnapshotSha256;testPhase='before_publication';packageCheckPhase='after_publication';rustTests=$rust;jvmTests=$suites;jvmTestTotal=($suites | Measure-Object tests -Sum).Sum;lintErrors=$lintErrors;lintWarnings=@($lint.issues.issue | Where-Object severity -eq 'Warning').Count;mutationChecks=$mutations;canvasScenarios=9;performanceHostOnly=$true;measuredQueriesMatchFinalSource=$true;formalApkOnly=$package.formalApkOnly;upgradeCertificateUnchanged=$package.upgradeCertificateUnchanged;windowsArtifactsUnchanged=$package.windowsArtifactsUnchanged;noDeviceOperations=$true;completed=[DateTime]::UtcNow.ToString('o')}
$receipt | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $evidence 'performance_acceptance.json') -Encoding utf8
$receipt | Select-Object passed,version,versionCode,jvmTestTotal,lintErrors,lintWarnings,sha256 | Format-List
