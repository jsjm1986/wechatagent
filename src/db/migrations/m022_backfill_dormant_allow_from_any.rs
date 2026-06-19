//! 2026_06_X6_001：G5 阶段2——给存量库 `user_operations.state_machine` 回填
//! `dormant_reactivation` 的 `allowFromAny: true`。
//!
//! 背景：阶段2 把 `dormant_reactivation` 在默认状态机（`prompts.rs`）标了
//! `allowFromAny:true`（客户任何阶段都可能流失 → 转休眠待唤醒合法）。但 `prompts.rs`
//! 只是**种子源**——运行时引擎 `check_state_transition` 读的是 DB
//! `operation_domain_configs.state_machine`。已部署库（如 117）经更早的 m003 跑过后，
//! 其 dormant_reactivation 当时默认 `allowFromAny=false` 未写入该 key，新加的 true 不会
//! 被任何已注册（幂等跳过的）migration 回填 → 存量库仍按旧 allowedFrom 校验，
//! 续费挽留失败转休眠会被拒（fail-soft：不阻断 reply，但产生 transition_rejected 噪声、
//! operation_state 轴不跟随 customer_stage）。
//!
//! 本 migration 复用 [`merge_allowed_from_defaults`]（它已含「默认 allowFromAny=true 且
//! state 缺该 key 时补 true」逻辑），按默认状态机回填——**只补缺失、不覆盖运营人员手改值**
//! （与 m003/m019 同精神）。

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;

use super::helpers::merge_allowed_from_defaults;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let default_state_machine = crate::prompts::default_user_operation_state_machine();
    let default_states = default_state_machine
        .get_array("states")
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_document().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut cursor = db
        .operation_domain_configs()
        .find(doc! { "domain": "user_operations" }, None)
        .await?;
    let mut modified = 0_u64;
    while let Some(config) = cursor.try_next().await? {
        let Some(id) = config.id else { continue };
        let mut state_machine = config.state_machine.clone();
        if merge_allowed_from_defaults(&mut state_machine, &default_states) {
            db.operation_domain_configs()
                .update_one(
                    doc! { "_id": id },
                    doc! {
                        "$set": {
                            "state_machine": state_machine,
                            "updated_at": DateTime::now()
                        }
                    },
                    None,
                )
                .await?;
            modified += 1;
        }
    }
    tracing::info!(
        modified,
        "backfilled state_machine allowFromAny (dormant_reactivation) for user_operations domain"
    );
    Ok(())
}
