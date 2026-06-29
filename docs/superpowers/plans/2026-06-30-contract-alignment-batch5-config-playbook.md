# 前后端契约对齐 批次5(配置/playbook 域) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为配置/playbook 域 5 个后端投影(playbook_json / prompt_template_json / evaluation_scenario_json / suspected_deal_json / outbox_entry_json)各配后端契约快照 + 前端键集对账,移除 ALLOWLIST 占位使防腐烂 lint 真正强制,完成五批契约对齐工程最后一批。

**Architecture:** 机制完全复用批次1/2/3/4:后端 `#[cfg(test)]` 测试构造全量 model → 调投影 `xxx_json()` → `crate::routes::contract_snapshot::assert_contract_fixture(name, value)` 写/读 `frontend/src/contracts/<name>.fixture.json`(前后端唯一真相源);前端 import 同一 fixture + 声明 `CANONICAL_KEYS` 做双向键集对账;最后从 ALLOWLIST 移除这 5 项,移除后 ALLOWLIST 只剩 6 项纯 helper/裸数组豁免。

**Tech Stack:** Rust 2021(cargo test --lib)+ React 19/Vite/TypeScript/vitest。fixture bless 用 `UPDATE_SNAPSHOTS=1`。

## Global Constraints

- 测试 only:5 个投影本体一字不改(playbook_json:438 / prompt_template_json:290 / evaluation_scenario_json:591 / suspected_deal_json:262 / outbox_entry_json:250)。diff 只应追加测试 + fixture + 前端 contract.ts + ALLOWLIST 收口。
- 过拟合红线:绝不为过测试改业务逻辑/prompt/guards/阈值/投影本体。
- 新增测试只增量叠加,不删改旧维度/旧断言(suspected_deal/outbox/evaluation 三文件已有 mod tests + 既有 shape 测试,只追加不动旧的)。
- `cargo test --lib` baseline ≥350/0 + 4 PBT 累计 ≥33/0 不退。批次4 合并后基线见 Task 8 实测(预期约 1734 + 本批 5 = 1739,实际以跑出为准——共享 worktree 并行会话可能令绝对数偏移,关键是 0 failed)。
- fixture 必须 bless 真实生成,绝不手写(嵌套 BSON 会泄漏 $oid/$date)。
- 对账只断言顶层键集合,不断言语义/值/可选性。Document/数组字段算一个顶层键不展开。
- **evaluation_scenario 的 contact_seed/ground_truth 是 Document,投影直发(未走 bson_doc_to_json)。经全面审查(见下"关键决策"):投影不改;fixture 用纯标量 doc! 构造(照搬生产种子 main.rs:382-395 的真实字段)即绝对安全。铁律:fixture 的这两个 doc! 绝不塞 ObjectId/DateTime。**
- 禁词纪律(no-human-takeover lint 硬门):fixture/测试/contract.ts/注释绝不出现 人工/接管/takeover/hand-off/human_handoff。
- 子 agent 一律 model: opus。
- commit 须用户批准(本 SDD 已获授权)。回复用中文。
- 工作目录:E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/e4-f21-closure;分支 feat/contract-alignment-batch5(从 main 883c3a3 切,批次4 已合并 PR#67)。
- cargo 命令前必须 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`。⚠️ 多 worktree 共享 target 可能令 test binary 被并行会话 clobber:只读对账诡异失败/测试"消失"时 `touch <改的源文件>` 强制重编再跑,确认真绿非假绿。

## 关键决策(执行前已全面审查,implementer 不必重新纠结)

**evaluation_scenario_json 的 contactSeed/groundTruth Document 直发** —— 经完整代码审查(写入侧 + bson_doc_to_json 行为 + serde 实证 + 前端读取侧)定论:

1. `bson_doc_to_json`(evolution.rs:545)**不是消毒器**:它 = `serde_json::to_value(Bson::Document(...))`,与 json! 直发 Document 走**同一条 serde 路径**,对同一 Document 字节级输出相同。所以"把直发改成 bson_doc_to_json 桥接"是 no-op,**不消除任何泄漏,投影无需改**。
2. 泄漏只取决于 Document 内层装什么:Int32/Int64/Double/String/Bool/数组/纯嵌套对象一律渲染干净(铁证:threshold_override_audit.fixture.json 的 `sampleSize:120` 是 Int32 渲染成干净 `120`);只有 ObjectId/DateTime 才泄漏 $oid/$date。
3. EvaluationScenario 的这两个 Document 来自 HTTP JSON 入口(axum Json 反序列化)+ 种子 doc!,**结构上不可能含 ObjectId/DateTime**——JSON 数据模型无此概念。
4. 前端只把 contactSeed/groundTruth 当不透明对象整体消费,不读子键,契约对账只锁顶层键足够。

→ **结论:投影一字不改;fixture 的 contact_seed/ground_truth 用纯标量 doc! 构造,照搬生产种子 main.rs:382-395 的真实字段形状(contact_seed={operationState,intentLevel} 字符串;ground_truth={trust,conversionReadiness,emotionalValue,nextBestActionScore} 整数)。这样 fixture 既安全又如实复刻生产。唯一纪律:绝不在 fixture 的 doc! 里塞 ObjectId/DateTime。**

## 投影事实表(已逐行核实自源码,2026-06-30)

| # | 投影 | 文件:行 | 签名 | 下发键数 | model | 特殊点 |
|---|---|---|---|---|---|---|
| 1 | playbook_json | playbooks.rs:438 | `(OperationPlaybook)` 值 | 18 | OperationPlaybook 19 字段 | 漏发 createdAt;无 Document;id→`.map(to_hex).unwrap_or_default()`(None→"");updatedAt→dt_to_string;**文件无 mod tests 需新建** |
| 2 | prompt_template_json | prompt_templates.rs:290 | `(PromptTemplate)` 值 | 13 | PromptTemplate 18 字段 | 漏发 createdAt/currentVersion/previousVersion/seededBy/locale;无 Document;**文件无 mod tests 需新建** |
| 3 | evaluation_scenario_json | evaluations.rs:591 | `(EvaluationScenario)` 值 | 12 | EvaluationScenario 13 字段 | 漏发 workspaceId;**contactSeed/groundTruth=Document 纯标量 doc!**(见关键决策);已有 mod tests :630(use super::*) |
| 4 | suspected_deal_json | admin_suspected_deals.rs:262 | `(SuspectedDealSignal)` 值(pub) | 13 | SuspectedDealSignal 13 字段 | 1:1 全发零漏;reviewedAt→`.and_then(dt_to_string)`(Option);contactId 是 String 直发非 hex;已有 mod tests :280(use super::*,有 sample_signal helper) |
| 5 | outbox_entry_json | admin_outbox.rs:250 | `(&OutboxEntry)` **引用** | 22 | OutboxEntry 25 字段 | 漏发 mediaAssetId/referralCardId/reclaimedInFlight;**decisionId=`.map(to_hex)` 无 unwrap → None 时 null(异于 id 的 "")**;已有 mod tests :277(use super::*,有 sample_entry helper) |

## model 字段速查(全字段构造用,逐字核实自 models.rs)

**OperationPlaybook**(models.rs:1049,19 字段):id:Option<ObjectId> / workspace_id:String / account_id:String / name:String / description:Option<String> / method_prompt:String / profile_method:Option<String> / tag_method:Option<String> / stage_method:Option<String> / intent_method:Option<String> / follow_up_method:Option<String> / reply_style:Option<String> / forbidden_rules:Option<String> / success_criteria:Option<String> / created_by:String / is_default:bool / version:i32 / created_at:DateTime / updated_at:DateTime。

**PromptTemplate**(models.rs:1011,18 字段):id:Option<ObjectId> / workspace_id:String / prompt_key:String / agent_kind:String / layer:String / title:String / description:Option<String> / content:String / status:String / version:i32 / prompt_pack_version:String / created_by:String / created_at:DateTime / updated_at:DateTime / current_version:bool / previous_version:Option<i32> / seeded_by:Option<String> / locale:Option<String>。

**EvaluationScenario**(models.rs:3149,13 字段):id:Option<ObjectId> / workspace_id:String / scenario_id:String / title:String / description:String / account_id:Option<String> / contact_seed:Document / inbound_messages:Vec<String> / ground_truth:Document / tags:Vec<String> / status:String / created_at:DateTime / updated_at:DateTime。

**SuspectedDealSignal**(models.rs:2952,13 字段):id:Option<ObjectId> / workspace_id:String / account_id:String / contact_id:String / value:String / evidence:Option<String> / confidence:i32 / status:String / occurrences:i32 / first_seen_at:DateTime / last_seen_at:DateTime / reviewed_at:Option<DateTime> / reviewed_by:Option<String>。

**OutboxEntry**(models.rs:2772,25 字段):id:Option<ObjectId> / workspace_id:String / account_id:String / contact_wxid:String / run_id:String / decision_id:Option<ObjectId> / source_event_id:String / source_kind:String / content:String / content_hash:String / idempotency_key:String / media_asset_id:Option<String> / referral_card_id:Option<String> / attempt:i32 / max_attempts:i32 / status:String / cancel_reason:Option<String> / last_error:Option<String> / next_retry_at:Option<DateTime> / worker_id:Option<String> / locked_until:Option<DateTime> / reclaimed_in_flight:bool / created_at:DateTime / updated_at:DateTime / sent_at:Option<DateTime>。

## 共享实现约定

- **dt_to_string**(models.rs:3364):`pub fn dt_to_string(dt: DateTime) -> Option<String>` —— 必填 DateTime 投影写 `crate::models::dt_to_string(x)`(序列化成 string),可选 DateTime 写 `x.and_then(crate::models::dt_to_string)`。fixture 里给了具体时间的键都是字符串值。
- 全字段 Some/非空:每个 model 的 Option 字段构造时给 Some(...),Vec 给非空,以最大化 fixture 形状覆盖。
- 时间戳样本统一用 `DateTime::from_millis(1_700_000_000_000)` 起(= 2023-11-14T22:13:20Z),多个时间戳递增 +100s 区分。
- 测试块内调用统一全路径:`crate::routes::contract_snapshot::assert_contract_fixture("<name>", value)`,无需 use。
- fixture 命名(投影名去 _json):`playbook` / `prompt_template` / `evaluation_scenario` / `suspected_deal` / `outbox_entry`。
- 任务 3/4/5 改的文件已有 mod tests + `use super::*`(追加测试函数进末尾即可);任务 1/2 改的文件**无 mod tests 需新建** `#[cfg(test)] mod tests { use super::*; ... }`。
- 5 个 model 都在 src/models.rs,各文件顶部是否已 import 该 model:照投影签名——签名用裸名(如 `playbook: OperationPlaybook`)即说明已 import,测试构造直接用裸名;若编译报未 import 才退回 `crate::models::Xxx`。

---

### Task 1: playbook_json 契约快照

**Files:**
- Modify: `src/routes/playbooks.rs`(**新建** `#[cfg(test)] mod tests`,因该文件无测试模块)
- Create(bless): `frontend/src/contracts/playbook.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `playbook_json(playbook: OperationPlaybook) -> Value`(:438,不改)。
- Produces: fixture `playbook.fixture.json`(18 顶层键),供 Task 6 前端对账。

**关键点**:OperationPlaybook 19 字段全构造(7 个 Option<String> 给 Some);投影下发 18 键(漏发 createdAt);无 Document 字段最干净;id→hex,updatedAt→RFC3339。文件无 mod tests,需在文件末尾新建。

- [ ] **Step 1: 在 playbooks.rs 末尾新建 mod tests 并加契约测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 契约快照:playbook_json。OperationPlaybook 19 字段全量构造(7 个 Option<String> 给 Some);
    /// id→Option.map(to_hex).unwrap_or_default();updatedAt→dt_to_string。投影下发 18 顶层键(漏发 createdAt)。
    #[test]
    fn playbook_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let playbook = OperationPlaybook {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            name: "默认销售剧本".to_string(),
            description: Some("用于高意向客户跟进".to_string()),
            method_prompt: "先共情再给方案".to_string(),
            profile_method: Some("三段式画像".to_string()),
            tag_method: Some("意向分级打标".to_string()),
            stage_method: Some("AIDA 阶段推进".to_string()),
            intent_method: Some("显式信号优先".to_string()),
            follow_up_method: Some("三天未回主动跟进".to_string()),
            reply_style: Some("简洁口语".to_string()),
            forbidden_rules: Some("不承诺无依据效果".to_string()),
            success_criteria: Some("客户主动询价".to_string()),
            created_by: "admin-1".to_string(),
            is_default: true,
            version: 3,
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = playbook_json(playbook);
        crate::routes::contract_snapshot::assert_contract_fixture("playbook", value);
    }
}
```

- [ ] **Step 2: bless** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib playbook_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib playbook_json_matches_contract_fixture`(PASS)。核对 `playbook.fixture.json`:18 键(id/workspaceId/accountId/name/description/methodPrompt/profileMethod/tagMethod/stageMethod/intentMethod/followUpMethod/replyStyle/forbiddenRules/successCriteria/createdBy/isDefault/version/updatedAt);**无 createdAt**(投影不下发);id hex;updatedAt RFC3339;无 $oid/$date 泄漏。

- [ ] **Step 4: Commit**

```bash
git add src/routes/playbooks.rs frontend/src/contracts/playbook.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): playbook 投影契约快照(批次5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: prompt_template_json 契约快照

**Files:**
- Modify: `src/routes/prompt_templates.rs`(**新建** `#[cfg(test)] mod tests`)
- Create(bless): `frontend/src/contracts/prompt_template.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `prompt_template_json(template: PromptTemplate) -> Value`(:290,不改)。
- Produces: fixture `prompt_template.fixture.json`(13 顶层键),供 Task 6 前端对账。

**关键点**:PromptTemplate 18 字段全构造(description/previous_version/seeded_by/locale 给 Some);投影只下发 13 键(漏发 createdAt/currentVersion/previousVersion/seededBy/locale 这 5 个 M4 多版本+语种内部字段);无 Document;id→hex,updatedAt→RFC3339。文件无 mod tests,需新建。

- [ ] **Step 1: 在 prompt_templates.rs 末尾新建 mod tests 并加契约测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 契约快照:prompt_template_json。PromptTemplate 18 字段全量构造(description/previous_version/
    /// seeded_by/locale 给 Some);id→Option.map(to_hex).unwrap_or_default();updatedAt→dt_to_string。
    /// 投影下发 13 顶层键(漏发 createdAt/currentVersion/previousVersion/seededBy/locale)。
    #[test]
    fn prompt_template_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let template = PromptTemplate {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            prompt_key: "reply_agent_main".to_string(),
            agent_kind: "reply".to_string(),
            layer: "policy".to_string(),
            title: "回复 Agent 主提示词".to_string(),
            description: Some("主决策层提示词".to_string()),
            content: "你是私域运营助手……".to_string(),
            status: "active".to_string(),
            version: 11,
            prompt_pack_version: "v2".to_string(),
            created_by: "system".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            current_version: true,
            previous_version: Some(10),
            seeded_by: Some("system".to_string()),
            locale: Some("zh-CN".to_string()),
        };
        let value = prompt_template_json(template);
        crate::routes::contract_snapshot::assert_contract_fixture("prompt_template", value);
    }
}
```

- [ ] **Step 2: bless** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib prompt_template_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib prompt_template_json_matches_contract_fixture`(PASS)。核对 `prompt_template.fixture.json`:13 键(id/workspaceId/promptKey/agentKind/layer/title/description/content/status/version/promptPackVersion/createdBy/updatedAt);**无 createdAt/currentVersion/previousVersion/seededBy/locale**;id hex;updatedAt RFC3339;无 $oid/$date 泄漏。

- [ ] **Step 4: Commit**

```bash
git add src/routes/prompt_templates.rs frontend/src/contracts/prompt_template.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): prompt_template 投影契约快照(批次5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: evaluation_scenario_json 契约快照

**Files:**
- Modify: `src/routes/evaluations.rs`(追加契约测试进现有 `mod tests` :630)
- Create(bless): `frontend/src/contracts/evaluation_scenario.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `evaluation_scenario_json(item: EvaluationScenario) -> Value`(:591,不改)。
- Produces: fixture `evaluation_scenario.fixture.json`(12 顶层键),供 Task 6 前端对账。

**关键点(本批唯一 Document 直发,见计划顶部"关键决策")**:contact_seed/ground_truth 是 Document,投影直发。**fixture 用纯标量 doc! 构造,照搬生产种子(main.rs:382-395)的真实字段**:contact_seed={operationState,intentLevel} 字符串;ground_truth={trust,conversionReadiness,emotionalValue,nextBestActionScore} 整数(bson Int32 渲染成干净数字不泄漏)。**绝不在 doc! 里塞 ObjectId/DateTime**。投影漏发 workspaceId。EvaluationScenario 13 字段全构造(account_id 给 Some)。已有 mod tests :630(use super::*),追加进末尾。

- [ ] **Step 1: 在 evaluations.rs 现有 mod tests(:630)末尾追加契约测试**

```rust
    /// 契约快照:evaluation_scenario_json。EvaluationScenario 13 字段全量构造
    /// (account_id 给 Some;contact_seed/ground_truth 用纯标量 doc! 照搬生产种子形状——
    /// 整数走 bson Int32 渲染干净不泄漏;铁律:绝不塞 ObjectId/DateTime);
    /// id→Option.map(to_hex).unwrap_or_default();created_at/updated_at→dt_to_string。
    /// 投影下发 12 顶层键(漏发 workspaceId)。
    #[test]
    fn evaluation_scenario_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let item = EvaluationScenario {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            scenario_id: "example_high_intent_user".to_string(),
            title: "高意向用户主动询问产品能力".to_string(),
            description: "用户主动表达需求并询问能否落地".to_string(),
            account_id: Some("acc-1".to_string()),
            contact_seed: doc! { "operationState": "need_discovery", "intentLevel": "高意向" },
            inbound_messages: vec!["AI 能不能帮忙跟进?".to_string()],
            ground_truth: doc! {
                "trust": 7,
                "conversionReadiness": 6,
                "emotionalValue": 7,
                "nextBestActionScore": 7
            },
            tags: vec!["example".to_string(), "high_intent".to_string()],
            status: "active".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = evaluation_scenario_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("evaluation_scenario", value);
    }
```

- [ ] **Step 2: bless** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib evaluation_scenario_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib evaluation_scenario_json_matches_contract_fixture`(PASS)。核对 `evaluation_scenario.fixture.json`:12 键(id/scenarioId/title/description/accountId/contactSeed/inboundMessages/groundTruth/tags/status/createdAt/updatedAt);**无 workspaceId**;contactSeed 是 `{intentLevel,operationState}` 纯标量对象;groundTruth 是 `{conversionReadiness,emotionalValue,nextBestActionScore,trust}` 纯整数对象;**全文件无 $oid/$date/$numberInt 泄漏**(这是本批最关键核对项);id hex;createdAt/updatedAt RFC3339。

- [ ] **Step 4: Commit**

```bash
git add src/routes/evaluations.rs frontend/src/contracts/evaluation_scenario.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): evaluation_scenario 投影契约快照(批次5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: suspected_deal_json 契约快照

**Files:**
- Modify: `src/routes/admin_suspected_deals.rs`(追加契约测试进现有 `mod tests` :280)
- Create(bless): `frontend/src/contracts/suspected_deal.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `suspected_deal_json(item: SuspectedDealSignal) -> Value`(:262,不改)。
- Produces: fixture `suspected_deal.fixture.json`(13 顶层键),供 Task 6 前端对账。

**关键点**:SuspectedDealSignal 13 字段全构造,投影 1:1 全发零漏。evidence/reviewed_at/reviewed_by 给 Some;contact_id 是 String 直发(非 hex 转换,sample 用 hex 串即可);reviewedAt→`.and_then(dt_to_string)`(Option,给 Some 时非 null);first_seen_at/last_seen_at→dt_to_string。已有 mod tests :280(use super::*,有 sample_signal helper——但本契约测试自建全字段构造,不复用 helper 以确保所有 Option 都 Some)。

- [ ] **Step 1: 在 admin_suspected_deals.rs 现有 mod tests(:280)末尾追加契约测试**

```rust
    /// 契约快照:suspected_deal_json。SuspectedDealSignal 13 字段全量构造(evidence/reviewed_at/
    /// reviewed_by 给 Some,reviewedAt 非 null);id→Option.map(to_hex).unwrap_or_default();
    /// contact_id 是 String 直发;first_seen_at/last_seen_at→dt_to_string,reviewed_at→and_then。
    /// 投影 1:1 下发 13 顶层键。
    #[test]
    fn suspected_deal_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let item = SuspectedDealSignal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_id: "wxid_alice".to_string(),
            value: "疑似成交·待核实".to_string(),
            evidence: Some("客户已确认付款意向".to_string()),
            confidence: 8,
            status: "pending".to_string(),
            occurrences: 3,
            first_seen_at: DateTime::from_millis(1_700_000_000_000),
            last_seen_at: DateTime::from_millis(1_700_000_100_000),
            reviewed_at: Some(DateTime::from_millis(1_700_000_200_000)),
            reviewed_by: Some("admin-1".to_string()),
        };
        let value = suspected_deal_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("suspected_deal", value);
    }
```

- [ ] **Step 2: bless** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib suspected_deal_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib suspected_deal_json_matches_contract_fixture`(PASS)。核对 `suspected_deal.fixture.json`:13 键(id/workspaceId/accountId/contactId/value/evidence/confidence/occurrences/status/firstSeenAt/lastSeenAt/reviewedAt/reviewedBy);id hex;contactId 是普通字符串 wxid_alice(非 hex 转换);reviewedAt 非 null(RFC3339);firstSeenAt/lastSeenAt RFC3339;无 $oid/$date 泄漏。

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_suspected_deals.rs frontend/src/contracts/suspected_deal.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): suspected_deal 投影契约快照(批次5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: outbox_entry_json 契约快照

**Files:**
- Modify: `src/routes/admin_outbox.rs`(追加契约测试进现有 `mod tests` :277)
- Create(bless): `frontend/src/contracts/outbox_entry.fixture.json`

**Interfaces:**
- Consumes: `assert_contract_fixture`;投影 `outbox_entry_json(entry: &OutboxEntry) -> Value`(:250,**吃引用**,不改)。
- Produces: fixture `outbox_entry.fixture.json`(22 顶层键),供 Task 6 前端对账。

**关键点**:OutboxEntry 25 字段全构造,投影下发 22 键(漏发 mediaAssetId/referralCardId/reclaimedInFlight)。**decisionId 给 Some(ObjectId) → fixture 是 hex 字符串(非 null);若给 None 则是 null——本契约给 Some 以暴露 hex 形状**。投影吃 `&OutboxEntry` 引用,调用写 `outbox_entry_json(&entry)`。多个 Option 字段(cancel_reason/last_error/worker_id 等)给 Some;3 个 Option<DateTime>(next_retry_at/locked_until/sent_at)给 Some→RFC3339。已有 mod tests :277(use super::*,有 sample_entry helper——本契约测试自建全字段构造确保所有 Option Some)。

- [ ] **Step 1: 在 admin_outbox.rs 现有 mod tests(:277)末尾追加契约测试**

```rust
    /// 契约快照:outbox_entry_json。OutboxEntry 25 字段全量构造(各 Option 给 Some;decision_id
    /// 给 Some 暴露 decisionId hex 形状;3 个 Option<DateTime> 给 Some);id→Option.map(to_hex)
    /// .unwrap_or_default(),decisionId→Option.map(to_hex)(None 则 null);必填 DateTime→dt_to_string,
    /// 可选 DateTime→and_then。投影下发 22 顶层键(漏发 mediaAssetId/referralCardId/reclaimedInFlight)。
    #[test]
    fn outbox_entry_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let entry = OutboxEntry {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: "wxid_alice".to_string(),
            run_id: "run-001".to_string(),
            decision_id: Some(ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap()),
            source_event_id: "evt-001".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "您好,已收到您的咨询".to_string(),
            content_hash: "abc123".to_string(),
            idempotency_key: "idem-001".to_string(),
            media_asset_id: Some("asset-1".to_string()),
            referral_card_id: Some("card-1".to_string()),
            attempt: 1,
            max_attempts: 3,
            status: "pending".to_string(),
            cancel_reason: Some("无".to_string()),
            last_error: Some("无".to_string()),
            next_retry_at: Some(DateTime::from_millis(1_700_000_200_000)),
            worker_id: Some("worker-1".to_string()),
            locked_until: Some(DateTime::from_millis(1_700_000_300_000)),
            reclaimed_in_flight: false,
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            sent_at: Some(DateTime::from_millis(1_700_000_400_000)),
        };
        let value = outbox_entry_json(&entry);
        crate::routes::contract_snapshot::assert_contract_fixture("outbox_entry", value);
    }
```

- [ ] **Step 2: bless** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib outbox_entry_json_matches_contract_fixture`。

- [ ] **Step 3: 只读对账 + 核对** — `... cargo test --lib outbox_entry_json_matches_contract_fixture`(PASS)。核对 `outbox_entry.fixture.json`:22 键(id/workspaceId/accountId/contactWxid/runId/decisionId/sourceEventId/sourceKind/content/contentHash/idempotencyKey/attempt/maxAttempts/status/cancelReason/lastError/nextRetryAt/workerId/lockedUntil/createdAt/updatedAt/sentAt);**无 mediaAssetId/referralCardId/reclaimedInFlight**(投影不下发);id 与 decisionId 都是 hex;nextRetryAt/lockedUntil/sentAt/createdAt/updatedAt RFC3339;无 $oid/$date 泄漏。

- [ ] **Step 4: Commit**

```bash
git add src/routes/admin_outbox.rs frontend/src/contracts/outbox_entry.fixture.json
git commit -m "$(cat <<'EOF'
feat(contract): outbox_entry 投影契约快照(批次5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 前端 5 投影键集对账 harness

**Files:**
- Create: `frontend/src/contracts/playbook.contract.ts`、`promptTemplate.contract.ts`、`evaluationScenario.contract.ts`、`suspectedDeal.contract.ts`、`outboxEntry.contract.ts`(5 份 `CANONICAL_KEYS`)
- Create: `frontend/src/__tests__/contracts/configPlaybookDomain.contract.test.ts`(5 投影双向对账)

**Interfaces:**
- Consumes: Task 1-5 bless 出的 5 份 `<name>.fixture.json`。
- Produces: 5 个 vitest 对账测试(进 frontend-contract CI job)。

**关键纪律**:`CANONICAL_KEYS` 每个键**逐字抄自对应 bless 出的 fixture**(读真相源,不靠本计划标注的键数)。先读全部 5 份 fixture 取真实顶层键(canonicalize 已字母序),再写 `as const`。模板照搬批次1/2/3/4 的 `assertKeysMatch`(双向集合比对 + 非空断言)。预期键数(以 fixture 实际为准):playbook 18 / promptTemplate 13 / evaluationScenario 12 / suspectedDeal 13 / outboxEntry 22。

- [ ] **Step 1: 读 5 份 fixture 取真实顶层键**

读这 5 份(Task 1-5 生成):playbook / prompt_template / evaluation_scenario / suspected_deal / outbox_entry。用 `node -e "console.log(JSON.stringify(Object.keys(require('./<name>.fixture.json')).sort()))"` 取每份顶层键。

- [ ] **Step 2: 写 5 份 contract.ts**

每份形如(以 playbook 为例,键**抄自 fixture**;顶部一行中文注释说明哪个后端投影):

```typescript
// frontend/src/contracts/playbook.contract.ts
// 后端 playbook_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "accountId",
  "createdBy",
  "description",
  "forbiddenRules",
  "followUpMethod",
  "id",
  "intentMethod",
  "isDefault",
  "methodPrompt",
  "name",
  "profileMethod",
  "replyStyle",
  "stageMethod",
  "successCriteria",
  "tagMethod",
  "updatedAt",
  "version",
  "workspaceId",
] as const;
```

> 上面 playbook 的键序仅为示例,**实际以 Step 1 读出的 fixture 真实键集为准**(字母序)。其余 4 份同构,文件名 camelCase(promptTemplate/evaluationScenario/suspectedDeal/outboxEntry)、键抄自各自 fixture。

- [ ] **Step 3: 写 configPlaybookDomain.contract.test.ts**

```typescript
import { describe, it, expect } from "vitest";
import playbookFixture from "../../contracts/playbook.fixture.json";
import promptTemplateFixture from "../../contracts/prompt_template.fixture.json";
import evaluationScenarioFixture from "../../contracts/evaluation_scenario.fixture.json";
import suspectedDealFixture from "../../contracts/suspected_deal.fixture.json";
import outboxEntryFixture from "../../contracts/outbox_entry.fixture.json";
import { CANONICAL_KEYS as PLAYBOOK_KEYS } from "../../contracts/playbook.contract";
import { CANONICAL_KEYS as PROMPT_TEMPLATE_KEYS } from "../../contracts/promptTemplate.contract";
import { CANONICAL_KEYS as EVALUATION_SCENARIO_KEYS } from "../../contracts/evaluationScenario.contract";
import { CANONICAL_KEYS as SUSPECTED_DEAL_KEYS } from "../../contracts/suspectedDeal.contract";
import { CANONICAL_KEYS as OUTBOX_ENTRY_KEYS } from "../../contracts/outboxEntry.contract";

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

describe("契约: 配置/playbook 域投影键集对账", () => {
  it("playbook 投影", () =>
    assertKeysMatch("playbook", playbookFixture, PLAYBOOK_KEYS));
  it("prompt_template 投影", () =>
    assertKeysMatch("promptTemplate", promptTemplateFixture, PROMPT_TEMPLATE_KEYS));
  it("evaluation_scenario 投影", () =>
    assertKeysMatch("evaluationScenario", evaluationScenarioFixture, EVALUATION_SCENARIO_KEYS));
  it("suspected_deal 投影", () =>
    assertKeysMatch("suspectedDeal", suspectedDealFixture, SUSPECTED_DEAL_KEYS));
  it("outbox_entry 投影", () =>
    assertKeysMatch("outboxEntry", outboxEntryFixture, OUTBOX_ENTRY_KEYS));
});
```

- [ ] **Step 4: 跑 tsc + vitest** — `cd frontend && npx tsc --noEmit && npx vitest run src/__tests__/contracts/configPlaybookDomain.contract.test.ts`。Expected: tsc 0 错(无关既有错如实记录不改);5 个对账测试全 PASS。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/contracts/playbook.contract.ts frontend/src/contracts/promptTemplate.contract.ts frontend/src/contracts/evaluationScenario.contract.ts frontend/src/contracts/suspectedDeal.contract.ts frontend/src/contracts/outboxEntry.contract.ts frontend/src/__tests__/contracts/configPlaybookDomain.contract.test.ts
git commit -m "$(cat <<'EOF'
feat(contract): 配置/playbook 域 5 投影前端键集对账 harness(批次5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: ALLOWLIST 移除 5 投影 + 防腐烂 lint 验证 + 五批工程收口

**Files:**
- Modify: `src/routes/contract_snapshot.rs`(ALLOWLIST 移除本批 5 项 + 删批次注释行)

**Interfaces:**
- Consumes: Task 1-5 的后端测试(让 lint 的 covered 集含这 5 投影)。
- Produces: 防腐烂 lint 真正强制全部域投影;移除后 ALLOWLIST 只剩 6 项纯 helper/裸数组豁免,五批契约对齐工程完成。

**说明**:批次1/2/3/4 把本批 5 项配置/playbook 域投影列入 ALLOWLIST 占位。Task 1-5 已为这 5 个投影配契约测试,现移除其豁免。移除后 ALLOWLIST 应只剩 6 项:bson_from_json/bson_doc_to_json/parse_warning_to_json/vision_generate_json/lesson_doc_to_json/cohort_run_ids_json(全是 helper/非 model→Value/裸数组,永久豁免)。

- [ ] **Step 1: 先 Read 当前 ALLOWLIST 确认确切文本**

Read `src/routes/contract_snapshot.rs` 的 ALLOWLIST(约 :111-125)。当前本批 5 项:suspected_deal_json / outbox_entry_json / evaluation_scenario_json / playbook_json / prompt_template_json,上方有两行"批次5 域投影:..."注释。

- [ ] **Step 2: 移除 5 个投影行 + 上方两行批次注释**

删除这 5 行(连同行尾逗号):`"suspected_deal_json"`、`"outbox_entry_json"`、`"evaluation_scenario_json"`、`"playbook_json"`、`"prompt_template_json"`。同时删除它们上方的两行注释:
```rust
            // 批次5 域投影:本批次只覆盖知识域 + 运营/Agent 域 + 字典/分类域 + 进化/实验域,配置/playbook 域在后续批次纳入。
            // 批次铺开时从本清单移除对应项,使 lint 真正强制。
```
保留 cohort_run_ids_json(helper 区)及前 5 项 helper 不动。移除后 ALLOWLIST 恰 6 项。

- [ ] **Step 3: 跑防腐烂 lint** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib every_projection_has_contract_test`。Expected: PASS(本批 5 投影都被各自测试 assert_contract_fixture 窗口覆盖;无 orphans;ALLOWLIST 仅剩 helper)。

- [ ] **Step 4: 跑全量 lib baseline** — `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib 2>&1 | tail -5`。Expected: `test result: ok. N passed; 0 failed`,N = 批次4 后基线 + 本批 5。绝对数以实测为准(共享 worktree 并行会话可能偏移),关键 0 failed。

> 若本地撞 worktree 共享 target 争用,遵「本地资源受限走 CI」纪律:不假绿,个体 task 测试趁空窗已亲跑留证,全量 baseline 留 CI baseline gate 验证。记入 SDD 账本。

- [ ] **Step 5: Commit**

```bash
git add src/routes/contract_snapshot.rs
git commit -m "$(cat <<'EOF'
feat(contract): 防腐烂 lint 强制配置/playbook 域 5 投影(批次5 ALLOWLIST 收口,五批工程完成)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review(写完计划后自查)

**1. Spec 覆盖**:本批对应 spec 第 5 批(配置/playbook 域)。5 个投影(playbook/prompt_template/evaluation_scenario/suspected_deal/outbox_entry)各一个后端 task(Task 1-5)+ 前端 harness(Task 6)+ ALLOWLIST 收口(Task 7)。这 5 项是 ALLOWLIST 里剩下的最后一批域投影,移除后只剩 6 项 helper——五批工程闭环。覆盖完整无gap。

**2. Placeholder 扫描**:无 TBD/TODO。每个后端 task 给全字段构造代码(5 个 model 字段逐字核实自 models.rs);前端 task 给 assertKeysMatch 全文 + contract.ts 模板 + 明确"键抄自 fixture";ALLOWLIST task 给确切删除文本。playbook.contract.ts 的示例键序标注"实际以 fixture 为准"避免误导。

**3. 类型一致性**:5 投影签名逐字核实(playbook/prompt_template/evaluation_scenario/suspected_deal 吃值,outbox_entry 吃 `&OutboxEntry` 引用——Task 5 调用写 `outbox_entry_json(&entry)`)。fixture 命名(playbook/prompt_template/evaluation_scenario/suspected_deal/outbox_entry)前后端一致。键数(18/13/12/13/22)来自源码核实,前端 task 标"以 fixture 为准"二次保险。dt_to_string 返回 Option 的行为在共享约定说明。decisionId(None→null)vs id(None→"")的差异在 Task 5 关键点明确。

**已知风险与对策**:
1. Task 1/2 改的文件(playbooks.rs/prompt_templates.rs)**无 mod tests 需新建**,Task 3/4/5 改的文件已有 mod tests 追加——已在事实表 + 各 task 标注。
2. evaluation_scenario 的 Document 直发——已在顶部"关键决策"完整定论(投影不改,fixture 喂纯标量 doc 照搬种子,铁律不塞 ObjectId/DateTime),Task 3 关键点 + Step3 核对($numberInt 泄漏)双重标注。
3. outbox_entry 吃引用 + decisionId null 差异——Task 5 明确。
4. 共享 worktree target 争用——全局约束 + Task 7 标注走 CI 不假绿。

**键数核对预期**(以 fixture 为最终真相源):playbook 18 / prompt_template 13 / evaluation_scenario 12 / suspected_deal 13 / outbox_entry 22。
