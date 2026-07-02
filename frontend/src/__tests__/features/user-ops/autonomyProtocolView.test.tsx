import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AutonomyProtocolView } from "../../../features/user-ops/cockpit/drilldowns/ConversationReviewView";
import type { AutonomyProtocol } from "../../../types";

const full: AutonomyProtocol = {
  userUnderstanding: "用户在比较两款方案价格",
  relationshipRead: "评估期，信任中等",
  operationGoal: "推进方案确认",
  knowledgeNeedReason: "需引用已核实报价",
  memoryUpdateReason: "记录预算区间",
  riskSelfCheck: "不对未验证功能承诺",
  selfCritique: "本轮放慢确认节奏",
  whyShouldReply: "用户主动询问差异，及时回应",
  whySkipReply: "",
};

describe("AutonomyProtocolView", () => {
  it("三组字段与值渲染，非空字段可见", () => {
    render(<AutonomyProtocolView protocol={full} />);
    expect(screen.getByText("回复决策")).toBeInTheDocument();
    expect(screen.getByText("理解")).toBeInTheDocument();
    expect(screen.getByText("运营依据")).toBeInTheDocument();
    expect(screen.getByText(/用户主动询问差异/)).toBeInTheDocument();
    expect(screen.getByText(/本轮放慢确认节奏/)).toBeInTheDocument();
    expect(screen.getByText(/推进方案确认/)).toBeInTheDocument();
  });

  it("空字段不渲染其标签（whySkipReply 空 → 不显）", () => {
    render(<AutonomyProtocolView protocol={full} />);
    expect(screen.queryByText("为何不回复")).toBeNull();
    expect(screen.getByText("为何回复")).toBeInTheDocument();
  });

  it("整组全空则该组不渲染", () => {
    const only: AutonomyProtocol = { whyShouldReply: "及时回应" };
    render(<AutonomyProtocolView protocol={only} />);
    expect(screen.getByText("回复决策")).toBeInTheDocument();
    expect(screen.queryByText("理解")).toBeNull();
    expect(screen.queryByText("运营依据")).toBeNull();
  });
});
