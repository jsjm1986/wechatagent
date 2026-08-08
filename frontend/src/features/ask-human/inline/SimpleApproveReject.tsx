import { api } from "../../../lib/api";
import type { RowCtx } from "../../../components/review/ReviewQueue";
import type { InboxItem } from "../../../lib/inboxApi";
import {
  GAP_SIGNAL_KIND_LABELS,
  GAP_SIGNAL_SEVERITY_LABELS,
  labelOf,
} from "../../../lib/reviewLabels";

export interface Endpoints {
  approve?: (id: string) => string; // 返回 POST url
  reject?: (id: string) => string;
  dismiss?: (id: string) => string;
}

/** 卡体详情：只渲染行头容不下的字段（摘要、判断依据、类型/严重度）。
 *  处置按钮已抽到 `SimpleActionButtons`，由 InboxRow 常驻在行内右侧，
 *  故本组件不再需要 `ctx` / `endpoints`。 */
export function SimpleApproveReject({ item }: { item: InboxItem }) {
  return (
    <div className="simpleActionRow">
      {/* 标题不在此渲染：InboxRow 行头已显示同一个 item.title，体内再渲染一次
          会整段重复（长标题如「孤立 chunk：[reviewer-misjudge] …」尤其刺眼）。
          这里只放行头容不下的细节。 */}
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
    </div>
  );
}

/** 处置按钮（通过 / 拒绝 / 忽略），从卡体中抽出以便同时用在 InboxRow 行内右侧。
 *
 *  这类来源点一下就完事，没有需要填写的参数，所以按钮常驻行内、不必先展开——
 *  展开后卡体里那份是同一组按钮，两处共用本组件，避免端点与文案分叉。
 *  （请示裁决不适用：它要选裁决类型、写转述意见，表单塞不进行头。） */
export function SimpleActionButtons({
  item,
  ctx,
  endpoints,
}: {
  item: InboxItem;
  ctx: RowCtx;
  endpoints: Endpoints;
}) {
  return (
    <>
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
    </>
  );
}
