import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";
import { useStrategyStore } from "../../stores/strategyStore";
import { useUiStore } from "../../stores/uiStore";

vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({ ok: true }),
    put: vi.fn(),
  },
}));

describe("strategyStore Soul 不可变版本身份", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "" });
    useStrategyStore.setState({
      editingSoulId: "published-v1",
      soulDraft: {
        agentKind: "user",
        name: "edited Soul",
        content: "new immutable content",
      },
      loadStrategyData: vi.fn().mockResolvedValue(undefined),
    });
  });

  it("保存后把发布目标切换到 PUT 返回的新草稿 ID", async () => {
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      ok: true,
      id: "draft-v2",
      version: 2,
    });

    await useStrategyStore.getState().saveSoul();

    expect(api.put).toHaveBeenCalledWith(
      "/api/agent-souls/published-v1",
      expect.objectContaining({ content: "new immutable content" })
    );
    expect(useStrategyStore.getState().editingSoulId).toBe("draft-v2");

    await useStrategyStore
      .getState()
      .publishSoul(useStrategyStore.getState().editingSoulId);
    expect(api.post).toHaveBeenCalledWith("/api/agent-souls/draft-v2/publish");
  });

  it("缺少新版本 ID 时不保留旧发布目标并显示错误", async () => {
    const setError = vi.fn();
    useUiStore.setState({ setError });
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true });

    await useStrategyStore.getState().saveSoul();

    expect(useStrategyStore.getState().editingSoulId).toBe("");
    expect(setError).toHaveBeenCalledWith("保存人格版本后端未返回新版本 ID");
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("PUT failure clears the stale publish target", async () => {
    const setError = vi.fn();
    useUiStore.setState({ setError });
    (api.put as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("HTTP 409"));

    await useStrategyStore.getState().saveSoul();

    expect(useStrategyStore.getState().editingSoulId).toBe("");
    expect(setError).toHaveBeenCalledWith("HTTP 409");
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });
});
