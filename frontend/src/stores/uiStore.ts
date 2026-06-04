import { create } from "zustand";

interface UiState {
  busy: boolean;
  error: string;
  setBusy: (busy: boolean) => void;
  setError: (error: string) => void;
}

export const useUiStore = create<UiState>((set) => ({
  busy: false,
  error: "",
  setBusy: (busy) => set({ busy }),
  setError: (error) => set({ error }),
}));
