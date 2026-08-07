import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore } from "../../stores/navigationStore";
import { useAuthStore } from "../../stores/authStore";
import { CHANNELS, GROUP_ORDER } from "../../app/channels";

const GROUPS = ["日常", "运营", "知识与内容", "成效", "设置"];

describe("Shell 导航（单列 + 常显分组标签）", () => {
  // 核心不变量，也是放弃手风琴与图标轨的理由：所有频道恒可见。
  // 手风琴同时只展开一组（跨组切换要两步）；图标轨同时只渲染一组（拿宽度换高度，
  // 导致中文标签换行）。单列全部渲染，超出交给滚动。
  it("所有分组的所有频道同时渲染，不需要任何展开动作", async () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    // 跨 4 个不同分组各取一个频道：日常 / 运营 / 知识与内容 / 设置。
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    expect(screen.getByText("用户运营")).toBeInTheDocument();
    expect(screen.getByText("知识库 Wiki")).toBeInTheDocument();
    expect(screen.getByText("系统策略")).toBeInTheDocument();
  });

  it("五个分组标签全部常显，且是静态文字而非按钮", () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    for (const g of GROUPS) {
      const label = screen.getByTestId(`nav-group-${g}`);
      expect(label).toBeInTheDocument();
      // 标签不再承担交互（不切面板、不折叠），用 button 会给出可点的错误暗示。
      expect(label.tagName).not.toBe("BUTTON");
    }
  });

  it("点频道即切换，无需先操作分组", () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    // 「系统策略」属「设置」组，与当前频道「工作台」（日常）不同组。
    // 单列下它本来就可见，一次点击直达——这正是手风琴要两步的那个场景。
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
