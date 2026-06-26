import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../../lib/api", () => ({ api: { put: vi.fn().mockResolvedValue({ ok: true }) } }));
import { api } from "../../../lib/api";
import { McpKeyForm } from "../../../features/command-center/McpKeyForm";

describe("McpKeyForm", () => {
  beforeEach(() => vi.clearAllMocks());

  it("提交以 snake_case 键 PUT 密钥,且不回显明文已存值", async () => {
    render(<McpKeyForm accountId="acc-1" configured={true} />);
    const input = screen.getByLabelText(/MCP 密钥/);
    expect((input as HTMLInputElement).type).toBe("password");
    expect((input as HTMLInputElement).value).toBe(""); // 不回显已存值
    fireEvent.change(input, { target: { value: "secret-key-123" } });
    fireEvent.click(screen.getByText("保存密钥"));
    await waitFor(() =>
      expect(api.put).toHaveBeenCalledWith(
        "/api/accounts/acc-1/mcp-key",
        expect.objectContaining({ mcp_api_key: "secret-key-123" }),
      ),
    );
    // 提交后输入框清空（不残留明文）
    expect((input as HTMLInputElement).value).toBe("");
  });
});
