# v1.1.0.3 - Keep one verified desktop entry for the latest committed Windows release.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ProjectRoot,
    [ValidateSet('Apply', 'Check', 'Test')][string]$Mode = 'Check'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-NormalizedPath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathRooted($Path)) { return $null }
    if ($Path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { $Path = '\\' + $Path.Substring(8) }
    elseif ($Path.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { $Path = $Path.Substring(4) }
    return [IO.Path]::GetFullPath($Path).TrimEnd('\')
}

function Test-OwnedTarget([string]$Target, [string]$Arguments, [string]$Root) {
    $normalized = Get-NormalizedPath $Target
    if ($null -eq $normalized -or -not [string]::IsNullOrWhiteSpace($Arguments)) { return $false }
    $prefix = (Get-NormalizedPath $Root) + '\'
    if (-not $normalized.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { return $false }
    $name = [IO.Path]::GetFileName($normalized)
    return $name -eq 'TenRate_Desktop_Launcher.exe' -or $name -eq 'timer_windows_client.exe' -or
        $name -match '^grid_timer_windows_client_v[0-9]+(?:\.[0-9]+){2,3}(?:-[a-zA-Z0-9]+)*\.exe$'
}

function Get-ShortcutRecord($Wsh, [string]$Path) {
    $item = Get-Item -LiteralPath $Path
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Shortcut cannot be a reparse point: $Path" }
    $link = $Wsh.CreateShortcut($Path)
    return [pscustomobject]@{
        path = $Path; target = $link.TargetPath; arguments = $link.Arguments
        icon = $link.IconLocation; workingDirectory = $link.WorkingDirectory; description = $link.Description
    }
}

function Assert-StableShortcut($Wsh, [string]$Path, [string]$Target) {
    $record = Get-ShortcutRecord $Wsh $Path
    if ((Get-NormalizedPath $record.target) -ne (Get-NormalizedPath $Target) -or
        $record.arguments -ne '' -or $record.icon -ne "$Target,0" -or
        (Get-NormalizedPath $record.workingDirectory) -ne (Get-NormalizedPath ([IO.Path]::GetDirectoryName($Target)))) {
        throw "The stable desktop shortcut did not preserve its target and embedded icon: $Path"
    }
    return $record
}

function Write-StableShortcut($Wsh, [string]$Path, [string]$Target, [string]$BackupRoot, [string]$Root) {
    if (Test-Path -LiteralPath $Path) {
        $existing = Get-ShortcutRecord $Wsh $Path
        if (-not (Test-OwnedTarget $existing.target $existing.arguments $Root)) {
            throw "The fixed shortcut name belongs to another application: $Path"
        }
    }
    $pending = Join-Path ([IO.Path]::GetDirectoryName($Path)) ('TenRate_pending_' + [Guid]::NewGuid().ToString('N') + '.lnk')
    $link = $Wsh.CreateShortcut($pending)
    $link.TargetPath = $Target
    $link.Arguments = ''
    $link.WorkingDirectory = [IO.Path]::GetDirectoryName($Target)
    $link.IconLocation = "$Target,0"
    $link.Description = '十倍率：专注计时、积累知识、把每一天变成长期进步。始终打开最新正式版。'
    $link.Save()
    $null = Assert-StableShortcut $Wsh $pending $Target
    if (Test-Path -LiteralPath $Path) {
        # File.Replace preserves the destination name. Retain the former shortcut for recovery.
        $backup = Join-Path $BackupRoot ('replaced_' + [Guid]::NewGuid().ToString('N') + '.lnk')
        [IO.File]::Replace($pending, $Path, $backup)
    } else {
        [IO.File]::Move($pending, $Path)
    }
    return Assert-StableShortcut $Wsh $Path $Target
}

function Repair-DesktopEntries($Wsh, [string[]]$Desktops, [string]$Root, [string]$Target, [string]$BackupRoot, [bool]$Apply) {
    $userDesktop = Get-NormalizedPath $Desktops[0]
    $canonical = Join-Path $userDesktop 'TenRate.lnk'
    $records = @(foreach ($directory in $Desktops | Select-Object -Unique) {
        if (Test-Path -LiteralPath $directory -PathType Container) {
            foreach ($file in Get-ChildItem -LiteralPath $directory -Filter '*.lnk' -File) { Get-ShortcutRecord $Wsh $file.FullName }
        }
    })
    $owned = @($records | Where-Object { Test-OwnedTarget $_.target $_.arguments $Root })
    $removed = @()
    if ($Apply) {
        [IO.Directory]::CreateDirectory($BackupRoot) | Out-Null
        $records | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $BackupRoot 'desktop_before.json') -Encoding UTF8
        $stable = Write-StableShortcut $Wsh $canonical $Target $BackupRoot $Root
        foreach ($record in $owned) {
            if ($record.path -eq $canonical) { continue }
            # Re-read immediately before removal, including ownership and the actual desktop directory.
            $fresh = Get-ShortcutRecord $Wsh $record.path
            $parent = Get-NormalizedPath ([IO.Path]::GetDirectoryName($fresh.path))
            if (-not ($Desktops | Where-Object { (Get-NormalizedPath $_) -eq $parent }) -or
                -not (Test-OwnedTarget $fresh.target $fresh.arguments $Root)) { throw "Shortcut ownership changed: $($fresh.path)" }
            $backup = Join-Path $BackupRoot ('removed_' + [Guid]::NewGuid().ToString('N') + '.lnk')
            [IO.File]::Copy($fresh.path, $backup, $false)
            if ((Get-FileHash -LiteralPath $fresh.path -Algorithm SHA256).Hash -ne
                (Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash) { throw 'Shortcut backup failed' }
            Remove-Item -LiteralPath $fresh.path
            $removed += $fresh.path
        }
    } else {
        $stable = Assert-StableShortcut $Wsh $canonical $Target
        if ($owned.Count -ne 1 -or $owned[0].path -ne $canonical) { throw 'The desktop must contain exactly one owned client entry' }
    }
    $unrelated = @($records | Where-Object { -not (Test-OwnedTarget $_.target $_.arguments $Root) })
    foreach ($record in $unrelated) {
        $fresh = Get-ShortcutRecord $Wsh $record.path
        if ($fresh.target -ne $record.target -or $fresh.arguments -ne $record.arguments -or $fresh.icon -ne $record.icon) {
            throw "Unrelated shortcut changed: $($record.path)"
        }
    }
    return [pscustomobject]@{ shortcut = $stable; removed = $removed; untouched = @($unrelated | ForEach-Object { $_.path }); backupRoot = $BackupRoot }
}

if ($Mode -eq 'Test') {
    $fixture = Join-Path ([IO.Path]::GetTempPath()) ('TenRateDesktopEntry_' + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($fixture) | Out-Null
    $user = Join-Path $fixture 'user'
    $public = Join-Path $fixture 'public'
    $root = Join-Path $fixture 'project'
    foreach ($directory in @($user, $public, $root)) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
    $wsh = New-Object -ComObject WScript.Shell
    $target = Join-Path $root 'release_artifacts\desktop_entry\TenRate_Desktop_Launcher.exe'
    $foreign = Join-Path $fixture 'project_other\grid_timer_windows_client_v1.1.0.1.exe'
    foreach ($spec in @(
        @{ path = (Join-Path $user 'old_version.lnk'); target = (Join-Path $root 'old_exes\grid_timer_windows_client_v1.1.0.1.exe') },
        @{ path = (Join-Path $public 'old_public.lnk'); target = $target },
        @{ path = (Join-Path $user 'unrelated.lnk'); target = $foreign }
    )) { $link=$wsh.CreateShortcut($spec.path); $link.TargetPath=$spec.target; $link.Save() }
    $foreignHash = (Get-FileHash -LiteralPath (Join-Path $user 'unrelated.lnk')).Hash
    $result = Repair-DesktopEntries $wsh @($user,$public) $root $target (Join-Path $fixture 'backup') $true
    if ($result.removed.Count -ne 2 -or (Get-FileHash -LiteralPath (Join-Path $user 'unrelated.lnk')).Hash -ne $foreignHash) { throw 'Cleanup scope test failed' }
    $null = Repair-DesktopEntries $wsh @($user,$public) $root $target (Join-Path $fixture 'backup') $false
    $again = Repair-DesktopEntries $wsh @($user,$public) $root $target (Join-Path $fixture 'backup2') $true
    if ($again.removed.Count -ne 0) { throw 'Stable shortcut reuse failed' }
    if (Test-OwnedTarget $foreign '' $root) { throw 'Project prefix boundary was not enforced' }
    if (Test-OwnedTarget $target '--sync-supervisor' $root) { throw 'A service entry must not be deleted as a client entry' }
    $fixed = Join-Path $user 'TenRate.lnk'
    $link=$wsh.CreateShortcut($fixed); $link.TargetPath=$foreign; $link.Save()
    $refused=$false
    try { $null=Repair-DesktopEntries $wsh @($user,$public) $root $target (Join-Path $fixture 'backup3') $true } catch { $refused=$true }
    if (-not $refused -or (Get-ShortcutRecord $wsh $fixed).target -ne $foreign) { throw 'Foreign fixed name was overwritten' }
    [pscustomobject]@{ok=$true; scopeTests=5; fixture=$fixture} | ConvertTo-Json
    exit 0
}

$root = Get-NormalizedPath $ProjectRoot
if ($null -eq $root -or -not (Test-Path -LiteralPath (Join-Path $root 'native/gridtimer_native/Cargo.toml') -PathType Leaf)) { throw 'Invalid application project root' }
$manifestPath = Join-Path $root 'release_artifacts/current/release_manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.productId -ne 'gridtimer' -or $manifest.version -notmatch '^[0-9]+(?:\.[0-9]+){2,3}$') { throw 'Invalid committed release identity' }
if (Test-Path -LiteralPath (Join-Path $root 'release_artifacts/.release-transaction-v1.json')) { throw 'Release transaction is not finalized' }
$client = @($manifest.files | Where-Object role -eq 'windows_client')
if ($client.Count -ne 1 -or $client[0].file_name -ne "grid_timer_windows_client_v$($manifest.version).exe") { throw 'Manifest client identity does not match its version' }
$clientPath = Join-Path $root ('release_artifacts/current/' + $client[0].file_name)
if ((Get-FileHash -LiteralPath $clientPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $client[0].sha256) { throw 'Published client hash does not match its manifest' }
$target = Join-Path $root 'release_artifacts/desktop_entry/TenRate_Desktop_Launcher.exe'
# The Rust launcher writes UTF-8. Read it explicitly rather than using PowerShell 5's OEM decoder.
$launcherInfo = New-Object Diagnostics.ProcessStartInfo
$launcherInfo.FileName = $target
$launcherInfo.Arguments = '--resolve-only'
$launcherInfo.UseShellExecute = $false
$launcherInfo.CreateNoWindow = $true
$launcherInfo.RedirectStandardOutput = $true
$launcherInfo.RedirectStandardError = $true
$launcherInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
$launcherInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
$launcherProcess = [Diagnostics.Process]::Start($launcherInfo)
try {
    if (-not $launcherProcess.WaitForExit(30000)) {
        # This process is the resolving helper just created by this script, never the user client.
        $launcherProcess.Kill()
        throw 'Stable launcher resolution exceeded 30 seconds'
    }
    $resolved = $launcherProcess.StandardOutput.ReadToEnd().Trim()
    $resolveError = $launcherProcess.StandardError.ReadToEnd().Trim()
    if ($launcherProcess.ExitCode -ne 0 -or (Get-NormalizedPath $resolved) -ne (Get-NormalizedPath $clientPath)) {
        throw "Stable launcher does not resolve the latest committed client: $resolveError"
    }
} finally { $launcherProcess.Dispose() }
$desktops = @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('CommonDesktopDirectory'))
$backupRoot = Join-Path $root ('release_artifacts/verification/windows_v' + $manifest.version + '/desktop_entry/' + [Guid]::NewGuid().ToString('N'))
$wsh = New-Object -ComObject WScript.Shell
$result = Repair-DesktopEntries $wsh $desktops $root $target $backupRoot ($Mode -eq 'Apply')
$result | Add-Member -NotePropertyName windowsVersion -NotePropertyValue $manifest.version
$result | Add-Member -NotePropertyName resolvedClient -NotePropertyValue $clientPath
if ($Mode -eq 'Apply') {
    $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $backupRoot 'desktop_after.json') -Encoding UTF8
    # Let Explorer invalidate this entry without restarting it or flushing the user's full icon cache.
    if (-not ('TenRateShortcutNotify' -as [type])) {
    Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class TenRateShortcutNotify { [DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern void SHChangeNotify(uint e,uint f,string p,IntPtr q); }'
    }
    [TenRateShortcutNotify]::SHChangeNotify(0x08000000, 0, $null, [IntPtr]::Zero)
}
$result | ConvertTo-Json -Depth 6