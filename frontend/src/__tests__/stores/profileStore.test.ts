import { describe, it, expect } from "vitest";
import { labelFor, type TaxonomyMap } from "../../stores/profileStore";

const tax: TaxonomyMap = {
  customer_stage: [
    { id: "first_contact", label: "初次接触" },
    { id: "qualified", label: "已确认意向" },
  ],
};

describe("labelFor 三情形分流", () => {
  it("命中字典 → display_name, status ok", () => {
    expect(labelFor(tax, "customer_stage", "first_contact")).toEqual({ text: "初次接触", status: "ok" });
  });
  it("有字典但值不在内 → 原值, status unknown_value", () => {
    expect(labelFor(tax, "customer_stage", "weird_value")).toEqual({ text: "weird_value", status: "unknown_value" });
  });
  it("维度无字典 → 原值, status no_dict", () => {
    expect(labelFor(tax, "emotion_state", "anxious")).toEqual({ text: "anxious", status: "no_dict" });
  });
});
