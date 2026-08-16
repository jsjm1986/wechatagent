import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    postForm: vi.fn(),
    delete: vi.fn(),
  },
}));

import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import { useContentStore } from "../../stores/contentStore";
import { useUiStore } from "../../stores/uiStore";
import type { Account, ContentAsset } from "../../types";

const PRIVATE_ASSET: ContentAsset = {
  id: "asset-a",
  accountId: "acc-a",
  kind: "media",
  title: "A 私有素材",
};

const SHARED_ASSET: ContentAsset = {
  id: "asset-shared",
  accountId: null,
  kind: "media",
  title: "共享素材",
};

function selectAccount(accountId: string): void {
  useAccountStore.setState({ selectedAccountId: accountId });
}

function installSnapshot(accountId: string, assets: ContentAsset[]): void {
  useContentStore.setState({
    assets,
    assetsAccountId: accountId,
    assetsRequestGeneration: 0,
    assetDraftAccountId: accountId,
  });
}

describe("contentStore account and entity scope", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [
        { id: "record-a", accountId: "acc-a", alias: "A", displayName: "A" } as Account,
        { id: "record-b", accountId: "acc-b", alias: "B", displayName: "B" } as Account,
      ],
      selectedAccountId: "acc-a",
    });
    useContentStore.setState({
      assets: [],
      assetsAccountId: "",
      assetsRequestGeneration: 0,
      assetDraft: {
        kind: "text",
        title: "",
        body: "",
        usageScene: "",
        minInjectTier: "full",
        enabled: true,
        allowedInsertionLevels: ["subtle", "contextual", "direct"],
        usageGuidance: "",
      },
      assetDraftAccountId: "",
    });
    useUiStore.setState({ busy: false, error: "" });
    vi.mocked(api.get).mockResolvedValue({ items: [] });
    vi.mocked(api.post).mockResolvedValue({ ok: true });
    vi.mocked(api.put).mockResolvedValue({ ok: true });
    vi.mocked(api.postForm).mockResolvedValue({ ok: true });
    vi.mocked(api.delete).mockResolvedValue({ ok: true });
  });

  it("freezes a private asset account scope in write requests", async () => {
    installSnapshot("acc-a", [PRIVATE_ASSET]);

    await useContentStore.getState().toggleAssetSendable(PRIVATE_ASSET, false, "acc-a");

    expect(api.post).toHaveBeenCalledWith("/api/content-assets/asset-a/toggle", {
      expectedScope: "account",
      expectedAccountId: "acc-a",
      sendable: false,
    });
  });

  it("marks shared asset writes as workspace scope", async () => {
    installSnapshot("acc-a", [SHARED_ASSET]);

    await useContentStore.getState().editAssetMeta(
      SHARED_ASSET,
      { title: "共享素材新版" },
      "acc-a"
    );

    expect(api.put).toHaveBeenCalledWith("/api/content-assets/asset-shared", {
      title: "共享素材新版",
      expectedScope: "workspace",
    });
  });

  it("drops a slow A response after B becomes current", async () => {
    let resolveA!: (value: { items: ContentAsset[] }) => void;
    const assetB: ContentAsset = {
      id: "asset-b",
      accountId: "acc-b",
      kind: "media",
      title: "B 私有素材",
    };
    vi.mocked(api.get)
      .mockImplementationOnce(() => new Promise((resolve) => { resolveA = resolve; }))
      .mockResolvedValueOnce({ items: [assetB] });

    const loadA = useContentStore.getState().loadAssets("acc-a");
    selectAccount("acc-b");
    const loadB = useContentStore.getState().loadAssets("acc-b");
    await loadB;
    resolveA({ items: [PRIVATE_ASSET] });
    await loadA;

    expect(useContentStore.getState().assetsAccountId).toBe("acc-b");
    expect(useContentStore.getState().assets).toEqual([assetB]);
  });

  it("does not issue an old-account action after account switching", async () => {
    installSnapshot("acc-a", [PRIVATE_ASSET]);
    selectAccount("acc-b");

    await useContentStore.getState().deleteAsset(PRIVATE_ASSET, "acc-a");

    expect(api.delete).not.toHaveBeenCalled();
  });
});
