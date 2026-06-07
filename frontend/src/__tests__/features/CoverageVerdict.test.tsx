import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { CoverageVerdict } from "../../features/knowledge/cockpit/CoverageVerdict";
import { parseCompleteness } from "../../features/knowledge/trustTypes";

const view = parseCompleteness({
  answeringMode: "product_safe",
  coverage: {
    capability: { verifiedFact: true, state: "verified" },
    pricing: { pendingDraft: true, state: "draft" },
    caseEvidence: { verifiedFact: true, state: "verified" },
    effectClaims: { state: "missing" },
    deliveryBoundary: { methodologyOnly: true, state: "methodology" },
  },
});

describe("CoverageVerdict", () => {
  it("渲染 5 维,每维一行带中文名", () => {
    render(<CoverageVerdict view={view} onDrillDown={() => {}} />);
    ["能力", "定价", "案例", "效果数据", "交付边界"].forEach((n) =>
      expect(screen.getByText(n)).toBeInTheDocument()
    );
  });
  it("effectClaims=missing 显示高风险大白话(含「拦」)", () => {
    render(<CoverageVerdict view={view} onDrillDown={() => {}} />);
    expect(screen.getByText(/拦/)).toBeInTheDocument();
  });
  it("点维度行触发 onDrillDown(维度 key)", () => {
    const fn = vi.fn();
    render(<CoverageVerdict view={view} onDrillDown={fn} />);
    screen.getByText("定价").click();
    expect(fn).toHaveBeenCalledWith("pricing");
  });
});
