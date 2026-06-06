import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
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
});
