import { Fragment, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useToast } from "../ui/Toast";

export interface RowCtx {
  busy: boolean;
  /// 置 busy → await fn → 成功 toast.success + refetch → 失败 toast.error。统一惯例。
  runAction: (fn: () => Promise<unknown>, successMsg?: string) => Promise<void>;
}

interface ReviewQueueProps<T> {
  fetchItems: () => Promise<T[]>;
  getId: (item: T) => string;
  renderItem: (item: T, ctx: RowCtx) => ReactNode;
  emptyText?: string;
  refreshToken?: number;
}

export function ReviewQueue<T>({
  fetchItems,
  getId,
  renderItem,
  emptyText,
  refreshToken,
}: ReviewQueueProps<T>) {
  const toast = useToast();
  const [items, setItems] = useState<T[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [listGeneration, setListGeneration] = useState(0);
  const acceptedGenerationRef = useRef(0);
  const acceptedIdsRef = useRef<Set<string>>(new Set());
  const loadingRef = useRef(false);
  const loadRequestRef = useRef(0);
  const getIdRef = useRef(getId);
  getIdRef.current = getId;

  const load = useCallback(async () => {
    const request = ++loadRequestRef.current;
    loadingRef.current = true;
    setLoading(true);
    setError(null);
    try {
      const nextItems = await fetchItems();
      if (request !== loadRequestRef.current) return;
      const nextGeneration = acceptedGenerationRef.current + 1;
      acceptedGenerationRef.current = nextGeneration;
      acceptedIdsRef.current = new Set(nextItems.map((item) => getIdRef.current(item)));
      setItems(nextItems);
      setListGeneration(nextGeneration);
    } catch (e) {
      if (request === loadRequestRef.current) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (request === loadRequestRef.current) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }, [fetchItems]);

  useEffect(() => {
    void load();
  }, [load, refreshToken]);

  const makeCtx = (id: string, generation: number): RowCtx => ({
    busy: loading || busyId === id,
    runAction: async (fn, successMsg) => {
      if (
        loadingRef.current
        || acceptedGenerationRef.current !== generation
        || !acceptedIdsRef.current.has(id)
      ) {
        toast.error("待办列表已刷新，请在最新条目上重新操作");
        return;
      }
      setBusyId(id);
      try {
        await fn();
        toast.success(successMsg ?? "已处置");
        await load();
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        setBusyId(null);
      }
    },
  });

  if (loading && items.length === 0) return <div className="reviewQueueLoading">加载中…</div>;
  if (error) return <div className="reviewQueueError">加载失败：{error}</div>;
  if (items.length === 0) return <div className="reviewQueueEmpty">{emptyText ?? "暂无待处理项"}</div>;
  return (
    <div className="reviewQueueList">
      {items.map((item) => {
        const id = getId(item);
        return (
          <Fragment key={id}>
            {renderItem(item, makeCtx(id, listGeneration))}
          </Fragment>
        );
      })}
    </div>
  );
}
