import { render, screen, fireEvent } from "@testing-library/react";
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
    const mockEditAssetMeta = vi.fn();
    const mockDeleteAsset = vi.fn();
    useContentStore.setState({
      assets: [
        {
          id: "asset1",
          kind: "faq",
          title: "测试FAQ资产",
          body: "这是一个测试FAQ",
          minInjectTier: "lean"
        } as ContentAsset
      ],
      assetDraft: {
        kind: "text",
        title: "",
        body: "",
        usageScene: "",
        minInjectTier: "full"
      },
      setAssetDraft: vi.fn(),
      loadAssets: mockLoadAssets,
      createAsset: vi.fn(),
      editAssetMeta: mockEditAssetMeta,
      deleteAsset: mockDeleteAsset,
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

  it("renders min inject tier select and drops legacy url/mediaId/moment fields", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("最低注入档")).toBeInTheDocument();
    expect(screen.queryByText("素材 URL")).toBeNull();
    expect(screen.queryByText("MCP Media ID")).toBeNull();
    expect(screen.queryByText("朋友圈素材")).toBeNull();
  });

  // E：文本行 kind 显示中文 label，不是英文原始值
  it("shows chinese kind label on text asset row, not raw english", () => {
    render(<ContentAssetsFeature />);
    // "FAQ" 同时出现在新增表单的 select 选项里，故用 getAllByText 断言至少一处
    expect(screen.getAllByText("FAQ").length).toBeGreaterThanOrEqual(1);
    // 关键：行内不再渲染英文原始值 "faq"（小写）
    expect(screen.queryByText("faq")).toBeNull();
  });

  // A：文本行展示注入档中文标签（minInjectTier=lean → 精简档）
  it("shows inject tier chinese label on text asset row", () => {
    render(<ContentAssetsFeature />);
    expect(screen.getByText("精简档")).toBeInTheDocument();
  });

  // B：文本行有编辑/删除入口；编辑改注入档 → editAssetMeta 被调用且 fields 含 minInjectTier
  it("edits a text asset and calls editAssetMeta with minInjectTier", () => {
    const editAssetMeta = useContentStore.getState().editAssetMeta;
    render(<ContentAssetsFeature />);

    fireEvent.click(screen.getByText("编辑"));
    // 编辑态出现「最低注入档」label（与新增表单共用文案，故应有 2 处）
    expect(screen.getAllByText("最低注入档").length).toBeGreaterThanOrEqual(2);

    fireEvent.click(screen.getByText("保存"));
    expect(editAssetMeta).toHaveBeenCalledTimes(1);
    const fields = vi.mocked(editAssetMeta).mock.calls[0][1] as Record<string, unknown>;
    expect(fields).toHaveProperty("minInjectTier", "lean");
    expect(fields).toHaveProperty("title", "测试FAQ资产");
  });

  // B：删除按钮经 window.confirm 确认后调用 deleteAsset
  it("deletes a text asset after confirm", () => {
    const deleteAsset = useContentStore.getState().deleteAsset;
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<ContentAssetsFeature />);

    fireEvent.click(screen.getByText("删除"));
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(deleteAsset).toHaveBeenCalledWith("asset1", "test123");
    confirmSpy.mockRestore();
  });

  // D：禁语资产行显示「恒注入」徽标，不渲染档位标签误导（后端禁语恒注入、无视档位）
  it("forbidden asset row shows 恒注入 badge not tier label", () => {
    useContentStore.setState({
      assets: [
        {
          id: "f1",
          kind: "forbidden_expression",
          title: "保本承诺",
          body: "不得说保本",
          minInjectTier: "full"
        } as ContentAsset
      ],
    });
    render(<ContentAssetsFeature />);
    expect(screen.getByText("恒注入")).toBeInTheDocument();
    // 不应把禁语渲染成「完整档」误导
    expect(screen.queryByText("完整档")).toBeNull();
  });
});