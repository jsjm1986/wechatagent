#!/usr/bin/env bash
# scripts/check-baseline.sh
#
# CI baseline verification (Linux / CI / bash).
# 关联：requirements.md R11.6 — 升级合并前 CI 必须跑：
#   - `cargo test --lib`：总通过数 >= 350（knowledge cleanup 后基线），0 失败
#   - 4 个 PBT 文件累计通过数 >= 33（升级前基线 6+9+6+12），0 失败
#     (state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter)
# 任一不达标即 exit 1。
#
# step 2:cargo check --tests (RUSTFLAGS=-D warnings) —— 把 tests/ 目录纳入
# -D warnings 编译检查,堵住 cargo test --lib 不编译 tests/ 的 unused import 盲区。
#
# G-后续Ⅱ/4：可选 step 4 —— Docker available 时跑无 LLM/MCP 的知识库集成
# 测试 `wiki_gap_signals_3kinds`（3 个 #[ignore] 测试，纯 testcontainers
# Mongo 路径），把"运维向"知识库回归引入 CI 而不阻塞本地开发：
#   - 环境无 docker / DOCKER_AVAILABLE!=1：step 4 跳过，不计入门槛
#   - 环境有 docker：3 个测试必须 pass，0 fail（任意 fail → exit 1）。
# 设计原因：
#   - 这 3 个测试不消耗 LLM/MCP，CI 跑无外部成本；
#   - 选这一个文件而非全量 #[ignore] 因为其它 ignore 测试要么调 LLM/MCP，
#     要么对运行时间/网络有依赖。后续可逐步扩；
#   - 把 testcontainers 触发硬编码在 baseline 脚本里也避免出现"CI 在跑但
#     人不知道"的暗黑路径——失败时报错信息明确指向本测试。

set -euo pipefail

LIB_BASELINE=350
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

echo "[baseline] step 1/3: cargo test --lib ..."
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
echo "[baseline] step 2/3: cargo check --tests (RUSTFLAGS=-D warnings) ..."
# 把 tests/ 目录纳入 -D warnings 编译检查。只编译不运行,不增磁盘压力、不依赖 Docker。
# 目的:堵住 `cargo test --lib` 不编译 tests/ 导致的 unused import / warning 盲区
# (见 domain_profile_e2e.rs unused import 漏网事件)。
# 注意:必须在 RUSTFLAGS=-D warnings 下,且本脚本顶层的 set -euo pipefail 会让
# cargo check 非 0 退出码直接终止脚本——这正是期望行为(编译警告即合并门违例)。
RUSTFLAGS="-D warnings" cargo check --tests --quiet
echo "[baseline] cargo check --tests OK"

echo ""
echo "[baseline] step 3/3: cargo test 4 PBT files (run individually to isolate failures) ..."
PBT_PASSED=0
PBT_FAILED=0
PBT_LOG=$(mktemp)
for test_file in state_transition_pbt memory_card_invariants wiki_chunk_revision_pbt llm_retry_jitter; do
    echo ""
    echo "[baseline] running --test $test_file ..."
    if cargo test --test "$test_file" 2>&1 | tee -a "$PBT_LOG" | tail -3; then
        :
    else
        echo "[baseline] WARNING: $test_file had non-zero exit, continuing to next..."
    fi
done
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

# ── step 4 (可选)：Docker available 时跑无 LLM/MCP 的知识库集成测试 ────
# 触发条件：DOCKER_AVAILABLE=1 显式开关；缺省 OFF。
# 这个 step 永远不会"沉默通过"——若开了 docker 但测试 fail，立刻退出 1。
if [ "${DOCKER_AVAILABLE:-0}" = "1" ]; then
    GAP_LOG=$(mktemp)
    GAP_BASELINE=3
    trap 'rm -f "$LIB_LOG" "$PBT_LOG" "$GAP_LOG"' EXIT

    echo ""
    echo "[baseline] step 4 (DOCKER_AVAILABLE=1): cargo test --test wiki_gap_signals_3kinds -- --ignored ..."
    cargo test --test wiki_gap_signals_3kinds -- --ignored 2>&1 | tee "$GAP_LOG" || true
    GAP_PASSED=$(parse_passed "$GAP_LOG")
    GAP_FAILED=$(parse_failed "$GAP_LOG")
    echo "[baseline] gap_signals summary: passed=$GAP_PASSED failed=$GAP_FAILED (need >= $GAP_BASELINE passed, 0 failed)"

    if [ "$GAP_FAILED" -gt 0 ]; then
        echo "[baseline] FAIL: wiki_gap_signals_3kinds has $GAP_FAILED failed test(s)"
        exit 1
    fi
    if [ "$GAP_PASSED" -lt "$GAP_BASELINE" ]; then
        echo "[baseline] FAIL: wiki_gap_signals_3kinds only $GAP_PASSED passed (< baseline $GAP_BASELINE)"
        exit 1
    fi
    echo ""
    echo "baseline OK: lib=$LIB_PASSED, pbt=$PBT_PASSED, gap_signals=$GAP_PASSED"
else
    echo ""
    echo "[baseline] step 3 skipped (DOCKER_AVAILABLE!=1)"
    echo "baseline OK: lib=$LIB_PASSED, pbt=$PBT_PASSED"
fi
exit 0
