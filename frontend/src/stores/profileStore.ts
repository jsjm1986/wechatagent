import { create } from "zustand";
import { api } from "../lib/api";
import type { DomainProfile } from "../types";

export interface TaxonomyValueLite {
  id: string;
  label: string;
}
export type TaxonomyMap = Record<string, TaxonomyValueLite[]>;
export interface ProfileDimensionView {
  kind: string;
  displayName: string;
  participatesInDecision: boolean;
}
export type LabelStatus = "ok" | "unknown_value" | "no_dict";
export interface LabelResult {
  text: string;
  status: LabelStatus;
}

// 纯函数：canonical 值 → 中文 display_name。三情形分流区分"数据野值"与"配置缺失"，
// 绝不显示错误销售标签（守诚实立场）。
export function labelFor(taxonomies: TaxonomyMap, kind: string, value: string): LabelResult {
  const entries = taxonomies[kind];
  if (!entries || entries.length === 0) return { text: value, status: "no_dict" };
  const hit = entries.find((e) => e.id === value);
  if (!hit) return { text: value, status: "unknown_value" };
  return { text: hit.label, status: "ok" };
}

interface ProfileState {
  activeProfile: DomainProfile | null;
  dimensions: ProfileDimensionView[];
  taxonomies: TaxonomyMap;
  loading: boolean;
  error: string | null;
  loadActiveProfile: () => Promise<void>;
  loadActiveView: () => Promise<void>;
}

export const useProfileStore = create<ProfileState>((set) => ({
  activeProfile: null,
  dimensions: [],
  taxonomies: {},
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
  loadActiveView: async () => {
    try {
      const data = await api.get<{
        dimensions: ProfileDimensionView[];
        taxonomies: TaxonomyMap;
      }>("/api/operation/active-view");
      set({ dimensions: data.dimensions ?? [], taxonomies: data.taxonomies ?? {} });
    } catch (err) {
      // 降级：拿不到取值字典时前端照常跑，labelFor 一律回落 no_dict。
      set({
        dimensions: [],
        taxonomies: {},
        error: err instanceof Error ? err.message : String(err),
      });
    }
  },
}));
