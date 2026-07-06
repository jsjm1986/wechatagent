import { useEffect, useState } from "react";
import { Workflow } from "lucide-react";
import { api } from "../../lib/api";
import { formatRate, formatNumber } from "../../lib/format";
import { useAccountStore } from "../../stores/accountStore";
import { ConfirmProvider, useConfirm } from "../../components/ui/ConfirmDialog";
import { promptDiffBody } from "../../components/prompt/usePromptSaveConfirm";
import { EvaluationScenariosPanel } from "./EvaluationScenariosPanel";
import { VERSION_STATUS_LABELS, labelOf } from "../../lib/reviewLabels";
import styles from "./Quality.module.css";

// 运营成效中心频道：长期指标 / 知识自动校验 / 公式遵守度评测 / 产品声明兜底标记词。
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

type PromptTemplateLite = {
  id: string;
  promptKey: string;
  status: string;
  version: number;
  content: string;
};

type QualityTab = "outcome" | "autoVerify" | "formula" | "markers";

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
              <th>当日 run 数</th>
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
        缺四个公式的场景标 invalid，不静默按 0 计入平均。
      </p>
      <EvaluationScenariosPanel />
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
              <tr key={item.scenarioId} className={item.invalid ? styles.rowInvalid : ""}>
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

export function ProductClaimMarkersTab() {
  // 路径B 二次确认弹框需 <ConfirmProvider> 祖先；quality 频道未挂全局 Provider，
  // 本 Tab 自包含补挂（独立内联 save()，与 strategyStore 无关）。
  return (
    <ConfirmProvider>
      <ProductClaimMarkersTabInner />
    </ConfirmProvider>
  );
}

function ProductClaimMarkersTabInner() {
  const confirm = useConfirm();
  const [template, setTemplate] = useState<PromptTemplateLite | null>(null);
  const [draft, setDraft] = useState<string>("");
  const [parseError, setParseError] = useState<string>("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string>("");
  const [statusMsg, setStatusMsg] = useState<string>("");

  async function load() {
    setErr("");
    setStatusMsg("");
    try {
      const data = await api.get<{ items: PromptTemplateLite[] }>("/api/prompt-templates");
      const found = (data.items || []).find(
        (item) => item.promptKey === "user.review.product_claim_markers" && item.status === "active"
      );
      if (!found) {
        setErr("未找到 active 的 user.review.product_claim_markers 模板，可能需要重置 prompt pack。");
        return;
      }
      setTemplate(found);
      setDraft(found.content);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }

  useEffect(() => {
    void load();
  }, []);

  function validateJson(text: string): string {
    try {
      const parsed = JSON.parse(text);
      if (!parsed || typeof parsed !== "object") return "JSON 顶层必须是对象";
      if (!Array.isArray(parsed.markers)) return "缺 markers 数组";
      if (!Array.isArray(parsed.whitelistPhrases)) return "缺 whitelistPhrases 数组";
      if (typeof parsed.whitelistWindowChars !== "number") return "whitelistWindowChars 必须是数字";
      for (const m of parsed.markers) {
        if (!m || typeof m !== "object") return "markers 中含非对象项";
        if (typeof m.kind !== "string") return "marker.kind 必须是字符串";
        if (typeof m.label !== "string") return "marker.label 必须是字符串";
      }
      return "";
    } catch (e) {
      return e instanceof Error ? e.message : String(e);
    }
  }

  function onChangeDraft(value: string) {
    setDraft(value);
    setParseError(validateJson(value));
  }

  async function save(force?: boolean) {
    if (!template) return;
    const validation = validateJson(draft);
    if (validation) {
      setParseError(validation);
      return;
    }
    setSaving(true);
    setErr("");
    setStatusMsg("");
    try {
      const resp = await api.put<{ status?: string; reason?: string; diff?: string }>(
        `/api/prompt-templates/${template.id}`,
        {
          promptKey: template.promptKey,
          agentKind: "user",
          layer: "review_guard",
          title: "产品事实风险兜底标记",
          description: "Rust 字符串兜底 guard 使用的可编辑标记词和白名单。",
          content: draft,
          status: "active",
          // force=true：管理者已逐字核对，覆盖 LLM 红线语义审查。
          ...(force ? { force: true } : {}),
        }
      );
      // 真 bug 修复：needs_human_confirm 是 200，不能静默当成功发布。
      if (resp && resp.status === "needs_human_confirm") {
        setSaving(false);
        const ok = await confirm({
          title: "改动需逐字核对后确认",
          body: promptDiffBody(resp.reason ?? "", resp.diff ?? ""),
          tone: "danger",
          confirmText: "已核对，强制保存",
          requireText: "已核对",
        });
        if (ok) await save(true);
        return;
      }
      // publish 独立三态流：publish 可能在 update 通过后仍被 publish 自己的 LLM 闸拦。
      // 用独立 try/catch + 内联 {force:true} 重发（不再重走 save(true)，避免 double-PUT/死循环）。
      const doPublish = async (publishForce?: boolean): Promise<void> => {
        try {
          const pubResp = await api.post<{ status?: string; reason?: string; diff?: string }>(
            `/api/prompt-templates/${template.id}/publish`,
            publishForce ? { force: true } : {}
          );
          if (pubResp && pubResp.status === "needs_human_confirm") {
            setSaving(false);
            const ok = await confirm({
              title: "发布前需逐字核对后确认",
              body: promptDiffBody(pubResp.reason ?? "", pubResp.diff ?? ""),
              tone: "danger",
              confirmText: "已核对，强制发布",
              requireText: "已核对",
            });
            // 带 force 重发 publish（不再重走 update）。
            if (ok) await doPublish(true);
            return;
          }
          setStatusMsg("已保存。注：该兜底守卫当前未启用，改动暂不影响评审行为。");
          await load();
        } catch (e) {
          const message = e instanceof Error ? e.message : String(e);
          // publish 阶段的红线 reject：内联带 force 重发 publish，不走外层 save(true)（避免 double-PUT）。
          if (message.includes("红线语义审查拒绝")) {
            setSaving(false);
            const ok = await confirm({
              title: "触碰自治边界红线，已被语义审查拦截",
              body: promptDiffBody(message, ""),
              tone: "danger",
              confirmText: "已核对无误，强制发布",
              requireText: "已核对",
            });
            if (ok) await doPublish(true);
            return;
          }
          setErr(message);
        }
      };
      // publish 的 force 与 update 的 force 独立：publish 始终先以无 force 跑自己的红线闸，
      // force 只由 publish 自身的 confirm 产生（不继承 update 的 force）。
      await doPublish();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      // Reject：4xx「红线语义审查拒绝」→ 弹框显拒绝理由 + 强制保存入口。
      if (message.includes("红线语义审查拒绝")) {
        setSaving(false);
        const ok = await confirm({
          title: "触碰自治边界红线，已被语义审查拦截",
          body: promptDiffBody(message, ""),
          tone: "danger",
          confirmText: "已核对无误，强制保存",
          requireText: "已核对",
        });
        if (ok) await save(true);
        return;
      }
      setErr(message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className={styles.tabPanel}>
      <p className={styles.desc}>
        产品事实风险兜底的标记词与白名单短语（预留配置，当前未启用）。当前线上对产品声明的把关由
        「已验证知识背书」机制负责——AI 只能基于已核验的知识条目做产品 / 价格 / 政策声明，否则拦截。
        本配置为将来重新启用字符串级兜底守卫时预留，编辑后不会改变当前评审行为。
      </p>
      {err && <div className={styles.error}>{err}</div>}
      {statusMsg && <div className={styles.success}>{statusMsg}</div>}
      {template && (
        <>
          <small className={styles.metaLine}>
            模板版本 v{template.version} · 状态：{labelOf(VERSION_STATUS_LABELS, template.status)}
          </small>
          <textarea
            className={styles.textarea}
            value={draft}
            onChange={(e) => onChangeDraft(e.target.value)}
            spellCheck={false}
          />
          {parseError && <div className={styles.error}>JSON 校验：{parseError}</div>}
          <div className={styles.actions}>
            <button className={styles.btnPrimary} onClick={() => void save()} disabled={saving || !!parseError}>
              {saving ? "发布中" : "保存并发布"}
            </button>
            <button className={styles.btnGhost} onClick={() => void load()} disabled={saving}>
              丢弃改动
            </button>
          </div>
        </>
      )}
    </div>
  );
}

const TABS: { id: QualityTab; label: string }[] = [
  { id: "outcome", label: "长期指标" },
  { id: "autoVerify", label: "知识自动校验" },
  { id: "formula", label: "公式遵守度" },
  { id: "markers", label: "产品声明标记词" },
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
        {tab === "markers" && <ProductClaimMarkersTab />}
      </section>
    </div>
  );
}
