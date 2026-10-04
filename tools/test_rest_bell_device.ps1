# v0.0.1 - Observe pause cancellation and a complete background rest on the authorized phone.
param([ValidateSet('pause','background')][string]$Mode='pause')
$ErrorActionPreference='Stop'
$bellRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$bellEvidence=Join-Path $bellRoot 'release_artifacts/verification/v2.22.48-rest-bell'
$bellAdb='C:\tools\android-sdk\platform-tools\adb.exe'
$bellDevice='10AE7G1EGZ002AE'
$bellPackage='com.ofairyo.gridtimer'
function Invoke-Phone([string[]]$Arguments){
    $out=& $bellAdb -s $bellDevice @Arguments 2>&1
    if($LASTEXITCODE -ne 0){throw ($out -join "`n")}
    return $out
}
function Assert-Focused {
    $focus=Invoke-Phone @('shell','dumpsys','window','-a')
    if(($focus | Select-String 'mCurrentFocus=.*com.ofairyo.gridtimer/').Count -ne 1){throw 'Phone focus changed; no input was sent'}
    $appWindow=[regex]::Match(($focus -join "`n"),'(?s)Window #\d+ Window\{[^}\r\n]*com\.ofairyo\.gridtimer/[^}]*\}:\s*(.*?)(?=\n  Window #|\z)').Value
    if($appWindow -notmatch 'mWindowingMode=fullscreen'){throw 'Phone window is scaled; no coordinate input was sent'}
}
function Get-Window([string]$Name){
    $dump=Invoke-Phone @('shell','uiautomator','dump','/sdcard/tenfold_bell_window.xml')
    if(($dump -join ' ') -notmatch 'dumped to:'){throw 'A fresh phone window could not be read'}
    Invoke-Phone @('pull','/sdcard/tenfold_bell_window.xml',(Join-Path $bellEvidence ($Name+'.xml'))) | Out-Null
    return [xml](Get-Content (Join-Path $bellEvidence ($Name+'.xml')) -Raw)
}
function Get-Button($Window,[string]$Text){
    $nodes=@($Window.SelectNodes('//node') | Where-Object {$_.text -eq $Text})
    foreach($node in $nodes){
        $button=$node.ParentNode
        if($button.clickable -ne 'true'){continue}
        if(-not $button.ParentNode.SelectSingleNode('.//node[@text="01"]')){continue}
        $b=[regex]::Matches($button.bounds,'\d+') | ForEach-Object {[int]$_.Value}
        return @((($b[0]+$b[2])/2),(($b[1]+$b[3])/2))
    }
    throw "Expected timer summary button missing: $Text"
}
function Tap-Button([double[]]$Point){
    Assert-Focused
    Invoke-Phone @('shell','input','tap',([int]$Point[0]).ToString(),([int]$Point[1]).ToString()) | Out-Null
}
$process=$null
$paused=$false
$rows=[Collections.Generic.List[string]]::new()
$events=[Collections.Generic.List[object]]::new()
function Read-Bell([int]$Milliseconds){
    if($null -eq $script:pendingLine){$script:pendingLine=$process.StandardOutput.ReadLineAsync()}
    if(-not $script:pendingLine.Wait($Milliseconds)){return $null}
    $line=$script:pendingLine.Result
    $script:pendingLine=$null
    if($null -eq $line){throw 'Phone log stream ended'}
    $rows.Add($line)
    if($line -match '^\s*([0-9]+\.[0-9]+).*TimerBell\s*:\s*(started|canceled) slot=(\d+) kind=(FOCUS|BREAK)'){
        $event=[ordered]@{at=[double]::Parse($Matches[1],[Globalization.CultureInfo]::InvariantCulture);action=$Matches[2];slot=[int]$Matches[3];kind=$Matches[4]}
        $events.Add($event)
        return $event
    }
    return $null
}
function Wait-Bell([string]$Kind,[int]$TimeoutSeconds=360,[double]$After=0){
    $until=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while([DateTime]::UtcNow -lt $until){
        $event=Read-Bell 500
        if($event -and $event.at -ge $After -and $event.slot -eq 1 -and $event.action -eq 'started' -and $event.kind -eq $Kind){return $event}
    }
    throw "No $Kind bell arrived within $TimeoutSeconds seconds"
}
function Save-Reminder([string]$Name,[string]$Expected){
    $dump=(Invoke-Phone @('shell','dumpsys','notification','--noredact')) -join "`n"
    $records=[regex]::Matches($dump,'(?ms)^ +NotificationRecord\(.*?(?=^ +NotificationRecord\(|^  [A-Za-z][^\n]*:\s*$|\z)')
    $own=@($records | Where-Object {$_.Value -match '^\s*NotificationRecord\([^\n]*pkg=com\.ofairyo\.gridtimer[^\n]*id=5301(?:\s|,)'} | ForEach-Object {$_.Value})
    $own | Set-Content (Join-Path $bellEvidence ($Name+'.txt')) -Encoding utf8
    if($Expected -eq 'absent'){
        if($own.Count -ne 0){throw 'A slot reminder remained after pause'}
    }elseif(($own -join "`n") -notmatch [regex]::Escape($Expected)){
        throw "Reminder did not match phase: $Expected"
    }
}
try {
    $version=(Invoke-Phone @('shell','dumpsys','package',$bellPackage)) -join "`n"
    if($version -notmatch 'versionName=2.22.48-rest-bell'){throw 'Expected signed release is not installed'}
    $installedPath=((Invoke-Phone @('shell','pm','path',$bellPackage)) | Where-Object {$_ -match '^package:.*base\.apk$'} | Select-Object -First 1) -replace '^package:',''
    if(-not $installedPath){throw 'Installed APK path is unavailable'}
    $installedSha=((Invoke-Phone @('shell','sha256sum',$installedPath)) -split '\s+')[0].ToLowerInvariant()
    $expectedSha=(Get-FileHash (Join-Path $bellRoot 'app/build/outputs/apk/release/tenfold_v2.22.48-rest-bell.apk')).Hash.ToLowerInvariant()
    if($installedSha -ne $expectedSha){throw 'Installed APK does not match this build'}
    Invoke-Phone @('shell','am','start','--user','0','--windowingMode','1','-n',($bellPackage+'/.MainActivity')) | Out-Null
    $window=Get-Window ($Mode+'_initial')
    $pausePoint=Get-Button $window '暂停'
    $phonePid=(Invoke-Phone @('shell','pidof',$bellPackage) | Out-String).Trim()
    $info=[Diagnostics.ProcessStartInfo]::new($bellAdb)
    $info.UseShellExecute=$false
    $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true
    $info.RedirectStandardError=$true
    foreach($argument in @('-s',$bellDevice,'logcat','-T','1','-v','epoch',"--pid=$phonePid",'TimerBell:I','*:S')){$info.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::Start($info)
    $script:pendingLine=$null
    if($Mode -eq 'background'){
        Assert-Focused
        Invoke-Phone @('shell','input','keyevent','KEYCODE_HOME') | Out-Null
        $backgroundSince=[double]::Parse(((Invoke-Phone @('shell','date','+%s.%N')) -join '').Trim(),[Globalization.CultureInfo]::InvariantCulture)
        Write-Output 'Observing next complete rest cycle in the background'
        $break=Wait-Bell 'BREAK' 360 $backgroundSince
        Save-Reminder 'background_break_notification' '休息中'
        $breakWindow=Invoke-Phone @('shell','dumpsys','window')
        $breakWindow | Select-String 'mCurrentFocus=' | Set-Content (Join-Path $bellEvidence 'background_break_focus.txt')
        if(($breakWindow | Select-String 'mCurrentFocus=.*com.ofairyo.gridtimer/').Count -ne 0){throw 'App returned to foreground during background test'}
        $focus=Wait-Bell 'FOCUS' 35
        Save-Reminder 'background_end_notification' '继续专注'
        $focusWindow=Invoke-Phone @('shell','dumpsys','window')
        $focusWindow | Select-String 'mCurrentFocus=' | Set-Content (Join-Path $bellEvidence 'background_end_focus.txt')
        if(($focusWindow | Select-String 'mCurrentFocus=.*com.ofairyo.gridtimer/').Count -ne 0){throw 'App returned to foreground during background test'}
        $gap=$focus.at-$break.at
        if($gap -lt 13 -or $gap -gt 17){throw "Rest boundary gap was $gap seconds"}
        $activeEvents=@($events | Where-Object {$_.at -ge $backgroundSince})
        if(@($activeEvents | Where-Object {$_.slot -eq 1 -and $_.action -eq 'started'}).Count -ne 2){throw 'Duplicate background bell observed'}
        $result=[ordered]@{passed=$true;mode=$Mode;restGapSeconds=$gap;events=$activeEvents}
    }else{
        Write-Output 'Waiting to pause slot 1 after its rest begins'
        $break=Wait-Bell 'BREAK'
        Invoke-Phone @('shell','am','start','--user','0','--windowingMode','1','-n',($bellPackage+'/.MainActivity')) | Out-Null
        $atBreak=Get-Window 'pause_at_break'
        $pausePoint=Get-Button $atBreak '暂停'
        Tap-Button $pausePoint
        $paused=$true
        $pausedWindow=Get-Window 'pause_confirmed'
        $resumePoint=Get-Button $pausedWindow '继续'
        $until=[DateTime]::UtcNow.AddSeconds(20)
        while([DateTime]::UtcNow -lt $until){Read-Bell 500 | Out-Null}
        $cancel=@($events | Where-Object {$_.action -eq 'canceled' -and $_.slot -eq 1 -and $_.at -ge $break.at}) | Select-Object -First 1
        if(-not $cancel){throw 'Pause did not cancel the rest playback'}
        $late=@($events | Where-Object {$_.action -eq 'started' -and $_.slot -eq 1 -and $_.at -gt $cancel.at})
        if($late.Count -ne 0){throw 'A bell started while the timer was paused'}
        Save-Reminder 'paused_reminder_entries' 'absent'
        Invoke-Phone @('shell','dumpsys','alarm') | Set-Content (Join-Path $bellEvidence 'paused_alarms.txt')
        Invoke-Phone @('shell','am','start','--user','0','--windowingMode','1','-n',($bellPackage+'/.MainActivity')) | Out-Null
        $resumeWindow=Get-Window 'before_resume'
        $resumePoint=Get-Button $resumeWindow '继续'
        Tap-Button $resumePoint
        $paused=$false
        $resumedBreak=Wait-Bell 'BREAK' 20
        $resumedFocus=Wait-Bell 'FOCUS' 25
        Save-Reminder 'resumed_end_notification' '继续专注'
        $gap=$resumedFocus.at-$resumedBreak.at
        if($gap -le 0 -or $gap -gt 16){throw "Resumed rest gap was $gap seconds"}
        Get-Window 'pause_resumed_focus' | Out-Null
        $result=[ordered]@{passed=$true;mode=$Mode;pauseAfterBreakSeconds=($cancel.at-$break.at);pausedObservationSeconds=20;latePlaybackCount=$late.Count;resumedRestGapSeconds=$gap;events=$events}
    }
    $result['apkSha256']=$installedSha
    $result | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $bellEvidence ($Mode+'_device_result.json')) -Encoding utf8
    $result | ConvertTo-Json -Depth 8
}finally{
    if($process -and -not $process.HasExited){$process.Kill();$process.WaitForExit()}
    $rows | Set-Content (Join-Path $bellEvidence ($Mode+'_bell_log.txt')) -Encoding utf8
    if($paused){
        $window=Get-Window 'pause_recovery'
        $point=Get-Button $window '继续'
        Tap-Button $point
    }
    if($Mode -eq 'background'){
        Invoke-Phone @('shell','am','start','--user','0','-n',($bellPackage+'/.MainActivity')) | Out-Null
    }
}
