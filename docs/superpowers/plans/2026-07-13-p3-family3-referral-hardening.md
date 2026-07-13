# P3 家族③ 名片子系统加固 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** referral（辅助模式受控例外）两处红线敏感加固——KE-03 让名片二次准入对齐候选加载器的 account 维度（account 校验加进权威内存判据 `validate_card_sendable`），KE-05 给 toggle/delete 补 fail-soft 审计留痕。全部低风险、不改 referral 主流程语义。

**Architecture:** KE-03 把 account 归属校验加进 `validate_card_sendable`（enabled/approved 同层，三处发送路径全部经过它 → 单一事实源）；`filter_referral_candidates` 透传 account 参数。KE-05 沿用 review_referral_card 的 fail-soft 审计模板（`write_event_for_account` + 失败仅 warn），toggle/delete 各补一条审计事件，delete 保留硬删但先查后删。两条独立关注点，各一个 task。

**Tech Stack:** Rust 2021，纯函数单测（lib，本地可跑）。KE-03 全纯函数可 lib 测；KE-05 handler 审计逻辑直白（照搬 review 模板），靠终审亲验 + 既有集成测覆盖。无 Docker、无新依赖。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-13-p3-family3-referral-hardening-design.md`（已获批 commit 7fcb190）。所有行号亲验于分支 fix/p3-family3-referral-hardening（基于 origin/main f2410b3 含 #194）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆。
- **referral 红线敏感**：referral 是「辅助模式（账号级默认关）受控例外」——AI 主动引荐真人顾问名片，对话始终 AI 在说，台前顾问 ≠ 幕后决策源。本 PR 是纵深加固，**绝不改** referral 主流程语义（assist_mode_active 判定 / 候选注入 / 发送门 / 幂等）。
- 反过拟合红线：真 bug 才修；改既有测试断言仅限"被本修复有意废除的旧行为 / 签名变更被迫更新"，绝不为过测试改业务逻辑。
- **KE-03 account 校验加在内存层 `validate_card_sendable`，不改 find_one 的 DB filter**（account 与 enabled/approved 分层一致；候选加载 DB filter 已含 account `$or` 不动）。global scope 卡（`account_id=None`）语义 = 任何 account 可用，与 `build_referral_cards_filter` 的 `$or:[account_id null, ==account_id]` 口径完全一致。
- **KE-05 保留硬删**（不做软删——软删涉及全链查询加 deleted_at 过滤，超范围）；toggle/delete 审计 fail-soft（写失败仅 warn、绝不影响主操作）。
- check-no-human-takeover lint 扫 `src/agent/` `src/routes/` 新增行禁词（`人工接管/接管/人工/takeover/hand-off` 等）。审计文案 / 注释用中性词（引荐/停用/删除/审计），不得含禁词。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不回退。改动不触 baseline 门 4 PBT。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用，误写主仓会污染他人分支）。

---

## File Structure

- `src/agent/referral.rs`：**Modify** `validate_card_sendable`（:26-28）加 `account_id: &str` 参 + account 归属判据；`filter_referral_candidates`（:30-44）加 `account_id: &str` 参透传；`send_outbound_namecard`（:118）调用点传 `&contact.account_id`；更新既有单测（:257-261 `validate_excludes_draft_and_disabled` + :264+ `filter_matches_stage_or_empty` + `card` helper 保持 account_id=None）+ 新增 account 门专项单测。KE-03 主体在此。
- `src/agent/decision.rs`：**Modify** `filter_referral_candidates` 唯一生产调用点传入 account（候选加载处）。
- `src/agent/gateway.rs:2829`：**Modify** `validate_card_sendable(&c, &contact.account_id)`。
- `src/routes/referral_cards.rs`：**Modify** `toggle_referral_card`（:185-209）/ `delete_referral_card`（:212-231）补 fail-soft 审计（delete 先查后删）。KE-05 全在此。

两个 task 互不依赖：Task 1 = KE-03（referral.rs + decision.rs + gateway.rs），Task 2 = KE-05（referral_cards.rs）。

---

## Task 1: KE-03 —— account 准入判据加进 validate_card_sendable（referral.rs + decision.rs + gateway.rs）

**Files:**
- Modify: `src/agent/referral.rs:26-28`（`validate_card_sendable` 加 account_id 参 + 判据）
- Modify: `src/agent/referral.rs:30-44`（`filter_referral_candidates` 加 account_id 参透传）
- Modify: `src/agent/referral.rs:118`（`send_outbound_namecard` 调用点传 `&contact.account_id`）
- Modify: `src/agent/referral.rs:257-262`（既有 `validate_excludes_draft_and_disabled` 补 account 参）
- Modify: `src/agent/referral.rs:264+`（`filter_matches_stage_or_empty` 经 filter_referral_candidates 补 account 参）
- Modify: `src/agent/decision.rs`（`filter_referral_candidates` 唯一生产调用点传 account）
- Modify: `src/agent/gateway.rs:2829`（`validate_card_sendable(&c, &contact.account_id)`）

**Interfaces:**
- Consumes: `ReferralCard`（字段 `account_id: Option<String>`，已亲验 referral.rs:203 helper / models.rs）；`Contact.account_id: String`（gateway.rs:2840 / referral.rs:127/149 已用）。
- Produces: `pub(crate) fn validate_card_sendable(card: &ReferralCard, account_id: &str) -> bool`（新签名）；`pub(crate) fn filter_referral_candidates<'a>(cards: &'a [ReferralCard], customer_stage: Option<&str>, account_id: &str) -> Vec<&'a ReferralCard>`（新签名）。

- [ ] **Step 1: 先写 account 门专项单测（先写，验证会编译失败——签名未改）**

在 `src/agent/referral.rs` 的 `mod tests` 内、既有 `validate_excludes_draft_and_disabled`（:257-262）**之后**新增（`card` helper 建的卡 `account_id: None`，account 门测需另建带 account_id 的卡——用内联构造）：

```rust
    #[test]
    fn validate_card_account_scope() {
        // global 卡（account_id=None）→ 任何 account 可用（行为不变，与候选 DB filter $or:[null,...] 一致）。
        assert!(validate_card_sendable(&card(true, "approved", vec![]), "acct_A"));
        assert!(validate_card_sendable(&card(true, "approved", vec![]), "acct_B"));

        // 绑定 acct_A 的卡：只有 acct_A 可用，acct_B 拒（KE-03 核心：跨账号不可推）。
        let mut bound = card(true, "approved", vec![]);
        bound.account_id = Some("acct_A".to_string());
        assert!(validate_card_sendable(&bound, "acct_A"), "本账号卡须放行");
        assert!(
            !validate_card_sendable(&bound, "acct_B"),
            "绑定 acct_A 的卡不得经 acct_B 会话推出(KE-03 跨账号防护,回退即红)"
        );

        // account 门与 enabled/approved 门叠加：绑定卡即使 account 匹配,draft/disabled 仍拒。
        let mut bound_draft = card(false, "draft", vec![]);
        bound_draft.account_id = Some("acct_A".to_string());
        assert!(!validate_card_sendable(&bound_draft, "acct_A"));
    }
```

- [ ] **Step 2: 运行确认编译失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib validate_card_account_scope 2>&1 | tail -20`
Expected: 编译错误 E0061（`validate_card_sendable` 旧签名只收 1 参，新测传 2 参）。

- [ ] **Step 3: 改 validate_card_sendable 签名 + account 判据**

把 `src/agent/referral.rs:25-28`：

```rust
/// 发送前准入：仅 enabled 且 approved 的名片可被 AI 选/发。
pub(crate) fn validate_card_sendable(card: &ReferralCard) -> bool {
    card.enabled && card.review_status == "approved"
}
```

替换为：

```rust
/// 发送前准入：仅 enabled + approved + account 归属匹配的名片可被 AI 选/发。
///
/// KE-03：account 归属校验与 enabled/approved 同层（三条发送路径——候选加载
/// `filter_referral_candidates`、gateway 二次准入、`send_outbound_namecard`——
/// 全部经过本纯函数做二次校验，故 account 归属加在此处一处生效、口径单一）。
/// `account_id` = 本 contact 的账号。global scope 卡（`card.account_id=None`）
/// 任何账号可用；绑定某账号的卡仅该账号可用——与候选加载 DB filter
/// `build_referral_cards_filter` 的 `$or:[{account_id:null},{account_id:==account_id}]`
/// 口径完全一致，杜绝「同 workspace 内绑定账号 A 的名片经账号 B 会话推出」。
pub(crate) fn validate_card_sendable(card: &ReferralCard, account_id: &str) -> bool {
    let account_ok = match card.account_id.as_deref() {
        None => true,                    // global scope 卡：任何账号可用
        Some(bound) => bound == account_id, // 绑定卡：仅本账号
    };
    card.enabled && card.review_status == "approved" && account_ok
}
```

- [ ] **Step 4: 改 filter_referral_candidates 加 account 参透传**

把 `src/agent/referral.rs:30-44`：

```rust
pub(crate) fn filter_referral_candidates<'a>(
    cards: &'a [ReferralCard],
    customer_stage: Option<&str>,
) -> Vec<&'a ReferralCard> {
    cards
        .iter()
        .filter(|c| validate_card_sendable(c))
        .filter(|c| {
            c.target_stages.is_empty()
                || customer_stage
                    .map(|cs| c.target_stages.iter().any(|s| s == cs))
                    .unwrap_or(false)
        })
        .collect()
}
```

替换为（加 `account_id: &str` 参，透传给 `validate_card_sendable`）：

```rust
pub(crate) fn filter_referral_candidates<'a>(
    cards: &'a [ReferralCard],
    customer_stage: Option<&str>,
    account_id: &str,
) -> Vec<&'a ReferralCard> {
    cards
        .iter()
        .filter(|c| validate_card_sendable(c, account_id))
        .filter(|c| {
            c.target_stages.is_empty()
                || customer_stage
                    .map(|cs| c.target_stages.iter().any(|s| s == cs))
                    .unwrap_or(false)
        })
        .collect()
}
```

- [ ] **Step 5: 改 send_outbound_namecard 调用点（referral.rs:118）**

把 `src/agent/referral.rs:117-122`：

```rust
    // 发送前准入二次校验（防 AI 幻觉/已撤下名片一路漏到发送）。
    if !validate_card_sendable(&card) {
        return Err(AppError::External(
            "referral card not sendable (draft/disabled)".into(),
        ));
    }
```

替换为（传 `&contact.account_id`，命中失败返既有 AppError）：

```rust
    // 发送前准入二次校验（防 AI 幻觉/已撤下名片一路漏到发送 + KE-03 account 归属）。
    if !validate_card_sendable(&card, &contact.account_id) {
        return Err(AppError::External(
            "referral card not sendable (draft/disabled/account mismatch)".into(),
        ));
    }
```

- [ ] **Step 6: 改 gateway 二次准入调用点（gateway.rs:2829）**

把 `src/agent/gateway.rs:2829`：

```rust
                    Some(c) if super::referral::validate_card_sendable(&c) => {
```

替换为（传 `&contact.account_id`，命中失败走既有 `_ =>` 分支的 `referral_card_rejected` :2874-2886）：

```rust
                    Some(c) if super::referral::validate_card_sendable(&c, &contact.account_id) => {
```

- [ ] **Step 7: 改 decision.rs 里 filter_referral_candidates 唯一生产调用点**

先定位调用点：

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -rn "filter_referral_candidates(" src/agent/decision.rs`
Expected: 命中生产调用处（形如 `filter_referral_candidates(&cards, customer_stage)`）。

在该调用点补第三参——传本 contact 的 account_id（decision.rs 该处 contact / account 变量名以实际代码为准，实现者亲验后传入本 run 的 account）。**注意**：候选加载 DB filter `build_referral_cards_filter` 已只加载本 account + global 卡，内存再校验 account 对本 account 卡 / global 卡恒通过 → 候选侧行为不变，此改动是口径对齐（防御纵深），非行为变更。

- [ ] **Step 8: 改既有单测补 account 参**

`src/agent/referral.rs:257-262` 的 `validate_excludes_draft_and_disabled`：三处 `validate_card_sendable(&card(...))` 补第二参（`card` helper 建的卡 account_id=None=global，传任意 account 恒通过 account 门，enabled/approved 门语义不变）：

```rust
    #[test]
    fn validate_excludes_draft_and_disabled() {
        assert!(validate_card_sendable(&card(true, "approved", vec![]), "acct"));
        assert!(!validate_card_sendable(&card(false, "approved", vec![]), "acct"));
        assert!(!validate_card_sendable(&card(true, "draft", vec![]), "acct"));
    }
```

`filter_matches_stage_or_empty`（:264+）里对 `filter_referral_candidates(...)` 的调用补第三参 `"acct"`（helper 卡 account_id=None，传任意 account 候选行为不变）。实现者按实际调用形态补参。

- [ ] **Step 9: 运行 KE-03 相关单测通过**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib validate_card 2>&1 | tail -30`
Expected: 全部 PASS（含新增 account 门 + 既有 draft/disabled 补参 + filter 测）。

- [ ] **Step 10: 全 lib 测确认无回归**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 11: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/agent/referral.rs src/agent/decision.rs src/agent/gateway.rs && git commit -m "fix(referral): account 归属校验加进 validate_card_sendable,防跨账号推名片 (KE-03 P3家族③)"
```

---

## Task 2: KE-05 —— toggle/delete 补 fail-soft 审计（referral_cards.rs）

**Files:**
- Modify: `src/routes/referral_cards.rs:185-209`（`toggle_referral_card` 补 `referral_card.toggled` 审计）
- Modify: `src/routes/referral_cards.rs:212-231`（`delete_referral_card` 先查后删 + 补 `referral_card.deleted` 审计）

**Interfaces:**
- Consumes: `crate::agent::write_event_for_account(state: &AppState, account_id: &str, contact_wxid: Option<&str>, kind: &str, status: &str, summary: &str, details: Option<Document>) -> AppResult<()>`（已亲验 gateway.rs:5121-5129；referral_cards.rs:167 review 已在用）；`admin.username`、`card.account_id`、`card.display_name`。
- Produces: 无对外接口变化（纯加审计副作用）。

- [ ] **Step 1: toggle_referral_card 补审计**

把 `src/routes/referral_cards.rs:205-208`（update 后的 matched_count 检查 + 返回）：

```rust
    if result.matched_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    Ok(Json(json!({ "ok": true })))
```

替换为（成功后回查拿 card 信息、写 `referral_card.toggled` 审计，fail-soft 照 review 模板 :151-180）：

```rust
    if result.matched_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    // KE-05：审计停用/启用（改变 AI 可引荐范围,红线敏感须留痕）。回查拿 account_id/display_name。
    // fail-soft：审计写失败只 warn,绝不影响启停结果（同 review_referral_card 模板）。
    if let Ok(Some(card)) = state
        .db
        .referral_cards()
        .find_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await
    {
        let account_id = card.account_id.clone().unwrap_or_default();
        let status = if payload.enabled { "enabled" } else { "disabled" };
        let details = doc! {
            "card_id": oid.to_hex(),
            "toggled_by": admin.username.clone(),
        };
        if let Err(e) = crate::agent::write_event_for_account(
            &state,
            &account_id,
            None,
            "referral_card.toggled",
            status,
            &format!("管理员{}名片：{}", if payload.enabled { "启用" } else { "停用" }, card.display_name),
            Some(details),
        )
        .await
        {
            tracing::warn!("referral_card.toggled 审计写入失败（不影响启停）: {e}");
        }
    }
    Ok(Json(json!({ "ok": true })))
```

- [ ] **Step 2: delete_referral_card 先查后删 + 补审计**

把 `src/routes/referral_cards.rs:216-231`（delete_referral_card 的 parse + delete_one + count 检查 + 返回）：

```rust
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid card id".to_string()))?;
    let result = state
        .db
        .referral_cards()
        .delete_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    Ok(Json(json!({ "ok": true })))
```

替换为（**先查后删**——删前回查拿 card 信息，delete 后用查到的信息写 `referral_card.deleted` 审计，fail-soft）：

```rust
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("invalid card id".to_string()))?;
    // KE-05：删前回查（硬删后拿不到 card 信息）。用于删除后的审计留痕。
    let card_before = state
        .db
        .referral_cards()
        .find_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await
        .ok()
        .flatten();
    let result = state
        .db
        .referral_cards()
        .delete_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::BadRequest("card not found".to_string()));
    }
    // KE-05：审计硬删（改变 AI 可引荐范围,红线敏感,硬删不可恢复须留痕）。
    // fail-soft：审计写失败只 warn,绝不影响删除结果（同 review_referral_card 模板）。
    if let Some(card) = card_before {
        let account_id = card.account_id.clone().unwrap_or_default();
        let details = doc! {
            "card_id": oid.to_hex(),
            "deleted_by": admin.username.clone(),
            "target_wxid": card.target_wxid.clone(),
        };
        if let Err(e) = crate::agent::write_event_for_account(
            &state,
            &account_id,
            None,
            "referral_card.deleted",
            "deleted",
            &format!("管理员删除名片：{}", card.display_name),
            Some(details),
        )
        .await
        {
            tracing::warn!("referral_card.deleted 审计写入失败（不影响删除）: {e}");
        }
    }
    Ok(Json(json!({ "ok": true })))
```

- [ ] **Step 3: 编译确认（KE-05 handler 审计逻辑无 lib 单测，靠编译 + 终审 + 集成测）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --lib 2>&1 | tail -15`
Expected: `Finished`——`write_event_for_account`（crate::agent 已 pub）/ `doc!` / `admin.username` / `card.*` 字段均已在作用域（referral_cards.rs:167 review 已同款用）。若本地撞 LNK1318 PDB（已知 Windows-only 非代码错），`cargo check` 已足够验证编译正确。

- [ ] **Step 4: 全 lib 测确认无回归**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok.` ≥ 350 passed / 0 failed。

- [ ] **Step 5: Commit**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/routes/referral_cards.rs && git commit -m "fix(referral): toggle/delete 补 fail-soft 审计留痕,改变可引荐范围可追溯 (KE-05 P3家族③)"
```

---

## Self-Review 结论

- **Spec coverage**：KE-03（account 准入进 validate_card_sendable）→ Task 1；KE-05（toggle/delete 审计）→ Task 2。两条 finding 全覆盖。
- **Placeholder scan**：无 TBD/TODO，每步含完整可编译代码 + 精确命令 + 期望输出。Step 7（decision.rs 调用点）因变量名依实际代码，明确要求实现者亲验后补参——非 placeholder，是"亲验真实调用形态"的红线要求。
- **Type consistency**：`validate_card_sendable` 新签名 2 参在函数定义（T1 Step3）、3 处调用点（filter_referral_candidates Step4 / send_outbound_namecard Step5 / gateway Step6）、既有单测（Step8）全部更新；`filter_referral_candidates` 新签名 3 参在定义（Step4）、decision.rs 调用点（Step7）、单测（Step8）一致。`write_event_for_account` 签名亲验一致（Step2 Consumes）。
- **既有测试冲击**：`validate_card_sendable` / `filter_referral_candidates` 加参数 → 既有单测被迫补参（签名变更被迫更新，反过拟合合规，account_id=None 的 helper 卡语义不变）。KE-05 纯加审计副作用，无既有断言冲击。
- **红线合规**：referral 主流程（assist 判定 / 候选注入 / 发送门 / 幂等）不动；KE-03 account 判据与 enabled/approved 同层不改 DB filter；KE-05 保留硬删不做软删；审计文案中性无禁词。
