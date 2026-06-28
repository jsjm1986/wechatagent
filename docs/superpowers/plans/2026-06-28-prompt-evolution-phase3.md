# Prompt 自优化阶段三：新旧对照证据透出 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把阶段二 G4 已写入 DB / eval_metrics 的「新旧对照证据」端到端透出到演化候选详情卡，让管理员在 RELEASE 前看到逐样本与聚合的新旧五闸/自评对照。

**Architecture:** 后端 `shadow_replay_json` 补 2 个 original 侧字段序列化（数据已在 `ShadowReplay` 模型，仅没透出）；前端 `ShadowReplaySample` 类型镜像补字段，`ShadowEvalReport` 在 prompt 类下加「聚合 5 闸涨跌表 + 逐样本新旧对照表」，聚合证据从 `MetadataSection` 通用表按 `kind==="prompt"` + key 白名单移出。threshold 类渲染与发布/回滚链路一律不动。

**Tech Stack:** Rust (Axum) 后端 JSON handler + 纯函数单测；React 19 + TypeScript + Vite 前端，CSS Modules，vitest + @testing-library/react。

## Global Constraints

- 所有路径基于 worktree `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\prompt-evolution`；Read/Edit 用此 worktree 下路径，Bash 的 cwd 已在此 worktree。
- Cargo 编译共享主仓 target：每条 cargo 命令前 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`。
- **本地 lib 基线门会被主 worktree 串产物污染**（共享 target + 同 crate metadata），lib 全量验证交由 CI；本地只跑「目标单测过滤」+ `cargo check`。
- 前端测试：`cd frontend && npm run test`（vitest run）；构建 `npm run build`。
- 后端补的 JSON key 用 camelCase（与 `shadow_replay_json` 既有字段一致：`newFinalReviewStatus` 等）。
- 前端读 `evalMetrics` 内字段用 **snake_case**（`grade_prompt` 写 snake_case，`bson_doc_to_json` 原样透出，不转 camelCase）。
- 五闸 key 固定顺序（与 `FIVE_GATE_KEYS` significance.rs:33 对齐）：`fact_risk_block, pressure_risk_block, human_like_score_rewrite, emotional_value_rewrite, product_accuracy_score_block`。
- 文案严守 AI 自主语义；`scripts/check-no-human-takeover.{sh,ps1}` 会扫前端新增行的禁用词（`人工`/`接管`/`takeover`/`hand-off` 等）。
- CSS 色值只用 `tokens.css` 变量（`--ink-1/2/3/4`、`--hairline`、`--surface-card/page`、`--color-running/blocked/held/brand`、`--fill-*`、`--r-sm/md`），禁止硬编码十六进制（现有 `.module.css` 个别旧值除外，新增规则不得引入）。
- 不删改现有测试用例（只增量叠加）。

---

## File Structure

| 文件 | 改动 | 责任 |
| --- | --- | --- |
| `src/routes/evolution.rs` | Modify `shadow_replay_json`(:507) + 加 1 单测于 `mod tests`(:709) | 后端透出 original 侧 5gate/selfCritique |
| `frontend/src/components/review/proposalTypes.ts` | Modify `ShadowReplaySample`(:60) | 类型镜像补 original 侧字段 |
| `frontend/src/components/review/evidenceMetrics.ts` | Create | 纯函数：从 `evalMetrics`(Record) 窄化读聚合证据 + 五闸标签/方向 const |
| `frontend/src/components/review/ProposalReleaseCard.tsx` | Modify `ShadowEvalReport`(:239) + `MetadataSection`(:304) | prompt 类对照表渲染 + 聚合证据移出 |
| `frontend/src/components/review/ProposalReleaseCard.module.css` | Modify | 对照表样式（复用 token 变量） |
| `frontend/src/__tests__/components/review/ProposalReleaseCard.test.tsx` | Modify | prompt 对照表 + threshold 回归 + 空值用例 |
| `frontend/src/__tests__/components/review/evidenceMetrics.test.ts` | Create | 窄化 helper 纯函数单测 |

---

## Task 1: 后端 `shadow_replay_json` 透出 original 侧对照字段

**Files:**
- Modify: `src/routes/evolution.rs:507-523`（`shadow_replay_json`）
- Test: `src/routes/evolution.rs:709`（`mod tests` 内追加）

**Interfaces:**
- Consumes: `ShadowReplay` 模型（models.rs:4324），含 `original_5gate_hit: Document`(:4341)、`original_self_critique_addressed: Option<bool>`(:4343)。
- Consumes: `bson_doc_to_json(&Document) -> Value`(evolution.rs:543)，空 Document → `{}`。
- Produces: `shadow_replay_json(&ShadowReplay) -> Value` 的返回 JSON 新增 `original5gateHit` / `originalSelfCritiqueAddressed`。

- [ ] **Step 1: 写失败测试**（追加到 `mod tests` 末尾，line 783 后、`test_app_config` 前）

```rust
    #[test]
    fn shadow_replay_json_exposes_original_side_for_comparison() {
        use mongodb::bson::doc;
        let r = ShadowReplay {
            id: Some(ObjectId::new()),
            proposal_id: ObjectId::new(),
            experiment_id: "EXP1".to_string(),
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            source_run_id: ObjectId::new(),
            status: "completed".to_string(),
            failure_reason: None,
            original_final_review_status: Some("held_by_ai_policy".to_string()),
            original_5gate_hit: doc! {
                "fact_risk_block": true,
                "pressure_risk_block": false,
                "human_like_score_rewrite": false,
                "emotional_value_rewrite": false,
                "product_accuracy_score_block": false,
            },
            original_self_critique_addressed: Some(false),
            new_final_review_status: Some("approved".to_string()),
            new_review_risks: Vec::new(),
            new_token_cost: Some(321),
            new_5gate_hit: doc! {
                "fact_risk_block": false,
                "pressure_risk_block": false,
                "human_like_score_rewrite": false,
                "emotional_value_rewrite": false,
                "product_accuracy_score_block": false,
            },
            new_self_critique_addressed: Some(true),
            similarity_to_original_text: 0.0,
            started_at: DateTime::now(),
            finished_at: Some(DateTime::now()),
        };
        let v = shadow_replay_json(&r);
        // 新增 original 侧对照字段
        assert_eq!(v["original5gateHit"]["fact_risk_block"], true);
        assert_eq!(v["original5gateHit"]["human_like_score_rewrite"], false);
        assert_eq!(v["originalSelfCritiqueAddressed"], false);
        // 既有 new 侧不回归
        assert_eq!(v["new5gateHit"]["fact_risk_block"], false);
        assert_eq!(v["newSelfCritiqueAddressed"], true);
        assert_eq!(v["originalFinalReviewStatus"], "held_by_ai_policy");
    }

    #[test]
    fn shadow_replay_json_empty_original_5gate_is_empty_object() {
        let r = ShadowReplay {
            id: None,
            proposal_id: ObjectId::new(),
            experiment_id: "EXP1".to_string(),
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            source_run_id: ObjectId::new(),
            status: "failed".to_string(),
            failure_reason: Some("source_message_unavailable".to_string()),
            original_final_review_status: None,
            original_5gate_hit: mongodb::bson::Document::new(),
            original_self_critique_addressed: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
            new_token_cost: None,
            new_5gate_hit: mongodb::bson::Document::new(),
            new_self_critique_addressed: None,
            similarity_to_original_text: 0.0,
            started_at: DateTime::now(),
            finished_at: None,
        };
        let v = shadow_replay_json(&r);
        // 空 Document → {}；Option None → null
        assert_eq!(v["original5gateHit"], serde_json::json!({}));
        assert!(v["originalSelfCritiqueAddressed"].is_null());
    }
```

- [ ] **Step 2: 运行验证失败**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib shadow_replay_json_exposes_original_side -- --exact 2>&1 | tail -20`
Expected: 编译失败或断言失败——`original5gateHit` 字段不存在于返回 JSON（`v["original5gateHit"]` 为 null，断言 `== true` 失败）。

- [ ] **Step 3: 写实现**（在 `shadow_replay_json` 的 json! 里，紧接 `originalFinalReviewStatus` 行后加 2 行）

```rust
fn shadow_replay_json(r: &ShadowReplay) -> Value {
    json!({
        "id": r.id.map(|o| o.to_hex()),
        "sourceRunId": r.source_run_id.to_hex(),
        "status": r.status,
        "failureReason": r.failure_reason,
        "originalFinalReviewStatus": r.original_final_review_status,
        "original5gateHit": bson_doc_to_json(&r.original_5gate_hit),
        "originalSelfCritiqueAddressed": r.original_self_critique_addressed,
        "newFinalReviewStatus": r.new_final_review_status,
        "newReviewRisks": r.new_review_risks,
        "newTokenCost": r.new_token_cost,
        "new5gateHit": bson_doc_to_json(&r.new_5gate_hit),
        "newSelfCritiqueAddressed": r.new_self_critique_addressed,
        "similarityToOriginalText": r.similarity_to_original_text,
        "startedAt": datetime_to_rfc3339(r.started_at),
        "finishedAt": r.finished_at.map(datetime_to_rfc3339),
    })
}
```

- [ ] **Step 4: 运行验证通过**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib shadow_replay_json -- 2>&1 | tail -20`
Expected: 2 个新测试 PASS（`shadow_replay_json_exposes_original_side_for_comparison`、`shadow_replay_json_empty_original_5gate_is_empty_object`）。

- [ ] **Step 5: Commit**

```bash
git add src/routes/evolution.rs
git commit -m "feat(evolution): shadow_replay_json 透出 original 侧 5gate/selfCritique 对照字段"
```

---

## Task 2: 前端类型补 original 侧 + 窄化读取 helper

**Files:**
- Modify: `frontend/src/components/review/proposalTypes.ts:60-74`（`ShadowReplaySample`）
- Create: `frontend/src/components/review/evidenceMetrics.ts`
- Test: `frontend/src/__tests__/components/review/evidenceMetrics.test.ts`

**Interfaces:**
- Consumes: 后端 `shadow_replay_json`（Task 1）的 `original5gateHit` / `originalSelfCritiqueAddressed`。
- Produces:
  - `ShadowReplaySample` 新增 `original5gateHit: Record<string, unknown>` + `originalSelfCritiqueAddressed: boolean | null`。
  - `FIVE_GATE_KEYS: readonly string[]`（5 个，固定序）。
  - `GATE_LABELS: Record<string, string>`（key → 中文短标签）。
  - `gateHit(doc: Record<string, unknown> | null | undefined, gate: string): boolean | null`（读布尔，缺失/非布尔 → null）。
  - `interface AggregateEvidence { gateDeltas: {gate:string; original:number|null; neu:number|null; delta:number|null}[]; originalCritiqueRate:number|null; newCritiqueRate:number|null; critiqueDelta:number|null; tokenDelta:number|null; }`
  - `readAggregateEvidence(evalMetrics: Record<string, unknown>): AggregateEvidence | null`（无任何聚合字段 → null）。
  - `PROMPT_AGG_METRIC_KEYS: readonly string[]`（MetadataSection 移出白名单）。

- [ ] **Step 1: 写失败测试** `frontend/src/__tests__/components/review/evidenceMetrics.test.ts`

```ts
import { describe, it, expect } from "vitest";
import {
  gateHit,
  readAggregateEvidence,
  FIVE_GATE_KEYS,
  PROMPT_AGG_METRIC_KEYS,
} from "../../../components/review/evidenceMetrics";

describe("evidenceMetrics 窄化读取", () => {
  it("FIVE_GATE_KEYS 固定 5 个且顺序与后端一致", () => {
    expect(FIVE_GATE_KEYS).toEqual([
      "fact_risk_block",
      "pressure_risk_block",
      "human_like_score_rewrite",
      "emotional_value_rewrite",
      "product_accuracy_score_block",
    ]);
  });

  it("gateHit 读布尔，缺失/非布尔回落 null", () => {
    expect(gateHit({ fact_risk_block: true }, "fact_risk_block")).toBe(true);
    expect(gateHit({ fact_risk_block: false }, "fact_risk_block")).toBe(false);
    expect(gateHit({}, "fact_risk_block")).toBeNull();
    expect(gateHit(null, "fact_risk_block")).toBeNull();
    expect(gateHit({ fact_risk_block: "yes" }, "fact_risk_block")).toBeNull();
  });

  it("readAggregateEvidence 读 snake_case 聚合字段", () => {
    const m = {
      kind: "prompt",
      five_gate_hit_delta_per_gate: {
        fact_risk_block: -0.2,
        pressure_risk_block: 0,
        human_like_score_rewrite: 0.1,
        emotional_value_rewrite: 0,
        product_accuracy_score_block: -0.1,
      },
      original_self_critique_addressed_rate: 0.4,
      new_self_critique_addressed_rate: 0.7,
      self_critique_addressed_delta_observed: 0.3,
      token_cost_delta_mean_observed: 120,
    };
    const ev = readAggregateEvidence(m)!;
    expect(ev).not.toBeNull();
    expect(ev.gateDeltas).toHaveLength(5);
    expect(ev.gateDeltas[0]).toEqual({
      gate: "fact_risk_block",
      original: null, // per-gate 原始率不在聚合 doc 里，只有 delta；original/neu 由调用方从样本另算或留 null
      neu: null,
      delta: -0.2,
    });
    expect(ev.originalCritiqueRate).toBe(0.4);
    expect(ev.newCritiqueRate).toBe(0.7);
    expect(ev.critiqueDelta).toBe(0.3);
    expect(ev.tokenDelta).toBe(120);
  });

  it("readAggregateEvidence 无聚合字段返回 null", () => {
    expect(readAggregateEvidence({})).toBeNull();
    expect(readAggregateEvidence({ win_rate: 0.5 })).toBeNull();
  });

  it("PROMPT_AGG_METRIC_KEYS 含被结构化展示的 prompt 专属 key", () => {
    expect(PROMPT_AGG_METRIC_KEYS).toContain("five_gate_hit_delta_per_gate");
    expect(PROMPT_AGG_METRIC_KEYS).toContain("per_sample_evidence");
    expect(PROMPT_AGG_METRIC_KEYS).toContain("self_critique_addressed_delta_observed");
  });
});
```

> 说明：聚合 doc（`grade_prompt` significance.rs:282）只写 per-gate **delta**（`five_gate_hit_delta_per_gate`）与自评的 original/new **率**，没有 per-gate 的 original/new 命中率。故 `gateDeltas[].original/neu` 在仅有聚合 doc 时为 null，per-gate 原始/新命中率由 §Task3 从逐样本 `per_sample_evidence` 或 `samples` 现算。helper 此处只透传 delta，original/neu 占位 null。

- [ ] **Step 2: 运行验证失败**

Run: `cd frontend && npm run test -- evidenceMetrics 2>&1 | tail -20`
Expected: FAIL（模块 `evidenceMetrics` 不存在 / 导出未定义）。

- [ ] **Step 3a: 改类型** `proposalTypes.ts`（`ShadowReplaySample` 接口，在 `originalFinalReviewStatus` 行后加 2 行）

```ts
export interface ShadowReplaySample {
  id: string | null;
  sourceRunId: string;
  status: string;
  failureReason: string | null;
  originalFinalReviewStatus: string | null;
  original5gateHit: Record<string, unknown>;
  originalSelfCritiqueAddressed: boolean | null;
  newFinalReviewStatus: string | null;
  newReviewRisks: unknown;
  newTokenCost: number | null;
  new5gateHit: Record<string, unknown>;
  newSelfCritiqueAddressed: boolean | null;
  similarityToOriginalText: number | null;
  startedAt: string;
  finishedAt: string | null;
}
```

- [ ] **Step 3b: 写 helper** `frontend/src/components/review/evidenceMetrics.ts`

```ts
// 阶段三：从演化候选 evalMetrics（Record<string,unknown>，bson 直转的 snake_case）
// 窄化读出新旧对照聚合证据。evalMetrics key 是 snake_case（grade_prompt
// significance.rs:282 写入，proposal_detail_json 用 bson_doc_to_json 原样透出，
// 不转 camelCase）——故此处一律读 snake_case。

export const FIVE_GATE_KEYS = [
  "fact_risk_block",
  "pressure_risk_block",
  "human_like_score_rewrite",
  "emotional_value_rewrite",
  "product_accuracy_score_block",
] as const;

export const GATE_LABELS: Record<string, string> = {
  fact_risk_block: "事实风险",
  pressure_risk_block: "施压风险",
  human_like_score_rewrite: "人性化(重写)",
  emotional_value_rewrite: "情感价值(重写)",
  product_accuracy_score_block: "产品准确度",
};

// MetadataSection 通用 evalMetrics 表里，prompt 类需移出（已被对照表结构化展示）的 key。
// 注意：five_gate_hit_delta_per_gate / max_5gate_hit_increase_observed 也被
// threshold 类 evalMetrics 使用（grade_threshold significance.rs:180/194），
// 故移出只在 kind==="prompt" 时按本白名单做，绝不对 threshold 类生效。
export const PROMPT_AGG_METRIC_KEYS = [
  "kind",
  "completed_replay_count",
  "failed_replay_count",
  "eligibility_basis",
  "original_self_critique_addressed_rate",
  "new_self_critique_addressed_rate",
  "self_critique_addressed_delta_observed",
  "max_5gate_hit_increase_observed",
  "token_cost_delta_mean_observed",
  "five_gate_hit_delta_per_gate",
  "per_sample_evidence",
] as const;

export function gateHit(
  doc: Record<string, unknown> | null | undefined,
  gate: string,
): boolean | null {
  if (!doc || typeof doc !== "object") return null;
  const v = (doc as Record<string, unknown>)[gate];
  return typeof v === "boolean" ? v : null;
}

function numOrNull(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

export interface GateDelta {
  gate: string;
  original: number | null;
  neu: number | null;
  delta: number | null;
}

export interface AggregateEvidence {
  gateDeltas: GateDelta[];
  originalCritiqueRate: number | null;
  newCritiqueRate: number | null;
  critiqueDelta: number | null;
  tokenDelta: number | null;
}

export function readAggregateEvidence(
  evalMetrics: Record<string, unknown>,
): AggregateEvidence | null {
  if (!evalMetrics || typeof evalMetrics !== "object") return null;
  const perGate = evalMetrics["five_gate_hit_delta_per_gate"];
  const hasPerGate = perGate != null && typeof perGate === "object";
  const originalCritiqueRate = numOrNull(
    evalMetrics["original_self_critique_addressed_rate"],
  );
  const newCritiqueRate = numOrNull(
    evalMetrics["new_self_critique_addressed_rate"],
  );
  const critiqueDelta = numOrNull(
    evalMetrics["self_critique_addressed_delta_observed"],
  );
  const tokenDelta = numOrNull(evalMetrics["token_cost_delta_mean_observed"]);

  // 无任何聚合字段 → 不是 prompt 证据，返回 null（threshold/旧数据）。
  if (
    !hasPerGate &&
    originalCritiqueRate === null &&
    newCritiqueRate === null &&
    critiqueDelta === null &&
    tokenDelta === null
  ) {
    return null;
  }

  const perGateObj = hasPerGate ? (perGate as Record<string, unknown>) : {};
  const gateDeltas: GateDelta[] = FIVE_GATE_KEYS.map((gate) => ({
    gate,
    original: null,
    neu: null,
    delta: numOrNull(perGateObj[gate]),
  }));

  return {
    gateDeltas,
    originalCritiqueRate,
    newCritiqueRate,
    critiqueDelta,
    tokenDelta,
  };
}
```

- [ ] **Step 4: 运行验证通过**

Run: `cd frontend && npm run test -- evidenceMetrics 2>&1 | tail -20`
Expected: 5 个测试全 PASS。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/review/proposalTypes.ts frontend/src/components/review/evidenceMetrics.ts frontend/src/__tests__/components/review/evidenceMetrics.test.ts
git commit -m "feat(review): ShadowReplaySample 补 original 侧 + evidenceMetrics 窄化读取 helper"
```

---

## Task 3: `ShadowEvalReport` prompt 类新旧对照表渲染

**Files:**
- Modify: `frontend/src/components/review/ProposalReleaseCard.tsx:239-296`（`ShadowEvalReport`）
- Modify: `frontend/src/components/review/ProposalReleaseCard.module.css`（加对照表样式）
- Test: `frontend/src/__tests__/components/review/ProposalReleaseCard.test.tsx`（加 prompt 对照表用例）

**Interfaces:**
- Consumes: Task 2 的 `FIVE_GATE_KEYS`、`GATE_LABELS`、`gateHit`、`readAggregateEvidence`、`AggregateEvidence`；`proposalPrimitives` 的 `formatPercent`、`formatNumber`。
- Consumes: `ShadowReplaySample`（Task 2 扩展后）、`ProposalDetail.evalMetrics`、`ProposalDetail.kind`。
- Produces: prompt 类详情卡内 `data-testid="evidence-aggregate"`（聚合表）+ `data-testid="evidence-samples"`（样本对照表）。

- [ ] **Step 1: 写失败测试**（追加到 `ProposalReleaseCard.test.tsx`，新 describe 块）

```tsx
describe("ProposalReleaseCard prompt 新旧对照表（阶段三）", () => {
  beforeEach(() => vi.clearAllMocks());

  function promptDetailWithEvidence() {
    const d = baseDetail({
      evalMetrics: {
        kind: "prompt",
        completed_replay_count: 2,
        failed_replay_count: 0,
        five_gate_hit_delta_per_gate: {
          fact_risk_block: -0.5,
          pressure_risk_block: 0,
          human_like_score_rewrite: 0,
          emotional_value_rewrite: 0,
          product_accuracy_score_block: -0.5,
        },
        original_self_critique_addressed_rate: 0.5,
        new_self_critique_addressed_rate: 1.0,
        self_critique_addressed_delta_observed: 0.5,
        token_cost_delta_mean_observed: 88,
      },
    });
    d.shadowReplays = {
      totalCompleted: 2,
      totalFailed: 0,
      samples: [
        {
          id: "s1",
          sourceRunId: "run-001",
          status: "completed",
          failureReason: null,
          originalFinalReviewStatus: "held_by_ai_policy",
          original5gateHit: { fact_risk_block: true, pressure_risk_block: false, human_like_score_rewrite: false, emotional_value_rewrite: false, product_accuracy_score_block: false },
          originalSelfCritiqueAddressed: false,
          newFinalReviewStatus: "approved",
          newReviewRisks: [],
          newTokenCost: 200,
          new5gateHit: { fact_risk_block: false, pressure_risk_block: false, human_like_score_rewrite: false, emotional_value_rewrite: false, product_accuracy_score_block: false },
          newSelfCritiqueAddressed: true,
          similarityToOriginalText: 0,
          startedAt: "2026-06-01T00:00:00Z",
          finishedAt: "2026-06-01T00:00:01Z",
        },
      ],
    };
    return d;
  }

  it("prompt 类有对照数据时渲染聚合表与样本对照表", async () => {
    getMock.mockResolvedValue(promptDetailWithEvidence());
    renderCard();

    const agg = await screen.findByTestId("evidence-aggregate");
    // 5 行 gate 标签
    expect(agg).toHaveTextContent("事实风险");
    expect(agg).toHaveTextContent("产品准确度");
    // 自评率对照
    expect(agg).toHaveTextContent("自评解决率");

    const samples = screen.getByTestId("evidence-samples");
    expect(samples).toHaveTextContent("run-001");
    expect(samples).toHaveTextContent("held_by_ai_policy");
    expect(samples).toHaveTextContent("approved");
  });

  it("空 original 数据时样本单元格显示占位不 crash", async () => {
    const d = promptDetailWithEvidence();
    d.shadowReplays.samples[0].original5gateHit = {};
    d.shadowReplays.samples[0].originalSelfCritiqueAddressed = null;
    getMock.mockResolvedValue(d);
    renderCard();

    const samples = await screen.findByTestId("evidence-samples");
    expect(samples).toHaveTextContent("run-001"); // 仍渲染，不 crash
  });
});
```

- [ ] **Step 2: 运行验证失败**

Run: `cd frontend && npm run test -- ProposalReleaseCard 2>&1 | tail -25`
Expected: FAIL（`evidence-aggregate` / `evidence-samples` testid 不存在）。

- [ ] **Step 3a: 改 `ShadowEvalReport`**（`ProposalReleaseCard.tsx`）

在文件顶部 import 处（:25-29 的 type import 附近）加：

```tsx
import {
  FIVE_GATE_KEYS,
  GATE_LABELS,
  gateHit,
  readAggregateEvidence,
} from "./evidenceMetrics";
```

把现有 `ShadowEvalReport`（:239-296）整体替换为（保留 threshold 三宫格 + 原样本表不变，prompt 类追加两表）：

```tsx
function ShadowEvalReport({
  summary,
  proposal,
}: {
  summary: ShadowReplaysSummary;
  proposal: ProposalDetail;
}) {
  const isPrompt = proposal.kind !== "threshold";
  const aggregate = isPrompt ? readAggregateEvidence(proposal.evalMetrics ?? {}) : null;
  return (
    <section className={styles.shadowEval} data-testid="shadow-eval">
      <h4>Shadow 评测</h4>
      <div className={styles.shadowGrid}>
        <div data-testid="shadow-completed">
          <span>完成</span>
          <strong>{summary.totalCompleted}</strong>
        </div>
        <div data-testid="shadow-failed">
          <span>失败</span>
          <strong>{summary.totalFailed}</strong>
        </div>
        <div data-testid="shadow-significance">
          <span>显著性</span>
          <strong>
            {proposal.significancePassed === null
              ? "—"
              : proposal.significancePassed
              ? "通过"
              : "未通过"}
          </strong>
        </div>
      </div>

      {isPrompt && aggregate && (
        <div className={styles.evidenceAggregate} data-testid="evidence-aggregate">
          <h5>新旧对照·五闸涨跌</h5>
          <table className={styles.evidenceTable}>
            <thead>
              <tr>
                <th>闸</th>
                <th>Δ 命中率</th>
              </tr>
            </thead>
            <tbody>
              {aggregate.gateDeltas.map((g) => (
                <tr key={g.gate}>
                  <td>{GATE_LABELS[g.gate] ?? g.gate}</td>
                  <td className={deltaToneClass(g.delta)}>
                    {g.delta === null ? "—" : `${g.delta > 0 ? "+" : ""}${formatPercent(g.delta)}`}
                  </td>
                </tr>
              ))}
              <tr>
                <td>自评解决率</td>
                <td>
                  {formatPercent(aggregate.originalCritiqueRate)} →{" "}
                  {formatPercent(aggregate.newCritiqueRate)}
                  {aggregate.critiqueDelta !== null && (
                    <span className={deltaToneClass(aggregate.critiqueDelta, true)}>
                      {" "}({aggregate.critiqueDelta > 0 ? "+" : ""}
                      {formatPercent(aggregate.critiqueDelta)})
                    </span>
                  )}
                </td>
              </tr>
              <tr>
                <td>token 均值Δ</td>
                <td>{formatNumber(aggregate.tokenDelta)}</td>
              </tr>
            </tbody>
          </table>
        </div>
      )}

      {isPrompt && summary.samples.length > 0 && (
        <div className={styles.evidenceSamples} data-testid="evidence-samples">
          <h5>逐样本新旧对照（前 5 条）</h5>
          <table className={styles.evidenceTable}>
            <thead>
              <tr>
                <th>run</th>
                <th>原 final</th>
                <th>新 final</th>
                <th>原五闸</th>
                <th>新五闸</th>
                <th>自评</th>
              </tr>
            </thead>
            <tbody>
              {summary.samples.map((s) => (
                <tr key={s.id ?? s.sourceRunId}>
                  <td>{s.sourceRunId}</td>
                  <td>{s.originalFinalReviewStatus ?? "—"}</td>
                  <td>{s.newFinalReviewStatus ?? "—"}</td>
                  <td>{renderGateDots(s.original5gateHit)}</td>
                  <td>{renderGateDots(s.new5gateHit)}</td>
                  <td>
                    {fmtCritique(s.originalSelfCritiqueAddressed)}→
                    {fmtCritique(s.newSelfCritiqueAddressed)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* threshold 类保留原样本表（new 侧），prompt 类已被上方对照表取代 */}
      {!isPrompt && summary.samples.length > 0 && (
        <details>
          <summary>样本（前 5 条）</summary>
          <table>
            <thead>
              <tr>
                <th>source_run_id</th>
                <th>原 final_review</th>
                <th>新 final_review</th>
                <th>tokens</th>
              </tr>
            </thead>
            <tbody>
              {summary.samples.map((s) => (
                <tr key={s.id ?? s.sourceRunId}>
                  <td>{s.sourceRunId}</td>
                  <td>{s.originalFinalReviewStatus ?? "—"}</td>
                  <td>{s.newFinalReviewStatus ?? "—"}</td>
                  <td>{s.newTokenCost ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </section>
  );
}

// 五闸命中点阵：按固定序渲染 ●(命中)/○(未中)/·(缺失)。
function renderGateDots(doc: Record<string, unknown>): string {
  return FIVE_GATE_KEYS.map((g) => {
    const h = gateHit(doc, g);
    return h === null ? "·" : h ? "●" : "○";
  }).join("");
}

function fmtCritique(v: boolean | null): string {
  return v === null ? "—" : v ? "已解决" : "未解决";
}

// Δ 语义色：五闸命中率下降为好(绿)、上升为坏(红)；自评率方向相反(critiqueGood=true 时升为好)。
function deltaToneClass(delta: number | null, critiqueGood = false): string {
  if (delta === null || delta === 0) return styles.deltaNeutral;
  const good = critiqueGood ? delta > 0 : delta < 0;
  return good ? styles.deltaGood : styles.deltaBad;
}
```

- [ ] **Step 3b: 加 CSS**（`ProposalReleaseCard.module.css` 末尾追加，复用 token 变量）

```css
/* —— 阶段三：新旧对照证据表 —— */
.evidenceAggregate,
.evidenceSamples { display: grid; gap: 8px; }
.evidenceAggregate h5,
.evidenceSamples h5 {
  margin: 0; font-size: 11.5px; font-weight: 600; color: var(--ink-3); letter-spacing: .1px;
}
.evidenceTable { width: 100%; border-collapse: collapse; }
.evidenceTable th {
  text-align: left; font-size: 11px; color: var(--ink-3); font-weight: 600;
  padding: 6px 8px; border-bottom: 1px solid var(--hairline);
}
.evidenceTable td {
  font-size: 11.5px; color: var(--ink-1); padding: 6px 8px;
  border-bottom: 1px solid var(--hairline);
  font-family: ui-monospace, "SF Mono", Menlo, monospace; word-break: break-all;
}
.evidenceTable tr:last-child td { border-bottom: none; }
.deltaGood { color: var(--color-running); }
.deltaBad { color: var(--color-blocked); }
.deltaNeutral { color: var(--ink-3); }
```

- [ ] **Step 4: 运行验证通过**

Run: `cd frontend && npm run test -- ProposalReleaseCard 2>&1 | tail -25`
Expected: 新增 2 用例 PASS；现有 2 用例仍 PASS（metadata 5 字段 + 全空不渲染）。

> 注意：现有「metadata 5 字段」用例的 `evalMetrics: { win_rate, sample_size }` 不含聚合 key，`readAggregateEvidence` 返回 null → 不渲染 evidence-aggregate，不影响该用例的 `proposal-eval-metrics` 断言（Task 4 才改 MetadataSection 移出，此处尚未移出，win_rate 仍在通用表）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/review/ProposalReleaseCard.tsx frontend/src/components/review/ProposalReleaseCard.module.css frontend/src/__tests__/components/review/ProposalReleaseCard.test.tsx
git commit -m "feat(review): ShadowEvalReport prompt 类渲染新旧对照聚合表+逐样本表"
```

---

## Task 4: `MetadataSection` 聚合证据移出（prompt + key 白名单）+ threshold 回归

**Files:**
- Modify: `frontend/src/components/review/ProposalReleaseCard.tsx:304-377`（`MetadataSection`）
- Test: `frontend/src/__tests__/components/review/ProposalReleaseCard.test.tsx`（加 threshold 回归 + prompt 移出用例）

**Interfaces:**
- Consumes: Task 2 的 `PROMPT_AGG_METRIC_KEYS`、`ProposalDetail.kind`。
- Produces: prompt 类通用 evalMetrics 表过滤掉白名单 key；threshold 类不变。

- [ ] **Step 1: 写失败测试**（追加到 `ProposalReleaseCard.test.tsx`）

```tsx
describe("MetadataSection 聚合证据移出（阶段三）", () => {
  beforeEach(() => vi.clearAllMocks());

  it("prompt 类通用 evalMetrics 表不再平铺已结构化的聚合 key", async () => {
    getMock.mockResolvedValue(
      baseDetail({
        evalMetrics: {
          kind: "prompt",
          five_gate_hit_delta_per_gate: { fact_risk_block: -0.1 },
          self_critique_addressed_delta_observed: 0.2,
          custom_extra_note: "保留这条",
        },
      }),
    );
    renderCard();
    await screen.findByTestId("proposal-detail");
    const metrics = screen.queryByTestId("proposal-eval-metrics");
    // 非白名单 key 仍展示
    expect(metrics).toHaveTextContent("custom_extra_note");
    // 白名单 key 不在通用表里（已被对照表结构化展示）
    expect(metrics).not.toHaveTextContent("five_gate_hit_delta_per_gate");
    expect(metrics).not.toHaveTextContent("self_critique_addressed_delta_observed");
  });

  it("threshold 类通用 evalMetrics 表保留全部 key（含共有的 five_gate_hit_delta_per_gate）", async () => {
    getMock.mockResolvedValue(
      baseDetail({
        kind: "threshold",
        gateKey: "fact_risk_block",
        evalMetrics: {
          kind: "threshold",
          original_send_success_rate: 0.8,
          new_send_success_rate: 0.85,
          five_gate_hit_delta_per_gate: { fact_risk_block: -0.05 },
          safety_regression_rate: 0.0,
        },
      }),
    );
    renderCard();
    await screen.findByTestId("proposal-detail");
    const metrics = screen.getByTestId("proposal-eval-metrics");
    // threshold 类完全不移出：独有 + 共有 key 都在
    expect(metrics).toHaveTextContent("original_send_success_rate");
    expect(metrics).toHaveTextContent("five_gate_hit_delta_per_gate");
    expect(metrics).toHaveTextContent("safety_regression_rate");
  });
});
```

- [ ] **Step 2: 运行验证失败**

Run: `cd frontend && npm run test -- ProposalReleaseCard 2>&1 | tail -25`
Expected: 第 1 个新用例 FAIL（当前 prompt 类仍平铺 `five_gate_hit_delta_per_gate`）；第 2 个 threshold 用例可能已 PASS（当前不区分 kind，全平铺）。

- [ ] **Step 3: 改 `MetadataSection`**（`ProposalReleaseCard.tsx`）

import 处补 `PROMPT_AGG_METRIC_KEYS`：

```tsx
import {
  FIVE_GATE_KEYS,
  GATE_LABELS,
  gateHit,
  readAggregateEvidence,
  PROMPT_AGG_METRIC_KEYS,
} from "./evidenceMetrics";
```

`MetadataSection` 的 `metricEntries` 计算行（当前 :314 `const metricEntries = Object.entries(proposal.evalMetrics ?? {});`）替换为：

```tsx
  // prompt 类：移出已被新旧对照表结构化展示的聚合 key（按白名单），
  // 避免与对照表重复平铺。threshold 类完全不动（其 evalMetrics 是另一套
  // send_success/safety 字段，且与 prompt 共享 five_gate_hit_delta_per_gate
  // 等 key——绝不能按 key 名笼统过滤，只在 kind==="prompt" 时按白名单移除）。
  const allMetricEntries = Object.entries(proposal.evalMetrics ?? {});
  const metricEntries =
    proposal.kind === "prompt"
      ? allMetricEntries.filter(
          ([key]) => !(PROMPT_AGG_METRIC_KEYS as readonly string[]).includes(key),
        )
      : allMetricEntries;
```

其余 `MetadataSection` 逻辑（空值不渲染、表渲染）不变——注意空判断里用的是 `metricEntries.length === 0`，过滤后若 prompt 类聚合 key 全移出且无其它 key，则该块自动不渲染（符合预期）。

- [ ] **Step 4: 运行验证通过**

Run: `cd frontend && npm run test -- ProposalReleaseCard 2>&1 | tail -25`
Expected: 全部用例 PASS（含 Task3 的对照表 + 本任务移出/回归 + 原有 metadata 用例）。

> 回归校验：原「metadata 5 字段」用例 evalMetrics 是 `{win_rate, sample_size}`（非白名单）→ prompt 类过滤后仍保留 → `proposal-eval-metrics` 含 win_rate 断言不破。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/review/ProposalReleaseCard.tsx frontend/src/__tests__/components/review/ProposalReleaseCard.test.tsx
git commit -m "feat(review): MetadataSection prompt 类移出已结构化的聚合 key(threshold 不动)"
```

---

## Task 5: 全量验证 + 文档收口

**Files:**
- Modify: `docs/superpowers/plans/2026-06-27-prompt-evolution-human-gated.md`（阶段三状态标完成，可选）

- [ ] **Step 1: 前端构建 + 全量测试**

Run: `cd frontend && npm run build 2>&1 | tail -15`
Expected: build 成功（tsc 无类型错误 + vite 打包通过）。

Run: `cd frontend && npm run test 2>&1 | tail -20`
Expected: 全部测试绿（含新增 evidenceMetrics + ProposalReleaseCard 用例）。

- [ ] **Step 2: 后端目标单测 + check**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib shadow_replay_json -- 2>&1 | tail -10`
Expected: Task1 两测试 PASS。

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo check 2>&1 | tail -10`
Expected: 无编译错误。
（lib 全量基线交由 CI——本地共享 target 会被主 worktree 串产物污染。）

- [ ] **Step 3: 双 lint**

Run: `bash scripts/check-no-human-takeover.sh 2>&1; echo "EXIT=$?"`
Expected: `EXIT=0`（前端新增行无禁用词）。

Run: `bash scripts/check-evolution-isolation.sh 2>&1; echo "EXIT=$?"`
Expected: `EXIT=0`（本阶段未碰 src/evolution/ 的发送符号——实际只改了 routes/evolution.rs 的 json，确认隔离不破）。

- [ ] **Step 4: 收口 commit（如有文档更新）**

```bash
git add docs/superpowers/plans/2026-06-27-prompt-evolution-human-gated.md
git commit -m "docs(evolution): 阶段三证据透出落地完成"
```

---

## Self-Review

**Spec coverage（逐条对 spec §4-§8）：**
- §4.A 后端 shadow_replay_json 补 2 字段 → Task 1 ✓
- §4.B 前端 ShadowReplaySample 补字段 + snake_case helper → Task 2 ✓
- §4.C ShadowEvalReport prompt 对照表 + MetadataSection 移出 → Task 3（渲染）+ Task 4（移出）✓
- §4.D 五闸 key 固定序 + 标签 → Task 2（FIVE_GATE_KEYS/GATE_LABELS）✓
- §5 错误处理（release 三态现有覆盖、空数据占位、前 5 条上限、类型安全窄化）→ Task 2 helper 窄化 + Task 3 占位渲染 ✓（三态链路不碰，spec 已确认）
- §6 测试（后端序列化、前端 prompt 对照/threshold 回归/空值）→ Task 1 + Task 3 + Task 4 ✓
- §7 验收（build/test/lib 不回归/lint/设计系统）→ Task 5 ✓
- §8 YAGNI（无图表/无分页/不动 threshold 渲染/不动 release 后端/无新依赖）→ 计划无越界项 ✓

**Placeholder 扫描：** 每个 code step 都有完整可粘贴代码；命令含预期输出；无 TBD/TODO。

**Type consistency：** `original5gateHit`/`originalSelfCritiqueAddressed`（Task1 后端 camelCase JSON ↔ Task2 前端类型）一致；`readAggregateEvidence`/`gateHit`/`FIVE_GATE_KEYS`/`GATE_LABELS`/`PROMPT_AGG_METRIC_KEYS`（Task2 定义 ↔ Task3/4 消费）签名一致；`AggregateEvidence` 字段（gateDeltas/originalCritiqueRate/newCritiqueRate/critiqueDelta/tokenDelta）Task2 定义 ↔ Task3 渲染一致。

**已知取舍（非缺陷）：** 聚合表 per-gate 只展示 Δ（聚合 doc 无 per-gate 原始/新命中率，仅 delta），原始/新命中率的逐样本明细在样本对照表的「原五闸/新五闸」点阵呈现——两表互补，符合 spec「表格对照」决策。
