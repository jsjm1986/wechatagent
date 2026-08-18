//! Account-wide, time-windowed persona context generated independently of any customer.
//!
//! The model owns the harmless social texture. Code owns account scoping, time windows, schema,
//! optional-call budgeting, CAS/upsert convergence, and the guarantee that customer context never
//! enters the generator input.

use std::sync::LazyLock;
use std::time::Duration;

use dashmap::{mapref::entry::Entry, DashMap};
use mongodb::{
    bson::{doc, oid::ObjectId, Bson, DateTime},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::models::PersonaWorldState;
use crate::prompts;
use crate::routes::AppState;

use super::budget::current_run_budget;
use super::generate_agent_json;

const PROMPT_KEY: &str = "user.persona_world_state.system";
const WINDOW_HOURS: i64 = 6;
const WINDOW_MS: i64 = WINDOW_HOURS * 60 * 60 * 1_000;
const REQUIRED_LLM_TAIL: i32 = 3;
const MAX_STATE_TEXT_CHARS: usize = 600;
const MAX_AVAILABILITY_CHARS: usize = 160;
const MAX_MOOD_CHARS: usize = 80;
const BACKGROUND_REFRESH_DELAY: Duration = Duration::from_secs(1);

type RefreshKey = (String, String, i64);
static REFRESH_IN_FLIGHT: LazyLock<DashMap<RefreshKey, ()>> = LazyLock::new(DashMap::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorldStateWindow {
    effective_from: DateTime,
    effective_until: DateTime,
    local_hour: u32,
    period: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedWorldState {
    state_text: String,
    availability: Option<String>,
    mood: Option<String>,
}

struct RefreshGuard(RefreshKey);

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        REFRESH_IN_FLIGHT.remove(&self.0);
    }
}

fn refresh_key(
    workspace_id: &str,
    account_id: &str,
    timezone_offset_hours: i32,
    now: DateTime,
) -> RefreshKey {
    let window = world_state_window(now, timezone_offset_hours);
    (
        workspace_id.to_string(),
        account_id.to_string(),
        window.effective_from.timestamp_millis(),
    )
}

fn claim_refresh_slot(key: RefreshKey) -> bool {
    match REFRESH_IN_FLIGHT.entry(key) {
        Entry::Occupied(_) => false,
        Entry::Vacant(entry) => {
            entry.insert(());
            true
        }
    }
}

/// Schedule optional account-level social texture after the customer turn has settled.
///
/// The spawned task does not inherit the current run budget, run id, or customer message. A
/// process-local slot coalesces concurrent contacts on the same account and six-hour window.
pub(crate) fn schedule_world_state_refresh(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    effective_soul: &str,
    timezone_offset_hours: i32,
) -> bool {
    let effective_soul = effective_soul.trim();
    if effective_soul.is_empty() {
        return false;
    }

    let key = refresh_key(
        workspace_id,
        account_id,
        timezone_offset_hours,
        DateTime::now(),
    );
    if !claim_refresh_slot(key.clone()) {
        return false;
    }

    let state = state.clone();
    let workspace_id = workspace_id.to_string();
    let account_id = account_id.to_string();
    let effective_soul = effective_soul.to_string();
    tokio::spawn(async move {
        let _guard = RefreshGuard(key);
        // Let the foreground turn release its provider permit and return to the caller before
        // optional account maintenance competes for capacity.
        tokio::time::sleep(BACKGROUND_REFRESH_DELAY).await;
        if let Err(error) = ensure_effective_world_state(
            &state,
            &workspace_id,
            &account_id,
            &effective_soul,
            timezone_offset_hours,
            None,
        )
        .await
        {
            tracing::warn!(
                %error,
                %workspace_id,
                %account_id,
                "persona world-state background refresh failed soft"
            );
        }
    });
    true
}

pub(crate) async fn ensure_effective_world_state(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    effective_soul: &str,
    timezone_offset_hours: i32,
    run_id: Option<&str>,
) -> AppResult<Option<PersonaWorldState>> {
    let now = DateTime::now();
    let current = load_current(state, workspace_id, account_id).await?;
    if let Some(current) = current.as_ref() {
        if state_is_effective(current, now) {
            return Ok(Some(current.clone()));
        }
        // A scheduled current row is an external lifecycle decision. Do not overwrite it merely
        // because its effective window has not started yet.
        if current.effective_from > now {
            return Ok(None);
        }
    }

    let soul = effective_soul.trim();
    if soul.is_empty() || !optional_generation_has_capacity() {
        return Ok(None);
    }

    let window = world_state_window(now, timezone_offset_hours);
    let generated = generate_world_state(
        state,
        workspace_id,
        account_id,
        soul,
        timezone_offset_hours,
        window,
        run_id,
    )
    .await?;
    if DateTime::now() >= window.effective_until {
        return Ok(None);
    }
    persist_generated_state(
        state,
        workspace_id,
        account_id,
        current.as_ref(),
        &generated,
        window,
        now,
    )
    .await
    .map(Some)
}

fn optional_generation_has_capacity() -> bool {
    current_run_budget().is_none_or(|budget| {
        !budget.should_stop_optional_llm_calls()
            && budget.available_llm_calls_before_tail(REQUIRED_LLM_TAIL) > 0
    })
}

async fn load_current(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Option<PersonaWorldState>> {
    state
        .db
        .persona_world_states()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "current": true,
            },
            None,
        )
        .await
        .map_err(AppError::from)
}

fn state_is_effective(state: &PersonaWorldState, now: DateTime) -> bool {
    state.current && state.effective_from <= now && state.effective_until > now
}

fn world_state_window(now: DateTime, timezone_offset_hours: i32) -> WorldStateWindow {
    let offset_hours = timezone_offset_hours.clamp(-12, 14);
    let offset_ms = i64::from(offset_hours) * 60 * 60 * 1_000;
    let local_ms = now.timestamp_millis().saturating_add(offset_ms);
    let local_window_start = local_ms.div_euclid(WINDOW_MS) * WINDOW_MS;
    let effective_from_ms = local_window_start.saturating_sub(offset_ms);
    let effective_until_ms = effective_from_ms.saturating_add(WINDOW_MS);
    let local_hour = local_ms.div_euclid(60 * 60 * 1_000).rem_euclid(24) as u32;
    let period = match local_hour {
        0..=5 => "late_night",
        6..=11 => "morning",
        12..=17 => "afternoon",
        _ => "evening",
    };
    WorldStateWindow {
        effective_from: DateTime::from_millis(effective_from_ms),
        effective_until: DateTime::from_millis(effective_until_ms),
        local_hour,
        period,
    }
}

async fn generate_world_state(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    effective_soul: &str,
    timezone_offset_hours: i32,
    window: WorldStateWindow,
    run_id: Option<&str>,
) -> AppResult<GeneratedWorldState> {
    let system = prompts::load_prompt(&state.db, workspace_id, PROMPT_KEY).await?;
    let user = serde_json::to_string(&json!({
        "publishedSoul": effective_soul,
        "trustedTimeContext": {
            "timezoneOffsetHours": timezone_offset_hours.clamp(-12, 14),
            "localHour": window.local_hour,
            "period": window.period,
            "windowStartMillis": window.effective_from.timestamp_millis(),
            "windowEndMillis": window.effective_until.timestamp_millis(),
        }
    }))?;
    let value = generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        None,
        run_id,
        PROMPT_KEY,
        &system,
        &user,
    )
    .await?;
    parse_generated_world_state(value)
}

fn parse_generated_world_state(value: Value) -> AppResult<GeneratedWorldState> {
    let root = value.as_object().ok_or_else(|| schema_error("root"))?;
    let state_text = required_bounded_string(root, "stateText", MAX_STATE_TEXT_CHARS)?;
    let availability = optional_bounded_string(root, "availability", MAX_AVAILABILITY_CHARS)?;
    let mood = optional_bounded_string(root, "mood", MAX_MOOD_CHARS)?;
    Ok(GeneratedWorldState {
        state_text,
        availability,
        mood,
    })
}

fn required_bounded_string(
    root: &serde_json::Map<String, Value>,
    key: &str,
    max_chars: usize,
) -> AppResult<String> {
    let value = root
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| schema_error(key))?;
    if value.chars().count() > max_chars {
        return Err(schema_error(key));
    }
    Ok(value.to_string())
}

fn optional_bounded_string(
    root: &serde_json::Map<String, Value>,
    key: &str,
    max_chars: usize,
) -> AppResult<Option<String>> {
    match root.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            if value.chars().count() > max_chars {
                return Err(schema_error(key));
            }
            Ok(Some(value.to_string()))
        }
        _ => Err(schema_error(key)),
    }
}

fn schema_error(field: &str) -> AppError {
    AppError::External(format!("persona_world_state_schema_invalid:{field}"))
}

async fn persist_generated_state(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    previous_current: Option<&PersonaWorldState>,
    generated: &GeneratedWorldState,
    window: WorldStateWindow,
    now: DateTime,
) -> AppResult<PersonaWorldState> {
    let mut filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "current": true,
    };
    if let Some(previous) = previous_current {
        if let Some(id) = previous.id {
            filter.insert("_id", id);
        }
        filter.insert("version", previous.version);
        filter.insert("effective_until", previous.effective_until);
    }

    let mut set = doc! {
        "state_text": &generated.state_text,
        "effective_from": window.effective_from,
        "effective_until": window.effective_until,
        "generated_by": format!("ai:{PROMPT_KEY}"),
        "updated_at": now,
    };
    set.insert(
        "availability",
        optional_bson(generated.availability.as_deref()),
    );
    set.insert("mood", optional_bson(generated.mood.as_deref()));
    let update = doc! {
        "$set": set,
        "$setOnInsert": {
            "_id": ObjectId::new(),
            "workspace_id": workspace_id,
            "account_id": account_id,
            "current": true,
            "created_at": now,
        },
        "$inc": { "version": 1i64 },
    };
    let result = state
        .db
        .persona_world_states()
        .find_one_and_update(
            filter,
            update,
            FindOneAndUpdateOptions::builder()
                .upsert(previous_current.is_none())
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;

    match result {
        Ok(Some(persisted)) => Ok(persisted),
        Ok(None) => converge_on_current(state, workspace_id, account_id, now).await,
        Err(error) => match converge_on_current(state, workspace_id, account_id, now).await {
            Ok(current) => Ok(current),
            Err(_) => Err(error.into()),
        },
    }
}

async fn converge_on_current(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    now: DateTime,
) -> AppResult<PersonaWorldState> {
    load_current(state, workspace_id, account_id)
        .await?
        .filter(|current| state_is_effective(current, now))
        .ok_or_else(|| {
            AppError::Conflict(
                "persona world-state CAS lost without an effective winner".to_string(),
            )
        })
}

fn optional_bson(value: Option<&str>) -> Bson {
    value
        .map(|value| Bson::String(value.to_string()))
        .unwrap_or(Bson::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_window_is_aligned_to_the_configured_local_period() {
        let now = DateTime::parse_rfc3339_str("2026-08-15T03:30:00Z").unwrap();
        let window = world_state_window(now, 8);
        assert_eq!(window.local_hour, 11);
        assert_eq!(window.period, "morning");
        assert_eq!(
            window.effective_from,
            DateTime::parse_rfc3339_str("2026-08-14T22:00:00Z").unwrap()
        );
        assert_eq!(
            window.effective_until,
            DateTime::parse_rfc3339_str("2026-08-15T04:00:00Z").unwrap()
        );
    }

    #[test]
    fn generated_contract_is_structural_and_bounded() {
        let parsed = parse_generated_world_state(json!({
            "stateText": "上午在整理当天的咨询记录，回复节奏比较从容。",
            "availability": "可以正常聊天，但不承诺即时处理现实事务。",
            "mood": "平稳、专注"
        }))
        .unwrap();
        assert!(!parsed.state_text.is_empty());
        assert!(parsed.availability.is_some());
        assert!(parsed.mood.is_some());

        assert!(parse_generated_world_state(json!({
            "stateText": "",
            "availability": null,
            "mood": null
        }))
        .is_err());
        assert!(parse_generated_world_state(json!({
            "stateText": "正常状态",
            "availability": [],
            "mood": null
        }))
        .is_err());
    }

    #[test]
    fn refresh_slots_coalesce_same_account_window() {
        let now = DateTime::parse_rfc3339_str("2026-08-18T03:30:00Z").unwrap();
        let key = refresh_key("workspace-slot-test", "account-slot-test", 8, now);
        assert!(claim_refresh_slot(key.clone()));
        assert!(!claim_refresh_slot(key.clone()));
        REFRESH_IN_FLIGHT.remove(&key);
        assert!(claim_refresh_slot(key.clone()));
        REFRESH_IN_FLIGHT.remove(&key);
    }
}
