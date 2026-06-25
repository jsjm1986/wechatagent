import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import PersonalityPanel from "../../../features/user-ops/PersonalityPanel";

vi.mock("../../../features/user-ops/PersonalityPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, k) => String(k) }),
}));

const profile = {
  openness: { score: 0.7, confidence: 0.4, evidenceRefs: [{ turn: 0, msgId: "x" }] },
  conscientiousness: { score: 0.5, confidence: 0.0, evidenceRefs: [] },
  extraversion: { score: 0.6, confidence: 0.3, evidenceRefs: [] },
  agreeableness: { score: 0.8, confidence: 0.5, evidenceRefs: [] },
  neuroticism: { score: 0.3, confidence: 0.2, evidenceRefs: [] },
  updatedAt: "",
  snapshots: [],
} as any;

describe("PersonalityPanel", () => {
  it("五维都渲染，低置信维度标注存疑", () => {
    render(<PersonalityPanel profile={profile} />);
    ["开放性", "尽责性", "外向性", "宜人性", "神经质"].forEach((n) =>
      expect(screen.getByText(n)).toBeInTheDocument()
    );
    // conscientiousness confidence=0 → 低置信视觉/文案
    expect(screen.getByText(/证据不足|存疑|低置信/)).toBeInTheDocument();
  });

  it("无 profile 显示空态", () => {
    render(<PersonalityPanel profile={undefined} />);
    expect(screen.getByText(/暂无人格分析/)).toBeInTheDocument();
  });

  it("snapshots>=2 画演化折线，否则提示", () => {
    const { container: c1 } = render(
      <PersonalityPanel
        profile={{
          ...profile,
          snapshots: [
            { consolidatedAt: "a", scores: [0.5, 0.5, 0.5, 0.5, 0.5], confidences: [0.5, 0.5, 0.5, 0.5, 0.5] },
            { consolidatedAt: "b", scores: [0.7, 0.5, 0.6, 0.8, 0.3], confidences: [0.5, 0.5, 0.5, 0.5, 0.5] },
          ],
        }}
      />
    );
    // 五维五条线
    expect(c1.querySelectorAll("polyline").length).toBe(5);

    render(<PersonalityPanel profile={profile} />);
    expect(screen.getByText(/演化需多次归并后呈现/)).toBeInTheDocument();
  });
});
