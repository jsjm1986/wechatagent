//! KD-05 端到端：改派与推送时刻的 sent 对账口径 + m031 回填历史行。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。
//!
//! 28 号交叉核验裁决（复刻漂移 #5）修正说明：本文件用例 1 原先复刻的是 KD-05
//! 初版"改派即 $set last_pushed_at_ms=改派时刻"，而生产 `reassign_escalation`
//! （src/agent/escalation/ledger.rs:1103-1115 亲验）早已改为 **$unset** ——
//! 推送时刻只在 Outbox 确认送达后由 `reconcile_principal_card_deliveries_once`
//! （ledger.rs:260-265）写回，保证"每位决策人从 sent 对账时刻起拿完整超时窗"。
//! `reassign_escalation` 是 pub(crate) 无法跨 crate 直调（真实接线由
//! ask_human_phase1_e2e 驱动生产函数守护）；本用例复刻其 update **形状**锁定
//! 语义契约：改派清空 → 未送达行不进 `$type:number` 类推送过滤 → 送达对账重置。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m031_backfill_escalation_last_pushed_at;

use crate::common::TestApp;

/// KD-05（sent 对账口径）：改派 $unset last_pushed_at_ms；送达确认后才重置。
#[tokio::test]
#[ignore]
async fn reassign_unsets_last_pushed_at_until_delivery_confirms() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();
    let old_ms = 1_000_000i64;
    let delivered_ms = 9_500_000i64;

    // 首推已送达：principal=A，last_pushed_at_ms=首推送达时刻。
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
                "protocol": doc! {
                    "delivery_generation": 1i64,
                    "delivery_state": "sent",
                },
            },
            None,
        )
        .await
        .expect("seed pending");

    // 改派到 B：复刻生产 reassign_escalation 的 update 形状（ledger.rs:1103-1115）——
    // $set 新决策人 + delivery_state 回 pending_enqueue、$inc generation、
    // **$unset last_pushed_at_ms / delivery_outbox_id**（不动 created_at）。
    raw.collection::<Document>("agent_principal_escalations")
        .update_one(
            doc! { "short_code": "E1A2", "workspace_id": &ws },
            doc! {
                "$set": {
                    "principal_wxid": "B",
                    "protocol.principal_account_id": "acc",
                    "protocol.delivery_state": "pending_enqueue",
                    "updated_at": mongodb::bson::DateTime::now(),
                },
                "$inc": { "protocol.delivery_generation": 1i64 },
                "$unset": {
                    "protocol.delivery_outbox_id": "",
                    "last_pushed_at_ms": "",
                },
            },
            None,
        )
        .await
        .expect("reassign");

    let row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "principal_wxid": "B", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    // 改派后推送时刻**为空**：新决策人尚未真正收到卡，不得从改派瞬间起算超时窗。
    assert!(
        row.get_i64("last_pushed_at_ms").is_err() && !row.contains_key("last_pushed_at_ms"),
        "改派必须 $unset last_pushed_at_ms（推送时刻只能来自送达对账），实际 row={row:?}"
    );
    // created_at 保持不变（真实创建审计）。
    assert_eq!(
        row.get_datetime("created_at").unwrap().timestamp_millis(),
        old_ms,
        "created_at 不被改派篡改（保真实创建审计）"
    );
    // 行为后果：骚扰门/超时扫描按 `last_pushed_at_ms: {$type:"number"}` 过滤
    // （ledger.rs:1138,1188,1211 同口径）——未送达的改派行天然不计入推送。
    let pushed_count = raw
        .collection::<Document>("agent_principal_escalations")
        .count_documents(
            doc! {
                "workspace_id": &ws,
                "principal_wxid": "B",
                "last_pushed_at_ms": { "$type": "number" },
            },
            None,
        )
        .await
        .expect("count pushes");
    assert_eq!(pushed_count, 0, "改派后未送达前不得计入推送次数");

    // 送达确认：复刻 reconcile 的 sent 写回（ledger.rs:260-265）——delivery_state=sent
    // 时才写 last_pushed_at_ms=送达时刻。
    raw.collection::<Document>("agent_principal_escalations")
        .update_one(
            doc! { "short_code": "E1A2", "workspace_id": &ws },
            doc! { "$set": {
                "protocol.delivery_state": "sent",
                "last_pushed_at_ms": delivered_ms,
            }},
            None,
        )
        .await
        .expect("reconcile sent");
    let row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "principal_wxid": "B", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        row.get_i64("last_pushed_at_ms").unwrap(),
        delivered_ms,
        "送达对账后推送时刻重置为送达时刻（超时窗从此刻起算）"
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
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        old_row.get_i64("last_pushed_at_ms").unwrap(),
        legacy_created,
        "老行 last_pushed_at_ms 回填成 created_at"
    );

    let new_row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "short_code": "NEW1", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(
        new_row.get_i64("last_pushed_at_ms").unwrap(),
        has_field_pushed,
        "已有 last_pushed_at_ms 的行不被迁移覆盖（幂等）"
    );
}
