import { useCallback, useRef } from "react";

export const CHUNKS_INVALIDATED_EVENT = "wikiChunksInvalidated";

export interface ChunkInvalidationDetail {
  reason: "revised" | "lagged" | "local";
  chunkId?: string;
  revisionKind?: string;
}

export function invalidateChunks(detail: ChunkInvalidationDetail): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent(CHUNKS_INVALIDATED_EVENT, { detail }));
}

/**
 * Coalesce invalidations that arrive while a collection reload is running.
 * One trailing reload is retained so an event observed during the request is
 * never lost, while bursts cannot create unbounded concurrent list requests.
 */
export function useCoalescedReload(loadOnce: () => Promise<void>): () => Promise<void> {
  const latestLoad = useRef(loadOnce);
  latestLoad.current = loadOnce;
  const running = useRef<Promise<void> | null>(null);
  const pending = useRef(false);

  return useCallback(() => {
    pending.current = true;
    if (running.current) return running.current;

    const task = (async () => {
      while (pending.current) {
        pending.current = false;
        await latestLoad.current();
      }
    })();
    const settled = task.finally(() => {
      if (running.current === settled) running.current = null;
    });
    running.current = settled;
    return settled;
  }, []);
}
