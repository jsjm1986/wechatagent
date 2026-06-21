import { useState } from "react";
import { api } from "../../../lib/api";
import type { RowCtx } from "../../../components/review/ReviewQueue";
import type { InboxItem } from "../../../lib/inboxApi";

export function EscalationInline({ item, ctx }: { item: InboxItem; ctx: RowCtx }) {
  const [substance, setSubstance] = useState("");
  const code = item.id; // escalation 的 id 即 short_code

  function resolve(verdict: "approved" | "rejected") {
    void ctx.runAction(
      () =>
        api.post(`/api/admin/principal-escalations/${encodeURIComponent(code)}/resolve`, {
          verdict,
          substance,
          constraints: [],
          authorizationWindowHours: null,
        }),
      verdict === "approved" ? "已批准并转述" : "已驳回",
    );
  }

  return (
    <div className="escalationInline">
      <div className="escalationInlineTitle">{item.title}</div>
      <div className="escalationInlineSummary">{item.summary}</div>
      <textarea
        placeholder="裁决意见（转述给客户的内容）"
        value={substance}
        onChange={(e) => setSubstance(e.target.value)}
      />
      <div className="escalationInlineActions">
        <button type="button" disabled={ctx.busy} onClick={() => resolve("approved")}>
          批准
        </button>
        <button type="button" disabled={ctx.busy} onClick={() => resolve("rejected")}>
          驳回
        </button>
      </div>
    </div>
  );
}
