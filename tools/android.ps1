# v0.0.24 - Require legal scan request settlement and retry business acceptance.
# v0.0.23 - Require current-document loading and parenthesized formula business acceptance.
# v0.0.22 - Freeze offline formula assets and verify Android answer rendering.
# v0.0.21 - Gate usable AI request boundaries and safe sync failure tests.
# v0.0.20 - Gate Android AI connection state tests and session source formatting.
# v0.0.19 - Support bounded JVM memory and worker count in the formal Android build.
# v0.0.18 - Gate source-complete grouping and knowledge filter state tests.
# v0.0.17 - Include bundled language catalogs in Android source provenance.
# v0.0.16 - Include note collection boundaries and compact canvas reply regression tests.
# v0.0.15 - Verify touch-canvas state, native transactions and offline regression tests.
# v0.0.14 - Verify streaming Android startup sources and native database receipts.
# v0.0.13 - Keep Windows transport cancellation suites in Windows acceptance.
# v0.0.12 - Verify run-bound rest bell sources and phase transitions.
# v0.0.11 - Verify structured knowledge compatibility and shared validators.
# v0.0.10 - Verify device-local timer synchronization sources.
# v0.0.9 - Verify the local startup source and transaction receipts.
# v0.0.8 - Track Android inputs separately from Windows client editing.
# v0.0.7 - Verify note lifecycle recovery in the formal build.
# v0.0.6 - Verify the pause layout source in the formal build.
[CmdletBinding()]
param(
    [ValidateSet('check','build')][string]$Mode = 'build',
    [switch]$ReuseNativeChecksForFixtureRepair,
    [ValidateRange(1024,4096)][int]$GradleHeapMiB = 2560,
    [ValidateRange(1,4)][int]$GradleWorkers = 2
)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskCrate = Join-Path $taskRoot 'native\gridtimer_native'
$taskGradleText = Get-Content -LiteralPath (Join-Path $taskRoot 'app\build.gradle') -Raw
$taskVersion = [regex]::Match($taskGradleText, "versionName '([^']+)'").Groups[1].Value
if($taskVersion -notmatch '^\d+\.\d+(?:\.\d+){0,2}[a-zA-Z0-9.-]*$'){throw 'Invalid Android version'}
$taskEvidence = Join-Path $taskRoot "release_artifacts\verification\v$taskVersion"
New-Item -ItemType Directory -Force -Path $taskEvidence | Out-Null

function Write-TaskJson([string]$Name, $Value) {
    [IO.File]::WriteAllText((Join-Path $taskEvidence $Name), ($Value | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
}
function Get-TaskInputs {
    $paths = @('app\build.gradle','gradle.properties','settings.gradle','native\gridtimer_native\Cargo.toml','native\gridtimer_native\Cargo.lock','tools\android.ps1','tools\publish_android_note.ps1','tools\verify_android_math_mutation.ps1','tools\verify_android_markdown_loading_mutation.ps1','tools\verify_legal_lifecycle_mutation.ps1') | ForEach-Object { Join-Path $taskRoot $_ }
    # Cargo gates timer_windows_client behind the desktop feature. Its private
    # desktop/ modules are not inputs to the Android library, generator or tests.
    # Shared library modules (including desktop_*.rs) remain in the snapshot.
    $taskWindowsClient = Join-Path $taskCrate 'src\bin\timer_windows_client.rs'
    $taskWindowsModules = (Join-Path $taskCrate 'src\desktop') + '\'
    $paths += Get-ChildItem -LiteralPath (Join-Path $taskCrate 'src') -Filter '*.rs' -File -Recurse | Where-Object {
        $_.FullName -ne $taskWindowsClient -and -not $_.FullName.StartsWith($taskWindowsModules,[StringComparison]::OrdinalIgnoreCase)
    } | ForEach-Object { $_.FullName }
    $paths += Get-ChildItem -LiteralPath (Join-Path $taskCrate 'examples') -Filter '*.rs' -File -Recurse | ForEach-Object { $_.FullName }
    $paths += Get-ChildItem -LiteralPath (Join-Path $taskCrate 'src\sourcegen\ui_locales') -File | ForEach-Object { $_.FullName }
    # The Android answer renderer embeds these shared offline assets. Unlike
    # private desktop Rust modules, their exact bytes are Android build inputs.
    $paths += @('katex_v0.18.7.min.js','katex_v0.18.7_embedded.css','katex_manifest.json','katex_LICENSE.txt') | ForEach-Object { Join-Path $taskCrate ('src\desktop\assets\'+$_) }
    $paths += Get-ChildItem -LiteralPath (Join-Path $taskRoot 'app\src') -File -Recurse | ForEach-Object { $_.FullName }
    $files = @($paths | Sort-Object -Unique | ForEach-Object {
        [ordered]@{path=$_.Substring($taskRoot.Length+1).Replace('\','/');sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()}
    })
    $bytes = [Text.Encoding]::UTF8.GetBytes(($files | ConvertTo-Json -Depth 4 -Compress))
    [ordered]@{sha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant();files=$files}
}
function Invoke-TaskCommand([string]$Executable, [string[]]$Arguments, [string]$Log) {
    $logPath = Join-Path $taskEvidence $Log
    & $Executable @Arguments *> $logPath
    if($LASTEXITCODE -ne 0){Get-Content -LiteralPath $logPath -Tail 30; throw "$Log failed with exit code $LASTEXITCODE"}
    Write-Output "$Log passed"
}

$taskEnvironmentNames = @('CARGO_HOME','CARGO_TARGET_DIR','RUSTUP_TOOLCHAIN','CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER','LIB','JAVA_HOME','PATH')
$taskOldEnvironment = @{}
foreach($name in $taskEnvironmentNames){$taskOldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name,'Process')}
Push-Location $taskRoot
try {
    $env:CARGO_HOME = if($env:CARGO_HOME){$env:CARGO_HOME}else{Join-Path $env:USERPROFILE '.cargo'}
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    $taskRustc = (& rustup which --toolchain $env:RUSTUP_TOOLCHAIN rustc).Trim()
    if($LASTEXITCODE -ne 0){throw 'Rust toolchain unavailable'}
    $taskToolchain = Split-Path -Parent (Split-Path -Parent $taskRustc)
    $taskCargo = Join-Path $taskToolchain 'bin\cargo.exe'
    $taskRustfmt = Join-Path $taskToolchain 'bin\rustfmt.exe'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $taskToolchain 'lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe'
    $env:CARGO_TARGET_DIR = 'C:\gt\gridtimer-build\sourcegen'
    $env:LIB = 'C:\tools\xwin\crt\lib\x86_64;C:\tools\xwin\sdk\lib\ucrt\x86_64;C:\tools\xwin\sdk\lib\um\x86_64'
    $env:JAVA_HOME = 'C:\tools\java\jdk-17.0.18+8'
    $env:PATH = "$env:JAVA_HOME\bin;$env:PATH"
    $javaProperties = (& "$env:JAVA_HOME\bin\java.exe" -XshowSettings:properties -version 2>&1 | Out-String)
    $taskNativeEncoding = [regex]::Match($javaProperties,'native.encoding\s*=\s*([^\s]+)').Groups[1].Value
    if([string]::IsNullOrWhiteSpace($taskNativeEncoding)){throw 'Cannot determine Java argument-file encoding'}
    # Java 17 reads worker argument files with the Windows native encoding.
    # Match the Gradle writer without changing the system locale or source encoding.
    $taskJvmArgs = "-Dorg.gradle.jvmargs=-Xmx${GradleHeapMiB}m -Xms256m -XX:MaxMetaspaceSize=512m -XX:TieredStopAtLevel=1 -XX:ActiveProcessorCount=2 -Dfile.encoding=$taskNativeEncoding"
    $before = Get-TaskInputs
    $reusedNativeChecks = $false
    if($ReuseNativeChecksForFixtureRepair){
        $priorSource = Get-Content -LiteralPath (Join-Path $taskEvidence 'android_source_provenance.json') -Raw | ConvertFrom-Json
        $allowedRepairs = @('tools/android.ps1','native/gridtimer_native/src/sourcegen/android_jvm_test_sources.rs')
        $priorInputs = @{}
        foreach($file in $priorSource.files){if($file.path -notin $allowedRepairs){$priorInputs[$file.path]=$file.sha256}}
        $currentInputs = @($before.files | Where-Object {$_.path -notin $allowedRepairs})
        if($priorInputs.Count -ne $currentInputs.Count){throw 'Fixture-only retry cannot reuse an altered source input set'}
        foreach($file in $currentInputs){
            if(!$priorInputs.ContainsKey($file.path) -or $priorInputs[$file.path] -cne $file.sha256){throw "Fixture-only retry found changed source: $($file.path)"}
        }
        $nativeLog = Get-Content -LiteralPath (Join-Path $taskEvidence 'native_tests.log') -Raw
        if($nativeLog -notmatch 'test result: ok\. \d+ passed; 0 failed;'){throw 'No passed native checks available for fixture-only retry'}
        Write-TaskJson 'native_checks_reuse.json' ([ordered]@{passed=$true;priorSourceSnapshotSha256=$priorSource.sha256;currentSourceSnapshotSha256=$before.sha256;unchangedApplicationAndNativeInputs=$true;allowedChangedFiles=$allowedRepairs;reason='Repair constructor-bypassing JVM fixture initialization; all native and application source inputs are byte-identical.'})
        $reusedNativeChecks = $true
    }
    Write-TaskJson 'android_source_provenance.json' $before
    $formatPaths = @('src\app_data.rs','src\lib.rs','src\product_identity.rs','src\bin\gridtimer_sourcegen.rs','src\bin\gridtimer_packager.rs','src\sourcegen\android_test_fixes.rs','src\sourcegen\android_jvm_test_sources.rs','src\sourcegen\android_sources.rs','src\sourcegen\android_snapshot_memory.rs','src\sourcegen\android_startup_loading.rs','src\sourcegen\diagnostics_export_ui.rs','src\sourcegen\legal_report_sync_source.rs','src\timer_insights.rs','examples\tenfold_offline_acceptance.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_privacy_generation.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_timer_action.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_timer_layout.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_note_background.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_local_startup.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_startup_stream.rs'
    $formatPaths += Join-Path $taskCrate 'src\android_snapshot_verifier.rs'
    $formatPaths += Join-Path $taskCrate 'src\android_snapshot_codec.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_history_storage.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_startup_decode.rs'
    $formatPaths += Join-Path $taskCrate 'src\timer_sync.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_device_timer_sync.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_rest_bell.rs'
    $formatPaths += @('src\sourcegen\android_knowledge_compat.rs','src\knowledge.rs','src\knowledge_sync.rs','src\knowledge_references.rs','src\knowledge_app_data.rs','src\knowledge_exchange.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += @('src\android_canvas.rs','src\android_canvas_tests.rs','src\sourcegen\android_canvas_ui.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_note_list_performance.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_note_collection_recovery.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_knowledge_filters.rs'
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\android_ui_localization.rs'
    $formatPaths += @('src\legal_scan.rs','src\android_legal_session.rs','src\sourcegen\android_legal_integration.rs','src\sourcegen\legal_risk_ui_source.rs','src\sourcegen\legal_storage_source.rs','src\sourcegen\finance_risk_v2_ui_source.rs','src\finance_profile.rs','src\ai_client.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += @('src\sourcegen\my_account_source.rs','src\sourcegen\android_update_history_source.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += @('src\finance_money.rs','src\finance_precision_guard.rs','src\sourcegen\finance_money_source.rs','src\sourcegen\android_note_save_queue.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += Join-Path $taskCrate 'src\android_ai_session.rs'
    $formatPaths += @('src\sourcegen\android_ai_workflow.rs','src\sourcegen\android_sync_failure.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += @('src\android_answer_render.rs','src\sourcegen\android_ai_answer_ui.rs') | ForEach-Object { Join-Path $taskCrate $_ }
    $formatPaths += Join-Path $taskCrate 'src\sourcegen\legal_risk_ui_source.rs'
    Invoke-TaskCommand $taskRustfmt (@('--check','--edition','2021','--config','skip_children=true') + $formatPaths) 'rust_format.log'
    $cargoBase = @('--manifest-path',(Join-Path $taskCrate 'Cargo.toml'),'--locked','--offline')
    # These suites exercise runtime cancellation and WinHTTP, neither of which
    # exists in the Android target. All shared native suites still run here.
    $taskNonAndroidSuites = @('sync_core::cancellation_tests::','sync_core::windows_http::')
    $taskNativeTestArgs = @('test') + $cargoBase + @('--lib','--')
    foreach($suite in $taskNonAndroidSuites){$taskNativeTestArgs += @('--skip',$suite)}
    if($reusedNativeChecks){Write-Output 'native_tests.log reused: application and native inputs are unchanged'}
    else {Invoke-TaskCommand $taskCargo $taskNativeTestArgs 'native_tests.log'}
    $taskMathSource = Get-Content -LiteralPath (Join-Path $taskCrate 'src\android_answer_render.rs') -Raw
    $taskMathTests = @(([regex]'(?m)#\[test\]\s*fn\s+(?<name>[A-Za-z0-9_]+)\s*\(').Matches($taskMathSource) | ForEach-Object { $_.Groups['name'].Value })
    $taskMathNativeLog = Get-Content -LiteralPath (Join-Path $taskEvidence 'native_tests.log') -Raw
    if($taskMathTests.Count -lt 1){throw 'Android answer renderer business tests are missing'}
    if([version]$taskVersion -ge [version]'2.23.2.7' -and ($taskMathTests.Count -lt 6 -or 'android_math_render_accepts_parenthesized_whitespace_without_treating_currency_as_math' -notin $taskMathTests)){throw 'Parenthesized formula and currency boundary business acceptance is missing'}
    foreach($testName in $taskMathTests){
        if($taskMathNativeLog -notmatch ('(?m)^test android_answer_render::tests::'+[regex]::Escape($testName)+' \.\.\. ok\s*$')){throw ('Android answer renderer business test did not pass: '+$testName)}
    }
    Invoke-TaskCommand $taskCargo (@('test') + $cargoBase + @('--bin','gridtimer_sourcegen')) 'sourcegen_tests.log'
    Invoke-TaskCommand $taskCargo (@('test') + $cargoBase + @('--bin','gridtimer_packager')) 'packager_tests.log'
    $gradleTasks = @(':app:testReleaseUnitTest',':app:lintRelease')
    if($Mode -eq 'build'){$gradleTasks += ':app:assembleRelease'}
    Invoke-TaskCommand 'C:\tools\gradle-8.7\bin\gradle.bat' (@('--offline','--no-daemon',('--max-workers='+$GradleWorkers),$taskJvmArgs) + $gradleTasks) 'android_build.log'
    $after = Get-TaskInputs
    if($before.sha256 -ne $after.sha256){throw 'Source inputs changed during verification'}
    $junitPath = Join-Path $taskRoot 'app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer.ui.TenfoldEditingTest.xml'
    [xml]$junit = Get-Content -LiteralPath $junitPath -Raw
    $expectedTests = ([regex]'@Test fun ').Matches((Get-Content -LiteralPath (Join-Path $taskCrate 'src\sourcegen\android_jvm_test_sources.rs') -Raw)).Count
    if([int]$junit.testsuite.tests -ne $expectedTests -or [int]$junit.testsuite.failures -ne 0 -or [int]$junit.testsuite.errors -ne 0 -or [int]$junit.testsuite.skipped -ne 0){throw 'Incomplete or failed editing tests'}
    Copy-Item -LiteralPath $junitPath -Destination (Join-Path $taskEvidence 'editing_tests.xml') -Force
    $canvasJunitPath = Join-Path $taskRoot 'app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer.ui.KnowledgeCanvasTest.xml'
    [xml]$canvasJunit = Get-Content -LiteralPath $canvasJunitPath -Raw
    $expectedCanvasTests = ([regex]'@Test fun ').Matches((Get-Content -LiteralPath (Join-Path $taskCrate 'src\sourcegen\android_canvas_ui.rs') -Raw)).Count
    if([int]$canvasJunit.testsuite.tests -ne $expectedCanvasTests -or [int]$canvasJunit.testsuite.failures -ne 0 -or [int]$canvasJunit.testsuite.errors -ne 0 -or [int]$canvasJunit.testsuite.skipped -ne 0){throw 'Incomplete or failed canvas tests'}
    Copy-Item -LiteralPath $canvasJunitPath -Destination (Join-Path $taskEvidence 'canvas_tests.xml') -Force
    $noteJunitPath = Join-Path $taskRoot 'app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer.ui.NoteListPerformanceTest.xml'
    [xml]$noteJunit = Get-Content -LiteralPath $noteJunitPath -Raw
    $expectedNoteTests = ([regex]'@Test fun ').Matches((Get-Content -LiteralPath (Join-Path $taskCrate 'src\sourcegen\android_note_list_performance.rs') -Raw)).Count
    if([int]$noteJunit.testsuite.tests -ne $expectedNoteTests -or [int]$noteJunit.testsuite.failures -ne 0 -or [int]$noteJunit.testsuite.errors -ne 0 -or [int]$noteJunit.testsuite.skipped -ne 0){throw 'Incomplete or failed note collection tests'}
    Copy-Item -LiteralPath $noteJunitPath -Destination (Join-Path $taskEvidence 'note_list_tests.xml') -Force
    $startupJunitPath = Join-Path $taskRoot 'app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer.data.StartupSnapshotDecodeTest.xml'
    [xml]$startupJunit = Get-Content -LiteralPath $startupJunitPath -Raw
    $expectedStartupTests = ([regex]'@Test fun ').Matches((Get-Content -LiteralPath (Join-Path $taskCrate 'src\sourcegen\android_startup_decode.rs') -Raw)).Count
    if([int]$startupJunit.testsuite.tests -ne $expectedStartupTests -or [int]$startupJunit.testsuite.failures -ne 0 -or [int]$startupJunit.testsuite.errors -ne 0 -or [int]$startupJunit.testsuite.skipped -ne 0){throw 'Incomplete or failed startup decode tests'}
    Copy-Item -LiteralPath $startupJunitPath -Destination (Join-Path $taskEvidence 'startup_decode_tests.xml') -Force
    $taskKnowledgeTestCounts = [ordered]@{}
    foreach($suite in @(
        @{name='NoteCollectionRecoveryTest';package='core';source='android_note_collection_recovery.rs'},
        @{name='KnowledgeFilterStateTest';package='ui';source='android_knowledge_filters.rs'},
        @{name='NoteSaveQueueTest';package='ui';source='android_note_save_queue.rs'},
        @{name='FinanceMoneyTest';package='data';source='finance_money_source.rs'}
    )){
        $suitePath = Join-Path $taskRoot ("app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer."+$suite.package+"."+$suite.name+".xml")
        [xml]$suiteResult = Get-Content -LiteralPath $suitePath -Raw
        $suiteExpected = ([regex]'@Test\s+fun ').Matches((Get-Content -LiteralPath (Join-Path $taskCrate ("src\sourcegen\"+$suite.source)) -Raw)).Count
        if($suiteExpected -lt 1 -or [int]$suiteResult.testsuite.tests -ne $suiteExpected -or [int]$suiteResult.testsuite.failures -ne 0 -or [int]$suiteResult.testsuite.errors -ne 0 -or [int]$suiteResult.testsuite.skipped -ne 0){throw ("Incomplete or failed knowledge suite: "+$suite.name)}
        Copy-Item -LiteralPath $suitePath -Destination (Join-Path $taskEvidence ($suite.name+'.xml')) -Force
        $taskKnowledgeTestCounts[$suite.name] = $suiteExpected
    }
    $taskAiSource = Get-Content -LiteralPath (Join-Path $taskCrate 'src\sourcegen\my_account_source.rs') -Raw
    $taskAiTestSource = [regex]::Match($taskAiSource, '(?s)pub const AI_TEST_CONTENTS:\s*&str\s*=\s*r(?<hashes>#+)"(?<body>.*?)"\k<hashes>;')
    if(!$taskAiTestSource.Success){throw 'Android AI connection test source is missing'}
    $taskAiExpected = ([regex]'@Test\s+fun ').Matches($taskAiTestSource.Groups['body'].Value).Count
    $taskAiJunitPath = Join-Path $taskRoot 'app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer.ui.AiConnectionControllerTest.xml'
    [xml]$taskAiJunit = Get-Content -LiteralPath $taskAiJunitPath -Raw
    if($taskAiExpected -lt 1 -or [int]$taskAiJunit.testsuite.tests -ne $taskAiExpected -or [int]$taskAiJunit.testsuite.failures -ne 0 -or [int]$taskAiJunit.testsuite.errors -ne 0 -or [int]$taskAiJunit.testsuite.skipped -ne 0){throw 'Incomplete or failed AI connection state tests'}
    Copy-Item -LiteralPath $taskAiJunitPath -Destination (Join-Path $taskEvidence 'AiConnectionControllerTest.xml') -Force
    $taskAiWorkflowCounts = [ordered]@{}
    foreach ($suite in @(
        @{name='KnowledgeAiRequestBoundaryTest';package='ui';source='android_ai_workflow.rs'},
        @{name='UnboundSyncFailureTest';package='data';source='android_sync_failure.rs'},
        @{name='AndroidMarkdownRenderBoundaryTest';package='ui';source='android_ai_answer_ui.rs'},
        @{name='LegalSendReadyTest';package='ui';source='legal_risk_ui_source.rs'}
    )) {
        $suiteSource = Get-Content -LiteralPath (Join-Path $taskCrate ('src\sourcegen\'+$suite.source)) -Raw
        $suiteBody = [regex]::Match($suiteSource, '(?s)pub const TEST_CONTENTS:\s*&str\s*=\s*r(?<hashes>#+)"(?<body>.*?)"\k<hashes>;')
        if (!$suiteBody.Success) { throw 'AI workflow test literal is missing' }
        $suiteExpected = ([regex]'@Test\s+fun ').Matches($suiteBody.Groups['body'].Value).Count
        $suitePath = Join-Path $taskRoot ('app\build\test-results\testReleaseUnitTest\TEST-com.ofairyo.gridtimer.'+$suite.package+'.'+$suite.name+'.xml')
        [xml]$suiteResult = Get-Content -LiteralPath $suitePath -Raw
        if ($suiteExpected -lt 1 -or [int]$suiteResult.testsuite.tests -ne $suiteExpected -or [int]$suiteResult.testsuite.failures -ne 0 -or [int]$suiteResult.testsuite.errors -ne 0 -or [int]$suiteResult.testsuite.skipped -ne 0) { throw ('AI workflow suite failed: '+$suite.name) }
        if([version]$taskVersion -ge [version]'2.23.2.7' -and $suite.name -eq 'AndroidMarkdownRenderBoundaryTest'){
            if($suiteExpected -lt 7){throw 'Current offline document loading boundary suite is incomplete'}
            foreach($testName in @('onlyExactCurrentMainDocumentCanLoadOffline','plainMarkdownNeedsItsDocumentAndBodyWithoutMathBootstrap','incompleteFormulaOrForeignDocumentCannotBecomeReady','explicitFailureCanRetryWithoutAcceptingOldCompletion')){
                if(@($suiteResult.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1){throw ('Current offline document business test is missing: '+$testName)}
            }
        }
        if($suite.name -eq 'LegalSendReadyTest'){
            if($suiteExpected -ne 13){throw 'Legal scan request lifecycle business suite must contain 13 tests'}
            foreach($testName in @('emptyScanCannotSend','missingAiConfigurationCannotSend','invalidatedAnalysisSettlesWithoutPublishingAndAllowsRetry','invalidatedPreparationCannotOverlapItsReplacement','duplicateClicksAndLateFinallyCannotReleaseAnotherWorker','closeRejectsLatePublicationCleanupAndNewRequests','invalidationDuringSuspendedIoStillClearsBusyInFinally','oldWorkspaceCannotReleaseNewWorkspaceRequest')){
                if(@($suiteResult.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1){throw ('Legal scan lifecycle business test is missing: '+$testName)}
            }
        }
        Copy-Item -LiteralPath $suitePath -Destination (Join-Path $taskEvidence ($suite.name+'.xml')) -Force
        $taskAiWorkflowCounts[$suite.name] = $suiteExpected
    }
    Copy-Item -LiteralPath (Join-Path $taskRoot 'app\build\reports\lint-results-release.xml') -Destination (Join-Path $taskEvidence 'lint-results-release.xml') -Force
    $apk = Join-Path $taskRoot "app\build\outputs\apk\release\tenfold_v$taskVersion.apk"
    $result = [ordered]@{schemaVersion=1;version=$taskVersion;mode=$Mode;passed=$true;noDeviceOperations=$true;sourceSnapshotSha256=$before.sha256;editingTests=$expectedTests;nativeArgumentEncoding=$taskNativeEncoding;completed=(Get-Date).ToUniversalTime().ToString('o');checks=@('rust_format','native_tests','sourcegen_tests','packager_tests','android_source_audit','android_jvm_tests','android_lint')}
    $result['excludedNonAndroidTestSuites'] = $taskNonAndroidSuites
    $result['nativeChecksReusedForFixtureOnlyRepair'] = $reusedNativeChecks
    $result['gradleHeapMiB'] = $GradleHeapMiB
    $result['gradleWorkers'] = $GradleWorkers
    $result['canvasTests'] = $expectedCanvasTests
    $result['noteListTests'] = $expectedNoteTests
    $result['startupDecodeTests'] = $expectedStartupTests
    $result['knowledgeRecoveryTests'] = $taskKnowledgeTestCounts
    $result['aiConnectionTests'] = $taskAiExpected
    $result['aiWorkflowTests'] = $taskAiWorkflowCounts
    $result['mathNativeTests'] = $taskMathTests.Count
    $result['mathNativeTestNames'] = $taskMathTests
    if($Mode -eq 'build'){
        $result['apkSha256']=(Get-FileHash -LiteralPath $apk -Algorithm SHA256).Hash.ToLowerInvariant()
        $result['apkBytes']=(Get-Item -LiteralPath $apk).Length
        $result.checks += 'signed_release_build'
    }
    Write-TaskJson 'offline_build_acceptance.json' $result
    Write-Output "Offline Android $Mode completed: $taskVersion"
} finally {
    Pop-Location
    foreach($name in $taskEnvironmentNames){[Environment]::SetEnvironmentVariable($name,$taskOldEnvironment[$name],'Process')}
}
