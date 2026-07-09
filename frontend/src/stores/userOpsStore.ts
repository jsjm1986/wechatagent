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
import { fetchSummary } from "../lib/inboxApi";
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
  // 判断条请示灯（Task 3）：统一收件箱里待本人裁决的请示数量（principalEscalation）。
  escalationPendingCount: number;

  // 表单/草稿
  importQuery: string;
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

  // 通讯录 roster 缓存（按 accountId 键控）：首次拉后缓存，进频道复用不重打 API；
  // 仅就绪结果落缓存（syncing 中不缓存，允许自动重拉覆盖）。force 才重拉。
  rosterCache: Record<string, { items: RosterEntry[]; syncing: boolean; fetchedAt: number }>;

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
  setImportQuery: (value: string) => void;
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
  loadMessages: (contact: Contact) => Promise<void>;
  loadEscalationCount: () => Promise<void>;
  loadPlaybooks: (accountId: string) => Promise<void>;
  loadContacts: (accountId: string) => Promise<void>;
  importContacts: () => Promise<void>;
  loadRoster: (accountId: string, opts?: { force?: boolean }) => Promise<{ items: RosterEntry[]; syncing: boolean }>;
  batchEnable: (payload: {
    accountId: string;
    candidates: { wxid: string; nickname?: string | null; remark?: string | null; avatarUrl?: string | null; sex?: number | null }[];
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
  saveManualTags: (tags: string[]) => Promise<void>;
  analyzeProfile: () => Promise<void>;
  previewGuideInstruction: (instruction: string) => Promise<void>;
  applyGuidePreview: () => Promise<UserOperationGuideApplyResult | null>;
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

  try {
    const params = [`accountId=${encodeURIComponent(currentAccountId)}`];
    const trimmed = q?.trim();
    if (trimmed) params.push(`q=${encodeURIComponent(trimmed)}`);
    const contactData = await api.get<{ items: Contact[] }>(`/api/contacts?${params.join("&")}`);
    useContactStore.getState().setContacts(contactData.items);
  } catch (error) {
    useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
  }
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
  escalationPendingCount: 0,

  profileNote: "",
  customAgentInstructions: "",
  assistOverride: "default",
  relationshipType: "",
  referredSpecialistAt: undefined,
  referredCardId: undefined,
  profileEditDraft: {},
  importQuery: "",
  searchQuery: "",
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

  rosterCache: {},

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
  setImportQuery: (value) => set({ importQuery: value }),
  setSearchQuery: (value) => set({ searchQuery: value }),
  setPlaybookDraft: (draft) => set({ playbookDraft: draft }),
  setGeneratePlaybookText: (text) => set({ generatePlaybookText: text }),
  setOptimizePlaybookText: (text) => set({ optimizePlaybookText: text }),
  setEditingPlaybookId: (id) => set({ editingPlaybookId: id }),
  setGuideBusy: (busy) => set({ guideBusy: busy }),
  setSimulationBusy: (busy) => set({ simulationBusy: busy }),
  setDomainDrafts: (drafts) => set({ domainDrafts: drafts }),
  setMemoryDraft: (patch) => set((s) => ({ memoryDraft: { ...s.memoryDraft, ...patch } })),

  // 选中联系人时同步状态
  hydrateSelected: (contact) => {
    set({
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
      guidePreview: null
    });
  },

  // 加载选中联系人的数据
  loadMessages: async (contact) => {
    const [messagesR, memoryR, candidateR, reviewsR, healthR] = await Promise.allSettled([
      api.get<{ items: Message[] }>(`/api/conversations/${contact.id}/messages?limit=50`),
      api.get<{ item: OperatingMemory }>(`/api/contacts/${contact.id}/operating-memory`),
      api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${contact.id}/memory-candidates?limit=30`),
      api.get<{ items: DecisionReview[] }>(`/api/decision-reviews?contactId=${contact.id}&limit=20`),
      api.get<OperationHealth>(`/api/contacts/${contact.id}/operation-health`),
    ]);
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
    // 判断条请示灯：与选中联系人数据并行加载（失败不阻断，优雅降级为 0）。
    void get().loadEscalationCount();
  },

  // 判断条请示灯：拉统一收件箱 summary，取待本人裁决的请示数（principalEscalation）。
  // 失败/字段缺失回落 0（不弹错、不渲染此 chip）——纯观测灯，不阻断驾驶舱。
  loadEscalationCount: async () => {
    try {
      const summary = await fetchSummary();
      const count = typeof summary.principalEscalation === "number" ? summary.principalEscalation : 0;
      set({ escalationPendingCount: count });
    } catch {
      set({ escalationPendingCount: 0 });
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

  // 加载（切账号 / 挂载时 / 搜索过滤时）联系人列表——用户运营页主体数据，
  // 复用模块内 refreshContacts，透传当前 searchQuery 作为 q 过滤。
  loadContacts: async (accountId) => {
    await refreshContacts(accountId || null, get().searchQuery);
  },

  // 搜索并导入好友：先 /search 拿只读候选，再 /import 真正写库，最后刷新列表。
  // 拆两步避免“搜索即改库”的误解（沿用后端 search/import 双路由语义）。
  importContacts: async () => {
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { importQuery, searchQuery } = get();
    if (!importQuery.trim() || !currentAccountId) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const search = await api.post<{ items: unknown[] }>("/api/contacts/search", {
        query: importQuery,
        accountId: currentAccountId
      });
      const candidates = search.items || [];
      if (candidates.length) {
        await api.post<{ items: Contact[] }>("/api/contacts/import", {
          accountId: currentAccountId,
          candidates
        });
      }
      set({ importQuery: "" });
      await refreshContacts(currentAccountId, searchQuery);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
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
    return await api.post<{ enabled: number; queued: number }>(
      "/api/contacts/batch-enable",
      payload
    );
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

  saveAssistOverride: async () => {
    const selected = useContactStore.getState().selected;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const { assistOverride } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/assist-override`, {
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
    const { relationshipType, profileEditDraft } = get();

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/operation-profile`, {
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

    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      // A2：把扁平 23 字段 memoryDraft 归组成后端要求的四个嵌套 Document
      // （userUnderstanding/relationshipState/productFit/nextAction）后整体 $set。
      await api.put(`/api/contacts/${selected.id}/operating-memory`, groupMemoryDraft(memoryDraft));
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

  saveManualTags: async (tags: string[]) => {
    // 标签可信度改造 - 运营录入层：保存运营录入标签，后端权威覆盖。
    const currentAccountId = useAccountStore.getState().currentAccountId();
    const selected = useContactStore.getState().selected;
    if (!selected) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/contacts/${selected.id}/manual-tags`, { tags });
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

      const next: Partial<UserOpsState> = { guidePreview: data.item };
      // FE-1：直接用后端构建好的 health（scores + canonical 7 项 items，与正常加载
      // 路径 /operation-health 同口径、同形态），不再用前端 healthFromScores 拿
      // healthScores 重建 4-key 占位分（旧函数 key 与后端零交集，展示伪造分）。
      if (data.item.health && data.item.health.items.length > 0) {
        next.operationHealth = data.item.health;
      }
      set(next);
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

    if (!selected || !guidePreview) return null;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const data = await api.post<{ item: UserOperationGuideApplyResult }>(
        "/api/user-operations/guide/apply",
        { previewId: guidePreview.id }
      );

      set({
        operatingMemory: data.item.operatingMemory,
        guidePreview: null,
        operationHealth: data.item.health
      });

      await refreshContacts(currentAccountId);
      return data.item;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return null;
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