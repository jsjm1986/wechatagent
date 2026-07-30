import { useEffect, useRef } from "react";
import type { Account } from "./types";
import { api } from "./lib/api";
import { Shell } from "./app/Shell";
import { GlobalErrorBanner } from "./app/GlobalErrorBanner";
import { useAccountStore } from "./stores/accountStore";
import { useUiStore } from "./stores/uiStore";
import { useProfileStore } from "./stores/profileStore";
import { invalidateChunks } from "./features/knowledge/chunkInvalidation";
// 频道视图已全部迁出至 features/*；App 只保留启动引导 + 全局 chunk WebSocket。
// 保留两个 re-export 让既有测试 `import { AutonomyOutcomesTab, formatRate } from "../App"` 继续解析。
export { AutonomyOutcomesTab } from "./features/autonomy";
export { formatRate } from "./lib/format";

// `useChunkEventStream` 在 App 顶层挂一次：连 ws://.../api/ws/chunks，把后端
// 推下来的 ChunkEvent 转成两类 window CustomEvent：
//   - `wikiChunkLocked` / `wikiChunkUnlocked`：lock 状态变迁
//   - `wikiChunkRevised`：chunk 被编辑（patch / archive / restore / split / merge / ...）
// Compatibility consumers may still listen to `wikiChunkRevised`; Knowledge
// collection views converge through `wikiChunksInvalidated`.
//
// 重连：onclose → 5s 后重试，最长 30s 退避。WebSocket/presence 失败不阻塞业务功能；
// 它只提供协作提示，写入一致性由后端认证、事务与 CAS 负责。
export type ChunkEventEnvelope =
  | { kind: "hello"; workspace: string }
  | { kind: "lagged" }
  | {
      kind: "locked";
      chunk_id: string;
      workspace_id: string;
      owner_user_id: string;
      owner_username: string;
      expires_at: string;
    }
  | {
      kind: "unlocked";
      chunk_id: string;
      workspace_id: string;
      owner_user_id: string;
    }
  | {
      kind: "revised";
      chunk_id: string;
      workspace_id: string;
      revision_kind: string;
      actor: string;
    };

export function dispatchChunkEvent(parsed: ChunkEventEnvelope): void {
  switch (parsed.kind) {
    case "hello":
      return;
    case "lagged":
      invalidateChunks({ reason: "lagged" });
      return;
    case "locked":
      window.dispatchEvent(new CustomEvent("wikiChunkLocked", { detail: parsed }));
      return;
    case "unlocked":
      window.dispatchEvent(new CustomEvent("wikiChunkUnlocked", { detail: parsed }));
      return;
    case "revised":
      window.dispatchEvent(new CustomEvent("wikiChunkRevised", { detail: parsed }));
      invalidateChunks({
        reason: "revised",
        chunkId: parsed.chunk_id,
        revisionKind: parsed.revision_kind,
      });
      return;
  }
}

function useChunkEventStream() {
  useEffect(() => {
    let socket: WebSocket | null = null;
    let cancelled = false;
    let backoffMs = 1000;
    let timer: number | null = null;

    const connect = () => {
      if (cancelled) return;
      const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
      const url = `${proto}//${window.location.host}/api/ws/chunks`;
      try {
        socket = new WebSocket(url);
      } catch {
        scheduleReconnect();
        return;
      }
      socket.onopen = () => {
        backoffMs = 1000;
      };
      socket.onmessage = (ev) => {
        let parsed: ChunkEventEnvelope | null = null;
        try {
          parsed = JSON.parse(typeof ev.data === "string" ? ev.data : "") as ChunkEventEnvelope;
        } catch {
          return;
        }
        if (!parsed) return;
        dispatchChunkEvent(parsed);
      };
      socket.onclose = () => {
        scheduleReconnect();
      };
      socket.onerror = () => {
        try {
          socket?.close();
        } catch {
          // ignore
        }
      };
    };

    const scheduleReconnect = () => {
      if (cancelled) return;
      if (timer != null) return;
      timer = window.setTimeout(() => {
        timer = null;
        connect();
      }, backoffMs);
      backoffMs = Math.min(backoffMs * 2, 30000);
    };

    connect();

    return () => {
      cancelled = true;
      if (timer != null) {
        window.clearTimeout(timer);
        timer = null;
      }
      try {
        socket?.close();
      } catch {
        // ignore
      }
    };
  }, []);
}


export function App() {
  // 登录态后挂一次 WebSocket，进程内所有 ChunkInspectorPane / 锁徽章共享。
  useChunkEventStream();

  // 启动引导：先拉 accounts 填 accountStore；账号收敛后由各 feature 自行加载所需数据。
  const accountsBootstrapRef = useRef(false);
  useEffect(() => {
    if (accountsBootstrapRef.current) return;
    accountsBootstrapRef.current = true;
    void api
      .get<{ items: Account[] }>("/api/accounts")
      .then((data) => useAccountStore.getState().setAccounts(data.items))
      .catch((err) => useUiStore.getState().setError(err instanceof Error ? err.message : String(err)));
    void useProfileStore.getState().loadActiveProfile();
    void useProfileStore.getState().loadActiveView();
  }, []);

  return (
    <>
      <GlobalErrorBanner />
      <Shell />
    </>
  );
}
