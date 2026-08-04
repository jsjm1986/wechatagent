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
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::{FindOneOptions, TransactionOptions},
    ClientSession,
};

use crate::routes::AppState;

use super::error::EvolutionError;
use super::revision::{
    content_sha256, parse_prompt_revision, parse_threshold_revision, prompt_revision,
    threshold_revision,
};

async fn ensure_release_gate_open(
    state: &AppState,
    workspace_id: &str,
) -> Result<(), EvolutionError> {
    if !state.config.evolution_enabled {
        return Err(EvolutionError::InvalidStatus(
            "evolution release disabled by EVOLUTION_ENABLED".to_string(),
        ));
    }
    let flag = super::runtime_flag::load_runtime_flag(state, workspace_id)
        .await
        .map_err(|e| EvolutionError::Internal(format!("load evolution runtime flag: {e}")))?;
    if !flag.map(|value| value.enabled).unwrap_or(false) {
        return Err(EvolutionError::InvalidStatus(format!(
            "evolution release disabled for workspace={workspace_id}"
        )));
    }
    Ok(())
}

fn release_event_document(
    kind: &str,
    workspace_id: &str,
    account_id: &str,
    proposal_id: ObjectId,
    admin: &str,
    extra: Option<Document>,
    created_at: DateTime,
) -> Document {
    let mut details = doc! {
        "proposal_id": proposal_id,
        "released_by": admin,
    };
    if let Some(extra) = extra {
        for (key, value) in extra {
            details.insert(key, value);
        }
    }
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_wxid": null,
        "kind": kind,
        "status": "ok",
        "summary": format!("evolution release: {kind} by {admin} for proposal {proposal_id}"),
        "details": details,
        "created_at": created_at,
        "dedupe_key": format!("evolution:{kind}:{proposal_id}"),
    }
}

async fn insert_release_observability_with_session(
    state: &AppState,
    session: &mut ClientSession,
    kind: &str,
    workspace_id: &str,
    account_id: &str,
    proposal_id: ObjectId,
    proposal_kind: &str,
    admin: &str,
    extra: Option<Document>,
    now: DateTime,
) -> Result<(), EvolutionError> {
    state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .insert_one_with_session(
            release_event_document(
                kind,
                workspace_id,
                account_id,
                proposal_id,
                admin,
                extra,
                now,
            ),
            None,
            session,
        )
        .await
        .map_err(EvolutionError::from)?;
    state
        .db
        .raw()
        .collection::<Document>("post_release_reviews")
        .insert_one_with_session(
            super::post_release::post_release_review_document(
                proposal_id,
                workspace_id,
                account_id,
                proposal_kind,
                now,
            ),
            None,
            session,
        )
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

async fn insert_event_with_session(
    state: &AppState,
    session: &mut ClientSession,
    kind: &str,
    workspace_id: &str,
    account_id: &str,
    proposal_id: ObjectId,
    admin: &str,
    extra: Option<Document>,
    now: DateTime,
) -> Result<(), EvolutionError> {
    state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .insert_one_with_session(
            release_event_document(
                kind,
                workspace_id,
                account_id,
                proposal_id,
                admin,
                extra,
                now,
            ),
            None,
            session,
        )
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

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
    workspace_id: &str,
    account_id: &str,
    admin: &str,
) -> Result<(), EvolutionError> {
    ensure_release_gate_open(state, workspace_id).await?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}"))
        })?;

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
    if !crate::agent::runtime::threshold_value_is_representable(&gate_key, proposed_value) {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold proposal value is not representable: {gate_key}={proposed_value}"
        )));
    }
    let base_revision = proposal.base_revision.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal missing base_revision: {proposal_id}"
        ))
    })?;
    let parsed_base = parse_threshold_revision(&base_revision).ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal has invalid base_revision: {proposal_id}"
        ))
    })?;
    if proposal.current_value.map(|value| value.to_bits()) != Some(parsed_base.value.to_bits()) {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold proposal current_value does not match base_revision: {proposal_id}"
        )));
    }
    if !crate::agent::runtime::threshold_value_is_representable(&gate_key, parsed_base.value) {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold base value is not representable: {gate_key}={} ",
            parsed_base.value
        )));
    }

    let now = DateTime::now();
    let override_id = ObjectId::new();
    let released_revision = threshold_revision(Some(override_id), proposed_value);
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

    let overrides = state.db.raw().collection::<Document>("threshold_overrides");

    let cooldown_hours = state
        .config
        .evolution_threshold_release_cooldown_hours
        .max(1) as i64;
    let cooldown_since = DateTime::from_millis(
        now.timestamp_millis()
            .saturating_sub(cooldown_hours * 60 * 60 * 1000),
    );
    let recent_release_count = overrides
        .count_documents_with_session(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "gate_key": &gate_key,
                "released_at": { "$gte": cooldown_since },
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if recent_release_count != 0 {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold release cooldown active: {workspace_id}/{account_id}/{gate_key}"
        )));
    }

    match parsed_base.source_id {
        Some(base_id) => {
            let retired = overrides
                .update_one_with_session(
                    doc! {
                        "_id": base_id,
                        "workspace_id": workspace_id,
                        "account_id": account_id,
                        "gate_key": &gate_key,
                        "value": parsed_base.value,
                        "current_version": true,
                        "rolled_back_at": null,
                    },
                    doc! { "$set": { "current_version": false } },
                    None,
                    &mut session,
                )
                .await
                .map_err(EvolutionError::from)?;
            if retired.matched_count != 1 {
                return Err(EvolutionError::InvalidStatus(format!(
                    "threshold base revision changed before release: {workspace_id}/{account_id}/{gate_key}"
                )));
            }
        }
        None => {
            let current_count = overrides
                .count_documents_with_session(
                    doc! {
                        "workspace_id": workspace_id,
                        "account_id": account_id,
                        "gate_key": &gate_key,
                        "current_version": true,
                        "rolled_back_at": null,
                    },
                    None,
                    &mut session,
                )
                .await
                .map_err(EvolutionError::from)?;
            if current_count != 0 {
                return Err(EvolutionError::InvalidStatus(format!(
                    "threshold baseline changed before release: {workspace_id}/{account_id}/{gate_key}"
                )));
            }
        }
    }

    let override_doc = doc! {
        "_id": override_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
        "gate_key": &gate_key,
        "value": proposed_value,
        "source_proposal_id": proposal_id,
        "base_revision": &base_revision,
        "released_revision": &released_revision,
        "current_version": true,
        "released_at": now,
        "released_by": admin,
        "rolled_back_at": null,
        "rolled_back_by": null,
    };
    overrides
        .insert_one_with_session(override_doc, None, &mut session)
        .await
        .map_err(EvolutionError::from)?;

    let proposal_update = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
                "status": "eligible_for_release",
                "base_revision": &base_revision,
            },
            doc! {
                "$set": {
                    "status": "released",
                    "released_at": now,
                    "released_by": admin,
                    "released_revision": &released_revision,
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if proposal_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold proposal scope/status changed before release: {workspace_id}/{account_id}/{proposal_id}"
        )));
    }

    // #155(P1)：audit 行与 override / proposal 推进写在同一 transaction，commit 前
    // 完成。旧实现 commit 后才 best-effort 写 + 仅 warn，阈值变更可能无审计行生效。
    let release_audit = build_threshold_override_audit(
        workspace_id,
        account_id,
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

    insert_release_observability_with_session(
        state,
        &mut session,
        "evolution_threshold_released",
        workspace_id,
        account_id,
        proposal_id,
        "threshold",
        admin,
        Some(doc! {
            "gate_key": &gate_key,
            "proposed_value": proposed_value,
            "current_value": parsed_base.value,
            "base_revision": &base_revision,
            "released_revision": &released_revision,
        }),
        now,
    )
    .await?;

    commit_with_session(&mut session).await?;

    Ok(())
}

/// 把 status="eligible_for_release" 的 prompt proposal 落地到 `prompt_templates`。
///
/// 写入路径（mongo transaction）：
/// 1. 重新加载 proposal，校验 `proposal_kind="prompt"` + `status="eligible_for_release"`
/// 2. 加载 `(workspace_id, prompt_key, current_version=true)` 那条；不存在则
///    `InvalidStatus`（不应当发生：seed 总会保证有 current）
/// 3. 把旧 current 置 `current_version=false`
/// 3.5. 合成 new_content = compose_appended_content(current.content, diff_snippet)
///      （末尾追加,原红线正文逐字保留）→ 过 validate_prompt_edit（禁词+锚点闸）
///      + review_prompt_edit（LLM 语义闸）；任一拒则 RedlineGateRejected,不写库
/// 4. insert 新一条 `version = old.version + 1`、`current_version=true`、
///    `previous_version = Some(old.version)`、`seeded_by="evolution_release"`、
///    `content` = new_content（原文 + 追加片段）
/// 5. update proposals: `status="released"`、`released_at`、`released_by`、
///    `previous_prompt_version = old.version.to_string()`
/// 6. commit 后 `state.prompt_pack_version.fetch_add(1, SeqCst)` 让 LRU cache 立即失效
/// 7. 写一条 `agent_events kind="evolution_prompt_released"`
pub async fn release_prompt(
    state: &AppState,
    proposal_id: ObjectId,
    workspace_id: &str,
    account_id: &str,
    admin: &str,
) -> Result<(), EvolutionError> {
    ensure_release_gate_open(state, workspace_id).await?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}"))
        })?;

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
    let append_snippet = proposal.diff_snippet.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing diff_snippet: {proposal_id}"
        ))
    })?;
    let base_revision = proposal.base_revision.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing base_revision: {proposal_id}"
        ))
    })?;
    let parsed_base = parse_prompt_revision(&base_revision).ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal has invalid base_revision: {proposal_id}"
        ))
    })?;

    let current = state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "_id": parsed_base.template_id,
                "workspace_id": workspace_id,
                "prompt_key": &prompt_key,
                "version": parsed_base.version,
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
    if content_sha256(&current.content) != parsed_base.content_sha256 {
        return Err(EvolutionError::InvalidStatus(format!(
            "prompt base content changed before release: workspace={workspace_id} key={prompt_key}"
        )));
    }

    // ── 红线三闸（与管理员手动编辑路径同源,从 prompt_guard 复用）──
    // 末尾追加:原 prompt 正文逐字保留,critic 片段追加到末尾。
    let new_content =
        crate::prompt_guard::compose_appended_content(&current.content, &append_snippet);
    // 闸 1+2:禁词 + 锚点完整性（原文保留 → 锚点天然过;不过说明原 prompt 已缺锚,fail-closed 正确）
    crate::prompt_guard::validate_prompt_edit(&prompt_key, &new_content)
        .map_err(EvolutionError::RedlineGateRejected)?;
    // 闸 3:LLM 语义审查追加增量（变相真人转介/削弱 grounding 等语义绕过）
    match crate::prompt_guard::review_prompt_edit(
        state,
        &workspace_id,
        &prompt_key,
        &current.content,
        &new_content,
    )
    .await
    {
        crate::prompt_guard::PromptEditVerdict::Pass => {}
        crate::prompt_guard::PromptEditVerdict::Reject(reason) => {
            return Err(EvolutionError::RedlineGateRejected(format!(
                "LLM 语义闸拒绝:{reason}"
            )));
        }
        crate::prompt_guard::PromptEditVerdict::NeedsHumanConfirm { reason, .. } => {
            // LLM 不可用 → 不 fail-open 放水,不 fail-closed 死路:本次 release 中止,
            // 要求管理员逐字核对后再确认（具体 UI 交互见阶段三）。
            return Err(EvolutionError::RedlineGateRejected(format!(
                "红线语义审查暂不可用,请逐字核对后再发布:{reason}"
            )));
        }
    }

    let old_version = current.version;
    let new_version = old_version + 1;
    let now = DateTime::now();
    let new_template_id = ObjectId::new();
    let released_revision = prompt_revision(new_template_id, new_version, &new_content);

    let client = state.db.client();
    let mut session = client
        .start_session(None)
        .await
        .map_err(EvolutionError::from)?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await
        .map_err(EvolutionError::from)?;

    let current_update = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .update_one_with_session(
            doc! {
                "_id": parsed_base.template_id,
                "workspace_id": workspace_id,
                "prompt_key": &prompt_key,
                "version": parsed_base.version,
                "content": &current.content,
                "current_version": true,
            },
            doc! {
                "$set": {
                    "current_version": false,
                    "status": "archived",
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if current_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "current prompt changed before release: workspace={workspace_id} key={prompt_key}"
        )));
    }

    let new_template = doc! {
        "_id": new_template_id,
        "workspace_id": workspace_id,
        "prompt_key": &prompt_key,
        "agent_kind": &current.agent_kind,
        "layer": &current.layer,
        "title": &current.title,
        "description": current.description.clone().unwrap_or_default(),
        "content": &new_content,
        "status": "active",
        "version": new_version,
        "prompt_pack_version": &current.prompt_pack_version,
        "created_by": admin,
        "created_at": now,
        "updated_at": now,
        "current_version": true,
        "previous_version": old_version,
        "seeded_by": "evolution_release",
        "locale": current.locale.clone(),
        "source_proposal_id": proposal_id,
    };
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates")
        .insert_one_with_session(new_template, None, &mut session)
        .await
        .map_err(EvolutionError::from)?;

    let proposal_update = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
                "status": "eligible_for_release",
                "base_revision": &base_revision,
            },
            doc! {
                "$set": {
                    "status": "released",
                    "released_at": now,
                    "released_by": admin,
                    "released_revision": &released_revision,
                    "previous_prompt_version": old_version.to_string(),
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if proposal_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "prompt proposal scope/status changed before release: {workspace_id}/{account_id}/{proposal_id}"
        )));
    }

    insert_release_observability_with_session(
        state,
        &mut session,
        "evolution_prompt_released",
        workspace_id,
        account_id,
        proposal_id,
        "prompt",
        admin,
        Some(doc! {
            "prompt_key": &prompt_key,
            "old_version": old_version,
            "new_version": new_version,
            "section": proposal.proposed_section.clone().unwrap_or_default(),
            "base_revision": &base_revision,
            "released_revision": &released_revision,
        }),
        now,
    )
    .await?;

    commit_with_session(&mut session).await?;

    // commit 后再 bump cache version——commit 失败时 cache 不会被错误地标脏。
    state.prompt_pack_version.fetch_add(1, Ordering::SeqCst);

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
    workspace_id: &str,
    account_id: &str,
    admin: &str,
) -> Result<(), EvolutionError> {
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}"))
        })?;

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
    let gate_key = proposal.gate_key.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal missing gate_key: {proposal_id}"
        ))
    })?;
    let released_revision = proposal.released_revision.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal missing released_revision: {proposal_id}"
        ))
    })?;
    let released = parse_threshold_revision(&released_revision).ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal has invalid released_revision: {proposal_id}"
        ))
    })?;
    let released_id = released.source_id.ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold released_revision has no artifact id: {proposal_id}"
        ))
    })?;
    let base_revision = proposal.base_revision.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal missing base_revision: {proposal_id}"
        ))
    })?;
    let base = parse_threshold_revision(&base_revision).ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "threshold proposal has invalid base_revision: {proposal_id}"
        ))
    })?;

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

    let override_update = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("threshold_overrides")
        .update_one_with_session(
            doc! {
                "_id": released_id,
                "source_proposal_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
                "gate_key": &gate_key,
                "value": released.value,
                "released_revision": &released_revision,
                "current_version": true,
                "rolled_back_at": null,
            },
            doc! {
                "$set": {
                    "current_version": false,
                    "rolled_back_at": now,
                    "rolled_back_by": admin,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if override_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold artifact is no longer current or owned by proposal: {workspace_id}/{account_id}/{proposal_id}"
        )));
    }

    if let Some(base_id) = base.source_id {
        let restored = state
            .db
            .raw()
            .collection::<Document>("threshold_overrides")
            .update_one_with_session(
                doc! {
                    "_id": base_id,
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "gate_key": &gate_key,
                    "value": base.value,
                    "released_revision": &base_revision,
                    "current_version": false,
                    "rolled_back_at": null,
                },
                doc! { "$set": { "current_version": true } },
                None,
                &mut session,
            )
            .await
            .map_err(EvolutionError::from)?;
        if restored.matched_count != 1 {
            return Err(EvolutionError::InvalidStatus(format!(
                "threshold rollback predecessor unavailable: {workspace_id}/{account_id}/{gate_key}"
            )));
        }
    }

    let proposal_update = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
                "status": "released",
                "released_revision": &released_revision,
            },
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
    if proposal_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "threshold proposal scope/status changed before rollback: {workspace_id}/{account_id}/{proposal_id}"
        )));
    }

    // #155(P1)：rollback 的 audit 行也写进同一 transaction，commit 前完成。
    // previous = 被回滚的 proposed_value；new_value 留 None（回滚后回到 baseline 或
    // 更早 override，由审计读路径自行还原）。
    let rollback_audit = build_threshold_override_audit(
        workspace_id,
        account_id,
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

    insert_event_with_session(
        state,
        &mut session,
        "evolution_rollback_completed",
        workspace_id,
        account_id,
        proposal_id,
        admin,
        Some(doc! {
            "kind": "threshold",
            "gate_key": &gate_key,
            "rolled_back_revision": &released_revision,
            "restored_revision": &base_revision,
        }),
        now,
    )
    .await?;

    commit_with_session(&mut session).await?;

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
    workspace_id: &str,
    account_id: &str,
    admin: &str,
) -> Result<(), EvolutionError> {
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!("proposal not found: {proposal_id}"))
        })?;

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
    let base_revision = proposal.base_revision.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing base_revision; legacy rollback rejected: {proposal_id}"
        ))
    })?;
    let parsed_base = parse_prompt_revision(&base_revision).ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal has invalid base_revision: {proposal_id}"
        ))
    })?;
    let released_revision = proposal.released_revision.clone().ok_or_else(|| {
        EvolutionError::InvalidStatus(format!(
            "prompt proposal missing released_revision; legacy rollback rejected: {proposal_id}"
        ))
    })?;

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

    let templates = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates");

    // 1. 只有该 proposal 仍拥有 current 产物且内容 revision 未漂移时，才能撤销。
    let current = templates
        .find_one_with_session(
            doc! {
                "workspace_id": workspace_id,
                "prompt_key": &prompt_key,
                "current_version": true,
                "source_proposal_id": proposal_id,
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!(
                "prompt rollback rejected because proposal artifact is no longer current: {workspace_id}/{prompt_key}/{proposal_id}"
            ))
        })?;
    let current_id = current.get_object_id("_id").map_err(|_| {
        EvolutionError::InvalidStatus(format!(
            "current prompt artifact missing _id: {proposal_id}"
        ))
    })?;
    let current_version = current.get_i32("version").map_err(|_| {
        EvolutionError::InvalidStatus(format!(
            "current prompt artifact missing version: {proposal_id}"
        ))
    })?;
    let current_content = current.get_str("content").map_err(|_| {
        EvolutionError::InvalidStatus(format!(
            "current prompt artifact missing content: {proposal_id}"
        ))
    })?;
    if prompt_revision(current_id, current_version, current_content) != released_revision {
        return Err(EvolutionError::InvalidStatus(format!(
            "prompt released artifact revision changed before rollback: {proposal_id}"
        )));
    }

    // 2. 冻结基线历史行必须仍存在且 hash 一致。
    let restored_base = templates
        .find_one_with_session(
            doc! {
                "_id": parsed_base.template_id,
                "workspace_id": workspace_id,
                "prompt_key": &prompt_key,
                "version": parsed_base.version,
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?
        .ok_or_else(|| {
            EvolutionError::InvalidStatus(format!(
                "rollback target prompt baseline no longer exists: {proposal_id}"
            ))
        })?;
    let restored_content = restored_base.get_str("content").map_err(|_| {
        EvolutionError::InvalidStatus(format!(
            "rollback target prompt baseline missing content: {proposal_id}"
        ))
    })?;
    if content_sha256(restored_content) != parsed_base.content_sha256 {
        return Err(EvolutionError::InvalidStatus(format!(
            "rollback target prompt baseline content changed: {proposal_id}"
        )));
    }

    let current_update = templates
        .update_one_with_session(
            doc! {
                "_id": current_id,
                "current_version": true,
                "source_proposal_id": proposal_id,
            },
            doc! {
                "$set": {
                    "current_version": false,
                    "status": "archived",
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if current_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "current prompt changed before rollback: workspace={workspace_id} key={prompt_key}"
        )));
    }

    // 3. 只恢复 proposal 冻结的基线行，不按可碰撞的 version 猜目标。
    let restored = templates
        .update_one_with_session(
            doc! {
                "_id": parsed_base.template_id,
                "workspace_id": workspace_id,
                "prompt_key": &prompt_key,
                "version": parsed_base.version,
                "current_version": false,
            },
            doc! {
                "$set": {
                    "current_version": true,
                    "status": "active",
                    "updated_at": now,
                }
            },
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;
    if restored.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "rollback target prompt baseline changed before restore: {proposal_id}"
        )));
    }

    // 4. 推 proposal 到 rolled_back。
    let proposal_update = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .update_one_with_session(
            doc! {
                "_id": proposal_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
                "status": "released",
                "released_revision": &released_revision,
            },
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
    if proposal_update.matched_count != 1 {
        return Err(EvolutionError::InvalidStatus(format!(
            "prompt proposal scope/status changed before rollback: {workspace_id}/{account_id}/{proposal_id}"
        )));
    }

    state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .insert_one_with_session(
            release_event_document(
                "evolution_rollback_completed",
                workspace_id,
                account_id,
                proposal_id,
                admin,
                Some(doc! {
                    "kind": "prompt",
                    "prompt_key": &prompt_key,
                    "rolled_back_to_version": parsed_base.version,
                    "released_revision": &released_revision,
                    "restored_revision": &base_revision,
                }),
                now,
            ),
            None,
            &mut session,
        )
        .await
        .map_err(EvolutionError::from)?;

    commit_with_session(&mut session).await?;

    state.prompt_pack_version.fetch_add(1, Ordering::SeqCst);

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
        let e = EvolutionError::InvalidStatus(
            "proposal not eligible for release (status=pending_eval)".to_string(),
        );
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
