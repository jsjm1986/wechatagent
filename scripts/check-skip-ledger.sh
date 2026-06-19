#!/usr/bin/env bash
# scripts/check-skip-ledger.sh
#
# R0.2：transient-skip 可观测化 + skip 率上限门。
#
# 背景：真模型测试用 `unwrap_or_skip_transient!` 宏在上游瞬时不可达（LlmUnavailable）
# 时 skip 而非 fail——本意是不让端点抖动把测试永久假红。但若**大面积 skip**，测试
# 实际啥也没验证却仍 conclusion=success，等于假绿。本脚本把"到底 skip 了多少"变成
# 可观测、可设上限的硬门：宏每次 skip 都 append 一行 JSON 到
# `${REAL_LLM_LEDGER:-target/real_llm_ledger}/skip_ledger.jsonl`，本脚本统计总数，
# 超过上限即 exit 1。
#
# 用法：在真模型 job 的测试 step 之后加一个 `if: always()` step 调本脚本。
#   REAL_LLM_MAX_SKIP=<上限，默认 6>  bash scripts/check-skip-ledger.sh
#
# 阈值说明：rsxermu 单点端点真实存在 5xx 抖动，正常一轮允许少量 skip；但全套
# 大面积 skip（如十几条全 http_5xx）说明这一轮根本没验证到能力，应红而非绿。
# 默认上限 6 是经验值（单 job 套件规模 ~3-16 测试），可按 job 经 env 调。

set -euo pipefail

LEDGER_DIR="${REAL_LLM_LEDGER:-target/real_llm_ledger}"
LEDGER="$LEDGER_DIR/skip_ledger.jsonl"
MAX_SKIP="${REAL_LLM_MAX_SKIP:-6}"

if [ ! -f "$LEDGER" ]; then
    echo "[skip-ledger] 无 skip 记录（$LEDGER 不存在）——本轮 0 skip，全部真跑。OK"
    exit 0
fi

SKIP_COUNT=$(wc -l < "$LEDGER" | tr -d ' ')
echo "[skip-ledger] 本轮 transient-skip 总数：$SKIP_COUNT（上限 $MAX_SKIP）"
echo "[skip-ledger] 按 kind 分布："
# 抽每行的 "kind":"xxx" 统计
grep -oE '"kind":"[a-z_0-9]+"' "$LEDGER" | sort | uniq -c || true
echo "[skip-ledger] 按 test 分布："
grep -oE '"test":"[^"]*"' "$LEDGER" | sort | uniq -c || true

if [ "$SKIP_COUNT" -gt "$MAX_SKIP" ]; then
    echo "[skip-ledger] FAIL：skip 数 $SKIP_COUNT > 上限 $MAX_SKIP。"
    echo "[skip-ledger] 大面积 skip = 这一轮真模型测试实际没验证到能力（多半端点持续抖动或配置错误）。"
    echo "[skip-ledger] 不当假绿，故 exit 1。端点恢复后重跑，或排查是否配错端点。"
    exit 1
fi

echo "[skip-ledger] OK：skip 数在上限内。"
exit 0
