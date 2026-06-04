import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Shell } from "../../app/Shell";
import { useNavigationStore } from "../../stores/navigationStore";

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
