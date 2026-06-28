import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import styles from "./Campaign.module.css";

interface StageOption { value: { id: string; label: string }; }

export function StageSelect({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [opts, setOpts] = useState<{ id: string; label: string }[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const r = await api.get<{ items: StageOption[] }>("/api/admin/taxonomies?kind=customer_stage");
        if (alive) setOpts(r.items.map((i) => ({ id: i.value.id, label: i.value.label })));
      } catch {
        if (alive) setFailed(true);
      }
    })();
    return () => { alive = false; };
  }, []);

  if (failed) return <div className={styles.fieldHint}>客户阶段选项加载失败</div>;

  return (
    <select className={styles.select} value={value} onChange={(e) => onChange(e.target.value)}>
      <option value="">不限</option>
      {opts.map((o) => (
        <option key={o.id} value={o.id}>{o.label}</option>
      ))}
    </select>
  );
}
