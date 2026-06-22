import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { EscalationInline } from "../../../../features/ask-human/inline/EscalationInline";
import { api } from "../../../../lib/api";

vi.mock("../../../../lib/api", () => ({ api: { post: vi.fn().mockResolvedValue({ ok: true }) } }));

const item = {
  source: "principal_escalation",
  id: "ESC1",
  title: "请示 #ESC1",
  summary: "客户要折扣",
  severity: "high" as const,
  createdAt: null,
  ageHours: 2,
  actionKind: "inline" as const,
};

describe("EscalationInline", () => {
  it("resolve posts verdict+substance to the short_code resolve endpoint", async () => {
    const runAction = vi.fn(async (fn: () => Promise<unknown>) => {
      await fn();
    });
    render(<EscalationInline item={item} ctx={{ busy: false, runAction }} />);
    fireEvent.change(screen.getByPlaceholderText(/裁决意见/), { target: { value: "可以给8折" } });
    fireEvent.click(screen.getByText("批准"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/principal-escalations/ESC1/resolve",
        expect.objectContaining({ verdict: "approved", substance: "可以给8折" }),
      ),
    );
  });
});
