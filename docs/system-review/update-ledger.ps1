param(
    [Parameter(Mandatory = $true)]
    [string]$BatchId,
    [Parameter(Mandatory = $true)]
    [string]$Evidence,
    [Parameter(Mandatory = $true)]
    [string[]]$Paths,
    [string]$LedgerPath = "docs/system-review/file-ledger.csv"
)

$ErrorActionPreference = "Stop"
$repoRoot = (git rev-parse --show-toplevel).Trim()
$ledgerFile = Join-Path $repoRoot $LedgerPath
$rows = @(Import-Csv -LiteralPath $ledgerFile)
$requested = @($Paths | ForEach-Object { $_.Replace('\', '/') } | Sort-Object -Unique)

foreach ($path in $requested) {
    $matches = @($rows | Where-Object path -eq $path)
    if ($matches.Count -ne 1) {
        throw "Expected exactly one ledger row for '$path', found $($matches.Count)"
    }
    $row = $matches[0]
    if ($row.content_class -ne "text_full_read") {
        throw "Refusing to mark non-text row as read_complete: $path ($($row.content_class))"
    }
    if ($row.review_status -notin @("pending_read", "in_progress", "read_complete")) {
        throw "Unexpected source status for '$path': $($row.review_status)"
    }
    $absolute = Join-Path $repoRoot $path
    # Compare Git-cleaned content with the frozen baseline blob. On Windows,
    # core.autocrlf may change raw worktree bytes without changing repository
    # content, so Get-FileHash alone produces false drift reports.
    $baselineBlob = (git rev-parse "$($row.baseline_commit):$path").Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($baselineBlob)) {
        throw "Unable to resolve baseline blob for '$path'"
    }
    $worktreeBlob = (git hash-object --path=$path $absolute).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($worktreeBlob)) {
        throw "Unable to hash worktree content for '$path'"
    }
    if ($worktreeBlob -ne $baselineBlob) {
        throw "Content drift while completing '$path': baseline_blob=$baselineBlob worktree_blob=$worktreeBlob"
    }
    $row.review_status = "read_complete"
    $row.batch_id = $BatchId
    $row.evidence = $Evidence
    $row.notes = "全文连续读取到 EOF；批次事实与交叉引用见 evidence 文档。"
}

$rows | Export-Csv -LiteralPath $ledgerFile -NoTypeInformation -Encoding utf8
Write-Host "batch_id=$BatchId"
Write-Host "marked_read_complete=$($requested.Count)"
