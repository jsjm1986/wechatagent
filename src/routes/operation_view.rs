//! 运营态聚合视图路由。
//!
//! `GET /api/operation/active-view`：运营态**只读**聚合端点。返回当前激活
//! DomainProfile 的维度声明（kind / displayName / participatesInDecision）+ 各
//! 维度的 taxonomy 取值字典（id → label），供前端 `labelFor` 把 canonical
//! 英文 id 翻译成中文 display_name。
//!
//! 与 admin-only 的 `active_domain_profile`（`domain_profiles.rs`）的区别：本端点
//! 在 profile 维度声明之上**聚合了 `system_taxonomies` 的取值字典**（流 B 前端翻译
//! 的后端地基），且 kind 集额外并入 `relationship_type`（admin 直写维度，不在
//! profile_dimensions 里，但前端关系下拉需要它的取值字典）。
//!
//! 单一真相源 `system_taxonomies`（canonical 英文 id → 中文 display_name）：本端点
//! 只读不写，不引入任何越权写入面。

use axum::{extract::State, Extension, Json};
use serde_json::{json, Map, Value};

use crate::{auth::AuthenticatedAdmin, error::AppResult};

use super::AppState;

/// `GET /api/operation/active-view` —— 当前 active profile 维度 + taxonomy 取值字典。
///
/// 鉴权：整个 `api_router` 被 `require_session` 层覆盖，注入 `AuthenticatedAdmin`
/// （cookie / JWT 同一角色，无 account_id 字段）。本系统唯一鉴权角色。
pub async fn active_view(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    // 1) 加载当前 workspace 的 active DomainProfile（无 active 时回落 DEFAULT 销售
    //    profile，不会 panic）。
    let profile =
        crate::agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await;

    // 2) 维度声明（camelCase wire）。
    let dimensions: Vec<Value> = profile
        .profile_dimensions
        .iter()
        .map(|d| {
            json!({
                "kind": d.kind,
                "displayName": d.display_name,
                "participatesInDecision": d.participates_in_decision,
            })
        })
        .collect();

    // 3) 取值字典的 kind 集 = profile_dimensions 的 kind ∪ ["relationship_type"]。
    //    relationship_type 是 admin 直写维度，不在 profile_dimensions 里（DEFAULT
    //    profile_dimensions 只有 customer_stage / intent_level），但前端关系下拉需要
    //    它的取值字典 —— 不并入则 Task 6 的关系下拉会空。
    let mut kinds: Vec<String> = profile
        .profile_dimensions
        .iter()
        .map(|d| d.kind.clone())
        .collect();
    if !kinds.iter().any(|k| k == "relationship_type") {
        kinds.push("relationship_type".to_string());
    }

    // 4) 预热进程级 taxonomy cache（冷 / 过期缓存会返回空，必须先 find_or_load）。
    let cache = crate::agent::taxonomy::global_taxonomy_cache();
    cache.find_or_load(&state.db).await;

    // 5) 逐 kind 建取值字典 {kind: [{id, label}]}。scope 第二参传 current_workspace：
    //    dimension_values_with_labels 在 account 私有 scope 未命中时回落 global，
    //    DEFAULT 取值都在 global seed，故全局字典可达。
    let mut taxonomies = Map::new();
    for kind in &kinds {
        let pairs = crate::agent::taxonomy::dimension_values_with_labels(
            kind,
            &admin.current_workspace,
            cache.as_ref(),
        );
        let values: Vec<Value> = pairs
            .into_iter()
            .map(|(id, label)| json!({ "id": id, "label": label }))
            .collect();
        taxonomies.insert(kind.clone(), Value::Array(values));
    }

    Ok(Json(json!({
        "dimensions": dimensions,
        "taxonomies": taxonomies,
    })))
}
