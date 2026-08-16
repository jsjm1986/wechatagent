// Backend appointment_json projection keys, locked by appointment.fixture.json.
export const CANONICAL_KEYS = [
  "accountId",
  "confirmationSourceId",
  "confirmationSourceType",
  "confirmedEnd",
  "confirmedStart",
  "contactWxid",
  "createdAt",
  "id",
  "idempotencyKey",
  "location",
  "requestText",
  "requestedEnd",
  "requestedStart",
  "sourceTurnId",
  "status",
  "updatedAt",
  "version",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];
