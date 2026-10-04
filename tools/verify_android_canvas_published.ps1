# v0.0.3 - Preserve UTC when checking scenario receipts and audit raw execution evidence.
# v0.0.2 - Aggregate structured test results correctly and force fresh lint analysis and reports.
# v0.0.1 - Run complete offline acceptance after publication and force fresh JVM tests.
$ErrorActionPreference='Stop'
$verifyRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match((Get-Content (Join-Path $verifyRoot 'app/build.gradle') -Raw),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $verifyRoot "release_artifacts/verification/v$version"
$published=Get-Content (Join-Path $evidence 'delivery_receipt.json') -Raw | ConvertFrom-Json
if(!$published.passed -or $published.version -ne $version){throw 'Publish the verified Android release first'}
$buildPath=Join-Path $evidence 'offline_build_acceptance.json'
$buildReceipt=[IO.File]::ReadAllText($buildPath)
$build=$buildReceipt | ConvertFrom-Json
if($build.mode -ne 'build' -or !$build.passed){throw 'Original build receipt missing'}
[IO.File]::WriteAllText((Join-Path $evidence 'release_build_acceptance.json'),$buildReceipt,[Text.UTF8Encoding]::new($false))
if(!(Test-Path -LiteralPath (Join-Path $evidence 'release_build.log'))){Copy-Item -LiteralPath (Join-Path $evidence 'android_build.log') -Destination (Join-Path $evidence 'release_build.log')}
$oldJava=$env:JAVA_HOME; $oldPath=$env:PATH
Push-Location $verifyRoot
try {
    & (Join-Path $PSScriptRoot 'android.ps1') -Mode check
    $check=Get-Content -LiteralPath $buildPath -Raw | ConvertFrom-Json
    if(!$check.passed -or $check.mode -ne 'check' -or $check.sourceSnapshotSha256 -ne $build.sourceSnapshotSha256){throw 'Post-publication acceptance mismatch'}
    $env:JAVA_HOME='C:\tools\java\jdk-17.0.18+8'
    $env:PATH="$env:JAVA_HOME\bin;$env:PATH"
    $started=(Get-Date).ToUniversalTime()
    & 'C:\tools\gradle-8.7\bin\gradle.bat' --offline --no-daemon '-Dorg.gradle.jvmargs=-Xmx2560m -Xms256m -XX:MaxMetaspaceSize=512m -XX:TieredStopAtLevel=1 -Dfile.encoding=GBK' :app:testReleaseUnitTest --rerun :app:lintAnalyzeRelease --rerun :app:lintAnalyzeReleaseAndroidTest --rerun :app:lintAnalyzeReleaseUnitTest --rerun :app:lintReportRelease --rerun :app:lintRelease --rerun *> (Join-Path $evidence 'post_publish_fresh_jvm.log')
    if($LASTEXITCODE -ne 0){Get-Content (Join-Path $evidence 'post_publish_fresh_jvm.log') -Tail 35; throw 'Fresh post-publication JVM tests or lint failed'}
    $suites=@(Get-ChildItem -LiteralPath (Join-Path $verifyRoot 'app/build/test-results/testReleaseUnitTest') -File -Filter 'TEST-*.xml' | ForEach-Object {
        [xml]$result=Get-Content -LiteralPath $_.FullName -Raw
        if([int]$result.testsuite.failures -ne 0 -or [int]$result.testsuite.errors -ne 0 -or [int]$result.testsuite.skipped -ne 0 -or $_.LastWriteTimeUtc -lt $started){throw 'Failed or stale JVM result after publication'}
        Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $evidence ('post_publish_'+$_.Name)) -Force
        [pscustomobject][ordered]@{name=[string]$result.testsuite.name;tests=[int]$result.testsuite.tests;failures=0;errors=0;skipped=0;modified=$_.LastWriteTimeUtc.ToString('o')}
    })
    if(@($suites | Where-Object name -eq 'com.ofairyo.gridtimer.ui.KnowledgeCanvasTest').Count -ne 1 -or ($suites | Measure-Object -Property tests -Sum).Sum -lt 107){throw 'Incomplete post-publication JVM suite'}
    $lintPath=Join-Path $verifyRoot 'app/build/reports/lint-results-release.xml'
    [xml]$lint=Get-Content -LiteralPath $lintPath -Raw
    if((Get-Item -LiteralPath $lintPath).LastWriteTimeUtc -lt $started -or @($lint.issues.issue | Where-Object {$_.severity -in @('Fatal','Error')}).Count){throw 'Failed or stale lint analysis after publication'}
    Copy-Item -LiteralPath $lintPath -Destination (Join-Path $evidence 'post_publish_lint.xml') -Force
    & (Join-Path $PSScriptRoot 'verify_canvas_scenarios.ps1')
    $scenarios=Get-Content (Join-Path $evidence 'canvas_scenarios/result.json') -Raw | ConvertFrom-Json
    if(!$scenarios.passed -or ([DateTime]$scenarios.completed).ToUniversalTime() -lt $started){throw 'Canvas scenarios were not executed after publication'}
    Copy-Item -LiteralPath (Join-Path $evidence 'canvas_scenarios/result.json') -Destination (Join-Path $evidence 'post_publish_canvas_scenarios.json') -Force
    [IO.File]::WriteAllText($buildPath,$buildReceipt,[Text.UTF8Encoding]::new($false))
    & (Join-Path $PSScriptRoot 'publish_android_canvas.ps1') -Mode verify
    $package=Get-Content (Join-Path $evidence 'post_publish_package_check.json') -Raw | ConvertFrom-Json
    if(!$package.passed -or $package.sha256 -ne $published.sha256){throw 'Published APK changed during acceptance'}
    [ordered]@{passed=$true;version=$version;noDeviceOperations=$true;sourceSnapshotSha256=$build.sourceSnapshotSha256;apkSha256=$published.sha256;nativeSourcegenPackagerAcceptance=$check;jvmTestsFreshlyExecuted=$true;jvmSuites=$suites;packageVerified=$true;completed=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json -Depth 15 | Set-Content -LiteralPath (Join-Path $evidence 'post_publish_acceptance.json') -Encoding utf8
} finally {
    [IO.File]::WriteAllText($buildPath,$buildReceipt,[Text.UTF8Encoding]::new($false))
    $env:JAVA_HOME=$oldJava; $env:PATH=$oldPath
    Pop-Location
}
Write-Output "Post-publication Android acceptance passed: $version"
& (Join-Path $PSScriptRoot 'finalize_android_canvas_acceptance.ps1')
