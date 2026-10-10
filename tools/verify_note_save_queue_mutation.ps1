# v0.0.2 - Verify admission epochs without private historical classpaths.
# v0.0.1 - Run the actual generated queue and preserve save/identity barriers.
param([string]$Version = '2.23.2.1')
$ErrorActionPreference = 'Stop'
$noteQueueRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($Version -notmatch '^\d+\.\d+\.\d+(?:\.\d+)?$') { throw 'Invalid evidence version' }
$noteQueueEmitter = Join-Path $noteQueueRoot 'native/gridtimer_native/src/sourcegen/android_note_save_queue.rs'
$noteQueueEvidence = Join-Path $noteQueueRoot "release_artifacts/verification/v$Version/note_save_queue_mutation"
$noteQueueSourceHash = (Get-FileHash -LiteralPath $noteQueueEmitter).Hash
function Get-QueueLiteral([string]$Name) {
    $raw = [IO.File]::ReadAllText($noteQueueEmitter)
    $match = [regex]::Match($raw, '(?:pub\s+)?const\s+' + [regex]::Escape($Name) + ':\s*&str\s*=\s*r(?<hash>#+)"')
    if (!$match.Success) { throw "Missing source literal $Name" }
    $from = $match.Index + $match.Length
    $to = $raw.IndexOf('"' + $match.Groups['hash'].Value + ';', $from)
    if ($to -lt $from) { throw 'Missing literal end' }
    $raw.Substring($from, $to - $from).Replace("`r`n", "`n")
}
$noteQueueProduction = Get-QueueLiteral 'CONTROLLER'
$noteQueueTests = Get-QueueLiteral 'TEST_CONTENTS'
$noteQueueTestCount = [regex]::Matches($noteQueueTests, '@Test\s+fun ').Count
if($noteQueueTestCount -lt 5){throw 'Queue admission epoch domain specification missing'}
$noteQueueHeader = @'
package com.ofairyo.gridtimer.ui
import com.ofairyo.gridtimer.data.NoteSaveFailure
import com.ofairyo.gridtimer.data.NoteSaveResult
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import java.util.concurrent.atomic.AtomicBoolean

'@
$noteQueueCompiled = Join-Path $noteQueueRoot 'app/build/tmp/kotlin-classes/release'
if (!(Test-Path -LiteralPath (Join-Path $noteQueueCompiled 'com/ofairyo/gridtimer/data/NoteSaveResult.class'))) { throw 'Real compiled release save result model is required' }
$noteQueueCache = Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1'
function Get-QueueDependency([string]$Coordinate,[string]$DependencyVersion,[string]$Name){
    $files = @(Get-ChildItem -LiteralPath (Join-Path $noteQueueCache ($Coordinate+'/'+$DependencyVersion)) -File -Recurse | Where-Object Name -eq $Name)
    if($files.Count -ne 1){throw ('Installed queue verification dependency missing or ambiguous: '+$Name)}
    $files[0].FullName
}
$noteQueueCompiler = @(
    (Get-QueueDependency 'org.jetbrains.kotlin/kotlin-compiler-embeddable' '1.9.24' 'kotlin-compiler-embeddable-1.9.24.jar'),
    (Get-QueueDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-QueueDependency 'org.jetbrains.kotlin/kotlin-script-runtime' '1.9.24' 'kotlin-script-runtime-1.9.24.jar'),
    (Get-QueueDependency 'org.jetbrains.kotlin/kotlin-reflect' '1.6.10' 'kotlin-reflect-1.6.10.jar'),
    (Get-QueueDependency 'org.jetbrains.kotlin/kotlin-daemon-embeddable' '1.9.24' 'kotlin-daemon-embeddable-1.9.24.jar'),
    (Get-QueueDependency 'org.jetbrains.intellij.deps/trove4j' '1.0.20200330' 'trove4j-1.0.20200330.jar'),
    (Get-QueueDependency 'org.jetbrains/annotations' '13.0' 'annotations-13.0.jar')
)
$noteQueueRuntime = @($noteQueueCompiled) + @(
    (Get-QueueDependency 'org.jetbrains.kotlin/kotlin-stdlib' '1.9.24' 'kotlin-stdlib-1.9.24.jar'),
    (Get-QueueDependency 'org.jetbrains.kotlinx/kotlinx-coroutines-core-jvm' '1.8.1' 'kotlinx-coroutines-core-jvm-1.8.1.jar'),
    (Get-QueueDependency 'junit/junit' '4.13.2' 'junit-4.13.2.jar'),
    (Get-QueueDependency 'org.hamcrest/hamcrest-core' '1.3' 'hamcrest-core-1.3.jar'),
    (Get-QueueDependency 'org.jetbrains/annotations' '23.0.0' 'annotations-23.0.0.jar')
)
foreach ($entry in $noteQueueCompiler) { if (!(Test-Path -LiteralPath $entry -PathType Leaf)) { throw "Compiler missing: $entry" } }
$noteQueueJava = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$noteQueueUtf8 = [Text.UTF8Encoding]::new($false)
$noteQueueResults = @()
foreach ($case in @('baseline', 'wording_only', 'removed_explicit_barrier', 'removed_identity_guard', 'removed_checkpoint_failure_guard', 'removed_admission_epoch')) {
    $directory = Join-Path $noteQueueEvidence $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $production = $noteQueueProduction
    if ($case -eq 'wording_only') {
        $production = $production.Replace('The note mutation queue is full, closing, or closed.', 'The note save queue is unavailable.')
        if ($production -ceq $noteQueueProduction) { throw 'Wording mutation did not change source' }
    }
    if ($case -eq 'removed_explicit_barrier') {
        $anchor = "        val queued = synchronized(lifecycleLock) {`n            replaceableTail = null"
        if (!$production.Contains($anchor)) { throw 'Explicit barrier anchor missing' }
        $production = $production.Replace($anchor, "        val queued = synchronized(lifecycleLock) {")
    }
    if ($case -eq 'removed_identity_guard') {
        $anchor = 'previous != null && previous.autosaveKey == key'
        if (!$production.Contains($anchor)) { throw 'Identity guard anchor missing' }
        $production = $production.Replace($anchor, 'previous != null')
    }
    if ($case -eq 'removed_checkpoint_failure_guard') {
        $anchor = 'val receipt = checkpoint() ?: return NoteSaveResult.failed(NoteSaveFailure.FLUSH_FAILED)'
        if (!$production.Contains($anchor)) { throw 'Checkpoint failure state anchor missing' }
        $production = $production.Replace($anchor, 'val receipt = checkpoint() ?: return NoteSaveResult.committed()')
    }
    if ($case -eq 'removed_admission_epoch') {
        $anchor = 'if (admitted) acceptedMutationEpoch += 1L'
        if (!$production.Contains($anchor)) { throw 'Actual autosave admission epoch guard missing' }
        $production = $production.Replace($anchor, '// accepted autosave epoch removed')
    }
    $sourcePath = Join-Path $directory 'NoteMutationDrainController.kt'
    $testPath = Join-Path $directory 'NoteSaveQueueTest.kt'
    [IO.File]::WriteAllText($sourcePath, $noteQueueHeader + $production, $noteQueueUtf8)
    [IO.File]::WriteAllText($testPath, $noteQueueTests, $noteQueueUtf8)
    $classes = Join-Path $directory ('classes_' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $noteQueueJava -Xmx512m -cp ($noteQueueCompiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$noteQueueCompiled" -classpath ($noteQueueRuntime -join ';') -d $classes $sourcePath $testPath *> (Join-Path $directory 'compile.log')
    if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath (Join-Path $directory 'compile.log') -Tail 35; throw "Fixture compile failed: $case" }
    & $noteQueueJava -Xmx512m -cp ($classes + ';' + ($noteQueueRuntime -join ';')) org.junit.runner.JUnitCore com.ofairyo.gridtimer.ui.NoteSaveQueueTest *> (Join-Path $directory 'tests.log')
    $testExit = $LASTEXITCODE
    $log = [IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $expectedPass = $case -in @('baseline', 'wording_only')
    $requiredFailure = if ($case -eq 'removed_admission_epoch') { 'acceptedQueueMutationsAdvanceFenceIncludingCoalescedAutosaves' } elseif ($case -eq 'removed_checkpoint_failure_guard') { 'failedCheckpointCannotEnterCommitAndReceiptPrecedesDurableCommit' } else { 'explicitBarrierWorkspaceAndNoteIdentityCannotBeCrossed' }
    $accepted = if ($expectedPass) { $testExit -eq 0 -and $log.Contains("OK ($noteQueueTestCount tests)") } else {
        $testExit -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains($requiredFailure)
    }
    $noteQueueResults += [ordered]@{ case=$case; compiled=$true; testExit=$testExit; expectedPass=$expectedPass; passed=$accepted; actualSourceSha256=(Get-FileHash -LiteralPath $sourcePath).Hash }
    if (!$accepted) { Get-Content -LiteralPath (Join-Path $directory 'tests.log') -Tail 45; throw "Unexpected queue mutation result: $case" }
    Write-Output "$case verified"
}
if ((Get-FileHash -LiteralPath $noteQueueEmitter).Hash -cne $noteQueueSourceHash) { throw 'Production source changed while testing' }
[ordered]@{passed=$true;hostOnly=$true;tests=$noteQueueTestCount;version=$Version;sourceSha256=$noteQueueSourceHash;productionUnchanged=$true;cases=$noteQueueResults;scope='Actual generated controller; real compiled NoteSaveResult model; no emulator or device'} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $noteQueueEvidence 'mutation_result.json') -Encoding utf8
