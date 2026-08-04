import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChunkReviewCard } from "../../../components/review/ChunkReviewCard";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn(),
    post: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

beforeEach(() => {
  vi.mocked(api.post).mockClear();
});

// verify-gate 红线真值表（ChunkReviewCard.tsx:96-100）：
//   canVerify = hasQuote && hasAnchor
//   hasQuote  = (source_quote ?? sourceQuote) 去空白后非空
//   hasAnchor = (source_anchors ?? sourceAnchors) 长度 > 0
// 「AI 永不自动 verify」红线由这个纯布尔逻辑承载——4 个分支若被重构悄悄放宽
// （改 || / 删条件 / 漏 trim）必须有回归告警。这是同目录唯一承载红线却原本零测试的卡片。
//
// 用 prefetched chunk prop 渲染：该模式挂载不发 GET（ChunkReviewCard.tsx:71-75），
// 无需 mock api，直接喂字段、读 verify 按钮的 disabled 真值。

function verifyButton() {
  return screen.getByRole("button", { name: "核验通过" }) as HTMLButtonElement;
}

describe("ChunkReviewCard verify-gate 真值表（红线：hasQuote && hasAnchor）", () => {
  it("quote 有 + anchor 有 → verify 可点（唯一放行分支）", () => {
    render(
      <ChunkReviewCard
        chunkId="c1"
        chunk={{ title: "t", source_quote: "依据原文", source_anchors: [{ start: 0 }] }}
      />,
    );
    expect(verifyButton().disabled).toBe(false);
  });

  it("quote 有 + anchor 无 → verify 禁用", () => {
    render(
      <ChunkReviewCard
        chunkId="c2"
        chunk={{ title: "t", source_quote: "依据原文", source_anchors: [] }}
      />,
    );
    expect(verifyButton().disabled).toBe(true);
  });

  it("quote 无 + anchor 有 → verify 禁用", () => {
    render(
      <ChunkReviewCard
        chunkId="c3"
        chunk={{ title: "t", source_quote: null, source_anchors: [{ start: 0 }] }}
      />,
    );
    expect(verifyButton().disabled).toBe(true);
  });

  it("quote 无 + anchor 无 → verify 禁用", () => {
    render(<ChunkReviewCard chunkId="c4" chunk={{ title: "t" }} />);
    expect(verifyButton().disabled).toBe(true);
  });

  it("quote 仅空白 → 不算有 quote → verify 禁用（trim 不可漏）", () => {
    render(
      <ChunkReviewCard
        chunkId="c5"
        chunk={{ title: "t", source_quote: "   ", source_anchors: [{ start: 0 }] }}
      />,
    );
    expect(verifyButton().disabled).toBe(true);
  });

  it("camelCase 拼写（steward 列表整形）同样被识别：sourceQuote+sourceAnchors 齐 → 放行", () => {
    render(
      <ChunkReviewCard
        chunkId="c6"
        chunk={{ title: "t", sourceQuote: "依据原文", sourceAnchors: [{ start: 0 }] }}
      />,
    );
    expect(verifyButton().disabled).toBe(false);
  });


  it("verify 提交管理员所见版本令牌", async () => {
    render(
      <ChunkReviewCard
        chunkId="c8"
        chunk={{
          title: "t",
          sourceQuote: "依据原文",
          sourceAnchors: [{ start: 0 }],
          updatedAt: "2026-08-05T01:02:03Z",
        }}
      />,
    );
    fireEvent.click(verifyButton());
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/operation-knowledge/chunks/c8/verify",
        { expectedUpdatedAt: "2026-08-05T01:02:03Z" },
      ),
    );
  });

  it("reject 永远可点（不受 verify-gate 约束）", () => {
    render(<ChunkReviewCard chunkId="c7" chunk={{ title: "t" }} />);
    const reject = screen.getByRole("button", { name: "驳回" }) as HTMLButtonElement;
    expect(reject.disabled).toBe(false);
  });
});
