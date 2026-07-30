import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ImportWizard } from "../../../features/knowledge/steward";

const realFetch = globalThis.fetch;

function response(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    async json() {
      return body;
    },
    async text() {
      return JSON.stringify(body);
    },
  } as unknown as Response;
}

describe("ImportWizard — sealed preview apply contract", () => {
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("submits only preview identity and selected candidate patches", async () => {
    const fetchMock = vi.fn(async (url: unknown, init?: RequestInit) => {
      const path = String(url);
      if (path.endsWith("/import-preview-jobs?status=running")) {
        return response({ jobs: [] });
      }
      if (path.endsWith("/import-preview") && init?.method === "POST") {
        return response({
          previewId: "64a1f2c3e4b5a6978899b002",
          previewHash: "sealed-preview-hash",
          document: { title: "sealed document", summary: "server-owned" },
          items: [{ title: "legacy item must not be replayed" }],
          chunks: [
            {
              candidateId: "candidate-0001",
              title: "first candidate",
              body: "first body",
              wikiType: "finding",
              productTags: [],
              businessTopics: [],
            },
            {
              candidateId: "candidate-0002",
              title: "second candidate",
              body: "second body",
              wikiType: "finding",
              productTags: [],
              businessTopics: [],
            },
          ],
          importReport: { totalSegments: 1, succeeded: 1, failed: 0 },
        });
      }
      if (path.endsWith("/import-apply") && init?.method === "POST") {
        return response({ chunkIds: ["64a1f2c3e4b5a6978899b101"] });
      }
      return response({});
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const user = userEvent.setup();
    render(
      <ConfirmProvider>
        <ImportWizard />
      </ConfirmProvider>,
    );

    await user.type(screen.getByPlaceholderText(/粘贴 markdown/), "source text");
    await user.click(screen.getByRole("button", { name: /下一步：预览/ }));

    const titleInputs = await screen.findAllByRole("textbox");
    const firstCandidateTitle = titleInputs.find(
      (input) => (input as HTMLInputElement).value === "first candidate",
    );
    expect(firstCandidateTitle).toBeTruthy();
    await user.clear(firstCandidateTitle!);
    await user.type(firstCandidateTitle!, "operator edited title");

    const candidateCheckboxes = screen.getAllByRole("checkbox");
    expect(candidateCheckboxes).toHaveLength(2);
    await user.click(candidateCheckboxes[1]);
    await user.click(screen.getByRole("button", { name: /应用 1 条/ }));

    await waitFor(() => {
      expect(screen.getByText(/已存入 1 条草稿/)).toBeInTheDocument();
    });
    const applyCall = fetchMock.mock.calls.find(
      ([url, init]) =>
        String(url).endsWith("/import-apply") && (init as RequestInit | undefined)?.method === "POST",
    );
    expect(applyCall).toBeTruthy();
    const sent = JSON.parse(String((applyCall![1] as RequestInit).body));
    expect(sent).toEqual({
      previewId: "64a1f2c3e4b5a6978899b002",
      previewHash: "sealed-preview-hash",
      chunks: [
        {
          candidateId: "candidate-0001",
          patch: { title: "operator edited title" },
        },
      ],
    });
    expect(sent).not.toHaveProperty("document");
    expect(sent).not.toHaveProperty("items");
    expect(sent).not.toHaveProperty("accountId");
    expect(sent).not.toHaveProperty("sourceName");
  });
});
