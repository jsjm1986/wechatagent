import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { useStrategyStore } from "../../../stores/strategyStore";
import { useUiStore } from "../../../stores/uiStore";

// Mock API
vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({}),
  },
}));

describe("SystemStrategy Feature", () => {
  beforeEach(() => {
    // Mock loadStrategyData to avoid API calls
    const mockLoadStrategyData = vi.fn();

    // Reset stores
    useStrategyStore.setState({
      souls: [],
      promptTemplates: [],
      soulDraft: { agentKind: "user", name: "", content: "" },
      editingSoulId: "",
      promptDraft: {
        promptKey: "",
        agentKind: "user",
        layer: "task_template",
        title: "",
        description: "",
        content: ""
      },
      editingPromptId: "",
      setSoulDraft: vi.fn(),
      setPromptDraft: vi.fn(),
      loadStrategyData: mockLoadStrategyData,
      createSoul: vi.fn(),
      saveSoul: vi.fn(),
      publishSoul: vi.fn(),
      createPromptTemplate: vi.fn(),
      savePromptTemplate: vi.fn(),
      publishPromptTemplate: vi.fn(),
      resetSystemPromptPack: vi.fn(),
      editSoul: vi.fn(),
      newSoulDraftFor: vi.fn(),
      editPromptTemplate: vi.fn(),
      newPromptDraftFor: vi.fn(),
    });

    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  it("should render system strategy view", () => {
    render(<SystemStrategyFeature />);

    // 检查关键文案
    expect(screen.getByText("系统总控策略")).toBeInTheDocument();
  });

  it("should render global strategy text", () => {
    render(<SystemStrategyFeature />);

    // 检查Global Strategy文案
    expect(screen.getByText("Global Strategy")).toBeInTheDocument();
  });

  // 一体化迁移后追加：四块面板 + 三类灰度面板 + 重置按钮在新视觉壳下真实渲染。
  it("一体化迁移：总控/Prompt/状态机/字典/教训四类面板小标题均渲染", () => {
    render(<SystemStrategyFeature />);

    expect(screen.getByText("系统总控 Prompt")).toBeInTheDocument();
    expect(screen.getByText("状态机动作策略灰度")).toBeInTheDocument();
    expect(screen.getByText("双层标签字典灰度")).toBeInTheDocument();
    expect(screen.getByText("跨用户教训归纳（14d 滑窗）")).toBeInTheDocument();
  });

  it("一体化迁移：暂无数据时灰度面板渲染空态，重置 Prompt Pack 按钮可见", async () => {
    render(<SystemStrategyFeature />);

    expect(screen.getByText("重置系统 Prompt Pack v2")).toBeInTheDocument();
    // api.get mock 返回空 items（异步 reload 后）→ 各灰度面板空态文案
    await waitFor(() => {
      expect(screen.getByText("暂无状态策略")).toBeInTheDocument();
    });
    expect(screen.getByText("暂无字典条目")).toBeInTheDocument();
    expect(screen.getByText("暂无教训聚合（窗口内无命中样本）")).toBeInTheDocument();
  });
});