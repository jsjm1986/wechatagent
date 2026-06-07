import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { AnsweringModeGauge } from "../../features/knowledge/cockpit/AnsweringModeGauge";

describe("AnsweringModeGauge", () => {
  it("product_safe 显示「可安全讲产品」+ 2/3 档", () => {
    render(<AnsweringModeGauge mode="product_safe" needsReviewChunks={2} summary="" />);
    expect(screen.getByText(/可安全讲产品/)).toBeInTheDocument();
    expect(screen.getByText(/2\s*\/\s*3/)).toBeInTheDocument();
  });
  it("有待审草稿时解读「为什么没到完全支撑」", () => {
    render(<AnsweringModeGauge mode="product_safe" needsReviewChunks={2} summary="" />);
    expect(screen.getByText(/待审/)).toBeInTheDocument();
  });
  it("有草稿的解读不得承诺「审掉必达完全支撑」(业务红线:clamp 只解除封顶,升档仍看覆盖度)", () => {
    render(<AnsweringModeGauge mode="product_safe" needsReviewChunks={3} summary="" />);
    const txt = screen.getByText(/待审草稿/).textContent ?? "";
    expect(txt).not.toMatch(/审掉即解锁/);
    expect(txt).toMatch(/覆盖|才有机会|不一定/);
  });
  it("fully_supported 显示「完全支撑」", () => {
    render(<AnsweringModeGauge mode="fully_supported" needsReviewChunks={0} summary="" />);
    expect(screen.getByText(/完全支撑/)).toBeInTheDocument();
  });
});
