import { describe, it, expect } from "vitest";
import statePolicyFixture from "../../contracts/operation_state_policy.fixture.json";
import candidateFixture from "../../contracts/taxonomy_candidate.fixture.json";
import suggestionFixture from "../../contracts/relationship_suggestion.fixture.json";
import entryFixture from "../../contracts/taxonomy_entry.fixture.json";
import domainFixture from "../../contracts/operation_domain.fixture.json";
import { CANONICAL_KEYS as STATE_POLICY_KEYS } from "../../contracts/operationStatePolicy.contract";
import { CANONICAL_KEYS as CANDIDATE_KEYS } from "../../contracts/taxonomyCandidate.contract";
import { CANONICAL_KEYS as SUGGESTION_KEYS } from "../../contracts/relationshipSuggestion.contract";
import { CANONICAL_KEYS as ENTRY_KEYS } from "../../contracts/taxonomyEntry.contract";
import { CANONICAL_KEYS as DOMAIN_KEYS } from "../../contracts/operationDomain.contract";

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

describe("契约: 字典/分类域投影键集对账", () => {
  it("operation_state_policy 投影", () =>
    assertKeysMatch("statePolicy", statePolicyFixture, STATE_POLICY_KEYS));
  it("taxonomy_candidate 投影", () =>
    assertKeysMatch("taxonomyCandidate", candidateFixture, CANDIDATE_KEYS));
  it("relationship_suggestion 投影", () =>
    assertKeysMatch("relationshipSuggestion", suggestionFixture, SUGGESTION_KEYS));
  it("taxonomy_entry 投影(顶层 9 键,value 嵌套不展开)", () =>
    assertKeysMatch("taxonomyEntry", entryFixture, ENTRY_KEYS));
  it("operation_domain 投影(顶层 20 键,Document/policy 嵌套不展开)", () =>
    assertKeysMatch("operationDomain", domainFixture, DOMAIN_KEYS));
});
