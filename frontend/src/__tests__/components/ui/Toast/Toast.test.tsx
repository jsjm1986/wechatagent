import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ToastProvider, useToast } from "../../../../components/ui/Toast";

function Harness() {
  const toast = useToast();
  return (
    <>
      <button onClick={() => toast.success("已保存")}>成功</button>
      <button onClick={() => toast.error("失败了")}>失败</button>
    </>
  );
}

describe("Toast", () => {
  it("success 推送出现在 role=status 区域", async () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>
    );
    fireEvent.click(screen.getByText("成功"));
    expect(await screen.findByText("已保存")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("已保存");
  });

  it("点关闭按钮移除该条", async () => {
    render(
      <ToastProvider>
        <Harness />
      </ToastProvider>
    );
    fireEvent.click(screen.getByText("失败"));
    await screen.findByText("失败了");
    fireEvent.click(screen.getByLabelText("关闭"));
    await waitFor(() => expect(screen.queryByText("失败了")).not.toBeInTheDocument());
  });
});
