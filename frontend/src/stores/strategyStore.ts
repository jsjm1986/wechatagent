import { create } from "zustand";
import type { AgentSoul, PromptTemplate, PromptTemplateDraft, DomainProfile, DomainProfileDraft } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

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
  savePromptTemplate: () => Promise<void>;
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
  saveDomainProfile: (id: string) => Promise<void>;
  publishDomainProfile: (id: string) => Promise<void>;
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

function promptPayload(draft: PromptTemplateDraft) {
  return {
    promptKey: draft.promptKey,
    agentKind: draft.agentKind,
    layer: draft.layer,
    title: draft.title,
    description: draft.description || undefined,
    content: draft.content
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

  savePromptTemplate: async () => {
    const { editingPromptId, promptDraft } = get();
    if (!editingPromptId || !promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.put(`/api/prompt-templates/${editingPromptId}`, promptPayload(promptDraft));
      await get().loadStrategyData();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
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
        methodology_generator_preamble: profile.methodology_generator_preamble ?? undefined,
      }
    });
  },

  newDomainProfileDraft: () => {
    set({
      editingProfile: null,
      profileDraft: {}
    });
  },

  setProfileDraft: (draft: DomainProfileDraft) => set({ profileDraft: draft }),

  saveDomainProfile: async (id: string) => {
    const { profileDraft } = get();
    useUiStore.getState().setBusy(true);
    try {
      await api.put(`/api/admin/domain-profiles/${id}`, profileDraft);
      await get().loadDomainProfiles();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  publishDomainProfile: async (id: string) => {
    useUiStore.getState().setBusy(true);
    try {
      await api.post(`/api/admin/domain-profiles/${id}/publish`, {});
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