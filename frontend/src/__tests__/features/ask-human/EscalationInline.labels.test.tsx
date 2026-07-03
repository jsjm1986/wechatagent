import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { EscalationInline } from "../../../features/ask-human/inline/EscalationInline";
import type { InboxItem } from "../../../lib/inboxApi";

vi.mock("../../../lib/api", () => ({ api: { post: vi.fn() } }));

const item = {
  id: "EQERR",
  source: "principal_escalation",
  title: "请示 #EQERR",
  summary: "候选回复人味较好",
  category: "high_risk_gated",
  contactWxid: "biztest_c9",
  questionForPrincipal: "该客户议题触发高风险闸门（产品说法未经核实）",
} as unknown as InboxItem;

describe("EscalationInline 翻译", () => {
  it("category 显示中文而非英文 id", () => {
    render(<EscalationInline item={item} ctx={{ busy: false, runAction: vi.fn() }} />);
    expect(screen.getByText("高风险待裁决")).toBeInTheDocument();
    expect(screen.queryByText("high_risk_gated")).toBeNull();
  });
  it("verdict 下拉项为中文标签", () => {
    render(<EscalationInline item={item} ctx={{ busy: false, runAction: vi.fn() }} />);
    expect(screen.getByRole("option", { name: "同意" })).toBeInTheDocument();
  });
});
