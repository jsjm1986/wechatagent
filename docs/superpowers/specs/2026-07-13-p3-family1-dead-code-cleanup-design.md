# P3 家族① 修复设计：死代码清理 / 文档漂移收敛（H-02 / F-02 / KB-05）

> P3 第 1 个修复家族。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` H-02（:341-351）+ F-02（:276-281）+ KB-05（:475-483）。三条均 Low、桶 B（就绪债/死代码/文档漂移），无功能行为变更风险。

## 背景与根因（全部主控当场 Read/Grep 亲验，行号基于 origin/main 427a3ea）

三条 finding 都是「误导性文档漂移 / 死代码分支 / 死桩路由」——不影响当前生效业务逻辑，但会误导后来的读者/维护者。用户已授权「桶 B+桶 C 全清（除桶 A）一条龙」，本家族取三条中的桶 B 死代码/漂移项集中清理。

### H-02（PLAUSIBLE · Low）：run_envelope R0 三函数是未接线的安全基建，doc 却写成"将会接线"

- `src/agent/run_envelope.rs` 模块头 doc（:1-27）声称 R0.1「LLM 调用前 insert lifecycle="started" 信封，确保超时/panic/JSON 失败也有可追溯条目」，并把接线写成将来时："W1 task 2.5 会把 gateway 入口改为先调 `write_run_envelope_started`…"（:23-27）。
- **关键亲验**：`write_run_envelope_started`（:362）/ `update_run_envelope_terminal`（:544）/ `install_panic_hook_for_envelope`（:713）三函数全仓**除定义 + doc 外无任何生产调用点**。gateway 仍走单次 insert 的 `write_agent_run_log_with_finalize`。全仓 Grep `write_run_envelope_started` 仅命中 models.rs:2897（一处 doc 引用）+ run_envelope.rs 自身 + `tests/run_envelope_integration.rs`（#[ignore] 集成测）。
- **澄清（改变处置性质）**：`run_envelope` 模块**整体是活的**——`FINAL_REVIEW_STATUS_VALUES` / `SOURCE_KIND_*` / `derive_lifecycle_from_status` / `is_valid_lifecycle_transition` 等被 gateway/cohort/replay/campaigns/observability 大量使用。死的**只有** R0 生命周期三函数（started/terminal/panic_hook）。它们有专属集成测 `tests/run_envelope_integration.rs`（4 条不变量，#[ignore] CI 跑）+ 模块 doc 明说 W1 task 2.5 会接线。即：**已设计、已写集成测、但未接线**的 R0 安全基建，不是可无脑删的死代码。
- **伤害**：doc 让读者误以为 R0 pre-LLM 追溯已生效；实际「决策前 panic/超时」的 run 不留 started 信封，R0 追溯在生产未生效。

### F-02（PLAUSIBLE · Low）：enqueue 与 dispatcher 的 max_attempts 兜底默认分歧（3 vs 5）

- `src/agent/outbox.rs:244-248` enqueue 兜底 `if req.max_attempts <= 0 { 3 } else { req.max_attempts.min(10) }` —— 落库恒 ≥1。
- `src/agent/outbox_dispatcher.rs:322-326` `schedule_retry_or_terminal` 用 `if entry.max_attempts <= 0 { 5 } else { entry.max_attempts }`。
- `<=0` 分支对 enqueue 产出的 entry 是**死代码**（enqueue 恒落 ≥1），只有手工/历史脏文档会走到。两侧默认值 3 vs 5 不一致，读代码时易误解。当前无生产影响。

### KB-05（PLAUSIBLE · Low）：propose_pack_repair 死桩返 400 但路由仍注册

- `src/routes/knowledge/repair.rs:576-586` `propose_pack_repair` 已下线（`operation_knowledge_items` 集合已删），恒返 400 "operation_knowledge_items has been removed; pack repair temporarily disabled"。
- 路由仍在 `src/routes/mod.rs:690-691` 注册 `POST /operation-knowledge/items/:id/repair`；use 块 :249 导入 `propose_pack_repair`。
- **关键亲验**：前端 `frontend/src` 全仓无 pack repair 调用（grep `items/.*repair` 空）。同区的 `record_repair_applied`（`repair/applied`，chunk 级）前端 `applyAiRepairPatch.ts:38` **真在用**（`__tests__/lib/applyAiRepairPatch.test.ts:34` 覆盖），是活的，**不碰**。

## 目标

消除三处误导：① H-02 把 R0 三函数的接线 doc 从"将会接线"改为"未接线/推迟"的明确标注（保留基建 + 集成测备将来接线）；② F-02 dispatcher 侧兜底默认对齐成 3；③ KB-05 删 pack repair 死桩 + 摘路由。全部零生效业务逻辑变更。

## 架构：三条独立清理

### H-02 —— 只改模块 doc 标注，不删函数、不动集成测（用户裁定）

`src/agent/run_envelope.rs` 模块头 doc（:1-27）改写：明确标注 R0 三函数（started/terminal/panic_hook）**当前生产未接线**——已实现 + 有集成测（`tests/run_envelope_integration.rs` #[ignore]）但 gateway 仍走单次 insert（`write_agent_run_log_with_finalize`），故「决策前 panic/超时」的 run 不留 started 信封、R0 pre-LLM 追溯在生产**未生效**；三函数保留备将来 W1 task 2.5 接线。删除误导性的将来时表述（"W1 task 2.5 会把 gateway 入口改为先调…"这类让读者误以为接线已排期/已生效的措辞）。

**只改注释**：不删任何 pub 函数、不改任何函数体、不动集成测、不动其它模块对 run_envelope 活符号的引用。零编译面变更（除注释）。

### F-02 —— dispatcher 侧兜底默认对齐成 3（用户裁定）

`src/agent/outbox_dispatcher.rs:322-326`：

```rust
// 旧：
let max_attempts = if entry.max_attempts <= 0 { 5 } else { entry.max_attempts };
// 新：
let max_attempts = if entry.max_attempts <= 0 { 3 } else { entry.max_attempts };
```

与 enqueue 侧 `outbox.rs:244` 的 `<=0→3` 一致。`<=0` 对 enqueue 产出 entry 是死分支，对齐后历史脏文档/手工文档也有确定一致行为。加一个单测锁定 dispatcher 侧 `<=0→3`（白盒计算，与 outbox.rs 现有 `enqueue_request_default_max_attempts_clamped` 同风格）。

### KB-05 —— 删死桩 + 摘路由（用户裁定）

- 删 `src/routes/knowledge/repair.rs:576-586` 的 `propose_pack_repair` 函数。
- 摘 `src/routes/mod.rs` 的路由注册 `.route("/operation-knowledge/items/:id/repair", post(propose_pack_repair))`（:690-691）。
- 删 mod.rs use 块（:249）里的 `propose_pack_repair` 导入符号。
- **不碰** `record_repair_applied` / `/operation-knowledge/repair/applied`（:694-695，chunk 级，前端在用）。
- **不碰** `propose_chunk_repair` / `answer_chunk_repair`（chunk 级，活的）。

## 回归风险

1. **H-02 纯注释改**：零编译面/行为变更，不动函数与集成测。
2. **F-02 死分支对齐**：`<=0` 对 enqueue 产出 entry 永不触发，对齐 5→3 不改任何活路径行为；仅统一手工/脏文档的确定性。加单测锁定。
3. **KB-05 删死桩**：函数恒返 400、前端无调用、路由摘除后该端点返 404（本就无人调用）。同区活 handler（record_repair_applied/propose_chunk_repair/answer_chunk_repair）严格不碰。删除后须确认 mod.rs use 块无残留 `propose_pack_repair` 引用致 E0432/E0425 编译错。
4. **baseline**：三条都不触 baseline 门 4 PBT（state_transition/memory_card/wiki_chunk_revision/llm_retry_jitter），`cargo test --lib` ≥ 350 不回退。
5. **check-no-human-takeover lint**：三条改动新增行不含禁词（run_envelope doc / dispatcher 数值 / 删代码）。

## 改动面

- **Modify** `src/agent/run_envelope.rs`：模块头 doc（:1-27）改标注未接线（H-02）。仅注释。
- **Modify** `src/agent/outbox_dispatcher.rs`：:322-326 `<=0` 兜底 5→3（F-02）+ 新增 1 单测。
- **Modify** `src/routes/knowledge/repair.rs`：删 `propose_pack_repair`（:576-586）（KB-05）。
- **Modify** `src/routes/mod.rs`：摘路由 :690-691 + 删 use :249 的 `propose_pack_repair` 导入（KB-05）。

## 测试计划

- **F-02 单测（lib）**：白盒断言 dispatcher 兜底：`entry.max_attempts <= 0` 时 effective max_attempts = 3（与 enqueue 侧一致）；`>0` 时原值透传。
- **KB-05 编译验证**：`cargo check` 通过（摘 use + 删函数 + 摘路由三处一致，无 E0432/E0425）。
- **H-02**：纯注释，无测试需求；`cargo check` 确认注释改动不破编译。
- 全量 `cargo test --lib` ≥ 350 passed / 0 failed 不回退。

## 非目标（YAGNI）

- **不**真接线 run_envelope R0 到 gateway（catch_unwind 改造是另开工程，超出死代码清理范畴，用户裁定只改 doc）。
- **不**删 run_envelope R0 三函数 + 集成测（保留规划的安全基建）。
- **不**碰 record_repair_applied / propose_chunk_repair / answer_chunk_repair（活 handler）。
- **不**做 F-03/F-04（outbox 其它边缘，归后续家族）。
