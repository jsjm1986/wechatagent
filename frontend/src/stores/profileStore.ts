import { create } from "zustand";
import { api } from "../lib/api";
import type { DomainProfile } from "../types";

interface ProfileState {
  activeProfile: DomainProfile | null;
  loading: boolean;
  error: string | null;
  loadActiveProfile: () => Promise<void>;
}

export const useProfileStore = create<ProfileState>((set) => ({
  activeProfile: null,
  loading: false,
  error: null,
  loadActiveProfile: async () => {
    set({ loading: true, error: null });
    try {
      const data = await api.get<{ item: DomainProfile | null }>(
        "/api/admin/domain-profiles/active"
      );
      set({ activeProfile: data.item, loading: false });
    } catch (err) {
      // 降级：拿不到 active profile 时前端照常跑，只是没有行业化数据。
      set({
        activeProfile: null,
        loading: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  },
}));
