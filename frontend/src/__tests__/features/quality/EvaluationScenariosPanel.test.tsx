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
  beforeEach(() => vi.clearAllMocks());

  it("渲染评测场景列表", async () => {
    render(<EvaluationScenariosPanel />);
    await waitFor(() => expect(screen.getByText("议价场景")).toBeInTheDocument());
  });

  it("新建场景提交 POST", async () => {
    render(<EvaluationScenariosPanel />);
    await waitFor(() => screen.getByText("议价场景"));
    fireEvent.change(screen.getByPlaceholderText(/场景标识/), { target: { value: "S2" } });
    fireEvent.change(screen.getByPlaceholderText(/场景标题/), { target: { value: "退款场景" } });
    fireEvent.click(screen.getByText("新建场景"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/evaluation-scenarios",
        expect.objectContaining({ scenarioId: "S2", title: "退款场景" }),
      ),
    );
  });

  it("新建场景将多行输入消息拆成 inboundMessages 数组", async () => {
    render(<EvaluationScenariosPanel />);
    await waitFor(() => screen.getByText("议价场景"));
    fireEvent.change(screen.getByPlaceholderText(/场景标识/), { target: { value: "S3" } });
    fireEvent.change(screen.getByPlaceholderText(/场景标题/), { target: { value: "咨询场景" } });
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
    render(<EvaluationScenariosPanel />);
    await waitFor(() => screen.getByText("议价场景"));
    fireEvent.click(screen.getByText("删除"));
    await waitFor(() =>
      expect(api.delete).toHaveBeenCalledWith("/api/evaluation-scenarios/id-1"),
    );
  });
});
