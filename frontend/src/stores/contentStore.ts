import { create } from "zustand";
import type { ContentAsset } from "../types";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";

interface ContentState {
  assets: ContentAsset[];
  assetDraft: {
    kind: string;
    title: string;
    body: string;
    url: string;
    mediaId: string;
    usageScene: string;
  };
}

interface ContentActions {
  setAssetDraft: (draft: {
    kind: string;
    title: string;
    body: string;
    url: string;
    mediaId: string;
    usageScene: string;
  }) => void;
  loadAssets: (accountId?: string) => Promise<void>;
  createAsset: (accountId?: string) => Promise<void>;
}

export const useContentStore = create<ContentState & ContentActions>((set, get) => ({
  assets: [],
  assetDraft: {
    kind: "text",
    title: "",
    body: "",
    url: "",
    mediaId: "",
    usageScene: ""
  },

  setAssetDraft: (draft) => set({ assetDraft: draft }),

  loadAssets: async (accountId?: string) => {
    try {
      const accountParam = accountId ? `?accountId=${accountId}` : "";
      const response = await api.get<{ items: ContentAsset[] }>(`/api/content-assets${accountParam}`);
      set({ assets: response.items });
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  },

  createAsset: async (accountId?: string) => {
    const { assetDraft } = get();
    if (!assetDraft.title.trim()) return;

    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");

    try {
      await api.post("/api/content-assets", {
        accountId: accountId || undefined,
        kind: assetDraft.kind,
        title: assetDraft.title,
        body: assetDraft.body || undefined,
        url: assetDraft.url || undefined,
        mediaId: assetDraft.mediaId || undefined,
        usageScene: assetDraft.usageScene || undefined
      });

      // 重置 draft，保留 kind
      set({
        assetDraft: {
          kind: assetDraft.kind,
          title: "",
          body: "",
          url: "",
          mediaId: "",
          usageScene: ""
        }
      });

      // 重新加载 assets
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
}));