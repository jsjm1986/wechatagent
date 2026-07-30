//! digest 兜底文案里内嵌的 final_review_status 拦截码 → 中文。
//! 取值来源:analyze_run_logs 扫描的 4 个状态(knowledge_digest/mod.rs:277-282)。
//! 未知回落原值。

pub(crate) fn block_reason_zh(reason: &str) -> String {
    match reason {
        "blocked_by_required_field" => "必填信息缺失".to_string(),
        "blocked_by_budget" => "本轮算力预算耗尽".to_string(),
        "blocked_unverified_product_claim" => "产品说法未经核实".to_string(),
        "blocked_by_safety_guard" => "安全门拦截".to_string(),
        "unknown" => "未知原因".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_block_reasons() {
        assert_eq!(
            block_reason_zh("blocked_unverified_product_claim"),
            "产品说法未经核实"
        );
        assert_eq!(block_reason_zh("blocked_by_required_field"), "必填信息缺失");
        assert_eq!(block_reason_zh("blocked_by_budget"), "本轮算力预算耗尽");
        assert_eq!(block_reason_zh("blocked_by_safety_guard"), "安全门拦截");
    }

    #[test]
    fn unknown_falls_back_to_input() {
        assert_eq!(block_reason_zh("brand_new_reason"), "brand_new_reason");
    }
}
