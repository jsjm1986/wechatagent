import { AutonomyLoopView } from "../../App";
import { useAccountStore } from "../../stores/accountStore";

export default function AutonomyFeature() {
  const accountId = useAccountStore((s) => s.currentAccountId());
  return <AutonomyLoopView accountId={accountId} />;
}
