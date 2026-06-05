import { useEffect } from "react";
import type { FormEvent } from "react";
import { FileText } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { useAccountStore } from "../../stores/accountStore";
import { useUiStore } from "../../stores/uiStore";
import { useContentStore } from "../../stores/contentStore";
import styles from "./ContentAssets.module.css";

const KIND_OPTIONS: { value: string; label: string }[] = [
  { value: "text", label: "文本资料" },
  { value: "faq", label: "FAQ" },
  { value: "script", label: "话术" },
  { value: "forbidden_expression", label: "禁用表达" },
  { value: "brand_voice", label: "品牌语气" },
  { value: "moment_media", label: "朋友圈素材" }
];

export default function ContentAssetsFeature() {
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const busy = useUiStore((s) => s.busy);

  const { assets, assetDraft, setAssetDraft, loadAssets, createAsset } = useContentStore();

  useEffect(() => {
    loadAssets(currentAccountId);
  }, [currentAccountId, loadAssets]);

  const handleCreateAsset = (event: FormEvent) => {
    event.preventDefault();
    void createAsset(currentAccountId);
  };

  return (
    <div className={styles.page}>
      <div className={styles.workbench}>
        <section className={styles.panel}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>Content Assets</span>
              <span className={styles.title}>内容资产库</span>
            </div>
            <span className={styles.headIcon}><FileText size={17} /></span>
          </div>

          {assets.length === 0 ? (
            <EmptyState title="暂无内容资产" hint="在右侧新增文本、FAQ、话术或品牌语气，供 Agent 自主运营调用。" />
          ) : (
            <div className={styles.list}>
              {assets.map((asset) => (
                <div key={asset.id} className={styles.row}>
                  <div className={styles.rowHead}>
                    <strong className={styles.rowTitle}>{asset.title}</strong>
                    <span className={styles.kind}>{asset.kind}</span>
                  </div>
                  <p className={styles.body}>
                    {asset.body || asset.url || asset.mediaId || asset.usageScene || "暂无内容"}
                  </p>
                </div>
              ))}
            </div>
          )}
        </section>

        <form className={styles.panel} onSubmit={handleCreateAsset}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>新增</span>
              <span className={styles.title}>新增资产</span>
            </div>
          </div>

          <div className={styles.form}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>类型</span>
              <select
                className={styles.select}
                value={assetDraft.kind}
                onChange={(event) => setAssetDraft({ ...assetDraft, kind: event.target.value })}
              >
                {KIND_OPTIONS.map((opt) => (
                  <option key={opt.value} value={opt.value}>{opt.label}</option>
                ))}
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>标题</span>
              <input
                className={styles.input}
                value={assetDraft.title}
                onChange={(event) => setAssetDraft({ ...assetDraft, title: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>正文</span>
              <textarea
                className={styles.textarea}
                value={assetDraft.body}
                onChange={(event) => setAssetDraft({ ...assetDraft, body: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>素材 URL</span>
              <input
                className={styles.input}
                value={assetDraft.url}
                onChange={(event) => setAssetDraft({ ...assetDraft, url: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>MCP Media ID</span>
              <input
                className={styles.input}
                value={assetDraft.mediaId}
                onChange={(event) => setAssetDraft({ ...assetDraft, mediaId: event.target.value })}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>使用场景</span>
              <input
                className={styles.input}
                value={assetDraft.usageScene}
                onChange={(event) => setAssetDraft({ ...assetDraft, usageScene: event.target.value })}
              />
            </label>
            <button className={styles.submit} type="submit" disabled={busy || !assetDraft.title.trim()}>
              保存资产
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
