import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
import { PackageSearch, BadgeCheck, CreditCard, HelpCircle, ClipboardCheck } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import type { Contact } from "../../types";
import styles from "./ProductsDeals.module.css";

type Tab = "catalog" | "deals" | "holdings" | "review";

interface Product {
  productId: string;
  workspaceId: string;
  name: string;
  /** 最小币种单位整数（分，19900=¥199.00）；展示用 fmtPrice ÷100。 */
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
  /** 最小币种单位整数（分）。 */
  unitPrice?: number | null;
  sku?: string | null;
  quantity: number;
  entitlementDays?: number | null;
}

interface OutcomeEvent {
  markedAt: string;
  occurredAt?: string | null;
  /** 最小币种单位整数（分，19900=¥199.00）；展示用 fmtPrice ÷100。 */
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

// 金额整数化：后端金额是最小币种单位整数（分，19900=¥199.00），仅在此展示层 ÷100 转元。
function fmtPrice(amountCents?: number | null, currency?: string | null): string {
  if (amountCents == null) return "—";
  const major = (amountCents / 100).toFixed(2);
  if (!currency) return major;
  return currency === "CNY" ? `¥${major}` : `${major} ${currency}`;
}

// 录入边界：用户输入「元」（input step=0.01），提交前 ×100 转「分」整数。
// Math.round 防 1.1*100=110.00000000000001 之类的浮点误差。空/纯空白/非数返回 null。
function yuanToCents(input: unknown): number | null {
  if (input == null) return null;
  const text = typeof input === "string" ? input.trim() : input;
  if (text === "") return null;
  const yuan = Number(text);
  if (!Number.isFinite(yuan)) return null;
  return Math.round(yuan * 100);
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
        <button
          className={tab === "review" ? styles.tabActive : styles.tab}
          onClick={() => setTab("review")}
        >
          疑似成交待核实
        </button>
      </div>

      {tab === "catalog" && <CatalogTab />}
      {tab === "deals" && <DealsTab />}
      {tab === "holdings" && <HoldingsTab />}
      {tab === "review" && <SuspectedDealsTab />}
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
      const priceNum = yuanToCents(draft.price);
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
            hint="在右侧录入产品（产品编号 / 价格 / SKU），AI 报价以此为准。无产品行业可留空。"
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
            <span className={styles.fieldLabel}>产品编号（自定，同一账号内不可重复）</span>
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
  const currentAccountId = useAccountStore((s) => s.currentAccountId);

  useEffect(() => {
    void (async () => {
      try {
        const accountId = currentAccountId();
        const url = accountId
          ? `/api/contacts?limit=100&accountId=${encodeURIComponent(accountId)}`
          : "/api/contacts?limit=100";
        const res = await api.get<{ items: Contact[] }>(url);
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, [currentAccountId]);

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

interface DealDraft {
  eventKind: "deal" | "reversal";
  productId: string;
  quantity: string;
  amount: string;
  currency: string;
  verification: "staff_confirmed" | "payment_verified";
  occurredAt: string;
  note: string;
}

const EMPTY_DEAL_DRAFT: DealDraft = {
  eventKind: "deal",
  productId: "",
  quantity: "1",
  amount: "",
  currency: "",
  verification: "staff_confirmed",
  occurredAt: "",
  note: "",
};

function DealsTab() {
  const [selected, setSelected] = useState<Contact | null>(null);
  const [events, setEvents] = useState<OutcomeEvent[]>([]);
  const [products, setProducts] = useState<Product[]>([]);
  const [draft, setDraft] = useState<DealDraft>(EMPTY_DEAL_DRAFT);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const loadEvents = useCallback(async (contact: Contact) => {
    try {
      const res = await api.get<{ items: OutcomeEvent[] }>(
        `/api/contacts/${contact.id}/outcome-events`
      );
      setEvents(res.items);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    if (!selected) {
      setEvents([]);
      return;
    }
    setDraft(EMPTY_DEAL_DRAFT);
    setFormError(null);
    setInfo(null);
    void loadEvents(selected);
  }, [selected, loadEvents]);

  // 产品下拉选项：拉一次本工作区产品表（deal 只选 active，reversal 放宽全部 status）。
  useEffect(() => {
    void (async () => {
      try {
        const res = await api.get<{ items: Product[] }>("/api/products");
        setProducts(res.items);
      } catch {
        setProducts([]);
      }
    })();
  }, []);

  const productOptions =
    draft.eventKind === "reversal"
      ? products
      : products.filter((p) => p.status === "active");

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!selected) return;
    setFormError(null);
    setInfo(null);
    // 前端预校验：reversal 必须关联产品（呼应后端 400 规则）。
    if (draft.eventKind === "reversal" && !draft.productId.trim()) {
      setFormError("退款/撤单必须选择关联产品（指明抵消哪个产品的持有）。");
      return;
    }
    // 校验用户输入的「元」值非负，再 ×100 转「分」整数传后端。
    // 单一入口：yuanToCents 内部已 trim+判空+判有限，避免与校验口径分叉（纯空格曾静默落 0 元）。
    const amountCents = yuanToCents(draft.amount);
    if (draft.amount.trim() !== "" && amountCents == null) {
      setFormError("金额必须是有效数字。");
      return;
    }
    if (amountCents != null && amountCents < 0) {
      setFormError("金额必须是非负数。");
      return;
    }
    // 只传非空字段；eventKind / verification 始终传。
    const body: Record<string, unknown> = {
      eventKind: draft.eventKind,
      verification: draft.verification,
    };
    if (draft.productId.trim()) {
      body.productId = draft.productId.trim();
      const qtyNum = Number(draft.quantity);
      body.quantity = Number.isFinite(qtyNum) && qtyNum >= 1 ? Math.floor(qtyNum) : 1;
    }
    if (amountCents != null) body.amount = amountCents;
    if (draft.currency.trim()) body.currency = draft.currency.trim();
    if (draft.occurredAt.trim()) {
      const ms = new Date(draft.occurredAt).getTime();
      if (Number.isFinite(ms)) body.occurredAtMs = ms;
    }
    if (draft.note.trim()) body.note = draft.note.trim();

    setBusy(true);
    try {
      await api.post(`/api/contacts/${selected.id}/deal-events`, body);
      setDraft(EMPTY_DEAL_DRAFT);
      setInfo(draft.eventKind === "reversal" ? "已登记退款/撤单。" : "已登记成交。");
      await loadEvents(selected);
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.workbench}>
      <ContactPicker selected={selected} onSelect={setSelected} />
      <div className={styles.stack}>
        {selected && (
          <form className={styles.panel} onSubmit={handleSubmit}>
            <div className={styles.head}>
              <div className={styles.headL}>
                <span className={styles.eyebrow}>Register</span>
                <span className={styles.title}>登记成交 / 退款</span>
              </div>
            </div>
            {formError && <p className={styles.error}>{formError}</p>}
            {info && <p className={styles.success}>{info}</p>}
            <div className={styles.form}>
              <div className={styles.field}>
                <span className={styles.fieldLabel}>事件方向</span>
                <div className={styles.segGroup}>
                  <button
                    type="button"
                    className={draft.eventKind === "deal" ? styles.segActive : styles.seg}
                    onClick={() => setDraft({ ...draft, eventKind: "deal" })}
                  >
                    成交
                  </button>
                  <button
                    type="button"
                    className={draft.eventKind === "reversal" ? styles.segActive : styles.seg}
                    onClick={() => setDraft({ ...draft, eventKind: "reversal" })}
                  >
                    退款 / 撤单
                  </button>
                </div>
              </div>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>
                  关联产品{draft.eventKind === "reversal" ? "（退款必选）" : "（可选）"}
                </span>
                <select
                  className={styles.select}
                  value={draft.productId}
                  onChange={(e) => {
                    const pid = e.target.value;
                    const prod = products.find((p) => p.productId === pid);
                    setDraft({
                      ...draft,
                      productId: pid,
                      currency: prod?.currency ?? draft.currency,
                    });
                  }}
                >
                  <option value="">
                    {draft.eventKind === "reversal" ? "请选择产品" : "（无关联产品）"}
                  </option>
                  {productOptions.map((p) => (
                    <option key={p.productId} value={p.productId}>
                      {p.name}
                      {p.status !== "active" ? "（已归档）" : ""}
                      {p.price != null ? ` · ${fmtPrice(p.price, p.currency)}` : ""}
                    </option>
                  ))}
                </select>
              </label>
              {draft.productId.trim() && (
                <label className={styles.field}>
                  <span className={styles.fieldLabel}>件数</span>
                  <input
                    className={styles.input}
                    type="number"
                    min="1"
                    step="1"
                    value={draft.quantity}
                    onChange={(e) => setDraft({ ...draft, quantity: e.target.value })}
                  />
                </label>
              )}
              <label className={styles.field}>
                <span className={styles.fieldLabel}>金额{draft.eventKind === "reversal" ? "（退款额）" : ""}</span>
                <input
                  className={styles.input}
                  type="number"
                  step="0.01"
                  min="0"
                  value={draft.amount}
                  onChange={(e) => setDraft({ ...draft, amount: e.target.value })}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>币种</span>
                <input
                  className={styles.input}
                  value={draft.currency}
                  placeholder="如 CNY"
                  onChange={(e) => setDraft({ ...draft, currency: e.target.value })}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>可信度</span>
                <select
                  className={styles.select}
                  value={draft.verification}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      verification: e.target.value as DealDraft["verification"],
                    })
                  }
                >
                  <option value="staff_confirmed">已核实</option>
                  <option value="payment_verified">支付核实</option>
                </select>
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>发生时间（可选，默认现在）</span>
                <input
                  className={styles.input}
                  type="date"
                  value={draft.occurredAt}
                  onChange={(e) => setDraft({ ...draft, occurredAt: e.target.value })}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>备注</span>
                <textarea
                  className={styles.textarea}
                  value={draft.note}
                  onChange={(e) => setDraft({ ...draft, note: e.target.value })}
                />
              </label>
              <button className={styles.submit} type="submit" disabled={busy}>
                {busy ? "提交中…" : draft.eventKind === "reversal" ? "登记退款" : "登记成交"}
              </button>
            </div>
          </form>
        )}

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

/** F23：一条疑似成交待核实信号（GET /api/admin/suspected-deals 返回形状）。 */
interface SuspectedDeal {
  id: string;
  contactId: string;
  value: string;
  evidence?: string | null;
  confidence: number;
  occurrences: number;
  status: string;
  lastSeenAt?: string | null;
}

/**
 * F23：疑似成交待核实闭环（方案B）。AI 决策时判断客户疑似成交会产出一条弱信号，
 * 沉到待核实队列——**绝不直接落成交**。本面板由运营核实：通过则后端调用落正式
 * 成交（verification=staff_confirmed），驳回则仅标记 rejected。富展示判断依据 /
 * 置信度 / 客户 / 出现次数（呼应 SimpleApproveReject 模式）。
 */
function SuspectedDealsTab() {
  const [items, setItems] = useState<SuspectedDeal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  // 每条信号独立的成交金额 / 币种录入（通过时一并提交，均可选）。
  const [drafts, setDrafts] = useState<Record<string, { amount: string; currency: string }>>({});

  const load = useCallback(async () => {
    try {
      const res = await api.get<{ items: SuspectedDeal[] }>(
        "/api/admin/suspected-deals?status=pending"
      );
      setItems(res.items);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const draftOf = (id: string) => drafts[id] ?? { amount: "", currency: "" };

  const handleApprove = async (item: SuspectedDeal) => {
    const d = draftOf(item.id);
    const amountCents = yuanToCents(d.amount);
    if (d.amount.trim() !== "" && amountCents == null) {
      setError("金额必须是有效数字。");
      return;
    }
    const body: Record<string, unknown> = {};
    if (amountCents != null) body.amount = amountCents;
    if (d.currency.trim()) body.currency = d.currency.trim();
    setBusyId(item.id);
    try {
      await api.post(`/api/admin/suspected-deals/${encodeURIComponent(item.id)}/approve`, body);
      setInfo("已确认成交（已核实）。");
      setError(null);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  };

  const handleReject = async (item: SuspectedDeal) => {
    const reason = window.prompt("请填写驳回原因（如：误判，实际只是咨询）");
    if (reason == null) return;
    if (!reason.trim()) {
      setError("驳回原因不能为空。");
      return;
    }
    setBusyId(item.id);
    try {
      await api.post(`/api/admin/suspected-deals/${encodeURIComponent(item.id)}/reject`, {
        reason: reason.trim(),
      });
      setInfo("已驳回该疑似成交线索。");
      setError(null);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className={styles.workbench}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Suspected Deals</span>
            <span className={styles.title}>疑似成交待核实</span>
          </div>
          <span className={styles.headIcon}>
            <ClipboardCheck size={17} />
          </span>
        </div>
        {error && <p className={styles.error}>{error}</p>}
        {info && <p className={styles.success}>{info}</p>}
        {items.length === 0 ? (
          <EmptyState
            title="暂无待核实线索"
            hint="AI 判断客户疑似成交时会在此沉淀线索，待运营核实后才落正式成交。"
          />
        ) : (
          <div className={styles.list}>
            {items.map((item) => {
              const d = draftOf(item.id);
              return (
                <div key={item.id} className={styles.row}>
                  <div className={styles.rowHead}>
                    <strong className={styles.rowTitle}>{item.value || "疑似成交·待核实"}</strong>
                    <span className={styles.badgeSuspected}>
                      <HelpCircle size={12} />
                      待核实
                    </span>
                  </div>
                  <p className={styles.body}>
                    {item.evidence ? `判断依据：${item.evidence}` : "判断依据：—"}
                    {` · 置信度 ${item.confidence}`}
                    {` · 出现 ${item.occurrences} 次`}
                    {` · 客户 ${item.contactId}`}
                    {item.lastSeenAt ? ` · 最近 ${fmtTs(item.lastSeenAt)}` : ""}
                  </p>
                  <div className={styles.form}>
                    <label className={styles.field}>
                      <span className={styles.fieldLabel}>成交金额（可选）</span>
                      <input
                        className={styles.input}
                        type="number"
                        step="0.01"
                        min="0"
                        value={d.amount}
                        onChange={(e) =>
                          setDrafts({ ...drafts, [item.id]: { ...d, amount: e.target.value } })
                        }
                      />
                    </label>
                    <label className={styles.field}>
                      <span className={styles.fieldLabel}>币种（可选）</span>
                      <input
                        className={styles.input}
                        value={d.currency}
                        placeholder="如 CNY"
                        onChange={(e) =>
                          setDrafts({ ...drafts, [item.id]: { ...d, currency: e.target.value } })
                        }
                      />
                    </label>
                  </div>
                  <div className={styles.rowActions}>
                    <button
                      className={styles.submit}
                      disabled={busyId === item.id}
                      onClick={() => void handleApprove(item)}
                    >
                      确认成交
                    </button>
                    <button
                      className={styles.linkBtn}
                      disabled={busyId === item.id}
                      onClick={() => void handleReject(item)}
                    >
                      驳回
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
