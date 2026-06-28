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

### 4.C 前端：`frontend/src/components/review/ProposalReleaseCard.tsx` `ShadowEvalReport`（:239）

- **threshold 类**：保持现状（完成/失败/显著性三宫格，无对照表）。
- **prompt 类**：三宫格下方加两块——
  - **聚合 5 闸涨跌表**：5 行 gate × (原命中率 / 新命中率 / Δ) + 自评解决率对照 + token 均值Δ。Δ 按「降低拦截/重写命中」方向标 ✓：所有 5 闸的语义都是「被该闸拦下或要求重写」，命中率**下降为好**（候选让更少回复触发闸）。五闸即 block/rewrite 类闸，新候选命中率降低即风险降低。
  - **逐样本新旧对照表**（前 5 条）：run / 原 final / 新 final / 原五闸 / 新五闸 / 自评(原→新)。五闸用紧凑符号（● 命中 / ○ 未中）。
- **聚合证据从 `MetadataSection` 移出**：prompt 类已被结构化展示的那几个 key 不再在通用 evalMetrics 表里重复平铺；threshold 类不受影响，仍走通用表。

### 4.D 五闸 key 顺序

固定顺序，与后端 `FIVE_GATE_KEYS`(significance.rs:33) 对齐避免渲染漂移：
`fact_risk_block, pressure_risk_block, human_like_score_rewrite, emotional_value_rewrite, product_accuracy_score_block`。
前端给每个 key 配中文短标签（如「事实风险」「施压风险」「人性化(重写)」「情感价值(重写)」「产品准确度」），标签映射以 const 形式集中定义，缺失 key 回落显示原始 key 串。

## 5. 错误处理与边界

- **release 三态**（成功 / 红线拒 / LLM 不可用）已由现有代码覆盖，阶段三不碰：
  - 成功 → `onDone` 刷新；
  - 红线拒 → `release_prompt` 抛 `RedlineGateRejected` → `evolution_error_to_app_error` 映射 `BadRequest`(evolution.rs:382) → 前端 `api.post` 抛 `parseApiError` → `ConfirmModal` 的 `err` 区（`role="alert"`）显示 reason；
  - LLM 不可用 → 同走 `RedlineGateRejected`（阶段一定的「请逐字核对后再发布」）。
  - 阶段三仅确认这条链对 prompt 候选生效，不新增机制。
- **空对照数据**：`original5gateHit` 为空 `{}` / `originalSelfCritiqueAddressed` 为 null（旧数据或 threshold 类）→ 对应单元格显示 `—`；聚合表在无任何聚合字段时整表不渲染（沿用现有空值不渲染惯例）。
- **样本上限**：维持现有「前 5 条」（后端 `aggregate_shadow_replays` 已 `samples.len() < 5` 截断，evolution.rs:270）。
- **类型安全**：eval_metrics 取值走窄化 helper（运行时 `typeof` 校验 + 缺失回落），不强转，脏数据不 crash 前端。

## 6. 测试策略

- **后端**：`shadow_replay_json` 新增字段序列化测试——构造带 `original_5gate_hit` / `original_self_critique_addressed` 的 ShadowReplay，断言 json 含 `original5gateHit` / `originalSelfCritiqueAddressed`；空 `Document` → `{}`。
- **前端**（`ProposalReleaseCard.test.tsx` 增量）：
  - prompt 类 + 有对照数据 → 聚合表渲染 5 行 gate + Δ + 样本表含新旧列；
  - threshold 类 → **不**渲染对照表（回归保护，三宫格不变）；
  - 空 original 数据 → 单元格 `—`，不 crash。
- 不删改现有测试维度（memory「新增测试只增量叠加」铁律）。

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
