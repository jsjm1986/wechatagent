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
  // 普通 Error(非 LlmUnavailableError)意味着失败发生在**客户端**——请求可能压根没
  // 发出去。此前这里回落成 kind:"unknown" → 标题显示「未知错误」，把一个前端
  // TypeError 冒充成上游模型故障（`crypto.randomUUID is not a function` 就这样被
  // 渲染成了 LLM 错误横幅）。改用 client_error：该键早已在 LLM_KIND_LABELS 里
  // 备好「客户端错误」文案，只是从没被 set 过。
  if (error instanceof Error) {
    return {
      kind: "client_error",
      retryCount: 0,
      detail: "",
      hint: error.message || "操作失败，请稍后再试。"
    };
  }
  return {
    kind: error.kind || "unknown",
    retryCount: error.retryCount || 0,
    detail: error.detail || "",
    hint: error.hint || error.detail || "调用 LLM 失败，请稍后再试。"
  };
}

/// 重试按钮文案。`client_error`（本地/浏览器端故障，请求根本没发出去）不能写
/// 「AI 重试」——那会让运营以为模型又跑了一遍。此时按调用方给的动作名（默认「重试」）
/// 陈述事实：重试的是这一次前端操作，不是一次 AI 调用。
function retryLabel(kind: string, retrying: boolean | undefined, actionLabel?: string): string {
  if (kind === "client_error") {
    const verb = actionLabel ?? "重试";
    return retrying ? `${verb}中…` : verb;
  }
  return retrying ? "AI 重试中…" : "AI 重试";
}

export function LlmErrorBanner({
  error,
  onRetry,
  retrying,
  retryActionLabel
}: {
  error: Error | LlmErrorPayload;
  onRetry?: () => void;
  retrying?: boolean;
  /// 本地故障（kind=client_error）时按钮显示的动作名，如「重新加载」。
  /// LLM 上游故障不受影响，仍显示「AI 重试」。
  retryActionLabel?: string;
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
            {retryLabel(normalized.kind, retrying, retryActionLabel)}
          </button>
        </div>
      ) : null}
    </div>
  );
}
