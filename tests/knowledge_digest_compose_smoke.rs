//! knowledge-digest-workstation Phase 2：合成路径 smoke 测试（无 Docker 依赖）。
//!
//! 守住 3 个 Phase 2 不变量，配合 `src/knowledge_digest/mod.rs` 内的 6 个单元测试，
//! 让 Phase 3 的画布 / Phase 4 的长任务 / Phase 5 的 tool-calling 改动都不会
//! 让 Phase 2 baseline 隐性退化。
//!
//! 1. `KnowledgeDigestCard` 在 `Vec<Document>` 嵌套 + `metric` Document 字段
//!    的情况下 BSON round-trip 不丢失精度。
//! 2. `KnowledgeDailyReport.status` 必须能写入 `partial` / `failed` / `ok` 三种
//!    状态值（封闭枚举），并且嵌入的 cards 数组在 status="failed" 时允许为空。
//! 3. `KnowledgeDigestCard.target_refs` 字段允许混合 `chunk` / `pack` / `proposal`
//!    三种 kind（Phase 2 prompt 输出契约）。

use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document};
use wechatagent::models::{KnowledgeDailyReport, KnowledgeDigestCard};

fn make_card(kind: &str, severity: &str, action: &str) -> KnowledgeDigestCard {
    KnowledgeDigestCard {
        card_id: ObjectId::new(),
        kind: kind.to_string(),
        title: format!("{kind} 卡片"),
        summary: format!("AI 建议处理 {kind}"),
        target_refs: vec![doc! { "kind": "chunk", "id": "abc123" }],
        suggested_action: action.to_string(),
        severity: severity.to_string(),
        metric: Some(doc! { "name": "block_count", "value": 3_i64, "threshold": 1_i64 }),
    }
}

#[test]
fn card_with_metric_document_roundtrips_via_bson() {
    let card = make_card("chunk_caused_block", "warn", "fix_chunk");
    let doc = to_document(&card).expect("serialize card");
    let back: KnowledgeDigestCard =
        mongodb::bson::from_document(doc).expect("roundtrip card");
    assert_eq!(back.kind, "chunk_caused_block");
    assert_eq!(back.severity, "warn");
    assert_eq!(back.suggested_action, "fix_chunk");
    assert_eq!(back.target_refs.len(), 1);
    let metric = back.metric.expect("metric should round-trip");
    assert_eq!(metric.get_str("name").unwrap(), "block_count");
    assert_eq!(metric.get_i64("value").unwrap(), 3);
}

#[test]
fn report_accepts_failed_status_with_empty_cards() {
    let report = KnowledgeDailyReport {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        report_date: "2026-05-24".to_string(),
        generated_at: DateTime::now(),
        generated_by: "worker".to_string(),
        status: "failed".to_string(),
        error_kind: Some("upstream_timeout".to_string()),
        budget_snapshot: doc! { "tokens_used": 0_i64, "llm_calls_used": 0_i64 },
        cards: vec![],
        dismissed_card_ids: vec![],
        prompt_versions: doc! { "knowledge.digest.compose": "v1" },
    };
    let bson = mongodb::bson::to_bson(&report).expect("serialize");
    let doc: Document = bson.as_document().expect("doc").clone();
    let back: KnowledgeDailyReport =
        mongodb::bson::from_document(doc).expect("roundtrip");
    assert_eq!(back.status, "failed");
    assert_eq!(back.error_kind.as_deref(), Some("upstream_timeout"));
    assert!(back.cards.is_empty());
}

#[test]
fn report_accepts_partial_status_with_budget_exceeded() {
    let report = KnowledgeDailyReport {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        report_date: "2026-05-24".to_string(),
        generated_at: DateTime::now(),
        generated_by: "worker".to_string(),
        status: "partial".to_string(),
        error_kind: Some("budget_exceeded".to_string()),
        budget_snapshot: doc! { "tokens_used": 24_000_i64, "llm_calls_used": 8_i64 },
        cards: vec![make_card("chunk_missing_field", "info", "fix_chunk")],
        dismissed_card_ids: vec![],
        prompt_versions: doc! { "knowledge.digest.compose": "v1" },
    };
    let bson = mongodb::bson::to_bson(&report).expect("serialize");
    let doc: Document = bson.as_document().expect("doc").clone();
    let back: KnowledgeDailyReport =
        mongodb::bson::from_document(doc).expect("roundtrip");
    assert_eq!(back.status, "partial");
    assert_eq!(back.error_kind.as_deref(), Some("budget_exceeded"));
    assert_eq!(back.cards.len(), 1);
    assert_eq!(
        back.budget_snapshot.get_i64("tokens_used").unwrap(),
        24_000
    );
}

#[test]
fn card_target_refs_support_mixed_kinds() {
    let mut card = make_card("evolution_pending", "critical", "review_evolution");
    card.target_refs = vec![
        doc! { "kind": "proposal", "id": "p_001" },
        doc! { "kind": "pack", "id": "pk_002" },
        doc! { "kind": "chunk", "id": "c_003" },
    ];
    let doc = to_document(&card).expect("serialize");
    let back: KnowledgeDigestCard =
        mongodb::bson::from_document(doc).expect("roundtrip");
    assert_eq!(back.target_refs.len(), 3);
    let kinds: Vec<&str> = back
        .target_refs
        .iter()
        .map(|d| d.get_str("kind").unwrap_or(""))
        .collect();
    assert!(kinds.contains(&"proposal"));
    assert!(kinds.contains(&"pack"));
    assert!(kinds.contains(&"chunk"));
}
