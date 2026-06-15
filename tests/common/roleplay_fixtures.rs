//! roleplay-fuzz P0 公共测试夹具。
//!
//! 关联设计：`docs/superpowers/specs/2026-06-15-roleplay-fuzz-testing-design.md`
//! §5 / §12 P0 + P0 退出条件。
//!
//! 本模块**只新增** roleplay 专属、`real_llm_ops_smoke.rs` 中尚不存在的 helper：
//! - `seed_active_domain_profile`：在独立 workspace seed 一条 active DomainProfile，
//!   并强制失效进程级缓存（否则 30s TTL 窗口内 `load_active_domain_profile` 仍返回
//!   seed 前旧值——见设计 §5.1 缓存失效坑）。
//! - `override_review_prompt`：在 `ensure_prompt_pack_v2` 之后覆写 `user.review.system`
//!   等 reviewer prompt（必须在 `TestApp::start()` 之后调用，见设计 §5.3 时序坑）。
//! - `seed_verified_chunk`：seed 一条 verified / active 知识切片（知识链路 smoke）。
//! - `RoleplayLedger`：按设计 §11 写 `target/real_llm_ledger/roleplay_<fixture>.jsonl`，
//!   每行带 `suspected_layer` 字段。
//!
//! **不抽取** `real_llm_ops_smoke.rs` 的私有 helper（MCP mock / managed_contact /
//! make_inbound / run_judge）——P0 退出条件要求 t4-t18 零变化；等 P2 真正跑 E2E、
//! 出现第二个消费方时再考虑抽取。

#![allow(dead_code)]

use std::io::Write as _;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::agent::{default_domain_profile, invalidate_global_domain_profile_cache};
use wechatagent::models::{
    Contact, DomainProfile, OperationKnowledgeChunk, PromptTemplate, QuietHoursMode,
};
use wechatagent::prompts;

use crate::common::TestApp;

/// roleplay-fuzz 第一版唯一 fixture 的独立 workspace_id（与 TestApp 默认 workspace
/// 完全隔离，避免残留 DEFAULT active profile 污染——见设计 §5.1）。
pub const EMOTIONAL_COMPANION_WORKSPACE: &str = "test_emotional_companion";

/// 在指定 workspace seed 一条 **active** DomainProfile，并强制失效进程级缓存。
///
/// 以 `default_domain_profile(workspace_id)` 为基底（保证所有 H11/H15/H17 等字段都
/// 有合法默认值），由调用方传入的 `mutate` 闭包覆盖行业专属字段（如
/// `conversation_modes` / `grounding_gate_bypass_without_claim` / `operation_mode`）。
///
/// **单活保证**：先把同 workspace 其它 `is_active=true` 行降级，再插入本行（模拟
/// activate 端点语义），确保 `reload_from_db` 的 `find` 只命中一条——否则"后插入者
/// 赢"会让结果不确定（设计 §5.1）。
///
/// **缓存失效**：插入后立即 `invalidate_global_domain_profile_cache()`，使下一次
/// `load_active_domain_profile` 重读当前 DB（绕开 30s TTL）。
pub async fn seed_active_domain_profile<F>(
    app: &TestApp,
    workspace_id: &str,
    profile_id: &str,
    mutate: F,
) -> ObjectId
where
    F: FnOnce(&mut DomainProfile),
{
    let mut profile = default_domain_profile(workspace_id);
    profile.id = Some(ObjectId::new());
    profile.profile_id = profile_id.to_string();
    profile.is_active = true;
    profile.current_version = true;
    profile.seeded_by = Some("roleplay_fixture".to_string());
    profile.updated_at = DateTime::now();
    mutate(&mut profile);

    // 单活：降级同 workspace 其它 active 行。
    app.state
        .db
        .domain_profiles()
        .update_many(
            doc! { "workspace_id": workspace_id, "is_active": true },
            doc! { "$set": { "is_active": false, "updated_at": DateTime::now() } },
            None,
        )
        .await
        .expect("soft-demote other active profiles");

    let id = profile.id.expect("profile id");
    app.state
        .db
        .domain_profiles()
        .insert_one(&profile, None)
        .await
        .expect("insert active domain profile");

    // 强制失效进程级缓存，下次 load 立即见最新（绕开 30s TTL）。
    invalidate_global_domain_profile_cache();
    id
}

/// 情感陪伴最小 fixture：在 [`EMOTIONAL_COMPANION_WORKSPACE`] seed 一条 active
/// DomainProfile，覆盖设计 §5.2 要求的关键字段。
pub async fn seed_emotional_companion_profile(app: &TestApp) -> ObjectId {
    seed_active_domain_profile(
        app,
        EMOTIONAL_COMPANION_WORKSPACE,
        "emotional_companion_minimal",
        |p| {
            p.display_name = "情感陪伴".to_string();
            p.description = "长期陪伴、情绪承接、尊重边界，不做成交推进".to_string();
            // H9：允许亲密陪伴模式（不只销售四模式）。
            p.conversation_modes = vec![
                "intimate_companion".to_string(),
                "casual_relationship".to_string(),
                "value_exchange".to_string(),
                "boundary_protection".to_string(),
            ];
            // H14：纯情感回复不应因无产品知识被 grounding 软分硬闸误拦。
            p.grounding_gate_bypass_without_claim = true;
            // H8：关闭漏斗推进（陪伴不催进成交）。
            p.operation_mode.funnel.enabled = false;
            // H3：行业业务上下文。
            p.prompt_fragment = Some(
                "本行业目标是长期陪伴、情绪承接、尊重对方节奏与边界，不是成交推进。\
                 主动关心、轻量追问本身是正当行为，不等于施压。"
                    .to_string(),
            );
        },
    )
    .await
}

/// 把一个 contact 的作息门控关闭（情感陪伴夜间黄金时段不被 22→8 静默压制）。
///
/// 设计 §5.2 方案 A：override 挂在 **contact** 级 `operation_mode_override.quiet_hours
/// .enabled_override = Some(false)`，webhook 层与 gateway precheck 调同一个
/// `effective_quiet_hours_enabled(contact, ...)`，一次关掉两层静默门。
pub fn disable_quiet_hours_for_contact(contact: &mut Contact) {
    let mut mode = contact.operation_mode_override.clone().unwrap_or_default();
    mode.quiet_hours = QuietHoursMode {
        enabled_override: Some(false),
    };
    contact.operation_mode_override = Some(mode);
}

/// 覆写 reviewer prompt（仅当前测试 DB 生效）。
///
/// **时序红线**：必须在 `TestApp::start()`**之后**调用——`start()` 内部
/// `ensure_prompt_pack_v2` 会 `delete_many` 再 insert，提前覆写会被清掉（设计 §5.3）。
///
/// 机制：插入一条同 `(workspace_id, prompt_key)`、`status="active"`、`version` 更高
/// 的模板。`load_prompt` 按 `version desc` 取一条，故新版本胜出。
pub async fn override_review_prompt(
    app: &TestApp,
    workspace_id: &str,
    prompt_key: &str,
    content: &str,
) {
    let now = DateTime::now();
    let template = PromptTemplate {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        prompt_key: prompt_key.to_string(),
        agent_kind: "user".to_string(),
        layer: "review".to_string(),
        title: "roleplay reviewer rubric override".to_string(),
        description: Some("情感陪伴 reviewer rubric（仅测试 DB）".to_string()),
        content: content.to_string(),
        // 远高于 seed 默认版本，保证 load_prompt 的 version desc 命中本条。
        version: 9_999,
        prompt_pack_version: prompts::PROMPT_PACK_VERSION.to_string(),
        created_by: "roleplay_fixture".to_string(),
        created_at: now,
        updated_at: now,
        status: "active".to_string(),
        current_version: true,
        previous_version: None,
        seeded_by: Some("roleplay_fixture".to_string()),
        locale: Some(prompts::DEFAULT_LOCALE.to_string()),
    };
    app.state
        .db
        .prompt_templates()
        .insert_one(&template, None)
        .await
        .expect("insert review prompt override");
}

/// seed 一条 verified / active 知识切片（知识链路 smoke，设计 §5.5）。
///
/// `domain="user_operations"` / `status="active"` / `integrity_status="verified"` /
/// `account_id=None`（全局），与 `load_operation_knowledge` 的加载条件一致。
pub async fn seed_verified_chunk(
    app: &TestApp,
    workspace_id: &str,
    title: &str,
    summary: &str,
    body: &str,
) -> String {
    let id = ObjectId::new();
    let now = DateTime::now();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: workspace_id.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("boundary".to_string()),
        title: title.to_string(),
        summary: Some(summary.to_string()),
        body: Some(body.to_string()),
        source_quote: Some(body.to_string()),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(90),
        status: "active".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        dynamic_confidence: Some(0.9),
        chunk_type: "product_fact".to_string(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert verified chunk");
    id.to_hex()
}

/// roleplay ledger writer（设计 §11）。每行一个 JSON 对象，append 到
/// `${REAL_LLM_LEDGER:-target/real_llm_ledger}/roleplay_<fixture>.jsonl`。
///
/// IO 失败仅 eprintln 不 panic（与现有 `real_llm_knowledge_quality::ledger_append`
/// 同口径——台账缺失不应让测试崩）。
pub struct RoleplayLedger {
    path: std::path::PathBuf,
}

impl RoleplayLedger {
    /// 为某 fixture 打开 ledger。目录不存在则创建。
    pub fn for_fixture(fixture_id: &str) -> Self {
        let dir = std::env::var("REAL_LLM_LEDGER")
            .unwrap_or_else(|_| "target/real_llm_ledger".to_string());
        if let Err(error) = std::fs::create_dir_all(&dir) {
            eprintln!("[roleplay-ledger] create_dir_all({dir}) failed: {error}");
        }
        let path = std::path::Path::new(&dir).join(format!("roleplay_{fixture_id}.jsonl"));
        Self { path }
    }

    /// 追加一行 JSON。调用方负责保证 `row` 含设计 §3.2 要求的 `suspected_layer`
    /// 字段（当它代表一个 issue 时）。
    pub fn append(&self, row: serde_json::Value) {
        let line = match serde_json::to_string(&row) {
            Ok(s) => s,
            Err(error) => {
                eprintln!("[roleplay-ledger] serialize failed: {error}");
                return;
            }
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path);
        match file {
            Ok(mut f) => {
                if let Err(error) = writeln!(f, "{line}") {
                    eprintln!("[roleplay-ledger] write failed: {error}");
                }
            }
            Err(error) => {
                eprintln!("[roleplay-ledger] open({:?}) failed: {error}", self.path);
            }
        }
    }

    /// 便捷：写一个带 `suspected_layer` 的 issue 行。
    pub fn append_issue(&self, scene_id: &str, suspected_layer: &str, detail: serde_json::Value) {
        self.append(serde_json::json!({
            "kind": "issue",
            "scene_id": scene_id,
            "suspected_layer": suspected_layer,
            "detail": detail,
        }));
    }
}
