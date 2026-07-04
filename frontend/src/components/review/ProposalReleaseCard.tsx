// Ask-Human Phase 2 Task 6：演化候选「发布/回滚」卡。源自演化中心 Tab
// 物理迁入（原 ProposalDetailView + 它依赖的 ConfirmModal/ThresholdDiffView/PromptDiffView/
// ShadowEvalReport）。中立化到 components/review/ 后，老页（演化中心 Tab）与统一收件箱频道
// 都从这里 import，不再各持一份定义。
//
// 渲染/确认逻辑逐行保留：threshold 类 = current vs proposed 数值条 + hit_rate；prompt 类 = 双栏
// diff + Critic reasoning + expectedImprovementOn 标签 + shadow eval 报告卡；[发布]/[回滚] 按 status
// 启用/置灰；ConfirmModal 必须输入 "RELEASE"/"ROLLBACK" 才启用确认。
//
// 数据获取统一走 lib/api 的 api.get/api.post（非 2xx 抛 parseApiError），替换原私有 apiGet/apiPost。
//
// 文案严守 AI 自主语义。仓库根 scripts/ 下的 CI lint 会在 PR 阻断任何回归到非 AI 自主表达的文案。
//
// 后端路由：
//   GET  /api/evolution/proposals/:id
//   POST /api/evolution/proposals/:id/release   body { confirmation: "RELEASE"  }
//   POST /api/evolution/proposals/:id/rollback  body { confirmation: "ROLLBACK" }

import { useEffect, useState } from "react";
import { api } from "../../lib/api";
// 零跨feature import（用户裁定 B）：原语/类型/CSS 均已提升到 components/review/ 中立家，
// 卡片不再反向依赖任一 feature 模块；老页同源 import 同一套定义，渲染字节级一致。
import styles from "./ProposalReleaseCard.module.css";
import { StatusBadge, formatNumber, formatPercent } from "./proposalPrimitives";
import type {
  ProposalDetail,
  ProposalDetailResponse,
  ShadowReplaysSummary,
} from "./proposalTypes";
import {
  FIVE_GATE_KEYS,
  GATE_LABELS,
  gateHit,
  readAggregateEvidence,
  PROMPT_AGG_METRIC_KEYS,
} from "./evidenceMetrics";
import { FINAL_REVIEW_STATUS_LABELS, labelOf } from "../../lib/reviewLabels";

export function ProposalReleaseCard({
  proposalId,
  onClose,
  onDone,
}: {
  proposalId: string;
  onClose?: () => void;
  onDone?: () => void;
}) {
  const [data, setData] = useState<ProposalDetailResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>("");
  const [modal, setModal] = useState<null | "release" | "rollback">(null);

  async function load() {
    setLoading(true);
    setError("");
    try {
      const d = await api.get<ProposalDetailResponse>(`/api/evolution/proposals/${proposalId}`);
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
      <aside className={styles.detail} data-testid="proposal-detail-loading">
        <div className={styles.loading}>加载中…</div>
      </aside>
    );
  }
  if (error) {
    return (
      <aside className={styles.detail} data-testid="proposal-detail-error">
        <div className={styles.error} role="alert">
          {error}
        </div>
        <button className={styles.btnQuiet} onClick={onClose}>关闭</button>
      </aside>
    );
  }
  if (!data) return null;

  const { proposal, shadowReplays, cohortRunIds } = data;
  const releaseEnabled = proposal.status === "eligible_for_release";
  const rollbackEnabled = proposal.status === "released";

  return (
    <aside className={styles.detail} data-testid="proposal-detail">
      <header className={styles.detailHead}>
        <h3>{proposal.kind === "threshold" ? "阈值候选" : "Prompt 候选"} 详情</h3>
        <button className={styles.btnQuiet} onClick={onClose}>关闭</button>
      </header>

      <div className={styles.detailStatusRow}>
        <StatusBadge status={proposal.status} />
        {proposal.failureReason && (
          <p className={styles.failureReason} data-testid="failure-reason">
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

      <MetadataSection proposal={proposal} cohortRunIds={cohortRunIds} />

      <footer className={styles.detailActions}>
        <button
          className={styles.btnPrimary}
          onClick={() => setModal("release")}
          disabled={!releaseEnabled}
          data-testid="release-button"
        >
          发布
        </button>
        <button
          className={styles.btnDanger}
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
            onDone?.();
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
            onDone?.();
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
    <section className={styles.thresholdDiff} data-testid="threshold-diff">
      <table className={styles.thresholdTable}>
        <tbody>
          <tr>
            <th>闸门项</th>
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
            <th>样本组命中率</th>
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
    <section className={styles.promptDiff} data-testid="prompt-diff">
      <div className={styles.promptDiffPanes}>
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
        <div className={styles.criticReasoning} data-testid="critic-reasoning">
          <h4>评审推理</h4>
          <p>{proposal.criticReasoning}</p>
        </div>
      )}
      {expected.length > 0 && (
        <div className={styles.expectedTags} data-testid="expected-improvement">
          {expected.map((tag) => (
            <span key={tag} className={styles.tag}>
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
  const isPrompt = proposal.kind === "prompt";
  const aggregate = isPrompt ? readAggregateEvidence(proposal.evalMetrics ?? {}) : null;
  return (
    <section className={styles.shadowEval} data-testid="shadow-eval">
      <h4>影子评测</h4>
      <div className={styles.shadowGrid}>
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

      {isPrompt && aggregate && (
        <div className={styles.evidenceAggregate} data-testid="evidence-aggregate">
          <h5>新旧对照·五闸涨跌</h5>
          <table className={styles.evidenceTable}>
            <thead>
              <tr>
                <th>闸</th>
                <th>Δ 命中率</th>
              </tr>
            </thead>
            <tbody>
              {aggregate.gateDeltas.map((g) => (
                <tr key={g.gate}>
                  <td>{GATE_LABELS[g.gate] ?? g.gate}</td>
                  <td className={deltaToneClass(g.delta)}>
                    {g.delta === null ? "—" : `${g.delta > 0 ? "+" : ""}${formatPercent(g.delta)}`}
                  </td>
                </tr>
              ))}
              <tr>
                <td>自评解决率</td>
                <td>
                  {formatPercent(aggregate.originalCritiqueRate)} →{" "}
                  {formatPercent(aggregate.newCritiqueRate)}
                  {aggregate.critiqueDelta !== null && (
                    <span className={deltaToneClass(aggregate.critiqueDelta, true)}>
                      {" "}({aggregate.critiqueDelta > 0 ? "+" : ""}
                      {formatPercent(aggregate.critiqueDelta)})
                    </span>
                  )}
                </td>
              </tr>
              <tr>
                <td>token 均值Δ</td>
                <td>{formatNumber(aggregate.tokenDelta, 0)}</td>
              </tr>
            </tbody>
          </table>
        </div>
      )}

      {isPrompt && summary.samples.length > 0 && (
        <div className={styles.evidenceSamples} data-testid="evidence-samples">
          <h5>逐样本新旧对照（前 5 条）</h5>
          <table className={styles.evidenceTable}>
            <thead>
              <tr>
                <th>运行编号</th>
                <th>原终审</th>
                <th>新终审</th>
                <th>原五闸</th>
                <th>新五闸</th>
                <th>自评</th>
              </tr>
            </thead>
            <tbody>
              {summary.samples.map((s) => (
                <tr key={s.id ?? s.sourceRunId}>
                  <td>{s.sourceRunId}</td>
                  <td>{labelOf(FINAL_REVIEW_STATUS_LABELS, s.originalFinalReviewStatus)}</td>
                  <td>{labelOf(FINAL_REVIEW_STATUS_LABELS, s.newFinalReviewStatus)}</td>
                  <td>{renderGateDots(s.original5gateHit)}</td>
                  <td>{renderGateDots(s.new5gateHit)}</td>
                  <td>
                    {fmtCritique(s.originalSelfCritiqueAddressed)}→
                    {fmtCritique(s.newSelfCritiqueAddressed)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* threshold 类保留原样本表（new 侧），prompt 类已被上方对照表取代 */}
      {!isPrompt && summary.samples.length > 0 && (
        <details>
          <summary>样本（前 5 条）</summary>
          <table>
            <thead>
              <tr>
                <th>运行编号</th>
                <th>原终审</th>
                <th>新终审</th>
                <th>token 数</th>
              </tr>
            </thead>
            <tbody>
              {summary.samples.map((s) => (
                <tr key={s.id ?? s.sourceRunId}>
                  <td>{s.sourceRunId}</td>
                  <td>{labelOf(FINAL_REVIEW_STATUS_LABELS, s.originalFinalReviewStatus)}</td>
                  <td>{labelOf(FINAL_REVIEW_STATUS_LABELS, s.newFinalReviewStatus)}</td>
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

// 五闸命中点阵：按固定序渲染 ●(命中)/○(未中)/·(缺失)。
function renderGateDots(doc: Record<string, unknown>): string {
  return FIVE_GATE_KEYS.map((g) => {
    const h = gateHit(doc, g);
    return h === null ? "·" : h ? "●" : "○";
  }).join("");
}

function fmtCritique(v: boolean | null): string {
  return v === null ? "—" : v ? "已解决" : "未解决";
}

// Δ 语义色：五闸命中率下降为好(绿)、上升为坏(红)；自评率方向相反(critiqueGood=true 时升为好)。
function deltaToneClass(delta: number | null, critiqueGood = false): string {
  if (delta === null || delta === 0) return styles.deltaNeutral;
  const good = critiqueGood ? delta > 0 : delta < 0;
  return good ? styles.deltaGood : styles.deltaBad;
}

// ── 候选元数据区（后端已返回但原卡片未渲染的 5 字段，E12）──
// 取值路径有别：diffSummary/riskNote/previousPromptVersion/evalMetrics 在 proposal.*；
// cohortRunIds 在 ProposalDetailResponse 顶层（由父组件解构后透传）。
// 空值（string 为 null/空、evalMetrics 空对象、cohortRunIds 空数组）时各区块整体不渲染，
// 避免详情卡出现空标题。

function MetadataSection({
  proposal,
  cohortRunIds,
}: {
  proposal: ProposalDetail;
  cohortRunIds: string[];
}) {
  const diffSummary = proposal.diffSummary?.trim() ?? "";
  const riskNote = proposal.riskNote?.trim() ?? "";
  const prevVersion = proposal.previousPromptVersion?.trim() ?? "";
  // prompt 类：移出已被新旧对照表结构化展示的聚合 key（按白名单），
  // 避免与对照表重复平铺。threshold 类完全不动（其 evalMetrics 是另一套
  // send_success/safety 字段，且与 prompt 共享 five_gate_hit_delta_per_gate
  // 等 key——绝不能按 key 名笼统过滤，只在 kind==="prompt" 时按白名单移除）。
  const allMetricEntries = Object.entries(proposal.evalMetrics ?? {});
  const metricEntries =
    proposal.kind === "prompt"
      ? allMetricEntries.filter(
          ([key]) => !(PROMPT_AGG_METRIC_KEYS as readonly string[]).includes(key),
        )
      : allMetricEntries;
  const runIds = cohortRunIds ?? [];

  // 5 字段全空时整段不渲染。
  if (
    !diffSummary &&
    !riskNote &&
    !prevVersion &&
    metricEntries.length === 0 &&
    runIds.length === 0
  ) {
    return null;
  }

  return (
    <section className={styles.metadata} data-testid="proposal-metadata">
      {diffSummary && (
        <div className={styles.metaBlock} data-testid="proposal-diff-summary">
          <h4>变更摘要</h4>
          <p>{diffSummary}</p>
        </div>
      )}
      {riskNote && (
        <div className={styles.metaBlock} data-testid="proposal-risk-note">
          <h4>风险提示</h4>
          <p>{riskNote}</p>
        </div>
      )}
      {prevVersion && (
        <div className={styles.metaBlock} data-testid="proposal-prev-version">
          <h4>上一版 Prompt 版本</h4>
          <p>{prevVersion}</p>
        </div>
      )}
      {metricEntries.length > 0 && (
        <div className={styles.metaBlock} data-testid="proposal-eval-metrics">
          <h4>评测指标</h4>
          <table className={styles.thresholdTable}>
            <tbody>
              {metricEntries.map(([key, value]) => (
                <tr key={key}>
                  <th>{key}</th>
                  <td>{formatMetricValue(value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {runIds.length > 0 && (
        <div className={styles.metaBlock} data-testid="proposal-cohort-runs">
          <h4>样本组运行（{runIds.length}）</h4>
          <div className={styles.expectedTags}>
            {runIds.map((runId) => (
              <span key={runId} className={styles.tag}>
                {runId}
              </span>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

// evalMetrics 值类型不定（数值/字符串/嵌套对象），统一转可读文本。
function formatMetricValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
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
      await api.post(`/api/evolution/proposals/${proposalId}/${kind}`, {
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
    <div className={styles.modalOverlay} data-testid={`confirm-modal-${kind}`}>
      <div className={styles.modal}>
        <h3>确认{verb}候选？</h3>
        <p>
          请输入 <code>{literal}</code> 以确认。任何不完全匹配的输入都会阻止提交。
        </p>
        <input
          className={styles.modalInput}
          type="text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={literal}
          data-testid={`confirm-input-${kind}`}
          autoFocus
        />
        {err && (
          <div className={styles.error} role="alert">
            {err}
          </div>
        )}
        <footer className={styles.modalFoot}>
          <button className={styles.btnQuiet} onClick={onClose} disabled={submitting}>
            取消
          </button>
          <button
            className={styles.btnPrimary}
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
