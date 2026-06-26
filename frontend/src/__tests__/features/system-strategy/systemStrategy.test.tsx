import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { api } from "../../../lib/api";
import { useStrategyStore } from "../../../stores/strategyStore";
import { useUiStore } from "../../../stores/uiStore";
import type { ProfileDimension, DomainProfileDraft } from "../../../types";

// CSS module identity mock：vitest 默认 css:false 会把 styles.xxx 解析为 undefined，
// 导致 className 不落到 DOM、无法按 .inlineError / .badgeOk 定位。这里把 CSS module
// 改成 identity 代理（styles.badgeOk === "badgeOk"），只影响 className 字符串，不动真实
// 渲染结构——让 409 用例可以语义化地断言 error 框（inlineError）确实不存在。
vi.mock("../../../features/system-strategy/SystemStrategy.module.css", () => ({
  default: new Proxy({}, { get: (_t, key) => String(key) }),
}));

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
      value: { id: "need_discovery", label: "需求挖掘", aliases: ["挖需求", "需求探索", "探需"], description: undefined, isTerminal: false, isReactivationTarget: false },
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
      label: "需求探索阶段", aliases: ["挖需求"], description: "", isTerminal: false, isReactivationTarget: false,
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
  it("历史版本行（currentVersion=false）不挂编辑/废弃按钮", async () => {
    const historyItem = {
      id: "id_hist", scope: "global", kind: "customer_stage",
      value: { id: "need_discovery", label: "需求挖掘(旧版)", aliases: [], description: "", status: "active" },
      version: 1, currentVersion: false, previousVersion: null, seededBy: "manual", updatedAt: "",
    };
    seedTaxonomyGet(historyItem);
    render(<SystemStrategyFeature />);
    // 条目本身渲染（标题可见），但写操作按钮对历史版本行隐藏。
    expect(await screen.findByText("需求挖掘(旧版)")).toBeInTheDocument();
    expect(screen.queryByText("编辑")).toBeNull();
    expect(screen.queryByText("废弃")).toBeNull();
  });
});

describe("TaxonomiesAdmin 边界", () => {
  beforeEach(() => {
    // 清理上一个用例累积的 spy 调用计数（postRaw 不发请求断言依赖干净计数）
    vi.clearAllMocks();
    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  it("新增重复条目(409) 显示 info 不显示 error", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    vi.spyOn(api, "postRaw").mockResolvedValue({
      ok: false,
      status: 409,
      data: { message: "(scope=global, kind=customer_stage, value.id=need_discovery) 已存在" },
    } as never);
    const { container } = render(<SystemStrategyFeature />);
    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "need_discovery" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.click(screen.getByText("保存"));
    // 409 走 info（badgeOk），不进 inlineError —— 文案以 message 内的「已存在」断言
    const infoBadge = await screen.findByText(/已存在/);
    expect(infoBadge).toBeInTheDocument();
    // 反向断言（消半永真）：info 与 error 共用 res.data.message 文案，仅靠文本无法区分 info/error，
    // 必须按 class 定位 error 框（styles.inlineError，vitest css:false 下为 identity className）。
    // 若 409 分支被误改成走 setError，则 .inlineError 会出现并命中该「已存在」文案 → 这两条断言才会红。
    expect(container.querySelector(".inlineError")).toBeNull();
    // info 徽章本身确实是 badgeOk（info 渲染位），双向锁住「显示 info」。
    expect(infoBadge.closest(".badgeOk")).not.toBeNull();
  });

  it("新增缺 canonical id 时本地校验拦下，不发请求", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    const postRaw = vi.spyOn(api, "postRaw");
    render(<SystemStrategyFeature />);
    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.click(screen.getByText("保存"));
    expect(await screen.findByText(/均不能为空/)).toBeInTheDocument();
    expect(postRaw).not.toHaveBeenCalled();
  });

  it("kind=customer_stage 显示状态机软提示，改成 intent_level 后不显示", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    render(<SystemStrategyFeature />);
    fireEvent.click(await screen.findByText("新增条目"));
    // 默认 kind=customer_stage → 软提示可见
    expect(screen.getByText(/状态机灰度.*同步配置/)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("customer_stage"), { target: { value: "intent_level" } });
    expect(screen.queryByText(/状态机灰度.*同步配置/)).not.toBeInTheDocument();
  });
});

describe("DomainProfile 维度配置 participates_in_decision", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  // 让 DomainProfilePanel 进入编辑态（isCreatingProfile=true 渲染右侧 ProfileEditor），
  // 并塞入一条 participates_in_decision=true 的维度。setProfileDraft 改成 spy，断言 onChange 写回。
  function seedProfileDraftWithDimension(setProfileDraft: (draft: DomainProfileDraft) => void) {
    const dim: ProfileDimension = {
      kind: "budget_sensitivity",
      display_name: "预算敏感度",
      participates_in_decision: true,
      description: "客户对价格的敏感程度",
    };
    useStrategyStore.setState({
      editingProfile: null,
      isCreatingProfile: true,
      profileDraft: { profile_id: "test_profile", display_name: "测试配置", profile_dimensions: [dim] },
      setProfileDraft,
      loadDomainProfiles: vi.fn(),
    });
  }

  it("D10: 维度行可切换 participates_in_decision 为只观测维度", async () => {
    const setProfileDraft = vi.fn();
    seedProfileDraftWithDimension(setProfileDraft);

    render(<SystemStrategyFeature />);

    // 维度行的「进决策」复选框初始为 checked（participates_in_decision=true）
    const checkbox = await screen.findByRole("checkbox", { name: "进决策" });
    expect(checkbox).toBeChecked();

    // 取消勾选 → onChange/update 写回该维度 participates_in_decision=false
    fireEvent.click(checkbox);
    await waitFor(() => expect(setProfileDraft).toHaveBeenCalled());
    const lastArg = setProfileDraft.mock.calls.at(-1)?.[0];
    expect(lastArg.profile_dimensions[0].participates_in_decision).toBe(false);
    // 其余字段保持不变（不误删 kind/display_name/description）
    expect(lastArg.profile_dimensions[0].kind).toBe("budget_sensitivity");
    expect(lastArg.profile_dimensions[0].display_name).toBe("预算敏感度");
    expect(lastArg.profile_dimensions[0].description).toBe("客户对价格的敏感程度");
  });
});

describe("DomainProfile completeness 维度 anchor_hint+initial_signal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  // 让 ProfileEditor 进入编辑态并塞入一条 coverage_dimension，setProfileDraft 改成 spy，
  // 断言 anchor_hint / initial_signal 两输入框 onChange 写回数组该项且不误删其它字段。
  function seedProfileDraftWithCoverage(setProfileDraft: (draft: DomainProfileDraft) => void) {
    useStrategyStore.setState({
      editingProfile: null,
      isCreatingProfile: true,
      profileDraft: {
        profile_id: "test_profile",
        display_name: "测试配置",
        coverage_dimensions: [{ key: "need", display_name: "需求", required: true }],
      },
      setProfileDraft,
      loadDomainProfiles: vi.fn(),
    });
  }

  it("D11: completeness 维度可编辑 anchor_hint 与 initial_signal 并提交", async () => {
    const setProfileDraft = vi.fn();
    seedProfileDraftWithCoverage(setProfileDraft);

    render(<SystemStrategyFeature />);

    // anchor_hint 输入框写入文本 → 写回该行 anchor_hint，保留 key/display_name/required
    const anchorInput = await screen.findByPlaceholderText(/anchor_hint/i);
    fireEvent.change(anchorInput, { target: { value: "在对话开场探明" } });
    await waitFor(() => expect(setProfileDraft).toHaveBeenCalled());
    let lastArg = setProfileDraft.mock.calls.at(-1)?.[0];
    expect(lastArg.coverage_dimensions[0].anchor_hint).toBe("在对话开场探明");
    expect(lastArg.coverage_dimensions[0].key).toBe("need");
    expect(lastArg.coverage_dimensions[0].display_name).toBe("需求");
    expect(lastArg.coverage_dimensions[0].required).toBe(true);

    // initial_signal 输入框写入文本 → 写回该行 initial_signal
    const signalInput = screen.getByPlaceholderText(/initial_signal/i);
    fireEvent.change(signalInput, { target: { value: "客户主动提及预算" } });
    await waitFor(() => expect(setProfileDraft.mock.calls.length).toBeGreaterThan(1));
    lastArg = setProfileDraft.mock.calls.at(-1)?.[0];
    expect(lastArg.coverage_dimensions[0].initial_signal).toBe("客户主动提及预算");
    expect(lastArg.coverage_dimensions[0].key).toBe("need");
  });
});