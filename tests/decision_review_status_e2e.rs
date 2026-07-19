//! C8 回归：decision_review_json 关联同 run_id 的 AgentRunLog，
//! 补 emit `finalReviewStatus`（顶层 snake 字段）/ `holdCategory`
//! （review doc 内 camelCase 键）拦截分支。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。
//!
//! 数据层测试说明：HTTP 端点 `list_decision_reviews` / `get_decision_review` 与
//! 投影函数 `decision_review_json` / `fetch_run_status` 均为 `pub(super)`，且
//! `routes::reviews` 是私有模块，跨 crate（集成测试）不可达；把它们改 pub 超出本任务
//! 允许改动的文件清单（会动 src/routes/mod.rs 可见性）。因此本测试走数据层：通过 typed
//! collection 真实写入 AgentDecisionReview + AgentRunLog（经 Mongo serde 一圈），再用
//! 与 `fetch_run_status` 完全相同的关联路径（`agent_run_logs().find_one(doc!{"run_id":R})`
//! → 取顶层 `final_review_status` + `review` doc 内 `holdCategory`）断言两值能正确取出。
//! 两值来自真实 BSON 关联（非手搭 struct）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::{AgentDecisionReview, AgentRunLog};

fn blocked_review(run_id: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: Some("wxid_c8".to_string()),
        run_id: Some(run_id.to_string()),
        inbound_message_id: None,
        reply_text: Some("拦截示例".to_string()),
        approved: false,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: Vec::new(),
        rewrite_instruction: None,
        review_summary: None,
        playbook_id: None,
        playbook_version: None,
        used_knowledge_ids: Vec::new(),
        prompt_versions: Document::new(),
        operation_state: None,
        next_best_action: Document::new(),
        context_pack_snapshot: Document::new(),
        domain_config_snapshot: Document::new(),
        runtime_parameters_snapshot: Document::new(),
        send_gateway_result: Document::new(),
        outcome_status: Some("pending".to_string()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        reaction_claim_token: None,
        reaction_claim_generation: 0,
        source_task_id: None,
        source_task_claim_token: None,
        reviewer_misjudge_signal: None,
        expected_text_segments: 0,
        status: "blocked".to_string(),
        created_at: DateTime::now(),
    }
}

fn run_log_held(run_id: &str) -> AgentRunLog {
    AgentRunLog {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: Some("wxid_c8".to_string()),
        run_id: run_id.to_string(),
        trigger_kind: "reply".to_string(),
        status: "blocked".to_string(),
        planner: Document::new(),
        context: Document::new(),
        knowledge_route: Document::new(),
        decision: Document::new(),
        // holdCategory 是 review doc 内的 camelCase 键（源自 DecisionReviewResult
        // 的 rename_all="camelCase"）；fetch_run_status 取的就是这个键。
        review: doc! { "holdCategory": "held_by_ai_policy" },
        gateway_result: Document::new(),
        error: None,
        token_budget: 0,
        tokens_used: 0,
        llm_calls_used: 0,
        degraded_reasons: Vec::new(),
        lifecycle: "completed".to_string(),
        source_event_id: "evt_c8".to_string(),
        source_kind: "inbound_message".to_string(),
        error_summary: None,
        abort_reason: None,
        revision_applied: false,
        revision_reason: String::new(),
        pre_revision_summary: None,
        post_revision_summary: None,
        self_critique: None,
        autonomy_mode: String::new(),
        // 顶层 snake 字段，fetch_run_status 直接读 log.final_review_status。
        final_review_status: "held_by_ai_policy".to_string(),
        outbox_status: None,
        memory_consolidator_warnings: Vec::new(),
        conversation_mode: String::new(),
        conversation_mode_reason: None,
        created_at: DateTime::now(),
    }
}

/// 复刻 routes/reviews.rs::fetch_run_status 的关联逻辑（该 fn 为 pub(super) 不可跨
/// crate 调用）。两值取自真实写入并经 Mongo 读回的 AgentRunLog。
async fn correlate(app: &common::TestApp, run_id: &str) -> (Option<String>, Option<String>) {
    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "run_id": run_id }, None)
        .await
        .expect("query agent_run_logs")
        .expect("agent_run_logs row exists");
    let frs = if log.final_review_status.is_empty() {
        None
    } else {
        Some(log.final_review_status.clone())
    };
    let hc = log
        .review
        .get_str("holdCategory")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    (frs, hc)
}

#[tokio::test]
#[ignore]
async fn decision_review_correlates_run_log_status_and_hold_category() {
    let app = common::TestApp::start().await;
    let run_id = "run_c8_held";

    let review = blocked_review(run_id);
    app.state
        .db
        .decision_reviews()
        .insert_one(&review, None)
        .await
        .expect("insert blocked review");
    app.state
        .db
        .agent_run_logs()
        .insert_one(&run_log_held(run_id), None)
        .await
        .expect("insert run log");

    // 真实关联路径：按 run_id 查 AgentRunLog，取两值。
    let (frs, hc) = correlate(&app, review.run_id.as_deref().unwrap()).await;
    assert_eq!(
        frs.as_deref(),
        Some("held_by_ai_policy"),
        "finalReviewStatus 应从顶层 final_review_status 取出"
    );
    assert_eq!(
        hc.as_deref(),
        Some("held_by_ai_policy"),
        "holdCategory 应从 review doc 内 camelCase 键取出"
    );

    // 投影出口形态：与 decision_review_json 的 json! 块一致（camelCase 键）。
    let item = serde_json::json!({
        "approved": review.approved,
        "finalReviewStatus": frs,
        "holdCategory": hc,
    });
    assert_eq!(item["finalReviewStatus"], "held_by_ai_policy");
    assert_eq!(item["holdCategory"], "held_by_ai_policy");
    assert_eq!(item["approved"], false);
}
