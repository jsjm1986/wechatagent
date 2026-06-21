import { useCallback, useEffect, useState } from "react";
import { ConfirmProvider } from "../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../components/ui/Toast";
import { useInboxStore } from "../../stores/inboxStore";
import { ReviewQueue, type RowCtx } from "../../components/review/ReviewQueue";
import { fetchInbox, type InboxItem } from "../../lib/inboxApi";
import { EscalationInline } from "./inline/EscalationInline";
import { SimpleApproveReject } from "./inline/SimpleApproveReject";
import { ChunkReviewCard } from "../../components/review/ChunkReviewCard";
import { ProfilePublishCard } from "../../components/review/ProfilePublishCard";
import { ProposalReleaseCard } from "../../components/review/ProposalReleaseCard";
import { LessonPromoteCard } from "../../components/review/LessonPromoteCard";
import "./AskHuman.css";

// summary 端点下发 camelCase key，inbox ?source= 过滤认 snake_case source id（两者由后端 ask_human_inbox.rs 定义）。
// 单一事实来源：summaryKey ↔ source ↔ 中文标签。chip 用它渲染并把 activeSource 统一存 snake_case source。
const SOURCE_META: { summaryKey: string; source: string; label: string }[] = [
  { summaryKey: "principalEscalation", source: "principal_escalation", label: "请示裁决" },
  { summaryKey: "knowledgeReview", source: "knowledge_review", label: "知识核验" },
  { summaryKey: "taxonomyCandidate", source: "taxonomy_candidate", label: "标签候选" },
  { summaryKey: "relationshipSuggestion", source: "relationship_suggestion", label: "关系建议" },
  { summaryKey: "gapSignal", source: "gap_signal", label: "知识缺口" },
  { summaryKey: "profileRisky", source: "profile_risky", label: "画像发布" },
  { summaryKey: "evolutionProposal", source: "evolution_proposal", label: "进化发布" },
  { summaryKey: "lessonsLearned", source: "lessons_learned", label: "经验晋升" },
];

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
    default:
      return <div className="askHumanUnknownRich">未知 rich 组件：{item.richComponent}</div>;
  }
}

// inline 分派：source → 处置器。端点 URL 已按 src/routes/mod.rs 实证修正。
function renderInline(item: InboxItem, ctx: RowCtx) {
  switch (item.source) {
    case "principal_escalation":
      return <EscalationInline item={item} ctx={ctx} />;
    case "taxonomy_candidate":
      return (
        <SimpleApproveReject
          item={item}
          ctx={ctx}
          endpoints={{
            approve: (id) => `/api/admin/taxonomy-candidates/${encodeURIComponent(id)}/approve`,
            reject: (id) => `/api/admin/taxonomy-candidates/${encodeURIComponent(id)}/reject`,
          }}
        />
      );
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

function AskHumanView() {
  const { errors, summary, loading, fatalError, activeSource, setActiveSource, load } =
    useInboxStore();
  const [refreshNonce, setRefreshNonce] = useState(0);

  const refreshAll = useCallback(
    (source?: string) => {
      setRefreshNonce((n) => n + 1);
      void load(source);
    },
    [load],
  );

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="askHumanChannel">
      <header className="askHumanHeader">
        <h1>统一收件箱</h1>
        <button type="button" onClick={() => refreshAll()} disabled={loading}>
          刷新
        </button>
      </header>

      {fatalError && (
        <div className="askHumanFatal">加载失败（显示上次数据）：{fatalError}</div>
      )}
      {errors.length > 0 && (
        <div className="askHumanSourceErrors">
          {errors.length} 个来源暂时不可用：{errors.map((e) => e.source).join("、")}
        </div>
      )}

      <div className="askHumanSummary">
        {summary &&
          SOURCE_META.map(({ summaryKey, source, label }) => (
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
                setActiveSource(next);
                refreshAll(next ?? undefined);
              }}
            >
              {label}: {summary[summaryKey] ?? 0}
            </button>
          ))}
      </div>

      <ReviewQueue<InboxItem>
        key={activeSource ?? "all"}
        refreshToken={refreshNonce}
        fetchItems={async () => (await fetchInbox(activeSource ?? undefined)).items}
        getId={(i) => `${i.source}:${i.id}`}
        renderItem={(item, ctx) =>
          item.actionKind === "rich" ? (
            <div className="askHumanRichRow">
              {renderRich(item, () => refreshAll(activeSource ?? undefined))}
            </div>
          ) : (
            <div className="askHumanInlineRow">{renderInline(item, ctx)}</div>
          )
        }
        emptyText="暂无待处理项"
      />
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
