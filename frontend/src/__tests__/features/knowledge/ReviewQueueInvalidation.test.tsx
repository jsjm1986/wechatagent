import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dispatchChunkEvent } from "../../../App";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { CHUNKS_INVALIDATED_EVENT, invalidateChunks } from "../../../features/knowledge/chunkInvalidation";
import { ReviewView } from "../../../features/knowledge/steward";

const realFetch = globalThis.fetch;

type Deferred<T> = { promise: Promise<T>; resolve: (value: T) => void };

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    async json() { return body; },
    async text() { return JSON.stringify(body); },
  } as Response;
}

function queueResponse(title: string) {
  return {
    items: [{
      id: `chunk-${title}`,
      title,
      body: `${title} body`,
      sourceQuote: null,
      sourceAnchors: [],
      integrityStatus: "needs_review",
      status: "draft",
      reviewCategories: ["needs_review", "source_orphan"],
    }],
    counts: {
      contested: 0,
      needs_review: 1,
      source_orphan: 1,
      pending_verification: 0,
      dependents_pending: 0,
    },
    effectiveFilter: {
      workspaceId: "ws-review",
      domain: "user_operations",
      lifecycleStatuses: ["draft", "active"],
      dimension: { key: "pricing", label: "报价", topicAliases: ["pricing", "报价", "价格"] },
    },
  };
}

function renderReviewView() {
  return render(
    <ConfirmProvider>
      <ReviewView initialDimFilter="pricing" />
    </ConfirmProvider>,
  );
}

describe("SR-132/SR-141 review projection and invalidation", () => {
  beforeEach(() => { vi.clearAllMocks(); });
  afterEach(() => { globalThis.fetch = realFetch; });

  it("dispatches lagged as a collection invalidation", () => {
    const listener = vi.fn();
    window.addEventListener(CHUNKS_INVALIDATED_EVENT, listener);
    try {
      dispatchChunkEvent({ kind: "lagged" });
      expect(listener).toHaveBeenCalledTimes(1);
      expect((listener.mock.calls[0][0] as CustomEvent).detail).toEqual({ reason: "lagged" });
    } finally {
      window.removeEventListener(CHUNKS_INVALIDATED_EVENT, listener);
    }
  });

  it("sends the dimension and consumes overlapping server facets without reclassification", async () => {
    const fetchMock = vi.fn(async () => jsonResponse(queueResponse("价格草稿")));
    globalThis.fetch = fetchMock as typeof fetch;
    renderReviewView();

    expect(await screen.findByText("价格草稿")).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledWith("/api/operation-knowledge/review-queue?dimension=pricing");
    expect(screen.getByText(/从概览「报价」维度下钻/)).toBeInTheDocument();
    expect(screen.getByText("待初审").closest("button")).toHaveTextContent("1");
    expect(screen.getByText("缺少来源").closest("button")).toHaveTextContent("1");

    fireEvent.click(screen.getByText("缺少来源"));
    expect(screen.getByText("价格草稿")).toBeInTheDocument();
  });

  it("clears stale rows immediately and serializes a burst into one trailing reload", async () => {
    const requests: Deferred<Response>[] = [];
    let inFlight = 0;
    let maxInFlight = 0;
    const fetchMock = vi.fn(() => {
      const request = deferred<Response>();
      requests.push(request);
      inFlight += 1;
      maxInFlight = Math.max(maxInFlight, inFlight);
      return request.promise.finally(() => { inFlight -= 1; });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    renderReviewView();
    await waitFor(() => expect(requests).toHaveLength(1));
    await act(async () => {
      requests[0].resolve(jsonResponse(queueResponse("旧快照")));
      await requests[0].promise;
    });
    expect(await screen.findByText("旧快照")).toBeInTheDocument();

    act(() => invalidateChunks({ reason: "lagged" }));
    await waitFor(() => expect(requests).toHaveLength(2));
    expect(screen.queryByText("旧快照")).not.toBeInTheDocument();
    expect(screen.getByText("\u5b9e\u65f6\u66f4\u65b0\u6709\u79ef\u538b\uff0c\u6b63\u5728\u91cd\u65b0\u540c\u6b65\u8bc4\u5ba1\u961f\u5217\u2026")).toBeInTheDocument();

    act(() => {
      invalidateChunks({ reason: "revised", chunkId: "chunk-old" });
      invalidateChunks({ reason: "revised", chunkId: "chunk-old" });
    });
    expect(requests).toHaveLength(2);
    expect(maxInFlight).toBe(1);

    await act(async () => {
      requests[1].resolve(jsonResponse(queueResponse("过期响应")));
      await requests[1].promise;
    });
    await waitFor(() => expect(requests).toHaveLength(3));
    expect(screen.queryByText("过期响应")).not.toBeInTheDocument();
    expect(maxInFlight).toBe(1);

    await act(async () => {
      requests[2].resolve(jsonResponse(queueResponse("最终快照")));
      await requests[2].promise;
    });
    expect(await screen.findByText("最终快照")).toBeInTheDocument();
    expect(screen.queryByText("\u5b9e\u65f6\u66f4\u65b0\u6709\u79ef\u538b\uff0c\u6b63\u5728\u91cd\u65b0\u540c\u6b65\u8bc4\u5ba1\u961f\u5217\u2026")).not.toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(maxInFlight).toBe(1);
  });
});
