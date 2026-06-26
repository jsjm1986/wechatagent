import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { Settings2, Inbox } from "lucide-react";
import { api } from "../../lib/api";
import { useUiStore } from "../../stores/uiStore";
import { useStrategyStore } from "../../stores/strategyStore";
import type { AgentSoul, PromptTemplate, PromptTemplateDraft, DomainProfile, DomainProfileDraft } from "../../types";
import { ProfilePublishCard } from "../../components/review/ProfilePublishCard";
import { LessonPromoteCard } from "../../components/review/LessonPromoteCard";
import { ConfirmProvider } from "../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../components/ui/Toast";
import styles from "./SystemStrategy.module.css";

// 系统策略频道：全局总控 Prompt（人格/任务）+ 状态机灰度 + 双层标签字典 + 跨用户教训。
// 大页头（eyebrow/title/subtitle）由 Shell 依据 channels.ts 渲染；组件仅保留面板级小标题。
// 自包含：本频道独有的 View / 灰度面板 / 版本动作条 / 空态从 App.tsx 迁出并 CSS Module 化。

type ActiveVersionMeta = {
  id: string;
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  seededBy?: string | null;
  updatedAt?: string;
};

type OperationStatePolicyEntry = ActiveVersionMeta & {
  workspaceId?: string;
  domain: string;
  stateKey: string;
  allowed: string[];
  forbidden: string[];
  recommendedPace?: string | null;
  status: string;
};

type TaxonomyEntry = ActiveVersionMeta & {
  scope: string;
  kind: string;
  value: {
    id: string;
    label: string;
    displayName?: string;
    description?: string;
    aliases?: string[];
    status: string;
  };
};

type TaxonomyDraft = {
  scope: string;
  kind: string;
  id: string;
  label: string;
  aliases: string;
  description: string;
};

type EditDraft = { label: string; aliases: string; description: string };

type LessonLearnedEntry = {
  lessonId: string;
  workspaceId: string;
  patternKind: string; // "success" | "reviewer_misjudge_negative" | "blocked_by_safety_guard"
  count: number;
  sampleRunIds: string[];
  updatedAt: string;
  createdAt: string;
  reviewStatus: string; // 默认 "pending_review"
  promotedChunkId: string | null;
};

function agentKindLabel(kind: string) {
  const labels: Record<string, string> = {
    user: "用户运营",
    management: "后台管理",
    methodology: "方法论生成",
    group: "微信群运营",
    moment: "朋友圈运营",
  };
  return labels[kind] || kind;
}

function statusSortOrder(status: string): number {
  switch (status) {
    case "active":
    case "published":
      return 0;
    case "draft":
      return 1;
    case "archived":
      return 2;
    default:
      return 3;
  }
}

function Empty({ text }: { text: string }) {
  return (
    <div className={styles.empty}>
      <Inbox size={26} />
      <p>{text}</p>
    </div>
  );
}

// 资源无关的版本动作条：发布新版本 / 切到当前 / 回滚到上一版本。
function ActiveVersionsBar({
  meta,
  endpointPrefix,
  resourceLabel,
  busy,
  canPublish = false,
  onAfterAction,
}: {
  meta: ActiveVersionMeta | undefined;
  endpointPrefix: string;
  resourceLabel: string;
  busy: boolean;
  canPublish?: boolean;
  onAfterAction?: () => void | Promise<void>;
}) {
  const [actionBusy, setActionBusy] = useState(false);
  if (!meta || !meta.id) {
    return null;
  }
  const version = meta.version ?? 1;
  const isCurrent = meta.currentVersion !== false;
  const previousVersion = meta.previousVersion ?? null;
  const seededBy = meta.seededBy ?? null;

  async function runAction(action: "publish" | "rollout" | "rollback") {
    if (!meta || !meta.id) return;
    const confirmText =
      action === "publish"
        ? `确认发布 ${resourceLabel} 新版本（version=${version + 1}）？`
        : action === "rollout"
        ? `确认把 ${resourceLabel} v${version} 设为当前生效版本？`
        : `确认回滚 ${resourceLabel} 到上一版本（v${previousVersion ?? "?"}）？`;
    if (!window.confirm(confirmText)) return;
    setActionBusy(true);
    try {
      await api.post(`${endpointPrefix}/${meta.id}/${action}`, {});
      if (onAfterAction) await onAfterAction();
    } catch (error) {
      window.alert(`${resourceLabel} ${action} 失败：${(error as Error).message}`);
    } finally {
      setActionBusy(false);
    }
  }

  const disabled = busy || actionBusy;

  return (
    <div className={styles.activeVersionsBar}>
      <div className={styles.activeVersionsMeta}>
        <span className={isCurrent ? styles.activeVersionsBadgeCurrent : styles.activeVersionsBadgeShadow}>
          v{version}
          {isCurrent ? " · current" : " · shadow"}
        </span>
        {previousVersion !== null && (
          <span className={styles.activeVersionsChain} title="previous_version 回滚链">
            ← v{previousVersion}
          </span>
        )}
        {seededBy && (
          <span className={styles.activeVersionsSeeded} title="写入来源">
            {seededBy}
          </span>
        )}
        {meta.updatedAt && (
          <span className={styles.activeVersionsTimestamp} title="updated_at">
            {meta.updatedAt}
          </span>
        )}
      </div>
      <div className={styles.activeVersionsActions}>
        {canPublish && (
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() => void runAction("publish")}
            disabled={disabled}
            title="基于当前 row 发布新版本（version+1，previous_version 自动写入）"
          >
            发布新版本
          </button>
        )}
        {!isCurrent && (
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() => void runAction("rollout")}
            disabled={disabled}
            title="把这一版本切到当前生效（其他版本 soft demote）"
          >
            切到当前
          </button>
        )}
        {previousVersion !== null && isCurrent && (
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() => void runAction("rollback")}
            disabled={disabled}
            title="把上一版本重新激活到当前生效"
          >
            回滚到 v{previousVersion}
          </button>
        )}
      </div>
    </div>
  );
}

// 人格设定 + 任务提示词工作台。management/methodology 与 user 各自复用，按 agentKinds 过滤。
function DomainPromptPanel({
  agentKinds,
  busy,
  defaultAgentKind,
  editingPromptId,
  editingSoulId,
  lockAgentKind = false,
  promptDraft,
  promptTemplates,
  soulDraft,
  souls,
  title,
  onCreatePromptTemplate,
  onCreateSoul,
  onEditPromptTemplate,
  onEditSoul,
  onNewPromptTemplate,
  onNewSoul,
  onPromptDraft,
  onPublishPromptTemplate,
  onPublishSoul,
  onSavePromptTemplate,
  onSaveSoul,
  onSoulDraft,
}: {
  agentKinds: string[];
  busy: boolean;
  defaultAgentKind: string;
  editingPromptId: string;
  editingSoulId: string;
  lockAgentKind?: boolean;
  promptDraft: PromptTemplateDraft;
  promptTemplates: PromptTemplate[];
  soulDraft: { agentKind: string; name: string; content: string };
  souls: AgentSoul[];
  title: string;
  onCreatePromptTemplate: (event: FormEvent) => void;
  onCreateSoul: (event: FormEvent) => void;
  onEditPromptTemplate: (template: PromptTemplate) => void;
  onEditSoul: (soul: AgentSoul) => void;
  onNewPromptTemplate: () => void;
  onNewSoul: () => void;
  onPromptDraft: (draft: PromptTemplateDraft) => void;
  onPublishPromptTemplate: (id: string) => void;
  onPublishSoul: (id: string) => void;
  onSavePromptTemplate: (event: FormEvent) => void;
  onSaveSoul: (event: FormEvent) => void;
  onSoulDraft: (draft: { agentKind: string; name: string; content: string }) => void;
}) {
  const visibleSouls = souls
    .filter((soul) => agentKinds.includes(soul.agentKind))
    .slice()
    .sort((a, b) => statusSortOrder(a.status) - statusSortOrder(b.status));
  const visiblePrompts = promptTemplates
    .filter((template) => agentKinds.includes(template.agentKind))
    .slice()
    .sort((a, b) => statusSortOrder(a.status) - statusSortOrder(b.status));
  const updateSoulDraft = (patch: Partial<typeof soulDraft>) =>
    onSoulDraft({
      ...soulDraft,
      ...(lockAgentKind ? { agentKind: defaultAgentKind } : {}),
      ...patch,
    });
  const updatePromptDraft = (patch: Partial<PromptTemplateDraft>) =>
    onPromptDraft({
      ...promptDraft,
      ...(lockAgentKind ? { agentKind: defaultAgentKind } : {}),
      ...patch,
    });

  return (
    <section className={styles.panel}>
      <div className={styles.panelHead}>
        <div className={styles.panelHeadL}>
          <span className={styles.eyebrow}>Agent 提示词</span>
          <span className={styles.title}>{title}</span>
        </div>
      </div>

      <section className={styles.workbench}>
        <section className={styles.assetList}>
          <div className={styles.sectionCaption}>人格设定</div>
          {visibleSouls.map((soul) => (
            <button
              key={soul.id}
              className={editingSoulId === soul.id ? styles.assetRowSelected : styles.assetRow}
              onClick={() => onEditSoul(soul)}
            >
              <strong>
                {soul.name}
                {soul.status === "draft" && <span className={styles.statusBadge}>草稿</span>}
              </strong>
              <span>
                {agentKindLabel(soul.agentKind)} / v{soul.version} / {soul.status}
              </span>
              <p>{soul.content}</p>
            </button>
          ))}
          {!visibleSouls.length && <Empty text="暂无人格设定" />}
        </section>
        <form className={styles.form} onSubmit={editingSoulId ? onSaveSoul : onCreateSoul}>
          <div className={styles.formHead}>
            <div className={styles.formHeadL}>
              <span className={styles.formHeadEyebrow}>{editingSoulId ? "编辑" : "新增"}</span>
              <span className={styles.formHeadTitle}>{editingSoulId ? "编辑人格设定" : "新增人格设定"}</span>
            </div>
            <button type="button" className={styles.btnGhost} onClick={onNewSoul}>
              新建
            </button>
          </div>
          {lockAgentKind ? (
            <div className={styles.staticField}>
              <span>适用对象</span>
              <strong>{agentKindLabel(defaultAgentKind)}</strong>
            </div>
          ) : (
            <label className={styles.field}>
              <span>Agent 类型</span>
              <select
                className={styles.select}
                value={soulDraft.agentKind || defaultAgentKind}
                onChange={(event) => onSoulDraft({ ...soulDraft, agentKind: event.target.value })}
              >
                {agentKinds.map((kind) => (
                  <option key={kind} value={kind}>
                    {agentKindLabel(kind)}
                  </option>
                ))}
              </select>
            </label>
          )}
          <label className={styles.field}>
            <span>名称</span>
            <input
              className={styles.input}
              value={soulDraft.name}
              onChange={(event) => updateSoulDraft({ name: event.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span>人格提示词</span>
            <textarea
              className={styles.textarea}
              value={soulDraft.content}
              onChange={(event) => updateSoulDraft({ content: event.target.value })}
            />
          </label>
          <div className={styles.buttonRow}>
            <button
              type="submit"
              className={styles.btnPrimary}
              disabled={busy || !soulDraft.name.trim() || !soulDraft.content.trim()}
            >
              {editingSoulId ? "保存修改" : "保存草稿"}
            </button>
            {editingSoulId && (
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => onPublishSoul(editingSoulId)}
                disabled={busy}
              >
                发布
              </button>
            )}
          </div>
        </form>
      </section>

      <section className={styles.workbench} style={{ marginTop: 16 }}>
        <section className={styles.assetList}>
          <div className={styles.sectionCaption}>任务提示词</div>
          {visiblePrompts.map((template) => (
            <button
              key={template.id}
              className={editingPromptId === template.id ? styles.assetRowSelected : styles.assetRow}
              onClick={() => onEditPromptTemplate(template)}
            >
              <strong>
                {template.title}
                {template.status === "draft" && <span className={styles.statusBadge}>草稿</span>}
              </strong>
              <span>
                {agentKindLabel(template.agentKind)} / {template.layer} / v{template.version} / {template.status}
              </span>
              <p>{template.description || template.content}</p>
            </button>
          ))}
          {!visiblePrompts.length && <Empty text="暂无任务提示词" />}
        </section>
        <form className={styles.form} onSubmit={editingPromptId ? onSavePromptTemplate : onCreatePromptTemplate}>
          <div className={styles.formHead}>
            <div className={styles.formHeadL}>
              <span className={styles.formHeadEyebrow}>{editingPromptId ? "编辑" : "新增"}</span>
              <span className={styles.formHeadTitle}>{editingPromptId ? "编辑任务提示词" : "新增任务提示词"}</span>
            </div>
            <button type="button" className={styles.btnGhost} onClick={onNewPromptTemplate}>
              新建
            </button>
          </div>
          <div className={styles.formGrid}>
            <label className={styles.field}>
              <span>层级</span>
              <select
                className={styles.select}
                value={promptDraft.layer}
                onChange={(event) => updatePromptDraft({ layer: event.target.value })}
              >
                <option value="system_contract">系统契约</option>
                <option value="policy">运营规则</option>
                <option value="task_template">任务模板</option>
                <option value="review">复盘审查</option>
                <option value="methodology_generator">方法论生成</option>
              </select>
            </label>
            <label className={styles.field}>
              <span>标题</span>
              <input
                className={styles.input}
                value={promptDraft.title}
                onChange={(event) => updatePromptDraft({ title: event.target.value })}
              />
            </label>
          </div>
          <label className={styles.field}>
            <span>业务说明</span>
            <input
              className={styles.input}
              value={promptDraft.description}
              onChange={(event) => updatePromptDraft({ description: event.target.value })}
            />
          </label>
          <label className={styles.field}>
            <span>Prompt 内容</span>
            <textarea
              className={styles.textarea}
              value={promptDraft.content}
              onChange={(event) => updatePromptDraft({ content: event.target.value })}
            />
          </label>
          <details className={styles.advanced}>
            <summary>高级字段</summary>
            <div className={styles.formGrid}>
              <label className={styles.field}>
                <span>模板标识</span>
                <input
                  className={styles.input}
                  value={promptDraft.promptKey}
                  onChange={(event) => updatePromptDraft({ promptKey: event.target.value })}
                />
              </label>
              {lockAgentKind ? (
                <div className={styles.staticField}>
                  <span>适用对象</span>
                  <strong>{agentKindLabel(defaultAgentKind)}</strong>
                </div>
              ) : (
                <label className={styles.field}>
                  <span>Agent 类型</span>
                  <select
                    className={styles.select}
                    value={promptDraft.agentKind || defaultAgentKind}
                    onChange={(event) => onPromptDraft({ ...promptDraft, agentKind: event.target.value })}
                  >
                    {agentKinds.map((kind) => (
                      <option key={kind} value={kind}>
                        {agentKindLabel(kind)}
                      </option>
                    ))}
                  </select>
                </label>
              )}
            </div>
          </details>
          <div className={styles.buttonRow}>
            <button
              type="submit"
              className={styles.btnPrimary}
              disabled={busy || !promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()}
            >
              {editingPromptId ? "保存修改" : "保存草稿"}
            </button>
            {editingPromptId && (
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => onPublishPromptTemplate(editingPromptId)}
                disabled={busy}
              >
                发布
              </button>
            )}
          </div>
        </form>
      </section>
    </section>
  );
}

// operation_state_policies 灰度面板（admin 只读列表 + 三动作）。
function StatePolicyAdmin({ busy }: { busy: boolean }) {
  const [items, setItems] = useState<OperationStatePolicyEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includeAll, setIncludeAll] = useState(true);

  async function reload() {
    setLoading(true);
    setError(null);
    try {
      const data = await api.get<{ items: OperationStatePolicyEntry[] }>(
        `/api/admin/operation-state-policies?includeAllVersions=${includeAll}`
      );
      setItems(data.items ?? []);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [includeAll]);

  return (
    <section className={styles.panel}>
      <div className={styles.panelHead}>
        <div className={styles.panelHeadL}>
          <span className={styles.eyebrow}>State Policies</span>
          <span className={styles.title}>状态机动作策略灰度</span>
        </div>
        <div className={styles.buttonRow}>
          <label className={styles.inlineCheckbox}>
            <input type="checkbox" checked={includeAll} onChange={(event) => setIncludeAll(event.target.checked)} />
            <span>显示历史版本</span>
          </label>
          <button type="button" className={styles.btnGhost} onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
        </div>
      </div>
      {error && <div className={styles.inlineError}>{error}</div>}
      {!loading && items.length === 0 && <Empty text="暂无状态策略" />}
      <div className={styles.versionedList}>
        {items.map((item) => (
          <div key={item.id} className={styles.versionedListItem}>
            <div className={styles.versionedListHead}>
              <div>
                <span className={styles.versionedListScope}>{item.domain}</span>
                <h3>{item.stateKey}</h3>
              </div>
              <span className={item.status === "active" ? styles.badgeOk : styles.badgeDegraded}>{item.status}</span>
            </div>
            <ActiveVersionsBar
              meta={item}
              endpointPrefix="/api/admin/operation-state-policies"
              resourceLabel={`State ${item.domain}/${item.stateKey}`}
              busy={busy}
              canPublish
              onAfterAction={reload}
            />
            <div className={styles.versionedListBody}>
              <div className={styles.versionedListChunk}>
                <span>allowed</span>
                <p>{item.allowed.join("，") || "—"}</p>
              </div>
              <div className={styles.versionedListChunk}>
                <span>forbidden</span>
                <p>{item.forbidden.join("，") || "—"}</p>
              </div>
              <div className={styles.versionedListChunk}>
                <span>recommendedPace</span>
                <p>{item.recommendedPace || "—"}</p>
              </div>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

// system_taxonomies 灰度面板。
function TaxonomiesAdmin({ busy }: { busy: boolean }) {
  const [items, setItems] = useState<TaxonomyEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includeAll, setIncludeAll] = useState(true);
  const [includeDeprecated, setIncludeDeprecated] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [createDraft, setCreateDraft] = useState<TaxonomyDraft>({
    scope: "global",
    kind: "customer_stage",
    id: "",
    label: "",
    aliases: "",
    description: "",
  });
  const [acting, setActing] = useState(false);
  const [info, setInfo] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState<EditDraft>({ label: "", aliases: "", description: "" });

  async function reload() {
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      params.set("includeAllVersions", String(includeAll));
      params.set("includeDeprecated", String(includeDeprecated));
      const data = await api.get<{ items: TaxonomyEntry[] }>(`/api/admin/taxonomies?${params.toString()}`);
      setItems(data.items ?? []);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [includeAll, includeDeprecated]);

  async function submitCreate() {
    if (!createDraft.scope.trim() || !createDraft.kind.trim() || !createDraft.id.trim() || !createDraft.label.trim()) {
      setError("scope / kind / canonical id / 显示名 均不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliases = createDraft.aliases.split(/[,，]/).map((a) => a.trim()).filter((a) => a.length > 0);
      const res = await api.postRaw<{ error?: string; message?: string }>("/api/admin/taxonomies", {
        scope: createDraft.scope.trim(),
        kind: createDraft.kind.trim(),
        value: {
          id: createDraft.id.trim(),
          label: createDraft.label.trim(),
          aliases,
          description: createDraft.description.trim() || undefined,
        },
      });
      if (res.status === 409) {
        setInfo(res.data?.message ?? "该字典条目已存在。");
      } else if (!res.ok) {
        setError(res.data?.message ?? res.data?.error ?? `HTTP ${res.status}`);
        return;
      } else {
        setInfo(`已新增：${createDraft.id.trim()}`);
        setShowCreate(false);
        setCreateDraft({ scope: "global", kind: "customer_stage", id: "", label: "", aliases: "", description: "" });
      }
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  async function submitEdit(id: string) {
    if (!editDraft.label.trim()) {
      setError("显示名不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliases = editDraft.aliases.split(/[,，]/).map((a) => a.trim()).filter((a) => a.length > 0);
      await api.patch(`/api/admin/taxonomies/${id}`, {
        label: editDraft.label.trim(),
        aliases,
        description: editDraft.description.trim(),
      });
      setInfo("已更新。");
      setEditingId(null);
      await reload();
    } catch (e) {
      setError((e as Error).message);
      await reload();
    } finally {
      setActing(false);
    }
  }

  async function deprecateEntry(id: string) {
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      await api.delete(`/api/admin/taxonomies/${id}`);
      setInfo(includeDeprecated ? "已废弃。" : "已废弃，勾选「显示已废弃」可查看。");
      await reload();
    } catch (e) {
      setError((e as Error).message);
      await reload();
    } finally {
      setActing(false);
    }
  }

  async function restoreEntry(id: string) {
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      await api.patch(`/api/admin/taxonomies/${id}`, { deprecated: false });
      setInfo("已恢复为启用。");
      await reload();
    } catch (e) {
      setError((e as Error).message);
      await reload();
    } finally {
      setActing(false);
    }
  }

  return (
    <section className={styles.panel}>
      <div className={styles.panelHead}>
        <div className={styles.panelHeadL}>
          <span className={styles.eyebrow}>Taxonomies</span>
          <span className={styles.title}>双层标签字典灰度</span>
        </div>
        <div className={styles.buttonRow}>
          <label className={styles.inlineCheckbox}>
            <input type="checkbox" checked={includeAll} onChange={(event) => setIncludeAll(event.target.checked)} />
            <span>显示历史版本</span>
          </label>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={includeDeprecated}
              onChange={(event) => setIncludeDeprecated(event.target.checked)}
            />
            <span>显示已废弃</span>
          </label>
          <button type="button" className={styles.btnGhost} onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
          <button type="button" className={styles.btnGhost} onClick={() => { setShowCreate((v) => !v); setEditingId(null); setInfo(null); setError(null); }} disabled={busy || loading}>
            {showCreate ? "收起新增" : "新增条目"}
          </button>
        </div>
      </div>
      {error && <div className={styles.inlineError}>{error}</div>}
      {info && <div className={styles.badgeOk} style={{ display: "inline-block", marginBottom: 8 }}>{info}</div>}
      {showCreate && (
        <div className={styles.form} style={{ marginBottom: 14 }}>
          <label className={styles.field}>
            <span>scope（global = 全局，填 accountId = 仅该账号）</span>
            <input className={styles.input} placeholder="global" value={createDraft.scope}
              onChange={(e) => setCreateDraft({ ...createDraft, scope: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>kind（维度，如 customer_stage / intent_level / objection_type）</span>
            <input className={styles.input} placeholder="customer_stage" value={createDraft.kind}
              onChange={(e) => setCreateDraft({ ...createDraft, kind: e.target.value })} />
          </label>
          {createDraft.kind.trim() === "customer_stage" && (
            <p className={styles.panelHint}>
              新增客户阶段后，需到上方「状态机灰度」面板同步配置对应 state，否则该阶段的 operation_state 流转校验会被跳过。
            </p>
          )}
          <label className={styles.field}>
            <span>canonical id（建议英文 snake_case，如 need_discovery）</span>
            <input className={styles.input} placeholder="canonical id（如 need_discovery）" value={createDraft.id}
              onChange={(e) => setCreateDraft({ ...createDraft, id: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>显示名</span>
            <input className={styles.input} placeholder="显示名（如 需求挖掘）" value={createDraft.label}
              onChange={(e) => setCreateDraft({ ...createDraft, label: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>别名（逗号分隔，可空）</span>
            <input className={styles.input} placeholder="别名（逗号分隔，可空）" value={createDraft.aliases}
              onChange={(e) => setCreateDraft({ ...createDraft, aliases: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>描述（可空）</span>
            <textarea className={styles.textarea} value={createDraft.description}
              onChange={(e) => setCreateDraft({ ...createDraft, description: e.target.value })} />
          </label>
          <div className={styles.buttonRow}>
            <button type="button" className={styles.btnPrimary} onClick={() => void submitCreate()} disabled={acting}>保存</button>
            <button type="button" className={styles.btnGhost} onClick={() => setShowCreate(false)} disabled={acting}>取消</button>
          </div>
        </div>
      )}
      {!loading && items.length === 0 && <Empty text="暂无字典条目" />}
      <div className={styles.versionedList}>
        {items.map((item) => (
          <div key={item.id} className={styles.versionedListItem}>
            <div className={styles.versionedListHead}>
              <div>
                <span className={styles.versionedListScope}>
                  {item.scope} · {item.kind}
                </span>
                <h3>{item.value.label || item.value.id}</h3>
              </div>
              <span className={item.value.status === "active" ? styles.badgeOk : styles.badgeDegraded}>
                {item.value.status}
              </span>
            </div>
            <ActiveVersionsBar
              meta={item}
              endpointPrefix="/api/admin/taxonomies"
              resourceLabel={`Taxonomy ${item.scope}/${item.kind}/${item.value.id}`}
              busy={busy}
              canPublish
              onAfterAction={reload}
            />
            <div className={styles.versionedListBody}>
              <div className={styles.versionedListChunk}>
                <span>id</span>
                <p>{item.value.id}</p>
              </div>
              <div className={styles.versionedListChunk}>
                <span>aliases</span>
                <p>{(item.value.aliases ?? []).join("，") || "—"}</p>
              </div>
              {item.value.description && (
                <div className={styles.versionedListChunk}>
                  <span>description</span>
                  <p>{item.value.description}</p>
                </div>
              )}
            </div>
            {editingId !== item.id && item.currentVersion !== false && (
              <div className={styles.buttonRow}>
                <button type="button" className={styles.btnGhost}
                  onClick={() => { setShowCreate(false); setEditingId(item.id); setEditDraft({ label: item.value.label, aliases: (item.value.aliases ?? []).join("，"), description: item.value.description ?? "" }); setInfo(null); setError(null); }}
                  disabled={busy || acting}>编辑</button>
                {item.value.status === "active" ? (
                  <button type="button" className={styles.btnGhost} onClick={() => void deprecateEntry(item.id)} disabled={busy || acting}>废弃</button>
                ) : (
                  <button type="button" className={styles.btnGhost} onClick={() => void restoreEntry(item.id)} disabled={busy || acting}>恢复</button>
                )}
              </div>
            )}
            {editingId === item.id && (
              <div className={styles.form} style={{ marginTop: 12 }}>
                <label className={styles.field}>
                  <span>显示名</span>
                  <input className={styles.input} value={editDraft.label}
                    onChange={(e) => setEditDraft({ ...editDraft, label: e.target.value })} />
                </label>
                <label className={styles.field}>
                  <span>别名（逗号分隔，可空）</span>
                  <input className={styles.input} value={editDraft.aliases}
                    onChange={(e) => setEditDraft({ ...editDraft, aliases: e.target.value })} />
                </label>
                <label className={styles.field}>
                  <span>描述（可空）</span>
                  <textarea className={styles.textarea} value={editDraft.description}
                    onChange={(e) => setEditDraft({ ...editDraft, description: e.target.value })} />
                </label>
                <div className={styles.buttonRow}>
                  <button type="button" className={styles.btnPrimary} onClick={() => void submitEdit(item.id)} disabled={acting}>保存编辑</button>
                  <button type="button" className={styles.btnGhost} onClick={() => setEditingId(null)} disabled={acting}>取消</button>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

interface TaxonomyCandidate {
  id: string;
  scope: string;
  kind: string;
  rawValue: string;
  evidence: string | null;
  confidence: number;
  occurrences: number;
  status: string;
  firstSeenAt: string | null;
  lastSeenAt: string | null;
  reviewedAt: string | null;
  reviewedBy: string | null;
  suggestedDisplayName: string | null;
}

interface ApproveDraft {
  id: string;
  label: string;
  aliases: string;
  description: string;
}

const CANDIDATE_STATUS_FILTERS = ["pending", "approved", "rejected", "all"] as const;
type CandidateStatusFilter = (typeof CANDIDATE_STATUS_FILTERS)[number];

const CANDIDATE_STATUS_LABEL: Record<CandidateStatusFilter, string> = {
  pending: "待审核",
  approved: "已采纳",
  rejected: "已驳回",
  all: "全部",
};

function candidateStatusBadgeClass(status: string): string {
  if (status === "approved") return styles.badgeOk;
  if (status === "rejected") return styles.badgeDegraded;
  return styles.badgeWarn;
}

function TaxonomyCandidatesAdmin({ busy }: { busy: boolean }) {
  const [items, setItems] = useState<TaxonomyCandidate[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<CandidateStatusFilter>("pending");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [approveDraft, setApproveDraft] = useState<ApproveDraft>({
    id: "",
    label: "",
    aliases: "",
    description: "",
  });
  const [rejectingId, setRejectingId] = useState<string | null>(null);
  const [rejectReason, setRejectReason] = useState("");
  const [acting, setActing] = useState(false);

  async function reload() {
    setLoading(true);
    setError(null);
    try {
      const data = await api.get<{ items: TaxonomyCandidate[] }>(
        `/api/admin/taxonomy-candidates?status=${encodeURIComponent(statusFilter)}`
      );
      setItems(data.items ?? []);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [statusFilter]);

  function openApprove(item: TaxonomyCandidate) {
    setRejectingId(null);
    setInfo(null);
    setError(null);
    setExpandedId(item.id);
    setApproveDraft({
      id: item.rawValue,
      label: item.suggestedDisplayName || item.rawValue,
      aliases: "",
      description: item.evidence ?? "",
    });
  }

  function openReject(item: TaxonomyCandidate) {
    setExpandedId(null);
    setInfo(null);
    setError(null);
    setRejectingId(item.id);
    setRejectReason("");
  }

  async function submitApprove(candidateId: string) {
    if (!approveDraft.id.trim() || !approveDraft.label.trim()) {
      setError("canonical id 与显示名不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliases = approveDraft.aliases
        .split(/[,，]/)
        .map((a) => a.trim())
        .filter((a) => a.length > 0);
      const res = await api.postRaw<{ error?: string; message?: string }>(
        `/api/admin/taxonomy-candidates/${candidateId}/approve`,
        {
          canonicalValue: {
            id: approveDraft.id.trim(),
            label: approveDraft.label.trim(),
            aliases,
            description: approveDraft.description.trim() || undefined,
          },
        }
      );
      if (res.status === 409) {
        // 409 不是错误：该 canonical 已在字典里，候选已被标记采纳。
        setInfo(res.data?.message ?? "该字典条目已存在，候选已标记采纳。");
      } else if (!res.ok) {
        setError(res.data?.message ?? res.data?.error ?? `HTTP ${res.status}`);
        return;
      } else {
        setInfo(`已采纳：${approveDraft.id.trim()}`);
      }
      setExpandedId(null);
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  async function submitReject(candidateId: string) {
    if (!rejectReason.trim()) {
      setError("驳回原因不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      await api.post(`/api/admin/taxonomy-candidates/${candidateId}/reject`, {
        reason: rejectReason.trim(),
      });
      setInfo("已驳回该候选。");
      setRejectingId(null);
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  return (
    <section className={styles.panel}>
      <div className={styles.panelHead}>
        <div className={styles.panelHeadL}>
          <span className={styles.eyebrow}>Taxonomy Candidates</span>
          <span className={styles.title}>新词候选审核</span>
        </div>
        <div className={styles.buttonRow}>
          {CANDIDATE_STATUS_FILTERS.map((s) => (
            <button
              key={s}
              type="button"
              className={`${styles.profileTab} ${statusFilter === s ? styles.profileTabActive : ""}`}
              onClick={() => setStatusFilter(s)}
            >
              {CANDIDATE_STATUS_LABEL[s]}
            </button>
          ))}
          <button type="button" className={styles.btnGhost} onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
        </div>
      </div>
      <p className={styles.panelHint}>
        AI 在对话中遇到字典外的新词会落为候选；采纳后并入 system_taxonomies 字典，驳回则不进字典。
      </p>
      {error && <div className={styles.inlineError}>{error}</div>}
      {info && <div className={styles.badgeOk} style={{ display: "inline-block", marginBottom: 8 }}>{info}</div>}
      {!loading && items.length === 0 && <Empty text="暂无候选" />}
      <div className={styles.versionedList}>
        {items.map((item) => (
          <div key={item.id} className={styles.versionedListItem}>
            <div className={styles.versionedListHead}>
              <div>
                <span className={styles.versionedListScope}>
                  {item.scope} · {item.kind}
                </span>
                <h3>{item.rawValue}</h3>
              </div>
              <span className={candidateStatusBadgeClass(item.status)}>{item.status}</span>
            </div>
            <div className={styles.versionedListBody}>
              <div className={styles.versionedListChunk}>
                <span>confidence / 出现次数</span>
                <p>
                  {item.confidence} · {item.occurrences} 次
                </p>
              </div>
              {item.evidence && (
                <div className={styles.versionedListChunk}>
                  <span>evidence</span>
                  <p>{item.evidence}</p>
                </div>
              )}
              {item.reviewedBy && (
                <div className={styles.versionedListChunk}>
                  <span>审核人</span>
                  <p>{item.reviewedBy}</p>
                </div>
              )}
            </div>

            {item.status === "pending" && expandedId !== item.id && rejectingId !== item.id && (
              <div className={styles.buttonRow}>
                <button type="button" className={styles.btnPrimary} onClick={() => openApprove(item)} disabled={busy || acting}>
                  采纳
                </button>
                <button type="button" className={styles.btnGhost} onClick={() => openReject(item)} disabled={busy || acting}>
                  驳回
                </button>
              </div>
            )}

            {expandedId === item.id && (
              <div className={styles.form} style={{ marginTop: 12 }}>
                <label className={styles.field}>
                  <span>canonical id（建议英文 slug，如 price_objection）</span>
                  <input
                    className={styles.input}
                    value={approveDraft.id}
                    onChange={(e) => setApproveDraft({ ...approveDraft, id: e.target.value })}
                  />
                </label>
                <label className={styles.field}>
                  <span>显示名</span>
                  <input
                    className={styles.input}
                    value={approveDraft.label}
                    onChange={(e) => setApproveDraft({ ...approveDraft, label: e.target.value })}
                  />
                </label>
                <label className={styles.field}>
                  <span>别名（逗号分隔，可空；rawValue 会自动并入）</span>
                  <input
                    className={styles.input}
                    value={approveDraft.aliases}
                    onChange={(e) => setApproveDraft({ ...approveDraft, aliases: e.target.value })}
                  />
                </label>
                <label className={styles.field}>
                  <span>描述（可空）</span>
                  <textarea
                    className={styles.textarea}
                    value={approveDraft.description}
                    onChange={(e) => setApproveDraft({ ...approveDraft, description: e.target.value })}
                  />
                </label>
                <div className={styles.buttonRow}>
                  <button type="button" className={styles.btnPrimary} onClick={() => void submitApprove(item.id)} disabled={acting}>
                    确认采纳
                  </button>
                  <button type="button" className={styles.btnGhost} onClick={() => setExpandedId(null)} disabled={acting}>
                    取消
                  </button>
                </div>
              </div>
            )}

            {rejectingId === item.id && (
              <div className={styles.form} style={{ marginTop: 12 }}>
                <label className={styles.field}>
                  <span>驳回原因</span>
                  <input
                    className={styles.input}
                    value={rejectReason}
                    placeholder="如：无业务相关性 / 与现有条目重复"
                    onChange={(e) => setRejectReason(e.target.value)}
                  />
                </label>
                <div className={styles.buttonRow}>
                  <button type="button" className={styles.btnPrimary} onClick={() => void submitReject(item.id)} disabled={acting}>
                    确认驳回
                  </button>
                  <button type="button" className={styles.btnGhost} onClick={() => setRejectingId(null)} disabled={acting}>
                    取消
                  </button>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

// ── DomainProfile 面板（行业配置向导）───────────────────────────────────────

function ProfileStatusBadge({ profile }: { profile: DomainProfile }) {
  if (profile.is_active) {
    return <span className={styles.badgeOk}>生效中</span>;
  }
  if (profile.current_version) {
    return <span className={styles.badgeWarn}>待激活</span>;
  }
  return <span className={styles.badge}>草稿</span>;
}

function ProfileTabBar({ tab, onSelect }: { tab: "list" | "generate"; onSelect: (t: "list" | "generate") => void }) {
  return (
    <div className={styles.profileTabBar}>
      <button
        type="button"
        className={`${styles.profileTab} ${tab === "list" ? styles.profileTabActive : ""}`}
        onClick={() => onSelect("list")}
      >
        已有配置
      </button>
      <button
        type="button"
        className={`${styles.profileTab} ${tab === "generate" ? styles.profileTabActive : ""}`}
        onClick={() => onSelect("generate")}
      >
        AI 生成新配置
      </button>
    </div>
  );
}

function ProfileEditor({
  profile,
  draft,
  onChange,
  onSave,
  onDelete,
  onRefresh,
  busy,
}: {
  profile: DomainProfile | null;
  draft: DomainProfileDraft;
  onChange: (d: DomainProfileDraft) => void;
  onSave: () => void;
  onDelete: () => void;
  onRefresh: () => void;
  busy: boolean;
}) {
  const update = (patch: Partial<DomainProfileDraft>) => onChange({ ...draft, ...patch });

  // 五闸阈值覆盖：改某个 camelCase 子字段。空串 → 删该 key；若改后整个对象再无任何
  // 数值，则把 threshold_overrides 设为 undefined（= 不声明，DEFAULT 零扰动，不发空对象）。
  const setThreshold = (key: keyof NonNullable<DomainProfileDraft["threshold_overrides"]>, raw: string) => {
    const next: NonNullable<DomainProfileDraft["threshold_overrides"]> = {
      ...(draft.threshold_overrides ?? {}),
    };
    if (raw.trim() === "") {
      delete next[key];
    } else {
      next[key] = Number(raw);
    }
    const hasAny = Object.values(next).some((v) => v != null);
    update({ threshold_overrides: hasAny ? next : undefined });
  };

  return (
    <div className={styles.profileEditor}>
      <div className={styles.profileEditorHead}>
        <span className={styles.formHeadEyebrow}>{profile ? "编辑" : "新增"}</span>
        <span className={styles.formHeadTitle}>
          {profile ? `行业配置：${profile.display_name || profile.profile_id}` : "新建行业配置"}
        </span>
      </div>

      <div className={styles.formGrid}>
        <label className={styles.field}>
          <span>Profile ID（唯一标识）</span>
          <input
            className={styles.input}
            value={draft.profile_id ?? ""}
            onChange={(e) => update({ profile_id: e.target.value })}
            placeholder="如 dental-implant-private"
            disabled={!!profile}
          />
        </label>
        <label className={styles.field}>
          <span>展示名</span>
          <input
            className={styles.input}
            value={draft.display_name ?? ""}
            onChange={(e) => update({ display_name: e.target.value })}
            placeholder="如 牙科种植 · 私立诊所"
          />
        </label>
      </div>

      <label className={styles.field}>
        <span>行业说明</span>
        <input
          className={styles.input}
          value={draft.description ?? ""}
          onChange={(e) => update({ description: e.target.value })}
          placeholder="本行业画像说明（人可读，一两句）"
        />
      </label>

      <label className={styles.field}>
        <span>Prompt 片段（注入决策提示）</span>
        <textarea
          className={styles.textarea}
          value={draft.prompt_fragment ?? ""}
          onChange={(e) => update({ prompt_fragment: e.target.value })}
          placeholder="本行业业务上下文片段（解释维度如何理解，不要写死与本行业无关的销售套路）"
          rows={4}
        />
      </label>

      <label className={styles.field}>
        <span>对话模式（逗号分隔，缺省四模式可不填）</span>
        <input
          className={styles.input}
          value={(draft.conversation_modes ?? []).join(", ")}
          onChange={(e) =>
            update({
              conversation_modes: e.target.value
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean),
            })
          }
          placeholder="friendly, informative, persuasive, supportive"
        />
      </label>

      <details className={styles.advanced}>
        <summary>维度配置</summary>
        <div className={styles.profileDimensionsSection}>
          {(draft.profile_dimensions ?? []).map((dim, i) => (
            <div key={i} className={styles.profileDimensionRow}>
              <input
                className={styles.input}
                value={dim.kind}
                placeholder="维度 key (snake_case)"
                onChange={(e) => {
                  const dims = [...(draft.profile_dimensions ?? [])];
                  dims[i] = { ...dim, kind: e.target.value };
                  update({ profile_dimensions: dims });
                }}
              />
              <input
                className={styles.input}
                value={dim.display_name}
                placeholder="中文维度名"
                onChange={(e) => {
                  const dims = [...(draft.profile_dimensions ?? [])];
                  dims[i] = { ...dim, display_name: e.target.value };
                  update({ profile_dimensions: dims });
                }}
              />
              <input
                className={styles.input}
                value={dim.description}
                placeholder="维度含义"
                onChange={(e) => {
                  const dims = [...(draft.profile_dimensions ?? [])];
                  dims[i] = { ...dim, description: e.target.value };
                  update({ profile_dimensions: dims });
                }}
              />
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => {
                  const dims = [...(draft.profile_dimensions ?? [])];
                  dims.splice(i, 1);
                  update({ profile_dimensions: dims });
                }}
              >
                删除
              </button>
            </div>
          ))}
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() =>
              update({
                profile_dimensions: [
                  ...(draft.profile_dimensions ?? []),
                  { kind: "", display_name: "", participates_in_decision: true, description: "" },
                ],
              })
            }
          >
            + 添加维度
          </button>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>承诺标记词</summary>
        <div className={styles.formGrid}>
          <label className={styles.field}>
            <span>绝对化效果承诺词（逗号分隔）</span>
            <textarea
              className={styles.textarea}
              value={(draft.commitment_markers?.product_effect ?? []).join(", ")}
              onChange={(e) =>
                update({
                  commitment_markers: {
                    ...(draft.commitment_markers ?? { product_effect: [], tone_only: [] }),
                    product_effect: e.target.value.split(",").map((s) => s.trim()).filter(Boolean),
                  },
                })
              }
              placeholder="保证 100%, 绝对有效, 一定能看到效果"
              rows={2}
            />
          </label>
          <label className={styles.field}>
            <span>语气类夸大词（逗号分隔）</span>
            <textarea
              className={styles.textarea}
              value={(draft.commitment_markers?.tone_only ?? []).join(", ")}
              onChange={(e) =>
                update({
                  commitment_markers: {
                    ...(draft.commitment_markers ?? { product_effect: [], tone_only: [] }),
                    tone_only: e.target.value.split(",").map((s) => s.trim()).filter(Boolean),
                  },
                })
              }
              placeholder="太棒了, 绝对值, 超级划算"
              rows={2}
            />
          </label>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>方法论生成器引导语（可选）</summary>
        <label className={styles.field}>
          <span>methodologyGeneratorPreamble</span>
          <textarea
            className={styles.textarea}
            value={draft.methodology_generator_preamble ?? ""}
            onChange={(e) => update({ methodology_generator_preamble: e.target.value })}
            placeholder="领域中性引导语，覆盖默认值。留空则用系统内置的领域中性引导语。"
            rows={4}
          />
        </label>
      </details>

      <details className={styles.advanced}>
        <summary>五闸阈值覆盖（可选）</summary>
        <p className={styles.panelHint}>
          留空 = 沿用该域默认（销售域 6/7/6/6/7）。仅调软/硬闸的触发分数线，不改闸的语义与
          「AI 永不自断成交 / 永不自动 verify」结构红线。0-10 分制。
        </p>
        <div className={styles.formGrid}>
          <label className={styles.field}>
            <span>事实风险拦截线（≥ 拦截，默认 6）</span>
            <input
              className={styles.input}
              type="number"
              min={0}
              max={10}
              value={draft.threshold_overrides?.factRiskBlockAt ?? ""}
              onChange={(e) => setThreshold("factRiskBlockAt", e.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span>压迫感拦截线（≥ 拦截，默认 7）</span>
            <input
              className={styles.input}
              type="number"
              min={0}
              max={10}
              value={draft.threshold_overrides?.pressureRiskBlockAt ?? ""}
              onChange={(e) => setThreshold("pressureRiskBlockAt", e.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span>拟人度改写线（&lt; 改写一次，默认 6）</span>
            <input
              className={styles.input}
              type="number"
              min={0}
              max={10}
              value={draft.threshold_overrides?.humanLikeRewriteBelow ?? ""}
              onChange={(e) => setThreshold("humanLikeRewriteBelow", e.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span>情绪价值改写线（&lt; 改写一次，默认 6）</span>
            <input
              className={styles.input}
              type="number"
              min={0}
              max={10}
              value={draft.threshold_overrides?.emotionalValueRewriteBelow ?? ""}
              onChange={(e) => setThreshold("emotionalValueRewriteBelow", e.target.value)}
            />
          </label>
          <label className={styles.field}>
            <span>产品准确度拦截线（&lt; 拦截产品声明，默认 7）</span>
            <input
              className={styles.input}
              type="number"
              min={0}
              max={10}
              value={draft.threshold_overrides?.productAccuracyBlockBelow ?? ""}
              onChange={(e) => setThreshold("productAccuracyBlockBelow", e.target.value)}
            />
          </label>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>人格 / 方法论本体覆盖（可选）</summary>
        <p className={styles.panelHint}>
          留空 = 回落 DB published soul/playbook + 内置销售域兜底（DEFAULT 逐字等价）。
          只换人格口吻与方法论叙述，<strong>不放宽边界保护红线</strong>（边界硬规则始终由系统 prompt 守护）。
        </p>
        <label className={styles.field}>
          <span>人格本体覆盖 soulOverride</span>
          <textarea
            className={styles.textarea}
            value={draft.soul_override ?? ""}
            onChange={(e) => update({ soul_override: e.target.value || undefined })}
            placeholder="整体替换决策系统提示的 Soul 层（人格本体）。留空回落默认。"
            rows={4}
          />
        </label>
        <label className={styles.field}>
          <span>方法论本体覆盖 methodologyOverride</span>
          <textarea
            className={styles.textarea}
            value={draft.methodology_override ?? ""}
            onChange={(e) => update({ methodology_override: e.target.value || undefined })}
            placeholder="整体替换拼进 user message 的「当前运营方法」段。留空回落 contact 绑定 playbook + 默认。"
            rows={4}
          />
        </label>
        <label className={styles.field}>
          <span>对话模式判定规则覆盖 conversationModePolicy</span>
          <textarea
            className={styles.textarea}
            value={draft.conversation_mode_policy ?? ""}
            onChange={(e) => update({ conversation_mode_policy: e.target.value || undefined })}
            placeholder="整体替换 policy「## 对话模式判定」段（写死销售世界观的判定规则）。建议以 ## 对话模式判定 开头，按本行业声明各 conversationMode 的命中优先级。留空回落默认销售判定。注意：模式与 5 闸关系、边界保护红线由系统写死守护，不受本字段影响。"
            rows={5}
          />
        </label>
      </details>

      <details className={styles.advanced}>
        <summary>自学习极性（H11）</summary>
        <p className={styles.panelHint}>
          正/负极 outcome 词集驱动召回排序 + 反向训练 + 卡死请示。留空回落内置销售极性。沉默/未分类一律删失（绝不臆测为负），本字段只声明正/负集。
        </p>
        <div className={styles.formGrid}>
          <label className={styles.field}>
            <span>正极 outcome（→ 强化，逗号分隔）</span>
            <textarea
              className={styles.textarea}
              value={(draft.outcome_polarity?.positive ?? []).join(", ")}
              onChange={(e) => {
                const positive = e.target.value.split(",").map((s) => s.trim()).filter(Boolean);
                const negative = draft.outcome_polarity?.negative ?? [];
                update({
                  outcome_polarity: positive.length || negative.length ? { positive, negative } : undefined,
                });
              }}
              placeholder="user_replied_buying_signal"
              rows={2}
            />
          </label>
          <label className={styles.field}>
            <span>负极 outcome（→ 反向，逗号分隔）</span>
            <textarea
              className={styles.textarea}
              value={(draft.outcome_polarity?.negative ?? []).join(", ")}
              onChange={(e) => {
                const negative = e.target.value.split(",").map((s) => s.trim()).filter(Boolean);
                const positive = draft.outcome_polarity?.positive ?? [];
                update({
                  outcome_polarity: positive.length || negative.length ? { positive, negative } : undefined,
                });
              }}
              placeholder="objection, stop_requested, unsubscribed, negative, complaint"
              rows={2}
            />
          </label>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>completeness 审计维度</summary>
        <div className={styles.profileDimensionsSection}>
          {(draft.coverage_dimensions ?? []).map((cov, i) => (
            <div key={i} className={styles.profileDimensionRow}>
              <input
                className={styles.input}
                value={cov.key}
                placeholder="维度 key"
                onChange={(e) => {
                  const arr = [...(draft.coverage_dimensions ?? [])];
                  arr[i] = { ...cov, key: e.target.value };
                  update({ coverage_dimensions: arr });
                }}
              />
              <input
                className={styles.input}
                value={cov.display_name}
                placeholder="中文维度名"
                onChange={(e) => {
                  const arr = [...(draft.coverage_dimensions ?? [])];
                  arr[i] = { ...cov, display_name: e.target.value };
                  update({ coverage_dimensions: arr });
                }}
              />
              <label className={styles.inlineCheckbox}>
                <input
                  type="checkbox"
                  checked={cov.required}
                  onChange={(e) => {
                    const arr = [...(draft.coverage_dimensions ?? [])];
                    arr[i] = { ...cov, required: e.target.checked };
                    update({ coverage_dimensions: arr });
                  }}
                />
                必备
              </label>
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => {
                  const arr = [...(draft.coverage_dimensions ?? [])];
                  arr.splice(i, 1);
                  update({ coverage_dimensions: arr });
                }}
              >
                删除
              </button>
            </div>
          ))}
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() =>
              update({
                coverage_dimensions: [
                  ...(draft.coverage_dimensions ?? []),
                  { key: "", display_name: "", required: false },
                ],
              })
            }
          >
            + 添加 completeness 维度
          </button>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>经营公式（H15）</summary>
        <p className={styles.panelHint}>reviewer 自检 + /evaluations 度量锚点，不进硬闸。留空回落销售四公式。</p>
        <div className={styles.profileDimensionsSection}>
          {(draft.business_formulas ?? []).map((f, i) => (
            <div key={i} className={styles.profileDimensionRow}>
              <input
                className={styles.input}
                value={f.key}
                placeholder="公式 key"
                onChange={(e) => {
                  const arr = [...(draft.business_formulas ?? [])];
                  arr[i] = { ...f, key: e.target.value };
                  update({ business_formulas: arr });
                }}
              />
              <input
                className={styles.input}
                value={f.display_name}
                placeholder="中文名"
                onChange={(e) => {
                  const arr = [...(draft.business_formulas ?? [])];
                  arr[i] = { ...f, display_name: e.target.value };
                  update({ business_formulas: arr });
                }}
              />
              <input
                className={styles.input}
                value={f.expression}
                placeholder="可读展开式"
                onChange={(e) => {
                  const arr = [...(draft.business_formulas ?? [])];
                  arr[i] = { ...f, expression: e.target.value };
                  update({ business_formulas: arr });
                }}
              />
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => {
                  const arr = [...(draft.business_formulas ?? [])];
                  arr.splice(i, 1);
                  update({ business_formulas: arr });
                }}
              >
                删除
              </button>
            </div>
          ))}
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() =>
              update({
                business_formulas: [
                  ...(draft.business_formulas ?? []),
                  { key: "", expression: "", display_name: "" },
                ],
              })
            }
          >
            + 添加公式
          </button>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>知识切片用途角色（H16）</summary>
        <p className={styles.panelHint}>替代写死的销售四态分桶。留空回落内置销售四态。</p>
        <div className={styles.profileDimensionsSection}>
          {(draft.chunk_roles ?? []).map((role, i) => (
            <div key={i} className={styles.profileDimensionRow}>
              <input
                className={styles.input}
                value={role.key}
                placeholder="chunk_type key"
                onChange={(e) => {
                  const arr = [...(draft.chunk_roles ?? [])];
                  arr[i] = { ...role, key: e.target.value };
                  update({ chunk_roles: arr });
                }}
              />
              <input
                className={styles.input}
                value={role.header}
                placeholder="分段标题 + 使用指令"
                onChange={(e) => {
                  const arr = [...(draft.chunk_roles ?? [])];
                  arr[i] = { ...role, header: e.target.value };
                  update({ chunk_roles: arr });
                }}
              />
              <input
                className={styles.input}
                type="number"
                value={role.order}
                placeholder="顺序"
                onChange={(e) => {
                  const arr = [...(draft.chunk_roles ?? [])];
                  arr[i] = { ...role, order: Number(e.target.value) || 0 };
                  update({ chunk_roles: arr });
                }}
              />
              <label className={styles.inlineCheckbox}>
                <input
                  type="checkbox"
                  checked={role.is_fallback}
                  onChange={(e) => {
                    const arr = [...(draft.chunk_roles ?? [])];
                    arr[i] = { ...role, is_fallback: e.target.checked };
                    update({ chunk_roles: arr });
                  }}
                />
                兜底桶
              </label>
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => {
                  const arr = [...(draft.chunk_roles ?? [])];
                  arr.splice(i, 1);
                  update({ chunk_roles: arr });
                }}
              >
                删除
              </button>
            </div>
          ))}
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() =>
              update({
                chunk_roles: [
                  ...(draft.chunk_roles ?? []),
                  { key: "", header: "", order: (draft.chunk_roles ?? []).length, is_fallback: false },
                ],
              })
            }
          >
            + 添加切片角色
          </button>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>记忆维度（H17）</summary>
        <p className={styles.panelHint}>memoryCard.extra 容器里的业务数组槽位。留空回落销售八槽。</p>
        <div className={styles.profileDimensionsSection}>
          {(draft.memory_dimensions ?? []).map((m, i) => (
            <div key={i} className={styles.profileDimensionRow}>
              <input
                className={styles.input}
                value={m.key}
                placeholder="槽位 key (camelCase)"
                onChange={(e) => {
                  const arr = [...(draft.memory_dimensions ?? [])];
                  arr[i] = { ...m, key: e.target.value };
                  update({ memory_dimensions: arr });
                }}
              />
              <input
                className={styles.input}
                value={m.display_name}
                placeholder="中文标签"
                onChange={(e) => {
                  const arr = [...(draft.memory_dimensions ?? [])];
                  arr[i] = { ...m, display_name: e.target.value };
                  update({ memory_dimensions: arr });
                }}
              />
              <input
                className={styles.input}
                type="number"
                min={1}
                value={m.cap}
                placeholder="上限"
                onChange={(e) => {
                  const arr = [...(draft.memory_dimensions ?? [])];
                  arr[i] = { ...m, cap: Math.max(1, Number(e.target.value) || 1) };
                  update({ memory_dimensions: arr });
                }}
              />
              <label className={styles.inlineCheckbox}>
                <input
                  type="checkbox"
                  checked={m.candidate_type}
                  onChange={(e) => {
                    const arr = [...(draft.memory_dimensions ?? [])];
                    arr[i] = { ...m, candidate_type: e.target.checked };
                    update({ memory_dimensions: arr });
                  }}
                />
                候选类型
              </label>
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => {
                  const arr = [...(draft.memory_dimensions ?? [])];
                  arr.splice(i, 1);
                  update({ memory_dimensions: arr });
                }}
              >
                删除
              </button>
            </div>
          ))}
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() =>
              update({
                memory_dimensions: [
                  ...(draft.memory_dimensions ?? []),
                  { key: "", display_name: "", cap: 8, is_core: false, candidate_type: false },
                ],
              })
            }
          >
            + 添加记忆维度
          </button>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>运营范式（H8/H19 三驱动力 + 作息门控）</summary>
        <p className={styles.panelHint}>关掉某驱动力 → 对应 planner 扫描短路（陪伴型常关 funnel）。</p>
        <div className={styles.formGrid}>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.operation_mode?.funnel?.enabled ?? true}
              onChange={(e) =>
                update({
                  operation_mode: {
                    funnel: { ...(draft.operation_mode?.funnel ?? { enabled: true }), enabled: e.target.checked },
                    silence: draft.operation_mode?.silence ?? { enabled: true },
                    commitment: draft.operation_mode?.commitment ?? { enabled: true },
                    quiet_hours: draft.operation_mode?.quiet_hours ?? {},
                  },
                })
              }
            />
            漏斗推进 funnel（停滞催进）
          </label>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.operation_mode?.silence?.enabled ?? true}
              onChange={(e) =>
                update({
                  operation_mode: {
                    funnel: draft.operation_mode?.funnel ?? { enabled: true },
                    silence: { ...(draft.operation_mode?.silence ?? { enabled: true }), enabled: e.target.checked },
                    commitment: draft.operation_mode?.commitment ?? { enabled: true },
                    quiet_hours: draft.operation_mode?.quiet_hours ?? {},
                  },
                })
              }
            />
            沉默唤醒 silence
          </label>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.operation_mode?.commitment?.enabled ?? true}
              onChange={(e) =>
                update({
                  operation_mode: {
                    funnel: draft.operation_mode?.funnel ?? { enabled: true },
                    silence: draft.operation_mode?.silence ?? { enabled: true },
                    commitment: { ...(draft.operation_mode?.commitment ?? { enabled: true }), enabled: e.target.checked },
                    quiet_hours: draft.operation_mode?.quiet_hours ?? {},
                  },
                })
              }
            />
            承诺到期 commitment
          </label>
        </div>
      </details>

      <details className={styles.advanced}>
        <summary>高级：交易/评审/轨迹（发布危险字段）</summary>
        <p className={styles.panelHint}>
          交易事实注入 / 评审取向 / 模式说明属发布危险字段，改动经发布确认流（riskyFields）二次确认。
        </p>
        <div className={styles.formGrid}>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.transaction_facts_enabled ?? false}
              onChange={(e) => update({ transaction_facts_enabled: e.target.checked })}
            />
            交易型域（注入产品目录 + 持有事实）transaction_facts_enabled
          </label>
        </div>
        <label className={styles.field}>
          <span>评审重点 review_focus</span>
          <input
            className={styles.input}
            type="text"
            value={draft.reviewer_orientation?.reviewFocus ?? ""}
            onChange={(e) =>
              update({
                reviewer_orientation: {
                  ...(draft.reviewer_orientation ?? {}),
                  reviewFocus: e.target.value || undefined,
                },
              })
            }
          />
        </label>
        <label className={styles.field}>
          <span>平衡原则 balance_principle</span>
          <input
            className={styles.input}
            type="text"
            value={draft.reviewer_orientation?.balancePrinciple ?? ""}
            onChange={(e) =>
              update({
                reviewer_orientation: {
                  ...(draft.reviewer_orientation ?? {}),
                  balancePrinciple: e.target.value || undefined,
                },
              })
            }
          />
        </label>
        <label className={styles.field}>
          <span>模式-闸说明覆盖 mode_gate_policy_override</span>
          <textarea
            className={styles.textarea}
            value={draft.mode_gate_policy_override ?? ""}
            onChange={(e) => update({ mode_gate_policy_override: e.target.value || undefined })}
          />
        </label>
        <label className={styles.field}>
          <span>去抖窗口（毫秒）debounce_window_ms_override</span>
          <input
            className={styles.input}
            type="number"
            value={draft.debounce_window_ms_override ?? ""}
            onChange={(e) =>
              update({
                debounce_window_ms_override: e.target.value ? Number(e.target.value) : undefined,
              })
            }
          />
        </label>
      </details>

      <details className={styles.advanced}>
        <summary>按关系类型分配运营范式（数字分身 per_relationship）</summary>
        <p className={styles.panelHint}>
          为不同关系类型(customer/peer/friend)各配一套范式。未配的关系类型回落 profile 级 operation_mode。
        </p>
        {(["customer", "peer", "friend"] as const).map((rt) => {
          const map = draft.per_relationship_operation_mode ?? {};
          const mode = map[rt];
          const enabled = !!mode;
          const setMode = (next: typeof mode | undefined) => {
            const nextMap = { ...(draft.per_relationship_operation_mode ?? {}) };
            if (next === undefined) {
              delete nextMap[rt];
            } else {
              nextMap[rt] = next;
            }
            update({ per_relationship_operation_mode: nextMap });
          };
          return (
            <div key={rt} className={styles.formGrid} data-testid={`per-rel-${rt}`}>
              <label className={styles.inlineCheckbox}>
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) =>
                    setMode(
                      e.target.checked
                        ? { funnel: { enabled: true }, silence: { enabled: true }, commitment: { enabled: true }, quiet_hours: {} }
                        : undefined,
                    )
                  }
                />
                为 {rt} 单独配置范式
              </label>
              {enabled && mode && (
                <>
                  <label className={styles.inlineCheckbox}>
                    <input
                      type="checkbox"
                      checked={mode.funnel?.enabled ?? true}
                      onChange={(e) =>
                        setMode({ ...mode, funnel: { ...(mode.funnel ?? { enabled: true }), enabled: e.target.checked } })
                      }
                    />
                    漏斗推进 funnel
                  </label>
                  <label className={styles.inlineCheckbox}>
                    <input
                      type="checkbox"
                      checked={mode.silence?.enabled ?? true}
                      onChange={(e) =>
                        setMode({ ...mode, silence: { ...(mode.silence ?? { enabled: true }), enabled: e.target.checked } })
                      }
                    />
                    沉默唤醒 silence
                  </label>
                  <label className={styles.inlineCheckbox}>
                    <input
                      type="checkbox"
                      checked={mode.commitment?.enabled ?? true}
                      onChange={(e) =>
                        setMode({ ...mode, commitment: { ...(mode.commitment ?? { enabled: true }), enabled: e.target.checked } })
                      }
                    />
                    承诺到期 commitment
                  </label>
                </>
              )}
            </div>
          );
        })}
      </details>

      <details className={styles.advanced}>
        <summary>领域标志位（高敏域可选）</summary>
        <div className={styles.formGrid}>
          <label className={styles.field}>
            <span>停滞计时驱动维度 stagnationDimension</span>
            <input
              className={styles.input}
              value={draft.stagnation_dimension ?? ""}
              onChange={(e) => update({ stagnation_dimension: e.target.value || undefined })}
              placeholder="留空回落 customer_stage"
            />
          </label>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.grounding_gate_bypass_without_claim ?? false}
              onChange={(e) => update({ grounding_gate_bypass_without_claim: e.target.checked })}
            />
            无产品声明时旁路 grounding 软闸（纯情感/关系域）
          </label>
          <label className={styles.inlineCheckbox}>
            <input
              type="checkbox"
              checked={draft.distrust_self_reported_low_risk ?? false}
              onChange={(e) => update({ distrust_self_reported_low_risk: e.target.checked })}
            />
            不信任自报低风险（强制走独立 review，高敏域）
          </label>
        </div>
      </details>

      <div className={styles.buttonRow}>
        <button
          type="button"
          className={styles.btnPrimary}
          onClick={onSave}
          disabled={busy || !draft.profile_id?.trim()}
        >
          {profile ? "保存修改" : "创建草稿"}
        </button>
        {profile && !profile.is_active && (
          <>
            <ProfilePublishCard profileId={profile.id} onDone={onRefresh} />
            <button
              type="button"
              className={styles.btnGhost}
              onClick={onDelete}
              disabled={busy}
            >
              删除
            </button>
          </>
        )}
      </div>
    </div>
  );
}

function DomainProfilePanel({ busy }: { busy: boolean }) {
  const {
    domainProfiles,
    editingProfile,
    profileDraft,
    profileTab,
    generating,
    generateError,
    generateResult,
    loadDomainProfiles,
    selectProfileTab,
    editDomainProfile,
    newDomainProfileDraft,
    setProfileDraft,
    saveDomainProfile,
    deleteDomainProfile,
    generateDomainProfile,
  } = useStrategyStore();

  const [gen_pid, setGen_pid] = useState("");
  const [gen_display_name, setGen_display_name] = useState("");
  const [gen_business_description, setGen_business_description] = useState("");

  useEffect(() => {
    void loadDomainProfiles();
  }, [loadDomainProfiles]);

  const activeProfile = domainProfiles.find((p) => p.is_active);
  const editing = editingProfile !== null;

  function handleProfileClick(profile: DomainProfile) {
    editDomainProfile(profile);
  }

  return (
    <section className={styles.panel}>
      <div className={styles.panelHead}>
        <div className={styles.panelHeadL}>
          <span className={styles.eyebrow}>Domain Profile</span>
          <span className={styles.title}>行业配置管理</span>
        </div>
      </div>

      {activeProfile && (
        <div className={styles.profileActiveBanner}>
          <span className={styles.profileActiveLabel}>当前生效：</span>
          <strong>{activeProfile.display_name || activeProfile.profile_id}</strong>
          <span className={styles.profileActiveMeta}>
            v{activeProfile.version} · {activeProfile.profile_id}
          </span>
        </div>
      )}

      <ProfileTabBar tab={profileTab} onSelect={selectProfileTab} />

      {profileTab === "generate" && (
        <div className={styles.profileGenerateSection}>
          <p className={styles.panelHint}>
            描述你的业务（行业/产品/客户/经营目标/对话风格），AI 将生成一份候选行业配置。
            候选需要你审核确认后 publish + activate 才会生效。
          </p>
          <label className={styles.field}>
            <span>Profile ID（英文唯一标识，生成后不可改）</span>
            <input
              className={styles.input}
              value={gen_pid}
              onChange={(e) => setGen_pid(e.target.value)}
              placeholder="如 emotional-companion-care 或 edu-k12-tuition"
            />
          </label>
          <label className={styles.field}>
            <span>展示名（可选）</span>
            <input
              className={styles.input}
              value={gen_display_name}
              onChange={(e) => setGen_display_name(e.target.value)}
              placeholder="如未填则用 profileId"
            />
          </label>
          <label className={styles.field}>
            <span>业务描述（请详细描述你的行业/产品/客户特征/运营目标）</span>
            <textarea
              className={styles.textarea}
              value={gen_business_description}
              onChange={(e) => setGen_business_description(e.target.value)}
              placeholder={"我是做K12辅导的，主要接触的是家长。\n说实话，这些家长比孩子更焦虑。他们不是来「了解课程」的，是来「找一个人帮他们解决一个问题」的。\n孩子成绩上不去，在家里说话都没底气。找到我的时候，其实是在找一个出口。\n我最怕说错话是：承诺「一个月提多少分」——家长一听就知道是假的，反而更不信任。\n真正打动家长的，是我愿意听他把孩子的具体情况说完，然后给一个真实、可落地的判断。"}
              rows={8}
            />
          </label>
          {generateError && <div className={styles.inlineError}>{generateError}</div>}
          {generateResult && (
            <div className={styles.profileGenerateSuccess}>
              ✅ 候选配置已生成！可在「已有配置」列表中找到 v1 草稿，逐项审核后 publish + activate。
              <br />
              如本次为新行业，AI 同时生成了取值字典候选（客户阶段 / 意向等级等维度的中文标签），需在本频道「新词候选审核」面板逐条采纳后，运营看板才会把这些维度显示为中文，否则将灰显英文原值。
            </div>
          )}
          <div className={styles.buttonRow}>
            <button
              type="button"
              className={styles.btnPrimary}
              onClick={() => void generateDomainProfile(gen_business_description, gen_pid, gen_display_name || undefined)}
              disabled={generating || !gen_business_description.trim() || !gen_pid.trim()}
            >
              {generating ? "生成中…" : "🚀 AI 生成候选配置"}
            </button>
            <button
              type="button"
              className={styles.btnGhost}
              onClick={() => {
                setGen_business_description("");
                setGen_pid("");
                setGen_display_name("");
                selectProfileTab("list");
              }}
              disabled={generating}
            >
              取消
            </button>
          </div>
        </div>
      )}

      {profileTab === "list" && (
        <div className={styles.profileListLayout}>
          {/* 左侧列表 */}
          <div className={styles.profileList}>
            {domainProfiles.length === 0 && !busy ? (
              <Empty text="暂无行业配置，请先 AI 生成或手动创建" />
            ) : (
              domainProfiles.map((profile) => (
                <button
                  key={profile.id}
                  type="button"
                  className={editingProfile?.id === profile.id ? styles.assetRowSelected : styles.assetRow}
                  onClick={() => handleProfileClick(profile)}
                >
                  <strong>
                    {profile.display_name || profile.profile_id}
                    {profile.seeded_by === "generated_by_ai" && (
                      <span className={styles.statusBadge}>AI 候选</span>
                    )}
                  </strong>
                  <span>
                    {profile.profile_id} · v{profile.version}
                    {profile.previous_version != null ? ` (← v${profile.previous_version})` : ""}
                  </span>
                  <p>{profile.description || "—"}</p>
                  <div className={styles.profileListMeta}>
                    <ProfileStatusBadge profile={profile} />
                    {profile.seeded_by && (
                      <span className={styles.activeVersionsSeeded}>{profile.seeded_by}</span>
                    )}
                    {profile.updated_at && (
                      <span className={styles.activeVersionsTimestamp}>{profile.updated_at}</span>
                    )}
                  </div>
                </button>
              ))
            )}
            <button
              type="button"
              className={styles.btnGhost}
              onClick={() => {
                newDomainProfileDraft();
                selectProfileTab("list");
              }}
              style={{ width: "100%", justifyContent: "center" }}
            >
              + 手动创建空白配置
            </button>
          </div>

          {/* 右侧编辑区 */}
          {editing ? (
            <ProfileEditor
              profile={editingProfile}
              draft={profileDraft}
              onChange={setProfileDraft}
              onSave={() => {
                if (editingProfile) void saveDomainProfile(editingProfile.id);
              }}
              onDelete={() => {
                if (editingProfile) void deleteDomainProfile(editingProfile.id);
              }}
              onRefresh={() => void loadDomainProfiles()}
              busy={busy}
            />
          ) : (
            <div className={styles.profileEditorPlaceholder}>
              <Inbox size={24} />
              <p>选择左侧一项配置进行编辑，或点击「AI 生成新配置」创建新的候选配置</p>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function LessonsLearnedAdmin({ busy }: { busy: boolean }) {
  const [items, setItems] = useState<LessonLearnedEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [patternKind, setPatternKind] = useState<string>("");
  const [promoting, setPromoting] = useState<string | null>(null); // lesson_id

  async function reload() {
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      if (patternKind) params.set("patternKind", patternKind);
      const qs = params.toString();
      const data = await api.get<{ items: LessonLearnedEntry[] }>(
        `/api/admin/lessons-learned${qs ? `?${qs}` : ""}`
      );
      setItems(data.items ?? []);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }

  function openPromote(lessonId: string) {
    setPromoting(lessonId);
  }

  function closePromote() {
    setPromoting(null);
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [patternKind]);

  function patternBadgeClass(kind: string): string {
    if (kind === "success") return styles.badgeOk;
    if (kind === "reviewer_misjudge_negative") return styles.badgeDegraded;
    if (kind === "blocked_by_safety_guard") return styles.badgeWarn;
    return styles.badge;
  }

  function patternLabel(kind: string): string {
    if (kind === "success") return "成功模式";
    if (kind === "reviewer_misjudge_negative") return "Reviewer 误判（用户负反应）";
    if (kind === "blocked_by_safety_guard") return "安全门拦截";
    return kind || "未识别";
  }

  return (
    <section className={styles.panel}>
      <div className={styles.panelHead}>
        <div className={styles.panelHeadL}>
          <span className={styles.eyebrow}>Lessons Learned</span>
          <span className={styles.title}>跨用户教训归纳（14d 滑窗）</span>
        </div>
        <div className={styles.buttonRow}>
          <select
            className={styles.selectInline}
            value={patternKind}
            onChange={(event) => setPatternKind(event.target.value)}
            disabled={busy || loading}
          >
            <option value="">全部模式</option>
            <option value="success">success</option>
            <option value="reviewer_misjudge_negative">reviewer_misjudge_negative</option>
            <option value="blocked_by_safety_guard">blocked_by_safety_guard</option>
          </select>
          <button type="button" className={styles.btnGhost} onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
        </div>
      </div>
      <p className={styles.panelHint}>
        feedback_worker 周期把 agent_run_logs 的胜/败模式压缩成可被下一轮决策检索的颗粒；
        admin 在此抽象为 chunk_type=peer_case 候选 chunk（仍走知识审核队列二次确认才能 verify）。
      </p>
      {error && <div className={styles.inlineError}>{error}</div>}
      {!loading && items.length === 0 && <Empty text="暂无教训聚合（窗口内无命中样本）" />}
      <div className={styles.versionedList}>
        {items.map((item) => (
          <div key={item.lessonId} className={styles.versionedListItem}>
            <div className={styles.versionedListHead}>
              <div>
                <span className={styles.versionedListScope}>{patternLabel(item.patternKind)}</span>
                <h3>
                  {item.lessonId}
                  <span className={styles.countTag}>×{item.count}</span>
                </h3>
              </div>
              <div className={styles.buttonRow}>
                <span className={patternBadgeClass(item.patternKind)}>{item.reviewStatus}</span>
                {item.reviewStatus !== "promoted" && (
                  <button
                    type="button"
                    className={styles.btnGhost}
                    onClick={() => openPromote(item.lessonId)}
                    disabled={busy || loading || promoting !== null}
                  >
                    晋升为 peer_case
                  </button>
                )}
              </div>
            </div>
            <div className={styles.versionedListBody}>
              <div className={styles.versionedListChunk}>
                <span>sample run ids ({item.sampleRunIds.length})</span>
                <p>
                  {item.sampleRunIds.length === 0
                    ? "—"
                    : item.sampleRunIds.map((rid) => (
                        <code key={rid} className={styles.codeChip}>
                          {rid}
                        </code>
                      ))}
                </p>
              </div>
              <div className={styles.versionedListChunk}>
                <span>updated</span>
                <p>{item.updatedAt || "—"}</p>
              </div>
              <div className={styles.versionedListChunk}>
                <span>created</span>
                <p>{item.createdAt || "—"}</p>
              </div>
              {item.promotedChunkId && (
                <div className={styles.versionedListChunk}>
                  <span>promoted chunk</span>
                  <p>
                    <code>{item.promotedChunkId}</code>
                  </p>
                </div>
              )}
              {promoting === item.lessonId && (
                <div className={styles.versionedListChunk} style={{ gridColumn: "1 / -1" }}>
                  <LessonPromoteCard
                    lessonId={item.lessonId}
                    onDone={() => {
                      closePromote();
                      void reload();
                    }}
                  />
                </div>
              )}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

export default function SystemStrategyFeature() {
  // 卡片（ProfilePublishCard 等中立化处置卡）用 useConfirm/useToast，必须有 Provider 祖先，
  // 否则运行时抛错。原 body 原封不动搬进 SystemStrategyInner，此处只加外层 Provider 包裹。
  return (
    <ConfirmProvider>
      <ToastProvider>
        <SystemStrategyInner />
      </ToastProvider>
    </ConfirmProvider>
  );
}

function SystemStrategyInner() {
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
    newPromptDraftFor,
  } = useStrategyStore();

  useEffect(() => {
    void loadStrategyData();
  }, [loadStrategyData]);

  const handleCreateSoul = (e: FormEvent) => {
    e.preventDefault();
    void createSoul();
  };
  const handleSaveSoul = (e: FormEvent) => {
    e.preventDefault();
    void saveSoul();
  };
  const handleCreatePromptTemplate = (e: FormEvent) => {
    e.preventDefault();
    void createPromptTemplate();
  };
  const handleSavePromptTemplate = (e: FormEvent) => {
    e.preventDefault();
    void savePromptTemplate();
  };

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.panelHead}>
          <div className={styles.panelHeadL}>
            <span className={styles.eyebrow}>Global Strategy</span>
            <span className={styles.title}>系统总控策略</span>
          </div>
          <div className={styles.headIcon}>
            <Settings2 size={18} />
          </div>
        </div>
        <div className={styles.methodCards}>
          <div className={styles.methodCard}>
            <span>后台管理 Agent</span>
            <p>把自然语言指令转成微信工具调用、项目配置和运营管理任务。</p>
          </div>
          <div className={styles.methodCard}>
            <span>方法论生成 Agent</span>
            <p>把业务目标、人群差异和复盘结果生成可读、可编辑、可验证的方法论。</p>
          </div>
          <div className={styles.methodCard}>
            <span>全局边界</span>
            <p>只管理跨模块规则；用户运营的具体长期策略在用户运营频道维护。</p>
          </div>
        </div>
        <div className={styles.buttonRow} style={{ marginTop: 14 }}>
          <button
            type="button"
            className={styles.btnGhost}
            onClick={() => void resetSystemPromptPack()}
            disabled={busy}
          >
            重置系统 Prompt Pack v2
          </button>
        </div>
      </section>

      <DomainPromptPanel
        busy={busy}
        editingPromptId={editingPromptId}
        editingSoulId={editingSoulId}
        promptDraft={promptDraft}
        promptTemplates={promptTemplates}
        soulDraft={soulDraft}
        souls={souls}
        agentKinds={["management", "methodology"]}
        defaultAgentKind="management"
        title="系统总控 Prompt"
        onCreatePromptTemplate={handleCreatePromptTemplate}
        onCreateSoul={handleCreateSoul}
        onEditPromptTemplate={editPromptTemplate}
        onEditSoul={editSoul}
        onNewPromptTemplate={() => newPromptDraftFor("management")}
        onNewSoul={() => newSoulDraftFor("management")}
        onPromptDraft={setPromptDraft}
        onPublishPromptTemplate={(id) => void publishPromptTemplate(id)}
        onPublishSoul={(id) => void publishSoul(id)}
        onSavePromptTemplate={handleSavePromptTemplate}
        onSaveSoul={handleSaveSoul}
        onSoulDraft={setSoulDraft}
      />

      <StatePolicyAdmin busy={busy} />
      <TaxonomiesAdmin busy={busy} />
      <TaxonomyCandidatesAdmin busy={busy} />
      <LessonsLearnedAdmin busy={busy} />
      <DomainProfilePanel busy={busy} />
    </div>
  );
}
