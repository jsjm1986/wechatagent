import { describe, it, expect } from "vitest";
import gatewayStatusValues from "../../contracts/gateway_status_values.fixture.json";
import {
  FINAL_REVIEW_STATUS_LABELS,
  HOLD_CATEGORY_LABELS,
  GATEWAY_STATUS_LABELS,
  GAP_SIGNAL_KIND_LABELS,
  ESCALATION_CATEGORY_LABELS,
  ESCALATION_VERDICT_LABELS,
  labelOf,
} from "../../lib/reviewLabels";

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

describe("reviewLabels 扩展字典", () => {
  it("gap_signal kind 覆盖 10 类且中文", () => {
    ["orphan","broken_link","missing_chunk","no_outlinks","low_confidence",
     "stale","contradiction","suggestion","dangling_anchor","recall_miss"
    ].forEach((k) => {
      expect(GAP_SIGNAL_KIND_LABELS[k]).toBeTruthy();
      expect(GAP_SIGNAL_KIND_LABELS[k]).not.toBe(k);
    });
  });

  it("escalation category 中文", () => {
    expect(ESCALATION_CATEGORY_LABELS["high_risk_gated"]).toBe("高风险待裁决");
    expect(ESCALATION_CATEGORY_LABELS["out_of_scope_decision"]).toBe("超出职权待决策");
    expect(ESCALATION_CATEGORY_LABELS["stuck_or_undelivered"]).toBe("多轮僵局待介入");
  });

  it("verdict 中文", () => {
    expect(ESCALATION_VERDICT_LABELS["approved"]).toBe("同意");
    expect(ESCALATION_VERDICT_LABELS["delegated_back"]).toBe("授权 AI 自行处理");
  });

  it("labelOf 未知值回落原值", () => {
    expect(labelOf(GAP_SIGNAL_KIND_LABELS, "brand_new")).toBe("brand_new");
  });
});

describe("GATEWAY_STATUS_LABELS 单一真相源", () => {
  it("覆盖后端 gateway 闭集 fixture 的全部值且为中文", () => {
    expect(gatewayStatusValues.length).toBeGreaterThan(0);
    for (const key of gatewayStatusValues) {
      expect(GATEWAY_STATUS_LABELS[key], `缺 ${key}`).toBeTruthy();
      expect(GATEWAY_STATUS_LABELS[key], `${key} 未翻译`).not.toBe(key);
    }
  });

  it("与 finalReviewStatus 交集键复用同一措辞(消除口径漂移)", () => {
    for (const key of Object.keys(FINAL_REVIEW_STATUS_LABELS)) {
      if (key in GATEWAY_STATUS_LABELS) {
        expect(GATEWAY_STATUS_LABELS[key], `${key} 两字典措辞不一致`).toBe(FINAL_REVIEW_STATUS_LABELS[key]);
      }
    }
  });

  it("delivery_finalizing 是发送对账内部状态，不伪装成 agent gateway 状态", () => {
    expect(gatewayStatusValues).not.toContain("delivery_finalizing");
  });
});
