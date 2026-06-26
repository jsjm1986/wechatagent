import { useState } from "react";
import { api } from "../../../lib/api";
import type { RowCtx } from "../../../components/review/ReviewQueue";
import type { InboxItem } from "../../../lib/inboxApi";

// 裁决口径闭集（与后端 ALLOWED_PRINCIPAL_VERDICT 同源）
const VERDICT_OPTIONS: { value: string; label: string }[] = [
  { value: "approved", label: "批准" },
  { value: "rejected", label: "驳回" },
  { value: "conditional", label: "有条件批准" },
  { value: "deferred", label: "暂缓" },
  { value: "delegated_back", label: "退回再议" },
];

export function EscalationInline({ item, ctx }: { item: InboxItem; ctx: RowCtx }) {
  const [substance, setSubstance] = useState("");
  const [verdict, setVerdict] = useState("approved");
  const [windowHours, setWindowHours] = useState("");
  const [constraintText, setConstraintText] = useState("");
  const [reassignWxid, setReassignWxid] = useState("");
  const code = item.id; // escalation 的 id 即 short_code

  function resolve(v: string) {
    const label = VERDICT_OPTIONS.find((o) => o.value === v)?.label ?? v;
    void ctx.runAction(
      () =>
        api.post(`/api/admin/principal-escalations/${encodeURIComponent(code)}/resolve`, {
          verdict: v,
          substance,
          constraints: constraintText ? [constraintText] : [],
          authorizationWindowHours:
            v === "conditional" && windowHours ? Number(windowHours) : null,
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
              <dd>{item.category}</dd>
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
      {verdict === "conditional" && (
        <>
          <label htmlFor={`window-${code}`}>授权窗(小时)</label>
          <input
            id={`window-${code}`}
            type="number"
            value={windowHours}
            onChange={(e) => setWindowHours(e.target.value)}
          />
          <input
            placeholder="约束条款"
            value={constraintText}
            onChange={(e) => setConstraintText(e.target.value)}
          />
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
