import { create } from "zustand";
import type { Account } from "../types";

const STORAGE_KEY = "wechatagent.accountId";

interface AccountState {
  accounts: Account[];
  selectedAccountId: string;
  setAccounts: (accounts: Account[]) => void;
  selectAccount: (accountId: string) => void;
  currentAccountId: () => string;
  currentAccount: () => Account | undefined;
  onlineCount: () => number;
}

export const useAccountStore = create<AccountState>((set, get) => ({
  accounts: [],
  selectedAccountId: localStorage.getItem(STORAGE_KEY) || "",
  setAccounts: (accounts) => set({ accounts }),
  selectAccount: (accountId) => {
    localStorage.setItem(STORAGE_KEY, accountId);
    set({ selectedAccountId: accountId });
  },
  currentAccountId: () => {
    const { accounts, selectedAccountId } = get();
    return accounts.some((a) => a.accountId === selectedAccountId)
      ? selectedAccountId
      : accounts[0]?.accountId ?? "";
  },
  currentAccount: () => {
    const id = get().currentAccountId();
    return get().accounts.find((a) => a.accountId === id);
  },
  onlineCount: () => get().accounts.filter((a) => a.online).length,
}));
