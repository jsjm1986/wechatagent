import { describe, it, expect, vi } from "vitest";
import { runGoLive } from "../../features/knowledge/cockpit/useGoLive";

describe("runGoLive(apply→verify 串调)", () => {
  it("apply 成功后才调 verify,两步都成功返回 ok", async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve({ ok: true, json: () => Promise.resolve({}) } as Response);
    }) as unknown as typeof fetch;
    const r = await runGoLive({ sessionId: "s1", chunkId: "c1" });
    expect(r.ok).toBe(true);
    expect(calls[0]).toContain("/chat/s1/apply");
    expect(calls[1]).toContain("/chunks/c1/verify");
  });
  it("verify 被 D2 闸拒(4xx)→ 返回 gate_blocked,不抛错", async () => {
    globalThis.fetch = vi.fn((url: string) =>
      Promise.resolve(String(url).includes("/verify")
        ? ({ ok: false, status: 400, json: () => Promise.resolve({ error: "缺 source_anchors" }) } as Response)
        : ({ ok: true, json: () => Promise.resolve({}) } as Response))
    ) as unknown as typeof fetch;
    const r = await runGoLive({ sessionId: "s1", chunkId: "c1" });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("gate_blocked");
  });
  it("无 sessionId(没经过对话)→ 跳过 apply 直接 verify", async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve({ ok: true, json: () => Promise.resolve({}) } as Response);
    }) as unknown as typeof fetch;
    const r = await runGoLive({ chunkId: "c1" });
    expect(r.ok).toBe(true);
    expect(calls.every((c) => !c.includes("/apply"))).toBe(true);
    expect(calls[0]).toContain("/chunks/c1/verify");
  });
  it("apply 失败 → 返回 apply_failed,不调 verify", async () => {
    const calls: string[] = [];
    globalThis.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve({ ok: false, status: 500, json: () => Promise.resolve({}) } as Response);
    }) as unknown as typeof fetch;
    const r = await runGoLive({ sessionId: "s1", chunkId: "c1" });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("apply_failed");
    expect(calls.some((c) => c.includes("/verify"))).toBe(false);
  });
  it("verify 服务端 5xx → 返回 server_error(区别于 4xx 的 gate_blocked)", async () => {
    globalThis.fetch = vi.fn((url: string) =>
      Promise.resolve(String(url).includes("/verify")
        ? ({ ok: false, status: 503, json: () => Promise.resolve({}) } as Response)
        : ({ ok: true, json: () => Promise.resolve({}) } as Response))
    ) as unknown as typeof fetch;
    const r = await runGoLive({ sessionId: "s1", chunkId: "c1" });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("server_error");
  });
  it("网络异常(fetch reject)→ 归一为 server_error,不抛出 unhandled rejection", async () => {
    globalThis.fetch = vi.fn(() => Promise.reject(new Error("network down"))) as unknown as typeof fetch;
    const r = await runGoLive({ chunkId: "c1" });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("server_error");
  });
});
