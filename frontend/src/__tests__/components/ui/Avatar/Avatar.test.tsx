import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Avatar } from "../../../../components/ui/Avatar";

describe("Avatar", () => {
  it("渲染姓名首字与 tone class", () => {
    const { container } = render(<Avatar name="陈先生" tone="running" />);
    expect(screen.getByText("陈")).toBeInTheDocument();
    expect(container.querySelector('[class*="running"]')).not.toBeNull();
  });

  it("默认 tone 为 inactive", () => {
    const { container } = render(<Avatar name="王总" />);
    expect(container.querySelector('[class*="inactive"]')).not.toBeNull();
  });
});
