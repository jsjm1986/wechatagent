import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DigestCanvas } from "../../../features/knowledge/today";

const realFetch = globalThis.fetch;

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

describe("DigestCanvas dismiss scope", () => {
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("includes the report accountId in the dismiss request", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === "/api/knowledge/digest/today") {
        return jsonResponse({
          reportId: "report-a",
          workspaceId: "ws-a",
          accountId: "account/a",
          reportDate: "2026-05-24",
          status: "ok",
          cards: [
            {
              cardId: "0123456789abcdef01234567",
              kind: "chunk_missing_field",
              title: "Scoped card",
              summary: "Needs a source quote",
              severity: "warn",
              suggestedAction: "fix_chunk",
            },
          ],
          dismissedCardIds: [],
        });
      }
      return jsonResponse({ ok: true });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const user = userEvent.setup();
    render(<DigestCanvas />);
    await user.click(await screen.findByRole("button", { name: /忽略/ }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/knowledge/digest/cards/0123456789abcdef01234567/dismiss?accountId=account%2Fa",
        { method: "POST" },
      );
    });
  });

  it("keeps the last successful cards visible and reports a failed regeneration", async () => {
    globalThis.fetch = vi.fn(async () =>
      jsonResponse({
        reportId: "report-a",
        workspaceId: "ws-a",
        accountId: "account-a",
        reportDate: "2026-07-26",
        status: "ok",
        attemptGeneration: 2,
        currentGeneration: 1,
        latestAttemptStatus: "failed",
        latestAttemptErrorKind: "upstream_timeout",
        cards: [
          {
            cardId: "0123456789abcdef01234567",
            kind: "chunk_missing_field",
            title: "Last successful card",
            summary: "Preserved after a failed regeneration",
            severity: "warn",
            suggestedAction: "fix_chunk",
          },
        ],
        dismissedCardIds: [],
      }),
    ) as typeof fetch;

    render(<DigestCanvas />);

    expect(await screen.findByText("Last successful card")).toBeInTheDocument();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "最近重算未成功（upstream_timeout）；当前继续展示上次成功结果。",
    );
  });

  it("SR-125: batch dispatch sends report/card hashes without client-authored steps", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/api/knowledge/digest/today")) {
        return jsonResponse({
          reportId: "fedcba987654321001234567",
          reportHash: "report-hash",
          workspaceId: "ws-a",
          accountId: "account-a",
          reportDate: "2026-07-27",
          status: "ok",
          currentGeneration: 7,
          cards: [{
            cardId: "0123456789abcdef01234567",
            cardHash: "card-hash",
            kind: "chunk_missing_field",
            title: "Bound card",
            summary: "Authoritative summary",
            severity: "warn",
            suggestedAction: "fix_chunk",
          }],
          dismissedCardIds: [],
        });
      }
      if (url === "/api/knowledge/chat/tasks" && init?.method === "POST") {
        return jsonResponse({ taskId: "task-a", status: "pending" });
      }
      return jsonResponse({ items: [] });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const user = userEvent.setup();
    render(<DigestCanvas />);
    await user.click(await screen.findByRole("checkbox", { name: "选择卡片 Bound card" }));
    await user.click(screen.getByRole("button", { name: "批量派工（1）" }));

    await waitFor(() => {
      const call = fetchMock.mock.calls.find(
        ([url, init]) => String(url) === "/api/knowledge/chat/tasks" && init?.method === "POST",
      );
      expect(call).toBeTruthy();
      const body = JSON.parse(String(call?.[1]?.body));
      expect(body.digestSelection).toEqual({
        accountId: "account-a",
        reportId: "fedcba987654321001234567",
        reportDate: "2026-07-27",
        reportGeneration: 7,
        reportHash: "report-hash",
        selectedCards: [{
          cardId: "0123456789abcdef01234567",
          cardHash: "card-hash",
        }],
      });
      expect(body).not.toHaveProperty("plannedSteps");
      expect(body).not.toHaveProperty("cardIds");
      expect(body).not.toHaveProperty("candidateHash");
    });
  });
});
