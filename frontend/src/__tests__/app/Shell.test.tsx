import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore, DEFAULT_GROUP } from "../../stores/navigationStore";
import { useAuthStore } from "../../stores/authStore";

describe("Shell 导航（图标轨 + 二级面板）", () => {
  it("默认选中「日常」组，面板只渲染该组频道", async () => {
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: DEFAULT_GROUP });
    render(<Shell />);
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    // 一次只渲染一组——这是「侧栏永不滚动」的结构依据（最坏 5 行 ≈ 220px）。
    expect(screen.queryByText("用户运营")).not.toBeInTheDocument();
    expect(screen.queryByText("系统策略")).not.toBeInTheDocument();
  });

  it("点轨上的分组图标即换面板，一步完成（不需要先折叠）", async () => {
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: "日常" });
    render(<Shell />);
    expect(screen.getByText("工作台")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("nav-group-设置"));
    expect(await screen.findByText("系统策略")).toBeInTheDocument();
    // 换组即换面板内容，旧组频道不再渲染。
    expect(screen.queryByText("工作台")).not.toBeInTheDocument();
  });

  it("点已选中的分组是幂等的——不会像手风琴那样把面板收起变空", () => {
    // 这是相对手风琴的关键行为差异：轨上永远有一组被选中，面板永不空白。
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: "日常" });
    render(<Shell />);
    expect(screen.getByText("工作台")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("nav-group-日常"));
    expect(screen.getByText("工作台")).toBeInTheDocument();
  });

  it("轨上选中态用 aria-selected 表达，且同时只有一个", () => {
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: "成效" });
    render(<Shell />);
    const groups = ["日常", "运营", "知识与内容", "成效", "设置"];
    const on = groups.filter(
      (g) => screen.getByTestId(`nav-group-${g}`).getAttribute("aria-selected") === "true"
    );
    expect(on).toEqual(["成效"]);
  });

  it("活跃频道在别的组时，轨上该组图标打点表达定位", () => {
    // systemStrategy 属「设置」，而轨上看的是「日常」——定位感靠圆点承担。
    useNavigationStore.setState({
      activeChannel: "systemStrategy",
      activeGroup: "日常",
    });
    render(<Shell />);
    const settings = screen.getByTestId("nav-group-设置");
    expect(within(settings).getByLabelText("当前频道在此组")).toBeInTheDocument();
    // 且只有「设置」有点（选中态自身已高亮，不叠点）。
    expect(screen.getAllByLabelText("当前频道在此组")).toHaveLength(1);
  });

  it("轨上选中的就是活跃频道所在组时，不叠加圆点", () => {
    useNavigationStore.setState({ activeChannel: "systemStrategy", activeGroup: "设置" });
    render(<Shell />);
    expect(screen.queryByLabelText("当前频道在此组")).not.toBeInTheDocument();
  });

  it("未上线占位频道渲染成不可点的灰显项", () => {
    // 「微信群运营」属「运营」组，故须选中该组。
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: "运营" });
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

  // 高度不变量：这是「侧栏不出滚动条」的根本依据，也是本次放弃手风琴的理由。
  // 逐组点一遍，每次都断言 (a) 轨上恰好一个组被选中，(b) 面板里渲染的频道行
  // 不超过最大单组容量。若哪天有人让面板同时渲染两组（或加回「活跃组也展开」），
  // 这条会先红，而不是等用户在矮屏上看到滚动条。
  it("任何时刻只渲染一个组的频道（高度不变量）", () => {
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: "日常" });
    render(<Shell />);
    const panel = screen.getByRole("navigation", { name: "Product channels" });
    const groups = ["日常", "运营", "知识与内容", "成效", "设置"];
    const selectedCount = () =>
      groups.filter(
        (g) => screen.getByTestId(`nav-group-${g}`).getAttribute("aria-selected") === "true"
      ).length;

    // 面板恒显示某一组 → 恒有且仅有一个选中，不存在「全部收起」态。
    expect(selectedCount()).toBe(1);
    for (const g of groups) {
      fireEvent.click(screen.getByTestId(`nav-group-${g}`));
      expect(selectedCount()).toBe(1);
      // 轨按钮在 tablist 里、频道行在 nav 里，所以这里数到的只有频道行。
      // 最大单组是 5（运营/成效），超过就说明渲染了不止一组。
      expect(within(panel).getAllByRole("button").length).toBeLessThanOrEqual(5);
    }
  });

  // 无障碍：轨是 tab 列表、面板是它控制的内容。角色标错的话读屏用户
  // 会把 5 个分组图标读成 5 个普通按钮，完全丢失「这是一组互斥选项」的信息。
  it("图标轨用 tablist/tab 语义，激活频道标 aria-current", () => {
    useNavigationStore.setState({ activeChannel: "overview", activeGroup: "日常" });
    render(<Shell />);
    const rail = screen.getByRole("tablist", { name: "频道分组" });
    expect(within(rail).getAllByRole("tab")).toHaveLength(5);
    // 激活频道除了视觉高亮，还要有程序可读的当前项标记。
    expect(screen.getByText("工作台").closest("button")).toHaveAttribute(
      "aria-current",
      "page"
    );
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
