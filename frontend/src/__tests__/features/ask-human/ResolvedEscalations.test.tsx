import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";

// 只读历史视图自取数：直接 mock api.get（与 pending 收件箱的 inboxStore 链路正交，互不影响）。
vi.mock("../../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../../lib/api")>(
    "../../../lib/api",
  );
  return { ...actual, api: { ...actual.api, get: vi.fn() } };
});

import { api } from "../../../lib/api";
import { ResolvedEscalations, formatExpiry } from "../../../features/ask-human/ResolvedEscalations";
import AskHumanFeature from "../../../features/ask-human/index";
import { useInboxStore } from "../../../stores/inboxStore";

const get = api.get as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  get.mockReset();
});

describe("ResolvedEscalations 已裁决历史", () => {
  it("fetches resolved escalations and renders verdict / 授权到期 / 裁决渠道", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "E1",
          contactWxid: "wxid_a",
          category: "pricing",
          decision: { verdict: "approved", substance: "同意 8 折", constraints: ["本周内付款"] },
          authorizationExpiresAt: "2026-07-01T00:00:00Z",
          resolvedVia: "principal_chat",
        },
      ],
    });

    render(<ResolvedEscalations />);

    // 调用了 resolved 端点。
    await waitFor(() =>
      expect(get).toHaveBeenCalledWith("/api/admin/principal-escalations?status=resolved"),
    );
    // 短码、裁决结果（verdict 标签用共享字典 approved→"同意" + substance）、授权到期、裁决渠道均渲染。
    await screen.findByText("E1");
    expect(screen.getByText("同意")).toBeTruthy();
    expect(screen.getByText(/同意 8 折/)).toBeTruthy();
    expect(screen.getByText(/2026/)).toBeTruthy();
    expect(screen.getByText(/裁决渠道/)).toBeTruthy();
  });

  it("authorizationExpiresAt 为 null 时显示本次转述不设期限", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "E2",
          decision: { verdict: "rejected", substance: "暂不考虑" },
          authorizationExpiresAt: null,
          resolvedVia: "admin",
        },
      ],
    });

    render(<ResolvedEscalations />);
    await screen.findByText("E2");
    expect(screen.getByText(/本次转述不设期限/)).toBeTruthy();
  });

  it("空列表显示占位文案", async () => {
    get.mockResolvedValue({ items: [] });
    render(<ResolvedEscalations />);
    await screen.findByText(/暂无已裁决记录/);
  });

  it("历史 bson 扩展 JSON 对象 {$date:{$numberLong}} 不崩溃且能解析出时间", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "E3",
          decision: { verdict: "approved", substance: "旧数据裁决" },
          // 旧部署/缓存残留的 wire 形态（后端已统一 RFC3339，此为防御回归）。
          authorizationExpiresAt: { $date: { $numberLong: "1767225600000" } },
          resolvedVia: "admin",
        },
      ],
    });

    render(<ResolvedEscalations />);
    await screen.findByText("E3");
    // 1767225600000 ms = 2026-01-01T00:00:00Z → 本地化渲染必含 2026 年份。
    expect(screen.getByText(/2026/)).toBeTruthy();
  });

  it("毫秒数形态的到期时间也能渲染", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "E4",
          decision: { verdict: "conditional", substance: "毫秒形态" },
          authorizationExpiresAt: 1767225600000,
          resolvedVia: "admin",
        },
      ],
    });

    render(<ResolvedEscalations />);
    await screen.findByText("E4");
    expect(screen.getByText(/2026/)).toBeTruthy();
  });
});

describe("formatExpiry 防御性时间解析", () => {
  it("RFC3339 字符串（后端契约主形态）解析为本地化时间", () => {
    expect(formatExpiry("2026-01-01T00:00:00Z")).toMatch(/2026/);
  });

  it("毫秒数与历史 bson 扩展 JSON 对象均能解析", () => {
    expect(formatExpiry(1767225600000)).toMatch(/2026/);
    expect(formatExpiry({ $date: { $numberLong: "1767225600000" } })).toMatch(/2026/);
    expect(formatExpiry({ $date: "2026-01-01T00:00:00Z" })).toMatch(/2026/);
  });

  it("空值显示不设期限；无法识别的值绝不原样返回对象", () => {
    expect(formatExpiry(null)).toBe("本次转述不设期限");
    expect(formatExpiry(undefined)).toBe("本次转述不设期限");
    expect(formatExpiry("")).toBe("本次转述不设期限");
    // 无法解析的字符串保持旧行为（原样展示字符串是安全的）。
    expect(formatExpiry("не время")).toBe("не время");
    // 任意对象必须落安全文案（React child 只能是字符串）。
    expect(formatExpiry({ foo: 1 })).toBe("时间格式无法识别");
    expect(formatExpiry({ $date: { $numberLong: "abc" } })).toBe("时间格式无法识别");
  });
});

describe("AskHumanView 已裁决历史切换", () => {
  beforeEach(() => {
    useInboxStore.setState({
      items: [],
      errors: [],
      summary: null,
      loading: false,
      fatalError: null,
      activeSource: null,
    });
  });

  it("切到已裁决历史时调 resolved 端点并展示历史", async () => {
    get.mockResolvedValue({
      items: [
        {
          shortCode: "EH",
          decision: { verdict: "approved", substance: "历史裁决" },
          authorizationExpiresAt: null,
          resolvedVia: "wechat",
        },
      ],
    });

    render(<AskHumanFeature />);
    // 默认在待处理视图，点击切换按钮进入已裁决历史。
    fireEvent.click(screen.getByText("已裁决历史"));
    await screen.findByText("EH");
    expect(get).toHaveBeenCalledWith("/api/admin/principal-escalations?status=resolved");
  });
});
