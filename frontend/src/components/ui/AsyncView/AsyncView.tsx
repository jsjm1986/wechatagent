import type { ReactNode } from "react";
import { EmptyState } from "../EmptyState";
import type { AsyncState } from "../../../hooks/useAsync";
import styles from "./AsyncView.module.css";

/// 统一渲染 useAsync 的四态：loading→骨架、error→错误卡+重试、empty→EmptyState、success→children。
/// errorRender 槽：knowledge 频道传入 LlmErrorBanner（区分 LLM 不可用 vs 普通错误），
/// 不传则用内置通用错误卡。保持 ui 层不反向依赖 features/。
export function AsyncView<T>({
  state,
  children,
  onRetry,
  retrying,
  isEmpty,
  loading,
  emptyTitle = "暂无数据",
  emptyHint,
  errorRender,
}: {
  state: AsyncState<T>;
  children: (data: T) => ReactNode;
  onRetry?: () => void;
  retrying?: boolean;
  isEmpty?: (data: T) => boolean;
  loading?: ReactNode;
  emptyTitle?: string;
  emptyHint?: string;
  errorRender?: (error: Error, onRetry?: () => void, retrying?: boolean) => ReactNode;
}) {
  if (state.status === "loading" || state.status === "idle") {
    return <>{loading ?? <DefaultSkeleton />}</>;
  }
  if (state.status === "error" && state.error) {
    if (errorRender) return <>{errorRender(state.error, onRetry, retrying)}</>;
    return (
      <div className={styles.error} role="alert">
        <p className={styles.errorMsg}>{state.error.message || "加载失败"}</p>
        {onRetry && (
          <button type="button" className={styles.retry} onClick={onRetry} disabled={retrying}>
            {retrying ? "重试中…" : "重试"}
          </button>
        )}
      </div>
    );
  }
  const data = state.data as T;
  if (isEmpty && isEmpty(data)) {
    return <EmptyState title={emptyTitle} hint={emptyHint} />;
  }
  return <>{children(data)}</>;
}

function DefaultSkeleton() {
  return (
    <div className={styles.skeleton} aria-busy="true" aria-label="加载中">
      <div className={styles.skelRow} />
      <div className={styles.skelRow} />
      <div className={styles.skelRow} />
    </div>
  );
}
