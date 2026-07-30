import { useState } from "react";
import { api } from "../../lib/api";
import { labelOf } from "../../lib/reviewLabels";
import styles from "./TaxonomyCandidateReviewCard.module.css";

// 维度键 → 中文名。取值来自运行时产候选的写入点（decision/reaction/gateway）：
// customer_stage / intent_level / objection_type / concern_type / emotional_state
// / relationship_type。未收录经 labelOf 回落原 key，不硬失败。
export const TAXONOMY_KIND_LABELS: Record<string, string> = {
  customer_stage: "客户阶段",
  intent_level: "意向强度",
  objection_type: "异议类型",
  concern_type: "顾虑类型",
  emotional_state: "情绪状态",
  relationship_type: "关系类型",
};

export interface TaxonomyCandidate {
  id: string;
  scope: string;
  kind: string;
  rawValue: string;
  evidence?: string;
  confidence?: number;
  occurrences?: number;
  suggestedDisplayName?: string;
}

export function TaxonomyCandidateReviewCard({
  candidate,
  onDone,
}: {
  candidate: TaxonomyCandidate;
  onDone: () => void;
}) {
  const [mode, setMode] = useState<"approve" | "reject">("approve");
  const [id, setId] = useState(candidate.rawValue);
  const [label, setLabel] = useState(candidate.suggestedDisplayName || candidate.rawValue);
  const [aliases, setAliases] = useState("");
  const [description, setDescription] = useState(candidate.evidence ?? "");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [acting, setActing] = useState(false);

  const kindLabel = labelOf(TAXONOMY_KIND_LABELS, candidate.kind);

  async function submitApprove() {
    if (!id.trim() || !label.trim()) {
      setError("标签标识与显示名不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliasList = aliases.split(/[,，]/).map((a) => a.trim()).filter((a) => a.length > 0);
      const res = await api.postRaw<{ error?: string; message?: string; mergedIntoExisting?: boolean }>(
        `/api/admin/taxonomy-candidates/${candidate.id}/approve`,
        {
          canonicalValue: {
            id: id.trim(),
            label: label.trim(),
            aliases: aliasList,
            description: description.trim() || undefined,
          },
        },
      );
      if (res.status === 409) {
        setInfo(res.data?.message ?? "该字典条目已存在，候选已标记采纳。");
      } else if (!res.ok) {
        setError(res.data?.message ?? res.data?.error ?? `HTTP ${res.status}`);
        return;
      } else if (res.data?.mergedIntoExisting) {
        setInfo(`已并入已有标签：${id.trim()}`);
      } else {
        setInfo(`已采纳：${id.trim()}`);
      }
      onDone();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  async function submitReject() {
    if (!reason.trim()) {
      setError("驳回原因不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    try {
      await api.post(`/api/admin/taxonomy-candidates/${candidate.id}/reject`, { reason: reason.trim() });
      onDone();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  return (
    <div className={styles.card}>
      <p className={styles.intro}>
        AI 在和客户对话时，识别到一个「{kindLabel}」维度上尚未收录的取值：
        <span className={styles.rawValue}> {candidate.rawValue}</span>
        。采纳后它会作为正式标签存入字典，今后 AI 可稳定使用；驳回则丢弃这次建议。
      </p>
      {(candidate.evidence || candidate.confidence !== undefined || candidate.occurrences !== undefined) && (
        <div className={styles.evidence}>
          {candidate.evidence && <span>判断依据：{candidate.evidence}</span>}
          {candidate.confidence !== undefined && <span>置信度：{candidate.confidence}</span>}
          {candidate.occurrences !== undefined && <span>出现次数：{candidate.occurrences}</span>}
        </div>
      )}
      {error && <div className={styles.error}>{error}</div>}
      {info && <div className={styles.info}>{info}</div>}

      {mode === "approve" && (
        <div className={styles.form}>
          <label className={styles.field}>
            <span>标签标识（英文，如 price_objection）</span>
            <input className={styles.input} value={id} onChange={(e) => setId(e.target.value)} />
          </label>
          <label className={styles.field}>
            <span>显示名</span>
            <input className={styles.input} value={label} onChange={(e) => setLabel(e.target.value)} />
          </label>
          <label className={styles.field}>
            <span>别名（逗号分隔，可空；原始取值会自动并入）</span>
            <input className={styles.input} value={aliases} onChange={(e) => setAliases(e.target.value)} />
          </label>
          <label className={styles.field}>
            <span>描述（可空）</span>
            <textarea className={styles.textarea} value={description} onChange={(e) => setDescription(e.target.value)} />
          </label>
          <div className={styles.buttons}>
            <button type="button" onClick={() => void submitApprove()} disabled={acting}>采纳</button>
            <button type="button" onClick={() => { setMode("reject"); setError(null); }} disabled={acting}>驳回</button>
          </div>
        </div>
      )}

      {mode === "reject" && (
        <div className={styles.form}>
          <label className={styles.field}>
            <span>驳回原因</span>
            <input
              className={styles.input}
              value={reason}
              placeholder="如：无业务相关性 / 与现有条目重复"
              onChange={(e) => setReason(e.target.value)}
            />
          </label>
          <div className={styles.buttons}>
            <button type="button" onClick={() => void submitReject()} disabled={acting}>确认驳回</button>
            <button type="button" onClick={() => { setMode("approve"); setError(null); }} disabled={acting}>取消</button>
          </div>
        </div>
      )}
    </div>
  );
}
