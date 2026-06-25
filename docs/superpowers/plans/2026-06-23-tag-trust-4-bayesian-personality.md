# 标签可信度改造 · 子计划 4：贝叶斯评估旁路 + 压缩时大五人格 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地两条纯观测线——(A) 贝叶斯评估旁路：最多 6 槽、AI 自由发现维度、严谨两阶段占槽（多轮强证据累积才占）、低置信淘汰、逐轮增量更新 + history 封顶、永不驱动行为；(B) 压缩时大五 OCEAN 人格分析：只在归并时更新、封闭五维、证据强制、诚实置信、专属版本化 prompt。

**Architecture:** 贝叶斯逐轮在 gateway 发送后增量更新 `Contact.bayesian_signals`（与 confirmed_tags 主路解耦）；占槽门是纯函数，靠 tag_observation 累积的 hit_count + 强证据数判定。人格在 `consolidate_contact_memory` 搭车产出，独立 prompt key，写 `Contact.personality_profile`。两者都**绝不进任何 filter/状态机/选材/触达门**。

**Tech Stack:** Rust 2021 / Axum / MongoDB / serde / LLM JSON。

## Global Constraints

- `cargo test --lib` ≥ 350 / 0；四 PBT 累计 ≥ 33 / 0。
- 本地只 `cargo test --lib` + 单 PBT；集成留 CI。
- **永不驱动行为铁律**：`bayesian_signals` / `personality_profile` 不得出现在任何 planner filter、状态机、media_send/referral 选材、check_state_transition 路径。CI/review 须验证。
- **agent-first**：人格五维由 LLM 看宽窗口判，不用关键词；贝叶斯维度 AI 自由发现。
- **占槽严谨**（用户强调"不能因为一两句话就占槽"）：占槽门高阈值、可配、保守默认。
- **no-human-takeover**：代码/注释/prompt 避禁用词。
- **过拟合红线**：人格 prompt 沉淀 OCEAN 方法论，不针对样本调。
- 提交需用户显式批准；精确 `git add`。

## 设计来源

`docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md` —— "贝叶斯评估旁路" + "压缩时人格分析（大五 OCEAN）"两节。

## 依赖

- **子计划 1**：`BayesianSignal` / `BayesianPoint` / `PersonalityProfile` / `PersonalityFacet` / `PersonalitySnapshot` / `Evidence` 结构 + Contact 字段已存在。
- **子计划 2**：`tag_observation` 候选（含 hitCount/evidences/evidence_strength 原料）、`resolve_evidence`。
- **子计划 3**：`consolidate_contact_memory_inner` 已加载宽窗口（人格分析复用同一窗口）。

## 现状核实（事实基线）

- 结构体（子计划 1 已建）：`BayesianSignal{dimension,current_value,current_confidence,locked,history}`、`BayesianPoint{turn,value,confidence,value_changed,confidence_changed,reason}`、`PersonalityProfile{5×PersonalityFacet,updated_at,snapshots}`、`PersonalityFacet{score,confidence,evidence_refs}`、`PersonalitySnapshot{consolidated_at,scores,confidences}`。
- 逐轮写入点：gateway 发送后（子计划 2 Task3 已在此加 `write_tag_observations`）。
- 压缩归并：`consolidate_contact_memory_inner`（memory.rs:900），子计划 3 已注入宽窗口 `window` 变量。
- LLM JSON 入口：`generate_agent_json`（memory.rs:1007 范本）。
- prompt seed：`prompts.rs` + `ensure_prompt_pack_v2`（startup seed）。新 prompt key 需 seed + bump 版本。
- runtime 可配范本：`RuntimeParametersTyped`（models.rs）+ `from_config`（runtime.rs:113）clamp。

---

## Task 1：贝叶斯占槽门纯函数

**Files:**
- Create: `src/agent/bayesian_slots.rs`（纯函数 + 单测）
- Modify: `src/agent/mod.rs`（`mod bayesian_slots;`）
- Test: 文件内 `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub const MAX_BAYESIAN_SLOTS: usize = 6;`
  - `pub struct SlotPromotionThreshold { pub min_hits: i32, pub min_strong_evidence: i32 }`（可配，默认 min_hits=3 / min_strong=2）
  - `pub fn should_promote(hit_count: i32, strong_evidence_count: i32, th: &SlotPromotionThreshold) -> bool`
  - ~~`pub fn should_evict(signal: &BayesianSignal, turns_absent: i32, max_absent: i32, low_conf: f64) -> bool`~~ —— **按 Option B / YAGNI 决策删除（未实现）**：bayesian_signals 是永不驱动旁路，槽僵化零业务影响；Step 验收弹性以 4 个不变量测试为准（不含淘汰）。详见交叉验证 D5-F2。
  - `pub fn apply_bayesian_update(signals: &mut Vec<BayesianSignal>, observed: &[ObservedDimension], turn: i32, th: &SlotPromotionThreshold)` —— 增量更新：已占槽的更新 history（封顶 100），未占槽的累积，达阈值才 lock 占槽（满 6 则排队）。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_promote_below_threshold() {
        let th = SlotPromotionThreshold { min_hits: 3, min_strong_evidence: 2 };
        assert!(!should_promote(2, 2, &th)); // hits 不够
        assert!(!should_promote(3, 1, &th)); // 强证据不够
        assert!(should_promote(3, 2, &th));  // 双达标
    }

    #[test]
    fn single_mention_never_promotes() {
        // 用户红线：一两句话不能占槽。
        let th = SlotPromotionThreshold { min_hits: 3, min_strong_evidence: 2 };
        assert!(!should_promote(1, 1, &th));
    }

    #[test]
    fn history_capped_at_100() {
        let th = SlotPromotionThreshold { min_hits: 1, min_strong_evidence: 0 };
        let mut signals = vec![];
        for turn in 0..150 {
            apply_bayesian_update(&mut signals, &[ObservedDimension {
                dimension: "价格敏感度".into(), value: "高".into(), confidence: 0.6, strong_evidence_count: 1,
            }], turn, &th);
        }
        let sig = signals.iter().find(|s| s.dimension == "价格敏感度").unwrap();
        assert!(sig.locked);
        assert!(sig.history.len() <= 100);
    }

    #[test]
    fn never_exceeds_six_locked_slots() {
        let th = SlotPromotionThreshold { min_hits: 1, min_strong_evidence: 0 };
        let mut signals = vec![];
        for d in 0..10 {
            apply_bayesian_update(&mut signals, &[ObservedDimension {
                dimension: format!("dim{d}"), value: "v".into(), confidence: 0.5, strong_evidence_count: 1,
            }], 0, &th);
        }
        assert!(signals.iter().filter(|s| s.locked).count() <= MAX_BAYESIAN_SLOTS);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib bayesian_slots`
Expected: 编译失败。

- [ ] **Step 3: 写实现**

```rust
// src/agent/bayesian_slots.rs
use crate::models::{BayesianSignal, BayesianPoint};

pub const MAX_BAYESIAN_SLOTS: usize = 6;
pub const HISTORY_CAP: usize = 100;

#[derive(Debug, Clone)]
pub struct SlotPromotionThreshold { pub min_hits: i32, pub min_strong_evidence: i32 }
impl Default for SlotPromotionThreshold {
    fn default() -> Self { Self { min_hits: 3, min_strong_evidence: 2 } }
}

/// 本轮观察到的一个维度（由 LLM 输出 + 代码侧证据强度统计得来）。
pub struct ObservedDimension {
    pub dimension: String,
    pub value: String,
    pub confidence: f64,
    pub strong_evidence_count: i32,
}

/// 占槽门：跨多轮命中 + 强证据累积双达标才占。一两句话（hit=1）永远不够。
pub fn should_promote(hit_count: i32, strong_evidence_count: i32, th: &SlotPromotionThreshold) -> bool {
    hit_count >= th.min_hits && strong_evidence_count >= th.min_strong_evidence
}

/// 淘汰门（对称高阈值）：连续缺席多轮 + 当前置信低。
/// **注：按 Option B / YAGNI 决策最终未实现（永不驱动旁路，槽僵化零业务影响）。下方为原设计示意，保留备查。**
pub fn should_evict(signal: &BayesianSignal, turns_absent: i32, max_absent: i32, low_conf: f64) -> bool {
    turns_absent >= max_absent && signal.current_confidence < low_conf
}

/// 增量更新贝叶斯信号。已占槽→更新值/置信/history（封顶）；未占槽→累积 hit，
/// 达阈值且未满 6 槽则 lock。永不驱动行为，纯观测。
pub fn apply_bayesian_update(
    signals: &mut Vec<BayesianSignal>,
    observed: &[ObservedDimension],
    turn: i32,
    th: &SlotPromotionThreshold,
) {
    for obs in observed {
        // hit_count 用 history.len()+pending 估计；这里用一个累积计数字段更直接——
        // 简化：用 history 长度近似 hit_count（已观察轮数）。
        if let Some(sig) = signals.iter_mut().find(|s| s.dimension == obs.dimension) {
            let value_changed = sig.current_value != obs.value;
            let confidence_changed = (sig.current_confidence - obs.confidence).abs() > f64::EPSILON;
            sig.current_value = obs.value.clone();
            sig.current_confidence = obs.confidence;
            sig.history.push(BayesianPoint {
                turn, value: obs.value.clone(), confidence: obs.confidence,
                value_changed, confidence_changed, reason: None,
            });
            while sig.history.len() > HISTORY_CAP { sig.history.remove(0); }
            // 未占槽的累积达阈值 + 未满 6 槽 → 占槽
            if !sig.locked {
                let hits = sig.history.len() as i32;
                let strong = obs.strong_evidence_count; // 累积口径见说明
                let locked_count = signals.iter().filter(|s| s.locked).count();
                // 重新借用：上面 sig 借用已结束（NLL）；实际实现需处理借用，见 Step 3 说明
                let _ = (hits, strong, locked_count);
            }
        } else {
            // 新维度：起一条未占槽的观察线
            signals.push(BayesianSignal {
                dimension: obs.dimension.clone(),
                current_value: obs.value.clone(),
                current_confidence: obs.confidence,
                locked: false,
                history: vec![BayesianPoint {
                    turn, value: obs.value.clone(), confidence: obs.confidence,
                    value_changed: false, confidence_changed: false, reason: None,
                }],
            });
        }
    }
    // 占槽判定单独一遍（避免上面 iter_mut 借用冲突）：
    let locked_count = signals.iter().filter(|s| s.locked).count();
    let mut budget = MAX_BAYESIAN_SLOTS.saturating_sub(locked_count);
    for sig in signals.iter_mut() {
        if budget == 0 { break; }
        if !sig.locked {
            let hits = sig.history.len() as i32;
            let strong = sig.history.iter().filter(|p| p.confidence >= 0.6).count() as i32; // 强证据累积近似
            if should_promote(hits, strong, th) {
                sig.locked = true;
                budget -= 1;
            }
        }
    }
}
```

> **实现者注意**：上面 `apply_bayesian_update` 的借用结构是示意——`strong_evidence_count` 的累积口径（用 history 里高置信点数近似 vs 单独存累积计数字段）实现时择一并在测试里钉死。关键不变量（测试已覆盖）：①hit=1 不占槽 ②history≤100 ③locked≤6。实现以通过这 4 个测试为准，借用细节自行理顺（可能需先收集待 promote 的 dimension 名再二次遍历）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib bayesian_slots`
Expected: 4 passed。

- [ ] **Step 5: 提交**

```bash
git add src/agent/bayesian_slots.rs src/agent/mod.rs
git commit -m "feat(tag-trust): bayesian slot promotion/eviction pure fns (6-slot cap, strict threshold) (子计划4 Task1)"
```

---

## Task 2：贝叶斯逐轮增量更新接入 + 可配阈值

**Files:**
- Modify: `src/models.rs`（RuntimeParametersTyped 加占槽阈值字段）+ `src/agent/runtime.rs`（UserRuntimeParameters 加字段 + clamp）
- Modify: `src/agent/gateway.rs`（发送后调用 apply_bayesian_update + 写回 bayesian_signals）
- Modify: `src/agent/types.rs`（AgentDecision 加 `bayesian_observations: Vec<...>` LLM 输出）
- Test: 增量更新接入的轻量断言

**Interfaces:**
- Consumes: Task 1 纯函数、子计划 2 的证据强度统计。
- Produces: 逐轮把 LLM 输出的贝叶斯维度观察 apply 进 `contact.bayesian_signals` 并写回 DB；阈值 `bayesian_slot_min_hits` / `bayesian_slot_min_strong` 可配。

- [ ] **Step 1: AgentDecision 加 LLM 贝叶斯输出字段**

`RawAgentDecision` + `AgentDecision`（types.rs）加：
```rust
    /// 贝叶斯评估旁路：LLM 自由发现的维度观察（最多取前 N 个，N>6 时代码侧截断）。
    #[serde(default)]
    pub bayesian_observations: Vec<BayesianObservationRaw>, // Raw 用 Option<Vec<>>
```
新结构 `BayesianObservationRaw { dimension: String, value: String, confidence: f64, evidence_turns: Vec<i32> }`（LLM 输出形态；strong_evidence_count 由代码侧用 resolve_evidence + evidence_strength 算，不信 LLM 自报）。carry_through 透传。

- [ ] **Step 2: 可配阈值**

`RuntimeParametersTyped` 加 `bayesian_slot_min_hits`（默认 3）、`bayesian_slot_min_strong`（默认 2）；`UserRuntimeParameters` 加字段，`from_config` clamp（min_hits clamp[1,20]，min_strong clamp[0,20]）。仿子计划 3 Task 2 的 runtime 加字段套路。

- [ ] **Step 3: gateway 接入**

在发送后（子计划 2 Task3 写 tag_observations 旁）加：
```rust
// 贝叶斯评估旁路：代码侧算每个维度的强证据数（不信 LLM 自报），增量更新，写回。
let th = SlotPromotionThreshold {
    min_hits: runtime.bayesian_slot_min_hits as i32,
    min_strong_evidence: runtime.bayesian_slot_min_strong as i32,
};
let observed: Vec<ObservedDimension> = decision.bayesian_observations.iter().map(|o| {
    let ev = resolve_evidence(&window, &o.evidence_turns);
    let strong = ev.iter().filter(|e| /* inbound */ window.get(e.turn as usize).map(|m| m.is_inbound()).unwrap_or(false)).count() as i32;
    ObservedDimension { dimension: o.dimension.clone(), value: o.value.clone(), confidence: o.confidence, strong_evidence_count: strong }
}).collect();
let mut signals = contact.bayesian_signals.clone();
apply_bayesian_update(&mut signals, &observed, current_turn, &th);
// 写回 set_doc.insert("bayesian_signals", to_bson(&signals)?)
```
`current_turn` 取窗口长度或会话消息计数（与 history turn 语义一致即可，文档化）。写回与 apply_agent_updates 的 set_doc 合并，或单独 update（确认不与主路写冲突——bayesian_signals 是独立字段，安全）。

> **解耦铁律标注**：本段写入只动 `bayesian_signals`，不读不写 confirmed_tags/manual_tags/customer_stage。注释明示"纯观测，永不驱动"。

- [ ] **Step 4: prompt schema 加贝叶斯输出 + bump 版本**

`prompts.rs` reply schema 加：
```
"bayesianObservations": [    // 可选：你对该客户的深层维度判断（最多 6 个，开放维度）
  { "dimension": "维度名（你自己命名，如 价格敏感度/决策果断度）", "value": "判断值", "confidence": 0.0~1.0, "evidenceTurns": [对话序号] }
]                            // 这些仅供评估，不影响你的回复决策
```
bump `PROMPT_PACK_VERSION`。

- [ ] **Step 5: 测试 + 编译 + 基线 + 提交**

Run: `cargo test --lib bayesian` + `cargo check --tests` → 0 errors。
Run: `cargo test --lib` → ≥ 350 / 0。
```bash
git add src/models.rs src/agent/runtime.rs src/agent/gateway.rs src/agent/types.rs src/prompts.rs
git commit -m "feat(tag-trust): per-turn bayesian incremental update with configurable thresholds (子计划4 Task2)"
```

---

## Task 3：压缩时大五人格分析

**Files:**
- Modify: `src/agent/memory.rs`（consolidate_contact_memory_inner 加人格分析调用 + 写回）
- Create: 新 prompt key `user.personality_analyzer.system` / `.task`（prompts.rs seed）
- Modify: `src/prompts.rs`（seed 新 key + bump 版本）
- Test: 人格解析纯函数单测

**Interfaces:**
- Consumes: 子计划 3 的宽窗口 `window`、子计划 1 `PersonalityProfile`/`PersonalityFacet`/`PersonalitySnapshot`、`resolve_evidence`。
- Produces: `pub(crate) fn parse_personality(value: &serde_json::Value, window: &[ConversationMessage]) -> Option<PersonalityProfile>` —— 解析五维 OCEAN，每维证据经 resolve_evidence，无证据维度 confidence 置低；写 `Contact.personality_profile` + append snapshot（封顶 50）。

- [ ] **Step 1: 写 parse_personality 失败测试**

```rust
#[test]
fn parse_personality_five_facets_with_evidence() {
    let window = vec![/* inbound 消息若干 */];
    let v = serde_json::json!({
        "personality": {
            "openness": { "score": 0.7, "confidence": 0.4, "evidenceTurns": [0] },
            "conscientiousness": { "score": 0.5, "confidence": 0.0, "evidenceTurns": [] },
            "extraversion": { "score": 0.6, "confidence": 0.3, "evidenceTurns": [0] },
            "agreeableness": { "score": 0.8, "confidence": 0.5, "evidenceTurns": [0] },
            "neuroticism": { "score": 0.3, "confidence": 0.2, "evidenceTurns": [0] }
        }
    });
    let p = parse_personality(&v, &window).expect("some");
    assert!((p.openness.score - 0.7).abs() < 1e-9);
    // 无证据维度 confidence 归 0（诚实置信）
    assert_eq!(p.conscientiousness.confidence, 0.0);
    assert!(p.conscientiousness.evidence_refs.is_empty());
}

#[test]
fn parse_personality_absent_yields_none() {
    assert!(parse_personality(&serde_json::json!({}), &[]).is_none());
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib parse_personality`
Expected: 编译失败。

- [ ] **Step 3: 实现 parse_personality + facet 解析**

```rust
fn parse_facet(v: &serde_json::Value, window: &[ConversationMessage]) -> PersonalityFacet {
    let score = v.get("score").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let turns: Vec<i32> = v.get("evidenceTurns").and_then(|t| t.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64().map(|n| n as i32)).collect()).unwrap_or_default();
    let evidence_refs = crate::agent::tag_evidence::resolve_evidence(window, &turns);
    // 诚实置信：无有效证据 → confidence 归 0，不许脑补人格
    let confidence = if evidence_refs.is_empty() { 0.0 }
        else { v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0) };
    PersonalityFacet { score, confidence, evidence_refs }
}

pub(crate) fn parse_personality(value: &serde_json::Value, window: &[ConversationMessage]) -> Option<PersonalityProfile> {
    let p = value.get("personality")?;
    Some(PersonalityProfile {
        openness: parse_facet(p.get("openness")?, window),
        conscientiousness: parse_facet(p.get("conscientiousness")?, window),
        extraversion: parse_facet(p.get("extraversion")?, window),
        agreeableness: parse_facet(p.get("agreeableness")?, window),
        neuroticism: parse_facet(p.get("neuroticism")?, window),
        updated_at: DateTime::now(),
        snapshots: vec![], // snapshot 在写回时基于旧 profile append，见 Step 5
    })
}
```

- [ ] **Step 4: 新 prompt key + 严谨 OCEAN 要求**

在 `prompts.rs` seed 两个 key（仿现有 `user.memory_consolidator.*` 的 seed 方式）：
- `user.personality_analyzer.system`：定位"严肃人格量表分析，基于大五 OCEAN"。
- `user.personality_analyzer.task`：5 条硬约束（设计 §"严谨科学提示词要求"）——①只输出 OCEAN 五维不许自创②每维必挂 evidenceTurns，无依据则 confidence=0③样本不足给低 confidence④行为锚定非贴标签⑤输出严格 JSON。输出 schema：
```
"personality": {
  "openness": {"score":0~1, "confidence":0~1, "evidenceTurns":[序号]},
  "conscientiousness": {...}, "extraversion": {...}, "agreeableness": {...}, "neuroticism": {...}
}
```
bump `PROMPT_PACK_VERSION`。确认 `ensure_prompt_pack_v2` 的 seed 列表加入新 key（grep ensure_prompt_pack_v2 看 prompt key 清单注册处）。

- [ ] **Step 5: 接入归并 + 写回（搭车，不额外 LLM 调用则合并进 consolidator 输出；否则单独一次）**

决策：人格作为**同一次归并 LLM 调用的额外输出段**（搭车，设计称"不额外起 LLM 调用"）——即把人格 schema 合并进 `user.memory_consolidator.task`，输出里多 `personality` 段。
> 备选：若 consolidator prompt 已很长、合并影响记忆归并质量，则单独一次 `generate_agent_json("user.personality_analyzer.task")`（多一次调用，但隔离清晰）。**实现时先试合并；若归并测试质量下降则拆开**，并在报告说明选择。

写回（memory.rs OCC 写入段，与 confirmed_tags 同一次 $set）：
```rust
if let Some(mut pp) = parse_personality(&value, &window) {
    // append snapshot：保留旧 snapshots + 本次（封顶 50）
    let mut snaps = contact.personality_profile.as_ref().map(|x| x.snapshots.clone()).unwrap_or_default();
    snaps.push(PersonalitySnapshot {
        consolidated_at: pp.updated_at,
        scores: vec![pp.openness.score, pp.conscientiousness.score, pp.extraversion.score, pp.agreeableness.score, pp.neuroticism.score],
        confidences: vec![pp.openness.confidence, pp.conscientiousness.confidence, pp.extraversion.confidence, pp.agreeableness.confidence, pp.neuroticism.confidence],
    });
    while snaps.len() > 50 { snaps.remove(0); }
    pp.snapshots = snaps;
    // set_doc.insert("personality_profile", to_bson(&pp)?)
}
```
> **解耦铁律**：人格写回只动 `personality_profile`，不碰其它线。

- [ ] **Step 6: 测试 + 编译 + 基线 + 提交**

Run: `cargo test --lib parse_personality` → passed。
Run: `cargo check --tests` → 0 errors。
Run: `cargo test --lib` → ≥ 350 / 0。
Run: `cargo test --test memory_card_invariants` → pass。
```bash
git add src/agent/memory.rs src/prompts.rs
git commit -m "feat(tag-trust): compression-time OCEAN personality analysis with strict prompt (子计划4 Task3)"
```

---

## Task 4：永不驱动铁律的守护测试

**Files:**
- Test: `src/agent/bayesian_slots.rs` 或新 `tests/` 注释级 + 一个静态守护测试

**Interfaces:**
- Produces: 一个文档化测试 / 注释，钉死"bayesian_signals 与 personality_profile 不进硬行为"。

- [ ] **Step 1: 写守护测试**

纯逻辑层无法直接"测一个字段没被某处读"，但可加防回归断言：grep 式静态检查不可靠，改为**契约测试** —— 验证 planner 的 candidate_filter / priority_key 不引用这两个字段（通过构造带 bayesian_signals 的 contact，断言 filter 输出与不带时一致）。

```rust
// 放在 planner 测试或新建：构造两个 contact，仅 bayesian_signals/personality_profile 不同，
// 断言 candidate_filter / priority_key 输出完全相同（证明这两字段不影响硬行为）。
#[test]
fn bayesian_and_personality_do_not_affect_planner_filters() {
    // 复用 planner 现有测试 helper 构造 contact；填充 vs 不填充 bayesian_signals，
    // 断言 stage_stagnation_candidate_filter / *_priority_key 结果一致。
}
```

> 实现者：复用 planner 现有测试构造方式（grep planner test helper）。若难以构造，退化为"在 bayesian_slots.rs 顶部写明铁律注释 + 在 review checklist 标注"，并在报告说明无法纯函数断言的原因。优先做契约测试。

- [ ] **Step 2: 运行 + 提交**

Run: `cargo test --lib bayesian_and_personality_do_not_affect`
Expected: passed。
```bash
git add <files>
git commit -m "test(tag-trust): guard bayesian/personality never drive planner hard behavior (子计划4 Task4)"
```

---

## Self-Review（写计划者自检）

**Spec 覆盖：**
- 6 槽上限 → Task 1（MAX_BAYESIAN_SLOTS + never_exceeds_six 测试）✓
- AI 自由发现维度 → Task 2（LLM 输出 dimension 自命名）✓
- 严谨两阶段占槽（多轮+强证据，一两句不占）→ Task 1（should_promote + single_mention_never 测试）✓
- 可配阈值 → Task 2 ✓
- 低置信淘汰 → **按 Option B / YAGNI 删除，未实现**（should_evict 不落地；永不驱动旁路，槽僵化零业务影响。见 D5-F2）
- history 封顶 100 → Task 1（history_capped 测试）✓
- 永不驱动 → Task 4（契约测试）✓
- 压缩时大五、只压缩更新 → Task 3（在 consolidate_inner 内）✓
- 封闭五维 + 证据强制 + 诚实置信 + 专属版本化 prompt → Task 3（parse 归 0 + 5 条 prompt 约束 + 新 key + bump）✓
- snapshot 演化封顶 → Task 3 Step 5（封顶 50）✓

**占位符扫描：** Task 1 Step 3 的 `apply_bayesian_update` 借用结构示意 + "实现以通过 4 个测试为准" —— 是有意把不变量交给测试钉死、借用细节留实现者（Rust 借用检查器会强制正确），非偷懒；4 个测试是硬验收。Task 3 Step 5 "先试合并失败则拆" 是真实的实现期质量判断点。Task 4 退化路径已说明。

**类型一致：** `BayesianSignal`/`BayesianPoint`/`PersonalityProfile`/`PersonalityFacet`/`PersonalitySnapshot`（子计划1）字段在 Task 1/3 引用一致；`ObservedDimension`/`SlotPromotionThreshold`/`BayesianObservationRaw` 本子计划新建并自洽；`resolve_evidence`（子计划2）复用 ✓。

**需实现期核实（已标注）：** RuntimeParametersTyped 加字段套路（同子计划3 Task2）、ensure_prompt_pack_v2 seed key 注册处、人格搭车 vs 单独调用的质量权衡、planner 测试 helper、current_turn 取值口径、ConversationMessage 是否有 is_inbound() 便捷方法（无则 matches! direction）。
