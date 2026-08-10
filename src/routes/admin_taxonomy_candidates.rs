//! agent-autonomy-loop W3 / Task 4.8：双层标签候选审核路由（admin）。
//!
//! 职责：列表 / approve / reject `taxonomy_candidates` 候选记录。
//!
//! - `GET /api/admin/taxonomy-candidates?status=pending&scope=&kind=`
//! - `POST /api/admin/taxonomy-candidates/:id/approve`
//!     body: `{ canonicalValue: { id, label, aliases? } }`
//!     行为：
//!       1. 读 candidate（必须 `status=pending`）。
//!       2. 写 `system_taxonomies`：新 canonical 新建版本；已有 current canonical
//!          则继承其运行字段，合并候选原值/请求 aliases，并追加 current 版本。
//!       3. 改 candidate `status="approved"`、`reviewed_at=now`。
//!       4. 同事务推进 workspace generation，提交后失效本进程 workspace cache。
//!     四步在同一 MongoDB transaction 中完成；并发 approve/reject 只有一个能从
//!     pending 状态推进，失败不会留下正式字典与候选状态不一致。
//! - `POST /api/admin/taxonomy-candidates/:id/reject`
//!     body: `{ reason }` —— 写入 candidate.reason 并 `status="rejected"`。
//!
//! 部署要求与 evolution release / Guide apply 一致：MongoDB 必须启用 replica set
//! transaction 支持。

use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};
use mongodb::options::{FindOneOptions, FindOptions, TransactionOptions};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent::taxonomy::invalidate_global_taxonomy_cache,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::{TaxonomyCandidate, TaxonomyEntry, TaxonomyValue},
};

use super::admin_taxonomies::authorize_taxonomy_scope;
use super::shared::*;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListCandidatesQuery {
    /// 默认只看 `pending`；前端可显式传 `approved` / `rejected` / `all` 看历史。
    status: Option<String>,
    scope: Option<String>,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApproveCandidateRequest {
    canonical_value: ApproveCanonicalValue,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApproveCanonicalValue {
    id: String,
    #[serde(alias = "displayName")]
    label: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RejectCandidateRequest {
    reason: String,
}

pub(super) async fn list_taxonomy_candidates(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListCandidatesQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("pending");
    if status != "all" {
        filter.insert("status", status);
    }
    if let Some(scope) = query.scope.as_ref().filter(|s| !s.trim().is_empty()) {
        authorize_taxonomy_scope(&state, &admin.current_workspace, scope.trim()).await?;
        filter.insert("scope", scope.trim());
    }
    if let Some(kind) = query.kind.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("kind", kind.trim());
    }

    let mut cursor = state
        .db
        .collection_taxonomy_candidates()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "last_seen_at": -1 })
                .limit(500)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(taxonomy_candidate_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn approve_taxonomy_candidate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ApproveCandidateRequest>,
) -> Result<Response, AppError> {
    let object_id = parse_object_id(&id)?;
    let actor = ReviewActor::from_admin(&admin)?;
    let outcome = approve_candidate_transaction(
        &state,
        &admin.current_workspace,
        object_id,
        None,
        &payload,
        &actor,
    )
    .await?;
    Ok(Json(json!({
        "item": taxonomy_candidate_json(outcome.candidate),
        "mergedIntoExisting": outcome.merged_into_existing,
    }))
    .into_response())
}

pub(super) async fn reject_taxonomy_candidate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<RejectCandidateRequest>,
) -> AppResult<Json<Value>> {
    if payload.reason.trim().is_empty() {
        return Err(AppError::BadRequest("reason 不能为空".to_string()));
    }
    let object_id = parse_object_id(&id)?;
    let candidates = state.db.collection_taxonomy_candidates();
    let actor = ReviewActor::from_admin(&admin)?;
    let now = DateTime::now();
    let result = candidates
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace,
                "status": "pending"
            },
            doc! {
                "$set": {
                    "status": "rejected",
                    "reviewed_at": now,
                    "reviewed_by": actor.as_str(),
                    // candidate 模型暂未声明 reason 字段（W0 占位）；以
                    // dynamic field 写入 BSON，仍然可被 mongo shell / UI 看到。
                    "rejection_reason": payload.reason.trim()
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "candidate not found or not pending".to_string(),
        ));
    }
    let updated = candidates
        .find_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("candidate not found".to_string()))?;
    Ok(Json(json!({ "item": taxonomy_candidate_json(updated) })))
}

/// 候选 scope 是否允许指定 account 审批（管理 Agent 工具侧 scope 隔离）。
/// TaxonomyCandidate/TaxonomyEntry 无 workspace_id 字段，隔离边界是 `scope`：
/// `"global"` 全局候选任何账号可审；否则 scope 必须等于发起者 account_id
/// （账号级候选只能本账号审）。纯函数便于单测覆盖三种放行/拒绝路径。
pub(in crate::routes) fn taxonomy_scope_allows(candidate_scope: &str, account_id: &str) -> bool {
    candidate_scope == "global" || candidate_scope == account_id
}

/// approve_taxonomy_candidate 的管理 Agent 工具侧入口：在 REST 版本基础上**新增
/// scope 校验**（管理者只能 approve global 或自己 account_id 的候选）。校验通过后
/// 复用与 REST handler 完全相同的「新建 canonical 或并入已有 canonical alias →
/// mark approved → 刷新缓存」事务，返回结构化 candidate 与
/// `mergedIntoExisting`，已存在 canonical 是成功合并而不是冲突。
///
/// 注：REST handler（无 Extension、无 account 来源）保持原样不加 scope 校验，
/// 维持现状不回归——scope 校验只在工具侧（有可信 account_id）施加。
pub(in crate::routes) async fn approve_taxonomy_candidate_inner(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    id: &str,
    payload: ApproveCandidateRequest,
    actor: ReviewActor,
) -> AppResult<Value> {
    let object_id = parse_object_id(id)?;
    let outcome = approve_candidate_transaction(
        state,
        workspace_id,
        object_id,
        Some(account_id),
        &payload,
        &actor,
    )
    .await?;
    Ok(json!({
        "item": taxonomy_candidate_json(outcome.candidate),
        "mergedIntoExisting": outcome.merged_into_existing,
    }))
}

struct ApproveOutcome {
    candidate: TaxonomyCandidate,
    merged_into_existing: bool,
}

async fn approve_candidate_transaction(
    state: &AppState,
    workspace_id: &str,
    candidate_id: mongodb::bson::oid::ObjectId,
    authorized_account_id: Option<&str>,
    payload: &ApproveCandidateRequest,
    actor: &ReviewActor,
) -> AppResult<ApproveOutcome> {
    if payload.canonical_value.id.trim().is_empty()
        || payload.canonical_value.label.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "canonicalValue.id / canonicalValue.label 不能为空".to_string(),
        ));
    }

    let candidates = state.db.collection_taxonomy_candidates();
    let client = state.db.client();
    let mut session = client.start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;

    let transaction_result: AppResult<(TaxonomyCandidate, bool)> = async {
        let candidate = candidates
            .find_one_with_session(
                doc! { "_id": candidate_id, "workspace_id": workspace_id },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::NotFound("candidate not found".to_string()))?;
        if let Some(account_id) = authorized_account_id {
            if !taxonomy_scope_allows(&candidate.scope, account_id) {
                return Err(AppError::NotFound("candidate not found".to_string()));
            }
        }
        if candidate.status != "pending" {
            return Err(AppError::Conflict(format!(
                "taxonomy_candidate_not_pending:{}",
                candidate.status
            )));
        }

        let now = DateTime::now();
        let claimed = candidates
            .update_one_with_session(
                doc! {
                    "_id": candidate_id,
                    "workspace_id": workspace_id,
                    "status": "pending",
                },
                doc! { "$set": { "status": "approving" } },
                None,
                &mut session,
            )
            .await?;
        if claimed.modified_count != 1 {
            return Err(AppError::Conflict(
                "taxonomy_candidate_claim_conflict".to_string(),
            ));
        }

        let canonical_id = payload.canonical_value.id.trim();
        let requested_aliases = normalized_aliases(
            &[],
            &payload.canonical_value.aliases,
            &candidate.raw_value,
            canonical_id,
        );
        let taxonomies = state.db.collection_system_taxonomies();
        let taxonomy_scope = doc! {
            "workspace_id": workspace_id,
            "scope": &candidate.scope,
            "kind": &candidate.kind,
            "value.id": canonical_id,
        };
        let mut current_cursor = taxonomies
            .find_with_session(
                doc! {
                    "workspace_id": workspace_id,
                    "scope": &candidate.scope,
                    "kind": &candidate.kind,
                    "value.id": canonical_id,
                    "current_version": true,
                },
                FindOptions::builder().limit(2).build(),
                &mut session,
            )
            .await?;
        let mut current_rows = Vec::with_capacity(2);
        while let Some(row) = current_cursor.next(&mut session).await.transpose()? {
            current_rows.push(row);
        }
        if current_rows.len() > 1 {
            return Err(AppError::Conflict(
                "taxonomy_multiple_current_versions".to_string(),
            ));
        }
        let current = current_rows.pop();
        let latest = taxonomies
            .find_one_with_session(
                taxonomy_scope,
                FindOneOptions::builder()
                    .sort(doc! { "version": -1_i32 })
                    .build(),
                &mut session,
            )
            .await?;
        let merged_into_existing = current.is_some();
        let entry = if let Some(current) = current {
            let aliases =
                normalized_aliases(&current.value.aliases, &requested_aliases, "", canonical_id);
            if aliases == current.value.aliases {
                None
            } else {
                let next_version = next_taxonomy_version(latest.as_ref())?;
                let previous_version = current.version;
                let current_id = current.id.ok_or_else(|| {
                    AppError::External("current taxonomy missing _id".to_string())
                })?;
                let retired = taxonomies
                    .update_one_with_session(
                        doc! {
                            "_id": current_id,
                            "workspace_id": workspace_id,
                            "version": current.version,
                            "current_version": true,
                        },
                        doc! { "$set": { "current_version": false } },
                        None,
                        &mut session,
                    )
                    .await?;
                if retired.modified_count != 1 {
                    return Err(AppError::Conflict(
                        "taxonomy_current_version_changed".to_string(),
                    ));
                }
                let mut next = current;
                next.id = None;
                next.value.aliases = aliases;
                next.updated_at = now;
                next.version = next_version;
                next.current_version = true;
                next.previous_version = Some(previous_version);
                next.seeded_by = Some("manual".to_string());
                Some(next)
            }
        } else {
            let next_version = next_taxonomy_version(latest.as_ref())?;
            Some(TaxonomyEntry {
                id: None,
                workspace_id: candidate.workspace_id.clone(),
                scope: candidate.scope.clone(),
                kind: candidate.kind.clone(),
                value: TaxonomyValue {
                    id: canonical_id.to_string(),
                    display_name: payload.canonical_value.label.trim().to_string(),
                    description: payload
                        .canonical_value
                        .description
                        .clone()
                        .unwrap_or_else(|| candidate.evidence.clone().unwrap_or_default()),
                    aliases: requested_aliases,
                    status: "active".to_string(),
                    priority_weight: None,
                    is_terminal: false,
                    is_reactivation_target: false,
                },
                updated_at: now,
                version: next_version,
                current_version: true,
                previous_version: latest.as_ref().map(|entry| entry.version),
                seeded_by: Some("manual".to_string()),
            })
        };
        let taxonomy_changed = entry.is_some();
        if let Some(entry) = entry {
            taxonomies
                .insert_one_with_session(&entry, None, &mut session)
                .await?;
        }
        if taxonomy_changed {
            crate::db::config_generation::bump_generation_with_session(
                &state.db,
                crate::db::config_generation::TAXONOMY_NAMESPACE,
                workspace_id,
                &mut session,
            )
            .await?;
        }

        let approved = candidates
            .update_one_with_session(
                doc! {
                    "_id": candidate_id,
                    "workspace_id": workspace_id,
                    "status": "approving",
                },
                doc! {
                    "$set": {
                        "status": "approved",
                        "reviewed_at": now,
                        "reviewed_by": actor.as_str(),
                    }
                },
                None,
                &mut session,
            )
            .await?;
        if approved.modified_count != 1 {
            return Err(AppError::Conflict(
                "taxonomy_candidate_finalize_conflict".to_string(),
            ));
        }
        let mut updated = candidate;
        updated.status = "approved".to_string();
        updated.reviewed_at = Some(now);
        updated.reviewed_by = Some(actor.as_str().to_string());
        Ok((updated, merged_into_existing))
    }
    .await;

    let (candidate, merged_into_existing) = match transaction_result {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(match error {
                AppError::Db(ref db_error)
                    if super::admin_taxonomies::is_duplicate_key_error(db_error) =>
                {
                    AppError::Conflict("taxonomy_identity_claim_conflict".to_string())
                }
                other => other,
            });
        }
    };
    loop {
        match session.commit_transaction().await {
            Ok(()) => break,
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                return Err(if super::admin_taxonomies::is_duplicate_key_error(&error) {
                    AppError::Conflict("taxonomy_identity_claim_conflict".to_string())
                } else {
                    error.into()
                })
            }
        }
    }
    invalidate_global_taxonomy_cache(&state.db, workspace_id);
    Ok(ApproveOutcome {
        candidate,
        merged_into_existing,
    })
}

fn next_taxonomy_version(latest: Option<&TaxonomyEntry>) -> AppResult<i32> {
    match latest {
        Some(entry) => entry
            .version
            .checked_add(1)
            .ok_or_else(|| AppError::External("taxonomy version overflow".to_string())),
        None => Ok(1),
    }
}

fn normalized_aliases(
    existing: &[String],
    requested: &[String],
    raw_value: &str,
    canonical_id: &str,
) -> Vec<String> {
    let canonical_id = canonical_id.trim();
    let mut aliases = Vec::new();
    for value in existing
        .iter()
        .map(String::as_str)
        .chain(requested.iter().map(String::as_str))
        .chain(std::iter::once(raw_value))
    {
        let value = value.trim();
        if value.is_empty() || value == canonical_id || aliases.iter().any(|alias| alias == value) {
            continue;
        }
        aliases.push(value.to_string());
    }
    aliases
}

pub(super) fn taxonomy_candidate_json(item: TaxonomyCandidate) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "scope": item.scope,
        "kind": item.kind,
        "rawValue": item.raw_value,
        "evidence": item.evidence,
        "confidence": item.confidence,
        "occurrences": item.occurrences,
        "status": item.status,
        "firstSeenAt": crate::models::dt_to_string(item.first_seen_at),
        "lastSeenAt": crate::models::dt_to_string(item.last_seen_at),
        "reviewedAt": item.reviewed_at.and_then(crate::models::dt_to_string),
        "reviewedBy": item.reviewed_by,
        "suggestedDisplayName": item.suggested_display_name
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::oid::ObjectId;

    fn sample_candidate(status: &str) -> TaxonomyCandidate {
        TaxonomyCandidate {
            id: Some(ObjectId::new()),
            workspace_id: "default".to_string(),
            scope: "global".to_string(),
            kind: "objection_type".to_string(),
            raw_value: "太贵了".to_string(),
            evidence: Some("用户说价格高".to_string()),
            confidence: 7,
            first_seen_at: DateTime::now(),
            last_seen_at: DateTime::now(),
            occurrences: 3,
            status: status.to_string(),
            reviewed_at: None,
            reviewed_by: None,
            suggested_display_name: None,
        }
    }

    /// W3 / Task 4.8：候选 JSON 形状稳定。
    #[test]
    fn candidate_json_shape_is_stable() {
        let c = sample_candidate("pending");
        let id_hex = c.id.unwrap().to_hex();
        let value = taxonomy_candidate_json(c);
        assert_eq!(value["id"], id_hex);
        assert_eq!(value["scope"], "global");
        assert_eq!(value["kind"], "objection_type");
        assert_eq!(value["rawValue"], "太贵了");
        assert_eq!(value["status"], "pending");
        assert_eq!(value["confidence"], 7);
        assert_eq!(value["occurrences"], 3);
        assert!(value["firstSeenAt"].is_string());
        assert!(value["lastSeenAt"].is_string());
        assert!(value["reviewedAt"].is_null());
    }

    /// W3 / Task 4.8：默认 list query 不传 status 时 handler 内部解析为 "pending"。
    #[test]
    fn list_query_defaults_to_pending() {
        let q: ListCandidatesQuery = serde_json::from_value(json!({})).unwrap();
        let resolved = q
            .status
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("pending");
        assert_eq!(resolved, "pending");
    }

    /// W3 / Task 4.8：approve 请求支持 `displayName` 别名。
    #[test]
    fn approve_request_accepts_display_name_alias() {
        let req: ApproveCandidateRequest = serde_json::from_value(json!({
            "canonicalValue": {
                "id": "price_objection",
                "displayName": "价格异议"
            }
        }))
        .unwrap();
        assert_eq!(req.canonical_value.id, "price_objection");
        assert_eq!(req.canonical_value.label, "价格异议");
        assert!(req.canonical_value.aliases.is_empty());
    }

    #[test]
    fn alias_merge_is_trimmed_stable_and_excludes_canonical_id() {
        assert_eq!(
            normalized_aliases(
                &["existing".to_string(), " padded ".to_string()],
                &[
                    "existing".to_string(),
                    "new-alias".to_string(),
                    "canonical".to_string(),
                ],
                " raw-value ",
                "canonical",
            ),
            vec!["existing", "padded", "new-alias", "raw-value"]
        );
    }

    #[test]
    fn next_version_uses_history_max_and_rejects_overflow() {
        let mut latest = TaxonomyEntry {
            id: None,
            workspace_id: "ws".to_string(),
            scope: "global".to_string(),
            kind: "kind".to_string(),
            value: TaxonomyValue {
                id: "value".to_string(),
                display_name: "Value".to_string(),
                description: String::new(),
                aliases: Vec::new(),
                status: "active".to_string(),
                priority_weight: None,
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: DateTime::now(),
            version: 7,
            current_version: false,
            previous_version: None,
            seeded_by: None,
        };
        assert_eq!(next_taxonomy_version(Some(&latest)).unwrap(), 8);
        latest.version = i32::MAX;
        assert!(next_taxonomy_version(Some(&latest)).is_err());
    }

    /// W3 / Task 4.8：reject 请求要求 `reason` 字段（serde 默认 missing 报错）。
    #[test]
    fn reject_request_requires_reason() {
        let parsed: Result<RejectCandidateRequest, _> = serde_json::from_value(json!({}));
        assert!(parsed.is_err(), "缺少 reason 应该被 serde 拒绝");
        let ok: RejectCandidateRequest =
            serde_json::from_value(json!({ "reason": "无业务相关性" })).unwrap();
        assert_eq!(ok.reason, "无业务相关性");
    }

    /// 契约快照:taxonomy_candidate_json。TaxonomyCandidate 12 字段全量构造
    /// (evidence/reviewed_at/reviewed_by/suggested_display_name 四个 Option 给 Some);id→hex;
    /// first_seen_at/last_seen_at→RFC3339;reviewed_at→Some 后 RFC3339。投影下发 13 键。
    #[test]
    fn taxonomy_candidate_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let item = TaxonomyCandidate {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "default".to_string(),
            scope: "global".to_string(),
            kind: "objection_type".to_string(),
            raw_value: "太贵了".to_string(),
            evidence: Some("用户说价格高".to_string()),
            confidence: 7,
            first_seen_at: DateTime::from_millis(1_700_000_000_000),
            last_seen_at: DateTime::from_millis(1_700_000_100_000),
            occurrences: 3,
            status: "pending".to_string(),
            reviewed_at: Some(DateTime::from_millis(1_700_000_200_000)),
            reviewed_by: Some("admin-1".to_string()),
            suggested_display_name: Some("价格异议".to_string()),
        };
        let value = taxonomy_candidate_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("taxonomy_candidate", value);
    }
}
