// Backend outbox_payload_json media projection keys, locked by outbox_payload.fixture.json.
export const CANONICAL_KEYS = [
  "assetId",
  "fileName",
  "kind",
  "title",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];
