import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";
import { AdminGovernanceView } from "../../../features/knowledge/atlas";

const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
  vi.restoreAllMocks();
});

function response(body: unknown, ok = true): Response {
  return {
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

const ISO_FIXTURE = "2026-06-26T07:25:11.049Z";

// 各治理面板的 GET 端点 → 固定 fixture。PublishBar 的 POST 不在本文件覆盖。
function mockGovernanceApi() {
  globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.startsWith("/api/admin/taxonomies")) {
      return response({
        items: [{
          id: "tax-a",
          scope: "global",
          kind: "churn_reason",
          value: { id: "need_changed", displayName: "需求变化", status: "active" },
          version: 1,
          currentVersion: true,
          updatedAt: ISO_FIXTURE,
        }],
      });
    }
    if (url.startsWith("/api/admin/operation-state-policies")) {
      return response({
        items: [{
          id: "policy-a",
          domain: "DEFAULT",
          version: 2,
          currentVersion: true,
          updatedAt: ISO_FIXTURE,
          states: [{ id: "new_contact" }],
        }],
      });
    }
    if (url.startsWith("/api/operation-domains")) {
      return response({
        items: [{
          id: "domain-a",
          domain: "DEFAULT",
          version: 3,
          currentVersion: true,
          updatedAt: ISO_FIXTURE,
        }],
      });
    }
    return response({ items: [] });
  }) as typeof fetch;
}

function renderGovernance() {
  return render(
    <ToastProvider>
      <ConfirmProvider>
        <AdminGovernanceView />
      </ConfirmProvider>
    </ToastProvider>,
  );
}

// 切到指定 tab。治理工坊四个 tab 同时只渲染一个面板，故全局查询不会串台。
async function openTab(tabName: string): Promise<HTMLTableElement> {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: tabName }));
  return waitFor(() => {
    const found = document.querySelector("table.wikiAdminTable");
    if (!found) throw new Error(`${tabName} 面板未渲染表格`);
    return found as HTMLTableElement;
  });
}

describe("治理工坊 PublishBar 按钮兜色", () => {
  // 三个按钮都在白底(.wikiPublishBar button 的 background)上。全局 button 基线
  // 是 color:#fff，任何一个按钮少了显式 color 就会白底白字彻底不可见。
  // 「发布给全部」触发不可逆的全量推送，尤其不能隐形。
  // jsdom 无 CSS 层叠，无法断言实际颜色，故断言每个按钮都带兜色 class——
  // 类名到颜色的映射由 Knowledge.css 保证，实际颜色需目视确认。
  it("三个按钮各自带兜色 class，无裸 button", async () => {
    mockGovernanceApi();
    renderGovernance();
    await openTab("分类系统");

    const bar = document.querySelector(".wikiPublishBar");
    expect(bar).not.toBeNull();

    const buttons = Array.from(bar!.querySelectorAll("button"));
    expect(buttons).toHaveLength(3);

    buttons.forEach((button) => {
      expect(button.className.trim()).not.toBe("");
    });

    expect(
      screen.getByRole("button", { name: /发布新版/ }).className,
    ).toContain("wikiActionBtn--verify");
    expect(
      screen.getByRole("button", { name: /发布给全部/ }).className,
    ).toContain("wikiActionBtn--neutral");
    expect(
      screen.getByRole("button", { name: /回退上版/ }).className,
    ).toContain("wikiActionBtn--reject");
  });
});
