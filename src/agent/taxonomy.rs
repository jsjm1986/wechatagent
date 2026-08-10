//! `system_taxonomies` 严格字典 + `taxonomy_candidates` 候选集合的运行时入口。
//!
//! 双层标签设计（运营领域无关）：
//!
//! 1. **严格字典层 (`system_taxonomies`)**：按 `(scope, kind)` 任意维度组织的可
//!    枚举取值。`kind` 是字符串，由运营在后台维护（不再硬编码具体维度）。
//! 2. **候选层 (`taxonomy_candidates`)**：Reply Agent 输出但不在字典里的取值
//!    自动落入此集合（含 evidence / first_seen_at / occurrences），由后台审核
//!    后并入正式字典。**候选 SHALL NOT 阻塞 Reply Agent**。
//!
//! 核心 API（`kind` 全部按 `&str` 传入，调用方自定语义）：
//!
//! - [`check_value`]：纯函数，对照 `TaxonomyCache` 命中判定，返回 [`TaxonomyMatch`]。
//! - [`upsert_candidate`]：幂等 upsert（按 `(scope, kind, raw_value)` 唯一），
//!   `pending` → 累加 `occurrences`、`rejected` → 仅刷 `last_seen_at`、不存在 → insert pending。
//! - [`TaxonomyCache`]：进程级 TTL 缓存，启动期 + API 写后失效。
//!
//! 与 `enforce_decision_guards` 接入：上层把 LLM 返回的 `domainSignals` 字典逐
//! 项调 `check_value(kind, value, ...)`，按 match 分支：
//! - `Active`：合法值，无操作；
//! - `AliasActive(canonical_id)`：把 decision 字段改写为 canonical_id；
//! - `Deprecated`：追加 `taxonomy_deprecated_value:<kind>:<value>` risk；
//! - `CandidateNew`：追加 `taxonomy_candidate:<kind>:<value>` risk + 异步 upsert
//!   候选；不强制 `review.approved=false`。

use mongodb::bson::{doc, Bson, DateTime, Document};
use parking_lot::Mutex as PlMutex;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{TaxonomyCandidate, TaxonomyEntry};

/// 缓存有效期：30s。后台 API 在 approve/reject/insert/update/delete 时
/// 主动失效 [`TaxonomyCache`]，保证下一次 `check_value` 命中最新数据；
/// 在没有写操作时 30s 摊开 DB 加载开销。
const TAXONOMY_CACHE_TTL: Duration = Duration::from_secs(30);

/// `check_value` 命中分支。
///
/// `enforce_decision_guards` 按本枚举做 4 路分支：`Active` 通过 /
/// `AliasActive` 改写 / `Deprecated` 追加 risk / `CandidateNew` 追加 risk +
/// upsert（**不**强制 review fail）。
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
    /// `(workspace_id, scope, kind)` → entries（active + deprecated 都进缓存）。
    entries: HashMap<(String, String, String), Vec<CachedEntry>>,
    /// Compatibility aggregate used by existing diagnostics/tests.
    fetched_at: Option<Instant>,
    /// Per-workspace refresh time keeps manual DB writes eventually visible without global scans.
    workspace_fetched_at: HashMap<String, Instant>,
    /// Last database-authoritative lightweight generation observed per workspace.
    source_generations: HashMap<String, i64>,
}

#[derive(Debug, Clone)]
struct CachedEntry {
    canonical_id: String,
    aliases: Vec<String>,
    /// `"active"` | `"deprecated"`。
    status: String,
    /// universal-domain-adaptation 1C：planner 漏斗排序权重（来自 TaxonomyValue，
    /// 1B 已 seed）。`None` = 该维度不参与漏斗排序（如 objection_type）。
    priority_weight: Option<i32>,
    /// universal-domain-adaptation 1C：是否终态（成交后维护 / 冷却 / 沉默等不再被
    /// stage_stagnation 段催促）。planner 据此构造 terminal 集合替代写死的 TERMINAL_STAGES。
    is_terminal: bool,
    /// universal-domain-adaptation #3：是否再激活目标 stage（来自 TaxonomyValue）。
    is_reactivation_target: bool,
    /// 取值字典的人类可读名（来自 TaxonomyValue.display_name）。流 A prompt 取值
    /// 指引 + 流 B 前端 labelFor 翻译都用它；早期只缓存 planner 排序字段时被丢弃。
    display_name: String,
}

fn build_workspace_entries(
    workspace_id: &str,
    rows: Vec<TaxonomyEntry>,
) -> AppResult<HashMap<(String, String, String), Vec<CachedEntry>>> {
    let mut current_counts: HashMap<(String, String, String), usize> = HashMap::new();
    let mut current_entries = Vec::new();
    for entry in rows {
        let logical_key = (
            entry.scope.clone(),
            entry.kind.clone(),
            entry.value.id.clone(),
        );
        let count = current_counts.entry(logical_key).or_default();
        if entry.current_version {
            *count += 1;
            current_entries.push(entry);
        }
    }
    if let Some((key, count)) = current_counts.iter().find(|(_, count)| **count != 1) {
        return Err(AppError::Conflict(format!(
            "taxonomy current pointer invalid for workspace={workspace_id} scope={} kind={} value_id={}: count={count}",
            key.0, key.1, key.2
        )));
    }
    let mut entries = HashMap::new();
    let mut active_claims: HashMap<(String, String, String), String> = HashMap::new();
    for entry in current_entries {
        if entry.value.status == "active" {
            for claim in
                crate::models::taxonomy_identity_claims(&entry.value.id, &entry.value.aliases)
            {
                let claim_key = (entry.scope.clone(), entry.kind.clone(), claim.clone());
                if let Some(existing) = active_claims.insert(claim_key, entry.value.id.clone()) {
                    if existing != entry.value.id {
                        return Err(AppError::Conflict(format!(
                            "taxonomy identity claim {claim:?} is ambiguous between {existing:?} and {:?}",
                            entry.value.id
                        )));
                    }
                }
            }
        }
        entries
            .entry((workspace_id.to_string(), entry.scope, entry.kind))
            .or_insert_with(Vec::new)
            .push(CachedEntry {
                canonical_id: entry.value.id,
                aliases: entry.value.aliases,
                status: entry.value.status,
                priority_weight: entry.value.priority_weight,
                is_terminal: entry.value.is_terminal,
                is_reactivation_target: entry.value.is_reactivation_target,
                display_name: entry.value.display_name,
            });
    }
    Ok(entries)
}

impl Default for TaxonomyCache {
    fn default() -> Self {
        Self {
            inner: PlMutex::new(TaxonomyCacheInner {
                entries: HashMap::new(),
                fetched_at: None,
                workspace_fetched_at: HashMap::new(),
                source_generations: HashMap::new(),
            }),
        }
    }
}

impl TaxonomyCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deep immutable copy for one prompt-shadow A/B comparison.
    ///
    /// The returned cache owns a cloned entries map and is never registered in
    /// the process-global cache registry. Admin invalidation or TTL refresh of
    /// the live cache therefore cannot change either branch mid-comparison.
    pub(crate) fn snapshot_copy(&self) -> Self {
        let inner = self.inner.lock();
        Self {
            inner: PlMutex::new(TaxonomyCacheInner {
                entries: inner.entries.clone(),
                fetched_at: inner.fetched_at,
                workspace_fetched_at: inner.workspace_fetched_at.clone(),
                source_generations: inner.source_generations.clone(),
            }),
        }
    }

    /// 显式失效缓存。后台 API 在 approve/reject/insert/update/delete 后调用，
    /// 让下一次 `check_value` 走 `find_or_load` 重新拉取最新数据。
    pub fn invalidate(&self) {
        let mut inner = self.inner.lock();
        inner.entries.clear();
        inner.fetched_at = None;
        inner.workspace_fetched_at.clear();
        inner.source_generations.clear();
    }

    pub fn invalidate_workspace(&self, workspace_id: &str) {
        let mut inner = self.inner.lock();
        inner
            .entries
            .retain(|(workspace, _, _), _| workspace != workspace_id);
        inner.workspace_fetched_at.remove(workspace_id);
        inner.source_generations.remove(workspace_id);
        inner.fetched_at = None;
    }

    /// 启动期预热：全库审计 current 指针并构建初始缓存。
    pub async fn warm_up(&self, db: &Database) -> AppResult<()> {
        self.reload_all_from_db(db).await
    }

    async fn reload_all_from_db(&self, db: &Database) -> AppResult<()> {
        use futures::TryStreamExt;
        let mut cursor = db
            .collection_system_taxonomies()
            .find(doc! {}, None)
            .await?;
        let mut rows = Vec::new();
        while let Some(entry) = cursor.try_next().await? {
            rows.push(entry);
        }
        let workspaces = rows
            .iter()
            .map(|entry| entry.workspace_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut rebuilt = HashMap::new();
        for workspace_id in workspaces.iter() {
            let workspace_rows = rows
                .iter()
                .filter(|entry| &entry.workspace_id == workspace_id)
                .cloned()
                .collect::<Vec<_>>();
            rebuilt.extend(build_workspace_entries(workspace_id, workspace_rows)?);
        }
        let mut source_generations = HashMap::new();
        for workspace_id in &workspaces {
            let generation = crate::db::config_generation::read_generation(
                db,
                crate::db::config_generation::TAXONOMY_NAMESPACE,
                workspace_id,
            )
            .await?;
            source_generations.insert(workspace_id.clone(), generation);
        }
        let now = Instant::now();
        let mut inner = self.inner.lock();
        inner.entries = rebuilt;
        inner.fetched_at = Some(now);
        inner.workspace_fetched_at = workspaces
            .iter()
            .map(|workspace| (workspace.clone(), now))
            .collect();
        inner.source_generations = source_generations;
        Ok(())
    }

    async fn reload_workspace_from_db(
        &self,
        db: &Database,
        workspace_id: &str,
        generation: i64,
    ) -> AppResult<()> {
        use futures::TryStreamExt;
        let mut cursor = db
            .collection_system_taxonomies()
            .find(doc! { "workspace_id": workspace_id }, None)
            .await?;
        let mut rows = Vec::new();
        while let Some(entry) = cursor.try_next().await? {
            rows.push(entry);
        }
        let rebuilt = build_workspace_entries(workspace_id, rows)?;
        let now = Instant::now();
        let mut inner = self.inner.lock();
        inner
            .entries
            .retain(|(workspace, _, _), _| workspace != workspace_id);
        inner.entries.extend(rebuilt);
        inner.fetched_at = Some(now);
        inner
            .workspace_fetched_at
            .insert(workspace_id.to_string(), now);
        inner
            .source_generations
            .insert(workspace_id.to_string(), generation);
        Ok(())
    }

    fn workspace_is_stale(&self, workspace_id: &str) -> bool {
        self.inner
            .lock()
            .workspace_fetched_at
            .get(workspace_id)
            .is_none_or(|fetched| fetched.elapsed() >= TAXONOMY_CACHE_TTL)
    }

    /// TTL 自愈判定：fetched_at 缺失（从未加载）或距今 ≥ TAXONOMY_CACHE_TTL → true。
    ///
    /// 抽出独立函数让 lib-level 单测（无 Docker 环境）能直接断言"warm_up 之后
    /// 30s 内 stale=false / 30s 后 stale=true"的 TTL 自愈语义；`find_or_load` 走
    /// 同一判定避免双份口径。
    #[cfg(test)]
    pub(crate) fn is_stale(&self) -> bool {
        let inner = self.inner.lock();
        match inner.fetched_at {
            Some(t) => t.elapsed() >= TAXONOMY_CACHE_TTL,
            None => true,
        }
    }

    /// Production read: compare one small generation row, then reload only this workspace.
    pub(crate) async fn find_or_load(&self, db: &Database, workspace_id: &str) -> AppResult<()> {
        let seeded = ensure_workspace_taxonomies(db, workspace_id).await?;
        let authoritative = crate::db::config_generation::read_generation(
            db,
            crate::db::config_generation::TAXONOMY_NAMESPACE,
            workspace_id,
        )
        .await?;
        let cached = self
            .inner
            .lock()
            .source_generations
            .get(workspace_id)
            .copied();
        if seeded || self.workspace_is_stale(workspace_id) || cached != Some(authoritative) {
            self.reload_workspace_from_db(db, workspace_id, authoritative)
                .await?;
        }
        Ok(())
    }

    /// Shadow/replay refresh without materializing built-in rows.
    pub(crate) async fn find_or_load_read_only(
        &self,
        db: &Database,
        workspace_id: &str,
    ) -> AppResult<()> {
        let authoritative = crate::db::config_generation::read_generation(
            db,
            crate::db::config_generation::TAXONOMY_NAMESPACE,
            workspace_id,
        )
        .await?;
        let cached = self
            .inner
            .lock()
            .source_generations
            .get(workspace_id)
            .copied();
        if self.workspace_is_stale(workspace_id) || cached != Some(authoritative) {
            self.reload_workspace_from_db(db, workspace_id, authoritative)
                .await?;
        }
        Ok(())
    }

    /// test-only：把 `fetched_at` 强制回拨指定时长，模拟"距上次加载已经过 N"，
    /// 让 [`Self::is_stale`] 的 TTL 判定可以在不真等 30s 的前提下被验证。
    #[cfg(test)]
    pub(crate) fn rewind_fetched_at_for_test(&self, dur: Duration) {
        let mut inner = self.inner.lock();
        if let Some(t) = inner.fetched_at {
            inner.fetched_at = Some(t.checked_sub(dur).unwrap_or(t));
        }
        for fetched in inner.workspace_fetched_at.values_mut() {
            *fetched = fetched.checked_sub(dur).unwrap_or(*fetched);
        }
    }
}

/// 纯查表 `check_value`（无 IO）。
///
/// 调用方负责保证 `cache` 已加载（`warm_up` 或 `find_or_load`）；本函数仅做
/// O(1) 查表 + alias 反向查找，不做 DB 调用。
///
/// `kind` 直接传字典里的 snake_case 字符串（与 `system_taxonomies.kind` 字段一致）。
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
    workspace_id: &str,
    kind: &str,
    raw_value: &str,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> TaxonomyMatch {
    let inner = cache.inner.lock();
    // 优先看 account 私有字典；未命中再看 global。
    for scope in [scope_account_id, "global"] {
        let key = (
            workspace_id.to_string(),
            scope.to_string(),
            kind.to_string(),
        );
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

/// universal-domain-adaptation 1C：取某 `kind` 维度所有取值的 `(canonical_id,
/// priority_weight, is_terminal, is_reactivation_target)`，供 planner 构造漏斗排序
/// 权重表 + 终态集合 + 再激活目标集合。
///
/// 只读 active + deprecated（与 check_value 同源缓存）；scope 优先 account 私有、
/// 回落 global。同 canonical_id 跨 scope 命中时 account 优先（先插入者赢，account
/// 在前）。返回的权重 `None` 表示该取值不参与漏斗排序。
///
/// 调用方负责保证 cache 已加载。planner 每个 tick 调一次构造 `PlannerStageConfig`，
/// 避免 N+1。空缓存（未配置 / 加载失败）返回空 Vec，planner 回落写死的 DEFAULT。
pub(crate) fn dimension_value_weights(
    workspace_id: &str,
    kind: &str,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> Vec<(String, Option<i32>, bool, bool)> {
    let inner = cache.inner.lock();
    let mut out: Vec<(String, Option<i32>, bool, bool)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for scope in [scope_account_id, "global"] {
        let key = (
            workspace_id.to_string(),
            scope.to_string(),
            kind.to_string(),
        );
        if let Some(entries) = inner.entries.get(&key) {
            for e in entries {
                if seen.insert(e.canonical_id.clone()) {
                    out.push((
                        e.canonical_id.clone(),
                        e.priority_weight,
                        e.is_terminal,
                        e.is_reactivation_target,
                    ));
                }
            }
        }
    }
    out
}

/// 查某 `kind` 下所有 status=active 的 `(canonical_id, display_name)` 对。
/// scope 回落：account 私有 scope 优先，再补 global；按 canonical_id 去重。
/// 流 A prompt 取值指引 + 流 B 前端字典翻译共用。
pub(crate) fn dimension_values_with_labels(
    workspace_id: &str,
    kind: &str,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> Vec<(String, String)> {
    let inner = cache.inner.lock();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for scope in [scope_account_id, "global"] {
        let key = (
            workspace_id.to_string(),
            scope.to_string(),
            kind.to_string(),
        );
        if let Some(entries) = inner.entries.get(&key) {
            for e in entries {
                if e.status == "active" && seen.insert(e.canonical_id.clone()) {
                    out.push((e.canonical_id.clone(), e.display_name.clone()));
                }
            }
        }
    }
    out
}

/// 某 `kind` 在缓存里是否有任何字典条目（account 私有 scope 或 global）。
///
/// 用于区分「字典未配置（该 kind 整个为空）」与「字典有条目但此值越界」——前者属
/// 「未约束」应回退信任原值（与 [`dimension_value_weights`] 空缓存回落 DEFAULT、
/// `decision_taxonomy::classify_decision_tags` 对 dict-miss 软处理一致），后者是真越界
/// 按写入通道处置（机器 drop / admin reject）。`check_value` 对两种情况都返回
/// `CandidateNew`，无法区分，故由调用方（`dimension_registry::lookup_dict`）配合本函数判别。
///
/// 调用方负责保证 cache 已加载。读 [scope, "global"] 两层，任一层存在 status=="active"
/// 的条目即 true。纯 deprecated 残留（active=0）视同「未配置」→ false（F-009 fail-soft：
/// 字典只剩 deprecated 时不 Reject，KindUnconfigured→Accept 回退信任原值）。
pub(crate) fn kind_has_entries(
    workspace_id: &str,
    kind: &str,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> bool {
    let inner = cache.inner.lock();
    [scope_account_id, "global"].iter().any(|s| {
        inner
            .entries
            .get(&(workspace_id.to_string(), s.to_string(), kind.to_string()))
            .is_some_and(|e| e.iter().any(|c| c.status == "active"))
    })
}

/// 异步 upsert 候选。
///
/// 行为：
/// - 已存在 `status="rejected"` → 仅 `last_seen_at` 刷新，**不**递增 occurrences；
/// - 已存在 `status="pending"` → 递增 `occurrences` + 刷 `last_seen_at`；
/// - 已存在 `status="approved"` → 这种情况理论上不该发生（approved 已并入字典），
///   保守处理为 `last_seen_at` 刷新 + warning log；
/// - 不存在 → insert 一条 `status="pending"` 的新候选。
///
/// 强幂等键：`(scope, kind, raw_value)` 唯一索引。`kind` 由调用方按字典中
/// 实际维度名传入（snake_case，与 `system_taxonomies.kind` 一致）。
/// 并发竞争（两个 run 同时 upsert 同 raw_value）由 unique index + retry 保护。
pub(crate) async fn upsert_candidate(
    db: &Database,
    workspace_id: &str,
    scope_account_id: &str,
    kind: &str,
    raw_value: &str,
    evidence: Option<&str>,
    confidence: i32,
    suggested_display_name: Option<&str>,
) -> AppResult<()> {
    let now = DateTime::now();
    let collection = db.collection_taxonomy_candidates();

    // 先查现有状态。
    let existing = collection
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "scope": scope_account_id,
                "kind": kind,
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
                    kind = kind,
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
        workspace_id: workspace_id.to_string(),
        scope: scope_account_id.to_string(),
        kind: kind.to_string(),
        raw_value: raw_value.to_string(),
        evidence: evidence.map(|s| s.to_string()),
        confidence: confidence.clamp(0, 10),
        first_seen_at: now,
        last_seen_at: now,
        occurrences: 1,
        status: "pending".to_string(),
        reviewed_at: None,
        reviewed_by: None,
        suggested_display_name: suggested_display_name.map(|s| s.to_string()),
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
                    kind = kind,
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

/// Projection-worker variant of [`upsert_candidate`] whose occurrence count is idempotent per
/// run. Strict replay identity lives in `projection_observations`; `source_run_ids` is only a
/// bounded recent-run display cache.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn upsert_candidate_once_per_run(
    db: &Database,
    workspace_id: &str,
    scope_account_id: &str,
    kind: &str,
    raw_value: &str,
    evidence: Option<&str>,
    confidence: i32,
    suggested_display_name: Option<&str>,
    run_id: &str,
) -> AppResult<()> {
    let now = DateTime::now();
    let collection = db
        .collection_taxonomy_candidates()
        .clone_with_type::<Document>();
    let filter = doc! {
        "workspace_id": workspace_id,
        "scope": scope_account_id,
        "kind": kind,
        "raw_value": raw_value,
    };

    let mut existing = collection.find_one(filter.clone(), None).await?;
    if existing.is_none() {
        let mut insert = doc! {
            "workspace_id": workspace_id,
            "scope": scope_account_id,
            "kind": kind,
            "raw_value": raw_value,
            "confidence": confidence.clamp(0, 10),
            "first_seen_at": now,
            "last_seen_at": now,
            "occurrences": 0,
            "status": "pending",
            "reviewed_at": Bson::Null,
            "reviewed_by": Bson::Null,
            "source_run_ids": Vec::<String>::new(),
        };
        insert.insert("evidence", evidence.map(Bson::from).unwrap_or(Bson::Null));
        insert.insert(
            "suggested_display_name",
            suggested_display_name.map(Bson::from).unwrap_or(Bson::Null),
        );
        match collection.insert_one(insert, None).await {
            Ok(_) => {}
            Err(error)
                if error.to_string().contains("E11000")
                    || error.to_string().contains("duplicate key") => {}
            Err(error) => return Err(error.into()),
        }
        existing = collection.find_one(filter.clone(), None).await?;
    }

    let existing = existing.ok_or_else(|| {
        AppError::External("taxonomy candidate disappeared after upsert".to_string())
    })?;
    let status = existing.get_str("status").unwrap_or("pending");
    if status == "approved" {
        tracing::warn!(
            scope = scope_account_id,
            kind,
            raw_value,
            "projection candidate hit approved row; cache may be stale"
        );
    }
    if matches!(status, "approved" | "rejected") {
        collection
            .update_one(filter, doc! { "$set": { "last_seen_at": now } }, None)
            .await?;
        return Ok(());
    }

    let entity_id = format!(
        "{}:{}:{}",
        hex::encode(scope_account_id.as_bytes()),
        hex::encode(kind.as_bytes()),
        hex::encode(raw_value.as_bytes())
    );
    let legacy_run_ids = super::projection_observations::source_run_ids(&existing);
    let ledger_count = super::projection_observations::record_and_count(
        db,
        workspace_id,
        "taxonomy_candidate",
        &entity_id,
        &legacy_run_ids,
        run_id,
    )
    .await?;
    let mut stages = vec![doc! { "$set": { "last_seen_at": now } }];
    stages.extend(super::projection_observations::reconcile_stages(
        ledger_count,
        run_id,
        legacy_run_ids.len() as i64,
    ));
    collection.update_one(filter, stages, None).await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// 进程级、按 Database 连接身份隔离的 TaxonomyCache registry。
//
// `enforce_decision_taxonomy_guards` 在每次 run 都会查 cache；启动期全库审计预热，
// 运行时按 workspace 比较轻量 generation，仅在变化或恢复 TTL 到期时重载该 shard。
// 后台写在同一事务推进 generation，并在提交后失效本进程 workspace shard。
// ─────────────────────────────────────────────────────────────────

struct TaxonomyCacheRegistryEntry {
    database_lifetime: Weak<()>,
    cache: Arc<TaxonomyCache>,
}

static TAXONOMY_CACHE_REGISTRY: std::sync::LazyLock<
    PlMutex<HashMap<u64, TaxonomyCacheRegistryEntry>>,
> = std::sync::LazyLock::new(|| PlMutex::new(HashMap::new()));

static INITIALIZED_TAXONOMY_WORKSPACES: std::sync::LazyLock<
    dashmap::DashMap<(u64, String), Weak<()>>,
> = std::sync::LazyLock::new(dashmap::DashMap::new);

/// Idempotently install the built-in taxonomy template for a workspace. The
/// process-local guard avoids repeated upserts while the connection identity
/// in the key keeps independent test databases isolated from each other.
pub async fn ensure_workspace_taxonomies(db: &Database, workspace_id: &str) -> AppResult<bool> {
    let key = (db.cache_identity(), workspace_id.to_string());
    INITIALIZED_TAXONOMY_WORKSPACES.retain(|_, lifetime| lifetime.upgrade().is_some());
    if INITIALIZED_TAXONOMY_WORKSPACES.contains_key(&key) {
        return Ok(false);
    }
    let inserted =
        crate::db::migrations::ensure_builtin_taxonomies_for_workspace(db, workspace_id).await?;
    INITIALIZED_TAXONOMY_WORKSPACES.insert(key, db.cache_lifetime());
    if inserted {
        // The durable seed transaction already advanced the shared generation atomically.
        // This layer only drops the local workspace shard so the next lookup rebuilds it.
        global_taxonomy_cache(db).invalidate_workspace(workspace_id);
    }
    Ok(inserted)
}

fn taxonomy_cache_for_identity(identity: u64, database_lifetime: Weak<()>) -> Arc<TaxonomyCache> {
    let mut registry = TAXONOMY_CACHE_REGISTRY.lock();
    registry.retain(|_, entry| entry.database_lifetime.upgrade().is_some());
    registry
        .entry(identity)
        .or_insert_with(|| TaxonomyCacheRegistryEntry {
            database_lifetime,
            cache: Arc::new(TaxonomyCache::new()),
        })
        .cache
        .clone()
}

/// Return the cache owned by this concrete [`Database`] connection. Clones of
/// one Database share an identity/cache; independently connected databases do
/// not, even when they use the same workspace ids.
pub(crate) fn global_taxonomy_cache(db: &Database) -> Arc<TaxonomyCache> {
    taxonomy_cache_for_identity(db.cache_identity(), db.cache_lifetime())
}

/// 启动期预热：由 `main.rs` 在迁移和唯一索引建立后调用。
pub async fn init_global_taxonomy_cache(db: &Database) -> AppResult<()> {
    global_taxonomy_cache(db).warm_up(db).await
}

/// Invalidate one workspace shard in this process after its durable mutation commits.
pub(crate) fn invalidate_global_taxonomy_cache(db: &Database, workspace_id: &str) {
    global_taxonomy_cache(db).invalidate_workspace(workspace_id);
}

/// Read-only runtime diagnostic used by cross-replica integration tests and health tooling.
/// It traverses the exact production cache refresh path, then reports the classification without
/// mutating candidates or decision state.
pub async fn inspect_taxonomy_value(
    db: &Database,
    workspace_id: &str,
    scope_account_id: &str,
    kind: &str,
    raw_value: &str,
) -> AppResult<String> {
    let cache = global_taxonomy_cache(db);
    cache.find_or_load_read_only(db, workspace_id).await?;
    Ok(
        match check_value(workspace_id, kind, raw_value, scope_account_id, &cache) {
            TaxonomyMatch::Active => "active".to_string(),
            TaxonomyMatch::AliasActive(canonical) => format!("alias:{canonical}"),
            TaxonomyMatch::Deprecated => "deprecated".to_string(),
            TaxonomyMatch::CandidateNew => "candidate_new".to_string(),
        },
    )
}

/// 测试用 helper — 把已构造好的 [`TaxonomyEntry`] 集合直接灌入一个新 cache。
/// 让其它模块（如 `guards.rs`）的单元测试可以构造任意"字典内容"并对照断言
/// `check_value` / 上层守卫的行为，而无需 Mongo 实例。
///
/// 同一 helper 也供 `tests/autonomy_protocol_pbt.rs` 在独立 crate 中调用，
/// 因此从 `cfg(test)` 升级为 `pub`。
pub fn taxonomy_cache_for_tests(entries: Vec<TaxonomyEntry>) -> TaxonomyCache {
    let cache = TaxonomyCache::new();
    let mut grouped: HashMap<(String, String, String), Vec<CachedEntry>> = HashMap::new();
    for entry in entries {
        let key = (
            entry.workspace_id.clone(),
            entry.scope.clone(),
            entry.kind.clone(),
        );
        grouped
            .entry(key)
            .or_insert_with(Vec::new)
            .push(CachedEntry {
                canonical_id: entry.value.id,
                aliases: entry.value.aliases,
                status: entry.value.status,
                priority_weight: entry.value.priority_weight,
                is_terminal: entry.value.is_terminal,
                is_reactivation_target: entry.value.is_reactivation_target,
                display_name: entry.value.display_name,
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
        let mut grouped: HashMap<(String, String, String), Vec<CachedEntry>> = HashMap::new();
        for entry in entries {
            let key = (
                entry.workspace_id.clone(),
                entry.scope.clone(),
                entry.kind.clone(),
            );
            grouped
                .entry(key)
                .or_insert_with(Vec::new)
                .push(CachedEntry {
                    canonical_id: entry.value.id,
                    aliases: entry.value.aliases,
                    status: entry.value.status,
                    priority_weight: entry.value.priority_weight,
                    is_terminal: entry.value.is_terminal,
                    is_reactivation_target: entry.value.is_reactivation_target,
                    display_name: entry.value.display_name,
                });
        }
        {
            let mut inner = cache.inner.lock();
            inner.entries = grouped;
            let now = Instant::now();
            inner.fetched_at = Some(now);
            inner
                .workspace_fetched_at
                .insert("default".to_string(), now);
            inner.source_generations.insert("default".to_string(), 0);
        }
        cache
    }

    fn make_entry(
        scope: &str,
        kind: &str,
        canonical_id: &str,
        display_name: &str,
        aliases: &[&str],
        status: &str,
    ) -> TaxonomyEntry {
        TaxonomyEntry {
            id: None,
            workspace_id: "default".to_string(),
            scope: scope.to_string(),
            kind: kind.to_string(),
            value: TaxonomyValue {
                id: canonical_id.to_string(),
                display_name: display_name.to_string(),
                description: String::new(),
                aliases: aliases.iter().map(|s| s.to_string()).collect(),
                status: status.to_string(),
                priority_weight: None,
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
        }
    }

    #[test]
    fn check_value_returns_active_when_canonical_id_matches() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &["新客", "刚加好友"],
            "active",
        )]);
        let m = check_value(
            "default",
            "customer_stage",
            "first_contact",
            "acct-1",
            &cache,
        );
        assert_eq!(m, TaxonomyMatch::Active);
    }

    #[test]
    fn check_value_returns_alias_active_when_alias_matches() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &["新客", "刚加好友"],
            "active",
        )]);
        let m = check_value("default", "customer_stage", "新客", "acct-1", &cache);
        assert_eq!(m, TaxonomyMatch::AliasActive("first_contact".to_string()));
    }

    #[test]
    fn check_value_returns_deprecated_when_canonical_id_status_is_deprecated() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "intent_level",
            "lukewarm",
            "温意向",
            &[],
            "deprecated",
        )]);
        let m = check_value("default", "intent_level", "lukewarm", "acct-1", &cache);
        assert_eq!(m, TaxonomyMatch::Deprecated);
    }

    #[test]
    fn check_value_returns_candidate_new_when_value_unknown() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "objection_type",
            "price",
            "价格异议",
            &["价格异议"],
            "active",
        )]);
        let m = check_value(
            "default",
            "objection_type",
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
            make_entry(
                "global",
                "customer_stage",
                "first_contact",
                "初次接触",
                &[],
                "active",
            ),
            make_entry(
                "acct-1",
                "customer_stage",
                "premium_first_contact",
                "尊享初次接触",
                &["first_contact"],
                "active",
            ),
        ]);
        let m = check_value(
            "default",
            "customer_stage",
            "first_contact",
            "acct-1",
            &cache,
        );
        // 命中 account scope 的 alias，返回 canonical_id = premium_first_contact
        assert_eq!(
            m,
            TaxonomyMatch::AliasActive("premium_first_contact".to_string())
        );
    }

    #[test]
    fn check_value_distinct_kinds_do_not_collide() {
        // 同一 raw_value 在不同 kind 下相互独立；本案验证 kind 字符串作为查表键
        // 不会被错误共享。
        let cache = make_cache_with_entries(vec![
            make_entry(
                "global",
                "customer_stage",
                "shared_value",
                "共享值",
                &[],
                "active",
            ),
            make_entry(
                "global",
                "intent_level",
                "shared_value",
                "共享值",
                &[],
                "deprecated",
            ),
        ]);
        let stage = check_value(
            "default",
            "customer_stage",
            "shared_value",
            "acct-1",
            &cache,
        );
        let intent = check_value("default", "intent_level", "shared_value", "acct-1", &cache);
        assert_eq!(stage, TaxonomyMatch::Active);
        assert_eq!(intent, TaxonomyMatch::Deprecated);
    }

    /// `taxonomy_candidate_persisted_on_unknown_value`
    /// 验证：当 LLM 输出了不在 `system_taxonomies` 中的取值时，`check_value` 必须返回
    /// `CandidateNew`——这是 `enforce_decision_taxonomy_guards` 决定写入
    /// `taxonomy_candidates` 候选队列的契约信号。同时校验已知 active 值不会落入候选路径。
    #[test]
    fn taxonomy_candidate_persisted_on_unknown_value() {
        let cache = make_cache_with_entries(vec![
            make_entry(
                "global",
                "customer_stage",
                "first_contact",
                "初次接触",
                &["新客"],
                "active",
            ),
            make_entry(
                "global",
                "intent_level",
                "hot",
                "高意向",
                &["高意向"],
                "active",
            ),
            make_entry(
                "global",
                "objection_type",
                "price",
                "价格异议",
                &["价格异议"],
                "active",
            ),
        ]);

        // 三类未知值都应判为 CandidateNew（由调用方写入 taxonomy_candidates）。
        let unknown_stage =
            check_value("default", "customer_stage", "未知阶段_xx", "acct-1", &cache);
        let unknown_intent =
            check_value("default", "intent_level", "lukewarm_xx", "acct-1", &cache);
        let unknown_objection =
            check_value("default", "objection_type", "全新异议_xx", "acct-1", &cache);
        assert_eq!(unknown_stage, TaxonomyMatch::CandidateNew);
        assert_eq!(unknown_intent, TaxonomyMatch::CandidateNew);
        assert_eq!(unknown_objection, TaxonomyMatch::CandidateNew);

        // 已知 active 值不进候选。
        let known = check_value(
            "default",
            "customer_stage",
            "first_contact",
            "acct-1",
            &cache,
        );
        assert_eq!(known, TaxonomyMatch::Active);
    }

    /// 同一 Database identity 必须共享 cache，不同 identity 必须隔离。
    /// 这是生产 AppState clone 共享缓存、并行测试数据库互不串扰的核心 invariant。
    #[test]
    fn taxonomy_cache_registry_is_scoped_by_database_identity() {
        let lifetime_a = Arc::new(());
        let lifetime_b = Arc::new(());
        let h1 = taxonomy_cache_for_identity(u64::MAX - 1, Arc::downgrade(&lifetime_a));
        let h2 = taxonomy_cache_for_identity(u64::MAX - 1, Arc::downgrade(&lifetime_a));
        let isolated = taxonomy_cache_for_identity(u64::MAX, Arc::downgrade(&lifetime_b));
        assert!(Arc::ptr_eq(&h1, &h2), "同一数据库 identity 必须共享 cache");
        assert!(
            !Arc::ptr_eq(&h1, &isolated),
            "不同数据库 identity 不得共享 cache"
        );

        // 同 identity 的失效互相可见，但不能影响另一数据库实例。
        {
            let mut inner = h1.inner.lock();
            inner.fetched_at = Some(Instant::now());
        }
        {
            let mut inner = isolated.inner.lock();
            inner.fetched_at = Some(Instant::now());
        }
        h1.invalidate();
        {
            let inner = h2.inner.lock();
            assert!(
                inner.fetched_at.is_none(),
                "同一数据库 identity 的 invalidate 必须命中共享实例"
            );
        }
        assert!(
            isolated.inner.lock().fetched_at.is_some(),
            "失效一个数据库不得清空另一数据库 cache"
        );
    }

    #[test]
    fn taxonomy_snapshot_copy_survives_live_cache_invalidation() {
        let live = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &["新客"],
            "active",
        )]);
        let snapshot = live.snapshot_copy();

        live.invalidate();
        assert_eq!(
            check_value("default", "customer_stage", "新客", "acct-1", &snapshot,),
            TaxonomyMatch::AliasActive("first_contact".to_string()),
            "prompt-shadow snapshot must own its entries after live invalidation",
        );
        assert_eq!(
            check_value("default", "customer_stage", "新客", "acct-1", &live,),
            TaxonomyMatch::CandidateNew,
            "the live cache should still reflect the invalidation",
        );
    }

    /// Phase A 落地验证 / `taxonomy_cache_stale_when_never_fetched`
    ///
    /// 新建的 cache 还没被 warm_up / find_or_load 触达过，is_stale 必须为 true，
    /// 让首条决策走 find_or_load 时立即 reload，不会用空表去判 CandidateNew。
    #[test]
    fn taxonomy_cache_stale_when_never_fetched() {
        let cache = TaxonomyCache::new();
        assert!(cache.is_stale(), "fresh cache should be stale");
    }

    /// Phase A 落地验证 / `taxonomy_cache_not_stale_immediately_after_load`
    ///
    /// `make_cache_with_entries` 内部把 fetched_at 设为 Instant::now()，模拟刚从 DB
    /// 拉完的状态；is_stale 必须为 false，避免每条决策都 reload 把 30s TTL 摊销变零。
    #[test]
    fn taxonomy_cache_not_stale_immediately_after_load() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &[],
            "active",
        )]);
        assert!(
            !cache.is_stale(),
            "freshly-loaded cache should NOT be stale within TTL window"
        );
    }

    /// Phase A 落地验证 / `taxonomy_cache_self_heals_after_ttl`
    ///
    /// 这是 CLAUDE.md 硬规则"unreviewed candidates must not block runs"在 cache 维度
    /// 的兜底契约：admin 长期未触发 invalidate 也不会让 cache 永远 stale —— TTL=30s
    /// 一过，下次 find_or_load 自动 reload。本测用 `rewind_fetched_at_for_test` 把
    /// 加载时间回拨到 31s 前，断言 is_stale 翻转为 true（reload 副作用本身要 DB，
    /// 留给 #[ignore] 集成测试覆盖）。
    #[test]
    fn taxonomy_cache_self_heals_after_ttl() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &[],
            "active",
        )]);
        assert!(!cache.is_stale());
        cache.rewind_fetched_at_for_test(TAXONOMY_CACHE_TTL + Duration::from_secs(1));
        assert!(
            cache.is_stale(),
            "cache fetched > TTL ago should report stale, triggering find_or_load reload"
        );
    }

    /// Phase A 落地验证 / `taxonomy_cache_invalidate_marks_stale`
    ///
    /// admin 写一条 system_taxonomies 后调 invalidate，紧接着的下一次 is_stale 必须
    /// 为 true，保证后台改动至少一次 reload 才能反映到决策路径（与 30s TTL 兜底解耦）。
    #[test]
    fn taxonomy_cache_invalidate_marks_stale() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &[],
            "active",
        )]);
        assert!(!cache.is_stale());
        cache.invalidate();
        assert!(
            cache.is_stale(),
            "invalidate must trigger reload on next find_or_load"
        );
    }

    /// `kind_has_entries`：该 kind 在缓存里有非空 entries（global 或 account scope）→ true。
    #[test]
    fn kind_has_entries_true_when_configured() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            "初次接触",
            &[],
            "active",
        )]);
        assert!(kind_has_entries(
            "default",
            "customer_stage",
            "acct-1",
            &cache
        ));
        // account 私有 scope 也算（命中任一层即 true）。
        let cache2 = make_cache_with_entries(vec![make_entry(
            "acct-1",
            "customer_stage",
            "first_contact",
            "初次接触",
            &[],
            "active",
        )]);
        assert!(kind_has_entries(
            "default",
            "customer_stage",
            "acct-1",
            &cache2
        ));
    }

    /// `kind_has_entries`：该 kind 字典整个为空（未配置，如 m012 删 seed 后）→ false。
    /// 这是「字典未配置→回退信任」与「有条目但越界→drop」分流的判据。
    #[test]
    fn kind_has_entries_false_when_empty() {
        // 缓存里只有别的 kind，customer_stage 整个未配置。
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "intent_level",
            "high",
            "高意向",
            &[],
            "active",
        )]);
        assert!(!kind_has_entries(
            "default",
            "customer_stage",
            "acct-1",
            &cache
        ));
        // 完全空缓存。
        let empty = make_cache_with_entries(vec![]);
        assert!(!kind_has_entries(
            "default",
            "customer_stage",
            "acct-1",
            &empty
        ));
    }

    /// `kind_has_entries`：该 kind 只剩 deprecated 残留（active=0）→ false（F-009）。
    /// 纯 deprecated 视同「未配置」→ KindUnconfigured→Accept fail-soft 放行，
    /// 避免字典只剩废弃条目时运营填任何目标阶段被 Reject。
    #[test]
    fn kind_has_entries_false_when_only_deprecated() {
        let cache = make_cache_with_entries(vec![make_entry(
            "global",
            "customer_stage",
            "legacy_stage",
            "旧阶段",
            &[],
            "deprecated",
        )]);
        assert!(!kind_has_entries(
            "default",
            "customer_stage",
            "acct-1",
            &cache
        ));
        // active + deprecated 混存时仍 true（有 active 即算已配置）。
        let mixed = make_cache_with_entries(vec![
            make_entry(
                "global",
                "customer_stage",
                "legacy_stage",
                "旧阶段",
                &[],
                "deprecated",
            ),
            make_entry(
                "global",
                "customer_stage",
                "first_contact",
                "初次接触",
                &[],
                "active",
            ),
        ]);
        assert!(kind_has_entries(
            "default",
            "customer_stage",
            "acct-1",
            &mixed
        ));
    }

    /// `dimension_values_with_labels_returns_id_label_pairs`
    /// 验证：返回该 kind 下 status=active 的 `(canonical_id, display_name)` 对，
    /// deprecated 条目被滤除（流 A prompt 取值指引 / 流 B 前端翻译只列在用取值）。
    #[test]
    fn dimension_values_with_labels_returns_id_label_pairs() {
        let cache = make_cache_with_entries(vec![
            make_entry(
                "global",
                "customer_stage",
                "first_contact",
                "初次接触",
                &[],
                "active",
            ),
            make_entry(
                "global",
                "customer_stage",
                "qualified",
                "已确认意向",
                &[],
                "active",
            ),
            make_entry(
                "global",
                "customer_stage",
                "old_dep",
                "废弃",
                &[],
                "deprecated",
            ),
        ]);
        let mut got = dimension_values_with_labels("default", "customer_stage", "acct1", &cache);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("first_contact".to_string(), "初次接触".to_string()),
                ("qualified".to_string(), "已确认意向".to_string()),
            ]
        );
    }

    #[test]
    fn cache_does_not_leak_entries_across_workspaces() {
        let mut entry = make_entry(
            "global",
            "customer_stage",
            "ws_a_only",
            "A 租户专用",
            &[],
            "active",
        );
        entry.workspace_id = "ws-a".to_string();
        let cache = make_cache_with_entries(vec![entry]);

        assert_eq!(
            check_value("ws-a", "customer_stage", "ws_a_only", "acct-1", &cache),
            TaxonomyMatch::Active
        );
        assert_eq!(
            check_value("ws-b", "customer_stage", "ws_a_only", "acct-1", &cache),
            TaxonomyMatch::CandidateNew
        );
        assert!(!kind_has_entries(
            "ws-b",
            "customer_stage",
            "acct-1",
            &cache
        ));
    }
}
