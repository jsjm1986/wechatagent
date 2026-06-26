import { describe, it, expect } from "vitest";
import { FINAL_REVIEW_STATUS_LABELS, HOLD_CATEGORY_LABELS, labelOf } from "../../lib/reviewLabels";

describe("reviewLabels 闭集标签(C7/C8 共用)", () => {
  it("FINAL_REVIEW_STATUS_LABELS 覆盖 10 项闭集", () => {
    const keys = [
      "approved", "revision_applied_approved", "revision_failed",
      "held_by_ai_policy", "blocked_by_safety_guard", "ai_waiting_for_more_context",
      "blocked_by_required_field", "blocked_by_budget", "blocked_unverified_product_claim",
      "legacy_mode_unchecked",
    ];
    for (const k of keys) {
      expect(FINAL_REVIEW_STATUS_LABELS[k], `缺 ${k}`).toBeTruthy();
    }
  });
  it("HOLD_CATEGORY_LABELS 覆盖 3 项闭集", () => {
    for (const k of ["held_by_ai_policy", "blocked_by_safety_guard", "ai_waiting_for_more_context"]) {
      expect(HOLD_CATEGORY_LABELS[k], `缺 ${k}`).toBeTruthy();
    }
  });
  it("labelOf 已知值翻译、未知值回落原始、空值回落 —", () => {
    expect(labelOf(FINAL_REVIEW_STATUS_LABELS, "approved")).toBe("已通过");
    expect(labelOf(FINAL_REVIEW_STATUS_LABELS, "weird_value")).toBe("weird_value");
    expect(labelOf(FINAL_REVIEW_STATUS_LABELS, null)).toBe("—");
  });
});
