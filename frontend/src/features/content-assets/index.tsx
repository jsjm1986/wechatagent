import { useEffect } from "react";
import { ContentAssetsView } from "../../App";
import { useAccountStore } from "../../stores/accountStore";
import { useUiStore } from "../../stores/uiStore";
import { useContentStore } from "../../stores/contentStore";

export default function ContentAssetsFeature() {
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const busy = useUiStore((s) => s.busy);

  const {
    assets,
    assetDraft,
    setAssetDraft,
    loadAssets,
    createAsset
  } = useContentStore();

  useEffect(() => {
    loadAssets(currentAccountId);
  }, [currentAccountId, loadAssets]);

  const handleCreateAsset = (event: React.FormEvent) => {
    event.preventDefault();
    void createAsset(currentAccountId);
  };

  return (
    <ContentAssetsView
      assets={assets}
      assetDraft={assetDraft}
      busy={busy}
      onAssetDraft={setAssetDraft}
      onCreateAsset={handleCreateAsset}
    />
  );
}