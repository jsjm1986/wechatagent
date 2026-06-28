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
