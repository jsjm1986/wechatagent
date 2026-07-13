# P3 家族⑦ escalation 超时改派守卫设计（KD-07）

> P3 桶C。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KD-07（:915-923）。单条 Low-Medium，决策人链改派守卫不一致。全部行号亲验于最新 origin/main（含 #200）。

## 背景与定位

决策人请示通道（幕后领导模式）有两条"推卡"入口：
- **首推** `escalate_held_decision`（`src/agent/escalation/mod.rs:43`）：高风险闸件首次向链首决策人推请示卡。
- **超时改派** `scan_escalation_timeouts`（`mod.rs:358`）：链首超时未决时，向链中下一位决策人改派并推卡。

两条入口对"决策人 == 客户"的守卫**不一致**：
- 首推（`mod.rs:75-79`）取 `principal_wxid`（`decider_chain.first()`）后、推卡前有 `if principal_wxid == contact.wxid → return Err`。
- 超时改派（`mod.rs:430` 取 `next_wxid` = `next_decider_on_timeout` 返回值，`:454` 推卡）**无任何 `next_wxid == contact_wxid` 校验**。

## 问题（KD-07，CONFIRMED）

`next_decider_on_timeout`（`policy.rs:106-122`）一次只返回链中 current 的下一位（或孤儿回落链首），不感知"谁是本请示的客户"。若 admin 误把某客户 wxid 配进 `decider_chain[1..]`（非链首，故首推守卫不拦），链首超时后改派会挑中这个被误配成客户的成员，把含 `entry.reason` / `entry.question_for_principal` / 客户标签的内部请示卡（`render_principal_card`，`mod.rs:444`）直接推给该客户 → **泄漏内部请示内容**。

- 严重度：Low-Medium（守卫缺失亲验坐实；触发需 admin 误配，故计 Low）。
- 根因层：两条推卡入口对"决策人 == 客户"守卫不一致；改派点在挑下一位决策人时不排除客户 wxid。

## 用户裁决（brainstorming）

1. **修复方向 = ③ 改 `next_decider_on_timeout` 跳过非法成员**（根上正确、天然复用既有链尾安抚兜底，是台账首选①"推卡点守卫"的完整正确版——①单独 `continue` 会因 next 是固定下一位导致每 tick 重算同一非法 next→永久卡住不改派、客户被晾死）。
2. **只修改派点**：首推守卫（`mod.rs:75-79` 链首==客户 `return Err`）保持不动。首推是"要不要发起请示"、改派是"发起后换谁"，语义不同；链首==客户意味着请示配置根本错乱，拒绝整个请示比跳过更合理。
3. **不加写入口校验**（方向②）：`put_ask_human_policy`（`domains.rs:206`）不加"decider 不得等于在管客户"校验。运行时方向③已彻底堵死泄漏（无论存量/新配、无论先配后加入客户）；写入口校验防御不完整（拦不住"先配 decider 后客户变 managed"的时序窗）且需查 workspace 全量 managed contacts = YAGNI。

## 关键亲验事实（决定方案，全部主控当场 Read）

1. **改派点无守卫**（`mod.rs:430`→`:454`）：`next_wxid = next.wxid.clone()`，经骚扰门（`:438` push_allowed）后直接 `render_principal_card` + `logged_call_for_account(recipient=next_wxid)`，全程无 `next_wxid == entry.contact_wxid` 判断。
2. **`next_decider_on_timeout` 现逻辑**（`policy.rs:106-122`）：`timeout_hours?` → `age < timeout` 返 None → `position(current)` 命中返 `get(idx+1)`（真链尾越界=None）、未命中（孤儿）返 `first()`（KD-06 回落）。一次只返回单个成员，不遍历跳过。
3. **`entry.contact_wxid` 现成可用**：`AgentPrincipalEscalation` 结构体含 `contact_wxid`，链尾安抚分支（`mod.rs:401/411`）已在用，改派点无需新取。
4. **首推守卫**（`mod.rs:75-79`）：`principal_wxid == contact.wxid → return Err("决策人配置等于客户 wxid，拒绝触发请示")`，只挡链首。
5. **既有 5 个 lib 单测**（`policy.rs:312-380`）全是 3 参调用 `next_decider_on_timeout(&p, current, age)`：`picks_following` / `none_when_timeout_unset` / `orphan_falls_back_to_head`（KD-06）/ `real_chain_tail_still_none`（KD-06 不误伤）/ `orphan_empty_chain_is_none`。

## 目标

超时改派挑下一位决策人时，跳过链中被误配成本请示客户 wxid 的成员，改派给第一个合法的下一位决策人；若无合法下一位，返 None → 走既有链尾安抚（客户收安抚话术、不收内部请示卡）。

## 架构：纯函数根上修复（单一改动点）

### 核心：`next_decider_on_timeout` 加 `contact_wxid` 参 + 跳过逻辑

签名 3 参 → 4 参（新增 `contact_wxid: &str`）：

```rust
pub(crate) fn next_decider_on_timeout<'a>(
    policy: &'a ResolvedAskHumanPolicy,
    current_wxid: &str,
    contact_wxid: &str, // KD-07：本请示的客户 wxid，跳过链中误配成该客户的成员
    age_hours: f64,
) -> Option<&'a DeciderRef> {
    let timeout = policy.timeout_hours?;
    if age_hours < timeout {
        return None;
    }
    // 起点：current 在链中→下一位；孤儿（不在链）→链首（KD-06 回落，保留）。
    let start = match policy.decider_chain.iter().position(|d| d.wxid == current_wxid) {
        Some(idx) => idx + 1,
        None => 0,
    };
    // KD-07：从起点起跳过误配成客户 wxid 的成员，返回第一个合法下一位（防内部请示卡直推客户）。
    // 无合法下一位（真链尾 / 剩余全是客户 / 空链）→ None → scan 走既有链尾安抚。
    policy.decider_chain[start..]
        .iter()
        .find(|d| d.wxid != contact_wxid)
}
```

性质：在 KD-06 既有语义（孤儿回落 None→0、真链尾 idx+1 越界→空切片→None）之上，仅叠加"跳过 == contact_wxid 的成员"。链中无客户成员时，`find` 命中 `start` 位，等价原 `get(start)`，正常场景零行为变化。

### 调用点：`scan_escalation_timeouts`（mod.rs:378）补传 contact_wxid

```rust
let Some(next) = next_decider_on_timeout(&policy, &entry.principal_wxid, &entry.contact_wxid, age_hours)
else {
    // 既有链尾安抚分支（mod.rs:380-428）一字不动。
    ...
};
```

推卡点（`mod.rs:454`）**不再加二次守卫**——`next` 现在保证 `!= contact_wxid`（纯函数已滤），推卡目标天然安全。这是方向③相对①的关键优势：守卫在根上，不在推卡点打补丁、不引入"每 tick 重算同一非法 next 卡住"。

### 首推守卫不动

`escalate_held_decision`（`mod.rs:75-79`）的 `principal_wxid == contact.wxid → return Err` 保持原样（用户裁决只修改派点）。

## 语义对照

| 场景 | 修复前 | 修复后 |
|---|---|---|
| decider_chain[1..] 含客户，链首超时 | 改派把内部卡直推客户（泄漏） | 跳过客户成员，改派给下一个合法决策人 |
| 链里除客户外还有合法下一位 | — | 推给那个合法的 |
| 跳过客户后无合法成员 | — | None → 链尾安抚（客户收安抚话术，不收内部卡） |
| 链首==客户（首推阶段） | return Err 拒绝请示 | 不变（return Err） |
| 链中无客户成员（正常配置） | get(idx+1) | find 命中 start 位（等价，零变化） |

## 改动面

- **Modify** `src/agent/escalation/policy.rs`：`next_decider_on_timeout` 签名 +1 参、body 加跳过逻辑；`mod tests` 既有 5 测补第 3 参（传不在链中的 contact wxid，旧断言值不变）+ append 2 新测。
- **Modify** `src/agent/escalation/mod.rs:378`：唯一生产调用点补传 `&entry.contact_wxid`。

## 测试计划

**新增确定性 lib 单测（真哨兵，本地可跑）**：
- `next_decider_skips_member_equal_to_contact`：链 = [leader_a, customer_x(误配), leader_c]，current=leader_a 超时 → 断言返 `Some("leader_c")`（跳过 customer_x）。回退（去掉 `.find(!=contact)` 改回 `get(start)`）→ 返 customer_x → 红。
- `next_decider_none_when_only_remaining_is_contact`：链 = [leader_a, customer_x]，current=leader_a 超时 → 断言 None（跳过客户后无合法下一位 → 走链尾安抚）。回退 → 返 customer_x → 红。

**既有 5 测补参（反过拟合红线）**：全部第 3 参补一个不在链中的 contact wxid（`"customer_x"`），旧断言值一字不变——链中无此 wxid → 跳过逻辑不触发 → 行为与原完全相同。纯签名补参保持旧维度存活，非改测试意图。

**不需集成测**：纯函数逻辑，lib 单测足够；改派 DB 时序（reassign / 骚扰门 / 链尾安抚）不受本改动影响。

## 回归风险

1. **纯函数逻辑叠加**：既有 5 测锁死 KD-06 孤儿回落 / 真链尾 / 空链 / 未超时行为不变；新参对"链中无客户"正常场景零影响（find 第一个即 start 位）。
2. **调用点单点补参**：`entry.contact_wxid` 现成，不新取、不改 DB 交互。
3. **baseline**：`cargo test --lib` ≥ 350 / 0 不回退（+2 新测）；4 PBT 不触。
4. **check-no-human-takeover lint**：`src/agent/escalation/` 在扫描范围——新增行/注释用中性词（决策人/客户/改派/链/请示），无禁词（人工/接管/takeover/hand-off）。
5. **check-no-model-hint lint**：新增行无模型/品牌名（无 gpt/claude/anthropic/deepseek 等字面量）。

## 非目标（YAGNI）

- 不加写入口 `put_ask_human_policy` 校验（用户裁决；运行时方向③已彻底堵死，写入口防御不完整 + 查全量客户成本）。
- 不动首推守卫 `escalate_held_decision:75-79`（用户裁决只修改派点）。
- 不动 `reassign_escalation` / 骚扰门 push_allowed / 链尾安抚 / `render_principal_card`。
- 不做"跳过后记审计事件"（跳过误配成员是静默的正确行为，无需额外事件；误配本身应由 admin 在 UI 侧发现）。
