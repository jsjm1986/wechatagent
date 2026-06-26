// A2：运营记忆可编辑表单的组件级测试。
// 渲染 cockpit 标签，改一个输入框（onChange 调 onMemoryDraftChange），
// 点"保存运营记忆"（onClick 调 onSaveOperatingMemory），断言回调被触发。
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { UserOperationCockpit } from "../../features/user-ops/legacy";
import type { Contact, OperatingMemoryDraft } from "../../types";

// 子区块（PlannerViewSection / SendHistorySection）挂载即 fetch，mock api 静默。
vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({}),
  },
}));

function emptyDraft(): OperatingMemoryDraft {
  return {
    identity: "", businessContext: "", jobsToBeDone: "", painPoints: "",
    motivations: "", decisionStyle: "", communicationPreference: "",
    sensitivePoints: "", trustLevel: "", temperature: "", lastEmotion: "",
    relationshipGoal: "", doNotDo: "", interestedProducts: "", fitReason: "",
    objections: "", riskPoints: "", unknowns: "", nextGoal: "",
    recommendedMove: "", avoid: "", timing: "", reason: "",
  };
}

const selected = { id: "c1", wxid: "wx1", agentStatus: "managed" } as Contact;

function renderCockpit(overrides: Record<string, unknown> = {}) {
  const onMemoryDraftChange = vi.fn();
  const onSaveOperatingMemory = vi.fn();
  const onProfileEditDraftChange = vi.fn();
  const noop = vi.fn();
  render(
    <UserOperationCockpit
      activeTab="cockpit"
      busy={false}
      decisionReviews={[]}
      guideBusy={false}
      guideInstruction=""
      guidePreview={null}
      health={null}
      memoryCandidates={[]}
      memoryDraft={emptyDraft()}
      messages={[]}
      operatingMemory={null}
      playbooks={[]}
      profileNote=""
      customAgentInstructions=""
      assistOverride="default"
      relationshipType=""
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
      onProfileEditDraftChange={onProfileEditDraftChange}
      onRunMemoryConsolidation={noop}
      onRunSimulation={noop}
      onSaveProfileNote={noop}
      onSaveCustomAgentInstructions={noop}
      onSaveAssistOverride={noop}
      onSaveRelationshipType={noop}
      onSaveManualTags={noop}
      onMemoryDraftChange={onMemoryDraftChange}
      onSaveOperatingMemory={onSaveOperatingMemory}
      onSelectedPlaybook={noop}
      onSimulationInput={noop}
      onTab={noop}
      {...overrides}
    />,
  );
  return { onMemoryDraftChange, onSaveOperatingMemory, onProfileEditDraftChange };
}

describe("UserOperationCockpit 运营记忆编辑表单", () => {
  it("编辑身份输入框时以 patch 调 onMemoryDraftChange", () => {
    const { onMemoryDraftChange } = renderCockpit();
    const input = screen.getByPlaceholderText("这个人是谁、什么角色");
    fireEvent.change(input, { target: { value: "工程师" } });
    expect(onMemoryDraftChange).toHaveBeenCalledWith({ identity: "工程师" });
  });

  it("点保存运营记忆按钮时调 onSaveOperatingMemory", () => {
    const { onSaveOperatingMemory } = renderCockpit();
    fireEvent.click(screen.getByText("保存运营记忆"));
    expect(onSaveOperatingMemory).toHaveBeenCalledTimes(1);
  });

  it("编辑 last_commitment 输入框时以 patch 调 onProfileEditDraftChange", () => {
    const { onProfileEditDraftChange } = renderCockpit({ activeTab: "profile" });
    const input = screen.getByPlaceholderText("例：本周内给到方案报价");
    fireEvent.change(input, { target: { value: "下周回复方案" } });
    expect(onProfileEditDraftChange).toHaveBeenCalledWith({ lastCommitment: "下周回复方案" });
  });

  it("编辑 follow_up_policy 输入框时以 patch 调 onProfileEditDraftChange", () => {
    const { onProfileEditDraftChange } = renderCockpit({ activeTab: "profile" });
    const input = screen.getByPlaceholderText("例：每周跟进一次，客户明确拒绝则停止");
    fireEvent.change(input, { target: { value: "每两周一次" } });
    expect(onProfileEditDraftChange).toHaveBeenCalledWith({ followUpPolicy: "每两周一次" });
  });
});
