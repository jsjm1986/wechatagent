import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { PlannerViewSection } from "../../../features/user-ops/legacy";
import { useProfileStore } from "../../../stores/profileStore";
import type { Contact } from "../../../types";

function setStore(dimensions: any[], taxonomies: any) {
  useProfileStore.setState({ dimensions, taxonomies } as any);
}

const baseContact = {
  wxid: "wx1",
  domainAttributes: { customer_stage: "first_contact", intent_level: "high" },
  domainAttributesUpdatedAt: "2026-06-26T00:00:00Z",
  commitments: [],
} as unknown as Contact;

describe("PlannerViewSection 多维度看板(A4)", () => {
  beforeEach(() => setStore([], {}));

  it("渲染 dimensions 中非 customer_stage 维度,经 labelFor 翻译为中文", () => {
    setStore(
      [
        { kind: "customer_stage", displayName: "客户阶段", participatesInDecision: true },
        { kind: "intent_level", displayName: "意向程度", participatesInDecision: true },
      ],
      {
        customer_stage: [{ id: "first_contact", label: "首次接触" }],
        intent_level: [{ id: "high", label: "高意向" }],
      },
    );
    render(<PlannerViewSection contact={baseContact} />);
    expect(screen.getByText("意向程度")).toBeInTheDocument();
    expect(screen.getByText("高意向")).toBeInTheDocument();
  });

  it("字典缺失的维度走 no_dict 灰显原始值(不显示错误销售标签)", () => {
    setStore(
      [{ kind: "emotion_state", displayName: "情绪状态", participatesInDecision: true }],
      {},
    );
    const c = { ...baseContact, domainAttributes: { emotion_state: "anxious" } } as unknown as Contact;
    render(<PlannerViewSection contact={c} />);
    expect(screen.getByText("anxious")).toBeInTheDocument();
  });

  it("维度无值时跳过不渲染", () => {
    setStore(
      [{ kind: "value_tier", displayName: "价值分层", participatesInDecision: true }],
      { value_tier: [{ id: "vip", label: "VIP" }] },
    );
    const c = { ...baseContact, domainAttributes: {} } as unknown as Contact;
    render(<PlannerViewSection contact={c} />);
    expect(screen.queryByText("价值分层")).not.toBeInTheDocument();
  });
});
