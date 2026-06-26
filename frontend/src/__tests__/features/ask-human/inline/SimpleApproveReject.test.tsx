import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { SimpleApproveReject } from "../../../../features/ask-human/inline/SimpleApproveReject";

vi.mock("../../../../lib/api", () => ({ api: { post: vi.fn().mockResolvedValue({ ok: true }) } }));

const baseItem = {
  source: "relationship_suggestion",
  id: "REL1",
  title: "关系类型建议：peer",
  summary: "多次自称同行",
  severity: "low" as const,
  createdAt: null,
  ageHours: 1,
  actionKind: "inline" as const,
};

const endpoints = {
  approve: (id: string) => `/api/admin/relationship-suggestions/${id}/approve`,
  reject: (id: string) => `/api/admin/relationship-suggestions/${id}/reject`,
};

describe("SimpleApproveReject", () => {
  it("有关系建议富字段时展示判断依据/置信度/出现次数/客户标识", () => {
    const richItem = {
      ...baseItem,
      evidence: "多次自称同行",
      confidence: 80,
      occurrences: 3,
      contactWxid: "contact_e10",
    };
    const runAction = vi.fn();
    render(<SimpleApproveReject item={richItem} ctx={{ busy: false, runAction }} endpoints={endpoints} />);
    expect(screen.getByText(/判断依据/)).toBeInTheDocument();
    expect(screen.getByText(/置信度/)).toBeInTheDocument();
    expect(screen.getByText(/出现次数/)).toBeInTheDocument();
    expect(screen.getByText(/客户标识/)).toBeInTheDocument();
    expect(screen.getByText(/contact_e10/)).toBeInTheDocument();
  });

  it("置信度为 0 时仍展示富区块（confidence===0 不应被当成缺省）", () => {
    const zeroItem = { ...baseItem, confidence: 0, occurrences: 0 };
    const runAction = vi.fn();
    render(<SimpleApproveReject item={zeroItem} ctx={{ busy: false, runAction }} endpoints={endpoints} />);
    expect(screen.getByText(/置信度/)).toBeInTheDocument();
    expect(screen.getByText(/出现次数/)).toBeInTheDocument();
  });

  it("无富字段（如 knowledgeReview 来源）时不渲染富区块，只显标题/摘要", () => {
    const plainItem = {
      ...baseItem,
      source: "knowledge_review",
      title: "知识核验",
      summary: "待核验切片",
    };
    const runAction = vi.fn();
    render(<SimpleApproveReject item={plainItem} ctx={{ busy: false, runAction }} endpoints={endpoints} />);
    expect(screen.getByText("知识核验")).toBeInTheDocument();
    expect(screen.queryByText(/判断依据/)).not.toBeInTheDocument();
    expect(screen.queryByText(/置信度/)).not.toBeInTheDocument();
    expect(screen.queryByText(/出现次数/)).not.toBeInTheDocument();
  });
});
