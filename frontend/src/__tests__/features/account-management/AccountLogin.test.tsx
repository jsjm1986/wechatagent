import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AccountLogin } from "../../../features/account-management/AccountLogin";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn(),
    post: vi.fn(),
  },
}));

describe("AccountLogin wire contract", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (api.get as any).mockResolvedValue({ status: "pending" });
  });

  it("renders qr_data_url and polls with session_id", async () => {
    (api.post as any).mockResolvedValueOnce({
      session_id: "login-session-1",
      qr_data_url: "data:image/png;base64,AAAA",
      login_page_url: "https://mcp.example/login",
      status: "pending",
    });

    render(<AccountLogin />);
    await userEvent.click(screen.getByRole("button", { name: "\u5f00\u59cb\u767b\u5f55" }));

    expect(await screen.findByRole("img", { name: "\u767b\u5f55\u4e8c\u7ef4\u7801" })).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    );
    expect(screen.getByRole("link", { name: /MCP/ })).toHaveAttribute(
      "href",
      "https://mcp.example/login",
    );
    await waitFor(() => {
      expect(api.get).toHaveBeenCalledWith(
        "/api/accounts/login/poll?loginSessionId=login-session-1",
      );
    });
  });

  it("does not accept the obsolete login_session_id fields", async () => {
    (api.post as any).mockResolvedValueOnce({
      login_session_id: "obsolete-session",
      qr_code_base64: "data:image/png;base64,OLD",
    });

    render(<AccountLogin />);
    await userEvent.click(screen.getByRole("button", { name: "\u5f00\u59cb\u767b\u5f55" }));

    expect(await screen.findByText("login response missing session_id")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "\u767b\u5f55\u4e8c\u7ef4\u7801" })).not.toBeInTheDocument();
    expect(api.get).not.toHaveBeenCalled();
  });

  it("drops a late poll success after the login session is canceled", async () => {
    let resolvePoll!: (value: { status: string; wxid: string }) => void;
    const pollResponse = new Promise<{ status: string; wxid: string }>((resolve) => {
      resolvePoll = resolve;
    });
    const onLoggedIn = vi.fn();
    (api.post as any).mockResolvedValueOnce({
      session_id: "login-session-stale",
      qr_data_url: "data:image/png;base64,STALE",
    });
    (api.get as any).mockReturnValueOnce(pollResponse);

    render(<AccountLogin onLoggedIn={onLoggedIn} />);
    await userEvent.click(screen.getByRole("button", { name: "\u5f00\u59cb\u767b\u5f55" }));
    await userEvent.click(await screen.findByRole("button", { name: "\u53d6\u6d88" }));
    await act(async () => {
      resolvePoll({ status: "success", wxid: "wx-stale" });
      await pollResponse;
    });

    expect(screen.queryByText("wx-stale")).not.toBeInTheDocument();
    expect(onLoggedIn).not.toHaveBeenCalled();
    expect(api.post).toHaveBeenCalledTimes(1);
  });
});
