// SSE 指数退避自动重连器。仅用于「长连接监听流」（断连重连幂等、只触发 reload）。
// 严禁用于一次性 RPC 流（如 /knowledge/ask/stream）——重连会重发查询、重复扣 token。
//
// 退避：delay = min(capMs, baseDelayMs × 2^attempt)。达 maxRetries 停止。任一注册事件触发即重置 attempt。
// 调用方负责在组件卸载 / 主动取消时调 close()，停止重连且清理 EventSource。
export interface SseReconnectOptions {
  // 事件名 → 回调。任一事件触发都视为「连接健康」，重置退避 attempt。
  onEvent: Record<string, (ev: MessageEvent) => void>;
  // 终止事件名清单（如后端正常终结流推的 "close"）。收到即停止重连并关闭 EventSource，
  // 等同主动 close() 的效果（但保留 handle 不变）。后端契约：收到 close 不应再重连、占用连接。
  terminalEvents?: string[];
  onReconnecting?: (attempt: number, delayMs: number) => void;
  onGaveUp?: () => void;
  baseDelayMs?: number; // 默认 1000
  capMs?: number;       // 默认 30000
  maxRetries?: number;  // 默认 6
}

export interface SseHandle {
  close: () => void;
}

export function createSseReconnector(url: string, opts: SseReconnectOptions): SseHandle {
  const base = opts.baseDelayMs ?? 1000;
  const cap = opts.capMs ?? 30000;
  const maxRetries = opts.maxRetries ?? 6;
  const terminalEvents = opts.terminalEvents ?? [];
  let attempt = 0;
  let stopped = false;
  let es: EventSource | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const clearTimer = () => { if (timer !== null) { clearTimeout(timer); timer = null; } };
  const cleanupEs = () => { if (es) { es.close(); es = null; } };
  const stop = () => { stopped = true; clearTimer(); cleanupEs(); };

  function connect() {
    if (stopped) return;
    cleanupEs();
    const next = new EventSource(url);
    es = next;
    for (const [name, cb] of Object.entries(opts.onEvent)) {
      next.addEventListener(name, (ev) => {
        attempt = 0; // 收到业务事件 → 连接健康，重置退避
        cb(ev as MessageEvent);
      });
    }
    // 终止事件：后端正常终结流（如推 "close"）→ 停止重连并关闭连接，遵守「收到 close 不再重连」契约。
    // stopped=true 后即便浏览器随即触发 error，error handler 的 `if (stopped) return` 也会早返回挡住。
    for (const name of terminalEvents) {
      next.addEventListener(name, () => { stop(); });
    }
    next.addEventListener("error", () => {
      if (stopped) return;
      cleanupEs();
      if (attempt >= maxRetries) { opts.onGaveUp?.(); return; }
      const delay = Math.min(cap, base * 2 ** attempt);
      attempt += 1;
      opts.onReconnecting?.(attempt, delay);
      clearTimer();
      timer = setTimeout(connect, delay);
    });
  }

  connect();
  return {
    close() { stop(); },
  };
}
