// Ask-Human Phase 2 Task 7：知识切片（steward 评审）的「单 chunk 处置」卡。源自
// steward 评审视图 ReviewView 的每行 verify/reject 渲染。中立化到
// components/review/ 后，老页（steward 评审队列）与统一收件箱频道都从这里 import，
// 单 chunk 的展示 + verify-gate + verify/reject 不再各持一份。
//
// 零跨feature import（用户裁定 B，Task 6 立的先例）：本卡片不依赖任何 feature 模块。
// verify-gate 是内联布尔逻辑（hasQuote && hasAnchor），无需借 steward 的 label/类型；
// 所以本任务无需提升任何原语到中立家。
//
// 字段命名实证：本卡片消费 Task 2 新增的 GET /api/operation-knowledge/chunks/:id，
// 该路由下发的是 **原始序列化的 OperationKnowledgeChunk**（src/routes/knowledge/crud.rs
// get_operation_knowledge_chunk → json!({ "item": item })），结构体无 rename_all，
// 因此字段是 **snake_case**（source_quote / source_anchors / integrity_status）。
// 这与 steward 列表（走 operation_knowledge_chunk_json 整形成 camelCase 的 sourceQuote/
// sourceAnchors）**形状不同**——卡片读 GET 拿到的 raw {item}，故必须用 snake_case。
//
// verify-gate 与 steward 逐字一致，红线不放宽：
//   hasQuote  = source_quote 非空且去空白后有内容
//   hasAnchor = source_anchors 长度 > 0
//   canVerify = hasQuote && hasAnchor（缺其一则 verify 按钮硬挡）
// AI 永不自动 verify：verify/reject 都是显式管理员维护动作。

import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";

// 字段子集，对齐 Task 2 GET /chunks/:id 下发的 raw OperationKnowledgeChunk（snake_case）。
// 只取卡片渲染 + verify-gate 需要的字段。
interface ChunkItem {
  _id?: unknown;
  title: string;
  body?: string | null;
  source_quote?: string | null;
  source_anchors?: unknown[] | null;
  integrity_status?: string | null;
}

export function ChunkReviewCard({ chunkId, onDone }: { chunkId: string; onDone?: () => void }) {
  const [chunk, setChunk] = useState<ChunkItem | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      const { item } = await api.get<{ item: ChunkItem }>(
        `/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}`,
      );
      setChunk(item);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [chunkId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function act(verb: "verify" | "reject") {
    setBusy(true);
    setError(null);
    try {
      await api.post(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/${verb}`, {});
      onDone?.();
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (error) return <div className="chunkReviewError">加载失败：{error}</div>;
  if (!chunk) return <div className="chunkReviewLoading">加载中…</div>;

  // verify-gate：与 steward ReviewView 逐字一致，不放宽红线。
  const hasQuote = !!chunk.source_quote && chunk.source_quote.trim().length > 0;
  const hasAnchor = (chunk.source_anchors?.length ?? 0) > 0;
  const canVerify = hasQuote && hasAnchor;

  return (
    <div className="chunkReviewCard">
      <div className="chunkReviewTitle">{chunk.title}</div>
      <div className="chunkReviewBody">{(chunk.body ?? "").slice(0, 200)}</div>
      {!canVerify && (
        <div className="chunkReviewGate">缺少 source_quote / anchor，未达 verify 门槛</div>
      )}
      <div className="chunkReviewActions">
        <button
          type="button"
          disabled={busy || !canVerify}
          title={!canVerify ? "verify gate：需 source_quote + source_anchors 全有" : "标记为 verified"}
          onClick={() => void act("verify")}
        >
          verify
        </button>
        <button type="button" disabled={busy} onClick={() => void act("reject")}>
          reject
        </button>
      </div>
    </div>
  );
}
