//! CONC-1：memory_card OCC 并发写语义（缺陷 #8 空壳落实，2026-08-13）。
//!
//! 默认 `#[ignore]`，需 Docker（testcontainers MongoDB）。
//!
//! ## 驱动面（只经公共 API，零生产代码改动）
//! 经 `wechatagent::agent::load_or_create_operating_memory`（pub 导出，先例见
//! operating_memory_insert_idempotent.rs）驱动其**种子卡 OCC 升级分支**
//! （memory.rs:1105-1163）：已有行的生效卡无信号、contact 种子卡有信号时，
//! 用 `occ_memory_filter(prev_version)` 版本谓词做 read-modify-write——
//! 这正是 CONC-1 的 OCC 模板（gateway.rs `apply_operating_memory_update` 的
//! memory_card 写点镜像同一 filter 构造器；该镜像是 pub(crate)，无法跨 crate
//! 直调，其 filter 形状由 memory.rs lib 单测锁定）。
//!
//! ## 锁定的不变量（CONC-1）
//! 1. 并发 N 路 writer **零 Err**：输 OCC 的一方静默让位（重读赢家结果返回），
//!    绝不透传错误、绝不 last-write-wins 覆盖；
//! 2. 版本单调且**恰一次 bump**：prev=0 → 终版恒 1（版本谓词保证至多一路命中）；
//! 3. 恰一路 modified 的内容级证据：每路 writer 种子卡内容互不相同，终卡内容
//!    恰等于其中**一路**的种子事实，且所有返回值与库内终卡一致（输者吃赢家结果）；
//! 4. 唯一索引下恒一行；
//! 5. 已有信号后不再重写：后续再带新种子调用，版本与卡内容原样保留（不覆盖）。

mod common;

use futures::future::join_all;
use mongodb::bson::{doc, DateTime, Document};
use wechatagent::agent::load_or_create_operating_memory;
use wechatagent::models::{AgentStatus, Contact};

use crate::common::TestApp;

const WXID: &str = "wx_occ_contact";

fn contact(ws: &str, acc: &str, profile_note: Option<&str>) -> Contact {
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: WXID.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: profile_note.map(|s| s.to_string()),
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

/// CONC-1：并发种子卡升级——零 Err、版本恰一次 bump、单一 writer 内容胜出、
/// 输者吃赢家结果、恒一行、已有信号后不再覆盖。
#[tokio::test]
#[ignore]
async fn concurrent_memory_card_write_does_not_lose_race_error() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();

    // 第 0 步：裸 contact（无任何画像信号）首触达——建行，卡无信号、version=0。
    let bare = load_or_create_operating_memory(&app.state, &contact(&ws, &acc, None))
        .await
        .expect("create bare operating memory");
    assert_eq!(bare.memory_card_version, 0, "无信号种子卡应落 version=0");

    // 第 1 步：4 路并发 OCC 升级，各带**互不相同**的运营备注（operator_manual 权威
    // 事实），谁赢一目了然。
    let notes = [
        "并发画像备注甲：客户经营烘焙工作室",
        "并发画像备注乙：客户是技术负责人",
        "并发画像备注丙：客户偏好晚间沟通",
        "并发画像备注丁：客户关注售后条款",
    ];
    let handles: Vec<_> = notes
        .iter()
        .map(|note| {
            let state = app.state.clone();
            let c = contact(&ws, &acc, Some(note));
            tokio::spawn(async move { load_or_create_operating_memory(&state, &c).await })
        })
        .collect();
    let results = join_all(handles).await;

    let mut returned = Vec::new();
    for (i, joined) in results.into_iter().enumerate() {
        let memory = joined
            .expect("writer task must not panic")
            .unwrap_or_else(|e| {
                panic!("CONC-1：并发 writer #{i} 不得返回 Err（输 OCC 必须静默让位）：{e:?}")
            });
        returned.push(memory);
    }

    // 版本单调 + 恰一次 bump：0 → 1。
    let final_row = app
        .state
        .db
        .operating_memories()
        .find_one(
            doc! { "workspace_id": &ws, "account_id": &acc, "contact_wxid": WXID },
            None,
        )
        .await
        .expect("read final memory")
        .expect("memory row exists");
    assert_eq!(
        final_row.memory_card_version, 1,
        "prev=0 的并发升级终版必须恰为 1（版本谓词保证至多一路命中）"
    );

    // 恰一路 modified 的内容级证据：终卡 core_facts 恰含四个备注之一，且只含一个。
    let final_card = mongodb::bson::to_document(&final_row.memory_card).expect("card to doc");
    let final_card_json = format!("{final_card:?}");
    let matched: Vec<&&str> = notes
        .iter()
        .filter(|note| final_card_json.contains(*note))
        .collect();
    assert_eq!(
        matched.len(),
        1,
        "终卡必须恰含单一 writer 的种子事实（无交错混写），实际命中 {matched:?}"
    );

    // 输者吃赢家结果：每路返回的 memory 卡与库内终卡一致、版本一致。
    for (i, memory) in returned.iter().enumerate() {
        assert_eq!(
            memory.memory_card_version, 1,
            "writer #{i} 返回的版本必须是赢家版本"
        );
        let card = mongodb::bson::to_document(&memory.memory_card).expect("card to doc");
        assert_eq!(
            card, final_card,
            "writer #{i} 返回的卡必须与库内终卡一致（输者重读赢家结果，不得本地覆盖）"
        );
    }

    // 唯一索引下恒一行。
    let rows = app
        .state
        .db
        .operating_memories()
        .count_documents(
            doc! { "workspace_id": &ws, "account_id": &acc, "contact_wxid": WXID },
            None,
        )
        .await
        .expect("count rows");
    assert_eq!(rows, 1, "并发升级不得裂出第二行");

    // 第 2 步：已有信号后再带新种子调用——版本与卡内容原样保留（不覆盖、不 bump）。
    let after = load_or_create_operating_memory(
        &app.state,
        &contact(&ws, &acc, Some("并发画像备注戊：迟到的新种子")),
    )
    .await
    .expect("post-upgrade load");
    assert_eq!(
        after.memory_card_version, 1,
        "已有信号的卡不再被种子升级路径重写（版本单调不回退也不虚涨）"
    );
    let after_card = mongodb::bson::to_document(&after.memory_card).expect("card to doc");
    assert_eq!(after_card, final_card, "迟到种子不得覆盖已有信号卡");

    app.cleanup().await;
}
