import { useState, useCallback } from "react";

export interface GoLiveResult {
  ok: boolean;
  reason?: "apply_failed" | "gate_blocked" | "server_error";
  message?: string;
}

export async function runGoLive(input: { sessionId?: string; chunkId: string }): Promise<GoLiveResult> {
  // 1. 有 sessionId 才先 apply(对话改过草稿);无则跳过直接 verify
  if (input.sessionId) {
    const applyResp = await fetch(
      `/api/operation-knowledge/chat/${encodeURIComponent(input.sessionId)}/apply`,
      { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
    );
    if (!applyResp.ok) {
      return { ok: false, reason: "apply_failed" };
    }
  }
  // 2. verify(过 D2 闸)
  const verifyResp = await fetch(
    `/api/operation-knowledge/chunks/${encodeURIComponent(input.chunkId)}/verify`,
    { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
  );
  if (verifyResp.ok) {
    return { ok: true };
  }
  // 4xx = D2 闸拒绝(缺锚点等),可恢复;5xx = 服务端错误
  if (verifyResp.status >= 400 && verifyResp.status < 500) {
    return { ok: false, reason: "gate_blocked" };
  }
  return { ok: false, reason: "server_error" };
}

export function useGoLive() {
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<GoLiveResult | null>(null);
  const goLive = useCallback(async (input: { sessionId?: string; chunkId: string }) => {
    setPending(true);
    try {
      const r = await runGoLive(input);
      setResult(r);
      return r;
    } finally {
      setPending(false);
    }
  }, []);
  return { goLive, pending, result };
}
