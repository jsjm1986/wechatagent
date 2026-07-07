// Task 8：通讯录视图（RosterView）组件测试。
// 验证：(1) 全量好友渲染（含 managed / not_imported 混合），managed 行 checkbox 禁用不可勾选；
//       (2) 勾选 2 条 not_imported + 填共享运营备注 + 点「加入 Agent 运营」→ POST /contacts/batch-enable，
//           body 含 accountId / candidates(len 2) / sharedNote（camelCase wire 键）。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RosterView } from "../../../features/user-ops/RosterView";
import { ToastProvider } from "../../../components/ui/Toast";
import { useAccountStore } from "../../../stores/accountStore";

const getMock = vi.fn();
const postMock = vi.fn();

vi.mock("../../../lib/api", () => ({
  api: {
    get: (url: string) => getMock(url),
    post: (url: string, body: unknown) => postMock(url, body),
  },
}));

const ROSTER = [
  { wxid: "wx_managed", nickname: "已托管的人", remark: "老客户", avatarUrl: "http://img/m", agentStatus: "managed" },
  { wxid: "wx_new1", nickname: "新好友一", remark: null, avatarUrl: null, agentStatus: "not_imported" },
  { wxid: "wx_new2", nickname: "新好友二", remark: "潜在客户", avatarUrl: "http://img/2", agentStatus: "not_imported" },
];

function seedAccount() {
  useAccountStore.setState({
    accounts: [
      {
        accountId: "acc1",
        alias: "测试账号",
        displayName: "测试账号",
        online: true,
      } as never,
    ],
    selectedAccountId: "acc1",
  });
}

describe("RosterView — 通讯录批量托管视图（Task 8）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getMock.mockResolvedValue({ items: ROSTER });
    postMock.mockResolvedValue({ enabled: 2, queued: 2 });
    seedAccount();
  });

  it("渲染全量好友，managed 行不可勾选", async () => {
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    // 三条好友都应渲染。显示名优先级 remark > nickname > wxid（对齐 ContactsView）：
    // wx_managed 有 remark「老客户」、wx_new1 无 remark 显 nickname「新好友一」、wx_new2 有 remark「潜在客户」。
    expect(await screen.findByText("老客户")).toBeInTheDocument();
    expect(screen.getByText("新好友一")).toBeInTheDocument();
    expect(screen.getByText("潜在客户")).toBeInTheDocument();
    // 已托管徽标可见。
    expect(screen.getByText("已托管")).toBeInTheDocument();
    // managed 行对应的卡片按钮 disabled。
    const managedCard = screen.getByText("老客户").closest("button");
    expect(managedCard).toBeDisabled();
  });

  it("勾选 2 条 + 填备注 + 提交 → POST batch-enable 带 camelCase body", async () => {
    const user = userEvent.setup();
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );

    // 勾选两个未导入好友（wx_new1 显 nickname「新好友一」、wx_new2 显 remark「潜在客户」）。
    await user.click((await screen.findByText("新好友一")).closest("button") as HTMLButtonElement);
    await user.click(screen.getByText("潜在客户").closest("button") as HTMLButtonElement);

    // 底部操作条出现「已选 2 人」。
    expect(await screen.findByText(/已选 2 人/)).toBeInTheDocument();

    // 填共享运营备注。
    const note = screen.getByPlaceholderText(/本批运营备注/);
    await user.type(note, "地产意向客户，热情专业");

    // 点「加入 Agent 运营」。
    await user.click(screen.getByText("加入 Agent 运营").closest("button") as HTMLButtonElement);

    await waitFor(() => {
      const call = postMock.mock.calls.find((c) => String(c[0]).includes("/contacts/batch-enable"));
      expect(call).toBeTruthy();
      const body = call![1] as {
        accountId: string;
        candidates: { wxid: string }[];
        sharedNote: string;
      };
      expect(body.accountId).toBe("acc1");
      expect(Array.isArray(body.candidates)).toBe(true);
      expect(body.candidates).toHaveLength(2);
      expect(body.candidates.map((c) => c.wxid).sort()).toEqual(["wx_new1", "wx_new2"]);
      expect(body.sharedNote).toBe("地产意向客户，热情专业");
    });
  });
});
