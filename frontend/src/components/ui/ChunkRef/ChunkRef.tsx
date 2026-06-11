import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import styles from "./ChunkRef.module.css";

/// 全局唯一的 ID 短展示规则，终结此前 slice(-6)/slice(0,8)/slice(0,12) 三套并存。
export function shortId(id: string): string {
  if (!id) return "";
  return id.length <= 10 ? id : `${id.slice(0, 6)}…${id.slice(-4)}`;
}

/// 统一的知识条目引用：显示标题（无标题回退短 ID），点击聚焦到 Inspector。
/// onFocus 由调用方注入（knowledge 传 focusChunk），保持 ui 层不依赖 features/。
export function ChunkRef({
  id,
  title,
  onFocus,
}: {
  id: string;
  title?: string | null;
  onFocus?: (id: string) => void;
}) {
  const label = title?.trim() || shortId(id);
  if (!onFocus) {
    return (
      <span className={styles.ref} title={id}>
        {label}
      </span>
    );
  }
  return (
    <button type="button" className={styles.refBtn} title={id} onClick={() => onFocus(id)}>
      {label}
    </button>
  );
}

export interface PickerChunk {
  id: string;
  title?: string | null;
}

/// 搜索式知识条目选择器：替代手输 24 位 ObjectId。
/// 内部拉一次 chunk 列表（由 loadChunks 注入，避免 ui 层耦合具体接口），按 title/id 过滤。
export function ChunkPicker({
  value,
  onChange,
  loadChunks,
  placeholder = "搜索知识条目（标题或编号）",
}: {
  value: string;
  onChange: (id: string) => void;
  loadChunks: () => Promise<PickerChunk[]>;
  placeholder?: string;
}) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [chunks, setChunks] = useState<PickerChunk[]>([]);
  const [loaded, setLoaded] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open || loaded) return;
    void loadChunks().then((list) => {
      setChunks(list);
      setLoaded(true);
    });
  }, [open, loaded, loadChunks]);

  useEffect(() => {
    if (!open) return;
    function onDoc(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    const pool = q
      ? chunks.filter((c) => (c.title ?? "").toLowerCase().includes(q) || c.id.toLowerCase().includes(q))
      : chunks;
    return pool.slice(0, 20);
  }, [query, chunks]);

  const selected = chunks.find((c) => c.id === value);

  return (
    <div className={styles.picker} ref={ref}>
      <button type="button" className={styles.pickerTrigger} onClick={() => setOpen((v) => !v)}>
        <Search size={13} />
        <span className={value ? styles.pickerValue : styles.pickerPlaceholder}>
          {value ? selected?.title?.trim() || shortId(value) : placeholder}
        </span>
      </button>
      {open && (
        <div className={styles.pickerMenu}>
          <input
            type="text"
            className={styles.pickerSearch}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="输入标题或编号过滤"
            autoFocus
          />
          <div className={styles.pickerList}>
            {!loaded ? (
              <div className={styles.pickerHint}>加载中…</div>
            ) : matches.length === 0 ? (
              <div className={styles.pickerHint}>无匹配</div>
            ) : (
              matches.map((c) => (
                <button
                  type="button"
                  key={c.id}
                  className={c.id === value ? styles.pickerItemActive : styles.pickerItem}
                  onClick={() => {
                    onChange(c.id);
                    setOpen(false);
                  }}
                >
                  <span className={styles.pickerItemTitle}>{c.title?.trim() || "（无标题）"}</span>
                  <span className={styles.pickerItemId}>{shortId(c.id)}</span>
                </button>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
