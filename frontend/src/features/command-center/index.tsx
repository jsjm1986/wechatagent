import { useEffect } from "react";
import { CommandCenterView } from "../../App";
import { useAccountStore } from "../../stores/accountStore";
import { useContactStore } from "../../stores/contactStore";
import { useCommandStore } from "../../stores/commandStore";

export default function CommandCenterFeature() {
  const accounts = useAccountStore((s) => s.accounts);
  const onlineCount = useAccountStore((s) => s.onlineCount());
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const currentAccount = useAccountStore((s) => s.currentAccount());

  const managedCount = useContactStore((s) => s.managedCount());

  const {
    commandDraft,
    commandResult,
    commandDryRun,
    commandBusy,
    souls,
    assets,
    pendingTasks,
    setCommandDraft,
    setCommandDryRun,
    loadCommandData,
    runCommand
  } = useCommandStore();

  useEffect(() => {
    loadCommandData(currentAccountId);
  }, [currentAccountId, loadCommandData]);

  const handleRunCommand = () => {
    if (currentAccountId) {
      runCommand(currentAccountId);
    }
  };

  return (
    <CommandCenterView
      accounts={accounts}
      assets={assets}
      commandDraft={commandDraft}
      commandBusy={commandBusy}
      commandResult={commandResult}
      commandDryRun={commandDryRun}
      setCommandDryRun={setCommandDryRun}
      currentAccount={currentAccount}
      managedCount={managedCount}
      onlineCount={onlineCount}
      onRunCommand={handleRunCommand}
      pendingTasks={pendingTasks}
      souls={souls}
      setCommandDraft={setCommandDraft}
    />
  );
}