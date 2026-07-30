import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { api } from "../../../lib/api";
import { useAccountStore } from "../../../stores/accountStore";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn(), post: vi.fn() } }));

// ContactPicker 未导出则测试父组件入口;若导出则直接测。实现时确保可测:
// 优先 export ContactPicker 供测试(named export,不改父用法)。
import { ContactPicker, DealsTab } from "../../../features/products-deals";

beforeEach(() => {
  vi.clearAllMocks();
  useAccountStore.setState({
    accounts: [
      { id: "record-a", accountId: "account-a", alias: "A", displayName: "A", online: true },
      { id: "record-b", accountId: "account-b", alias: "B", displayName: "B", online: true },
    ],
    selectedAccountId: "account-a",
  });
  (api.get as any).mockResolvedValue({ items: [
    { id: "c1", accountId: "account-a", wxid: "wxid_x", nickname: "客户甲" },
    { id: "c2", accountId: "account-a", wxid: "wxid_y", nickname: "客户乙" },
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

  it("clears the selected contact synchronously when the account changes", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url === "/api/products") return Promise.resolve({ items: [] });
      if (url.includes("accountId=account-a")) {
        return Promise.resolve({
          items: [{ id: "contact-a", accountId: "account-a", wxid: "wxid_a", nickname: "A customer" }],
        });
      }
      return Promise.resolve({ items: [] });
    });

    render(<DealsTab />);
    fireEvent.click(await screen.findByRole("button", { name: "选择好友…" }));
    fireEvent.click(await screen.findByText("A customer"));
    expect(await screen.findByText("登记成交 / 退款")).toBeInTheDocument();

    act(() => useAccountStore.setState({ selectedAccountId: "account-b" }));

    expect(screen.queryByText("登记成交 / 退款")).not.toBeInTheDocument();
    expect(api.post).not.toHaveBeenCalled();
  });
});
