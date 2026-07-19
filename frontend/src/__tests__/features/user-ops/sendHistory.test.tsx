// SendHistorySection：拉取失败必须与"确实没发过"区分开——
// 失败时不得渲染"AI 还没有主动给该客户发送过素材或名片"这句事实性断言
// （否则网络故障被当成真实业务结论展示给运营，误导决策）。
//
// 用 globalThis.fetch mock（而非 mock api 模块返回 rejected promise）：
// 让 api.get 内部自行产生并由组件 .catch 消费 rejection，测试作用域里不留
// 裸 Promise.reject，规避 vitest unhandled-rejection 误判（见 useGoLive.test.ts:58 同款）。
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { SendHistorySection } from "../../../features/user-ops/legacy";
import { useAccountStore } from "../../../stores/accountStore";

const EMPTY_CLAIM = /还没有主动给该客户发送过素材或名片/;

function okJson(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(JSON.stringify(body)),
    headers: new Headers({ "content-type": "application/json" }),
  } as unknown as Response;
}

function errResponse(status: number): Response {
  return {
    ok: false,
    status,
    json: () => Promise.resolve({ error: "boom" }),
    text: () => Promise.resolve(JSON.stringify({ error: "boom" })),
    headers: new Headers({ "content-type": "application/json" }),
  } as unknown as Response;
}

describe("SendHistorySection 加载失败不吞成空态", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useAccountStore.setState({
      accounts: [{ accountId: "account-a" } as never],
      selectedAccountId: "account-a",
    });
  });

  it("拉取成功但确实无记录 → 展示空态事实句", async () => {
    globalThis.fetch = vi.fn(() => Promise.resolve(okJson({ items: [] }))) as unknown as typeof fetch;
    render(<SendHistorySection wxid="wx_ok_empty" />);
    expect(await screen.findByText(EMPTY_CLAIM)).toBeInTheDocument();
    expect(globalThis.fetch).toHaveBeenCalledWith(
      expect.stringContaining("accountId=account-a")
    );
  });

  it("拉取失败 → 不得展示'还没发过'事实句，且要给出失败提示", async () => {
    globalThis.fetch = vi.fn(() => Promise.resolve(errResponse(500))) as unknown as typeof fetch;
    render(<SendHistorySection wxid="wx_fail" />);
    expect(await screen.findByText(/加载失败|稍后重试/)).toBeInTheDocument();
    expect(screen.queryByText(EMPTY_CLAIM)).not.toBeInTheDocument();
  });
});
