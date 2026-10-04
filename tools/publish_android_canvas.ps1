# v0.0.2 - Keep current an exact release set and roll back Android publication under the shared release mutex.
# v0.0.1 - Verify and publish a signed Android canvas release while retaining Windows.
[CmdletBinding()]
param([ValidateSet('publish','verify')][string]$Mode='verify', [switch]$RequireStartupUnderOneSecond)
$ErrorActionPreference='Stop'
$releaseRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$gradle=[IO.File]::ReadAllText((Join-Path $releaseRoot 'app/build.gradle'))
$releaseVersion=[regex]::Match($gradle,"versionName '([^']+)'").Groups[1].Value
$releaseCode=[int][regex]::Match($gradle,'versionCode (\d+)').Groups[1].Value
$evidence=Join-Path $releaseRoot "release_artifacts/verification/v$releaseVersion"
$name="tenfold_v$releaseVersion.apk"
$apk=Join-Path $releaseRoot "app/build/outputs/apk/release/$name"
$manifestPath=Join-Path $releaseRoot 'release_artifacts/current/release_manifest.json'
$beforeText=[IO.File]::ReadAllText($manifestPath)
$manifest=$beforeText | ConvertFrom-Json
$build=Get-Content (Join-Path $evidence 'offline_build_acceptance.json') -Raw | ConvertFrom-Json
if(!$build.passed -or $build.mode -ne 'build' -or $build.version -ne $releaseVersion -or $build.canvasTests -lt 9){throw 'Formal canvas build acceptance missing'}
$hash=(Get-FileHash -LiteralPath $apk).Hash.ToLowerInvariant()
if($hash -ne $build.apkSha256){throw 'APK changed after verification'}
$startupMeasurementPath=Join-Path $evidence 'device_startup/optimized_result.json'
if($RequireStartupUnderOneSecond){
    $startupMeasurement=Get-Content -LiteralPath $startupMeasurementPath -Raw | ConvertFrom-Json
    if($startupMeasurement.version -ne $releaseVersion -or !$startupMeasurement.allLoadingUnderOneSecond -or $startupMeasurement.runs.Count -lt 5){throw 'Real-phone startup acceptance is missing or exceeds one second'}
    foreach($pair in @(@('history_codec_mutation','native/gridtimer_native/src/android_snapshot_codec.rs'),@('startup_decode_mutation','native/gridtimer_native/src/sourcegen/android_startup_decode.rs'))){
        $startupMutation=Get-Content -LiteralPath (Join-Path $evidence ($pair[0]+'/mutation_result.json')) -Raw | ConvertFrom-Json
        if(!$startupMutation.passed -or !$startupMutation.productionUnchanged -or $startupMutation.sourceSha256 -ne (Get-FileHash -LiteralPath (Join-Path $releaseRoot $pair[1])).Hash){throw 'Startup mutation evidence differs from release sources'}
    }
}
$provenance=Get-Content (Join-Path $evidence 'android_source_provenance.json') -Raw | ConvertFrom-Json
if($provenance.sha256 -ne $build.sourceSnapshotSha256){throw 'Source snapshot mismatch'}
foreach($file in $provenance.files){
    if((Get-FileHash -LiteralPath (Join-Path $releaseRoot $file.path)).Hash.ToLowerInvariant() -ne $file.sha256){throw "Source changed: $($file.path)"}
}
foreach($pair in @(@('canvas_mutation','native/gridtimer_native/src/android_canvas.rs'),@('canvas_ui_mutation','native/gridtimer_native/src/sourcegen/android_canvas_ui.rs'))){
    $mutation=Get-Content (Join-Path $evidence ($pair[0]+'/mutation_result.json')) -Raw | ConvertFrom-Json
    if(!$mutation.passed -or !$mutation.productionUnchanged -or $mutation.sourceSha256 -ne (Get-FileHash -LiteralPath (Join-Path $releaseRoot $pair[1])).Hash){throw 'Canvas mutation evidence does not match the source'}
}
$scenarios=Get-Content (Join-Path $evidence 'canvas_scenarios/result.json') -Raw | ConvertFrom-Json
if(!$scenarios.passed -or $scenarios.scenarios.Count -ne 9 -or $scenarios.sourceSha256 -ne (Get-FileHash -LiteralPath (Join-Path $releaseRoot 'native/gridtimer_native/src/android_canvas.rs')).Hash){throw 'Canvas persistence and capacity scenario evidence is missing or stale'}
function Assert-ReleasePath([string]$Path){
    $resolved=[IO.Path]::GetFullPath($Path)
    if(!$resolved.StartsWith($releaseRoot+'\',[StringComparison]::OrdinalIgnoreCase)){throw "Path outside project: $resolved"}
    return $resolved
}
$windows=@($manifest.files | Where-Object role -ne 'apk' | ForEach-Object {
    $relative=if($_.role -eq 'cloudflared'){'release_artifacts/current/tools/'+$_.file_name}else{'release_artifacts/current/'+$_.file_name}
    $path=Join-Path $releaseRoot $relative
    if((Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ne $_.sha256){throw "Retained Windows artifact mismatch: $relative"}
    [ordered]@{path=$path;sha256=$_.sha256;modified=(Get-Item -LiteralPath $path).LastWriteTimeUtc.Ticks}
})
$windowsVersion=$manifest.version
$oldJava=$env:JAVA_HOME
$oldPath=$env:PATH
try {
    $env:JAVA_HOME='C:\tools\java\jdk-17.0.18+8'
    $env:PATH="$env:JAVA_HOME\bin;$env:PATH"
    $sdk='C:\tools\android-sdk\build-tools\35.0.0'
    & (Join-Path $sdk 'apksigner.bat') verify --verbose --print-certs $apk *> (Join-Path $evidence 'apk_signature.txt')
    if($LASTEXITCODE -ne 0){throw 'Release signature verification failed'}
    $signature=Get-Content (Join-Path $evidence 'apk_signature.txt') -Raw
    $certificate=[regex]::Match($signature,'Signer #1 certificate SHA-256 digest: ([a-f0-9]+)').Groups[1].Value
    $previousManifest=Get-Content (Join-Path $evidence 'release_manifest_before.json') -Raw | ConvertFrom-Json
    $previousEntry=@($previousManifest.files | Where-Object role -eq 'apk')[0]
    $previousName=$previousEntry.file_name
    $previousCandidates=@((Join-Path $releaseRoot ('release_artifacts/current/'+$previousName)),(Join-Path $releaseRoot ('old_apks/'+[IO.Path]::GetFileNameWithoutExtension($previousName)+"_current_before_v$releaseVersion.apk")),(Join-Path $releaseRoot ('release_artifacts/verification/v'+$previousManifest.androidRelease.version+'/'+$previousName)))
    $previousApk=$previousCandidates | Where-Object {Test-Path -LiteralPath $_ -PathType Leaf} | Select-Object -First 1
    if(!$previousApk -or (Get-FileHash -LiteralPath $previousApk).Hash.ToLowerInvariant() -ne $previousEntry.sha256){throw 'Previous published APK is unavailable or changed'}
    & (Join-Path $sdk 'apksigner.bat') verify --verbose --print-certs $previousApk *> (Join-Path $evidence 'previous_apk_signature.txt')
    if($LASTEXITCODE -ne 0){throw 'Previous release signature verification failed'}
    $oldSignature=Get-Content (Join-Path $evidence 'previous_apk_signature.txt') -Raw
    if(!$certificate -or $certificate -ne [regex]::Match($oldSignature,'Signer #1 certificate SHA-256 digest: ([a-f0-9]+)').Groups[1].Value){throw 'Upgrade certificate changed'}
    Push-Location (Split-Path $apk -Parent)
    try {
        & (Join-Path $sdk 'aapt.exe') dump badging $name *> (Join-Path $evidence 'apk_badging.txt')
        if($LASTEXITCODE -ne 0){throw 'APK metadata read failed'}
        & (Join-Path $sdk 'zipalign.exe') -c -P 16 -v 4 $name *> (Join-Path $evidence 'apk_alignment.txt')
        if($LASTEXITCODE -ne 0){throw 'APK alignment failed'}
    } finally { Pop-Location }
    $badging=Get-Content (Join-Path $evidence 'apk_badging.txt') -Raw
    if(!$badging.Contains("package: name='com.ofairyo.gridtimer' versionCode='$releaseCode' versionName='$releaseVersion'") -or $badging.Contains('application-debuggable')){throw 'Wrong release package/version or debug flag'}
    $readelf='C:\tools\android-sdk\ndk\27.1.12297006\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-readelf.exe'
    $zip=[IO.Compression.ZipFile]::OpenRead($apk)
    $abis=@('arm64-v8a','armeabi-v7a','x86_64')
    $dexNames=@{'KnowledgeCanvasNative'=$false;'KnowledgeCanvasModel'=$false;'KnowledgeCanvasScreenKt'=$false}
    try {
        foreach($abi in $abis){
            $entry=$zip.GetEntry("lib/$abi/libgridtimer_native.so")
            if(!$entry){throw "Missing ABI $abi"}
            $directory=Join-Path $evidence ('native_symbols/'+$abi)
            New-Item -ItemType Directory -Force -Path $directory | Out-Null
            $library=Join-Path $directory 'libgridtimer_native.so'
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$library,$true)
            & $readelf --dyn-syms --wide $library *> (Join-Path $directory 'symbols.txt')
            if($LASTEXITCODE -ne 0){throw 'Native symbol read failed'}
            $symbols=Get-Content (Join-Path $directory 'symbols.txt') -Raw
            foreach($method in @('open','command','close')){if(!$symbols.Contains('Java_com_ofairyo_gridtimer_ui_KnowledgeCanvasNative_'+$method)){throw "Canvas JNI method missing: $abi $method"}}
        }
        foreach($entry in $zip.Entries | Where-Object FullName -like '*.dex'){
            $stream=$entry.Open(); $memory=[IO.MemoryStream]::new()
            try { $stream.CopyTo($memory); $text=[Text.Encoding]::Latin1.GetString($memory.ToArray()); foreach($className in @($dexNames.Keys)){if($text.Contains('Lcom/ofairyo/gridtimer/ui/'+$className+';')){$dexNames[$className]=$true}} }
            finally { $stream.Dispose(); $memory.Dispose() }
        }
    } finally { $zip.Dispose() }
    if(@($dexNames.Values | Where-Object {!$_}).Count){throw 'Canvas classes are missing from the packaged DEX'}
} finally { $env:JAVA_HOME=$oldJava; $env:PATH=$oldPath }
$folders=[ordered]@{root=$releaseRoot;APK=(Join-Path $releaseRoot 'APK');current=(Join-Path $releaseRoot 'release_artifacts/current');build=(Split-Path $apk -Parent)}
if($Mode -eq 'publish'){
    $previous=$manifest.androidRelease.version
    $parts=$previous.Split('.')
    if($parts.Count -ne 4){throw 'Expected a four-part Android release version'}
    $next=($parts[0..2] -join '.')+'.'+([int]$parts[3]+1)
    if($previous -ne $releaseVersion -and $next -ne $releaseVersion){throw 'Android release must increase only the fourth component by one'}
    foreach($role in @('root','APK')){
        $target=Assert-ReleasePath (Join-Path $folders[$role] $name)
        if((Test-Path -LiteralPath $target) -and (Get-FileHash -LiteralPath $target).Hash.ToLowerInvariant() -ne $hash){throw "Same-version package differs: $target"}
        Copy-Item -LiteralPath $apk -Destination $target -Force
    }
    $apkEntry=@($manifest.files | Where-Object role -eq 'apk')
    if($apkEntry.Count -ne 1){throw 'Expected exactly one APK manifest entry'}
    $oldCurrent=Assert-ReleasePath (Join-Path $folders.current $apkEntry[0].file_name)
    $newCurrent=Assert-ReleasePath (Join-Path $folders.current $name)
    $archive=Assert-ReleasePath (Join-Path $releaseRoot 'old_apks')
    New-Item -ItemType Directory -Force -Path $archive | Out-Null
    $oldArchive=Assert-ReleasePath (Join-Path $archive ([IO.Path]::GetFileNameWithoutExtension($oldCurrent)+"_current_before_v$releaseVersion.apk"))
    $oldCurrentHash=$apkEntry[0].sha256
    $apkEntry[0].file_name=$name; $apkEntry[0].size=(Get-Item -LiteralPath $apk).Length; $apkEntry[0].sha256=$hash
    $manifest.androidRelease=[ordered]@{version=$releaseVersion;versionCode=$releaseCode;androidOnly=$true;verification="release_artifacts/verification/v$releaseVersion";sourceSnapshotSha256=$build.sourceSnapshotSha256;sha256=$hash;createdAt=(Get-Date).ToUniversalTime().ToString('o')}
    # Stage outside current: the stable entry accepts exactly the declared files.
    $pending=Assert-ReleasePath (Join-Path $evidence 'release_manifest.pending.json')
    [IO.File]::WriteAllText($pending,($manifest | ConvertTo-Json -Depth 30),[Text.UTF8Encoding]::new($false))
    $normalizedRoot=[IO.Path]::GetFullPath((Join-Path $releaseRoot 'release_artifacts')).Replace('/','\').ToLowerInvariant().TrimEnd('\')
    $mutexHash=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($normalizedRoot))).ToLowerInvariant()
    $releaseMutex=[Threading.Mutex]::new($false,('Local\TenRate.ReleaseArtifacts.'+$mutexHash))
    $ownsMutex=$false; $movedOld=$false; $copiedNew=$false
    function Restore-CanvasPublication {
        if($copiedNew -and (Test-Path -LiteralPath $newCurrent)){
            if((Get-FileHash -LiteralPath $newCurrent).Hash.ToLowerInvariant() -ne $hash){throw 'Cannot roll back a concurrently changed APK'}
            Remove-Item -LiteralPath $newCurrent -Force
        }
        if($movedOld){
            if((Get-FileHash -LiteralPath $oldArchive).Hash.ToLowerInvariant() -ne $oldCurrentHash){throw 'Cannot restore an altered previous APK'}
            Move-Item -LiteralPath $oldArchive -Destination $oldCurrent
        }
        [IO.File]::WriteAllText($manifestPath,$beforeText,[Text.UTF8Encoding]::new($false))
    }
    try {
        try {$ownsMutex=$releaseMutex.WaitOne(30000)} catch [Threading.AbandonedMutexException] {$ownsMutex=$true}
        if(!$ownsMutex){throw 'Another release operation is active'}
        if(Test-Path -LiteralPath (Join-Path $releaseRoot 'release_artifacts/.release-transaction-v1.json')){throw 'Resolve the existing release transaction before publishing Android'}
        if([IO.File]::ReadAllText($manifestPath) -ne $beforeText){throw 'Release manifest changed concurrently'}
        try {
            if($oldCurrent -ne $newCurrent){
                if((Get-FileHash -LiteralPath $oldCurrent).Hash.ToLowerInvariant() -ne $oldCurrentHash -or (Test-Path -LiteralPath $oldArchive)){throw 'Previous Android release cannot be archived safely'}
                if(Test-Path -LiteralPath $newCurrent){throw 'Unexpected candidate APK in current'}
                Move-Item -LiteralPath $oldCurrent -Destination $oldArchive
                $movedOld=$true
                Copy-Item -LiteralPath $apk -Destination $newCurrent
                $copiedNew=$true
            } elseif((Get-FileHash -LiteralPath $newCurrent).Hash.ToLowerInvariant() -ne $hash){throw 'Same-version current APK differs'}
            Move-Item -LiteralPath $pending -Destination $manifestPath -Force
        } catch {Restore-CanvasPublication; throw}
    } finally {if($ownsMutex){$releaseMutex.ReleaseMutex()}; $releaseMutex.Dispose()}
    $launcher=Join-Path $releaseRoot 'release_artifacts/desktop_entry/TenRate_Desktop_Launcher.exe'
    $process=Start-Process -FilePath $launcher -ArgumentList @('--sync-supervisor','--resolve-only') -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $evidence 'windows_entry_resolution.log') -RedirectStandardError (Join-Path $evidence 'windows_entry_resolution_errors.log')
    if($process.ExitCode -ne 0){
        $releaseMutex=[Threading.Mutex]::new($false,('Local\TenRate.ReleaseArtifacts.'+$mutexHash)); $ownsMutex=$false
        try {
            try {$ownsMutex=$releaseMutex.WaitOne(30000)} catch [Threading.AbandonedMutexException] {$ownsMutex=$true}
            if(!$ownsMutex){throw 'Cannot acquire the release mutex for rollback'}
            Restore-CanvasPublication
        } finally {if($ownsMutex){$releaseMutex.ReleaseMutex()}; $releaseMutex.Dispose()}
        throw 'Windows entry rejected the updated manifest; previous exact release set restored'
    }
    foreach($role in $folders.Keys){
        foreach($old in Get-ChildItem -LiteralPath $folders[$role] -File -Filter '*.apk'){
            if($old.Name -eq $name){continue}
            if($old.Name -notmatch '^(tenfold|grid_timer_app)_v'){throw "Unrecognized APK: $($old.Name)"}
            $source=Assert-ReleasePath $old.FullName
            $destination=Assert-ReleasePath (Join-Path $archive ($old.BaseName+"_${role}_before_v$releaseVersion.apk"))
            if(Test-Path -LiteralPath $destination){throw "Archive destination already exists: $destination"}
            $oldHash=(Get-FileHash -LiteralPath $source).Hash
            Move-Item -LiteralPath $source -Destination $destination
            if((Get-FileHash -LiteralPath $destination).Hash -ne $oldHash){throw 'Archived APK hash mismatch'}
        }
    }
}
$published=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if($published.version -ne $windowsVersion -or $published.androidRelease.version -ne $releaseVersion -or $published.androidRelease.sha256 -ne $hash){throw 'Published version or digest mismatch'}
$copies=foreach($role in $folders.Keys){
    $path=Join-Path $folders[$role] $name
    if((Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant() -ne $hash){throw "Published APK mismatch: $role"}
    [ordered]@{role=$role;path=$path;sha256=$hash;bytes=(Get-Item -LiteralPath $path).Length}
}
foreach($file in $windows){if((Get-FileHash -LiteralPath $file.path).Hash.ToLowerInvariant() -ne $file.sha256 -or (Get-Item -LiteralPath $file.path).LastWriteTimeUtc.Ticks -ne $file.modified){throw 'A retained Windows file changed'}}
$unexpected=@(Get-ChildItem -LiteralPath (Join-Path $releaseRoot 'app') -Recurse -File | Where-Object {$_.Extension -eq '.aab' -or ($_.Extension -eq '.apk' -and $_.FullName -ne $apk)})
if($unexpected.Count){throw 'Non-delivery Android packages remain in app build outputs'}
$receipt=[ordered]@{passed=$true;version=$releaseVersion;versionCode=$releaseCode;sha256=$hash;certificateSha256=$certificate;upgradeCertificateUnchanged=$true;sourceSnapshotSha256=$build.sourceSnapshotSha256;alignment16KVerified=$true;canvasNativeAbis=$abis;canvasDexClasses=$dexNames;formalApkOnly=$true;noDeviceOperations=!(Test-Path -LiteralPath (Join-Path $evidence 'device_installation.json'));windowsVersion=$windowsVersion;windowsArtifactsUnchanged=$true;files=$copies;completed=(Get-Date).ToUniversalTime().ToString('o')}
if($RequireStartupUnderOneSecond){$receipt['startupVerifiedOnRealPhone']=$true;$receipt['startupMeasurement']='device_startup/optimized_result.json'}
$receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $evidence $(if($Mode -eq 'publish'){'delivery_receipt.json'}else{'post_publish_package_check.json'})) -Encoding utf8
[pscustomobject]$receipt | Select-Object passed,version,versionCode,sha256
