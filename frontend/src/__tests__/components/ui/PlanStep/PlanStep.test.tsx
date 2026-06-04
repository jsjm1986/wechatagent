import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PlanStep } from "../../../../components/ui/PlanStep";

describe("PlanStep", () => {
  it("渲染 title/detail 与 status class", () => {
    const { container } = render(<PlanStep title="加载工具目录" detail="从 MCP 获取" status="ready" />);
    expect(screen.getByText("加载工具目录")).toBeInTheDocument();
    expect(screen.getByText("从 MCP 获取")).toBeInTheDocument();
    expect(container.querySelector('[class*="ready"]')).not.toBeNull();
  });
});
