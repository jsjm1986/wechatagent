#!/usr/bin/env bash
# scripts/test-ci-lints.sh
#
# CI 红线 lint 的行为自测（agent-autonomy-loop Task11）。
#
# 目的：给 `check-no-human-takeover.sh` 的禁词正则加一个可执行的守门自测——
#   1. 行为断言：该拦的正向 fixture 真拦（命中）、该放的负向 fixture 真放（不命中）；
#   2. 专门锁死 `hand_off` 下划线分叉不再复现（历史 bug：shell 正则漏了 `_`）；
#   3. 三方一致性断言（本自测的灵魂）：把 `src/evolution/lint.rs` 的
#      `FORBIDDEN_LITERALS_LOWER` 词表与 shell 的 `FORBIDDEN_PATTERN` 正则做交叉，
#      未来任一方加词 / 改正则、另一方漏跟，本自测立即变红。
#
# 关键设计：正则和词表都**从源文件动态读取**（下方 eval / sed 提取），绝不在本
# 脚本里另抄一份——另抄一份就是"平行实现自证"，被测源改了、抄的没改，测不出 drift。
#
# 不依赖 git 历史 / 真 diff：直接把被测脚本里那一份真实正则跑在 fixture 字符串上，
# 避免在 CI 里造临时 commit 的复杂度。
#
# 用法：bash scripts/test-ci-lints.sh   （本机 Git Bash 亦可，无需 Docker / cargo）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SH="$SCRIPT_DIR/check-no-human-takeover.sh"
LINT="$REPO_ROOT/src/evolution/lint.rs"

fail() {
    echo "[test-ci-lints] FAIL: $*" >&2
    exit 1
}

[ -f "$SH" ] || fail "找不到被测脚本 $SH"
[ -f "$LINT" ] || fail "找不到词表源 $LINT"

# ---------------------------------------------------------------------------
# 动态读取被测脚本里"那一份真实的" FORBIDDEN_PATTERN（不硬编码复制）。
# 取出 `FORBIDDEN_PATTERN=...` 那一行原样 eval，让本脚本作用域里的
# $FORBIDDEN_PATTERN 与被测脚本字符级同源；被测脚本改正则，这里自动跟着改。
# ---------------------------------------------------------------------------
pattern_line="$(grep -E '^FORBIDDEN_PATTERN=' "$SH" || true)"
[ -n "$pattern_line" ] || fail "无法从 $SH 提取 FORBIDDEN_PATTERN 定义行"
eval "$pattern_line"
[ -n "${FORBIDDEN_PATTERN:-}" ] || fail "FORBIDDEN_PATTERN 提取后为空"
echo "[test-ci-lints] 动态取到 shell 正则: $FORBIDDEN_PATTERN"

# ---------------------------------------------------------------------------
# 动态读取 lint.rs 的 FORBIDDEN_LITERALS_LOWER 词表（不硬编码复制）。
# 截取 const 声明到首个 `];` 之间的块，抽出每个双引号字面量、去引号。
# 该块内除词条外无其它双引号字符串（注释行为中文无引号），提取干净。
# ---------------------------------------------------------------------------
mapfile -t LINT_WORDS < <(
    sed -n '/const FORBIDDEN_LITERALS_LOWER/,/\];/p' "$LINT" \
        | grep -oE '"[^"]+"' \
        | sed 's/"//g'
)
[ "${#LINT_WORDS[@]}" -gt 0 ] || fail "从 $LINT 未提取到任何词表词条"
echo "[test-ci-lints] 动态取到 lint.rs 词表 ${#LINT_WORDS[@]} 条: ${LINT_WORDS[*]}"

# ---------------------------------------------------------------------------
# 断言辅助：对单个字符串跑被测正则（grep -E -i，与被测脚本 :72 同款调用）。
# ---------------------------------------------------------------------------
matches() {
    printf '%s\n' "$1" | grep -E -i -q "$FORBIDDEN_PATTERN"
}

assert_hit() {
    if matches "$1"; then
        echo "[test-ci-lints]   ok  正向命中: '$1'"
    else
        fail "正向 fixture 应命中却漏拦: '$1'"
    fi
}

assert_no_hit() {
    if matches "$1"; then
        fail "负向 fixture 不应命中却被拦: '$1'"
    else
        echo "[test-ci-lints]   ok  负向放行: '$1'"
    fi
}

# ---------------------------------------------------------------------------
# 1) 正向 fixture：必须命中。含 hand_off —— 锁死本次修复的关键断言：
#    修复前 shell 正则 `hand[ -]?off` 漏 `_`，此条会漏拦 → 本自测 exit 1。
# ---------------------------------------------------------------------------
echo "[test-ci-lints] --- 正向 fixture（必须命中）---"
POSITIVE=(
    "人工接管"
    "human-takeover"
    "human_takeover"
    "handoff"
    "hand-off"
    "hand off"
    "let x = hand_off_flag;"
    "接管"
    "人工"
)
for s in "${POSITIVE[@]}"; do
    assert_hit "$s"
done

# ---------------------------------------------------------------------------
# 2) 负向 fixture：必须不命中。已亲验不含 takeover/接管/人工/handoff/hand 等
#    任何禁词子串（automatic_reply / send_message 均为正常业务词）。
# ---------------------------------------------------------------------------
echo "[test-ci-lints] --- 负向 fixture（必须放行）---"
NEGATIVE=(
    "automatic_reply"
    "send_message"
    "let outbox = build_send_outbox();"
)
for s in "${NEGATIVE[@]}"; do
    assert_no_hit "$s"
done

# ---------------------------------------------------------------------------
# 3) 三方一致性断言（灵魂）：lint.rs 词表里每一条（英文 + 中文）都必须能被
#    shell 的 FORBIDDEN_PATTERN 命中。任一 lint.rs 词 shell 漏拦 → drift → 红。
#    这把"同款词典"契约从注释变成可执行守门。
# ---------------------------------------------------------------------------
echo "[test-ci-lints] --- 三方一致性：lint.rs 每条词都须被 shell 正则覆盖 ---"
for w in "${LINT_WORDS[@]}"; do
    if matches "$w"; then
        echo "[test-ci-lints]   ok  lint.rs 词被 shell 覆盖: '$w'"
    else
        fail "三方分叉: lint.rs 词 '$w' 未被 shell FORBIDDEN_PATTERN 命中（词表/正则 drift）"
    fi
done

echo "[test-ci-lints] ok: 全部断言通过（正向 ${#POSITIVE[@]} / 负向 ${#NEGATIVE[@]} / lint.rs 一致性 ${#LINT_WORDS[@]}）。"
exit 0
