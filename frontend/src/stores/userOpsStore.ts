import { create } from "zustand";
import type {
  Contact,
  Message,
  DecisionReview,
  OperationPlaybook,
  PlaybookDraft,
  OperatingMemory,
  MemoryCandidateItem,
  OperatingMemoryDraft,
  OperationHealth,
  UserOperationGuidePreview,
  SimulationTurn,
  UserOpsMode,
  SmartOpsTab,
  TraditionalOpsTab,
  OperationDomainConfig,
  OperationDomainDraft,
  DomainKey
} from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";
import { useContactStore } from "./contactStore";
import { useAccountStore } from "./accountStore";
import { domainPayload, domainDraftsFromConfigs } from "./userOpsDomainHelpers";

interface UserOpsState {
  // 模式/Tab
  userOpsMode: UserOpsMode;
  smartOpsTab: SmartOpsTab;
  traditionalOpsTab: TraditionalOpsTab;

  // 选中联动数据
  messages: Message[];
  operatingMemory: OperatingMemory | null;
  memoryCandidates: MemoryCandidateItem[];
  memoryDraft: OperatingMemoryDraft;
  operationHealth: OperationHealth | null;
  decisionReviews: DecisionReview[];

  // 表单/草稿
  profileNote: string;
  customAgentInstructions: string;
  guideInstruction: string;
  guidePreview: UserOperationGuidePreview | null;
  simulationInput: string;
  simulationTurns: SimulationTurn[];
  selectedPlaybookId: string;

  // 数据
  playbooks: OperationPlaybook[];
  playbookDraft: PlaybookDraft;
  generatePlaybookText: string;
  optimizePlaybookText: string;
  editingPlaybookId: string;

  // Domain 配置相关
  operationDomains: OperationDomainConfig[];
  domainDrafts: Record<string, OperationDomainDraft>;

  // 忙碌状态
  guideBusy: boolean;
  simulationBusy: boolean;
}

interface UserOpsActions {
  // 设置器
  setUserOpsMode: (mode: UserOpsMode) => void;
  setSmartOpsTab: (tab: SmartOpsTab) => void;
  setTraditionalOpsTab: (tab: TraditionalOpsTab) => void;
  setProfileNote: (note: string) => void;
  setCustomAgentInstructions: (instructions: string) => void;
  setGuideInstruction: (instruction: string) => void;
  setSimulationInput: (input: string) => void;
  setSelectedPlaybookId: (id: string) => void;
  setPlaybookDraft: (draft: PlaybookDraft) => void;
  setGeneratePlaybookText: (text: string) => void;
  setOptimizePlaybookText: (text: string) => void;
  setEditingPlaybookId: (id: string) => void;
  setGuideBusy: (busy: boolean) => void;
  setSimulationBusy: (busy: boolean) => void;
  setDomainDrafts: (drafts: Record<string, OperationDomainDraft>) => void;

  // 核心业务方法
  hydrateSelected: (contact: Contact) => void;
  loadMessages: (contact: Contact) => Promise<void>;
  loadPlaybooks: (accountId: string) => Promise<void>;
  loadDomains: () => Promise<void>;

  // 15个业务回调
  enableAgent: () => Promise<void>;
  disableAgent: () => Promise<void>;
  saveProfileNote: () => Promise<void>;
  saveCustomAgentInstructions: () => Promise<void>;
  analyzeProfile: () => Promise<void>;
  previewGuideInstruction: (instruction: string) => Promise<void>;
  applyGuidePreview: () => Promise<void>;
  runMemoryConsolidation: () => Promise<void>;
  runDialogueSimulation: () => Promise<void>;
  createPlaybook: () => Promise<void>;
  savePlaybook: () => Promise<void>;
  optimizePlaybook: (id: string) => Promise<void>;
  generatePlaybook: () => Promise<void>;
  setDefaultPlaybook: (id: string) => Promise<void>;
  editPlaybook: (playbook: OperationPlaybook) => void;
  newPlaybookDraft: () => void;

  // Domain 配置相关业务方法
  saveOperationDomain: (domain: string) => Promise<void>;
  resetOperationDomain: (domain: string) => Promise<void>;
}

// 辅助函数
function emptyMemoryDraft(): OperatingMemoryDraft {
  return {
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
  };
}

function emptyPlaybookDraft(): PlaybookDraft {
  return {
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
  };
}

function playbookPayload(draft: PlaybookDraft) {
  return {
    name: draft.name,
    description: draft.description || undefined,
    methodPrompt: draft.methodPrompt,
    profileMethod: draft.profileMethod || undefined,
    tagMethod: draft.tagMethod || undefined,
    stageMethod: draft.stageMethod || undefined,
    intentMethod: draft.intentMethod || undefined,
    followUpMethod: draft.followUpMethod || undefined,
    replyStyle: draft.replyStyle || undefined,
    forbiddenRules: draft.forbiddenRules || undefined,
    successCriteria: draft.successCriteria || undefined,
    isDefault: draft.isDefault
  };
}

function defaultHealthItems() {
  return [
    { key: "trust_level", label: "信任度", score: 5, tone: "warn" as const, detail: "与客户的信任关系待加强" },
    { key: "engagement", label: "参与度", score: 6, tone: "good" as const, detail: "客户参与度良好" },
    { key: "intent_clarity", label: "意图明确度", score: 4, tone: "warn" as const, detail: "客户意图不够清晰" },
    { key: "relationship_depth", label: "关系深度", score: 3, tone: "danger" as const, detail: "关系较浅，需要深化" }
  ];
}

function healthFromScores(scores: Record<string, unknown>): OperationHealth {
  const items = defaultHealthItems().map(item => {
    const score = typeof scores[item.key] === "number" ? scores[item.key] as number : item.score;
    let tone: "good" | "warn" | "danger";
    if (score >= 7) tone = "good";
    else if (score >= 5) tone = "warn";
    else tone = "danger";

    return {
      ...item,
      score,
      tone
    };
  });

  // 确保scores类型正确
  const numericScores: Record<string, number> = {};
  for (const [key, value] of Object.entries(scores)) {
    if (typeof value === "number") {
      numericScores[key] = value;
    }
  }

  return {
    scores: numericScores,
    items
  };
}

// 辅助函数：刷新联系人列表
async function refreshContacts(currentAccountId: string | null) {
  if (!currentAccountId) return;

  try {
    const accountParam = `accountId=${encodeURIComponent(currentAccountId)}`;
    const contactData = await api.get<{ items: Contact[] }>(`/api/contacts?${accountParam}`);
    useContactStore.getState().setContacts(contactData.items);
  } catch (error) {
    useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
  }
}

export const useUserOpsStore = create<UserOpsState & UserOpsActions>((set, get) => ({
  // 初始状态
  userOpsMode: "smart",
  smartOpsTab: "cockpit",
  traditionalOpsTab: "playbooks",

  messages: [],
  operatingMemory: null,
  memoryCandidates: [],
  memoryDraft: emptyMemoryDraft(),
  operationHealth: null,
  decisionReviews: [],

  profileNote: "",
  customAgentInstructions: "",
  guideInstruction: "",
  guidePreview: null,
  simulationInput: "我最近在看 AI 运营，想了解你们能做到什么程度。\n我们现在几百个客户，销售经常跟丢，但我不想做机器人群发。\n如果客户三天没回，你们会一直追吗？",
  simulationTurns: [],
  selectedPlaybookId: "",

  playbooks: [],
  playbookDraft: emptyPlaybookDraft(),
  generatePlaybookText: "我们运营 AI 软件定制客户，希望像真实顾问朋友一样长期理解用户，在信任不受损的前提下自然推进需求沟通、方案确认和成交。",
  optimizePlaybookText: "让方法更像真人朋友，减少营销感；对高意向用户更自然地主动推进；对沉默客户降低打扰频率。",
  editingPlaybookId: "",

  // Domain 配置相关
  operationDomains: [],
  domainDrafts: {},

  guideBusy: false,
  simulationBusy: false,

  // 设置器
  setUserOpsMode: (mode) => set({ userOpsMode: mode }),
  setSmartOpsTab: (tab) => set({ smartOpsTab: tab }),
  setTraditionalOpsTab: (tab) => set({ traditionalOpsTab: tab }),
  setProfileNote: (note) => set({ profileNote: note }),
  setCustomAgentInstructions: (instructions) => set({ customAgentInstructions: instructions }),
  setGuideInstruction: (instruction) => set({ guideInstruction: instruction }),
  setSimulationInput: (input) => set({ simulationInput: input }),
  setSelectedPlaybookId: (id) => set({ selectedPlaybookId: id }),
  setPlaybookDraft: (draft) => set({ playbookDraft: draft }),
  setGeneratePlaybookText: (text) => set({ generatePlaybookText: text }),
  setOptimizePlaybookText: (text) => set({ optimizePlaybookText: text }),
  setEditingPlaybookId: (id) => set({ editingPlaybookId: id }),
  setGuideBusy: (busy) => set({ guideBusy: busy }),
  setSimulationBusy: (busy) => set({ simulationBusy: busy }),
  setDomainDrafts: (drafts) => set({ domainDrafts: drafts }),

  // 选中联系人时同步状态
  hydrateSelected: (contact) => {
    set({
      profileNote: contact.humanProfileNote || "",
      customAgentInstructions: contact.customAgentInstructions || "",
      selectedPlaybookId: contact.playbookId || "",
      guidePreview: null
    });
  },

  // 加载选中联系人的数据
  loadMessages: async (contact) => {
    try {
      const [
        messagesData,
        memoryData,
        candidateData,
        reviewsData,
        healthData
      ] = await Promise.all([
        api.get<{ items: Message[] }>(`/api/contacts/${contact.id}/messages?limit=50`),
        api.get<{ item: OperatingMemory }>(`/api/contacts/${contact.id}/operating-memory`),
        api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${contact.id}/memory-candidates?limit=30`),
        api.get<{ items: DecisionReview[] }>(`/api/contacts/${contact.id}/decision-reviews?limit=20`),
        api.get<any>(`/api/contacts/${contact.id}/operation-health`)
      ]);

      set({
        messages: messagesData.items,
        operatingMemory: memoryData.item,
        memoryCandidates: candidateData.items,
        decisionReviews: reviewsData.items,
        operationHealth: healthData
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  // 加载剧本列表
  loadPlaybooks: async (accountId) => {
    try {
      const accountParam = accountId ? `accountId=${encodeURIComponent(accountId)}` : "";
      const data = await api.get<{ items: OperationPlaybook[] }>(`/api/operation-playbooks${accountParam ? `?${accountParam}` : ""}`);
      set({ playbooks: data.items });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  // 加载 Domain 配置
  loadDomains: async () => {
    try {
      const data = await api.get<{ items: OperationDomainConfig[] }>("/api/operation-domains");
      set({
        operationDomains: data.items,
        domainDrafts: domainDraftsFromConfigs(data.items)
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  // 业务回调
  enableAgent: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { profileNote } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/contacts/${selected.id}/enable-agent`, {
        humanProfileNote: profileNote || undefined
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  disableAgent: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/contacts/${selected.id}/disable-agent`, {});
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveProfileNote: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { profileNote } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/profile-note`, {
        humanProfileNote: profileNote || undefined
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveCustomAgentInstructions: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { customAgentInstructions } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/custom-agent-instructions`, {
        customAgentInstructions: customAgentInstructions || undefined
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  analyzeProfile: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/contacts/${selected.id}/analyze-profile`, {});
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  previewGuideInstruction: async (instruction) => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (!selected || !currentAccountId) return;

    set({ guideBusy: true });
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: UserOperationGuidePreview }>("/api/user-operations/guide/preview", {
        accountId: currentAccountId,
        contactId: selected.id,
        instruction
      });

      set({
        guidePreview: data.item,
        operationHealth: healthFromScores(data.item.healthScores)
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      set({ guideBusy: false });
    }
  },

  applyGuidePreview: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { guidePreview } = get();

    if (!selected || !guidePreview) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: { contact: Contact; operatingMemory: OperatingMemory; health: any } }>(
        "/api/user-operations/guide/apply",
        { previewId: guidePreview.id }
      );

      set({
        operatingMemory: data.item.operatingMemory,
        guidePreview: null,
        operationHealth: data.item.health
      });

      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  runMemoryConsolidation: async () => {
    const selected = useContactStore.getState().selected;
    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: OperatingMemory }>(`/api/contacts/${selected.id}/memory-consolidation/run`, {});
      set({ operatingMemory: data.item });

      const candidateData = await api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${selected.id}/memory-candidates?limit=30`);
      set({ memoryCandidates: candidateData.items });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  runDialogueSimulation: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { simulationInput } = get();

    if (!selected || !currentAccountId) return;

    set({ simulationBusy: true });
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ items: SimulationTurn[]; runMode: string; applied: boolean }>(
        "/api/user-operations/simulations/dialogue",
        {
          accountId: currentAccountId,
          contactId: selected.id,
          inboundText: simulationInput,
          runMode: "once",
          dryRun: true
        }
      );

      set({ simulationTurns: data.items || [] });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      set({ simulationBusy: false });
    }
  },

  createPlaybook: async () => {
    const { playbookDraft } = get();
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (!playbookDraft.name.trim() || !currentAccountId) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post("/api/operation-playbooks", {
        accountId: currentAccountId,
        ...playbookPayload(playbookDraft)
      });

      set({
        playbookDraft: emptyPlaybookDraft(),
        editingPlaybookId: ""
      });

      await get().loadPlaybooks(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  savePlaybook: async () => {
    const { playbookDraft, editingPlaybookId } = get();
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (!editingPlaybookId || !playbookDraft.name.trim() || !currentAccountId) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/operation-playbooks/${editingPlaybookId}`, {
        accountId: currentAccountId,
        ...playbookPayload(playbookDraft)
      });

      await get().loadPlaybooks(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  optimizePlaybook: async (id) => {
    const { optimizePlaybookText } = get();
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (!optimizePlaybookText.trim() || !currentAccountId) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: OperationPlaybook }>(`/api/operation-playbooks/${id}/optimize`, {
        prompt: optimizePlaybookText
      });

      set({
        playbookDraft: {
          name: data.item.name,
          description: data.item.description || "",
          methodPrompt: data.item.methodPrompt,
          profileMethod: data.item.profileMethod || "",
          tagMethod: data.item.tagMethod || "",
          stageMethod: data.item.stageMethod || "",
          intentMethod: data.item.intentMethod || "",
          followUpMethod: data.item.followUpMethod || "",
          replyStyle: data.item.replyStyle || "",
          forbiddenRules: data.item.forbiddenRules || "",
          successCriteria: data.item.successCriteria || "",
          isDefault: data.item.isDefault
        },
        editingPlaybookId: id
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  generatePlaybook: async () => {
    const { generatePlaybookText } = get();
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (!generatePlaybookText.trim() || !currentAccountId) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: OperationPlaybook }>("/api/operation-playbooks/generate", {
        accountId: currentAccountId,
        prompt: generatePlaybookText
      });

      set({
        playbookDraft: {
          name: data.item.name,
          description: data.item.description || "",
          methodPrompt: data.item.methodPrompt,
          profileMethod: data.item.profileMethod || "",
          tagMethod: data.item.tagMethod || "",
          stageMethod: data.item.stageMethod || "",
          intentMethod: data.item.intentMethod || "",
          followUpMethod: data.item.followUpMethod || "",
          replyStyle: data.item.replyStyle || "",
          forbiddenRules: data.item.forbiddenRules || "",
          successCriteria: data.item.successCriteria || "",
          isDefault: data.item.isDefault
        },
        editingPlaybookId: ""
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  setDefaultPlaybook: async (id) => {
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (!currentAccountId) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/operation-playbooks/${id}/set-default`, {});
      await get().loadPlaybooks(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  editPlaybook: (playbook) => {
    set({
      editingPlaybookId: playbook.id,
      playbookDraft: {
        name: playbook.name,
        description: playbook.description || "",
        methodPrompt: playbook.methodPrompt,
        profileMethod: playbook.profileMethod || "",
        tagMethod: playbook.tagMethod || "",
        stageMethod: playbook.stageMethod || "",
        intentMethod: playbook.intentMethod || "",
        followUpMethod: playbook.followUpMethod || "",
        replyStyle: playbook.replyStyle || "",
        forbiddenRules: playbook.forbiddenRules || "",
        successCriteria: playbook.successCriteria || "",
        isDefault: playbook.isDefault
      }
    });
  },

  newPlaybookDraft: () => {
    set({
      editingPlaybookId: "",
      playbookDraft: emptyPlaybookDraft()
    });
  },

  // Domain 配置相关业务方法
  saveOperationDomain: async (domain) => {
    const { domainDrafts } = get();
    const draft = domainDrafts[domain];
    if (!draft?.name.trim()) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/operation-domains/${domain}`, domainPayload(draft));
      await get().loadDomains();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  resetOperationDomain: async (domain) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/operation-domains/${domain}/reset`);
      await get().loadDomains();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  }
}));