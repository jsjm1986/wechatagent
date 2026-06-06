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
  it("fully_supported 显示「完全支撑」", () => {
    render(<AnsweringModeGauge mode="fully_supported" needsReviewChunks={0} summary="" />);
    expect(screen.getByText(/完全支撑/)).toBeInTheDocument();
  });
});
