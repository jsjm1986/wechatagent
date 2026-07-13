# P3 家族⑦ escalation 超时改派守卫 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** KD-07——超时改派 `next_decider_on_timeout` 挑下一位决策人时跳过链中被误配成本请示客户 wxid 的成员（防内部请示卡直推客户泄漏），无合法下一位则 None→走既有链尾安抚。

**Architecture:** 单一纯函数根上修复。`next_decider_on_timeout`（`src/agent/escalation/policy.rs`）签名 3 参→4 参（新增 `contact_wxid: &str`），body 把 `get(idx+1)`/`first()` 的单点取值改为从起点起 `.find(|d| d.wxid != contact_wxid)` 跳过客户成员。唯一生产调用点 `scan_escalation_timeouts`（`mod.rs:378`）补传 `&entry.contact_wxid`。5 个既有 lib 单测补第 3 参（传不在链中的 contact wxid，旧断言值不变），append 2 个新哨兵测。首推守卫不动、不加写入口校验（用户裁决）。

**Tech Stack:** Rust 2021。改动是纯函数逻辑 + 单调用点补参 → lib 单测（本地可跑）覆盖，无需集成测。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-14-p3-family7-escalation-reassign-guard-design.md`（已获批 commit 85af647）。所有行号亲验于分支 `fix/p3-family7-escalation-reassign-guard`（基于含 #200 的最新 origin/main）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆（行号可能漂移，以 Read 到的真实代码为准）。
- **只修改派点**：首推守卫 `escalate_held_decision`（`mod.rs:75-79` 的 `principal_wxid == contact.wxid → return Err`）**保持不动**。
- **不加写入口校验**：`put_ask_human_policy`（`domains.rs:206`）不碰。
- **保留 KD-06 语义**：孤儿回落链首（current 不在链→起点 0）、真链尾返 None（起点越界→空切片→None）行为一字不变；新逻辑仅在其上叠加"跳过 == contact_wxid 成员"。既有 5 测锁死这些语义，补参后旧断言值不得变。
- 反过拟合红线：真 bug 才修；2 新哨兵测驱动真跳过逻辑（回退去掉 `.find(!=contact)` 即变红）。5 既有测纯签名补参（传不在链中的 wxid，行为等价），绝不改旧断言意图。
- check-no-human-takeover lint 扫 `src/agent/`——新增行/注释禁词（人工接管/接管/人工/takeover/hand-off 等）；用中性词（决策人/客户/改派/链/请示）。
- check-no-model-hint lint 扫 `src/`——新增行禁模型/品牌名（gpt/claude/anthropic/deepseek/gemini/qwen/kimi 等）。本任务测试用 wxid 字面量（leader_a/customer_x/leader_c）无品牌词，安全。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不触 4 PBT。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用）。
- 本地若撞 LNK1318 PDB（Windows-only 非代码错），`cargo test --lib next_decider` 已足够验证纯函数；全量 `cargo test --lib` 撞 LNK1318 时 `cargo check --lib` + 人工核对。

## File Structure

- `src/agent/escalation/policy.rs`：`next_decider_on_timeout`（:106-122）签名+1 参、body 改 find 跳过；`mod tests`（:302-380 区）5 既有测补参 + append 2 新测。
- `src/agent/escalation/mod.rs`：`scan_escalation_timeouts`（:378）唯一生产调用点补传 `&entry.contact_wxid`。

单 Task（两文件一次改完、一个测试周期）：改函数签名必须同时改所有调用点（生产 + 测试）否则不编译，无法拆分独立可测的子任务。

---

## Task 1: next_decider_on_timeout 加 contact_wxid 跳过误配客户成员

**Files:**
- Modify: `src/agent/escalation/policy.rs:106-122`（函数签名 + body）
- Modify: `src/agent/escalation/policy.rs:312-380`（5 既有测补参 + append 2 新测）
- Modify: `src/agent/escalation/mod.rs:378`（唯一生产调用点补参）

**Interfaces:**
- Consumes: `ResolvedAskHumanPolicy`（policy.rs，含 `decider_chain: Vec<DeciderRef>`、`timeout_hours: Option<f64>`）；`DeciderRef { wxid: String, display_name: Option<String> }`；`AgentPrincipalEscalation`（含 `contact_wxid: String`、`principal_wxid: String`，调用点 `entry` 现成）。
- Produces: `fn next_decider_on_timeout<'a>(policy: &'a ResolvedAskHumanPolicy, current_wxid: &str, contact_wxid: &str, age_hours: f64) -> Option<&'a DeciderRef>`（签名 +1 参）。

- [ ] **Step 1: 亲验两处真实现状**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "fn next_decider_on_timeout\|next_decider_on_timeout(" src/agent/escalation/policy.rs src/agent/escalation/mod.rs`
Expected: 确认函数定义（policy.rs 约 :106）、5 处既有测调用（policy.rs :321/323/325/336/351/367/379——注意有 7 个调用点，其中 :321 有 2 个断言在一个测试里，实际 5 个 #[test] 函数）、1 处生产调用（mod.rs 约 :378）。**实现者 Read policy.rs:104-122 函数全貌 + :312-380 测试全貌 + mod.rs:374-430 调用点及其后链尾安抚分支全貌**后再改（行号以真实为准）。

- [ ] **Step 2: 改 next_decider_on_timeout 签名 + body（跳过客户成员）**

把 `src/agent/escalation/policy.rs` 的函数（现 :106-122）：

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

改为（doc 注释同步补 KD-07；签名加 `contact_wxid`；body 用 find 跳过）：

```rust
pub(crate) fn next_decider_on_timeout<'a>(
    policy: &'a ResolvedAskHumanPolicy,
    current_wxid: &str,
    contact_wxid: &str,
    age_hours: f64,
) -> Option<&'a DeciderRef> {
    let timeout = policy.timeout_hours?;
    if age_hours < timeout {
        return None;
    }
    // 起点：current 在链中→下一位（idx+1）；current 不在链（KD-06：admin 改 decider_chain 删/换人
    // 后的孤儿 pending）→ 回落链首（起点 0）让孤儿重新入链。
    let start = match policy.decider_chain.iter().position(|d| d.wxid == current_wxid) {
        Some(idx) => idx + 1,
        None => 0,
    };
    // KD-07：从起点起跳过被误配成本请示客户 wxid 的成员，返回第一个合法下一位决策人，防止把
    // 含内部请示内容的卡直推给客户。无合法下一位（真链尾越界=空切片 / 剩余全是客户 / 空链）→
    // None → scan 走既有链尾安抚（客户收安抚话术、不收内部请示卡）。真链尾 idx+1 越界时
    // decider_chain[start..] 为空切片，find 返 None，KD-06 行为保持。
    policy.decider_chain[start..]
        .iter()
        .find(|d| d.wxid != contact_wxid)
}
```

（`decider_chain[start..]` 当 `start == len` 时是空切片安全，不 panic；`start > len` 不可能因 idx+1 最多 = len。实现者按亲验的真实代码块精确替换 body，doc 注释若在 :104-105 也同步补一句 KD-07 跳过客户成员。）

- [ ] **Step 3: 改唯一生产调用点 mod.rs:378 补传 &entry.contact_wxid**

把 `src/agent/escalation/mod.rs` 的调用（现约 :378）：

```rust
            let Some(next) = next_decider_on_timeout(&policy, &entry.principal_wxid, age_hours)
```

改为：

```rust
            let Some(next) = next_decider_on_timeout(&policy, &entry.principal_wxid, &entry.contact_wxid, age_hours)
```

（`else { ... }` 链尾安抚分支及其后所有代码一字不动。`entry.contact_wxid` 已在该分支 :401/:411 使用、现成可用。实现者亲验 `entry` 是 `AgentPrincipalEscalation` 且有 `contact_wxid` 字段。）

- [ ] **Step 4: 5 个既有测补第 3 参（传不在链中的 contact wxid，旧断言值不变）**

在 `src/agent/escalation/policy.rs` 的 `mod tests`，给 5 处既有 `next_decider_on_timeout` 调用补第 3 参 `"customer_x"`（一个绝不在任何测试链中的 wxid，故跳过逻辑不触发、行为与原完全相同）。逐处改：

`next_decider_picks_following_after_timeout`（现 :321/323/325）：
```rust
        // 当前 a，已等 25h > 24h → 转 b
        assert_eq!(next_decider_on_timeout(&p, "a", "customer_x", 25.0).map(|d| d.wxid.as_str()), Some("b"));
        // 未超时 → None
        assert_eq!(next_decider_on_timeout(&p, "a", "customer_x", 10.0), None);
        // 已是链尾 b → None（继续等）
        assert_eq!(next_decider_on_timeout(&p, "b", "customer_x", 99.0), None);
```

`next_decider_none_when_timeout_unset`（现 :336）：
```rust
        assert_eq!(next_decider_on_timeout(&p, "a", "customer_x", 9999.0), None);
```

`next_decider_orphan_current_falls_back_to_chain_head`（现 :351）：
```rust
        assert_eq!(
            next_decider_on_timeout(&p, "ghost", "customer_x", 99.0).map(|d| d.wxid.as_str()),
            Some("a"),
            "改链孤儿（current 不在链）超时后须回落链首重新入链，而非静默退化链尾"
        );
```

`next_decider_real_chain_tail_still_none`（现 :367）：
```rust
        assert_eq!(
            next_decider_on_timeout(&p, "b", "customer_x", 99.0),
            None,
            "真链尾必须仍返 None（合法继续等），不得被孤儿回落逻辑误伤"
        );
```

`next_decider_orphan_empty_chain_is_none`（现 :379）：
```rust
        assert_eq!(next_decider_on_timeout(&p, "ghost", "customer_x", 99.0), None);
```

（这 5 处链中都不含 `"customer_x"`，第 3 参补入后跳过逻辑不触发、断言值全部不变——纯签名适配，非改测试意图。实现者按真实测试代码逐处补参。）

- [ ] **Step 5: append 2 个新哨兵测**

在 `src/agent/escalation/policy.rs` 的 `mod tests`（`next_decider_orphan_empty_chain_is_none` 之后）append：

```rust
    #[test]
    fn next_decider_skips_member_equal_to_contact() {
        // KD-07：decider_chain[1..] 误配成客户 wxid，链首超时改派须跳过该成员、改派给下一个合法
        // 决策人，而非把内部请示卡推给客户。
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![
            DeciderRef { wxid: "leader_a".into(), display_name: None },
            DeciderRef { wxid: "customer_x".into(), display_name: None }, // 误配=客户
            DeciderRef { wxid: "leader_c".into(), display_name: None },
        ];
        // 当前 leader_a 超时，下一位是被误配的 customer_x → 必须跳过，改派给 leader_c。
        assert_eq!(
            next_decider_on_timeout(&p, "leader_a", "customer_x", 99.0).map(|d| d.wxid.as_str()),
            Some("leader_c"),
            "改派须跳过误配成客户 wxid 的链成员，防内部请示卡直推客户"
        );
    }

    #[test]
    fn next_decider_none_when_only_remaining_is_contact() {
        // KD-07：跳过客户成员后链中无其它合法下一位 → None → scan 走链尾安抚（客户不收内部卡）。
        let mut p = resolved_with(None, None);
        p.timeout_hours = Some(24.0);
        p.decider_chain = vec![
            DeciderRef { wxid: "leader_a".into(), display_name: None },
            DeciderRef { wxid: "customer_x".into(), display_name: None },
        ];
        assert_eq!(
            next_decider_on_timeout(&p, "leader_a", "customer_x", 99.0),
            None,
            "跳过客户成员后无合法下一位须返 None（走链尾安抚），不得回退到推客户"
        );
    }
```

（`resolved_with`、`DeciderRef` 已在 mod tests 作用域内被既有测使用，实现者亲验其可用性。）

- [ ] **Step 6: 编译 + 跑目标测试 + 全 lib 测**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib next_decider 2>&1 | tail -18 && cargo test --lib 2>&1 | tail -5`
Expected: 7 个 next_decider 测（5 既有 + 2 新）全 PASS；全 lib `test result: ok.` ≥ 350 passed / 0 failed。若 LNK1318（Windows-only）→ `cargo test --lib next_decider`（纯函数测能跑）+ `cargo check --lib` + 人工核对。

- [ ] **Step 7: 本地预验两个 lint（须先 commit 才能对 HEAD 扫）**

先 commit（Step 8），再回跑：
Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && bash scripts/check-no-human-takeover.sh origin/main HEAD && bash scripts/check-no-model-hint.sh origin/main HEAD`
Expected: 两个都 `ok: 0 violations`。（新增行只有中性词 + wxid 字面量 leader_a/customer_x/leader_c，无禁词无品牌名。）

- [ ] **Step 8: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/agent/escalation/policy.rs src/agent/escalation/mod.rs && git commit -m "fix(escalation): 超时改派跳过误配成客户wxid的链成员,防内部请示卡直推客户 (KD-07 P3家族⑦)"
```

（Step 7 的 lint 预验在 commit 后跑；若发现违规则修正后 `git commit --amend` 或补一个修正 commit。）

---

## Self-Review 结论

- **Spec coverage**：设计的唯一目标（改派跳过客户成员）→ Task 1 Step 2（函数）+ Step 3（调用点）；语义对照表 5 行全被测试覆盖（Step 4 的既有 5 测锁 KD-06/正常场景不变，Step 5 的 2 新测锁 KD-07 跳过 + 链尾安抚回退）。首推守卫不动/不加写入口校验（设计非目标）在 Global Constraints + 计划里通过"只改指定两处"落实。
- **Placeholder scan**：无 TBD/TODO。所有代码块是完整可粘贴的真实替换。
- **Type consistency**：`next_decider_on_timeout` 4 参签名在定义（Step 2）、生产调用（Step 3）、5 既有测（Step 4）、2 新测（Step 5）完全一致（第 3 参 `contact_wxid: &str`，传 `&entry.contact_wxid` / `"customer_x"`）。返回类型 `Option<&DeciderRef>` 不变。
- **反过拟合**：2 新哨兵真驱动跳过逻辑（回退去 `.find(!=contact)` → skip 测返 customer_x → 红）；5 既有测纯补参、断言值不变（链中无 customer_x → 跳过不触发）。
- **红线合规**：首推守卫/reassign/骚扰门/链尾安抚/render_principal_card/写入口全不动；新增行中性词无禁词、无模型名；baseline 不回退；worktree 绝对路径。
