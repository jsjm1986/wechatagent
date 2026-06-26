import { describe, it, expect, vi, beforeEach } from "vitest";

const setError = vi.fn();
vi.mock("../../lib/api", () => ({ api: { get: vi.fn() } }));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError }) },
}));

import { api } from "../../lib/api";
import { useOperationsStore } from "../../stores/operationsStore";

describe("operationsStore.loadOperationsData", () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it("加载失败时上报全局错误横幅而非静默吞错", async () => {
    (api.get as any).mockRejectedValue(new Error("events 500"));
    await useOperationsStore.getState().loadOperationsData("acc-1");
    expect(setError).toHaveBeenCalledWith(expect.stringContaining("events 500"));
  });
});

describe("operationsStore.loadAgentRuns (C6)", () => {
  beforeEach(() => { vi.clearAllMocks(); });

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
