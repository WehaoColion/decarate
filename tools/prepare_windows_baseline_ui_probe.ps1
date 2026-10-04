# Copy an immutable baseline source tree and add only first-update timestamps.
# This prepares evidence; it does not compile, launch, publish, or edit baseline.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SourceRoot,
    [Parameter(Mandatory = $true)][string]$ProbeRoot,
    [Parameter(Mandatory = $true)][string]$EvidenceRoot
)

$ErrorActionPreference = 'Stop'
$probeSource = (Resolve-Path -LiteralPath $SourceRoot).Path.TrimEnd('\', '/')
$probeDestination = [IO.Path]::GetFullPath($ProbeRoot).TrimEnd('\', '/')
$probeEvidence = [IO.Path]::GetFullPath($EvidenceRoot)
$probeUtf8 = [Text.UTF8Encoding]::new($false)
if ($probeDestination -eq $probeSource -or $probeDestination.StartsWith($probeSource + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'ProbeRoot must be a new sibling tree, not the preserved source or one of its children.'
}
if (Test-Path -LiteralPath $probeDestination) { throw 'Probe preparation never overwrites an existing directory.' }
$relativeMain = 'src/bin/timer_windows_client.rs'
$sourceMain = Join-Path $probeSource $relativeMain
$before = [IO.File]::ReadAllText($sourceMain)
$newline = if ($before.Contains("`r`n")) { "`r`n" } else { "`n" }
$entryNeedle = 'impl eframe::App for TimerWindowsClient {' + $newline + '    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {'
$exitNeedle = '        self.sync_rich_editor_webview(ctx, frame);' + $newline + '    }' + $newline + $newline + '    fn on_exit('
foreach ($needle in @($entryNeedle, $exitNeedle)) {
    if ([regex]::Matches($before, [regex]::Escape($needle)).Count -ne 1) {
        throw 'Baseline update structure differs; inspect it before changing probe placement.'
    }
}
$entryLines = @(
    '        // Isolated baseline measurement hook; no application behavior changes.',
    '        static STARTUP_PROBE_FIRST_UPDATE: AtomicBool = AtomicBool::new(true);',
    '        let startup_probe_first = STARTUP_PROBE_FIRST_UPDATE.swap(false, AtomicOrdering::Relaxed);',
    '        if startup_probe_first {',
    '            append_client_runtime_log("STARTUP_FIRST_FRAME probe=baseline");',
    '        }'
) -join $newline
$exitLines = @(
    '        self.sync_rich_editor_webview(ctx, frame);',
    '        if startup_probe_first {',
    '            append_client_runtime_log(&format!("STARTUP_READY_FRAME writable={} probe=baseline", self.workspace_persistence_ready));',
    '        }',
    '    }',
    '',
    '    fn on_exit('
) -join $newline
$after = $before.Replace($entryNeedle, $entryNeedle + $newline + $entryLines).Replace($exitNeedle, $exitLines)

New-Item -ItemType Directory -Path $probeDestination | Out-Null
Get-ChildItem -LiteralPath $probeSource -Force | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $probeDestination -Recurse
}
$probeMain = Join-Path $probeDestination $relativeMain
[IO.File]::WriteAllText($probeMain, $after, $probeUtf8)
New-Item -ItemType Directory -Path $probeEvidence -Force | Out-Null
$diff = & git diff --no-index -- $sourceMain $probeMain 2>&1
if ($LASTEXITCODE -notin @(0, 1)) { throw 'Could not record the probe diff.' }
[IO.File]::WriteAllText((Join-Path $probeEvidence 'baseline_ui_probe.diff'), ($diff -join "`n"), $probeUtf8)
$receipt = [ordered]@{
    kind = 'baseline_first_ui_update_probe'
    sourceRoot = $probeSource
    probeRoot = $probeDestination
    sourceSha256 = (Get-FileHash -LiteralPath $sourceMain -Algorithm SHA256).Hash.ToLowerInvariant()
    probeSha256 = (Get-FileHash -LiteralPath $probeMain -Algorithm SHA256).Hash.ToLowerInvariant()
    changedFile = $relativeMain
    hookCount = 2
    timeOrigin = 'Runtime log epoch timestamp minus OS process StartTime, measured by measure_windows_process_performance.ps1.'
    changes = @('One relaxed atomic flag check per update.', 'Two local runtime log writes in the first update: entry and completed editable frame.')
    exclusions = @('No startup loader changes.', 'No skipped validation.', 'No automatic exit or networking changes.', 'Not the unchanged official baseline binary; report instrumentation explicitly.')
    preparedUtc = [DateTime]::UtcNow.ToString('o')
}
[IO.File]::WriteAllText((Join-Path $probeEvidence 'baseline_ui_probe_receipt.json'), ($receipt | ConvertTo-Json -Depth 8), $probeUtf8)
$receipt | ConvertTo-Json -Depth 8
