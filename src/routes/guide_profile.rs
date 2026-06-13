//! universal-domain-adaptation Phase 3（3A-4）：引导层「行业配置生成器」。
//!
//! 运营用自然语言描述业务（+ 已导入的行业文档）→ AI 生成一份**候选** `DomainProfile`
//! 草案。候选直接落 `domain_profiles`，状态 = `current_version=false` + `is_active=false`
//! （与 3A-3 `create` 同一草稿态）——**不阻塞运行时**（无 active 时回落 DEFAULT_PROFILE），
//! 也**不自动生效**。运营随后在审核 UI 逐项编辑（走 3A-3 `update`），确认后 `publish`
//! 定稿、`activate` 生效。
//!
//! **红线继承**：
//! - AI 生成的 profile = 候选，必须人审才能 activate（继承「AI 永不自动 verify」）。
//! - 生成器 system 引导语走 active profile 的 `methodology_generator_preamble`，DEFAULT
//!   回落**领域中性**的 `PLAYBOOK_METHODOLOGY_SYSTEM`（C3 已去销售偏见，不污染非销售行业）。
//! - LLM 只返结构化候选 JSON，不直接定稿/激活（patch-only 精神）。

use axum::{extract::State, Extension, Json};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::DomainProfile,
    prompts,
};

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GenerateProfileRequest {
    /// 运营对业务的自然语言描述（行业/产品/客户/经营目标/对话风格等）。
    business_description: String,
    /// 目标 profile slug（如 `dental-implant-private`）；落候选时作 `profile_id`。
    profile_id: String,
    /// 可选展示名；缺省用 profile_id。
    #[serde(default)]
    display_name: Option<String>,
}

/// 拉本 workspace 最近若干条已导入知识切片的标题，作为生成器的「行业文档线索」上下文。
/// 只取标题（不灌全文，控 token）；无文档时返回空串，生成器仅凭描述工作。
async fn gather_knowledge_titles(state: &AppState, workspace_id: &str) -> String {
    let coll = state
        .db
        .operation_knowledge_chunks()
        .clone_with_type::<Document>();
    let cursor = coll
        .find(
            doc! { "workspace_id": workspace_id },
            FindOptions::builder()
                .sort(doc! { "created_at": -1_i32 })
                .limit(40_i64)
                .projection(doc! { "title": 1_i32 })
                .build(),
        )
        .await;
    let mut titles: Vec<String> = Vec::new();
    if let Ok(cursor) = cursor {
        let raw = cursor.try_collect::<Vec<Document>>().await.unwrap_or_default();
        for d in raw {
            if let Ok(t) = d.get_str("title") {
                if !t.trim().is_empty() {
                    titles.push(format!("- {t}"));
                }
            }
        }
    }
    if titles.is_empty() {
        String::new()
    } else {
        format!("\n\n已导入的行业文档（标题，供你理解本行业术语/字段）：\n{}", titles.join("\n"))
    }
}

/// 构造引导层生成器的 user prompt：业务描述 + 文档线索 + 期望的 DomainProfile JSON 形态。
fn build_profile_generation_prompt(
    business_description: &str,
    profile_id: &str,
    display_name: &str,
    knowledge_context: &str,
) -> String {
    format!(
        r#"运营对自己业务的描述：
{business_description}
{knowledge_context}

请基于以上信息，生成一份本行业的画像配置候选（DomainProfile）。严格输出 JSON，字段：
{{
  "displayName": "{display_name}",
  "description": "本行业画像说明（人可读，一两句）",
  "profileDimensions": [
    {{"kind": "维度英文key(snake_case)", "displayName": "中文维度名", "participatesInDecision": true, "description": "该维度含义"}}
  ],
  "promptFragment": "本行业业务上下文片段（注入决策提示，解释这些维度怎么理解；不要写死与本行业无关的销售套路）",
  "conversationModes": ["按本行业需要选择的对话模式key，缺省四模式可不填"],
  "businessFormulas": [
    {{"key": "公式key(camelCase)", "expression": "可读展开式", "displayName": "中文名"}}
  ],
  "commitmentMarkers": {{"productEffect": ["本行业绝对化效果承诺词"], "toneOnly": ["语气类夸大词"]}},
  "coverageDimensions": [
    {{"key": "covKey", "displayName": "中文名", "required": false}}
  ]
}}

要求：
- profile_id 固定为 "{profile_id}"，不要在 JSON 里改它。
- 维度/取值/词表都用本行业真实语义，不要套用销售域（如「成交阶段」）除非本行业确实是销售。
- 不确定的字段给空数组/空串，不要编造。"#,
        business_description = business_description,
        knowledge_context = knowledge_context,
        display_name = display_name,
        profile_id = profile_id,
    )
}

pub(super) async fn generate_domain_profile_candidate(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<GenerateProfileRequest>,
) -> AppResult<Json<Value>> {
    if payload.business_description.trim().is_empty() {
        return Err(AppError::BadRequest("businessDescription 不能为空".to_string()));
    }
    if payload.profile_id.trim().is_empty() {
        return Err(AppError::BadRequest("profileId 不能为空".to_string()));
    }
    let workspace_id = admin.current_workspace.clone();
    let display_name = payload
        .display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| payload.profile_id.clone());

    // C3：生成器 system 走 active profile 的领域中性引导语（DEFAULT 已去销售偏见）。
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await;
    let system = match active_profile
        .methodology_generator_preamble
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(p) => p.to_string(),
        None => prompts::PLAYBOOK_METHODOLOGY_SYSTEM.to_string(),
    };

    let knowledge_context = gather_knowledge_titles(&state, &workspace_id).await;
    let user = build_profile_generation_prompt(
        &payload.business_description,
        &payload.profile_id,
        &display_name,
        &knowledge_context,
    );
    let generated = agent::generate_agent_json(
        &state,
        None,
        None,
        None,
        "guide.domain_profile.draft",
        &system,
        &user,
    )
    .await?;

    // 用生成的 JSON 反序列化成 DomainProfile,强制后端管理字段为草稿态。
    let mut doc: Document = mongodb::bson::to_document(&generated)
        .map_err(|e| AppError::External(format!("LLM 输出非对象: {e}")))?;
    doc.insert("profile_id", &payload.profile_id);
    doc.insert("workspace_id", &workspace_id);
    if !doc.contains_key("display_name") && !doc.contains_key("displayName") {
        doc.insert("display_name", &display_name);
    }
    let mut profile: DomainProfile = mongodb::bson::from_document(doc).map_err(|e| {
        AppError::External(format!("AI 生成的 profile 字段不合法,请重试或手填: {e}"))
    })?;
    let now = DateTime::now();
    profile.id = None;
    profile.profile_id = payload.profile_id.clone();
    profile.workspace_id = workspace_id.clone();
    profile.display_name = display_name;
    profile.version = next_candidate_version(&state, &workspace_id, &payload.profile_id).await?;
    profile.current_version = false; // 候选草稿:需人审 → publish → activate
    profile.previous_version = None;
    profile.is_active = false;
    profile.seeded_by = Some("generated_by_ai".to_string());
    profile.created_at = now;
    profile.updated_at = now;

    let inserted = state.db.domain_profiles().insert_one(&profile, None).await?;
    let hex = inserted
        .inserted_id
        .as_object_id()
        .map(|i| i.to_hex())
        .unwrap_or_default();
    Ok(Json(json!({
        "ok": true,
        "id": hex,
        "profileId": profile.profile_id,
        "status": "candidate",
        "note": "AI 生成的候选 profile 已落草稿(未生效)。请在审核 UI 逐项确认/编辑后 publish + activate。",
    })))
}

/// 候选版本号：同 (workspace, profile_id) 取 max(version)+1（与 3A-3 同口径）。
async fn next_candidate_version(
    state: &AppState,
    workspace_id: &str,
    profile_id: &str,
) -> AppResult<i32> {
    let raw = state.db.domain_profiles().clone_with_type::<Document>();
    let mut cursor = raw
        .find(
            doc! { "workspace_id": workspace_id, "profile_id": profile_id },
            FindOptions::builder()
                .sort(doc! { "version": -1_i32 })
                .limit(1_i64)
                .projection(doc! { "version": 1_i32 })
                .build(),
        )
        .await?;
    let max = if let Some(d) = cursor.try_next().await? {
        d.get_i32("version").unwrap_or(0)
    } else {
        0
    };
    Ok(max + 1)
}
