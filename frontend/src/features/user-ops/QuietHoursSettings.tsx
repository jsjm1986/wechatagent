import { Clock3, RefreshCw, Save } from "lucide-react";
import type { OperationDomainDraft } from "../../types";
import {
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
  onChange,
  onReload,
  onSave
}: {
  busy: boolean;
  draft?: OperationDomainDraft;
  onChange: (draft: OperationDomainDraft) => void;
  onReload: () => void;
  onSave: (draft: OperationDomainDraft) => void;
}) {
  if (!draft) {
    return (
      <section className={`panel ${styles.panel}`} aria-label="作息时间">
        <div className={styles.loadingCopy}>
          <Clock3 size={18} />
          <div>
            <strong>作息时间</strong>
            <span>正在读取当前 workspace 的运行策略。</span>
          </div>
        </div>
        <button type="button" className="secondary" onClick={onReload} disabled={busy}>
          <RefreshCw size={16} />
          重新加载
        </button>
      </section>
    );
  }

  const settings = quietHoursSettingsFromDraft(draft);
  const validationError = settings.enabled && settings.startHour === settings.endHour;
  const update = (patch: Partial<QuietHoursSettingsValue>) => {
    const next = { ...settings, ...patch };
    onChange({
      ...draft,
      runtimeParameters: runtimeParametersWithQuietHours(draft.runtimeParameters, next)
    });
  };
  const save = () => {
    // Save all four fields even when an older config only displayed compatibility defaults.
    onSave({
      ...draft,
      runtimeParameters: runtimeParametersWithQuietHours(draft.runtimeParameters, settings)
    });
  };

  return (
    <section className={`panel ${styles.panel}`} aria-labelledby="quiet-hours-title">
      <div className={styles.heading}>
        <div className={styles.titleBlock}>
          <Clock3 size={19} />
          <div>
            <span>Workspace 全局策略</span>
            <h2 id="quiet-hours-title">作息时间</h2>
          </div>
        </div>
        <label className={styles.toggleRow}>
          <input
            type="checkbox"
            checked={settings.enabled}
            onChange={(event) => update({ enabled: event.target.checked })}
            disabled={busy}
          />
          <span>{settings.enabled ? "已启用" : "已关闭"}</span>
        </label>
      </div>

      <div className={styles.controls}>
        <label>
          <span>休息开始</span>
          <select
            aria-label="休息开始"
            value={settings.startHour}
            onChange={(event) => update({ startHour: Number(event.target.value) })}
            disabled={busy || !settings.enabled}
          >
            {HOURS.map((hour) => <option key={hour} value={hour}>{hourLabel(hour)}</option>)}
          </select>
        </label>
        <label>
          <span>醒来时间</span>
          <select
            aria-label="醒来时间"
            value={settings.endHour}
            onChange={(event) => update({ endHour: Number(event.target.value) })}
            disabled={busy || !settings.enabled}
          >
            {HOURS.map((hour) => <option key={hour} value={hour}>{hourLabel(hour)}</option>)}
          </select>
        </label>
        <label>
          <span>时区</span>
          <select
            aria-label="时区"
            value={settings.tzOffsetHours}
            onChange={(event) => update({ tzOffsetHours: Number(event.target.value) })}
            disabled={busy || !settings.enabled}
          >
            {TIMEZONE_OFFSETS.map((offset) => (
              <option key={offset} value={offset}>{timezoneLabel(offset)}</option>
            ))}
          </select>
        </label>
        <button type="button" onClick={save} disabled={busy || validationError}>
          <Save size={16} />
          保存作息
        </button>
      </div>

      <div className={validationError ? styles.effectError : styles.effect} role={validationError ? "alert" : undefined}>
        {quietHoursEffectText(settings)} 时区：{timezoneLabel(settings.tzOffsetHours)}。
      </div>
      <p className={styles.notice}>
        保存只影响后续新消息和后续主动跟进；已经排队的延迟回复仍按原计划执行。
      </p>
    </section>
  );
}
