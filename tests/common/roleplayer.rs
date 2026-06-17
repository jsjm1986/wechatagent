//! R5.1 LLM Roleplayer —— 让真实大模型**扮演客户**做动态博弈。
//!
//! 关联设计：`docs/superpowers/specs/2026-06-15-roleplay-fuzz-testing-design.md` §7
//! + `.kiro/specs/universal-test-coverage/requirements.md` R5.1。
//!
//! ## 核心价值
//! 现有所有「多轮」测试客户台词 100% 写死，博弈链是断的（客户 t3 说"别问"是预设，
//! 不是因 agent t2 真追问了）。本模块让 LLM **按人设+场景目标真实反应 agent 上一句**：
//! agent 接得好→客户软化推进，接不好→客户升级刁难。这是「全部 LLM 驱动的测试」的核心。
//!
//! ## 防作弊（设计 §7.2，硬约束）
//! - roleplayer **只看对话历史**（`history`），**绝不喂** reviewer 分数 / operation_state /
//!   agent reasoning ——否则它会"知道答案"演得失真。本模块的 API 在类型层面就只接受
//!   对话历史，拿不到 agent 内部状态。
//! - 每轮输出 1-3 句微信口语；可不配合 agent 但不出戏评价测试。
//! - parse / timeout / 429 → 用 `fallback_line`，标 `source=Fallback`（fallback **不是**
//!   "测试通过"，只说明本轮外部扮演器不可用）。
//!
//! ## 异族硬门（R5.0.1）
//! agent=claude-opus-4-8 / judge=gpt-5.4 / **roleplayer=第三族**（默认 NVIDIA
//! llama-3.3-70b）。roleplayer 用 temperature ~0.8（要有变化、像真人），经
//! `LlmClient::with_temperature` 覆盖（生产默认 0.2）。

#![allow(dead_code)]

use std::sync::Arc;

use wechatagent::llm::{LlmClient, LlmFormat, LlmProvider};

/// 客户人设契约——给 roleplayer 的"我是谁、要什么、边界在哪"。防"乱演"。
#[derive(Debug, Clone)]
pub struct UserPersona {
    /// 身份一句话（如"刚搬到新城市、一个人住的年轻人"）。
    pub identity: String,
    /// 性格/说话风格（如"话少、慢热、不爱直接表达情绪"）。
    pub temperament: String,
    /// 这次来的真实诉求（如"不需要被教，只需要被听见"）。
    pub need: String,
    /// 边界/不会做的事（如"不会主动说太多、被追问会退缩"）。
    pub boundary: String,
}

/// 一轮对话。`Customer` = roleplayer 演的客户，`Agent` = 被测 AI。
#[derive(Debug, Clone)]
pub enum Speaker {
    Customer,
    Agent,
}

#[derive(Debug, Clone)]
pub struct DialogueTurn {
    pub speaker: Speaker,
    pub text: String,
}

/// roleplayer 单轮产出来源。`Fallback` 表示外部扮演器不可用（非测试通过信号）。
#[derive(Debug, Clone, PartialEq)]
pub enum RoleplaySource {
    Generated,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct RoleplayTurnResult {
    pub message: String,
    pub source: RoleplaySource,
    pub provider_label: Option<String>,
    pub parse_error: Option<String>,
}

/// 构造第三族 roleplayer client（默认 NVIDIA llama-3.3-70b @ temperature 0.8）。
///
/// 读 `ROLEPLAYER_*` env（独立于 agent 的 REAL_LLM_* / judge 的 REAL_LLM_JUDGE_*，
/// 保证异族）。缺 `ROLEPLAYER_API_KEY` → None（调用方自我跳过，不回落 agent client，
/// 否则违反 R5.0.1 异族硬门）。
pub fn roleplayer_client() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("ROLEPLAYER_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("ROLEPLAYER_BASE_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
    let model = std::env::var("ROLEPLAYER_MODEL")
        .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string());
    let temperature = std::env::var("ROLEPLAYER_TEMPERATURE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.8);
    let fmt = match std::env::var("ROLEPLAYER_FORMAT").ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    };
    let client = LlmClient::with_format(base_url, api_key, model, fmt, 120, 4, 2000)
        .ok()?
        .with_temperature(temperature);
    Some(Arc::new(client))
}

/// 让 roleplayer 演客户产出**下一句**。只喂人设+场景目标+对话历史（防作弊）。
///
/// `scene_goal`：本场景客户的整体处境/目标（如"夜间情绪低落主动发消息，想要被承接
/// 而非被解决"）。`history`：到目前为止的完整对话（客户+agent 交替）。
///
/// roleplayer 失败（端点抖动/parse 失败）时返回 `fallback_line` + `source=Fallback`，
/// 不 panic——让调用方决定是 skip 还是用 fallback 继续。
pub async fn roleplay_user_turn(
    client: &Arc<LlmClient>,
    persona: &UserPersona,
    scene_goal: &str,
    history: &[DialogueTurn],
    fallback_line: &str,
) -> RoleplayTurnResult {
    let system = build_roleplayer_system(persona, scene_goal);
    let user = render_history_for_roleplayer(history);

    match client.generate_json(&system, &user).await {
        Ok(value) => {
            // roleplayer 被要求输出 {"message":"..."} —— 抽取 message 字段。
            match value.get("message").and_then(|m| m.as_str()) {
                Some(msg) if !msg.trim().is_empty() => RoleplayTurnResult {
                    message: msg.trim().to_string(),
                    source: RoleplaySource::Generated,
                    provider_label: Some("roleplayer".to_string()),
                    parse_error: None,
                },
                _ => RoleplayTurnResult {
                    message: fallback_line.to_string(),
                    source: RoleplaySource::Fallback,
                    provider_label: Some("roleplayer".to_string()),
                    parse_error: Some(format!("缺 message 字段: {value}")),
                },
            }
        }
        Err(e) => RoleplayTurnResult {
            message: fallback_line.to_string(),
            source: RoleplaySource::Fallback,
            provider_label: Some("roleplayer".to_string()),
            parse_error: Some(format!("roleplayer 调用失败: {e}")),
        },
    }
}

/// roleplayer 的 system prompt：立人设契约 + 防作弊约束 + 输出格式。
fn build_roleplayer_system(persona: &UserPersona, scene_goal: &str) -> String {
    format!(
        r#"你在一个测试里**扮演一位微信上的真实客户**，对方（"助理"）是一个 AI 客服/陪伴助手。你要像真人一样跟它聊天。

【你是谁】{identity}
【你的性格/说话风格】{temperament}
【你这次来的真实诉求】{need}
【你的边界】{boundary}

【本次场景】{scene_goal}

【扮演规则——务必遵守】
1. 你**只能**根据上面的人设和下面的对话历史反应，**像真人一样**回应助理的上一句话。
2. 助理接得好（懂你、给你空间、有温度）→ 你可以软化、慢慢多说一点；助理接不好（敷衍、官腔、一直追问、答非所问）→ 你可以冷淡、升级、或表达不满。**真实地反应，不要无条件配合。**
3. 每次只说 **1-3 句**微信口语，像真的在打字聊天，不要长篇大论、不要分点。
4. **绝对不要**跳出角色评价这个测试、不要解释你在扮演、不要提"AI"或"测试"。你就是那个客户。
5. 不要替助理说话，只说你自己（客户）这一轮要发的消息。

【输出格式】只输出一个 JSON 对象：{{"message":"你这一轮要发给助理的微信消息"}}
第一个字符是 {{，最后一个字符是 }}，不要任何解释、不要代码块围栏。"#,
        identity = persona.identity,
        temperament = persona.temperament,
        need = persona.need,
        boundary = persona.boundary,
        scene_goal = scene_goal,
    )
}

/// 把对话历史渲染成喂给 roleplayer 的 user message。**只含对话文本**，不含任何
/// agent 内部状态（reviewer 分数/operation_state/reasoning）——防作弊。
fn render_history_for_roleplayer(history: &[DialogueTurn]) -> String {
    if history.is_empty() {
        return "（还没有对话，请你作为客户主动发出第一条消息。）".to_string();
    }
    let mut out = String::from("到目前为止的对话（你是\"客户\"，对方是\"助理\"）：\n\n");
    for turn in history {
        let who = match turn.speaker {
            Speaker::Customer => "客户（你）",
            Speaker::Agent => "助理",
        };
        out.push_str(&format!("{who}：{}\n", turn.text));
    }
    out.push_str("\n请你作为\"客户\"，根据助理上一句的表现，发出你的下一条消息。");
    out
}
