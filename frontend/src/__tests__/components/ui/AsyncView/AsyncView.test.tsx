import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useAsync } from "../../../../hooks/useAsync";
import { AsyncView } from "../../../../components/ui/AsyncView";

function Probe<T>({ fn, isEmpty }: { fn: () => Promise<T>; isEmpty?: (d: T) => boolean }) {
  const state = useAsync(fn);
  return (
    <AsyncView
      state={state}
      onRetry={state.reload}
      isEmpty={isEmpty}
      emptyTitle="空空如也"
      loading={<div>加载骨架</div>}
    >
      {(data) => <div>数据：{String(data)}</div>}
    </AsyncView>
  );
}

describe("useAsync + AsyncView", () => {
  it("loading → success 渲染数据", async () => {
    render(<Probe fn={() => Promise.resolve("ok")} />);
    expect(screen.getByText("加载骨架")).toBeInTheDocument();
    expect(await screen.findByText("数据：ok")).toBeInTheDocument();
  });

  it("success 但 isEmpty → 渲染空态", async () => {
    render(<Probe fn={() => Promise.resolve([] as string[])} isEmpty={(d) => d.length === 0} />);
    expect(await screen.findByText("空空如也")).toBeInTheDocument();
  });

  it("error → 渲染错误卡 + 重试可重新拉取", async () => {
    const fn = vi
      .fn()
      .mockRejectedValueOnce(new Error("炸了"))
      .mockResolvedValueOnce("恢复");
    render(<Probe fn={fn} />);
    expect(await screen.findByText("炸了")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(screen.getByText("数据：恢复")).toBeInTheDocument());
  });
});
