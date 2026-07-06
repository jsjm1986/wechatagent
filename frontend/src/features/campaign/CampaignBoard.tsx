import { useEffect, useState } from "react";
import { Megaphone, Download } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge } from "../../components/ui/StatusBadge";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CampaignSendItem } from "../../stores/campaignStore";
import { bucketTone, bucketLabel, bucketCount } from "./buckets";
import { campaignStatusLabel } from "./CampaignList";
import { toCsv } from "./csv";
import { SEND_OUTCOME_REASON_LABELS, labelOf } from "../../lib/reviewLabels";
import styles from "./Campaign.module.css";

const BUCKETS = ["sent", "pending", "blocked", "escalated", "canceled", "skipped", "unknown"] as const;
const PAGE_SIZE = 50;

function downloadCsv(filename: string, csv: string) {
  const blob = new Blob(["﻿" + csv], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export default function CampaignBoard() {
  const { selectedCampaignId, report, loadReport } = useCampaignStore();
  const [filter, setFilter] = useState<string>("all");
  const loading = useCampaignStore((s) => s.loading);
  const lastAttemptedId = useCampaignStore((s) => s.lastAttemptedId);
  const page = useCampaignStore((s) => s.page);
  const setPage = useCampaignStore((s) => s.setPage);

  useEffect(() => {
    if (selectedCampaignId && !report && !loading && selectedCampaignId !== lastAttemptedId) {
      void loadReport(selectedCampaignId);
    }
  }, [selectedCampaignId, report, loading, lastAttemptedId, loadReport]);

  if (!selectedCampaignId) {
    return (
      <div className={styles.page}>
        <EmptyState
          icon={<Megaphone size={28} />}
          title="暂无活动结果"
          hint="在 AI 总控下发活动推送后，点「查看推送结果」进入这里查看真实触达分布。"
        />
      </div>
    );
  }

  const summary = report?.summary;
  const items: CampaignSendItem[] = report?.items ?? [];
  const shown = filter === "all" ? items : items.filter((it) => it.status === filter);
  const pageCount = Math.max(1, Math.ceil(shown.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const pageRows = shown.slice(safePage * PAGE_SIZE, safePage * PAGE_SIZE + PAGE_SIZE);

  const reasonMap = (bucket: "blocked" | "canceled" | "escalated"): Record<string, number> =>
    (summary?.[bucket] as Record<string, number> | undefined) ?? {};

  const pickFilter = (b: string) => { setFilter(b); setPage(0); };

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Campaign Result</span>
            <span className={styles.title}>{report ? report.title : "—"}</span>
          </div>
          {report && <StatusBadge tone="scheduled">{campaignStatusLabel(report.status)}</StatusBadge>}
        </div>
        <div className={styles.metrics}>
          {BUCKETS.map((b) => (
            <div key={b} className={styles.metric} data-testid={`metric-${b}`}>
              <span className={styles.metricLabel}>{bucketLabel(b)}</span>
              <span className={styles.metricValue}>{summary ? bucketCount(summary as unknown as Record<string, unknown>, b) : "—"}</span>
              {(b === "blocked" || b === "canceled" || b === "escalated") && summary && (
                <div className={styles.reasons}>
                  {Object.entries(reasonMap(b)).map(([reason, n]) => (
                    <span key={reason} className={styles.reasonItem}>{labelOf(SEND_OUTCOME_REASON_LABELS, reason)} ×{n}</span>
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
            <span className={styles.eyebrow}>逐人明细</span>
            <span className={styles.title}>推送明细</span>
          </div>
          <button
            type="button"
            className={styles.exportBtn}
            disabled={items.length === 0}
            onClick={() => downloadCsv(`campaign-${selectedCampaignId}-sends.csv`, toCsv(items))}
          >
            <Download size={14} /> 导出 CSV
          </button>
        </div>

        <div className={styles.filters}>
          <button type="button" className={`${styles.chip} ${filter === "all" ? styles.chipActive : ""}`} onClick={() => pickFilter("all")}>
            全部 ({items.length})
          </button>
          {BUCKETS.map((b) => (
            <button key={b} type="button" className={`${styles.chip} ${filter === b ? styles.chipActive : ""}`} onClick={() => pickFilter(b)}>
              {bucketLabel(b)}
            </button>
          ))}
        </div>

        {shown.length === 0 ? (
          <EmptyState title="暂无推送明细" hint="该筛选下没有客户记录。" />
        ) : (
          <>
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
                {pageRows.map((it) => (
                  <tr key={it.contactWxid} className={styles.tr} data-testid="detail-row">
                    <td className={`${styles.td} ${styles.tdName}`}>{it.name || "—"}</td>
                    <td className={`${styles.td} ${styles.tdWxid}`}>{it.contactWxid}</td>
                    <td className={styles.td}><StatusBadge tone={bucketTone(it.status)}>{bucketLabel(it.status)}</StatusBadge></td>
                    <td className={styles.td}>{it.reason ? labelOf(SEND_OUTCOME_REASON_LABELS, it.reason) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {pageCount > 1 && (
              <div className={styles.pager}>
                <button type="button" className={styles.pagerBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
                <span className={styles.pagerInfo}>{safePage + 1} / {pageCount}</span>
                <button type="button" className={styles.pagerBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
              </div>
            )}
          </>
        )}
      </section>
    </div>
  );
}
