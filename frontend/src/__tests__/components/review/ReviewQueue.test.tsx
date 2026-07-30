import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor, fireEvent } from "@testing-library/react";
import { useState } from "react";
import { ToastProvider } from "../../../components/ui/Toast";
import { ReviewQueue } from "../../../components/review/ReviewQueue";
import { TaxonomyCandidateReviewCard, type TaxonomyCandidate } from "../../../components/review/TaxonomyCandidateReviewCard";
import { api } from "../../../lib/api";

vi.mock("../../../components/review/TaxonomyCandidateReviewCard.module.css", () => ({
  default: new Proxy({}, { get: (_target, key) => String(key) }),
}));

vi.mock("../../../lib/api", () => ({
  api: {
    postRaw: vi.fn().mockResolvedValue({ ok: true, status: 200, data: {} }),
    post: vi.fn().mockResolvedValue({}),
  },
}));

function setup(fetchItems: () => Promise<{ id: string }[]>, onAction: () => Promise<void>) {
  return render(
    <ToastProvider>
      <ReviewQueue<{ id: string }>
        fetchItems={fetchItems}
        getId={(i) => i.id}
        renderItem={(item, ctx) => (
          <div>
            <span>row-{item.id}</span>
            <button disabled={ctx.busy} onClick={() => ctx.runAction(onAction, "ok")}>act-{item.id}</button>
          </div>
        )}
      />
    </ToastProvider>,
  );
}

describe("ReviewQueue", () => {
  beforeEach(() => vi.clearAllMocks());

  it("fetches and renders items on mount", async () => {
    setup(() => Promise.resolve([{ id: "1" }, { id: "2" }]), () => Promise.resolve());
    expect(await screen.findByText("row-1")).toBeTruthy();
    expect(screen.getByText("row-2")).toBeTruthy();
  });

  it("runAction refetches after a successful action", async () => {
    const fetchItems = vi.fn().mockResolvedValue([{ id: "1" }]);
    const onAction = vi.fn().mockResolvedValue(undefined);
    setup(fetchItems, onAction);
    await screen.findByText("row-1");
    expect(fetchItems).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByText("act-1"));
    await waitFor(() => expect(onAction).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(fetchItems).toHaveBeenCalledTimes(2)); // refetch
  });

  it("binds local review drafts to object ids when [A,B] becomes [B]", async () => {
    const candidateA: TaxonomyCandidate = {
      id: "candidate-a",
      scope: "global",
      kind: "customer_stage",
      rawValue: "raw-a",
      suggestedDisplayName: "label-a",
    };
    const candidateB: TaxonomyCandidate = {
      id: "candidate-b",
      scope: "global",
      kind: "customer_stage",
      rawValue: "raw-b",
      suggestedDisplayName: "label-b",
    };
    const fetchItems = vi
      .fn<() => Promise<TaxonomyCandidate[]>>()
      .mockResolvedValueOnce([candidateA, candidateB])
      .mockResolvedValue([candidateB]);
    const postRaw = vi.mocked(api.postRaw);

    function Harness() {
      const [refreshToken, setRefreshToken] = useState(0);
      return (
        <ToastProvider>
          <button type="button" onClick={() => setRefreshToken((value) => value + 1)}>refresh</button>
          <ReviewQueue<TaxonomyCandidate>
            fetchItems={fetchItems}
            getId={(item) => item.id}
            refreshToken={refreshToken}
            renderItem={(item) => (
              <TaxonomyCandidateReviewCard candidate={item} onDone={() => undefined} />
            )}
          />
        </ToastProvider>
      );
    }

    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    render(<Harness />);
    const labels = await screen.findAllByLabelText(/显示名/);
    fireEvent.change(labels[0], { target: { value: "edited-a" } });

    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await waitFor(() => expect(screen.queryByDisplayValue("edited-a")).toBeNull());
    expect(screen.getByDisplayValue("label-b")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "采纳" }));
    await waitFor(() => expect(postRaw).toHaveBeenCalledTimes(1));
    const [url, body] = postRaw.mock.calls[0] as [
      string,
      { canonicalValue: { id: string; label: string } },
    ];
    expect(url).toContain("/candidate-b/approve");
    expect(body.canonicalValue).toMatchObject({ id: "raw-b", label: "label-b" });
    expect(
      consoleError.mock.calls.some((call) => call.some((value) => String(value).includes("unique key"))),
    ).toBe(false);
    consoleError.mockRestore();
  });

  it("rejects an action closure captured from an older accepted generation", async () => {
    const fetchItems = vi
      .fn<() => Promise<{ id: string }[]>>()
      .mockResolvedValueOnce([{ id: "a" }])
      .mockResolvedValue([{ id: "b" }]);
    const staleEffect = vi.fn().mockResolvedValue(undefined);
    let staleAction: (() => Promise<void>) | undefined;

    function Harness() {
      const [refreshToken, setRefreshToken] = useState(0);
      return (
        <ToastProvider>
          <button type="button" onClick={() => setRefreshToken((value) => value + 1)}>refresh</button>
          <ReviewQueue<{ id: string }>
            fetchItems={fetchItems}
            getId={(item) => item.id}
            refreshToken={refreshToken}
            renderItem={(item, ctx) => {
              if (item.id === "a") staleAction = () => ctx.runAction(staleEffect);
              return <div>row-{item.id}</div>;
            }}
          />
        </ToastProvider>
      );
    }

    render(<Harness />);
    await screen.findByText("row-a");
    fireEvent.click(screen.getByRole("button", { name: "refresh" }));
    await screen.findByText("row-b");

    await act(async () => {
      await staleAction?.();
    });
    expect(staleEffect).not.toHaveBeenCalled();
    expect(screen.getByText(/列表已刷新/)).toBeTruthy();
  });
});
