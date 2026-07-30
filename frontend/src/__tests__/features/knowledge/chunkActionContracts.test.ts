import { describe, expect, it } from "vitest";
import {
  chunkMergeRequest,
  chunkPatchRequest,
  chunkRelateRequest,
  chunkSplitRequest,
} from "../../../features/knowledge/chunkActionContracts";

describe("Chunk action wire contracts", () => {
  it("matches the Axum camelCase DTOs exactly", () => {
    expect(chunkPatchRequest("new summary")).toEqual({
      patch: { summary: "new summary" },
    });
    expect(chunkSplitRequest(200)).toEqual({ offset: 200 });
    expect(chunkMergeRequest("target-1")).toEqual({ targetId: "target-1" });
    expect(chunkRelateRequest("target-2", "references", "source")).toEqual({
      targetId: "target-2",
      kind: "references",
      note: "source",
    });
  });
});
