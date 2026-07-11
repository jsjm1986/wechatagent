import { useMemo, useState } from "react";
import { Overlay } from "../Overlay/Overlay";
import styles from "./FriendPickerModal.module.css";

export type FriendPickerItem = {
  wxid: string;
  nickname?: string | null;
  remark?: string | null;
  avatarUrl?: string | null;
  sex?: number | null;
  badge?: string;
};

const PAGE_SIZE = 60;

function label(item: FriendPickerItem): string {
  return item.remark || item.nickname || item.wxid;
}

export function FriendPickerModal({
  open,
  items,
  onSelect,
  onClose,
  title = "选择好友",
  loading = false,
  error = null,
  allowManualWxid = false,
  onManualWxid,
}: {
  open: boolean;
  items: FriendPickerItem[];
  onSelect: (item: FriendPickerItem) => void;
  onClose: () => void;
  title?: string;
  loading?: boolean;
  error?: string | null;
  allowManualWxid?: boolean;
  onManualWxid?: (wxid: string) => void;
}) {
  const [q, setQ] = useState("");
  const [page, setPage] = useState(0);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualWxid, setManualWxid] = useState("");

  const filtered = useMemo(() => {
    const query = q.trim().toLowerCase();
    if (!query) return items;
    return items.filter((it) =>
      [it.remark, it.nickname, it.wxid].some((v) => v?.toLowerCase().includes(query))
    );
  }, [items, q]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const pageRows = filtered.slice(safePage * PAGE_SIZE, safePage * PAGE_SIZE + PAGE_SIZE);

  if (!open) return null;

  return (
    <Overlay open={open} onClose={onClose} labelledBy="friendPickerTitle">
      <div className={styles.head}>
        <span className={styles.title} id="friendPickerTitle">{title}</span>
        <button type="button" className={styles.closeBtn} aria-label="关闭" onClick={onClose}>×</button>
      </div>

      <div className={styles.search}>
        <input
          className={styles.searchInput}
          placeholder="搜索好友（昵称/备注/wxid）"
          value={q}
          onChange={(e) => { setQ(e.target.value); setPage(0); }}
        />
      </div>

      {loading ? (
        <div className={styles.state}>加载中…</div>
      ) : error ? (
        <div className={styles.state} role="alert">加载失败：{error}</div>
      ) : filtered.length === 0 ? (
        <div className={styles.state}>{items.length === 0 ? "暂无好友" : "没有匹配的好友，换个关键词试试"}</div>
      ) : (
        <>
          <div className={styles.grid}>
            {pageRows.map((it) => (
              <button key={it.wxid} type="button" className={styles.card} onClick={() => onSelect(it)}>
                {it.avatarUrl ? (
                  <img className={styles.avatar} src={it.avatarUrl} alt="" loading="lazy" />
                ) : (
                  <span className={styles.avatarFallback}>{label(it).trim().charAt(0).toUpperCase()}</span>
                )}
                <span className={styles.cardBody}>
                  <span className={styles.name}>{label(it)}</span>
                  <span className={styles.sub}>{it.wxid}</span>
                </span>
                {it.badge && <span className={styles.badge}>{it.badge}</span>}
              </button>
            ))}
          </div>
          {pageCount > 1 && (
            <div className={styles.pager}>
              <button type="button" className={styles.pagerBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
              <span>{safePage + 1} / {pageCount}</span>
              <button type="button" className={styles.pagerBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
            </div>
          )}
        </>
      )}

      {allowManualWxid && (
        <div className={styles.manual}>
          {manualOpen ? (
            <div className={styles.manualRow}>
              <input
                className={styles.manualInput}
                placeholder="输入好友微信 wxid"
                value={manualWxid}
                onChange={(e) => setManualWxid(e.target.value)}
              />
              <button
                type="button"
                className={styles.manualBtn}
                disabled={!manualWxid.trim()}
                onClick={() => { onManualWxid?.(manualWxid.trim()); setManualWxid(""); setManualOpen(false); }}
              >
                确认
              </button>
            </div>
          ) : (
            <button type="button" className={styles.manualToggle} onClick={() => setManualOpen(true)}>
              找不到？手动输入 wxid
            </button>
          )}
        </div>
      )}
    </Overlay>
  );
}
