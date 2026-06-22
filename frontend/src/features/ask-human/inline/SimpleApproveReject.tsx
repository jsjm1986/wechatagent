import { api } from "../../../lib/api";
import type { RowCtx } from "../../../components/review/ReviewQueue";
import type { InboxItem } from "../../../lib/inboxApi";

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
