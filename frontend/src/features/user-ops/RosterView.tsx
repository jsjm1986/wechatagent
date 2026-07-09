import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw, Check, Users } from "lucide-react";
import { useAccountStore } from "../../stores/accountStore";
import { useUserOpsStore } from "../../stores/userOpsStore";
import { useToast } from "../../components/ui/Toast";
import type { RosterEntry } from "../../types";
import styles from "./RosterView.module.css";

// 本地 6 行泛型分页 hook（卡片网格每页 60）——避免改动正在工作的 system-strategy 文件。
const ROSTER_PAGE_SIZE = 60;
function usePagedList<T>(items: T[]) {
  const [page, setPage] = useState(0);
  const pageCount = Math.max(1, Math.ceil(items.length / ROSTER_PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const pageRows = items.slice(safePage * ROSTER_PAGE_SIZE, safePage * ROSTER_PAGE_SIZE + ROSTER_PAGE_SIZE);
  return { pageRows, pageCount, safePage, setPage };
}

const sexLabel = (sex?: number | null): string | null => {
  if (sex === 1) return "男";
  if (sex === 2) return "女";
  if (sex === 0) return "未知";
  return null; // 缺失（旧形态/无数据）不展示
};

// 通讯录视图：拉指定账号的全量微信好友（含头像），勾选后批量进入 Agent 运营。
// 纯浏览不写库；仅点「加入 Agent 运营」时提交 batch-enable。
export function RosterView() {
  const accounts = useAccountStore((s) => s.accounts);
  const selectAccount = useAccountStore((s) => s.selectAccount);
  const effectiveAccountId = useAccountStore((s) =>
    s.accounts.some((a) => a.accountId === s.selectedAccountId)
      ? s.selectedAccountId
      : s.accounts[0]?.accountId ?? ""
  );
  const loadRoster = useUserOpsStore((s) => s.loadRoster);
  const batchEnable = useUserOpsStore((s) => s.batchEnable);
  const playbooks = useUserOpsStore((s) => s.playbooks);
  const toast = useToast();

  const [roster, setRoster] = useState<RosterEntry[]>([]);
  const [syncing, setSyncing] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [selectedWxids, setSelectedWxids] = useState<Set<string>>(new Set());
  const [sharedNote, setSharedNote] = useState("");
  const [playbookId, setPlaybookId] = useState("");
  const [submitting, setSubmitting] = useState(false);
  // 请求序号守卫：快速切账号时并发多个 loadRoster，只有最新一次的响应才允许落地，
  // 丢弃过时响应——否则先发的（账号 A）若晚于后发的（账号 B）返回，会用 A 的好友覆盖
  // B 的列表，而选中账号已是 B，导致列表与账号错配、勾选提交用 B 的 accountId 配 A 的 wxid。
  const reqSeqRef = useRef(0);

  const refresh = useCallback(
    async (accountId: string) => {
      if (!accountId) return;
      const seq = ++reqSeqRef.current;
      const isStale = () => seq !== reqSeqRef.current;
      setLoading(true);
      setError(null);
      // 切账号/刷新时连同本批草稿一起清空——否则账号 A 的运营备注/剧本会残留到账号 B。
      setSelectedWxids(new Set());
      setSharedNote("");
      setPlaybookId("");
      try {
        const { items, syncing: isSyncing } = await loadRoster(accountId);
        if (isStale()) return; // 已有更新的请求发出，丢弃本次过时结果。
        setRoster(items);
        setSyncing(isSyncing);
      } catch (e) {
        if (isStale()) return;
        setError(e instanceof Error ? e.message : "加载好友列表失败");
      } finally {
        // 仅最新请求负责收起 loading——过时请求提前收起会让进行中的最新请求看起来已完成。
        if (!isStale()) setLoading(false);
      }
    },
    [loadRoster]
  );

  useEffect(() => {
    void refresh(effectiveAccountId);
  }, [effectiveAccountId, refresh]);

  // cache 同步中时每 8s 自动重拉，直到就绪（syncing 变 false）或账号切换。
  useEffect(() => {
    if (!syncing || !effectiveAccountId) return;
    const timer = setInterval(() => {
      void refresh(effectiveAccountId);
    }, 8000);
    return () => clearInterval(timer);
  }, [syncing, effectiveAccountId, refresh]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return roster;
    return roster.filter((r) =>
      [r.remark, r.nickname, r.wxid].some((v) => v?.toLowerCase().includes(q))
    );
  }, [roster, filter]);

  const { pageRows, pageCount, safePage, setPage } = usePagedList(filtered);

  const toggle = (entry: RosterEntry) => {
    if (entry.agentStatus === "managed") return; // 已托管不可重复勾选
    setSelectedWxids((prev) => {
      const next = new Set(prev);
      if (next.has(entry.wxid)) next.delete(entry.wxid);
      else next.add(entry.wxid);
      return next;
    });
  };

  const onSubmit = async () => {
    if (!selectedWxids.size || !sharedNote.trim()) return;
    setSubmitting(true);
    setError(null);
    try {
      const candidates = roster
        .filter((r) => selectedWxids.has(r.wxid))
        .map((r) => ({
          wxid: r.wxid,
          nickname: r.nickname,
          remark: r.remark,
          avatarUrl: r.avatarUrl,
          sex: r.sex,
        }));
      const res = await batchEnable({
        accountId: effectiveAccountId,
        candidates,
        sharedNote: sharedNote.trim(),
        playbookId: playbookId || undefined,
      });
      toast.success(`已加入 ${res.enabled} 人，画像后台生成中`);
      setSharedNote("");
      setPlaybookId("");
      await refresh(effectiveAccountId);
    } catch (e) {
      setError(e instanceof Error ? e.message : "批量加入运营失败");
    } finally {
      setSubmitting(false);
    }
  };

  const statusBadge = (status: RosterEntry["agentStatus"]) => {
    if (status === "managed") return <span className={styles.badgeManaged}>已托管</span>;
    if (status === "normal") return <span className={styles.badgeNormal}>已导入</span>;
    return <span className={styles.badgeNew}>未导入</span>;
  };

  const initial = (entry: RosterEntry) =>
    (entry.remark || entry.nickname || entry.wxid).trim().charAt(0).toUpperCase();

  return (
    <section className={styles.page}>
      <div className={styles.header}>
        <div className={styles.headerText}>
          <span className={styles.eyebrow}>通讯录</span>
          <h2 className={styles.title}>微信好友总览</h2>
          <p className={styles.subtitle}>选择账号拉取全部好友，勾选后批量进入 Agent 运营。</p>
        </div>
        <div className={styles.headerActions}>
          {accounts.length > 1 && (
            <select
              className={styles.accountSelect}
              value={effectiveAccountId}
              onChange={(e) => selectAccount(e.target.value)}
            >
              {accounts.map((a) => (
                <option key={a.accountId} value={a.accountId}>
                  {a.alias || a.displayName || a.accountId}
                </option>
              ))}
            </select>
          )}
          <button
            type="button"
            className={styles.ghostBtn}
            onClick={() => void refresh(effectiveAccountId)}
            disabled={loading || !effectiveAccountId}
          >
            <RefreshCw size={14} className={loading ? styles.spin : ""} />
            {loading ? "加载中…" : "刷新"}
          </button>
        </div>
      </div>

      {error && <div className={styles.err}>{error}</div>}

      <label className={styles.filter}>
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="按备注 / 昵称 / 微信号过滤"
        />
      </label>

      {loading && !syncing ? (
        <div className={styles.loading}>加载中…</div>
      ) : filtered.length === 0 ? (
        <div className={styles.empty}>
          <Users size={22} />
          {syncing ? (
            <>
              <strong>正在从微信同步好友…</strong>
              <p>GeWe 正在准备该账号的好友列表，稍候会自动刷新。也可点「刷新」重试。</p>
            </>
          ) : (
            <>
              <strong>暂无好友</strong>
              <p>该账号还没有拉取到好友，或过滤条件无匹配。点「刷新」重新从 MCP 拉取。</p>
            </>
          )}
        </div>
      ) : (
        <>
          <div className={styles.grid}>
            {pageRows.map((entry) => {
              const checked = selectedWxids.has(entry.wxid);
              const managed = entry.agentStatus === "managed";
              return (
                <button
                  key={entry.wxid}
                  type="button"
                  className={`${styles.card} ${checked ? styles.cardChecked : ""} ${managed ? styles.cardManaged : ""}`}
                  onClick={() => toggle(entry)}
                  disabled={managed}
                >
                  <div className={styles.checkbox}>{checked && <Check size={13} />}</div>
                  {entry.avatarUrl ? (
                    <img className={styles.avatar} src={entry.avatarUrl} alt="" loading="lazy" />
                  ) : (
                    <span className={styles.avatarFallback}>{initial(entry)}</span>
                  )}
                  <div className={styles.cardBody}>
                    <strong className={styles.name}>
                      {entry.remark || entry.nickname || entry.wxid}
                    </strong>
                    <small className={styles.sub}>{entry.wxid}</small>
                    {sexLabel(entry.sex) && <small className={styles.sub}>{sexLabel(entry.sex)}</small>}
                  </div>
                  {statusBadge(entry.agentStatus)}
                </button>
              );
            })}
          </div>
          {pageCount > 1 && (
            <div className={styles.pager}>
              <button type="button" className={styles.ghostBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
              <span className={styles.pagerInfo}>{safePage + 1} / {pageCount}</span>
              <button type="button" className={styles.ghostBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
            </div>
          )}
        </>
      )}

      {selectedWxids.size > 0 && (
        <div className={styles.actionBar}>
          <div className={styles.actionCount}>已选 {selectedWxids.size} 人</div>
          <textarea
            className={styles.noteInput}
            value={sharedNote}
            onChange={(e) => setSharedNote(e.target.value)}
            placeholder="本批运营备注（人类给 Agent 的运营意图，整批共享，如：地产意向客户，热情专业、以约看房为目标）"
            rows={2}
          />
          {playbooks.length > 0 && (
            <select
              className={styles.playbookSelect}
              value={playbookId}
              onChange={(e) => setPlaybookId(e.target.value)}
            >
              <option value="">账号默认运营方法</option>
              {playbooks.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          )}
          <button
            type="button"
            className={styles.primaryBtn}
            onClick={() => void onSubmit()}
            disabled={submitting || !sharedNote.trim()}
          >
            {submitting ? "加入中…" : "加入 Agent 运营"}
          </button>
        </div>
      )}
    </section>
  );
}
