import { api } from "./api";

export interface InboxItem {
  source: string;
  id: string;
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
}
export interface SourceError {
  source: string;
  error: string;
}
export interface InboxResponse {
  items: InboxItem[];
  errors: SourceError[];
}
export type InboxSummary = Record<string, number>;

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

export async function fetchInbox(source?: string): Promise<InboxResponse> {
  const qs = source ? `?source=${encodeURIComponent(source)}` : "";
  const raw = await api.get<Partial<InboxResponse>>(`/api/admin/ask-human/inbox${qs}`);
  return { items: raw.items ?? [], errors: raw.errors ?? [] };
}

export async function fetchSummary(): Promise<InboxSummary> {
  return api.get<InboxSummary>("/api/admin/ask-human/summary");
}
