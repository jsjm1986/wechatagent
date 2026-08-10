import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DigestCanvas, digestTargetRefLabels } from "../../../features/knowledge/today";

const realFetch = globalThis.fetch;

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

/** 两张 kind/title 完全相同、只有 targetRefs 不同的卡片 —— 线上截图里的形态。 */
const TWIN_CARDS_REPORT = {
  reportId: "fedcba987654321001234567",
  reportHash: "report-hash",
  workspaceId: "ws-a",
  accountId: "account-a",
  reportDate: "2026-08-10",
  status: "ok",
  currentGeneration: 1,
  generatedAt: "2026-08-10 9:02:11.81 +00:00:00",
  cards: [
    {
      cardId: "0123456789abcdef01234567",
      cardHash: "hash-1",
      kind: "chunk_missing_field",
      title: "切片缺失 sourceQuote 与 integrityStatus",
      summary: "AI 建议补完该切片的字段，确保负例上下文完整可信。",
      severity: "warn",
      suggestedAction: "fix_chunk",
      targetRefs: [{ kind: "chunk", id: "aaaaaaaaaaaaaaaaaa111111" }],
      metric: { name: "missing_fields", value: 2, threshold: 0 },
    },
    {
      cardId: "89abcdef0123456701234567",
      cardHash: "hash-2",
      kind: "chunk_missing_field",
      title: "切片缺失 sourceQuote 与 integrityStatus",
      summary: "AI 建议补完该切片的字段，避免负例样本在后续检索中失真。",
      severity: "warn",
      suggestedAction: "fix_chunk",
      targetRefs: [{ kind: "chunk", id: "bbbbbbbbbbbbbbbbbb222222" }],
      metric: { name: "missing_fields", value: 2, threshold: 0 },
    },
  ],
  dismissedCardIds: [],
};

describe("digestTargetRefLabels", () => {
  it("取尾 6 位并带中文类型名", () => {
    const chips = digestTargetRefLabels([{ kind: "chunk", id: "aaaaaaaaaaaaaaaaaa111111" }]);
    expect(chips).toHaveLength(1);
    expect(chips[0].kindLabel).toBe("切片");
    expect(chips[0].shortId).toBe("…111111");
    expect(chips[0].id).toBe("aaaaaaaaaaaaaaaaaa111111");
  });

  it("丢弃缺 id / 空 id 的 ref，与后端同规则", () => {
    expect(
      digestTargetRefLabels([{ kind: "chunk" }, { kind: "chunk", id: "" }, { kind: "chunk", id: "   " }]),
    ).toEqual([]);
  });

  it("同 kind+id 去重，最多保留 3 条", () => {
    const dup = digestTargetRefLabels([
      { kind: "chunk", id: "abc123456789012345678901" },
      { kind: "chunk", id: "abc123456789012345678901" },
    ]);
    expect(dup).toHaveLength(1);

    const many = digestTargetRefLabels(
      ["a", "b", "c", "d", "e"].map((c) => ({ kind: "chunk", id: c.repeat(24) })),
    );
    expect(many).toHaveLength(3);
  });

  it("kind 缺失时不留下孤零零的破折号", () => {
    const chips = digestTargetRefLabels([{ id: "abc123456789012345678901" }]);
    expect(chips[0].kindLabel).toBe("");
  });

  it("未知 kind 回落原值而不是吞掉", () => {
    const chips = digestTargetRefLabels([{ kind: "brand_new_kind", id: "abc123456789012345678901" }]);
    expect(chips[0].kindLabel).toBe("brand_new_kind");
  });

  it("空/缺省输入返回空数组", () => {
    expect(digestTargetRefLabels(undefined)).toEqual([]);
    expect(digestTargetRefLabels([])).toEqual([]);
  });
});

describe("DigestCanvas 卡片渲染", () => {
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("同标题卡片靠 targetRefs 可区分，metric 名称中文化，阈值 0 不上屏", async () => {
    globalThis.fetch = vi.fn(async () => jsonResponse(TWIN_CARDS_REPORT)) as typeof fetch;
    render(<DigestCanvas />);

    // 两张卡标题一致 —— 这正是运营无法区分的根源。
    const titles = await screen.findAllByText("切片缺失 sourceQuote 与 integrityStatus");
    expect(titles).toHaveLength(2);

    // 但目标切片必须显式上屏，且两张各不相同。
    expect(screen.getByText("…111111")).toBeInTheDocument();
    expect(screen.getByText("…222222")).toBeInTheDocument();
    expect(screen.getAllByText("切片")).toHaveLength(2);

    // metric.name 不再是原始 snake_case。
    expect(screen.getAllByText("缺失字段数")).toHaveLength(2);
    expect(screen.queryByText("missing_fields")).toBeNull();

    // threshold: 0 语义是「只要有就算问题」，没有可对比的门线，不该渲染。
    expect(screen.queryByText(/阈值/)).toBeNull();
  });

  it("非零阈值照常渲染", async () => {
    globalThis.fetch = vi.fn(async () =>
      jsonResponse({
        ...TWIN_CARDS_REPORT,
        cards: [
          {
            ...TWIN_CARDS_REPORT.cards[0],
            metric: { name: "hit_rate", value: 0.42, threshold: 0.7 },
          },
        ],
      }),
    ) as typeof fetch;
    render(<DigestCanvas />);

    expect(await screen.findByText("检索命中率")).toBeInTheDocument();
    expect(screen.getByText("阈值 0.7")).toBeInTheDocument();
  });
});
