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
