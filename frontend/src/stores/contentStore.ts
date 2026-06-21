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
  uploadMediaAsset: (form: FormData, accountId?: string) => Promise<boolean>;
  reviewMediaAsset: (
    id: string,
    status: "approved" | "draft",
    note?: string,
    accountId?: string
  ) => Promise<void>;
  editAssetMeta: (id: string, fields: Record<string, unknown>, accountId?: string) => Promise<void>;
  replaceAssetFile: (id: string, form: FormData, accountId?: string) => Promise<boolean>;
  toggleAssetSendable: (id: string, sendable: boolean, accountId?: string) => Promise<void>;
  deleteAsset: (id: string, accountId?: string) => Promise<void>;
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

  uploadMediaAsset: async (form: FormData, accountId?: string) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.postForm<{ id: string }>("/api/content-assets/upload", form);
      await get().loadAssets(accountId);
      return true;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  reviewMediaAsset: async (
    id: string,
    status: "approved" | "draft",
    note?: string,
    accountId?: string
  ) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.post(`/api/content-assets/${id}/review`, { status, note });
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  editAssetMeta: async (id, fields, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.put(`/api/content-assets/${id}`, fields);
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  replaceAssetFile: async (id, form, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.postForm(`/api/content-assets/${id}/file`, form);
      await get().loadAssets(accountId);
      return true;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  toggleAssetSendable: async (id, sendable, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.post(`/api/content-assets/${id}/toggle`, { sendable });
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  deleteAsset: async (id, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.delete(`/api/content-assets/${id}`);
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
}));