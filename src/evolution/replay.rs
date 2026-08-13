//! Shadow replay 调度（M4 W3 Task 4.1 / 4.3）。
//!
//! 对每条候选 × 每条 cohort run 执行一次"短路 gateway"重放：
//!
//! - **Threshold 候选**：纯重判。读源 run 的 `review.scores.{factRisk, pressureRisk,
//!   humanLike, emotionalValue, productAccuracy}` 与候选阈值对比，给出 new_5gate_hit
//!   与 new_final_review_status（不调 LLM、不写 outbox / mcp / conversation_messages
//!   outbound / agent_run_logs）。
//! - **Prompt 候选**：调 `crate::agent::prompt_shadow::shadow_replay_prompt_one`
//!   用「原 prompt + critic 追加片段」对单条源样本跑一次真实的 Reply + Review
//!   链路（纯演练，永不触达发送链 / outbox / MCP），把新旧两侧的 review.scores
//!   推回 5 闸命中向量 + selfCritique addressed 落进 `shadow_replays`（含 G4
//!   真实 original 侧字段），供显著性测试与管理员 release 对照。
//!
//! 严格隔离：
//! - **不**调 `agent::run_user_operation_gateway` / `handle_managed_message` /
//!   `handle_follow_up_task`；
//! - **不**调 `agent::outbox` 任何 enqueue；
//! - **不**调 `mcp::*`；
//! - **不**写 `conversation_messages` 出站文档；
//! - **不**写 `agent_run_logs`（避免污染 outcomes）。
//!
//! `evolution::lint::passes_forbidden_words` 在 critic 路径已守过，本模块不再
//! 二次扫描；本模块自身的代码 / 字符串都在 CI lint 静态扫描范围内。
//!
//! 并行：使用 `tokio::sync::Semaphore::new(evolution_replay_concurrency)` 限并发。
//! 预算超额时仍未启动的 replay 写 `failed` + `failure_reason="evolution_budget_exceeded"`。

use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};
use crate::models::{AgentRunLog, Proposal, ShadowReplay};
use crate::routes::AppState;

use super::budget::EvolutionBudget;
use super::error::EvolutionError;

/// Block-class gates with `score >= threshold` 触发（分数越高越危险）。
const BLOCK_DIRECTION_GTE: &[&str] = &["fact_risk_block", "pressure_risk_block"];

/// Block / rewrite 类用 `score < threshold` 触发（分数越低越触发）。
/// 注意 `product_accuracy_score_block` 走 < 方向：业务上"产品准确度过低 →
/// 拦截送出"，与 `review_passed` 的 `>= product_accuracy_block_below` 对偶。
const REWRITE_DIRECTION_LT: &[&str] = &[
    "human_like_score_rewrite",
    "emotional_value_rewrite",
    "product_accuracy_score_block",
];

/// 对单条 proposal 的全部 cohort runs 起 replay。最多同时执行
/// `state.config.evolution_replay_concurrency` 条。budget 触顶时未启动的 replay
/// 写 `failed` + `failure_reason="evolution_budget_exceeded"`。
pub async fn eval_all(
    state: &AppState,
    experiment_id: &str,
    workspace_id: &str,
    account_id: &str,
    budget: &mut EvolutionBudget,
) -> Result<(), EvolutionError> {
    // 1. 加载本 experiment 下 status=pending_eval 的所有 proposals。
    let proposals: Vec<Proposal> = state
        .db
        .proposals()
        .find(
            doc! {
                "experiment_id": experiment_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
                "status": "pending_eval",
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .try_collect()
        .await
        .map_err(EvolutionError::from)?;

    if proposals.is_empty() {
        return Ok(());
    }

    // 2. 加载 experiment envelope（拿 cohort_run_ids）。
    let envelope_doc = state
        .db
        .experiments()
        .find_one(
            doc! {
                "experiment_id": experiment_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!("experiment_id not found: {experiment_id}"))
        })?;
    let threshold_runs = envelope_doc.cohort_threshold_run_ids.clone();
    let prompt_runs = envelope_doc.cohort_prompt_run_ids.clone();

    // 3. 并发限流 + 调度。每条 (proposal, source_run_id) 起一个 task。
    //    EvolutionBudget 是 mut 借用，不能跨 task；budget 余量不足 → 直接写
    //    failed shadow_replay 不起 task。
    let permits = state.config.evolution_replay_concurrency.max(1);
    let semaphore = Arc::new(Semaphore::new(permits));
    let mut handles = Vec::new();

    for proposal in proposals {
        let pid = match proposal.id {
            Some(id) => id,
            None => continue,
        };
        let source_runs: Vec<ObjectId> = match proposal.proposal_kind.as_str() {
            "threshold" => threshold_runs.clone(),
            "prompt" => prompt_runs.clone(),
            _ => continue,
        };
        for src in source_runs {
            // budget 静态预检：threshold 重放不调 LLM；prompt 重放在 task 内
            // 调真实 LLM（shadow_replay_prompt_one），但 EvolutionBudget 是 mut
            // 借用、无法跨 task 计量，影子消耗不回写本预算——exhausted() 预检
            // 是唯一控制点，超额时后续 replay 直接写 failed 不启动。
            if budget.exhausted() {
                let _ =
                    insert_replay_failed(state, &proposal, pid, src, "evolution_budget_exceeded")
                        .await;
                continue;
            }

            let state_cloned = state.clone();
            let proposal_cloned = proposal.clone();
            let sem_cloned = semaphore.clone();
            let handle = tokio::spawn(async move {
                let _permit = sem_cloned.acquire_owned().await.ok();
                let _ = run_shadow_replay(&state_cloned, &proposal_cloned, src).await;
            });
            handles.push(handle);
        }
    }

    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

/// 单条 replay：读源 run + inbound message → 短路评估 → 写 shadow_replays。
pub async fn run_shadow_replay(
    state: &AppState,
    proposal: &Proposal,
    source_run_id: ObjectId,
) -> AppResult<()> {
    let started_at = DateTime::now();

    // 1. 反查源 run。
    let original = match state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "_id": source_run_id,
                "workspace_id": &proposal.workspace_id,
                "account_id": &proposal.account_id,
            },
            None,
        )
        .await
        .map_err(AppError::from)?
    {
        Some(o) => o,
        None => {
            return persist_replay(
                state,
                proposal,
                source_run_id,
                started_at,
                ReplayOutcome::failed("source_run_not_found"),
            )
            .await;
        }
    };

    // 2. inbound message 必须仍在（retention 未清理）。源 inbound id 落在
    //    AgentRunLog 顶层 `source_event_id`（= envelope 的 message.message_id，见
    //    gateway `trigger_envelope_source`）——gateway 从不往 `context` 写
    //    inboundMessageId。retention 探针**只对 prompt 候选**做：threshold 重判
    //    纯读 review.scores、不需要原文，探针会把 retention 已清理的源 run 错杀；
    //    prompt 重放才需要真实历史消息。follow_up / 合成兜底（`synthetic:` 前缀）
    //    的 source_event_id 查 messages 必 miss——也按 source_message_unavailable 短路。
    if proposal.proposal_kind == "prompt" {
        let inbound_id = original.source_event_id.trim();
        let contact_wxid = original.contact_wxid.as_deref().unwrap_or("").trim();
        let probe_ok = original.source_kind
            == crate::agent::run_envelope::SOURCE_KIND_INBOUND_MESSAGE
            && !inbound_id.is_empty()
            && !inbound_id.starts_with("synthetic:")
            && !contact_wxid.is_empty();
        let count = if probe_ok {
            // ConversationMessage 无 rename_all → BSON 字段是 snake_case `message_id`
            // （见 models.rs ConversationMessage、db/indexes.rs:49 的索引、webhooks.rs
            // 写入路径、prompt_shadow.rs 的同款 find_one）。此处必须用 `message_id`，
            // 否则 retention 探针对任何真实消息都 count==0 → 误判 source_message_unavailable，
            // 把所有 prompt shadow happy path 错杀成 failed。
            state
                .db
                .messages()
                .count_documents(
                    prompt_shadow_retention_message_filter(
                        &proposal.workspace_id,
                        &proposal.account_id,
                        contact_wxid,
                        inbound_id,
                    ),
                    None,
                )
                .await
                .map_err(AppError::from)?
        } else {
            0
        };
        if count == 0 {
            return persist_replay(
                state,
                proposal,
                source_run_id,
                started_at,
                ReplayOutcome::failed("source_message_unavailable"),
            )
            .await;
        }
    }

    // 3. 按 proposal kind 分派。
    let outcome = match proposal.proposal_kind.as_str() {
        "threshold" => evaluate_threshold(proposal, &original),
        // prompt 候选走真模型对照（async）。短路 gateway / outbox / MCP——
        // shadow_replay_prompt_one 只跑 decide_reply + review_decision 演练。
        "prompt" => {
            match crate::agent::prompt_shadow::shadow_replay_prompt_one(
                state,
                proposal,
                source_run_id,
            )
            .await
            {
                Ok(sample) => prompt_sample_to_outcome(sample),
                // DB / LLM 故障 → 记 failed，不向上抛（避免单条 replay 拖垮整批）。
                Err(e) => ReplayOutcome::failed_with(format!("prompt_shadow_error:{e}")),
            }
        }
        other => ReplayOutcome::failed_with(format!("unknown_proposal_kind:{other}")),
    };

    persist_replay(state, proposal, source_run_id, started_at, outcome).await
}

fn prompt_shadow_retention_message_filter(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    message_id: &str,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_wxid": contact_wxid,
        "message_id": message_id,
    }
}

/// Threshold 重判：纯函数，输入候选 + 原 run 的 review.scores，返回 5 闸新命中向量。
fn evaluate_threshold(proposal: &Proposal, original: &AgentRunLog) -> ReplayOutcome {
    let gate_key = match proposal.gate_key.as_deref() {
        Some(g) => g,
        None => return ReplayOutcome::failed("threshold_proposal_missing_gate_key"),
    };
    let new_value = match proposal.proposed_value {
        Some(v) => v,
        None => return ReplayOutcome::failed("threshold_proposal_missing_proposed_value"),
    };
    let scores = match original.review.get_document("scores").ok() {
        Some(s) => s.clone(),
        None => {
            return ReplayOutcome::failed("source_run_missing_review_scores");
        }
    };

    // 同时产出 original / new 两个 5 闸命中向量:被改的 gate 两侧只差阈值
    // (original 用 current_value、new 用 proposed_value),其余 4 个 gate 本
    // proposal 不动 → 两侧都用 default 阈值、delta 恒 0。original_5gate_hit
    // 供 significance compute_5gate_deltas 算真实基线(此前恒空 → 偏拒,M9)。
    let mut original_5gate_hit = Document::new();
    let mut new_5gate_hit = Document::new();
    for gate in [
        "fact_risk_block",
        "pressure_risk_block",
        "human_like_score_rewrite",
        "emotional_value_rewrite",
        "product_accuracy_score_block",
    ] {
        if gate == gate_key {
            let current = proposal
                .current_value
                .or_else(|| default_gate_threshold(gate))
                .unwrap_or(0.0);
            original_5gate_hit.insert(gate, evaluate_single_gate(&scores, gate, current));
            new_5gate_hit.insert(gate, evaluate_single_gate(&scores, gate, new_value));
        } else {
            let hit = evaluate_single_gate_default(&scores, gate);
            original_5gate_hit.insert(gate, hit);
            new_5gate_hit.insert(gate, hit);
        }
    }

    // 如果"被改的"那个 gate 仍然命中（block / rewrite 触发），final_review_status
    // 沿用源 run（多半是 blocked_*）；如果 new gate 未命中且其它 gate 也未命中，
    // 可标 approved；否则保留源 run 的 final 状态作为"无显著变化"信号。
    let new_final = final_status_from_5gate(&new_5gate_hit);

    ReplayOutcome {
        completed: true,
        failure_reason: None,
        // KE-02：original 终态用 5闸重推(基于已在上方算好的 original_5gate_hit)，
        // 与 prompt 路径 prompt_sample_to_outcome 及 new 侧同口径。旧代码用源 run 真实
        // 终态 original.final_review_status，若终态是非-5gate 因素(blocked_by_budget/
        // ai_waiting_for_more_context 等)会让 original 侧算"发送失败"、new 侧 5闸算"成功"，
        // 凭空 +send_delta 虚假翻越 min_send_success_delta 门。两侧同口径后唯一变量是被改 gate。
        original_final_review_status: Some(
            final_status_from_5gate(&original_5gate_hit).to_string(),
        ),
        original_5gate_hit,
        original_self_critique_addressed: None,
        new_final_review_status: Some(new_final.to_string()),
        new_review_risks: Vec::new(),
        new_token_cost: Some(0),
        new_self_critique_addressed: Some(matches!(
            new_final,
            "approved" | "revision_applied_approved"
        )),
        new_5gate_hit,
    }
}

/// 单 gate 命中判断。`scores` 是 `agent_run_logs.review.scores` Document
/// （camelCase）。block 类用 `>=`；rewrite 类用 `<`。
fn evaluate_single_gate(scores: &Document, gate: &str, threshold: f64) -> bool {
    // 复用双键兼容的 read_gate_score(factRisk/hallucinationScore 等两套键名都读);
    // 缺分 → 0.0,与 prompt 路径 scores_to_5gate_hit 的保守处理一致。
    let score = read_gate_score(scores, gate).unwrap_or(0.0);
    if BLOCK_DIRECTION_GTE.contains(&gate) {
        score >= threshold
    } else if REWRITE_DIRECTION_LT.contains(&gate) {
        score < threshold
    } else {
        false
    }
}

/// 默认其它 4 个 gate 的命中判断（不带 proposed_value 时）：当前 review.scores
/// 已经过 gateway 写入，用业务"惯用阈值"硬常量推断（fact ≥ 6 / pressure ≥ 7 /
/// product < 7 / human < 6 / emotional < 6）。prompt shadow 重放刻意不引入
/// per-contact `resolve_thresholds`——新旧两侧固定同一组默认阈值，唯一变量是
/// prompt 片段，才能把 5 闸命中差异干净归因到 prompt 改动本身。
fn evaluate_single_gate_default(scores: &Document, gate: &str) -> bool {
    let default_threshold = match default_gate_threshold(gate) {
        Some(t) => t,
        None => return false,
    };
    evaluate_single_gate(scores, gate, default_threshold)
}

/// 5 闸的业务"惯用阈值"硬常量（与生产 gateway 默认一致）。prompt shadow 重放
/// 不改阈值——新旧两侧都用同一组默认阈值把 review.scores 推回 5 闸命中口径，
/// 唯一变量是 prompt 片段。
fn default_gate_threshold(gate: &str) -> Option<f64> {
    match gate {
        "fact_risk_block" => Some(6.0),
        "pressure_risk_block" => Some(7.0),
        "human_like_score_rewrite" => Some(6.0),
        "emotional_value_rewrite" => Some(6.0),
        "product_accuracy_score_block" => Some(7.0),
        _ => None,
    }
}

/// 从 review.scores Document 读某 5 闸对应分数；兼容两套字段命名——reviewer
/// 历史以 `factRisk`/`productAccuracy` 命名，`ReviewScores` 序列化形态是
/// `hallucinationScore`/`knowledgeGroundingScore`（仅反序列化带 alias）。任一
/// 命中即取，i32 / f64 落库都接。
fn read_gate_score(scores: &Document, gate: &str) -> Option<f64> {
    let candidates: &[&str] = match gate {
        "fact_risk_block" => &["factRisk", "hallucinationScore"],
        "pressure_risk_block" => &["pressureRisk"],
        "human_like_score_rewrite" => &["humanLike"],
        "emotional_value_rewrite" => &["emotionalValue"],
        "product_accuracy_score_block" => &["productAccuracy", "knowledgeGroundingScore"],
        _ => return None,
    };
    for k in candidates {
        if let Ok(v) = scores.get_i32(k) {
            return Some(v as f64);
        }
        if let Ok(v) = scores.get_f64(k) {
            return Some(v);
        }
    }
    None
}

/// 把 review.scores 推回 5 闸命中布尔向量（用默认阈值）。prompt shadow 用它
/// 把新旧两侧 scores 统一映射成与 threshold 路径同形态的 `*_5gate_hit` Document。
/// 缺字段按 0.0 计（保守：block 类不命中、rewrite 类命中——与生产对缺分的保守
/// 处理一致）。
fn scores_to_5gate_hit(scores: &Document) -> Document {
    let mut hit = Document::new();
    for gate in [
        "fact_risk_block",
        "pressure_risk_block",
        "human_like_score_rewrite",
        "emotional_value_rewrite",
        "product_accuracy_score_block",
    ] {
        let score = read_gate_score(scores, gate).unwrap_or(0.0);
        let threshold = default_gate_threshold(gate).unwrap_or(0.0);
        let h = if BLOCK_DIRECTION_GTE.contains(&gate) {
            score >= threshold
        } else if REWRITE_DIRECTION_LT.contains(&gate) {
            score < threshold
        } else {
            false
        };
        hit.insert(gate, h);
    }
    hit
}

/// 从 5 闸命中向量推 final_review_status：block 类（fact > pressure > product
/// 取最严）→ 拦截态；否则 rewrite 类命中 → `revision_applied_approved`；全不命中
/// → `approved`。threshold / prompt 两条 shadow 路径共用同一口径。
fn final_status_from_5gate(hit: &Document) -> &'static str {
    let any_block_hit = hit.get_bool("fact_risk_block").unwrap_or(false)
        || hit.get_bool("pressure_risk_block").unwrap_or(false)
        || hit
            .get_bool("product_accuracy_score_block")
            .unwrap_or(false);
    let any_rewrite_hit = hit.get_bool("human_like_score_rewrite").unwrap_or(false)
        || hit.get_bool("emotional_value_rewrite").unwrap_or(false);
    if any_block_hit {
        if hit.get_bool("fact_risk_block").unwrap_or(false) {
            "held_by_ai_policy"
        } else if hit.get_bool("pressure_risk_block").unwrap_or(false) {
            "blocked_by_safety_guard"
        } else {
            "blocked_unverified_product_claim"
        }
    } else if any_rewrite_hit {
        "revision_applied_approved"
    } else {
        "approved"
    }
}

/// 把 `PromptShadowSample` 映射成 `ReplayOutcome`。新旧两侧 review.scores 各自
/// 推回 5 闸命中向量（同一组默认阈值，唯一变量是 prompt 片段），再推 final
/// 状态；selfCritique addressed 两侧直接透传。`status="failed"` 的 sample →
/// `ReplayOutcome::failed*`（completed=false，进 significance 的 failed 分母）。
fn prompt_sample_to_outcome(
    sample: crate::agent::prompt_shadow::PromptShadowSample,
) -> ReplayOutcome {
    if sample.status != "completed" {
        return ReplayOutcome::failed_with(
            sample
                .failure_reason
                .unwrap_or_else(|| "prompt_shadow_failed".to_string()),
        );
    }
    let original_5gate_hit = sample
        .original_scores
        .as_ref()
        .map(scores_to_5gate_hit)
        .unwrap_or_default();
    let new_5gate_hit = sample
        .new_scores
        .as_ref()
        .map(scores_to_5gate_hit)
        .unwrap_or_default();
    let original_final = sample.original_final_review_status.or_else(|| {
        sample
            .original_scores
            .as_ref()
            .map(|_| final_status_from_5gate(&original_5gate_hit).to_string())
    });
    let new_final = sample
        .new_final_review_status
        .unwrap_or_else(|| final_status_from_5gate(&new_5gate_hit).to_string());

    ReplayOutcome {
        completed: true,
        failure_reason: None,
        original_final_review_status: original_final,
        original_5gate_hit,
        original_self_critique_addressed: sample.original_self_critique_addressed,
        new_final_review_status: Some(new_final),
        new_review_risks: sample.new_review_risks,
        new_token_cost: None,
        new_self_critique_addressed: sample.new_self_critique_addressed,
        new_5gate_hit,
    }
}

#[derive(Debug, Clone)]
struct ReplayOutcome {
    completed: bool,
    failure_reason: Option<String>,
    original_final_review_status: Option<String>,
    /// G4：源 run 的 5 闸命中向量（threshold 路径不填——其 original 侧 hit 由
    /// significance 用 send_success 口径推；prompt 路径把源 run review.scores
    /// 推回 5 闸口径填这里，喂 significance 真实新旧对照）。
    original_5gate_hit: Document,
    /// G4：源 run 的 selfCritique 是否被解决。
    original_self_critique_addressed: Option<bool>,
    new_final_review_status: Option<String>,
    new_review_risks: Vec<String>,
    new_token_cost: Option<i64>,
    new_self_critique_addressed: Option<bool>,
    new_5gate_hit: Document,
}

impl ReplayOutcome {
    fn failed(reason: &'static str) -> Self {
        Self {
            completed: false,
            failure_reason: Some(reason.to_string()),
            original_final_review_status: None,
            original_5gate_hit: Document::new(),
            original_self_critique_addressed: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
            new_token_cost: None,
            new_self_critique_addressed: None,
            new_5gate_hit: Document::new(),
        }
    }
    fn failed_with(reason: String) -> Self {
        Self {
            completed: false,
            failure_reason: Some(reason),
            original_final_review_status: None,
            original_5gate_hit: Document::new(),
            original_self_critique_addressed: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
            new_token_cost: None,
            new_self_critique_addressed: None,
            new_5gate_hit: Document::new(),
        }
    }
}

async fn persist_replay(
    state: &AppState,
    proposal: &Proposal,
    source_run_id: ObjectId,
    started_at: DateTime,
    outcome: ReplayOutcome,
) -> AppResult<()> {
    let proposal_id = match proposal.id {
        Some(id) => id,
        None => {
            return Err(AppError::External(
                "shadow replay called for proposal without _id".to_string(),
            ));
        }
    };
    let row = ShadowReplay {
        id: None,
        proposal_id,
        experiment_id: proposal.experiment_id.clone(),
        workspace_id: proposal.workspace_id.clone(),
        account_id: proposal.account_id.clone(),
        source_run_id,
        status: if outcome.completed {
            "completed"
        } else {
            "failed"
        }
        .to_string(),
        failure_reason: outcome.failure_reason,
        original_final_review_status: outcome.original_final_review_status,
        original_5gate_hit: outcome.original_5gate_hit,
        original_self_critique_addressed: outcome.original_self_critique_addressed,
        new_final_review_status: outcome.new_final_review_status,
        new_review_risks: outcome.new_review_risks,
        new_token_cost: outcome.new_token_cost,
        new_5gate_hit: outcome.new_5gate_hit,
        new_self_critique_addressed: outcome.new_self_critique_addressed,
        similarity_to_original_text: 0.0,
        started_at,
        finished_at: Some(DateTime::now()),
    };
    state
        .db
        .shadow_replays()
        .insert_one(row, None)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

async fn insert_replay_failed(
    state: &AppState,
    proposal: &Proposal,
    proposal_id: ObjectId,
    source_run_id: ObjectId,
    reason: &'static str,
) -> AppResult<()> {
    let row = ShadowReplay {
        id: None,
        proposal_id,
        experiment_id: proposal.experiment_id.clone(),
        workspace_id: proposal.workspace_id.clone(),
        account_id: proposal.account_id.clone(),
        source_run_id,
        status: "failed".to_string(),
        failure_reason: Some(reason.to_string()),
        original_final_review_status: None,
        new_final_review_status: None,
        new_review_risks: Vec::new(),
        new_token_cost: None,
        new_5gate_hit: Document::new(),
        original_5gate_hit: Document::new(),
        original_self_critique_addressed: None,
        new_self_critique_addressed: None,
        similarity_to_original_text: 0.0,
        started_at: DateTime::now(),
        finished_at: Some(DateTime::now()),
    };
    state
        .db
        .shadow_replays()
        .insert_one(row, None)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn prompt_shadow_retention_probe_uses_full_message_scope() {
        let filter = prompt_shadow_retention_message_filter("ws-a", "acct-a", "wxid-a", "msg-a");
        assert_eq!(filter.get_str("workspace_id").unwrap(), "ws-a");
        assert_eq!(filter.get_str("account_id").unwrap(), "acct-a");
        assert_eq!(filter.get_str("contact_wxid").unwrap(), "wxid-a");
        assert_eq!(filter.get_str("message_id").unwrap(), "msg-a");
    }

    fn mk_run_log(scores: Document, final_status: &str) -> AgentRunLog {
        AgentRunLog {
            id: Some(ObjectId::new()),
            workspace_id: "ws".to_string(),
            account_id: "acct".to_string(),
            contact_wxid: Some("wx_test".to_string()),
            run_id: "run_test".to_string(),
            trigger_kind: "inbound_message".to_string(),
            status: "completed".to_string(),
            planner: Document::new(),
            context: Document::new(),
            knowledge_route: Document::new(),
            decision: Document::new(),
            review: doc! { "scores": scores },
            gateway_result: Document::new(),
            error: None,
            token_budget: 0,
            tokens_used: 0,
            llm_calls_used: 0,
            unknown_usage_calls: 0,
            degraded_reasons: vec![],
            lifecycle: "completed".to_string(),
            source_event_id: "msg_x".to_string(),
            source_kind: "inbound_message".to_string(),
            error_summary: None,
            abort_reason: None,
            revision_applied: false,
            revision_reason: String::new(),
            pre_revision_summary: None,
            post_revision_summary: None,
            self_critique: None,
            autonomy_mode: "auto".to_string(),
            final_review_status: final_status.to_string(),
            outbox_status: None,
            memory_consolidator_warnings: vec![],
            conversation_mode: String::new(),
            conversation_mode_reason: None,
            created_at: DateTime::now(),
        }
    }

    fn mk_threshold_proposal(gate: &str, current: f64, proposed: f64) -> Proposal {
        Proposal {
            id: Some(ObjectId::new()),
            experiment_id: "exp_test".to_string(),
            workspace_id: "ws".to_string(),
            account_id: "acct".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "pending_eval".to_string(),
            gate_key: Some(gate.to_string()),
            current_value: Some(current),
            proposed_value: Some(proposed),
            cohort_notes: Document::new(),
            proposed_template_key: None,
            proposed_section: None,
            diff_summary: None,
            diff_snippet: None,
            critic_reasoning: None,
            expected_improvement_on: vec![],
            risk_note: None,
            base_revision: None,
            released_revision: None,
            previous_prompt_version: None,
            eval_metrics: Document::new(),
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: None,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    /// 4.7 case: 收紧 fact_risk_block 6.0 → 7.0；源 run scores.factRisk=6 → 旧命中、新不命中
    #[test]
    fn evaluate_threshold_relaxes_fact_risk_block() {
        let scores = doc! {
            "factRisk": 6_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(outcome.new_final_review_status.as_deref(), Some("approved"));
        assert_eq!(
            outcome.new_5gate_hit.get_bool("fact_risk_block").unwrap(),
            false
        );
    }

    /// KE-02：源 run 真实终态是**非-5gate**因素（如 blocked_by_budget）但 review.scores
    /// 5 闸全过。修复后 original_final_review_status 必须用 5闸重推值（approved），
    /// 不再是源真实终态——否则 original 侧算"发送失败"、new 侧 5闸算"成功"，凭空 +send_delta。
    /// 回退到 `original.final_review_status.clone()` 即变红。
    #[test]
    fn evaluate_threshold_original_uses_5gate_not_real_terminal() {
        let scores = doc! {
            "factRisk": 1_i32,       // 远低于阈值 → 不 block
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        // 源 run 真实终态 = 非-5gate 因素（预算耗尽），但 scores 5 闸全过。
        let run = mk_run_log(scores, "blocked_by_budget");
        // 放松 fact_risk_block 6→7（升阈候选）；两侧 fact_risk 都不命中（scores 极低）。
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        // KE-02 核心：original 终态 = 5闸重推(approved)，不再是源真实终态 blocked_by_budget。
        assert_eq!(
            outcome.original_final_review_status.as_deref(),
            Some("approved"),
            "original 须用 5闸重推(与 new 侧同口径),不得用源真实非-5gate 终态"
        );
        // new 侧同样 approved（升阈后仍不命中）→ send_delta 对该 run 贡献 0，不再凭空 +。
        assert_eq!(outcome.new_final_review_status.as_deref(), Some("approved"));
    }

    /// 反方向：放松 fact_risk_block 6 → 5，原 factRisk=5 不命中、新命中
    #[test]
    fn evaluate_threshold_tightens_fact_risk_block() {
        let scores = doc! {
            "factRisk": 5_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "approved");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 5.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit.get_bool("fact_risk_block").unwrap(),
            true
        );
        assert_eq!(
            outcome.new_final_review_status.as_deref(),
            Some("held_by_ai_policy")
        );
    }

    /// rewrite 类（emotional_value_rewrite < 阈值则触发）
    #[test]
    fn evaluate_threshold_rewrite_class_triggers_below_threshold() {
        let scores = doc! {
            "factRisk": 0_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 4_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "revision_applied_approved");
        let proposal = mk_threshold_proposal("emotional_value_rewrite", 5.0, 6.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome
                .new_5gate_hit
                .get_bool("emotional_value_rewrite")
                .unwrap(),
            true
        );
        assert_eq!(
            outcome.new_final_review_status.as_deref(),
            Some("revision_applied_approved")
        );
    }

    /// H11 真护栏:源 run 用生产真实序列化键 `hallucinationScore` 时,
    /// evaluate_single_gate 必须读到它(旧代码读 factRisk→miss→0.0)。
    /// seed hallucinationScore=8,收紧 fact_risk_block 6→7 → new 命中(8≥7)。
    /// 旧 bug 下读 0.0→0≥7 false→断言 true 失败。
    #[test]
    fn evaluate_threshold_reads_real_hallucination_score_key() {
        let scores = doc! {
            "hallucinationScore": 8_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "knowledgeGroundingScore": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit.get_bool("fact_risk_block").unwrap(),
            true,
            "hallucinationScore=8 ≥ 7 应命中(旧代码读 factRisk miss→0.0→不命中)"
        );
    }

    /// H11 真护栏(product 方向):源 run 用真实键 knowledgeGroundingScore。
    /// product_accuracy_score_block 是 LT(score<threshold 命中),seed=9,阈值 7
    /// → 9<7 false 不命中。旧 bug 读 productAccuracy→miss→0.0→0<7 true 命中→断言 false 失败。
    #[test]
    fn evaluate_threshold_reads_real_knowledge_grounding_score_key() {
        let scores = doc! {
            "hallucinationScore": 0_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "knowledgeGroundingScore": 9_i32,
        };
        let run = mk_run_log(scores, "approved");
        let proposal = mk_threshold_proposal("product_accuracy_score_block", 7.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit.get_bool("product_accuracy_score_block").unwrap(),
            false,
            "knowledgeGroundingScore=9 ≥ 7 不该触发 product block(旧代码读 productAccuracy miss→0.0→<7 误命中)"
        );
    }

    /// M9 真护栏:original_5gate_hit 必须非空且正确(旧代码恒 Document::new())。
    /// seed hallucinationScore=8;放松 fact_risk_block 6→7(current=6,proposed=7)。
    /// original 用 current=6:8≥6 命中=true。旧代码 original_5gate_hit 空→get_bool None→失败。
    #[test]
    fn evaluate_threshold_fills_original_5gate_hit_baseline() {
        let scores = doc! {
            "hallucinationScore": 8_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "knowledgeGroundingScore": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.original_5gate_hit.get_bool("fact_risk_block"),
            Ok(true),
            "original 用 current_value=6:hallucinationScore=8≥6 命中(旧代码恒空→None)"
        );
        assert_eq!(outcome.new_5gate_hit.get_bool("fact_risk_block"), Ok(true));
    }

    /// 缺 review.scores → failed("source_run_missing_review_scores")
    #[test]
    fn evaluate_threshold_fails_when_review_scores_missing() {
        let mut run = mk_run_log(doc! {}, "approved");
        run.review = Document::new(); // 整个 review 不带 scores
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(!outcome.completed);
        assert_eq!(
            outcome.failure_reason.as_deref(),
            Some("source_run_missing_review_scores")
        );
    }

    /// 任务 4.7 case 5：evaluate_threshold 是纯函数，调用前后多次 invoke
    /// 在相同输入下 SHALL 给出一致输出（决定性 + 无副作用）。该 test 与
    /// `scripts/check-evolution-isolation.sh` 的静态扫描互补——
    /// 静态扫描禁掉 `outbox / mcp::` 引用，单测兜底确认行为决定性。
    #[test]
    fn evaluate_threshold_is_pure_and_deterministic() {
        let scores = doc! {
            "factRisk": 7_i32,
            "pressureRisk": 2_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "productAccuracy": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 8.0);
        let o1 = evaluate_threshold(&proposal, &run);
        let o2 = evaluate_threshold(&proposal, &run);
        let o3 = evaluate_threshold(&proposal, &run);
        assert_eq!(o1.new_final_review_status, o2.new_final_review_status);
        assert_eq!(o2.new_final_review_status, o3.new_final_review_status);
        assert_eq!(o1.new_5gate_hit, o2.new_5gate_hit);
        assert_eq!(o2.new_5gate_hit, o3.new_5gate_hit);
    }

    /// 任务 4.7 case 4：source_message_unavailable / source_run_not_found
    /// 都不算 completed —— ReplayOutcome::failed 全部 completed=false，
    /// 显著性聚合时进入 `eval_replays_failed` 分母。
    #[test]
    fn failed_outcomes_are_not_completed() {
        let o1 = ReplayOutcome::failed("source_run_not_found");
        let o2 = ReplayOutcome::failed("source_message_unavailable");
        let o3 = ReplayOutcome::failed("evolution_budget_exceeded");
        let o4 = ReplayOutcome::failed_with("custom_reason".to_string());
        for o in [&o1, &o2, &o3, &o4] {
            assert!(!o.completed);
            assert!(o.failure_reason.is_some());
        }
    }
}
