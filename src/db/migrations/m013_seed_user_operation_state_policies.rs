//! 2026_05_W4_001：为每个 user_operations 状态机里的 state_key 写一行默认 policy。
//!
//! 默认策略遵循"宽允许 / 窄禁止"原则：所有 state 默认允许回复、确认、静默、跟进和
//! 客户预约请求记录；只有 `forbidsProactive` state 强制 `forbidden=["reply"]`。
//! 已存在的 (workspace_id, domain, state_key) 行被跳过，保留运营人员的手工调整。

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;
use crate::models::OperationStatePolicy;

/// H13：把 `forbidsProactive` 标志映射成 `(allowed, forbidden)` action 列表的**唯一真相**。
///
/// 抽成纯函数后被两条派生路径共用——本迁移（从 DEFAULT 机器 seed）与
/// [`crate::routes::admin_ops_versions::publish_state_machine_version`]（profile activate
/// 联动 publish 行业状态机时重派生）。两处共用一份逻辑，杜绝「m013 与 publish 路径漂移」。
///
/// - `true`  → 禁普通主动回复，但允许无事实确认、静默、跟进和记录客户预约请求；
/// - `false` → 允许完整动作集合。
pub(crate) fn derive_state_policy_lists(forbids_proactive: bool) -> (Vec<String>, Vec<String>) {
    if forbids_proactive {
        (
            vec![
                "acknowledgement".to_string(),
                "silent".to_string(),
                "follow_up".to_string(),
                "appointment_request".to_string(),
            ],
            vec!["reply".to_string()],
        )
    } else {
        (
            vec![
                "reply".to_string(),
                "acknowledgement".to_string(),
                "silent".to_string(),
                "follow_up".to_string(),
                "appointment_request".to_string(),
            ],
            Vec::new(),
        )
    }
}

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let mut cursor = db
        .operation_domain_configs()
        .find(doc! { "domain": "user_operations" }, None)
        .await?;
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    while let Some(config) = cursor.try_next().await? {
        let states = config
            .state_machine
            .get_array("states")
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_document().cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for state in &states {
            let Some(state_key) = state.get_str("key").ok().map(ToString::to_string) else {
                continue;
            };
            let existing = db
                .operation_state_policies()
                .find_one(
                    doc! {
                        "workspace_id": &config.workspace_id,
                        "domain": "user_operations",
                        "state_key": &state_key,
                    },
                    None,
                )
                .await?;
            if existing.is_some() {
                skipped += 1;
                continue;
            }
            // H13：禁主动触达的 state 读 `forbidsProactive` 标志（替代写死的
            // `state_key == "cooldown"`）。DEFAULT 状态机仅 cooldown 标 forbidsProactive
            // → 与改造前逐字等价；换行业的 profile 可标别的 state 禁回复。
            let forbids_proactive = state.get_bool("forbidsProactive").unwrap_or(false);
            let (allowed, forbidden): (Vec<String>, Vec<String>) =
                derive_state_policy_lists(forbids_proactive);
            let policy = OperationStatePolicy {
                id: None,
                workspace_id: config.workspace_id.clone(),
                domain: "user_operations".to_string(),
                state_key: state_key.clone(),
                allowed,
                forbidden,
                recommended_pace: None,
                status: "active".to_string(),
                updated_at: DateTime::now(),
                version: 1,
                current_version: true,
                previous_version: None,
                seeded_by: Some("legacy_migration".to_string()),
            };
            db.operation_state_policies()
                .insert_one(&policy, None)
                .await?;
            inserted += 1;
        }
    }
    tracing::info!(
        migration_id = "2026_05_W4_001_seed_user_operation_state_policies",
        inserted,
        skipped,
        "seeded operation_state_policies for user_operations"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::derive_state_policy_lists;

    /// `forbidsProactive=true` blocks ordinary replies but still permits recording a customer
    /// appointment request because that write is reactive and independently authorized.
    #[test]
    fn derive_forbids_proactive_blocks_reply() {
        let (allowed, forbidden) = derive_state_policy_lists(true);
        assert_eq!(
            allowed,
            vec![
                "acknowledgement".to_string(),
                "silent".to_string(),
                "follow_up".to_string(),
                "appointment_request".to_string(),
            ]
        );
        assert_eq!(forbidden, vec!["reply".to_string()]);
    }

    /// `forbidsProactive=false`：宽允许——allowed=["reply","silent","follow_up"]、forbidden=[]。
    #[test]
    fn derive_allows_all_when_not_forbidden() {
        let (allowed, forbidden) = derive_state_policy_lists(false);
        assert_eq!(
            allowed,
            vec![
                "reply".to_string(),
                "acknowledgement".to_string(),
                "silent".to_string(),
                "follow_up".to_string(),
                "appointment_request".to_string(),
            ]
        );
        assert!(forbidden.is_empty());
    }
}
