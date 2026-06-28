# Prompt 自优化阶段三：新旧对照证据透出（设计 spec）

> 日期：2026-06-28
> 关联：阶段一（PR #50，安全基线）、阶段二（PR #51，证据闭环 G1/G4）
> 上游设计：`docs/superpowers/specs/2026-06-27-prompt-evolution-human-gated-design.md`
> 上游计划：`docs/superpowers/plans/2026-06-27-prompt-evolution-human-gated.md`（阶段三骨架在此细化为本 spec）

## 1. 背景与缺口

prompt 自优化「AI 提议 + 人工把关发布」三阶段计划的**阶段三**。阶段二（PR #51）已让 prompt 候选在发布前用真实模型跑「旧 prompt vs 新 prompt」单条对照，并把对照证据写进两处：

- `ShadowReplay` 模型新增 `original_5gate_hit`(models.rs:4341) / `original_self_critique_addressed`(models.rs:4343)（G4 修复，与 `new_*` 对称）；
- `grade_prompt`(significance.rs:226) 把逐样本与聚合对照写进 `proposal.eval_metrics`（`per_sample_evidence` + `five_gate_hit_delta_per_gate` + 各率 + tokenΔ）。

**探索确证的现状**：前端 `ProposalReleaseCard.tsx`（Ask-Human Phase 2 Task 6 落地）及整条发布/回滚链路**已存在且功能完整**——Critic 推理、双栏 diff、`expectedImprovementOn` 标签、shadow 报告框架、`evalMetrics` 通用 key-value 表、RELEASE/ROLLBACK 确认串弹窗都在。

**真正缺口**：阶段二写入的 original 侧对照数据**进了 DB 与 eval_metrics，但没结构化透出到前端**——

1. `shadow_replay_json`(evolution.rs:507) 只序列化 new 侧字段，漏 `original_5gate_hit` / `original_self_critique_addressed`；
2. 前端 `ShadowReplaySample`(proposalTypes.ts:60) 类型缺 original 侧字段；
3. `ShadowEvalReport`(ProposalReleaseCard.tsx:239) 样本表只展示 new 侧 final_review + tokens，看不到逐样本新旧五闸/自评对照；
4. 聚合证据（`five_gate_hit_delta_per_gate` 等）被 `MetadataSection` 当通用 key-value + `JSON.stringify` 平铺，对照语义没结构化呈现。

阶段三**只补这一段端到端透出**，不重做已存在的链路。

## 2. 目标

prompt 候选详情卡里，管理员在点 RELEASE 前能看到：

- **聚合 5 闸涨跌表**：每闸的原命中率 / 新命中率 / Δ，加自评解决率对照与 token 均值Δ；
- **逐样本新旧对照表**（前 5 条）：run / 原 final_review / 新 final_review / 原五闸 / 新五闸 / 自评（原→新）。

threshold 候选详情卡保持字节级不变。

## 3. 数据流

```
ShadowReplay 模型（已有 original 侧字段，阶段二 G4）
  ├── shadow_replay_json 补 2 字段序列化（本阶段）
  └── grade_prompt 聚合 → proposal.eval_metrics（已有，阶段二）
        ↓
GET /api/evolution/proposals/:id 返回
  （shadowReplays.samples 带 original 侧 + proposal.evalMetrics 聚合）
        ↓
前端 ShadowReplaySample 类型加 original 侧字段（本阶段）
        ↓
ShadowEvalReport（prompt 类）渲染：
  ① 聚合 5 闸涨跌表（从 evalMetrics 结构化读）
  ② 逐样本新旧对照表（从 shadowReplays.samples 读）
```

样本逐条数据源选定 **shadowReplays.samples（修后端透出）**，与现有样本表同源、语义清晰；聚合数据源为 `proposal.evalMetrics`（grade_prompt 已写全）。

## 4. 改动面（4 处，均为已有文件的小增量）

### 4.A 后端：`src/routes/evolution.rs` `shadow_replay_json`（:507）

在现有 json 里补 2 个 key（与 new 侧命名对称），其余字段一字不动：

```rust
"original5gateHit": bson_doc_to_json(&r.original_5gate_hit),
"originalSelfCritiqueAddressed": r.original_self_critique_addressed,
```

`bson_doc_to_json`(evolution.rs:543) 已存在；空 `Document` → `{}`。

### 4.B 前端：`frontend/src/components/review/proposalTypes.ts` `ShadowReplaySample`（:60）

镜像后端补 original 侧字段：

```ts
original5gateHit: Record<string, unknown>;
originalSelfCritiqueAddressed: boolean | null;
```

`evalMetrics` 仍保持 `Record<string, unknown>`（不破坏 MetadataSection 现有通用表与闭集类型 `ProposalStatus`）。聚合字段通过**纯读取 helper** 在运行时窄化取出，不改类型定义。

> **关键：evalMetrics 的 key 是 snake_case。** `grade_prompt`(significance.rs:282) 用 `doc!{"five_gate_hit_delta_per_gate": ..., "original_self_critique_addressed_rate": ..., "new_self_critique_addressed_rate": ..., "self_critique_addressed_delta_observed": ..., "token_cost_delta_mean_observed": ..., "per_sample_evidence": ...}` 写入，`proposal_detail_json`(evolution.rs:474) 用 `bson_doc_to_json` 原样递归透出，**不做 camelCase 转换**（其它 proposal 字段是 handler 手写 camelCase，但 evalMetrics 整块是 bson→json 直转）。故前端窄化 helper 必须读 **snake_case** key，不是 camelCase。这是与该卡片其它字段不同的边界，实现时勿误用 camelCase。

### 4.C 前端：`frontend/src/components/review/ProposalReleaseCard.tsx` `ShadowEvalReport`（:239）

主体已有分支 `proposal.kind === "threshold" ? <ThresholdDiffView> : <PromptDiffView>`（:102-106，非 threshold 即 prompt）。`ShadowEvalReport`(:239) 当前对两类统一渲染（三宫格 + 样本表），阶段三在其内部按 kind 增量：

- **threshold 类**：保持现状（完成/失败/显著性三宫格 + 现有样本表，无对照表）。
- **prompt 类**：三宫格下方加两块——
  - **聚合 5 闸涨跌表**：5 行 gate × (原命中率 / 新命中率 / Δ) + 自评解决率对照 + token 均值Δ。Δ 按「降低拦截/重写命中」方向标 ✓：所有 5 闸的语义都是「被该闸拦下或要求重写」，命中率**下降为好**（候选让更少回复触发闸）。五闸即 block/rewrite 类闸，新候选命中率降低即风险降低。
  - **逐样本新旧对照表**（前 5 条）：run / 原 final / 新 final / 原五闸 / 新五闸 / 自评(原→新)。五闸用紧凑符号（● 命中 / ○ 未中）。
- **聚合证据从 `MetadataSection` 移出——仅对 prompt 类，且按 key 白名单**：
  - `MetadataSection` 的通用 evalMetrics 表当前对**所有 kind** 平铺（ProposalReleaseCard.tsx:314 `Object.entries(proposal.evalMetrics)`，不区分 kind）。
  - **threshold 类有自己独立的一套 evalMetrics key**（grade_threshold significance.rs:173 写 `original_send_success_rate` / `new_send_success_rate` / `send_success_rate_delta` / `safety_regression_*` 等），且**与 prompt 类共享** `five_gate_hit_delta_per_gate`(significance.rs:194) 与 `max_5gate_hit_increase_observed`(significance.rs:180) 两个 key。
  - 因此移出**必须双重限定**：①仅当 `proposal.kind === "prompt"` 时；②只过滤掉**已被聚合表结构化展示的那批 prompt 专属 key**（`kind` / `completed_replay_count` / `failed_replay_count` / `eligibility_basis` / `original_self_critique_addressed_rate` / `new_self_critique_addressed_rate` / `self_critique_addressed_delta_observed` / `max_5gate_hit_increase_observed` / `token_cost_delta_mean_observed` / `five_gate_hit_delta_per_gate` / `per_sample_evidence`）。
  - threshold 类**完全不动**：通用表照常平铺全部 key（含它独有的 send_success/safety 字段），不受白名单影响。绝不能按 key 名笼统过滤（否则会抹掉 threshold 类通用表里的 `five_gate_hit_delta_per_gate` 等共有 key）。

### 4.D 五闸 key 顺序

固定顺序，与后端 `FIVE_GATE_KEYS`(significance.rs:33) 对齐避免渲染漂移：
`fact_risk_block, pressure_risk_block, human_like_score_rewrite, emotional_value_rewrite, product_accuracy_score_block`。
前端给每个 key 配中文短标签（如「事实风险」「施压风险」「人性化(重写)」「情感价值(重写)」「产品准确度」），标签映射以 const 形式集中定义，缺失 key 回落显示原始 key 串。

## 5. 错误处理与边界

- **release 三态**（成功 / 红线拒 / LLM 不可用）已由现有代码覆盖，阶段三不碰：
  - 成功 → `onDone` 刷新；
  - 红线拒 → `release_prompt` 抛 `RedlineGateRejected(String)` → `evolution_error_to_app_error` 映射 `AppError::BadRequest`(evolution.rs:382) → `IntoResponse` 序列化为 `400 + {"error": <reason 字符串>}`(error.rs:62-63) → 前端 `api.post` 的 `parseApiError` 命中 JSON `error` 字符串分支 → `new Error(json.error)`(api.ts:31-32) → `ConfirmModal` catch 后 `setErr(e.message)`(:420)，在 `err` 区（`role="alert"`，:442-446）显示 reason 原文。**已核实：reason 字符串能原样到达管理员**。
  - LLM 不可用：阶段一 `release_prompt` 此情况返回的也是 `RedlineGateRejected`（「请逐字核对后再发布」），同走上面 BadRequest 链——**不是** 503 `llm_unavailable`，故前端不会命中 `LlmUnavailableError` 分支，而是正常显示该提示文字。
  - 阶段三仅确认这条链对 prompt 候选生效，不新增机制、不改 error 映射。
- **空对照数据**：`original5gateHit` 为空 `{}` / `originalSelfCritiqueAddressed` 为 null（旧数据或 threshold 类）→ 对应单元格显示 `—`；聚合表在无任何聚合字段时整表不渲染（沿用现有空值不渲染惯例）。
- **样本上限**：维持现有「前 5 条」（后端 `aggregate_shadow_replays` 已 `samples.len() < 5` 截断，evolution.rs:270）。
- **类型安全**：eval_metrics 取值走窄化 helper（运行时 `typeof` 校验 + 缺失回落），不强转，脏数据不 crash 前端。

## 6. 测试策略

- **后端**：`shadow_replay_json` 新增字段序列化测试——构造带 `original_5gate_hit` / `original_self_critique_addressed` 的 ShadowReplay，断言 json 含 `original5gateHit` / `originalSelfCritiqueAddressed`；空 `Document` → `{}`。
- **前端**（`ProposalReleaseCard.test.tsx` 增量）：现有测试只有 2 个用例（`baseDetail` 固定 `kind:"prompt"` 且 `shadowReplays.samples` 恒为 `[]`，覆盖 metadata 5 字段渲染 + 全空不渲染），**未测 threshold 类、未测样本表渲染**。阶段三新增：
  - prompt 类 + 有对照数据（samples 含 original/new 5gate + evalMetrics 含 snake_case 聚合 key）→ 聚合表渲染 5 行 gate + Δ + 样本表含新旧列；
  - threshold 类（新造 `kind:"threshold"` fixture）→ **不**渲染对照表（回归保护，三宫格 + 通用 evalMetrics 表不变，仍含 send_success/safety 字段）；
  - 空 original 数据（`original5gateHit:{}`、`originalSelfCritiqueAddressed:null`）→ 单元格 `—`，不 crash。
- 不删改现有 2 个用例（memory「新增测试只增量叠加」铁律）。

## 7. 验收标准

- prompt 候选详情卡能看到逐样本新旧五闸/自评对照 + 聚合涨跌表；
- threshold 候选详情卡字节级不变（回归）；
- `cd frontend && npm run build` 通过；`npm run test`（vitest）绿；
- 后端 `cargo test --lib` 不回归基线；
- no-human-takeover lint 绿（前端文案改动会被扫）；
- 遵守 `docs/frontend-design-system.md`：真实 token 在 `components/ui/tokens.css`，CSS 用 `.module.css`；复用 `proposalPrimitives` 的 `formatPercent` / `formatNumber` / `StatusBadge`；蓝仅主操作、紫仅 AI 身份，对照表用中性色，Δ 正负用语义色。

## 8. 不做（YAGNI）

- 不做图表可视化（柱状/折线），对照只用表格；
- 不做样本分页 / 全量展开（维持前 5 条）；
- 不动 threshold 类渲染；
- 不动 release/rollback 后端逻辑（阶段一/二已定）；
- 不引入新前端依赖。

## 9. 设计自审

- **Placeholder 扫描**：无 TBD/TODO；五闸 key 已核对 `FIVE_GATE_KEYS`(significance.rs:33) 真实取值（`fact_risk_block / pressure_risk_block / human_like_score_rewrite / emotional_value_rewrite / product_accuracy_score_block`），非占位遗漏。
- **内部一致性**：数据流（§3）↔ 改动面（§4）↔ 测试（§6）三处对照表的字段口径一致；后端补的 2 字段（§4.A）与前端类型（§4.B）、渲染（§4.C）逐一对应。
- **范围**：聚焦单一缺口（G4 对照数据透出），4 处小增量，适合单个实现计划，无需拆分。
- **歧义**：聚合数据源（evalMetrics）与逐样本数据源（shadowReplays.samples）已显式区分；Δ 的「好/坏」方向已按闸显式定义。

## 10. 深度代码核实记录（2026-06-28，零猜测）

逐条对真实代码核实 spec 断言，结论与证据：

| 断言 | 结论 | 真实代码证据 |
| --- | --- | --- |
| prompt 类 `original_5gate_hit` 真被填充（非空） | CONFIRMED | `shadow_replay_prompt_one` 产 `original_scores`(源 run review.scores, prompt_shadow.rs:103)→ `prompt_sample_to_outcome` 调 `scores_to_5gate_hit`(replay.rs:456/459)→ `persist_replay` 写 `ShadowReplay.original_5gate_hit`(replay.rs:560) |
| `original_self_critique_addressed` 真透传 | CONFIRMED | 源 run `selfCritiqueAddressed`(prompt_shadow.rs:105)→ outcome(replay.rs:477)→ persist(replay.rs:561) |
| eval_metrics 完整（含嵌套）透出前端 | CONFIRMED | `proposal_detail_json`(evolution.rs:474) `bson_doc_to_json(&p.eval_metrics)`，`bson_doc_to_json`(evolution.rs:543) = `serde_json::to_value(Bson::Document)` 递归转换 |
| `shadow_replay_json` 当前漏 original 侧 5gate/selfCritique | CONFIRMED | evolution.rs:507-523 只序列化 new 侧 + `originalFinalReviewStatus`，无 `original5gateHit`/`originalSelfCritiqueAddressed` |
| 红线拒 reason 能原样显示给管理员 | CONFIRMED | `RedlineGateRejected→BadRequest`(evolution.rs:382)→`{"error":msg}`(error.rs:62-63)→`parseApiError` JSON error 分支→`new Error(json.error)`(api.ts:31-32)→`setErr(e.message)`(ProposalReleaseCard.tsx:420) |
| eval_metrics key 是 snake_case（前端 helper 须读 snake_case） | CONFIRMED（已补 §4.B） | `grade_prompt` 写 `doc!{"five_gate_hit_delta_per_gate"...}`(significance.rs:282-299) snake_case，bson 直转不改名 |
| threshold 类 evalMetrics 与 prompt 共享 `five_gate_hit_delta_per_gate`/`max_5gate_hit_increase_observed` | CONFIRMED（已修 §4.C 移出逻辑双重限定） | grade_threshold significance.rs:180/194 也写这两 key；移出须限 `kind==="prompt"` + 具体 key 白名单 |
| threshold 类 `original_5gate_hit` 为空 Document | CONFIRMED | `evaluate_threshold` 返回 `original_5gate_hit: Document::new()`(replay.rs:311) → 对照表只对 prompt 类有数据，与 §4.C/§5 限定一致 |
| 现有 `ProposalReleaseCard.test.tsx` 未测 threshold 类/样本表 | CONFIRMED（已补 §6） | 仅 2 用例，baseDetail 固定 `kind:"prompt"` + `samples:[]` |
| `PromptDiffView` 渲染条件 | 措辞已修正（§4.C） | 真实是 `kind === "threshold" ? Threshold : Prompt` 三元(ProposalReleaseCard.tsx:102-106)，非 `!== "threshold"` |
| `formatPercent`/`formatNumber`/`StatusBadge` 可复用 | CONFIRMED | proposalPrimitives.tsx 导出；null/NaN→`"—"` |

**结论**：spec 全部技术断言已对真实代码闭环；核实中发现并修正 4 处（五闸 key 拼写、evalMetrics snake_case、MetadataSection 移出须双重限定、PromptDiffView 渲染条件措辞）。无残留猜测。
