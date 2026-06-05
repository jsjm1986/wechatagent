import { useEffect } from "react";
import {
  UserOperationCockpit,
  ContactsView,
  UserOpsModeHeader,
  UserPlaybookPanel,
  DomainPromptPanel,
  DomainConfigEditor,
  TraditionalOpsTabs
} from "./legacy";
import OperationsFeature from "../operations";
import { useUserOpsStore } from "../../stores/userOpsStore";
import { useStrategyStore } from "../../stores/strategyStore";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import { useUiStore } from "../../stores/uiStore";
import type {
  Contact,
  DomainKey,
  OperationDomainConfig,
  OperationDomainDraft
} from "../../types";
import { useMemo } from "react";

// 辅助函数
function emptyDomainDraft(): OperationDomainDraft {
  return {
    name: "",
    goal: "",
    methodology: "",
    workflow: "",
    toolPolicy: "",
    automationPolicy: "",
    reviewPolicy: "",
    runtimeParameters: "",
    stateMachine: ""
  };
}

function operationDomainByKey(configs: OperationDomainConfig[] | undefined, domain: string) {
  return configs?.find((config) => config.domain === domain);
}

export default function UserOpsFeature() {
  // Store状态
  const userOpsStore = useUserOpsStore();
  const contactStore = useContactStore();
  const accountStore = useAccountStore();
  const uiStore = useUiStore();

  // 从store中解构需要的状态
  const {
    userOpsMode,
    smartOpsTab,
    traditionalOpsTab,
    messages,
    operatingMemory,
    memoryCandidates,
    memoryDraft,
    operationHealth,
    decisionReviews,
    profileNote,
    customAgentInstructions,
    guideInstruction,
    guidePreview,
    simulationInput,
    simulationTurns,
    selectedPlaybookId,
    playbooks,
    playbookDraft,
    generatePlaybookText,
    optimizePlaybookText,
    editingPlaybookId,
    guideBusy,
    simulationBusy,
    // Domain 配置相关
    operationDomains,
    domainDrafts,
    // Actions
    setUserOpsMode,
    setSmartOpsTab,
    setTraditionalOpsTab,
    setProfileNote,
    setCustomAgentInstructions,
    setGuideInstruction,
    setSimulationInput,
    setSelectedPlaybookId,
    setPlaybookDraft,
    setGeneratePlaybookText,
    setOptimizePlaybookText,
    setDomainDrafts,
    hydrateSelected,
    loadMessages,
    loadPlaybooks,
    loadContacts,
    loadDomains,
    // 15个业务回调
    enableAgent,
    disableAgent,
    saveProfileNote,
    saveCustomAgentInstructions,
    analyzeProfile,
    previewGuideInstruction,
    applyGuidePreview,
    runMemoryConsolidation,
    runDialogueSimulation,
    createPlaybook,
    savePlaybook,
    optimizePlaybook,
    generatePlaybook,
    setDefaultPlaybook,
    editPlaybook,
    newPlaybookDraft,
    // Domain 配置业务方法
    saveOperationDomain,
    resetOperationDomain
  } = userOpsStore;

  const { contacts, selected, contactTab, setSelected, setContactTab } = contactStore;
  const { accounts, onlineCount } = accountStore;
  // 订阅派生的原始 accountId 字符串（切账号时值变 → effect 重拉），
  // 不要解构 currentAccountId 函数引用——它恒稳定，依赖它的 effect 永不触发。
  const effectiveAccountId = useAccountStore((s) =>
    s.accounts.some((a) => a.accountId === s.selectedAccountId)
      ? s.selectedAccountId
      : s.accounts[0]?.accountId ?? ""
  );
  const { busy, error, setBusy, setError } = uiStore;

  // 传统模式 prompts tab：复用 strategyStore 的 souls/prompt CRUD（与系统策略页同一套）
  const strategyStore = useStrategyStore();
  const {
    souls,
    promptTemplates,
    soulDraft,
    editingSoulId,
    promptDraft,
    editingPromptId,
    setSoulDraft,
    setPromptDraft,
    loadStrategyData,
    createSoul,
    saveSoul,
    publishSoul,
    editSoul,
    newSoulDraftFor,
    createPromptTemplate,
    savePromptTemplate,
    publishPromptTemplate,
    editPromptTemplate,
    newPromptDraftFor
  } = strategyStore;

  // 计算衍生状态
  const managedCount = useMemo(
    () => contacts.filter((contact) => contact.agentStatus === "managed").length,
    [contacts]
  );
  const normalCount = contacts.length - managedCount;

  const filteredContacts = useMemo(() => {
    if (contactTab === "managed") return contacts.filter((contact) => contact.agentStatus === "managed");
    if (contactTab === "normal") return contacts.filter((contact) => contact.agentStatus === "normal");
    return contacts;
  }, [contacts, contactTab]);

  // 占位数据（这些需要从适当的地方获取）
  const importQuery = "";
  const setImportQuery = () => {};
  const query = "";
  const setQuery = () => {};

  // 待办计数徽标——真实运营数据现由自包含 OperationsFeature/operationsStore 负责加载，
  // 这里不再用占位 tasks 反推；徽标后续可订阅 operationsStore.pending 派生。
  const pendingTasks = 0;

  // 占位函数
  const importContacts = async () => {};
  const loadAll = async () => {};

  const openContact = async (contact: Contact) => {
    setSelected(contact);
    hydrateSelected(contact);
    await loadMessages(contact);
  };

  // 挂载 + 切账号时加载主体数据（联系人列表 + 剧本），依赖 effectiveAccountId 原始值
  useEffect(() => {
    if (effectiveAccountId) {
      setSelected(null); // 切账号清掉上个账号选中的联系人，避免串号
      void loadContacts(effectiveAccountId);
      void loadPlaybooks(effectiveAccountId);
    }
  }, [effectiveAccountId, loadContacts, loadPlaybooks, setSelected]);

  // 切到 traditional 模式时加载 souls/promptTemplates（prompts tab 复用 strategyStore）和 domains
  useEffect(() => {
    if (userOpsMode === "traditional") {
      void loadStrategyData();
      void loadDomains();
    }
  }, [userOpsMode, loadStrategyData, loadDomains]);

  // 选中联系人变化时的处理
  useEffect(() => {
    if (selected) {
      hydrateSelected(selected);
      loadMessages(selected);
    }
  }, [selected, hydrateSelected, loadMessages]);

  return (
    <section className="userOpsWorkspace">
      <UserOpsModeHeader mode={userOpsMode} onMode={setUserOpsMode} />

      {userOpsMode === "smart" && (
        <section className="userCockpitGrid">
          <ContactsView
            busy={busy}
            contactTab={contactTab}
            contacts={filteredContacts}
            importQuery={importQuery}
            query={query}
            totalCount={contacts.length}
            managedCount={managedCount}
            normalCount={normalCount}
            selected={selected}
            onContactTab={setContactTab}
            onImport={importContacts}
            onImportQuery={setImportQuery}
            onLoadAll={loadAll}
            onOpenContact={openContact}
            onQuery={setQuery}
          />
          <UserOperationCockpit
            activeTab={smartOpsTab}
            busy={busy}
            decisionReviews={decisionReviews}
            guideBusy={guideBusy}
            guideInstruction={guideInstruction}
            guidePreview={guidePreview}
            health={operationHealth}
            memoryCandidates={memoryCandidates}
            memoryDraft={memoryDraft}
            messages={messages}
            operatingMemory={operatingMemory}
            playbooks={playbooks}
            profileNote={profileNote}
            customAgentInstructions={customAgentInstructions}
            selected={selected}
            selectedPlaybookId={selectedPlaybookId}
            simulationBusy={simulationBusy}
            simulationInput={simulationInput}
            simulationTurns={simulationTurns}
            onAnalyzeProfile={analyzeProfile}
            onApplyGuidePreview={applyGuidePreview}
            onDisableAgent={disableAgent}
            onEnableAgent={enableAgent}
            onGuideInstruction={setGuideInstruction}
            onPreviewGuide={previewGuideInstruction}
            onProfileNote={setProfileNote}
            onCustomAgentInstructions={setCustomAgentInstructions}
            onRunMemoryConsolidation={runMemoryConsolidation}
            onRunSimulation={runDialogueSimulation}
            onSaveProfileNote={saveProfileNote}
            onSaveCustomAgentInstructions={saveCustomAgentInstructions}
            onSelectedPlaybook={setSelectedPlaybookId}
            onSimulationInput={setSimulationInput}
            onTab={setSmartOpsTab}
          />
        </section>
      )}

      {userOpsMode === "traditional" && (
        <>
          <TraditionalOpsTabs
            active={traditionalOpsTab}
            managedCount={managedCount}
            pendingTasks={pendingTasks}
            onChange={setTraditionalOpsTab}
          />

          {traditionalOpsTab === "playbooks" && (
            <UserPlaybookPanel
              busy={busy}
              editingPlaybookId={editingPlaybookId}
              generatePlaybookText={generatePlaybookText}
              optimizePlaybookText={optimizePlaybookText}
              playbookDraft={playbookDraft}
              playbooks={playbooks}
              onCreatePlaybook={(e) => { e.preventDefault(); void createPlaybook(); }}
              onEditPlaybook={editPlaybook}
              onGeneratePlaybook={(e) => { e.preventDefault(); void generatePlaybook(); }}
              onGeneratePlaybookText={setGeneratePlaybookText}
              onNewPlaybook={newPlaybookDraft}
              onOptimizePlaybook={() => optimizePlaybook(editingPlaybookId)}
              onOptimizePlaybookText={setOptimizePlaybookText}
              onPlaybookDraft={setPlaybookDraft}
              onSavePlaybook={(e) => { e.preventDefault(); void savePlaybook(); }}
              onSetDefaultPlaybook={setDefaultPlaybook}
            />
          )}

          {traditionalOpsTab === "prompts" && (
            <DomainPromptPanel
              agentKinds={["user"]}
              busy={busy}
              defaultAgentKind="user"
              editingPromptId={editingPromptId}
              editingSoulId={editingSoulId}
              lockAgentKind
              promptDraft={promptDraft}
              promptTemplates={promptTemplates}
              soulDraft={soulDraft}
              souls={souls}
              title="用户运营 Agent 提示词"
              onCreatePromptTemplate={(e) => { e.preventDefault(); void createPromptTemplate(); }}
              onCreateSoul={(e) => { e.preventDefault(); void createSoul(); }}
              onEditPromptTemplate={editPromptTemplate}
              onEditSoul={editSoul}
              onNewPromptTemplate={() => newPromptDraftFor("user")}
              onNewSoul={() => newSoulDraftFor("user")}
              onPromptDraft={setPromptDraft}
              onPublishPromptTemplate={publishPromptTemplate}
              onPublishSoul={publishSoul}
              onSavePromptTemplate={(e) => { e.preventDefault(); void savePromptTemplate(); }}
              onSaveSoul={(e) => { e.preventDefault(); void saveSoul(); }}
              onSoulDraft={setSoulDraft}
            />
          )}

          {traditionalOpsTab === "settings" && (
            <DomainConfigEditor
              busy={busy}
              config={operationDomainByKey(operationDomains, "user_operations")}
              draft={domainDrafts?.user_operations ?? emptyDomainDraft()}
              mode="primary"
              onDraft={(draft) => setDomainDrafts({ ...(domainDrafts || {}), user_operations: draft })}
              onReset={() => void resetOperationDomain("user_operations")}
              onSave={() => void saveOperationDomain("user_operations")}
              onAfterVersionAction={loadDomains}
            />
          )}

          {traditionalOpsTab === "audit" && <OperationsFeature />}
        </>
      )}
    </section>
  );
}