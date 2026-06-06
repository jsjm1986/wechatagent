// 可信度治理:与后端 completeness / chunk 富字段对齐的类型 + 解析层。
// 后端真实响应见 src/routes/knowledge.rs:3367-3378(completeness)。

export type AnsweringMode = "relationship_only" | "product_safe" | "fully_supported";
export type CoverageState = "verified" | "methodology" | "draft" | "missing";

export interface CoverageFlags {
  verifiedFact: boolean;
  methodologyOnly: boolean;
  pendingDraft: boolean;
  state: CoverageState;
}

export interface CoverageDimension extends CoverageFlags {
  key: string;
  label: string; // 中文维度名
}

type DimKey = "capability" | "pricing" | "caseEvidence" | "effectClaims" | "deliveryBoundary";

export interface CompletenessView {
  totalChunks: number;
  verifiedChunks: number;
  anchoredChunks: number;
  evidenceChunks: number;
  needsReviewChunks: number;
  answeringMode: AnsweringMode;
  summary: string;
  coverage: Record<DimKey, CoverageFlags>;
  gaps: string[];
  dimensionList: CoverageDimension[];
}

const DIM_ORDER: { key: DimKey; label: string }[] = [
  { key: "capability", label: "能力" },
  { key: "pricing", label: "定价" },
  { key: "caseEvidence", label: "案例" },
  { key: "effectClaims", label: "效果数据" },
  { key: "deliveryBoundary", label: "交付边界" },
];

function flags(raw: unknown): CoverageFlags {
  const o = (raw ?? {}) as Record<string, unknown>;
  const verifiedFact = o.verifiedFact === true;
  const methodologyOnly = o.methodologyOnly === true;
  const pendingDraft = o.pendingDraft === true;
  const state: CoverageState =
    typeof o.state === "string" &&
    ["verified", "methodology", "draft", "missing"].includes(o.state)
      ? (o.state as CoverageState)
      : verifiedFact ? "verified"
      : methodologyOnly ? "methodology"
      : pendingDraft ? "draft"
      : "missing";
  return { verifiedFact, methodologyOnly, pendingDraft, state };
}

export function parseCompleteness(raw: unknown): CompletenessView {
  const o = (raw ?? {}) as Record<string, unknown>;
  const cov = (o.coverage ?? {}) as Record<string, unknown>;
  const coverage = Object.fromEntries(
    DIM_ORDER.map((d) => [d.key, flags(cov[d.key])])
  ) as Record<DimKey, CoverageFlags>;
  const mode = o.answeringMode;
  const answeringMode: AnsweringMode =
    mode === "product_safe" || mode === "fully_supported" ? mode : "relationship_only";
  return {
    totalChunks: Number(o.totalChunks ?? 0),
    verifiedChunks: Number(o.verifiedChunks ?? 0),
    anchoredChunks: Number(o.anchoredChunks ?? 0),
    evidenceChunks: Number(o.evidenceChunks ?? 0),
    needsReviewChunks: Number(o.needsReviewChunks ?? 0),
    answeringMode,
    summary: typeof o.summary === "string" ? o.summary : "",
    coverage,
    gaps: Array.isArray(o.gaps) ? o.gaps.filter((g): g is string => typeof g === "string") : [],
    dimensionList: DIM_ORDER.map((d) => ({ key: d.key, label: d.label, ...coverage[d.key] })),
  };
}
