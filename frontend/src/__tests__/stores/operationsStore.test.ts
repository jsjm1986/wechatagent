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
