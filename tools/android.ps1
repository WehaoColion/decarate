# v0.0.35 - Gate legal report navigation state tests and stale-selection mutation.
# v0.0.34 - Verify actual Agent progress tests and generated JNI polling.
# v0.0.33 - Gate executed document readability tests and the additional mutation case.
# v0.0.32 - Gate executed Agent review tests and the shared generated save policy.
# v0.0.31 - Freeze structured page edits and gate their real JVM models and write routes.
# v0.0.30 - Gate knowledge navigation format, actual JVM suites and generated routing.
# v0.0.29 - Include the document text alignment transform in formal Rust formatting.
# v0.0.28 - Gate document Markdown block projection and executed separator mutations.
# v0.0.27 - Freeze finance navigation mutation and gate its real generated integration.
# v0.0.26 - Freeze document caret and header verification and gate their generated integration.
# v0.0.25 - Freeze the legal workflow verifier and require generated confirmation boundaries.
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
    if([version]$taskVersion -ge [version]'2.23.2.9'){$paths += Join-Path $taskRoot 'tools\verify_legal_workflow_mutation.ps1'}
    if([version]$taskVersion -ge [version]'2.23.2.10'){$paths += @('tools\verify_document_caret_mutation.ps1','tools\verify_knowledge_header_mutation.ps1') | ForEach-Object {Join-Path $taskRoot $_}}
    if([version]$taskVersion -ge [version]'2.23.2.11'){$paths += Join-Path $taskRoot 'tools\verify_finance_workspace_mutation.ps1'}
    if([version]$taskVersion -ge [version]'2.23.2.12'){$paths += Join-Path $taskRoot 'tools\verify_document_markdown_mutation.ps1'}
    if([version]$taskVersion -ge [version]'2.23.2.23'){$paths += Join-Path $taskRoot 'tools/verify_android_agent_mutation.ps1'}
    if([version]$taskVersion -ge [version]'2.23.2.26'){$paths += Join-Path $taskRoot 'tools/verify_legal_navigation_mutation.ps1'}
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
    $formatPaths += @('src\sourcegen\android_knowledge_header.rs','src\sourcegen\android_document_caret.rs') | ForEach-Object {Join-Path $taskCrate $_}
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
    if([version]$taskVersion -ge [version]'2.23.2.9'){$formatPaths += Join-Path $taskCrate 'src\sourcegen\android_legal_workflow.rs'}
    if([version]$taskVersion -ge [version]'2.23.2.11'){$formatPaths += Join-Path $taskCrate 'src\sourcegen\android_finance_workspace.rs'}
    if([version]$taskVersion -ge [version]'2.23.2.12'){$formatPaths += Join-Path $taskCrate 'src\sourcegen\android_document_markdown.rs'}
    if([version]$taskVersion -ge [version]'2.23.2.13'){$formatPaths += Join-Path $taskCrate 'src\sourcegen\android_document_block_alignment.rs'}
    if([version]$taskVersion -ge [version]'2.23.2.15'){$formatPaths += Join-Path $taskCrate 'src\sourcegen\android_knowledge_navigation.rs'}
    if([version]$taskVersion -ge [version]'2.23.2.16'){$formatPaths += @('src\sourcegen\structured_mobile_data.rs','src\sourcegen\structured_mobile_tests.rs') | ForEach-Object {Join-Path $taskCrate $_}}
    if([version]$taskVersion -ge [version]'2.23.2.23'){$formatPaths += @('src/android_agent_upgrade.rs','src/android_agent_registry.rs','src/sourcegen/android_agent_upgrade.rs') | ForEach-Object {Join-Path $taskCrate $_}}
    if([version]$taskVersion -ge [version]'2.23.2.25'){$formatPaths += @('src/android_agent_progress.rs','src/sourcegen/android_agent_progress_ui.rs') | ForEach-Object {Join-Path $taskCrate $_}}
    if([version]$taskVersion -ge [version]'2.23.2.26'){$formatPaths += Join-Path $taskCrate 'src/sourcegen/android_legal_integration.rs'}
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
    $taskWorkflowSuites = @(
        @{name='KnowledgeAiRequestBoundaryTest';package='ui';source='android_ai_workflow.rs'},
        @{name='UnboundSyncFailureTest';package='data';source='android_sync_failure.rs'},
        @{name='AndroidMarkdownRenderBoundaryTest';package='ui';source='android_ai_answer_ui.rs'},
        @{name='LegalSendReadyTest';package='ui';source='legal_risk_ui_source.rs'}
    )
    if([version]$taskVersion -ge [version]'2.23.2.9'){$taskWorkflowSuites += @{name='LegalAnalysisWorkflowTest';package='ui';source='android_legal_workflow.rs'}}
    if([version]$taskVersion -ge [version]'2.23.2.10'){$taskWorkflowSuites += @{name='DocumentCaretPolicyTest';package='ui';source='android_document_caret.rs'}}
    if([version]$taskVersion -ge [version]'2.23.2.12'){$taskWorkflowSuites += @{name='DocumentMarkdownBlocksTest';package='ui';source='android_document_markdown.rs'}}
    if([version]$taskVersion -ge [version]'2.23.2.26'){$taskWorkflowSuites += @{name='NoteSelectionCheckTest';package='ui';source='android_legal_integration.rs'}}
    foreach ($suite in $taskWorkflowSuites) {
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
    if([version]$taskVersion -ge [version]'2.23.2.26'){
        Invoke-TaskCommand (Get-Command pwsh -ErrorAction Stop).Source @('-NoProfile','-File',(Join-Path $taskRoot 'tools/verify_legal_navigation_mutation.ps1'),'-Version',$taskVersion) 'legal_navigation_mutation.log'
        $navigationSource=[IO.File]::ReadAllText((Join-Path $taskCrate 'src/sourcegen/android_legal_integration.rs'))
        $navigationLiteral=[regex]::Match($navigationSource,'(?s)pub const HELPER_CONTENTS:\s*&str\s*=\s*r(?<hashes>#+)"(?<body>.*?)"\k<hashes>;').Groups['body'].Value
        $navigationHelper=[IO.File]::ReadAllText((Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/NoteSelectionCheck.kt'))
        if(!$navigationLiteral -or $navigationLiteral.Trim() -cne $navigationHelper.Trim()){throw 'Generated legal selection helper differs from tested production'}
        $navigationStudio=[IO.File]::ReadAllText((Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/NoteStudioSheet.kt'))
        if([regex]::Matches($navigationStudio,'NoteSelectionCheck\(selectedNoteId, selectedNote\?\.id\)').Count -ne 2 -or [regex]::Matches($navigationStudio,'noteSelectionCheck\.shouldClear\(selectedNoteId\)').Count -ne 2 -or $navigationStudio.Contains('LaunchedEffect(selectedNoteId, selectedNote)')){throw 'Legal selection policy must be connected to both generated note editors'}
        $navigationReceipt=Get-Content -LiteralPath (Join-Path $taskEvidence 'legal_navigation_mutation/receipt.json') -Raw | ConvertFrom-Json
        if(!$navigationReceipt.passed -or !$navigationReceipt.productionUnchanged -or $navigationReceipt.tests -ne $taskAiWorkflowCounts.NoteSelectionCheckTest -or $navigationReceipt.cases.Count -ne 4 -or @($navigationReceipt.cases | Where-Object {!$_.passed}).Count -ne 0){throw 'Legal record navigation mutation verification failed'}
    }
    if([version]$taskVersion -ge [version]'2.23.2.23'){
        $upgradeSource = Get-Content -LiteralPath (Join-Path $taskCrate 'src/android_agent_upgrade.rs') -Raw
        $upgradeTestNames = @([regex]::Matches($upgradeSource,'(?m)#\[test\]\s*fn\s+(\w+)\(') | ForEach-Object {$_.Groups[1].Value})
        $upgradeNativeLog = Get-Content -LiteralPath (Join-Path $taskEvidence 'native_tests.log') -Raw
        if($upgradeTestNames.Count -lt 1){throw 'Agent review domain tests are missing'}
        foreach($name in $upgradeTestNames){
            if($upgradeNativeLog -notmatch ('(?m)^test android_agent_upgrade::tests::'+[regex]::Escape($name)+' \.\.\. ok\s*$')){throw ('Agent review test did not execute: '+$name)}
        }
        $upgradeStudio = Get-Content -LiteralPath (Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/NoteStudioSheet.kt') -Raw
        if(([regex]::Matches($upgradeStudio,'KnowledgeAgentReviewPolicy\.canSave\(')).Count -ne 2 -or !$upgradeStudio.Contains('deepReview = latestAgentDeepReview')){throw 'Agent review gates are not connected to generated save function and button'}
        $upgradeHelper = Get-Content -LiteralPath (Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/KnowledgeAiRequestBoundary.kt') -Raw
        $upgradeHelperSource = Get-Content -LiteralPath (Join-Path $taskCrate 'src/sourcegen/android_ai_workflow.rs') -Raw
        $upgradeLiteral = [regex]::Match($upgradeHelperSource,'(?s)pub const CONTENTS:\s*&str\s*=\s*r(?<hashes>#+)"(?<body>.*?)"\k<hashes>;').Groups['body'].Value
        if($upgradeHelper.Trim() -cne $upgradeLiteral.Trim()){throw 'Tested Agent review helper differs from generated model'}
        $upgradeMutation = Get-Content -LiteralPath (Join-Path $taskEvidence 'agent_mutation/receipt.json') -Raw | ConvertFrom-Json
        $upgradeMutationExpected = if([version]$taskVersion -ge [version]'2.23.2.25'){9}elseif([version]$taskVersion -ge [version]'2.23.2.24'){6}else{5}
        if(!$upgradeMutation.passed -or !$upgradeMutation.productionUnchanged -or $upgradeMutation.cases.Count -ne $upgradeMutationExpected -or @($upgradeMutation.cases | Where-Object {!$_.passed}).Count -ne 0 -or $upgradeMutation.sourceSha256 -ne (Get-FileHash -LiteralPath (Join-Path $taskCrate 'src/sourcegen/android_ai_workflow.rs') -Algorithm SHA256).Hash.ToLowerInvariant()){throw 'Agent save authorization/review mutation receipt is missing or stale'}
        $registrySource = Get-Content -LiteralPath (Join-Path $taskCrate 'src/android_agent_registry.rs') -Raw
        $registryTestNames = @([regex]::Matches($registrySource,'(?m)#\[test\]\s*fn\s+(\w+)\(') | ForEach-Object {$_.Groups[1].Value})
        if($registryTestNames.Count -lt 1){throw 'Agent cancellation registry tests are missing'}
        foreach($name in $registryTestNames){
            if($upgradeNativeLog -notmatch ('(?m)^test android_agent_registry::tests::'+[regex]::Escape($name)+' \.\.\. ok\s*$')){throw ('Agent cancellation registry test did not execute: '+$name)}
        }
        Write-TaskJson 'agent_review_acceptance.json' ([ordered]@{passed=$true;nativeTests=$upgradeTestNames.Count;nativeTestNames=$upgradeTestNames;registryTests=$registryTestNames.Count;registryTestNames=$registryTestNames;generatedSaveGates=2;generatedHelperMatchesTestedSource=$true;mutationCases=$upgradeMutationExpected;hostOnly=$true;deviceVerified=$false;realProviderVerified=$false})
    }
    if([version]$taskVersion -ge [version]'2.23.2.25'){
        $progressSource = Get-Content -LiteralPath (Join-Path $taskCrate 'src/android_agent_progress.rs') -Raw
        $progressNames = @([regex]::Matches($progressSource,'(?m)#\[test\]\s*fn\s+(\w+)\(') | ForEach-Object {$_.Groups[1].Value})
        if($progressNames.Count -lt 1){throw 'Agent progress domain tests are missing'}
        foreach($name in $progressNames){
            if($upgradeNativeLog -notmatch ('(?m)^test android_agent_progress::tests::'+[regex]::Escape($name)+' \.\.\. ok\s*$')){throw ('Agent progress test did not execute: '+$name)}
        }
        $bridge = Get-Content -LiteralPath (Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/core/NativeOptimizerBridge.kt') -Raw
        if(!$bridge.Contains('nativeAndroidKnowledgeAgentProgress(') -or !$upgradeStudio.Contains('androidKnowledgeAgentProgress(')) {throw 'Agent progress JNI polling is not connected to the generated dialog'}
        foreach($path in @('native/gridtimer_native/src/android_agent_progress.rs','native/gridtimer_native/src/sourcegen/android_agent_progress_ui.rs')){
            if(@($before.files | Where-Object path -eq $path).Count -ne 1){throw ('Agent progress source was not frozen: '+$path)}
        }
        Write-TaskJson 'agent_progress_acceptance.json' ([ordered]@{passed=$true;nativeTestNames=$progressNames;generatedPollingConnected=$true;sourceSnapshotSha256=$before.sha256;hostOnly=$true;deviceVerified=$false;realProviderVerified=$false})
    }
    if([version]$taskVersion -ge [version]'2.23.2.24'){
        [xml]$documentAiResult = Get-Content -LiteralPath (Join-Path $taskRoot 'app/build/test-results/testReleaseUnitTest/TEST-com.ofairyo.gridtimer.ui.KnowledgeAiRequestBoundaryTest.xml') -Raw
        $documentAiTests=@('documentEntryStartsKnowledgeAndOrdinaryEntryStartsDirect','wholeDocumentUsesFreshReadableTargetWithoutOtherSources','unreadableWholeDocumentCannotSendEvenWhenOtherPagesExist','failedSourceProjectionIsIsolatedAndTargetFailureCannotSend')
        foreach($name in $documentAiTests){
            if(@($documentAiResult.testsuite.testcase | Where-Object name -eq $name).Count -ne 1){throw ('Document AI business test did not execute: '+$name)}
        }
        Write-TaskJson 'whole_document_ai_acceptance.json' ([ordered]@{passed=$true;tests=$documentAiTests;readabilityMutationPassed=$true;hostOnly=$true;deviceVerified=$false;crashStackAvailable=$false})
    }
    $taskLegalWorkflowIntegration = $null
    if([version]$taskVersion -ge [version]'2.23.2.9'){
        $workflowSourcePath='native/gridtimer_native/src/sourcegen/android_legal_workflow.rs'
        $workflowVerifierPath='tools/verify_legal_workflow_mutation.ps1'
        foreach($inputPath in @($workflowSourcePath,$workflowVerifierPath)){
            if(@($before.files | Where-Object path -eq $inputPath).Count -ne 1){throw ('Legal workflow input was not frozen: '+$inputPath)}
        }
        $generator=Get-Content -LiteralPath (Join-Path $taskCrate 'src\bin\gridtimer_sourcegen.rs') -Raw
        if(!$generator.Contains('android_legal_workflow::render(relative_path, contents)')){throw 'Final legal workflow transform is not connected to Android source generation'}
        $gridRelative='app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/GridTimerScreen.kt'
        $legalRelative='app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/LegalRiskScreen.kt'
        $gridGenerated=Get-Content -LiteralPath (Join-Path $taskRoot $gridRelative) -Raw
        $legalGenerated=Get-Content -LiteralPath (Join-Path $taskRoot $legalRelative) -Raw
        if(!$gridGenerated.Contains('testTag("legal_open_analysis")') -or !$gridGenerated.Contains('usePlatformDefaultWidth = false') -or !$gridGenerated.Contains('LegalRiskScreen(')){throw 'Generated finance entry does not open the isolated legal workflow'}
        foreach($tag in @('legal_primary_action','legal_confirm_send')){
            if(!$legalGenerated.Contains('testTag("'+$tag+'")')){throw ('Generated legal workflow action is missing: '+$tag)}
        }
        if(!$legalGenerated.Contains('LegalSendConsent') -or !$legalGenerated.Contains('.beginRequest(') -or ([regex]::Matches($legalGenerated,'NativeOptimizerBridge\.runLegalScan\(')).Count -ne 1){throw 'Generated legal model request is not connected to the confirmation boundary'}
        $taskLegalWorkflowIntegration=[ordered]@{passed=$true;finalTransformConnected=$true;isolatedEntryPresent=$true;primaryActionPresent=$true;explicitConfirmationPresent=$true;sendBoundaryPresent=$true;modelRequestSites=1;businessTests=$taskAiWorkflowCounts.LegalAnalysisWorkflowTest;hostOnly=$true;deviceVerified=$false;generatedFiles=@($gridRelative,$legalRelative | ForEach-Object {[ordered]@{path=$_;sha256=(Get-FileHash -LiteralPath (Join-Path $taskRoot $_)).Hash.ToLowerInvariant()}})}
        Write-TaskJson 'legal_workflow_generated_acceptance.json' $taskLegalWorkflowIntegration
    }
    $taskDocumentIntegration = $null
    if([version]$taskVersion -ge [version]'2.23.2.10'){
        $generatedRoot = Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui'
        $editor = Get-Content -LiteralPath (Join-Path $generatedRoot 'NoteDocumentEditor.kt') -Raw
        $header = Get-Content -LiteralPath (Join-Path $generatedRoot 'NoteStudioSheet.kt') -Raw
        $policy = Get-Content -LiteralPath (Join-Path $generatedRoot 'DocumentCaretPolicy.kt') -Raw
        $visibility = Get-Content -LiteralPath (Join-Path $generatedRoot 'DocumentCaretVisibility.kt') -Raw
        if(!$editor.Contains('state = editorListState') -or !$editor.Contains('onTextLayout = { caretVisibility.layout = it }') -or !$editor.Contains('caretVisibility.focused = focusState.isFocused') -or !$editor.Contains('bringIntoViewRequester(caretVisibility.requester)') -or !$visibility.Contains('documentCaretOffset(') -or !$visibility.Contains('documentCaretRevealBounds(')){throw 'Caret policy is not connected to the generated editor and real layout callbacks'}
        if(!$editor.Contains('remember(focusWorkspaceKey, note.id, block.id)') -or !$editor.Contains('LaunchedEffect(focusWorkspaceKey, note.id, block.id, requestFocus)')){throw 'Deferred document focus must be cancelled when workspace or note changes'}
        if(([regex]::Matches($header,'testTag\("knowledge_compact_header"\)')).Count -ne 1 -or $header.Contains('FlowusWorkspaceHeader(')){throw 'Actual knowledge header transformation is missing or duplicated'}
        foreach($tag in @('knowledge_header_create','knowledge_header_trash','knowledge_header_folders')){if(!$header.Contains('testTag("'+$tag+'")')){throw 'A generated knowledge header action was lost'}}
        $taskDocumentIntegration=[ordered]@{passed=$true;policyConnectedToRealEditor=$true;workspaceAndNoteFocusLifetime=$true;compactHeaderPresent=$true;originalHeaderActionsPresent=$true;businessTests=$taskAiWorkflowCounts.DocumentCaretPolicyTest;hostOnly=$true;deviceVerified=$false;generatedFiles=@('NoteDocumentEditor.kt','NoteStudioSheet.kt','DocumentCaretPolicy.kt','DocumentCaretVisibility.kt' | ForEach-Object {[ordered]@{path=('app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/'+$_);sha256=(Get-FileHash -LiteralPath (Join-Path $generatedRoot $_)).Hash.ToLowerInvariant()}})}
        Write-TaskJson 'document_generated_acceptance.json' $taskDocumentIntegration
    }
    $taskFinanceIntegration = $null
    if([version]$taskVersion -ge [version]'2.23.2.11'){
        $financeSourcePath='native/gridtimer_native/src/sourcegen/android_finance_workspace.rs'
        $financeVerifierPath='tools/verify_finance_workspace_mutation.ps1'
        foreach($inputPath in @($financeSourcePath,$financeVerifierPath)){
            if(@($before.files | Where-Object path -eq $inputPath).Count -ne 1){throw ('Finance workspace input was not frozen: '+$inputPath)}
        }
        $financeSource=Get-Content -LiteralPath (Join-Path $taskRoot $financeSourcePath) -Raw
        $financePolicyLiteral=[regex]::Match($financeSource,'(?s)pub const POLICY:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
        $financeTestLiteral=[regex]::Match($financeSource,'(?s)pub const TEST_CONTENTS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
        if(!$financePolicyLiteral.Success -or !$financeTestLiteral.Success){throw 'Rust-owned finance policy or tests are missing'}
        $financeTestNames=@([regex]::Matches($financeTestLiteral.Groups['body'].Value,'@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object {$_.Groups[1].Value})
        $financeRequiredTests=@('newMonthClosesOldDetailAndReturnsToOverview','newWorkspaceDoesNotInheritFinancialDetail','switchingTaskClearsThePreviousDialog','reviewCountUsesAllFourExplicitFlags','alertPreviewIsBoundedWithoutChangingTheTotal')
        if($financeTestNames.Count -ne 5 -or @($financeTestNames | Sort-Object -Unique).Count -ne 5 -or @($financeRequiredTests | Where-Object {$_ -notin $financeTestNames}).Count){throw 'Finance workspace suite must contain the five distinct state and boundary tests'}
        $financeJunitPath=Join-Path $taskRoot 'app/build/test-results/testReleaseUnitTest/TEST-com.ofairyo.gridtimer.ui.FinanceWorkspacePolicyTest.xml'
        [xml]$financeJunit=Get-Content -LiteralPath $financeJunitPath -Raw
        if([int]$financeJunit.testsuite.tests -ne 5 -or [int]$financeJunit.testsuite.failures -ne 0 -or [int]$financeJunit.testsuite.errors -ne 0 -or [int]$financeJunit.testsuite.skipped -ne 0){throw 'Formal finance workspace state tests are incomplete or failed'}
        foreach($testName in $financeRequiredTests){if(@($financeJunit.testsuite.testcase | Where-Object name -eq $testName).Count -ne 1){throw ('Formal finance workspace test is absent: '+$testName)}}
        Copy-Item -LiteralPath $financeJunitPath -Destination (Join-Path $taskEvidence 'FinanceWorkspacePolicyTest.xml') -Force
        $financeGenerator=Get-Content -LiteralPath (Join-Path $taskCrate 'src/bin/gridtimer_sourcegen.rs') -Raw
        if(!$financeGenerator.Contains('android_finance_workspace::render(relative_path, &contents)') -or !$financeGenerator.Contains('android_finance_workspace::TEST_PATH') -or !$financeGenerator.Contains('android_finance_workspace::TEST_CONTENTS')){throw 'Finance transform or domain test source is not connected to final Android generation'}
        $financePanelPath='app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/FinanceRiskV2Panel.kt'
        $financeGridPath='app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/GridTimerScreen.kt'
        $financePanel=(Get-Content -LiteralPath (Join-Path $taskRoot $financePanelPath) -Raw).Replace("`r`n","`n")
        $financeGrid=Get-Content -LiteralPath (Join-Path $taskRoot $financeGridPath) -Raw
        if(!$financePanel.Contains($financePolicyLiteral.Groups['body'].Value.Replace("`r`n","`n")) -or !$financePanel.Contains('remember(workspaceKey, monthKey)') -or !$financePanel.Contains('savedRoute.forScope(workspaceKey, monthKey)') -or !$financePanel.Contains('FinanceWorkspaceDetailDialog(openedDetail.title')){throw 'The tested finance policy or scope lifetime is not connected to the generated panel'}
        foreach($detail in @('FACTS','FORECAST','VERIFY','RECURRING','ALERTS')){if(!$financePanel.Contains('FinanceWorkspaceDetail.'+$detail+' -> {')){throw ('Generated finance detail is missing: '+$detail)}}
        if(!$financeGrid.Contains('testTag("legal_open_analysis")') -or !$financeGrid.Contains('onClick = { showLegalRisk = true }') -or !$financeGrid.Contains('LegalRiskScreen(')){throw 'Finance navigation lost the actual legal workflow entry'}
        $taskFinanceIntegration=[ordered]@{passed=$true;finalTransformConnected=$true;policyConnectedToRealPanel=$true;workspaceAndMonthLifetime=$true;allDetailSectionsPresent=$true;legalWorkflowEntryPreserved=$true;businessTests=5;testNames=$financeTestNames;hostOnly=$true;deviceVerified=$false;generatedFiles=@($financePanelPath,$financeGridPath | ForEach-Object {[ordered]@{path=$_;sha256=(Get-FileHash -LiteralPath (Join-Path $taskRoot $_)).Hash.ToLowerInvariant()}})}
        Write-TaskJson 'finance_workspace_generated_acceptance.json' $taskFinanceIntegration
    }
    $taskMarkdownIntegration = $null
    if([version]$taskVersion -ge [version]'2.23.2.12'){
        $markdownSourcePath='native/gridtimer_native/src/sourcegen/android_document_markdown.rs'
        $markdownTemplatePath='native/gridtimer_native/src/sourcegen/kotlin_sources.rs'
        $markdownVerifierPath='tools/verify_document_markdown_mutation.ps1'
        foreach($inputPath in @($markdownSourcePath,$markdownTemplatePath,$markdownVerifierPath)){
            if(@($before.files | Where-Object path -eq $inputPath).Count -ne 1){throw ('Document Markdown input was not frozen: '+$inputPath)}
        }
        Invoke-TaskCommand (Get-Command pwsh -ErrorAction Stop).Source @('-NoProfile','-File',(Join-Path $taskRoot $markdownVerifierPath),'-Version',$taskVersion) 'document_markdown_mutation.log'
        $markdownReceipt=Get-Content -LiteralPath (Join-Path $taskEvidence 'document_markdown_mutation/receipt.json') -Raw | ConvertFrom-Json
        if(!$markdownReceipt.passed -or !$markdownReceipt.productionUnchanged -or $markdownReceipt.version -ne $taskVersion -or $markdownReceipt.sourceSnapshotSha256 -cne $before.sha256 -or $markdownReceipt.tests -ne $taskAiWorkflowCounts.DocumentMarkdownBlocksTest -or $markdownReceipt.cases.Count -ne 5){throw 'Document Markdown mutation is not bound to the actual formal build'}
        foreach($identity in @(
            @{path=$markdownSourcePath;before=$markdownReceipt.sourceSha256Before;after=$markdownReceipt.sourceSha256After},
            @{path=$markdownTemplatePath;before=$markdownReceipt.templateSha256Before;after=$markdownReceipt.templateSha256After},
            @{path=$markdownVerifierPath;before=$markdownReceipt.verifierSha256;after=$markdownReceipt.verifierSha256}
        )){
            $frozen=@($before.files | Where-Object path -eq $identity.path)
            if($frozen.Count -ne 1 -or $frozen[0].sha256 -cne $identity.before -or $frozen[0].sha256 -cne $identity.after -or (Get-FileHash -LiteralPath (Join-Path $taskRoot $identity.path)).Hash.ToLowerInvariant() -cne $identity.after){throw ('Document Markdown mutation source identity changed: '+$identity.path)}
        }
        $markdownCaseKinds=@{baseline='';cosmetic='';removed_paragraph_separator='independentParagraphsHaveBlankLineBoundaries';removed_fence_continuation='splitBacktickFencePreservesCodeLinesAndBlankLines';restored=''}
        foreach($caseName in $markdownCaseKinds.Keys){
            $case=@($markdownReceipt.cases | Where-Object name -eq $caseName)
            if($case.Count -ne 1 -or !$case[0].passed -or $case[0].compileExit -ne 0 -or !$case[0].allBusinessTestsObserved -or $null -eq $case[0].testExit -or $case[0].testsSha256 -cne $markdownReceipt.testsSha256){throw ('Document Markdown mutation case is incomplete: '+$caseName)}
            $compilePath=Join-Path $taskEvidence ('document_markdown_mutation/'+$caseName+'/compile.log')
            $testPath=Join-Path $taskEvidence ('document_markdown_mutation/'+$caseName+'/tests.log')
            if((Get-FileHash -LiteralPath $compilePath).Hash.ToLowerInvariant() -cne $case[0].compileLogSha256 -or (Get-FileHash -LiteralPath $testPath).Hash.ToLowerInvariant() -cne $case[0].testLogSha256){throw ('Document Markdown mutation logs changed: '+$caseName)}
            if($markdownCaseKinds[$caseName]){
                if($case[0].testExit -eq 0 -or !$case[0].requiredFailureObserved -or $case[0].requiredFailureTest -ne $markdownCaseKinds[$caseName]){throw ('Document Markdown state guard removal was not detected: '+$caseName)}
            }elseif($case[0].testExit -ne 0 -or $case[0].helperSha256 -cne $markdownReceipt.helperSha256 -and $caseName -ne 'cosmetic'){
                throw ('Document Markdown baseline or cosmetic verification failed: '+$caseName)
            }
        }
        $markdownSource=Get-Content -LiteralPath (Join-Path $taskRoot $markdownSourcePath) -Raw
        $markdownHelperLiteral=[regex]::Match($markdownSource,'(?s)(?:pub\s+)?const HELPERS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
        $markdownTestsLiteral=[regex]::Match($markdownSource,'(?s)pub const TEST_CONTENTS:\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
        if(!$markdownHelperLiteral.Success -or !$markdownTestsLiteral.Success){throw 'Rust-owned Markdown projection or tests are absent'}
        $markdownHash={param([string]$value) [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($value.Replace("`r`n","`n")))).ToLowerInvariant()}
        if((& $markdownHash $markdownHelperLiteral.Groups['body'].Value) -cne $markdownReceipt.helperLiteralSha256 -or (& $markdownHash $markdownTestsLiteral.Groups['body'].Value) -cne $markdownReceipt.testsSha256){throw 'Markdown mutation used a different Rust-owned helper or test suite'}
        $markdownEditorPath='app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/NoteDocumentEditor.kt'
        $markdownTestsPath='app/build/generated/source/rustAndroid/test/com/ofairyo/gridtimer/ui/DocumentMarkdownBlocksTest.kt'
        $markdownEditor=(Get-Content -LiteralPath (Join-Path $taskRoot $markdownEditorPath) -Raw).Replace("`r`n","`n")
        $previewMarker='internal fun documentMarkdownPreviewText('
        $previewStart=$markdownEditor.IndexOf($previewMarker,[StringComparison]::Ordinal)
        $previewEnd=if($previewStart -ge 0){$markdownEditor.IndexOf("`n}`n",$previewStart,[StringComparison]::Ordinal)}else{-1}
        if($previewEnd -lt $previewStart -or $previewStart -lt 0 -or !$markdownEditor.Contains($markdownHelperLiteral.Groups['body'].Value.Replace("`r`n","`n"))){throw 'The executed Markdown helper is not present in the generated document editor'}
        $generatedPreview=$markdownEditor.Substring($previewStart,$previewEnd-$previewStart+2)
        if((& $markdownHash $generatedPreview) -cne $markdownReceipt.previewSha256 -or (& $markdownHash (Get-Content -LiteralPath (Join-Path $taskRoot $markdownTestsPath) -Raw)) -cne $markdownReceipt.testsSha256){throw 'The generated document projection or suite differs from the executed mutation'}
        $taskMarkdownIntegration=[ordered]@{passed=$true;businessTests=$taskAiWorkflowCounts.DocumentMarkdownBlocksTest;mutationPassed=$true;sourceSnapshotSha256=$before.sha256;helperLiteralSha256=$markdownReceipt.helperLiteralSha256;previewSha256=$markdownReceipt.previewSha256;testsSha256=$markdownReceipt.testsSha256;receiptSha256=(Get-FileHash -LiteralPath (Join-Path $taskEvidence 'document_markdown_mutation/receipt.json')).Hash.ToLowerInvariant();hostOnly=$true;deviceVerified=$false;generatedFiles=@($markdownEditorPath,$markdownTestsPath | ForEach-Object {[ordered]@{path=$_;sha256=(Get-FileHash -LiteralPath (Join-Path $taskRoot $_)).Hash.ToLowerInvariant()}})}
        Write-TaskJson 'document_markdown_generated_acceptance.json' $taskMarkdownIntegration
    }
    Copy-Item -LiteralPath (Join-Path $taskRoot 'app\build\reports\lint-results-release.xml') -Destination (Join-Path $taskEvidence 'lint-results-release.xml') -Force
    $apk = Join-Path $taskRoot "app\build\outputs\apk\release\tenfold_v$taskVersion.apk"

    if([version]$taskVersion -ge [version]'2.23.2.15'){
        $navigationSource=[IO.File]::ReadAllText((Join-Path $taskCrate 'src/sourcegen/android_knowledge_navigation.rs'))
        $navigationCounts=[ordered]@{}
        foreach($suite in @(@('TEST_CONTENTS','KnowledgeNavigationIndexTest'),@('SNAPSHOT_TEST_CONTENTS','KnowledgeNavigationSnapshotTest'))){
            $literal=[regex]::Match($navigationSource,'(?s)pub const '+$suite[0]+':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
            if(!$literal.Success){throw 'Navigation JVM source literal missing'}
            $expected=([regex]::Matches($literal.Groups['body'].Value,'@Test fun ')).Count
            $xmlPath=Join-Path $taskRoot ('app/build/test-results/testReleaseUnitTest/TEST-com.ofairyo.gridtimer.ui.'+$suite[1]+'.xml')
            [xml]$navigationJunit=Get-Content -LiteralPath $xmlPath -Raw
            if($expected -lt 1 -or [int]$navigationJunit.testsuite.tests -ne $expected -or [int]$navigationJunit.testsuite.failures -ne 0 -or [int]$navigationJunit.testsuite.errors -ne 0 -or [int]$navigationJunit.testsuite.skipped -ne 0){throw ('Navigation JVM gate failed: '+$suite[1])}
            Copy-Item -LiteralPath $xmlPath -Destination (Join-Path $taskEvidence ($suite[1]+'.xml')) -Force
            $navigationCounts[$suite[1]]=$expected
        }
        $navigationGenerated=[ordered]@{}
        foreach($literalName in @('INDEX_CONTENTS','UI_CONTENTS')){
            $literal=[regex]::Match($navigationSource,'(?s)pub const '+$literalName+':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
            $fileName=if($literalName -eq 'INDEX_CONTENTS'){'KnowledgeNavigationIndex.kt'}else{'KnowledgeNavigation.kt'}
            $path=Join-Path $taskRoot ('app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/'+$fileName)
            if(!$literal.Success -or [IO.File]::ReadAllText($path).Replace("`r`n","`n").Trim() -cne $literal.Groups['body'].Value.Replace("`r`n","`n").Trim()){throw ('Actual generated navigation differs from frozen Rust source: '+$fileName)}
            $navigationGenerated[$fileName]=(Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant()
        }
        $studio=[IO.File]::ReadAllText((Join-Path $taskRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui/NoteStudioSheet.kt'))
        if(([regex]::Matches($studio,'testTag\("knowledge_open_navigation"\)')).Count -ne 1 -or ([regex]::Matches($studio,'KnowledgeNavigationDialog\(')).Count -ne 1 -or !$studio.Contains('onOpenNote(note)')){throw 'Actual collection navigation route missing or duplicated'}
        if($taskSourcegenOutput -notmatch '(?m)^test android_knowledge_navigation::tests::missing_duplicate_and_reapplied_hooks_fail_closed \.\.\. ok\s*$'){
            $navRustLog=[IO.File]::ReadAllText((Join-Path $taskEvidence 'sourcegen_tests.log'))
            if($navRustLog -notmatch '(?m)^test android_knowledge_navigation::tests::missing_duplicate_and_reapplied_hooks_fail_closed \.\.\. ok\s*$'){throw 'Navigation generator failure boundary did not execute'}
        }
        Write-TaskJson 'knowledge_navigation_generated_acceptance.json' ([ordered]@{passed=$true;sourceSnapshotSha256=$before.sha256;jvmTests=$navigationCounts;generatedFiles=$navigationGenerated;uniqueCollectionEntry=$true;existingNoteRoute=$true;deviceVerified=$false})
    }

    $taskStructuredIntegration = $null
    if([version]$taskVersion -ge [version]'2.23.2.16'){
        $structuredSources = @{}
        foreach($name in @('android_knowledge_compat.rs','structured_mobile_data.rs','structured_mobile_tests.rs')){
            $relative='native/gridtimer_native/src/sourcegen/'+$name
            if(@($before.files | Where-Object path -eq $relative).Count -ne 1){throw ('Structured page source was not frozen: '+$relative)}
            $structuredSources[$name]=[IO.File]::ReadAllText((Join-Path $taskRoot $relative))
        }
        $structuredLiteral = {
            param([string]$owner,[string]$name)
            $match=[regex]::Match($structuredSources[$owner],'(?s)(?:pub\s+)?const '+[regex]::Escape($name)+':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
            if(!$match.Success){throw ('Structured page literal missing: '+$owner+'/'+$name)}
            $match.Groups['body'].Value.Replace("`r`n","`n")
        }
        $structuredGenerated = [ordered]@{}
        $structuredCounts = [ordered]@{}
        foreach($entry in @(
            @{owner='structured_mobile_data.rs';literal='POLICY_CONTENTS';kind='main';name='StructuredEditPolicy'},
            @{owner='structured_mobile_data.rs';literal='EDIT_CONTENTS';kind='main';name='StructuredNoteEditing'},
            @{owner='structured_mobile_tests.rs';literal='TEST_CONTENTS';kind='test';name='StructuredEditPolicyTest'},
            @{owner='structured_mobile_tests.rs';literal='EDIT_TEST_CONTENTS';kind='test';name='StructuredNoteEditingTest'}
        )){
            $literal=& $structuredLiteral $entry.owner $entry.literal
            $relative='app/build/generated/source/rustAndroid/'+$entry.kind+'/com/ofairyo/gridtimer/data/'+$entry.name+'.kt'
            $generated=[IO.File]::ReadAllText((Join-Path $taskRoot $relative)).Replace("`r`n","`n")
            if($generated.Trim() -cne $literal.Trim()){throw ('Generated structured model or tests differ from frozen Rust source: '+$entry.name)}
            $structuredGenerated[$relative]=(Get-FileHash -LiteralPath (Join-Path $taskRoot $relative)).Hash.ToLowerInvariant()
            if($entry.kind -eq 'test'){
                $names=@([regex]::Matches($literal,'@Test\s+fun\s+(\w+)\s*\(') | ForEach-Object {$_.Groups[1].Value})
                $xmlPath=Join-Path $taskRoot ('app/build/test-results/testReleaseUnitTest/TEST-com.ofairyo.gridtimer.data.'+$entry.name+'.xml')
                [xml]$suite=Get-Content -LiteralPath $xmlPath -Raw
                if($names.Count -lt 1 -or @($names | Sort-Object -Unique).Count -ne $names.Count -or [int]$suite.testsuite.tests -ne $names.Count -or [int]$suite.testsuite.failures -ne 0 -or [int]$suite.testsuite.errors -ne 0 -or [int]$suite.testsuite.skipped -ne 0){throw ('Structured page JVM suite incomplete or failed: '+$entry.name)}
                foreach($name in $names){if(@($suite.testsuite.testcase | Where-Object name -eq $name).Count -ne 1){throw ('Structured page business test not executed: '+$entry.name+'/'+$name)}}
                Copy-Item -LiteralPath $xmlPath -Destination (Join-Path $taskEvidence ($entry.name+'.xml')) -Force
                $structuredCounts[$entry.name]=$names.Count
            }
        }
        $structuredRoutes = @{}
        foreach($relative in @('data/TimerRepository.kt','ui/TimerViewModel.kt','ui/NoteDocumentEditor.kt','ui/SmartisanNoteUi.kt','ui/NoteStudioSheet.kt','ui/StructuredKnowledgeReader.kt')){
            $path='app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/'+$relative
            $structuredRoutes[$relative]=[IO.File]::ReadAllText((Join-Path $taskRoot $path)).Replace("`r`n","`n")
            $structuredGenerated[$path]=(Get-FileHash -LiteralPath (Join-Path $taskRoot $path)).Hash.ToLowerInvariant()
        }
        if(([regex]::Matches($structuredRoutes['data/TimerRepository.kt'],'internal suspend fun applyStructuredNotePatch\(')).Count -ne 1 -or !$structuredRoutes['data/TimerRepository.kt'].Contains((& $structuredLiteral 'android_knowledge_compat.rs' 'REPOSITORY_WRITE')) -or ([regex]::Matches($structuredRoutes['ui/TimerViewModel.kt'],'internal fun applyStructuredNotePatch\(')).Count -ne 1 -or !$structuredRoutes['ui/TimerViewModel.kt'].Contains((& $structuredLiteral 'android_knowledge_compat.rs' 'VIEW_MODEL_WRITE'))){throw 'Structured edits are not connected once to the durable repository and existing save queue'}
        if(([regex]::Matches($structuredRoutes['ui/NoteDocumentEditor.kt'],'viewModel\.applyStructuredNotePatch\(patch, structuredWorkspace, done\)')).Count -ne 1 -or ([regex]::Matches($structuredRoutes['ui/SmartisanNoteUi.kt'],'StructuredKnowledgeReader\(')).Count -ne 1 -or !$structuredRoutes['ui/StructuredKnowledgeReader.kt'].Contains('writer(request)') -or ([regex]::Matches($structuredRoutes['ui/NoteStudioSheet.kt'],'NoteDraftPreset\.STRUCTURED -> return com\.ofairyo\.gridtimer\.data\.buildStructuredMobileNote\(folderId, now\)')).Count -ne 1){throw 'Structured editor, protected reader or document preset route is missing or duplicated'}
        $structuredRustLog=[IO.File]::ReadAllText((Join-Path $taskEvidence 'sourcegen_tests.log'))
        if($structuredRustLog -notmatch '(?m)^test android_knowledge_compat::tests::real_structured_hooks_reject_missing_duplicate_and_reapplied_input \.\.\. ok\s*$'){throw 'Structured generator failure boundary did not execute'}
        $taskStructuredIntegration=[ordered]@{passed=$true;sourceSnapshotSha256=$before.sha256;jvmTests=$structuredCounts;actualGeneratedModelAndTests=$true;durableRepositoryAndSaveQueueConnected=$true;protectedReaderAndPresetConnected=$true;generatedFiles=$structuredGenerated;hostOnly=$true;deviceVerified=$false}
        Write-TaskJson 'structured_page_generated_acceptance.json' $taskStructuredIntegration
    }

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
    if($taskLegalWorkflowIntegration){$result['legalWorkflowIntegration']=$taskLegalWorkflowIntegration}
    if($taskDocumentIntegration){$result['documentIntegration']=$taskDocumentIntegration}
    if($taskFinanceIntegration){$result['financeWorkspaceTests']=5;$result['financeWorkspaceIntegration']=$taskFinanceIntegration}
    if($taskMarkdownIntegration){$result['documentMarkdownTests']=$taskMarkdownIntegration.businessTests;$result['documentMarkdownIntegration']=$taskMarkdownIntegration}
    if($taskStructuredIntegration){$result['structuredPageIntegration']=$taskStructuredIntegration}
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
