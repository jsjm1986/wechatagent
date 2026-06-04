import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusLine } from "../../../../components/ui/StatusLine";

describe("StatusLine", () => {
  it("渲染 label/value 与 tone class", () => {
    const { container } = render(<StatusLine label="运营好友" tone="ai" value="12 managed" />);
    expect(screen.getByText("运营好友")).toBeInTheDocument();
    expect(screen.getByText("12 managed")).toBeInTheDocument();
    expect(container.querySelector('[class*="ai"]')).not.toBeNull();
  });
});
