//! Durable commit boundary for autonomous proactive work.
//!
//! Candidate selection and copy remain owned by the existing Planner/Agent
//! paths. This module only linearizes the database side of an accepted
//! business intent: one deterministic task, one audit event, and one daily
//! quota reservation commit together in a MongoDB transaction.

use mongodb::{
    bson::{doc, oid::ObjectId, to_document, Bson, DateTime, Document},
    options::{ReadConcern, TransactionOptions, UpdateOptions},
    ClientSession,
};
use sha2::{Digest, Sha256};

use crate::{
    models::{AgentEvent, AgentTask, BehaviorSignal, Contact},
    routes::AppState,
};

const MAX_TRANSACTION_ATTEMPTS: usize = 12;
const INTENT_HASH_FIELD: &str = "proactive_intent_hash";
const QUOTA_RETENTION_DAYS: i64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    Emitted,
    Duplicate,
    Capped,
}

#[derive(Debug, Clone)]
pub struct DailyQuota {
    pub namespace: &'static str,
    /// `Some(account)` gives an account-scoped bucket; `None` gives one bucket
    /// for the whole workspace (used by the existing cold/silence policies).
    pub account_scope: Option<String>,
    pub total_cap: i64,
    pub segment_cap: Option<i64>,
    /// Counts observed in the durable legacy event log. During a rolling
    /// deploy, every reservation monotonically raises the bucket to at least
    /// these baselines before incrementing it, so an older process that emits
    /// after bucket creation cannot make the persistent quota undercount.
    pub initial_total: i64,
    pub initial_segment: i64,
}

#[derive(Debug, Clone)]
pub struct FollowUpIntent {
    pub contact: Contact,
    pub segment: &'static str,
    /// Stable business fact/generation selected by the caller, never prose.
    pub subject: String,
    pub content: String,
    pub event_kind: &'static str,
    pub event_summary: String,
    pub event_details: Document,
    pub now: DateTime,
    pub quota: DailyQuota,
}

fn sha256(parts: &[&str]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

fn hex_hash(hash: &[u8; 32]) -> String {
    hex::encode(hash)
}

fn object_id_from_hash(hash: &[u8; 32]) -> ObjectId {
    let mut bytes = [0_u8; 12];
    bytes.copy_from_slice(&hash[..12]);
    ObjectId::from_bytes(bytes)
}

fn utc_day_key(now: DateTime) -> String {
    let day = now.timestamp_millis().div_euclid(86_400_000);
    day.to_string()
}

fn validate_identity(name: &str, value: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.trim() != value || value.contains('\0') {
        anyhow::bail!("invalid proactive {name}");
    }
    Ok(())
}

fn validate_token(name: &str, value: &str) -> anyhow::Result<()> {
    validate_identity(name, value)?;
    if value.contains('.') || value.starts_with('$') {
        anyhow::bail!("invalid proactive {name}");
    }
    Ok(())
}

fn validate_quota_request(
    workspace_id: &str,
    segment: &str,
    quota: &DailyQuota,
) -> anyhow::Result<()> {
    // `segment` becomes the dynamic MongoDB field `segments.<segment>`.
    // Reject field-path metacharacters rather than letting a future caller
    // create nested/colliding counters. Namespace is protocol-controlled and
    // uses the same conservative token grammar. Workspace/account are ordinary
    // BSON values and hash inputs, so preserve their existing ability to
    // contain `.` or `$` while still rejecting ambiguous whitespace/NUL forms.
    validate_token("segment", segment)?;
    validate_token("quota namespace", quota.namespace)?;
    validate_identity("workspace", workspace_id)?;
    if let Some(account_scope) = quota.account_scope.as_deref() {
        validate_identity("quota account scope", account_scope)?;
    }
    Ok(())
}

fn intent_identity(intent: &FollowUpIntent) -> (ObjectId, String) {
    let hash = sha256(&[
        "proactive-follow-up:v1",
        &intent.contact.workspace_id,
        &intent.contact.account_id,
        &intent.contact.wxid,
        intent.segment,
        &intent.subject,
    ]);
    (object_id_from_hash(&hash), hex_hash(&hash))
}

fn quota_id(workspace_id: &str, quota: &DailyQuota, now: DateTime) -> String {
    let account = quota.account_scope.as_deref().unwrap_or("*");
    hex_hash(&sha256(&[
        "proactive-daily-quota:v1",
        quota.namespace,
        workspace_id,
        account,
        &utc_day_key(now),
    ]))
}

fn build_task(intent: &FollowUpIntent, task_id: ObjectId) -> AgentTask {
    AgentTask {
        id: Some(task_id),
        workspace_id: intent.contact.workspace_id.clone(),
        account_id: intent.contact.account_id.clone(),
        contact_wxid: intent.contact.wxid.clone(),
        kind: "follow_up".to_string(),
        run_at: intent.now,
        expires_at: Some(DateTime::from_millis(
            intent.now.timestamp_millis() + 48 * 60 * 60 * 1000,
        )),
        content: intent.content.clone(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: true,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: intent.now,
        updated_at: intent.now,
    }
}

fn build_event(intent: &FollowUpIntent, event_id: ObjectId) -> AgentEvent {
    AgentEvent {
        id: Some(event_id),
        workspace_id: intent.contact.workspace_id.clone(),
        account_id: intent.contact.account_id.clone(),
        contact_wxid: Some(intent.contact.wxid.clone()),
        kind: intent.event_kind.to_string(),
        status: "emitted".to_string(),
        summary: intent.event_summary.clone(),
        details: Some(intent.event_details.clone()),
        created_at: intent.now,
        dedupe_key: None,
    }
}

async fn ensure_and_reserve_quota(
    state: &AppState,
    workspace_id: &str,
    segment: &str,
    now: DateTime,
    quota: &DailyQuota,
    session: &mut ClientSession,
) -> anyhow::Result<bool> {
    if quota.total_cap <= 0 || quota.segment_cap.is_some_and(|cap| cap <= 0) {
        return Ok(false);
    }

    let id = quota_id(workspace_id, quota, now);
    let segment_path = format!("segments.{segment}");
    let mut initial_segments = Document::new();
    initial_segments.insert(segment, quota.initial_segment.max(0));
    let bucket = state
        .db
        .raw()
        .collection::<Document>("proactive_daily_quotas");
    bucket
        .update_one_with_session(
            doc! { "_id": &id },
            doc! {
                "$setOnInsert": {
                    "workspace_id": workspace_id,
                    "account_scope": quota.account_scope.as_deref().map(Bson::from).unwrap_or(Bson::Null),
                    "namespace": quota.namespace,
                    "day": utc_day_key(now),
                    "total": quota.initial_total.max(0),
                    "segments": initial_segments,
                    "created_at": now,
                    "expires_at": DateTime::from_millis(
                        now.timestamp_millis()
                            + QUOTA_RETENTION_DAYS * 24 * 60 * 60 * 1000
                    ),
                }
            },
            UpdateOptions::builder().upsert(true).build(),
            session,
        )
        .await?;

    // A legacy process can continue appending emit events after a new process
    // has already created this bucket. Reconcile both counters monotonically
    // on every reservation, not only on insert. `$max` also materializes a
    // planner segment first seen after another segment created the shared
    // bucket. New-protocol events are already reflected in both the event log
    // and bucket, so taking the maximum never double-counts them.
    let mut observed_baseline = doc! { "total": quota.initial_total.max(0) };
    observed_baseline.insert(segment_path.clone(), quota.initial_segment.max(0));
    bucket
        .update_one_with_session(
            doc! { "_id": &id },
            doc! { "$max": observed_baseline },
            None,
            session,
        )
        .await?;

    let mut and_filters = vec![doc! { "total": { "$lt": quota.total_cap } }];
    if let Some(segment_cap) = quota.segment_cap {
        let mut segment_filter = Document::new();
        segment_filter.insert(segment_path.clone(), doc! { "$lt": segment_cap });
        and_filters.push(segment_filter);
    }
    let mut inc = doc! { "total": 1_i64 };
    inc.insert(segment_path, 1_i64);
    let result = bucket
        .update_one_with_session(
            doc! { "_id": &id, "$and": and_filters },
            doc! {
                "$inc": inc,
                "$set": { "updated_at": now },
            },
            None,
            session,
        )
        .await?;
    Ok(result.matched_count == 1)
}

async fn commit_follow_up_once(
    state: &AppState,
    intent: &FollowUpIntent,
    task_id: ObjectId,
    intent_hash: &str,
) -> anyhow::Result<CommitOutcome> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: anyhow::Result<CommitOutcome> = async {
        // Claim the deterministic business identity before touching quota. If
        // another scanner already owns this intent, the duplicate-key error
        // aborts this transaction without consuming a reservation. This order
        // also keeps a same-intent loser classified as Duplicate after the
        // winner fills the last quota slot (rather than incorrectly Capped).
        let mut task = to_document(&build_task(intent, task_id))?;
        task.insert(INTENT_HASH_FIELD, intent_hash);
        state
            .db
            .tasks()
            .clone_with_type::<Document>()
            .insert_one_with_session(task, None, &mut session)
            .await?;

        if !ensure_and_reserve_quota(
            state,
            &intent.contact.workspace_id,
            intent.segment,
            intent.now,
            &intent.quota,
            &mut session,
        )
        .await?
        {
            return Ok(CommitOutcome::Capped);
        }

        let mut event = to_document(&build_event(intent, task_id))?;
        event.insert(INTENT_HASH_FIELD, intent_hash);
        state
            .db
            .events()
            .clone_with_type::<Document>()
            .insert_one_with_session(event, None, &mut session)
            .await?;
        Ok(CommitOutcome::Emitted)
    }
    .await;

    match result {
        Ok(CommitOutcome::Capped) => {
            let _ = session.abort_transaction().await;
            Ok(CommitOutcome::Capped)
        }
        Ok(outcome) => {
            commit_transaction(&mut session).await?;
            Ok(outcome)
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

async fn committed_identity_matches(
    state: &AppState,
    id: ObjectId,
    intent_hash: &str,
) -> anyhow::Result<bool> {
    // Both documents commit in one transaction, so inspect them through one
    // snapshot as well. Two independent reads could otherwise straddle the
    // winner's commit and manufacture a transient 1/2 observation even though
    // no partial commit ever existed.
    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(
            TransactionOptions::builder()
                .read_concern(ReadConcern::snapshot())
                .build(),
        )
        .await?;
    let result: anyhow::Result<u8> = async {
        let mut present = 0_u8;
        for collection in ["agent_tasks", "agent_events"] {
            if let Some(row) = state
                .db
                .raw()
                .collection::<Document>(collection)
                .find_one_with_session(doc! { "_id": id }, None, &mut session)
                .await?
            {
                let stored = row.get_str(INTENT_HASH_FIELD).unwrap_or("");
                if stored == intent_hash {
                    present += 1;
                    continue;
                }
                anyhow::bail!("proactive intent ObjectId collision in {collection}");
            }
        }
        Ok(present)
    }
    .await;
    let abort_result = session.abort_transaction().await;
    match result {
        Ok(present) => {
            abort_result?;
            classify_committed_identity_presence(present)
        }
        Err(error) => Err(error),
    }
}

fn classify_committed_identity_presence(present: u8) -> anyhow::Result<bool> {
    match present {
        0 => Ok(false),
        2 => Ok(true),
        _ => anyhow::bail!("proactive intent has a partial task/event commit"),
    }
}

fn retryable(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<mongodb::error::Error>() else {
        return false;
    };
    error.contains_label("TransientTransactionError") || is_duplicate_key_error(error)
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            is_duplicate_key_code(write_error.code)
        }
        ErrorKind::BulkWrite(bulk) => bulk.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|write_error| is_duplicate_key_code(write_error.code))
        }),
        // A transactional insert can surface an identity collision as a
        // command error in mongodb 2.8.x. It is the same OCC miss and remains
        // safe to retry because the deterministic committed identity is read
        // before every retry.
        ErrorKind::Command(command_error) => is_duplicate_key_code(command_error.code),
        _ => false,
    }
}

fn is_duplicate_key_code(code: i32) -> bool {
    matches!(code, 11000 | 11001)
}

fn retry_delay(attempt: usize) -> std::time::Duration {
    // Keep contention convergence bounded while allowing a winning
    // transaction enough time to become visible on a slower replica set.
    // Attempts 0..=6 grow from 5ms to 320ms; later attempts stay capped.
    std::time::Duration::from_millis(5_u64 << attempt.min(6))
}

async fn commit_transaction(session: &mut ClientSession) -> mongodb::error::Result<()> {
    loop {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => {
                let _ = session.abort_transaction().await;
                return Err(error);
            }
        }
    }
}

pub async fn commit_follow_up(
    state: &AppState,
    intent: FollowUpIntent,
) -> anyhow::Result<CommitOutcome> {
    validate_quota_request(&intent.contact.workspace_id, intent.segment, &intent.quota)?;
    validate_identity("account", &intent.contact.account_id)?;
    validate_identity("contact", &intent.contact.wxid)?;
    validate_identity("intent subject", &intent.subject)?;
    validate_identity("event kind", intent.event_kind)?;
    let (task_id, intent_hash) = intent_identity(&intent);
    if committed_identity_matches(state, task_id, &intent_hash).await? {
        return Ok(CommitOutcome::Duplicate);
    }

    for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
        match commit_follow_up_once(state, &intent, task_id, &intent_hash).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                if committed_identity_matches(state, task_id, &intent_hash).await? {
                    return Ok(CommitOutcome::Duplicate);
                }
                if attempt + 1 < MAX_TRANSACTION_ATTEMPTS && retryable(&error) {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
    unreachable!("bounded proactive commit loop always returns")
}

fn signal_identity(signal: &BehaviorSignal) -> (ObjectId, String) {
    let hash = sha256(&[
        "proactive-signal:v1",
        &signal.workspace_id,
        &signal.account_id,
        &signal.dedupe_key,
    ]);
    (object_id_from_hash(&hash), hex_hash(&hash))
}

async fn signal_exists(
    state: &AppState,
    signal: &BehaviorSignal,
    id: ObjectId,
    intent_hash: &str,
) -> anyhow::Result<bool> {
    let row = state
        .db
        .behavior_signals()
        .clone_with_type::<Document>()
        .find_one(
            doc! {
                "$or": [
                    { "_id": id },
                    {
                        "workspace_id": &signal.workspace_id,
                        "account_id": &signal.account_id,
                        "dedupe_key": &signal.dedupe_key,
                    }
                ]
            },
            None,
        )
        .await?;
    let Some(row) = row else { return Ok(false) };
    let same_business_identity = row.get_str("workspace_id").ok() == Some(&signal.workspace_id)
        && row.get_str("account_id").ok() == Some(&signal.account_id)
        && row.get_str("dedupe_key").ok() == Some(&signal.dedupe_key);
    if same_business_identity
        && row
            .get_str(INTENT_HASH_FIELD)
            .map(|stored| stored == intent_hash)
            .unwrap_or(true)
    {
        return Ok(true);
    }
    anyhow::bail!("proactive signal ObjectId collision")
}

async fn commit_signal_once(
    state: &AppState,
    mut signal: BehaviorSignal,
    segment: &'static str,
    quota: &DailyQuota,
    id: ObjectId,
    intent_hash: &str,
) -> anyhow::Result<CommitOutcome> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: anyhow::Result<CommitOutcome> = async {
        // As with follow-ups, acquire the deterministic signal identity before
        // quota so duplicate observations never consume or get misclassified
        // by a full daily bucket. A capped transaction aborts this insert.
        signal.id = Some(id);
        let mut row = to_document(&signal)?;
        row.insert(INTENT_HASH_FIELD, intent_hash);
        state
            .db
            .behavior_signals()
            .clone_with_type::<Document>()
            .insert_one_with_session(row, None, &mut session)
            .await?;

        if !ensure_and_reserve_quota(
            state,
            &signal.workspace_id,
            segment,
            signal.observed_at,
            quota,
            &mut session,
        )
        .await?
        {
            return Ok(CommitOutcome::Capped);
        }
        Ok(CommitOutcome::Emitted)
    }
    .await;
    match result {
        Ok(CommitOutcome::Capped) => {
            let _ = session.abort_transaction().await;
            Ok(CommitOutcome::Capped)
        }
        Ok(outcome) => {
            commit_transaction(&mut session).await?;
            Ok(outcome)
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

pub async fn commit_signal_with_daily_quota(
    state: &AppState,
    signal: BehaviorSignal,
    segment: &'static str,
    quota: DailyQuota,
) -> anyhow::Result<CommitOutcome> {
    validate_quota_request(&signal.workspace_id, segment, &quota)?;
    validate_identity("account", &signal.account_id)?;
    validate_identity("contact", &signal.contact_wxid)?;
    validate_identity("signal dedupe key", &signal.dedupe_key)?;
    validate_identity("signal type", &signal.signal_type)?;
    let (id, intent_hash) = signal_identity(&signal);
    if signal_exists(state, &signal, id, &intent_hash).await? {
        return Ok(CommitOutcome::Duplicate);
    }
    for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
        match commit_signal_once(state, signal.clone(), segment, &quota, id, &intent_hash).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                if signal_exists(state, &signal, id, &intent_hash).await? {
                    return Ok(CommitOutcome::Duplicate);
                }
                if attempt + 1 < MAX_TRANSACTION_ATTEMPTS && retryable(&error) {
                    tokio::time::sleep(retry_delay(attempt)).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
    unreachable!("bounded proactive signal commit loop always returns")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_id_is_stable_and_subject_sensitive() {
        let now = DateTime::from_millis(1_700_000_000_000);
        let mut contact: Contact = mongodb::bson::from_document(doc! {
            "workspace_id": "ws",
            "account_id": "acc",
            "wxid": "wxid",
            "agent_status": "managed",
            "operation_policy": {},
            "profile_attributes": {},
            "created_at": now,
            "updated_at": now,
        })
        .expect("minimal contact");
        contact.last_inbound_at = Some(now);
        let make = |subject: &str| FollowUpIntent {
            contact: contact.clone(),
            segment: "silent",
            subject: subject.to_string(),
            content: "AI decides later".to_string(),
            event_kind: "strategic_planner_emit",
            event_summary: "emitted".to_string(),
            event_details: Document::new(),
            now,
            quota: DailyQuota {
                namespace: "planner",
                account_scope: Some("acc".to_string()),
                total_cap: 20,
                segment_cap: None,
                initial_total: 0,
                initial_segment: 0,
            },
        };
        assert_eq!(
            intent_identity(&make("generation-1")),
            intent_identity(&make("generation-1"))
        );
        assert_ne!(
            intent_identity(&make("generation-1")),
            intent_identity(&make("generation-2"))
        );
    }

    #[test]
    fn quota_bucket_is_day_and_scope_sensitive() {
        let quota = DailyQuota {
            namespace: "planner",
            account_scope: Some("a".to_string()),
            total_cap: 1,
            segment_cap: None,
            initial_total: 0,
            initial_segment: 0,
        };
        let day = DateTime::from_millis(86_400_000);
        assert_eq!(quota_id("ws", &quota, day), quota_id("ws", &quota, day));
        assert_ne!(
            quota_id("ws", &quota, day),
            quota_id("ws", &quota, DateTime::from_millis(2 * 86_400_000))
        );
        let mut other = quota.clone();
        other.account_scope = Some("b".to_string());
        assert_ne!(quota_id("ws", &quota, day), quota_id("ws", &other, day));
    }

    #[test]
    fn quota_identity_tokens_reject_ambiguous_or_unsafe_fields() {
        let quota = DailyQuota {
            namespace: "planner",
            account_scope: Some("account-a".to_string()),
            total_cap: 1,
            segment_cap: None,
            initial_total: 0,
            initial_segment: 0,
        };
        assert!(validate_quota_request("ws", "calendar", &quota).is_ok());
        assert!(validate_quota_request("ws", "calendar.care", &quota).is_err());
        assert!(validate_quota_request("ws", "$calendar", &quota).is_err());
        assert!(validate_quota_request("ws", " calendar", &quota).is_err());

        let mut punctuation_scope = quota.clone();
        punctuation_scope.account_scope = Some("account.$legacy".to_string());
        assert!(
            validate_quota_request("workspace.$legacy", "calendar", &punctuation_scope).is_ok()
        );

        let mut invalid_scope = quota.clone();
        invalid_scope.account_scope = Some("".to_string());
        assert!(validate_quota_request("ws", "calendar", &invalid_scope).is_err());
        assert!(validate_quota_request("", "calendar", &quota).is_err());
        assert!(validate_quota_request(" ws", "calendar", &quota).is_err());

        let invalid_namespace = DailyQuota {
            namespace: "planner.daily",
            ..quota
        };
        assert!(validate_quota_request("ws", "calendar", &invalid_namespace).is_err());
    }

    #[test]
    fn ordinary_identity_values_allow_punctuation_but_reject_ambiguous_forms() {
        assert!(validate_identity("account", "account.$legacy").is_ok());
        assert!(validate_identity("contact", "wxid.with-punctuation").is_ok());
        assert!(validate_identity("contact", "").is_err());
        assert!(validate_identity("contact", " wxid").is_err());
        assert!(validate_identity("contact", "wxid\0suffix").is_err());
    }

    #[test]
    fn duplicate_key_codes_are_narrowly_classified() {
        assert!(is_duplicate_key_code(11000));
        assert!(is_duplicate_key_code(11001));
        assert!(!is_duplicate_key_code(10999));
        assert!(!is_duplicate_key_code(112));
    }

    #[test]
    fn committed_identity_presence_is_fail_closed() {
        assert!(!classify_committed_identity_presence(0).unwrap());
        assert!(classify_committed_identity_presence(2).unwrap());
        assert!(classify_committed_identity_presence(1).is_err());
        assert!(classify_committed_identity_presence(3).is_err());
    }

    #[test]
    fn retry_delay_grows_and_caps_without_unbounded_waits() {
        let millis = (0..9)
            .map(|attempt| retry_delay(attempt).as_millis())
            .collect::<Vec<_>>();
        assert_eq!(millis, vec![5, 10, 20, 40, 80, 160, 320, 320, 320]);
    }
}
