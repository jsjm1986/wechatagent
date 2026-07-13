# 批D家族② 决策请示通道修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修 KD-05（骚扰门统计口径漂移）+ KD-06（孤儿 pending 永不改派）两条决策请示通道 Medium bug；KD-02 经裁决不改代码。

**Architecture:** KD-05 给 `AgentPrincipalEscalation` 加真实"最近推送时刻"字段 `last_pushed_at_ms`（首推+改派刷新，骚扰门查询改用它），补 m031 backfill 历史行。KD-06 让 `next_decider_on_timeout` 在 current 不在链中（改链孤儿）时回落链首、而非静默退化为链尾。

**Tech Stack:** Rust 2021 / Axum / mongodb bson / testcontainers（集成测）。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-12-kd-family2-escalation-channel-design.md`（已获批 commit cb2fc09）。所有行号亲验于 origin/main 36dfda8。
- 红线：改代码前 100% 读懂；引用必亲验 file:line；不靠记忆。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限被本修复有意废除的旧行为。
- **KD-02 不写任何代码**（经用户裁决：字符级领导泄漏词表是威胁模型错误 backstop，同 PR#185 删数字护栏；忠实度交 prompt + 独立 Review Agent）。
- **存储键 snake_case**：`AgentPrincipalEscalation`（models.rs:3702）无 `#[serde(rename_all)]` → 全字段 snake_case 存储。新字段 `last_pushed_at_ms` 及所有 `doc!` 查询键须 snake_case。
- **加 struct 字段须补全 6 处字面量构造点**（否则 E0063）：ledger.rs:40 生产 / logic.rs:487 / ask_human_inbox.rs:605 / tests/ask_human_phase1_e2e.rs:66 / tests/principal_decision_channel.rs:89 / tests/real_llm_principal_relay.rs:395。生产点用 `Some(now)`，测试点用 `None`。**须 `cargo check --tests`（`--lib` 不编译 tests/，漏改测试点的 E0063 只有 --tests 或 CI 暴露）。**
- m031 迁移**不加 APP_ENV=production 守卫**（语义保持型回填，同 m018/m022/m025/m030）。
- baseline `cargo test --lib` ≥ 350 passed / 0 failed 不回退。集成测 `#[ignore]` CI Docker 跑；本地磁盘紧只 `cargo test --lib` + `cargo check --tests`。
- 子任务派 subagent 省略 model 参数（继承主会话 opus）。绝不动任何 sibling worktree 的 target/。

---

## File Structure

- `src/models.rs`：`AgentPrincipalEscalation` 加 `last_pushed_at_ms` 字段（KD-05）。
- `src/agent/escalation/ledger.rs`：insert 初始化 / reassign 刷新 / 两骚扰门查询换键（KD-05）。
- `src/agent/escalation/policy.rs`：`next_decider_on_timeout` position 未命中回落链首（KD-06，独立）。
- `src/db/migrations/m031_backfill_escalation_last_pushed_at.rs` + `mod.rs`：backfill 历史 pending 行（KD-05 治本）。
- 6 处 `AgentPrincipalEscalation {}` 字面量构造点补字段。
- `tests/escalation_push_time_reassign.rs`：KD-05 + m031 集成测。

---

## Task 1: KD-05 —— last_pushed_at_ms 字段 + ledger 读写改造 + 全构造点补齐

**Files:**
- Modify: `src/models.rs:3734-3735`（在 `last_holding_reply_ms` 字段后加新字段）
- Modify: `src/agent/escalation/ledger.rs`（:55 insert 初始化 / :313 reassign $set / :360 count_pushes_today / :379-386 latest_push_ms）
- Modify: `src/agent/escalation/logic.rs:502`（测试字面量）
- Modify: `src/routes/ask_human_inbox.rs:605`（测试字面量）
- Modify: `tests/ask_human_phase1_e2e.rs:66` / `tests/principal_decision_channel.rs:89` / `tests/real_llm_principal_relay.rs:395`（测试字面量）

**Interfaces:**
- Produces: `AgentPrincipalEscalation.last_pushed_at_ms: Option<i64>`；`count_pushes_today` / `latest_push_ms` 行为不变（签名不变，只换内部查询键）。

- [ ] **Step 1: models.rs 加字段**

在 `src/models.rs` `last_holding_reply_ms` 字段（:3734-3735）之后、`created_at`（:3736）之前插入：

```rust
    /// KD-05：本条台账最近一次被推卡给【当前 principal】的时刻（epoch ms）。骚扰门
    /// count_pushes_today / latest_push_ms 用它而非 created_at——改派换 principal 时
    /// created_at 不刷新会低估对 next 的打扰。首推创建时=created_at；每次 reassign 刷新为
    /// 改派时刻。`#[serde(default)]` 兼容旧文档（缺字段→None，由 m031 backfill 补成 created_at）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pushed_at_ms: Option<i64>,
```

- [ ] **Step 2: 补齐 6 处字面量构造点（cargo check --tests 前必须全补，否则 E0063）**

每处在 `last_holding_reply_ms: ...,` 行之后加一行。生产点（ledger.rs:55，其上下文有 `let now = DateTime::now();` at :35）：

`src/agent/escalation/ledger.rs:55`（`last_holding_reply_ms: None,` 后）：
```rust
            last_pushed_at_ms: Some(now.timestamp_millis()),
```

其余 5 处测试字面量（`last_holding_reply_ms: None,` 后各加）：
```rust
            last_pushed_at_ms: None,
```
- `src/agent/escalation/logic.rs:502`（缩进 12 空格，同上下文）
- `src/routes/ask_human_inbox.rs:605` 构造块内
- `tests/ask_human_phase1_e2e.rs:66` 构造块内
- `tests/principal_decision_channel.rs:89` 构造块内
- `tests/real_llm_principal_relay.rs:395` 构造块内

（先读每个文件确认该构造块里 `last_holding_reply_ms` 的确切缩进，逐字对齐后插入。）

- [ ] **Step 3: reassign_escalation 刷新 last_pushed_at_ms**

`src/agent/escalation/ledger.rs:313` 的 `$set`：
```rust
            doc! { "$set": { "principal_wxid": to_wxid, "updated_at": DateTime::now() } },
```
改为：
```rust
            doc! { "$set": {
                "principal_wxid": to_wxid,
                "updated_at": DateTime::now(),
                // KD-05：改派=给 next 的新推送时刻，与 updated_at 同步刷新，骚扰门据此正确计对 next 的打扰。
                "last_pushed_at_ms": DateTime::now().timestamp_millis(),
            } },
```

- [ ] **Step 4: count_pushes_today 查询键换 last_pushed_at_ms**

`src/agent/escalation/ledger.rs:356-361` 的 filter：
```rust
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "created_at": { "$gte": DateTime::from_millis(since_ms) },
            },
```
改为（注意：last_pushed_at_ms 是 i64，用裸 since_ms 而非 DateTime）：
```rust
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                // KD-05：用真实最近推送时刻，而非 created_at（改派后 created_at 不刷新会漏计）。
                "last_pushed_at_ms": { "$gte": since_ms },
            },
```
并更新函数 doc 注释（:346）把"以台账 created_at 作为推送时刻近似"改为"以 last_pushed_at_ms（首推+改派刷新）为推送时刻"。

- [ ] **Step 5: latest_push_ms 排序键+返回值换 last_pushed_at_ms**

`src/agent/escalation/ledger.rs:379-386`：
```rust
        .find_one(
            doc! { "workspace_id": workspace_id, "principal_wxid": principal_wxid },
            mongodb::options::FindOneOptions::builder()
                .sort(doc! { "created_at": -1 })
                .build(),
        )
        .await?;
    Ok(latest.map(|e| e.created_at.timestamp_millis()))
```
改为：
```rust
        .find_one(
            doc! { "workspace_id": workspace_id, "principal_wxid": principal_wxid },
            mongodb::options::FindOneOptions::builder()
                // KD-05：按真实最近推送时刻排序取最近一次推卡时刻（改派刷新后才准）。
                .sort(doc! { "last_pushed_at_ms": -1 })
                .build(),
        )
        .await?;
    // last_pushed_at_ms 已是 epoch ms；旧行缺字段→None（m031 backfill 前），用 created_at 兜底保口径。
    Ok(latest.and_then(|e| e.last_pushed_at_ms.or_else(|| Some(e.created_at.timestamp_millis()))))
```
并更新函数 doc 注释（:368-370）同理。

- [ ] **Step 6: cargo check --tests 确认 6 处构造点全补齐**

Run: `cargo check --tests 2>&1 | grep -E "E0063|error\[|error:" | head`
Expected: 空（无 E0063 缺字段错误）。若报某文件 missing field last_pushed_at_ms → 补该处。

- [ ] **Step 7: 全 lib 测无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 8: Commit**

```bash
git add src/models.rs src/agent/escalation/ledger.rs src/agent/escalation/logic.rs src/routes/ask_human_inbox.rs tests/ask_human_phase1_e2e.rs tests/principal_decision_channel.rs tests/real_llm_principal_relay.rs
git commit -m "fix(escalation): 骚扰门用真实 last_pushed_at_ms 而非 created_at (KD-05)"
```

---

## Task 2: KD-05 治本 —— m031 backfill 迁移

**Files:**
- Create: `src/db/migrations/m031_backfill_escalation_last_pushed_at.rs`
- Modify: `src/db/migrations/mod.rs`（mod 声明 + MIGRATIONS 追加）

**Interfaces:**
- Produces: `pub async fn run_step(db: &Database) -> AppResult<()>`（pub 供集成测）；`pub(super) fn backfill_filter() -> Document`；`pub(super) fn backfill_pipeline() -> Vec<Document>`。

- [ ] **Step 1: 新建 m031 文件（含纯函数单测）**

Create `src/db/migrations/m031_backfill_escalation_last_pushed_at.rs`：

```rust
//! 2026_07_031：回填 agent_principal_escalations 缺失的 last_pushed_at_ms（KD-05 治本）。
//!
//! 背景：KD-05 给台账加 last_pushed_at_ms（骚扰门真实推送时刻，改派刷新）。旧 pending 行
//! 无此字段（serde default→None），count_pushes_today/latest_push_ms 用 $gte/sort 会漏计。
//! 本迁移把现有行的 last_pushed_at_ms 补成 created_at（历史行"最近推送时刻"就近似取创建时刻，
//! 与旧 created_at 口径字节等价）。
//!
//! **不加 APP_ENV=production 守卫**：语义保持型回填（写的就是旧口径值），非破坏、幂等——
//! 与 m018/m022/m025/m030 同类（均无守卫、生产照跑）。误加会致 117 生产静默 SKIP。
//!
//! 幂等：仅 last_pushed_at_ms 缺失的行命中；二次跑 matched=0。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

/// 命中过滤器：缺 last_pushed_at_ms 的台账行（纯函数，便于单测）。
pub(super) fn backfill_filter() -> Document {
    doc! { "last_pushed_at_ms": { "$exists": false } }
}

/// 回填 pipeline：last_pushed_at_ms = created_at 的 epoch ms（纯函数，便于单测）。
/// $toLong($created_at) 把 BSON Date 转 epoch ms（与 last_pushed_at_ms 的 i64 存储一致）。
pub(super) fn backfill_pipeline() -> Vec<Document> {
    vec![doc! { "$set": { "last_pushed_at_ms": { "$toLong": "$created_at" } } }]
}

/// 迁移主体。`pub` 暴露给 tests/ 集成测（同 m018/m029/m030 先例）。
pub async fn run_step(db: &Database) -> AppResult<()> {
    let result = db
        .agent_principal_escalations()
        .update_many(backfill_filter(), backfill_pipeline(), None)
        .await?;
    tracing::info!(
        migration_id = "2026_07_031_backfill_escalation_last_pushed_at",
        modified = result.modified_count,
        matched = result.matched_count,
        "backfilled escalation last_pushed_at_ms from created_at (KD-05)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_targets_missing_field_only() {
        let f = backfill_filter();
        let cond = f.get_document("last_pushed_at_ms").unwrap();
        assert!(!cond.get_bool("$exists").unwrap(), "只命中 last_pushed_at_ms 缺失的行");
    }

    #[test]
    fn pipeline_sets_from_created_at_as_long() {
        let p = backfill_pipeline();
        assert_eq!(p.len(), 1);
        let set = p[0].get_document("$set").unwrap();
        let field = set.get_document("last_pushed_at_ms").unwrap();
        // $toLong($created_at)：BSON Date → epoch ms i64，与字段存储类型一致。
        assert_eq!(field.get_str("$toLong").unwrap(), "$created_at");
    }
}
```

- [ ] **Step 2: 注册 mod 声明**

`src/db/migrations/mod.rs` 在 m030 的 mod 声明后加一行：
```rust
mod m031_backfill_escalation_last_pushed_at;
```
（先 grep 确认 m030 的 mod 声明是 `mod` 还是 `pub mod`——m030 供集成测暴露用了 `pub mod`；m031 也需 `pub mod` 因 Task 4 集成测跨 crate 调 run_step。用 `pub mod m031_backfill_escalation_last_pushed_at;`。）

- [ ] **Step 3: 注册进 MIGRATIONS 数组**

`src/db/migrations/mod.rs` MIGRATIONS 数组末尾（m030 条目后）追加：
```rust
    Migration {
        id: "2026_07_031_backfill_escalation_last_pushed_at",
        run: |db| Box::pin(m031_backfill_escalation_last_pushed_at::run_step(db)),
    },
```

- [ ] **Step 4: 单测 + id 顺序单测**

Run: `cargo test --lib m031 2>&1 | tail -8 && cargo test --lib migration_ids 2>&1 | tail -6`
Expected: m031 两纯函数单测 PASS；`migration_ids_are_unique` + `migration_ids_are_chronologically_ordered` PASS（`2026_07_030` < `2026_07_031`）。

- [ ] **Step 5: 全 lib 测无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/db/migrations/m031_backfill_escalation_last_pushed_at.rs src/db/migrations/mod.rs
git commit -m "feat(migration): m031 回填 escalation last_pushed_at_ms=created_at (KD-05 治本)"
```

---

## Task 3: KD-06 —— next_decider_on_timeout position 未命中回落链首

**Files:**
- Modify: `src/agent/escalation/policy.rs:105-116`（`next_decider_on_timeout` 函数体 + doc）
- Modify: `src/agent/escalation/policy.rs`（tests mod 加 KD-06 用例）

**Interfaces:**
- Consumes: `ResolvedAskHumanPolicy`（policy.rs:8）、`DeciderRef`（models.rs）。
- Produces: `next_decider_on_timeout` 签名不变，返值语义修正（current 不在链→链首而非 None）。

- [ ] **Step 1: 写失败测试（KD-06 四象限）**

在 `src/agent/escalation/policy.rs` 的 `mod tests` 内（`next_decider_none_when_timeout_unset` 之后，:331 附近）加：

```rust
    #[test]
    fn next_decider_orphan_current_falls_back_to_chain_head() {
        // KD-06：admin 改链后当前 principal 已不在链中（孤儿）。旧实现 position(...)? → None
        // → scan 当链尾晾住、永不改派。修复后应回落链首，让孤儿重新入链。
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![
            DeciderRef { wxid: "a".into(), display_name: None },
            DeciderRef { wxid: "b".into(), display_name: None },
        ];
        // 当前 principal "ghost" 不在链中、已超时 → 回落链首 a。
        assert_eq!(
            next_decider_on_timeout(&p, "ghost", 99.0).map(|d| d.wxid.as_str()),
            Some("a"),
            "改链孤儿（current 不在链）超时后须回落链首重新入链，而非静默退化链尾"
        );
    }

    #[test]
    fn next_decider_real_chain_tail_still_none() {
        // KD-06 不得误伤：真链尾（current 是链中最后一位）超时仍返 None（继续等链尾决策人）。
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![
            DeciderRef { wxid: "a".into(), display_name: None },
            DeciderRef { wxid: "b".into(), display_name: None },
        ];
        assert_eq!(
            next_decider_on_timeout(&p, "b", 99.0),
            None,
            "真链尾必须仍返 None（合法继续等），不得被孤儿回落逻辑误伤"
        );
    }

    #[test]
    fn next_decider_orphan_empty_chain_is_none() {
        // 空链 + current 不在链 → first()=None（无人可推，scan 走安抚）。
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![];
        assert_eq!(next_decider_on_timeout(&p, "ghost", 99.0), None);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib next_decider_orphan_current_falls_back_to_chain_head 2>&1 | tail -12`
Expected: FAIL —— 旧实现 `position(...)?` 对 "ghost" 返 None，断言 `Some("a")` 失败。

- [ ] **Step 3: 改 next_decider_on_timeout**

`src/agent/escalation/policy.rs:105-116`：
```rust
pub(crate) fn next_decider_on_timeout<'a>(
    policy: &'a ResolvedAskHumanPolicy,
    current_wxid: &str,
    age_hours: f64,
) -> Option<&'a DeciderRef> {
    let timeout = policy.timeout_hours?;
    if age_hours < timeout {
        return None;
    }
    let idx = policy.decider_chain.iter().position(|d| d.wxid == current_wxid)?;
    policy.decider_chain.get(idx + 1)
}
```
改为：
```rust
pub(crate) fn next_decider_on_timeout<'a>(
    policy: &'a ResolvedAskHumanPolicy,
    current_wxid: &str,
    age_hours: f64,
) -> Option<&'a DeciderRef> {
    let timeout = policy.timeout_hours?;
    if age_hours < timeout {
        return None;
    }
    // KD-06：current 不在链中（admin 改 decider_chain 删/换人后的孤儿 pending）时，
    // 旧 `position(...)?` 返 None → scan 误当链尾永不改派。改为回落链首让孤儿重新入链；
    // current 在链中时保持原语义（下一位；真链尾 get(idx+1)=None → 合法继续等，行为不变）。
    match policy.decider_chain.iter().position(|d| d.wxid == current_wxid) {
        Some(idx) => policy.decider_chain.get(idx + 1),
        None => policy.decider_chain.first(),
    }
}
```
并把函数 doc（:103-104）"已是链尾 → None"补充为"current 不在链（改链孤儿）→ 回落链首；在链中链尾 → None 继续等"。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib next_decider 2>&1 | tail -12`
Expected: PASS —— 三个新用例 + 既有 `next_decider_picks_following_after_timeout` / `next_decider_none_when_timeout_unset` 全绿。

- [ ] **Step 5: 全 lib 测无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/agent/escalation/policy.rs
git commit -m "fix(escalation): next_decider 孤儿(current不在链)回落链首而非退化链尾 (KD-06)"
```

---

## Task 4: 集成测 —— KD-05 改派刷新 + 骚扰门口径 + m031 回填

**Files:**
- Create: `tests/escalation_push_time_reassign.rs`

**Interfaces:**
- Consumes: `m031_backfill_escalation_last_pushed_at::run_step`（Task 2 pub）；`common::TestApp`；`wechatagent::db` 层。因 `reassign_escalation`/`latest_push_ms` 是 `pub(crate)` 不可跨 crate 直调，集成测走 raw Document + 手工模拟改派 $set 验证口径（见 Step 1 说明）。

**说明（实现者先读）**：`reassign_escalation` / `count_pushes_today` / `latest_push_ms` 都是 `pub(crate)`，集成测（独立 crate）不可直调。故集成测用 raw Document 层：(a) 插一条 pending 行 → 手工 `$set` 模拟 reassign（改 principal_wxid + last_pushed_at_ms）→ 断言查询按 last_pushed_at_ms 能正确取到改派时刻；(b) m031 用 `run_step`（pub）验回填。**不要**擅自改 `reassign_escalation` 等的可见性——若认为需要跨 crate 测，记为 finding 报主控裁决。

- [ ] **Step 1: 写集成测**

Create `tests/escalation_push_time_reassign.rs`：

```rust
//! KD-05 端到端：改派刷新 last_pushed_at_ms + 骚扰门按它取推送时刻 + m031 回填历史行。
//!
//! 默认 #[ignore]，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, Document};
use wechatagent::db::migrations::m031_backfill_escalation_last_pushed_at;

use crate::common::TestApp;

/// KD-05：改派把 last_pushed_at_ms 刷新为改派时刻（≠ 陈旧 created_at），按 last_pushed_at_ms
/// sort 取最近推送时刻返回改派时刻。用 raw Document 模拟 reassign 的 $set（reassign_escalation
/// 是 pub(crate) 不可跨 crate 直调）。
#[tokio::test]
#[ignore]
async fn reassign_refreshes_last_pushed_at_ms() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();
    let old_ms = 1_000_000i64;
    let reassign_ms = 9_000_000i64;

    // 首推：principal=A，created_at/last_pushed_at_ms=old。
    raw.collection::<Document>("agent_principal_escalations")
        .insert_one(
            doc! {
                "workspace_id": &ws,
                "account_id": "acc",
                "contact_wxid": "cust",
                "short_code": "E1A2",
                "status": "pending",
                "category": "out_of_scope_decision",
                "reason": "r",
                "question_for_principal": "q",
                "principal_wxid": "A",
                "is_generalizable": false,
                "knowledge_proposal_emitted": false,
                "created_at": mongodb::bson::DateTime::from_millis(old_ms),
                "updated_at": mongodb::bson::DateTime::from_millis(old_ms),
                "last_pushed_at_ms": old_ms,
            },
            None,
        )
        .await
        .expect("seed pending");

    // 模拟 reassign 到 B：$set principal_wxid + last_pushed_at_ms=改派时刻（不动 created_at）。
    raw.collection::<Document>("agent_principal_escalations")
        .update_one(
            doc! { "short_code": "E1A2", "workspace_id": &ws },
            doc! { "$set": { "principal_wxid": "B", "last_pushed_at_ms": reassign_ms } },
            None,
        )
        .await
        .expect("reassign");

    // 按 last_pushed_at_ms 取 B 的最近推送时刻 → 改派时刻（非陈旧 created_at）。
    let row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "principal_wxid": "B", "workspace_id": &ws }, None)
        .await
        .expect("find")
        .expect("exists");
    assert_eq!(row.get_i64("last_pushed_at_ms").unwrap(), reassign_ms, "改派须刷新 last_pushed_at_ms 为改派时刻");
    // created_at 保持不变（真实创建审计）。
    assert_eq!(
        row.get_datetime("created_at").unwrap().timestamp_millis(),
        old_ms,
        "created_at 不被改派篡改（保真实创建审计）"
    );
}

/// m031：缺 last_pushed_at_ms 的历史行回填成 created_at；已有值的行不被覆盖（幂等）。
#[tokio::test]
#[ignore]
async fn m031_backfills_last_pushed_at_from_created_at() {
    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let raw = app.state.db.raw();
    let legacy_created = 2_000_000i64;
    let has_field_created = 3_000_000i64;
    let has_field_pushed = 8_000_000i64;

    // 老行：无 last_pushed_at_ms。
    raw.collection::<Document>("agent_principal_escalations")
        .insert_one(
            doc! {
                "workspace_id": &ws, "account_id": "acc", "contact_wxid": "c1",
                "short_code": "OLD1", "status": "pending", "category": "out_of_scope_decision",
                "reason": "r", "question_for_principal": "q", "principal_wxid": "A",
                "is_generalizable": false, "knowledge_proposal_emitted": false,
                "created_at": mongodb::bson::DateTime::from_millis(legacy_created),
                "updated_at": mongodb::bson::DateTime::from_millis(legacy_created),
            },
            None,
        )
        .await
        .expect("seed legacy");

    // 新行：已有 last_pushed_at_ms（迁移不得覆盖）。
    raw.collection::<Document>("agent_principal_escalations")
        .insert_one(
            doc! {
                "workspace_id": &ws, "account_id": "acc", "contact_wxid": "c2",
                "short_code": "NEW1", "status": "pending", "category": "out_of_scope_decision",
                "reason": "r", "question_for_principal": "q", "principal_wxid": "A",
                "is_generalizable": false, "knowledge_proposal_emitted": false,
                "created_at": mongodb::bson::DateTime::from_millis(has_field_created),
                "updated_at": mongodb::bson::DateTime::from_millis(has_field_created),
                "last_pushed_at_ms": has_field_pushed,
            },
            None,
        )
        .await
        .expect("seed new");

    m031_backfill_escalation_last_pushed_at::run_step(&app.state.db)
        .await
        .expect("run m031");

    let old_row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "short_code": "OLD1", "workspace_id": &ws }, None)
        .await.expect("find").expect("exists");
    assert_eq!(
        old_row.get_i64("last_pushed_at_ms").unwrap(), legacy_created,
        "老行 last_pushed_at_ms 回填成 created_at"
    );

    let new_row = raw
        .collection::<Document>("agent_principal_escalations")
        .find_one(doc! { "short_code": "NEW1", "workspace_id": &ws }, None)
        .await.expect("find").expect("exists");
    assert_eq!(
        new_row.get_i64("last_pushed_at_ms").unwrap(), has_field_pushed,
        "已有 last_pushed_at_ms 的行不被迁移覆盖（幂等）"
    );
}
```

- [ ] **Step 2: 编译集成测（本地无 Docker 不跑）**

Run: `cargo test --test escalation_push_time_reassign --no-run 2>&1 | grep -E "escalation_push_time|error\[|error:|Finished|Executable" | head`
Expected: `Finished` + `Executable ...escalation_push_time_reassign-*.exe`（0 编译错误）。若 E0603（m031 mod 非 pub）→ 确认 Task 2 Step 2 用了 `pub mod`。

- [ ] **Step 3: Commit**

```bash
git add tests/escalation_push_time_reassign.rs
git commit -m "test: KD-05 改派刷新 last_pushed_at_ms + m031 回填(端到端)"
```

---

## Self-Review

**1. Spec coverage：**
- KD-05 字段 + insert/reassign/两查询 → Task 1 ✓
- KD-05 治本 backfill → Task 2（m031，无 APP_ENV 守卫）✓
- KD-06 position 未命中回落链首 + 真链尾不变 → Task 3 ✓
- KD-02 不改代码 → 无 task（设计文档已记录裁决）✓
- 6 处字面量构造点 → Task 1 Step 2 全列 ✓
- 集成测 → Task 4 ✓

**2. Placeholder scan：** 无 TBD/TODO；每个 code step 含完整代码。

**3. Type consistency：**
- `last_pushed_at_ms: Option<i64>` — Task 1 定义、Task 2 backfill 写、Task 4 断言，全 i64/snake_case 一致 ✓
- `run_step(db) -> AppResult<()>` — Task 2 定义、Task 4 消费 ✓
- `count_pushes_today`/`latest_push_ms` 签名不变（只换内部键），调用方（mod.rs:436-437）无需改 ✓
- 迁移 id `2026_07_031` > `2026_07_030` 字符串序 ✓
- 可见性：m031 用 `pub mod`（Task 2 Step 2）；reassign 等 pub(crate) 不跨 crate 调（Task 4 走 raw Document）✓
