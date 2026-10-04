# v0.0.1 - Measure the real previous edge draw loop against the current render cache offline.
$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match((Get-Content (Join-Path $taskRoot 'app/build.gradle') -Raw),"versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $taskRoot "release_artifacts/verification/v$version"
$directory=Join-Path $evidence 'canvas_render_performance'
New-Item -ItemType Directory -Force -Path $directory | Out-Null
function Get-Literal([string]$Path,[string]$Name) {
    $raw=[IO.File]::ReadAllText($Path)
    $start=$raw.IndexOf('pub const '+$Name+': &str = r####"')
    if($start -lt 0){throw "Missing Rust source literal $Name"}
    $start=$raw.IndexOf('r####"',$start)+6
    $end=$raw.IndexOf('"####;',$start)
    return $raw.Substring($start,$end-$start)
}
$beforePath=Join-Path $evidence 'before/android_canvas_ui.rs'
$afterPath=Join-Path $taskRoot 'native/gridtimer_native/src/sourcegen/android_canvas_ui.rs'
$before=Get-Literal $beforePath 'CONTENTS'
$after=Get-Literal $afterPath 'CONTENTS'
$priorStart=$before.IndexOf('            fun position(node: CanvasNode)')
$priorEnd=$before.IndexOf('            canvas.nodes.forEach',$priorStart)
if($priorStart -lt 0 -or $priorEnd -le $priorStart){throw 'Previous production draw loop missing'}
$probe=(Get-Literal (Join-Path $PSScriptRoot 'canvas_render_probe.rs') 'PROBE').Replace('__PRIOR_EDGE_BODY__',$before.Substring($priorStart,$priorEnd-$priorStart))
$start=$after.IndexOf('internal object KnowledgeCanvasNative')
$end=$after.IndexOf('private fun canvasTint(')
if($start -lt 0 -or $end -le $start){throw 'Current production helper boundaries missing'}
$imports="package com.ofairyo.gridtimer.ui`nimport androidx.compose.ui.geometry.Offset`nimport androidx.compose.ui.unit.IntSize`nimport com.ofairyo.gridtimer.data.*`nimport kotlinx.coroutines.*`nimport kotlinx.coroutines.flow.*`nimport kotlinx.coroutines.channels.Channel`nimport kotlinx.serialization.Serializable`nimport kotlinx.serialization.json.*`nimport kotlin.math.*`n"
$utf8=[Text.UTF8Encoding]::new($false)
$source=Join-Path $directory 'KnowledgeCanvasState.kt'; $probePath=Join-Path $directory 'CanvasRenderProbe.kt'
[IO.File]::WriteAllText($source,$imports+$after.Substring($start,$end-$start),$utf8)
[IO.File]::WriteAllText($probePath,$probe,$utf8)
$prior=Get-Content (Join-Path $taskRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$compiled=Join-Path $taskRoot 'app/build/tmp/kotlin-classes/release'
$runtime=@($compiled)+@($prior.runtime | ForEach-Object {if($_ -match '^(.*?)\\app\\build\\(.*)$'){Join-Path $taskRoot ('app/build/'+$Matches[2])}else{$_}} | Where-Object {(Test-Path -LiteralPath $_ -PathType Leaf) -and $_.EndsWith('.jar')})
$plugin=@(& rg --files (Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1/org.jetbrains.kotlin/kotlin-serialization-compiler-plugin-embeddable/1.9.24') -g '*.jar')
if($plugin.Count -ne 1){throw 'Serialization compiler plugin unavailable'}
$classes=Join-Path $directory ('classes_'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
$java='C:\tools\java\jdk-17.0.18+8\bin\java.exe'
& $java -Xmx768m -cp ($prior.compiler -join ';') org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$compiled" "-Xplugin=$($plugin[0])" -classpath ($runtime -join ';') -d $classes $source $probePath *> (Join-Path $directory 'compile.log')
if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'compile.log') -Tail 30; throw 'Render probe compilation failed'}
& $java -Xms128m -Xmx768m -cp ($classes+';'+($runtime -join ';')) com.ofairyo.gridtimer.ui.CanvasRenderProbeKt 1> (Join-Path $directory 'measurement.json') 2> (Join-Path $directory 'run.log')
if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'run.log') -Tail 30; throw 'Render probe failed'}
$result=Get-Content (Join-Path $directory 'measurement.json') -Raw | ConvertFrom-Json
if(!$result.passed -or $result.equivalentGeometryFrames -ne 32){throw 'Incomplete render comparison'}
$result | Add-Member -NotePropertyName beforeSourceSha256 -NotePropertyValue (Get-FileHash -LiteralPath $beforePath).Hash
$result | Add-Member -NotePropertyName sourceSha256 -NotePropertyValue (Get-FileHash -LiteralPath $afterPath).Hash
$result | Add-Member -NotePropertyName probeSha256 -NotePropertyValue (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'canvas_render_probe.rs')).Hash
$result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $directory 'comparison.json') -Encoding utf8
$result.scenarios | ForEach-Object { [pscustomobject]@{operation=$_.operation;beforeMicros=$_.before.medianMicrosPerFrame;afterMicros=$_.after.medianMicrosPerFrame;reductionPercent=[math]::Round((1-$_.after.medianMicrosPerFrame/$_.before.medianMicrosPerFrame)*100,1);beforeCalls=$_.before.drawCallsPerFrame;afterCalls=$_.after.drawCallsPerFrame} }
