# v0.0.2 - Verify current release phase recovery and initial deadline delivery.
# v0.0.1 - Exercise actual bell guards with isolated compiled mutations.
$ErrorActionPreference = 'Stop'
$bellRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$bellVersion = [regex]::Match((Get-Content (Join-Path $bellRoot 'app/build.gradle') -Raw), "versionName '([^']+)'").Groups[1].Value
$bellEvidence = Join-Path $bellRoot "release_artifacts/verification/v$bellVersion"
$bellGenerated = Join-Path $bellRoot 'app/build/generated/source/rustAndroid/main/com/ofairyo/gridtimer'
$bellJava = 'C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$bellEncoding = [Text.UTF8Encoding]::new($false)
$old = Get-Content (Join-Path $bellRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$runtime = @($old.runtime | ForEach-Object {
    if($_ -match '^(.*?)\\app\\build\\(.*)$'){Join-Path $bellRoot ('app/build/' + $Matches[2])}else{$_}
} | Where-Object { (Test-Path -LiteralPath $_ -PathType Leaf) -and $_.EndsWith('.jar') })
$compiler = $old.compiler -join ';'
$dependencyRoot=Join-Path $bellEvidence ('mutation_dependencies/'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
New-Item -ItemType Directory -Force -Path $dependencyRoot | Out-Null
$snapshotRuntime=@()
$friendPaths=''
foreach($dependency in $runtime){
    $resolved=$dependency
    if($dependency.StartsWith($bellRoot,[StringComparison]::OrdinalIgnoreCase)){
        $resolved=Join-Path $dependencyRoot ($snapshotRuntime.Count.ToString()+'_'+(Split-Path $dependency -Leaf))
        $digest=(Get-FileHash -LiteralPath $dependency).Hash
        Copy-Item -LiteralPath $dependency -Destination $resolved
        if((Get-FileHash -LiteralPath $resolved).Hash -ne $digest){throw 'Build dependency changed while copying'}
    }
    if($dependency -match 'runtime_app_classes_jar.*classes.jar$'){$friendPaths=$resolved}
    $snapshotRuntime+=$resolved
}
if(-not $friendPaths){throw 'Compiled app classes are unavailable'}
$runtime=$snapshotRuntime
$tests = [IO.File]::ReadAllText((Join-Path $bellRoot 'app/build/generated/source/rustAndroid/test/com/ofairyo/gridtimer/ui/TenfoldEditingTest.kt'))
$end = $tests.IndexOf('    @Test fun structuredKnowledgeSurvivesAndroidSerializationAndKeepsEditorGuard()')
if($end -lt 0){throw 'Boundary test source missing'}
$tests = $tests.Substring(0,$end).Replace('class TenfoldEditingTest {','class TenfoldBellBoundaryTest {') + "`n}`n"
$startupStart = $tests.IndexOf('    @Test fun startupPipelineWaitsForTheLastVerdictBeforeReadiness()')
$startupEnd = $tests.IndexOf('    @Test fun clockAndAlarmDeliverOnlyOnceAcrossAResume()')
if($startupStart -lt 0 -or $startupEnd -le $startupStart){throw 'Startup test boundaries missing'}
$tests = $tests.Remove($startupStart, $startupEnd - $startupStart)
$testCount = ([regex]'@Test fun ').Matches($tests).Count
if($testCount -ne 13){throw 'Unexpected bell regression test count'}
$sources = @{}
foreach($relative in @('notifications/TimerBellPlayer.kt','notifications/MicroBreakReminderNotifier.kt','data/MicroBreaks.kt')){
    $sources[(Split-Path $relative -Leaf)] = [IO.File]::ReadAllText((Join-Path $bellGenerated $relative))
}
$tokenStart=$sources['MicroBreaks.kt'].IndexOf('internal data class TimerBellToken(')
if($tokenStart -lt 0){throw 'Production bell token source missing'}
$sources['MicroBreaks.kt']="package com.ofairyo.gridtimer.data`nimport kotlinx.coroutines.flow.collectLatest`n`n"+$sources['MicroBreaks.kt'].Substring($tokenStart)
$sourceHashes = @{}
foreach($relative in @('notifications/TimerBellPlayer.kt','notifications/MicroBreakReminderNotifier.kt','data/MicroBreaks.kt')){
    $sourceHashes[$relative]=(Get-FileHash (Join-Path $bellGenerated $relative)).Hash
}
$results = @()
foreach($case in @('baseline','wording_only','removed_phase_recovery','removed_initial_delivery','removed_manual_start_guard','removed_validity_guard','removed_stop','removed_phase_guard','canceled_commit','removed_pending_pause','removed_deduplication')){
    $directory = Join-Path $bellEvidence ('mutation/' + $case)
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $paths = @()
    foreach($name in $sources.Keys){
        $text = $sources[$name]
        if($case -eq 'wording_only' -and $name -eq 'MicroBreakReminderNotifier.kt'){
            $text = $text.Replace('休息结束后自动继续。','休息结束自动继续。')
        }
        if($case -eq 'removed_phase_recovery' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('val recovered = resolution.data.slots.mapNotNull', 'val recovered = emptyList<TimerSlot>().mapNotNull')
        }
        if($case -eq 'removed_initial_delivery' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('            deliver(resolution, at)', '            if (false) deliver(resolution, at)')
        }
        if($case -eq 'removed_manual_start_guard' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('if (slot.activeRunId == manualRun || slot.activeRunId.startsWith("$manualRun-")) return@mapNotNull null', 'if (false) return@mapNotNull null')
        }
        if($case -eq 'removed_validity_guard' -and $name -eq 'TimerBellPlayer.kt'){
            $text = $text.Replace('if (!ticket.isCurrent()) {','if (false) {')
        }
        if($case -eq 'removed_stop' -and $name -eq 'TimerBellPlayer.kt'){
            $text = $text.Replace('tickets.remove(slotId)?.cleanup?.forEach { runCatching(it) }','tickets.remove(slotId)')
        }
        if($case -eq 'removed_phase_guard' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('slot.microBreakPhase == phase && slot.microBreakCycleIndex == cycle &&','true &&')
        }
        if($case -eq 'canceled_commit' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('kotlinx.coroutines.withContext(kotlinx.coroutines.NonCancellable) { block() }','block()')
        }
        if($case -eq 'removed_pending_pause' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('(pauses[token.workspace to token.slotId] ?: 0) == 0','true')
        }
        if($case -eq 'removed_deduplication' -and $name -eq 'MicroBreaks.kt'){
            $text = $text.Replace('delivered[key] == token ||','false ||')
        }
        $outputName=if($name -eq 'MicroBreaks.kt'){'TimerBellState.kt'}else{$name}
        $path=Join-Path $directory $outputName
        [IO.File]::WriteAllText($path,$text,$bellEncoding)
        $paths += $path
    }
    $testPath=Join-Path $directory 'TenfoldBellBoundaryTest.kt'
    [IO.File]::WriteAllText($testPath,$tests,$bellEncoding)
    $classes=Join-Path $directory ('classes_'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $bellJava -Xmx768m -cp $compiler org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$friendPaths" -classpath ($runtime -join ';') -d $classes @paths $testPath *> (Join-Path $directory 'compile.log')
    if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'compile.log') -Tail 20; throw "Mutation $case did not compile"}
    & $bellJava -Xmx512m -cp (($classes + ';') + ($runtime -join ';')) org.junit.runner.JUnitCore com.ofairyo.gridtimer.ui.TenfoldBellBoundaryTest *> (Join-Path $directory 'tests.log')
    $exitCode=$LASTEXITCODE
    $log=[IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $expectedPass=$case -in @('baseline','wording_only')
    $passed=if($expectedPass){$exitCode -eq 0 -and $log.Contains("OK ($testCount tests)")}else{$exitCode -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains("Tests run: $testCount")}
    $results += [ordered]@{case=$case;exitCode=$exitCode;expectedPass=$expectedPass;passed=$passed}
    if(-not $passed){throw "Mutation $case did not meet the business-state assertion expectation"}
    Write-Output "$case passed"
}
foreach($relative in $sourceHashes.Keys){
    if((Get-FileHash (Join-Path $bellGenerated $relative)).Hash -ne $sourceHashes[$relative]){throw 'Production source changed during isolated mutation'}
}
[ordered]@{passed=$true;tests=$testCount;cases=$results;sourceSha256=$sourceHashes;productionUnchanged=$true} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $bellEvidence 'mutation_result.json') -Encoding utf8
