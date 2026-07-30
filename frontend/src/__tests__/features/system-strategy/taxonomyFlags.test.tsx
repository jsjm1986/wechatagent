import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { api } from "../../../lib/api";
import { useUiStore } from "../../../stores/uiStore";

// 与 systemStrategy.test.tsx 同款 CSS module identity mock：vitest css:false 下
// styles.xxx 会解析成 undefined，className 不落 DOM。代理成 identity 让结构稳定。
vi.mock("../../../features/system-strategy/SystemStrategy.module.css", () => ({
  default: new Proxy({}, { get: (_t, key) => String(key) }),
}));

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

// Task 7 / D6：字典两 flag（is_terminal / is_reactivation_target）配置入口。
// 验证 create 表单勾选两复选后，submit body 的 value 对象带 camelCase 键且为 true——
// 这是"改字典即通用"在 UI 能真正写入终态 / 再激活语义的回归锁。
// tab 化后：TaxonomiesAdmin 在「标签与状态」tab，需先切 tab 才渲染。
function selectTab(name: "总控与 Prompt" | "标签与状态" | "行业配置" | "经验教训") {
  fireEvent.click(screen.getByRole("button", { name }));
}

// 「新增条目」disabled={busy||loading}：挂载 reload() 置 loading，findByText 不等 loading 落地，
// CI 慢机点击落在 disabled 窗口→表单不展开。等按钮 enabled 再点（与 systemStrategy.test.tsx 同源竞态）。
async function openCreateForm() {
  const btn = await screen.findByText("新增条目");
  await waitFor(() => expect(btn).not.toBeDisabled());
  fireEvent.click(btn);
}

describe("TaxonomiesAdmin 终态 / 再激活 flag 配置（D6）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({
      busy: false,
      error: "",
      setBusy: vi.fn(),
      setError: vi.fn(),
    });
  });

  it("勾选两复选后，create submit body 携带 value.isTerminal===true / value.isReactivationTarget===true", async () => {
    const postRaw = vi.spyOn(api, "postRaw").mockResolvedValue({ ok: true, status: 200, data: { item: {} } });
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");

    await openCreateForm();
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "customer_success" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "成交维护" } });
    // 勾两个复选（按 label 文案中的 flag 名定位）。
    fireEvent.click(screen.getByLabelText(/可作再激活目标/));
    fireEvent.click(screen.getByLabelText(/终态/));
    fireEvent.click(screen.getByText("保存"));

    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    const [, body] = postRaw.mock.calls[0] as [string, { value: Record<string, unknown> }];
    expect(body.value.isTerminal).toBe(true);
    expect(body.value.isReactivationTarget).toBe(true);
  });

  it("不勾选时 create submit body 两 flag 为 false（默认向后兼容）", async () => {
    const postRaw = vi.spyOn(api, "postRaw").mockResolvedValue({ ok: true, status: 200, data: { item: {} } });
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");

    await openCreateForm();
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "first_contact" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "初次接触" } });
    fireEvent.click(screen.getByText("保存"));

    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    const [, body] = postRaw.mock.calls[0] as [string, { value: Record<string, unknown> }];
    expect(body.value.isTerminal).toBe(false);
    expect(body.value.isReactivationTarget).toBe(false);
  });

  it("编辑时完整回显 flags，显式切换只 PATCH 变化的 flag", async () => {
    const item = {
      id: "taxonomy-runtime-flags",
      scope: "global",
      kind: "customer_stage",
      value: {
        id: "dormant_reactivation",
        label: "休眠再激活",
        aliases: [],
        description: "",
        status: "active",
        priorityWeight: 10,
        isTerminal: true,
        isReactivationTarget: true,
      },
      version: 1,
      currentVersion: true,
      previousVersion: null,
      seededBy: "system",
      updatedAt: "",
    };
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve((url.includes("/api/admin/taxonomies") ? { items: [item] } : { items: [] }) as never),
    );
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item } as never);

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    fireEvent.click(await screen.findByText("编辑"));
    expect(screen.getByLabelText(/可作再激活目标/)).toBeChecked();
    expect(screen.getByLabelText(/终态/)).toBeChecked();
    fireEvent.click(screen.getByLabelText(/可作再激活目标/));
    fireEvent.click(screen.getByText("保存编辑"));

    await waitFor(() => expect(patch).toHaveBeenCalledWith(
      "/api/admin/taxonomies/taxonomy-runtime-flags",
      { isReactivationTarget: false },
    ));
  });
});
