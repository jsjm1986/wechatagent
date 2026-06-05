import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import LlmProvidersFeature from "../../../features/llm-providers";

// LlmProvidersFeature 走本地 useState + lib/api（内部用全局 fetch），不依赖任何 store。
// 直接 stub fetch，断言真实 DOM 文案。
vi.stubGlobal("fetch", vi.fn());

const LIST_RESPONSE = {
  items: [
    {
      providerId: "primary-chat",
      name: "主对话供应商",
      // 后端边界已把品牌值规范化为中性协议名；前端只认 chat/messages。
      format: "chat",
      baseUrl: "https://api.example.com/v1",
      apiKeyMasked: "sk-****abcd",
      model: "demo-text-pro",
      isActive: true,
      timeoutSeconds: 30,
      maxRetries: 2,
      retryBaseMs: 500,
      supportsVision: false,
      isVisionActive: false,
      createdAt: 1_700_000_000_000,
      updatedAt: 1_700_000_000_000,
    },
    {
      providerId: "vision-messages",
      name: "视觉供应商",
      format: "messages",
      baseUrl: "https://api.vision.example.com",
      apiKeyMasked: "sk-****wxyz",
      model: "demo-vision",
      isActive: false,
      timeoutSeconds: null,
      maxRetries: null,
      retryBaseMs: null,
      supportsVision: true,
      isVisionActive: true,
      createdAt: 1_700_000_000_000,
      updatedAt: 1_700_000_000_000,
    },
  ],
  active: {
    providerId: "primary-chat",
    format: "chat",
    model: "demo-text-pro",
    baseUrl: "https://api.example.com/v1",
  },
};

describe("LlmProvidersFeature", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      json: async () => LIST_RESPONSE,
    } as Response);
  });

  it("渲染面板标题与两条供应商，含中性协议文案与激活/视觉徽章", async () => {
    render(<LlmProvidersFeature />);

    // 面板级 eyebrow + title（Shell 拥有大页头，组件只保留小标题）
    expect(screen.getByText("模型供应商配置")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("主对话供应商")).toBeInTheDocument();
    });
    expect(screen.getByText("视觉供应商")).toBeInTheDocument();

    // 中性协议标签（无任何 LLM 品牌字面量）
    expect(screen.getByText("Chat Completions 协议")).toBeInTheDocument();
    expect(screen.getByText("Messages 协议")).toBeInTheDocument();

    // 状态徽章来自真实数据
    expect(screen.getByText("已激活")).toBeInTheDocument();
    expect(screen.getByText("视觉模型")).toBeInTheDocument();
  });

  it("无数据时渲染空态引导", async () => {
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      json: async () => ({ items: [], active: null }),
    } as Response);

    render(<LlmProvidersFeature />);

    await waitFor(() => {
      expect(screen.getByText("暂无供应商配置")).toBeInTheDocument();
    });
  });
});
