import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { ReviewChat } from "../../features/knowledge/cockpit/ReviewChat";

const chunk = {
  id: "c1", title: "企业版年费 12800", summary: "含 5 个坐席",
  sourceQuote: "企业版一年 12800", sourceAnchors: [{ startLine: 1 }],
  integrityStatus: "needs_review", status: "draft",
  updatedAt: "2026-07-27T03:00:00Z",
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
  it("对话产 patch → AI 回合下渲染 patch diff 预览(字段中文label + 新值)", async () => {
    const fetchMock = vi.fn((_input: RequestInfo | URL, _init?: RequestInit) =>
      Promise.resolve({
        ok: true,
        json: () =>
          Promise.resolve({
            sessionId: "s1",
            naturalReply: "好的,已经按你说的改好了。",
            targetChunkId: "c1",
            expectedUpdatedAt: "2026-07-27T03:00:00Z",
            draftPreview: { title: "企业版年费 19800", summary: "含 10 个坐席" },
          }),
      } as Response)
    );
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    await userEvent.type(screen.getByPlaceholderText(/让 AI 改这条/), "把年费改成 19800");
    await userEvent.click(screen.getByRole("button", { name: /发送/ }));
    // patch 预览:字段中文 label
    await waitFor(() => expect(screen.getByText(/标题/)).toBeInTheDocument());
    expect(screen.getByText(/摘要/)).toBeInTheDocument();
    // patch 预览:新值
    expect(screen.getByText(/企业版年费 19800/)).toBeInTheDocument();
    expect(screen.getByText(/含 10 个坐席/)).toBeInTheDocument();
    const request = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(JSON.parse(String(request.body)).attachments).toEqual([{
      chunkId: "c1",
      expectedUpdatedAt: "2026-07-27T03:00:00Z",
      operation: "update",
    }]);
  });
  it("patch 键为 snake_case / 未知键 → label 兜底(已知映射中文,未知显原键)", async () => {
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        json: () =>
          Promise.resolve({
            sessionId: "s2",
            naturalReply: "改好了。",
            targetChunkId: "c1",
            expectedUpdatedAt: "2026-07-27T03:00:00Z",
            draftPreview: { knowledge_type: "product_fact", customField: "abc" },
          }),
      } as Response)
    ) as unknown as typeof fetch;
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    await userEvent.type(screen.getByPlaceholderText(/让 AI 改这条/), "改类型");
    await userEvent.click(screen.getByRole("button", { name: /发送/ }));
    // snake_case 已知键映射中文
    await waitFor(() => expect(screen.getByText(/知识类型/)).toBeInTheDocument());
    // 未知键不吞,显原键名
    expect(screen.getByText(/customField/)).toBeInTheDocument();
  });
  it("响应目标或冻结版本不匹配时不展示 patch", async () => {
    globalThis.fetch = vi.fn(() => Promise.resolve({
      ok: true,
      json: () => Promise.resolve({
        sessionId: "s3",
        naturalReply: "改好了。",
        targetChunkId: "other",
        expectedUpdatedAt: "2026-07-27T03:00:00Z",
        draftPreview: { title: "不应展示" },
      }),
    } as Response)) as unknown as typeof fetch;
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    await userEvent.type(screen.getByPlaceholderText(/让 AI 改这条/), "改标题");
    await userEvent.click(screen.getByRole("button", { name: /发送/ }));
    await waitFor(() => expect(screen.getByText(/目标或版本已变化/)).toBeInTheDocument());
    expect(screen.queryByText("不应展示")).not.toBeInTheDocument();
  });
});
