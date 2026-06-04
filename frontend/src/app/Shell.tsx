import { Suspense } from "react";
import { CHANNELS } from "./channels";
import { useNavigationStore } from "../stores/navigationStore";
import styles from "./Shell.module.css";

const GROUP_ORDER: ReadonlyArray<"运营" | "知识" | "系统"> = ["运营", "知识", "系统"];

export function Shell() {
  const activeChannel = useNavigationStore((s) => s.activeChannel);
  const setChannel = useNavigationStore((s) => s.setChannel);
  const def = CHANNELS.find((c) => c.id === activeChannel) ?? CHANNELS[0];
  const { Component } = def;

  return (
    <div className={styles.shell}>
      <aside className={styles.side}>
        <nav className={styles.nav} aria-label="Product channels">
          {GROUP_ORDER.map((group) => (
            <div key={group}>
              <div className={styles.group}>{group}</div>
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
