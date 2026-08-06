import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore, DEFAULT_COLLAPSED } from "../../stores/navigationStore";
import { useAuthStore } from "../../stores/authStore";

describe("Shell", () => {
  it("默认渲染展开组（日常/运营）的 channel 标签", async () => {
    useNavigationStore.setState({ activeChannel: "overview", collapsedGroups: DEFAULT_COLLAPSED });
    render(<Shell />);
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    expect(screen.getByText("用户运营")).toBeInTheDocument();
    // 「设置」默认收起 → 组内频道不渲染（分级导航的核心行为）。
    expect(screen.queryByText("系统策略")).not.toBeInTheDocument();
  });

  it("点击组标题展开收起的组，其频道随之出现", async () => {
    useNavigationStore.setState({ activeChannel: "overview", collapsedGroups: DEFAULT_COLLAPSED });
    render(<Shell />);
    expect(screen.queryByText("系统策略")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("nav-group-设置"));
    expect(await screen.findByText("系统策略")).toBeInTheDocument();
  });

  it("当前频道所在组即使被折叠也强制展开（不丢定位）", () => {
    // systemStrategy 属「设置」，且「设置」在 collapsedGroups 里 → 仍须可见。
    useNavigationStore.setState({
      activeChannel: "systemStrategy",
      collapsedGroups: ["日常", "运营", "知识与内容", "成效", "设置"],
    });
    render(<Shell />);
    // 「系统策略」同时是侧栏频道名与页头 h1，故把查询限定在侧栏 nav 内。
    const nav = screen.getByRole("navigation", { name: "Product channels" });
    expect(within(nav).getByText("系统策略")).toBeInTheDocument();
  });

  it("未上线占位频道渲染成不可点的灰显项", () => {
    useNavigationStore.setState({ activeChannel: "overview", collapsedGroups: [] });
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
