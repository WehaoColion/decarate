# v0.0.1 - Execute generated query bodies, measure allocations and remove a deletion guard.
$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match([IO.File]::ReadAllText((Join-Path $taskRoot 'app/build.gradle')),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $taskRoot "release_artifacts/verification/v$version/note_performance"
New-Item -ItemType Directory -Force -Path $evidence | Out-Null
$emitter=Join-Path $taskRoot 'native/gridtimer_native/src/sourcegen/android_note_list_performance.rs'
$raw=[IO.File]::ReadAllText($emitter)
function Get-NoteLiteral([string]$Text,[string]$Name) {
    $start=$Text.IndexOf('const '+$Name+': &str = r###"')
    if($start -lt 0){throw "Missing literal $Name"}
    $start=$Text.IndexOf('r###"',$start)+5; $end=$Text.IndexOf('"###;',$start)
    return $Text.Substring($start,$end-$start)
}
function Get-NoteSection([string]$Text,[string]$Start,[string]$End) {
    $from=$Text.IndexOf($Start); if($from -lt 0){throw "Missing section $Start"}
    $to=$Text.IndexOf($End,$from); if($to -lt 0){throw "Missing section $End"}
    return $Text.Substring($from,$to-$from)
}
$before=[IO.File]::ReadAllText((Join-Path $taskRoot "release_artifacts/verification/v$version/before/Models.kt.txt"))
$header="import com.ofairyo.gridtimer.data.*`nimport com.ofairyo.gridtimer.core.NativeOptimizerBridge`nimport kotlinx.serialization.*`nimport kotlinx.serialization.json.*`n"
$comparator=Get-NoteSection $before 'private fun noteComparator(' 'private fun sanitizeTimestamp('
$old="package perf.before`n"+$header+(Get-NoteSection $before '@OptIn(ExperimentalSerializationApi::class)' '@Serializable')+(Get-NoteSection $before 'fun AppData.activeNotes():' 'internal fun AppData.allNoteAttachments()')+(Get-NoteSection $before 'fun AppData.sortedActiveNotes(' 'fun AppData.sortedTrashedNotes()')+$comparator
$current="package perf.current`n"+$header+(Get-NoteLiteral $raw 'VISIBILITY')+(Get-NoteLiteral $raw 'SORTING')+$comparator
$test=Get-NoteLiteral $raw 'TEST_CONTENTS'
$imports=@('activeNotes','trashedNotes','activeNotebookDocuments','activeStickyNotes','trashedNotebookDocuments','sortedActiveNotes','sortedActiveNotebookDocuments','sortedActiveStickyNotes','noteSortTitles') | ForEach-Object {"import perf.current.$_"}
$test=$test.Replace('import com.ofairyo.gridtimer.data.*',"import com.ofairyo.gridtimer.data.*`n"+($imports -join "`n"))
$driver=Get-NoteLiteral ([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'note_list_probe_sources.rs'))) 'DRIVER'
$prior=Get-Content (Join-Path $taskRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$compiled=Join-Path $taskRoot 'app/build/tmp/kotlin-classes/release'
$runtime=@($compiled)+@($prior.runtime | ForEach-Object {if($_ -match '^(.*?)\\app\\build\\(.*)$'){Join-Path $taskRoot ('app/build/'+$Matches[2])}else{$_}} | Where-Object {(Test-Path -LiteralPath $_ -PathType Leaf) -and $_.EndsWith('.jar')})
$androidRuntime=@(& rg --files (Join-Path $env:USERPROFILE '.gradle/caches/transforms-4') -g android.jar)
if($androidRuntime.Count -ne 1){throw 'Resolve the current mockable Android jar'}
$runtime+=$androidRuntime[0]
$java='C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$utf8=[Text.UTF8Encoding]::new($false)
$results=@()
foreach($case in @('baseline','removed_deleted_guard')) {
    $directory=Join-Path $evidence $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $source=$current
    if($case -eq 'removed_deleted_guard') {
        $guard='!note.isDeleted() && (kind == null'
        if(!$source.Contains($guard)){throw 'Deletion guard missing'}
        $source=$source.Replace($guard,'(kind == null')
    }
    $paths=@('PreviousNotes.kt','CurrentNotes.kt','NoteListPerformanceTest.kt','NoteProbe.kt') | ForEach-Object {Join-Path $directory $_}
    $sources=@($old,$source,$test,$driver)
    for($i=0;$i -lt $paths.Count;$i++){[IO.File]::WriteAllText($paths[$i],$sources[$i],$utf8)}
    $classes=Join-Path $directory ('classes_'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $java -Xmx768m -cp ($prior.compiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$compiled" -classpath ($runtime -join ';') -d $classes @paths *> (Join-Path $directory 'compile.log')
    if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'compile.log') -Tail 30; throw "Note probe compilation failed: $case"}
    & $java -Xmx768m -cp ($classes+';'+($runtime -join ';')) org.junit.runner.JUnitCore com.ofairyo.gridtimer.ui.NoteListPerformanceTest *> (Join-Path $directory 'tests.log')
    $code=$LASTEXITCODE; $log=[IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $passed=if($case -eq 'baseline'){$code -eq 0 -and $log.Contains('OK (4 tests)')}else{$code -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains('folderAndKindFiltersRunWithoutAdmittingDeletedOrForeignNotes')}
    if(!$passed){Get-Content (Join-Path $directory 'tests.log') -Tail 30; throw "Unexpected note mutation: $case"}
    $results += [pscustomobject]@{case=$case;passed=$passed;exitCode=$code}
    if($case -eq 'baseline') {
        & $java -Xmx768m -cp ($classes+';'+($runtime -join ';')) perf.NoteProbeKt 1> (Join-Path $evidence 'measurement.json') 2> (Join-Path $directory 'probe.log')
        if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'probe.log') -Tail 20; throw 'Note performance benchmark failed'}
    }
    Write-Output "$case note query checks passed"
}
$measurement=Get-Content (Join-Path $evidence 'measurement.json') -Raw | ConvertFrom-Json
if(!$measurement.passed){throw 'Note measurement failed'}
$summary=@($measurement.results | ForEach-Object {
    $times=@($_.samples.nanos | Sort-Object)
    [pscustomobject]@{case=$_.case;medianMicros=$times[10]/1000;allocatedBytes=($_.samples.allocatedBytes | Measure-Object -Average).Average}
})
[pscustomobject]@{passed=$true;hostOnly=$true;version=$version;sourceSha256=(Get-FileHash -LiteralPath $emitter).Hash;tests=4;cases=$results;measurements=$summary;optimizedTitleReads=$measurement.optimizedTitleReads;completed=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $evidence 'acceptance.json') -Encoding utf8
$summary | Format-Table
