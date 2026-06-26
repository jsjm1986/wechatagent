import { useState } from "react";
import type { Contact } from "../../types";
import BayesianTrendChart from "./BayesianTrendChart";
import styles from "./TagTrustPanel.module.css";

// AI 确信来源（后端 ConfirmedTag.confirmedBy 闭集）→ 中文标签 + tooltip 说明。
// strong_evidence = 直接证据快通道确信；consolidation = 记忆压缩时整体重新判定确信。
// 未知/缺省 → 返回 undefined（不显徽标，不崩）。
const CONFIRMED_BY_META: Record<string, { label: string; hint: string }> = {
  strong_evidence: { label: "强证据", hint: "直接证据快通道确信，可信度较高" },
  consolidation: { label: "压缩重判", hint: "记忆压缩时整体重新判定确信" },
};

/**
 * 标签可信度改造 - 三层标签面板（子计划5 Task2）。
 * 物理分离三层，对应改造的可信度模型：
 *  1) 运营录入层（权威）：manualTags，可编辑自由文本逗号分隔，经 onSaveManualTags 落库。中性色——这是人的层，不是 AI。
 *  2) AI 确信层（可能调整）：confirmedTags，只读 chip，每条展示证据条数。紫色系标 AI 身份。
 *  3) 贝叶斯评估层：占位 section，Task 3 填走势图。紫色系标 AI 身份。
 */
export default function TagTrustPanel({
  contact,
  onSaveManualTags
}: {
  contact: Contact;
  onSaveManualTags: (tags: string[]) => void;
}) {
  const manualTags = contact.manualTags || [];
  const confirmedTags = contact.confirmedTags || [];

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  const startEdit = () => {
    setDraft(manualTags.join(", "));
    setEditing(true);
  };

  const save = () => {
    const tags = draft
      .split(/[,，]/)
      .map((t) => t.trim())
      .filter((t) => t.length > 0);
    onSaveManualTags(tags);
    setEditing(false);
  };

  return (
    <section className={styles.panel}>
      {/* 第一层：运营录入（权威）—— 中性 ink 色，人的层 */}
      <section className={styles.layer}>
        <div className={styles.layerHead}>
          <div className={styles.layerTitleWrap}>
            <span className={styles.layerTitle}>运营录入</span>
            <span className={styles.layerNote}>权威</span>
          </div>
          {!editing && (
            <button type="button" className={styles.editBtn} onClick={startEdit}>
              编辑
            </button>
          )}
        </div>
        {editing ? (
          <div className={styles.editRow}>
            <input
              className={styles.input}
              value={draft}
              placeholder="多个标签用逗号分隔，例如：VIP, 老客户"
              onChange={(e) => setDraft(e.target.value)}
            />
            <button type="button" className={styles.saveBtn} onClick={save}>
              保存
            </button>
            <button type="button" className={styles.cancelBtn} onClick={() => setEditing(false)}>
              取消
            </button>
          </div>
        ) : (
          <div className={styles.chips}>
            {manualTags.length > 0 ? (
              manualTags.map((tag) => (
                <span key={tag} className={styles.manualChip}>
                  {tag}
                </span>
              ))
            ) : (
              <span className={styles.empty}>暂无运营录入标签</span>
            )}
          </div>
        )}
      </section>

      {/* 第二层：AI 确信层（可能调整）—— 紫色系，AI 身份，只读，带证据条数 */}
      <section className={styles.layer}>
        <div className={styles.layerHead}>
          <div className={styles.layerTitleWrap}>
            <span className={`${styles.layerTitle} ${styles.aiTitle}`}>AI 判断</span>
            <span className={styles.layerNote}>可能调整</span>
          </div>
        </div>
        <div className={styles.chips}>
          {confirmedTags.length > 0 ? (
            confirmedTags.map((tag) => {
              const meta = CONFIRMED_BY_META[tag.confirmedBy];
              return (
                <span key={tag.value} className={styles.aiChip}>
                  {tag.value}
                  <span className={styles.evidenceCount}>{tag.evidences.length} 条证据</span>
                  {meta ? (
                    <span className={styles.confirmedBySource} title={meta.hint}>
                      {meta.label}
                    </span>
                  ) : null}
                </span>
              );
            })
          ) : (
            <span className={styles.empty}>AI 暂未确信任何标签</span>
          )}
        </div>
      </section>

      {/* 第三层：贝叶斯评估层 —— 占位，Task 3 填走势图 */}
      <section className={`${styles.layer} ${styles.bayesianLayer}`}>
        <div className={styles.layerHead}>
          <div className={styles.layerTitleWrap}>
            <span className={`${styles.layerTitle} ${styles.aiTitle}`}>贝叶斯评估</span>
            <span className={styles.layerNote}>持续观测，永不驱动行为</span>
          </div>
        </div>
        <BayesianTrendChart signals={contact.bayesianSignals ?? []} />
      </section>
    </section>
  );
}
