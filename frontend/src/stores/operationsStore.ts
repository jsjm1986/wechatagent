import { create } from "zustand";
import type { EventItem, TaskItem, DecisionReview, LlmUsageResponse, OpsTab, AgentRunItem } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

interface OperationsState {
  events: EventItem[];
  tasks: TaskItem[];
  decisionReviews: DecisionReview[];
  llmUsage: LlmUsageResponse | null;
  agentRuns: AgentRunItem[];
  loading: boolean;
  opsTab: OpsTab;
  setOpsTab: (tab: OpsTab) => void;
  loadOperationsData: (accountId?: string) => Promise<void>;
  loadAgentRuns: (accountId?: string) => Promise<void>;
}

export const useOperationsStore = create<OperationsState>((set) => ({
  events: [],
  tasks: [],
  decisionReviews: [],
  llmUsage: null,
  agentRuns: [],
  loading: false,
  opsTab: "tasks",

  setOpsTab: (tab: OpsTab) => set({ opsTab: tab }),

  loadOperationsData: async (accountId?: string) => {
    const accountParam = accountId ? `accountId=${encodeURIComponent(accountId)}` : "";

    set({ loading: true });
    try {
      // 并行加载所有数据（agent-runs 与 events 同处一次加载，确保 run envelope 视图就绪）
      const [eventsRes, tasksRes, reviewsRes, llmUsageRes, agentRunsRes] = await Promise.all([
        api.get(`/api/events${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/tasks${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/decision-reviews${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/llm-usage${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/agent-runs${accountParam ? `?${accountParam}` : ""}`),
      ]);

      set({
        events: (eventsRes as any).items || [],
        tasks: (tasksRes as any).items || [],
        decisionReviews: (reviewsRes as any).items || [],
        llmUsage: llmUsageRes as LlmUsageResponse | null,
        agentRuns: (agentRunsRes as any).items || [],
      });
    } catch (error) {
      console.error("Failed to load operations data:", error);
      useUiStore.getState().setError(
        error instanceof Error ? error.message : String(error),
      );
      // 设置空数据以避免界面错误（错误横幅负责区分错误态，置空保持渲染不崩）
      set({
        events: [],
        tasks: [],
        decisionReviews: [],
        llmUsage: null,
        agentRuns: [],
      });
    } finally {
      set({ loading: false });
    }
  },

  // C6：独立拉取 Agent 运行日志（run envelope）。视图 tab 切换时可单独刷新，
  // 不必重拉整个 operations 数据集。失败同样上报全局错误横幅、置空保持渲染不崩。
  loadAgentRuns: async (accountId?: string) => {
    const accountParam = accountId ? `accountId=${encodeURIComponent(accountId)}` : "";
    try {
      const res = await api.get(`/api/agent-runs${accountParam ? `?${accountParam}` : ""}`);
      set({ agentRuns: (res as any).items || [] });
    } catch (error) {
      console.error("Failed to load agent runs:", error);
      useUiStore.getState().setError(
        error instanceof Error ? error.message : String(error),
      );
      set({ agentRuns: [] });
    }
  },
}));
