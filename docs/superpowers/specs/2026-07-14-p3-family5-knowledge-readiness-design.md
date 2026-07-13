# P3 家族⑤ knowledge 就绪债设计（KB-07 + KB-06 + KB-12）

> P3 桶B/C。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KB-07（:495-504）+ KB-06（:485-493）+ KB-12（:596-604）。三条 Low，性质各异（非同源）。全部行号亲验于分支基点 origin/main `c530264`（含 #197）。

## 背景与定位

三条 knowledge 子系统就绪债，一个 PR：
- **KB-07（真逻辑修复）**：召回热路径在线 gap 信号 dedup 逻辑不严谨，同 kind 多主题时漏合并、产重复条。
- **KB-06（doc 标注，就绪债）**：structural_proposals 只产 pending_review 无 apply/人审消费方——功能半实装。
- **KB-12（doc 标注，就绪债）**：reviewer_stats 只按 workspace 聚合缺 account 维度——刻意设计但口径与另两端点不一致。

**用户裁决（brainstorming）**：KB-07 真修 + KB-06/KB-12 doc 标注（不补 apply UI、不加 account 维度）。

## 关键亲验事实（决定方案，全部主控当场 Read 亲验）

1. **在线 vs 离线合并逻辑不同**（KB-07 修复正确性的核心）：
   - 在线 `persist_recall_signal`（gap_signals.rs:596-665）：`find_one({workspace_id, status:pending, kind}, 无序)`（:604-605）→ 对返回的**单条** `.filter(dedup_key==key)`（:610）。命中则合并 **affected_chunk_ids + search_queries 两者**（:612-648），未命中新建（:650+，source=`recall_trace`）。**无 auto-resolve**。
   - 离线 `persist_signals`（gap_signals.rs:484-554）：全量 `find({workspace_id, status:pending})`（:491）→ 载入 `HashMap<dedup_key, signal>`（:499-503）→ 候选按 key `.get()` 精确命中。命中则**只并 affected_chunk_ids**（:513-529，新建时 search_queries=Vec::new() :540），source=`rule`，且有 auto-resolve（:558+）。
   - **结论**：两路径合并语义不同（在线多并 search_queries、不同 source、无 auto-resolve）。KB-07 **只换"如何找到该合并的 existing"这一步**（find_one 无序 → 全量 find 按 dedup_key 精确命中），**保留在线原有合并逻辑（含 search_queries 并集）一字不动**——绝不抽共用函数强行合并两路径（会破坏在线的 search_queries 特性/source/auto-resolve 差异）。
2. **KB-07 根因**（亲验）：在线为省一次全量查询走 `find_one`（只按 kind 过滤、无序、无 dedup_key 过滤），对随机返回的单条做内存 filter。同一 kind（如 recall_miss）下已有多条不同 title 的 pending 时，find_one 返"任意一条"很可能不匹配 dedup_key → filter 掉 → 走新建 → 同主题信号漏合并、产重复条。有 signal_id 唯一索引（indexes.rs:1386）兜底不写坏数据，仅噪音（重复条稀释信号、search_queries 变体不累积）。
3. **signal_dedup_key 是已存在的纯函数**（gap_signals.rs:470）：`signal_dedup_key(kind, title, affected)`，两路径都用它。修复复用它、不新造。
4. **KB-06 模块头已自述 out-of-scope**（structural_proposals.rs:1-14）：模块头明写"结构化写……本轮**只产 intent proposal**……系统侧原子化应用属于**下一轮**（本轮 out-of-scope：无 apply worker / 无版本一致性机器）"。全仓消费方只有 `knowledge_agent.rs:30`（调 propose_structural_change 产提案）——无任何 apply/人审 UI 消费。doc 基本已有，补一句更醒目的顶部"⚠️ 生产未接线"标注（同家族1 H-02 手法）。
5. **KB-12 是刻意 workspace 级 + 有单测锁死**（reviewer_stats.rs）：`aggregate_reviewer_stats_for_workspace`（:49）三条 count_documents 只按 workspace_id（:62/73/85），stat_id=`{workspace_id}::reviewer`（:101，一 workspace 一行），单测 `stat_id_is_workspace_scoped`（:175）锁死。reviewer 的 prompt/model 是 workspace 级属性，故度量刻意做 workspace 级。加 account 维度是投机未来需求（YAGNI）。
6. **测试基础设施**（KB-07 哨兵）：`persist_recall_signal`（:596 pub）/`GapSignalCandidate`（:398 pub struct + :410 impl）/`gap_signals` 模块（mod.rs:24 pub）均可从集成测直接调。既有 `tests/wiki_gap_signals_3kinds.rs`（307 行，`#[ignore]` Docker）已 `use wechatagent::knowledge_wiki::gap_signals;` + `TestApp::start()` + `list_pending(app, kind)` helper（:57 直查 knowledge_gap_signals）——KB-07 哨兵扩展此文件。`use futures::TryStreamExt`（gap_signals.rs:35）已导入，KB-07 改法的 try_collect 无需加 import。

## 目标

- KB-07：在线 gap 信号 dedup 改全量 find + 按 dedup_key 精确命中，对齐离线查找口径，杜绝同 kind 多主题漏合并。
- KB-06/KB-12：doc 标注就绪债现状（未接线 / workspace 级刻意），消除误解，不改行为。

## 架构：一真修 + 两 doc 标注

### KB-07 —— 在线 dedup 改全量精确命中（gap_signals.rs:601-610）

只换"找 existing"这一步，保留其余全部：

```rust
// 现状（:601-610）：find_one 无序 + 对单条 filter
let key = candidate.dedup_key();
let existing = db.knowledge_gap_signals()
    .find_one(doc! { "workspace_id": workspace_id, "status": "pending", "kind": &candidate.kind }, None)
    .await.map_err(AppError::from)?
    .filter(|s| signal_dedup_key(&s.kind, &s.title, &s.affected_chunk_ids) == key);

// 改为：全量 find + 按 dedup_key 精确命中（对齐离线 persist_signals:489-503 查找口径）
let key = candidate.dedup_key();
let pending: Vec<KnowledgeGapSignal> = db.knowledge_gap_signals()
    .find(doc! { "workspace_id": workspace_id, "status": "pending" }, None)
    .await.map_err(AppError::from)?
    .try_collect().await.map_err(AppError::from)?;
let existing = pending
    .into_iter()
    .find(|s| signal_dedup_key(&s.kind, &s.title, &s.affected_chunk_ids) == key);
```

**其余一字不动**：`if let Some(existing)` 合并块（affected_chunk_ids + search_queries 并集，:612-648）、新建分支（:650+）保留。用 `Vec::into_iter().find()` 而非建 HashMap——在线单候选只需命中一条，`.find()` 精确且简洁；离线多候选才需 HashMap 摊销。语义等价（全量 pending 内按 dedup_key 精确匹配，不再受 find_one 无序影响），复杂度对在线场景最优。

**安全性质**：纯改查找方式。原 find_one 无序→随机看一条→可能漏；新全量→必看到所有 pending→按 dedup_key 精确命中该合并的那条。只会让"本该合并却漏了"变正确，绝不产生新的错误合并（dedup_key 精确匹配，不放宽）。

### KB-06 —— structural_proposals 模块头补"生产未接线"标注

在 structural_proposals.rs 模块头最前补：

```rust
//! ⚠️ **生产未接线（就绪债 KB-06）**：本模块只产 `status=pending_review` 提案，
//! 全仓无任何 apply worker / 人审 UI 消费方（提案产出后纯躺集合）。这是红线
//! **正确**的一面（AI 绝不自动 apply split/merge），但功能未闭环——接线属下一轮。
//!
```

不改任何函数/行为。不补 apply 功能（涉红线人审 UI，超一条 Low 范围）。

### KB-12 —— reviewer_stats 补 workspace 级刻意的 doc

在 `aggregate_reviewer_stats_for_workspace`（reviewer_stats.rs:49）现有 doc 补：

```rust
/// ⚠️ **workspace 级聚合（刻意，就绪债 KB-12）**：本函数按 workspace_id 聚合、
/// stat_id=`{workspace_id}::reviewer`（一 workspace 一行），**不带 account_id 维度**
/// ——与 outcome_metrics / outcomes_autonomy 的 (workspace_id, account_id) 双维不同。
/// reviewer 的 prompt/model 是 workspace 级属性，故度量刻意做 workspace 级。若未来
/// 一 workspace 多账号成常态需按账号切，再加 account 维度对齐另两端点。
```

不改聚合逻辑/单测。不加 account 维度（YAGNI）。

## 改动面

- **Modify** `src/knowledge_wiki/gap_signals.rs`：`persist_recall_signal`（:601-610）改全量 find + `.find()` 精确命中（KB-07 唯一逻辑改动）。
- **Modify** `src/knowledge_wiki/structural_proposals.rs`：模块头补"生产未接线"标注（KB-06 doc）。
- **Modify** `src/knowledge_wiki/reviewer_stats.rs`：`aggregate_reviewer_stats_for_workspace` 补 doc（KB-12 doc）。
- **Modify/扩展** `tests/wiki_gap_signals_3kinds.rs`：append KB-07 精确合并哨兵（Docker `#[ignore]`）。

## 测试计划

- **KB-07 集成测（Docker `#[ignore]`，CI 跑）**：扩展 `tests/wiki_gap_signals_3kinds.rs`。哨兵 `recall_signal_dedup_merges_correct_topic_among_multiple_pending`：
  - seed 同 kind（如 recall_miss）下 2 条不同 title 的 pending 信号（直接 insert_one KnowledgeGapSignal，或调 persist_recall_signal 两次不同 title）。
  - 调 `persist_recall_signal` 一个 dedup_key 匹配**第 2 条**title 的候选（带新 search_query）。
  - 断言：pending 仍 2 条（**不新建第 3 条**）+ 第 2 条的 search_queries 增长（合并进正确的那条）。
  - 真回归哨兵：回退到 find_one 无序时，find_one 可能返第 1 条→filter 不匹配→误新建第 3 条→断言 count==2 失败变红。实现者亲验 `GapSignalCandidate` 构造方式（:410 impl）+ 既有 3kinds 测的 seed 范式后写。
- **KB-06/KB-12**：纯 doc 注释，无行为改动 → 无需测试，`cargo check --lib` 确认注释不破坏编译即可。

## 回归风险

1. **KB-07 纯改查找方式**：合并/新建逻辑不动。全量 find 比 find_one 多载入本 workspace pending（recall 热路径，但 pending 信号量通常小，与离线同成本；signal_id 唯一索引兜底）。查找口径与离线一致。
2. **KB-06/KB-12 纯 doc**：零行为改动、零回归。
3. **baseline**：`cargo test --lib` ≥ 350 / 0 不回退（不碰 lib 单测，signal_dedup_key 测 :1837 不动）。
4. **check-no-human-takeover lint**：亲验 lint 只扫 `src/agent/ src/routes/ src/evolution/ frontend/src/` 四目录（scripts/check-no-human-takeover.sh:27-30），**`src/knowledge_wiki/` 不在扫描范围**——本家族三处改动全在 knowledge_wiki/，不触发 lint。但为语义清晰仍避免歧义词：doc 注释用"提案/合并/度量/审核/未接线"等中性表述（禁词表含"人工"，故不写"人工审核"，用"人审 UI / 人审消费方"即可；虽不被扫但保持定位一致）。

## 非目标（YAGNI）

- 不补 structural_proposals 的 apply worker / 人审 UI（涉红线人审闭环，超一条 Low 范围；KB-06 只标注未接线）。
- 不给 reviewer_stats 加 account 维度（多租户默认关、workspace 级是刻意设计，投机未来 YAGNI；KB-12 只补 doc）。
- 不抽共用函数合并在线/离线 dedup（两路径合并语义不同——在线并 search_queries + source recall_trace + 无 auto-resolve，离线只并 affected + source rule + 有 auto-resolve，强行合并会破坏差异）。
- 不改离线 persist_signals（本就正确）。
- 不动 signal_dedup_key 纯函数及其单测。
