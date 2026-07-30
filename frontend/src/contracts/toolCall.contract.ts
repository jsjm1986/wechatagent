// Backend tool_call_json projection keys, locked by tool_call.fixture.json.
export const CANONICAL_KEYS = [
  "arguments",
  "error",
  "id",
  "response",
  "status",
  "toolName",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];
