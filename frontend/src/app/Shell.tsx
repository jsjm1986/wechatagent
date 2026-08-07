import { Suspense, useEffect, useRef, useState } from "react";
import { LogOut, Check, ChevronsUpDown, RefreshCw } from "lucide-react";
import { CHANNELS, GROUP_META, type ChannelGroup } from "./channels";
import { useNavigationStore } from "../stores/navigationStore";
import { useAuthStore } from "../stores/authStore";
import { useAccountStore } from "../stores/accountStore";
import { useProfileStore } from "../stores/profileStore";
import { api } from "../lib/api";
import type { Account } from "../types";
import styles from "./Shell.module.css";

/// 从 MCP 拉取并 upsert 微信号，再回拉账号列表刷新 store。供账号选择器
/// 复用——0 账号空态（新部署）和已有账号下都能触发。返回同步到的账号数。
async function syncAccounts(): Promise<number> {
  const res = await api.post<{ synced: number }>("/api/accounts/sync");
  const data = await api.get<{ items: Account[] }>("/api/accounts");
  useAccountStore.getState().setAccounts(data.items);
  return res.synced;
}

// 分组顺序与图标现在是 channels.ts 的 GROUP_META 单点定义（轨上的图标、
// 顺序、tooltip 文案都从那里来），此处不再维护第二份顺序数组——两份必然漂移。

function AccountSwitcher() {
  const accounts = useAccountStore((s) => s.accounts);
  const selectedAccountId = useAccountStore((s) => s.selectedAccountId);
  const selectAccount = useAccountStore((s) => s.selectAccount);
  const [open, setOpen] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [syncError, setSyncError] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDoc(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  async function handleSync() {
    if (syncing) return;
    setSyncing(true);
    setSyncError("");
    try {
      await syncAccounts();
    } catch (e) {
      setSyncError(e instanceof Error ? e.message : String(e));
    } finally {
      setSyncing(false);
    }
  }

  // 0 账号空态（如全新部署尚未同步）：选择器无内容可选，直接给一个同步入口，
  // 否则用户在 UI 里永远拉不进微信号。
  if (accounts.length === 0) {
    return (
      <div className={styles.acct}>
        <button
          type="button"
          className={styles.acctSync}
          onClick={handleSync}
          disabled={syncing}
        >
          <RefreshCw size={14} className={syncing ? styles.acctSyncSpin : ""} />
          <span>{syncing ? "正在同步…" : "同步微信号"}</span>
        </button>
        {syncError && <div className={styles.acctSyncErr}>{syncError}</div>}
      </div>
    );
  }

  const currentAccountId = accounts.some((a) => a.accountId === selectedAccountId)
    ? selectedAccountId
    : accounts[0]?.accountId ?? "";
  const onlineCount = accounts.filter((a) => a.online).length;
  const current = accounts.find((a) => a.accountId === currentAccountId);
  const label = (a: Account) => a.alias || a.displayName || a.accountId;

  return (
    <div className={styles.acct} ref={ref}>
      <button
        type="button"
        className={`${styles.acctTrigger} ${open ? styles.acctTriggerOpen : ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        <span className={current?.online ? styles.acctItemDot : styles.acctItemDotOff} />
        <span className={styles.acctTriggerName}>{current ? label(current) : "选择账号"}</span>
        <em className={styles.acctCount}>{onlineCount}/{accounts.length} 在线</em>
        <ChevronsUpDown size={14} className={styles.acctChevron} />
      </button>
      {open && (
        <div className={styles.acctMenu} role="listbox">
          {accounts.map((a) => {
            const active = a.accountId === currentAccountId;
            return (
              <button
                type="button"
                key={a.id ?? a.accountId}
                role="option"
                aria-selected={active}
                className={`${styles.acctOption} ${active ? styles.acctOptionActive : ""}`}
                onClick={() => {
                  selectAccount(a.accountId);
                  setOpen(false);
                }}
              >
                <span className={a.online ? styles.acctItemDot : styles.acctItemDotOff} />
                <span className={styles.acctOptionName}>{label(a)}</span>
                {active && <Check size={14} className={styles.acctCheck} />}
              </button>
            );
          })}
          <div className={styles.acctMenuDivider} />
          <button
            type="button"
            className={styles.acctOption}
            onClick={handleSync}
            disabled={syncing}
          >
            <RefreshCw size={14} className={syncing ? styles.acctSyncSpin : ""} />
            <span className={styles.acctOptionName}>
              {syncing ? "正在同步…" : "同步微信号"}
            </span>
          </button>
          {syncError && <div className={styles.acctSyncErr}>{syncError}</div>}
        </div>
      )}
    </div>
  );
}

/// 多 workspace 切换器：仅在 user.workspaces.length > 1 时渲染（单 workspace
/// 维持纯文本，见 Shell 内分支）。结构对齐 AccountSwitcher——trigger 显当前项 +
/// ChevronsUpDown，点开 listbox 每项 option + active 项 Check，点击外部关闭。
/// 选中某项调 authStore 的 onSwitchWorkspace（main.tsx 注入：POST 后 reload）。
function WorkspaceSwitcher({
  workspaces,
  current,
}: {
  workspaces: string[];
  current: string;
}) {
  const onSwitchWorkspace = useAuthStore((s) => s.onSwitchWorkspace);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDoc(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div className={styles.ws} ref={ref}>
      <button
        type="button"
        data-testid="workspace-switcher-trigger"
        className={`${styles.wsTrigger} ${open ? styles.wsTriggerOpen : ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        <span className={styles.wsTriggerName}>{current}</span>
        <ChevronsUpDown size={12} className={styles.wsChevron} />
      </button>
      {open && (
        <div className={styles.wsMenu} role="listbox">
          {workspaces.map((ws) => {
            const active = ws === current;
            return (
              <button
                type="button"
                key={ws}
                role="option"
                aria-selected={active}
                data-testid={`workspace-option-${ws}`}
                className={`${styles.wsOption} ${active ? styles.wsOptionActive : ""}`}
                onClick={() => {
                  onSwitchWorkspace?.(ws);
                  setOpen(false);
                }}
              >
                <span className={styles.wsOptionName}>{ws}</span>
                {active && <Check size={12} className={styles.wsCheck} />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function Shell() {
  const activeChannel = useNavigationStore((s) => s.activeChannel);
  const setChannel = useNavigationStore((s) => s.setChannel);
  const activeGroup = useNavigationStore((s) => s.activeGroup);
  const selectGroup = useNavigationStore((s) => s.selectGroup);
  const activeProfile = useProfileStore((s) => s.activeProfile);
  const user = useAuthStore((s) => s.user);
  const onLogout = useAuthStore((s) => s.onLogout);
  const def = CHANNELS.find((c) => c.id === activeChannel) ?? CHANNELS[0];
  const { Component } = def;

  // 轨与面板都要按 profile 过滤同一份频道，抽出来避免两处写法漂移
  // （漂移会导致轨上显示某组、点进去面板却是空的）。
  const groupItems = (group: ChannelGroup) =>
    CHANNELS.filter((c) => c.group === group).filter((c) =>
      c.visibleWhen ? c.visibleWhen(activeProfile) : true
    );

  const workspaces = user?.workspaces ?? [];
  const workspace = user?.currentWorkspace ?? workspaces[0] ?? "";
  const showWorkspace = workspaces.length > 1;

  return (
    <div className={styles.shell}>
      <aside className={styles.side}>
        <div className={styles.brand}>
          <div className={styles.brandMark} />
          <div className={styles.brandText}>
            <b>WeAgent</b>
            <span>私域自主运营</span>
          </div>
        </div>

        {/* 导航 = 图标轨（分组）+ 二级面板（该组频道）。
            为什么不再用手风琴：那不是设计选择、是高度妥协。侧栏 nav 可用高约 550px，
            20 个频道全展开需 1042px，两组同展也要 612px，于是被迫锁成「同时只开一组」，
            代价是跨组切频道要两步（先折叠再展开）且内容跳动。
            图标轨一次只渲染一组（最坏 5 行 ≈ 220px），任何视口都放得下，
            高度问题从结构上消失——原来那 6 档把行高压到 21px 的紧凑响应随之全删。 */}
        <div className={styles.navWrap}>
          {/* 轨上每个图标 = 一个分组。role=tablist 是准确语义：它切换的正是右侧面板。 */}
          <div
            className={styles.rail}
            role="tablist"
            aria-orientation="vertical"
            aria-label="频道分组"
          >
            {GROUP_META.map(({ group, icon: GroupIcon, hint }) => {
              const items = groupItems(group);
              if (items.length === 0) return null;
              const selected = activeGroup === group;
              // 当前频道在本组、但轨上选中的是别的组 → 打点，保住定位感。
              // 选中态自身已有高亮，不必再叠一个点。
              const holdsActive = !selected && items.some((c) => c.id === activeChannel);
              return (
                <button
                  key={group}
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  aria-label={group}
                  title={hint}
                  className={`${styles.railBtn} ${selected ? styles.railBtnOn : ""}`}
                  onClick={() => selectGroup(group)}
                  data-testid={`nav-group-${group}`}
                >
                  <GroupIcon size={18} />
                  {holdsActive && (
                    <span className={styles.railDot} aria-label="当前频道在此组" />
                  )}
                </button>
              );
            })}
          </div>

          {/* 二级面板：只画选中组的频道。组名在这里当面板标题，
              所以轨上的图标不需要再配文字标签（hover 有 title 兜底）。 */}
          <nav className={styles.panel} aria-label="Product channels">
            <p className={styles.panelTitle}>{activeGroup}</p>
            {groupItems(activeGroup).map((c) => {
              const Icon = c.icon;
              // 占位频道（Component 仍是工作台）不可点，免得点进去看到别的页面。
              if (c.comingSoon) {
                return (
                  <div
                    key={c.id}
                    className={`${styles.channel} ${styles.channelSoon}`}
                    aria-disabled="true"
                    title="下一阶段建设，暂未上线"
                  >
                    <Icon size={17} />
                    <span>{c.label}</span>
                    <span className={styles.soonBadge}>未上线</span>
                  </div>
                );
              }
              return (
                <button
                  key={c.id}
                  className={`${styles.channel} ${c.id === activeChannel ? styles.active : ""}`}
                  onClick={() => setChannel(c.id)}
                  aria-current={c.id === activeChannel ? "page" : undefined}
                >
                  <Icon size={17} />
                  <span>{c.label}</span>
                </button>
              );
            })}
          </nav>
        </div>

        {user && (
          <div className={styles.foot}>
            <AccountSwitcher />
            <div className={styles.userBar}>
              <div className={styles.userAvatar}>{user.username.slice(0, 1).toUpperCase()}</div>
              <div className={styles.userInfo}>
                <span className={styles.userName}>{user.username}</span>
                {showWorkspace ? (
                  <WorkspaceSwitcher workspaces={workspaces} current={workspace} />
                ) : (
                  workspace && <span className={styles.userWs}>{workspace}</span>
                )}
              </div>
              <button className={styles.logout} onClick={() => onLogout?.()}>
                <LogOut size={14} />
                登出
              </button>
            </div>
          </div>
        )}
      </aside>

      <main className={styles.main}>
        <header className={styles.header}>
          <p className={styles.eyebrow}>{def.eyebrow}</p>
          <h1 className={styles.title}>{def.title}</h1>
          <span className={styles.subtitle}>{def.subtitle}</span>
        </header>
        <Suspense fallback={<div className={styles.skeleton}>加载中…</div>}>
          <Component />
        </Suspense>
      </main>
    </div>
  );
}
