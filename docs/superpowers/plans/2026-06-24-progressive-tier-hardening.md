# 渐进式三档机制加固 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给已上线的渐进式三档机制加固 5 类敞口（强升高危态/灰区观测/运维开关/run log tier 字段/Clarify prompt 收紧），收口 7-agent 交叉审查结论。

**Architecture:** 强升闸堵确定高危（coverage=missing 且需知识、missing_tier 非法 → 当场升 Full），观测盯不硬堵的灰区（weak 乐观、关系档漏判、自评 JSON 失效 → 只记 telemetry），运维开关 `PROGRESSIVE_TIER_ENABLED` 给一键退回单程止损。三者职责正交。

**Tech Stack:** Rust 2021 / Axum / MongoDB（bson Document）/ cargo test。

## Global Constraints（逐条来自 spec，每个 task 都隐含适用）

- **agent-first**：判据只用 sufficiency / knowledge_coverage / knowledge_need 的客观字符串匹配 + 结构判断，**绝不引入关键词词表 / 文本启发式**。
- **正向精确匹配**：所有 coverage/sufficiency 判据用 `==` 正向匹配，**绝不用 `!=` 否定**（防 not_required 寒暄轮 / `_=>` 兜底误命中）。
- **并行会话隔离**：分支 `feat/tag-trust` 上有其他会话并行改 tag-trust 子计划。只 `git add` 自己改的文件，**绝不 `git add -A/.`**。共享文件（gateway.rs / config.rs / prompts.rs / models.rs）提交前 `git diff --cached --name-only` 核对未误纳并行会话改动。**每块编译验证后立即 commit，不积压未提交改动**（上次被并行会话 `git stash` 卷走的教训）。
- **commit 前缀**：所有提交用 `[ptier]` 前缀。
- **不碰 models.rs**：run log tier 信息塞进既有 `AgentRunLog.gateway_result: Document` 自由字段。
- **基线**：`cargo test --lib` ≥ 350 passed / 0 failed 不回归。
- **当前 PROMPT_PACK_VERSION**：`wechatagent_prompt_pack_v10_2026_06_24_bayesian_obs`（块 E bump 到 v11，须先核对并行会话 schema 无文本冲突）。

---

### Task 1: sufficiency.rs 纯谓词层（强升判据 + 观测收窄 + 回落 Full + 自评识别）

**Files:**
- Modify: `src/agent/sufficiency.rs`（谓词 + 单测都在此文件 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `super::guards::decision_requires_knowledge(&AgentDecision) -> bool`（已存在，guards.rs:68，三态 `required|insufficient|knowledge_required`）；`AgentDecision.sufficiency: String`、`.knowledge_need: String`、`.missing_tier: String`。
- Produces（块 B/gateway 依赖）：
  - `pub(crate) fn should_force_full_on_missing(decision: &AgentDecision, knowledge_coverage: &str) -> bool`
  - `pub(crate) fn is_coverage_optimism(decision: &AgentDecision, knowledge_coverage: &str) -> bool`（收窄为仅 weak）
  - `pub(crate) fn is_sufficiency_recognized(decision: &AgentDecision) -> bool`
  - `pub fn decide_tier_escalation(decision: &AgentDecision) -> TierDecision`（签名不变，仅兜底改 Full）

- [ ] **Step 1: 写强升谓词的失败测试**

在 `src/agent/sufficiency.rs` 的 `mod tests` 末尾（`coverage_optimism_*` 测试附近）追加。注意现有 `decision_with_need(sufficiency, knowledge_need)` helper 已存在，复用它：

```rust
    #[test]
    fn force_full_hits_on_enough_missing_and_needs_knowledge() {
        let d = decision_with_need("enough", "required");
        assert!(should_force_full_on_missing(&d, "missing"));
    }

    #[test]
    fn force_full_skips_weak_and_adequate_coverage() {
        // weak 归观测、不强升；enough/not_required 覆盖足够不强升。
        let d = decision_with_need("enough", "required");
        assert!(!should_force_full_on_missing(&d, "weak"));
        assert!(!should_force_full_on_missing(&d, "enough"));
        assert!(!should_force_full_on_missing(&d, "not_required"));
    }

    #[test]
    fn force_full_skips_when_knowledge_not_needed() {
        // 寒暄轮 knowledge_need=not_required，即便 coverage=missing 也不强升。
        let d = decision_with_need("enough", "not_required");
        assert!(!should_force_full_on_missing(&d, "missing"));
    }

    #[test]
    fn force_full_requires_positive_enough_not_negation() {
        // _=>Enough 兜底的 unknown/空不是"自评够了"，不强升。
        assert!(!should_force_full_on_missing(&decision_with_need("unknown", "required"), "missing"));
        assert!(!should_force_full_on_missing(&decision_with_need("", "required"), "missing"));
        assert!(!should_force_full_on_missing(&decision_with_need("need_more_context", "required"), "missing"));
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib sufficiency::tests::force_full 2>&1 | tail -15`
Expected: FAIL，编译错误 `cannot find function should_force_full_on_missing`。

- [ ] **Step 3: 实现强升谓词**

在 `is_coverage_optimism` 函数定义**之前**插入：

```rust
/// 纯谓词：本轮是否构成「确定高危、必须当场升 Full」——自评说够了（enough），但本轮确实
/// 需要产品知识（decision_requires_knowledge）、且知识路由覆盖度为 `missing`（连弱证据都没有）。
///
/// 与 [`is_coverage_optimism`] 正交：missing → 强升（本谓词，硬动作）；weak → 观测（那个谓词，
/// 先观测后判罚）。两者各管一态，互不重叠。必须正向 `== "missing"`，绝不用 `!=`。
pub(crate) fn should_force_full_on_missing(
    decision: &AgentDecision,
    knowledge_coverage: &str,
) -> bool {
    decision.sufficiency.as_str() == "enough"
        && knowledge_coverage == "missing"
        && super::guards::decision_requires_knowledge(decision)
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib sufficiency::tests::force_full 2>&1 | tail -15`
Expected: PASS（4 个 force_full 测试）。

- [ ] **Step 5: 收窄 is_coverage_optimism 到仅 weak + 改对应测试**

把 `is_coverage_optimism` 函数体的 coverage 匹配从 `matches!(knowledge_coverage, "missing" | "weak")` 改为仅 weak，并更新其 doc 注释：

```rust
/// 纯谓词：本轮是否构成「需观测的自评乐观灰区」——自评说够了（enough）、本轮需产品知识、
/// 但知识覆盖只是 `weak`（有弱证据、未硬到 missing）。missing 已由
/// [`should_force_full_on_missing`] 强升接管，本谓词只盯不硬堵的 weak 灰区。
///
/// 命中只记观测 telemetry（先观测后判罚），不改档位决策。正向 `== "weak"`，绝不用 `!=`。
pub(crate) fn is_coverage_optimism(decision: &AgentDecision, knowledge_coverage: &str) -> bool {
    decision.sufficiency.as_str() == "enough"
        && knowledge_coverage == "weak"
        && super::guards::decision_requires_knowledge(decision)
}
```

然后改现有测试。找到 `coverage_optimism_hits_on_enough_plus_missing_plus_required`，改为断言 missing **不再**命中（归强升）：

```rust
    #[test]
    fn coverage_optimism_only_weak_not_missing() {
        // 收窄后：missing 归强升、不再算观测乐观；weak 才算。
        let d = decision_with_need("enough", "required");
        assert!(!is_coverage_optimism(&d, "missing"));
        assert!(is_coverage_optimism(&d, "weak"));
    }
```

找到 `coverage_optimism_hits_on_weak_too`，它断言 weak 命中——保留（仍正确），但删掉其中对 missing 的任何断言（若有）。检查 `coverage_optimism_skips_when_coverage_adequate` / `_skips_when_knowledge_not_needed` / `_requires_positive_enough_match_not_negation` 三个测试：它们用 enough/not_required 或 unknown，与 weak 无关，保持不变即可。若 `coverage_optimism_hits_on_enough_plus_missing_plus_required` 还存在则删除（被上面新测试取代）。

- [ ] **Step 6: 改 decide_tier_escalation 兜底为 Full + 改测试**

把 `decide_tier_escalation` 里 `need_more_context` 分支的 missing_tier 兜底：

```rust
        "need_more_context" => {
            let tier = match decision.missing_tier.as_str() {
                "relational" => PromptTier::Relational,
                "full" => PromptTier::Full,
                // 非法值回落 Full（更保守，宁可多注入，避免复合高价值轮被卡在无知识档）。
                _ => PromptTier::Full,
            };
            TierDecision::Escalate(tier)
        }
```

改测试 `test_need_more_context_invalid_tier_falls_back_to_relational`：

```rust
    #[test]
    fn test_need_more_context_invalid_tier_falls_back_to_full() {
        let d = make_decision("need_more_context", "garbage");
        assert_eq!(decide_tier_escalation(&d), TierDecision::Escalate(PromptTier::Full));
    }
```

- [ ] **Step 7: 写 is_sufficiency_recognized 谓词 + 测试**

在 `should_force_full_on_missing` 附近插入：

```rust
/// 纯谓词：sufficiency 是否落在已知三态（enough / need_more_context / need_clarification）内。
/// false = LLM 输出畸形（空/乱值），decide_tier_escalation 会走 `_=>Enough` 兜底 = 静默降级，
/// 应被观测（块 B 的 ptier_self_assessment_malformed）。
pub(crate) fn is_sufficiency_recognized(decision: &AgentDecision) -> bool {
    matches!(
        decision.sufficiency.as_str(),
        "enough" | "need_more_context" | "need_clarification"
    )
}
```

测试：

```rust
    #[test]
    fn sufficiency_recognized_three_states_only() {
        assert!(is_sufficiency_recognized(&make_decision("enough", "")));
        assert!(is_sufficiency_recognized(&make_decision("need_more_context", "")));
        assert!(is_sufficiency_recognized(&make_decision("need_clarification", "")));
        assert!(!is_sufficiency_recognized(&make_decision("", "")));
        assert!(!is_sufficiency_recognized(&make_decision("garbage", "")));
    }
```

- [ ] **Step 8: 运行 sufficiency 全模块测试**

Run: `cargo test --lib sufficiency:: 2>&1 | tail -20`
Expected: PASS，全部 sufficiency 测试通过（含新增强升 4 + 收窄 1 + 自评识别 1 + 既有）。

- [ ] **Step 9: 提交**

```bash
git add src/agent/sufficiency.rs
git commit -m "$(cat <<'EOF'
[ptier] feat(sufficiency): 强升谓词+观测收窄weak+回落Full+自评识别(加固块A)

- should_force_full_on_missing(enough && missing && 需知识):决定gateway强升,正向匹配
- is_coverage_optimism 收窄为仅 weak(missing 归强升,两谓词正交)
- missing_tier 非法值回落 Relational→Full(更保守)
- is_sufficiency_recognized 三态识别,供自评失效观测判据

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: gateway 接线（强升 + 对称观测 + used_knowledge_ids 口径 + run log tier 字段）

**Files:**
- Modify: `src/agent/gateway.rs`（`run_user_operation_gateway_inner` 两程循环段 ~940-1070，run log 写入段 ~4106-4180）

**Interfaces:**
- Consumes: Task 1 的 `should_force_full_on_missing` / `is_coverage_optimism`(weak) / `is_sufficiency_recognized`；既有 `decide_reply_with_promote(..., PromptTier)`、`write_event_for_account(state, account_id, contact_wxid, kind, status, summary, Option<Document>)`、`route_used_knowledge_ids(&knowledge_route)`、`knowledge_route.knowledge_coverage`、`contact.intent_trajectory`。
- Produces: 新事件 kind `ptier_forced_full` / `ptier_self_assessment_malformed` / `ptier_relational_optimism`；run log `gateway_result` Document 增键 `tier_used` / `sufficiency` / `escalated` / `forced_full`。

> **实现前先核对行号**：gateway.rs 是并行会话热点，行号可能漂移。先 `grep -n 'PromptTier::Lean\|TierDecision::Enough\|route_used_knowledge_ids\|decide_tier_escalation' src/agent/gateway.rs` 定位真实位置再改。

- [ ] **Step 1: 强升——在第一程判定后、Enough 分支内插入强升逻辑**

定位 `let tier_decision = crate::agent::sufficiency::decide_tier_escalation(&decision_first);` 后的 `match tier_decision { TierDecision::Enough => { ... } ... }`。当前 Enough 分支已有 `is_coverage_optimism` 观测块（块②之前做的）。改造 Enough 分支为：先查强升，命中则升 Full 第二程并标记 forced_full；否则走原观测逻辑。

注意 `decide_reply_with_promote` 第一程调用有 14 个参数（见 gateway:940-956），第二程强升复用同样的参数、仅 tier 改 Full。用一个可变标志 `forced_full` 记录是否强升，供后面写 run log。

```rust
    let mut forced_full = false;
    let (mut decision, mut promote_risks) = match tier_decision {
        crate::agent::sufficiency::TierDecision::Enough => {
            if crate::agent::sufficiency::should_force_full_on_missing(
                &decision_first,
                &knowledge_route.knowledge_coverage,
            ) {
                // ②强升：自评 enough 但 coverage=missing 且需知识 = 确定高危(凭空答产品/事实),
                // 当场升 Full 重生成。最多一次:Full 结果直接进五闸,不再触发强升(Full 已最高档)。
                forced_full = true;
                write_event_for_account(
                    state,
                    &contact.account_id,
                    Some(&contact.wxid),
                    "ptier_forced_full",
                    "info",
                    "第一程自评 enough 但 coverage=missing 且需知识，强制升 Full 重生成",
                    Some(doc! {
                        "run_id": &run_id,
                        "knowledge_coverage": &knowledge_route.knowledge_coverage,
                        "knowledge_need": &decision_first.knowledge_need,
                    }),
                )
                .await
                .ok();
                decide_reply_with_promote(
                    state, &contact, &inbound, &recent_messages, &pending_tasks,
                    playbook.as_ref(), domain_config.as_ref(), &runtime, &memory,
                    &context_pack, &selected_chunks, &knowledge_route, None, Some(&run_id),
                    crate::agent::sufficiency::PromptTier::Full,
                )
                .await?
            } else {
                // ①观测(weak 灰区):未强升时查收窄后的 is_coverage_optimism,只记不拦。
                if crate::agent::sufficiency::is_coverage_optimism(
                    &decision_first,
                    &knowledge_route.knowledge_coverage,
                ) {
                    write_event_for_account(
                        state, &contact.account_id, Some(&contact.wxid),
                        "ptier_coverage_optimism", "info",
                        "第一程自评 enough 但 coverage=weak 且需知识（观测，不拦截）",
                        Some(doc! {
                            "run_id": &run_id,
                            "knowledge_coverage": &knowledge_route.knowledge_coverage,
                            "knowledge_need": &decision_first.knowledge_need,
                        }),
                    )
                    .await
                    .ok();
                }
                (decision_first, promote_risks_first)
            }
        }
        // Escalate / Clarify 分支保持现状（Clarify 的事件增强在块②之前已做）。
        crate::agent::sufficiency::TierDecision::Escalate(target_tier) => {
            // ... 现有 Escalate 逻辑不动 ...
```

> 注意：上面的 Escalate/Clarify 分支只是示意，实现时**保留它们的现有代码原样**，只改 Enough 分支。把 `let (mut decision, mut promote_risks) =` 前面加 `let mut forced_full = false;`。

- [ ] **Step 2: 自评失效观测——在 decide_tier_escalation 调用后立即补**

在 `let tier_decision = ...decide_tier_escalation(&decision_first);` 之后、`match` 之前插入：

```rust
    // ①对称观测:第一程 sufficiency 落到 _=> 兜底(空/乱值)= 静默降级,记一条供发现。
    if !crate::agent::sufficiency::is_sufficiency_recognized(&decision_first) {
        write_event_for_account(
            state, &contact.account_id, Some(&contact.wxid),
            "ptier_self_assessment_malformed", "warn",
            "第一程 sufficiency 非已知三态，decide_tier_escalation 走兜底（静默降级）",
            Some(doc! { "run_id": &run_id, "sufficiency": &decision_first.sufficiency }),
        )
        .await
        .ok();
    }
```

- [ ] **Step 3: 关系档漏判观测——在 Enough 分支未强升路径补**

在 Step 1 的 `else` 分支里（is_coverage_optimism 观测之后），追加关系档漏判观测。判据：enough 且本轮触及关系信号（intent_trajectory 非空）却停在 Lean（即走了 Enough、没强升、没升档）：

```rust
                // ①对称观测:自评 enough 停 Lean,但本轮触及关系信号(意图轨迹非空)→ 疑似关系档漏判。
                if decision_first.sufficiency == "enough" && !contact.intent_trajectory.is_empty() {
                    write_event_for_account(
                        state, &contact.account_id, Some(&contact.wxid),
                        "ptier_relational_optimism", "info",
                        "第一程自评 enough 停 Lean，但本轮存在意图轨迹（疑似关系档漏判，观测）",
                        Some(doc! {
                            "run_id": &run_id,
                            "intent_trajectory_len": contact.intent_trajectory.len() as i64,
                        }),
                    )
                    .await
                    .ok();
                }
```

> 核实 `contact.intent_trajectory` 字段名与类型：`grep -n 'intent_trajectory' src/models.rs`。若是 `Vec<_>` 则 `.is_empty()`/`.len()` 可用；若是 `Option<Vec<_>>` 则改 `.as_ref().map_or(0, |v| v.len())` 且判据用 `.map_or(false, |v| !v.is_empty())`。

- [ ] **Step 4: 修 used_knowledge_ids 口径——仅 Lean-Enough 终决策那处**

`grep -n 'used_knowledge_ids = route_used_knowledge_ids' src/agent/gateway.rs` 找全部赋值点。定位**第一程 Lean 走 Enough 后、未强升、未升档**的终决策赋值（块②之前在 ~1066 `decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);`）。改为只在该决策实际带知识时记：

```rust
    // ⑤口径修正:Lean 第一程未注入知识(走 Enough 未强升/未升档)却记路由命中 id,会架空
    // grounding 硬闸(取 used∩verified 非空即放行)。仅当本决策确实经知识档(forced_full 或
    // 升档=注入了知识)时才记路由 id;纯 Lean-Enough 决策不记(它没读过任何切片)。
    if forced_full || matches!(tier_decision, crate::agent::sufficiency::TierDecision::Escalate(_)) {
        decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
    }
```

> 风险核对：确认这处赋值是否在 `tier_decision` 仍在作用域内（match 是 move 还是 borrow）。`TierDecision` 实现了 `Clone+PartialEq`，若被 match 消费了，改用 Step 1 设的 `forced_full` + 另设一个 `let escalated = matches!(tier_decision, Escalate(_));` 在 match 前求值保存。**实现时优先在 match 前求值 `let escalated = matches!(...);` 避免作用域问题。** 其余 4 处赋值（初始 planner gateway:231、revised gateway:1197/1493、Step1 强升后的 Full 决策天然带知识）保持不动。

- [ ] **Step 5: run log tier 字段——写进 gateway_result Document**

`grep -n 'gateway_result' src/agent/gateway.rs` 定位写 run log 处（~4106-4180，`send_gateway_result: to_document(gateway_result)` 或构造 `gateway_result` Document 处）。在组装 run log 的 gateway_result Document 时合并 tier 观测字段。找到最终写 run log 的位置，把 tier 信息加入。由于 tier 状态（tier_used/escalated/forced_full）在两程循环局部，需用一个变量串到 run log 写入点：在两程循环结束后构造一个 `let ptier_meta = doc! { "tier_used": ..., "sufficiency": &decision.sufficiency, "escalated": escalated, "forced_full": forced_full };` 并合并进 run log 的 gateway_result。

```rust
    // run log tier 元信息(不碰 models.rs,塞既有 gateway_result Document)。
    // tier_used: Lean 走 Enough 未升=lean;forced_full=full;Escalate=对应档。
    let tier_used = if forced_full {
        "full"
    } else if escalated {
        "escalated"
    } else {
        "lean"
    };
    // 在写 agent_run_logs 的 gateway_result Document 合并点插入这几个键(见该函数实际组装处)。
```

> 此步与具体 run log 写入函数耦合，实现时读 `write_run_log` / `finalize` 相关函数（gateway.rs:4106 区域 `record_*` / `to_document(gateway_result)`），把 `tier_used`/`sufficiency`/`escalated`/`forced_full` 合并进已有的 gateway_result Document。若 run log 写入点拿不到这些局部变量，最简做法：在两程循环结束处直接 `write_event_for_account(... "ptier_run_tier" ... doc!{tier_used,sufficiency,escalated,forced_full})` 落一条事件（与既有 ptier_* 事件同形），等效可观测、且完全不依赖 run log 函数签名改动。**优先用事件方式（零签名改动、零跨函数变量穿透）。**

- [ ] **Step 6: 编译验证**

Run: `cargo test --lib 2>&1 | tail -8`
Expected: 编译通过，lib 基线 ≥350 passed / 0 failed。

- [ ] **Step 7: 提交**

```bash
git add src/agent/gateway.rs
# 核对未误纳并行会话改动:
git diff --cached --name-only
git commit -m "$(cat <<'EOF'
[ptier] feat(gateway): 高危强升Full+灰区/关系档/自评失效对称观测+used_knowledge_ids口径(加固块B)

- 第一程 Enough 后先查 should_force_full_on_missing,missing+需知识强升Full(最多一次,Full后不再升)
- 补 ptier_self_assessment_malformed(自评畸形静默降级)+ptier_relational_optimism(关系档漏判)
  +observation 覆盖;is_coverage_optimism 收窄后只记 weak 灰区
- used_knowledge_ids 仅在 forced_full/Escalate(实际注入知识)时记,纯Lean-Enough不记(防架空grounding硬闸)
- run tier 元信息走 ptier_run_tier 事件(不碰 models.rs)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: config 运维开关 PROGRESSIVE_TIER_ENABLED

**Files:**
- Modify: `src/config.rs`（struct 字段区 ~52/179、构造区 ~436/572、debug 打印区 ~542）
- Modify: `src/agent/gateway.rs`（第一程 tier 选择 ~955）
- Modify: `.env.example`（加文档）

**Interfaces:**
- Consumes: 既有 `parse_bool` / `env_or` helper、`state.config` 访问模式。
- Produces: `Config.progressive_tier_enabled: bool`（默认 true）。

- [ ] **Step 1: config struct 加字段**

`src/config.rs` 在 `pub knowledge_exploration_enabled: bool,`（:179 附近）后加：

```rust
    /// 默认 **true**——渐进式三档机制开关（与多数 *_ENABLED 默认 false 相反）。
    /// 关时第一程直接传 Full、退回 ptier 前单程行为，等于 kill switch（上线初期止损/灰度/A-B）。
    pub progressive_tier_enabled: bool,
```

- [ ] **Step 2: config 构造区读 env（默认 true）**

在 `knowledge_exploration_enabled: parse_bool(...)`（:572 附近）旁加：

```rust
            progressive_tier_enabled: parse_bool(&env_or("PROGRESSIVE_TIER_ENABLED", "true")),
```

- [ ] **Step 3: gateway 第一程 tier 读开关**

`src/agent/gateway.rs` 第一程 `decide_reply_with_promote(..., crate::agent::sufficiency::PromptTier::Lean)`（:955）改为按开关选档。在该调用前求值：

```rust
    // PROGRESSIVE_TIER_ENABLED 关 → 第一程直接 Full,退回单程(kill switch)。
    let first_pass_tier = if state.config.progressive_tier_enabled {
        crate::agent::sufficiency::PromptTier::Lean
    } else {
        crate::agent::sufficiency::PromptTier::Full
    };
```

把 `decide_reply_with_promote(...)` 第一程调用末参 `PromptTier::Lean` 改为 `first_pass_tier`。

> 注意：开关关时第一程是 Full，第一程自评仍会跑，但 Full 档信息已全量、decide_tier_escalation 多半判 Enough、should_force_full_on_missing 因 coverage 可能仍 missing 而触发强升再跑一次 Full（浪费）。为避免：开关关时跳过强升。在 Task 2 Step 1 的强升条件加 `state.config.progressive_tier_enabled &&` 前缀——即开关关时不强升（已经是 Full 了）。**实现 Task 3 时回到 gateway 强升块补这个前缀。**

- [ ] **Step 4: .env.example 文档**

`.env.example` 找到其他 `*_ENABLED` 附近加：

```
# 渐进式三档提示词加载机制开关(默认 true)。关闭=第一程直接全量 Full、退回单程,
# 用于上线初期止损 / 账号灰度 / A-B 对照。
PROGRESSIVE_TIER_ENABLED=true
```

- [ ] **Step 5: 编译验证**

Run: `cargo test --lib 2>&1 | tail -6`
Expected: 编译通过，基线不回归。

- [ ] **Step 6: 提交**

```bash
git add src/config.rs src/agent/gateway.rs .env.example
git diff --cached --name-only
git commit -m "$(cat <<'EOF'
[ptier] feat(config): PROGRESSIVE_TIER_ENABLED 开关(默认true,关退回单程Full)(加固块C)

kill switch:关时第一程直接 Full、跳过强升、退回 ptier 前单程,给上线止损/灰度/A-B。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: prompt 收紧 Clarify + bump v11

**Files:**
- Modify: `src/prompts.rs`（`PROMPT_PACK_VERSION` :15、`user.reply.task` 模板 sufficiency 字段说明处）

**Interfaces:**
- Consumes: 无。
- Produces: PROMPT_PACK_VERSION → v11；user.reply.task 模板 sufficiency 段加澄清约束。

- [ ] **Step 1: 先核对并行会话 schema 无文本冲突**

Run: `grep -n 'sufficiency\|clarificationIntent\|need_clarification' src/prompts.rs`
确认 sufficiency/clarificationIntent 字段说明段（块②之前加的）仍在、文本未被并行会话改动。记下要改的那一行。

- [ ] **Step 2: 在 sufficiency 字段说明处加澄清约束**

找到 `user.reply.task` 模板里 `"sufficiency": "enough"` 或 `clarificationIntent` 的说明注释段（块②之前加的三档自评契约），在其后追加一句约束（保持 JSON 注释风格一致）：

```
  // 【need_clarification 硬约束】当 sufficiency=need_clarification 时,replyText 只能是面向客户的
  // 澄清问句本身,不得给任何推测性答案/硬答(信息不足时硬答=幻觉风险)。把不确定的点直接问清楚。
```

> 具体插入位置：紧跟 `"clarificationIntent": ...` 那一行之后。保持与现有注释缩进/全角标点一致。

- [ ] **Step 3: bump PROMPT_PACK_VERSION**

`src/prompts.rs:15`：

```rust
pub const PROMPT_PACK_VERSION: &str = "wechatagent_prompt_pack_v11_2026_06_24_clarify_tighten";
```

> 若执行时并行会话已 bump 到 >v10，则在其基础上 +1（vN→vN+1），保持单调递增、不抢号。

- [ ] **Step 4: 编译 + prompt 测试**

Run: `cargo test --lib prompts 2>&1 | tail -10`
Expected: 编译通过，prompt 相关测试（含 reply_schema_* 锚点测试）通过。

- [ ] **Step 5: 禁词扫描（no-human-takeover lint）**

Run: `git diff src/prompts.rs | grep '^+' | grep -nE 'human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工' || echo "✓ 无禁词"`
Expected: ✓ 无禁词。

- [ ] **Step 6: 提交**

```bash
git add src/prompts.rs
git diff --cached --name-only
git commit -m "$(cat <<'EOF'
[ptier] feat(prompts): Clarify收紧need_clarification只输出澄清问句不硬答+bump v11(加固块E)

user.reply.task sufficiency 段加硬约束:need_clarification 时 replyText 只能是澄清问句、
不得推测性硬答(信息不足硬答=幻觉)。纯LLM语义约束,agent-first。bump v10→v11。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: 真模型行为测试扩充（#[ignore] 留 CI）+ 基线终验

**Files:**
- Modify: `tests/real_llm_progressive_tier.rs`（加强升/关系档观测的真模型断言）

**Interfaces:**
- Consumes: 既有 `find_ptier_event(&state, wxid, kind)` helper、TestApp、真 LLM provider 构造（文件内已有）。

- [ ] **Step 1: 加强升真模型测试**

在 `tests/real_llm_progressive_tier.rs` 仿现有 p1/p2/p3 加一个 `#[ignore]` 测试：产品/价格问询 + 知识库无对应切片（coverage=missing）的客户消息，跑 handle_managed_message 后断言 `find_ptier_event(..., "ptier_forced_full")` 命中（软断言+观测，抗真模型抖动，仿 p2 风格）。

```rust
/// 产品问询 + 知识库 missing → 应强升 Full（ptier_forced_full）。软断言+观测。
#[tokio::test]
#[ignore]
async fn p4_product_inquiry_missing_coverage_forces_full() {
    // 仿 p2 结构:构造 managed contact + 产品问询 inbound,跑 handle_managed_message。
    // 拿到 ptier_forced_full 则 assert details 含非空 run_id;没拿到只 eprintln 不 panic。
    // (完整骨架照搬 p2_product_inquiry_escalates_to_full,把断言 kind 换 ptier_forced_full)
}
```

> 实现时照搬 `p2_product_inquiry_escalates_to_full` 的完整 fixture 构造与 env-gated skip 逻辑，仅把断言事件 kind 改 `ptier_forced_full`。

- [ ] **Step 2: 编译验证（不跑 ignore）**

Run: `cargo test --test real_llm_progressive_tier 2>&1 | tail -10`
Expected: 编译通过，纯函数测试 PASS、新真模型测试标记 ignored。

- [ ] **Step 3: 基线终验**

Run: `cargo test --lib 2>&1 | tail -6`
Expected: lib ≥350 passed / 0 failed。

- [ ] **Step 4: 提交**

```bash
git add tests/real_llm_progressive_tier.rs
git commit -m "$(cat <<'EOF'
[ptier] test(三档): 强升Full真模型行为测试(p4,#[ignore]留CI)(加固块收尾)

产品问询+知识库missing→断言 ptier_forced_full 命中(软断言+观测,抗真模型抖动)。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec 覆盖**：
- 块 A（强升谓词 + 收窄 + 回落 Full + 自评识别）→ Task 1 ✓
- 块 B（gateway 强升 + 对称观测 + used_knowledge_ids）→ Task 2 ✓
- 块 C（PROGRESSIVE_TIER_ENABLED）→ Task 3 ✓
- 块 D（run log tier，用既有 Document/事件）→ Task 2 Step 5 ✓
- 块 E（Clarify prompt + bump v11）→ Task 4 ✓
- 测试策略 → Task 1 单测 + Task 5 真模型 ✓

**占位扫描**：无 TODO/TBD。Task 2 的 run log 写入点和 used_knowledge_ids 作用域两处标了"实现时核对"，但都给了确定的 fallback 方案（事件方式 / match 前求值 `escalated`），非占位。

**类型一致性**：`should_force_full_on_missing` / `is_coverage_optimism` / `is_sufficiency_recognized` / `decide_tier_escalation` 签名在 Task 1 定义、Task 2 消费一致；`forced_full` / `escalated` 变量在 Task 2 内定义并贯穿到 Step 4/5；`PromptTier::Full` / `TierDecision::Escalate` 用法与既有一致。

**并行会话风险**：每 Task 独立 commit、只 add 自己文件、提交前 `git diff --cached --name-only` 核对——已写进每个提交步骤和 Global Constraints。
