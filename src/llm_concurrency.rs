//! Process-local LLM admission control.
//!
//! Foreground customer-delivery calls may consume every total permit. Explicitly
//! classified background work must first acquire the background semaphore, so it
//! can never occupy the permits reserved for foreground work. Mongo durability
//! remains the retry/source-of-truth boundary; this governor only controls local
//! pressure on the shared upstream provider.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmPriority {
    Foreground,
    Background,
}

impl LlmPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Background => "background",
        }
    }
}

/// Keep the list deliberately narrow. Unknown and interactive prompt keys stay
/// foreground; only durable/replayable work is allowed to use background slots.
pub fn priority_for_prompt(prompt_key: &str) -> LlmPriority {
    if prompt_key == "user.projection.task"
        || prompt_key.starts_with("user.memory_consolidator.")
        || prompt_key.starts_with("user.initial_profile.")
        || prompt_key.starts_with("evolution.")
        || prompt_key.starts_with("knowledge.digest.")
        || prompt_key.starts_with("knowledge.import.")
        || prompt_key.starts_with("knowledge.auto_verify")
        || prompt_key.starts_with("knowledge.tags.")
    {
        LlmPriority::Background
    } else {
        LlmPriority::Foreground
    }
}

#[derive(Clone)]
pub struct LlmConcurrencyGovernor {
    total: Arc<Semaphore>,
    background: Arc<Semaphore>,
    total_limit: usize,
    background_limit: usize,
}

impl std::fmt::Debug for LlmConcurrencyGovernor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConcurrencyGovernor")
            .field("total_limit", &self.total_limit)
            .field("background_limit", &self.background_limit)
            .finish()
    }
}

impl LlmConcurrencyGovernor {
    pub fn new(total_limit: usize, foreground_reserved: usize) -> Self {
        let total_limit = total_limit.max(1);
        let foreground_reserved = foreground_reserved.clamp(1, total_limit);
        // A total limit of one cannot reserve one and also admit background work.
        // Keep one background lane in that degenerate deployment, while normal
        // deployments retain the configured foreground reservation.
        let background_limit = total_limit
            .saturating_sub(foreground_reserved)
            .max(usize::from(total_limit == 1));
        Self {
            total: Arc::new(Semaphore::new(total_limit)),
            background: Arc::new(Semaphore::new(background_limit)),
            total_limit,
            background_limit,
        }
    }

    pub fn limits(&self) -> (usize, usize) {
        (self.total_limit, self.background_limit)
    }

    pub async fn acquire(&self, priority: LlmPriority) -> LlmAdmission {
        let started = Instant::now();
        let background_permit = if priority == LlmPriority::Background {
            Some(
                self.background
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("LLM background semaphore is never closed"),
            )
        } else {
            None
        };
        let total_permit = self
            .total
            .clone()
            .acquire_owned()
            .await
            .expect("LLM total semaphore is never closed");
        LlmAdmission {
            priority,
            queue_wait: started.elapsed(),
            _total_permit: total_permit,
            _background_permit: background_permit,
        }
    }
}

pub struct LlmAdmission {
    priority: LlmPriority,
    queue_wait: Duration,
    _total_permit: OwnedSemaphorePermit,
    _background_permit: Option<OwnedSemaphorePermit>,
}

impl LlmAdmission {
    pub fn priority(&self) -> LlmPriority {
        self.priority
    }

    pub fn queue_wait(&self) -> Duration {
        self.queue_wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn prompt_priority_is_conservative() {
        assert_eq!(
            priority_for_prompt("user.reply.fast.task"),
            LlmPriority::Foreground
        );
        assert_eq!(
            priority_for_prompt("user.reaction.task"),
            LlmPriority::Foreground
        );
        assert_eq!(
            priority_for_prompt("knowledge.agent"),
            LlmPriority::Foreground
        );
        assert_eq!(
            priority_for_prompt("unknown.interactive"),
            LlmPriority::Foreground
        );
        assert_eq!(
            priority_for_prompt("user.projection.task"),
            LlmPriority::Background
        );
        assert_eq!(
            priority_for_prompt("user.memory_consolidator.task"),
            LlmPriority::Background
        );
    }

    #[tokio::test]
    async fn background_cannot_consume_reserved_foreground_capacity() {
        let governor = LlmConcurrencyGovernor::new(4, 2);
        let active_background = Arc::new(AtomicUsize::new(0));
        let peak_background = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let governor = governor.clone();
            let active = active_background.clone();
            let peak = peak_background.clone();
            tasks.push(tokio::spawn(async move {
                let _permit = governor.acquire(LlmPriority::Background).await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        let foreground = tokio::time::timeout(
            Duration::from_millis(20),
            governor.acquire(LlmPriority::Foreground),
        )
        .await;
        assert!(
            foreground.is_ok(),
            "foreground reservation must remain available"
        );
        futures::future::join_all(tasks).await;
        assert_eq!(peak_background.load(Ordering::SeqCst), 2);
    }
}
