import { api } from "./api";

export interface InboxItem {
  source: string;
  id: string;
  /** Owning business account; absent means workspace-global governance work. */
  accountId?: string;
  title: string;
  summary: string;
  severity: "high" | "medium" | "low" | string;
  createdAt: string | null;
  ageHours: number;
  actionKind: "inline" | "rich";
  richComponent?: string;
  richParams?: Record<string, unknown>;
  category?: string;
  questionForPrincipal?: string;
  contactWxid?: string;
  principalWxid?: string;
  evidence?: string;
  confidence?: number;
  occurrences?: number;
  kind?: string;
  signalSeverity?: string;
  integrityStatus?: string;
}
export interface SourceError {
  source: string;
  error: string;
}
export interface InboxResponse {
  items: InboxItem[];
  errors: SourceError[];
}
export interface InboxSummary {
  status: "complete" | "partial" | "error";
  asOf: string | null;
  counts: Record<string, number | null>;
  errors: SourceError[];
  total: number | null;
}

export function severityRank(s: string): number {
  switch (s) {
    case "high":
      return 3;
    case "medium":
      return 2;
    case "low":
      return 1;
    default:
      return 0;
  }
}

/// 严重度降序，同级按 ageHours 降序（最久最严重的排最前）。返回新数组，不改入参。
export function sortItems(items: InboxItem[]): InboxItem[] {
  return [...items].sort((a, b) => {
    const bySev = severityRank(b.severity) - severityRank(a.severity);
    if (bySev !== 0) return bySev;
    return b.ageHours - a.ageHours;
  });
}

function inboxQuery(source?: string, accountId?: string): string {
  const params = new URLSearchParams();
  if (source) params.set("source", source);
  if (accountId) params.set("accountId", accountId);
  const query = params.toString();
  return query ? `?${query}` : "";
}

export async function fetchInbox(
  source?: string,
  accountId?: string,
): Promise<InboxResponse> {
  const raw = await api.get<Partial<InboxResponse>>(
    `/api/admin/ask-human/inbox${inboxQuery(source, accountId)}`,
  );
  return { items: raw.items ?? [], errors: raw.errors ?? [] };
}

export async function fetchSummary(accountId?: string): Promise<InboxSummary> {
  const raw = await api.get<Record<string, unknown>>(
    `/api/admin/ask-human/summary${inboxQuery(undefined, accountId)}`,
  );
  const nestedCounts = raw.counts;
  const counts =
    nestedCounts &&
    typeof nestedCounts === "object" &&
    !Array.isArray(nestedCounts)
      ? Object.fromEntries(
          Object.entries(nestedCounts).map(([key, value]) => [
            key,
            typeof value === "number" ? value : null,
          ]),
        )
      : Object.fromEntries(
          Object.entries(raw)
            .filter(
              ([key]) => !["status", "asOf", "errors", "total"].includes(key),
            )
            .map(([key, value]) => [
              key,
              typeof value === "number" ? value : null,
            ]),
        );
  const errors = Array.isArray(raw.errors)
    ? raw.errors.filter(
        (value): value is SourceError =>
          !!value &&
          typeof value === "object" &&
          typeof (value as SourceError).source === "string" &&
          typeof (value as SourceError).error === "string",
      )
    : [];
  const status =
    raw.status === "partial" ||
    raw.status === "error" ||
    raw.status === "complete"
      ? raw.status
      : errors.length > 0
        ? "partial"
        : "complete";
  return {
    status,
    asOf: typeof raw.asOf === "string" ? raw.asOf : null,
    counts,
    errors,
    total: typeof raw.total === "number" ? raw.total : null,
  };
}
