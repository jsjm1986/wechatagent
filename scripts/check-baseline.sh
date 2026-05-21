#!/usr/bin/env bash
# scripts/check-baseline.sh
#
# CI baseline verification (Linux / CI / bash).
# 关联：requirements.md R11.6 — 升级合并前 CI 必须跑：
#   - `cargo test --lib`：总通过数 >= 78（升级前基线），0 失败
#   - 4 个 PBT 文件累计通过数 >= 33（升级前基线 6+9+6+12），0 失败
#     (state_transition_pbt / memory_card_invariants / string_fact_risk_guard / llm_retry_jitter)
# 任一不达标即 exit 1。

set -euo pipefail

LIB_BASELINE=78
PBT_BASELINE=33

parse_passed() {
    # 仅在 "test result:" 汇总行里抽取 "<N> passed" 之前的数字并累加
    awk '/test result:/ {
        for (i = 1; i <= NF; i++) {
            if ($i == "passed;" && i > 1) { s += $(i - 1) + 0 }
        }
    } END { print s + 0 }' "$1"
}

parse_failed() {
    # 仅在 "test result:" 汇总行里抽取 "<N> failed" 之前的数字并累加
    awk '/test result:/ {
        for (i = 1; i <= NF; i++) {
            if ($i == "failed;" && i > 1) { s += $(i - 1) + 0 }
        }
    } END { print s + 0 }' "$1"
}

LIB_LOG=$(mktemp)
PBT_LOG=$(mktemp)
trap 'rm -f "$LIB_LOG" "$PBT_LOG"' EXIT

echo "[baseline] step 1/2: cargo test --lib ..."
# 不要因 cargo 非 0 退出码就提前终止；后面用 parse 出来的 passed/failed 数值判定
cargo test --lib 2>&1 | tee "$LIB_LOG" || true
LIB_PASSED=$(parse_passed "$LIB_LOG")
LIB_FAILED=$(parse_failed "$LIB_LOG")
echo "[baseline] lib summary: passed=$LIB_PASSED failed=$LIB_FAILED (need >= $LIB_BASELINE passed, 0 failed)"

if [ "$LIB_FAILED" -gt 0 ]; then
    echo "[baseline] FAIL: cargo test --lib has $LIB_FAILED failed test(s)"
    exit 1
fi
if [ "$LIB_PASSED" -lt "$LIB_BASELINE" ]; then
    echo "[baseline] FAIL: cargo test --lib only $LIB_PASSED passed (< baseline $LIB_BASELINE)"
    exit 1
fi

echo ""
echo "[baseline] step 2/2: cargo test 4 PBT files ..."
cargo test \
    --test state_transition_pbt \
    --test memory_card_invariants \
    --test string_fact_risk_guard \
    --test llm_retry_jitter 2>&1 | tee "$PBT_LOG" || true
PBT_PASSED=$(parse_passed "$PBT_LOG")
PBT_FAILED=$(parse_failed "$PBT_LOG")
echo "[baseline] pbt summary: passed=$PBT_PASSED failed=$PBT_FAILED (need >= $PBT_BASELINE passed, 0 failed)"

if [ "$PBT_FAILED" -gt 0 ]; then
    echo "[baseline] FAIL: PBT has $PBT_FAILED failed test(s)"
    exit 1
fi
if [ "$PBT_PASSED" -lt "$PBT_BASELINE" ]; then
    echo "[baseline] FAIL: PBT cumulative only $PBT_PASSED passed (< baseline $PBT_BASELINE)"
    exit 1
fi

echo ""
echo "baseline OK: lib=$LIB_PASSED, pbt=$PBT_PASSED"
exit 0
