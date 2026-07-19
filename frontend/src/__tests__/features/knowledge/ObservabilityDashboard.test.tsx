import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ObservabilityDashboard } from "../../../features/knowledge/steward";

const realFetch = globalThis.fetch;

function response(ok: boolean, body: unknown): Response {
  return {
    ok,
    status: ok ? 200 : 503,
    async json() {
      return body;
    },
  } as Response;
}

describe("ObservabilityDashboard catalog envelopes", () => {
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("按真实响应包络显示已持久化目录与实时文档数", async () => {
    globalThis.fetch = vi.fn(async (url: unknown) => {
      const path = String(url);
      if (path.endsWith("/catalog/persisted")) {
        return response(true, {
          documents: [
            { catalogSummaryPersisted: "已构建" },
            { catalogSummaryPersisted: null },
          ],
        });
      }
      if (path.endsWith("/operation-knowledge/catalog")) {
        return response(true, {
          item: { documents: [{ id: "d1" }, { id: "d2" }], items: [], chunks: [] },
        });
      }
      return response(false, {});
    }) as typeof fetch;

    render(<ObservabilityDashboard />);

    const card = (await screen.findByText("目录覆盖")).closest("article");
    expect(card).not.toBeNull();
    await waitFor(() => {
      const view = within(card as HTMLElement);
      expect(view.getByText("1")).toBeInTheDocument();
      expect(view.getByText("2")).toBeInTheDocument();
      expect(view.getByText("+1")).toBeInTheDocument();
    });
  });
});
