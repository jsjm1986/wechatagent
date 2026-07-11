import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn() } }));

// ContactPicker 未导出则测试父组件入口;若导出则直接测。实现时确保可测:
// 优先 export ContactPicker 供测试(named export,不改父用法)。
import { ContactPicker } from "../../../features/products-deals";

beforeEach(() => {
  (api.get as any).mockResolvedValue({ items: [
    { id: "c1", accountId: "102", wxid: "wxid_x", nickname: "客户甲" },
    { id: "c2", accountId: "102", wxid: "wxid_y", nickname: "客户乙" },
  ]});
});

describe("products-deals ContactPicker 换壳", () => {
  it("点按钮开弹窗,点选好友以正确 Contact 调 onSelect", async () => {
    const onSelect = vi.fn();
    render(<ContactPicker selected={null} onSelect={onSelect} />);
    fireEvent.click(await screen.findByText(/选择好友|选择联系人/));
    fireEvent.click(await screen.findByText("客户乙"));
    await waitFor(() => {
      expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: "c2", wxid: "wxid_y" }));
    });
  });
});
