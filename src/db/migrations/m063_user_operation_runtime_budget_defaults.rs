//! Upgrade untouched system-seeded user-operation budgets to the loop-capable defaults.
//!
//! The legacy seed could fund only the first generation/review stages. The current harness may
//! also need knowledge routing, repair, and a second authorization pass. This migration changes
//! only current `seeded_by=system` rows that still carry the complete legacy budget tuple and have
//! never persisted an escalated budget. Any operator-owned or partially customized configuration
//! remains untouched.

use mongodb::bson::{doc, DateTime, Document};

use crate::db::Database;
use crate::error::AppResult;

const LEGACY_RUN_TOKEN_BUDGET: i64 = 30_000;
const LEGACY_RUN_MAX_LLM_CALLS: i32 = 6;
const LEGACY_SIMULATION_TOKEN_BUDGET: i64 = 60_000;

pub(crate) const RUN_TOKEN_BUDGET: i64 = 300_000;
pub(crate) const RUN_TOKEN_BUDGET_ESCALATED: i64 = 600_000;
pub(crate) const RUN_MAX_LLM_CALLS: i32 = 10;
pub(crate) const SIMULATION_TOKEN_BUDGET: i64 = 300_000;

fn untouched_legacy_system_budget_filter() -> Document {
    doc! {
        "domain": "user_operations",
        "current_version": true,
        "seeded_by": "system",
        "runtime_parameters.runTokenBudget": LEGACY_RUN_TOKEN_BUDGET,
        "runtime_parameters.runMaxLlmCalls": LEGACY_RUN_MAX_LLM_CALLS,
        "runtime_parameters.simulationTokenBudget": LEGACY_SIMULATION_TOKEN_BUDGET,
        "runtime_parameters.runTokenBudgetEscalated": { "$exists": false },
    }
}

pub async fn run_step(db: &Database) -> AppResult<()> {
    let collection = db
        .operation_domain_configs()
        .clone_with_type::<mongodb::bson::Document>();
    let result = collection
        .update_many(
            untouched_legacy_system_budget_filter(),
            doc! {
                "$set": {
                    "runtime_parameters.runTokenBudget": RUN_TOKEN_BUDGET,
                    "runtime_parameters.runTokenBudgetEscalated": RUN_TOKEN_BUDGET_ESCALATED,
                    "runtime_parameters.runMaxLlmCalls": RUN_MAX_LLM_CALLS,
                    "runtime_parameters.simulationTokenBudget": SIMULATION_TOKEN_BUDGET,
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await?;
    tracing::info!(
        matched = result.matched_count,
        changed = result.modified_count,
        "upgraded untouched system user-operation runtime budgets"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuntimeParametersTyped;

    #[test]
    fn migration_target_matches_current_runtime_defaults() {
        let defaults = RuntimeParametersTyped::default();
        assert_eq!(defaults.run_token_budget, RUN_TOKEN_BUDGET);
        assert_eq!(
            defaults.run_token_budget_escalated,
            RUN_TOKEN_BUDGET_ESCALATED
        );
        assert_eq!(defaults.run_max_llm_calls, RUN_MAX_LLM_CALLS);
        assert_eq!(defaults.simulation_token_budget, SIMULATION_TOKEN_BUDGET);
    }
}
