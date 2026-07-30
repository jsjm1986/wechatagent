import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStrategyStore } from "../../stores/strategyStore";
import { useUiStore } from "../../stores/uiStore";
import { api } from "../../lib/api";

// Task 8（路径B）store 三态：savePromptTemplate 必须把后端三态翻译成 SavePromptResult，
// 且 needs_human_confirm（200 body）不能被静默当成功 reload —— 这是 Task 6.6 引入的真 bug。
vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

const validDraft = {
  promptKey: "user.review.x",
  agentKind: "user",
  layer: "review",
  title: "标题",
  description: "说明",
  content: "新内容",
};

describe("strategyStore.savePromptTemplate 三态", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ busy: false, error: "" });
    useStrategyStore.setState({
      editingPromptId: "pt-1",
      promptDraft: validDraft,
      // 用 spy 拦截 reload，断言 needs_human_confirm 不触发 reload
      loadStrategyData: vi.fn().mockResolvedValue(undefined),
    });
  });

  it("Pass appends a draft and hands the new id to subsequent publish", async () => {
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      ok: true,
      id: "pt-2",
      version: 2,
      status: "draft",
    });
    const result = await useStrategyStore.getState().savePromptTemplate();
    expect(result).toEqual({ ok: true });
    expect(useStrategyStore.getState().editingPromptId).toBe("pt-2");
    expect(useStrategyStore.getState().loadStrategyData).toHaveBeenCalledTimes(1);
  });

  it("NeedsHumanConfirm(200 body) → 返回 {needsConfirm} 且不 reload、不当成功", async () => {
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      status: "needs_human_confirm",
      reason: "审查服务暂不可用",
      diff: "+转给后台老师跟进",
    });
    const result = await useStrategyStore.getState().savePromptTemplate();
    expect(result).toEqual({
      needsConfirm: true,
      reason: "审查服务暂不可用",
      diff: "+转给后台老师跟进",
    });
    // 关键：不能把待确认静默当成功 reload
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("Reject(4xx → Error 含『红线语义审查拒绝』) → 返回 {rejected}", async () => {
    (api.put as ReturnType<typeof vi.fn>).mockRejectedValueOnce(
      new Error("红线语义审查拒绝：变相引入真人转介（确认无误可带 force 覆盖）")
    );
    const result = await useStrategyStore.getState().savePromptTemplate();
    expect(result).toMatchObject({ rejected: true });
    if ("rejected" in result) {
      expect(result.reason).toContain("红线语义审查拒绝");
    }
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });

  it("force=true → PUT body 带 force:true（覆盖语义审查）", async () => {
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true, id: "pt-2" });
    await useStrategyStore.getState().savePromptTemplate(true);
    expect(api.put).toHaveBeenCalledWith(
      "/api/prompt-templates/pt-1",
      expect.objectContaining({ force: true })
    );
  });

  it("普通错误不吞：走 setError 并返回明确 error 结果", async () => {
    const setError = vi.fn();
    useUiStore.setState({ setError });
    (api.put as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("HTTP 500"));
    const result = await useStrategyStore.getState().savePromptTemplate();
    expect(result).toMatchObject({ error: true });
    expect(useStrategyStore.getState().editingPromptId).toBe("");
    expect(setError).toHaveBeenCalledWith("HTTP 500");
  });

  it("成功形状缺新版本 id 时清空旧发布目标并 fail closed", async () => {
    const setError = vi.fn();
    useUiStore.setState({ setError });
    (api.put as ReturnType<typeof vi.fn>).mockResolvedValueOnce({ ok: true });
    const result = await useStrategyStore.getState().savePromptTemplate();
    expect(result).toMatchObject({ error: true });
    expect(useStrategyStore.getState().editingPromptId).toBe("");
    expect(setError).toHaveBeenCalledWith("保存提示词版本后端未返回新版本 ID");
    expect(useStrategyStore.getState().loadStrategyData).not.toHaveBeenCalled();
  });
});
