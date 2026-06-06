import { describe, it, expect } from "vitest";
import { parseCompleteness, parseIntegrityReport, type CompletenessView, type CoverageDimension } from "../../features/knowledge/trustTypes";

describe("parseCompleteness", () => {
  it("解析后端真实响应的 answeringMode + 5 维 coverage", () => {
    const raw = {
      totalChunks: 40, verifiedChunks: 12, anchoredChunks: 10,
      evidenceChunks: 3, needsReviewChunks: 2,
      answeringMode: "product_safe", summary: "可安全讲产品",
      coverage: {
        capability: { verifiedFact: true, methodologyOnly: false, pendingDraft: false, state: "verified" },
        pricing: { verifiedFact: false, methodologyOnly: false, pendingDraft: true, state: "draft" },
        caseEvidence: { verifiedFact: true, methodologyOnly: false, pendingDraft: false, state: "verified" },
        effectClaims: { verifiedFact: false, methodologyOnly: false, pendingDraft: false, state: "missing" },
        deliveryBoundary: { verifiedFact: false, methodologyOnly: true, pendingDraft: false, state: "methodology" },
      },
      gaps: ["效果数据维度无任何已验证知识"],
    };
    const v: CompletenessView = parseCompleteness(raw);
    expect(v.answeringMode).toBe("product_safe");
    expect(v.coverage.pricing.pendingDraft).toBe(true);
    expect(v.coverage.effectClaims.state).toBe("missing");
    expect(v.gaps).toHaveLength(1);
    expect(v.needsReviewChunks).toBe(2);
  });

  it("缺字段时降级为安全默认(空 coverage / relationship_only)", () => {
    const v = parseCompleteness({});
    expect(v.answeringMode).toBe("relationship_only");
    expect(v.coverage.capability.state).toBe("missing");
    expect(v.gaps).toEqual([]);
  });

  it("dimensionList 按固定顺序返回 5 维带中文名", () => {
    const v = parseCompleteness({});
    const dims: CoverageDimension[] = v.dimensionList;
    expect(dims.map((d) => d.key)).toEqual([
      "capability", "pricing", "caseEvidence", "effectClaims", "deliveryBoundary",
    ]);
    expect(dims[0].label).toBe("能力");
  });
});

describe("parseIntegrityReport", () => {
  it("读后端 item.{total,verified,needsReview,rejected}", () => {
    const v = parseIntegrityReport({ item: { total: 40, verified: 12, needsReview: 2, rejected: 1 } });
    expect(v.total).toBe(40);
    expect(v.verified).toBe(12);
    expect(v.needsReview).toBe(2);
    expect(v.rejected).toBe(1);
  });
  it("缺 item 时全 0", () => {
    const v = parseIntegrityReport({});
    expect(v.total).toBe(0);
    expect(v.verified).toBe(0);
  });
});
