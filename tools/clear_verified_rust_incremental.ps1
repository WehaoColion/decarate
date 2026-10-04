# v0.0.1 - Clear only the two explicitly approved project incremental caches.
[CmdletBinding()]
param([ValidateSet('Inspect','Clean')][string]$Mode='Inspect')
$ErrorActionPreference='Stop'
$taskRoots=@('C:\gt\gridtimer-build\sourcegen\debug\incremental','C:\gt\gridtimer-build\windows-release\x86_64-pc-windows-msvc\debug\incremental')
$taskProjectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskEvidence=Join-Path $taskProjectRoot 'release_artifacts/verification/windows_v1.1.0.3/deep_optimization'
if(@(Get-CimInstance Win32_Process | Where-Object {$_.Name -match '^(cargo|rustc)\.exe$' -and $_.CommandLine -and $_.CommandLine.Contains('C:\gt\gridtimer-build')}).Count){throw 'Project compiler is active'}
$taskInventory=foreach($root in $taskRoots){
 if([IO.Path]::GetFullPath($root) -cne $root -or !$root.StartsWith('C:\gt\gridtimer-build\',[StringComparison]::OrdinalIgnoreCase)){throw 'Unexpected cache root'}
 if(!(Test-Path -LiteralPath $root)){continue}
 $parent=Get-Item -LiteralPath $root -Force
 while($parent -and $parent.FullName -ne 'C:\'){
  if(([long]$parent.Attributes -band [long][IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Cache path has a link'}
  $parent=$parent.Parent
 }
 $members=@((Get-Item -LiteralPath $root -Force)) + @(Get-ChildItem -LiteralPath $root -Recurse -Force)
 if(@($members | Where-Object {([long]$_.Attributes -band [long][IO.FileAttributes]::ReparsePoint) -ne 0}).Count){throw 'Cache has a link'}
 foreach($member in $members | Where-Object {!$_.PSIsContainer}){
  $resolved=[IO.Path]::GetFullPath($member.FullName)
  if(!$resolved.StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Cache file escaped root'}
  [pscustomobject]@{root=$root;path=$resolved;bytes=$member.Length;modified=$member.LastWriteTimeUtc.Ticks}
 }
}
$taskBefore=[IO.DriveInfo]::new('C:\').AvailableFreeSpace
if($Mode -eq 'Clean'){
 foreach($entry in $taskInventory){
  if($entry.root -cnotin $taskRoots -or !$entry.path.StartsWith($entry.root+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Cache scope changed'}
  $current=Get-Item -LiteralPath $entry.path -Force
  if($current.PSIsContainer -or $current.Length -ne $entry.bytes -or $current.LastWriteTimeUtc.Ticks -ne $entry.modified -or ([long]$current.Attributes -band [long][IO.FileAttributes]::ReparsePoint)){throw 'Cache file changed'}
  Remove-Item -LiteralPath $entry.path -Force
 }
}
$taskResult=[ordered]@{mode=$Mode;approvedRoots=$taskRoots;files=$taskInventory.Count;bytes=($taskInventory|Measure-Object bytes -Sum).Sum;availableBefore=$taskBefore;availableAfter=[IO.DriveInfo]::new('C:\').AvailableFreeSpace;completed=[DateTimeOffset]::UtcNow.ToString('o')}
$taskResult|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $taskEvidence ('incremental_cache_'+$Mode.ToLowerInvariant()+'.json')) -Encoding utf8
$taskResult|ConvertTo-Json -Depth 8
