//! 主动发送台账端到端：ledger CRUD round-trip + 转化字段可空回填 round-trip +
//! workspace_id scope 隔离。需 Docker(testcontainers Mongo)，默认 `#[ignore]`，
//! CI integration job 用 `cargo test --test send_ledger_integration -- --ignored` 跑。
//!
//! 可见性说明（决定本文件能覆盖到哪些层）：
//! - `scan_send_ledger_outcomes` / `build_ledger_entry` / `record_send` 为 `pub(crate)`
//!   且 `send_ledger` 模块未对外 `pub use`，**跨 crate 不可见**（转化判定纯函数已由
//!   `src/agent/send_ledger.rs` 内联单测覆盖，本文件不重复）。
//! - 故集成测试走**公开路径**：直接对 `Database::agent_send_ledger()`(pub accessor)
//!   做集合 CRUD round-trip + workspace scope 验证（IDOR 数据层前提）。
#![cfg(test)]

mod common;

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use wechatagent::models::AgentSendLedger;

/// 构造一条台账 fixture（snake_case 落库，与 models.rs 上 AgentSendLedger 同款）。
fn make_row(workspace: &str, contact: &str, kind: &str, target: &str) -> AgentSendLedger {
    AgentSendLedger {
        id: None,
        workspace_id: workspace.into(),
        account_id: "acct1".into(),
        contact_wxid: contact.into(),
        send_kind: kind.into(),
        target_id: target.into(),
        target_title: "fixture".into(),
        run_id: "run1".into(),
        trigger_reason: None,
        customer_stage_at_send: Some("意向".into()),
        sent_at: DateTime::now(),
        responded: None,
        response_window_hours: None,
        stage_advanced: None,
        outcome_evaluated_at: None,
    }
}

// ── Test 1: 插入 → 回填转化字段 → 读回断言 ────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn ledger_roundtrip_and_outcome_update() {
    let app = common::TestApp::start().await;
    let coll = app.state.db.agent_send_ledger();

    // 1. 插入一条未评估的台账（转化字段全空）。
    let res = coll
        .insert_one(make_row("ws1", "wxA", "media", "asset1"), None)
        .await
        .expect("insert ledger row");
    let id = res.inserted_id.as_object_id().expect("ledger oid");

    // 2. 插入即能读回，且转化字段为空（回扫前状态）。
    let before = coll
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one before")
        .expect("row exists before backfill");
    assert_eq!(before.workspace_id, "ws1");
    assert_eq!(before.send_kind, "media");
    assert_eq!(before.target_id, "asset1");
    assert_eq!(before.customer_stage_at_send.as_deref(), Some("意向"));
    assert_eq!(before.responded, None);
    assert_eq!(before.stage_advanced, None);
    assert!(before.outcome_evaluated_at.is_none());

    // 3. 回扫回填转化字段。
    coll.update_one(
        doc! { "_id": id },
        doc! { "$set": {
            "responded": true,
            "stage_advanced": false,
            "outcome_evaluated_at": DateTime::now(),
        } },
        None,
    )
    .await
    .expect("update outcome fields");

    // 4. 回填后能读回新值（Option<bool>/Option<DateTime> 正确 round-trip）。
    let back = coll
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find_one after")
        .expect("row exists after backfill");
    assert_eq!(back.responded, Some(true));
    assert_eq!(back.stage_advanced, Some(false));
    assert!(back.outcome_evaluated_at.is_some());
}

// ── Test 2: 按 workspace_id 查询是 workspace-scoped（IDOR 数据层前提）────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn ledger_query_is_workspace_scoped() {
    let app = common::TestApp::start().await;
    let coll = app.state.db.agent_send_ledger();

    // 两个不同 workspace 各插一条。
    coll.insert_one(make_row("wsA", "wxA", "media", "a1"), None)
        .await
        .expect("insert wsA row");
    coll.insert_one(make_row("wsB", "wxB", "namecard", "c1"), None)
        .await
        .expect("insert wsB row");

    // 只查 wsA：不能看到 wsB 的条目（IDOR 防护的数据层前提）。
    let mut cursor = coll
        .find(doc! { "workspace_id": "wsA" }, None)
        .await
        .expect("find wsA scope");
    let mut count = 0;
    while let Some(row) = cursor.try_next().await.expect("cursor next") {
        assert_eq!(row.workspace_id, "wsA");
        count += 1;
    }
    assert_eq!(count, 1, "wsA scope 必须只返回自己的 1 条台账");
}
