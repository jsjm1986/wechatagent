# 前后端契约对齐 批次4(进化/实验域) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为进化/实验域 8 个实体投影各配后端契约快照测试 + 前端键集对账,并移除防腐烂 lint 的 ALLOWLIST 占位(cohort_run_ids_json 改归 helper 豁免区),使这 8 投影真正受机制强制。

**Architecture:** 复用批次1/2/3 已落地的契约对齐机制——后端 `#[cfg(test)]` 测试构造全量赋值 model → 调投影 `xxx_json()` → `crate::routes::contract_snapshot::assert_contract_fixture(name, value)` 写/读 `frontend/src/contracts/<name>.fixture.json`(canonicalize 递归排序键消抖动;默认只读对账、`UPDATE_SNAPSHOTS=1` bless);前端 vitest 导入**同一份** fixture + `CANONICAL_KEYS` 双向键集对账。双门互补 + `every_projection_has_contract_test` 防腐烂 lint。

**Tech Stack:** Rust 2021 (Axum) 后端 `cargo test --lib`;React 19 + TypeScript + Vitest 前端 `npx tsc --noEmit && npx vitest run`。

## Global Constraints

- 测试 only,绝不为过测试改业务逻辑/prompt/guards/阈值(过拟合红线);**8 个投影函数本体一字不改**,只新增契约测试 + bless fixture + 移 ALLOWLIST。
- 新增测试只增量叠加,不删改旧维度/旧断言(evolution.rs 已有 `mod tests` 含既有 `threshold_override_audit_json_carries_value_transition` / `shadow_replay_json_exposes_*` 等测试,契约测试**追加**进现有模块,不动既有测试)。
- `cargo test --lib` baseline ≥350/0 + 4 PBT 累计 ≥33/0 不退(批次3 后基线 1720,本批 +8 = 1728)。
- 所有 cargo 命令前 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`(worktree 共享 target 避免 binary clobber)。
- 对账只断言**顶层键集合**,不断言子键/语义/值/可选性。嵌套对象(experiment_summary 的 experiment/proposals、proposal_detail 的 cohortNotes/evalMetrics、shadow_replay 的 original5gateHit/new5gateHit 等)内部形状由后端快照固定,不进对账。
- raw Document fixture 必须 bless 生成,绝不手写;构造 Document 字段用**纯标量** doc!(批次2/3 经验:bson_doc_to_json = `to_value(Bson::Document)`,嵌套 DateTime/ObjectId 会泄漏 $oid/$date;纯标量 doc! 不泄漏,既有测试 shadow_replay :800 全 bool doc! 作证)。
- §3.3 全字段构造:每个 Option 给 Some、每个 Vec 非空、每个 Document 给纯标量非空内容(暴露顶层键 + 与批次2/3 同口径)。
- 禁词纪律(no-human-takeover lint 硬门):fixture/测试/注释绝不出现 人工/接管/takeover/hand-off/human_handoff。本域 model/字段含 `held_by_ai_policy` 等 AI 内部状态名是既有定义可用;测试新增注释/字符串不得引入禁词。
- 子 agent 一律 `model: "opus"`。
- commit 须用户批准(本 SDD 已获授权)。
- 回复用中文。

---

## 投影事实表(逐行核实自源码 evolution.rs,2026-06-29)

> evolution.rs 私有 helper:`datetime_to_rfc3339(dt: DateTime) -> String`(:540,非 Option 直转;Option 用 `.map(datetime_to_rfc3339)`);`bson_doc_to_json(&Document) -> Value`(:545,= `to_value(Bson::Document)`,纯标量不泄漏、空 Document→`{}`)。

| # | 投影 | 行 | 签名 | 下发顶层键数 | 特殊点 |
|---|---|---|---|---|---|
| 1 | `runtime_flag_json` | :699 | `(f: &EvolutionRuntimeFlag)` | 7 | 最简;rolloutPercent=`rollout_percent_clamped()`方法+rolloutPercentRaw双发;**无 id 下发**;updatedBy 是 Option |
| 2 | `threshold_override_json` | :527 | `(o: &ThresholdOverride)` | 8 | id→`Option.map(to_hex)`;sourceProposalId→`to_hex()`(非Option);rolledBackAt/rolledBackBy 是 Option |
| 3 | `threshold_override_audit_json` | :684 | `(a: &ThresholdOverrideAudit)` | 10 | significanceMetrics=`Option<Document>.as_ref().map(bson_doc_to_json)`;previousValue/newValue/hitRateObserved 是 Option<f64>;现有测试 :762 完整构造可参照 |
| 4 | `experiment_envelope_json` | :416 | `(exp: &Experiment)` | 14 | cohortThresholdSize/cohortPromptSize=`Vec.len() as i64`;finishedAt 是 Option;id 不下发 |
| 5 | `proposal_summary_json` | :435 | `(p: &Proposal)` | 14 | id→`Option.map(to_hex)`;多 Option 字段;无 Document |
| 6 | `shadow_replay_json` | :507 | `(r: &ShadowReplay)` | 15 | sourceRunId→`to_hex()`(非Option);original5gateHit/new5gateHit=bson_doc_to_json(纯标量 doc!);现有测试 :788 完整构造可参照 |
| 7 | `proposal_detail_json` | :454 | `(p: &Proposal)` | 28 | **最重**:cohortNotes/evalMetrics=bson_doc_to_json(纯标量 doc!);多 Option;experimentId/workspaceId/accountId 直发 |
| 8 | `experiment_summary_json` | :395 | `(exp: &Experiment, proposals: Vec<Proposal>)` | 3 | **聚合**:顶层仅 experiment(嵌套 envelope)/proposalsCounts(BTreeMap)/proposals(数组);吃 2 参数 |

> **cohort_run_ids_json(:488)不纳入本批**:返回裸数组 `json!([...])` 非对象,无顶层键集概念,契约机制(canonicalize+CANONICAL_KEYS 比 Object.keys)只适配对象;且它从不作顶层响应——作 `cohortRunIds` 键嵌在 proposal 详情端点 inline 聚合(:132)里,形状由该端点间接覆盖。本质是数组化 helper,Task 9 把它从 ALLOWLIST"待覆盖批次域"区移到 helper 豁免区并加注释。

## 各 model 全字段构造速查(逐行核实)

**EvolutionRuntimeFlag**(models.rs:1301,7 字段):`id:Option<ObjectId>` / `workspace_id:String` / `enabled:bool` / `rollout_percent:u32` / `updated_by:Option<String>` / `threshold_auto_release_enabled:bool` / `updated_at:DateTime`。`rollout_percent_clamped()` 是方法(`min(100)`,models.rs:1325)。投影下发 7 键(id 不下发;rolloutPercent=clamped 方法值 + rolloutPercentRaw=原始字段双发)。

**ThresholdOverride**(models.rs:4529,9 字段):`id:Option<ObjectId>` / `workspace_id:String` / `account_id:String` / `gate_key:String` / `value:f64` / `source_proposal_id:ObjectId` / `released_at:DateTime` / `released_by:String` / `rolled_back_at:Option<DateTime>` / `rolled_back_by:Option<String>`。

**ThresholdOverrideAudit**(models.rs:4557,12 字段):`id:Option<ObjectId>` / `workspace_id:String` / `account_id:String` / `gate_key:String` / `action:String` / `previous_value:Option<f64>` / `new_value:Option<f64>` / `source_proposal_id:ObjectId` / `decided_by:String` / `decided_at:DateTime` / `hit_rate_observed:Option<f64>` / `significance_metrics:Option<Document>`。现有测试 :762 完整构造可参照。

**Experiment**(models.rs:4405,15 字段):`id:Option<ObjectId>` / `experiment_id:String` / `workspace_id:String` / `account_id:String` / `status:String` / `window_hours:i32` / `started_at:DateTime` / `updated_at:DateTime` / `finished_at:Option<DateTime>` / `cohort_threshold_run_ids:Vec<ObjectId>` / `cohort_prompt_run_ids:Vec<ObjectId>` / `budget_used_tokens:i64` / `budget_used_calls:i32` / `proposals_count:i32` / `proposals_eligible_count:i32`。

**Proposal**(models.rs:4434,28 字段):`id:Option<ObjectId>` / `experiment_id:String` / `workspace_id:String` / `account_id:String` / `proposal_kind:String` / `status:String` / `gate_key:Option<String>` / `current_value:Option<f64>` / `proposed_value:Option<f64>` / `cohort_notes:Document` / `proposed_template_key:Option<String>` / `proposed_section:Option<String>` / `diff_summary:Option<String>` / `diff_snippet:Option<String>` / `critic_reasoning:Option<String>` / `expected_improvement_on:Vec<String>` / `risk_note:Option<String>` / `previous_prompt_version:Option<String>` / `eval_metrics:Document` / `eval_replays_completed:i32` / `eval_replays_failed:i32` / `significance_passed:Option<bool>` / `failure_reason:Option<String>` / `released_at:Option<DateTime>` / `released_by:Option<String>` / `rolled_back_at:Option<DateTime>` / `rolled_back_by:Option<String>` / `created_at:DateTime` / `updated_at:DateTime`。

**ShadowReplay**(models.rs:4492,19 字段):`id:Option<ObjectId>` / `proposal_id:ObjectId` / `experiment_id:String` / `workspace_id:String` / `account_id:String` / `source_run_id:ObjectId` / `status:String` / `failure_reason:Option<String>` / `original_final_review_status:Option<String>` / `original_5gate_hit:Document` / `original_self_critique_addressed:Option<bool>` / `new_final_review_status:Option<String>` / `new_review_risks:Vec<String>` / `new_token_cost:Option<i64>` / `new_5gate_hit:Document` / `new_self_critique_addressed:Option<bool>` / `similarity_to_original_text:f64` / `started_at:DateTime` / `finished_at:Option<DateTime>`。现有测试 :788 完整构造可参照。

---

## 共享实现约定(所有后端 task 适用)

- **插入位置**:evolution.rs 已有 `#[cfg(test)] mod tests { use super::*; ... }`(:711 起,模块闭合 `}` 在文件末尾)。契约测试函数**追加进现有 `mod tests` 内部**,放在该模块最后一个 `}` 之前,**不新建模块**。因 `use super::*` 已透传投影函数名,契约测试**无需**额外 `use super::xxx_json;`。
- **调用形态**:投影都吃**引用**(`&Model`),构造 `let model = Model{...};` 后传 `&model`(experiment_summary 额外吃 `Vec<Proposal>` 传 owned vec)。`let value = <投影>(&model);` → `crate::routes::contract_snapshot::assert_contract_fixture("<name>", value);`。
- **局部 use**:测试函数体内需 `DateTime`/`ObjectId`/`doc!`,在函数体首行局部 `use mongodb::bson::{oid::ObjectId, DateTime, doc};`(注:mod tests 顶部 `use super::*` 可能已透传部分,但局部 use 更稳妥;若编译报 unused/重复,按编译器提示删冗余)。
- **bless 流程**:先写测试 → `export CARGO_TARGET_DIR=... && UPDATE_SNAPSHOTS=1 cargo test --lib <test_name>` bless → 再 `cargo test --lib <test_name>` 只读对账确认绿。
- **Document 字段构造**:cohort_notes/eval_metrics/original_5gate_hit/new_5gate_hit/significance_metrics 全用**纯标量** doc!(只放 number/string/bool),如 `doc!{"hitRate": 0.012}` / `doc!{"fact_risk_block": true}`。绝不放 DateTime/ObjectId/嵌套数组对象。
- **worktree 争用预案**:若 cargo 报 binary clobber / "Blocking waiting for file lock" / 测试 0 命中,遵「本地资源受限走 CI」纪律——个体测试趁空窗 touch 源文件强制重编亲跑留证,不假绿;记入 SDD 账本。
- **完成纪律**:每个 task 做完务必 ① commit ② 写报告 ③ 返回明确 status(DONE+commit hash),不要停在"等待轮询"。

---

> **全 8 个后端 task 改同一文件 evolution.rs 的同一 `mod tests`**:SDD 串行 dispatch,每个 task commit 后文件变化。implementer **务必先 Read evolution.rs 当前 mod tests 末尾再编辑**(用磁盘级插入,别依赖陈旧行号);worktree 共享 target 可能 binary clobber,遵争用预案。

### Task 1: runtime_flag_json 契约快照

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`,:711 起)
- Create(bless): `frontend/src/contracts/runtime_flag.fixture.json`

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture`;投影 `runtime_flag_json(f: &EvolutionRuntimeFlag) -> Value`(:699,不改)。
- Produces: fixture `runtime_flag.fixture.json`(7 顶层键),供 Task 9 前端对账。

- [ ] **Step 1: 在 evolution.rs 的 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:runtime_flag_json。EvolutionRuntimeFlag 7 字段全量构造
    /// (updated_by 给 Some);rolloutPercent=rollout_percent_clamped() 方法值 +
    /// rolloutPercentRaw=原始字段双发;id 不下发。投影下发 7 顶层键。
    #[test]
    fn runtime_flag_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let f = EvolutionRuntimeFlag {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            enabled: true,
            rollout_percent: 25,
            updated_by: Some("admin-1".to_string()),
            threshold_auto_release_enabled: false,
            updated_at: DateTime::from_millis(1_700_000_000_000),
        };
        let value = runtime_flag_json(&f);
        crate::routes::contract_snapshot::assert_contract_fixture("runtime_flag", value);
    }
```

- [ ] **Step 2: bless** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib runtime_flag_json_matches_contract_fixture`(写出 fixture)。

- [ ] **Step 3: 只读对账 + 核对** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib runtime_flag_json_matches_contract_fixture`(PASS)。核对 `runtime_flag.fixture.json`:7 键(enabled/rolloutPercent/rolloutPercentRaw/thresholdAutoReleaseEnabled/updatedAt/updatedBy/workspaceId);**无 id**(投影不下发);rolloutPercent=25(clamped);updatedAt RFC3339;无 $oid/$date。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/runtime_flag.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): runtime_flag 投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: threshold_override_json 契约快照

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/threshold_override.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `threshold_override_json(o: &ThresholdOverride) -> Value`(:527,不改)。
- Produces: fixture `threshold_override.fixture.json`(8 顶层键),供 Task 9 前端对账。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:threshold_override_json。ThresholdOverride 9 字段全量构造
    /// (rolled_back_at/rolled_back_by 两 Option 给 Some);id→Option.map(to_hex);
    /// source_proposal_id→to_hex()(非 Option)。投影下发 8 顶层键。
    #[test]
    fn threshold_override_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let o = ThresholdOverride {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            gate_key: "fact_risk_block".to_string(),
            value: 5.5,
            source_proposal_id: ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap(),
            released_at: DateTime::from_millis(1_700_000_000_000),
            released_by: "admin-1".to_string(),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_100_000)),
            rolled_back_by: Some("admin-2".to_string()),
        };
        let value = threshold_override_json(&o);
        crate::routes::contract_snapshot::assert_contract_fixture("threshold_override", value);
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib threshold_override_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib threshold_override_json_matches_contract_fixture`(PASS)。核对 `threshold_override.fixture.json`:8 键(id/gateKey/value/sourceProposalId/releasedAt/releasedBy/rolledBackAt/rolledBackBy);**无 workspaceId/accountId**(投影不下发);id+sourceProposalId 都是 hex;releasedAt/rolledBackAt RFC3339;无 $oid/$date。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/threshold_override.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): threshold_override 投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: threshold_override_audit_json 契约快照

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/threshold_override_audit.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `threshold_override_audit_json(a: &ThresholdOverrideAudit) -> Value`(:684,不改)。
- Produces: fixture `threshold_override_audit.fixture.json`(10 顶层键),供 Task 9 前端对账。

**关键点**:significance_metrics 是 `Option<Document>`,投影用 `.as_ref().map(bson_doc_to_json)` → Some 时是对象、None 时 null。本 task 给 **Some(纯标量 doc!)** 暴露该键非 null。现有测试 :762 已有完整构造可参照(但它给 significance_metrics: None——本契约测试要改成 Some)。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:threshold_override_audit_json。ThresholdOverrideAudit 12 字段全量构造
    /// (previous_value/new_value/hit_rate_observed 给 Some;significance_metrics 给
    /// Some(纯标量 doc!) 暴露 significanceMetrics 非 null);id→Option.map(to_hex);
    /// source_proposal_id→to_hex()。投影下发 10 顶层键。
    #[test]
    fn threshold_override_audit_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let a = ThresholdOverrideAudit {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            gate_key: "fact_risk_block".to_string(),
            action: "auto_released".to_string(),
            previous_value: Some(6.0),
            new_value: Some(5.5),
            source_proposal_id: ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap(),
            decided_by: "evolution_auto_release".to_string(),
            decided_at: DateTime::from_millis(1_700_000_000_000),
            hit_rate_observed: Some(0.012),
            significance_metrics: Some(doc! { "pValue": 0.03, "sampleSize": 120 }),
        };
        let value = threshold_override_audit_json(&a);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "threshold_override_audit",
            value,
        );
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib threshold_override_audit_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib threshold_override_audit_json_matches_contract_fixture`(PASS)。核对 `threshold_override_audit.fixture.json`:10 键(id/gateKey/action/previousValue/newValue/sourceProposalId/decidedBy/decidedAt/hitRateObserved/significanceMetrics);**无 workspaceId/accountId**(投影不下发);significanceMetrics 是对象 `{pValue,sampleSize}` 非 null;id+sourceProposalId hex;decidedAt RFC3339;**无 $oid/$date/$numberInt 泄漏**(significanceMetrics 纯标量)。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/threshold_override_audit.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): threshold_override_audit 投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: experiment_envelope_json 契约快照

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/experiment_envelope.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `experiment_envelope_json(exp: &Experiment) -> Value`(:416,不改)。
- Produces: fixture `experiment_envelope.fixture.json`(14 顶层键),供 Task 9 前端对账。

**关键点**:cohortThresholdSize/cohortPromptSize 是 `Vec<ObjectId>.len() as i64`(下发的是**长度数字**非数组,所以即使 Vec 含 ObjectId 也不泄漏);finished_at 是 Option 给 Some;id 不下发。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:experiment_envelope_json。Experiment 15 字段全量构造
    /// (finished_at 给 Some;两 cohort Vec 非空——投影只下发 .len() 数字不泄漏 ObjectId);
    /// id 不下发。投影下发 14 顶层键。
    #[test]
    fn experiment_envelope_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let exp = Experiment {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            status: "evaluating".to_string(),
            window_hours: 24,
            started_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            finished_at: Some(DateTime::from_millis(1_700_000_200_000)),
            cohort_threshold_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap()],
            cohort_prompt_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439013").unwrap()],
            budget_used_tokens: 45000,
            budget_used_calls: 120,
            proposals_count: 5,
            proposals_eligible_count: 2,
        };
        let value = experiment_envelope_json(&exp);
        crate::routes::contract_snapshot::assert_contract_fixture("experiment_envelope", value);
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib experiment_envelope_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib experiment_envelope_json_matches_contract_fixture`(PASS)。核对 `experiment_envelope.fixture.json`:14 键(experimentId/workspaceId/accountId/status/windowHours/startedAt/updatedAt/finishedAt/cohortThresholdSize/cohortPromptSize/budgetUsedTokens/budgetUsedCalls/proposalsCount/proposalsEligibleCount);**无 id**;cohortThresholdSize=1/cohortPromptSize=1(数字非数组);三时间戳 RFC3339;**无 $oid/$date 泄漏**(Vec<ObjectId> 只取 .len())。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/experiment_envelope.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): experiment_envelope 投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: proposal_summary_json 契约快照

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/proposal_summary.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `proposal_summary_json(p: &Proposal) -> Value`(:435,不改)。
- Produces: fixture `proposal_summary.fixture.json`(14 顶层键),供 Task 9 前端对账。

**关键点**:Proposal 28 字段全构造(proposal_summary 只下发其中 14 键,proposal_detail 下发 28 键;两 task 复用同一构造形态)。cohort_notes/eval_metrics 是 Document,proposal_summary 不下发它们但全字段构造仍须赋值(纯标量 doc! 或空 `Document::new()`——summary 不下发故空亦可,但为与 Task 7 一致建议纯标量)。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:proposal_summary_json。Proposal 28 字段全量构造(各 Option 给 Some、
    /// expected_improvement_on 非空 Vec、cohort_notes/eval_metrics 纯标量 doc!);
    /// id→Option.map(to_hex)。投影下发 14 顶层键(summary 子集)。
    #[test]
    fn proposal_summary_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let p = Proposal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "eligible_for_release".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(5.5),
            cohort_notes: doc! { "hitRate": 0.012 },
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("收紧 fact_risk 阈值".to_string()),
            diff_snippet: Some("- 6.0\n+ 5.5".to_string()),
            critic_reasoning: Some("命中率证据充分".to_string()),
            expected_improvement_on: vec!["fact_accuracy".to_string()],
            risk_note: Some("低风险".to_string()),
            previous_prompt_version: Some("v11".to_string()),
            eval_metrics: doc! { "pValue": 0.03 },
            eval_replays_completed: 20,
            eval_replays_failed: 1,
            significance_passed: Some(true),
            failure_reason: Some("none".to_string()),
            released_at: Some(DateTime::from_millis(1_700_000_300_000)),
            released_by: Some("admin-1".to_string()),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_400_000)),
            rolled_back_by: Some("admin-2".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = proposal_summary_json(&p);
        crate::routes::contract_snapshot::assert_contract_fixture("proposal_summary", value);
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib proposal_summary_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib proposal_summary_json_matches_contract_fixture`(PASS)。核对 `proposal_summary.fixture.json`:14 键(id/kind/status/gateKey/proposedTemplateKey/proposedSection/currentValue/proposedValue/significancePassed/evalReplaysCompleted/evalReplaysFailed/failureReason/createdAt/updatedAt);id hex;createdAt/updatedAt RFC3339;无 $oid/$date 泄漏。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/proposal_summary.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): proposal_summary 投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: shadow_replay_json 契约快照

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/shadow_replay.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `shadow_replay_json(r: &ShadowReplay) -> Value`(:507,不改)。
- Produces: fixture `shadow_replay.fixture.json`(15 顶层键),供 Task 9 前端对账。

**关键点**:original_5gate_hit/new_5gate_hit 是 Document,用纯标量 doc!(全 bool,现有测试 :800 同款);source_run_id→`to_hex()`(非 Option);new_review_risks 非空 Vec;现有测试 :788 完整构造可参照。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:shadow_replay_json。ShadowReplay 19 字段全量构造(各 Option 给 Some、
    /// new_review_risks 非空 Vec、两 5gate Document 纯标量 bool doc!);
    /// id→Option.map(to_hex);source_run_id→to_hex()。投影下发 15 顶层键。
    #[test]
    fn shadow_replay_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let r = ShadowReplay {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            proposal_id: ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap(),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            source_run_id: ObjectId::parse_str("507f1f77bcf86cd799439013").unwrap(),
            status: "completed".to_string(),
            failure_reason: Some("none".to_string()),
            original_final_review_status: Some("held_by_ai_policy".to_string()),
            original_5gate_hit: doc! { "fact_risk_block": true, "pressure_risk_block": false },
            original_self_critique_addressed: Some(false),
            new_final_review_status: Some("approved".to_string()),
            new_review_risks: vec!["minor_tone".to_string()],
            new_token_cost: Some(321),
            new_5gate_hit: doc! { "fact_risk_block": false, "pressure_risk_block": false },
            new_self_critique_addressed: Some(true),
            similarity_to_original_text: 0.85,
            started_at: DateTime::from_millis(1_700_000_000_000),
            finished_at: Some(DateTime::from_millis(1_700_000_100_000)),
        };
        let value = shadow_replay_json(&r);
        crate::routes::contract_snapshot::assert_contract_fixture("shadow_replay", value);
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib shadow_replay_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib shadow_replay_json_matches_contract_fixture`(PASS)。核对 `shadow_replay.fixture.json`:15 键(id/sourceRunId/status/failureReason/originalFinalReviewStatus/original5gateHit/originalSelfCritiqueAddressed/newFinalReviewStatus/newReviewRisks/newTokenCost/new5gateHit/newSelfCritiqueAddressed/similarityToOriginalText/startedAt/finishedAt);id+sourceRunId hex;original5gateHit/new5gateHit 是纯标量 bool 对象;startedAt/finishedAt RFC3339;**无 $oid/$date 泄漏**。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/shadow_replay.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): shadow_replay 投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: proposal_detail_json 契约快照(本批最重之一,28 键)

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/proposal_detail.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `proposal_detail_json(p: &Proposal) -> Value`(:454,不改)。
- Produces: fixture `proposal_detail.fixture.json`(28 顶层键),供 Task 9 前端对账。

**关键点**:Proposal 28 字段全构造(与 Task 5 同一构造形态——可直接复用 Task 5 的 Proposal 构造代码);cohort_notes/eval_metrics 是 Document,proposal_detail **下发** cohortNotes/evalMetrics(bson_doc_to_json),必须纯标量 doc! 避泄漏。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:proposal_detail_json。Proposal 28 字段全量构造(同 Task5 形态;各 Option
    /// 给 Some、expected_improvement_on 非空、cohort_notes/eval_metrics 纯标量 doc!——
    /// detail 下发 cohortNotes/evalMetrics 故必须纯标量避泄漏);id→Option.map(to_hex)。
    /// 投影下发 28 顶层键。
    #[test]
    fn proposal_detail_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let p = Proposal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "eligible_for_release".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(5.5),
            cohort_notes: doc! { "hitRate": 0.012 },
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("收紧 fact_risk 阈值".to_string()),
            diff_snippet: Some("- 6.0\n+ 5.5".to_string()),
            critic_reasoning: Some("命中率证据充分".to_string()),
            expected_improvement_on: vec!["fact_accuracy".to_string()],
            risk_note: Some("低风险".to_string()),
            previous_prompt_version: Some("v11".to_string()),
            eval_metrics: doc! { "pValue": 0.03 },
            eval_replays_completed: 20,
            eval_replays_failed: 1,
            significance_passed: Some(true),
            failure_reason: Some("none".to_string()),
            released_at: Some(DateTime::from_millis(1_700_000_300_000)),
            released_by: Some("admin-1".to_string()),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_400_000)),
            rolled_back_by: Some("admin-2".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = proposal_detail_json(&p);
        crate::routes::contract_snapshot::assert_contract_fixture("proposal_detail", value);
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib proposal_detail_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib proposal_detail_json_matches_contract_fixture`(PASS)。核对 `proposal_detail.fixture.json`:28 键(id/experimentId/workspaceId/accountId/kind/status/gateKey/currentValue/proposedValue/cohortNotes/proposedTemplateKey/proposedSection/diffSummary/diffSnippet/criticReasoning/expectedImprovementOn/riskNote/previousPromptVersion/evalMetrics/evalReplaysCompleted/evalReplaysFailed/significancePassed/failureReason/releasedAt/releasedBy/rolledBackAt/rolledBackBy/createdAt/updatedAt——以实际 Object.keys 为准);cohortNotes/evalMetrics 是纯标量对象;id hex;四时间戳 RFC3339;**无 $oid/$date/$numberInt 泄漏**(cohort_notes/eval_metrics 纯标量)。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/proposal_detail.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): proposal_detail 投影契约快照(批次4,28 键)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: experiment_summary_json 契约快照(聚合,顶层 3 键)

**Files:**
- Modify: `src/routes/evolution.rs`(追加契约测试进现有 `mod tests`)
- Create(bless): `frontend/src/contracts/experiment_summary.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `experiment_summary_json(exp: &Experiment, proposals: Vec<Proposal>) -> Value`(:395,不改)。
- Produces: fixture `experiment_summary.fixture.json`(3 顶层键),供 Task 9 前端对账。

**关键点**:**聚合投影**,吃 `(&Experiment, Vec<Proposal>)` 两参数。顶层只 3 键:experiment(嵌套 experiment_envelope_json,14 子键)/proposalsCounts(BTreeMap 按 proposal.status 计数,如 `{eligibleForRelease:1}`)/proposals(数组,每元素 proposal_summary_json)。对账只锁这 3 顶层键,嵌套内部不展开。构造 1 个 Experiment + 含 1 个 Proposal 的 Vec(proposals 非空)。

- [ ] **Step 1: 在 `mod tests` 末尾追加契约测试**

```rust
    /// 契约快照:experiment_summary_json。聚合投影吃 (&Experiment, Vec<Proposal>)。
    /// 顶层 3 键:experiment(嵌套 envelope)/proposalsCounts(BTreeMap status 计数)/
    /// proposals(数组)。构造 1 Experiment + 含 1 Proposal 的非空 Vec。
    #[test]
    fn experiment_summary_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let exp = Experiment {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            status: "evaluating".to_string(),
            window_hours: 24,
            started_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            finished_at: Some(DateTime::from_millis(1_700_000_200_000)),
            cohort_threshold_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap()],
            cohort_prompt_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439013").unwrap()],
            budget_used_tokens: 45000,
            budget_used_calls: 120,
            proposals_count: 1,
            proposals_eligible_count: 1,
        };
        let p = Proposal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439014").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "eligible_for_release".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(5.5),
            cohort_notes: doc! { "hitRate": 0.012 },
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("收紧 fact_risk 阈值".to_string()),
            diff_snippet: Some("- 6.0\n+ 5.5".to_string()),
            critic_reasoning: Some("命中率证据充分".to_string()),
            expected_improvement_on: vec!["fact_accuracy".to_string()],
            risk_note: Some("低风险".to_string()),
            previous_prompt_version: Some("v11".to_string()),
            eval_metrics: doc! { "pValue": 0.03 },
            eval_replays_completed: 20,
            eval_replays_failed: 1,
            significance_passed: Some(true),
            failure_reason: Some("none".to_string()),
            released_at: Some(DateTime::from_millis(1_700_000_300_000)),
            released_by: Some("admin-1".to_string()),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_400_000)),
            rolled_back_by: Some("admin-2".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = experiment_summary_json(&exp, vec![p]);
        crate::routes::contract_snapshot::assert_contract_fixture("experiment_summary", value);
    }
```

- [ ] **Step 2: bless** — `... UPDATE_SNAPSHOTS=1 cargo test --lib experiment_summary_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib experiment_summary_json_matches_contract_fixture`(PASS)。核对 `experiment_summary.fixture.json`:**3 顶层键**(experiment/proposals/proposalsCounts);experiment 是嵌套对象(14 子键);proposals 是数组(1 元素,14 子键);proposalsCounts 是对象 `{eligibleForRelease:1}`;**无 $oid/$date 泄漏**(嵌套都走 envelope/summary 投影,id→hex、时间→RFC3339)。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evolution.rs frontend/src/contracts/experiment_summary.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): experiment_summary 聚合投影契约快照(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: 前端 8 投影键集对账 harness

**Files:**
- Create: `frontend/src/contracts/runtimeFlag.contract.ts`、`thresholdOverride.contract.ts`、`thresholdOverrideAudit.contract.ts`、`experimentEnvelope.contract.ts`、`proposalSummary.contract.ts`、`shadowReplay.contract.ts`、`proposalDetail.contract.ts`、`experimentSummary.contract.ts`(8 份 `CANONICAL_KEYS`)
- Create: `frontend/src/__tests__/contracts/evolutionDomain.contract.test.ts`(8 投影双向对账)

**Interfaces:**
- Consumes: Task 1-8 bless 出的 8 份 `<name>.fixture.json`。
- Produces: 8 个 vitest 对账测试(进 frontend-contract CI job)。

**关键纪律**:`CANONICAL_KEYS` 每个键**逐字抄自对应 bless 出的 fixture**(读真相源,不靠本计划标注的键数)。先读全部 8 份 fixture 取真实顶层键(canonicalize 已字母序),再写 `as const`。模板照搬批次1/2/3 的 `assertKeysMatch`(双向集合比对 + 非空断言)。

> experiment_summary 的 experiment/proposals、proposal_detail 的 cohortNotes/evalMetrics、shadow_replay 的 5gateHit 等是嵌套对象/数组,但 CANONICAL_KEYS 只列**顶层键**(各算一个顶层键不展开)。

- [ ] **Step 1: 读 8 份 fixture 取真实顶层键**

读这 8 份(Task 1-8 生成):runtime_flag / threshold_override / threshold_override_audit / experiment_envelope / proposal_summary / shadow_replay / proposal_detail / experiment_summary。用 `node -e "console.log(JSON.stringify(Object.keys(require('./<name>.fixture.json')).sort()))"` 取每份顶层键。

- [ ] **Step 2: 写 8 份 contract.ts**

每份形如(以 runtime_flag 为例,键**抄自 fixture**;顶部一行中文注释说明是哪个后端投影):

```typescript
// frontend/src/contracts/runtimeFlag.contract.ts
// 后端 runtime_flag_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "enabled",
  "rolloutPercent",
  "rolloutPercentRaw",
  "thresholdAutoReleaseEnabled",
  "updatedAt",
  "updatedBy",
  "workspaceId",
] as const;
```

其余 7 份同构,文件名 camelCase、键抄自各自 fixture。预期键数(以 fixture 实际为准):runtimeFlag 7 / thresholdOverride 8 / thresholdOverrideAudit 10 / experimentEnvelope 14 / proposalSummary 14 / shadowReplay 15 / proposalDetail 28 / experimentSummary 3(experiment/proposals/proposalsCounts)。

- [ ] **Step 3: 写 evolutionDomain.contract.test.ts**

```typescript
import { describe, it, expect } from "vitest";
import runtimeFlagFixture from "../../contracts/runtime_flag.fixture.json";
import thresholdOverrideFixture from "../../contracts/threshold_override.fixture.json";
import thresholdOverrideAuditFixture from "../../contracts/threshold_override_audit.fixture.json";
import experimentEnvelopeFixture from "../../contracts/experiment_envelope.fixture.json";
import proposalSummaryFixture from "../../contracts/proposal_summary.fixture.json";
import shadowReplayFixture from "../../contracts/shadow_replay.fixture.json";
import proposalDetailFixture from "../../contracts/proposal_detail.fixture.json";
import experimentSummaryFixture from "../../contracts/experiment_summary.fixture.json";
import { CANONICAL_KEYS as RUNTIME_FLAG_KEYS } from "../../contracts/runtimeFlag.contract";
import { CANONICAL_KEYS as THRESHOLD_OVERRIDE_KEYS } from "../../contracts/thresholdOverride.contract";
import { CANONICAL_KEYS as THRESHOLD_OVERRIDE_AUDIT_KEYS } from "../../contracts/thresholdOverrideAudit.contract";
import { CANONICAL_KEYS as EXPERIMENT_ENVELOPE_KEYS } from "../../contracts/experimentEnvelope.contract";
import { CANONICAL_KEYS as PROPOSAL_SUMMARY_KEYS } from "../../contracts/proposalSummary.contract";
import { CANONICAL_KEYS as SHADOW_REPLAY_KEYS } from "../../contracts/shadowReplay.contract";
import { CANONICAL_KEYS as PROPOSAL_DETAIL_KEYS } from "../../contracts/proposalDetail.contract";
import { CANONICAL_KEYS as EXPERIMENT_SUMMARY_KEYS } from "../../contracts/experimentSummary.contract";

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

describe("契约: 进化/实验域投影键集对账", () => {
  it("runtime_flag 投影", () =>
    assertKeysMatch("runtimeFlag", runtimeFlagFixture, RUNTIME_FLAG_KEYS));
  it("threshold_override 投影", () =>
    assertKeysMatch("thresholdOverride", thresholdOverrideFixture, THRESHOLD_OVERRIDE_KEYS));
  it("threshold_override_audit 投影", () =>
    assertKeysMatch("thresholdOverrideAudit", thresholdOverrideAuditFixture, THRESHOLD_OVERRIDE_AUDIT_KEYS));
  it("experiment_envelope 投影", () =>
    assertKeysMatch("experimentEnvelope", experimentEnvelopeFixture, EXPERIMENT_ENVELOPE_KEYS));
  it("proposal_summary 投影", () =>
    assertKeysMatch("proposalSummary", proposalSummaryFixture, PROPOSAL_SUMMARY_KEYS));
  it("shadow_replay 投影", () =>
    assertKeysMatch("shadowReplay", shadowReplayFixture, SHADOW_REPLAY_KEYS));
  it("proposal_detail 投影(28 键)", () =>
    assertKeysMatch("proposalDetail", proposalDetailFixture, PROPOSAL_DETAIL_KEYS));
  it("experiment_summary 聚合投影(顶层 3 键)", () =>
    assertKeysMatch("experimentSummary", experimentSummaryFixture, EXPERIMENT_SUMMARY_KEYS));
});
```

- [ ] **Step 4: 跑 tsc + vitest** — `cd frontend && npx tsc --noEmit && npx vitest run src/__tests__/contracts/evolutionDomain.contract.test.ts`。Expected: tsc 0 错(无关既有错如实记录不改);8 个对账测试全 PASS。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/contracts/runtimeFlag.contract.ts frontend/src/contracts/thresholdOverride.contract.ts frontend/src/contracts/thresholdOverrideAudit.contract.ts frontend/src/contracts/experimentEnvelope.contract.ts frontend/src/contracts/proposalSummary.contract.ts frontend/src/contracts/shadowReplay.contract.ts frontend/src/contracts/proposalDetail.contract.ts frontend/src/contracts/experimentSummary.contract.ts frontend/src/__tests__/contracts/evolutionDomain.contract.test.ts
git commit -m "$(cat <<'EOF'
feat(contract): 进化/实验域 8 投影前端键集对账 harness(批次4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: ALLOWLIST 移除 8 投影 + cohort_run_ids 改归 helper + 防腐烂 lint 验证 + 批次收口

**Files:**
- Modify: `src/routes/contract_snapshot.rs`(ALLOWLIST 移除本批 8 项 + cohort_run_ids_json 改归 helper 豁免区加注释)

**Interfaces:**
- Consumes: Task 1-8 的后端测试(让 lint 的 covered 集含这 8 投影)。
- Produces: 防腐烂 lint 真正强制本批 8 投影;cohort_run_ids_json 留豁免但分类正确。

**说明**:批次1/2/3 把本批 9 项进化域投影列入 ALLOWLIST 占位。Task 1-8 已为 8 个对象投影配契约测试,现移除其豁免。cohort_run_ids_json 是裸数组 helper(非对象投影,见事实表),**保留豁免但移到 helper 区**并加准确注释。

- [ ] **Step 1: 先 Read 当前 ALLOWLIST 确认 9 项进化域投影的确切文本**

Read `src/routes/contract_snapshot.rs` 的 ALLOWLIST(约 :111-138)。当前进化域 9 项:experiment_summary_json / experiment_envelope_json / proposal_summary_json / proposal_detail_json / cohort_run_ids_json / shadow_replay_json / threshold_override_json / threshold_override_audit_json / runtime_flag_json。

- [ ] **Step 2: 移除 8 个对象投影 + cohort_run_ids 改归 helper 区**

删除这 8 行(连同行尾逗号):`"experiment_summary_json"`、`"experiment_envelope_json"`、`"proposal_summary_json"`、`"proposal_detail_json"`、`"shadow_replay_json"`、`"threshold_override_json"`、`"threshold_override_audit_json"`、`"runtime_flag_json"`。

把 `"cohort_run_ids_json"` 从进化域区移到 helper 豁免区(与 bson_from_json/bson_doc_to_json 同组),改注释为:
```rust
            "cohort_run_ids_json",     // helper:返回裸数组(json!([hex...]))非对象投影,无顶层键集;形状由 proposal 详情端点 cohortRunIds 键间接覆盖
```

保留其余批次5 域项(suspected_deal_json/outbox_entry_json/evaluation_scenario_json/playbook_json/prompt_template_json)不动。

- [ ] **Step 3: 跑防腐烂 lint** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib every_projection_has_contract_test`。Expected: PASS(8 投影都被各自测试 `assert_contract_fixture` 窗口覆盖;cohort_run_ids 仍豁免;无 orphans)。

- [ ] **Step 4: 跑全量 lib baseline** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib 2>&1 | tail -5`。Expected: `test result: ok. N passed; 0 failed`,N = 批次3 后基线 1720 + 本批 8 = 1728。

> 若本地撞 worktree 共享 target 争用,遵「本地资源受限走 CI」纪律:不假绿,个体 task 测试趁空窗已亲跑留证,全量 baseline 留 CI baseline gate 验证。记入 SDD 账本。

- [ ] **Step 5: Commit**

```bash
git add src/routes/contract_snapshot.rs
git commit -m "$(cat <<'EOF'
feat(contract): 防腐烂 lint 强制进化/实验域 8 投影(批次4 ALLOWLIST 移除 + cohort_run_ids 归 helper)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review(写完计划后自查)

**Spec 覆盖**:本批对应 spec §5 第 4 批(进化/实验域 evolution.rs)。spec §5 列 7 项(experiment_summary/proposal_{summary,detail}/shadow_replay/threshold_override/threshold_override_audit/runtime_flag),本批额外覆盖 experiment_envelope(被 summary 嵌套但 list 端点独立用,应各配),共 8 对象投影;cohort_run_ids 按 spec §4.2(裸数组/inline 非实体投影)归 helper 豁免。机制复用 §3.3/§3.4 双门 + §6 防腐烂 lint(Task10)+ §4.3 raw Document 容差(纯标量 doc!)。CI §7 不需改。

**Placeholder 扫描**:无 TBD/TODO;每个后端 task 给全字段构造代码;前端 task 给 assertKeysMatch 全文 + contract.ts 模板;键集明确「抄自 fixture」并给核对预期。

**类型一致性**:8 投影 + 6 model(EvolutionRuntimeFlag:1301/ThresholdOverride:4529/ThresholdOverrideAudit:4557/Experiment:4405/Proposal:4434/ShadowReplay:4492)字段名/类型逐行核对自 models.rs;投影体逐字核对自 evolution.rs;datetime_to_rfc3339/bson_doc_to_json 私有 helper 行为已核对(纯标量不泄漏、空 Document→{})。Task5 与 Task7 复用同一 Proposal 构造形态(字段名一致)。

**已知风险与对策**:
1. 8 task 改同一文件同一 mod tests——已在 task 区开头标注 implementer 先 Read 当前状态再磁盘级插入(批次2 同坑教训)。
2. Document 字段(cohort_notes/eval_metrics/5gate_hit/significance_metrics)纯标量 doc! 避泄漏——已在每个相关 task 关键点 + Step3 核对标注。
3. cohort_run_ids 裸数组不纳入——已全面审查确认归 helper,Task10 改注释。
4. experiment_summary 聚合吃 2 参数、顶层仅 3 键——Task8 关键点明确。
5. 投影"不下发"字段(runtime_flag/experiment_envelope 的 id、threshold_override/audit 的 workspaceId/accountId)——各 task Step3 核对明确 fixture 应无这些键。

**键数核对预期**(以 fixture 为最终真相源):runtime_flag 7 / threshold_override 8 / threshold_override_audit 10 / experiment_envelope 14 / proposal_summary 14 / shadow_replay 15 / proposal_detail 28 / experiment_summary 3。

