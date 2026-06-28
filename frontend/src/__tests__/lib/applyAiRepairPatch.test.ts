import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { applyAiRepairPatch } from "../../lib/applyAiRepairPatch";

const okResp = (body: unknown = {}) => ({ ok: true, status: 200, json: async () => body }) as Response;
const errResp = (status: number) => ({ ok: false, status, json: async () => ({}) }) as Response;

let fetchSpy: ReturnType<typeof vi.fn>;
beforeEach(() => { fetchSpy = vi.fn(); vi.stubGlobal("fetch", fetchSpy); });
afterEach(() => { vi.unstubAllGlobals(); });

const BASE = {
  chunkId: "c1",
  originalChunk: { title: "原标题", body: "原body", summary: "原summary", sourceQuote: "原quote" },
  patch: { summary: "AI改的summary", sourceQuote: "AI脑补的quote", title: "AI改的标题", body: "AI改的body" },
  sessionId: "s1", turn: 1, confidenceHint: 70,
};

describe("applyAiRepairPatch", () => {
  it("勾选2/4字段：PUT body只含勾选字段覆盖+原值其余保留（防清空），applied分组正确", async () => {
    fetchSpy.mockResolvedValueOnce(okResp()).mockResolvedValueOnce(okResp());
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary", "title"] });
    expect(r.ok).toBe(true);
    // 第一个 fetch = PUT 落库
    const [putUrl, putInit] = fetchSpy.mock.calls[0];
    expect(putUrl).toContain("/api/operation-knowledge/chunks/c1");
    expect(putInit.method).toBe("PUT");
    const putBody = JSON.parse(putInit.body);
    expect(putBody.summary).toBe("AI改的summary");   // 勾选→用patch值
    expect(putBody.title).toBe("AI改的标题");          // 勾选→用patch值
    expect(putBody.body).toBe("原body");               // 没勾→保留原值（防清空）
    expect(putBody.sourceQuote).toBe("原quote");       // 没勾→保留原值（防清空，不被AI脑补覆盖）
    // 第二个 fetch = applied 闭账
    const [appliedUrl, appliedInit] = fetchSpy.mock.calls[1];
    expect(appliedUrl).toContain("/api/operation-knowledge/repair/applied");
    const aBody = JSON.parse(appliedInit.body);
    expect(aBody.targetKind).toBe("chunk");
    expect(aBody.targetId).toBe("c1");
    expect(aBody.sessionId).toBe("s1");
    expect(aBody.acceptedFields.sort()).toEqual(["summary", "title"]);
    expect(aBody.skippedFields.sort()).toEqual(["body", "sourceQuote"]); // patch有但没勾
    expect(aBody.thenVerify).toBe(false);              // 红线：恒false
    expect(aBody).not.toHaveProperty("then_verify");   // serde命门：camelCase
  });

  it("PUT落库失败：不发applied，返回apply_failed", async () => {
    fetchSpy.mockResolvedValueOnce(errResp(400));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("apply_failed");
    expect(fetchSpy).toHaveBeenCalledTimes(1); // 没发 applied
  });

  it("落库成功但闭账失败：返回audit_failed（不误报落库失败），message提示已落库", async () => {
    fetchSpy.mockResolvedValueOnce(okResp()).mockResolvedValueOnce(errResp(500));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("audit_failed");
    expect(r.message).toMatch(/已落库/);
  });

  it("fetch reject 归一为 server_error 不冒泡", async () => {
    fetchSpy.mockRejectedValueOnce(new Error("network"));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("server_error");
  });
});
