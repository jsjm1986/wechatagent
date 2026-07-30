import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({
      items: [{ id: "id-1", scenarioId: "S1", title: "议价场景", status: "active" }],
    }),
    post: vi.fn().mockResolvedValue({ ok: true }),
    delete: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

import { api } from "../../../lib/api";
import { EvaluationScenariosPanel } from "../../../features/quality/EvaluationScenariosPanel";

describe("EvaluationScenariosPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.get).mockImplementation(async (url: string) => {
      if (url === "/api/admin/domain-profiles/active") return { item: null } as never;
      return {
        items: [{ id: "id-1", scenarioId: "S1", title: "议价场景", status: "active" }],
      } as never;
    });
  });

  function fillRequiredScenario(id: string, title: string) {
    fireEvent.change(screen.getByPlaceholderText(/场景标识/), { target: { value: id } });
    fireEvent.change(screen.getByPlaceholderText(/场景标题/), { target: { value: title } });
    fireEvent.change(screen.getByPlaceholderText(/每行一条/), {
      target: { value: "你好" },
    });
    for (const input of screen.getAllByPlaceholderText("0–10")) {
      fireEvent.change(input, { target: { value: "7" } });
    }
  }

  it("渲染评测场景列表", async () => {
    render(<EvaluationScenariosPanel accountId="account-a" />);
    await waitFor(() => expect(screen.getByText("议价场景")).toBeInTheDocument());
  });

  it("新建场景提交 POST", async () => {
    render(<EvaluationScenariosPanel accountId="account-a" />);
    await waitFor(() => screen.getByText("议价场景"));
    fillRequiredScenario("S2", "退款场景");
    fireEvent.click(screen.getByText("新建场景"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/evaluation-scenarios",
        expect.objectContaining({
          scenarioId: "S2",
          title: "退款场景",
          accountId: "account-a",
          status: "active",
          groundTruth: {
            trust: 7,
            conversionReadiness: 7,
            emotionalValue: 7,
            nextBestActionScore: 7,
          },
        }),
      ),
    );
  });

  it("新建场景将多行输入消息拆成 inboundMessages 数组", async () => {
    render(<EvaluationScenariosPanel accountId="account-a" />);
    await waitFor(() => screen.getByText("议价场景"));
    fillRequiredScenario("S3", "咨询场景");
    fireEvent.change(screen.getByPlaceholderText(/每行一条/), {
      target: { value: "你好\n在吗\n" },
    });
    fireEvent.click(screen.getByText("新建场景"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/evaluation-scenarios",
        expect.objectContaining({
          scenarioId: "S3",
          title: "咨询场景",
          inboundMessages: ["你好", "在吗"],
        }),
      ),
    );
  });

  it("删除场景调用 DELETE 端点", async () => {
    render(<EvaluationScenariosPanel accountId="account-a" />);
    await waitFor(() => screen.getByText("议价场景"));
    fireEvent.click(screen.getByText("删除"));
    await waitFor(() =>
      expect(api.delete).toHaveBeenCalledWith("/api/evaluation-scenarios/id-1"),
    );
  });

  it("按 active DomainProfile 的动态公式录入完整金标", async () => {
    vi.mocked(api.get).mockImplementation(async (url: string) => {
      if (url === "/api/admin/domain-profiles/active") {
        return {
          item: {
            business_formulas: [
              { key: "relationshipHealth", display_name: "关系健康度", expression: "" },
            ],
          },
        } as never;
      }
      return { items: [] } as never;
    });
    render(<EvaluationScenariosPanel accountId="account-b" />);
    await waitFor(() => screen.getByText("关系健康度"));
    fillRequiredScenario("S4", "关系维护");
    fireEvent.change(screen.getByPlaceholderText("0–10"), { target: { value: "8.5" } });
    fireEvent.click(screen.getByText("新建场景"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/evaluation-scenarios",
        expect.objectContaining({
          accountId: "account-b",
          groundTruth: { relationshipHealth: 8.5 },
        }),
      ),
    );
  });
});
