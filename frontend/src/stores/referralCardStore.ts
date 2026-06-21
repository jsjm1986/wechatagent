import { create } from "zustand";
import type { ReferralCard, ReferralCardDraft } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

const emptyDraft: ReferralCardDraft = {
  displayName: "",
  targetWxid: "",
  sendTriggerHint: "",
  targetStages: ""
};

interface ReferralCardState {
  cards: ReferralCard[];
  cardDraft: ReferralCardDraft;
}

interface ReferralCardActions {
  setCardDraft: (draft: ReferralCardDraft) => void;
  loadCards: () => Promise<void>;
  createCard: (accountId?: string) => Promise<boolean>;
  reviewCard: (id: string, status: "approved" | "draft", note?: string) => Promise<void>;
  toggleCard: (id: string, enabled: boolean) => Promise<void>;
  deleteCard: (id: string) => Promise<void>;
}

export const useReferralCardStore = create<ReferralCardState & ReferralCardActions>((set, get) => ({
  cards: [],
  cardDraft: { ...emptyDraft },

  setCardDraft: (draft) => set({ cardDraft: draft }),

  loadCards: async () => {
    try {
      const response = await api.get<{ items: ReferralCard[] }>("/api/referral-cards");
      set({ cards: response.items });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  createCard: async (accountId?: string) => {
    const { cardDraft } = get();
    if (!cardDraft.displayName.trim() || !cardDraft.targetWxid.trim()) return false;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      const targetStages = cardDraft.targetStages
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      await api.post("/api/referral-cards", {
        accountId: accountId || undefined,
        displayName: cardDraft.displayName.trim(),
        targetWxid: cardDraft.targetWxid.trim(),
        sendTriggerHint: cardDraft.sendTriggerHint.trim() || undefined,
        targetStages
      });
      set({ cardDraft: { ...emptyDraft } });
      await get().loadCards();
      return true;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  reviewCard: async (id: string, status: "approved" | "draft", note?: string) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.post(`/api/referral-cards/${id}/review`, { status, note });
      await get().loadCards();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  toggleCard: async (id: string, enabled: boolean) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.post(`/api/referral-cards/${id}/toggle`, { enabled });
      await get().loadCards();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  deleteCard: async (id: string) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.delete(`/api/referral-cards/${id}`);
      await get().loadCards();
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  }
}));
