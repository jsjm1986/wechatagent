import { useState } from "react";
import { api } from "../../../lib/api";
import type { RowCtx } from "../../../components/review/ReviewQueue";
import type { InboxItem } from "../../../lib/inboxApi";
import { ESCALATION_CATEGORY_LABELS, labelOf } from "../../../lib/reviewLabels";

// 裁决口径闭集（与后端 ALLOWED_PRINCIPAL_VERDICT 同源；口径与 ESCALATION_VERDICT_LABELS 统一）
const VERDICT_OPTIONS: { value: string; label: string }[] = [
  { value: "approved", label: "同意" },
  { value: "rejected", label: "拒绝" },
  { value: "conditional", label: "有条件同意" },
  { value: "deferred", label: "暂缓待定" },
  { value: "delegated_back", label: "授权 AI 自行处理" },
];
const EXEMPTION_OPTIONS: { value: string; label: string }[] = [
  { value: "none", label: "仅本次裁决，不授予长期豁免" },
  { value: "customer_only", label: "仅该客户长期豁免（可撤销）" },
  { value: "knowledge", label: "该客户长期豁免，并沉淀为通用知识" },
];


export function EscalationInline({ item, ctx }: { item: InboxItem; ctx: RowCtx }) {
  const [substance, setSubstance] = useState("");
  const [verdict, setVerdict] = useState("approved");
  const [windowHours, setWindowHours] = useState("");
  const [constraintText, setConstraintText] = useState("");
  const [exemptionType, setExemptionType] = useState("none");
  const [reassignWxid, setReassignWxid] = useState("");
  const code = item.id; // escalation 的 id 即 short_code

  function resolve(v: string) {
    const label = VERDICT_OPTIONS.find((o) => o.value === v)?.label ?? v;
    const authorizes = v === "approved" || v === "conditional";
    void ctx.runAction(
      () =>
        api.post(`/api/admin/principal-escalations/${encodeURIComponent(code)}/resolve`, {
          verdict: v,
          substance,
          constraints: constraintText ? [constraintText] : [],
          authorizationWindowHours: authorizes && windowHours ? Number(windowHours) : null,
          exemptionType: authorizes ? exemptionType : "none",
        }),
      `裁决已提交（${label}）`,
    );
  }

  function reassign() {
    void ctx.runAction(
      () =>
        api.post(`/api/admin/principal-escalations/${encodeURIComponent(code)}/reassign`, {
          toWxid: reassignWxid,
        }),
      "已改派给备选决策人",
    );
  }

  return (
    <div className="escalationInline">
      <div className="escalationInlineTitle">{item.title}</div>
      <div className="escalationInlineSummary">{item.summary}</div>
      {(item.contactWxid || item.questionForPrincipal || item.category) && (
        <dl className="escalationInlineMeta">
          {item.contactWxid && (
            <>
              <dt>客户</dt>
              <dd>{item.contactWxid}</dd>
            </>
          )}
          {item.questionForPrincipal && (
            <>
              <dt>具体问题</dt>
              <dd>{item.questionForPrincipal}</dd>
            </>
          )}
          {item.category && (
            <>
              <dt>类别</dt>
              <dd>{labelOf(ESCALATION_CATEGORY_LABELS, item.category)}</dd>
            </>
          )}
        </dl>
      )}
      <label htmlFor={`verdict-${code}`}>裁决类型</label>
      <select
        id={`verdict-${code}`}
        value={verdict}
        onChange={(e) => setVerdict(e.target.value)}
      >
        {VERDICT_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      {(verdict === "approved" || verdict === "conditional") && (
        <>
          <label htmlFor={`window-${code}`}>本次转述有效期(小时，可空)</label>
          <input
            id={`window-${code}`}
            type="number"
            min="0.01"
            max="8760"
            value={windowHours}
            onChange={(e) => setWindowHours(e.target.value)}
          />
          <label htmlFor={`exemption-${code}`}>后续产品豁免范围</label>
          <select
            id={`exemption-${code}`}
            value={exemptionType}
            onChange={(e) => setExemptionType(e.target.value)}
          >
            {EXEMPTION_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
          {exemptionType !== "none" && (
            <div className="wikiHint">客户级豁免长期有效，不受上方本次转述期限影响；可在客户页显式撤销。</div>
          )}
          {verdict === "conditional" && (
            <input
              placeholder="约束条款"
              value={constraintText}
              onChange={(e) => setConstraintText(e.target.value)}
            />
          )}
        </>
      )}
      <textarea
        placeholder="裁决意见（转述给客户的内容）"
        value={substance}
        onChange={(e) => setSubstance(e.target.value)}
      />
      <div className="escalationInlineActions">
        <button type="button" disabled={ctx.busy} onClick={() => resolve(verdict)}>
          提交裁决
        </button>
      </div>
      <div className="escalationInlineReassign">
        <input
          placeholder="备选决策人 wxid"
          value={reassignWxid}
          onChange={(e) => setReassignWxid(e.target.value)}
        />
        <button type="button" disabled={ctx.busy} onClick={() => reassign()}>
          改派
        </button>
      </div>
    </div>
  );
}
