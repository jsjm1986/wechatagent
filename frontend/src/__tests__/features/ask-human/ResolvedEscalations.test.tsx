import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";

// 只读历史视图自取数：直接 mock api.get（与 pending 收件箱的 inboxStore 链路正交，互不影响）。
vi.mock("../../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../../lib/api")>(
    "../../../lib/api",
  );
  return { ...actual, api: { ...actual.api, get: vi.fn() } };
});

import { api } from "../../../lib/api";
import { ResolvedEscalations } from "../../../features/ask-human/ResolvedEscalations";
import AskHumanFeature from "../../../features/ask-human/index";
import { useInboxStore } from "../../../stores/inboxStore";

const get = api.get as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  get.mockReset();
});

describe("ResolvedEscalations 已裁决历史", () => {
  it("fetches resolved escalations and renders verdict / 授权到期 / 裁决渠道", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "E1",
          contactWxid: "wxid_a",
          category: "pricing",
          decision: { verdict: "approved", substance: "同意 8 折", constraints: ["本周内付款"] },
          authorizationExpiresAt: "2026-07-01T00:00:00Z",
          resolvedVia: "principal_chat",
        },
      ],
    });

    render(<ResolvedEscalations />);

    // 调用了 resolved 端点。
    await waitFor(() =>
      expect(get).toHaveBeenCalledWith("/api/admin/principal-escalations?status=resolved"),
    );
    // 短码、裁决结果（verdict 标签用共享字典 approved→"同意" + substance）、授权到期、裁决渠道均渲染。
    await screen.findByText("E1");
    expect(screen.getByText("同意")).toBeTruthy();
    expect(screen.getByText(/同意 8 折/)).toBeTruthy();
    expect(screen.getByText(/2026/)).toBeTruthy();
    expect(screen.getByText(/裁决渠道/)).toBeTruthy();
  });

  it("authorizationExpiresAt 为 null 时显示长期有效", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "E2",
          decision: { verdict: "rejected", substance: "暂不考虑" },
          authorizationExpiresAt: null,
          resolvedVia: "admin",
        },
      ],
    });

    render(<ResolvedEscalations />);
    await screen.findByText("E2");
    expect(screen.getByText(/长期有效/)).toBeTruthy();
  });

  it("空列表显示占位文案", async () => {
    get.mockResolvedValue({ items: [] });
    render(<ResolvedEscalations />);
    await screen.findByText(/暂无已裁决记录/);
  });
});

describe("AskHumanView 已裁决历史切换", () => {
  beforeEach(() => {
    useInboxStore.setState({
      items: [],
      errors: [],
      summary: null,
      loading: false,
      fatalError: null,
      activeSource: null,
    });
  });

  it("切到已裁决历史时调 resolved 端点并展示历史", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "EH",
          decision: { verdict: "approved", substance: "历史裁决" },
          authorizationExpiresAt: null,
          resolvedVia: "wechat",
        },
      ],
    });

    render(<AskHumanFeature />);
    // 默认在待处理视图，点击切换按钮进入已裁决历史。
    fireEvent.click(screen.getByText("已裁决历史"));
    await screen.findByText("EH");
    expect(get).toHaveBeenCalledWith("/api/admin/principal-escalations?status=resolved");
  });
});
