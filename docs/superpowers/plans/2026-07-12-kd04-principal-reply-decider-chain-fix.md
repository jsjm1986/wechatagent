# KD-04 修复实施计划：领导微信回复识别改用 decider_chain

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。Steps 用 checkbox 跟踪。**红线：改任何代码前必 100% 读懂相关代码，引用必当场 Read/Grep 亲验 file:line，不猜。**

**Goal:** 让领导用 `decider_chain`（推荐配置）时的微信回复能被正确识别为裁决，修复 KD-04（深度审查批 D 唯一 High）。

**Architecture:** 抽纯谓词 `is_decider_for_config`（复用权威解析器 resolve_ask_human_policy，已内含旧 principal_decider 回落）；重写 `lookup_principal_config` 从只查旧标量字段改为遍历 current_version 配置调纯谓词。TDD：先写失败测试证伪旧逻辑 → 抽纯函数 → 重写 DB 壳 → 全绿。

**Tech Stack:** Rust 2021 / Axum / MongoDB。`cargo test --lib` 跑单测（无需 Docker）。

## Global Constraints
- **改前必 100% 读懂 + 引用必亲验 file:line**（CLAUDE.md 最高红线）。
- **只改 2 文件**：`src/agent/escalation/policy.rs`（+纯谓词+5 测试）、`src/agent/escalation/ledger.rs`（重写 lookup_principal_config）。**不改** webhooks.rs:443 调用点 / handle_principal_reply / put_ask_human_policy / principal_decider 字段。
- **返回类型不变**：lookup_principal_config 仍 `AppResult<Option<String>>`（单一调用者 webhooks.rs:443 只用 `.is_some()`）。
- **baseline 不回退**：`cargo test --lib` ≥ 350 passed / 0 failed（scripts/check-baseline）。新增测试只增不减。
- **不改**：前端 / API 契约 / 配置结构 / .env / 迁移——纯后端逻辑修复。
- **no-human-takeover lint**：src/agent/ 新增行不得含 人工接管/takeover/hand-off/人工介入/人工托管/接管/人工（本修复无这些词，注释用「决策人/领导/请示通道」等既有措辞）。
- 设计文档：`docs/superpowers/specs/2026-07-12-kd04-principal-reply-decider-chain-fix-design.md`。

## 亲验的现有代码事实（实现者仍须自己 Read 确认）
- `resolve_ask_human_policy(config: &OperationDomainConfig) -> ResolvedAskHumanPolicy`（policy.rs:21）：ask_human_policy 存在取其 decider_chain；None 时回落 `config.principal_decider.map(|w| vec![DeciderRef{wxid:w,display_name:None}]).unwrap_or_default()`（policy.rs:36-40）。`pub(crate)`。
- `ResolvedAskHumanPolicy.decider_chain: Vec<DeciderRef>`（policy.rs:9）。
- `DeciderRef { wxid: String, display_name: Option<String> }`（models.rs:1118）。
- policy.rs 已有 `#[cfg(test)] mod tests`（:108），内有 `base_config() -> OperationDomainConfig` helper（:113-138，principal_decider=None、ask_human_policy=None、其余字段全设），`use crate::models::{AskHumanPolicy, OperationDomainConfig};`（:111），且 `use super::*;`（:110，故 DeciderRef/resolve_ask_human_policy 在 tests 内可见）。
- `AskHumanPolicy` 完整构造范式见 policy.rs:167-177（decider_chain/escalate_*/dedupe_window_hours/daily_push_cap/quiet_hours/timeout_hours 全字段）。
- 现 `lookup_principal_config`（ledger.rs:215-233）：`find_one(doc!{"workspace_id","principal_decider":from_wxid,"current_version":true})` → `Ok(cfg.map(|c| c.domain))`。
- 现 ledger.rs 顶部 use：实现者 Read ledger.rs:1-20 确认 `futures::TryStreamExt` 是否已导入（scan_escalation_timeouts 用了 `use futures::TryStreamExt;` 函数内局部导入，mod.rs:359）。

---

## Task 1: 抽纯谓词 is_decider_for_config + 5 个确定性单测（TDD）

**Files:**
- Modify: `src/agent/escalation/policy.rs`（新增 pub(crate) fn is_decider_for_config，在 resolve_ask_human_policy 之后、in_quiet_hours 之前，约 policy.rs:52 后）
- Test: `src/agent/escalation/policy.rs` 内 `#[cfg(test)] mod tests`（:108，复用 base_config helper）

**Interfaces:**
- Consumes: `resolve_ask_human_policy(&OperationDomainConfig) -> ResolvedAskHumanPolicy`（policy.rs:21，已存在）、`DeciderRef{wxid,display_name}`、`AskHumanPolicy`（已在 tests 导入）。
- Produces: `pub(crate) fn is_decider_for_config(cfg: &OperationDomainConfig, from_wxid: &str) -> bool`（Task 2 的 lookup_principal_config 调用）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/agent/escalation/policy.rs:1-60`（resolve_ask_human_policy 全貌 + 回落逻辑）+ `:108-197`（tests mod + base_config helper + AskHumanPolicy 构造范式）。确认：base_config() 返回 principal_decider=None/ask_human_policy=None 的 config；tests 内 `use super::*` 使 resolve_ask_human_policy/DeciderRef 可见；AskHumanPolicy 全字段。**说不清就继续读，不动手。**

- [ ] **Step 2: 写 5 个失败测试**

在 policy.rs 的 `#[cfg(test)] mod tests` 内（base_config 之后、现有 #[test] 之间任意位置）追加：

```rust
    // ── KD-04 修复：is_decider_for_config 纯谓词 ──
    #[test]
    fn kd04_decider_chain_member_recognized() {
        // KD-04 复现+修复：只配 decider_chain（推荐配置）、principal_decider=None。
        // 旧逻辑只认 principal_decider → 领导 wxid 不被识别；新谓词应识别。
        let mut cfg = base_config();
        cfg.ask_human_policy = Some(AskHumanPolicy {
            decider_chain: vec![DeciderRef { wxid: "leader1".into(), display_name: None }],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: None,
        });
        assert!(is_decider_for_config(&cfg, "leader1"), "decider_chain 成员必须被识别为决策人");
        assert!(cfg.principal_decider.is_none(), "本用例前提：principal_decider=None（推荐配置）");
    }

    #[test]
    fn kd04_non_first_decider_recognized() {
        // 覆盖改派 next：链中非首位决策人回复也须被识别。
        let mut cfg = base_config();
        cfg.ask_human_policy = Some(AskHumanPolicy {
            decider_chain: vec![
                DeciderRef { wxid: "leader1".into(), display_name: None },
                DeciderRef { wxid: "leader2".into(), display_name: None },
            ],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: None,
        });
        assert!(is_decider_for_config(&cfg, "leader2"), "链中非首位（改派 next）也须被识别");
    }

    #[test]
    fn kd04_legacy_principal_decider_still_recognized() {
        // 旧配置兼容：只设 principal_decider、ask_human_policy=None → resolve 回落 → 识别。
        let mut cfg = base_config();
        cfg.principal_decider = Some("oldboss".into());
        assert!(is_decider_for_config(&cfg, "oldboss"), "旧 principal_decider 经 resolve 回落仍须识别");
    }

    #[test]
    fn kd04_non_decider_returns_false() {
        let mut cfg = base_config();
        cfg.ask_human_policy = Some(AskHumanPolicy {
            decider_chain: vec![DeciderRef { wxid: "leader1".into(), display_name: None }],
            escalate_safety_guard: true,
            escalate_unverified_product: true,
            escalate_ai_policy_hold: false,
            escalate_stuck: true,
            dedupe_window_hours: None,
            daily_push_cap: None,
            quiet_hours: None,
            timeout_hours: None,
        });
        assert!(!is_decider_for_config(&cfg, "stranger"), "非决策人不得被识别");
    }

    #[test]
    fn kd04_empty_chain_returns_false() {
        // 未启用请示通道（decider_chain 空 + principal_decider None）→ 任何 wxid 都不是决策人。
        let cfg = base_config();
        assert!(!is_decider_for_config(&cfg, "anyone"), "未启用请示通道时任何 wxid 都非决策人");
    }
```

- [ ] **Step 3: 跑测试确认失败（编译失败）**

Run: `cargo test --lib -p wechatagent kd04 2>&1 | tail -20`（或 `cargo test --lib kd04`）
Expected: 编译错误 `cannot find function is_decider_for_config`（函数未定义）——这就是"失败"的形态（TDD red）。

- [ ] **Step 4: 实现纯谓词**

在 policy.rs 的 `resolve_ask_human_policy` 函数结束后（约 :52 后、`in_quiet_hours` 之前）插入：

```rust
/// KD-04：from_wxid 是否是该 config 解析后 decider_chain 的成员。
/// 复用 resolve_ask_human_policy（已内含旧 principal_decider 回落），故新旧配置都覆盖，
/// 且覆盖链中全部决策人（含改派后的 next 决策人）。纯函数、无 IO。
pub(crate) fn is_decider_for_config(config: &OperationDomainConfig, from_wxid: &str) -> bool {
    resolve_ask_human_policy(config)
        .decider_chain
        .iter()
        .any(|d| d.wxid == from_wxid)
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --lib kd04 2>&1 | tail -20`
Expected: 5 个 kd04_* 测试全 PASS。

- [ ] **Step 6: Commit**

```bash
git add src/agent/escalation/policy.rs
git commit -m "fix(escalation): 抽 is_decider_for_config 纯谓词识别 decider_chain 成员(KD-04)

复用 resolve_ask_human_policy(已含旧 principal_decider 回落),覆盖链中全部决策人+改派 next。
5 确定性 lib 单测:decider_chain命中/链中非首位/旧配置兼容/非决策人false/空链false。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: 重写 lookup_principal_config 用纯谓词遍历 current_version 配置

**Files:**
- Modify: `src/agent/escalation/ledger.rs:215-233`（lookup_principal_config 函数体）

**Interfaces:**
- Consumes: `crate::agent::escalation::policy::is_decider_for_config(&OperationDomainConfig, &str) -> bool`（Task 1 产出）。
- Produces: `lookup_principal_config(state, workspace_id, from_wxid) -> AppResult<Option<String>>`（签名不变，webhooks.rs:443 调用点不改）。

- [ ] **Step 1: 先读懂（红线）**

Read `src/agent/escalation/ledger.rs:1-20`（确认顶部 use：doc!/AppResult/是否已导入 TryStreamExt）+ `:215-233`（现 lookup_principal_config）+ `src/agent/escalation/mod.rs:358-375`（scan_escalation_timeouts 的 `.find(doc!{"current_version":true})` + `use futures::TryStreamExt;` 遍历范式）。确认 is_decider_for_config 从 ledger.rs 的可见路径（policy 是 escalation 子模块，`crate::agent::escalation::policy::is_decider_for_config` 全路径，pub(crate) 可达）。**说不清就继续读。**

- [ ] **Step 2: 重写函数体**

把 ledger.rs:215-233 的 `lookup_principal_config` 整体替换为（保留 doc 注释更新）：

```rust
/// KD-04：判断 from_wxid 是否为本 workspace 任一 current_version 域配置的决策人
/// （解析后的 decider_chain 成员，含旧 principal_decider 回落）。返回 Some(domain) 表示
/// 是决策人（domain 供调用方观测，webhooks 仅用 is_some 分流）；None 表示非决策人。
/// 从只查旧标量 principal_decider 改为复用 resolve_ask_human_policy——修复推荐配置
/// （只配 decider_chain）下领导回复不被识别的缺陷。
pub(crate) async fn lookup_principal_config(
    state: &AppState,
    workspace_id: &str,
    from_wxid: &str,
) -> AppResult<Option<String>> {
    use futures::TryStreamExt;
    let mut cursor = state
        .db
        .operation_domain_configs()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "current_version": true,
            },
            None,
        )
        .await?;
    while let Some(cfg) = cursor.try_next().await? {
        if crate::agent::escalation::policy::is_decider_for_config(&cfg, from_wxid) {
            return Ok(Some(cfg.domain));
        }
    }
    Ok(None)
}
```

注意：`use futures::TryStreamExt;` 放函数内局部导入（与 scan_escalation_timeouts mod.rs:359 同款）；若 Step 1 发现 ledger.rs 顶部已有该 use，则删掉此局部 use 避免 unused/重复告警。

- [ ] **Step 3: cargo check 确认编译**

Run: `cargo check --lib 2>&1 | tail -20`
Expected: 0 error（可能有 is_decider_for_config 若可见性不对的报错→按报错在 mod.rs 补 pub(crate) 或修全路径）。

- [ ] **Step 4: 跑既有 escalation 单测确认不回退**

Run: `cargo test --lib escalation 2>&1 | tail -20`
Expected: escalation 模块所有既有测试 + Task 1 的 5 个 kd04 测试全 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/agent/escalation/ledger.rs
git commit -m "fix(escalation): lookup_principal_config 改遍历 current_version 配置调 is_decider_for_config(KD-04)

从 find_one(principal_decider) 改 find(current_version)+纯谓词,识别 decider_chain 全部决策人。
签名 Option<String> 不变,webhooks.rs:443 调用点不改。根治推荐配置下领导微信回复掉进客户链路。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: 全量 --lib baseline 验证 + 无回退确认

**Files:** 无改动（纯验证）

- [ ] **Step 1: 全量 lib 测试**

Run: `cargo test --lib 2>&1 | tail -15`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350（baseline 不回退，且比修复前 +5）。

- [ ] **Step 2: 若有 failed 立即修**

若出现 failed：读失败用例名（不甩锅既存 flaky——feedback_dont_blame_preexisting_flaky），核对是否本修复引入。是则修，非则拉日志核对。不假绿。

- [ ] **Step 3: no-human-takeover lint 自检**

Run: `git diff origin/main -- src/agent/ | grep -nE "人工接管|takeover|hand.?off|人工介入|人工托管|接管|人工" || echo "lint clean"`
Expected: `lint clean`（本修复新增行用「决策人/领导/请示通道」措辞，无禁词）。

- [ ] **Step 4: push + 开修复 PR**

```bash
git push -u origin fix/kd04-principal-reply-decider-chain
gh pr create --title "fix(escalation): 领导微信回复识别改用 decider_chain (KD-04)" --body "$(cat <<'EOF'
## Summary
修复深度审查批 D 唯一 High（KD-04）：用 decider_chain（推荐配置）时领导的微信回复永不被识别为裁决——lookup_principal_config 只查旧标量 principal_decider，而 put_ask_human_policy 从不写它，导致领导裁决掉进普通客户入站链路（领导若是 managed contact 甚至被 AI 当客户自动回复）。

- policy.rs 抽纯谓词 is_decider_for_config（复用 resolve_ask_human_policy，含旧 principal_decider 回落）
- ledger.rs 重写 lookup_principal_config：find_one(principal_decider) → find(current_version)+纯谓词，识别链中全部决策人+改派 next
- 签名不变，webhooks/handle_principal_reply/put_ask_human_policy 不动

## Test plan
- [x] cargo test --lib（+5 确定性单测：decider_chain命中/链中非首位/旧配置兼容/非决策人/空链，baseline≥350 不回退）
- [x] no-human-takeover lint clean
- [ ] 集成/117 真跑：留后续（本地无 Docker，CI integration job 覆盖）

设计：docs/superpowers/specs/2026-07-12-kd04-principal-reply-decider-chain-fix-design.md
台账：docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md KD-04

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review 结论
- **Spec coverage**：设计 3 组件（纯谓词/lookup 重写/5 测试）↔ Task1（谓词+测试）/Task2（lookup 重写）/Task3（baseline 验证+PR），全覆盖；边界处理（DB fail `?` 上抛/空链 None）在 Task2 代码体现；不改清单在 Global Constraints。
- **Placeholder scan**：无 TBD/TODO；每 Step 给完整可编译代码（测试用亲验的 base_config/AskHumanPolicy/DeciderRef 构造）+ 确切命令 + 预期输出。
- **Type consistency**：is_decider_for_config 签名 `(&OperationDomainConfig, &str) -> bool` 在 Task1 定义、Task2 调用一致；lookup_principal_config 返回 `AppResult<Option<String>>` 前后一致；DeciderRef{wxid,display_name} 构造与 policy.rs:168 亲验一致。
- **TDD**：Task1 先写失败测试（Step2）→ 确认失败（Step3）→ 实现（Step4）→ 通过（Step5）；Task2 check+既有测试不回退；Task3 全量 baseline。
- **红线**：每个改代码 Task 的 Step 1 都是"先读懂 + 亲验 file:line"，说不清不动手。
