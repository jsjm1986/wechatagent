# 标签可信度改造 · 子计划 2：证据绑定 + 强弱证据 + customer_stage 快通道 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Reply Agent 逐轮输出的标签判断带"证据引用 + explicit_intent 标志"，写进 `memory_candidates`（`source="tag_observation"`）作为暂定 tally 层；新增 `evidence_strength` 纯函数（靠"是否锚定客户 inbound 消息 + explicit 标志"客观判强弱）；给 `customer_stage` 加强证据快通道（强证据立即驱动现有状态机，弱证据只进暂定层不驱动）。

**Architecture:** LLM 输出加证据字段（窗口序位 + explicit_intent），代码侧把序位映射回消息 `_id` hex 并 fail-closed 校验。逐轮标签判断不再写 `Contact.confirmed_tags`（那是子计划 3 压缩重判的产物），只写 `memory_candidates` 暂定层。`customer_stage` 是唯一例外：强证据走现有写入链路实时生效，弱证据沉淀暂定层。

**Tech Stack:** Rust 2021 / Axum / MongoDB (bson ObjectId) / serde / DeepSeek-OpenAI 兼容 LLM JSON。

## Global Constraints

- `cargo test --lib` ≥ 350 passed / 0 failed；四 PBT 累计 ≥ 33 / 0 不回归。
- 本地只跑 `cargo test --lib` + 单 PBT；完整集成留 CI。
- **agent-first**：强弱证据由代码按"消息 direction + explicit 标志"客观判，**不靠 LLM 自称置信**，不引入关键词词表。
- **no-human-takeover**：新代码/注释/prompt 文案避开禁用词。
- **过拟合红线**：prompt 改动只沉淀可复现的抽象要求（"标签须挂证据"），不对单条对话点对点修补。
- **既成事实纪律**：标签 observations 写库失败不阻断 reply（reply 已走 outbox），只 `tracing::warn!`。
- 提交需用户显式批准；精确 `git add`。

## 设计来源

`docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md` —— "证据绑定"、"强/弱证据判定"、"customer_stage 双层 + 强证据快通道"三节。

## 依赖

- **子计划 1 已完成**：`Evidence`、`ConfirmedTag`、Contact 新字段已存在。
- 本子计划新增 AgentDecision/RawAgentDecision 的证据字段、observations 写入、强弱证据纯函数、stage 快通道。

## 现状核实（已亲读 + subagent 核实，事实基线）

- `ConversationMessage`：`src/models.rs:485-504`。`id: Option<ObjectId>`（`#[serde(rename="_id")]`，唯一锚点）、`message_id: Option<String>`（微信侧，常 None，**不可作锚**）、`direction: MessageDirection`、`content: String`、`created_at: DateTime`。**无 turn 字段**。
- `MessageDirection`：`src/models.rs:479-482`，`enum { Inbound, Outbound }`。
- 消息查询：`load_recent_messages`（`gateway.rs:4172-4198`）filter workspace+account+contact_wxid，sort `{created_at:-1}` limit N。`load_context_messages`（`gateway.rs:4200-4207`）limit `recent_message_limit*6` clamp(24,80)。
- `AgentDecision`：`src/agent/types.rs:82-`。`tags: Vec<String>`（:98，`#[serde(default, deserialize_with="string_or_vec")]`）、`customer_stage: Option<String>`（:99，独立 typed 字段）、`intent_level`（:100）、`domain_signals: Document`（:109）、`operation_state_confidence: Option<i32>`（:123）。**无 evidence 字段**。
- `RawAgentDecision`：`types.rs:348-`，`tags: Option<Vec<String>>`（:389）、`customer_stage: Option<String>`（:390）。
- `carry_through_fields`：`types.rs:889-923`，tags 透传在 :908-910，customer_stage :911-913。
- 逐轮写候选入口：`write_memory_candidates`（`memory.rs:1220-1273`），`MemoryCandidate.source` 字段已存在（当前填 `decision.run_mode`），`candidates: Vec<Document>`，`validated_memory_candidate`（:1288）强制 type/content/evidence 非空。
- customer_stage 写入链路：⑧块 `gateway.rs:3315-3386`，状态机校验 `check_state_transition`（:3337），非法→剔除 customer_stage（:3366-3373）→ `insert_domain_signal_values`（:3378）。C2 块 :3458-3496 派生 operation_state。
- `check_state_transition` 签名：`src/agent/guards.rs:156-160`，`(domain_config: Option<&OperationDomainConfig>, from: Option<&str>, to: &str) -> Option<String>`（Some=拒绝原因，None=放行）。
- prompt 输出 schema：`src/prompts.rs:935-937`（`tags`/`customerStage`/`intentLevel`）、:1123-1124（自由域 schema）。

---

## Task 1：evidence_strength + 窗口序位映射纯函数

**Files:**
- Create: `src/agent/tag_evidence.rs`（新文件，纯函数 + 单测，避免堆进已大的 gateway/decision）
- Modify: `src/agent/mod.rs`（加 `mod tag_evidence;` + 必要 re-export）
- Test: `src/agent/tag_evidence.rs` 文件内 `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub enum EvidenceStrength { Strong, Weak }`
  - `pub fn resolve_evidence(window: &[ConversationMessage], turn_indices: &[i32]) -> Vec<Evidence>` —— 把 LLM 给的窗口序位映射成 `Evidence{turn, msg_id=_id.hex}`，越界序位丢弃（fail-closed）。
  - `pub fn evidence_strength(evidences: &[Evidence], window: &[ConversationMessage], explicit_intent: bool) -> EvidenceStrength` —— 任一证据指向 Inbound 消息且 explicit_intent=true → Strong，否则 Weak。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversationMessage, MessageDirection};
    use bson::oid::ObjectId;

    fn msg(dir: MessageDirection) -> ConversationMessage {
        ConversationMessage {
            id: Some(ObjectId::new()),
            workspace_id: "w".into(), account_id: "a".into(), contact_wxid: "c".into(),
            message_id: None, dedupe_key: None,
            direction: dir, content: "x".into(),
            msg_type: None, media_ref: None, raw: None,
            created_at: bson::DateTime::from_millis(0),
        }
    }

    #[test]
    fn resolve_evidence_maps_index_to_oid_and_drops_out_of_range() {
        let w = vec![msg(MessageDirection::Inbound), msg(MessageDirection::Outbound)];
        let ev = resolve_evidence(&w, &[0, 5]); // 0 有效, 5 越界
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].turn, 0);
        assert_eq!(ev[0].msg_id, w[0].id.unwrap().to_hex());
    }

    #[test]
    fn strength_strong_when_inbound_and_explicit() {
        let w = vec![msg(MessageDirection::Inbound)];
        let ev = resolve_evidence(&w, &[0]);
        assert!(matches!(evidence_strength(&ev, &w, true), EvidenceStrength::Strong));
    }

    #[test]
    fn strength_weak_when_outbound_even_if_explicit() {
        let w = vec![msg(MessageDirection::Outbound)];
        let ev = resolve_evidence(&w, &[0]);
        assert!(matches!(evidence_strength(&ev, &w, true), EvidenceStrength::Weak));
    }

    #[test]
    fn strength_weak_when_not_explicit() {
        let w = vec![msg(MessageDirection::Inbound)];
        let ev = resolve_evidence(&w, &[0]);
        assert!(matches!(evidence_strength(&ev, &w, false), EvidenceStrength::Weak));
    }
}
```

> 实现者：核对 `ConversationMessage` 字段名/构造（subagent 报告 models.rs:485-504），若有字段遗漏按真实结构补。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib tag_evidence`
Expected: 编译失败 —— 模块/函数未定义。

- [ ] **Step 3: 写实现**

```rust
// src/agent/tag_evidence.rs
use crate::models::{ConversationMessage, Evidence, MessageDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStrength { Strong, Weak }

/// 把 LLM 给的窗口序位（0-based，按 created_at 排序后的下标）映射成 Evidence。
/// 越界序位 / 无 _id 的消息直接丢弃（fail-closed：锚不上不放水）。
pub fn resolve_evidence(window: &[ConversationMessage], turn_indices: &[i32]) -> Vec<Evidence> {
    let mut out = Vec::new();
    for &idx in turn_indices {
        if idx < 0 { continue; }
        let Some(msg) = window.get(idx as usize) else { continue; };
        let Some(oid) = msg.id else { continue; };
        out.push(Evidence { turn: idx, msg_id: oid.to_hex() });
    }
    out
}

/// 强证据：至少一条证据指向客户本人(Inbound)消息，且 LLM 标注 explicit_intent=true。
/// 否则弱。强弱由消息 direction + explicit 标志客观决定，不读 LLM 自称置信。
pub fn evidence_strength(
    evidences: &[Evidence],
    window: &[ConversationMessage],
    explicit_intent: bool,
) -> EvidenceStrength {
    if !explicit_intent {
        return EvidenceStrength::Weak;
    }
    let anchored_to_customer = evidences.iter().any(|e| {
        window
            .get(e.turn as usize)
            .map(|m| matches!(m.direction, MessageDirection::Inbound))
            .unwrap_or(false)
    });
    if anchored_to_customer { EvidenceStrength::Strong } else { EvidenceStrength::Weak }
}
```

在 `src/agent/mod.rs` 加 `mod tag_evidence;`（pub 程度按 gateway/decision 是否跨模块引用决定，需要则 `pub(crate) use`）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib tag_evidence`
Expected: 4 passed。

- [ ] **Step 5: 提交**

```bash
git add src/agent/tag_evidence.rs src/agent/mod.rs
git commit -m "feat(tag-trust): evidence_strength + window-index→ObjectId resolver (子计划2 Task1)"
```

---

## Task 2：AgentDecision/RawAgentDecision 加证据字段 + carry_through

**Files:**
- Modify: `src/agent/types.rs:98`（AgentDecision 加字段）、`:389`（RawAgentDecision 加字段）、`:908-913`（carry_through）
- Test: `src/agent/types.rs` 测试 mod（carry_through 透传）

**Interfaces:**
- Produces: AgentDecision/RawAgentDecision 新增 `tag_evidence_turns: Vec<i32>`（LLM 指认的标签证据窗口序位）、`stage_evidence_turns: Vec<i32>`、`stage_explicit_intent: bool`。carry_through 透传。

> 设计取舍：标签是 `Vec<String>`，给整批标签共享一个证据序位集合（`tag_evidence_turns`）即可——逐标签精确配对会让 LLM 输出复杂度暴增、收益低（标签本就要在压缩重判时重新指认证据）。customer_stage 单独有 `stage_evidence_turns` + `stage_explicit_intent`（快通道判定需要）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn carry_through_propagates_evidence_fields() {
    let mut raw = RawAgentDecision::default(); // 若无 Default，用现有构造方式
    raw.tag_evidence_turns = Some(vec![1, 2]);
    raw.stage_evidence_turns = Some(vec![3]);
    raw.stage_explicit_intent = Some(true);
    let mut decision = AgentDecision::default();
    carry_through_fields(&raw, &mut decision);
    assert_eq!(decision.tag_evidence_turns, vec![1, 2]);
    assert_eq!(decision.stage_evidence_turns, vec![3]);
    assert!(decision.stage_explicit_intent);
}
```

> 核对 `RawAgentDecision` / `AgentDecision` 是否有 `Default`（subagent 未报，需 grep `derive.*Default` 或现有构造）。无则用文件内现有的 fixture 构造模式。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib carry_through_propagates_evidence`
Expected: 编译失败 —— 字段不存在。

- [ ] **Step 3: 写实现**

AgentDecision（types.rs:98 附近，紧邻 tags）：
```rust
    /// 子计划2：LLM 指认的标签证据——窗口内消息序位（0-based）。代码侧映射回 _id 并 fail-closed 校验。
    #[serde(default)]
    pub tag_evidence_turns: Vec<i32>,
    /// customer_stage 判断的证据序位。
    #[serde(default)]
    pub stage_evidence_turns: Vec<i32>,
    /// LLM 标注：customer_stage 是否基于客户明示意图（非 AI 语境推断）。
    #[serde(default)]
    pub stage_explicit_intent: bool,
```

RawAgentDecision（types.rs:389 附近，紧邻 tags）：
```rust
    #[serde(default)]
    pub tag_evidence_turns: Option<Vec<i32>>,
    #[serde(default)]
    pub stage_evidence_turns: Option<Vec<i32>>,
    #[serde(default)]
    pub stage_explicit_intent: Option<bool>,
```

carry_through_fields（types.rs:910 之后补）：
```rust
    if let Some(v) = raw.tag_evidence_turns.clone() { decision.tag_evidence_turns = v; }
    if let Some(v) = raw.stage_evidence_turns.clone() { decision.stage_evidence_turns = v; }
    if let Some(v) = raw.stage_explicit_intent { decision.stage_explicit_intent = v; }
```

> 字段 serde wire 名：确认 AgentDecision 是否 camelCase（若是，`tag_evidence_turns` → `tagEvidenceTurns`，但 `#[serde(rename_all=...)]` 在结构体级已处理则无需逐字段 rename）。与 prompt schema（Task 5）的输出键名必须一致。

- [ ] **Step 4: 运行确认通过 + 编译**

Run: `cargo test --lib carry_through_propagates_evidence`
Expected: passed。
Run: `cargo check --tests`
Expected: 0 errors（所有 AgentDecision/Raw 构造点补字段——多带 default，但字面量构造仍需补）。

- [ ] **Step 5: 提交**

```bash
git add src/agent/types.rs
git commit -m "feat(tag-trust): AgentDecision carries tag/stage evidence turns + explicit flag (子计划2 Task2)"
```

---

## Task 3：逐轮标签判断写 tag_observation（暂定层）

**Files:**
- Create/Modify: `src/agent/memory.rs`（新增 `write_tag_observations`，仿 `write_memory_candidates` :1220）
- Modify: `src/agent/gateway.rs`（发送后调用点，仿 `write_memory_candidates` 的调用位置）
- Test: observations 文档形态的纯函数构造单测

**Interfaces:**
- Consumes: Task 1 `resolve_evidence`、Task 2 `decision.tags` + `tag_evidence_turns`、子计划 1 的 `memory_candidates` collection。
- Produces: `pub(crate) async fn write_tag_observations(state, contact, decision, window, run_id)` —— 把 `decision.tags` 连同证据写进 `memory_candidates`，`source="tag_observation"`，每个标签一个 candidate doc `{ dimension:"tag", value, evidences:[{turn,msgId}], hit_count:1 }`。

- [ ] **Step 1: 核实 write_memory_candidates 调用点**

Read `gateway.rs` 中 `write_memory_candidates(` 的调用行（grep）。确认它在发送/决策后、有 `recent_messages` 或 `load_context_messages` 在手的位置。tag_observations 应在**同一处**调用（共享窗口变量）。

- [ ] **Step 2: 写候选构造纯函数 + 失败测试**

```rust
/// 把一轮标签判断转成 tag_observation 候选 docs（纯函数，便于单测）。
/// 每个标签一条；evidences 由 resolve_evidence 产出（已 fail-closed）。
/// 标签共享本轮 tag_evidence_turns（设计取舍：不逐标签配对）。
pub(crate) fn build_tag_observation_docs(
    tags: &[String],
    evidences: &[Evidence],
) -> Vec<Document> { /* ... */ }
```

```rust
#[test]
fn build_tag_observation_docs_one_per_tag_with_shared_evidence() {
    let ev = vec![Evidence { turn: 0, msg_id: "deadbeef".into() }];
    let docs = build_tag_observation_docs(&["价格敏感".into(), "犹豫".into()], &ev);
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].get_str("dimension").unwrap(), "tag");
    assert_eq!(docs[0].get_str("value").unwrap(), "价格敏感");
    assert_eq!(docs[0].get_i32("hitCount").unwrap(), 1);
    assert!(docs[0].get_array("evidences").is_ok());
}

#[test]
fn build_tag_observation_docs_empty_tags_yields_empty() {
    assert!(build_tag_observation_docs(&[], &[]).is_empty());
}
```

- [ ] **Step 3: 实现 build_tag_observation_docs + write_tag_observations**

```rust
pub(crate) fn build_tag_observation_docs(tags: &[String], evidences: &[Evidence]) -> Vec<Document> {
    let ev_bson: Vec<Document> = evidences.iter()
        .map(|e| doc! { "turn": e.turn, "msgId": &e.msg_id })
        .collect();
    tags.iter().map(|t| doc! {
        "dimension": "tag",
        "value": t,
        "hitCount": 1,
        "evidences": &ev_bson,
    }).collect()
}

/// 逐轮把标签判断写进 memory_candidates 暂定层（source="tag_observation"）。
/// 不写 confirmed_tags（那是压缩重判产物）。写库失败不阻断 reply，仅 warn。
pub(crate) async fn write_tag_observations(
    state: &AppState,
    contact: &Contact,
    decision: &AgentDecision,
    window: &[ConversationMessage],
    run_id: &str,
) -> AppResult<()> {
    if decision.tags.is_empty() { return Ok(()); }
    let evidences = crate::agent::tag_evidence::resolve_evidence(window, &decision.tag_evidence_turns);
    // 无证据的标签判断丢弃（fail-closed：从源头掐脑补）。
    if evidences.is_empty() { return Ok(()); }
    let docs = build_tag_observation_docs(&decision.tags, &evidences);
    let candidate = MemoryCandidate {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        run_id: Some(run_id.to_string()),
        source: "tag_observation".to_string(),
        candidates: docs,
        memory_write_score: 0,
        status: "pending".to_string(),
        reason: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    state.db.memory_candidates().insert_one(&candidate, None).await?;
    Ok(())
}
```

> `MemoryCandidate` 字段对齐 `models.rs:1092`（subagent 已确认含 source/candidates/status 等）。`reason` 是否 `Option<String>`：核对。

- [ ] **Step 4: 在 gateway 调用 + 测试**

在 Step 1 找到的 `write_memory_candidates` 调用点旁加 `write_tag_observations(...)`（同样 `.await` + 失败只 warn 的既成事实处理——参考 `write_memory_candidates` 调用是否被 `let _ =` 或 `?`；标签 observations 失败不应阻断，用 `if let Err(e) = ... { tracing::warn!(...) }`）。

Run: `cargo test --lib build_tag_observation_docs`
Expected: 2 passed。
Run: `cargo check` → 0 errors。

- [ ] **Step 5: 提交**

```bash
git add src/agent/memory.rs src/agent/gateway.rs
git commit -m "feat(tag-trust): write per-turn tag judgments to tag_observation tentative layer (子计划2 Task3)"
```

---

## Task 4：customer_stage 强证据快通道

**Files:**
- Modify: `src/agent/gateway.rs:3315-3386`（⑧块 stage 写入处）
- Test: 快通道门控纯函数单测

**Interfaces:**
- Consumes: Task 1 `evidence_strength`、Task 2 `decision.stage_evidence_turns` + `stage_explicit_intent`。
- Produces: stage 写入前的强弱门——强证据走现有写入链路（实时生效）；弱证据**不写 domain_attributes.customer_stage**（改为只进 tag_observation 暂定层，等压缩重判）。

- [ ] **Step 1: 抽门控纯函数 + 失败测试**

```rust
/// customer_stage 是否允许逐轮实时写入：仅强证据放行。
/// 弱证据 → false（不实时写，沉淀暂定层，等压缩重判）。
pub(crate) fn stage_realtime_write_allowed(strength: EvidenceStrength) -> bool {
    matches!(strength, EvidenceStrength::Strong)
}
```

```rust
#[test]
fn stage_realtime_write_only_on_strong() {
    assert!(stage_realtime_write_allowed(EvidenceStrength::Strong));
    assert!(!stage_realtime_write_allowed(EvidenceStrength::Weak));
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib stage_realtime_write_only_on_strong`
Expected: 编译失败。

- [ ] **Step 3: 实现 + 接入 ⑧块**

在 ⑧块（gateway.rs:3315 起）写 customer_stage 前，先算强弱：
```rust
let stage_strength = {
    let ev = crate::agent::tag_evidence::resolve_evidence(&window, &decision.stage_evidence_turns);
    crate::agent::tag_evidence::evidence_strength(&ev, &window, decision.stage_explicit_intent)
};
```
若 `!stage_realtime_write_allowed(stage_strength)` → 从 `signals_for_attrs`（写入 domain_attributes 的过滤副本）中**剔除 customer_stage**（复用现有 :3366-3373 的剔除机制），并把该 stage 判断作为一条 `dimension:"customer_stage"` 的 observation 写进暂定层（复用 Task 3 的 write 路径，或在 build_tag_observation_docs 加 dimension 参数）。
强证据则保持现有链路不变（照常过 `check_state_transition`）。

> 关键：弱证据时 stage **不写 domain_attributes**，但仍要落暂定层 observation（否则压缩重判看不到这个 stage 判断）。需要 `window` 变量在 ⑧块在手——确认 `apply_agent_updates` 能拿到窗口；若拿不到需把 window 透传进来（Read `apply_agent_updates` 签名与调用，gateway.rs:3169）。

- [ ] **Step 4: 测试 + 编译 + 基线**

Run: `cargo test --lib stage_realtime_write_only_on_strong`
Expected: passed。
Run: `cargo check --tests` → 0 errors。
Run: `cargo test --lib` → ≥ 350 / 0（状态机相关测试不回归）。

- [ ] **Step 5: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "feat(tag-trust): customer_stage strong-evidence fast path; weak goes tentative (子计划2 Task4)"
```

---

## Task 5：prompt schema 加证据输出要求

**Files:**
- Modify: `src/prompts.rs:935-937`（profile schema 的 tags/customerStage）、:1123-1124（自由域 schema）
- Modify: `src/prompts.rs`（bump `PROMPT_PACK_VERSION` —— grep 确认常量名与位置）
- Test: prompt 文本含新字段要求的断言（若 prompt 是常量字符串）

**Interfaces:**
- Produces: LLM 被要求额外输出 `tagEvidenceTurns: [窗口序位]`、`stageEvidenceTurns: [...]`、`stageExplicitIntent: bool`，并在 prompt 里解释"窗口序位"含义 + "无证据的标签不要输出"。

- [ ] **Step 1: 核实 prompt 形态与版本常量**

Read `src/prompts.rs:920-1000`（profile schema 块）与 :1100-1140（自由域）。grep `PROMPT_PACK_VERSION` 找版本常量（改 prompt 必 bump，见 referral 记忆经验：prompt 改须 bump 版本）。确认 schema 是 Rust 字符串常量还是 DB seed 文本。

- [ ] **Step 2: 改 schema 文案**

在 tags/customerStage 输出说明处追加（保持 agent-first、抽象表述，不针对单条对话）：
```
"tagEvidenceTurns": [证据消息的窗口序号数组]   // 你给的每个标签都应有对话依据；
                                              // 序号 = 下方对话列表中消息的编号（从 0 起）。
                                              // 没有对话依据支撑的标签不要输出。
"stageEvidenceTurns": [...],                  // 判定 customerStage 的依据消息序号
"stageExplicitIntent": true/false             // customerStage 是否基于客户明确表达（而非你的推断）
```
同时在对话列表渲染处确认每条消息**带可见序号**（装 prompt 时编号；若现状未编号，需在装窗口的渲染函数加序号前缀——Read 装对话的渲染点）。

- [ ] **Step 3: bump 版本 + 测试**

bump `PROMPT_PACK_VERSION`。若 prompt 是常量，加断言：
```rust
#[test]
fn reply_schema_requests_evidence_turns() {
    assert!(REPLY_TASK_SCHEMA.contains("tagEvidenceTurns"));
    assert!(REPLY_TASK_SCHEMA.contains("stageExplicitIntent"));
}
```
（常量名对齐实际。若 schema 在 DB seed 文本里，则此断言改为对 seed 函数返回值断言。）

- [ ] **Step 4: 编译 + 测试 + 基线**

Run: `cargo test --lib reply_schema_requests_evidence`（或对应名）
Expected: passed。
Run: `cargo test --lib` → ≥ 350 / 0。

> 真模型验证（LLM 是否真按新 schema 输出证据序号）依赖真实 LLM CI 套件，留 CI。本地只验文本 + 编译。

- [ ] **Step 5: 提交**

```bash
git add src/prompts.rs
git commit -m "feat(tag-trust): prompt schema requires evidence turns + explicit intent; bump pack version (子计划2 Task5)"
```

---

## Self-Review（写计划者自检）

**Spec 覆盖：**
- 证据存引用 + fail-closed → Task 1（resolve_evidence 越界丢弃）✓
- 无证据不许写 → Task 3（evidences 空则不写 observation）✓
- 强弱证据纯函数、不靠 LLM 自称 → Task 1（evidence_strength 读 direction + explicit）✓
- 逐轮写暂定层不写 confirmed → Task 3 ✓
- customer_stage 强证据快通道 / 弱证据等压缩 → Task 4 ✓
- LLM 输出证据字段 → Task 2（结构）+ Task 5（prompt）✓

**占位符扫描：** `build_tag_observation_docs` 函数体在 Step 2 留 `/* ... */`，Step 3 给出完整实现 —— 是 TDD 的"先声明签名测、再实现"节奏，非占位。Task 4 Step 3 的"window 是否在 apply_agent_updates 在手"是真实的实现期核实点，已标注 Read 动作。

**类型一致性：** `EvidenceStrength`、`Evidence{turn,msg_id}`（子计划1）、`tag_evidence_turns`/`stage_evidence_turns`/`stage_explicit_intent`（Task 2 定义，Task 3/4 消费）、wire 名 camelCase（`tagEvidenceTurns`）在 Task 2 结构与 Task 5 prompt schema 必须一致 —— 已在 Task 2 Step 3 标注核对 serde rename。

**跨子计划衔接：** 本子计划产出的 `tag_observation` 候选（含 evidences/hitCount）是子计划 3 压缩重判的输入；observation 的 doc 形态（dimension/value/hitCount/evidences）在子计划 3 会被读取，键名需一致。

**需实现期核实（已在步骤标注）：** RawAgentDecision/AgentDecision 是否有 Default、MemoryCandidate.reason 类型、apply_agent_updates 能否拿到 window、PROMPT_PACK_VERSION 常量名、对话列表是否已带序号渲染。
