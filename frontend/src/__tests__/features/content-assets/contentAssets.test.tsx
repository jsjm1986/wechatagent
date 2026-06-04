import { render, screen } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import ContentAssetsFeature from "../../../features/content-assets";
import { useContentStore } from "../../../stores/contentStore";
import { useAccountStore } from "../../../stores/accountStore";
import { useUiStore } from "../../../stores/uiStore";
import type { Account, ContentAsset } from "../../../types";

// Mock fetch
vi.stubGlobal('fetch', vi.fn());

describe("ContentAssetsFeature", () => {
  beforeEach(() => {
    // Mock fetch to return empty response
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      json: async () => ({ items: [] }),
    } as Response);

    // Mock loadAssets to avoid API calls
    const mockLoadAssets = vi.fn();

    // Reset stores
    useContentStore.setState({
      assets: [
        {
          id: "asset1",
          kind: "faq",
          title: "测试FAQ资产",
          body: "这是一个测试FAQ",
        } as ContentAsset
      ],
      assetDraft: {
        kind: "text",
        title: "",
        body: "",
        url: "",
        mediaId: "",
        usageScene: ""
      },
      setAssetDraft: vi.fn(),
      loadAssets: mockLoadAssets,
      createAsset: vi.fn(),
    });

    useAccountStore.setState({
      accounts: [
        {
          id: "acc1",
          accountId: "test123",
          alias: "测试账号",
          displayName: "Test Account",
          online: true,
          mcpKeyConfigured: true
        } as Account
      ],
      selectedAccountId: "acc1",
    });

    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  it("renders content assets title", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("内容资产库")).toBeInTheDocument();
  });

  it("renders Content Assets header", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("Content Assets")).toBeInTheDocument();
  });

  it("displays asset in the list", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("测试FAQ资产")).toBeInTheDocument();
  });

  it("renders new asset form", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("新增资产")).toBeInTheDocument();
    expect(screen.getByText("保存资产")).toBeInTheDocument();
  });

  it("renders form fields", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("类型")).toBeInTheDocument();
    expect(screen.getByText("标题")).toBeInTheDocument();
    expect(screen.getByText("正文")).toBeInTheDocument();
  });
});