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

describe("gap_signal kind/severity 翻译", () => {
  it("kind 显示中文而非英文 id", () => {
    render(
      <SimpleApproveReject
        item={gapItem}
        ctx={{ busy: false, runAction: vi.fn() }}
        endpoints={{}}
      />,
    );
    expect(screen.getByText(/孤立知识/)).toBeInTheDocument();
    expect(screen.queryByText(/orphan/)).toBeNull();
  });

  it("signalSeverity 显示中文而非英文 id", () => {
    render(
      <SimpleApproveReject
        item={gapItem}
        ctx={{ busy: false, runAction: vi.fn() }}
        endpoints={{}}
      />,
    );
    expect(screen.getByText(/需注意/)).toBeInTheDocument();
    expect(screen.queryByText(/warning/)).toBeNull();
  });
});
