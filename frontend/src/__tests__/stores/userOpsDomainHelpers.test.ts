import { describe, expect, it } from "vitest";
import { USER_RUNTIME_PARAMETER_FIELDS } from "../../stores/userOpsDomainHelpers";

describe("user operation runtime defaults", () => {
  it("keeps the complete harness budget tuple in the shared editor schema", () => {
    const defaults = Object.fromEntries(
      USER_RUNTIME_PARAMETER_FIELDS.map((field) => [field.key, field.defaultValue])
    );

    expect(defaults).toMatchObject({
      runTokenBudget: 300000,
      runTokenBudgetEscalated: 600000,
      runMaxLlmCalls: 10,
      simulationTokenBudget: 300000
    });
  });
});
