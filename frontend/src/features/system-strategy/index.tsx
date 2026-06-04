import { useEffect } from "react";
import { SystemStrategyView } from "../../App";
import { useUiStore } from "../../stores/uiStore";
import { useStrategyStore } from "../../stores/strategyStore";

export default function SystemStrategyFeature() {
  const busy = useUiStore((s) => s.busy);

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
    createPromptTemplate,
    savePromptTemplate,
    publishPromptTemplate,
    resetSystemPromptPack,
    editSoul,
    newSoulDraftFor,
    editPromptTemplate,
    newPromptDraftFor
  } = useStrategyStore();

  useEffect(() => {
    void loadStrategyData();
  }, [loadStrategyData]);

  const handleCreateSoul = (e: React.FormEvent) => {
    e.preventDefault();
    void createSoul();
  };

  const handleSaveSoul = (e: React.FormEvent) => {
    e.preventDefault();
    void saveSoul();
  };

  const handleCreatePromptTemplate = (e: React.FormEvent) => {
    e.preventDefault();
    void createPromptTemplate();
  };

  const handleSavePromptTemplate = (e: React.FormEvent) => {
    e.preventDefault();
    void savePromptTemplate();
  };

  const handlePublishSoul = (id: string) => {
    void publishSoul(id);
  };

  const handlePublishPromptTemplate = (id: string) => {
    void publishPromptTemplate(id);
  };

  const handleResetPromptPack = () => {
    void resetSystemPromptPack();
  };

  const handleNewSoul = () => {
    newSoulDraftFor("management");
  };

  const handleNewPromptTemplate = () => {
    newPromptDraftFor("management");
  };

  return (
    <SystemStrategyView
      busy={busy}
      editingPromptId={editingPromptId}
      editingSoulId={editingSoulId}
      promptDraft={promptDraft}
      promptTemplates={promptTemplates}
      soulDraft={soulDraft}
      souls={souls}
      onCreatePromptTemplate={handleCreatePromptTemplate}
      onCreateSoul={handleCreateSoul}
      onEditPromptTemplate={editPromptTemplate}
      onEditSoul={editSoul}
      onNewPromptTemplate={handleNewPromptTemplate}
      onNewSoul={handleNewSoul}
      onPromptDraft={setPromptDraft}
      onPublishPromptTemplate={handlePublishPromptTemplate}
      onPublishSoul={handlePublishSoul}
      onResetPromptPack={handleResetPromptPack}
      onSavePromptTemplate={handleSavePromptTemplate}
      onSaveSoul={handleSaveSoul}
      onSoulDraft={setSoulDraft}
    />
  );
}