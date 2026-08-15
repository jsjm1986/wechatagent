import { useEffect, useState } from "react";
import { Workflow } from "lucide-react";
import { api } from "../../lib/api";
import { formatRate, formatNumber } from "../../lib/format";
import { useAccountStore } from "../../stores/accountStore";
import { EvaluationScenariosPanel } from "./EvaluationScenariosPanel";
import styles from "./Quality.module.css";

// 运营成效中心频道：长期指标 / 知识自动校验 / 公式遵守度评测。
// 大页头（eyebrow/title/subtitle）由 Shell 依据 channels.ts 渲染，组件仅保留面板级小标题 + Tab 条。

type OutcomeMetric = {
  id: string;
  accountId: string;
  horizon: string;
  date: string;
  replyRate: number | null;
  conversationDepth: number | null;
  aiHoldClearedRate: number | null;
  agentBlockRate: number | null;
  dailyRunCount: number;
  dailyRunTokenTotal: number;
};

type FormulaItem = {
  scenarioId: string;
  title?: string;
  predicted?: Record<string, number | null>;
  groundTruth?: Record<string, number>;
  deviations?: Record<string, number | string>;
  adherenceScore?: number;
  invalid?: boolean;
  invalidReason?: string;
  unscored?: boolean;
  missingFormulas?: number;
  skipped?: boolean;
  reason?: string;
  error?: string;
};

type FormulaSummary = {
  degraded: boolean;
  degradedReason?: string | null;
  scenarioCount: number;
  meanAdherence: number;
  totalTokensUsed?: number;
  totalTokenBudget?: number;
  processedBeforeBudgetExceeded?: number;
  totalLlmCallsUsed?: number;
  unknownUsageCalls?: number;
  usageComplete?: boolean;
  unscoredCount?: number;
  reason?: string;
};

type AutoVerifyResult = {
  processed: number;
  verified: number;
  needsReview: number;
  rejected: number;
  needsHumanAudit: number;
  degraded: boolean;
  budget?: Record<string, unknown>;
};

type QualityTab = "outcome" | "autoVerify" | "formula";

export function OutcomeMetricsTab({ accountId }: { accountId?: string }) {
  const [horizon, setHorizon] = useState<"7d" | "30d">("7d");
  const [items, setItems] = useState<OutcomeMetric[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string>("");

  async function load() {
    if (!accountId) return;
    setLoading(true);
    setErr("");
    try {
      const data = await api.get<{ items: OutcomeMetric[] }>(
        `/api/agent-outcome-metrics?accountId=${encodeURIComponent(accountId)}&horizon=${horizon}&limit=60`
      );
      setItems(data.items || []);
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

  return (
    <div className={styles.tabPanel}>
      <div className={styles.toolbar}>
        <select
          className={styles.select}
          value={horizon}
          onChange={(e) => setHorizon(e.target.value as "7d" | "30d")}
        >
          <option value="7d">7 天窗口</option>
          <option value="30d">30 天窗口</option>
        </select>
        <button className={styles.btnGhost} onClick={() => void load()} disabled={loading || !accountId}>
          {loading ? "加载中" : "刷新"}
        </button>
        <small className={styles.toolbarHint}>
          指标说明：显示"—"表示该窗口内无样本；不要把它当 0 解读。
        </small>
      </div>
      {err && <div className={styles.error}>{err}</div>}
      {!accountId && <p className={styles.hint}>请先在顶部选择一个微信账号。</p>}
      {accountId && items.length === 0 && !loading && (
        <p className={styles.hint}>该账号在选定周期内还没有效果汇总任务跑过。系统每天会自动生成。</p>
      )}
      {items.length > 0 && (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>日期</th>
              <th>回复率</th>
              <th>对话深度</th>
              <th>AI暂缓澄清率</th>
              <th>AI 拦截率</th>
              <th>当日运行数</th>
              <th>当日 token</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr key={item.id}>
                <td>{item.date}</td>
                <td>{formatRate(item.replyRate)}</td>
                <td>{formatNumber(item.conversationDepth)}</td>
                <td>{formatRate(item.aiHoldClearedRate)}</td>
                <td>{formatRate(item.agentBlockRate)}</td>
                <td>{item.dailyRunCount}</td>
                <td>{item.dailyRunTokenTotal.toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

export function AutoVerifyTab({ accountId }: { accountId?: string }) {
  const [threshold, setThreshold] = useState(7);
  const [sampleRate, setSampleRate] = useState(0.1);
  const [limit, setLimit] = useState(50);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<AutoVerifyResult | null>(null);
  const [err, setErr] = useState<string>("");

  async function run() {
    if (!accountId) return;
    setBusy(true);
    setErr("");
    setResult(null);
    try {
      const data = await api.post<AutoVerifyResult>("/api/operation-knowledge/auto-verify", {
        accountId,
        confidenceThreshold: threshold,
        humanAuditSampleRate: sampleRate,
        limit,
      });
      setResult(data);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={styles.tabPanel}>
      <p className={styles.desc}>
        对<strong>待确认</strong>状态的知识条目做 AI 预审分诊：满足「自带原文引用 + 原文出处可定位 +
        模型自评通过 + 置信分 ≥ 阈值」的条目会被挑出、转为<strong>待人工抽查</strong>，其余降级为待确认或退回。
        为守住「AI 永不自动放行」红线，预审<strong>绝不</strong>把任何知识直接标记为已确认，最终是否采纳一律由运营核验。
      </p>
      <div className={styles.toolbar}>
        <label className={styles.label}>
          置信阈值
          <input
            className={styles.input}
            type="number"
            min={0}
            max={10}
            value={threshold}
            onChange={(e) => setThreshold(Number(e.target.value) || 0)}
          />
        </label>
        <label className={styles.label}>
          抽样比例
          <input
            className={styles.input}
            type="number"
            step={0.05}
            min={0}
            max={1}
            value={sampleRate}
            onChange={(e) => setSampleRate(Number(e.target.value) || 0)}
          />
        </label>
        <label className={styles.label}>
          单次上限
          <input
            className={styles.input}
            type="number"
            min={1}
            max={500}
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value) || 1)}
          />
        </label>
        <button className={styles.btnPrimary} onClick={() => void run()} disabled={busy || !accountId}>
          {busy ? "校验中" : "开始自动校验"}
        </button>
      </div>
      {err && <div className={styles.error}>{err}</div>}
      {result && (
        <div className={styles.resultCard}>
          <h3>
            校验结果（共 {result.processed} 条）
            {result.degraded && <span className={styles.badgeDegraded}>预算超额降级</span>}
          </h3>
          <ul>
            <li>待人工抽查：{result.needsHumanAudit}</li>
            <li>仍待确认：{result.needsReview}</li>
            <li>已退回：{result.rejected}</li>
          </ul>
          {result.budget && <pre>{JSON.stringify(result.budget, null, 2)}</pre>}
        </div>
      )}
    </div>
  );
}

export function FormulaAdherenceTab({ accountId }: { accountId?: string }) {
  const [busy, setBusy] = useState(false);
  const [summary, setSummary] = useState<FormulaSummary | null>(null);
  const [items, setItems] = useState<FormulaItem[]>([]);
  const [err, setErr] = useState<string>("");

  async function run() {
    if (!accountId) return;
    setBusy(true);
    setErr("");
    setSummary(null);
    setItems([]);
    try {
      const data = await api.post<{ summary: FormulaSummary; items: FormulaItem[] }>(
        "/api/user-operations/evaluations/formula-adherence",
        { accountId }
      );
      setSummary(data.summary);
      setItems(data.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={styles.tabPanel}>
      <p className={styles.desc}>
        对所有 <code>active</code> 的 evaluation_scenarios 跑一次 simulate_user_dialogue，
        抓最后一个 turn 的 <code>review.formulaBreakdown</code> 与 <code>scores</code>，
        与场景的 <code>ground_truth</code> 比较计算 adherence。整批共享一个累计 token 预算
        （每场景 simulationTokenBudget × scenarios 数），超额时返回部分结果 + degraded:true。
        缺模型公式输出的场景标 invalid；缺失或非法管理员金标的存量场景标 unscored，均不按 0
        计入平均。预算只累计本次评测私有 simulation run；上游未报告 usage 时停止后续场景。
      </p>
      <EvaluationScenariosPanel accountId={accountId} />
      <div className={styles.toolbar}>
        <button className={styles.btnPrimary} onClick={() => void run()} disabled={busy || !accountId}>
          {busy ? "评测中" : "开始评测"}
        </button>
      </div>
      {err && <div className={styles.error}>{err}</div>}
      {summary && (
        <div className={styles.summaryCard}>
          <h3>
            平均 adherence：{summary.meanAdherence.toFixed(3)}（{summary.scenarioCount} 个有效场景）
            {summary.degraded && (
              <span className={styles.badgeDegraded}>
                降级：{summary.degradedReason || summary.reason || "未知"}
              </span>
            )}
          </h3>
          {summary.totalTokenBudget !== undefined && (
            <small>
              预算使用：{summary.totalTokensUsed?.toLocaleString() || 0} /{" "}
              {summary.totalTokenBudget.toLocaleString()}
              {summary.processedBeforeBudgetExceeded !== undefined &&
                ` · 超额前完成 ${summary.processedBeforeBudgetExceeded} 个`}
              {summary.totalLlmCallsUsed !== undefined &&
                ` · LLM 调用 ${summary.totalLlmCallsUsed} 次`}
              {(summary.unscoredCount ?? 0) > 0 && ` · 未评分 ${summary.unscoredCount} 个`}
              {summary.usageComplete === false &&
                ` · ${summary.unknownUsageCalls ?? 0} 次调用未报告 token usage`}
            </small>
          )}
        </div>
      )}
      {items.length > 0 && (
        <table className={styles.table}>
          <thead>
            <tr>
              <th>场景</th>
              <th>状态</th>
              <th>adherence</th>
              <th>偏差（预测 - 实际）</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr
                key={item.scenarioId}
                className={item.invalid || item.unscored ? styles.rowInvalid : ""}
              >
                <td>
                  <strong>{item.title || item.scenarioId}</strong>
                  <br />
                  <small>{item.scenarioId}</small>
                </td>
                <td>
                  {item.error
                    ? `❌ ${item.error}`
                    : item.skipped
                    ? `⏭ ${item.reason || "skipped"}`
                    : item.unscored
                    ? `⚠ ${item.reason || "unscored"}`
                    : item.invalid
                    ? `⚠ ${item.invalidReason || "invalid"}`
                    : "✓ 完成"}
                </td>
                <td>{item.adherenceScore !== undefined ? item.adherenceScore.toFixed(3) : "—"}</td>
                <td>
                  {item.deviations ? (
                    <code>
                      {Object.entries(item.deviations)
                        .map(([k, v]) => `${k}=${typeof v === "number" ? v.toFixed(2) : v}`)
                        .join(", ")}
                    </code>
                  ) : (
                    "—"
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

const TABS: { id: QualityTab; label: string }[] = [
  { id: "outcome", label: "长期指标" },
  { id: "autoVerify", label: "知识自动校验" },
  { id: "formula", label: "公式遵守度" },
];

export default function QualityFeature() {
  const accountId = useAccountStore((s) =>
    s.accounts.some((a) => a.accountId === s.selectedAccountId)
      ? s.selectedAccountId
      : s.accounts[0]?.accountId ?? ""
  );
  const [tab, setTab] = useState<QualityTab>("outcome");

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.panelHead}>
          <div className={styles.panelHeadL}>
            <span className={styles.eyebrow}>Outcome & Quality</span>
            <span className={styles.title}>运营成效中心</span>
          </div>
          <div className={styles.headIcon}>
            <Workflow size={18} />
          </div>
        </div>
        <div className={styles.tabs}>
          {TABS.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? styles.tabActive : styles.tab}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        {tab === "outcome" && <OutcomeMetricsTab accountId={accountId} />}
        {tab === "autoVerify" && <AutoVerifyTab accountId={accountId} />}
        {tab === "formula" && <FormulaAdherenceTab accountId={accountId} />}
      </section>
    </div>
  );
}
