//! `real_llm_smoke` —— 用**真实大模型**跑真实任务的端到端 smoke。
//!
//! 与其它集成测试的关键区别：其它测试用 mock LLM（`TestLlmGenerator` 预排队
//! JSON），只验证「业务逻辑接住了正确形状的输出」；本套件把 `AppState.llm`
//! 换成真实 [`LlmClient`]（OpenAI 兼容端点，默认 deepseek-v4），验证**真模型在真实
//! prompt 下输出的 JSON 能否被 serde 解析、过五闸门、知识 agent 多轮 tool-loop
//! 在真模型下是否真的收敛、真实多模态模型能否抽出 chunk**。
//!
//! ## 红线
//! - **MCP 永远是桩**：`rebuild_app_state_with_real_llm` 把 `mcp_base_url` 指向
//!   wiremock，绝不真发微信（不可逆副作用归零）。
//! - **密钥零泄漏**：只从 env 读 `REAL_LLM_API_KEY`，断言信息不打印 key。
//! - **知识仍 needs_review**：vision 抽取出的 chunk 必断言 `draft`+`needs_review`。
//! - **env-gated**：无 `REAL_LLM_API_KEY` 时每个 test 自我跳过（eprintln + return），
//!   不 panic；默认 `#[ignore]`，本地 `cargo test` 不触网。
//!
//! ## 运行
//! ```sh
//! REAL_LLM_API_KEY=... REAL_LLM_MODEL=deepseek-v4-pro \
//!   cargo test --test real_llm_smoke -- --ignored --nocapture
//! ```
//! 缺 Docker 时 testcontainers 起不来——本套件与其它集成测试一样需要 Docker，
//! 由 GitHub CI 的 `real-llm` job 驱动（见 `.github/workflows/ci.yml`）。

mod common;

use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::agent::run_envelope::FINAL_REVIEW_STATUS_VALUES;
use wechatagent::agent::{
    atomic_claim_pending, handle_managed_message, process_entry, record_user_reaction, OutboxStatus,
};
use wechatagent::llm::{LlmClient, LlmFormat};
use wechatagent::models::{
    AgentDecisionReview, AgentStatus, Contact, ConversationMessage, LlmProviderConfig,
    MessageDirection, OperationKnowledgeChunk,
};

use crate::common::capability_evidence::CapabilityEvidence;
use crate::common::TestApp;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── env-gated 真实 provider 构造 ───────────────────────────────────────────

/// 从 env 构造真实文本 provider。缺 `REAL_LLM_API_KEY` → None（调用方自我跳过）。
///
/// timeout=180s / retries=6 / retry_base=2500ms（指数退避≈2.5+5+10+20+40+80s + 尊重 Retry-After
/// 头取 max），熬过 NVIDIA integrate 限流窗（429 风暴下少重试即 skip 假绿，[[reference_llm_backup_gpt55]]）。
/// `REAL_LLM_BASE_URL` / `REAL_LLM_MODEL` 有合理默认值。
///
/// provider 沿革（2026-06-03）：MiMo 配额耗尽 + 端点下线 → 文本默认切
/// deepseek-v4（api.supxh.xin，OpenAI 兼容 /v1，已验 json_object 可用）。
fn real_llm_from_env() -> Option<Arc<LlmClient>> {
    let api_key = std::env::var("REAL_LLM_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    let base_url = std::env::var("REAL_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.supxh.xin/v1".to_string());
    let model = std::env::var("REAL_LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".to_string());
    let client = build_real_client(base_url, api_key, model, "REAL_LLM_FORMAT", 10);
    Some(Arc::new(client))
}

/// 按 `<format_env>`（openai/anthropic，缺省 openai）构造 LlmClient。claude 系走
/// Anthropic `/v1/messages`（非流式）；gpt/其它走 OpenAI `/v1/chat/completions`。
/// 与 `real_llm_ops_smoke.rs::build_real_client` 同口径——端点切到 rsxermu666.cn
/// （主 claude-opus-4-8）时主模型走 Anthropic，避免被当 OpenAI 走错路径返回 4xx 后被
/// `unwrap_or_skip_transient!` 跳过出假绿。
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

/// vision 副模型名：`REAL_LLM_VISION_MODEL`，缺省默认专职视觉 provider 模型 `nemotron-nano-12b-v2-vl`。
///
/// 与文本端点彻底解耦——文本端点不支持多模态，T3 vision 走独立视觉
/// provider（NVIDIA integrate）。不再回退 `REAL_LLM_MODEL`（旧实现把 vision 与文本模型
/// 双关绑死，切 provider 必撞）。
fn real_vision_model() -> String {
    std::env::var("REAL_LLM_VISION_MODEL")
        .unwrap_or_else(|_| "nvidia/nemotron-nano-12b-v2-vl".to_string())
}

/// vision 端点 base_url：独立 `REAL_LLM_VISION_BASE_URL`，缺省回落通用
/// `REAL_LLM_BASE_URL`，再缺省专职视觉 provider `NVIDIA integrate`。
fn real_vision_base_url() -> String {
    std::env::var("REAL_LLM_VISION_BASE_URL")
        .or_else(|_| std::env::var("REAL_LLM_BASE_URL"))
        .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1".to_string())
}

/// vision 端点 api_key：独立 `REAL_LLM_VISION_API_KEY`，缺省回落通用
/// `REAL_LLM_API_KEY`（兼容单端点既支持文字又支持 vision 的形态）。
fn real_vision_api_key() -> String {
    std::env::var("REAL_LLM_VISION_API_KEY")
        .or_else(|_| std::env::var("REAL_LLM_API_KEY"))
        .expect("require_real_llm 已保证 REAL_LLM_API_KEY 存在")
}

/// 跳过宏：无 key 时打印一行 skip 并 `return`（不 panic、不算失败）。
macro_rules! require_real_llm {
    () => {{
        match real_llm_from_env() {
            Some(llm) => llm,
            None => {
                eprintln!("skip: REAL_LLM_API_KEY 未配置，跳过真实大模型 smoke");
                return;
            }
        }
    }};
}

/// 链路解包宏：真模型上游瞬时不可达（限流 429 / 超时等 `LlmUnavailable`）→ 打印 skip 并
/// `return`，**不算能力失败**——上游没产出内容时 agent 根本没真正运行，断言无对象，跳过
/// 而非误判崩溃（这不是放水：链路真返回内容时所有红线断言照旧执行）。其它 `Err` 仍 panic。
/// 镜像姊妹套件 `real_llm_ops_smoke.rs` / `real_llm_knowledge_quality.rs` 的同名宏，使
/// 运营 Agent 决策+审查链路（T1）、知识 tool-loop（T2）、vision 抽取（T3）的真模型回归
/// 在上游限流时产出有效信号而非永久假红。
macro_rules! unwrap_or_skip_transient {
    ($result:expr, $what:expr) => {{
        match $result {
            Ok(value) => value,
            Err(wechatagent::error::AppError::LlmUnavailable { kind, retry_count, detail, .. }) => {
                // R0.3：配错类 4xx（404 endpoint_not_found / 其它 http_4xx，除账户级 401/402）
                // **不是端点抖动**，是 baseUrl/model/path 配错（重演 t12 漏 /v1 → 405 被当抖动
                // skip 假绿的坑）。这类直接 panic 暴露，不 skip、不写 ledger。401/402（未授权/
                // 欠费）仍按瞬时处理（fork 没 key / 配额临时耗尽，独立端点可救）。
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
                     按计划「真模型抖动有限重试+跳过」处理，不算能力失败",
                    $what
                );
                // R0.2：transient-skip 可观测化——每次 skip append 一行到公共 ledger，
                // CI 跑完按 skip 率校验阈值（高 skip 假绿 → 红）。IO 失败仅诊断不影响 skip。
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

// ── wiremock MCP 成功桩（每请求唯一 newMsgId）────────────────────────────────
// 与 outbox_integration.rs 同形：gateway 把 newMsgId 写进 conversation_messages
// .message_id（sparse+unique 索引），同 id 会撞 E11000，故逐请求递增。

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
                    "newMsgId": format!("real_smoke_msg_{seq}"),
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

fn count_named_tool_calls(requests: &[wiremock::Request], tool_name: &str) -> usize {
    requests
        .iter()
        .filter(|request| {
            serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| {
                    (body.get("method").and_then(|value| value.as_str()) == Some("tools/call"))
                        .then(|| {
                            body.pointer("/params/name")
                                .and_then(|value| value.as_str())
                                .map(|name| name == tool_name)
                                .unwrap_or(false)
                        })
                })
                .unwrap_or(false)
        })
        .count()
}

fn mcp_call_sequence(requests: &[wiremock::Request]) -> Vec<String> {
    requests
        .iter()
        .filter_map(|request| {
            let body = serde_json::from_slice::<serde_json::Value>(&request.body).ok()?;
            match body.get("method").and_then(|value| value.as_str()) {
                Some("initialize") => Some("initialize".to_string()),
                Some("tools/call") => Some(format!(
                    "tools/call:{}",
                    body.pointer("/params/name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("<missing-name>")
                )),
                Some(method) => Some(method.to_string()),
                None => None,
            }
        })
        .collect()
}

fn pretty_document(document: &Document) -> String {
    serde_json::to_string_pretty(document).unwrap_or_else(|_| format!("{document:?}"))
}

fn write_t4_transcript(transcript: &serde_json::Value) {
    let rendered = serde_json::to_string_pretty(transcript).expect("render T4 transcript");
    eprintln!("\n========== T4 BUSINESS TRANSCRIPT ==========\n{rendered}\n========== END T4 BUSINESS TRANSCRIPT ==========");
    let dir =
        std::env::var("REAL_LLM_LEDGER").unwrap_or_else(|_| "target/real_llm_ledger".to_string());
    std::fs::create_dir_all(&dir).expect("create real-llm ledger directory");
    std::fs::write(format!("{dir}/t4_business_transcript.json"), rendered)
        .expect("write T4 transcript artifact");
}

fn document_number(document: &Document, key: &str) -> Option<f64> {
    document.get(key).and_then(|value| match value {
        mongodb::bson::Bson::Int32(value) => Some(*value as f64),
        mongodb::bson::Bson::Int64(value) => Some(*value as f64),
        mongodb::bson::Bson::Double(value) => Some(*value),
        _ => None,
    })
}

fn reaction_confidence_is_high(document: &Document) -> bool {
    let Some(confidence) = document_number(document, "confidence") else {
        return false;
    };
    if confidence <= 1.0 {
        confidence >= 0.7
    } else {
        confidence >= 7.0
    }
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

fn assert_short_single_question(reply: &str, round: &str) {
    let trimmed = reply.trim();
    let char_count = trimmed.chars().count();
    assert!(
        (6..=180).contains(&char_count),
        "{round} 回复必须简短但有实质内容（6..=180 字），实际 {char_count} 字：{trimmed:?}"
    );
    let question_count = trimmed
        .chars()
        .filter(|ch| matches!(ch, '?' | '？'))
        .count();
    assert_eq!(
        question_count, 1,
        "{round} 必须提出且只提出一个清晰问题，实际问号数={question_count}：{trimmed:?}"
    );
    for forbidden in [
        "转人工",
        "人工客服",
        "人工处理",
        "稍后回复",
        "晚点回复",
        "请等待",
        "请稍候",
        "无法帮助",
    ] {
        assert!(
            !trimmed.contains(forbidden),
            "{round} 不得以转人工/拖延/拒绝逃避当前问题，命中 {forbidden:?}：{trimmed:?}"
        );
    }
}

fn assert_review_quality(review: &AgentDecisionReview, round: &str) {
    assert!(
        review.approved,
        "{round} Reviewer 必须最终批准；summary={:?}, risks={:?}, scores={}",
        review.review_summary,
        review.risks,
        pretty_document(&review.scores)
    );
    assert!(
        review.risks.is_empty(),
        "{round} 是无事实声明、无施压的简单澄清问句，不应残留风险：{:?}",
        review.risks
    );

    let score = |key: &str| {
        document_number(&review.scores, key).unwrap_or_else(|| {
            panic!(
                "{round} Reviewer 缺少评分 {key}: {}",
                pretty_document(&review.scores)
            )
        })
    };
    let threshold = |key: &str| {
        document_number(&review.runtime_parameters_snapshot, key).unwrap_or_else(|| {
            panic!(
                "{round} 缺少运行阈值 {key}: {}",
                pretty_document(&review.runtime_parameters_snapshot)
            )
        })
    };
    assert!(
        score("hallucinationScore") < threshold("factRiskBlockAt"),
        "{round} 事实风险评分未过闸"
    );
    assert!(
        score("humanLike") >= threshold("humanLikeRewriteBelow"),
        "{round} 人味评分未过闸"
    );
    assert!(
        score("emotionalValue") >= threshold("emotionalValueRewriteBelow"),
        "{round} 情绪价值评分未过闸"
    );
    assert!(
        score("knowledgeGroundingScore") >= threshold("productAccuracyBlockBelow"),
        "{round} grounding 评分未过闸"
    );
    let pressure = score("pressureRisk");
    assert!(
        pressure == 0.0 || pressure < threshold("pressureRiskBlockAt"),
        "{round} 压迫风险评分未过闸"
    );
    let boundary = score("boundaryPrivacySafety");
    assert!(
        boundary == 0.0 || boundary > 3.0,
        "{round} 边界/隐私评分未过闸"
    );
}

// ── fixtures ────────────────────────────────────────────────────────────────

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("真实 smoke 客户".to_string()),
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

async fn seed_verified_chunk(app: &TestApp, workspace_id: &str, title: &str, body: &str) -> String {
    let id = ObjectId::new();
    let now = DateTime::now();
    let chunk = OperationKnowledgeChunk {
        id: Some(id),
        workspace_id: workspace_id.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("product_capability".to_string()),
        title: title.to_string(),
        summary: Some(body.to_string()),
        body: Some(body.to_string()),
        source_quote: Some(body.to_string()),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(88),
        status: "active".to_string(),
        priority: 10,
        created_at: now,
        updated_at: now,
        wiki_type: Some("methodology".to_string()),
        dynamic_confidence: Some(0.9),
        chunk_type: "product_fact".to_string(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert verified chunk");
    id.to_hex()
}

/// 测试内嵌的一张含可读英文文字（"Refund Policy / Window: 7 days /
/// Contact: support"）的 1-bit PNG，base64（无 data-uri 前缀）。
/// 由 PIL 生成（360×140，1-bit，~1KB）。
const TEXT_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAWgAAACMAQAAAAB5PA5YAAAD50lEQVR42u2XT2gcVRzHPzM7yY5tbIYiskJkNx6LlkUjKKTJaykiUjF6UgT/IejBQ60gpcTuSwnSk0Tw4sEm0NJjLSh6qrwk1QSNuIiIlEInEN09CJ3oJm7W2fl52EhF8l5dRVDYd5vhw2++7/dnfr+fJ3RxfHp0j+7R/3vaC4BpANaYD0AOaBsuAiKSExGRmhgqkk2K5fgwWweGAEjpmzBgHLaLaUVkfNt2LRuXrOKwDbD9x0qJAYzjlto3eJ2nALwYlIvO1MLWkpp5UK1PApzikp5ea++kPQBg/DgqWi4ml0FKlbaq+M+hLbaRoc/KJskntauUkFFjTpZ1alOydvIFL1RRQS9fJm5lh1Dpyh7PQq8O48WYhAeilK1wdCo22qdp8bfON+XMWGW2+FWxJsZrypnJV2Vr3RbLkOtTmIQmKX3PQGJAfFsOfsheQUUlCLhrDiKlIbHdsgTrmCQOSYnxSIwmCGz0HiRBRTQJAGFGwY7uxu8kiWCSEnScrM1p0bHNdjjNMMzEoV4DArSqsBDa6M5nE5pJSgkqGDJlrPk9VhgbGyiOLy3mZUmy28Ym03I2aPG39r36Qa9xNA7vTAHvR2VOVb3IqiSeqrAY0YxeBJjSystbK23nM+6otH/495Gu6E3VDb1j6eBZppmf9nRD9zpJj+7RN6PbokagBcBUibqb9nVXSjIGfn+qlNx0ADy7+gf94c1u6TXe1revN+pVRLWfXKjXQzvtzUv/xqV7v36IL8n0xvJj1I44bKfl1iPphVvjqwD+x/t1dt7p79Pmeb+qydGmcYsaiPqd88nmBP0QlQAKxzCuaeYKvgEOb89ekfaXXUoKYQK83w8tkLE3h084Pbg5gU9UUgChQh930IfZZdjE73Tf7zLarugkA80kR3nuLDQYgJxyZ2z/BE/8DG0GyB5+B3+fi/ZbJvftfoOmwe6nXven1mz0Dr2hnfv3Ku37rui9drrX03r0f4Be153VLLvxNrbvgJ8WO4Po1o0JdNa+A1452o2SOfXXhYuMX1/8YWgwn8QVyc++OzgjgzN5qxIhenTXsXtGv3n8ZTmiciMjB0buL9iVKLn7tYPpR7tDsvMX0wtfxBc+1y5/H4oKfVR1gf7oXKjeCJWrA0IDfxkFaq5FFarOWHrQPgT4yyYIUIHDg56pywrpJuCdIE3VxV/LLtsxI9tL9/HyOUzkOZWERNBkjjbVp0ncG/rE/CevAOU5oDzdUmdbFx30fTMrAdcmNqTu79sYij6gHCX2yCeVxV9qb92RpC9JfvW9YlUGry1ZIv/nXWrLmq077VK5riqtTbkLOrCHvdcbenSP/tv0b6fIkB+gBlVDAAAAAElFTkSuQmCC";

// ── T1 · 真实文本决策 + 审查链路 ───────────────────────────────────────────

/// 真模型跑 Reply Agent + Review Agent 全链路。MCP 用 wiremock 桩。
///
/// 验证点（核心）：真模型输出的 JSON 能被 serde 解析、过五闸门、闸门不崩；
/// `agent_run_logs` 落一行且 `final_review_status` ∈ 闭集；若 approved →
/// outbox 入队 → `process_entry` 推到 `sent`（命中 MCP 桩）。
#[tokio::test]
#[ignore]
async fn t1_real_text_decision_review_chain() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());

    let contact = managed_contact("real_smoke_user_t1");
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let inbound = make_inbound(
        &contact,
        "real_smoke_msg_t1",
        "你好，我想了解下你们的产品大概多久能上线？预算大概要准备多少？",
    );
    state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert inbound");

    // 真实链路：决策 → 审查 →（可选 revision）→ outbox。真模型抖动/限流时
    // LlmClient 自带 retries=3；耗尽后上游仍 429/超时 → skip（不算能力失败），
    // 链路真返回 Ok 时下方所有红线断言照旧执行。
    unwrap_or_skip_transient!(
        handle_managed_message(&state, contact.clone(), &inbound).await,
        "真实大模型决策+审查链路（T1）"
    );

    // agent_run_logs 必落一行，且 final_review_status 是闭集内合法枚举。
    let log = state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query agent_run_logs")
        .expect("真实链路必须落一行 agent_run_logs");

    assert!(
        FINAL_REVIEW_STATUS_VALUES.contains(&log.final_review_status.as_str()),
        "final_review_status 必须 ∈ 闭集，实际 = {:?}",
        log.final_review_status
    );
    eprintln!(
        "[t1] final_review_status={} llm_calls_used={} revision_applied={}",
        log.final_review_status, log.llm_calls_used, log.revision_applied
    );

    // 若真模型这轮决定回复且过闸（approved 系列）→ outbox 应入队，
    // 推一次 dispatcher，命中 MCP 桩后状态必须是 sent。
    let outbox_entry = state
        .db
        .collection_agent_send_outbox()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("query outbox");

    if let Some(entry) = outbox_entry {
        // 有 outbox 行（approved 路径）：claim + process 必须推进到 sent。
        let entry_id = entry.id.expect("outbox _id");
        let claimed = atomic_claim_pending(&state, "real_smoke_worker_t1", 60)
            .await
            .expect("claim pending")
            .expect("刚入队的 outbox 必须能被 claim 到");
        assert_eq!(claimed.id, Some(entry_id), "claim 到的应是刚入队那条");
        process_entry(&state, &claimed)
            .await
            .expect("process_entry");

        let after =
            common::wait_for_outbox_processed(&state, entry_id, Duration::from_secs(10)).await;
        assert_eq!(
            after.status,
            OutboxStatus::Sent.as_str(),
            "命中 MCP 成功桩后 outbox 必须 sent，实际 {:?}",
            after.status
        );
        eprintln!("[t1] outbox → sent（真模型 approved 并经桩 MCP 完成投递）");
    } else {
        // 无 outbox 行：真模型这轮选择不回复 / 被闸门 hold —— 也是合法终态，
        // 只要 final_review_status 在闭集内即可（上面已断言）。
        eprintln!(
            "[t1] 本轮无 outbox（final_review_status={}）—— 合法的不发终态",
            log.final_review_status
        );
    }
}

// ── T2 · 真实知识 agent tool-loop ──────────────────────────────────────────

/// 真模型跑 list_catalog → open_chunk → answer 渐进式披露循环。
///
/// 验证点：真模型能驱动 tool-loop 收敛（rounds_used ≥ 1）；answer 非空；
/// cite 的 chunk id ⊆ seed id（真模型不应引用不存在的 chunk）。
#[tokio::test]
#[ignore]
async fn t2_real_knowledge_tool_loop_converges() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    // 知识链路不发消息，但仍把 llm 换成真实 provider（MCP 桩占位即可）。
    let mcp_server = start_mcp_mock_success().await;
    let state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());

    let ws = state.config.default_workspace_id.clone();
    let id1 = seed_verified_chunk(
        &app,
        &ws,
        "退款政策",
        "下单后 7 天内可无理由退款，需保持商品完好。超过 7 天按损耗比例处理。",
    )
    .await;
    let id2 = seed_verified_chunk(
        &app,
        &ws,
        "实施周期",
        "标准实施周期为 2 到 4 周：第 1~2 周梳理流程并接通试点，第 3~4 周扩到核心场景。",
    )
    .await;
    let id3 = seed_verified_chunk(
        &app,
        &ws,
        "对接方式",
        "支持 OpenAPI 与 Webhook 两种对接方式，提供测试沙箱与示例代码。",
    )
    .await;
    let seed_ids = [id1, id2, id3];

    let req = AnswerRequest {
        workspace_id: ws.clone(),
        // seed 的 chunk 落在 account_id=None（全局 chunk，见 seed_verified_chunk），
        // list_catalog 在 account_id=None 时匹配 {account_id: null}，命中本测试 seed。
        // 与已验证通过的 knowledge_ask_e2e.rs 同形（账户作用域用全局 null 池）。
        account_id: None,
        query: "你们的退款政策是怎样的？".to_string(),
        filter: CatalogFilter::default(),
        max_rounds: None,
    };

    let result = unwrap_or_skip_transient!(
        answer(&state, req).await,
        "真实知识 agent answer（T2 tool-loop）"
    );

    eprintln!(
        "[t2] rounds_used={} truncated={} cited={:?} answer.len={}",
        result.rounds_used,
        result.truncated,
        result.cited_chunk_ids,
        result.answer.chars().count()
    );

    assert!(
        result.rounds_used >= 1,
        "真模型必须至少跑 1 轮 tool-loop，实际 rounds_used={}",
        result.rounds_used
    );
    assert!(!result.answer.trim().is_empty(), "真模型 answer 不应为空");

    // tool_trace 第一步必须是 list_catalog（渐进式披露入口）。
    let first_tool = result
        .tool_trace
        .first()
        .and_then(|d| d.get_str("tool").ok().map(str::to_string));
    assert_eq!(
        first_tool.as_deref(),
        Some("list_catalog"),
        "tool_trace 首步必须是 list_catalog，实际 {:?}",
        first_tool
    );

    // 引用约束（红线）：真模型 cite 的 chunk id 必须 ⊆ seed 集合，
    // 绝不能凭空引用不存在的 chunk。
    for cited in &result.cited_chunk_ids {
        assert!(
            seed_ids.contains(cited),
            "真模型 cite 了不存在的 chunk id={}，seed={:?}",
            cited,
            seed_ids
        );
    }
}

// ── T3 · 真实多模态 vision 抽取 ────────────────────────────────────────────

/// 真多模态模型从图片抽取 chunk。走「专职视觉副模型」分支（seed 一个
/// `LlmProviderConfig{supports_vision, is_vision_active}` → handler 临时构造
/// 真实 vision client）。
///
/// 软断言：真模型抽取不保证命中——抽出 chunk 或 fence 空都通过。
/// **硬断言只锁红线**：任何落库 chunk 必 `draft` + `needs_review`。
#[tokio::test]
#[ignore]
async fn t3_real_vision_extraction_keeps_needs_review() {
    let mut evidence = CapabilityEvidence::new("t3_vision_artifact");
    // vision 也需要真实 key（副模型走真实 HTTP）。
    let _llm = require_real_llm!();
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed 专职视觉副模型，文字主模型不存在 → handler 走 Dedicated 分支，
    // 用这条配置真实构造 vision client。vision 端点独立配置：deepseek 文字端点
    // 不支持多模态，故走 REAL_LLM_VISION_* 三元组（默认 NVIDIA integrate），缺 VISION
    // key 时回落通用 REAL_LLM_API_KEY/BASE_URL。
    let api_key = real_vision_api_key();
    let vision_cfg = LlmProviderConfig {
        id: Some(ObjectId::new()),
        workspace_id: ws.clone(),
        provider_id: "real_vision".to_string(),
        name: "real_vision".to_string(),
        format: "openai".to_string(),
        base_url: real_vision_base_url(),
        api_key,
        model: real_vision_model(),
        is_active: false,
        timeout_seconds: Some(180),
        max_retries: Some(10),
        retry_base_ms: Some(2500),
        supports_vision: true,
        is_vision_active: true,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    app.state
        .db
        .llm_provider_configs()
        .insert_one(&vision_cfg, None)
        .await
        .expect("insert vision provider");

    use axum::extract::State;
    use axum::{Extension, Json};
    use wechatagent::auth::AuthenticatedAdmin;
    use wechatagent::routes::ext_knowledge::{
        import_operation_knowledge_apply_image, ImportApplyImageRequest,
    };

    let admin = Extension(AuthenticatedAdmin {
        user_id: "real_smoke_admin".into(),
        username: "real_smoke_admin".into(),
        current_workspace: ws.clone(),
    });
    let req = ImportApplyImageRequest {
        image_base64: TEXT_PNG_BASE64.to_string(),
        mime: Some("image/png".to_string()),
        source_name: Some("real_smoke_image".to_string()),
        account_id: None,
        hint: Some("退款政策图片".to_string()),
    };

    evidence.attempted();
    let resp = unwrap_or_skip_transient!(
        import_operation_knowledge_apply_image(State(app.state.clone()), admin, Json(req)).await,
        "真实 vision 抽取（T3）"
    );
    let body = resp.0;
    let chunk_ids = body["chunkIds"].as_array().cloned().unwrap_or_default();
    evidence.observe_llm_calls(1);
    eprintln!(
        "[t3] vision chunkIds={} fallbackBlob={:?} note={:?}",
        chunk_ids.len(),
        body.get("fallbackBlob"),
        body.get("note")
    );

    assert!(
        !chunk_ids.is_empty(),
        "T3 vision 必须产出至少一条可复查 chunk"
    );
    // 硬断言（红线）：任何落库 chunk 必 draft + needs_review。
    for id in &chunk_ids {
        let id_hex = id.as_str().expect("chunkId str");
        let chunk = app
            .state
            .db
            .operation_knowledge_chunks()
            .find_one(
                doc! {
                    "_id": ObjectId::parse_str(id_hex).expect("parse oid"),
                    "workspace_id": &ws,
                },
                None,
            )
            .await
            .expect("query chunk")
            .expect("chunk exists");
        assert_eq!(
            chunk.status, "draft",
            "vision chunk 必须 draft（AI 永不自动 verify）"
        );
        assert_eq!(
            chunk.integrity_status.as_deref(),
            Some("needs_review"),
            "vision chunk 必须 needs_review（AI 永不自动 verify）"
        );
    }
    evidence.branch("vision_chunks_persisted");
    evidence.detail("chunk_count", chunk_ids.len());
    evidence.pass(chunk_ids.len(), 1 + chunk_ids.len() * 2);
}

// ── T4 · 真实 Reply/Review → 发送 → Reaction 停止 → 待发取消 ─────────────

/// 严格业务任务门：Reply / Review / Reaction 全部使用真实模型，MCP 仅使用本地桩。
///
/// 与 T1/T9 的“合法不回复可跳过”不同，本测试要求两轮 Reply/Review 都实际产生文本
/// outbox：第一轮完成投递，第二轮保持 pending；随后用户明确要求停止联系，真实
/// Reaction 必须判为 stop/unsubscribe 并在 dispatcher 调 MCP 前取消第二轮。任何一轮
/// 不回复、Review hold、Reaction 未识别停止或发生第二次客户投递都会直接失败。
#[tokio::test]
#[ignore]
async fn t4_real_reply_review_reaction_stop_cancels_before_second_send() {
    let llm = require_real_llm!();
    let app = TestApp::start().await;
    let mcp_server = start_mcp_mock_success().await;
    let mut state = common::rebuild_app_state_with_real_llm(&app, llm, mcp_server.uri());
    // 让每轮模型回复对应恰好一条文本 outbox，便于把“发送次数”与业务轮次精确对账。
    state.config.agent_reply_max_segments = 1;
    state.config.agent_reply_max_segment_chars = 2_000;
    // 本任务要连续验证两轮 Reply/Review，再验证 Reaction 在第二条发送前取消。
    // 默认 20 秒最小回复间隔属于生产防刷屏策略，会在模型调用前把第二轮挡成
    // rate_limited，与本测试要验证的 Reply/Review/Reaction 所有权弧无关。
    state.config.agent_min_reply_interval_seconds = 0;

    let mut contact = managed_contact("real_smoke_user_t4_stop");
    contact.custom_agent_instructions = Some(
        "客户正在梳理职业目标。每轮必须结合客户刚说的具体信息，只问一个推进思考的关键问题；回复简短自然，不要沉默、暂缓或转人工。"
            .to_string(),
    );
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    let round1_text =
        "我最近想换工作，但方向很乱。请先帮我把职业目标理清，只问我一个最关键的问题。";
    let round2_text = "我更在意未来三年的成长，不想只看眼前工资。请继续只问一个问题，帮我在岗位方向和行业方向之间收窄。";
    let stop_text = "不要再联系我了，停止发送任何消息，请取消刚才准备发的内容。";
    let mut transcript = serde_json::json!({
        "test": "t4_real_reply_review_reaction_stop_cancels_before_second_send",
        "purpose": "人工审阅真实 Reply/Review/Reaction 内容与 outbox 执行轨迹；MCP 为桩，不真发微信",
        "conversation": [
            { "role": "customer", "messageId": "real_smoke_t4_round_1", "content": round1_text },
            { "role": "customer", "messageId": "real_smoke_t4_round_2", "content": round2_text },
            { "role": "customer", "messageId": "real_smoke_t4_stop", "content": stop_text }
        ],
        "round1": null,
        "round2": null,
        "reaction": null,
        "execution": null
    });
    write_t4_transcript(&transcript);

    // 第一轮必须由真实 Reply + Review 产出 outbox，并经 MCP 桩实际提交一次。
    let inbound1 = make_inbound(&contact, "real_smoke_t4_round_1", round1_text);
    state
        .db
        .messages()
        .insert_one(&inbound1, None)
        .await
        .expect("insert inbound1");
    if let Err(err) = handle_managed_message(&state, contact.clone(), &inbound1).await {
        transcript["round1"] = serde_json::json!({ "pipelineError": err.to_string() });
        write_t4_transcript(&transcript);
        panic!("T4 第一轮真实 Reply/Review 失败，不允许 transient skip: {err:?}");
    }
    let run1 = state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "source_event_id": "real_smoke_t4_round_1",
            },
            None,
        )
        .await
        .expect("query round1 run log");
    let review1 = state
        .db
        .decision_reviews()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "inbound_message_id": "real_smoke_t4_round_1",
            },
            None,
        )
        .await
        .expect("query round1 review");
    let first = state
        .db
        .collection_agent_send_outbox()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "source_event_id": "real_smoke_t4_round_1",
                "media_asset_id": null,
                "referral_card_id": null,
            },
            None,
        )
        .await
        .expect("query first outbox");
    transcript["round1"] = serde_json::json!({
        "runLog": run1,
        "decisionReview": review1,
        "outboxBeforeDispatch": first,
    });
    write_t4_transcript(&transcript);
    let run1 = run1.expect("T4 第一轮必须落 agent_run_log");
    let review1 = review1.expect("T4 第一轮必须落 decision_review");
    let first = first.expect("T4 第一轮必须由真实 Reply/Review 产生文本 outbox，不能以不回复假绿");
    let first_id = first.id.expect("first outbox id");
    let first_decision_id = first.decision_id.expect("first decision id");
    let claimed = atomic_claim_pending(&state, "real-smoke-t4-first", 60)
        .await
        .expect("claim first")
        .expect("first outbox must be claimable");
    assert_eq!(claimed.id, Some(first_id), "必须 claim 第一轮文本 outbox");
    process_entry(&state, &claimed)
        .await
        .expect("send first outbox");
    let first_after =
        common::wait_for_outbox_processed(&state, first_id, Duration::from_secs(10)).await;
    transcript["round1"]["outboxAfterDispatch"] =
        serde_json::to_value(&first_after).expect("serialize round1 outbox after dispatch");
    write_t4_transcript(&transcript);

    // 用第一轮发送后的最新 contact 跑第二轮；第二轮必须真实产生 outbox，但故意不 dispatch。
    contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("reload contact")
        .expect("contact exists");
    let inbound2 = make_inbound(&contact, "real_smoke_t4_round_2", round2_text);
    state
        .db
        .messages()
        .insert_one(&inbound2, None)
        .await
        .expect("insert inbound2");
    if let Err(err) = handle_managed_message(&state, contact.clone(), &inbound2).await {
        transcript["round2"] = serde_json::json!({ "pipelineError": err.to_string() });
        write_t4_transcript(&transcript);
        panic!("T4 第二轮真实 Reply/Review 失败，不允许 transient skip: {err:?}");
    }
    let run2 = state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "source_event_id": "real_smoke_t4_round_2",
            },
            None,
        )
        .await
        .expect("query round2 run log");
    let review2 = state
        .db
        .decision_reviews()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "inbound_message_id": "real_smoke_t4_round_2",
            },
            None,
        )
        .await
        .expect("query round2 review");
    let second = state
        .db
        .collection_agent_send_outbox()
        .find_one(
            doc! {
                "contact_wxid": &contact.wxid,
                "source_event_id": "real_smoke_t4_round_2",
                "media_asset_id": null,
                "referral_card_id": null,
            },
            None,
        )
        .await
        .expect("query second outbox");
    transcript["round2"] = serde_json::json!({
        "runLog": run2,
        "decisionReview": review2,
        "outboxBeforeStop": second,
    });
    write_t4_transcript(&transcript);
    let run2 = run2.expect("T4 第二轮必须落 agent_run_log");
    let review2 = review2.expect("T4 第二轮必须落 decision_review");
    let second =
        second.expect("T4 第二轮必须由真实 Reply/Review 产生文本 outbox，不能以不回复假绿");
    let second_id = second.id.expect("second outbox id");

    // 第三条只进入真实 Reaction，不再跑 Reply；明确停止后第二轮必须在 MCP 前取消。
    let stop = make_inbound(&contact, "real_smoke_t4_stop", stop_text);
    state
        .db
        .messages()
        .insert_one(&stop, None)
        .await
        .expect("insert stop inbound");
    if let Err(err) = record_user_reaction(&state, &contact, &stop).await {
        transcript["reaction"] = serde_json::json!({ "pipelineError": err.to_string() });
        write_t4_transcript(&transcript);
        panic!("T4 真实 Reaction 停止识别失败，不允许 transient skip: {err:?}");
    }

    let reacted = state
        .db
        .decision_reviews()
        .find_one(doc! { "_id": first_decision_id }, None)
        .await
        .expect("reload first review")
        .expect("first review exists");
    let second_after = state
        .db
        .collection_agent_send_outbox()
        .find_one(doc! { "_id": second_id }, None)
        .await
        .expect("reload second outbox")
        .expect("second outbox exists");
    let post_stop_claim = atomic_claim_pending(&state, "real-smoke-t4-must-not-send", 60)
        .await
        .expect("post-stop claim");
    let requests = mcp_server
        .received_requests()
        .await
        .expect("received MCP requests");
    let sequence = mcp_call_sequence(&requests);
    let mcp_bodies: Vec<serde_json::Value> = requests
        .iter()
        .filter_map(|request| serde_json::from_slice(&request.body).ok())
        .collect();
    transcript["reaction"] = serde_json::json!({
        "claimedSentReviewId": first_decision_id.to_hex(),
        "outcomeStatus": reacted.outcome_status,
        "analysis": reacted.reaction_analysis,
        "reviewAfterReaction": reacted,
    });
    transcript["round2"]["outboxAfterStop"] =
        serde_json::to_value(&second_after).expect("serialize round2 outbox after stop");
    transcript["execution"] = serde_json::json!({
        "mcpCallSequence": sequence,
        "mcpJsonRpcBodies": mcp_bodies,
        "messageSendTextCalls": count_named_tool_calls(&requests, "message_send_text"),
        "postStopClaimedEntry": post_stop_claim,
    });
    // Iron rule: 完整证据先输出并落 artifact，再做任何内容/评分/状态断言。
    write_t4_transcript(&transcript);

    let reply1 = first.content.trim();
    let reply2 = second.content.trim();
    assert_short_single_question(reply1, "第一轮");
    assert_short_single_question(reply2, "第二轮");
    assert!(
        contains_any(
            reply1,
            &["职业", "工作", "方向", "目标", "岗位", "行业", "优先"]
        ),
        "第一轮必须针对职业目标混乱，而不是泛泛寒暄：{reply1:?}"
    );
    assert!(
        contains_any(reply2, &["成长", "三年", "岗位", "行业", "方向", "能力"]),
        "第二轮必须承接三年成长与岗位/行业收窄：{reply2:?}"
    );
    assert_ne!(reply1, reply2, "两轮不得复读同一句模板");
    assert_review_quality(&review1, "第一轮");
    assert_review_quality(&review2, "第二轮");
    for (round, run) in [("第一轮", &run1), ("第二轮", &run2)] {
        assert!(
            run.llm_calls_used >= 2,
            "{round} 必须真实调用 Reply + Reviewer，实际 llm_calls_used={}",
            run.llm_calls_used
        );
        assert!(
            matches!(
                run.final_review_status.as_str(),
                "approved" | "revision_applied_approved"
            ),
            "{round} 最终评审状态必须批准，实际={:?}",
            run.final_review_status
        );
        assert!(
            run.degraded_reasons.is_empty(),
            "{round} 不允许靠降级路径通过：{:?}",
            run.degraded_reasons
        );
    }
    assert_eq!(first_after.status, OutboxStatus::Sent.as_str());
    assert_eq!(second.status, OutboxStatus::Pending.as_str());
    assert!(
        matches!(
            reacted.outcome_status.as_deref(),
            Some("user_replied_stop_requested" | "user_replied_unsubscribed")
        ),
        "真实 Reaction 必须识别明确停止/退订，实际={:?} analysis={}",
        reacted.outcome_status,
        pretty_document(&reacted.reaction_analysis)
    );
    assert!(
        reacted
            .reaction_analysis
            .get_bool("stopRequested")
            .unwrap_or(false),
        "Reaction outcome 与 stopRequested 标志必须一致：{}",
        pretty_document(&reacted.reaction_analysis)
    );
    assert!(
        reaction_confidence_is_high(&reacted.reaction_analysis),
        "如此明确的停止原话必须高置信识别：{}",
        pretty_document(&reacted.reaction_analysis)
    );
    assert_eq!(second_after.status, OutboxStatus::Canceled.as_str());
    assert_eq!(
        second_after.cancel_reason.as_deref(),
        Some("user_reaction_stop_requested")
    );
    assert!(
        post_stop_claim.is_none(),
        "停止后的第二轮 outbox 不得再被 claim"
    );
    assert_eq!(
        count_named_tool_calls(&requests, "message_send_text"),
        1,
        "只有第一轮允许客户投递；第二轮必须被 Reaction 在 MCP 前取消"
    );
}
