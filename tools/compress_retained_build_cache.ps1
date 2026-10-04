param([string]$Evidence = 'release_artifacts\verification\windows_v1.1.0.5\retained_cache_compression.json')
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$allowedRoots = @(
    'C:\gt\gridtimer-build\windows-release\x86_64-pc-windows-msvc\debug\deps',
    'C:\gt\gridtimer-build\windows-release\x86_64-pc-windows-msvc\release\deps',
    'C:\gt\gridtimer-build\sourcegen\debug\deps',
    'C:\gt\gridtimer-build\audit\debug\deps',
    'C:\gt\gridtimer-build\native\aarch64-linux-android\release\deps',
    'C:\gt\gridtimer-build\native\armv7-linux-androideabi\release\deps',
    'C:\gt\gridtimer-build\native\x86_64-linux-android\release\deps'
)
$cutoff = [datetime]::Now
$builders = @(Get-CimInstance Win32_Process | Where-Object {$_.Name -in @('cargo.exe','rustc.exe','gridtimer_packager.exe','gridtimer_sourcegen.exe')})
if($builders.Count){throw 'A build is active; cache compression is deferred.'}
$evidencePath = [IO.Path]::GetFullPath((Join-Path $taskRoot $Evidence))
if(-not $evidencePath.StartsWith($taskRoot+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Evidence path outside project'}
New-Item -ItemType Directory -Path (Split-Path -Parent $evidencePath) -Force | Out-Null
$receipt = [ordered]@{mode='Retain files and force LZX compression; no deletion or recycling';startedAt=[datetimeoffset]::Now.ToString('o');freeBefore=(Get-PSDrive C).Free;files=@();freeAfter=$null}
$selected = @()
foreach($root in $allowedRoots){
    $resolvedRoot = [IO.Path]::GetFullPath($root)
    $rootItem = Get-Item -LiteralPath $resolvedRoot
    if($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Cache root is a reparse point'}
    $selected += Get-ChildItem -LiteralPath $resolvedRoot -File | Where-Object {
        $_.Extension -in @('.pdb','.rlib','.exe','.dll') -and $_.Length -ge 8MB -and $_.LastWriteTime -lt $cutoff -and
        -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
    }
}
foreach($file in $selected){
    $parent = [IO.Path]::GetFullPath($file.DirectoryName)
    if($parent -notin $allowedRoots){throw 'Cache file outside fixed roots'}
    $beforeHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    & "$env:WINDIR\System32\compact.exe" /C /F /EXE:LZX /Q $file.FullName | Out-Null
    if($LASTEXITCODE -ne 0){throw "Compression failed: $($file.FullName)"}
    $after = Get-Item -LiteralPath $file.FullName
    $afterHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    if($afterHash -cne $beforeHash -or $after.Length -ne $file.Length){throw 'Cache bytes changed during compression'}
    $receipt.files += [ordered]@{path=$file.FullName;bytes=$file.Length;sha256=$afterHash.ToLowerInvariant()}
    if($receipt.files.Count % 20 -eq 0){Write-Output "Verified $($receipt.files.Count) retained cache files"}
}
$receipt.freeAfter = (Get-PSDrive C).Free
$receipt['completedAt'] = [datetimeoffset]::Now.ToString('o')
[IO.File]::WriteAllText($evidencePath,($receipt | ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
Write-Output "Retained $($receipt.files.Count) files; free space $([Math]::Round($receipt.freeAfter/1GB,2)) GiB"
