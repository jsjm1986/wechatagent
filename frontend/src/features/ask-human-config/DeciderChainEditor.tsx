import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../lib/api";
import { FriendPickerModal, type FriendPickerItem } from "../../components/ui/FriendPickerModal";
import { useToast } from "../../components/ui/Toast";
import { useAccountStore } from "../../stores/accountStore";
import { useUserOpsStore } from "../../stores/userOpsStore";
import type { DeciderRef, RosterEntry } from "../../types";
import { isPickableDecider } from "./deciderCandidates";
import styles from "./AskHumanConfig.module.css";

function rosterLabel(entry: RosterEntry): string {
  return entry.remark || entry.nickname || entry.wxid;
}

export function DeciderChainEditor({
  chain,
  onChange,
}: {
  chain: DeciderRef[];
  onChange: (next: DeciderRef[]) => void;
}) {
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [loading, setLoading] = useState(false);

  const toast = useToast();
  const accountId = useAccountStore((s) => s.currentAccountId());
  const loadRoster = useUserOpsStore((s) => s.loadRoster);
  const rosterCache = useUserOpsStore((s) => s.rosterCache);
  const roster = rosterCache[accountId]?.items ?? [];

  // 请求序号守卫：快速切账号时并发多次 loadRoster，只有最新一次允许落地，
  // 否则先发的（账号 A）若晚于后发的（账号 B）返回，会用 A 的好友覆盖 B 的列表。
  // 抄 RosterView.tsx 的 reqSeqRef 做法。
  const reqSeqRef = useRef(0);

  const refresh = useCallback(
    async (id: string) => {
      if (!id) return;
      const seq = ++reqSeqRef.current;
      const isStale = () => seq !== reqSeqRef.current;
      setLoading(true);
      setError(null);
      try {
        const { syncing: isSyncing } = await loadRoster(id);
        if (isStale()) return;
        setSyncing(isSyncing);
      } catch (e) {
        if (isStale()) return;
        setError(e instanceof Error ? e.message : "加载通讯录失败");
      } finally {
        if (!isStale()) setLoading(false);
      }
    },
    [loadRoster],
  );

  // 仅在打开选择器时拉取，避免页面加载就打通讯录接口。
  useEffect(() => {
    if (!picking) return;
    void refresh(accountId);
  }, [picking, accountId, refresh]);

  // 快照同步中时每 10s 自动重拉（不带 force，只读快照）；后台单飞任务写好快照后
  // 普通读自然读到、syncing 变 false、轮询自停。抄 RosterView.tsx 的同款 effect。
  useEffect(() => {
    if (!picking || !syncing || !accountId) return;
    const timer = setInterval(() => {
      void refresh(accountId);
    }, 10000);
    return () => clearInterval(timer);
  }, [picking, syncing, accountId, refresh]);

  const inChain = useMemo(() => new Set(chain.map((d) => d.wxid)), [chain]);

  // 双重过滤：isPickableDecider 与后端 import 守卫等价（见 deciderCandidates.ts），
  // 再排除已在链中的。未入库的保留在候选里并打 badge——选中时自动导入。
  const candidates: FriendPickerItem[] = useMemo(
    () =>
      roster
        .filter((entry) => isPickableDecider(entry))
        .filter((entry) => !inChain.has(entry.wxid))
        .map((entry) => ({
          wxid: entry.wxid,
          nickname: entry.nickname,
          remark: entry.remark,
          avatarUrl: entry.avatarUrl,
          sex: entry.sex,
          ...(entry.agentStatus === "not_imported" ? { badge: "未导入" } : {}),
        })),
    [roster, inChain],
  );

  async function pick(item: FriendPickerItem) {
    const entry = roster.find((r) => r.wxid === item.wxid);
    const displayName = entry ? rosterLabel(entry) : item.wxid;
    if (!accountId) {
      // 失败提示一律走 toast：弹窗此刻仍开着，Overlay 的 scrim 是全屏半透明层
      // （z-index --z-overlay:1000），内联错误会被它盖住 → 用户看到「点了没反应」。
      // toast portal 到 body 且 z-index --z-toast:1100 > scrim，能浮在遮罩之上。
      toast.error("未选择账号，无法添加决策人");
      return;
    }

    // 后端 put_ask_human_policy fail-closed 要求决策人已在 contacts 表
    // （src/routes/domains.rs 的 contact_exists 校验），故未入库的好友必须先落库再入链。
    // 用 /api/contacts/import（upsert 的 $setOnInsert 写 agent_status: "normal"，不托管），
    // 不用 /contacts/batch-enable——后者无条件写 "managed" 并建 enrollment intent，
    // 会把内部决策者当客户交给 AI 运营，语义不对。
    if (entry?.agentStatus === "not_imported") {
      setImporting(true);
      try {
        const res = await api.post<{ items: unknown[] }>("/api/contacts/import", {
          accountId,
          candidates: [
            {
              wxid: entry.wxid,
              ...(entry.nickname ? { nickname: entry.nickname } : {}),
              ...(entry.remark ? { remark: entry.remark } : {}),
            },
          ],
        });
        // 坑：接口回 200 不代表导入成功——upsert 返回 None 时 handler 静默跳过
        // （src/routes/contacts.rs import_contacts_endpoint 的 `if let Some(contact)`），
        // items 为空。只看是否 throw 会把静默失败当成功，随后保存时才被后端拒绝。
        if (!Array.isArray(res.items) || res.items.length === 0) {
          toast.error(
            `「${displayName}」未能导入通讯录（可能被识别为非真人账号），请换一位或先到「账号管理」同步通讯录`,
          );
          return;
        }
        // 导入成功后不强制重拉 roster（设计 §4.4）：agentStatus 变化不影响已选中项的
        // 正确性，force 会触发后端 spawn_roster_refresh 全量重拉、打断连续添加多人。
        // 「未导入」badge 在下次自然刷新时消失即可。
      } catch (e) {
        toast.error(e instanceof Error ? e.message : "导入通讯录失败");
        return;
      } finally {
        setImporting(false);
      }
    }

    onChange([...chain, { wxid: item.wxid, displayName, accountId }]);
    setPicking(false);
  }

  function remove(idx: number) {
    onChange(chain.filter((_, i) => i !== idx));
  }
  function move(idx: number, dir: -1 | 1) {
    const j = idx + dir;
    if (j < 0 || j >= chain.length) return;
    const next = [...chain];
    [next[idx], next[j]] = [next[j], next[idx]];
    onChange(next);
  }

  return (
    <div className={styles.chainEditor}>
      {chain.length === 0 && <div className={styles.chainEmpty}>尚未配置决策人</div>}
      {chain.map((d, idx) => (
        <div key={d.wxid} className={styles.chainRow}>
          <span className={styles.chainName} title={d.wxid}>
            {d.displayName ?? d.wxid}
            <span className={styles.chainWxid}>{d.accountId ? `账号 ${d.accountId}` : "未绑定账号"}</span>
          </span>
          <div className={styles.chainActions}>
            <button type="button" aria-label="上移" disabled={idx === 0} onClick={() => move(idx, -1)}>↑</button>
            <button type="button" aria-label="下移" disabled={idx === chain.length - 1} onClick={() => move(idx, 1)}>↓</button>
            <button type="button" aria-label="删除" onClick={() => remove(idx)}>✕</button>
          </div>
        </div>
      ))}
      <div className={styles.chainHint}>超时未响应时，按此顺序转交链中下一位</div>

      <button
        type="button"
        className={styles.linkBtn}
        disabled={importing}
        onClick={() => setPicking(true)}
      >
        {importing ? "导入中…" : "+ 从通讯录添加"}
      </button>

      <FriendPickerModal
        open={picking}
        items={candidates}
        onSelect={(item) => void pick(item)}
        onClose={() => setPicking(false)}
        title="选择决策人"
        loading={loading && roster.length === 0 && !syncing}
        error={error}
        emptyText={
          syncing
            ? "通讯录同步中，稍等几秒会自动出现…"
            : "该账号通讯录为空。请先到「账号管理」同步通讯录。"
        }
      />
    </div>
  );
}
