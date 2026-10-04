# v1.1.0.2 - Sample the isolated legal preparation probe without AI/network calls.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$EvidenceRoot
)
$ErrorActionPreference = 'Stop'
$memoryExecutable = (Resolve-Path -LiteralPath $Executable).Path
$memoryEvidence = [IO.Path]::GetFullPath($EvidenceRoot)
if ([IO.Path]::GetExtension($memoryExecutable) -ne '.exe') { throw 'An explicit test executable is required.' }
New-Item -ItemType Directory -Path $memoryEvidence -Force | Out-Null
$memoryLog = Join-Path $memoryEvidence 'legal_memory_stdout.log'
$memoryErrorLog = Join-Path $memoryEvidence 'legal_memory_stderr.log'
$env:DESKTOP_LEGAL_PREPARATION_MEMORY_OUTPUT = Join-Path $memoryEvidence 'legal_memory_ownership.json'
$env:DESKTOP_LEGAL_PREPARATION_MEMORY_HOLD_MS = '2000'
$memoryProcess = Start-Process -FilePath $memoryExecutable -ArgumentList @(
    'tests::desktop_performance_probe::legal_preparation_memory_reclamation_probe',
    '--ignored', '--exact', '--nocapture', '--test-threads=1'
) -WindowStyle Hidden -PassThru -RedirectStandardOutput $memoryLog -RedirectStandardError $memoryErrorLog
$memoryRows = [Collections.Generic.List[object]]::new()
$memoryClock = [Diagnostics.Stopwatch]::StartNew()
$memoryLastProgress = -15.0
try {
    while (-not $memoryProcess.HasExited) {
        $memoryProcess.Refresh()
        $memoryStage = 'starting'
        $memoryRaw = Get-Content -LiteralPath $memoryLog -Raw -ErrorAction SilentlyContinue
        if ($memoryRaw) {
            $memoryMatches = [regex]::Matches($memoryRaw, '(?m)^LEGAL_PREPARATION_MEMORY_STAGE (.+)$')
            if ($memoryMatches.Count) {
                $memoryStage = ($memoryMatches[$memoryMatches.Count - 1].Groups[1].Value | ConvertFrom-Json).stage
            }
        }
        $memoryRows.Add([pscustomobject]@{
            elapsedMillis = $memoryClock.Elapsed.TotalMilliseconds
            stage = $memoryStage
            workingSetBytes = $memoryProcess.WorkingSet64
            privateBytes = $memoryProcess.PrivateMemorySize64
            peakWorkingSetBytes = $memoryProcess.PeakWorkingSet64
        })
        if ($memoryClock.Elapsed.TotalSeconds - $memoryLastProgress -ge 15) {
            Write-Output ('Legal memory probe: PID {0}, {1:N1}s, {2}, private {3:N1} MiB' -f $memoryProcess.Id, $memoryClock.Elapsed.TotalSeconds, $memoryStage, ($memoryProcess.PrivateMemorySize64 / 1MB))
            $memoryLastProgress = $memoryClock.Elapsed.TotalSeconds
        }
        if ($memoryClock.Elapsed.TotalSeconds -gt 150) { throw 'Probe exceeded deadline; the exact owned PID is recorded in the output. No process was terminated.' }
        Start-Sleep -Milliseconds 100
    }
    $memoryProcess.WaitForExit()
    $memoryResult = [ordered]@{
        executable = $memoryExecutable
        executableSha256 = (Get-FileHash -LiteralPath $memoryExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        processId = $memoryProcess.Id
        exitCode = $memoryProcess.ExitCode
        sampleIntervalMillis = 100
        elapsedMillis = $memoryClock.Elapsed.TotalMilliseconds
        scope = 'Isolated synthetic 32 MiB preparation and report ownership; Windows process memory, no real user data or AI/network calls.'
        notes = @('Private Bytes may retain allocator pages after strong references are released.', 'Stage is inferred from the latest flushed probe marker; sampling does not instrument native allocator internals.')
        stages = @($memoryRows | Group-Object stage | ForEach-Object {
            [pscustomobject]@{
                stage = $_.Name
                samples = $_.Count
                maxWorkingSetBytes = ($_.Group | Measure-Object workingSetBytes -Maximum).Maximum
                maxPrivateBytes = ($_.Group | Measure-Object privateBytes -Maximum).Maximum
                finalPrivateBytes = $_.Group[-1].privateBytes
            }
        })
        samples = $memoryRows
    }
    $memoryResult | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $memoryEvidence 'legal_memory_process.json') -Encoding utf8
    Get-Content -LiteralPath $memoryLog -Tail 18
    if ($memoryProcess.ExitCode -ne 0) { throw 'Legal memory probe failed; inspect preserved stdout and stderr.' }
} finally { $memoryProcess.Dispose() }
