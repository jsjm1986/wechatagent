import styles from "./StatusLine.module.css";

export type StatusLineTone = "ai" | "good" | "neutral" | "warn";

export function StatusLine({ label, tone, value }: {
  label: string;
  tone: StatusLineTone;
  value: string;
}) {
  return (
    <div className={`${styles.line} ${styles[tone]}`}>
      <span className={styles.label}>{label}</span>
      <strong className={styles.value}>{value}</strong>
    </div>
  );
}
