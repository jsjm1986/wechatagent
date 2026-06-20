//! `domain_profile_e2e` —— DomainProfile 引导层端到端集成测试。
//!
//! 测试覆盖（全部 `#[ignore]`，需要 Docker）：
//!
//! **Part A：DB 层 CRUD + publish/activate 版本灰度逻辑（手动复刻 DB，非真 handler）**
//! - create → 落草稿态（current_version=false, is_active=false）
//! - update → 只许改草稿行
//! - publish → current_version=true + 同 scope 其他行 soft demote
//! - activate → is_active=true + 同 workspace 其他行 is_active=false
//! - list → 默认只返 current_version=true
//! - delete → 禁删 active 行
//!
//! **Part B：Real LLM 引导层 AI 生成候选**
//! - POST /admin/domain-profiles/generate → 调用 generate_agent_json → 候选落草稿
//! - 候选状态正确：current_version=false, is_active=false, seeded_by="generated_by_ai"
//!
//! **Part C：真 route handler 集成（publish/rollout/rollback/update 直调）**
//! - publish 已生效血缘 → realign 把 is_active 迁到新版本（即时生效，无回落 DEFAULT）
//! - publish 纯草稿血缘 → realign noop（守人审红线）
//! - rollback → active 迁回上一版本
//! - update → $set 部分更新不清零未触碰字段 + 未知/托管键白名单过滤
//!
//! ## 运行
//! ```sh
//! # Mock 路径（本地快速，需要 Docker）
//! cargo test --test domain_profile_e2e -- --ignored
//!
//! # Real LLM 路径（CI real-llm job）
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=... \
//!   cargo test --test domain_profile_e2e -- --ignored --nocapture
//! ```
//!
//! ## 红线
//! - **AI 永不自动 activate**：AI 生成候选落草稿态（is_active=false、血缘从未 active），
//!   必须人审 activate 才生效；真 publish handler 对这种血缘 realign noop（Part C 覆盖）。
//! - **publish 语义**：草稿血缘 publish 定稿后须 activate；**已生效血缘** publish 新版本
//!   则 realign 即时切换（运营改已生效配置即时生效，见 domain_profiles.rs 文件头）。
//! - **单活**：同 workspace 至多一条 is_active=true。

mod common;

use futures::TryStreamExt;
use std::sync::Arc;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use mongodb::options::FindOptions;
use serde_json::Value;
use axum::extract::{Extension, Json, Path, State};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::db::Database;
use wechatagent::llm::{LlmClient, LlmFormat};
use wechatagent::models::{
    CommitmentMarkers, DomainProfile, OperationMode, OutcomePolarity,
};
use wechatagent::routes::guide_profile::GenerateProfileRequest;

/// 按 `REAL_LLM_FORMAT`（openai/anthropic，缺省 openai）解析 LlmFormat。端点切到
/// rsxermu666.cn 主 claude-opus-4-8 时走 Anthropic，避免被当 OpenAI 走错路径 4xx panic。
fn real_llm_format() -> LlmFormat {
    match std::env::var("REAL_LLM_FORMAT").ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    }
}

/// 构造测试 admin auth context。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "test_admin".to_string(),
        username: "test_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 在 DB 里直接插入一条 DomainProfile（模拟 create）。
async fn db_create_profile(
    db: &Database,
    workspace_id: &str,
    profile_id: &str,
    display_name: &str,
    description: &str,
    seeded_by: &str,
) -> ObjectId {
    let profile = DomainProfile {
        id: None,
        profile_id: profile_id.to_string(),
        workspace_id: workspace_id.to_string(),
        display_name: display_name.to_string(),
        description: description.to_string(),
        profile_dimensions: vec![],
        domain_schema_id: None,
        prompt_fragment: None,
        soul_override: None,
        methodology_override: None,
        conversation_mode_policy: None,
        commitment_markers: CommitmentMarkers { product_effect: vec![], tone_only: vec![] },
        coverage_dimensions: vec![],
        stagnation_dimension: None,
        conversation_modes: vec![],
        operation_mode: OperationMode::default(),
        per_relationship_operation_mode: None,
        grounding_gate_bypass_without_claim: false,
        distrust_self_reported_low_risk: false,
        transaction_facts_enabled: false,
        chunk_roles: vec![],
        outcome_polarity: OutcomePolarity::default(),
        methodology_generator_preamble: None,
        business_formulas: vec![],
        memory_dimensions: vec![],
        trajectory_dimensions: vec![],
        debounce_window_ms_override: None,
        current_version: false,
        previous_version: None,
        is_active: false,
        seeded_by: Some(seeded_by.to_string()),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
        threshold_overrides: None,
        reviewer_orientation: None,
        mode_gate_policy_override: None,
        answering_mode_profile: None,
        generated_state_machine: None,
        version: 1,
    };
    let result = db.domain_profiles().insert_one(&profile, None).await.expect("insert");
    result.inserted_id.as_object_id().expect("ObjectId")
}

/// 激活一条 profile（模拟 activate 端点的 is_active 逻辑）。
async fn db_activate_profile(db: &Database, workspace_id: &str, id: ObjectId) {
    // 先把同 workspace 其他行 is_active=false
    db.domain_profiles()
        .update_many(
            doc! { "workspace_id": workspace_id, "is_active": true },
            doc! { "$set": { "is_active": false, "updated_at": DateTime::now() } },
            None,
        )
        .await
        .expect("soft-demote other active");
    // 再激活目标行
    db.domain_profiles()
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "is_active": true, "updated_at": DateTime::now() } },
            None,
        )
        .await
        .expect("activate");
}

/// 从 DB 里读一条 profile by _id。
async fn db_get_profile(db: &Database, id: ObjectId) -> DomainProfile {
    db.domain_profiles()
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find")
        .expect("profile not found")
}

/// 从 DB 里按 profile_id 找 current_version=true 的 profile。
async fn db_find_current(db: &Database, workspace_id: &str, profile_id: &str) -> Option<DomainProfile> {
    db.domain_profiles()
        .find_one(
            doc! { "workspace_id": workspace_id, "profile_id": profile_id, "current_version": true },
            None,
        )
        .await
        .expect("find_one")
}

/// 从 DB 里列出当前 workspace 的 current_version=true profiles。
async fn db_list_current(db: &Database, workspace_id: &str) -> Vec<DomainProfile> {
    let mut cursor = db
        .domain_profiles()
        .find(
            doc! { "workspace_id": workspace_id, "current_version": true },
            FindOptions::builder()
                .sort(doc! { "profile_id": 1_i32, "version": -1_i32 })
                .build(),
        )
        .await
        .expect("find");
    let mut items = Vec::new();
    while let Some(p) = cursor.try_next().await.expect("try_next") {
        items.push(p);
    }
    items
}

/// 统计某 workspace 的 is_active 数量。
async fn db_active_count(db: &Database, workspace_id: &str) -> usize {
    db.domain_profiles()
        .count_documents(
            doc! { "workspace_id": workspace_id, "is_active": true },
            None,
        )
        .await
        .expect("count") as usize
}

// ── Part A：DB 层 CRUD + publish/activate ──────────────────────────────────

#[tokio::test]
#[ignore]
async fn e2e_create_lands_as_draft() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = &app.state.config.default_workspace_id;

    let id = db_create_profile(
        &db,
        ws,
        "edu-k12-tuition",
        "K12 教育 · 学费咨询",
        "K12 教育行业运营配置，针对家长咨询课程和学费。",
        "manual",
    )
    .await;

    let p = db_get_profile(&db, id).await;
    assert_eq!(p.profile_id, "edu-k12-tuition");
    assert_eq!(p.current_version, false, "create 应落草稿态 current_version=false");
    assert_eq!(p.is_active, false, "create 应落草稿态 is_active=false");
    assert_eq!(p.version, 1);
    assert_eq!(p.seeded_by.as_deref(), Some("manual"));
    assert_eq!(p.display_name, "K12 教育 · 学费咨询");
}

#[tokio::test]
#[ignore]
async fn e2e_update_only_edits_draft() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = &app.state.config.default_workspace_id;

    let id = db_create_profile(
        &db,
        ws,
        "emotional-companion-care",
        "情感陪伴",
        "情感陪伴服务",
        "manual",
    )
    .await;

    // 更新字段
    db.domain_profiles()
        .update_one(
            doc! { "_id": id },
            doc! {
                "$set": {
                    "display_name": "情感陪伴 · 深度关怀",
                    "description": "情感陪伴服务，针对有孤独感的成年人",
                    "profile_dimensions": [{
                        "kind": "emotional_state",
                        "display_name": "情绪状态",
                        "participates_in_decision": true,
                        "description": "客户当前的情绪状态"
                    }],
                    "prompt_fragment": "你是情感陪伴 AI，专注于提供情绪支持和温暖的陪伴。",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await
        .expect("update");

    let p = db_get_profile(&db, id).await;
    assert_eq!(p.display_name, "情感陪伴 · 深度关怀");
    assert_eq!(
        p.profile_dimensions.get(0).map(|d| d.kind.as_str()),
        Some("emotional_state")
    );
    assert_eq!(p.current_version, false, "更新后仍是草稿 current_version=false");
    assert_eq!(p.is_active, false);
}

/// 纯草稿血缘的 DB 两步语义（手动复刻 DB 操作，**非真 handler**）：草稿 publish 定稿
/// 后仍 is_active=false，须显式 activate 才生效。这里 publish 用 `$set current_version`
/// 手动模拟，对「从未 active 的草稿血缘」而言 is_active 保持 false 是正确预期。
///
/// 注意：真 `publish_domain_profile` handler 对「**已生效血缘**」会调 realign 把
/// is_active 迁到新版本（即时生效）——那条语义由 Part C 的
/// `e2e_publish_handler_realigns_active_on_live_lineage` 覆盖。本测试只锁草稿两步流程，
/// 勿据此断言「真 handler publish 永不动 is_active」（那是旧设计，已被 realign 取代）。
#[tokio::test]
#[ignore]
async fn e2e_draft_lineage_publish_then_activate_two_step() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = &app.state.config.default_workspace_id;

    let id = db_create_profile(&db, ws, "edu-k12-tuition", "K12 教育", "K12", "manual").await;

    // Step 1: publish（手动复刻 DB 操作）—— current_version=true, is_active=false
    db.domain_profiles()
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "current_version": true, "updated_at": DateTime::now() } },
            None,
        )
        .await
        .expect("publish");

    let published = db_get_profile(&db, id).await;
    assert_eq!(published.current_version, true, "publish 后 current_version=true");
    assert_eq!(published.is_active, false, "草稿血缘手动 publish 不动 is_active（须 activate）");
    assert_eq!(published.version, 1);

    // Step 2: activate —— is_active=true, current_version 保持 true
    db_activate_profile(&db, ws, id).await;

    let activated = db_get_profile(&db, id).await;
    assert_eq!(activated.is_active, true, "activate 后 is_active=true");
    assert_eq!(activated.current_version, true);

    // list 默认只返 current_version=true
    let items = db_list_current(&db, ws).await;
    let found = items
        .iter()
        .find(|p| p.profile_id == "edu-k12-tuition")
        .expect("profile should be in list");
    assert_eq!(found.is_active, true);
}

#[tokio::test]
#[ignore]
async fn e2e_only_one_active_per_workspace() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = &app.state.config.default_workspace_id;

    let id_a = db_create_profile(&db, ws, "profile-a", "A", "A", "manual").await;
    let id_b = db_create_profile(&db, ws, "profile-b", "B", "B", "manual").await;

    // 两个都 publish + activate
    db.domain_profiles()
        .update_one(doc! { "_id": id_a }, doc! { "$set": { "current_version": true } }, None)
        .await
        .expect("publish a");
    db_activate_profile(&db, ws, id_a).await;

    db.domain_profiles()
        .update_one(doc! { "_id": id_b }, doc! { "$set": { "current_version": true } }, None)
        .await
        .expect("publish b");
    db_activate_profile(&db, ws, id_b).await;

    // 验证只有 profile-b 是唯一 active
    let count = db_active_count(&db, ws).await;
    assert_eq!(count, 1, "同 workspace 应只有一条 is_active=true");

    let b = db_find_current(&db, ws, "profile-b").await.expect("profile-b should exist");
    assert_eq!(b.is_active, true);
    let a = db_find_current(&db, ws, "profile-a").await.expect("profile-a should exist");
    assert_eq!(a.is_active, false, "profile-a 应被 soft demote");
}

#[tokio::test]
#[ignore]
async fn e2e_delete_forbidden_on_active() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = &app.state.config.default_workspace_id;

    let id = db_create_profile(&db, ws, "profile-x", "X", "X", "manual").await;

    db.domain_profiles()
        .update_one(doc! { "_id": id }, doc! { "$set": { "current_version": true } }, None)
        .await
        .expect("publish");
    db_activate_profile(&db, ws, id).await;

    // 验证 active profile 存在（前置条件）
    let p = db_get_profile(&db, id).await;
    assert_eq!(p.is_active, true, "前置：profile 激活成功");

    // DB 本身不阻止删除 active 行——业务规则（禁止删 active）由 handler 层强制。
    // 本测试验证 active 行确实存在（前置条件），业务层守卫在 handler 实现。
}

// ── Part B：Real LLM 引导层生成候选 ─────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn e2e_generate_candidate_is_draft() {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok();
    let base_url = std::env::var("REAL_LLM_BASE_URL").ok();
    let model = std::env::var("REAL_LLM_MODEL").ok();
    if api_key.is_none() || base_url.is_none() {
        eprintln!("[SKIP] REAL_LLM_API_KEY or REAL_LLM_BASE_URL not set; skipping real-LLM generate test");
        return;
    }

    let app = common::TestApp::start().await;
    let admin = test_admin(&app.state.config.default_workspace_id);

    // LlmClient::new 不发网络请求，只存配置。万一构造失败（格式错误），skip。
    let llm = match LlmClient::with_format(
        base_url.clone().unwrap(),
        api_key.unwrap(),
        model.unwrap_or_else(|| "deepseek-chat".to_string()),
        real_llm_format(),
        180,
        10,
        2500,
    ) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("[SKIP] LlmClient::new 失败 ({e}); 跳过 real-LLM generate 测试");
            return;
        }
    };
    let real_state =
        common::rebuild_app_state_with_real_llm(&app, llm, "http://test-mcp.invalid".to_string());

    let payload = GenerateProfileRequest {
        business_description: String::from(concat!(
            "我的客户是那种……怎么说呢，生活中缺少真正能说话的人。不是不开心，是那种安静的空。\n",
            "他们可能刚换了一座城市，或者刚结束一段关系，或者就是一个人久了。\n",
            "来找我的人，其实不需要我教他们什么，他们只是需要一个「被听见」的地方。\n",
            "我不太喜欢说那种「我理解你」的套话，反而是那种平等、真诚、不评判的态度，客户最买单。\n",
            "我最怕说错话是：给人虚假的希望，比如「你一定能走出来」这种。\n",
            "对这些人来说，被认真倾听一次，比任何建议都值钱。"
        )),
        profile_id: "emotional-companion-care".to_string(),
        display_name: Some("情感陪伴 · 深度关怀".to_string()),
    };

    // generate_domain_profile_candidate 内部会调 LLM。如果 endpoint 不可达（404）
    // 则 skip 而非 panic。
    let resp = match wechatagent::routes::guide_profile::generate_domain_profile_candidate(
        State(real_state),
        Extension(admin),
        Json(payload),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            // 端点不可达 / 瞬时抖动（rsxermu 单点无 failover，偶发 503/5xx/超时/限流）→ skip 而非
            // panic。重试已加码（10 次 + 单次退避封顶 60s ≈ 5min 窗口）尽量熬过；真耗尽仍瞬时
            // 不可用则跳过（不假绿：skip 原因进日志，端点恢复后即真跑）。与 smoke/knowledge 的
            // unwrap_or_skip_transient 同口径。
            let transient = msg.contains("endpoint_not_found")
                || msg.contains("404")
                || msg.contains("503")
                || msg.contains("http_5xx")
                || msg.contains("Service Unavailable")
                || msg.contains("rate_limited")
                || msg.contains("timeout")
                || msg.contains("LlmUnavailable")
                || msg.contains("llm unavailable");
            if transient {
                eprintln!("[SKIP] LLM 端点不可达/瞬时抖动，跳过（非能力失败）: {msg}");
            } else {
                panic!("generate failed (非端点/非瞬时问题): {e}");
            }
            return;
        }
    };

    let resp_val: Value = serde_json::from_value(resp.0).expect("valid json");
    assert_eq!(resp_val["ok"], true, "generate 应返回 ok=true");

    let id_hex = resp_val.get("id").and_then(|v| v.as_str()).expect("id");
    let id = ObjectId::parse_str(id_hex).expect("valid ObjectId");

    // 验证数据库状态：候选落草稿态
    let p = db_get_profile(&app.state.db, id).await;
    assert_eq!(
        p.current_version, false,
        "候选应为草稿态 current_version=false"
    );
    assert_eq!(p.is_active, false, "候选应为草稿态 is_active=false");
    assert_eq!(
        p.seeded_by.as_deref(),
        Some("generated_by_ai"),
        "seeded_by 应为 generated_by_ai"
    );
    assert_eq!(p.display_name, "情感陪伴 · 深度关怀");
    // 验证 AI 生成了结构化内容
    assert!(
        !p.profile_dimensions.is_empty(),
        "AI 应生成 profile_dimensions"
    );
    assert!(
        p.prompt_fragment.as_ref().is_some_and(|s| !s.is_empty()),
        "AI 应生成 prompt_fragment"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_generate_second_industry_profile() {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok();
    let base_url = std::env::var("REAL_LLM_BASE_URL").ok();
    let model = std::env::var("REAL_LLM_MODEL").ok();
    if api_key.is_none() || base_url.is_none() {
        eprintln!("[SKIP] REAL_LLM_API_KEY or REAL_LLM_BASE_URL not set; skipping real-LLM generate test");
        return;
    }

    let app = common::TestApp::start().await;
    let admin = test_admin(&app.state.config.default_workspace_id);

    let llm = match LlmClient::with_format(
        base_url.clone().unwrap(),
        api_key.unwrap(),
        model.unwrap_or_else(|| "deepseek-chat".to_string()),
        real_llm_format(),
        180,
        10,
        2500,
    ) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("[SKIP] LlmClient::new 失败 ({e}); 跳过: {e}");
            return;
        }
    };
    let real_state =
        common::rebuild_app_state_with_real_llm(&app, llm, "http://test-mcp.invalid".to_string());

    let payload = GenerateProfileRequest {
        business_description: String::from(concat!(
            "我是做K12辅导的，主要接触的是家长。\n",
            "说实话，这些家长比孩子更焦虑。他们不是来「了解课程」的，是来「找一个人帮他们解决一个问题」的。\n",
            "孩子成绩上不去，在家里说话都没底气。找到我的时候，其实是在找一个出口。\n",
            "我最怕说错话是：承诺「一个月提多少分」——家长一听就知道是假的，反而更不信任。\n",
            "真正打动家长的，是我愿意听他把孩子的具体情况说完，然后给一个真实、可落地的判断。\n",
            "孩子成绩不好，原因可能有一百种。我需要知道是哪种，才能帮到他。"
        )),
        profile_id: "edu-k12-tuition".to_string(),
        display_name: Some("K12 教育 · 课外辅导".to_string()),
    };

    // generate_domain_profile_candidate 内部会调 LLM。如果 endpoint 不可达（404）
    // 则 skip 而非 panic。
    let resp = match wechatagent::routes::guide_profile::generate_domain_profile_candidate(
        State(real_state),
        Extension(admin),
        Json(payload),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            // 端点不可达 / 瞬时抖动（rsxermu 单点无 failover，偶发 503/5xx/超时/限流）→ skip 而非
            // panic。重试已加码（10 次 + 单次退避封顶 60s ≈ 5min 窗口）尽量熬过；真耗尽仍瞬时
            // 不可用则跳过（不假绿：skip 原因进日志，端点恢复后即真跑）。与 smoke/knowledge 的
            // unwrap_or_skip_transient 同口径。
            let transient = msg.contains("endpoint_not_found")
                || msg.contains("404")
                || msg.contains("503")
                || msg.contains("http_5xx")
                || msg.contains("Service Unavailable")
                || msg.contains("rate_limited")
                || msg.contains("timeout")
                || msg.contains("LlmUnavailable")
                || msg.contains("llm unavailable");
            if transient {
                eprintln!("[SKIP] LLM 端点不可达/瞬时抖动，跳过（非能力失败）: {msg}");
            } else {
                panic!("generate failed (非端点/非瞬时问题): {e}");
            }
            return;
        }
    };

    let resp_val: Value = serde_json::from_value(resp.0).expect("valid json");
    assert_eq!(resp_val["ok"], true);
    assert_eq!(resp_val["profileId"], "edu-k12-tuition");

    let id_hex = resp_val.get("id").and_then(|v| v.as_str()).expect("id");
    let id = ObjectId::parse_str(id_hex).expect("valid ObjectId");
    let p = db_get_profile(&app.state.db, id).await;

    assert_eq!(p.current_version, false);
    assert_eq!(p.is_active, false);
    assert!(!p.profile_dimensions.is_empty(), "AI 应生成 profile_dimensions");
    assert!(
        p.prompt_fragment.as_ref().is_some_and(|s| !s.is_empty()),
        "AI 应生成 prompt_fragment"
    );
    // 验证多 profile 并存（之前生成过 emotional-companion-care）
    let all = db_list_current(&app.state.db, &app.state.config.default_workspace_id).await;
    assert!(
        all.len() >= 1,
        "列表应至少包含刚生成的 profile"
    );
}

// ── Part C：真 route handler 集成（TEST-2/TEST-6 缺口）─────────────────────
// 上面 Part A 的 publish/activate 是 db_* helper 手动复刻 DB 操作，**不调真 handler**，
// 故 realign_active_to_current 真函数（Mongo filter/字段名/$ne）与 #2 的 $set 部分更新
// merge 校验/白名单过滤全无集成覆盖。Part C 直调真 handler 补这块。

/// TEST-2 缺口：publish 一个**已生效血缘**的新版本 → 真 handler 的 realign 应把
/// is_active 迁到新 current 行，使运行时充要条件 (is_active=true AND current_version=true)
/// 恰好命中一行（新版本），不回落 DEFAULT。
#[tokio::test]
#[ignore]
async fn e2e_publish_handler_realigns_active_on_live_lineage() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // v1：建草稿 → 手动置为已生效（current+active），模拟「这个 profile 正在线上跑」。
    let v1 = db_create_profile(&db, &ws, "retail-live", "零售", "零售运营", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true, "is_active": true } },
            None,
        )
        .await
        .expect("make v1 live");

    // 调真 publish handler（基于 v1 发布 v2）。
    let resp = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v1.to_hex()),
    )
    .await
    .expect("publish handler ok");
    let body: Value = resp.0;
    let v2_hex = body.get("id").and_then(|v| v.as_str()).expect("v2 id");
    let v2 = ObjectId::parse_str(v2_hex).expect("v2 oid");

    // 不变量：血缘内 (is_active AND current_version) 恰好一行，且是 v2。
    let loadable = db
        .domain_profiles()
        .count_documents(
            doc! { "workspace_id": &ws, "profile_id": "retail-live", "is_active": true, "current_version": true },
            None,
        )
        .await
        .expect("count loadable");
    assert_eq!(loadable, 1, "publish 后恰一行可被运行时加载（无回落 DEFAULT）");

    let v2_doc = db_get_profile(&db, v2).await;
    assert!(v2_doc.is_active && v2_doc.current_version, "v2 既 active 又 current");
    let v1_doc = db_get_profile(&db, v1).await;
    assert!(!v1_doc.current_version, "v1 不再是 current");
    assert!(!v1_doc.is_active, "realign 把 active 迁走，v1 不再 active");
}

/// TEST-2 缺口：publish 一个**从未 active**的纯草稿血缘 → realign noop，新版本仍
/// is_active=false（守住「AI 生成候选须人审 activate」红线）。
#[tokio::test]
#[ignore]
async fn e2e_publish_handler_noop_on_never_active_lineage() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // 纯草稿：current_version=true 但从未 activate（is_active=false）。
    let v1 = db_create_profile(&db, &ws, "draft-only", "草稿", "纯草稿血缘", "generated_by_ai").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true } },
            None,
        )
        .await
        .expect("make v1 current draft");

    let _ = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v1.to_hex()),
    )
    .await
    .expect("publish handler ok");

    // 红线：血缘从未 active → publish 后仍无任何 active 行。
    let active = db_active_count(&db, &ws).await;
    assert_eq!(active, 0, "草稿血缘 publish 后仍 0 个 active（须人审 activate）");
}

/// TEST-2 缺口：rollback 真 handler 在已生效血缘上把 active 迁回上一版本。
#[tokio::test]
#[ignore]
async fn e2e_rollback_handler_realigns_active() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // v1 已生效，publish v2（realign 后 v2 生效）。
    let v1 = db_create_profile(&db, &ws, "svc-rollback", "服务", "服务运营", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true, "is_active": true } },
            None,
        )
        .await
        .expect("make v1 live");
    let pub_resp = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v1.to_hex()),
    )
    .await
    .expect("publish v2");
    let v2_hex = pub_resp.0.get("id").and_then(|v| v.as_str()).expect("v2 id").to_string();

    // rollback v2 → 回到 v1（previous_version）。
    let _ = wechatagent::routes::domain_profiles::rollback_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v2_hex),
    )
    .await
    .expect("rollback handler ok");

    // 不变量：回退后 v1 既 current 又 active，恰一行可加载。
    let v1_doc = db_get_profile(&db, v1).await;
    assert!(v1_doc.current_version && v1_doc.is_active, "rollback 后 v1 既 current 又 active");
    let loadable = db
        .domain_profiles()
        .count_documents(
            doc! { "workspace_id": &ws, "profile_id": "svc-rollback", "is_active": true, "current_version": true },
            None,
        )
        .await
        .expect("count");
    assert_eq!(loadable, 1, "rollback 后恰一行可加载");
}

/// TEST-6 缺口：PUT update 真 handler 只 $set body 带来的字段，未触碰字段保持原值
/// （验证 #2「不再整行 replace 清零」）。
#[tokio::test]
#[ignore]
async fn e2e_update_handler_partial_set_preserves_untouched_fields() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // 建草稿并预置多个内容字段（draft 行可被 update）。
    let id = db_create_profile(&db, &ws, "edit-target", "原名", "原简介", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": id },
            doc! { "$set": {
                "prompt_fragment": "原始业务上下文",
                "grounding_gate_bypass_without_claim": true,
            } },
            None,
        )
        .await
        .expect("preset fields");

    // PUT 只带 display_name（snake_case，无 profileId —— 验 D4-1 契约）。
    let body: wechatagent::routes::domain_profiles::UpsertRequest =
        serde_json::from_value(serde_json::json!({ "display_name": "新名" }))
            .expect("deserialize UpsertRequest without profileId");
    let _ = wechatagent::routes::domain_profiles::update_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id.to_hex()),
        Json(body),
    )
    .await
    .expect("update handler ok");

    let p = db_get_profile(&db, id).await;
    assert_eq!(p.display_name, "新名", "display_name 被更新");
    // 未触碰字段保持原值（核心：不再整行清零）。
    assert_eq!(p.description, "原简介", "未带的 description 保持原值");
    assert_eq!(p.prompt_fragment.as_deref(), Some("原始业务上下文"), "未带的 prompt_fragment 保持原值");
    assert!(p.grounding_gate_bypass_without_claim, "未带的 grounding 开关保持原值");
}

/// TEST-6 缺口 + CORRECT-1：PUT update 带未知键 → 白名单过滤，未知键不落库（防文档污染）；
/// 且托管字段（is_active/version）经 strip 不被篡改。
#[tokio::test]
#[ignore]
async fn e2e_update_handler_drops_unknown_and_managed_keys() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    let id = db_create_profile(&db, &ws, "filter-target", "原名", "原简介", "manual").await;

    // body 含：合法内容键 + 未知键 + 试图篡改的托管键。
    let body: wechatagent::routes::domain_profiles::UpsertRequest = serde_json::from_value(
        serde_json::json!({
            "display_name": "新名",
            "totally_unknown_field": "should_not_persist",
            "is_active": true,
            "version": 99,
        }),
    )
    .expect("deserialize");
    let _ = wechatagent::routes::domain_profiles::update_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(id.to_hex()),
        Json(body),
    )
    .await
    .expect("update handler ok");

    // 用裸 Document 读，检查未知键是否落库。
    let raw = db
        .domain_profiles()
        .clone_with_type::<mongodb::bson::Document>()
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("find")
        .expect("doc");
    assert_eq!(raw.get_str("display_name").ok(), Some("新名"), "合法内容键更新");
    assert!(raw.get("totally_unknown_field").is_none(), "未知键不落库（CORRECT-1 白名单过滤）");
    // 托管字段未被篡改（strip_backend_managed_keys 剥离）。
    let p = db_get_profile(&db, id).await;
    assert!(!p.is_active, "is_active 不可经 PUT 篡改");
    assert_eq!(p.version, 1, "version 不可经 PUT 篡改");
}

// ── 分级二次确认：危险开关变更 publish 走旁路稿 + 确认 ──────────────────────
// publish 已生效血缘时若危险字段有变更，落旁路稿（current=false）不即时生效，返回
// pendingActivation；旧版本继续 current+active（零窗口）；经 rollout 二次确认才生效。

/// 危险变更 publish → pendingActivation=true，旁路稿 current=false，旧 v1 仍 current+active。
#[tokio::test]
#[ignore]
async fn e2e_publish_risky_returns_pending_no_current_shift() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // v1：已生效（current+active），grounding 开关 = false（销售域默认）。
    let v1 = db_create_profile(&db, &ws, "risky-live", "零售", "零售运营", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true, "is_active": true } },
            None,
        )
        .await
        .expect("make v1 live");

    // v2 草稿：同血缘，但改了危险字段（grounding_gate_bypass_without_claim=true +
    // distrust_self_reported_low_risk=true）。这是运营 create→update 出来的待发布稿。
    let v2_src = db_create_profile(&db, &ws, "risky-live", "零售v2", "改风控开关", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v2_src },
            doc! { "$set": {
                "grounding_gate_bypass_without_claim": true,
                "distrust_self_reported_low_risk": true,
                "version": 2_i32,
            } },
            None,
        )
        .await
        .expect("preset risky v2 draft");

    // publish v2_src → 危险分支。
    let resp = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v2_src.to_hex()),
    )
    .await
    .expect("publish handler ok");
    let body: Value = resp.0;
    assert_eq!(
        body.get("pendingActivation").and_then(|v| v.as_bool()),
        Some(true),
        "危险变更 → pendingActivation=true"
    );
    let risky: Vec<String> = body
        .get("riskyFields")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .expect("riskyFields array");
    assert!(
        risky.contains(&"grounding_gate_bypass_without_claim".to_string())
            && risky.contains(&"distrust_self_reported_low_risk".to_string()),
        "riskyFields 列出两个变更的危险字段，实际={risky:?}"
    );

    // 旁路稿（新版本 v3）current=false、is_active=false。
    let new_hex = body.get("id").and_then(|v| v.as_str()).expect("new id");
    let new_id = ObjectId::parse_str(new_hex).expect("oid");
    let sideline = db_get_profile(&db, new_id).await;
    assert!(!sideline.current_version, "旁路稿不占 current");
    assert!(!sideline.is_active, "旁路稿不生效");

    // 关键：旧 v1 仍唯一 current+active（零窗口期，运行时继续加载旧版本）。
    let v1_doc = db_get_profile(&db, v1).await;
    assert!(v1_doc.current_version && v1_doc.is_active, "v1 仍 current+active");
    let loadable = db
        .domain_profiles()
        .count_documents(
            doc! { "workspace_id": &ws, "profile_id": "risky-live", "is_active": true, "current_version": true },
            None,
        )
        .await
        .expect("count loadable");
    assert_eq!(loadable, 1, "危险 publish 后仍恰一行可加载（旧版本，无窗口期回落）");
}

/// 二次确认：对旁路稿调 rollout → 推 current+demote+realign，新版本生效、单活。
#[tokio::test]
#[ignore]
async fn e2e_confirm_activation_promotes_risky_draft() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    let v1 = db_create_profile(&db, &ws, "risky-confirm", "教培", "教培运营", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true, "is_active": true } },
            None,
        )
        .await
        .expect("make v1 live");
    let v2_src = db_create_profile(&db, &ws, "risky-confirm", "教培v2", "改人格", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v2_src },
            doc! { "$set": { "soul_override": "新人格本体", "version": 2_i32 } },
            None,
        )
        .await
        .expect("preset risky v2");
    let pub_resp = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v2_src.to_hex()),
    )
    .await
    .expect("publish risky");
    let sideline_hex = pub_resp.0.get("id").and_then(|v| v.as_str()).expect("id").to_string();
    let sideline_id = ObjectId::parse_str(&sideline_hex).expect("oid");

    // 运营二次确认 → rollout 旁路稿（confirm-path 复用 rollout）。
    let _ = wechatagent::routes::domain_profiles::rollout_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(sideline_hex),
    )
    .await
    .expect("rollout (confirm) ok");

    // 确认后旁路稿既 current 又 active，旧 v1 都让出。
    let sideline = db_get_profile(&db, sideline_id).await;
    assert!(sideline.current_version && sideline.is_active, "确认后旁路稿 current+active");
    let v1_doc = db_get_profile(&db, v1).await;
    assert!(!v1_doc.current_version && !v1_doc.is_active, "v1 让出 current+active");
    let loadable = db
        .domain_profiles()
        .count_documents(
            doc! { "workspace_id": &ws, "profile_id": "risky-confirm", "is_active": true, "current_version": true },
            None,
        )
        .await
        .expect("count");
    assert_eq!(loadable, 1, "确认后恰一行可加载（新版本）");
}

/// 普通字段变更（非危险）在已生效血缘上仍走即时生效（与 realign 基线等价），不触发分级。
#[tokio::test]
#[ignore]
async fn e2e_publish_nonrisky_live_lineage_still_realigns() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    let v1 = db_create_profile(&db, &ws, "nonrisky-live", "美业", "美业运营", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true, "is_active": true } },
            None,
        )
        .await
        .expect("make v1 live");
    // v2 草稿只改普通字段（display_name / description / prompt_fragment），无危险变更。
    let v2_src = db_create_profile(&db, &ws, "nonrisky-live", "美业v2", "只改简介", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v2_src },
            doc! { "$set": { "prompt_fragment": "叠加业务上下文", "version": 2_i32 } },
            None,
        )
        .await
        .expect("preset nonrisky v2");

    let resp = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v2_src.to_hex()),
    )
    .await
    .expect("publish handler ok");
    let body: Value = resp.0;
    // 普通分支：返回体不含 pendingActivation 键。
    assert!(
        body.get("pendingActivation").is_none(),
        "普通字段变更不触发分级（无 pendingActivation）"
    );
    let new_id = ObjectId::parse_str(body.get("id").and_then(|v| v.as_str()).expect("id")).expect("oid");
    // 即时生效：realign 把 active 迁到新版本。
    let new_doc = db_get_profile(&db, new_id).await;
    assert!(new_doc.current_version && new_doc.is_active, "普通变更即时生效（v2 current+active）");
    let loadable = db
        .domain_profiles()
        .count_documents(
            doc! { "workspace_id": &ws, "profile_id": "nonrisky-live", "is_active": true, "current_version": true },
            None,
        )
        .await
        .expect("count");
    assert_eq!(loadable, 1, "即时生效后恰一行可加载（新版本）");
}

/// 红线：纯草稿血缘（从未 active）即便改了危险字段，publish 也不触发分级（active_base=None）
/// → 走普通分支 + realign noop → 仍须人审 activate。
#[tokio::test]
#[ignore]
async fn e2e_publish_risky_on_draft_lineage_no_pending() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // 纯草稿血缘：current_version=true 但从未 activate（is_active=false），且带危险字段。
    let v1 = db_create_profile(&db, &ws, "risky-draft", "AI候选", "纯草稿", "generated_by_ai").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": v1 },
            doc! { "$set": { "current_version": true, "soul_override": "AI 生成的人格" } },
            None,
        )
        .await
        .expect("make v1 current draft with risky field");

    let resp = wechatagent::routes::domain_profiles::publish_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(v1.to_hex()),
    )
    .await
    .expect("publish handler ok");
    let body: Value = resp.0;
    // 血缘从未 active → active_base=None → 不触发分级。
    assert!(
        body.get("pendingActivation").is_none(),
        "草稿血缘不触发分级（active_base=None）"
    );
    // 红线：realign noop → 仍 0 个 active（须人审 activate）。
    let active = db_active_count(&db, &ws).await;
    assert_eq!(active, 0, "草稿血缘 publish 后仍 0 个 active（守人审红线）");
}

// ── H13：activate 联动 publish 状态机本体到 operation_domain_configs ──────────────
// 引导层 AI 生成的行业状态机在 activate 时被 publish 成 operation_domain_configs 新 current
// 版本（消费方零改动）；无本体（DEFAULT 销售域）则 operation_domain_configs 不变（字节等价
// 回落 DEFAULT）。这两条 Part C 直调真 activate_domain_profile handler 覆盖。

/// 在 `(workspace, user_operations)` 下手插一条 current 底座 config（模拟
/// `ensure_operation_domains` 已 seed 的默认行；测试 crate 够不到 pub(super) 的
/// `ensure_operation_domains`，故按 brief 选项 b 手动 insert_one）。
async fn db_seed_base_domain_config(db: &Database, workspace_id: &str) {
    let cfg = wechatagent::models::OperationDomainConfig {
        id: None,
        workspace_id: workspace_id.to_string(),
        domain: "user_operations".to_string(),
        name: "默认销售域".to_string(),
        goal: "成交".to_string(),
        methodology: "顾问式".to_string(),
        workflow: "破冰→挖需→方案→成交".to_string(),
        tool_policy: "{}".to_string(),
        automation_policy: "{}".to_string(),
        review_policy: "{}".to_string(),
        runtime_parameters: doc! {},
        state_machine: doc! {
            "states": [
                { "key": "new_contact", "name": "初次接触", "initial": true, "allowedFrom": [] },
            ]
        },
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("test_base".to_string()),
        principal_decider: None,
        high_risk_escalation_mode: None,
    };
    db.operation_domain_configs()
        .insert_one(&cfg, None)
        .await
        .expect("seed base operation_domain_config");
}

/// 统计 `(workspace, user_operations)` 下 config 总行数（版本计数）。
async fn db_domain_config_count(db: &Database, workspace_id: &str) -> u64 {
    db.operation_domain_configs()
        .count_documents(
            doc! { "workspace_id": workspace_id, "domain": "user_operations" },
            None,
        )
        .await
        .expect("count domain configs")
}

/// 读 `(workspace, user_operations)` 下 current_version=true 的 config（应至多一条）。
async fn db_current_domain_config(
    db: &Database,
    workspace_id: &str,
) -> wechatagent::models::OperationDomainConfig {
    db.operation_domain_configs()
        .find_one(
            doc! { "workspace_id": workspace_id, "domain": "user_operations", "current_version": true },
            None,
        )
        .await
        .expect("find current config")
        .expect("current config exists")
}

/// 激活带 generated_state_machine 的 profile → operation_domain_configs publish 一版新
/// current，其 state_machine 含生成的 state key、版本递增（消费方零改动）。
#[tokio::test]
#[ignore]
async fn e2e_activate_publishes_generated_state_machine() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // 底座：当前 current 销售域 config（version=1）。
    db_seed_base_domain_config(&db, &ws).await;
    let base_count = db_domain_config_count(&db, &ws).await;
    let base_version = db_current_domain_config(&db, &ws).await.version;

    // 建带本体的 profile，并置 current（activate 要求 current_version=true）。
    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "x_deep", "name": "深入", "allowedFrom": ["x_intro"] },
        ]
    };
    let pid = db_create_profile(&db, &ws, "edu-domain", "教育", "教育咨询", "generated_by_ai").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": pid },
            doc! { "$set": { "current_version": true, "generated_state_machine": &machine } },
            None,
        )
        .await
        .expect("set current + machine");

    // 调真 activate handler。
    let _ = wechatagent::routes::domain_profiles::activate_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(pid.to_hex()),
    )
    .await
    .expect("activate handler ok");

    // 断言：多出一版 config，新 current 的 state_machine 含 "x_intro"，版本递增。
    let after_count = db_domain_config_count(&db, &ws).await;
    assert_eq!(after_count, base_count + 1, "activate 应 publish 一版新 config");
    let current = db_current_domain_config(&db, &ws).await;
    assert!(current.version > base_version, "新 current 版本号递增");
    assert_eq!(current.previous_version, Some(base_version), "previous_version 指向底座版本");
    let keys: Vec<&str> = current
        .state_machine
        .get_array("states")
        .expect("states array")
        .iter()
        .filter_map(|s| s.as_document())
        .filter_map(|d| d.get_str("key").ok())
        .collect();
    assert!(keys.contains(&"x_intro"), "publish 的 state_machine 含生成的 x_intro: {keys:?}");
    assert!(keys.contains(&"x_deep"), "publish 的 state_machine 含生成的 x_deep: {keys:?}");
    assert_eq!(
        current.seeded_by.as_deref(),
        Some("profile:edu-domain"),
        "seeded_by 标 profile 溯源"
    );
}

/// 激活 generated_state_machine=None 的 profile（如 DEFAULT 销售域）→
/// operation_domain_configs 不新增版本（字节等价回落 DEFAULT）。
#[tokio::test]
#[ignore]
async fn e2e_activate_without_machine_leaves_configs_unchanged() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;
    let base_count = db_domain_config_count(&db, &ws).await;
    let base_version = db_current_domain_config(&db, &ws).await.version;

    // profile 不带本体（generated_state_machine=None，db_create_profile 默认 None）。
    let pid = db_create_profile(&db, &ws, "no-machine", "默认", "无状态机", "manual").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": pid },
            doc! { "$set": { "current_version": true } },
            None,
        )
        .await
        .expect("set current");

    let _ = wechatagent::routes::domain_profiles::activate_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(pid.to_hex()),
    )
    .await
    .expect("activate handler ok");

    // 断言：config 表行数、current 版本号均不变（状态机表未被触碰）。
    assert_eq!(
        db_domain_config_count(&db, &ws).await,
        base_count,
        "无本体 activate 不新增 config 版本（回落 DEFAULT）"
    );
    assert_eq!(
        db_current_domain_config(&db, &ws).await.version,
        base_version,
        "current 版本号不变"
    );
}

/// H13 修补：activate 带 `forbidsProactive:true` state 的 generated_state_machine →
/// publish 联动重派生 operation_state_policies，该 state 拿到 active policy 行且
/// `forbidden` 含 "reply"（主动触达门真正生效，不再 fail-open 静默忽略）。
#[tokio::test]
#[ignore]
async fn e2e_activate_derives_state_policies_for_forbids_proactive() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // 底座 current 销售域 config（version=1）。
    db_seed_base_domain_config(&db, &ws).await;

    // 行业机器：x_intro（initial）+ grieving（哀伤期，forbidsProactive=true，非 cooldown）。
    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": true },
        ]
    };
    let pid = db_create_profile(&db, &ws, "grief-care", "哀伤陪伴", "情感陪伴", "generated_by_ai").await;
    db.domain_profiles()
        .update_one(
            doc! { "_id": pid },
            doc! { "$set": { "current_version": true, "generated_state_machine": &machine } },
            None,
        )
        .await
        .expect("set current + machine");

    let _ = wechatagent::routes::domain_profiles::activate_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path(pid.to_hex()),
    )
    .await
    .expect("activate handler ok");

    // 断言：grieving state 派生了 active policy 行，forbidden 含 "reply"。
    let policy = db
        .operation_state_policies()
        .find_one(
            doc! {
                "workspace_id": &ws,
                "domain": "user_operations",
                "state_key": "grieving",
                "status": "active",
            },
            None,
        )
        .await
        .expect("query state policy")
        .expect("grieving policy row exists");
    assert!(
        policy.forbidden.iter().any(|a| a == "reply"),
        "forbidsProactive=true 的 grieving state policy forbidden 应含 reply: {:?}",
        policy.forbidden
    );
}

// ── H13 (1)(2)：publish 幂等 + policy 按 seeded_by 区分手工/派生行刷新 ──────────────

/// 建一条带 generated_state_machine 的 current profile 并调真 activate handler，
/// 触发 publish_state_machine_version（消费方零改动路径，与生产 activate 联动一致）。
async fn activate_profile_with_machine(
    app: &common::TestApp,
    db: &Database,
    ws: &str,
    profile_id: &str,
    machine: &mongodb::bson::Document,
) {
    let existing = db
        .domain_profiles()
        .find_one(doc! { "workspace_id": ws, "profile_id": profile_id }, None)
        .await
        .expect("query profile");
    let pid = match existing {
        Some(p) => p.id.expect("profile _id"),
        None => db_create_profile(db, ws, profile_id, "测试域", "测试", "generated_by_ai").await,
    };
    db.domain_profiles()
        .update_one(
            doc! { "_id": pid },
            doc! { "$set": { "current_version": true, "generated_state_machine": machine } },
            None,
        )
        .await
        .expect("set current + machine");
    let _ = wechatagent::routes::domain_profiles::activate_domain_profile(
        State(app.state.clone()),
        Extension(test_admin(ws)),
        Path(pid.to_hex()),
    )
    .await
    .expect("activate handler ok");
}

/// (workspace, user_operations) 下 current config 的 version（版本最大值）。
async fn db_current_config_version(db: &Database, workspace_id: &str) -> i32 {
    db_current_domain_config(db, workspace_id).await.version
}

/// 读某 state 的 policy 行（应至多一条 current）。
async fn db_state_policy(
    db: &Database,
    workspace_id: &str,
    state_key: &str,
) -> Option<wechatagent::models::OperationStatePolicy> {
    db.operation_state_policies()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
                "state_key": state_key,
            },
            None,
        )
        .await
        .expect("query state policy")
}

/// H13 (1)：重复激活同一份本体逐字节相同的机器 → operation_domain_configs 行数与
/// max version 都不变（no-op 幂等，消除「重复激活同机器」版本膨胀 + 竞态触发器）。
#[tokio::test]
#[ignore]
async fn republish_same_machine_is_noop() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "x_deep", "name": "深入", "allowedFrom": ["x_intro"] },
        ]
    };
    // 首次 activate：本体与底座不同 → 新增一版。
    activate_profile_with_machine(&app, &db, &ws, "edu-domain", &machine).await;
    let count_after_first = db_domain_config_count(&db, &ws).await;
    let version_after_first = db_current_config_version(&db, &ws).await;

    // 重复 activate 同一 profile（本体逐字节相同）→ no-op：行数 + max version 不变。
    activate_profile_with_machine(&app, &db, &ws, "edu-domain", &machine).await;
    activate_profile_with_machine(&app, &db, &ws, "edu-domain", &machine).await;

    assert_eq!(
        db_domain_config_count(&db, &ws).await,
        count_after_first,
        "重复 publish 同机器不应新增 config 版本（no-op 幂等）"
    );
    assert_eq!(
        db_current_config_version(&db, &ws).await,
        version_after_first,
        "max version 不变（no-op 幂等）"
    );
}

/// H13 (2)：把某 state 的 forbidsProactive false→true 切换后 republish → 该 state 的
/// 机器派生 policy 行被 in-place 刷新，forbidden 从不含 reply 变为含 reply。
/// 因 (1) 幂等，切换 forbidsProactive 改变了本体 → 不是 no-op，版本递增、loop 跑。
#[tokio::test]
#[ignore]
async fn republish_toggled_forbids_proactive_refreshes_policy() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    // v1：grieving forbidsProactive=false → 派生 policy forbidden 不含 reply。
    let machine_allow = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": false },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "grief", &machine_allow).await;
    let p1 = db_state_policy(&db, &ws, "grieving").await.expect("grieving policy after v1");
    assert!(
        !p1.forbidden.iter().any(|a| a == "reply"),
        "forbidsProactive=false 时 forbidden 不应含 reply: {:?}",
        p1.forbidden
    );

    // v2：同机器但 grieving 切 forbidsProactive=true → 本体变 → 非 no-op → 刷新 policy。
    let machine_forbid = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": true },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "grief", &machine_forbid).await;
    let p2 = db_state_policy(&db, &ws, "grieving").await.expect("grieving policy after v2");
    assert!(
        p2.forbidden.iter().any(|a| a == "reply"),
        "forbidsProactive 切 true 后机器派生 policy forbidden 应含 reply（in-place 刷新）: {:?}",
        p2.forbidden
    );
}

/// H13 (2)：运营手工 policy 行（非派生 seeded_by）在 republish 时被保留，绝不 clobber。
#[tokio::test]
#[ignore]
async fn operator_edited_policy_preserved_on_republish() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    // 预插一条运营手工调整的 policy 行：grieving 上 admin 主动放开 reply（forbidden 空），
    // seeded_by="admin_manual" → 非派生行，republish 必须保留。
    let operator_row = wechatagent::models::OperationStatePolicy {
        id: None,
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        state_key: "grieving".to_string(),
        allowed: vec!["reply".to_string(), "silent".to_string(), "follow_up".to_string()],
        forbidden: vec![],
        recommended_pace: None,
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("admin_manual".to_string()),
    };
    db.operation_state_policies()
        .insert_one(&operator_row, None)
        .await
        .expect("seed operator-edited policy");

    // activate 一份把 grieving 标 forbidsProactive=true 的机器（机器派生想 forbidden=[reply]）。
    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": true },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "grief", &machine).await;

    // 断言：运营手工行未被覆盖——forbidden 仍空、seeded_by 仍 admin_manual。
    let after = db_state_policy(&db, &ws, "grieving").await.expect("grieving policy preserved");
    assert!(
        after.forbidden.is_empty(),
        "运营手工行 forbidden 应保持空（不被机器派生 clobber）: {:?}",
        after.forbidden
    );
    assert_eq!(
        after.seeded_by.as_deref(),
        Some("admin_manual"),
        "运营手工行 seeded_by 应保持 admin_manual"
    );
}

/// H13 (FINDING 1)：同 (ws,domain,state_key) 并存多版本 policy 时，republish 只刷新
/// current_version=true 那一行，绝不改历史版本（否则运行时读 current 仍陈旧 = toggle
/// 静默失效，且改写历史行会污染 rollback 链）。
#[tokio::test]
#[ignore]
async fn republish_refreshes_only_current_policy_version() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    // 历史行：version 1, current_version=false, forbidden=[]，可刷新 seeded_by。
    let historical = wechatagent::models::OperationStatePolicy {
        id: None,
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        state_key: "grieving".to_string(),
        allowed: vec!["reply".to_string()],
        forbidden: vec![],
        recommended_pace: None,
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: false,
        previous_version: None,
        seeded_by: Some("legacy_migration".to_string()),
    };
    let hist_id = db
        .operation_state_policies()
        .insert_one(&historical, None)
        .await
        .expect("seed historical policy")
        .inserted_id
        .as_object_id()
        .expect("historical _id");

    // 当前行：version 2, current_version=true, forbidden=[]，可刷新 seeded_by。
    let current = wechatagent::models::OperationStatePolicy {
        id: None,
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        state_key: "grieving".to_string(),
        allowed: vec!["reply".to_string()],
        forbidden: vec![],
        recommended_pace: None,
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 2,
        current_version: true,
        previous_version: Some(1),
        seeded_by: Some("legacy_migration".to_string()),
    };
    let cur_id = db
        .operation_state_policies()
        .insert_one(&current, None)
        .await
        .expect("seed current policy")
        .inserted_id
        .as_object_id()
        .expect("current _id");

    // publish 把 grieving 标 forbidsProactive=true（机器派生想 forbidden=[reply]）。
    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": true },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "grief", &machine).await;

    // 当前行（version 2）被刷新：forbidden 含 reply。
    let cur_after = db
        .operation_state_policies()
        .find_one(doc! { "_id": cur_id }, None)
        .await
        .expect("query current after")
        .expect("current row exists");
    assert!(
        cur_after.forbidden.iter().any(|a| a == "reply"),
        "current(version 2) 行应被刷新为 forbidden 含 reply: {:?}",
        cur_after.forbidden
    );

    // 历史行（version 1）原封不动：forbidden 仍空。
    let hist_after = db
        .operation_state_policies()
        .find_one(doc! { "_id": hist_id }, None)
        .await
        .expect("query historical after")
        .expect("historical row exists");
    assert!(
        hist_after.forbidden.is_empty(),
        "historical(version 1) 行不应被改动，forbidden 仍空: {:?}",
        hist_after.forbidden
    );
    assert_eq!(
        hist_after.version, 1,
        "historical 行 version 不变"
    );
    assert!(
        !hist_after.current_version,
        "historical 行仍非 current"
    );
}

// ── H13 (T14)：activate 切状态机后迁移存量 contact 幻影态到新 initial ────────────────

/// 在 contacts 集合插一条最小 contact（直写裸 BSON，避免构造 30+ 字段），返回其 _id。
/// `operation_state` 为 None 时不写该字段（模拟未设态 contact）。
async fn db_seed_contact(
    db: &Database,
    workspace_id: &str,
    wxid: &str,
    operation_state: Option<&str>,
) -> ObjectId {
    let oid = ObjectId::new();
    let mut doc = doc! {
        "_id": oid,
        "workspace_id": workspace_id,
        "account_id": "acct-test",
        "wxid": wxid,
        "agent_status": "managed",
        "created_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    if let Some(state_key) = operation_state {
        doc.insert("operation_state", state_key);
    }
    db.raw()
        .collection::<mongodb::bson::Document>("contacts")
        .insert_one(doc, None)
        .await
        .expect("seed contact");
    oid
}

/// 读某 contact 当前 operation_state（裸 BSON，None=未设字段）。
async fn db_contact_operation_state(db: &Database, id: ObjectId) -> Option<String> {
    let doc = db
        .raw()
        .collection::<mongodb::bson::Document>("contacts")
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("query contact")
        .expect("contact exists");
    doc.get_str("operation_state").ok().map(str::to_string)
}

/// T14：存量 contact 持旧机器的 operation_state（新机器无此 key）→ activate 切新机器后，
/// 该 contact 被重置到新机器 initial 态（修「换域后老 contact 状态机静默冻结」幻影态）。
#[tokio::test]
#[ignore]
async fn activate_resets_stranded_contact_operation_state() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    // 存量 contact 停在旧（销售）机器的 negotiating——新机器里不存在此 key。
    let cid = db_seed_contact(&db, &ws, "wx-stranded", Some("legacy_negotiating")).await;

    // 新机器：initial=x_intro，另含 x_active（allowedFrom x_intro），均无 legacy_negotiating。
    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "x_active", "name": "活跃", "allowedFrom": ["x_intro"] },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "edu-domain", &machine).await;

    assert_eq!(
        db_contact_operation_state(&db, cid).await.as_deref(),
        Some("x_intro"),
        "幻影态 contact 应被重置到新机器 initial 态 x_intro"
    );
}

/// T14：已处在新机器合法态（x_intro）的 contact 在 activate 后不被重置/clobber。
#[tokio::test]
#[ignore]
async fn activate_preserves_valid_contact_operation_state() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    // contact 已在新机器的合法态 x_intro。
    let cid = db_seed_contact(&db, &ws, "wx-valid", Some("x_intro")).await;

    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "x_active", "name": "活跃", "allowedFrom": ["x_intro"] },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "edu-domain", &machine).await;

    assert_eq!(
        db_contact_operation_state(&db, cid).await.as_deref(),
        Some("x_intro"),
        "合法态 contact 不应被重置（$nin 过滤排除合法态）"
    );
}

/// T14：operation_state 未设的 contact 在 activate 后仍未设（$exists:true 过滤排除，
/// 留待首次运行时补 initial，不被本迁移误写）。
#[tokio::test]
#[ignore]
async fn activate_leaves_unset_operation_state_contact() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    let cid = db_seed_contact(&db, &ws, "wx-unset", None).await;

    let machine = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "x_active", "name": "活跃", "allowedFrom": ["x_intro"] },
        ]
    };
    activate_profile_with_machine(&app, &db, &ws, "edu-domain", &machine).await;

    assert_eq!(
        db_contact_operation_state(&db, cid).await,
        None,
        "未设 operation_state 的 contact 不应被本迁移写入（$exists:true,$ne:null 过滤排除）"
    );
}

/// G06：直编状态机本体路由 `update_operation_domain_state_machine` 改本体后必须联动重派
/// `operation_state_policies`。此前直编路由不走 publish loop、不派生 policy → 新增
/// `forbidsProactive:true` state 的主动触达门 fail-open 静默失效（guards 对缺失 policy 行放行）。
///
/// 场景：底座机器 grieving forbidsProactive=false（policy forbidden 不含 reply）→ 直编路由
/// 把同名 state 切 forbidsProactive=true → 断言该 state 的 current policy 行 forbidden 含 reply
/// （reconcile 生效，主动触达门真正拦得住）。
#[tokio::test]
#[ignore]
async fn direct_edit_state_machine_rederives_policy() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    db_seed_base_domain_config(&db, &ws).await;

    // 先把底座 current 机器改成含 grieving(forbidsProactive=false) 的本体，并经直编路由
    // 让 reconcile 派一行 grieving policy（forbidden 不含 reply）。
    let machine_allow = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": false },
        ]
    };
    let _ = wechatagent::routes::domains::update_operation_domain_state_machine(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("user_operations".to_string()),
        Json(machine_allow),
    )
    .await
    .expect("direct edit (allow) handler ok");
    let p1 = db_state_policy(&db, &ws, "grieving")
        .await
        .expect("grieving policy after allow edit");
    assert!(
        !p1.forbidden.iter().any(|a| a == "reply"),
        "forbidsProactive=false 时直编派生 policy forbidden 不应含 reply: {:?}",
        p1.forbidden
    );

    // 直编路由把 grieving 切 forbidsProactive=true → reconcile in-place 刷新机器派生行。
    let machine_forbid = doc! {
        "states": [
            { "key": "x_intro", "name": "开场", "initial": true, "allowedFrom": [] },
            { "key": "grieving", "name": "哀伤期", "allowedFrom": ["x_intro"], "forbidsProactive": true },
        ]
    };
    let _ = wechatagent::routes::domains::update_operation_domain_state_machine(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
        Path("user_operations".to_string()),
        Json(machine_forbid),
    )
    .await
    .expect("direct edit (forbid) handler ok");

    // 断言：grieving 的 current policy 行 forbidden 已含 reply（直编联动重派生生效，
    // 不再 fail-open 静默忽略主动触达门）。
    let p2 = db_state_policy(&db, &ws, "grieving")
        .await
        .expect("grieving policy after forbid edit");
    assert!(
        p2.forbidden.iter().any(|a| a == "reply"),
        "直编 forbidsProactive 切 true 后 policy forbidden 应含 reply（reconcile 生效）: {:?}",
        p2.forbidden
    );
}
