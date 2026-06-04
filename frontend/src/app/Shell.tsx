import { Suspense, useEffect, useRef, useState } from "react";
import { LogOut, Check, ChevronsUpDown } from "lucide-react";
import { CHANNELS } from "./channels";
import { useNavigationStore } from "../stores/navigationStore";
import { useAuthStore } from "../stores/authStore";
import { useAccountStore } from "../stores/accountStore";
import type { Account } from "../types";
import styles from "./Shell.module.css";

const GROUP_ORDER: ReadonlyArray<"运营" | "知识" | "系统"> = ["运营", "知识", "系统"];

function AccountSwitcher() {
  const accounts = useAccountStore((s) => s.accounts);
  const selectedAccountId = useAccountStore((s) => s.selectedAccountId);
  const selectAccount = useAccountStore((s) => s.selectAccount);
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

  if (accounts.length === 0) return null;

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
        </div>
      )}
    </div>
  );
}

export function Shell() {
  const activeChannel = useNavigationStore((s) => s.activeChannel);
  const setChannel = useNavigationStore((s) => s.setChannel);
  const user = useAuthStore((s) => s.user);
  const onLogout = useAuthStore((s) => s.onLogout);
  const def = CHANNELS.find((c) => c.id === activeChannel) ?? CHANNELS[0];
  const { Component } = def;

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

        <nav className={styles.nav} aria-label="Product channels">
          {GROUP_ORDER.map((group) => (
            <div key={group} className={styles.group}>
              <div className={styles.groupLabel}>{group}</div>
              {CHANNELS.filter((c) => c.group === group).map((c) => {
                const Icon = c.icon;
                return (
                  <button
                    key={c.id}
                    className={`${styles.channel} ${c.id === activeChannel ? styles.active : ""}`}
                    onClick={() => setChannel(c.id)}
                  >
                    <Icon size={17} />
                    <span>{c.label}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </nav>

        {user && (
          <div className={styles.foot}>
            <AccountSwitcher />
            <div className={styles.userBar}>
              <div className={styles.userAvatar}>{user.username.slice(0, 1).toUpperCase()}</div>
              <div className={styles.userInfo}>
                <span className={styles.userName}>{user.username}</span>
                {showWorkspace && <span className={styles.userWs}>{workspace}</span>}
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
