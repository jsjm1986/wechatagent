import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../../../lib/api";
import {
  SuspectedDealReviewCard,
  yuanToCents,
} from "../../../../features/ask-human/inline/SuspectedDealReviewCard";

vi.mock("../../../../lib/api", () => ({
  api: { post: vi.fn().mockResolvedValue({}) },
}));

const props = {
  signalId: "deal/sig-1",
  contactId: "contact-1",
  evidence: "客户明确表示准备付款",
  confidence: 86,
  occurrences: 2,
};

describe("SuspectedDealReviewCard", () => {
  beforeEach(() => vi.clearAllMocks());

  it("按元录入金额，提交时换算为分并规范化币种", async () => {
    const onDone = vi.fn();
    const post = vi.spyOn(api, "post").mockResolvedValue({} as never);
    render(<SuspectedDealReviewCard {...props} onDone={onDone} />);

    fireEvent.change(screen.getByLabelText("成交金额（元，可选）"), {
      target: { value: "12.34" },
    });
    fireEvent.change(screen.getByLabelText("币种"), { target: { value: "usd" } });
    fireEvent.click(screen.getByRole("button", { name: "确认成交" }));

    await waitFor(() =>
      expect(post).toHaveBeenCalledWith(
        "/api/admin/suspected-deals/deal%2Fsig-1/approve",
        { amount: 1234, currency: "USD" },
      ),
    );
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("非法金额在客户端拦截，不发送审批请求", () => {
    const post = vi.spyOn(api, "post");
    render(<SuspectedDealReviewCard {...props} onDone={() => {}} />);

    fireEvent.change(screen.getByLabelText("成交金额（元，可选）"), {
      target: { value: "-1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "确认成交" }));

    expect(screen.getByText(/金额必须/)).toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();
    expect(yuanToCents("1.10")).toBe(110);
    expect(yuanToCents(" ")).toBeNull();
  });

  it("驳回原因必填，填写后调用 reject 并刷新", async () => {
    const onDone = vi.fn();
    const post = vi.spyOn(api, "post").mockResolvedValue({} as never);
    render(<SuspectedDealReviewCard {...props} onDone={onDone} />);

    fireEvent.click(screen.getByRole("button", { name: "驳回线索" }));
    fireEvent.click(screen.getByRole("button", { name: "提交驳回" }));
    expect(screen.getByText("驳回原因不能为空")).toBeInTheDocument();
    expect(post).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("驳回原因"), {
      target: { value: "  只是咨询，尚未成交  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "提交驳回" }));

    await waitFor(() =>
      expect(post).toHaveBeenCalledWith(
        "/api/admin/suspected-deals/deal%2Fsig-1/reject",
        { reason: "只是咨询，尚未成交" },
      ),
    );
    expect(onDone).toHaveBeenCalledTimes(1);
  });
});
