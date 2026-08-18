import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConfigureView } from "../../../features/user-ops/cockpit/ConfigureView";
import { useProfileStore } from "../../../stores/profileStore";
import { useUserOpsStore } from "../../../stores/userOpsStore";
import type { CockpitPanelProps } from "../../../features/user-ops/cockpit/CockpitPanel";
import type { Contact } from "../../../types";

// CSS Module 走 Proxy（与 observeView.test 同款），className 回传键名，便于断言 tab active class。
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

// memoryDraft 用 Proxy 返回 ""，保证所有字段受控（避免 controlled/uncontrolled 警告）。
const memoryDraftProxy = new Proxy({}, { get: () => "" });

beforeEach(() => {
  useProfileStore.setState({ dimensions: [], taxonomies: {} } as any);
  useUserOpsStore.setState({ clearReferral: vi.fn() } as any);
});

function renderView(overrides: Partial<CockpitPanelProps> = {}) {
  const props = {
    busy: false,
    guideBusy: false,
    guideInstruction: "",
    guidePreview: null,
    memoryCandidates: [],
    memoryDraft: memoryDraftProxy as any,
    operatingMemory: null,
    playbooks: [],
    profileNote: "",
    customAgentInstructions: "",
    assistOverride: "default",
    relationshipType: "",
    referredSpecialistAt: undefined,
    profileEditDraft: {},
    selected: fakeContact,
    selectedPlaybookId: "",
    simulationBusy: false,
    simulationInput: "",
    simulationTurns: [],
    onAnalyzeProfile: vi.fn(),
    onApplyGuidePreview: vi.fn(),
    onDisableAgent: vi.fn(),
    onEnableAgent: vi.fn(),
    onGuideInstruction: vi.fn(),
    onPreviewGuide: vi.fn(),
    onProfileNote: vi.fn(),
    onCustomAgentInstructions: vi.fn(),
    onAssistOverride: vi.fn(),
    onRelationshipType: vi.fn(),
    onProfileEditDraftChange: vi.fn(),
    onRunMemoryConsolidation: vi.fn(),
    onRunSimulation: vi.fn(),
    onSaveProfileNote: vi.fn(),
    onSaveCustomAgentInstructions: vi.fn(),
    onSaveAssistOverride: vi.fn(),
    onSaveRelationshipType: vi.fn(),
    onMemoryDraftChange: vi.fn(),
    onSaveOperatingMemory: vi.fn(),
    onSelectedPlaybook: vi.fn(),
    onSimulationInput: vi.fn(),
    ...overrides,
  } as unknown as CockpitPanelProps;
  render(<ConfigureView {...props} />);
}

describe("ConfigureView 4-tab", () => {
  it("默认显示画像 tab，其它 tab 内容不渲染", () => {
    renderView();
    // 4 个 tab 按钮都在
    expect(screen.getByRole("tab", { name: "画像" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "指令" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "记忆" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "工具" })).toBeInTheDocument();
    // 画像 tab 内容（profile 专有 label）可见
    expect(screen.getByText("你对这个用户的判断")).toBeInTheDocument();
    // 指令 / 工具 tab 内容此时不渲染
    expect(screen.queryByText("你想怎么运营这个用户？")).toBeNull();
    expect(screen.queryByText("影子验证")).toBeNull();
  });

  it("点击指令 tab 显示 AI 调整，画像内容隐藏", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "指令" }));
    expect(screen.getByText("你想怎么运营这个用户？")).toBeInTheDocument();
    expect(screen.queryByText("你对这个用户的判断")).toBeNull();
  });

  it("记忆 tab：4 分组标题都在，默认只展开第一组，其余折叠", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "记忆" }));
    // 4 个分组标题
    expect(screen.getByText("用户理解")).toBeInTheDocument();
    expect(screen.getByText("关系状态")).toBeInTheDocument();
    expect(screen.getByText("产品契合")).toBeInTheDocument();
    expect(screen.getByText("下一步动作")).toBeInTheDocument();
    // 第一组（用户理解）展开：其字段"身份"可见
    expect(screen.getByText("身份")).toBeInTheDocument();
    // 第二组（关系状态）折叠：其字段"信任程度"不渲染
    expect(screen.queryByText("信任程度")).toBeNull();
  });

  it("记忆 tab：点击折叠的分组标题展开其字段", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "记忆" }));
    expect(screen.queryByText("信任程度")).toBeNull();
    // 点击"关系状态"分组头
    fireEvent.click(screen.getByText("关系状态").closest("button")!);
    expect(screen.getByText("信任程度")).toBeInTheDocument();
  });

  it("点击工具 tab 显示长期记忆 + 影子验证", () => {
    renderView();
    fireEvent.click(screen.getByRole("tab", { name: "工具" }));
    expect(screen.getByText("影子验证")).toBeInTheDocument();
    expect(screen.getByText("Agent 已确认和待整理的信息")).toBeInTheDocument();
  });

  it("影子验证展示运行级与单轮性能诊断", () => {
    renderView({
      simulationRunMetrics: {
        projectionMode: "response_only",
        projectionDeferred: true,
        totalMs: 245,
        llmCallsUsed: 3,
      },
      simulationTurns: [{
        turn: 1,
        inboundText: "你好",
        shouldReply: true,
        replyText: "你好呀",
        status: "would_send",
        decision: {},
        review: { scores: {} },
        gatewayResult: { allowed: true },
        knowledgeRoute: { selectedChunkIds: [] },
        memoryPreview: { status: "deferred" },
        stateTransition: {},
        performance: {
          totalMs: 240,
          replyAndAuthorizationMs: 180,
          projectionMs: 0,
          projectionStatus: "deferred",
        },
      }],
    });

    fireEvent.click(screen.getByRole("tab", { name: "工具" }));
    expect(screen.getByText("模式：仅回复链路")).toBeInTheDocument();
    expect(screen.getByText("总耗时：245 ms")).toBeInTheDocument();
    expect(screen.getByText("LLM：3 次")).toBeInTheDocument();
    expect(screen.getByText("本轮：240 ms")).toBeInTheDocument();
    expect(screen.getByText("投影：deferred / 0 ms")).toBeInTheDocument();
  });

  it("managed contact has an explicit action to persist the selected operating style", () => {
    const onSaveRelationshipType = vi.fn();
    renderView({
      playbooks: [{ id: "pb-1", accountId: "A1", name: "Consultant", methodPrompt: "method", createdBy: "manual", releaseStatus: "published", isDefault: false, version: 3 }],
      selectedPlaybookId: "pb-1",
      onSaveRelationshipType,
    } as Partial<CockpitPanelProps>);

    fireEvent.click(screen.getByRole("button", { name: "保存运营风格" }));
    expect(onSaveRelationshipType).toHaveBeenCalledTimes(1);
  });

  it("renders only the authoritative diff and ignores model suggested changes", () => {
    renderView({
      guidePreview: {
        id: "preview-1",
        accountId: "A1",
        contactId: "c1",
        contactWxid: "wx1",
        instruction: "guide",
        mode: "smart",
        status: "pending",
        summary: "model summary",
        impactScope: "current_contact",
        scopeReason: "Only this contact.",
        readableChanges: ["MODEL READABLE CHANGE"],
        healthScores: {},
        suggestedChanges: { humanProfileNote: "MODEL SUGGESTED VALUE" },
        riskWarnings: [],
        candidateHash: "candidate-hash",
        authoritativeChanges: [{
          target: "contact",
          field: "human_profile_note",
          label: "human_profile_note",
          before: "old note",
          after: "authoritative note",
        }],
        requiresStrongConfirmation: false,
        playbookAffectedContacts: 0,
      },
    } as Partial<CockpitPanelProps>);

    fireEvent.click(screen.getByRole("tab", { name: "指令" }));
    expect(screen.getByText("old note → authoritative note")).toBeInTheDocument();
    expect(screen.queryByText("MODEL SUGGESTED VALUE")).toBeNull();
    expect(screen.queryByText("MODEL READABLE CHANGE")).toBeNull();
  });

  it("does not apply shared impact when strong confirmation is cancelled", () => {
    const onApplyGuidePreview = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderView({
      onApplyGuidePreview,
      guidePreview: {
        id: "preview-shared",
        accountId: "A1",
        contactId: "c1",
        contactWxid: "wx1",
        instruction: "guide",
        mode: "smart",
        status: "pending",
        summary: "shared change",
        impactScope: "shared_playbook",
        scopeReason: "Changes a shared playbook.",
        readableChanges: [],
        healthScores: {},
        suggestedChanges: {},
        riskWarnings: [],
        candidateHash: "candidate-hash",
        authoritativeChanges: [],
        requiresStrongConfirmation: true,
        playbookAffectedContacts: 12,
      },
    } as Partial<CockpitPanelProps>);

    fireEvent.click(screen.getByRole("tab", { name: "指令" }));
    fireEvent.click(screen.getByRole("button", { name: "确认应用" }));
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(onApplyGuidePreview).not.toHaveBeenCalled();
    confirm.mockRestore();
  });
});
