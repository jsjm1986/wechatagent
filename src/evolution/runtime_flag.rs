//! Phase C / C3：演化器运行时灰度开关。
//!
//! 把 M4 W1 时引入的 `EVOLUTION_ENABLED` env-var 抬升为 mongo 单文档运行时
//! 开关 + 灰度比例（0..=100）。运维可在不重启的情况下：
//!   1. `enabled=false` 一键关停整个 evolution worker；
//!   2. `enabled=true, rollout_percent=5` 进 5% 灰度；
//!   3. 监控正常后逐步抬到 20% / 50% / 100%。
//!
//! 灰度按 `contact_id` 哈希分桶（`hash(contact_id) % 100`），结果对同一
//! contact 在 rollout_percent 单调上升时永远是"先进先稳"——一个 contact
//! 一旦在 5% 桶里命中，rollout 抬到 20% 时它仍然在桶内，避免来回切换。
//!
//! `EVOLUTION_ENABLED=false` env-var 仍然作为最外层熔断：env 关停时
//! `is_evolution_enabled_for` 直接返回 false，不再读 mongo 文档；env 开启
//! 时才进一步读 mongo flag 决定 contact 是否落在灰度桶里。这样 env 可以
//! 作为生产紧急 kill switch（不需要 mongo 写权限），mongo flag 是日常
//! 灰度调度面板。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use mongodb::bson::doc;

use crate::error::AppResult;
use crate::models::EvolutionRuntimeFlag;
use crate::routes::AppState;

/// 读取指定 workspace 的 `evolution_runtime_flags` 单文档；不存在时返回 `None`。
///
/// 调用方默认应把 `None` 视作"灰度未开"——即便 env `EVOLUTION_ENABLED=true`
/// 也不应该让全部流量进 evolution（那是 W1 时代的行为，C3 后被 mongo flag 接管）。
pub async fn load_runtime_flag(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<Option<EvolutionRuntimeFlag>> {
    let doc = state
        .db
        .evolution_runtime_flags()
        .find_one(doc! { "workspace_id": workspace_id }, None)
        .await?;
    Ok(doc)
}

/// 计算 `contact_id` 的灰度桶索引（0..=99）。
///
/// 用 `DefaultHasher` 而非 `md5/sha`：方法决定的稳定性来自 Rust 标准库
/// `BuildHasher` 在同一进程内的一致性 + 同一 contact_id 输入产生同一
/// `u64` 输出，足以在 worker / webhook / shadow 三路调用方拿到一致的桶号。
/// 跨版本不保证，但 evolution 灰度不依赖跨版本稳定（rollout_percent
/// 抬升时由 admin 显式确认，不依赖"上次的桶号还在原位"）。
pub fn rollout_bucket_index(contact_id: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    contact_id.hash(&mut hasher);
    (hasher.finish() % 100) as u32
}

/// 单 contact 是否进灰度桶：
///   `flag.enabled && rollout_bucket_index(contact_id) < flag.rollout_percent_clamped()`。
///
/// `enabled=false` 时永远 false（一键关停优先级最高）；rollout_percent=0
/// 时桶索引 0..99 都不小于 0，全部回 false——此即"灰度门关闭但 worker
/// 仍跑空 tick"的状态。
pub fn bucket_for_contact(flag: &EvolutionRuntimeFlag, contact_id: &str) -> bool {
    if !flag.enabled {
        return false;
    }
    rollout_bucket_index(contact_id) < flag.rollout_percent_clamped()
}

/// 综合判定：env `EVOLUTION_ENABLED` + mongo `evolution_runtime_flags` 双闸。
///
/// 顺序：
///   1. `state.config.evolution_enabled = false` → 直接 false（env kill switch）；
///   2. mongo 文档不存在 → false（默认保守，不灰度）；
///   3. `flag.enabled = false` → false；
///   4. `rollout_bucket_index(contact_id) < flag.rollout_percent_clamped()` → true。
///
/// 任何一步抛错（mongo 不可用等）都按 false 返回 + warn 日志，避免演化器在
/// 数据库抖动时把自己升级成"全量启用"。
pub async fn is_evolution_enabled_for(
    state: &AppState,
    workspace_id: &str,
    contact_id: &str,
) -> bool {
    if !state.config.evolution_enabled {
        return false;
    }
    match load_runtime_flag(state, workspace_id).await {
        Ok(Some(flag)) => bucket_for_contact(&flag, contact_id),
        Ok(None) => false,
        Err(err) => {
            tracing::warn!(
                workspace_id,
                contact_id,
                ?err,
                "evolution runtime flag lookup failed; default to disabled"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::DateTime;

    fn flag(enabled: bool, rollout_percent: u32) -> EvolutionRuntimeFlag {
        EvolutionRuntimeFlag {
            id: None,
            workspace_id: "ws_test".to_string(),
            enabled,
            rollout_percent,
            updated_by: None,
            updated_at: DateTime::now(),
            threshold_auto_release_enabled: false,
        }
    }

    /// 同一 contact_id 永远落同一桶——是 A/B 桶稳定性的基础。
    #[test]
    fn rollout_bucket_index_deterministic() {
        let a1 = rollout_bucket_index("contact_abc");
        let a2 = rollout_bucket_index("contact_abc");
        let a3 = rollout_bucket_index("contact_abc");
        assert_eq!(a1, a2);
        assert_eq!(a2, a3);
        assert!(a1 < 100, "bucket must be in [0, 100)");
    }

    /// 不同 contact_id 至少能产出不同桶号（probabilistic：1000 个里允许偶发碰撞，
    /// 但 distinct 桶数 ≥ 50 是合理下界）。
    #[test]
    fn rollout_bucket_index_distributes() {
        use std::collections::HashSet;
        let mut buckets = HashSet::new();
        for i in 0..1000 {
            buckets.insert(rollout_bucket_index(&format!("contact_{i}")));
        }
        assert!(
            buckets.len() >= 50,
            "expected reasonable bucket spread, got {}",
            buckets.len()
        );
    }

    /// `enabled=false` 一票否决：rollout_percent 100 也不灰度。
    #[test]
    fn bucket_for_contact_disabled_overrides_percent() {
        let f = flag(false, 100);
        assert!(!bucket_for_contact(&f, "anyone"));
    }

    /// `rollout_percent=0` 时所有 contact 都不在桶内。
    #[test]
    fn bucket_for_contact_zero_percent_excludes_all() {
        let f = flag(true, 0);
        for i in 0..200 {
            assert!(!bucket_for_contact(&f, &format!("c_{i}")));
        }
    }

    /// `rollout_percent=100` 时所有 contact 都在桶内。
    #[test]
    fn bucket_for_contact_full_percent_includes_all() {
        let f = flag(true, 100);
        for i in 0..200 {
            assert!(bucket_for_contact(&f, &format!("c_{i}")));
        }
    }

    /// rollout_percent 单调上升时，原本在小桶里的 contact 必然仍在大桶里。
    /// 这是 A/B 稳定性的核心保证：5% → 20% → 50% 不应让任何已命中桶的
    /// contact 退出。
    #[test]
    fn bucket_for_contact_monotonic_rollout() {
        for i in 0..500 {
            let cid = format!("c_{i}");
            let f5 = flag(true, 5);
            if bucket_for_contact(&f5, &cid) {
                assert!(bucket_for_contact(&flag(true, 20), &cid));
                assert!(bucket_for_contact(&flag(true, 50), &cid));
                assert!(bucket_for_contact(&flag(true, 100), &cid));
            }
        }
    }

    /// `rollout_percent_clamped` 把脏数据 200 钳到 100。
    #[test]
    fn rollout_percent_clamped_caps_at_100() {
        let f = flag(true, 200);
        assert_eq!(f.rollout_percent_clamped(), 100);
        assert!(bucket_for_contact(&f, "anyone"));
    }
}
