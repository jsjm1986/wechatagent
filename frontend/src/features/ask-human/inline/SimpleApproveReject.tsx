import { api } from "../../../lib/api";
import type { RowCtx } from "../../../components/review/ReviewQueue";
import type { InboxItem } from "../../../lib/inboxApi";
import {
  GAP_SIGNAL_KIND_LABELS,
  GAP_SIGNAL_SEVERITY_LABELS,
  labelOf,
} from "../../../lib/reviewLabels";

interface Endpoints {
  approve?: (id: string) => string; // 返回 POST url
  reject?: (id: string) => string;
  dismiss?: (id: string) => string;
}

export function SimpleApproveReject({
  item,
  ctx,
  endpoints,
}: {
  item: InboxItem;
  ctx: RowCtx;
  endpoints: Endpoints;
}) {
  return (
    <div className="simpleActionRow">
      <div className="simpleActionTitle">{item.title}</div>
      <div className="simpleActionSummary">{item.summary}</div>
      {(item.evidence || item.confidence !== undefined || item.occurrences !== undefined) && (
        <div className="simpleActionEvidence" style={{ fontSize: 12, color: "#666", marginTop: 4 }}>
          {item.evidence && <div>判断依据：{item.evidence}</div>}
          {item.confidence !== undefined && <div>置信度：{item.confidence}</div>}
          {item.occurrences !== undefined && <div>出现次数：{item.occurrences}</div>}
          {item.contactWxid && <div>客户标识：{item.contactWxid}</div>}
        </div>
      )}
      {(item.kind || item.signalSeverity) && (
        <div className="simpleActionMeta">
          {item.kind && <span>类型：{labelOf(GAP_SIGNAL_KIND_LABELS, item.kind)}</span>}
          {item.signalSeverity && (
            <span>严重度：{labelOf(GAP_SIGNAL_SEVERITY_LABELS, item.signalSeverity)}</span>
          )}
        </div>
      )}
      <div className="simpleActionButtons">
        {endpoints.approve && (
          <button
            type="button"
            disabled={ctx.busy}
            onClick={() => ctx.runAction(() => api.post(endpoints.approve!(item.id), {}), "已通过")}
          >
            通过
          </button>
        )}
        {endpoints.reject && (
          <button
            type="button"
            disabled={ctx.busy}
            onClick={() => ctx.runAction(() => api.post(endpoints.reject!(item.id), {}), "已拒绝")}
          >
            拒绝
          </button>
        )}
        {endpoints.dismiss && (
          <button
            type="button"
            disabled={ctx.busy}
            onClick={() => ctx.runAction(() => api.post(endpoints.dismiss!(item.id), {}), "已忽略")}
          >
            忽略
          </button>
        )}
      </div>
    </div>
  );
}
