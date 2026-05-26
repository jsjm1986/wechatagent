//! agent-autonomy-loop W3 / Task 4.6：`system_taxonomies` 字典 + `taxonomy_candidates`
//! 候选集合的运行时入口。
//!
//! 双层标签设计（详见 design.md §3.3 / §3.4 + requirements.md R8）：
//!
//! 1. **严格字典层 (`system_taxonomies`)**：`customer_stage / intent_level /
//!    objection_type` 三个维度的可枚举取值，由迁移 `2026_05_006_taxonomy_seed`
//!    填充默认值，后台 API 可增删改（详见 task 4.8）。
//! 2. **候选层 (`taxonomy_candidates`)**：Reply Agent 输出但不在字典里的取值
//!    自动落入此集合（含 evidence / first_seen_at / occurrences），由后台审核
//!    后并入正式字典。**候选 SHALL NOT 阻塞 Reply Agent**（R8.4）。
//!
//! 核心 API：
//!
//! - [`check_value`]：纯函数，对照 `TaxonomyCache` 命中判定，返回 [`TaxonomyMatch`]。
//! - [`upsert_candidate`]：幂等 upsert（按 `(scope, kind, raw_value)` 唯一），
//!   `pending` → 累加 `occurrences`、`rejected` → 仅刷 `last_seen_at`、不存在 → insert pending。
//! - [`approve`] / [`reject`]：后台审核入口；approve 时事务性把 candidate 写入
//!   `system_taxonomies` 并把 candidate.status=approved。
//! - [`TaxonomyCache`]：进程级 TTL 缓存，启动期 + API 写后失效。
//!
//! 与 `enforce_decision_guards` 的接入在 task 4.7：对 `decision.customer_stage /
//! intent_level / objection_type` 三字段调 `check_value`，按 match 分支：
//! - `Active`：合法值，无操作；
//! - `AliasActive(canonical_id)`：把 decision 字段改写为 canonical_id；
//! - `Deprecated`：追加 `taxonomy_deprecated_value:<kind>:<value>` risk；
//! - `CandidateNew`：追加 `taxonomy_candidate:<kind>:<value>` risk + 异步 upsert
//!   候选；不强制 `review.approved=false`。

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use parking_lot::Mutex as PlMutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{TaxonomyCandidate, TaxonomyEntry, TaxonomyValue};

/// 缓存有效期：30s。后台 API 在 approve/reject/insert/update/delete 时
/// 主动失效 [`TaxonomyCache`]，保证下一次 `check_value` 命中最新数据；
/// 在没有写操作时 30s 摊开 DB 加载开销。
const TAXONOMY_CACHE_TTL: Duration = Duration::from_secs(30);

/// agent-autonomy-loop W3 / Task 4.6：维度类型枚举（强类型化常用 kind）。
///
/// `enforce_decision_guards` 只需要查 `customer_stage / intent_level /
/// objection_type` 三个维度，强类型化能避免拼写错误并让 `&str` 转换在编译期
/// 提前失败。`as_str` 返回小写 snake_case 与字典里的 `kind` 字段一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TaxonomyKind {
    CustomerStage,
    IntentLevel,
    ObjectionType,
}

impl TaxonomyKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaxonomyKind::CustomerStage => "customer_stage",
            TaxonomyKind::IntentLevel => "intent_level",
            TaxonomyKind::ObjectionType => "objection_type",
        }
    }
}

/// agent-autonomy-loop W3 / Task 4.6：`check_value` 命中分支。
///
/// 与 design.md §3.3 一致；`enforce_decision_guards` 在 task 4.7 按本枚举做
/// 4 路分支：`Active` 通过 / `AliasActive` 改写 / `Deprecated` 追加 risk /
/// `CandidateNew` 追加 risk + upsert（**不**强制 review fail）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaxonomyMatch {
    /// 命中字典中 `status="active"` 且 `value.id == raw`。
    Active,
    /// 命中 alias，需把 decision 字段改写为 canonical id。
    AliasActive(String),
    /// 命中字典中 `status="deprecated"` 的取值（合法但建议迁移）。
    Deprecated,
    /// 不在字典中：候选新值，需 upsert candidate。
    CandidateNew,
}

/// agent-autonomy-loop W3 / Task 4.6：进程级 TTL 缓存。
///
/// 内部按 `(scope, kind)` 索引一组 [`TaxonomyEntry`]，并对每组预计算 alias →
/// canonical_id 的反向 map。`check_value` 是 O(1) 查表 + alias 查找。
///
/// 缓存失效：通过 [`Self::invalidate`] 显式失效（后台 API 写后调用）；
/// `find_or_load` 在 TTL 到期 / 失效后自动重新加载。`Default` 直接给空实例，
/// 后台 / 入口启动期需调一次 [`Self::warm_up`] 预热（避免第一条决策被冷启动延迟）。
pub struct TaxonomyCache {
    inner: PlMutex<TaxonomyCacheInner>,
}

struct TaxonomyCacheInner {
    /// `(scope, kind)` → entries（active + deprecated 都进缓存）。
    entries: HashMap<(String, String), Vec<CachedEntry>>,
    fetched_at: Option<Instant>,
}

#[derive(Debug, Clone)]
struct CachedEntry {
    canonical_id: String,
    aliases: Vec<String>,
    /// `"active"` | `"deprecated"`。
    status: String,
}

impl Default for TaxonomyCache {
    fn default() -> Self {
        Self {
            inner: PlMutex::new(TaxonomyCacheInner {
                entries: HashMap::new(),
                fetched_at: None,
            }),
        }
    }
}

impl TaxonomyCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 显式失效缓存。后台 API 在 approve/reject/insert/update/delete 后调用，
    /// 让下一次 `check_value` 走 `find_or_load` 重新拉取最新数据。
    pub fn invalidate(&self) {
        let mut inner = self.inner.lock();
        inner.entries.clear();
        inner.fetched_at = None;
    }

    /// 启动期预热：从 DB 加载 `system_taxonomies` 全表并填充缓存。
    /// 失败被静默（缓存留空，下次 `check_value` 重新尝试加载）。
    pub async fn warm_up(&self, db: &Database) {
        if let Err(error) = self.reload_from_db(db).await {
            tracing::warn!(?error, "TaxonomyCache.warm_up failed; cache remains empty");
        }
    }

    async fn reload_from_db(&self, db: &Database) -> AppResult<()> {
        use futures::TryStreamExt;
        let mut cursor = db.collection_system_taxonomies().find(doc! {}, None).await?;
        let mut entries: HashMap<(String, String), Vec<CachedEntry>> = HashMap::new();
        while let Some(entry) = cursor.try_next().await? {
            let key = (entry.scope.clone(), entry.kind.clone());
            entries
                .entry(key)
                .or_insert_with(Vec::new)
                .push(CachedEntry {
                    canonical_id: entry.value.id,
                    aliases: entry.value.aliases,
                    status: entry.value.status,
                });
        }
        let mut inner = self.inner.lock();
        inner.entries = entries;
        inner.fetched_at = Some(Instant::now());
        Ok(())
    }

    /// 查找或自动加载（TTL 过期 → 异步加载）。
    /// 注意：本方法保持调用方 `&self`，内部异步加载完成后写回 inner。
    async fn find_or_load(&self, db: &Database) {
        let needs_reload = {
            let inner = self.inner.lock();
            match inner.fetched_at {
                Some(t) => t.elapsed() >= TAXONOMY_CACHE_TTL,
                None => true,
            }
        };
        if needs_reload {
            if let Err(error) = self.reload_from_db(db).await {
                tracing::warn!(?error, "TaxonomyCache.reload_from_db failed");
            }
        }
    }
}

/// agent-autonomy-loop W3 / Task 4.6：纯查表 `check_value`（无 IO）。
///
/// 调用方负责保证 `cache` 已加载（`warm_up` 或 `find_or_load`）；本函数仅做
/// O(1) 查表 + alias 反向查找，不做 DB 调用。
///
/// 命中规则（按优先级）：
/// 1. 任一 entry 的 `canonical_id == raw && status == "active"` → [`TaxonomyMatch::Active`]
/// 2. 任一 entry 的 `aliases` 含 `raw && status == "active"` → [`TaxonomyMatch::AliasActive(canonical_id)`]
/// 3. 任一 entry 的 `canonical_id == raw && status == "deprecated"` → [`TaxonomyMatch::Deprecated`]
///    （aliases 命中 deprecated 同上）
/// 4. 否则 → [`TaxonomyMatch::CandidateNew`]
///
/// `scope` 优先按 `account_id` 查，未命中再按 `"global"` 查（两层 fallback）。
pub(crate) fn check_value(
    kind: TaxonomyKind,
    raw_value: &str,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> TaxonomyMatch {
    let kind_str = kind.as_str();
    let inner = cache.inner.lock();
    // 优先看 account 私有字典；未命中再看 global。
    for scope in [scope_account_id, "global"] {
        let key = (scope.to_string(), kind_str.to_string());
        if let Some(entries) = inner.entries.get(&key) {
            // 1) canonical_id 命中（active 优先于 deprecated）。
            if let Some(entry) = entries
                .iter()
                .find(|e| e.canonical_id == raw_value && e.status == "active")
            {
                let _ = entry; // explicitly used
                return TaxonomyMatch::Active;
            }
            if let Some(entry) = entries
                .iter()
                .find(|e| e.canonical_id == raw_value && e.status == "deprecated")
            {
                let _ = entry;
                return TaxonomyMatch::Deprecated;
            }
            // 2) alias 命中。
            if let Some(entry) = entries
                .iter()
                .find(|e| e.aliases.iter().any(|a| a == raw_value) && e.status == "active")
            {
                return TaxonomyMatch::AliasActive(entry.canonical_id.clone());
            }
            if let Some(entry) = entries
                .iter()
                .find(|e| e.aliases.iter().any(|a| a == raw_value) && e.status == "deprecated")
            {
                let _ = entry;
                return TaxonomyMatch::Deprecated;
            }
        }
    }
    TaxonomyMatch::CandidateNew
}

/// agent-autonomy-loop W3 / Task 4.6：异步 upsert 候选。
///
/// 行为：
/// - 已存在 `status="rejected"` → 仅 `last_seen_at` 刷新，**不**递增 occurrences；
/// - 已存在 `status="pending"` → 递增 `occurrences` + 刷 `last_seen_at`；
/// - 已存在 `status="approved"` → 这种情况理论上不该发生（approved 已并入字典），
///   保守处理为 `last_seen_at` 刷新 + warning log；
/// - 不存在 → insert 一条 `status="pending"` 的新候选。
///
/// 强幂等键：`(scope, kind, raw_value)` 唯一索引（详见 task 1.2）。
/// 并发竞争（两个 run 同时 upsert 同 raw_value）由 unique index + retry 保护。
#[allow(dead_code)]
pub(crate) async fn upsert_candidate(
    db: &Database,
    scope_account_id: &str,
    kind: TaxonomyKind,
    raw_value: &str,
    evidence: Option<&str>,
    confidence: i32,
) -> AppResult<()> {
    let kind_str = kind.as_str();
    let now = DateTime::now();
    let collection = db.collection_taxonomy_candidates();

    // 先查现有状态。
    let existing = collection
        .find_one(
            doc! {
                "scope": scope_account_id,
                "kind": kind_str,
                "raw_value": raw_value,
            },
            None,
        )
        .await?;

    if let Some(existing) = existing {
        match existing.status.as_str() {
            "rejected" => {
                // 仅刷 last_seen_at，不递增 occurrences（避免 reject 后被反复刷新干扰运营）。
                collection
                    .update_one(
                        doc! { "_id": existing.id },
                        doc! { "$set": { "last_seen_at": now } },
                        None,
                    )
                    .await?;
            }
            "approved" => {
                // 不该发生：approved 候选已并入字典；保守处理。
                tracing::warn!(
                    scope = scope_account_id,
                    kind = kind_str,
                    raw_value,
                    "upsert_candidate hit status=approved candidate; cache may be stale"
                );
                collection
                    .update_one(
                        doc! { "_id": existing.id },
                        doc! { "$set": { "last_seen_at": now } },
                        None,
                    )
                    .await?;
            }
            _ => {
                // status="pending" 或其它非法值：递增 occurrences。
                collection
                    .update_one(
                        doc! { "_id": existing.id },
                        doc! {
                            "$set": { "last_seen_at": now },
                            "$inc": { "occurrences": 1 }
                        },
                        None,
                    )
                    .await?;
            }
        }
        return Ok(());
    }

    let candidate = TaxonomyCandidate {
        id: None,
        scope: scope_account_id.to_string(),
        kind: kind_str.to_string(),
        raw_value: raw_value.to_string(),
        evidence: evidence.map(|s| s.to_string()),
        confidence: confidence.clamp(0, 10),
        first_seen_at: now,
        last_seen_at: now,
        occurrences: 1,
        status: "pending".to_string(),
        reviewed_at: None,
        reviewed_by: None,
    };

    // unique index 冲突视为竞态：另一个并发 run 已经写入；忽略错误，留给下次累加。
    match collection.insert_one(&candidate, None).await {
        Ok(_) => Ok(()),
        Err(error) => {
            // mongodb 11000 = duplicate key
            let msg = error.to_string();
            if msg.contains("E11000") || msg.contains("duplicate key") {
                tracing::debug!(
                    scope = scope_account_id,
                    kind = kind_str,
                    raw_value,
                    "upsert_candidate insert lost race; another worker won, ignored"
                );
                Ok(())
            } else {
                Err(error.into())
            }
        }
    }
}

/// agent-autonomy-loop W3 / Task 4.6：后台审核 — 通过候选。
///
/// 行为：
/// 1. 把候选 `(scope, kind, raw_value)` 作为 `value.id` 写入 `system_taxonomies`
///    （`status="active"`、`display_name = raw_value`、aliases 空）；
/// 2. 把候选 `status` 改为 `"approved"`、`reviewed_at=now`、`reviewed_by=by`；
/// 3. 让 [`TaxonomyCache`] 失效（调用方传入）。
///
/// 注意：本函数 SHALL 由后台 API 用单独的 transaction 包裹（task 4.8 实现），
/// 这里只暴露最小事务无关的步骤。失败时若 system_taxonomies 已写入但 candidate
/// 未更新，下次审核会发现 `status != "pending"` 而幂等跳过；若 candidate 更新
/// 成功但 system_taxonomies 写入失败，下次相同 value 会被视为 CandidateNew 重新
/// 走流程（少量重复但不破坏正确性）。
#[allow(dead_code)]
pub(crate) async fn approve(
    db: &Database,
    candidate_id: ObjectId,
    by: &str,
    cache: Option<&Arc<TaxonomyCache>>,
) -> AppResult<TaxonomyEntry> {
    let collection_candidates = db.collection_taxonomy_candidates();
    let candidate = collection_candidates
        .find_one(doc! { "_id": candidate_id }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("候选 taxonomy 不存在".to_string()))?;
    if candidate.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "候选状态 = {}，仅 status=pending 可 approve",
            candidate.status
        )));
    }

    let now = DateTime::now();
    let entry = TaxonomyEntry {
        id: None,
        scope: candidate.scope.clone(),
        kind: candidate.kind.clone(),
        value: TaxonomyValue {
            id: candidate.raw_value.clone(),
            display_name: candidate.raw_value.clone(),
            description: candidate.evidence.clone().unwrap_or_default(),
            aliases: Vec::new(),
            status: "active".to_string(),
        },
        updated_at: now,
    };

    // 先写字典：(scope, kind, value.id) 唯一索引保证幂等；冲突视为已存在，跳过。
    match db
        .collection_system_taxonomies()
        .insert_one(&entry, None)
        .await
    {
        Ok(_) => {}
        Err(error) => {
            let msg = error.to_string();
            if !(msg.contains("E11000") || msg.contains("duplicate key")) {
                return Err(error.into());
            }
            tracing::info!(
                scope = candidate.scope.as_str(),
                kind = candidate.kind.as_str(),
                value_id = candidate.raw_value.as_str(),
                "approve_candidate found existing taxonomy entry, skipping insert"
            );
        }
    }

    collection_candidates
        .update_one(
            doc! { "_id": candidate_id },
            doc! {
                "$set": {
                    "status": "approved",
                    "reviewed_at": now,
                    "reviewed_by": by,
                }
            },
            None,
        )
        .await?;

    if let Some(cache) = cache {
        cache.invalidate();
    }
    Ok(entry)
}

/// agent-autonomy-loop W3 / Task 4.6：后台审核 — 拒绝候选。
/// 仅把候选 `status` 改为 `"rejected"`，**不**写字典。
#[allow(dead_code)]
pub(crate) async fn reject(
    db: &Database,
    candidate_id: ObjectId,
    by: &str,
) -> AppResult<()> {
    let now = DateTime::now();
    let result = db
        .collection_taxonomy_candidates()
        .update_one(
            doc! { "_id": candidate_id, "status": "pending" },
            doc! {
                "$set": {
                    "status": "rejected",
                    "reviewed_at": now,
                    "reviewed_by": by,
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::BadRequest(
            "候选不存在或状态不是 pending".to_string(),
        ));
    }
    Ok(())
}

/// 公开 helper：让 cache 自动加载（异步），供 `enforce_decision_guards`
/// 在调用 `check_value` 之前用。
#[allow(dead_code)]
pub(crate) async fn ensure_cache_loaded(cache: &TaxonomyCache, db: &Database) {
    cache.find_or_load(db).await;
}

// ─────────────────────────────────────────────────────────────────
// agent-autonomy-loop W3 / Task 4.7：进程级共享 TaxonomyCache。
//
// `enforce_decision_taxonomy_guards` 在每次 run 都会查 cache；启动期
// 由 `init_global_taxonomy_cache(db)` 预热（task 4.6 / main.rs 接入），
// 后台 API（task 4.8）写后调 [`invalidate_global_taxonomy_cache`] 失效。
// ─────────────────────────────────────────────────────────────────

static GLOBAL_TAXONOMY_CACHE: std::sync::LazyLock<Arc<TaxonomyCache>> =
    std::sync::LazyLock::new(|| Arc::new(TaxonomyCache::new()));

/// 进程级单例 cache 句柄；`enforce_decision_taxonomy_guards` 调用方在没有
/// 注入自定义 cache 时使用本入口。
pub(crate) fn global_taxonomy_cache() -> Arc<TaxonomyCache> {
    GLOBAL_TAXONOMY_CACHE.clone()
}

/// 启动期预热：由 `main.rs` / `Database::ensure_indexes` 后调用。失败被静默
/// （log warning），不阻塞应用启动；下次 `check_value` 会触发懒加载。
#[allow(dead_code)]
pub(crate) async fn init_global_taxonomy_cache(db: &Database) {
    GLOBAL_TAXONOMY_CACHE.warm_up(db).await;
}

/// 后台 API（admin_taxonomies / admin_taxonomy_candidates）在写后调用以让缓
/// 存立即失效。
#[allow(dead_code)]
pub(crate) fn invalidate_global_taxonomy_cache() {
    GLOBAL_TAXONOMY_CACHE.invalidate();
}

/// `find_or_load` 的公开包装：调用方传入 `&Database`，本函数自动按 TTL 决定
/// 是否重新加载。供 `enforce_decision_taxonomy_guards` 在每次 run 调用前
/// 使用，确保字典数据不超过 TTL 陈旧度。
#[allow(dead_code)]
pub(crate) async fn ensure_global_cache_loaded(db: &Database) {
    GLOBAL_TAXONOMY_CACHE.find_or_load(db).await;
}

/// agent-autonomy-loop W3 / Task 4.10：测试用 helper — 把已构造好的
/// [`TaxonomyEntry`] 集合直接灌入一个新 cache。让其它模块（如 `guards.rs`）的
/// 单元测试可以构造任意"字典内容"并对照断言 `check_value` / 上层守卫的行为，
/// 而无需 Mongo 实例。
///
/// agent-autonomy-loop W3 / Task 4.11：同一 helper 也供 `tests/autonomy_protocol_pbt.rs`
/// 在独立 crate 中调用，因此从 `cfg(test)` 升级为 `pub`（类型 `TaxonomyCache`
/// 已经是 `pub(crate)`，需要同步升 pub —— 见 `TaxonomyCache` 上方）。
pub fn taxonomy_cache_for_tests(entries: Vec<TaxonomyEntry>) -> TaxonomyCache {
    let cache = TaxonomyCache::new();
    let mut grouped: HashMap<(String, String), Vec<CachedEntry>> = HashMap::new();
    for entry in entries {
        let key = (entry.scope.clone(), entry.kind.clone());
        grouped.entry(key).or_insert_with(Vec::new).push(CachedEntry {
            canonical_id: entry.value.id,
            aliases: entry.value.aliases,
            status: entry.value.status,
        });
    }
    {
        let mut inner = cache.inner.lock();
        inner.entries = grouped;
        inner.fetched_at = Some(Instant::now());
    }
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TaxonomyEntry, TaxonomyValue};

    fn make_cache_with_entries(entries: Vec<TaxonomyEntry>) -> TaxonomyCache {
        let cache = TaxonomyCache::new();
        let mut grouped: HashMap<(String, String), Vec<CachedEntry>> = HashMap::new();
        for entry in entries {
            let key = (entry.scope.clone(), entry.kind.clone());
            grouped
                .entry(key)
                .or_insert_with(Vec::new)
                .push(CachedEntry {
                    canonical_id: entry.value.id,
                    aliases: entry.value.aliases,
                    status: entry.value.status,
                });
        }
        {
            let mut inner = cache.inner.lock();
            inner.entries = grouped;
            inner.fetched_at = Some(Instant::now());
        }
        cache
    }

    fn make_entry(
        scope: &str,
        kind: &str,
        canonical_id: &str,
        aliases: &[&str],
        status: &str,
    ) -> TaxonomyEntry {
        TaxonomyEntry {
            id: None,
            scope: scope.to_string(),
            kind: kind.to_string(),
            value: TaxonomyValue {
                id: canonical_id.to_string(),
                display_name: canonical_id.to_string(),
                description: String::new(),
                aliases: aliases.iter().map(|s| s.to_string()).collect(),
                status: status.to_string(),
            },
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn check_value_returns_active_when_canonical_id_matches() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            &["新客", "刚加好友"],
            "active",
        )]);
        let m = check_value(TaxonomyKind::CustomerStage, "first_contact", "acct-1", &cache);
        assert_eq!(m, TaxonomyMatch::Active);
    }

    #[test]
    fn check_value_returns_alias_active_when_alias_matches() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            &["新客", "刚加好友"],
            "active",
        )]);
        let m = check_value(TaxonomyKind::CustomerStage, "新客", "acct-1", &cache);
        assert_eq!(m, TaxonomyMatch::AliasActive("first_contact".to_string()));
    }

    #[test]
    fn check_value_returns_deprecated_when_canonical_id_status_is_deprecated() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "intent_level",
            "lukewarm",
            &[],
            "deprecated",
        )]);
        let m = check_value(TaxonomyKind::IntentLevel, "lukewarm", "acct-1", &cache);
        assert_eq!(m, TaxonomyMatch::Deprecated);
    }

    #[test]
    fn check_value_returns_candidate_new_when_value_unknown() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "objection_type",
            "price",
            &["价格异议"],
            "active",
        )]);
        let m = check_value(
            TaxonomyKind::ObjectionType,
            "完全没听过的异议类型",
            "acct-1",
            &cache,
        );
        assert_eq!(m, TaxonomyMatch::CandidateNew);
    }

    #[test]
    fn check_value_account_scope_overrides_global_scope() {
        // account 私有字典里有 first_contact aliased to acct-special；
        // global 字典里 first_contact 是 active。account scope 优先。
        let cache = make_cache_with_entries(vec![
            make_entry("global", "customer_stage", "first_contact", &[], "active"),
            make_entry(
                "acct-1",
                "customer_stage",
                "premium_first_contact",
                &["first_contact"],
                "active",
            ),
        ]);
        let m = check_value(TaxonomyKind::CustomerStage, "first_contact", "acct-1", &cache);
        // 命中 account scope 的 alias，返回 canonical_id = premium_first_contact
        assert_eq!(
            m,
            TaxonomyMatch::AliasActive("premium_first_contact".to_string())
        );
    }

    #[test]
    fn taxonomy_kind_str_matches_dict_kind_field() {
        assert_eq!(TaxonomyKind::CustomerStage.as_str(), "customer_stage");
        assert_eq!(TaxonomyKind::IntentLevel.as_str(), "intent_level");
        assert_eq!(TaxonomyKind::ObjectionType.as_str(), "objection_type");
    }

    /// Phase A6: `taxonomy_candidate_persisted_on_unknown_value`
    /// 验证：当用户对话使用了不在 `system_taxonomies` 中的 `customer_stage / intent_level
    /// / objection_type` 取值时，`check_value` 必须返回 `CandidateNew`——这是
    /// `enforce_decision_taxonomy_guards` 决定写入 `taxonomy_candidates` 候选队列的契约信号。
    /// 同时校验已知 active 值不会落入候选路径。
    #[test]
    fn taxonomy_candidate_persisted_on_unknown_value() {
        let cache = make_cache_with_entries(vec![
            make_entry("global", "customer_stage", "first_contact", &["新客"], "active"),
            make_entry("global", "intent_level", "hot", &["高意向"], "active"),
            make_entry("global", "objection_type", "price", &["价格异议"], "active"),
        ]);

        // 三类未知值都应判为 CandidateNew（由调用方写入 taxonomy_candidates）。
        let unknown_stage =
            check_value(TaxonomyKind::CustomerStage, "未知阶段_xx", "acct-1", &cache);
        let unknown_intent =
            check_value(TaxonomyKind::IntentLevel, "lukewarm_xx", "acct-1", &cache);
        let unknown_objection =
            check_value(TaxonomyKind::ObjectionType, "全新异议_xx", "acct-1", &cache);
        assert_eq!(unknown_stage, TaxonomyMatch::CandidateNew);
        assert_eq!(unknown_intent, TaxonomyMatch::CandidateNew);
        assert_eq!(unknown_objection, TaxonomyMatch::CandidateNew);

        // 已知 active 值不进候选。
        let known = check_value(TaxonomyKind::CustomerStage, "first_contact", "acct-1", &cache);
        assert_eq!(known, TaxonomyMatch::Active);
    }

    /// Phase A6: `taxonomy_init_runs_at_startup`
    /// 验证：进程级单例 `GLOBAL_TAXONOMY_CACHE` 唯一可达；`init_global_taxonomy_cache`
    /// 与 `invalidate_global_taxonomy_cache` 都通过 `global_taxonomy_cache()` 操作同一句柄
    /// （`main.rs` 启动序列依赖该 invariant）。
    #[test]
    fn taxonomy_init_runs_at_startup() {
        let h1 = global_taxonomy_cache();
        let h2 = global_taxonomy_cache();
        assert!(Arc::ptr_eq(&h1, &h2), "单例 Arc 必须同源");

        // invalidate 必须真正落到同一句柄上（清空内部 fetched_at）。
        {
            let mut inner = h1.inner.lock();
            inner.fetched_at = Some(Instant::now());
        }
        invalidate_global_taxonomy_cache();
        {
            let inner = h2.inner.lock();
            assert!(
                inner.fetched_at.is_none(),
                "invalidate 应通过单例清空 fetched_at"
            );
        }
    }
}
