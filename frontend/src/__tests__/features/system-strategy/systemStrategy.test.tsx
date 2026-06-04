import { render, screen } from "@testing-library/react";
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
});