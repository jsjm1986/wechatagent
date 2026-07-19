param(
    [string]$OutputPath = "docs/system-review/file-ledger.csv",
    [string]$BaselinePath = "docs/system-review/baseline.json"
)

$ErrorActionPreference = "Stop"
$repoRoot = (git rev-parse --show-toplevel).Trim()
if (-not $repoRoot) {
    throw "Not inside a Git repository"
}

$baselineFile = Join-Path $repoRoot $BaselinePath
if (-not (Test-Path -LiteralPath $baselineFile -PathType Leaf)) {
    throw "Baseline metadata missing: $BaselinePath"
}
$baseline = Get-Content -Raw -LiteralPath $baselineFile | ConvertFrom-Json
$baselineCommit = [string]$baseline.head_oid
if ([string]::IsNullOrWhiteSpace($baselineCommit)) {
    throw "baseline.json head_oid is missing"
}
$objectType = (git cat-file -t $baselineCommit).Trim()
if ($LASTEXITCODE -ne 0 -or $objectType -ne "commit") {
    throw "Baseline commit is unavailable: $baselineCommit"
}
$currentHead = (git rev-parse HEAD).Trim()
if ($currentHead -ne $baselineCommit) {
    throw "Refusing to build ledger from a different checkout: HEAD=$currentHead baseline=$baselineCommit"
}
$paths = @(git ls-tree -r --name-only $baselineCommit)
if ($LASTEXITCODE -ne 0) {
    throw "git ls-tree failed"
}

function Get-ContentClass([string]$path) {
    $lower = $path.ToLowerInvariant()
    if ($lower.EndsWith(".png")) { return "image_structural" }
    if ($lower.EndsWith(".pem")) { return "test_key_structural" }
    if ($lower.EndsWith(".b64")) { return "binary_fixture_structural" }
    if ($lower -eq "cargo.lock" -or $lower.EndsWith("/package-lock.json")) {
        return "dependency_lock_structural"
    }
    return "text_full_read"
}

function Get-InitialSubsystem([string]$path) {
    $p = $path.Replace('\', '/')
    switch -Regex ($p) {
        '^src/agent/' { return "agent" }
        '^src/auth/' { return "auth" }
        '^src/db/' { return "database" }
        '^src/evolution/' { return "evolution" }
        '^src/knowledge_wiki/' { return "knowledge_wiki" }
        '^src/knowledge_task/' { return "knowledge_task" }
        '^src/routes/knowledge/' { return "knowledge_routes" }
        '^src/routes/' { return "routes" }
        '^src/' { return "backend_core" }
        '^tests/' { return "tests" }
        '^frontend/src/' { return "frontend" }
        '^frontend/' { return "frontend_tooling" }
        '^scripts/e2e/' { return "e2e_scripts" }
        '^scripts/' { return "automation_scripts" }
        '^\.github/' { return "ci" }
        '^\.kiro/' { return "specifications" }
        '^docs/' { return "documentation" }
        '^website/' { return "website" }
        default { return "root_tooling" }
    }
}

$rows = foreach ($path in $paths) {
    $absolute = Join-Path $repoRoot $path
    if (-not (Test-Path -LiteralPath $absolute -PathType Leaf)) {
        throw "Baseline file missing from working tree: $path"
    }
    $item = Get-Item -LiteralPath $absolute
    $hash = (Get-FileHash -LiteralPath $absolute -Algorithm SHA256).Hash.ToLowerInvariant()
    $class = Get-ContentClass $path
    [pscustomobject][ordered]@{
        baseline_commit = $baselineCommit
        path = $path.Replace('\', '/')
        bytes = $item.Length
        sha256 = $hash
        content_class = $class
        subsystem = Get-InitialSubsystem $path
        review_status = if ($class -eq "text_full_read") { "pending_read" } else { "pending_verify" }
        batch_id = ""
        evidence = ""
        notes = ""
    }
}

$resolvedOutput = Join-Path $repoRoot $OutputPath
$parent = Split-Path -Parent $resolvedOutput
if (-not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Path $parent | Out-Null
}
$rows | Export-Csv -LiteralPath $resolvedOutput -NoTypeInformation -Encoding utf8

$uniquePaths = @($rows.path | Sort-Object -Unique)
if ($rows.Count -ne $paths.Count -or $uniquePaths.Count -ne $paths.Count) {
    throw "Ledger set mismatch: git=$($paths.Count), rows=$($rows.Count), unique=$($uniquePaths.Count)"
}

$grouped = $rows | Group-Object content_class | Sort-Object Name
Write-Host "baseline_commit=$baselineCommit"
Write-Host "baseline_pr=$($baseline.pull_request)"
Write-Host "tracked_files=$($paths.Count)"
foreach ($group in $grouped) {
    Write-Host "$($group.Name)=$($group.Count)"
}
