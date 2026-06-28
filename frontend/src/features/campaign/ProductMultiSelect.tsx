import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import styles from "./Campaign.module.css";

interface ProductOption { productId: string; name: string; }

export function ProductMultiSelect({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const [opts, setOpts] = useState<ProductOption[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const r = await api.get<{ items: ProductOption[] }>("/api/products?active_only=true");
        if (alive) setOpts(r.items);
      } catch {
        if (alive) setFailed(true);
      }
    })();
    return () => { alive = false; };
  }, []);

  if (failed) return <div className={styles.fieldHint}>产品选项加载失败</div>;
  if (opts.length === 0) return <div className={styles.fieldHint}>暂无可选产品</div>;

  const toggle = (pid: string) => {
    onChange(value.includes(pid) ? value.filter((x) => x !== pid) : [...value, pid]);
  };

  return (
    <div className={styles.checkGroup}>
      {opts.map((o) => (
        <label key={o.productId} className={styles.checkItem}>
          <input type="checkbox" checked={value.includes(o.productId)} onChange={() => toggle(o.productId)} />
          <span>{o.name}</span>
        </label>
      ))}
    </div>
  );
}
