import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import styles from "./OutboxPanel.module.css";

// 发件箱（agent_send_outbox）逐条只读 + 取消入口。
// outbox 是 approved 决策发送链路的真相源（CLAUDE.md 硬规则）：之前前端只有
// /outcomes/autonomy 的聚合比率，无法逐条排障或取消卡住的待发条目。
// 后端：GET /api/admin/outbox（带 /admin 前缀）+ POST /api/admin/outbox/:id/cancel。
// cancel 端点 serde 强制非空 cancelReason（admin_outbox.rs:124），故取消请求必带 body。
// autonomy feature 无独立 store，组件内 useState + api，自取 accountId。

type OutboxItem = {
  id: string;
  status: string;
  content: string;
  contactWxid: string | null;
  createdAt: string | null;
};

// 仅 pending / in_flight 可取消（与后端 outbox_status_is_user_cancelable 一致）；
// 其它状态后端返回 409，前端直接隐藏取消按钮避免误点。
const CANCELABLE_STATUSES = new Set(["pending", "in_flight"]);

// 发件箱状态(src/agent/outbox.rs OutboxStatus 闭集;未知值回落原值)。
const OUTBOX_STATUS_LABELS: Record<string, string> = {
  pending: "待发送",
  in_flight: "发送中",
  sent: "已送达",
  failed_terminal: "发送失败",
  canceled: "已取消",
};
function outboxStatusLabel(status: string): string {
  return OUTBOX_STATUS_LABELS[status] ?? status;
}

export function OutboxPanel() {
  const accountId = useAccountStore((s) => s.currentAccountId());
  const [items, setItems] = useState<OutboxItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    setErr("");
    try {
      const qs = accountId ? `?accountId=${encodeURIComponent(accountId)}` : "";
      const data = await api.get<{ items: OutboxItem[] }>(
        `/api/admin/outbox${qs}`
      );
      setItems(data.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [accountId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function cancel(id: string) {
    setErr("");
    try {
      await api.post(`/api/admin/outbox/${id}/cancel`, {
        cancelReason: "admin_outbox_panel_cancel",
      });
      await load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className={styles.panel}>
      <div className={styles.head}>
        <h3 className={styles.title}>发件箱</h3>
        <button
          type="button"
          className={styles.refresh}
          onClick={() => void load()}
          disabled={loading}
        >
          {loading ? "加载中" : "刷新"}
        </button>
      </div>
      {err && <div className={styles.error}>{err}</div>}
      {!accountId && (
        <p className={styles.hint}>请先在顶部选择一个微信账号。</p>
      )}
      {accountId && items.length === 0 && !loading && (
        <p className={styles.hint}>该账号当前没有发件箱条目。</p>
      )}
      {items.length > 0 && (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>状态</th>
              <th>联系人</th>
              <th>内容</th>
              <th>入队时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {items.map((it) => (
              <tr key={it.id}>
                <td>{outboxStatusLabel(it.status)}</td>
                <td>{it.contactWxid || "—"}</td>
                <td className={styles.contentCell}>{it.content}</td>
                <td>{it.createdAt || "—"}</td>
                <td>
                  {CANCELABLE_STATUSES.has(it.status) ? (
                    <button
                      type="button"
                      className={styles.linkBtn}
                      onClick={() => void cancel(it.id)}
                    >
                      取消
                    </button>
                  ) : (
                    <span className={styles.muted}>—</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
