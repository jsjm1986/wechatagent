import styles from "./MetricCard.module.css";

export function MetricCard({ detail, label, onClick, value }: {
  detail: string;
  label: string;
  onClick: () => void;
  value: number;
}) {
  return (
    <button className={styles.card} onClick={onClick}>
      <span className={styles.label}>{label}</span>
      <strong className={styles.value}>{value}</strong>
      <small className={styles.detail}>{detail}</small>
    </button>
  );
}
