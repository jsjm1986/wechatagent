import { create } from "zustand";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

export type SendStatRow = {
  targetId: string;
  targetTitle: string;
  sentCount: number;
  contactCount: number;
  responseRate: number;
  stageAdvanceRate: number;
};
export type SendOverview = {
  totalSends: number;
  responseRate: number;
  stageAdvanceRate: number;
};

interface SendAnalyticsState {
  overview: SendOverview | null;
  mediaStats: SendStatRow[];
  namecardStats: SendStatRow[];
  loadOverview: (accountId: string) => Promise<void>;
  loadStats: (kind: "media" | "namecard", accountId: string) => Promise<void>;
}

export const useSendAnalyticsStore = create<SendAnalyticsState>((set) => ({
  overview: null,
  mediaStats: [],
  namecardStats: [],
  loadOverview: async (accountId) => {
    if (!accountId) {
      set({ overview: null });
      return;
    }
    try {
      const r = await api.get<SendOverview>(
        `/api/send-ledger/overview?accountId=${encodeURIComponent(accountId)}`
      );
      set({ overview: r });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    }
  },
  loadStats: async (kind, accountId) => {
    if (!accountId) {
      if (kind === "media") set({ mediaStats: [] });
      else set({ namecardStats: [] });
      return;
    }
    try {
      const params = new URLSearchParams({ kind, accountId });
      const r = await api.get<{ items: SendStatRow[] }>(`/api/send-ledger/stats?${params}`);
      if (kind === "media") set({ mediaStats: r.items });
      else set({ namecardStats: r.items });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    }
  },
}));
