//! Gateway run-local performance and LLM-call audit buffering.
//!
//! Production gateways install one [`RunAuditBuffer`] task-local. LLM calls
//! enqueue complete [`LlmCallLog`] rows in memory and the gateway settle path
//! flushes them with one `insert_many`. Calls outside a gateway keep their
//! immediate-write behavior.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use mongodb::{
    bson::{doc, oid::ObjectId, Document},
    options::ReplaceOptions,
};
use parking_lot::Mutex;

use crate::{
    models::{AgentEvent, LlmCallLog},
    routes::AppState,
};

#[derive(Debug, Clone, Default)]
struct StageTiming {
    count: i64,
    total_ms: i64,
    max_ms: i64,
}

#[derive(Debug, Clone, Default)]
struct RunPathMetadata {
    tier: Option<String>,
    rewrite: bool,
    revision: bool,
    no_reply: bool,
    manual: bool,
}

impl RunPathMetadata {
    fn kind(&self) -> &'static str {
        if self.manual {
            "manual"
        } else if self.no_reply {
            "no_reply"
        } else if self.revision {
            "revision"
        } else if self.rewrite {
            "rewrite"
        } else if self.tier.as_deref().is_some_and(|tier| tier != "lean") {
            "escalated"
        } else {
            "direct"
        }
    }

    fn to_document(&self) -> Document {
        let mut path = doc! {
            "kind": self.kind(),
            "rewrite": self.rewrite,
            "revision": self.revision,
            "noReply": self.no_reply,
            "manual": self.manual,
        };
        if let Some(tier) = &self.tier {
            path.insert("tier", tier);
        }
        path
    }
}

/// Reliable flush outcome persisted into `agent_run_logs.gateway_result.performance`.
#[derive(Debug, Clone, Default)]
pub(crate) struct LlmAuditFlushReport {
    pub queued: i64,
    pub persisted: i64,
    pub batch_succeeded: bool,
    pub fallback_used: bool,
    pub failed: i64,
    pub error: Option<String>,
    pub latency_ms: i64,
}

impl LlmAuditFlushReport {
    pub(crate) fn to_document(&self) -> Document {
        let mut doc = doc! {
            "queued": self.queued,
            "persisted": self.persisted,
            "batchSucceeded": self.batch_succeeded,
            "fallbackUsed": self.fallback_used,
            "failed": self.failed,
            "latencyMs": self.latency_ms,
        };
        if let Some(error) = &self.error {
            doc.insert("error", error.chars().take(512).collect::<String>());
        }
        doc
    }
}

/// Per-gateway mutable audit state. All locks are short, synchronous sections;
/// no guard is held across an `.await`.
pub(crate) struct RunAuditBuffer {
    started_at: Instant,
    llm_logs: Mutex<Vec<LlmCallLog>>,
    observability_events: Mutex<Vec<AgentEvent>>,
    stages: Mutex<BTreeMap<String, StageTiming>>,
    path: Mutex<RunPathMetadata>,
}

impl RunAuditBuffer {
    pub(crate) fn new() -> Self {
        Self {
            started_at: Instant::now(),
            llm_logs: Mutex::new(Vec::new()),
            observability_events: Mutex::new(Vec::new()),
            stages: Mutex::new(BTreeMap::new()),
            path: Mutex::new(RunPathMetadata::default()),
        }
    }

    fn push_llm_log(&self, mut log: LlmCallLog) {
        // Stable ids make an insert_many partial success recoverable through
        // idempotent replace_one(upsert=true) calls.
        if log.id.is_none() {
            log.id = Some(ObjectId::new());
        }
        self.llm_logs.lock().push(log);
    }

    fn push_observability_event(&self, mut event: AgentEvent) {
        if event.id.is_none() {
            event.id = Some(ObjectId::new());
        }
        self.observability_events.lock().push(event);
    }

    pub(crate) fn set_tier(&self, tier: &str) {
        self.path.lock().tier = Some(tier.to_string());
    }

    pub(crate) fn mark_rewrite(&self) {
        self.path.lock().rewrite = true;
    }

    pub(crate) fn mark_revision(&self) {
        self.path.lock().revision = true;
    }

    pub(crate) fn mark_no_reply(&self) {
        self.path.lock().no_reply = true;
    }

    pub(crate) fn mark_manual(&self) {
        self.path.lock().manual = true;
    }

    pub(crate) fn record_stage(&self, name: &str, elapsed: Duration) {
        let elapsed_ms = elapsed.as_millis().min(i64::MAX as u128) as i64;
        let mut stages = self.stages.lock();
        let entry = stages.entry(name.to_string()).or_default();
        entry.count += 1;
        entry.total_ms = entry.total_ms.saturating_add(elapsed_ms);
        entry.max_ms = entry.max_ms.max(elapsed_ms);
    }

    pub(crate) fn performance_document(
        &self,
        llm_flush: &LlmAuditFlushReport,
        event_flush: &LlmAuditFlushReport,
    ) -> Document {
        let stages = self.stages.lock();
        let mut stage_doc = Document::new();
        for (name, timing) in stages.iter() {
            stage_doc.insert(
                name,
                doc! {
                    "count": timing.count,
                    "totalMs": timing.total_ms,
                    "maxMs": timing.max_ms,
                },
            );
        }
        doc! {
            "totalMs": self.started_at.elapsed().as_millis().min(i64::MAX as u128) as i64,
            "path": self.path.lock().to_document(),
            "stages": stage_doc,
            "llmLogFlush": llm_flush.to_document(),
            "eventLogFlush": event_flush.to_document(),
        }
    }

    /// Flush buffered rows. `insert_many` is the fast path. If it reports any
    /// error (including partial success), every row is replayed by stable `_id`
    /// with upsert so already-inserted rows are replaced and missing rows are
    /// inserted without duplication.
    pub(crate) async fn flush_llm_logs(&self, state: &AppState) -> LlmAuditFlushReport {
        let started = Instant::now();
        let logs = std::mem::take(&mut *self.llm_logs.lock());
        let queued = logs.len() as i64;
        if logs.is_empty() {
            return LlmAuditFlushReport {
                batch_succeeded: true,
                latency_ms: started.elapsed().as_millis() as i64,
                ..Default::default()
            };
        }

        if state
            .db
            .llm_call_logs()
            .insert_many(logs.clone(), None)
            .await
            .is_ok()
        {
            return LlmAuditFlushReport {
                queued,
                persisted: queued,
                batch_succeeded: true,
                latency_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
                ..Default::default()
            };
        }

        let options = ReplaceOptions::builder().upsert(true).build();
        let mut persisted = 0_i64;
        let mut errors = Vec::new();
        for log in logs {
            let Some(id) = log.id else {
                errors.push("buffered LLM log missing stable _id".to_string());
                continue;
            };
            match state
                .db
                .llm_call_logs()
                .replace_one(doc! { "_id": id }, log, options.clone())
                .await
            {
                Ok(_) => persisted += 1,
                Err(error) => errors.push(error.to_string()),
            }
        }
        let failed = queued.saturating_sub(persisted);
        LlmAuditFlushReport {
            queued,
            persisted,
            batch_succeeded: false,
            fallback_used: true,
            failed,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
            latency_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
        }
    }

    pub(crate) async fn flush_observability_events(&self, state: &AppState) -> LlmAuditFlushReport {
        let started = Instant::now();
        let events = std::mem::take(&mut *self.observability_events.lock());
        let queued = events.len() as i64;
        if events.is_empty() {
            return LlmAuditFlushReport {
                batch_succeeded: true,
                latency_ms: started.elapsed().as_millis() as i64,
                ..Default::default()
            };
        }
        if state
            .db
            .events()
            .insert_many(events.clone(), None)
            .await
            .is_ok()
        {
            return LlmAuditFlushReport {
                queued,
                persisted: queued,
                batch_succeeded: true,
                latency_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
                ..Default::default()
            };
        }
        let options = ReplaceOptions::builder().upsert(true).build();
        let mut persisted = 0_i64;
        let mut errors = Vec::new();
        for event in events {
            let Some(id) = event.id else {
                errors.push("buffered observability event missing stable _id".to_string());
                continue;
            };
            match state
                .db
                .events()
                .replace_one(doc! { "_id": id }, event, options.clone())
                .await
            {
                Ok(_) => persisted += 1,
                Err(error) => errors.push(error.to_string()),
            }
        }
        let failed = queued.saturating_sub(persisted);
        LlmAuditFlushReport {
            queued,
            persisted,
            batch_succeeded: false,
            fallback_used: true,
            failed,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
            latency_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
        }
    }
}

tokio::task_local! {
    pub(crate) static RUN_AUDIT_BUFFER: Arc<RunAuditBuffer>;
}

/// Queue a log when inside a production gateway. Returns `Err(log)` outside a
/// gateway so the caller can preserve the existing immediate insert behavior.
pub(crate) fn try_buffer_llm_log(log: LlmCallLog) -> Result<(), LlmCallLog> {
    match RUN_AUDIT_BUFFER.try_with(Arc::clone) {
        Ok(buffer) => {
            buffer.push_llm_log(log);
            Ok(())
        }
        Err(_) => Err(log),
    }
}

const BUFFERED_OBSERVABILITY_EVENT_KINDS: &[&str] = &[
    "ptier_self_assessment_malformed",
    "ptier_forced_full",
    "ptier_coverage_optimism",
    "ptier_relational_optimism",
    "ptier_escalated",
    "ptier_clarify",
    "ptier_run_tier",
];

pub(crate) fn try_buffer_observability_event(event: AgentEvent) -> Result<(), AgentEvent> {
    if !BUFFERED_OBSERVABILITY_EVENT_KINDS.contains(&event.kind.as_str()) {
        return Err(event);
    }
    match RUN_AUDIT_BUFFER.try_with(Arc::clone) {
        Ok(buffer) => {
            buffer.push_observability_event(event);
            Ok(())
        }
        Err(_) => Err(event),
    }
}

fn with_current_audit(action: impl FnOnce(&RunAuditBuffer)) {
    if let Ok(audit) = RUN_AUDIT_BUFFER.try_with(Arc::clone) {
        action(&audit);
    }
}

pub(crate) fn mark_tier(tier: &str) {
    with_current_audit(|audit| audit.set_tier(tier));
}

pub(crate) fn mark_rewrite() {
    with_current_audit(RunAuditBuffer::mark_rewrite);
}

pub(crate) fn mark_revision() {
    with_current_audit(RunAuditBuffer::mark_revision);
}

pub(crate) fn mark_no_reply() {
    with_current_audit(RunAuditBuffer::mark_no_reply);
}

pub(crate) fn mark_manual() {
    with_current_audit(RunAuditBuffer::mark_manual);
}

/// RAII timer that records on normal return, error propagation, and unwind.
pub(crate) struct RunStageTimer {
    audit: Option<Arc<RunAuditBuffer>>,
    name: &'static str,
    started_at: Instant,
}

impl Drop for RunStageTimer {
    fn drop(&mut self) {
        if let Some(audit) = &self.audit {
            audit.record_stage(self.name, self.started_at.elapsed());
        }
    }
}

pub(crate) fn stage_timer(name: &'static str) -> RunStageTimer {
    RunStageTimer {
        audit: RUN_AUDIT_BUFFER.try_with(Arc::clone).ok(),
        name,
        started_at: Instant::now(),
    }
}

pub(crate) fn record_llm_queue_wait(
    priority: crate::llm_concurrency::LlmPriority,
    elapsed: Duration,
) {
    with_current_audit(|audit| {
        audit.record_stage("llm_queue_wait", elapsed);
        audit.record_stage(
            match priority {
                crate::llm_concurrency::LlmPriority::Foreground => "llm_queue_wait_foreground",
                crate::llm_concurrency::LlmPriority::Background => "llm_queue_wait_background",
            },
            elapsed,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::{
        try_buffer_llm_log, try_buffer_observability_event, LlmAuditFlushReport, RunAuditBuffer,
        RUN_AUDIT_BUFFER,
    };
    use crate::models::{AgentEvent, LlmCallLog};
    use mongodb::bson::DateTime;
    use std::{sync::Arc, time::Duration};

    fn sample_log() -> LlmCallLog {
        LlmCallLog {
            id: None,
            workspace_id: "ws".to_string(),
            account_id: Some("account".to_string()),
            contact_wxid: Some("wxid".to_string()),
            run_id: Some("run".to_string()),
            run_mode: "live".to_string(),
            prompt_key: "user.reply.task".to_string(),
            model: "test".to_string(),
            status: "success".to_string(),
            latency_ms: 1,
            queue_wait_ms: 0,
            provider_latency_ms: 0,
            priority: "foreground".to_string(),
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
            prompt_cache_hit_tokens: 0,
            prompt_cache_miss_tokens: 1,
            usage_known: true,
            error: None,
            retry_count: 0,
            final_status: Some("success".to_string()),
            created_at: DateTime::now(),
        }
    }

    #[test]
    fn stage_timings_aggregate_count_total_and_max() {
        let audit = RunAuditBuffer::new();
        audit.record_stage("reply", Duration::from_millis(7));
        audit.record_stage("reply", Duration::from_millis(11));
        let doc = audit.performance_document(
            &LlmAuditFlushReport::default(),
            &LlmAuditFlushReport::default(),
        );
        let reply = doc
            .get_document("stages")
            .unwrap()
            .get_document("reply")
            .unwrap();
        assert_eq!(reply.get_i64("count").unwrap(), 2);
        assert_eq!(reply.get_i64("totalMs").unwrap(), 18);
        assert_eq!(reply.get_i64("maxMs").unwrap(), 11);
    }

    #[test]
    fn path_classification_has_stable_precedence() {
        let audit = RunAuditBuffer::new();
        audit.set_tier("full");
        assert_eq!(audit.path.lock().kind(), "escalated");
        audit.mark_rewrite();
        assert_eq!(audit.path.lock().kind(), "rewrite");
        audit.mark_revision();
        assert_eq!(audit.path.lock().kind(), "revision");
        audit.mark_no_reply();
        assert_eq!(audit.path.lock().kind(), "no_reply");
        audit.mark_manual();
        assert_eq!(audit.path.lock().kind(), "manual");
    }

    fn sample_event(kind: &str) -> AgentEvent {
        AgentEvent {
            id: None,
            workspace_id: "ws".to_string(),
            account_id: "account".to_string(),
            contact_wxid: Some("wxid".to_string()),
            kind: kind.to_string(),
            status: "info".to_string(),
            summary: "summary".to_string(),
            details: None,
            created_at: DateTime::now(),
            dedupe_key: None,
        }
    }

    #[tokio::test]
    async fn only_ptier_observability_events_buffer_inside_gateway_scope() {
        assert!(try_buffer_observability_event(sample_event("ptier_run_tier")).is_err());
        let audit = Arc::new(RunAuditBuffer::new());
        RUN_AUDIT_BUFFER
            .scope(audit.clone(), async {
                assert!(try_buffer_observability_event(sample_event("ptier_run_tier")).is_ok());
                assert!(try_buffer_observability_event(sample_event("blocked_review")).is_err());
                assert!(
                    try_buffer_observability_event(sample_event("run_budget_exceeded")).is_err()
                );
            })
            .await;
        let events = audit.observability_events.lock();
        assert_eq!(events.len(), 1);
        assert!(events[0].id.is_some());
    }

    #[tokio::test]
    async fn llm_log_buffers_only_inside_gateway_scope_and_assigns_stable_id() {
        let outside = sample_log();
        assert!(try_buffer_llm_log(outside).is_err());

        let audit = Arc::new(RunAuditBuffer::new());
        RUN_AUDIT_BUFFER
            .scope(audit.clone(), async {
                assert!(try_buffer_llm_log(sample_log()).is_ok());
            })
            .await;

        let logs = audit.llm_logs.lock();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].id.is_some());
    }
}
