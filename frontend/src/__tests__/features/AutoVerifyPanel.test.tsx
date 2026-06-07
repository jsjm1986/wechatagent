import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { AutoVerifyPanel } from "../../features/knowledge/cockpit/AutoVerifyPanel";

describe("AutoVerifyPanel", () => {
  it("默认显示三档松紧(适中选中)+ 留复查开关", () => {
    render(<AutoVerifyPanel />);
    expect(screen.getByText("适中")).toBeInTheDocument();
    expect(screen.getByText(/留.*复查/)).toBeInTheDocument();
  });
  it("点开始筛 → 调 auto-verify,结果分三堆显示", async () => {
    globalThis.fetch = vi.fn(() => Promise.resolve({ ok: true, json: () => Promise.resolve({
      processed: 50, verified: 31, needsReview: 14, rejected: 0, needsHumanAudit: 5,
    }) } as Response)) as unknown as typeof fetch;
    render(<AutoVerifyPanel />);
    fireEvent.click(screen.getByRole("button", { name: /开始筛/ }));
    await waitFor(() => expect(screen.getByText("31")).toBeInTheDocument());
    expect(screen.getByText("5")).toBeInTheDocument();   // 留复查
    expect(screen.getByText("14")).toBeInTheDocument();  // 没把握
  });
});
