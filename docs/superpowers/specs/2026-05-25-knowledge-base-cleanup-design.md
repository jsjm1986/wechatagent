# Spec: 旧销售话术 RAG 全面清理 → Wiki 方法论独占

**Date:** 2026-05-25
**Status:** Approved (route A — 9-commit big bang)
**Owner:** WechatAgent backend + frontend

## 1 · 背景与决策

### 1.1 起点

新 wiki 子系统（9 类 `wiki_type` + `chunk_revisions` + `knowledge_gap_signals` + `domain_schemas` + `catalog_rebuild_jobs` + `apply_chunk_revision` 三层保护 + structural lint + 两阶段 sweep）已落地（Phase A-G 完成，510/0 lib 测试 + 37/0 PBT）。但旧"销售话术增强 RAG"形态仍并存：

- `OperationKnowledgeItem` 35 销售域硬编码字段（item-pack 中间层）
- `OperationKnowledgeChunk` 上销售域专属字段（`routing_card / safe_claims / forbidden_claims / evidence_items / distortion_risks / unsupported_claims / verified_claims`）
- `Contact.customer_stage / customer_stage_updated_at / intent_level`
- `SystemTaxonomy` 三套销售 seed（`customer_stage` 9 项、`intent_level` 3 项、`objection_type` 7 项）
- `AppConfig` 三阈值（`fact_risk_block_at / pressure_risk_block_at / product_accuracy_block_below`）
- `prompts.rs` ~700 LOC 销售域硬编码（domain_configs / Soul v3 / Reply Agent prompt / chunk 切片 prompt / playbook & evolution prompt）
- `routes/knowledge.rs` ~3500 LOC 旧 item CRUD / import-apply / catalog 实时聚合 / repair / chat 销售域路径
- `frontend/src/App.tsx` ~3500 LOC 旧 `OperationKnowledgeView` 与销售域硬编码文案
- 测试 ~1500 LOC 销售域 fixture（`evolution_*` 整组 + `planner_*` 整组 + `string_fact_risk_guard` PBT + `autonomy_protocol_pbt` 部分等）

### 1.2 决策

用户授权（开发期数据无需兼容）：彻底清理旧形态，全面切到 wiki 方法论。

**路线 A · 9-commit 大爆炸：** 每个 commit 内部 `cargo check` 绿；R11.6 baseline 在 commit 7 重定义；不引入 feature flag、不留过渡形态。

### 1.3 不变约束

- 隔离红线：knowledge 子系统不引用 `crate::agent::gateway/outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`
- 所有 LLM 调用走 `agent::generate_agent_json`
- AI 写入永不自动 verify（`status="draft" + integrity_status="needs_review"`）
- 0 新依赖（Rust 不增 crate；前端仍 React 19 + lucide-react + 单一 styles.css）
- 不在 prompt / schema / UI / docs 露任何模型名 / 品牌词
- AI-自主用语贯穿（`scripts/check-no-human-takeover.sh` 0 命中）
- 中文回复、bash on Windows、`CARGO_TARGET_DIR=target-pbt`

## 2 · 整体架构（清理后）

```
React Admin
  └─ KnowledgeWiki channel（唯一）
       ├─ DomainSchemaTab
       ├─ GapSignalsTab
       └─ ChunkRevisionsDrawer

Rust Axum
  ├─ routes/knowledge.rs（瘦身后约 1500 LOC）
  │   ├─ /catalog/persisted（O(1) 读 documents.catalog_summary_persisted）
  │   ├─ /chunks/:id/{patch,split,merge,archive,restore,rollback,revisions,relate}
  │   ├─ /chunks/:id/verify
  │   ├─ /import/preview + /import/apply（chunked 块流）
  │   ├─ /knowledge/gap-signals + /sweep + /dismiss + /apply
  │   └─ /admin/domain-schemas/*
  │
  ├─ knowledge_wiki/（独立子系统，不引用销售域）
  │   ├─ page_merge.rs        三层保护纯函数 + PBT
  │   ├─ chunk_revisions.rs   apply_chunk_revision 状态机
  │   ├─ gap_signals.rs       structural + semantic lint + 两阶段 sweep
  │   ├─ catalog_rebuild.rs   消费 catalog_rebuild_jobs 队列
  │   └─ feedback_worker.rs   主循环：usage → dynamic_confidence → lint → sweep
  │
  ├─ agent/
  │   ├─ guards.rs            仅留 3 闸：knowledge_grounding / hallucination / run_budget
  │   ├─ review.rs            ReviewScores 收敛 4 项
  │   ├─ decision.rs          LLM 输出 schema 去销售域
  │   └─ knowledge_router.rs  排序键改 dynamic_confidence × wiki_type_priority + 邻接 boost
  │
  └─ models.rs                Item struct 删除；Chunk 字段瘦身；Contact.domain_attributes 接管
```

## 3 · 9 类 wiki_type（保持稳定）

```rust
pub const WIKI_TYPES: &[&str] = &[
    "source",       // 原文（文档 / 会议纪要 / 口径文件）
    "entity",       // 实体（产品 / 角色 / 团队 / 地点 / 人物）
    "concept",      // 概念（术语 / 定义 / 规则 / 政策 / SOP）
    "comparison",   // 对比（vs 对手 / 方案 A vs B）
    "synthesis",    // 综合（跨多 source 总结）
    "methodology",  // 方法（话术框架 / 决策树 / 检查清单）
    "finding",      // 发现（数据点 / 案例事实 / 量化结论）
    "query",        // 查询（FAQ）
    "thesis",       // 论点（带立场判断）
];
```

业务字段下沉到 `domain_attributes: bson::Document`，由 `DomainSchema` 校验。

## 4 · Phase 拆解

### Phase A · 数据 & 模型层切除（commit 1-2）

#### A.1 cleanup migration（一次性 drop 旧数据）

`src/db/migrations.rs` 新增：

```rust
async fn m_2026_05_25_drop_legacy_sales_collections(db: &Database) -> Result<()> {
    // 开发数据，直接 drop 集合
    db.collection::<Document>("operation_knowledge_items").drop(None).await.ok();
    // documents / chunks 集合保留（结构沿用 wiki 形态），仅 drop 数据
    db.collection::<Document>("operation_knowledge_documents").delete_many(doc!{}, None).await?;
    db.collection::<Document>("operation_knowledge_chunks").delete_many(doc!{}, None).await?;
    Ok(())
}

async fn m_2026_05_25_drop_legacy_taxonomy_seed(db: &Database) -> Result<()> {
    db.collection::<Document>("system_taxonomies").delete_many(
        doc!{"kind": {"$in": ["customer_stage", "intent_level", "objection_type"]}},
        None,
    ).await?;
    Ok(())
}
```

并删除已落地但本轮已无意义的迁移：
- `m_2026_05_25_chunks_wiki_type_default`（drop 后无 chunk 需要 backfill）
- `m_2026_05_009_contact_customer_stage_updated_at_backfill`

#### A.2 删 `OperationKnowledgeItem` 整 struct

- `src/models.rs:378-437` 整段删除
- `src/db/mod.rs:116-125` 删 `operation_knowledge_items()` typed accessor
- `src/db/indexes.rs:156-196` 删旧 items 索引（保留 documents / chunks 索引但内容只剩 wiki 字段）

#### A.3 `OperationKnowledgeChunk` 字段瘦身

`src/models.rs:486-572` 删字段：

```text
- routing_card: Vec<String>
- safe_claims: Vec<String>
- forbidden_claims: Vec<String>
- evidence_items: Vec<EvidenceItem>
- distortion_risks: Vec<String>
- unsupported_claims: Vec<String>
- verified_claims: Vec<String>
- applicable_scenes（如仅销售用法，迁 domain_attributes）
```

保留：

```text
chunk_id / document_id / workspace_id / wiki_type / title / summary
+ body / sources / source_anchor / source_quote
+ tags / search_terms / related_chunks
+ status / integrity_status / verified_at / verified_by / approved_at
+ provenance / valid_from / valid_to / superseded_by / previous_version_id
+ usage_stats / dynamic_confidence / integrity_score / locked_fields
+ domain_attributes: bson::Document（业务字段容器）
```

#### A.4 Contact 销售域字段下沉

`src/models.rs:97-104` 删 `customer_stage / customer_stage_updated_at / intent_level`：

```rust
// 改为：
pub struct Contact {
    // ...
    pub domain_attributes: Option<bson::Document>,
    pub domain_attributes_updated_at: Option<DateTime>,
    // ...
}
```

`src/models.rs:1351-1391` 出参映射同步删除 customer_stage 相关字段。

#### A.5 SystemTaxonomy 销售 seed 清空

`src/db/migrations.rs:474-708, 1159-1280` 整段删除（不再 seed `customer_stage / intent_level / objection_type`）。Collection 结构保留，由用户在 admin 通过 DomainSchema + 自定义 taxonomy 路径自配。

#### A.6 AppConfig 三阈值清理

`src/models.rs:1448-1517` 删：

```text
- fact_risk_block_at
- pressure_risk_block_at
- product_accuracy_block_below
```

新增（可空，用户可调）：

```rust
pub struct Thresholds {
    pub hallucination_block_at: u8,        // 默认 6
    pub knowledge_grounding_block_below: u8, // 默认 7
    pub human_like_score_rewrite_below: u8,  // 默认 6（保留）
    pub emotional_value_rewrite_below: u8,    // 默认 5（保留）
}
```

### Phase B · Guards / Review 全面重写（commit 3）

#### B.1 删除整段函数

- `src/agent/guards.rs:114-141`（`enforce_decision_guards` 销售部分）
- `src/agent/guards.rs:493-521`（`enforce_string_fact_risk_guard` 整段）
- `src/agent/guards.rs:1108, 1653` 销售域测试 fixture
- `src/agent/review.rs:1132-1738` review 中销售域测试 fixture（保留通用 review 测试）

#### B.2 ReviewScores 收敛

`src/agent/review.rs:111-205` 重写：

```rust
pub struct ReviewScores {
    pub hallucination_score: u8,         // 0-10，越高越严重
    pub knowledge_grounding_score: u8,   // 0-10，越高越好
    pub human_like_score: u8,            // 0-10，越高越好
    pub emotional_value: u8,             // 0-10，越高越好
}
```

去掉 `product_accuracy / pressure_risk / fact_risk`。

#### B.3 三闸保留

```rust
// guards.rs 新版仅这三闸：
pub fn enforce_decision_guards(...) -> Result<(), GuardBlock> {
    enforce_knowledge_grounding(decision, knowledge)?;  // 任何产品/事实声明必须 cite + source_quote 非空
    enforce_hallucination(decision, scores, thresholds)?;  // hallucination_score >= block_at → block
    enforce_run_budget(budget)?;                         // RunBudget 超额
    Ok(())
}
```

`enforce_knowledge_grounding` 实现要点：

- decision.cited_chunk_ids 中每条必须存在于 `operation_knowledge_chunks` 且 `status != archived`
- decision 文本中如果出现"产品名 / 价格 / 数据 / 政策"类断言（基于轻量分类器或固定关键词集，可由 DomainSchema guard_dsl 增强），必须有至少一条 cited chunk 命中且其 `verified=true && source_quote.is_some()`
- 否则 `GuardBlock::UnverifiedClaim`（`agent_send_outbox` 状态映射为 `held_by_ai_policy`）

### Phase C · Decision / Knowledge Router 重构（commit 4）

#### C.1 Decision LLM 输出 schema

`src/agent/decision.rs:116-119, 384-385` 重写：

```jsonc
// 旧
{ "customerStage": "...", "intentLevel": "...", "objectionType": "...",
  "pressureRisk": 0, "factRisk": 0, "productAccuracyScore": 0, "replyText": "..." }

// 新
{
  "operationState": "...",          // 来自 operation_domain_configs 的状态字典
  "citedChunkIds": ["c_1", "c_2"],  // 知识引用
  "hallucinationScore": 0,          // 0-10
  "humanLikeScore": 0,
  "emotionalValue": 0,
  "knowledgeGroundingScore": 0,
  "domainSignals": { /* 任意 JSON，由 DomainSchema 校验 */ },
  "replyText": "..."
}
```

`src/agent/decision.rs` 中 `customerStage / intentLevel` 解析路径删除，改读 `domainSignals` 写入 `Contact.domain_attributes`。

#### C.2 Knowledge Router 排序键

`src/agent/knowledge_router.rs:233, 252, 285-310, 545, 800-821`：

```rust
// 旧排序：routing_card 命中权重 + integrity_score
// 新排序：
fn rank_score(c: &Chunk, query_signals: &QuerySignals) -> f64 {
    let base = c.dynamic_confidence.unwrap_or(0.5);
    let type_priority = match c.wiki_type.as_deref() {
        Some("methodology" | "concept") => 1.0,
        Some("query" | "finding") => 0.9,
        Some("comparison" | "synthesis") => 0.8,
        Some("entity" | "thesis" | "source") => 0.7,
        _ => 0.5,
    };
    let adjacency_boost = related_boost(c, query_signals);  // 与已选 chunk 在 related_chunks 邻接 → +0.1
    base * 0.6 + type_priority * 0.3 + adjacency_boost * 0.1
}
```

tool-loop 三跳形态（catalog → list_chunks → open_slice）保留。`record_chunk_hit` fire-and-forget hook 保留。

#### C.3 Catalog 实时聚合整段删除

- `src/routes/knowledge.rs:2684-2876` 删除（`build_operation_knowledge_catalog` 实时聚合）
- `src/routes/knowledge.rs:5177-5394` 删除（chat catalog 渲染读销售域字段）
- 仅保留 `/api/operation-knowledge/catalog/persisted`（O(1) 读 `documents.catalog_summary_persisted`）+ `catalog_rebuild_worker` 后台消费 jobs 队列

### Phase D · Prompts 中性化（commit 5）

#### D.1 default_domain_configs 重写

`src/prompts.rs:339-541`：

```text
旧 forbidden_rules：「不得主动销售」「不得承诺折扣」「客户阶段必须按 9 阶段输出」
新 forbidden_rules：「不得编造产品功能」「不得承诺平台未授权事项」「不得越出引用知识范围」「不得在缺少出处时给出关键事实声明」
旧 method_prompt：「顾问式销售 / 长期关系运营 / 异议处理 / 转化漏斗」
新 method_prompt：「以引用为锚 / 状态机驱动 / 不越知识边界 / 自检后再发送」
```

#### D.2 Soul v4

`src/prompts.rs:604-642` 删 5 阶段销售客户分类，改：

```text
你是运营对象的长期协作 AI。你的职责：理解对话上下文 → 引用 wiki 知识 → 生成符合人物 Soul 的回复。
你不预设对象的"阶段"或"意图类型"——这些由用户在 DomainSchema 中定义，作为 domain_signals 的可选属性输出。
```

#### D.3 Reply Agent prompt

`src/prompts.rs:691-893` 输出 schema 与 §C.1 对齐；删除 `customerStage` 必填字段；新增 `citedChunkIds` 必填。

#### D.4 Chunk 切片 prompt

`src/prompts.rs:1229-1412` 字段名改：

```text
旧：routing_card / safe_claims / forbidden_claims / common_questions / customerStages / intentLevels
新：wiki_type / title / summary / body / sources / source_quote / tags / related_chunks
```

#### D.5 Playbook & evolution prompt

`src/prompts.rs:1628-1786` 阈值名词改：

```text
旧：fact_risk / pressure_risk / unverified_product_claim / product_accuracy_score_block / fact_risk_block / human_like_score_rewrite / 顾问式销售 / 长期关系运营
新：hallucination_score / knowledge_grounding_score / blocked_unverified_claim / hallucination_block / knowledge_grounding_block / human_like_score_rewrite（保留） / 以引用为锚的运营
```

### Phase E · Routes + knowledge_wiki/ 内部洗白（commit 6）

#### E.1 routes/knowledge.rs 整段删除

| 行号区间 | 内容 |
|---|---|
| L65-125, L163-171, L373-407 | 请求 payload 销售域字段 |
| L1227-1267 | item-level CRUD（create/update/delete_operation_knowledge） |
| L1286-1666 | import-apply 旧版（item-pack 整批导入） + extract_tags |
| L1819-1894, L1984-2166 | OperationKnowledgeItem JSON 渲染 |
| L2208-2306 | JSON snake/camel 双向 fallback |
| L2684-2876 | catalog 实时聚合 |
| L3327-3641 | chunk repair / pack repair |
| L5177-5394 | chat catalog 渲染 |
| L5583-5731 | patch 适配器销售域字段白名单 |

保留并保持 wiki 方法论：

| 行号区间 | 内容 |
|---|---|
| L6479-7203 | 新 wiki 风格 chunks 子路径（patch/archive/restore/rollback/revisions/split/merge/relate） |
| 新 import-apply chunked 块流 | `---CHUNK: id---...---END CHUNK---` fence-aware parser |
| /catalog/persisted | O(1) 读 |
| /knowledge/gap-signals + /sweep + /dismiss + /apply | gap-signals 路由 |
| /admin/domain-schemas/* | DomainSchema admin |

#### E.2 routes/mod.rs 路由注册瘦身

`src/routes/mod.rs:92-124, 236-434` 删 30+ 条 `/operation-knowledge/*` 旧注册，仅保留 wiki 子路由 + chunked import + catalog/persisted + 7 编辑路由 + gap-signals + domain-schemas。

#### E.3 knowledge_wiki/ 内部洗白

- `src/knowledge_wiki/catalog_rebuild.rs:179-354` 删除 routing_card 相关渲染，仅渲染 `wiki_type / title / summary / source_quote / tags`
- `src/knowledge_wiki/gap_signals.rs:614-618` 测试 fixture 改为 wiki 字段
- `src/knowledge_wiki/page_merge.rs:60` `forbidden_claims` 从合并白名单删除；新白名单：`tags / related_chunks / search_terms / sources`

### Phase F · Tests 重写 + Baseline 调整（commit 7）

#### F.1 删除整文件

```text
tests/evolution_rollback.rs
tests/evolution_threshold_e2e.rs
tests/evolution_significance_pbt.rs
tests/evolution_isolation.rs
tests/evolution_prompt_e2e.rs
tests/planner_priority.rs
tests/planner_stage_stagnation.rs
tests/string_fact_risk_guard.rs        ← R11.6 baseline 替换为 wiki_chunk_revision_pbt
```

#### F.2 部分改写

| 文件 | 动作 |
|---|---|
| `tests/autonomy_protocol_pbt.rs:275-619, 698` | customer_stage / fact_risk PBT 段改为运营中性 fixture（state_transition + cited_chunk_ids） |
| `tests/keyword_fastpath_router.rs:34-38` | customer_stage fixture → domain_attributes |
| `tests/last_inbound_split.rs:32` | 同上 |
| `tests/happy_path_run.rs:43` | 同上 |
| `tests/outbox_integration.rs:48` | 同上 |
| `tests/outcomes_autonomy_endpoint.rs:290, 303, 317` | `taxonomy_candidate:objection_type / fact_risk:product_unverified / pressure_risk:hard_close` 改为 `taxonomy_candidate:domain_signal / hallucination:unverified_claim` |

#### F.3 新增 PBT

`tests/wiki_chunk_revision_pbt.rs`（≥ 8 properties）：

1. **锁定字段不变量** — 任何 patch 携带 `chunk_id / wiki_type / created_at / source_anchor / verified_at / verified_by / approved_at` 中任一字段必被拒
2. **数组字段 union 单调** — 任意 existing.tags + patch.tags merge 后长度 ≥ max(existing, patch) 且包含两者全集
3. **70% body 阈值** — 当 patch body 长度 < existing × 0.7 时必拒
4. **Hash 不变** — apply 失败时 chunk hash 不变
5. **AI 写入 status 强制** — source=ai 时 status=draft, integrity_status=needs_review
6. **revision id 唯一性** — 任意并发 apply 后 revision_ids 全部唯一
7. **Rollback 幂等** — rollback 到同一 revision_id 两次结果一致
8. **Cleanup dangling refs 不误伤** — `normalize_ref_key("openai")` ≠ `normalize_ref_key("ai")`，archived chunk 的 substring-similar id 不被错误清理

#### F.4 Baseline 调整

`scripts/check-baseline.sh:8, 59` 与 `scripts/check-baseline.ps1:8, 63`：

```diff
- 4 PBT cumulative: state_transition_pbt, memory_card_invariants, string_fact_risk_guard, llm_retry_jitter ≥ 33/0
+ 4 PBT cumulative: state_transition_pbt, memory_card_invariants, wiki_chunk_revision_pbt, llm_retry_jitter ≥ 33/0
```

`cargo test --lib` 新 baseline 估约 **350-400 通过 / 0 失败**，commit 7 末锁定具体数字。

#### F.5 新自检脚本

`scripts/check-no-sales-domain.sh`：

```sh
#!/usr/bin/env bash
# 扫描 src/ docs/ frontend/src/，禁词命中即 fail
PATTERN='customer_stage|customerStage|objection_type|objectionType|intent_level|intentLevel|forbidden_claims|forbiddenClaims|safe_claims|safeClaims|routing_card|routingCard|fact_risk|factRisk|pressure_risk|pressureRisk|product_accuracy|productAccuracy|sales[_-]positioning'
EXCLUDE='--glob=!docs/real-task-runbook.md --glob=!docs/superpowers/specs/* --glob=!.kiro/specs/*'
HITS=$(rg -i "$PATTERN" src/ docs/ frontend/src/ $EXCLUDE | wc -l)
[ "$HITS" -eq 0 ] || { echo "Found $HITS sales-domain residue:"; rg -i "$PATTERN" src/ docs/ frontend/src/ $EXCLUDE; exit 1; }
```

`scripts/check-baseline.sh` 末尾 + 调用一次 `scripts/check-no-sales-domain.sh`。

### Phase G · Docs + Specs（commit 8）

#### G.1 删整文件

- `docs/sales-positioning-knowledge.md`

#### G.2 重写

| 文件 | 行号 | 动作 |
|---|---|---|
| `docs/agent-policy.md` | L265-337 | 5 闸 → 3 闸：knowledge_grounding / hallucination / run_budget |
| `docs/agent-policy.md` | L453-495 | 已是 wiki 章节，保留 |
| `docs/data-and-api.md` | L82, L98 | 删 customer_stage 字段 + Chunk 销售域字段表，新增 domain_attributes 表 |
| `docs/data-and-api.md` | L493-540+ | 已是 wiki 子系统，保留 |
| `docs/architecture.md` | L260-275 | 已是 wiki 子系统图，保留 |
| `docs/knowledge-wiki.md` | 新增章节 | "Contact / Chunk 销售域字段下沉到 domain_attributes，由 DomainSchema 决定" |

#### G.3 加 sunset notice

`docs/real-task-runbook.md` 顶部加：

```markdown
> **Note:** 该文件记录的运行案例使用了已废弃的销售域方法论（`fact_risk` / `pressure_risk` / `customer_stage` 等），自 2026-05-25 起方法论已切换为 wiki + 3 闸。本文件仅作历史参考。
```

#### G.4 Specs 中性化

| 文件 | 段落 | 动作 |
|---|---|---|
| `.kiro/specs/agent-autonomy-loop/{requirements,design,tasks}.md` | R8.x、P4、P6 | 销售域章节改为运营中性 |
| `.kiro/specs/user-ops-agent-hardening/{design,requirements,tasks}.md:420-1219` | enforce_string_fact_risk_guard 段 | 标 deprecated，新增"已被 knowledge_grounding_guard 替换"链接 |
| `.kiro/specs/agent-self-evolution/tasks.md:222` | fact_risk 阈值引用 | 改 hallucination_score |
| `.kiro/specs/knowledge-digest-workstation/{design,tasks}.md:150-95` | 同上 | 改 hallucination_score |

### Phase H · Frontend 最小生存（commit 9）

UI 重做留下一轮，本轮仅做删除。

#### H.1 删除

| 文件 | 行号 | 动作 |
|---|---|---|
| `frontend/src/App.tsx` | 43, 872 | 删 `"knowledge"` channel 入口，仅保留 `"knowledgeWiki"` |
| `frontend/src/App.tsx` | 281-407 | 删 `OperationKnowledgeItem / Draft` 类型 |
| `frontend/src/App.tsx` | 962, 1047-1141, 1572-2099 | 删 `operationKnowledge` state + 所有旧 `/api/operation-knowledge*` 调用（保留 wiki 路径） |
| `frontend/src/App.tsx` | 2168, 2417, 2684-2698, 4149, 4225-5188 | 删 `OperationKnowledgeView / KnowledgeChannel` 视图与子组件 |
| `frontend/src/App.tsx` | 5485-5693, 7754-8172 | 删旧 chunk/pack 编辑 textarea |
| `frontend/src/App.tsx` | 8520, 9602-9629, 10125-10158 | 删销售域硬编码文案 |
| `frontend/src/styles.css` | 263-384, 3385-4340 | 删 `.knowledge*` 命名空间（~1100 LOC） |

#### H.2 保留

`frontend/src/App.tsx:8731, 2874-2882, 9107, 10664-10718` 中 KnowledgeWiki channel + `/inbox` + `chunks/:id/revisions` 调用保留不动。

#### H.3 KnowledgeWiki 视图复用 class

KnowledgeWiki 现有视图（DomainSchemaTab / GapSignalsTab / ChunkRevisionsDrawer）当前复用旧 `.knowledge*` class —— 删除这些 class 后视图会无样式。处理：把这三个 Tab 的样式提取到独立的 `.knowledgeWiki` 命名空间（最小可用样式约 200 LOC，先满足"能用"，UI 美化下一轮）。

## 5 · 索引与 Collection 状态（清理后）

| Collection | 状态 |
|---|---|
| `operation_knowledge_documents` | 保留结构，drop 数据 |
| `operation_knowledge_chunks` | 保留结构（瘦身字段后），drop 数据 |
| `operation_knowledge_items` | drop 集合 |
| `chunk_revisions` | 保留 |
| `knowledge_gap_signals` | 保留 |
| `domain_schemas` | 保留 |
| `catalog_rebuild_jobs` | 保留 |
| `system_taxonomies` | 保留结构，删 customer_stage / intent_level / objection_type seed |

索引（`src/db/indexes.rs`）：

- 删：L156-196 旧三层索引（`trigger_keywords` / `document_id+item_id+status` / `status+priority`）
- 保留：L766-924 新 wiki 索引区段全部不动

## 6 · 风险与回滚

### 6.1 失败模式

| 场景 | 处理 |
|---|---|
| Phase F 删 evolution_* 测试后某个保留测试失败 | 回到测试单独看，必要时把销售域 fixture 改成中性 fixture 而不是删除 |
| `cargo test --lib` 数量低于 350 | 评估是否过度删除；保留通用断言型测试，仅删销售域专属 |
| Phase H 删 styles.css 旧 class 后 KnowledgeWiki 视图崩样式 | H.3 的 .knowledgeWiki 命名空间提取必须与 H.1 同 commit；不可分两 commit |
| `cleanup migration` 在生产环境意外执行 | migration 加守卫：`if env!("APP_ENV") != "development" { panic!("禁止在非开发环境执行 cleanup migration") }` |
| `check-no-sales-domain.sh` 漏检 | 加 Camel 形式 `customerStage / objectionType` 等到禁词正则；新增到本 spec 的禁词列表 |

### 6.2 回滚

9 commits 倒序 `git revert` 即可。drop 集合和 drop seed 是单向操作（数据丢失），但开发数据无价值，回滚后重新 seed 测试数据即可。

## 7 · 验证门

| # | 命令 | 期望 |
|---|---|---|
| 1 | `cargo check` | 0 错 |
| 2 | `cargo test --lib` | ≥ 350/0（commit 7 末锁定具体数） |
| 3 | 4 PBT 累计 | ≥ 33/0（含 `wiki_chunk_revision_pbt` ≥ 8 properties） |
| 4 | `bash scripts/check-baseline.sh` | exit 0 |
| 5 | `bash scripts/check-no-human-takeover.sh` | 0 命中 |
| 6 | `bash scripts/check-no-model-hint.sh` | 0 命中 |
| 7 | `bash scripts/check-no-sales-domain.sh`（新） | 0 命中 |
| 8 | `cd frontend && npx tsc --noEmit` | 0 错 |
| 9 | `cd frontend && npm run build` | dist 干净 |
| 10 | `cargo run` → import 一份 8 chunk md → catalog/persisted 在 3s 内含 8 条 | catalog_version 自增；每 chunk 都有 chunk_revision (op=create, source=imported) |
| 11 | mongosh 巡检 | `operation_knowledge_items` 不存在；`system_taxonomies` 中无 customer_stage / intent_level / objection_type；`Contact.customer_stage` 字段已不存在；`OperationKnowledgeChunk.routing_card` 等字段已不存在 |

任一项 ✗ → 按 `docs/agent-policy.md` runbook §5 七步流程定位修复。

## 8 · 9 commit 顺序

| # | 名称 | 范围 |
|---|---|---|
| 1 | drop legacy collections + cleanup migration | A.1 + A.5（drop 数据 + drop taxonomy seed） |
| 2 | models / db / indexes 瘦身 | A.2 + A.3 + A.4 + A.6（删 Item struct + Chunk 字段 + Contact 下沉 + AppConfig 阈值清理） |
| 3 | guards / review 重写 | Phase B 全部 |
| 4 | decision / knowledge_router 重构 | Phase C 全部 |
| 5 | prompts 中性化 | Phase D 全部 |
| 6 | routes 清理 + knowledge_wiki/ 洗白 | Phase E 全部 |
| 7 | tests 重写 + baseline 调整 + check-no-sales-domain.sh | Phase F 全部 |
| 8 | docs + specs 中性化 | Phase G 全部 |
| 9 | frontend 旧 UI 删除 + KnowledgeWiki 样式独立 | Phase H 全部 |

每 commit 内 `cargo check` 绿。Commit 3-6 可能短暂破坏部分测试（在 commit 7 修复）；用 `cargo check` 而非 `cargo test --lib` 作为中间 gate。

## 9 · 明确不在本轮范围

- Frontend KnowledgeWiki UI 全面美化（仅做"能用"样式，下一轮设计）
- embedding / RRF / community detection / hybrid search 召回升级
- DomainSchema guard_dsl 表达力扩展（仍仅 `field OP value`）
- multimodal（图片 / PDF / 表格抽取）
- chunk_revisions git 风格 branch / merge tree
- 跨 workspace 知识共享 / 联邦
- chunk 自动 redirect 解析（`superseded_by` 链跳转）

## 10 · Out-of-Scope 但需追踪

清理后这些点的语义会改变，下一轮需关注：

- `taxonomy_candidates` 仍存在但 `kind` 字段不再有销售域取值 → 改为运营人自定义 kind
- Real-task-runbook 历史案例失去现行参考价值 → 是否在合适时机用新方法论重跑标注几条经典 case
- `operation_playbooks` 中销售域内容当前 prompts.rs 不再 seed → 需 admin 重新填充新方法论 playbook
- evolution worker 中 fact_risk_block / pressure_risk_block 评分维度删除后，significance 信号改为 hallucination_score / knowledge_grounding_score 维度
