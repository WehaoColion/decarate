# v1.1.0.7 - Verify actual Windows AI consent and worker ownership without editing production.
[CmdletBinding()]
param([string]$EvidenceDirectory = 'C:\gt\windows-parity-20261005\ai_query_mutation')
$ErrorActionPreference = 'Stop'
$queryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$querySource = Join-Path $queryRoot 'native/gridtimer_native/src/desktop/ai_query_boundary.rs'
$queryUiSource = Join-Path $queryRoot 'native/gridtimer_native/src/desktop/ai_answer_ui.rs'
$querySourceHash = (Get-FileHash -LiteralPath $querySource -Algorithm SHA256).Hash.ToLowerInvariant()
$queryUiHash = (Get-FileHash -LiteralPath $queryUiSource -Algorithm SHA256).Hash.ToLowerInvariant()
$queryRaw = [IO.File]::ReadAllText($querySource)
$queryUiRaw = [IO.File]::ReadAllText($queryUiSource)
$queryCompiler = (Get-Command rustc -ErrorAction Stop).Source
$queryUtf8 = [Text.UTF8Encoding]::new($false)
function Replace-QueryAnchor([string]$Value, [string]$Anchor, [string]$Replacement) {
    if ([regex]::Matches($Value, [regex]::Escape($Anchor)).Count -ne 1) { throw 'Mutation anchor is missing or ambiguous' }
    $Value.Replace($Anchor, $Replacement)
}
$queryCases = @()
foreach ($case in @('baseline', 'cosmetic', 'removedAuthorizationBinding', 'removedSingleUse', 'removedReadiness', 'removedCancelledResultGuard', 'restored')) {
    $directory = Join-Path $EvidenceDirectory $case
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $source = $queryRaw
    $ui = $queryUiRaw
    $requiredFailure = ''
    switch ($case) {
        'cosmetic' {
            $ui = Replace-QueryAnchor $ui '"确认发送给 AI"' '"核对 AI 发送资料"'
            $ui = Replace-QueryAnchor $ui '.default_width(640.0)' '.default_width(680.0)'
        }
        'removedAuthorizationBinding' {
            $source = Replace-QueryAnchor $source 'consent.binding != *binding' 'false'
            $requiredFailure = 'tests::authorization_is_bound_and_can_only_start_once'
        }
        'removedSingleUse' {
            $source = Replace-QueryAnchor $source ' || consent.consumed' ''
            $requiredFailure = 'tests::authorization_is_bound_and_can_only_start_once'
        }
        'removedReadiness' {
            $source = Replace-QueryAnchor $source ' || !ready' ''
            $requiredFailure = 'tests::incomplete_or_unwritable_state_never_consumes_consent'
        }
        'removedCancelledResultGuard' {
            $source = Replace-QueryAnchor $source '!self.cancelled' 'true'
            $requiredFailure = 'tests::cancelled_request_holds_ownership_until_worker_finishes'
        }
    }
    $isolatedSource = Join-Path $directory 'ai_query_boundary.rs'
    [IO.File]::WriteAllText($isolatedSource, $source, $queryUtf8)
    [IO.File]::WriteAllText((Join-Path $directory 'ai_answer_ui.rs'), $ui, $queryUtf8)
    $executable = Join-Path $directory 'query_domain_tests.exe'
    $compileLog = Join-Path $directory 'compile.log'
    $testLog = Join-Path $directory 'tests.log'
    & $queryCompiler --edition 2021 --test $isolatedSource -o $executable *> $compileLog
    $compileExit = $LASTEXITCODE
    if ($compileExit -ne 0) { Get-Content -LiteralPath $compileLog -Tail 30; throw "Mutation must compile successfully: $case" }
    & $executable *> $testLog
    $testExit = $LASTEXITCODE
    $output = [IO.File]::ReadAllText($testLog)
    $expectedPass = !$requiredFailure
    $observed = $requiredFailure -and $output.Contains($requiredFailure + ' ... FAILED') -and $output.Contains('assertion failed')
    $passed = $(if ($expectedPass) { $testExit -eq 0 -and $output.Contains('4 passed; 0 failed') } else { $testExit -ne 0 -and $observed })
    $queryCases += [ordered]@{ case=$case; compiled=$compileExit -eq 0; testExit=$testExit; expectedPass=$expectedPass; requiredFailure=$requiredFailure; requiredFailureObserved=[bool]$observed; passed=[bool]$passed; sourceSha256=(Get-FileHash -LiteralPath $isolatedSource -Algorithm SHA256).Hash.ToLowerInvariant(); uiSha256=(Get-FileHash -LiteralPath (Join-Path $directory 'ai_answer_ui.rs') -Algorithm SHA256).Hash.ToLowerInvariant() }
    if (!$passed) { Get-Content -LiteralPath $testLog -Tail 40; throw "Unexpected domain mutation result: $case" }
    Write-Output "$case verified: compile $compileExit, tests $testExit"
}
if ((Get-FileHash -LiteralPath $querySource -Algorithm SHA256).Hash.ToLowerInvariant() -cne $querySourceHash -or (Get-FileHash -LiteralPath $queryUiSource -Algorithm SHA256).Hash.ToLowerInvariant() -cne $queryUiHash) { throw 'Actual production source changed during verification' }
$receipt = [ordered]@{ passed=$true; productionUnchanged=$true; sourcePath='native/gridtimer_native/src/desktop/ai_query_boundary.rs'; sourceSha256=$querySourceHash; uiPath='native/gridtimer_native/src/desktop/ai_answer_ui.rs'; uiSha256=$queryUiHash; tests=4; cases=$queryCases; hostOnly=$true; networkRequests=0; deviceOperations=0; scope='Actual Windows AI domain authorization, single use, cancellation and worker ownership. Cosmetic UI copy and width changes do not affect domain tests. No window or provider acceptance is claimed.'; checkedAt=[DateTimeOffset]::Now.ToString('o') }
[IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'receipt.json'), ($receipt | ConvertTo-Json -Depth 12), $queryUtf8)
Write-Output 'Windows AI authorization mutation verification passed'
