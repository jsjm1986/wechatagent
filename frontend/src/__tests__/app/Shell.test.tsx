import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore, DEFAULT_EXPANDED } from "../../stores/navigationStore";
import { useAuthStore } from "../../stores/authStore";

describe("Shell", () => {
  it("默认只展开「日常」组，其余组的频道不渲染", async () => {
    useNavigationStore.setState({ activeChannel: "overview", expandedGroup: DEFAULT_EXPANDED });
    render(<Shell />);
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    // 手风琴：同时只展开一组，所以「运营」「设置」的频道都不渲染。
    expect(screen.queryByText("用户运营")).not.toBeInTheDocument();
    expect(screen.queryByText("系统策略")).not.toBeInTheDocument();
  });

  it("点击组标题展开该组，同时自动收起原先展开的组", async () => {
    useNavigationStore.setState({ activeChannel: "overview", expandedGroup: "日常" });
    render(<Shell />);
    expect(screen.getByText("工作台")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("nav-group-设置"));
    expect(await screen.findByText("系统策略")).toBeInTheDocument();
    // 这是手风琴的关键保证：展开新组必须收起旧组，否则行数无上限、滚动条回来。
    expect(screen.queryByText("工作台")).not.toBeInTheDocument();
  });

  it("再次点击已展开的组标题会收起它", () => {
    useNavigationStore.setState({ activeChannel: "overview", expandedGroup: "日常" });
    render(<Shell />);
    expect(screen.getByText("工作台")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("nav-group-日常"));
    expect(screen.queryByText("工作台")).not.toBeInTheDocument();
  });

  it("当前频道所在组被收起时，标题上打活跃圆点表达定位", () => {
    // 手风琴下不再「强制展开」活跃组（那会让同时展开变 2 组、滚动条回来），
    // 定位感由圆点承担。systemStrategy 属「设置」，而展开的是「日常」。
    useNavigationStore.setState({
      activeChannel: "systemStrategy",
      expandedGroup: "日常",
    });
    render(<Shell />);
    const nav = screen.getByRole("navigation", { name: "Product channels" });
    // 组内频道确实没渲染（未被强制展开）
    expect(within(nav).queryByText("系统策略")).not.toBeInTheDocument();
    // 但「设置」标题上有活跃圆点，且只有它有
    expect(within(nav).getByLabelText("当前频道在此组")).toBeInTheDocument();
    const settings = screen.getByTestId("nav-group-设置");
    expect(within(settings).getByLabelText("当前频道在此组")).toBeInTheDocument();
  });

  it("未上线占位频道渲染成不可点的灰显项", () => {
    // 「微信群运营」属「运营」组，故须展开该组。
    useNavigationStore.setState({ activeChannel: "overview", expandedGroup: "运营" });
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

  // 不变量测试：这是「侧栏不出滚动条」的根本依据。逐组点一遍，
  // 每次点完都断言 aria-expanded=true 的组不超过 1 个。若哪天有人
  // 加回「强制展开活跃组」之类的逻辑，这条会先红，而不是等用户看到滚动条。
  it("任何时刻展开的组不超过一个（手风琴不变量）", () => {
    useNavigationStore.setState({ activeChannel: "overview", expandedGroup: "日常" });
    render(<Shell />);
    const nav = screen.getByRole("navigation", { name: "Product channels" });
    const groups = ["日常", "运营", "知识与内容", "成效", "设置"];
    const openCount = () =>
      groups.filter(
        (g) => screen.getByTestId(`nav-group-${g}`).getAttribute("aria-expanded") === "true"
      ).length;

    expect(openCount()).toBeLessThanOrEqual(1);
    for (const g of groups) {
      fireEvent.click(screen.getByTestId(`nav-group-${g}`));
      expect(openCount()).toBeLessThanOrEqual(1);
    }
    // 同时：渲染出的频道行数不超过最大单组的容量（5），否则高度账不成立。
    const rows = within(nav).getAllByRole("button").length - groups.length;
    expect(rows).toBeLessThanOrEqual(5);
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
