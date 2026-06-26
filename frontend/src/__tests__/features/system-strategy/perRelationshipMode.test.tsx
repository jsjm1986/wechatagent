import { describe, it, expect } from "vitest";
import { useStrategyStore } from "../../../stores/strategyStore";
import type { DomainProfile } from "../../../types";

describe("D8 per_relationship_operation_mode", () => {
  it("editDomainProfile 回填 per_relationship_operation_mode map", () => {
    const profile = {
      profile_id: "p1", display_name: "数字分身", description: "",
      profile_dimensions: [], prompt_fragment: "", conversation_modes: [],
      business_formulas: [], commitment_markers: { product_effect: [], tone_only: [] },
      coverage_dimensions: [],
      per_relationship_operation_mode: {
        friend: {
          funnel: { enabled: false },
          silence: { enabled: true },
          commitment: { enabled: false },
          quiet_hours: {},
        },
      },
      version: 1, current_version: true, previous_version: null, is_active: false, seeded_by: null,
      id: "x", workspace_id: "w",
    } as unknown as DomainProfile;
    useStrategyStore.getState().editDomainProfile(profile);
    const d = useStrategyStore.getState().profileDraft;
    expect(d.per_relationship_operation_mode?.friend.funnel.enabled).toBe(false);
    expect(d.per_relationship_operation_mode?.friend.silence.enabled).toBe(true);
  });
});
