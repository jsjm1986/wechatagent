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

    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "customer_success" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "成交维护" } });
    // 勾两个复选（按 label 文案中的 flag 名定位）。
    fireEvent.click(screen.getByLabelText(/is_reactivation_target/));
    fireEvent.click(screen.getByLabelText(/is_terminal/));
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

    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "first_contact" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "初次接触" } });
    fireEvent.click(screen.getByText("保存"));

    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    const [, body] = postRaw.mock.calls[0] as [string, { value: Record<string, unknown> }];
    expect(body.value.isTerminal).toBe(false);
    expect(body.value.isReactivationTarget).toBe(false);
  });
});
