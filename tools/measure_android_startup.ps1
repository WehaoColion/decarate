# v0.0.1 - Measure an explicitly authorized real phone without clearing its app data or logs.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$Serial, [int]$Runs=5, [string]$Label='optimized')
$ErrorActionPreference='Stop'
if($Serial -match '^emulator-'){throw 'Only explicitly authorized physical phones are supported'}
if($Runs -lt 1 -or $Runs -gt 20){throw 'Runs must be between 1 and 20'}
if($Label -notmatch '^[a-zA-Z0-9_-]+$'){throw 'Invalid evidence label'}
$startupRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version=[regex]::Match([IO.File]::ReadAllText((Join-Path $startupRoot 'app/build.gradle')), "versionName '([^']+)'").Groups[1].Value
$evidence=Join-Path $startupRoot "release_artifacts/verification/v$version/device_startup"
New-Item -ItemType Directory -Force -Path $evidence | Out-Null
$adb='C:\tools\android-sdk\platform-tools\adb.exe'
function Invoke-Phone([string[]]$Arguments) {
    $result=& $adb -s $Serial @Arguments 2>&1 | Out-String
    if($LASTEXITCODE -ne 0){throw "ADB command failed: $result"}
    return $result.Trim()
}
$devices=& $adb devices | Out-String
if($devices -notmatch ([regex]::Escape($Serial)+'\s+device')){throw 'Authorized phone is not available'}
$package=Invoke-Phone @('shell','dumpsys','package','com.ofairyo.gridtimer')
if($package -notmatch ('versionName='+[regex]::Escape($version)+'(?:\s|$)')){throw 'Installed version differs from candidate source'}
$results=@()
for($run=1;$run -le $Runs;$run++) {
    Invoke-Phone @('shell','am','force-stop','com.ofairyo.gridtimer') | Out-Null
    $started=(Get-Date).ToUniversalTime().ToString('o')
    $launch=Invoke-Phone @('shell','am','start','-W','-n','com.ofairyo.gridtimer/.MainActivity')
    if($launch -notmatch 'Status: ok' -or $launch -notmatch 'LaunchState: COLD'){throw "Cold launch failed: $launch"}
    $phoneProcess=Invoke-Phone @('shell','pidof','com.ofairyo.gridtimer')
    if($phoneProcess -notmatch '^\d+$'){throw 'App process was not uniquely identified'}
    $deadline=(Get-Date).AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 800
        $logs=Invoke-Phone @('shell','logcat','-d',"--pid=$phoneProcess",'-v','threadtime','-s','StartupLatency:I','AndroidRuntime:E')
        if($logs -match 'FATAL EXCEPTION'){throw "App crash: $logs"}
        $gate=[regex]::Matches($logs,'loadingGate elapsedMs=(\d+)')
    } while($gate.Count -eq 0 -and (Get-Date) -lt $deadline)
    [IO.File]::WriteAllText((Join-Path $evidence "$Label`_$run.log"), $launch+"`n"+$logs)
    if($gate.Count -eq 0){throw 'No measured ready frame before timeout'}
    $ready=[regex]::Matches($logs,'ready elapsedMs=(\d+)')
    $scan=[regex]::Matches($logs,'snapshotStream rows=(\d+) elapsedMs=(\d+)')
    if($ready.Count -eq 0 -or $scan.Count -eq 0 -or $logs -match 'fallback|incomplete'){throw "Startup fell back or lacks a verified scan: $logs"}
    $result=[ordered]@{run=$run;startedUtc=$started;processId=[int]$phoneProcess;loadingGateMs=[int]$gate[-1].Groups[1].Value;repositoryReadyMs=[int]$ready[-1].Groups[1].Value;verifiedRows=[long]$scan[-1].Groups[1].Value;historyScanMs=[int]$scan[-1].Groups[2].Value;initialDisplayMs=[int][regex]::Match($launch,'TotalTime: (\d+)').Groups[1].Value}
    $results+=$result
    $result | ConvertTo-Json -Compress | Write-Output
    Start-Sleep -Seconds 2
}
$summary=[ordered]@{version=$version;serial=$Serial;kind='real_phone_process_cold_start';dataCleared=$false;logCleared=$false;installedPackage='com.ofairyo.gridtimer';runs=$results;allLoadingUnderOneSecond=(@($results | Where-Object loadingGateMs -ge 1000).Count -eq 0);maxLoadingMs=($results.loadingGateMs | Measure-Object -Maximum).Maximum;meanLoadingMs=($results.loadingGateMs | Measure-Object -Average).Average;completedUtc=(Get-Date).ToUniversalTime().ToString('o')}
[IO.File]::WriteAllText((Join-Path $evidence "$Label`_result.json"), ($summary | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
if(!$summary.allLoadingUnderOneSecond){throw 'The one-second loading target was not met on every run'}
