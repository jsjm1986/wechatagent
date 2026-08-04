import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { EscalationInline } from "../../../../features/ask-human/inline/EscalationInline";
import { api } from "../../../../lib/api";

vi.mock("../../../../lib/api", () => ({ api: { post: vi.fn().mockResolvedValue({ ok: true }) } }));

const item = {
  source: "principal_escalation",
  id: "ESC1",
  title: "请示 #ESC1",
  summary: "客户要折扣",
  severity: "high" as const,
  createdAt: null,
  ageHours: 2,
  actionKind: "inline" as const,
};

describe("EscalationInline", () => {
  it("resolve posts verdict+substance to the short_code resolve endpoint", async () => {
    const runAction = vi.fn(async (fn: () => Promise<unknown>) => {
      await fn();
    });
    render(<EscalationInline item={item} ctx={{ busy: false, runAction }} />);
    fireEvent.change(screen.getByLabelText(/裁决类型/), { target: { value: "approved" } });
    fireEvent.change(screen.getByPlaceholderText(/裁决意见/), { target: { value: "可以给8折" } });
    fireEvent.click(screen.getByText("提交裁决"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/principal-escalations/ESC1/resolve",
        expect.objectContaining({ verdict: "approved", substance: "可以给8折" }),
      ),
    );
  });

  it("选 conditional 时显示授权窗输入,提交 body 含 constraints+authorizationWindowHours", async () => {
    const runAction = vi.fn(async (fn: () => Promise<unknown>) => {
      await fn();
    });
    render(<EscalationInline item={item} ctx={{ busy: false, runAction }} />);
    // 选择 conditional 裁决
    fireEvent.change(screen.getByLabelText(/裁决类型/), { target: { value: "conditional" } });
    // 授权窗输入出现
    const win = screen.getByLabelText(/本次转述有效期/);
    fireEvent.change(win, { target: { value: "48" } });
    fireEvent.change(screen.getByPlaceholderText(/约束条款/), { target: { value: "仅限本月" } });
    fireEvent.change(screen.getByLabelText(/后续产品豁免范围/), { target: { value: "customer_only" } });
    fireEvent.change(screen.getByPlaceholderText(/裁决意见/), { target: { value: "有条件同意" } });
    fireEvent.click(screen.getByText("提交裁决"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/principal-escalations/ESC1/resolve",
        expect.objectContaining({
          verdict: "conditional",
          authorizationWindowHours: 48,
          exemptionType: "customer_only",
          constraints: ["仅限本月"],
        }),
      ),
    );
  });

  it("富展示客户/问题/类别", () => {
    const richItem = {
      ...item,
      contactWxid: "wxid_cust",
      questionForPrincipal: "能否给折扣",
      category: "pricing",
    };
    const runAction = vi.fn();
    render(<EscalationInline item={richItem} ctx={{ busy: false, runAction }} />);
    expect(screen.getByText(/能否给折扣/)).toBeInTheDocument();
    expect(screen.getByText(/wxid_cust/)).toBeInTheDocument();
  });

  it("改派提交 toWxid 到 reassign 端点", async () => {
    const runAction = vi.fn(async (fn: () => Promise<unknown>) => {
      await fn();
    });
    render(<EscalationInline item={item} ctx={{ busy: false, runAction }} />);
    fireEvent.change(screen.getByPlaceholderText(/备选决策人/), {
      target: { value: "wxid_backup" },
    });
    fireEvent.click(screen.getByText("改派"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/principal-escalations/ESC1/reassign",
        { toWxid: "wxid_backup" },
      ),
    );
  });
});
