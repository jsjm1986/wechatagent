import { describe, it, expect, vi, beforeEach } from "vitest";

const setError = vi.fn();
const setBusy = vi.fn();
vi.mock("../../lib/api", () => ({
  api: { get: vi.fn(), post: vi.fn(), put: vi.fn() },
}));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError, setBusy }) },
}));

import { api } from "../../lib/api";
import { useStrategyStore } from "../../stores/strategyStore";

describe("strategyStore.saveDomainProfile create/update 双路（D5 死链路修复）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // loadDomainProfiles 在保存后会被调用，给个空列表避免抛错。
    (api.get as any).mockResolvedValue({ items: [] });
  });

  it("无 editingProfile（新建）时走 POST /api/admin/domain-profiles", async () => {
    (api.post as any).mockResolvedValue({ item: { id: "new1" } });
    useStrategyStore.setState({
      editingProfile: null,
      profileDraft: { profile_id: "p_new", display_name: "新域" },
    });
    await useStrategyStore.getState().saveDomainProfile();
    expect(api.post).toHaveBeenCalledWith(
      "/api/admin/domain-profiles",
      expect.objectContaining({ profileId: "p_new" }),
    );
    expect(api.put).not.toHaveBeenCalled();
  });

  it("有 editingProfile（编辑）时走 PUT /api/admin/domain-profiles/:id", async () => {
    (api.put as any).mockResolvedValue({ item: { id: "x1" } });
    useStrategyStore.setState({
      editingProfile: { id: "x1" } as any,
      profileDraft: { display_name: "改" },
    });
    await useStrategyStore.getState().saveDomainProfile();
    expect(api.put).toHaveBeenCalledWith(
      "/api/admin/domain-profiles/x1",
      expect.anything(),
    );
    expect(api.post).not.toHaveBeenCalled();
  });
});
