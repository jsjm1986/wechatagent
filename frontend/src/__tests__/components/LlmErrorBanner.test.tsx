import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LlmErrorBanner } from "../../components/LlmErrorBanner";
import { LlmUnavailableError } from "../../lib/api";

describe("LlmErrorBanner 故障归属", () => {
  it("普通 Error 归为「客户端错误」，不冒充上游模型故障", () => {
    // 这正是线上那条红条的成因：一个前端 TypeError
    // （crypto.randomUUID is not a function）此前被标成「未知错误」，
    // 看起来像 AI 挂了，实际请求压根没发出去。
    render(<LlmErrorBanner error={new TypeError("crypto.randomUUID is not a function")} />);

    expect(screen.getByText("客户端错误")).toBeInTheDocument();
    expect(screen.queryByText("未知错误")).not.toBeInTheDocument();
    expect(
      screen.getByText("crypto.randomUUID is not a function"),
    ).toBeInTheDocument();
  });

  it("客户端故障时按钮不写「AI 重试」，用调用方给的动作名", () => {
    render(
      <LlmErrorBanner
        error={new Error("本地炸了")}
        onRetry={() => {}}
        retryActionLabel="重新加载"
      />,
    );

    expect(screen.getByRole("button", { name: "重新加载" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "AI 重试" })).not.toBeInTheDocument();
  });

  it("客户端故障未给动作名时回落成中性的「重试」", () => {
    render(<LlmErrorBanner error={new Error("本地炸了")} onRetry={() => {}} />);

    expect(screen.getByRole("button", { name: "重试" })).toBeInTheDocument();
  });

  it("客户端故障 + retrying 时显示「<动作>中…」", () => {
    render(
      <LlmErrorBanner
        error={new Error("本地炸了")}
        onRetry={() => {}}
        retrying
        retryActionLabel="重新加载"
      />,
    );

    expect(screen.getByRole("button", { name: "重新加载中…" })).toBeDisabled();
  });

  it("真正的上游故障仍显示上游分类与「AI 重试」", () => {
    render(
      <LlmErrorBanner
        error={
          new LlmUnavailableError({
            kind: "timeout",
            retryCount: 2,
            detail: "upstream deadline exceeded",
            hint: "调用 LLM 超时",
          })
        }
        onRetry={() => {}}
        retryActionLabel="重新加载"
      />,
    );

    expect(screen.getByText("上游超时")).toBeInTheDocument();
    expect(screen.getByText("已自动重试 2 次")).toBeInTheDocument();
    // 上游故障不受 retryActionLabel 影响——重试确实会再打一次模型。
    expect(screen.getByRole("button", { name: "AI 重试" })).toBeInTheDocument();
  });

  it("结构化 payload（非 Error 实例）缺 kind 时仍回落 unknown", () => {
    render(<LlmErrorBanner error={{ detail: "raw payload" }} />);

    expect(screen.getByText("未知错误")).toBeInTheDocument();
  });

  it("onRetry 缺省时不渲染任何重试按钮", () => {
    render(<LlmErrorBanner error={new Error("no retry")} />);

    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("点击重试会回调", async () => {
    const onRetry = vi.fn();
    const { default: userEvent } = await import("@testing-library/user-event");
    render(<LlmErrorBanner error={new Error("x")} onRetry={onRetry} retryActionLabel="重新加载" />);

    await userEvent.setup().click(screen.getByRole("button", { name: "重新加载" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});
