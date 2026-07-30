import { useCallback, useEffect, useState } from "react";
import { ConfirmProvider } from "../../components/ui/ConfirmDialog";
import { ToastProvider, useToast } from "../../components/ui/Toast";
import { api } from "../../lib/api";
import type { AskHumanPolicy } from "../../types";
import { defaultPolicy, extractPolicy, validatePolicy } from "./policyForm";
import { DeciderChainEditor } from "./DeciderChainEditor";
import styles from "./AskHumanConfig.module.css";

const DOMAIN = "user_operations";

// 仅 4 个 boolean 开关字段的键联合，避免把非-bool 字段的 key 混入（否则 checked/赋值 boolean 会 TS 报错）。
type EscalateKey = "escalateSafetyGuard" | "escalateUnverifiedProduct" | "escalateAiPolicyHold" | "escalateStuck";

const ESCALATE_FIELDS: { key: EscalateKey; label: string; hint: string }[] = [
  { key: "escalateSafetyGuard", label: "安全门拦截时", hint: "命中安全护栏被拦截，请示决策人定夺" },
  { key: "escalateUnverifiedProduct", label: "产品声明未经核验时", hint: "缺可核验知识支撑的产品声明，先请示再答复" },
  { key: "escalateAiPolicyHold", label: "AI 策略主动暂缓时", hint: "AI 依策略主动暂缓，交由决策人裁决" },
  { key: "escalateStuck", label: "对话停滞推不动时", hint: "对话长时间停滞，请示决策人介入推进" },
];

function AskHumanConfigView() {
  const toast = useToast();
  const [draft, setDraft] = useState<AskHumanPolicy>(defaultPolicy());
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    setLoaded(false);
    try {
      const res = await api.get<{ item: unknown }>(`/api/operation-domains/${DOMAIN}`);
      setDraft(extractPolicy(res.item));
      setLoaded(true);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 静默时段三格：任一改动重建 quietHours；三格全空则删除 quietHours。
  function setQuietHour(field: "startHour" | "endHour" | "tzOffsetHours", raw: string) {
    setDraft((d) => {
      const cur = d.quietHours ?? { startHour: NaN, endHour: NaN, tzOffsetHours: NaN };
      const next = { ...cur, [field]: raw.trim() === "" ? NaN : Number(raw) };
      const allEmpty = Number.isNaN(next.startHour) && Number.isNaN(next.endHour) && Number.isNaN(next.tzOffsetHours);
      const copy = { ...d };
      if (allEmpty) {
        delete copy.quietHours;
      } else {
        // 仅当三格都是有效数字才落 quietHours，否则保留编辑中态（用 0 占位避免 NaN 进 body）。
        copy.quietHours = {
          startHour: Number.isNaN(next.startHour) ? 0 : next.startHour,
          endHour: Number.isNaN(next.endHour) ? 0 : next.endHour,
          tzOffsetHours: Number.isNaN(next.tzOffsetHours) ? 0 : next.tzOffsetHours,
        };
      }
      return copy;
    });
  }

  async function save() {
    if (!loaded || loadError) {
      toast.error("现有配置尚未成功读取，禁止覆盖保存");
      return;
    }
    const errs = validatePolicy(draft);
    if (errs.length > 0) {
      toast.error(errs[0]);
      return;
    }
    setSaving(true);
    try {
      await api.put(`/api/operation-domains/${DOMAIN}/ask-human-policy`, draft);
      toast.success("已保存");
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
      // 保存失败草稿不丢
    } finally {
      setSaving(false);
    }
  }

  // 可选数值字段：空字符串 → 删除该键（undefined）；有值 → number。
  function setNumField(key: "dedupeWindowHours" | "dailyPushCap" | "timeoutHours", raw: string) {
    setDraft((d) => {
      const next = { ...d };
      if (raw.trim() === "") {
        delete next[key];
      } else {
        const n = Number(raw);
        if (Number.isFinite(n)) next[key] = n;
      }
      return next;
    });
  }

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>请示通道配置</h1>
        <button type="button" className={styles.saveBtn} onClick={() => void save()} disabled={saving || loading || !loaded || Boolean(loadError)}>
          {saving ? "保存中…" : "保存"}
        </button>
      </header>

      {loadError && <div className={styles.loadError} role="alert">读取现有配置失败，已禁止保存以避免覆盖线上策略：{loadError}</div>}

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>决策人链</h2>
        <DeciderChainEditor chain={draft.deciderChain} onChange={(c) => setDraft((d) => ({ ...d, deciderChain: c }))} />
        <div className={styles.chainHint}>清空决策人链并保存，即明确关闭请示通道。</div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>触发请示的情形</h2>
        {ESCALATE_FIELDS.map((f) => (
          <label key={f.key} className={styles.toggleRow}>
            <input
              type="checkbox"
              checked={Boolean(draft[f.key])}
              onChange={(e) => setDraft((d) => ({ ...d, [f.key]: e.target.checked }))}
            />
            <span className={styles.toggleLabel}>{f.label}</span>
            <span className={styles.toggleHint}>{f.hint}</span>
          </label>
        ))}
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>超时转备选</h2>
        <div className={styles.fieldRow}>
          <input
            className={styles.numInput}
            type="number"
            min={0}
            placeholder="不限"
            value={draft.timeoutHours ?? ""}
            onChange={(e) => setNumField("timeoutHours", e.target.value)}
          />
          <span className={styles.fieldUnit}>小时（留空=无限等待）</span>
        </div>
        {draft.quietHours && (
          <button
            type="button"
            className={styles.linkBtn}
            onClick={() => setDraft((current) => {
              const next = { ...current };
              delete next.quietHours;
              return next;
            })}
          >
            清除静默时段
          </button>
        )}
        <div className={styles.chainHint}>主决策人多久没响应就转交链中下一位</div>
      </section>

      <details className={styles.advanced}>
        <summary className={styles.advancedSummary}>高级：推送频控</summary>
        <div className={styles.fieldRow}>
          <span className={styles.fieldLabel}>去重窗口</span>
          <input className={styles.numInput} type="number" min={0} placeholder="不去重"
            value={draft.dedupeWindowHours ?? ""} onChange={(e) => setNumField("dedupeWindowHours", e.target.value)} />
          <span className={styles.fieldUnit}>小时</span>
        </div>
        <div className={styles.fieldRow}>
          <span className={styles.fieldLabel}>每日上限</span>
          <input className={styles.numInput} type="number" min={1} placeholder="不限"
            value={draft.dailyPushCap ?? ""} onChange={(e) => setNumField("dailyPushCap", e.target.value)} />
          <span className={styles.fieldUnit}>条</span>
        </div>
        <div className={styles.fieldRow}>
          <span className={styles.fieldLabel}>静默时段</span>
          <input className={styles.numInputSm} type="number" min={0} max={23} placeholder="起"
            value={draft.quietHours?.startHour ?? ""}
            onChange={(e) => setQuietHour("startHour", e.target.value)} />
          <span className={styles.fieldUnit}>~</span>
          <input className={styles.numInputSm} type="number" min={0} max={23} placeholder="止"
            value={draft.quietHours?.endHour ?? ""}
            onChange={(e) => setQuietHour("endHour", e.target.value)} />
          <span className={styles.fieldUnit}>时区</span>
          <input className={styles.numInputSm} type="number" placeholder="+8"
            value={draft.quietHours?.tzOffsetHours ?? ""}
            onChange={(e) => setQuietHour("tzOffsetHours", e.target.value)} />
        </div>
        <div className={styles.chainHint}>三项留空=全天可推；静默时段三格须同时填</div>
      </details>
    </div>
  );
}

export default function AskHumanConfigFeature() {
  return (
    <ConfirmProvider>
      <ToastProvider>
        <AskHumanConfigView />
      </ToastProvider>
    </ConfirmProvider>
  );
}
