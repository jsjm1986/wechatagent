import { QualityCenterView } from "../../App";
import { useAccountStore } from "../../stores/accountStore";

export default function QualityFeature() {
  const accountId = useAccountStore((s) => s.currentAccountId());
  return <QualityCenterView accountId={accountId} />;
}
