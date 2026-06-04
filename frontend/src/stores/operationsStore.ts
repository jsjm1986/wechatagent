import { create } from "zustand";
import type { EventItem, TaskItem, DecisionReview, LlmUsageResponse, OpsTab } from "../types";
import { api } from "../lib/api";

interface OperationsState {
  events: EventItem[];
  tasks: TaskItem[];
  decisionReviews: DecisionReview[];
  llmUsage: LlmUsageResponse | null;
  opsTab: OpsTab;
  setOpsTab: (tab: OpsTab) => void;
  loadOperationsData: (accountId?: string) => Promise<void>;
}

export const useOperationsStore = create<OperationsState>((set) => ({
  events: [],
  tasks: [],
  decisionReviews: [],
  llmUsage: null,
  opsTab: "tasks",

  setOpsTab: (tab: OpsTab) => set({ opsTab: tab }),

  loadOperationsData: async (accountId?: string) => {
    const accountParam = accountId ? `accountId=${encodeURIComponent(accountId)}` : "";

    try {
      // 并行加载所有数据
      const [eventsRes, tasksRes, reviewsRes, llmUsageRes] = await Promise.all([
        api.get(`/api/events${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/tasks${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/decision-reviews${accountParam ? `?${accountParam}` : ""}`),
        api.get(`/api/llm-usage${accountParam ? `?${accountParam}` : ""}`),
      ]);

      set({
        events: (eventsRes as any).items || [],
        tasks: (tasksRes as any).items || [],
        decisionReviews: (reviewsRes as any).items || [],
        llmUsage: llmUsageRes as LlmUsageResponse | null,
      });
    } catch (error) {
      console.error("Failed to load operations data:", error);
      // 设置空数据以避免界面错误
      set({
        events: [],
        tasks: [],
        decisionReviews: [],
        llmUsage: null,
      });
    }
  },
}));