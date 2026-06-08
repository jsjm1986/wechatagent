import { useState, useEffect, useMemo } from "react";
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
import { parseApiError, LlmUnavailableError } from "../../lib/api";
import { type TrustChunkFields, chunkTypeLabel } from "./trustTypes";

const LLM_KIND_LABELS: Record<string, string> = {
  timeout: "上游超时",
  connect_failed: "无法连接",
  body_decode_error: "响应体损坏",
  network_error: "网络异常",
  rate_limited: "上游限流",
  http_5xx: "上游 5xx",
  http_4xx: "上游 4xx",
  empty_response: "空响应",
  external_error: "上游错误",
  json_decode_error: "JSON 解析失败",
  unknown: "未知错误"
};

function llmKindLabel(kind: string): string {
  return LLM_KIND_LABELS[kind] ?? kind;
}

/**
 * 统一渲染 LLM 调用失败的提示横幅，给所有调 LLM 的面板复用。
 *
 * - `error` 是 `LlmUnavailableError` → 显示 kind 标签 + hint + 重试次数 + 「AI 重试」
 * - `error` 是普通 `Error` → 显示 message + 「AI 重试」（走通用错误路径）
 */
export function LlmErrorBanner(props: {
  error: Error;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  const { error, onRetry, retrying } = props;
  const isLlm = error instanceof LlmUnavailableError;
  const kind = isLlm ? (error as LlmUnavailableError).kind : "unknown";
  const hint = isLlm
    ? (error as LlmUnavailableError).hint
    : error.message || "调用 LLM 失败，请稍后再试。";
  const retryCount = isLlm ? (error as LlmUnavailableError).retryCount : 0;
  const detail = isLlm ? (error as LlmUnavailableError).detail : "";
  return (
    <div className="llmErrorBanner" role="alert">
      <div className="llmErrorBanner__head">
        <span className="llmErrorBanner__kind">{llmKindLabel(kind)}</span>
        {retryCount > 0 ? (
          <span className="llmErrorBanner__retries">已自动重试 {retryCount} 次</span>
        ) : null}
      </div>
      <div className="llmErrorBanner__hint">{hint}</div>
      {detail && detail !== hint ? (
        <details className="llmErrorBanner__detail">
          <summary>查看技术细节</summary>
          <code>{detail}</code>
        </details>
      ) : null}
      {onRetry ? (
        <div className="llmErrorBanner__actions">
          <button
            type="button"
            className="primary"
            onClick={onRetry}
            disabled={retrying}
          >
            {retrying ? "AI 重试中…" : "AI 重试"}
          </button>
        </div>
      ) : null}
    </div>
  );
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

export function classifyChunk(
  c: ReviewChunkItem,
  activeIds: Set<string>
): ReviewCategory | null {
  // 优先级：contested > needs_review > source_orphan > pending_verification > dependents_pending
  if (c.integrityStatus === "rejected") return "contested";
  const hasQuote = !!c.sourceQuote && c.sourceQuote.trim().length > 0;
  const hasAnchor = (c.sourceAnchors?.length ?? 0) > 0;
  if (c.integrityStatus === "needs_review") {
    if (!hasQuote || !hasAnchor) return "source_orphan";
    return "pending_verification";
  }
  if (!hasQuote || !hasAnchor) return "source_orphan";
  if (c.relatedChunks && c.relatedChunks.length > 0) {
    const broken = c.relatedChunks.some((r) => !activeIds.has(r.chunk_id));
    if (broken) return "dependents_pending";
  }
  // verified 且关系完好的 chunk 不进 review 视图。
  return null;
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
  const [reloadKey, setReloadKey] = useState(0);
  const lock = useChunkInspectorLock(chunkId);

  useEffect(() => {
    if (!chunkId) return;
    setLoading(true);
    setError(null);
    fetch("/api/operation-knowledge/chunks")
      .then(async (r) => {
        if (!r.ok) throw await parseApiError(r);
        return r.json() as Promise<{ items: TreeChunkItem[] }>;
      })
      .then((data) => setItems(data.items ?? []))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, [chunkId, reloadKey]);

  const reload = () => setReloadKey((k) => k + 1);

  // P1-4：另一端写入此 chunk 时自动 reload，让两个 admin 同步看到。
  useEffect(() => {
    if (!chunkId) return;
    const onRevised = (e: Event) => {
      const detail = (e as CustomEvent<{ chunk_id?: string }>).detail;
      if (detail?.chunk_id === chunkId) {
        setReloadKey((k) => k + 1);
      }
    };
    window.addEventListener("wikiChunkRevised", onRevised);
    return () => window.removeEventListener("wikiChunkRevised", onRevised);
  }, [chunkId]);

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
          <Eye size={14} /> Inspector
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
            未找到 chunk <code>{chunkId}</code>，可能已 archived 或不在当前 workspace。
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
                  {chunk.integrityStatus ?? "—"}
                </span>{" "}
                <span className="wikiBadge">{chunk.status ?? "—"}</span>
              </dd>
              <dt>chunk id</dt>
              <dd><code>{chunk.id}</code></dd>
              {chunk.wikiType ? (<><dt>wiki type</dt><dd><span className="wikiArchiveTag">{chunk.wikiType}</span></dd></>) : null}
              {chunkTypeLabel(chunk.chunkType) ? (<><dt>运营用途</dt><dd><span className="wikiArchiveTag">{chunkTypeLabel(chunk.chunkType)}</span></dd></>) : null}
              {Array.isArray(chunk.businessTopics) && chunk.businessTopics.length > 0 ? (
                <>
                  <dt>business topics</dt>
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
                        <span className="wikiRelatedKind">previous</span>
                        <span className="wikiRelatedTitle">{prev ? prev.title : chunk.previousVersionId}</span>
                      </button>
                    </dd>
                  </>
                );
              })() : null}
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
              <div className="wikiHint">无 source_quote — 该 chunk 不可被 verify。</div>
            )}
            {anchors.length > 0 ? (
              <section className="wikiInspectorSection">
                <div className="wikiInspectorSectionTitle">source_anchors（{anchors.length}）</div>
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
                <div className="wikiInspectorSectionTitle">related_chunks（{related.length}）</div>
                <div className="wikiRelatedList">
                  {related.map((r, i) => {
                    const target = indexById.get(r.chunk_id);
                    const dead = !target;
                    return (
                      <button
                        type="button"
                        key={`${chunk.id}-irel-${i}`}
                        className={`wikiRelatedChip ${dead ? "dead" : ""}`}
                        disabled={dead}
                        onClick={() => focusChunk(r.chunk_id)}
                        title={dead ? "目标 chunk 不在活跃集合" : r.note ?? ""}
                      >
                        <span className="wikiRelatedKind">{r.kind}</span>
                        <span className="wikiRelatedTitle">{target ? target.title : r.chunk_id}</span>
                      </button>
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
            <ChunkActionsBar chunk={chunk} onChanged={reload} />
            <ChunkReferrersList chunkId={chunk.id} />
            <ChunkSourceSection chunkId={chunk.id} />
            <ChunkRevisionsTimeline chunkId={chunk.id} onRolledBack={reload} />
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

// ── P1-4 · WebSocket 软锁 + 事件总线 ───────────────────────────────────────
//
// 锁状态机：
//   - 'idle' 初始；
//   - 'self' 当前 admin 持锁，60s 心跳续期；
//   - 'other' 已被他人持锁（409 返回 lock 信息）；
//   - 'error' 网络错或 5xx，UI 静默退化为只读。
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
        <span>我正在编辑{at ? ` · 自动续期至 ${at}` : ""}</span>
      </div>
    );
  }
  if (lock.state === "other") {
    const at = formatLockExpiry(lock.holder.expiresAt);
    const who = lock.holder.ownerUsername || lock.holder.ownerUserId || "其他 admin";
    return (
      <div className="wikiInspectorLockBadge wikiInspectorLockBadge--other" role="status">
        <span className="wikiInspectorLockDot" aria-hidden />
        <span>由 {who} 编辑中{at ? `（至 ${at}）` : ""} · 暂只读</span>
      </div>
    );
  }
  return (
    <div className="wikiInspectorLockBadge wikiInspectorLockBadge--error" role="status">
      <span className="wikiInspectorLockDot" aria-hidden />
      <span>锁信道异常 · {lock.reason}</span>
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

    // WebSocket 推 unlocked 时刷一次（他人主动 release，给当前 admin 一次抢锁机会）
    const onUnlocked = (e: Event) => {
      const detail = (e as CustomEvent<{ chunk_id?: string }>).detail;
      if (detail?.chunk_id === chunkId) {
        void acquire();
      }
    };
    const onLocked = (e: Event) => {
      const detail = (e as CustomEvent<{ chunk_id?: string; owner_user_id?: string; owner_username?: string; expires_at?: string }>).detail;
      if (detail?.chunk_id === chunkId) {
        // 别人加锁——只有不是我自己时才覆盖；我自己的 acquire 会先把状态写成 self。
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
}: {
  chunk: TreeChunkItem;
  onChanged: () => void;
}) {
  const [state, setState] = useState<ChunkActionState>({ busy: null, error: null, info: null });

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
      setState({ busy: null, error: null, info: `已${action}` });
      onChanged();
    } catch (e: unknown) {
      setState({ busy: null, error: e instanceof Error ? e.message : String(e), info: null });
    }
  }

  const id = encodeURIComponent(chunk.id);
  const isArchived = chunk.status === "archived";
  const isVerified = chunk.integrityStatus === "verified";

  async function onPatch() {
    const summary = window.prompt("新摘要（覆盖 summary，留空保持不变）", chunk.summary ?? "");
    if (summary === null) return;
    await call(
      "patch",
      "POST",
      `/api/operation-knowledge/chunks/${id}/patch`,
      { summary: summary || undefined, actor: "admin" },
    );
  }

  async function onReject() {
    const reason = window.prompt("reject 原因（必填）");
    if (!reason) return;
    await call(
      "reject",
      "POST",
      `/api/operation-knowledge/chunks/${id}/reject`,
      { reason },
    );
  }

  async function onArchive() {
    if (!window.confirm(`确认 archive chunk ${chunk.id}?`)) return;
    await call(
      "archive",
      "POST",
      `/api/operation-knowledge/chunks/${id}/archive`,
      { actor: "admin" },
    );
  }

  async function onSplit() {
    const cutoff = window.prompt("切点（正则或字符位置整数，必填）");
    if (!cutoff) return;
    const num = Number(cutoff);
    const body = Number.isFinite(num)
      ? { offset: num, actor: "admin" }
      : { regex: cutoff, actor: "admin" };
    await call(
      "split",
      "POST",
      `/api/operation-knowledge/chunks/${id}/split`,
      body,
    );
  }

  async function onMerge() {
    const targetId = window.prompt("合并目标 chunk id（必填）");
    if (!targetId) return;
    if (!window.confirm(`将 ${chunk.id} 合并到 ${targetId}？原 chunk 会被 archived。`)) return;
    await call(
      "merge",
      "POST",
      `/api/operation-knowledge/chunks/${id}/merge`,
      { target_id: targetId, actor: "admin" },
    );
  }

  async function onRelate() {
    const targetId = window.prompt("关联目标 chunk id");
    if (!targetId) return;
    const kind = window.prompt("关联 kind（如 supports / contradicts / superseded_by）", "supports");
    if (!kind) return;
    const note = window.prompt("备注（可空）", "") ?? "";
    await call(
      "relate",
      "POST",
      `/api/operation-knowledge/chunks/${id}/relate`,
      { target_id: targetId, kind, note: note || null, actor: "admin" },
    );
  }

  return (
    <section className="wikiInspectorSection">
      <div className="wikiInspectorSectionTitle">编辑动作</div>
      <div className="wikiActionsBar">
        <button
          type="button"
          className="wikiBtn wikiActionBtn--verify"
          disabled={!!state.busy || isVerified}
          onClick={() =>
            void call(
              "verify",
              "POST",
              `/api/operation-knowledge/chunks/${id}/verify`,
              {},
            )
          }
          title="标记为 verified（AI 永不自动调用）"
        >
          <CheckCircle2 size={13} /> verify
        </button>
        <button
          type="button"
          className="wikiBtn wikiActionBtn--reject"
          disabled={!!state.busy}
          onClick={() => void onReject()}
        >
          <X size={13} /> reject
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={!!state.busy}
          onClick={() => void onPatch()}
        >
          <SquarePen size={13} /> patch
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={!!state.busy || isArchived}
          onClick={() => void onArchive()}
        >
          <Archive size={13} /> archive
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={!!state.busy || !isArchived}
          onClick={() =>
            void call(
              "restore",
              "POST",
              `/api/operation-knowledge/chunks/${id}/restore`,
              { actor: "admin" },
            )
          }
        >
          <Undo2 size={13} /> restore
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={!!state.busy}
          onClick={() => void onSplit()}
        >
          <Scissors size={13} /> split
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={!!state.busy}
          onClick={() => void onMerge()}
        >
          <GitMerge size={13} /> merge
        </button>
        <button
          type="button"
          className="wikiBtn"
          disabled={!!state.busy}
          onClick={() => void onRelate()}
        >
          <Link2 size={13} /> relate
        </button>
      </div>
      {state.error ? <div className="wikiAlert error">{state.error}</div> : null}
      {state.info ? <div className="wikiAlert info">{state.info}</div> : null}
      <div className="wikiHint">
        rollback 入口在下方"修订时间轴"。AI 强制 status=draft + integrity_status=needs_review；verify 仅 admin 手工触发。
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
    fetch(`/api/operation-knowledge/chunks/referrers?target_id=${encodeURIComponent(chunkId)}`)
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
          <div className="wikiInspectorEmpty">无 chunk 引用此 chunk。</div>
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
                  {r.wikiType ? <span className="wikiArchiveTag">{r.wikiType}</span> : null}
                  <span className="wikiReferrerKind">{r.kind ?? "—"}</span>
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
    if (!window.confirm(`确认回滚到 revision ${rid} (op=${rev.op})？将创建新 revision(op=rollback_to)。`))
      return;
    setBusyRev(rid);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/rollback/${encodeURIComponent(rid)}`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ actor: "admin" }) },
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
          <div className="wikiInspectorEmpty">无 revisions。</div>
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
                    <span className="wikiArchiveTag">{rev.op}</span>
                    {rev.source ? <span className="wikiArchiveTag">{rev.source}</span> : null}
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
