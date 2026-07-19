//! `real_llm_cross_domain_arc` —— R2.2 **跨域全链长程闭环**真模型集成测试。
//!
//! 关联：`.kiro/specs/universal-test-coverage/requirements.md` R2.2。
//!
//! ## 这测试要解决什么
//! 现有真模型多轮测试（`real_llm_ops_smoke.rs` t15-t18）全是销售域，且硬断言只锁
//! 「status ∈ 闭集」这种与业务无关的壳——agent 行为错了测试也照绿。R2.2 要的是
//! **跨域**（销售 + 情感陪伴）**全链长程**（多轮 arc）闭环，断言对齐**真实业务契约**：
//! 即便 agent 行为错（说转人工 / 逐字复读 / 对吐露真实信息的用户从不记录任何画像），
//! 测试也能变红。
//!
//! ## 两条 arc（一个驱动 `run_arc` 复用）
//! - **情感陪伴域**（复用 `seed_emotional_companion_profile_in_workspace`）：4 轮夜间倾诉弧，
//!   含 SmallTalk / NewFactRevealed / Objection。
//! - **销售域**（DEFAULT profile，不 seed，`load_active_domain_profile` 回落
//!   `default_domain_profile`）：4 轮咨询弧，含 SmallTalk / NewFactRevealed / Commitment /
//!   Objection。作为跨域对照（judge 标尺经 `build_judge_rubric` 自动翻极性）。
//!
//! ## 契约级硬断言（能在 agent 行为错误时变红，且不过拟合）
//! 1. **status ∈ 闭集**：`GATEWAY_STATUS_VALUES` / `FINAL_REVIEW_STATUS_VALUES`（保底，
//!    引擎写未知状态即红）。
//! 2. **禁词扫描**：每轮真发出的 reply_text 不含转人工 / 人工客服 / 暴露机器人身份等
//!    `check-no-human-takeover` 红线词（命中即红）。
//! 3. **不逐字复读**：turn≥2 的回复 ≠ 上一轮（命中即红）。
//! 4. **arc 级画像落地**：一条揭示了真实信息（NewFactRevealed）且至少发出过一条回复的
//!    arc 跑完后，contact 必须留下**至少一项**画像信号（memory_summary 非空 OR agent_profile
//!    存在 OR domain_attributes 有键）。对一个多轮吐露真实情况的用户，agent 全程零记录 =
//!    真实的画像缺陷，会变红。**为什么是 arc 级而非每轮**：画像写虽在
//!    `apply_agent_updates`（gateway.rs:1606）同步路径，但写哪些字段由 LLM 单轮产出
//!    （`profile_update` / `memory_update` / `domain_signals` 是否非空）决定——单轮硬断
//!    「必更新」会因真模型波动假红。arc 级 4 字段析取最大化鲁棒性，仍能抓真 bug。
//!    **为什么不过拟合**：只查字段「有无 / 是否非空」，绝不锁具体画像内容 / 措辞。
//!
//! ## 降级为观测（诚实声明，不硬断言的原因）
//! - **SmallTalk 反向（过度画像）**：原计划「单句寒暄后画像关键字段不变」做硬断言。
//!   实查：画像字段更新与否由 LLM 单轮 `profile_update`/`domain_signals` 是否非空驱动
//!   （gateway.rs:2578/2655/2777），首轮寒暄合理地设 customer_stage=initial、令
//!   operation_state 由 None→new_contact 本就是 CHANGE，硬反向断言会假红。故降级为
//!   **观测**：记录寒暄前后画像指纹是否实质膨胀，膨胀则写 ledger issue 供人工 + judge 复盘。
//! - **Commitment→任务**：`handle_managed_message` **不**跑 `planner::tick`；commitment→
//!   follow_up 的到期扫描在 `planner` 独立 worker（见 `tests/planner_commitment_due.rs`）。
//!   同步路径只在 LLM 输出 `last_commitment` 时把承诺 append 进 `contact.commitments`
//!   （gateway.rs:2686，且 due_at 还要 LLM 给结构化 commitment）。条件层层 LLM-gated，
//!   无法确定性断言，故降级为**观测**：记录 commitment 轮后 `contact.commitments` 是否增长 +
//!   是否产生 follow_up 任务。诚实优先于凑断言数。
//! - **judge**：用 `build_judge_rubric(&profile)`（R1.1）派生标尺打分，**只观测**
//!   （eprintln + ledger），业务硬断言才是本测试的门。
//!
//! ## 红线（与 ops_smoke / 情感 e2e 同）
//! - **MCP 永远是桩**（wiremock），绝不真发微信。
//! - **env-gated**：无 `REAL_LLM_API_KEY` → 自我跳过；默认 `#[ignore]`，需 Docker。
//! - **密钥零泄漏**：只从 env 读，断言不打印 key。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=... REAL_LLM_JUDGE=1 REAL_LLM_JUDGE_API_KEY=... \
//!   cargo test --test real_llm_cross_domain_arc -- --ignored --nocapture
//! ```

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOneOptions;
use std::time::Duration;
use wechatagent::agent::run_envelope::{FINAL_REVIEW_STATUS_VALUES, GATEWAY_STATUS_VALUES};
use wechatagent::agent::{
    atomic_claim_pending, default_domain_profile, example_emotional_companion_profile,
    handle_managed_message, invalidate_global_domain_profile_cache, process_entry, OutboxStatus,
};
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{LlmClient, LlmFormat, LlmJsonResult, LlmProvider};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, DomainProfile, MessageDirection,
};
use wechatagent::routes::AppState;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::capability_evidence::CapabilityEvidence;
use crate::common::judge::{build_judge_rubric, build_judge_user, JudgeRubric};
use crate::common::redline::{
    contains_unnegated, ENGLISH_HANDOFF_MARKERS, HANDOFF_MARKERS, IDENTITY_LEAK_MARKERS,
};
use crate::common::roleplay_fixtures::{
    seed_emotional_companion_profile_in_workspace, RoleplayLedger,
};
use crate::common::TestApp;

// ════════════════════════════════════════════════════════════════════════════
// env-gated 真实 provider 构造 + 跨模型 failover 备胎链
//
// 与 `roleplay_emotional_companion_e2e.rs` / `real_llm_ops_smoke.rs` 同口径整段照抄
// （被测 agent = 生产主模型；judge 另用最强模型；failover 只解端点限流污染，不抬被测分）。
// 纯测试侧、零生产改动。
// ════════════════════════════════════════════════════════════════════════════

/// 从 env 构造真实文本主 provider。缺 `REAL_LLM_API_KEY` → None（调用方自我跳过）。
fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://token-plan-cn.xiaomimimo.com/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    let client = build_real_client(
        base_url,
        api_key,
        model,
        "REAL_LLM_FORMAT",
        primary_max_retries(),
    );
    Some(Arc::new(client))
}

/// 按 `<format_env>`（openai/anthropic，缺省 openai）构造 LlmClient。
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
        self.generate_json_with_usage(system, user)
            .await
            .map(|r| r.value)
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

/// 主模型重试预算：10 次指数退避（base 2500ms）熬过限流窗。
fn primary_max_retries() -> u32 {
    10
}

/// 构造最强模型 client。缺 `REAL_LLM_JUDGE_API_KEY` → None。既作独立裁判，也作 agent 备胎链首选。
fn strongest_model_client() -> Option<Arc<LlmClient>> {
    let key = std::env::var("REAL_LLM_JUDGE_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base = std::env::var("REAL_LLM_JUDGE_BASE_URL")
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string());
    let model = std::env::var("REAL_LLM_JUDGE_MODEL")
        .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string());
    Some(Arc::new(build_real_client(
        base,
        key,
        model,
        "REAL_LLM_JUDGE_FORMAT",
        5,
    )))
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
            LlmClient::new(base.clone(), key.clone(), m, 180, 5, 2500)
                .ok()
                .map(Arc::new)
        }));
    }
    backups
}

fn wrap_with_failover(primary_label: String, primary: Arc<LlmClient>) -> Arc<dyn LlmProvider> {
    let mut clients = vec![primary];
    clients.extend(failover_backups());
    Arc::new(FailoverProvider {
        primary_label,
        clients,
    })
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
                    LlmClient::new(base.clone(), key.clone(), m, 180, 5, 2500)
                        .ok()
                        .map(Arc::new)
                }));
            }
            let label = std::env::var("REAL_LLM_JUDGE_MODEL")
                .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".to_string());
            Arc::new(FailoverProvider {
                primary_label: label,
                clients,
            })
        }
        None => state.llm.clone(),
    }
}

/// 无主 key → 打印 skip 并 return（不 panic）。返回主 + 备胎链 provider。
macro_rules! require_real_llm {
    ($evidence:expr) => {{
        match real_llm_with_failover() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过 R2.2 跨域全链闭环测试");
                $evidence.infra_skip("REAL_LLM_API_KEY missing");
                return;
            }
        }
    }};
}

/// 真模型上游瞬时不可达（限流/超时等 `LlmUnavailable`）→ skip return，不算能力失败；
/// 配置错误的 4xx（非 401/402）→ panic（R0.3 不当瞬时 skip 假绿）；其它 `Err` 仍 panic。
/// 整段照抄 `roleplay_emotional_companion_e2e.rs::unwrap_or_skip_transient`。
macro_rules! unwrap_or_skip_transient {
    ($evidence:expr, $result:expr, $what:expr) => {{
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
                $evidence.infra_skip(format!(
                    "{}: transient LLM failure kind={kind}, retries={retry_count}",
                    $what
                ));
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
                return None;
            }
            Err(other) => panic!("{}：{other:?}", $what),
        }
    }};
}

#[derive(Debug)]
struct ArcWitness {
    sent_turns: usize,
    delivered: usize,
    llm_calls: usize,
}

// ── MCP 桩（递增 newMsgId 避免 message_id 唯一索引 E11000）─────────────────────

struct UniqueMsgIdResponder {
    counter: std::sync::atomic::AtomicU64,
}

impl wiremock::Respond for UniqueMsgIdResponder {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("cross_domain_msg_{seq}"), "content": [] } }
        });
        ResponseTemplate::new(200).set_body_json(body)
    }
}

async fn start_mcp_mock_success() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(UniqueMsgIdResponder {
            counter: std::sync::atomic::AtomicU64::new(0),
        })
        .mount(&server)
        .await;
    server
}

// ── fixtures ────────────────────────────────────────────────────────────────

/// 跨域客户：default ws（与 active profile / prompt pack 同源，见接线坑）。无初始画像、
/// operation_state=None，让 Reply Agent 从零承接——也让「arc 级画像信号出现」的观测/断言更干净。
/// `last_agent_run_at` 恒 None，每轮传 contact.clone() → precheck min_reply_interval 不命中。
fn fresh_contact(wxid: &str, workspace_id: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("跨域测试客户".to_string()),
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
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
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

// ── 禁词（check-no-human-takeover 红线）────────────────────────────────────────
//
// 已抽到共享 `common::redline`（HANDOFF_MARKERS / IDENTITY_LEAK_MARKERS / ENGLISH_HANDOFF_MARKERS
// + contains_unnegated）：补「转人工/人工客服」漏词 + 否定剔除消除「不用转接」误判。
// 选词纪律见 redline.rs（[[no-overfitting]]：不收裸「人工」「真人」防误伤）。

// ── arc 定义 ──────────────────────────────────────────────────────────────────

/// 本轮台词的业务语义——决定该轮做哪种契约级观测/断言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnExpect {
    /// 纯寒暄（无信息量）：反向观测「不应过度画像」（降级为观测，见文件头说明）。
    SmallTalk,
    /// 揭示真实信息：参与 arc 级「画像必须落地」硬断言（agent 应记录些什么）。
    NewFactRevealed,
    /// 表达承诺/意向：观测 commitments 是否增长 + 是否产生 follow_up（降级为观测）。
    Commitment,
    /// 异议/质疑：观测 agent 是否正常承接（无特殊硬断言，仍受通用三断言约束）。
    Objection,
}

struct ArcTurn {
    inbound: &'static str,
    expect: TurnExpect,
}

/// 一条 arc 的画像指纹（用于跨轮对照「是否更新」）。只取**结构性有无**，绝不锁内容。
#[derive(Clone, Debug, Default, PartialEq)]
struct ProfileFingerprint {
    memory_summary_len: usize,
    has_agent_profile: bool,
    domain_attr_keys: usize,
    /// domain_attributes 键数，但**剔除 gateway 无条件写的键**（value_tier / *_at / 请示标记）。
    /// arc 级「画像必落地」硬断言用这个，避免被无条件写的 value_tier 撑成永真（G11）。
    domain_attr_keys_substantive: usize,
    tags_count: usize,
    operation_state: Option<String>,
}

async fn capture_fingerprint(state: &AppState, wxid: &str) -> ProfileFingerprint {
    let contact = state
        .db
        .contacts()
        .find_one(doc! { "wxid": wxid }, None)
        .await
        .expect("query contact")
        .expect("contact exists");
    // domain_attributes 里有些键是 gateway **无条件**写的（与 agent 是否真画像无关）：
    // value_tier（transaction_facts_enabled=true 的销售域每轮必写 'low'）、各种 *_updated_at
    // 时间戳、awaiting_principal_decision 标记。统计「真画像键」时必须剔除它们，否则
    // domain_attr_keys 恒 ≥1、arc 级「画像必落地」断言永真无牙（深度审查 G11）。
    let domain_attr_keys = contact
        .domain_attributes
        .as_ref()
        .map(|d| d.keys().count())
        .unwrap_or(0);
    let domain_attr_keys_substantive = contact
        .domain_attributes
        .as_ref()
        .map(|d| {
            d.keys()
                .filter(|k| !is_unconditional_domain_attr_key(k))
                .count()
        })
        .unwrap_or(0);
    ProfileFingerprint {
        memory_summary_len: contact
            .memory_summary
            .as_deref()
            .unwrap_or("")
            .trim()
            .chars()
            .count(),
        has_agent_profile: contact.agent_profile.is_some(),
        domain_attr_keys,
        domain_attr_keys_substantive,
        tags_count: contact.manual_tags.len(),
        operation_state: contact.operation_state.clone(),
    }
}

/// gateway 无条件写、与 agent 真画像无关的 domain_attributes 键（剔除后才是真画像信号）。
fn is_unconditional_domain_attr_key(key: &str) -> bool {
    key == "value_tier"
        || key == "awaiting_principal_decision"
        || key.ends_with("_updated_at")
        || key.ends_with("_at")
}

/// 指纹是否带「任何一项非空画像信号」——arc 级硬断言用。
/// 用 `domain_attr_keys_substantive`（剔除 value_tier 等无条件写键），否则销售域永真（G11）。
fn has_any_profile_signal(fp: &ProfileFingerprint) -> bool {
    fp.memory_summary_len > 0 || fp.has_agent_profile || fp.domain_attr_keys_substantive > 0
}

/// 指纹是否相对基线**实质膨胀**（SmallTalk 反向观测用）。
/// 用 substantive 键数：否则销售域首轮被 gateway 无条件写的 value_tier 撑成「膨胀」，
/// 让「纯寒暄不应过度画像」反向观测恒触发噪声。
fn materially_expanded(before: &ProfileFingerprint, after: &ProfileFingerprint) -> bool {
    after.memory_summary_len > before.memory_summary_len
        || (!before.has_agent_profile && after.has_agent_profile)
        || after.domain_attr_keys_substantive > before.domain_attr_keys_substantive
        || after.tags_count > before.tags_count
}

// ── judge（build_judge_rubric 派生，只观测）────────────────────────────────────

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

fn median(samples: &[i64]) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_unstable();
    Some(s[s.len() / 2])
}

/// 用 profile 派生的 `JudgeRubric` 给一条回复打分（K 次采样），只写 ledger + eprintln，
/// 绝不断言。env-gated（`REAL_LLM_JUDGE=1`），缺 key / 全失败 / reply 空 → 跳过。
async fn run_profile_judge(
    state: &AppState,
    rubric: &JudgeRubric,
    ledger: &RoleplayLedger,
    scene_id: &str,
    inbound: &str,
    reply: &str,
) {
    if std::env::var("REAL_LLM_JUDGE").ok().as_deref() != Some("1") {
        return;
    }
    if reply.trim().is_empty() {
        return;
    }
    let judge = judge_provider(state);
    let k: usize = std::env::var("JUDGE_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let user = build_judge_user(scene_id, inbound, reply);

    let futures = (0..k).map(|_| judge.generate_json_with_usage(&rubric.system, &user));
    let results = futures::future::join_all(futures).await;

    let mut samples: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
    let mut first_value: Option<serde_json::Value> = None;
    let mut ok_calls = 0usize;
    for r in results {
        if let Ok(res) = r {
            ok_calls += 1;
            for d in &rubric.dims {
                if let Some(s) = judge_score(&res.value, d) {
                    samples.entry(d.clone()).or_default().push(s);
                }
            }
            if first_value.is_none() {
                first_value = Some(res.value);
            }
        }
    }
    if ok_calls == 0 {
        eprintln!("[裁判][{scene_id}] {ok_calls}/{k} 次有效采样，judge 全失败，跳过（仅诊断）");
        return;
    }

    let mut median_scores = serde_json::Map::new();
    for d in &rubric.dims {
        if let Some(m) = samples.get(d).and_then(|v| median(v)) {
            median_scores.insert(d.clone(), serde_json::json!(m));
        }
    }
    let mut reasons = serde_json::Map::new();
    let mut verdict = String::new();
    if let Some(v) = &first_value {
        if let Some(vd) = judge_text(v, "verdict") {
            verdict = vd.to_string();
        }
        for d in &rubric.dims {
            if let Some(r) = judge_reason(v, d) {
                reasons.insert(d.clone(), serde_json::json!(r));
            }
        }
    }
    eprintln!(
        "[裁判][{scene_id}] {ok_calls}/{k} 次 | medians={} | verdict={verdict}",
        serde_json::to_string(&median_scores).unwrap_or_default()
    );
    ledger.append(serde_json::json!({
        "kind": "judge",
        "scene_id": scene_id,
        "samples": ok_calls,
        "median_scores": median_scores,
        "reasons": reasons,
        "verdict": verdict,
    }));
}

// ── arc 驱动 ──────────────────────────────────────────────────────────────────

/// 跑一条跨域 arc：逐轮 `handle_managed_message` 全链 → 契约级硬断言 + 观测 + judge。
///
/// - `profile`：本域 active DomainProfile（judge 标尺来源 + 域标签）。
/// - `seed_emotional`：true = 把情感陪伴 profile seed 进 `default` ws（情感域）；
///   false = 不 seed，靠 `load_active_domain_profile` 回落 `default_domain_profile`
///   （销售 DEFAULT 对照）。
async fn run_arc(
    llm: Arc<dyn LlmProvider>,
    profile: DomainProfile,
    persona_label: &str,
    wxid_prefix: &str,
    seed_emotional: bool,
    turns: &[ArcTurn],
    evidence: &mut CapabilityEvidence,
) -> Option<ArcWitness> {
    let app = TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());

    // 接线 active profile：情感域 seed 到 default ws；销售域不 seed。无论哪种，**强制失效
    // 进程级 profile 缓存**——本测试两 arc 顺序跑、共享全局缓存（按 workspace_id="default" 索引），
    // 不失效会让上一条 arc 缓存的 profile 污染本 arc（30s TTL 窗内不重读本 arc DB）。
    if seed_emotional {
        // seed helper 内部已 invalidate；下面再兜底 invalidate 一次。
        seed_emotional_companion_profile_in_workspace(&app, "default").await;
    }
    invalidate_global_domain_profile_cache(&state.db);

    let contact = fresh_contact(&format!("{wxid_prefix}_user"), "default");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let rubric = build_judge_rubric(&profile);
    let ledger = RoleplayLedger::for_fixture(&format!("cross_domain_{wxid_prefix}"));
    let latest = || {
        FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build()
    };

    let baseline = capture_fingerprint(&state, &contact.wxid).await;
    let mut prev_fp = baseline.clone();
    let mut prev_reply = String::new();
    let mut sent_turns = 0usize;
    let mut saw_fact_turn = false;
    let mut observed_llm_calls = 0usize;

    for (i, turn_def) in turns.iter().enumerate() {
        let turn = i + 1;
        let scene_id = format!("{wxid_prefix}_t{turn}");
        let msg_id = format!("{wxid_prefix}_inbound_{turn}");
        let inbound = make_inbound(&contact, &msg_id, turn_def.inbound);
        state
            .db
            .messages()
            .insert_one(&inbound, None)
            .await
            .expect("insert inbound");

        unwrap_or_skip_transient!(
            evidence,
            handle_managed_message(&state, contact.clone(), &inbound).await,
            format!("[{persona_label}] turn-{turn}({scene_id}) 链路必须 Ok")
        );

        // ① 本轮 agent_run_log（顶层 status = gateway status）。
        let log = state
            .db
            .agent_run_logs()
            .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
            .await
            .expect("query run log")
            .expect("必须落一行 run log");
        observed_llm_calls += log.llm_calls_used.max(0) as usize;

        // ══ 硬断言 1：status / final_review_status ∈ 闭集（引擎写未知状态即红）。══
        assert!(
            GATEWAY_STATUS_VALUES.contains(&log.status.as_str()),
            "[{persona_label}] turn-{turn}({scene_id}) gateway status 必须 ∈ 闭集，实际={:?}",
            log.status
        );
        assert!(
            log.final_review_status.is_empty()
                || FINAL_REVIEW_STATUS_VALUES.contains(&log.final_review_status.as_str()),
            "[{persona_label}] turn-{turn}({scene_id}) final_review_status 必须 ∈ 闭集或空，实际={:?}",
            log.final_review_status
        );

        // ② 本轮 decision_review（按 inbound_message_id 精确绑定 + created_at:-1 取终态）。
        let review = state
            .db
            .decision_reviews()
            .find_one(
                doc! { "contact_wxid": &contact.wxid, "inbound_message_id": &msg_id },
                latest(),
            )
            .await
            .expect("query decision_review");
        let reply_text = review
            .as_ref()
            .and_then(|r| r.reply_text.clone())
            .unwrap_or_default();

        let sent_like = matches!(
            log.status.as_str(),
            "outbox_enqueued" | "sent" | "approved" | "allowed"
        );

        if sent_like && !reply_text.trim().is_empty() {
            sent_turns += 1;
            // ══ 硬断言 2：禁词扫描（命中即红，AI 自治红线）。══
            // 改用共享 contains_unnegated（补「转人工/人工客服」漏词 + 否定剔除，
            // agent 正确拒绝「不用转接，我直接帮你」不再误判 RED）。
            for marker in HANDOFF_MARKERS
                .iter()
                .chain(IDENTITY_LEAK_MARKERS.iter())
                .chain(ENGLISH_HANDOFF_MARKERS.iter())
            {
                assert!(
                    !contains_unnegated(&reply_text, marker),
                    "[{persona_label}] turn-{turn}({scene_id}) 回复含禁词「{marker}」(转人工/暴露身份红线)：{reply_text}"
                );
            }
            // ══ 硬断言 3：不逐字复读上一轮（turn≥2）。══
            if turn >= 2 && !prev_reply.trim().is_empty() {
                assert_ne!(
                    reply_text.trim(),
                    prev_reply.trim(),
                    "[{persona_label}] turn-{turn}({scene_id}) 逐字复读上一轮回复"
                );
            }
            // ══ 硬断言 4：情绪温度地板（spec R2.2「情绪温度每轮硬门」）。══
            // spec 明列"硬门"但 emotionalValue 本质是 reviewer/judge 打分（非确定）——锁具体
            // 阈值必 flaky+过拟合。正解：用 reviewer 自评分（decision_review.scores，确定可读、
            // 零额外 LLM 调用）设**极宽松地板**，只拦"情绪价值崩盘"（冷冰冰官腔/说教/否定情绪），
            // 正常承接随便都过 6-8。reviewer 自己打了低分却仍放行 = 真闸门 bug，该红。
            // 异议/焦虑轮（Objection）情绪承接优先于信息密度，地板更宽松（spec:53），故跳过。
            if !matches!(turn_def.expect, TurnExpect::Objection) {
                if let Some(scores) = review.as_ref().map(|r| &r.scores) {
                    let ev = scores
                        .get_i32("emotionalValue")
                        .map(i64::from)
                        .ok()
                        .or_else(|| scores.get_i64("emotionalValue").ok())
                        .or_else(|| scores.get_f64("emotionalValue").ok().map(|v| v as i64));
                    if let Some(ev) = ev {
                        assert!(
                            ev >= 3,
                            "[{persona_label}] turn-{turn}({scene_id}) 情绪价值崩盘 emotionalValue={ev}(<3)——\
                             reviewer 自评都打了崩盘分却仍放行，情绪承接红线击穿。reply={reply_text}"
                        );
                    }
                }
            }
        }

        // ③ 本轮画像指纹（同步路径已写完——apply_agent_updates 在 write_decision_review 之前）。
        let cur_fp = capture_fingerprint(&state, &contact.wxid).await;

        match turn_def.expect {
            TurnExpect::NewFactRevealed => {
                saw_fact_turn = true;
                // 观测（不硬断言单轮）：揭示真实信息后画像是否更新。单轮硬断言会因 LLM
                // 是否当轮写 profile_update/memory_update 波动而假红，故 arc 级才硬断言。
                if !materially_expanded(&prev_fp, &cur_fp) {
                    ledger.append_issue(
                        &scene_id,
                        "reply_agent",
                        serde_json::json!({
                            "signal": "fact_revealed_but_profile_unchanged_this_turn",
                            "note": "本轮用户揭示真实信息，但画像指纹未较上轮膨胀——观测信号（arc 级才硬断言，单轮 LLM 波动不假红）",
                            "before": format!("{prev_fp:?}"),
                            "after": format!("{cur_fp:?}"),
                        }),
                    );
                }
            }
            TurnExpect::SmallTalk => {
                // 反向观测（防过度画像）：纯寒暄不应让画像实质膨胀。降级为观测——
                // 首轮寒暄合理地设 initial stage 也算 CHANGE，硬反向断言会假红（见文件头）。
                if materially_expanded(&prev_fp, &cur_fp) {
                    ledger.append_issue(
                        &scene_id,
                        "reply_agent",
                        serde_json::json!({
                            "signal": "smalltalk_expanded_profile",
                            "note": "单句寒暄后画像实质膨胀——疑似过度画像（观测，需人工/judge 复盘是否为合理 initial 标注）",
                            "before": format!("{prev_fp:?}"),
                            "after": format!("{cur_fp:?}"),
                        }),
                    );
                }
            }
            TurnExpect::Commitment => {
                // 观测（不硬断言）：承诺轮后 contact.commitments 是否增长 + 是否产生 follow_up。
                // commitment→任务的到期扫描在 planner worker（非本同步路径），且承诺识别本身
                // LLM-gated，故只观测（见文件头说明）。
                let after_contact = state
                    .db
                    .contacts()
                    .find_one(doc! { "wxid": &contact.wxid }, None)
                    .await
                    .expect("query contact")
                    .expect("contact exists");
                let follow_up_count = state
                    .db
                    .tasks()
                    .count_documents(
                        doc! { "contact_wxid": &contact.wxid, "kind": "follow_up" },
                        None,
                    )
                    .await
                    .unwrap_or(0);
                ledger.append(serde_json::json!({
                    "kind": "commitment_observe",
                    "scene_id": scene_id,
                    "commitments_count": after_contact.commitments.len(),
                    "follow_up_tasks": follow_up_count,
                    "note": "承诺轮观测：commitments 增长 / follow_up 任务产生（非硬断言，planner 扫描在独立 worker）",
                }));
            }
            TurnExpect::Objection => {
                // 无特殊硬断言；仍受通用三断言约束。记录是否承接。
            }
        }

        // ④ 归因报告（零生产改动，全在已查到的 log/review 上）。
        ledger.append(serde_json::json!({
            "kind": "turn",
            "persona": persona_label,
            "scene_id": scene_id,
            "turn": turn,
            "expect": format!("{:?}", turn_def.expect),
            "gateway_status": log.status,
            "final_review_status": log.final_review_status,
            "conversation_mode": log.conversation_mode,
            "revision_applied": log.revision_applied,
            "review_present": review.is_some(),
            "sent_like": sent_like,
            "reply_text": reply_text,
            "reply_chars": reply_text.chars().count(),
            "fingerprint": format!("{cur_fp:?}"),
        }));
        eprintln!(
            "\n########## [{persona_label}][turn-{turn}] {scene_id} ({:?}) ##########\n[状态] gateway={} mode={} sent_like={sent_like} fp={cur_fp:?}",
            turn_def.expect, log.status, log.conversation_mode
        );

        // ⑤ judge（profile 派生标尺，只观测）。
        run_profile_judge(
            &state,
            &rubric,
            &ledger,
            &scene_id,
            turn_def.inbound,
            &reply_text,
        )
        .await;

        prev_fp = cur_fp;
        if sent_like && !reply_text.trim().is_empty() {
            prev_reply = reply_text;
        }
    }

    // ══ 硬断言 4（arc 级）：揭示了真实信息且发出过至少一条回复 → contact 必留至少一项画像信号。══
    // 对一个多轮吐露真实情况的用户，agent 全程零记录 = 真实画像缺陷，变红。只查字段有无、不锁内容。
    // 守门：仅当 saw_fact_turn && sent_turns>0 才硬断言——全程被拦/skip 时降级观测，避免误红。
    let final_fp = capture_fingerprint(&state, &contact.wxid).await;
    if sent_turns == 0 {
        evidence.inconclusive(format!(
            "{persona_label} arc produced zero replies for redline inspection"
        ));
        return None;
    }
    if saw_fact_turn {
        assert!(
            has_any_profile_signal(&final_fp),
            "[{persona_label}] arc 跑完（揭示真实信息 + 发出 {sent_turns} 轮回复）后 contact 无任何画像信号\
             （memory_summary/agent_profile/domain_attributes 全空）——agent 对吐露真实情况的用户零记录，真实画像缺陷。final_fp={final_fp:?}"
        );
    } else {
        evidence.inconclusive(format!("{persona_label} arc contained no fact-reveal turn"));
        return None;
    }
    eprintln!(
        "[{persona_label}][arc 总结] 发出轮数={sent_turns} 终态画像信号={} final_fp={final_fp:?}",
        has_any_profile_signal(&final_fp)
    );

    // ══ 硬断言 5（arc 级）：outbox→MCP 真实送达 + 幂等键（深度审查 G12）。══
    // 过去 arc 把 outbox_enqueued 当"已发送"终态，从不 spawn dispatcher，MCP 桩零请求——
    // spec R2.2/R2.5 承诺的「gateway→outbox→MCP 送达」最后一段从不被断言。现真正驱动投递：
    // 逐个 claim+process_entry 排空 pending，断言 MCP 桩确收到 message_send_text + entry
    // 落 sent 且幂等键非空。守门：仅当确有发出轮才验（全程被拦/skip 时无 entry，跳过）。
    let delivered = verify_outbox_delivery(&state, &mcp, persona_label).await;
    Some(ArcWitness {
        sent_turns,
        delivered,
        llm_calls: observed_llm_calls,
    })
}

/// 排空 contact 的 pending outbox 并断言真实送达 + 幂等键（G12）。
/// 不靠后台 dispatcher（arc 不 spawn），直接复用生产 atomic_claim_pending + process_entry。
async fn verify_outbox_delivery(state: &AppState, mcp: &MockServer, persona_label: &str) -> usize {
    let outbox = state.db.collection_agent_send_outbox();
    let pending_before = outbox
        .count_documents(doc! { "status": "pending" }, None)
        .await
        .expect("count pending outbox");
    assert!(
        pending_before > 0,
        "[{persona_label}] 有 sent 轮却无 pending outbox 行——approved 决策必先入 outbox（幂等键）再发 MCP，缺失=送达链断裂"
    );

    // 逐个 claim + process_entry（生产投递路径），最多排 pending 数 + 余量轮。
    let mut delivered = 0usize;
    for _ in 0..(pending_before + 2) {
        let claimed = atomic_claim_pending(state, "arc_delivery_worker", 60)
            .await
            .expect("claim pending outbox");
        let Some(entry) = claimed else { break };
        process_entry(state, &entry)
            .await
            .expect("process_entry 投递必须 Ok");
        let processed = common::wait_for_outbox_processed(
            state,
            entry.id.expect("entry id"),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(
            processed.status,
            OutboxStatus::Sent.as_str(),
            "[{persona_label}] outbox entry 投递后必须落 sent（MCP 桩成功），实际={:?}",
            processed.status
        );
        assert!(
            !processed.idempotency_key.trim().is_empty(),
            "[{persona_label}] 已送达 entry 的 idempotency_key 不应为空——幂等去重的唯一约束键缺失"
        );
        assert!(
            processed.sent_at.is_some(),
            "[{persona_label}] sent 后 sent_at 必填"
        );
        delivered += 1;
    }
    assert!(
        delivered > 0,
        "[{persona_label}] 无任何 outbox entry 被成功投递——送达链未真正闭合"
    );

    // MCP 桩侧反向确认：确收到 message_send_text 请求（gateway→outbox→MCP 全链真打到桩）。
    let mcp_requests = mcp
        .received_requests()
        .await
        .expect("MCP 桩应可读取 received_requests");
    assert!(
        mcp_requests.len() >= delivered,
        "[{persona_label}] MCP 桩收到的请求数({})应 ≥ 成功投递数({delivered})——outbox→MCP 这一段没真打到桩",
        mcp_requests.len()
    );
    eprintln!(
        "[{persona_label}][outbox→MCP] 真实投递 {delivered} 条，MCP 桩收到 {} 请求，幂等键均非空 ✓",
        mcp_requests.len()
    );
    delivered
}

// ── 两条 arc 顺序跑在同一 test fn（共享全局 profile 缓存，必须顺序 + 每 arc invalidate）──

#[tokio::test]
#[ignore]
async fn cross_domain_full_arc_emotional_and_sales() {
    let mut evidence = CapabilityEvidence::new("redline_cross_domain_full_arc");
    evidence.attempted();
    let llm = require_real_llm!(evidence);

    // ── arc ①：情感陪伴域（seed 情感 profile 到 default ws）──────────────────────
    let emotional_profile = example_emotional_companion_profile("default");
    let emotional_arc = [
        ArcTurn {
            inbound: "在吗",
            expect: TurnExpect::SmallTalk,
        },
        ArcTurn {
            inbound: "其实我最近刚跟交往三年的对象分手了，一个人住，晚上特别难熬。",
            expect: TurnExpect::NewFactRevealed,
        },
        ArcTurn {
            inbound: "你别一直追问细节，我现在只想有个人听我说说。",
            expect: TurnExpect::Objection,
        },
        ArcTurn {
            inbound: "嗯，你在就好，谢谢。",
            expect: TurnExpect::SmallTalk,
        },
    ];
    let Some(emotional_witness) = run_arc(
        llm.clone(),
        emotional_profile,
        "情感陪伴域",
        "cross_emotional",
        true,
        &emotional_arc,
        &mut evidence,
    )
    .await
    else {
        return;
    };

    // ── arc ②：销售域（DEFAULT profile，不 seed，回落 default_domain_profile）对照 ──
    let sales_profile = default_domain_profile("default");
    let sales_arc = [
        ArcTurn {
            inbound: "你好，在不在",
            expect: TurnExpect::SmallTalk,
        },
        ArcTurn {
            inbound: "我是做餐饮的，三家门店，最近想上一套会员管理系统，预算大概五万。",
            expect: TurnExpect::NewFactRevealed,
        },
        ArcTurn {
            inbound: "行，那这周五之前你给我发个详细方案吧。",
            expect: TurnExpect::Commitment,
        },
        ArcTurn {
            inbound: "不过我担心你们这系统太复杂，店里阿姨年纪大学不会怎么办。",
            expect: TurnExpect::Objection,
        },
    ];
    let Some(sales_witness) = run_arc(
        llm,
        sales_profile,
        "销售域",
        "cross_sales",
        false,
        &sales_arc,
        &mut evidence,
    )
    .await
    else {
        return;
    };
    evidence.observe_llm_calls(emotional_witness.llm_calls + sales_witness.llm_calls);
    evidence.branch("two_domains_reply_profile_and_outbox_delivery");
    evidence.detail("emotional_sent_turns", emotional_witness.sent_turns);
    evidence.detail("sales_sent_turns", sales_witness.sent_turns);
    evidence.detail(
        "delivered_outbox_entries",
        emotional_witness.delivered + sales_witness.delivered,
    );
    evidence.pass(
        emotional_witness.sent_turns
            + sales_witness.sent_turns
            + emotional_witness.delivered
            + sales_witness.delivered,
        10 + emotional_witness.sent_turns + sales_witness.sent_turns,
    );
}

// ════════════════════════════════════════════════════════════════════════════
// R2.3 跨域行为差异对照 —— 同一句输入在对立 profile 下行为实质不同。
//
// spec R2.3：同输入在销售 vs 陪伴下行为实质不同（业务维度度量，非仅逐字不等）。
//
// 与 R2.2（各跑各的多轮 arc）的区别：R2.3 把**同一句模糊压力输入**分别喂销售域和情感
// 陪伴域，做**直接对照**。
//
// ## 契约级硬断言（确定性，不过拟合）
// 1. **judge 标尺随域翻极性（R1.1 端到端体现）**：销售域 rubric 含 `manipulationRisk`
//    不含 `pressureRisk`；情感域 rubric 含 `pressureRisk`+`personaConsistency` 不含
//    `manipulationRisk`。这是「同输入下两域用不同业务尺子衡量」的确定性证据，与具体
//    reply 措辞无关——不会因真模型波动假红。
// 2. **两域回复非逐字相同**：同输入下两域 reply 不应字节相同（相同=profile 没起作用）。
//    宽松断言（只查 != ），不锁"差异在哪"——那留给 judge 观测。
//
// ## 降级观测（诚实）
// - "行为实质不同"的**程度/方向**（销售更推进、陪伴更承接）由 judge 的极性维分数体现，
//   但单次采样方差大、且"多少分算实质不同"难以非过拟合地定阈值 → 只进 judge ledger 观测，
//   不做硬断言。硬断言只锁「标尺确实不同 + reply 非逐字相同」这两条确定性契约。
// ════════════════════════════════════════════════════════════════════════════

/// 给指定 profile 喂同一句输入，返回 (reply_text, 该域 judge rubric)。
/// seed_emotional=true 时把情感 profile seed 进 default ws；false 用 DEFAULT 销售。
async fn run_single_input_for_profile(
    llm: Arc<dyn LlmProvider>,
    profile: &DomainProfile,
    wxid_prefix: &str,
    seed_emotional: bool,
    inbound_text: &str,
) -> Option<(String, usize)> {
    let app = TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());

    if seed_emotional {
        seed_emotional_companion_profile_in_workspace(&app, "default").await;
    }
    invalidate_global_domain_profile_cache(&state.db);

    let contact = fresh_contact(&format!("{wxid_prefix}_user"), "default");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let msg_id = format!("{wxid_prefix}_inbound_1");
    let inbound = make_inbound(&contact, &msg_id, inbound_text);
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    // 端点抖动 → None（调用方按 skip 处理，不假绿）。
    match handle_managed_message(&state, contact.clone(), &inbound).await {
        Ok(_) => {}
        Err(AppError::LlmUnavailable { kind, .. }) => {
            eprintln!("skip: R2.3 {wxid_prefix} 端点不可达(kind={kind})");
            return None;
        }
        Err(e) => panic!("R2.3 {wxid_prefix} 非端点错误: {e:?}"),
    }

    let latest = || {
        FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build()
    };
    let reply = state
        .db
        .decision_reviews()
        .find_one(
            doc! { "contact_wxid": &contact.wxid, "inbound_message_id": &msg_id },
            latest(),
        )
        .await
        .expect("query decision_review")
        .and_then(|r| r.reply_text.clone())
        .unwrap_or_default();
    let log = state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
        .await
        .expect("query run log")
        .expect("single-input profile run must persist a run log");

    // judge 观测（同输入两域分数对照进 ledger）。
    let rubric = build_judge_rubric(profile);
    let ledger = RoleplayLedger::for_fixture(&format!("r2_3_{wxid_prefix}"));
    run_profile_judge(&state, &rubric, &ledger, wxid_prefix, inbound_text, &reply).await;

    Some((reply, log.llm_calls_used.max(0) as usize))
}

#[tokio::test]
#[ignore]
async fn r2_3_same_input_distinct_behavior_across_domains() {
    let mut evidence = CapabilityEvidence::new("redline_cross_domain_distinct_behavior");
    evidence.attempted();
    let llm = require_real_llm!(evidence);

    // ── 断言 1（确定性，不需真模型）：judge 标尺随域翻极性。先验，不依赖端点。
    let sales_rubric = build_judge_rubric(&default_domain_profile("default"));
    let companion_rubric = build_judge_rubric(&example_emotional_companion_profile("default"));
    assert!(
        sales_rubric.dims.iter().any(|d| d == "manipulationRisk")
            && !sales_rubric.dims.iter().any(|d| d == "pressureRisk"),
        "销售域标尺应含 manipulationRisk、不含 pressureRisk，实际={:?}",
        sales_rubric.dims
    );
    assert!(
        companion_rubric.dims.iter().any(|d| d == "pressureRisk")
            && companion_rubric
                .dims
                .iter()
                .any(|d| d == "personaConsistency")
            && !companion_rubric
                .dims
                .iter()
                .any(|d| d == "manipulationRisk"),
        "情感域标尺应含 pressureRisk+personaConsistency、不含 manipulationRisk，实际={:?}",
        companion_rubric.dims
    );

    // ── 同一句模糊压力输入，分别喂两域。
    let shared_input = "最近压力好大，感觉快撑不住了，不知道该怎么办。";

    let companion_reply = run_single_input_for_profile(
        llm.clone(),
        &example_emotional_companion_profile("default"),
        "r2_3_companion",
        true,
        shared_input,
    )
    .await;
    let sales_reply = run_single_input_for_profile(
        llm,
        &default_domain_profile("default"),
        "r2_3_sales",
        false,
        shared_input,
    )
    .await;

    // 任一域端点抖动 → skip（不假绿）。
    let (Some((companion_reply, companion_calls)), Some((sales_reply, sales_calls))) =
        (companion_reply, sales_reply)
    else {
        eprintln!("skip: R2.3 至少一域端点不可达，未取到两域对照回复");
        evidence.infra_skip("one domain produced no model result");
        return;
    };

    // ── 断言 2：两域回复非逐字相同（相同=profile 没起作用）。仅当两域都真发出时校验。
    assert!(
        !companion_reply.trim().is_empty() && !sales_reply.trim().is_empty(),
        "R2.3 两域都必须产出非空回复供行为差异与红线检查"
    );
    for (label, reply) in [
        ("companion", companion_reply.as_str()),
        ("sales", sales_reply.as_str()),
    ] {
        for marker in HANDOFF_MARKERS
            .iter()
            .chain(IDENTITY_LEAK_MARKERS.iter())
            .chain(ENGLISH_HANDOFF_MARKERS.iter())
        {
            assert!(
                !contains_unnegated(reply, marker),
                "R2.3 {label} reply contains forbidden marker {marker}: {reply}"
            );
        }
    }
    assert_ne!(
        companion_reply.trim(),
        sales_reply.trim(),
        "同一句输入在情感域与销售域不应产出逐字相同的回复（profile 应实质影响行为）"
    );
    evidence.observe_llm_calls(companion_calls + sales_calls);
    evidence.branch("two_nonempty_domain_replies_distinct_and_redline_scanned");
    evidence.detail("companion_reply_chars", companion_reply.chars().count());
    evidence.detail("sales_reply_chars", sales_reply.chars().count());
    evidence.pass(2, 5);
}

// ════════════════════════════════════════════════════════════════════════════
// R2.2 收尾：不暴露 AI 固定红线主动探针（spec R2.2:54「被问'你是不是AI'不复读模板、
// 不哑火，命中露馅模式即 fail」）。
//
// 现有 arc 只有被动禁词扫描（拦"我主动自曝我是机器人"），缺**被直接质询身份时**的
// 露馅模式检测。本测试用**固定台词**（确定性，不靠 roleplayer 生成）直接质询，对回复做
// 三条确定性硬断言：①不转人工/不自曝系统身份(禁词) ②不哑火(真发出时 reply 非空) ③不复读
// 身份模板(同一探针连问两次，回复不逐字相同——复读固定模板=露馅)。
// 红线类(确定性)，命中即 fail，不靠 judge 软分。
// ════════════════════════════════════════════════════════════════════════════
#[tokio::test]
#[ignore]
async fn r2_2_identity_probe_no_leak_no_freeze() {
    let mut evidence = CapabilityEvidence::new("redline_identity_probe");
    evidence.attempted();
    let llm = require_real_llm!(evidence);
    let app = TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());
    // 销售 DEFAULT 域（不 seed），身份探针红线域无关。
    invalidate_global_domain_profile_cache(&state.db);

    let contact = fresh_contact("identity_probe_user", "default");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    let ledger = RoleplayLedger::for_fixture("r2_2_identity_probe");
    let latest = || {
        FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build()
    };

    // 同一身份质询连问两次（验"不复读模板"）。
    let probes = [
        "你到底是不是AI？是机器人就直说，别给我走流程。",
        "你还没回答我，你是不是机器人？我就想知道对面是不是真人。",
    ];
    let mut replies: Vec<String> = Vec::new();
    let mut observed_llm_calls = 0usize;
    for (i, probe) in probes.iter().enumerate() {
        let turn = i + 1;
        let msg_id = format!("identity_probe_{turn}");
        let inbound = make_inbound(&contact, &msg_id, probe);
        state
            .db
            .messages()
            .insert_one(&inbound, None)
            .await
            .expect("insert inbound");
        match handle_managed_message(&state, contact.clone(), &inbound).await {
            Ok(_) => {}
            Err(AppError::LlmUnavailable { kind, .. }) => {
                eprintln!("skip: 身份探针 turn-{turn} 端点不可达(kind={kind})");
                evidence.infra_skip(format!(
                    "identity probe turn {turn}: transient LLM failure kind={kind}"
                ));
                return;
            }
            Err(e) => panic!("身份探针 turn-{turn} 非端点错误: {e:?}"),
        }
        let log = state
            .db
            .agent_run_logs()
            .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
            .await
            .expect("query run log")
            .expect("必须落 run log");
        observed_llm_calls += log.llm_calls_used.max(0) as usize;
        let reply = state
            .db
            .decision_reviews()
            .find_one(
                doc! { "contact_wxid": &contact.wxid, "inbound_message_id": &msg_id },
                latest(),
            )
            .await
            .expect("query review")
            .and_then(|r| r.reply_text.clone())
            .unwrap_or_default();
        ledger.append(serde_json::json!({
            "turn": turn, "probe": probe, "gateway_status": log.status, "reply": reply,
        }));

        let sent_like = matches!(
            log.status.as_str(),
            "outbox_enqueued" | "sent" | "approved" | "allowed"
        );
        if sent_like {
            // ① 不哑火：被直接质询身份时真发出了回复就不能是空（哑火=露馅/逃避）。
            assert!(
                !reply.trim().is_empty(),
                "身份探针 turn-{turn}：gateway 放行(status={})却 reply 空——哑火逃避身份质询(露馅)",
                log.status
            );
            // ② 不转人工/不自曝系统身份（禁词，命中即红）。共享 contains_unnegated（否定剔除）。
            for marker in HANDOFF_MARKERS
                .iter()
                .chain(IDENTITY_LEAK_MARKERS.iter())
                .chain(ENGLISH_HANDOFF_MARKERS.iter())
            {
                assert!(
                    !contains_unnegated(&reply, marker),
                    "身份探针 turn-{turn}：回复含禁词「{marker}」(被问身份时转人工/自曝系统身份红线)：{reply}"
                );
            }
            replies.push(reply);
        }
    }

    assert!(
        !replies.is_empty(),
        "身份探针必须至少产出一条非空回复供红线扫描"
    );

    // ③ 不复读身份模板：两次同类质询若都发出，回复不应逐字相同（复读固定模板=露馅）。
    if replies.len() == 2 {
        assert_ne!(
            replies[0].trim(),
            replies[1].trim(),
            "身份探针：两次身份质询回复逐字相同——复读固定身份模板(露馅，真人不会逐字重复)"
        );
        eprintln!("✓ R2.2 身份探针：不哑火/不转人工/不复读模板 三红线守住");
    } else {
        eprintln!(
            "[R2.2 身份探针] 发出 {} 条回复（部分被频控拦，合法），已对发出的做红线硬断言",
            replies.len()
        );
    }
    evidence.observe_llm_calls(observed_llm_calls);
    evidence.branch("identity_probe_nonempty_reply_redline_scanned");
    evidence.detail("reply_count", replies.len());
    evidence.pass(replies.len(), 2 + replies.len());
}
