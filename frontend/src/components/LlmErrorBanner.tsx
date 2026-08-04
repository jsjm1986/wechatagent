import { LlmUnavailableError } from "../lib/api";
import "./LlmErrorBanner.css";

export type LlmErrorPayload = {
  kind?: string;
  retryCount?: number;
  detail?: string;
  hint?: string;
};

const LLM_KIND_LABELS: Record<string, string> = {
  timeout: "上游超时",
  connect_failed: "无法连接",
  body_decode_error: "响应体损坏",
  network_error: "网络异常",
  rate_limited: "上游限流",
  http_5xx: "上游 5xx",
  http_4xx: "上游 4xx",
  endpoint_not_found: "地址路径错(404)",
  empty_response: "空响应",
  external_error: "上游错误",
  json_decode_error: "JSON 解析失败",
  client_error: "客户端错误",
  unknown: "未知错误"
};

function normalizeError(error: Error | LlmErrorPayload): Required<LlmErrorPayload> {
  if (error instanceof LlmUnavailableError) {
    return {
      kind: error.kind || "unknown",
      retryCount: error.retryCount || 0,
      detail: error.detail || "",
      hint: error.hint || error.message || "调用 LLM 失败，请稍后再试。"
    };
  }
  if (error instanceof Error) {
    return {
      kind: "unknown",
      retryCount: 0,
      detail: "",
      hint: error.message || "调用 LLM 失败，请稍后再试。"
    };
  }
  return {
    kind: error.kind || "unknown",
    retryCount: error.retryCount || 0,
    detail: error.detail || "",
    hint: error.hint || error.detail || "调用 LLM 失败，请稍后再试。"
  };
}

export function LlmErrorBanner({
  error,
  onRetry,
  retrying
}: {
  error: Error | LlmErrorPayload;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  const normalized = normalizeError(error);
  return (
    <div className="llmErrorBanner" role="alert">
      <div className="llmErrorBanner__head">
        <span className="llmErrorBanner__kind">
          {LLM_KIND_LABELS[normalized.kind] ?? normalized.kind}
        </span>
        {normalized.retryCount > 0 ? (
          <span className="llmErrorBanner__retries">
            已自动重试 {normalized.retryCount} 次
          </span>
        ) : null}
      </div>
      <div className="llmErrorBanner__hint">{normalized.hint}</div>
      {normalized.detail && normalized.detail !== normalized.hint ? (
        <details className="llmErrorBanner__detail">
          <summary>查看技术细节</summary>
          <code>{normalized.detail}</code>
        </details>
      ) : null}
      {onRetry ? (
        <div className="llmErrorBanner__actions">
          <button type="button" className="primary" onClick={onRetry} disabled={retrying}>
            {retrying ? "AI 重试中…" : "AI 重试"}
          </button>
        </div>
      ) : null}
    </div>
  );
}
