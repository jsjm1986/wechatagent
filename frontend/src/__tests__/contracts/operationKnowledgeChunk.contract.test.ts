import { describe, it, expect } from "vitest";
import fixture from "../../contracts/operation_knowledge_chunk.fixture.json";
import { CANONICAL_KEYS } from "../../contracts/operationKnowledgeChunk.contract";

// 契约对账:后端 operation_knowledge_chunk_json 写出的 fixture(线上真相源)
// 与前端 CANONICAL_KEYS 声明双向比对。任何一侧漂移都在此测红。
describe("契约: operation_knowledge_chunk 列表投影", () => {
  const actualKeys = Object.keys(fixture).sort();
  const declaredKeys = [...CANONICAL_KEYS].sort();

  it("后端下发的键集 == 前端声明的 CANONICAL_KEYS(无缺、无多)", () => {
    const missingInFrontend = actualKeys.filter((k) => !declaredKeys.includes(k));
    const deadInFrontend = declaredKeys.filter((k) => !actualKeys.includes(k));
    expect(
      { missingInFrontend, deadInFrontend },
      "后端新增字段→前端须在 CANONICAL_KEYS 登记并处理;后端删字段→前端须清理死键",
    ).toEqual({ missingInFrontend: [], deadInFrontend: [] });
  });

  it("fixture 是非空对象(防 bless 写空)", () => {
    expect(actualKeys.length).toBeGreaterThan(0);
  });
});
