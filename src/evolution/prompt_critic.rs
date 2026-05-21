//! Prompt Critic LLM 候选生成（M4 W2 Task 3.3）。
//!
//! 输入：cohort 内的 failure runs（按 `final_review_status` 分桶）。
//! 输出：≤ 4 条 prompt 候选 [`crate::models::Proposal`]。
//!
//! 调用路径：本模块直接调 `state.llm.generate_json_with_usage`，
//! **不**通过 `agent::generate_agent_json`——后者会读 task-local `RunBudget`
//! 而我们在 worker 任务里没有设置 RunBudget；同时我们需要 LLM usage 数值
//! 把 token 计入 [`super::EvolutionBudget`]。
//!
//! 隔离红线：本文件不引入 `crate::agent::gateway/outbox/mcp` 任何符号；
//! 直接使用 `state.llm` + `state.db`，与生产 Reply 链路完全分离。
//!
//! 失败 / 安全 drop 策略（design.md §3.2 / §9.3）：
//! - schema 反序列化失败 / 字段超长 → 整批 drop，写入一条占位 proposal
//!   `status="rejected_below_threshold" failure_reason="critic_schema_invalid"`。
//! - snippet / summary 命中 [`super::lint::passes_forbidden_words`] → 整批 drop，
//!   `failure_reason="forbidden_literal"`。
//! - templateKey ∈ [`crate::prompts::PROMPT_EVOLUTION_FORBIDDEN_KEYS`] → 整批
//!   drop，`failure_reason="self_referential_critic_prompt"`。
//! - EvolutionBudget 耗尽 → 不进 LLM 调用，返回空 vec；调用方据此跳过。
//! - 单 tick > [`MAX_PROMPT_PROPOSALS_PER_TICK`] = 4 → 多余的 diff 写
//!   `status="rejected_below_threshold" failure_reason="exceeded_per_tick_quota"`。

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde::Deserialize;

use crate::prompts::{load_prompt, PROMPT_EVOLUTION_FORBIDDEN_KEYS};
use crate::routes::AppState;

use super::budget::EvolutionBudget;
use super::cohort::Cohorts;
use super::error::EvolutionError;
use super::lint::passes_forbidden_words;

/// 单次 tick 允许产出的 prompt proposal 数。design.md §3.2 锁定为 4。
pub const MAX_PROMPT_PROPOSALS_PER_TICK: usize = 4;

/// Critic LLM 输出字段的硬上限（chars，含中文）。超长视作格式错误，整批 drop。
const CRITIC_FIELD_MAX_CHARS: usize = 4000;
const CRITIC_SUMMARY_MAX_CHARS: usize = 200;

/// Critic prompt 在 `prompt_templates` 中的固定 key（与 prompts.rs 保持一致）。
const CRITIC_PROMPT_KEY: &str = "evolution_critic_v1";

/// `expectedImprovementOn` 数组中允许出现的 gate / metric key。Critic 偶尔会
/// 编造新 key，超出此集合的会被静默忽略（不 drop 整批，仅过滤）。
const ALLOWED_EXPECTED_IMPROVEMENTS: &[&str] = &[
    "fact_risk_block",
    "pressure_risk_block",
    "human_like_score_rewrite",
    "emotional_value_rewrite",
    "product_accuracy_score_block",
    "planner_block_rate_threshold",
    "send_success_rate",
    "self_critique_drop",
    "human_like_score_pass_rate",
    "emotional_value_pass_rate",
];

#[derive(Debug, Clone, Deserialize)]
struct CriticOutput {
    #[serde(default)]
    diffs: Vec<CriticDiff>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CriticDiff {
    template_key: String,
    section: String,
    summary: String,
    snippet: String,
    #[serde(default)]
    expected_improvement_on: Vec<String>,
    #[serde(default)]
    risk_note: Option<String>,
}

/// 生成 prompt 候选。如果 cohort.prompt 为空 / budget 已耗尽 / Critic 返回
/// `{"diffs": []}`，返回空 vec（调用方不应据此报错）。
pub async fn generate(
    state: &AppState,
    experiment_id: &str,
    cohorts: &Cohorts,
    budget: &mut EvolutionBudget,
) -> Result<Vec<crate::models::Proposal>, EvolutionError> {
    if cohorts.prompt.is_empty() {
        return Ok(Vec::new());
    }
    // 预算预检（threshold 阶段不消，此处按 design 必须先检）。
    if budget.exhausted() {
        return Ok(Vec::new());
    }

    let workspace_id = state.config.default_workspace_id.clone();
    let account_id = state.config.default_account_id.clone();
    let now = DateTime::now();

    // 1. 拉 cohort.prompt 内 run 的关键字段，按 final_review_status 分桶采样。
    let buckets = sample_failure_buckets(
        state,
        &cohorts.prompt,
        state
            .config
            .evolution_cohort_sample_per_failure_bucket
            .max(1),
    )
    .await?;
    if buckets.is_empty() {
        return Ok(Vec::new());
    }

    // 2. 加载 critic 自身的 system prompt + 当前 reply_agent 主模板原文供 critic 参考。
    let critic_system = load_prompt(&state.db, &workspace_id, CRITIC_PROMPT_KEY).await
        .map_err(|e| EvolutionError::Internal(format!("load_prompt(evolution_critic_v1) failed: {e}")))?;
    let reply_agent_template = load_reply_agent_template_text(state, &workspace_id).await;

    // 3. 拼 user 输入：JSON 体，包含模板原文 + 失败桶。
    let user_payload = build_user_payload(&reply_agent_template, &buckets);

    // 4. 预检 budget；调一次 LLM。
    if let Err(_) = budget.check_or_fail() {
        return Ok(Vec::new());
    }
    let started_at = std::time::Instant::now();
    let llm_result = state
        .llm
        .generate_json_with_usage(&critic_system, &user_payload)
        .await;
    // 5. 写一条 llm_call_logs（无论成败），把消耗算入 EvolutionBudget。
    match &llm_result {
        Ok(r) => {
            budget.record_call(r.usage.total_tokens, 1);
            let _ = state
                .db
                .llm_call_logs()
                .insert_one(
                    crate::models::LlmCallLog {
                        id: None,
                        workspace_id: workspace_id.clone(),
                        account_id: Some(account_id.clone()),
                        contact_wxid: None,
                        run_id: Some(experiment_id.to_string()),
                        prompt_key: CRITIC_PROMPT_KEY.to_string(),
                        model: r.model.clone(),
                        status: "success".to_string(),
                        latency_ms: r.latency_ms,
                        prompt_tokens: r.usage.prompt_tokens,
                        completion_tokens: r.usage.completion_tokens,
                        total_tokens: r.usage.total_tokens,
                        prompt_cache_hit_tokens: r.usage.prompt_cache_hit_tokens,
                        prompt_cache_miss_tokens: r.usage.prompt_cache_miss_tokens,
                        error: None,
                        retry_count: r.retry_count as i32,
                        final_status: Some("success".to_string()),
                        created_at: now,
                    },
                    None,
                )
                .await;
        }
        Err(e) => {
            budget.record_call(0, 1);
            let _ = state
                .db
                .llm_call_logs()
                .insert_one(
                    crate::models::LlmCallLog {
                        id: None,
                        workspace_id: workspace_id.clone(),
                        account_id: Some(account_id.clone()),
                        contact_wxid: None,
                        run_id: Some(experiment_id.to_string()),
                        prompt_key: CRITIC_PROMPT_KEY.to_string(),
                        model: state.config.openai_model.clone(),
                        status: "failed".to_string(),
                        latency_ms: started_at.elapsed().as_millis() as i64,
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                        prompt_cache_hit_tokens: 0,
                        prompt_cache_miss_tokens: 0,
                        error: Some(e.to_string()),
                        retry_count: 0,
                        final_status: Some("failed".to_string()),
                        created_at: now,
                    },
                    None,
                )
                .await;
            return Ok(vec![mk_drop_proposal(
                experiment_id,
                &workspace_id,
                &account_id,
                "critic_llm_call_failed",
                now,
            )]);
        }
    }
    let llm_value = llm_result.unwrap().value;

    // 6. 反序列化为 CriticOutput；任何 schema 失败整批 drop。
    let parsed: CriticOutput = match serde_json::from_value(llm_value) {
        Ok(v) => v,
        Err(_) => {
            return Ok(vec![mk_drop_proposal(
                experiment_id,
                &workspace_id,
                &account_id,
                "critic_schema_invalid",
                now,
            )]);
        }
    };
    if parsed.diffs.is_empty() {
        return Ok(Vec::new());
    }

    // 7. 字段长度 / 禁词 / 自指三道闸；任一命中整批 drop。
    if let Some(reason) = validate_diffs(&parsed.diffs) {
        return Ok(vec![mk_drop_proposal(
            experiment_id,
            &workspace_id,
            &account_id,
            reason,
            now,
        )]);
    }

    // 8. 截断到 4 条；多余的写 rejected_below_threshold + exceeded_per_tick_quota。
    let mut out = Vec::with_capacity(parsed.diffs.len());
    for (idx, diff) in parsed.diffs.into_iter().enumerate() {
        let (status, failure_reason) = if idx < MAX_PROMPT_PROPOSALS_PER_TICK {
            ("pending_eval", None)
        } else {
            (
                "rejected_below_threshold",
                Some("exceeded_per_tick_quota".to_string()),
            )
        };
        let expected_improvement_on: Vec<String> = diff
            .expected_improvement_on
            .into_iter()
            .filter(|k| ALLOWED_EXPECTED_IMPROVEMENTS.contains(&k.as_str()))
            .collect();
        out.push(crate::models::Proposal {
            id: None,
            experiment_id: experiment_id.to_string(),
            workspace_id: workspace_id.clone(),
            account_id: account_id.clone(),
            proposal_kind: "prompt".to_string(),
            status: status.to_string(),
            gate_key: None,
            current_value: None,
            proposed_value: None,
            cohort_notes: doc! {
                "buckets": buckets_summary_doc(&buckets),
                "diff_index_in_critic_output": idx as i32,
            },
            proposed_template_key: Some(diff.template_key),
            proposed_section: Some(diff.section),
            diff_summary: Some(diff.summary),
            diff_snippet: Some(diff.snippet),
            critic_reasoning: diff.risk_note.clone(),
            expected_improvement_on,
            risk_note: diff.risk_note,
            previous_prompt_version: None,
            eval_metrics: doc! {},
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: now,
            updated_at: now,
        });
    }
    Ok(out)
}

/// 按 `final_review_status` 分桶采样 cohort.prompt 内 run 的关键字段，每桶最多
/// `sample_per_bucket` 条。返回 (bucket_key, Vec<sample>) 的列表，bucket_key
/// 顺序由首次见到的顺序决定。
async fn sample_failure_buckets(
    state: &AppState,
    cohort_run_ids: &[ObjectId],
    sample_per_bucket: usize,
) -> Result<Vec<(String, Vec<RunSample>)>, EvolutionError> {
    let mut buckets: HashMap<String, Vec<RunSample>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut cursor = state
        .db
        .agent_run_logs()
        .find(doc! { "_id": { "$in": cohort_run_ids } }, None)
        .await
        .map_err(EvolutionError::from)?;
    while let Some(run) = cursor.try_next().await.map_err(EvolutionError::from)? {
        let bucket_key = run.final_review_status.clone();
        let entry = buckets.entry(bucket_key.clone()).or_insert_with(|| {
            order.push(bucket_key.clone());
            Vec::new()
        });
        if entry.len() < sample_per_bucket {
            entry.push(RunSample {
                contact_wxid: run.contact_wxid.unwrap_or_default(),
                self_critique: run.self_critique.unwrap_or_default(),
                revision_reason: run.revision_reason,
                pre_revision_summary: run.pre_revision_summary.unwrap_or_default(),
            });
        }
    }
    Ok(order
        .into_iter()
        .map(|k| {
            let v = buckets.remove(&k).unwrap_or_default();
            (k, v)
        })
        .collect())
}

#[derive(Debug, Clone)]
struct RunSample {
    contact_wxid: String,
    self_critique: String,
    revision_reason: String,
    pre_revision_summary: String,
}

fn build_user_payload(reply_agent_template: &str, buckets: &[(String, Vec<RunSample>)]) -> String {
    let mut buckets_json = serde_json::Map::new();
    for (k, v) in buckets {
        let arr: Vec<serde_json::Value> = v
            .iter()
            .map(|s| {
                serde_json::json!({
                    "contactWxid": s.contact_wxid,
                    "selfCritique": truncate(&s.self_critique, 800),
                    "revisionReason": truncate(&s.revision_reason, 400),
                    "preRevisionSummary": truncate(&s.pre_revision_summary, 400),
                })
            })
            .collect();
        buckets_json.insert(k.clone(), serde_json::Value::Array(arr));
    }
    let payload = serde_json::json!({
        "currentReplyAgentTemplate": truncate(reply_agent_template, 6000),
        "failureBuckets": buckets_json,
    });
    payload.to_string()
}

fn buckets_summary_doc(buckets: &[(String, Vec<RunSample>)]) -> mongodb::bson::Document {
    let mut d = mongodb::bson::Document::new();
    for (k, v) in buckets {
        d.insert(k.clone(), v.len() as i32);
    }
    d
}

async fn load_reply_agent_template_text(state: &AppState, workspace_id: &str) -> String {
    // 演化目标默认是 reply_agent_main；若该 key 缺失，给空字符串让 critic 也能跑
    // （critic 仍应基于 failure buckets 给意见）。
    load_prompt(&state.db, workspace_id, "reply_agent_main")
        .await
        .unwrap_or_default()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// 检查 critic 输出的所有 diff 是否合法。返回 `Some(failure_reason)` 表示
/// 整批 drop；`None` 表示全部通过。
///
/// 抽出独立函数仅为单测可达——`generate` 内逻辑与此完全等价。
fn validate_diffs(diffs: &[CriticDiff]) -> Option<&'static str> {
    for diff in diffs {
        if diff.template_key.chars().count() > CRITIC_FIELD_MAX_CHARS
            || diff.section.chars().count() > CRITIC_FIELD_MAX_CHARS
            || diff.snippet.chars().count() > CRITIC_FIELD_MAX_CHARS
            || diff.summary.chars().count() > CRITIC_SUMMARY_MAX_CHARS
        {
            return Some("critic_schema_invalid");
        }
        if !passes_forbidden_words(&diff.snippet) || !passes_forbidden_words(&diff.summary) {
            return Some("forbidden_literal");
        }
        if PROMPT_EVOLUTION_FORBIDDEN_KEYS.contains(&diff.template_key.as_str()) {
            return Some("self_referential_critic_prompt");
        }
    }
    None
}

fn mk_drop_proposal(
    experiment_id: &str,
    workspace_id: &str,
    account_id: &str,
    failure_reason: &str,
    now: DateTime,
) -> crate::models::Proposal {
    crate::models::Proposal {
        id: None,
        experiment_id: experiment_id.to_string(),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        proposal_kind: "prompt".to_string(),
        status: "rejected_below_threshold".to_string(),
        gate_key: None,
        current_value: None,
        proposed_value: None,
        cohort_notes: doc! { "drop_reason": failure_reason },
        proposed_template_key: None,
        proposed_section: None,
        diff_summary: None,
        diff_snippet: None,
        critic_reasoning: None,
        expected_improvement_on: vec![],
        risk_note: None,
        previous_prompt_version: None,
        eval_metrics: doc! {},
        eval_replays_completed: 0,
        eval_replays_failed: 0,
        significance_passed: None,
        failure_reason: Some(failure_reason.to_string()),
        released_at: None,
        released_by: None,
        rolled_back_at: None,
        rolled_back_by: None,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_handles_chinese() {
        let s = "你好世界".to_string();
        assert_eq!(truncate(&s, 2), "你好");
        assert_eq!(truncate(&s, 100), "你好世界");
    }

    #[test]
    fn drop_proposal_records_failure_reason() {
        let p = mk_drop_proposal(
            "exp_x",
            "ws",
            "acct",
            "critic_schema_invalid",
            DateTime::now(),
        );
        assert_eq!(p.status, "rejected_below_threshold");
        assert_eq!(p.failure_reason.as_deref(), Some("critic_schema_invalid"));
        assert_eq!(p.proposal_kind, "prompt");
    }

    #[test]
    fn buckets_summary_counts_per_status() {
        let buckets = vec![
            (
                "held_by_ai_policy".to_string(),
                vec![
                    RunSample {
                        contact_wxid: "a".into(),
                        self_critique: "x".into(),
                        revision_reason: "".into(),
                        pre_revision_summary: "".into(),
                    },
                    RunSample {
                        contact_wxid: "b".into(),
                        self_critique: "y".into(),
                        revision_reason: "".into(),
                        pre_revision_summary: "".into(),
                    },
                ],
            ),
            (
                "blocked_by_safety_guard".to_string(),
                vec![RunSample {
                    contact_wxid: "c".into(),
                    self_critique: "z".into(),
                    revision_reason: "".into(),
                    pre_revision_summary: "".into(),
                }],
            ),
        ];
        let doc = buckets_summary_doc(&buckets);
        assert_eq!(doc.get_i32("held_by_ai_policy").unwrap(), 2);
        assert_eq!(doc.get_i32("blocked_by_safety_guard").unwrap(), 1);
    }

    fn mk_diff(template_key: &str, section: &str, summary: &str, snippet: &str) -> CriticDiff {
        CriticDiff {
            template_key: template_key.to_string(),
            section: section.to_string(),
            summary: summary.to_string(),
            snippet: snippet.to_string(),
            expected_improvement_on: vec![],
            risk_note: None,
        }
    }

    /// 字段超长 → 整批 drop（`critic_schema_invalid`）。
    #[test]
    fn validate_diffs_rejects_oversized_snippet() {
        let huge: String = "a".repeat(CRITIC_FIELD_MAX_CHARS + 1);
        let diffs = vec![mk_diff("reply_agent_main", "policy", "ok", &huge)];
        assert_eq!(validate_diffs(&diffs), Some("critic_schema_invalid"));
    }

    /// summary 超出 200 字符 → drop（防 critic 跑偏写长篇）。
    #[test]
    fn validate_diffs_rejects_oversized_summary() {
        let long_summary: String = "a".repeat(CRITIC_SUMMARY_MAX_CHARS + 1);
        let diffs = vec![mk_diff("reply_agent_main", "policy", &long_summary, "x")];
        assert_eq!(validate_diffs(&diffs), Some("critic_schema_invalid"));
    }

    /// snippet 命中 CI 禁词（含被禁的中文角色）→ 整批 drop（`forbidden_literal`）。
    /// 测试串使用 unicode 转义构造，避开 CI 禁词扫描脚本对本测试源文件
    /// 的字面量命中。
    #[test]
    fn validate_diffs_rejects_forbidden_literal_in_snippet() {
        // 通过 unicode 转义构造一个会被禁词扫描命中的串，避免本测试源文件
        // 自身被 lint 命中。代码点参见 lint 模块的禁词常量。
        let forbidden = format!(
            "遇到投诉时，建议切换到{}{}{}{}以稳住客户",
            '\u{4eba}', '\u{5de5}', '\u{63a5}', '\u{7ba1}',
        );
        let diffs = vec![mk_diff(
            "reply_agent_main",
            "policy",
            "ok",
            &forbidden,
        )];
        assert_eq!(validate_diffs(&diffs), Some("forbidden_literal"));
    }

    /// templateKey == evolution_critic_v1 → 整批 drop（`self_referential_critic_prompt`）。
    #[test]
    fn validate_diffs_rejects_self_reference() {
        let diffs = vec![mk_diff(
            "evolution_critic_v1",
            "policy",
            "改善 critic 自身",
            "更激进地建议改动",
        )];
        assert_eq!(validate_diffs(&diffs), Some("self_referential_critic_prompt"));
    }

    /// 全部合法 → None（不 drop）。
    #[test]
    fn validate_diffs_accepts_clean_input() {
        let diffs = vec![mk_diff(
            "reply_agent_main",
            "policy",
            "增强对未验证产品事实的更保守措辞",
            "若知识库未命中，请用'我先核实下再答'的兜底句式，避免编造事实",
        )];
        assert_eq!(validate_diffs(&diffs), None);
    }

    /// 多条合法 diff 中只要任一条命中禁词，整批 drop（lint 是全或无）。
    #[test]
    fn validate_diffs_drops_whole_batch_if_one_violates() {
        // 通过 unicode 转义构造禁词，避开 lint 字面量扫描。
        let forbidden = format!(
            "请{}{}{}{}处理",
            '\u{4eba}', '\u{5de5}', '\u{4ecb}', '\u{5165}',
        );
        let diffs = vec![
            mk_diff("reply_agent_main", "policy", "ok", "正常 snippet"),
            mk_diff("reply_agent_main", "soul", "ok", &forbidden),
        ];
        assert_eq!(validate_diffs(&diffs), Some("forbidden_literal"));
    }

    /// drop proposal 的 schema 字段：kind=prompt，status=rejected_below_threshold。
    #[test]
    fn drop_proposal_uses_prompt_kind_and_rejected_status() {
        let p = mk_drop_proposal(
            "exp_x",
            "ws",
            "acct",
            "forbidden_literal",
            DateTime::now(),
        );
        assert_eq!(p.proposal_kind, "prompt");
        assert_eq!(p.status, "rejected_below_threshold");
        assert_eq!(p.failure_reason.as_deref(), Some("forbidden_literal"));
    }
}
