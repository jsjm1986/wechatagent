//! agent-self-evolution M4 W4 Task 5.2：演化器 release 路径。
//!
//! `release_threshold` / `release_prompt` 是把 `eligible_for_release` 候选写入
//! 生产生效集合的唯一入口。两个函数都通过 mongo session transaction 把
//! `threshold_overrides` / `prompt_templates` 的写入与 `proposals.status` 的
//! 推进绑成 atomic，避免出现"已 release 但 proposal 状态还是 eligible"的
//! 污染状态（Requirements 6.3 / 6.4）。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、
//! `agent_send_outbox` 写入路径，或 `run_user_operation_gateway / handle_managed_message
//! / handle_follow_up_task` 等生产链路入口。`scripts/check-evolution-isolation.sh`
//! 在 CI 内静态扫描该目录强制此约束。

use std::sync::atomic::Ordering;

use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    options::{FindOneOptions, TransactionOptions},
    ClientSession,
};

use crate::routes::AppState;

use super::error::EvolutionError;

/// 把 status="eligible_for_release" 的 threshold proposal 落地到 `threshold_overrides`。
///
/// 写入路径（mongo transaction）：
/// 1. 重新加载 proposal，校验 `proposal_kind="threshold"` + `status="eligible_for_release"`；
///    其它状态返回 `EvolutionError::InvalidStatus`，事务不开始
/// 2. insert 一条新 `threshold_overrides` 文档（`rolled_back_at=null`）
/// 3. update `proposals.status="released"` + `released_at` + `released_by`
/// 4. commit 后写一条 `agent_events kind="evolution_threshold_released"`
///
/// 不消耗 `EvolutionBudget`（release 不调 LLM）。
pub async fn release_threshold(
    state: &AppState,
    proposal_id: ObjectId,
    admin: &str,
) -> Result<(), EvolutionError> {
    let proposal = state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}")))?;

    if proposal.proposal_kind != "threshold" {
        return Err(EvolutionError::InvalidStatus(format!(
            "expected proposal_kind=threshold, got {}",
            proposal.proposal_kind
        )));
    }
    if proposal.status != "eligible_for_release" {
        return Err(EvolutionError::InvalidStatus(format!(
            "proposal not eligible for release (status={})",
            proposal.status
        )));
    }
    let gate_key = proposal.gate_key.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal missing gate_key: {proposal_id}"
        ))
    })?;
    let proposed_value = proposal.proposed_value.ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal missing proposed_value: {proposal_id}"
        ))
    })?;

    let now = DateTime::now();
    let workspace_id = proposal.workspace_id.clone();
    let account_id = proposal.account_id.clone();

    let client = state.db.client();
    let mut session = client
        .start_session(None)
        .await
        .map_err(EvolutionError::from)?;
    let txn_opts = TransactionOptions::builder().build();
    session
        .start_transaction(txn_opts)
        .await
        .map_err(EvolutionError::from)?;

    let override_doc = doc! {
        "workspace_id": &workspace_id,
        "account_id": &account_id,
        "gate_key": &gate_key,
        "value": proposed_value,
        "source_proposal_id": proposal_id,
        "released_at": now,
        "released_by": admin,
        "rolled_back_at": null,
        "rolled_back_by": null,
    };
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("threshold_overrides")
        .insert_one_with_session(override_doc, None, &mut session)
        .await
        .map_err(EvolutionError::from)?;

    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! { "_id": proposal_id },
            doc! {
                "$set": {
                    "status": "released",
                    "released_at": now,
                    "released_by": admin,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    // #155(P1)：audit 行与 override / proposal 推进写在同一 transaction，commit 前
    // 完成。旧实现 commit 后才 best-effort 写 + 仅 warn，阈值变更可能无审计行生效。
    let release_audit = build_threshold_override_audit(
        &workspace_id,
        &account_id,
        &gate_key,
        "released",
        proposal.current_value,
        Some(proposed_value),
        proposal_id,
        admin,
        proposal.cohort_notes.get_f64("hit_rate_observed").ok(),
        Some(proposal.eval_metrics.clone()),
    );
    state
        .db
        .threshold_overrides_audit()
        .insert_one_with_session(release_audit, None, &mut session)
        .await
        .map_err(EvolutionError::from)?;

    commit_with_session(&mut session).await?;

    write_release_event(
        state,
        "evolution_threshold_released",
        &workspace_id,
        &account_id,
        proposal_id,
        admin,
        Some(doc! {
            "gate_key": &gate_key,
            "proposed_value": proposed_value,
            "current_value": proposal.current_value.unwrap_or(0.0),
        }),
    )
    .await?;

    // M4 W4 Task 5.6：登记 +24h post-release review 任务。失败仅 warn，不影响 release。
    if let Err(e) = super::post_release::schedule_post_release_review(
        state,
        proposal_id,
        &workspace_id,
        &account_id,
        "threshold",
        now,
    )
    .await
    {
        tracing::warn!(?e, "schedule_post_release_review failed for threshold release; continuing");
    }

    Ok(())
}

/// 把 status="eligible_for_release" 的 prompt proposal 落地到 `prompt_templates`。
///
/// 写入路径（mongo transaction）：
/// 1. 重新加载 proposal，校验 `proposal_kind="prompt"` + `status="eligible_for_release"`
/// 2. 加载 `(workspace_id, prompt_key, current_version=true)` 那条；不存在则
///    `InvalidStatus`（不应当发生：seed 总会保证有 current）
/// 3. 把旧 current 置 `current_version=false`
/// 4. insert 新一条 `version = old.version + 1`、`current_version=true`、
///    `previous_version = Some(old.version)`、`seeded_by="evolution_release"`、
///    `content` = proposal.diff_snippet（W4 简化路径：把整段 diff_snippet 当成新 content）
/// 5. update proposals: `status="released"`、`released_at`、`released_by`、
///    `previous_prompt_version = old.version.to_string()`
/// 6. commit 后 `state.prompt_pack_version.fetch_add(1, SeqCst)` 让 LRU cache 立即失效
/// 7. 写一条 `agent_events kind="evolution_prompt_released"`
pub async fn release_prompt(
    state: &AppState,
    proposal_id: ObjectId,
    admin: &str,
) -> Result<(), EvolutionError> {
    let proposal = state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}")))?;

    if proposal.proposal_kind != "prompt" {
        return Err(EvolutionError::InvalidStatus(format!(
            "expected proposal_kind=prompt, got {}",
            proposal.proposal_kind
        )));
    }
    if proposal.status != "eligible_for_release" {
        return Err(EvolutionError::InvalidStatus(format!(
            "proposal not eligible for release (status={})",
            proposal.status
        )));
    }
    let prompt_key = proposal.proposed_template_key.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing proposed_template_key: {proposal_id}"
        ))
    })?;
    let new_content = proposal.diff_snippet.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing diff_snippet (W4 release path requires a complete content body): {proposal_id}"
        ))
    })?;

    let workspace_id = proposal.workspace_id.clone();
    let account_id = proposal.account_id.clone();

    let current = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "prompt_key": &prompt_key,
                "current_version": true,
            },
            FindOneOptions::default(),
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!(
                "no current_version prompt template for key={prompt_key} workspace={workspace_id}"
            ))
        })?;
    let old_version = current.version;
    let new_version = old_version + 1;
    let now = DateTime::now();

    let client = state.db.client();
    let mut session = client
        .start_session(None)
        .await
        .map_err(EvolutionError::from)?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await
        .map_err(EvolutionError::from)?;

    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .update_one_with_session(
            doc! {
                "workspace_id": &workspace_id,
                "prompt_key": &prompt_key,
                "current_version": true,
            },
            doc! {
                "$set": {
                    "current_version": false,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    let new_template = doc! {
        "workspace_id": &workspace_id,
        "prompt_key": &prompt_key,
        "agent_kind": &current.agent_kind,
        "layer": &current.layer,
        "title": &current.title,
        "description": current.description.clone().unwrap_or_default(),
        "content": &new_content,
        "status": &current.status,
        "version": new_version,
        "prompt_pack_version": &current.prompt_pack_version,
        "created_by": admin,
        "created_at": now,
        "updated_at": now,
        "current_version": true,
        "previous_version": old_version,
        "seeded_by": "evolution_release",
    };
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .insert_one_with_session(new_template, None, &mut session)
        .await
        .map_err(EvolutionError::from)?;

    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! { "_id": proposal_id },
            doc! {
                "$set": {
                    "status": "released",
                    "released_at": now,
                    "released_by": admin,
                    "previous_prompt_version": old_version.to_string(),
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    commit_with_session(&mut session).await?;

    // commit 后再 bump cache version——commit 失败时 cache 不会被错误地标脏。
    state.prompt_pack_version.fetch_add(1, Ordering::SeqCst);

    write_release_event(
        state,
        "evolution_prompt_released",
        &workspace_id,
        &account_id,
        proposal_id,
        admin,
        Some(doc! {
            "prompt_key": &prompt_key,
            "old_version": old_version,
            "new_version": new_version,
            "section": proposal.proposed_section.clone().unwrap_or_default(),
        }),
    )
    .await?;

    // M4 W4 Task 5.6：登记 +24h post-release review 任务。失败仅 warn，不影响 release。
    if let Err(e) = super::post_release::schedule_post_release_review(
        state,
        proposal_id,
        &workspace_id,
        &account_id,
        "prompt",
        now,
    )
    .await
    {
        tracing::warn!(?e, "schedule_post_release_review failed for prompt release; continuing");
    }

    Ok(())
}

/// commit transaction，遇到瞬时错误（`UnknownTransactionCommitResult`）按 mongo
/// 推荐做法重试一次。
async fn commit_with_session(session: &mut ClientSession) -> Result<(), EvolutionError> {
    loop {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(e) if e.contains_label("UnknownTransactionCommitResult") => {
                continue;
            }
            Err(e) => return Err(EvolutionError::from(e)),
        }
    }
}

/// 把已 release 的 threshold proposal 回滚——把对应 `threshold_overrides`
/// 文档置 `rolled_back_at=now`，并把 proposal 推到 `rolled_back`。
///
/// `resolve_thresholds` 读 override 时已过滤 `rolled_back_at=null`，因此回滚后
/// 下一个 run 立即读回到上一档（baseline 来自 contact.runtime_parameters /
/// AppConfig）。Requirements 6.6。
pub async fn rollback_threshold(
    state: &AppState,
    proposal_id: ObjectId,
    admin: &str,
) -> Result<(), EvolutionError> {
    let proposal = state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}")))?;

    if proposal.proposal_kind != "threshold" {
        return Err(EvolutionError::InvalidStatus(format!(
            "expected proposal_kind=threshold, got {}",
            proposal.proposal_kind
        )));
    }
    if proposal.status != "released" {
        return Err(EvolutionError::InvalidStatus(format!(
            "proposal not released (status={}); rollback rejected",
            proposal.status
        )));
    }

    let now = DateTime::now();
    let workspace_id = proposal.workspace_id.clone();
    let account_id = proposal.account_id.clone();

    let client = state.db.client();
    let mut session = client
        .start_session(None)
        .await
        .map_err(EvolutionError::from)?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await
        .map_err(EvolutionError::from)?;

    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("threshold_overrides")
        .update_one_with_session(
            doc! {
                "source_proposal_id": proposal_id,
                "rolled_back_at": null,
            },
            doc! {
                "$set": {
                    "rolled_back_at": now,
                    "rolled_back_by": admin,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! { "_id": proposal_id },
            doc! {
                "$set": {
                    "status": "rolled_back",
                    "rolled_back_at": now,
                    "rolled_back_by": admin,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    // #155(P1)：rollback 的 audit 行也写进同一 transaction，commit 前完成。
    // previous = 被回滚的 proposed_value；new_value 留 None（回滚后回到 baseline 或
    // 更早 override，由审计读路径自行还原）。
    let rollback_audit = build_threshold_override_audit(
        &workspace_id,
        &account_id,
        proposal.gate_key.as_deref().unwrap_or(""),
        "rolled_back",
        proposal.proposed_value,
        None,
        proposal_id,
        admin,
        None,
        None,
    );
    state
        .db
        .threshold_overrides_audit()
        .insert_one_with_session(rollback_audit, None, &mut session)
        .await
        .map_err(EvolutionError::from)?;

    commit_with_session(&mut session).await?;

    write_release_event(
        state,
        "evolution_rollback_completed",
        &workspace_id,
        &account_id,
        proposal_id,
        admin,
        Some(doc! {
            "kind": "threshold",
            "gate_key": proposal.gate_key.clone().unwrap_or_default(),
        }),
    )
    .await?;

    Ok(())
}

/// 把已 release 的 prompt proposal 回滚——把当前 `current_version=true` 那条置
/// false，把 `previous_version` 那条置 true。proposal 推到 `rolled_back`。
///
/// 回滚后 commit 也 fetch_add `prompt_pack_version`，让 LRU 立即失效。
/// Requirements 6.6。
pub async fn rollback_prompt(
    state: &AppState,
    proposal_id: ObjectId,
    admin: &str,
) -> Result<(), EvolutionError> {
    let proposal = state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id }, None)
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}")))?;

    if proposal.proposal_kind != "prompt" {
        return Err(EvolutionError::InvalidStatus(format!(
            "expected proposal_kind=prompt, got {}",
            proposal.proposal_kind
        )));
    }
    if proposal.status != "released" {
        return Err(EvolutionError::InvalidStatus(format!(
            "proposal not released (status={}); rollback rejected",
            proposal.status
        )));
    }
    let prompt_key = proposal.proposed_template_key.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing proposed_template_key: {proposal_id}"
        ))
    })?;
    let previous_version_str = proposal.previous_prompt_version.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing previous_prompt_version (was it released by W4 release_prompt?): {proposal_id}"
        ))
    })?;
    let previous_version: i32 = previous_version_str.parse().map_err(|_| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal previous_prompt_version not parseable as i32: {previous_version_str}"
        ))
    })?;

    let workspace_id = proposal.workspace_id.clone();
    let account_id = proposal.account_id.clone();
    let now = DateTime::now();

    let client = state.db.client();
    let mut session = client
        .start_session(None)
        .await
        .map_err(EvolutionError::from)?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await
        .map_err(EvolutionError::from)?;

    // 1. 把当前 current 置 false
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .update_one_with_session(
            doc! {
                "workspace_id": &workspace_id,
                "prompt_key": &prompt_key,
                "current_version": true,
            },
            doc! {
                "$set": {
                    "current_version": false,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 2. 把 previous_version 那条重新置 true
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .update_one_with_session(
            doc! {
                "workspace_id": &workspace_id,
                "prompt_key": &prompt_key,
                "version": previous_version,
            },
            doc! {
                "$set": {
                    "current_version": true,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 3. 推 proposal 到 rolled_back
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! { "_id": proposal_id },
            doc! {
                "$set": {
                    "status": "rolled_back",
                    "rolled_back_at": now,
                    "rolled_back_by": admin,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    commit_with_session(&mut session).await?;

    state.prompt_pack_version.fetch_add(1, Ordering::SeqCst);

    write_release_event(
        state,
        "evolution_rollback_completed",
        &workspace_id,
        &account_id,
        proposal_id,
        admin,
        Some(doc! {
            "kind": "prompt",
            "prompt_key": &prompt_key,
            "rolled_back_to_version": previous_version,
        }),
    )
    .await?;

    Ok(())
}

async fn write_release_event(
    state: &AppState,
    kind: &str,
    workspace_id: &str,
    account_id: &str,
    proposal_id: ObjectId,
    admin: &str,
    extra: Option<mongodb::bson::Document>,
) -> Result<(), EvolutionError> {
    let mut details = doc! {
        "proposal_id": proposal_id,
        "released_by": admin,
    };
    if let Some(extra) = extra {
        for (k, v) in extra {
            details.insert(k, v);
        }
    }
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: kind.to_string(),
        status: "ok".to_string(),
        summary: format!("evolution release: {kind} by {admin} for proposal {proposal_id}"),
        details: Some(details),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

/// Phase C / C5：在 `threshold_overrides_audit` 追加一条不可变变更日志。
///
/// release / rollback / auto-release 三条主路径在 commit 成功之后调用，失败仅
/// warn——audit 是事后审计字段，缺一行不影响主路径正确性，但绝不能因为 audit
/// 写失败就回滚已经落地的 threshold 变更。
///
/// `previous_value` / `new_value` 调用方根据动作语义传入：
///   - released：previous = 上一条 active override.value（无则 baseline 兜底）, new = proposal.proposed_value
///   - rolled_back：previous = proposal.proposed_value（即被回滚的值）, new = 回滚后生效值（baseline 或更早 override）
/// Phase C / C5 + #155(P1)：构造一条 `threshold_overrides_audit`。
///
/// 不再独立 `insert_one`（旧实现 commit 后 best-effort + 仅 warn，阈值变更可能在
/// 无审计行的情况下生效）。调用方现在在 release / rollback 的同一 transaction 内
/// `insert_one_with_session(...)` 写入本 struct，commit 前完成——审计行与阈值
/// 变更 atomic：要么都生效要么都回滚。
#[allow(clippy::too_many_arguments)]
fn build_threshold_override_audit(
    workspace_id: &str,
    account_id: &str,
    gate_key: &str,
    action: &str,
    previous_value: Option<f64>,
    new_value: Option<f64>,
    source_proposal_id: ObjectId,
    decided_by: &str,
    hit_rate_observed: Option<f64>,
    significance_metrics: Option<mongodb::bson::Document>,
) -> crate::models::ThresholdOverrideAudit {
    crate::models::ThresholdOverrideAudit {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        gate_key: gate_key.to_string(),
        action: action.to_string(),
        previous_value,
        new_value,
        source_proposal_id,
        decided_by: decided_by.to_string(),
        decided_at: DateTime::now(),
        hit_rate_observed,
        significance_metrics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W4 Task 5.2：构造 mock proposal 触发各路径的 InvalidStatus 分支可读性。
    /// 实际写库 + transaction 路径靠 W4 Task 5.9 的 testcontainers 集成测试覆盖
    /// （`tests/evolution_threshold_e2e.rs` / `tests/evolution_prompt_e2e.rs`）。
    #[test]
    fn invalid_status_messages_carry_actionable_context() {
        let e =
            EvolutionError::InvalidStatus("proposal not eligible for release (status=pending_eval)".to_string());
        let msg = format!("{e}");
        assert!(msg.contains("eligible"));
        assert!(msg.contains("pending_eval"));
    }

    /// #155(P1)：audit 构造器把入参原样落到不可变审计行；release / rollback 各自
    /// 在同一 transaction 内 insert 本 struct（不再 commit 后 best-effort）。
    #[test]
    fn build_threshold_override_audit_carries_all_fields() {
        let pid = ObjectId::new();
        let metrics = doc! { "p_value": 0.01_f64 };
        let audit = build_threshold_override_audit(
            "ws-1",
            "acct-1",
            "pressure_risk_block",
            "released",
            Some(7.0),
            Some(6.5),
            pid,
            "admin@x",
            Some(0.12),
            Some(metrics.clone()),
        );
        assert_eq!(audit.workspace_id, "ws-1");
        assert_eq!(audit.account_id, "acct-1");
        assert_eq!(audit.gate_key, "pressure_risk_block");
        assert_eq!(audit.action, "released");
        assert_eq!(audit.previous_value, Some(7.0));
        assert_eq!(audit.new_value, Some(6.5));
        assert_eq!(audit.source_proposal_id, pid);
        assert_eq!(audit.decided_by, "admin@x");
        assert_eq!(audit.hit_rate_observed, Some(0.12));
        assert_eq!(audit.significance_metrics, Some(metrics));
        assert!(audit.id.is_none());
    }

    #[test]
    fn build_threshold_override_audit_rollback_leaves_new_value_none() {
        let audit = build_threshold_override_audit(
            "ws-1",
            "acct-1",
            "fact_risk_block",
            "rolled_back",
            Some(5.5),
            None,
            ObjectId::new(),
            "admin@x",
            None,
            None,
        );
        assert_eq!(audit.action, "rolled_back");
        assert_eq!(audit.previous_value, Some(5.5));
        assert!(audit.new_value.is_none());
    }
}
