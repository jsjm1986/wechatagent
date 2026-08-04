import { describe, it, expect, vi } from "vitest";
import { runGoLive } from "../../features/knowledge/cockpit/useGoLive";

const SNAPSHOT = "2026-08-05T01:00:00Z";
const APPLIED = "2026-08-05T01:00:01Z";

function ok(body: unknown = {}) {
  return { ok: true, status: 200, json: () => Promise.resolve(body) } as Response;
}

describe("runGoLive(apply→verify 串调)", () => {
  it("apply 后用回执的新版本核验，而不是管理员看到的旧版本", async () => {
    const requests: { url: string; init?: RequestInit }[] = [];
    globalThis.fetch = vi.fn((url: string, init?: RequestInit) => {
      requests.push({ url: String(url), init });
      return Promise.resolve(
        String(url).includes("/apply") ? ok({ result: { updatedAt: APPLIED } }) : ok(),
      );
    }) as unknown as typeof fetch;

    const result = await runGoLive({
      sessionId: "s1",
      chunkId: "c1",
      expectedUpdatedAt: SNAPSHOT,
    });

    expect(result.ok).toBe(true);
    expect(requests[0].url).toContain("/chat/s1/apply");
    expect(requests[1].url).toContain("/chunks/c1/verify");
    expect(JSON.parse(String(requests[1].init?.body))).toEqual({ expectedUpdatedAt: APPLIED });
  });

  it("apply 回执缺新版本时 fail closed 且不调 verify", async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve(ok({ result: {} }));
    }) as unknown as typeof fetch;
    const result = await runGoLive({ sessionId: "s1", chunkId: "c1", expectedUpdatedAt: SNAPSHOT });
    expect(result.reason).toBe("apply_failed");
    expect(calls.some((url) => url.includes("/verify"))).toBe(false);
  });

  it("verify 被 D2/版本闸拒绝时返回 gate_blocked", async () => {
    globalThis.fetch = vi.fn((url: string) =>
      Promise.resolve(
        String(url).includes("/verify")
          ? ({ ok: false, status: 409, json: () => Promise.resolve({ error: "chunk_revision_conflict" }) } as Response)
          : ok({ result: { updatedAt: APPLIED } }),
      ),
    ) as unknown as typeof fetch;
    const result = await runGoLive({ sessionId: "s1", chunkId: "c1", expectedUpdatedAt: SNAPSHOT });
    expect(result.reason).toBe("gate_blocked");
  });

  it("无 sessionId 时直接核验管理员看到的版本", async () => {
    let verifyBody: unknown;
    globalThis.fetch = vi.fn((_url: string, init?: RequestInit) => {
      verifyBody = JSON.parse(String(init?.body));
      return Promise.resolve(ok());
    }) as unknown as typeof fetch;
    const result = await runGoLive({ chunkId: "c1", expectedUpdatedAt: SNAPSHOT });
    expect(result.ok).toBe(true);
    expect(verifyBody).toEqual({ expectedUpdatedAt: SNAPSHOT });
  });

  it("缺少版本令牌时 fail closed 且不发请求", async () => {
    const fetchMock = vi.fn();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const result = await runGoLive({ chunkId: "c1", expectedUpdatedAt: "  " });
    expect(result.reason).toBe("gate_blocked");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("apply 失败不调 verify", async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve({ ok: false, status: 500 } as Response);
    }) as unknown as typeof fetch;
    const result = await runGoLive({ sessionId: "s1", chunkId: "c1", expectedUpdatedAt: SNAPSHOT });
    expect(result.reason).toBe("apply_failed");
    expect(calls.some((url) => url.includes("/verify"))).toBe(false);
  });

  it("verify 5xx 与网络异常归一为 server_error", async () => {
    globalThis.fetch = vi.fn(() => Promise.resolve({ ok: false, status: 503 } as Response)) as unknown as typeof fetch;
    expect((await runGoLive({ chunkId: "c1", expectedUpdatedAt: SNAPSHOT })).reason).toBe("server_error");
    globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down"))) as unknown as typeof fetch;
    expect((await runGoLive({ chunkId: "c1", expectedUpdatedAt: SNAPSHOT })).reason).toBe("server_error");
  });
});
