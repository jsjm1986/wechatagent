//! 运行时禁词扫描（M4 W2 Task 3.4）。
//!
//! 与 `scripts/check-no-human-takeover.{sh,ps1}` 同款词典，用于 prompt critic
//! 候选输出的运行时审查：Critic LLM 即使被 instructed 也偶尔会写出违反 AI
//! 自治定位的中文 / 英文术语；任何命中 SHALL 让整批 proposal 被 drop。
//!
//! threshold 类（数值）不需要这步——纯数值无文本。
//!
//! 实现选择：纯字符串匹配（不引入 regex 依赖）。CI lint shell 走 `grep -E`
//! 那批连字符变体（`hand off / hand-off / hand_off`），运行时这里也用同款变体
//! 一一展开（变体数量小、有限可控，比拉一个 regex crate 进来更轻）。

const FORBIDDEN_LITERALS_LOWER: &[&str] = &[
    "human takeover",
    "human-takeover",
    "human_takeover",
    "hand off",
    "hand-off",
    "hand_off",
    "handoff",
    "takeover",
    // 中文词不区分大小写（Unicode），逐字比较即可。
    "人工接管",
    "人工介入",
    "人工托管",
    "接管",
    "人工",
];

/// 返回 `true` 当字符串**不含**任何禁词；`false` 表示命中至少一条禁词。
///
/// ASCII 字符按小写匹配（统一 `to_ascii_lowercase`），中文按字面量比较。
pub fn passes_forbidden_words(snippet: &str) -> bool {
    let lowered = snippet.to_ascii_lowercase();
    for needle in FORBIDDEN_LITERALS_LOWER {
        if lowered.contains(needle) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_clean_text_passes() {
        assert!(passes_forbidden_words(""));
        assert!(passes_forbidden_words("Reply Agent should ground claims in knowledge"));
        assert!(passes_forbidden_words("AI 策略主动暂缓 / 安全门拦截"));
    }

    #[test]
    fn english_takeover_variants_blocked() {
        assert!(!passes_forbidden_words("if user is angry, hand off to a human"));
        assert!(!passes_forbidden_words("trigger human-takeover branch"));
        assert!(!passes_forbidden_words("human_takeover required"));
        assert!(!passes_forbidden_words("requires hand-off to operator"));
        assert!(!passes_forbidden_words("schedule takeover at 9am"));
        assert!(!passes_forbidden_words("requires handoff to ops"));
    }

    #[test]
    fn chinese_takeover_variants_blocked() {
        assert!(!passes_forbidden_words("当用户愤怒时切换到人工接管"));
        assert!(!passes_forbidden_words("人工介入是必要兜底"));
        assert!(!passes_forbidden_words("由人工托管"));
        assert!(!passes_forbidden_words("由运营人工跟进"));
        assert!(!passes_forbidden_words("接管该会话"));
    }

    #[test]
    fn case_insensitive_match() {
        assert!(!passes_forbidden_words("HUMAN_TAKEOVER required"));
        assert!(!passes_forbidden_words("Hand Off pls"));
        assert!(!passes_forbidden_words("HANDOFF"));
    }
}
