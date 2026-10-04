# v0.0.1 - Register current-user logon and recovery triggers through the stable entry.
$ErrorActionPreference = 'Stop'
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$taskName = 'GridTimerSyncService'
$taskExecutable = $env:GRID_TIMER_STARTUP_EXE
$taskArguments = $env:GRID_TIMER_STARTUP_ARGUMENTS
if ([string]::IsNullOrWhiteSpace($taskExecutable) -or -not [IO.Path]::IsPathRooted($taskExecutable)) {
    throw 'A fully qualified startup executable is required.'
}
$taskIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$taskSid = $taskIdentity.User.Value
$scheduler = New-Object -ComObject 'Schedule.Service'
$scheduler.Connect()
$folder = $scheduler.GetFolder('\')

function Resolve-TaskSid([string]$value) {
    if ($value.StartsWith('S-1-')) { return $value }
    return ([Security.Principal.NTAccount]::new($value)).Translate([Security.Principal.SecurityIdentifier]).Value
}

function Test-TaskContract($definition) {
    try {
        if ((Resolve-TaskSid $definition.Principal.UserId) -ne $taskSid -or
            $definition.Principal.LogonType -ne 3 -or $definition.Principal.RunLevel -ne 0) { return $false }
        if ($definition.Actions.Count -ne 1) { return $false }
        $action = $definition.Actions.Item(1)
        if ($action.Type -ne 0 -or $action.Path -ne $taskExecutable -or $action.Arguments -ne $taskArguments -or
            $action.WorkingDirectory -ne [IO.Path]::GetDirectoryName($taskExecutable)) { return $false }
        $settings = $definition.Settings
        if (-not $settings.Enabled -or -not $settings.StartWhenAvailable -or -not $settings.AllowDemandStart -or
            $settings.DisallowStartIfOnBatteries -or $settings.StopIfGoingOnBatteries -or
            $settings.RunOnlyIfNetworkAvailable -or $settings.RunOnlyIfIdle -or $settings.WakeToRun -or
            $settings.IdleSettings.StopOnIdleEnd -or
            $settings.AllowHardTerminate -or $settings.MultipleInstances -ne 2 -or
            $settings.ExecutionTimeLimit -ne 'PT0S' -or $settings.RestartInterval -ne 'PT1M' -or
            $settings.RestartCount -ne 3 -or $settings.Priority -ne 7) { return $false }
        if ($definition.Triggers.Count -ne 2) { return $false }
        $logonOk = $false
        $recoveryOk = $false
        foreach ($trigger in $definition.Triggers) {
            if (-not $trigger.Enabled) { return $false }
            if ($trigger.Type -eq 9) {
                $logonOk = (Resolve-TaskSid $trigger.UserId) -eq $taskSid
            } elseif ($trigger.Type -eq 1) {
                $recoveryOk = $trigger.Repetition.Interval -eq 'PT1M' -and
                    [string]::IsNullOrEmpty($trigger.Repetition.Duration) -and
                    -not $trigger.Repetition.StopAtDurationEnd -and
                    [string]::IsNullOrEmpty($trigger.EndBoundary) -and
                    -not [string]::IsNullOrWhiteSpace($trigger.StartBoundary)
            } else { return $false }
        }
        return $logonOk -and $recoveryOk
    } catch { return $false }
}

$definition = $scheduler.NewTask(0)
$definition.RegistrationInfo.Description = 'Keep TenRate sync available after sign-in and recover it after an unexpected exit.'
$definition.Principal.UserId = $taskSid
$definition.Principal.LogonType = 3
$definition.Principal.RunLevel = 0
$definition.Settings.Enabled = $true
$definition.Settings.Hidden = $true
$definition.Settings.StartWhenAvailable = $true
$definition.Settings.AllowDemandStart = $true
$definition.Settings.DisallowStartIfOnBatteries = $false
$definition.Settings.StopIfGoingOnBatteries = $false
$definition.Settings.RunOnlyIfNetworkAvailable = $false
$definition.Settings.RunOnlyIfIdle = $false
$definition.Settings.IdleSettings.StopOnIdleEnd = $false
$definition.Settings.WakeToRun = $false
$definition.Settings.AllowHardTerminate = $false
$definition.Settings.MultipleInstances = 2
$definition.Settings.ExecutionTimeLimit = 'PT0S'
$definition.Settings.RestartInterval = 'PT1M'
$definition.Settings.RestartCount = 3
$definition.Settings.Priority = 7
$logon = $definition.Triggers.Create(9)
$logon.Id = 'TenRateLogon'
$logon.UserId = $taskSid
$logon.Enabled = $true
$recovery = $definition.Triggers.Create(1)
$recovery.Id = 'TenRateRecovery'
$recovery.StartBoundary = [DateTime]::Now.AddMinutes(1).ToString('yyyy-MM-ddTHH:mm:ss')
$recovery.Repetition.Interval = 'PT1M'
$recovery.Repetition.StopAtDurationEnd = $false
$recovery.Enabled = $true
$action = $definition.Actions.Create(0)
$action.Path = $taskExecutable
$action.Arguments = $taskArguments
$action.WorkingDirectory = [IO.Path]::GetDirectoryName($taskExecutable)
if (-not (Test-TaskContract $definition)) { throw 'Startup task failed its safety and recovery contract.' }

if ($env:GRID_TIMER_STARTUP_DRY_RUN -eq '1') {
    [pscustomobject]@{valid=$true;xml=$definition.XmlText} | ConvertTo-Json -Compress
    exit 0
}

$existing = $null
try { $existing = $folder.GetTask($taskName) }
catch {
    $lookupError = $_.Exception
    while ($lookupError.InnerException) { $lookupError = $lookupError.InnerException }
    if ($lookupError.HResult -ne -2147024894) { throw }
}
if ($existing) {
    if ((Resolve-TaskSid $existing.Definition.Principal.UserId) -ne $taskSid) {
        throw 'The existing startup task belongs to a different user.'
    }
    if (Test-TaskContract $existing.Definition) { Write-Output 'UNCHANGED'; exit 0 }
}
$registered = $folder.RegisterTaskDefinition($taskName, $definition, 6, $null, $null, 3, $null)
if (-not (Test-TaskContract $registered.Definition)) { throw 'Registered startup task could not be verified.' }
Write-Output 'REGISTERED'
