import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStrategyStore } from "../../stores/strategyStore";
import { useUiStore } from "../../stores/uiStore";
import { api } from "../../lib/api";

// #2 修复：后端 publish 补 LLM 红线三闸后，publishPromptTemplate 必须读 200 三态，
// needs_human_confirm 不能静默当成功 reload。
vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

describe("strategyStore.publishPromptTemplate 三态", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "" });
    useStrategyStore.setState({
      loadStrategyData: vi.fn().mockResolvedValue(undefined),
    });
  });

  it("ok(200 {ok:true}) → 返回 {ok:true} 且触发 reload", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true });
    const result = await useStrategyStore.getState().publishPromptTemplate("pt-1");
    expect(result).toEqual({ ok: true });
    expect(useStrategyStore.getState().loadStrategyData).toHaveBeenCalledTimes(1);
  });

  it("NeedsHumanConfirm(200 body) → 返回 {needsConfirm} 且不 reload", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      status: "needs_human_confirm",
      reason: "审查服务暂不可用",
      diff: "+变相转介",
    });
    const result = await useStrategyStore.getState().publishPromptTemplate("pt-1");
    expect(result).toEqual({ needsConfirm: true, reason: "审查服务暂不可用", diff: "+变相转介" });
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("Reject(4xx Error 含『红线语义审查拒绝』) → 返回 {rejected}", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("红线语义审查拒绝：变相引入真人转介（确认无误可带 force 覆盖）")
    );
    const result = await useStrategyStore.getState().publishPromptTemplate("pt-1");
    expect(result).toMatchObject({ rejected: true });
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("force=true → POST body 带 force:true", async () => {
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true });
    await useStrategyStore.getState().publishPromptTemplate("pt-1", true);
    expect(api.post).toHaveBeenCalledWith(
      "/api/prompt-templates/pt-1/publish",
      expect.objectContaining({ force: true })
    );
  });
});
