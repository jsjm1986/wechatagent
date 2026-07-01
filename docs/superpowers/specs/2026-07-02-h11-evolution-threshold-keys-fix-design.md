# H11+M9+L1 evolution threshold 重判修复设计

> 日期：2026-07-02
> 分支：`fix/h11-evolution-threshold-keys`（从 origin/main 545ffcf 切，含 H1/H7）
> 来源：终极审判审计 H11(UPHELD High) + M9(UPHELD Medium) + L1(UPHELD Low)——三者同在 `src/evolution/replay.rs` 的 threshold 重判路径，共根因，一并修。

## 1. 漏洞描述（三项，全部对最新代码亲验）

`src/evolution/replay.rs` 的 `evaluate_threshold`（:257-322）对每条 threshold 候选做「纯重判」：读源 run 的 `review.scores`，用候选阈值算 5 闸新命中向量 + `new_final_review_status`，供 significance 显著性测试对照。

### H11：score 键名不匹配 → 恒读到 0.0

`evaluate_single_gate`（:326-339）经 `gate_key_to_score_field`（:43-52）把 gate 映射成**单个** camelCase 键：`fact_risk_block → "factRisk"`、`product_accuracy_score_block → "productAccuracy"` 等，再 `scores.get_i32(field)`。

但 `review.scores` 的**真实序列化键**不是这些。数据流亲验：
- `agent_run_logs.review` = `to_document(&review)`（gateway.rs:2019/2152/2205/2323），`review: DecisionReviewResult`。
- `DecisionReviewResult.scores: ReviewScores`（types.rs:1168），`ReviewScores` 带 `#[serde(rename_all="camelCase")]`（types.rs:1136），字段 `hallucination_score` → 序列化键 **`hallucinationScore`**；`knowledge_grounding_score` → **`knowledgeGroundingScore`**。`factRisk`/`productAccuracy` 只是 `#[serde(alias)]`（types.rs:1144/1147），**仅反序列化生效，序列化绝不产生**。

所以生产 `review.scores` 里根本没有 `factRisk`/`productAccuracy` 键 → `scores.get_i32("factRisk")` miss → `.unwrap_or(0.0)` → **score 恒为 0.0**。后果：
- `fact_risk_block`（BLOCK_DIRECTION_GTE，`score >= threshold` 触发）：0.0 ≥ 6 恒 false → **恒不命中**。
- `product_accuracy_score_block`（REWRITE_DIRECTION_LT，`score < threshold` 触发）：0.0 < 7 恒 true → **恒命中**。
- 其余 `pressureRisk`/`humanLike`/`emotionalValue` 键名恰好与序列化键一致（无 alias、字段名 camelCase 化后同名），侥幸正确——**只有 factRisk/productAccuracy 两个带 alias 的 gate 中招**。

**铁证**：同模块的 `read_gate_score`（:371-389）**已经**用两套键名兼容 `fact_risk_block → ["factRisk","hallucinationScore"]`、`product_accuracy_score_block → ["productAccuracy","knowledgeGroundingScore"]`，且 i32/f64 都接——但它只被 prompt shadow 路径（`scores_to_5gate_hit` :404）用，**threshold 路径的 `evaluate_single_gate` 没复用它**，还在用坏掉的 `gate_key_to_score_field`。

### M9：original_5gate_hit 恒空 → 空基线偏拒

`evaluate_threshold`（:311）硬编码 `original_5gate_hit: Document::new()`（空）。而 significance 的 `compute_5gate_deltas`（significance.rs:415-419）用 `original_5gate_hit_or_default(gate)`（:464-466，缺 gate → false）算 `original_rate`，空文档 → 每个 gate 的 `original_rate` 恒 0 → `delta = new_rate - 0 = new_rate` 虚高 → `max_5gate_hit_increase` 偏大 → 更易触发 `gate_hit_increase_above_threshold` reject（significance.rs:170/202）。审计判据：**只误拒、不误放行**，故 Medium。

（`original_final_review_status` 用的是源 run 真实 `final_review_status`（:308），是 send_success 基线的 ground truth，**不受此影响、不动**。M9 只影响 5 闸涨幅这一项门。）

### L1：其它 4 gate 分支的 match 第一臂死代码

`evaluate_threshold`（:284-296）算 5 闸时，对「非被改」的 gate 走 else 分支：
```rust
} else {
    match proposal.current_value {
        Some(c) if proposal.gate_key.as_deref() == Some(gate) => {   // 死臂
            evaluate_single_gate(&scores, gate, c)
        }
        _ => evaluate_single_gate_default(&scores, gate),
    }
};
```
这个 else 分支的前提就是 `gate != gate_key`，故 guard `proposal.gate_key == Some(gate)` 恒 false → 第一臂**恒不可达（死代码）** → 永远走 `evaluate_single_gate_default`。注释也误导（声称用 current_value，实际从不用）。

### 现有测试为何是「假绿」

replay.rs 的 threshold 单测（:698/:720/:744/:811 等）全部用 `doc!{"factRisk": 6, ..., "productAccuracy": 9}` seed——**用的是生产从不产生的旧键名**。因为 `read_gate_score` 会接受 factRisk（向后兼容），若我把 evaluate_single_gate 换成 read_gate_score，这些测试仍绿；但即便在**当前坏代码**下它们也绿（坏代码恰好读 factRisk）——所以它们**从没测到真 bug**（生产键 hallucinationScore 读不到）。

## 2. 根因

`evaluate_single_gate` 用了**只认单个旧键名**的 `gate_key_to_score_field`，而正确的、双键兼容的 `read_gate_score` 就在同文件却没被复用。M9 的空基线与 L1 的死臂都在同一个「算 5 闸向量」的循环里，是同一段逻辑的三个缺陷。

## 3. 方案选型

### 方案 A（选定）：threshold 路径复用 read_gate_score + 补真基线 + 删死臂

三处改动，全在 `src/evolution/replay.rs`：

1. **H11**：`evaluate_single_gate` 改用 `read_gate_score(scores, gate)`（双键兼容）替代 `gate_key_to_score_field(gate)` + `scores.get_i32`。保留「缺分 → 0.0」语义（`.unwrap_or(0.0)`，与 prompt 路径 `scores_to_5gate_hit` 一致）。`gate_key_to_score_field` 随之零调用者 → 删函数 + 删其单测 `gate_key_field_mapping`。
2. **M9 + L1**：重写 `evaluate_threshold` 的 5 闸循环，**同时**产出 `original_5gate_hit`（被改 gate 用 `current_value`、其余用 default）和 `new_5gate_hit`（被改 gate 用 `proposed_value`、其余用 default）。这一次重写既填了真基线（M9），又消掉了那个 else 里的死 match 臂（L1，改成直白的 `if gate == gate_key { ... } else { 两侧都用 default }`）。`ReplayOutcome.original_5gate_hit` 从 `Document::new()` 改为真实向量。

**为什么选 A**：复用已存在的正确函数（read_gate_score），不新造；三缺陷同在一段逻辑，一次重写最小改动收全；`original_final_review_status`/send_success 基线不动，只修 5 闸涨幅这条被架空的门。

### 否决 B：给 evaluate_single_gate 的 gate_key_to_score_field 补第二个键
就是把 read_gate_score 的逻辑再抄一份进 gate_key_to_score_field。重复代码、两套键名映射漂移风险。既然 read_gate_score 已是正解，直接复用。否决。

### 否决 C：只修 H11，M9/L1 另开
三者同在 `evaluate_threshold` 一段循环，分开修要动同一段代码两次，且 M9 的真基线本就依赖 H11 修好的 evaluate_single_gate 才准确。合并修一次到位。否决。

## 4. 核心改动

落点：`src/evolution/replay.rs`。

### 4.1 evaluate_single_gate 改用 read_gate_score（:326-339）
```rust
fn evaluate_single_gate(scores: &Document, gate: &str, threshold: f64) -> bool {
    // 复用双键兼容的 read_gate_score（factRisk/hallucinationScore 等两套键名都读）；
    // 缺分 → 0.0，与 prompt 路径 scores_to_5gate_hit 的保守处理一致。
    let score = read_gate_score(scores, gate).unwrap_or(0.0);
    if BLOCK_DIRECTION_GTE.contains(&gate) {
        score >= threshold
    } else if REWRITE_DIRECTION_LT.contains(&gate) {
        score < threshold
    } else {
        false
    }
}
```
（行为对「缺分」保持 0.0 不变；唯一变化是现在能读到真实的 hallucinationScore/knowledgeGroundingScore。）

### 4.2 删除 gate_key_to_score_field（:42-52）+ 其单测 gate_key_field_mapping（:781-803）
零生产调用者（grep 确认仅 replay.rs 的 def/该 call/该 test）。

### 4.3 evaluate_threshold 的 5 闸循环重写（:276-320）
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
        original_5gate_hit,                       // M9:真基线(原为 Document::new())
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
（L1 死 match 臂随重写消失；`original_final_review_status` 仍取源 run 真实状态不变。）

**不动**：`read_gate_score`、`scores_to_5gate_hit`、`final_status_from_5gate`、`evaluate_single_gate_default`、`default_gate_threshold`、prompt shadow 路径、significance.rs（它的消费逻辑本就正确，只是此前拿到空基线）。

## 5. 行为验证（改动后）

| 场景 | 改动前 | 改动后 |
| --- | --- | --- |
| 源 scores 含真实键 `hallucinationScore` | evaluate_single_gate 读 factRisk→miss→0.0→fact_risk 恒不命中 | read_gate_score 读到真值→命中判断正确 |
| 源 scores 含真实键 `knowledgeGroundingScore` | 读 productAccuracy→miss→0.0→product 恒命中 | 读到真值→命中判断正确 |
| 旧键 `factRisk`(向后兼容/legacy 文档) | 读到 | read_gate_score 仍接受 factRisk→读到（不回归） |
| 5 闸涨幅 delta(M9) | original 恒空→原命中率恒 0→delta 虚高→偏拒 | original 用当前阈值算真命中→delta 准确 |
| 非被改 gate 的 delta | original 0、new=default 命中→非零虚假 delta | 两侧都 default→delta 恒 0（本 proposal 不动它们，正确） |
| send_success 基线 | 用源 run final_review_status | 不变（不受本次改动影响） |

## 6. 测试设计

改 `src/evolution/replay.rs` 的 `#[cfg(test)] mod tests`。**只增不删旧维度**（旧 factRisk 键测试保留——它们验证 read_gate_score 的 factRisk 向后兼容分支仍工作，是合法覆盖），仅删掉主体已消失的 `gate_key_field_mapping`。

**新增测试 1（H11 真护栏）——真实序列化键 hallucinationScore 能被读到并正确命中：**
seed `doc!{"hallucinationScore": 6, "knowledgeGroundingScore": 9, "pressureRisk":1, "humanLike":8, "emotionalValue":7}`（生产真实键），proposal 收紧 fact_risk_block 6→7 → 断言 `new_5gate_hit.fact_risk_block == false`（score=6 < 7 不命中）。**在坏代码下**：evaluate_single_gate 读 factRisk→miss→0.0→0 < 7 也 false，**恰好也 false，测不出差异**——所以这个 case 要用能区分的阈值：seed hallucinationScore=8，收紧 6→7，断言 new fact_risk_block==true（8≥7 命中）。坏代码下读 0.0→0≥7 false → 断言 true 失败。**真护栏。**

**新增测试 2（H11 product 方向）——knowledgeGroundingScore 正确读：**
seed `"knowledgeGroundingScore": 9`，product_accuracy_score_block（LT，score<threshold 命中），阈值 7 → 9<7 false 不命中。坏代码读 productAccuracy→miss→0.0→0<7 true 命中 → 断言 false 失败。真护栏。

**新增测试 3（M9 真基线）——original_5gate_hit 非空且正确：**
seed hallucinationScore 使被改 gate 在当前阈值下命中，proposal 放松该 gate → 断言 `outcome.original_5gate_hit` 非空、被改 gate 的 original 命中值正确（旧代码恒空 → get_bool 返 None → 断言失败）。

**基线影响**：这些是 replay.rs 内的 `#[cfg(test)]` **lib 单测**，进 `cargo test --lib` 计数，commit 时必须全绿（净 +2/+3 测试，删 1 个 gate_key_field_mapping）。lib ≥350/0 不回归。

## 7. 范围边界

- **不做（YAGNI）**：不改 significance.rs（消费逻辑本正确）、不改 prompt shadow 路径、不改 read_gate_score/scores_to_5gate_hit、不动 send_success 口径、不新增键名映射。
- **过拟合红线**：新测试锁「真实序列化键能被读到」「original 基线非空」两个真实不变量，不为过测试改任何阈值/业务逻辑。修的是让被架空的 #152 安全回归门与 5 闸涨幅门重新生效——这是让门**正确**，不是调松/调紧。
- **禁词 lint**：不涉禁词。
- **仅 lib 单测**：无需 Docker，本地可全跑验证。
