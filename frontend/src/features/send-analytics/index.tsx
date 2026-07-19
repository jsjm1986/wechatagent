import { useEffect, useState } from "react";
import { BarChart3, Image, Contact } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { useSendAnalyticsStore } from "../../stores/sendAnalyticsStore";
import { useAccountStore } from "../../stores/accountStore";
import type { SendStatRow } from "../../stores/sendAnalyticsStore";
import styles from "./SendAnalytics.module.css";

type StatTab = "media" | "namecard";

const pct = (rate: number) => `${(rate * 100).toFixed(1)}%`;

export default function SendAnalyticsFeature() {
  const { overview, mediaStats, namecardStats, loadOverview, loadStats } = useSendAnalyticsStore();
  const accountId = useAccountStore((s) => s.currentAccountId());
  const [tab, setTab] = useState<StatTab>("media");

  useEffect(() => {
    void loadOverview(accountId);
  }, [accountId, loadOverview]);

  useEffect(() => {
    void loadStats(tab, accountId);
  }, [accountId, tab, loadStats]);

  const rows = tab === "media" ? mediaStats : namecardStats;

  return (
    <div className={styles.page}>
      <p className={styles.intro}>
        统计 AI 在私聊运营中主动发送的素材与专属顾问名片的成效：发送次数、覆盖客户数、客户响应率与阶段推进率。
        响应率指收到客户后续回复的占比，阶段推进率指发送后客户阶段向前推进的占比。
      </p>

      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Send Overview</span>
            <span className={styles.title}>总览</span>
          </div>
          <span className={styles.headIcon}><BarChart3 size={17} /></span>
        </div>

        <div className={styles.metrics}>
          <div className={styles.metric}>
            <span className={styles.metricLabel}>总发送数</span>
            <span className={styles.metricValue}>{overview ? overview.totalSends.toLocaleString() : "—"}</span>
          </div>
          <div className={styles.metric}>
            <span className={styles.metricLabel}>响应率</span>
            <span className={styles.metricValue}>{overview ? pct(overview.responseRate) : "—"}</span>
          </div>
          <div className={styles.metric}>
            <span className={styles.metricLabel}>阶段推进率</span>
            <span className={styles.metricValue}>{overview ? pct(overview.stageAdvanceRate) : "—"}</span>
          </div>
        </div>
      </section>

      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Send Effectiveness</span>
            <span className={styles.title}>{tab === "media" ? "素材效果排行" : "名片效果排行"}</span>
          </div>
          <span className={styles.headIcon}>
            {tab === "media" ? <Image size={17} /> : <Contact size={17} />}
          </span>
        </div>

        <div className={styles.tabs} role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "media"}
            className={`${styles.tab} ${tab === "media" ? styles.tabActive : ""}`}
            onClick={() => setTab("media")}
          >
            素材效果
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "namecard"}
            className={`${styles.tab} ${tab === "namecard" ? styles.tabActive : ""}`}
            onClick={() => setTab("namecard")}
          >
            名片效果
          </button>
        </div>

        {rows.length === 0 ? (
          <EmptyState
            title={tab === "media" ? "暂无素材发送数据" : "暂无名片引荐数据"}
            hint="AI 在私聊运营中主动发送后，这里会按发送次数排序展示各项成效。"
          />
        ) : (
          <table className={styles.table}>
            <thead>
              <tr className={styles.tr}>
                <th className={`${styles.th} ${styles.thName}`}>名称</th>
                <th className={`${styles.th} ${styles.thNum}`}>已发次数</th>
                <th className={`${styles.th} ${styles.thNum}`}>覆盖客户数</th>
                <th className={`${styles.th} ${styles.thNum}`}>响应率</th>
                <th className={`${styles.th} ${styles.thNum}`}>阶段推进率</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row: SendStatRow) => (
                <tr key={row.targetId} className={styles.tr}>
                  <td className={`${styles.td} ${styles.tdName}`}>{row.targetTitle}</td>
                  <td className={`${styles.td} ${styles.tdNum}`}>{row.sentCount.toLocaleString()}</td>
                  <td className={`${styles.td} ${styles.tdNum}`}>{row.contactCount.toLocaleString()}</td>
                  <td className={`${styles.td} ${styles.tdNum}`}>{pct(row.responseRate)}</td>
                  <td className={`${styles.td} ${styles.tdNum}`}>{pct(row.stageAdvanceRate)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
