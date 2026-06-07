import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { ReviewChat } from "../../features/knowledge/cockpit/ReviewChat";

const chunk = {
  id: "c1", title: "企业版年费 12800", summary: "含 5 个坐席",
  sourceQuote: "企业版一年 12800", sourceAnchors: [{ startLine: 1 }],
  integrityStatus: "needs_review", status: "draft",
};

describe("ReviewChat", () => {
  it("左栏裁决:双检查全过 → 显示「可以生效/让 AI 用」+ 生效键可用", () => {
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    expect(screen.getByText(/可以生效|让 AI 可以用/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /让 AI 可以用这条/ })).toBeEnabled();
  });
  it("缺锚点 → 生效键禁用 + 大白话说明", () => {
    render(<ReviewChat chunk={{ ...chunk, sourceAnchors: [] } as never} onResolved={() => {}} />);
    expect(screen.getByRole("button", { name: /让 AI 可以用这条/ })).toBeDisabled();
    expect(screen.getByText(/这话.*哪来|来源|出处/)).toBeInTheDocument();
  });
  it("右栏对话标题写明「只动这条 · 改完仍由你放行」", () => {
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    expect(screen.getByText(/只动这条/)).toBeInTheDocument();
  });
  it("显形富字段:用量 / 降级痕迹 / 字段锁(大白话)", () => {
    const rich = {
      id: "c2", title: "测试", summary: "x",
      sourceQuote: "q", sourceAnchors: [{ startLine: 1 }],
      integrityStatus: "needs_review", status: "draft",
      usageStats: { hitCount30d: 8, blockedCount30d: 2 },
      distortionRisks: ["提交为 verified 但缺锚点,已降级"],
      lockedFields: ["sourceQuote"],
    };
    render(<ReviewChat chunk={rich as never} onResolved={() => {}} />);
    expect(screen.getByText(/8 次|用了 8|被用过 8/)).toBeInTheDocument();
    expect(screen.getByText(/降级|为什么被打回|打回/)).toBeInTheDocument();
  });
  it("点「退回」→ 调 reject 端点,成功后关面板(onResolved)", async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve({ ok: true, json: () => Promise.resolve({}) } as Response);
    }) as unknown as typeof fetch;
    const onResolved = vi.fn();
    render(<ReviewChat chunk={chunk as never} onResolved={onResolved} />);
    await userEvent.click(screen.getByRole("button", { name: /退回/ }));
    await waitFor(() => expect(onResolved).toHaveBeenCalledTimes(1));
    expect(calls.some((c) => c.includes("/chunks/c1/reject"))).toBe(true);
  });
  it("退回失败 → 不关面板,显示大白话错误", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({ ok: false, status: 500, json: () => Promise.resolve({}) } as Response)
    ) as unknown as typeof fetch;
    const onResolved = vi.fn();
    render(<ReviewChat chunk={chunk as never} onResolved={onResolved} />);
    await userEvent.click(screen.getByRole("button", { name: /退回/ }));
    await waitFor(() => expect(screen.getByText(/退回没成功/)).toBeInTheDocument());
    expect(onResolved).not.toHaveBeenCalled();
  });
});
