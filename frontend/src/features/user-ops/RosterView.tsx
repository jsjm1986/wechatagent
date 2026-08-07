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
  // 当前账号对象：用于在头部显示「本账号的微信 ID」。
  // 不用 store 的 currentAccount() 作为 selector——它每次返回新引用会导致重复渲染；
  // 从已订阅的 accounts + effectiveAccountId 推导，引用稳定。
  const currentAccount = accounts.find((a) => a.accountId === effectiveAccountId);
  const loadRoster = useUserOpsStore((s) => s.loadRoster);
  const batchEnable = useUserOpsStore((s) => s.batchEnable);
  const playbooks = useUserOpsStore((s) => s.playbooks);
  const publishedPlaybooks = playbooks.filter((playbook) => playbook.releaseStatus === "published");
  const toast = useToast();
  // ToastProvider 每次渲染都重建 api 对象字面量，引用不稳定。若把 toast 放进
  // 轮询 effect 的依赖，每次渲染都会重建 interval、3s 定时器永远走不到头，轮询
  // 形同不存在（实测就是这样：新快照落地也不提示）。用 ref 拿最新引用、不进依赖。
  const toastRef = useRef(toast);
  toastRef.current = toast;

  const rosterCache = useUserOpsStore((s) => s.rosterCache);
  const cached = rosterCache[effectiveAccountId];
  const roster = cached?.items ?? [];
  // syncing 是瞬态、不入缓存（store 仅缓存就绪结果，syncing:true 从不落缓存以允许自动重拉覆盖），
  // 故不能从 rosterCache 派生——保留本地 state，由 refresh 的返回值驱动。roster 仍从缓存派生以跨挂载存活。
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
  // 点「刷新」时记下当时的快照时间戳作为基线：后端 force 是「触发后台单飞 + 立即返回
  // 旧快照」，所以点完列表内容一模一样，看起来像坏了。记下基线后轮询，直到 fetchedAt
  // 变化才提示「已更新」，用户才知道刷新真的完成了。null = 当前没有等待中的刷新。
  const [awaitingSince, setAwaitingSince] = useState<string | null | undefined>(undefined);

  const refresh = useCallback(
    async (accountId: string, opts?: { force?: boolean }) => {
      if (!accountId) return;
      const seq = ++reqSeqRef.current;
      const isStale = () => seq !== reqSeqRef.current;
      setLoading(true);
      setError(null);
      // 切账号/刷新时连同本批草稿一起清空——否则账号 A 的运营备注/剧本会残留到账号 B。
      setSelectedWxids(new Set());
      setSharedNote("");
      setPlaybookId("");
      // force 前先取基线（此刻缓存里的快照龄）。从 getState() 直读而非闭包捕获，
      // 避免拿到上一次渲染的过时值。
      const baseline = opts?.force
        ? useUserOpsStore.getState().rosterCache[accountId]?.serverFetchedAt ?? null
        : undefined;
      try {
        const { syncing: isSyncing } = await loadRoster(accountId, opts);
        if (isStale()) return; // 已有更新的请求发出，丢弃本次过时结果。
        // roster 现从 store 缓存派生（跨挂载存活）；syncing 瞬态不入缓存，仍在此驱动自动重拉 effect。
        setSyncing(isSyncing);
        if (opts?.force) setAwaitingSince(baseline);
      } catch (e) {
        if (isStale()) return;
        setError(e instanceof Error ? e.message : "加载好友列表失败");
        setAwaitingSince(undefined); // 失败就不再等待，否则会一直转圈。
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

  // cache 同步中时每 10s 自动重拉（只读快照，不带 force→不触发新的后台拉取）；
  // 后台单飞任务写好快照后，普通读自然读到、syncing 变 false、轮询自停。
  useEffect(() => {
    if (!syncing || !effectiveAccountId) return;
    const timer = setInterval(() => {
      void refresh(effectiveAccountId);
    }, 10000);
    return () => clearInterval(timer);
  }, [syncing, effectiveAccountId, refresh]);

  // 刷新等待轮询：点「刷新」后每 3s 静默重读，直到快照的 serverFetchedAt 与基线不同
  // → 后台已写入新快照 → 提示「已更新」并停。兜底 60s 超时：后台可能仍在退避重试
  // （最多 5 次、3/6/12/24/48s），超时就告诉用户稍后再看，而不是无限转圈。
  //
  // **直调 loadRoster 而非 refresh**：refresh 会清空勾选/备注/剧本草稿（切账号语义）。
  // 用户点刷新后往往继续勾人，若每 3s 走一次 refresh，勾到一半会被清空——这比原本
  // 「刷新没反应」更糟。loadRoster 只更新缓存里的列表，不碰任何草稿 state。
  // 同理不设 loading（不带 force 的静默重读，不该让按钮一直转）。
  useEffect(() => {
    if (awaitingSince === undefined || !effectiveAccountId) return;
    const startedAt = Date.now();
    const timer = setInterval(() => {
      void (async () => {
        try {
          // revalidate（不是 force:false）：force:false 会命中 store 缓存直接返回，
          // 根本不发请求，于是永远观测不到新快照、轮询必然走到 60s 超时。
          // revalidate 绕过缓存重读一次，但不触发后端再起一个后台拉取任务。
          await loadRoster(effectiveAccountId, { revalidate: true });
        } catch {
          // 静默重读失败不打扰用户（主列表仍可用），下一轮再试；超时分支兜底收尾。
          return;
        }
        const now =
          useUserOpsStore.getState().rosterCache[effectiveAccountId]?.serverFetchedAt ?? null;
        if (now !== awaitingSince) {
          setAwaitingSince(undefined);
          toast.success("好友列表已更新");
          return;
        }
        if (Date.now() - startedAt > 60000) {
          setAwaitingSince(undefined);
          toast.info("微信侧仍在同步，稍后回来看");
        }
      })();
    }, 3000);
    return () => clearInterval(timer);
    // **toast 不进依赖**：ToastProvider 的 api 是对象字面量，每次渲染都是新引用。
    // 把它列进依赖会让本 effect 每渲染都重建 → 3s 定时器被无限重置 → 轮询永远
    // 不触发一次（实测抓到：点刷新后永远等不到「已更新」）。改走 toastRef。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [awaitingSince, effectiveAccountId, loadRoster]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return roster;
    return roster.filter((r) =>
      [r.remark, r.nickname, r.wxid].some((v) => v?.toLowerCase().includes(q))
    );
  }, [roster, filter]);

  const humanRows = useMemo(() => filtered.filter((r) => !r.isNonHuman), [filtered]);
  const nonHumanRows = useMemo(() => filtered.filter((r) => r.isNonHuman), [filtered]);
  const [showNonHuman, setShowNonHuman] = useState(false);

  const { pageRows, pageCount, safePage, setPage } = usePagedList(humanRows);

  const toggle = (entry: RosterEntry) => {
    if (entry.agentStatus === "managed" || entry.isNonHuman) return;
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
        source: "roster",
        candidates,
        sharedNote: sharedNote.trim(),
        playbookId: playbookId || undefined,
      });
      toast.success(`已加入 ${res.enabled} 人，画像后台生成中`);
      setSharedNote("");
      setPlaybookId("");
      await refresh(effectiveAccountId, { force: true });
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
          {/* 本账号身份行：列表里每张卡显示的是**好友**的 wxid，容易和自己的搞混，
              所以在头部明确标出当前账号自己的微信 ID。
              wxid 可能为空（MCP 未回传身份，如刚建账号未登录），此时整行不渲染，
              而不是显示「微信 ID: 空」。 */}
          {currentAccount?.wxid && (
            <p className={styles.selfIdentity}>
              <span className={styles.selfIdentityLabel}>本账号微信 ID</span>
              <code className={styles.selfIdentityValue}>{currentAccount.wxid}</code>
              {currentAccount.nickName && (
                <span className={styles.selfIdentityNick}>{currentAccount.nickName}</span>
              )}
            </p>
          )}
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
            onClick={() => void refresh(effectiveAccountId, { force: true })}
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
      ) : humanRows.length === 0 && nonHumanRows.length === 0 ? (
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

      {nonHumanRows.length > 0 && (
        <div className={styles.nonHumanSection}>
          <button type="button" className={styles.nonHumanToggle} onClick={() => setShowNonHuman((v) => !v)}>
            含 {nonHumanRows.length} 个系统账号（{showNonHuman ? "收起" : "展开"}）
          </button>
          {showNonHuman && (
            <div className={styles.grid}>
              {nonHumanRows.map((entry) => {
                return (
                  <button
                    key={entry.wxid}
                    type="button"
                    className={`${styles.card} ${styles.cardManaged}`}
                    disabled
                    aria-label={`${entry.remark || entry.nickname || entry.wxid}（系统账号，不可运营）`}
                  >
                    <div className={styles.checkbox} />
                    {entry.avatarUrl ? (
                      <img className={styles.avatar} src={entry.avatarUrl} alt="" loading="lazy" />
                    ) : (
                      <span className={styles.avatarFallback}>{initial(entry)}</span>
                    )}
                    <div className={styles.cardBody}>
                      <strong className={styles.name}>{entry.remark || entry.nickname || entry.wxid}</strong>
                      <small className={styles.sub}>{entry.wxid}</small>
                      <small className={styles.sysTag}>系统账号</small>
                    </div>
                    {statusBadge(entry.agentStatus)}
                  </button>
                );
              })}
            </div>
          )}
        </div>
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
          {publishedPlaybooks.length > 0 && (
            <select
              className={styles.playbookSelect}
              value={playbookId}
              onChange={(e) => setPlaybookId(e.target.value)}
            >
              <option value="">账号默认运营方法</option>
              {publishedPlaybooks.map((p) => (
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
