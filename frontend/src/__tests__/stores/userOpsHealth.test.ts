import { describe, expect, it, beforeEach, vi } from "vitest";
import { api } from "../../lib/api";
import { useUserOpsStore } from "../../stores/userOpsStore";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import type { Account, Contact } from "../../types";

// FE-1 前端回归：guide preview 分支必须把后端构建好的 `data.item.health`
//（scores + canonical 7 项 items）原样赋给 operationHealth，
// 而不是用已删除的 healthFromScores 拿 healthScores 重建 4-key 占位分。
vi.mock("../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [] }),
    post: vi.fn(),
    put: vi.fn().mockResolvedValue({}),
    patch: vi.fn().mockResolvedValue({}),
    delete: vi.fn().mockResolvedValue({}),
  },
}));

const contact = (id: string): Contact =>
  ({ id, accountId: "a1", agentStatus: "managed" } as Contact);

const account = (id: string): Account =>
  ({ accountId: id, online: true } as Account);

// 后端 Task 3 产出的 guide preview 响应形态：health = { scores, items }（canonical 7 项）。
const backendHealth = {
  scores: { userUnderstanding: 80, hallucinationRisk: 90 },
  items: [
    { key: "userUnderstanding", label: "用户理解完整度", score: 80, tone: "good", detail: "..." },
    { key: "relationshipQuality", label: "信任关系质量", score: 70, tone: "good", detail: "..." },
    { key: "productFit", label: "产品匹配清晰度", score: 60, tone: "warn", detail: "..." },
    { key: "rhythmRisk", label: "跟进节奏风险", score: 20, tone: "good", detail: "..." },
    { key: "knowledgeGrounding", label: "知识匹配度", score: 75, tone: "good", detail: "..." },
    { key: "hallucinationRisk", label: "幻觉风险", score: 90, tone: "danger", detail: "..." },
    { key: "pressureRisk", label: "销售压迫感风险", score: 30, tone: "good", detail: "..." },
  ],
};

describe("guide preview health", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const selected = contact("c1");
    useContactStore.setState({
      contacts: [selected],
      selected,
      dataAccountId: "a1",
      contactTab: "all",
    });
    useAccountStore.setState({ accounts: [account("a1")], selectedAccountId: "a1" });
    useUserOpsStore.getState().hydrateSelected(selected);
    useUserOpsStore.setState({ operationHealth: null, guidePreview: null });
    (api.post as ReturnType<typeof vi.fn>).mockResolvedValue({
      item: {
        id: "p1",
        accountId: "a1",
        contactId: "c1",
        health: backendHealth,
        // 后端仍保留旧 healthScores 键（兼容），前端迁移后不再消费它。
        healthScores: { trust_level: 5, engagement: 6 },
      },
    });
  });

  it("uses backend-built health.items (7 canonical items), not 4-key placeholder", async () => {
    await useUserOpsStore.getState().previewGuideInstruction("更专业一点");

    const operationHealth = useUserOpsStore.getState().operationHealth;
    expect(operationHealth).not.toBeNull();
    expect(operationHealth!.items.length).toBe(7);

    // 风险类高分=danger（后端已按 key.ends_with("Risk") 反转量纲算对）。
    const halluc = operationHealth!.items.find((i) => i.key === "hallucinationRisk");
    expect(halluc?.tone).toBe("danger");

    // 不再含旧 4-key 占位项。
    const keys = operationHealth!.items.map((i) => i.key);
    expect(keys).not.toContain("trust_level");
    expect(keys).not.toContain("engagement");
    expect(keys).not.toContain("intent_clarity");
    expect(keys).not.toContain("relationship_depth");

    // canonical key 在场。
    expect(keys).toContain("userUnderstanding");
    expect(keys).toContain("relationshipQuality");
    expect(keys).toContain("productFit");
  });
});
