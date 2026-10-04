# v1.0.3.10 - Verify cosmetic tolerance and all five property/canvas regression guards.
$ErrorActionPreference = 'Stop'
$auditProject = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$auditEvidence = Join-Path $auditProject 'release_artifacts/verification/v1.0.3.10-windows-audit'
$auditFiles = @{
    database = Join-Path $auditProject 'native/gridtimer_native/src/desktop/knowledge_database_ui.rs'
    canvas = Join-Path $auditProject 'native/gridtimer_native/src/desktop/knowledge_canvas.rs'
    client = Join-Path $auditProject 'native/gridtimer_native/src/bin/timer_windows_client.rs'
}
$auditOriginal = @{}
$auditText = @{}
$auditHashes = @{}
$auditEncoding = [Text.UTF8Encoding]::new($false)
foreach ($entry in $auditFiles.GetEnumerator()) {
    $auditOriginal[$entry.Key] = [IO.File]::ReadAllBytes($entry.Value)
    $auditText[$entry.Key] = [Text.Encoding]::UTF8.GetString($auditOriginal[$entry.Key])
    $auditHashes[$entry.Key] = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash
}
function Write-AuditVariant([string]$Key, [string]$Content) {
    [IO.File]::WriteAllBytes($auditFiles[$Key], $auditEncoding.GetBytes($Content))
}
$auditResult = [ordered]@{passed=$false;cosmeticPassed=$false;removedGuardsDetected=$false;restored=$false;sourceSha256=$auditHashes}
& (Join-Path $auditProject 'tools/windows.ps1') doctor *> (Join-Path $auditEvidence 'mutation_environment.log')
$auditArgs = @('run','stable-x86_64-pc-windows-msvc','cargo','test','--manifest-path',
    (Join-Path $auditProject 'native/gridtimer_native/Cargo.toml'),'--locked','--offline',
    '--features','desktop','--target','x86_64-pc-windows-msvc','--bin','timer_windows_client',
    'input_boundary_','--','--nocapture')
try {
    $cosmetic = $auditText.database.Replace('创建记录', '新增记录').Replace('ui.add_space(12.0);', 'ui.add_space(13.0);')
    if ($cosmetic -eq $auditText.database) { throw 'Cosmetic anchor missing' }
    Write-AuditVariant 'database' $cosmetic
    Write-Output 'Checking wording and spacing changes.'
    & rustup @auditArgs *> (Join-Path $auditEvidence 'mutation_cosmetic.log')
    $auditResult.cosmeticPassed = $LASTEXITCODE -eq 0
    if (-not $auditResult.cosmeticPassed) { throw 'Cosmetic changes failed business tests' }

    $mutant = $auditText.database.Replace('&& (input.text == input.source', '&& (true || input.text == input.source')
    $mutant = $mutant.Replace('let values = self.knowledge_form_values_with_pending_inputs(db);',
        'let values = Ok::<_, String>(self.desktop_ui.knowledge.form_values.clone());')
    if ($mutant -eq $auditText.database) { throw 'Input guard anchors missing' }
    Write-AuditVariant 'database' $mutant
    $canvas = $auditText.canvas.Replace('if ui.is_enabled() {', 'if true {')
    if ($canvas -eq $auditText.canvas) { throw 'Canvas guard anchor missing' }
    Write-AuditVariant 'canvas' $canvas
    $client = $auditText.client.Replace('self.flush_knowledge_property_inputs()', 'Ok(())')
    if ($client -eq $auditText.client) { throw 'Navigation guard anchor missing' }
    Write-AuditVariant 'client' $client
    Write-Output 'Checking that removing state guards fails the five regression tests.'
    & rustup @auditArgs *> (Join-Path $auditEvidence 'mutation_removed_guards.log')
    $mutantExit = $LASTEXITCODE
    $mutantLog = [IO.File]::ReadAllText((Join-Path $auditEvidence 'mutation_removed_guards.log'))
    $auditResult.removedGuardsDetected = $mutantExit -ne 0 -and
        $mutantLog.Contains('0 passed; 5 failed') -and $mutantLog.Contains('panicked at') -and
        -not $mutantLog.Contains('could not compile')
    if (-not $auditResult.removedGuardsDetected) { throw 'The five removed guards were not detected by business assertions' }
}
finally {
    foreach ($entry in $auditFiles.GetEnumerator()) {
        [IO.File]::WriteAllBytes($entry.Value, $auditOriginal[$entry.Key])
    }
    $auditResult.restored = @($auditFiles.GetEnumerator() | Where-Object {
        (Get-FileHash -LiteralPath $_.Value -Algorithm SHA256).Hash -ne $auditHashes[$_.Key]
    }).Count -eq 0
    $auditResult.passed = $auditResult.cosmeticPassed -and $auditResult.removedGuardsDetected -and $auditResult.restored
    $auditResult | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $auditEvidence 'mutation_result.json') -Encoding utf8
    if (-not $auditResult.restored) { throw 'Original source bytes were not restored' }
}
$auditResult | ConvertTo-Json -Depth 4
