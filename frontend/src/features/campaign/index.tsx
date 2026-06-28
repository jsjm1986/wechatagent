import { useEffect, useState } from "react";
import { Megaphone } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CampaignSendItem } from "../../stores/campaignStore";
import styles from "./Campaign.module.css";

const BUCKETS = ["sent", "pending", "blocked", "escalated", "canceled", "skipped", "unknown"] as const;

export function bucketTone(bucket: string): StatusTone {
  switch (bucket) {
    case "sent": return "running";
    case "pending": return "scheduled";
    case "blocked": return "blocked";
    case "escalated": return "held";
    default: return "inactive"; // canceled / skipped / unknown / 未知值
  }
}

export function bucketLabel(bucket: string): string {
  switch (bucket) {
    case "sent": return "已送达";
    case "pending": return "在途";
    case "blocked": return "被拦";
    case "escalated": return "已请示";
    case "canceled": return "已取消";
    case "skipped": return "去重跳过";
    default: return "未知";
  }
}

// 标量桶取 summary 上的计数；reason 桶取子 map 总和。
function bucketCount(summary: Record<string, unknown>, bucket: string): number {
  const v = summary[bucket];
  if (typeof v === "number") return v;
  if (v && typeof v === "object") {
    return Object.values(v as Record<string, number>).reduce((a, b) => a + b, 0);
  }
  return 0;
}

export default function CampaignFeature() {
  const { selectedCampaignId, report, loadReport } = useCampaignStore();
  const [filter, setFilter] = useState<string>("all");

  // 直接切到本频道（未经 openReport）且有选中 id 但无 report 且不在加载中 → 补一次加载。
  const loading = useCampaignStore((s) => s.loading);
  useEffect(() => {
    if (selectedCampaignId && !report && !loading) void loadReport(selectedCampaignId);
  }, [selectedCampaignId, report, loading, loadReport]);

  if (!selectedCampaignId) {
    return (
      <div className={styles.page}>
        <EmptyState
          icon={<Megaphone size={28} />}
          title="暂无活动结果"
          hint="在 AI 总控 dispatch 活动后，点「查看推送结果」进入这里查看真实触达分布。"
        />
      </div>
    );
  }

  const summary = report?.summary;
  const items: CampaignSendItem[] = report?.items ?? [];
  const shown = filter === "all" ? items : items.filter((it) => it.status === filter);

  const reasonMap = (bucket: "blocked" | "canceled" | "escalated"): Record<string, number> =>
    (summary?.[bucket] as Record<string, number> | undefined) ?? {};

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Campaign Result</span>
            <span className={styles.title}>{report ? report.title : "—"}</span>
          </div>
          {report && <StatusBadge tone="scheduled">{report.status}</StatusBadge>}
        </div>

        <div className={styles.metrics}>
          {BUCKETS.map((b) => (
            <div key={b} className={styles.metric} data-testid={`metric-${b}`}>
              <span className={styles.metricLabel}>{bucketLabel(b)}</span>
              <span className={styles.metricValue}>{summary ? bucketCount(summary as unknown as Record<string, unknown>, b) : "—"}</span>
              {(b === "blocked" || b === "canceled" || b === "escalated") && summary && (
                <div className={styles.reasons}>
                  {Object.entries(reasonMap(b)).map(([reason, n]) => (
                    <span key={reason} className={styles.reasonItem}>{reason} ×{n}</span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Per-Contact</span>
            <span className={styles.title}>推送明细</span>
          </div>
        </div>

        <div className={styles.filters} role="tablist">
          <button
            type="button"
            className={`${styles.chip} ${filter === "all" ? styles.chipActive : ""}`}
            onClick={() => setFilter("all")}
          >
            全部 ({items.length})
          </button>
          {BUCKETS.map((b) => (
            <button
              key={b}
              type="button"
              className={`${styles.chip} ${filter === b ? styles.chipActive : ""}`}
              onClick={() => setFilter(b)}
            >
              {bucketLabel(b)}
            </button>
          ))}
        </div>

        {shown.length === 0 ? (
          <EmptyState title="暂无推送明细" hint="该筛选下没有客户记录。" />
        ) : (
          <table className={styles.table}>
            <thead>
              <tr className={styles.tr}>
                <th className={`${styles.th} ${styles.thName}`}>客户</th>
                <th className={styles.th}>wxid</th>
                <th className={styles.th}>状态</th>
                <th className={styles.th}>原因</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((it) => (
                <tr key={it.contactWxid} className={styles.tr} data-testid="detail-row">
                  <td className={`${styles.td} ${styles.tdName}`}>{it.name || "—"}</td>
                  <td className={`${styles.td} ${styles.tdWxid}`}>{it.contactWxid}</td>
                  <td className={styles.td}><StatusBadge tone={bucketTone(it.status)}>{bucketLabel(it.status)}</StatusBadge></td>
                  <td className={styles.td}>{it.reason || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
