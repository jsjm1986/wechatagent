# scripts/check-evolution-isolation.ps1
#
# 与 check-evolution-isolation.sh 等价的 PowerShell 版本（M4 W0 Task 1.4）。
# 静态扫描 src/evolution/ 是否引用 gateway / outbox / MCP 等禁用符号。
# 任一命中 → exit 1。
#
# 用法：
#   pwsh scripts/check-evolution-isolation.ps1

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$root = git rev-parse --show-toplevel 2>$null
if (-not $root) { $root = (Get-Location).Path }
$evoDir = Join-Path $root "src/evolution"

if (-not (Test-Path $evoDir)) {
    Write-Host "[evolution-isolation] no src/evolution/ yet; skip."
    exit 0
}

$forbidden = @(
    'crate::agent::gateway',
    'crate::agent::outbox',
    'crate::mcp::',
    'agent_send_outbox\.insert',
    'mcp_client\.send',
    'run_user_operation_gateway',
    'handle_managed_message',
    'handle_follow_up_task'
)

$violations = 0
$files = Get-ChildItem -Recurse -Path $evoDir -Filter '*.rs'

foreach ($f in $files) {
    $lines = Get-Content -Path $f.FullName
    $rel = Resolve-Path -Relative -Path $f.FullName
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        # 跳过纯注释行（前导空白后以 // 开头）。
        if ($line -match '^\s*//') { continue }
        foreach ($pat in $forbidden) {
            if ($line -match $pat) {
                Write-Host "[evolution-isolation] FAIL: $rel:$($i+1) 引用禁用符号 `"$pat`"："
                Write-Host "    $line"
                $violations++
            }
        }
    }
}

if ($violations -gt 0) {
    Write-Host ""
    Write-Host "[evolution-isolation] $violations violation(s)：演化器禁与 gateway/outbox/MCP 直接耦合。"
    exit 1
}

Write-Host "[evolution-isolation] ok: src/evolution/ 与 gateway / outbox / MCP 解耦。"
exit 0
