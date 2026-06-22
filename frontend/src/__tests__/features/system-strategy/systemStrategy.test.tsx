import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { api } from "../../../lib/api";
import { useStrategyStore } from "../../../stores/strategyStore";
import { useUiStore } from "../../../stores/uiStore";

// Mock API
vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({}),
    patch: vi.fn().mockResolvedValue({}),
    delete: vi.fn().mockResolvedValue({}),
    postRaw: vi.fn().mockResolvedValue({ ok: true, status: 200, data: { item: {} } }),
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

describe("TaxonomiesAdmin 新增条目", () => {
  beforeEach(() => {
    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  it("新增提交 POST /api/admin/taxonomies，body 形态正确，别名中英文逗号都 split", async () => {
    const postRaw = vi.spyOn(api, "postRaw").mockResolvedValue({ ok: true, status: 200, data: { item: {} } });
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);

    render(<SystemStrategyFeature />);

    // 与现有用例同款：SystemStrategyFeature 一次渲染全部面板，无需切 tab。
    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "need_discovery" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.change(screen.getByPlaceholderText(/别名/i), { target: { value: "挖需求，需求探索, 探需" } });
    fireEvent.click(screen.getByText("保存"));

    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    expect(postRaw).toHaveBeenCalledWith("/api/admin/taxonomies", {
      scope: "global",
      kind: "customer_stage",
      value: { id: "need_discovery", label: "需求挖掘", aliases: ["挖需求", "需求探索", "探需"], description: undefined },
    });
  });
});

describe("TaxonomiesAdmin 编辑与废弃恢复", () => {
  beforeEach(() => {
    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  const activeItem = {
    id: "id_active", scope: "global", kind: "customer_stage",
    value: { id: "need_discovery", label: "需求挖掘", aliases: ["挖需求"], description: "", status: "active" },
    version: 1, currentVersion: true, previousVersion: null, seededBy: "manual", updatedAt: "",
  };
  const deprecatedItem = {
    id: "id_dep", scope: "global", kind: "intent_level",
    value: { id: "low", label: "低意向", aliases: [], description: "", status: "deprecated" },
    version: 1, currentVersion: true, previousVersion: null, seededBy: "manual", updatedAt: "",
  };

  // 多个灰度面板共用 api.get；仅 /taxonomies 端点返回 seed 条目，其余面板（状态机/候选）保持空，
  // 避免把字典条目喂给 StatePolicyAdmin 触发其 item.allowed.join 崩溃（非本 task 范围）。
  function seedTaxonomyGet(seed: object) {
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve((url.includes("/api/admin/taxonomies") ? { items: [seed] } : { items: [] }) as never)
    );
  }

  it("编辑提交 PATCH /:id，body 仅 label/aliases/description（无 id/scope/kind）", async () => {
    seedTaxonomyGet(activeItem);
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item: activeItem } as never);
    render(<SystemStrategyFeature />);
    fireEvent.click(await screen.findByText("编辑"));
    fireEvent.change(screen.getByDisplayValue("需求挖掘"), { target: { value: "需求探索阶段" } });
    fireEvent.click(screen.getByText("保存编辑"));
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch).toHaveBeenCalledWith("/api/admin/taxonomies/id_active", {
      label: "需求探索阶段", aliases: ["挖需求"], description: "",
    });
  });

  it("active 条目显示「废弃」，点击调 api.delete", async () => {
    seedTaxonomyGet(activeItem);
    const del = vi.spyOn(api, "delete").mockResolvedValue({ ok: true } as never);
    render(<SystemStrategyFeature />);
    fireEvent.click(await screen.findByText("废弃"));
    await waitFor(() => expect(del).toHaveBeenCalledWith("/api/admin/taxonomies/id_active"));
  });

  it("deprecated 条目显示「恢复」，点击调 api.patch {deprecated:false}", async () => {
    seedTaxonomyGet(deprecatedItem);
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item: deprecatedItem } as never);
    render(<SystemStrategyFeature />);
    // deprecated 条目需勾「显示已废弃」才可见（现有 includeDeprecated checkbox）
    fireEvent.click(screen.getByText("显示已废弃"));
    fireEvent.click(await screen.findByText("恢复"));
    await waitFor(() => expect(patch).toHaveBeenCalledWith("/api/admin/taxonomies/id_dep", { deprecated: false }));
  });
});