import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ContactsView } from "../../../features/user-ops/legacy";

describe("ContactsView 运营池", () => {
  // baseProps 对齐当前 ContactsView 实际 props 类型（legacy.tsx:440-464，无 busy）。
  const baseProps = {
    contactTab: "all" as const,
    contacts: [],
    managedCount: 2,
    normalCount: 61,
    query: "",
    selected: null,
    totalCount: 63,
    onContactTab: vi.fn(),
    onLoadAll: vi.fn(),
    onOpenContact: vi.fn(),
    onQuery: vi.fn()
  };

  it("三 tab 文案为 已互动 / Agent / 待启用，计数来自 props", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.getByText("已互动 63")).toBeInTheDocument();
    expect(screen.getByText("Agent 2")).toBeInTheDocument();
    expect(screen.getByText("待启用 61")).toBeInTheDocument();
    // 旧文案不得残留。
    expect(screen.queryByText(/^全部 /)).toBeNull();
    expect(screen.queryByText(/^普通 /)).toBeNull();
  });

  it("导入框已移除，只保留过滤框", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.queryByPlaceholderText("搜索并导入好友，例如 AI应用开发")).toBeNull();
    expect(screen.getByPlaceholderText("过滤已互动")).toBeInTheDocument();
  });
});
