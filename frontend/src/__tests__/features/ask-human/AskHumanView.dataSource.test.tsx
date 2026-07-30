import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";

// 只 mock 网络层（fetchInbox/fetchSummary），保留真实 sortItems —— 这样测试验证的是
// "频道渲染的列表 = store.items（经真实 sortItems 排序）" 这条单数据源链路。
vi.mock("../../../lib/inboxApi", async () => {
  const actual = await vi.importActual<typeof import("../../../lib/inboxApi")>(
    "../../../lib/inboxApi",
  );
  return {
    ...actual,
    fetchInbox: vi.fn(),
    fetchSummary: vi.fn(),
  };
});

import { fetchInbox, fetchSummary, type InboxItem } from "../../../lib/inboxApi";
import { useInboxStore } from "../../../stores/inboxStore";
import AskHumanFeature from "../../../features/ask-human/index";

const fi = fetchInbox as unknown as ReturnType<typeof vi.fn>;
const fs = fetchSummary as unknown as ReturnType<typeof vi.fn>;

// 用未知 source → renderInline 走 default 分支 = 纯 <div>{title}</div>，不触发子组件网络。
function item(
  id: string,
  severity: string,
  ageHours = 0,
  source = "src_default",
  extra: Partial<InboxItem> = {},
): InboxItem {
  return {
    source,
    id,
    title: `t-${id}`,
    summary: "",
    severity,
    createdAt: null,
    ageHours,
    actionKind: "inline",
    ...extra,
  };
}

beforeEach(() => {
  fi.mockReset();
  fs.mockReset();
  // zustand store 全局单例，测试间必须 reset，否则旧 items/activeSource 串台。
  useInboxStore.setState({
    items: [],
    errors: [],
    summary: null,
    loading: false,
    fatalError: null,
    activeSource: null,
  });
});

describe("AskHumanView 单数据源", () => {
  it("渲染的列表来自 store.items（经真实 sortItems 排序，high 冒泡到顶）", async () => {
    // 乱序：low 在首、high 在尾。若渲染的是未排序的原始 fetch 结果，high 不会在顶。
    fi.mockResolvedValue({
      items: [item("low1", "low", 1), item("high1", "high", 2), item("med1", "medium", 3)],
      errors: [],
    });
    fs.mockResolvedValue({
      status: "complete",
      asOf: null,
      counts: { principalEscalation: 0 },
      errors: [],
      total: 0,
    });

    const { container } = render(<AskHumanFeature />);

    await screen.findByText("t-high1");
    const list = container.querySelector(".reviewQueueList")!;
    const rows = within(list as HTMLElement).getAllByText(/^t-/);
    // 排序后顺序应为 high → medium → low（severity 降序）。
    expect(rows.map((r) => r.textContent)).toEqual(["t-high1", "t-med1", "t-low1"]);
  });

  it("单次刷新只 fetch 一次 inbox（mount 1 次 + 刷新 1 次 = 2，非旧的双倍）", async () => {
    fi.mockResolvedValue({ items: [item("a", "high")], errors: [] });
    fs.mockResolvedValue({ status: "complete", asOf: null, counts: {}, errors: [], total: 0 });

    render(<AskHumanFeature />);
    await screen.findByText("t-a");
    // mount：ReviewQueue 挂载 → fetchItems → store.load → fetchInbox 1 次。
    // （旧实现 mount 时 useEffect load + ReviewQueue 各 fetch 一次 = 2 次）
    expect(fi).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByText("刷新"));
    // 刷新：refreshNonce bump → ReviewQueue refetch → store.load → fetchInbox 再 1 次 = 累计 2。
    // （旧实现 refreshAll 还会额外 void load 一次 → 累计 +2）
    await waitFor(() => expect(fi).toHaveBeenCalledTimes(2));
  });

  it("降级一致：刷新失败时 fatalError 横幅出现且列表仍显旧数据（不走 error 短路）", async () => {
    fi.mockResolvedValueOnce({ items: [item("keep", "high")], errors: [] });
    fs.mockResolvedValue({ status: "complete", asOf: null, counts: {}, errors: [], total: 0 });

    render(<AskHumanFeature />);
    await screen.findByText("t-keep");

    // 第二次 fetchInbox（刷新）失败 → store.load catch 保留旧 items + 置 fatalError，不 rethrow。
    fi.mockRejectedValueOnce(new Error("network down"));
    fireEvent.click(screen.getByText("刷新"));

    // 横幅出现。
    await screen.findByText(/加载失败（显示上次数据）/);
    // 列表仍显旧数据，而非 ReviewQueue 自己的 "加载失败" 短路空白。
    expect(screen.getByText("t-keep")).toBeTruthy();
    expect(screen.queryByText(/^加载失败：/)).toBeNull();
  });

  it("切源：点 chip 后列表变为该 source 的数据（setActiveSource + nonce + key 时序）", async () => {
    // 默认全部源返回 itemAll；切到 knowledge_review 源返回 itemKR。
    fi.mockImplementation((source?: string) =>
      Promise.resolve({
        items:
          source === "knowledge_review"
            ? [item("kr", "high")]
            : [item("all", "high")],
        errors: [],
      }),
    );
    fs.mockResolvedValue({
      status: "complete",
      asOf: null,
      counts: { knowledgeReview: 1 },
      errors: [],
      total: 1,
    });

    render(<AskHumanFeature />);
    await screen.findByText("t-all");

    // chip 文案 "知识核验: 1"。
    fireEvent.click(screen.getByText(/知识核验/));
    await screen.findByText("t-kr");
    expect(screen.queryByText("t-all")).toBeNull();
    // 验证 fetchInbox 确实带上了该 source。
    expect(fi).toHaveBeenCalledWith("knowledge_review");
  });

  // 接线验证：renderItem 是内联闭包，只能经整棵视图渲染来验证 tag 是否被真实接线（而非只测 InboxRow 本身）。
  it("接线：knowledge_review + needs_human_audit 的 item 显 held 徽章;其它 integrityStatus 不显", async () => {
    fi.mockResolvedValue({
      items: [
        item("audit", "high", 2, "knowledge_review", { integrityStatus: "needs_human_audit" }),
        item("plain", "medium", 1, "knowledge_review", { integrityStatus: "verified" }),
      ],
      errors: [],
    });
    fs.mockResolvedValue({
      status: "complete",
      asOf: null,
      counts: { knowledgeReview: 2 },
      errors: [],
      total: 2,
    });

    render(<AskHumanFeature />);
    await screen.findByText("t-audit");

    // needs_human_audit 的那条经真实 renderItem 接线后出现 held 徽章。
    const badges = screen.getAllByText("AI预审通过·待复核");
    expect(badges).toHaveLength(1);
    // 徽章确用 held 色类（复用 --fill-held，不新造色）。
    expect(badges[0].className).toContain("inboxTag--held");
    // verified 的那条不显徽章（tag=undefined）。
    expect(screen.getByText("t-plain")).toBeTruthy();
  });

  it("summary source errors render unavailable instead of a false zero", async () => {
    fi.mockResolvedValue({ items: [item("a", "high")], errors: [] });
    fs.mockResolvedValue({
      status: "partial",
      asOf: "2026-07-20T00:00:00Z",
      counts: { principalEscalation: null, knowledgeReview: 3 },
      errors: [{ source: "principal_escalation", error: "count unavailable" }],
      total: null,
    });

    render(<AskHumanFeature />);
    await screen.findByText("t-a");
    expect(screen.getByText("请示裁决: 不可用")).toBeTruthy();
    expect(screen.queryByText("请示裁决: 0")).toBeNull();
    expect(screen.getByText("知识核验: 3")).toBeTruthy();
  });
});
