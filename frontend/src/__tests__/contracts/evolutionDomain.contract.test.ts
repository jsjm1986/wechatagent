import { describe, it, expect } from "vitest";
import runtimeFlagFixture from "../../contracts/runtime_flag.fixture.json";
import thresholdOverrideFixture from "../../contracts/threshold_override.fixture.json";
import thresholdOverrideAuditFixture from "../../contracts/threshold_override_audit.fixture.json";
import experimentEnvelopeFixture from "../../contracts/experiment_envelope.fixture.json";
import proposalSummaryFixture from "../../contracts/proposal_summary.fixture.json";
import shadowReplayFixture from "../../contracts/shadow_replay.fixture.json";
import proposalDetailFixture from "../../contracts/proposal_detail.fixture.json";
import experimentSummaryFixture from "../../contracts/experiment_summary.fixture.json";
import { CANONICAL_KEYS as RUNTIME_FLAG_KEYS } from "../../contracts/runtimeFlag.contract";
import { CANONICAL_KEYS as THRESHOLD_OVERRIDE_KEYS } from "../../contracts/thresholdOverride.contract";
import { CANONICAL_KEYS as THRESHOLD_OVERRIDE_AUDIT_KEYS } from "../../contracts/thresholdOverrideAudit.contract";
import { CANONICAL_KEYS as EXPERIMENT_ENVELOPE_KEYS } from "../../contracts/experimentEnvelope.contract";
import { CANONICAL_KEYS as PROPOSAL_SUMMARY_KEYS } from "../../contracts/proposalSummary.contract";
import { CANONICAL_KEYS as SHADOW_REPLAY_KEYS } from "../../contracts/shadowReplay.contract";
import { CANONICAL_KEYS as PROPOSAL_DETAIL_KEYS } from "../../contracts/proposalDetail.contract";
import { CANONICAL_KEYS as EXPERIMENT_SUMMARY_KEYS } from "../../contracts/experimentSummary.contract";

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

describe("契约: 进化/实验域投影键集对账", () => {
  it("runtime_flag 投影", () =>
    assertKeysMatch("runtimeFlag", runtimeFlagFixture, RUNTIME_FLAG_KEYS));
  it("threshold_override 投影", () =>
    assertKeysMatch("thresholdOverride", thresholdOverrideFixture, THRESHOLD_OVERRIDE_KEYS));
  it("threshold_override_audit 投影", () =>
    assertKeysMatch("thresholdOverrideAudit", thresholdOverrideAuditFixture, THRESHOLD_OVERRIDE_AUDIT_KEYS));
  it("experiment_envelope 投影", () =>
    assertKeysMatch("experimentEnvelope", experimentEnvelopeFixture, EXPERIMENT_ENVELOPE_KEYS));
  it("proposal_summary 投影", () =>
    assertKeysMatch("proposalSummary", proposalSummaryFixture, PROPOSAL_SUMMARY_KEYS));
  it("shadow_replay 投影", () =>
    assertKeysMatch("shadowReplay", shadowReplayFixture, SHADOW_REPLAY_KEYS));
  it("proposal_detail 投影(29 键)", () =>
    assertKeysMatch("proposalDetail", proposalDetailFixture, PROPOSAL_DETAIL_KEYS));
  it("experiment_summary 聚合投影(顶层 3 键)", () =>
    assertKeysMatch("experimentSummary", experimentSummaryFixture, EXPERIMENT_SUMMARY_KEYS));
});
