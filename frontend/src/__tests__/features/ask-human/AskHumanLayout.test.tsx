import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";

// 只 mock 网络层，保留真实 store/排序——验证的是渲染结构，不是数据管道。
vi.mock("../../../lib/inboxApi", async () => {
  const actual = await vi.importActual<typeof import("../../../lib/inboxApi")>(
    "../../../lib/inboxApi",
  );
  return { ...actual, fetchInbox: vi.fn(), fetchSummary: vi.fn() };
});

import { fetchInbox, fetchSummary, type InboxItem } from "../../../lib/inboxApi";
import { useInboxStore } from "../../../stores/inboxStore";
import AskHumanFeature from "../../../features/ask-human/index";

const fi = fetchInbox as unknown as ReturnType<typeof vi.fn>;
const fs = fetchSummary as unknown as ReturnType<typeof vi.fn>;

function item(id: string): InboxItem {
  return {
    source: "src_default", // 未知 source → renderInline 走 default，纯 div，不触发子组件网络
    id,
    title: `t-${id}`,
    summary: "",
    severity: "high",
    createdAt: null,
    ageHours: 0,
    actionKind: "inline",
  };
}

/** Task 2、3 复用：铺好网络返回值。total 传 null 可模拟计数不可用。 */
function mockInbox(items: InboxItem[], total: number | null = items.length) {
  fi.mockResolvedValue({ items, errors: [] });
  fs.mockResolvedValue({
    status: "complete",
    asOf: null,
    counts: { principalEscalation: 1, knowledgeReview: 2 },
    errors: [],
    total,
  });
}

/** Task 2、3 复用：渲染整个频道（含 Confirm/Toast provider，由 feature 默认导出自带）。 */
function renderInbox() {
  return render(<AskHumanFeature />);
}

beforeEach(() => {
  fi.mockReset();
  fs.mockReset();
  // zustand 全局单例，测试间必须 reset，否则旧 items/activeSource 串台。
  useInboxStore.setState({
    items: [],
    errors: [],
    summary: null,
    loading: false,
    fatalError: null,
    activeSource: null,
    requestGeneration: 0,
    summaryRequestGeneration: 0,
  });
});

describe("统一收件箱外壳结构", () => {
  it("内容包在白卡内，且白卡不占用全局 .panel 类名", async () => {
    mockInbox([item("a")]);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const panel = container.querySelector(".askHumanPanel");
    expect(panel).not.toBeNull();
    // 列表必须在白卡内部，而非与白卡并列贴在灰底上。
    expect(panel!.querySelector(".reviewQueueList")).not.toBeNull();
    // 不得占用全局 .panel（user-ops 频道在用，会相互污染）。
    expect(container.querySelector(".panel")).toBeNull();
    expect(container.querySelector(".panelHead")).toBeNull();
  });

  it("panelHead 显示待处理总数，按钮组仍在其内", async () => {
    mockInbox([item("a"), item("b")], 7);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const head = container.querySelector(".askHumanPanelHead");
    expect(head).not.toBeNull();
    expect(head!.textContent).toContain("待处理 7 项");
    // 按钮组容器仍是 .askHumanHeader —— 「刷新」按钮的样式全靠
    // `.askHumanHeader button` 这条规则兜，改名会让它掉回全局蓝色基线。
    expect(head!.querySelector(".askHumanHeader")).not.toBeNull();
    expect(screen.getByText("刷新")).toBeTruthy();
  });

  it("total 为 null 时不显示计数（null 表示不可用，不是 0）", async () => {
    mockInbox([item("a")], null);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const head = container.querySelector(".askHumanPanelHead")!;
    expect(head.textContent).not.toContain("待处理 0 项");
    expect(head.textContent).not.toMatch(/待处理\s*\d+\s*项/);
    // 计数缺失不影响按钮可用。
    expect(screen.getByText("刷新")).toBeTruthy();
  });
});
