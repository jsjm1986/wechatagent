import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
import { PackageSearch, BadgeCheck, CreditCard, HelpCircle } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { api } from "../../lib/api";
import type { Contact } from "../../types";
import styles from "./ProductsDeals.module.css";

type Tab = "catalog" | "deals" | "holdings";

interface Product {
  productId: string;
  workspaceId: string;
  name: string;
  price: number | null;
  currency: string | null;
  sku: string | null;
  status: string;
  summary: string | null;
  attributes: Record<string, unknown>;
  createdAt: number;
  updatedAt: number;
}

interface OutcomeProductRef {
  productId: string;
  name: string;
  unitPrice?: number | null;
  sku?: string | null;
  quantity: number;
}

interface OutcomeEvent {
  markedAt: string;
  occurredAt?: string | null;
  amount?: number | null;
  currency?: string | null;
  source: string;
  markedBy: string;
  note?: string | null;
  verification: string;
  productRef?: OutcomeProductRef | null;
  eventKind?: string | null;
}

interface Entitlement {
  productId: string;
  name: string;
  ownedSince: number;
  quantity: number;
  inAftercare: boolean | null;
  expiresAt: number | null;
}

// §8.5.5 verification 徽标文案：一律用 AI 中性词，规避 CI 命名红线 lint 的禁词集。
const VERIFICATION_LABEL: Record<string, string> = {
  conversation_inferred: "疑似成交·待核实",
  staff_confirmed: "已核实",
  payment_verified: "支付核实",
};

function verificationBadgeClass(verification: string): string {
  switch (verification) {
    case "payment_verified":
      return styles.badgePayment;
    case "staff_confirmed":
      return styles.badgeConfirmed;
    default:
      return styles.badgeSuspected;
  }
}

function verificationIcon(verification: string) {
  switch (verification) {
    case "payment_verified":
      return <CreditCard size={12} />;
    case "staff_confirmed":
      return <BadgeCheck size={12} />;
    default:
      return <HelpCircle size={12} />;
  }
}

function fmtPrice(amount?: number | null, currency?: string | null): string {
  if (amount == null) return "—";
  return currency ? `${amount} ${currency}` : `${amount}`;
}

function fmtDate(ms?: number | null): string {
  if (ms == null) return "—";
  return new Date(ms).toISOString().slice(0, 10);
}

function fmtTs(ts?: string | null): string {
  if (!ts) return "—";
  return ts.slice(0, 10);
}

interface CatalogDraft {
  productId: string;
  name: string;
  price: string;
  currency: string;
  sku: string;
  summary: string;
}

const EMPTY_DRAFT: CatalogDraft = {
  productId: "",
  name: "",
  price: "",
  currency: "CNY",
  sku: "",
  summary: "",
};

export default function ProductsDealsFeature() {
  const [tab, setTab] = useState<Tab>("catalog");

  return (
    <div className={styles.page}>
      <div className={styles.tabs}>
        <button
          className={tab === "catalog" ? styles.tabActive : styles.tab}
          onClick={() => setTab("catalog")}
        >
          产品目录
        </button>
        <button
          className={tab === "deals" ? styles.tabActive : styles.tab}
          onClick={() => setTab("deals")}
        >
          成交记录
        </button>
        <button
          className={tab === "holdings" ? styles.tabActive : styles.tab}
          onClick={() => setTab("holdings")}
        >
          客户持有
        </button>
      </div>

      {tab === "catalog" && <CatalogTab />}
      {tab === "deals" && <DealsTab />}
      {tab === "holdings" && <HoldingsTab />}
    </div>
  );
}

function CatalogTab() {
  const [products, setProducts] = useState<Product[]>([]);
  const [draft, setDraft] = useState<CatalogDraft>(EMPTY_DRAFT);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const res = await api.get<{ items: Product[] }>("/api/products");
      setProducts(res.items);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleCreate = async (event: FormEvent) => {
    event.preventDefault();
    if (!draft.productId.trim() || !draft.name.trim()) return;
    setBusy(true);
    try {
      const priceNum = draft.price.trim() === "" ? null : Number(draft.price);
      await api.post("/api/products", {
        productId: draft.productId.trim(),
        name: draft.name.trim(),
        price: priceNum,
        currency: draft.currency.trim() || null,
        sku: draft.sku.trim() || null,
        summary: draft.summary.trim() || null,
      });
      setDraft(EMPTY_DRAFT);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleArchive = async (productId: string, archived: boolean) => {
    setBusy(true);
    try {
      const action = archived ? "restore" : "archive";
      await api.post(`/api/products/${encodeURIComponent(productId)}/${action}`);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.workbench}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Product Catalog</span>
            <span className={styles.title}>产品目录</span>
          </div>
          <span className={styles.headIcon}>
            <PackageSearch size={17} />
          </span>
        </div>
        {error && <p className={styles.error}>{error}</p>}
        {products.length === 0 ? (
          <EmptyState
            title="暂无产品"
            hint="在右侧录入结构化产品（product_id / 价格 / SKU），agent 报价以此为准。无产品行业可留空。"
          />
        ) : (
          <div className={styles.list}>
            {products.map((p) => (
              <div key={p.productId} className={styles.row}>
                <div className={styles.rowHead}>
                  <strong className={styles.rowTitle}>{p.name}</strong>
                  <span className={p.status === "active" ? styles.statusActive : styles.statusArchived}>
                    {p.status === "active" ? "在售" : "已归档"}
                  </span>
                </div>
                <p className={styles.body}>
                  id={p.productId} · {fmtPrice(p.price, p.currency)}
                  {p.sku ? ` · SKU=${p.sku}` : ""}
                  {p.summary ? ` · ${p.summary}` : ""}
                </p>
                <div className={styles.rowActions}>
                  <button
                    className={styles.linkBtn}
                    disabled={busy}
                    onClick={() => handleArchive(p.productId, p.status !== "active")}
                  >
                    {p.status === "active" ? "归档" : "恢复"}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      <form className={styles.panel} onSubmit={handleCreate}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>新增</span>
            <span className={styles.title}>录入产品</span>
          </div>
        </div>
        <div className={styles.form}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>product_id（业务主键，工作区内唯一）</span>
            <input
              className={styles.input}
              value={draft.productId}
              onChange={(e) => setDraft({ ...draft, productId: e.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>产品名</span>
            <input
              className={styles.input}
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>价格</span>
            <input
              className={styles.input}
              type="number"
              step="0.01"
              value={draft.price}
              onChange={(e) => setDraft({ ...draft, price: e.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>币种</span>
            <input
              className={styles.input}
              value={draft.currency}
              onChange={(e) => setDraft({ ...draft, currency: e.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>SKU</span>
            <input
              className={styles.input}
              value={draft.sku}
              onChange={(e) => setDraft({ ...draft, sku: e.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>简述</span>
            <textarea
              className={styles.textarea}
              value={draft.summary}
              onChange={(e) => setDraft({ ...draft, summary: e.target.value })}
            />
          </label>
          <button
            className={styles.submit}
            type="submit"
            disabled={busy || !draft.productId.trim() || !draft.name.trim()}
          >
            保存产品
          </button>
        </div>
      </form>
    </div>
  );
}

function ContactPicker({
  selected,
  onSelect,
}: {
  selected: Contact | null;
  onSelect: (c: Contact | null) => void;
}) {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [q, setQ] = useState("");

  useEffect(() => {
    void (async () => {
      try {
        const res = await api.get<{ items: Contact[] }>("/api/contacts?limit=100");
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, []);

  const filtered = q.trim()
    ? contacts.filter(
        (c) =>
          (c.nickname ?? "").includes(q) ||
          (c.remark ?? "").includes(q) ||
          c.wxid.includes(q)
      )
    : contacts;

  return (
    <section className={styles.pickerPanel}>
      <input
        className={styles.input}
        placeholder="搜索好友（昵称/备注/wxid）"
        value={q}
        onChange={(e) => setQ(e.target.value)}
      />
      <div className={styles.pickerList}>
        {filtered.map((c) => (
          <button
            key={c.id}
            className={selected?.id === c.id ? styles.pickerItemActive : styles.pickerItem}
            onClick={() => onSelect(c)}
          >
            {c.nickname || c.remark || c.wxid}
          </button>
        ))}
      </div>
    </section>
  );
}

function DealsTab() {
  const [selected, setSelected] = useState<Contact | null>(null);
  const [events, setEvents] = useState<OutcomeEvent[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected) {
      setEvents([]);
      return;
    }
    void (async () => {
      try {
        const res = await api.get<{ items: OutcomeEvent[] }>(
          `/api/contacts/${selected.id}/outcome-events`
        );
        setEvents(res.items);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [selected]);

  return (
    <div className={styles.workbench}>
      <ContactPicker selected={selected} onSelect={setSelected} />
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Deal Records</span>
            <span className={styles.title}>成交记录</span>
          </div>
        </div>
        {error && <p className={styles.error}>{error}</p>}
        {!selected ? (
          <EmptyState title="请选择好友" hint="从左侧选择一个好友查看其成交记录。" />
        ) : events.length === 0 ? (
          <EmptyState title="暂无成交记录" hint="该好友还没有登记成交事件。" />
        ) : (
          <div className={styles.list}>
            {events.map((ev, i) => {
              const isReversal = ev.eventKind === "reversal";
              return (
                <div key={i} className={styles.row}>
                  <div className={styles.rowHead}>
                    <strong className={styles.rowTitle}>
                      {isReversal ? "退款/撤单 · " : ""}
                      {ev.productRef?.name ?? "（无关联产品）"}
                    </strong>
                    <span className={verificationBadgeClass(ev.verification)}>
                      {verificationIcon(ev.verification)}
                      {VERIFICATION_LABEL[ev.verification] ?? ev.verification}
                    </span>
                  </div>
                  <p className={styles.body}>
                    {isReversal ? "退款 " : ""}
                    {fmtPrice(ev.amount, ev.currency)}
                    {ev.productRef ? ` · ${isReversal ? "退" : ""}${ev.productRef.quantity} 件` : ""}
                    {` · 标记于 ${fmtTs(ev.occurredAt ?? ev.markedAt)}`}
                    {ev.markedBy ? ` · ${ev.markedBy}` : ""}
                    {ev.note ? ` · ${ev.note}` : ""}
                  </p>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}

function HoldingsTab() {
  const [selected, setSelected] = useState<Contact | null>(null);
  const [items, setItems] = useState<Entitlement[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected) {
      setItems([]);
      return;
    }
    void (async () => {
      try {
        const res = await api.get<{ items: Entitlement[]; total: number }>(
          `/api/contacts/${selected.id}/entitlements`
        );
        setItems(res.items);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [selected]);

  return (
    <div className={styles.workbench}>
      <ContactPicker selected={selected} onSelect={setSelected} />
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Holdings</span>
            <span className={styles.title}>客户持有</span>
          </div>
        </div>
        {error && <p className={styles.error}>{error}</p>}
        {!selected ? (
          <EmptyState title="请选择好友" hint="从左侧选择一个好友查看其当前持有的产品。" />
        ) : items.length === 0 ? (
          <EmptyState
            title="暂无持有"
            hint="该好友没有已核实成交派生的持有记录（疑似线索不计入）。"
          />
        ) : (
          <div className={styles.list}>
            {items.map((e) => (
              <div key={e.productId} className={styles.row}>
                <div className={styles.rowHead}>
                  <strong className={styles.rowTitle}>{e.name}</strong>
                  {e.inAftercare === true && <span className={styles.statusActive}>售后/有效期内</span>}
                  {e.inAftercare === false && <span className={styles.statusArchived}>有效期已过</span>}
                </div>
                <p className={styles.body}>
                  共 {e.quantity} 件 · 自 {fmtDate(e.ownedSince)} 起持有
                  {e.expiresAt != null ? ` · 至 ${fmtDate(e.expiresAt)}` : ""}
                </p>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
