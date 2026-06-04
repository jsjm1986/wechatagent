import { useUiStore } from "../stores/uiStore";
import styles from "./GlobalErrorBanner.module.css";

export function GlobalErrorBanner() {
  const error = useUiStore((s) => s.error);
  const setError = useUiStore((s) => s.setError);
  if (!error) return null;
  return (
    <div className={styles.banner} role="alert">
      <span className={styles.text}>{error}</span>
      <button className={styles.close} onClick={() => setError("")} aria-label="关闭">
        ✕
      </button>
    </div>
  );
}
