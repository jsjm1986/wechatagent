import { useState, useCallback } from "react";

export interface GoLiveResult {
  ok: boolean;
  reason?: "apply_failed" | "gate_blocked" | "server_error";
  message?: string;
}

interface ChatApplyResponse {
  result?: { updatedAt?: string };
}

export async function runGoLive(input: {
  sessionId?: string;
  chunkId: string;
  expectedUpdatedAt: string;
  accountId?: string;
}): Promise<GoLiveResult> {
  try {
    // 无对话修改时核验用户实际看到的版本；有 session 时 apply 会产生新版本，必须
    // 改用 apply 回执里的 updatedAt，不能拿 apply 前快照去核验。
    let expectedUpdatedAt = input.expectedUpdatedAt.trim();
    if (!expectedUpdatedAt) return { ok: false, reason: "gate_blocked" };
    if (input.sessionId) {
      const applyResp = await fetch(
        `/api/operation-knowledge/chat/${encodeURIComponent(input.sessionId)}/apply`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ accountId: input.accountId || null }),
        },
      );
      if (!applyResp.ok) return { ok: false, reason: "apply_failed" };
      const applied = (await applyResp.json()) as ChatApplyResponse;
      const appliedVersion = applied.result?.updatedAt?.trim();
      if (!appliedVersion) return { ok: false, reason: "apply_failed" };
      expectedUpdatedAt = appliedVersion;
    }
    const verifyResp = await fetch(
      `/api/operation-knowledge/chunks/${encodeURIComponent(input.chunkId)}/verify`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ expectedUpdatedAt }),
      },
    );
    if (verifyResp.ok) return { ok: true };
    if (verifyResp.status >= 400 && verifyResp.status < 500) {
      return { ok: false, reason: "gate_blocked" };
    }
    return { ok: false, reason: "server_error" };
  } catch {
    return { ok: false, reason: "server_error" };
  }
}

export function useGoLive() {
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<GoLiveResult | null>(null);
  const goLive = useCallback(async (input: {
    sessionId?: string;
    chunkId: string;
    expectedUpdatedAt: string;
    accountId?: string;
  }) => {
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
