import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

// OutboxPanel：approved 决策发送链路真相源（agent_send_outbox）的逐条只读 + 取消入口。
// 后端 GET /api/admin/outbox + POST /api/admin/outbox/:id/cancel（带 cancelReason body）。
// cancel 端点 serde 要求非空 cancelReason，故取消请求必须带 body（与后端 admin_outbox.rs 一致）。

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({
      items: [
        {
          id: "OB1",
          status: "pending",
          content: "待发文本",
          contactWxid: "wxid_alice",
          createdAt: null,
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

  it("点击取消调用 cancel 端点并携带 cancelReason", async () => {
    render(<OutboxPanel />);
    await waitFor(() => screen.getByText("待发文本"));
    fireEvent.click(screen.getByText("取消"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/outbox/OB1/cancel",
        expect.objectContaining({ cancelReason: expect.any(String) })
      )
    );
  });
});
