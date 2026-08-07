import { Suspense, useEffect, useRef, useState } from "react";
import { LogOut, Check, ChevronsUpDown, RefreshCw, ChevronRight } from "lucide-react";
import { CHANNELS, GROUP_ORDER, type ChannelGroup } from "./channels";
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

// 分组顺序由 channels.ts 的 GROUP_ORDER 单点提供，此处不再维护第二份数组
// ——两份必然漂移。

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
  const collapsedGroups = useNavigationStore((s) => s.collapsedGroups);
  const toggleGroup = useNavigationStore((s) => s.toggleGroup);
  const activeProfile = useProfileStore((s) => s.activeProfile);
  const user = useAuthStore((s) => s.user);
  const onLogout = useAuthStore((s) => s.onLogout);
  const def = CHANNELS.find((c) => c.id === activeChannel) ?? CHANNELS[0];
  const { Component } = def;

  // 按 profile 过滤频道。抽成函数是因为「渲染行」与「判断组内是否含活跃频道」
  // 两处都要用同一份结果，各写一遍必然漂移。
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

        {/* 导航 = 单列 + 可独立折叠的分组。
            与最初的手风琴的关键区别是**没有互斥**：手风琴强制「同时只展开一组」，
            所以展开 B 必然收起 A，跨组切频道要两步、内容还会跳动。那个约束是为了
            「侧栏绝不出滚动条」而设的，而滚动本身完全正常（VS Code / Linear 都滚），
            约束本身就是错的。现在允许滚动，互斥就没有必要了：每组自己记住开合，
            用户想同时开三组就开三组。
            默认收起「成效」「设置」（看结果的 + 配好不常动的），常用三组展开，
            高度 596px，在 900/800 高的屏上都不滚动。 */}
        <nav className={styles.nav} aria-label="Product channels">
          {GROUP_ORDER.map((group) => {
            const items = groupItems(group);
            if (items.length === 0) return null;
            const collapsed = collapsedGroups.has(group);
            // 收起时若活跃频道在组内，标签上打点——否则用户折叠后就丢失了「我在哪」。
            const holdsActive = collapsed && items.some((c) => c.id === activeChannel);
            return (
              <div key={group} className={styles.group}>
                {/* 分组标签是折叠开关。必须是 button（不是 p + onClick）：
                    键盘可达 + 读屏报「按钮，已展开」，aria-expanded 表达开合状态。 */}
                <button
                  type="button"
                  className={styles.groupLabel}
                  onClick={() => toggleGroup(group)}
                  aria-expanded={!collapsed}
                  data-testid={`nav-group-${group}`}
                >
                  <ChevronRight
                    size={13}
                    className={`${styles.groupChevron} ${collapsed ? "" : styles.groupChevronOpen}`}
                  />
                  <span className={styles.groupName}>{group}</span>
                  {holdsActive && (
                    <span className={styles.groupActiveDot} aria-label="当前频道在此组" />
                  )}
                  {collapsed && <span className={styles.groupCount}>{items.length}</span>}
                </button>
                {!collapsed && items.map((c) => {
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
              </div>
            );
          })}
        </nav>

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
