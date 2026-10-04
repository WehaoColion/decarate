# Windows startup/idle probe. Launches only an explicit executable against a
# captured synthetic LOCALAPPDATA tree. Does not send UI input or kill processes.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('CaptureFixture', 'RunSample', 'FinalizeExit', 'Summarize')]
    [string]$Mode,
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [string]$ProfileRoot,
    [string]$Executable,
    [ValidatePattern('^[A-Za-z0-9_-]+$')][string]$Label,
    [ValidateRange(1, 99)][int]$Trial = 1,
    [ValidateSet('FreshProcess', 'Reopen')][string]$Stage = 'FreshProcess',
    [ValidateScript({ $_ -eq 0 -or ($_ -ge 60 -and $_ -le 300) })][int]$IdleSeconds = 60,
    [ValidateRange(0, 30)][int]$SettleSeconds = 5,
    [ValidateRange(30, 600)][int]$StartupTimeoutSeconds = 180,
    [ValidateRange(20, 1000)][int]$SampleIntervalMilliseconds = 50,
    [ValidateRange(1, 60)][int]$ExitTimeoutSeconds = 30,
    [string]$UiObservation = ''
)

$ErrorActionPreference = 'Stop'
$measureProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$measureEvidence = [IO.Path]::GetFullPath($EvidenceRoot)
$measureUtf8 = [Text.UTF8Encoding]::new($false)

function Write-Json([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 20), $measureUtf8)
}

function Full-Path([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'A required path was omitted.' }
    [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
}

function Assert-Descendant([string]$Path, [string]$Parent) {
    $resolved = Full-Path $Path
    $allowed = (Full-Path $Parent) + [IO.Path]::DirectorySeparatorChar
    if (-not $resolved.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path is outside its permitted directory: $resolved"
    }
    $resolved
}

function Assert-SyntheticProfile([string]$Path) {
    $resolved = Full-Path $Path
    if ([IO.Path]::GetFileName($resolved) -notlike 'startup_synthetic*') {
        throw 'The isolated LOCALAPPDATA directory name must start with startup_synthetic.'
    }
    foreach ($actual in @($env:LOCALAPPDATA, $env:USERPROFILE)) {
        if (-not [string]::IsNullOrWhiteSpace($actual) -and $resolved -eq (Full-Path $actual)) {
            throw 'A real user profile cannot be used for this measurement.'
        }
    }
    $resolved
}

function File-Inventory([string]$Root) {
    $prefix = (Full-Path $Root) + [IO.Path]::DirectorySeparatorChar
    @(Get-ChildItem -LiteralPath $Root -Recurse -File | Sort-Object FullName | ForEach-Object {
        [pscustomobject]@{
            relativePath = $_.FullName.Substring($prefix.Length)
            sizeBytes = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    })
}

function Assert-NoReparsePoint([string]$Root) {
    foreach ($item in @((Get-Item -LiteralPath $Root)) + @(Get-ChildItem -LiteralPath $Root -Recurse -Force)) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Synthetic fixture contains a link or reparse point: $($item.FullName)"
        }
    }
}

function Copy-Tree([string]$Source, [string]$Destination) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    Get-ChildItem -LiteralPath $Source -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $Destination -Recurse -Force
    }
}

function Read-Receipt {
    $receipt = Get-Content -LiteralPath (Join-Path $measureEvidence 'fixture_receipt.json') -Raw | ConvertFrom-Json
    if ($receipt.kind -ne 'isolated_synthetic_windows_startup' -or $receipt.projectRoot -ne $measureProject) {
        throw 'The fixture receipt does not belong to this project.'
    }
    $receipt
}

function Find-OwnedProcess($Identity) {
    $found = Get-Process -Id ([int]$Identity.processId) -ErrorAction SilentlyContinue
    if ($null -eq $found) { return $null }
    try {
        $found.Refresh()
        if ($found.StartTime.ToUniversalTime().Ticks -ne [long]$Identity.processStartUtcTicks -or
            (Full-Path $found.Path) -ne (Full-Path $Identity.executable)) {
            return $null # PID reuse is not ownership of a replacement process.
        }
        return $found
    } catch { return $null }
}

function Assert-NoActiveSample {
    foreach ($file in @(Get-ChildItem -LiteralPath $measureEvidence -Filter process_identity.json -Recurse -File -ErrorAction SilentlyContinue)) {
        $identity = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
        if ($null -ne (Find-OwnedProcess $identity)) {
            throw "A prior sample is still running. Close its observed window through computer-use first: PID $($identity.processId)"
        }
        $sampleMeasurement = Join-Path $file.DirectoryName 'measurement.json'
        if (Test-Path -LiteralPath $sampleMeasurement) {
            $prior = Get-Content -LiteralPath $sampleMeasurement -Raw | ConvertFrom-Json
            $exitPath = Join-Path $file.DirectoryName 'exit.json'
            if ($prior.passed -and (-not (Test-Path -LiteralPath $exitPath))) {
                throw 'FinalizeExit must record normal closure of a completed sample before restoring its profile.'
            }
            if ($prior.passed -and -not (Get-Content -LiteralPath $exitPath -Raw | ConvertFrom-Json).cleanExit) {
                throw 'A completed sample did not exit cleanly; investigate before restoring its profile.'
            }
        }
    }
}

function Runtime-Events([string]$LogPath, [long]$SinceEpoch) {
    if (-not (Test-Path -LiteralPath $LogPath -PathType Leaf)) { return @() }
    try {
        $stream = [IO.File]::Open($LogPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try {
            $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8)
            $raw = $reader.ReadToEnd()
        } finally { $stream.Dispose() }
        @($raw -split '\r?\n' | ForEach-Object {
            if ($_ -match '^(\d+) (.*)$' -and [long]$Matches[1] -ge $SinceEpoch) {
                [pscustomobject]@{ epochMillis = [long]$Matches[1]; message = $Matches[2] }
            }
        })
    } catch { @() }
}

function Sample-Directory {
    if ([string]::IsNullOrWhiteSpace($Label)) { throw 'Label is required for this mode.' }
    Join-Path $measureEvidence ('samples/{0}_{1}_{2:00}' -f $Label, $Stage, $Trial)
}

New-Item -ItemType Directory -Path $measureEvidence -Force | Out-Null
if ($Mode -eq 'CaptureFixture') {
    $measureProfile = Assert-SyntheticProfile $ProfileRoot
    Assert-NoReparsePoint $measureProfile
    if (-not (Test-Path -LiteralPath (Join-Path $measureProfile 'GridTimerClientV2') -PathType Container)) {
        throw 'Generate the synthetic fixture at its final GridTimerClientV2 path before capture.'
    }
    $template = Join-Path $measureEvidence 'profile_template'
    if ((Test-Path -LiteralPath $template) -or (Test-Path -LiteralPath (Join-Path $measureEvidence 'fixture_receipt.json'))) {
        throw 'Fixture capture never overwrites an earlier template or receipt.'
    }
    Assert-NoActiveSample
    Copy-Tree $measureProfile $template
    $inventory = File-Inventory $template
    $receipt = [ordered]@{
        kind = 'isolated_synthetic_windows_startup'
        projectRoot = $measureProject
        profileRoot = $measureProfile
        templateRoot = $template
        capturedUtc = [DateTime]::UtcNow.ToString('o')
        inventory = $inventory
        note = 'Template bytes remain bound to profileRoot. Restore only to that exact root; do not rebind recovery evidence.'
    }
    Write-Json (Join-Path $measureEvidence 'fixture_receipt.json') $receipt
    $receipt | ConvertTo-Json -Depth 4
    exit
}

if ($Mode -eq 'RunSample') {
    $receipt = Read-Receipt
    $measureProfile = Assert-SyntheticProfile $receipt.profileRoot
    if ($ProfileRoot -and (Full-Path $ProfileRoot) -ne $measureProfile) { throw 'ProfileRoot differs from captured bound root.' }
    $measureBinary = (Resolve-Path -LiteralPath $Executable).Path
    if ([IO.Path]::GetExtension($measureBinary) -ne '.exe') { throw 'An explicit executable is required.' }
    $published = (Full-Path (Join-Path $measureProject 'release_artifacts/current')) + '\'
    if ($measureBinary.StartsWith($published, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Use an audited baseline/candidate copy; the live published client cannot be a test process.'
    }
    Assert-NoActiveSample
    if (@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and (Full-Path $_.ExecutablePath) -eq (Full-Path $measureBinary) }).Count -ne 0) {
        throw 'The selected executable already has a process. It has not been closed or terminated.'
    }
    $sample = Sample-Directory
    if (Test-Path -LiteralPath $sample) { throw "Sample already exists: $sample" }
    New-Item -ItemType Directory -Path $sample -Force | Out-Null
    if ($Stage -eq 'FreshProcess') {
        $actual = @(File-Inventory $receipt.templateRoot)
        if (($actual | ConvertTo-Json -Depth 5 -Compress) -ne ($receipt.inventory | ConvertTo-Json -Depth 5 -Compress)) {
            throw 'Captured template hash inventory has changed.'
        }
        # This is the only recursive delete: an exact receipt-bound synthetic
        # profile, never a computed user-data path, with no active owned sample.
        if (Test-Path -LiteralPath $measureProfile) {
            Assert-NoReparsePoint $measureProfile
            if ((Full-Path $measureProfile) -ne (Full-Path $receipt.profileRoot)) { throw 'Restore path check failed.' }
            Remove-Item -LiteralPath $measureProfile -Recurse -Force
        }
        Copy-Tree $receipt.templateRoot $measureProfile
    }
    $log = Join-Path $measureProfile 'GridTimerClientRuntime/windows_client.log'
    $binaryHash = (Get-FileHash -LiteralPath $measureBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $measureBinary
    $startInfo.WorkingDirectory = Split-Path -Parent $measureBinary
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $false
    $startInfo.EnvironmentVariables['LOCALAPPDATA'] = $measureProfile
    $memoryBefore = Get-CimInstance Win32_OperatingSystem | Select-Object TotalVisibleMemorySize, FreePhysicalMemory, TotalVirtualMemorySize, FreeVirtualMemory
    $startEpoch = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    $process = [Diagnostics.Process]::Start($startInfo)
    $process.Refresh()
    $processStart = [DateTimeOffset]::new($process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    $identity = [ordered]@{
        processId = $process.Id
        processStartUtcTicks = $process.StartTime.ToUniversalTime().Ticks
        processStartEpochMillis = $processStart
        launchCallEpochMillis = $startEpoch
        executable = $measureBinary
        executableSha256 = $binaryHash
        profileRoot = $measureProfile
        runtimeLog = $log
        sample = $sample
    }
    Write-Json (Join-Path $sample 'process_identity.json') $identity
    $firstWindow = $null; $firstTitledWindow = $null; $firstFrame = $null; $readyFrame = $null
    $peakWorking = [long]0; $peakPrivate = [long]0
    $idleStarted = $null; $idleCpuStart = $null; $idleWallStart = $null
    $lastProgress = -15.0; $lastEvents = @(); $failure = $null
    try {
        while ($true) {
            $process.Refresh()
            if ($process.HasExited) { $failure = "Process exited before idle measurement; exit code $($process.ExitCode)."; break }
            $peakWorking = [Math]::Max($peakWorking, $process.PeakWorkingSet64)
            $peakPrivate = [Math]::Max($peakPrivate, $process.PrivateMemorySize64)
            if ($null -eq $firstWindow -and $process.MainWindowHandle -ne [IntPtr]::Zero) {
                $firstWindow = [pscustomobject]@{ observedMillisAfterLaunchCall = $elapsed.Elapsed.TotalMilliseconds; title = $process.MainWindowTitle; handle = $process.MainWindowHandle.ToInt64() }
            }
            if ($null -eq $firstTitledWindow -and $process.MainWindowHandle -ne [IntPtr]::Zero -and $process.MainWindowTitle -match '^十倍率 Windows v') {
                $firstTitledWindow = [pscustomobject]@{
                    observedMillisAfterLaunchCall = $elapsed.Elapsed.TotalMilliseconds
                    observedMillisAfterProcessStart = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $processStart
                    title = $process.MainWindowTitle
                    handle = $process.MainWindowHandle.ToInt64()
                }
            }
            $lastEvents = @(Runtime-Events $log $startEpoch)
            if ($null -eq $firstFrame) { $firstFrame = $lastEvents | Where-Object message -match '^STARTUP_FIRST_FRAME\b' | Select-Object -First 1 }
            if ($null -eq $readyFrame) { $readyFrame = $lastEvents | Where-Object message -match '^STARTUP_READY_FRAME\b.*writable=true\b' | Select-Object -First 1 }
            $bad = $lastEvents | Where-Object message -match '^(PANIC|STARTUP_RECOVERY|EXIT startup_failure)\b' | Select-Object -First 1
            if ($bad) { $failure = $bad.message; break }
            if ($null -eq $readyFrame -and $elapsed.Elapsed.TotalSeconds -ge $StartupTimeoutSeconds) {
                $failure = 'No writable STARTUP_READY_FRAME marker before deadline. Window visibility/responding is not substituted for Ready.'; break
            }
            # Startup repeats do not need a second idle benchmark. Zero skips
            # idle sampling explicitly; it never emits a synthetic zero CPU.
            if ($readyFrame -and $IdleSeconds -eq 0 -and
                ($firstTitledWindow -or ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $readyFrame.epochMillis) -ge 2000)) { break }
            if ($readyFrame -and $null -eq $idleStarted -and ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - $readyFrame.epochMillis) -ge ($SettleSeconds * 1000)) {
                $idleStarted = $elapsed.Elapsed.TotalMilliseconds
                $idleCpuStart = $process.TotalProcessorTime.TotalMilliseconds
                $idleWallStart = [Diagnostics.Stopwatch]::StartNew()
            }
            if ($idleStarted -ne $null -and $idleWallStart.Elapsed.TotalSeconds -ge $IdleSeconds) { break }
            if ($elapsed.Elapsed.TotalSeconds - $lastProgress -ge 15) {
                Write-Output ('Sample {0} {1} {2}: PID {3}, elapsed {4:N1}s, Ready={5}, idle={6}' -f $Label, $Stage, $Trial, $process.Id, $elapsed.Elapsed.TotalSeconds, ($null -ne $readyFrame), ($null -ne $idleStarted))
                $lastProgress = $elapsed.Elapsed.TotalSeconds
            }
            Start-Sleep -Milliseconds $SampleIntervalMilliseconds
        }
        $process.Refresh()
        $idleWall = if ($idleWallStart) { $idleWallStart.Elapsed.TotalMilliseconds } else { $null }
        $idleCpu = if ($idleCpuStart -ne $null -and -not $process.HasExited) { $process.TotalProcessorTime.TotalMilliseconds - $idleCpuStart } else { $null }
        $report = [ordered]@{
            label = $Label; trial = $Trial; stage = $Stage
            passed = ($null -eq $failure); failure = $failure
            processIdentity = $identity
            firstWindowHandle = $firstWindow
            firstTitledWindow = $firstTitledWindow
            measurementFormatVersion = 2
            firstUiUpdateFromProcessStartMillis = if ($firstFrame) { $firstFrame.epochMillis - $processStart } else { $null }
            firstWritableFrameFromProcessStartMillis = if ($readyFrame) { $readyFrame.epochMillis - $processStart } else { $null }
            firstUiUpdateFromLaunchCallMillis = if ($firstFrame) { $firstFrame.epochMillis - $startEpoch } else { $null }
            firstWritableFrameFromLaunchCallMillis = if ($readyFrame) { $readyFrame.epochMillis - $startEpoch } else { $null }
            startupDeadlineSeconds = $StartupTimeoutSeconds
            settleSeconds = $SettleSeconds
            idleWallMillis = $idleWall; idleCpuMillis = $idleCpu
            idleCpuPercentOneCore = if ($idleWall -and $idleCpu -ne $null) { 100.0 * $idleCpu / $idleWall } else { $null }
            idleCpuPercentMachine = if ($idleWall -and $idleCpu -ne $null) { 100.0 * $idleCpu / $idleWall / [Environment]::ProcessorCount } else { $null }
            peakWorkingSetBytes = $peakWorking; sampledPeakPrivateBytes = $peakPrivate
            logicalProcessorCount = [Environment]::ProcessorCount
            systemMemoryBeforeKiB = $memoryBefore
            finalResponding = if (-not $process.HasExited) { $process.Responding } else { $false }
            actualStartupEvents = $lastEvents
            samplingIntervalMillis = $SampleIntervalMilliseconds
            notes = @('Process start includes initialization before the eframe creation context, including fonts.', 'OS window handle is visibility metadata; update markers are render-work boundaries, not a GPU-present timestamp.', 'Fresh process and restored data; OS disk caches are not flushed.', 'CPU is process CPU time over the measured idle interval; UI observer verification and normal close are separate.')
            requiresObservedUiVerification = $true
            requiresNormalClose = $true
        }
        Write-Json (Join-Path $sample 'measurement.json') $report
        $report | ConvertTo-Json -Depth 8
        if ($failure) { throw $failure }
    } finally { $process.Dispose() }
    # Leave this explicitly owned synthetic window open for sky observation and
    # a normal close. Never CloseMainWindow, inject input, or kill from this script.
    exit
}

if ($Mode -eq 'FinalizeExit') {
    $sample = Sample-Directory
    $identity = Get-Content -LiteralPath (Join-Path $sample 'process_identity.json') -Raw | ConvertFrom-Json
    $wait = [Diagnostics.Stopwatch]::StartNew()
    while ($null -ne (Find-OwnedProcess $identity)) {
        if ($wait.Elapsed.TotalSeconds -ge $ExitTimeoutSeconds) { throw 'Owned sample still runs. No process was terminated; use the observed computer-use window to close it.' }
        Start-Sleep -Milliseconds 100
    }
    $events = @(Runtime-Events $identity.runtimeLog $identity.launchCallEpochMillis)
    $clean = @($events | Where-Object message -eq 'EXIT clean').Count -gt 0
    $exitReport = [ordered]@{ cleanExit = $clean; processId = $identity.processId; uiObservation = $UiObservation; completedUtc = [DateTime]::UtcNow.ToString('o') }
    Write-Json (Join-Path $sample 'exit.json') $exitReport
    $exitReport | ConvertTo-Json
    if (-not $clean) { throw 'Process disappeared without EXIT clean; preserve the evidence and investigate before the next measurement.' }
    exit
}

if ($Mode -eq 'Summarize') {
    $rows = @(Get-ChildItem -LiteralPath (Join-Path $measureEvidence 'samples') -Filter measurement.json -Recurse -File | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json })
    $summaries = @($rows | Group-Object label, stage | ForEach-Object {
        $group = @($_.Group)
        $valid = @($group | Where-Object passed)
        $ready = @($valid.firstWritableFrameFromProcessStartMillis | Sort-Object)
        $frame = @($valid.firstUiUpdateFromProcessStartMillis | Sort-Object)
        [pscustomobject]@{
            label = $group[0].label; stage = $group[0].stage; samples = $group.Count; validSamples = $valid.Count
            cleanlyClosedSamples = @($valid | Where-Object {
                $exitFile = Join-Path $_.processIdentity.sample 'exit.json'
                (Test-Path -LiteralPath $exitFile) -and (Get-Content -LiteralPath $exitFile -Raw | ConvertFrom-Json).cleanExit
            }).Count
            medianFirstUiUpdateMillis = if ($frame.Count) { $frame[[int][Math]::Floor($frame.Count / 2)] } else { $null }
            medianFirstWritableFrameMillis = if ($ready.Count) { $ready[[int][Math]::Floor($ready.Count / 2)] } else { $null }
            meanIdleCpuPercentOneCore = ($valid | Measure-Object idleCpuPercentOneCore -Average).Average
            maxWorkingSetBytes = ($valid | Measure-Object peakWorkingSetBytes -Maximum).Maximum
            maxSampledPrivateBytes = ($valid | Measure-Object sampledPeakPrivateBytes -Maximum).Maximum
        }
    })
    $output = [ordered]@{ profile = 'Windows native GUI; exact fixture restoration; fresh processes; real process-start origin'; summaries = $summaries; samples = $rows }
    Write-Json (Join-Path $measureEvidence 'process_performance_summary.json') $output
    $summaries | Format-Table -AutoSize
}
