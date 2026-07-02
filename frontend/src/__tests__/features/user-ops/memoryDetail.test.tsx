import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { MemoryDetailView } from "../../../features/user-ops/cockpit/drilldowns/MemoryDetailView";

// CSS Module 走 Proxy（与既有 observeView.test 同款），className 回传键名。
vi.mock("../../../features/user-ops/cockpit/cockpit.module.css", () => ({
  default: new Proxy({}, { get: (_t, k) => String(k) }),
}));

describe("MemoryDetailView", () => {
  it("记忆事实展示溯源徽标（置信/重要），而非只显纯文本", () => {
    render(
      <MemoryDetailView
        memoryCard={
          {
            coreFacts: [
              { text: "客户偏好微信沟通", confidence: 8, importance: 9, deprecatedAt: null },
            ],
          } as any
        }
        onBack={() => {}}
      />,
    );
    // 事实正文
    expect(screen.getByText(/客户偏好微信沟通/)).toBeInTheDocument();
    // 溯源字段被渲染成徽标（confidence / importance），不是被吞进纯文本
    expect(screen.getByText(/置信 8/)).toBeInTheDocument();
    expect(screen.getByText(/重要 9/)).toBeInTheDocument();
  });

  it("已弃用事实把 deprecatedAt / deprecationReason 显性呈现", () => {
    render(
      <MemoryDetailView
        memoryCard={
          {
            deprecatedFacts: [
              { text: "旧联系方式 138xxxx", deprecatedAt: "2026-06-01", deprecationReason: "客户已更换号码" },
            ],
          } as any
        }
        onBack={() => {}}
      />,
    );
    expect(screen.getByText(/旧联系方式/)).toBeInTheDocument();
    expect(screen.getByText(/已弃用/)).toBeInTheDocument();
    expect(screen.getByText(/客户已更换号码/)).toBeInTheDocument();
  });

  it("兼容 coreFacts 的字符串旧形态（Vec<String> 向后兼容）", () => {
    render(
      <MemoryDetailView
        memoryCard={{ coreFacts: ["客户是老板，决策权在自己"] } as any}
        onBack={() => {}}
      />,
    );
    expect(screen.getByText(/客户是老板/)).toBeInTheDocument();
  });

  it("onBack 返回按钮可点击", () => {
    const onBack = vi.fn();
    render(<MemoryDetailView memoryCard={{}} onBack={onBack} />);
    fireEvent.click(screen.getByText("返回"));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
