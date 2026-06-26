import { useEffect } from "react";
import { Clock3 } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useOperationsStore } from "../../stores/operationsStore";
import { useAccountStore } from "../../stores/accountStore";
import { api } from "../../lib/api";
import { FINAL_REVIEW_STATUS_LABELS, HOLD_CATEGORY_LABELS, labelOf } from "../../lib/reviewLabels";
import type { DecisionReview } from "../../types";
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

const SCORE_LABELS: Record<string, string> = {
  humanLike: "拟人度",
  emotionalValue: "情绪价值",
  hallucinationScore: "幻觉风险",
  knowledgeGroundingScore: "知识接地",
  pressureRisk: "压迫风险",
  boundaryPrivacySafety: "隐私边界",
};

export function formatScores(scores: Record<string, number>) {
  const entries = Object.entries(scores ?? {}).filter(([, v]) => v !== undefined && v !== null);
  return (
    entries
      .map(([key, v]) => `${SCORE_LABELS[key] ?? key}:${v}`)
      .join(" / ") || "-"
  );
}

function nextBestActionLabel(action?: Record<string, unknown>) {
  if (!action) return "-";
  const type = typeof action.type === "string" ? action.type : "-";
  const score = typeof action.score === "number" ? ` / ${action.score}` : "";
  return `${type}${score}`;
}

function reviewTone(review: DecisionReview): StatusTone {
  return review.approved ? "running" : "blocked";
}

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

  const tabs: { id: typeof opsTab; label: string }[] = [
    { id: "tasks", label: "跟进任务" },
    { id: "events", label: "运营事件" },
    { id: "reviews", label: "Review 记录" },
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
            <span className={styles.title}>任务、事件与 Review</span>
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
          (tasks.length === 0 ? (
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
                    <td><StatusBadge tone={taskStatusTone(task.status)}>{task.status}</StatusBadge></td>
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
          (events.length === 0 ? (
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
          (decisionReviews.length === 0 ? (
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
                    <td className={styles.cellMuted}>{review.outcomeStatus || "pending"}</td>
                    <td className={styles.cellNum}>{formatScores(review.scores)}</td>
                    <td>{review.reviewSummary || review.replyText || "-"}</td>
                    <td className={styles.cellMuted}>{formatTime(review.createdAt)}</td>
                  </tr>
                ))}
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
            {usageItems.length === 0 ? (
              <EmptyState title="暂无 LLM 调用记录" hint="Agent 的每次模型调用都会在这里计量成本。" />
            ) : (
              <table className={styles.table}>
                <thead>
                  <tr>
                    <th>Prompt Key</th>
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
                      <td className={styles.cellMuted}>{item.status}</td>
                      <td className={styles.cellNum}>{item.latencyMs}ms</td>
                      <td className={styles.cellNum}>hit {item.promptCacheHitTokens}</td>
                      <td className={styles.cellNum}>miss {item.promptCacheMissTokens}</td>
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
