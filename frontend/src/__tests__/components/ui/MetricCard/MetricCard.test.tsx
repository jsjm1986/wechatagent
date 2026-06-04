import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MetricCard } from "../../../../components/ui/MetricCard";

describe("MetricCard", () => {
  it("渲染 label/value/detail 并响应点击", () => {
    const onClick = vi.fn();
    render(<MetricCard label="Managed Users" value={128} detail="Agent 运营好友" onClick={onClick} />);
    expect(screen.getByText("Managed Users")).toBeInTheDocument();
    expect(screen.getByText("128")).toBeInTheDocument();
    expect(screen.getByText("Agent 运营好友")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });
});
