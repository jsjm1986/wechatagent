//! 决策请示通道 admin REST 端点：列表 / admin 直接裁决 / 改派。
//! admin 在此是"幕后决策人"（真人决策），客户仍只收 AI 口吻转述（relay 下游不变）。

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::agent::escalation::{
    list_escalations_by_workspace, materialize_principal_card_delivery, reassign_escalation,
    resolve_escalation, sanitize_verdict,
};
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::models::PrincipalDecision;
use mongodb::bson::DateTime;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// GET /api/admin/principal-escalations?status=pending|resolved|delivery_failed
pub async fn list_principal_escalations(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let status = q.status.as_deref().unwrap_or("pending");
    if !crate::models::ALLOWED_PRINCIPAL_ESCALATION_STATUS.contains(&status) {
        return Err(AppError::BadRequest(
            "status 只能是 pending|resolved|delivery_failed".into(),
        ));
    }
    let items = list_escalations_by_workspace(&state, &admin.current_workspace, status).await?;
    let now = DateTime::now().timestamp_millis();
    let json_items: Vec<Value> = items
        .iter()
        .map(|e| {
            let age_hours = (now - e.created_at.timestamp_millis()) as f64 / (3600.0 * 1000.0);
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

fn default_resolve_exemption_type() -> String {
    crate::models::EXEMPTION_TYPE_NONE.to_string()
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
    /// 领导授权豁免类型（none/customer_only/knowledge）；admin 后台裁决由请求体决定，缺省 none。
    #[serde(default = "default_resolve_exemption_type")]
    pub exemption_type: String,
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
    let pending =
        list_escalations_by_workspace(&state, &admin.current_workspace, "pending").await?;
    let Some(entry) = pending.into_iter().find(|e| e.short_code == short_code) else {
        // 不在本 workspace pending 列表：可能已 resolved（幂等）或越权 → 幂等成功避免泄漏存在性。
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
    };
    let decision = sanitize_verdict(PrincipalDecision {
        verdict: body.verdict,
        substance: body.substance,
        constraints: body.constraints,
        authorization_window_hours: body.authorization_window_hours,
        exemption_type: body.exemption_type,
    });
    // deferred：领导/admin 暂缓 → 保持 pending 继续等待（与 wechat 路径 mod.rs 一致），不 resolve、不 relay。
    if decision.verdict == crate::models::PRINCIPAL_VERDICT_DEFERRED {
        return Ok(Json(json!({ "ok": true, "deferred": true })));
    }
    let expires = decision.authorization_window_hours.and_then(|hours| {
        if hours > 0.0 {
            Some(DateTime::from_millis(
                DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64,
            ))
        } else {
            None
        }
    });
    let resolved = resolve_escalation(&state, &entry, &decision, expires, "admin").await?;
    if resolved.is_none() {
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
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
    let pending =
        list_escalations_by_workspace(&state, &admin.current_workspace, "pending").await?;
    let Some(entry) = pending
        .into_iter()
        .find(|entry| entry.short_code == short_code)
    else {
        return Err(AppError::NotFound("无此 pending 请示或已处置".into()));
    };
    if body.to_wxid == entry.contact_wxid {
        return Err(AppError::BadRequest("决策人不能是请示客户本人".into()));
    }
    if body.to_wxid == entry.principal_wxid {
        return Err(AppError::BadRequest("目标决策人已是当前决策人".into()));
    }
    let protocol = entry
        .protocol
        .as_ref()
        .ok_or_else(|| AppError::Conflict("旧请示缺少冻结协议，不能自动改派".into()))?;
    let decider = protocol
        .policy
        .decider_chain
        .iter()
        .find(|decider| decider.wxid == body.to_wxid)
        .ok_or_else(|| AppError::BadRequest("to_wxid 不在该请示的冻结决策人链内".into()))?;
    let account_id = decider
        .account_id
        .as_deref()
        .filter(|account_id| !account_id.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("目标决策人未绑定发送账号".into()))?
        .to_string();
    let updated = reassign_escalation(
        &state,
        &admin.current_workspace,
        &short_code,
        &entry.principal_wxid,
        protocol.delivery_generation,
        &body.to_wxid,
        &account_id,
    )
    .await?
    .ok_or_else(|| AppError::Conflict("当前投递尚未终结或请示已被并发处置，不能改派".into()))?;
    // 即时尝试物化；若进程在此中断，worker 会按同一 generation 幂等补偿。
    materialize_principal_card_delivery(&state, &updated).await?;
    Ok(Json(json!({ "ok": true })))
}
