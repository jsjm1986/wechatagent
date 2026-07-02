import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ObserveView } from "../../../features/user-ops/cockpit/ObserveView";
import { useProfileStore } from "../../../stores/profileStore";
import type { Contact, OperationHealth } from "../../../types";

// CSS Module 走 Proxy（与既有 tagTrustPanel.test 同款），让 className 直接回传键名，
// 便于断言 tone class（health_good / health_warn / health_danger）。
vi.mock("../../../features/user-ops/cockpit/cockpit.module.css", () => ({
  default: new Proxy({}, { get: (_t, k) => String(k) }),
}));

const fakeContact = {
  id: "c1",
  wxid: "wx1",
  nickname: "阿明",
  agentStatus: "managed",
  manualTags: [],
  confirmedTags: [],
  bayesianSignals: [],
  domainAttributes: {},
  commitments: [],
} as unknown as Contact;

const fakeHealth: OperationHealth = {
  scores: {},
  items: [
    { key: "a", label: "理解", score: 80, tone: "good", detail: "x" },
    { key: "b", label: "关系", score: 55, tone: "warn", detail: "y" },
    { key: "c", label: "匹配", score: 20, tone: "danger", detail: "z" },
  ],
};

// ObserveView 内嵌 PlannerViewSection（读 profileStore），先给个空底座。
beforeEach(() => useProfileStore.setState({ dimensions: [], taxonomies: {} } as any));

function renderView(onDrilldown = vi.fn()) {
  render(
    <ObserveView
      selected={fakeContact}
      decisionReviews={[]}
      memoryDraft={{} as any}
      health={fakeHealth}
      operatingMemory={null}
      onSaveManualTags={vi.fn()}
      onDrilldown={onDrilldown}
    />,
  );
  return onDrilldown;
}

describe("ObserveView", () => {
  it("健康度按 tone 渲染三项且带对应 tone class", () => {
    renderView();
    const good = screen.getByText("理解");
    const warn = screen.getByText("关系");
    const danger = screen.getByText("匹配");
    expect(good).toBeInTheDocument();
    expect(warn).toBeInTheDocument();
    expect(danger).toBeInTheDocument();
    // 健康度卡容器带 tone class（health_good/warn/danger），tone 三色由 token 化 class 承载。
    expect(good.closest("[class*='health_good']")).not.toBeNull();
    expect(warn.closest("[class*='health_warn']")).not.toBeNull();
    expect(danger.closest("[class*='health_danger']")).not.toBeNull();
  });

  it("点击记忆要点卡触发 onDrilldown('memory')", () => {
    const onDrilldown = renderView();
    fireEvent.click(screen.getByTestId("observe-memory-card"));
    expect(onDrilldown).toHaveBeenCalledWith("memory");
  });

  it("点击查看发送历史触发 onDrilldown('sendHistory')", () => {
    const onDrilldown = renderView();
    fireEvent.click(screen.getByText("查看发送历史"));
    expect(onDrilldown).toHaveBeenCalledWith("sendHistory");
  });

  it("点击查看走势详情触发 onDrilldown('trends')", () => {
    const onDrilldown = renderView();
    fireEvent.click(screen.getByText("查看走势详情"));
    expect(onDrilldown).toHaveBeenCalledWith("trends");
  });
});
