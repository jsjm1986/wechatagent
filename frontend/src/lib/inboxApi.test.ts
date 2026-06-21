import { describe, it, expect } from "vitest";
import { sortItems, severityRank, type InboxItem } from "./inboxApi";

function item(p: Partial<InboxItem>): InboxItem {
  return {
    source: "principal_escalation", id: "x", title: "t", summary: "s",
    severity: "low", createdAt: null, ageHours: 0, actionKind: "inline",
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
    const input = [item({ id: "a", severity: "low" }), item({ id: "b", severity: "high" })];
    const copy = [...input];
    sortItems(input);
    expect(input).toEqual(copy);
  });
});
