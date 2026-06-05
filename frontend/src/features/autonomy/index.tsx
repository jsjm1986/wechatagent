import { useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { api } from "../../lib/api";
import { formatRate } from "../../lib/format";
import { useAccountStore } from "../../stores/accountStore";
import styles from "./Autonomy.module.css";

// 自治回路监控频道：从 /api/outcomes/autonomy 拉指标 + revision 记录，渲染
// 修订触发/通过率、AI 暂缓三类细分、未验证产品声明拦截、发送链路状态与 Planner 调度。
// 大页头（eyebrow/title/subtitle）由 Shell 依据 channels.ts 渲染，组件仅保留面板级小标题。
//
// 文案严守 AI 自主语义（AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文），CI lint 在 PR 阻断回归。
// data-testid 与既有单测一一镜像，保持不变。

type PlannerSection = {
  silent: {
    tick: number;
    scanned: number;
    emitted: number;
    tickDetailEmitted: number;
    capped: number;
    backoff: number;
  };
  commitment: {
    tick: number;
    overdueEmits: number;
    imminentEmits: number;
    backoff: number;
  };
  stagnation: {
    tick: number;
    emitted: number;
    backoff: number;
  };
};

export function AutonomyOutcomesTab({ accountId }: { accountId?: string }) {
  type AutonomyMetrics = {
    horizon: string;
    accountId: string;
    totalRuns: number;
    legacyModeUnchecked: number;
    metrics: {
      revisionTriggerRate: number | null;
      revisionPassRate: number | null;
      aiHoldBreakdown: {
        heldByAiPolicy: number | null;
        blockedBySafetyGuard: number | null;
        aiWaitingForMoreContext: number | null;
      };
      taxonomyCandidateRate: number | null;
      unverifiedClaimBlockRate: number | null;
      selfCritiqueAddressedRate: number | null;
      autonomyModeDistribution: {
        auto: number | null;
        assisted: number | null;
        blocked: number | null;
      };
    };
    rawCounts: Record<string, number>;
    outboxLink: {
      totalEnqueued: number;
      sent: number;
      canceled: number;
      failedTerminal: number;
      sendSuccessRate: number | null;
      canceledRate: number | null;
      failedTerminalRate: number | null;
    };
    /** M3 / Task 70：Planner 三段 tick / emit / capped / backoff 计数。 */
    planner?: PlannerSection;
  };
  type RevisionItem = {
    runId: string;
    contactWxid: string | null;
    contactName: string | null;
    preReplyExcerpt: string;
    postReplyExcerpt: string;
    preRevisionSummary: string;
    postRevisionSummary: string;
    revisionDirection: string;
    finalReviewStatus: string;
    holdCategory: string;
    selfCritique: string | null;
    createdAt: string;
  };

  const [horizon, setHorizon] = useState<"24h" | "7d" | "30d">("24h");
  const [data, setData] = useState<AutonomyMetrics | null>(null);
  const [revisions, setRevisions] = useState<RevisionItem[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string>("");

  async function load() {
    if (!accountId) return;
    setLoading(true);
    setErr("");
    try {
      const qs = `accountId=${encodeURIComponent(accountId)}&horizon=${horizon}`;
      const [metrics, revs] = await Promise.all([
        api.get<AutonomyMetrics>(`/api/outcomes/autonomy?${qs}`),
        api.get<{ items: RevisionItem[] }>(
          `/api/outcomes/autonomy/revisions?${qs}&limit=50`
        ),
      ]);
      setData(metrics);
      setRevisions(revs.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId, horizon]);

  const m = data?.metrics;
  const ob = data?.outboxLink;
  const breakdown = m?.aiHoldBreakdown;
  const dist = m?.autonomyModeDistribution;

  return (
    <div className={styles.outcomes}>
      <div className={styles.toolbar}>
        <select
          className={styles.select}
          value={horizon}
          onChange={(e) => setHorizon(e.target.value as "24h" | "7d" | "30d")}
        >
          <option value="24h">24 小时窗口</option>
          <option value="7d">7 天窗口</option>
          <option value="30d">30 天窗口</option>
        </select>
        <button className={styles.btnGhost} onClick={() => void load()} disabled={loading || !accountId}>
          {loading ? "加载中" : "刷新"}
        </button>
        <small className={styles.toolbarHint}>
          指标说明：null（"—"）= 该窗口内没有升级后样本；legacy 行单独计数不进任何分子分母。
        </small>
      </div>
      {err && <div className={styles.error}>{err}</div>}
      {!accountId && <p className={styles.hint}>请先在顶部选择一个微信账号。</p>}
      {accountId && data && (
        <>
          <div className={styles.runHeader}>
            <span>升级后 run 数：<strong>{data.totalRuns}</strong></span>
            <span>未升级 run（独立计数）：<strong>{data.legacyModeUnchecked}</strong></span>
          </div>
          <div className={styles.metricGrid}>
            <AutonomyMetricCard
              label="revision 触发率"
              value={m?.revisionTriggerRate ?? null}
              hint={`${data.rawCounts.revisionApplied}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="revision 通过率"
              value={m?.revisionPassRate ?? null}
              hint={`${data.rawCounts.revisionPass}/${data.rawCounts.revisionApplied}`}
            />
            <AutonomyMetricCard
              label="未验证产品声明拦截率"
              value={m?.unverifiedClaimBlockRate ?? null}
              hint={`${data.rawCounts.unverifiedClaimBlock}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="新词候选触发率"
              value={m?.taxonomyCandidateRate ?? null}
              hint={`${data.rawCounts.taxonomyCandidate}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="自我批判已回应率"
              value={m?.selfCritiqueAddressedRate ?? null}
              hint={`${data.rawCounts.selfCritiqueAddressed}/${data.rawCounts.revisionApplied}`}
            />
            <AutonomyMetricCard
              label="自治模式：auto"
              value={dist?.auto ?? null}
              hint={`${data.rawCounts.autonomyAuto}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="自治模式：assisted / blocked"
              value={null}
              hint={`assisted ${data.rawCounts.autonomyAssisted} · blocked ${data.rawCounts.autonomyBlocked}`}
            />
          </div>

          <div className={styles.section}>
            <h3>AI 暂缓原因分布</h3>
            <div className={styles.holdBars}>
              <HoldBar label="AI 策略主动暂缓" value={breakdown?.heldByAiPolicy ?? null} count={data.rawCounts.heldByAiPolicy} />
              <HoldBar label="安全门拦截" value={breakdown?.blockedBySafetyGuard ?? null} count={data.rawCounts.blockedBySafetyGuard} />
              <HoldBar label="AI 等待更多上下文" value={breakdown?.aiWaitingForMoreContext ?? null} count={data.rawCounts.aiWaitingForMoreContext} />
            </div>
          </div>

          <div className={styles.section}>
            <h3>发送链路状态</h3>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>入队总数</th>
                  <th>已送达</th>
                  <th>已取消</th>
                  <th>终态失败</th>
                  <th>送达率</th>
                  <th>取消率</th>
                  <th>失败率</th>
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td>{ob?.totalEnqueued ?? 0}</td>
                  <td>{ob?.sent ?? 0}</td>
                  <td>{ob?.canceled ?? 0}</td>
                  <td>{ob?.failedTerminal ?? 0}</td>
                  <td>{formatRate(ob?.sendSuccessRate ?? null)}</td>
                  <td>{formatRate(ob?.canceledRate ?? null)}</td>
                  <td>{formatRate(ob?.failedTerminalRate ?? null)}</td>
                </tr>
              </tbody>
            </table>
          </div>

          {data.planner && <AutonomyPlannerSection planner={data.planner} />}

          <div className={styles.section}>
            <h3>近 50 条 revision 记录</h3>
            {revisions.length === 0 ? (
              <p className={styles.hint}>该窗口内没有 revision 记录。</p>
            ) : (
              <table className={styles.table}>
                <thead>
                  <tr>
                    <th>联系人</th>
                    <th>修订前摘要</th>
                    <th>修订后摘要</th>
                    <th>修订方向</th>
                    <th>归档状态</th>
                    <th>暂缓分类</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {revisions.map((r) => (
                    <RevisionRow
                      key={r.runId}
                      item={r}
                      expanded={expanded === r.runId}
                      onToggle={() => setExpanded(expanded === r.runId ? null : r.runId)}
                    />
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </>
      )}
    </div>
  );
}

export function AutonomyMetricCard({ label, value, hint }: { label: string; value: number | null; hint: string }) {
  return (
    <div className={styles.metricCard}>
      <div className={styles.metricLabel}>{label}</div>
      <div className={styles.metricValue}>{formatRate(value)}</div>
      <div className={styles.metricHint}>{hint}</div>
    </div>
  );
}

/**
 * M3 / Task 77：Planner 三段事件可视化。展示 tick / scanned / emitted / capped / backoff
 * 计数；标签使用 AI 自主策略表达，符合"全自治"产品定位。
 */
export function AutonomyPlannerSection({ planner }: { planner: PlannerSection }) {
  return (
    <div className={styles.section} data-testid="planner-section">
      <h3>Planner 自主调度</h3>
      <small className={styles.toolbarHint}>
        三段扫描器（沉默跟进 / 承诺到期 / 阶段停滞）的 tick、emit、capped、backoff 计数；backoff 表示 AI 因 block-rate 过高自主回退。
      </small>
      <div className={styles.metricGrid} style={{ marginTop: 12 }}>
        <div className={styles.metricCard} data-testid="planner-silent">
          <div className={styles.metricLabel}>沉默跟进</div>
          <div className={`${styles.metricValue} autonomyMetricValue`}>{planner.silent.emitted}</div>
          <div className={styles.metricHint}>
            tick {planner.silent.tick} · scanned {planner.silent.scanned} · capped {planner.silent.capped} · backoff {planner.silent.backoff}
          </div>
        </div>
        <div className={styles.metricCard} data-testid="planner-commitment">
          <div className={styles.metricLabel}>承诺到期</div>
          <div className={`${styles.metricValue} autonomyMetricValue`}>
            {planner.commitment.overdueEmits + planner.commitment.imminentEmits}
          </div>
          <div className={styles.metricHint}>
            tick {planner.commitment.tick} · overdue {planner.commitment.overdueEmits} · imminent {planner.commitment.imminentEmits} · backoff {planner.commitment.backoff}
          </div>
        </div>
        <div className={styles.metricCard} data-testid="planner-stagnation">
          <div className={styles.metricLabel}>阶段停滞</div>
          <div className={`${styles.metricValue} autonomyMetricValue`}>{planner.stagnation.emitted}</div>
          <div className={styles.metricHint}>
            tick {planner.stagnation.tick} · backoff {planner.stagnation.backoff}
          </div>
        </div>
      </div>
    </div>
  );
}

export function HoldBar({ label, value, count }: { label: string; value: number | null; count: number }) {
  const pct = value === null || value === undefined ? 0 : Math.max(0, Math.min(1, value)) * 100;
  return (
    <div className={styles.holdBar}>
      <div className={styles.holdLabel}>
        <span>{label}</span>
        <span>{formatRate(value)}（{count} 条）</span>
      </div>
      <div className={styles.holdTrack}>
        <div className={styles.holdFill} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function RevisionRow({
  item,
  expanded,
  onToggle,
}: {
  item: {
    runId: string;
    contactName: string | null;
    contactWxid: string | null;
    preReplyExcerpt: string;
    postReplyExcerpt: string;
    preRevisionSummary: string;
    postRevisionSummary: string;
    revisionDirection: string;
    finalReviewStatus: string;
    holdCategory: string;
    selfCritique: string | null;
  };
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr>
        <td>{item.contactName || item.contactWxid || "—"}</td>
        <td>{item.preReplyExcerpt || "—"}</td>
        <td>{item.postReplyExcerpt || "—"}</td>
        <td>{item.revisionDirection || "—"}</td>
        <td>{item.finalReviewStatus}</td>
        <td>{item.holdCategory || "—"}</td>
        <td>
          <button className={styles.linkBtn} onClick={onToggle}>
            {expanded ? "收起" : "展开"}
          </button>
        </td>
      </tr>
      {expanded && (
        <tr className={styles.revisionDetail}>
          <td colSpan={7}>
            <div>
              <strong>修订前完整摘要：</strong>
              <pre>{item.preRevisionSummary || "—"}</pre>
            </div>
            <div>
              <strong>修订后完整摘要：</strong>
              <pre>{item.postRevisionSummary || "—"}</pre>
            </div>
            <div>
              <strong>自我批判（selfCritique）：</strong>
              <pre>{item.selfCritique || "—"}</pre>
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

export default function AutonomyFeature() {
  const accountId = useAccountStore((s) =>
    s.accounts.some((a) => a.accountId === s.selectedAccountId)
      ? s.selectedAccountId
      : s.accounts[0]?.accountId ?? ""
  );
  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.panelHead}>
          <div className={styles.panelHeadL}>
            <span className={styles.eyebrow}>Autonomy Loop</span>
            <span className={styles.title}>修订 · AI 暂缓 · 发送链路 · Planner</span>
          </div>
          <div className={styles.headIcon}>
            <ShieldCheck size={18} />
          </div>
        </div>
        <AutonomyOutcomesTab accountId={accountId} />
      </section>
    </div>
  );
}
