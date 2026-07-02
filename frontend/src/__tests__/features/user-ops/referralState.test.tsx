// E2：referral 已引荐态可观测的组件级测试。
// 驾驶舱重构后，辅助模式区块统一落在配置段 ConfigureView（原 profile 标签内容）。
// 当 referredSpecialistAt 存在时，应显式展示"已引荐 · AI 已退辅助答疑"指示（含引荐时间）；
// 不存在时不展示。
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ConfigureView } from "../../../features/user-ops/cockpit/ConfigureView";
import type { Contact } from "../../../types";

// 子区块（PlannerViewSection / SendHistorySection）挂载即 fetch，mock api 静默。
vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({}),
  },
}));

const selected = { id: "c1", wxid: "wx1", agentStatus: "managed" } as Contact;

function renderCockpit(overrides: Record<string, unknown> = {}) {
  const noop = vi.fn();
  render(
    <ConfigureView
      busy={false}
      decisionReviews={[]}
      guideBusy={false}
      guideInstruction=""
      guidePreview={null}
      health={null}
      memoryCandidates={[]}
      memoryDraft={{} as any}
      messages={[]}
      operatingMemory={null}
      playbooks={[]}
      profileNote=""
      customAgentInstructions=""
      assistOverride="default"
      relationshipType=""
      referredSpecialistAt={undefined}
      profileEditDraft={{}}
      selected={selected}
      selectedPlaybookId=""
      simulationBusy={false}
      simulationInput=""
      simulationTurns={[]}
      onAnalyzeProfile={noop}
      onApplyGuidePreview={noop}
      onDisableAgent={noop}
      onEnableAgent={noop}
      onGuideInstruction={noop}
      onPreviewGuide={noop}
      onProfileNote={noop}
      onCustomAgentInstructions={noop}
      onAssistOverride={noop}
      onRelationshipType={noop}
      onProfileEditDraftChange={noop}
      onRunMemoryConsolidation={noop}
      onRunSimulation={noop}
      onSaveProfileNote={noop}
      onSaveCustomAgentInstructions={noop}
      onSaveAssistOverride={noop}
      onSaveRelationshipType={noop}
      onSaveManualTags={noop}
      onMemoryDraftChange={noop}
      onSaveOperatingMemory={noop}
      onSelectedPlaybook={noop}
      onSimulationInput={noop}
      {...overrides}
    />,
  );
}

describe("配置段（ConfigureView）已引荐态可观测（E2）", () => {
  it("referredSpecialistAt 存在时展示已引荐指示", () => {
    renderCockpit({ referredSpecialistAt: "2026-06-26T00:00:00Z" });
    expect(screen.getByText(/已引荐/)).toBeInTheDocument();
    expect(screen.getByText(/AI 已退辅助答疑/)).toBeInTheDocument();
  });

  it("referredSpecialistAt 不存在时不展示已引荐指示", () => {
    renderCockpit({ referredSpecialistAt: undefined });
    expect(screen.queryByText(/AI 已退辅助答疑/)).not.toBeInTheDocument();
  });
});
