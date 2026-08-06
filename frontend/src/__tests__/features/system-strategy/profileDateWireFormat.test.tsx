// 复现：GET /api/admin/domain-profiles 返回的 updated_at/created_at 是 bson::DateTime
// 经 serde_json 的扩展 JSON 对象 {"$date":{"$numberLong":"..."}}，而非字符串。
// 前端把它当文本渲染 → React "Objects are not valid as a React child" → 整个频道白屏。
//
// 注意：本文件的 mock 用**线上实测形态**（scripts 从 mongo 取出后 serde_json 序列化所得），
// 既有 domainProfileVersions.test.tsx 用的是字符串 "2026-06-26T00:00:00Z"，
// 所以那些测试永远抓不到这个 bug。
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { api } from "../../../lib/api";
import { useUiStore } from "../../../stores/uiStore";

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

// 线上 /api/admin/domain-profiles 实际返回形态（117.72.54.28，两条 profile 均如此）。
const BSON_DATE = { $date: { $numberLong: "1782458710964" } } as unknown as string;

const profileWithBsonDate = {
  id: "6a3e29579d28a161324c2d80",
  profile_id: "sales-with-lifecycle-example",
  workspace_id: "default",
  display_name: "销售 + 购买生命周期（示例草稿）",
  description: "示例草稿",
  profile_dimensions: [],
  prompt_fragment: "",
  conversation_modes: [],
  business_formulas: [],
  commitment_markers: { product_effect: [], tone_only: [] },
  coverage_dimensions: [],
  version: 1,
  current_version: false,
  previous_version: null,
  release_status: "published" as const,
  is_active: false,
  seeded_by: "g1_migration",
  created_at: BSON_DATE,
  updated_at: BSON_DATE,
};

function selectTab(name: "总控与 Prompt" | "标签与状态" | "行业配置" | "经验教训") {
  fireEvent.click(screen.getByRole("button", { name }));
}

describe("domain profile 时间字段 wire 形态", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "", setBusy: vi.fn(), setError: vi.fn() });
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url === "/api/admin/domain-profiles"
          ? { items: [profileWithBsonDate] }
          : { items: [] }) as never
      )
    );
  });

  it("列表项在 updated_at 为 bson 扩展 JSON 对象时仍能渲染（不抛 React child 错）", async () => {
    render(<SystemStrategyFeature />);
    selectTab("行业配置");
    // 只要 profile 名字渲染出来，就说明没有在 profileListMeta 处崩溃。
    expect(await screen.findByText(/销售 \+ 购买生命周期/)).toBeInTheDocument();
  });
});
