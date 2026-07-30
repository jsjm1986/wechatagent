import { describe, expect, it, beforeEach, vi } from "vitest";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import { useContactStore } from "../../stores/contactStore";
import type { Account, Contact } from "../../types";

vi.mock("../../lib/api", () => ({ api: { get: vi.fn() } }));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError: vi.fn() }) },
}));

const c = (id: string, managed: boolean, accountId = "A"): Contact =>
  ({ id, accountId, agentStatus: managed ? "managed" : "normal" } as Contact);

const account = (accountId: string): Account =>
  ({ id: accountId, accountId, alias: accountId, displayName: accountId, online: true } as Account);

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("contactStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [account("A"), account("B")],
      selectedAccountId: "A",
    });
    useContactStore.setState({
      contacts: [],
      selected: null,
      dataAccountId: "A",
      requestGeneration: 0,
      loading: false,
      contactTab: "all",
    });
  });

  it("managedCount / normalCount 派生正确", () => {
    useContactStore.getState().setContacts([c("a", true), c("b", false), c("d", true)]);
    expect(useContactStore.getState().managedCount()).toBe(2);
    expect(useContactStore.getState().normalCount()).toBe(1);
  });

  it("A 慢 B 快时只提交当前 B 账号联系人快照", async () => {
    const responseA = deferred<{ items: Contact[] }>();
    const responseB = deferred<{ items: Contact[] }>();
    (api.get as any).mockImplementation((url: string) =>
      url.includes("accountId=A") ? responseA.promise : responseB.promise
    );

    const loadA = useContactStore.getState().loadContacts("A");
    useAccountStore.setState({ selectedAccountId: "B" });
    const loadB = useContactStore.getState().loadContacts("B");

    responseB.resolve({ items: [c("contact-b", true, "B")] });
    await loadB;
    responseA.resolve({ items: [c("contact-a", true, "A")] });
    await loadA;

    expect(useContactStore.getState().dataAccountId).toBe("B");
    expect(useContactStore.getState().contacts.map((item) => item.id)).toEqual(["contact-b"]);
  });
});
