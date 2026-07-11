import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ProductsDealsFeature from "../../../features/products-deals";
import { api } from "../../../lib/api";

// F23：疑似成交待核实闭环（方案B）前端。AI 产出的疑似成交弱信号沉到待核实队列，
// 运营在此核实——通过则后端落正式成交（verification=staff_confirmed），驳回仅标记。
// 测试断言：点「确认成交」POST .../:id/approve；点「驳回」POST .../:id/reject 带 reason。

const SAMPLE = {
  id: "deal-sig-1",
  contactId: "507f1f77bcf86cd799439011",
  value: "疑似成交·待核实",
  evidence: "客户说要下单",
  confidence: 75,
  occurrences: 2,
  status: "pending",
  lastSeenAt: "2026-06-26T00:00:00Z",
};

describe("ProductsDealsFeature — F23 疑似成交待核实", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  async function openReviewTab() {
    // 默认拉空（catalog/holdings 等其它 tab 的 GET），review tab 拉待核实列表。
    vi.spyOn(api, "get").mockImplementation((url: string) => {
      if (url.includes("/api/admin/suspected-deals")) {
        return Promise.resolve({ items: [SAMPLE] } as never);
      }
      return Promise.resolve({ items: [] } as never);
    });
    const user = userEvent.setup();
    render(<ProductsDealsFeature />);
    await user.click(screen.getByRole("button", { name: "疑似成交待核实" }));
    // 列表加载完成，富展示判断依据。
    await screen.findByText(/判断依据：客户说要下单/);
    return user;
  }

  it("suspected deal approve posts staff_confirmed deal", async () => {
    const postSpy = vi.spyOn(api, "post").mockResolvedValue({} as never);
    const user = await openReviewTab();

    await user.click(screen.getByRole("button", { name: "确认成交" }));

    await waitFor(() => expect(postSpy).toHaveBeenCalled());
    expect(postSpy).toHaveBeenCalledWith(
      "/api/admin/suspected-deals/deal-sig-1/approve",
      expect.any(Object),
    );
  });

  it("suspected deal reject posts reason", async () => {
    const postSpy = vi.spyOn(api, "post").mockResolvedValue({} as never);
    const user = await openReviewTab();

    // 新交互（F-017）：点「驳回」展开内嵌原因输入框（替代原生 window.prompt），
    // 填原因后点「确认驳回」才提交。
    await user.click(screen.getByRole("button", { name: "驳回" }));
    await user.type(
      screen.getByPlaceholderText("如：误判，实际只是咨询"),
      "误判，实际只是咨询",
    );
    await user.click(screen.getByRole("button", { name: "确认驳回" }));

    await waitFor(() => expect(postSpy).toHaveBeenCalled());
    expect(postSpy).toHaveBeenCalledWith(
      "/api/admin/suspected-deals/deal-sig-1/reject",
      expect.objectContaining({ reason: "误判，实际只是咨询" }),
    );
  });
});
