# chat 知识检索 scorer 迁移到 relevance_score 设计

> 2026-06-23 全仓审查 #8「knowledge 两套 scorer」修复设计。
> 关联：[[project_codebase_audit_2026_06_23]]、[[project_agent_first_no_keyword_filters]]、[[feedback_no_overfitting]]。

## 问题（实证）

chat 知识检索（`exec_search` 的内存切片路径 + DB 路径）用 `score_chunk_for_query`
（`src/agent/knowledge_tools.rs:358`）打分：

```rust
fn score_chunk_for_query(chunk: &OperationKnowledgeChunk, query: &str) -> f64 {
    let q = query.to_lowercase();
    let mut score = 0.0;
    if chunk.title.to_lowercase().contains(&q) { score += 3.0; }
    if summary.to_lowercase().contains(&q) { score += 2.0; }
    if body.to_lowercase().contains(&q) { score += 1.0; }
    if score > 0.0 && integrity == "verified" { score += 0.5; }
    score
}
```

**缺陷**：纯 `contains` 是**整串匹配**。中文查询稍长（如「客户怎么退款」），只要 chunk 文本不
逐字包含整个查询串就命中 0 分——中文检索召回极差。

对照：生产 router 路径已有 `pub fn relevance_score`（`knowledge_agent.rs:1816`），用
`text_signals` 做 **CJK bigram + ASCII token 分词**，返回 `命中信号数 / 查询信号数`（0~1），
召回好得多。这是同一件事（chunk 相关度打分）的两套实现，chat 半边停留在朴素版。

## 方案 A：字段加权骨架不变，匹配内核换成 relevance_score

保留 chat `score_chunk_for_query` 的**字段加权结构**（title×3 / summary×2 / body×1）和
**verified 加分**（+0.5），只把每个字段的 `contains` 布尔判断换成 `relevance_score` 的连续命
中度：

```rust
fn score_chunk_for_query(chunk: &OperationKnowledgeChunk, query: &str) -> f64 {
    let mut score = 0.0;
    score += relevance_score(query, &chunk.title) * 3.0;
    if let Some(summary) = chunk.summary.as_ref() {
        score += relevance_score(query, summary) * 2.0;
    }
    if let Some(body) = chunk.body.as_ref() {
        score += relevance_score(query, body) * 1.0;
    }
    if score > 0.0
        && chunk.integrity_status.as_deref() == Some("verified")
    {
        score += 0.5;
    }
    score
}
```

### 为什么是方案 A（而非纯换 / 复用 rank_key）

- **纯换 `relevance_score(query, chunk_haystack)`**（方案 B）：会丢掉字段加权（title 命中和
  body 命中同分）和 verified 加分（不再优先）。是排序信号退化。
- **复用 router 的 `rank_key`**（方案 C）：rank_key 要 `now: DateTime` + 面向 DB chunk 的
  superseded/expired 信号，而 chat 的内存切片已预过滤 active+verified，这些信号冗余；耦合过
  重，YAGNI。
- **方案 A**：精确解决"中文召回差"的真缺陷（分词），同时不丢 chat 已有的字段加权 / verified
  语义。排序逻辑最小惊讶——权重 3/2/1 顺序不变，只是匹配从"整串包含"升级为"分词命中度"。

### 不过拟合 / agent-first

- 权重 3/2/1 + verified 0.5 是**沿用 chat 现有值**，不新调参、不对单条查询调优
  （[[feedback_no_overfitting]]）。
- relevance_score 是检索召回的**客观度量**（分词命中率），不是关键词快路径——决策仍由
  Reply Agent 做。换分词算法改进的是召回信号，符合 [[project_agent_first_no_keyword_filters]]。

## 复用方式

- `relevance_score` 已是 `pub`（knowledge_agent.rs:1816）。knowledge_tools.rs 加
  `use crate::agent::knowledge_agent::relevance_score;`。
- `text_signals` / `push_cjk` / `is_cjk` 是 relevance_score 的私有依赖，**不动、不暴露**——
  它们随 relevance_score 封装在 knowledge_agent 内，跨模块只调 pub 的 relevance_score。
- 删除 chat 旧 scorer 的 `query.to_lowercase()` 整串匹配内核（relevance_score 内部自己处理大
  小写归一，见 text_signals 的 `to_ascii_lowercase`）。

## 调用点不变

两个调用点逻辑完全不动，只是 score 算法变了：
- `exec_search` 内存切片路径（knowledge_tools.rs:310）：`score>0 过滤 → 降序排序 →
  take(top_k) → build_search_hit`。
- `exec_search` DB 路径（:789）：同上结构。

`build_search_hit(chunk, score)`（:334）把 score 原样放进 JSON tool result 的 `"score"` 字段。
**已实证**该 score 下游只给 LLM 看（tool result），无整数 / 固定区间硬假设（grep 确认无消费方
对它做精确断言）。新 score 是连续值（如 `0.67×3=2.0`）而非旧的离散整数，对 LLM 排序消费无影响。

## 测试影响

1. **新增字段加权 scorer 单测**（增量，钉住新行为的关键不变量）：
   - **中文召回改善（核心价值）**：构造中文 query + 仅中文 body 命中的 chunk，旧整串 contains
     得 0、新分词得 >0。证明缺陷已修。
   - **字段加权保留**：title 命中的 chunk 分数 > 仅 body 命中的（权重 3>1 仍成立）。
   - **verified 加分保留**：其它条件相同时 verified chunk 分更高。
2. **现有 chat search 测试不回归**：`search_verified_returns_snippet_with_redacted_false`
   （:1389，query="beta" ASCII 命中 verified chunk）在方案 A 下仍应过（ASCII token 命中 +
   verified 加分）。迁移后必须确认绿。
3. **relevance_score 本身不动**：router 路径零影响，只多一个 import 它的消费者。

## 约束

- `cargo check --tests` 0 error / 0 warning；`cargo test --lib` ≫350 / 0；四 PBT ≥33 / 0。
- 磁盘受限：本地 `cargo check` + 点跑 knowledge_tools 单测（纯函数小 footprint，可跑）。
- 纯质量改进：不改 chat 检索调用结构、不动 router、不动 relevance_score / text_signals。
- 提交需用户显式批准；精确 `git add`（只 knowledge_tools.rs）。

## 风险与回滚

- 风险：新 score 连续值改变了排序的**绝对分值**（但相对顺序语义保留）。缓解：score 下游只
  给 LLM、无硬假设（已实证）；新增单测钉住字段加权 + verified 顺序不变。
- 风险：跨模块 import 引入 knowledge_tools → knowledge_agent 依赖。已实证 knowledge_tools
  已 `use crate::agent::...` 多处，同模块树内依赖，无循环（relevance_score 不反向依赖
  knowledge_tools）。
- 回滚：单函数内核替换，`git revert` 即恢复。

## 实现状态

✅ **已实现**（2026-06-24）

- commit: `853ca82` — 新增三个单测（中文召回 / 字段加权 / verified 加分）
- commit: `9903c23` — scorer 内核迁移到 relevance_score
- 验证：`cargo test --lib` 1498/0，四 PBT 36/0，现有测试不回归
