//! 健康检查路由：返回服务状态及基础元数据，供前端 / 监控探活使用。

use axum::{extract::State, Json};
use serde_json::{json, Value};

use super::AppState;

pub(super) async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "appBaseUrl": state.config.app_base_url,
        // M4 W4 Task 5.8：前端 EvolutionCenterTab 据此决定是否渲染"演化器未启用"占位。
        "evolutionEnabled": state.config.evolution_enabled,
    }))
}
