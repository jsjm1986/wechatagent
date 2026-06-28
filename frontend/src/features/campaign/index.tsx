import { useCampaignStore } from "../../stores/campaignStore";
import CampaignList from "./CampaignList";
import CampaignCreate from "./CampaignCreate";
import CampaignBoard from "./CampaignBoard";

export default function CampaignFeature() {
  const view = useCampaignStore((s) => s.view);
  if (view === "create") return <CampaignCreate />;
  if (view === "board") return <CampaignBoard />;
  return <CampaignList />;
}
