import { create } from "zustand";
import type { CommandResult, AgentSoul, ContentAsset } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";
import { useAccountStore } from "./accountStore";

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
  clearCommandResult: () => void;
  loadCommandData: (accountId?: string) => Promise<void>;
  runCommand: (accountId: string) => Promise<void>;
  confirmCommand: (id: string) => Promise<void>;
  rejectCommand: (id: string) => Promise<void>;
}

// confirm/reject 端点返回形状（src/routes/management.rs:506-549）。
// 命中处理 → succeeded/failed/canceled；并发未命中 → already_processed_or_not_found。
interface ConfirmResponse {
  status: string;
  summary?: string;
  toolCalls?: CommandResult["toolCalls"];
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

  clearCommandResult: () => set({ commandResult: null }),

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

      // The operator may switch accounts while planning is in flight. Never
      // render a stale account's plan under the newly selected account.
      if (useAccountStore.getState().currentAccountId() === accountId) {
        set({ commandResult: data.command });
      }

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

  // 确认执行此前因高风险暂存（pending_confirmation）的命令。
  // 后端真执行已确认的计划，返回新 status + toolCalls，合进 commandResult。
  confirmCommand: async (id: string) => {
    if (!id) return;
    const command = get().commandResult;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    if (
      !command ||
      command.id !== id ||
      !command.accountId ||
      !command.planHash ||
      command.accountId !== currentAccountId
    ) {
      useUiStore.getState().setError("该执行计划不属于当前账号或缺少冻结标识，请重新生成计划");
      return;
    }
    set({ commandBusy: true });
    useUiStore.getState().setError("");
    try {
      const data = await api.post<ConfirmResponse>(
        `/api/management-agent/commands/${id}/confirm`,
        { accountId: command.accountId, planHash: command.planHash }
      );
      set((state) => {
        if (
          !state.commandResult ||
          state.commandResult.id !== id ||
          state.commandResult.accountId !== command.accountId ||
          state.commandResult.planHash !== command.planHash ||
          useAccountStore.getState().currentAccountId() !== command.accountId
        ) return {};
        return {
          commandResult: {
            ...state.commandResult,
            status: data.status,
            summary: data.summary ?? state.commandResult.summary,
            toolCalls: data.toolCalls ?? state.commandResult.toolCalls,
          },
        };
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      set({ commandBusy: false });
    }
  },

  // 否决此前暂存的命令：后端原子改 canceled，未执行任何工具。
  rejectCommand: async (id: string) => {
    if (!id) return;
    const command = get().commandResult;
    const currentAccountId = useAccountStore.getState().currentAccountId();
    if (
      !command ||
      command.id !== id ||
      !command.accountId ||
      !command.planHash ||
      command.accountId !== currentAccountId
    ) {
      useUiStore.getState().setError("该执行计划不属于当前账号或缺少冻结标识，请重新生成计划");
      return;
    }
    set({ commandBusy: true });
    useUiStore.getState().setError("");
    try {
      const data = await api.post<ConfirmResponse>(
        `/api/management-agent/commands/${id}/reject`,
        { accountId: command.accountId, planHash: command.planHash }
      );
      set((state) => {
        if (
          !state.commandResult ||
          state.commandResult.id !== id ||
          state.commandResult.accountId !== command.accountId ||
          state.commandResult.planHash !== command.planHash ||
          useAccountStore.getState().currentAccountId() !== command.accountId
        ) return {};
        return {
          commandResult: { ...state.commandResult, status: data.status },
        };
      });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      set({ commandBusy: false });
    }
  },
}));
