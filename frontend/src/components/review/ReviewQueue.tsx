import { useCallback, useEffect, useState, type ReactNode } from "react";
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

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(await fetchItems());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [fetchItems]);

  useEffect(() => {
    void load();
  }, [load, refreshToken]);

  const makeCtx = (id: string): RowCtx => ({
    busy: busyId === id,
    runAction: async (fn, successMsg) => {
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
  return <div className="reviewQueueList">{items.map((it) => renderItem(it, makeCtx(getId(it))))}</div>;
}
