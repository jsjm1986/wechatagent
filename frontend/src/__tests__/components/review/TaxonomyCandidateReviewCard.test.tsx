import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { TaxonomyCandidateReviewCard } from "../../../components/review/TaxonomyCandidateReviewCard";
import { api } from "../../../lib/api";

vi.mock("../../../components/review/TaxonomyCandidateReviewCard.module.css", () => ({
  default: new Proxy({}, { get: (_t, key) => String(key) }),
}));

vi.mock("../../../lib/api", () => ({
  api: {
    postRaw: vi.fn().mockResolvedValue({ ok: true, status: 200, data: {} }),
    post: vi.fn().mockResolvedValue({}),
  },
}));

const candidate = {
  id: "cand1",
  scope: "global",
  kind: "emotional_state",
  rawValue: "anxious",
  evidence: "客户连续两条消息表达担心",
  confidence: 7,
  occurrences: 3,
  suggestedDisplayName: "焦虑",
};

describe("TaxonomyCandidateReviewCard", () => {
  beforeEach(() => vi.clearAllMocks());

  it("渲染 rawValue、证据、命名表单预填（显示名取 suggestedDisplayName）", () => {
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={() => {}} />);
    expect(screen.getByText(/anxious/)).toBeTruthy();
    // evidence 在证据区展示 + 预填进描述 textarea 各出现一次；用「判断依据：」前缀
    // 唯一定位证据区展示，避免匹配到 textarea 的预填值（多元素歧义）。
    expect(screen.getByText(/判断依据：客户连续两条消息表达担心/)).toBeTruthy();
    const label = screen.getByLabelText(/显示名/) as HTMLInputElement;
    expect(label.value).toBe("焦虑");
    const idInput = screen.getByLabelText(/canonical id/i) as HTMLInputElement;
    expect(idInput.value).toBe("anxious");
  });

  it("采纳发 canonicalValue body 并在成功后回调 onDone", async () => {
    const onDone = vi.fn();
    const postRaw = vi.spyOn(api, "postRaw").mockResolvedValue({ ok: true, status: 200, data: {} } as never);
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    const [url, body] = postRaw.mock.calls[0] as [string, { canonicalValue: Record<string, unknown> }];
    expect(url).toContain("/api/admin/taxonomy-candidates/cand1/approve");
    expect(body.canonicalValue.id).toBe("anxious");
    expect(body.canonicalValue.label).toBe("焦虑");
    await waitFor(() => expect(onDone).toHaveBeenCalled());
  });

  it("id 或显示名清空后采纳被拦截，不发请求", async () => {
    const postRaw = vi.spyOn(api, "postRaw");
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={() => {}} />);
    fireEvent.change(screen.getByLabelText(/显示名/), { target: { value: "  " } });
    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    expect(postRaw).not.toHaveBeenCalled();
    expect(screen.getByText(/不能为空/)).toBeTruthy();
  });

  it("409 视为已存在提示，不当错误", async () => {
    vi.spyOn(api, "postRaw").mockResolvedValue({ ok: false, status: 409, data: { message: "该字典条目已存在" } } as never);
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    await waitFor(() => expect(screen.getByText(/已存在/)).toBeTruthy());
  });

  it("驳回需填原因，填后 POST reason 并回调 onDone", async () => {
    const onDone = vi.fn();
    const post = vi.spyOn(api, "post").mockResolvedValue({} as never);
    render(<TaxonomyCandidateReviewCard candidate={candidate} onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: "驳回" }));
    fireEvent.change(screen.getByLabelText(/驳回原因/), { target: { value: "无业务相关性" } });
    fireEvent.click(screen.getByRole("button", { name: "确认驳回" }));
    await waitFor(() => expect(post).toHaveBeenCalledWith(
      expect.stringContaining("/api/admin/taxonomy-candidates/cand1/reject"),
      { reason: "无业务相关性" },
    ));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
  });
});
