//! m029 存量清理语义验证：清理 contacts 的身份污染（webhook 建档 bug 遗留）。
//!
//! `TestApp::start()` 在空库上已跑过 m029（迁移账册存在），故这里手动插入受污染
//! contacts + 一条 roster 快照后，**直接调用** `m029::run_step` 验证三步治理：
//! 1. 删非真人 normal 记录（gh_/群），且 conversation_messages 不受影响。
//! 2. 真人 roster 命中 → nickname/avatar_url 回填正确。
//! 3. nickname=="Demi" 且 roster 未命中 → nickname 变 None（$unset）。
//! 4. managed 记录（哪怕 gh_/群）一律保留（可能被回填昵称，绝不删）。
//! 5. operation_state / agent_status 等运营字段零改动。
//! 6. 二次执行结果一致（幂等）。
//!
//! 默认 `#[ignore]`，需要 Docker；CI 用 `cargo test -- --ignored` 触发。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m029_cleanup_contact_identity as m029;

/// 取某 wxid 的 contact 原始 BSON 文档（绕过 Contact serde，直接看物理字段）。
async fn raw_contact(app: &common::TestApp, wxid: &str) -> Option<Document> {
    app.state
        .db
        .raw()
        .collection::<Document>("contacts")
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query raw contact")
}

#[tokio::test]
#[ignore]
async fn cleans_up_stale_contact_identity() {
    let app = common::TestApp::start().await;
    let contacts = app.state.db.raw().collection::<Document>("contacts");
    let messages = app
        .state
        .db
        .raw()
        .collection::<Document>("conversation_messages");
    let rosters = app
        .state
        .db
        .raw()
        .collection::<Document>("roster_snapshots");

    // 一条 roster 快照：真人 wxid_real 有正确昵称/头像；wxid_demi 不在 roster（未命中）。
    rosters
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "friends": [
                    {
                        "wxid": "wxid_real",
                        "nickname": "真实客户",
                        "remark": mongodb::bson::Bson::Null,
                        "avatar_url": "https://example.com/real.jpg",
                        "sex": mongodb::bson::Bson::Null,
                        "is_non_human": false,
                    }
                ],
                "total": 1_i64,
                "fetched_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert roster snapshot");

    // (A) 非真人 normal：gh_ 公众号 → 应删。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "gh_official_1",
                "nickname": "Demi",
                "agent_status": "normal",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert A");

    // (B) 非真人 normal：@chatroom 群 → 应删。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "12345@chatroom",
                "nickname": "Demi",
                "agent_status": "normal",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert B");

    // (C) 真人 normal，roster 命中 → 回填 nickname/avatar_url，不删。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "wxid_real",
                "nickname": "Demi",
                "agent_status": "normal",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert C");

    // (D) 真人 normal，nickname=Demi 且 roster 未命中 → $unset nickname（变 None），不删。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "wxid_demi",
                "nickname": "Demi",
                "agent_status": "normal",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert D");

    // (E) managed 且是 gh_（非真人）→ 一律保留（绝不删）。带运营字段验证零改动。
    contacts
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "gh_managed_keep",
                "nickname": "Demi",
                "agent_status": "managed",
                "operation_state": "need_discovery",
                "created_at": mongodb::bson::DateTime::now(),
                "updated_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert E");

    // 一条属于被删 gh_ contact 的历史消息 → 迁移绝不删消息。
    messages
        .insert_one(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "wxid": "gh_official_1",
                "direction": "inbound",
                "content": "历史消息不应被删",
            },
            None,
        )
        .await
        .expect("insert message");

    // 执行清理。
    m029::run_step(&app.state.db).await.expect("run m029");

    // (A)(B) 非真人 normal 已删。
    assert!(
        raw_contact(&app, "gh_official_1").await.is_none(),
        "A: gh_ normal 应被删"
    );
    assert!(
        raw_contact(&app, "12345@chatroom").await.is_none(),
        "B: @chatroom normal 应被删"
    );

    // conversation_messages 不受影响（被删 contact 的历史消息保留）。
    let msg_count = messages
        .count_documents(doc! { "wxid": "gh_official_1" }, None)
        .await
        .expect("count messages");
    assert_eq!(msg_count, 1, "被删 contact 的历史消息必须保留");

    // (C) 真人 roster 命中 → nickname/avatar_url 回填正确，未删。
    let c = raw_contact(&app, "wxid_real").await.expect("C: 真人应保留");
    assert_eq!(
        c.get_str("nickname").ok(),
        Some("真实客户"),
        "C: nickname 应从 roster 回填"
    );
    assert_eq!(
        c.get_str("avatar_url").ok(),
        Some("https://example.com/real.jpg"),
        "C: avatar_url 应从 roster 回填"
    );

    // (D) roster 未命中 + Demi → nickname $unset（字段不存在）。
    let d = raw_contact(&app, "wxid_demi").await.expect("D: 真人应保留");
    assert!(
        !d.contains_key("nickname"),
        "D: Demi 未命中应 $unset nickname"
    );

    // (E) managed gh_ 保留，且运营字段零改动。
    let e = raw_contact(&app, "gh_managed_keep")
        .await
        .expect("E: managed 一律保留");
    assert_eq!(
        e.get_str("agent_status").ok(),
        Some("managed"),
        "E: agent_status 零改动"
    );
    assert_eq!(
        e.get_str("operation_state").ok(),
        Some("need_discovery"),
        "E: operation_state 零改动"
    );

    // 二次执行幂等：结果一致（删已删 no-op / 回填同值 / 清已清 no-op）。
    m029::run_step(&app.state.db).await.expect("rerun m029");
    assert!(raw_contact(&app, "gh_official_1").await.is_none());
    let c2 = raw_contact(&app, "wxid_real").await.expect("C2 保留");
    assert_eq!(c2.get_str("nickname").ok(), Some("真实客户"));
    let d2 = raw_contact(&app, "wxid_demi").await.expect("D2 保留");
    assert!(!d2.contains_key("nickname"), "D2: 幂等，仍无 nickname");
    let e2 = raw_contact(&app, "gh_managed_keep").await.expect("E2 保留");
    assert_eq!(e2.get_str("agent_status").ok(), Some("managed"));
}
