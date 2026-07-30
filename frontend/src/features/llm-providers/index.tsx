import { useEffect, useRef, useState } from "react";
import {
  Bot,
  Eye,
  EyeOff,
  FlaskConical,
  Plus,
  RefreshCw,
  SquarePen,
  Trash2,
  X
} from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge } from "../../components/ui/StatusBadge";
import { api } from "../../lib/api";
import styles from "./LlmProviders.module.css";

// —— 内部类型（从原 App.tsx 巨壳 inline 迁移，不从 App 导入）——

type LlmProviderItem = {
  providerId: string;
  name: string;
  format: string;
  baseUrl: string;
  apiKeyMasked: string;
  model: string;
  isActive: boolean;
  timeoutSeconds?: number | null;
  effectiveTimeoutSeconds: number;
  timeoutSecondsSource: "provider" | "global_default";
  maxRetries?: number | null;
  effectiveMaxRetries: number;
  maxRetriesSource: "provider" | "global_default";
  retryBaseMs?: number | null;
  effectiveRetryBaseMs: number;
  retryBaseMsSource: "provider" | "global_default";
  supportsVision: boolean;
  isVisionActive: boolean;
  createdAt: number;
  updatedAt: number;
};

type LlmProviderListResponse = {
  items: LlmProviderItem[];
  active: { providerId: string; format: string; model: string; baseUrl: string } | null;
};

type LlmProviderTestResponse = {
  ok: boolean;
  latencyMs: number;
  preview?: unknown;
  error?: { kind?: string; retryCount?: number; detail?: string; hint?: string };
  activeUpdateApproval?: {
    token: string;
    expectedUpdatedAt: number;
    expiresAt: number;
  };
};

// 协议形态的中性判别值（UI 内部用），与后端品牌字面量解耦。
type ProtocolFormat = "chat" | "messages";

type LlmProviderDraft = {
  isNew: boolean;
  isActive: boolean;
  isVisionActive: boolean;
  updatedAt: number | null;
  providerId: string;
  name: string;
  format: ProtocolFormat;
  baseUrl: string;
  apiKey: string;
  model: string;
  timeoutSeconds: string;
  maxRetries: string;
  retryBaseMs: string;
  supportsVision: boolean;
};

// 前后端统一的中性协议 wire 值，与 LLM 品牌完全解耦：
// "chat" = Chat Completions 协议，"messages" = Messages 协议。
// 后端 LlmFormat::parse 接受这两个中性别名并做品牌映射，前端零品牌字面量。
function toWireFormat(format: ProtocolFormat): string {
  return format === "messages" ? "messages" : "chat";
}

function protocolFromWire(raw: string): ProtocolFormat {
  return (raw || "").trim().toLowerCase() === "messages" ? "messages" : "chat";
}

function protocolLabel(format: ProtocolFormat): string {
  return format === "messages" ? "Messages 协议" : "Chat Completions 协议";
}

function valueSourceLabel(source: "provider" | "global_default"): string {
  return source === "provider" ? "Provider 覆盖" : "全局默认";
}

function emptyLlmProviderDraft(): LlmProviderDraft {
  return {
    isNew: true,
    isActive: false,
    isVisionActive: false,
    updatedAt: null,
    providerId: "",
    name: "",
    format: "chat",
    baseUrl: "",
    apiKey: "",
    model: "",
    timeoutSeconds: "",
    maxRetries: "",
    retryBaseMs: "",
    supportsVision: false
  };
}

function draftFromItem(item: LlmProviderItem): LlmProviderDraft {
  return {
    isNew: false,
    isActive: item.isActive,
    isVisionActive: item.isVisionActive,
    updatedAt: item.updatedAt,
    providerId: item.providerId,
    name: item.name,
    format: protocolFromWire(item.format),
    baseUrl: item.baseUrl,
    apiKey: item.apiKeyMasked,
    model: item.model,
    timeoutSeconds: item.timeoutSeconds == null ? "" : String(item.timeoutSeconds),
    maxRetries: item.maxRetries == null ? "" : String(item.maxRetries),
    retryBaseMs: item.retryBaseMs == null ? "" : String(item.retryBaseMs),
    supportsVision: Boolean(item.supportsVision)
  };
}

export default function LlmProvidersFeature() {
  const [items, setItems] = useState<LlmProviderItem[]>([]);
  const [active, setActive] = useState<LlmProviderListResponse["active"]>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<LlmProviderDraft | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<LlmProviderTestResponse | null>(null);
  const [activeUpdateApproval, setActiveUpdateApproval] = useState<
    LlmProviderTestResponse["activeUpdateApproval"] | null
  >(null);
  const [showApiKey, setShowApiKey] = useState(false);
  const draftGenerationRef = useRef(0);

  async function refetch() {
    setLoading(true);
    setError(null);
    try {
      const data = await api.get<LlmProviderListResponse>("/api/admin/llm-providers");
      setItems(data.items || []);
      setActive(data.active || null);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refetch();
  }, []);

  function startCreate() {
    draftGenerationRef.current += 1;
    setDraft(emptyLlmProviderDraft());
    setTestResult(null);
    setActiveUpdateApproval(null);
  }

  function startEdit(item: LlmProviderItem) {
    draftGenerationRef.current += 1;
    setDraft(draftFromItem(item));
    setTestResult(null);
    setActiveUpdateApproval(null);
  }

  function cancelEdit() {
    draftGenerationRef.current += 1;
    setDraft(null);
    setTestResult(null);
    setActiveUpdateApproval(null);
  }

  function changeDraft(patch: Partial<LlmProviderDraft>) {
    if (!draft) return;
    draftGenerationRef.current += 1;
    setDraft({ ...draft, ...patch });
    setTesting(false);
    setTestResult(null);
    setActiveUpdateApproval(null);
  }

  function buildUpsertBody(d: LlmProviderDraft) {
    const body: Record<string, unknown> = {
      providerId: d.providerId.trim(),
      name: d.name.trim() || d.providerId.trim(),
      format: toWireFormat(d.format),
      baseUrl: d.baseUrl.trim(),
      apiKey: d.apiKey,
      model: d.model.trim(),
      supportsVision: d.supportsVision
    };
    if (!d.timeoutSeconds.trim() && !d.isNew) {
      body.timeoutSeconds = null;
    } else if (d.timeoutSeconds.trim()) {
      const v = Number(d.timeoutSeconds);
      if (!Number.isNaN(v) && v > 0) body.timeoutSeconds = Math.floor(v);
    }
    if (!d.maxRetries.trim() && !d.isNew) {
      body.maxRetries = null;
    } else if (d.maxRetries.trim()) {
      const v = Number(d.maxRetries);
      if (!Number.isNaN(v) && v >= 0) body.maxRetries = Math.floor(v);
    }
    if (!d.retryBaseMs.trim() && !d.isNew) {
      body.retryBaseMs = null;
    } else if (d.retryBaseMs.trim()) {
      const v = Number(d.retryBaseMs);
      if (!Number.isNaN(v) && v > 0) body.retryBaseMs = Math.floor(v);
    }
    return body;
  }

  async function saveDraft() {
    if (!draft) return;
    if (!draft.providerId.trim()) {
      window.alert("providerId 不能为空");
      return;
    }
    if (!draft.baseUrl.trim() || !draft.apiKey.trim() || !draft.model.trim()) {
      window.alert("baseUrl / apiKey / model 不能为空");
      return;
    }
    setBusy(true);
    try {
      const body = buildUpsertBody(draft);
      if (draft.isNew) {
        await api.post("/api/admin/llm-providers", body);
      } else {
        if (draft.isActive) {
          if (!activeUpdateApproval || draft.updatedAt !== activeUpdateApproval.expectedUpdatedAt) {
            window.alert("当前激活配置必须先对这份草稿完成连通性测试");
            return;
          }
          if (!window.confirm("确认发布这份已测试配置？保存后将立即热切换全部生产对话。")) {
            return;
          }
          body.expectedUpdatedAt = activeUpdateApproval.expectedUpdatedAt;
          body.activeUpdateConfirmed = true;
          body.activeUpdateTestToken = activeUpdateApproval.token;
        }
        await api.put(`/api/admin/llm-providers/${encodeURIComponent(draft.providerId)}`, body);
      }
      await refetch();
      setDraft(null);
      setTestResult(null);
      setActiveUpdateApproval(null);
    } catch (err) {
      if (draft.isActive) setActiveUpdateApproval(null);
      window.alert(`保存失败：${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function deleteItem(item: LlmProviderItem) {
    if (item.isActive) {
      window.alert("当前激活配置不可删除，请先激活其它供应商");
      return;
    }
    if (item.isVisionActive) {
      window.alert("当前视觉模型不可删除，请先取消视觉指派或改派其它供应商");
      return;
    }
    if (!window.confirm(`确认删除供应商「${item.name || item.providerId}」？`)) return;
    setBusy(true);
    try {
      await api.delete(`/api/admin/llm-providers/${encodeURIComponent(item.providerId)}`);
      await refetch();
      if (draft && !draft.isNew && draft.providerId === item.providerId) {
        setDraft(null);
      }
    } catch (err) {
      window.alert(`删除失败：${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function activateItem(item: LlmProviderItem) {
    if (!window.confirm(`确认激活供应商「${item.name || item.providerId}」？将立即对所有生产对话生效。`)) return;
    setBusy(true);
    try {
      await api.post(`/api/admin/llm-providers/${encodeURIComponent(item.providerId)}/activate`);
      await refetch();
    } catch (err) {
      window.alert(`激活失败：${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  // #574：指派 / 取消本 workspace 专职视觉模型。要求 supportsVision=true。
  async function setVisionItem(item: LlmProviderItem, activeFlag: boolean) {
    if (activeFlag && !item.supportsVision) {
      window.alert("该供应商未勾选「支持图片输入」，请先在编辑里勾选后再指派为视觉模型");
      return;
    }
    if (
      activeFlag &&
      !item.isVisionActive &&
      !window.confirm(`确认将「${item.name || item.providerId}」设为视觉模型？当前视觉指派将被原子替换。`)
    ) {
      return;
    }
    setBusy(true);
    try {
      await api.post(`/api/admin/llm-providers/${encodeURIComponent(item.providerId)}/vision`, {
        active: activeFlag
      });
      await refetch();
    } catch (err) {
      window.alert(`设置视觉模型失败：${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function runTest() {
    if (!draft) return;
    const requestGeneration = draftGenerationRef.current;
    setTestResult(null);
    setActiveUpdateApproval(null);
    setTesting(true);
    try {
      const normalized = buildUpsertBody(draft);
      const body: Record<string, unknown> = {};
      if (draft && !draft.isNew) {
        Object.assign(body, normalized);
        if (draft.apiKey.includes("****")) delete body.apiKey;
      } else if (draft) {
        Object.assign(body, normalized);
      }
      const result = await api.post<LlmProviderTestResponse>(
        "/api/admin/llm-providers/test",
        body
      );
      if (draftGenerationRef.current !== requestGeneration) return;
      setTestResult(result);
      setActiveUpdateApproval(result.ok ? result.activeUpdateApproval ?? null : null);
    } catch (err) {
      if (draftGenerationRef.current !== requestGeneration) return;
      setTestResult({
        ok: false,
        latencyMs: 0,
        error: { kind: "client_error", detail: (err as Error).message }
      });
    } finally {
      if (draftGenerationRef.current === requestGeneration) setTesting(false);
    }
  }

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>LLM Providers</span>
            <span className={styles.title}>模型供应商配置</span>
            <span className={styles.activeHint}>
              {active ? (
                <>
                  当前激活：<strong>{active.providerId}</strong> ·{" "}
                  {protocolLabel(protocolFromWire(active.format))} · <code>{active.model}</code>
                </>
              ) : (
                "尚未加载激活配置"
              )}
            </span>
          </div>
          <div className={styles.headR}>
            <div className={styles.headIcon}>
              <Bot size={17} />
            </div>
            <div className={styles.actions}>
              <button
                type="button"
                className={styles.btnGhost}
                onClick={() => void refetch()}
                disabled={loading}
              >
                <RefreshCw size={14} /> 刷新
              </button>
              <button
                type="button"
                className={styles.btnPrimary}
                onClick={startCreate}
                disabled={busy}
              >
                <Plus size={14} /> 新增供应商
              </button>
            </div>
          </div>
        </div>

        {error && <div className={styles.errorBanner}>{error}</div>}

        {items.length === 0 && !loading ? (
          <EmptyState
            icon={<Bot size={28} />}
            title="暂无供应商配置"
            hint="点击右上「新增供应商」创建第一条，兼容主流 Chat Completions 与 Messages 协议。"
            action={
              <button type="button" className={styles.btnPrimary} onClick={startCreate}>
                <Plus size={14} /> 新增供应商
              </button>
            }
          />
        ) : (
          <div className={styles.list}>
            {items.map((item) => {
              const proto = protocolFromWire(item.format);
              return (
                <article
                  key={item.providerId}
                  className={`${styles.card}${item.isActive ? ` ${styles.cardActive}` : ""}`}
                >
                  <header className={styles.cardHead}>
                    <div className={styles.cardName}>
                      <strong>{item.name || item.providerId}</strong>
                      <code className={styles.providerId}>{item.providerId}</code>
                    </div>
                    <div className={styles.badges}>
                      <span className={styles.protoBadge}>{protocolLabel(proto)}</span>
                      {item.isActive && <StatusBadge tone="running">已激活</StatusBadge>}
                      {item.supportsVision && <span className={styles.capChip}>支持图片</span>}
                      {item.isVisionActive && <StatusBadge tone="scheduled">视觉模型</StatusBadge>}
                    </div>
                  </header>

                  <dl className={styles.meta}>
                    <div className={styles.metaRow}>
                      <dt>baseUrl</dt>
                      <dd className={styles.mono}>{item.baseUrl}</dd>
                    </div>
                    <div className={styles.metaRow}>
                      <dt>model</dt>
                      <dd className={styles.mono}>{item.model}</dd>
                    </div>
                    <div className={styles.metaRow}>
                      <dt>apiKey</dt>
                      <dd className={styles.mono}>{item.apiKeyMasked}</dd>
                    </div>
                    <div className={styles.metaRow}>
                      <dt>超时 / 重试</dt>
                      <dd>
                        {item.effectiveTimeoutSeconds}s（{valueSourceLabel(item.timeoutSecondsSource)}） · 重试{" "}
                        {item.effectiveMaxRetries} 次（{valueSourceLabel(item.maxRetriesSource)}） · 退避基线{" "}
                        {item.effectiveRetryBaseMs}ms（{valueSourceLabel(item.retryBaseMsSource)}）
                      </dd>
                    </div>
                  </dl>

                  <footer className={styles.cardFoot}>
                    {!item.isActive && (
                      <button
                        type="button"
                        className={styles.btnGhost}
                        onClick={() => void activateItem(item)}
                        disabled={busy}
                      >
                        激活
                      </button>
                    )}
                    {item.supportsVision && !item.isVisionActive && (
                      <button
                        type="button"
                        className={styles.btnGhost}
                        onClick={() => void setVisionItem(item, true)}
                        disabled={busy}
                        title="指派为本 workspace 处理图片的专职视觉模型"
                      >
                        设为视觉模型
                      </button>
                    )}
                    {item.isVisionActive && (
                      <button
                        type="button"
                        className={styles.btnGhost}
                        onClick={() => void setVisionItem(item, false)}
                        disabled={busy}
                        title="取消视觉模型指派"
                      >
                        取消视觉模型
                      </button>
                    )}
                    <button
                      type="button"
                      className={styles.btnGhost}
                      onClick={() => startEdit(item)}
                      disabled={busy}
                    >
                      <SquarePen size={13} /> 编辑
                    </button>
                    <button
                      type="button"
                      className={styles.btnDanger}
                      onClick={() => void deleteItem(item)}
                      disabled={busy || item.isActive || item.isVisionActive}
                      title={
                        item.isActive
                          ? "请先激活其它供应商后再删除"
                          : item.isVisionActive
                            ? "请先取消视觉指派或改派其它供应商"
                            : "删除"
                      }
                    >
                      <Trash2 size={13} /> 删除
                    </button>
                  </footer>
                </article>
              );
            })}
          </div>
        )}
      </section>

      {draft && (
        <section className={styles.panel}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>
                {draft.isNew ? "新增供应商" : "编辑供应商"}
              </span>
              <span className={styles.title}>
                {draft.isNew ? "新增模型供应商" : draft.name || draft.providerId}
              </span>
              {!draft.isNew && (
                <span className={styles.activeHint}>
                  <code>{draft.providerId}</code> · {protocolLabel(draft.format)}
                </span>
              )}
            </div>
            <button
              type="button"
              className={styles.btnGhost}
              onClick={cancelEdit}
              disabled={busy}
            >
              <X size={14} /> 关闭
            </button>
          </div>

          <div className={styles.sectionTitle}>协议格式</div>
          <div className={styles.protoGrid}>
            <button
              type="button"
              className={`${styles.protoCard}${draft.format === "chat" ? ` ${styles.protoCardSelected}` : ""}`}
              onClick={() => changeDraft({ format: "chat" })}
              disabled={busy}
            >
              <div className={styles.protoTitle}>Chat Completions 协议</div>
              <div className={styles.protoMeta}>POST /chat/completions · Authorization: Bearer</div>
              <div className={styles.protoSub}>兼容 Chat Completions 协议形态的服务商或自建网关</div>
            </button>
            <button
              type="button"
              className={`${styles.protoCard}${draft.format === "messages" ? ` ${styles.protoCardSelected}` : ""}`}
              onClick={() => changeDraft({ format: "messages" })}
              disabled={busy}
            >
              <div className={styles.protoTitle}>Messages 协议</div>
              <div className={styles.protoMeta}>POST /v1/messages · x-api-key</div>
              <div className={styles.protoSub}>兼容 Messages 协议形态的服务商或自建网关</div>
            </button>
          </div>

          <div className={styles.sectionTitle}>基本信息</div>
          <div className={styles.formGrid}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>供应商标识 (providerId)</span>
              <input
                className={styles.input}
                value={draft.providerId}
                onChange={(e) => changeDraft({ providerId: e.target.value })}
                disabled={!draft.isNew || busy}
                placeholder="如 my-llm-prod / gateway-a"
              />
              <small className={styles.fieldHint}>唯一 slug，保存后不可修改</small>
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>展示名称</span>
              <input
                className={styles.input}
                value={draft.name}
                onChange={(e) => changeDraft({ name: e.target.value })}
                disabled={busy}
                placeholder="便于识别的展示名"
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>model</span>
              <input
                className={styles.input}
                value={draft.model}
                onChange={(e) => changeDraft({ model: e.target.value })}
                disabled={busy}
                placeholder={
                  draft.format === "messages"
                    ? "请填写 Messages 协议形态的模型 ID"
                    : "请填写 Chat Completions 协议形态的模型 ID"
                }
              />
            </label>
          </div>

          <div className={styles.sectionTitle}>连接配置</div>
          <div className={styles.formGrid}>
            <label className={`${styles.field} ${styles.spanFull}`}>
              <span className={styles.fieldLabel}>baseUrl</span>
              <input
                className={styles.input}
                value={draft.baseUrl}
                onChange={(e) => changeDraft({ baseUrl: e.target.value })}
                disabled={busy}
                placeholder={
                  draft.format === "messages"
                    ? "https://api.your-provider.com"
                    : "https://api.your-provider.com/v1"
                }
              />
              <small className={styles.fieldHint}>
                {draft.format === "messages" ? (
                  <>
                    系统会向 <code>baseUrl + /v1/messages</code> 发请求。请填到根域，<strong>不要</strong>带 <code>/v1</code>（如 <code>https://api.your-provider.com</code>）。
                  </>
                ) : (
                  <>
                    系统会向 <code>baseUrl + /chat/completions</code> 发请求，<strong>不会自动补任何路径</strong>。请直接粘贴服务商文档里的「OpenAI 兼容 base_url」原文，末尾不带斜杠。常见两种形态：
                    <br />· 带版本号路径：<code>https://api.your-provider.com/v1</code>
                    <br />· 带兼容模式路径：<code>https://api.your-provider.com/compatible-mode/v1</code>
                  </>
                )}
              </small>
            </label>
            <label className={`${styles.field} ${styles.spanFull}`}>
              <span className={styles.fieldLabel}>apiKey</span>
              <div className={styles.inputWrap}>
                <input
                  className={styles.input}
                  type={showApiKey ? "text" : "password"}
                  value={draft.apiKey}
                  onChange={(e) => changeDraft({ apiKey: e.target.value })}
                  disabled={busy}
                  placeholder={draft.isNew ? "请填写 apiKey" : "保留 mask 占位则不更新"}
                  autoComplete="new-password"
                />
                <button
                  type="button"
                  className={styles.iconBtn}
                  onClick={() => setShowApiKey((v) => !v)}
                  disabled={busy}
                  aria-label={showApiKey ? "隐藏 apiKey" : "显示 apiKey"}
                  title={showApiKey ? "隐藏" : "显示"}
                >
                  {showApiKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
              <small className={styles.fieldHint}>
                编辑模式下若不修改请保持「****」mask 占位，提交时不会覆盖原 key
              </small>
            </label>
          </div>

          <div className={styles.sectionTitle}>重试与超时</div>
          <div className={`${styles.formGrid} ${styles.formGridThree}`}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>超时秒数</span>
              <input
                className={styles.input}
                type="number"
                min={1}
                value={draft.timeoutSeconds}
                onChange={(e) => changeDraft({ timeoutSeconds: e.target.value })}
                disabled={busy}
                placeholder="留空则使用全局默认"
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>最大重试</span>
              <input
                className={styles.input}
                type="number"
                min={0}
                value={draft.maxRetries}
                onChange={(e) => changeDraft({ maxRetries: e.target.value })}
                disabled={busy}
                placeholder="留空则使用全局默认"
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>重试退避基线 (ms)</span>
              <input
                className={styles.input}
                type="number"
                min={1}
                value={draft.retryBaseMs}
                onChange={(e) => changeDraft({ retryBaseMs: e.target.value })}
                disabled={busy}
                placeholder="留空则使用全局默认"
              />
            </label>
          </div>

          <div className={styles.sectionTitle}>多模态能力</div>
          <div className={styles.formGrid}>
            <label className={`${styles.field} ${styles.spanFull} ${styles.checkboxRow}`}>
              <input
                className={styles.checkbox}
                type="checkbox"
                checked={draft.supportsVision}
                onChange={(e) => {
                  if (draft.isVisionActive && !e.target.checked) {
                    window.alert("当前供应商仍是视觉模型，请先取消视觉指派或改派其它供应商");
                    return;
                  }
                  changeDraft({ supportsVision: e.target.checked });
                }}
                disabled={busy}
              />
              <span>支持图片输入（多模态视觉）</span>
            </label>
            <small className={`${styles.fieldHint} ${styles.spanFull}`}>
              勾选后该模型可识别图片。若文字主模型不支持图片，可单独配置一条支持图片的模型，保存后在卡片上「设为视觉模型」——图片导入会自动路由到该视觉模型；否则图片导入会因缺少可用视觉模型而失败。
            </small>
          </div>

          <div className={styles.editorFoot}>
            <button
              type="button"
              className={styles.btnGhost}
              onClick={() => void runTest()}
              disabled={testing || busy}
            >
              <FlaskConical size={14} /> {testing ? "测试中…" : "测试连通性"}
            </button>
            <div className={styles.footSpacer} />
            <button
              type="button"
              className={styles.btnGhost}
              onClick={cancelEdit}
              disabled={busy}
            >
              取消
            </button>
            <button
              type="button"
              className={styles.btnPrimary}
              onClick={() => void saveDraft()}
              disabled={busy || (draft.isActive && !activeUpdateApproval)}
              title={
                draft.isActive && !activeUpdateApproval
                  ? "请先对当前草稿完成连通性测试"
                  : undefined
              }
            >
              {draft.isNew ? "创建" : draft.isActive ? "确认发布" : "保存"}
            </button>
          </div>

          {testResult && (
            <div
              className={`${styles.testResult} ${testResult.ok ? styles.testOk : styles.testFail}`}
            >
              <div className={styles.testHead}>
                <strong>{testResult.ok ? "测试成功" : "测试失败"}</strong>
                <span>耗时 {testResult.latencyMs} ms</span>
              </div>
              {testResult.ok ? (
                <div className={styles.testBody}>
                  {draft.isActive && activeUpdateApproval && (
                    <div>该测试结果可用于一次“确认发布”，修改任何字段后需重新测试。</div>
                  )}
                  <pre>
                    {typeof testResult.preview === "string"
                      ? testResult.preview
                      : JSON.stringify(testResult.preview, null, 2)}
                  </pre>
                </div>
              ) : (
                <div className={styles.testBody}>
                  <div>
                    <em>错误类型：</em>
                    {testResult.error?.kind || "unknown"}
                    {testResult.error?.retryCount != null && (
                      <span> · 重试 {testResult.error.retryCount} 次</span>
                    )}
                  </div>
                  {testResult.error?.detail && <pre>{testResult.error.detail}</pre>}
                  {testResult.error?.hint && (
                    <div className={styles.testHintLine}>建议：{testResult.error.hint}</div>
                  )}
                </div>
              )}
            </div>
          )}
        </section>
      )}
    </div>
  );
}
