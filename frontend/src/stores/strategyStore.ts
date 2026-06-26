import { create } from "zustand";
import type { AgentSoul, PromptTemplate, PromptTemplateDraft, DomainProfile, DomainProfileDraft } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

// Task 8（路径B）：prompt 编辑保存的三态结果。后端 Task 6.6 第三闸：
// - Pass / force → {ok:true}（200）
// - NeedsHumanConfirm → 200 body {status:"needs_human_confirm", reason, diff}（非错误，必须从返回体读，不能静默当成功）
// - Reject → 4xx，api 抛 Error，message 含「红线语义审查拒绝」
// store 保持无 UI 依赖：只翻译成结构化结果返回，弹框交给组件层。
export type SavePromptResult =
  | { ok: true }
  | { needsConfirm: true; reason: string; diff: string }
  | { rejected: true; reason: string }
  | { error: true; reason: string };

interface StrategyState {
  souls: AgentSoul[];
  promptTemplates: PromptTemplate[];
  soulDraft: { agentKind: string; name: string; content: string };
  editingSoulId: string;
  promptDraft: PromptTemplateDraft;
  editingPromptId: string;
  // ── DomainProfile ───────────────────────────────────────────────────────
  domainProfiles: DomainProfile[];
  editingProfile: DomainProfile | null;
  // D5：手动新建空白配置时 editingProfile 仍为 null（尚无 id），用此标志让右侧编辑器
  // 在「新建态」也渲染（否则 editing===null 永远只渲染占位 → 死链路）。
  isCreatingProfile: boolean;
  profileDraft: DomainProfileDraft;
  profileTab: "list" | "generate";
  generating: boolean;
  generateError: string;
  generateResult: { id: string; profileId: string } | null;
}

interface StrategyActions {
  setSoulDraft: (draft: { agentKind: string; name: string; content: string }) => void;
  setPromptDraft: (draft: PromptTemplateDraft) => void;
  loadStrategyData: () => Promise<void>;
  createSoul: () => Promise<void>;
  saveSoul: () => Promise<void>;
  publishSoul: (id: string) => Promise<void>;
  createPromptTemplate: () => Promise<void>;
  savePromptTemplate: (force?: boolean) => Promise<SavePromptResult>;
  publishPromptTemplate: (id: string) => Promise<void>;
  resetSystemPromptPack: () => Promise<void>;
  editSoul: (soul: AgentSoul) => void;
  newSoulDraftFor: (kind: string) => void;
  editPromptTemplate: (template: PromptTemplate) => void;
  newPromptDraftFor: (kind: string) => void;
  // ── DomainProfile ───────────────────────────────────────────────────────
  loadDomainProfiles: () => Promise<void>;
  generateDomainProfile: (businessDescription: string, profileId: string, displayName?: string) => Promise<void>;
  selectProfileTab: (tab: "list" | "generate") => void;
  editDomainProfile: (profile: DomainProfile) => void;
  newDomainProfileDraft: () => void;
  setProfileDraft: (draft: DomainProfileDraft) => void;
  saveDomainProfile: () => Promise<void>;
  publishDomainProfile: (id: string) => Promise<{ id: string; riskyFields: string[] } | null>;
  confirmRiskyActivation: (id: string) => Promise<void>;
  activateDomainProfile: (id: string) => Promise<void>;
  deleteDomainProfile: (id: string) => Promise<void>;
}

// 辅助函数
function emptyPromptTemplateDraft(): PromptTemplateDraft {
  return {
    promptKey: "",
    agentKind: "user",
    layer: "task_template",
    title: "",
    description: "",
    content: ""
  };
}

function promptPayload(draft: PromptTemplateDraft, force?: boolean) {
  return {
    promptKey: draft.promptKey,
    agentKind: draft.agentKind,
    layer: draft.layer,
    title: draft.title,
    description: draft.description || undefined,
    content: draft.content,
    // force=true：管理者已逐字核对，覆盖 LLM 红线语义审查（字面双闸仍跑）。
    ...(force ? { force: true } : {})
  };
}

function newPromptDraft(set: (fn: (state: StrategyState) => Partial<StrategyState>) => void) {
  set(() => ({
    editingPromptId: "",
    promptDraft: emptyPromptTemplateDraft()
  }));
}

export const useStrategyStore = create<StrategyState & StrategyActions>((set, get) => ({
  souls: [],
  promptTemplates: [],
  soulDraft: { agentKind: "user", name: "", content: "" },
  editingSoulId: "",
  promptDraft: emptyPromptTemplateDraft(),
  editingPromptId: "",
  // DomainProfile initial state
  domainProfiles: [],
  editingProfile: null,
  isCreatingProfile: false,
  profileDraft: {},
  profileTab: "list",
  generating: false,
  generateError: "",
  generateResult: null,

  setSoulDraft: (draft) => set({ soulDraft: draft }),
  setPromptDraft: (draft) => set({ promptDraft: draft }),

  loadStrategyData: async () => {
    try {
      const [soulsResponse, promptsResponse] = await Promise.all([
        api.get<{ items: AgentSoul[] }>("/api/agent-souls"),
        api.get<{ items: PromptTemplate[] }>("/api/prompt-templates")
      ]);
      set({
        souls: soulsResponse.items,
        promptTemplates: promptsResponse.items
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  createSoul: async () => {
    const { soulDraft } = get();
    if (!soulDraft.name.trim() || !soulDraft.content.trim()) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post("/api/agent-souls", soulDraft);
      set({
        soulDraft: { ...soulDraft, name: "", content: "" }
      });
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  saveSoul: async () => {
    const { editingSoulId, soulDraft } = get();
    if (!editingSoulId || !soulDraft.name.trim() || !soulDraft.content.trim()) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/agent-souls/${editingSoulId}`, soulDraft);
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  publishSoul: async (id: string) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/agent-souls/${id}/publish`);
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  createPromptTemplate: async () => {
    const { promptDraft } = get();
    if (!promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post("/api/prompt-templates", promptPayload(promptDraft));
      newPromptDraft(set);
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  savePromptTemplate: async (force?: boolean): Promise<SavePromptResult> => {
    const { editingPromptId, promptDraft } = get();
    if (!editingPromptId || !promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()) {
      return { error: true, reason: "缺少必要字段" };
    }

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      const resp = await api.put<{ status?: string; reason?: string; diff?: string }>(
        `/api/prompt-templates/${editingPromptId}`,
        promptPayload(promptDraft, force)
      );
      // 真 bug 修复：needs_human_confirm 是 200，不能当成功 reload。
      if (resp && resp.status === "needs_human_confirm") {
        return { needsConfirm: true, reason: resp.reason ?? "", diff: resp.diff ?? "" };
      }
      await get().loadStrategyData();
      return { ok: true };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      // Reject：后端 4xx body {error:"红线语义审查拒绝：…"} → api 抛 Error(message)。
      // 交组件层弹「逐字核对 + force 覆盖」，不进全局 setError（否则与普通错误混淆）。
      if (message.includes("红线语义审查拒绝")) {
        return { rejected: true, reason: message };
      }
      useUiStore.getState().setError(message);
      return { error: true, reason: message };
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  publishPromptTemplate: async (id: string) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post(`/api/prompt-templates/${id}/publish`);
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  resetSystemPromptPack: async () => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post("/api/prompt-templates/reset-system-pack");
      set({
        editingPromptId: "",
        promptDraft: emptyPromptTemplateDraft()
      });
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  editSoul: (soul: AgentSoul) => {
    set({
      editingSoulId: soul.id,
      soulDraft: {
        agentKind: soul.agentKind,
        name: soul.name,
        content: soul.content
      }
    });
  },

  newSoulDraftFor: (agentKind: string) => {
    set({
      editingSoulId: "",
      soulDraft: { agentKind, name: "", content: "" }
    });
  },

  editPromptTemplate: (template: PromptTemplate) => {
    set({
      editingPromptId: template.id,
      promptDraft: {
        promptKey: template.promptKey,
        agentKind: template.agentKind,
        layer: template.layer,
        title: template.title,
        description: template.description ?? "",
        content: template.content
      }
    });
  },

  newPromptDraftFor: (agentKind: string) => {
    set({
      editingPromptId: "",
      promptDraft: { ...emptyPromptTemplateDraft(), agentKind }
    });
  },

  // ── DomainProfile actions ────────────────────────────────────────────────

  loadDomainProfiles: async () => {
    try {
      const data = await api.get<{ items: DomainProfile[] }>("/api/admin/domain-profiles");
      set({ domainProfiles: data.items ?? [] });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  selectProfileTab: (tab) => set({ profileTab: tab, generateError: "", generateResult: null }),

  generateDomainProfile: async (businessDescription: string, profileId: string, displayName?: string) => {
    if (!businessDescription.trim() || !profileId.trim()) return;
    set({ generating: true, generateError: "", generateResult: null });
    useUiStore.getState().setBusy(true);
    try {
      const payload: Record<string, string> = { businessDescription, profileId };
      if (displayName) payload.displayName = displayName;
      const result = await api.post<{ id: string; profileId: string }>("/api/admin/domain-profiles/generate", payload);
      set({ generating: false, generateResult: { id: result.id, profileId: result.profileId } });
      await get().loadDomainProfiles();
    } catch (error) {
      set({ generating: false, generateError: error instanceof Error ? error.message : String(error) });
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  editDomainProfile: (profile: DomainProfile) => {
    set({
      editingProfile: profile,
      isCreatingProfile: false,
      profileDraft: {
        profile_id: profile.profile_id,
        display_name: profile.display_name,
        description: profile.description,
        profile_dimensions: profile.profile_dimensions,
        prompt_fragment: profile.prompt_fragment,
        conversation_modes: profile.conversation_modes,
        business_formulas: profile.business_formulas,
        commitment_markers: profile.commitment_markers,
        coverage_dimensions: profile.coverage_dimensions,
        threshold_overrides: profile.threshold_overrides ?? undefined,
        methodology_generator_preamble: profile.methodology_generator_preamble ?? undefined,
        soul_override: profile.soul_override ?? undefined,
        methodology_override: profile.methodology_override ?? undefined,
        conversation_mode_policy: profile.conversation_mode_policy ?? undefined,
        stagnation_dimension: profile.stagnation_dimension ?? undefined,
        grounding_gate_bypass_without_claim: profile.grounding_gate_bypass_without_claim ?? undefined,
        distrust_self_reported_low_risk: profile.distrust_self_reported_low_risk ?? undefined,
        chunk_roles: profile.chunk_roles ?? undefined,
        memory_dimensions: profile.memory_dimensions ?? undefined,
        outcome_polarity: profile.outcome_polarity ?? undefined,
        operation_mode: profile.operation_mode ?? undefined,
        transaction_facts_enabled: profile.transaction_facts_enabled ?? undefined,
        reviewer_orientation: profile.reviewer_orientation ?? undefined,
        mode_gate_policy_override: profile.mode_gate_policy_override ?? undefined,
        trajectory_dimensions: profile.trajectory_dimensions ?? undefined,
        debounce_window_ms_override: profile.debounce_window_ms_override ?? undefined,
        per_relationship_operation_mode: profile.per_relationship_operation_mode ?? undefined,
      }
    });
  },

  newDomainProfileDraft: () => {
    set({
      editingProfile: null,
      isCreatingProfile: true,
      // 最小合法空白 draft：profile_id/display_name 空串占位，保证编辑器渲染这些输入框
      // （而非 {} 时部分 ?? "" 仍可工作但语义不清）。create POST 时由 saveDomainProfile
      // 显式注入顶层 camelCase profileId（后端 UpsertRequest 读 profileId，非内层 profile_id）。
      profileDraft: { profile_id: "", display_name: "" }
    });
  },

  setProfileDraft: (draft: DomainProfileDraft) => set({ profileDraft: draft }),

  saveDomainProfile: async () => {
    const { editingProfile, profileDraft } = get();
    useUiStore.getState().setBusy(true);
    try {
      if (editingProfile?.id) {
        // update：PUT 到已有 id。后端 update 不消费顶层 profileId（只用 existing.profile_id），
        // 直接发 DomainProfileDraft（snake_case，flatten 进 profile）即可。
        await api.put(`/api/admin/domain-profiles/${editingProfile.id}`, profileDraft);
      } else {
        // create：POST 无 id。后端 UpsertRequest 顶层读 camelCase `profileId`（rename），
        // DomainProfileDraft 只有 snake_case profile_id（会 flatten 进内层 profile），
        // 故必须显式补顶层 profileId，否则 create 因 profileId 为空被 400。
        await api.post(`/api/admin/domain-profiles`, {
          ...profileDraft,
          profileId: profileDraft.profile_id ?? ""
        });
      }
      await get().loadDomainProfiles();
      // 保存后清新建/编辑态，回到占位（与列表刷新一致）。
      set({ editingProfile: null, isCreatingProfile: false });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  publishDomainProfile: async (id: string) => {
    useUiStore.getState().setBusy(true);
    try {
      const resp = await api.post<{
        ok: boolean;
        pendingActivation?: boolean;
        riskyFields?: string[];
        id: string;
      }>(`/api/admin/domain-profiles/${id}/publish`, {});
      await get().loadDomainProfiles();
      // 危险开关变更：后端落旁路稿不即时生效，返回 pendingActivation + 变更字段，
      // 交调用方二次确认后调 confirmRiskyActivation 才生效。普通变更返回 null。
      if (resp.pendingActivation) {
        return { id: resp.id, riskyFields: resp.riskyFields ?? [] };
      }
      return null;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  // 危险开关二次确认：对 publish 落的旁路稿调 rollout（推 current + demote + realign），
  // 让新版本真正生效。复用 rollout 端点（无前置状态校验，旁路稿 current=false 适用）。
  confirmRiskyActivation: async (id: string) => {
    useUiStore.getState().setBusy(true);
    try {
      await api.post(`/api/admin/domain-profiles/${id}/rollout`, {});
      await get().loadDomainProfiles();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  activateDomainProfile: async (id: string) => {
    useUiStore.getState().setBusy(true);
    try {
      await api.post(`/api/admin/domain-profiles/${id}/activate`, {});
      await get().loadDomainProfiles();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  deleteDomainProfile: async (id: string) => {
    if (!window.confirm("确认删除该行业配置？")) return;
    useUiStore.getState().setBusy(true);
    try {
      await api.delete(`/api/admin/domain-profiles/${id}`);
      set({ editingProfile: null });
      await get().loadDomainProfiles();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
}));