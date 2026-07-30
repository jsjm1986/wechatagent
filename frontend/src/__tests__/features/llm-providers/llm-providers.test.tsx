import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
      effectiveTimeoutSeconds: 30,
      timeoutSecondsSource: "provider",
      maxRetries: 2,
      effectiveMaxRetries: 2,
      maxRetriesSource: "provider",
      retryBaseMs: 500,
      effectiveRetryBaseMs: 500,
      retryBaseMsSource: "provider",
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
      effectiveTimeoutSeconds: 45,
      timeoutSecondsSource: "global_default",
      maxRetries: null,
      effectiveMaxRetries: 5,
      maxRetriesSource: "global_default",
      retryBaseMs: null,
      effectiveRetryBaseMs: 1500,
      retryBaseMsSource: "global_default",
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
    vi.spyOn(window, "alert").mockImplementation(() => undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
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
    expect(screen.getByText(/45s（全局默认）/)).toBeInTheDocument();
    expect(screen.getByText(/30s（Provider 覆盖）/)).toBeInTheDocument();
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

  it("active provider cannot be saved before this exact draft passes connectivity test", async () => {
    const user = userEvent.setup();
    render(<LlmProvidersFeature />);

    await user.click((await screen.findAllByRole("button", { name: "编辑" }))[0]);
    const publish = screen.getByRole("button", { name: "确认发布" });

    expect(publish).toBeDisabled();
    expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "PUT")).toBe(false);
  });

  it("active provider publishes only after test success and explicit confirmation", async () => {
    const user = userEvent.setup();
    vi.mocked(fetch).mockImplementation(async (input, init) => {
      const url = String(input);
      if (url.endsWith("/test") && init?.method === "POST") {
        return {
          ok: true,
          json: async () => ({
            ok: true,
            latencyMs: 12,
            preview: { ok: true },
            activeUpdateApproval: {
              token: "approval-token-1",
              expectedUpdatedAt: 1_700_000_000_000,
              expiresAt: 1_700_000_600_000,
            },
          }),
        } as Response;
      }
      if (init?.method === "PUT") {
        return { ok: true, json: async () => ({ item: LIST_RESPONSE.items[0] }) } as Response;
      }
      return { ok: true, json: async () => LIST_RESPONSE } as Response;
    });

    render(<LlmProvidersFeature />);
    await user.click((await screen.findAllByRole("button", { name: "编辑" }))[0]);
    await user.clear(screen.getByLabelText("model"));
    await user.type(screen.getByLabelText("model"), "demo-text-v2");
    await user.click(screen.getByRole("button", { name: "测试连通性" }));
    expect(await screen.findByText("测试成功")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "确认发布" }));

    await waitFor(() => {
      expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true);
    });
    expect(window.confirm).toHaveBeenCalledWith(
      "确认发布这份已测试配置？保存后将立即热切换全部生产对话。",
    );
    const putCall = vi.mocked(fetch).mock.calls.find(([, init]) => init?.method === "PUT");
    expect(putCall).toBeDefined();
    const body = JSON.parse(String(putCall![1]?.body)) as Record<string, unknown>;
    expect(body).toMatchObject({
      providerId: "primary-chat",
      model: "demo-text-v2",
      expectedUpdatedAt: 1_700_000_000_000,
      activeUpdateConfirmed: true,
      activeUpdateTestToken: "approval-token-1",
    });
  });

  it("editing any field after a successful test invalidates the approval", async () => {
    const user = userEvent.setup();
    vi.mocked(fetch).mockImplementation(async (input, init) => {
      if (String(input).endsWith("/test") && init?.method === "POST") {
        return {
          ok: true,
          json: async () => ({
            ok: true,
            latencyMs: 8,
            preview: { ok: true },
            activeUpdateApproval: {
              token: "approval-token-stale",
              expectedUpdatedAt: 1_700_000_000_000,
              expiresAt: 1_700_000_600_000,
            },
          }),
        } as Response;
      }
      return { ok: true, json: async () => LIST_RESPONSE } as Response;
    });

    render(<LlmProvidersFeature />);
    await user.click((await screen.findAllByRole("button", { name: "编辑" }))[0]);
    await user.click(screen.getByRole("button", { name: "测试连通性" }));
    expect(await screen.findByText("测试成功")).toBeInTheDocument();
    await user.type(screen.getByLabelText("展示名称"), " changed");
    expect(screen.queryByText("测试成功")).not.toBeInTheDocument();

    expect(screen.getByRole("button", { name: "确认发布" })).toBeDisabled();
    expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "PUT")).toBe(false);
  });

  it("clearing retry fields on an existing provider sends explicit null through test and save", async () => {
    const user = userEvent.setup();
    vi.mocked(fetch).mockImplementation(async (input, init) => {
      const url = String(input);
      if (url.endsWith("/test") && init?.method === "POST") {
        return {
          ok: true,
          json: async () => ({
            ok: true,
            latencyMs: 9,
            preview: { ok: true },
            activeUpdateApproval: {
              token: "synthetic-approval-clear-defaults",
              expectedUpdatedAt: 1_700_000_000_000,
              expiresAt: 1_700_000_600_000,
            },
          }),
        } as Response;
      }
      if (init?.method === "PUT") {
        return { ok: true, json: async () => ({ item: LIST_RESPONSE.items[0] }) } as Response;
      }
      return { ok: true, json: async () => LIST_RESPONSE } as Response;
    });

    render(<LlmProvidersFeature />);
    await user.click((await screen.findAllByRole("button", { name: "编辑" }))[0]);
    for (const label of ["超时秒数", "最大重试", "重试退避基线 (ms)"]) {
      await user.clear(screen.getByLabelText(label));
    }
    await user.click(screen.getByRole("button", { name: "测试连通性" }));
    expect(await screen.findByText("测试成功")).toBeInTheDocument();

    const testCall = vi.mocked(fetch).mock.calls.find(([input, init]) =>
      String(input).endsWith("/test") && init?.method === "POST"
    );
    expect(JSON.parse(String(testCall![1]?.body))).toMatchObject({
      timeoutSeconds: null,
      maxRetries: null,
      retryBaseMs: null,
    });

    await user.click(screen.getByRole("button", { name: "确认发布" }));
    await waitFor(() => {
      expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "PUT")).toBe(true);
    });
    const putCall = vi.mocked(fetch).mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(String(putCall![1]?.body))).toMatchObject({
      timeoutSeconds: null,
      maxRetries: null,
      retryBaseMs: null,
      activeUpdateTestToken: "synthetic-approval-clear-defaults",
    });
  });

  it("prevents disabling or deleting the currently assigned vision provider", async () => {
    const user = userEvent.setup();
    render(<LlmProvidersFeature />);

    const editButtons = await screen.findAllByRole("button", { name: "编辑" });
    const deleteButtons = screen.getAllByRole("button", { name: "删除" });
    expect(deleteButtons[1]).toBeDisabled();

    await user.click(editButtons[1]);
    const supportsVision = screen.getByLabelText("支持图片输入（多模态视觉）");
    expect(supportsVision).toBeChecked();
    await user.click(supportsVision);

    expect(window.alert).toHaveBeenCalledWith(
      "当前供应商仍是视觉模型，请先取消视觉指派或改派其它供应商",
    );
    expect(supportsVision).toBeChecked();
    expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "DELETE")).toBe(false);
  });

  it("confirms and submits one atomic vision reassignment request", async () => {
    const user = userEvent.setup();
    const response = structuredClone(LIST_RESPONSE);
    response.items[0].supportsVision = true;
    vi.mocked(fetch).mockImplementation(async (input, init) => {
      if (String(input).endsWith("/primary-chat/vision") && init?.method === "POST") {
        return { ok: true, json: async () => ({ ok: true, item: response.items[0] }) } as Response;
      }
      return { ok: true, json: async () => response } as Response;
    });

    render(<LlmProvidersFeature />);
    await user.click(await screen.findByRole("button", { name: "设为视觉模型" }));

    expect(window.confirm).toHaveBeenCalledWith(
      "确认将「主对话供应商」设为视觉模型？当前视觉指派将被原子替换。",
    );
    await waitFor(() => {
      expect(
        vi.mocked(fetch).mock.calls.filter(([input, init]) =>
          String(input).endsWith("/primary-chat/vision") && init?.method === "POST"
        ),
      ).toHaveLength(1);
    });
    const call = vi.mocked(fetch).mock.calls.find(([input, init]) =>
      String(input).endsWith("/primary-chat/vision") && init?.method === "POST"
    );
    expect(JSON.parse(String(call![1]?.body))).toEqual({ active: true });
  });
});
