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

use super::generate_agent_json;
use super::outbox::{enqueue as outbox_enqueue, EnqueueOutcome, EnqueueRequest};
use super::run_envelope::SOURCE_KIND_FOLLOW_UP_TASK;
use super::types::{AgentDecision, DecisionReviewResult};
use crate::error::{AppError, AppResult};
use crate::mcp;
use crate::models::{
    AgentDecisionReview, AgentPrincipalEscalation, AgentTask, Contact, OperationDomainConfig,
    PrincipalDecision, AWAITING_PRINCIPAL_DECISION_ATTR, ESCALATION_CATEGORY_HIGH_RISK_GATED,
    PRINCIPAL_VERDICT_DEFERRED,
};
use crate::prompts;
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};

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

/// hold→升级请示：被风险闸门拦下的高风险件，按 workspace 升级模式请示领导。
///
/// 与 `trigger_principal_escalation` 的区别：后者用于 approved 路径（占位已由 outbox 发出）；
/// 本函数用于 hold 路径，只推领导卡 + 落 pending 台账 + 写 awaiting 标记，**不向客户发任何消息**。
/// 客户侧的安抚占位由网关守卫 `ensure_customer_acknowledged` 统一负责（解耦"安抚客户"与
/// "请示领导"：前者对任何 Inbound 零回复无条件补，后者受领导骚扰门 / 去重约束）。
///
/// 调用方对本函数错误只记 warn、不阻断 run、不改终态。
pub(crate) async fn escalate_held_decision(
    state: &AppState,
    contact: &Contact,
    review: &DecisionReviewResult,
    final_decision: &AgentDecision,
    domain_config: Option<&OperationDomainConfig>,
    blocked_status: &str,
) -> AppResult<()> {
    let policy = match domain_config {
        Some(cfg) => crate::agent::escalation::resolve_ask_human_policy(cfg),
        // domain_config 缺省时保持旧行为字节等价：parse_high_risk_mode(None)=DecisionOnly,
        // 即 safety/product/stuck 升级、ai_policy 不升级。真正的「是否启用请示」由下方
        // decider_chain 是否为空兜住(链空则 return Ok)。故此处不可短路 return。
        None => crate::agent::escalation::ResolvedAskHumanPolicy {
            decider_chain: vec![],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: None,
        },
    };
    if !should_escalate_held(blocked_status, &policy) {
        return Ok(());
    }
    let Some(decider) = policy.decider_chain.first() else {
        return Ok(()); // 决策人链空 = 本 workspace 未启用请示通道
    };
    let principal_wxid = decider.wxid.clone();
    if principal_wxid == contact.wxid {
        return Err(AppError::BadRequest(
            "决策人配置等于客户 wxid，拒绝触发请示".into(),
        ));
    }
    // 骚扰门：daily_push_cap / quiet_hours（None 配置全放行，字节等价）。
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let since_ms = now_ms - 24 * 3600 * 1000;
    let today = count_pushes_today(state, &contact.workspace_id, &principal_wxid, since_ms).await?;
    let last_push = latest_push_ms(state, &contact.workspace_id, &principal_wxid).await?;
    if !crate::agent::escalation::push_allowed(&policy, today, last_push, now_ms) {
        return Ok(()); // 骚扰门关：跳过推卡（pending 台账可由 admin 在收件箱处置）
    }
    // 去重：同客户同类别已有 pending → 不重复推卡骚扰领导。
    if has_pending_for_contact(
        state,
        &contact.workspace_id,
        &contact.wxid,
        ESCALATION_CATEGORY_HIGH_RISK_GATED,
    )
    .await?
    {
        return Ok(());
    }
    let reason = if !review.hold_reason.trim().is_empty() {
        review.hold_reason.clone()
    } else {
        review.review_summary.clone()
    };
    let question = format!(
        "该客户议题触发高风险闸门（{}），AI 暂不自行答复。拟答风险等级：{}。请领导定夺该如何回复。",
        labels::blocked_status_zh(blocked_status),
        labels::risk_level_zh(&final_decision.risk_level),
    );
    let Some(entry) = insert_pending_escalation(
        state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
        ESCALATION_CATEGORY_HIGH_RISK_GATED,
        &reason,
        &question,
        &principal_wxid,
        false, // 高风险硬闸件默认不泛化（领导裁决可能是个案）
    )
    .await?
    else {
        // 并发已插入同客户同类别 pending（pending 去重索引兜住）→ 不重复推卡。
        return Ok(());
    };
    let customer_label = contact
        .remark
        .clone()
        .or_else(|| contact.nickname.clone())
        .or_else(|| contact.alias.clone())
        .unwrap_or_else(|| contact.wxid.clone());
    let card = render_principal_card(&entry.short_code, &customer_label, &reason, &question);
    mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_text",
        serde_json::json!({ "recipient": principal_wxid, "content": card }),
    )
    .await?;
    // 写 awaiting 标记（hold 路径不走 apply_agent_updates，需单独写），
    // 否则下一轮 build_decision_signals_text 读不到等待信号。用 dotted key $set，不覆盖其它 domain_attributes。
    let set_key = format!("domain_attributes.{}", AWAITING_PRINCIPAL_DECISION_ATTR);
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "wxid": &contact.wxid,
            },
            doc! { "$set": { set_key: true, "domain_attributes_updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(())
}

/// 处理 principal_decision_relay task：领导已裁决，把决策用 AI 口吻转述给客户。
pub(crate) async fn handle_principal_decision_relay(
    state: &AppState,
    task: &AgentTask,
) -> AppResult<()> {
    handle_principal_decision_relay_with_claim(state, task, None).await
}

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
        &state.config.default_workspace_id,
        "escalation.principal.interpret",
    )
    .await?;
    let value = generate_agent_json(
        state,
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
    let pending = list_pending_for_principal(state, workspace_id, principal_wxid).await?;
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
            mcp::logged_call_for_account(
                state,
                account_id,
                "message_send_text",
                serde_json::json!({ "recipient": principal_wxid, "content": ask }),
            )
            .await?;
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
            let resolved =
                resolve_escalation(state, &short_code, &decision, expires, "wechat").await?;
            if resolved.is_none() {
                return Ok(true); // 已被并发 resolve；幂等。
            }
            enqueue_relay_task(state, &entry).await?;
            Ok(true)
        }
    }
}

/// 超时转备选：扫所有 pending 请示，age > timeout_hours 且当前决策人非链尾 → 改派下一位 + 重推卡。
/// AI 绝不替决策人拍板——只把请示转给链上下一位真人。timeout=None → 无限等待，不动。
/// 顺序：gate(next) → 推卡 MCP → 推成功才 reassign（落库同时刷新 updated_at，age 自此起算）。
/// gate 拦或推失败都【不改派】——原 principal 不变、age 仍超时，下一 tick 重新算出同一个
/// next 再试，绝不把台账困在链尾（改派只发生在卡确实送达 next 之后）。
pub async fn scan_escalation_timeouts(state: &AppState) -> AppResult<()> {
    use futures::TryStreamExt;
    let now_ms = DateTime::now().timestamp_millis();
    // 取所有 current_version config，建 workspace+domain → resolved policy 映射。
    let configs: Vec<OperationDomainConfig> = state
        .db
        .operation_domain_configs()
        .find(doc! { "current_version": true }, None)
        .await?
        .try_collect()
        .await?;
    for cfg in &configs {
        let policy = resolve_ask_human_policy(cfg);
        if policy.timeout_hours.is_none() {
            continue;
        }
        let pending = list_escalations_by_workspace(state, &cfg.workspace_id, "pending").await?;
        for entry in pending {
            let age_hours =
                (now_ms - entry.updated_at.timestamp_millis()) as f64 / (3600.0 * 1000.0);
            let Some(next) = next_decider_on_timeout(
                &policy,
                &entry.principal_wxid,
                &entry.contact_wxid,
                age_hours,
            ) else {
                // next_decider_on_timeout 返回 None 有两种情形：①尚未超时 ②已超时但到链尾。
                // 仅情形②需安抚客户。policy.timeout_hours 在 :365 已确保 Some，可直接比对区分。
                let timed_out = policy.timeout_hours.map_or(false, |t| age_hours >= t);
                if timed_out {
                    // 链尾：无更多决策人可改派。客户不能被永久晾着——发 AI 自主延期安抚话术，
                    // 台账保持 pending 继续等领导。去重：每 holding_reply_min_interval_hours 最多一条。
                    // 安抚发给**客户**（非领导推卡），故**不过** push_allowed（quiet_hours 约束打扰领导；
                    // 客户安抚只受 min_interval 去重约束）。
                    let min_interval_ms =
                        (state.config.holding_reply_min_interval_hours * 3600.0 * 1000.0) as i64;
                    let should_send = entry
                        .last_holding_reply_ms
                        .map_or(true, |last| now_ms - last >= min_interval_ms);
                    if should_send {
                        // Chain-tail holding reply: semantic review first, durable outbox second.
                        let holding_text = generate_holding_reply(
                            state,
                            &entry.account_id,
                            &entry.contact_wxid,
                            HoldingReplyScene::ChainTail,
                            None,
                        )
                        .await;
                        let window_ms = min_interval_ms.max(1);
                        let window = now_ms.div_euclid(window_ms);
                        let enqueue_result = enqueue_holding_reply(
                            state,
                            &cfg.workspace_id,
                            &entry.account_id,
                            &entry.contact_wxid,
                            format!("holding-chain-tail-{}-{window}", entry.short_code),
                            format!("principal-chain-tail:{}:{window}", entry.short_code),
                            None,
                            holding_text,
                        )
                        .await;
                        match enqueue_result {
                            Ok(_) => {
                                if let Err(e) = touch_last_holding_reply_ms(
                                    state,
                                    &cfg.workspace_id,
                                    &entry.short_code,
                                    now_ms,
                                )
                                .await
                                {
                                    tracing::warn!(short_code = %entry.short_code, error = ?e, "链尾安抚已入队但更新 last_holding_reply_ms 失败");
                                }
                            }
                            Err(error) => {
                                tracing::warn!(short_code = %entry.short_code, ?error, "chain-tail holding reply enqueue failed; retry remains eligible");
                            }
                        }
                    }
                }
                continue;
            };
            let next_wxid = next.wxid.clone();

            // 骚扰门先于改派（关键）：此刻台账仍挂【原】principal，查 next 的 count/latest 不含本条，
            // 无自我命中。命中则本 tick 不改派、不推——原 principal age 仍超时，下一 tick 会重新
            // 算出同一个 next 再试（绝不把台账困在链尾：改派只发生在卡确实送达 next 之后）。
            let since_ms = now_ms - 24 * 3600 * 1000;
            let today = count_pushes_today(state, &cfg.workspace_id, &next_wxid, since_ms).await?;
            let last_push = latest_push_ms(state, &cfg.workspace_id, &next_wxid).await?;
            if !push_allowed(&policy, today, last_push, now_ms) {
                continue; // 骚扰门拦：本 tick 跳过（不改派），待下一 tick
            }

            // 先推卡给 next，推成功才改派落库（reassign 落库同时刷新 updated_at）。
            let label = entry.contact_wxid.clone();
            let card = render_principal_card(
                &entry.short_code,
                &label,
                &entry.reason,
                &entry.question_for_principal,
            );
            match mcp::logged_call_for_account(
                state,
                &entry.account_id,
                "message_send_text",
                serde_json::json!({ "recipient": &next_wxid, "content": card }),
            )
            .await
            {
                Ok(_) => {
                    // 推达 next 才改派：principal_wxid → next，updated_at 刷新（age 自此起算）。
                    // reassign 落库失败时 next 已收到卡（体验无损），下一 tick principal 仍是原值会
                    // 重推同一个 next（幂等，可接受），不丢不卡死。
                    if let Err(e) =
                        reassign_escalation(state, &cfg.workspace_id, &entry.short_code, &next_wxid)
                            .await
                    {
                        tracing::warn!(short_code = %entry.short_code, error = ?e, "改派推卡成功但落库改派失败，下一 tick 将重推");
                    }
                }
                Err(e) => {
                    // 推卡失败：不改派，原 principal/age 不变，下一 tick 重推给同一个 next。
                    tracing::warn!(short_code = %entry.short_code, next = %next_wxid, error = ?e, "改派推卡失败，下一 tick 将重试");
                }
            }
        }
    }
    Ok(())
}
