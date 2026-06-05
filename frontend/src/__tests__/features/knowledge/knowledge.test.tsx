import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import KnowledgeFeature from "../../../features/knowledge";

// knowledge 频道一体化迁移后的视觉/集成测试（追加，不改既有任何用例）。
// 验证：(1) 自包含 KnowledgeFeature 在新 tokens.css 视觉壳（Knowledge.module.css 全量重塑）
//           下，渲染档案馆小标题 + 工作站标题（Shell 拥有页头，这里是面板级标题）；
//       (2) 4 个 mode-bar 模式按钮（今日/探索/治理/全景）真实 DOM 仍正确；
//       (3) 默认 today 模式按钮持 active 态；点击「治理」后 active 态正确转移——
//           证明 CSS Module :global{} 壳下 className 动态切换未被破坏。

const realFetch = globalThis.fetch;

function installBenignFetch() {
  // 子视图挂载会触发取数；返回空集合即可，避免 render 期未捕获 reject。
  globalThis.fetch = vi.fn(async () => {
    const body = {
      items: [],
      chunks: [],
      signals: [],
      revisions: [],
      metrics: {},
      cards: [],
      dismissedCardIds: [],
    };
    return {
      ok: true,
      status: 200,
      async json() {
        return body;
      },
      async text() {
        return JSON.stringify(body);
      },
    } as unknown as Response;
  }) as typeof fetch;
}

describe("KnowledgeFeature — 一体化频道（全量重塑视觉壳）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installBenignFetch();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("渲染档案馆小标题与工作站标题", () => {
    render(<KnowledgeFeature />);
    expect(
      screen.getByText("Knowledge Workstation · 知识档案馆"),
    ).toBeInTheDocument();
    expect(screen.getByText("知识库工作站")).toBeInTheDocument();
  });

  it("渲染 4 个模式按钮（今日 / 探索 / 治理 / 全景）", () => {
    render(<KnowledgeFeature />);
    for (const label of ["今日", "探索", "治理", "全景"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    // caption 也应在视觉壳内真实渲染
    expect(screen.getByText("Digest 与待办")).toBeInTheDocument();
    expect(screen.getByText("Schema、指标、记忆")).toBeInTheDocument();
  });

  it("默认 today 模式按钮持 active 态，点击「治理」后 active 转移", async () => {
    const user = userEvent.setup();
    render(<KnowledgeFeature />);

    const todayBtn = screen.getByText("今日").closest("button");
    const stewardBtn = screen.getByText("治理").closest("button");
    expect(todayBtn).not.toBeNull();
    expect(stewardBtn).not.toBeNull();

    // 初始：today active、steward 非 active
    expect(todayBtn?.className).toContain("active");
    expect(stewardBtn?.className).not.toContain("active");

    await user.click(stewardBtn as HTMLButtonElement);

    // 切换后：active 态从 today 转移到 steward
    expect(stewardBtn?.className).toContain("active");
    expect(todayBtn?.className).not.toContain("active");
  });
});
