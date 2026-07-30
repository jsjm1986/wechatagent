//! Immutable revision tokens for Evolution proposals and released artifacts.
//!
//! Tokens are deliberately opaque outside this module. They bind evaluation
//! evidence to the exact threshold override or prompt template that was read.

use mongodb::bson::oid::ObjectId;
use sha2::{Digest, Sha256};

const THRESHOLD_REVISION_PREFIX: &str = "threshold-v1";
const PROMPT_REVISION_PREFIX: &str = "prompt-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRevision {
    pub template_id: ObjectId,
    pub version: i32,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdRevision {
    pub source_id: Option<ObjectId>,
    pub value: f64,
}

/// Prompt keys that the shadow path can actually inject and evaluate.
pub const EVOLVABLE_PROMPT_KEYS: &[&str] = &[
    "user.reply.system",
    "user.reply.policy",
    "user.reply.task",
    "user.review.system",
    "user.review.light.system",
];

pub fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn threshold_revision(source_id: Option<ObjectId>, value: f64) -> String {
    let source = source_id
        .map(|id| id.to_hex())
        .unwrap_or_else(|| "baseline".to_string());
    format!(
        "{THRESHOLD_REVISION_PREFIX}:{source}:{:016x}",
        value.to_bits()
    )
}

pub fn parse_threshold_revision(token: &str) -> Option<ThresholdRevision> {
    let mut parts = token.split(':');
    if parts.next()? != THRESHOLD_REVISION_PREFIX {
        return None;
    }
    let source = parts.next()?;
    let source_id = if source == "baseline" {
        None
    } else {
        Some(ObjectId::parse_str(source).ok()?)
    };
    let bits = u64::from_str_radix(parts.next()?, 16).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(ThresholdRevision {
        source_id,
        value: f64::from_bits(bits),
    })
}

pub fn prompt_revision(template_id: ObjectId, version: i32, content: &str) -> String {
    format!(
        "{PROMPT_REVISION_PREFIX}:{}:{version}:{}",
        template_id.to_hex(),
        content_sha256(content)
    )
}

pub fn parse_prompt_revision(token: &str) -> Option<PromptRevision> {
    let mut parts = token.split(':');
    if parts.next()? != PROMPT_REVISION_PREFIX {
        return None;
    }
    let template_id = ObjectId::parse_str(parts.next()?).ok()?;
    let version = parts.next()?.parse().ok()?;
    let content_sha256 = parts.next()?.to_string();
    if parts.next().is_some() || content_sha256.len() != 64 {
        return None;
    }
    Some(PromptRevision {
        template_id,
        version,
        content_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_revision_round_trips_and_binds_content() {
        let id = ObjectId::new();
        let token = prompt_revision(id, 7, "alpha");
        let parsed = parse_prompt_revision(&token).expect("valid token");
        assert_eq!(parsed.template_id, id);
        assert_eq!(parsed.version, 7);
        assert_eq!(parsed.content_sha256, content_sha256("alpha"));
        assert_ne!(token, prompt_revision(id, 7, "beta"));
    }

    #[test]
    fn threshold_revision_binds_source_and_float_bits() {
        let id = ObjectId::new();
        assert_eq!(
            threshold_revision(Some(id), 6.5),
            threshold_revision(Some(id), 6.5)
        );
        assert_ne!(
            threshold_revision(Some(id), 6.5),
            threshold_revision(None, 6.5)
        );
        assert_ne!(
            threshold_revision(Some(id), 6.5),
            threshold_revision(Some(id), 6.0)
        );
        assert_eq!(
            parse_threshold_revision(&threshold_revision(Some(id), 6.5)),
            Some(ThresholdRevision {
                source_id: Some(id),
                value: 6.5,
            })
        );
        assert_eq!(
            parse_threshold_revision(&threshold_revision(None, 7.0)),
            Some(ThresholdRevision {
                source_id: None,
                value: 7.0,
            })
        );
    }
}
