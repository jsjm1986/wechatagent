//! 去抖聚合调度器 **真 async runner** 端到端集成测试。
//!
//! 与 `tests/debounce_barge_in_run.rs`（直调下游 `handle_managed_message_aggregated`，
//! 绕过 runner）互补：本文件真的走 `register_inbound` + `run_debounce_pipeline`，
//! 覆盖调度器的 **去抖睡眠 → 快照 generation/最新入站 → reload → reaction →
//! 聚合网关 → 退休/重算** 完整循环。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB），与全仓集成测试一致。
//!
//! 命门（见 wave1-task4-brief）：
//! - `static PENDING` 跨测试共享（同一 binary 内并发跑）→ **每个测试用唯一 wxid**，
//!   contact_key 唯一，测试间零 generation/deadline 串扰。
//! - MockLlm 队列空返回 `Err`（非 panic）→ push 足量响应，用 `llm.calls()` 断言实际触达。
//! - 全新 contact 首轮 reaction claim 拿不到已 sent 的 decision_review → 跳过 reaction
//!   LLM（0 调用）；单轮聚合只消费 decision + review = **2 次** LLM。
//! - 真 async runner 的"网关执行中途抢占"本质是竞态（MockLlm 响应近乎瞬时，晚到
//!   入站只会落在去抖 sleep 期而非 gateway 期）→ 无法确定性复现。Step2/Step3 因此
//!   **降级为可确定性验证的子命题**（见各测试注释与报告），绝不写靠 sleep 凑时序的
//!   flaky 断言。gateway 层"guard 恒真→superseded、不入 outbox、last_run 不推进"的
//!   抢占语义由现存 `debounce_barge_in_run.rs` 既有覆盖。

mod common;

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};
use wechatagent::webhooks::{contact_key, register_inbound, run_debounce_pipeline};

// ── helpers（照抄 tests/debounce_barge_in_run.rs，仅 wxid 参数化）──

fn make_managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("测试客户".to_string()),
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

fn make_inbound(contact: &Contact, message_id: &str, content: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: Some(message_id.to_string()),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: content.to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    }
}

fn reply_agent_decision_json(reply_text: &str, why_should_reply: &str) -> serde_json::Value {
    json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户表达明确，正在评估我方在企业 IM 场景下的方案适配度，并给出落地预算与时间。",
        "relationshipRead": "对话氛围积极，对我方专业度信任，但对实施周期与成本有一定顾虑，关系处于稳步推进期。",
        "operationGoal": "聚焦在帮客户厘清下一步排期与成本边界，让客户在不被推销压力下感到掌控感与确定性。",
        "knowledgeNeedReason": "客户提及了具体场景与预算需求，需要结合产品能力切片确认我方覆盖范围与交付承诺边界。",
        "memoryUpdateReason": "本轮新增客户预算与时间锚点信息，需要写入长期记忆以支持后续节奏与产品方案匹配。",
        "selfCritique": "上一轮我方过早提到价格档位，本次需收敛信息密度并先确认客户优先级再给出下一步建议。",
        "whyShouldReply": why_should_reply,
        "whySkipReply": "",
        "riskSelfCheck": "本轮回复不涉及未验证的产品能力承诺，仅给出节奏与下一步动作建议，不触发安全门阈值。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "need_discovery",
        "shouldReply": true,
        "replyText": reply_text,
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户进入方案/能力评估阶段，按顾问模式明确处理产品与排期问题。",
    })
}

fn review_agent_pass_json(review_summary: &str) -> serde_json::Value {
    json!({
        "approved": true,
        "scores": {
            "humanLike": 8,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "relationshipProgress": 7,
            "conversionReadiness": 6,
            "pressureRisk": 2,
            "factRisk": 1,
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "候选回复仅承接节奏，不涉及具体产品能力承诺。",
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": review_summary,
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true,
    })
}

/// 入队 `rounds` 轮聚合网关所需响应（每轮 decision + review 各一条，FIFO 顺序）。
/// 多入队几轮不会误消费——MockLlm 队列空才报错，用不完的响应留在队里无害。
fn push_gateway_rounds(app: &common::TestApp, rounds: usize) {
    for _ in 0..rounds {
        app.llm.push_response(reply_agent_decision_json(
            "我们一般 2~4 周可上线，预算和场景深度相关，要不要先按你们的优先级排排序？",
            "客户主动询问实施周期与预算，回复能确认需求颗粒度并降低决策摩擦，是关键推进时机。",
        ));
        app.llm.push_response(review_agent_pass_json(
            "回复语气良好、不越界承诺，可放行。",
        ));
    }
}

/// 轮询等待 outbox 出现至少 `expected` 行（真 runner 异步落库），超时 panic。
async fn wait_for_outbox_count(
    app: &common::TestApp,
    wxid: &str,
    expected: u64,
    timeout: Duration,
) -> u64 {
    let start = std::time::Instant::now();
    let mut last = 0u64;
    while start.elapsed() < timeout {
        last = app
            .state
            .db
            .collection_agent_send_outbox()
            .count_documents(doc! { "contact_wxid": wxid }, None)
            .await
            .expect("count outbox by contact_wxid");
        if last >= expected {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "wait_for_outbox_count({wxid}) timed out after {:?}: expected >= {}, last = {}",
        timeout, expected, last
    );
}

async fn count_run_logs(app: &common::TestApp, contact: &Contact) -> u64 {
    app.state
        .db
        .agent_run_logs()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("count agent_run_logs")
}

// ── Step1：连发 3 条 → 去抖聚合只产 1 次 gateway 决策 ──

/// 连发 3 条入站，全部落在同一去抖窗口内 → runner 只跑一次聚合网关：
/// - agent_run_logs 该 contact **恰好 1 行**（3 条聚合成 1 次运行，非 3 次）；
/// - outbox 入队 **恰好 1 行**（只发一次，不重复回复）；
/// - `llm.calls() == 2`（单轮 decision + review；全新 contact reaction claim 空跳过）。
#[tokio::test]
#[ignore]
async fn three_rapid_inbounds_aggregate_into_single_gateway_run() {
    let app = common::TestApp::start().await;
    let wxid = "debounce_agg_user_1";
    let contact = make_managed_contact(wxid);
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    // 3 条入站写库（模拟连发；msg_id 各异）。
    for (i, content) in [
        "你们的实施周期一般多久？",
        "大概预算需要多少？",
        "对了，能先给个大致排期吗？",
    ]
    .iter()
    .enumerate()
    {
        let inbound = make_inbound(&contact, &format!("msg_agg_{i}"), content);
        app.state
            .db
            .messages()
            .insert_one(&inbound, None)
            .await
            .expect("insert inbound message");
    }

    // 单轮聚合只需 2 条；push 2 轮量做冗余护栏（用不完无害）。
    push_gateway_rounds(&app, 2);

    let key = contact_key(&contact.workspace_id, &contact.account_id, wxid);

    // 3 次 register_inbound 在同一 async fn 内同步连续完成（无 .await 间隔）→ 都落在
    // 50ms 窗口内。先全部注册再 spawn，runner 起跑必然快照到 generation=3、
    // latest_inbound=第 3 条，确保聚合成一轮（比"边注册边 spawn"更确定性，runner 逻辑不变）。
    let mut spawn_handle = None;
    for i in 0..3usize {
        let inbound = make_inbound(&contact, &format!("msg_agg_{i}"), "");
        let (st, spawned_now) = register_inbound(key.clone(), inbound, 50);
        if spawned_now {
            assert_eq!(i, 0, "只有首条 register 应 spawn，实际在 i={i}");
            let state = app.state.clone();
            let k = key.clone();
            let account = contact.account_id.clone();
            let from = wxid.to_string();
            spawn_handle = Some(tokio::spawn(async move {
                run_debounce_pipeline(state, k, st, account, from, None).await;
            }));
        } else {
            assert_ne!(i, 0, "首条之外的 register 必须只 bump（spawned_now=false）");
        }
    }
    let handle = spawn_handle.expect("首条 register 应 spawn runner");

    // 等 runner 完成：outbox 出现一行是可观测终态信号（比单次 sleep 猜时长更稳）。
    let outbox_count = wait_for_outbox_count(&app, wxid, 1, Duration::from_secs(30)).await;
    handle.await.expect("runner task joined");

    assert_eq!(
        outbox_count, 1,
        "3 条聚合应只入队 1 行 outbox（不重复回复），实际 {outbox_count} 行"
    );
    let run_logs = count_run_logs(&app, &contact).await;
    assert_eq!(
        run_logs, 1,
        "3 条入站应聚合成 1 次 gateway 运行（agent_run_logs 恰好 1 行），实际 {run_logs} 行"
    );
    assert_eq!(
        app.llm.calls(),
        2,
        "单轮聚合只应触达 decision + review 两次 LLM（reaction claim 空跳过），实际 {} 次",
        app.llm.calls()
    );
}

// ── Step2：runner 用聚合后（最新入站）的上下文 ──

/// **确定性子命题**（非 flaky 的"网关执行中途抢占"——那本质是竞态，MockLlm 响应
/// 瞬时使晚到入站只落在去抖 sleep 期而非 gateway 期，无法确定性复现；gateway 层
/// 抢占语义由 `debounce_barge_in_run.rs` 覆盖）：
///
/// 验证 runner 在去抖窗口内被后到入站刷新后，**快照并使用最新入站**做决策——
/// 即 decision_review 的 inbound_message_id 对应最后一条（最新）入站，而非首条。
/// 这是"抢占重算用聚合后上下文"里可确定性检验的核心不变量。
#[tokio::test]
#[ignore]
async fn runner_uses_latest_inbound_snapshot_for_decision() {
    let app = common::TestApp::start().await;
    let wxid = "debounce_latest_user_2";
    let contact = make_managed_contact(wxid);
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    let first = make_inbound(&contact, "msg_latest_first", "先问个笼统的。");
    let latest = make_inbound(&contact, "msg_latest_final", "算了，直接给我最新报价。");
    for m in [&first, &latest] {
        app.state
            .db
            .messages()
            .insert_one(m, None)
            .await
            .expect("insert inbound message");
    }

    push_gateway_rounds(&app, 2);

    let key = contact_key(&contact.workspace_id, &contact.account_id, wxid);

    // 首条 register → spawn；第二条在同窗口内 bump（刷新 latest_inbound=latest）。
    let (st, spawned_now) = register_inbound(key.clone(), first, 50);
    assert!(spawned_now, "首条 register 应 spawn runner");
    let state = app.state.clone();
    let k = key.clone();
    let account = contact.account_id.clone();
    let from = wxid.to_string();
    let handle = tokio::spawn(async move {
        run_debounce_pipeline(state, k, st, account, from, None).await;
    });

    let (_st2, spawned_again) = register_inbound(key.clone(), latest.clone(), 50);
    assert!(
        !spawned_again,
        "runner 存活期间的第二条 register 必须只 bump（不重复 spawn）"
    );

    wait_for_outbox_count(&app, wxid, 1, Duration::from_secs(30)).await;
    handle.await.expect("runner task joined");

    // decision_review 的 inbound_message_id = 最新入站（快照取 latest_inbound）。
    let review = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "contact_wxid": wxid }, None)
        .await
        .expect("query decision_reviews")
        .expect("decision_reviews row exists");
    assert_eq!(
        review.inbound_message_id.as_deref(),
        Some("msg_latest_final"),
        "runner 应快照最新入站做决策，inbound_message_id 实际 {:?}",
        review.inbound_message_id
    );
    // 聚合仍只跑一轮（无中途抢占）→ 只 2 次 LLM、outbox 1 行。
    assert_eq!(app.llm.calls(), 2, "单轮聚合只应 2 次 LLM，实际 {}", app.llm.calls());
    let run_logs = count_run_logs(&app, &contact).await;
    assert_eq!(run_logs, 1, "聚合应只产 1 次运行，实际 {run_logs} 行");
}

// ── Step3：不丢消息 — runner 存活期间晚到入站不重复 spawn 且被纳入处理 ──

/// **确定性子命题**（非 flaky 的"退休瞬间晚到"——原子退休窗极窄、竞态不可确定性
/// 复现）：验证 runner 存活期间再 register_inbound
/// - 返回 `spawned_now=false`（**不重复 spawn 第二个 runner**，即不丢/不并发处理），
/// - 且 bump 了 generation（晚到入站不会被静默丢弃）,
/// - 最终 outbox 仍恰好入队 1 行（晚到入站被同一 runner 聚合处理，无重复回复）。
#[tokio::test]
#[ignore]
async fn late_inbound_bumps_generation_without_duplicate_spawn() {
    let app = common::TestApp::start().await;
    let wxid = "debounce_late_user_3";
    let contact = make_managed_contact(wxid);
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    let first = make_inbound(&contact, "msg_late_first", "第一条。");
    let late = make_inbound(&contact, "msg_late_second", "补一条晚到的。");
    for m in [&first, &late] {
        app.state
            .db
            .messages()
            .insert_one(m, None)
            .await
            .expect("insert inbound message");
    }

    push_gateway_rounds(&app, 2);

    let key = contact_key(&contact.workspace_id, &contact.account_id, wxid);

    let (st, spawned_now) = register_inbound(key.clone(), first, 50);
    assert!(spawned_now, "首条 register 应 spawn runner");
    let gen_after_first = st.generation.load(std::sync::atomic::Ordering::Acquire);
    assert_eq!(gen_after_first, 1, "首条 register 后 generation 应为 1");

    let state = app.state.clone();
    let k = key.clone();
    let account = contact.account_id.clone();
    let from = wxid.to_string();
    let handle = tokio::spawn(async move {
        run_debounce_pipeline(state, k, st, account, from, None).await;
    });

    // 晚到入站：不重复 spawn，但 bump generation（保证不丢）。
    let (st2, spawned_again) = register_inbound(key.clone(), late, 50);
    assert!(
        !spawned_again,
        "runner 存活期间晚到入站必须只 bump，不得重复 spawn 第二个 runner"
    );
    assert_eq!(
        st2.generation.load(std::sync::atomic::Ordering::Acquire),
        2,
        "晚到入站应把 generation bump 到 2（晚到消息不被丢弃）"
    );

    // 晚到入站被同一 runner 聚合处理：最终 outbox 恰好 1 行（无重复回复）。
    let outbox_count = wait_for_outbox_count(&app, wxid, 1, Duration::from_secs(30)).await;
    handle.await.expect("runner task joined");
    assert_eq!(
        outbox_count, 1,
        "晚到入站应被同一 runner 聚合，最终 outbox 恰好 1 行，实际 {outbox_count} 行"
    );
    let run_logs = count_run_logs(&app, &contact).await;
    assert_eq!(
        run_logs, 1,
        "首条 + 晚到入站落在同窗口应聚合成 1 次运行，实际 {run_logs} 行"
    );
}
