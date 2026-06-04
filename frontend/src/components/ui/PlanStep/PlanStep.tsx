import { CheckCircle2 } from "lucide-react";
import styles from "./PlanStep.module.css";

export type PlanStepStatus = "ready" | "pending";

export function PlanStep({ detail, status, title }: {
  detail: string;
  status: PlanStepStatus;
  title: string;
}) {
  return (
    <div className={`${styles.step} ${styles[status]}`}>
      <CheckCircle2 size={16} />
      <div>
        <strong className={styles.title}>{title}</strong>
        <span className={styles.detail}>{detail}</span>
      </div>
    </div>
  );
}
