//! Shared authentication rate limit and privacy-preserving failure audit.
//!
//! `/auth/login` and `/auth/token` use the same limiter. Each attempt reserves
//! capacity for both the direct peer and the normalized target before Argon2 is
//! invoked. Only salted fingerprints are retained or persisted.

use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use mongodb::bson::{doc, DateTime, Document};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::db::Database;

const MAX_TRACKED_AUTH_ATTEMPTS: usize = 100_000;
const MAX_TRACKED_REJECTION_AUDITS: usize = 100_000;
const AUTH_AUDIT_RETENTION_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthAuditSubject {
    pub client_fingerprint: String,
    pub target_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRateLimitExceeded {
    pub retry_after_seconds: u64,
    pub dimension: &'static str,
    pub subject: AuthAuditSubject,
    /// True only for the first equivalent rejection in the active window.
    pub should_audit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptState {
    Pending,
    Failed,
}

#[derive(Debug, Clone)]
struct Attempt {
    subject: AuthAuditSubject,
    started_at: Instant,
    state: AttemptState,
}

#[derive(Debug, Default)]
struct LimiterInner {
    next_id: u64,
    attempts: HashMap<u64, Attempt>,
    rejection_audits: HashMap<String, Instant>,
}

#[derive(Debug)]
pub struct AuthRateLimiter {
    window: Duration,
    client_capacity: usize,
    target_capacity: usize,
    global_capacity: usize,
    process_salt: [u8; 16],
    inner: Mutex<LimiterInner>,
}

impl AuthRateLimiter {
    pub fn new(
        window_seconds: u64,
        client_capacity: u32,
        target_capacity: u32,
        global_capacity: u32,
    ) -> Self {
        let process_salt = *uuid::Uuid::new_v4().as_bytes();
        Self {
            window: Duration::from_secs(window_seconds.max(1)),
            client_capacity: client_capacity.max(1) as usize,
            target_capacity: target_capacity.max(1) as usize,
            global_capacity: (global_capacity.max(1) as usize).min(MAX_TRACKED_AUTH_ATTEMPTS),
            process_salt,
            inner: Mutex::new(LimiterInner::default()),
        }
    }

    /// Atomically reserve both dimensions before password hashing starts.
    pub fn begin(
        self: &Arc<Self>,
        client_identity: &str,
        target: &str,
    ) -> Result<AuthAttemptPermit, AuthRateLimitExceeded> {
        self.begin_at(client_identity, target, Instant::now())
    }

    fn begin_at(
        self: &Arc<Self>,
        client_identity: &str,
        target: &str,
        now: Instant,
    ) -> Result<AuthAttemptPermit, AuthRateLimitExceeded> {
        let subject = AuthAuditSubject {
            client_fingerprint: self.fingerprint("client", client_identity),
            target_fingerprint: self
                .fingerprint("target", target.trim().to_ascii_lowercase().as_str()),
        };
        let mut inner = self.inner.lock();
        inner
            .attempts
            .retain(|_, attempt| now.saturating_duration_since(attempt.started_at) < self.window);
        inner
            .rejection_audits
            .retain(|_, started_at| now.saturating_duration_since(*started_at) < self.window);

        let client_count = inner
            .attempts
            .values()
            .filter(|attempt| attempt.subject.client_fingerprint == subject.client_fingerprint)
            .count();
        let target_count = inner
            .attempts
            .values()
            .filter(|attempt| attempt.subject.target_fingerprint == subject.target_fingerprint)
            .count();
        let client_full = client_count >= self.client_capacity;
        let target_full = target_count >= self.target_capacity;
        // The global cap protects Argon2 concurrency only. Retained failures continue to
        // enforce per-client/per-target limits, but random failed identities must not fill a
        // process-wide denial slot and lock every administrator out.
        let global_full = inner
            .attempts
            .values()
            .filter(|attempt| attempt.state == AttemptState::Pending)
            .count()
            >= self.global_capacity;
        if client_full || target_full || global_full {
            let earliest = inner
                .attempts
                .values()
                .filter(|attempt| {
                    (global_full && attempt.state == AttemptState::Pending)
                        || (client_full
                            && attempt.subject.client_fingerprint == subject.client_fingerprint)
                        || (target_full
                            && attempt.subject.target_fingerprint == subject.target_fingerprint)
                })
                .map(|attempt| attempt.started_at)
                .min()
                .unwrap_or(now);
            let elapsed = now.saturating_duration_since(earliest);
            let retry_after_seconds = self
                .window
                .saturating_sub(elapsed)
                .as_secs()
                .saturating_add(1)
                .max(1);
            let dimension = if global_full {
                "global"
            } else if client_full && target_full {
                "client_and_target"
            } else if client_full {
                "client"
            } else {
                "target"
            };
            let audit_key = rejection_audit_key(dimension, &subject);
            let should_audit = if inner.rejection_audits.contains_key(&audit_key) {
                false
            } else if inner.rejection_audits.len() >= MAX_TRACKED_REJECTION_AUDITS {
                false
            } else {
                inner.rejection_audits.insert(audit_key, now);
                true
            };
            return Err(AuthRateLimitExceeded {
                retry_after_seconds,
                dimension,
                subject,
                should_audit,
            });
        }

        inner.next_id = inner.next_id.wrapping_add(1).max(1);
        let id = inner.next_id;
        inner.attempts.insert(
            id,
            Attempt {
                subject: subject.clone(),
                started_at: now,
                state: AttemptState::Pending,
            },
        );
        Ok(AuthAttemptPermit {
            limiter: Arc::downgrade(self),
            id,
            subject,
            finished: false,
        })
    }

    fn fingerprint(&self, namespace: &str, value: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.process_salt);
        hasher.update(namespace.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn mark_failed(&self, id: u64) {
        if let Some(attempt) = self.inner.lock().attempts.get_mut(&id) {
            attempt.state = AttemptState::Failed;
        }
    }

    fn mark_success(&self, id: u64) {
        let mut inner = self.inner.lock();
        let Some(current) = inner.attempts.get(&id).cloned() else {
            return;
        };
        inner.attempts.retain(|attempt_id, attempt| {
            if *attempt_id == id {
                return false;
            }
            !(attempt.state == AttemptState::Failed && attempt.subject == current.subject)
        });
    }

    fn cancel(&self, id: u64) {
        self.inner.lock().attempts.remove(&id);
    }
}

fn rejection_audit_key(dimension: &str, subject: &AuthAuditSubject) -> String {
    match dimension {
        "global" => "global".to_string(),
        "client" => format!("client:{}", subject.client_fingerprint),
        "target" => format!("target:{}", subject.target_fingerprint),
        _ => format!(
            "client_and_target:{}:{}",
            subject.client_fingerprint, subject.target_fingerprint
        ),
    }
}

/// Reservation whose Drop path releases a pending slot on cancellation/panic.
#[derive(Debug)]
pub struct AuthAttemptPermit {
    limiter: Weak<AuthRateLimiter>,
    id: u64,
    subject: AuthAuditSubject,
    finished: bool,
}

impl AuthAttemptPermit {
    pub fn audit_subject(&self) -> AuthAuditSubject {
        self.subject.clone()
    }

    pub fn mark_invalid(mut self) {
        if let Some(limiter) = self.limiter.upgrade() {
            limiter.mark_failed(self.id);
        }
        self.finished = true;
    }

    pub fn mark_success(mut self) {
        if let Some(limiter) = self.limiter.upgrade() {
            limiter.mark_success(self.id);
        }
        self.finished = true;
    }
}

impl Drop for AuthAttemptPermit {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(limiter) = self.limiter.upgrade() {
                limiter.cancel(self.id);
            }
        }
    }
}

/// Persist a failure without raw usernames, addresses, passwords, or tokens.
pub async fn write_auth_failure_audit(
    db: &Database,
    entrypoint: &str,
    outcome: &str,
    subject: &AuthAuditSubject,
    dimension: Option<&str>,
    retry_after_seconds: Option<u64>,
) -> mongodb::error::Result<()> {
    let event = build_auth_failure_audit_document(
        entrypoint,
        outcome,
        subject,
        dimension,
        retry_after_seconds,
    );
    db.raw()
        .collection::<Document>("auth_security_events")
        .insert_one(event, None)
        .await?;
    Ok(())
}

fn build_auth_failure_audit_document(
    entrypoint: &str,
    outcome: &str,
    subject: &AuthAuditSubject,
    dimension: Option<&str>,
    retry_after_seconds: Option<u64>,
) -> Document {
    let created_at = DateTime::now();
    let expires_at = DateTime::from_millis(
        created_at
            .timestamp_millis()
            .saturating_add(AUTH_AUDIT_RETENTION_DAYS * 24 * 60 * 60 * 1000),
    );
    let mut event = doc! {
        "event_id": uuid::Uuid::new_v4().to_string(),
        "entrypoint": entrypoint,
        "outcome": outcome,
        "client_fingerprint": &subject.client_fingerprint,
        "target_fingerprint": &subject.target_fingerprint,
        "fingerprint_scheme": "sha256-process-salt-v1",
        "created_at": created_at,
        "expires_at": expires_at,
    };
    if let Some(dimension) = dimension {
        event.insert("limit_dimension", dimension);
    }
    if let Some(retry_after_seconds) = retry_after_seconds {
        event.insert("retry_after_seconds", retry_after_seconds as i64);
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(client_capacity: u32, target_capacity: u32) -> Arc<AuthRateLimiter> {
        Arc::new(AuthRateLimiter::new(
            300,
            client_capacity,
            target_capacity,
            100,
        ))
    }

    #[test]
    fn target_limit_is_shared_across_clients_and_entrypoints() {
        let limiter = limiter(10, 2);
        limiter.begin("10.0.0.1", "Alice").unwrap().mark_invalid();
        limiter.begin("10.0.0.2", "alice").unwrap().mark_invalid();
        let denied = limiter.begin("10.0.0.3", "ALICE").unwrap_err();
        assert_eq!(denied.dimension, "target");
    }

    #[test]
    fn client_limit_is_shared_across_targets() {
        let limiter = limiter(2, 10);
        limiter.begin("10.0.0.1", "alice").unwrap().mark_invalid();
        limiter.begin("10.0.0.1", "bob").unwrap().mark_invalid();
        let denied = limiter.begin("10.0.0.1", "carol").unwrap_err();
        assert_eq!(denied.dimension, "client");
    }

    #[test]
    fn success_clears_failed_pair_without_clearing_other_targets() {
        let limiter = limiter(3, 3);
        limiter.begin("10.0.0.1", "alice").unwrap().mark_invalid();
        limiter.begin("10.0.0.1", "bob").unwrap().mark_invalid();
        limiter.begin("10.0.0.1", "alice").unwrap().mark_success();
        limiter.begin("10.0.0.1", "alice").unwrap().mark_invalid();
        limiter.begin("10.0.0.1", "carol").unwrap().mark_invalid();
        assert!(limiter.begin("10.0.0.1", "dave").is_err());
    }

    #[test]
    fn dropped_pending_attempt_releases_capacity() {
        let limiter = limiter(1, 1);
        drop(limiter.begin("10.0.0.1", "alice").unwrap());
        assert!(limiter.begin("10.0.0.1", "alice").is_ok());
    }

    #[test]
    fn historical_failures_do_not_fill_the_global_concurrency_cap() {
        let limiter = Arc::new(AuthRateLimiter::new(300, 10, 10, 2));
        limiter.begin("10.0.0.1", "alice").unwrap().mark_invalid();
        limiter.begin("10.0.0.2", "bob").unwrap().mark_invalid();
        assert!(limiter.begin("10.0.0.3", "carol").is_ok());
    }

    #[test]
    fn global_limit_still_caps_concurrent_pending_hashes() {
        let limiter = Arc::new(AuthRateLimiter::new(300, 10, 10, 2));
        let _first = limiter.begin("10.0.0.1", "alice").unwrap();
        let _second = limiter.begin("10.0.0.2", "bob").unwrap();
        let denied = limiter.begin("10.0.0.3", "carol").unwrap_err();
        assert_eq!(denied.dimension, "global");
        assert!(denied.should_audit);
    }

    #[test]
    fn equivalent_rejections_are_audited_once_per_window() {
        let limiter = limiter(1, 10);
        limiter.begin("10.0.0.1", "alice").unwrap().mark_invalid();
        let first = limiter.begin("10.0.0.1", "bob").unwrap_err();
        let second = limiter.begin("10.0.0.1", "carol").unwrap_err();
        assert_eq!(first.dimension, "client");
        assert!(first.should_audit);
        assert!(!second.should_audit);
    }

    #[test]
    fn fingerprints_are_stable_but_do_not_retain_raw_identifiers() {
        let limiter = limiter(2, 2);
        let first = limiter.begin("203.0.113.7", "Alice").unwrap();
        let first_subject = first.audit_subject();
        drop(first);
        let second = limiter.begin("203.0.113.7", "alice").unwrap();
        let second_subject = second.audit_subject();
        assert_eq!(first_subject, second_subject);
        assert!(!first_subject.client_fingerprint.contains("203.0.113.7"));
        assert!(!first_subject.target_fingerprint.contains("alice"));
    }

    #[test]
    fn audit_document_contains_no_raw_identity_or_secret_fields() {
        let limiter = limiter(2, 2);
        let permit = limiter.begin("203.0.113.7", "alice@example.test").unwrap();
        let subject = permit.audit_subject();
        let document =
            build_auth_failure_audit_document("login", "invalid_credentials", &subject, None, None);
        let rendered = format!("{document:?}");
        assert!(!rendered.contains("203.0.113.7"));
        assert!(!rendered.contains("alice@example.test"));
        assert!(!document.contains_key("username"));
        assert!(!document.contains_key("password"));
        assert!(!document.contains_key("ip"));
        let created_at = document.get_datetime("created_at").unwrap();
        let expires_at = document.get_datetime("expires_at").unwrap();
        assert_eq!(
            expires_at.timestamp_millis() - created_at.timestamp_millis(),
            AUTH_AUDIT_RETENTION_DAYS * 24 * 60 * 60 * 1000
        );
    }
}
