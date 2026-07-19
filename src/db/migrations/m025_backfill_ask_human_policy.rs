//! 2026_06_X9_001：回填 ask_human_policy。把现有 (principal_decider,
//! high_risk_escalation_mode) 映射成 ask_human_policy（decider_chain + 四 escalate_*）。
//! 幂等：已有 ask_human_policy 的行跳过。不删旧字段（向后兼容兜底）。
//!
//! 映射须与纯函数 `resolve_ask_human_policy` 的 None 路径（旧字段映射）字节等价：
//! - all_mode = high_risk_escalation_mode == Some("all")
//! - decider_chain = [{wxid: principal_decider}] 若 Some，否则空
//! - escalateSafetyGuard/escalateUnverifiedProduct/escalateStuck = true
//! - escalateAiPolicyHold = all_mode
//!
//! BSON 键 camelCase（`AskHumanPolicy` derive `#[serde(rename_all="camelCase")]`）。
//! 可选字段（dedupeWindowHours 等）`skip_serializing_if=None`，回填行合法省略。

use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    use futures::TryStreamExt;
    let coll = db.operation_domain_configs();
    let cursor = coll
        .find(doc! { "ask_human_policy": { "$exists": false } }, None)
        .await?;
    let rows: Vec<crate::models::OperationDomainConfig> = cursor.try_collect().await?;
    for cfg in rows {
        let all_mode = cfg.high_risk_escalation_mode.as_deref() == Some("all");
        let chain: Vec<mongodb::bson::Document> = cfg
            .principal_decider
            .as_ref()
            .map(|w| vec![doc! { "wxid": w }])
            .unwrap_or_default();
        let policy = doc! {
            "deciderChain": chain,
            "escalateSafetyGuard": true,
            "escalateUnverifiedProduct": true,
            "escalateAiPolicyHold": all_mode,
            "escalateStuck": true,
        };
        let Some(id) = cfg.id else {
            continue;
        };
        coll.update_one(
            doc! { "_id": id },
            doc! { "$set": { "ask_human_policy": policy, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    }
    Ok(())
}
