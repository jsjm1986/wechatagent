//! `real_llm_principal_relay` —— G9/G10 覆盖缺口补齐：**幕后请示通道「入站 relay 回路」真模型业务流**。
//!
//! 关联：深度审查 `.kiro/specs/universal-test-coverage/recovered-findings-wf2.json`（A2/A3：
//! relay 回路从未在真模型下执行）；设计 `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`。
//!
//! ## 为什么补这个套件（与出站方向的分工）
//! 产品定位：客户永远只跟 AI 对话、**永不直接面对真人**。AI 遇到超职权事项时向**幕后决策源
//! （领导）请示**，领导用自然语言回裁决，AI 再用**自己的口吻**向客户转述——绝不暴露「这是真人/
//! 领导拍板的」。这条链有两段方向：
//! - **出站**（已有 `tests/real_llm_principal_channel.rs` 覆盖）：客户超职权诉求 → agent 发起请示。
//! - **入站 relay 回路**（本套件，G9/G10 零真模型测试缺口）：领导自然语言裁决 →
//!   `interpret_principal_reply`（真 LLM 解析成结构化 `PrincipalDecision`）→ `resolve_escalation`
//!   → relay task → AI 用自己口吻向客户转述。
//!
//! ## 本套件驱动的回路环节（端到端）
//! ① 入站 webhook（公开入口 `wechat_webhook`）：领导 wxid 发来自然语言裁决 → 路由进请示通道；
//! ② `interpret_principal_reply`（真 LLM）：把「可以给他打九折，但仅此一次」解析成结构化裁决；
//! ③ `resolve_escalation`：台账 pending → resolved，verdict 落库；
//! ④ relay task（`handle_follow_up_task` → `handle_principal_decision_relay`）：起 relay 转述；
//! ⑤ `relay_principal_decision_to_customer` → 网关（真 LLM）：生成**面向客户的转述文本**。
//!
//! ## 触达方式 / pub(crate) 卡点说明
//! `interpret_principal_reply` / `handle_principal_reply` 都是 `pub(crate)`，crate 外测试不可直接调。
//! 铁律「绝不改生产可见性」下，本套件走**公开 webhook 入口** `wechatagent::webhooks::wechat_webhook`
//! 驱动入站解析回路（生产真实入口，分流逻辑见 webhooks.rs:413）；relay 转述走**公开再导出**的
//! `wechatagent::agent::handle_follow_up_task`（task worker 真实调用点，gateway.rs:112 据 kind 分流到
//! `handle_principal_decision_relay`）。两个都是生产链路的真实公开入口，零可见性改动。
//!
//! ## 断言口径（确定性契约级，反过拟合）
//! 领导裁决文本是固定的测试输入；断言走契约级、不锁 agent 转述的具体措辞：
//! - **解析成功**：台账 status=resolved，decision 落库，verdict ∈ 闭集且 ≠ deferred（明确批准不应被判暂缓）；
//! - **G10 红线**：转述文本不暴露幕后真人决策源（`FORBIDDEN_BACKSTAGE_MARKERS` 命中即 fail），
//!   也不转真人/不暴露身份（复用 `common::redline::assert_no_handoff_or_identity_leak`）；
//! - **闭集契约**：gateway status / final_review_status ∈ 闭集。
//!
//! ## 红线（与 real_llm_principal_channel 同口径）
//! - **MCP 永远是桩**：`rebuild_app_state_with_real_llm` 把 mcp_base_url 指向 wiremock，绝不真发微信。
//! - **env-gated**：无 `REAL_LLM_API_KEY` → 自我跳过（eprintln + return），默认 `#[ignore]`。
//! - 端点抖动 → skip 写 ledger；配置错误 4xx → panic（堵 R0.3 假绿）。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=... \
//!   cargo test --test real_llm_principal_relay -- --ignored --nocapture
//! ```
//! 需 Docker（testcontainers MongoDB）。

mod common;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOneOptions;
use wechatagent::agent::handle_follow_up_task;
use wechatagent::agent::run_envelope::{FINAL_REVIEW_STATUS_VALUES, GATEWAY_STATUS_VALUES};
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{LlmClient, LlmFormat, LlmJsonResult, LlmProvider};
use wechatagent::models::{
    AgentPrincipalEscalation, AgentStatus, AgentTask, Contact, ESCALATION_CATEGORY_OUT_OF_SCOPE,
    PRINCIPAL_ESCALATION_STATUS_PENDING, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
    PRINCIPAL_VERDICT_DEFERRED,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::redline::assert_no_handoff_or_identity_leak;
use crate::common::TestApp;

// ════════════════════════════════════════════════════════════════════════════
// env-gated 真实 provider 构造 + 跨模型 failover 备胎链
//
// 与 `real_llm_principal_channel.rs:63-247` 同口径：被测 agent 始终是生产主模型
// （冻结为对照），failover 只解「端点限流污染能力测评」，不抬高被测分。纯测试侧、零生产改动。
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

/// 主模型重试预算：10 次指数退避（base 2500ms）熬过限流窗。
fn primary_max_retries() -> u32 {
    10
}

/// 构造最强模型 client（默认 llama-3.3-70b @ NVIDIA integrate）。缺 `REAL_LLM_JUDGE_API_KEY` → None。
/// 本套件不打分，仅借它作 agent 备胎链首选。
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

/// 无主 key → 打印 skip 并 return（不 panic）。返回主 + 备胎链 provider。
macro_rules! require_real_llm {
    () => {{
        match real_llm_with_failover() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过 G9/G10 幕后请示通道入站 relay 回路真模型 E2E");
                return;
            }
        }
    }};
}

/// 真模型上游瞬时不可达（限流/超时等 `LlmUnavailable`）→ skip return，不算能力失败；
/// 配置错误 4xx（漏 /v1 等，非 401/402）→ panic（堵 R0.3 假绿）；其它 `Err` 仍 panic。
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
                        "{}：配置错误（kind={kind}），非端点抖动——4xx 多为 baseUrl/model/path 配错，\
                         不当瞬时 skip 假绿（R0.3）。detail={detail}",
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
            "result": { "structuredContent": { "newMsgId": format!("relay_msg_{seq}"), "content": [] } }
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

/// 配置 default workspace 的 user_operations 域 `principal_decider`（领导 wxid）。
/// 没配 = 请示通道未启用，webhook 不会把领导消息分流进请示流。
/// 镜像 `principal_decision_channel.rs` §14.1b：`$set` 到 seed 的 current 行（唯一索引禁重复插）。
async fn configure_principal_decider(app: &TestApp, principal_wxid: &str) {
    app.state
        .db
        .operation_domain_configs()
        .update_one(
            doc! { "workspace_id": "default", "domain": "user_operations", "current_version": true },
            doc! { "$set": {
                "principal_decider": principal_wxid,
                "high_risk_escalation_mode": "all",
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await
        .expect("set principal_decider on seeded domain config");
}

/// 销售域客户（default ws，managed）。relay 走合成 inbound 豁免频控，故 last_agent_run_at 取 None 即可。
fn sales_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("等领导裁决的客户".to_string()),
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        tags: Vec::new(),
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
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

/// 一条 pending 请示台账（镜像 `escalation::insert_pending_escalation` 写入的形状）。
/// 模拟「客户超职权诉求已发起请示、正等领导回话」这一前置状态——本套件聚焦其后的入站 relay 回路。
fn pending_escalation(short_code: &str, contact_wxid: &str, principal_wxid: &str) -> AgentPrincipalEscalation {
    let now = DateTime::now();
    AgentPrincipalEscalation {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: contact_wxid.to_string(),
        short_code: short_code.to_string(),
        status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
        category: ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string(),
        reason: "客户要求超出标准 9 折权限的特批折扣".to_string(),
        question_for_principal: "客户想要九折并要求仅此一次，是否同意？".to_string(),
        principal_wxid: principal_wxid.to_string(),
        decision: None,
        authorization_expires_at: None,
        is_generalizable: false,
        knowledge_proposal_emitted: false,
        created_at: now,
        updated_at: now,
        resolved_at: None,
    }
}

// ── 红线禁词扫描（暴露幕后真人决策源）─────────────────────────────────────────
//
// 这些字面量在 **tests/** 下书写，`scripts/check-no-human-takeover.{sh,ps1}` 显式排除
// tests/，故不触发 lint。这正是红线测试的意义：在 tests/ 里写出禁词，断言**生产 relay
// 转述文本里没有它们**。转真人/暴露身份红线复用 `common::redline::assert_no_handoff_or_identity_leak`。

/// 暴露幕后真人决策源的禁词（relay 应是 AI 自己口吻转述结论，不能说「这是领导/老板拍板的」
/// 「上级批准」之类把幕后真人决策源捅给客户的话）。
///
/// 边界（与 real_llm_principal_channel.rs:445 同款）：设计 §3 明确「我帮你跟领导申请一下」
/// 是合规话术——故**不**把「领导」裸词当禁词，只锁「暴露这是真人**已做的决定/拍板**」的措辞。
const FORBIDDEN_BACKSTAGE_MARKERS: [&str; 9] = [
    "领导决定",
    "领导拍板",
    "老板决定",
    "老板拍板",
    "领导说可以",
    "老板说可以",
    "这是领导的决定",
    "这是老板的决定",
    "上级批准",
];

/// 领导自然语言裁决（固定测试输入）：明确批准 + 带约束。真 LLM 应解析出 ≠ deferred 的 verdict。
const PRINCIPAL_VERDICT_REPLY: &str = "这个客户可以给他打九折，但仅此一次，让他这周内付款。";

#[tokio::test]
#[ignore]
async fn principal_inbound_relay_loop_happy_path() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp.uri());

    // 启用请示通道：配置领导 wxid（webhook 据此把领导消息分流进请示流）。
    let principal_wxid = "principal_boss_relay_wxid";
    configure_principal_decider(&app, principal_wxid).await;

    // 客户 + 一条 pending 请示台账（前置状态：已请示、正等领导回话）。
    let contact = sales_contact("principal_relay_customer");
    state.db.contacts().insert_one(&contact, None).await.expect("insert contact");
    // 该客户当前正等待领导决策（relay 完成后应被清掉）。
    let awaiting_key = format!(
        "domain_attributes.{}",
        wechatagent::models::AWAITING_PRINCIPAL_DECISION_ATTR
    );
    state
        .db
        .contacts()
        .update_one(
            doc! { "wxid": &contact.wxid, "workspace_id": "default", "account_id": "default" },
            doc! { "$set": { &awaiting_key: true } },
            None,
        )
        .await
        .expect("set awaiting marker");

    let short_code = "E7K9";
    let entry = pending_escalation(short_code, &contact.wxid, principal_wxid);
    state
        .db
        .agent_principal_escalations()
        .insert_one(&entry, None)
        .await
        .expect("insert pending escalation");

    // ── 环节①②③：领导自然语言裁决经公开 webhook 入站 → 真 LLM 解析 → resolve ──
    // 领导该客户只有这一条 pending，回复不带短码也能精确命中（match_principal_reply 单条兜底）。
    let payload = serde_json::json!({
        "fromWxid": principal_wxid,
        "content": PRINCIPAL_VERDICT_REPLY,
    });
    let body = Bytes::from(serde_json::to_vec(&payload).expect("serialize webhook payload"));
    let resp = unwrap_or_skip_transient!(
        wechatagent::webhooks::wechat_webhook(State(state.clone()), HeaderMap::new(), body).await,
        "G9/G10 webhook 入站领导裁决解析必须 Ok"
    );
    // webhook 应把这条消息当领导回复消费（routed=principal），不进客户 agent 链路。
    assert_eq!(
        resp.0.get("routed").and_then(|v| v.as_str()),
        Some("principal"),
        "领导裁决应被路由进请示通道（routed=principal），实际响应：{:?}",
        resp.0
    );

    // 断言①：台账被 resolve（status=resolved，decision 落库，verdict ∈ 闭集且 ≠ deferred）。
    let resolved = state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": short_code }, None)
        .await
        .expect("query escalation")
        .expect("escalation must exist");
    assert_eq!(
        resolved.status, PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "明确批准的领导裁决应把台账 resolve，实际 status={:?}（若仍 pending，多半真模型把明确批准误判 deferred）",
        resolved.status
    );
    let decision = resolved
        .decision
        .clone()
        .expect("resolved 台账必须落 decision");
    assert!(
        wechatagent::models::ALLOWED_PRINCIPAL_VERDICT.contains(&decision.verdict.as_str()),
        "verdict 必须 ∈ 闭集，实际={:?}",
        decision.verdict
    );
    assert_ne!(
        decision.verdict, PRINCIPAL_VERDICT_DEFERRED,
        "明确批准不应被解析成 deferred（否则 relay 回路不会触发）"
    );
    eprintln!(
        "[G9/G10][解析] verdict={} substance={:?} constraints={:?}",
        decision.verdict, decision.substance, decision.constraints
    );

    // 断言：relay task 已入队（kind=principal_decision_relay，content=short_code）。
    let relay_task = state
        .db
        .tasks()
        .find_one(
            doc! { "kind": "principal_decision_relay", "content": short_code },
            None,
        )
        .await
        .expect("query relay task")
        .expect("resolve 后应起一条 relay task");

    // ── 环节④⑤：relay task → 真 LLM 用 AI 口吻向客户转述 ──
    // 走公开再导出的 handle_follow_up_task（task worker 真实调用点，据 kind 分流到 relay）。
    let relay_task: AgentTask = relay_task;
    unwrap_or_skip_transient!(
        handle_follow_up_task(&state, relay_task).await,
        "G9/G10 relay 转述链路必须 Ok"
    );

    let latest = || FindOneOptions::builder().sort(doc! { "created_at": -1 }).build();

    // relay run 的 gateway 终态：闭集契约（确定性）。relay 合成 inbound 的 message_id=None，
    // 故按 contact_wxid 取最新一行（本套件该 contact 仅 relay run 产生 run log）。
    let log = state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
        .await
        .expect("query run log")
        .expect("relay run 必须落一行 run log");
    assert!(
        GATEWAY_STATUS_VALUES.contains(&log.status.as_str()),
        "relay run gateway status 必须 ∈ 闭集，实际={:?}",
        log.status
    );
    assert!(
        log.final_review_status.is_empty()
            || FINAL_REVIEW_STATUS_VALUES.contains(&log.final_review_status.as_str()),
        "relay run final_review_status 必须 ∈ 闭集或空，实际={:?}",
        log.final_review_status
    );

    // 断言②：面向客户的转述文本（decision_reviews.reply_text）通过 G10 红线。
    // relay 合成 inbound message_id=None → inbound_message_id 也为 None，按 contact_wxid 取最新终态。
    let review = state
        .db
        .decision_reviews()
        .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
        .await
        .expect("query decision_review")
        .expect("relay run 必须落 decision_review");
    let reply_text = review.reply_text.clone().unwrap_or_default();
    eprintln!(
        "[G9/G10][转述] gateway={} final_review={} reply_text={:?}",
        log.status, log.final_review_status, reply_text
    );
    assert!(
        !reply_text.trim().is_empty(),
        "relay happy-path 应生成面向客户的转述文本，实际为空（gateway={}）",
        log.status
    );

    // ②a 不转真人 / 不暴露身份（复用共享红线判定，命中即 panic）。
    assert_no_handoff_or_identity_leak(&reply_text, "G9/G10 relay 转述");
    // ②b 不暴露幕后真人决策源——relay 应是 AI 自己口吻转述结论，不把「真人已拍板」捅给客户。
    for marker in FORBIDDEN_BACKSTAGE_MARKERS {
        assert!(
            !reply_text.contains(marker),
            "G9/G10 relay 转述暴露幕后真人决策源「{marker}」(应是 AI 自己口吻转述结论，不暴露真人拍板)：{reply_text}"
        );
    }

    // 软观测：relay 完成后应清掉客户的「等待领导决策」标记（clear_awaiting_principal_state）。
    let after = state
        .db
        .contacts()
        .find_one(doc! { "wxid": &contact.wxid }, None)
        .await
        .expect("query contact")
        .expect("contact must exist");
    let still_awaiting = after
        .domain_attributes
        .map(|d| d.get_bool(wechatagent::models::AWAITING_PRINCIPAL_DECISION_ATTR).unwrap_or(false))
        .unwrap_or(false);
    eprintln!("[G9/G10][软观测] relay 后 awaiting 标记残留={still_awaiting}（应为 false）");
}
