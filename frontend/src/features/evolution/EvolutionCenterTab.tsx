// agent-self-evolution M4 W4 Task 5.7：演化中心 Tab。
//
// 三层结构：
//   1) 聚合卡：最近 7 天 experiments / proposals / released / rolled_back / 显著性通过率
//   2) Proposal 列表：status 徽章 + shadow eval 摘要
//   3) ProposalDetail 展开：threshold 类 = current vs proposed 数值条 + hit_rate；
//      prompt 类 = 双栏 diff（current_section_text | proposed_section_text）+
//      Critic reasoning + expectedImprovementOn 标签 + shadow eval 报告卡
//
// [发布] / [回滚] 按钮按 status 启用/置灰。ReleaseModal 必须输入 "RELEASE" 才启用确认；
// RollbackModal 必须输入 "ROLLBACK"。
//
// 文案严守 AI 自主语义。仓库根 scripts/ 下的 CI lint 会在 PR 阻断任何回归到非
// AI 自主表达的文案。
//
// 视觉：apple-liquid-alive 白卡基调，样式经 CSS Modules 局部化（EvolutionCenterTab.module.css），
// 不再依赖全局 styles.css。所有 data-testid / 文案 / data-tone 与既有单测一一镜像，保持不变。
//
// 后端路由：
//   GET  /api/evolution/experiments?limit=20
//   GET  /api/evolution/proposals/:id
//   POST /api/evolution/proposals/:id/release   body { confirmation: "RELEASE"  }
//   POST /api/evolution/proposals/:id/rollback  body { confirmation: "ROLLBACK" }

import { useEffect, useMemo, useState } from "react";
import styles from "./EvolutionCenterTab.module.css";
// 候选「发布/回滚」卡已中立化迁入 components/review/（Ask-Human Phase 2 Task 6）。
// 老页改薄壳：只持列表/选中逻辑 + 共享原语（StatusBadge/formatNumber/...），详情卡复用迁出件。
import { ProposalReleaseCard } from "../../components/review/ProposalReleaseCard";
// 共享原语/类型已提升到 components/review/ 中立家（零跨feature修订，用户裁定 B）：
// 老页与卡片同源 import 同一套定义；老页继续以原签名使用，渲染字节级不变。
import {
  StatusBadge,
  statusLabel,
  statusTone,
  formatNumber,
  formatPercent,
} from "../../components/review/proposalPrimitives";
import type {
  ProposalStatus,
  ProposalKind,
  ExperimentEnvelope,
  ProposalSummary,
  ExperimentItem,
  ExperimentsResponse,
  ShadowReplaySample,
  ShadowReplaysSummary,
  ProposalDetail,
  ProposalDetailResponse,
} from "../../components/review/proposalTypes";
// 保留既有 re-export 路径（root src/EvolutionCenterTab.tsx → 此处 → 卡/中立家）：
// 单测 import { ConfirmModal, StatusBadge, ... } from "../EvolutionCenterTab" 不受迁移影响。
export { ConfirmModal } from "../../components/review/ProposalReleaseCard";
export { StatusBadge, statusLabel, statusTone, formatNumber, formatPercent };
export type {
  ProposalStatus,
  ProposalKind,
  ExperimentEnvelope,
  ProposalSummary,
  ExperimentItem,
  ExperimentsResponse,
  ShadowReplaySample,
  ShadowReplaysSummary,
  ProposalDetail,
  ProposalDetailResponse,
};

// ── API helper（不复用 App.tsx 的 module-scoped 实例，方便单测局部 mock fetch） ──

async function apiGet<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

async function apiPut<T>(url: string, body: unknown): Promise<T> {
  const r = await fetch(url, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

// runtime-flag GET/PUT 同形（camelCase）；server 钳 rolloutPercent ≤ 100。
export interface RuntimeFlag {
  enabled: boolean;
  rolloutPercent: number;
}

// 后端响应把配置体放在 .flag 子对象里（未配置时 flag === null）：
//   GET  → { workspaceId, envEvolutionEnabled, flag: { enabled, rolloutPercent, ... } | null }
//   PUT  → { ok, flag: { enabled, rolloutPercent, ... } }
// 读回必须从 .flag 内层取；flag 为 null（未配置）时回落逻辑默认 false/0。
export interface RuntimeFlagResponse {
  // GET 返回；PUT 响应无此字段，故可选。true=env 允许 UI 开启；false=运维硬锁定。
  envEvolutionEnabled?: boolean;
  flag: RuntimeFlag | null;
}

// 阈值变更不可变审计行（release / rollback / auto-release）。后端字段为 camelCase，
// 以宽松类型读取主要列（gateKey / action / decidedBy / decidedAt / value transition）。
export interface ThresholdAuditRow {
  id?: string | null;
  gateKey?: string | null;
  action?: string | null;
  previousValue?: number | null;
  newValue?: number | null;
  sourceProposalId?: string | null;
  decidedBy?: string | null;
  decidedAt?: string | null;
  hitRateObserved?: number | null;
  [k: string]: unknown;
}

// 阈值审计动作 → 中文（后端 action 闭集：released / rolled_back / auto_released，models.rs:4563）。
function auditActionLabel(action?: string | null): string {
  switch (action) {
    case "released":
      return "已发布";
    case "rolled_back":
      return "已回滚";
    case "auto_released":
      return "自动发布";
    default:
      return action ?? "—";
  }
}

/// 7 天聚合（client 端从 experiments[] 推算 — 不打额外请求；后端尚未提供专用聚合 endpoint）。
export function aggregateLast7Days(items: ExperimentItem[]): {
  experiments: number;
  proposals: number;
  released: number;
  rolledBack: number;
  significancePassRate: number | null;
} {
  const cutoff = Date.now() - 7 * 24 * 60 * 60 * 1000;
  let experiments = 0;
  let proposals = 0;
  let released = 0;
  let rolledBack = 0;
  let evaluated = 0;
  let passed = 0;
  for (const item of items) {
    const startedMs = Date.parse(item.experiment.startedAt);
    if (Number.isNaN(startedMs) || startedMs < cutoff) continue;
    experiments += 1;
    proposals += item.proposals.length;
    for (const p of item.proposals) {
      if (p.status === "released") released += 1;
      if (p.status === "rolled_back") rolledBack += 1;
      if (p.significancePassed !== null) {
        evaluated += 1;
        if (p.significancePassed === true) passed += 1;
      }
    }
  }
  return {
    experiments,
    proposals,
    released,
    rolledBack,
    significancePassRate: evaluated === 0 ? null : passed / evaluated,
  };
}

// ── 主组件 ──

export function EvolutionCenterTab({ enabled = true }: { enabled?: boolean }) {
  const [items, setItems] = useState<ExperimentItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>("");
  const [selectedProposalId, setSelectedProposalId] = useState<string | null>(null);

  // ── runtime-flag 灰度控件（workspace 级 enabled + rolloutPercent 0-100）──
  const [flagEnabled, setFlagEnabled] = useState(false);
  const [envAllowed, setEnvAllowed] = useState<boolean | null>(null);
  const [rollout, setRollout] = useState<string>("0");
  const [flagBusy, setFlagBusy] = useState(false);
  const [flagMsg, setFlagMsg] = useState<string>("");
  const [flagError, setFlagError] = useState<string>("");

  async function loadFlag() {
    setFlagBusy(true);
    setFlagMsg("");
    setFlagError("");
    try {
      const resp = await apiGet<RuntimeFlagResponse>("/api/evolution/runtime-flag");
      setEnvAllowed(resp.envEvolutionEnabled !== false); // 缺省按允许；显式 false 才硬锁
      setFlagEnabled(Boolean(resp.flag?.enabled ?? false));
      setRollout(String(resp.flag?.rolloutPercent ?? 0));
    } catch (e) {
      // 拉取失败必须落到可见错误态，否则 envAllowed 永远 null → 卡在"加载中"且错误无处显示。
      setFlagError(e instanceof Error ? e.message : String(e));
    } finally {
      setFlagBusy(false);
    }
  }

  async function saveFlag() {
    setFlagBusy(true);
    setFlagMsg("");
    try {
      // 开=全量：enabled 时若高级灰度值为 0 则按 100 全量；关时保留原 rollout 值。
      const advanced = Math.max(0, Math.min(100, Number(rollout) || 0));
      const pct = flagEnabled ? (advanced === 0 ? 100 : advanced) : advanced;
      const resp = await apiPut<RuntimeFlagResponse>("/api/evolution/runtime-flag", {
        enabled: flagEnabled,
        rolloutPercent: pct,
      });
      setFlagEnabled(Boolean(resp.flag?.enabled ?? flagEnabled));
      setRollout(String(resp.flag?.rolloutPercent ?? pct));
      setFlagMsg("演化中心总开关已保存");
    } catch (e) {
      setFlagMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setFlagBusy(false);
    }
  }

  async function saveFlagWith(nextEnabled: boolean) {
    setFlagBusy(true);
    setFlagMsg("");
    try {
      const advanced = Math.max(0, Math.min(100, Number(rollout) || 0));
      const pct = nextEnabled ? (advanced === 0 ? 100 : advanced) : advanced;
      const resp = await apiPut<RuntimeFlagResponse>("/api/evolution/runtime-flag", {
        enabled: nextEnabled,
        rolloutPercent: pct,
      });
      setFlagEnabled(Boolean(resp.flag?.enabled ?? nextEnabled));
      setRollout(String(resp.flag?.rolloutPercent ?? pct));
      setFlagMsg("演化中心总开关已保存");
    } catch (e) {
      setFlagMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setFlagBusy(false);
    }
  }

  // ── 阈值变更审计日志（点按钮加载，与 runtime-flag 同模式，挂载期不自动 GET）──
  const [auditRows, setAuditRows] = useState<ThresholdAuditRow[]>([]);
  const [auditLoaded, setAuditLoaded] = useState(false);
  const [auditBusy, setAuditBusy] = useState(false);
  const [auditError, setAuditError] = useState<string>("");

  async function loadAudit() {
    setAuditBusy(true);
    setAuditError("");
    try {
      const data = await apiGet<{ items: ThresholdAuditRow[] }>(
        "/api/evolution/threshold-overrides/audit",
      );
      setAuditRows(data.items || []);
      setAuditLoaded(true);
    } catch (e) {
      setAuditError(e instanceof Error ? e.message : String(e));
    } finally {
      setAuditBusy(false);
    }
  }

  async function load() {
    if (!enabled || envAllowed === false || !flagEnabled) return;
    setLoading(true);
    setError("");
    try {
      const data = await apiGet<ExperimentsResponse>("/api/evolution/experiments?limit=20");
      setItems(data.items || []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadFlag();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, envAllowed, flagEnabled]);

  // useMemo 必须在任何早返回之前调用——hooks 调用顺序在每次渲染必须一致，
  // 否则 envAllowed null→bool 跳变时早返回路径的 hook 数量不同 → React 崩溃。
  const aggregate = useMemo(() => aggregateLast7Days(items), [items]);

  const proposalsFlat = useMemo<ProposalSummary[]>(
    () => items.flatMap((it) => it.proposals).sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
    [items]
  );

  const locked = !enabled || envAllowed === false;
  if (locked) {
    return (
      <div className={styles.disabled} data-testid="evolution-disabled">
        演化中心已被运维硬锁定（EVOLUTION_ENABLED=false），请联系运维解除后再在此开启。
      </div>
    );
  }
  if (envAllowed === null) {
    if (flagError) {
      return (
        <div className={styles.disabled} data-testid="evolution-flag-error">
          <div className={styles.error} role="alert">
            读取演化中心开关失败：{flagError}
          </div>
          <button className={styles.btnGhost} onClick={() => void loadFlag()} disabled={flagBusy}>
            重试
          </button>
        </div>
      );
    }
    return (
      <div className={styles.disabled} data-testid="evolution-flag-loading">
        加载中…
      </div>
    );
  }

  return (
    <section className={styles.center} data-testid="evolution-center">
      <header className={styles.aggregate}>
        <AggregateCard label="近 7 天实验" value={aggregate.experiments} testid="agg-experiments" />
        <AggregateCard label="候选总数" value={aggregate.proposals} testid="agg-proposals" />
        <AggregateCard label="已发布" value={aggregate.released} testid="agg-released" />
        <AggregateCard label="已回滚" value={aggregate.rolledBack} testid="agg-rolled-back" />
        <AggregateCard
          label="显著性通过率"
          value={formatPercent(aggregate.significancePassRate)}
          testid="agg-significance"
        />
      </header>

      <div className={styles.flagPanel} data-testid="runtime-flag-panel">
        <div className={styles.flagRow}>
          <label className={styles.flagToggle}>
            <input
              type="checkbox"
              checked={flagEnabled}
              onChange={(e) => {
                setFlagEnabled(e.target.checked);
                // 状态更新后保存：用新值直接 PUT，避免读到旧 state。
                void saveFlagWith(e.target.checked);
              }}
              disabled={flagBusy}
            />
            <span>演化中心总开关</span>
          </label>
          <details className={styles.advanced}>
            <summary>高级设置（灰度比例）</summary>
            <label className={styles.flagField}>
              <span>灰度比例（%）</span>
              <input
                type="number"
                min={0}
                max={100}
                value={rollout}
                onChange={(e) => setRollout(e.target.value)}
                disabled={flagBusy}
              />
            </label>
          </details>
          <button className={styles.btnGhost} onClick={() => void loadFlag()} disabled={flagBusy}>
            读取当前配置
          </button>
          <button className={styles.btnPrimary} onClick={() => void saveFlag()} disabled={flagBusy}>
            保存灰度
          </button>
        </div>
        {flagMsg && (
          <div className={styles.flagMsg} data-testid="runtime-flag-msg">
            {flagMsg}
          </div>
        )}
      </div>

      <div className={styles.toolbar}>
        <button className={styles.btnGhost} onClick={() => void load()} disabled={loading}>
          {loading ? "加载中" : "刷新"}
        </button>
        <button
          className={styles.btnGhost}
          onClick={() => void loadAudit()}
          disabled={auditBusy}
          data-testid="threshold-audit-load"
        >
          {auditBusy ? "加载中" : "阈值变更审计"}
        </button>
      </div>

      {(auditLoaded || auditError) && (
        <div className={styles.auditPanel} data-testid="threshold-audit-panel">
          {auditError && (
            <div className={styles.error} role="alert">
              {auditError}
            </div>
          )}
          {auditLoaded && auditRows.length === 0 && !auditError && (
            <p className={styles.proposalEmpty} data-testid="threshold-audit-empty">
              暂无审计记录。
            </p>
          )}
          {auditRows.length > 0 && (
            <table className={styles.proposalList} data-testid="threshold-audit-table">
              <thead>
                <tr>
                  <th>动作</th>
                  <th>阈值项</th>
                  <th>值变更</th>
                  <th>操作者</th>
                  <th>时间</th>
                </tr>
              </thead>
              <tbody>
                {auditRows.map((row, idx) => (
                  <tr
                    key={row.id ?? `${row.decidedAt ?? "row"}-${idx}`}
                    data-testid={`threshold-audit-row-${row.id ?? idx}`}
                  >
                    <td>{auditActionLabel(row.action)}</td>
                    <td>{row.gateKey ?? "—"}</td>
                    <td>
                      {formatNumber(row.previousValue ?? null)} → {formatNumber(row.newValue ?? null)}
                    </td>
                    <td>{row.decidedBy ?? "—"}</td>
                    <td>{row.decidedAt ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}

      {error && (
        <div className={styles.error} role="alert">
          {error}
        </div>
      )}

      <ProposalList
        proposals={proposalsFlat}
        selectedId={selectedProposalId}
        onSelect={(id) => setSelectedProposalId(id)}
      />

      {selectedProposalId && (
        <ProposalReleaseCard
          proposalId={selectedProposalId}
          onClose={() => setSelectedProposalId(null)}
          onDone={() => {
            setSelectedProposalId(null);
            void load();
          }}
        />
      )}
    </section>
  );
}

function AggregateCard({
  label,
  value,
  testid,
}: {
  label: string;
  value: number | string;
  testid: string;
}) {
  return (
    <div className={styles.metricCard} data-testid={testid}>
      <div className={styles.metricLabel}>{label}</div>
      <div className={styles.metricValue}>{value}</div>
    </div>
  );
}

function ProposalList({
  proposals,
  selectedId,
  onSelect,
}: {
  proposals: ProposalSummary[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  if (proposals.length === 0) {
    return <p className={styles.proposalEmpty} data-testid="proposal-list-empty">最近 N 个实验还没有候选。</p>;
  }
  return (
    <table className={styles.proposalList} data-testid="proposal-list">
      <thead>
        <tr>
          <th>状态</th>
          <th>类型</th>
          <th>主题</th>
          <th>显著性</th>
          <th>回放</th>
          <th>创建时间</th>
        </tr>
      </thead>
      <tbody>
        {proposals.map((p) => (
          <tr
            key={p.id ?? p.createdAt}
            data-testid={`proposal-row-${p.id ?? "no-id"}`}
            data-selected={p.id === selectedId ? "true" : "false"}
            onClick={() => p.id && onSelect(p.id)}
            style={{ cursor: p.id ? "pointer" : "default" }}
          >
            <td>
              <StatusBadge status={p.status} />
            </td>
            <td>{p.kind === "threshold" ? "阈值" : "提示词"}</td>
            <td>
              {p.kind === "threshold"
                ? `${p.gateKey ?? "—"}: ${formatNumber(p.currentValue)} → ${formatNumber(p.proposedValue)}`
                : `${p.proposedTemplateKey ?? "—"} / ${p.proposedSection ?? "—"}`}
            </td>
            <td>{p.significancePassed === null ? "—" : p.significancePassed ? "通过" : "未通过"}</td>
            <td>
              {p.evalReplaysCompleted ?? 0} / {(p.evalReplaysCompleted ?? 0) + (p.evalReplaysFailed ?? 0)}
            </td>
            <td>{p.createdAt}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

