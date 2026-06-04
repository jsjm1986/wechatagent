import { create } from "zustand";
import type { AgentSoul, PromptTemplate, PromptTemplateDraft } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

interface StrategyState {
  souls: AgentSoul[];
  promptTemplates: PromptTemplate[];
  soulDraft: { agentKind: string; name: string; content: string };
  editingSoulId: string;
  promptDraft: PromptTemplateDraft;
  editingPromptId: string;
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
}));