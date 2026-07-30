import { create } from "zustand";
import type { Contact, ContactTab } from "../types";
import { api } from "../lib/api";
import { useAccountStore } from "./accountStore";
import { useUiStore } from "./uiStore";

interface ContactState {
  contacts: Contact[];
  selected: Contact | null;
  dataAccountId: string;
  requestGeneration: number;
  loading: boolean;
  contactTab: ContactTab;
  setContacts: (contacts: Contact[], accountId?: string) => void;
  setSelected: (c: Contact | null) => void;
  setContactTab: (t: ContactTab) => void;
  loadContacts: (accountId: string, query?: string, options?: { silent?: boolean }) => Promise<void>;
  clearForAccount: (accountId: string) => void;
  managedCount: () => number;
  normalCount: () => number;
}

function currentAccountId(): string {
  return useAccountStore.getState().currentAccountId();
}

export const useContactStore = create<ContactState>((set, get) => {
  const scopedContacts = () => {
    const state = get();
    return state.dataAccountId === currentAccountId() ? state.contacts : [];
  };

  return {
    contacts: [],
    selected: null,
    dataAccountId: "",
    requestGeneration: 0,
    loading: false,
    contactTab: "all",

    setContacts: (contacts, accountId) => {
      const scope = accountId || contacts[0]?.accountId || currentAccountId();
      set({ contacts, dataAccountId: scope });
    },

    setSelected: (selected) => {
      const state = get();
      if (
        selected
        && (selected.accountId !== state.dataAccountId || selected.accountId !== currentAccountId())
      ) {
        return;
      }
      set({ selected });
    },

    setContactTab: (contactTab) => set({ contactTab }),

    clearForAccount: (accountId) => set((state) => ({
      contacts: [],
      selected: null,
      dataAccountId: accountId,
      requestGeneration: state.requestGeneration + 1,
      loading: false,
    })),

    loadContacts: async (accountId, query, options) => {
      const generation = get().requestGeneration + 1;
      const scopeChanged = get().dataAccountId !== accountId;
      set({
        dataAccountId: accountId,
        requestGeneration: generation,
        loading: Boolean(accountId),
        ...(scopeChanged ? { contacts: [], selected: null } : {}),
      });
      if (!accountId || currentAccountId() !== accountId) {
        set({ loading: false });
        return;
      }

      const params = [`accountId=${encodeURIComponent(accountId)}`, "limit=500"];
      const trimmed = query?.trim();
      if (trimmed) params.push(`q=${encodeURIComponent(trimmed)}`);

      try {
        const data = await api.get<{ items: Contact[] }>(`/api/contacts?${params.join("&")}`);
        const state = get();
        if (
          state.requestGeneration !== generation
          || state.dataAccountId !== accountId
          || currentAccountId() !== accountId
        ) {
          return;
        }
        if (data.items.some((contact) => contact.accountId !== accountId)) {
          set({ contacts: [], selected: null, loading: false });
          if (!options?.silent) {
            useUiStore.getState().setError("联系人响应账号与当前账号不一致，已拒绝显示。");
          }
          return;
        }
        set({ contacts: data.items });
      } catch (error) {
        const state = get();
        if (
          state.requestGeneration === generation
          && state.dataAccountId === accountId
          && currentAccountId() === accountId
        ) {
          set({ contacts: [], selected: null });
          if (!options?.silent) {
            useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
          }
        }
      } finally {
        const state = get();
        if (
          state.requestGeneration === generation
          && state.dataAccountId === accountId
          && currentAccountId() === accountId
        ) {
          set({ loading: false });
        }
      }
    },

    managedCount: () => scopedContacts().filter((contact) => contact.agentStatus === "managed").length,
    normalCount: () => scopedContacts().length - get().managedCount(),
  };
});
