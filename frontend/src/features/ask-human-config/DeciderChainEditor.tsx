import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { Contact, DeciderRef } from "../../types";
import styles from "./AskHumanConfig.module.css";

function contactLabel(c: Contact): string {
  return c.nickname || c.remark || c.alias || c.wxid;
}

export function DeciderChainEditor({
  chain,
  onChange,
}: {
  chain: DeciderRef[];
  onChange: (next: DeciderRef[]) => void;
}) {
  const [picking, setPicking] = useState(false);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");

  useEffect(() => {
    if (!picking) return;
    void (async () => {
      try {
        const res = await api.get<{ items: Contact[] }>("/api/contacts?limit=100");
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, [picking]);

  const inChain = new Set(chain.map((d) => d.wxid));
  const candidates = contacts
    .filter((c) => !inChain.has(c.wxid))
    .filter((c) => (q.trim() ? contactLabel(c).includes(q) || c.wxid.includes(q) : true));

  function add(c: Contact) {
    onChange([...chain, { wxid: c.wxid, displayName: contactLabel(c) }]);
    setPicking(false);
    setQ("");
  }
  function remove(idx: number) {
    onChange(chain.filter((_, i) => i !== idx));
  }
  function move(idx: number, dir: -1 | 1) {
    const j = idx + dir;
    if (j < 0 || j >= chain.length) return;
    const next = [...chain];
    [next[idx], next[j]] = [next[j], next[idx]];
    onChange(next);
  }

  return (
    <div className={styles.chainEditor}>
      {chain.length === 0 && <div className={styles.chainEmpty}>尚未配置决策人</div>}
      {chain.map((d, idx) => (
        <div key={d.wxid} className={styles.chainRow}>
          <span className={styles.chainName} title={d.wxid}>
            {d.displayName ?? d.wxid}
            <span className={styles.chainWxid}>{d.wxid}</span>
          </span>
          <div className={styles.chainActions}>
            <button type="button" aria-label="上移" disabled={idx === 0} onClick={() => move(idx, -1)}>↑</button>
            <button type="button" aria-label="下移" disabled={idx === chain.length - 1} onClick={() => move(idx, 1)}>↓</button>
            <button type="button" aria-label="删除" onClick={() => remove(idx)}>✕</button>
          </div>
        </div>
      ))}
      <div className={styles.chainHint}>超时未响应时，按此顺序转交链中下一位</div>
      {picking ? (
        <div className={styles.pickerPanel}>
          <input
            className={styles.input}
            placeholder="搜索联系人（昵称/备注/wxid）"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <div className={styles.pickerList}>
            {candidates.map((c) => (
              <button key={c.id} type="button" className={styles.pickerItem} onClick={() => add(c)}>
                {contactLabel(c)}
                <span className={styles.chainWxid}>{c.wxid}</span>
              </button>
            ))}
            {candidates.length === 0 && <div className={styles.chainEmpty}>无可选联系人</div>}
          </div>
          <button type="button" className={styles.linkBtn} onClick={() => { setPicking(false); setQ(""); }}>取消</button>
        </div>
      ) : (
        <button type="button" className={styles.linkBtn} onClick={() => setPicking(true)}>+ 从联系人添加</button>
      )}
    </div>
  );
}
