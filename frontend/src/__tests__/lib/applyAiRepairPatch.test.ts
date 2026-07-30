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
  it("勾选2/4字段：一次提交 proposal patch 与 acceptedFields，由服务端筛选落库", async () => {
    fetchSpy.mockResolvedValueOnce(okResp());
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary", "title"] });
    expect(r.ok).toBe(true);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    const [appliedUrl, appliedInit] = fetchSpy.mock.calls[0];
    expect(appliedUrl).toContain("/api/operation-knowledge/repair/applied");
    const aBody = JSON.parse(appliedInit.body);
    expect(aBody.targetKind).toBe("chunk");
    expect(aBody.targetId).toBe("c1");
    expect(aBody.patch).toEqual(BASE.patch);
    expect(aBody.sessionId).toBe("s1");
    expect(aBody.acceptedFields.sort()).toEqual(["summary", "title"]);
    expect(aBody.skippedFields.sort()).toEqual(["body", "sourceQuote"]); // patch有但没勾
    expect(aBody.thenVerify).toBe(false);              // 红线：恒false
    expect(aBody).not.toHaveProperty("then_verify");   // serde命门：camelCase
  });

  it("服务端提交失败：返回apply_failed", async () => {
    fetchSpy.mockResolvedValueOnce(errResp(400));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("apply_failed");
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it("fetch reject 归一为 server_error 不冒泡", async () => {
    fetchSpy.mockRejectedValueOnce(new Error("network"));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("server_error");
  });
});
