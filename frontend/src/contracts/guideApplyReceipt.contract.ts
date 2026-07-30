// Backend receipt_json projection keys, locked by guide_apply_receipt.fixture.json.
export const CANONICAL_KEYS = [
  "appliedFields",
  "candidateHash",
  "committed",
  "committedAt",
  "impactScope",
  "previewId",
  "skippedFields",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];
