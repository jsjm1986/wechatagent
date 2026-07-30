//! 簇 B 标注质量门集成测试：缺口 6（target_stages 归一/校验）/ 缺口 3（审核审计）/
//! 缺口 2（客户级辅助模式 override + IDOR）。全部 `#[ignore]`，需 Docker testcontainers。
//! CI `integration` job 用 `cargo test --test annotation_quality_gate_integration -- --ignored` 跑。
//!
//! ## 测试形态说明（为何不走 brief 里的 HTTP/login 骨架）
//! 本仓 `TestApp`（`tests/common/mod.rs`）是 **state-only** 工厂：只建 testcontainers
//! Mongo + 同形 `AppState`，**没有** HTTP server，也没有 `login_admin` / multipart 上传
//! helper。既有集成测试（`tests/ask_human_phase1_e2e.rs`、`tests/domain_profile_e2e.rs`）
//! 一律 **直调 route handler 真函数**：handler 是普通 async fn，参数是 axum extractor
//! （`State` / `Extension` / `Path` / `Json`），构造好就 `.await`，错误以
//! `Err(AppError::BadRequest|NotFound)` 变体断言（直调时不经 axum→不产 HTTP 状态码）。
//! 本文件沿用该惯例。
//!
//! ## 各缺口可达性
//! - 缺口 3（`review_media_asset`）、缺口 2（`update_assist_override`）：handler 参数均
//!   可手工构造，直调验证。本 Task 把这两个 module 提为 `pub`、handler 提为 `pub`、请求
//!   体结构提为 `pub`（字段仍私有，测试用 `serde_json::from_value` 构造），仿
//!   `domain_profiles` / `ask_human_inbox` 先例。
//! - 缺口 6（target_stages 校验）：`upload_media_asset` handler 取 `Multipart` extractor，
//!   **tests crate 无法手工构造 `Multipart`**（见 `src/routes/mod.rs` 对 `import_*` 同款
//!   说明）。故缺口 6 直驱 handler 内部调用的归一函数
//!   `wechatagent::agent::normalize_target_stages`（本 Task 提为 `pub` 暴露），验证落库
//!   前的归一/校验语义——即 upload 端点 400/放行 行为的真实判定来源。
//!
//! ## 缺口 6 测试路径决策（brief「实现者决策点」）
//! brief 给两条路：①「字典越界 → Err」+「字典未配置 → 放行原值」；②若设施已有 taxonomy
//! 种子则改测「alias→canonical 真归一」。本仓 `TestApp::start()` **总是 re-seed m006**
//! （`customer_stage` 字典含 canonical id + 中文/英文 alias，如「需求挖掘」→`need_discovery`），
//! 所以「alias→canonical 真归一」**免费可达**，比 brief 默认路径更强，故采用：
//!   - `upload_alias_stage_normalizes_to_canonical`：「需求挖掘」→ `Ok(["need_discovery"])`（真归一）。
//!   - `upload_out_of_dict_stage_rejected`：「不存在的阶段名」→ `Err`（字典有条目但越界 → 400 来源）。
//! 「字典未配置 → 放行原值」一条**不在此处测**：它需要一个 `customer_stage` 字典为空的 DB，
//! 而归一走 **进程级全局 taxonomy 缓存**（30s TTL 单例，多测试共享）——若本测试把缓存
//! warm 到「空字典」状态，会污染并行跑的其它测试。该 fail-soft 分支已由 lib 纯函数单测
//! `dimension_registry::tests::classify_kind_unconfigured_accepts_machine` 确定性覆盖，
//! 不在共享缓存的集成层重复制造污染风险（YAGNI + 反过拟合）。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Json, Path, State};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::{AgentStatus, Contact, ContentAsset};
use wechatagent::routes::contacts::{update_assist_override, AssistOverrideRequest};
use wechatagent::routes::media_assets::{review_media_asset, ReviewRequest};
use wechatagent::routes::AppState;

// ── helpers ─────────────────────────────────────────────────────────────────

/// 构造测试 admin auth context（`current_workspace` 决定 handler 可见/可写范围）。
/// 仿 `tests/ask_human_phase1_e2e.rs::test_admin`。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "aqg_admin".to_string(),
        username: "aqg_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 构造一条 draft 媒体素材（缺口 3 用：审核它会落审计事件）。
/// `account_id=Some("default")` → 审计事件 `account_id` 为 "default"，便于回查。
fn make_draft_asset(workspace_id: &str, title: &str) -> ContentAsset {
    let now = DateTime::now();
    ContentAsset {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: Some("default".to_string()),
        kind: "media".to_string(),
        title: title.to_string(),
        body: None,
        tags: vec![],
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some("file".to_string()),
        file_path: Some("media/test/aqg.pdf".to_string()),
        file_name: Some("aqg.pdf".to_string()),
        file_size: Some(1024),
        mime_type: Some("application/pdf".to_string()),
        file_sha256: Some("aqgsha".to_string()),
        sendable: Some(true),
        send_trigger_hint: None,
        target_stages: None,
        expression_pref: None,
        requires_principal_approval: Some(false),
        review_status: Some("draft".to_string()),
        review_note: None,
        min_inject_tier: None,
        created_at: now,
        updated_at: now,
    }
}

/// 构造一条 managed 联系人（缺口 2 用：写 / 回查 domain_attributes.assist_mode_override）。
/// 仿 `tests/media_asset_send_integration.rs::make_managed_contact`。
fn make_managed_contact(workspace_id: &str, wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("AQG 测试客户".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: vec![],
        confirmed_tags: vec![],
        bayesian_signals: vec![],
        personality_profile: None,
        manual_tags_updated_at: None,
        manual_tags_by: None,
        tags_version: 0,
        domain_attributes: None,
        domain_attributes_updated_at: None,
        commitments: vec![],
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
        intent_trajectory: vec![],
        locale: None,
        outcome_events: vec![],
        created_at: now,
        updated_at: now,
    }
}

/// 构造 `AssistOverrideRequest`（字段私有但派生 `Deserialize`，用 from_value 构造，
/// 无需放开字段可见性——仿 `ext_knowledge` 请求体先例）。
fn assist_override_request(mode: &str) -> AssistOverrideRequest {
    serde_json::from_value(json!({
        "expectedAccountId": "default",
        "mode": mode
    }))
    .expect("build AssistOverrideRequest")
}

/// 构造 `ReviewRequest`（同上）。
fn review_request(status: &str, note: Option<&str>) -> ReviewRequest {
    serde_json::from_value(json!({
        "expectedScope": "account",
        "expectedAccountId": "default",
        "status": status,
        "note": note
    }))
    .expect("build ReviewRequest")
}

/// 回查某 contact 的 `domain_attributes.assist_mode_override` 字段（缺口 2 断言用）。
/// 返回 None 表示键不存在（default → $unset 后的预期）。
async fn read_assist_override(
    state: &AppState,
    workspace_id: &str,
    oid: ObjectId,
) -> Option<String> {
    let contact = state
        .db
        .contacts()
        .find_one(doc! { "_id": oid, "workspace_id": workspace_id }, None)
        .await
        .expect("query contact")
        .expect("contact exists");
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| {
            d.get_str(wechatagent::models::ASSIST_MODE_OVERRIDE_ATTR)
                .ok()
        })
        .map(ToString::to_string)
}

// ── 缺口 6：target_stages 归一/校验（直驱 normalize_target_stages）──────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn upload_alias_stage_normalizes_to_canonical() {
    // m006 已 seed：customer_stage「需求挖掘」是 need_discovery 的 alias。
    // AdminWrite 路径下 alias 应被归一为 canonical id，落库的是 canonical（与 contact
    // customer_stage 同空间，运行时 `s == cs` 才能命中、素材才发得出去）。
    let app = common::TestApp::start().await;
    let raw = vec!["需求挖掘".to_string()];
    let normalized = wechatagent::agent::normalize_target_stages(
        &app.state.db,
        &app.state.config.default_workspace_id,
        "",
        &raw,
    )
    .await
    .expect("alias 应被接受并归一，而非报错");
    assert_eq!(
        normalized,
        vec!["need_discovery".to_string()],
        "alias「需求挖掘」必须归一为 canonical need_discovery"
    );
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn upload_out_of_dict_stage_rejected() {
    // 字典已配置 ≥1 个 customer_stage 条目（m006 seed 了 9 个），填字典里不存在、也非
    // 任何 alias 的阶段名 → AdminWrite 越界 → Err（upload 端点据此返回 400）。
    let app = common::TestApp::start().await;
    let raw = vec!["不存在的阶段名".to_string()];
    let result = wechatagent::agent::normalize_target_stages(
        &app.state.db,
        &app.state.config.default_workspace_id,
        "",
        &raw,
    )
    .await;
    assert!(
        result.is_err(),
        "字典已配置时越界 stage 必须 Err（端点 400），实际: {result:?}"
    );
}

// ── 缺口 3：审核动作落审计事件 ───────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn review_media_asset_writes_audit_event() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    // seed 一条 draft 素材。
    let asset = make_draft_asset(ws, "AQG 审核素材");
    let asset_id = asset.id.expect("asset id");
    app.state
        .db
        .content_assets()
        .insert_one(&asset, None)
        .await
        .expect("insert draft asset");

    // 直调 review handler：设为 approved。
    let resp = review_media_asset(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(asset_id.to_hex()),
        Json(review_request("approved", Some("审核通过"))),
    )
    .await
    .expect("review handler 应成功");
    assert_eq!(
        resp.0.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "review 返回 ok:true"
    );

    // events 集合应落一条 kind=media_asset.reviewed、status=approved、details.reviewed_by=admin。
    let events: Vec<_> = app
        .state
        .db
        .events()
        .find(doc! { "kind": "media_asset.reviewed" }, None)
        .await
        .expect("query events")
        .try_collect()
        .await
        .expect("collect events");
    assert_eq!(events.len(), 1, "review 后应恰好落一条审计事件");
    let evt = &events[0];
    assert_eq!(evt.status, "approved", "审计事件 status 应为 approved");
    let details = evt.details.as_ref().expect("审计 details 应非空");
    assert_eq!(
        details.get_str("reviewed_by").unwrap_or(""),
        admin.username,
        "details.reviewed_by 应为审核管理员用户名"
    );
    assert_eq!(
        details.get_str("asset_id").unwrap_or(""),
        asset_id.to_hex(),
        "details.asset_id 应指向被审核的素材"
    );

    // 库内素材状态确已变为 approved（审计 + 业务副作用同时落地）。
    let updated = app
        .state
        .db
        .content_assets()
        .find_one(doc! { "_id": asset_id }, None)
        .await
        .expect("query asset")
        .expect("asset exists");
    assert_eq!(updated.review_status.as_deref(), Some("approved"));
}

// ── 缺口 2：客户级辅助模式 override + IDOR ──────────────────────────────────

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn assist_override_force_on_then_default_unsets() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    let contact = make_managed_contact(ws, "user_assist_override");
    let contact_id = contact.id.expect("contact id");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // force_on → domain_attributes.assist_mode_override = "force_on"
    let resp = update_assist_override(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(contact_id.to_hex()),
        Json(assist_override_request("force_on")),
    )
    .await
    .expect("force_on 应成功");
    assert_eq!(
        resp.0.get("mode").and_then(|v| v.as_str()),
        Some("force_on")
    );
    assert_eq!(
        read_assist_override(&app.state, ws, contact_id).await,
        Some("force_on".to_string()),
        "force_on 应写入 domain_attributes.assist_mode_override"
    );

    // default → 键被 $unset
    update_assist_override(
        State(app.state.clone()),
        Extension(admin.clone()),
        Path(contact_id.to_hex()),
        Json(assist_override_request("default")),
    )
    .await
    .expect("default 应成功")
    .0;
    assert_eq!(
        read_assist_override(&app.state, ws, contact_id).await,
        None,
        "default 应 $unset assist_mode_override 键"
    );
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn assist_override_invalid_mode_rejected() {
    let app = common::TestApp::start().await;
    let ws = "default";
    let admin = test_admin(ws);

    let contact = make_managed_contact(ws, "user_assist_bogus");
    let contact_id = contact.id.expect("contact id");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let result = update_assist_override(
        State(app.state.clone()),
        Extension(admin),
        Path(contact_id.to_hex()),
        Json(assist_override_request("bogus")),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::BadRequest(_))),
        "闭集外 mode 必须 BadRequest（400），实际: {result:?}"
    );
}

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn assist_override_cross_workspace_404() {
    // IDOR 守卫：contact 落在 other_ws，admin 在 default → find_contact_by_id 锁 workspace
    // → NotFound（不泄漏存在性、不可跨 workspace 写）。
    let app = common::TestApp::start().await;
    let admin = test_admin("default");

    let foreign = make_managed_contact("other_ws", "user_foreign");
    let foreign_id = foreign.id.expect("contact id");
    app.state
        .db
        .contacts()
        .insert_one(&foreign, None)
        .await
        .expect("insert foreign-workspace contact");

    let result = update_assist_override(
        State(app.state.clone()),
        Extension(admin),
        Path(foreign_id.to_hex()),
        Json(assist_override_request("force_on")),
    )
    .await;
    assert!(
        matches!(result, Err(AppError::NotFound(_))),
        "跨 workspace contact 必须 NotFound（404，IDOR 守卫），实际: {result:?}"
    );

    // 副作用断言：foreign contact 的 domain_attributes 未被写入（写穿防护真生效）。
    assert_eq!(
        read_assist_override(&app.state, "other_ws", foreign_id).await,
        None,
        "跨 workspace 写必须不落地"
    );
}
