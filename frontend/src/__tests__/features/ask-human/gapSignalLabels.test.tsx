import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { SimpleApproveReject } from "../../../features/ask-human/inline/SimpleApproveReject";
import type { InboxItem } from "../../../lib/inboxApi";

vi.mock("../../../lib/api", () => ({ api: { post: vi.fn() } }));

const gapItem = {
  id: "g1",
  source: "gap_signal",
  title: "孤立切片：定价政策",
  summary: "该切片无任何入向引用",
  severity: "medium",
  kind: "orphan",
  signalSeverity: "warning",
} as unknown as InboxItem;

// SimpleApproveReject 现在只渲染详情（摘要/依据/类型/严重度）。处置按钮已抽成
// SimpleActionButtons 常驻 InboxRow 行内，故此组件不再收 ctx / endpoints。
describe("gap_signal kind/severity 翻译", () => {
  it("kind 显示中文而非英文 id", () => {
    render(<SimpleApproveReject item={gapItem} />);
    expect(screen.getByText(/孤立知识/)).toBeInTheDocument();
    expect(screen.queryByText(/orphan/)).toBeNull();
  });

  it("signalSeverity 显示中文而非英文 id", () => {
    render(<SimpleApproveReject item={gapItem} />);
    expect(screen.getByText(/需注意/)).toBeInTheDocument();
    expect(screen.queryByText(/warning/)).toBeNull();
  });
});
