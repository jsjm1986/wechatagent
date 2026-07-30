import { create } from "zustand";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";
import { useNavigationStore } from "./navigationStore";

export interface CampaignSummary {
  targetCount: number;
  sent: number;
  pending: number;
  skipped: number;
  unknown: number;
  blocked: Record<string, number>;
  canceled: Record<string, number>;
  escalated: Record<string, number>;
}

export interface CampaignSendItem {
  contactWxid: string;
  name: string;
  status: string;
  reason?: string;
}

export interface CampaignReport {
  campaignId: string;
  title: string;
  status: string;
  summary: CampaignSummary;
  items: CampaignSendItem[];
}

export interface CampaignListItem {
  campaignId: string;
  title: string;
  status: string;
  targetCount?: number;
  dispatchedCount: number;
  createdBy: string;
  createdAt?: string;
}

interface CampaignState {
  selectedCampaignId: string | null;
  report: CampaignReport | null;
  loading: boolean;
  lastAttemptedId: string | null;
  reportRequestGeneration: number;
  view: "list" | "create" | "board";
  campaigns: CampaignListItem[];
  listLoading: boolean;
  listLoaded: boolean;
  page: number;
  openReport: (id: string) => void;
  loadReport: (id: string) => Promise<void>;
  setView: (v: "list" | "create" | "board") => void;
  loadCampaigns: () => Promise<void>;
  setPage: (n: number) => void;
  clear: () => void;
}

export const useCampaignStore = create<CampaignState>((set, get) => ({
  selectedCampaignId: null,
  report: null,
  loading: false,
  lastAttemptedId: null,
  reportRequestGeneration: 0,
  view: "list",
  campaigns: [],
  listLoading: false,
  listLoaded: false,
  page: 0,
  openReport: (id) => {
    set({ selectedCampaignId: id, report: null, view: "board", page: 0 });
    useNavigationStore.getState().setChannel("campaign");
    void get().loadReport(id);
  },
  loadReport: async (id) => {
    const generation = get().reportRequestGeneration + 1;
    set((state) => ({
      loading: true,
      lastAttemptedId: id,
      reportRequestGeneration: generation,
      report: state.report?.campaignId === id ? state.report : null,
    }));
    try {
      const r = await api.get<CampaignReport>(`/api/campaigns/${id}/sends`);
      const current = get();
      if (
        current.reportRequestGeneration === generation &&
        current.selectedCampaignId === id &&
        r.campaignId === id
      ) {
        set({ report: r });
      }
    } catch (e) {
      const current = get();
      if (current.reportRequestGeneration === generation && current.selectedCampaignId === id) {
        useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      const current = get();
      if (current.reportRequestGeneration === generation && current.selectedCampaignId === id) {
        set({ loading: false });
      }
    }
  },
  setView: (v) => set({ view: v }),
  loadCampaigns: async () => {
    set({ listLoading: true, listLoaded: true });
    try {
      const r = await api.get<{ items: CampaignListItem[] }>("/api/campaigns");
      set({ campaigns: r.items });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    } finally {
      set({ listLoading: false });
    }
  },
  setPage: (n) => set({ page: n }),
  clear: () => set((state) => ({ selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null, reportRequestGeneration: state.reportRequestGeneration + 1, view: "list", campaigns: [], listLoading: false, listLoaded: false, page: 0 })),
}));
