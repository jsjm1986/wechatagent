import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProductClaimMarkersTab } from "../../../features/quality";
import { api } from "../../../lib/api";

// Task 8（路径B / 质量频道独立内联 save()）：识别 needs_human_confirm(200 body) + Reject(catch)，
// 修掉「200 needs_human_confirm 被当成功」的 bug。
vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn(),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

const validJson = JSON.stringify({ markers: [], whitelistPhrases: [], whitelistWindowChars: 20 });

const activeTemplate = {
  id: "tpl-9",
  promptKey: "user.review.product_claim_markers",
  status: "active",
  version: 3,
  content: validJson,
};

describe("ProductClaimMarkersTab 路径B 二次确认", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue({ items: [activeTemplate] });
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValue({});
  });

  it("needs_human_confirm(200) → 不发布、显确认区含 diff；确认后带 force:true 重提", async () => {
    (api.put as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce({
        status: "needs_human_confirm",
        reason: "审查服务暂不可用",
        diff: "+引导客户私下找老师",
      })
      .mockResolvedValueOnce({ ok: true });

    render(<ProductClaimMarkersTab />);
    fireEvent.click(await screen.findByText("保存并发布"));

    // 确认区出现，含 diff + reason；且第一次未发布（publish 未被调用）
    expect(await screen.findByText("+引导客户私下找老师")).toBeInTheDocument();
    expect(screen.getByText(/审查服务暂不可用/)).toBeInTheDocument();
    expect(api.post).not.toHaveBeenCalled();

    fireEvent.change(screen.getByPlaceholderText("已核对"), { target: { value: "已核对" } });
    fireEvent.click(screen.getByRole("button", { name: /强制保存|确认/ }));

    await waitFor(() => {
      const calls = (api.put as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls.length).toBe(2);
      expect(calls[1][1]).toMatchObject({ force: true });
    });
    // 确认覆盖后才发布
    await waitFor(() => expect(api.post).toHaveBeenCalled());
  });

  it("Reject(4xx) → 显拒绝理由 + 强制保存入口；点后带 force:true", async () => {
    (api.put as ReturnType<typeof vi.fn>)
      .mockRejectedValueOnce(
        new Error("红线语义审查拒绝：变相引入真人转介（确认无误可带 force 覆盖）")
      )
      .mockResolvedValueOnce({ ok: true });

    render(<ProductClaimMarkersTab />);
    fireEvent.click(await screen.findByText("保存并发布"));

    expect(await screen.findByText(/红线语义审查拒绝/)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("已核对"), { target: { value: "已核对" } });
    fireEvent.click(screen.getByRole("button", { name: /强制保存|确认/ }));

    await waitFor(() => {
      const calls = (api.put as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls.length).toBe(2);
      expect(calls[1][1]).toMatchObject({ force: true });
    });
  });
});
