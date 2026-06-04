import { create } from "zustand";
import type { CommandResult, AgentSoul, ContentAsset } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

interface CommandState {
  commandDraft: string;
  commandResult: CommandResult | null;
  commandDryRun: boolean;
  commandBusy: boolean;
  souls: AgentSoul[];
  assets: ContentAsset[];
  pendingTasks: number;
}

interface CommandActions {
  setCommandDraft: (value: string) => void;
  setCommandDryRun: (value: boolean) => void;
  loadCommandData: (accountId?: string) => Promise<void>;
  runCommand: (accountId: string) => Promise<void>;
}

export const useCommandStore = create<CommandState & CommandActions>((set, get) => ({
  commandDraft: "把 AI应用开发 加入 Agent 运营列表，并生成一份克制、专业的运营备注",
  commandResult: null,
  commandDryRun: true,
  commandBusy: false,
  souls: [],
  assets: [],
  pendingTasks: 0,

  setCommandDraft: (value: string) => set({ commandDraft: value }),

  setCommandDryRun: (value: boolean) => set({ commandDryRun: value }),

  loadCommandData: async (accountId?: string) => {
    try {
      const accountParam = accountId ? `accountId=${accountId}` : "";
      const [assetsRes, soulsRes, tasksRes] = await Promise.all([
        api.get<{ items: ContentAsset[] }>(`/api/content-assets${accountParam ? `?${accountParam}` : ""}`),
        api.get<{ items: AgentSoul[] }>("/api/agent-souls"),
        api.get<{ items: { status: string }[] }>(`/api/tasks${accountParam ? `?${accountParam}` : ""}`),
      ]);

      const pendingCount = tasksRes.items.filter(task => task.status === "pending").length;

      set({
        assets: assetsRes.items,
        souls: soulsRes.items,
        pendingTasks: pendingCount
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  runCommand: async (accountId: string) => {
    const { commandDraft, commandDryRun } = get();
    if (!accountId || !commandDraft.trim()) return;

    set({ commandBusy: true });
    useUiStore.getState().setError("");

    try {
      // 创建 session
      const session = await api.post<{ id: string }>("/api/management-agent/sessions", {
        accountId,
        title: commandDraft.slice(0, 40),
        dryRun: commandDryRun
      });

      // 发送消息
      const data = await api.post<{ command: CommandResult }>(
        `/api/management-agent/sessions/${session.id}/messages`,
        {
          accountId,
          content: commandDraft,
          dryRun: commandDryRun
        }
      );

      set({ commandResult: data.command });

      // 重新加载 tasks 来更新 pendingTasks
      const accountParam = `accountId=${accountId}`;
      const tasksRes = await api.get<{ items: { status: string }[] }>(`/api/tasks?${accountParam}`);
      const pendingCount = tasksRes.items.filter(task => task.status === "pending").length;
      set({ pendingTasks: pendingCount });

    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      set({ commandBusy: false });
    }
  },
}));