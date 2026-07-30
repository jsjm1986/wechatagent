param(
    [string]$LedgerPath = "docs/system-review/file-ledger.csv",
    [string]$BaselinePath = "docs/system-review/baseline.json",
    [switch]$RequireComplete
)

$ErrorActionPreference = "Stop"
$repoRoot = (git rev-parse --show-toplevel).Trim()
$baseline = Get-Content -Raw -LiteralPath (Join-Path $repoRoot $BaselinePath) | ConvertFrom-Json
$baselineCommit = [string]$baseline.head_oid
$ledger = Import-Csv -LiteralPath (Join-Path $repoRoot $LedgerPath)
$gitPaths = @(git ls-tree -r --name-only $baselineCommit | ForEach-Object { $_.Replace('\', '/') })
$ledgerPaths = @($ledger.path)
$wrongCommit = @($ledger | Where-Object { $_.baseline_commit -ne $baselineCommit })
if ($wrongCommit.Count) {
    throw "Ledger contains $($wrongCommit.Count) row(s) from a non-baseline commit"
}

$missing = @($gitPaths | Where-Object { $_ -notin $ledgerPaths })
$extra = @($ledgerPaths | Where-Object { $_ -notin $gitPaths })
$duplicates = @($ledger | Group-Object path | Where-Object Count -ne 1)
if ($missing.Count -or $extra.Count -or $duplicates.Count) {
    throw "Ledger set mismatch: missing=$($missing.Count), extra=$($extra.Count), duplicate_groups=$($duplicates.Count)"
}

# Compare the complete working tree to the frozen commit in one Git process.
# Ignore only a checkout-added CR at end-of-line. Real edits, staged changes,
# and deletions remain visible and still fail the frozen-baseline check.
$drift = @(git diff --ignore-cr-at-eol --name-only $baselineCommit -- | ForEach-Object { $_.Replace('\', '/') })
if ($LASTEXITCODE -ne 0) {
    throw "git diff failed while checking baseline drift"
}
if ($drift.Count) {
    $drift | Select-Object -First 20 | ForEach-Object { Write-Error "changed:$_" }
    throw "Baseline drift detected in $($drift.Count) file(s)"
}

$allowed = @("pending_read", "pending_verify", "in_progress", "read_complete", "verified_non_text")
$invalidStatus = @($ledger | Where-Object { $_.review_status -notin $allowed })
if ($invalidStatus.Count) {
    throw "Invalid review_status rows: $($invalidStatus.Count)"
}

if ($RequireComplete) {
    $incomplete = @($ledger | Where-Object { $_.review_status -notin @("read_complete", "verified_non_text") })
    if ($incomplete.Count) {
        throw "Review incomplete: $($incomplete.Count) file(s) remain"
    }
    $withoutEvidence = @($ledger | Where-Object { [string]::IsNullOrWhiteSpace($_.batch_id) -or [string]::IsNullOrWhiteSpace($_.evidence) })
    if ($withoutEvidence.Count) {
        throw "Completed rows without batch/evidence: $($withoutEvidence.Count)"
    }
}

$statusGroups = $ledger | Group-Object review_status | Sort-Object Name
Write-Host "ledger_files=$($ledger.Count)"
Write-Host "baseline_pr=$($baseline.pull_request)"
Write-Host "baseline_commit=$baselineCommit"
Write-Host "baseline_drift=0"
foreach ($group in $statusGroups) {
    Write-Host "$($group.Name)=$($group.Count)"
}
