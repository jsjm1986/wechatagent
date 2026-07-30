import { describe, it, expect } from "vitest";
import playbookFixture from "../../contracts/playbook.fixture.json";
import promptTemplateFixture from "../../contracts/prompt_template.fixture.json";
import evaluationScenarioFixture from "../../contracts/evaluation_scenario.fixture.json";
import suspectedDealFixture from "../../contracts/suspected_deal.fixture.json";
import outboxEntryFixture from "../../contracts/outbox_entry.fixture.json";
import outboxPayloadFixture from "../../contracts/outbox_payload.fixture.json";
import toolCallFixture from "../../contracts/tool_call.fixture.json";
import { CANONICAL_KEYS as PLAYBOOK_KEYS } from "../../contracts/playbook.contract";
import { CANONICAL_KEYS as PROMPT_TEMPLATE_KEYS } from "../../contracts/promptTemplate.contract";
import { CANONICAL_KEYS as EVALUATION_SCENARIO_KEYS } from "../../contracts/evaluationScenario.contract";
import { CANONICAL_KEYS as SUSPECTED_DEAL_KEYS } from "../../contracts/suspectedDeal.contract";
import { CANONICAL_KEYS as OUTBOX_ENTRY_KEYS } from "../../contracts/outboxEntry.contract";
import { CANONICAL_KEYS as OUTBOX_PAYLOAD_KEYS } from "../../contracts/outboxPayload.contract";
import { CANONICAL_KEYS as TOOL_CALL_KEYS } from "../../contracts/toolCall.contract";

// 后端投影写出的 fixture(线上真相源)与前端 CANONICAL_KEYS 双向键集对账。
// missingInFrontend=后端发了前端没声明;deadInFrontend=前端声明了后端没发。
function assertKeysMatch(
  label: string,
  fixture: Record<string, unknown>,
  declared: readonly string[],
) {
  const actual = Object.keys(fixture).sort();
  const decl = [...declared].sort();
  const missingInFrontend = actual.filter((k) => !decl.includes(k));
  const deadInFrontend = decl.filter((k) => !actual.includes(k));
  expect(
    { missingInFrontend, deadInFrontend },
    `${label}: 后端新增字段→前端须在 CANONICAL_KEYS 登记;后端删字段→前端须清理死键`,
  ).toEqual({ missingInFrontend: [], deadInFrontend: [] });
  expect(actual.length, `${label}: fixture 非空`).toBeGreaterThan(0);
}

describe("契约: 配置/playbook 域投影键集对账", () => {
  it("playbook 投影", () =>
    assertKeysMatch("playbook", playbookFixture, PLAYBOOK_KEYS));
  it("prompt_template 投影", () =>
    assertKeysMatch("promptTemplate", promptTemplateFixture, PROMPT_TEMPLATE_KEYS));
  it("evaluation_scenario 投影(contactSeed/groundTruth 顶层各算一键)", () =>
    assertKeysMatch("evaluationScenario", evaluationScenarioFixture, EVALUATION_SCENARIO_KEYS));
  it("suspected_deal 投影", () =>
    assertKeysMatch("suspectedDeal", suspectedDealFixture, SUSPECTED_DEAL_KEYS));
  it("outbox_entry 投影保留 typed payload 业务身份", () => {
    assertKeysMatch("outboxEntry", outboxEntryFixture, OUTBOX_ENTRY_KEYS);
    expect(outboxEntryFixture.payload).toEqual(
      expect.objectContaining({ kind: "media", assetId: "asset-1" }),
    );
  });
  it("outbox_payload media 投影", () => {
    assertKeysMatch("outboxPayload", outboxPayloadFixture, OUTBOX_PAYLOAD_KEYS);
    expect(outboxPayloadFixture).toEqual(
      expect.objectContaining({ kind: "media", assetId: "asset-1" }),
    );
  });
  it("tool_call 投影", () =>
    assertKeysMatch("toolCall", toolCallFixture, TOOL_CALL_KEYS));
});
