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
  { value: "brand_voice", label: "品牌语气" }
];

// kind → 中文标签；查不到回退原值（兼容 moment_media 等老数据）。
function kindLabel(kind: string): string {
  const hit = KIND_OPTIONS.find((o) => o.value === kind);
  if (hit) return hit.label;
  if (kind === "moment_media") return "朋友圈素材";
  return kind;
}

// 禁语是安全红线，后端恒注入、无视 minInjectTier，故行内不显档位而显「恒注入」。
const FORBIDDEN_TIER_BADGE = "恒注入";

// 最低注入档 → 中文标签；缺失/未知按完整档（与后端 None=full 语义一致）。
function tierLabel(tier?: string): string {
  switch (tier) {
    case "lean":
      return "精简档";
    case "relational":
      return "关系档";
    default:
      return "完整档";
  }
}

// 素材类型 → 中文标签（与上传表单下拉同源：image/file/video）；未知回退「文件」。
function mediaTypeLabel(mediaType?: string): string {
  switch (mediaType) {
    case "image":
      return "图片";
    case "video":
      return "视频";
    default:
      return "文件";
  }
}

const ACCEPT =
  "image/*,application/pdf,.doc,.docx,.xls,.xlsx,.ppt,.pptx,video/mp4";

function formatSize(bytes?: number): string {
  if (!bytes || bytes <= 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const EMPTY_ASSET_DRAFT = {
  kind: "text",
  title: "",
  body: "",
  usageScene: "",
  minInjectTier: "full",
};

export default function ContentAssetsFeature() {
  const currentAccountId = useAccountStore((s) => s.currentAccountId());
  return <ContentAssetsWorkbench key={currentAccountId} currentAccountId={currentAccountId} />;
}

function ContentAssetsWorkbench({ currentAccountId }: { currentAccountId: string }) {
  const busy = useUiStore((s) => s.busy);

  const {
    assets,
    assetsAccountId,
    assetDraft,
    assetDraftAccountId,
    setAssetDraft,
    loadAssets,
    createAsset,
    uploadMediaAsset,
    reviewMediaAsset,
    editAssetMeta,
    replaceAssetFile,
    toggleAssetSendable,
    deleteAsset
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
  const [tags, setTags] = useState("");
  const [needsApproval, setNeedsApproval] = useState(false);

  // 列表按标签筛选
  const [filterTag, setFilterTag] = useState("");

  const scopedDraft = assetDraftAccountId === currentAccountId
    ? assetDraft
    : EMPTY_ASSET_DRAFT;
  const scopedAssets = assetsAccountId === currentAccountId
    ? assets.filter((asset) => !asset.accountId || asset.accountId === currentAccountId)
    : [];

  useEffect(() => {
    useUiStore.getState().setBusy(false);
    useUiStore.getState().setError("");
    setAssetDraft(currentAccountId, EMPTY_ASSET_DRAFT);
    void loadAssets(currentAccountId);
  }, [currentAccountId, loadAssets, setAssetDraft]);

  const handleFilter = (event: FormEvent) => {
    event.preventDefault();
    void loadAssets(currentAccountId, filterTag.trim() || undefined);
  };

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
    fd.append("tags", tags);
    fd.append("requiresPrincipalApproval", String(needsApproval));
    if (currentAccountId) fd.append("accountId", currentAccountId);
    const ok = await uploadMediaAsset(fd, currentAccountId);
    if (ok) {
      setFile(null);
      setMediaTitle("");
      setTriggerHint("");
      setStages("");
      setTags("");
      setNeedsApproval(false);
    }
  };

  const mediaAssets = scopedAssets.filter((a) => a.kind === "media");
  const textAssets = scopedAssets.filter((a) => a.kind !== "media");

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

          <form className={styles.field} style={{ marginBottom: 14 }} onSubmit={handleFilter}>
            <span className={styles.fieldLabel}>按标签筛选素材</span>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                className={styles.input}
                placeholder="输入标签后回车，例如：报价类"
                value={filterTag}
                onChange={(event) => setFilterTag(event.target.value)}
              />
              <button className={styles.reviewBtn} type="submit" disabled={busy} style={{ marginTop: 0, flexShrink: 0 }}>
                筛选
              </button>
              {filterTag && (
                <button
                  className={styles.reviewBtn}
                  type="button"
                  disabled={busy}
                  style={{ marginTop: 0, flexShrink: 0 }}
                  onClick={() => {
                    setFilterTag("");
                    void loadAssets(currentAccountId);
                  }}
                >
                  清除
                </button>
              )}
            </div>
          </form>

          {scopedAssets.length === 0 ? (
            <EmptyState title="暂无内容资产" hint="在右侧新增文本、FAQ、话术或品牌语气，供 Agent 自主运营调用。" />
          ) : (
            <>
              {textAssets.length > 0 && (
                <div className={styles.list}>
                  {textAssets.map((asset) => (
                    <TextAssetRow
                      key={asset.id}
                      asset={asset}
                      busy={busy}
                      onEditMeta={(fields) =>
                        void editAssetMeta(asset, fields, currentAccountId)
                      }
                      onDelete={() => void deleteAsset(asset, currentAccountId)}
                    />
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
                          void reviewMediaAsset(asset, "approved", undefined, currentAccountId)
                        }
                        onToggleSendable={(sendable) =>
                          void toggleAssetSendable(asset, sendable, currentAccountId)
                        }
                        onDelete={() => void deleteAsset(asset, currentAccountId)}
                        onEditMeta={(fields) =>
                          void editAssetMeta(asset, fields, currentAccountId)
                        }
                        onReplaceFile={(form) =>
                          void replaceAssetFile(asset, form, currentAccountId)
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
                  value={scopedDraft.kind}
                  onChange={(event) => {
                    const kind = event.target.value;
                    // 切到禁用表达时注入档字段被隐藏（后端恒注入、无视 minInjectTier），
                    // 同步把 draft 值归位默认档，避免残留上次所选档位落库（死字段、不整洁）。
                    setAssetDraft(
                      currentAccountId,
                      kind === "forbidden_expression"
                        ? { ...scopedDraft, kind, minInjectTier: "full" }
                        : { ...scopedDraft, kind }
                    );
                  }}
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
                  value={scopedDraft.title}
                  onChange={(event) => setAssetDraft(currentAccountId, { ...scopedDraft, title: event.target.value })}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>正文</span>
                <textarea
                  className={styles.textarea}
                  value={scopedDraft.body}
                  onChange={(event) => setAssetDraft(currentAccountId, { ...scopedDraft, body: event.target.value })}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>使用场景</span>
                <input
                  className={styles.input}
                  value={scopedDraft.usageScene}
                  onChange={(event) => setAssetDraft(currentAccountId, { ...scopedDraft, usageScene: event.target.value })}
                />
              </label>
              {scopedDraft.kind !== "forbidden_expression" && (
                <label className={styles.field}>
                  <span className={styles.fieldLabel}>最低注入档</span>
                  <select
                    className={styles.select}
                    value={scopedDraft.minInjectTier}
                    onChange={(event) => setAssetDraft(currentAccountId, { ...scopedDraft, minInjectTier: event.target.value })}
                  >
                    <option value="lean">精简档（任何对话都注入，最常生效）</option>
                    <option value="relational">关系档（进入关系经营时注入）</option>
                    <option value="full">完整档（仅深入业务时注入）</option>
                  </select>
                  <span className={styles.hint}>核心禁语/口吻选精简档时刻生效；重型话术/长 FAQ 选完整档。</span>
                </label>
              )}
              <button className={styles.submit} type="submit" disabled={busy || !scopedDraft.title.trim()}>
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
                <span className={styles.fieldLabel}>目标阶段（逗号分隔；取值需在运营域配置阶段字典）</span>
                <input
                  className={styles.input}
                  placeholder="多个阶段用逗号分隔"
                  value={stages}
                  onChange={(event) => setStages(event.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>标签（逗号分隔）</span>
                <input
                  className={styles.input}
                  placeholder="例如：报价类,价格"
                  value={tags}
                  onChange={(event) => setTags(event.target.value)}
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

function TextAssetRow({
  asset,
  busy,
  onEditMeta,
  onDelete
}: {
  asset: ContentAsset;
  busy: boolean;
  onEditMeta: (fields: Record<string, unknown>) => void;
  onDelete: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState(asset.title);
  const [editBody, setEditBody] = useState(asset.body ?? "");
  const [editUsageScene, setEditUsageScene] = useState(asset.usageScene ?? "");
  const [editTier, setEditTier] = useState(asset.minInjectTier ?? "full");

  const openEdit = () => {
    setEditTitle(asset.title);
    setEditBody(asset.body ?? "");
    setEditUsageScene(asset.usageScene ?? "");
    setEditTier(asset.minInjectTier ?? "full");
    setEditing(true);
  };

  const handleSaveMeta = () => {
    onEditMeta({
      title: editTitle.trim(),
      body: editBody,
      usageScene: editUsageScene,
      minInjectTier: editTier
    });
    setEditing(false);
  };

  const handleDelete = () => {
    if (window.confirm("确认删除该资产？此操作不可撤销。")) {
      onDelete();
    }
  };

  return (
    <div className={styles.row}>
      <div className={styles.rowHead}>
        <strong className={styles.rowTitle}>{asset.title}</strong>
        <span style={{ display: "flex", gap: 6, flexShrink: 0 }}>
          <span className={styles.kind}>
            {asset.accountId ? `账号专属 · ${asset.accountId}` : "全账号共享"}
          </span>
          <span className={styles.kind}>{kindLabel(asset.kind)}</span>
          <span className={styles.kind}>
            {asset.kind === "forbidden_expression" ? FORBIDDEN_TIER_BADGE : tierLabel(asset.minInjectTier)}
          </span>
        </span>
      </div>
      <p className={styles.body}>{asset.body || asset.usageScene || "暂无内容"}</p>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8, marginTop: 8 }}>
        <button
          className={styles.reviewBtn}
          type="button"
          disabled={busy}
          onClick={() => (editing ? setEditing(false) : openEdit())}
          style={{ marginTop: 0 }}
        >
          编辑
        </button>
        <button
          className={styles.reviewBtn}
          type="button"
          disabled={busy}
          onClick={handleDelete}
          style={{ marginTop: 0 }}
        >
          删除
        </button>
      </div>

      {editing && (
        <div className={styles.form} style={{ marginTop: 12 }}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>标题</span>
            <input
              className={styles.input}
              value={editTitle}
              onChange={(event) => setEditTitle(event.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>正文</span>
            <textarea
              className={styles.textarea}
              value={editBody}
              onChange={(event) => setEditBody(event.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>使用场景</span>
            <input
              className={styles.input}
              value={editUsageScene}
              onChange={(event) => setEditUsageScene(event.target.value)}
            />
          </label>
          {asset.kind !== "forbidden_expression" && (
            <label className={styles.field}>
              <span className={styles.fieldLabel}>最低注入档</span>
              <select
                className={styles.select}
                value={editTier}
                onChange={(event) => setEditTier(event.target.value)}
              >
                <option value="lean">精简档（任何对话都注入，最常生效）</option>
                <option value="relational">关系档（进入关系经营时注入）</option>
                <option value="full">完整档（仅深入业务时注入）</option>
              </select>
            </label>
          )}
          <button
            className={styles.reviewBtn}
            type="button"
            disabled={busy || !editTitle.trim()}
            onClick={handleSaveMeta}
            style={{ marginTop: 0 }}
          >
            保存
          </button>
        </div>
      )}
    </div>
  );
}

function MediaAssetRow({
  asset,
  busy,
  onApprove,
  onToggleSendable,
  onDelete,
  onEditMeta,
  onReplaceFile
}: {
  asset: ContentAsset;
  busy: boolean;
  onApprove: () => void;
  onToggleSendable: (sendable: boolean) => void;
  onDelete: () => void;
  onEditMeta: (fields: Record<string, unknown>) => void;
  onReplaceFile: (form: FormData) => void;
}) {
  const isApproved = asset.reviewStatus === "approved";
  // 旧数据 / 缺省 sendable 视为可发送（true）
  const isSendable = asset.sendable !== false;
  const Icon = asset.mediaType === "image" ? ImageIcon : asset.mediaType === "video" ? Film : FileText;

  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState(asset.title);
  const [editHint, setEditHint] = useState(asset.sendTriggerHint ?? "");
  const [editStages, setEditStages] = useState((asset.targetStages ?? []).join(","));
  const [editTags, setEditTags] = useState((asset.tags ?? []).join(","));
  const [editFile, setEditFile] = useState<File | null>(null);
  const [editMediaType, setEditMediaType] = useState<"image" | "file" | "video">(
    asset.mediaType ?? "file"
  );

  const openEdit = () => {
    setEditTitle(asset.title);
    setEditHint(asset.sendTriggerHint ?? "");
    setEditStages((asset.targetStages ?? []).join(","));
    setEditTags((asset.tags ?? []).join(","));
    setEditMediaType(asset.mediaType ?? "file");
    setEditFile(null);
    setEditing(true);
  };

  const handleSaveMeta = () => {
    const targetStages = editStages
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const tags = editTags
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    onEditMeta({
      title: editTitle.trim(),
      sendTriggerHint: editHint,
      targetStages,
      tags
    });
    setEditing(false);
  };

  const handleReplace = () => {
    if (!editFile) return;
    const fd = new FormData();
    fd.append("file", editFile);
    fd.append("mediaType", editMediaType);
    onReplaceFile(fd);
    setEditFile(null);
  };

  const handleDelete = () => {
    if (window.confirm("确认删除该素材？此操作不可撤销。")) {
      onDelete();
    }
  };

  return (
    <div className={styles.row}>
      <div className={styles.mediaRow}>
        <span className={styles.fileCard}>
          <Icon size={20} />
        </span>
        <div className={styles.mediaMeta}>
          <div className={styles.rowHead}>
            <strong className={styles.rowTitle}>{asset.title}</strong>
            <span style={{ display: "flex", gap: 6, flexShrink: 0 }}>
              <span className={styles.kind}>
                {asset.accountId ? `账号专属 · ${asset.accountId}` : "全账号共享"}
              </span>
              <span className={`${styles.badge} ${isApproved ? styles.badgeApproved : styles.badgeDraft}`}>
                {isApproved ? "可发送" : "草稿待审"}
              </span>
            </span>
          </div>
          <p className={styles.metaLine}>
            {asset.fileName || "未命名文件"} · {mediaTypeLabel(asset.mediaType)} · {formatSize(asset.fileSize)}
          </p>
          {asset.sendTriggerHint && (
            <p className={styles.metaLine}>时机：{asset.sendTriggerHint}</p>
          )}
          {(asset.tags?.length ?? 0) > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 6 }}>
              {asset.tags!.map((tag) => (
                <span key={tag} className={styles.kind}>{tag}</span>
              ))}
            </div>
          )}
          <div style={{ display: "flex", flexWrap: "wrap", gap: 8, marginTop: 8 }}>
            {!isApproved && (
              <button className={styles.reviewBtn} type="button" disabled={busy} onClick={onApprove} style={{ marginTop: 0 }}>
                标记为可发送
              </button>
            )}
            <button
              className={styles.reviewBtn}
              type="button"
              disabled={busy}
              onClick={() => onToggleSendable(!isSendable)}
              style={{ marginTop: 0 }}
            >
              {isSendable ? "停用" : "启用"}
            </button>
            <button
              className={styles.reviewBtn}
              type="button"
              disabled={busy}
              onClick={() => (editing ? setEditing(false) : openEdit())}
              style={{ marginTop: 0 }}
            >
              编辑
            </button>
            <button
              className={styles.reviewBtn}
              type="button"
              disabled={busy}
              onClick={handleDelete}
              style={{ marginTop: 0 }}
            >
              删除
            </button>
          </div>

          {editing && (
            <div className={styles.form} style={{ marginTop: 12 }}>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>标题</span>
                <input
                  className={styles.input}
                  value={editTitle}
                  onChange={(event) => setEditTitle(event.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>发送时机提示</span>
                <textarea
                  className={styles.textarea}
                  value={editHint}
                  onChange={(event) => setEditHint(event.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>目标阶段（逗号分隔；取值需在运营域配置阶段字典）</span>
                <input
                  className={styles.input}
                  placeholder="多个阶段用逗号分隔"
                  value={editStages}
                  onChange={(event) => setEditStages(event.target.value)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>标签（逗号分隔）</span>
                <input
                  className={styles.input}
                  placeholder="例如：报价类,价格"
                  value={editTags}
                  onChange={(event) => setEditTags(event.target.value)}
                />
              </label>
              <button
                className={styles.reviewBtn}
                type="button"
                disabled={busy || !editTitle.trim()}
                onClick={handleSaveMeta}
                style={{ marginTop: 0 }}
              >
                保存
              </button>

              <label className={styles.field}>
                <span className={styles.fieldLabel}>换文件</span>
                <input
                  className={styles.fileInput}
                  type="file"
                  accept={ACCEPT}
                  onChange={(event) => setEditFile(event.target.files?.[0] ?? null)}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>素材类型</span>
                <select
                  className={styles.select}
                  value={editMediaType}
                  onChange={(event) => setEditMediaType(event.target.value as typeof editMediaType)}
                >
                  <option value="image">图片</option>
                  <option value="file">文件</option>
                  <option value="video">视频</option>
                </select>
              </label>
              <button
                className={styles.reviewBtn}
                type="button"
                disabled={busy || !editFile}
                onClick={handleReplace}
                style={{ marginTop: 0 }}
              >
                换文件
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
