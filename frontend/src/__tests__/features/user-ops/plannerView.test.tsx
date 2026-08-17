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
  stagnationDimension: "customer_stage",
  stagnationValue: "first_contact",
  stagnationUpdatedAt: "2026-06-01T00:00:00Z",
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

  it("仅有额外维度(无 stage/commitments/mode)时仍渲染(守卫上提回归)", () => {
    setStore(
      [{ kind: "value_tier", displayName: "价值分层", participatesInDecision: true }],
      { value_tier: [{ id: "vip", label: "VIP" }] },
    );
    // 无 domainAttributesUpdatedAt → hasStage=false；commitments 空；无 lastConversationMode。
    // 仅 domainAttributes 携带一个非 customer_stage 的额外维度取值。
    const c = {
      wxid: "wx-extra-only",
      domainAttributes: { value_tier: "vip" },
      commitments: [],
    } as unknown as Contact;
    render(<PlannerViewSection contact={c} />);
    expect(screen.getByTestId("planner-view-section")).toBeInTheDocument();
    expect(screen.getByText("价值分层")).toBeInTheDocument();
    expect(screen.getByText("VIP")).toBeInTheDocument();
  });

  it("conversation_mode 经字典翻译中文(A5)", () => {
    setStore(
      [],
      { conversation_mode: [{ id: "consultative", label: "顾问咨询" }] },
    );
    const c = { ...baseContact, lastConversationMode: "consultative" } as any;
    render(<PlannerViewSection contact={c} />);
    expect(screen.getByText("顾问咨询")).toBeInTheDocument();
  });

  it("阶段时间只读专用停滞时间，不使用容器更新时间", () => {
    setStore(
      [{ kind: "customer_stage", displayName: "客户阶段", participatesInDecision: true }],
      { customer_stage: [{ id: "first_contact", label: "首次接触" }] },
    );
    render(<PlannerViewSection contact={baseContact} />);
    const row = screen.getByTestId("planner-stage-row");
    expect(row.textContent).toContain("2026-06-01");
    expect(row.textContent).not.toContain("2026-06-26");
  });

  it("主动跟进视角只展示 active 承诺", () => {
    const contact = {
      ...baseContact,
      commitments: [
        { id: "active", text: "继续跟进", status: "active" },
        { id: "fulfilled", text: "已完成", status: "fulfilled" },
        { id: "cancelled", text: "已取消", status: "cancelled" },
      ],
    } as Contact;
    render(<PlannerViewSection contact={contact} />);
    expect(screen.getByText("继续跟进")).toBeInTheDocument();
    expect(screen.queryByText("已完成")).not.toBeInTheDocument();
    expect(screen.queryByText("已取消")).not.toBeInTheDocument();
  });

  it("自定义停滞维度显示同源值和时间", () => {
    setStore(
      [{ kind: "relationship_closeness", displayName: "关系亲密度", participatesInDecision: true }],
      { relationship_closeness: [{ id: "close", label: "亲密" }] },
    );
    const contact = {
      ...baseContact,
      stagnationDimension: "relationship_closeness",
      stagnationValue: "close",
      stagnationUpdatedAt: "2026-05-02T00:00:00Z",
    } as Contact;
    render(<PlannerViewSection contact={contact} />);
    const row = screen.getByTestId("planner-stage-row");
    expect(row.textContent).toContain("关系亲密度");
    expect(row.textContent).toContain("亲密");
    expect(row.textContent).toContain("2026-05-02");
  });

  it("停滞时间缺失时明确显示未知，不借用容器时间", () => {
    setStore(
      [{ kind: "customer_stage", displayName: "客户阶段", participatesInDecision: true }],
      { customer_stage: [{ id: "first_contact", label: "首次接触" }] },
    );
    const contact = { ...baseContact, stagnationUpdatedAt: null } as Contact;
    render(<PlannerViewSection contact={contact} />);
    const row = screen.getByTestId("planner-stage-row");
    expect(row.textContent).toContain("时间未知");
    expect(row.textContent).not.toContain("06/26");
  });
});
