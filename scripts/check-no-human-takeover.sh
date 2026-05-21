#!/usr/bin/env bash
# scripts/check-no-human-takeover.sh
#
# CI 严禁词文本 lint（agent-autonomy-loop W6 / Task 7.7 / Requirement 2.7）。
#
# 在 git diff 新增行（+ 开头）范围内扫描 src/agent/ src/routes/ frontend/src/
# 下是否引入了"human / 人工 / 接管 / takeover / hand-off"等违反"全自治、
# 无人工接管"产品定位的字面量。任意命中 → exit 1。
#
# 使用：
#   scripts/check-no-human-takeover.sh                # 默认：origin/main..HEAD
#   scripts/check-no-human-takeover.sh main           # 指定 base
#   scripts/check-no-human-takeover.sh origin/main HEAD~1
#
# 注意：
# - 仅扫描"diff 新增内容"，老代码里历史命中不计；这是渐进治理路径。
# - 命中字面量但属于注释/文档说明（如 sunset-plan.md）不算违规：本脚本只
#   扫 src/agent/ src/routes/ frontend/src/ 三个目录。

set -euo pipefail

BASE="${1:-origin/main}"
HEAD_REF="${2:-HEAD}"

# 受 lint 的目录前缀（POSIX path）。
SCAN_DIRS=(
    "src/agent/"
    "src/routes/"
    "src/evolution/"
    "frontend/src/"
)

# 严禁词列表（不区分大小写）。中文词通过 grep -E 直接匹配；
# 英文词同时覆盖 hyphen / underscore 变体。
FORBIDDEN_PATTERN='(human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工)'

# 列出 base..HEAD 之间在 SCAN_DIRS 下变更的文件（仅文本文件，排除删除）。
mapfile -t CHANGED < <(
    git diff --name-only --diff-filter=ACMR "$BASE..$HEAD_REF" -- \
        "${SCAN_DIRS[@]}" 2>/dev/null || true
)

if [ "${#CHANGED[@]}" -eq 0 ]; then
    echo "[no-human-takeover] no changed files under scan dirs; ok."
    exit 0
fi

VIOLATIONS=0
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

for f in "${CHANGED[@]}"; do
    # 跳过非文本（图片 / 二进制） + 跳过测试文件（test 写预期失败串很常见）。
    case "$f" in
        *.png|*.jpg|*.jpeg|*.gif|*.ico|*.woff|*.woff2|*.ttf) continue ;;
        */tests/*|tests/*|*/__tests__/*|*.test.*|*.spec.*) continue ;;
        # M4 W2：演化器自带禁词词典本身就需要列出全部禁词作为运行期黑名单
        # （`evolution::lint::FORBIDDEN_WORDS`），不应被字面量 lint 反向命中。
        src/evolution/lint.rs) continue ;;
    esac
    [ -f "$f" ] || continue

    # 取该文件 base..HEAD 的新增行（+ 开头但跳过 diff header 的 +++ ）。
    git diff "$BASE..$HEAD_REF" -- "$f" \
        | awk '/^\+\+\+/{next} /^\+/{print substr($0,2)}' \
        > "$TMP"

    if [ ! -s "$TMP" ]; then
        continue
    fi

    if grep -E -i -n "$FORBIDDEN_PATTERN" "$TMP" > /dev/null; then
        echo "[no-human-takeover] FAIL: $f 包含严禁词："
        grep -E -i -n "$FORBIDDEN_PATTERN" "$TMP" | sed 's/^/    +/'
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done

if [ "$VIOLATIONS" -gt 0 ]; then
    echo ""
    echo "[no-human-takeover] $VIOLATIONS file(s) violated 全自治、无人工接管 定位。"
    echo "如确属合法引用（如 sunset 文档解释历史词），请把变更挪到不在扫描目录下的位置。"
    exit 1
fi

echo "[no-human-takeover] ok: 0 violations across ${#CHANGED[@]} changed file(s)."
exit 0
