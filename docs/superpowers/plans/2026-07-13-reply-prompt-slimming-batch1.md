# 运营 Agent 回复 prompt 瘦身 · 批次 1（零风险清理）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从 `user.reply.task` prompt 移除 4 个死字段声明 + 从 reply 决策 user 段移除 2 处纯冗余注入槽（硬运行参数、context_pack 重复）+ 裁掉知识路由的 3 个纯调试元数据字段，在零信息损失、零行为改变的前提下削减单次 LLM 调用的 prompt token。

**Architecture:** 全部改动落在两个文件——`src/prompts.rs`（reply.task 模板文本，删 4 死字段的 schema 声明）与 `src/agent/decision.rs`（`decide_reply_with_promote` 的 user `format!` 拼装块，删槽5/槽7 两个位置参数、把槽11 从整对象序列化换成新的 `format_knowledge_route_for_prompt` 纯函数裁剪）。不动任何 struct 字段、不动 `RawAgentDecision`/`AgentDecision`、不动 carry_through、不动函数签名。每一处删除都靠 `cargo build`（位置参数配平）+ 精确单测（模板不含死字段键 / 路由函数只输出保留字段）双重锁定。

**Tech Stack:** Rust 2021 / Axum；测试用 `cargo test --lib`；prompt 模板断言复用现有 `prompt_specs()` + `.find(|s| s.key == "user.reply.task")` 范式（prompts.rs 测试区）。

---

## 背景与已亲验事实（写 plan 前逐一 Read/Grep 确认，非猜测）

- **4 死字段**：`intentAnalysis`（prompts.rs:1249）/ `productFitScore`（:1330）/ `forbiddenClaimRisk`（:1333）/ `recommendedResourceIds`（:1335）。全库 grep（2026-07-13）：仅出现于 `types.rs`（struct 定义 + `Default` + `RawAgentDecision` + `validate_and_promote` 的 carry_through 透传）、`prompts.rs`（模板声明）、`gates.rs`（`intentAnalysis` 恰是 reviewer **排除**项，:1296）。无任何 guard / 阈值 / 发送逻辑读取其值。四者在 `RawAgentDecision` 全是 `Option<...>`（types.rs:479/484/486/488），LM 不输出 → 反序列化 None → carry_through 拿 None/空 → 零 promote 影响、零闸门影响。**故从模板删除字段声明 = LM 不再输出 = 零行为改变。struct 字段保留不动**（删字段要动 carry_through/AgentDecision，风险大于收益）。
- **user `format!` 是位置参数**（decision.rs:841-973）：模板串里每个 `{}` 按顺序绑定 921-972 的实参。删一个槽必须**同时删**「标签行 + `{}` 占位 + 对应实参」三处配对，否则位置错位、后续槽全部错填。
- **槽5 runtime_text**（decision.rs:479-483，实参 :925）：`serde_json::to_string(&runtime.as_document())`，系统运行参数（recentMessageLimit 等）。`runtime` 参数在 :998/:1000（`validate_and_promote(runtime)`）仍被消费 → 删注入文本**不会**造成 unused 变量，不动函数签名。
- **槽7 memory_card_text**（decision.rs:545-549，实参 :927）：`serde_json::to_string(context_pack)`。`context_pack` 已完整嵌在**槽6 memory_text**（:533-544）的 `"memoryCard": context_pack.clone()` 里（:535）→ 槽7 是同一份的第二次注入，纯冗余。doNotDo/commitments 也在槽6 的 memoryCard 内（:527-528 注释佐证），删槽7 安全语义不丢。
- **槽11 knowledge_route_text**（decision.rs:489-493，实参 :931）：现为 `serde_json::to_string(knowledge_route)`，序列化整个 `KnowledgeRouteResult`（13 字段）。其中 `tool_trace`（Vec<Document>）/ `evidence_excerpts`（Vec<String>）/ `selected_chunk_rankings`（Vec<SelectedChunkRanking>，注释明说"只采集落库、不参与任何加权"，types.rs:1363-1371）是**纯调试/落库元数据**，回复文本生成不消费。其余 10 字段（needed_categories / selected_*_ids / selected_slice_reasons / risk_level / requires_evidence / knowledge_coverage / missing_knowledge / reason）对 LLM 有语义，**保留**。
- **`KnowledgeRouteResult` 带 `#[serde(rename_all = "camelCase")]`**（types.rs:1336-1338）+ `Serialize`（:1336 derive）→ 现产出 camelCase key（`neededCategories` / `selectedKnowledgeIds` / `knowledgeCoverage` ...）。新裁剪函数**必须保持 camelCase**，否则等于偷改 LLM 看到的字段名。
- **`knowledge_route.xxx` 的 Rust 字段访问**（gateway.rs:1229/1244/1279/1291/1443、prompt_shadow.rs:311、simulation.rs:175）与 prompt 文本序列化**无关** → 裁剪槽11 不影响这些代码路径。
- **无整串 user prompt 黄金快照测试**：grep "逐字等价 / 字节等价"（prompts.rs / decision.rs）全部是 **tier 降级 vs 历史基线**的函数级护栏（`render_safety_donts_commitments` 空串、`render_business_context_fragment` None→空、`assemble_system_prompt` 层序），**没有**断言整条 user 串字节的快照 → 批次1 改动不会撞隐藏红测。删槽5/7 会改变 Full 档 user 串形态，这是**本 spec 的预期改动**（瘦身），非违反等价护栏（那些护栏约束的是"tier/profile 不该引入差异"，不是"prompt 永不变"）。
- **测试接缝**：`prompt_specs()`（prompts.rs:1054，私有）返回 `Vec<PromptSpec>`，同模块 `#[cfg(test)]` 可用，现有测试 `reply_task_prompt_offers_only_final_phase`（:2583）/ `reply_schema_requests_evidence_turns`（:2650）就是 `.find(|s| s.key == "user.reply.task")` 后 `assert!(task.content.contains(...))`。删死字段的断言用 `!task.content.contains("死字段键")` 同范式。

## File Structure

- **`src/prompts.rs`** — 删 reply.task 模板里 4 死字段的 schema 声明行 + 各自注释；在测试区新增 1 个断言测试（模板不含 4 死字段键）。
- **`src/agent/decision.rs`** — ① 删 user `format!` 的槽5（runtime）标签行+占位+实参、槽7（memory_card）标签行+占位+实参；② 删槽5 的 `runtime_text` binding（:479-483）与槽7 的 `memory_card_text` binding（:545-549）；③ 新增纯函数 `format_knowledge_route_for_prompt(&KnowledgeRouteResult) -> String`，把槽11 实参从 `knowledge_route_text` 换成该函数调用（`knowledge_route_text` binding 相应改为调用新函数或删除原 binding 改用新函数）；④ 测试区新增 2 个纯函数单测。

## 提交纪律

TDD + 频繁提交。每个 Task 末尾 commit。**未经用户显式许可不 push、不建 PR**（本项目红线）。commit message 末尾附 `Co-Authored-By: Claude <noreply@anthropic.com>`。

---

## Task 1: 从 reply.task 模板删除 4 个死字段声明

**Files:**
- Modify: `src/prompts.rs`（reply.task 模板：删 `intentAnalysis` 块 1249-1254、`productFitScore` 行 1330、`forbiddenClaimRisk` 行 1333、`recommendedResourceIds` 行 1335）
- Test: `src/prompts.rs`（测试区，仿 `reply_task_prompt_offers_only_final_phase` :2583）

**死字段精确边界（已亲验）：**
- `intentAnalysis` 是**独立对象块**，占 6 行（prompts.rs:1249-1254）：
  ```
    "intentAnalysis": {
      "userIntent": "用户此刻真实意图",
      "emotionalState": "用户情绪",
      "relationshipMoment": "陪伴/解释/推进/等待/修复",
      "shouldAdvance": false
    },
  ```
- `productFitScore`（:1330）/ `forbiddenClaimRisk`（:1333）/ `recommendedResourceIds`（:1335）是**三条单行**，夹在存活字段之间。周边存活字段**必须保留**：`memoryWriteScore`(:1329)、`matchedKnowledgeIds`(:1331)、`safeClaimsUsed`(:1332)、`objectionsDetected`(:1334)、`usedKnowledgeIds`(:1336)。当前 1329-1336 连续 8 行：
  ```
    "memoryWriteScore": 0,
    "productFitScore": 0,          ← 删
    "matchedKnowledgeIds": [],
    "safeClaimsUsed": [],
    "forbiddenClaimRisk": 0,       ← 删
    "objectionsDetected": [],
    "recommendedResourceIds": [],  ← 删
    "usedKnowledgeIds": [],
  ```

- [ ] **Step 1: 写失败测试**

在 `src/prompts.rs` 末尾 `#[cfg(test)] mod ...` 区（紧邻 `reply_task_prompt_offers_only_final_phase` 所在 mod，即 :2599 那个 `}` 之前）加：

```rust
    /// 批次1 瘦身护栏：4 个死字段（全库无任何 guard/阈值/发送逻辑消费，仅 types.rs
    /// carry_through 透传 None）已从 reply.task 契约删除 → LM 不再输出、不再占 token。
    /// 断真 prompt pack 文本防死字段回流。struct 字段保留（Option 透传无害），故这里
    /// 只断模板 schema 不含这些 wire key。
    #[test]
    fn reply_task_prompt_drops_dead_fields() {
        let specs = prompt_specs();
        let task = specs
            .iter()
            .find(|s| s.key == "user.reply.task")
            .expect("user.reply.task prompt spec 存在");
        for dead in ["intentAnalysis", "productFitScore", "forbiddenClaimRisk", "recommendedResourceIds"] {
            assert!(
                !task.content.contains(dead),
                "reply.task 模板不应再声明死字段 {dead}（无消费点，白占 token）"
            );
        }
        // 存活字段仍在（防误删相邻行）
        for keep in ["memoryWriteScore", "matchedKnowledgeIds", "safeClaimsUsed", "objectionsDetected", "usedKnowledgeIds"] {
            assert!(task.content.contains(keep), "存活字段 {keep} 被误删");
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib reply_task_prompt_drops_dead_fields`
Expected: FAIL —— 模板当前仍含 `intentAnalysis` 等键，`!contains` 断言不成立。

- [ ] **Step 3: 删模板里的 4 死字段**

用 Edit 工具在 `src/prompts.rs` 的 reply.task 模板串内：
1. 删 `intentAnalysis` 整块（连同前导 2 空格缩进的 `"intentAnalysis": {` 到 `},` 共 6 行）。
2. 删单行 `  "productFitScore": 0,`
3. 删单行 `  "forbiddenClaimRisk": 0,`
4. 删单行 `  "recommendedResourceIds": [],`

注意：Edit 的 old_string 要带足上下文唯一定位（如 `intentAnalysis` 块用前一行 `},`+ 块 + 后一行 `"profileUpdate"` 锚定；三条单行各自带前后相邻存活行锚定），不改动任何存活字段的缩进/逗号。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib reply_task_prompt_drops_dead_fields`
Expected: PASS

- [ ] **Step 5: 跑既有 reply.task 护栏测试确认没误伤**

Run: `cargo test --lib reply_task_prompt_offers_only_final_phase reply_schema_requests_evidence_turns`
Expected: PASS（final 形态契约 + tagEvidenceTurns/stageEvidenceTurns/stageExplicitIntent/bayesianObservations 仍在，证明只删了死字段没伤到 schema 主体）

- [ ] **Step 6: Commit**

```bash
git add src/prompts.rs
git commit -m "$(cat <<'EOF'
perf(reply-prompt): 批次1① reply.task 契约删4死字段(无消费点)

intentAnalysis/productFitScore/forbiddenClaimRisk/recommendedResourceIds
全库仅 types.rs carry_through 透传 None,无任何 guard/阈值/发送逻辑读取;
intentAnalysis 更是 reviewer 排除项。从模板删声明→LM 不再输出→零行为
改变、削 prompt token。struct 字段保留(Option 透传无害)。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: 删 user 段槽5（硬运行参数）与槽7（context_pack 重复注入）

**Files:**
- Modify: `src/agent/decision.rs`（`decide_reply_with_promote`：删 `runtime_text` binding :479-483、`memory_card_text` binding :545-549；删 `format!` 模板串里「硬运行参数:」标签段+「长期记忆卡片:」标签段各含一个 `{}`；删实参列表里 `runtime_text`(:925) 与 `memory_card_text`(:927)）

**位置参数配对（已亲验 decision.rs:841-973）——三处必须同批删，否则槽位错位：**

槽5「硬运行参数」在模板串（:853-854）：
```
硬运行参数:
{}

```
对应实参 `runtime_text`（:925）。

槽7「长期记忆卡片」在模板串（:859-860）：
```
长期记忆卡片:
{}

```
对应实参 `memory_card_text`（:927）。

**注意槽6「长期运营记忆」（:856-857 标签 + :926 实参 `memory_text`）必须保留**——它才是 context_pack 的**权威注入**（:535 `"memoryCard": context_pack.clone()`）。删的是槽7 这份重复。

- [ ] **Step 1: 删两个 binding**

用 Edit 删 `src/agent/decision.rs:479-483`：
```rust
    let runtime_text = if include_business {
        serde_json::to_string(&runtime.as_document()).unwrap_or_default()
    } else {
        String::new()
    };
```
（整段删除。`runtime` 参数在 :998/:1000 仍被 `validate_and_promote` 消费，不会 unused。）

用 Edit 删 `src/agent/decision.rs:545-549`：
```rust
    let memory_card_text = if include_relational {
        serde_json::to_string(context_pack).unwrap_or_default()
    } else {
        String::new()
    };
```
（整段删除。`context_pack` 在槽6 :535 仍被使用，不会 unused。）

- [ ] **Step 2: 删模板串的两个标签段**

用 Edit 在 `format!` 模板串里删（连同其后一个空行，保持段间单空行不乱）：

删「硬运行参数」段——old_string 锚定：
```
运营状态机:
{}

硬运行参数:
{}

长期运营记忆:
```
new_string：
```
运营状态机:
{}

长期运营记忆:
```

删「长期记忆卡片」段——old_string 锚定：
```
长期运营记忆:
{}

长期记忆卡片:
{}

最近 5 条已弃用记忆（不要再引用，仅供识别变化）:
```
new_string：
```
长期运营记忆:
{}

最近 5 条已弃用记忆（不要再引用，仅供识别变化）:
```

- [ ] **Step 3: 删实参列表里的两个变量**

用 Edit 在实参列表（:921-972）删 `runtime_text,`（:925）与 `memory_card_text,`（:927）两行。删后实参顺序：
```
        task_template,
        playbook_text,
        domain_text,
        state_machine_text,
        memory_text,
        serde_json::to_string(&deprecated_facts_recent).unwrap_or_default(),
```
（即 `state_machine_text` 后直接跟 `memory_text`，中间不再有 `runtime_text`/`memory_card_text`。）

- [ ] **Step 4: cargo build 验证位置参数配平**

Run: `cargo build --lib`
Expected: 编译通过。若模板 `{}` 数与实参数不匹配，`format!` 宏会在编译期报 "argument never used" / "invalid reference to positional argument" —— 编译绿即证明标签/占位/实参三处配平、无槽位错位。

- [ ] **Step 5: 跑 lib 基线确认不回归**

Run: `cargo test --lib`
Expected: ≥350 passed, 0 failed（CLAUDE.md 基线门）。特别关注 decision.rs 测试区的 `assemble_system_prompt_*` / `render_safety_donts_commitments` 相关测试仍 PASS（它们锁 system 段与 Lean 档安全子片，与本改动无关，应不受影响）。

- [ ] **Step 6: check-no-human-takeover lint**

Run: `bash scripts/check-no-human-takeover.sh`（或 Windows `pwsh scripts/check-no-human-takeover.ps1`）
Expected: 绿。本改动纯删注入槽，新增行仅 Edit 后的锚定上下文（无禁词），应通过。

- [ ] **Step 7: Commit**

```bash
git add src/agent/decision.rs
git commit -m "$(cat <<'EOF'
perf(reply-prompt): 批次1② 删 reply user 段槽5(运行参数)/槽7(context_pack 重复)

槽5 runtime_text=系统运行参数(recentMessageLimit 等)对回复语义无用;
槽7 memory_card_text 是 context_pack 的第二次注入,权威份已在槽6
memory_text 的 memoryCard 字段(含 doNotDo/commitments)→删槽7零信息损失。
位置参数三处配对同删,cargo build 保证配平。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 槽11 知识路由裁剪（新增纯函数 `format_knowledge_route_for_prompt` + 单测）

**Files:**
- Modify: `src/agent/decision.rs`（新增 `pub(crate) fn format_knowledge_route_for_prompt(route: &KnowledgeRouteResult) -> String`；把 `knowledge_route_text` binding :489-493 改为调用它；测试区新增 2 个单测）

**已亲验（types.rs:1336-1372）：** `KnowledgeRouteResult` 带 `#[serde(rename_all = "camelCase")]` + `Serialize`。现槽11 `serde_json::to_string(knowledge_route)`（:490）产出全 13 字段的 camelCase JSON。要删的 3 个纯调试/落库字段：`tool_trace`→`toolTrace`、`evidence_excerpts`→`evidenceExcerpts`、`selected_chunk_rankings`→`selectedChunkRankings`。保留其余 10 字段，key 大小写**必须与现状一致**（camelCase），否则偷改 LLM 契约。

**实现策略：** 不手写 10 个字段的 `json!`（易漏字段/拼错大小写、且未来加字段会漂）。改用「serde 序列化成 `serde_json::Value` → 从 Map 里 `remove` 掉 3 个 camelCase key → 再 `to_string`」——保证保留字段的 key 大小写与 struct `rename_all` 派生完全一致，只精确剔除 3 个调试字段。

- [ ] **Step 1: 写失败测试（红）**

在 `src/agent/decision.rs` 测试区（`#[cfg(test)] mod tests` 内，与现有 `assemble_system_prompt_*` 测试同级）追加：

```rust
    /// 批次1③：知识路由注入槽只喂 LLM 有语义的字段，剔除 3 个纯调试/落库元数据
    /// （toolTrace / evidenceExcerpts / selectedChunkRankings）——它们回复文本生成不消费
    /// （selectedChunkRankings 注释明说"只采集落库、不参与加权"，types.rs:1363-1371）。
    #[test]
    fn format_knowledge_route_drops_debug_metadata() {
        use crate::agent::types::{KnowledgeRouteResult, SelectedChunkRanking};
        let route = KnowledgeRouteResult {
            needed_categories: vec!["product".to_string()],
            selected_knowledge_ids: vec!["k1".to_string()],
            selected_chunk_ids: vec!["c1".to_string()],
            knowledge_coverage: "full".to_string(),
            reason: "命中产品事实切片".to_string(),
            requires_evidence: true,
            missing_knowledge: vec!["定价细则".to_string()],
            // —— 应被剔除的 3 个调试字段（给非空值，断言输出里不出现）——
            tool_trace: vec![mongodb::bson::doc! { "tool": "search" }],
            evidence_excerpts: vec!["某条摘录".to_string()],
            selected_chunk_rankings: vec![SelectedChunkRanking {
                chunk_id: "c1".to_string(),
                rank: 0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = format_knowledge_route_for_prompt(&route);
        // 保留字段（camelCase，与 rename_all 派生一致）在
        assert!(out.contains("neededCategories"), "保留 neededCategories");
        assert!(out.contains("selectedKnowledgeIds"), "保留 selectedKnowledgeIds");
        assert!(out.contains("knowledgeCoverage"), "保留 knowledgeCoverage");
        assert!(out.contains("missingKnowledge"), "保留 missingKnowledge");
        assert!(out.contains("命中产品事实切片"), "保留 reason 内容");
        // 3 个调试字段的 key 与其内容都不得出现
        assert!(!out.contains("toolTrace"), "剔除 toolTrace");
        assert!(!out.contains("evidenceExcerpts"), "剔除 evidenceExcerpts");
        assert!(!out.contains("selectedChunkRankings"), "剔除 selectedChunkRankings");
        assert!(!out.contains("某条摘录"), "剔除 evidenceExcerpts 内容");
    }

    /// 空路由（全默认）不 panic、产合法 JSON、仍不含调试字段 key。
    #[test]
    fn format_knowledge_route_empty_is_valid_json_without_debug_keys() {
        use crate::agent::types::KnowledgeRouteResult;
        let out = format_knowledge_route_for_prompt(&KnowledgeRouteResult::default());
        let parsed: serde_json::Value =
            serde_json::from_str(&out).expect("产出必须是合法 JSON");
        assert!(parsed.is_object(), "产出应为 JSON 对象");
        assert!(!out.contains("toolTrace"));
        assert!(!out.contains("evidenceExcerpts"));
        assert!(!out.contains("selectedChunkRankings"));
    }
```

- [ ] **Step 2: 跑测试确认失败（红）**

Run: `cargo test --lib format_knowledge_route`
Expected: 编译失败——`format_knowledge_route_for_prompt` 未定义（`cannot find function`）。这确认测试真的在测新函数。

- [ ] **Step 3: 写最小实现（绿）**

在 `src/agent/decision.rs` 恰当位置（建议紧邻其它 `format_*_for_prompt` 函数，如 `format_playbook_for_prompt` :1230 附近，同一 `pub(crate) fn` 可见性）新增：

```rust
/// 批次1③：知识路由注入槽的裁剪渲染。原槽11 直接 `serde_json::to_string(route)` 会把
/// `toolTrace` / `evidenceExcerpts` / `selectedChunkRankings` 三个**纯调试/落库元数据**
/// 一并喂给 LLM（`selectedChunkRankings` 注释明说只采集落库、不参与加权，types.rs:1363-1371），
/// 回复文本生成不消费。这里序列化后精确 remove 这 3 个 camelCase key，保留其余 10 个
/// 对 LLM 有语义的字段，且 key 大小写完全沿用 `KnowledgeRouteResult` 的 `rename_all` 派生
/// （不手写 json! 以免字段漂移/大小写拼错，避免偷改 LLM 看到的字段名）。
pub(crate) fn format_knowledge_route_for_prompt(route: &KnowledgeRouteResult) -> String {
    let mut value = match serde_json::to_value(route) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    if let Some(map) = value.as_object_mut() {
        map.remove("toolTrace");
        map.remove("evidenceExcerpts");
        map.remove("selectedChunkRankings");
    }
    serde_json::to_string(&value).unwrap_or_default()
}
```

- [ ] **Step 4: 把槽11 binding 换成调用新函数**

用 Edit 改 `src/agent/decision.rs:489-493`：
```rust
    let knowledge_route_text = if include_business {
        serde_json::to_string(knowledge_route).unwrap_or_default()
    } else {
        String::new()
    };
```
改为：
```rust
    let knowledge_route_text = if include_business {
        format_knowledge_route_for_prompt(knowledge_route)
    } else {
        String::new()
    };
```
（`knowledge_route_text` 仍是槽11 的实参 :931，无需动模板串/实参列表——只换 binding 的赋值来源。位置参数不变。）

- [ ] **Step 5: 跑测试确认通过（绿）**

Run: `cargo test --lib format_knowledge_route`
Expected: 2 passed, 0 failed。

- [ ] **Step 6: 跑 lib 基线 + build**

Run: `cargo build --lib && cargo test --lib`
Expected: 编译绿；lib ≥350 passed, 0 failed。

- [ ] **Step 7: check-no-human-takeover lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: 绿（新增函数/测试无禁词）。

- [ ] **Step 8: Commit**

```bash
git add src/agent/decision.rs
git commit -m "$(cat <<'EOF'
perf(reply-prompt): 批次1③ 知识路由注入裁掉3个纯调试元数据字段

新增 format_knowledge_route_for_prompt:序列化后 remove toolTrace/
evidenceExcerpts/selectedChunkRankings(纯采集落库、回复不消费),保留
其余10个有语义字段,key 大小写沿用 rename_all 派生不偷改 LLM 契约。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: 批次1 整体验证 + 基线门

**Files:** 无新改动，纯验证。

- [ ] **Step 1: 全 lib 测试 + build**

Run: `cargo build --lib && cargo test --lib`
Expected: 编译绿；`cargo test --lib` ≥350 passed, 0 failed（`scripts/check-baseline` 的 LIB_BASELINE=350，只增不减）。

- [ ] **Step 2: `cargo check --tests`（确认集成测试签名不炸）**

Run: `cargo check --tests`
Expected: 绿。改了 decision.rs 的 user format! 结构与新增函数，需确认没有集成测试（tests/）引用被删的注入行为或旧签名。

- [ ] **Step 3: 4 个 PBT 文件不回归**

Run: `cargo test --test state_transition_pbt && cargo test --test memory_card_invariants && cargo test --test wiki_chunk_revision_pbt && cargo test --test llm_retry_jitter`
Expected: 累计 ≥33 passed, 0 failed（check-baseline 第二门）。
说明：本批不碰 state machine / memory card / wiki chunk / llm retry 逻辑，理论上零影响；跑一遍确认。

- [ ] **Step 4: check-no-human-takeover lint（全 diff）**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: 绿。本批删的是死字段/冗余注入，无禁词新增行。

- [ ] **Step 5: 人工核对模板 diff（防误删存活字段）**

Run: `git diff HEAD~3 -- src/prompts.rs`
逐行确认：只删了 `intentAnalysis` 块（原 1249-1254）+ `productFitScore` / `forbiddenClaimRisk` / `recommendedResourceIds` 三条单行；**存活字段** `memoryWriteScore` / `matchedKnowledgeIds` / `safeClaimsUsed` / `objectionsDetected` / `usedKnowledgeIds` 一条不少（它们夹在被删行之间，最易误删）。

- [ ] **Step 6: 人工核对 decision.rs format! 配平**

Run: `git diff HEAD~3 -- src/agent/decision.rs`
逐行确认：user `format!` 的标签行数 = `{}` 占位数 = 实参数（删槽5、槽7 后各减 1，其余槽的相对顺序不变）；`runtime_text` / `memory_card_text` 两个 binding 已删；`knowledge_route_text` 改为调用新函数；新函数 `format_knowledge_route_for_prompt` 存在且被引用。

## 验证与上线策略（批次1 = 单测即可全量，无 A/B）

批次1 是零信息损失/零行为改变的纯清理（死字段 LM 本就不该输出、冗余是同一份数据的二次注入、调试元数据回复不消费），故**不需要 A/B 灰度**——单测绿 + 基线门绿即可全量上线。区别于批次 2/3（改动触及 LLM 实际可见的规则表述/上下文颗粒度，必须 A/B 数据验证）。

- 本批**不物理覆盖** prompt_templates 里的 active 版本吗？—— reply.task 模板改的是**代码内 `prompt_specs()` 的种子文本**（prompts.rs），生产库的模板由 `ensure_prompt_pack_v2` 按内容 diff 决定是否重种（见 memory `project_prompt_pack_version_not_effect_gate`：生效闸是内容 diff，不是版本号）。部署后重启，启动对齐会把新模板文本重种进库。槽5/7/11 的改动是**纯后端代码**（decision.rs 拼装逻辑），随二进制部署即生效。
- **部署需用户显式许可**（红线）。部署方式：paramiko `_push_bundle_direct.py` 推 + `_remote_run_direct.py` 重启 117（app port 3003）。前端无改动，无需 rebuild。
- **上线后观测**（非本 plan 阻塞项，供部署后核对）：查 `agent_run_logs` / `llm_call_logs` 里 `user.reply.task` 的 `prompt_tokens` 应较改前下降（槽5/7/11 三处 + 4 死字段声明省下的量）；`blocked_by_budget` / `no_reply` / review 跳过率不上升（预算已由方案A调至 300000，本就不该再触发，此处只作回归确认）。

---

## Self-Review（writing-plans 规范：写完后对着 spec 逐条自查）

**1. Spec 覆盖** —— spec「批次 1」现有 4 项，逐一对应：
- 删 4 死字段 → **Task 1** ✅
- 去 context_pack 重复注入（删槽7）→ **Task 2**（含槽5、槽7）✅
- 删知识路由调试元数据（槽11 只删 3 个纯调试字段）→ **Task 3** ✅
- 删硬运行参数（删槽5）→ **Task 2** ✅
- spec「批次1不动」（Soul 红线冗余 / 有消费字段）→ 计划中显式保留、Task 4 Step 5 专门核对存活字段不误删 ✅
- spec「删纯跨层重复」已在 spec 里亲验剔除 → 本 plan 不含，正确 ✅

**2. Placeholder 扫描** —— 无 TBD/TODO/"类似 Task N"/"添加适当错误处理"。所有代码步骤都有完整代码块；所有命令都有 Expected 输出。✅

**3. 类型一致性** —— `format_knowledge_route_for_prompt(route: &KnowledgeRouteResult) -> String` 在 Task 3 定义与调用（Step 3 定义、Step 4 调用、测试 Step 1 引用）三处签名一致；`KnowledgeRouteResult` / `SelectedChunkRanking` 字段名与 types.rs:1338-1397 亲验一致（`tool_trace`/`evidence_excerpts`/`selected_chunk_rankings` 的 snake→camel 派生 `toolTrace`/`evidenceExcerpts`/`selectedChunkRankings` 已亲验 `rename_all="camelCase"`）；`runtime` 参数不删签名（:998/:1000 仍消费）已核。✅

---

## Scope Check（writing-plans 规范：多子系统应拆多 plan）

本 spec 三个批次是**三个独立可测的交付单元**，已按规范拆分。**本轮只交付批次 1**（用户 2026-07-13 定：先做批次1，落地看生产效果再议后续）：
- **批次 1（本 plan）**：零风险清理，单测全量，无 A/B。**本轮唯一交付项。**
- **批次 2 / 批次 3**：暂不写 plan。批次1 部署 117 后，先看 `agent_run_logs.prompt_tokens` 实测降幅与回复体感，**由用户决定是否继续**。批次1 诚实局限（见 spec）：省的 token 是小头，真正把 40k 打回合理区间靠批次3 的动态大块——但那是风险最高处、需 A/B 灰度，故不预先承诺、看数据再议。

每个 plan 各自产出可编译、可单测、可独立上线的软件。若后续启动批次 2/3，批次间不并行改 prompt（A/B 归因要求单变量）。

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-13-reply-prompt-slimming-batch1.md`. 两种执行方式：

1. **Subagent-Driven（推荐）** —— 每个 Task 派一个全新 subagent 实现，Task 间我做两阶段 review、快速迭代。
2. **Inline Execution** —— 在本会话内用 executing-plans 批量执行、带检查点 review。

选哪种？
