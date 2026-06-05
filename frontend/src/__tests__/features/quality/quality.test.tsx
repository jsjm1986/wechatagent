import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import QualityFeature, { OutcomeMetricsTab } from "../../../features/quality";

// quality（运营成效中心）频道一体化迁移后的视觉/集成测试（追加，不改既有套件）。
// 验证：(1) 自包含 QualityFeature 渲染面板小标题 + 四个 Tab；
//       (2) outcome Tab 经 accountStore 选中账号驱动取数，null 指标渲染为 "—"。

vi.mock("../../../stores/accountStore", () => ({
  useAccountStore: (selector: (s: unknown) => unknown) => {
    const state = {
      accounts: [{ accountId: "acc-1", nickname: "测试号" }],
      selectedAccountId: "acc-1",
      currentAccountId: () => "acc-1",
    };
    return typeof selector === "function" ? selector(state) : state;
  },
}));

const realFetch = globalThis.fetch;

function installFetch(items: unknown[]) {
  globalThis.fetch = vi.fn(async () => {
    const body = { items };
    return {
      ok: true,
      status: 200,
      async json() {
        return body;
      },
      async text() {
        return JSON.stringify(body);
      },
    } as unknown as Response;
  }) as typeof fetch;
}

describe("QualityFeature — 一体化频道（新视觉壳）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("渲染面板小标题与四个 Tab", () => {
    installFetch([]);
    render(<QualityFeature />);

    expect(screen.getByText("运营成效中心")).toBeInTheDocument();
    expect(screen.getByText("长期指标")).toBeInTheDocument();
    expect(screen.getByText("知识自动校验")).toBeInTheDocument();
    expect(screen.getByText("公式遵守度")).toBeInTheDocument();
    expect(screen.getByText("产品声明标记词")).toBeInTheDocument();
  });

  it("outcome Tab：null 指标渲染为 '—'，数值正常显示", async () => {
    installFetch([
      {
        id: "m-1",
        accountId: "acc-1",
        horizon: "7d",
        date: "2026-06-01",
        replyRate: 0.42,
        conversationDepth: null,
        aiHoldClearedRate: null,
        agentBlockRate: 0.1,
        dailyRunCount: 12,
        dailyRunTokenTotal: 3456,
      },
    ]);
    render(<OutcomeMetricsTab accountId="acc-1" />);

    await waitFor(() => {
      expect(screen.getByText("42.0%")).toBeInTheDocument();
    });
    // conversationDepth + aiHoldClearedRate 均为 null → 两处 "—"
    expect(screen.getAllByText("—").length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("10.0%")).toBeInTheDocument();
    expect(screen.getByText("2026-06-01")).toBeInTheDocument();
  });
});
