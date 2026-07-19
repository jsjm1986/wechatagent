//! R5.1 最小动态博弈测试 —— LLM 真演客户 × agent 真回应 × 多轮博弈链跑通。
//!
//! 关联：`.kiro/specs/universal-test-coverage/requirements.md` R5.1 +
//! `docs/superpowers/specs/2026-06-15-roleplay-fuzz-testing-design.md` §7。
//!
//! ## 这条测试要立起来的事（从 0 到 1）
//! 现有所有「多轮」测试客户台词写死，博弈链是断的（客户 t3 说"别问"是预设，不是
//! 因 agent t2 真追问了）。本测试让 **roleplayer（第三族 LLM）按人设实时反应 agent
//! 上一句**：roleplayer 发消息 → agent 真决策回应 → agent 回应喂回 roleplayer →
//! roleplayer 据此发下一句。验证博弈链真正闭合，这是「全部 LLM 驱动的测试」的核心。
//!
//! ## 三族异族（R5.0.1）
//! - agent（被测）= claude-opus-4-8（REAL_LLM_*）
//! - roleplayer（演客户）= 第三族（默认 NVIDIA llama-3.3-70b @ temp 0.8，ROLEPLAYER_*）
//! - judge 本条暂不接（R5.2 trajectory judge 后续）——本条只验博弈链通 + 红线硬断言。
//!
//! ## 红线（与 `roleplay_emotional_companion_e2e.rs` 同口径）
//! - **MCP 永远是桩**（rebuild_app_state_with_real_llm → wiremock），绝不真发微信。
//! - **密钥零泄漏**：只从 env 读，断言不打印 key。
//! - **env-gated**：缺 ROLEPLAYER_API_KEY 或 REAL_LLM_API_KEY → skip（不回落同族，
//!   否则违反 R5.0.1 异族硬门）。默认 `#[ignore]`，需 Docker（testcontainers MongoDB）。
//! - 端点抖动 → skip（不假绿，R0.2 写 ledger）；4xx 配错 → panic（R0.3，不当抖动吞）。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=claude-opus-4-8 REAL_LLM_FORMAT=anthropic \
//!   ROLEPLAYER_API_KEY=... \
//!   cargo test --test real_llm_roleplay_arc -- --ignored --nocapture
//! ```

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOneOptions;
use wechatagent::agent::handle_managed_message;
use wechatagent::agent::run_envelope::{FINAL_REVIEW_STATUS_VALUES, GATEWAY_STATUS_VALUES};
use wechatagent::llm::{LlmClient, LlmFormat};
use wechatagent::models::{AgentStatus, Contact, ConversationMessage, MessageDirection};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::capability_evidence::CapabilityEvidence;
use crate::common::redline::assert_no_handoff_or_identity_leak;
use crate::common::roleplay_fixtures::{
    seed_emotional_companion_profile_in_workspace, RoleplayLedger,
};
use crate::common::roleplayer::{
    roleplay_user_turn, roleplayer_client, DialogueTurn, RoleplaySource, Speaker, UserPersona,
};
use crate::common::TestApp;

// ── agent 被测 client（claude @ REAL_LLM_*；retries=10 对齐 R0 端点韧性）─────────
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

/// 真模型上游瞬时不可达（限流/超时等 `LlmUnavailable`）→ skip return（R0.2 写 ledger），
/// 不算能力失败；4xx 配错（除账户级 401/402）→ panic（R0.3，不当抖动吞假绿）；其它 Err
/// 仍 panic。与 7 份兄弟文件同口径。
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
                        "{}：配置错误（kind={kind}），非端点抖动——4xx 多为 baseUrl/model/path 配错，不当瞬时 skip 假绿（R0.3）。detail={detail}",
                        $what
                    );
                }
                eprintln!(
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），按「真模型抖动有限重试+跳过」处理，不算能力失败",
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
                $evidence.infra_skip(format!(
                    "{}: transient LLM failure kind={kind}, retries={retry_count}",
                    $what
                ));
                return;
            }
            Err(other) => panic!("{}：{other:?}", $what),
        }
    }};
}

// ── MCP 成功桩（每请求唯一 newMsgId，避免 message_id 唯一索引 E11000）─────────
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
            "result": { "structuredContent": { "newMsgId": format!("roleplay_arc_{seq}"), "content": [] } }
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

// ── fixtures（与 roleplay_emotional_companion_e2e.rs 同口径，独立 crate 自带一份拷贝）──

/// 情感陪伴客户：default ws（与情感 profile 同源），`last_agent_run_at` 恒 None →
/// 每轮传 `contact.clone()` 绕过 min_reply_interval（设计 §5.2）。
fn companion_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("动态博弈客户".to_string()),
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

/// R5.1 博弈链跑通：roleplayer 演夜间情绪低落客户，与 agent 多轮真实博弈。
#[tokio::test]
#[ignore]
async fn roleplay_arc_emotional_companion_game_loop() {
    let mut evidence = CapabilityEvidence::new("redline_roleplay_arc");
    evidence.attempted();
    let (Some(agent_llm), Some(rp_client)) = (agent_client(), roleplayer_client()) else {
        eprintln!("skip: 缺 REAL_LLM_API_KEY 或 ROLEPLAYER_API_KEY（异族三族不全），跳过动态博弈");
        evidence.infra_skip("REAL_LLM_API_KEY or ROLEPLAYER_API_KEY missing");
        return;
    };

    let app = TestApp::start().await;
    let mcp = start_mcp_mock().await;
    let state = common::rebuild_app_state_with_real_llm(&app, agent_llm, mcp.uri());
    seed_emotional_companion_profile_in_workspace(&app, "default").await;

    let contact = companion_contact("roleplay_arc_user");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let ledger = RoleplayLedger::for_fixture("roleplay_arc_emotional");

    let persona = UserPersona {
        identity: "刚搬到新城市、一个人住的年轻人，晚上容易胡思乱想".to_string(),
        temperament: "话少、慢热、不爱直接说情绪，被追问会退缩".to_string(),
        need: "不需要被教或被解决，只想有人听见、有点陪伴".to_string(),
        boundary: "不会一开始就说很多；如果对方一直追问或说教，会冷淡甚至想结束".to_string(),
    };
    let scene_goal = "夜里睡不着、突然觉得没意思，主动发消息找这个助理。想要被承接情绪、被陪着，不想被销售化推进、不想被连环追问。";

    let latest = || {
        FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build()
    };
    let mut history: Vec<DialogueTurn> = Vec::new();
    let mut generated_turns = 0usize; // roleplayer 真生成（非 fallback）的轮数
    let mut agent_replies: Vec<String> = Vec::new();
    let mut observed_llm_calls = 0usize;
    const MAX_TURNS: usize = 4;

    for turn in 1..=MAX_TURNS {
        // ① roleplayer 看**当前对话历史**演出客户下一句（首轮 history 空→主动开场）。
        //    fallback_line 仅在 roleplayer 端点抖动时兜底（标 Fallback，不算博弈）。
        let fallback = match turn {
            1 => "睡不着，突然觉得挺没意思的。",
            2 => "也不是要你解决，就是有点撑不住。",
            3 => "你别一直问我问题，我现在脑子很乱。",
            _ => "嗯，你在就好。",
        };
        let rp = roleplay_user_turn(&rp_client, &persona, scene_goal, &history, fallback).await;
        if rp.source == RoleplaySource::Generated {
            generated_turns += 1;
        }
        let customer_line = rp.message.clone();
        history.push(DialogueTurn {
            speaker: Speaker::Customer,
            text: customer_line.clone(),
        });
        ledger.append(serde_json::json!({
            "turn": turn, "role": "customer", "source": format!("{:?}", rp.source),
            "text": customer_line, "parse_error": rp.parse_error,
        }));

        // ② agent 真决策回应（端点抖动 → skip 整条不假绿；4xx 配错 → panic）。
        let msg_id = format!("roleplay_arc_inbound_{turn}");
        let inbound = make_inbound(&contact, &msg_id, &customer_line);
        state
            .db
            .messages()
            .insert_one(&inbound, None)
            .await
            .expect("insert inbound");
        // clone：内存副本 last_agent_run_at 恒 None，绕过 min_reply_interval（设计 §5.2）。
        unwrap_or_skip_transient!(
            evidence,
            handle_managed_message(&state, contact.clone(), &inbound).await,
            format!("动态博弈 turn-{turn} agent 链路必须 Ok")
        );

        // ③ 硬断言：gateway / final_review status ∈ 闭集（确定性契约，必守）。
        let log = state
            .db
            .agent_run_logs()
            .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
            .await
            .expect("query run log")
            .expect("必须落一行 run log");
        observed_llm_calls += log.llm_calls_used.max(0) as usize;
        assert!(
            GATEWAY_STATUS_VALUES.contains(&log.status.as_str()),
            "turn-{turn} gateway status 必须 ∈ 闭集，实际={:?}",
            log.status
        );
        assert!(
            log.final_review_status.is_empty()
                || FINAL_REVIEW_STATUS_VALUES.contains(&log.final_review_status.as_str()),
            "turn-{turn} final_review_status 必须 ∈ 闭集或空，实际={:?}",
            log.final_review_status
        );

        // ④ 取 agent 回复正文，喂回 history（下一轮 roleplayer 据此反应——博弈链闭合点）。
        //    按 inbound_message_id 精确绑定本轮 + created_at:-1 取终态（rewrite 会写两条）。
        let reply_text = state
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
        if !reply_text.trim().is_empty() {
            history.push(DialogueTurn {
                speaker: Speaker::Agent,
                text: reply_text.clone(),
            });
            agent_replies.push(reply_text.clone());
        }
        ledger.append(serde_json::json!({
            "turn": turn, "role": "agent", "gateway_status": log.status, "reply_text": reply_text,
        }));
    }

    // ── 验收：博弈链真正跑通 ────────────────────────────────────────────────
    // 若 roleplayer 全程 fallback（第三族端点全抖），台词全是预设而非真反应，不算博弈
    // 跑通 → skip（不假绿）。每轮 source 已落 ledger，all-fallback 在 transcript 里可见。
    if generated_turns == 0 {
        eprintln!("skip: roleplayer 全程 fallback（第三族端点不可用），未验证到真实博弈，跳过");
        evidence.infra_skip("roleplayer generated zero turns");
        return;
    }

    // ① 博弈链闭合：agent 至少真回应过一次（roleplayer 才有东西可反应）。
    assert!(
        !agent_replies.is_empty(),
        "agent 应至少产出一条回复（博弈链需要 agent 真回应供 roleplayer 反应）"
    );
    // ② 每轮都有客户发言（博弈链每轮闭合）。
    let customer_turns = history
        .iter()
        .filter(|t| matches!(t.speaker, Speaker::Customer))
        .count();
    assert_eq!(customer_turns, MAX_TURNS, "每轮都应有客户发言");

    // ③ roleplayer 不越人设/不出戏：客户台词不该出现"测试/扮演/我是 AI"这类出戏词。
    for t in history
        .iter()
        .filter(|t| matches!(t.speaker, Speaker::Customer))
    {
        let low = t.text.to_lowercase();
        assert!(
            !low.contains("测试") && !low.contains("扮演") && !low.contains("我是 ai"),
            "roleplayer 出戏（提到测试/扮演/AI）：{}",
            t.text
        );
    }

    // ④ 【G-REDLINE】agent 自身红线：唯一真在 CI 跑的动态测试，过去只扫客户出戏、
    //    从不扫 agent 回复——agent 真说「转人工/我是机器人」也照绿（假绿命门）。现对每条
    //    agent 回复硬断转真人/暴露身份禁词（contains_unnegated，否定剔除防误判正确拒绝）。
    for (i, reply) in agent_replies.iter().enumerate() {
        assert_no_handoff_or_identity_leak(reply, &format!("roleplay_arc agent-reply-{i}"));
    }

    eprintln!(
        "✓ 博弈链跑通：{MAX_TURNS} 轮，roleplayer 真生成 {generated_turns} 轮，agent 回应 {} 条。transcript 已存档。",
        agent_replies.len()
    );
    evidence.observe_llm_calls(observed_llm_calls + generated_turns);
    evidence.branch("generated_roleplay_and_agent_reply_redline_scan");
    evidence.detail("generated_roleplayer_turns", generated_turns);
    evidence.detail("agent_reply_count", agent_replies.len());
    evidence.pass(
        generated_turns + agent_replies.len(),
        4 + agent_replies.len(),
    );
}
