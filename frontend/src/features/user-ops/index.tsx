import { useEffect } from "react";
import {
  UserOperationCockpit,
  ContactsView,
  UserOpsModeHeader,
  UserPlaybookPanel,
  DomainPromptPanel,
  DomainConfigEditor,
  TraditionalOpsTabs,
  OperationsView
} from "../../App";
import { useUserOpsStore } from "../../stores/userOpsStore";
import { useStrategyStore } from "../../stores/strategyStore";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import { useUiStore } from "../../stores/uiStore";
import type {
  Contact
} from "../../types";
import { useMemo } from "react";

// 临时类型定义（这些应该最终移到types中）
type DomainKey = "user_operations" | "group_operations" | "moment_operations";

type OperationDomainConfig = {
  id: string;
  domain: DomainKey;
  name: string;
  goal: string;
  methodology: string;
  workflow: string;
  toolPolicy: string;
  automationPolicy: string;
  reviewPolicy: string;
  runtimeParameters: Record<string, unknown>;
  stateMachine: Record<string, unknown>;
  status: string;
  updatedAt?: string;
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  seededBy?: string | null;
};

type OperationDomainDraft = {
  name: string;
  goal: string;
  methodology: string;
  workflow: string;
  toolPolicy: string;
  automationPolicy: string;
  reviewPolicy: string;
  runtimeParameters: string;
  stateMachine: string;
};

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

function operationDomainByKey(configs: OperationDomainConfig[], domain: string) {
  return configs.find((config) => config.domain === domain);
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
    hydrateSelected,
    loadMessages,
    loadPlaybooks,
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
    newPlaybookDraft
  } = userOpsStore;

  const { contacts, selected, contactTab, setSelected, setContactTab } = contactStore;
  const { currentAccountId, accounts, onlineCount } = accountStore;
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
  const events: any[] = [];
  const tasks: any[] = [];
  const llmUsage: any = null;
  const opsTab = "tasks";
  const setOpsTab = () => {};
  const importQuery = "";
  const setImportQuery = () => {};
  const query = "";
  const setQuery = () => {};

  // 占位数据 - operationDomains（settings tab 的 domain 配置尚未迁移到独立 store）
  const operationDomains: OperationDomainConfig[] = [];
  const domainDrafts: Record<string, OperationDomainDraft> = {};
  const setDomainDrafts = (_drafts: Record<string, OperationDomainDraft>) => {};

  const pendingTasks = tasks.filter((task) => task.status === "pending").length;

  // 占位函数
  const importContacts = async () => {};
  const loadAll = async () => {};

  const openContact = async (contact: Contact) => {
    setSelected(contact);
    hydrateSelected(contact);
    await loadMessages(contact);
  };

  // settings tab 的 domain 配置回调（domain store 尚未迁移，暂保留无操作）
  const resetOperationDomain = async (_domain: string) => {};
  const saveOperationDomain = async (_domain: string) => {};

  // 挂载时加载剧本
  useEffect(() => {
    const accountId = currentAccountId();
    if (accountId) {
      loadPlaybooks(accountId);
    }
  }, [currentAccountId, loadPlaybooks]);

  // 切到 traditional 模式时加载 souls/promptTemplates（prompts tab 复用 strategyStore）
  useEffect(() => {
    if (userOpsMode === "traditional") {
      void loadStrategyData();
    }
  }, [userOpsMode, loadStrategyData]);

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
              draft={domainDrafts.user_operations ?? emptyDomainDraft()}
              mode="primary"
              onDraft={(draft) => setDomainDrafts({ ...domainDrafts, user_operations: draft })}
              onReset={() => void resetOperationDomain("user_operations")}
              onSave={() => void saveOperationDomain("user_operations")}
              onAfterVersionAction={loadAll}
            />
          )}

          {traditionalOpsTab === "audit" && (
            <OperationsView
              decisionReviews={decisionReviews}
              events={events}
              llmUsage={llmUsage}
              opsTab={opsTab}
              tasks={tasks}
              onOpsTab={setOpsTab}
            />
          )}
        </>
      )}
    </section>
  );
}