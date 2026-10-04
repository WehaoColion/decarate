# v0.0.4 - Require drag tests to reject stale cached connection geometry.
# v0.0.3 - Require board capacity tests to reject removal of the creation guard.
# v0.0.2 - Compile current reply serializers alongside the real worker and guards.
# v0.0.1 - Run production canvas worker, geometry and privacy guards on the JVM.
$ErrorActionPreference='Stop'
$canvasRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$canvasVersion=[regex]::Match((Get-Content (Join-Path $canvasRoot 'app/build.gradle') -Raw),"versionName '([^']+)'").Groups[1].Value
$canvasEvidence=Join-Path $canvasRoot "release_artifacts/verification/v$canvasVersion/canvas_ui_mutation"
$canvasEmitter=Join-Path $canvasRoot 'native/gridtimer_native/src/sourcegen/android_canvas_ui.rs'
$canvasHash=(Get-FileHash -LiteralPath $canvasEmitter).Hash
$raw=[IO.File]::ReadAllText($canvasEmitter)
function Get-CanvasLiteral([string]$Name) {
    $start=$raw.IndexOf('pub const '+$Name+': &str = r####"')
    if($start -lt 0){throw "Missing source literal $Name"}
    $start=$raw.IndexOf('r####"',$start)+6
    $end=$raw.IndexOf('"####;',$start)
    return $raw.Substring($start,$end-$start)
}
$production=Get-CanvasLiteral 'CONTENTS'
$testSource=Get-CanvasLiteral 'TEST_CONTENTS'
$testCount=([regex]'@Test fun ').Matches($testSource).Count
# Execute exact non-UI bodies and serializers with the app's compiler plugin.
# UI composables are checked by the formal Android compiler.
$end=$production.IndexOf('private fun canvasTint(')
$modelStart=$production.IndexOf('@Serializable internal data class CanvasNode')
$modelEnd=$production.IndexOf('// This worker outlives')
if($end -lt 0 -or $modelStart -lt 0 -or $modelEnd -le $modelStart){throw 'Production extraction boundaries missing'}
$production=$production.Substring(0,$end)
$production="package com.ofairyo.gridtimer.ui`nimport androidx.compose.ui.geometry.Offset`nimport androidx.compose.ui.unit.IntSize`nimport com.ofairyo.gridtimer.data.*`nimport kotlinx.coroutines.*`nimport kotlinx.coroutines.flow.*`nimport kotlinx.coroutines.channels.Channel`nimport kotlinx.serialization.Serializable`nimport kotlinx.serialization.json.*`nimport kotlin.math.*`n"+$production.Substring($production.IndexOf('internal object KnowledgeCanvasNative'))
$prior=Get-Content (Join-Path $canvasRoot 'release_artifacts/verification/v2.22.42-timer-response/test_classpath.json') -Raw | ConvertFrom-Json
$compiled=Join-Path $canvasRoot 'app/build/tmp/kotlin-classes/release'
if(!(Test-Path (Join-Path $compiled 'com/ofairyo/gridtimer/ui/CanvasReply.class'))){throw 'Compile the current release canvas before JVM mutation checks'}
$runtime=@($compiled)+@($prior.runtime | ForEach-Object {if($_ -match '^(.*?)\\app\\build\\(.*)$'){Join-Path $canvasRoot ('app/build/'+$Matches[2])}else{$_}} | Where-Object {(Test-Path -LiteralPath $_ -PathType Leaf) -and $_.EndsWith('.jar')})
$androidRuntime=@(& rg --files (Join-Path $env:USERPROFILE '.gradle/caches/transforms-4') -g android.jar)
if($androidRuntime.Count -ne 1){throw 'Resolve the current Gradle mockable Android jar before running canvas JVM checks'}
$runtime += $androidRuntime[0]
$compiler=$prior.compiler -join ';'
$serializationPlugin=@(& rg --files (Join-Path $env:USERPROFILE '.gradle/caches/modules-2/files-2.1/org.jetbrains.kotlin/kotlin-serialization-compiler-plugin-embeddable/1.9.24') -g '*.jar')
if($serializationPlugin.Count -ne 1){throw 'Resolve the Kotlin serialization plugin used by this release'}
$java='C:\tools\java\jdk-17.0.18+8\bin\java.exe'
$utf8=[Text.UTF8Encoding]::new($false)
$results=@()
foreach($case in @('baseline','wording_only','removed_encrypted_preview_guard','removed_board_creation_guard','removed_drag_geometry_update')) {
    $directory=Join-Path $canvasEvidence $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $text=$production
    if($case -eq 'wording_only'){$text=$text.Replace('已加密页面','加密页面')}
    if($case -eq 'removed_encrypted_preview_guard'){
        $guard='note.encryption != null ->'
        if(!$text.Contains($guard)){throw 'Privacy guard missing'}
        $text=$text.Replace($guard,'false ->')
    }
    if($case -eq 'removed_board_creation_guard'){
        $guard='if (request["op"]?.jsonPrimitive?.contentOrNull == "new" && !canCreateBoard) return false'
        if(!$text.Contains($guard)){throw 'Board creation guard missing'}
        $text=$text.Replace($guard,'')
    }
    if($case -eq 'removed_drag_geometry_update'){
        $guard='if (dragged != null && (edge.from.id == dragged || edge.to.id == dragged))'
        if(!$text.Contains($guard)){throw 'Drag geometry update missing'}
        $text=$text.Replace($guard,'if (false)')
    }
    $sourcePath=Join-Path $directory 'KnowledgeCanvasState.kt'
    $testPath=Join-Path $directory 'KnowledgeCanvasTest.kt'
    [IO.File]::WriteAllText($sourcePath,$text,$utf8)
    [IO.File]::WriteAllText($testPath,$testSource,$utf8)
    $classes=Join-Path $directory ('classes_'+[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    & $java -Xmx768m -cp $compiler org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect -jvm-target 17 -module-name app_release "-Xfriend-paths=$compiled" "-Xplugin=$($serializationPlugin[0])" -classpath ($runtime -join ';') -d $classes $sourcePath $testPath *> (Join-Path $directory 'compile.log')
    if($LASTEXITCODE -ne 0){Get-Content (Join-Path $directory 'compile.log') -Tail 30; throw "Canvas JVM mutation compilation failed: $case"}
    & $java -Xmx512m -cp ($classes+';'+($runtime -join ';')) org.junit.runner.JUnitCore com.ofairyo.gridtimer.ui.KnowledgeCanvasTest *> (Join-Path $directory 'tests.log')
    $code=$LASTEXITCODE
    $log=[IO.File]::ReadAllText((Join-Path $directory 'tests.log'))
    $expectedPass=$case -in @('baseline','wording_only')
    $failureTest=switch($case){'removed_board_creation_guard' {'creationDisablesWhileSavingAndAtTheBoardLimit'} 'removed_drag_geometry_update' {'draggingUpdatesIncidentConnectionsAndCancellingRestoresTheSavedGeometry'} default {'encryptedPagePreviewsNeverExposeAnUnlockedCopy'}}
    $passed=if($expectedPass){$code -eq 0 -and $log.Contains("OK ($testCount tests)")}else{$code -ne 0 -and $log.Contains('java.lang.AssertionError') -and $log.Contains($failureTest)}
    $results += [ordered]@{case=$case;exitCode=$code;expectedPass=$expectedPass;passed=$passed}
    if(!$passed){Get-Content (Join-Path $directory 'tests.log') -Tail 45; throw "Unexpected canvas JVM mutation result: $case"}
    Write-Output "$case verified"
}
if((Get-FileHash -LiteralPath $canvasEmitter).Hash -ne $canvasHash){throw 'Canvas production emitter changed during verification'}
[ordered]@{passed=$true;tests=$testCount;sourceSha256=$canvasHash;productionUnchanged=$true;cases=$results} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $canvasEvidence 'mutation_result.json') -Encoding utf8
