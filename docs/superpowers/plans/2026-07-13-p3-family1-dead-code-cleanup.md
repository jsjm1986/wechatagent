# P3 家族① 死代码清理 / 文档漂移收敛 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 清理三处「误导性文档漂移 / 死代码分支 / 死桩路由」——H-02 run_envelope R0 doc 标注未接线、F-02 dispatcher max_attempts 兜底对齐、KB-05 pack repair 死桩清除，全部低风险、不改任何生效业务逻辑。

**Architecture:** 三条独立清理，互不依赖。H-02 只改模块头注释（不删函数、不动集成测）；F-02 一处常量对齐 + 单测；KB-05 删死桩函数 + 摘路由注册 + 删 use 导入。三个 task 各自独立可 review。

**Tech Stack:** Rust 2021，纯函数单测（lib，本地可跑），无 Docker、无新依赖。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-13-p3-family1-dead-code-cleanup-design.md`（已获批 commit 71ee13a）。所有行号亲验于分支 fix/p3-family1-dead-code（基于 origin/main 427a3ea）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为 / 签名变更被迫更新"，绝不为过测试改业务逻辑。
- **H-02 只改 doc 注释**：`run_envelope.rs` 三函数（`write_run_envelope_started` / `update_run_envelope_terminal` / `install_panic_hook_for_envelope`）保留、`tests/run_envelope_integration.rs` 不动。这三函数是已设计+已测+未接线的 R0 安全基建，删它们=毁掉规划的安全特性，明确禁止。
- **KB-05 只碰 `propose_pack_repair`**：同区 `record_repair_apply`（`/operation-knowledge/repair/applied`，chunk 级）前端 `applyAiRepairPatch.ts:38` 真在用，是活的，绝不碰。
- check-no-human-takeover lint 扫 `src/agent/` `src/routes/` `src/evolution/` `frontend/src/` 新增行禁词（`人工接管/接管/人工/takeover/hand-off` 等）。本 PR 改动为删除 + 中性注释，注释文本不得含禁词。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不回退。改动不触 baseline 门 4 PBT（state_transition/memory_card/wiki_chunk_revision/llm_retry_jitter）。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。绝不动任何 sibling worktree 的 target/。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用，误写主仓会污染他人分支）。

---

## File Structure

- `src/agent/run_envelope.rs`：**Modify** 模块头 doc（:1-27）——将 R0 三函数的接线从将来时"W1 task 2.5 会接线"改为明确的"未接线/推迟"标注。仅注释。H-02 全在此文件。
- `src/agent/outbox_dispatcher.rs`：**Modify** 抽私有纯函数 `effective_max_attempts(raw: i32) -> i32`（`<=0→3` 对齐 enqueue）、`schedule_retry_or_terminal`（:322-326）改调该函数 + 在既有 `mod tests`（:1252 前）新增 1 个驱动该真函数的单测。F-02 全在此文件。
- `src/routes/knowledge/repair.rs`：**Delete** `propose_pack_repair` 死桩函数（:576-586）。
- `src/routes/mod.rs`：**Delete** 路由注册（:689-692）+ use 块里的 `propose_pack_repair` 导入（:249）。KB-05 跨这两文件。

三个 task 互不依赖，可按任意顺序，但建议 H-02（纯注释）→ F-02（一处常量+测）→ KB-05（删函数+摘路由）。

---

## Task 1: H-02 —— run_envelope 模块头 doc 标注 R0 未接线（run_envelope.rs）

**Files:**
- Modify: `src/agent/run_envelope.rs:1-27`（模块头 doc 注释）

**Interfaces:**
- Consumes: 无。
- Produces: 无对外接口变化（纯注释）。

- [ ] **Step 1: 改模块头 doc**

把 `src/agent/run_envelope.rs:1-27` 的整段模块头 doc：

```rust
//! Run Envelope 模块（agent-autonomy-loop W1 / Task 2.4）。
//!
//! 本模块负责 [`AgentRunLog`] 的 R0 Run Envelope 生命周期：
//!
//! * [`write_run_envelope_started`]：在任何 LLM 调用之前 `insert_one` 一条
//!   `lifecycle="started"` 的信封记录，确保即使 Reply Agent 超时 / panic /
//!   JSON 解析失败也有可追溯条目（requirements.md R0.1 / R0.5）。
//! * [`update_run_envelope_terminal`]：用 `update_one({run_id}, $set)` 落终态字段；
//!   `matched_count == 0` 时走单次 `insert_one` 兜底 + 写
//!   `agent_events kind="run_envelope_recovered_via_insert"`（R0.2）。
//! * [`install_panic_hook_for_envelope`]：注册全局 `std::panic::set_hook`，把
//!   panic message + location 通过 `tracing::error!` 输出。**实际的 lifecycle
//!   推进**仍然在 W1 task 2.5 的 `catch_unwind` 包装层完成（panic hook 不能直接
//!   调 async update_one；强行 spawn 会有 panic-in-panic 风险）。
//!
//! 使用顺序（W1 task 2.5 接入）：
//! ```text
//! write_run_envelope_started(&db, &run_id, ..).await?;
//! let result = std::panic::catch_unwind(|| run_pipeline()).unwrap_or_else(|_| failed_terminal());
//! update_run_envelope_terminal(&db, &run_id, build_terminal_fields(&result)).await?;
//! ```
//!
//! 与现有 `write_agent_run_log`（`src/agent/gateway.rs`）的关系：
//! 现阶段（W1 task 2.4）`write_agent_run_log` 仍走 `insert_one` 直接落最终
//! 字段；W1 task 2.5 会把 gateway 入口改为先调 [`write_run_envelope_started`]、
//! 主流程结束（含错误路径）调 [`update_run_envelope_terminal`]，从此告别
//! 多次 insert 引发的 DuplicateKey 风险。
```

替换为（把将来时的接线承诺改成明确的"未接线/推迟"现状标注，消除"pre-LLM 追溯已生效"的错觉）：

```rust
//! Run Envelope 模块（agent-autonomy-loop W1 / Task 2.4）。
//!
//! 本模块提供 [`AgentRunLog`] 的 R0 Run Envelope 生命周期原语。
//!
//! ⚠️ **接线状态（2026-07-13 核实）：R0 生命周期三函数在生产未接线。**
//! 下述 `write_run_envelope_started` / `update_run_envelope_terminal` /
//! `install_panic_hook_for_envelope` 均已实现并有集成测覆盖
//! （`tests/run_envelope_integration.rs`，4 条不变量，`#[ignore]` / CI 跑），
//! 但**没有任何生产调用点**：gateway 仍走单次 `insert_one` 的
//! `write_agent_run_log_with_finalize`（`src/agent/gateway.rs`），并未先写
//! `lifecycle="started"` 信封。因此「决策产出前 panic / 超时」的 run 目前
//! **不留** started 信封，R0.1 的 pre-LLM 可追溯不变量在生产**尚未生效**。
//! 对「决策已产出」的 run，单次 insert 的追溯是完整的；缺口仅限决策前的
//! 极端 run。三函数保留备将来接线（见下方各自 doc 的接入设想），本模块的
//! 其余常量 / 纯函数（`FINAL_REVIEW_STATUS_VALUES` / `SOURCE_KIND_*` /
//! `derive_lifecycle_from_status` 等）已被 gateway / cohort / replay /
//! observability 正常使用，不受此接线状态影响。
//!
//! * [`write_run_envelope_started`]：在任何 LLM 调用之前 `insert_one` 一条
//!   `lifecycle="started"` 的信封记录，确保即使 Reply Agent 超时 / panic /
//!   JSON 解析失败也有可追溯条目（requirements.md R0.1 / R0.5）。**未接线。**
//! * [`update_run_envelope_terminal`]：用 `update_one({run_id}, $set)` 落终态字段；
//!   `matched_count == 0` 时走单次 `insert_one` 兜底 + 写
//!   `agent_events kind="run_envelope_recovered_via_insert"`（R0.2）。**未接线。**
//! * [`install_panic_hook_for_envelope`]：注册全局 `std::panic::set_hook`，把
//!   panic message + location 通过 `tracing::error!` 输出。**实际的 lifecycle
//!   推进**需在 `catch_unwind` 包装层完成（panic hook 不能直接调 async
//!   update_one；强行 spawn 会有 panic-in-panic 风险）。**未接线。**
//!
//! 将来接线设想（尚未落地）：
//! ```text
//! write_run_envelope_started(&db, &run_id, ..).await?;
//! let result = std::panic::catch_unwind(|| run_pipeline()).unwrap_or_else(|_| failed_terminal());
//! update_run_envelope_terminal(&db, &run_id, build_terminal_fields(&result)).await?;
//! ```
//! 接线后 gateway 入口先调 [`write_run_envelope_started`]、主流程结束（含错误
//! 路径）调 [`update_run_envelope_terminal`]，即可告别多次 insert 引发的
//! DuplicateKey 风险。
```

- [ ] **Step 2: 编译确认无破坏**

Run: `cargo build --lib 2>&1 | tail -5`
Expected: 编译通过（纯注释改动，无代码变更）。本地若撞 LNK1318 PDB 链接错（已知 Windows-only，非代码错），改用 `cargo check --lib 2>&1 | tail -5`，Expected: `Finished`。

- [ ] **Step 3: Commit**

```bash
git add src/agent/run_envelope.rs
git commit -m "docs(run_envelope): 标注 R0 生命周期三函数生产未接线,消除 pre-LLM 追溯已生效的错觉 (H-02 P3家族①)"
```

---

## Task 2: F-02 —— dispatcher max_attempts 兜底抽纯函数并对齐 enqueue（outbox_dispatcher.rs）

**Files:**
- Modify: `src/agent/outbox_dispatcher.rs`（抽私有纯函数 `effective_max_attempts` + `schedule_retry_or_terminal` :322-326 改调它）
- Test: `src/agent/outbox_dispatcher.rs`（既有 `mod tests` 内新增 1 单测驱动真函数）

**Interfaces:**
- Consumes: `OutboxEntry`（字段 `max_attempts: i32`，已亲验 outbox.rs:148）。
- Produces: 新私有纯函数 `fn effective_max_attempts(raw: i32) -> i32`（模块内私有，`schedule_retry_or_terminal` 与单测共用）；无对外接口变化。

**为何抽函数而非内联复刻表达式**：dispatcher 的 `mod tests` 既有惯例是对纯函数（`backoff_with_jitter_seeded` :951 / `decide_cap_action` :1181）做真单测。若单测里复刻一个 lambda 断言自身（如 enqueue 侧 outbox.rs:928 的 `enqueue_request_default_max_attempts_clamped` 那样），测试与生产代码是两份独立表达式、生产改动测试不会红——是 tautological 空测，SDD reviewer 会打回。抽成真函数让单测驱动生产同一份逻辑，改回 `<=0→5` 即变红，是有效回归哨兵。

- [ ] **Step 1: 先写单测（先写，验证会编译失败——函数尚不存在）**

在 `src/agent/outbox_dispatcher.rs` 的既有 `mod tests`（结尾 `legacy_outbox_doc_defaults_reclaimed_in_flight_false` 之后、闭合 `mod tests` 的 `}` 即 :1252 那行 `}` 之前）新增：

```rust
    /// F-02：dispatcher 侧 max_attempts 兜底须与 enqueue 侧（outbox.rs:244 `<=0→3`）
    /// 同口径。历史脏文档 / 手工写入的 max_attempts<=0 时，两处兜底一致才有确定行为。
    /// 该分支对 enqueue 正常产出的 entry 是死代码（enqueue 恒产出 ≥1），此测锁定口径对齐。
    /// 驱动生产纯函数 effective_max_attempts——改回 `<=0→5` 即变红（真回归哨兵，非 tautology）。
    #[test]
    fn effective_max_attempts_fallback_aligns_with_enqueue() {
        assert_eq!(effective_max_attempts(0), 3, "max_attempts=0 兜底须为 3(对齐 enqueue outbox.rs:244)");
        assert_eq!(effective_max_attempts(-1), 3, "max_attempts<0 兜底须为 3");
        assert_eq!(effective_max_attempts(1), 1, "max_attempts>0 原样透传");
        assert_eq!(effective_max_attempts(5), 5, "max_attempts>0 原样透传");
    }
```

- [ ] **Step 2: 运行确认编译失败**

Run: `cargo test --lib effective_max_attempts_fallback_aligns_with_enqueue 2>&1 | tail -20`
Expected: 编译错误 E0425（`effective_max_attempts` 未定义）。

- [ ] **Step 3: 抽纯函数 + schedule_retry_or_terminal 改调它**

在 `src/agent/outbox_dispatcher.rs` 的 `schedule_retry_or_terminal`（:313）**函数定义之前**（紧邻其上方 doc 注释 :308-312 之前，即 :307 那行 `}` 之后）新增私有纯函数：

```rust
/// F-02：max_attempts 兜底口径——与 enqueue 侧（`outbox.rs:244` `<=0→3`）对齐。
/// enqueue 恒产出 ≥1，故 `<=0` 分支对正常入队 entry 是死代码；仅历史脏文档 /
/// 手工写入的 `<=0` 走到，两处同口径才有确定一致行为。
fn effective_max_attempts(raw: i32) -> i32 {
    if raw <= 0 {
        3
    } else {
        raw
    }
}
```

再把 `schedule_retry_or_terminal` 内 `src/agent/outbox_dispatcher.rs:322-326`：

```rust
    let max_attempts = if entry.max_attempts <= 0 {
        5
    } else {
        entry.max_attempts
    };
```

替换为：

```rust
    let max_attempts = effective_max_attempts(entry.max_attempts);
```

- [ ] **Step 4: 运行确认单测通过**

Run: `cargo test --lib effective_max_attempts_fallback_aligns_with_enqueue 2>&1 | tail -20`
Expected: PASS。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/agent/outbox_dispatcher.rs
git commit -m "fix(outbox): dispatcher max_attempts 兜底抽纯函数并 5→3 对齐 enqueue (F-02 P3家族①)"
```

---

## Task 3: KB-05 —— pack repair 死桩清除（repair.rs + mod.rs）

**Files:**
- Delete: `src/routes/knowledge/repair.rs:576-586`（`propose_pack_repair` 死桩函数）
- Delete: `src/routes/mod.rs:689-692`（路由注册 `/operation-knowledge/items/:id/repair`）
- Delete: `src/routes/mod.rs:249`（use 块里的 `propose_pack_repair` 导入）

**Interfaces:**
- Consumes: 无。
- Produces: 移除路由 `POST /operation-knowledge/items/:id/repair`（死桩恒返 400，前端 `frontend/src` 无调用，已 grep 亲验）。

- [ ] **Step 1: 删死桩函数**

删除 `src/routes/knowledge/repair.rs:576-586` 的整个函数：

```rust
pub(in crate::routes) async fn propose_pack_repair(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> AppResult<Json<Value>> {
    // operation_knowledge_items 已删除；pack-level 修复路径暂时下线，
    // 等 wiki Phase 重新规划包级别 repair。
    Err(AppError::BadRequest(
        "operation_knowledge_items has been removed; pack repair temporarily disabled"
            .to_string(),
    ))
}
```

（删除后，其上方的 `propose_chunk_repair` 相关代码与下方 `classify_extras_kind` 直接相邻，均不受影响。）

- [ ] **Step 2: 摘路由注册**

删除 `src/routes/mod.rs:689-692` 的路由块：

```rust
        .route(
            "/operation-knowledge/items/:id/repair",
            post(propose_pack_repair),
        )
```

（保留其后 :693-696 的 `/operation-knowledge/repair/applied` → `record_repair_apply`——它是活的 chunk 级路由，前端在用。删除后 `analyze_operation_knowledge_logs` 路由块直接接 `record_repair_apply` 路由块。）

- [ ] **Step 3: 删 use 导入**

`src/routes/mod.rs:249` 当前：

```rust
    propose_chunk_repair, propose_pack_repair,
```

改为（去掉 `propose_pack_repair`）：

```rust
    propose_chunk_repair,
```

- [ ] **Step 4: 编译确认无残留引用**

Run: `cargo check --lib 2>&1 | tail -15`
Expected: `Finished`——无 `propose_pack_repair` 未定义 / 未使用导入 / 未使用 import（`AppError` 等仍被 repair.rs 其它函数使用，不会因删这一个函数变未使用；若 check 报 unused import 再按提示清理）。

- [ ] **Step 5: 全 lib 测确认无回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 6: Commit**

```bash
git add src/routes/knowledge/repair.rs src/routes/mod.rs
git commit -m "fix(routes): 删 pack repair 死桩 propose_pack_repair + 摘路由注册,消除误导性 400 入口 (KB-05 P3家族①)"
```

---

## Self-Review 结论

- **Spec coverage**：H-02（doc 标注）→ Task 1；F-02（兜底对齐）→ Task 2；KB-05（死桩清除）→ Task 3。三条 finding 全覆盖。
- **Placeholder scan**：无 TBD/TODO，每步含完整可编译代码 + 精确命令 + 期望输出。
- **Type consistency**：Task 2 抽私有纯函数 `effective_max_attempts(raw: i32) -> i32`，生产（`schedule_retry_or_terminal` 改调它）与单测（驱动同一函数）共用一份逻辑，非两份复刻；Task 3 删除的 `propose_pack_repair` 在函数定义（repair.rs）、路由注册（mod.rs:690）、use 导入（mod.rs:249）三处一并删除，无残留引用。
- **测试有效性（反 tautology）**：Task 2 单测驱动生产纯函数 `effective_max_attempts`，改回 `<=0→5` 即变红，是真回归哨兵——刻意不复刻 enqueue 侧 `enqueue_request_default_max_attempts_clamped`（outbox.rs:928）那种断言本地 lambda 的 tautological 写法。
- **既有测试冲击**：Task 1 纯注释零冲击；Task 2 新增单测 + 抽函数（`schedule_retry_or_terminal` 行为不变，只是兜底 5→3），不改既有断言；Task 3 删死桩——前端无调用（已 grep 亲验）、无 lib 单测断言此路由，零冲击。
- **红线合规**：H-02 保留三函数 + 集成测（不毁安全基建）；KB-05 不碰 `record_repair_apply`（活路由）；无禁词。
