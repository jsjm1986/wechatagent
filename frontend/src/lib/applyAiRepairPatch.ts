// AI 修复 patch 落库 + 闭账。照 useGoLive.ts 的 runGoLive 形态返回 {ok,reason}，不抛错。
// 防清空：PUT body 从 originalChunk 出发，只用勾选字段覆盖。
// 红线：thenVerify 恒 false（落库只到 draft+needs_review，AI 永不自动 verify）。
export interface ApplyRepairInput {
  chunkId: string;
  originalChunk: Record<string, unknown>;
  patch: Record<string, unknown>;
  acceptedFieldNames: string[];
  sessionId: string;
  turn: number;
  confidenceHint: number;
  extras?: unknown;
}
export interface ApplyRepairResult {
  ok: boolean;
  reason?: "apply_failed" | "audit_failed" | "server_error";
  message?: string;
}

export async function applyAiRepairPatch(input: ApplyRepairInput): Promise<ApplyRepairResult> {
  const accepted = new Set(input.acceptedFieldNames);
  // 防清空：从原 chunk 值出发，只覆盖勾选字段。
  const putBody: Record<string, unknown> = { ...input.originalChunk };
  for (const name of input.acceptedFieldNames) {
    if (name in input.patch) putBody[name] = input.patch[name];
  }
  // skipped = patch 里有、但没勾选的字段名。
  const skippedFields = Object.keys(input.patch).filter((k) => k !== "extras" && !accepted.has(k));

  try {
    const putResp = await fetch(
      `/api/operation-knowledge/chunks/${encodeURIComponent(input.chunkId)}`,
      { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(putBody) },
    );
    if (!putResp.ok) return { ok: false, reason: "apply_failed" };

    const appliedResp = await fetch(
      `/api/operation-knowledge/repair/applied`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          targetKind: "chunk",
          targetId: input.chunkId,
          sessionId: input.sessionId,
          turn: input.turn,
          acceptedFields: input.acceptedFieldNames,
          skippedFields,
          confidenceHint: input.confidenceHint,
          extras: input.extras ?? null,
          thenVerify: false,
        }),
      },
    );
    if (!appliedResp.ok) {
      return { ok: false, reason: "audit_failed", message: "已落库为草稿，但审计记录写入失败" };
    }
    return { ok: true };
  } catch {
    return { ok: false, reason: "server_error" };
  }
}
