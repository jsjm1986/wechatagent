import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore, DEFAULT_COLLAPSED } from "../../stores/navigationStore";
import { useAuthStore } from "../../stores/authStore";
import { CHANNELS, GROUP_ORDER } from "../../app/channels";

/** 新分类（按「你在管什么对象」单一轴切分）。旧的五组混了三个轴：
 *  「日常/运营/知识与内容」是任务轴、「成效」是页面类型轴、「设置」是变更频率轴，
 *  三轴混在一列里，找东西时脑子要切标准。典型症状：请示被劈成两半（收件箱在最上、
 *  通道配置在最下）、自治回路/演化被归进「成效」（它们是系统自我调节，不是业务成效）。 */
const GROUPS = [
  "运营对象",
  "AI 的资料",
  "需要你决策",
  "运行与结果",
  "系统",
  "即将上线",
];

/** 每个用例都从「全部展开」起步，除非它自己要测折叠——store 是模块级单例，
 *  不重置会串上一个用例的折叠态。 */
function seedExpanded() {
  useNavigationStore.setState({
    activeChannel: "overview",
    collapsedGroups: new Set(),
  });
}

describe("Shell 导航（单列 + 独立折叠）", () => {
  it("展开态下，所有分组的频道同时渲染", async () => {
    seedExpanded();
    render(<Shell />);
    // 跨 4 个不同分组各取一个：运行与结果 / 运营对象 / AI 的资料 / 系统。
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    expect(screen.getByText("用户运营")).toBeInTheDocument();
    expect(screen.getByText("知识库 Wiki")).toBeInTheDocument();
    expect(screen.getByText("系统策略")).toBeInTheDocument();
  });

  it("六个分组标签全部常显，且是可折叠的按钮", () => {
    seedExpanded();
    render(<Shell />);
    for (const g of GROUPS) {
      const label = screen.getByTestId(`nav-group-${g}`);
      // 标签承担折叠交互，必须是 button：键盘可达 + 读屏报「按钮，已展开」。
      expect(label.tagName).toBe("BUTTON");
      expect(label).toHaveAttribute("aria-expanded", "true");
    }
  });

  // 这是本轮相对手风琴的**关键差异**。手风琴强制互斥（开一组必关另一组），
  // 导致跨组切频道要两步、内容跳动。互斥的唯一理由是「侧栏不能滚」，而滚动
  // 现在被允许了，所以约束没必要。这条守着：折叠一组不影响其它组。
  it("折叠是各组独立的，不互斥（收起一组不影响其它组）", () => {
    seedExpanded();
    render(<Shell />);
    // 收起「运行与结果」（含工作台）
    fireEvent.click(screen.getByTestId("nav-group-运行与结果"));
    expect(screen.queryByText("工作台")).not.toBeInTheDocument();
    // 其它组**仍然展开**——手风琴在这里会把它们全关掉。
    expect(screen.getByText("用户运营")).toBeInTheDocument();
    expect(screen.getByText("系统策略")).toBeInTheDocument();
    // 再收起「运营对象」，「系统」依旧不受影响。
    fireEvent.click(screen.getByTestId("nav-group-运营对象"));
    expect(screen.queryByText("用户运营")).not.toBeInTheDocument();
    expect(screen.getByText("系统策略")).toBeInTheDocument();
  });

  it("同一标签再点一次即展开回来（幂等开合）", () => {
    seedExpanded();
    render(<Shell />);
    const label = screen.getByTestId("nav-group-运行与结果");
    fireEvent.click(label);
    expect(screen.queryByText("工作台")).not.toBeInTheDocument();
    expect(label).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(label);
    expect(screen.getByText("工作台")).toBeInTheDocument();
    expect(label).toHaveAttribute("aria-expanded", "true");
  });

  // 折叠后若活跃频道被藏起来，必须有替代信号，否则用户丢失「我在哪」。
  it("收起的组含活跃频道时，标签上打点表达定位", () => {
    useNavigationStore.setState({
      activeChannel: "systemStrategy", // 属「系统」
      collapsedGroups: new Set(["系统"] as const),
    });
    render(<Shell />);
    const settings = screen.getByTestId("nav-group-系统");
    expect(within(settings).getByLabelText("当前频道在此组")).toBeInTheDocument();
    // 只有「系统」有点：展开的组里活跃频道自己已高亮，不需要重复表达。
    expect(screen.getAllByLabelText("当前频道在此组")).toHaveLength(1);
  });

  it("展开的组不打点（活跃频道自身已高亮，不重复表达）", () => {
    useNavigationStore.setState({
      activeChannel: "systemStrategy",
      collapsedGroups: new Set(),
    });
    render(<Shell />);
    expect(screen.queryByLabelText("当前频道在此组")).not.toBeInTheDocument();
  });

  it("收起时显示组内频道数，让用户知道里面有东西没丢", () => {
    useNavigationStore.setState({
      activeChannel: "overview",
      collapsedGroups: new Set(["运营对象"] as const),
    });
    render(<Shell />);
    const ops = screen.getByTestId("nav-group-运营对象");
    // 「运营对象」组有 3 个频道（用户运营 / 活动 / 产品与成交）。
    expect(ops).toHaveTextContent("3");
  });

  // 默认收起 3 组而非 2 组：分组从 5 个变 6 个后，标签+组间距的固定开销从 179px
  // 涨到 218px，可用高只剩约 10 行的余量。实测收起「AI 的资料 / 系统 / 即将上线」
  // 后展开 10 行 = 578px，在 900 高视口下不滚动（余 33px）。
  it("默认收起「AI 的资料」「系统」「即将上线」——日常动线三组保持展开", () => {
    useNavigationStore.setState({
      activeChannel: "overview",
      collapsedGroups: new Set(DEFAULT_COLLAPSED),
    });
    render(<Shell />);
    // 日常动线可见：运营对象 / 需要你决策 / 运行与结果
    expect(screen.getByText("用户运营")).toBeInTheDocument();
    expect(screen.getByText("统一收件箱")).toBeInTheDocument();
    expect(screen.getByText("工作台")).toBeInTheDocument();
    // 默认收起的三组不渲染
    expect(screen.queryByText("知识库 Wiki")).not.toBeInTheDocument();
    expect(screen.queryByText("系统策略")).not.toBeInTheDocument();
    expect(screen.queryByText("微信群运营")).not.toBeInTheDocument();
  });

  it("点频道即切换，展开态下跨组一步直达", () => {
    seedExpanded();
    render(<Shell />);
    // 「系统策略」属「设置」组，与当前频道「工作台」（日常）不同组。
    // 展开态下它本来就可见，一次点击直达——这正是手风琴要两步的那个场景。
    fireEvent.click(screen.getByText("系统策略").closest("button")!);
    expect(useNavigationStore.getState().activeChannel).toBe("systemStrategy");
  });

  it("激活频道标 aria-current，且全导航只有一个", () => {
    useNavigationStore.setState({ activeChannel: "userOps" });
    render(<Shell />);
    const nav = screen.getByRole("navigation", { name: "Product channels" });
    const current = within(nav)
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveTextContent("用户运营");
  });

  it("未上线占位频道渲染成不可点的灰显项", () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    // 「微信群运营」标 comingSoon → 不是 button，且带「未上线」角标。
    expect(screen.queryByRole("button", { name: /微信群运营/ })).not.toBeInTheDocument();
    expect(screen.getByText("微信群运营")).toBeInTheDocument();
    expect(screen.getAllByText("未上线").length).toBeGreaterThan(0);
  });

  it("渲染当前 channel 的页头标题", () => {
    useNavigationStore.setState({ activeChannel: "userOps" });
    render(<Shell />);
    expect(screen.getByRole("heading", { name: "用户运营" })).toBeInTheDocument();
  });

  // 回归守卫：频道行不带任何角标时，文字列必须有足够宽度。这条守的是
  // 「不要再拿宽度换高度」——图标轨那轮把文字列压到 48px（带角标的行），
  // 「微信群运营」需 65px 于是换行。这里断言结构上不再有第二列侧栏。
  it("导航是单列结构，不含图标轨/二级面板", () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    // 图标轨曾用 role=tablist；单列下不该再有。
    expect(screen.queryByRole("tablist", { name: "频道分组" })).not.toBeInTheDocument();
    // 分组标签也不该再是 tab。
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
  });

  // 分组完整性：每个频道都必须落在 GROUP_ORDER 的某一组里，否则它在单列下
  // 根本不会被渲染（map 只遍历 GROUP_ORDER）——静默消失比报错更难查。
  it("每个频道的 group 都在 GROUP_ORDER 中（否则该频道不会被渲染）", () => {
    const known = new Set<string>(GROUP_ORDER);
    const orphans = CHANNELS.filter((c) => !known.has(c.group)).map((c) => c.label);
    expect(orphans).toEqual([]);
  });
});

describe("Shell workspace 切换器", () => {
  afterEach(() => {
    useAuthStore.setState({ user: null, onSwitchWorkspace: null });
  });

  it("多 workspace 时选择某项调用 onSwitchWorkspace", () => {
    const spy = vi.fn();
    useNavigationStore.setState({ activeChannel: "overview" });
    useAuthStore.setState({
      user: {
        username: "admin",
        userId: "u1",
        workspaces: ["ws1", "ws2"],
        currentWorkspace: "ws1",
      },
      onSwitchWorkspace: spy,
    });

    render(<Shell />);

    // 打开切换器
    fireEvent.click(screen.getByTestId("workspace-switcher-trigger"));
    // 点选 ws2
    fireEvent.click(screen.getByTestId("workspace-option-ws2"));

    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith("ws2");
  });

  it("单 workspace 时不渲染切换器触发器", () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    useAuthStore.setState({
      user: {
        username: "admin",
        userId: "u1",
        workspaces: ["ws1"],
        currentWorkspace: "ws1",
      },
      onSwitchWorkspace: vi.fn(),
    });

    render(<Shell />);

    expect(screen.queryByTestId("workspace-switcher-trigger")).not.toBeInTheDocument();
  });
});
