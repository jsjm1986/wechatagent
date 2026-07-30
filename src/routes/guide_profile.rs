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

/// 递归地将 camelCase 字符串转换为 snake_case。
/// `displayName` → `display_name`, `profileDimensions` → `profile_dimensions`
///
/// **已知限制**:本实现仅在「当前字符大写 ∧ 前一字符是字母/数字 ∧ 后一字符是小写字母」
/// 时插下划线,因此对**末尾连续大写**(如 `v2API` → `v2api` 而非 `v2_api`、
/// `profileID` → `profileid` 而非 `profile_id`)不插下划线。连续大写后跟小写的场景
/// (如 `HTTPServer` → `http_server`)能正确处理。当前仅用于归一化 LLM 输出的典型
/// camelCase key(`displayName` / `profileDimensions` / `profileId` 等),不触发此限制。
/// 若未来 LLM 输出末尾缩略词 key,需替换为支持末尾连续大写的实现(例如引入 `heck`)。
/// 已知限制由 `tests::to_snake_case_known_limitation_trailing_uppercase` 锁定。
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().copied().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            // 检查是否需要在前面插入 _：前一个字符是字母/数字，后一个字符是字母
            let prev_is_letter_or_digit = chars[i - 1].is_alphanumeric();
            let next_is_lower = chars
                .get(i + 1)
                .is_some_and(|next| next.is_ascii_lowercase());
            if prev_is_letter_or_digit && next_is_lower {
                result.push('_');
            }
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

/// 递归归一化：处理所有层级的 camelCase keys → snake_case。
fn normalize_json_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let normalized: serde_json::Map<String, Value> = map
                .into_iter()
                .map(|(k, v)| {
                    let new_key = to_snake_case(&k);
                    (new_key, normalize_json_keys(v))
                })
                .collect();
            Value::Object(normalized)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(normalize_json_keys).collect()),
        other => other,
    }
}

/// 把 LLM 偶发"该是字符串却给成对象/数组"的标量字段压平成文本——`DomainProfile` 的
/// `description` / `prompt_fragment` 是 `String` / `Option<String>`，但生成 prompt 把
/// `promptFragment` 描述成"一段真实的 AI 决策思考"，claude 在复杂域（情感陪伴/教培）
/// 偶发把它组织成 `{"客户处境":"…","我的思考":"…"}` 这类对象 → `from_document`
/// 反序列化到 `String` 直接 `invalid type: map, expected a string`（CI run 27678306055
/// 实测）。这里在 snake_case 归一后、转 Document 前，对**顶层已知标量字段**做类型矫正：
/// 值是对象/数组则序列化成紧凑 JSON 文本（**内容不丢**，符合"要的是它的内容"），
/// 是字符串/null 原样。
///
/// G32：嵌套 `profileDimensions[].description`（`ProfileDimension.description` 是 `String`
/// 非 `Option`，models.rs）同样可能被 LLM 给成对象/数组 → `from_document` 失败。因此除顶层
/// 标量字段外，再对 `profile_dimensions`（normalize 后是 snake key）数组每个元素的
/// `description` 做同样压平。其余数组/嵌套结构不动。
fn coerce_scalar_string_fields(value: Value) -> Value {
    const SCALAR_STRING_KEYS: &[&str] = &[
        "description",
        "prompt_fragment",
        "soul_override",
        "methodology_override",
        "conversation_mode_policy",
    ];
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in SCALAR_STRING_KEYS {
        if let Some(v) = map.get_mut(*key) {
            if v.is_object() || v.is_array() {
                // 对象/数组 → 紧凑 JSON 文本；序列化失败（不应发生）则退成空串占位。
                let text = serde_json::to_string(v).unwrap_or_default();
                *v = Value::String(text);
            }
        }
    }
    // G32: profileDimensions[].description 是 String(models.rs ProfileDimension),LLM 偶发给对象 → 压平。
    if let Some(Value::Array(dims)) = map.get_mut("profile_dimensions") {
        for dim in dims.iter_mut() {
            if let Value::Object(dim_map) = dim {
                if let Some(d) = dim_map.get_mut("description") {
                    if d.is_object() || d.is_array() {
                        *d = Value::String(serde_json::to_string(d).unwrap_or_default());
                    }
                }
            }
        }
    }
    Value::Object(map)
}

/// 流 C：从 LLM 生成的 JSON 中提取每个维度的 `suggestedValues`（AI 建议的取值集），
/// **同时把 `suggestedValues` 键从各维度对象里 remove**——仿 `stateMachine` 的 pre-normalize
/// 抽取法（`ProfileDimension` 只有 kind/display_name/participates_in_decision/description 四
/// 字段，无 suggestedValues；显式 remove 避免污染 `from_document` 反序列化，与 stateMachine
/// 一致更稳）。
///
/// **必须在 `normalize_json_keys` 之前调用**：此时维度键仍是 camelCase（`kind` /
/// `suggestedValues` / `id` / `label`），直接读 `dim["kind"]`、`dim["suggestedValues"]`。
///
/// 返回 `Vec<(kind, Vec<(id, label)>)>`：每个维度的英文 kind + 其建议取值的 (id, label) 对。
/// 缺 `suggestedValues`、非数组、或 id/label 缺失/空 → 该维度贡献空 vec / 跳过该取值（软化）。
fn extract_suggested_values(generated: &mut Value) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let Some(dims) = generated
        .get_mut("profileDimensions")
        .and_then(Value::as_array_mut)
    else {
        return out;
    };
    for dim in dims.iter_mut() {
        let Some(dim_obj) = dim.as_object_mut() else {
            continue;
        };
        let kind = dim_obj
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());
        // 无论 kind 是否有效都 remove suggestedValues（避免残留污染反序列化）。
        let raw_values = dim_obj.remove("suggestedValues");
        let Some(kind) = kind else { continue };
        let mut values: Vec<(String, String)> = Vec::new();
        if let Some(Value::Array(arr)) = raw_values {
            for v in arr {
                let id = v
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let label = v
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if let (Some(id), Some(label)) = (id, label) {
                    values.push((id.to_string(), label.to_string()));
                }
            }
        }
        out.push((kind, values));
    }
    out
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateProfileRequest {
    /// 运营对业务的自然语言描述（行业/产品/客户/经营目标/对话风格等）。
    pub business_description: String,
    /// 目标 profile slug（如 `dental-implant-private`）；落候选时作 `profile_id`。
    pub profile_id: String,
    /// 可选展示名；缺省用 profile_id。
    #[serde(default)]
    pub display_name: Option<String>,
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
        let raw = cursor
            .try_collect::<Vec<Document>>()
            .await
            .unwrap_or_default();
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
        format!(
            "\n\n已导入的行业文档（标题，供你理解本行业术语/字段）：\n{}",
            titles.join("\n")
        )
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
        r#"你好，我需要你帮我理解我们这个行业。

我先说说我们是什么样的：
{business_description}
{knowledge_context}

---

请帮我生成一份「行业画像配置」——这份配置会被 AI 用来理解客户、判断该怎么回应。

它不是写给我自己看的，而是写给 AI 看的。所以要回答这些问题：

**我服务的客户是什么样的人？**（不是人口统计，是他们的处境、痛点、期待）
**我们和客户对话时，什么是真正重要的？**（那些让客户觉得"你懂我"的时刻）
**有没有哪些话说出来会让我失去客户的信任？**（比如夸大效果、用错语境）
**客户来了之后，通常会经历怎样的心理过程？**（从陌生到信任，中间有关键节点）
**用什么方式说话客户会觉得舒服？**（语气、用词风格、边界感）

请严格输出 JSON，结构如下：
{{
  "displayName": "{display_name}",
  "description": "一两句话描述这个行业的 AI 对话画像",
  "profileDimensions": [
    {{
      "kind": "维度英文key(snake_case)",
      "displayName": "中文维度名",
      "participatesInDecision": true,
      "description": "这个维度如何影响 AI 的判断（写给 AI 看的，不是写给人看的）",
      "suggestedValues": [
        {{"id": "取值英文id(snake_case)", "label": "中文取值名"}}
      ]
    }}
  ],
  "promptFragment": "一段真实的 AI 决策提示片段——如果你是 AI，面对一个客户，你会怎么想这些问题。要有行业灵魂，不要空洞。",
  "soulOverride": "本行业的 AI 人格本体——它是谁、面对客户的根本姿态。会整体替换默认人格。非销售行业建议给，纯销售可留空。",
  "methodologyOverride": "本行业的运营方法论——客户会经历哪些阶段、每阶段怎么推进（这里写清各阶段的取值语义与推进规则）。会整体替换默认方法论。",
  "conversationModePolicy": "本行业的对话模式判定规则——什么情况进入哪种对话模式（对应 conversationModes）。会整体替换默认判定段。",
  "conversationModes": ["这个行业真正需要的对话模式，不是填四个标准模式"],
  "businessFormulas": [
    {{"key": "公式key(camelCase)", "expression": "客户视角的可读展开式", "displayName": "中文名"}}
  ],
  "commitmentMarkers": {{
    "productEffect": ["这类话一说出来客户就会失去信任（绝对化效果承诺）"],
    "toneOnly": ["这类话只有语气上的分量，没有实质承诺"]
  }},
  "coverageDimensions": [
    {{"key": "covKey", "displayName": "中文名", "required": false}}
  ],
  "stateMachine": {{
    "states": [
      {{
        "key": "状态英文id(snake_case)",
        "name": "中文状态名（客户此刻所处的阶段）",
        "goal": "AI 在这个阶段最该达成的事",
        "advanceSignals": ["客户说/做了什么，意味着可以往下走"],
        "riskRules": ["这个阶段绝不能踩的雷（会吓退客户的话/动作）"],
        "initial": true,
        "allowedFrom": ["允许从哪些状态迁入本状态（含自身则填本状态key）"],
        "forbidsProactive": false
      }}
    ]
  }}
}}

关于 stateMachine，我想说的是：每个行业的客户都会经历一段心理旅程——从陌生、试探，到慢慢信任、最后做决定。你最懂你的客户会经历哪些阶段。请把这段旅程拆成几个「状态」，每个状态写清楚：客户此刻在想什么（name/goal）、什么信号说明他准备好进入下一步（advanceSignals）、这个阶段哪些话千万不能说（riskRules）。`initial: true` 标的是客户刚找上门时所处的第一个状态（至少要有一个状态标 true）；`allowedFrom` 写这个状态可以从哪些状态走过来。

**重要提醒：**
- profile_id 是唯一标识，固定为「{profile_id}」，不要改动。
- 如果某个**公式/承诺词/覆盖维度**在你的行业里没有对应的，不要硬凑——给空数组或空串就好。
- promptFragment 要写得像一段真实思考，不是产品说明书。
- **字段类型严格**：`promptFragment`、`description` 必须是**单个纯文本字符串**（哪怕内容很长、包含多段思考，也要写在一个字符串里，用换行分隔），**不要写成 `{{...}}` 对象或数组**。
- `profileDimensions` **必须**给出至少 3 个维度（这是配置的核心，不能为空数组）；每个维度的 `kind` 和 `displayName` 都必须是非空字符串。
- 每个维度尽量给 3-8 个该行业典型取值（suggestedValues）：`id` 用 snake_case 英文 canonical、`label` 用中文行业术语。这些是「建议候选」，运营审核采纳后才生效，不必穷尽。
- `stateMachine` 是**可选**的——如果你的业务没有清晰的分阶段旅程（客户来了就一锤子买卖、或纯随性陪伴聊天），就省略它或给空的 `states` 数组，AI 运行时会自动回落到一套通用默认阶段。
- soulOverride / methodologyOverride / conversationModePolicy 是把本行业世界观「整段」写清楚——客户阶段（customer_stage 等）的取值语义、推进规则都写在这里（不要用销售词如「成交/逼单/续费」，除非你就是销售行业）。这三段决定 typed 维度对本行业的真实含义。留空则回落销售域默认。"#,
        business_description = business_description,
        knowledge_context = knowledge_context,
        display_name = display_name,
        profile_id = profile_id,
    )
}

pub async fn generate_domain_profile_candidate(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<GenerateProfileRequest>,
) -> AppResult<Json<Value>> {
    if payload.business_description.trim().is_empty() {
        return Err(AppError::BadRequest(
            "businessDescription 不能为空".to_string(),
        ));
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
        agent::domain_profile::load_active_domain_profile(&state.db, &workspace_id).await?;
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
        &workspace_id,
        None,
        None,
        None,
        "guide.domain_profile.draft",
        &system,
        &user,
    )
    .await?;

    // H13：把 stateMachine 从顶层抽出，**绕过 normalize_json_keys**。状态机本体由运行时引擎
    // （guards.rs / migrations）和 prompts.rs 的 DEFAULT 种子消费，内部 key 全是 camelCase
    // （`states` / `key` / `initial` / `allowedFrom` / `allowFromAny` / `forbidsProactive` /
    // `advanceSignals` / `riskRules` / `goal` / `name`）。若让它过 normalize_json_keys，
    // `allowedFrom`→`allowed_from`、`allowFromAny`→`allow_from_any`，引擎 `get_array("allowedFrom")`
    // / `get_bool("allowFromAny")` 会静默读不到。LLM 按 prompt 输出 camelCase 顶层 key，
    // 所以这里直接 remove("stateMachine")（DomainProfile struct 无对应 serde 字段，留着也会被
    // from_document 丢弃，显式抽出更干净且能单独走 validate）。
    let mut generated = generated;
    let raw_state_machine = generated
        .as_object_mut()
        .and_then(|m| m.remove("stateMachine"));

    // 流 C：在 normalize 之前提取每个维度的 suggestedValues（AI 建议取值），同时从
    // 维度对象里 remove（避免污染 ProfileDimension 反序列化，与 stateMachine 同法）。
    let suggested_values = extract_suggested_values(&mut generated);

    let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
    let mut doc: Document = mongodb::bson::to_document(&normalized)
        .map_err(|e| AppError::External(format!("LLM 输出非对象: {e}")))?;
    doc.insert("profile_id", &payload.profile_id);
    doc.insert("workspace_id", &workspace_id);
    doc.insert("display_name", &display_name);
    // is_active / current_version / created_at / updated_at 在 struct 层面强制覆盖，
    // 不依赖 LLM 输出（它们无 #[serde(default)]，必须存在才能反序列化）。
    let now = DateTime::now();
    doc.insert("is_active", false);
    doc.insert("current_version", false);
    doc.insert("created_at", now);
    doc.insert("updated_at", now);
    let mut profile: DomainProfile = mongodb::bson::from_document(doc).map_err(|e| {
        AppError::External(format!("AI 生成的 profile 字段不合法,请重试或手填: {e}"))
    })?;
    profile.id = None;
    profile.profile_id = payload.profile_id.clone();
    profile.workspace_id = workspace_id.clone();
    profile.display_name = display_name;
    profile.version = 0;
    profile.current_version = false; // 候选草稿:需人审 → publish → activate
    profile.previous_version = None;
    profile.release_status = "draft".to_string();
    profile.is_active = false;
    profile.seeded_by = Some("generated_by_ai".to_string());
    profile.created_at = now;
    profile.updated_at = now;

    // H13：把抽出的 camelCase stateMachine 转 Document（**不 snake_case**，保留 camelCase 内层
    // key 供引擎/validate 读）→ validate。Ok 落 draft；Err 回落 None + warn（状态机校验不过
    // 不阻断 profile 生成，运行时缺 active 状态机自动回落 DEFAULT）；LLM 没产出或给空对象 →
    // None（debug 级，非异常）。
    profile.generated_state_machine = match raw_state_machine {
        Some(sm) if sm.is_object() => match mongodb::bson::to_document(&sm) {
            Ok(sm_doc) => match crate::routes::domains::validate_state_machine(&sm_doc) {
                Ok(()) => Some(sm_doc),
                Err(e) => {
                    tracing::warn!(
                        profile_id = %payload.profile_id,
                        error = %e,
                        "AI 生成的 stateMachine 未过校验，候选不落状态机（运行时回落 DEFAULT）"
                    );
                    None
                }
            },
            Err(e) => {
                tracing::warn!(
                    profile_id = %payload.profile_id,
                    error = %e,
                    "AI 生成的 stateMachine 转 Document 失败，候选不落状态机"
                );
                None
            }
        },
        _ => {
            tracing::debug!(
                profile_id = %payload.profile_id,
                "AI 未产出 stateMachine，候选 generated_state_machine = None"
            );
            None
        }
    };

    let profile = super::domain_profiles::append_domain_profile_draft(&state.db, profile).await?;
    let hex = profile.id.map(|i| i.to_hex()).unwrap_or_default();

    // 流 C：AI 建议的维度取值落候选层（绝不直接进 system_taxonomies——守「AI 永不自动
    // verify」红线），复用运行时同一候选 → admin approve 通路。confidence 传 10（即
    // upsert_candidate 内 clamp(0,10) 的上界，表「确定性生成的建议」；运行时 0/50 同样被
    // 钳进 [0,10]，本字段只表强度档位、不区分来源）。scope="global"：与 taxonomy global
    // seed 同 scope，前端 active-view scope 回落 global 可达。
    // 失败软化（let _）：候选落库失败不阻断 profile 生成（profile 已落库）。
    for (kind, values) in &suggested_values {
        for (id, label) in values {
            let _ = agent::taxonomy::upsert_candidate(
                &state.db,
                &admin.current_workspace,
                "global",
                kind,
                id,
                None,
                10,
                Some(label.as_str()),
            )
            .await;
        }
    }

    Ok(Json(json!({
        "ok": true,
        "id": hex,
        "profileId": profile.profile_id,
        "status": "candidate",
        "note": "AI 生成的候选 profile 已落草稿(未生效)。请在审核 UI 逐项确认/编辑后 publish + activate。",
    })))
}

#[cfg(test)]
mod tests {
    use super::{
        coerce_scalar_string_fields, extract_suggested_values, normalize_json_keys, to_snake_case,
    };
    use serde_json::json;

    /// 生产输入的正确性基线:LLM 实际输出的典型 camelCase key 必须正确归一化。
    /// 若此测失败,说明 to_snake_case 退化,会直接污染 domain_profile 候选落库。
    #[test]
    fn to_snake_case_typical_camelcase() {
        assert_eq!(to_snake_case("displayName"), "display_name");
        assert_eq!(to_snake_case("profileDimensions"), "profile_dimensions");
        assert_eq!(to_snake_case("profileId"), "profile_id");
        assert_eq!(to_snake_case("commitmentMarkers"), "commitment_markers");
        assert_eq!(to_snake_case("coverageDimensions"), "coverage_dimensions");
        assert_eq!(to_snake_case("businessFormulas"), "business_formulas");
    }

    /// 连续大写后跟小写:当前实现能正确识别缩略词边界。
    /// 锁定此正向行为,防止回归。
    #[test]
    fn to_snake_case_consecutive_uppercase_followed_by_lower() {
        // HTTPServer → http_server(S 后跟 e,在 S 前插下划线)。
        assert_eq!(to_snake_case("HTTPServer"), "http_server");
        // APIKey → api_key(K 后跟 e,在 K 前插下划线)。
        assert_eq!(to_snake_case("APIKey"), "api_key");
    }

    #[test]
    fn to_snake_case_handles_non_ascii_prefix_without_byte_indexing() {
        assert_eq!(to_snake_case("客户Stage"), "客户_stage");
        assert_eq!(to_snake_case("éValue"), "é_value");
    }

    /// 已知限制锁定:末尾连续大写不插下划线(后面没有小写字母触发分隔)。
    /// 这是 to_snake_case 当前实现的**预期行为**,不是 bug —— 改实现时必须同步
    /// 更新本断言。
    #[test]
    fn to_snake_case_known_limitation_trailing_uppercase() {
        // v2API → v2api(末尾 API 后无小写,不分隔)。
        assert_eq!(to_snake_case("v2API"), "v2api");
        // profileID → profileid(末尾 ID 后无小写,被压平)。
        assert_eq!(to_snake_case("profileID"), "profileid");
    }

    /// normalize_json_keys 必须递归处理嵌套 object 与 array,把所有层级的
    /// camelCase key 归一化为 snake_case。
    #[test]
    fn normalize_json_keys_recurses_object_and_array() {
        let input = json!({
            "displayName": "教培",
            "profileDimensions": [
                { "kind": "stage", "displayName": "学段" }
            ],
            "nested": { "coverageDimensions": [] }
        });
        let out = normalize_json_keys(input);
        assert_eq!(out["display_name"], json!("教培"));
        assert_eq!(out["profile_dimensions"][0]["kind"], json!("stage"));
        assert_eq!(out["profile_dimensions"][0]["display_name"], json!("学段"));
        assert!(out["nested"]["coverage_dimensions"].is_array());
        // 原始 camelCase key 不应残留。
        assert!(out.get("displayName").is_none());
        assert!(out["nested"].get("coverageDimensions").is_none());
    }

    #[test]
    fn coerce_flattens_object_prompt_fragment_to_text() {
        // claude 把 prompt_fragment 写成对象（复杂域常见）→ 压平成 JSON 文本，内容不丢。
        let input = json!({
            "description": "正常字符串",
            "prompt_fragment": { "客户处境": "焦虑", "我的思考": "先倾听" }
        });
        let out = coerce_scalar_string_fields(input);
        assert!(out["prompt_fragment"].is_string(), "对象应被压平成字符串");
        let s = out["prompt_fragment"].as_str().unwrap();
        assert!(
            s.contains("客户处境") && s.contains("先倾听"),
            "内容须保留: {s}"
        );
        // 本就是字符串的 description 原样不动。
        assert_eq!(out["description"], json!("正常字符串"));
    }

    #[test]
    fn coerce_leaves_valid_strings_and_other_fields_untouched() {
        // 字符串/数组结构字段（profile_dimensions）不受影响——只矫正已知标量字段的对象/数组值。
        let input = json!({
            "description": "一句话描述",
            "prompt_fragment": "一段纯文本思考",
            "profile_dimensions": [{ "kind": "stage", "display_name": "学段" }]
        });
        let out = coerce_scalar_string_fields(input.clone());
        assert_eq!(out, input, "全合法输入应原样返回");
    }

    #[test]
    fn coerce_flattens_array_description() {
        // description 偶发被写成数组（分点）→ 压平成 JSON 文本而非反序列化失败。
        let input = json!({ "description": ["点1", "点2"] });
        let out = coerce_scalar_string_fields(input);
        assert!(out["description"].is_string());
        assert!(out["description"].as_str().unwrap().contains("点1"));
    }

    /// 流 C：`extract_suggested_values` 从含 suggestedValues 的维度提取 (id,label) 对，
    /// 并把 suggestedValues 键从维度对象 remove（避免污染 ProfileDimension 反序列化）。
    #[test]
    fn extract_suggested_values_collects_pairs_and_removes_key() {
        let mut generated = json!({
            "profileDimensions": [
                {
                    "kind": "customer_stage",
                    "displayName": "客户阶段",
                    "suggestedValues": [
                        {"id": "first_contact", "label": "初次接触"},
                        {"id": "qualified", "label": "已确认意向"}
                    ]
                },
                {
                    "kind": "intent_level",
                    "displayName": "意向强度",
                    "suggestedValues": [{"id": "hot", "label": "高意向"}]
                }
            ]
        });
        let out = extract_suggested_values(&mut generated);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "customer_stage");
        assert_eq!(
            out[0].1,
            vec![
                ("first_contact".to_string(), "初次接触".to_string()),
                ("qualified".to_string(), "已确认意向".to_string()),
            ]
        );
        assert_eq!(out[1].0, "intent_level");
        assert_eq!(out[1].1, vec![("hot".to_string(), "高意向".to_string())]);
        // suggestedValues 键必须已从每个维度对象 remove（否则 normalize 后污染反序列化）。
        let dims = generated["profileDimensions"].as_array().unwrap();
        for dim in dims {
            assert!(
                dim.get("suggestedValues").is_none(),
                "suggestedValues 须被 remove"
            );
        }
    }

    /// 流 C：缺 suggestedValues 的维度 → 提取得空 vec、不 panic、维度本身仍正常
    /// （`from_document` 后 profile_dimensions 不含 suggestedValues 键）。
    #[test]
    fn extract_suggested_values_missing_is_empty_and_dim_survives() {
        let mut generated = json!({
            "profileDimensions": [
                {"kind": "trust", "displayName": "信任", "description": "x"}
            ]
        });
        let out = extract_suggested_values(&mut generated);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "trust");
        assert!(out[0].1.is_empty(), "缺 suggestedValues → 空 vec");

        // 提取后维度经 normalize → from_document 仍正常，不含 suggestedValues 键。
        let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
        let doc = to_profile_doc(normalized);
        let profile: crate::models::DomainProfile =
            mongodb::bson::from_document(doc).expect("缺 suggestedValues 应能反序列化");
        assert_eq!(profile.profile_dimensions[0].kind, "trust");
    }

    /// 流 C：完全没有 profileDimensions / 非数组 → 提取得空 vec（软化，不 panic）。
    #[test]
    fn extract_suggested_values_no_dimensions_returns_empty() {
        let mut none = json!({ "displayName": "x" });
        assert!(extract_suggested_values(&mut none).is_empty());
        let mut not_array = json!({ "profileDimensions": "oops" });
        assert!(extract_suggested_values(&mut not_array).is_empty());
    }

    /// 流 C：取值缺 id 或 label / 空串 → 跳过该取值（软化），有效的保留。
    #[test]
    fn extract_suggested_values_skips_incomplete_values() {
        let mut generated = json!({
            "profileDimensions": [{
                "kind": "stage",
                "suggestedValues": [
                    {"id": "ok", "label": "有效"},
                    {"id": "", "label": "空id"},
                    {"id": "no_label"},
                    {"label": "无id"},
                    {"id": "blank_label", "label": "  "}
                ]
            }]
        });
        let out = extract_suggested_values(&mut generated);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, vec![("ok".to_string(), "有效".to_string())]);
    }

    /// 把 brief 给的「裁剪版」LLM 输出补齐成 `from_document` 可反序列化的完整 doc：注入
    /// `DomainProfile` 那几个无 `#[serde(default)]` 的必填字段（profile_id / workspace_id /
    /// is_active / created_at / updated_at），复刻生产 `generate_domain_profile_candidate`
    /// 在 from_document 前的注入步骤。这样测试若失败，只会因被测的 G08/G32 缺陷，而不是
    /// 无关的「缺必填字段」错误。
    fn to_profile_doc(normalized: serde_json::Value) -> mongodb::bson::Document {
        let mut doc = mongodb::bson::to_document(&normalized).expect("normalized → Document");
        doc.insert("profile_id", "test-profile");
        doc.insert("workspace_id", "test-ws");
        if !doc.contains_key("display_name") {
            doc.insert("display_name", "测试画像");
        }
        doc.insert("is_active", false);
        let now = mongodb::bson::DateTime::now();
        doc.insert("created_at", now);
        doc.insert("updated_at", now);
        doc
    }

    /// G08：`businessFormulas[].displayName` 经 `normalize_json_keys` 被 snake 化成
    /// `display_name`，因 `BusinessFormula` 是 `rename_all="camelCase"`，老代码只认 wire key
    /// `displayName` → 反序列化匹配不上 → 落 `#[serde(default)]` 空串（静默 data-loss）。
    /// 加 `#[serde(default, alias = "display_name")]` 后，snake 化的 key 也被接受 → 值保留。
    #[test]
    fn business_formula_display_name_survives_normalize() {
        let generated = json!({
            "profileDimensions": [{"kind":"trust","displayName":"信任","description":"x"}],
            "businessFormulas": [{"key":"trust","expression":"A×B","displayName":"信任度"}]
        });
        let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
        let doc = to_profile_doc(normalized);
        let profile: crate::models::DomainProfile =
            mongodb::bson::from_document(doc).expect("补齐必填字段后应能反序列化");
        assert_eq!(
            profile.business_formulas[0].display_name, "信任度",
            "displayName 经 normalize→snake 后不应丢(G08)"
        );
    }

    /// G32：嵌套 `profileDimensions[].description`（`ProfileDimension.description` 是 `String`）
    /// 若 LLM 偶发给成对象/数组，`coerce_scalar_string_fields` 老实现只护顶层标量字段 →
    /// 嵌套对象原样进 doc → `from_document` 到 `String` 报 `invalid type: map`。扩 coerce 到
    /// 嵌套后应被压平成 JSON 文本，反序列化成功且内容非空。
    #[test]
    fn profile_dimension_description_object_coerced() {
        let generated = json!({
            "profileDimensions": [{"kind":"stage","displayName":"阶段","description":{"a":"b"}}]
        });
        let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
        let doc = to_profile_doc(normalized);
        let profile: crate::models::DomainProfile =
            mongodb::bson::from_document(doc).expect("嵌套 description 对象应被 coerce 压平(G32)");
        assert!(
            !profile.profile_dimensions[0].description.is_empty(),
            "压平后的 description 须保留内容"
        );
    }

    /// Task9：AI 生成的三段 override（soulOverride / methodologyOverride /
    /// conversationModePolicy）经 normalize_json_keys（camelCase→snake_case）+ coerce 后，
    /// 应落到 DomainProfile 的 soul_override / methodology_override / conversation_mode_policy
    /// （`Option<String>`，无 rename），值原样保留。
    #[test]
    fn generate_parses_overrides_when_present() {
        let generated = json!({
            "profileDimensions": [{"kind":"trust","displayName":"信任","description":"x"}],
            "soulOverride": "我是教培行业的陪伴式顾问",
            "methodologyOverride": "客户经历：试听→评估→报名，各阶段如下…",
            "conversationModePolicy": "## 对话模式判定\n用户表达情绪→empathetic_support"
        });
        let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
        // normalize 后键应为 snake_case，值保留。
        assert_eq!(
            normalized["soul_override"],
            json!("我是教培行业的陪伴式顾问")
        );
        assert!(
            normalized.get("soulOverride").is_none(),
            "camelCase 键不应残留"
        );
        // from_document 落 DomainProfile，三 Option 字段为 Some。
        let doc = to_profile_doc(normalized);
        let profile: crate::models::DomainProfile =
            mongodb::bson::from_document(doc).expect("含 override 应能反序列化");
        assert_eq!(
            profile.soul_override.as_deref(),
            Some("我是教培行业的陪伴式顾问")
        );
        assert!(profile
            .methodology_override
            .as_deref()
            .unwrap()
            .contains("试听→评估→报名"));
        assert!(profile
            .conversation_mode_policy
            .as_deref()
            .unwrap()
            .contains("empathetic_support"));
    }

    /// Task9：不含三段 override 的输出（纯销售域，AI 留空不给）→ normalize 后无这三键 →
    /// from_document 落 None（DEFAULT 兜底回落，逐字等价不回归）。
    #[test]
    fn generate_overrides_absent_default_to_none() {
        let generated = json!({
            "profileDimensions": [{"kind":"trust","displayName":"信任","description":"x"}]
        });
        let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
        assert!(normalized.get("soul_override").is_none());
        assert!(normalized.get("methodology_override").is_none());
        assert!(normalized.get("conversation_mode_policy").is_none());
        let doc = to_profile_doc(normalized);
        let profile: crate::models::DomainProfile =
            mongodb::bson::from_document(doc).expect("缺 override 应能反序列化");
        assert_eq!(profile.soul_override, None);
        assert_eq!(profile.methodology_override, None);
        assert_eq!(profile.conversation_mode_policy, None);
    }

    /// Task9：LLM 偶发把 soulOverride 给成对象（与 description/prompt_fragment 同款风险）→
    /// coerce_scalar_string_fields 应把 snake 化后的 soul_override 压平成 JSON 文本，内容不丢、
    /// from_document 不报 `invalid type: map`。
    #[test]
    fn generate_coerces_object_soul_override_to_text() {
        let generated = json!({
            "profileDimensions": [{"kind":"trust","displayName":"信任","description":"x"}],
            "soulOverride": {"身份": "顾问", "姿态": "倾听"}
        });
        let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
        assert!(
            normalized["soul_override"].is_string(),
            "对象应被压平成字符串"
        );
        let doc = to_profile_doc(normalized);
        let profile: crate::models::DomainProfile =
            mongodb::bson::from_document(doc).expect("压平后应能反序列化");
        let s = profile.soul_override.unwrap();
        assert!(s.contains("顾问") && s.contains("倾听"), "内容须保留: {s}");
    }

    /// H13：生成 prompt 必须含 stateMachine 本体 schema，引导 AI 输出客户旅程状态机。
    /// 注意 build_profile_generation_prompt 真实签名是 4 参（business_description /
    /// profile_id / display_name / knowledge_context）。
    #[test]
    fn generation_prompt_includes_state_machine_schema() {
        let prompt = super::build_profile_generation_prompt("卖课的教育机构", "edu-x", "教培", "");
        assert!(
            prompt.contains("stateMachine"),
            "生成 prompt 须含状态机本体 schema"
        );
        assert!(prompt.contains("initial"), "状态机须声明 initial 标志");
    }

    /// H13：allowedFrom 引用未知状态 → validate 拒 → 候选 generated_state_machine 回落 None。
    /// 注意 validate_state_machine 并**不**强制存在 initial 态；这里被拒是因为 `b` 是未知态
    /// （allowedFrom references unknown state → reject）。
    #[test]
    fn invalid_state_machine_falls_back_to_none() {
        let bad = mongodb::bson::doc! { "states": [ { "key": "a", "allowedFrom": ["b"] } ] };
        assert!(
            crate::routes::domains::validate_state_machine(&bad).is_err(),
            "allowedFrom 引用未知态 b 应被拒"
        );
    }

    /// H13：合法的 camelCase stateMachine（每态 key 非空唯一、allowedFrom 只引已知态）过校验。
    /// 锁定 camelCase 内层 key 直通 validate（不被 snake_case 化）。
    #[test]
    fn valid_camelcase_state_machine_passes_validate() {
        let good = mongodb::bson::doc! {
            "states": [
                { "key": "a", "name": "初识", "initial": true, "allowedFrom": ["a"] },
                { "key": "b", "name": "深入", "allowedFrom": ["a", "b"], "forbidsProactive": false }
            ]
        };
        assert!(
            crate::routes::domains::validate_state_machine(&good).is_ok(),
            "合法 camelCase 状态机应过校验"
        );
    }

    /// H13 命门 e2e 锁：`generate_domain_profile_candidate` 在 `normalize_json_keys` **之前**
    /// `as_object_mut().remove("stateMachine")`，让状态机内层 key 保持 camelCase
    /// （`allowedFrom`，而非 `allowed_from`）。运行期引擎（guards.rs / migrations）读 camelCase
    /// `allowedFrom`/`allowFromAny`/`initial`；若有人把 remove 挪到 normalize 之后，键会被 snake_case
    /// 化 → 引擎 `get_array("allowedFrom")` 静默读不到，但 validate_state_machine 仍可能空过
    /// （它 `let Ok(states) = machine.get_array("states") else { return Ok(()) }`）。
    /// 本测在单元层钉死该不变量：
    /// (1) 抽出的 stateMachine 经 `to_document` 后内层仍是 camelCase `allowedFrom`；
    /// (2) 反证——若 stateMachine 留在 remainder 走 normalize_json_keys，`allowedFrom` 会被
    ///     mangle 成 `allowed_from`，证明抽取步骤正是防住此 bug 的关键。
    #[test]
    fn state_machine_bypasses_snake_casing_via_pre_normalize_extraction() {
        // 模拟 LLM 顶层输出：camelCase 顶层字段 + camelCase 内层 stateMachine。
        let mut generated = json!({
            "displayName": "教培",
            "stateMachine": {
                "states": [
                    { "key": "a", "allowedFrom": ["a"], "initial": true, "allowFromAny": false }
                ]
            }
        });

        // —— 复刻生产抽取顺序：normalize 之前 remove("stateMachine") ——
        let raw_state_machine = generated
            .as_object_mut()
            .and_then(|m| m.remove("stateMachine"))
            .expect("应抽出 stateMachine");

        // (1) 抽出的 stateMachine 经 to_document 后内层 key 仍是 camelCase。
        let sm_doc =
            mongodb::bson::to_document(&raw_state_machine).expect("stateMachine → Document");
        let states = sm_doc.get_array("states").expect("states 数组应在");
        let first = states[0].as_document().expect("首态应是 document");
        assert!(
            first.get_array("allowedFrom").is_ok(),
            "抽取绕过 normalize → 内层须保持 camelCase allowedFrom（引擎读这个键）"
        );
        assert!(
            first.get("allowed_from").is_none(),
            "不应出现 snake_case allowed_from（一旦出现说明被 normalize 误伤）"
        );
        assert_eq!(
            first.get_bool("allowFromAny").ok(),
            Some(false),
            "allowFromAny 须保持 camelCase（引擎 get_bool 读这个键）"
        );

        // remainder（剩余顶层字段）走 normalize：顶层 displayName → display_name（这是期望行为）。
        let normalized_remainder = normalize_json_keys(generated);
        assert_eq!(normalized_remainder["display_name"], json!("教培"));
        assert!(
            normalized_remainder.get("stateMachine").is_none(),
            "stateMachine 已被抽出，不应残留在 remainder"
        );

        // (2) 反证：若 stateMachine 留在 normalize 输入里，内层 allowedFrom 会被 mangle。
        let mangled = normalize_json_keys(json!({
            "stateMachine": { "states": [ { "allowedFrom": [] } ] }
        }));
        assert!(
            mangled["state_machine"]["states"][0]
                .get("allowed_from")
                .is_some(),
            "反证：未抽取时 normalize_json_keys 会把 allowedFrom → allowed_from（引擎静默读不到）"
        );
        assert!(
            mangled["state_machine"]["states"][0]
                .get("allowedFrom")
                .is_none(),
            "反证：camelCase allowedFrom 在未抽取路径下消失——这正是抽取步骤要防的 bug"
        );
    }
}
