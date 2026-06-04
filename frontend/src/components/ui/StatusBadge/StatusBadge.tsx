import styles from "./StatusBadge.module.css";

export type StatusTone = "running" | "scheduled" | "held" | "blocked" | "inactive";

export function StatusBadge({ tone, children }: { tone: StatusTone; children: React.ReactNode }) {
  return (
    <span className={`${styles.badge} ${styles[tone]}`}>
      <span className={styles.dot} />
      {children}
    </span>
  );
}