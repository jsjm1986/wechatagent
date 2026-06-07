import { render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { CockpitView } from "../../features/knowledge/cockpit/CockpitView";

beforeEach(() => {
  globalThis.fetch = vi.fn((url: string) => {
    if (String(url).includes("/completeness")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({
        answeringMode: "product_safe", needsReviewChunks: 12,
        coverage: { effectClaims: { state: "missing" } },
      }) } as Response);
    }
    if (String(url).includes("/integrity-report")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ item: { total: 40, needsReview: 12, rejected: 3 } }) } as Response);
    }
    return Promise.resolve({ ok: true, json: () => Promise.resolve({}) } as Response);
  }) as unknown as typeof fetch;
});

describe("CockpitView", () => {
  it("加载后显示 answeringMode 仪表 + 5 维裁决 + 待办计数", async () => {
    render(<CockpitView onOpenReview={() => {}} onOpenAutoVerify={() => {}} />);
    await waitFor(() => expect(screen.getByText(/可安全讲产品/)).toBeInTheDocument());
    expect(screen.getByText("效果数据")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument(); // 待审草稿计数
  });
});
