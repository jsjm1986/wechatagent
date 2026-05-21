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
// 后端路由：
//   GET  /api/evolution/experiments?limit=20
//   GET  /api/evolution/proposals/:id
//   POST /api/evolution/proposals/:id/release   body { confirmation: "RELEASE"  }
//   POST /api/evolution/proposals/:id/rollback  body { confirmation: "ROLLBACK" }

import { useEffect, useMemo, useState } from "react";

// ── API helper（不复用 App.tsx 的 module-scoped 实例，方便单测局部 mock fetch） ──

async function apiGet<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

async function apiPost<T>(url: string, body?: unknown): Promise<T> {
  const r = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
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
      <div className="evolutionEmpty" data-testid="evolution-disabled">
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
    <section className="evolutionCenter" data-testid="evolution-center">
      <header className="evolutionAggregate">
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

      <div className="evolutionToolbar">
        <button onClick={() => void load()} disabled={loading}>
          {loading ? "加载中" : "刷新"}
        </button>
      </div>

      {error && (
        <div className="error" role="alert">
          {error}
        </div>
      )}

      <ProposalList
        proposals={proposalsFlat}
        selectedId={selectedProposalId}
        onSelect={(id) => setSelectedProposalId(id)}
      />

      {selectedProposalId && (
        <ProposalDetailView
          proposalId={selectedProposalId}
          onClose={() => setSelectedProposalId(null)}
          onActionDone={() => {
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
    <div className="metric-card" data-testid={testid}>
      <div className="metric-label">{label}</div>
      <div className="metric-value">{value}</div>
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
    return <p data-testid="proposal-list-empty">最近 N 个 experiment 还没有候选。</p>;
  }
  return (
    <table className="evolutionProposalList" data-testid="proposal-list">
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
  return (
    <span
      className={`badge badge-${statusTone(status)}`}
      data-testid={`status-badge-${status}`}
      data-tone={statusTone(status)}
    >
      {statusLabel(status)}
    </span>
  );
}

function ProposalDetailView({
  proposalId,
  onClose,
  onActionDone,
}: {
  proposalId: string;
  onClose: () => void;
  onActionDone: () => void;
}) {
  const [data, setData] = useState<ProposalDetailResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>("");
  const [modal, setModal] = useState<null | "release" | "rollback">(null);

  async function load() {
    setLoading(true);
    setError("");
    try {
      const d = await apiGet<ProposalDetailResponse>(`/api/evolution/proposals/${proposalId}`);
      setData(d);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [proposalId]);

  if (loading) {
    return (
      <aside className="evolutionDetail" data-testid="proposal-detail-loading">
        加载中…
      </aside>
    );
  }
  if (error) {
    return (
      <aside className="evolutionDetail" data-testid="proposal-detail-error">
        <div className="error" role="alert">
          {error}
        </div>
        <button onClick={onClose}>关闭</button>
      </aside>
    );
  }
  if (!data) return null;

  const { proposal, shadowReplays } = data;
  const releaseEnabled = proposal.status === "eligible_for_release";
  const rollbackEnabled = proposal.status === "released";

  return (
    <aside className="evolutionDetail" data-testid="proposal-detail">
      <header>
        <h3>{proposal.kind === "threshold" ? "阈值候选" : "Prompt 候选"} 详情</h3>
        <button onClick={onClose}>关闭</button>
      </header>

      <div>
        <StatusBadge status={proposal.status} />
        {proposal.failureReason && (
          <p className="failureReason" data-testid="failure-reason">
            未通过原因：{proposal.failureReason}
          </p>
        )}
      </div>

      {proposal.kind === "threshold" ? (
        <ThresholdDiffView proposal={proposal} currentState={data.currentState} />
      ) : (
        <PromptDiffView proposal={proposal} currentState={data.currentState} />
      )}

      <ShadowEvalReport summary={shadowReplays} proposal={proposal} />

      <footer className="evolutionDetailActions">
        <button
          onClick={() => setModal("release")}
          disabled={!releaseEnabled}
          data-testid="release-button"
        >
          发布
        </button>
        <button
          onClick={() => setModal("rollback")}
          disabled={!rollbackEnabled}
          data-testid="rollback-button"
        >
          回滚
        </button>
      </footer>

      {modal === "release" && (
        <ConfirmModal
          kind="release"
          proposalId={proposal.id ?? proposalId}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            onActionDone();
          }}
        />
      )}
      {modal === "rollback" && (
        <ConfirmModal
          kind="rollback"
          proposalId={proposal.id ?? proposalId}
          onClose={() => setModal(null)}
          onDone={() => {
            setModal(null);
            onActionDone();
          }}
        />
      )}
    </aside>
  );
}

function ThresholdDiffView({
  proposal,
  currentState,
}: {
  proposal: ProposalDetail;
  currentState: Record<string, unknown>;
}) {
  const cur = (currentState["currentValue"] ?? null) as number | null;
  const proposed = proposal.proposedValue;
  const cohort = (proposal.cohortNotes ?? {}) as Record<string, unknown>;
  const hitRate = (cohort["hit_rate_observed"] ?? cohort["hitRateObserved"] ?? null) as
    | number
    | null;
  return (
    <section className="thresholdDiff" data-testid="threshold-diff">
      <table className="thresholdTable">
        <tbody>
          <tr>
            <th>Gate Key</th>
            <td data-testid="threshold-gate-key">{proposal.gateKey ?? "—"}</td>
          </tr>
          <tr>
            <th>当前生效值</th>
            <td data-testid="threshold-current">{formatNumber(cur)}</td>
          </tr>
          <tr>
            <th>候选值</th>
            <td data-testid="threshold-proposed">{formatNumber(proposed)}</td>
          </tr>
          <tr>
            <th>cohort 命中率</th>
            <td data-testid="threshold-hit-rate">{formatPercent(hitRate)}</td>
          </tr>
        </tbody>
      </table>
    </section>
  );
}

function PromptDiffView({
  proposal,
  currentState,
}: {
  proposal: ProposalDetail;
  currentState: Record<string, unknown>;
}) {
  const currentText = (currentState["currentSectionText"] ??
    currentState["current_section_text"] ??
    "") as string;
  const proposedText = proposal.diffSnippet ?? "";
  const expected = proposal.expectedImprovementOn ?? [];
  return (
    <section className="promptDiff" data-testid="prompt-diff">
      <div className="promptDiffPanes">
        <div data-testid="prompt-diff-current">
          <h4>当前内容</h4>
          <pre>{currentText || "(空)"}</pre>
        </div>
        <div data-testid="prompt-diff-proposed">
          <h4>候选内容</h4>
          <pre>{proposedText || "(空)"}</pre>
        </div>
      </div>
      {proposal.criticReasoning && (
        <div data-testid="critic-reasoning">
          <h4>Critic 推理</h4>
          <p>{proposal.criticReasoning}</p>
        </div>
      )}
      {expected.length > 0 && (
        <div className="expectedImprovementTags" data-testid="expected-improvement">
          {expected.map((tag) => (
            <span key={tag} className="tag">
              {tag}
            </span>
          ))}
        </div>
      )}
    </section>
  );
}

function ShadowEvalReport({
  summary,
  proposal,
}: {
  summary: ShadowReplaysSummary;
  proposal: ProposalDetail;
}) {
  return (
    <section className="shadowEval" data-testid="shadow-eval">
      <h4>Shadow 评测</h4>
      <div className="shadowEvalGrid">
        <div data-testid="shadow-completed">
          <span>完成</span>
          <strong>{summary.totalCompleted}</strong>
        </div>
        <div data-testid="shadow-failed">
          <span>失败</span>
          <strong>{summary.totalFailed}</strong>
        </div>
        <div data-testid="shadow-significance">
          <span>显著性</span>
          <strong>
            {proposal.significancePassed === null
              ? "—"
              : proposal.significancePassed
              ? "通过"
              : "未通过"}
          </strong>
        </div>
      </div>
      {summary.samples.length > 0 && (
        <details>
          <summary>样本（前 5 条）</summary>
          <table>
            <thead>
              <tr>
                <th>source_run_id</th>
                <th>原 final_review</th>
                <th>新 final_review</th>
                <th>tokens</th>
              </tr>
            </thead>
            <tbody>
              {summary.samples.map((s) => (
                <tr key={s.id ?? s.sourceRunId}>
                  <td>{s.sourceRunId}</td>
                  <td>{s.originalFinalReviewStatus ?? "—"}</td>
                  <td>{s.newFinalReviewStatus ?? "—"}</td>
                  <td>{s.newTokenCost ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
    </section>
  );
}

// ── 确认弹窗 ──

const RELEASE_LITERAL = "RELEASE";
const ROLLBACK_LITERAL = "ROLLBACK";

export function ConfirmModal({
  kind,
  proposalId,
  onClose,
  onDone,
}: {
  kind: "release" | "rollback";
  proposalId: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const literal = kind === "release" ? RELEASE_LITERAL : ROLLBACK_LITERAL;
  const verb = kind === "release" ? "发布" : "回滚";
  const [text, setText] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [err, setErr] = useState<string>("");

  const matches = text === literal;

  async function submit() {
    if (!matches || submitting) return;
    setSubmitting(true);
    setErr("");
    try {
      await apiPost(`/api/evolution/proposals/${proposalId}/${kind}`, {
        confirmation: literal,
      });
      onDone();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="modalOverlay" data-testid={`confirm-modal-${kind}`}>
      <div className="modal">
        <h3>确认{verb}候选？</h3>
        <p>
          请输入 <code>{literal}</code> 以确认。任何不完全匹配的输入都会阻止提交。
        </p>
        <input
          type="text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={literal}
          data-testid={`confirm-input-${kind}`}
          autoFocus
        />
        {err && (
          <div className="error" role="alert">
            {err}
          </div>
        )}
        <footer>
          <button onClick={onClose} disabled={submitting}>
            取消
          </button>
          <button
            onClick={() => void submit()}
            disabled={!matches || submitting}
            data-testid={`confirm-submit-${kind}`}
          >
            {submitting ? `${verb}中…` : `确认${verb}`}
          </button>
        </footer>
      </div>
    </div>
  );
}
