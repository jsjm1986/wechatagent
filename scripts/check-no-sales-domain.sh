#!/usr/bin/env bash
# scripts/check-no-sales-domain.sh
#
# Knowledge-base cleanup（spec：docs/superpowers/specs/2026-05-25-knowledge-base-cleanup-design.md）
# 的字面量防回归闸：扫描 src/ docs/ frontend/src/ 中的销售域残留命名。
# 0 命中即 exit 0；任一命中即列出所有 hit 并 exit 1。
#
# 排除：旧 spec 与旧 runbook（保留作历史档案）。
#   - docs/real-task-runbook.md
#   - docs/superpowers/specs/* （历史 spec）
#   - .kiro/specs/*           （历史 spec）

set -euo pipefail

# 同时覆盖 snake_case 与 camelCase；外加 sales[_-]positioning 这种独立命名。
PATTERN='customer_stage|customerStage|objection_type|objectionType|intent_level|intentLevel|forbidden_claims|forbiddenClaims|safe_claims|safeClaims|routing_card|routingCard|fact_risk|factRisk|pressure_risk|pressureRisk|product_accuracy|productAccuracy|sales[_-]positioning'

ROOTS=(src docs frontend/src)
EXCLUDES=(
    --glob='!docs/real-task-runbook.md'
    --glob='!docs/superpowers/specs/*'
    --glob='!.kiro/specs/*'
    # 历史 schema migration 文件冻结：seed 迁移只为旧库一次性重放服务，
    # 字符串字面量不可改；新库由 W3_001/W3_002 cleanup migration 统一 drop。
    --glob='!src/db/migrations.rs'
)

# 用 ripgrep；找不到则降级 grep -R。
if command -v rg >/dev/null 2>&1; then
    HITS=$(rg -ni "$PATTERN" "${EXCLUDES[@]}" "${ROOTS[@]}" 2>/dev/null | wc -l | tr -d ' ')
    if [ "$HITS" -eq 0 ]; then
        echo "[no-sales-domain] OK: 0 命中"
        exit 0
    fi
    echo "[no-sales-domain] FAIL: $HITS 处销售域残留:"
    rg -ni "$PATTERN" "${EXCLUDES[@]}" "${ROOTS[@]}"
    exit 1
fi

# 兜底（CI 没装 rg 时）—— grep -R 不支持 ripgrep glob，因此排除走 path filter。
HITS=$(grep -RnEi "$PATTERN" "${ROOTS[@]}" \
    --exclude-dir='specs' \
    --exclude='real-task-runbook.md' \
    --exclude='migrations.rs' \
    2>/dev/null | wc -l | tr -d ' ')
if [ "$HITS" -eq 0 ]; then
    echo "[no-sales-domain] OK: 0 命中（grep 兜底路径）"
    exit 0
fi
echo "[no-sales-domain] FAIL: $HITS 处销售域残留:"
grep -RnEi "$PATTERN" "${ROOTS[@]}" \
    --exclude-dir='specs' \
    --exclude='real-task-runbook.md' \
    --exclude='migrations.rs' \
    2>/dev/null
exit 1
