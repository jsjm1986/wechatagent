# P3 家族⑤ knowledge 就绪债 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修 KB-07（在线 gap 信号 dedup 改全量精确命中，杜绝同 kind 多主题漏合并）+ 给 KB-06/KB-12 补"就绪债现状"doc 标注。

**Architecture:** 三条非同源、一 PR。KB-07 是唯一逻辑改动：`persist_recall_signal` 把"找 existing"从 `find_one({workspace,status,kind}无序)+对单条filter` 换成"全量 `find({workspace,status:pending})` + `.into_iter().find(dedup_key==key)` 精确命中"（对齐离线 persist_signals 查找口径），**保留在线原有合并逻辑一字不动**。KB-06/KB-12 是纯 doc 注释（structural_proposals 模块头补"生产未接线"、reviewer_stats 补"workspace 级刻意"）。全在 src/knowledge_wiki/（亲验不被 no-human-takeover lint 扫）。

**Tech Stack:** Rust 2021，MongoDB（`find` + `try_collect`）。KB-07 是 DB 交互逻辑无独立纯函数 → 集成测（Docker `#[ignore]`）覆盖；KB-06/12 纯注释靠 `cargo check` 确认不破坏编译。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-14-p3-family5-knowledge-readiness-design.md`（已获批 commit ea2113e）。所有行号亲验于分支 `fix/p3-family5-knowledge-readiness`（基于 origin/main c530264 含 #197）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆（行号可能漂移，以 Read 到的真实代码为准）。
- **KB-07 只换"找 existing"这一步**（find_one 无序 → 全量 find + `.find()` 精确命中）。**在线合并逻辑（affected_chunk_ids + search_queries 并集，gap_signals.rs:612-648）+ 新建分支（:650+）一字不动**。绝不抽共用函数合并在线/离线——亲验两路径合并语义不同（在线并 search_queries + source=recall_trace + 无 auto-resolve；离线只并 affected + source=rule + 有 auto-resolve），强行合并会破坏差异。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为"，绝不为过测试改业务逻辑。集成测用类型化 struct 读回、非空断言、真哨兵（回退 find_one 即绿变红）。
- KB-06/KB-12 纯 doc 注释：零行为改动，不改函数/单测。
- check-no-human-takeover lint 亲验只扫 `src/agent/ src/routes/ src/evolution/ frontend/src/`（scripts/check-no-human-takeover.sh:27-30），**src/knowledge_wiki/ 不在范围**——本家族三处改动全在 knowledge_wiki/ 不触发 lint。但注释仍用中性词（提案/合并/度量/未接线/人审 UI；不写"人工审核"）保持定位一致。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不触 4 PBT。signal_dedup_key 单测（gap_signals.rs:1837+）不动。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用）。
- 本地若撞 LNK1318 PDB（Windows-only 非代码错），`cargo check --lib` / `cargo check --tests` 足够验证编译；集成测 `#[ignore]` 靠 CI Docker 跑。

## File Structure

- `src/knowledge_wiki/gap_signals.rs`：`persist_recall_signal`（:596-610）改全量 find + `.find()` 精确命中。KB-07 唯一逻辑改动。
- `src/knowledge_wiki/structural_proposals.rs`：模块头（:1）补"⚠️ 生产未接线（KB-06）"标注。纯注释。
- `src/knowledge_wiki/reviewer_stats.rs`：`aggregate_reviewer_stats_for_workspace`（:49）补 doc。纯注释。
- `tests/wiki_gap_signals_3kinds.rs`：append KB-07 精确合并哨兵（Docker `#[ignore]`）。

Task 1 = KB-07（gap_signals.rs 逻辑 + 集成测哨兵，有测试周期）。Task 2 = KB-06 + KB-12（两处纯 doc 注释，编译确认）。两 task 独立。

---

## Task 1: KB-07 —— 在线 dedup 改全量精确命中（gap_signals.rs + 集成测哨兵）

**Files:**
- Modify: `src/knowledge_wiki/gap_signals.rs:601-610`（`persist_recall_signal` 的"找 existing"步）
- Test: `tests/wiki_gap_signals_3kinds.rs`（append 精确合并哨兵）

**Interfaces:**
- Consumes: `KnowledgeGapSignal`（models，gap_signals.rs:41 已 use）；`signal_dedup_key(kind:&str, title:&str, affected:&[String]) -> String`（gap_signals.rs:470 已存纯函数）；`futures::TryStreamExt`（gap_signals.rs:35 已 use，`try_collect` 可用）。
- Produces: 无签名变化（`persist_recall_signal` 签名不变，仅内部改查找方式）。

- [ ] **Step 1: 亲验当前 persist_recall_signal 全貌 + 离线 persist_signals 查找口径**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "fn persist_recall_signal\|fn persist_signals\|find_one\|\.find(\|try_collect\|signal_dedup_key" src/knowledge_wiki/gap_signals.rs | head -20`
Expected: 确认 `persist_recall_signal`（约 :596）用 find_one（约 :604）、离线 `persist_signals`（约 :484）用 `.find()` + try_collect（约 :491-497）。**实现者 Read 两函数全貌**，确认在线合并块（affected + search_queries 并集）与离线（只并 affected）语义不同——本任务只换在线的"找 existing"步，合并块不动。

- [ ] **Step 2: 改 persist_recall_signal 的"找 existing"步（gap_signals.rs:601-610）**

把（以实现者 Read 到的真实代码为准，形如）：

```rust
    let key = candidate.dedup_key();
    let existing = db
        .knowledge_gap_signals()
        .find_one(
            doc! { "workspace_id": workspace_id, "status": "pending", "kind": &candidate.kind },
            None,
        )
        .await
        .map_err(AppError::from)?
        .filter(|s| signal_dedup_key(&s.kind, &s.title, &s.affected_chunk_ids) == key);
```

替换为（全量 find + `.into_iter().find()` 精确命中，对齐离线查找口径）：

```rust
    let key = candidate.dedup_key();
    // KB-07：全量载入本 workspace 的 pending 信号，按 dedup_key 精确命中该合并的那条。
    // 原 find_one({workspace,status,kind} 无序)+对单条 filter 在同 kind 多主题时只随机看
    // 一条 → 常不匹配 → 漏合并、产重复条。改为全量 find 后按 dedup_key 精确匹配（与离线
    // persist_signals 同查找口径）。在线单候选只需命中一条，用 .find() 而非建 HashMap。
    let pending: Vec<KnowledgeGapSignal> = db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": workspace_id, "status": "pending" },
            None,
        )
        .await
        .map_err(AppError::from)?
        .try_collect()
        .await
        .map_err(AppError::from)?;
    let existing = pending
        .into_iter()
        .find(|s| signal_dedup_key(&s.kind, &s.title, &s.affected_chunk_ids) == key);
```

**其余不动**：`if let Some(existing)` 合并块（affected_chunk_ids + search_queries 并集 + update $set）+ 新建分支保留原样。

- [ ] **Step 3: 编译确认**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --lib 2>&1 | tail -10`
Expected: `Finished`（`KnowledgeGapSignal`/`try_collect`/`signal_dedup_key` 均已在作用域）。若 LNK1318（Windows-only）则 check 已足够。

- [ ] **Step 4: 亲验 GapSignalCandidate 构造方式 + dedup_key 对 title 的归一（写哨兵前必读）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "fn recall_miss_from_product_block\|fn new\|fn signal_dedup_key\|normalize\|to_lowercase\|trim" src/knowledge_wiki/gap_signals.rs | head`
然后 Read `signal_dedup_key`（约 :470-482）+ `recall_miss_from_product_block`（约 :435-455）全貌。

关键事实（实现者须亲验坐实）：
- `GapSignalCandidate::new` 是**私有**；recall_miss 的唯一 pub 构造是 `recall_miss_from_product_block(customer_query: String)`（:435），它写死 title=`"产品宣称缺 verified 知识背书：{query 前40字}"`、severity=high、把 customer_query push 进 search_queries。
- `dedup_key` = `signal_dedup_key(kind, title, affected)`；recall_miss 的 dedup_key 走 `(kind, normalized_title)`（affected 为空）。**title 由 query 前40字决定** → 不同 query（前40字不同）= 不同 title = 不同 dedup_key；相同前40字 = 相同 dedup_key。

- [ ] **Step 5: 写 KB-07 精确合并哨兵（append tests/wiki_gap_signals_3kinds.rs）**

在 `tests/wiki_gap_signals_3kinds.rs` append（该文件已 `use wechatagent::knowledge_wiki::gap_signals;`、有 `TestApp`、`list_pending(app, kind)` helper、`#[ignore]` 范式）。哨兵思路（实现者按亲验的 dedup_key 归一细节落地）：

```rust
/// KB-07 真回归哨兵：同 kind(recall_miss) 下多条不同主题的 pending 信号存在时，
/// persist_recall_signal 必须精确合并进 dedup_key 匹配的那一条，绝不因 find_one 无序
/// 漏合并、误新建重复条。回退到 find_one 无序 → 可能返回不匹配的那条 → 误新建 → count 变 3 → 红。
#[tokio::test]
#[ignore]
async fn recall_signal_merges_correct_topic_among_multiple_pending() {
    let app = TestApp::start().await;
    let ws = /* 用该文件的 WS 常量或 app 默认 workspace，实现者对齐既有测 */;

    // 建两条不同主题(title 前40字不同 → dedup_key 不同)的 recall_miss pending 信号。
    // 用 recall_miss_from_product_block 保证 title/dedup_key 与生产构造完全一致。
    let query_a = "A产品的保修期是多久".to_string();
    let query_b = "B套餐每月流量上限".to_string();
    gap_signals::persist_recall_signal(&app.state.db, ws,
        gap_signals::GapSignalCandidate::recall_miss_from_product_block(query_a.clone()))
        .await.expect("seed A");
    gap_signals::persist_recall_signal(&app.state.db, ws,
        gap_signals::GapSignalCandidate::recall_miss_from_product_block(query_b.clone()))
        .await.expect("seed B");

    let pending = list_pending(&app, "recall_miss").await;
    assert_eq!(pending.len(), 2, "两个不同主题应建 2 条 pending");

    // 再来一次匹配主题 B(title 前40字相同 → dedup_key 命中第2条)但带新 query 变体的信号。
    // 关键：query_b2 的前40字须与 query_b 相同(命中同 dedup_key)、但整串不同(产生新 search_query)。
    // 实现者据 dedup_key 对 title 的归一细节构造 query_b2：若 title=query前40字，则 query_b2
    // = query_b + " 具体是多少"(前40字相同前提下)；若 query_b 已近40字需调整。亲验后定。
    let query_b2 = /* 前40字与 query_b 同、整串不同的 query，实现者按 title 归一亲验构造 */;
    gap_signals::persist_recall_signal(&app.state.db, ws,
        gap_signals::GapSignalCandidate::recall_miss_from_product_block(query_b2.clone()))
        .await.expect("merge into B");

    // 断言：仍 2 条(精确合并进 B、未误新建第 3 条) + B 的 search_queries 累积了新变体。
    let pending2 = list_pending(&app, "recall_miss").await;
    assert_eq!(pending2.len(), 2, "精确合并须仍 2 条,回退 find_one 无序会误新建变 3");
    let b = pending2.iter()
        .find(|s| s.dedup_key_matches(/* B 的 dedup_key，或按 title 前缀找 */))
        .expect("B 仍在");
    assert!(b.search_queries.len() >= 2, "B 应累积 query_b 与 query_b2 两个变体");
}
```

**注意**：上面是哨兵骨架，实现者必须亲验以下后落地精确代码：①`ws` 取值（对齐该文件既有测的 workspace 常量）；②`recall_miss_from_product_block` 的 title 归一规则（前40字如何取、dedup_key 是否对 title 再 normalize）→ 据此构造 query_b2 使其 dedup_key 命中 B 而 search_query 是新的；③`list_pending` 返回的 KnowledgeGapSignal 有无现成方法找 B（否则按 title 前缀或 search_queries 包含 query_b 来定位）。若 title 取前40字导致 query_b2 难以构造"同前缀异整串"，改用**直接 insert_one 两条不同 title 的 KnowledgeGapSignal 做 seed** + 一次 persist_recall_signal 匹配第2条——实现者择更可靠者，但必须保证：seed 阶段同 kind 多条 pending、merge 目标是非首条、断言不新建。

- [ ] **Step 6: 全 lib 测确认无回归**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 7: 集成测编译确认**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --tests --test wiki_gap_signals_3kinds 2>&1 | tail -10`
Expected: `Finished`（哨兵测编译通过；`#[ignore]` 本地不跑靠 CI Docker）。

- [ ] **Step 8: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/knowledge_wiki/gap_signals.rs tests/wiki_gap_signals_3kinds.rs && git commit -m "fix(knowledge): 在线 gap 信号 dedup 改全量精确命中,杜绝同kind多主题漏合并 (KB-07 P3家族⑤)"
```

---

## Task 2: KB-06 + KB-12 —— 就绪债 doc 标注（两处纯注释）

**Files:**
- Modify: `src/knowledge_wiki/structural_proposals.rs:1`（模块头补标注）
- Modify: `src/knowledge_wiki/reviewer_stats.rs`（`aggregate_reviewer_stats_for_workspace` 补 doc）

**Interfaces:**
- Consumes: 无。Produces: 无。纯注释，零行为/签名改动。

- [ ] **Step 1: 亲验两文件当前注释位置**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && sed -n '1,14p' src/knowledge_wiki/structural_proposals.rs && echo "=====" && grep -n "pub async fn aggregate_reviewer_stats_for_workspace\|^///" src/knowledge_wiki/reviewer_stats.rs | head`
Expected: 确认 structural_proposals.rs 模块头（`//!` 块，已有"本轮 out-of-scope"字样）+ reviewer_stats.rs 的 aggregate 函数位置（约 :49）及其上方现有 doc。

- [ ] **Step 2: KB-06 —— structural_proposals.rs 模块头补"生产未接线"标注**

在 `src/knowledge_wiki/structural_proposals.rs` 模块头**最前**（第一行 `//!` 之前，成为新的首行块）插入：

```rust
//! ⚠️ **生产未接线（就绪债 KB-06）**：本模块只产 `status=pending_review` 提案，
//! 全仓无任何 apply worker / 人审 UI 消费方（提案产出后纯躺集合）。这是红线
//! **正确**的一面（AI 绝不自动 apply split/merge），但功能未闭环——接线属下一轮。
//!
```

（放在现有 `//! 结构化写 **意图提案**...` 之前。不改任何 use/struct/fn。）

- [ ] **Step 3: KB-12 —— reviewer_stats.rs aggregate 函数补 doc**

在 `src/knowledge_wiki/reviewer_stats.rs` 的 `pub async fn aggregate_reviewer_stats_for_workspace`（约 :49）**上方现有 doc 之后 / 函数签名之前**补（若函数上方无 doc 则新增）：

```rust
/// ⚠️ **workspace 级聚合（刻意，就绪债 KB-12）**：本函数按 workspace_id 聚合、
/// stat_id=`{workspace_id}::reviewer`（一 workspace 一行），**不带 account_id 维度**
/// ——与 outcome_metrics / outcomes_autonomy 的 (workspace_id, account_id) 双维不同。
/// reviewer 的 prompt/model 是 workspace 级属性，故度量刻意做 workspace 级。若未来
/// 一 workspace 多账号成常态需按账号切,再加 account 维度对齐另两端点。
```

（不改聚合逻辑 / count_documents 过滤 / stat_id / 单测。）

- [ ] **Step 4: 编译确认（纯注释不破坏编译）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --lib 2>&1 | tail -5`
Expected: `Finished`（doc 注释不影响编译）。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed（纯注释零回归）。

- [ ] **Step 6: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/knowledge_wiki/structural_proposals.rs src/knowledge_wiki/reviewer_stats.rs && git commit -m "docs(knowledge): 标注 structural_proposals 未接线 + reviewer_stats workspace级刻意 (KB-06/12 P3家族⑤)"
```

---

## Self-Review 结论

- **Spec coverage**：KB-07（在线 dedup 全量精确命中）→ Task 1；KB-06（structural_proposals 未接线标注）+ KB-12（reviewer_stats workspace 级 doc）→ Task 2。三条 finding 全覆盖。设计"非目标 YAGNI"（不补 apply UI/不加 account 维度/不抽共用函数/不改离线/不动 signal_dedup_key）在计划里通过"只改指定行 + 纯注释"落实。
- **Placeholder scan**：无 TBD/TODO。Task 1 Step 5 哨兵测因依赖 dedup_key 对 title 的归一细节 + GapSignalCandidate 私有构造约束，明确要求实现者亲验后落地精确代码（含备选 seed 策略）——非 placeholder，是"亲验真实构造约束"的红线要求（recall_miss 唯一 pub 构造 recall_miss_from_product_block 写死 title 格式，哨兵须据此构造能命中同 dedup_key 的 query 变体）。
- **Type consistency**：`persist_recall_signal` 签名不变（仅内部改查找）；`KnowledgeGapSignal`/`signal_dedup_key`/`try_collect` 均已在 gap_signals.rs 作用域（:41/:470/:35）。KB-06/12 纯注释无类型。
- **反过拟合**：KB-07 集成测是真哨兵（同 kind 多 pending 精确合并、回退 find_one 无序即误新建变红）。KB-06/12 纯注释无测试冲击。无为过测试改逻辑。
- **红线合规**：在线合并逻辑不动、不抽共用函数（两路径语义不同）、不改离线；lint 亲验 knowledge_wiki 不在扫描范围但注释仍中性；baseline 不回退；worktree 绝对路径。
