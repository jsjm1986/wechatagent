# 通用化能力审查修复批次 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复通用化能力全面交叉审查终审报告确认的 4 必修 Medium + 全部可缓 Low + 文档订正（无 Critical/High）。

**Architecture:** 一组定义明确的独立修复，按「逻辑修复（TDD）→ 一致性 Low → CI/测试硬门 → 文档订正」组织。每条收口在单一函数/单点，互不耦合。policy 派生三处（G06/G11/G12）合并为一个共享 helper 子任务。

**Tech Stack:** Rust 2021 (Axum + MongoDB/BSON)，cargo test。

## Global Constraints

逐条复制自 spec（`docs/superpowers/specs/2026-06-20-universal-audit-remediation-design.md`）与项目 CLAUDE.md，每个任务隐含包含：

- **红线 ①DEFAULT 销售域字节等价**：DomainProfile 字段缺省（None/空集/false）时运行时行为与改造前逐字相同。`default_domain_profile()`（src/agent/domain_profile.rs）是销售基准。每条逻辑修复必须有「DEFAULT 路径不变」的断言或论证。
- **红线 ②serde 向后兼容**：新字段 `#[serde(default)]` 或 `skip_serializing_if`，老库文档能反序列化。
- **红线 ③AI 永不自动 verify**：知识入库恒 `status=draft + needs_review`，任何修复不得绕过。
- **红线 ④不造双真相源**：H13 发布后运行时只读 `operation_domain_configs`。
- **红线 ⑤无人工接管**：不出现「人工接管/转人工/takeover/hand-off/人工介入」（`scripts/check-no-human-takeover.sh` 词表）。held/blocked 用 AI 内部状态名。
- **红线 ⑥反过拟合**：只沉淀可复现抽象方法论，不对单条对话/单测点对点硬编码。
- **红线 ⑦boundary_protection 不被 profile 放宽**：soul/mode/orientation override 只换口吻/范式，不放宽反接管硬规则。
- **磁盘纪律**：编译前 `rm -rf target/debug/incremental` + `CARGO_INCREMENTAL=0`。本地只跑 `cargo test --lib` 和单个 PBT 文件，**绝不 `cargo build --tests`**（撑爆小磁盘）。需 Docker 的集成测试写 `#[ignore]` 留 CI，最多 `cargo test --test <name> --no-run` 编译检查。
- **基线门**：`cargo test --lib` ≥350/0；四 PBT（state_transition_pbt/memory_card_invariants/wiki_chunk_revision_pbt/llm_retry_jitter）累计 ≥33/0；`bash scripts/check-no-human-takeover.sh` 0 violations；`RUSTFLAGS="-Dwarnings"` 0 警告。新工作只加测试不降基线。
- **提交边界**：精确 `git add` 命名文件，**绝不 `git add -A`/`.`**。排除并行会话产物：`tests/real_llm_*`、`tests/roleplay_*`、`tests/common/*`、`.kiro/specs/universal-test-coverage/*`、`AGENTS.md`、`agent_t*.txt`、`t15_single.txt`、`docs/superpowers/plans/2026-06-18-*`。
- **subagent**：所有子代理 model:opus。提交需用户授权才推送/合并。回复中文。

---

## Task 1: G21 — 多租户首屏画像取错 workspace（必修 Medium）

**Files:**
- Modify: `src/agent/decision.rs:32-95`（`build_initial_operation_profile` 加 workspace_id 参数）
- Modify: `src/routes/contacts.rs:392,485,754`（三调用点传真实 workspace）
- Modify: `src/routes/management.rs:744`（MCP enable_contact_agent 调用点传 workspace_id）
- Modify: `tests/real_llm_ops_smoke.rs`（三处调用编译强制补参数：约 :1688/:2523 + 1 处）

**根因（已亲核）**：`build_initial_operation_profile`（decision.rs:32）内 4 处用 `state.config.default_workspace_id`（:41 load_user_operation_domain_config / :54 load_active_domain_profile / :63 load_prompt system / :69 load_prompt task），完全没有 workspace 参数。多租户下非默认 workspace 的 contact 首屏画像取的是**默认 workspace 的 profile/prompt**，错。4 个调用点都手握真实 workspace：contacts.rs:392（create，`admin.current_workspace` 在 :387）、:485（profile-note，`admin.current_workspace` 在 :483）、:754（`admin.current_workspace` 在上文 find_contact）、management.rs:744（MCP，局部变量 `workspace_id`）。

**Interfaces:**
- 改 `build_initial_operation_profile(state, note, playbook)` → `build_initial_operation_profile(state, workspace_id: &str, note, playbook)`（workspace_id 放 state 后第一参）。
- 4 处 `state.config.default_workspace_id` 改用 `workspace_id`。

- [ ] **Step 1: 改函数签名 + 内部 4 处取值**

`src/agent/decision.rs:32` 签名加 `workspace_id: &str` 参数（在 `state: &AppState` 后）。函数体内 4 处 `&state.config.default_workspace_id`（:41/:54/:63/:69）改成 `workspace_id`。注意 :54 的 `load_active_domain_profile(&state.db, workspace_id)`、:41 的 `load_user_operation_domain_config(state, workspace_id)`、:63/:69 的 `prompts::load_prompt(&state.db, workspace_id, ...)`。

- [ ] **Step 2: 改 4 个生产调用点传真实 workspace**

- `src/routes/contacts.rs:392`：`build_initial_operation_profile(&state, &admin.current_workspace, &payload.human_profile_note, Some(&playbook))`
- `src/routes/contacts.rs:485`：`build_initial_operation_profile(&state, &admin.current_workspace, &payload.human_profile_note, playbook.as_ref())`
- `src/routes/contacts.rs:754`：传该上下文的 `&admin.current_workspace`（先读 :700-754 确认变量名，可能是 `admin.current_workspace` 或已有 workspace 局部变量）
- `src/routes/management.rs:744`：`build_initial_operation_profile(state, workspace_id, &note, Some(&playbook))`（:740 已有 `workspace_id`）

- [ ] **Step 3: 改测试调用点（编译强制）**

`tests/real_llm_ops_smoke.rs` 三处调用补参数。这些是 `#[ignore]` 真模型测试，传 `&state.config.default_workspace_id`（或测试里已有的 workspace 常量）保持原单测语义。先 grep `build_initial_operation_profile` 在该文件的全部行号，逐个补。注意本文件是并行会话产物边缘——只改函数调用签名这一编译强制改动，不动测试逻辑。

- [ ] **Step 4: 编译检查（磁盘纪律）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib ≥350 passed, 0 failed, 0 warnings。
再 `CARGO_INCREMENTAL=0 cargo test --test real_llm_ops_smoke --no-run 2>&1 | tail -5` 确认该测试文件编译过。

- [ ] **Step 5: Commit**

```bash
git add src/agent/decision.rs src/routes/contacts.rs src/routes/management.rs tests/real_llm_ops_smoke.rs
git commit -m "fix(universal/G21): build_initial_operation_profile 加 workspace_id 参数(修多租户首屏画像取错 workspace)"
```

**测试说明**：本任务正确性靠编译强制（4 调用点签名）+ 多租户语义论证。无独立纯函数可单测（函数体是 LLM + DB IO）。DEFAULT 单租户下 `current_workspace`==`default_workspace_id`，行为不变（红线①）。

---

## Task 2: G13 — 五闸阈值无 clamp（必修 Medium）

**Files:**
- Modify: `src/agent/runtime.rs:225-245`（`apply_profile_threshold_overrides` 加 clamp）
- Test: `src/agent/runtime.rs`（同文件 `#[cfg(test)]` mod，已有 M2 单测在 :818-884 附近）

**根因（已亲核）**：`apply_profile_threshold_overrides`（runtime.rs:225-245）是五闸阈值覆盖的**单一收口点**，五字段（fact_risk_block_at/pressure_risk_block_at/human_like_rewrite_below/emotional_value_rewrite_below/product_accuracy_block_below）直接 `self.x = v` 赋值无 clamp。admin 配 `fact_risk_block_at=100` → `review_passed` 的 `hallucination_score < 100` 恒真 → 幻觉硬闸禁用。所有写路径都过这个函数，clamp 加这里单点覆盖。runtime 阈值是 **i32**（`fact_risk_block_at: i32` runtime.rs:26）。注意：evolution/threshold.rs:52-53 的 `FIVE_GATE_HARD_MIN/MAX` 是 **f64 且模块私有**，不要跨模块引——直接用 i32 字面量 `.clamp(1, 10)`。

- [ ] **Step 1: 写失败测试**

在 runtime.rs 的 `#[cfg(test)]` mod 加（先读 :818-884 已有 M2 测试的构造模式，复用 `UserRuntimeParameters` 构造）：

```rust
#[test]
fn apply_threshold_overrides_clamps_out_of_range() {
    let mut rt = UserRuntimeParameters::from_config(&crate::config::Config::default());
    let overrides = crate::models::ProfileThresholds {
        fact_risk_block_at: Some(100),       // 越界高 → clamp 10
        pressure_risk_block_at: Some(0),     // 越界低 → clamp 1
        human_like_rewrite_below: Some(-5),  // 越界低 → clamp 1
        emotional_value_rewrite_below: Some(50), // 越界高 → clamp 10
        product_accuracy_block_below: None,  // None → 不动
    };
    let before_product = rt.product_accuracy_block_below;
    rt.apply_profile_threshold_overrides(Some(&overrides));
    assert_eq!(rt.fact_risk_block_at, 10);
    assert_eq!(rt.pressure_risk_block_at, 1);
    assert_eq!(rt.human_like_rewrite_below, 1);
    assert_eq!(rt.emotional_value_rewrite_below, 10);
    assert_eq!(rt.product_accuracy_block_below, before_product); // None 回落不动
}
```
（先核 `ProfileThresholds` 字段名与 `Config::default()` 是否存在；若 Config 无 default，用 :818-884 测试里现成的 runtime 构造方式。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib apply_threshold_overrides_clamps 2>&1 | tail`
Expected: FAIL（fact_risk_block_at == 100 而非 10）。

- [ ] **Step 3: 加 clamp**

`apply_profile_threshold_overrides`（runtime.rs:230-244）每个 `self.x = v;` 改为 `self.x = v.clamp(1, 10);`。五字段都加。加一行注释说明：`// G13: clamp 到 1..=10 防 admin 误配极值禁用安全硬闸（与 evolution THRESHOLD_REASONABLE_BANDS 同口径，此处独立写路径需自守）`。

- [ ] **Step 4: 跑测试确认通过 + DEFAULT 等价**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: 新测试 PASS，lib ≥350/0，0 警告。已有 M2 单测（overrides=None 不动、Some 正常范围值不变）应仍绿——确认 clamp 不影响 1..=10 内的正常值（红线①：DEFAULT None → 早 return 不进 clamp）。

- [ ] **Step 5: Commit**

```bash
git add src/agent/runtime.rs
git commit -m "fix(universal/G13): apply_profile_threshold_overrides clamp 五闸阈值到 1..=10(防 admin 误配禁用硬闸)"
```

---

## Task 3: G01 — grounding bypass 漏加在 review_passed（必修 Medium）

**Files:**
- Modify: `src/agent/review/gates.rs:20-33`（`review_passed` grounding 项加 bypass 守卫）
- Test: `src/agent/review/gates.rs`（同文件 `#[cfg(test)]` mod）

**根因（已亲核）**：`review_passed`（gates.rs:20-33）是权威 approve 闸（route_dual_gate:205-206 用它写 review.approved，finalize:769 用 review.approved 决定 Approved/Held）。其第 28 行 `review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below` **无条件、无 bypass 分支**。而 `classify_dual_gate`:120-121 已有守卫 `grounding_gate_applies = !runtime.grounding_gate_bypass_without_claim || claim_requires_product_knowledge(&review.claim_analysis)`。两者不对齐 → bypass=true + 无产品声明 + grounding 低分时：classify=AllPass 不写 needs_revision，但 review_passed=false → approved=false → finalize 落 held_by_ai_policy，纯情感回复仍被拦。H14 承诺未兑现。`claim_requires_product_knowledge`（guards.rs:319）签名 `(claim_analysis: &Document) -> bool`，pub(crate)，`review.claim_analysis` 是 Document，可直接调（classify_dual_gate 就这么用）。

- [ ] **Step 1: 写失败测试**

在 gates.rs 的 `#[cfg(test)]` mod 加（先读现有 review_passed 测试或 :1232 附近 use 区确认 DecisionReviewResult/UserRuntimeParameters 构造方式 + claim_analysis 怎么构造「无产品声明」的空 Document）：

```rust
#[test]
fn review_passed_honors_grounding_bypass_for_non_product_reply() {
    let mut runtime = /* 构造默认 UserRuntimeParameters，与现有测试同法 */;
    runtime.grounding_gate_bypass_without_claim = true;
    runtime.product_accuracy_block_below = 7;
    let mut review = /* 构造一个除 grounding 外全过、approved=true 的 DecisionReviewResult */;
    review.scores.knowledge_grounding_score = 3; // 低于阈值 7
    // claim_analysis 表示「无产品声明」(requiresProductKnowledge 缺失/false)
    review.claim_analysis = mongodb::bson::doc! {};
    assert!(review_passed(&review, &runtime), "bypass=true + 无产品声明 + 低 grounding 应放行(H14)");

    // 对照①：DEFAULT bypass=false → 低 grounding 仍拦(字节等价)
    runtime.grounding_gate_bypass_without_claim = false;
    assert!(!review_passed(&review, &runtime), "bypass=false 时低 grounding 必拦(字节等价)");

    // 对照②：bypass=true 但有产品声明 → 低 grounding 仍拦
    runtime.grounding_gate_bypass_without_claim = true;
    review.claim_analysis = mongodb::bson::doc! { "requiresProductKnowledge": true };
    assert!(!review_passed(&review, &runtime), "有产品声明时 bypass 不豁免 grounding");
}
```
（构造细节先照搬同文件现有 review_passed/classify 测试的 helper；claim_analysis 的 requiresProductKnowledge 字段名以 `claim_requires_product_knowledge`(guards.rs:319) 实际读的 key 为准——打开该函数确认。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib review_passed_honors_grounding_bypass 2>&1 | tail`
Expected: FAIL（第一个 assert：当前无条件判 grounding，bypass 不生效返回 false）。

- [ ] **Step 3: review_passed grounding 项加 bypass 守卫**

`review_passed`（gates.rs:25-32）把第 28 行
```rust
&& review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below
```
改为与 classify_dual_gate:120-121 对齐：
```rust
&& (!(runtime.grounding_gate_bypass_without_claim)
    || crate::agent::guards::claim_requires_product_knowledge(&review.claim_analysis)
    || review.scores.knowledge_grounding_score >= runtime.product_accuracy_block_below)
```
语义：bypass=true 且无产品声明 → 整段为真（豁免 grounding）；否则要求 grounding>=阈值。加注释引用 classify_dual_gate:120 同源。

- [ ] **Step 4: 跑测试确认通过 + 基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: 新测试 PASS，lib ≥350/0，0 警告。已有 review_passed/finalize 测试应仍绿（DEFAULT bypass=false 时新条件退化为原 `grounding>=阈值`，字节等价）。

- [ ] **Step 5: Commit**

```bash
git add src/agent/review/gates.rs
git commit -m "fix(universal/G01): review_passed grounding 项加 bypass 守卫(对齐 classify_dual_gate,兑现 H14 纯情感不被 grounding 误拦)"
```

---

## Task 4: G07 — 负反应率未 profile 化（必修 Medium）

**Files:**
- Modify: `src/evolution/post_release.rs:341-390`（`compute_negative_reaction_rate` 走 profile 极性 + 订正注释）
- Test: `src/evolution/post_release.rs`（同文件 `#[cfg(test)]` mod，已有 :464-477 附近的 classify 测试）

**根因（已亲核）**：`compute_negative_reaction_rate`（post_release.rs:351）签名**已有 `workspace_id` 参数**，但函数体 :358 import 裸 `classify_outcome_label`、:382 调它（写死销售负极 5 词），**没用** profile 极性。doc :346-347 却假声称「自动跟随 active DomainProfile.outcome_polarity」。参数化版本与加载件全部就绪：`classify_outcome_label_with_polarity`（gap_signals.rs:686）、`resolve_effective_polarity`（gap_signals.rs:765，空集回落销售 const，已有测试）。auto_release.rs:88 也调本函数（同极性源，自动一并修正）。

**Interfaces:**
- 用 `load_active_domain_profile(&state.db, workspace_id)`（src/agent/domain_profile.rs，返回 DomainProfile）取 profile.outcome_polarity → `resolve_effective_polarity(&polarity) -> (Vec<String>, Vec<String>)` → `classify_outcome_label_with_polarity(status, &positive, &negative)`。

- [ ] **Step 1: 写失败测试**

先读 gap_signals.rs:765 `resolve_effective_polarity` 签名 + :686 `classify_outcome_label_with_polarity` 签名确认参数顺序。本函数体是 DB aggregate，难纯函数单测；改为测「分类逻辑走 profile 极性」这个纯函数组合（不测 DB 部分）。在 post_release.rs 测试 mod 加：

```rust
#[test]
fn negative_reaction_classify_follows_profile_polarity() {
    use crate::knowledge_wiki::gap_signals::{classify_outcome_label_with_polarity, OutcomeLabel};
    // 情感域自定义负极
    let positive = vec!["user_emotion_opened_up".to_string()];
    let negative = vec!["user_went_cold".to_string()];
    // 自定义负极被识别为 Block(销售默认 5 词不含 user_went_cold)
    assert_eq!(
        classify_outcome_label_with_polarity(Some("user_went_cold"), &positive, &negative),
        OutcomeLabel::Block
    );
    // 销售默认负极在自定义极性下不再算 Block(证明真的换了极性源)
    assert_ne!(
        classify_outcome_label_with_polarity(Some("user_replied_objection"), &positive, &negative),
        OutcomeLabel::Block
    );
}
```
（此测试锁定「参数化分类按传入极性工作」；compute_negative_reaction_rate 接线正确性靠下面的代码改动 + 既有 #[ignore] 集成测试。注：gap_signals.rs:1912 已有 `classify_outcome_label_polarity_is_parametric` 覆盖类似语义——若重复则跳过本测试，只做代码改动，在报告里说明已有覆盖。）

- [ ] **Step 2: 跑测试确认（或确认已有等价测试绿）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib classify_outcome_label_polarity_is_parametric 2>&1 | tail`
Expected: 已有测试 PASS（证明参数化件就绪）。

- [ ] **Step 3: compute_negative_reaction_rate 改走 profile 极性**

`post_release.rs:358` 把 `use ...::{classify_outcome_label, OutcomeLabel};` 改为 `use ...::{classify_outcome_label_with_polarity, resolve_effective_polarity, OutcomeLabel};`。在 :377 `let mut hits` 之前加载 profile 极性：
```rust
let profile = crate::agent::domain_profile::load_active_domain_profile(&state.db, workspace_id).await;
let (positive, negative) = crate::knowledge_wiki::gap_signals::resolve_effective_polarity(&profile.outcome_polarity);
```
（先核 `load_active_domain_profile` 真实返回类型——是 `DomainProfile` 还是 `Result/Option`；post_release 用 `EvolutionError`，若返回 Result 需适配。decision.rs:52 的调用 `load_active_domain_profile(&state.db, ws).await` 直接拿到 profile 无 `?`，照此用法。）
把 :382 的 `classify_outcome_label(status)` 改为 `classify_outcome_label_with_polarity(status, &positive, &negative)`。

- [ ] **Step 4: 订正假声明注释**

把 :346-347 的「2.5-main-2 把 classify 的极性源换成 active DomainProfile.outcome_polarity 后，本观测指标自动跟随同一极性，无需二次接线」订正为实情：「G07 修复：本函数加载 active profile 的 outcome_polarity，与回路① 同源 classify_outcome_label_with_polarity；空极性回落销售 const（resolve_effective_polarity，字节等价）」。

- [ ] **Step 5: 跑测试 + 基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib ≥350/0，0 警告。post_release.rs:464-477 既有 classify 测试仍绿（DEFAULT 销售极性下 resolve_effective_polarity 回落销售 const，行为不变，红线①）。

- [ ] **Step 6: Commit**

```bash
git add src/evolution/post_release.rs
git commit -m "fix(universal/G07): compute_negative_reaction_rate 走 active profile 极性(与回路①同源,订正假声明注释)"
```

---

## Task 5: G06 + G11 + G12 — 状态机 policy 派生一致性（抽共享 helper，串行做）

**Files:**
- Modify: `src/routes/admin_ops_versions.rs:292-407`（抽出 policy 派生 loop 为 `reconcile_state_policies_for_machine` helper；no-op 短路 :242-249 前补 reconcile=G11；rollout/rollback :411-498 切 current 后调 reconcile=G12）
- Modify: `src/routes/domains.rs:87-128`（`update_operation_domain` $set state_machine 后调 helper=G06）
- Modify: `src/routes/domains.rs:140-168`（`update_operation_domain_state_machine` $set 后调 helper=G06）
- Test: `tests/domain_profile_e2e.rs`（新增 `#[ignore]` 集成测试，testcontainers）

**根因（已亲核）**：`forbidsProactive` 状态门由派生表 `operation_state_policies` enforce，而该表只在 `publish_state_machine_version`（admin_ops_versions.rs:210）的 :292-407 loop 里联动重派生。三处缺这个联动：
- **G06**：两个直编路由 `update_operation_domain`（domains.rs:119 `$set state_machine`）和 `update_operation_domain_state_machine`（:160 `$set state_machine`）直接改本体，**不**走 `publish_state_machine_version`、**不**派生 policy → 新增 `forbidsProactive:true` state 主动触达门 fail-open 静默失效（guards 对缺失 policy 行放行）。
- **G11**：no-op 幂等短路（:242-249）在 policy 派生 loop **之前** `return Ok(())`。若首次 activate 某 state 的 policy 派生 best-effort 失败（warn 跳过），之后重 activate 同本体走 no-op 短路，遗漏的 policy 行**永不补派**。
- **G12**：`rollout_operation_domain_version`（:411-443）/ `rollback_operation_domain_version`（:445-498）只切 `operation_domain_configs` 的 `current_version`，**不**碰 `operation_state_policies` → 切到的版本若 `forbidsProactive` 与当前 policy 行不一致，policy current 行漂移（下次 publish 才自愈）。

三处同根：缺「按当前 current 机器的 states 重派 policy」这一步。抽成幂等共享 helper，三处调用。helper 必须保持 :292-407 的全部语义不变：per-state warn-and-continue、只认 `current_version:true` 行、`is_refreshable_policy_seeded_by`（:174）区分机器派生行/手工行、`Ok(None)` 用 `next_version_for_scope` 避撞唯一索引。

**Interfaces:**
- Produces: `pub(crate) async fn reconcile_state_policies_for_machine(db: &Database, workspace_id: &str, domain: &str, state_machine: &Document, policy_seeded_by: &str, now: DateTime) -> ()`（best-effort，无 Result——内部 per-state warn-and-continue，与现有 loop 一致；不向外传播错误以免拖垮已成功的主操作）。
- Consumes（helper 内部，均已存在于 admin_ops_versions.rs）：`is_refreshable_policy_seeded_by`（:174）、`next_version_for_scope`、`m013_seed_user_operation_state_policies::derive_state_policy_lists`、`OperationStatePolicy`。

- [ ] **Step 1: 抽 helper（纯重构，不改语义）**

把 `publish_state_machine_version` 体内 :264-271（取 states）+ :292-407（policy 派生 loop）整段提取为新函数 `reconcile_state_policies_for_machine`，签名如上。helper 内部从 `state_machine.get_array("states")`（复用 :264-271 逻辑）取 states，再跑原 loop（:292-407 逐字搬，`policy_seeded_by` 改用入参 `&str`，`now` 用入参）。`publish_state_machine_version` 在 :280 `insert_new_current_domain_config` 之后改为调用：
```rust
reconcile_state_policies_for_machine(db, workspace_id, domain, &source_machine_for_policy, &policy_seeded_by, now).await;
```
注意：原 loop 在 `insert_new_current_domain_config` 把 `new_state_machine` move 走之后用 `&states`（:264 先取了 states 副本）。重构后 helper 需要 `&Document`——在 move 前先 `let machine_clone = new_state_machine.clone();`（或调整 insert 顺序，传引用给 helper 后再 insert）。**优先**：先 `let states_doc = new_state_machine.clone();` 再 insert，helper 收 `&states_doc`。clone 一次状态机本体在 admin 低频 publish 路径可接受。

- [ ] **Step 2: 编译确认重构等价**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib ≥350/0，0 警告。纯重构，行为不变。

- [ ] **Step 3: G11 — no-op 短路前补 reconcile**

`publish_state_machine_version` no-op 短路（:242-249）的 `return Ok(())` **之前**插入一次 reconcile（补派首次失败遗漏的 policy 行）：
```rust
if new_state_machine == source.state_machine {
    // G11: 本体未变仍 reconcile 一次——首次 activate 若某 state policy 派生 best-effort 失败,
    // 重激活同本体走此短路,遗漏行永不补;此处幂等 reconcile 补齐(已存在且一致的行内部 continue 不写)。
    let policy_seeded_by = format!("statemachine_publish:{seeded_by}");
    reconcile_state_policies_for_machine(db, workspace_id, domain, &source.state_machine, &policy_seeded_by, DateTime::now()).await;
    tracing::debug!(workspace_id, domain, "publish_state_machine_version: state machine unchanged, skip republish (no-op 幂等; policy 已 reconcile)");
    return Ok(());
}
```
reconcile 幂等：已存在且 (allowed,forbidden) 一致的行内部 `continue` 不写（:328），只补缺失/刷新陈旧机器派生行。

- [ ] **Step 4: G06 — 两直编路由调 helper**

`update_operation_domain`（domains.rs:126 `Ok(Json(...))` 之前）和 `update_operation_domain_state_machine`（:166 之前）在 `$set` 成功后调：
```rust
// G06: 直编路由改状态机本体后联动重派 policy(否则 forbidsProactive 新增 state 主动触达门 fail-open 静默失效)。
let policy_seeded_by = format!("statemachine_edit:{}", &domain);
crate::routes::admin_ops_versions::reconcile_state_policies_for_machine(
    &state.db, &admin.current_workspace, &domain, &payload.state_machine, &policy_seeded_by, mongodb::bson::DateTime::now(),
).await;
```
（`update_operation_domain` 的 payload.state_machine 是 `Document`；`update_operation_domain_state_machine` 的 payload 本身即 state_machine Document——传 `&payload`。先核 domains.rs 顶部 `use` 是否已引 admin_ops_versions；若 helper 是 `pub(crate)` 同 crate 可达。`statemachine_edit:` 前缀须通过 `is_refreshable_policy_seeded_by` 判定——当前该函数只认 `statemachine_publish:`/`legacy_migration`/None。**必须**同步把 `is_refreshable_policy_seeded_by`（admin_ops_versions.rs:177）的 `starts_with` 扩为 `s.starts_with("statemachine_publish:") || s.starts_with("statemachine_edit:")`，否则直编派生行下次 publish 被当手工行不刷新。）

- [ ] **Step 5: G12 — rollout/rollback 切 current 后 reconcile**

`rollout_operation_domain_version`（:441 `Ok(Json)` 前）：切 current 后用 `target.state_machine` reconcile：
```rust
// G12: 切 current 配置版本后按新 current 机器重派 policy(否则 policy current 行与机器 forbidsProactive 漂移)。
let policy_seeded_by = format!("statemachine_publish:{}", &target.seeded_by.clone().unwrap_or_default());
reconcile_state_policies_for_machine(&state.db, &target.workspace_id, &target.domain, &target.state_machine, &policy_seeded_by, now).await;
```
`rollback_operation_domain_version`（:496 前）：同理但用 `prev.state_machine` / `prev.workspace_id` / `prev.domain`。
（先核 `OperationDomainConfig` 是否有 `seeded_by` 字段及其类型；若无则用固定 `"statemachine_publish:rollout"` 标签。reconcile 只刷新机器派生行+补缺失，rollback 到的历史机器其 policy 行被重新对齐到该机器的 forbidsProactive。）

- [ ] **Step 6: 写 #[ignore] 集成测试（留 CI）**

`tests/domain_profile_e2e.rs` 加（先读该文件 testcontainers setup helper 复用方式，如 `db_create_profile`/真 handler 调用模式）：
```rust
#[tokio::test]
#[ignore] // testcontainers MongoDB，留 CI
async fn direct_edit_state_machine_rederives_policy() {
    // setup: 创建 domain config(含一个 forbidsProactive:false state)
    // PUT update_operation_domain_state_machine 带同 state 但 forbidsProactive:true
    // 断言: operation_state_policies 该 state current 行的 forbidden 含主动触达动作(派生生效)
}
```
本测试锁 G06 主路径。G11/G12 的漂移窗口窄，靠 helper 幂等性 + 代码论证（reconcile 内部已 test-covered 的 derive_state_policy_lists）。

- [ ] **Step 7: 编译 + 基线 + 集成编译检查**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib ≥350/0，0 警告。
再 `CARGO_INCREMENTAL=0 cargo test --test domain_profile_e2e --no-run 2>&1 | tail -5` 确认集成测试编译过。

- [ ] **Step 8: Commit**

```bash
git add src/routes/admin_ops_versions.rs src/routes/domains.rs tests/domain_profile_e2e.rs
git commit -m "fix(universal/G06+G11+G12): 抽 reconcile_state_policies_for_machine 共享 helper(直编路由/no-op/rollback 联动重派 policy,防 forbidsProactive 主动触达门静默漂移)"
```

**红线**：DEFAULT 销售域 publish 路径行为不变（helper 是原 loop 逐字搬）；reconcile 幂等（一致行不写）；best-effort 不阻断主操作（红线①+审计哲学）。

---

## Task 6: G31 — RISKY_FIELD 补 reviewer 取向 + prompt 锚漂移护栏（可缓 Low）

**Files:**
- Modify: `src/routes/domain_profiles.rs:685-741`（`RISKY_FIELD_NAMES` 11→13，`risky_fields_changed` 加两字段比较）
- Test: `src/routes/domain_profiles.rs`（同文件 `#[cfg(test)]` mod，:1020-1130 已有 risky_fields 测试）
- Test: `src/agent/domain_profile.rs`（同文件 `#[cfg(test)]` mod，加锚常量在真 prompt 中存在的断言）

**根因（已亲核）**：`RISKY_FIELD_NAMES`（domain_profiles.rs:685）是 `[&str; 11]`，不含 `reviewer_orientation`/`mode_gate_policy_override`（DomainProfile 字段 models.rs:1506/1513）。改这两个已生效血缘的 reviewer 取向字段绕过旁路稿二次确认即时生效。且锚常量 `DEFAULT_REVIEWER_REVIEW_FOCUS`（domain_profile.rs:488）/`DEFAULT_REVIEWER_BALANCE_PRINCIPLE`（:493）与真 prompt（reviewer system/user prompt）是两份独立字面量，无 `format!` 单一真相源、无测试断言真 prompt 含锚——锚漂移则 `apply_reviewer_review_focus` 静默找不到锚、原样返回不替换（profile 取向静默失效）。

- [ ] **Step 1: 写失败测试（RISKY_FIELD）**

domain_profiles.rs 测试 mod 加（复用 :1020-1130 已有 risky_fields 测试的 profile 构造 helper）：
```rust
#[test]
fn risky_fields_detects_reviewer_orientation_and_mode_gate() {
    let base = /* 默认 DomainProfile，与现有测试同构造 */;
    let mut changed_ro = base.clone();
    changed_ro.reviewer_orientation = Some(crate::models::ReviewerOrientation::default());
    assert!(risky_fields_changed(&base, &changed_ro).contains(&"reviewer_orientation"));

    let mut changed_mg = base.clone();
    changed_mg.mode_gate_policy_override = Some("x".to_string());
    assert!(risky_fields_changed(&base, &changed_mg).contains(&"mode_gate_policy_override"));
}
```
（先核 `ReviewerOrientation` 是否 `impl Default`；若否，构造一个 `Some(ReviewerOrientation { review_focus: Some("…".into()), .. })`。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib risky_fields_detects_reviewer 2>&1 | tail`
Expected: FAIL（当前清单不含两字段）。

- [ ] **Step 3: 扩 RISKY_FIELD_NAMES + 比较**

`RISKY_FIELD_NAMES` 改 `[&str; 13]`，末尾加 `"reviewer_orientation"`, `"mode_gate_policy_override"`。`risky_fields_changed` 末尾（:739 `transaction_facts_enabled` 之后）加：
```rust
if old.reviewer_orientation != new.reviewer_orientation {
    changed.push(RISKY_FIELD_NAMES[11]);
}
if old.mode_gate_policy_override != new.mode_gate_policy_override {
    changed.push(RISKY_FIELD_NAMES[12]);
}
```
更新 :683 doc 注释里的字段清单措辞（把 reviewer_orientation/mode_gate_policy_override 从"普通字段即时生效"挪到危险字段列表）。

- [ ] **Step 4: 写锚漂移护栏测试**

domain_profile.rs 测试 mod 加——断言真 reviewer prompt 含锚常量（prompt 真实位置先 grep 确认：reviewer system prompt 在 prompts.rs，user prompt 在 review/mod.rs 或 prompts.rs）：
```rust
#[test]
fn reviewer_prompt_contains_review_focus_anchor() {
    // 真 reviewer system prompt 必含 DEFAULT_REVIEWER_REVIEW_FOCUS 锚,否则 apply_reviewer_review_focus
    // 找不到锚静默不替换(profile 取向失效)。先确认真 prompt 来源(grep DEFAULT_REVIEWER_REVIEW_FOCUS 的消费点)。
    let sys = crate::prompts::/* reviewer system prompt 常量或构造函数 */;
    assert!(sys.contains(REVIEWER_REVIEW_FOCUS_LABEL), "reviewer prompt 须含锚标签");
    assert!(sys.contains(DEFAULT_REVIEWER_REVIEW_FOCUS), "reviewer prompt 须含默认取向锚(漂移则 profile override 静默失效)");
}
```
先 grep `DEFAULT_REVIEWER_REVIEW_FOCUS` / `REVIEWER_REVIEW_FOCUS_LABEL` 全仓消费点确认真 prompt 字符串来源——可能是 prompts.rs 的 const 或 ensure_prompt_pack_v2 的种子文本。若真 prompt 在 DB 种子（运行时才有），改为断言 `prompts.rs` 里的种子常量字符串含锚。balance_principle 同理加一个测试。

- [ ] **Step 5: 跑测试 + 基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: 新测试 PASS，lib ≥350/0，0 警告。

- [ ] **Step 6: Commit**

```bash
git add src/routes/domain_profiles.rs src/agent/domain_profile.rs
git commit -m "fix(universal/G31): RISKY_FIELD 补 reviewer_orientation/mode_gate_policy_override + reviewer prompt 锚漂移护栏测试"
```

**红线**：boundary 不放宽（G31 子断言已证 profile override 替换不掉红线）；纯增字段+测试，DEFAULT None 行为不变。

---

## Task 7: G03 — renewal/reactivation 扫描器短路漏 per_relationship（可缓 Low）

**Files:**
- Modify: `src/planner/mod.rs:1763-1765`（scan_renewal 短路条件）
- Modify: `src/planner/mod.rs:1953-1955`（scan_reactivation 短路条件）
- Test: `src/planner/mod.rs`（同文件 `#[cfg(test)]` mod，:2995-3168 已有 resolve_operation_mode 测试）

**根因（已亲核）**：`scan_renewal`（planner/mod.rs:1763）短路 `if !profile.operation_mode.renewal.enabled { return Ok(()); }` 只看 profile 默认层。profile 默认 renewal 关 + `per_relationship_operation_mode["customer"].renewal.enabled=true` 时，scan 第一行就 return，per_relationship 开启的续费范式整段跳过。profile 已在 :1756-1757 加载，可读 `per_relationship_operation_mode`（models.rs:1410，`Option<BTreeMap<String, OperationMode>>`）。reactivation（:1953）同。注释 :1758-1762 已自承 deferred。**注意**：这是扫描器级粗过滤（省 DB 扫描），逐 contact 仍走 `resolve_operation_mode`（:938）三级链——所以只需放宽粗过滤，不改逐 contact 逻辑。contact 级 override 开启的边缘（profile+per_relationship 都关但某 contact override 开）本轮不覆盖（需全表扫描每个 contact override，与"省 DB 扫描"初衷冲突，留注释说明）。

- [ ] **Step 1: 写失败测试**

planner/mod.rs 测试 mod 加纯函数判定 helper + 测试（不测整个 scan 的 DB 部分，测短路判定逻辑）。先抽短路条件为纯函数：
```rust
/// 扫描器级粗过滤：profile 默认开 OR per_relationship 任一关系类型开 → 放行扫描。
/// 纯函数,便于单测;contact 级 override 的边缘不在此层覆盖(逐 contact resolve 兜底)。
fn renewal_scan_should_run(profile: &crate::models::DomainProfile) -> bool {
    profile.operation_mode.renewal.enabled
        || profile.per_relationship_operation_mode.as_ref().is_some_and(|m| {
            m.values().any(|om| om.renewal.enabled)
        })
}
```
测试：
```rust
#[test]
fn renewal_scan_runs_when_per_relationship_enables_it() {
    let mut profile = crate::models::DomainProfile::default(); // renewal 默认关
    assert!(!renewal_scan_should_run(&profile), "DEFAULT: profile 默认关 + 无 per_relationship → 不扫(字节等价)");

    let mut customer = crate::models::OperationMode::default();
    customer.renewal.enabled = true;
    let mut map = std::collections::BTreeMap::new();
    map.insert("customer".to_string(), customer);
    profile.per_relationship_operation_mode = Some(map);
    assert!(renewal_scan_should_run(&profile), "per_relationship customer 开 renewal → 应扫");
}
```
（先核 `DomainProfile::default()` 的 renewal 是否真为 false——是销售 DEFAULT，renewal 默认关，符合。reactivation 同写一个 `reactivation_scan_should_run` + 测试。）

- [ ] **Step 2: 跑测试确认失败（编译失败=helper 未定义）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib renewal_scan_runs_when_per_relationship 2>&1 | tail`
Expected: 编译失败（helper 未定义）→ 定义后 FAIL 或 PASS。

- [ ] **Step 3: 加 helper + 改两处短路**

定义 `renewal_scan_should_run` / `reactivation_scan_should_run`（放 resolve_operation_mode 附近 :938）。`scan_renewal`（:1763）改：
```rust
if !renewal_scan_should_run(&profile) {
    return Ok(());
}
```
`scan_reactivation`（:1953）改用 `reactivation_scan_should_run`。更新 :1758-1762 注释：删 deferred 自承，改为「粗过滤：profile 默认开或 per_relationship 任一开即扫；contact 级 override 的边缘靠逐 contact resolve_operation_mode 兜底（不在粗过滤层全表扫 override，省 DB 扫描）」。

- [ ] **Step 4: 跑测试 + 基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: 新测试 PASS，lib ≥350/0，0 警告。:2995-3168 既有 resolve_operation_mode 测试仍绿。

- [ ] **Step 5: Commit**

```bash
git add src/planner/mod.rs
git commit -m "fix(universal/G03): renewal/reactivation 扫描器短路放宽到 per_relationship 任一开(数字分身续费范式不再被默认层吞掉)"
```

**红线**：DEFAULT（profile 默认关 + 无 per_relationship）→ 短路返回，零 DB 扫描，字节等价。

---

## Task 8: G04 — quiet_hours 覆盖绕过 resolve_operation_mode（可缓 Low）

**Files:**
- Modify: `src/agent/quiet_hours.rs:94-103`（`effective_quiet_hours_enabled` 加 profile 参数，经 resolve_operation_mode 读 profile 级）
- Modify: `src/agent/gateway.rs:2068-2071`（消费点传 profile）
- Modify: `src/webhooks.rs:565`（消费点传 profile）
- Test: `src/agent/quiet_hours.rs`（同文件 `#[cfg(test)]` mod）

**根因（已亲核）**：`effective_quiet_hours_enabled`（quiet_hours.rs:94）只读 `contact.operation_mode_override.quiet_hours.enabled_override`，不经 `resolve_operation_mode`（planner:938）三级链。profile 级设 `quiet_hours.enabled_override=Some(false)`（QuietHoursMode models.rs:1757，`OperationMode.quiet_hours` 字段 :1664）运行时无效。OperationMode 其余驱动力都走 resolve 三级，唯 quiet_hours 漏接。两消费点：gateway.rs:2068（FollowUp 静默门）、webhooks.rs:565。

**Interfaces:**
- 改 `effective_quiet_hours_enabled(contact, global_enabled)` → `effective_quiet_hours_enabled(contact, profile, global_enabled)`，内部用 `resolve_operation_mode(contact, profile).quiet_hours.enabled_override.unwrap_or(global_enabled)`。
- Consumes: `crate::planner::resolve_operation_mode(contact, profile) -> OperationMode`（:938，pub(crate)）。

- [ ] **Step 1: 写失败测试**

quiet_hours.rs 测试 mod 加（先核 resolve_operation_mode 可见性 + DomainProfile 构造）：
```rust
#[test]
fn quiet_hours_honors_profile_level_override() {
    use crate::models::{Contact, DomainProfile, OperationMode};
    let contact = /* Contact，operation_mode_override = None */;
    let mut profile = DomainProfile::default();
    let mut om = OperationMode::default();
    om.quiet_hours.enabled_override = Some(false); // profile 级显式关静默门
    // 把 om 设为 profile 默认 operation_mode（resolve 在 contact override=None 时回落它）
    profile.operation_mode = om;
    // global_enabled=true 但 profile 级关 → 应返回 false
    assert!(!effective_quiet_hours_enabled(&contact, &profile, true));

    // 对照: profile 级 None + contact None → 回落 global(字节等价)
    let default_profile = DomainProfile::default();
    assert!(effective_quiet_hours_enabled(&contact, &default_profile, true), "DEFAULT 回落 global=true(字节等价)");
}
```
（先核 `DomainProfile::default().operation_mode.quiet_hours.enabled_override` 是否 None——应是，QuietHoursMode::default() :1764 = None。Contact 构造复用现有测试 helper 或 `Contact::default()` 若有。）

- [ ] **Step 2: 跑测试确认失败（编译失败=签名不符）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib quiet_hours_honors_profile 2>&1 | tail`
Expected: 编译失败（旧签名 2 参）。

- [ ] **Step 3: 改签名走 resolve_operation_mode**

`effective_quiet_hours_enabled`（:94）改：
```rust
pub(crate) fn effective_quiet_hours_enabled(
    contact: &crate::models::Contact,
    profile: &crate::models::DomainProfile,
    global_enabled: bool,
) -> bool {
    crate::planner::resolve_operation_mode(contact, profile)
        .quiet_hours
        .enabled_override
        .unwrap_or(global_enabled)
}
```
更新 doc 注释：从「只读 contact override」改为「经 resolve_operation_mode 三级链（contact override → per_relationship → profile 默认），与其余 6 驱动力一致；DEFAULT 全 None → global，字节等价」。

- [ ] **Step 4: 改两消费点传 profile**

gateway.rs:2068：传该上下文已加载的 profile（gateway 主链早已 `load_active_domain_profile`，找到那个变量名，如 `active_profile`）。webhooks.rs:565：同——先读 :550-570 确认 profile 是否已加载；若未加载需 `load_active_domain_profile(&state.db, &contact.workspace_id).await` 取（webhooks 静默门判定处）。先 grep 两处上下文确认 profile 变量可达，不可达则就近加载。

- [ ] **Step 5: 跑测试 + 基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: 新测试 PASS，lib ≥350/0，0 警告。既有 quiet_hours 测试（:116+）仍绿。

- [ ] **Step 6: Commit**

```bash
git add src/agent/quiet_hours.rs src/agent/gateway.rs src/webhooks.rs
git commit -m "fix(universal/G04): effective_quiet_hours_enabled 经 resolve_operation_mode 读 profile/per_relationship 级(quiet_hours 不再是运行时死字段)"
```

**红线**：DEFAULT contact override None + profile None → resolve 回落 global_enabled，字节等价。

---

## Task 9: G08 + G32 — camelCase 归一 data-loss（可缓 Low）

**Files:**
- Modify: `src/routes/guide_profile.rs:88-103`（`coerce_scalar_string_fields` 扩到嵌套 profileDimensions[].description=G32）
- Modify: `src/models.rs:2031-2040`（BusinessFormula 加 serde alias=G08）
- Test: `src/routes/guide_profile.rs`（同文件 `#[cfg(test)]` mod）

**根因（已亲核）**：`normalize_json_keys`（guide_profile.rs:61）递归把所有 key snake 化。`BusinessFormula`（models.rs:2031，`#[serde(rename_all="camelCase")]`）的 `displayName` 被转成 `display_name` → serde 期望 camelCase wire key `displayName` → 反序列化匹配不上 → 因 `#[serde(default)]`（:2039）静默落空串（G08，伤 admin 生成的候选 profile，display_name 运行时无消费纯 cosmetic，但仍是 data-loss）。`coerce_scalar_string_fields`（:88）只护顶层 `description`/`prompt_fragment`，嵌套 `profileDimensions[].description`（ProfileDimension.description models.rs:1861 是 `String` 非 Option）若 LLM 给成对象/数组会反序列化失败（G32）。

**方案**：G08 用 serde alias（比 stateMachine 式抽取简单，businessFormulas 是普通 serde 字段无需绕 normalize）；G32 扩 coerce 到嵌套。

- [ ] **Step 1: 写失败测试**

guide_profile.rs 测试 mod 加（复用 normalize+coerce+to_document+from_document 往返）：
```rust
#[test]
fn business_formula_display_name_survives_normalize() {
    use serde_json::json;
    let generated = json!({
        "profileDimensions": [{"kind":"trust","displayName":"信任","description":"x"}],
        "businessFormulas": [{"key":"trust","expression":"A×B","displayName":"信任度"}]
    });
    let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
    let doc = mongodb::bson::to_document(&normalized).unwrap();
    let profile: crate::models::DomainProfile = mongodb::bson::from_document(doc).unwrap();
    assert_eq!(profile.business_formulas[0].display_name, "信任度", "displayName 经 normalize 后不应丢(G08)");
}

#[test]
fn profile_dimension_description_object_coerced() {
    use serde_json::json;
    // LLM 把嵌套 description 给成对象 → coerce 应压平成 JSON 文本,不致 from_document 失败
    let generated = json!({
        "profileDimensions": [{"kind":"stage","displayName":"阶段","description":{"a":"b"}}]
    });
    let normalized = coerce_scalar_string_fields(normalize_json_keys(generated));
    let doc = mongodb::bson::to_document(&normalized).unwrap();
    let profile: crate::models::DomainProfile = mongodb::bson::from_document(doc).expect("嵌套 description 对象应被 coerce 压平(G32)");
    assert!(!profile.profile_dimensions[0].description.is_empty());
}
```
（先核 DomainProfile 的 `business_formulas`/`profile_dimensions` 字段名 + ProfileDimension 字段；确认 from_document 对缺失 business_formulas 不 panic——有 `#[serde(default)]`。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib business_formula_display_name_survives 2>&1 | tail`
Expected: FAIL（display_name 空串）；第二个测试 panic（from_document 对 description 对象失败）。

- [ ] **Step 3: G08 — BusinessFormula 加 serde alias**

models.rs:2039 `display_name` 字段加 alias 接受 snake 化后的 key：
```rust
#[serde(default, alias = "display_name")]
pub display_name: String,
```
（`rename_all="camelCase"` 让 wire key 是 `displayName`，alias 额外接受 `display_name`——normalize 后的 key。key/expression 恰好 snake≡camel 不丢，无需 alias，但为稳健可一并加 `#[serde(alias=...)]`——仅在 snake≠camel 字段必要，故只 display_name 必须。）

- [ ] **Step 4: G32 — coerce 扩到嵌套 profileDimensions[].description**

`coerce_scalar_string_fields`（:88）在处理完顶层 SCALAR_STRING_KEYS 后，加对 `profile_dimensions` 数组每元素的 `description` 做同样压平（注意此时已 normalize，key 是 snake `profile_dimensions`/`description`）：
```rust
// G32: profileDimensions[].description 是 String(models.rs ProfileDimension),LLM 偶发给对象 → 压平。
if let Some(Value::Array(dims)) = map.get_mut("profile_dimensions") {
    for dim in dims.iter_mut() {
        if let Value::Object(dim_map) = dim {
            if let Some(d) = dim_map.get_mut("description") {
                if d.is_object() || d.is_array() {
                    *d = Value::String(serde_json::to_string(d).unwrap_or_default());
                }
            }
        }
    }
}
```
更新 :80-87 doc 注释：补「G32：嵌套 profileDimensions[].description 同样压平」。

- [ ] **Step 5: 跑测试 + 基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: 两新测试 PASS，lib ≥350/0，0 警告。

- [ ] **Step 6: Commit**

```bash
git add src/routes/guide_profile.rs src/models.rs
git commit -m "fix(universal/G08+G32): BusinessFormula.displayName 加 serde alias 防 normalize 丢值 + coerce 扩到嵌套 profileDimensions[].description"
```

**红线**：serde alias 向后兼容（老库 camelCase 仍认，新增 snake alias 不破坏）；DEFAULT 走代码 seed 不经此路由，零回归。

---

## Task 10: G16 — taxonomy 候选双 upsert 去重（可缓 Low）

**Files:**
- Modify: `src/agent/gateway.rs:1043-1055`（去 gateway 路径的候选 upsert，或确认两路径职责后单点保留）
- Test: 靠 lib 编译 + 现有 taxonomy 候选测试

**根因（已亲核）**：同 `(scope, kind, raw_value)` 候选在单 run 内被双 upsert → occurrences ~2× 膨胀（仅 Approved run、软门、不改相对排序，故 Low）。两 upsert 不同步：
- decision 路径：`validate_and_normalize_decision`（decision_taxonomy.rs:84）内 `upsert_candidate(..., None, 0)`（:112，confidence=0，pre-review 决策）。decision.rs:711 调用。
- gateway 路径：`taxonomy_upsert_candidate(..., Some("user-ops decision path"), 50)`（gateway.rs:1044，confidence=50，post-review final_decision，:1043 遍历 `outcome.candidate_writes`）。

**关键（Explore 已纠正初判）**：不能简单删任一个——两者捕获不同决策态（pre-review confidence-0 vs post-review final_decision confidence-50），且 final_decision 是经 review 矫正后的权威值。简单删 gateway 会丢 final_decision 的候选；简单删 decision 会丢 pre-review 信号。**正解**：确认 final_decision 路径（gateway）是权威——若 decision.rs:711 的 validate 已对同一 decision 跑过 upsert，且 gateway 用的是**同一份经矫正的 decision** 再 upsert 一次，则 gateway 的是 final 权威值，decision 路径的 pre-review upsert 冗余。**先核**：gateway.rs:1043 的 `outcome.candidate_writes` 与 decision.rs:711 validate 处理的是否同一 decision 的同一批 kind/raw。

- [ ] **Step 1: 核实两路径的 candidate_writes 来源**

读 gateway.rs:3690-3730（`candidate_writes` 如何 build，:3695 字段定义 + :3724 填充逻辑）+ decision.rs:700-720（validate_and_normalize_decision 调用上下文）。确认：
- 若 gateway 的 candidate_writes 来自 final_decision（review 后），decision 的 upsert 来自同一 decision pre-review → 同 (scope,kind,raw) 双写，gateway 权威。
- 若两者 raw_value 可能不同（review 改写过 stage）→ 不是重复，是两个不同候选，**不可删**（此时 G16 证伪，记报告说明）。

- [ ] **Step 2: 据核实结果决定改法**

**情况 A（确认重复，同 scope/kind/raw）**：去掉 decision.rs:711 路径的 pre-review upsert（保留 gateway 的 final 权威 upsert），或反之保留信息更全的一方。优先保留 gateway（confidence=50 + final_decision 权威 + 已有 source 标注 "user-ops decision path"）。decision_taxonomy.rs 的 validate 仍做校验/normalize（那是它的主职责），只是不再 upsert 候选（把 :112 的 upsert_candidate 调用移除，校验逻辑保留）。

**情况 B（raw 可能不同，非重复）**：不改代码，在报告记 G16 PARTIAL-REFUTED（双写捕获不同决策态，非冗余）。

- [ ] **Step 3: 若改代码——跑基线**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib ≥350/0，0 警告。现有 candidate_writes 测试（gateway.rs:3966-4105）仍绿。

- [ ] **Step 4: Commit（仅情况 A）**

```bash
git add src/agent/gateway.rs src/agent/decision_taxonomy.rs
git commit -m "fix(universal/G16): 去 taxonomy 候选双 upsert(保留 final_decision 权威单点,消 occurrences 2x 膨胀)"
```

**红线**：软门不改判决；occurrences 是观测计数，去重不影响相对排序。**这是审查中唯一需先核实再决定改不改的任务**——执行 agent 必须先做 Step 1 核实，证伪则记报告不强改。

---

## Task 11: G24 — 出站正文运行期 fail-closed 禁词校验（可缓 Low，纵深加固红线⑤）

**Files:**
- Modify: `src/agent/gateway.rs:969+`（Approved 分支出站前加 `passes_forbidden_words` 运行期校验）
- Test: `src/agent/gateway.rs`（同文件 `#[cfg(test)]` mod）

**根因（已亲核）**：终审红线⑤判 PASS（运行时）/ RISK（纵深）：出站正文唯一守卫是 relay 专用 `leaks_internal_payload`，`passes_forbidden_words`（evolution/lint.rs:33，扫"人工接管/转人工/takeover/hand-off"等）只在 CI 静态扫 diff，不覆盖 profile override 注入的文本经 LLM 流入回复正文的路径。boundary_protection prompt 是唯一运行期防线。加一道运行期 fail-closed：Approved 的 final_decision.reply_text 出站前过 `passes_forbidden_words`，命中禁词则降级 Held（不发），与红线⑤ AI 内部状态名一致。

**Interfaces:**
- Consumes: `crate::evolution::lint::passes_forbidden_words(&str) -> bool`（lint.rs:33，pub，true=干净）。

- [ ] **Step 1: 写失败测试**

gateway.rs 测试 mod 加纯函数级测试——抽一个判定 helper（不测整个 gateway DB 流）：
```rust
/// G24: 出站正文运行期 fail-closed——命中禁词则不放行(降级 Held)。纯函数便于单测。
fn reply_text_passes_runtime_redline(reply_text: &str) -> bool {
    crate::evolution::lint::passes_forbidden_words(reply_text)
}

#[test]
fn outbound_reply_blocks_forbidden_words_at_runtime() {
    assert!(reply_text_passes_runtime_redline("您好,这边帮您看下订单"), "正常正文放行");
    assert!(!reply_text_passes_runtime_redline("这个我帮您转人工处理"), "含'转人工'必拦(红线⑤运行期 fail-closed)");
}
```
（先核 lint.rs FORBIDDEN_LITERALS_LOWER 是否含"转人工"——CLAUDE.md 词表含 `人工接管|takeover|hand-off|人工介入|人工`，确认 lint.rs 与 check-no-human-takeover.sh 同词表。若 lint.rs 词表不同，测试用其真实命中的词。）

- [ ] **Step 2: 跑测试确认（helper 未接线时编译/逻辑确认）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo test --lib outbound_reply_blocks_forbidden_words 2>&1 | tail`
Expected: PASS（lint 函数已存在，测试验证它对正文生效）。

- [ ] **Step 3: Approved 分支接线 fail-closed**

gateway.rs:969 `if matches!(finalize_status, GatewayStatusFinal::Approved)` 块内（state_action_policy 校验之后、taxonomy 软闸之前或之后，但必须在 outbox enqueue 之前），加：
```rust
// G24: 出站正文运行期 fail-closed 红线⑤校验。CI lint 只扫静态 diff,profile override 注入文本
// 经 LLM 流入正文不在其覆盖面;此处对 final reply_text 做运行期禁词校验,命中则降级 Held 不发。
if matches!(finalize_status, GatewayStatusFinal::Approved)
    && !crate::evolution::lint::passes_forbidden_words(&final_decision.reply_text)
{
    review.approved = false;
    review.final_review_status = "blocked_by_safety_guard".to_string();
    final_decision.should_reply = false;
    final_decision.autonomy_mode = "blocked".to_string();
    if !review.risks.iter().any(|r| r == "outbound_redline_forbidden_word") {
        review.risks.push("outbound_redline_forbidden_word".to_string());
    }
    finalize_status = GatewayStatusFinal::Held("blocked_by_safety_guard".to_string());
    write_event_for_account(
        state, &contact.account_id, Some(&contact.wxid),
        "outbound_redline_blocked", "blocked",
        "出站正文命中红线禁词,降级 Held 不发(运行期 fail-closed)",
        Some(doc! { "run_id": &run_id }),
    ).await?;
}
```
（先核 `GatewayStatusFinal::Held` 构造 + `blocked_by_safety_guard` 是否闭集合法 status（CLAUDE.md 提到 `blocked_by_safety_guard` 是 AI 内部状态名，应合法）。确认 write_event 签名与 :988 一致。放置点须在 enqueue（搜 outbox_enqueue 调用）之前。）

- [ ] **Step 4: 跑测试 + 基线 + 红线门**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4 && bash scripts/check-no-human-takeover.sh && echo REDLINE_OK`
Expected: lib ≥350/0，0 警告，REDLINE_OK（新增代码用 AI 内部状态名，不含禁词字面量）。

- [ ] **Step 5: Commit**

```bash
git add src/agent/gateway.rs
git commit -m "fix(universal/G24): 出站正文运行期 fail-closed 禁词校验(纵深加固红线⑤,补静态 lint 覆盖不到的 profile override→正文路径)"
```

**红线**：fail-closed 方向安全（命中降级 Held 不发，非放过）；DEFAULT 正常正文不含禁词，零拦截、零回归。

---

## Task 12: G09 — 确定性派生断言拆纯函数版进 baseline 硬门（可缓 Low）

**Files:**
- Create/Modify: 在 `src/` 对应模块加 `#[cfg(test)]` 纯函数测试（迁移 c2/domain_profile_e2e 的 DB 无关断言）
- 保留：`tests/c2_*.rs` / `tests/domain_profile_e2e.rs` 原 `#[ignore]` 集成测试不动（留 CI）

**根因（已亲核 D13 报告）**：baseline 门（check-baseline.sh）只跑 `cargo test --lib` + 4 固定 PBT，不含 c2_state_transition_cross_domain / domain_profile_e2e；这俩在 integration job 跑但 `continue-on-error:true` 红了不拦。核心迁移命门 `check_state_transition` 已被 state_transition_pbt 守住（终审确认），缺口仅在 e2e 接线 + publish/realign 灰度层。**最小动作**：把 c2/domain_profile_e2e 里**不依赖 Docker/DB** 的纯逻辑断言（如状态机派生优先级、policy 派生列表、版本号分配纯函数）拆成 lib 内 `#[cfg(test)]` 测试进 baseline。**不**改 check-baseline.sh 阈值机制，靠 `cargo test --lib` 自动纳入。

- [ ] **Step 1: 盘点可拆的纯函数断言**

读 `tests/c2_state_transition_cross_domain.rs:1-187`（已知 0 ignore、6 test、纯函数跨域状态机）。确认其断言调的是哪些纯函数（如 `check_state_transition` / `derive_*`）。这些函数若在 `src/` 内有 `pub(crate)` 入口，则直接在该 src 模块的 test mod 加等价断言（不经 tests/ 集成层）。读 `tests/domain_profile_e2e.rs` 找 Part A/C 里**不连 testcontainers** 的纯断言（如 risky_fields/派生列表）。

- [ ] **Step 2: 迁移纯函数断言到 lib test mod**

对每个可拆断言，在对应 src 模块（如 guards.rs 的 check_state_transition test mod / migrations m013 的 derive_state_policy_lists test mod / admin_ops_versions.rs 的 next_version test mod）加等价 `#[test]`。**增量叠加，不删原集成测试**（红线：feedback_additive_tests）。例：c2 跨域状态机断言 → guards.rs test mod 加同构纯函数断言（构造 state machine doc + 调 check_state_transition + 断言 Ok/Err）。先确认这些断言尚未被 state_transition_pbt 覆盖（避免重复）——终审说 check_state_transition 已进 PBT，故**只拆 PBT 未覆盖的派生/版本/优先级逻辑**。

- [ ] **Step 3: 跑 baseline 确认新测试纳入**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib 计数较修复前增加（新拆的纯函数测试），0 failed，0 警告。

- [ ] **Step 4: Commit**

```bash
git add <修改的 src 模块文件>
git commit -m "test(universal/G09): c2/domain_profile_e2e 的纯函数派生断言拆进 lib baseline 硬门(PBT 未覆盖的版本/policy 派生层)"
```

**说明**：这是测试增量任务，不改生产代码。原 #[ignore] 集成测试保留留 CI 跑完整 DB 路径。若盘点发现 c2/e2e 断言全是 DB 依赖型（无可拆纯函数），则记报告说明「确定性命门已被 state_transition_pbt 充分覆盖，e2e 是 DB 接线层无纯函数可拆」，不强拆。

---

## Task 13: G10 — redline job 补 Require ROLEPLAYER_API_KEY（可缓 Low，CI 配置）

**Files:**
- Modify: `.github/workflows/ci.yml:1026-1095`（real-llm-redline job 加 Require ROLEPLAYER_API_KEY 步骤）

**根因（已亲核 D13 报告）**：`real-llm-redline` job（ci.yml:1026-1095）只有 `Require REAL_LLM_API_KEY`（:1046），无 `Require ROLEPLAYER_API_KEY`，而 roleplay-arc job（ci.yml:963-967）有。digital_twin arc 的 roleplayer 走 ROLEPLAYER_API_KEY（映射 secrets.NVIDIA_KEY），缺则 turn-0 静默 skip → 数字分身真模型轴可能假绿。补一道 Require 闸与 roleplay-arc 对齐。

- [ ] **Step 1: 读两 job 的 Require 步骤模式**

读 ci.yml:963-967（roleplay-arc 的 `Require ROLEPLAYER_API_KEY` 步骤写法）+ ci.yml:1040-1050（redline job 的 `Require REAL_LLM_API_KEY` 步骤写法）。确认 step 结构（`if`/`run` 检查 env 非空否则 exit 1）。

- [ ] **Step 2: 在 redline job 补 Require 步骤**

在 redline job（:1046 `Require REAL_LLM_API_KEY` 之后）按 roleplay-arc:963-967 同款加：
```yaml
      - name: Require ROLEPLAYER_API_KEY
        run: |
          if [ -z "${ROLEPLAYER_API_KEY}" ]; then
            echo "ROLEPLAYER_API_KEY 未配置——数字分身 redline arc 会静默 skip,视为配置错误" >&2
            exit 1
          fi
        env:
          ROLEPLAYER_API_KEY: ${{ secrets.NVIDIA_KEY }}
```
（以 roleplay-arc:963-967 真实写法为准逐字对齐；若该 job 用不同 env 名/secret 映射，照搬其真实形态。）

- [ ] **Step 3: YAML 语法校验**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && python -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml',encoding='utf-8')); print('YAML_OK')" 2>&1 | tail -3`
Expected: YAML_OK（无语法错）。

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(universal/G10): real-llm-redline job 补 Require ROLEPLAYER_API_KEY(与 roleplay-arc 对齐,防数字分身轴静默 skip 假绿)"
```

**说明**：纯 CI 配置。坐实 NVIDIA_KEY 是否仍配（终审说大概率已配）属运维核查，不在本任务——本任务只补闸，缺 key 时 fail-loud 而非假绿。

---

## Task 14: G18 + G05 — 纯文档/注释订正（可缓 Low，零代码风险）

**Files:**
- Modify: `src/agent/decision.rs:428`（G18「字节等价」→「内容/语义等价」）
- Modify: `src/agent/domain_profile.rs:910,921`（G05 删/订正「口吻最像本人」不可达过度承诺注释）

**根因（已亲核）**：
- **G18**：decision.rs:428 注释称经营公式段处理后「prompt 字节等价」，但实际公式段从 policy 中部移到末尾（内容保留、位置变），措辞过宽。应改「内容/语义等价（段从中部移末尾）」。
- **G05**：domain_profile.rs:910「friend=…口吻最像本人」、:921「朋友：…口吻最像本人」是数字分身样例注释，但 OperationMode 零 voice/tone/soul 字段，decision.rs soul 注入不读 relationship_type → 注释承诺的「口吻分化」当前代码不可达（终审 CONFIRMED）。订正为实情 + 指明是独立专题。

- [ ] **Step 1: G18 注释订正**

decision.rs:428 把「→ prompt 字节等价、销售域零变化」改为「→ 内容/语义等价（经营公式段从 policy 中部移至末尾，内容逐字保留、仅位置变；LLM 见到的内容完全一致，零运行时影响）、销售域零行为变化」。沿用 domain_profile.rs 已有的语义等价豁免措辞口径。

- [ ] **Step 2: G05 注释订正**

domain_profile.rs:910 把「friend=漏斗关、只留日历关怀、口吻最像本人」改为「friend=漏斗关、只留日历关怀（**注**：口吻分化即 per_relationship 各异 soul/tone 当前未实现——OperationMode 无 voice/tone/soul 字段、decision.rs soul 注入不读 relationship_type；数字分身口吻分化是独立专题，见审查终审报告第 5 节）」。:921「朋友：…口吻最像本人」同样删掉「口吻最像本人」改为指明未实现。

- [ ] **Step 3: 编译确认（仅注释，应零影响）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_INCREMENTAL=0 cargo build --lib 2>&1 | tail -3`
Expected: 编译过（纯注释改动）。

- [ ] **Step 4: Commit**

```bash
git add src/agent/decision.rs src/agent/domain_profile.rs
git commit -m "docs(universal/G18+G05): 订正注释——公式段'字节等价'改'内容/语义等价'+ 删数字分身'口吻最像本人'不可达过度承诺(指明独立专题)"
```

**红线**：纯注释，零代码/测试改动，零运行时影响。

---

## 收尾：全批 merge gate + 提交边界

全部 14 任务完成后（**不含 G02——终审列必修但本批用户范围是「4 必修」即 G21/G13/G06/G01，G02 归可缓但 spec 未展开为独立 task；若执行时有余力可补，否则记路线图**）：

- [ ] **完整 merge gate**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && rm -rf target/debug/incremental && RUSTFLAGS="-Dwarnings" CARGO_INCREMENTAL=0 cargo test --lib 2>&1 | tail -4`
Expected: lib ≥350/0，0 警告。
四 PBT：`for t in state_transition_pbt memory_card_invariants wiki_chunk_revision_pbt llm_retry_jitter; do CARGO_INCREMENTAL=0 cargo test --test $t 2>&1 | tail -2; done`，累计 ≥33/0。
红线门：`bash scripts/check-no-human-takeover.sh && echo CLEAN`。

- [ ] **提交边界自查**

`git status` 确认只改了本批命名文件。**绝不 `git add -A`/`.`**。排除并行会话产物（tests/real_llm_*、tests/roleplay_*、tests/common/*、.kiro/specs/universal-test-coverage/*、AGENTS.md、agent_t*.txt、t15_single.txt、docs/superpowers/plans/2026-06-18-*）。

- [ ] **推送/合并需用户显式授权**——本计划只到本地 commit；推送 main 前停下问用户。
