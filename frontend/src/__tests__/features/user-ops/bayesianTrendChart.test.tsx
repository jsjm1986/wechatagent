import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import BayesianTrendChart from "../../../features/user-ops/BayesianTrendChart";

vi.mock("../../../features/user-ops/BayesianTrendChart.module.css", () => ({
  default: new Proxy({}, { get: (_t, k) => String(k) }),
}));

const signals = [
  {
    dimension: "价格敏感度",
    currentValue: "高",
    currentConfidence: 0.7,
    locked: true,
    history: [
      { turn: 1, value: "高", confidence: 0.4, valueChanged: false, confidenceChanged: true },
      { turn: 2, value: "高", confidence: 0.7, valueChanged: false, confidenceChanged: true },
    ],
  },
] as any;

describe("BayesianTrendChart", () => {
  it("每个 locked 维度渲染一条 polyline + 图例", () => {
    const { container } = render(<BayesianTrendChart signals={signals} />);
    expect(container.querySelectorAll("polyline").length).toBe(1);
    expect(screen.getByText("价格敏感度")).toBeInTheDocument();
  });

  it("未占槽(locked=false)维度不画线", () => {
    const { container } = render(
      <BayesianTrendChart signals={[{ ...signals[0], locked: false }]} />
    );
    expect(container.querySelectorAll("polyline").length).toBe(0);
  });

  it("无数据显示空态", () => {
    render(<BayesianTrendChart signals={[]} />);
    expect(screen.getByText(/暂无评估维度/)).toBeInTheDocument();
  });
});
