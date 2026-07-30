import { render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { CockpitView } from "../../features/knowledge/cockpit/CockpitView";

beforeEach(() => {
  globalThis.fetch = vi.fn((url: string) => {
    if (String(url).includes("/completeness")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({
        answeringMode: "product_safe", needsReviewChunks: 12,
        coverage: { effectClaims: { state: "missing" } },
        gaps: ["缺少报价区间", "缺少售后政策"],
      }) } as Response);
    }
    if (String(url).includes("/integrity-report")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ item: { total: 40, needsReview: 12, rejected: 3, anchorsMissing: 2 } }) } as Response);
    }
    if (String(url).includes("/gap-signals")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ signals: [{ signalId: "g1" }, { signalId: "g2" }, { signalId: "g3" }] }) } as Response);
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

  it("三计数卡口径：待审草稿=needsReview / D2 降级=anchorsMissing / 知识缺口=gap-signals pending", async () => {
    render(<CockpitView onOpenReview={() => {}} onOpenAutoVerify={() => {}} />);
    // 三张卡的标题首屏就存在；必须等待并行请求全部提交状态，不能把静态标题
    // 当成“数据已加载”的同步点，否则慢 runner 会在初始 `—` 帧上偶发失败。
    await waitFor(() => {
      const draftCard = screen.getByText("待审草稿").closest("button");
      expect(draftCard?.textContent).toContain("12"); // needsReview

      const d2Card = screen.getByText("缺原文出处").closest("button");
      expect(d2Card?.textContent).toContain("2"); // anchorsMissing
      expect(d2Card?.textContent).toContain("已启用但没填原文出处，AI 用前需补齐");

      const gapCard = screen.getByText("知识缺口").closest("button");
      expect(gapCard?.textContent).toContain("3"); // gap-signals pending 计数 = signals.length
    });
  });

  it("渲染 completeness.gaps 缺口明细列表", async () => {
    render(<CockpitView onOpenReview={() => {}} onOpenAutoVerify={() => {}} />);
    await waitFor(() => expect(screen.getByText("缺口明细")).toBeInTheDocument());
    expect(screen.getByText("缺少报价区间")).toBeInTheDocument();
    expect(screen.getByText("缺少售后政策")).toBeInTheDocument();
  });
});
