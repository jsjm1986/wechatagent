import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
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
  const [error, setError] = useState<string | null>(null);
  const accountId = useAccountStore((s) => s.currentAccountId());

  useEffect(() => {
    if (!picking) return;
    void (async () => {
      try {
        const url = accountId
          ? `/api/contacts?limit=100&accountId=${encodeURIComponent(accountId)}`
          : "/api/contacts?limit=100";
        const res = await api.get<{ items: Contact[] }>(url);
        setContacts(res.items);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setContacts([]);
      }
    })();
  }, [picking, accountId]);

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
            {error ? (
              <div className={styles.loadError} role="alert">加载联系人失败：{error}</div>
            ) : (
              <>
                {candidates.map((c) => (
                  <button key={c.id} type="button" className={styles.pickerItem} onClick={() => add(c)}>
                    {contactLabel(c)}
                    <span className={styles.chainWxid}>{c.wxid}</span>
                  </button>
                ))}
                {candidates.length === 0 &&
                  (contacts.length === 0 ? (
                    <div className={styles.chainEmpty}>
                      当前账号还没有联系人。请先到「账号管理」同步该账号的通讯录，再来配置决策人。
                    </div>
                  ) : (
                    <div className={styles.chainEmpty}>
                      {q.trim() ? "没有匹配的联系人，换个关键词试试。" : "该账号联系人都已在决策链中。"}
                    </div>
                  ))}
              </>
            )}
          </div>
          <button type="button" className={styles.linkBtn} onClick={() => { setPicking(false); setQ(""); }}>取消</button>
        </div>
      ) : (
        <button type="button" className={styles.linkBtn} onClick={() => setPicking(true)}>+ 从联系人添加</button>
      )}
    </div>
  );
}
