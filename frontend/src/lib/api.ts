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
  }
};

export { parseApiError };