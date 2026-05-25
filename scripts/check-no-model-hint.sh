#!/usr/bin/env bash
# scripts/check-no-model-hint.sh
#
# CI 严禁词 lint（knowledge-wiki Phase G）。
#
# 在 git diff 新增行（+ 开头）范围内扫描 src/ 与 frontend/src/ 下是否
# 在新增内容里硬编码具体模型/品牌名称（gpt / claude / gemini / openai /
# anthropic / deepseek / qwen / 千问 / 豆包 / kimi / chatgpt / 模型推荐 ...）。
# 任意命中 → exit 1。
#
# 设计目标：
# - LLM provider 由用户在 LlmProviderConfigs 里自填；产品代码、prompt、UI、
#   docs（除 docs/agent-policy.md 等历史文件中明确允许的中性词）一律不出现
#   具体模型/品牌字眼，避免给任何模型做"广告提示 / 暗示用户使用什么模型"
#   的信息。
#
# 使用：
#   scripts/check-no-model-hint.sh                # 默认：origin/main..HEAD
#   scripts/check-no-model-hint.sh main           # 指定 base
#   scripts/check-no-model-hint.sh origin/main HEAD~1
#
# 注意：
# - 仅扫描"diff 新增内容"，老代码历史命中不计。
# - 历史 src/llm.rs / src/config.rs / .env.example 等需要保留 OPENAI_BASE_URL
#   等环境变量名的文件视为白名单，命中不算违规。
# - 测试文件（tests/、*_test.rs、*.test.ts 等）允许硬编码模型名做 mock。
#   通过 SKIP_PATHS_REGEX 排除。

set -euo pipefail

BASE="${1:-origin/main}"
HEAD_REF="${2:-HEAD}"

# 受 lint 的目录前缀（POSIX path）。
SCAN_DIRS=(
    "src/"
    "frontend/src/"
    "docs/"
)

# 排除路径（白名单）。
# - .env.example 与 src/config.rs 必须保留 OPENAI_BASE_URL 等历史 env 变量名；
# - src/llm.rs 是 LLM 客户端实现，需要 openai-compat / deepseek 等字面量；
# - src/error.rs 错误描述允许提及 LLM 上游品牌（中性提示运维）；
# - tests/ 与 src/**/tests.rs 允许 mock 模型名；
# - docs/llm-config.md 等运维文档允许出现 OpenAI/DeepSeek 等品牌（中性介绍）；
# - docs/real-task-* 是运维 runbook，允许提及当前实际使用的 LLM 品牌；
# - knowledge-wiki 设计文档（design.md）允许提及 LLW 等借鉴来源。
SKIP_PATHS_REGEX='(^|/)((\.env\.example)|(config\.rs)|(llm\.rs)|(error\.rs)|tests/|.*/tests\.rs|.*\.test\.(ts|tsx|js|jsx)|llm-config\.md|llm-providers\.md|README\.md|real-task-.*\.md|.*-design\.md)$'

# 严禁字面量（大小写不敏感）。
# 注意：
# - 用单词边界减少误伤；
# - 排除"CLAUDE.md"/"CLAUDE_CODE"等专属文件名（用 negative lookbehind 不行，
#   退而用 grep -v 后置过滤）；
# - 仅匹配 model 暗示语境（如 "GPT-4" / "Claude 3" / "推荐使用 GPT" 等）。
FORBIDDEN_PATTERN='(\bgpt[- ]?[0-9]|\bclaude[- ]?[0-9]|\bgemini[- ]?[0-9]?|\banthropic\b|\bdeepseek[- ]?[a-z0-9]|\bqwen[- ]?[a-z0-9]?|\bkimi[- ]?[a-z0-9]?|\bchatgpt\b|千问|豆包|文心一言|ChatGLM|模型推荐|模型建议|默认模型|推荐使用 ?GPT|推荐使用 ?Claude|默认 ?GPT|默认 ?Claude)'

mapfile -t CHANGED < <(
    git diff --name-only --diff-filter=ACMR "$BASE..$HEAD_REF" -- \
        "${SCAN_DIRS[@]}" 2>/dev/null || true
)

if [ "${#CHANGED[@]}" -eq 0 ]; then
    echo "[no-model-hint] no changed files under scan dirs; ok."
    exit 0
fi

VIOLATIONS=0
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

for FILE in "${CHANGED[@]}"; do
    # 跳过白名单
    if echo "$FILE" | grep -qE "$SKIP_PATHS_REGEX"; then
        continue
    fi
    # 仅扫 + 行（新增）；剥掉 +++ 头部
    git diff --no-color "$BASE..$HEAD_REF" -- "$FILE" \
        | grep -E '^\+[^+]' \
        | grep -iE "$FORBIDDEN_PATTERN" \
        | sed "s|^|$FILE: |" \
        > "$TMP" || true
    if [ -s "$TMP" ]; then
        echo "[no-model-hint] violation in $FILE:"
        cat "$TMP" | sed 's|^|  |'
        VIOLATIONS=$((VIOLATIONS + $(wc -l < "$TMP")))
    fi
done

if [ "$VIOLATIONS" -gt 0 ]; then
    echo
    echo "[no-model-hint] FAIL: $VIOLATIONS violation(s)"
    echo "Tip: replace concrete model names with neutral terms (LLM / provider / model_alias)."
    echo "     LLM provider configuration belongs in LlmProviderConfigs (user-filled)."
    exit 1
fi

echo "[no-model-hint] ok: 0 violations across ${#CHANGED[@]} changed file(s)."
