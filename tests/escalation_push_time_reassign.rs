//! KD-05 端到端：改派刷新 last_pushed_at_ms + 骚扰门按它取推送时刻 + m031 回填历史行。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m031_backfill_escalation_last_pushed_at;

use crate::common::TestApp;

/// KD-05：改派把 last_pushed_at_ms 刷新为改派时刻（≠ 陈旧 created_at），按 last_pushed_at_ms
/// sort 取最近推送时刻返回改派时刻。用 raw Document 模拟 reassign 的 $set（reassign_escalation
/// 是 pub(crate) 不可跨 crate 直调）。
#[tokio::test]
#[ignore]
async fn reassign_refreshes_last_pushed_at_ms() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();
    let old_ms = 1_000_000i64;
    let reassign_ms = 9_000_000i64;

    // 首推：principal=A，created_at/last_pushed_at_ms=old。
    raw.collection::<Document>("agent_principal_escalations")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "contact_wxid": "cust",
                "short_code": "E1A2",
                "status": "pending",
                "category": "out_of_scope_decision",
                "reason": "r",
                "question_for_principal": "q",
                "principal_wxid": "A",
                "is_generalizable": false,
                "knowledge_proposal_emitted": false,
                "created_at": mongodb::bson::DateTime::from_millis(old_ms),
                "updated_at": mongodb::bson::DateTime::from_millis(old_ms),
                "last_pushed_at_ms": old_ms,
            },
            None,
        )
        .await
        .expect("seed pending");

    // 模拟 reassign 到 B：$set principal_wxid + last_pushed_at_ms=改派时刻（不动 created_at）。
    raw.collection::<Document>("agent_principal_escalations")
        .update_one(
            doc! { "short_code": "E1A2", "workspace_id": &ws },
            doc! { "$set": { "principal_wxid": "B", "last_pushed_at_ms": reassign_ms } },
            None,
        )
        .await
        .expect("reassign");

    // 按 last_pushed_at_ms 取 B 的最近推送时刻 → 改派时刻（非陈旧 created_at）。
    let row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "principal_wxid": "B", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(row.get_i64("last_pushed_at_ms").unwrap(), reassign_ms, "改派须刷新 last_pushed_at_ms 为改派时刻");
    // created_at 保持不变（真实创建审计）。
    assert_eq!(
        row.get_datetime("created_at").unwrap().timestamp_millis(),
        old_ms,
        "created_at 不被改派篡改（保真实创建审计）"
    );
}

/// m031：缺 last_pushed_at_ms 的历史行回填成 created_at；已有值的行不被覆盖（幂等）。
#[tokio::test]
#[ignore]
async fn m031_backfills_last_pushed_at_from_created_at() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();
    let legacy_created = 2_000_000i64;
    let has_field_created = 3_000_000i64;
    let has_field_pushed = 8_000_000i64;

    // 老行：无 last_pushed_at_ms。
    raw.collection::<Document>("agent_principal_escalations")
        .insert_one(
            doc! {
                "workspace_id": &ws, "account_id": "acc", "contact_wxid": "c1",
                "short_code": "OLD1", "status": "pending", "category": "out_of_scope_decision",
                "reason": "r", "question_for_principal": "q", "principal_wxid": "A",
                "is_generalizable": false, "knowledge_proposal_emitted": false,
                "created_at": mongodb::bson::DateTime::from_millis(legacy_created),
                "updated_at": mongodb::bson::DateTime::from_millis(legacy_created),
            },
            None,
        )
        .await
        .expect("seed legacy");

    // 新行：已有 last_pushed_at_ms（迁移不得覆盖）。
    raw.collection::<Document>("agent_principal_escalations")
        .insert_one(
            doc! {
                "workspace_id": &ws, "account_id": "acc", "contact_wxid": "c2",
                "short_code": "NEW1", "status": "pending", "category": "out_of_scope_decision",
                "reason": "r", "question_for_principal": "q", "principal_wxid": "A",
                "is_generalizable": false, "knowledge_proposal_emitted": false,
                "created_at": mongodb::bson::DateTime::from_millis(has_field_created),
                "updated_at": mongodb::bson::DateTime::from_millis(has_field_created),
                "last_pushed_at_ms": has_field_pushed,
            },
            None,
        )
        .await
        .expect("seed new");

    m031_backfill_escalation_last_pushed_at::run_step(&app.state.db)
        .await
        .expect("run m031");

    let old_row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "short_code": "OLD1", "workspace_id": &ws }, None)
        .await.expect("find").expect("exists");
    assert_eq!(
        old_row.get_i64("last_pushed_at_ms").unwrap(), legacy_created,
        "老行 last_pushed_at_ms 回填成 created_at"
    );

    let new_row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "short_code": "NEW1", "workspace_id": &ws }, None)
        .await.expect("find").expect("exists");
    assert_eq!(
        new_row.get_i64("last_pushed_at_ms").unwrap(), has_field_pushed,
        "已有 last_pushed_at_ms 的行不被迁移覆盖（幂等）"
    );
}
