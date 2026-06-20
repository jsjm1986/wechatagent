import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { FileText, Image as ImageIcon, Film, Upload } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { useAccountStore } from "../../stores/accountStore";
import { useUiStore } from "../../stores/uiStore";
import { useContentStore } from "../../stores/contentStore";
import type { ContentAsset } from "../../types";
import styles from "./ContentAssets.module.css";

const KIND_OPTIONS: { value: string; label: string }[] = [
  { value: "text", label: "文本资料" },
  { value: "faq", label: "FAQ" },
  { value: "script", label: "话术" },
  { value: "forbidden_expression", label: "禁用表达" },
  { value: "brand_voice", label: "品牌语气" },
  { value: "moment_media", label: "朋友圈素材" }
];

const ACCEPT =
  "image/*,application/pdf,.doc,.docx,.xls,.xlsx,.ppt,.pptx,video/mp4";

function formatSize(bytes?: number): string {
  if (!bytes || bytes <= 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function ContentAssetsFeature() {
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  const busy = useUiStore((s) => s.busy);

  const {
    assets,
    assetDraft,
    setAssetDraft,
    loadAssets,
    createAsset,
    uploadMediaAsset,
    reviewMediaAsset
  } = useContentStore();

  // 素材上传表单本地态
  const [file, setFile] = useState<File | null>(null);
  const [mediaTitle, setMediaTitle] = useState("");
  const [mediaType, setMediaType] = useState<"image" | "file" | "video">("file");
  const [triggerHint, setTriggerHint] = useState("");
  const [expressionPref, setExpressionPref] = useState<"file_primary" | "file_support">(
    "file_primary"
  );
  const [stages, setStages] = useState("");
  const [needsApproval, setNeedsApproval] = useState(false);

  useEffect(() => {
    loadAssets(currentAccountId);
  }, [currentAccountId, loadAssets]);

  const handleCreateAsset = (event: FormEvent) => {
    event.preventDefault();
    void createAsset(currentAccountId);
  };

  const handleUpload = async (event: FormEvent) => {
    event.preventDefault();
    if (!file || !mediaTitle.trim()) return;
    const fd = new FormData();
    fd.append("file", file);
    fd.append("title", mediaTitle.trim());
    fd.append("mediaType", mediaType);
    fd.append("sendTriggerHint", triggerHint);
    fd.append("expressionPref", expressionPref);
    fd.append("targetStages", stages);
    fd.append("requiresPrincipalApproval", String(needsApproval));
    if (currentAccountId) fd.append("accountId", currentAccountId);
    const ok = await uploadMediaAsset(fd, currentAccountId);
    if (ok) {
      setFile(null);
      setMediaTitle("");
      setTriggerHint("");
      setStages("");
      setNeedsApproval(false);
    }
  };

  const mediaAssets = assets.filter((a) => a.kind === "media");
  const textAssets = assets.filter((a) => a.kind !== "media");

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
            <>
              {textAssets.length > 0 && (
                <div className={styles.list}>
                  {textAssets.map((asset) => (
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

              {mediaAssets.length > 0 && (
                <>
                  <p className={styles.sectionTitle} style={{ marginTop: 18 }}>
                    销售素材文件
                  </p>
                  <div className={styles.list}>
                    {mediaAssets.map((asset) => (
                      <MediaAssetRow
                        key={asset.id}
                        asset={asset}
                        busy={busy}
                        onApprove={() =>
                          void reviewMediaAsset(asset.id, "approved", undefined, currentAccountId)
                        }
                      />
                    ))}
                  </div>
                </>
              )}
            </>
          )}
        </section>

        <div style={{ display: "grid", gap: 18 }}>
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

          <form className={styles.panel} onSubmit={handleUpload}>
            <div className={styles.head}>
              <div className={styles.headL}>
                <span className={styles.eyebrow}>上传</span>
                <span className={styles.title}>上传销售素材</span>
              </div>
              <span className={styles.headIcon}><Upload size={17} /></span>
            </div>

            <p className={styles.hint}>
              提示：若知识库已有同内容文本，请确认两边信息一致（如价格、政策），避免素材与话术口径冲突。
            </p>

            <div className={styles.form}>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>文件（图片 / PDF / Office / MP4）</span>
                <input
                  className={styles.fileInput}
                  type="file"
                  accept={ACCEPT}
                  onChange={(event) => setFile(event.target.files?.[0] ?? null)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>素材标题</span>
                <input
                  className={styles.input}
                  value={mediaTitle}
                  onChange={(event) => setMediaTitle(event.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>素材类型</span>
                <select
                  className={styles.select}
                  value={mediaType}
                  onChange={(event) => setMediaType(event.target.value as typeof mediaType)}
                >
                  <option value="image">图片</option>
                  <option value="file">文件</option>
                  <option value="video">视频</option>
                </select>
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>发送时机提示</span>
                <textarea
                  className={styles.textarea}
                  placeholder="例如：客户问价格时发"
                  value={triggerHint}
                  onChange={(event) => setTriggerHint(event.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>表达偏好</span>
                <select
                  className={styles.select}
                  value={expressionPref}
                  onChange={(event) => setExpressionPref(event.target.value as typeof expressionPref)}
                >
                  <option value="file_primary">以文件为主</option>
                  <option value="file_support">文件为辅（话术为主）</option>
                </select>
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>目标阶段（逗号分隔）</span>
                <input
                  className={styles.input}
                  placeholder="例如：意向,未成交"
                  value={stages}
                  onChange={(event) => setStages(event.target.value)}
                />
              </label>
              <label className={styles.checkboxField}>
                <input
                  type="checkbox"
                  checked={needsApproval}
                  onChange={(event) => setNeedsApproval(event.target.checked)}
                />
                <span>发送前需领导审批</span>
              </label>
              <button
                className={styles.submit}
                type="submit"
                disabled={busy || !file || !mediaTitle.trim()}
              >
                上传（待审核）
              </button>
            </div>
          </form>
        </div>
      </div>
    </div>
  );
}

function MediaAssetRow({
  asset,
  busy,
  onApprove
}: {
  asset: ContentAsset;
  busy: boolean;
  onApprove: () => void;
}) {
  const isApproved = asset.reviewStatus === "approved";
  const Icon = asset.mediaType === "image" ? ImageIcon : asset.mediaType === "video" ? Film : FileText;
  return (
    <div className={styles.row}>
      <div className={styles.mediaRow}>
        <span className={styles.fileCard}>
          <Icon size={20} />
        </span>
        <div className={styles.mediaMeta}>
          <div className={styles.rowHead}>
            <strong className={styles.rowTitle}>{asset.title}</strong>
            <span className={`${styles.badge} ${isApproved ? styles.badgeApproved : styles.badgeDraft}`}>
              {isApproved ? "可发送" : "草稿待审"}
            </span>
          </div>
          <p className={styles.metaLine}>
            {asset.fileName || "未命名文件"} · {asset.mediaType || "file"} · {formatSize(asset.fileSize)}
          </p>
          {asset.sendTriggerHint && (
            <p className={styles.metaLine}>时机：{asset.sendTriggerHint}</p>
          )}
          {!isApproved && (
            <button className={styles.reviewBtn} type="button" disabled={busy} onClick={onApprove}>
              标记为可发送
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
