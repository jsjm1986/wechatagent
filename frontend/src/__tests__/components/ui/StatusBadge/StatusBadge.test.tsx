import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusBadge } from "../../../../components/ui/StatusBadge/StatusBadge";

describe("StatusBadge", () => {
  it("渲染文案与对应语义 tone 的 class", () => {
    const { container } = render(<StatusBadge tone="running">自主回复</StatusBadge>);
    expect(screen.getByText("自主回复")).toBeInTheDocument();
    // CSS Module 编译后类名带哈希，用 [class*=] 匹配语义片段
    expect(container.querySelector('[class*="running"]')).not.toBeNull();
  });

  it("held tone 不应带 running 呼吸类", () => {
    const { container } = render(<StatusBadge tone="held">暂缓</StatusBadge>);
    expect(container.querySelector('[class*="running"]')).toBeNull();
  });
});