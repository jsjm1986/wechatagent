import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../../lib/inboxApi", () => ({
  fetchInbox: vi.fn(),
  fetchSummary: vi.fn(),
  sortItems: (x: unknown[]) => x, // 测里不关心排序，原样返回
}));
import { fetchInbox, fetchSummary } from "../../lib/inboxApi";
import { useInboxStore } from "../../stores/inboxStore";

const fi = fetchInbox as unknown as ReturnType<typeof vi.fn>;
const fs = fetchSummary as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  fi.mockReset();
  fs.mockReset();
  useInboxStore.setState({ items: [], errors: [], summary: null, loading: false, fatalError: null });
});

describe("inboxStore.load", () => {
  it("populates items + per-source errors on success (bad source does not clear good items)", async () => {
    fi.mockResolvedValue({
      items: [{ source: "principal_escalation", id: "a", title: "", summary: "", severity: "high", createdAt: null, ageHours: 0, actionKind: "inline" }],
      errors: [{ source: "taxonomy_candidate", error: "boom" }],
    });
    fs.mockResolvedValue({
      status: "complete",
      asOf: null,
      counts: { principalEscalation: 1 },
      errors: [],
      total: 1,
    });
    await useInboxStore.getState().load();
    const s = useInboxStore.getState();
    expect(s.items).toHaveLength(1);
    expect(s.errors).toEqual([{ source: "taxonomy_candidate", error: "boom" }]);
    expect(s.fatalError).toBeNull();
  });

  it("request-level failure KEEPS previous items and sets fatalError (never clears)", async () => {
    useInboxStore.setState({
      items: [{ source: "principal_escalation", id: "old", title: "", summary: "", severity: "high", createdAt: null, ageHours: 0, actionKind: "inline" }],
    });
    fi.mockRejectedValue(new Error("network down"));
    fs.mockRejectedValue(new Error("network down"));
    await useInboxStore.getState().load();
    const s = useInboxStore.getState();
    expect(s.items).toHaveLength(1); // 旧 items 保留，绝不清空
    expect(s.items[0].id).toBe("old");
    expect(s.fatalError).toContain("network down");
  });
});
