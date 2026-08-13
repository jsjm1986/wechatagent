import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import {
  Archive,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Eye,
  GitMerge,
  History,
  Link2,
  Scissors,
  SquarePen,
  Undo2,
  X,
} from "lucide-react";
import { api, parseApiError } from "../../lib/api";
export { LlmErrorBanner } from "../../components/LlmErrorBanner";
import { useConfirm } from "../../components/ui/ConfirmDialog";
import { useFormDialog } from "../../components/ui/FormDialog";
import { useToast } from "../../components/ui/Toast";
import type { PickerChunk } from "../../components/ui/ChunkRef";
import { type TrustChunkFields, chunkTypeLabel } from "./trustTypes";
import { wikiTypeLabel, statusLabel, integrityStatusLabel, revisionOpLabel, revisionSourceLabel, relatedKindLabel } from "./labels";
import { ChunkRepairPanel } from "./ChunkRepairPanel";
import {
  CHUNKS_INVALIDATED_EVENT,
  invalidateChunks,
  useCoalescedReload,
  type ChunkInvalidationDetail,
} from "./chunkInvalidation";
import {
  chunkMergeRequest,
  chunkPatchRequest,
  chunkRelateRequest,
  chunkSplitRequest,
  type ChunkRelationKind,
} from "./chunkActionContracts";

/// ChunkPicker 列表加载器:拉全部 chunk 供搜索选择,替代手输 ObjectId。
/// 模块级缓存 20s,避免多个选择器/Inspector 首次聚焦各拉一次全量。
let chunkOptionsCache: { at: number; items: PickerChunk[] } | null = null;
const CHUNK_OPTIONS_TTL_MS = 20_000;
export async function loadChunkOptions(): Promise<PickerChunk[]> {
  if (chunkOptionsCache && Date.now() - chunkOptionsCache.at < CHUNK_OPTIONS_TTL_MS) {
    return chunkOptionsCache.items;
  }
  try {
    const r = await fetch("/api/operation-knowledge/chunks");
    if (!r.ok) return chunkOptionsCache?.items ?? [];
    const data = (await r.json()) as { items?: { id: string; title?: string | null }[] };
    const items = (data.items ?? []).map((c) => ({ id: c.id, title: c.title }));
    chunkOptionsCache = { at: Date.now(), items };
    return items;
  } catch {
    return chunkOptionsCache?.items ?? [];
  }
}

export type ReviewCategory =
  | "contested"
  | "needs_review"
  | "source_orphan"
  | "pending_verification"
  | "dependents_pending";

export interface ReviewChunkItem extends TrustChunkFields {
  id: string;
  workspaceId?: string;
  accountId?: string | null;
  title: string;
  summary?: string | null;
  body?: string | null;
  sourceQuote?: string | null;
  sourceAnchors?: unknown[] | null;
  integrityStatus?: string | null;
  status?: string | null;
  wikiType?: string | null;
  businessTopics?: string[] | null;
  relatedChunks?: { chunk_id: string; kind: string; note?: string | null }[] | null;
  supersededBy?: string | null;
  previousVersionId?: string | null;
  updatedAt?: string | null;
}

// ChunkInspectorPane：Explore 第三栏。监听 wikiFocusChunk 事件 → 拉单 chunk
// 详情。lazy-load：首次聚焦才发起 list 请求；之后从本地 indexById 直接命中。
export function ChunkInspectorPane({
  chunkId,
  onClose,
  onClear,
}: {
  chunkId: string | null;
  onClose: () => void;
  onClear: () => void;
}) {
  const [items, setItems] = useState<TreeChunkItem[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [syncNotice, setSyncNotice] = useState<string | null>(null);
  const requestGeneration = useRef(0);
  const lock = useChunkInspectorLock(chunkId);

  const loadOnce = useCallback(async () => {
    if (!chunkId) return;
    const generation = requestGeneration.current;
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/chunks");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: TreeChunkItem[] };
      if (generation === requestGeneration.current) setItems(data.items ?? []);
    } catch (e: unknown) {
      if (generation === requestGeneration.current) {
        setItems(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (generation === requestGeneration.current) setLoading(false);
    }
  }, [chunkId]);
  const coalescedReload = useCoalescedReload(loadOnce);
  const reload = useCallback(() => {
    requestGeneration.current += 1;
    setItems(null);
    setError(null);
    return coalescedReload();
  }, [coalescedReload]);

  useEffect(() => {
    if (!chunkId) {
      requestGeneration.current += 1;
      setItems(null);
      setLoading(false);
      setError(null);
      return;
    }
    void reload();
  }, [chunkId, reload]);
  const confirm = useConfirm();
  const toast = useToast();
  const [unrelating, setUnrelating] = useState<string | null>(null);

  // E5：解除当前 chunk 指向某目标的关联（DELETE /relate/:target_id 只删关联不删 chunk）。
  // :id 是源 chunk.id，:target_id 是 related 项的 chunk_id。成功后 reload 刷新关联列表。
  async function onUnrelate(targetId: string, label: string) {
    const ok = await confirm({
      title: "解除这条知识关联？",
      body: `将移除当前知识指向「${label}」的关联，目标知识本身不受影响。`,
      tone: "danger",
      confirmText: "确认解除",
    });
    if (!ok) return;
    setUnrelating(targetId);
    try {
      await api.delete(
        `/api/operation-knowledge/chunks/${encodeURIComponent(chunkId!)}/relate/${encodeURIComponent(targetId)}`,
      );
      toast.success("已解除关联");
      invalidateChunks({ reason: "local", chunkId: chunkId! });
    } catch (e: unknown) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setUnrelating(null);
    }
  }

  useEffect(() => {
    if (!chunkId) return;
    const onInvalidated = (event: Event) => {
      const detail = (event as CustomEvent<ChunkInvalidationDetail>).detail;
      if (detail?.reason === "lagged") {
        setSyncNotice("\u5b9e\u65f6\u66f4\u65b0\u6709\u79ef\u538b\uff0c\u6b63\u5728\u91cd\u65b0\u540c\u6b65\u77e5\u8bc6\u8be6\u60c5\u2026");
      }
      void reload().finally(() => setSyncNotice(null));
    };
    window.addEventListener(CHUNKS_INVALIDATED_EVENT, onInvalidated);
    return () => window.removeEventListener(CHUNKS_INVALIDATED_EVENT, onInvalidated);
  }, [chunkId, reload]);

  const indexById = useMemo(() => {
    const m = new Map<string, TreeChunkItem>();
    if (items) for (const it of items) m.set(it.id, it);
    return m;
  }, [items]);

  const chunk = chunkId ? indexById.get(chunkId) ?? null : null;
  const anchors = useMemo(() => {
    if (!chunk?.sourceAnchors) return [] as Record<string, unknown>[];
    return chunk.sourceAnchors as Record<string, unknown>[];
  }, [chunk]);
  const related = useMemo(() => {
    if (!chunk?.relatedChunks) return [] as { chunk_id: string; kind: string; note?: string | null }[];
    return chunk.relatedChunks;
  }, [chunk]);
  const hasQuote = !!chunk?.sourceQuote;

  return (
    <aside className="wikiInspectorPane wikiModePane--side">
      <header className="wikiInspectorHead">
        <div className="wikiInspectorTitle">
          <Eye size={14} /> 详情
        </div>
        <div style={{ display: "flex", gap: 4 }}>
          {chunk ? (
            <button
              type="button"
              className="wikiInspectorClose"
              onClick={onClear}
              title="清空选中 chunk"
            >
              清空
            </button>
          ) : null}
          <button
            type="button"
            className="wikiInspectorClose"
            onClick={onClose}
            title="收起 Inspector"
          >
            <ChevronRight size={14} />
          </button>
        </div>
      </header>
      <div className="wikiInspectorBody">
        {syncNotice ? <div className="wikiAlert info">{syncNotice}</div> : null}
        {chunkId ? <ChunkLockBadge lock={lock} /> : null}
        {!chunkId ? (
          <div className="wikiInspectorEmpty">
            点击左侧树节点或问答中的引用 chunk，详情会出现在这里。
          </div>
        ) : loading ? (
          <div className="wikiInspectorEmpty">加载中…</div>
        ) : error ? (
          <div className="wikiAlert error">{error}</div>
        ) : !chunk ? (
          <div className="wikiInspectorEmpty">
            未找到知识条目 <code>{chunkId}</code>，可能已归档或不在当前工作区。
          </div>
        ) : (
          <>
            {chunk.supersededBy ? (() => {
              const successor = indexById.get(chunk.supersededBy!);
              return (
                <div className="wikiArchiveRedirect">
                  <span className="wikiArchiveRedirectLabel">已被替代</span>
                  <span className="wikiArchiveRedirectTitle">
                    {successor ? successor.title : <code>{chunk.supersededBy}</code>}
                  </span>
                  <button
                    type="button"
                    className="wikiArchiveRedirectBtn"
                    disabled={!successor}
                    onClick={() => focusChunk(chunk.supersededBy!)}
                    title={successor ? "跳转到新版本" : "目标 chunk 不在活跃集合"}
                  >
                    跳转 →
                  </button>
                </div>
              );
            })() : null}
            <dl className="wikiArchiveMeta">
              <dt>状态</dt>
              <dd>
                <span className={`wikiSev ${chunk.integrityStatus === "rejected" ? "error" : "info"}`}>
                  {integrityStatusLabel(chunk.integrityStatus ?? undefined)}
                </span>{" "}
                <span className="wikiBadge">{statusLabel(chunk.status ?? undefined)}</span>
              </dd>
              <dt>编号</dt>
              <dd><code>{chunk.id}</code></dd>
              {chunk.wikiType ? (<><dt>知识类型</dt><dd><span className="wikiArchiveTag">{wikiTypeLabel(chunk.wikiType)}</span></dd></>) : null}
              {chunkTypeLabel(chunk.chunkType) ? (<><dt>运营用途</dt><dd><span className="wikiArchiveTag">{chunkTypeLabel(chunk.chunkType)}</span></dd></>) : null}
              {Array.isArray(chunk.businessTopics) && chunk.businessTopics.length > 0 ? (
                <>
                  <dt>业务主题</dt>
                  <dd>{chunk.businessTopics.map((t, i) => <span key={i} className="wikiArchiveTag">{t}</span>)}</dd>
                </>
              ) : null}
              {chunk.previousVersionId ? (() => {
                const prev = indexById.get(chunk.previousVersionId!);
                return (
                  <>
                    <dt>上一版本</dt>
                    <dd>
                      <button
                        type="button"
                        className="wikiRelatedChip"
                        disabled={!prev}
                        onClick={() => focusChunk(chunk.previousVersionId!)}
                        title={prev ? "跳转到上一版本" : "目标 chunk 不在活跃集合"}
                      >
                        <span className="wikiRelatedKind">上一版</span>
                        <span className="wikiRelatedTitle">{prev ? prev.title : chunk.previousVersionId}</span>
                      </button>
                    </dd>
                  </>
                );
              })() : null}
              {chunk.provenance ? (
                <>
                  <dt>来源</dt>
                  <dd className="wikiProvenance">
                    {chunk.provenance.source ? <span className="wikiArchiveTag">{chunk.provenance.source}</span> : null}
                    {chunk.provenance.editedBy ? <span className="wikiProvBy">编辑者：{chunk.provenance.editedBy}</span> : null}
                    {chunk.provenance.llmModelAlias ? <span className="wikiProvModel">{chunk.provenance.llmModelAlias}</span> : null}
                  </dd>
                </>
              ) : null}
            </dl>
            <hr className="wikiArchiveRule" />
            <h3 className="wikiInspectorChunkTitle">{chunk.title || "（无标题）"}</h3>
            {chunk.summary ? <p className="wikiInspectorSummary">{chunk.summary}</p> : null}
            {hasQuote ? (
              <blockquote className="wikiArchiveCitation">
                {chunk.sourceQuote}
                <span className="wikiArchiveCitationSource">
                  {chunk.id}
                  {anchors.length > 0 ? ` · L${numberOr(anchors[0]["startLine"]) ?? "?"}-${numberOr(anchors[0]["endLine"]) ?? "?"}` : ""}
                </span>
              </blockquote>
            ) : (
              <div className="wikiHint">无原文引用 — 该知识片段不可核验。</div>
            )}
            {anchors.length > 0 ? (
              <section className="wikiInspectorSection">
                <div className="wikiInspectorSectionTitle">原文定位（{anchors.length}）</div>
                <div className="wikiSourceAnchorList">
                  {anchors.map((a, i) => {
                    const sl = numberOr(a["startLine"]);
                    const el = numberOr(a["endLine"]);
                    const hash = stringOr(a["quoteHash"]);
                    return (
                      <span key={`${chunk.id}-ia-${i}`} className="wikiSourceAnchor">
                        <span className="wikiSourceAnchorRange">L{sl}-L{el}</span>
                        {hash ? (
                          <code className="wikiSourceAnchorHash">{hash.slice(0, 12)}…</code>
                        ) : null}
                      </span>
                    );
                  })}
                </div>
              </section>
            ) : null}
            {related.length > 0 ? (
              <section className="wikiInspectorSection">
                <div className="wikiInspectorSectionTitle">关联知识（{related.length}）</div>
                <div className="wikiRelatedList">
                  {related.map((r, i) => {
                    const target = indexById.get(r.chunk_id);
                    const dead = !target;
                    const label = target ? target.title : r.chunk_id;
                    return (
                      <div
                        key={`${chunk.id}-irel-${i}`}
                        className={`wikiRelatedChipWrap ${dead ? "dead" : ""}`}
                        title={dead ? "目标 chunk 不在活跃集合" : r.note ?? ""}
                      >
                        <button
                          type="button"
                          className="wikiRelatedJump"
                          disabled={dead}
                          onClick={() => focusChunk(r.chunk_id)}
                        >
                          <span className="wikiRelatedKind">{relatedKindLabel(r.kind)}</span>
                          <span className="wikiRelatedTitle">{label}</span>
                        </button>
                        <button
                          type="button"
                          className="wikiRelatedUnrelate"
                          disabled={unrelating === r.chunk_id}
                          onClick={() => onUnrelate(r.chunk_id, label)}
                          title="解除关联"
                          aria-label="解除关联"
                        >
                          {unrelating === r.chunk_id ? "…" : "解除"}
                        </button>
                      </div>
                    );
                  })}
                </div>
              </section>
            ) : null}
            {chunk.body ? (
              <section className="wikiInspectorSection">
                <div className="wikiInspectorSectionTitle">正文</div>
                <pre>{chunk.body}</pre>
              </section>
            ) : null}
            {chunk.integrityStatus === "needs_review" ? (
              <ChunkRepairPanel
                chunkId={chunk.id}
                originalChunk={chunk as unknown as Record<string, unknown>}
                onApplied={() => {
                  invalidateChunks({ reason: "local", chunkId: chunk.id });
                }}
              />
            ) : null}
            <ChunkActionsBar
              chunk={chunk}
              onChanged={() => invalidateChunks({ reason: "local", chunkId: chunk.id })}
              presenceByOther={lock.state === "other"}
            />
            <ChunkReferrersList chunkId={chunk.id} />
            <ChunkSourceSection chunkId={chunk.id} />
            <ChunkRevisionsTimeline
              chunkId={chunk.id}
              onRolledBack={() => invalidateChunks({ reason: "local", chunkId: chunk.id })}
            />
          </>
        )}
      </div>
    </aside>
  );
}

// ChunkSourceSection：调 GET /api/operation-knowledge/chunks/:id/source，
// 折叠加载父文档 raw_content + chunk source_anchors 范围。后端已存在
// 但前端未挂；这里 lazy-load，默认折叠避免大文档把 Inspector 撑爆。
function ChunkSourceSection({ chunkId }: { chunkId: string }) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<{
    document?: { id?: string; title?: string; rawContent?: string | null } | null;
    chunk?: { sourceAnchors?: Record<string, unknown>[] } | null;
  } | null>(null);

  async function expand() {
    if (open) {
      setOpen(false);
      return;
    }
    setOpen(true);
    if (data) return;
    setLoading(true);
    setError(null);
    try {
      const r = await fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/source`);
      if (!r.ok) throw await parseApiError(r);
      const body = (await r.json()) as typeof data;
      setData(body);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  const raw = data?.document?.rawContent ?? "";
  // 截 8KB 防止 5MB 整本手册一次塞 DOM。
  const truncated = raw.length > 8000;
  const display = truncated ? raw.slice(0, 8000) + "\n…（已截断 " + (raw.length - 8000) + " 字符）" : raw;
  const anchors = (data?.chunk?.sourceAnchors ?? []) as Record<string, unknown>[];
  const ranges = anchors
    .map((a) => {
      const sl = numberOr(a["startLine"]);
      const el = numberOr(a["endLine"]);
      return sl != null && el != null ? `L${sl}-L${el}` : null;
    })
    .filter((s): s is string => !!s);

  return (
    <section className="wikiInspectorSection">
      <button
        type="button"
        className="wikiInspectorSectionTitle"
        style={{ display: "flex", alignItems: "center", gap: 6, background: "none", border: 0, padding: 0, cursor: "pointer", width: "100%" }}
        onClick={() => void expand()}
        aria-expanded={open}
      >
        <span>{open ? "▾" : "▸"}</span>
        <span>原文</span>
        <span style={{ marginLeft: "auto", fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--muted)" }}>
          {data ? (data.document ? `${ranges.join(" / ") || "—"}` : "无父文档") : ""}
        </span>
      </button>
      {open ? (
        loading ? (
          <div className="wikiHint">正在拉父文档…</div>
        ) : error ? (
          <div className="wikiAlert error">{error}</div>
        ) : !data?.document ? (
          <div className="wikiHint">该 chunk 无父文档，无法回看 raw_content。</div>
        ) : (
          <>
            <div style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)", margin: "4px 0 8px" }}>
              {data.document.title ?? "（无标题文档）"} · {raw.length} chars
            </div>
            <pre
              style={{
                maxHeight: 400,
                overflow: "auto",
                fontFamily: "var(--font-mono)",
                fontSize: 12,
                lineHeight: 1.55,
                background: "var(--surface-2, #f4efe5)",
                padding: 10,
                border: "1px solid var(--line)",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
              }}
            >
              {display}
            </pre>
            {truncated ? (
              <div className="wikiHint">原文超过 8KB，已截断展示。完整内容仍存在后端。</div>
            ) : null}
          </>
        )
      ) : null}
    </section>
  );
}

// 全局事件桥：发布"打开 chunk Inspector"，AskView / KnowledgeTreeView 调用，
// ExploreMode / ChunkInspectorPane 监听。
export function focusChunk(chunkId: string) {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent("wikiFocusChunk", { detail: { chunkId } }));
}

// ── P1-4 · WebSocket 协作 presence + 事件总线 ──────────────────────────────
//
// Presence 状态机：
//   - 'idle' 初始；
//   - 'self' 当前 admin 已登记 presence，60s 心跳续期；
//   - 'other' 他人也在查看/编辑（409 返回 presence 信息）；
//   - 'error' 网络错或 5xx，仅失去协作提示，不影响 mutation。
type LockHolder = {
  ownerUserId: string;
  ownerUsername: string;
  expiresAt: string;
};

type ChunkLockState =
  | { state: "idle" }
  | { state: "self"; holder: LockHolder }
  | { state: "other"; holder: LockHolder }
  | { state: "error"; reason: string };

function formatLockExpiry(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

function ChunkLockBadge({ lock }: { lock: ChunkLockState }) {
  if (lock.state === "idle") return null;
  if (lock.state === "self") {
    const at = formatLockExpiry(lock.holder.expiresAt);
    return (
      <div className="wikiInspectorLockBadge wikiInspectorLockBadge--self" role="status">
        <span className="wikiInspectorLockDot" aria-hidden />
        <span>已发送协作提示{at ? ` · 自动续期至 ${at}` : ""}</span>
      </div>
    );
  }
  if (lock.state === "other") {
    const at = formatLockExpiry(lock.holder.expiresAt);
    const who = lock.holder.ownerUsername || lock.holder.ownerUserId || "其他 admin";
    return (
      <div className="wikiInspectorLockBadge wikiInspectorLockBadge--other" role="status">
        <span className="wikiInspectorLockDot" aria-hidden />
        <span>由 {who} 查看或编辑中{at ? `（至 ${at}）` : ""} · 仅提示，不阻止提交</span>
      </div>
    );
  }
  return (
    <div className="wikiInspectorLockBadge wikiInspectorLockBadge--error" role="status">
      <span className="wikiInspectorLockDot" aria-hidden />
      <span>协作提示不可用 · {lock.reason}（不影响提交）</span>
    </div>
  );
}

function useChunkInspectorLock(chunkId: string | null): ChunkLockState {
  const [lock, setLock] = useState<ChunkLockState>({ state: "idle" });

  useEffect(() => {
    if (!chunkId) {
      setLock({ state: "idle" });
      return;
    }
    let cancelled = false;
    let heartbeat: number | null = null;

    const acquire = async () => {
      try {
        const r = await fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/lock`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
        });
        if (cancelled) return;
        const body = await r.json().catch(() => ({}) as Record<string, unknown>);
        if (r.status === 409) {
          const lk = (body as { lock?: { owner_user_id?: string; owner_username?: string; expires_at?: string } }).lock;
          if (lk) {
            setLock({
              state: "other",
              holder: {
                ownerUserId: lk.owner_user_id ?? "",
                ownerUsername: lk.owner_username ?? "",
                expiresAt: lk.expires_at ?? "",
              },
            });
          } else {
            setLock({ state: "error", reason: "lock_conflict_no_payload" });
          }
          return;
        }
        if (!r.ok) {
          setLock({ state: "error", reason: `http_${r.status}` });
          return;
        }
        const lk = (body as { lock?: { owner_user_id?: string; owner_username?: string; expires_at?: string } }).lock;
        if (!lk) {
          setLock({ state: "error", reason: "missing_lock_payload" });
          return;
        }
        setLock({
          state: "self",
          holder: {
            ownerUserId: lk.owner_user_id ?? "",
            ownerUsername: lk.owner_username ?? "",
            expiresAt: lk.expires_at ?? "",
          },
        });
      } catch (e) {
        if (!cancelled) setLock({ state: "error", reason: String(e) });
      }
    };

    void acquire();
    // 60s 心跳：再 POST 一次相当于续期
    heartbeat = window.setInterval(() => {
      void acquire();
    }, 60000);

    // WebSocket 推 unlocked 时刷新一次 presence（不影响 mutation 权限）。
    const onUnlocked = (e: Event) => {
      const detail = (e as CustomEvent<{ chunk_id?: string }>).detail;
      if (detail?.chunk_id === chunkId) {
        void acquire();
      }
    };
    const onLocked = (e: Event) => {
      const detail = (e as CustomEvent<{ chunk_id?: string; owner_user_id?: string; owner_username?: string; expires_at?: string }>).detail;
      if (detail?.chunk_id === chunkId) {
        // 别人登记 presence——只有不是我自己时才覆盖。
        setLock((prev) => {
          if (prev.state === "self" && prev.holder.ownerUserId === detail.owner_user_id) {
            return prev;
          }
          return {
            state: "other",
            holder: {
              ownerUserId: detail.owner_user_id ?? "",
              ownerUsername: detail.owner_username ?? "",
              expiresAt: detail.expires_at ?? "",
            },
          };
        });
      }
    };
    window.addEventListener("wikiChunkUnlocked", onUnlocked);
    window.addEventListener("wikiChunkLocked", onLocked);

    return () => {
      cancelled = true;
      if (heartbeat != null) window.clearInterval(heartbeat);
      window.removeEventListener("wikiChunkUnlocked", onUnlocked);
      window.removeEventListener("wikiChunkLocked", onLocked);
      // best-effort release：unmount / 切 chunk 时把锁还回去
      void fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/lock`, {
        method: "DELETE",
      }).catch(() => undefined);
    };
  }, [chunkId]);

  return lock;
}

// ── G3 · ChunkActionsBar：9 类编辑动作（admin 手工触发） ───────────────────
// 路由全部为 /api/operation-knowledge/chunks/:id/<action>。AI 永不自动 verify。
type ChunkActionState = { busy: string | null; error: string | null; info: string | null };

function ChunkActionsBar({
  chunk,
  onChanged,
  presenceByOther,
}: {
  chunk: TreeChunkItem;
  onChanged: () => void;
  presenceByOther?: boolean;
}) {
  const [state, setState] = useState<ChunkActionState>({ busy: null, error: null, info: null });
  const confirm = useConfirm();
  const form = useFormDialog();
  const toast = useToast();

  async function call(
    action: string,
    method: "POST" | "DELETE",
    path: string,
    body?: Record<string, unknown>,
  ) {
    setState({ busy: action, error: null, info: null });
    try {
      const init: RequestInit = { method, headers: { "Content-Type": "application/json" } };
      if (body !== undefined) init.body = JSON.stringify(body);
      const r = await fetch(path, init);
      if (!r.ok) throw await parseApiError(r);
      setState({ busy: null, error: null, info: null });
      toast.success(`已${action}`);
      onChanged();
    } catch (e: unknown) {
      setState({ busy: null, error: e instanceof Error ? e.message : String(e), info: null });
    }
  }

  const id = encodeURIComponent(chunk.id);
  const isArchived = chunk.status === "archived";
  const isVerified = chunk.integrityStatus === "verified";
  // Presence 只作协作提示；真正并发保护由后端 transaction + CAS 提供。
  const writeDisabled = !!state.busy;

  async function onPatch() {
    const v = await form({
      title: "改写摘要",
      fields: [
        { kind: "textarea", name: "summary", label: "新摘要", defaultValue: chunk.summary ?? "", placeholder: "留空保持不变" },
      ],
    });
    if (!v) return;
    await call(
      "改写摘要",
      "POST",
      `/api/operation-knowledge/chunks/${id}/patch`,
      chunkPatchRequest(v.summary),
    );
  }

  async function onReject() {
    const v = await form({
      title: "退回这条知识",
      fields: [
        { kind: "textarea", name: "reason", label: "退回原因", required: true, placeholder: "如：来源失效 / 内容过期 / 与现有知识矛盾" },
      ],
    });
    if (!v) return;
    await call("退回", "POST", `/api/operation-knowledge/chunks/${id}/reject`, { reason: v.reason });
  }

  async function onArchive() {
    const ok = await confirm({
      title: "归档这条知识？",
      body: "归档后 AI 不再使用它回复客户，可在已归档列表恢复。",
      tone: "danger",
      confirmText: "确认归档",
    });
    if (!ok) return;
    await call("归档", "POST", `/api/operation-knowledge/chunks/${id}/archive`, {});
  }

  async function onSplit() {
    const v = await form({
      title: "拆分知识条目",
      fields: [
        { kind: "text", name: "cutoff", label: "字符切点", required: true, hint: "填一个整数，如 200（从第 200 个字处切开）" },
      ],
    });
    if (!v) return;
    const offset = Number(v.cutoff);
    if (!Number.isInteger(offset) || offset <= 0) {
      setState({ busy: null, error: "字符位置必须是正整数", info: null });
      return;
    }
    await call(
      "拆分",
      "POST",
      `/api/operation-knowledge/chunks/${id}/split`,
      chunkSplitRequest(offset),
    );
  }

  async function onMerge() {
    const v = await form({
      title: "合并到另一条知识",
      loadChunks: loadChunkOptions,
      fields: [
        { kind: "chunkRef", name: "target", label: "合并目标", required: true, hint: "当前知识会被归档，内容并入目标" },
      ],
    });
    if (!v) return;
    await call(
      "合并",
      "POST",
      `/api/operation-knowledge/chunks/${id}/merge`,
      chunkMergeRequest(v.target),
    );
  }

  async function onRelate() {
    const v = await form({
      title: "建立知识关联",
      loadChunks: loadChunkOptions,
      fields: [
        { kind: "chunkRef", name: "target", label: "关联目标", required: true },
        {
          kind: "select",
          name: "kind",
          label: "关系类型",
          options: [
            { value: "references", label: "引用" },
            { value: "requires", label: "依赖" },
            { value: "contradicts", label: "矛盾" },
            { value: "clarifies", label: "澄清" },
            { value: "refines", label: "细化" },
            { value: "superseded_by", label: "被取代" },
          ],
        },
        { kind: "text", name: "note", label: "备注", placeholder: "可空" },
      ],
    });
    if (!v) return;
    await call(
      "关联",
      "POST",
      `/api/operation-knowledge/chunks/${id}/relate`,
      chunkRelateRequest(v.target, v.kind as ChunkRelationKind, v.note),
    );
  }

  return (
    <section className="wikiInspectorSection">
      <div className="wikiInspectorSectionTitle">编辑动作</div>
      {presenceByOther ? (
        <div className="wikiAlert info">
          其他管理员也在查看或编辑这条知识；这是协作提示，仍可提交，冲突由版本校验拒绝。
        </div>
      ) : null}
      <div className="wikiActionsBar">
        <button
          type="button"
          className="wikiBtn wikiActionBtn--verify"
          disabled={writeDisabled || isVerified}
          onClick={() => {
            const expectedUpdatedAt = chunk.updatedAt?.trim();
            if (!expectedUpdatedAt) {
              setState({ busy: null, error: "缺少版本信息，请刷新后重试", info: null });
              return;
            }
            void call("确认放行", "POST", `/api/operation-knowledge/chunks/${id}/verify`, {
              expectedUpdatedAt,
            });
          }}
          title="确认这条知识可被 AI 用于回复客户（AI 永不自动调用）"
        >
          <CheckCircle2 size={13} /> 确认放行
        </button>
        <button
          type="button"
          className="wikiBtn wikiActionBtn--reject"
          disabled={writeDisabled}
          onClick={() => void onReject()}
        >
          <X size={13} /> 退回
        </button>
        <button type="button" className="wikiBtn" disabled={writeDisabled} onClick={() => void onPatch()}>
          <SquarePen size={13} /> 改摘要
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={writeDisabled || isArchived}
          onClick={() => void onArchive()}
        >
          <Archive size={13} /> 归档
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={writeDisabled || !isArchived}
          onClick={() =>
            void call("恢复", "POST", `/api/operation-knowledge/chunks/${id}/restore`, {})
          }
        >
          <Undo2 size={13} /> 恢复
        </button>
        <button type="button" className="wikiBtn" disabled={writeDisabled} onClick={() => void onSplit()}>
          <Scissors size={13} /> 拆分
        </button>
        <button type="button" className="wikiBtn" disabled={writeDisabled} onClick={() => void onMerge()}>
          <GitMerge size={13} /> 合并
        </button>
        <button type="button" className="wikiBtn" disabled={writeDisabled} onClick={() => void onRelate()}>
          <Link2 size={13} /> 关联
        </button>
      </div>
      {state.error ? <div className="wikiAlert error">{state.error}</div> : null}
      {state.info ? <div className="wikiAlert info">{state.info}</div> : null}
      <div className="wikiHint">
        恢复历史版本的入口在下方"修订时间轴"。AI 起草的知识强制为草稿、待确认；只有管理员能手动确认放行。
      </div>
    </section>
  );
}

// ── G3 · ChunkReferrersList：反向引用查询 ────────────────────────
type ReferrerEntry = {
  chunkId: string;
  title?: string | null;
  wikiType?: string | null;
  status?: string | null;
  kind?: string | null;
  note?: string | null;
};

function ChunkReferrersList({ chunkId }: { chunkId: string }) {
  const [items, setItems] = useState<ReferrerEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open || items !== null) return;
    setLoading(true);
    fetch(`/api/operation-knowledge/chunks/referrers?targetId=${encodeURIComponent(chunkId)}`)
      .then(async (r) => {
        if (!r.ok) throw await parseApiError(r);
        return r.json() as Promise<{ items: ReferrerEntry[] }>;
      })
      .then((data) => setItems(data.items ?? []))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, [open, items, chunkId]);

  // chunkId 变化重置
  useEffect(() => {
    setItems(null);
    setOpen(false);
    setError(null);
  }, [chunkId]);

  return (
    <section className="wikiInspectorSection">
      <button
        type="button"
        className="wikiInspectorSectionTitle wikiCollapseHead"
        onClick={() => setOpen((v) => !v)}
      >
        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />} 被引用
        {items ? `（${items.length}）` : "（点击查询）"}
      </button>
      {open ? (
        loading ? (
          <div className="wikiInspectorEmpty">加载中…</div>
        ) : error ? (
          <div className="wikiAlert error">{error}</div>
        ) : !items || items.length === 0 ? (
          <div className="wikiInspectorEmpty">没有其他知识条目引用此条目。</div>
        ) : (
          <div className="wikiReferrerList">
            {items.map((r, i) => (
              <button
                type="button"
                key={`${r.chunkId}-${i}`}
                className="wikiReferrerCard"
                onClick={() => focusChunk(r.chunkId)}
                title={r.note ?? ""}
              >
                <div className="wikiReferrerCardHead">
                  {r.wikiType ? <span className="wikiArchiveTag">{wikiTypeLabel(r.wikiType)}</span> : null}
                  <span className="wikiReferrerKind">{relatedKindLabel(r.kind)}</span>
                </div>
                <div className="wikiReferrerCardTitle">{r.title || r.chunkId}</div>
                {r.note ? <div className="wikiReferrerCardNote">{r.note}</div> : null}
              </button>
            ))}
          </div>
        )
      ) : null}
    </section>
  );
}

// ── G3 · ChunkRevisionsTimeline：版本时间轴 + rollback ────────────────
type RevisionEntry = {
  id?: string;
  revisionId?: string;
  op: string;
  source?: string | null;
  author?: string | null;
  createdAt?: string | null;
  summary?: string | null;
  diff?: unknown;
};

export function ChunkRevisionsTimeline({
  chunkId,
  onRolledBack,
}: {
  chunkId: string;
  onRolledBack: () => void;
}) {
  const confirm = useConfirm();
  const [items, setItems] = useState<RevisionEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [busyRev, setBusyRev] = useState<string | null>(null);

  function load() {
    setLoading(true);
    setError(null);
    fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/revisions`)
      .then(async (r) => {
        if (!r.ok) throw await parseApiError(r);
        return r.json() as Promise<{ items: RevisionEntry[] }>;
      })
      .then((data) => setItems(data.items ?? []))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    if (open && items === null) load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, items, chunkId]);

  useEffect(() => {
    setItems(null);
    setOpen(false);
    setError(null);
  }, [chunkId]);

  async function rollback(rev: RevisionEntry) {
    const rid = rev.revisionId ?? rev.id;
    if (!rid) return;
    const ok = await confirm({
      title: "回滚到该版本？",
      body: "会基于这个历史版本生成一条新的修订记录，当前内容仍可追溯。",
      tone: "danger",
      confirmText: "确认回滚",
    });
    if (!ok) return;
    setBusyRev(rid);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/rollback/${encodeURIComponent(rid)}`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({}) },
      );
      if (!r.ok) throw await parseApiError(r);
      setItems(null);
      onRolledBack();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyRev(null);
    }
  }

  return (
    <section className="wikiInspectorSection">
      <button
        type="button"
        className="wikiInspectorSectionTitle wikiCollapseHead"
        onClick={() => setOpen((v) => !v)}
      >
        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        <History size={12} /> 修订时间轴
        {items ? `（${items.length}）` : "（点击查询）"}
      </button>
      {open ? (
        loading ? (
          <div className="wikiInspectorEmpty">加载中…</div>
        ) : error ? (
          <div className="wikiAlert error">{error}</div>
        ) : !items || items.length === 0 ? (
          <div className="wikiInspectorEmpty">暂无修订记录。</div>
        ) : (
          <ol className="wikiArchiveTimeline">
            {items.map((rev, i) => {
              const rid = rev.revisionId ?? rev.id ?? `rev-${i}`;
              return (
                <li key={rid} className="wikiArchiveTimelineItem">
                  <span className="wikiArchiveTimelineDot" aria-hidden />
                  <div className="wikiArchiveTimelineMeta">
                    <span className="wikiArchiveTimelineTime">
                      {rev.createdAt ?? "—"}
                    </span>
                    <span className="wikiArchiveTag">{revisionOpLabel(rev.op)}</span>
                    {rev.source ? <span className="wikiArchiveTag">{revisionSourceLabel(rev.source)}</span> : null}
                    {rev.author ? <code>{rev.author}</code> : null}
                  </div>
                  {rev.summary ? (
                    <div className="wikiArchiveTimelineSummary">{rev.summary}</div>
                  ) : null}
                  <div className="wikiArchiveTimelineActions">
                    <button
                      type="button"
                      className="wikiBtn"
                      disabled={busyRev === rid}
                      onClick={() => void rollback(rev)}
                      title="回滚到此版本（创建新 revision)"
                    >
                      <Undo2 size={12} /> 回滚至此
                    </button>
                  </div>
                </li>
              );
            })}
          </ol>
        )
      ) : null}
    </section>
  );
}

export interface TreeChunkItem extends ReviewChunkItem {
  businessTopics?: string[] | null;
}

export function numberOr(v: unknown): number {
  return typeof v === "number" ? v : Number(v ?? 0) || 0;
}
function stringOr(v: unknown): string {
  return typeof v === "string" ? v : "";
}

export { stringOr };
