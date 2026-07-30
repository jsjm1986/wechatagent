//! 用户运营池真人漏斗重设计 / Task 3：webhook 建档接线回归。
//!
//! 验三条性质（对应三个已修 bug）：
//! 1. 非真人（gh_ 公众号 / @chatroom 群）入站 → 消息仍落 conversation_messages，
//!    但 **不建 contact**（这类 wxid 不可能 managed）。
//! 2. 真人 wxid + roster 命中 → contact.nickname / avatar_url == roster 快照里的值
//!    （不再是 payload `_mcp.nickName` 里的账号自身昵称 "Demi"，也不再无头像）。
//! 3. 真人 wxid + roster 未命中 → contact 仍建成，但 nickname / avatar_url 为 None
//!    （best-effort 富化：拿不到就留空，绝不阻断建档）。
//!
//! 直调 public handler `wechat_webhook`（本仓 TestApp 是 state-only 工厂，无 HTTP
//! server；沿用 account_security_integration.rs 的「直调 route handler 真函数」惯例）。
//! payload 不带 appId → resolve_account_context 回落 default workspace/account。
//!
//! 默认 `#[ignore]`，需 Docker（testcontainers MongoDB）。
//! CI `integration` job 用 `cargo test --test webhook_contact_upsert_integration -- --ignored` 跑。
#![cfg(test)]

mod common;

use axum::extract::State;
use axum::http::HeaderMap;
use mongodb::bson::doc;
use serde_json::json;

use wechatagent::mcp::{write_roster_snapshot, RosterFriend};
use wechatagent::webhooks::wechat_webhook;

use crate::common::TestApp;

/// 构造一条不带 appId 的入站 webhook payload（回落 default account）。
/// 用小写驼峰键（手工/自测风格），走 find_string 回落分支——GeWe 大写驼峰路径
/// 由 msg_type 单测覆盖，这里只关心建档行为。
fn inbound_body(from_wxid: &str, content: &str, msg_id: &str) -> axum::body::Bytes {
    let v = json!({
        "fromWxid": from_wxid,
        "content": content,
        "msgId": msg_id,
    });
    axum::body::Bytes::from(serde_json::to_vec(&v).expect("serialize payload"))
}

/// 非真人（gh_ 公众号）入站：消息落库，但不建 contact。
#[tokio::test]
#[ignore]
async fn non_person_gh_persists_message_but_no_contact() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acct = app.state.config.default_account_id.clone();
    let gh_wxid = "gh_abc123";

    let resp = wechat_webhook(
        State(app.state.clone()),
        HeaderMap::new(),
        inbound_body(gh_wxid, "公众号推送内容", "msg_gh_1"),
    )
    .await
    .expect("webhook 应成功返回（非真人优雅跳过）");
    // 优雅短路：skipped=not_operatable_contact。
    assert_eq!(
        resp.0.get("skipped").and_then(|v| v.as_str()),
        Some("not_operatable_contact"),
        "gh_ 发件人应被标记 skipped=not_operatable_contact，实际：{:?}",
        resp.0
    );

    // 消息必须已落 conversation_messages（上游先落库，只是不建 contact）。
    let msg = app
        .state
        .db
        .messages()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acct, "contact_wxid": gh_wxid },
            None,
        )
        .await
        .expect("query message");
    assert!(
        msg.is_some(),
        "gh_ 入站消息必须落 conversation_messages（不建 contact ≠ 丢消息）"
    );

    // contacts 里绝不能有该 gh_ wxid。
    let contact = app
        .state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acct, "wxid": gh_wxid },
            None,
        )
        .await
        .expect("query contact");
    assert!(
        contact.is_none(),
        "gh_ 公众号绝不能进运营池 contacts，实际建成：{:?}",
        contact.map(|c| c.wxid)
    );
}

/// 非真人（@chatroom 群）入站：同样只落库、不建 contact。
#[tokio::test]
#[ignore]
async fn non_person_chatroom_persists_message_but_no_contact() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acct = app.state.config.default_account_id.clone();
    let room_wxid = "12345678@chatroom";

    let resp = wechat_webhook(
        State(app.state.clone()),
        HeaderMap::new(),
        inbound_body(room_wxid, "群消息", "msg_room_1"),
    )
    .await
    .expect("webhook 应成功返回（群消息优雅跳过）");
    assert_eq!(
        resp.0.get("skipped").and_then(|v| v.as_str()),
        Some("not_operatable_contact"),
        "@chatroom 发件人应被标记 skipped=not_operatable_contact，实际：{:?}",
        resp.0
    );

    let msg = app
        .state
        .db
        .messages()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acct, "contact_wxid": room_wxid },
            None,
        )
        .await
        .expect("query message");
    assert!(msg.is_some(), "群入站消息必须落 conversation_messages");

    let contact = app
        .state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acct, "wxid": room_wxid },
            None,
        )
        .await
        .expect("query contact");
    assert!(contact.is_none(), "@chatroom 群绝不能进运营池 contacts");
}

/// 真人 wxid + roster 命中：contact.nickname / avatar_url 来自 roster 快照，
/// **不是** payload 里的账号自身昵称。
#[tokio::test]
#[ignore]
async fn person_with_roster_hit_enriches_from_roster() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acct = app.state.config.default_account_id.clone();
    let wxid = "wxid_real_person_1";

    // seed roster 快照：该 wxid 有真实昵称 + 头像。
    write_roster_snapshot(
        &app.state,
        &ws,
        &acct,
        &[RosterFriend {
            wxid: wxid.to_string(),
            nickname: Some("张三".to_string()),
            remark: None,
            avatar_url: Some("http://img.example/zhangsan.jpg".to_string()),
            sex: Some(1),
            is_non_human: false,
        }],
    )
    .await
    .expect("seed roster snapshot");

    // payload 里塞一个会被旧 find_string 递归命中的 _mcp.nickName（账号自己昵称 "Demi"），
    // 验证富化后**不再**取到它。
    let body = axum::body::Bytes::from(
        serde_json::to_vec(&json!({
            "fromWxid": wxid,
            "content": "你好",
            "msgId": "msg_person_1",
            "_mcp": { "nickName": "Demi" }
        }))
        .unwrap(),
    );
    let _ = wechat_webhook(State(app.state.clone()), HeaderMap::new(), body)
        .await
        .expect("webhook 应成功");

    let contact = app
        .state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acct, "wxid": wxid },
            None,
        )
        .await
        .expect("query contact")
        .expect("真人 contact 必须建成");
    assert_eq!(
        contact.nickname.as_deref(),
        Some("张三"),
        "昵称必须来自 roster 快照，不能是账号自身昵称 Demi，实际：{:?}",
        contact.nickname
    );
    assert_eq!(
        contact.avatar_url.as_deref(),
        Some("http://img.example/zhangsan.jpg"),
        "头像必须来自 roster 快照，实际：{:?}",
        contact.avatar_url
    );
    // 建档默认 normal，不触发 Agent 流水线（本测试不排 LLM 响应）。
    assert_eq!(
        contact.agent_status,
        wechatagent::models::AgentStatus::Normal,
        "webhook 新建 contact 默认 normal"
    );
}

/// 真人 wxid + roster 未命中：contact 建成，但 nickname / avatar_url 为 None
/// （best-effort：roster 拿不到就留空，绝不阻断建档、绝不写 payload 里的脏昵称）。
#[tokio::test]
#[ignore]
async fn person_without_roster_hit_leaves_identity_none() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acct = app.state.config.default_account_id.clone();
    let wxid = "wxid_real_person_2";

    // roster 快照存在，但不含本 wxid（未命中分支）。
    write_roster_snapshot(
        &app.state,
        &ws,
        &acct,
        &[RosterFriend {
            wxid: "wxid_someone_else".to_string(),
            nickname: Some("别人".to_string()),
            remark: None,
            avatar_url: Some("http://img.example/other.jpg".to_string()),
            sex: Some(0),
            is_non_human: false,
        }],
    )
    .await
    .expect("seed roster snapshot");

    let body = axum::body::Bytes::from(
        serde_json::to_vec(&json!({
            "fromWxid": wxid,
            "content": "在吗",
            "msgId": "msg_person_2",
            "_mcp": { "nickName": "Demi" }
        }))
        .unwrap(),
    );
    let _ = wechat_webhook(State(app.state.clone()), HeaderMap::new(), body)
        .await
        .expect("webhook 应成功");

    let contact = app
        .state
        .db
        .contacts()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acct, "wxid": wxid },
            None,
        )
        .await
        .expect("query contact")
        .expect("真人 contact 必须建成（即使 roster 未命中）");
    assert_eq!(
        contact.nickname, None,
        "roster 未命中时 nickname 必须为 None（绝不回落 payload 脏昵称），实际：{:?}",
        contact.nickname
    );
    assert_eq!(
        contact.avatar_url, None,
        "roster 未命中时 avatar_url 必须为 None，实际：{:?}",
        contact.avatar_url
    );
}
