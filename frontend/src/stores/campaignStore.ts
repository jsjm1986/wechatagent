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

interface CampaignState {
  selectedCampaignId: string | null;
  report: CampaignReport | null;
  loading: boolean;
  lastAttemptedId: string | null;
  openReport: (id: string) => void;
  loadReport: (id: string) => Promise<void>;
  clear: () => void;
}

export const useCampaignStore = create<CampaignState>((set, get) => ({
  selectedCampaignId: null,
  report: null,
  loading: false,
  lastAttemptedId: null,
  openReport: (id) => {
    set({ selectedCampaignId: id, report: null });
    useNavigationStore.getState().setChannel("campaign");
    void get().loadReport(id);
  },
  loadReport: async (id) => {
    set({ loading: true, lastAttemptedId: id });
    try {
      const r = await api.get<CampaignReport>(`/api/campaigns/${id}/sends`);
      set({ report: r });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    } finally {
      set({ loading: false });
    }
  },
  clear: () => set({ selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null }),
}));
