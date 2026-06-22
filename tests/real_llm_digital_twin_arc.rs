//! R2.1 + COV-2 数字分身全链真模型 arc —— peer_social / formal_business 两域端到端。
//!
//! 关联：`.kiro/specs/universal-test-coverage/requirements.md` R2.1（随机身份生成器驱动
//! 全域）+ 深度审查 G20（generate_identity 定义后零调用 = 死代码；peer_social /
//! formal_business 域从无全链真模型 arc，跨域 arc 只覆盖 Sales + Companion 两域）。
//!
//! ## 这条测试要立起来的事
//! 通用化的卖点是「适配任意行业 + 数字分身（同行/正式业务/朋友…）」，但此前**没有任何
//! 真模型多轮 arc 让 peer_social / formal_business 域的 active DomainProfile 真正进过
//! gateway**。`tests/common/identity_generator.rs::generate_identity`（用真 LLM 把行业骨架
//! 丰满成 DomainProfile + UserPersona + 开场白）写好了却**从无调用点**。本测试把它接起来：
//! 生成身份 → seed 成 active profile → roleplayer 按生成的人设与 agent 多轮博弈 → 契约级
//! 硬断言（闭集 status + 转真人/身份红线 + 画像落地）。
//!
//! ## 三族异族（R5.0.1）
//! - agent（被测）= claude-opus-4-8（REAL_LLM_*）
//! - 身份生成器 + roleplayer（演客户）= 第三族（ROLEPLAYER_*，默认 NVIDIA）
//!
//! ## 红线 / 纪律
//! - MCP 永远是桩（wiremock），绝不真发微信；密钥零泄漏（只从 env 读）。
//! - env-gated：缺 REAL_LLM_API_KEY 或 ROLEPLAYER_API_KEY → skip（不假绿，不回落同族）。
//! - 端点抖动 → skip（写 ledger，R0.2）；4xx 配错 → panic（R0.3）。默认 #[ignore]。
//! - 身份由 LLM 动态生成 → 断言只走**契约级**（闭集 / 命中禁词即 fail / 画像字段有无），
//!   绝不锁单条措辞或单个行业（[[no-overfitting]]）。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=claude-opus-4-8 REAL_LLM_FORMAT=anthropic \
//!   ROLEPLAYER_API_KEY=... \
//!   cargo test --test real_llm_digital_twin_arc -- --ignored --nocapture
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

use crate::common::identity_generator::{generate_identity, GeneratedIdentity, IdentityCategory};
use crate::common::redline::assert_no_handoff_or_identity_leak;
use crate::common::roleplay_fixtures::{seed_active_domain_profile, RoleplayLedger};
use crate::common::roleplayer::{
    roleplay_user_turn, roleplayer_client, DialogueTurn, RoleplaySource, Speaker,
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

/// 端点瞬时不可达 → skip（写 ledger，R0.2）；4xx 配错 → panic（R0.3）。与 7 份兄弟文件同口径。
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
                        "{}：配置错误（kind={kind}），非端点抖动——4xx 多为 baseUrl/model/path 配错，不当瞬时 skip 假绿（R0.3）。detail={detail}",
                        $what
                    );
                }
                eprintln!(
                    "skip: {} —— 真模型上游瞬时不可达（kind={kind}, retry_count={retry_count}），不算能力失败",
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
                                "test": $what, "kind": kind, "retry_count": retry_count,
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

// ── MCP 成功桩（每请求唯一 newMsgId，避免 message_id 唯一索引 E11000）─────────
struct UniqueMsgIdResponder {
    counter: std::sync::atomic::AtomicU64,
}
impl wiremock::Respond for UniqueMsgIdResponder {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        let seq = self.counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("dtwin_{seq}"), "content": [] } }
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

fn twin_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("数字分身对话方".to_string()),
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        tags: Vec::new(),
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
        created_at: DateTime::now(),
    }
}

/// 跑一条数字分身全链 arc：用 `generate_identity` 真生成身份 → seed 成 active profile →
/// roleplayer 按生成人设与 agent 多轮博弈 → 契约级硬断言。`seed` 确定性选行业骨架，
/// `expect_category` 守门（生成结果必须落在目标大类，否则 seed 选错，panic 提示）。
async fn run_twin_arc(
    agent_llm: Arc<LlmClient>,
    rp_client: Arc<LlmClient>,
    seed: usize,
    expect_category: IdentityCategory,
) {
    // ① 真生成身份（端点抖动 → None → skip 不假绿）。
    let Some(identity): Option<GeneratedIdentity> = generate_identity(&rp_client, seed).await else {
        eprintln!("skip: generate_identity(seed={seed}) 返回 None（端点抖动 / 关键字段缺失），跳过本域 arc");
        return;
    };
    assert_eq!(
        identity.category, expect_category,
        "seed={seed} 应落在大类 {:?}，实际 {:?}——select_skeleton 映射错位",
        expect_category, identity.category
    );
    let category_label = identity.category.as_str();
    eprintln!(
        "[digital-twin][{category_label}] 生成身份：profile={} persona={} opening={:?}",
        identity.profile.display_name, identity.persona.identity, identity.opening_inbound
    );

    let app = TestApp::start().await;
    let mcp = start_mcp_mock().await;
    let state = common::rebuild_app_state_with_real_llm(&app, agent_llm, mcp.uri());

    // ② seed 生成的 profile 成 default ws 的 active profile（覆盖 default 骨架的全部字段）。
    let generated_profile = identity.profile.clone();
    let profile_id = generated_profile.profile_id.clone();
    seed_active_domain_profile(&app, "default", &profile_id, move |p| {
        // 用生成 profile 的全部业务字段覆盖（保留 seed_active 设的 id/is_active/current_version）。
        let id = p.id;
        let is_active = p.is_active;
        let current_version = p.current_version;
        *p = generated_profile.clone();
        p.id = id;
        p.is_active = is_active;
        p.current_version = current_version;
        p.workspace_id = "default".to_string();
    })
    .await;

    let contact = twin_contact(&format!("dtwin_{category_label}_{seed}_user"));
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let ledger = RoleplayLedger::for_fixture(&format!("digital_twin_{category_label}"));
    let latest = || FindOneOptions::builder().sort(doc! { "created_at": -1 }).build();
    let persona = identity.persona.clone();
    let scene_goal = identity.persona.need.clone();

    let mut history: Vec<DialogueTurn> = Vec::new();
    let mut generated_turns = 0usize;
    let mut agent_replies: Vec<String> = Vec::new();
    let mut sent_turns = 0usize;
    const MAX_TURNS: usize = 4;

    for turn in 1..=MAX_TURNS {
        // 首轮用生成的开场白（标 Generated）；后续 roleplayer 按历史实时反应。
        let (customer_line, source) = if turn == 1 {
            (identity.opening_inbound.clone(), RoleplaySource::Generated)
        } else {
            let fallback = "嗯，你继续说。";
            let rp = roleplay_user_turn(&rp_client, &persona, &scene_goal, &history, fallback).await;
            (rp.message.clone(), rp.source)
        };
        if source == RoleplaySource::Generated {
            generated_turns += 1;
        }
        history.push(DialogueTurn {
            speaker: Speaker::Customer,
            text: customer_line.clone(),
        });
        ledger.append(serde_json::json!({
            "turn": turn, "role": "customer", "category": category_label,
            "source": format!("{source:?}"), "text": customer_line,
        }));

        let msg_id = format!("dtwin_{category_label}_inbound_{turn}");
        let inbound = make_inbound(&contact, &msg_id, &customer_line);
        state.db.messages().insert_one(&inbound, None).await.expect("insert inbound");
        unwrap_or_skip_transient!(
            handle_managed_message(&state, contact.clone(), &inbound).await,
            format!("[{category_label}] turn-{turn} agent 链路必须 Ok")
        );

        // 硬断言：gateway / final_review status ∈ 闭集（引擎写未知状态即红）。
        let log = state
            .db
            .agent_run_logs()
            .find_one(doc! { "contact_wxid": &contact.wxid }, latest())
            .await
            .expect("query run log")
            .expect("必须落一行 run log");
        assert!(
            GATEWAY_STATUS_VALUES.contains(&log.status.as_str()),
            "[{category_label}] turn-{turn} gateway status 必须 ∈ 闭集，实际={:?}",
            log.status
        );
        assert!(
            log.final_review_status.is_empty()
                || FINAL_REVIEW_STATUS_VALUES.contains(&log.final_review_status.as_str()),
            "[{category_label}] turn-{turn} final_review_status 必须 ∈ 闭集或空，实际={:?}",
            log.final_review_status
        );
        let sent_like = matches!(
            log.status.as_str(),
            "outbox_enqueued" | "sent" | "approved" | "allowed"
        );

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
        if sent_like && !reply_text.trim().is_empty() {
            sent_turns += 1;
            history.push(DialogueTurn {
                speaker: Speaker::Agent,
                text: reply_text.clone(),
            });
            agent_replies.push(reply_text.clone());
            // 转真人 / 身份暴露红线（任何域都不该转真人）——共享 contains_unnegated（否定剔除）。
            assert_no_handoff_or_identity_leak(
                &reply_text,
                &format!("digital-twin[{category_label}] turn-{turn}"),
            );
        }
        ledger.append(serde_json::json!({
            "turn": turn, "role": "agent", "category": category_label,
            "gateway_status": log.status, "reply_text": reply_text,
        }));
    }

    // 博弈链跑通校验：roleplayer 全程 fallback（仅首轮生成开场）→ 未验证到真实多轮博弈，skip。
    if generated_turns <= 1 {
        eprintln!("skip: [{category_label}] roleplayer 后续轮全 fallback（第三族端点抖），未验真实多轮博弈，跳过");
        return;
    }

    // arc 级硬断言：发出过回复 → contact 必留至少一项画像信号（数字分身也要记住对话方）。
    if sent_turns > 0 {
        let final_contact = state
            .db
            .contacts()
            .find_one(doc! { "wxid": &contact.wxid }, None)
            .await
            .expect("reload contact")
            .expect("contact exists");
        // 非交易域 profile（peer/formal 经 apply_category_semantics）不无条件写 value_tier，
        // 故 domain_attributes 任一键 / memory_summary / agent_profile 任一非空即算记住对话方。
        let has_signal = final_contact.memory_summary.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
            || final_contact.agent_profile.is_some()
            || final_contact.domain_attributes.as_ref().map(|d| d.keys().count() > 0).unwrap_or(false);
        assert!(
            has_signal,
            "[{category_label}] arc 跑完（发出 {sent_turns} 轮回复）后 contact 无任何画像信号——数字分身对吐露信息的对话方零记录"
        );
    }

    eprintln!(
        "✓ [{category_label}] 数字分身全链跑通：{MAX_TURNS} 轮，roleplayer 真生成 {generated_turns} 轮，agent 发出 {sent_turns} 轮，红线/闭集/画像断言通过。"
    );
}

/// peer_social 域（同行交流搭子）全链 arc。seed=5 → PeerSocial（见 industry_candidates）。
#[tokio::test]
#[ignore]
async fn digital_twin_peer_social_full_arc() {
    let (Some(agent_llm), Some(rp_client)) = (agent_client(), roleplayer_client()) else {
        eprintln!("skip: 缺 REAL_LLM_API_KEY 或 ROLEPLAYER_API_KEY（异族不全），跳过数字分身 arc");
        return;
    };
    run_twin_arc(agent_llm, rp_client, 5, IdentityCategory::PeerSocial).await;
}

/// formal_business 域（正式业务咨询）全链 arc。seed=7 → FormalBusiness。
#[tokio::test]
#[ignore]
async fn digital_twin_formal_business_full_arc() {
    let (Some(agent_llm), Some(rp_client)) = (agent_client(), roleplayer_client()) else {
        eprintln!("skip: 缺 REAL_LLM_API_KEY 或 ROLEPLAYER_API_KEY（异族不全），跳过数字分身 arc");
        return;
    };
    run_twin_arc(agent_llm, rp_client, 7, IdentityCategory::FormalBusiness).await;
}
