import { useEffect } from "react";
import type { FormEvent } from "react";
import { Contact, UserRoundPlus } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { useAccountStore } from "../../stores/accountStore";
import { useUiStore } from "../../stores/uiStore";
import { useReferralCardStore } from "../../stores/referralCardStore";
import type { ReferralCard } from "../../types";
import styles from "./ReferralCards.module.css";

export default function ReferralCardsFeature() {
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const busy = useUiStore((s) => s.busy);

  const {
    cards,
    cardDraft,
    setCardDraft,
    loadCards,
    createCard,
    reviewCard,
    toggleCard,
    deleteCard
  } = useReferralCardStore();

  useEffect(() => {
    void loadCards();
  }, [loadCards]);

  const handleCreate = async (event: FormEvent) => {
    event.preventDefault();
    await createCard(currentAccountId);
  };

  const handleDelete = (card: ReferralCard) => {
    if (window.confirm(`确认删除专属顾问名片「${card.displayName}」？删除后 AI 将不再引荐该顾问。`)) {
      void deleteCard(card.id);
    }
  };

  return (
    <div className={styles.page}>
      <p className={styles.intro}>
        管理可由 AI 主动引荐给客户的真人「专属顾问」名片。开启账号「辅助模式」后，AI
        会在客户契合下方标注的引荐条件时，主动把已审核启用的顾问名片推送给客户，由顾问完成临门一脚，AI
        随后退为辅助答疑角色。新建名片默认为草稿且停用，须管理员标记为「可引荐」并启用后 AI 才会选用。
      </p>
      <p className={styles.notice}>
        辅助模式为账号级开关（默认关闭），对应运营域配置的 assist_mode_enabled
        字段，需在账号/运营域配置开启。当前后端尚未通过运营域接口暴露该字段的读写，前端开关接线留作后续；
        本页仅维护名片库本身。
      </p>

      <div className={styles.workbench}>
        <section className={styles.panel}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>Referral Cards</span>
              <span className={styles.title}>专属顾问名片库</span>
            </div>
            <span className={styles.headIcon}><Contact size={17} /></span>
          </div>

          {cards.length === 0 ? (
            <EmptyState
              title="暂无专属顾问名片"
              hint="在右侧录入一位真人顾问的名片与引荐条件，审核启用后供 AI 在辅助模式下主动引荐。"
            />
          ) : (
            <div className={styles.list}>
              {cards.map((card) => (
                <ReferralCardRow
                  key={card.id}
                  card={card}
                  busy={busy}
                  onApprove={() => void reviewCard(card.id, "approved")}
                  onRevoke={() => void reviewCard(card.id, "draft")}
                  onToggle={() => void toggleCard(card.id, !card.enabled)}
                  onDelete={() => handleDelete(card)}
                />
              ))}
            </div>
          )}
        </section>

        <form className={styles.panel} onSubmit={handleCreate}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>新增</span>
              <span className={styles.title}>录入专属顾问</span>
            </div>
            <span className={styles.headIcon}><UserRoundPlus size={17} /></span>
          </div>

          <div className={styles.form}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>顾问名称</span>
              <input
                className={styles.input}
                placeholder="例如：张顾问"
                value={cardDraft.displayName}
                onChange={(event) => setCardDraft({ ...cardDraft, displayName: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>顾问微信号（targetWxid）</span>
              <input
                className={styles.input}
                placeholder="用于发送名片的微信 wxid"
                value={cardDraft.targetWxid}
                onChange={(event) => setCardDraft({ ...cardDraft, targetWxid: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>引荐时机（自然语言）</span>
              <textarea
                className={styles.textarea}
                placeholder="例如：客户明确要签约或要到店参观时引荐"
                value={cardDraft.sendTriggerHint}
                onChange={(event) => setCardDraft({ ...cardDraft, sendTriggerHint: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>目标阶段（逗号分隔，可留空）</span>
              <input
                className={styles.input}
                placeholder="例如：意向,待成交"
                value={cardDraft.targetStages}
                onChange={(event) => setCardDraft({ ...cardDraft, targetStages: event.target.value })}
              />
            </label>
            <button
              className={styles.submit}
              type="submit"
              disabled={busy || !cardDraft.displayName.trim() || !cardDraft.targetWxid.trim()}
            >
              保存（待审核）
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function ReferralCardRow({
  card,
  busy,
  onApprove,
  onRevoke,
  onToggle,
  onDelete
}: {
  card: ReferralCard;
  busy: boolean;
  onApprove: () => void;
  onRevoke: () => void;
  onToggle: () => void;
  onDelete: () => void;
}) {
  const isApproved = card.reviewStatus === "approved";
  return (
    <div className={styles.row}>
      <div className={styles.rowHead}>
        <strong className={styles.rowTitle}>{card.displayName}</strong>
        <div className={styles.badges}>
          <span className={`${styles.badge} ${isApproved ? styles.badgeApproved : styles.badgeDraft}`}>
            {isApproved ? "可引荐" : "待审核（草稿）"}
          </span>
          <span className={`${styles.badge} ${card.enabled ? styles.badgeOn : styles.badgeOff}`}>
            {card.enabled ? "已启用" : "已停用"}
          </span>
        </div>
      </div>
      <p className={styles.metaLine}>微信号：{card.targetWxid}</p>
      {card.sendTriggerHint && (
        <p className={styles.metaLine}>引荐时机：{card.sendTriggerHint}</p>
      )}
      {card.targetStages.length > 0 && (
        <p className={styles.metaLine}>目标阶段：{card.targetStages.join("、")}</p>
      )}
      <div className={styles.actions}>
        {isApproved ? (
          <button className={styles.ghostBtn} type="button" disabled={busy} onClick={onRevoke}>
            撤回为草稿
          </button>
        ) : (
          <button className={styles.reviewBtn} type="button" disabled={busy} onClick={onApprove}>
            标记为可引荐
          </button>
        )}
        <button className={styles.ghostBtn} type="button" disabled={busy} onClick={onToggle}>
          {card.enabled ? "停用" : "启用"}
        </button>
        <button className={styles.dangerBtn} type="button" disabled={busy} onClick={onDelete}>
          删除
        </button>
      </div>
    </div>
  );
}
