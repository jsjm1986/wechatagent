import { useEffect } from "react";
import { OperationsView } from "../../App";
import { useOperationsStore } from "../../stores/operationsStore";
import { useAccountStore } from "../../stores/accountStore";

function formatTime(value?: string) {
  if (!value) return "-";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}

export default function OperationsFeature() {
  const {
    events,
    tasks,
    decisionReviews,
    llmUsage,
    opsTab,
    setOpsTab,
    loadOperationsData
  } = useOperationsStore();

  const { currentAccountId } = useAccountStore();

  useEffect(() => {
    loadOperationsData(currentAccountId());
  }, [loadOperationsData, currentAccountId]);

  return (
    <OperationsView
      events={events}
      tasks={tasks}
      decisionReviews={decisionReviews}
      llmUsage={llmUsage}
      opsTab={opsTab}
      onOpsTab={setOpsTab}
    />
  );
}