# scripts/check-no-human-takeover.ps1
#
# CI 严禁词文本 lint（agent-autonomy-loop W6 / Task 7.7 / Requirement 2.7）的
# Windows / PowerShell 版本，与 .sh 等价：扫描 git diff 新增行在 src/agent/
# src/routes/ src/evolution/ frontend/src/ 下是否引入"human / 人工 / 接管 /
# takeover / hand-off"等违反"全自治、无人工接管"产品定位的字面量。
#
# 用法：
#   pwsh scripts/check-no-human-takeover.ps1
#   pwsh scripts/check-no-human-takeover.ps1 -Base main
#   pwsh scripts/check-no-human-takeover.ps1 -Base origin/main -HeadRef HEAD~1

[CmdletBinding()]
param(
    [string]$Base = "origin/main",
    [string]$HeadRef = "HEAD"
)

$ErrorActionPreference = "Stop"

$ScanDirs = @(
    "src/agent/",
    "src/routes/",
    "src/evolution/",
    "frontend/src/"
)

# 与 .sh 同一份正则：中文词 + 英文词 + 常见连字符变体。不区分大小写。
$ForbiddenPattern = '(human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工)'

# 列出 base..HEAD 之间在 ScanDirs 下变更的文件（仅文本文件，排除删除）。
$changed = git diff --name-only --diff-filter=ACMR "$Base..$HeadRef" -- $ScanDirs 2>$null
if (-not $changed) {
    Write-Host "[no-human-takeover] no changed files under scan dirs; ok."
    exit 0
}

$violations = 0
foreach ($f in $changed) {
    # 跳过非文本 + 跳过测试文件（test 写预期失败串很常见）。
    switch -Wildcard ($f) {
        "*.png"   { continue }
        "*.jpg"   { continue }
        "*.jpeg"  { continue }
        "*.gif"   { continue }
        "*.ico"   { continue }
        "*.woff"  { continue }
        "*.woff2" { continue }
        "*.ttf"   { continue }
        "*/tests/*" { continue }
        "tests/*" { continue }
        "*/__tests__/*" { continue }
        "*.test.*" { continue }
        "*.spec.*" { continue }
        # M4 W2：演化器自带禁词词典本身就需要列出全部禁词作为运行期黑名单
        # （`evolution::lint::FORBIDDEN_WORDS`），不应被字面量 lint 反向命中。
        "src/evolution/lint.rs" { continue }
    }
    if (-not (Test-Path $f)) { continue }

    # 取该文件 base..HEAD 的新增行（以 + 开头但跳过 diff header 的 +++ 行）。
    $diffLines = git diff "$Base..$HeadRef" -- $f
    $addedLines = @()
    foreach ($line in $diffLines) {
        if ($line -like "+++*") { continue }
        if ($line.StartsWith("+")) {
            $addedLines += $line.Substring(1)
        }
    }
    if ($addedLines.Count -eq 0) { continue }

    $hits = $addedLines | Select-String -Pattern $ForbiddenPattern -CaseSensitive:$false
    if ($hits) {
        Write-Host "[no-human-takeover] FAIL: $f 包含严禁词："
        foreach ($h in $hits) {
            Write-Host "    +$($h.Line)"
        }
        $violations++
    }
}

if ($violations -gt 0) {
    Write-Host ""
    Write-Host "[no-human-takeover] $violations file(s) violated 全自治、无人工接管 定位。"
    Write-Host "如确属合法引用（如 sunset 文档解释历史词），请把变更挪到不在扫描目录下的位置。"
    exit 1
}

Write-Host "[no-human-takeover] ok: 0 violations across $($changed.Count) changed file(s)."
exit 0
