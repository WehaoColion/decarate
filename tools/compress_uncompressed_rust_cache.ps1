param()
$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$roots=@('C:\gt\gridtimer-build\audit\debug\incremental','C:\gt\gridtimer-build\sourcegen\debug\deps','C:\gt\gridtimer-build\sourcegen\debug\build','C:\gt\gridtimer-build\native\aarch64-linux-android\release','C:\gt\gridtimer-build\native\armv7-linux-androideabi\release','C:\gt\gridtimer-build\native\x86_64-linux-android\release','C:\gt\gridtimer-build\native\release')
if(@(Get-CimInstance Win32_Process | Where-Object {$_.Name -in @('cargo.exe','rustc.exe','gridtimer_packager.exe','gridtimer_sourcegen.exe')}).Count){throw 'Build active'}
Add-Type 'using System; using System.Runtime.InteropServices; public static class CachePhysicalSize { [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] public static extern uint GetCompressedFileSizeW(string name,out uint high); }'
$receipt=[ordered]@{mode='Retain uncompressed Rust cache files; LZX compression with byte verification';freeBefore=(Get-PSDrive C).Free;files=@()}
foreach($root in $roots){
    if((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Root is reparse point'}
    foreach($file in (Get-ChildItem -LiteralPath $root -File -Recurse | Where-Object {$_.Length -ge 64KB})){
        if($file.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'File is reparse point'}
        if(-not ([IO.Path]::GetFullPath($file.FullName)).StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Outside fixed cache root'}
        [uint32]$high=0; $low=[CachePhysicalSize]::GetCompressedFileSizeW($file.FullName,[ref]$high)
        if($low -eq [uint32]::MaxValue -and [Runtime.InteropServices.Marshal]::GetLastWin32Error() -ne 0){throw 'Cannot read physical size'}
        $physical=([uint64]$high -shl 32) + [uint64]$low
        if($physical -lt $file.Length){continue}
        $hash=(Get-FileHash -LiteralPath $file.FullName).Hash
        & "$env:WINDIR/System32/compact.exe" /C /F /EXE:LZX /Q $file.FullName | Out-Null
        if($LASTEXITCODE -ne 0){throw 'Compression failed'}
        $after=Get-Item -LiteralPath $file.FullName
        if($after.Length -ne $file.Length -or (Get-FileHash -LiteralPath $file.FullName).Hash -cne $hash){throw 'Cache content changed'}
        $receipt.files+=@{path=$file.FullName;bytes=$file.Length;sha256=$hash.ToLowerInvariant()}
        if($receipt.files.Count%50 -eq 0){Write-Output ('Retained cache files '+$receipt.files.Count)}
    }
}
$receipt.freeAfter=(Get-PSDrive C).Free
$receipt.completedAt=[datetimeoffset]::Now.ToString('o')
[IO.File]::WriteAllText((Join-Path $taskRoot 'release_artifacts/verification/windows_v1.1.0.5/uncompressed_cache_compression.json'),($receipt|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
Write-Output ('Retained '+$receipt.files.Count+' files; free GiB '+[Math]::Round($receipt.freeAfter/1GB,2))
