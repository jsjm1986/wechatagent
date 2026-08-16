import { create } from "zustand";
import type { ContentAsset } from "../types";
import { api } from "../lib/api";
import { useAccountStore } from "./accountStore";
import { useUiStore } from "./uiStore";

export type AssetDraft = {
  kind: string;
  title: string;
  body: string;
  usageScene: string;
  minInjectTier: string;
  enabled: boolean;
  allowedInsertionLevels: Array<"subtle" | "contextual" | "direct">;
  usageGuidance: string;
};

const EMPTY_DRAFT: AssetDraft = {
  kind: "text",
  title: "",
  body: "",
  usageScene: "",
  minInjectTier: "full",
  enabled: true,
  allowedInsertionLevels: ["subtle", "contextual", "direct"],
  usageGuidance: "",
};

type AssetScopePayload =
  | { expectedScope: "account"; expectedAccountId: string }
  | { expectedScope: "workspace" };

interface ContentState {
  assets: ContentAsset[];
  assetsAccountId: string;
  assetsRequestGeneration: number;
  assetDraft: AssetDraft;
  assetDraftAccountId: string;
}

interface ContentActions {
  setAssetDraft: (accountId: string, draft: AssetDraft) => void;
  loadAssets: (accountId: string, tag?: string) => Promise<void>;
  createAsset: (accountId: string) => Promise<void>;
  uploadMediaAsset: (form: FormData, accountId: string) => Promise<boolean>;
  reviewMediaAsset: (
    asset: ContentAsset,
    status: "approved" | "draft",
    note: string | undefined,
    pageAccountId: string
  ) => Promise<void>;
  editAssetMeta: (
    asset: ContentAsset,
    fields: Record<string, unknown>,
    pageAccountId: string
  ) => Promise<void>;
  replaceAssetFile: (
    asset: ContentAsset,
    form: FormData,
    pageAccountId: string
  ) => Promise<boolean>;
  toggleAssetSendable: (
    asset: ContentAsset,
    sendable: boolean,
    pageAccountId: string
  ) => Promise<void>;
  deleteAsset: (asset: ContentAsset, pageAccountId: string) => Promise<void>;
}

function assetScope(asset: ContentAsset): AssetScopePayload {
  return asset.accountId
    ? { expectedScope: "account", expectedAccountId: asset.accountId }
    : { expectedScope: "workspace" };
}

function currentAccountId(): string {
  return useAccountStore.getState().currentAccountId();
}

function entityMatchesPage(asset: ContentAsset, pageAccountId: string): boolean {
  return !asset.accountId || asset.accountId === pageAccountId;
}

export const useContentStore = create<ContentState & ContentActions>((set, get) => {
  function actionIsCurrent(asset: ContentAsset, pageAccountId: string): boolean {
    const state = get();
    return Boolean(pageAccountId)
      && currentAccountId() === pageAccountId
      && state.assetsAccountId === pageAccountId
      && entityMatchesPage(asset, pageAccountId)
      && state.assets.some(
        (candidate) => candidate.id === asset.id && candidate.accountId === asset.accountId
      );
  }

  async function refreshIfCurrent(pageAccountId: string): Promise<void> {
    if (currentAccountId() === pageAccountId) {
      await get().loadAssets(pageAccountId);
    }
  }

  function reportIfCurrent(pageAccountId: string, error: unknown): void {
    if (currentAccountId() === pageAccountId) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    }
  }

  return {
    assets: [],
    assetsAccountId: "",
    assetsRequestGeneration: 0,
    assetDraft: EMPTY_DRAFT,
    assetDraftAccountId: "",

    setAssetDraft: (accountId, draft) => set({
      assetDraftAccountId: accountId,
      assetDraft: draft,
    }),

    loadAssets: async (accountId, tag) => {
      const generation = get().assetsRequestGeneration + 1;
      set({
        assets: [],
        assetsAccountId: accountId,
        assetsRequestGeneration: generation,
      });
      if (!accountId || currentAccountId() !== accountId) return;
      try {
        const params = new URLSearchParams({ accountId });
        if (tag) params.set("tag", tag);
        const response = await api.get<{ items: ContentAsset[] }>(
          `/api/content-assets?${params.toString()}`
        );
        if (
          get().assetsRequestGeneration !== generation
          || get().assetsAccountId !== accountId
          || currentAccountId() !== accountId
        ) return;
        const items = response.items || [];
        if (items.some((asset) => asset.accountId && asset.accountId !== accountId)) {
          set({ assets: [] });
          useUiStore.getState().setError("素材响应账号与当前账号不一致，已拒绝显示。");
          return;
        }
        set({ assets: items });
      } catch (error) {
        if (
          get().assetsRequestGeneration === generation
          && get().assetsAccountId === accountId
          && currentAccountId() === accountId
        ) {
          useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
        }
      }
    },

    createAsset: async (accountId) => {
      const { assetDraft, assetDraftAccountId } = get();
      if (
        !accountId
        || currentAccountId() !== accountId
        || assetDraftAccountId !== accountId
        || !assetDraft.title.trim()
      ) return;
      const frozenDraft = { ...assetDraft };
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        await api.post("/api/content-assets", {
          accountId,
          kind: frozenDraft.kind,
          title: frozenDraft.title,
          body: frozenDraft.body || undefined,
          usageScene: frozenDraft.usageScene || undefined,
          minInjectTier: frozenDraft.minInjectTier,
          enabled: frozenDraft.enabled,
          allowedInsertionLevels: frozenDraft.allowedInsertionLevels,
          usageGuidance: frozenDraft.usageGuidance || undefined,
        });
        if (currentAccountId() !== accountId) return;
        set({
          assetDraftAccountId: accountId,
          assetDraft: {
            ...EMPTY_DRAFT,
            kind: frozenDraft.kind,
            minInjectTier: frozenDraft.minInjectTier,
          },
        });
        await refreshIfCurrent(accountId);
      } catch (error) {
        reportIfCurrent(accountId, error);
      } finally {
        if (currentAccountId() === accountId) useUiStore.getState().setBusy(false);
      }
    },

    uploadMediaAsset: async (form, accountId) => {
      if (!accountId || currentAccountId() !== accountId) return false;
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        await api.postForm<{ id: string }>("/api/content-assets/upload", form);
        if (currentAccountId() !== accountId) return false;
        await refreshIfCurrent(accountId);
        return true;
      } catch (error) {
        reportIfCurrent(accountId, error);
        return false;
      } finally {
        if (currentAccountId() === accountId) useUiStore.getState().setBusy(false);
      }
    },

    reviewMediaAsset: async (asset, status, note, pageAccountId) => {
      if (!actionIsCurrent(asset, pageAccountId)) return;
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        await api.post(`/api/content-assets/${asset.id}/review`, {
          ...assetScope(asset),
          status,
          note,
        });
        await refreshIfCurrent(pageAccountId);
      } catch (error) {
        reportIfCurrent(pageAccountId, error);
      } finally {
        if (currentAccountId() === pageAccountId) useUiStore.getState().setBusy(false);
      }
    },

    editAssetMeta: async (asset, fields, pageAccountId) => {
      if (!actionIsCurrent(asset, pageAccountId)) return;
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        await api.put(`/api/content-assets/${asset.id}`, {
          ...fields,
          ...assetScope(asset),
        });
        await refreshIfCurrent(pageAccountId);
      } catch (error) {
        reportIfCurrent(pageAccountId, error);
      } finally {
        if (currentAccountId() === pageAccountId) useUiStore.getState().setBusy(false);
      }
    },

    replaceAssetFile: async (asset, form, pageAccountId) => {
      if (!actionIsCurrent(asset, pageAccountId)) return false;
      const scope = assetScope(asset);
      form.set("expectedScope", scope.expectedScope);
      if (scope.expectedScope === "account") {
        form.set("expectedAccountId", scope.expectedAccountId);
      } else {
        form.delete("expectedAccountId");
      }
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        await api.postForm(`/api/content-assets/${asset.id}/file`, form);
        if (currentAccountId() !== pageAccountId) return false;
        await refreshIfCurrent(pageAccountId);
        return true;
      } catch (error) {
        reportIfCurrent(pageAccountId, error);
        return false;
      } finally {
        if (currentAccountId() === pageAccountId) useUiStore.getState().setBusy(false);
      }
    },

    toggleAssetSendable: async (asset, sendable, pageAccountId) => {
      if (!actionIsCurrent(asset, pageAccountId)) return;
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        await api.post(`/api/content-assets/${asset.id}/toggle`, {
          ...assetScope(asset),
          sendable,
        });
        await refreshIfCurrent(pageAccountId);
      } catch (error) {
        reportIfCurrent(pageAccountId, error);
      } finally {
        if (currentAccountId() === pageAccountId) useUiStore.getState().setBusy(false);
      }
    },

    deleteAsset: async (asset, pageAccountId) => {
      if (!actionIsCurrent(asset, pageAccountId)) return;
      useUiStore.getState().setBusy(true);
      useUiStore.getState().setError("");
      try {
        const params = new URLSearchParams(assetScope(asset));
        await api.delete(`/api/content-assets/${asset.id}?${params.toString()}`);
        await refreshIfCurrent(pageAccountId);
      } catch (error) {
        reportIfCurrent(pageAccountId, error);
      } finally {
        if (currentAccountId() === pageAccountId) useUiStore.getState().setBusy(false);
      }
    },
  };
});
