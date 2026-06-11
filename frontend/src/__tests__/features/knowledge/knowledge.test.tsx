import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import KnowledgeFeature from "../../../features/knowledge";

// knowledge 频道 IA 重组后的视觉/集成测试。
// 验证：(1) 自包含 KnowledgeFeature 渲染档案馆小标题 + 工作站标题（Shell 拥有页头）；
//       (2) 3 个 mode-bar 模式按钮（工作台/知识库/控制台）真实 DOM 正确；
//       (3) 默认 workbench 模式按钮持 active 态；点击「控制台」后 active 态正确转移。

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

  it("渲染频道小标题与工作站标题", () => {
    render(<KnowledgeFeature />);
    expect(screen.getByText("知识运营工作台")).toBeInTheDocument();
    expect(screen.getByText("知识库工作站")).toBeInTheDocument();
  });

  it("渲染 3 个模式按钮（工作台 / 知识库 / 控制台）", () => {
    render(<KnowledgeFeature />);
    for (const label of ["工作台", "知识库", "控制台"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    // caption 也应在视觉壳内真实渲染
    expect(screen.getByText("今日待办与起草")).toBeInTheDocument();
    expect(screen.getByText("录入、Schema 与系统")).toBeInTheDocument();
  });

  it("默认 workbench 模式按钮持 active 态，点击「控制台」后 active 转移", async () => {
    const user = userEvent.setup();
    render(<KnowledgeFeature />);

    const workbenchBtn = screen.getByText("工作台").closest("button");
    const consoleBtn = screen.getByText("控制台").closest("button");
    expect(workbenchBtn).not.toBeNull();
    expect(consoleBtn).not.toBeNull();

    // 初始：workbench active、console 非 active
    expect(workbenchBtn?.className).toContain("active");
    expect(consoleBtn?.className).not.toContain("active");

    await user.click(consoleBtn as HTMLButtonElement);

    // 切换后：active 态从 workbench 转移到 console
    expect(consoleBtn?.className).toContain("active");
    expect(workbenchBtn?.className).not.toContain("active");
  });
});
