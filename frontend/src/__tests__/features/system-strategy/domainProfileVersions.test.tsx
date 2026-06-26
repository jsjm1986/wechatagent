import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { api } from "../../../lib/api";
import { useUiStore } from "../../../stores/uiStore";

// CSS module identity 代理（与 systemStrategy.test.tsx 同款）：vitest css:false 会把
// styles.xxx 解析为 undefined，这里保持 className 字符串可定位。
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

// 生效中的 domain profile：current_version=true + previous_version 非空 → ActiveVersionsBar
// 应渲染「回滚到 v1」「发布新版本」。id 是后端 :id 期望的 ObjectId hex。
const activeProfile = {
  id: "65f0aabbccddeeff00112233",
  profile_id: "edu-k12-tuition",
  workspace_id: "default",
  display_name: "K12 辅导",
  description: "",
  profile_dimensions: [],
  prompt_fragment: "",
  conversation_modes: [],
  business_formulas: [],
  commitment_markers: { product_effect: [], tone_only: [] },
  coverage_dimensions: [],
  version: 2,
  current_version: true,
  previous_version: 1,
  is_active: true,
  seeded_by: "manual",
  updated_at: "2026-06-26T00:00:00Z",
};

describe("D4 domain-profiles 版本回滚 UI", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "", setBusy: vi.fn(), setError: vi.fn() });
    // 仅 /api/admin/domain-profiles 列表端点返回生效 profile，其余面板保持空态。
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url === "/api/admin/domain-profiles" ? { items: [activeProfile] } : { items: [] }) as never
      )
    );
  });

  it("DomainProfilePanel renders ActiveVersionsBar with domain-profiles endpoint", async () => {
    render(<SystemStrategyFeature />);
    // ActiveVersionsBar 在生效 profile（current+有 previous）下渲染回滚 + 发布按钮。
    expect(await screen.findByText("回滚到 v1")).toBeInTheDocument();
    expect(screen.getByText("发布新版本")).toBeInTheDocument();
  });

  it("点击回滚调用 POST /api/admin/domain-profiles/:id/rollback（:id 为 ObjectId hex）", async () => {
    const post = vi.spyOn(api, "post").mockResolvedValue({} as never);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<SystemStrategyFeature />);
    fireEvent.click(await screen.findByText("回滚到 v1"));
    await waitFor(() =>
      expect(post).toHaveBeenCalledWith(
        "/api/admin/domain-profiles/65f0aabbccddeeff00112233/rollback",
        {}
      )
    );
    confirmSpy.mockRestore();
  });
});
