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

// tab 化后：非默认 tab 的面板要先切 tab 才渲染。默认 tab = 总控与 Prompt。
function selectTab(name: "总控与 Prompt" | "标签与状态" | "行业配置" | "经验教训") {
  fireEvent.click(screen.getByRole("button", { name }));
}

// 「新增条目」按钮 disabled={busy||loading}：TaxonomiesAdmin 挂载即 reload() 置 loading=true，
// findByText 只等文字出现、不等 loading 落地，CI 慢机上点击会落在 disabled 窗口内→表单不展开。
// 必须等按钮 enabled 再点，否则 showCreate 不翻转、canonical id 输入框永不渲染（真实竞态，非 mock 污染）。
async function openCreateForm() {
  const btn = await screen.findByText("新增条目");
  await waitFor(() => expect(btn).not.toBeDisabled());
  fireEvent.click(btn);
}

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
  it("一体化迁移：总控/Prompt/状态机/字典/教训面板小标题在各自 tab 渲染", () => {
    render(<SystemStrategyFeature />);
    // control tab（默认）
    expect(screen.getByText("系统总控 Prompt")).toBeInTheDocument();
    // taxonomy tab
    selectTab("标签与状态");
    expect(screen.getByText("状态机动作策略灰度")).toBeInTheDocument();
    expect(screen.getByText("双层标签字典灰度")).toBeInTheDocument();
    // lessons tab
    selectTab("经验教训");
    expect(screen.getByText("跨用户教训归纳（14d 滑窗）")).toBeInTheDocument();
  });

  it("一体化迁移：各 tab 空态渲染，重置 Prompt Pack 按钮在总控 tab 可见", async () => {
    render(<SystemStrategyFeature />);
    // control tab
    expect(screen.getByText("重置系统提示词包 v2")).toBeInTheDocument();
    // taxonomy tab 空态
    selectTab("标签与状态");
    await waitFor(() => {
      expect(screen.getByText("暂无状态策略")).toBeInTheDocument();
    });
    expect(screen.getByText("暂无字典条目")).toBeInTheDocument();
    // lessons tab 空态（tab 化后面板切入即挂载，reload 异步 settle 后现空态）
    selectTab("经验教训");
    expect(await screen.findByText("暂无教训聚合（窗口内无命中样本）")).toBeInTheDocument();
  });

  it("SR-138: reset 取消时零调用，输入精确认短语后才提交", async () => {
    const reset = vi.fn().mockResolvedValue(undefined);
    useStrategyStore.setState({ resetSystemPromptPack: reset });
    render(<SystemStrategyFeature />);

    fireEvent.click(screen.getByRole("button", { name: "重置系统提示词包 v2" }));
    expect(await screen.findByText("重置系统提示词包？")).toBeInTheDocument();
    expect(screen.getByText(/替换当前 workspace 的 Prompt、Playbook 与 Domain Config/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(reset).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "重置系统提示词包 v2" }));
    const submit = await screen.findByRole("button", { name: "确认重置" });
    expect(submit).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText("RESET PROMPT PACK"), {
      target: { value: "RESET PROMPT PACK" },
    });
    expect(submit).not.toBeDisabled();
    fireEvent.click(submit);
    await waitFor(() => expect(reset).toHaveBeenCalledWith("RESET PROMPT PACK"));
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
    selectTab("标签与状态");

    await openCreateForm();
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
    value: { id: "need_discovery", label: "需求挖掘", aliases: ["挖需求"], description: "", status: "active", priorityWeight: 60, isTerminal: true, isReactivationTarget: true },
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

  it("普通改名只 PATCH label，不覆盖运行时 taxonomy flags", async () => {
    seedTaxonomyGet(activeItem);
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item: activeItem } as never);
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    fireEvent.click(await screen.findByText("编辑"));
    fireEvent.change(screen.getByDisplayValue("需求挖掘"), { target: { value: "需求探索阶段" } });
    fireEvent.click(screen.getByText("保存编辑"));
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch).toHaveBeenCalledWith("/api/admin/taxonomies/id_active", { label: "需求探索阶段" });
  });

  it("active 条目显示「废弃」，点击调 api.delete", async () => {
    seedTaxonomyGet(activeItem);
    const del = vi.spyOn(api, "delete").mockResolvedValue({ ok: true } as never);
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    fireEvent.click(await screen.findByText("废弃"));
    await waitFor(() => expect(del).toHaveBeenCalledWith("/api/admin/taxonomies/id_active"));
  });

  it("deprecated 条目显示「恢复」，点击调 api.patch {deprecated:false}", async () => {
    seedTaxonomyGet(deprecatedItem);
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item: deprecatedItem } as never);
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
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
    selectTab("标签与状态");
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
    selectTab("标签与状态");
    await openCreateForm();
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
    selectTab("标签与状态");
    await openCreateForm();
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.click(screen.getByText("保存"));
    expect(await screen.findByText(/均不能为空/)).toBeInTheDocument();
    expect(postRaw).not.toHaveBeenCalled();
  });

  it("kind=customer_stage 显示状态机软提示，改成 intent_level 后不显示", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await openCreateForm();
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
    selectTab("行业配置");

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
  // 断言 review_topic_aliases / anchor_hint / initial_signal 写回数组该项且不误删其它字段。
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

  it("D11: completeness 维度可编辑评审主题别名、anchor_hint 与 initial_signal", async () => {
    const setProfileDraft = vi.fn();
    seedProfileDraftWithCoverage(setProfileDraft);

    render(<SystemStrategyFeature />);
    selectTab("行业配置");

    const aliasesInput = await screen.findByPlaceholderText(/review_topic_aliases/i);
    fireEvent.change(aliasesInput, { target: { value: "预算, 报价，计费" } });
    await waitFor(() => expect(setProfileDraft).toHaveBeenCalled());
    let lastArg = setProfileDraft.mock.calls.at(-1)?.[0];
    expect(lastArg.coverage_dimensions[0].review_topic_aliases).toEqual(["预算", "报价", "计费"]);
    expect(lastArg.coverage_dimensions[0].key).toBe("need");

    // anchor_hint 输入框写入文本 → 写回该行 anchor_hint，保留 key/display_name/required
    const anchorInput = screen.getByPlaceholderText(/anchor_hint/i);
    fireEvent.change(anchorInput, { target: { value: "在对话开场探明" } });
    await waitFor(() => expect(setProfileDraft).toHaveBeenCalled());
    lastArg = setProfileDraft.mock.calls.at(-1)?.[0];
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

// 候选审核面板分页（消除 176 条全平铺致页面无限长；每页 20 条，复用 CampaignBoard 分页模式）。
describe("TaxonomyCandidatesAdmin 分页", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "", setBusy: vi.fn(), setError: vi.fn() });
  });

  function makeCandidates(n: number) {
    return Array.from({ length: n }, (_, i) => ({
      id: `cand${i}`,
      scope: "global",
      kind: "churn_reason",
      rawValue: `候选词${i}`,
      evidence: null,
      confidence: 0.5,
      occurrences: 1,
      status: "pending",
      firstSeenAt: null,
      lastSeenAt: null,
      reviewedAt: null,
      reviewedBy: null,
      suggestedDisplayName: null,
    }));
  }

  it("候选 25 条时首页只渲染 20 条并显示翻页控件，点下一页显示其余 5 条", async () => {
    // 候选面板挂载即 GET /api/admin/taxonomy-candidates?status=pending
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(25) } : { items: [] }) as never,
      ),
    );

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");

    // 首页：候选词0/候选词19 可见，候选词20 不可见（被分页切掉）。
    // 用 heading 精确匹配（rawValue 落在 <h3>），避免「候选词2」子串匹配「候选词20/21…」。
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "候选词19" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "候选词20" })).toBeNull();
    // 页码 1 / 2
    expect(screen.getByText("1 / 2")).toBeInTheDocument();

    // 点「下一页」→ 第 21..25 条可见，第 1 条移出
    fireEvent.click(screen.getByText("下一页"));
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词20" })).toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "候选词24" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "候选词0" })).toBeNull();
    expect(screen.getByText("2 / 2")).toBeInTheDocument();
  });

  it("候选 ≤20 条时不显示翻页控件", async () => {
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(20) } : { items: [] }) as never,
      ),
    );

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");

    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());
    expect(screen.getByRole("heading", { name: "候选词19" })).toBeInTheDocument();
    // 正好 20 条 = 1 页，无翻页控件
    expect(screen.queryByText("下一页")).toBeNull();
    expect(screen.queryByText("上一页")).toBeNull();
  });

  it("选择 kind 下拉后重新请求候选列表并带上 kind= 参数", async () => {
    const getSpy = vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(3) } : { items: [] }) as never,
      ),
    );

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");

    // 初次挂载：只带 status=pending，不带 kind=
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());
    expect(
      getSpy.mock.calls.some(
        ([u]) => typeof u === "string" && u.includes("/api/admin/taxonomy-candidates") && !u.includes("kind="),
      ),
    ).toBe(true);

    // 选 kind = objection_type（异议类型）
    const kindSelect = screen.getByTestId("candidate-kind-filter") as HTMLSelectElement;
    fireEvent.change(kindSelect, { target: { value: "objection_type" } });

    // 重新请求带上 kind=objection_type
    await waitFor(() =>
      expect(
        getSpy.mock.calls.some(
          ([u]) => typeof u === "string" && u.includes("/api/admin/taxonomy-candidates") && u.includes("kind=objection_type"),
        ),
      ).toBe(true),
    );
  });

  it("批量驳回：勾选 2 条 pending + 填原因 + 确认 → 发 2 次 reject 请求", async () => {
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(3) } : { items: [] }) as never,
      ),
    );
    const postSpy = vi.spyOn(api, "post").mockResolvedValue({} as never);

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());

    // 勾选 cand0、cand1
    fireEvent.click(screen.getByTestId("candidate-check-cand0"));
    fireEvent.click(screen.getByTestId("candidate-check-cand1"));

    // 填驳回原因
    fireEvent.change(screen.getByTestId("bulk-reject-reason"), { target: { value: "无业务相关性" } });

    // 点批量驳回 → 弹确认窗
    fireEvent.click(screen.getByTestId("bulk-reject-btn"));

    // 确认弹窗（useConfirm 渲染 confirmText="确认驳回"）
    fireEvent.click(await screen.findByText("确认驳回"));

    // 发出 2 次 reject POST，带 reason
    await waitFor(() => expect(postSpy).toHaveBeenCalledTimes(2));
    expect(postSpy).toHaveBeenCalledWith(
      "/api/admin/taxonomy-candidates/cand0/reject",
      { reason: "无业务相关性" },
    );
    expect(postSpy).toHaveBeenCalledWith(
      "/api/admin/taxonomy-candidates/cand1/reject",
      { reason: "无业务相关性" },
    );
  });

  it("批量驳回按钮：未勾选或未填原因时 disabled", async () => {
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(3) } : { items: [] }) as never,
      ),
    );
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());

    // 未勾选 → disabled
    expect(screen.getByTestId("bulk-reject-btn")).toBeDisabled();

    // 勾一条但没填原因 → 仍 disabled
    fireEvent.click(screen.getByTestId("candidate-check-cand0"));
    expect(screen.getByTestId("bulk-reject-btn")).toBeDisabled();

    // 填原因 → enabled
    fireEvent.change(screen.getByTestId("bulk-reject-reason"), { target: { value: "重复" } });
    expect(screen.getByTestId("bulk-reject-btn")).not.toBeDisabled();
  });

  it("非 pending 视图（已采纳）不渲染复选框与批量驳回入口", async () => {
    vi.spyOn(api, "get").mockImplementation((url: string) => {
      if (url.includes("/api/admin/taxonomy-candidates")) {
        // 返回 1 条 approved 候选（无论 status filter，简化 mock）
        const items = makeCandidates(1).map((c) => ({ ...c, status: "approved" }));
        return Promise.resolve({ items } as never);
      }
      return Promise.resolve({ items: [] } as never);
    });

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());

    // 切到「已采纳」status filter
    fireEvent.click(screen.getByRole("button", { name: "已采纳" }));

    await waitFor(() => {
      expect(screen.queryByTestId("bulk-reject-bar")).toBeNull();
    });
    expect(screen.queryByTestId("candidate-check-cand0")).toBeNull();
  });
});
