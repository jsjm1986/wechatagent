import styles from "./Avatar.module.css";
import type { StatusTone } from "../StatusBadge";

export function Avatar({ name, tone = "inactive" }: { name: string; tone?: StatusTone }) {
  return <div className={`${styles.avatar} ${styles[tone]}`}>{name.slice(0, 1)}</div>;
}
