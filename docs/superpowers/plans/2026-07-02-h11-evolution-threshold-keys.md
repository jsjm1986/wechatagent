# H11+M9+L1 evolution threshold 重判修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修 `src/evolution/replay.rs` 的 threshold 重判路径三缺陷:H11(evaluate_single_gate 用坏掉的 gate_key_to_score_field 只读 factRisk/productAccuracy,而真实序列化键是 hallucinationScore/knowledgeGroundingScore → score 恒 0.0)、M9(original_5gate_hit 硬编码空 → 空基线偏拒)、L1(其它 4 gate 分支 match 第一臂死代码)。

**Architecture:** evaluate_single_gate 改用同模块已有的双键兼容 read_gate_score;evaluate_threshold 重写 5 闸循环同时产出真实 original/new 两向量(补 M9 真基线 + 消 L1 死臂);删零调用者的 gate_key_to_score_field 及其单测。只改 replay.rs 一个文件。send_success 基线(original_final_review_status)不动。

**Tech Stack:** Rust 2021 / `#[cfg(test)] mod tests` lib 单测(无需 Docker,本地可全跑验证)。

## Global Constraints

- 分支:`fix/h11-evolution-threshold-keys`(从 origin/main 545ffcf 切,含 H1/H7;spec commit 899372d 已在其上)。绝不 push main,只在 worktree `E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/e4-f21-closure` 干活。
- cargo 命令前:`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"` + `export CARGO_INCREMENTAL=0`。磁盘紧先删 `target/debug/incremental`。
- 基线不回归:`cargo test --lib` ≥ 350 passed / 0 failed。本任务新增/改动的是 replay.rs 内 lib 单测(进 lib 计数),commit 时必须全绿。
- 过拟合红线:绝不为过测试改业务逻辑/阈值。测试锁「真实序列化键能被读到」「original 基线非空且正确」两个真实不变量。修的是让被架空的 #152 安全回归门+5 闸涨幅门重新生效(让门正确,不是调松/调紧)。
- 禁词 lint:不涉禁词(人工/接管/takeover/hand-off)。
- commit:具名 `git add src/evolution/replay.rs`,绝不 -A/.;消息 `Co-Authored-By: Claude <noreply@anthropic.com>` 结尾。已授权 commit/push/PR/cron 监控 CI/squash 合并。
- 动手前先 Read `src/evolution/replay.rs` 确认 evaluate_single_gate / gate_key_to_score_field / evaluate_threshold 5闸循环 / read_gate_score / ReplayOutcome 构造 与本计划一致(行号可能漂移,以 string anchor 为准)。

---

## 文件结构

- **Modify:** `src/evolution/replay.rs` — (1) evaluate_single_gate 改用 read_gate_score;(2) 删 gate_key_to_score_field 函数 + 删单测 gate_key_field_mapping;(3) evaluate_threshold 5 闸循环重写(补 original_5gate_hit 真基线 + 消死臂);(4) 新增 3 个用真实序列化键 seed 的护栏单测。

单任务:三缺陷同在 evaluate_threshold 一段逻辑,一次内聚改动。TDD 在任务内:先写用真实键 seed 的护栏测试(旧 evaluate_single_gate 下 miss→断言失败),再改主体转绿,一次 commit。

---

## Task 1: H11+M9+L1 threshold 重判修复 + 真实键护栏测试

**Files:** Modify: `src/evolution/replay.rs`

**Interfaces:**
- Consumes: `read_gate_score(scores, gate) -> Option<f64>`(replay.rs:371,双键兼容,不改);`evaluate_single_gate_default(scores, gate) -> bool`(:345,不改);`default_gate_threshold(gate) -> Option<f64>`(:356,不改);`final_status_from_5gate(&Document) -> &str`(:421,不改);`BLOCK_DIRECTION_GTE`/`REWRITE_DIRECTION_LT`(:55/:63,不改);`Proposal.current_value/proposed_value/gate_key`。
- Produces: evaluate_single_gate/evaluate_threshold 行为修正;删除 gate_key_to_score_field。签名全不变。

- [ ] **Step 1: 动手前先读码验证(不猜)**

Read `src/evolution/replay.rs` 确认(以 string anchor 为准):
- `fn gate_key_to_score_field`(约 :43-52)当前存在,被 `evaluate_single_gate`(约 :327)唯一调用(+ 单测 gate_key_field_mapping 约 :782)。
- `fn evaluate_single_gate`(约 :326-339)当前 `let field = match gate_key_to_score_field(gate) {...}; let score = scores.get_i32(field)...`。
- `fn read_gate_score`(约 :371-389)双键兼容,i32/f64 都接,是正解。
- `evaluate_threshold`(约 :276-320)的 5 闸 for 循环 + `ReplayOutcome { ... original_5gate_hit: Document::new(), ... }`。
- 单测区(约 :662-)有 `mk_run_log(scores, final_status)` / `mk_threshold_proposal(gate, current, proposed)` helper。
若与本计划不符,以真实代码为准修正,report 记明分歧。

- [ ] **Step 2: 先写 3 个真实键护栏测试(TDD 红)**

在 replay.rs 的 `#[cfg(test)] mod tests` 内**追加**(不删旧的 factRisk 键测试——它们验证 read_gate_score 向后兼容分支仍工作)。用生产真实序列化键 `hallucinationScore`/`knowledgeGroundingScore` seed:

```rust
    /// H11 真护栏:源 run 用生产真实序列化键 `hallucinationScore` 时,
    /// evaluate_single_gate 必须读到它(旧代码读 factRisk→miss→0.0)。
    /// seed hallucinationScore=8,收紧 fact_risk_block 6→7 → new 命中(8≥7)。
    /// 旧 bug 下读 0.0→0≥7 false→断言 true 失败。
    #[test]
    fn evaluate_threshold_reads_real_hallucination_score_key() {
        let scores = doc! {
            "hallucinationScore": 8_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "knowledgeGroundingScore": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit.get_bool("fact_risk_block").unwrap(),
            true,
            "hallucinationScore=8 ≥ 7 应命中(旧代码读 factRisk miss→0.0→不命中)"
        );
    }

    /// H11 真护栏(product 方向):源 run 用真实键 knowledgeGroundingScore。
    /// product_accuracy_score_block 是 LT(score<threshold 命中),seed=9,阈值 7
    /// → 9<7 false 不命中。旧 bug 读 productAccuracy→miss→0.0→0<7 true 命中→断言 false 失败。
    #[test]
    fn evaluate_threshold_reads_real_knowledge_grounding_score_key() {
        let scores = doc! {
            "hallucinationScore": 0_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "knowledgeGroundingScore": 9_i32,
        };
        let run = mk_run_log(scores, "approved");
        let proposal = mk_threshold_proposal("product_accuracy_score_block", 7.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.new_5gate_hit.get_bool("product_accuracy_score_block").unwrap(),
            false,
            "knowledgeGroundingScore=9 ≥ 7 不该触发 product block(旧代码读 productAccuracy miss→0.0→<7 误命中)"
        );
    }

    /// M9 真护栏:original_5gate_hit 必须非空且正确(旧代码恒 Document::new())。
    /// seed hallucinationScore=8;放松 fact_risk_block 6→7(current=6,proposed=7)。
    /// original 用 current=6:8≥6 命中=true。旧代码 original_5gate_hit 空→get_bool None→失败。
    #[test]
    fn evaluate_threshold_fills_original_5gate_hit_baseline() {
        let scores = doc! {
            "hallucinationScore": 8_i32,
            "pressureRisk": 1_i32,
            "humanLike": 8_i32,
            "emotionalValue": 7_i32,
            "knowledgeGroundingScore": 9_i32,
        };
        let run = mk_run_log(scores, "held_by_ai_policy");
        let proposal = mk_threshold_proposal("fact_risk_block", 6.0, 7.0);
        let outcome = evaluate_threshold(&proposal, &run);
        assert!(outcome.completed);
        assert_eq!(
            outcome.original_5gate_hit.get_bool("fact_risk_block"),
            Ok(true),
            "original 用 current_value=6:hallucinationScore=8≥6 命中(旧代码恒空→None)"
        );
        // new 用 proposed=7:8≥7 仍命中。
        assert_eq!(outcome.new_5gate_hit.get_bool("fact_risk_block"), Ok(true));
    }
```

- [ ] **Step 3: 跑测试确认红(TDD 红)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib evolution::replay 2>&1 | tail -25`
Expected: 3 个新测试**红**——`reads_real_hallucination_score_key`(旧读 factRisk miss→0.0→不命中→断言 true 失败)、`reads_real_knowledge_grounding_score_key`(旧读 productAccuracy miss→0.0→误命中→断言 false 失败)、`fills_original_5gate_hit_baseline`(旧 original_5gate_hit 空→get_bool None≠Ok(true) 失败)。看到红即证明护栏有效。

- [ ] **Step 4: evaluate_single_gate 改用 read_gate_score(绿·其一)**

old_string(约 :326-331):
```rust
fn evaluate_single_gate(scores: &Document, gate: &str, threshold: f64) -> bool {
    let field = match gate_key_to_score_field(gate) {
        Some(f) => f,
        None => return false,
    };
    let score = scores.get_i32(field).ok().map(|v| v as f64).unwrap_or(0.0);
```
new_string:
```rust
fn evaluate_single_gate(scores: &Document, gate: &str, threshold: f64) -> bool {
    // 复用双键兼容的 read_gate_score(factRisk/hallucinationScore 等两套键名都读);
    // 缺分 → 0.0,与 prompt 路径 scores_to_5gate_hit 的保守处理一致。
    let score = read_gate_score(scores, gate).unwrap_or(0.0);
```

- [ ] **Step 5: 删 gate_key_to_score_field 函数(绿·其二)**

删除整个 `fn gate_key_to_score_field`(约 :42-52,含其上方 `/// 5 闸 gate_key → review.scores BSON 字段(camelCase)映射。` doc 注释)。删后零调用者(Step 4 已改)。

- [ ] **Step 6: evaluate_threshold 5 闸循环重写(绿·其三,补 M9+消 L1)**

old_string(约 :276-320,从 `let mut new_5gate_hit = Document::new();` 到 `ReplayOutcome { ... new_5gate_hit, }` 整块)——先 Read 确认精确边界,再替换为:
```rust
    let mut original_5gate_hit = Document::new();
    let mut new_5gate_hit = Document::new();
    for gate in [
        "fact_risk_block",
        "pressure_risk_block",
        "human_like_score_rewrite",
        "emotional_value_rewrite",
        "product_accuracy_score_block",
    ] {
        if gate == gate_key {
            // 被改的 gate:original 用当前生效阈值(current_value,缺则 default)、
            // new 用 proposed_value。两侧对同一源 scores 的差异只来自阈值变化。
            let current = proposal
                .current_value
                .or_else(|| default_gate_threshold(gate))
                .unwrap_or(0.0);
            original_5gate_hit.insert(gate, evaluate_single_gate(&scores, gate, current));
            new_5gate_hit.insert(gate, evaluate_single_gate(&scores, gate, new_value));
        } else {
            // 其余 4 个 gate 本 proposal 不动 → 两侧都用 default 阈值,delta 恒 0。
            let hit = evaluate_single_gate_default(&scores, gate);
            original_5gate_hit.insert(gate, hit);
            new_5gate_hit.insert(gate, hit);
        }
    }

    let new_final = final_status_from_5gate(&new_5gate_hit);

    ReplayOutcome {
        completed: true,
        failure_reason: None,
        original_final_review_status: Some(original.final_review_status.clone()),
        original_5gate_hit,
        original_self_critique_addressed: None,
        new_final_review_status: Some(new_final.to_string()),
        new_review_risks: Vec::new(),
        new_token_cost: Some(0),
        new_self_critique_addressed: Some(matches!(
            new_final,
            "approved" | "approved_after_revision"
        )),
        new_5gate_hit,
    }
```
注意:Read 时确认原块里 `original_5gate_hit: Document::new()` 与那段"threshold 路径不推 original 5 闸"的误导注释在替换范围内被移除。

- [ ] **Step 7: 删单测 gate_key_field_mapping**

删除整个 `#[test] fn gate_key_field_mapping()`(约 :781-803,含其 doc 注释)——被测函数已删。

- [ ] **Step 8: 跑测试确认全绿(TDD 绿)**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib evolution::replay 2>&1 | tail -25`
Expected: 全绿(3 个新护栏测试 + 旧的 factRisk 键测试都过——旧测试因 read_gate_score 接受 factRisk 向后兼容仍绿)。若红,读断言消息核对,绝不为过测试塞假数据。

- [ ] **Step 9: 跑全量 lib 基线**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" CARGO_INCREMENTAL=0; cargo test --lib 2>&1 | tail -6`
Expected: `test result: ok. N passed; 0 failed`,N ≥ 350(净 +2 测试:新增 3 删 1)。

- [ ] **Step 10: Commit**
```bash
git add src/evolution/replay.rs
git commit -m "$(cat <<'EOF'
fix(evolution): threshold 重判读真实 score 键+补真基线+删死码(H11/M9/L1)

H11:evaluate_single_gate 经 gate_key_to_score_field 只读 factRisk/
productAccuracy,但 ReviewScores rename_all=camelCase 序列化的真实键是
hallucinationScore/knowledgeGroundingScore(factRisk 仅反序列化 alias)。
生产文档无 factRisk→get_i32 miss→score 恒 0.0→fact_risk 恒不命中、product
恒命中→#152 安全回归门+5 闸涨幅门被架空。改用同模块已有的双键兼容
read_gate_score;gate_key_to_score_field 零调用者→删函数+删单测。

M9:evaluate_threshold 的 original_5gate_hit 硬编码 Document::new()→
significance compute_5gate_deltas 的 original_rate 恒 0→delta 虚高→偏拒。
重写 5 闸循环同时产出真实 original(用 current_value)/new(用 proposed)两向量。

L1:其它 4 gate 分支 match 第一臂 guard 恒 false(死代码)+误导注释;
重写后改成直白 if gate==gate_key/else 两侧 default,死臂消失。

新增 3 个用真实序列化键 seed 的护栏测试(旧代码下 miss→断言失败)。
只改 src/evolution/replay.rs;significance/prompt shadow/send_success 基线不变。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage:** §4.1 evaluate_single_gate→Step4 ✓;§4.2 删函数+单测→Step5/7 ✓;§4.3 循环重写→Step6 ✓;§6 三护栏测试→Step2 ✓。
**Placeholder scan:** 无 TBD;每步给完整 old/new_string 或明确删除目标;commit 消息完整。
**Type consistency:** `read_gate_score -> Option<f64>`,`.unwrap_or(0.0) -> f64`,与 threshold: f64 比较 ✓;`get_bool -> Result<bool, _>`,测试用 `Ok(true)` / `.unwrap()` 匹配 ✓;`original_5gate_hit: Document`,`.insert(gate, bool)` ✓;`evaluate_single_gate_default -> bool` ✓。
**注意(TDD 红态):** Step 2 后 3 测试红是预期(证明真护栏),Step 4-7 修后转绿;commit(Step10)在全绿+基线不回归后。
