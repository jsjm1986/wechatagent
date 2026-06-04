import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EmptyState } from "../../../../components/ui/EmptyState";

describe("EmptyState", () => {
  it("渲染 title，hint/action 可选", () => {
    render(<EmptyState title="暂无运营事件" hint="稍后再来看看" />);
    expect(screen.getByText("暂无运营事件")).toBeInTheDocument();
    expect(screen.getByText("稍后再来看看")).toBeInTheDocument();
  });

  it("无 hint 时不渲染提示文案", () => {
    render(<EmptyState title="空空如也" />);
    expect(screen.getByText("空空如也")).toBeInTheDocument();
  });
});
