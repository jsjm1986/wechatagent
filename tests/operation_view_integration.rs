//! `operation_view_integration` —— 运营态聚合端点 `GET /api/operation/active-view`
//! 集成测试（`#[ignore]`，需要 Docker / testcontainers MongoDB）。
//!
//! 验证流 B（前端翻译）后端地基：
//! - dimensions 来自当前 active DomainProfile 的 profile_dimensions（camelCase wire）;
//! - taxonomies 把每个维度 kind 的 system_taxonomies 取值映射成 {id, label}；
//! - kind 集 = profile_dimensions ∪ ["relationship_type"]（M3）——即便 relationship_type
//!   不在 profile_dimensions 里、其取值字典为空，taxonomies 仍须含该键，供前端关系下拉。
//!
//! 直调真 handler（`State` + `Extension(AuthenticatedAdmin)`），仿 domain_profile_e2e 先例，
//! 不经 axum HTTP 层；鉴权角色由测试构造的 `test_admin` 注入。
//!
//! ## 运行
//! ```sh
//! cargo test --test operation_view_integration -- --ignored --nocapture
//! ```

mod common;

use axum::extract::{Extension, State};
use mongodb::bson::DateTime;
use serde_json::Value;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{
    CommitmentMarkers, DomainProfile, OperationMode, OutcomePolarity, ProfileDimension,
    TaxonomyEntry, TaxonomyValue,
};

/// 构造测试 admin auth context（与 domain_profile_e2e::test_admin 同形）。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "test_admin".to_string(),
        username: "test_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条 active DomainProfile，profile_dimensions 含给定维度。
fn make_active_profile(workspace_id: &str, dimensions: Vec<ProfileDimension>) -> DomainProfile {
    DomainProfile {
        id: None,
        profile_id: "test-active-view".to_string(),
        workspace_id: workspace_id.to_string(),
        display_name: "测试 active 视图 profile".to_string(),
        description: "operation_view_integration 专用".to_string(),
        profile_dimensions: dimensions,
        prompt_fragment: None,
        soul_override: None,
        methodology_override: None,
        conversation_mode_policy: None,
        commitment_markers: CommitmentMarkers {
            product_effect: vec![],
            tone_only: vec![],
        },
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
        current_version: true,
        previous_version: None,
        is_active: true,
        seeded_by: Some("manual".to_string()),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
        threshold_overrides: None,
        reviewer_orientation: None,
        mode_gate_policy_override: None,
        answering_mode_profile: None,
        generated_state_machine: None,
        version: 1,
    }
}

/// 构造一条 active 的 global taxonomy 取值条目。
fn make_taxonomy_entry(
    workspace_id: &str,
    kind: &str,
    id: &str,
    display_name: &str,
) -> TaxonomyEntry {
    TaxonomyEntry {
        id: None,
        workspace_id: workspace_id.to_string(),
        scope: "global".to_string(),
        kind: kind.to_string(),
        value: TaxonomyValue {
            id: id.to_string(),
            display_name: display_name.to_string(),
            description: String::new(),
            aliases: vec![],
            status: "active".to_string(),
            priority_weight: None,
            is_terminal: false,
            is_reactivation_target: false,
        },
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: Some("manual".to_string()),
    }
}

#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn active_view_returns_dimensions_and_taxonomy_labels() {
    let app = common::TestApp::start().await;
    let db = app.state.db.clone();
    let ws = app.state.config.default_workspace_id.clone();

    // 种一条 active DomainProfile：profile_dimensions 含 2 维度（customer_stage +
    // emotion_state）。
    let profile = make_active_profile(
        &ws,
        vec![
            ProfileDimension {
                kind: "customer_stage".to_string(),
                display_name: "客户阶段".to_string(),
                participates_in_decision: true,
                description: "客户当前所处的阶段".to_string(),
            },
            ProfileDimension {
                kind: "emotion_state".to_string(),
                display_name: "情绪状态".to_string(),
                participates_in_decision: false,
                description: "客户当前的情绪状态".to_string(),
            },
        ],
    );
    db.domain_profiles()
        .insert_one(&profile, None)
        .await
        .expect("insert active profile");

    // 种 system_taxonomies：customer_stage:first_contact→初次接触。
    db.collection_system_taxonomies()
        .insert_one(
            &make_taxonomy_entry(&ws, "customer_stage", "first_contact", "初次接触"),
            None,
        )
        .await
        .expect("insert taxonomy entry");

    // 重新预热两个进程级缓存，使其对齐本测试刚种入的 DB 状态（warm_up 忽略 TTL
    // 无条件 reload；与 TestApp::start 内启动期预热同源）。
    wechatagent::agent::init_global_taxonomy_cache(&db).await;
    wechatagent::agent::init_global_domain_profile_cache(&db).await;

    // 直调真 handler。
    let response = wechatagent::routes::operation_view::active_view(
        State(app.state.clone()),
        Extension(test_admin(&ws)),
    )
    .await
    .expect("active_view handler ok");

    let body: Value = response.0;

    // dimensions：长度 2，含 customer_stage + emotion_state（camelCase wire）。
    let dimensions = body["dimensions"].as_array().expect("dimensions array");
    assert_eq!(dimensions.len(), 2, "应返回 profile 声明的 2 个维度");
    let dim_kinds: Vec<&str> = dimensions
        .iter()
        .map(|d| d["kind"].as_str().expect("dimension kind"))
        .collect();
    assert!(dim_kinds.contains(&"customer_stage"));
    assert!(dim_kinds.contains(&"emotion_state"));
    let customer_stage_dim = dimensions
        .iter()
        .find(|d| d["kind"] == "customer_stage")
        .expect("customer_stage dimension present");
    assert_eq!(customer_stage_dim["displayName"], "客户阶段");
    assert_eq!(customer_stage_dim["participatesInDecision"], true);

    // taxonomies.customer_stage[0] == {id:"first_contact", label:"初次接触"}。
    let cs_values = body["taxonomies"]["customer_stage"]
        .as_array()
        .expect("customer_stage taxonomy array");
    assert!(
        cs_values
            .iter()
            .any(|v| v["id"] == "first_contact" && v["label"] == "初次接触"),
        "customer_stage 取值字典应含 first_contact→初次接触，实际: {cs_values:?}"
    );

    // M3：taxonomies 必含 relationship_type 键（即便取值为空数组），验证 ∪ 逻辑——
    // relationship_type 不在 profile_dimensions 里但被强制并入 kind 集。
    assert!(
        body["taxonomies"]
            .as_object()
            .expect("taxonomies object")
            .contains_key("relationship_type"),
        "taxonomies 必须含 relationship_type 键（M3 kind 集 ∪ relationship_type）"
    );
    assert!(
        body["taxonomies"]["relationship_type"].is_array(),
        "relationship_type 取值应是数组（可空）"
    );

    // A5：taxonomies 必含 conversation_mode 键，验证 kind 集 ∪ conversation_mode——
    // 它不在 profile_dimensions 里但被强制并入 kind 集，供前端 labelFor 翻译对话模式。
    assert!(
        body["taxonomies"]
            .as_object()
            .expect("taxonomies object")
            .contains_key("conversation_mode"),
        "taxonomies 必须含 conversation_mode 键（A5 kind 集 ∪ conversation_mode）"
    );
    let cm_values = body["taxonomies"]["conversation_mode"]
        .as_array()
        .expect("conversation_mode 取值应是数组");
    // m028 seed 的四个默认值经 active-view 下发，consultative→顾问咨询 可被 labelFor 命中。
    assert!(
        cm_values
            .iter()
            .any(|v| v["id"] == "consultative" && v["label"] == "顾问咨询"),
        "conversation_mode 取值字典应含 consultative→顾问咨询（m028 seed），实际: {cm_values:?}"
    );
}
