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

// 跨端键名契约（BIZ-1 回归锁）：后端 domain_signals.rs 把客户阶段写在
// domain_attributes.customer_stage（snake_case 内层键，ApiContact 的 camelCase
// rename 不递归进 Document 内层）。PlannerViewSection 必须用同名键读取，
// 否则 stageRaw 恒空、字典翻译永不触发、运营看板永远显示"未分层"。
describe("domainAttributes 客户阶段键名契约", () => {
  const readStage = (attrs: Record<string, unknown>): string => {
    const stage = attrs.customer_stage;
    return typeof stage === "string" ? stage : "";
  };
  it("从 customer_stage 键读出值并经 labelFor 翻译为中文", () => {
    const attrs = { customer_stage: "qualified", intent_level: "high" };
    const raw = readStage(attrs);
    expect(raw).toBe("qualified");
    expect(labelFor(tax, "customer_stage", raw)).toEqual({ text: "已确认意向", status: "ok" });
  });
  it("旧错误键 stage 不再被读到（后端从不写裸 stage）", () => {
    const attrs = { stage: "qualified" } as Record<string, unknown>;
    expect(readStage(attrs)).toBe("");
  });
});

