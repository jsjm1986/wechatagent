import { describe, expect, it, beforeEach } from "vitest";
import { useAccountStore } from "../../stores/accountStore";
import type { Account } from "../../types";

const acc = (id: string, online = true): Account =>
  ({ id, accountId: id, alias: "", displayName: "", online } as Account);

describe("accountStore", () => {
  beforeEach(() => useAccountStore.setState({ accounts: [], selectedAccountId: "" }));

  it("currentAccountId 在 selected 无效时回退首个账号", () => {
    useAccountStore.getState().setAccounts([acc("a"), acc("b")]);
    expect(useAccountStore.getState().currentAccountId()).toBe("a");
  });

  it("onlineCount 统计在线账号数", () => {
    useAccountStore.getState().setAccounts([acc("a", true), acc("b", false)]);
    expect(useAccountStore.getState().onlineCount()).toBe(1);
  });

  it("selectAccount 写入并持久化", () => {
    useAccountStore.getState().setAccounts([acc("a"), acc("b")]);
    useAccountStore.getState().selectAccount("b");
    expect(useAccountStore.getState().currentAccountId()).toBe("b");
    expect(localStorage.getItem("wechatagent.accountId")).toBe("b");
  });
});
