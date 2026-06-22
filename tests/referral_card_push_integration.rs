//! 专属顾问名片引荐端到端：审核门（enabled+approved 才可被加载） + outbox
//! 名片条目按 card_id 幂等。需 Docker(testcontainers Mongo)，默认 `#[ignore]`，
//! CI integration job 用 `cargo test --test referral_card_push_integration -- --ignored` 跑。
//!
//! 可见性说明（决定本文件能覆盖到哪些层）：
//! - `outbox::enqueue` / `EnqueueRequest`(含 `referral_card_id`) / `EnqueueOutcome` /
//!   `OutboxStatus` 在 `agent/mod.rs` 对外 `pub use`，故名片 outbox 幂等可从 tests
//!   crate 完整端到端验证（见 `namecard_outbox_entry_idempotent_per_card`）。
//! - `decision::load_referral_cards` / `build_referral_cards_filter` /
//!   `referral::{filter_referral_candidates, assist_mode_active, validate_card_sendable}`
//!   均为 `pub(crate)` 且 `referral` 模块未对外 `pub use`，**跨 crate 不可见**。
//!   为不破坏封装（不为测试把 pub(crate) 改 pub），审核门改测公开路径：直接对
//!   `Database::referral_cards()` 集合做 CRUD round-trip，断言用与生产
//!   `build_referral_cards_filter`(decision.rs) **同形**的过滤条件
//!   （`enabled:true` + `review_status:"approved"`）只命中已审核+已启用的名片。
//! - 辅助模式短路（`assist_mode_active` 的 override>account>默认关 真值表）为不可见的
//!   `pub(crate)` 纯函数，已由 `src/agent/referral.rs` 内联单测
//!   `assist_mode_override_beats_account_flag` 覆盖，本文件不重复（无法从 tests crate
//!   访问，且强行从公开 gateway 走需真 LLM 决策注入指令，超出本 Task 范围）。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, DateTime};
use wechatagent::agent::{enqueue, EnqueueOutcome, EnqueueRequest, OutboxStatus};
use wechatagent::models::ReferralCard;

/// 构造一张名片 fixture（snake_case 落库，与 OperationDomainConfig 同款，无 rename_all）。
fn make_card(
    account_id: Option<&str>,
    display_name: &str,
    target_wxid: &str,
    enabled: bool,
    review_status: &str,
    target_stages: &[&str],
) -> ReferralCard {
    let now = DateTime::now();
    ReferralCard {
        id: None,
        workspace_id: "default".to_string(),
        account_id: account_id.map(ToString::to_string),
        target_wxid: target_wxid.to_string(),
        display_name: display_name.to_string(),
        send_trigger_hint: "客户要签约/到店时引荐".to_string(),
        target_stages: target_stages.iter().map(|s| s.to_string()).collect(),
        tags: vec![],
        enabled,
        review_status: review_status.to_string(),
        review_note: None,
        created_at: now,
        updated_at: now,
    }
}

/// 与生产 `decision::build_referral_cards_filter` 同形的「可加载名片」过滤条件。
/// 该函数为 `pub(crate)` 跨 crate 不可见，这里以同一语义内联，验证集合层行为一致。
fn loadable_filter(account_id: &str) -> mongodb::bson::Document {
    doc! {
        "workspace_id": "default",
        "$or": [
            { "account_id": null },
            { "account_id": account_id }
        ],
        "enabled": true,
        "review_status": "approved",
    }
}

/// 构造一个发名片的 [`EnqueueRequest`]（`referral_card_id` 有值 → content 可空）。
fn namecard_enqueue_request(
    run_id: &str,
    source_event_id: &str,
    contact_wxid: &str,
    referral_card_id: &str,
) -> EnqueueRequest {
    EnqueueRequest {
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        run_id: run_id.to_string(),
        decision_id: None,
        source_event_id: source_event_id.to_string(),
        source_kind: "inbound_message".to_string(),
        // 名片条目允许空 content（content_required_for 对 referral_card_id 放行）。
        content: String::new(),
        media_asset_id: None,
        referral_card_id: Some(referral_card_id.to_string()),
        max_attempts: 3,
    }
}

// ── Test 1: 审核门——仅 enabled + approved 名片可被「加载」 ─────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn only_approved_enabled_card_is_loadable() {
    let app = common::TestApp::start().await;
    let state = app.state.clone();
    let collection = state.db.referral_cards();
    let account_id = "default";

    // 1. 插入一张 draft + disabled 名片。
    let draft = make_card(
        Some(account_id),
        "老王",
        "wxid_boss_wang",
        false,
        "draft",
        &["意向"],
    );
    let insert = collection
        .insert_one(&draft, None)
        .await
        .expect("insert draft card");
    let card_id = insert.inserted_id.as_object_id().expect("card oid");

    // 2. 用与生产 build_referral_cards_filter 同形的过滤条件 → draft+disabled 不命中。
    let loadable_before: Vec<ReferralCard> = {
        use futures::TryStreamExt;
        let cursor = collection
            .find(loadable_filter(account_id), None)
            .await
            .expect("find loadable before");
        cursor.try_collect().await.expect("collect before")
    };
    assert!(
        loadable_before.is_empty(),
        "draft+disabled card must NOT be loadable, got {loadable_before:?}"
    );

    // 3. 人类审核：置 enabled=true + review_status=approved。
    collection
        .update_one(
            doc! { "_id": card_id },
            doc! { "$set": { "enabled": true, "review_status": "approved" } },
            None,
        )
        .await
        .expect("approve card");

    // 4. 现在可被加载，且就是这张。
    let loadable_after: Vec<ReferralCard> = {
        use futures::TryStreamExt;
        let cursor = collection
            .find(loadable_filter(account_id), None)
            .await
            .expect("find loadable after");
        cursor.try_collect().await.expect("collect after")
    };
    assert_eq!(
        loadable_after.len(),
        1,
        "approved+enabled card must be loadable"
    );
    assert_eq!(loadable_after[0].id, Some(card_id));
    assert_eq!(loadable_after[0].display_name, "老王");

    // 5. 另插一张 approved 但 disabled 的名片 → 仍不被加载（enabled 门独立生效）。
    let approved_but_disabled = make_card(
        Some(account_id),
        "老李",
        "wxid_boss_li",
        false,
        "approved",
        &[],
    );
    collection
        .insert_one(&approved_but_disabled, None)
        .await
        .expect("insert approved-but-disabled");
    let loadable_final: Vec<ReferralCard> = {
        use futures::TryStreamExt;
        let cursor = collection
            .find(loadable_filter(account_id), None)
            .await
            .expect("find loadable final");
        cursor.try_collect().await.expect("collect final")
    };
    assert_eq!(
        loadable_final.len(),
        1,
        "disabled-but-approved card must NOT leak into loadable set, got {loadable_final:?}"
    );
    assert_eq!(loadable_final[0].id, Some(card_id), "only 老王 remains loadable");
}

// ── Test 2: outbox 名片条目按 card_id 幂等 ─────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn namecard_outbox_entry_idempotent_per_card() {
    let app = common::TestApp::start().await;
    let state = app.state.clone();
    let contact_wxid = "user_namecard_idem";

    // 同 (run_id, contact_wxid, referral_card_id) 入队两次 → 第一次 Created、第二次 IdempotentSkip。
    // 名片走 synthetic_namecard 形态（compute_synthetic_key 含 card_id），content 为空也不挡。
    let req = namecard_enqueue_request("run_card_idem", "evt_card_1", contact_wxid, "card_aaa");

    let first = enqueue(&state, req.clone()).await.expect("first enqueue ok");
    let first_id = match first {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("first enqueue expected Created, got {other:?}"),
    };

    let second = enqueue(&state, req.clone())
        .await
        .expect("second enqueue ok");
    match second {
        EnqueueOutcome::IdempotentSkip { .. } => {}
        other => panic!("second enqueue expected IdempotentSkip, got {other:?}"),
    }

    // 同 run 不同 referral_card_id → 应当独立入队（synthetic_namecard key 含 card_id）。
    let other_card = namecard_enqueue_request(
        "run_card_idem",
        "evt_card_1",
        contact_wxid,
        "card_bbb",
    );
    let other = enqueue(&state, other_card)
        .await
        .expect("other-card enqueue ok");
    let other_id = match other {
        EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        other => panic!("other-card enqueue expected Created, got {other:?}"),
    };
    assert_ne!(
        first_id, other_id,
        "different card_id must yield a distinct outbox row"
    );

    // DB 落地：该 contact 名下恰好两行名片 outbox，且都带 referral_card_id、状态 pending。
    let collection = state.db.collection_agent_send_outbox();
    let total = collection
        .count_documents(doc! { "contact_wxid": contact_wxid }, None)
        .await
        .expect("count namecard outbox rows");
    assert_eq!(
        total, 2,
        "two unique card_ids → two outbox rows, the dup is deduped by unique idempotency_key"
    );

    for (id, expected_card) in [(first_id, "card_aaa"), (other_id, "card_bbb")] {
        let entry = collection
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("query outbox entry")
            .expect("entry exists");
        assert_eq!(
            entry.referral_card_id.as_deref(),
            Some(expected_card),
            "outbox row must carry its referral_card_id"
        );
        assert_eq!(
            entry.status,
            OutboxStatus::Pending.as_str(),
            "freshly enqueued namecard entry is pending"
        );
        assert!(
            entry.media_asset_id.is_none(),
            "namecard entry must not also be a media entry (互斥)"
        );
    }
}
