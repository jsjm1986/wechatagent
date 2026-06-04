import { Suspense } from "react";
import { LogOut } from "lucide-react";
import { CHANNELS } from "./channels";
import { useNavigationStore } from "../stores/navigationStore";
import { useAuthStore } from "../stores/authStore";
import styles from "./Shell.module.css";

const GROUP_ORDER: ReadonlyArray<"运营" | "知识" | "系统"> = ["运营", "知识", "系统"];

export function Shell() {
  const activeChannel = useNavigationStore((s) => s.activeChannel);
  const setChannel = useNavigationStore((s) => s.setChannel);
  const user = useAuthStore((s) => s.user);
  const onLogout = useAuthStore((s) => s.onLogout);
  const def = CHANNELS.find((c) => c.id === activeChannel) ?? CHANNELS[0];
  const { Component } = def;

  const workspace = user?.currentWorkspace ?? (user?.workspaces ?? [])[0] ?? "default";

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
          <div className={styles.userBar}>
            <div className={styles.userAvatar}>{user.username.slice(0, 1).toUpperCase()}</div>
            <div className={styles.userInfo}>
              <span className={styles.userName}>{user.username}</span>
              <span className={styles.userWs}>{workspace}</span>
            </div>
            <button className={styles.logout} onClick={() => onLogout?.()}>
              <LogOut size={14} />
              登出
            </button>
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
