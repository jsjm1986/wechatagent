import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "../../../components/ui/Toast";
import { ReviewQueue } from "../../../components/review/ReviewQueue";

function setup(fetchItems: () => Promise<{ id: string }[]>, onAction: () => Promise<void>) {
  return render(
    <ToastProvider>
      <ReviewQueue<{ id: string }>
        fetchItems={fetchItems}
        getId={(i) => i.id}
        renderItem={(item, ctx) => (
          <div key={item.id}>
            <span>row-{item.id}</span>
            <button disabled={ctx.busy} onClick={() => ctx.runAction(onAction, "ok")}>act-{item.id}</button>
          </div>
        )}
      />
    </ToastProvider>,
  );
}

describe("ReviewQueue", () => {
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
});
