import { create } from "zustand";
import type {
  AgentRunItem,
  DecisionReview,
  EventItem,
  LlmUsageResponse,
  OpsTab,
  TaskItem,
} from "../types";
import { api } from "../lib/api";
import { useAccountStore } from "./accountStore";
import { useUiStore } from "./uiStore";

interface OperationsState {
  events: EventItem[];
  tasks: TaskItem[];
  decisionReviews: DecisionReview[];
  llmUsage: LlmUsageResponse | null;
  agentRuns: AgentRunItem[];
  dataAccountId: string;
  requestGeneration: number;
  agentRunsGeneration: number;
  loading: boolean;
  opsTab: OpsTab;
  setOpsTab: (tab: OpsTab) => void;
  loadOperationsData: (accountId: string) => Promise<void>;
  loadAgentRuns: (accountId: string) => Promise<void>;
  reviewTaskNow: (task: TaskItem, pageAccountId: string) => Promise<void>;
  cancelTask: (task: TaskItem, pageAccountId: string) => Promise<void>;
}

function currentAccountId(): string {
  return useAccountStore.getState().currentAccountId();
}

function emptyData() {
  return {
    events: [] as EventItem[],
    tasks: [] as TaskItem[],
    decisionReviews: [] as DecisionReview[],
    llmUsage: null as LlmUsageResponse | null,
    agentRuns: [] as AgentRunItem[],
  };
}

export const useOperationsStore = create<OperationsState>((set, get) => {
  function taskActionIsCurrent(task: TaskItem, pageAccountId: string): boolean {
    const state = get();
    return Boolean(pageAccountId)
      && currentAccountId() === pageAccountId
      && state.dataAccountId === pageAccountId
      && task.accountId === pageAccountId
      && state.tasks.some(
        (candidate) => candidate.id === task.id && candidate.accountId === task.accountId,
      );
  }

  async function runTaskAction(
    task: TaskItem,
    pageAccountId: string,
    action: "review-now" | "cancel",
  ): Promise<void> {
    if (!taskActionIsCurrent(task, pageAccountId)) return;
    useUiStore.getState().setError("");
    try {
      await api.post(`/api/agent-tasks/${task.id}/${action}`, {
        expectedAccountId: task.accountId,
      });
      if (currentAccountId() === pageAccountId) {
        await get().loadOperationsData(pageAccountId);
      }
    } catch (error) {
      if (currentAccountId() === pageAccountId) {
        useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      }
    }
  }

  return {
    ...emptyData(),
    dataAccountId: "",
    requestGeneration: 0,
    agentRunsGeneration: 0,
    loading: false,
    opsTab: "tasks",

    setOpsTab: (tab) => set({ opsTab: tab }),

    loadOperationsData: async (accountId) => {
      const generation = get().requestGeneration + 1;
      set({
        ...emptyData(),
        dataAccountId: accountId,
        requestGeneration: generation,
        loading: Boolean(accountId),
      });
      if (!accountId || currentAccountId() !== accountId) {
        set({ loading: false });
        return;
      }
      const accountParam = `accountId=${encodeURIComponent(accountId)}`;
      try {
        const [eventsRes, tasksRes, reviewsRes, llmUsageRes, agentRunsRes] = await Promise.all([
          api.get<{ items?: EventItem[] }>(`/api/events?${accountParam}`),
          api.get<{ items?: TaskItem[] }>(`/api/tasks?${accountParam}`),
          api.get<{ items?: DecisionReview[] }>(`/api/decision-reviews?${accountParam}`),
          api.get<LlmUsageResponse>(`/api/llm-usage?${accountParam}`),
          api.get<{ items?: AgentRunItem[] }>(`/api/agent-runs?${accountParam}`),
        ]);
        if (
          get().requestGeneration !== generation
          || get().dataAccountId !== accountId
          || currentAccountId() !== accountId
        ) return;
        const tasks = tasksRes.items ?? [];
        if (tasks.some((task) => task.accountId !== accountId)) {
          set({ ...emptyData(), loading: false });
          useUiStore.getState().setError("任务响应账号与当前账号不一致，已拒绝显示。");
          return;
        }
        set({
          events: eventsRes.items ?? [],
          tasks,
          decisionReviews: reviewsRes.items ?? [],
          llmUsage: llmUsageRes,
          agentRuns: agentRunsRes.items ?? [],
        });
      } catch (error) {
        if (
          get().requestGeneration === generation
          && get().dataAccountId === accountId
          && currentAccountId() === accountId
        ) {
          set(emptyData());
          useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (
          get().requestGeneration === generation
          && get().dataAccountId === accountId
          && currentAccountId() === accountId
        ) {
          set({ loading: false });
        }
      }
    },

    loadAgentRuns: async (accountId) => {
      const generation = get().agentRunsGeneration + 1;
      set({ agentRuns: [], agentRunsGeneration: generation });
      if (!accountId || currentAccountId() !== accountId) return;
      try {
        const res = await api.get<{ items?: AgentRunItem[] }>(
          `/api/agent-runs?accountId=${encodeURIComponent(accountId)}`,
        );
        if (
          get().agentRunsGeneration === generation
          && get().dataAccountId === accountId
          && currentAccountId() === accountId
        ) {
          set({ agentRuns: res.items ?? [] });
        }
      } catch (error) {
        if (
          get().agentRunsGeneration === generation
          && currentAccountId() === accountId
        ) {
          set({ agentRuns: [] });
          useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
        }
      }
    },

    reviewTaskNow: (task, pageAccountId) =>
      runTaskAction(task, pageAccountId, "review-now"),
    cancelTask: (task, pageAccountId) =>
      runTaskAction(task, pageAccountId, "cancel"),
  };
});
