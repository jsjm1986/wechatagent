import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../../lib/api", () => ({ api: { put: vi.fn().mockResolvedValue({ ok: true }) } }));
import { api } from "../../../lib/api";
import { McpKeyForm } from "../../../features/command-center/McpKeyForm";

describe("McpKeyForm", () => {
  beforeEach(() => vi.clearAllMocks());

  it("提交以 camelCase 键 PUT 密钥,且不回显明文已存值", async () => {
    render(<McpKeyForm accountRecordId="record-1" accountId="acc-1" configured={true} />);
    const input = screen.getByLabelText(/MCP 密钥/);
    expect((input as HTMLInputElement).type).toBe("password");
    expect((input as HTMLInputElement).value).toBe(""); // 不回显已存值
    fireEvent.change(input, { target: { value: "secret-key-123" } });
    fireEvent.click(screen.getByText("保存密钥"));
    await waitFor(() =>
      expect(api.put).toHaveBeenCalledWith(
        "/api/accounts/record-1/mcp-key",
        expect.objectContaining({
          expectedAccountId: "acc-1",
          mcpApiKey: "secret-key-123",
        }),
      ),
    );
    // 提交后输入框清空（不残留明文）
    expect((input as HTMLInputElement).value).toBe("");
  });

  it("切换账号立即销毁密钥草稿，不能把 A 的秘密提交给 B", async () => {
    const { rerender } = render(
      <McpKeyForm accountRecordId="record-a" accountId="account-a" configured={false} />,
    );
    fireEvent.change(screen.getByLabelText(/MCP 密钥/), { target: { value: "secret-a" } });
    fireEvent.change(screen.getByLabelText(/MCP Base URL/), { target: { value: "https://a.example" } });

    rerender(<McpKeyForm accountRecordId="record-b" accountId="account-b" configured={false} />);

    // Passive effect 尚未清空 DOM 前也不能把 A 草稿按 B scope 提交。
    fireEvent.click(screen.getByText("保存密钥"));
    expect(api.put).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByLabelText(/MCP 密钥/)).toHaveValue(""));
    expect(screen.getByLabelText(/MCP Base URL/)).toHaveValue("");
    fireEvent.click(screen.getByText("保存密钥"));
    expect(api.put).not.toHaveBeenCalled();
  });

  it("A 保存响应迟到时不在 B 表单显示已保存", async () => {
    let resolve!: () => void;
    (api.put as ReturnType<typeof vi.fn>).mockImplementationOnce(
      () => new Promise<void>((done) => { resolve = done; }),
    );
    const { rerender } = render(
      <McpKeyForm accountRecordId="record-a" accountId="account-a" configured={false} />,
    );
    fireEvent.change(screen.getByLabelText(/MCP 密钥/), { target: { value: "secret-a" } });
    fireEvent.click(screen.getByText("保存密钥"));
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1));

    rerender(<McpKeyForm accountRecordId="record-b" accountId="account-b" configured={false} />);
    resolve();

    await waitFor(() => expect(screen.getByText("保存密钥")).toBeEnabled());
    expect(screen.queryByText("已保存")).not.toBeInTheDocument();
    expect(api.put).toHaveBeenCalledWith("/api/accounts/record-a/mcp-key", {
      expectedAccountId: "account-a",
      mcpApiKey: "secret-a",
    });
  });
});
