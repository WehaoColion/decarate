# v1.1.0.7 Windows - Test legal authorization mutations in copies without rewriting build inputs.
[CmdletBinding()]
param([string]$EvidenceDirectory = '')
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$source = Join-Path $project 'native/gridtimer_native/src/desktop/legal_workflow.rs'
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $EvidenceDirectory = Join-Path $project 'release_artifacts/verification/windows_v1.1.0.7/legal_workflow'
}
$EvidenceDirectory = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Force -Path $EvidenceDirectory | Out-Null
$before = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
$original = [IO.File]::ReadAllText($source)
$variants = [ordered]@{
    baseline = $original
    cosmetic = $original.Replace('整理资料并继续', '重新核对资料')
    removeAuthorization = $original.Replace("        if self.consumed {`n            return false;`n        }", '')
    removeConfigurationBinding = $original.Replace('readiness.can_send() && self.binding == *current', 'readiness.can_send()')
    removeReadiness = $original.Replace('readiness.can_send() && self.binding == *current', 'self.binding == *current')
    removePreparationGuard = $original.Replace('self.workspace_ready && !self.preparing && !self.running', 'self.workspace_ready')
}
foreach ($name in @('cosmetic', 'removeAuthorization', 'removeConfigurationBinding', 'removeReadiness', 'removePreparationGuard')) {
    if ($variants[$name] -eq $original) { throw "Missing mutation anchor: $name" }
}
$toolchain = 'stable-x86_64-pc-windows-msvc'
$sysroot = (& rustup run $toolchain rustc --print sysroot | Select-Object -Last 1).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot locate the Windows Rust toolchain' }
$linker = Join-Path $sysroot 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
$priorLib = [Environment]::GetEnvironmentVariable('LIB', 'Process')
$results = @()
try {
    $env:LIB = 'C:/tools/xwin/crt/lib/x86_64;C:/tools/xwin/sdk/lib/ucrt/x86_64;C:/tools/xwin/sdk/lib/um/x86_64'
    foreach ($entry in $variants.GetEnumerator()) {
        $caseSource = Join-Path $EvidenceDirectory ($entry.Key + '.rs')
        $caseExe = Join-Path $EvidenceDirectory ($entry.Key + '.exe')
        $compileLog = Join-Path $EvidenceDirectory ($entry.Key + '_compile.log')
        $testLog = Join-Path $EvidenceDirectory ($entry.Key + '_test.log')
        [IO.File]::WriteAllText($caseSource, $entry.Value, [Text.UTF8Encoding]::new($false))
        & rustup run $toolchain rustc --edition 2021 --test --target x86_64-pc-windows-msvc -C "linker=$linker" $caseSource -o $caseExe *> $compileLog
        $compileExit = $LASTEXITCODE
        if ($compileExit -ne 0) { throw "Mutation did not compile: $($entry.Key). See $compileLog" }
        & $caseExe --nocapture *> $testLog
        $testExit = $LASTEXITCODE
        $log = [IO.File]::ReadAllText($testLog)
        $expectedPass = $entry.Key -in @('baseline', 'cosmetic')
        $assertionFailed = $log.Contains('panicked at') -and $log.Contains('assertion failed') -and $log.Contains('test result: FAILED')
        $passed = if ($expectedPass) { $testExit -eq 0 -and $log.Contains('5 passed; 0 failed') } else { $testExit -ne 0 -and $assertionFailed }
        $results += [pscustomobject]@{case=$entry.Key;compileExit=$compileExit;testExit=$testExit;expectedPass=$expectedPass;assertionFailed=$assertionFailed;passed=$passed}
        if (-not $passed) { throw "Mutation verification failed: $($entry.Key). See $testLog" }
    }
} finally {
    [Environment]::SetEnvironmentVariable('LIB', $priorLib, 'Process')
    $after = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    [pscustomobject]@{passed=($results.Count -eq $variants.Count -and @($results | Where-Object { !$_.passed }).Count -eq 0 -and $before -eq $after);sourceSha256=$before;productionSourceUnchanged=($before -eq $after);domainTestCount=5;cases=$results} |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'mutation_result.json') -Encoding utf8
    if ($before -ne $after) { throw 'Production source changed during mutation verification' }
}
Write-Output 'Windows legal send mutation verification passed: 5 domain specifications, 6 cases, unchanged production source.'
