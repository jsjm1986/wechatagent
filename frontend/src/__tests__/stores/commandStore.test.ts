import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import { useCommandStore } from "../../stores/commandStore";
import { useUiStore } from "../../stores/uiStore";

vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn(),
    post: vi.fn(),
  },
}));

const pendingCommand = {
  id: "run-1",
  accountId: "account-a",
  planHash: "frozen-plan-hash",
  status: "pending_confirmation",
  summary: "待确认",
  toolCalls: [],
};

describe("commandStore frozen confirmation binding", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUiStore.setState({ error: "" });
    useAccountStore.setState({
      accounts: [
        {
          id: "a",
          accountId: "account-a",
          alias: "Account A",
          displayName: "Account A",
          online: true,
        },
        {
          id: "b",
          accountId: "account-b",
          alias: "Account B",
          displayName: "Account B",
          online: true,
        },
      ],
      selectedAccountId: "account-a",
    });
    useCommandStore.setState({
      commandResult: pendingCommand,
      commandBusy: false,
    });
  });

  it("sends accountId and planHash when confirming", async () => {
    vi.mocked(api.post).mockResolvedValue({
      status: "succeeded",
      summary: "完成",
      toolCalls: [],
    });

    await useCommandStore.getState().confirmCommand("run-1");

    expect(api.post).toHaveBeenCalledWith(
      "/api/management-agent/commands/run-1/confirm",
      { accountId: "account-a", planHash: "frozen-plan-hash" },
    );
    expect(useCommandStore.getState().commandResult?.status).toBe("succeeded");
  });

  it("sends the same frozen binding when rejecting", async () => {
    vi.mocked(api.post).mockResolvedValue({ status: "canceled" });

    await useCommandStore.getState().rejectCommand("run-1");

    expect(api.post).toHaveBeenCalledWith(
      "/api/management-agent/commands/run-1/reject",
      { accountId: "account-a", planHash: "frozen-plan-hash" },
    );
    expect(useCommandStore.getState().commandResult?.status).toBe("canceled");
  });

  it("does not confirm a plan after the selected account changes", async () => {
    useAccountStore.setState({ selectedAccountId: "account-b" });

    await useCommandStore.getState().confirmCommand("run-1");

    expect(api.post).not.toHaveBeenCalled();
    expect(useUiStore.getState().error).toContain("不属于当前账号");
  });

  it("does not confirm a legacy plan without a hash", async () => {
    useCommandStore.setState({
      commandResult: { ...pendingCommand, planHash: "" },
    });

    await useCommandStore.getState().confirmCommand("run-1");

    expect(api.post).not.toHaveBeenCalled();
    expect(useUiStore.getState().error).toContain("缺少冻结标识");
  });
});
