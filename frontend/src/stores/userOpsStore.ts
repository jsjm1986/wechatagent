import { create } from "zustand";
import type {
  Contact,
  RosterEntry,
  Message,
  DecisionReview,
  OperationPlaybook,
  PlaybookDraft,
  OperatingMemory,
  MemoryCandidateItem,
  OperatingMemoryDraft,
  OperationHealth,
  UserOperationGuidePreview,
  UserOperationGuideApplyResult,
  SimulationTurn,
  UserOpsMode,
  TraditionalOpsTab,
  OperationDomainConfig,
  OperationDomainDraft,
  DomainKey
} from "../types";
import { api } from "../lib/api";
import { useInboxStore } from "./inboxStore";
import { useUiStore } from "./uiStore";
import { useContactStore } from "./contactStore";
import { useAccountStore } from "./accountStore";
import { domainPayload, domainDraftsFromConfigs } from "./userOpsDomainHelpers";

interface UserOpsState {
  // 模式/Tab
  userOpsMode: UserOpsMode;
  traditionalOpsTab: TraditionalOpsTab;

  // 选中联动数据
  messages: Message[];
  operatingMemory: OperatingMemory | null;
  memoryCandidates: MemoryCandidateItem[];
  memoryDraft: OperatingMemoryDraft;
  operationHealth: OperationHealth | null;
  decisionReviews: DecisionReview[];
  detailAccountId: string;
  detailContactId: string;
  detailGeneration: number;
  // 表单/草稿
  searchQuery: string;
  profileNote: string;
  customAgentInstructions: string;
  assistOverride: string; // "default" | "force_on" | "force_off"
  relationshipType: string; // "" | "customer" | "peer" | "friend"
  // E2：referral 已引荐态（只读观测）。后端把引荐时间/名片 id 写入 contact 的
  // domain_attributes（referred_specialist_at / referred_card_id），前端经
  // domainAttributes dotted-key 回填，仅用于详情面板展示"已引荐 · AI 已退辅助答疑"。
  referredSpecialistAt?: string;
  referredCardId?: string;
  // A3：operation-profile 运营可编辑草稿（last_commitment / follow_up_policy）。
  // customer_stage / intent_level 由 AI 派生、前端只读，不进此草稿。
  profileEditDraft: { lastCommitment?: string; followUpPolicy?: string };
  guideInstruction: string;
  guidePreview: UserOperationGuidePreview | null;
  guidePreviewGeneration: number;
  simulationInput: string;
  simulationTurns: SimulationTurn[];
  selectedPlaybookId: string;

  // 数据
  playbooks: OperationPlaybook[];
  playbookDraft: PlaybookDraft;
  generatePlaybookText: string;
  optimizePlaybookText: string;
  editingPlaybookId: string;
  editingPlaybookAccountId: string;
  editingPlaybookVersion: number | null;
  playbookScopeAccountId: string;
  playbookRequestGeneration: number;

  // Domain 配置相关
  operationDomains: OperationDomainConfig[];
  domainDrafts: Record<string, OperationDomainDraft>;

  // 通讯录 roster 缓存（按 accountId 键控）：首次拉后缓存，进频道复用不重打 API；
  // 仅就绪结果落缓存（syncing 中不缓存，允许自动重拉覆盖）。force 才重拉。
  rosterCache: Record<string, { items: RosterEntry[]; syncing: boolean; fetchedAt: number }>;

  // 运营池三 tab 的后端真实计数（不受 list_contacts 的 limit 截断影响）。
  contactCounts: { all: number; managed: number; normal: number };

  // 忙碌状态
  guideBusy: boolean;
  simulationBusy: boolean;
}

interface UserOpsActions {
  // 设置器
  setUserOpsMode: (mode: UserOpsMode) => void;
  setTraditionalOpsTab: (tab: TraditionalOpsTab) => void;
  setProfileNote: (note: string) => void;
  setCustomAgentInstructions: (instructions: string) => void;
  setAssistOverride: (mode: string) => void;
  setRelationshipType: (value: string) => void;
  setProfileEditDraft: (patch: Partial<{ lastCommitment: string; followUpPolicy: string }>) => void;
  setGuideInstruction: (instruction: string) => void;
  setSimulationInput: (input: string) => void;
  setSelectedPlaybookId: (id: string) => void;
  setSearchQuery: (value: string) => void;
  setPlaybookDraft: (draft: PlaybookDraft) => void;
  setGeneratePlaybookText: (text: string) => void;
  setOptimizePlaybookText: (text: string) => void;
  setEditingPlaybookId: (id: string) => void;
  setGuideBusy: (busy: boolean) => void;
  setSimulationBusy: (busy: boolean) => void;
  setDomainDrafts: (drafts: Record<string, OperationDomainDraft>) => void;
  setMemoryDraft: (patch: Partial<OperatingMemoryDraft>) => void;

  // 核心业务方法
  hydrateSelected: (contact: Contact) => void;
  clearContactDetail: (accountId: string) => void;
  loadMessages: (contact: Contact) => Promise<void>;
  loadPlaybooks: (accountId: string) => Promise<void>;
  loadContacts: (accountId: string) => Promise<void>;
  loadContactCounts: (accountId: string) => Promise<void>;
  hideFromPool: (accountId: string, contactId: string) => Promise<void>;
  loadRoster: (accountId: string, opts?: { force?: boolean }) => Promise<{ items: RosterEntry[]; syncing: boolean }>;
  batchEnable: (payload: {
    accountId: string;
    source: "pool" | "roster";
    candidates: { contactId?: string; wxid: string; nickname?: string | null; remark?: string | null; avatarUrl?: string | null; sex?: number | null }[];
    sharedNote: string;
    playbookId?: string;
  }) => Promise<{ enabled: number; queued: number }>;
  loadDomains: () => Promise<void>;

  // 15个业务回调
  enableAgent: () => Promise<void>;
  disableAgent: () => Promise<void>;
  saveProfileNote: () => Promise<void>;
  saveCustomAgentInstructions: () => Promise<void>;
  saveAssistOverride: () => Promise<void>;
  saveOperationProfile: () => Promise<void>;
  saveOperatingMemory: () => Promise<void>;
  clearReferral: (contactId: string) => Promise<void>;
  saveManualTags: (contact: Contact, tags: string[]) => Promise<void>;
  analyzeProfile: () => Promise<void>;
  previewGuideInstruction: (instruction: string) => Promise<void>;
  applyGuidePreview: (confirmGlobalImpact?: boolean) => Promise<UserOperationGuideApplyResult | null>;
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
  saveOperationDomain: (domain: string) => Promise<boolean>;
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

// 扁平 memoryDraft → 后端四个 Document 的归组键映射（OperatingMemoryRequest，
// camelCase wire 键）。saveOperatingMemory 据此把 23 个扁平字段拆进四组提交，
// loadMessages 据此把后端四组拆回扁平 draft 回填表单——两向用同一份映射，避免漂移。
const MEMORY_DRAFT_GROUPS: Record<
  "userUnderstanding" | "relationshipState" | "productFit" | "nextAction",
  (keyof OperatingMemoryDraft)[]
> = {
  userUnderstanding: [
    "identity",
    "businessContext",
    "jobsToBeDone",
    "painPoints",
    "motivations",
    "decisionStyle",
    "communicationPreference"
  ],
  relationshipState: [
    "sensitivePoints",
    "trustLevel",
    "temperature",
    "lastEmotion",
    "relationshipGoal",
    "doNotDo"
  ],
  productFit: ["interestedProducts", "fitReason"],
  nextAction: [
    "objections",
    "riskPoints",
    "unknowns",
    "nextGoal",
    "recommendedMove",
    "avoid",
    "timing",
    "reason"
  ]
};

// 扁平 draft → 四个嵌套 Document（提交给后端 PUT /operating-memory）。
function groupMemoryDraft(draft: OperatingMemoryDraft) {
  const grouped: Record<string, Record<string, string>> = {};
  for (const [group, keys] of Object.entries(MEMORY_DRAFT_GROUPS)) {
    grouped[group] = {};
    for (const key of keys) {
      grouped[group][key] = draft[key];
    }
  }
  return grouped;
}

// 后端 OperatingMemory 四个 Document → 扁平 draft（回填可编辑表单）。
// memory 为 null 时回落空表单。值缺失或非字符串时归一为空串。
function memoryDraftFromMemory(memory: OperatingMemory | null): OperatingMemoryDraft {
  const draft = emptyMemoryDraft();
  if (!memory) return draft;
  for (const [group, keys] of Object.entries(MEMORY_DRAFT_GROUPS)) {
    const source = (memory as unknown as Record<string, unknown>)[group] as
      | Record<string, unknown>
      | undefined;
    if (!source) continue;
    for (const key of keys) {
      const value = source[key];
      if (typeof value === "string") {
        draft[key] = value;
      }
    }
  }
  return draft;
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

// 辅助函数：刷新联系人列表。透传 searchQuery 作为后端 q= 过滤参数
// （后端 GET /api/contacts 支持 q 子串过滤已导入好友）。
async function refreshContacts(currentAccountId: string | null, q?: string) {
  if (!currentAccountId) return;
  await useContactStore.getState().loadContacts(currentAccountId, q);
}

function detailActionIsCurrent(state: UserOpsState, contact: Contact | null): contact is Contact {
  if (!contact) return false;
  const currentAccountId = useAccountStore.getState().currentAccountId();
  const contactState = useContactStore.getState();
  return Boolean(currentAccountId)
    && contact.accountId === currentAccountId
    && contactState.dataAccountId === currentAccountId
    && contactState.selected?.id === contact.id
    && contactState.selected?.accountId === currentAccountId
    && state.detailAccountId === currentAccountId
    && state.detailContactId === contact.id;
}

export const useUserOpsStore = create<UserOpsState & UserOpsActions>((set, get) => ({
  // 初始状态
  userOpsMode: "smart",
  traditionalOpsTab: "playbooks",

  messages: [],
  operatingMemory: null,
  memoryCandidates: [],
  memoryDraft: emptyMemoryDraft(),
  operationHealth: null,
  decisionReviews: [],
  detailAccountId: "",
  detailContactId: "",
  detailGeneration: 0,

  profileNote: "",
  customAgentInstructions: "",
  assistOverride: "default",
  relationshipType: "",
  referredSpecialistAt: undefined,
  referredCardId: undefined,
  profileEditDraft: {},
  searchQuery: "",
  guideInstruction: "",
  guidePreview: null,
  guidePreviewGeneration: 0,
  simulationInput: "我最近在看 AI 运营，想了解你们能做到什么程度。\n我们现在几百个客户，销售经常跟丢，但我不想做机器人群发。\n如果客户三天没回，你们会一直追吗？",
  simulationTurns: [],
  selectedPlaybookId: "",

  playbooks: [],
  playbookDraft: emptyPlaybookDraft(),
  generatePlaybookText: "我们运营 AI 软件定制客户，希望像真实顾问朋友一样长期理解用户，在信任不受损的前提下自然推进需求沟通、方案确认和成交。",
  optimizePlaybookText: "让方法更像真人朋友，减少营销感；对高意向用户更自然地主动推进；对沉默客户降低打扰频率。",
  editingPlaybookId: "",
  editingPlaybookAccountId: "",
  editingPlaybookVersion: null,
  playbookScopeAccountId: "",
  playbookRequestGeneration: 0,

  // Domain 配置相关
  operationDomains: [],
  domainDrafts: {},

  rosterCache: {},
  contactCounts: { all: 0, managed: 0, normal: 0 },

  guideBusy: false,
  simulationBusy: false,

  // 设置器
  setUserOpsMode: (mode) => set({ userOpsMode: mode }),
  setTraditionalOpsTab: (tab) => set({ traditionalOpsTab: tab }),
  setProfileNote: (note) => set({ profileNote: note }),
  setCustomAgentInstructions: (instructions) => set({ customAgentInstructions: instructions }),
  setAssistOverride: (mode) => set({ assistOverride: mode }),
  setRelationshipType: (value) => set({ relationshipType: value }),
  setProfileEditDraft: (patch) => set((s) => ({ profileEditDraft: { ...s.profileEditDraft, ...patch } })),
  setGuideInstruction: (instruction) => set({ guideInstruction: instruction }),
  setSimulationInput: (input) => set({ simulationInput: input }),
  setSelectedPlaybookId: (id) => set({ selectedPlaybookId: id }),
  setSearchQuery: (value) => set({ searchQuery: value }),
  setPlaybookDraft: (draft) => set({ playbookDraft: draft }),
  setGeneratePlaybookText: (text) => set({ generatePlaybookText: text }),
  setOptimizePlaybookText: (text) => set({ optimizePlaybookText: text }),
  setEditingPlaybookId: (id) => set({ editingPlaybookId: id }),
  setGuideBusy: (busy) => set({ guideBusy: busy }),
  setSimulationBusy: (busy) => set({ simulationBusy: busy }),
  setDomainDrafts: (drafts) => set({ domainDrafts: drafts }),
  setMemoryDraft: (patch) => set((s) => ({ memoryDraft: { ...s.memoryDraft, ...patch } })),

  clearContactDetail: (accountId) => set((state) => ({
    messages: [],
    operatingMemory: null,
    memoryCandidates: [],
    memoryDraft: emptyMemoryDraft(),
    operationHealth: null,
    decisionReviews: [],
    detailAccountId: accountId,
    detailContactId: "",
    detailGeneration: state.detailGeneration + 1,
    profileNote: "",
    customAgentInstructions: "",
    assistOverride: "default",
    relationshipType: "",
    referredSpecialistAt: undefined,
    referredCardId: undefined,
    profileEditDraft: {},
    selectedPlaybookId: "",
    guidePreview: null,
    guidePreviewGeneration: state.guidePreviewGeneration + 1,
    guideBusy: false,
  })),

  // 选中联系人时同步状态
  hydrateSelected: (contact) => {
    set((state) => ({
      messages: [],
      operatingMemory: null,
      memoryCandidates: [],
      memoryDraft: emptyMemoryDraft(),
      operationHealth: null,
      decisionReviews: [],
      detailAccountId: contact.accountId,
      detailContactId: contact.id,
      detailGeneration: state.detailGeneration + 1,
      profileNote: contact.humanProfileNote || "",
      customAgentInstructions: contact.customAgentInstructions || "",
      assistOverride:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "assist_mode_override"
        ] as string) || "default",
      relationshipType:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "relationship_type"
        ] as string) || "",
      // E2：从 domain_attributes 回填已引荐态（只读观测）。
      referredSpecialistAt:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "referred_specialist_at"
        ] as string) || undefined,
      referredCardId:
        ((contact.domainAttributes as Record<string, unknown> | undefined)?.[
          "referred_card_id"
        ] as string) || undefined,
      // A3：回填运营可编辑的两字段（有则填，无则空），customer_stage/intent_level 不回填（AI 派生只读）。
      profileEditDraft: {
        lastCommitment: contact.lastCommitment || "",
        followUpPolicy: contact.followUpPolicy || "",
      },
      selectedPlaybookId: contact.playbookId || "",
      guidePreview: null,
      guidePreviewGeneration: state.guidePreviewGeneration + 1,
      guideBusy: false,
    }));
  },

  // 加载选中联系人的数据
  loadMessages: async (contact) => {
    const accountId = contact.accountId;
    const detailGeneration = get().detailGeneration;
    const contactState = useContactStore.getState();
    if (
      useAccountStore.getState().currentAccountId() !== accountId ||
      contactState.dataAccountId !== accountId ||
      contactState.selected?.id !== contact.id ||
      contactState.selected?.accountId !== accountId ||
      get().detailAccountId !== accountId ||
      get().detailContactId !== contact.id
    ) return;
    const [messagesR, memoryR, candidateR, reviewsR, healthR] = await Promise.allSettled([
      api.get<{ items: Message[] }>(`/api/conversations/${contact.id}/messages?limit=50`),
      api.get<{ item: OperatingMemory }>(`/api/contacts/${contact.id}/operating-memory`),
      api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${contact.id}/memory-candidates?limit=30`),
      api.get<{ items: DecisionReview[] }>(`/api/decision-reviews?accountId=${encodeURIComponent(accountId)}&contactId=${contact.id}&limit=20`),
      api.get<OperationHealth>(`/api/contacts/${contact.id}/operation-health`),
    ]);
    const currentContactState = useContactStore.getState();
    if (
      get().detailGeneration !== detailGeneration ||
      get().detailAccountId !== accountId ||
      get().detailContactId !== contact.id ||
      useAccountStore.getState().currentAccountId() !== accountId ||
      currentContactState.dataAccountId !== accountId ||
      currentContactState.selected?.id !== contact.id ||
      currentContactState.selected?.accountId !== accountId
    ) return;
    set({
      messages: messagesR.status === "fulfilled" ? messagesR.value.items : [],
      operatingMemory: memoryR.status === "fulfilled" ? memoryR.value.item : null,
      // A2：把 operatingMemory 拆回扁平字段回填 memoryDraft，让可编辑表单先看到现有值；
      // 加载失败/无记忆时回落空表单（memoryDraftFromMemory(null)）。
      memoryDraft: memoryDraftFromMemory(
        memoryR.status === "fulfilled" ? memoryR.value.item : null,
      ),
      memoryCandidates: candidateR.status === "fulfilled" ? candidateR.value.items : [],
      decisionReviews: reviewsR.status === "fulfilled" ? reviewsR.value.items : [],
      operationHealth: healthR.status === "fulfilled" ? healthR.value : null,
    });
    const firstErr = [messagesR, memoryR, candidateR, reviewsR, healthR]
      .find((r) => r.status === "rejected") as PromiseRejectedResult | undefined;
    if (firstErr) {
      useUiStore.getState().setError(
        firstErr.reason instanceof Error ? firstErr.reason.message : String(firstErr.reason),
      );
    }
    // 判断条请示灯与请示频道共用 Inbox summary；失败保留最后成功快照，不能伪装成 0。
    void useInboxStore.getState().refreshSummary();
  },

  // 加载剧本列表
  loadPlaybooks: async (accountId) => {
    if (!accountId) return;
    const generation = get().playbookRequestGeneration + 1;
    const accountChanged = get().playbookScopeAccountId !== accountId;
    set({
      playbookScopeAccountId: accountId,
      playbookRequestGeneration: generation,
      ...(accountChanged
        ? {
            playbooks: [],
            editingPlaybookId: "",
            editingPlaybookAccountId: "",
            editingPlaybookVersion: null,
            playbookDraft: emptyPlaybookDraft(),
            generatePlaybookText: "",
            optimizePlaybookText: ""
          }
        : {})
    });
    try {
      const data = await api.get<{ items: OperationPlaybook[] }>(
        `/api/operation-playbooks?accountId=${encodeURIComponent(accountId)}`
      );
      const current = get();
      if (
        current.playbookRequestGeneration !== generation ||
        current.playbookScopeAccountId !== accountId ||
        useAccountStore.getState().currentAccountId() !== accountId
      ) {
        return;
      }
      if (data.items.some((playbook) => playbook.accountId !== accountId)) {
        throw new Error("Playbook 响应账号不匹配");
      }
      set({ playbooks: data.items });
    } catch (error) {
      const current = get();
      if (
        current.playbookRequestGeneration === generation &&
        current.playbookScopeAccountId === accountId
      ) {
        useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      }
    }
  },

  // 加载（切账号 / 挂载时 / 搜索过滤时）联系人列表——用户运营页主体数据，
  // 复用模块内 refreshContacts，透传当前 searchQuery 作为 q 过滤。
  loadContacts: async (accountId) => {
    await refreshContacts(accountId || null, get().searchQuery);
  },

  // 拉运营池三 tab 的后端真实计数。失败回落保留旧值（不弹错、不清零），
  // 避免网络抖动把计数瞬间清 0 误导运营。
  loadContactCounts: async (accountId) => {
    if (!accountId) return;
    try {
      const data = await api.get<{ all: number; managed: number; normal: number }>(
        `/api/contacts/counts?accountId=${encodeURIComponent(accountId)}`
      );
      set({ contactCounts: { all: data.all, managed: data.managed, normal: data.normal } });
    } catch {
      // 保留旧值，静默降级。
    }
  },

  // 通讯录：拉指定账号的全量好友（MCP）+ 本地 contacts 左连接标注 agentStatus。纯浏览，不写库。
  loadRoster: async (accountId, opts) => {
    const cached = get().rosterCache[accountId];
    if (!opts?.force && cached) {
      return { items: cached.items, syncing: cached.syncing };
    }
    const url = `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}${
      opts?.force ? "&force=true" : ""
    }`;
    const data = await api.get<{ items: RosterEntry[]; syncing?: boolean }>(url);
    const result = { items: data.items, syncing: data.syncing ?? false };
    // 仅就绪结果落缓存；同步中(syncing)不缓存，避免卡在同步中态、允许自动重拉覆盖。
    if (!result.syncing) {
      set((s) => ({
        rosterCache: { ...s.rosterCache, [accountId]: { ...result, fetchedAt: Date.now() } },
      }));
    }
    return result;
  },

  // 批量托管：把勾选好友一次性置 managed + 共享运营备注，后端异步入队 initial_profile 生成画像。
  batchEnable: async (payload) => {
    return await api.post<{ enabled: number; queued: number; rejectedSelf?: number; rejectedNonHuman?: number }>(
      "/api/contacts/batch-enable",
      payload
    );
  },

  // 手动把联系人从运营池移除（媒体号等无法自动判定的非目标）。调后端标记
  // hidden_from_pool（不删记录），成功后刷新列表 + 计数。
  hideFromPool: async (accountId, contactId) => {
    await api.post(`/api/contacts/${encodeURIComponent(contactId)}/hide-from-pool`);
    await refreshContacts(accountId || null, get().searchQuery);
    await get().loadContactCounts(accountId);
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
    const { profileNote, selectedPlaybookId } = get();

    if (!detailActionIsCurrent(get(), selected)) return;

    set({ guideBusy: true });
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/contacts/${selected.id}/enable-agent`, {
        expectedAccountId: selected.accountId,
        humanProfileNote: profileNote || undefined,
        playbookId: selectedPlaybookId || undefined,
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      set({ guideBusy: false });
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

    if (!detailActionIsCurrent(get(), selected)) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/profile-note`, {
        expectedAccountId: selected.accountId,
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

    if (!detailActionIsCurrent(get(), selected)) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/custom-agent-instructions`, {
        expectedAccountId: selected.accountId,
        instructions: customAgentInstructions
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveAssistOverride: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { assistOverride } = get();

    if (!detailActionIsCurrent(get(), selected)) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/assist-override`, {
        expectedAccountId: selected.accountId,
        mode: assistOverride
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveOperationProfile: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { relationshipType, profileEditDraft, selectedPlaybookId } = get();

    if (!detailActionIsCurrent(get(), selected)) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/operation-profile`, {
        expectedAccountId: selected.accountId,
        playbookId: selectedPlaybookId || undefined,
        relationshipType: relationshipType || undefined,
        lastCommitment: profileEditDraft.lastCommitment || undefined,
        followUpPolicy: profileEditDraft.followUpPolicy || undefined,
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveOperatingMemory: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { memoryDraft } = get();

    if (!detailActionIsCurrent(get(), selected)) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      // A2：把扁平 23 字段 memoryDraft 归组成后端要求的四个嵌套 Document
      // （userUnderstanding/relationshipState/productFit/nextAction）后整体 $set。
      await api.put(`/api/contacts/${selected.id}/operating-memory`, {
        expectedAccountId: selected.accountId,
        ...groupMemoryDraft(memoryDraft),
      });
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  clearReferral: async (contactId: string) => {
    const currentAccountId = useAccountStore.getState().currentAccountId();
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/contacts/${contactId}/clear-referral`);
      await refreshContacts(currentAccountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveManualTags: async (contact, tags) => {
    // 标签可信度改造 - 运营录入层：保存运营录入标签，后端权威覆盖。
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const selected = useContactStore.getState().selected;
    if (!detailActionIsCurrent(get(), contact) || selected?.id !== contact.id) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${contact.id}/manual-tags`, {
        expectedAccountId: contact.accountId,
        tags,
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

    if (!selected || !currentAccountId || !detailActionIsCurrent(get(), selected)) return;
    const requestGeneration = get().guidePreviewGeneration + 1;

    set({
      guideBusy: true,
      guidePreview: null,
      guidePreviewGeneration: requestGeneration,
    });
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: UserOperationGuidePreview }>("/api/user-operations/guide/preview", {
        accountId: currentAccountId,
        contactId: selected.id,
        instruction
      });

      if (
        get().guidePreviewGeneration !== requestGeneration ||
        !detailActionIsCurrent(get(), selected)
      ) {
        return;
      }
      if (
        data.item.accountId !== currentAccountId ||
        data.item.contactId !== selected.id
      ) {
        throw new Error("guide_preview_identity_conflict");
      }

      const next: Partial<UserOpsState> = { guidePreview: data.item };
      // FE-1：直接用后端构建好的 health（scores + canonical 7 项 items，与正常加载
      // 路径 /operation-health 同口径、同形态），不再用前端 healthFromScores 拿
      // healthScores 重建 4-key 占位分（旧函数 key 与后端零交集，展示伪造分）。
      if (data.item.health && data.item.health.items.length > 0) {
        next.operationHealth = data.item.health;
      }
      set(next);
    } catch (error) {
      if (
        get().guidePreviewGeneration === requestGeneration &&
        detailActionIsCurrent(get(), selected)
      ) {
        useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (get().guidePreviewGeneration === requestGeneration) {
        set({ guideBusy: false });
      }
    }
  },

  applyGuidePreview: async (confirmGlobalImpact = false) => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { guidePreview, guidePreviewGeneration } = get();

    if (
      !selected ||
      !currentAccountId ||
      !guidePreview ||
      !detailActionIsCurrent(get(), selected) ||
      guidePreview.accountId !== currentAccountId ||
      guidePreview.contactId !== selected.id ||
      !guidePreview.candidateHash
    ) {
      return null;
    }

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: UserOperationGuideApplyResult }>(
        "/api/user-operations/guide/apply",
        {
          previewId: guidePreview.id,
          expectedAccountId: currentAccountId,
          expectedContactId: selected.id,
          candidateHash: guidePreview.candidateHash,
          confirmGlobalImpact,
        }
      );

      if (
        get().guidePreviewGeneration !== guidePreviewGeneration ||
        !detailActionIsCurrent(get(), selected)
      ) {
        return null;
      }

      set((state) => ({
        guidePreview: null,
        guidePreviewGeneration: state.guidePreviewGeneration + 1,
        guideBusy: false,
      }));

      await refreshContacts(currentAccountId);
      const refreshed = useContactStore.getState().contacts.find((item) => item.id === selected.id);
      if (refreshed && detailActionIsCurrent(get(), selected)) {
        useContactStore.getState().setSelected(refreshed);
        get().hydrateSelected(refreshed);
        await get().loadMessages(refreshed);
      }
      return data.item;
    } catch (error) {
      if (
        get().guidePreviewGeneration === guidePreviewGeneration &&
        detailActionIsCurrent(get(), selected)
      ) {
        useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      }
      return null;
    } finally {
      if (get().guidePreviewGeneration === guidePreviewGeneration) {
        set({ guideBusy: false });
      }
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
      const messages = simulationInput
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line.length > 0);

      const data = await api.post<{ items: SimulationTurn[]; runMode: string; applied: boolean }>(
        "/api/user-operations/simulations/dialogue",
        {
          accountId: currentAccountId,
          contactId: selected.id,
          messages
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
    const requestGeneration = get().playbookRequestGeneration;

    if (
      !playbookDraft.name.trim() ||
      !currentAccountId ||
      get().playbookScopeAccountId !== currentAccountId
    ) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post("/api/operation-playbooks", {
        accountId: currentAccountId,
        ...playbookPayload(playbookDraft)
      });

      if (
        useAccountStore.getState().currentAccountId() === currentAccountId &&
        get().playbookScopeAccountId === currentAccountId &&
        get().playbookRequestGeneration === requestGeneration
      ) {
        set({
          playbookDraft: emptyPlaybookDraft(),
          editingPlaybookId: "",
          editingPlaybookAccountId: "",
          editingPlaybookVersion: null
        });
        await get().loadPlaybooks(currentAccountId);
      }
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  savePlaybook: async () => {
    const {
      playbookDraft,
      editingPlaybookId,
      editingPlaybookAccountId,
      editingPlaybookVersion,
      playbookScopeAccountId
    } = get();
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (
      !editingPlaybookId ||
      !playbookDraft.name.trim() ||
      !currentAccountId ||
      editingPlaybookAccountId !== currentAccountId ||
      playbookScopeAccountId !== currentAccountId ||
      editingPlaybookVersion === null
    ) {
      if (editingPlaybookAccountId && editingPlaybookAccountId !== currentAccountId) {
        set({
          editingPlaybookId: "",
          editingPlaybookAccountId: "",
          editingPlaybookVersion: null,
          playbookDraft: emptyPlaybookDraft()
        });
      }
      return;
    }

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const result = await api.put<{ version: number }>(`/api/operation-playbooks/${editingPlaybookId}`, {
        accountId: editingPlaybookAccountId,
        expectedVersion: editingPlaybookVersion,
        ...playbookPayload(playbookDraft)
      });
      if (
        useAccountStore.getState().currentAccountId() === editingPlaybookAccountId &&
        get().editingPlaybookId === editingPlaybookId &&
        get().editingPlaybookVersion === editingPlaybookVersion
      ) {
        set({ editingPlaybookVersion: result.version });
        await get().loadPlaybooks(editingPlaybookAccountId);
      }
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  optimizePlaybook: async (id) => {
    const {
      optimizePlaybookText,
      editingPlaybookId,
      editingPlaybookAccountId,
      editingPlaybookVersion,
      playbookScopeAccountId
    } = get();
    const currentAccountId = useAccountStore.getState().currentAccountId();

    if (
      !optimizePlaybookText.trim() ||
      !currentAccountId ||
      id !== editingPlaybookId ||
      editingPlaybookAccountId !== currentAccountId ||
      playbookScopeAccountId !== currentAccountId ||
      editingPlaybookVersion === null
    ) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: OperationPlaybook }>(`/api/operation-playbooks/${id}/optimize`, {
        accountId: editingPlaybookAccountId,
        expectedVersion: editingPlaybookVersion,
        instruction: optimizePlaybookText
      });

      if (
        useAccountStore.getState().currentAccountId() !== editingPlaybookAccountId ||
        get().editingPlaybookId !== id ||
        get().editingPlaybookVersion !== editingPlaybookVersion ||
        data.item.accountId !== editingPlaybookAccountId ||
        !data.item.id ||
        data.item.id === id ||
        data.item.version !== editingPlaybookVersion + 1
      ) return;
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
        // AI optimization returns a new non-default candidate. Keep the source
        // row untouched and move the editor to the candidate identity.
        editingPlaybookId: data.item.id,
        editingPlaybookAccountId,
        editingPlaybookVersion: data.item.version
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
    const requestGeneration = get().playbookRequestGeneration;

    if (
      !generatePlaybookText.trim() ||
      !currentAccountId ||
      get().playbookScopeAccountId !== currentAccountId
    ) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: OperationPlaybook }>("/api/operation-playbooks/generate", {
        accountId: currentAccountId,
        description: generatePlaybookText
      });

      if (
        useAccountStore.getState().currentAccountId() !== currentAccountId ||
        get().playbookScopeAccountId !== currentAccountId ||
        get().playbookRequestGeneration !== requestGeneration ||
        data.item.accountId !== currentAccountId
      ) return;
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
        editingPlaybookId: data.item.id,
        editingPlaybookAccountId: data.item.accountId,
        editingPlaybookVersion: data.item.version
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  setDefaultPlaybook: async (id) => {
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const playbook = get().playbooks.find(
      (item) => item.id === id && item.accountId === currentAccountId
    );

    if (!currentAccountId || get().playbookScopeAccountId !== currentAccountId || !playbook) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/operation-playbooks/${id}/set-default`, {
        accountId: playbook.accountId,
        expectedVersion: playbook.version
      });
      if (
        useAccountStore.getState().currentAccountId() === currentAccountId &&
        get().playbookScopeAccountId === currentAccountId
      ) {
        await get().loadPlaybooks(currentAccountId);
      }
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  editPlaybook: (playbook) => {
    const currentAccountId = useAccountStore.getState().currentAccountId();
    if (
      !currentAccountId ||
      playbook.accountId !== currentAccountId ||
      get().playbookScopeAccountId !== currentAccountId
    ) return;
    set({
      editingPlaybookId: playbook.id,
      editingPlaybookAccountId: playbook.accountId,
      editingPlaybookVersion: playbook.version,
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
      editingPlaybookAccountId: "",
      editingPlaybookVersion: null,
      playbookDraft: emptyPlaybookDraft()
    });
  },

  // Domain 配置相关业务方法
  saveOperationDomain: async (domain) => {
    const { domainDrafts } = get();
    const draft = domainDrafts[domain];
    if (!draft?.name.trim()) return false;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/operation-domains/${domain}`, domainPayload(draft));
      await get().loadDomains();
      return true;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return false;
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
