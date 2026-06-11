export class LlmUnavailableError extends Error {
  kind: string;
  retryCount: number;
  detail: string;
  hint: string;
  constructor(payload: { kind: string; retryCount: number; detail: string; hint: string }) {
    super(payload.hint || payload.detail || "LLM 暂不可用");
    this.name = "LlmUnavailableError";
    this.kind = payload.kind;
    this.retryCount = payload.retryCount;
    this.detail = payload.detail;
    this.hint = payload.hint;
  }
}

async function parseApiError(response: Response): Promise<Error> {
  const text = await response.text();
  try {
    const json = JSON.parse(text) as Record<string, unknown>;
    if (json && json.error === "llm_unavailable") {
      return new LlmUnavailableError({
        kind: typeof json.kind === "string" ? json.kind : "unknown",
        retryCount: typeof json.retryCount === "number" ? json.retryCount : 0,
        detail: typeof json.detail === "string" ? json.detail : text,
        hint:
          typeof json.hint === "string"
            ? json.hint
            : "调用 LLM 失败，请稍后再试。"
      });
    }
    if (json && typeof json.error === "string") {
      return new Error(json.error);
    }
  } catch {
    /* 不是 JSON：可能是 SPA fallback 返回的 HTML / Axum 默认 405 文本，
       不能把整段 body 当成错误信息渲染（screenshot 里就是因为这个把
       <!doctype html> 整个塞进 UI）。统一脱壳成 HTTP 状态码。 */
  }
  const ct = response.headers.get("content-type") ?? "";
  if (ct.toLowerCase().includes("text/html")) {
    return new Error(`HTTP ${response.status}（服务端未返回 JSON，可能是后端尚未编译该接口）`);
  }
  const trimmed = text.trim();
  if (!trimmed || /^<!doctype|^<html/i.test(trimmed)) {
    return new Error(`HTTP ${response.status}`);
  }
  // 非 JSON 但是是较短的纯文本（如 axum 的 "Method Not Allowed"），保留前 120 字
  const safe = trimmed.length > 120 ? `${trimmed.slice(0, 120)}…` : trimmed;
  return new Error(`HTTP ${response.status}：${safe}`);
}

export const api = {
  async get<T>(url: string): Promise<T> {
    const response = await fetch(url);
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  async post<T>(url: string, body?: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined
    });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  async put<T>(url: string, body: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  async delete<T>(url: string): Promise<T> {
    const response = await fetch(url, { method: "DELETE" });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  /// multipart 文件上传（PDF / 图片）。不设 Content-Type，让浏览器带 boundary。
  async postForm<T>(url: string, form: FormData): Promise<T> {
    const response = await fetch(url, { method: "POST", body: form });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  /// 不抛错的原始 POST：返回 { ok, status, data }，调用方自行处理非 2xx
  /// （如 lock 409 带 payload 的分支）。data 解析失败时为 null。
  async postRaw<T>(
    url: string,
    body?: unknown
  ): Promise<{ ok: boolean; status: number; data: T | null }> {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined,
    });
    let data: T | null = null;
    try {
      data = (await response.json()) as T;
    } catch {
      data = null;
    }
    return { ok: response.ok, status: response.status, data };
  },
};

/// 统一 SSE 订阅：封装 close/error 兜底，返回关闭函数。
/// 替代散落的裸 EventSource（断流不收尾、error 靠旧闭包判断等坑）。
export function openEventSource(
  url: string,
  handlers: {
    onEvent?: (type: string, data: string) => void;
    onError?: () => void;
    events?: string[];
  }
): () => void {
  if (typeof window === "undefined" || typeof window.EventSource === "undefined") {
    return () => {};
  }
  const es = new EventSource(url);
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    es.close();
  };
  for (const evt of handlers.events ?? []) {
    es.addEventListener(evt, (e) => handlers.onEvent?.(evt, (e as MessageEvent).data));
  }
  es.addEventListener("error", () => {
    handlers.onError?.();
    close();
  });
  return close;
}

export { parseApiError };