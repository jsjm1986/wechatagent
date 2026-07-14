# P3 家族⑨ outbox/抢占时序 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** F-04（reclaim 累加 reclaim_count，>5 转 failed_terminal，止住 worker 反复崩溃的无限 reclaim）+ B-03（second_safety_gate 发送前 fresh 复核 managed，决策期翻 normal 时拦截）+ B-01（抢占入队尾窗双回复标注为已知产品取舍，不改逻辑）。

**Architecture:** F-04 在 `reclaim_expired_leases`（outbox_dispatcher.rs）reclaim 的 update 加 `$inc reclaim_count`，再单独一个 `update_many` 把 `reclaim_count > 5` 转 failed_terminal（单 update 无法基于 inc 后值分流）。B-03 给纯函数 `check_second_safety_gate_pure`（**outbox.rs**，非 dispatcher）加 `is_managed` 末参，外层 `second_safety_gate` 从已 fresh 查的 contact 取 `agent_status==Managed` 传入，非 managed → `Some("not_managed_at_send")`。B-01 在 gateway.rs:2513 兜底 guard 附近加注释。`OutboxEntry` 加 `reclaim_count: i32`（serde default）。

**Tech Stack:** Rust 2021，Axum，MongoDB。B-03 纯函数 → lib 单测（本地可跑）；F-04 DB 语义 → 终审亲验 + 集成测；B-01 纯注释。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-14-p3-family9-outbox-timing-design.md`（已获批 commit a66c79b）。所有行号亲验于分支 `fix/p3-family9-outbox-timing`（基于含 #202 的最新 origin/main）。
- **设计文档一处 file 修正**：设计里 B-03 写 `check_second_safety_gate_pure` 在 outbox_dispatcher.rs——**实际在 `src/agent/outbox.rs:497`**（主控亲验）。`second_safety_gate` 外层在 outbox_dispatcher.rs:179。以本计划的真实位置为准。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- **F-04**：reclaim_count 独立计数（不复用 max_attempts，reclaim ≠ 发送 attempt）；`OUTBOX_MAX_RECLAIMS=5` 文件常量；第二个 update_many 幂等。不动 atomic_claim_pending / 正常发送。
- **B-03**：只给 check_second_safety_gate_pure 加 `is_managed: bool` 末参 + 函数体最前 `if !is_managed { return Some(...) }`；second_safety_gate 外层取 is_managed 传入。contact=None → is_managed=false。不动 precheck_send_gateway 入参快照判定。
- **B-01**：纯注释，不改任何逻辑。
- 反过拟合：B-03 纯函数新哨兵真驱动（回退删 is_managed 行即红）；既有 check_second_safety_gate_pure lib 测补 is_managed=true 末参，旧断言值不变。
- check-no-human-takeover lint 扫 src/agent/——新增注释/reason 中性词（发送/复核/账号/运营/终态/抢占/尾窗），`not_managed_at_send` 是 AI 内部状态名非"人工接管"，无禁词（人工/接管/takeover/hand-off）。
- check-no-model-hint lint 扫 src/——新增行无模型/品牌名。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不触 4 PBT。
- 子任务派 subagent 省略 model 参数。**所有路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**。
- 本地若撞 LNK1318（Windows-only），`cargo test --lib check_second_safety_gate`（纯函数测能跑）+ `cargo check --lib` 足够。

## File Structure

- `src/models.rs`：`OutboxEntry`（:2987-3027）加 `reclaim_count: i32`（serde default）。
- `src/agent/outbox_dispatcher.rs`：F-04 `reclaim_expired_leases`（:98-128）加 $inc + 第二个 update_many + 常量；B-03 `second_safety_gate`（:179-227）外层取 is_managed 传入。
- `src/agent/outbox.rs`：B-03 `check_second_safety_gate_pure`（:497-520）加 is_managed 参 + 函数体判断；mod tests 补哨兵测 + 既有测补参。
- `src/agent/gateway.rs`：B-01 注释（:2513 附近）。
- `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：B-01 状态标注 + F-04/B-03 状态→Fixed。

单 Task（多文件但都服务这三条紧密相关的 outbox 时序加固，一个测试周期）。

---

## Task 1: outbox 时序三条加固（F-04 + B-03 + B-01 标注）

**Files:**
- Modify: `src/models.rs:2987-3027`（OutboxEntry 加 reclaim_count）
- Modify: `src/agent/outbox_dispatcher.rs:98-128`（F-04 reclaim）+ `:179-227`（B-03 second_safety_gate 外层）
- Modify: `src/agent/outbox.rs:497-520`（B-03 纯函数）+ mod tests（哨兵测 + 既有测补参）
- Modify: `src/agent/gateway.rs:2513` 附近（B-01 注释）
- Modify: `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（状态行）

**Interfaces:**
- Consumes: `OutboxStatus`（outbox.rs:41，`InFlight/Pending/FailedTerminal`，`.as_str()`）；`AgentStatus`（models.rs，`Managed`）；`Contact.agent_status`。
- Produces: `check_second_safety_gate_pure(now_ms, entry_created_ms, cooldown_until_ms, last_inbound_ms, outcome, decision_created_ms, stale_threshold_ms, is_managed: bool) -> Option<String>`（8 参）；`OutboxEntry.reclaim_count: i32`。

- [ ] **Step 1: 亲验五处真实现状**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "fn reclaim_expired_leases\|fn second_safety_gate\|fn check_second_safety_gate_pure\|struct OutboxEntry\|reclaimed_in_flight\|should_run_send\|is_managed" src/agent/outbox_dispatcher.rs src/agent/outbox.rs src/models.rs src/agent/gateway.rs | head -30`
Expected: 确认 reclaim_expired_leases（outbox_dispatcher.rs 约 :98）、second_safety_gate（约 :179）、check_second_safety_gate_pure（**outbox.rs 约 :497**）、OutboxEntry（models.rs 约 :2987，含 reclaimed_in_flight :3022-3023）、gateway.rs 兜底 guard（约 :2513）。**实现者 Read 这五段全貌**后再改。

- [ ] **Step 2: OutboxEntry 加 reclaim_count 字段**

在 `src/models.rs` 的 `OutboxEntry`（reclaimed_in_flight 字段 :3022-3023 之后）加：

```rust
    /// F-04：被 `reclaim_expired_leases` 回收的累计次数（每次 lease 过期 reclaim +1）。
    /// worker 反复在同位置崩溃 → 无限 reclaim 永不进终态，超 OUTBOX_MAX_RECLAIMS 转
    /// failed_terminal 止损。reclaim ≠ 发送 attempt，故独立计数不复用 max_attempts。
    /// `#[serde(default)]` 兼容旧文档（=0）。
    #[serde(default)]
    pub reclaim_count: i32,
```

（加在 reclaimed_in_flight 之后、created_at 之前。实现者亲验真实字段顺序，插在合适位置。）

- [ ] **Step 3: F-04 —— reclaim_expired_leases 加 $inc + 超限转终态**

在 `src/agent/outbox_dispatcher.rs`，文件常量区（如 ACCOUNT_OFFLINE_DEFER_SECONDS :79 附近）加：

```rust
/// F-04：单条 entry 允许被 reclaim 的上限。超过则转 failed_terminal——worker 反复在
/// 同位置崩溃（无限 reclaim 永不进终态）时止损交 admin。reclaim ≠ 发送 attempt。
const OUTBOX_MAX_RECLAIMS: i32 = 5;
```

把 reclaim_expired_leases（约 :98-128）的 update_many 加 `$inc`，并在其后加第二个 update_many：

```rust
    let result = collection
        .update_many(
            doc! {
                "status": OutboxStatus::InFlight.as_str(),
                "locked_until": { "$lt": now },
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::Pending.as_str(),
                    "reclaimed_in_flight": true,
                    "updated_at": now,
                },
                "$unset": { "worker_id": "", "locked_until": "" },
                "$inc": { "reclaim_count": 1 },
            },
            None,
        )
        .await?;
    if result.modified_count > 0 {
        tracing::info!(
            modified_count = result.modified_count,
            "outbox dispatcher reclaimed expired leases"
        );
    }
    // F-04：reclaim_count 超上限的 entry 转 failed_terminal。单 update 无法按 $inc 后的
    // 新值分流，故单独一遍 update_many（幂等：已 failed_terminal 不再匹配 status:Pending）。
    let terminated = collection
        .update_many(
            doc! {
                "status": OutboxStatus::Pending.as_str(),
                "reclaim_count": { "$gt": OUTBOX_MAX_RECLAIMS },
            },
            doc! {
                "$set": {
                    "status": OutboxStatus::FailedTerminal.as_str(),
                    "updated_at": now,
                    "last_error": "reclaim 超限（worker 反复崩溃，止损转终态）",
                }
            },
            None,
        )
        .await?;
    if terminated.modified_count > 0 {
        tracing::warn!(
            terminated = terminated.modified_count,
            "outbox reclaim 超限转 failed_terminal"
        );
    }
    Ok(result.modified_count)
```

（返回值保持 `result.modified_count`（reclaim 条数），不变。实现者按真实代码块精确替换，保留原 tracing::info。）

- [ ] **Step 4: B-03 —— check_second_safety_gate_pure 加 is_managed 参（outbox.rs:497）**

把 `src/agent/outbox.rs` 的 check_second_safety_gate_pure（:497-520）签名加末参 + 函数体最前加判断：

```rust
pub(crate) fn check_second_safety_gate_pure(
    now_ms: i64,
    entry_created_ms: i64,
    cooldown_until_ms: Option<i64>,
    last_inbound_ms: Option<i64>,
    outcome: &str,
    decision_created_ms: i64,
    stale_threshold_ms: i64,
    is_managed: bool,
) -> Option<String> {
    // B-03：发送前 fresh 复核 managed。决策运行期（~10-15s）admin 把 contact 改 normal 想
    // 立即止住 AI，precheck 的入参快照复核不到；dispatcher 发送前 fresh 查 contact 是最接近
    // 实际发送的复核点，非 managed（含 contact 被删 → is_managed=false）→ 拦截，不发在途回复。
    if !is_managed {
        return Some("not_managed_at_send".to_string());
    }
    if let Some(cooldown) = cooldown_until_ms {
        if cooldown > now_ms {
            return Some("contact_cooldown_active".to_string());
        }
    }
    if let Some(last_inbound) = last_inbound_ms {
        if last_inbound > decision_created_ms && outcome_signals_stop(outcome) {
            return Some("user_stop_requested_after_decision".to_string());
        }
    }
    if now_ms.saturating_sub(entry_created_ms) > stale_threshold_ms {
        return Some("outbox_stale_30min".to_string());
    }
    None
}
```

- [ ] **Step 5: B-03 —— second_safety_gate 外层取 is_managed 传入（outbox_dispatcher.rs:179-227）**

在 `src/agent/outbox_dispatcher.rs` 的 second_safety_gate 里（已 fresh 查 contact `:184-195`，用 `contact` 变量），在调用 check_second_safety_gate_pure（约 :218）前算 is_managed 并作末参传入：

```rust
    let is_managed = contact
        .as_ref()
        .map_or(false, |c| c.agent_status == AgentStatus::Managed);
    let decision_created_ms = entry.created_at.timestamp_millis();
    Ok(check_second_safety_gate_pure(
        now.timestamp_millis(),
        entry.created_at.timestamp_millis(),
        cooldown_until_ms,
        last_inbound_ms,
        &outcome,
        decision_created_ms,
        STALE_THRESHOLD_MILLIS,
        is_managed,
    ))
```

（`contact` 是 :184 的 `find_one` 结果 `Option<Contact>`——实现者亲验变量名。`AgentStatus` 若未 import 加 `use crate::models::AgentStatus;`（或亲验现有 import 路径）。实现者确认 `Contact.agent_status` 字段类型是 `AgentStatus` 枚举。）

- [ ] **Step 6: B-03 —— mod tests 补哨兵测 + 既有测补参**

先 grep 既有 check_second_safety_gate_pure 的 lib 测：
Run: `grep -n "check_second_safety_gate_pure" src/agent/outbox.rs`
给**每个既有调用**补末参 `true`（is_managed=true，保持旧断言值不变——旧测都是 managed 场景的其它闸判定）。

再在 outbox.rs mod tests 补两个新哨兵测：

```rust
    #[test]
    fn second_gate_blocks_when_not_managed() {
        // B-03：发送前非 managed（决策期 admin 改 normal / contact 被删）→ 拦截。
        let now = 1_000_000_000_000i64;
        let r = check_second_safety_gate_pure(now, now, None, None, "", now, 30 * 60 * 1000, false);
        assert_eq!(r, Some("not_managed_at_send".to_string()), "非 managed 必须拦截");
    }

    #[test]
    fn second_gate_managed_normal_passes() {
        // is_managed=true + 无 cooldown/stop/陈旧 → None（不误伤正常发送）。
        let now = 1_000_000_000_000i64;
        let r = check_second_safety_gate_pure(now, now, None, None, "", now, 30 * 60 * 1000, true);
        assert_eq!(r, None, "managed 且其它闸未命中应放行");
    }
```

（真哨兵：回退删 `if !is_managed` 行 → not_managed 测返 None → 变红。实现者亲验 mod tests 里 STALE_THRESHOLD 相关既有测的构造范式对齐。）

- [ ] **Step 7: B-01 —— gateway.rs 兜底 guard 附近加注释（不改逻辑）**

在 `src/agent/gateway.rs` 兜底 guard（约 :2513 `if let Some(guard) = &should_abort_send`，在 `if should_run_send(...)` 内）之前加注释：

```rust
        // B-01（已知产品取舍，暂不修）：此兜底 guard 到下方多段 enqueue 循环之间仍有极窄
        // 尾窗（每段一次 outbox_enqueue DB 往返，约 10-100ms）——新入站若恰落在此窗口内，
        // 本轮过时回复会全部 enqueue，同时 runner 检测 generation 变化重算再 enqueue 一批，
        // 两批 segment 幂等 key 不同不互相去重 → 客户可能收两次回复。彻底消除需"入队后按
        // run_id/generation 撤销上一 gen 的 pending outbox"补偿（EnqueueRequest 已带 run_id），
        // 但该方案触碰 outbox 幂等核心 + runner 重算交互（并发正确性改动，风险最高），且此窗口
        // 极窄、生产自然触发概率低，故列为已知产品取舍待专项，不在本轮低危加固批消除。详见台账 B-01。
```

（加在兜底 guard 那段之上，不改任何代码逻辑。实现者亲验真实缩进 + guard 真实位置。）

- [ ] **Step 8: 台账状态更新**

在 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：
- B-01（:142 finding 的"状态: Open"）→ `状态: 已知产品取舍待专项（家族⑨ 标注不修 —— 协作式抢占固有尾窗，彻底消除需 gen 撤销补偿触碰 outbox 幂等核心风险最高；#待补）`
- B-03（:166 finding 的"状态"行）→ `状态: Fixed（家族⑨ #待补 —— second_safety_gate 发送前 fresh 复核 managed，非 managed 拦截 not_managed_at_send）`
- F-04（:290 finding 的"状态"行）→ `状态: Fixed（家族⑨ #待补 —— reclaim 累加 reclaim_count，>5 转 failed_terminal 止损）`

（实现者 Read 各 finding 的真实"状态:"行文字后精确替换，只改状态行。B-01/B-03/F-04 各一行。）

- [ ] **Step 9: 编译 + 纯函数测 + 全 lib 测**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib check_second_safety_gate 2>&1 | tail -15 && cargo test --lib 2>&1 | tail -5`
Expected: B-03 哨兵测（含既有 + 2 新）全 PASS；全 lib `test result: ok.` ≥ 350 passed / 0 failed。若 LNK1318（Windows-only）→ `cargo test --lib check_second_safety_gate` + `cargo check --lib` + 人工核对。本地只跑 --lib，绝不集成测。

- [ ] **Step 10: Commit + lint 预验（commit 后）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/models.rs src/agent/outbox_dispatcher.rs src/agent/outbox.rs src/agent/gateway.rs docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md && git commit -m "fix(outbox): reclaim超限转终态 + 发送前fresh复核managed + 抢占尾窗标注 (F-04/B-03/B-01 P3家族⑨)"
bash scripts/check-no-human-takeover.sh origin/main HEAD
bash scripts/check-no-model-hint.sh origin/main HEAD
```

两个 lint 都必须 `ok: 0 violations`。只 add 这五个文件，绝不 git add -A。若 lint 报违规，修正后补一个修正 commit。

---

## Self-Review 结论

- **Spec coverage**：F-04 reclaim_count 上限（Step 2 字段 + Step 3 reclaim）+ B-03 fresh managed 复核（Step 4 纯函数 + Step 5 外层 + Step 6 测）+ B-01 标注（Step 7 注释 + Step 8 台账）→ 覆盖设计三条 + 台账更新。设计非目标（B-01 不做 gen 撤销/F-04 不复用 max_attempts/不动 atomic_claim）通过"只改指定处"落实。
- **Placeholder scan**：无 TBD/TODO。台账 Step 8 的"#待补"是 PR 号占位（合并后为本 PR），状态字段本身完整。
- **Type consistency**：`check_second_safety_gate_pure` 8 参签名在定义（Step 4）、外层调用（Step 5）、既有测补参（Step 6）、新测（Step 6）一致；`is_managed: bool` 末参、`OutboxEntry.reclaim_count: i32`、`OUTBOX_MAX_RECLAIMS` 常量在 F-04 各步一致。
- **反过拟合**：B-03 新哨兵真驱动（回退删 is_managed 行 → not_managed 测红）；既有测补 `true` 保持旧断言。F-04 靠终审 + 集成测。
- **红线合规**：atomic_claim_pending / cancel_entry / dispatcher 主循环 / precheck 入参快照 / 多段 enqueue 全不动；`not_managed_at_send` 是 AI 内部状态名（非人工接管）；新增注释中性词无禁词无模型名；baseline 不回退；设计文档 file 位置错误在本计划已修正（check_second_safety_gate_pure 在 outbox.rs 非 dispatcher）；worktree 绝对路径。
