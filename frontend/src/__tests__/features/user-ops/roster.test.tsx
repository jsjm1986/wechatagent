// Task 8：通讯录视图（RosterView）组件测试。
// 验证：(1) 全量好友渲染（含 managed / not_imported 混合），managed 行 checkbox 禁用不可勾选；
//       (2) 勾选 2 条 not_imported + 填共享运营备注 + 点「加入 Agent 运营」→ POST /contacts/batch-enable，
//           body 含 accountId / candidates(len 2) / sharedNote（camelCase wire 键）。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
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

// 手动控制 resolve 时序的 promise，用于制造"旧账号请求晚于新账号请求返回"的竞态。
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
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

  // 竞态回归：快速切账号 A→B，若账号 A 的 roster 响应晚于账号 B 返回，
  // 过时的 A 结果不得覆盖已选中账号 B 的列表（请求序号守卫）。
  it("切账号竞态：过时账号的迟到响应不覆盖当前账号列表", async () => {
    // 两个账号可选（RosterView 仅在 accounts.length > 1 时渲染账号下拉）。
    useAccountStore.setState({
      accounts: [
        { accountId: "accA", alias: "账号A", displayName: "账号A", online: true } as never,
        { accountId: "accB", alias: "账号B", displayName: "账号B", online: true } as never,
      ],
      selectedAccountId: "accA",
    });

    const ROSTER_A = [
      { wxid: "wx_a_only", nickname: "A的好友", remark: null, avatarUrl: null, agentStatus: "not_imported" },
    ];
    const ROSTER_B = [
      { wxid: "wx_b_only", nickname: "B的好友", remark: null, avatarUrl: null, agentStatus: "not_imported" },
    ];

    const defA = deferred<{ items: typeof ROSTER_A }>();
    const defB = deferred<{ items: typeof ROSTER_B }>();
    // 按 url 里的 accountId 路由到各自的 deferred：A 先发起、B 后发起，
    // 但下面刻意让 B 先 resolve、A 后 resolve（迟到）。
    getMock.mockImplementation((url: string) => {
      if (url.includes("accountId=accA")) return defA.promise;
      if (url.includes("accountId=accB")) return defB.promise;
      return Promise.resolve({ items: [] });
    });

    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );

    // 挂载即对账号 A 发起 roster 请求（seq=1，pending）。
    await waitFor(() => {
      expect(getMock.mock.calls.some((c) => String(c[0]).includes("accountId=accA"))).toBe(true);
    });

    // 切到账号 B（seq=2，pending）——effectiveAccountId 变 → useEffect 重跑 refresh。
    act(() => {
      useAccountStore.getState().selectAccount("accB");
    });
    await waitFor(() => {
      expect(getMock.mock.calls.some((c) => String(c[0]).includes("accountId=accB"))).toBe(true);
    });

    // 账号 B 先返回（最新请求）→ 列表落 B。
    await act(async () => {
      defB.resolve({ items: ROSTER_B });
    });
    expect(await screen.findByText("B的好友")).toBeInTheDocument();

    // 账号 A 迟到返回（过时请求）→ 守卫应丢弃，列表仍是 B、不得出现 A 的好友。
    await act(async () => {
      defA.resolve({ items: ROSTER_A });
    });

    // 给过时响应一个落地的机会：仍应看到 B、看不到 A。
    await waitFor(() => {
      expect(screen.getByText("B的好友")).toBeInTheDocument();
    });
    expect(screen.queryByText("A的好友")).not.toBeInTheDocument();
  });
});
