# 前后端契约对齐 批次3(字典/分类域) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为字典/分类域 5 个实体投影(`operation_state_policy_json` / `operation_domain_json` / `taxonomy_entry_json` / `taxonomy_candidate_json` / `relationship_suggestion_json`)各配后端契约快照测试 + 前端键集对账,并移除防腐烂 lint 的 ALLOWLIST 占位,使这 5 投影真正受机制强制。

**Architecture:** 复用批次1/2 已落地的契约对齐机制——后端 `#[cfg(test)]` 测试构造全量赋值 model → 调投影 `xxx_json()` → `crate::routes::contract_snapshot::assert_contract_fixture(name, value)` 写/读 `frontend/src/contracts/<name>.fixture.json`(canonicalize 递归排序键消抖动;默认只读对账、`UPDATE_SNAPSHOTS=1` bless);前端 vitest 导入**同一份** fixture + `CANONICAL_KEYS` 双向键集对账(missingInFrontend/deadInFrontend)。双门互补 + `every_projection_has_contract_test` 防腐烂 lint 移除本批 5 项 ALLOWLIST 使其受强制。

**Tech Stack:** Rust 2021 (Axum) 后端 `cargo test --lib`;React 19 + TypeScript + Vitest 前端 `npx tsc --noEmit && npx vitest run`。

## Global Constraints

- 测试 only,绝不为过测试改业务逻辑/prompt/guards/阈值(过拟合红线);**5 个投影函数本体一字不改**,只新增契约测试 + bless fixture + 移 ALLOWLIST。
- 新增测试只增量叠加,不删改旧维度/旧断言(本批 5 文件均已有 `mod tests`,契约测试**追加**进现有模块,不动既有 `*_carries_*` / `*_shape_is_stable` 测试)。
- `cargo test --lib` baseline ≥350/0 + 4 PBT(state_transition_pbt/memory_card_invariants/wiki_chunk_revision_pbt/llm_retry_jitter)累计 ≥33/0 不退。
- 所有 cargo 命令前 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`(worktree 共享 target 避免 binary clobber)。
- 对账只断言**顶层键集合**,不断言子键/语义/值/可选性(§3.6)。嵌套对象(taxonomy_entry 的 value、operation_domain 的 askHumanPolicy 等)的内部形状由后端快照固定,不进对账。
- raw Document fixture 必须 bless 生成,绝不手写(嵌套 BSON DateTime/ObjectId 会泄漏 $oid/$date);构造 Document 字段用**纯标量** doc!(批次2 经验)。
- §3.3 全字段构造:每个 Option 给 Some、每个 Vec 非空、每个嵌套结构体给完整纯标量值(暴露顶层键 + 与批次2 decision_review 给 Some 同口径)。
- 禁词纪律(no-human-takeover lint 硬门):fixture/测试/注释绝不出现 人工/接管/takeover/hand-off/human_handoff。本域 model 既有字段名(principal_decider/ask_human_policy/escalate_* 等)是 model 定义不可改,但**投影 operation_domain_json 不下发 principal_decider/high_risk_escalation_mode**,且这些非禁词;测试新增的注释/字符串不得引入禁词。
- 子 agent 一律 `model: "opus"`。
- commit 须用户批准(本 SDD 已获授权:用户选 subagent-driven 执行 + 已授权 commit+push+PR+merge)。
- 回复用中文。

---

## 投影事实表(逐行核实自源码,2026-06-29)

| # | 投影 | 文件:行 | model(字段数) | 下发顶层键数 | 特殊点 |
|---|---|---|---|---|---|
| 1 | `operation_state_policy_json` | admin_state_policies.rs:98 | `OperationStatePolicy`(13) | 13 | allowed/forbidden 是 `Vec<String>`;recommended_pace/previous_version/seeded_by 是 Option;现有测试 :125 有完整构造可参照 |
| 2 | `taxonomy_candidate_json` | admin_taxonomy_candidates.rs:424 | `TaxonomyCandidate`(12) | 13 | 扁平无 Document;id→hex;evidence/reviewed_at/reviewed_by/suggested_display_name 是 Option;现有 `sample_candidate` :447 可参照 |
| 3 | `relationship_suggestion_json` | admin_relationship_suggestions.rs:261 | `RelationshipTypeSuggestion`(13) | 13 | 扁平无 Document;id→hex;evidence/reviewed_at/reviewed_by 是 Option;现有 `sample_suggestion` :284 可参照 |
| 4 | `taxonomy_entry_json` | admin_taxonomies.rs:281 | `TaxonomyEntry`(9)+`TaxonomyValue`(8) | 9 | **value 嵌套对象**(6 子键 id/label/displayName/description/aliases/status);TaxonomyValue 有 3 个**不下发**字段(priority_weight/is_terminal/is_reactivation_target);现有测试 :325 有完整构造可参照 |
| 5 | `operation_domain_json` | domains.rs:273 | `OperationDomainConfig`(22) | 20 | **本批最重**:5 个 String(methodology/workflow/tool_policy/automation_policy/review_policy,**非** Document)+2 个 `Document`(runtime_parameters/state_machine,纯标量 doc!)+`ask_human_policy: Option<AskHumanPolicy>`(给 Some 全字段)+assist_mode_enabled: Option<bool>;**principal_decider/high_risk_escalation_mode 不下发**(22-2=20) |

> 投影顶层键数是契约 fixture 的真相源,但 fixture bless 后以实际 `Object.keys` 为准(本表是核对预期)。

## 各 model 全字段构造速查(逐行核实)

**OperationStatePolicy**(models.rs:1246,13 字段):
`id:Option<ObjectId>` / `workspace_id:String` / `domain:String` / `state_key:String` / `allowed:Vec<String>` / `forbidden:Vec<String>` / `recommended_pace:Option<String>` / `status:String` / `updated_at:DateTime` / `version:i32` / `current_version:bool` / `previous_version:Option<i32>` / `seeded_by:Option<String>`。

**TaxonomyCandidate**(models.rs:2876,12 字段):
`id:Option<ObjectId>` / `scope:String` / `kind:String` / `raw_value:String` / `evidence:Option<String>` / `confidence:i32` / `first_seen_at:DateTime` / `last_seen_at:DateTime` / `occurrences:i32` / `status:String` / `reviewed_at:Option<DateTime>` / `reviewed_by:Option<String>` / `suggested_display_name:Option<String>`。

**RelationshipTypeSuggestion**(models.rs:2910,13 字段):
`id:Option<ObjectId>` / `workspace_id:String` / `account_id:String` / `contact_id:String` / `suggested_value:String` / `evidence:Option<String>` / `confidence:i32` / `status:String` / `occurrences:i32` / `first_seen_at:DateTime` / `last_seen_at:DateTime` / `reviewed_at:Option<DateTime>` / `reviewed_by:Option<String>`。

**TaxonomyEntry**(models.rs:2812,9 字段):
`id:Option<ObjectId>` / `scope:String` / `kind:String` / `value:TaxonomyValue` / `updated_at:DateTime` / `version:i32` / `current_version:bool` / `previous_version:Option<i32>` / `seeded_by:Option<String>`。

**TaxonomyValue**(models.rs:2841,8 字段,**注意非 camelCase 字段名是 snake，但 derive rename_all=camelCase**):
`id:String` / `display_name:String` / `description:String` / `aliases:Vec<String>` / `status:String` / `priority_weight:Option<i32>` / `is_terminal:bool` / `is_reactivation_target:bool`。

**OperationDomainConfig**(models.rs:1173,22 字段):
`id:Option<ObjectId>` / `workspace_id:String` / `domain:String` / `name:String` / `goal:String` / `methodology:String` / `workflow:String` / `tool_policy:String` / `automation_policy:String` / `review_policy:String` / `runtime_parameters:Document` / `state_machine:Document` / `status:String` / `updated_at:DateTime` / `version:i32` / `current_version:bool` / `previous_version:Option<i32>` / `seeded_by:Option<String>` / `principal_decider:Option<String>` / `high_risk_escalation_mode:Option<String>` / `ask_human_policy:Option<AskHumanPolicy>` / `assist_mode_enabled:Option<bool>`。
> 注:这是 22 字段 model,投影 operation_domain_json 只下发 **20** 键——`principal_decider` 和 `high_risk_escalation_mode` 两个不下发(22-2=20),但全字段构造仍须赋值它俩(否则 E0063)。

**AskHumanPolicy**(models.rs:1085,9 字段,derive rename_all=camelCase):
`decider_chain:Vec<DeciderRef>` / `escalate_safety_guard:bool` / `escalate_unverified_product:bool` / `escalate_ai_policy_hold:bool` / `escalate_stuck:bool` / `dedupe_window_hours:Option<f64>` / `daily_push_cap:Option<u32>` / `quiet_hours:Option<AskHumanQuietHours>` / `timeout_hours:Option<f64>`。
**DeciderRef**(models.rs:1067):`wxid:String` / `display_name:Option<String>`。
**AskHumanQuietHours**(models.rs:1076):`start_hour:u8` / `end_hour:u8` / `tz_offset_hours:i8`。

---

## 共享实现约定(所有后端 task 适用)

- **插入位置**:本批 5 个文件**都已有** `#[cfg(test)] mod tests { use super::*; ... }`(state_policies :119 / candidates :442 / suggestions :279 / taxonomies :318;domains 见 Task 5)。契约测试函数**追加进现有 `mod tests` 内部**,放在该模块最后一个 `}`(即文件末尾的闭合括号)**之前**,**不新建模块**。因 `use super::*` 已透传投影函数名,契约测试**无需**额外 `use super::xxx_json;`(与批次2 shared.rs 用 item-specific import 不同,本批靠 `use super::*`)。
- **调用形态**(批次2 同款):构造全字段 model → `let value = <投影>(item);` → `crate::routes::contract_snapshot::assert_contract_fixture("<name>", value);`。
- **局部 use**:测试函数体内若需 `DateTime` / `ObjectId`,在函数体首行局部 `use`(如 `use mongodb::bson::DateTime;`),避免依赖模块级 use 是否已引入。批次2 同款写法。
- **bless 流程**:先写测试(此时 fixture 不存在,只读对账会 panic)→ 运行 `UPDATE_SNAPSHOTS=1 cargo test --lib <test_name>` bless 生成 fixture → 再次 `cargo test --lib <test_name>` 只读对账确认绿。
- **worktree 争用预案**:若 cargo 报 binary clobber / "Blocking waiting for file lock" / 测试 0 命中,遵「本地资源受限走 CI」纪律——个体测试趁空窗亲跑留证,不假绿;记入 SDD 账本。

---

> **执行顺序按 Task 编号 1→7**(本文档物理排版因编辑顺序为 4,5,1,2,3,6,7;`task-brief` 按 `### Task N` 编号精确提取,物理顺序不影响执行)。Task 1-5 后端契约快照彼此独立可并行思路实现(但 SDD 串行 dispatch);Task 6 前端 harness 依赖 Task 1-5 的 fixture;Task 7 ALLOWLIST 收口依赖 Task 1-5 的后端测试。

---

### Task 4: taxonomy_entry_json 契约快照(含 value 嵌套对象)

**Files:**
- Modify: `src/routes/admin_taxonomies.rs`(追加契约测试进现有 `mod tests`,模块闭合 `}` 在文件末尾 :397)
- Create(bless 生成): `frontend/src/contracts/taxonomy_entry.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `taxonomy_entry_json(entry: TaxonomyEntry) -> Value`(同文件 :281,本 task 不改它)。
- Produces: fixture `taxonomy_entry.fixture.json`(9 顶层键,含 value 嵌套对象),供 Task 6 前端对账。

**关键点**:taxonomy_entry_json 把 `entry.value`(TaxonomyValue)手工映射成嵌套对象 `value: {id,label,displayName,description,aliases,status}`(6 子键)。对账只锁**顶层** 9 键(value 作为一个键),value 内部 6 子键的形状由后端快照固定、不进前端 CANONICAL_KEYS。TaxonomyValue 的 priority_weight/is_terminal/is_reactivation_target 三字段投影不映射进 value 子对象——但全字段构造仍须赋值它们(否则 E0063)。

- [ ] **Step 1: 在 admin_taxonomies.rs 的 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:taxonomy_entry_json。TaxonomyEntry 9 字段 + TaxonomyValue 8 字段全量构造
    /// (previous_version/seeded_by/priority_weight 三个 Option 给 Some);id→hex;
    /// updated_at→RFC3339。顶层 9 键,value 是嵌套对象(6 子键,内部形状由快照固定不进对账)。
    /// TaxonomyValue 的 priority_weight/is_terminal/is_reactivation_target 投影不下发,但须赋值。
    #[test]
    fn taxonomy_entry_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let entry = TaxonomyEntry {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: "first_contact".to_string(),
                display_name: "首次接触".to_string(),
                description: "刚加上微信、还没业务对话".to_string(),
                aliases: vec!["new_lead".to_string()],
                status: "active".to_string(),
                priority_weight: Some(10),
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: DateTime::from_millis(1_700_000_000_000),
            version: 1,
            current_version: true,
            previous_version: Some(0),
            seeded_by: Some("system".to_string()),
        };
        let value = taxonomy_entry_json(entry);
        crate::routes::contract_snapshot::assert_contract_fixture("taxonomy_entry", value);
    }
```

- [ ] **Step 2: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib taxonomy_entry_json_matches_contract_fixture`
Expected: PASS(写出 fixture)。

- [ ] **Step 3: 只读对账 + 核对 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib taxonomy_entry_json_matches_contract_fixture`
Expected: PASS。
核对 `taxonomy_entry.fixture.json`:**9 顶层键**(id/scope/kind/value/updatedAt/version/currentVersion/previousVersion/seededBy);value 是对象含 6 子键(id/label/displayName/description/aliases/status,canonicalize 已字母序);value 子对象**无** priorityWeight/isTerminal/isReactivationTarget(投影不映射);id hex;updatedAt RFC3339;value 内无 $oid/$date。

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_taxonomies.rs frontend/src/contracts/taxonomy_entry.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): taxonomy_entry 投影契约快照(批次3,含 value 嵌套对象)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: operation_domain_json 契约快照(本批最重:2 Document + AskHumanPolicy)

**Files:**
- Modify: `src/routes/domains.rs`(追加契约测试进现有 `mod tests`,模块闭合 `}` 在文件末尾 :595)
- Create(bless 生成): `frontend/src/contracts/operation_domain.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `operation_domain_json(config: OperationDomainConfig) -> Value`(同文件 :273,本 task 不改它)。
- Produces: fixture `operation_domain.fixture.json`(20 顶层键),供 Task 6 前端对账。

**关键点(逐条核实)**:
1. `methodology/workflow/tool_policy/automation_policy/review_policy` 是 **String 非 Document**(投影直发字符串值)。
2. `runtime_parameters/state_machine` 是 **`Document`**(非 Option)——用**纯标量** `doc!`(避免嵌套 DateTime/ObjectId 泄漏 $date/$oid)。
3. `ask_human_policy: Option<AskHumanPolicy>` 给 **Some**(§3.3 暴露完整;内部纯标量结构,无 Document/时间戳/ObjectId,不泄漏)。需构造 `AskHumanPolicy` 9 字段 + `DeciderRef` + `AskHumanQuietHours`。
4. `assist_mode_enabled: Option<bool>` 给 Some。
5. `principal_decider/high_risk_escalation_mode` 投影**不下发**,但全字段构造须赋值(给 Some)否则 E0063。
6. 投影下发 **20 顶层键**:OperationDomainConfig 22 字段中,`principal_decider` 和 `high_risk_escalation_mode` 两个不下发(投影体 domains.rs:274-294 共 20 个 json 键,22-2=20)。bless 后核对 fixture `Object.keys` 应恰为 20 键且无 principalDecider/highRiskEscalationMode。

- [ ] **Step 1: 在 domains.rs 的 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:operation_domain_json。OperationDomainConfig 22 字段全量构造。
    /// methodology 等 5 个是 String(非 Document);runtime_parameters/state_machine 是
    /// Document(纯标量 doc! 避泄漏);ask_human_policy 给 Some(完整 AskHumanPolicy);
    /// principal_decider/high_risk_escalation_mode 赋值但投影不下发。顶层下发 20 键。
    #[test]
    fn operation_domain_json_matches_contract_fixture() {
        use crate::models::{AskHumanPolicy, AskHumanQuietHours, DeciderRef};
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let config = OperationDomainConfig {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            domain: "user_operations".to_string(),
            name: "私域用户运营".to_string(),
            goal: "把新客养成签约客户".to_string(),
            methodology: "FAB + 顾问式销售".to_string(),
            workflow: "破冰→需求挖掘→方案→促单".to_string(),
            tool_policy: "知识库优先,无依据不报价".to_string(),
            automation_policy: "高风险静默 hold".to_string(),
            review_policy: "独立复核 + 一次重写".to_string(),
            runtime_parameters: doc! { "minIntervalSeconds": 60, "dailyCap": 20 },
            state_machine: doc! { "states": "new_contact,need_discovery" },
            status: "active".to_string(),
            updated_at: DateTime::from_millis(1_700_000_000_000),
            version: 2,
            current_version: true,
            previous_version: Some(1),
            seeded_by: Some("system".to_string()),
            principal_decider: Some("wxid_leader".to_string()),
            high_risk_escalation_mode: Some("decision_only".to_string()),
            ask_human_policy: Some(AskHumanPolicy {
                decider_chain: vec![DeciderRef {
                    wxid: "wxid_leader".to_string(),
                    display_name: Some("张总".to_string()),
                }],
                escalate_safety_guard: true,
                escalate_unverified_product: true,
                escalate_ai_policy_hold: false,
                escalate_stuck: true,
                dedupe_window_hours: Some(6.0),
                daily_push_cap: Some(10),
                quiet_hours: Some(AskHumanQuietHours {
                    start_hour: 22,
                    end_hour: 8,
                    tz_offset_hours: 8,
                }),
                timeout_hours: Some(24.0),
            }),
            assist_mode_enabled: Some(false),
        };
        let value = operation_domain_json(config);
        crate::routes::contract_snapshot::assert_contract_fixture("operation_domain", value);
    }
```

> 若构造报 E0063(缺字段)或 E0560(字段名错):以 models.rs:1173 的 OperationDomainConfig 定义为准逐字核对字段名。若 `DeciderRef`/`AskHumanQuietHours`/`AskHumanPolicy` 导入路径报错:它们都在 `crate::models`(models.rs:1067/1076/1085)。

- [ ] **Step 2: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib operation_domain_json_matches_contract_fixture`
Expected: PASS(写出 fixture)。

- [ ] **Step 3: 只读对账 + 核对 fixture(本 task 重点核对 BSON 泄漏)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operation_domain_json_matches_contract_fixture`
Expected: PASS。
**逐项核对** `operation_domain.fixture.json`:
- 顶层键集(以实际 `Object.keys` 为准,预期 20 键:id/workspaceId/domain/name/goal/methodology/workflow/toolPolicy/automationPolicy/reviewPolicy/runtimeParameters/stateMachine/assistModeEnabled/status/updatedAt/version/currentVersion/previousVersion/seededBy/askHumanPolicy)。
- **无** principalDecider / highRiskEscalationMode(投影不下发,证明过滤生效)。
- **无 BSON 泄漏**:全文件 grep 不到 `$oid` / `$date` / `$numberInt` / `$numberLong`。runtimeParameters/stateMachine 是纯标量对象;askHumanPolicy 是嵌套对象(decider_chain 数组等,纯标量);id hex;updatedAt RFC3339 字符串。
- methodology 等 5 个是普通字符串值(非对象)。

- [ ] **Step 4: Commit**

```bash
git add src/routes/domains.rs frontend/src/contracts/operation_domain.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): operation_domain 投影契约快照(批次3,2 Document + AskHumanPolicy)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 1: operation_state_policy_json 契约快照

**Files:**
- Modify: `src/routes/admin_state_policies.rs`(追加契约测试进现有 `mod tests`,模块闭合 `}` 在 :200)
- Create(bless 生成): `frontend/src/contracts/operation_state_policy.fixture.json`

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture(name: &str, value: serde_json::Value)`(批次1 已落地);投影 `operation_state_policy_json(policy: OperationStatePolicy) -> Value`(同文件 :98,本 task 不改它)。
- Produces: fixture `operation_state_policy.fixture.json`(13 顶层键),供 Task 6 前端对账。

- [ ] **Step 1: 在 admin_state_policies.rs 的 `mod tests` 末尾(:200 闭合 `}` 之前)追加契约测试**

```rust
    /// 契约快照:operation_state_policy_json。OperationStatePolicy 13 字段全量构造
    /// (recommended_pace/previous_version/seeded_by 三个 Option 给 Some;allowed/forbidden
    /// 非空 Vec);id→hex。投影下发全部 13 键(无字段过滤)。
    #[test]
    fn operation_state_policy_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let policy = OperationStatePolicy {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            domain: "user_operations".to_string(),
            state_key: "need_discovery".to_string(),
            allowed: vec!["text_reply".to_string()],
            forbidden: vec!["product_pitch".to_string()],
            recommended_pace: Some("normal".to_string()),
            status: "active".to_string(),
            updated_at: DateTime::from_millis(1_700_000_000_000),
            version: 3,
            current_version: true,
            previous_version: Some(2),
            seeded_by: Some("manual".to_string()),
        };
        let value = operation_state_policy_json(policy);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "operation_state_policy",
            value,
        );
    }
```

- [ ] **Step 2: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib operation_state_policy_json_matches_contract_fixture`
Expected: PASS(写出 `frontend/src/contracts/operation_state_policy.fixture.json`)。

- [ ] **Step 3: 只读对账确认绿 + 核对 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operation_state_policy_json_matches_contract_fixture`
Expected: PASS(只读对账)。
核对 `frontend/src/contracts/operation_state_policy.fixture.json`:13 顶层键(id/workspaceId/domain/stateKey/allowed/forbidden/recommendedPace/status/updatedAt/version/currentVersion/previousVersion/seededBy);id 是 hex 字符串 `507f...`(非 `{$oid}`);updatedAt 是 RFC3339 字符串(非 `{$date}`);allowed/forbidden 是字符串数组。

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_state_policies.rs frontend/src/contracts/operation_state_policy.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): operation_state_policy 投影契约快照(批次3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: taxonomy_candidate_json 契约快照

**Files:**
- Modify: `src/routes/admin_taxonomy_candidates.rs`(追加契约测试进现有 `mod tests`,模块闭合 `}` 在文件末尾 :520)
- Create(bless 生成): `frontend/src/contracts/taxonomy_candidate.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `taxonomy_candidate_json(item: TaxonomyCandidate) -> Value`(同文件 :424,本 task 不改它)。
- Produces: fixture `taxonomy_candidate.fixture.json`(13 顶层键),供 Task 6 前端对账。

- [ ] **Step 1: 在 admin_taxonomy_candidates.rs 的 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:taxonomy_candidate_json。TaxonomyCandidate 12 字段全量构造
    /// (evidence/reviewed_at/reviewed_by/suggested_display_name 四个 Option 给 Some);id→hex;
    /// first_seen_at/last_seen_at→RFC3339;reviewed_at→Some 后 RFC3339。投影下发 13 键。
    #[test]
    fn taxonomy_candidate_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let item = TaxonomyCandidate {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            scope: "global".to_string(),
            kind: "objection_type".to_string(),
            raw_value: "太贵了".to_string(),
            evidence: Some("用户说价格高".to_string()),
            confidence: 7,
            first_seen_at: DateTime::from_millis(1_700_000_000_000),
            last_seen_at: DateTime::from_millis(1_700_000_100_000),
            occurrences: 3,
            status: "pending".to_string(),
            reviewed_at: Some(DateTime::from_millis(1_700_000_200_000)),
            reviewed_by: Some("admin-1".to_string()),
            suggested_display_name: Some("价格异议".to_string()),
        };
        let value = taxonomy_candidate_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("taxonomy_candidate", value);
    }
```

- [ ] **Step 2: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib taxonomy_candidate_json_matches_contract_fixture`
Expected: PASS(写出 fixture)。

- [ ] **Step 3: 只读对账 + 核对 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib taxonomy_candidate_json_matches_contract_fixture`
Expected: PASS。
核对 `taxonomy_candidate.fixture.json`:13 键(id/scope/kind/rawValue/evidence/confidence/occurrences/status/firstSeenAt/lastSeenAt/reviewedAt/reviewedBy/suggestedDisplayName);id hex;三个时间戳 RFC3339 字符串;reviewedAt 非 null(给了 Some)。

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_taxonomy_candidates.rs frontend/src/contracts/taxonomy_candidate.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): taxonomy_candidate 投影契约快照(批次3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: relationship_suggestion_json 契约快照

**Files:**
- Modify: `src/routes/admin_relationship_suggestions.rs`(追加契约测试进现有 `mod tests`,模块闭合 `}` 在文件末尾 :354)
- Create(bless 生成): `frontend/src/contracts/relationship_suggestion.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `relationship_suggestion_json(item: RelationshipTypeSuggestion) -> Value`(同文件 :261,本 task 不改它)。
- Produces: fixture `relationship_suggestion.fixture.json`(13 顶层键),供 Task 6 前端对账。

- [ ] **Step 1: 在 admin_relationship_suggestions.rs 的 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:relationship_suggestion_json。RelationshipTypeSuggestion 13 字段全量构造
    /// (evidence/reviewed_at/reviewed_by 三个 Option 给 Some);id→hex;
    /// first_seen_at/last_seen_at→RFC3339;reviewed_at→Some 后 RFC3339。投影下发全部 13 键。
    #[test]
    fn relationship_suggestion_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let item = RelationshipTypeSuggestion {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: "507f1f77bcf86cd799439012".to_string(),
            suggested_value: "peer".to_string(),
            evidence: Some("用户自称同行".to_string()),
            confidence: 7,
            status: "pending".to_string(),
            occurrences: 2,
            first_seen_at: DateTime::from_millis(1_700_000_000_000),
            last_seen_at: DateTime::from_millis(1_700_000_100_000),
            reviewed_at: Some(DateTime::from_millis(1_700_000_200_000)),
            reviewed_by: Some("admin-1".to_string()),
        };
        let value = relationship_suggestion_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "relationship_suggestion",
            value,
        );
    }
```

- [ ] **Step 2: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib relationship_suggestion_json_matches_contract_fixture`
Expected: PASS(写出 fixture)。

- [ ] **Step 3: 只读对账 + 核对 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib relationship_suggestion_json_matches_contract_fixture`
Expected: PASS。
核对 `relationship_suggestion.fixture.json`:13 键(id/workspaceId/accountId/contactId/suggestedValue/evidence/confidence/occurrences/status/firstSeenAt/lastSeenAt/reviewedAt/reviewedBy);id hex;contactId 是普通字符串(投影直发 contact_id,非 hex 转换);三时间戳 RFC3339;reviewedAt 非 null。

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_relationship_suggestions.rs frontend/src/contracts/relationship_suggestion.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): relationship_suggestion 投影契约快照(批次3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 前端 5 投影键集对账 harness

**Files:**
- Create: `frontend/src/contracts/operationStatePolicy.contract.ts`、`taxonomyCandidate.contract.ts`、`relationshipSuggestion.contract.ts`、`taxonomyEntry.contract.ts`、`operationDomain.contract.ts`(5 份 `CANONICAL_KEYS`)
- Create: `frontend/src/__tests__/contracts/taxonomyDomain.contract.test.ts`(5 投影双向对账)

**Interfaces:**
- Consumes: Task 1-5 bless 出的 5 份 `<name>.fixture.json`。
- Produces: 5 个 vitest 对账测试(进 frontend-contract CI job)。

**关键纪律**:`CANONICAL_KEYS` 的每个键**逐字抄自对应 bless 出的 fixture**(读真相源,不靠本计划标注的键数)。先读全部 5 份 fixture 取真实顶层键(canonicalize 已字母序),再写 `as const`。模板照搬批次1/2 的 `assertKeysMatch`(双向集合比对 + 非空断言)。

> taxonomy_entry 的 `value`、operation_domain 的 `runtimeParameters/stateMachine/askHumanPolicy` 是**嵌套对象**,但对账只锁**顶层**键——CANONICAL_KEYS 里它们各算**一个顶层键**(如 `"value"`、`"askHumanPolicy"`),不展开内部子键。

- [ ] **Step 1: 读 5 份 fixture 取真实顶层键**

读这 5 份(Task 1-5 生成):`operation_state_policy` / `taxonomy_candidate` / `relationship_suggestion` / `taxonomy_entry` / `operation_domain`。用 `node -e "console.log(JSON.stringify(Object.keys(require('./<name>.fixture.json')).sort()))"` 或直接 Read 取每份顶层键。

- [ ] **Step 2: 写 5 份 contract.ts**

每份形如(以 operation_state_policy 为例,键**抄自 fixture**;顶部一行中文注释说明是哪个后端投影):

```typescript
// frontend/src/contracts/operationStatePolicy.contract.ts
// 后端 operation_state_policy_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "allowed",
  "currentVersion",
  "domain",
  "forbidden",
  "id",
  "previousVersion",
  "recommendedPace",
  "seededBy",
  "stateKey",
  "status",
  "updatedAt",
  "version",
  "workspaceId",
] as const;
```

其余 4 份同构,文件名 camelCase、键抄自各自 fixture:
- `taxonomyCandidate.contract.ts`(预期 13 键:confidence/evidence/firstSeenAt/id/kind/lastSeenAt/occurrences/rawValue/reviewedAt/reviewedBy/scope/status/suggestedDisplayName)
- `relationshipSuggestion.contract.ts`(预期 13 键:accountId/confidence/contactId/evidence/firstSeenAt/id/lastSeenAt/occurrences/reviewedAt/reviewedBy/status/suggestedValue/workspaceId)
- `taxonomyEntry.contract.ts`(预期 9 顶层键:currentVersion/id/kind/previousVersion/scope/seededBy/updatedAt/value/version——`value` 是一个顶层键不展开)
- `operationDomain.contract.ts`(预期 20 顶层键,以 fixture 实际为准:askHumanPolicy/assistModeEnabled/automationPolicy/currentVersion/domain/goal/id/methodology/name/previousVersion/reviewPolicy/runtimeParameters/seededBy/stateMachine/status/toolPolicy/updatedAt/version/workflow/workspaceId)

> 预期键以本计划标注为核对参考,**最终以 bless 出的 fixture `Object.keys` 为准**。若不一致,以 fixture 为准并在报告说明。

- [ ] **Step 3: 写 taxonomyDomain.contract.test.ts**

```typescript
import { describe, it, expect } from "vitest";
import statePolicyFixture from "../../contracts/operation_state_policy.fixture.json";
import candidateFixture from "../../contracts/taxonomy_candidate.fixture.json";
import suggestionFixture from "../../contracts/relationship_suggestion.fixture.json";
import entryFixture from "../../contracts/taxonomy_entry.fixture.json";
import domainFixture from "../../contracts/operation_domain.fixture.json";
import { CANONICAL_KEYS as STATE_POLICY_KEYS } from "../../contracts/operationStatePolicy.contract";
import { CANONICAL_KEYS as CANDIDATE_KEYS } from "../../contracts/taxonomyCandidate.contract";
import { CANONICAL_KEYS as SUGGESTION_KEYS } from "../../contracts/relationshipSuggestion.contract";
import { CANONICAL_KEYS as ENTRY_KEYS } from "../../contracts/taxonomyEntry.contract";
import { CANONICAL_KEYS as DOMAIN_KEYS } from "../../contracts/operationDomain.contract";

// 后端投影写出的 fixture(线上真相源)与前端 CANONICAL_KEYS 双向键集对账。
// missingInFrontend=后端发了前端没声明;deadInFrontend=前端声明了后端没发。
function assertKeysMatch(
  label: string,
  fixture: Record<string, unknown>,
  declared: readonly string[],
) {
  const actual = Object.keys(fixture).sort();
  const decl = [...declared].sort();
  const missingInFrontend = actual.filter((k) => !decl.includes(k));
  const deadInFrontend = decl.filter((k) => !actual.includes(k));
  expect(
    { missingInFrontend, deadInFrontend },
    `${label}: 后端新增字段→前端须在 CANONICAL_KEYS 登记;后端删字段→前端须清理死键`,
  ).toEqual({ missingInFrontend: [], deadInFrontend: [] });
  expect(actual.length, `${label}: fixture 非空`).toBeGreaterThan(0);
}

describe("契约: 字典/分类域投影键集对账", () => {
  it("operation_state_policy 投影", () =>
    assertKeysMatch("statePolicy", statePolicyFixture, STATE_POLICY_KEYS));
  it("taxonomy_candidate 投影", () =>
    assertKeysMatch("taxonomyCandidate", candidateFixture, CANDIDATE_KEYS));
  it("relationship_suggestion 投影", () =>
    assertKeysMatch("relationshipSuggestion", suggestionFixture, SUGGESTION_KEYS));
  it("taxonomy_entry 投影(顶层 9 键,value 嵌套不展开)", () =>
    assertKeysMatch("taxonomyEntry", entryFixture, ENTRY_KEYS));
  it("operation_domain 投影(顶层 20 键,Document/policy 嵌套不展开)", () =>
    assertKeysMatch("operationDomain", domainFixture, DOMAIN_KEYS));
});
```

- [ ] **Step 4: 跑 tsc + vitest 确认全绿**

Run: `cd frontend && npx tsc --noEmit && npx vitest run src/__tests__/contracts/taxonomyDomain.contract.test.ts`
Expected: tsc 0 错(若报与本任务无关的既有错误,如实记录不改无关文件);5 个对账测试全 PASS。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/contracts/operationStatePolicy.contract.ts frontend/src/contracts/taxonomyCandidate.contract.ts frontend/src/contracts/relationshipSuggestion.contract.ts frontend/src/contracts/taxonomyEntry.contract.ts frontend/src/contracts/operationDomain.contract.ts frontend/src/__tests__/contracts/taxonomyDomain.contract.test.ts
git commit -m "$(cat <<'EOF'
feat(contract): 字典/分类域 5 投影前端键集对账 harness(批次3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: ALLOWLIST 移除 5 投影 + 防腐烂 lint 验证 + 批次收口

**Files:**
- Modify: `src/routes/contract_snapshot.rs`(ALLOWLIST 移除本批 5 项)

**Interfaces:**
- Consumes: Task 1-5 的后端测试(让 lint 的 covered 集含这 5 投影)。
- Produces: 防腐烂 lint 真正强制本批 5 投影(忘配测试即红)。

**说明**:批次1/2 把本批 5 投影列入 ALLOWLIST 占位。Task 1-5 既已为它们配了契约测试,现移除豁免,让 lint `every_projection_has_contract_test` 真正咬这 5 个。

- [ ] **Step 1: 从 ALLOWLIST 移除本批 5 项**

在 `src/routes/contract_snapshot.rs` 的 ALLOWLIST 删除这 5 行(连同行尾逗号):
`"operation_state_policy_json"`、`"taxonomy_candidate_json"`、`"relationship_suggestion_json"`、`"taxonomy_entry_json"`、`"operation_domain_json"`。

**保留**其余批次5 域项(`suspected_deal_json` / `outbox_entry_json` / `evaluation_scenario_json` / `playbook_json` / `prompt_template_json`)+ 批次4 进化域项 + helper 项不动。删除前先 Read ALLOWLIST 当前内容,逐项确认只动这 5 个(它们可能与批次5 域项交错排列,按投影名精确删除,不按行号段删)。

- [ ] **Step 2: 跑防腐烂 lint 确认 5 投影被覆盖、无 orphan**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib every_projection_has_contract_test`
Expected: PASS(5 投影都在各自测试块的 `assert_contract_fixture` 窗口内被覆盖;无 orphans)。

> 若报 orphans 含本批某投影:说明该投影测试的 `assert_contract_fixture("<name>",...)` 调用与 `fn <投影名>` 不在同一覆盖窗口(lint 在投影名出现处前 600/后 200 字符找 `assert_contract_fixture`)。本批每个测试都有 `let value = <投影>(...);` + 紧随 `assert_contract_fixture(...)` → 投影名 token 在窗口内 → covered。

- [ ] **Step 3: 跑全量 lib baseline 确认不退**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`,N = 批次2 后基线 1695 + 本批 5 测试 = 1700。

> 若本地撞 worktree 共享 target 争用,遵「本地资源受限走 CI」纪律:不假绿,个体 task 测试趁空窗已亲跑留证,全量 baseline 留 CI baseline gate 验证。记入 SDD 账本。

- [ ] **Step 4: Commit**

```bash
git add src/routes/contract_snapshot.rs
git commit -m "$(cat <<'EOF'
feat(contract): 防腐烂 lint 强制字典/分类域 5 投影(批次3 ALLOWLIST 移除)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review(写完计划后自查)

**Spec 覆盖**:本批对应 spec §5 第 3 批(字典/分类域)5 投影,逐一有 task(Task1-5)。机制复用 spec §3.3/§3.4 双门、§6 防腐烂 lint(Task7 移除 ALLOWLIST)、§4.3 raw Document 容差(operation_domain 用纯标量 Document 避泄漏)。CI §7 不需改(批次1 已建 frontend-contract job + paths-filter,本批 fixture/contract 落在已覆盖路径)。

**Placeholder 扫描**:无 TBD/TODO;每个后端 task 给全字段构造代码;前端 task 给 assertKeysMatch 全文 + contract.ts 模板;键集明确「抄自 fixture」并给核对预期。

**类型一致性**:5 个投影 + 6 个 model(OperationStatePolicy:1246 / TaxonomyCandidate:2876 / RelationshipTypeSuggestion:2910 / TaxonomyEntry:2812 + TaxonomyValue:2841 / OperationDomainConfig:1173 + AskHumanPolicy:1085 + DeciderRef:1067 + AskHumanQuietHours:1076)字段名/类型逐行核对自 models.rs;投影体逐字核对自各 routes 文件。

**已知风险与对策**:
1. operation_domain 的 5 个 String 字段易误当 Document(已在事实表 + Task5 关键点明确标 String);2 个真 Document(runtime_parameters/state_machine)用纯标量 doc! 避泄漏。
2. ask_human_policy 给 Some 需构造 3 个嵌套结构体(已给完整代码 + 导入路径)。
3. operation_domain model **22** 字段,投影下发 20 键(principal_decider/high_risk_escalation_mode 两个不下发,22-2=20,无其它差额);Task5 Step3 仍以 bless fixture 实际 `Object.keys` 复核。
4. 5 文件 mod tests 都用 `use super::*`,契约测试无需函数级 use(与批次2 shared.rs 不同,已在共享实现约定标注)。
5. 文档物理顺序 4,5,1,2,3,6,7——已加导航注记;task-brief 按编号提取已实测鲁棒。

**键数核对预期**(以 fixture 为最终真相源):state_policy 13 / candidate 13 / suggestion 13 / entry 9(顶层)/ domain 20(顶层)。
