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
// 保留既有 re-export 路径（root src/EvolutionCenterTab.tsx → 此处 → 卡）：单测 import { ConfirmModal }
// from "../EvolutionCenterTab" 不受迁移影响。
export { ConfirmModal } from "../../components/review/ProposalReleaseCard";

// ── API helper（不复用 App.tsx 的 module-scoped 实例，方便单测局部 mock fetch） ──

async function apiGet<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

// ── 类型镜像后端 src/routes/evolution.rs 返回 schema ──

export type ProposalStatus =
  | "pending_eval"
  | "evaluating"
  | "eligible_for_release"
  | "rejected_below_threshold"
  | "released"
  | "rolled_back";

export type ProposalKind = "threshold" | "prompt";

export interface ExperimentEnvelope {
  experimentId: string;
  workspaceId: string;
  accountId: string;
  status: string;
  windowHours: number;
  startedAt: string;
  updatedAt: string;
  finishedAt: string | null;
  cohortThresholdSize: number;
  cohortPromptSize: number;
  budgetUsedTokens: number;
  budgetUsedCalls: number;
  proposalsCount: number;
  proposalsEligibleCount: number;
}

export interface ProposalSummary {
  id: string | null;
  kind: ProposalKind;
  status: ProposalStatus | string;
  gateKey: string | null;
  proposedTemplateKey: string | null;
  proposedSection: string | null;
  currentValue: number | null;
  proposedValue: number | null;
  significancePassed: boolean | null;
  evalReplaysCompleted: number | null;
  evalReplaysFailed: number | null;
  failureReason: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ExperimentItem {
  experiment: ExperimentEnvelope;
  proposalsCounts: Record<string, number>;
  proposals: ProposalSummary[];
}

export interface ExperimentsResponse {
  items: ExperimentItem[];
}

export interface ShadowReplaySample {
  id: string | null;
  sourceRunId: string;
  status: string;
  failureReason: string | null;
  originalFinalReviewStatus: string | null;
  newFinalReviewStatus: string | null;
  newReviewRisks: unknown;
  newTokenCost: number | null;
  new5gateHit: Record<string, unknown>;
  newSelfCritiqueAddressed: boolean | null;
  similarityToOriginalText: number | null;
  startedAt: string;
  finishedAt: string | null;
}

export interface ShadowReplaysSummary {
  totalCompleted: number;
  totalFailed: number;
  samples: ShadowReplaySample[];
}

export interface ProposalDetail {
  id: string | null;
  experimentId: string;
  workspaceId: string;
  accountId: string;
  kind: ProposalKind;
  status: ProposalStatus | string;
  gateKey: string | null;
  currentValue: number | null;
  proposedValue: number | null;
  cohortNotes: Record<string, unknown>;
  proposedTemplateKey: string | null;
  proposedSection: string | null;
  diffSummary: string | null;
  diffSnippet: string | null;
  criticReasoning: string | null;
  expectedImprovementOn: string[] | null;
  riskNote: string | null;
  previousPromptVersion: string | null;
  evalMetrics: Record<string, unknown>;
  evalReplaysCompleted: number | null;
  evalReplaysFailed: number | null;
  significancePassed: boolean | null;
  failureReason: string | null;
  releasedAt: string | null;
  releasedBy: string | null;
  rolledBackAt: string | null;
  rolledBackBy: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProposalDetailResponse {
  proposal: ProposalDetail;
  experiment: ExperimentEnvelope | null;
  cohortRunIds: string[];
  shadowReplays: ShadowReplaysSummary;
  currentState: Record<string, unknown>;
}

// ── 工具函数 ──

const STATUS_LABELS: Record<string, string> = {
  pending_eval: "待评测",
  evaluating: "评测中",
  eligible_for_release: "可发布",
  rejected_below_threshold: "未达标",
  released: "已发布",
  rolled_back: "已回滚",
};

const STATUS_TONES: Record<string, string> = {
  pending_eval: "neutral",
  evaluating: "info",
  eligible_for_release: "success",
  rejected_below_threshold: "warn",
  released: "primary",
  rolled_back: "danger",
};

// tone → CSS Module 徽章类（保留 data-tone 原值供测试断言；class 走局部化）。
const TONE_CLASS: Record<string, string> = {
  neutral: styles.badgeNeutral,
  info: styles.badgeInfo,
  success: styles.badgeSuccess,
  warn: styles.badgeWarn,
  primary: styles.badgePrimary,
  danger: styles.badgeDanger,
};

export function statusLabel(s: string): string {
  return STATUS_LABELS[s] ?? s;
}

export function statusTone(s: string): string {
  return STATUS_TONES[s] ?? "neutral";
}

export function formatNumber(v: number | null | undefined, digits = 2): string {
  if (v === null || v === undefined || Number.isNaN(v)) return "—";
  return Number(v).toFixed(digits);
}

export function formatPercent(v: number | null | undefined): string {
  if (v === null || v === undefined || Number.isNaN(v)) return "—";
  return `${(v * 100).toFixed(1)}%`;
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

  async function load() {
    if (!enabled) return;
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
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);

  if (!enabled) {
    return (
      <div className={styles.disabled} data-testid="evolution-disabled">
        演化器未启用（EVOLUTION_ENABLED=false）。启用后此处会展示自动产出的实验信封与候选。
      </div>
    );
  }

  const aggregate = useMemo(() => aggregateLast7Days(items), [items]);

  const proposalsFlat = useMemo<ProposalSummary[]>(
    () => items.flatMap((it) => it.proposals).sort((a, b) => b.createdAt.localeCompare(a.createdAt)),
    [items]
  );

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

      <div className={styles.toolbar}>
        <button className={styles.btnGhost} onClick={() => void load()} disabled={loading}>
          {loading ? "加载中" : "刷新"}
        </button>
      </div>

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
    return <p className={styles.proposalEmpty} data-testid="proposal-list-empty">最近 N 个 experiment 还没有候选。</p>;
  }
  return (
    <table className={styles.proposalList} data-testid="proposal-list">
      <thead>
        <tr>
          <th>状态</th>
          <th>类型</th>
          <th>主题</th>
          <th>显著性</th>
          <th>Replays</th>
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
            <td>{p.kind === "threshold" ? "阈值" : "Prompt"}</td>
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

export function StatusBadge({ status }: { status: string }) {
  const tone = statusTone(status);
  return (
    <span
      className={`${styles.badge} ${TONE_CLASS[tone] ?? styles.badgeNeutral}`}
      data-testid={`status-badge-${status}`}
      data-tone={tone}
    >
      {statusLabel(status)}
    </span>
  );
}
