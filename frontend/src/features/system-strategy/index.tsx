import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { Settings2, Inbox } from "lucide-react";
import { api } from "../../lib/api";
import { useUiStore } from "../../stores/uiStore";
import { useStrategyStore } from "../../stores/strategyStore";
import type { AgentSoul, PromptTemplate, PromptTemplateDraft } from "../../types";
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
        </div>
      </div>
      {error && <div className={styles.inlineError}>{error}</div>}
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
          </div>
        ))}
      </div>
    </section>
  );
}

function LessonsLearnedAdmin({ busy }: { busy: boolean }) {
  const [items, setItems] = useState<LessonLearnedEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [patternKind, setPatternKind] = useState<string>("");
  const [promoting, setPromoting] = useState<string | null>(null); // lesson_id
  const [draftTitle, setDraftTitle] = useState("");
  const [draftBody, setDraftBody] = useState("");
  const [draftSummary, setDraftSummary] = useState("");
  const [promoteError, setPromoteError] = useState<string | null>(null);

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
    setDraftTitle("");
    setDraftBody("");
    setDraftSummary("");
    setPromoteError(null);
  }

  function closePromote() {
    setPromoting(null);
    setDraftTitle("");
    setDraftBody("");
    setDraftSummary("");
    setPromoteError(null);
  }

  async function submitPromote() {
    if (!promoting) return;
    if (!draftTitle.trim() || !draftBody.trim()) {
      setPromoteError("title 和 body 都不能为空");
      return;
    }
    setPromoteError(null);
    try {
      const payload: Record<string, string> = {
        title: draftTitle.trim(),
        body: draftBody.trim(),
      };
      if (draftSummary.trim()) payload.summary = draftSummary.trim();
      await api.post(
        `/api/admin/lessons-learned/${encodeURIComponent(promoting)}/promote-to-peer-case`,
        payload
      );
      closePromote();
      void reload();
    } catch (e) {
      setPromoteError((e as Error).message);
    }
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
                  <span>晋升为 peer_case 候选 chunk（仍需 admin 在知识审核队列 verify）</span>
                  <div className={styles.promoteForm}>
                    <input
                      className={styles.input}
                      type="text"
                      placeholder="title（≤ 200 字）"
                      value={draftTitle}
                      onChange={(e) => setDraftTitle(e.target.value)}
                      maxLength={200}
                    />
                    <input
                      className={styles.input}
                      type="text"
                      placeholder="summary（一句话，可选）"
                      value={draftSummary}
                      onChange={(e) => setDraftSummary(e.target.value)}
                    />
                    <textarea
                      className={styles.textarea}
                      placeholder="body：案例正文（≤ 4000 字）"
                      value={draftBody}
                      onChange={(e) => setDraftBody(e.target.value)}
                      rows={6}
                      maxLength={4000}
                    />
                    {promoteError && <div className={styles.inlineError}>{promoteError}</div>}
                    <div className={styles.buttonRow}>
                      <button
                        type="button"
                        className={styles.btnPrimary}
                        onClick={() => void submitPromote()}
                        disabled={busy || !draftTitle.trim() || !draftBody.trim()}
                      >
                        提交晋升
                      </button>
                      <button type="button" className={styles.btnGhost} onClick={closePromote}>
                        取消
                      </button>
                    </div>
                  </div>
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
      <LessonsLearnedAdmin busy={busy} />
    </div>
  );
}
