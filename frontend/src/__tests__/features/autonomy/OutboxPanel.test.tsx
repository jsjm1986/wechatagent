import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";

// OutboxPanel：approved 决策发送链路真相源（agent_send_outbox）的逐条只读 + 取消入口。
// 后端 GET /api/admin/outbox + POST /api/admin/outbox/:id/cancel（带 cancelReason body）。
// cancel 端点 serde 要求非空 cancelReason，故取消请求必须带 body（与后端 admin_outbox.rs 一致）。

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({
      items: [
        {
          id: "OB1",
          accountId: "acc-1",
          status: "pending",
          content: "待发文本",
          payload: { kind: "text", text: "待发文本" },
          contactWxid: "wxid_alice",
          createdAt: null,
          cancelRequested: false,
          cancelRequestedAt: null,
          sendStartedAt: null,
          reclaimedInFlight: false,
          reclaimCount: 0,
        },
      ],
    }),
    post: vi.fn().mockResolvedValue({ item: {} }),
  },
}));
vi.mock("../../../stores/accountStore", () => ({
  useAccountStore: (sel?: (s: unknown) => unknown) => {
    const st = { currentAccountId: () => "acc-1" };
    return typeof sel === "function" ? sel(st) : st;
  },
}));

import { api } from "../../../lib/api";
import { OutboxPanel } from "../../../features/autonomy/OutboxPanel";

describe("OutboxPanel", () => {
  beforeEach(() => vi.clearAllMocks());

  it("拉取并渲染 outbox 逐条记录", async () => {
    render(<OutboxPanel />);
    await waitFor(() =>
      expect(screen.getByText("待发文本")).toBeInTheDocument()
    );
    expect(api.get).toHaveBeenCalledWith(
      expect.stringContaining("/api/admin/outbox")
    );
    // accountId 应进入查询串
    expect(api.get).toHaveBeenCalledWith(expect.stringContaining("acc-1"));
  });

  it("取消前展示账号、客户和目标，确认后才调用 cancel 端点", async () => {
    render(<OutboxPanel />);
    await waitFor(() => screen.getByText("待发文本"));
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(await screen.findByText("确认取消这条发送？")).toBeInTheDocument();
    expect(screen.getByText("业务号：acc-1")).toBeInTheDocument();
    expect(screen.getByText("客户：wxid_alice")).toBeInTheDocument();
    expect(screen.getByText("发送对象：待发文本")).toBeInTheDocument();
    expect(api.post).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "确认取消" }));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/outbox/OB1/cancel",
        expect.objectContaining({ cancelReason: expect.any(String) })
      )
    );
  });

  it("明确显示取消请求中与送达待核验，且不再提供重复取消", async () => {
    vi.mocked(api.get).mockResolvedValueOnce({
      items: [
        {
          id: "OB2",
          accountId: "acc-1",
          status: "in_flight",
          content: "边界中的文本",
          payload: { kind: "text", text: "边界中的文本" },
          contactWxid: "wxid_bob",
          createdAt: null,
          cancelRequested: true,
          cancelRequestedAt: "2026-01-01T00:00:00Z",
          sendStartedAt: null,
          reclaimedInFlight: false,
          reclaimCount: 0,
        },
        {
          id: "OB3",
          accountId: "acc-1",
          status: "delivery_unknown",
          content: "待人工核验文本",
          payload: { kind: "text", text: "待人工核验文本" },
          contactWxid: "wxid_cara",
          createdAt: null,
          cancelRequested: false,
          cancelRequestedAt: null,
          sendStartedAt: "2026-01-01T00:00:00Z",
          reclaimedInFlight: true,
          reclaimCount: 1,
        },
      ],
    });
    render(<OutboxPanel />);
    expect(await screen.findByText("取消请求中（等待发送结果）")).toBeInTheDocument();
    expect(screen.getByText("送达待核验")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "取消" })).not.toBeInTheDocument();
  });

  it("同一客户的两份素材分别显示身份，取消确认只引用选中的目标", async () => {
    vi.mocked(api.get).mockResolvedValueOnce({
      items: [
        {
          id: "OB-MEDIA-A",
          accountId: "acc-1",
          status: "pending",
          content: "",
          payload: {
            kind: "media",
            assetId: "asset-a",
            title: "A 版报价单",
            fileName: "a.pdf",
          },
          contactWxid: "wxid_same",
          createdAt: null,
          cancelRequested: false,
          cancelRequestedAt: null,
          sendStartedAt: null,
          reclaimedInFlight: false,
          reclaimCount: 0,
        },
        {
          id: "OB-MEDIA-B",
          accountId: "acc-1",
          status: "pending",
          content: "",
          payload: {
            kind: "media",
            assetId: "asset-b",
            title: "B 版方案书",
            fileName: "b.pdf",
          },
          contactWxid: "wxid_same",
          createdAt: null,
          cancelRequested: false,
          cancelRequestedAt: null,
          sendStartedAt: null,
          reclaimedInFlight: true,
          reclaimCount: 2,
        },
      ],
    });
    render(<OutboxPanel />);
    expect(await screen.findByText("素材 · A 版报价单")).toBeInTheDocument();
    expect(screen.getByText("素材 · B 版方案书")).toBeInTheDocument();
    expect(screen.getByText("素材 ID：asset-a")).toBeInTheDocument();
    expect(screen.getByText("素材 ID：asset-b")).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: "取消" })[1]);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("发送对象：素材 · B 版方案书")).toBeInTheDocument();
    expect(within(dialog).getByText("素材 ID：asset-b")).toBeInTheDocument();
    expect(within(dialog).queryByText("发送对象：素材 · A 版报价单")).not.toBeInTheDocument();
    expect(within(dialog).getByText(/曾从发送中恢复 2 次，远端可能已经收到/)).toBeInTheDocument();
    expect(within(dialog).getByText(/取消不能撤回已送达内容/)).toBeInTheDocument();
    expect(api.post).not.toHaveBeenCalled();
  });
});
