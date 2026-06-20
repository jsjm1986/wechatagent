//! 决策请示通道 admin REST 端点：列表 / admin 直接裁决 / 改派。
//! admin 在此是"幕后决策人"（真人决策），客户仍只收 AI 口吻转述（relay 下游不变）。

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::escalation::{
    enqueue_relay_task, list_escalations_by_workspace, reassign_escalation, resolve_ask_human_policy,
    resolve_escalation, sanitize_verdict,
};
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::models::PrincipalDecision;
use super::AppState;
use mongodb::bson::DateTime;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// GET /api/admin/principal-escalations?status=pending|resolved
pub async fn list_principal_escalations(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let status = q.status.as_deref().unwrap_or("pending");
    if status != "pending" && status != "resolved" {
        return Err(AppError::BadRequest("status 只能是 pending|resolved".into()));
    }
    let items = list_escalations_by_workspace(&state, &admin.current_workspace, status).await?;
    let now = DateTime::now().timestamp_millis();
    let json_items: Vec<Value> = items
        .iter()
        .map(|e| {
            let age_hours =
                (now - e.created_at.timestamp_millis()) as f64 / (3600.0 * 1000.0);
            json!({
                "shortCode": e.short_code,
                "contactWxid": e.contact_wxid,
                "category": e.category,
                "reason": e.reason,
                "questionForPrincipal": e.question_for_principal,
                "principalWxid": e.principal_wxid,
                "status": e.status,
                "ageHours": age_hours,
                "createdAt": e.created_at,
                "decision": e.decision,
                "authorizationExpiresAt": e.authorization_expires_at,
                "resolvedVia": e.resolved_via,
            })
        })
        .collect();
    Ok(Json(json!({ "items": json_items })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveBody {
    pub verdict: String,
    #[serde(default)]
    pub substance: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub authorization_window_hours: Option<f64>,
}

/// POST /api/admin/principal-escalations/:short_code/resolve
/// admin 结构化裁决 → 复用 relay 下游（跳过 LLM interpret）。
pub async fn resolve_principal_escalation(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(short_code): Path<String>,
    Json(body): Json<ResolveBody>,
) -> AppResult<Json<Value>> {
    // 先确认该条属于本 workspace 且 pending（IDOR + 幂等）。
    let pending = list_escalations_by_workspace(&state, &admin.current_workspace, "pending").await?;
    let Some(entry) = pending.into_iter().find(|e| e.short_code == short_code) else {
        // 不在本 workspace pending 列表：可能已 resolved（幂等）或越权 → 幂等成功避免泄漏存在性。
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
    };
    let decision = sanitize_verdict(PrincipalDecision {
        verdict: body.verdict,
        substance: body.substance,
        constraints: body.constraints,
        authorization_window_hours: body.authorization_window_hours,
    });
    let expires = decision.authorization_window_hours.and_then(|hours| {
        if hours > 0.0 {
            Some(DateTime::from_millis(
                DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64,
            ))
        } else {
            None
        }
    });
    let resolved = resolve_escalation(&state, &short_code, &decision, expires, "admin").await?;
    if resolved.is_none() {
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
    }
    // deferred 不转述（领导/admin 暂缓）；其余起 relay task 用 AI 口吻转述客户。
    if decision.verdict != crate::models::PRINCIPAL_VERDICT_DEFERRED {
        enqueue_relay_task(&state, &entry).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReassignBody {
    pub to_wxid: String,
}

/// POST /api/admin/principal-escalations/:short_code/reassign
pub async fn reassign_principal_escalation(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(short_code): Path<String>,
    Json(body): Json<ReassignBody>,
) -> AppResult<Json<Value>> {
    // 校验 to_wxid 在 decider_chain 内（取 current_version config 解析）。
    let cfg = state
        .db
        .operation_domain_configs()
        .find_one(
            mongodb::bson::doc! {
                "workspace_id": &admin.current_workspace,
                "domain": "user_operations",
                "current_version": true,
            },
            None,
        )
        .await?;
    let in_chain = cfg
        .as_ref()
        .map(|c| {
            let p = resolve_ask_human_policy(c);
            p.decider_chain.iter().any(|d| d.wxid == body.to_wxid)
        })
        .unwrap_or(false);
    if !in_chain {
        return Err(AppError::BadRequest(
            "to_wxid 不在该 workspace 的决策人链内".into(),
        ));
    }
    let updated =
        reassign_escalation(&state, &admin.current_workspace, &short_code, &body.to_wxid).await?;
    if updated.is_none() {
        return Err(AppError::NotFound("无此 pending 请示或已处置".into()));
    }
    Ok(Json(json!({ "ok": true })))
}
