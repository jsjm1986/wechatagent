// 决策复盘下钻（Task 5）。会话流 verbatim 迁移自 CockpitPanel/legacy 的 conversation 块
// （ConversationStream + reviewList），数据流不变。新增：每条复盘可展开显示
// DecisionReview 上真实存在的结构化字段（scores 打分 / risks 风险 / nextBestAction /
// finalReviewStatus / holdCategory），把这些被列表压掉的自治判断证据上前台。
//
// 说明：自治协议独白字段（selfCritique / whyShouldReply / userUnderstanding）并不在
// DecisionReview 类型上（它们由 features/autonomy 的独立聚合接口提供），本视图只渲染
// DecisionReview 真实携带的字段，避免渲染 undefined / tsc 报错。
import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import type { AutonomyProtocol, DecisionReview, Message } from "../../../../types";
import { ConversationStream, EmptyInline, formatTime, nextBestActionLabel } from "../../legacy";
import { FINAL_REVIEW_STATUS_LABELS, HOLD_CATEGORY_LABELS, labelOf } from "../../../../lib/reviewLabels";
import styles from "../cockpit.module.css";

function scoreEntries(scores?: Record<string, number>): Array<[string, number]> {
  if (!scores) return [];
  return Object.entries(scores).filter(([, v]) => typeof v === "number");
}

const PROTOCOL_GROUPS: Array<{ title: string; fields: Array<[keyof AutonomyProtocol, string]> }> = [
  { title: "回复决策", fields: [["whyShouldReply", "为何回复"], ["whySkipReply", "为何不回复"], ["selfCritique", "自我批判"]] },
  { title: "理解", fields: [["userUnderstanding", "用户理解"], ["relationshipRead", "关系解读"], ["operationGoal", "运营目标"]] },
  { title: "运营依据", fields: [["knowledgeNeedReason", "知识需求"], ["memoryUpdateReason", "记忆更新理由"], ["riskSelfCheck", "风险自查"]] },
];

export function AutonomyProtocolView({ protocol }: { protocol: AutonomyProtocol }) {
  return (
    <div className={styles.protocolSection}>
      <div className={styles.protocolHeading}>AI 内心独白</div>
      {PROTOCOL_GROUPS.map((group) => {
        const rows = group.fields.filter(([key]) => (protocol[key] ?? "").trim() !== "");
        if (rows.length === 0) return null;
        return (
          <div key={group.title} className={styles.protocolGroup}>
            <div className={styles.protocolGroupTitle}>{group.title}</div>
            {rows.map(([key, label]) => (
              <div key={key} className={styles.protocolField}>
                <span className={styles.protocolLabel}>{label}</span>
                <p className={styles.protocolText}>{protocol[key]}</p>
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}

function ReviewItem({ review }: { review: DecisionReview }) {
  const [expanded, setExpanded] = useState(false);
  const scores = scoreEntries(review.scores);
  const risks = Array.isArray(review.risks) ? review.risks : [];
  const nextAction = review.nextBestAction ? nextBestActionLabel(review.nextBestAction) : "";
  const protocol = review.autonomyProtocol;
  const hasProtocol =
    !!protocol && PROTOCOL_GROUPS.some((g) => g.fields.some(([k]) => (protocol[k] ?? "").trim() !== ""));
  const hasDetail =
    scores.length > 0 ||
    risks.length > 0 ||
    Boolean(nextAction && nextAction !== "-") ||
    Boolean(review.finalReviewStatus) ||
    Boolean(review.holdCategory);

  return (
    <div className="reviewItem">
      <strong>
        {review.approved ? "通过" : "拦截"} / {review.operationState || "未记录状态"}
      </strong>
      <p>{review.reviewSummary || review.replyText || "-"}</p>
      <span>{formatTime(review.createdAt)}</span>
      {(hasDetail || hasProtocol) && (
        <button
          type="button"
          className={styles.reviewToggle}
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "收起判断依据" : "展开判断依据"}
        </button>
      )}
      {expanded && (hasDetail || hasProtocol) && (
        <div className={styles.reviewDetail}>
          {(review.finalReviewStatus || review.holdCategory) && (
            <div className={styles.reviewMetaRow}>
              {review.finalReviewStatus && (
                <span className={styles.reviewChip}>终审 {labelOf(FINAL_REVIEW_STATUS_LABELS, review.finalReviewStatus)}</span>
              )}
              {review.holdCategory && (
                <span className={styles.reviewChip}>暂缓类别 {labelOf(HOLD_CATEGORY_LABELS, review.holdCategory)}</span>
              )}
            </div>
          )}
          {scores.length > 0 && (
            <div className={styles.reviewScores}>
              {scores.map(([key, value]) => (
                <span key={key} className={styles.reviewChip} title={key}>
                  {key} {value}
                </span>
              ))}
            </div>
          )}
          {risks.length > 0 && (
            <div className="riskList compact">
              {risks.map((risk, index) => (
                <span key={`${risk}-${index}`}>{risk}</span>
              ))}
            </div>
          )}
          {nextAction && nextAction !== "-" && (
            <p className={styles.reviewNextAction}>下一步建议：{nextAction}</p>
          )}
          {hasProtocol && protocol && <AutonomyProtocolView protocol={protocol} />}
        </div>
      )}
    </div>
  );
}

export function ConversationReviewView({
  messages,
  decisionReviews,
  onBack
}: {
  messages: Message[];
  decisionReviews: DecisionReview[];
  onBack: () => void;
}) {
  return (
    <section className="smartTabPanel">
      <div className={styles.drilldownHead}>
        <button className={styles.backButton} type="button" onClick={onBack}>
          <ArrowLeft size={15} />
          返回
        </button>
        <strong>会话记录</strong>
      </div>

      <section className="conversationGrid">
        <ConversationStream messages={messages} />

        <div className="reviewList">
          <div className="sectionCaption">最近复盘</div>
          {decisionReviews.slice(0, 8).map((review) => (
            <ReviewItem key={review.id} review={review} />
          ))}
          {!decisionReviews.length && <EmptyInline text="暂无决策复盘" />}
        </div>
      </section>
    </section>
  );
}
