# v0.0.1 - Verify retained document exit completion and persistence deduplication.
param([string]$Version)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$Version) { $Version = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'app/build.gradle')), "versionName '([^']+)'").Groups[1].Value }
if ($Version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Invalid Android version' }
$evidence = Join-Path $root "release_artifacts/verification/v$Version/note_exit_save_mutation"
$emitter = Join-Path $root 'native/gridtimer_native/src/sourcegen/android_note_save_queue.rs'
$uiPath = Join-Path $root 'native/gridtimer_native/src/sourcegen/kotlin_sources.rs'
$emitterHash = (Get-FileHash -LiteralPath $emitter).Hash
$uiHash = (Get-FileHash -LiteralPath $uiPath).Hash
$raw = [IO.File]::ReadAllText($emitter)
function Get-Literal([string]$Name) {
    $match = [regex]::Match($raw, '(?s)pub const '+$Name+':\s*&str\s*=\s*r(?<hash>#+)"(?<body>.*?)"\k<hash>;')
    if (!$match.Success) { throw "Missing production literal: $Name" }
    return $match.Groups['body'].Value.Replace("`r`n", "`n")
}
$helper = Get-Literal 'EXIT_SAVE_GATE_CONTENTS'
$tests = Get-Literal 'EXIT_SAVE_GATE_TEST_CONTENTS'
$testCount = [regex]::Matches($tests, '@Test\s+fun ').Count
if ($testCount -lt 20) { throw 'Exit save boundary tests missing' }
$cache = Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1'
function Get-ExitSaveDependency([string]$Coordinate, [string]$DependencyVersion, [string]$Name) {
    $directory = Join-Path $cache ($Coordinate + '/' + $DependencyVersion)
    $files = @(Get-ChildItem -LiteralPath $directory -File -Recurse | Where-Object Name -eq $Name)
    if ($files.Count -ne 1) { throw "Installed navigation verification dependency is missing or ambiguous: $Name" }
    $files[0].FullName
}
$compiler = @(
    (Get-ExitSaveDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-ExitSaveDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-ExitSaveDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-ExitSaveDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-ExitSaveDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-ExitSaveDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-ExitSaveDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$runtime = @(
    (Get-ExitSaveDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-ExitSaveDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-ExitSaveDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-ExitSaveDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
$java = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$utf8 = [Text.UTF8Encoding]::new($false)
$results = @()
$generated = Join-Path $root 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer/ui'
$actualHelper = Join-Path $generated 'NoteEditorExitSaveGate.kt'
$actualEditor = Join-Path $generated 'NoteDocumentEditor.kt'
if (![IO.File]::Exists($actualHelper) -or ![IO.File]::Exists($actualEditor)) { throw 'Generate production Android sources before running this gate' }
if ([IO.File]::ReadAllText($actualHelper).Replace("`r`n", "`n").Trim() -cne $helper.Trim()) { throw 'Generated production exit helper does not match its Rust source' }
$editor = [IO.File]::ReadAllText($actualEditor)
foreach ($binding in @(
    'val exitSaveGate = NoteEditorExitSaveGate<NoteEntry>()',
    'val persistenceTicket = editorSession.exitSaveGate.beginPersistence(',
    'val completion = editorSession.exitSaveGate.completePersistence(',
    'completion.accepted && completion.latest && persistenceGeneration == saveRequestGeneration',
    'completion.deliver()',
    'val exitTicket = editorSession.exitSaveGate.begin() ?: return',
    'val completion = editorSession.exitSaveGate.complete(',
    'if (!completion.notifyUi) return@exitCompletion',
    'val alreadyDurable = editorSession.exitSaveGate.canSkipDurableExit(',
    'cachedDraft = committed',
    'visibleDraft = latestNote',
    'candidate = candidate',
    'val exitAction = if (requestedExitAction == SmartisanNoteExitAction.Noop)',
    'queueEpoch = viewModel.noteMutationEpoch()',
    'editorSession.exitSaveGate.markEnqueued(persistenceTicket, viewModel.noteMutationEpoch())'
)) { if (!$editor.Contains($binding)) { throw "Production editor is not using the domain gate: $binding" } }
$backStart = $editor.IndexOf('    fun handleBack()')
$backEnd = $editor.IndexOf('    fun undoDraft()', $backStart)
if ($backStart -lt 0 -or $backEnd -le $backStart) { throw 'Production document back handler missing' }
if ($editor.Substring($backStart, $backEnd - $backStart).Contains('saveRequestGeneration += 1L')) { throw 'Joined back save invalidates the existing persistence generation' }
$persistStart = $editor.IndexOf('    fun persistLatestDraft(')
$persistEnd = $editor.IndexOf('    val latestExitPersistence', $persistStart)
if ($persistStart -lt 0 -or $persistEnd -le $persistStart) { throw 'Production document persistence function missing' }
$persistBody = $editor.Substring($persistStart, $persistEnd - $persistStart)
if ($persistBody.Contains('if (!editorSaveCallbackActive.get()')) { throw 'Disposed editor still abandons retained persistence completion' }
foreach ($case in @('baseline','cosmetic_only','removed_completion_ownership','abandoned_disposed_completion','removed_workspace_delivery_guard','removed_blank_deletion_strength','removed_persistence_completion_ownership','removed_persistence_tail_order','removed_stronger_deletion_join','removed_queue_barrier_fence','removed_completed_cache_epoch','removed_complete_payload_equality')) {
    $directory = Join-Path $evidence $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $production = $helper
    if ($case -eq 'cosmetic_only') {
        $label = 'localizedToast(context, "'+(-join @([char]0x4fdd,[char]0x5b58,[char]0x5931,[char]0x8d25,[char]0xff0c,[char]0x7b14,[char]0x8bb0,[char]0x4ecd,[char]0x7559,[char]0x5728,[char]0x7f16,[char]0x8f91,[char]0x9875,[char]0x3002))+'", Toast.LENGTH_LONG)'
        if (!$editor.Contains($label)) { throw 'Actual editor failure wording missing' }
        $cosmeticEditor = $editor.Replace($label, 'localizedToast(context, "Save failed. The draft is still here.", Toast.LENGTH_LONG)')
        [IO.File]::WriteAllText((Join-Path $directory 'NoteDocumentEditor_wording.kt'), $cosmeticEditor, $utf8)
    }
    $mutations = @{
        removed_queue_barrier_fence = @('it.enqueuedEpoch == queueEpoch &&', '')
        removed_completed_cache_epoch = @('durableEpoch != null && durableEpoch == queueEpoch', 'durableEpoch != null')
        removed_complete_payload_equality = @('candidate == cachedDraft && candidate == visibleDraft &&', '')
        removed_completion_ownership = @('        if (current !== ticket) return Completion(false, false, false)', '')
        abandoned_disposed_completion = @('        current = null', "        if (!uiIsCurrent) return Completion(false, false, false)`n        current = null")
        removed_workspace_delivery_guard = @('notifyUi = uiIsCurrent && workspaceMatches', 'notifyUi = uiIsCurrent')
        removed_blank_deletion_strength = @('it.draft == draft && (it.requiresBlankDeletion || !requiresBlankDeletion)', 'it.draft == draft')
        removed_persistence_tail_order = @('val pending = latestPersistence?.takeIf {', 'val pending = pendingPersistence.firstOrNull {')
        removed_stronger_deletion_join = @('(it.requiresBlankDeletion || !requiresBlankDeletion)', 'it.requiresBlankDeletion == requiresBlankDeletion')
        removed_persistence_completion_ownership = @('if (!pendingPersistence.remove(ticket)) {', 'if (false) {')
    }
    if ($mutations.ContainsKey($case)) {
        $mutation = $mutations[$case]
        if (!$production.Contains($mutation[0])) { throw "Production state guard missing: $case" }
        $production = $production.Replace($mutation[0], $mutation[1])
    }
    $helperFile = Join-Path $directory 'NoteEditorExitSaveGate.kt'
    $testFile = Join-Path $directory 'NoteEditorExitSaveGateTest.kt'
    [IO.File]::WriteAllText($helperFile, $production, $utf8)
    [IO.File]::WriteAllText($testFile, $tests, $utf8)
    $classes = Join-Path $directory ('classes_'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $java -Xmx512m -cp ($compiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -classpath ($runtime -join ';') -d $classes $helperFile $testFile *> (Join-Path $directory 'compile.log')
    if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath (Join-Path $directory 'compile.log') -Tail 20; throw "Mutation compilation failed: $case" }
    & $java -Xmx256m -cp ($classes+';'+($runtime -join ';')) org.junit.runner.JUnitCore com.ofairyo.gridtimer.ui.NoteEditorExitSaveGateTest *> (Join-Path $directory 'tests.log')
    $exitCode = $LASTEXITCODE
    $log = [IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $expectedPass = $case -in @('baseline','cosmetic_only')
    $requiredFailure = switch ($case) {
        'removed_completion_ownership' { 'staleCompletionCannotReleaseNewerExit' }
        'abandoned_disposed_completion' { 'disposedEditorStillSettlesRetainedSaveWithoutNavigatingReplacement' }
        'removed_workspace_delivery_guard' { 'workspaceChangeSettlesOldBusyWithoutSuccessOrNavigation' }
        'removed_blank_deletion_strength' { 'blankDeletionCannotJoinWeakerPendingPersistence' }
        'removed_persistence_completion_ownership' { 'duplicatePersistenceCompletionCannotDeliverCallbacksAgain' }
        'removed_persistence_tail_order' { 'sameDraftCannotJoinAcrossAnotherAcceptedDraft' }
        'removed_stronger_deletion_join' { 'pendingBlankDeletionAlsoSatisfiesWeakerLifecycleSave' }
        'removed_queue_barrier_fence' { 'interveningQueueBarrierPreventsJoiningOlderLifecycleWrite' }
        'removed_completed_cache_epoch' { 'completedDurableProofExpiresWhenAnotherQueueMutationIsAccepted' }
        'removed_complete_payload_equality' { 'equalTextWithChangedNonTextPayloadCannotSkipDurableExit' }
        default { '' }
    }
    $passed = if ($expectedPass) { $exitCode -eq 0 -and $log.Contains("OK ($testCount tests)") } else { $exitCode -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains($requiredFailure) }
    $results += [ordered]@{case=$case;passed=$passed;expectedPass=$expectedPass;exitCode=$exitCode;tests=$testCount;helperSha256=(Get-FileHash -LiteralPath $helperFile).Hash.ToLowerInvariant();uiWordingChanged=($case -eq 'cosmetic_only')}
    if (!$passed) { Get-Content -LiteralPath (Join-Path $directory 'tests.log') -Tail 25; throw "Unexpected mutation result: $case" }
    Write-Output "$case verified"
}
if ((Get-FileHash -LiteralPath $emitter).Hash -cne $emitterHash -or (Get-FileHash -LiteralPath $uiPath).Hash -cne $uiHash) { throw 'Production changed during mutation verification' }
[ordered]@{passed=$true;version=$Version;tests=$testCount;productionUnchanged=$true;sourceSha256=$emitterHash.ToLowerInvariant();generatedHelperSha256=(Get-FileHash -LiteralPath $actualHelper).Hash.ToLowerInvariant();generatedEditorSha256=(Get-FileHash -LiteralPath $actualEditor).Hash.ToLowerInvariant();productionIntegrationVerified=$true;cases=$results;hostOnly=$true;deviceVerified=$false} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'receipt.json') -Encoding utf8
