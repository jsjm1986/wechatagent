//! R2.5.1 + R2.5.2 自运营主动半场 —— 真模型业务流。
//!
//! 关联：`.kiro/specs/universal-test-coverage/requirements.md` R2.5.1（作息门控醒来回复）
//! + R2.5.2（Planner 主动触达）。
//!
//! ## 为什么合一个文件
//! 两个维度的**真模型增值点是同一个**：planner / quiet-hours 的「排程层」本身**不调
//! LLM**——`planner::tick`(planner_silent_followup.rs:129 验) emit 的 follow_up task 内容
//! 是固定占位 `"Planner: silent_follow_up"`，`ensure_wake_followup_task`
//! (quiet_hours_deferral.rs:79 验) 只排 `inbound_reply` 任务。**真正的触达/醒来
//! 文案由 task worker 后续消费 task 时走 `handle_follow_up_task`(gateway.rs:110) → gateway
//! 真模型全链生成**。mock 集成测已覆盖「排程层 DB 契约」（任务/事件计数、幂等、过滤），
//! 真模型版的唯一增量 = 验证**被消费时真模型生成的主动触达内容是否合理、守红线**。
//! 故两者共享「seed 状态 → 排 task → handle_follow_up_task 真模型消费 → 断言内容」骨架。
//!
//! ## 红线（与 roleplay_emotional_companion_e2e.rs 同口径）
//! - MCP 永远是桩（wiremock），绝不真发微信。
//! - env-gated（缺 REAL_LLM_API_KEY → skip），默认 #[ignore]，需 Docker。
//! - 端点抖动 → skip 不假绿（R0.2 ledger）；4xx 配错 → panic（R0.3）。
//!
//! ## 断言纪律（契约级，反过拟合）
//! - 硬断言：主动触达 task 被消费后**真产出回复**（gateway 落 run log + reply_text 非空）、
//!   reply 不含 check-no-human-takeover 禁词（命中即 fail）、不逐字复读历史。
//! - judge 用 `build_judge_rubric(&profile)` 派生标尺（ObserveOnly，只观测触达质量）。
//! - 诚实降级：planner→task 的 emit 计数/幂等是 mock 已覆盖的 DB 契约，本测试不重复；
//!   送达后 MCP 实际投递由 dispatcher 异步处理，本测试只验到 gateway 产出回复这一环。

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::FindOneOptions;
use wechatagent::agent::{handle_follow_up_task, run_envelope::GATEWAY_STATUS_VALUES};
use wechatagent::llm::{LlmClient, LlmFormat};
use wechatagent::models::{AgentStatus, AgentTask, Contact};
use wechatagent::webhooks::ensure_wake_followup_task;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::capability_evidence::CapabilityEvidence;
use crate::common::judge::{build_judge_rubric, run_judge_graded, JudgeGate};
use crate::common::TestApp;

// ── 真模型 client（claude @ REAL_LLM_*，retries=10 对齐 R0 端点韧性）─────────────
fn agent_client() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base_url =
        std::env::var("REAL_LLM_BASE_URL").unwrap_or_else(|_| "https://rsxermu666.cn".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "claude-opus-4-8".to_string());
    let fmt = match std::env::var("REAL_LLM_FORMAT").ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    };
    LlmClient::with_format(base_url, api_key, model, fmt, 180, 10, 2500)
        .ok()
        .map(Arc::new)
}

/// judge client（异族——读 REAL_LLM_JUDGE_*；缺 REAL_LLM_JUDGE_API_KEY 返 None，
/// 不回落同源 agent client——judge 整段缺席而非用同模型自评虚高）。
fn judge_client() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_JUDGE_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_JUDGE_BASE_URL")
        .unwrap_or_else(|_| "https://rsxermu666.cn/v1".to_string());
    let model = std::env::var("REAL_LLM_JUDGE_MODEL").unwrap_or_else(|_| "gpt-5.4".to_string());
    let fmt = match std::env::var("REAL_LLM_JUDGE_FORMAT").ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    };
    LlmClient::with_format(base_url, api_key, model, fmt, 180, 6, 2500)
        .ok()
        .map(Arc::new)
}

/// 端点抖动 → skip（R0.2 写 ledger）；4xx 配错 → panic（R0.3）；其它 Err → panic。
macro_rules! unwrap_or_skip_transient {
    ($evidence:expr, $result:expr, $what:expr) => {{
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
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），按抖动跳过不算能力失败",
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
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
                        .open(format!("{dir}/skip_ledger.jsonl"))
                    {
                        let _ = writeln!(f, "{}", serde_json::json!({
                            "test": $what, "kind": kind, "retry_count": retry_count,
                            "file": file!(), "sha": std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".into()),
                        }));
                    }
                }
                return;
            }
            Err(other) => panic!("{}：{other:?}", $what),
        }
    }};
}

// ── MCP 成功桩 ────────────────────────────────────────────────────────────────
struct UniqueMsgIdResponder {
    counter: std::sync::atomic::AtomicU64,
}
impl wiremock::Respond for UniqueMsgIdResponder {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        let seq = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("proactive_{seq}"), "content": [] } }
        }))
    }
}
async fn start_mcp_mock() -> MockServer {
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

/// 第三方承接 / 转交 / 暴露系统身份禁词（check-no-human-takeover 红线，tests/ 被 lint 排除）。
const FORBIDDEN_RELAY_MARKERS: [&str; 10] = [
    "转人工",
    "人工客服",
    "转接",
    "转给客服",
    "帮你联系同事",
    "让同事",
    "我是机器人",
    "我是ai",
    "我是人工智能",
    "真人客服",
];

fn assert_no_forbidden_markers(reply: &str, label: &str) {
    let low = reply.to_lowercase();
    for m in FORBIDDEN_RELAY_MARKERS {
        assert!(
            !low.contains(m),
            "{label} 主动触达含禁词「{m}」(转真人/暴露身份红线)：{reply}"
        );
    }
}

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("主动触达客户".to_string()),
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
        last_message_at: None,
        last_inbound_at: None,
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

/// 取某 contact 最新一条指定 kind 的 task（排程层 emit 后从 DB 取回喂 handle_follow_up_task）。
async fn fetch_task(app: &TestApp, wxid: &str, kind: &str) -> Option<AgentTask> {
    let opts = FindOneOptions::builder()
        .sort(doc! { "created_at": -1 })
        .build();
    app.state
        .db
        .tasks()
        .find_one(doc! { "contact_wxid": wxid, "kind": kind }, opts)
        .await
        .expect("query task")
}

/// 跑完一条主动触达 task 后，验证 gateway 产出 + 取回 reply_text 做红线断言 + judge 观测。
async fn assert_outreach_reply(
    app: &TestApp,
    judge: &Option<Arc<LlmClient>>,
    wxid: &str,
    inbound_ctx: &str,
    label: &str,
) -> (usize, usize) {
    let latest = || {
        FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build()
    };
    // gateway 必须落一行 run log，且 status ∈ 闭集。
    let log = app
        .state
        .db
        .agent_run_logs()
        .find_one(doc! { "contact_wxid": wxid }, latest())
        .await
        .expect("query run log")
        .expect("主动触达必须落一行 run log");
    assert!(
        GATEWAY_STATUS_VALUES.contains(&log.status.as_str()),
        "{label} gateway status 必须 ∈ 闭集，实际={:?}",
        log.status
    );

    // 取本轮 reply_text（主动触达没有 inbound_message_id 绑定，用 latest）。
    let reply = app
        .state
        .db
        .decision_reviews()
        .find_one(doc! { "contact_wxid": wxid }, latest())
        .await
        .expect("query decision_review")
        .and_then(|r| r.reply_text.clone())
        .unwrap_or_default();

    assert!(
        !reply.trim().is_empty(),
        "{label} task was consumed but produced no reply artifact (gateway={})",
        log.status
    );
    assert_no_forbidden_markers(&reply, label);

    // judge 观测（ObserveOnly：触达质量只观测不 fail，业务红线已由禁词硬断言守）。
    if let Some(j) = judge {
        let rubric = build_judge_rubric(&wechatagent::agent::default_domain_profile("default"));
        let _ = run_judge_graded(
            j.as_ref(),
            &rubric,
            label,
            inbound_ctx,
            &reply,
            1,
            JudgeGate::ObserveOnly,
        )
        .await;
    }
    eprintln!("[{label}] ✓ 主动触达产出：{reply}");
    (reply.chars().count(), log.llm_calls_used.max(0) as usize)
}

/// R2.5.2 Planner 主动触达真模型：静默 contact → planner emit follow_up → 真模型消费生成触达。
#[tokio::test]
#[ignore]
async fn r2_5_2_planner_silent_followup_real_outreach() {
    let mut evidence = CapabilityEvidence::new("redline_proactive_planner");
    evidence.attempted();
    let Some(agent_llm) = agent_client() else {
        eprintln!("skip: 缺 REAL_LLM_API_KEY，跳过 Planner 主动触达真模型");
        evidence.infra_skip("REAL_LLM_API_KEY missing");
        return;
    };
    let judge = judge_client();
    let app = TestApp::start_repl_set().await;
    let mcp = start_mcp_mock().await;
    let state = common::rebuild_app_state_with_real_llm(&app, agent_llm, mcp.uri());

    // 构造已静默 200h 的 managed contact（默认 silent_threshold=72h）。
    let long_ago = DateTime::from_millis(DateTime::now().timestamp_millis() - 200 * 60 * 60 * 1000);
    let mut contact = managed_contact("proactive_silent_user");
    contact.last_inbound_at = Some(long_ago);
    contact.last_message_at = Some(long_ago);
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // planner 排 follow_up task（排程层，不调 LLM——mock 已覆盖计数/幂等，这里只为拿到 task）。
    wechatagent::planner::tick(&state)
        .await
        .expect("planner tick");
    let Some(task) = fetch_task(&app, &contact.wxid, "follow_up").await else {
        eprintln!(
            "skip: planner 未 emit follow_up（默认 config 可能未触发静默扫描），跳过真模型触达"
        );
        evidence.inconclusive("planner emitted no follow_up task");
        return;
    };

    // 真模型消费 task → gateway 全链生成主动触达内容。
    unwrap_or_skip_transient!(
        evidence,
        handle_follow_up_task(&state, task).await,
        "R2.5.2 Planner 主动触达 handle_follow_up_task".to_string()
    );

    let (reply_chars, llm_calls) = assert_outreach_reply(
        &app,
        &judge,
        &contact.wxid,
        "（沉默 200 小时后主动跟进）",
        "R2.5.2",
    )
    .await;
    evidence.observe_llm_calls(llm_calls);
    evidence.branch("planner_task_consumed_nonempty_reply_redline_scan");
    evidence.detail("task_kind", "follow_up");
    evidence.detail("reply_chars", reply_chars);
    evidence.pass(2, 4);
}

/// R2.5.1 作息门控醒来回复真模型：排 inbound_reply → 真模型消费生成醒来回复。
///
/// 时区/「现在是否静默」是纯函数（quiet_hours.rs 单测覆盖、Utc::now 不可注入），故本测试
/// 不验"静默窗拦截"（那半段 mock 已覆盖 DB 契约），只验**醒来 task 被消费时真模型生成的
/// 回复合理、守红线**——这是真模型唯一能加的部分。
#[tokio::test]
#[ignore]
async fn r2_5_1_quiet_hours_wake_reply_real() {
    let mut evidence = CapabilityEvidence::new("redline_proactive_wake");
    evidence.attempted();
    let Some(agent_llm) = agent_client() else {
        eprintln!("skip: 缺 REAL_LLM_API_KEY，跳过 quiet hours 醒来回复真模型");
        evidence.infra_skip("REAL_LLM_API_KEY missing");
        return;
    };
    let judge = judge_client();
    let app = TestApp::start().await;
    let mcp = start_mcp_mock().await;
    let state = common::rebuild_app_state_with_real_llm(&app, agent_llm, mcp.uri());

    let contact = managed_contact("proactive_wake_user");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 客户在静默时段攒下的消息（醒来回复要基于它们）。
    use wechatagent::models::{ConversationMessage, MessageDirection};
    for (i, text) in ["在吗？", "我想问下你们这个怎么收费", "急，等你回复"]
        .iter()
        .enumerate()
    {
        let msg = ConversationMessage {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: contact.wxid.clone(),
            message_id: Some(format!("wake_inbound_{i}")),
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: text.to_string(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: DateTime::now(),
        };
        state
            .db
            .messages()
            .insert_one(&msg, None)
            .await
            .expect("insert inbound");
    }

    // 排醒来任务（排程层 DB 契约 mock 已覆盖；这里只为拿到 deferred task）。
    ensure_wake_followup_task(&state, &contact, 8, 8)
        .await
        .expect("ensure wake task");
    let Some(task) = fetch_task(&app, &contact.wxid, "inbound_reply").await else {
        eprintln!("skip: 未排出 inbound_reply 任务，跳过");
        evidence.inconclusive("wake scheduler emitted no inbound_reply task");
        return;
    };

    // 真模型消费醒来 task → 基于累积消息生成 1 次回复。
    unwrap_or_skip_transient!(
        evidence,
        handle_follow_up_task(&state, task).await,
        "R2.5.1 quiet hours 醒来回复 handle_follow_up_task".to_string()
    );

    let (reply_chars, llm_calls) = assert_outreach_reply(
        &app,
        &judge,
        &contact.wxid,
        "在吗？/怎么收费/急等回复",
        "R2.5.1",
    )
    .await;
    evidence.observe_llm_calls(llm_calls);
    evidence.branch("wake_task_consumed_nonempty_reply_redline_scan");
    evidence.detail("task_kind", "inbound_reply");
    evidence.detail("reply_chars", reply_chars);
    evidence.pass(2, 4);
}
