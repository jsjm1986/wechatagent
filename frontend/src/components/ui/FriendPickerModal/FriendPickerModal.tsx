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
  emptyText = "暂无好友",
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
  emptyText?: string;
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
    // maxWidth 720：双列网格放两个完整昵称需要的宽度（默认 480 只够一列半，
    // 昵称会大面积截断成 wxid_42jvcxc4…）。
    <Overlay open={open} onClose={onClose} labelledBy="friendPickerTitle" maxWidth={720}>
      {/* Overlay 的 .panel 自身无 padding（ConfirmDialog 也是自己包一层容器加的），
          故此处必须自带内边距容器，否则内容顶到面板边缘。
          .shell 是四行网格：头 / 搜索 / 列表(1fr) / 页脚。 */}
      <div className={styles.shell}>
        <div className={styles.head}>
          <span className={styles.title} id="friendPickerTitle">{title}</span>
          {/* 计数紧跟标题：4800 人的通讯录里，这是判断「搜索是否收窄了范围」的
              主要反馈。加载中/出错时不显示，避免亮出误导性的 0。 */}
          {!loading && !error && items.length > 0 && (
            <span className={styles.count}>
              {q.trim() ? `匹配 ${filtered.length} 位` : `共 ${filtered.length} 位`}
            </span>
          )}
          <button type="button" className={styles.closeBtn} aria-label="关闭" onClick={onClose}>×</button>
        </div>

        <input
          className={styles.searchInput}
          placeholder="搜索昵称 / 备注 / wxid"
          value={q}
          onChange={(e) => { setQ(e.target.value); setPage(0); }}
        />

        {loading ? (
          <div className={styles.state}>加载中…</div>
        ) : error ? (
          <div className={styles.state} role="alert">加载失败：{error}</div>
        ) : filtered.length === 0 ? (
          <div className={styles.state}>{items.length === 0 ? emptyText : "没有匹配的好友，换个关键词试试"}</div>
        ) : (
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
        )}

        {/* 页脚：左手动输入入口、右分页。两者都没有时整行不渲染，
            免得留一条空的分隔线。 */}
        {(allowManualWxid || pageCount > 1) && (
          <div className={styles.footer}>
            {allowManualWxid &&
              (manualOpen ? (
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
              ))}
            {pageCount > 1 && (
              <div className={styles.pager}>
                <button type="button" className={styles.pagerBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
                <span className={styles.pageInfo}>{safePage + 1} / {pageCount}</span>
                <button type="button" className={styles.pagerBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
              </div>
            )}
          </div>
        )}
      </div>
    </Overlay>
  );
}
