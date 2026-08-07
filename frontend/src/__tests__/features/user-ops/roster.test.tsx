// Task 8：通讯录视图（RosterView）组件测试。
// 验证：(1) 全量好友渲染（含 managed / not_imported 混合），managed 行 checkbox 禁用不可勾选；
//       (2) 勾选 2 条 not_imported + 填共享运营备注 + 点「加入 Agent 运营」→ POST /contacts/batch-enable，
//           body 含 accountId / candidates(len 2) / sharedNote（camelCase wire 键）。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, act, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RosterView } from "../../../features/user-ops/RosterView";
import { ToastProvider } from "../../../components/ui/Toast";
import { useAccountStore } from "../../../stores/accountStore";
import { useUserOpsStore } from "../../../stores/userOpsStore";

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
    // rosterCache 是 store 单例、跨用例常驻——每例清空，避免上一例的缓存命中污染本例 mock。
    useUserOpsStore.setState({ rosterCache: {}, playbooks: [] });
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
        source: string;
        candidates: { wxid: string }[];
        sharedNote: string;
      };
      expect(body.accountId).toBe("acc1");
      expect(body.source).toBe("roster");
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

  it("cache 同步中(syncing:true,空列表)显示同步中提示而非「暂无好友」", async () => {
    // 后端 cache 未就绪：items 空 + syncing:true。
    getMock.mockResolvedValue({ items: [], syncing: true });
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    // 显示「同步中」文案，不显示「暂无好友」。
    expect(await screen.findByText(/正在从微信同步好友/)).toBeInTheDocument();
    expect(screen.queryByText("暂无好友")).not.toBeInTheDocument();
  });

  it("syncing:false 且空列表才显示「暂无好友」", async () => {
    getMock.mockResolvedValue({ items: [], syncing: false });
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    expect(await screen.findByText("暂无好友")).toBeInTheDocument();
    expect(screen.queryByText(/正在从微信同步好友/)).not.toBeInTheDocument();
  });

  it("展示性别文字（男/女），并透传 sex 到 batch-enable", async () => {
    // 好友名首字刻意避开「男/女/未知」，否则无头像时 avatarFallback 取首字母会与性别文字撞车。
    getMock.mockResolvedValue({
      items: [
        { wxid: "wx_m", nickname: "张三", remark: null, avatarUrl: null, sex: 1, agentStatus: "not_imported" },
        { wxid: "wx_f", nickname: "李四", remark: null, avatarUrl: null, sex: 2, agentStatus: "not_imported" },
      ],
      syncing: false,
    });
    const user = userEvent.setup();
    render(<ToastProvider><RosterView /></ToastProvider>);
    expect(await screen.findByText("张三")).toBeInTheDocument();
    expect(screen.getByText("男")).toBeInTheDocument();
    expect(screen.getByText("女")).toBeInTheDocument();

    await user.click(screen.getByText("张三").closest("button") as HTMLButtonElement);
    await user.type(screen.getByPlaceholderText(/本批运营备注/), "测试备注");
    await user.click(screen.getByText("加入 Agent 运营").closest("button") as HTMLButtonElement);
    await waitFor(() => {
      const call = postMock.mock.calls.find((c) => String(c[0]).includes("/contacts/batch-enable"));
      const body = call![1] as { candidates: { wxid: string; sex?: number | null }[] };
      expect(body.candidates[0].sex).toBe(1);
    });
  });

  it("超过一页时分页，切页显示下一批", async () => {
    const many = Array.from({ length: 75 }, (_, i) => ({
      wxid: `wx_${i}`, nickname: `好友${i}`, remark: null, avatarUrl: null, sex: 0, agentStatus: "not_imported",
    }));
    getMock.mockResolvedValue({ items: many, syncing: false });
    const user = userEvent.setup();
    render(<ToastProvider><RosterView /></ToastProvider>);
    // 首页 60 条：好友0 在，好友60 不在。
    expect(await screen.findByText("好友0")).toBeInTheDocument();
    expect(screen.queryByText("好友60")).not.toBeInTheDocument();
    // 翻到下一页：好友60 出现。
    await user.click(screen.getByRole("button", { name: /下一页/ }));
    expect(await screen.findByText("好友60")).toBeInTheDocument();
  });

  it("非真人账号默认折叠，展开后只读且不可提交", async () => {
    getMock.mockResolvedValue({
      items: [
        { wxid: "wx_real", nickname: "张三", remark: null, avatarUrl: null, sex: 1, isNonHuman: false, agentStatus: "not_imported" },
        { wxid: "fmessage", nickname: "朋友推荐消息", remark: null, avatarUrl: null, sex: 0, isNonHuman: true, agentStatus: "not_imported" },
      ],
      syncing: false,
    });
    const user = userEvent.setup();
    render(<ToastProvider><RosterView /></ToastProvider>);
    // 真人直接可见。
    expect(await screen.findByText("张三")).toBeInTheDocument();
    // 非真人默认折叠：不直接可见，但有折叠入口(含 1 个)。
    expect(screen.queryByText("朋友推荐消息")).not.toBeInTheDocument();
    expect(screen.getByText(/系统账号/)).toBeInTheDocument();
    // 展开后可见。
    await user.click(screen.getByText(/系统账号/).closest("button") as HTMLButtonElement);
    expect(await screen.findByText("朋友推荐消息")).toBeInTheDocument();
    const systemCard = screen.getByText("朋友推荐消息").closest("button");
    expect(systemCard).toBeDisabled();
    await user.click(systemCard as HTMLButtonElement);
    expect(screen.queryByText(/已选 1 人/)).not.toBeInTheDocument();
  });

  it("批量托管的 Playbook 下拉不展示 AI 草稿", async () => {
    useUserOpsStore.setState({
      playbooks: [
        {
          id: "published-playbook",
          accountId: "acc1",
          name: "已发布方法",
          methodPrompt: "published",
          createdBy: "manual",
          releaseStatus: "published",
          isDefault: false,
          version: 1,
        },
        {
          id: "draft-playbook",
          accountId: "acc1",
          name: "AI 草稿方法",
          methodPrompt: "draft",
          createdBy: "agent",
          releaseStatus: "draft",
          isDefault: false,
          version: 1,
        },
      ],
    });
    const user = userEvent.setup();
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    await user.click((await screen.findByText("新好友一")).closest("button") as HTMLButtonElement);
    expect(screen.getByRole("option", { name: "已发布方法" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "AI 草稿方法" })).not.toBeInTheDocument();
  });

  it("二次 loadRoster 命中缓存不重复请求，force 才重拉", async () => {
    getMock.mockResolvedValue({ items: ROSTER, syncing: false });
    // 首次拉：打 API。
    await useUserOpsStore.getState().loadRoster("accCache");
    const after1 = getMock.mock.calls.length;
    expect(after1).toBeGreaterThan(0);
    // 二次非 force：走缓存，不再打 API。
    const r2 = await useUserOpsStore.getState().loadRoster("accCache");
    expect(getMock.mock.calls.length).toBe(after1);
    expect(r2.items.length).toBe(ROSTER.length);
    // force：强制重拉。
    await useUserOpsStore.getState().loadRoster("accCache", { force: true });
    expect(getMock.mock.calls.length).toBe(after1 + 1);
  });

  it("force 刷新时 URL 带 &force=true，非 force 不带", async () => {
    getMock.mockResolvedValue({ items: ROSTER, syncing: false });
    // 非 force：URL 不含 force。
    await useUserOpsStore.getState().loadRoster("accForce");
    const firstUrl = String(getMock.mock.calls.at(-1)?.[0] ?? "");
    expect(firstUrl).toContain("accountId=accForce");
    expect(firstUrl).not.toContain("force=true");
    // force：URL 含 &force=true。
    await useUserOpsStore.getState().loadRoster("accForce", { force: true });
    const forceUrl = String(getMock.mock.calls.at(-1)?.[0] ?? "");
    expect(forceUrl).toContain("force=true");
  });

  it("syncing 期间每 10s 自动重拉，且不闪现「加载中…」", async () => {
    vi.useFakeTimers();
    try {
      getMock.mockResolvedValue({ items: [], syncing: true });
      render(
        <ToastProvider>
          <RosterView />
        </ToastProvider>
      );
      // 首次加载落地（syncing:true）。vi.waitFor 会在假定时器下推进定时器并冲洗 microtask，
      // 直到同步中提示渲染出来。
      await vi.waitFor(() => {
        expect(screen.getByText(/正在从微信同步好友/)).toBeInTheDocument();
      });
      expect(screen.queryByText("加载中…")).not.toBeInTheDocument();
      const callsAfterFirst = getMock.mock.calls.length;
      // 推进 10s，自动重拉应再次调用 loadRoster。
      await vi.advanceTimersByTimeAsync(10000);
      expect(getMock.mock.calls.length).toBeGreaterThan(callsAfterFirst);
      // 后台重拉期间仍不闪「加载中…」，同步中提示保持。
      expect(screen.queryByText("加载中…")).not.toBeInTheDocument();
      expect(screen.getByText(/正在从微信同步好友/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  // 列表里每张卡显示的是**好友**的 wxid，和自己的账号 ID 混在一起看不出区别，
  // 所以头部要标明「本账号微信 ID」。
  it("头部显示当前账号自己的微信 ID 与昵称", async () => {
    useAccountStore.setState({
      accounts: [
        {
          accountId: "acc1",
          alias: "测试账号",
          displayName: "测试账号",
          wxid: "wxid_self_me",
          nickName: "我的昵称",
          online: true,
        } as never,
      ],
      selectedAccountId: "acc1",
    });
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    expect(await screen.findByText("本账号")).toBeInTheDocument();
    expect(screen.getByText("wxid_self_me")).toBeInTheDocument();
    expect(screen.getByText("我的昵称")).toBeInTheDocument();
  });

  // wxid 可能为空（账号刚建、MCP 未回传身份）。此时不渲染身份段，
  // 而不是显示「本账号（空）」。
  it("账号无 wxid 时不渲染身份段", async () => {
    // seedAccount() 的账号本身不带 wxid。
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    // 等列表落地，确认组件已渲染完毕再断言缺失。
    expect(await screen.findByText("老客户")).toBeInTheDocument();
    expect(screen.queryByText("本账号")).not.toBeInTheDocument();
  });

  // 回归：外层 UserOpsModeHeader 已渲染「通讯录」标题 + 「拉取该账号的全部微信好友，
  // 勾选后批量进入 Agent 运营。」描述。本视图曾重复渲染同名 eyebrow + 同义 subtitle，
  // 同一屏出现两遍标题两遍说明。此处钉住不再重复。
  it("不重复外层已有的标题与说明", async () => {
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    expect(await screen.findByText("微信好友总览")).toBeInTheDocument();
    // eyebrow「通讯录」与外层标题字字相同 → 不再渲染。
    expect(screen.queryByText("通讯录")).not.toBeInTheDocument();
    // subtitle 与外层描述同义 → 不再渲染。
    expect(
      screen.queryByText("选择账号拉取全部好友，勾选后批量进入 Agent 运营。")
    ).not.toBeInTheDocument();
  });

  // 元信息行的总数段：回答「有多少人」，是外层标题给不了的信息。
  it("元信息行显示好友总数", async () => {
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    // ROSTER 有 3 条。
    expect(await screen.findByText(/共 3 位好友/)).toBeInTheDocument();
  });

  // 后端 force 是「触发后台单飞 + 立即返回旧快照」，点刷新时列表内容一模一样。
  // 前端据 fetchedAt 基线轮询，直到快照真的换了才提示「已更新」——否则用户以为按钮坏了。
  it("点刷新后轮询到 fetchedAt 变化 → 提示已更新", async () => {
    vi.useFakeTimers();
    try {
      getMock.mockResolvedValue({ items: ROSTER, fetchedAt: "2026-08-07T01:00:00Z" });
      render(
        <ToastProvider>
          <RosterView />
        </ToastProvider>
      );
      await vi.waitFor(() => expect(screen.getByText("老客户")).toBeInTheDocument());

      // 点刷新进入等待轮询态。用 fireEvent 而非 userEvent：后者内部有自己的
      // 延时链，在假定时器下会挂住。
      fireEvent.click(screen.getByRole("button", { name: /刷新/ }));
      // 此刻后端仍返回旧快照（force 是「触发后台单飞 + 立即返回旧快照」），
      // 所以缓存里的快照龄还是基线值——这正是「点了看起来没反应」的由来。
      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000);
      });
      expect(
        useUserOpsStore.getState().rosterCache.acc1?.serverFetchedAt
      ).toBe("2026-08-07T01:00:00Z");

      // 后台写入新快照 → 轮询下一轮观测到 fetchedAt 变化 → 落缓存、停轮询、提示已更新。
      // 断言落在缓存收敛而非 toast DOM：toast 只活 3s（DURATION.success），
      // 假定时器下推进时间必然把它推到消失，断 DOM 会脆。
      getMock.mockResolvedValue({ items: ROSTER, fetchedAt: "2026-08-07T02:00:00Z" });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000);
      });
      expect(
        useUserOpsStore.getState().rosterCache.acc1?.serverFetchedAt
      ).toBe("2026-08-07T02:00:00Z");
    } finally {
      vi.useRealTimers();
    }
  });

  // 回归：轮询若走 refresh() 会清空勾选/备注草稿（那是切账号语义），
  // 用户点完刷新继续勾人时勾到一半被清空 —— 比原本「刷新没反应」更糟。
  it("刷新等待轮询不清空已勾选的好友", async () => {
    vi.useFakeTimers();
    try {
      getMock.mockResolvedValue({ items: ROSTER, fetchedAt: "2026-08-07T01:00:00Z" });
      render(
        <ToastProvider>
          <RosterView />
        </ToastProvider>
      );
      await vi.waitFor(() => expect(screen.getByText("新好友一")).toBeInTheDocument());

      // 先点刷新进入等待轮询态。点击瞬间 loading=true、列表被「加载中…」替换，
      // 所以要等它落地、卡片回到 DOM 才能勾人（模拟用户边等边操作）。
      fireEvent.click(screen.getByRole("button", { name: /刷新/ }));
      await vi.waitFor(() => expect(screen.getByText("新好友一")).toBeInTheDocument());
      fireEvent.click(screen.getByText("新好友一").closest("button")!);
      expect(screen.getByText(/已选 1 人/)).toBeInTheDocument();

      // 轮询跑两轮，勾选必须还在。
      await vi.advanceTimersByTimeAsync(7000);
      expect(screen.getByText(/已选 1 人/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
