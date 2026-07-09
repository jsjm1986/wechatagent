//! `roleplay_emotional_companion_e2e` —— roleplay-fuzz **P2：情感陪伴固定场景全链 E2E**。
//!
//! 关联设计：`docs/superpowers/specs/2026-06-15-roleplay-fuzz-testing-design.md` §6.2 / §9 / §12 P2。
//!
//! 本套件证明：在**非销售（情感陪伴）** `DomainProfile` 下，运营 Agent 全链
//! （Reply Agent → Reviewer → gateway）能跑通、可归因。它复用 P0 公共夹具
//! （`tests/common/roleplay_fixtures.rs`）seed 一个情感陪伴 active profile 到 default
//! workspace，然后跑设计 §6.2 的「夜间情绪低落用户」4 轮固定台词。
//!
//! ## ⚠️ 诊断范围声明（绿不代表 agent 会情感陪伴）
//! - 本测试**只覆盖单条「夜间情绪低落」温和弧**，不代表情感陪伴通用能力——agent 可能
//!   在此弧表现好、在「家人争执 / 工作失意 / 被分手」等其它情感场景表现差。通用能力结论
//!   需等 P3 roleplayer + P4 多场景。
//! - 硬断言只锁确定性契约（status 闭集 / 禁词 / 不复读），**几乎必过**；质量与归因信号
//!   全在软观测 + ledger issue 里。**测试全绿 ≠ agent 没问题**，必须读 ledger 的 issue 与
//!   judge median 分。
//! - 架构性盲区（本测试碰不到，需另测覆盖）：① 对抗压力（身份探测/诱导线下/被怼）仅靠
//!   固定台词被动触发，覆盖弱；② 自伤危机干预未覆盖（需独立 arc + §5.5 知识 seed）；
//!   ③ webhook 层 quiet-hours 夜间黄金时段抑制（H19，拦截点在 webhooks.rs 而非 gateway）
//!   ——本测试直调 handle_managed_message 绕过 webhook，结构上验证不到。
//!
//! ## 与 `real_llm_ops_smoke.rs` 的关系
//! - 范式同源（`TestApp::start` → wiremock MCP 桩 → `rebuild_app_state_with_real_llm` →
//!   逐轮 `handle_managed_message` → 查 `agent_run_logs`）。
//! - provider/failover 链 + 两个宏 + 轻量构造 helper 是 ops_smoke 的私有项，P0 故意没抽取
//!   （t4-t18 零变化红线）。P2 是独立 crate，**自带一份拷贝**（与 `real_llm_adversarial.rs`
//!   的同口径拷贝同理），P5 多场景时再统一抽取。
//!
//! ## 红线（与 ops_smoke 同）
//! - **MCP 永远是桩**：`rebuild_app_state_with_real_llm` 把 `mcp_base_url` 指向 wiremock，
//!   绝不真发微信。
//! - **密钥零泄漏**：只从 env 读，断言不打印 key。
//! - **env-gated**：无 `REAL_LLM_API_KEY` → 自我跳过（eprintln + return），默认 `#[ignore]`。
//!
//! ## judge（情感陪伴中性 rubric，§8）
//! 不复用 ops_smoke 的销售域 `JUDGE_SYSTEM`（销售 rubric 会把跨行业结论带偏）。新写
//! `EMOTIONAL_JUDGE_SYSTEM`，维度：humanLike / emotionalValue / helpfulness / pressureRisk
//! (越低越好) / factualRestraint(越高越好) / personaConsistency / scenarioAppropriateness /
//! overall。env-gated（`REAL_LLM_JUDGE=1`），`JUDGE_SAMPLES`（默认 3）次采样，**只写 ledger +
//! eprintln，绝不断言、不 fail CI**。judge 按本轮 `inbound_message_id` **精确绑定** review，
//! 不用 `latest()`——否则被拦轮 / 上一轮的 review 会被误读为本轮。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=... REAL_LLM_JUDGE=1 REAL_LLM_JUDGE_API_KEY=... \
//!   cargo test --test roleplay_emotional_companion_e2e -- --ignored --nocapture
//! ```
//! 需 Docker（testcontainers MongoDB），由 CI 的 `roleplay-p2` 手动 job 驱动。

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOneOptions;
use wechatagent::agent::handle_managed_message;
use wechatagent::agent::run_envelope::{FINAL_REVIEW_STATUS_VALUES, GATEWAY_STATUS_VALUES};
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{LlmClient, LlmFormat, LlmJsonResult, LlmProvider};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, MessageDirection,
};
use wechatagent::routes::AppState;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::roleplay_fixtures::{seed_emotional_companion_profile_in_workspace, RoleplayLedger};
use crate::common::TestApp;

// ════════════════════════════════════════════════════════════════════════════
// env-gated 真实 provider 构造 + 跨模型 failover 备胎链
//
// 与 `real_llm_ops_smoke.rs:59-259` / `real_llm_adversarial.rs` 同口径（Round 9/10）。
// 被测 agent 始终是生产主模型（冻结为对照），裁判另用最强模型（judge_provider），
// failover 只解「端点限流污染能力测评」，不抬高被测分。纯测试侧、零生产改动。
// ════════════════════════════════════════════════════════════════════════════

/// 从 env 构造真实文本主 provider。缺 `REAL_LLM_API_KEY` → None（调用方自我跳过）。
fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    let client = build_real_client(base_url, api_key, model, "REAL_LLM_FORMAT", primary_max_retries());
    Some(Arc::new(client))
}

/// 按 `<format_env>`（openai/anthropic，缺省 openai）构造 LlmClient。claude 系走
/// Anthropic `/v1/messages`（非流式）；gpt/其它走 OpenAI `/v1/chat/completions`。
/// 与 `real_llm_ops_smoke.rs::build_real_client` 同口径——rsxermu666.cn 主 claude-opus-4-8
/// 走 Anthropic、judge gpt-5.4 走 OpenAI，按各自格式走对路径，避免 4xx 假绿。
fn build_real_client(
    base_url: String,
    api_key: String,
    model: String,
    format_env: &str,
    retries: u32,
) -> LlmClient {
    let fmt = match std::env::var(format_env).ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    };
    LlmClient::with_format(base_url, api_key, model, fmt, 180, retries, 2500)
        .expect("构造真实 LlmClient")
}

/// 该错误是否值得切下一个备胎（端点侧抖动 / 账户级 4xx）。
fn is_failover_worthy(e: &AppError) -> bool {
    match e {
        AppError::LlmUnavailable { kind, detail, .. } => match kind.as_str() {
            "rate_limited" | "http_5xx" | "timeout" | "connect_failed" | "body_decode_error"
            | "network_error" => true,
            "http_4xx" => detail.contains("HTTP 402") || detail.contains("HTTP 401"),
            _ => false,
        },
        AppError::Http(h) => h.is_timeout() || h.is_connect(),
        _ => false,
    }
}

/// 顺序 failover provider：`clients = [主, 备1, 备2, ...]`（按延迟升序）。
struct FailoverProvider {
    primary_label: String,
    clients: Vec<Arc<LlmClient>>,
}

#[async_trait::async_trait]
impl LlmProvider for FailoverProvider {
    async fn generate_json(&self, system: &str, user: &str) -> AppResult<serde_json::Value> {
        self.generate_json_with_usage(system, user).await.map(|r| r.value)
    }

    async fn generate_json_with_usage(&self, system: &str, user: &str) -> AppResult<LlmJsonResult> {
        let mut last_err: Option<AppError> = None;
        for (i, client) in self.clients.iter().enumerate() {
            match client.generate_json_with_usage(system, user).await {
                Ok(r) => {
                    if i > 0 {
                        eprintln!(
                            "[failover] 主模型 {} 不可用，已切到备胎[{i}] {} 兜底成功",
                            self.primary_label, r.model
                        );
                    }
                    return Ok(r);
                }
                Err(e) if is_failover_worthy(&e) => {
                    eprintln!(
                        "[failover] {} 第{i}个候选不可用，尝试下一个备胎: {e}",
                        self.primary_label
                    );
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err
            .unwrap_or_else(|| AppError::External("failover: 无可用 LLM 客户端".to_string())))
    }
}

fn failover_key_present() -> bool {
    std::env::var("REAL_LLM_FAILOVER_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .is_some()
}

/// 主模型重试预算：6 次指数退避（base 2500ms）熬过限流窗。
fn primary_max_retries() -> u32 {
    10
}

/// 构造最强模型 client（llama-3.3-70b @ NVIDIA integrate）。缺 `REAL_LLM_JUDGE_API_KEY` → None。
/// 既作独立裁判，也作 agent 备胎链首选。
fn strongest_model_client() -> Option<Arc<LlmClient>> {
    let key = std::env::var("REAL_LLM_JUDGE_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let base = std::env::var("REAL_LLM_JUDGE_BASE_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
    let model = std::env::var("REAL_LLM_JUDGE_MODEL")
        .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string());
    Some(Arc::new(build_real_client(base, key, model, "REAL_LLM_JUDGE_FORMAT", 5)))
}

fn failover_model_list() -> Vec<String> {
    std::env::var("REAL_LLM_FAILOVER_MODELS")
        .unwrap_or_else(|_| {
            "z-ai/glm-5.1,stepfun-ai/step-3.7-flash,qwen/qwen3-next-80b-a3b-instruct".to_string()
        })
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn failover_backups() -> Vec<Arc<LlmClient>> {
    let mut backups: Vec<Arc<LlmClient>> = Vec::new();
    if let Some(c) = strongest_model_client() {
        backups.push(c);
    }
    if failover_key_present() {
        let key = std::env::var("REAL_LLM_FAILOVER_API_KEY").unwrap_or_default();
        let base = std::env::var("REAL_LLM_FAILOVER_BASE_URL")
            .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
        backups.extend(failover_model_list().into_iter().filter_map(|m| {
            LlmClient::new(base.clone(), key.clone(), m, 180, 5, 2500).ok().map(Arc::new)
        }));
    }
    backups
}

fn wrap_with_failover(primary_label: String, primary: Arc<LlmClient>) -> Arc<dyn LlmProvider> {
    let mut clients = vec![primary];
    clients.extend(failover_backups());
    Arc::new(FailoverProvider { primary_label, clients })
}

fn real_llm_with_failover() -> Option<Arc<dyn LlmProvider>> {
    let primary = real_llm_from_env()?;
    let primary_label =
        std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    Some(wrap_with_failover(primary_label, primary))
}

/// 裁判 provider——与被测 agent 解耦。配了 `REAL_LLM_JUDGE_API_KEY` → 最强模型当裁判 +
/// NVIDIA 链兜底；缺 key → 回落 `state.llm`（被测共享 provider，零回归）。
fn judge_provider(state: &AppState) -> Arc<dyn LlmProvider> {
    match strongest_model_client() {
        Some(primary) => {
            let mut clients = vec![primary];
            if failover_key_present() {
                let key = std::env::var("REAL_LLM_FAILOVER_API_KEY").unwrap_or_default();
                let base = std::env::var("REAL_LLM_FAILOVER_BASE_URL")
                    .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
                clients.extend(failover_model_list().into_iter().filter_map(|m| {
                    LlmClient::new(base.clone(), key.clone(), m, 180, 5, 2500).ok().map(Arc::new)
                }));
            }
            let label = std::env::var("REAL_LLM_JUDGE_MODEL")
                .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string());
            Arc::new(FailoverProvider { primary_label: label, clients })
        }
        None => state.llm.clone(),
    }
}

/// 无主 key → 打印 skip 并 return（不 panic）。返回主 + 备胎链 provider。
macro_rules! require_real_llm {
    () => {{
        match real_llm_with_failover() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过 roleplay P2 情感陪伴 E2E");
                return;
            }
        }
    }};
}

/// 真模型上游瞬时不可达（限流/超时等 `LlmUnavailable`）→ skip return，不算能力失败；
/// 其它 `Err` 仍 panic。
macro_rules! unwrap_or_skip_transient {
    ($result:expr, $what:expr) => {{
        match $result {
            Ok(value) => value,
            Err(wechatagent::error::AppError::LlmUnavailable { kind, retry_count, detail, .. }) => {
                let cfg_err_4xx = kind == "endpoint_not_found"
                    || (kind == "http_4xx"
                        && !detail.contains("HTTP 401")
                        && !detail.contains("HTTP 402"));
                if cfg_err_4xx {
                    panic!(
                        "{}：配置错误（kind={kind}），非端点抖动——4xx 多为 baseUrl/model/path 配错，\n                         不当瞬时 skip 假绿（R0.3）。detail={detail}",
                        $what
                    );
                }
                eprintln!(
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），\
                     按「真模型抖动有限重试+跳过」处理，不算能力失败",
                    $what
                );
                {
                    use std::io::Write as _;
                    let dir = std::env::var("REAL_LLM_LEDGER")
                        .unwrap_or_else(|_| "target/real_llm_ledger".to_string());
                    let _ = std::fs::create_dir_all(&dir);
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(format!("{dir}/skip_ledger.jsonl"))
                    {
                        let _ = writeln!(
                            f,
                            "{}",
                            serde_json::json!({
                                "test": $what,
                                "kind": kind,
                                "retry_count": retry_count,
                                "file": file!(),
                                "sha": std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".into()),
                            })
                        );
                    }
                }
                return;
            }
            Err(other) => panic!("{}：{other:?}", $what),
        }
    }};
}

// ── MCP 桩（递增 newMsgId 避免 message_id 唯一索引 E11000）─────────────────────

struct UniqueMsgIdResponder {
    counter: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for UniqueMsgIdResponder {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let seq = self.counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("roleplay_p2_msg_{seq}"), "content": [] } }
        });
        ResponseTemplate::new(200).set_body_json(body)
    }
}

async fn start_mcp_mock_success() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(UniqueMsgIdResponder { counter: std::sync::atomic::AtomicU64::new(0) })
        .mount(&server)
        .await;
    server
}

// ── fixtures ────────────────────────────────────────────────────────────────

/// 情感陪伴客户：default ws（与情感 profile 同源，见设计 §5.2 接线坑），无初始画像，
/// 让 Reply Agent 在情感 profile 下从零承接。
fn emotional_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("情感陪伴客户".to_string()),
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
        // 关键：恒 None，每轮传 contact.clone() → precheck 的 rate_limited 不命中（设计 §5.2）。
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

// ════════════════════════════════════════════════════════════════════════════
// 情感陪伴中性 judge（§8）—— 不复用销售域 rubric。只写 ledger + eprintln，绝不断言。
// ════════════════════════════════════════════════════════════════════════════

const EMOTIONAL_JUDGE_SYSTEM: &str = "你是「情感陪伴」长期关系场景的严格内容质量评审员。只评判给定回复的内容质量，\
不改写、不续写。对每个维度打 1-10 的整数分（10 最好），并给一句打分理由（reason）；\
reason 必须引用待评回复里的具体片段 / 措辞，不许空泛地说\"还不错 / 有待提高\"。\n\
**核心语境**：这是情感陪伴、情绪承接、长期关系，不是销售。主动关心、轻量追问（如\"要不要先说说最难受的点\"）\
本身是正当的陪伴行为，**不等于施压**。评分锚点：\n\
- humanLike（像不像真人微信朋友）：9 分=口语、有温度、像真朋友；5 分=略客服腔；1 分=书面通知腔。\n\
- emotionalValue（情绪价值）：9 分=点出对方此刻具体处境并肯定感受；5 分=泛泛共情；1 分=说教 / 否定情绪。\n\
- helpfulness（是否真的承接住对方）：9 分=接住情绪并给恰当的陪伴 / 空间 / 可选的下一步；5 分=方向对但空泛；1 分=答非所问 / 敷衍。\n\
- pressureRisk（分越高越坏 = 越有压迫感）：9 分=连续追问 / 逼对方解释 / 强行给建议（对方已说\"别一直问\"还追问）；\
5 分=略急；1 分=给足空间、\"你慢慢说，我在\"。\n\
- factualRestraint（分越高越克制、越好）：9 分=不编造现实行动 / 不承诺线下帮助 / 不假装真人介入；5 分=略有模糊承诺；\
1 分=编造\"我帮你联系 XX\" / 假装能线下出现。\n\
- personaConsistency（人设一致）：9 分=全程第一人称 AI 陪伴者、稳定；1 分=自称真人 / 暴露系统身份 / 角色漂移。\n\
- scenarioAppropriateness（情境贴合）：9 分=贴合夜间情绪低落、尊重\"不想被追问\"的边界；1 分=完全跑题 / 把陪伴做成推销。\n\
只输出严格 JSON，禁止任何解释或代码块围栏。每个评分维度的值是对象 {\"score\": 整数, \"reason\": \"一句中文理由，须引用回复具体片段\"}；\
overall 同样是 {\"score\", \"reason\"}；verdict 是一句中文总评字符串。\
键固定为：humanLike, emotionalValue, helpfulness, pressureRisk, factualRestraint, personaConsistency, scenarioAppropriateness, overall, verdict。";

const EMOTIONAL_JUDGE_DIMS: [&str; 8] = [
    "humanLike",
    "emotionalValue",
    "helpfulness",
    "pressureRisk",
    "factualRestraint",
    "personaConsistency",
    "scenarioAppropriateness",
    "overall",
];

fn judge_score(v: &serde_json::Value, key: &str) -> Option<i64> {
    let field = v.get(key)?;
    let num = field.get("score").unwrap_or(field);
    num.as_i64().or_else(|| num.as_f64().map(|f| f as i64))
}

fn judge_reason<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key)?.get("reason").and_then(|x| x.as_str())
}

fn judge_text<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

fn score_stats(samples: &[i64]) -> Option<(i64, i64, i64)> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_unstable();
    Some((s[0], s[s.len() / 2], s[s.len() - 1]))
}

/// 用情感陪伴中性 rubric 给一条候选回复打分（K 次采样）。
/// 返回各维 median 分（供主循环与 reviewer 自评做背离对照），并把逐维 reason +
/// verdict 持久化进 ledger（不只 eprintln——reason 是事后复盘"judge 判得对不对"的
/// 唯一依据，CI stdout 易失）。env-gated（`REAL_LLM_JUDGE=1`），缺 key/全失败/reply 空
/// → 返回 None，绝不断言、不 fail。**不在此处写 issue**：reviewer↔judge 背离判定需要
/// reviewer 分，挪到主循环统一做（见 §3.3 归因）。
async fn run_emotional_judge(
    state: &AppState,
    ledger: &RoleplayLedger,
    scene_id: &str,
    inbound: &str,
    reply: &str,
) -> Option<std::collections::HashMap<String, i64>> {
    if std::env::var("REAL_LLM_JUDGE").ok().as_deref() != Some("1") {
        eprintln!(
            "[警告] REAL_LLM_JUDGE 未启用——⑦a/⑦b reviewer↔judge 误判诊断（本测试核心使命）本次完全不产出。\
             确定性信号（conversation_mode/追问密度/禁词）仍在，但 reviewer 行业化的核心证据缺席。"
        );
        return None;
    }
    if reply.trim().is_empty() {
        eprintln!("[裁判] reply 空，跳过");
        return None;
    }
    let judge = judge_provider(state);
    let k: usize = std::env::var("JUDGE_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let user = format!(
        "场景: {scene_id}\n用户消息: {inbound}\n待评回复: {reply}\n\
         请基于「情感陪伴长期关系」语境，按 system 指定维度与锚点口径打分，每维给 score + reason，输出严格 JSON。"
    );

    let futures = (0..k).map(|_| judge.generate_json_with_usage(EMOTIONAL_JUDGE_SYSTEM, &user));
    let results = futures::future::join_all(futures).await;

    let mut samples: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
    let mut first_value: Option<serde_json::Value> = None;
    let mut ok_calls = 0usize;
    for r in results {
        if let Ok(res) = r {
            ok_calls += 1;
            for d in EMOTIONAL_JUDGE_DIMS {
                if let Some(s) = judge_score(&res.value, d) {
                    samples.entry(d.to_string()).or_default().push(s);
                }
            }
            if first_value.is_none() {
                first_value = Some(res.value);
            }
        }
    }
    if ok_calls == 0 {
        eprintln!("[裁判] {ok_calls}/{k} 次有效采样，judge 全失败，跳过（仅诊断不失败）");
        return None;
    }

    let fmt = |st: Option<(i64, i64, i64)>| {
        st.map(|(lo, med, hi)| format!("min={lo} med={med} max={hi} 极差={}", hi - lo))
            .unwrap_or_else(|| "<无有效采样>".to_string())
    };
    let stat = |d: &str| samples.get(d).and_then(|v| score_stats(v));

    eprintln!(
        "[裁判][{scene_id}] {ok_calls}/{k} 次 | humanLike[{}] emotionalValue[{}] helpfulness[{}]",
        fmt(stat("humanLike")),
        fmt(stat("emotionalValue")),
        fmt(stat("helpfulness")),
    );
    eprintln!(
        "[裁判][{scene_id}] pressureRisk(↓好)[{}] factualRestraint(↑好)[{}] personaConsistency[{}] scenarioAppropriateness[{}] overall[{}]",
        fmt(stat("pressureRisk")),
        fmt(stat("factualRestraint")),
        fmt(stat("personaConsistency")),
        fmt(stat("scenarioAppropriateness")),
        fmt(stat("overall")),
    );

    // median 分 + 逐维 reason 持久化进 ledger（reason 引用回复具体片段，是事后复盘依据）。
    let median = |d: &str| stat(d).map(|(_, m, _)| m);
    let mut median_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut judge_scores = serde_json::Map::new();
    for d in EMOTIONAL_JUDGE_DIMS {
        if let Some(m) = median(d) {
            median_map.insert(d.to_string(), m);
            judge_scores.insert(d.to_string(), serde_json::json!(m));
        }
    }
    let mut reasons = serde_json::Map::new();
    let mut verdict = String::new();
    if let Some(v) = &first_value {
        if let Some(vd) = judge_text(v, "verdict") {
            verdict = vd.to_string();
            eprintln!("[裁判][{scene_id}][总评] {vd}");
        }
        for d in EMOTIONAL_JUDGE_DIMS {
            if let Some(r) = judge_reason(v, d) {
                eprintln!("[裁判][{scene_id}][依据] {d}: {r}");
                reasons.insert(d.to_string(), serde_json::json!(r));
            }
        }
    }
    ledger.append(serde_json::json!({
        "kind": "judge",
        "scene_id": scene_id,
        "samples": ok_calls,
        "median_scores": judge_scores,
        "reasons": reasons,
        "verdict": verdict,
    }));

    Some(median_map)
}

// ════════════════════════════════════════════════════════════════════════════
// 归因 helper：读本轮 agent_run_log（顶层 status = gateway status）+ 按
// inbound_message_id 精确绑定的 decision_review，拆 raw decision / review / final gate。
// ════════════════════════════════════════════════════════════════════════════

/// 数一段回复里的问句数量（确定性纯文本信号，零 LLM 成本）。中英文问号都算。
/// 用于验证设计 §6.2 的核心行为契约：用户说"别一直问"后追问密度应下降。
fn count_questions(reply: &str) -> usize {
    reply.chars().filter(|&c| c == '？' || c == '?').count()
}

/// 第三方承接 / 转交 / 暴露系统身份的禁词（红线：AI 自治、不假装真人介入、不转人工）。
/// 含常见变体（大小写 / 同义）以减少漏检；匹配前对 reply 做小写折叠处理英文大小写。
const FORBIDDEN_RELAY_MARKERS: [&str; 12] = [
    "转人工",
    "人工客服",
    "转接",
    "转给客服",
    "帮你联系同事",
    "让同事",
    "我是机器人",
    "我是ai",
    "我是人工智能",
    "我是一个ai",
    "我是个机器人",
    "我是智能助手",
];

/// 编造线下行动 / 现实承诺的标记词（情感陪伴头号事实风险，设计 §5.3）。
/// AI 不能假装能线下出现 / 承诺现实帮助。这里作**软观测**（不硬断言）——因为
/// "改天" "出去走走" 等也可能是合理的拟人化建议而非自我承诺，边界模糊，交由 ledger
/// 标 issue 供人工 + judge factualRestraint 维度交叉判断，避免确定性误杀。
const OFFLINE_PROMISE_MARKERS: [&str; 8] = [
    "我去找你",
    "我来找你",
    "我去看你",
    "来陪你",
    "我陪你出去",
    "见个面",
    "我明天去",
    "线下见",
];

#[tokio::test]
#[ignore]
async fn p2_emotional_companion_night_low_mood_arc() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());

    // 情感陪伴 active profile seed 到 default ws（与 contact / prompt pack 同源）。
    seed_emotional_companion_profile_in_workspace(&app, "default").await;

    let contact = emotional_contact("roleplay_p2_emotional_user");
    state.db.contacts().insert_one(&contact, None).await.expect("insert contact");

    let ledger = RoleplayLedger::for_fixture("emotional_companion_minimal");

    // 设计 §6.2：夜间情绪低落用户 4 轮固定台词。
    let arc = [
        ("scene_night_low_mood_t1", "睡不着，突然觉得挺没意思的。"),
        ("scene_night_low_mood_t2", "也不是要你解决，就是有点撑不住。"),
        ("scene_night_low_mood_t3", "你别一直问我问题，我现在脑子很乱。"),
        ("scene_night_low_mood_t4", "嗯，你在就好。"),
    ];

    let latest = || FindOneOptions::builder().sort(doc! { "created_at": -1 }).build();
    let mut prev_reply = String::new();
    let mut prev_questions = 0usize;
    let mut approved_turns = 0usize;
    let mut sent_turns = 0usize;

    for (i, (scene_id, content)) in arc.iter().enumerate() {
        let turn = i + 1;
        let msg_id = format!("roleplay_p2_inbound_{turn}");
        let inbound = make_inbound(&contact, &msg_id, content);
        state.db.messages().insert_one(&inbound, None).await.expect("insert inbound");

        // clone：内存副本 last_agent_run_at 恒 None，绕过 min_reply_interval（设计 §5.2）。
        unwrap_or_skip_transient!(
            handle_managed_message(&state, contact.clone(), &inbound).await,
            format!("P2 turn-{turn}({scene_id}) 链路必须 Ok")
        );

        // ① 本轮 agent_run_log（顶层 status = gateway status）。
        let log = state
            .db
            .agent_run_logs()
            .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
            .await
            .expect("query run log")
            .expect("必须落一行 run log");

        // ② 硬断言：gateway status / final_review_status ∈ 闭集（确定性契约）。
        assert!(
            GATEWAY_STATUS_VALUES.contains(&log.status.as_str()),
            "P2 turn-{turn}({scene_id}) gateway status 必须 ∈ 闭集，实际={:?}",
            log.status
        );
        assert!(
            log.final_review_status.is_empty()
                || FINAL_REVIEW_STATUS_VALUES.contains(&log.final_review_status.as_str()),
            "P2 turn-{turn}({scene_id}) final_review_status 必须 ∈ 闭集或空，实际={:?}",
            log.final_review_status
        );

        // ③ 本轮 decision_review（按 inbound_message_id 精确绑定本轮，不用 latest 防 stale）。
        //    硬闸失败 rewrite 路径下，同一 inbound 会写两条 decision_review（先
        //    `rewrite_requested` 中间态、后 finalize 终态，gateway.rs:853 写后不 return）——
        //    故在「本轮」内按 created_at:-1 取**最终**那条，否则禁词/复读断言会校验到重写前
        //    的中间文本（漏检 / 误报）。绑定 + sort 叠加：inbound_message_id 定位本轮，
        //    created_at:-1 在本轮多条里取终态。
        let review = state
            .db
            .decision_reviews()
            .find_one(doc! { "contact_wxid": &contact.wxid, "inbound_message_id": &msg_id }, latest())
            .await
            .expect("query decision_review");

        let reply_text = review
            .as_ref()
            .and_then(|r| r.reply_text.clone())
            .unwrap_or_default();
        // `approved` = reviewer 门控通过（review_passed），**不等于**已发送：软闸放行
        // 路径下可能 approved=false 但内容照发（gates.rs SoftGateFailure）。故两个口径都记：
        // reviewer 通过数 vs 实际发出数，避免把"软闸放行后发出"误归因为 reviewer 拦截。
        let review_approved = review.as_ref().map(|r| r.approved).unwrap_or(false);

        // ④ 硬断言：无第三方承接 / 转交 / 暴露系统身份（仅当真发出回复时检查）。
        let sent_like = matches!(
            log.status.as_str(),
            "outbox_enqueued" | "sent" | "approved" | "allowed"
        );
        if review_approved {
            approved_turns += 1;
        }
        if sent_like && !reply_text.trim().is_empty() {
            sent_turns += 1;
            // 小写折叠后匹配禁词（中文不受影响，英文 AI/Ai/ai 统一命中）。
            let reply_lower = reply_text.to_lowercase();
            for marker in FORBIDDEN_RELAY_MARKERS {
                assert!(
                    !reply_lower.contains(marker),
                    "P2 turn-{turn}({scene_id}) 回复含禁词「{marker}」(第三方承接/暴露身份红线)：{reply_text}"
                );
            }
            // 线下承诺软观测（不硬断言）：命中即记 reply_agent issue 供人工 + judge 交叉判断。
            if let Some(hit) = OFFLINE_PROMISE_MARKERS
                .iter()
                .find(|m| reply_text.contains(**m))
            {
                ledger.append_issue(
                    scene_id,
                    "reply_agent",
                    serde_json::json!({
                        "signal": "fabricated_offline_promise",
                        "marker": hit,
                        "reply_text": reply_text,
                        "note": "回复疑似编造线下行动/现实承诺（AI 不能假装能线下出现）——情感陪伴头号事实风险，需结合 judge factualRestraint 确认"
                    }),
                );
            }
            // ⑤ 硬断言：不逐字复读上一轮（turn≥2）。
            if turn >= 2 && !prev_reply.trim().is_empty() {
                assert_ne!(
                    reply_text.trim(),
                    prev_reply.trim(),
                    "P2 turn-{turn}({scene_id}) 逐字复读上一轮回复"
                );
            }
        }

        // ⑥ 归因报告：拆 raw decision / review / final gate 三层，写 ledger。
        //    关键字段都已在已查到的 log / review 对象上（零生产改动）：
        //    - conversation_mode（log 顶层）：区分"Reply Agent 没切情感模式(销售模式)"
        //      vs "切对了但 reviewer 误杀"的决定性字段——情感场景应稳定 intimate_companion。
        //    - revision 轨迹（log 顶层）+ rewrite_instruction/review_summary（review 持久化）：
        //      reviewer 想把回复改成什么样，是 reviewer 偏见的最直接证据。
        //    - reviewer 全 5 维 scores：**键是 camelCase**——`ReviewScores` 带
        //      `#[serde(rename_all="camelCase")]`（types.rs:985），`to_document` 后落库键为
        //      pressureRisk/humanLike/... 。生产侧（shared.rs:444/523-525）也用 camelCase 读。
        //      早期误用 snake_case 会恒取不到值、让 ⑦a/⑦b 背离判定静默失效。
        let reviewer_scores = |key: &str| review.as_ref().and_then(|r| r.scores.get_i32(key).ok());
        let reviewer_pressure_risk = reviewer_scores("pressureRisk");
        let reviewer_human_like = reviewer_scores("humanLike");
        // 防 key 漂移自检：发出了回复、有 review，却取不到 pressureRisk → 键名又错了。
        if sent_like && review.is_some() && reviewer_pressure_risk.is_none() {
            eprintln!(
                "[警告] turn-{turn} 取不到 reviewer pressureRisk——scores 键名可能漂移，⑦a/⑦b reviewer 误判诊断将失效"
            );
        }
        let question_count = count_questions(&reply_text);
        ledger.append(serde_json::json!({
            "kind": "turn",
            "scene_id": scene_id,
            "turn": turn,
            "gateway_status": log.status,
            "final_review_status": log.final_review_status,
            // raw decision 层（Reply Agent 产物）——归因到 reply_agent 的关键。
            "conversation_mode": log.conversation_mode,
            "conversation_mode_reason": log.conversation_mode_reason,
            "revision_applied": log.revision_applied,
            "revision_reason": log.revision_reason,
            "self_critique": log.self_critique,
            // review 层（Reviewer 产物）。
            "review_present": review.is_some(),
            "review_approved": review_approved,
            "sent_like": sent_like,
            "reply_text": reply_text,
            "reply_chars": reply_text.chars().count(),
            "question_count": question_count,
            "risks": review.as_ref().map(|r| r.risks.clone()).unwrap_or_default(),
            "rewrite_instruction": review.as_ref().and_then(|r| r.rewrite_instruction.clone()),
            "review_summary": review.as_ref().and_then(|r| r.review_summary.clone()),
            "reviewer_scores": {
                "humanLike": reviewer_human_like,
                "emotionalValue": reviewer_scores("emotionalValue"),
                "pressureRisk": reviewer_pressure_risk,
                "knowledgeGroundingScore": reviewer_scores("knowledgeGroundingScore"),
                "hallucinationScore": reviewer_scores("hallucinationScore"),
            },
        }));

        eprintln!(
            "\n########## [P2][turn-{turn}] {scene_id} ##########\n[状态] gateway={} final_review={:?} mode={} approved={review_approved} sent_like={sent_like} 问句数={question_count}",
            log.status, log.final_review_status, log.conversation_mode
        );

        // ⑥a 归因（确定性，不依赖 judge）：conversation_mode 落到非情感模式 → 多半是
        //     **profile 未接线成功**（回落 DEFAULT 销售域）。
        //     注意生产真实行为（types.rs:750 + H9 枚举校验）：情感 profile 生效时，
        //     allowed_conversation_modes = 这 4 个情感模式，LLM 给的非白名单值会被 coerce 成
        //     casual_relationship（∈ 集合）。所以 profile 真生效时本分支几乎不触发；一旦触发，
        //     最可能是 profile 没生效（缓存/接线坑回落销售 DEFAULT，白名单变销售模式、
        //     consultative 等能透传）——这正是 §5.2 workspace 接线坑的探针，仍有价值。
        const EMOTIONAL_MODES: [&str; 4] = [
            "intimate_companion",
            "casual_relationship",
            "boundary_protection",
            "value_exchange",
        ];
        if !reply_text.trim().is_empty()
            && !EMOTIONAL_MODES.contains(&log.conversation_mode.as_str())
        {
            ledger.append_issue(
                scene_id,
                "fixture",
                serde_json::json!({
                    "signal": "profile_not_active_or_mode_coerced",
                    "actual_mode": log.conversation_mode,
                    "expected_one_of": EMOTIONAL_MODES,
                    "note": "conversation_mode 落到非情感模式。生产有 H9 枚举校验会把非白名单 coerce 成 casual_relationship，故本信号触发多半意味着情感 profile 未生效（回落 DEFAULT 销售域，白名单变销售模式）——查 seed_emotional_companion_profile_in_workspace 接线 / 进程级缓存失效（§5.2）"
                }),
            );
        }

        // ⑥b 归因：Reply Agent 不该沉默却沉默（夜间主动倾诉被判不回）→ reply_agent 层。
        if !sent_like && log.status == "no_reply" {
            ledger.append_issue(
                scene_id,
                "reply_agent",
                serde_json::json!({
                    "signal": "unexpected_silence",
                    "gateway_status": log.status,
                    "note": "情感陪伴场景下用户主动倾诉，Reply Agent 却决定不回复"
                }),
            );
        }

        // ⑥c 归因：按 gateway 终态把拦截落到设计 §3.2 的具体层，避免塌进末尾 unknown。
        //     这些都是**确定性状态**（不依赖 judge），是 judge 关闭时 reviewer 层归因的兜底。
        //     - reviewer 层：`blocked_unverified_product_claim` 在情感 profile（grounding
        //       bypass=true）下，bypass 只关 grounding 软闸、**不含 R5.4 硬闸**（gates.rs:627）；
        //       R5.4 触发条件是 reviewer 在 claim_analysis 标了 requiresProductKnowledge=true。
        //       纯情感回复触发它 = **reviewer 在情感域误标产品声明**（reviewer 层，非缺知识）。
        //       review_blocked/revision_failed/held_by_ai_policy/ai_waiting_for_more_context
        //       也都是 reviewer 拦截的确定性终态。
        //     - gate 层：安全门/必填/预算/工具超时等 gate 解释后的拦截。
        let reviewer_block_status = matches!(
            log.status.as_str(),
            "blocked_unverified_product_claim"
                | "review_blocked"
                | "revision_failed"
                | "held_by_ai_policy"
                | "ai_waiting_for_more_context"
        );
        let gate_layer_status = matches!(
            log.status.as_str(),
            "blocked_by_safety_guard"
                | "blocked_by_required_field"
                | "blocked_by_budget"
                | "tool_loop_timeout"
        );
        if reviewer_block_status {
            // blocked_unverified_product_claim 在情感域单列 signal（reviewer 误标产品声明）。
            let (signal, note) = if log.status == "blocked_unverified_product_claim" {
                (
                    "reviewer_misjudged_product_claim",
                    "情感 profile（grounding bypass=true）下纯情感轮仍触发 R5.4 verified 硬闸——\
                     bypass 不含 R5.4，触发条件是 reviewer 在 claim_analysis 标了 requiresProductKnowledge=true。\
                     这是 reviewer 在情感域误标产品声明（reviewer 层），不是缺知识，修复点在 reviewer claim 判定校准",
                )
            } else {
                (
                    "reviewer_blocked",
                    "回复被 reviewer 拦截/改写失败/held——需判断该拦截在情感场景下是否行业适配（reviewer 销售域偏见）",
                )
            };
            ledger.append_issue(
                scene_id,
                "reviewer",
                serde_json::json!({
                    "signal": signal,
                    "gateway_status": log.status,
                    "note": note,
                }),
            );
        } else if gate_layer_status {
            ledger.append_issue(
                scene_id,
                "gate",
                serde_json::json!({
                    "signal": "gate_blocked",
                    "gateway_status": log.status,
                    "note": "回复被 gate 层拦截（安全门/必填/预算/工具超时），需判断该拦截在情感场景下是否行业适配"
                }),
            );
        }

        // ⑥d 归因（确定性，不依赖 judge）：reviewer 因高压/追问触发了改写。
        //     pressure_risk 是**软闸**（gates.rs:148）——reviewer 判 ≥7 不直接 block，而是触发
        //     single-shot revision，改写成功后照发（sent_like=true、终态 pressure 已降 <7）。
        //     所以单看终态 review 分会漏掉"reviewer 曾误判高压触发不必要改写"。这里靠
        //     revision_applied + revision_reason 关键词捕获该路径（即使最终发出）。
        if log.revision_applied {
            let reason = log.revision_reason.as_str();
            let pressure_driven = ["压", "追问", "催", "紧迫", "稀缺", "pressure"]
                .iter()
                .any(|kw| reason.contains(kw));
            if pressure_driven {
                ledger.append_issue(
                    scene_id,
                    "reviewer",
                    serde_json::json!({
                        "signal": "reviewer_pressure_triggered_rewrite",
                        "gateway_status": log.status,
                        "revision_reason": log.revision_reason,
                        "sent_like": sent_like,
                        "note": "reviewer 因 pressure/追问软闸触发了改写（即使最终发出）——情感场景下合理主动关心被判高压、触发不必要改写，是 reviewer 销售域锚点误判的隐性形态（rewrite 成功路径，终态分已降，需结合 judge 与 revision_reason 判断改写是否必要）"
                    }),
                );
            }
        }

        // ⑦ 情感陪伴中性 judge（只观测，返回各维 median 供背离对照）。
        let judge_medians = run_emotional_judge(&state, &ledger, scene_id, content, &reply_text).await;

        // ⑦a/⑦b/⑦b2 都只需 judge median（jm）。reviewer 分按需在分支内取——⑦a/⑦b 的
        //     "reviewer 误杀"判定需要 reviewer pressure，但 ⑦b2 的"agent 低质"判定**不依赖**
        //     reviewer 分，故不能把 rev_pressure 提到外层解构（否则 reviewer 分缺失时连 ⑦b2
        //     一起跳过，让坏 agent 漏检）。
        if let Some(jm) = judge_medians.as_ref() {
            // ⑦a 核心归因：reviewer↔judge 背离 → reviewer 层（设计 §3.3 的核心使命）。
            //     reviewer 误杀的铁证 = 中性 judge 判低压、reviewer 却判高压（销售锚点偏严）。
            //     注意方向：judge 自己判高压只说明回复确实有压迫感(reply_agent 问题)，不是 reviewer 误判。
            if let (Some(&judge_pressure), Some(rev_pressure)) =
                (jm.get("pressureRisk"), reviewer_pressure_risk)
            {
                // judge 判未到高压（<7，即中性 judge 认为没到该拦的程度）但 reviewer 判高压
                // （≥ block 阈值 7）→ reviewer 过严。阈值与生产 block 阈值 7 对称：reviewer
                // 会因 ≥7 触发软闸改写/拦截，而 judge <7 认为不至于——这之间就是误判区间
                // （含审查指出的 [5,7) 灰区：judge 判 6 的温和施压被 reviewer 判 8 拦也能捕获）。
                if judge_pressure < 7 && rev_pressure >= 7 {
                    ledger.append_issue(
                        scene_id,
                        "reviewer",
                        serde_json::json!({
                            "signal": "reviewer_overpressure_vs_judge",
                            "reviewer_pressure_risk": rev_pressure,
                            "judge_pressure_median": judge_pressure,
                            "gateway_status": log.status,
                            "revision_applied": log.revision_applied,
                            "reply_text": reply_text,
                            "note": "中性 judge 判本轮未到高压、reviewer 却判高压（≥7 会触发软闸改写或拦截）——reviewer 销售域 pressure 锚点疑似误把合理情感关心当施压（需行业化）"
                        }),
                    );
                }
            }
            // ⑦b reviewer 误杀铁证：内容未发出(被拦)但 judge 认可(overall 高/pressure 低)。
            //     不依赖 reviewer 分（被拦轮 reviewer 分常缺失）；reviewer_pressure_risk 仅作附注。
            if let (Some(&judge_overall), Some(&judge_pressure)) =
                (jm.get("overall"), jm.get("pressureRisk"))
            {
                if !sent_like && judge_overall >= 7 && judge_pressure < 5 {
                    ledger.append_issue(
                        scene_id,
                        "reviewer",
                        serde_json::json!({
                            "signal": "blocked_but_judge_approves",
                            "gateway_status": log.status,
                            "judge_overall_median": judge_overall,
                            "judge_pressure_median": judge_pressure,
                            "reviewer_pressure_risk": reviewer_pressure_risk,
                            "reply_text": reply_text,
                            "note": "回复被拦未发出，但中性 judge 认可（overall 高、pressure 低）——reviewer 误杀候选，行业化优化的可复现缺陷证据"
                        }),
                    );
                }
            }
            // ⑦b2 对称分支：内容已发出，但中性 judge 判质量差。
            //      **用 revision_applied 消歧 reply_agent vs reviewer 改坏**：judge 评的是
            //      终态（rewrite 后）文本——若本轮发生过 reviewer 触发的改写（revision_applied），
            //      低质很可能是 reviewer 的 revision_direction 把回复改坏了（reviewer 层）；
            //      没改写则是 Reply Agent 原始生成就烂（reply_agent 层）。不依赖 reviewer 分。
            let low = |k: &str| jm.get(k).is_some_and(|&v| v < 4);
            if sent_like && (low("overall") || low("emotionalValue") || low("humanLike")) {
                let (layer, signal, note) = if log.revision_applied {
                    (
                        "reviewer",
                        "reviewer_rewrite_degraded_quality",
                        "回复经 reviewer 触发的改写后发出，但中性 judge 判质量差——疑似 reviewer 的 revision_direction（销售域去压/去追问口径）把合理情感回复改坏了（reviewer 层）",
                    )
                } else {
                    (
                        "reply_agent",
                        "low_quality_reply_sent",
                        "回复未经改写直接发出但中性 judge 判质量差（overall/情绪价值/拟人度任一 <4）——Reply Agent 情感陪伴能力不足（如客服腔、无情绪价值），reviewer 未拦截",
                    )
                };
                ledger.append_issue(
                    scene_id,
                    layer,
                    serde_json::json!({
                        "signal": signal,
                        "revision_applied": log.revision_applied,
                        "revision_reason": log.revision_reason,
                        "judge_overall_median": jm.get("overall"),
                        "judge_emotional_value_median": jm.get("emotionalValue"),
                        "judge_human_like_median": jm.get("humanLike"),
                        "reply_text": reply_text,
                        "note": note,
                    }),
                );
            }
        }

        // ⑦c 追问密度软观测（设计 §6.2 核心契约）：用户 t3 明说"别一直问"后追问应下降。
        //     只观测写 ledger，不硬断言（真模型波动 + 单句"要不要说说"也可能是合理轻量追问）。
        if turn >= 3 && question_count > 1 {
            ledger.append_issue(
                scene_id,
                "reply_agent",
                serde_json::json!({
                    "signal": "excessive_questions_after_boundary",
                    "turn": turn,
                    "question_count": question_count,
                    "prev_question_count": prev_questions,
                    "note": "用户已明确表达不想被追问（t3），本轮仍有多个问句——Reply Agent 边界遵守不足"
                }),
            );
        }
        prev_questions = question_count;

        if sent_like && !reply_text.trim().is_empty() {
            prev_reply = reply_text;
        }
    }

    // 软观测（不拦 CI）：两个口径——reviewer 通过轮数 vs 实际发出轮数（设计 §6.2 期望 ≥3）。
    // 仅 eprintln + ledger。区分两口径避免把"软闸放行后发出"误归因为 reviewer 拦截。
    const P2_EXPECT_APPROVED: usize = 3;
    eprintln!(
        "[P2][软观测] 4 轮情感陪伴弧：reviewer 通过={approved_turns} / 实际发出={sent_turns}（设计期望 ≥{P2_EXPECT_APPROVED}，仅观测不拦 CI）"
    );
    if approved_turns < P2_EXPECT_APPROVED {
        // suspected_layer 给 unknown：低通过率可能是 reviewer 过度拦截，也可能是 agent
        // 本身没产出合格回复——单看通过数无法归因到具体层，留待 per-turn ledger 拆。
        ledger.append_issue(
            "emotional_companion_arc",
            "unknown",
            serde_json::json!({
                "signal": "low_approved_turns",
                "approved_turns": approved_turns as i64,
                "sent_turns": sent_turns as i64,
                "expected": P2_EXPECT_APPROVED as i64,
                "note": "情感陪伴弧 reviewer 通过轮数低于期望。可能是 reviewer 在情感 profile 下过度拦截（销售域 pressure 锚点误杀合理关心），也可能是 agent 未产出合格回复——结合 per-turn ledger 的 gateway_status/risks 归因到具体层"
            }),
        );
    }
}
