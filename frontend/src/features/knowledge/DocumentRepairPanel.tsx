import { useEffect, useState, useCallback } from "react";
import { ChunkRepairPanel } from "./ChunkRepairPanel";
import { integrityStatusLabel } from "./labels";
import { invalidateChunks } from "./chunkInvalidation";

interface ChunkView {
  id: string;
  documentId?: string | null;
  title?: string;
  integrityStatus?: string;
  [k: string]: unknown;
}

export function DocumentRepairPanel({
  documentId,
  documentTitle,
  onClose,
  onRepaired,
}: {
  documentId: string;
  documentTitle?: string;
  onClose?: () => void;
  onRepaired?: () => void;
}) {
  const [chunks, setChunks] = useState<ChunkView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [doneIds, setDoneIds] = useState<Set<string>>(new Set());

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/documents/${encodeURIComponent(documentId)}/chunks`,
      );
      if (!r.ok) throw new Error("加载失败");
      const data = (await r.json()) as { items?: ChunkView[] };
      setChunks(data.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, [documentId]);

  useEffect(() => { void reload(); }, [reload]);

  const needsReview = chunks.filter(
    (c) => c.integrityStatus === "needs_review" && !doneIds.has(c.id),
  );

  return (
    <section className="wikiDocRepair">
      <header className="wikiDocRepairHead">
        <h3>批量 AI 修复{documentTitle ? `：${documentTitle}` : ""}</h3>
        {onClose ? (
          <button type="button" className="wikiDocRepairClose" onClick={onClose}>关闭</button>
        ) : null}
      </header>
      {error ? (
        <div className="wikiAlert error">{error}</div>
      ) : loading ? (
        <div className="wikiDocRepairHint">加载待修切片…</div>
      ) : needsReview.length === 0 ? (
        <div className="wikiDocRepairHint">该文档无待修切片（needs_review）。</div>
      ) : (
        <ul className="wikiDocRepairList">
          {needsReview.map((chunk) => (
            <li className="wikiDocRepairItem" key={chunk.id}>
              <button
                type="button"
                className="wikiDocRepairItemHead"
                onClick={() => setExpandedId(expandedId === chunk.id ? null : chunk.id)}
              >
                <span className="wikiDocRepairItemTitle">{chunk.title || chunk.id}</span>
                <span className="wikiDocRepairItemTag">{integrityStatusLabel("needs_review")}</span>
              </button>
              {expandedId === chunk.id ? (
                <ChunkRepairPanel
                  chunkId={chunk.id}
                  originalChunk={chunk as unknown as Record<string, unknown>}
                  onApplied={() => {
                    setDoneIds((prev) => new Set(prev).add(chunk.id));
                    setExpandedId(null);
                    invalidateChunks({ reason: "local", chunkId: chunk.id });
                    onRepaired?.();
                  }}
                />
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
