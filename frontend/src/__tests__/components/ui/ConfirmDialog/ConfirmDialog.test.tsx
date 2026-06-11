import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ConfirmProvider, useConfirm } from "../../../../components/ui/ConfirmDialog";

function Harness({ onResult, ...opts }: { onResult: (ok: boolean) => void } & Record<string, unknown>) {
  const confirm = useConfirm();
  return (
    <button
      onClick={async () => {
        const ok = await confirm({ title: "确定删除？", ...opts });
        onResult(ok);
      }}
    >
      触发
    </button>
  );
}

function setup(opts: Record<string, unknown> = {}) {
  const results: boolean[] = [];
  render(
    <ConfirmProvider>
      <Harness onResult={(ok) => results.push(ok)} {...opts} />
    </ConfirmProvider>
  );
  fireEvent.click(screen.getByText("触发"));
  return results;
}

describe("ConfirmDialog", () => {
  it("点确认 resolve(true)", async () => {
    const results = setup();
    expect(await screen.findByText("确定删除？")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    await waitFor(() => expect(results).toEqual([true]));
  });

  it("点取消 resolve(false)", async () => {
    const results = setup();
    await screen.findByText("确定删除？");
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(results).toEqual([false]));
  });

  it("Esc resolve(false)", async () => {
    const results = setup();
    await screen.findByText("确定删除？");
    act(() => {
      fireEvent.keyDown(document, { key: "Escape" });
    });
    await waitFor(() => expect(results).toEqual([false]));
  });

  it("requireText 未匹配时确认按钮禁用，匹配后可点", async () => {
    const results = setup({ tone: "danger", requireText: "全量", confirmText: "灰度全量" });
    await screen.findByText("确定删除？");
    const confirmBtn = screen.getByRole("button", { name: "灰度全量" });
    expect(confirmBtn).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText("全量"), { target: { value: "全量" } });
    expect(confirmBtn).toBeEnabled();
    fireEvent.click(confirmBtn);
    await waitFor(() => expect(results).toEqual([true]));
  });
});
