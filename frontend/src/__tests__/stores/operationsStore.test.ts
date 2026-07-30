import { describe, it, expect, vi, beforeEach } from "vitest";

const setError = vi.fn();
vi.mock("../../lib/api", () => ({ api: { get: vi.fn(), post: vi.fn() } }));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError }) },
}));

import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import { useOperationsStore } from "../../stores/operationsStore";
import type { TaskItem } from "../../types";

function selectAccount(accountId: string): void {
  useAccountStore.setState({
    accounts: [
      { id: accountId, accountId, alias: accountId, displayName: accountId, online: true },
    ] as any,
    selectedAccountId: accountId,
  });
}

function resetOperations(): void {
  useOperationsStore.setState({
    events: [],
    tasks: [],
    decisionReviews: [],
    llmUsage: null,
    agentRuns: [],
    dataAccountId: "",
    requestGeneration: 0,
    agentRunsGeneration: 0,
    loading: false,
  });
}

function task(id: string, accountId: string): TaskItem {
  return {
    id,
    accountId,
    contactWxid: `wx-${id}`,
    kind: "follow_up",
    content: id,
    status: "pending",
  };
}

describe("operationsStore.loadOperationsData", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetOperations();
    selectAccount("acc-1");
  });

  it("加载失败时上报全局错误横幅而非静默吞错", async () => {
    (api.get as any).mockRejectedValue(new Error("events 500"));
    await useOperationsStore.getState().loadOperationsData("acc-1");
    expect(setError).toHaveBeenCalledWith(expect.stringContaining("events 500"));
  });

  it("A 慢 B 快时只提交当前账号 B 的快照", async () => {
    const pending = new Map<string, Array<(value: any) => void>>();
    (api.get as any).mockImplementation((url: string) => new Promise((resolve) => {
      const account = url.includes("accountId=acc-a") ? "acc-a" : "acc-b";
      const list = pending.get(account) ?? [];
      list.push(resolve);
      pending.set(account, list);
    }));

    selectAccount("acc-a");
    const loadA = useOperationsStore.getState().loadOperationsData("acc-a");
    selectAccount("acc-b");
    const loadB = useOperationsStore.getState().loadOperationsData("acc-b");

    const bResolvers = pending.get("acc-b") ?? [];
    bResolvers.forEach((resolve, index) => resolve(index === 1
      ? { items: [task("b-task", "acc-b")] }
      : index === 3
        ? { summary: {}, items: [] }
        : { items: [] }));
    await loadB;

    const aResolvers = pending.get("acc-a") ?? [];
    aResolvers.forEach((resolve, index) => resolve(index === 1
      ? { items: [task("a-task", "acc-a")] }
      : index === 3
        ? { summary: {}, items: [] }
        : { items: [] }));
    await loadA;

    expect(useOperationsStore.getState().dataAccountId).toBe("acc-b");
    expect(useOperationsStore.getState().tasks.map((item) => item.id)).toEqual(["b-task"]);
  });
});

describe("operationsStore task action scope", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetOperations();
    selectAccount("acc-a");
    (api.get as any).mockResolvedValue({ items: [] });
    (api.post as any).mockResolvedValue({ ok: true });
  });

  it("提交任务实体冻结的 expectedAccountId", async () => {
    const rendered = task("task-a", "acc-a");
    useOperationsStore.setState({ dataAccountId: "acc-a", tasks: [rendered] });

    await useOperationsStore.getState().cancelTask(rendered, "acc-a");

    expect(api.post).toHaveBeenCalledWith("/api/agent-tasks/task-a/cancel", {
      expectedAccountId: "acc-a",
    });
  });

  it("切号后旧任务动作零请求", async () => {
    const rendered = task("task-a", "acc-a");
    useOperationsStore.setState({ dataAccountId: "acc-a", tasks: [rendered] });
    selectAccount("acc-b");

    await useOperationsStore.getState().reviewTaskNow(rendered, "acc-a");

    expect(api.post).not.toHaveBeenCalled();
  });
});

describe("operationsStore.loadAgentRuns (C6)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetOperations();
    selectAccount("acc1");
    useOperationsStore.setState({ dataAccountId: "acc1" });
  });

  it("loadAgentRuns 拉取 /api/agent-runs 并落 agentRuns 状态", async () => {
    (api.get as any).mockResolvedValue({
      items: [
        {
          id: "r1",
          runId: "run-1",
          status: "succeeded",
          triggerKind: "inbound",
          decision: { sufficiency: "enough", missingTier: "none" },
          gatewayResult: { status: "sent" },
        },
      ],
    });
    await useOperationsStore.getState().loadAgentRuns("acc1");
    expect(api.get).toHaveBeenCalledWith(expect.stringContaining("/api/agent-runs"));
    expect(api.get).toHaveBeenCalledWith(expect.stringContaining("accountId=acc1"));
    expect(useOperationsStore.getState().agentRuns).toHaveLength(1);
    expect(useOperationsStore.getState().agentRuns[0].runId).toBe("run-1");
  });

  it("loadAgentRuns 失败时上报错误横幅并置空 agentRuns", async () => {
    (api.get as any).mockRejectedValue(new Error("agent-runs 500"));
    await useOperationsStore.getState().loadAgentRuns("acc1");
    expect(setError).toHaveBeenCalledWith(expect.stringContaining("agent-runs 500"));
    expect(useOperationsStore.getState().agentRuns).toEqual([]);
  });
});
