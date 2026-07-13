# P3 家族③ 名片子系统加固设计（KE-03 + KE-05）

> P3 桶C。深度审查台账 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md` KE-03（:1074-1082）+ KE-05（:1094-1101）。两条 Low，但均落在 **referral 辅助模式受控例外**红线路径（改变 AI 可引荐范围）。全部行号亲验于分支 `fix/p3-family3-referral-hardening`（基于 origin/main `f2410b3` 含 #194）。

## 背景与红线定位

referral（专属顾问名片引荐）是 CLAUDE.md 里「全自治模式默认关」的**辅助模式受控例外**：账号显式开启辅助模式后，AI 判定客户契合引荐条件时主动推真人顾问名片。名片库 CRUD 由管理员驱动（`AuthenticatedAdmin` + workspace scope），发送经 `outbox → send_outbound_namecard`。本家族两条 finding 都是这条红线路径上的**纵深加固**（非主流程 bug）：

- **KE-03**：候选加载是账号级过滤，但两处发送准入只按 workspace_id 不按 account_id → 同租户内绑定账号 A 的名片理论上可经账号 B 会话推出（触发需 LLM 幻觉合法他账号 card ObjectId，概率极低，故 Low）。
- **KE-05**：toggle 停用 / delete 硬删已审批名片直接改变 AI 可引荐范围，却无审计留痕（review 有），硬删误删不可恢复。

## 全面代码审查发现（决定最优方案，全部主控当场 Read 亲验）

审查 referral 全链后，KE-03 的最优落点与台账初判（「两处 find_one filter 加 account $or」）不同——**account 准入判据的权威归属点是内存纯函数 `validate_card_sendable`，不是 DB filter**：

- **权威准入判据是 `validate_card_sendable`**（referral.rs:26-28）：`card.enabled && card.review_status == "approved"`。三条发送路径全部经过它做二次校验：
  - 候选加载 `filter_referral_candidates`（referral.rs:36）逐卡调用；
  - gateway 二次准入（gateway.rs:2829）`Some(c) if validate_card_sendable(&c)`；
  - `send_outbound_namecard`（referral.rs:118）`if !validate_card_sendable(&card)`。
- **两处准入的 find_one filter 只带 `{_id, workspace_id}`**（gateway.rs:2820 / referral.rs:111）——enabled/approved **不在 DB filter 里**，是靠随后的 `validate_card_sendable` 在内存校验。account 归属与 enabled/approved 是**同一类「这张卡能不能对本 contact 用」的准入语义**。
- **候选加载 DB filter `build_referral_cards_filter`**（decision.rs:1406-1420）：`workspace_id + $or:[{account_id:null},{account_id:==account_id}] + enabled + approved`——account `$or` 语义 = 「global scope 卡（account_id 缺省）任何账号可用，否则须账号匹配」。

**结论**：把 account 校验加进 `validate_card_sendable`（enabled/approved 同层），一处修改三处生效、口径单一事实源，优于在 gateway/referral 两处 inline 改 filter（易漏、口径分散）。且 `validate_card_sendable` 是纯函数 → account 校验可直接 lib 单测（无需 Docker）。这与「准入判据独立于列表 filter」的方向一致——准入判据本就集中在这个纯函数里。

## 目标

referral 发送准入（三处同源）对齐 account 维度（KE-03）；名片库 toggle/delete 补审计留痕（KE-05）。两条独立、都在 referral 隔离路径。

## 架构：两条独立加固

### KE-03 —— account 准入判据加进 `validate_card_sendable`（内存层，单一事实源）

`validate_card_sendable` 加 `account_id: &str` 参数（本 contact 的 account）：

```rust
// referral.rs:26（改）
pub(crate) fn validate_card_sendable(card: &ReferralCard, account_id: &str) -> bool {
    card.enabled
        && card.review_status == "approved"
        && card_account_matches(card, account_id)
}

/// account 归属：global scope 卡（account_id 缺省）任何账号可用，否则须精确匹配。
/// 口径与候选加载 build_referral_cards_filter 的 $or:[{account_id:null},{==account_id}] 一致。
fn card_account_matches(card: &ReferralCard, account_id: &str) -> bool {
    card.account_id.is_none() || card.account_id.as_deref() == Some(account_id)
}
```

三处调用点传入本 contact 的 account：
- `filter_referral_candidates`（referral.rs:30-44）：加 `account_id: &str` 参数透传给 `validate_card_sendable`；其唯一调用点（decision.rs 候选加载处）传入 account。候选侧行为不变——DB filter 已只加载本 account + global 卡，内存再校验 account 恒通过（global 卡也通过）。
- gateway.rs:2829：`Some(c) if validate_card_sendable(&c, &contact.account_id)`。命中失败落既有 `referral_card_rejected`（:2879）。
- referral.rs:118：`if !validate_card_sendable(&card, &contact.account_id)`。命中失败返既有 `AppError::External("...not sendable...")`（:119-121）。

**安全性质**：这是纯**收窄**——新增一个 AND 条件（account 匹配），只会让「他账号名片」这一原本能过的边缘情况被拒，绝不放行任何原本被拒的卡。global scope 卡（account_id=None）行为完全不变。

**不改 find_one 的 filter**：filter 仍 `{_id, workspace_id}`（保持跨租户 IDOR 防御），account 校验统一在内存判据。理由：与 enabled/approved 现有分层一致（那两个也不在 filter、在 validate），避免 filter 与内存判据两套 account 口径。

### KE-05 —— toggle/delete 补 fail-soft 审计（照 review 模板，硬删保留）

沿用 `review_referral_card` 的审计模板（referral_cards.rs:167-179）：`write_event_for_account(&state, &account_id, None, kind, status, summary, Some(details))`，写失败仅 `tracing::warn!` 不影响主操作。

- **toggle_referral_card**（referral_cards.rs:185-209）：update 成功后回查 card（拿 account_id/display_name）→ 写 `referral_card.toggled`，status = `enabled ? "enabled" : "disabled"`，details 含 card_id / display_name / operated_by（admin.username）。
- **delete_referral_card**（referral_cards.rs:212-231）：**保留 `delete_one` 硬删**，但调整顺序为**先回查 card**（拿 display_name/account_id，删后就查不到了）→ delete_one → deleted_count>0 后写 `referral_card.deleted`（同 fail-soft），details 含 card_id / display_name / deleted_by。

硬删 + 审计（非软删）的理由：KE-05 finding 本质是「审计缺失」，补审计精准命中；软删涉及全链 5+ 处读查询加 `deleted_at==null` 过滤（候选加载 / gateway 准入 / send_outbound_namecard / list），是 referral 红线路径上的大改造，远超一条 Low finding 范围（YAGNI）。

## 回归风险

1. **KE-03 是纯收窄**（上证单调只减误放行）；global scope 卡行为不变；候选加载 DB filter 已含 account 故候选侧内存校验恒通过。
2. **KE-05 纯加法**：toggle/delete 主逻辑（update/delete_one + matched/deleted_count 检查）不动，只在成功后追加 fail-soft 审计；delete 仅调整「先查后删」顺序。
3. **既有测试冲击（反过拟合边界）**：`validate_card_sendable` 加参数 → 所有调用点 + 既有单测被迫更新签名（签名变更被迫更新，非为过测试改逻辑）。gateway/referral 集成测若断言准入行为不受影响（account 匹配时行为不变）。
4. **check-no-human-takeover lint**：referral.rs / gateway.rs / referral_cards.rs 均在 lint 扫描范围；新增审计文案 / 注释用中性词（「引荐 / 停用 / 删除 / 审计」），不得含禁词。
5. **baseline**：`cargo test --lib` ≥ 350 / 0，不触 4 PBT。

## 改动面

- **Modify** `src/agent/referral.rs`：`validate_card_sendable` 加 account_id 参 + 新增 `card_account_matches` 私有纯函数；`filter_referral_candidates` 加 account_id 参透传；`send_outbound_namecard`（:118）调用点传 `contact.account_id`；更新既有单测。
- **Modify** `src/agent/decision.rs`：`filter_referral_candidates` 唯一调用点传入 account。
- **Modify** `src/agent/gateway.rs:2829`：`validate_card_sendable(&c, &contact.account_id)`。
- **Modify** `src/routes/referral_cards.rs`：`toggle_referral_card` / `delete_referral_card` 补 fail-soft 审计（delete 先查后删）。

## 测试计划

- **KE-03 纯函数单测（lib，本地可跑）**：`validate_card_sendable`
  - global 卡（account_id=None）+ 任意 account → true（行为不变）；
  - account 卡匹配本 account → true；
  - account 卡不匹配（他账号）→ **false**（KE-03 核心收窄，回退到不校验 account 即变红，真回归哨兵）；
  - enabled=false / review_status≠approved → false（既有判据不变）。
- **KE-05 审计（handler 级，需 Docker 集成测，CI 跑）**：toggle/delete 后断言 agent_events 有 `referral_card.toggled` / `referral_card.deleted`。若集成测成本高，退一步靠终审亲验代码正确性（照搬 review fail-soft 模板）+ 现有 referral 集成测无回归。

## 非目标（YAGNI）

- 不改 find_one 的 DB filter（account 校验统一在内存判据 validate_card_sendable，与 enabled/approved 分层一致）。
- 不做软删（涉及全链查询改造，超 KE-05 范围）。
- 不动 KE-04（防重推软设计，有意 tradeoff，本 PR 不含）。
- 不动 referral 主流程（assist_mode_active / render_referral_lines / MCP send / build_referred_set_doc）。
