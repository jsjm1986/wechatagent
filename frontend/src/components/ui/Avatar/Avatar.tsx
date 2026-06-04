import styles from "./Avatar.module.css";
import type { StatusTone } from "../StatusBadge";

export function Avatar({ name, tone = "inactive", live = false }: { name: string; tone?: StatusTone; live?: boolean }) {
  return <div className={`${styles.avatar} ${styles[tone]} ${live ? styles.live : ""}`}>{name.slice(0, 1)}</div>;
}
