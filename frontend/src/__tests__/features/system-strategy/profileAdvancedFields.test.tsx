import { describe, it, expect } from "vitest";
import { useStrategyStore } from "../../../stores/strategyStore";
import type { DomainProfile } from "../../../types";

describe("D7 profile 高级字段", () => {
  it("editDomainProfile 回填 transaction_facts_enabled / reviewer_orientation 等 5 字段", () => {
    const profile = {
      profile_id: "p1", display_name: "测试", description: "",
      profile_dimensions: [], prompt_fragment: "", conversation_modes: [],
      business_formulas: [], commitment_markers: { product_effect: [], tone_only: [] },
      coverage_dimensions: [],
      transaction_facts_enabled: true,
      reviewer_orientation: { review_focus: "情感共鸣", balance_principle: "陪伴优先" },
      mode_gate_policy_override: "自定模式说明",
      trajectory_dimensions: [{ kind: "emotion", display_name: "情绪轨迹" }],
      debounce_window_ms_override: 8000,
      version: 1, current_version: true, previous_version: null, is_active: false, seeded_by: null,
      id: "x", workspace_id: "w",
    } as unknown as DomainProfile;
    useStrategyStore.getState().editDomainProfile(profile);
    const d = useStrategyStore.getState().profileDraft;
    expect(d.transaction_facts_enabled).toBe(true);
    expect(d.reviewer_orientation?.review_focus).toBe("情感共鸣");
    expect(d.mode_gate_policy_override).toBe("自定模式说明");
    expect(d.trajectory_dimensions?.[0].kind).toBe("emotion");
    expect(d.debounce_window_ms_override).toBe(8000);
  });
});
