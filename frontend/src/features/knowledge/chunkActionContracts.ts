export type ChunkRelationKind =
  | "references"
  | "requires"
  | "contradicts"
  | "clarifies"
  | "refines"
  | "superseded_by";

export function chunkPatchRequest(summary: string) {
  return { patch: { summary } } as const;
}

export function chunkSplitRequest(offset: number) {
  return { offset } as const;
}

export function chunkMergeRequest(targetId: string) {
  return { targetId } as const;
}

export function chunkRelateRequest(
  targetId: string,
  kind: ChunkRelationKind,
  note: string,
) {
  return { targetId, kind, note: note || null } as const;
}
