// AI 修复 patch 落库 + 闭账。服务端按 acceptedFields 从 proposal patch 取值，
// 统一经过 revision harness；客户端不再先 PUT 整个 Chunk。
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
  void input.originalChunk; // retained in the public type for caller compatibility
  // skipped = patch 里有、但没勾选的字段名。
  const skippedFields = Object.keys(input.patch).filter((k) => k !== "extras" && !accepted.has(k));

  try {
    const appliedResp = await fetch(
      `/api/operation-knowledge/repair/applied`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          targetKind: "chunk",
          targetId: input.chunkId,
          patch: input.patch,
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
      return { ok: false, reason: "apply_failed" };
    }
    return { ok: true };
  } catch {
    return { ok: false, reason: "server_error" };
  }
}
