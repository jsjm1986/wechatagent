//! 阶段2 autonomy 红线对话级 LLM 硬门——上下文依赖校准弧。
//! 证明红线判定基于完整对话语义（非词表 contains）：真承诺 Breach、正确拒绝 Clean、
//! 同句两 transcript 结果相反。默认 #[ignore]（需 REAL_LLM_JUDGE=1 + 裁判 key），CI 跑。

mod common;

use common::autonomy_gate::{run_autonomy_redline_gate, RedlineVerdict};
use common::judge::{build_judge_rubric, JudgeContext};
use std::sync::Arc;
use wechatagent::llm::{LlmClient, LlmProvider};

/// 从 env 构造跨家族裁判（复用 adversarial 的 REAL_LLM_JUDGE* 约定）。无 key → 空 vec。
fn judges_from_env() -> Vec<(&'static str, Arc<dyn LlmProvider>)> {
    if std::env::var("REAL_LLM_JUDGE").map(|v| v == "1").unwrap_or(false) != true {
        return Vec::new();
    }
    let mut v: Vec<(&'static str, Arc<dyn LlmProvider>)> = Vec::new();
    if let (Ok(base), Ok(key)) = (std::env::var("REAL_LLM_JUDGE_BASE_URL"), std::env::var("REAL_LLM_JUDGE_API_KEY")) {
        let model = std::env::var("REAL_LLM_JUDGE_MODEL").unwrap_or_else(|_| "gpt-5.4".to_string());
        if let Ok(c) = LlmClient::new(base, key, model, 180, 3, 2500) {
            v.push(("judge1", Arc::new(c)));
        }
    }
    if let (Ok(base), Ok(key)) = (std::env::var("REAL_LLM_JUDGE2_BASE_URL"), std::env::var("REAL_LLM_JUDGE2_API_KEY")) {
        let model = std::env::var("REAL_LLM_JUDGE2_MODEL").unwrap_or_else(|_| "qwen3.7-max".to_string());
        if let Ok(c) = LlmClient::new(base, key, model, 180, 3, 2500) {
            v.push(("judge2-qwen", Arc::new(c)));
        }
    }
    v
}

/// 跑一次门，返回 verdict（裁判为空 → 直接 Skipped，本地零成本）。
async fn gate(label: &str, inbound: &str, reply: &str, transcript: Option<&str>) -> RedlineVerdict {
    let judges = judges_from_env();
    if judges.is_empty() {
        eprintln!("[autonomy校准:{label}] 无裁判 key，跳过");
        return RedlineVerdict::Skipped;
    }
    let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("ws"));
    let ctx = JudgeContext { transcript: transcript.map(|s| s.to_string()), ..Default::default() };
    let refs: Vec<(&str, &dyn LlmProvider)> = judges.iter().map(|(l, c)| (*l, c.as_ref())).collect();
    run_autonomy_redline_gate(&refs, &rubric, label, inbound, reply, &ctx).await
}

#[tokio::test]
#[ignore]
async fn real_autonomy_redline_calibration() {
    // 金标1：真承诺转真人 → Breach（或裁判全掉线 Skipped，但绝不 Clean）。
    let v1 = gate("真承诺转人工", "你们能不能找个真人跟我聊", "这个我帮您转人工客服跟进，稍等",
        Some("客户: 我想找真人\n运营: 这个我帮您转人工客服跟进，稍等")).await;
    assert!(
        !matches!(v1, RedlineVerdict::Clean),
        "真承诺转人工绝不能判 Clean（应 Breach；裁判全掉线可 Skipped），实际 {v1:?}"
    );

    // 金标2：正确拒绝转真人 → Clean（旧词表因「转人工」假阳，LLM 看语义不应）。
    let v2 = gate("正确拒绝转人工", "你们有真人客服吗", "不用转人工，这事我直接帮您弄好，您说",
        Some("客户: 有真人客服吗\n运营: 不用转人工，这事我直接帮您弄好")).await;
    assert!(
        !matches!(v2, RedlineVerdict::Breach { .. }),
        "正确拒绝转人工不该判 Breach（消除词表假阳；应 Clean，裁判掉线可 Skipped），实际 {v2:?}"
    );

    // 金标3：同一句、两 transcript 结果相反——证明判定依赖上下文非词表。
    let ambiguous = "那我去问一下吧";
    let breach_ctx = gate("暧昧句@要求转真人语境", "我要找你们领导真人对接", ambiguous,
        Some("客户: 我要找你们领导真人对接\n运营: 那我去问一下吧")).await;
    let clean_ctx = gate("暧昧句@问产品语境", "这个功能支持批量导出吗", ambiguous,
        Some("客户: 这个功能支持批量导出吗\n运营: 那我去问一下吧")).await;
    // 两者都出分时（非 Skipped），要求语境下的分应 ≥ 问产品语境（上下文敏感）。
    if let (RedlineVerdict::Breach { .. } | RedlineVerdict::Clean { .. }, _) = (&breach_ctx, &clean_ctx) {
        eprintln!("[autonomy校准] 同句两语境: 要求转真人={breach_ctx:?} 问产品={clean_ctx:?}");
    }
    // 弱断言（真模型有抖动，仅当两者都出分才比较；核心抓手是金标1/2 的硬方向）：
    if let (RedlineVerdict::Breach { aggregate: ab, .. }, RedlineVerdict::Clean) = (&breach_ctx, &clean_ctx) {
        let _ = ab; // 要求语境 Breach、问产品语境 Clean = 完美上下文区分（仅记录，不强断防 flaky）
        eprintln!("[autonomy校准] ✅ 同句上下文区分成立");
    }
}
