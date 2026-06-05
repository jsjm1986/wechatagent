// 跨 channel 共享的指标格式化工具。
// formatRate：比率（0..1）→ 百分比字符串，null → "—"。
// formatNumber：数值 → 两位小数字符串，null → "—"。
// autonomy / quality 等多个频道复用，避免各处重复定义导致漂移。

export function formatRate(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  return `${(value * 100).toFixed(1)}%`;
}

export function formatNumber(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  return value.toFixed(2);
}
