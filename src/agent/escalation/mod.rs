//! 决策请示通道（Principal Decision Channel）。
//!
//! 运营 Agent 撞"决策墙"（超职权 / 高风险件 / 多轮卡死）时，向幕后真人决策源
//! 请示，拿到裁决后用 AI 口吻向客户转述。客户永远只跟 Agent 对话——真人是
//! 幕后决策源，绝不直接面对客户。这不是真人下场：AI 向内部决策源请示，转述仍由 AI 完成。

mod holding_reply;
mod labels;
mod ledger;
mod logic;
mod policy;

pub(crate) use holding_reply::generate_holding_reply;
pub(crate) use ledger::*;
pub(crate) use logic::*;
pub(crate) use policy::*;
// fallback_holding_reply 需 crate 外可见（tests/principal_decision_channel.rs §14.9b
// 红线测试在 crate 外断言兜底文案不含转接类措辞）；pub(crate) use logic::* 会把它降级，
// 故单独 pub re-export 还原其原始 `pub` 可见性。
pub use logic::fallback_holding_reply;

/// Materialize durable principal-relay intents that survived an interrupted
/// resolve request. The task worker calls this once per tick; the public seam
/// also lets integration tests exercise the exact recovery path.
pub async fn reconcile_pending_relay_intents(state: &AppState) -> AppResult<u64> {
    ledger::reconcile_pending_relay_intents_once(state).await
}

pub async fn reconcile_principal_escalation_intents(state: &AppState) -> AppResult<u64> {
    ledger::reconcile_principal_escalation_intents_once(state).await
}

pub async fn reconcile_principal_card_deliveries(state: &AppState) -> AppResult<u64> {
    ledger::reconcile_principal_card_deliveries_once(state).await
}

use super::generate_agent_json;
use super::outbox::{enqueue as outbox_enqueue, EnqueueOutcome, EnqueueRequest};
use super::run_envelope::{SOURCE_KIND_FOLLOW_UP_TASK, SOURCE_KIND_PRINCIPAL_CLARIFICATION};
use super::types::{AgentDecision, DecisionReviewResult};
use crate::error::AppResult;
use crate::models::{
    AgentDecisionReview, AgentPrincipalEscalation, AgentTask, Contact, OperationDomainConfig,
    PrincipalDecision, ESCALATION_CATEGORY_HIGH_RISK_GATED, EXEMPTION_TYPE_NONE,
    PRINCIPAL_VERDICT_CONDITIONAL, PRINCIPAL_VERDICT_DEFERRED,
};
use crate::prompts;
use crate::routes::AppState;
use mongodb::bson::{doc, to_document, DateTime, Document};

pub(crate) const PRINCIPAL_INTENT_STATUS_PENDING: &str = "pending";
pub(crate) const PRINCIPAL_INTENT_STATUS_PROCESSING: &str = "processing";
pub(crate) const PRINCIPAL_INTENT_STATUS_RETRY: &str = "retry";
pub(crate) const PRINCIPAL_INTENT_STATUS_MATERIALIZED: &str = "materialized";
pub(crate) const PRINCIPAL_INTENT_STATUS_DEDUPLICATED: &str = "deduplicated";
pub(crate) const PRINCIPAL_INTENT_STATUS_INVALID: &str = "invalid";

/// Return the model-selected principal handoff only when its transport schema is
/// complete. This validates structure, not customer wording or semantic intent.
pub(crate) fn explicit_principal_escalation_request(
    decision: &AgentDecision,
) -> Option<&crate::models::EscalationRequest> {
    decision.escalation_request.as_ref().filter(|request| {
        request.needed
            && request.category.as_deref().is_some_and(|category| {
                crate::models::ALLOWED_ESCALATION_CATEGORY.contains(&category)
            })
            && request
                .reason
                .as_deref()
                .is_some_and(|reason| !reason.trim().is_empty())
            && request
                .question_for_principal
                .as_deref()
                .is_some_and(|question| !question.trim().is_empty())
    })
}

/// Freeze one durable ask-human intent into the decision review written by the
/// production transaction. AI owns whether an explicit request is needed;
/// deterministic code only supplies the configured held-decision fallback and
/// merges approval-required media into the same recoverable request.
pub(crate) fn build_principal_escalation_intent(
    decision: &AgentDecision,
    review: &DecisionReviewResult,
    domain_config: Option<&OperationDomainConfig>,
    authorization_disposition: &str,
    blocked_status: &str,
    principal_media_titles: &[String],
    now: DateTime,
) -> Option<Document> {
    let media_is_relevant =
        authorization_disposition == "authorized" && !principal_media_titles.is_empty();
    let (mut request, mut source) =
        if let Some(explicit) = explicit_principal_escalation_request(decision) {
            (explicit.clone(), "explicit_model_request".to_string())
        } else if authorization_disposition != "authorized"
            && domain_config.is_some_and(|config| {
                let policy = resolve_ask_human_policy(config);
                should_escalate_held(blocked_status, &policy)
            })
        {
            let reason = if !review.hold_reason.trim().is_empty() {
                review.hold_reason.trim().to_string()
            } else if !review.review_summary.trim().is_empty() {
                review.review_summary.trim().to_string()
            } else {
                "当前回复涉及需要有权人员确认的业务边界".to_string()
            };
            (
                crate::models::EscalationRequest {
                    needed: true,
                    category: Some(ESCALATION_CATEGORY_HIGH_RISK_GATED.to_string()),
                    reason: Some(reason),
                    question_for_principal: Some(format!(
                        "请确认这次应采用的客户回复口径。当前暂停原因：{}。",
                        labels::blocked_status_zh(blocked_status)
                    )),
                    self_serviceable_part: None,
                    is_generalizable: false,
                },
                "held_decision_fallback".to_string(),
            )
        } else if media_is_relevant {
            (
                media_approval_request(principal_media_titles),
                "principal_media_approval".to_string(),
            )
        } else {
            return None;
        };

    if media_is_relevant && source != "principal_media_approval" {
        append_media_approval(&mut request, principal_media_titles);
        source.push_str("_with_media");
    }

    let request = to_document(&request).ok()?;
    Some(doc! {
        "protocol_version": 1i32,
        "status": PRINCIPAL_INTENT_STATUS_PENDING,
        "source": source,
        "request": request,
        "attempts": 0i64,
        "claim_generation": 0i64,
        "next_retry_at": now,
        "created_at": now,
        "updated_at": now,
    })
}

fn media_approval_request(titles: &[String]) -> crate::models::EscalationRequest {
    let names = media_title_summary(titles);
    crate::models::EscalationRequest {
        needed: true,
        category: Some(crate::models::ESCALATION_CATEGORY_OUT_OF_SCOPE.to_string()),
        reason: Some(format!("本轮拟发送的素材需要负责人核准：{names}")),
        question_for_principal: Some(format!("是否同意向该客户发送这些素材：{names}？")),
        self_serviceable_part: None,
        is_generalizable: false,
    }
}

fn append_media_approval(request: &mut crate::models::EscalationRequest, titles: &[String]) {
    let names = media_title_summary(titles);
    let media_reason = format!("本轮另有需核准素材：{names}");
    let media_question = format!("同时请确认是否可以发送这些素材：{names}？");
    request.reason = Some(match request.reason.as_deref().map(str::trim) {
        Some(reason) if !reason.is_empty() => format!("{reason}；{media_reason}"),
        _ => media_reason,
    });
    request.question_for_principal = Some(
        match request.question_for_principal.as_deref().map(str::trim) {
            Some(question) if !question.is_empty() => format!("{question} {media_question}"),
            _ => media_question,
        },
    );
}

fn media_title_summary(titles: &[String]) -> String {
    titles
        .iter()
        .map(|title| title.trim())
        .filter(|title| !title.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("、")
}

async fn enqueue_holding_reply(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    run_id: String,
    source_event_id: String,
    decision_id: Option<mongodb::bson::oid::ObjectId>,
    content: String,
) -> AppResult<EnqueueOutcome> {
    Ok(outbox_enqueue(
        state,
        EnqueueRequest {
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
            contact_wxid: contact_wxid.to_string(),
            run_id,
            decision_id,
            source_event_id,
            source_kind: SOURCE_KIND_FOLLOW_UP_TASK.to_string(),
            content,
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await?)
}

/// Enqueue an expired-authorization holding reply under the same fenced task protocol as the
/// normal gateway. The review is the durable task→decision binding consumed by the dispatcher.
async fn enqueue_expired_relay_holding_reply(
    state: &AppState,
    task: &AgentTask,
    claim: &crate::tasks::TaskClaim,
    contact: &Contact,
    short_code: &str,
    content: String,
) -> AppResult<bool> {
    let review_id = mongodb::bson::oid::ObjectId::new();
    let run_id = format!("holding-expired-{}", claim.task_id.to_hex());
    state
        .db
        .decision_reviews()
        .insert_one(
            AgentDecisionReview {
                id: Some(review_id),
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: Some(contact.wxid.clone()),
                run_id: Some(run_id.clone()),
                inbound_message_id: None,
                reply_text: Some(content.clone()),
                approved: true,
                scores: Default::default(),
                formula_breakdown: Default::default(),
                risks: Vec::new(),
                rewrite_instruction: None,
                review_summary: Some(
                    "expired authorization neutral holding reply passed independent safety review"
                        .to_string(),
                ),
                playbook_id: None,
                playbook_version: None,
                used_knowledge_ids: Vec::new(),
                prompt_versions: Default::default(),
                operation_state: None,
                next_best_action: Default::default(),
                context_pack_snapshot: Default::default(),
                domain_config_snapshot: Default::default(),
                runtime_parameters_snapshot: Default::default(),
                send_gateway_result: doc! {
                    "allowed": true,
                    "status": "outbox_enqueuing",
                    "deliveryKind": "expired_principal_authorization_holding",
                },
                outcome_status: Some("pending".to_string()),
                reaction_analysis: Default::default(),
                reaction_claimed_at: None,
                reaction_claim_token: None,
                reaction_claim_generation: 0,
                source_task_id: None,
                source_task_claim_token: None,
                reviewer_misjudge_signal: None,
                expected_text_segments: 1,
                status: "outbox_enqueuing".to_string(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;

    if !crate::tasks::bind_task_decision_if_owned(state, claim, review_id).await? {
        state
            .db
            .decision_reviews()
            .update_one(
                doc! { "_id": review_id },
                doc! { "$set": { "status": "stale_task_claim" } },
                None,
            )
            .await?;
        return Ok(false);
    }

    let _ = enqueue_holding_reply(
        state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
        run_id,
        format!("principal-expired:{short_code}:{}", claim.claim_token),
        Some(review_id),
        content,
    )
    .await?;

    if !crate::tasks::authorize_task_outbox_if_owned(state, claim, review_id).await? {
        state
            .db
            .decision_reviews()
            .update_one(
                doc! { "_id": review_id, "status": "outbox_enqueuing" },
                doc! { "$set": { "status": "stale_task_claim" } },
                None,
            )
            .await?;
        return Ok(false);
    }
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": review_id, "status": "outbox_enqueuing" },
            doc! { "$set": { "status": "outbox_enqueued" } },
            None,
        )
        .await?;
    let _ = task;
    Ok(true)
}

/// 处理 principal_decision_relay task：领导已裁决，把决策用 AI 口吻转述给客户。
/// 生产任务入口必须携带当前 claim，以阻止 lease 重领后的旧 owner 继续提交。
pub(crate) async fn handle_principal_decision_relay_with_claim(
    state: &AppState,
    task: &AgentTask,
    task_claim: Option<&crate::tasks::TaskClaim>,
) -> AppResult<()> {
    if let Some(claim) = task_claim {
        if !crate::tasks::task_claim_is_current(state, claim).await? {
            return Ok(());
        }
    }
    let short_code = task.content.trim();
    let entry = state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": short_code }, None)
        .await?;
    let Some(entry) = entry else {
        return Ok(());
    };
    let Some(decision) = entry.decision.clone() else {
        return Ok(());
    };

    let now = mongodb::bson::DateTime::now();
    if relay_substance_if_usable(&decision, entry.authorization_expires_at, now).is_none() {
        // 授权过期：不拿过期授权乱承诺，但议题已被领导处理过——必须清 awaiting 标记
        // （否则下一轮 build_decision_signals_text 仍读到"等待裁决"，永久压制对该议题的自主回复）
        // + 发一条不含 substance 的中性收尾话术（否则客户零反馈、被晾死）。
        // 下一轮客户来消息由 AI 正常对话延续。fail-soft：发话术失败不 return Err（清标记已成功）。
        if !terminalize_principal_relay(state, &entry, "authorization_expired").await? {
            // Another terminal path (normally a confirmed relay delivery) already owns the
            // outcome. Do not enqueue a second, contradictory neutral close-out.
            return Ok(());
        }
        let contact = state
            .db
            .contacts()
            .find_one(
                doc! {
                    "workspace_id": &entry.workspace_id,
                    "account_id": &entry.account_id,
                    "wxid": &entry.contact_wxid
                },
                None,
            )
            .await?;
        if let Some(contact) = contact {
            // Expired authorization is not a fact source. The neutral holding reply is reviewed
            // independently and then handed to the durable outbox rather than sent by bare MCP.
            let holding_text = generate_holding_reply(
                state,
                &contact.workspace_id,
                &contact.account_id,
                &contact.wxid,
                HoldingReplyScene::ExpiredAuthorization,
                None,
            )
            .await;
            if let Some(claim) = task_claim {
                let _ = enqueue_expired_relay_holding_reply(
                    state,
                    task,
                    claim,
                    &contact,
                    short_code,
                    holding_text,
                )
                .await?;
            } else {
                // Compatibility for direct test/tool invocations that do not represent a claimed
                // AgentTask. Production worker/admin paths always carry a claim.
                let _ = enqueue_holding_reply(
                    state,
                    &contact.workspace_id,
                    &contact.account_id,
                    &contact.wxid,
                    format!(
                        "holding-expired-{}",
                        task.id
                            .map(|id| id.to_hex())
                            .unwrap_or_else(|| short_code.to_string())
                    ),
                    format!("principal-expired:{short_code}"),
                    None,
                    holding_text,
                )
                .await?;
            }
        }
        return Ok(());
    }

    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid
            },
            None,
        )
        .await?;
    let Some(contact) = contact else {
        return Ok(());
    };

    if let Some(claim) = task_claim {
        if !crate::tasks::task_claim_is_current(state, claim).await? {
            return Ok(());
        }
    }

    let task_context = task
        .id
        .map(|task_id| crate::tasks::TaskRunContext::new(task_id, task_claim));
    crate::agent::gateway::relay_principal_decision_to_customer(
        state,
        contact,
        &entry,
        &decision,
        task_context,
    )
    .await
}

/// 用 LLM 把真人自然语言回复解读成结构化裁决。绝不原话转发给客户。
/// 解析失败或 verdict 越界时回落 deferred（保守：宁可当"领导还没定"也不乱转述）。
pub(crate) async fn interpret_principal_reply(
    state: &AppState,
    account_id: &str,
    escalation: &AgentPrincipalEscalation,
    principal_reply_text: &str,
) -> AppResult<PrincipalDecision> {
    let user = format!(
        "客户请示问题：{}\n领导回复原话：{}",
        escalation.question_for_principal, principal_reply_text
    );
    let system = prompts::load_prompt(
        &state.db,
        &escalation.workspace_id,
        "escalation.principal.interpret",
    )
    .await?;
    let value = generate_agent_json(
        state,
        &escalation.workspace_id,
        Some(account_id),
        Some(&escalation.contact_wxid),
        None,
        "escalation.principal.interpret",
        &system,
        &user,
    )
    .await?;
    let decision: PrincipalDecision = match serde_json::from_value(value) {
        Ok(d) => d,
        Err(_) => {
            return Ok(PrincipalDecision {
                verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
                substance: String::new(),
                constraints: vec![],
                authorization_window_hours: None,
                exemption_type: crate::models::EXEMPTION_TYPE_NONE.to_string(),
            });
        }
    };
    Ok(sanitize_verdict(decision))
}

/// 处理真人（领导）的微信回复。匹配未决台账→解读→resolve→起 relay task。
/// 业务决策 #4：不带码且多条未决时反问澄清（向领导发一条，不回流客户）。
/// 返回 true 表示已作为领导回复消费（调用方据此不再进客户 agent 链路）。
pub(crate) async fn handle_principal_reply(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    principal_wxid: &str,
    reply_text: &str,
) -> AppResult<bool> {
    let pending =
        list_pending_for_principal(state, workspace_id, account_id, principal_wxid).await?;
    match match_principal_reply(reply_text, &pending) {
        ReplyMatch::NoPending => {
            tracing::info!(
                principal_wxid,
                "领导主动消息但无未决请示，不自动生效（待 admin 确认）"
            );
            Ok(true)
        }
        ReplyMatch::Ambiguous(codes) => {
            let list = codes
                .iter()
                .map(|c| format!("#{c}"))
                .collect::<Vec<_>>()
                .join(" / ");
            let ask = format!(
                "您刚回复的是哪一条？目前挂着这几条：{list}，麻烦带上编号（如 #{}）再回我一次。",
                codes.first().cloned().unwrap_or_default()
            );
            // Internal clarification is still a real outbound side effect. Route it through
            // the same durable Outbox/Dispatcher boundary as principal cards so retries,
            // cancellation, pacing, receipt classification, and delivery_unknown all apply.
            let mut stable_codes = codes.clone();
            stable_codes.sort();
            stable_codes.dedup();
            let source_event_id = format!(
                "principal-clarification:{}:{}",
                principal_wxid,
                stable_codes.join("-")
            );
            let outcome = outbox_enqueue(
                state,
                EnqueueRequest {
                    workspace_id: workspace_id.to_string(),
                    account_id: account_id.to_string(),
                    contact_wxid: principal_wxid.to_string(),
                    run_id: source_event_id.clone(),
                    decision_id: None,
                    source_event_id,
                    source_kind: SOURCE_KIND_PRINCIPAL_CLARIFICATION.to_string(),
                    content: ask,
                    media_asset_id: None,
                    referral_card_id: None,
                    max_attempts: 3,
                },
            )
            .await?;
            tracing::info!(
                principal_wxid,
                pending_codes = ?stable_codes,
                idempotent = matches!(outcome, EnqueueOutcome::IdempotentSkip { .. }),
                "principal ambiguity clarification accepted by durable outbox"
            );
            Ok(true)
        }
        ReplyMatch::Matched(short_code) => {
            let entry = pending
                .iter()
                .find(|e| e.short_code == short_code)
                .cloned()
                .expect("matched code must be in pending");
            let decision = interpret_principal_reply(state, account_id, &entry, reply_text).await?;
            if decision.verdict == crate::models::PRINCIPAL_VERDICT_DEFERRED {
                tracing::info!(short_code = %short_code, "领导暂缓，保持 pending 继续等待");
                return Ok(true);
            }
            // 授权过期时间：领导说了算。LLM 解读出领导明确说的时限→authorization_window_hours；
            // 领导没提→None=不设过期窗。不再硬编码默认窗。
            let expires = decision.authorization_window_hours.and_then(|hours| {
                if hours > 0.0 {
                    Some(DateTime::from_millis(
                        DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64,
                    ))
                } else {
                    None
                }
            });
            let resolved = resolve_escalation(state, &entry, &decision, expires, "wechat").await?;
            if resolved.is_none() {
                return Ok(true); // 已被并发 resolve；幂等。
            }
            Ok(true)
        }
    }
}

/// 超时转备选：扫描新协议 pending 请示并严格使用创建时冻结的 policy/account 快照。
/// 改派以 escalation 单文档 CAS 开启下一 delivery generation，再由 Outbox 幂等物化；
/// 这里不直接跨越 MCP 远端边界。
///
/// 两个入口共用同一套超时收敛语义（[`converge_timed_out_escalation`]），仅时间基准不同：
/// - 常规：当前卡已由 Outbox 确认送达（delivery_state=sent 且有推送时刻），
///   以 `last_pushed_at_ms` 计龄；
/// - 滞留：投递终失败 / 不可核验（failed_terminal / delivery_unknown），或 sent 但缺
///   推送时刻的异常形态——没有可信推送时刻，以 `created_at` 计龄。此前这些行进不了
///   常规扫描、会静默滞留（无改派也无安抚）。
pub async fn scan_escalation_timeouts(state: &AppState) -> AppResult<()> {
    let now_ms = DateTime::now().timestamp_millis();
    for entry in list_timeout_eligible_escalations(state).await? {
        let Some(last_pushed_at_ms) = entry.last_pushed_at_ms else {
            continue;
        };
        converge_timed_out_escalation(state, &entry, now_ms, last_pushed_at_ms).await?;
    }
    for entry in list_stranded_delivery_escalations(state).await? {
        let created_at_ms = entry.created_at.timestamp_millis();
        converge_timed_out_escalation(state, &entry, now_ms, created_at_ms).await?;
    }
    Ok(())
}

/// 单条 pending 请示的超时收敛：按冻结 policy 判定是否超时；超时且链上有下一位
/// 决策人 → 骚扰门放行后改派并重推卡（刷新投递代次）；超时且已到链尾 → 给客户
/// 发链尾安抚（按最小间隔去重）；未超时 → 不动。`age_base_ms` 是计龄基准
/// （常规卡=推送时刻，滞留卡=创建时刻），语义由调用方保证。
async fn converge_timed_out_escalation(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    now_ms: i64,
    age_base_ms: i64,
) -> AppResult<()> {
    let Some(protocol) = entry.protocol.as_ref() else {
        return Ok(());
    };
    let policy = resolve_ask_human_policy_snapshot(&protocol.policy);
    let age_hours = (now_ms - age_base_ms) as f64 / (3600.0 * 1000.0);
    let Some(next) = next_decider_on_timeout(
        &policy,
        &entry.principal_wxid,
        &entry.contact_wxid,
        age_hours,
    ) else {
        let timed_out = policy
            .timeout_hours
            .is_some_and(|timeout| age_hours >= timeout);
        if timed_out {
            // S5-5：链尾无人应答且台账年龄超过运营预设时限 → 执行预授权底线
            // （前置于安抚：底线一旦生效，客户等到的是可转述的方案而非又一条安抚）。
            if apply_standing_order_if_due(state, entry, &policy, now_ms).await? {
                return Ok(());
            }
            let min_interval_ms =
                (state.config.holding_reply_min_interval_hours * 3600.0 * 1000.0) as i64;
            let should_send = entry
                .last_holding_reply_ms
                .is_none_or(|last| now_ms - last >= min_interval_ms);
            if should_send {
                let holding_text = generate_holding_reply(
                    state,
                    &entry.workspace_id,
                    &entry.account_id,
                    &entry.contact_wxid,
                    HoldingReplyScene::ChainTail,
                    None,
                )
                .await;
                let window_ms = min_interval_ms.max(1);
                let window = now_ms.div_euclid(window_ms);
                match enqueue_holding_reply(
                    state,
                    &entry.workspace_id,
                    &entry.account_id,
                    &entry.contact_wxid,
                    format!("holding-chain-tail-{}-{window}", entry.short_code),
                    format!("principal-chain-tail:{}:{window}", entry.short_code),
                    None,
                    holding_text,
                )
                .await
                {
                    Ok(_) => {
                        if let Err(error) = touch_last_holding_reply_ms(
                            state,
                            &entry.workspace_id,
                            &entry.short_code,
                            now_ms,
                        )
                        .await
                        {
                            tracing::warn!(short_code = %entry.short_code, ?error, "链尾安抚已入队但更新时间失败");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(short_code = %entry.short_code, ?error, "chain-tail holding reply enqueue failed");
                    }
                }
            }
        }
        return Ok(());
    };
    let next_wxid = next.wxid.clone();
    let Some(next_account_id) = next
        .account_id
        .as_deref()
        .filter(|account_id| !account_id.trim().is_empty())
        .map(str::to_string)
    else {
        tracing::error!(short_code = %entry.short_code, next = %next_wxid, "冻结决策人缺少发送账号，拒绝改派");
        return Ok(());
    };
    let since_ms = now_ms - 24 * 3600 * 1000;
    let today = count_pushes_today(state, &entry.workspace_id, &next_wxid, since_ms).await?;
    let last_push = latest_push_ms(state, &entry.workspace_id, &next_wxid).await?;
    if !push_allowed(&policy, today, last_push, now_ms) {
        return Ok(());
    }
    if let Some(next_entry) = reassign_escalation(
        state,
        &entry.workspace_id,
        &entry.short_code,
        &entry.principal_wxid,
        protocol.delivery_generation,
        &next_wxid,
        &next_account_id,
    )
    .await?
    {
        // 入队确认失败时保留 pending_enqueue，下一 worker tick 会按 generation 幂等补偿。
        materialize_principal_card_delivery(state, &next_entry).await?;
    }
    Ok(())
}

/// S5-5 请示预授权底线：链尾无人应答超过 `standing_order_after_hours`（以台账
/// created_at 计龄）后，把运营预写的 `standing_order` 口径当作一条与领导裁决同形的
/// conditional 预授权执行——AI 只是执行方，裁决实质是人类提前写好的授权。
/// 复用 [`resolve_escalation`]→[`materialize_relay_task`] 既有链路（零新发送路径），
/// 客户随后收到的是 relay task 经网关以 AI 口吻转述的底线方案。
///
/// 返回 `true` = 本台账已被 resolve（或已被并发路径 resolve），调用方跳过链尾安抚；
/// `false` = 底线未配置/时限未到，调用方维持既有安抚行为。
///
/// 幂等：resolve 内核 CAS 只吃 pending 台账（resolved 终态排除在扫描 filter 外），
/// 同一台账至多应用一次；并发被领导/admin 抢先 resolve 时静默让路、不写事件。
async fn apply_standing_order_if_due(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    policy: &ResolvedAskHumanPolicy,
    now_ms: i64,
) -> AppResult<bool> {
    let created_at_ms = entry.created_at.timestamp_millis();
    let Some(text) = standing_order_due(policy, created_at_ms, now_ms) else {
        return Ok(false);
    };
    let decision = PrincipalDecision {
        verdict: PRINCIPAL_VERDICT_CONDITIONAL.to_string(),
        substance: text.to_string(),
        constraints: vec![],
        // 运营常备底线不设授权过期窗；口径变更走配置编辑，只影响后续新触发。
        authorization_window_hours: None,
        exemption_type: EXEMPTION_TYPE_NONE.to_string(),
    };
    let resolved =
        resolve_escalation(state, entry, &decision, None, "standing_order_policy").await?;
    if resolved.is_none() {
        // 并发已被领导回复 / admin 裁决抢先 resolve → 幂等让路（议题已有真人裁决，
        // 本轮既不再应用底线也不再发链尾安抚）。
        return Ok(true);
    }
    let elapsed_hours = (now_ms - created_at_ms) as f64 / (3600.0 * 1000.0);
    // fail-soft 审计：事件写失败不回滚 resolve（relay intent 已持久化）。
    if let Err(error) = crate::agent::gateway::write_event_for_account(
        state,
        &entry.workspace_id,
        &entry.account_id,
        Some(&entry.contact_wxid),
        "escalation_standing_order_applied",
        "ok",
        "链尾决策人持续未应答，按运营预授权底线口径出具转述方案",
        Some(doc! {
            "short_code": &entry.short_code,
            "elapsed_hours": elapsed_hours,
            "standing_order_after_hours": policy.standing_order_after_hours,
        }),
    )
    .await
    {
        tracing::warn!(short_code = %entry.short_code, ?error, "standing-order 审计事件写入失败");
    }
    Ok(true)
}
