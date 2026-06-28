//! 销售素材上传 → 审核 → 发送 的端到端数据流。需 Docker(testcontainers Mongo)，
//! 默认 #[ignore]，CI integration job 跑。
//!
//! Task 11 回填真实断言。覆盖两条端到端数据流：
//! 1. `upload_then_review_then_only_approved_is_sendable`：真实 DB 读写——draft 态
//!    被 `sendable_query` 过滤、approved 态被返回；再按 `target_stages` 命中/不命中
//!    customer_stage（选材纯函数语义）。
//! 2. `media_outbox_entry_is_idempotent_per_asset`：真实 `agent::enqueue` 入口——
//!    同 (run, contact, media_asset_id) 二次入队 → IdempotentSkip；同 run 不同
//!    media_asset_id → 两条都 Created（验证幂等键含 asset_id、不撞键）。
//!
//! 覆盖策略说明：选材模块 `load_sendable_assets` / `filter_sendable_candidates`
//! 是 `pub(crate)`，tests/ 独立 crate 引不到，故 Test 1 用与 `load_sendable_assets`
//! **完全相同**的 Mongo 查询条件直接读 content_assets（验证 DB 真实读写 + 索引/字段
//! 序列化），并复刻 `filter_sendable_candidates` 的 stage 命中谓词（该纯函数的逻辑
//! 已在 Task 5 的 lib 单测覆盖）。outbox 链路（Test 2）则是完整 public API 端到端。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOptions;
use futures::TryStreamExt;
use wechatagent::agent::{enqueue, EnqueueOutcome, EnqueueRequest};
use wechatagent::models::{AgentStatus, Contact, ContentAsset};

// ── fixtures ──────────────────────────────────────────────────────────────

/// 构造一条销售素材文件。默认 sendable=true / media_type="file"，
/// review_status / target_stages 由调用方指定。
fn make_file_asset(
    title: &str,
    review_status: &str,
    target_stages: Option<Vec<String>>,
) -> ContentAsset {
    let now = DateTime::now();
    ContentAsset {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: None, // 全局素材（account_id=null）——load_sendable_assets 的 $or 覆盖
        kind: "media_file".to_string(),
        title: title.to_string(),
        body: None,
        tags: Vec::new(),
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some("file".to_string()),
        file_path: Some("media/test/demo.pdf".to_string()),
        file_name: Some("demo.pdf".to_string()),
        file_size: Some(1024),
        mime_type: Some("application/pdf".to_string()),
        file_sha256: Some("deadbeef".to_string()),
        sendable: Some(true),
        send_trigger_hint: Some("客户问报价时发".to_string()),
        target_stages,
        expression_pref: Some("file_support".to_string()),
        requires_principal_approval: Some(false),
        review_status: Some(review_status.to_string()),
        review_note: None,
        min_inject_tier: None,
        created_at: now,
        updated_at: now,
    }
}

fn make_managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("集成测试客户".to_string()),
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        manual_tags_updated_at: None,
        manual_tags_by: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: Some("need_discovery".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(7),
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: Some(now),
        last_inbound_at: Some(now),
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        outcome_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

/// 与 `decision::load_sendable_assets` **完全相同**的查询条件（pub(crate) 引不到，
/// 故在此复刻；任何一方改条件、这里跟着改，保证集成测试盯住生产查询语义）。
async fn sendable_query(
    state: &wechatagent::routes::AppState,
    account_id: &str,
) -> Vec<ContentAsset> {
    let mut cursor = state
        .db
        .content_assets()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id }
                ],
                "sendable": true,
                "review_status": "approved",
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(30)
                .build(),
        )
        .await
        .expect("query content_assets");
    let mut out = Vec::new();
    while let Some(a) = cursor.try_next().await.expect("cursor next") {
        out.push(a);
    }
    out
}

/// 复刻 `media_send::filter_sendable_candidates` 的 target_stages 命中谓词：
/// None / 空 = 总命中；非空则需包含当前 stage。该纯函数本体已在 Task 5 lib 单测覆盖，
/// 这里复用以让集成测试断言"命中/不命中"端到端结果。
fn stage_hits(asset: &ContentAsset, customer_stage: Option<&str>) -> bool {
    match (&asset.target_stages, customer_stage) {
        (None, _) => true,
        (Some(stages), _) if stages.is_empty() => true,
        (Some(stages), Some(cs)) => stages.iter().any(|s| s == cs),
        (Some(_), None) => false,
    }
}

// ── Test 1: upload → review → only approved is sendable ─────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn upload_then_review_then_only_approved_is_sendable() {
    let app = common::TestApp::start().await;
    let state = &app.state;

    // 1. 插入一条 draft 素材（sendable=true, media_type="file", target_stages=["意向"]）。
    let asset = make_file_asset("产品报价单", "draft", Some(vec!["意向".to_string()]));
    let asset_id = asset.id.expect("asset id");
    state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("insert draft asset");

    // 2. draft 态 → 不在可发清单（review_status != "approved" 被过滤）。
    let loaded = sendable_query(state, "default").await;
    assert!(
        !loaded.iter().any(|a| a.id == Some(asset_id)),
        "draft 素材不应出现在可发清单"
    );

    // 3. 审核通过：update review_status="approved"。
    state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": asset_id },
            doc! { "$set": { "review_status": "approved" } },
            None,
        )
        .await
        .expect("approve asset");

    // 4. approved 态 → 出现在可发清单。
    let loaded = sendable_query(state, "default").await;
    let got = loaded
        .iter()
        .find(|a| a.id == Some(asset_id))
        .expect("approved 素材应出现在可发清单");
    assert_eq!(got.review_status.as_deref(), Some("approved"));
    assert_eq!(got.media_type.as_deref(), Some("file"));
    assert_eq!(got.sendable, Some(true));

    // 5. stage 命中："意向" 命中、"已成交" 不命中。
    assert!(
        stage_hits(got, Some("意向")),
        "target_stages=[意向] 应命中 customer_stage=意向"
    );
    assert!(
        !stage_hits(got, Some("已成交")),
        "target_stages=[意向] 不应命中 customer_stage=已成交"
    );
}

// ── Test 2: outbox 媒体条目按 asset 幂等 ────────────────────────────────────

fn media_enqueue_request(run_id: &str, contact_wxid: &str, media_asset_id: &str) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        run_id: run_id.to_string(),
        decision_id: None,
        // 模拟入站消息触发：source_event_id 非空。媒体条目仍应走 synthetic_media
        // 路径（key 含 asset_id），不撞文本 content_hash 键。
        source_event_id: "evt_media_1".to_string(),
        source_kind: "inbound_message".to_string(),
        content: String::new(), // 媒体条目允许空 content（content_required_for=false）
        media_asset_id: Some(media_asset_id.to_string()),
        referral_card_id: None,
        max_attempts: 3,
    }
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn media_outbox_entry_is_idempotent_per_asset() {
    let app = common::TestApp::start().await;
    let state = &app.state;

    let contact = make_managed_contact("user_media");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let asset_a = ObjectId::new().to_hex();
    let asset_b = ObjectId::new().to_hex();

    // 第一次入队同 (run, contact, asset_a) → Created。
    let first = enqueue(state, media_enqueue_request("run_media", &contact.wxid, &asset_a))
        .await
        .expect("first enqueue ok");
    let first_key = match first {
        EnqueueOutcome::Created { idempotency_key, .. } => idempotency_key,
        other => panic!("expected Created, got {other:?}"),
    };

    // 第二次入队同 (run, contact, asset_a) → IdempotentSkip（永不发两次）。
    let second = enqueue(state, media_enqueue_request("run_media", &contact.wxid, &asset_a))
        .await
        .expect("second enqueue ok");
    match second {
        EnqueueOutcome::IdempotentSkip { idempotency_key } => {
            assert_eq!(
                idempotency_key, first_key,
                "同 asset 二次入队应命中同一幂等键"
            );
        }
        other => panic!("expected IdempotentSkip, got {other:?}"),
    }

    // 同 run 不同 media_asset_id → 第二条也 Created（幂等键含 asset_id、不撞键）。
    let other = enqueue(state, media_enqueue_request("run_media", &contact.wxid, &asset_b))
        .await
        .expect("other-asset enqueue ok");
    let other_key = match other {
        EnqueueOutcome::Created { idempotency_key, .. } => idempotency_key,
        other => panic!("expected Created for different asset, got {other:?}"),
    };
    assert_ne!(
        other_key, first_key,
        "不同 media_asset_id 的幂等键必须不同（key 含 asset_id）"
    );

    // 落库核对：outbox 里应有恰好 2 条本 run 的媒体条目（asset_a / asset_b 各一）。
    let count = state
        .db
        .collection_agent_send_outbox()
        .count_documents(
            doc! { "run_id": "run_media", "media_asset_id": { "$ne": null } },
            None,
        )
        .await
        .expect("count media outbox entries");
    assert_eq!(count, 2, "应有两条不同 asset 的媒体 outbox 条目");
}
