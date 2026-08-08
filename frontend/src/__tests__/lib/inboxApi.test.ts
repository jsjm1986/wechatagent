import { describe, it, expect } from "vitest";
import {
  fetchInbox,
  fetchSummary,
  sortItems,
  severityRank,
  type InboxItem,
} from "../../lib/inboxApi";
import { api } from "../../lib/api";
import { vi } from "vitest";

function item(p: Partial<InboxItem>): InboxItem {
  return {
    source: "principal_escalation",
    id: "x",
    title: "t",
    summary: "s",
    severity: "low",
    createdAt: null,
    ageHours: 0,
    actionKind: "inline",
    ...p,
  };
}

describe("severityRank", () => {
  it("orders high > medium > low", () => {
    expect(severityRank("high")).toBeGreaterThan(severityRank("medium"));
    expect(severityRank("medium")).toBeGreaterThan(severityRank("low"));
  });
  it("unknown severity ranks lowest (0), never throws", () => {
    expect(severityRank("bogus")).toBe(0);
  });
});

describe("sortItems", () => {
  it("sorts by severity desc, then ageHours desc (oldest-most-severe first)", () => {
    const out = sortItems([
      item({ id: "a", severity: "low", ageHours: 1 }),
      item({ id: "b", severity: "high", ageHours: 1 }),
      item({ id: "c", severity: "high", ageHours: 99 }),
    ]);
    expect(out.map((i) => i.id)).toEqual(["c", "b", "a"]);
  });
  it("does not mutate the input array", () => {
    const input = [
      item({ id: "a", severity: "low" }),
      item({ id: "b", severity: "high" }),
    ];
    const copy = [...input];
    sortItems(input);
    expect(input).toEqual(copy);
  });
});

describe("fetchSummary", () => {
  it("preserves unavailable counts as null", async () => {
    vi.spyOn(api, "get").mockResolvedValueOnce({
      status: "partial",
      asOf: "2026-07-20T00:00:00Z",
      counts: { principalEscalation: null, knowledgeReview: 2 },
      errors: [{ source: "principal_escalation", error: "count unavailable" }],
      total: null,
    });
    const summary = await fetchSummary();
    expect(summary.counts.principalEscalation).toBeNull();
    expect(summary.counts.knowledgeReview).toBe(2);
    expect(summary.status).toBe("partial");
  });
});

describe("account-scoped requests", () => {
  it("encodes source and accountId on inbox requests", async () => {
    const get = vi
      .spyOn(api, "get")
      .mockResolvedValueOnce({ items: [], errors: [] });
    await fetchInbox("suspected_deal", "acc /一");
    expect(get).toHaveBeenCalledWith(
      "/api/admin/ask-human/inbox?source=suspected_deal&accountId=acc+%2F%E4%B8%80",
    );
  });

  it("scopes summary counts with the same accountId", async () => {
    const get = vi.spyOn(api, "get").mockResolvedValueOnce({
      status: "complete",
      counts: {},
      errors: [],
      total: 0,
    });
    await fetchSummary("acc-2");
    expect(get).toHaveBeenCalledWith(
      "/api/admin/ask-human/summary?accountId=acc-2",
    );
  });
});
