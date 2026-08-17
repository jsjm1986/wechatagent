//! 请示串里内嵌的内部状态码 → 运营可读中文映射。
//! 这些值嵌在给领导看的自然语言句子里(escalation/mod.rs),前端无法字典翻译,
//! 故在后端拼串前就转中文。未知值回落原字面量(不吞信息)。

/// blocked_status 内部码 → 中文。取值来源:should_escalate_held(logic.rs:333-343)
/// 覆盖 HOLD_CATEGORY_VALUES 三值 + 游离裸串 blocked_unverified_product_claim(logic.rs:337)。
pub(crate) fn blocked_status_zh(status: &str) -> String {
    match status {
        "blocked_unverified_product_claim" => "产品说法未经核实".to_string(),
        "blocked_by_safety_guard" => "安全门拦截".to_string(),
        "held_by_ai_policy" => "AI 策略主动暂缓".to_string(),
        "ai_waiting_for_more_context" => "AI 等待更多上下文".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_status_maps_known_values() {
        assert_eq!(
            blocked_status_zh("blocked_unverified_product_claim"),
            "产品说法未经核实".to_string()
        );
        assert_eq!(
            blocked_status_zh("blocked_by_safety_guard"),
            "安全门拦截".to_string()
        );
        assert_eq!(
            blocked_status_zh("held_by_ai_policy"),
            "AI 策略主动暂缓".to_string()
        );
        assert_eq!(
            blocked_status_zh("ai_waiting_for_more_context"),
            "AI 等待更多上下文".to_string()
        );
    }

    #[test]
    fn blocked_status_unknown_falls_back_to_input() {
        assert_eq!(
            blocked_status_zh("some_new_status"),
            "some_new_status".to_string()
        );
    }
}
