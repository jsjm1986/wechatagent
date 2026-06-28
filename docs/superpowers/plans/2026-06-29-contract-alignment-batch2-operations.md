# 前后端契约对齐 批次2(运营/Agent 域)实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `src/routes/shared.rs` + `outcome_metrics.rs` + `behavior_signal_metrics.rs` 的 9 个运营/Agent 域投影函数各补一个契约快照测试 + 前端键集对账,并从防腐烂 lint 的 ALLOWLIST 移除它们,使 lint 真正强制这批投影。

**Architecture:** 完全复用批次1 已落地的机制(`src/routes/contract_snapshot.rs` 的 `assert_contract_fixture`/`canonicalize`):后端 `#[cfg(test)]` 测试构造全量 model → 调投影 → 写/读 `frontend/src/contracts/<name>.fixture.json`;前端 vitest 导入同一份 fixture + `CANONICAL_KEYS` 双向对账。批次1 的知识域 4 投影对账在 `knowledgeDomain.contract.test.ts`,本批新建 `operationsDomain.contract.test.ts` 平行扩展。

**Tech Stack:** Rust 2021 + `serde_json::json!` + `mongodb::bson`(DateTime/ObjectId/Document);前端 Vitest + TypeScript `as const`。

## Global Constraints

逐字遵守(每个 task 隐含):

- 仅在用户明确要求时 commit(本 SDD 执行前用户已授权 commit+push+PR+merge,沿用)。
- 测试 only,绝不为过测试改业务逻辑/prompt/guards/阈值(过拟合红线)。投影函数本体一字不改。
- 新增测试只增量叠加,不删改旧维度/旧断言。
- `cargo test --lib` baseline ≥350/0 + 4 PBT 累计 ≥33/0 不退。
- 跑 cargo 前 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`。
- fixture 路径由 `assert_contract_fixture` 用 `env!("CARGO_MANIFEST_DIR")` 定位,无需手拼。
- 对账只断言**键集合**,不断言语义/值/可选性。
- `CANONICAL_KEYS` 仅服务对账测试,**不**改 `types/index.ts` 业务类型、**不**强塞既有组件。
- 子 agent 一律 `model: "opus"`。
- 回复用中文。
- **raw Document/嵌套桥接字段的 fixture 必须 bless 生成(`UPDATE_SNAPSHOTS=1`),绝不手写**——嵌套 BSON DateTime/ObjectId 会序列化成 `{$date}`/`{$oid}`,手写必错。bless 后肉眼核对 fixture 内容合理再提交。
- **禁词纪律(no-human-takeover lint 硬门)**:fixture JSON / 测试代码 / 注释里**绝不出现** `人工/接管/takeover/hand-off/human_handoff` 等禁词(lint 扫 `frontend/src/` + `src/routes/` 新增行)。本批构造只用 AI-内部状态名(如 `ai_hold_cleared_rate`/`autonomy_mode:"auto"`/`final_review_status:"approved_sent"`),**不要**写 `AgentOutcomeMetric` 的历史 serde alias `"human_handoff_success_rate"`(用新字段名 `ai_hold_cleared_rate`)。本计划文档里的"肉眼核对"等字仅为指令,不进代码。
- 键数以 **bless 出的 fixture 为唯一真相源**,本计划标注的键数是核对预期,若 bless 结果不同以 fixture 为准(批次1 document"16键"笔误教训)。

## 关键事实(已逐行核对 src/models.rs + 投影函数体,非靠摘要)

9 个投影 + 入参 model 精确字段数 + 投影下发键数:

| # | 投影 | 文件:行 | model(字段数,Default?) | 下发键数 | 特殊点 |
|---|---|---|---|---|---|
| 1 | `behavior_signal_metric_json` | behavior_signal_metrics.rs:65 | `BehaviorSignalMetric`(7,无) | 8 | `id:String`直发;无Document |
| 2 | `outcome_metric_json` | outcome_metrics.rs:74 | `AgentOutcomeMetric`(12,无) | 11 | `id:String`;workspace_id不下发;4个`Option<f64>` |
| 3 | `llm_call_log_json` | shared.rs:1032 | `LlmCallLog`(18,无) | 16 | id→hex;account_id/contact_wxid是Option;retry_count/final_status不下发 |
| 4 | `memory_candidate_json` | shared.rs:1015 | `MemoryCandidate`(12,无) | 12 | id→hex;`candidates:Vec<Document>`桥接 |
| 5 | `operating_memory_json` | shared.rs:966 | `OperatingMemory`(16,无) | 12 | id→hex;4个Document+memoryCard(helper);context_pack系列不下发 |
| 6 | `agent_run_json` | shared.rs:1091 | `AgentRunLog`(**35**,无) | 15 | id→hex;6个Document桥接;**model字段极多必须全填** |
| 7 | `decision_review_json` | shared.rs:1053 | `AgentDecisionReview`(29,无)+2参数 | 29 | id→hex;9个Document桥接;final_review_status/hold_category是函数参数 |
| 8 | `guide_preview_json` | shared.rs:937 | `UserOperationGuidePreview`(17,无) | 17 | id→hex;contact_id→hex;health嵌套{scores,items};suggested_changes桥接 |
| 9 | `operation_health_json` | shared.rs:447 | `&Contact`(44,无)+`&OperatingMemory`+`Option<&AgentDecisionReview>` | 2 | 聚合:scores(7键i32)+items(7项×5键);调health_scores_document/health_items_from_scores |

**通用变换**:`dt_to_string(dt)->Option<String>` 让所有 DateTime(含非Option)序列化成 `string|null`;`id:Option<ObjectId>` → `.map(|id| id.to_hex()).unwrap_or_default()`(None→空串)。

**模板参照**:批次1 知识域测试在 `src/routes/knowledge/mod.rs:1407-1534`(model 全量构造 + `DateTime::from_millis(固定值)` + `assert_contract_fixture(name, projection(model))`);前端对账在 `frontend/src/__tests__/contracts/knowledgeDomain.contract.test.ts`(`assertKeysMatch` 双向比对)。

**测试落点**:`shared.rs` 的 6 个投影测试统一放 `src/routes/shared.rs` 现有 `mod tests`(已存在,见 :1494 `guide_preview_json_builds_health_items_with_correct_risk_tone`);`outcome_metrics.rs`/`behavior_signal_metrics.rs` 各新建 `#[cfg(test)] mod contract_tests`。

**ALLOWLIST 移除**:9 个投影名当前在 `src/routes/contract_snapshot.rs:134-146`,Task 11 统一移除全部 9 项 + lint 真绿。

---

## 文件结构

- **Modify**: `src/routes/behavior_signal_metrics.rs` — 末尾加 `#[cfg(test)] mod contract_tests`(1 测试)
- **Modify**: `src/routes/outcome_metrics.rs` — 末尾加 `#[cfg(test)] mod contract_tests`(1 测试)
- **Modify**: `src/routes/shared.rs` — 现有 `mod tests` 内加 6 个契约测试
- **Modify**: `src/routes/contract_snapshot.rs` — ALLOWLIST 逐项移除本批 9 投影(Task 11)
- **Create**: `frontend/src/contracts/behavior_signal_metric.fixture.json` 等 9 份 fixture(bless 生成)
- **Create**: `frontend/src/contracts/behaviorSignalMetric.contract.ts` 等 9 份 `CANONICAL_KEYS`
- **Create**: `frontend/src/__tests__/contracts/operationsDomain.contract.test.ts` — 9 投影双向对账

---

### Task 1: behavior_signal_metric 契约快照

**Files:**
- Modify: `src/routes/behavior_signal_metrics.rs`(末尾加 `#[cfg(test)] mod contract_tests`)
- Create: `frontend/src/contracts/behavior_signal_metric.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture(name: &str, value: serde_json::Value)`(批次1 已落地)、投影 `behavior_signal_metric_json(item: BehaviorSignalMetric) -> Value`(同文件 :65,本 task 不改它)。
- Produces: fixture `behavior_signal_metric.fixture.json`(8 键),供 Task 10 前端对账。

`BehaviorSignalMetric` 全字段(models.rs:713,7 字段,无 Default,全部显式赋值):
`id:String`、`workspace_id:String`、`date:String`、`persisted:i64`、`dedupe_skipped:i64`、`errors:i64`、`last_success_at:Option<DateTime>`、`updated_at:DateTime`。

- [ ] **Step 1: 在 behavior_signal_metrics.rs 末尾追加测试模块**

```rust
#[cfg(test)]
mod contract_tests {
    use super::*;
    use mongodb::bson::DateTime;

    /// 契约快照：behavior_signal_metric_json 线上形状钉死到 fixture。
    /// 全量赋值（每个 Option 给 Some、DateTime 用固定值），调投影 → assert_contract_fixture。
    #[test]
    fn behavior_signal_metric_json_matches_contract_fixture() {
        let item = BehaviorSignalMetric {
            id: "ws-1:2026-06-29".to_string(),
            workspace_id: "ws-1".to_string(),
            date: "2026-06-29".to_string(),
            persisted: 42,
            dedupe_skipped: 3,
            errors: 1,
            last_success_at: Some(DateTime::from_millis(1_700_000_000_000)),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = behavior_signal_metric_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "behavior_signal_metric",
            projected,
        );
    }
}
```

- [ ] **Step 2: 运行测试确认 fixture 缺失而失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib behavior_signal_metric_json_matches_contract_fixture`
Expected: FAIL,panic 信息含「契约 fixture 缺失」+ bless 指令。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib behavior_signal_metric_json_matches_contract_fixture`
Expected: PASS,生成 `frontend/src/contracts/behavior_signal_metric.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture 合理**

Read `frontend/src/contracts/behavior_signal_metric.fixture.json`,确认:8 个键(`date`/`dedupeSkipped`/`errors`/`id`/`lastSuccessAt`/`persisted`/`updatedAt`/`workspaceId`,canonicalize 后字母序);`lastSuccessAt`/`updatedAt` 是 RFC3339 字符串(非 `{$date}` 包装);无多余键。

- [ ] **Step 5: 只读对账复跑确认稳定绿**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib behavior_signal_metric_json_matches_contract_fixture`
Expected: PASS（只读对账,不写文件）。

- [ ] **Step 6: Commit**

```bash
git add src/routes/behavior_signal_metrics.rs frontend/src/contracts/behavior_signal_metric.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): behavior_signal_metric 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: outcome_metric 契约快照

**Files:**
- Modify: `src/routes/outcome_metrics.rs`(末尾加 `#[cfg(test)] mod contract_tests`)
- Create: `frontend/src/contracts/outcome_metric.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `outcome_metric_json(item: AgentOutcomeMetric) -> Value`(同文件 :74)。
- Produces: fixture `outcome_metric.fixture.json`(11 键)。

`AgentOutcomeMetric` 全字段(models.rs:3105,12 字段,无 Default;注意投影**不下发** `workspace_id`):
`id:String`、`workspace_id:String`、`account_id:String`、`horizon:String`、`date:String`、`reply_rate:Option<f64>`、`conversation_depth:Option<f64>`、`ai_hold_cleared_rate:Option<f64>`、`agent_block_rate:Option<f64>`、`daily_run_count:i64`、`daily_run_token_total:i64`、`created_at:DateTime`。

- [ ] **Step 1: 追加测试模块**

```rust
#[cfg(test)]
mod contract_tests {
    use super::*;
    use mongodb::bson::DateTime;

    /// 契约快照：outcome_metric_json。4 个 Option<f64> 全给 Some（穿透 number 分支）；
    /// workspace_id 赋值但投影不下发（fixture 应无 workspaceId 键，证明投影过滤生效）。
    #[test]
    fn outcome_metric_json_matches_contract_fixture() {
        let item = AgentOutcomeMetric {
            id: "acc-1:7d:2026-06-29".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            horizon: "7d".to_string(),
            date: "2026-06-29".to_string(),
            reply_rate: Some(0.42),
            conversation_depth: Some(3.5),
            ai_hold_cleared_rate: Some(0.8),
            agent_block_rate: Some(0.1),
            daily_run_count: 120,
            daily_run_token_total: 45000,
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = outcome_metric_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("outcome_metric", projected);
    }
}
```

- [ ] **Step 2: 运行确认 fixture 缺失失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib outcome_metric_json_matches_contract_fixture`
Expected: FAIL（fixture 缺失）。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib outcome_metric_json_matches_contract_fixture`
Expected: PASS,生成 `outcome_metric.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:11 键（`accountId`/`agentBlockRate`/`aiHoldClearedRate`/`conversationDepth`/`createdAt`/`dailyRunCount`/`dailyRunTokenTotal`/`date`/`horizon`/`id`/`replyRate`,字母序);**无 `workspaceId`**(投影不下发);4 个 rate 是 number 非 null;`createdAt` 是 RFC3339 字符串。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib outcome_metric_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/outcome_metrics.rs frontend/src/contracts/outcome_metric.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): outcome_metric 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: llm_call_log 契约快照

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/llm_call_log.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `llm_call_log_json(item: LlmCallLog) -> Value`(shared.rs:1032)。
- Produces: fixture `llm_call_log.fixture.json`(16 键)。

`LlmCallLog` 全字段(models.rs:2969,18 字段,无 Default;投影**不下发** `retry_count`/`final_status`):
`id:Option<ObjectId>`、`workspace_id:String`、`account_id:Option<String>`、`contact_wxid:Option<String>`、`run_id:Option<String>`、`prompt_key:String`、`model:String`、`status:String`、`latency_ms:i64`、`prompt_tokens:i64`、`completion_tokens:i64`、`total_tokens:i64`、`prompt_cache_hit_tokens:i64`、`prompt_cache_miss_tokens:i64`、`error:Option<String>`、`retry_count:i32`、`final_status:Option<String>`、`created_at:DateTime`。

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

放在现有 `mod tests` 内(与 `guide_preview_json_builds_health_items_with_correct_risk_tone` 同级)。`mod tests` 顶部已 `use super::*;`,但 model 类型需显式引入:

```rust
    /// 契约快照：llm_call_log_json。id 走 ObjectId→hex;account_id/contact_wxid 是
    /// Option（给 Some 穿透 string 分支）;retry_count/final_status 赋值但投影不下发。
    #[test]
    fn llm_call_log_json_matches_contract_fixture() {
        use super::llm_call_log_json;
        use crate::models::LlmCallLog;
        use mongodb::bson::{oid::ObjectId, DateTime};

        let item = LlmCallLog {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899c001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: Some("acc-1".to_string()),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: Some("run-1".to_string()),
            prompt_key: "user.reply".to_string(),
            model: "provider-a".to_string(),
            status: "success".to_string(),
            latency_ms: 1200,
            prompt_tokens: 800,
            completion_tokens: 200,
            total_tokens: 1000,
            prompt_cache_hit_tokens: 600,
            prompt_cache_miss_tokens: 200,
            error: Some("none".to_string()),
            retry_count: 1,
            final_status: Some("success".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = llm_call_log_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("llm_call_log", projected);
    }
```

- [ ] **Step 2: 运行确认 fixture 缺失失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib llm_call_log_json_matches_contract_fixture`
Expected: FAIL（fixture 缺失）。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib llm_call_log_json_matches_contract_fixture`
Expected: PASS,生成 `llm_call_log.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:16 键;**无 `retryCount`/`finalStatus`**;`id` 是 hex 字符串 `"64a1f2c3e4b5a6978899c001"`(非 `{$oid}`);`createdAt` 是 RFC3339 字符串;`error`/`accountId` 等 Option 是字符串值。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib llm_call_log_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/llm_call_log.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): llm_call_log 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: memory_candidate 契约快照

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/memory_candidate.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `memory_candidate_json(item: MemoryCandidate) -> Value`(shared.rs:1015)。
- Produces: fixture `memory_candidate.fixture.json`(12 键)。

`MemoryCandidate` 全字段(models.rs:1351,12 字段,无 Default):
`id:Option<ObjectId>`、`workspace_id:String`、`account_id:String`、`contact_wxid:String`、`run_id:Option<String>`、`source:String`、`candidates:Vec<Document>`、`memory_write_score:i32`、`status:String`、`reason:Option<String>`、`created_at:DateTime`、`updated_at:DateTime`。

**桥接说明**:`candidates:Vec<Document>` 直接进 `json!`。fixture 里 candidates 是数组,内含我们 `doc!{}` 的内容。**只放纯标量(String/i32),不放 ObjectId/DateTime**——否则会泄漏 `{$oid}`/`{$date}`(对账只锁顶层键集,但 fixture 要干净可读)。

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

```rust
    /// 契约快照：memory_candidate_json。candidates:Vec<Document> 桥接,放纯标量
    /// （String/i32）避免 BSON 包装泄漏;id 走 ObjectId→hex。
    #[test]
    fn memory_candidate_json_matches_contract_fixture() {
        use super::memory_candidate_json;
        use crate::models::MemoryCandidate;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let item = MemoryCandidate {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899d001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: "wxid_abc".to_string(),
            run_id: Some("run-1".to_string()),
            source: "consolidator".to_string(),
            candidates: vec![doc! { "text": "客户偏好下午沟通", "confidence": 8i32 }],
            memory_write_score: 7,
            status: "pending".to_string(),
            reason: Some("高价值事实".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = memory_candidate_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("memory_candidate", projected);
    }
```

- [ ] **Step 2: 运行确认 fixture 缺失失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib memory_candidate_json_matches_contract_fixture`
Expected: FAIL（fixture 缺失）。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib memory_candidate_json_matches_contract_fixture`
Expected: PASS,生成 `memory_candidate.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:12 顶层键;`id` 是 hex 非 `{$oid}`;`candidates` 是数组,元素是 `{"text":...,"confidence":8}` 纯 JSON(**无 `$` 包装**——证明纯标量 Document 桥接干净);`createdAt`/`updatedAt` 是 RFC3339 字符串。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib memory_candidate_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/memory_candidate.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): memory_candidate 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: operating_memory 契约快照

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/operating_memory.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `operating_memory_json(memory: OperatingMemory) -> Value`(shared.rs:966)。
- Produces: fixture `operating_memory.fixture.json`(12 键)。

`OperatingMemory` 全字段(models.rs:1323,16 字段,无 Default;投影**不下发** `context_pack`/`context_pack_version`/`context_pack_updated_at`/`created_at`):
`id:Option<ObjectId>`、`workspace_id:String`、`account_id:String`、`contact_wxid:String`、`user_understanding:Document`、`relationship_state:Document`、`product_fit:Document`、`next_action:Document`、`context_pack:Document`、`context_pack_version:i32`、`context_pack_updated_at:Option<DateTime>`、`memory_card:MemoryCardTyped`、`memory_card_version:i32`、`memory_card_updated_at:Option<DateTime>`、`created_at:DateTime`、`updated_at:DateTime`。

**memoryCard 分支说明**(投影调 `effective_route_memory_card(&memory)`,shared.rs:983):若 `memory_card` 非空走 typed;否则 `context_pack` 非空走 from_document;**都空 → 第三分支 default skeleton**(键 coreProfile/relationshipState/preferences/doNotDo/commitments/objections/openLoops/recentEpisodeSummary/conflicts)。本 task 让 `memory_card: MemoryCardTyped::default()`(空)+ `context_pack: Document::new()`(空)→ **走第三分支 skeleton**(全确定、无需构造 MemoryFact)。对账只锁顶层 12 键,memoryCard 内部形状不影响顶层键集,但 fixture 整体确定可读。

**4 个下发 Document(`user_understanding`/`relationship_state`/`product_fit`/`next_action`)只放纯标量**避免泄漏。

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

```rust
    /// 契约快照：operating_memory_json。memory_card/context_pack 都空 → memoryCard
    /// 走 default skeleton 分支（确定形状）;4 个下发 Document 放纯标量;id→hex。
    /// context_pack 系列 + created_at 赋值但投影不下发。
    #[test]
    fn operating_memory_json_matches_contract_fixture() {
        use super::operating_memory_json;
        use crate::models::{MemoryCardTyped, OperatingMemory};
        use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

        let memory = OperatingMemory {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899e001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: "wxid_abc".to_string(),
            user_understanding: doc! { "identity": "企业主", "businessContext": "餐饮连锁" },
            relationship_state: doc! { "trustLevel": "high", "temperature": "warm" },
            product_fit: doc! { "fitReason": "需要私域自动化" },
            next_action: doc! { "action": "follow_up", "due": "2026-07-01" },
            context_pack: Document::new(),
            context_pack_version: 0,
            context_pack_updated_at: None,
            memory_card: MemoryCardTyped::default(),
            memory_card_version: 3,
            memory_card_updated_at: Some(DateTime::from_millis(1_700_000_050_000)),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = operating_memory_json(memory);
        crate::routes::contract_snapshot::assert_contract_fixture("operating_memory", projected);
    }
```

- [ ] **Step 2: 运行确认 fixture 缺失失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operating_memory_json_matches_contract_fixture`
Expected: FAIL（fixture 缺失）。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib operating_memory_json_matches_contract_fixture`
Expected: PASS,生成 `operating_memory.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:12 顶层键(`accountId`/`contactWxid`/`id`/`memoryCard`/`memoryCardUpdatedAt`/`memoryCardVersion`/`nextAction`/`productFit`/`relationshipState`/`updatedAt`/`userUnderstanding`/`workspaceId`);**无 `contextPack`/`createdAt`**;`id` hex 非 `{$oid}`;4 个 Document 是纯 JSON 无 `$` 包装;`memoryCard` 是 skeleton 对象(coreProfile 等键);`memoryCardUpdatedAt`/`updatedAt` 是 RFC3339 字符串。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operating_memory_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/operating_memory.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): operating_memory 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: agent_run 契约快照（AgentRunLog 35 字段，本批最重）

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/agent_run.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `agent_run_json(item: AgentRunLog) -> Value`(shared.rs:1091)。
- Produces: fixture `agent_run.fixture.json`(15 键)。

**⚠️ 关键**:`AgentRunLog`(models.rs:2631→2740)有 **35 个字段**,无 Default,必须全部显式赋值,漏一个 E0063 编译失败。投影只下发 15 键,其余 20 字段赋值但不下发(证明投影过滤)。6 个下发 Document(`planner`/`context`/`knowledge_route`/`decision`/`review`/`gateway_result`)只放纯标量避免泄漏。

下方构造代码已含全部 35 字段(逐行核对自 models.rs:2631-2740),直接照抄:

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

```rust
    /// 契约快照：agent_run_json。AgentRunLog 38 字段全量构造（无 Default）;6 个
    /// 下发 Document 放纯标量;投影只下发 15 键,其余 23 字段不下发。
    #[test]
    fn agent_run_json_matches_contract_fixture() {
        use super::agent_run_json;
        use crate::models::AgentRunLog;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let item = AgentRunLog {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899f001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: "run-1".to_string(),
            trigger_kind: "inbound_message".to_string(),
            status: "completed".to_string(),
            planner: doc! { "step": "plan", "n": 1i32 },
            context: doc! { "loaded": true },
            knowledge_route: doc! { "matched": 2i32 },
            decision: doc! { "action": "reply" },
            review: doc! { "approved": true },
            gateway_result: doc! { "status": "sent" },
            error: Some("none".to_string()),
            token_budget: 8000,
            tokens_used: 1200,
            llm_calls_used: 3,
            degraded_reasons: vec!["none".to_string()],
            lifecycle: "completed".to_string(),
            source_event_id: "evt-1".to_string(),
            source_kind: "inbound_message".to_string(),
            error_summary: Some("ok".to_string()),
            abort_reason: Some("none".to_string()),
            revision_applied: false,
            revision_reason: "none".to_string(),
            pre_revision_summary: Some("before".to_string()),
            post_revision_summary: Some("after".to_string()),
            self_critique: Some("looks good".to_string()),
            autonomy_mode: "auto".to_string(),
            conversation_mode: "consultative".to_string(),
            conversation_mode_reason: Some("customer_stage:proposal_evaluation".to_string()),
            final_review_status: "approved_sent".to_string(),
            outbox_status: Some("sent".to_string()),
            memory_consolidator_warnings: vec!["none".to_string()],
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = agent_run_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("agent_run", projected);
    }
```

> **校验提示**:若 Step 2 报 E0063 missing field,说明 AgentRunLog 字段有增删——以编译器报的字段名为准补齐(models.rs:2631 起),不要猜。本计划构造已对照 :2631-2740 全字段。

- [ ] **Step 2: 运行确认 fixture 缺失失败（且编译通过）**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib agent_run_json_matches_contract_fixture`
Expected: FAIL,panic 含「契约 fixture 缺失」(**不是** E0063——若 E0063 先补字段)。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib agent_run_json_matches_contract_fixture`
Expected: PASS,生成 `agent_run.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:15 顶层键(`accountId`/`context`/`contactWxid`/`createdAt`/`decision`/`error`/`gatewayResult`/`id`/`knowledgeRoute`/`planner`/`review`/`runId`/`status`/`triggerKind`/`workspaceId`);**无 `tokenBudget`/`lifecycle`/`autonomyMode`/`finalReviewStatus` 等 20 个不下发字段**;`id` hex 非 `{$oid}`;6 个 Document 纯 JSON 无 `$` 包装;`createdAt` RFC3339。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib agent_run_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/agent_run.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): agent_run 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: decision_review 契约快照（29 键 + 2 函数参数）

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/decision_review.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `decision_review_json(review: AgentDecisionReview, final_review_status: Option<String>, hold_category: Option<String>) -> Value`(shared.rs:1053)。
- Produces: fixture `decision_review.fixture.json`(29 键)。

`AgentDecisionReview` 全字段(models.rs:2580,29 字段,无 Default;投影**不下发** `reaction_claimed_at`/`reviewer_misjudge_signal`):
`id:Option<ObjectId>`、`workspace_id:String`、`account_id:String`、`contact_wxid:Option<String>`、`run_id:Option<String>`、`inbound_message_id:Option<String>`、`reply_text:Option<String>`、`approved:bool`、`scores:Document`、`formula_breakdown:Document`、`risks:Vec<String>`、`rewrite_instruction:Option<String>`、`review_summary:Option<String>`、`playbook_id:Option<ObjectId>`、`playbook_version:Option<i32>`、`used_knowledge_ids:Vec<ObjectId>`、`prompt_versions:Document`、`operation_state:Option<String>`、`next_best_action:Document`、`context_pack_snapshot:Document`、`domain_config_snapshot:Document`、`runtime_parameters_snapshot:Document`、`send_gateway_result:Document`、`outcome_status:Option<String>`、`reaction_analysis:Document`、`reaction_claimed_at:Option<DateTime>`、`reviewer_misjudge_signal:Option<String>`、`status:String`、`created_at:DateTime`。

**桥接说明**:9 个下发 Document(scores/formula_breakdown/prompt_versions/next_best_action/context_pack_snapshot/domain_config_snapshot/runtime_parameters_snapshot/send_gateway_result/reaction_analysis)放纯标量。`used_knowledge_ids:Vec<ObjectId>` 投影 `.to_hex()` 成纯字符串数组(**不泄漏** `$oid`)。`playbook_id` 投影 `.map(to_hex)` → Some 时 hex 字符串。**函数参数 final_review_status/hold_category 给 Some**(None vs Some 不影响键集,都给 Some 让 fixture 值非 null)。

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

```rust
    /// 契约快照：decision_review_json（29 键）。AgentDecisionReview 29 字段全量构造;
    /// 9 个下发 Document 放纯标量;used_knowledge_ids:Vec<ObjectId>→hex 字符串数组（不泄漏）;
    /// final_review_status/hold_category 是函数参数,给 Some;reaction_claimed_at/
    /// reviewer_misjudge_signal 赋值但投影不下发。
    #[test]
    fn decision_review_json_matches_contract_fixture() {
        use super::decision_review_json;
        use crate::models::AgentDecisionReview;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let review = AgentDecisionReview {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889a0001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: Some("run-1".to_string()),
            inbound_message_id: Some("msg-1".to_string()),
            reply_text: Some("您好，已收到".to_string()),
            approved: true,
            scores: doc! { "humanLikeScore": 8i32, "pressureRisk": 2i32 },
            formula_breakdown: doc! { "weighted": "ok" },
            risks: vec!["low".to_string()],
            rewrite_instruction: Some("无需改写".to_string()),
            review_summary: Some("通过".to_string()),
            playbook_id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889a0002").unwrap()),
            playbook_version: Some(2),
            used_knowledge_ids: vec![
                ObjectId::parse_str("64a1f2c3e4b5a697889a0003").unwrap(),
            ],
            prompt_versions: doc! { "user.reply": "v2" },
            operation_state: Some("negotiation".to_string()),
            next_best_action: doc! { "action": "follow_up" },
            context_pack_snapshot: doc! { "ctx": "snap" },
            domain_config_snapshot: doc! { "domain": "user_operations" },
            runtime_parameters_snapshot: doc! { "temp": "0.7" },
            send_gateway_result: doc! { "status": "sent" },
            outcome_status: Some("replied".to_string()),
            reaction_analysis: doc! { "sentiment": "positive" },
            reaction_claimed_at: Some(DateTime::from_millis(1_700_000_050_000)),
            reviewer_misjudge_signal: Some("none".to_string()),
            status: "approved".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = decision_review_json(
            review,
            Some("approved_sent".to_string()),
            Some("none".to_string()),
        );
        crate::routes::contract_snapshot::assert_contract_fixture("decision_review", projected);
    }
```

> **校验提示**:若 Step 2 报 E0063,以编译器字段名为准补齐(models.rs:2580 起)。

- [ ] **Step 2: 运行确认 fixture 缺失失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib decision_review_json_matches_contract_fixture`
Expected: FAIL（fixture 缺失,非 E0063）。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib decision_review_json_matches_contract_fixture`
Expected: PASS,生成 `decision_review.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:29 顶层键;**无 `reactionClaimedAt`/`reviewerMisjudgeSignal`**;`id`/`playbookId` 是 hex 非 `{$oid}`;`usedKnowledgeIds` 是 hex 字符串数组(`["64a1f2c3e4b5a697889a0003"]`,**无 `$oid`**);9 个 Document 纯 JSON 无 `$` 包装;`finalReviewStatus`/`holdCategory` 是字符串(参数值);`createdAt` RFC3339。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib decision_review_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/decision_review.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): decision_review 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: guide_preview 契约快照（17 键，含 health 嵌套）

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/guide_preview.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `guide_preview_json(preview: UserOperationGuidePreview) -> Value`(shared.rs:937)。
- Produces: fixture `guide_preview.fixture.json`(17 顶层键)。

`UserOperationGuidePreview` 全字段(models.rs:2996,17 字段,无 Default;投影**不下发** `workspace_id`):
`id:Option<ObjectId>`、`workspace_id:String`、`account_id:String`、`contact_id:ObjectId`(非 Option)、`contact_wxid:String`、`instruction:String`、`mode:String`、`status:String`、`summary:String`、`impact_scope:String`、`scope_reason:String`、`readable_changes:Vec<String>`、`health_scores:Document`、`suggested_changes:Document`、`risk_warnings:Vec<String>`、`created_at:DateTime`、`updated_at:DateTime`。

**说明**:投影 `contact_id.to_hex()`(非 Option,直接 hex)。`health_scores` 给 7 个 i32 键(穿透 `health_items_from_scores`),它同时下发为顶层 `healthScores` 和嵌套 `health.scores`。`impact_scope`/`scope_reason` 给非空值(走 pass-through 分支)。`suggested_changes` 放纯标量。`health` 顶层算 1 键(值是 `{scores,items}` 嵌套)。

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

```rust
    /// 契约快照：guide_preview_json（17 顶层键）。health_scores 给 7 个 i32 键;
    /// contact_id→hex（非 Option）;impact_scope/scope_reason 非空走 pass-through;
    /// suggested_changes 纯标量;workspace_id 赋值但不下发。
    #[test]
    fn guide_preview_json_matches_contract_fixture() {
        // guide_preview_json + UserOperationGuidePreview 已由 mod tests 模块级 import
        // （shared.rs:1459/1461）。这里函数级 import 与之遮蔽兼容（同名遮蔽合法，非 E0252）。
        use crate::models::UserOperationGuidePreview;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let preview = UserOperationGuidePreview {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889b0001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: ObjectId::parse_str("64a1f2c3e4b5a697889b0002").unwrap(),
            contact_wxid: "wxid_abc".to_string(),
            instruction: "更关注客户情绪".to_string(),
            mode: "tune".to_string(),
            status: "pending".to_string(),
            summary: "测试预览".to_string(),
            impact_scope: "current_contact".to_string(),
            scope_reason: "只影响当前好友".to_string(),
            readable_changes: vec!["语气更温和".to_string()],
            health_scores: doc! {
                "userUnderstanding": 80i32,
                "relationshipQuality": 50i32,
                "productFit": 30i32,
                "rhythmRisk": 20i32,
                "knowledgeGrounding": 70i32,
                "hallucinationRisk": 10i32,
                "pressureRisk": 10i32,
            },
            suggested_changes: doc! { "tone": "warmer" },
            risk_warnings: vec!["勿过度承诺".to_string()],
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = guide_preview_json(preview);
        crate::routes::contract_snapshot::assert_contract_fixture("guide_preview", projected);
    }
```

- [ ] **Step 2: 运行确认 fixture 缺失失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib guide_preview_json_matches_contract_fixture`
Expected: FAIL（fixture 缺失）。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib guide_preview_json_matches_contract_fixture`
Expected: PASS,生成 `guide_preview.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:17 顶层键(含 `health`/`healthScores`);**无 `workspaceId`**;`id`/`contactId` 是 hex 非 `{$oid}`;`health` 是 `{scores:{7键},items:[7项]}` 嵌套;每个 item 有 `key`/`label`/`score`/`tone`/`detail` 5 键;`healthScores` 是 7 i32 键;`createdAt`/`updatedAt` RFC3339。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib guide_preview_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/guide_preview.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): guide_preview 投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: operation_health 契约快照（聚合，吃 3 个 model）

**Files:**
- Modify: `src/routes/shared.rs`(现有 `mod tests` 内加测试)
- Create: `frontend/src/contracts/operation_health.fixture.json`(bless 生成)

**Interfaces:**
- Consumes: `assert_contract_fixture`、投影 `operation_health_json(contact: &Contact, memory: &OperatingMemory, review: Option<&AgentDecisionReview>) -> Value`(shared.rs:447)。
- Produces: fixture `operation_health.fixture.json`(2 顶层键 `scores`+`items`)。

**聚合说明**:投影吃 `&Contact`/`&OperatingMemory`/`Option<&AgentDecisionReview>` 三个引用,调 `health_scores_document`(算 7 个 i32 score)+ `health_items_from_scores`(组装 7 项,每项 5 键)。下发 2 顶层键:`scores`(7 i32 键)+ `items`(7 项数组)。`review` 给 `Some` 让 knowledgeGrounding/hallucinationRisk/pressureRisk 非零(读 review.scores 的 i32 子键 `knowledgeGroundingScore`/`hallucinationScore`/`pressureRisk`)。**键集对 Some/None 不变**(只值变),给 Some 让 fixture 值更全。

**构造负担**:`Contact`(models.rs:131,44 字段,无 Default)必须全字段构造——这是本批最长构造。`OperatingMemory` 复用 Task 5 形态。`AgentDecisionReview` 复用 Task 7 形态(此处只需 review.scores 有 3 个 i32 子键,其余可同 Task 7)。

下方 Contact 构造已含全部 44 字段(逐行核对 models.rs:131-237),直接照抄。health 只读 `human_profile_note`/`domain_attributes`(customer_stage/intent_level)/`follow_up_policy`/`cooldown_until`/`last_agent_run_at`/`last_message_at`,其余字段填最小合法值(`AgentStatus` 枚举给 `Managed`,Vec 空,Option 多数 None)。

- [ ] **Step 1: 在 shared.rs 的 `mod tests` 内追加测试**

```rust
    /// 契约快照：operation_health_json（聚合 2 键 scores+items）。吃 Contact（44 字段全量
    /// 构造,无 Default）+ OperatingMemory + Some(&AgentDecisionReview)。review 给 Some
    /// 让 3 个 review-derived score 非零;键集对 Some/None 不变。
    #[test]
    fn operation_health_json_matches_contract_fixture() {
        use super::operation_health_json;
        use crate::models::{
            AgentDecisionReview, AgentStatus, Contact, MemoryCardTyped, OperatingMemory,
        };
        use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

        let contact = Contact {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889c0001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            wxid: "wxid_abc".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            agent_status: AgentStatus::Managed,
            human_profile_note: Some("企业主，关注降本".to_string()),
            custom_agent_instructions: None,
            operation_mode_override: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            manual_tags: vec![],
            manual_tags_updated_at: None,
            manual_tags_by: None,
            confirmed_tags: vec![],
            bayesian_signals: vec![],
            personality_profile: None,
            tags_version: 0,
            domain_attributes: Some(doc! { "customer_stage": "negotiation", "intent_level": "high" }),
            domain_attributes_updated_at: None,
            commitments: vec![],
            follow_up_policy: Some("每周一次".to_string()),
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: Some(DateTime::from_millis(1_700_000_200_000)),
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: Some(DateTime::from_millis(1_700_000_000_000)),
            last_outbound_style: None,
            intent_trajectory: vec![],
            outcome_events: vec![],
            locale: None,
            created_at: DateTime::from_millis(1_699_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };

        let memory = OperatingMemory {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889c0002").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: "wxid_abc".to_string(),
            user_understanding: doc! { "identity": "企业主", "businessContext": "餐饮连锁" },
            relationship_state: doc! { "trustLevel": "high", "temperature": "warm" },
            product_fit: doc! { "fitReason": "需要私域自动化" },
            next_action: doc! { "action": "follow_up" },
            context_pack: Document::new(),
            context_pack_version: 0,
            context_pack_updated_at: None,
            memory_card: MemoryCardTyped::default(),
            memory_card_version: 0,
            memory_card_updated_at: None,
            created_at: DateTime::from_millis(1_699_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };

        let review = AgentDecisionReview {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889c0003").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: Some("run-1".to_string()),
            inbound_message_id: None,
            reply_text: None,
            approved: true,
            scores: doc! {
                "knowledgeGroundingScore": 8i32,
                "hallucinationScore": 2i32,
                "pressureRisk": 3i32,
            },
            formula_breakdown: Document::new(),
            risks: vec![],
            rewrite_instruction: None,
            review_summary: None,
            playbook_id: None,
            playbook_version: None,
            used_knowledge_ids: vec![],
            prompt_versions: Document::new(),
            operation_state: None,
            next_best_action: Document::new(),
            context_pack_snapshot: Document::new(),
            domain_config_snapshot: Document::new(),
            runtime_parameters_snapshot: Document::new(),
            send_gateway_result: Document::new(),
            outcome_status: None,
            reaction_analysis: Document::new(),
            reaction_claimed_at: None,
            reviewer_misjudge_signal: None,
            status: "approved".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
        };

        let projected = operation_health_json(&contact, &memory, Some(&review));
        crate::routes::contract_snapshot::assert_contract_fixture("operation_health", projected);
    }
```

> **校验提示**:Contact / AgentDecisionReview 字段较多,若 Step 2 报 E0063 missing field,以编译器报的字段名为准补齐（models.rs:131 / :2580 起）。`AgentStatus` 枚举变体确认为 `Normal`/`Managed`（models.rs:8）。

- [ ] **Step 2: 运行确认 fixture 缺失失败（编译通过）**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operation_health_json_matches_contract_fixture`
Expected: FAIL,panic 含「契约 fixture 缺失」(非 E0063)。

- [ ] **Step 3: bless 生成 fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib operation_health_json_matches_contract_fixture`
Expected: PASS,生成 `operation_health.fixture.json`。

- [ ] **Step 4: 肉眼核对 fixture**

Read fixture,确认:2 顶层键 `items`+`scores`;`scores` 有 7 i32 键(`userUnderstanding`/`relationshipQuality`/`productFit`/`rhythmRisk`/`knowledgeGrounding`/`hallucinationRisk`/`pressureRisk`);`knowledgeGrounding`=80(8×10)/`hallucinationRisk`=20/`pressureRisk`=30(证明 review Some 穿透);`items` 是 7 项数组,每项 `detail`/`key`/`label`/`score`/`tone` 5 键;无 `$oid`/`$date`(全 i32/String)。

- [ ] **Step 5: 只读对账复跑**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operation_health_json_matches_contract_fixture`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/routes/shared.rs frontend/src/contracts/operation_health.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): operation_health 聚合投影契约快照(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: 前端 9 投影键集对账 harness

**Files:**
- Create: `frontend/src/contracts/behaviorSignalMetric.contract.ts`、`outcomeMetric.contract.ts`、`llmCallLog.contract.ts`、`memoryCandidate.contract.ts`、`operatingMemory.contract.ts`、`agentRun.contract.ts`、`decisionReview.contract.ts`、`guidePreview.contract.ts`、`operationHealth.contract.ts`(9 份 `CANONICAL_KEYS`)
- Create: `frontend/src/__tests__/contracts/operationsDomain.contract.test.ts`(9 投影双向对账)

**Interfaces:**
- Consumes: Task 1-9 bless 出的 9 份 `<name>.fixture.json`。
- Produces: 9 个 vitest 对账测试(进 frontend-contract CI job)。

**关键纪律**:`CANONICAL_KEYS` 的每个键**逐字抄自对应 bless 出的 fixture**(读真相源,不靠本计划标注的键数——计划标注是核对预期,fixture 是准)。先读全部 9 份 fixture 拿真实顶层键,再写 `as const`。

模板照搬批次1 `knowledgeDomain.contract.test.ts` 的 `assertKeysMatch`(双向集合比对 + 非空断言)。

- [ ] **Step 1: 读 9 份 fixture 取真实顶层键**

Read 这 9 份(Task 1-9 生成):`behavior_signal_metric` / `outcome_metric` / `llm_call_log` / `memory_candidate` / `operating_memory` / `agent_run` / `decision_review` / `guide_preview` / `operation_health`。记下每份 `Object.keys` 顶层键(canonicalize 已字母序)。

- [ ] **Step 2: 写 9 份 contract.ts**

每份形如(以 behavior_signal_metric 为例,键**抄自 fixture**):

```typescript
// frontend/src/contracts/behaviorSignalMetric.contract.ts
// 后端 behavior_signal_metric_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "date",
  "dedupeSkipped",
  "errors",
  "id",
  "lastSuccessAt",
  "persisted",
  "updatedAt",
  "workspaceId",
] as const;
```

其余 8 份同构,文件名 camelCase、键抄自各自 fixture。`operation_health` 的 CANONICAL_KEYS 只有顶层 2 键 `["items","scores"]`(对账只锁顶层,items/scores 内部形状由后端快照固定)。

- [ ] **Step 3: 写 operationsDomain.contract.test.ts**

```typescript
import { describe, it, expect } from "vitest";
import behaviorSignalFixture from "../../contracts/behavior_signal_metric.fixture.json";
import outcomeFixture from "../../contracts/outcome_metric.fixture.json";
import llmCallLogFixture from "../../contracts/llm_call_log.fixture.json";
import memoryCandidateFixture from "../../contracts/memory_candidate.fixture.json";
import operatingMemoryFixture from "../../contracts/operating_memory.fixture.json";
import agentRunFixture from "../../contracts/agent_run.fixture.json";
import decisionReviewFixture from "../../contracts/decision_review.fixture.json";
import guidePreviewFixture from "../../contracts/guide_preview.fixture.json";
import operationHealthFixture from "../../contracts/operation_health.fixture.json";
import { CANONICAL_KEYS as BEHAVIOR_SIGNAL_KEYS } from "../../contracts/behaviorSignalMetric.contract";
import { CANONICAL_KEYS as OUTCOME_KEYS } from "../../contracts/outcomeMetric.contract";
import { CANONICAL_KEYS as LLM_CALL_LOG_KEYS } from "../../contracts/llmCallLog.contract";
import { CANONICAL_KEYS as MEMORY_CANDIDATE_KEYS } from "../../contracts/memoryCandidate.contract";
import { CANONICAL_KEYS as OPERATING_MEMORY_KEYS } from "../../contracts/operatingMemory.contract";
import { CANONICAL_KEYS as AGENT_RUN_KEYS } from "../../contracts/agentRun.contract";
import { CANONICAL_KEYS as DECISION_REVIEW_KEYS } from "../../contracts/decisionReview.contract";
import { CANONICAL_KEYS as GUIDE_PREVIEW_KEYS } from "../../contracts/guidePreview.contract";
import { CANONICAL_KEYS as OPERATION_HEALTH_KEYS } from "../../contracts/operationHealth.contract";

// 后端投影写出的 fixture（线上真相源）与前端 CANONICAL_KEYS 双向键集对账。
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

describe("契约: 运营/Agent 域投影键集对账", () => {
  it("behavior_signal_metric 投影", () =>
    assertKeysMatch("behaviorSignal", behaviorSignalFixture, BEHAVIOR_SIGNAL_KEYS));
  it("outcome_metric 投影", () =>
    assertKeysMatch("outcome", outcomeFixture, OUTCOME_KEYS));
  it("llm_call_log 投影", () =>
    assertKeysMatch("llmCallLog", llmCallLogFixture, LLM_CALL_LOG_KEYS));
  it("memory_candidate 投影", () =>
    assertKeysMatch("memoryCandidate", memoryCandidateFixture, MEMORY_CANDIDATE_KEYS));
  it("operating_memory 投影", () =>
    assertKeysMatch("operatingMemory", operatingMemoryFixture, OPERATING_MEMORY_KEYS));
  it("agent_run 投影", () =>
    assertKeysMatch("agentRun", agentRunFixture, AGENT_RUN_KEYS));
  it("decision_review 投影", () =>
    assertKeysMatch("decisionReview", decisionReviewFixture, DECISION_REVIEW_KEYS));
  it("guide_preview 投影", () =>
    assertKeysMatch("guidePreview", guidePreviewFixture, GUIDE_PREVIEW_KEYS));
  it("operation_health 聚合投影（顶层 scores+items）", () =>
    assertKeysMatch("operationHealth", operationHealthFixture, OPERATION_HEALTH_KEYS));
});
```

- [ ] **Step 4: 跑 tsc + vitest 确认全绿**

Run: `cd frontend && npx tsc --noEmit && npx vitest run src/__tests__/contracts/operationsDomain.contract.test.ts`
Expected: tsc 0 错;9 个对账测试全 PASS。

> 若 tsc 报 `.includes(k: string)` 类型错(`as const` 窄联合),`assertKeysMatch` 已用 `[...declared]` 展开为宽数组规避——与批次1 同款修法。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/contracts/behaviorSignalMetric.contract.ts frontend/src/contracts/outcomeMetric.contract.ts frontend/src/contracts/llmCallLog.contract.ts frontend/src/contracts/memoryCandidate.contract.ts frontend/src/contracts/operatingMemory.contract.ts frontend/src/contracts/agentRun.contract.ts frontend/src/contracts/decisionReview.contract.ts frontend/src/contracts/guidePreview.contract.ts frontend/src/contracts/operationHealth.contract.ts frontend/src/__tests__/contracts/operationsDomain.contract.test.ts
git commit -m "$(cat <<'EOF'
feat(contract): 运营/Agent 域 9 投影前端键集对账 harness(批次2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: ALLOWLIST 移除 9 投影 + 防腐烂 lint 验证 + 批次收口

**Files:**
- Modify: `src/routes/contract_snapshot.rs`(ALLOWLIST 移除本批 9 项,contract_snapshot.rs:134-146)

**Interfaces:**
- Consumes: Task 1-9 的后端测试(让 lint 的 covered 集含这 9 投影)。
- Produces: 防腐烂 lint 真正强制本批 9 投影(忘配测试即红)。

**说明**:批次1 把本批 9 投影列入 ALLOWLIST 占位(注释「批次2/3/5 域投影」)。Task 1-9 既已为它们配了契约测试,现移除豁免,让 lint `every_projection_has_contract_test` 真正咬这 9 个。移除后 lint 会扫到它们,在各自测试块的 `assert_contract_fixture` 窗口内找到投影名 → covered → 不报 orphan。

- [ ] **Step 1: 从 ALLOWLIST 移除本批 9 项**

在 `src/routes/contract_snapshot.rs` 的 ALLOWLIST 删除这 9 行(连同行尾逗号):
`"operation_health_json"`、`"guide_preview_json"`、`"operating_memory_json"`、`"memory_candidate_json"`、`"llm_call_log_json"`、`"decision_review_json"`、`"agent_run_json"`、`"outcome_metric_json"`、`"behavior_signal_metric_json"`。

保留其余批次3/4/5 域项(taxonomy/evolution/config 等)+ helper 项不动。

- [ ] **Step 2: 跑防腐烂 lint 确认 9 投影被覆盖、无 orphan**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib every_projection_has_contract_test`
Expected: PASS（9 投影都在各自测试块被 `assert_contract_fixture` 覆盖;无 orphans）。

> 若报 orphans 含本批某投影:说明该投影测试的 `assert_contract_fixture("<name>",...)` 调用与 `fn <投影名>` 不在同一覆盖窗口（lint 在投影名出现处前 600/后 200 字符找 `assert_contract_fixture`）。检查测试函数体里投影名 token 是否出现(如 `let projected = <投影名>(...)` 这行)。本批每个测试都有 `let projected = <投影>(...)` 调用 → 投影名 token 在窗口内 → covered。

- [ ] **Step 3: 跑全量 lib baseline 确认不退**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`,N ≥ 350 基线 + 本批新增 9 测试(批次1 后基线 1686 → 本批约 1695)。

> 若本地撞 worktree 共享 target 争用(test binary 被 clobber / "Blocking waiting for file lock" / 新测试 0 命中),遵「本地资源受限走 CI」纪律:不假绿,个体 task 测试趁空窗已亲跑留证(Task1-9 各 Step5),全量 baseline 留 CI baseline gate 验证。记入 SDD 账本。

- [ ] **Step 4: Commit**

```bash
git add src/routes/contract_snapshot.rs
git commit -m "$(cat <<'EOF'
feat(contract): 防腐烂 lint 强制运营/Agent 域 9 投影(批次2 ALLOWLIST 移除)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review(写完计划后自查)

**Spec 覆盖**:本批对应 spec §5 第 2 批(运营/Agent 域 shared.rs)9 投影,逐一有 task(Task1-9)。机制复用 spec §3.3/§3.4 双门、§6 防腐烂 lint(Task11 移除 ALLOWLIST)、§4.3 raw Document 容差(各 task 用纯标量 Document 避泄漏)。CI §7 不需改(批次1 已建 frontend-contract job + paths-filter,本批 fixture/contract 落在已覆盖路径)。

**Placeholder 扫描**:无 TBD/TODO;每个后端 task 给全字段构造代码;前端 task 给 assertKeysMatch 全文 + contract.ts 模板;键集明确「抄自 fixture」并给核对预期。

**类型一致性**:9 个 model 字段名/类型逐行核对自 models.rs(behavior:713 / outcome:3105 / llm:2969 / candidate:1351 / memory:1323 / run:2631-2740 / review:2580 / preview:2996 / Contact:131-237);helper 行为(effective_route_memory_card 三分支 / MemoryCardTyped::default().is_empty() / dt_to_string→Option / id.to_hex())均亲自核对。投影体逐字核对(各 task 前)。

**已知风险**:①raw Document 桥接 fixture 必 bless 不手写(各 task 已标);②AgentRunLog 38 字段最易漏(Task6 标 E0063 提示);③operation_health 吃 Contact 45 字段(Task9 标 E0063 提示 + AgentStatus 枚举确认);④本地 target 争用可能挡全量验证(Task11 Step3 标走 CI)。

**键数核对预期**(以 fixture 为最终真相源):behavior 8 / outcome 11 / llm 16 / candidate 12 / memory 12 / run 15 / review 29 / preview 17 / health 2。

