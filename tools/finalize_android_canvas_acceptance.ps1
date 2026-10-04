# v0.0.1 - Reconcile actual post-publication logs, fresh reports, source hashes and delivered APKs.
# This does not run or skip tests. It only accepts complete execution evidence newer than publication.
$ErrorActionPreference='Stop'
$auditRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match((Get-Content (Join-Path $auditRoot 'app/build.gradle') -Raw),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $auditRoot "release_artifacts/verification/v$version"
$publication=Get-Content (Join-Path $evidence 'delivery_receipt.json') -Raw | ConvertFrom-Json
$build=Get-Content (Join-Path $evidence 'release_build_acceptance.json') -Raw | ConvertFrom-Json
$publishedAt=([DateTime]$publication.completed).ToUniversalTime()
if(!$publication.passed -or !$build.passed -or $publication.version -ne $version -or $build.version -ne $version -or $build.apkSha256 -ne $publication.sha256){throw 'Release identity mismatch'}
function Read-FreshEvidence([string]$Name){
    $path=Join-Path $evidence $Name
    if((Get-Item -LiteralPath $path).LastWriteTimeUtc -le $publishedAt){throw "Evidence predates publication: $Name"}
    return Get-Content -LiteralPath $path -Raw
}
$provenance=Get-Content (Join-Path $evidence 'android_source_provenance.json') -Raw | ConvertFrom-Json
if($provenance.sha256 -ne $build.sourceSnapshotSha256 -or $publication.sourceSnapshotSha256 -ne $build.sourceSnapshotSha256){throw 'Source provenance mismatch'}
foreach($file in $provenance.files){if((Get-FileHash -LiteralPath (Join-Path $auditRoot $file.path)).Hash -ne $file.sha256){throw "Source differs from the released build: $($file.path)"}}
$native=foreach($item in @(@('native_tests.log',783,7,4),@('sourcegen_tests.log',109,0,0),@('packager_tests.log',61,2,0))){
    $log=Read-FreshEvidence $item[0]
    $match=[regex]::Match($log,'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out;')
    if(!$match.Success -or [int]$match.Groups[1].Value -ne $item[1] -or [int]$match.Groups[2].Value -ne 0 -or [int]$match.Groups[3].Value -ne $item[2] -or [int]$match.Groups[4].Value -ne $item[3]){throw "Incomplete native execution evidence: $($item[0])"}
    [pscustomobject]@{log=$item[0];passed=$item[1];failed=0;ignored=$item[2];filtered=$item[3];completed=(Get-Item (Join-Path $evidence $item[0])).LastWriteTimeUtc.ToString('o')}
}
$checkLog=Read-FreshEvidence 'android_build.log'
if(!$checkLog.Contains('BUILD SUCCESSFUL')){throw 'Post-publication Android check did not finish'}
$freshLog=Read-FreshEvidence 'post_publish_fresh_jvm.log'
if(!$freshLog.Contains('BUILD SUCCESSFUL')){throw 'Fresh JVM and lint execution did not finish'}
foreach($task in @('testReleaseUnitTest','lintAnalyzeRelease','lintAnalyzeReleaseAndroidTest','lintAnalyzeReleaseUnitTest','lintReportRelease','lintRelease')){
    if($freshLog -notmatch ('(?m)^> Task :app:'+[regex]::Escape($task)+'\r?$')){throw "Task did not execute freshly: $task"}
}
$suites=foreach($item in @(@('KnowledgeCanvasTest',9),@('TenfoldEditingTest',98))){
    $name='post_publish_TEST-com.ofairyo.gridtimer.ui.'+$item[0]+'.xml'
    [xml]$xml=Read-FreshEvidence $name
    if([int]$xml.testsuite.tests -ne $item[1] -or [int]$xml.testsuite.failures -ne 0 -or [int]$xml.testsuite.errors -ne 0 -or [int]$xml.testsuite.skipped -ne 0){throw "Incomplete or failed JVM suite: $name"}
    [pscustomobject]@{name=[string]$xml.testsuite.name;tests=[int]$xml.testsuite.tests;failures=0;errors=0;skipped=0;modified=(Get-Item (Join-Path $evidence $name)).LastWriteTimeUtc.ToString('o')}
}
[xml]$lint=Read-FreshEvidence 'post_publish_lint.xml'
if(@($lint.issues.issue | Where-Object {$_.severity -in @('Fatal','Error')}).Count){throw 'Lint contains errors'}
$scenarios=Get-Content (Join-Path $evidence 'canvas_scenarios/result.json') -Raw | ConvertFrom-Json
if(!$scenarios.passed -or $scenarios.scenarios.Count -ne 9 -or ([DateTime]$scenarios.completed).ToUniversalTime() -le $publishedAt -or $scenarios.sourceSha256 -ne (Get-FileHash (Join-Path $auditRoot 'native/gridtimer_native/src/android_canvas.rs')).Hash){throw 'Canvas scenario results are stale or incomplete'}
Copy-Item -LiteralPath (Join-Path $evidence 'canvas_scenarios/result.json') -Destination (Join-Path $evidence 'post_publish_canvas_scenarios.json') -Force
& (Join-Path $PSScriptRoot 'publish_android_canvas.ps1') -Mode verify
$package=Get-Content (Join-Path $evidence 'post_publish_package_check.json') -Raw | ConvertFrom-Json
if(!$package.passed -or $package.sha256 -ne $publication.sha256){throw 'Published package changed'}
$before=Get-Content (Join-Path $evidence 'release_manifest_before.json') -Raw | ConvertFrom-Json
$current=Get-Content (Join-Path $auditRoot 'release_artifacts/current/release_manifest.json') -Raw | ConvertFrom-Json
if($current.version -ne $before.version){throw 'Windows version changed during Android publication'}
foreach($file in $before.files | Where-Object role -ne 'apk'){
    $retained=@($current.files | Where-Object role -eq $file.role)
    if($retained.Count -ne 1 -or $retained[0].file_name -ne $file.file_name -or $retained[0].sha256 -ne $file.sha256 -or $retained[0].size -ne $file.size){throw 'Windows release descriptor changed'}
}
[ordered]@{
    passed=$true;version=$version;noDeviceOperations=$true;sourceSnapshotSha256=$build.sourceSnapshotSha256;apkSha256=$publication.sha256
    publicationCompleted=$publishedAt.ToString('o');nativeSuites=$native;jvmTestsFreshlyExecuted=$true;jvmSuites=$suites
    lintFreshlyExecuted=$true;lintErrors=0;lintWarnings=@($lint.issues.issue | Where-Object severity -eq 'Warning').Count
    canvasScenarios=$scenarios;packageVerified=$true;windowsRetained=$true;completed=(Get-Date).ToUniversalTime().ToString('o')
} | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $evidence 'post_publish_acceptance.json') -Encoding utf8
Write-Output "Raw execution evidence and delivered Android package verified: $version"
