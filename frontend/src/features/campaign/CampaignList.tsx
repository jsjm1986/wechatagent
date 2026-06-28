import { useEffect } from "react";
import { Megaphone, Plus } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CampaignListItem } from "../../stores/campaignStore";
import styles from "./Campaign.module.css";

export function campaignStatusTone(status: string): StatusTone {
  switch (status) {
    case "dispatching":
    case "completed": return "running";
    case "previewed":
    case "confirmed": return "scheduled";
    case "canceled": return "blocked";
    default: return "inactive"; // draft / 未知
  }
}

export function campaignStatusLabel(status: string): string {
  switch (status) {
    case "draft": return "草稿";
    case "previewed": return "已预览";
    case "confirmed": return "已确认";
    case "dispatching": return "推送中";
    case "completed": return "已完成";
    case "canceled": return "已取消";
    default: return status;
  }
}

export default function CampaignList() {
  const campaigns = useCampaignStore((s) => s.campaigns);
  const listLoaded = useCampaignStore((s) => s.listLoaded);
  const loadCampaigns = useCampaignStore((s) => s.loadCampaigns);
  const setView = useCampaignStore((s) => s.setView);
  const openReport = useCampaignStore((s) => s.openReport);

  useEffect(() => {
    if (!listLoaded) void loadCampaigns();
  }, [listLoaded, loadCampaigns]);

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Campaigns</span>
            <span className={styles.title}>活动列表</span>
          </div>
          <button type="button" className={styles.primaryBtn} onClick={() => setView("create")}>
            <Plus size={14} /> 新建活动
          </button>
        </div>

        {campaigns.length === 0 ? (
          <EmptyState icon={<Megaphone size={28} />} title="还没有活动" hint="点「新建活动」按条件圈人并预览，确认推送在 AI 总控对话中完成。" />
        ) : (
          <table className={styles.table}>
            <thead>
              <tr className={styles.tr}>
                <th className={`${styles.th} ${styles.thName}`}>活动标题</th>
                <th className={styles.th}>状态</th>
                <th className={styles.th} title="已扇出的跟进任务数，非真实送达数">已扇出</th>
                <th className={styles.th} title="圈人命中数，真实送达见结果看板">命中数</th>
                <th className={styles.th}>创建人</th>
                <th className={styles.th}>创建时间</th>
              </tr>
            </thead>
            <tbody>
              {campaigns.map((c: CampaignListItem) => (
                <tr key={c.campaignId} className={`${styles.tr} ${styles.rowClickable}`} data-testid="campaign-row" onClick={() => openReport(c.campaignId)}>
                  <td className={`${styles.td} ${styles.tdName}`}>{c.title}</td>
                  <td className={styles.td}><StatusBadge tone={campaignStatusTone(c.status)}>{campaignStatusLabel(c.status)}</StatusBadge></td>
                  <td className={styles.td}>{c.dispatchedCount}</td>
                  <td className={styles.td}>{c.targetCount ?? "—"}</td>
                  <td className={styles.td}>{c.createdBy}</td>
                  <td className={styles.td}>{c.createdAt ? new Date(c.createdAt).toLocaleString() : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
