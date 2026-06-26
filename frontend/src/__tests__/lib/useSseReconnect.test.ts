import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createSseReconnector } from "../../lib/useSseReconnect";

class FakeES {
  static instances: FakeES[] = [];
  url: string;
  listeners: Record<string, ((ev: unknown) => void)[]> = {};
  closed = false;
  constructor(url: string) { this.url = url; FakeES.instances.push(this); }
  addEventListener(t: string, cb: (ev: unknown) => void) { (this.listeners[t] ||= []).push(cb); }
  close() { this.closed = true; }
  emit(t: string, ev?: unknown) { (this.listeners[t] || []).forEach((cb) => cb(ev)); }
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeES.instances = [];
  vi.stubGlobal("EventSource", FakeES as unknown as typeof EventSource);
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe("createSseReconnector", () => {
  it("error 触发指数退避重连（base×2^attempt）", () => {
    const r = createSseReconnector("/s", { onEvent: {}, baseDelayMs: 1000, capMs: 30000, maxRetries: 5 });
    expect(FakeES.instances).toHaveLength(1);
    FakeES.instances[0].emit("error");          // 第 1 次断连
    expect(FakeES.instances[0].closed).toBe(true);
    vi.advanceTimersByTime(999); expect(FakeES.instances).toHaveLength(1); // 还没到 1000ms
    vi.advanceTimersByTime(1);   expect(FakeES.instances).toHaveLength(2); // 1000ms 后重连
    FakeES.instances[1].emit("error");          // 第 2 次断连
    vi.advanceTimersByTime(2000); expect(FakeES.instances).toHaveLength(3); // 2000ms 后
    r.close();
  });

  it("达 maxRetries 停止重连", () => {
    const r = createSseReconnector("/s", { onEvent: {}, baseDelayMs: 100, capMs: 30000, maxRetries: 2 });
    for (let i = 0; i < 5; i++) {
      const es = FakeES.instances[FakeES.instances.length - 1];
      es.emit("error");
      vi.advanceTimersByTime(60000);
    }
    expect(FakeES.instances.length).toBeLessThanOrEqual(3); // 初次 + 最多 2 次重连
    r.close();
  });

  it("成功事件重置 attempt", () => {
    const r = createSseReconnector("/s", { onEvent: { turn: () => {} }, baseDelayMs: 1000, capMs: 30000, maxRetries: 5 });
    FakeES.instances[0].emit("error");
    vi.advanceTimersByTime(1000);                // 重连 → instances[1]
    FakeES.instances[1].emit("turn");            // 成功事件 → 重置 attempt
    FakeES.instances[1].emit("error");
    vi.advanceTimersByTime(1000);                // attempt 重置后仍是 base×2^0=1000ms
    expect(FakeES.instances).toHaveLength(3);
    r.close();
  });

  it("close() 后不再重连", () => {
    const r = createSseReconnector("/s", { onEvent: {}, baseDelayMs: 100, capMs: 30000, maxRetries: 5 });
    r.close();
    FakeES.instances[0].emit("error");
    vi.advanceTimersByTime(60000);
    expect(FakeES.instances).toHaveLength(1);
  });
});
