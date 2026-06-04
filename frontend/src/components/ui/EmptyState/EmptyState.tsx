import { Inbox } from "lucide-react";
import styles from "./EmptyState.module.css";

export function EmptyState({ icon, title, hint, action }: {
  icon?: React.ReactNode;
  title: string;
  hint?: string;
  action?: React.ReactNode;
}) {
  return (
    <div className={styles.empty}>
      <div className={styles.icon}>{icon ?? <Inbox size={28} />}</div>
      <strong className={styles.title}>{title}</strong>
      {hint && <p className={styles.hint}>{hint}</p>}
      {action && <div className={styles.action}>{action}</div>}
    </div>
  );
}
