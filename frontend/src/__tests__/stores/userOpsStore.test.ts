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
