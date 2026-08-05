import { describe, expect, it, beforeEach, vi } from "vitest";
import { api } from "../../lib/api";
import { useUserOpsStore } from "../../stores/userOpsStore";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import type {
  Account,
  Contact,
  OperationDomainDraft,
  OperationPlaybook,
  UserOperationGuidePreview,
} from "../../types";

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

const contact = (id: string, accountId = "A1"): Contact =>
  ({ id, accountId, agentStatus: "managed" } as Contact);

const account = (accountId: string): Account =>
  ({ id: accountId, accountId, alias: accountId, displayName: accountId, online: true } as Account);

function bindContact(selected: Contact): void {
  useAccountStore.setState({
    accounts: [account(selected.accountId)],
    selectedAccountId: selected.accountId,
  });
  useContactStore.setState({
    contacts: [selected],
    selected,
    dataAccountId: selected.accountId,
    contactTab: "all",
  });
  useUserOpsStore.getState().hydrateSelected(selected);
}

const playbook = (id: string, accountId: string, version = 1): OperationPlaybook => ({
  id,
  accountId,
  name: `${accountId}-${id}`,
  methodPrompt: "method",
  createdBy: "manual",
  releaseStatus: "published",
  isDefault: false,
  version
});

const guidePreview = (
  id: string,
  accountId: string,
  contactId: string,
): UserOperationGuidePreview => ({
  id,
  accountId,
  contactId,
  contactWxid: `wx-${contactId}`,
  instruction: "guide",
  mode: "smart",
  status: "pending",
  summary: "preview",
  impactScope: "current_contact",
  scopeReason: "test",
  readableChanges: [],
  healthScores: {},
  suggestedChanges: {},
  riskWarnings: [],
  candidateHash: `hash-${id}`,
  authoritativeChanges: [],
  requiresStrongConfirmation: false,
  playbookAffectedContacts: 0,
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("userOpsStore.saveManualTags", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [account("A1")],
      selectedAccountId: "A1",
    });
    useContactStore.setState({
      contacts: [],
      selected: null,
      dataAccountId: "A1",
      contactTab: "all",
    });
    useUserOpsStore.getState().clearContactDetail("A1");
  });

  it("无选中联系人时早退、不调 api.put", async () => {
    await useUserOpsStore.getState().saveManualTags(contact("missing"), ["vip"]);
    expect(api.put).not.toHaveBeenCalled();
  });

  it("有选中联系人时以正确 URL/body 调 api.put", async () => {
    const selected = contact("c123");
    bindContact(selected);
    await useUserOpsStore.getState().saveManualTags(selected, ["vip"]);
    expect(api.put).toHaveBeenCalledWith("/api/contacts/c123/manual-tags", {
      expectedAccountId: "A1",
      tags: ["vip"],
    });
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

    const selected = contact("C1");
    bindContact(selected);
    await useUserOpsStore.getState().loadMessages(selected);

    const calledUrls = (api.get as any).mock.calls.map((c: any[]) => c[0]);
    expect(calledUrls).toContain("/api/conversations/C1/messages?limit=50");
    expect(calledUrls).toContain("/api/decision-reviews?accountId=A1&contactId=C1&limit=20");
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

    const selected = contact("C1");
    bindContact(selected);
    await useUserOpsStore.getState().loadMessages(selected);

    const st = useUserOpsStore.getState();
    expect(st.messages).toEqual([{ id: "m1" }]);      // 成功面板照常填充
    expect(st.operationHealth).toBeNull();             // 失败面板保持默认
  });

  it("A 详情迟到时不得覆盖已切换到 B 的详情草稿", async () => {
    const responseA = deferred<any>();
    const responseB = deferred<any>();
    (api.get as any).mockImplementation((url: string) =>
      url.includes("contact-a") ? responseA.promise : responseB.promise
    );

    const contactA = contact("contact-a", "A");
    const contactB = contact("contact-b", "B");
    useAccountStore.setState({
      accounts: [account("A"), account("B")],
      selectedAccountId: "A",
    });
    useContactStore.setState({ contacts: [contactA], selected: contactA, dataAccountId: "A" });
    useUserOpsStore.getState().hydrateSelected(contactA);
    const loadA = useUserOpsStore.getState().loadMessages(contactA);

    useAccountStore.setState({ selectedAccountId: "B" });
    useContactStore.setState({ contacts: [contactB], selected: contactB, dataAccountId: "B" });
    useUserOpsStore.getState().hydrateSelected(contactB);
    const loadB = useUserOpsStore.getState().loadMessages(contactB);

    responseB.resolve({
      items: [],
      item: { userUnderstanding: { identity: "B 的记忆" } },
    });
    await loadB;
    responseA.resolve({
      items: [],
      item: { userUnderstanding: { identity: "A 的记忆" } },
    });
    await loadA;

    expect(useUserOpsStore.getState().detailAccountId).toBe("B");
    expect(useUserOpsStore.getState().detailContactId).toBe("contact-b");
    expect(useUserOpsStore.getState().memoryDraft.identity).toBe("B 的记忆");
  });
});

describe("userOpsStore stale contact actions", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("A 标签编辑后切到 B，保存旧草稿必须零请求", async () => {
    const contactA = contact("contact-a", "A");
    const contactB = contact("contact-b", "B");
    useAccountStore.setState({
      accounts: [account("A"), account("B")],
      selectedAccountId: "A",
    });
    useContactStore.setState({ contacts: [contactA], selected: contactA, dataAccountId: "A" });
    useUserOpsStore.getState().hydrateSelected(contactA);

    useAccountStore.setState({ selectedAccountId: "B" });
    useContactStore.setState({ contacts: [contactB], selected: contactB, dataAccountId: "B" });
    useUserOpsStore.getState().hydrateSelected(contactB);
    await useUserOpsStore.getState().saveManualTags(contactA, ["A 的旧草稿"]);

    expect(api.put).not.toHaveBeenCalled();
  });
});

describe("userOpsStore guide preview identity", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUserOpsStore.setState({
      guidePreview: null,
      guidePreviewGeneration: 0,
      guideBusy: false,
    });
  });

  it("drops a late A preview after B becomes current", async () => {
    const previewA = deferred<{ item: UserOperationGuidePreview }>();
    const previewB = deferred<{ item: UserOperationGuidePreview }>();
    (api.post as any).mockImplementation((_url: string, body: { accountId: string }) =>
      body.accountId === "A" ? previewA.promise : previewB.promise
    );

    const contactA = contact("contact-a", "A");
    const contactB = contact("contact-b", "B");
    useAccountStore.setState({
      accounts: [account("A"), account("B")],
      selectedAccountId: "A",
    });
    useContactStore.setState({ contacts: [contactA], selected: contactA, dataAccountId: "A" });
    useUserOpsStore.getState().hydrateSelected(contactA);
    const requestA = useUserOpsStore.getState().previewGuideInstruction("A guide");

    useAccountStore.setState({ selectedAccountId: "B" });
    useContactStore.setState({ contacts: [contactB], selected: contactB, dataAccountId: "B" });
    useUserOpsStore.getState().hydrateSelected(contactB);
    const requestB = useUserOpsStore.getState().previewGuideInstruction("B guide");

    previewB.resolve({ item: guidePreview("preview-b", "B", "contact-b") });
    await requestB;
    previewA.resolve({ item: guidePreview("preview-a", "A", "contact-a") });
    await requestA;

    expect(useUserOpsStore.getState().guidePreview?.id).toBe("preview-b");
    expect(useUserOpsStore.getState().guideBusy).toBe(false);
  });

  it("sends no apply request for a stale preview from another contact", async () => {
    const contactB = contact("contact-b", "B");
    bindContact(contactB);
    useUserOpsStore.setState({
      guidePreview: guidePreview("preview-a", "A", "contact-a"),
      guidePreviewGeneration: 4,
    });

    const result = await useUserOpsStore.getState().applyGuidePreview();

    expect(result).toBeNull();
    expect(api.post).not.toHaveBeenCalled();
  });

  it("freezes account and contact identity in a valid apply request", async () => {
    const selected = contact("contact-a", "A");
    bindContact(selected);
    useUserOpsStore.setState({
      guidePreview: guidePreview("preview-a", "A", "contact-a"),
      guidePreviewGeneration: 7,
    });
    (api.post as any).mockResolvedValueOnce({
      item: {
        committed: true,
        previewId: "preview-a",
        candidateHash: "hash-preview-a",
        appliedFields: [],
        skippedFields: [],
        impactScope: "current_contact",
      },
    });

    await useUserOpsStore.getState().applyGuidePreview();

    expect(api.post).toHaveBeenCalledWith("/api/user-operations/guide/apply", {
      previewId: "preview-a",
      expectedAccountId: "A",
      expectedContactId: "contact-a",
      candidateHash: "hash-preview-a",
      confirmGlobalImpact: false,
    });
  });
});

describe("userOpsStore.saveOperatingMemory", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useContactStore.setState({ contacts: [], selected: null, contactTab: "all" });
  });

  it("无选中联系人时早退、不调 api.put", async () => {
    await useUserOpsStore.getState().saveOperatingMemory();
    expect(api.put).not.toHaveBeenCalled();
  });

  it("把扁平 memoryDraft 归组进四个 Document 后 PUT operating-memory", async () => {
    bindContact(contact("c1"));
    useUserOpsStore.getState().setMemoryDraft({
      identity: "工程师",
      relationshipGoal: "长期信任",
      interestedProducts: "A产品",
      nextGoal: "确认需求",
    });
    await useUserOpsStore.getState().saveOperatingMemory();
    expect(api.put).toHaveBeenCalledWith(
      "/api/contacts/c1/operating-memory",
      expect.objectContaining({
        userUnderstanding: expect.objectContaining({ identity: "工程师" }),
        relationshipState: expect.objectContaining({ relationshipGoal: "长期信任" }),
        productFit: expect.objectContaining({ interestedProducts: "A产品" }),
        nextAction: expect.objectContaining({ nextGoal: "确认需求" }),
      }),
    );
  });
});

describe("userOpsStore.saveOperationProfile", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useContactStore.setState({ contacts: [], selected: null, contactTab: "all" });
    useUserOpsStore.setState({ relationshipType: "", profileEditDraft: {}, selectedPlaybookId: "" } as any);
  });

  it("无选中联系人时早退、不调 api.put", async () => {
    await useUserOpsStore.getState().saveOperationProfile();
    expect(api.put).not.toHaveBeenCalled();
  });

  it("提交 relationshipType + lastCommitment + followUpPolicy，且 body 不含 AI 派生字段", async () => {
    bindContact(contact("c1"));
    useUserOpsStore.setState({
      relationshipType: "customer",
      profileEditDraft: { lastCommitment: "下周回复", followUpPolicy: "每周跟进" },
      selectedPlaybookId: "playbook-1",
    } as any);
    await useUserOpsStore.getState().saveOperationProfile();
    expect(api.put).toHaveBeenCalledWith(
      "/api/contacts/c1/operation-profile",
      expect.objectContaining({
        relationshipType: "customer",
        lastCommitment: "下周回复",
        followUpPolicy: "每周跟进",
        playbookId: "playbook-1",
      }),
    );
    // customer_stage/intent_level 由 AI 派生，前端只读，不应出现在 body
    const body = (api.put as any).mock.calls[0][1] as Record<string, unknown>;
    expect(body).not.toHaveProperty("customerStage");
    expect(body).not.toHaveProperty("intentLevel");
  });
});

describe("userOpsStore.setMemoryDraft", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("以 patch 形式增量合并 memoryDraft，不覆盖其它字段", () => {
    useUserOpsStore.getState().setMemoryDraft({ identity: "工程师" });
    useUserOpsStore.getState().setMemoryDraft({ nextGoal: "确认需求" });
    const draft = useUserOpsStore.getState().memoryDraft;
    expect(draft.identity).toBe("工程师");
    expect(draft.nextGoal).toBe("确认需求");
  });
});

describe("userOpsStore.loadMessages memoryDraft 回填", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUserOpsStore.setState({
      messages: [], operatingMemory: null, memoryCandidates: [],
      decisionReviews: [], operationHealth: null,
    } as any);
  });

  it("从 operatingMemory 的四个 Document 拆回扁平 memoryDraft", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url.includes("/operating-memory"))
        return Promise.resolve({
          item: {
            id: "om",
            userUnderstanding: { identity: "工程师" },
            relationshipState: { relationshipGoal: "长期信任" },
            productFit: { interestedProducts: "A产品" },
            nextAction: { nextGoal: "确认需求" },
          },
        });
      if (url.includes("/messages")) return Promise.resolve({ items: [] });
      if (url.includes("/memory-candidates")) return Promise.resolve({ items: [] });
      if (url.includes("/decision-reviews")) return Promise.resolve({ items: [] });
      if (url.includes("/operation-health")) return Promise.resolve({ ok: true });
      return Promise.reject(new Error("unexpected url " + url));
    });

    const selected = contact("C1");
    bindContact(selected);
    await useUserOpsStore.getState().loadMessages(selected);

    const draft = useUserOpsStore.getState().memoryDraft;
    expect(draft.identity).toBe("工程师");
    expect(draft.relationshipGoal).toBe("长期信任");
    expect(draft.interestedProducts).toBe("A产品");
    expect(draft.nextGoal).toBe("确认需求");
  });

  it("operatingMemory 为 null 时 memoryDraft 回落空表单", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url.includes("/operating-memory")) return Promise.reject(new Error("404"));
      if (url.includes("/messages")) return Promise.resolve({ items: [] });
      if (url.includes("/memory-candidates")) return Promise.resolve({ items: [] });
      if (url.includes("/decision-reviews")) return Promise.resolve({ items: [] });
      if (url.includes("/operation-health")) return Promise.resolve({ ok: true });
      return Promise.reject(new Error("unexpected url " + url));
    });
    const selected = contact("C1");
    bindContact(selected);
    useUserOpsStore.getState().setMemoryDraft({ identity: "脏数据" });

    await useUserOpsStore.getState().loadMessages(selected);

    expect(useUserOpsStore.getState().memoryDraft.identity).toBe("");
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

describe("userOpsStore Playbook account binding", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAccountStore.setState({
      accounts: [account("A"), account("B")],
      selectedAccountId: "A"
    });
    useUserOpsStore.setState({
      playbooks: [],
      playbookScopeAccountId: "",
      playbookRequestGeneration: 0,
      editingPlaybookId: "",
      editingPlaybookAccountId: "",
      editingPlaybookVersion: null
    });
  });

  it("drops a late A response after B becomes the selected account", async () => {
    const a = deferred<{ items: OperationPlaybook[] }>();
    const b = deferred<{ items: OperationPlaybook[] }>();
    (api.get as any).mockImplementation((url: string) =>
      url.includes("accountId=A") ? a.promise : b.promise
    );

    const loadA = useUserOpsStore.getState().loadPlaybooks("A");
    useAccountStore.setState({ selectedAccountId: "B" });
    const loadB = useUserOpsStore.getState().loadPlaybooks("B");
    b.resolve({ items: [playbook("pb-b", "B")] });
    await loadB;
    a.resolve({ items: [playbook("pb-a", "A")] });
    await loadA;

    expect(useUserOpsStore.getState().playbooks).toEqual([playbook("pb-b", "B")]);
    expect(useUserOpsStore.getState().playbookScopeAccountId).toBe("B");
  });

  it("clears A edit identity on account switch and sends no stale writes", async () => {
    useUserOpsStore.setState({
      playbooks: [playbook("pb-a", "A", 4)],
      playbookScopeAccountId: "A"
    });
    useUserOpsStore.getState().editPlaybook(playbook("pb-a", "A", 4));
    expect(useUserOpsStore.getState().editingPlaybookAccountId).toBe("A");

    useAccountStore.setState({ selectedAccountId: "B" });
    (api.get as any).mockResolvedValueOnce({ items: [playbook("pb-b", "B")] });
    await useUserOpsStore.getState().loadPlaybooks("B");
    await useUserOpsStore.getState().savePlaybook();
    await useUserOpsStore.getState().setDefaultPlaybook("pb-a");

    expect(useUserOpsStore.getState().editingPlaybookId).toBe("");
    expect(useUserOpsStore.getState().playbookDraft.name).toBe("");
    expect(api.put).not.toHaveBeenCalled();
    expect(api.post).not.toHaveBeenCalled();
  });
});

describe("userOpsStore.enableAgent playbook binding", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("freezes account identity and sends the selected playbook", async () => {
    const selected = { ...contact("c-enable", "A1"), agentStatus: "normal" as const };
    bindContact(selected);
    useUserOpsStore.setState({
      profileNote: "known customer",
      selectedPlaybookId: "playbook-enable",
    });

    await useUserOpsStore.getState().enableAgent();

    expect(api.post).toHaveBeenCalledWith("/api/contacts/c-enable/enable-agent", {
      expectedAccountId: "A1",
      humanProfileNote: "known customer",
      playbookId: "playbook-enable",
    });
    expect(useUserOpsStore.getState().guideBusy).toBe(false);
  });

  it("resets guideBusy when enabling fails", async () => {
    const selected = { ...contact("c-enable-fail", "A1"), agentStatus: "normal" as const };
    bindContact(selected);
    (api.post as any).mockRejectedValueOnce(new Error("enable failed"));

    await useUserOpsStore.getState().enableAgent();

    expect(useUserOpsStore.getState().guideBusy).toBe(false);
  });
});

describe("userOpsStore.saveOperationDomain", () => {
  const domainDraft: OperationDomainDraft = {
    name: "用户运营",
    goal: "goal",
    methodology: "method",
    workflow: "workflow",
    toolPolicy: "tools",
    automationPolicy: "automation",
    reviewPolicy: "review",
    runtimeParameters: [
      "maxDailyTouches = 3",
      "quietHoursEnabled = false",
      "quietHoursStart = 23",
      "quietHoursEnd = 7",
      "quietHoursTzOffsetHours = 8"
    ].join("\n"),
    stateMachine: "",
    assistModeEnabled: false
  };

  beforeEach(() => {
    vi.clearAllMocks();
    useUserOpsStore.setState({
      operationDomains: [],
      domainDrafts: { user_operations: domainDraft }
    });
    (api.get as any).mockResolvedValue({ items: [] });
  });

  it("提交完整作息参数并返回成功状态", async () => {
    const saved = await useUserOpsStore.getState().saveOperationDomain("user_operations");

    expect(saved).toBe(true);
    expect(api.put).toHaveBeenCalledWith(
      "/api/operation-domains/user_operations",
      expect.objectContaining({
        runtimeParameters: expect.objectContaining({
          maxDailyTouches: 3,
          quietHoursEnabled: false,
          quietHoursStart: 23,
          quietHoursEnd: 7,
          quietHoursTzOffsetHours: 8
        })
      })
    );
  });

  it("写入失败时返回 false", async () => {
    (api.put as any).mockRejectedValueOnce(new Error("save failed"));

    await expect(
      useUserOpsStore.getState().saveOperationDomain("user_operations")
    ).resolves.toBe(false);
  });

  it("可直接提交弹窗草稿，不依赖先写入全局 domainDrafts", async () => {
    const override = {
      ...domainDraft,
      runtimeParameters: domainDraft.runtimeParameters.replace(
        "quietHoursEnd = 7",
        "quietHoursEnd = 9"
      )
    };

    const saved = await useUserOpsStore.getState().saveOperationDomain(
      "user_operations",
      override
    );

    expect(saved).toBe(true);
    expect(api.put).toHaveBeenCalledWith(
      "/api/operation-domains/user_operations",
      expect.objectContaining({
        runtimeParameters: expect.objectContaining({ quietHoursEnd: 9 })
      })
    );
  });
});
