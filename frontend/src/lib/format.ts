// 跨 channel 共享的指标格式化工具（单一真相源）。
// formatRate：比率（0..1）→ 百分比字符串，null/undefined/NaN → "—"。
// formatNumber：数值 → 定点小数字符串（默认 2 位，可选 digits），null/undefined/NaN → "—"。
// autonomy / quality / review / evolution 等多个频道复用，避免各处重复定义导致漂移。
// （review/evolution 经 proposalPrimitives 以 formatNumber / formatPercent 名复用，见该文件。）

export function formatRate(value: number | null | undefined): string {
  if (value === null || value === undefined || Number.isNaN(value)) return "—";
  return `${(value * 100).toFixed(1)}%`;
}

export function formatNumber(value: number | null | undefined, digits = 2): string {
  if (value === null || value === undefined || Number.isNaN(value)) return "—";
  return Number(value).toFixed(digits);
}

/// 时间戳 → 可安全渲染的字符串。永不返回对象。
///
/// 后端时间字段的**契约**是 RFC3339 字符串（`models.rs::dt_to_string`），绝大多数
/// route 都遵守。但裸 `bson::DateTime` 一旦被 `serde_json::to_value` 整体序列化，
/// 会变成扩展 JSON 对象 `{"$date":{"$numberLong":"…"}}`；把它当 React child 渲染
/// 会抛 "Objects are not valid as a React child"，且前端无 ErrorBoundary，
/// 整个频道白屏（domain-profiles 行业配置 tab 即因此崩过）。
///
/// 后端已在各 view 层脱壳，此函数是**第二道防线**：wire 形态再漂移只会显示为
/// 时间或 "—"，不会让页面崩掉。传入字符串时原样返回，不改变既有显示。
export function formatTimestamp(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value;
  // bson 扩展 JSON：{"$date":{"$numberLong":"1782458710964"}} 或 {"$date":"2026-…Z"}
  if (typeof value === "object") {
    const inner = (value as Record<string, unknown>).$date;
    if (typeof inner === "string") return inner;
    if (typeof inner === "number") return new Date(inner).toISOString();
    if (inner && typeof inner === "object") {
      const ms = (inner as Record<string, unknown>).$numberLong;
      const parsed = typeof ms === "string" ? Number(ms) : typeof ms === "number" ? ms : NaN;
      if (Number.isFinite(parsed)) return new Date(parsed).toISOString();
    }
    return "—";
  }
  if (typeof value === "number" && Number.isFinite(value)) return new Date(value).toISOString();
  return "—";
}
