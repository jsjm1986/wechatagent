import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore } from "../../stores/navigationStore";
import { useAuthStore } from "../../stores/authStore";

describe("Shell", () => {
  it("默认渲染侧栏所有 channel 标签", async () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    expect(screen.getByText("用户运营")).toBeInTheDocument();
    expect(screen.getByText("系统策略")).toBeInTheDocument();
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
