//! Typed evidence for real-model capability tests.
//!
//! A case starts as inconclusive and writes exactly one terminal JSON document on drop.
//! Call `pass` only after the target artifact exists and its assertions have executed.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

#[derive(Debug)]
pub struct CapabilityEvidence {
    case_id: &'static str,
    attempted: bool,
    llm_calls: usize,
    branch: String,
    artifacts: usize,
    assertions_run: usize,
    verdict: &'static str,
    reason: String,
    details: Map<String, Value>,
}

impl CapabilityEvidence {
    pub fn new(case_id: &'static str) -> Self {
        assert!(
            case_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "capability evidence case id must be filename-safe: {case_id}"
        );
        Self {
            case_id,
            attempted: false,
            llm_calls: 0,
            branch: String::new(),
            artifacts: 0,
            assertions_run: 0,
            verdict: "inconclusive",
            reason: "case exited before positive evidence was committed".to_string(),
            details: Map::new(),
        }
    }

    pub fn attempted(&mut self) {
        self.attempted = true;
    }

    pub fn observe_llm_calls(&mut self, count: usize) {
        self.llm_calls += count;
    }

    pub fn branch(&mut self, branch: impl Into<String>) {
        self.branch = branch.into();
    }

    pub fn detail(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.details.insert(key.into(), value.into());
    }

    pub fn inconclusive(&mut self, reason: impl Into<String>) {
        self.verdict = "inconclusive";
        self.reason = reason.into();
    }

    pub fn infra_skip(&mut self, reason: impl Into<String>) {
        self.verdict = "infra_skip";
        self.reason = reason.into();
    }

    pub fn pass(&mut self, artifacts: usize, assertions_run: usize) {
        assert!(self.attempted, "pass requires attempted=true");
        assert!(
            self.llm_calls > 0,
            "pass requires at least one observed LLM call"
        );
        assert!(
            !self.branch.trim().is_empty(),
            "pass requires a branch witness"
        );
        assert!(
            artifacts > 0,
            "pass requires at least one capability artifact"
        );
        assert!(
            assertions_run > 0,
            "pass requires at least one executed assertion"
        );
        self.artifacts = artifacts;
        self.assertions_run = assertions_run;
        self.verdict = "pass";
        self.reason.clear();
    }

    fn path(&self) -> PathBuf {
        let dir = std::env::var("REAL_LLM_LEDGER")
            .unwrap_or_else(|_| "target/real_llm_ledger".to_string());
        PathBuf::from(dir).join(format!("capability_outcome.{}.json", self.case_id))
    }

    fn persist(&self) -> std::io::Result<()> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_vec_pretty(&json!({
            "schema": "real_llm_capability_outcome/v1",
            "case_id": self.case_id,
            "attempted": self.attempted,
            "llm_calls": self.llm_calls,
            "branch": self.branch,
            "artifacts": self.artifacts,
            "assertions_run": self.assertions_run,
            "verdict": self.verdict,
            "skipped_reason": self.reason,
            "details": self.details,
            "file": file!(),
            "sha": std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_string()),
            "github_run_id": std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
            "github_run_attempt": std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_default(),
        }))?;
        std::fs::write(path, body)
    }
}

impl Drop for CapabilityEvidence {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.verdict = "failed";
            self.reason = "test panicked after capability evidence started".to_string();
        }
        if let Err(err) = self.persist() {
            eprintln!(
                "[capability-evidence] failed to persist {}: {err}",
                self.case_id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_requires_positive_artifact_and_assertion_counts() {
        let mut evidence = CapabilityEvidence::new("unit_contract");
        evidence.attempted();
        evidence.observe_llm_calls(2);
        evidence.branch("positive_path");
        evidence.pass(2, 3);
        assert!(evidence.attempted);
        assert_eq!(evidence.llm_calls, 2);
        assert_eq!(evidence.branch, "positive_path");
        assert_eq!(evidence.artifacts, 2);
        assert_eq!(evidence.assertions_run, 3);
        assert_eq!(evidence.verdict, "pass");
    }
}
