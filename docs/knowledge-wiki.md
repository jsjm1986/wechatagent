# 运营知识库 wiki-style 方法论（knowledge-wiki）

本文是知识库子系统的"方法论 + 决策表 + lifecycle"参考。日常运维 / 写新 chunk / 设计行业 schema 时先翻这里。设计动机与当时的 LLW 借鉴清单见 [`docs/agent-policy.md` §运营知识库 wiki-style 方法论](agent-policy.md#运营知识库-wiki-style-方法论knowledge-wiki)；后端字段 / 路由 / 索引 / 集合定义见 [`docs/data-and-api.md` §knowledge-wiki](data-and-api.md#knowledge-wiki-子系统phase-a-g)。

## 1. 为什么不是销售话术 RAG

旧形态把 chunk 主表硬编码成销售域字段（`customer_stage / objection_type / pressure_level ...`）；换行业 = 改 schema + 改 prompt = 改产品。新形态按"知识形态"分层：跨行业稳定的 9 类 wiki_type 是 chunk 的种类，业务可变字段下沉到 `chunks.domain_attributes` JSON 子文档，行业差异由 `domain_schemas` 配。**主表稳定，业务可变字段在子文档里**。

不暗示模型品牌：CI 由 [`scripts/check-no-model-hint.sh`](../scripts/check-no-model-hint.sh) 在 PR diff 上 lint 出常见品牌字面量（具体清单见脚本里的 `FORBIDDEN_PATTERN`），命中即拒。LLM provider 由运营在 `LlmProviderConfigs` 自填，`ChunkProvenance.llm_model_alias` 字段只写 `provider_id`（`"default"` / `"reviewer"`），不写品牌名。

## 2. 借鉴 nashsu/llm_wiki（LLW）对照表

| 借鉴点 | LLW 出处 | WechatAgent 落地 |
| --- | --- | --- |
| 9 类 wiki page type | `wiki-page-types.ts:1-23` | `OperationKnowledgeChunk.wiki_type` 9 枚举 |
| frontmatter 锁定字段 | `page-merge.ts:43-44, 162-176` | `apply_chunk_revision` 拒收带锁定字段的 patch |
| 数组字段 union 合并 | `page-merge.ts:30-31, 175` | `tags / related_chunks / sources / search_terms / applicable_scenes` 应用层 union |
| 70% body 长度阈值反 truncate | `page-merge.ts:53, 148-158` | patch 改 `answer / explanation` 时 `is_body_truncated` 拒收 |
| chunked structured output | `ingest.ts:65-274` | `import_operation_knowledge_apply` `---CHUNK: id---...---END CHUNK---` 流式块 |
| enrich-wikilinks 只返 patch | `enrich-wikilinks.ts:7-22` | LLM 永远只返字段级 patch JSON，后端调 `apply_chunk_revision` |
| 结构 lint + 语义 lint 两层 | `lint.ts:69-156, 164-299` | `gap_signals.rs::run_structural_lint`（必跑）+ stage 2 LLM lint（接口预留） |
| 两阶段 sweep stale reviews | `sweep-reviews.ts:340-460` | `sweep_stale_signals` stage 1 规则消解；stage 2 LLM 留接口 |
| 删除级联（normalize key） | `wiki-cleanup.ts:49-130` | `archive` 后清 `related_chunks` 中指向已归档 chunk 的引用 |
| structural lint: orphan / broken-link | `lint.ts:69-156` | `compute_structural_candidates` 5 规则 kind |

LLW 的 community detection (Louvain) / `graph-relevance` / `embedding` / `search-rrf` 不在本轮范围（用户重置过：召回算法零改动）。先把 schema / 写入 / 版本 / 反馈 / lint 五件事做扎实，召回升级留后续。

## 3. 9 类 wiki_type 决策表

**先问一个 chunk 在描述什么**，再选类型：

| 这个 chunk 在说什么 | wiki_type | 销售域举例 | 教培域举例 | 医疗域举例 |
| --- | --- | --- | --- | --- |
| 一篇原始文档/纪要/扫描件 | `source` | 销售口径 v3 PDF | 教研周会纪要 | 临床指南 PDF |
| 一个具体对象（产品/角色/课程/药品） | `entity` | SKU / 客户 persona | 课程包 / 师资 | 药品 / 科室 |
| 一条规则/政策/术语/SOP | `concept` | 退款政策 / "高意向客户"定义 | 招生合规清单 | 用药禁忌定义 |
| 两个东西并排比 | `comparison` | 我方 vs 竞品 | 班型 A vs B | 用药 X vs Y |
| 多 source 聚合的全景 | `synthesis` | 行业图谱 / 市场综述 | 学段升学路径 | 疾病分型综述 |
| 一套带步骤/分支的方法 | `methodology` | 反对意见处理框架 | 家长砍价 SOP | 询诊话术框架 |
| 一个具体数据点/案例事实 | `finding` | "Q3 转化率 14%" | "本季度续报 76%" | "X 药临床有效率 81%" |
| FAQ：被问过的问题及答 | `query` | "能否分期"答 | "孩子基础差能否上 X 班"答 | "孕期能否用 X 药"答 |
| 一个带立场的判断 | `thesis` | "X 客户必须 face-to-face" | "Y 学段必须冲刺班" | "建议先做 X 检查再用药" |

**经验法则**：如果犹豫 `concept` 还是 `methodology` —— 有步骤/分支的选 `methodology`，纯定义的选 `concept`。如果犹豫 `finding` 还是 `thesis` —— 有数据/事实的选 `finding`，需要论据支撑的判断选 `thesis`。

## 4. provenance 矩阵

每个 chunk 写 `ChunkProvenance`，**source 是必填字段，标记 chunk 的来源链**：

| `source` | 含义 | 写入路径 | `status` 默认 | `integrity_status` 默认 | 谁可以 verify |
| --- | --- | --- | --- | --- | --- |
| `imported` | 从 markdown / PDF / 表格批量导入 | `/operation-knowledge/import-apply` | `draft` | `needs_review` | 运营手动 |
| `ai` | AI chat / 编辑器自主写入 | `chat_apply` / `chunks/:id/patch` | **强制** `draft` | **强制** `needs_review` | 运营手动（AI 永不自动 verify） |
| `human` | 运营在 UI 直接新建/编辑 | `chunks` POST/PUT | 由 UI 决定（默认 draft） | 由 UI 决定 | 运营手动 |
| `rule` | 系统级清理（删除级联、scheduled archive） | `cleanup_dangling_refs` 等 | 沿用既有 | 沿用既有 | 不需要（系统操作） |

`provenance.llm_model_alias` 字段：仅当 `source=ai`，写 LLM provider 的 `provider_id`（如 `"default"`），不写品牌名/模型名。`provenance.source_quote` 字段在 source=imported 时记录原文片段，便于事后核对。

**硬约束**：`source=ai` 的 chunk 永远不会自动 verify，必须经 `/chunks/:id/verify` + sourceQuote→anchor gate 通过后才会进入 catalog 召回。

## 5. lifecycle 状态机

```text
[creating]
   ↓ apply_chunk_revision (op=create)
[draft, needs_review]
   ↓ /chunks/:id/verify (sourceQuote → anchor gate)
[verified, integrity_ok]
   ↓ tool-loop 命中 → record_chunk_hit → usage_stats++ → dynamic_confidence ↑
   |
   ↓ patch / split / merge → apply_chunk_revision → 新 revision_id 累积
   |
   ↓ /chunks/:id/archive  (软删，写 op=archive 历史)
[archived]
   ↓ /chunks/:id/restore   (写 op=restore 历史)
[draft, needs_review]   ← 重新进入 review 流程
   |
   ↓ /chunks/:id/rollback/:revision_id  (找 before-state 重写为 current；写 op=rollback 历史)
[draft, needs_review]
```

任何 op 都进 `chunk_revisions` 不可变 timeline；rollback 不删历史，只是把 before-state 重写为当前并追加新 revision。

## 6. 写入路径三层保护（apply_chunk_revision）

详见 [`src/knowledge_wiki/chunk_revisions.rs`](../src/knowledge_wiki/chunk_revisions.rs) + [`src/knowledge_wiki/page_merge.rs`](../src/knowledge_wiki/page_merge.rs)。所有写入（import / patch / split / merge / archive / restore / rollback）走同一函数，三层保护一律生效：

1. **锁定字段守门** — patch 试图改 `chunk_id / wiki_type / created_at / source_anchor / verified_at / verified_by / approved_at` 任意一项 → `400 BadRequest`，错误信息明确指出受锁定字段。
2. **数组字段 union** — `tags / related_chunks / sources / search_terms / applicable_scenes` 永远 `existing ∪ patch`；即使 LLM 返 `tags: ["仅这一项"]`，应用层 union 后真实落地是并集。0 风险 0 LLM 成本。
3. **70% body 长度阈值** — 当 patch 改 `answer / explanation` 字段时，`new_len < old_len × 0.7` → `400 BadRequest`，识别 LLM 截断 / 偷懒 / 误重写。

附加：`source=ai` 强制 `status="draft" + integrity_status="needs_review"`；写入侧双写"先 revisions 后 chunks"（保留"试图但未成功"的痕迹）；写完即 enqueue `catalog_rebuild_jobs`，写入路径不阻塞。

## 7. patch-only 协议

LLM 编辑 chunk 不返完整页，只返 `patch: { ...field-level diff... }` JSON。后端拿到直接调 `apply_chunk_revision`，模型不可能"顺手"改它没列在 patch 里的字段。借鉴 LLW `enrich-wikilinks.ts:7-22`：

> "Previously we asked the LLM to return the complete page, but many models treat this as an invitation to rewrite. The new design: LLM only returns substitutions as JSON. Code does the actual replacement."

应用到 chat-canvas 的"补出处 / 改路由 / 换说法 / 应用为草稿"全部强制 patch JSON 形态。

## 8. 反馈闭环（feedback_worker）

详见 [`src/knowledge_wiki/feedback_worker.rs`](../src/knowledge_wiki/feedback_worker.rs) + [`src/knowledge_wiki/gap_signals.rs`](../src/knowledge_wiki/gap_signals.rs)。每 `KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS`（默认 600，0 = 关停）一轮：

| 步骤 | 动作 | 输出 |
| --- | --- | --- |
| 1 | 30d 滑窗聚合 `knowledge_usage_logs` | 每 chunk 的 `usage_stats.hit_count_30d / blocked_count_30d / last_used_at / last_blocked_reason` |
| 2 | 朴素公式 | `dynamic_confidence = clamp(integrity_score × 0.6 + hit_rate × 0.4 - stale_penalty, 0, 1)` |
| 3 | structural lint | `knowledge_gap_signals` 5 类规则信号（见下表） |
| 4 | stage 1 sweep | candidate 不再被规则生成的 pending signal → `auto_resolved`；broken_link 的 target 已恢复 → `auto_resolved`；stale 的 valid_to 被推到未来 → `auto_resolved` |

**5 类 structural lint 规则**：

| kind | 触发条件 | severity | 备注 |
| --- | --- | --- | --- |
| `orphan` | chunk 无入链（其它 chunk 的 `related_chunks` 都不指它）且 30d `hit_count == 0` | `info` | 提示运营考虑归档或补关系 |
| `broken_link` | `related_chunks.chunk_id` 指向不存在 / 已 archived 的 chunk | `warning` | target 恢复后 stage 1 自动 resolve |
| `no_outlinks` | `wiki_type ∈ {synthesis, comparison, methodology}` 但 `related_chunks` 为空 | `info` | 这三类按方法论应交叉引用 |
| `low_confidence` | `dynamic_confidence < 0.3` 且 30d `hit_count > 0` | `warning` | 召回但被频繁 block，疑似过期/低质 |
| `stale` | `valid_to < now` 且 `status != "archived"` | `warning` | valid_to 推到未来后 stage 1 自动 resolve |

stage 2（LLM 批裁决：contradiction / suggestion / 残留信号是否仍适用）接口预留在 `sweep_stale_signals`，本轮**不进入热路径**（避免在召回算法零改动的同一个 PR 里引入 LLM 成本不确定性）。

`record_chunk_hit` 在 `agent::knowledge_router::write_knowledge_usage_log` 写 log 后 fire-and-forget 调用，命中即 `$inc usage_stats.hit_count_30d`，被 block 即 `$inc usage_stats.blocked_count_30d` + `$set last_blocked_reason`。**不阻塞召回路径**。

## 9. 行业可配 schema（domain_schemas）

`domain_schemas` 让产品在不同行业用同一份 chunk 主表，`active=true` 一条 / workspace。chunk 写入时按 active schema 校验 `chunks.domain_attributes`：

- `field.required=true` 但缺失 → reject
- `field.kind=enum` 值不在 `allowed_values` → reject
- 命中 `alias_dict` 的 key → 透明 rewrite 为 canonical name（不破坏写入）

校验红线（`/admin/domain-schemas` 路由 [`src/routes/domain_schemas.rs`](../src/routes/domain_schemas.rs)）：

- `fields.len() <= 64`
- `field.name` 不能与 chunk 主表 base 字段名冲突（黑名单：`chunk_id / wiki_type / domain_attributes / provenance / tags / ...`）
- `field.name` 全 schema 内唯一
- `field.kind ∈ {string, enum, number, date, reference}`
- `kind=="enum"` 时必须提供非空 `allowed_values`
- `alias_dict` 每个 value 必须存在于 `fields[].name`

切换 active：`POST /admin/domain-schemas/:id/activate` 把同 workspace 其它 active 全部置 false，再把目标置 true（同 workspace 同时只能 1 条 active）。**切换 active 不重新校验既有 chunk**（`domain_attributes` 历史值原样保留），新写入按新 schema 校验。复杂 DSL（任意条件 / 计算字段）留下一轮，本轮 `guard_dsl` 仅 `field OP value` 简版。

## 10. catalog 双轨

| 路由 | 数据源 | 复杂度 | 何时用 |
| --- | --- | --- | --- |
| `GET /operation-knowledge/catalog/persisted` | `documents.catalog_summary_persisted` 落库快照 + `dynamic_confidence` 排序 | O(N 文档读) | 生产路径默认；前端列表 / 召回 router 拉 catalog |
| `GET /operation-knowledge/catalog` (live) | 实时聚合 N × M chunk | O(N × M) | ops debug / 对账 / 新建 chunk 后立刻看效果 |

写 chunk 后 < 3 s 内 catalog/persisted 反映新数据（`apply_chunk_revision` 写完即 enqueue `catalog_rebuild_jobs`，worker 200 ms 一轮消费）。catalog rebuild 失败 3 次 → `job.status=failed` + 写 `last_error`；feedback worker 周期捞 failed 重试一次。

## 11. 删除级联

`POST /chunks/:id/archive`（软删）后：

1. 写 `chunk_revisions` op=archive 历史；
2. `cleanup_dangling_refs`：grep 其它 chunk 的 `related_chunks.chunk_id == :id`，按 `normalize_ref_key`（lowercase + 去 `[ -_]`）匹配防止子串误伤（"openai" 不匹配 "ai"），命中的 chunk 走 `apply_chunk_revision` op=patch source=rule 删除那条 related_chunks 引用 + 改写 note 中 `[[archived-id]]` 为纯文本。

物理删除 `DELETE /chunks/:id` 仅运营 admin，移除 chunks 行 + 移除 chunk_revisions 历史 + 同上 dangling refs 清理。

## 12. 查得到 / 改得了 / 优化得了 自检清单

每次给 chunk 做一次操作，事后能不能复盘：

- **查得到**：`GET /operation-knowledge/catalog/persisted` 能否在 3 s 内看到这条 chunk？wiki_type / tags / search_terms 是否正确？
- **改得了**：`POST /chunks/:id/patch` 写一个无害字段（如 `tags: ["test_tag"]`）是否成功？`GET /chunks/:id/revisions` 能否看到这条 patch 的 op + before/after hash + reason？
- **优化得了**：手动调用 `POST /knowledge/gap-signals/sweep` 能否在 1 轮内识别新 broken_link / low_confidence？`GET /knowledge/gap-signals?status=pending` 能否列出待处理项？

三件事任一项答 ✗ → 按 [`docs/agent-policy.md` §运维 runbook](agent-policy.md) 七步流程定位修复。

## 13. 不在范围（明确留下一轮）

- 召回算法升级（embedding / hybrid search / RRF / community-aware retrieval）—— 用户明确说本轮不动
- chunk 之间 community detection（Louvain 类算法）—— LLW 有但本轮不抄
- domain_schema `guard_dsl` 扩展（任意条件 / 计算字段）
- AI 协作工作站 UI 推倒重做（双模 segmented control + chat-canvas + status rail）—— 单独一轮 PR
- chunk_revisions 的 git 风格 branch / merge tree —— 本轮仅线性 revision
- multimodal（图片 / PDF / 表格抽取）
- 跨 workspace 知识共享 / 联邦
- chunk 自动 redirect 解析（`superseded_by` 链跳转优先级）—— 写好字段先，召回侧用法下一轮
