import { describe, it, expect } from "vitest";
import behaviorSignalFixture from "../../contracts/behavior_signal_metric.fixture.json";
import outcomeFixture from "../../contracts/outcome_metric.fixture.json";
import llmCallLogFixture from "../../contracts/llm_call_log.fixture.json";
import memoryCandidateFixture from "../../contracts/memory_candidate.fixture.json";
import operatingMemoryFixture from "../../contracts/operating_memory.fixture.json";
import agentRunFixture from "../../contracts/agent_run.fixture.json";
import decisionReviewFixture from "../../contracts/decision_review.fixture.json";
import guidePreviewFixture from "../../contracts/guide_preview.fixture.json";
import operationHealthFixture from "../../contracts/operation_health.fixture.json";
import guideApplyReceiptFixture from "../../contracts/guide_apply_receipt.fixture.json";
import { CANONICAL_KEYS as BEHAVIOR_SIGNAL_KEYS } from "../../contracts/behaviorSignalMetric.contract";
import { CANONICAL_KEYS as OUTCOME_KEYS } from "../../contracts/outcomeMetric.contract";
import { CANONICAL_KEYS as LLM_CALL_LOG_KEYS } from "../../contracts/llmCallLog.contract";
import { CANONICAL_KEYS as MEMORY_CANDIDATE_KEYS } from "../../contracts/memoryCandidate.contract";
import { CANONICAL_KEYS as OPERATING_MEMORY_KEYS } from "../../contracts/operatingMemory.contract";
import { CANONICAL_KEYS as AGENT_RUN_KEYS } from "../../contracts/agentRun.contract";
import { CANONICAL_KEYS as DECISION_REVIEW_KEYS } from "../../contracts/decisionReview.contract";
import { CANONICAL_KEYS as GUIDE_PREVIEW_KEYS } from "../../contracts/guidePreview.contract";
import { CANONICAL_KEYS as OPERATION_HEALTH_KEYS } from "../../contracts/operationHealth.contract";
import { CANONICAL_KEYS as GUIDE_APPLY_RECEIPT_KEYS } from "../../contracts/guideApplyReceipt.contract";

// 后端投影写出的 fixture（线上真相源）与前端 CANONICAL_KEYS 双向键集对账。
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

describe("契约: 运营/Agent 域投影键集对账", () => {
  it("behavior_signal_metric 投影", () =>
    assertKeysMatch("behaviorSignal", behaviorSignalFixture, BEHAVIOR_SIGNAL_KEYS));
  it("outcome_metric 投影", () =>
    assertKeysMatch("outcome", outcomeFixture, OUTCOME_KEYS));
  it("llm_call_log 投影", () =>
    assertKeysMatch("llmCallLog", llmCallLogFixture, LLM_CALL_LOG_KEYS));
  it("memory_candidate 投影", () =>
    assertKeysMatch("memoryCandidate", memoryCandidateFixture, MEMORY_CANDIDATE_KEYS));
  it("operating_memory 投影", () =>
    assertKeysMatch("operatingMemory", operatingMemoryFixture, OPERATING_MEMORY_KEYS));
  it("agent_run 投影", () =>
    assertKeysMatch("agentRun", agentRunFixture, AGENT_RUN_KEYS));
  it("decision_review 投影", () =>
    assertKeysMatch("decisionReview", decisionReviewFixture, DECISION_REVIEW_KEYS));
  it("guide_preview 投影", () =>
    assertKeysMatch("guidePreview", guidePreviewFixture, GUIDE_PREVIEW_KEYS));
  it("operation_health 聚合投影（顶层 scores+items）", () =>
    assertKeysMatch("operationHealth", operationHealthFixture, OPERATION_HEALTH_KEYS));
  it("guide_apply_receipt 投影", () =>
    assertKeysMatch("guideApplyReceipt", guideApplyReceiptFixture, GUIDE_APPLY_RECEIPT_KEYS));
});
