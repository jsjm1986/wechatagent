import { describe, expect, it, beforeEach, vi } from "vitest";
import { api } from "../../lib/api";
import { useUserOpsStore } from "../../stores/userOpsStore";
import { useContactStore } from "../../stores/contactStore";
import type { Contact } from "../../types";

// Mock API：断言 saveManualTags 调 api.put 的 URL/body。
vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn().mockResolvedValue({}),
    put: vi.fn().mockResolvedValue({}),
    patch: vi.fn().mockResolvedValue({}),
    delete: vi.fn().mockResolvedValue({}),
  },
}));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError: vi.fn(), setBusy: vi.fn() }) },
}));

const contact = (id: string): Contact =>
  ({ id, agentStatus: "managed" } as Contact);

describe("userOpsStore.saveManualTags", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useContactStore.setState({ contacts: [], selected: null, contactTab: "all" });
  });

  it("无选中联系人时早退、不调 api.put", async () => {
    await useUserOpsStore.getState().saveManualTags(["vip"]);
    expect(api.put).not.toHaveBeenCalled();
  });

  it("有选中联系人时以正确 URL/body 调 api.put", async () => {
    useContactStore.setState({ selected: contact("c123") });
    await useUserOpsStore.getState().saveManualTags(["vip"]);
    expect(api.put).toHaveBeenCalledWith("/api/contacts/c123/manual-tags", { tags: ["vip"] });
  });
});

describe("userOpsStore.loadMessages", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUserOpsStore.setState({
      messages: [], operatingMemory: null, memoryCandidates: [],
      decisionReviews: [], operationHealth: null,
    } as any);
  });

  it("调用正确的会话与决策复盘端点", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url.includes("/messages")) return Promise.resolve({ items: [{ id: "m1" }] });
      if (url.includes("/operating-memory")) return Promise.resolve({ item: { id: "om" } });
      if (url.includes("/memory-candidates")) return Promise.resolve({ items: [] });
      if (url.includes("/decision-reviews")) return Promise.resolve({ items: [{ id: "dr1" }] });
      if (url.includes("/operation-health")) return Promise.resolve({ ok: true });
      return Promise.reject(new Error("unexpected url " + url));
    });

    await useUserOpsStore.getState().loadMessages(contact("C1"));

    const calledUrls = (api.get as any).mock.calls.map((c: any[]) => c[0]);
    expect(calledUrls).toContain("/api/conversations/C1/messages?limit=50");
    expect(calledUrls).toContain("/api/decision-reviews?contactId=C1&limit=20");
    // 不再调用已废弃的 contact-scoped 死端点
    expect(calledUrls).not.toContain("/api/contacts/C1/messages?limit=50");
    expect(calledUrls).not.toContain("/api/contacts/C1/decision-reviews?limit=20");

    const st = useUserOpsStore.getState();
    expect(st.messages).toEqual([{ id: "m1" }]);
    expect(st.decisionReviews).toEqual([{ id: "dr1" }]);
  });

  it("单面板失败不拖垮其余面板（allSettled 加固）", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url.includes("/operation-health")) return Promise.reject(new Error("health 500"));
      if (url.includes("/messages")) return Promise.resolve({ items: [{ id: "m1" }] });
      if (url.includes("/operating-memory")) return Promise.resolve({ item: { id: "om" } });
      if (url.includes("/memory-candidates")) return Promise.resolve({ items: [] });
      if (url.includes("/decision-reviews")) return Promise.resolve({ items: [{ id: "dr1" }] });
      return Promise.reject(new Error("unexpected"));
    });

    await useUserOpsStore.getState().loadMessages(contact("C1"));

    const st = useUserOpsStore.getState();
    expect(st.messages).toEqual([{ id: "m1" }]);      // 成功面板照常填充
    expect(st.operationHealth).toBeNull();             // 失败面板保持默认
  });
});

describe("userOpsStore.clearReferral", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useContactStore.setState({ contacts: [], selected: null, contactTab: "all" });
  });

  it("clearReferral 调用撤销引荐端点", async () => {
    await useUserOpsStore.getState().clearReferral("C1");
    expect(api.post).toHaveBeenCalledWith("/api/contacts/C1/clear-referral");
  });
});
