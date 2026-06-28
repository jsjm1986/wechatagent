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
