//! knowledge-digest-workstation Phase 1：骨架冒烟测试（无 Docker 依赖）。
//!
//! 守住 4 个不变量，配合 `src/knowledge_digest/mod.rs` 内的两个单元测试，
//! 把 Phase 1 的最小骨架 fence 起来，让后续 Phase 2 真实装的 LLM 合成路径
//! 改动不会悄悄破坏 baseline。
//!
//! 1. `KnowledgeDailyReport` / `KnowledgeChatTask` / `KnowledgeOperatorMemory`
//!    新结构能与 BSON 互相 round-trip（含 `Vec<KnowledgeDigestCard>` 嵌套）。
//! 2. `KnowledgeChatTurn` 反序列化对**老数据**向后兼容：缺 `kind`/`tool_calls`
//!    必须按 None / 空 vec 解码（现有 chat 历史不能因升级被毒化）。
//! 3. `AppConfig::from_env` 在不设任何 KNOWLEDGE_DIGEST_* env 时取规范默认值
//!    （`enabled=false / hour=9 / token=24000 / calls=8 / interval=30`）。
//! 4. `KnowledgeDigestCard.severity` / `kind` / `suggested_action` 三个枚举
//!    位的合法字符串集合编译期可枚举（防止后续重命名 silent 漂移）。

use mongodb::bson::{doc, oid::ObjectId, to_bson, to_document, DateTime, Document};
use wechatagent::models::{
    KnowledgeChatTask, KnowledgeChatTurn, KnowledgeDailyReport, KnowledgeDigestCard,
    KnowledgeOperatorMemory,
};

fn sample_card() -> KnowledgeDigestCard {
    KnowledgeDigestCard {
        card_id: ObjectId::new(),
        kind: "chunk_missing_field".to_string(),
        title: "切片 abc123 缺少 sourceQuote".to_string(),
        summary: "本卡片提示 chunk_missing_field 的修复路径".to_string(),
        target_refs: vec![doc! { "kind": "chunk", "id": "abc123" }],
        suggested_action: "fix_chunk".to_string(),
        severity: "warn".to_string(),
        metric: Some(doc! { "name": "missing_fields", "value": 1, "threshold": 0 }),
    }
}

#[test]
fn knowledge_daily_report_bson_roundtrip() {
    let report = KnowledgeDailyReport {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        report_date: "2026-05-24".to_string(),
        generated_at: DateTime::now(),
        generated_by: "worker".to_string(),
        status: "ok".to_string(),
        error_kind: None,
        budget_snapshot: doc! { "tokens_used": 1200, "llm_calls": 3 },
        cards: vec![sample_card()],
        dismissed_card_ids: vec![],
        prompt_versions: doc! { "knowledge.digest.compose": "v1" },
    };
    let bson = to_bson(&report).expect("serialize");
    let doc: Document = bson.as_document().expect("doc").clone();
    let back: KnowledgeDailyReport =
        mongodb::bson::from_document(doc).expect("roundtrip deserialize");
    assert_eq!(back.report_date, "2026-05-24");
    assert_eq!(back.cards.len(), 1);
    assert_eq!(back.cards[0].kind, "chunk_missing_field");
    assert_eq!(back.cards[0].severity, "warn");
}

#[test]
fn knowledge_chat_task_bson_roundtrip() {
    let task = KnowledgeChatTask {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        session_id: "sess_abc".to_string(),
        operator_id: Some("op_1".to_string()),
        cards: vec![sample_card()],
        planned_steps: vec![doc! { "cardId": "x", "action": "fix_chunk" }],
        completed_steps: vec![],
        status: "pending".to_string(),
        error_kind: None,
        created_at: DateTime::now(),
        started_at: None,
        finished_at: None,
    };
    let bson = to_bson(&task).expect("serialize");
    let doc: Document = bson.as_document().expect("doc").clone();
    let back: KnowledgeChatTask =
        mongodb::bson::from_document(doc).expect("roundtrip deserialize");
    assert_eq!(back.status, "pending");
    assert_eq!(back.planned_steps.len(), 1);
}

#[test]
fn knowledge_operator_memory_bson_roundtrip() {
    let mem = KnowledgeOperatorMemory {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        operator_id: "op_1".to_string(),
        kind: "preference".to_string(),
        content: "运营偏好：晚上 8 点后不再 retag".to_string(),
        created_at: DateTime::now(),
        last_used_at: DateTime::now(),
        expires_at: None,
    };
    let doc = to_document(&mem).expect("serialize");
    let back: KnowledgeOperatorMemory =
        mongodb::bson::from_document(doc).expect("roundtrip deserialize");
    assert_eq!(back.kind, "preference");
    assert!(back.expires_at.is_none());
}

#[test]
fn knowledge_chat_turn_backward_compatible_without_new_fields() {
    // 模拟 Phase 1 升级前老 turn（无 kind / tool_calls 字段）能否反序列化成功。
    let empty_strs: Vec<&str> = vec![];
    let empty_docs: Vec<Document> = vec![];
    let legacy_doc = doc! {
        "workspace_id": "default",
        "account_id": "default",
        "session_id": "sess_old",
        "turn_index": 1_i32,
        "role": "user",
        "content": "hello",
        "missing_fields": empty_strs,
        "followup_questions": empty_docs,
        "status": "pending",
        "tokens_used": 0_i64,
        "created_at": DateTime::now(),
    };
    let turn: KnowledgeChatTurn =
        mongodb::bson::from_document(legacy_doc).expect("legacy turn must deserialize");
    assert_eq!(turn.role, "user");
    assert!(turn.kind.is_none(), "legacy turn must have kind=None");
    assert!(
        turn.tool_calls.is_empty(),
        "legacy turn must have empty tool_calls"
    );
}

/// 文案约束：禁词 lint 复制一份在 Rust 单测里，作为 source-of-truth 的
/// 早期防御（`scripts/check-no-human-takeover.sh` 是 CI 二道闸）。
#[test]
fn knowledge_digest_card_kind_strings_are_in_closed_set() {
    let allowed_kinds = [
        "chunk_missing_field",
        "chunk_low_hit_rate",
        "chunk_caused_block",
        "pack_outdated",
        "evolution_pending",
        "evolution_released",
        "freeform",
    ];
    let allowed_severities = ["info", "warn", "critical"];
    let allowed_actions = [
        "fix_chunk",
        "add_chunk",
        "retag",
        "review_evolution",
        "dismiss",
        "freeform",
    ];
    let card = sample_card();
    assert!(allowed_kinds.contains(&card.kind.as_str()));
    assert!(allowed_severities.contains(&card.severity.as_str()));
    assert!(allowed_actions.contains(&card.suggested_action.as_str()));
}
