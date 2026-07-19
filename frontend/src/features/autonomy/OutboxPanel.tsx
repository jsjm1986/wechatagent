import { useCallback, useEffect, useState } from "react";
import { ConfirmProvider, useConfirm } from "../../components/ui/ConfirmDialog";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import styles from "./OutboxPanel.module.css";

// 发件箱（agent_send_outbox）逐条只读 + 取消入口。
// outbox 是 approved 决策发送链路的真相源（CLAUDE.md 硬规则）：之前前端只有
// /outcomes/autonomy 的聚合比率，无法逐条排障或取消卡住的待发条目。
// 后端：GET /api/admin/outbox（带 /admin 前缀）+ POST /api/admin/outbox/:id/cancel。
// cancel 端点 serde 强制非空 cancelReason（admin_outbox.rs:124），故取消请求必带 body。
// autonomy feature 无独立 store，组件内 useState + api，自取 accountId。

type OutboxPayload =
  | { kind: "text"; text: string }
  | { kind: "media"; assetId: string; title: string | null; fileName: string | null }
  | { kind: "referralCard"; cardId: string; displayName: string | null; targetWxid: string | null }
  | { kind: "invalid"; mediaAssetId: string; referralCardId: string; reason: string };

type OutboxItem = {
  id: string;
  accountId: string;
  status: string;
  content: string;
  payload: OutboxPayload;
  contactWxid: string | null;
  createdAt: string | null;
  cancelRequested: boolean;
  cancelRequestedAt: string | null;
  sendStartedAt: string | null;
  reclaimedInFlight: boolean;
  reclaimCount: number;
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
  delivery_unknown: "送达待核验",
};
function outboxStatusLabel(item: OutboxItem): string {
  if (item.status === "in_flight" && item.cancelRequested) {
    return "取消请求中（等待发送结果）";
  }
  return OUTBOX_STATUS_LABELS[item.status] ?? item.status;
}

function payloadLabel(payload: OutboxPayload): string {
  switch (payload.kind) {
    case "text":
      return payload.text || "（空文本）";
    case "media":
      return `素材 · ${payload.title || payload.fileName || payload.assetId}`;
    case "referralCard":
      return `顾问名片 · ${payload.displayName || payload.targetWxid || payload.cardId}`;
    case "invalid":
      return "异常：同时绑定素材与名片";
  }
}

function payloadIdentity(payload: OutboxPayload): string | null {
  switch (payload.kind) {
    case "text":
      return null;
    case "media":
      return `素材 ID：${payload.assetId}`;
    case "referralCard":
      return `名片 ID：${payload.cardId}${payload.targetWxid ? ` · ${payload.targetWxid}` : ""}`;
    case "invalid":
      return `素材 ID：${payload.mediaAssetId} · 名片 ID：${payload.referralCardId}`;
  }
}

function cancellationRisk(item: OutboxItem): string {
  if (item.sendStartedAt) {
    return "该条目已越过最后可取消点；本次操作只会登记取消请求，最终状态仍以真实发送回执为准。";
  }
  if (item.reclaimedInFlight || item.reclaimCount > 0) {
    return `该条目曾从发送中恢复 ${Math.max(item.reclaimCount, 1)} 次，远端可能已经收到；取消不能撤回已送达内容，请先核对目标。`;
  }
  if (item.status === "in_flight") {
    return "该条目正在发送；确认后将登记取消请求。若请求先于最后发送边界落库则停止，否则最终状态以真实发送回执为准。";
  }
  return "该条目尚未越过远端发送边界，确认后将停止本次发送。";
}

function OutboxPanelInner() {
  const accountId = useAccountStore((s) => s.currentAccountId());
  const confirm = useConfirm();
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

  async function cancel(item: OutboxItem) {
    const identity = payloadIdentity(item.payload);
    const ok = await confirm({
      title: "确认取消这条发送？",
      body: (
        <div className={styles.confirmBody}>
          <div>业务号：{item.accountId || accountId || "—"}</div>
          <div>客户：{item.contactWxid || "—"}</div>
          <div>发送对象：{payloadLabel(item.payload)}</div>
          {identity && <div>{identity}</div>}
          <div className={styles.risk}>{cancellationRisk(item)}</div>
        </div>
      ),
      tone: "danger",
      confirmText: item.status === "in_flight" ? "请求取消" : "确认取消",
    });
    if (!ok) return;
    setErr("");
    try {
      await api.post(`/api/admin/outbox/${item.id}/cancel`, {
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
                <td>{outboxStatusLabel(it)}</td>
                <td>{it.contactWxid || "—"}</td>
                <td className={styles.contentCell}>
                  <strong className={styles.payloadLabel}>{payloadLabel(it.payload)}</strong>
                  {payloadIdentity(it.payload) && (
                    <small className={styles.payloadIdentity}>{payloadIdentity(it.payload)}</small>
                  )}
                  {(it.reclaimedInFlight || it.reclaimCount > 0) && (
                    <small className={styles.recoveryWarning}>
                      曾恢复发送 {Math.max(it.reclaimCount, 1)} 次
                    </small>
                  )}
                </td>
                <td>{it.createdAt || "—"}</td>
                <td>
                  {CANCELABLE_STATUSES.has(it.status) && !it.cancelRequested ? (
                    <button
                      type="button"
                      className={styles.linkBtn}
                      onClick={() => void cancel(it)}
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

export function OutboxPanel() {
  return (
    <ConfirmProvider>
      <OutboxPanelInner />
    </ConfirmProvider>
  );
}
