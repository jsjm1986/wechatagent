import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SystemStrategyFeature from "../../../features/system-strategy";
import { api } from "../../../lib/api";
import { useStrategyStore } from "../../../stores/strategyStore";
import { useUiStore } from "../../../stores/uiStore";

// Task 8（路径B）：prompt 编辑器保存时识别后端三态并弹二次确认框。
// 这里用真实 store action（不 mock savePromptTemplate），只 mock api 层返回三态。
vi.mock("../../../features/system-strategy/SystemStrategy.module.css", () => ({
  default: new Proxy({}, { get: (_t, key) => String(key) }),
}));

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({ ok: true }),
    patch: vi.fn().mockResolvedValue({}),
    delete: vi.fn().mockResolvedValue({}),
    postRaw: vi.fn().mockResolvedValue({ ok: true, status: 200, data: { item: {} } }),
  },
}));

function seedEditingPrompt() {
  useStrategyStore.setState({
    editingPromptId: "pt-1",
    promptDraft: {
      promptKey: "management.x",
      agentKind: "management",
      layer: "policy",
      title: "总控策略",
      description: "说明",
      content: "新内容（含变更）",
    },
  });
}

describe("SystemStrategy 路径B 二次确认", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue({ items: [] });
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValue({ ok: true });
    useUiStore.setState({ busy: false, error: "", setBusy: vi.fn(), setError: vi.fn() });
    // 用真实 store（重新 create 后 import 已是真实实现），只设编辑态
  });

  it("needs_human_confirm(200) → 弹确认框含 diff；勾选后带 force:true 重提", async () => {
    (api.put as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce({
        status: "needs_human_confirm",
        reason: "审查服务暂不可用",
        diff: "+转给后台老师跟进",
      })
      .mockResolvedValueOnce({ ok: true });

    render(<SystemStrategyFeature />);
    seedEditingPrompt();

    // 进入编辑态后保存按钮文案为「保存修改」
    fireEvent.click(await screen.findByText("保存修改"));

    // 确认框出现，含 diff 文本与理由
    expect(await screen.findByText("+转给后台老师跟进")).toBeInTheDocument();
    expect(screen.getByText(/审查服务暂不可用/)).toBeInTheDocument();

    // requireText「已核对」解锁确认
    fireEvent.change(screen.getByPlaceholderText("已核对"), { target: { value: "已核对" } });
    fireEvent.click(screen.getByRole("button", { name: /强制保存|确认/ }));

    await waitFor(() => {
      const calls = (api.put as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls.length).toBe(2);
      expect(calls[1][1]).toMatchObject({ force: true });
    });
  });

  it("Reject(4xx) → 显示拒绝理由 + 强制保存入口；点后带 force:true", async () => {
    (api.put as ReturnType<typeof vi.fn>)
      .mockRejectedValueOnce(
        new Error("红线语义审查拒绝：变相引入真人转介（确认无误可带 force 覆盖）")
      )
      .mockResolvedValueOnce({ ok: true });

    render(<SystemStrategyFeature />);
    seedEditingPrompt();

    fireEvent.click(await screen.findByText("保存修改"));

    expect(await screen.findByText(/红线语义审查拒绝/)).toBeInTheDocument();
    // 强制保存入口
    fireEvent.change(screen.getByPlaceholderText("已核对"), { target: { value: "已核对" } });
    fireEvent.click(screen.getByRole("button", { name: /强制保存|确认/ }));

    await waitFor(() => {
      const calls = (api.put as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls.length).toBe(2);
      expect(calls[1][1]).toMatchObject({ force: true });
    });
  });
});
