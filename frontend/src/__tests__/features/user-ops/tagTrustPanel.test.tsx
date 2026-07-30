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
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(baseContact, ["VIP", "老客户"]));
  });

  it("F11: AI 确信标签显示 strong_evidence 来源徽标", () => {
    const c = {
      id: "c2", manualTags: [], bayesianSignals: [],
      confirmedTags: [{ value: "VIP", evidences: [{turn:1,msgId:"m1"}], confirmedAt:"", confirmedBy:"strong_evidence" }],
    } as any;
    render(<TagTrustPanel contact={c} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText("强证据")).toBeInTheDocument();
  });

  it("F11: consolidation 来源显示压缩重判徽标", () => {
    const c = {
      id: "c3", manualTags: [], bayesianSignals: [],
      confirmedTags: [{ value: "价格敏感", evidences: [{turn:0,msgId:"x"}], confirmedAt:"", confirmedBy:"consolidation" }],
    } as any;
    render(<TagTrustPanel contact={c} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText("压缩重判")).toBeInTheDocument();
  });

  it("F11: 未知/空 confirmedBy 不崩且不显徽标", () => {
    const c = {
      id: "c4", manualTags: [], bayesianSignals: [],
      confirmedTags: [{ value: "孤标", evidences: [{turn:0,msgId:"x"}], confirmedAt:"", confirmedBy:"" }],
    } as any;
    render(<TagTrustPanel contact={c} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText("孤标")).toBeInTheDocument();
    expect(screen.queryByText("强证据")).not.toBeInTheDocument();
    expect(screen.queryByText("压缩重判")).not.toBeInTheDocument();
  });
});
