import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import TagTrustPanel from "../../../features/user-ops/TagTrustPanel";

vi.mock("../../../features/user-ops/TagTrustPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, k) => String(k) }),
}));

const baseContact = {
  id: "c1", manualTags: ["VIP"], confirmedTags: [{ value: "价格敏感", evidences: [{turn:0,msgId:"x"}], confirmedAt:"", confirmedBy:"consolidation" }],
  bayesianSignals: [],
} as any;

describe("TagTrustPanel", () => {
  it("三层分区都渲染，人工层与 AI 层来源可区分", () => {
    render(<TagTrustPanel contact={baseContact} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText("VIP")).toBeInTheDocument();
    expect(screen.getByText("价格敏感")).toBeInTheDocument();
    expect(screen.getByText(/运营录入/)).toBeInTheDocument();
    expect(screen.getByText(/AI 判断/)).toBeInTheDocument();
  });

  it("AI 确信标签显示证据条数", () => {
    render(<TagTrustPanel contact={baseContact} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText(/1 条证据/)).toBeInTheDocument();
  });

  it("编辑人工标签保存调 onSaveManualTags", async () => {
    const onSave = vi.fn();
    render(<TagTrustPanel contact={baseContact} onSaveManualTags={onSave} />);
    fireEvent.click(screen.getByText("编辑"));
    fireEvent.change(screen.getByPlaceholderText(/逗号分隔/), { target: { value: "VIP, 老客户" } });
    fireEvent.click(screen.getByText("保存"));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["VIP", "老客户"]));
  });
});
