import { useEffect, useState } from "react";
import { Clock3 } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useOperationsStore } from "../../stores/operationsStore";
import { useAccountStore } from "../../stores/accountStore";
import { api } from "../../lib/api";
import { FINAL_REVIEW_STATUS_LABELS, GATEWAY_STATUS_LABELS, HOLD_CATEGORY_LABELS, NEXT_BEST_ACTION_TYPE_LABELS, REVIEW_SCORE_LABELS, labelOf } from "../../lib/reviewLabels";
import type { DecisionReview, AgentRunItem } from "../../types";
import styles from "./Operations.module.css";

function formatTime(value?: string) {
  if (!value) return "-";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}

type EventTone = "ai" | "good" | "warn" | "error" | "neutral";

function eventTone(status?: string): EventTone {
  const s = (status || "").toLowerCase();
  if (!s) return "neutral";
  if (s.includes("success") || s === "ok" || s === "approved" || s.includes("done")) return "good";
  if (s.includes("fail") || s.includes("error") || s.includes("blocked") || s.includes("rejected")) return "error";
  if (s.includes("warn") || s.includes("hold") || s.includes("pending") || s.includes("waiting")) return "warn";
  if (s.includes("ai") || s.includes("agent")) return "ai";
  return "neutral";
}

function taskStatusTone(status?: string): StatusTone {
  const s = (status || "").toLowerCase();
  if (s.includes("done") || s.includes("success") || s.includes("completed")) return "running";
  if (s.includes("fail") || s.includes("error") || s.includes("cancel")) return "blocked";
  if (s.includes("pending") || s.includes("wait") || s.includes("scheduled")) return "scheduled";
  if (s.includes("hold")) return "held";
  return "inactive";
}

// LLM 调用日志状态(models.rs:3005 闭集:success/cache_hit/failed/json_error);未知回落原值。
const LLM_CALL_STATUS_LABELS: Record<string, string> = {
  success: "成功",
  cache_hit: "缓存命中",
  failed: "失败",
  json_error: "返回解析失败",
};

export function formatScores(scores: Record<string, number>) {
  const entries = Object.entries(scores ?? {}).filter(([, v]) => v !== undefined && v !== null);
  return (
    entries
      .map(([key, v]) => `${REVIEW_SCORE_LABELS[key] ?? key}:${v}`)
      .join(" / ") || "-"
  );
}

function nextBestActionLabel(action?: Record<string, unknown>) {
  if (!action) return "-";
  const type = typeof action.type === "string" ? action.type : "-";
  const typeLabel = NEXT_BEST_ACTION_TYPE_LABELS[type] ?? type;
  const score = typeof action.score === "number" ? ` / ${action.score}` : "";
  return `${typeLabel}${score}`;
}

function reviewTone(review: DecisionReview): StatusTone {
  return review.approved ? "running" : "blocked";
}

// run envelope 终态 → StatusBadge tone（gateway_status 闭集，未知值回落 inactive）。
function runStatusTone(status?: string): StatusTone {
  const s = (status || "").toLowerCase();
  if (s.includes("sent") || s.includes("success") || s.includes("done") || s.includes("approved")) return "running";
  if (s.includes("fail") || s.includes("error") || s.includes("blocked") || s.includes("reject")) return "blocked";
  if (s.includes("hold") || s.includes("held")) return "held";
  if (s.includes("pending") || s.includes("wait") || s.includes("retry")) return "scheduled";
  return "inactive";
}

// C9 tier 遥测：tier_used / sufficiency / escalated / forced_full 的实际可达数据源是
// run envelope 的 decision 文档（AgentDecision 序列化 camelCase：sufficiency / missingTier），
// 不是 gatewayResult（SendGatewayResult 无 tier 字段），也不是 events.detail（/api/events 不下发 detail）。
// missingTier 即本轮需要升到的档位（none → Lean 足够；relational / full → 需升档）。
const TIER_LABELS: Record<string, string> = {
  none: "精简档已足够",
  relational: "需关系档",
  full: "需完整知识档",
};
const SUFFICIENCY_LABELS: Record<string, string> = {
  enough: "信息充分",
  need_more_context: "需更多上下文",
  need_clarification: "需澄清",
};

// 跟进任务状态(agent_tasks.status;未知值回落原值)。
const TASK_STATUS_LABELS: Record<string, string> = {
  pending: "待执行",
  scheduled: "已排程",
  running: "执行中",
  done: "已完成",
  completed: "已完成",
  failed: "已失败",
  cancelled: "已取消",
  canceled: "已取消",
};

// 客户反应 outcome_status(reaction.rs;LLM 派生 + polarity 配置,未知值回落原值)。
const OUTCOME_STATUS_LABELS: Record<string, string> = {
  user_replied_positive: "客户正面回应",
  user_replied_neutral: "客户中性回应",
  user_replied_negative: "客户负面回应",
  user_replied_objection: "客户提出异议",
  user_replied_stop_requested: "客户要求停止",
  user_replied_buying_signal: "客户释放购买信号",
  user_replied_continue_exploring: "客户继续了解",
  user_replied_unclassified: "反应待分类",
  pending: "待客户反应",
};

// run 触发来源 trigger_kind(未知值回落原值)。
const TRIGGER_KIND_LABELS: Record<string, string> = {
  inbound_message: "客户来信",
  reply: "回复触发",
  follow_up: "主动跟进",
  scheduled: "定时任务",
  envelope_recovered: "任务恢复重跑",
};

type TierTelemetry = {
  sufficiency: string;
  missingTier: string;
  escalated: boolean;
};

function tierTelemetry(run: AgentRunItem): TierTelemetry | null {
  const decision = run.decision;
  if (!decision || typeof decision !== "object") return null;
  const sufficiency = typeof decision.sufficiency === "string" ? decision.sufficiency : "";
  const missingTier = typeof decision.missingTier === "string" ? decision.missingTier : "";
  if (!sufficiency && !missingTier) return null;
  // escalated = 本轮需升档（missingTier 非 none/空 即代表 Lean 不够、需要关系/完整档）。
  const escalated = missingTier !== "" && missingTier !== "none";
  return { sufficiency, missingTier, escalated };
}

// 各阶段 Document 通用 key-value 渲染：未知字段不写死，标量直显，对象/数组 JSON 兜底。
function renderStageValue(value: unknown): string {
  if (value === null || value === undefined) return "-";
  if (typeof value === "string") return value || "-";
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

const RUN_STAGE_KEYS: { key: keyof AgentRunItem; label: string }[] = [
  { key: "planner", label: "规划" },
  { key: "context", label: "上下文" },
  { key: "knowledgeRoute", label: "知识路由" },
  { key: "decision", label: "决策" },
  { key: "review", label: "复核" },
  { key: "gatewayResult", label: "送达网关" },
];

// 未通过时按可用字段选具体中文标签;finalReviewStatus / holdCategory 都缺失
// (老数据 / 向后兼容)则回落二元"拦截"。注意 labelOf 对空值返回 "—"(truthy),
// 故不能用 || 串联兜底,须显式判断字段存在性。
function blockedLabel(review: DecisionReview): string {
  if (review.finalReviewStatus) return labelOf(FINAL_REVIEW_STATUS_LABELS, review.finalReviewStatus);
  if (review.holdCategory) return labelOf(HOLD_CATEGORY_LABELS, review.holdCategory);
  return "拦截";
}

export default function OperationsFeature() {
  const {
    events,
    tasks,
    decisionReviews,
    llmUsage,
    agentRuns,
    loading,
    opsTab,
    setOpsTab,
    loadOperationsData
  } = useOperationsStore();

  const currentAccountId = useAccountStore((s) =>
    s.accounts.some((a) => a.accountId === s.selectedAccountId)
      ? s.selectedAccountId
      : s.accounts[0]?.accountId ?? ""
  );

  useEffect(() => {
    loadOperationsData(currentAccountId);
  }, [loadOperationsData, currentAccountId]);

  // run envelope 视图：当前展开的运行（runId），点列表行切换。
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);

  const tabs: { id: typeof opsTab; label: string }[] = [
    { id: "tasks", label: "跟进任务" },
    { id: "events", label: "运营事件" },
    { id: "reviews", label: "复核记录" },
    { id: "runs", label: "运行日志" },
    { id: "llm", label: "LLM 成本" }
  ];

  const usage = llmUsage?.summary;
  const usageItems = llmUsage?.items || [];

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Operations</span>
            <span className={styles.title}>任务、事件与复核</span>
          </div>
          <span className={styles.clock}><Clock3 size={17} /></span>
        </div>

        <div className={styles.tabs}>
          {tabs.map((t) => (
            <button
              key={t.id}
              className={`${styles.tab} ${opsTab === t.id ? styles.tabActive : ""}`}
              onClick={() => setOpsTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>

        {opsTab === "tasks" &&
          (loading ? (
            <EmptyState title="加载中…" hint="正在拉取运营数据。" />
          ) : tasks.length === 0 ? (
            <EmptyState title="暂无跟进任务" hint="Agent 排程的跟进会在这里按计划呈现。" />
          ) : (
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>状态</th>
                  <th>任务内容</th>
                  <th>计划执行</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {tasks.map((task) => (
                  <tr key={task.id}>
                    <td><StatusBadge tone={taskStatusTone(task.status)}>{labelOf(TASK_STATUS_LABELS, task.status)}</StatusBadge></td>
                    <td>{task.content}</td>
                    <td className={styles.cellMuted}>{formatTime(task.runAt)}</td>
                    <td className={styles.cellActions}>
                      <button
                        type="button"
                        className={styles.linkBtn}
                        onClick={async () => {
                          await api.post(`/api/agent-tasks/${task.id}/review-now`);
                          loadOperationsData(currentAccountId);
                        }}
                      >
                        立即复核
                      </button>
                      <button
                        type="button"
                        className={styles.linkBtn}
                        onClick={async () => {
                          await api.post(`/api/agent-tasks/${task.id}/cancel`);
                          loadOperationsData(currentAccountId);
                        }}
                      >
                        取消
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ))}

        {opsTab === "events" &&
          (loading ? (
            <EmptyState title="加载中…" hint="正在拉取运营数据。" />
          ) : events.length === 0 ? (
            <EmptyState title="暂无运营事件" hint="跟进任务、Agent 决策与拦截会按时间在这里呈现。" />
          ) : (
            <ol className={styles.timeline}>
              {events.map((event) => {
                const tone = eventTone(event.status);
                return (
                  <li key={event.id} className={`${styles.tItem} ${styles[tone]}`}>
                    <span className={styles.tDot} />
                    <div className={styles.tCard}>
                      <div className={styles.tHead}>
                        <strong>{event.kind}</strong>
                        <span>{formatTime(event.createdAt)}</span>
                      </div>
                      {event.summary && <p>{event.summary}</p>}
                      {event.detail && Object.keys(event.detail).length > 0 && (
                        <details className={styles.eventDetail}>
                          <summary>结构化明细</summary>
                          <pre>{JSON.stringify(event.detail, null, 2)}</pre>
                        </details>
                      )}
                      {event.status && (
                        <div className={styles.tChips}>
                          <span>{event.status}</span>
                        </div>
                      )}
                    </div>
                  </li>
                );
              })}
            </ol>
          ))}

        {opsTab === "reviews" &&
          (loading ? (
            <EmptyState title="加载中…" hint="正在拉取运营数据。" />
          ) : decisionReviews.length === 0 ? (
            <EmptyState title="暂无 Review 记录" hint="独立复盘 Agent 的结论与评分会在这里留痕。" />
          ) : (
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>结论</th>
                  <th>下一步</th>
                  <th>结果</th>
                  <th>评分</th>
                  <th>摘要</th>
                  <th>时间</th>
                </tr>
              </thead>
              <tbody>
                {decisionReviews.map((review) => (
                  <tr key={review.id}>
                    <td><StatusBadge tone={reviewTone(review)}>{review.approved ? "通过" : blockedLabel(review)}</StatusBadge></td>
                    <td>{nextBestActionLabel(review.nextBestAction)}</td>
                    <td className={styles.cellMuted}>{labelOf(OUTCOME_STATUS_LABELS, review.outcomeStatus || "pending")}</td>
                    <td className={styles.cellNum}>{formatScores(review.scores)}</td>
                    <td>{review.reviewSummary || review.replyText || "-"}</td>
                    <td className={styles.cellMuted}>{formatTime(review.createdAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ))}

        {opsTab === "runs" &&
          (loading ? (
            <EmptyState title="加载中…" hint="正在拉取运营数据。" />
          ) : agentRuns.length === 0 ? (
            <EmptyState title="暂无运行日志" hint="AI 每轮决策的运行记录（含档位遥测）会在这里留痕。" />
          ) : (
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>状态</th>
                  <th>运行 ID</th>
                  <th>触发</th>
                  <th>档位遥测</th>
                  <th>时间</th>
                </tr>
              </thead>
              <tbody>
                {agentRuns.map((run) => {
                  const tier = tierTelemetry(run);
                  const expanded = expandedRunId === (run.runId || run.id);
                  return (
                    <RunEnvelopeRows
                      key={run.id || run.runId}
                      run={run}
                      tier={tier}
                      expanded={expanded}
                      onToggle={() =>
                        setExpandedRunId(expanded ? null : run.runId || run.id)
                      }
                    />
                  );
                })}
              </tbody>
            </table>
          ))}

        {opsTab === "llm" && (
          <>
            <div className={styles.usageGrid}>
              <div className={styles.usageCard}>
                <div className={styles.usageK}>调用次数</div>
                <div className={styles.usageV}>{usage?.totalCalls ?? 0}</div>
              </div>
              <div className={styles.usageCard}>
                <div className={styles.usageK}>总 token</div>
                <div className={styles.usageV}>{usage?.totalTokens ?? 0}</div>
              </div>
              <div className={styles.usageCard}>
                <div className={styles.usageK}>缓存命中 token</div>
                <div className={styles.usageV}>{usage?.promptCacheHitTokens ?? 0}</div>
              </div>
              <div className={styles.usageCard}>
                <div className={styles.usageK}>缓存命中率</div>
                <div className={`${styles.usageV} ${styles.key}`}>{Math.round((usage?.promptCacheHitRate ?? 0) * 100)}%</div>
              </div>
            </div>
            {loading ? (
              <EmptyState title="加载中…" hint="正在拉取运营数据。" />
            ) : usageItems.length === 0 ? (
              <EmptyState title="暂无 LLM 调用记录" hint="Agent 的每次模型调用都会在这里计量成本。" />
            ) : (
              <table className={styles.table}>
                <thead>
                  <tr>
                    <th>提示词</th>
                    <th>状态</th>
                    <th>耗时</th>
                    <th>命中</th>
                    <th>未命中</th>
                    <th>时间</th>
                  </tr>
                </thead>
                <tbody>
                  {usageItems.map((item) => (
                    <tr key={item.id}>
                      <td>{item.promptKey}</td>
                      <td className={styles.cellMuted}>{LLM_CALL_STATUS_LABELS[item.status] ?? item.status}</td>
                      <td className={styles.cellNum}>{item.latencyMs}ms</td>
                      <td className={styles.cellNum}>{item.promptCacheHitTokens}</td>
                      <td className={styles.cellNum}>{item.promptCacheMissTokens}</td>
                      <td className={styles.cellMuted}>{formatTime(item.createdAt)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </>
        )}
      </section>
    </div>
  );
}

// 单条 run envelope：摘要行 + 展开行（各阶段通用 key-value 渲染 + C9 档位遥测）。
function RunEnvelopeRows({
  run,
  tier,
  expanded,
  onToggle,
}: {
  run: AgentRunItem;
  tier: TierTelemetry | null;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr>
        <td><StatusBadge tone={runStatusTone(run.status)}>{run.status ? labelOf(GATEWAY_STATUS_LABELS, run.status) : "-"}</StatusBadge></td>
        <td className={styles.cellMuted}>{run.runId || run.id || "-"}</td>
        <td>{run.triggerKind ? labelOf(TRIGGER_KIND_LABELS, run.triggerKind) : "-"}</td>
        <td className={styles.cellMuted}>
          {tier
            ? `${TIER_LABELS[tier.missingTier] ?? tier.missingTier ?? "-"}${tier.escalated ? " · 需升档" : ""}`
            : "-"}
        </td>
        <td className={styles.cellMuted}>{formatTime(run.createdAt)}</td>
        <td className={styles.cellActions}>
          <button type="button" className={styles.linkBtn} onClick={onToggle}>
            {expanded ? "收起" : "展开"}
          </button>
        </td>
      </tr>
      {expanded && (
        <tr>
          <td colSpan={6}>
            <div className={styles.tCard}>
              {/* C9 档位遥测：来自 decision 文档（sufficiency / missingTier）。 */}
              {tier && (
                <div className={styles.tChips}>
                  <span>档位:{TIER_LABELS[tier.missingTier] ?? tier.missingTier ?? "-"}</span>
                  <span>充分性:{SUFFICIENCY_LABELS[tier.sufficiency] ?? tier.sufficiency ?? "-"}</span>
                  <span>是否升档:{tier.escalated ? "是" : "否"}</span>
                </div>
              )}
              {run.error && <p className={styles.cellMuted}>错误：{run.error}</p>}
              {RUN_STAGE_KEYS.map(({ key, label }) => {
                const stage = run[key];
                if (!stage || typeof stage !== "object") return null;
                const entries = Object.entries(stage as Record<string, unknown>);
                if (entries.length === 0) return null;
                return (
                  <div key={key as string} className={styles.tHead}>
                    <strong>{label}</strong>
                    <table className={styles.table}>
                      <tbody>
                        {entries.map(([k, v]) => (
                          <tr key={k}>
                            <td className={styles.cellMuted}>{k}</td>
                            <td>{renderStageValue(v)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                );
              })}
            </div>
          </td>
        </tr>
      )}
    </>
  );
}
