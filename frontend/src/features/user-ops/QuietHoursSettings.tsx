import { useState } from "react";
import { Clock3, RefreshCw, Save, X } from "lucide-react";
import { Overlay } from "../../components/ui/Overlay";
import type { OperationDomainDraft } from "../../types";
import {
  QUIET_HOURS_COMPATIBILITY_DEFAULTS,
  quietHoursSettingsFromDraft,
  runtimeParametersWithQuietHours,
  type QuietHoursSettingsValue
} from "../../stores/userOpsDomainHelpers";
import styles from "./QuietHoursSettings.module.css";

const HOURS = Array.from({ length: 24 }, (_, hour) => hour);
const TIMEZONE_OFFSETS = Array.from({ length: 27 }, (_, index) => index - 12);

function hourLabel(hour: number) {
  return `${String(hour).padStart(2, "0")}:00`;
}

function timezoneLabel(offset: number) {
  const utc = offset === 0 ? "UTC" : `UTC${offset > 0 ? "+" : ""}${offset}`;
  return offset === 8 ? `${utc} 中国标准时间` : utc;
}

function triggerSummary(settings: QuietHoursSettingsValue) {
  return settings.enabled
    ? `${hourLabel(settings.startHour)}–${hourLabel(settings.endHour)}`
    : "已关闭";
}

export function quietHoursEffectText(settings: QuietHoursSettingsValue) {
  if (!settings.enabled) return "作息门控已关闭，Agent 全天正常处理新消息和主动跟进。";
  if (settings.startHour === settings.endHour) return "休息开始时间不能与醒来时间相同。";
  const period = settings.startHour > settings.endHour
    ? `每天 ${hourLabel(settings.startHour)} 至次日 ${hourLabel(settings.endHour)}`
    : `每天 ${hourLabel(settings.startHour)} 至 ${hourLabel(settings.endHour)}`;
  return `${period} 暂缓回复和主动跟进，醒来后恢复处理。`;
}

export function QuietHoursSettings({
  busy,
  draft,
  onReload,
  onSave
}: {
  busy: boolean;
  draft?: OperationDomainDraft;
  onReload: () => void;
  onSave: (draft: OperationDomainDraft) => Promise<boolean>;
}) {
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [form, setForm] = useState<QuietHoursSettingsValue>(QUIET_HOURS_COMPATIBILITY_DEFAULTS);
  const persisted = draft ? quietHoursSettingsFromDraft(draft) : null;

  const openDialog = () => {
    if (!draft || !persisted) return;
    setForm(persisted);
    setOpen(true);
  };
  const closeDialog = () => {
    if (!saving) setOpen(false);
  };
  const validationError = form.enabled && form.startHour === form.endHour;
  const save = async () => {
    if (!draft || validationError || saving) return;
    setSaving(true);
    try {
      const saved = await onSave({
        ...draft,
        runtimeParameters: runtimeParametersWithQuietHours(draft.runtimeParameters, form)
      });
      if (saved) setOpen(false);
    } finally {
      setSaving(false);
    }
  };

  if (!persisted) {
    return (
      <button type="button" className={styles.trigger} onClick={onReload} disabled={busy}>
        <RefreshCw size={15} />
        重新加载作息
      </button>
    );
  }

  return (
    <>
      <button
        type="button"
        className={styles.trigger}
        onClick={openDialog}
        disabled={busy}
        aria-haspopup="dialog"
      >
        <Clock3 size={15} />
        <span>作息</span>
        <strong>{triggerSummary(persisted)}</strong>
      </button>

      <Overlay open={open} onClose={closeDialog} labelledBy="quiet-hours-title" describedBy="quiet-hours-effect">
        <div className={styles.dialog}>
          <header className={styles.dialogHead}>
            <div>
              <span>Workspace 全局策略</span>
              <h3 id="quiet-hours-title">作息时间</h3>
            </div>
            <button type="button" className={styles.iconButton} onClick={closeDialog} disabled={saving} aria-label="关闭">
              <X size={17} />
            </button>
          </header>

          <label className={styles.toggleRow}>
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(event) => setForm((current) => ({ ...current, enabled: event.target.checked }))}
              disabled={saving}
            />
            <span>
              <strong>启用作息门控</strong>
              <small>休息时段暂停自动回复与主动跟进</small>
            </span>
          </label>

          <div className={styles.controls}>
            <label>
              <span>休息开始</span>
              <select
                aria-label="休息开始"
                value={form.startHour}
                onChange={(event) => setForm((current) => ({ ...current, startHour: Number(event.target.value) }))}
                disabled={saving || !form.enabled}
              >
                {HOURS.map((hour) => <option key={hour} value={hour}>{hourLabel(hour)}</option>)}
              </select>
            </label>
            <label>
              <span>醒来时间</span>
              <select
                aria-label="醒来时间"
                value={form.endHour}
                onChange={(event) => setForm((current) => ({ ...current, endHour: Number(event.target.value) }))}
                disabled={saving || !form.enabled}
              >
                {HOURS.map((hour) => <option key={hour} value={hour}>{hourLabel(hour)}</option>)}
              </select>
            </label>
            <label className={styles.timezoneField}>
              <span>时区</span>
              <select
                aria-label="时区"
                value={form.tzOffsetHours}
                onChange={(event) => setForm((current) => ({ ...current, tzOffsetHours: Number(event.target.value) }))}
                disabled={saving || !form.enabled}
              >
                {TIMEZONE_OFFSETS.map((offset) => (
                  <option key={offset} value={offset}>{timezoneLabel(offset)}</option>
                ))}
              </select>
            </label>
          </div>

          <div
            id="quiet-hours-effect"
            className={validationError ? styles.effectError : styles.effect}
            role={validationError ? "alert" : undefined}
          >
            {quietHoursEffectText(form)} 时区：{timezoneLabel(form.tzOffsetHours)}。
          </div>

          <section className={styles.behaviorNote} aria-label="生效说明">
            <strong>保存成功后立即生效，无需重启</strong>
            <p>后续新消息与任务下一次处理时使用新设置。</p>
            <p>已排队的醒来回复保留原执行时间；普通主动跟进若命中新休息时段，会顺延到新的醒来时间。</p>
          </section>

          <footer className={styles.actions}>
            <button type="button" className={styles.cancel} onClick={closeDialog} disabled={saving}>取消</button>
            <button type="button" className={styles.save} onClick={() => void save()} disabled={saving || validationError}>
              <Save size={16} />
              {saving ? "保存中" : "保存作息"}
            </button>
          </footer>
        </div>
      </Overlay>
    </>
  );
}
