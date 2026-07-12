import { describe, it, expect } from "vitest";
import documentFixture from "../../contracts/operation_knowledge_document.fixture.json";
import usageFixture from "../../contracts/knowledge_usage_log.fixture.json";
import revisionFixture from "../../contracts/revision_applied.fixture.json";
import detailFixture from "../../contracts/operation_knowledge_chunk_detail.fixture.json";
import importJobProgressFixture from "../../contracts/import_job_progress.fixture.json";
import { CANONICAL_KEYS as DOCUMENT_KEYS } from "../../contracts/operationKnowledgeDocument.contract";
import { CANONICAL_KEYS as USAGE_KEYS } from "../../contracts/knowledgeUsageLog.contract";
import { CANONICAL_KEYS as REVISION_KEYS } from "../../contracts/revisionApplied.contract";
import { CANONICAL_KEYS as DETAIL_KEYS } from "../../contracts/operationKnowledgeChunkDetail.contract";
import { CANONICAL_KEYS as IMPORT_JOB_KEYS } from "../../contracts/importJobProgress.contract";

// 后端投影写出的 fixture(线上真相源)与前端 CANONICAL_KEYS 声明双向键集对账。
// 任一侧漂移即测红:missingInFrontend=后端发了前端没声明;deadInFrontend=前端声明了后端没发。
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

describe("契约: 知识域投影键集对账", () => {
  it("operation_knowledge_document 列表投影", () =>
    assertKeysMatch("document", documentFixture, DOCUMENT_KEYS));
  it("knowledge_usage_log 投影", () =>
    assertKeysMatch("usage", usageFixture, USAGE_KEYS));
  it("revision_applied 投影", () =>
    assertKeysMatch("revision", revisionFixture, REVISION_KEYS));
  it("operation_knowledge_chunk_detail 详情裸 struct 投影(顶层 item 包裹)", () =>
    assertKeysMatch("detail", detailFixture, DETAIL_KEYS));
  it("import_job_progress 异步导入进度投影(get/list 端点)", () =>
    assertKeysMatch("importJob", importJobProgressFixture, IMPORT_JOB_KEYS));
});
