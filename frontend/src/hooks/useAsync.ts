import { useCallback, useEffect, useRef, useState } from "react";

export type AsyncStatus = "idle" | "loading" | "success" | "error";

export interface AsyncState<T> {
  status: AsyncStatus;
  data: T | null;
  error: Error | null;
}

/// 统一异步三态。替代散落的 useState(loading/error/data) + 9 种 loading 命名。
/// 配合 <AsyncView> 渲染 loading/empty/error/success 四态。
/// fn 内部应调 api.*（抛 Error / LlmUnavailableError），AsyncView 据类型分流。
export function useAsync<T>(
  fn: () => Promise<T>,
  opts: { immediate?: boolean; deps?: unknown[] } = {}
): AsyncState<T> & { run: () => Promise<void>; reload: () => void } {
  const { immediate = true, deps = [] } = opts;
  const [state, setState] = useState<AsyncState<T>>({
    status: immediate ? "loading" : "idle",
    data: null,
    error: null,
  });
  const fnRef = useRef(fn);
  fnRef.current = fn;
  // 防止已卸载后 setState，以及竞态：只接受最后一次 run 的结果
  const mountedRef = useRef(true);
  const callIdRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const run = useCallback(async () => {
    const myId = ++callIdRef.current;
    setState((s) => ({ ...s, status: "loading", error: null }));
    try {
      const data = await fnRef.current();
      if (!mountedRef.current || myId !== callIdRef.current) return;
      setState({ status: "success", data, error: null });
    } catch (e) {
      if (!mountedRef.current || myId !== callIdRef.current) return;
      setState({ status: "error", data: null, error: e instanceof Error ? e : new Error(String(e)) });
    }
  }, []);

  useEffect(() => {
    if (immediate) void run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return { ...state, run, reload: run };
}
