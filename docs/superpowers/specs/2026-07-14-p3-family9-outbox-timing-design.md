# P3 家族⑨ outbox/抢占时序设计（B-01 标注 + F-04 / B-03 实修）

> P3 桶B/C，**最后一族**。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` B-01（:142）/ B-03（:166）/ F-04（:290）。三条 Low，outbox/抢占并发时序。全部行号亲验于最新 origin/main（含 #202）。

## 背景与定位

三条并发/边缘时序 finding，一个 PR。经全链路亲验 + 用户裁决分两类：

- **F-04（真实边缘缺陷）→ 实修**：`reclaim_expired_leases` 不消耗任何 attempt，worker 同位置反复崩溃 → 无限 reclaim、永不进 failed_terminal。
- **B-03（真实逻辑缺陷）→ 实修**：managed→unmanaged 翻转在决策运行期（~10-15s）不复核 → 照发在途回复。
- **B-01（协作式抢占固有尾窗）→ 标注**：入队尾窗新入站导致双回复。台账明标"彻底消除需 gen 撤销补偿、触碰 outbox 幂等核心、产品取舍 Open 待裁决、风险最高" → 代码注释 + 台账标注"已知产品取舍待专项"，不改逻辑。

## 关键亲验事实（决定方案，全部主控当场 Read）

1. **F-04**（outbox_dispatcher.rs:98-128）：`reclaim_expired_leases` 对过期 in_flight（`locked_until < now`）`$set status=pending + reclaimed_in_flight=true + $unset worker_id/locked_until`，**不动 attempt**。`atomic_claim_pending`（:135）重新抢占后走全新实发，不累加 attempt、不受 max_attempts 约束。worker 每次同位置崩溃 → 无限 reclaim。
2. **B-03**（gateway.rs:3102 `precheck_send_gateway`）：:3108 `if contact.agent_status != AgentStatus::Managed` 用的是**入参 contact 快照**（runner 起点 reload 的），决策期翻转复核不到。dispatcher 侧 `second_safety_gate`（outbox_dispatcher.rs:179-227）**发送前 fresh 查 contact**（:184-195），把标量传给纯函数 `check_second_safety_gate_pure`（:218）判定——当前**不查 agent_status**。这是发送前最接近实际 MCP 发送的复核点。
3. **B-01**（gateway.rs）：入队前有两道 guard——主检查（:2306，apply_agent_updates 前）+ 兜底（:2513，outbox 入队前）。B-01 窗口 = :2513 guard 过后到多段 enqueue 循环（:2545-2567，每段一次 `outbox_enqueue` DB 往返，约 10-100ms）进行中新入站落入 → 本轮过时回复全 enqueue，同时 runner 检测 generation 变化重算再 enqueue 一批，两批 segment 幂等 key（`{src}#seg{idx}`，含各自 run_id）不同不去重 → 客户收两次回复。`EnqueueRequest` 已带 `run_id`（:2558）。
4. **纯函数可测点**：`check_second_safety_gate_pure`（outbox_dispatcher.rs:218 调用、定义在同文件）是纯判定函数，DB 查询在外层 `second_safety_gate` 做、纯函数收标量 → B-03 加 `is_managed` 参可确定性 lib 单测。

## 用户裁决（brainstorming）

1. **范围 = B-01 标注 + F-04/B-03 实修**（同 KD-02/桶A"有意识不做"模式）。
2. **F-04 阈值 = reclaim_count > 5 转 failed_terminal**，硬编码文件级常量（不新增 config——纯防御边界值，运维几乎不需调）。
3. **B-03 落点 = second_safety_gate（发送前二次安全门）加 agent_status==Managed 检查**——复用它已 fresh 查的 contact（零额外 DB 读）、dispatcher 侧翻转窗口最小、纯函数可测。

## 目标

- F-04：reclaim 累加独立 `reclaim_count`，> 5 转 failed_terminal，止住 worker 反复崩溃的无限 reclaim。
- B-03：发送前 fresh 复核 managed，决策期翻 normal 时拦截在途回复不发。
- B-01：代码注释 + 台账标注固有尾窗与补偿方案（gen 撤销），列为已知产品取舍待专项，不改逻辑。

## 架构

### F-04 —— reclaim_count 上限（outbox_dispatcher.rs:98-128）

reclaim 的 update 加 `$inc: { reclaim_count: 1 }`；因单个 update_many 无法基于 inc 后的值分流，**再单独一个 update_many** 把超限 pending 转 failed_terminal：

```rust
const OUTBOX_MAX_RECLAIMS: i32 = 5; // 文件级常量

// reclaim（现逻辑 + $inc）
let result = collection.update_many(
    doc! { "status": OutboxStatus::InFlight.as_str(), "locked_until": { "$lt": now } },
    doc! {
        "$set": { "status": OutboxStatus::Pending.as_str(), "reclaimed_in_flight": true, "updated_at": now },
        "$unset": { "worker_id": "", "locked_until": "" },
        "$inc": { "reclaim_count": 1 },
    },
    None,
).await?;
// F-04：reclaim_count 超上限的 entry 转 failed_terminal（worker 反复在同位置崩溃 → 无限
// reclaim 永不进终态时止损，交 admin）。reclaim ≠ 发送 attempt，故独立计数不复用 max_attempts。
let terminated = collection.update_many(
    doc! { "status": OutboxStatus::Pending.as_str(), "reclaim_count": { "$gt": OUTBOX_MAX_RECLAIMS } },
    doc! { "$set": {
        "status": OutboxStatus::FailedTerminal.as_str(),
        "updated_at": now,
        "last_error": "reclaim 超限（worker 反复崩溃，止损转终态）",
    } },
    None,
).await?;
if terminated.modified_count > 0 {
    tracing::warn!(terminated = terminated.modified_count, "outbox reclaim 超限转 failed_terminal");
}
```

`OutboxEntry`（models.rs）加 `reclaim_count: i32`（`#[serde(default)]`，兼容旧行 =0）——实现者亲验字段名/枚举 `OutboxStatus::FailedTerminal` 真实值。第二个 update_many 幂等（已 failed_terminal 不再匹配 `status:Pending`）。

### B-03 —— second_safety_gate 加 is_managed（outbox_dispatcher.rs:179-227）

`check_second_safety_gate_pure` 加 `is_managed: bool` 参；外层从 fresh contact 取：

```rust
// second_safety_gate 外层（已 fresh 查 contact，:184-195）：
let is_managed = contact.as_ref().map_or(false, |c| c.agent_status == AgentStatus::Managed);
// ... 传入纯函数（末参）：
Ok(check_second_safety_gate_pure(
    now.timestamp_millis(), entry.created_at.timestamp_millis(), cooldown_until_ms,
    last_inbound_ms, &outcome, decision_created_ms, STALE_THRESHOLD_MILLIS,
    is_managed,
))

// check_second_safety_gate_pure 内最前：
// B-03：发送前 fresh 复核 managed。决策运行期（~10-15s）admin 把 contact 改 normal 想立即
// 止住 AI，precheck 的入参快照复核不到；dispatcher 发送前 fresh 查 contact 是最接近实际
// 发送的复核点，非 managed（含 contact 被删 → is_managed=false）→ 拦截，不发在途回复。
if !is_managed {
    return Some("not_managed_at_send".to_string());
}
```

`contact=None`（被删）→ is_managed=false → 拦截（不该发）。`AgentStatus` 若未 import 则加 use。返回的 `Some(reason)` 复用现有拦截路径（process_entry 对 cooldown/stop 的 Some 已 cancel entry，not_managed_at_send 同路径）——实现者亲验现有调用点对 Some 的处理。

### B-01 —— 标注（gateway.rs:2513 附近，不改逻辑）

在兜底 guard（:2513）附近加注释，说明固有尾窗 + 补偿方案（gen 撤销，run_id 已现成）+ 为何暂不做（产品取舍 + 触碰 outbox 幂等核心 + 风险最高 + 窗口极窄）。台账 B-01 状态 Open → "已知产品取舍待专项（家族⑨ 标注不修）"。

## 改动面

- **Modify** `src/agent/outbox_dispatcher.rs`：F-04 reclaim（:98-128 加 $inc + 第二个 update_many + 常量）；B-03 `check_second_safety_gate_pure` 加参 + `second_safety_gate` 外层取 is_managed；mod tests 补 B-03 纯函数单测。
- **Modify** `src/agent/gateway.rs`：B-01 注释（:2513 附近）。
- **Modify** `src/models.rs`：`OutboxEntry` 加 `reclaim_count: i32`（`#[serde(default)]`）——若字段不存在。
- **Modify** `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：B-01 状态标注 + F-04/B-03 状态→Fixed。

## 测试计划

- **B-03（纯函数 lib 单测，本地可跑）**：`check_second_safety_gate_pure` 加 is_managed 后——`is_managed=false` → 断言 `Some("not_managed_at_send")`（回退删该行即变红，真哨兵）；`is_managed=true` + 其它条件正常 → None（不误伤）。既有该纯函数的 lib 测补 is_managed 末参（传 true 保持旧断言值不变，反过拟合纯签名适配）。
- **F-04**：reclaim + 超限转 failed 是 DB update_many 语义，无独立纯函数。靠终审亲验 + `tests/outbox_integration.rs`（已驱动 reclaim_expired_leases）扩展一条"reclaim 6 次 → failed_terminal"断言（#[ignore]，CI 跑）；若无低成本扩展点则终审亲验（改动直白：加 $inc + 一个 update_many）。
- **B-01**：纯注释，无测试。
- **baseline**：`cargo test --lib` ≥ 350 / 0 不回退（B-03 纯函数测 +）；4 PBT 不触。

## 回归风险

1. **F-04**：新增 `reclaim_count`（serde default=0 兼容旧行）；正常 entry 几乎不被 reclaim，>5 极难自然达到；第二个 update_many 幂等（已 failed_terminal 不匹配 status:Pending）。不改 atomic_claim_pending / 正常发送路径。
2. **B-03**：纯函数加参，既有 lib 测补 `is_managed=true` 保持旧断言；正常 managed 发送行为不变，仅决策期翻 normal 时拦截（修复目标）。复用已有 fresh contact 查询，零额外 DB 读。
3. **B-01**：纯注释，零逻辑变更。
4. **check-no-human-takeover（扫 src/agent）+ check-no-model-hint（扫 src/）lint**：新增注释/reason 用中性词（发送/复核/账号/运营/终态/抢占/尾窗），无禁词（人工/接管/takeover/hand-off）、无模型品牌名。`not_managed_at_send` 是 AI 内部状态名（非"人工接管"），合规。

## 非目标（YAGNI）

- **B-01 不做 gen 撤销补偿**（产品取舍待专项；触碰 outbox 幂等核心 + runner 重算交互 = 并发正确性改动，风险最高，且难在低危批充分验证）。
- F-04 不复用 max_attempts（reclaim ≠ 发送 attempt，语义不同，独立计数）；不新增 config（纯防御边界值）。
- B-03 不动 precheck_send_gateway 的入参快照判定（保留，作为第一道；second_safety_gate 是发送前最后 fresh 复核）。
- 不动 atomic_claim_pending / cancel_entry / dispatcher 主循环 / 多段 enqueue 逻辑。
