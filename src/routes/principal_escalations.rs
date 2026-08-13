//! 决策请示通道 admin REST 端点：列表 / admin 直接裁决 / 改派。
//! admin 在此是"幕后决策人"（真人决策），客户仍只收 AI 口吻转述（relay 下游不变）。

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::agent::escalation::{
    list_escalations_by_workspace, materialize_principal_card_delivery, reassign_escalation,
    resolve_escalation,
};
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::models::{AgentPrincipalEscalation, PrincipalDecision};
use mongodb::bson::DateTime;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// 单条请示 → 列表项 JSON（具名以便单测）。
///
/// `createdAt` / `authorizationExpiresAt` 必须经 `dt_to_string` 转 RFC3339 字符串：
/// 裸 `bson::DateTime` 会序列化成扩展 JSON 对象 `{"$date":{"$numberLong":…}}`，
/// 前端（ResolvedEscalations.formatExpiry）把对象当 React child 渲染会整页崩溃
/// （domain_profiles::profile_view 注释记录的白屏事故同款形态，wire 统一 RFC3339）。
fn escalation_list_item_json(e: &AgentPrincipalEscalation, now_ms: i64) -> Value {
    let age_hours = (now_ms - e.created_at.timestamp_millis()) as f64 / (3600.0 * 1000.0);
    json!({
        "shortCode": e.short_code,
        "contactWxid": e.contact_wxid,
        "category": e.category,
        "reason": e.reason,
        "questionForPrincipal": e.question_for_principal,
        "principalWxid": e.principal_wxid,
        "status": e.status,
        "ageHours": age_hours,
        "createdAt": crate::models::dt_to_string(e.created_at),
        "decision": e.decision,
        "authorizationExpiresAt": e.authorization_expires_at.and_then(crate::models::dt_to_string),
        "resolvedVia": e.resolved_via,
    })
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
        .map(|e| escalation_list_item_json(e, now))
        .collect();
    Ok(Json(json!({ "items": json_items })))
}

fn default_resolve_exemption_type() -> String {
    crate::models::EXEMPTION_TYPE_NONE.to_string()
}

const MAX_AUTHORIZATION_WINDOW_HOURS: f64 = 24.0 * 365.0;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

fn validate_admin_decision(body: ResolveBody) -> AppResult<PrincipalDecision> {
    let verdict = body.verdict.trim().to_string();
    if !crate::models::ALLOWED_PRINCIPAL_VERDICT.contains(&verdict.as_str()) {
        return Err(AppError::BadRequest(
            "verdict must be approved|rejected|conditional|deferred|delegated_back".to_string(),
        ));
    }
    let exemption_type = body.exemption_type.trim().to_string();
    if !matches!(
        exemption_type.as_str(),
        crate::models::EXEMPTION_TYPE_NONE
            | crate::models::EXEMPTION_TYPE_CUSTOMER_ONLY
            | crate::models::EXEMPTION_TYPE_KNOWLEDGE
    ) {
        return Err(AppError::BadRequest(
            "exemptionType must be none|customer_only|knowledge".to_string(),
        ));
    }
    let substance = body.substance.trim().to_string();
    if matches!(
        verdict.as_str(),
        crate::models::PRINCIPAL_VERDICT_APPROVED | crate::models::PRINCIPAL_VERDICT_CONDITIONAL
    ) && substance.is_empty()
    {
        return Err(AppError::BadRequest(
            "approved or conditional decision requires non-empty substance".to_string(),
        ));
    }
    if exemption_type != crate::models::EXEMPTION_TYPE_NONE
        && !matches!(
            verdict.as_str(),
            crate::models::PRINCIPAL_VERDICT_APPROVED
                | crate::models::PRINCIPAL_VERDICT_CONDITIONAL
        )
    {
        return Err(AppError::BadRequest(
            "exemptionType requires approved or conditional verdict".to_string(),
        ));
    }
    if body.authorization_window_hours.is_some()
        && !matches!(
            verdict.as_str(),
            crate::models::PRINCIPAL_VERDICT_APPROVED
                | crate::models::PRINCIPAL_VERDICT_CONDITIONAL
        )
    {
        return Err(AppError::BadRequest(
            "authorizationWindowHours requires approved or conditional verdict".to_string(),
        ));
    }
    let authorization_window_hours = match body.authorization_window_hours {
        Some(hours)
            if hours.is_finite() && hours > 0.0 && hours <= MAX_AUTHORIZATION_WINDOW_HOURS =>
        {
            Some(hours)
        }
        Some(_) => {
            return Err(AppError::BadRequest(format!(
            "authorizationWindowHours must be finite and in (0, {MAX_AUTHORIZATION_WINDOW_HOURS}]"
        )))
        }
        None => None,
    };
    let constraints = body
        .constraints
        .into_iter()
        .map(|constraint| constraint.trim().to_string())
        .filter(|constraint| !constraint.is_empty())
        .collect();
    Ok(PrincipalDecision {
        verdict,
        substance,
        constraints,
        authorization_window_hours,
        exemption_type,
    })
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
    let decision = validate_admin_decision(body)?;
    // deferred：领导/admin 暂缓 → 保持 pending 继续等待（与 wechat 路径 mod.rs 一致），不 resolve、不 relay。
    if decision.verdict == crate::models::PRINCIPAL_VERDICT_DEFERRED {
        return Ok(Json(json!({ "ok": true, "deferred": true })));
    }
    // 仅约束本次裁决转述的可用期；customer_only / knowledge 产生的客户豁免
    // 按专项设计长期常驻，直到管理员显式撤销，二者是独立维度。
    let expires = decision.authorization_window_hours.map(|hours| {
        DateTime::from_millis(DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64)
    });
    let resolved = resolve_escalation(&state, &entry, &decision, expires, "admin").await?;
    if resolved.is_none() {
        return Ok(Json(json!({ "ok": true, "alreadyResolved": true })));
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod resolve_validation_tests {
    use super::*;

    fn body(verdict: &str) -> ResolveBody {
        ResolveBody {
            verdict: verdict.to_string(),
            substance: "同意按该口径处理".to_string(),
            constraints: vec![],
            authorization_window_hours: None,
            exemption_type: crate::models::EXEMPTION_TYPE_NONE.to_string(),
        }
    }

    #[test]
    fn rejects_invalid_verdict_instead_of_silent_defer() {
        assert!(validate_admin_decision(body("maybe")).is_err());
    }

    #[test]
    fn approved_requires_substance() {
        let mut value = body(crate::models::PRINCIPAL_VERDICT_APPROVED);
        value.substance = "  ".to_string();
        assert!(validate_admin_decision(value).is_err());
    }

    #[test]
    fn rejects_invalid_exemption_and_window() {
        let mut exemption = body(crate::models::PRINCIPAL_VERDICT_APPROVED);
        exemption.exemption_type = "global_forever".to_string();
        assert!(validate_admin_decision(exemption).is_err());

        let mut rejected = body(crate::models::PRINCIPAL_VERDICT_REJECTED);
        rejected.authorization_window_hours = Some(24.0);
        assert!(validate_admin_decision(rejected).is_err());

        for hours in [
            0.0,
            -1.0,
            f64::INFINITY,
            MAX_AUTHORIZATION_WINDOW_HOURS + 1.0,
        ] {
            let mut value = body(crate::models::PRINCIPAL_VERDICT_CONDITIONAL);
            value.authorization_window_hours = Some(hours);
            assert!(validate_admin_decision(value).is_err(), "hours={hours}");
        }
    }

    #[test]
    fn accepts_explicit_long_lived_customer_exemption_with_bounded_relay_window() {
        let mut value = body(crate::models::PRINCIPAL_VERDICT_CONDITIONAL);
        value.exemption_type = crate::models::EXEMPTION_TYPE_CUSTOMER_ONLY.to_string();
        value.authorization_window_hours = Some(24.0);
        let decision = validate_admin_decision(value).expect("valid decision");
        assert_eq!(decision.authorization_window_hours, Some(24.0));
        assert_eq!(decision.exemption_type, "customer_only");
    }
}

#[cfg(test)]
mod list_projection_tests {
    use super::*;

    fn resolved_escalation_fixture() -> AgentPrincipalEscalation {
        let created = DateTime::from_millis(1_700_000_000_000);
        AgentPrincipalEscalation {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            contact_wxid: "wxid_cust".into(),
            short_code: "E1A2".into(),
            status: "resolved".into(),
            category: "discount_request".into(),
            reason: "客户想要折扣，超出 AI 职权".into(),
            question_for_principal: "能否给折扣".into(),
            principal_wxid: "wxid_boss".into(),
            protocol: None,
            decision: Some(PrincipalDecision {
                verdict: "conditional".into(),
                substance: "同意 9 折，本周内有效".into(),
                constraints: vec!["本周内付款".into()],
                authorization_window_hours: Some(24.0),
                exemption_type: crate::models::EXEMPTION_TYPE_NONE.into(),
            }),
            authorization_expires_at: Some(DateTime::from_millis(1_700_086_400_000)),
            is_generalizable: false,
            knowledge_proposal_emitted: false,
            last_holding_reply_ms: None,
            last_pushed_at_ms: None,
            created_at: created,
            updated_at: created,
            resolved_at: Some(created),
            resolved_via: Some("admin".into()),
            relay_state: None,
            relay_task_id: None,
            relay_enqueued_at: None,
            relay_terminal_at: None,
            relay_terminal_reason: None,
        }
    }

    /// 裁决历史契约：时间字段必须是 RFC3339 字符串，绝不能是 bson 扩展 JSON
    /// 对象 `{"$date":…}`——前端把对象当 React child 渲染会整页崩溃
    /// （domain_profiles::profile_view 注释记录的白屏事故同款形态）。
    #[test]
    fn list_item_serializes_datetimes_as_rfc3339_strings_not_extjson_objects() {
        let entry = resolved_escalation_fixture();
        let v = escalation_list_item_json(&entry, 1_700_000_000_000);

        let created = v["createdAt"].as_str().expect("createdAt must be a string");
        assert!(created.starts_with("2023-11-14T22:13:20"), "{created}");
        let expires = v["authorizationExpiresAt"]
            .as_str()
            .expect("authorizationExpiresAt must be a string");
        assert!(expires.starts_with("2023-11-15T22:13:20"), "{expires}");

        let raw = serde_json::to_string(&v).expect("serialize");
        assert!(!raw.contains("$date"), "no bson extjson leakage: {raw}");
        // decision 内层键保持 snake_case（PrincipalDecision 无 rename_all，前端按此消费）。
        assert_eq!(v["decision"]["authorization_window_hours"], 24.0);
    }

    /// 契约快照：固定输入 → fixture 对账
    /// （`frontend/src/contracts/escalation_list_item.fixture.json`）。防线上形状
    /// 漂移，尤其时间字段回退成 bson 扩展 JSON 对象（白屏事故形态）。
    #[test]
    fn escalation_list_item_projection_matches_contract_fixture() {
        let entry = resolved_escalation_fixture();
        // now = created + 1h 整，ageHours 恒为 1.0，快照稳定。
        let projected = escalation_list_item_json(&entry, 1_700_003_600_000);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "escalation_list_item",
            projected,
        );
    }

    #[test]
    fn list_item_keeps_null_expiry_and_pending_shape() {
        let mut entry = resolved_escalation_fixture();
        entry.authorization_expires_at = None;
        entry.decision = None;
        entry.resolved_via = None;
        let v = escalation_list_item_json(&entry, 1_700_000_000_000);
        assert!(v["authorizationExpiresAt"].is_null());
        assert!(v["decision"].is_null());
        assert!(v["resolvedVia"].is_null());
        assert_eq!(v["shortCode"], "E1A2");
        assert_eq!(v["ageHours"], 0.0);
    }
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
