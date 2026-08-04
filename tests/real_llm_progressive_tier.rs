//! `real_llm_progressive_tier` —— **渐进式三档提示词加载 + 信息充分性自评**机制的真实
//! 大模型端到端回归（独立套件）。
//!
//! 被测机制（2026-06-23 落地，本文件只测不改生产）：主回复路径 `handle_managed_message`
//! 现在是「两程循环」——
//! 1. 第一程用【小档 Lean】瘦 prompt 生成回复候选 + 一个【充分性自评】
//!    （`AgentDecision.sufficiency` ∈ `enough` / `need_more_context` / `need_clarification`）。
//! 2. 纯函数 `decide_tier_escalation(&decision)` 读自评分支：
//!    - `Enough`   → 直接进五闸（多数寒暄轮）；
//!    - `Escalate(tier)` → 升档第二程重生成，gateway 写一条 `agent_events`，
//!      `kind="ptier_escalated"`，`details` 含 `run_id` + `target_tier`（gateway.rs:973）；
//!    - `Clarify` → 输出澄清向回复，gateway 写一条 `agent_events`，
//!      `kind="ptier_clarify"`，`details` 含 `run_id`（gateway.rs:1007）。
//!
//! ## 断言策略（关键：抗真模型抖动、不过拟合到单次输出）
//! 不解析回复文本，而是查 `agent_events` 集合里有没有对应 `kind` 的事件来验证三档行为：
//! - **寒暄轮**（"在吗"/"你好"）：期望走 `Enough` → **不应**出现 `ptier_escalated`。
//!   这是机制核心价值（寒暄不该吞重型槽位）——相对稳，作**硬断言**。
//! - **产品/价格问询轮**：期望第一程小档自评 `need_more_context` → 出现 `ptier_escalated`
//!   且 `target_tier` 含 `"Full"`——真模型不保证每次都这么判，作**软断言 + eprintln 观测**。
//! - **含糊轮**（只发"这个"无指代）：期望可能走 `Clarify` → 可能出现 `ptier_clarify`——
//!   同样**软断言 + eprintln 观测**（拿到事件则 assert 形状，没拿到只 warn 不 panic）。
//!
//! ## 红线（与 real_llm_ops_smoke 同口径）
//! - **MCP 永远是桩**：`rebuild_app_state_with_real_llm` 把 `mcp_base_url` 指向 wiremock，
//!   绝不真发微信。
//! - **密钥零泄漏**：只从 env 读 `REAL_LLM_API_KEY`，断言信息不打印 key。
//! - **env-gated**：无 `REAL_LLM_API_KEY` 时每个真模型 test 自我跳过（eprintln + return），
//!   不 panic；默认 `#[ignore]`，本地 `cargo test` 不触网。
//! - 末尾另有一个**不带 `#[ignore]`** 的纯函数分支测试（`decide_tier_escalation` 三分支），
//!   无需 LLM / Docker，本地 `cargo test --test real_llm_progressive_tier` 即可跑。
//!
//! ## 运行（真模型部分）
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=... \
//!   cargo test --test real_llm_progressive_tier -- --ignored --nocapture
//! ```
//! 真模型 test 需要 Docker（testcontainers Mongo），由 GitHub CI 的 `real-llm` job 驱动。

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::agent::handle_managed_message;
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{LlmClient, LlmFormat, LlmJsonResult, LlmProvider};
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};

use crate::common::TestApp;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── env-gated 真实 provider 构造（与 real_llm_ops_smoke.rs 同口径，各自持一份副本）──
//
// 这些 failover/provider 辅助函数在姊妹真模型套件之间就是各自复制一份（测试文件互不
// import 跨文件私有项），故此处原样复制需要的几个。

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

/// 该错误是否值得切下一个备胎（端点侧抖动 = 换独立端点可能成功）。
fn is_failover_worthy(e: &AppError) -> bool {
    match e {
        AppError::LlmUnavailable { kind, detail, .. } => match kind.as_str() {
            "http_4xx" => detail.contains("HTTP 402") || detail.contains("HTTP 401"),
            _ => wechatagent::llm::is_transient_llm_unavailable_kind(kind),
        },
        AppError::Http(h) => h.is_timeout() || h.is_connect(),
        _ => false,
    }
}

/// 顺序 failover provider：`clients = [主, 备1, 备2, ...]`（已按延迟升序）。
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

/// FAILOVER key 是否已配。
fn failover_key_present() -> bool {
    std::env::var("REAL_LLM_FAILOVER_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .is_some()
}

/// 主模型重试预算：统一 10 次指数退避熬过限流窗。
fn primary_max_retries() -> u32 {
    10
}

/// 构造最强模型 client（llama-3.3-70b @ NVIDIA integrate，OpenAI 兼容）。缺 key → None。
/// 兼作 agent 备胎链首选。
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

/// 备胎 model 名列表（逗号分隔异族链）。
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

/// 从 env 构造备胎链（延迟/能力升序）：①最强模型 →②NVIDIA 异族链。两 key 全缺 → vec![]
/// （FailoverProvider 退化为只主模型，零回归）。
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

/// 把主 client 包成带备胎链的 `FailoverProvider`（备胎缺失时退化为只主）。
fn wrap_with_failover(primary_label: String, primary: Arc<LlmClient>) -> Arc<dyn LlmProvider> {
    let mut clients = vec![primary];
    clients.extend(failover_backups());
    Arc::new(FailoverProvider {
        primary_label,
        clients,
    })
}

/// 主模型 + 备胎链 → `Arc<dyn LlmProvider>`。缺主 key → None。
fn real_llm_with_failover() -> Option<Arc<dyn LlmProvider>> {
    let primary = real_llm_from_env()?;
    let primary_label =
        std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    Some(wrap_with_failover(primary_label, primary))
}

/// 跳过宏：无 key 时打印一行 skip 并 `return`（不 panic、不算失败）。
macro_rules! require_real_llm {
    () => {{
        match real_llm_with_failover() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过真实大模型渐进式三档行为测试");
                return;
            }
        }
    }};
}

/// 链路解包宏：真模型上游瞬时不可达（限流/超时等 `LlmUnavailable`）→ 打印 skip 并
/// `return`，不算能力失败；配置错误（4xx 非 401/402）仍 panic；其它 Err 也 panic。
macro_rules! unwrap_or_skip_transient {
    ($result:expr, $what:expr) => {{
        match $result {
            Ok(value) => value,
            Err(wechatagent::error::AppError::LlmUnavailable { kind, retry_count, detail, .. }) => {
                if !wechatagent::llm::is_transient_llm_unavailable_kind(&kind) {
                    panic!(
                        "{}：非瞬时 LLM 错误（kind={kind}），不得 skip 假绿。detail={detail}",
                        $what
                    );
                }
                eprintln!(
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），\
                     按「真模型抖动有限重试+跳过」处理，不算能力失败",
                    $what
                );
                return;
            }
            Err(other) => panic!("{}：{other:?}", $what),
        }
    }};
}

// ── MCP 桩（绝不真发微信）────────────────────────────────────────────────────
// gateway 把 newMsgId 写进 conversation_messages.message_id（sparse+unique 索引），
// 同 id 会撞 E11000，故逐请求递增。
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
            "result": {
                "structuredContent": {
                    "newMsgId": format!("real_ptier_msg_{seq}"),
                    "content": []
                }
            }
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

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("真实三档 smoke 客户".to_string()),
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
        operation_state: Some("new_contact".to_string()),
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

// ── 事件查询 helper ──────────────────────────────────────────────────────────

/// 查某 contact 是否落了某 `kind` 的 ptier 事件（取最新一条）。返回 Some(details) 或 None。
async fn find_ptier_event(
    state: &wechatagent::routes::AppState,
    wxid: &str,
    kind: &str,
) -> Option<wechatagent::models::AgentEvent> {
    use mongodb::options::FindOneOptions;
    let opts = FindOneOptions::builder()
        .sort(doc! { "created_at": -1 })
        .build();
    state
        .db
        .events()
        .find_one(doc! { "contact_wxid": wxid, "kind": kind }, opts)
        .await
        .expect("query ptier event")
}

// ════════════════════════════════════════════════════════════════════════════
// P1 · 寒暄轮 → Enough（**不应**升档）—— 机制核心价值的硬断言
// ════════════════════════════════════════════════════════════════════════════

/// 客户发一句纯寒暄（"在吗"），第一程小档应自评 `enough` → `Enough` 分支直接进闸，
/// **不应**写 `ptier_escalated` 事件（小档就够，寒暄不该吞重型槽位）。这是渐进式三档
/// 机制的核心价值，相对稳定 → 作**硬断言**。
#[tokio::test]
#[ignore]
async fn p1_greeting_stays_lean_no_escalation() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());

    let contact = managed_contact("real_ptier_user_greet");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let inbound = make_inbound(&contact, "real_ptier_msg_greet", "在吗");
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    unwrap_or_skip_transient!(
        handle_managed_message(&state, contact.clone(), &inbound).await,
        "寒暄轮链路必须 Ok"
    );

    // 硬断言：寒暄轮不该升档。
    let escalated = find_ptier_event(&state, &contact.wxid, "ptier_escalated").await;
    assert!(
        escalated.is_none(),
        "寒暄轮（\"在吗\"）不应触发 ptier_escalated 升档——小档就够，重型槽位被吞是机制回归。\
         实际 details={:?}",
        escalated.and_then(|e| e.details)
    );
    eprintln!("[p1] 寒暄轮未升档（Enough 分支命中，符合机制核心价值）");

    // 观测：若走了澄清分支也打印出来（寒暄一般不该澄清，但真模型偶发，仅记录不判罚）。
    if let Some(ev) = find_ptier_event(&state, &contact.wxid, "ptier_clarify").await {
        eprintln!(
            "[p1][观测] 寒暄轮意外走了 ptier_clarify（真模型抖动，不判罚）：details={:?}",
            ev.details
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// P2 · 产品/价格问询轮 → 期望 need_more_context 升档到 Full（软断言 + 观测）
// ════════════════════════════════════════════════════════════════════════════

/// 客户问产品功能/价格细节，第一程小档没有业务/产品槽位，期望自评 `need_more_context`
/// → 出现 `ptier_escalated` 且 `target_tier` 含 `"Full"`。真模型不保证每次都升档
/// （可能直接寒暄式回避或判 enough）→ **软断言 + eprintln 观测**：拿到事件则校验形状，
/// 没拿到只 warn 不 panic（避免真模型正常抖动假红）。
#[tokio::test]
#[ignore]
async fn p2_product_inquiry_escalates_to_full() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());

    let contact = managed_contact("real_ptier_user_product");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let inbound = make_inbound(
        &contact,
        "real_ptier_msg_product",
        "你们企业版具体多少钱？能详细讲讲都有哪些功能、怎么落地吗？",
    );
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    unwrap_or_skip_transient!(
        handle_managed_message(&state, contact.clone(), &inbound).await,
        "产品问询轮链路必须 Ok"
    );

    // 软断言：拿到升档事件则校验形状（含 run_id + target_tier，且 target_tier 含 "Full"）；
    // 没拿到只 warn——真模型可能判 enough/clarify，不判罚。
    match find_ptier_event(&state, &contact.wxid, "ptier_escalated").await {
        Some(ev) => {
            let details = ev.details.unwrap_or_default();
            let target_tier = details.get_str("target_tier").unwrap_or("<none>");
            let run_id = details.get_str("run_id").unwrap_or("<none>");
            assert!(
                !run_id.is_empty() && run_id != "<none>",
                "ptier_escalated 事件 details 必须含非空 run_id，实际 details={details:?}"
            );
            eprintln!(
                "[p2] 产品问询轮升档命中：target_tier={target_tier:?} run_id={run_id:?}（符合预期）"
            );
            // target_tier 期望含 "Full"（产品/价格需业务槽位 → Full 档）；真模型也可能升到
            // Relational，仅作软观测不硬断（避免抖动假红）。
            if !target_tier.contains("Full") {
                eprintln!(
                    "[p2][观测] target_tier={target_tier:?} 未含 \"Full\"（期望产品轮升 Full 档，\
                     真模型升到了别的档，仅记录不判罚）"
                );
            }
        }
        None => {
            eprintln!(
                "[p2][观测] 产品问询轮未出现 ptier_escalated 事件——真模型本轮可能自评 enough/clarify\
                 （非确定，软断言不判罚；多次运行应大概率升档）"
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// P3 · 含糊轮 → 期望 need_clarification（软断言 + 观测）
// ════════════════════════════════════════════════════════════════════════════

/// 客户只发一个没有任何上下文指代的"这个"，期望第一程自评 `need_clarification`
/// → 出现 `ptier_clarify` 事件（details 含 run_id）。真模型可能改判 enough（直接反问）
/// 或 need_more_context（升档）→ **软断言 + eprintln 观测**：拿到事件校验形状，
/// 没拿到只 warn 不 panic。
#[tokio::test]
#[ignore]
async fn p3_ambiguous_message_may_clarify() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());

    let contact = managed_contact("real_ptier_user_ambiguous");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 没有任何上下文指代的含糊消息。
    let inbound = make_inbound(&contact, "real_ptier_msg_ambiguous", "这个怎么样");
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    unwrap_or_skip_transient!(
        handle_managed_message(&state, contact.clone(), &inbound).await,
        "含糊轮链路必须 Ok"
    );

    // 软断言：拿到澄清事件则校验 details 含非空 run_id；没拿到只 warn。
    match find_ptier_event(&state, &contact.wxid, "ptier_clarify").await {
        Some(ev) => {
            let details = ev.details.unwrap_or_default();
            let run_id = details.get_str("run_id").unwrap_or("<none>");
            assert!(
                !run_id.is_empty() && run_id != "<none>",
                "ptier_clarify 事件 details 必须含非空 run_id，实际 details={details:?}"
            );
            eprintln!("[p3] 含糊轮澄清命中：run_id={run_id:?}（符合预期）");
        }
        None => {
            eprintln!(
                "[p3][观测] 含糊轮未出现 ptier_clarify 事件——真模型本轮可能自评 enough（直接反问）\
                 或 need_more_context（升档），非确定，软断言不判罚"
            );
            // 顺带观测是否升了档（含糊轮也可能被判为需更多上下文）。
            if let Some(ev) = find_ptier_event(&state, &contact.wxid, "ptier_escalated").await {
                eprintln!(
                    "[p3][观测] 含糊轮改走了 ptier_escalated：details={:?}",
                    ev.details
                );
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// P4 · 产品问询 + 知识库 missing → 期望强升 Full（ptier_forced_full，软断言 + 观测）
// ════════════════════════════════════════════════════════════════════════════

/// 客户问产品/价格，但第一程小档自评 `enough`（自以为够了）而知识库无对应切片
/// （coverage=missing）且本轮需产品知识时，gateway 应**强升 Full** 重生成，写
/// `ptier_forced_full` 事件（details 含 run_id / knowledge_coverage / knowledge_need）。
///
/// 真模型不保证每次都「自评 enough 但 coverage missing」——产品轮更常见是自评
/// need_more_context（走 ptier_escalated，见 p2）。故本测试**软断言 + eprintln 观测**：
/// 拿到 ptier_forced_full 则校验形状，没拿到只 warn 不 panic（避免真模型正常抖动假红）。
#[tokio::test]
#[ignore]
async fn p4_product_inquiry_missing_coverage_forces_full() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());

    let contact = managed_contact("real_ptier_user_forcefull");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let inbound = make_inbound(
        &contact,
        "real_ptier_msg_forcefull",
        "你们这个产品到底能解决什么问题？给我说说就行。",
    );
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    unwrap_or_skip_transient!(
        handle_managed_message(&state, contact.clone(), &inbound).await,
        "产品问询强升轮链路必须 Ok"
    );

    // 软断言：拿到强升事件则校验 details 含非空 run_id；没拿到只 warn——真模型本轮可能
    // 自评 need_more_context（走 escalate）或 enough+coverage 非 missing，不判罚。
    match find_ptier_event(&state, &contact.wxid, "ptier_forced_full").await {
        Some(ev) => {
            let details = ev.details.unwrap_or_default();
            let run_id = details.get_str("run_id").unwrap_or("<none>");
            assert!(
                !run_id.is_empty() && run_id != "<none>",
                "ptier_forced_full 事件 details 必须含非空 run_id，实际 details={details:?}"
            );
            let coverage = details.get_str("knowledge_coverage").unwrap_or("<none>");
            eprintln!(
                "[p4] 强升 Full 命中：run_id={run_id:?} knowledge_coverage={coverage:?}（符合预期）"
            );
        }
        None => {
            eprintln!(
                "[p4][观测] 产品问询轮未出现 ptier_forced_full——真模型本轮可能自评 need_more_context\
                 （走 ptier_escalated）或 coverage 非 missing，强升三条件未同时满足（软断言不判罚）"
            );
            // 顺带观测是否走了升档（产品轮更常见路径）。
            if let Some(ev) = find_ptier_event(&state, &contact.wxid, "ptier_escalated").await {
                eprintln!(
                    "[p4][观测] 本轮改走了 ptier_escalated：details={:?}",
                    ev.details
                );
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// 纯函数分支测试（不需要 LLM / Docker，本地 cargo test 即可跑）
// ════════════════════════════════════════════════════════════════════════════

/// 从**独立 crate 边界**再覆盖一次 `decide_tier_escalation` 三分支（src 内已有 lib 单测，
/// 这里验证 `PromptTier` / `TierDecision` / `decide_tier_escalation` 经
/// `wechatagent::agent::sufficiency::*` 公共路径可达且语义正确）。保证本文件至少有一个
/// 能在本地无 key 跑通的真实断言。
#[test]
fn decide_tier_escalation_branches_via_public_path() {
    use wechatagent::agent::sufficiency::{decide_tier_escalation, PromptTier, TierDecision};
    use wechatagent::agent::AgentDecision;

    // enough → Enough（直接进闸）。
    let enough = AgentDecision {
        sufficiency: "enough".into(),
        ..Default::default()
    };
    assert_eq!(
        decide_tier_escalation(&enough),
        TierDecision::Enough,
        "sufficiency=enough 应判 Enough"
    );

    // need_more_context + missing_tier=full → Escalate(Full)。
    let escalate_full = AgentDecision {
        sufficiency: "need_more_context".into(),
        missing_tier: "full".into(),
        ..Default::default()
    };
    assert_eq!(
        decide_tier_escalation(&escalate_full),
        TierDecision::Escalate(PromptTier::Full),
        "need_more_context + missing_tier=full 应判 Escalate(Full)"
    );

    // need_clarification → Clarify。
    let clarify = AgentDecision {
        sufficiency: "need_clarification".into(),
        ..Default::default()
    };
    assert_eq!(
        decide_tier_escalation(&clarify),
        TierDecision::Clarify,
        "sufficiency=need_clarification 应判 Clarify"
    );
}
