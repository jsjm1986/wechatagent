import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import KnowledgeFeature from "../../../features/knowledge";
import { DomainSchemaTab } from "../../../features/knowledge/atlas";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";

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

  it("派工创建 POST chat/tasks 含 sessionId + plannedSteps（camelCase wire 键）", async () => {
    const user = userEvent.setup();
    // ChatWorkbench 派工要求已有 sessionId（后端 sessionId 非空校验）；预置进 localStorage。
    window.localStorage.setItem("knowledgeChat.sessionId", "S1");
    const fetchMock = vi.fn(async (url: unknown, init?: { method?: string; body?: string }) => {
      const u = String(url);
      if (u.includes("/knowledge/chat/tasks")) {
        const body =
          init?.method === "POST"
            ? { taskId: "T1", sessionId: "S1", status: "pending", totalSteps: 1 }
            : {
                taskId: "T1",
                sessionId: "S1",
                status: "pending",
                totalSteps: 1,
                completedSteps: [],
                cards: [],
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
      }
      const benign = {
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
          return benign;
        },
        async text() {
          return JSON.stringify(benign);
        },
      } as unknown as Response;
    });
    globalThis.fetch = fetchMock as typeof fetch;

    try {
      render(<KnowledgeFeature />);
      // 切到「AI 协作」pane 暴露 ChatWorkbench。
      await user.click(screen.getByText("AI 协作").closest("button") as HTMLButtonElement);
      const stepsInput = await screen.findByPlaceholderText(/每行一个步骤/);
      await user.type(stepsInput, "分析最近 24h 拦截日志");
      await user.click(screen.getByText("派工").closest("button") as HTMLButtonElement);

      const call = fetchMock.mock.calls.find(
        (c) => String(c[0]).includes("/knowledge/chat/tasks") && (c[1] as { method?: string })?.method === "POST"
      );
      expect(call).toBeTruthy();
      const body = JSON.parse((call as unknown as [string, { body: string }])[1].body);
      expect(body).toHaveProperty("sessionId");
      expect(body).toHaveProperty("plannedSteps");
      expect(Array.isArray(body.plannedSteps)).toBe(true);
      // 每个 step 必须带 action（后端 ALLOWED_TASK_ACTIONS 闭集校验，缺则 400）。
      expect(body.plannedSteps[0]).toHaveProperty("action");
    } finally {
      window.localStorage.removeItem("knowledgeChat.sessionId");
    }
  });
});

describe("DomainSchemaTab — D9 字段表写表单（create/edit/delete 对接 CRUD）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  function renderTab() {
    return render(
      <ToastProvider>
        <ConfirmProvider>
          <DomainSchemaTab />
        </ConfirmProvider>
      </ToastProvider>,
    );
  }

  it("D9: 新建字段表 POST 到 /api/admin/domain-schemas，body 含 camelCase schemaId", async () => {
    const fetchMock = vi.fn(async (url: unknown, init?: { method?: string; body?: string }) => {
      const u = String(url);
      // create POST 返回 ok；list GET 返回空。
      const body =
        u.includes("/api/admin/domain-schemas") && init?.method === "POST"
          ? { ok: true }
          : { items: [] };
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
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const user = userEvent.setup();
    renderTab();

    // 空态出现后点「+ 新建字段表」。
    await screen.findByText("还没有配置字段表");
    await user.click(screen.getByText("+ 新建字段表"));

    // 填 schemaId + name → 保存。
    await user.type(screen.getByPlaceholderText(/schemaId/i), "real_estate");
    await user.type(screen.getByPlaceholderText(/字段表名称/), "房产销售");
    await user.click(screen.getByText("保存"));

    const call = fetchMock.mock.calls.find(
      (c) => String(c[0]) === "/api/admin/domain-schemas" && (c[1] as { method?: string })?.method === "POST",
    );
    expect(call).toBeTruthy();
    const sent = JSON.parse((call as unknown as [string, { body: string }])[1].body);
    expect(sent).toHaveProperty("schemaId", "real_estate");
    expect(sent).toHaveProperty("name", "房产销售");
    // wire 键 camelCase（非 snake_case）。
    expect(sent).toHaveProperty("aliasDict");
    expect(sent).not.toHaveProperty("alias_dict");
  });

  it("D9: 文案不再承诺只读（去掉「不能直接改内容」，出现「创建、编辑、删除」）", async () => {
    globalThis.fetch = vi.fn(async () => {
      const body = { items: [] };
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

    renderTab();
    await screen.findByText("还没有配置字段表");
    expect(screen.queryByText(/不能直接改内容/)).toBeNull();
    expect(screen.getByText(/创建、编辑、删除字段表/)).toBeInTheDocument();
  });
});
