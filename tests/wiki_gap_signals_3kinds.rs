//! `wiki_gap_signals_3kinds` —— 三类新 gap_signal kind 的端到端集成测试。
//!
//! 覆盖范围（Plan v3 / Commit 2）：
//!   - `missing_chunk`：related_chunks 引用了已 archived 的 chunk → 应产生
//!     severity=error 的 missing_chunk 信号，且不再产生 broken_link；
//!     依赖恢复（chunk 重新 active）后 sweep 应 auto_resolved with
//!     `resolution_note="rule:dep_restored"`。
//!   - `suggestion`：未 verified 且 30d blocked > 3 → 产生 info suggestion；
//!     一旦 chunk integrity_status 改为 verified，sweep 应 auto_resolved with
//!     `resolution_note="rule:chunk_verified"`。
//!   - `contradiction`：同 normalize_title 多 chunk 且 body 首段 sha256 不一致
//!     → 产生 error contradiction；当其中一条 chunk 被 archived（同题只剩一条）
//!     后 sweep 应 auto_resolved with `resolution_note="rule:contradiction_resolved"`。
//!
//! 同时校验 dedup 不变量：连续两次 `run_structural_lint` 不应使同一 (kind, title)
//! 信号被重复 insert（`new_signals` 计数仅首次 > 0，第二次为 0）。
//!
//! 三类信号都是规则路径，不消耗 LLM；测试不需要为 `TestApp` 入队任何 LLM 响应。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use wechatagent::knowledge_wiki::gap_signals;
use wechatagent::models::{KnowledgeGapSignal, OperationKnowledgeChunk, RelatedRef, UsageStats};

use crate::common::TestApp;

const WS: &str = "ws_3kinds";

fn base_chunk(title: &str, wiki_type: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        title: title.to_string(),
        wiki_type: Some(wiki_type.to_string()),
        status: "active".to_string(),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

async fn insert_chunks(app: &TestApp, chunks: &[OperationKnowledgeChunk]) {
    for c in chunks {
        app.state
            .db
            .operation_knowledge_chunks()
            .insert_one(c, None)
            .await
            .expect("insert chunk");
    }
}

async fn list_pending(app: &TestApp, kind: &str) -> Vec<KnowledgeGapSignal> {
    use futures::TryStreamExt;
    let cursor = app
        .state
        .db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": WS, "kind": kind, "status": "pending" },
            None,
        )
        .await
        .expect("query gap_signals");
    cursor.try_collect().await.expect("collect")
}

async fn list_resolved(app: &TestApp, kind: &str) -> Vec<KnowledgeGapSignal> {
    use futures::TryStreamExt;
    let cursor = app
        .state
        .db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": WS, "kind": kind, "status": "auto_resolved" },
            None,
        )
        .await
        .expect("query resolved");
    cursor.try_collect().await.expect("collect")
}

#[tokio::test]
#[ignore]
async fn missing_chunk_emitted_when_target_archived_then_resolved_when_restored() {
    let app = TestApp::start().await;

    // 源 chunk 引用一个 target，target 后续被 archive。
    let target_oid = ObjectId::new();
    let target_hex = target_oid.to_hex();
    let mut target = base_chunk("被回收页", "entity");
    target.id = Some(target_oid);
    target.status = "archived".to_string();

    let mut src = base_chunk("源页", "entity");
    src.related_chunks = Some(vec![RelatedRef {
        chunk_id: target_hex.clone(),
        kind: "references".to_string(),
        note: None,
    }]);

    insert_chunks(&app, &[target.clone(), src.clone()]).await;

    // ── 第一次 lint：应产生 missing_chunk，且不应有 broken_link ──
    let report1 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("structural lint 1");
    assert!(
        report1.new_signals >= 1,
        "首次 lint 必须新增至少一条 signal, got {report1:?}"
    );
    let missing_pending = list_pending(&app, "missing_chunk").await;
    assert_eq!(
        missing_pending.len(),
        1,
        "missing_chunk 必须正好 1 条 pending（target archived）"
    );
    let sig = &missing_pending[0];
    assert_eq!(sig.severity, "error");
    assert_eq!(sig.source, "rule");
    assert!(
        sig.affected_chunk_ids.iter().any(|id| id == &target_hex),
        "affected_chunk_ids 必须包含 target id, got {:?}",
        sig.affected_chunk_ids
    );

    let broken_pending = list_pending(&app, "broken_link").await;
    assert!(
        broken_pending.is_empty(),
        "target 在 archived 时不该再产生 broken_link, got {broken_pending:?}"
    );

    // ── dedup：第二次 lint 不应再 insert 同 kind+title ──
    let report2 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("structural lint 2");
    assert_eq!(
        report2.new_signals, 0,
        "重复 lint 不该再 insert（dedup_key 命中）, got {report2:?}"
    );
    let still_pending = list_pending(&app, "missing_chunk").await;
    assert_eq!(still_pending.len(), 1, "pending 数量不应变化");

    // ── 依赖恢复：把 target 改回 active，再 sweep ──
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": target_oid },
            doc! { "$set": { "status": "active" } },
            None,
        )
        .await
        .expect("restore target");

    let sweep = gap_signals::sweep_stale_signals(&app.state.db, WS)
        .await
        .expect("sweep");
    assert!(
        sweep.stage1_auto_resolved >= 1,
        "依赖恢复后 sweep 应至少消解 1 条, got {sweep:?}"
    );

    let resolved = list_resolved(&app, "missing_chunk").await;
    assert_eq!(resolved.len(), 1, "missing_chunk 应有一条 auto_resolved");
    assert_eq!(
        resolved[0].resolution_note.as_deref(),
        Some("rule:dep_restored"),
        "resolution_note 必须区分自愈原因"
    );
}

#[tokio::test]
#[ignore]
async fn suggestion_emitted_when_unverified_and_blocked_then_resolved_when_verified() {
    let app = TestApp::start().await;

    // 草稿 chunk：integrity_status=needs_review, blocked_count_30d=5
    let oid = ObjectId::new();
    let hex = oid.to_hex();
    let mut chunk = base_chunk("常被 grounding 拦的草稿", "entity");
    chunk.id = Some(oid);
    chunk.integrity_status = Some("needs_review".to_string());
    chunk.usage_stats = Some(UsageStats {
        hit_count_30d: 1,
        blocked_count_30d: 5,
        last_used_at: None,
        last_blocked_reason: Some("missing_source_quote".to_string()),
    });

    insert_chunks(&app, &[chunk]).await;

    // ── lint：suggestion 必须出现 ──
    gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 1");
    let pending = list_pending(&app, "suggestion").await;
    assert_eq!(pending.len(), 1, "suggestion 必须正好 1 条 pending");
    assert_eq!(pending[0].severity, "info");
    assert_eq!(pending[0].source, "rule");
    assert_eq!(pending[0].affected_chunk_ids, vec![hex.clone()]);

    // ── dedup：第二次 lint 不应重复 insert ──
    let report2 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 2");
    assert_eq!(
        report2.new_signals, 0,
        "suggestion dedup 失败，第二次仍 insert"
    );

    // ── verify chunk → sweep 应 auto_resolved ──
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": oid },
            doc! { "$set": { "integrity_status": "verified" } },
            None,
        )
        .await
        .expect("verify chunk");

    let sweep = gap_signals::sweep_stale_signals(&app.state.db, WS)
        .await
        .expect("sweep");
    assert!(
        sweep.stage1_auto_resolved >= 1,
        "verify 后 sweep 必须消解, got {sweep:?}"
    );
    let resolved = list_resolved(&app, "suggestion").await;
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].resolution_note.as_deref(),
        Some("rule:chunk_verified")
    );
}

#[tokio::test]
#[ignore]
async fn contradiction_emitted_when_same_title_diff_first_paragraph_then_resolved_when_archived() {
    let app = TestApp::start().await;

    // 同题两条 chunk，body 首段不同 → contradiction
    let mut a = base_chunk("产品价格策略", "methodology");
    a.body = Some("策略一：阶梯价。\n\n详细说明……".to_string());

    let mut b = base_chunk("产品价格策略", "methodology");
    let b_oid = b.id.expect("oid");
    b.body = Some("策略二：固定价。\n\n详细说明……".to_string());

    insert_chunks(&app, &[a.clone(), b.clone()]).await;

    // ── lint：contradiction 应出现 ──
    gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 1");
    let pending = list_pending(&app, "contradiction").await;
    assert_eq!(pending.len(), 1, "contradiction 必须正好 1 条 pending");
    assert_eq!(pending[0].severity, "error");
    assert_eq!(pending[0].source, "rule");
    // affected_chunk_ids 应包含两条 chunk 的 id（顺序不强制）
    assert_eq!(
        pending[0].affected_chunk_ids.len(),
        2,
        "contradiction affected 必须含两条 chunk id"
    );

    // ── dedup：第二次 lint 不应重复 ──
    let report2 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 2");
    assert_eq!(
        report2.new_signals, 0,
        "contradiction dedup 失败，第二次仍 insert"
    );

    // ── 把 b archive 掉（同题只剩 a）→ sweep 应消解 ──
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": b_oid },
            doc! { "$set": { "status": "archived" } },
            None,
        )
        .await
        .expect("archive b");

    let sweep = gap_signals::sweep_stale_signals(&app.state.db, WS)
        .await
        .expect("sweep");
    assert!(
        sweep.stage1_auto_resolved >= 1,
        "同题去重后 sweep 必须消解 contradiction, got {sweep:?}"
    );
    let resolved = list_resolved(&app, "contradiction").await;
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].resolution_note.as_deref(),
        Some("rule:contradiction_resolved")
    );
}
