import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import UserOpsFeature from "../../../features/user-ops";

// Mock所有store
vi.mock("../../../stores/userOpsStore", () => ({
  useUserOpsStore: vi.fn()
}));

vi.mock("../../../stores/contactStore", () => ({
  useContactStore: vi.fn()
}));

vi.mock("../../../stores/accountStore", () => ({
  useAccountStore: vi.fn()
}));

vi.mock("../../../stores/uiStore", () => ({
  useUiStore: vi.fn()
}));

vi.mock("../../../stores/strategyStore", () => ({
  useStrategyStore: vi.fn()
}));

// Mock所有从App导入的组件
vi.mock("../../../App", () => ({
  UserOperationCockpit: vi.fn(() => <div data-testid="user-operation-cockpit">UserOperationCockpit</div>),
  ContactsView: vi.fn(() => <div data-testid="contacts-view">ContactsView</div>),
  UserOpsModeHeader: vi.fn(() => <div data-testid="user-ops-mode-header">UserOpsModeHeader</div>),
  UserPlaybookPanel: vi.fn(() => <div data-testid="user-playbook-panel">UserPlaybookPanel</div>),
  DomainPromptPanel: vi.fn(() => <div data-testid="domain-prompt-panel">DomainPromptPanel</div>),
  DomainConfigEditor: vi.fn(() => <div data-testid="domain-config-editor">DomainConfigEditor</div>),
  TraditionalOpsTabs: vi.fn(() => <div data-testid="traditional-ops-tabs">TraditionalOpsTabs</div>),
  OperationsView: vi.fn(() => <div data-testid="operations-view">OperationsView</div>)
}));

import { useUserOpsStore } from "../../../stores/userOpsStore";
import { useContactStore } from "../../../stores/contactStore";
import { useAccountStore } from "../../../stores/accountStore";
import { useUiStore } from "../../../stores/uiStore";
import { useStrategyStore } from "../../../stores/strategyStore";

describe("UserOpsFeature", () => {
  const mockContact = {
    id: "test-contact-1",
    accountId: "test-account-1",
    wxid: "test-wxid",
    nickname: "测试联系人",
    agentStatus: "managed" as const,
    tags: [],
    operationPolicy: {},
    profileAttributes: {},
    updatedAt: "2023-01-01T00:00:00Z"
  };

  const createMockStore = (overrides = {}) => ({
    userOpsMode: "smart",
    smartOpsTab: "cockpit",
    traditionalOpsTab: "playbooks",
    messages: [],
    operatingMemory: null,
    memoryCandidates: [],
    memoryDraft: {
      identity: "",
      businessContext: "",
      jobsToBeDone: "",
      painPoints: "",
      motivations: "",
      decisionStyle: "",
      communicationPreference: "",
      sensitivePoints: "",
      trustLevel: "",
      temperature: "",
      lastEmotion: "",
      relationshipGoal: "",
      doNotDo: "",
      interestedProducts: "",
      fitReason: "",
      objections: "",
      riskPoints: "",
      unknowns: "",
      nextGoal: "",
      recommendedMove: "",
      avoid: "",
      timing: "",
      reason: ""
    },
    operationHealth: null,
    decisionReviews: [],
    profileNote: "",
    customAgentInstructions: "",
    guideInstruction: "",
    guidePreview: null,
    simulationInput: "",
    simulationTurns: [],
    selectedPlaybookId: "",
    playbooks: [],
    playbookDraft: {
      name: "",
      description: "",
      methodPrompt: "",
      profileMethod: "",
      tagMethod: "",
      stageMethod: "",
      intentMethod: "",
      followUpMethod: "",
      replyStyle: "",
      forbiddenRules: "",
      successCriteria: "",
      isDefault: false
    },
    generatePlaybookText: "",
    optimizePlaybookText: "",
    editingPlaybookId: "",
    guideBusy: false,
    simulationBusy: false,
    // Domain 配置相关
    operationDomains: [],
    domainDrafts: {},
    // Actions
    setUserOpsMode: vi.fn(),
    setSmartOpsTab: vi.fn(),
    setTraditionalOpsTab: vi.fn(),
    setProfileNote: vi.fn(),
    setCustomAgentInstructions: vi.fn(),
    setGuideInstruction: vi.fn(),
    setSimulationInput: vi.fn(),
    setSelectedPlaybookId: vi.fn(),
    setPlaybookDraft: vi.fn(),
    setGeneratePlaybookText: vi.fn(),
    setOptimizePlaybookText: vi.fn(),
    setDomainDrafts: vi.fn(),
    hydrateSelected: vi.fn(),
    loadMessages: vi.fn().mockResolvedValue(undefined),
    loadPlaybooks: vi.fn().mockResolvedValue(undefined),
    loadDomains: vi.fn().mockResolvedValue(undefined),
    enableAgent: vi.fn().mockResolvedValue(undefined),
    disableAgent: vi.fn().mockResolvedValue(undefined),
    saveProfileNote: vi.fn().mockResolvedValue(undefined),
    saveCustomAgentInstructions: vi.fn().mockResolvedValue(undefined),
    analyzeProfile: vi.fn().mockResolvedValue(undefined),
    previewGuideInstruction: vi.fn().mockResolvedValue(undefined),
    applyGuidePreview: vi.fn().mockResolvedValue(undefined),
    runMemoryConsolidation: vi.fn().mockResolvedValue(undefined),
    runDialogueSimulation: vi.fn().mockResolvedValue(undefined),
    createPlaybook: vi.fn().mockResolvedValue(undefined),
    savePlaybook: vi.fn().mockResolvedValue(undefined),
    optimizePlaybook: vi.fn().mockResolvedValue(undefined),
    generatePlaybook: vi.fn().mockResolvedValue(undefined),
    setDefaultPlaybook: vi.fn().mockResolvedValue(undefined),
    editPlaybook: vi.fn(),
    newPlaybookDraft: vi.fn(),
    // Domain 配置业务方法
    saveOperationDomain: vi.fn().mockResolvedValue(undefined),
    resetOperationDomain: vi.fn().mockResolvedValue(undefined),
    ...overrides
  });

  beforeEach(() => {
    // 重置所有mock
    vi.clearAllMocks();

    // 设置默认mock
    (useUserOpsStore as any).mockReturnValue(createMockStore());

    (useContactStore as any).mockReturnValue({
      contacts: [mockContact],
      selected: mockContact,
      contactTab: "all",
      setSelected: vi.fn(),
      setContactTab: vi.fn()
    });

    (useAccountStore as any).mockReturnValue({
      currentAccountId: vi.fn(() => "test-account-1"),
      accounts: [
        {
          id: "test-account-1",
          accountId: "test-account-1",
          alias: "测试账号",
          displayName: "测试账号",
          online: true
        }
      ],
      onlineCount: vi.fn(() => 1)
    });

    (useUiStore as any).mockReturnValue({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn()
    });

    (useStrategyStore as any).mockReturnValue({
      souls: [],
      promptTemplates: [],
      soulDraft: { agentKind: "", name: "", content: "" },
      editingSoulId: "",
      promptDraft: { promptKey: "", agentKind: "", layer: "", title: "", description: "", content: "" },
      editingPromptId: "",
      setSoulDraft: vi.fn(),
      setPromptDraft: vi.fn(),
      loadStrategyData: vi.fn().mockResolvedValue(undefined),
      createSoul: vi.fn().mockResolvedValue(undefined),
      saveSoul: vi.fn().mockResolvedValue(undefined),
      publishSoul: vi.fn().mockResolvedValue(undefined),
      editSoul: vi.fn(),
      newSoulDraftFor: vi.fn(),
      createPromptTemplate: vi.fn().mockResolvedValue(undefined),
      savePromptTemplate: vi.fn().mockResolvedValue(undefined),
      publishPromptTemplate: vi.fn().mockResolvedValue(undefined),
      editPromptTemplate: vi.fn(),
      newPromptDraftFor: vi.fn()
    });

    // Mock fetch
    (globalThis as any).fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: vi.fn().mockResolvedValue({ items: [] })
    });
  });

  it("should render smart mode by default", () => {
    render(<UserOpsFeature />);

    // 应该渲染userOps工作区
    expect(screen.getByTestId("user-ops-mode-header")).toBeInTheDocument();
    expect(screen.getByTestId("contacts-view")).toBeInTheDocument();
    expect(screen.getByTestId("user-operation-cockpit")).toBeInTheDocument();
  });

  it("should render traditional mode when userOpsMode is traditional", () => {
    // 修改mock以返回traditional模式
    (useUserOpsStore as any).mockReturnValue(createMockStore({
      userOpsMode: "traditional",
      traditionalOpsTab: "playbooks"
    }));

    render(<UserOpsFeature />);

    // 应该渲染traditional模式组件
    expect(screen.getByTestId("user-ops-mode-header")).toBeInTheDocument();
    expect(screen.getByTestId("traditional-ops-tabs")).toBeInTheDocument();
    expect(screen.getByTestId("user-playbook-panel")).toBeInTheDocument();
  });

  it("should render prompts tab in traditional mode", () => {
    (useUserOpsStore as any).mockReturnValue(createMockStore({
      userOpsMode: "traditional",
      traditionalOpsTab: "prompts"
    }));

    render(<UserOpsFeature />);

    expect(screen.getByTestId("domain-prompt-panel")).toBeInTheDocument();
  });

  it("should render settings tab in traditional mode", () => {
    (useUserOpsStore as any).mockReturnValue(createMockStore({
      userOpsMode: "traditional",
      traditionalOpsTab: "settings"
    }));

    render(<UserOpsFeature />);

    expect(screen.getByTestId("domain-config-editor")).toBeInTheDocument();
  });

  it("should render audit tab in traditional mode", () => {
    (useUserOpsStore as any).mockReturnValue(createMockStore({
      userOpsMode: "traditional",
      traditionalOpsTab: "audit"
    }));

    render(<UserOpsFeature />);

    expect(screen.getByTestId("operations-view")).toBeInTheDocument();
  });
});