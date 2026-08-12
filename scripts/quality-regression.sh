#!/usr/bin/env bash
# scripts/quality-regression.sh
#
# 金标质量回归环 v1 一键入口（优化线 C · C1b）。
#
# 链路：tests/fixtures/quality_gold/ 五类合成场景 × simulate_user_dialogue（shadow，
# 零真实发送）× 红线硬断言 × judge 打分（可选）→ target/quality_gold/run-*.jsonl ledger。
#
# 门槛（v1 软门）：红线违规即 fail；judge 分数只落 ledger 不 fail（累积 ≥3 次运行且
# 方差可接受后再由主会话决策升硬门）。
#
# 必填 env（被测 agent 的真实 LLM——不内置任何模型名，全部显式给定）：
#   REAL_LLM_API_KEY     上游 API key
#   REAL_LLM_BASE_URL    上游 base url
#   REAL_LLM_MODEL       模型标识（由使用者自填）
#   REAL_LLM_FORMAT      可选：openai（默认）| anthropic
#
# 可选 judge（异族评审，启用后三项必填）：
#   REAL_LLM_JUDGE=1
#   REAL_LLM_JUDGE_API_KEY / REAL_LLM_JUDGE_BASE_URL / REAL_LLM_JUDGE_MODEL
#   REAL_LLM_JUDGE_FORMAT  可选：openai（默认）| anthropic
#   QUALITY_GOLD_JUDGE_SAMPLES  可选：K 次采样取中位（默认 3）
#
# 子集回归（本地分钟级迭代）：
#   QUALITY_GOLD_CATEGORY=casual|objection|pressure|knowledge|boundary
#   QUALITY_GOLD_ID=<场景 id>    QUALITY_GOLD_LIMIT=<条数上限>
#   QUALITY_GOLD_FLOOR=<judge overall 软门下限，默认 6.0>
#
# 其它前置：Docker（testcontainers MongoDB）。

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT"

missing=()
for var in REAL_LLM_API_KEY REAL_LLM_BASE_URL REAL_LLM_MODEL; do
    if [ -z "${!var:-}" ]; then
        missing+=("$var")
    fi
done
if [ "${REAL_LLM_JUDGE:-}" = "1" ]; then
    for var in REAL_LLM_JUDGE_API_KEY REAL_LLM_JUDGE_BASE_URL REAL_LLM_JUDGE_MODEL; do
        if [ -z "${!var:-}" ]; then
            missing+=("$var")
        fi
    done
fi
if [ "${#missing[@]}" -gt 0 ]; then
    echo "[quality-regression] FAIL: 缺少必填环境变量：${missing[*]}" >&2
    echo "  金标回归环需要真实 LLM。设置示例：" >&2
    echo "    export REAL_LLM_API_KEY=...    # 上游 key" >&2
    echo "    export REAL_LLM_BASE_URL=...   # 上游 base url" >&2
    echo "    export REAL_LLM_MODEL=...      # 模型标识（使用者自填，不内置任何默认模型）" >&2
    echo "  可选 judge：export REAL_LLM_JUDGE=1 REAL_LLM_JUDGE_API_KEY=... REAL_LLM_JUDGE_BASE_URL=... REAL_LLM_JUDGE_MODEL=..." >&2
    exit 1
fi

if ! docker info >/dev/null 2>&1; then
    echo "[quality-regression] FAIL: Docker 不可用（testcontainers MongoDB 需要）。" >&2
    exit 1
fi

if [ "${REAL_LLM_JUDGE:-}" = "1" ]; then
    echo "[quality-regression] judge 已启用（K=${QUALITY_GOLD_JUDGE_SAMPLES:-3} 采样取中位）"
else
    echo "[quality-regression] judge skipped（未设 REAL_LLM_JUDGE=1；本次只跑红线硬断言）"
fi

echo "[quality-regression] running gold regression..."
cargo test --test quality_gold_regression -- --ignored --nocapture

LATEST=$(ls -t target/quality_gold/run-*.jsonl 2>/dev/null | head -n 1 || true)
if [ -z "$LATEST" ]; then
    echo "[quality-regression] FAIL: 未产生 ledger（target/quality_gold/run-*.jsonl）。" >&2
    exit 1
fi

echo
echo "[quality-regression] ledger: $LATEST"
echo "[quality-regression] summary（ledger 末行）："
tail -n 1 "$LATEST"
echo
echo "[quality-regression] done. 五类分布已随测试输出打印；ledger 累积 ≥3 次运行后可评估软门升级。"
