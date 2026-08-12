# 优化线 A · 发送链 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复发送链两个已核证缺陷（毒丸消息行、请示卡滞留），清除 deferred_wake legacy 分支与 manual_send 死代码豁免，清理过时注释。

**Architecture:** 全部改动限于线 A 文件所有权（webhooks/tasks/gateway/outbox*/escalation/quiet_hours）；每任务 TDD（先写失败测试，Docker 集成测试标 `#[ignore]` 本地跑单个）；行为语义保守（fail-safe 方向）。

**Tech Stack:** Rust/Axum、MongoDB testcontainers 集成测试。

## Global Constraints（每任务隐含）

- 模型：仅 Fable 5 1M max（本 worktree 内不再派生 subagent）。
- 红线：动手前把受影响函数全文读懂并当场重验计划里引用的每个 file:line（工作区在演进，行号可能漂移）；发现锚点失效或行为与计划描述不符 → 停该任务并在交付报告标注，不猜测硬改。
- 文件边界：只许修改 `src/webhooks.rs`、`src/tasks.rs`、`src/agent/gateway.rs`、`src/agent/outbox.rs`、`src/agent/outbox_dispatcher.rs`、`src/agent/escalation/**`、`src/agent/quiet_hours.rs` 与 `tests/**` 中对应测试文件。需要触碰其他文件 → 停下登记，不越界。
- 禁词：所有新增代码/注释/事件文案不得含"人工/接管/takeover/hand-off"（提交前跑 `bash scripts/check-no-human-takeover.sh`）。
- 每任务收尾：`cargo check --tests` 无 warning（RUSTFLAGS="-D warnings"）+ 相关单测绿 + 独立 commit（Conventional Commits）。
- 全线收尾：`cargo test --lib` ≥ 2530 passed / 0 failed；四个基线 PBT 全绿。

---

### Task A1: 毒丸消息行修复（缺陷 #2）

**Files:**
- Modify: `src/webhooks.rs`（`reconcile_pending_inbound_handoffs`，约 :803-880 段，动手前重验）
- Test: `tests/` 新建 `poison_inbound_handoff_integration.rs`

**行为契约:**
- 现状：循环内 `mongodb::bson::from_document` 失败 `map_err(...)?` 直接中止整个 reconcile；调用侧 `tasks.rs:1069,1121` 以 `?` 传播使两个 worker 每轮 tick 停摆；坏行按 `created_at` 升序恒排最前形成永久毒丸。
- 目标：decode 失败的行——①写 `handoff_status="quarantined"` + `handoff_updated_at`（直接以 `_id` 定位 update，raw Document 路径不经 typed 反序列化）；②写一条 `agent_events`（kind=`inbound_handoff_quarantined`，details 含 message `_id` 与 decode error 文本，best-effort 失败仅 warn）；③`continue` 处理后续行，函数不再因单行坏数据返回 Err。
- 查询 filter（`$in: [pending, deferred]`）天然排除 quarantined 行——隔离后不再重复扫描。

**Steps:**
- [ ] 读 `reconcile_pending_inbound_handoffs` 全文与 `mark_inbound_handoff`、两处调用点；重验锚点。
- [ ] 写失败集成测试（`#[ignore]`，testcontainers）：插入一条 direction=inbound、handoff_status=pending、但缺 typed 必填字段的坏 Document + 一条其后的正常 pending 行；断言 reconcile 返回 Ok、坏行变 quarantined、正常行完成 handoff（物化出任务）、事件在场。本地 `cargo test --test poison_inbound_handoff_integration -- --ignored` 需 Docker，若本机无 Docker 则至少 `cargo check --tests` 过并在交付报告注明留 CI。
- [ ] 实现隔离逻辑；跑通测试。
- [ ] Commit：`fix(webhooks): quarantine undecodable inbound handoff rows instead of stalling reconcile`

### Task A2: deferred_wake legacy 分支清淤（终裁 01-2）

**Files:**
- Modify: `src/agent/gateway.rs`（4 处 `DEFERRED_INBOUND_REPLY_KIND` 判定：约 :3659、:5266、:7615、:8579，重验）、`src/webhooks.rs`（`reconcile_workspace_reply_obligations` 的 legacy 分支，约 :674-677、:716-717）、`src/agent/quiet_hours.rs`（常量与其测试）
- Test: 现有相关单测更新

**行为契约:**
- 依据：`DEFERRED_INBOUND_REPLY_KIND` 全仓无创建点（22 号终裁+主会话亲证），代码自标 legacy。现行静默唤醒走 `inbound_reply`（Inbound 语义）。
- 目标：删除 gateway 4 处判定分支（`is_deferred_wake` 及其在 quiet_hours/context_changed 门的豁免逻辑随之简化——注意：删除后这两门对 FollowUp 的判定行为必须逐字保持，仅移除永假的 deferred 分支）、webhooks reconcile 的 `$in` 收窄为仅 `DURABLE_INBOUND_REPLY_KIND` 且删 `is_legacy` 分支、quiet_hours.rs 删常量与其断言测试。
- 风险控制：删除前全仓 `rg DEFERRED_INBOUND_REPLY_KIND` 确认仅这些引用；`cargo test --lib` 全量回归确认无行为变化（该分支永假，删除应零行为差）。

**Steps:**
- [ ] 全仓 grep 确认引用封闭；逐处读上下文全文。
- [ ] 删除并简化；确保 `is_deferred_wake` 相关注释一并更新（不留幽灵描述）。
- [ ] `cargo test --lib` 全绿（数量不降）；`cargo check --tests` 过。
- [ ] Commit：`chore(gateway): remove legacy deferred_inbound_reply branches (kind has no producer)`

### Task A3: manual_send 死代码豁免清理（缺陷 #5，保守语义定案）

**Files:**
- Modify: `src/agent/outbox_dispatcher.rs`（`check_contact_status_pure` 约 :2730-2754 与其注释；`second_safety_gate` 注释 :913-929，重验）
- Test: 该纯函数现有单测更新

**行为契约:**
- 语义定案（spec §4-A3）：二次安全门保持对 manual_send 拦截（fail-safe：撤管竞态时宁可取消 admin 已确认的发送）。
- 目标：`check_contact_status_pure` 删除 `SOURCE_KIND_MANUAL_SEND` 豁免（该分支在执行顺序上不可达——二次门先取消）；改写两处注释如实描述"manual_send 与普通托管发送同受撤管即停约束，admin 确认不豁免撤管竞态"；单测同步。
- 若重验发现执行顺序与终裁描述不符（二次门并非先行），停该任务报告。

**Steps:**
- [ ] 读 `process_entry` 中两门的实际调用顺序（约 :2799、:2830 附近）确认不可达成立。
- [ ] 修改纯函数与注释、更新单测；`cargo test --lib` 相关模块绿。
- [ ] Commit：`fix(outbox): drop unreachable manual_send exemption and document conservative unmanage semantics`

### Task A4: delivery_unknown 请示卡滞留修复（23 号终裁）

**Files:**
- Modify: `src/agent/escalation/ledger.rs`（超时扫描 filter，约 :1130-1145，重验）或 `src/agent/escalation/mod.rs` 扫描入口
- Test: escalation 相关集成测试补一条（`#[ignore]`）

**行为契约:**
- 现状：`scan_escalation_timeouts` 只收 `protocol.delivery_state == sent` 且 `last_pushed_at_ms` 为数字的行；投递结果不可核验（delivery_unknown）的请示卡永不进超时改派也无安抚，静默滞留。
- 目标：滞留卡（pending 且 delivery_state ∈ {failed_terminal, delivery_unknown}，或 sent 但缺 last_pushed_at_ms 的异常形态）纳入一个独立收敛分支：按 timeout 语义改派下一位决策人重推卡（复用 `reassign_escalation` + `materialize_principal_card_delivery` 现有原语；改派时刷新 delivery 协议代次）；链尾则走既有 ChainTail 安抚路径。滞留判定的时间基准用 `created_at`（无可信推送时刻）。
- 不改变正常 sent 卡的既有超时语义。

**Steps:**
- [ ] 读 `scan_escalation_timeouts` 全文 + `reassign_escalation` + card delivery 协议（`ledger.rs` 相关段）。
- [ ] 写失败测试：构造 delivery_unknown 的 pending 台账行，断言扫描后被改派（或链尾安抚事件在场）。
- [ ] 实现；测试绿；`cargo test --lib` 全绿。
- [ ] Commit：`fix(escalation): converge stranded principal cards with unverifiable delivery into timeout reassignment`

### Task A5: 过时注释清理

**Files:**
- Modify: `src/agent/gateway.rs`（apply_agent_updates 前置注释段约 :3915；`:2915` 附近退役 key 残留注释；`:5659-5662` 计数注释补"闸门侧行为"说明——A2 完成后该注释需与新现实一致）

**Steps:**
- [ ] 逐处重验注释与现实的差距（对照 24 号/29 号记录），改写为与代码一致的描述。
- [ ] `cargo check` 过；Commit：`docs(gateway): align stale inline comments with current behavior`

### 收尾

- [ ] 全线验证：`cargo test --lib`（≥2530/0 failed）、四 PBT、`RUSTFLAGS="-D warnings" cargo check --tests`、禁词 lint。
- [ ] 交付报告：每任务的锚点重验记录、行为差异说明、留 CI 的 ignored 测试清单、档案回写要点（主会话执行回写）。
