#!/usr/bin/env bash
# scripts/check-evolution-isolation.sh
#
# CI 演化器隔离 lint（agent-self-evolution M4 / W0 Task 1.4 / Requirement 1.6 / 8.4 / 9.4）。
#
# 演化器 (`src/evolution/`) 是独立 worker，永远不能与生产链路（gateway / outbox /
# MCP）共享代码路径——否则一次 shadow eval 误调 outbox 就会真的发出去。本脚本
# 静态扫描该目录是否引用了被禁的符号或字符串字面量；任意命中 → exit 1。
#
# 设计：
# - 全量扫描 `src/evolution/**/*.rs`（不只 git diff）。这种隔离一旦破坏后果严重，
#   不依赖 review 注意力，每次 CI 都重新校验全量。
# - 命中点带行号输出，便于 PR 上直接定位。
# - 排除注释行（开头 `//` 或 `///`）：注释里写禁词只是文档说明，不会真调。
#
# 使用：
#   scripts/check-evolution-isolation.sh

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
EVO_DIR="$ROOT/src/evolution"

if [ ! -d "$EVO_DIR" ]; then
    echo "[evolution-isolation] no src/evolution/ yet; skip."
    exit 0
fi

# 禁词（任一命中即违规）：
# - `crate::agent::gateway` / `crate::agent::outbox` / `crate::mcp::`
# - `agent_send_outbox.insert` / `mcp_client.send`
# - `run_user_operation_gateway` / `handle_managed_message` / `handle_follow_up_task`
FORBIDDEN_PATTERNS=(
    'crate::agent::gateway'
    'crate::agent::outbox'
    'crate::mcp::'
    'agent_send_outbox.insert'
    'mcp_client\.send'
    'run_user_operation_gateway'
    'handle_managed_message'
    'handle_follow_up_task'
)

VIOLATIONS=0

while IFS= read -r -d '' f; do
    # 跳过注释纯净的扫描：先去掉以 // 或 /// 开头的行（保留代码行）。
    # 用 awk 双管道：1) 排除注释行；2) grep 匹配禁词。
    for pat in "${FORBIDDEN_PATTERNS[@]}"; do
        hits=$(awk '!/^[[:space:]]*\/\//' "$f" | grep -n -E "$pat" || true)
        if [ -n "$hits" ]; then
            rel="${f#$ROOT/}"
            echo "[evolution-isolation] FAIL: $rel 引用了禁用符号 \"$pat\"："
            echo "$hits" | sed 's/^/    /'
            VIOLATIONS=$((VIOLATIONS + 1))
        fi
    done
done < <(find "$EVO_DIR" -type f -name '*.rs' -print0)

if [ "$VIOLATIONS" -gt 0 ]; then
    echo ""
    echo "[evolution-isolation] $VIOLATIONS violation(s)：演化器禁与 gateway/outbox/MCP 直接耦合。"
    echo "shadow replay 需保持短路；release/rollback 路径走 routes/evolution.rs，不调发送链。"
    exit 1
fi

echo "[evolution-isolation] ok: src/evolution/ 与 gateway / outbox / MCP 解耦。"
exit 0
