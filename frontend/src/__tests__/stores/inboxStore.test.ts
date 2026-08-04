import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../../lib/inboxApi", () => ({
  fetchInbox: vi.fn(),
  fetchSummary: vi.fn(),
  sortItems: (x: unknown[]) => x, // 测里不关心排序，原样返回
}));
import { fetchInbox, fetchSummary } from "../../lib/inboxApi";
import { principalEscalationCount, useInboxStore } from "../../stores/inboxStore";

const fi = fetchInbox as unknown as ReturnType<typeof vi.fn>;
const fs = fetchSummary as unknown as ReturnType<typeof vi.fn>;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  fi.mockReset();
  fs.mockReset();
  useInboxStore.setState({
    items: [], errors: [], summary: null, loading: false, fatalError: null,
    activeSource: null, requestGeneration: 0, summaryRequestGeneration: 0,
  });
});

describe("inboxStore.load", () => {

  it("keeps one summary truth and drops an older summary response", async () => {
    const oldRequest = deferred<any>();
    const newRequest = deferred<any>();
    fs.mockReturnValueOnce(oldRequest.promise).mockReturnValueOnce(newRequest.promise);

    const oldRefresh = useInboxStore.getState().refreshSummary();
    const newRefresh = useInboxStore.getState().refreshSummary();
    newRequest.resolve({
      status: "complete", asOf: null,
      counts: { principalEscalation: 2 }, errors: [], total: 2,
    });
    await newRefresh;
    oldRequest.resolve({
      status: "complete", asOf: null,
      counts: { principalEscalation: 9 }, errors: [], total: 9,
    });
    await oldRefresh;

    const summary = useInboxStore.getState().summary;
    expect(principalEscalationCount(summary)).toBe(2);
  });
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

  it("drops an older response that resolves after a newer source filter", async () => {
    const oldRequest = deferred<any>();
    const newRequest = deferred<any>();
    fi.mockImplementation((source?: string) =>
      source === "taxonomy_candidate" ? newRequest.promise : oldRequest.promise
    );
    fs.mockResolvedValue(null);

    const oldLoad = useInboxStore.getState().load();
    const newLoad = useInboxStore.getState().load("taxonomy_candidate");
    newRequest.resolve({ items: [{ id: "new" }], errors: [] });
    await newLoad;
    oldRequest.resolve({ items: [{ id: "old" }], errors: [] });
    await oldLoad;

    expect(useInboxStore.getState().items).toEqual([{ id: "new" }]);
    expect(useInboxStore.getState().loading).toBe(false);
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
