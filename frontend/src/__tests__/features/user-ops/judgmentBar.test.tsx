import { describe, it, expect } from "vitest";
import { finalReviewTone } from "../../../features/user-ops/cockpit/JudgmentBar";

describe("finalReviewTone", () => {
  it("approved 系 → sent", () => {
    expect(finalReviewTone("approved")).toBe("sent");
    expect(finalReviewTone("revision_applied_approved")).toBe("sent");
  });
  it("暂缓系 → held", () => {
    expect(finalReviewTone("held_by_ai_policy")).toBe("held");
    expect(finalReviewTone("ai_waiting_for_more_context")).toBe("held");
  });
  it("拦截系 → blocked", () => {
    ["blocked_by_safety_guard", "blocked_by_required_field", "blocked_by_budget", "blocked_unverified_product_claim", "revision_failed"].forEach((s) =>
      expect(finalReviewTone(s)).toBe("blocked"));
  });
  it("legacy/未知/缺失 → other", () => {
    expect(finalReviewTone("legacy_mode_unchecked")).toBe("other");
    expect(finalReviewTone(undefined)).toBe("other");
    expect(finalReviewTone("some_future_value")).toBe("other");
  });
});
