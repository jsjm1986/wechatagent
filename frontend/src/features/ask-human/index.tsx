import { useCallback, useState } from "react";
import { ConfirmProvider } from "../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../components/ui/Toast";
import { useInboxStore } from "../../stores/inboxStore";
import { ReviewQueue, type RowCtx } from "../../components/review/ReviewQueue";
import { EmptyState } from "../../components/ui/EmptyState";
import { type InboxItem } from "../../lib/inboxApi";
import { EscalationInline } from "./inline/EscalationInline";
import { SimpleApproveReject } from "./inline/SimpleApproveReject";
import { ResolvedEscalations } from "./ResolvedEscalations";
import { ChunkReviewCard } from "../../components/review/ChunkReviewCard";
import { ProfilePublishCard } from "../../components/review/ProfilePublishCard";
import { ProposalReleaseCard } from "../../components/review/ProposalReleaseCard";
import { LessonPromoteCard } from "../../components/review/LessonPromoteCard";
import { TaxonomyCandidateReviewCard } from "../../components/review/TaxonomyCandidateReviewCard";
import { SuspectedDealReviewCard } from "./inline/SuspectedDealReviewCard";
import "./AskHuman.css";

// summary 端点下发 camelCase key，inbox ?source= 过滤认 snake_case source id（两者由后端 ask_human_inbox.rs 定义）。
// 单一事实来源：summaryKey ↔ source ↔ 中文标签。chip 用它渲染并把 activeSource 统一存 snake_case source。
const SOURCE_META: { summaryKey: string; source: string; label: string }[] = [
  { summaryKey: "principalEscalation", source: "principal_escalation", label: "请示裁决" },
  { summaryKey: "knowledgeReview", source: "knowledge_review", label: "知识核验" },
  { summaryKey: "taxonomyCandidate", source: "taxonomy_candidate", label: "标签候选" },
  { summaryKey: "relationshipSuggestion", source: "relationship_suggestion", label: "关系建议" },
  { summaryKey: "suspectedDeal", source: "suspected_deal", label: "疑似成交" },
  { summaryKey: "gapSignal", source: "gap_signal", label: "知识缺口" },
  { summaryKey: "profileRisky", source: "profile_risky", label: "画像发布" },
  { summaryKey: "evolutionProposal", source: "evolution_proposal", label: "进化发布" },
  { summaryKey: "lessonsLearned", source: "lessons_learned", label: "经验晋升" },
];

// InboxRow badge 的语义色调：source → tone（tone 值对齐 AskHuman.css 的 .inboxBadge--* 与 tokens.css 语义色）。
const SOURCE_TONE: Record<string, string> = {
  principal_escalation: "brand",
  knowledge_review: "scheduled",
  taxonomy_candidate: "neutral",
  relationship_suggestion: "neutral",
  suspected_deal: "held",
  gap_signal: "held",
  profile_risky: "blocked",
  evolution_proposal: "running",
  lessons_learned: "neutral",
};

// rich 分派：richComponent → 卡片。richParams 提供 id（key 已对齐 ask_human_inbox.rs 实证）。
function renderRich(item: InboxItem, onDone: () => void) {
  const p = item.richParams ?? {};
  switch (item.richComponent) {
    case "knowledgeReview":
      return <ChunkReviewCard chunkId={String(p.chunkId)} onDone={onDone} />;
    case "profilePublish":
      return <ProfilePublishCard profileId={String(p.profileId)} onDone={onDone} />;
    case "evolutionRelease":
      return <ProposalReleaseCard proposalId={String(p.proposalId)} onDone={onDone} />;
    case "lessonsPromote":
      return <LessonPromoteCard lessonId={String(p.lessonId)} onDone={onDone} />;
    case "taxonomyCandidateReview": {
      const c = item.richParams ?? {};
      return (
        <TaxonomyCandidateReviewCard
          candidate={{
            id: String(c.candidateId ?? item.id),
            scope: String(c.scope ?? ""),
            kind: String(c.kind ?? ""),
            rawValue: String(c.rawValue ?? ""),
            evidence: c.evidence != null ? String(c.evidence) : undefined,
            confidence: c.confidence != null ? Number(c.confidence) : undefined,
            occurrences: c.occurrences != null ? Number(c.occurrences) : undefined,
            suggestedDisplayName: c.suggestedDisplayName != null ? String(c.suggestedDisplayName) : undefined,
          }}
          onDone={onDone}
        />
      );
    }
    case "suspectedDealReview": {
      const deal = item.richParams ?? {};
      return (
        <SuspectedDealReviewCard
          signalId={String(deal.signalId ?? item.id)}
          contactId={String(deal.contactId ?? item.contactWxid ?? "")}
          evidence={deal.evidence != null ? String(deal.evidence) : item.evidence}
          confidence={deal.confidence != null ? Number(deal.confidence) : item.confidence}
          occurrences={deal.occurrences != null ? Number(deal.occurrences) : item.occurrences}
          onDone={onDone}
        />
      );
    }
    default:
      return <div className="askHumanUnknownRich">未知 rich 组件：{item.richComponent}</div>;
  }
}

// inline 分派：source → 处置器。端点 URL 已按 src/routes/mod.rs 实证修正。
function renderInline(item: InboxItem, ctx: RowCtx) {
  switch (item.source) {
    case "principal_escalation":
      return <EscalationInline item={item} ctx={ctx} />;
    case "relationship_suggestion":
      return (
        <SimpleApproveReject
          item={item}
          ctx={ctx}
          endpoints={{
            approve: (id) =>
              `/api/admin/relationship-type-suggestions/${encodeURIComponent(id)}/approve`,
            reject: (id) =>
              `/api/admin/relationship-type-suggestions/${encodeURIComponent(id)}/reject`,
          }}
        />
      );
    case "gap_signal":
      return (
        <SimpleApproveReject
          item={item}
          ctx={ctx}
          endpoints={{
            dismiss: (id) => `/api/knowledge/gap-signals/${encodeURIComponent(id)}/dismiss`,
          }}
        />
      );
    default:
      return <div className="askHumanUnknownInline">{item.title}</div>;
  }
}

export function InboxRow({
  badge,
  title,
  preview,
  tag,
  children,
}: {
  badge: { label: string; tone: string };
  title: string;
  preview: string;
  tag?: { label: string; tone: string };
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="inboxRow">
      <button type="button" className="inboxRowHead" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        <span className={`inboxBadge inboxBadge--${badge.tone}`}>{badge.label}</span>
        <span className="inboxRowTitle">{title}</span>
        {tag && <span className={`inboxTag inboxTag--${tag.tone}`}>{tag.label}</span>}
        {!open && <span className="inboxRowPreview">{preview}</span>}
        <span className="inboxRowChevron">{open ? "▾" : "▸"}</span>
      </button>
      {open && <div className="inboxRowBody">{children}</div>}
    </div>
  );
}

function AskHumanView() {
  const { errors, summary, loading, fatalError, activeSource, setActiveSource, load } =
    useInboxStore();
  const [refreshNonce, setRefreshNonce] = useState(0);
  // 视图维度（与 pending 收件箱正交）：pending=待处理统一收件箱，resolved=已裁决历史只读回顾。
  const [showResolved, setShowResolved] = useState(false);

  // 所有刷新统一经 refreshNonce → ReviewQueue refetch → fetchItems → load()。
  // store 是唯一 fetch 来源，不再在这里额外 void load()（消除单次刷新打两次 /inbox）。
  const refreshAll = useCallback(() => {
    setRefreshNonce((n) => n + 1);
  }, []);

  // 必须 memoize：fetchItems 走 load() 会改 store → AskHumanView 重渲染。若每渲染重建此回调，
  // ReviewQueue 的 load(useCallback deps=[fetchItems]) 身份变 → 其 effect 重跑 → 再 load → 死循环。
  // load 是 zustand action，身份稳定，故 fetchItems 稳定，ReviewQueue 仅在 refreshToken 变时 refetch。
  const fetchItems = useCallback(async () => {
    await load(); // store 唯一 fetch：items(已排序)+errors+summary+降级保留；catch 不 rethrow
    return useInboxStore.getState().items; // load 成功取新值，失败取保留的旧值（不 throw → 列表显旧数据）
  }, [load]);

  const summaryErrors = summary?.errors ?? [];
  const unavailableSources = new Set(summaryErrors.map((error) => error.source));

  return (
    <div className="askHumanChannel">
      <div className="askHumanPanel">
        <div className="askHumanPanelHead">
          {/* total 为 null 表示计数不可用，此时不渲染——显示「待处理 0 项」是错误信息。
              按钮组靠 margin-left:auto 恒定贴右，故这里无需空元素占位。 */}
          {summary?.total != null && (
            <span className="askHumanPanelHeadCount">待处理 {summary.total} 项</span>
          )}
          <div className="askHumanHeaderActions askHumanHeader">
            <button
              type="button"
              className={
                showResolved ? "askHumanViewToggle askHumanViewToggle--active" : "askHumanViewToggle"
              }
              onClick={() => setShowResolved((v) => !v)}
            >
              {showResolved ? "待处理" : "已裁决历史"}
            </button>
            {!showResolved && (
              <button type="button" onClick={() => refreshAll()} disabled={loading}>
                刷新
              </button>
            )}
          </div>
        </div>

        {showResolved ? (
          <ResolvedEscalations />
        ) : (
          <>
            {fatalError && (
              <div className="askHumanFatal">加载失败（显示上次数据）：{fatalError}</div>
            )}
            {errors.length > 0 && (
              <div className="askHumanSourceErrors">
                {errors.length} 个来源暂时不可用：{errors.map((e) => SOURCE_META.find((m) => m.source === e.source)?.label ?? e.source).join("、")}
              </div>
            )}
            {summary && summary.status !== "complete" && (
              <div className="askHumanSourceErrors">
                待办计数部分不可用：
                {summaryErrors
                  .map((e) => SOURCE_META.find((m) => m.source === e.source)?.label ?? e.source)
                  .join("、") || "全部来源"}
              </div>
            )}

            <div className="askHumanToolbar askHumanSummary">
              {summary &&
                SOURCE_META.map(({ summaryKey, source, label }) => {
                  const count = summary.counts[summaryKey];
                  const unavailable = count == null || unavailableSources.has(source);
                  return (
                    <button
                      key={source}
                      type="button"
                      className={
                        activeSource === source
                          ? "askHumanSummaryChip askHumanSummaryChip--active"
                          : "askHumanSummaryChip"
                      }
                      onClick={() => {
                        const next = activeSource === source ? null : source;
                        setActiveSource(next); // 同步改 store.activeSource，load() 不传参时读它
                        setRefreshNonce((n) => n + 1); // 触发 ReviewQueue 重 fetch（经 load 读新 activeSource）
                      }}
                    >
                      {label}: {unavailable ? "不可用" : count}
                    </button>
                  );
                })}
            </div>

            <ReviewQueue<InboxItem>
              key={activeSource ?? "all"}
              refreshToken={refreshNonce}
              fetchItems={fetchItems}
              getId={(i) => `${i.source}:${i.id}`}
              renderItem={(item, ctx) => {
                const meta = SOURCE_META.find((m) => m.source === item.source);
                return (
                  <InboxRow
                    badge={{ label: meta?.label ?? item.source, tone: SOURCE_TONE[item.source] ?? "neutral" }}
                    title={item.title}
                    preview={item.summary ?? ""}
                    tag={
                      item.source === "knowledge_review" && item.integrityStatus === "needs_human_audit"
                        ? { label: "AI预审通过·待复核", tone: "held" }
                        : undefined
                    }
                  >
                    {item.actionKind === "rich"
                      ? renderRich(item, () => refreshAll())
                      : renderInline(item, ctx)}
                  </InboxRow>
                );
              }}
              emptyText={
                <EmptyState
                  title="暂无待处理项"
                  hint="AI 自主运行中，需要决策或审核的事项会自动出现在这里。"
                />
              }
            />
          </>
        )}
      </div>
    </div>
  );
}

export default function AskHumanFeature() {
  return (
    <ConfirmProvider>
      <ToastProvider>
        <AskHumanView />
      </ToastProvider>
    </ConfirmProvider>
  );
}
