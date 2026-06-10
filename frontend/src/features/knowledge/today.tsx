import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import {
  ArrowRight,
  CheckCircle2,
  FileBox,
  Inbox,
  Loader2,
  MessageSquareText,
  Plus,
  RefreshCw,
  Search,
  SendHorizonal,
  Sparkles,
  SquarePen,
  Trash2,
  X,
} from "lucide-react";
import { parseApiError } from "../../lib/api";
import { LlmErrorBanner, focusChunk } from "./shared";
import { useConfirm } from "../../components/ui/ConfirmDialog";
import { useToast } from "../../components/ui/Toast";
import { severityLabel, priorityLabel, originLabel } from "./labels";

interface ChatTurnView {
  role: "user" | "assistant";
  turnIndex: number;
  intent?: string | null;
  content: string;
  naturalReply?: string | null;
  draftKind?: string | null;
  draftPreview?: Record<string, unknown> | null;
  missingFields?: string[];
  followupQuestions?: string[];
  canApply?: boolean;
  status?: string;
  attachments?: Array<{ chunkId?: string; itemId?: string }>;
  targetChunkId?: string | null;
  targetPackId?: string | null;
}

interface ChatTurnResponse {
  sessionId: string;
  turnIndex: number;
  intent: string;
  naturalReply: string;
  draftKind?: string | null;
  draftPreview?: Record<string, unknown> | null;
  missingFields?: string[];
  followupQuestions?: string[];
  canApply?: boolean;
  targetChunkId?: string | null;
  targetPackId?: string | null;
}

export function ChatWorkbench({ initialAttachChunkId }: { initialAttachChunkId?: string | null } = {}) {
  const confirm = useConfirm();
  const toast = useToast();
  const [sessionId, setSessionId] = useState<string>(() => {
    if (typeof window === "undefined") return "";
    return window.localStorage.getItem("knowledgeChat.sessionId") ?? "";
  });
  const [draft, setDraft] = useState("");
  const [attachChunkId, setAttachChunkId] = useState<string>("");

  // B2：从待办收件箱「找 AI 协作」跳转过来时预填 chunkId。
  useEffect(() => {
    if (initialAttachChunkId) setAttachChunkId(initialAttachChunkId);
  }, [initialAttachChunkId]);
  const [turns, setTurns] = useState<ChatTurnView[]>([]);
  const [pending, setPending] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const esRef = useRef<EventSource | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const persistSession = useCallback((sid: string) => {
    if (typeof window === "undefined") return;
    if (sid) window.localStorage.setItem("knowledgeChat.sessionId", sid);
    else window.localStorage.removeItem("knowledgeChat.sessionId");
  }, []);

  const loadHistory = useCallback(async (sid: string) => {
    if (!sid) {
      setTurns([]);
      return;
    }
    try {
      const r = await fetch(`/api/operation-knowledge/chat/${encodeURIComponent(sid)}`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: unknown[] };
      const items = Array.isArray(data.items) ? data.items : [];
      const list: ChatTurnView[] = items.map((raw) => {
        const obj = (raw ?? {}) as Record<string, unknown>;
        return {
          role: (obj.role as ChatTurnView["role"]) ?? "user",
          turnIndex: Number(obj.turnIndex ?? 0),
          intent: (obj.intent as string | null | undefined) ?? null,
          content: String(obj.content ?? ""),
          naturalReply: (obj.naturalReply as string | null | undefined) ?? null,
          draftKind: (obj.draftKind as string | null | undefined) ?? null,
          draftPreview: (obj.patch as Record<string, unknown> | null | undefined) ?? null,
          missingFields: (obj.missingFields as string[] | undefined) ?? [],
          followupQuestions: (obj.followupQuestions as string[] | undefined) ?? [],
          canApply: Boolean(obj.canApply),
          status: (obj.status as string | undefined) ?? "",
          attachments: (obj.attachments as Array<{ chunkId?: string; itemId?: string }> | undefined) ?? []
        };
      });
      setTurns(list);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    if (sessionId) void loadHistory(sessionId);
  }, [sessionId, loadHistory]);

  useEffect(() => {
    if (!sessionId || typeof window === "undefined" || typeof window.EventSource === "undefined") return;
    esRef.current?.close();
    const es = new EventSource(
      `/api/knowledge/chat/sessions/${encodeURIComponent(sessionId)}/stream`
    );
    esRef.current = es;
    es.addEventListener("turn", () => {
      void loadHistory(sessionId);
    });
    es.addEventListener("close", () => {
      es.close();
    });
    es.addEventListener("error", () => {
      es.close();
    });
    return () => {
      es.close();
    };
  }, [sessionId, loadHistory]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [turns]);

  function newSession() {
    setSessionId("");
    persistSession("");
    setTurns([]);
    setDraft("");
    setAttachChunkId("");
    setError(null);
    setInfo(null);
  }

  async function submit() {
    const content = draft.trim();
    if (!content) {
      setError("请输入内容");
      return;
    }
    setPending(true);
    setError(null);
    setInfo(null);
    try {
      const body: Record<string, unknown> = { content };
      if (sessionId) body.sessionId = sessionId;
      const aid = attachChunkId.trim();
      if (aid) body.attachments = [{ chunkId: aid }];
      const r = await fetch("/api/operation-knowledge/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body)
      });
      if (!r.ok) throw await parseApiError(r);
      const resp = (await r.json()) as ChatTurnResponse;
      if (resp.sessionId !== sessionId) {
        setSessionId(resp.sessionId);
        persistSession(resp.sessionId);
      }
      setDraft("");
      await loadHistory(resp.sessionId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  async function apply() {
    if (!sessionId) return;
    setApplying(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}/apply`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({})
        }
      );
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { chunkId?: string; itemId?: string; status?: string };
      const fid = data.chunkId || data.itemId;
      setInfo(`已应用为草稿（${data.status ?? "draft"}）${fid ? `：${fid}` : ""}`);
      if (data.chunkId) {
        window.dispatchEvent(
          new CustomEvent("wikiFocusChunk", { detail: { chunkId: data.chunkId } })
        );
      }
      await loadHistory(sessionId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setApplying(false);
    }
  }

  async function discard() {
    if (!sessionId) return;
    const ok = await confirm({
      title: "丢弃当前草稿？",
      body: "将丢弃本会话最后一份 AI 起草内容，此操作不可恢复。",
      tone: "danger",
      confirmText: "确认丢弃",
    });
    if (!ok) return;
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}/discard`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
      );
      if (!r.ok) throw await parseApiError(r);
      toast.success("已丢弃当前草稿");
      await loadHistory(sessionId);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      toast.error(msg);
    }
  }

  const lastAssistant = useMemo(
    () => [...turns].reverse().find((t) => t.role === "assistant"),
    [turns]
  );

  return (
    <div className="wikiArchiveShell wikiChatWorkbench">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">today / chat</div>
          <h2>AI 协作工坊</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <span className="wikiArchiveTag">session</span>
          <span className="wikiChatSessionId">{sessionId || "未开始"}</span>
          <button type="button" onClick={newSession}>
            <Plus size={14} /> 新会话
          </button>
        </div>
      </header>

      {error ? <div className="wikiBannerError">{error}</div> : null}
      {info ? <div className="wikiBannerInfo">{info}</div> : null}

      <div className="wikiChatStream" ref={scrollRef}>
        {turns.length === 0 ? (
          <div className="wikiEmpty">
            <MessageSquareText size={28} /> 与 AI 协作起草 / 修复切片。AI 起草不会自动验证，需要运营点击「应用为草稿」。
          </div>
        ) : null}
        {turns.map((t) => (
          <article
            key={`${t.role}-${t.turnIndex}`}
            className={`wikiChatTurn wikiChatTurn--${t.role}`}
          >
            <div className="wikiChatTurnHead">
              <span className="wikiArchiveTag">{t.role === "user" ? "运营" : "AI"}</span>
              <span className="wikiArchiveTimelineTime">#{t.turnIndex}</span>
              {t.intent ? <span className="wikiArchiveTag">{t.intent}</span> : null}
              {t.draftKind ? <span className="wikiArchiveTag">{t.draftKind}</span> : null}
            </div>
            <div className="wikiChatTurnBody">
              {t.role === "assistant" && t.naturalReply ? t.naturalReply : t.content}
            </div>
            {t.role === "assistant" && t.followupQuestions && t.followupQuestions.length > 0 ? (
              <ul className="wikiChatFollowups">
                {t.followupQuestions.map((q, i) => (
                  <li key={i}>{q}</li>
                ))}
              </ul>
            ) : null}
            {t.role === "assistant" && t.draftPreview ? (
              <details className="wikiChatDraftPreview">
                <summary>查看 AI 起草内容</summary>
                <pre>{JSON.stringify(t.draftPreview, null, 2)}</pre>
              </details>
            ) : null}
            {t.role === "assistant" &&
            t.missingFields &&
            t.missingFields.length > 0 ? (
              <div className="wikiChatMissing">
                缺字段：
                {t.missingFields.map((f) => (
                  <span key={f} className="wikiArchiveTag">{f}</span>
                ))}
              </div>
            ) : null}
          </article>
        ))}
      </div>

      <footer className="wikiChatFooter">
        <textarea
          className="wikiChatInput"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="向 AI 描述要起草 / 修复 / 拆分的切片，可附带 chunkId 引用现有切片"
          disabled={pending}
          rows={3}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <div className="wikiChatFooterRow">
          <input
            type="text"
            className="wikiChatAttachInput"
            value={attachChunkId}
            onChange={(e) => setAttachChunkId(e.target.value)}
            placeholder="可选：附带 chunkId"
            disabled={pending}
          />
          <button
            type="button"
            className="primary"
            onClick={() => void submit()}
            disabled={pending}
          >
            <SendHorizonal size={14} /> {pending ? "发送中…" : "发送"}
          </button>
          <button
            type="button"
            onClick={() => void apply()}
            disabled={applying || !lastAssistant?.canApply}
            title={lastAssistant?.canApply ? "把当前 AI 草稿落库为草稿（status=draft, integrity=needs_review）" : "无可应用草稿"}
          >
            <CheckCircle2 size={14} /> {applying ? "应用中…" : "应用为草稿"}
          </button>
          <button type="button" onClick={() => void discard()} disabled={!sessionId}>
            <Trash2 size={14} /> 丢弃草稿
          </button>
        </div>
      </footer>
    </div>
  );
}

interface InboxItemView {
  id: string;
  priority: "high" | "mid" | "low" | string;
  kind: string;
  title: string;
  contextSummary: string;
  targetChunkId?: string | null;
  targetPackId?: string | null;
  suggestedActions: string[];
  origin: string;
  createdAt: string;
}

interface InboxResp {
  items: InboxItemView[];
  stats: { total: number; high: number; mid: number; low: number };
}

export function KnowledgeInbox({
  onOpenChat,
  onFocusChunk,
}: {
  onOpenChat?: (chunkId?: string) => void;
  onFocusChunk?: (chunkId: string) => void;
} = {}) {
  const toast = useToast();
  const [data, setData] = useState<InboxResp | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [priority, setPriority] = useState<"" | "high" | "mid" | "low">("");
  const [dismissed, setDismissed] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      if (priority) params.set("priority", priority);
      const r = await fetch(
        `/api/operation-knowledge/inbox${params.toString() ? "?" + params : ""}`
      );
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as InboxResp;
      setData(d);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, [priority]);

  useEffect(() => {
    void load();
  }, [load]);

  const focus = onFocusChunk ?? focusChunk;

  function handleOpenChat(chunkId?: string) {
    if (onOpenChat) onOpenChat(chunkId);
    else toast.info("请到「AI 协作」标签与 AI 协作补充这条知识");
  }

  function handleDismiss(id: string) {
    // 本地乐观隐藏 + toast（后端暂无逐条 dismiss 接口时不发死请求）
    setDismissed((prev) => new Set(prev).add(id));
    toast.success("已从待办中移除");
  }

  return (
    <div className="wikiArchiveShell wikiInbox">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">今日 / 待办</div>
          <h2>待办收件箱</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <select
            value={priority}
            onChange={(e) => setPriority(e.target.value as typeof priority)}
          >
            <option value="">全部优先级</option>
            <option value="high">高</option>
            <option value="mid">中</option>
            <option value="low">低</option>
          </select>
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <div className="wikiBannerError">{error}</div> : null}

      {data ? (
        <div className="wikiInboxStats">
          <span className="wikiArchiveTag">共 {data.stats.total}</span>
          <span className="wikiArchiveTag">高 {data.stats.high}</span>
          <span className="wikiArchiveTag">中 {data.stats.mid}</span>
          <span className="wikiArchiveTag">低 {data.stats.low}</span>
        </div>
      ) : null}

      <div className="wikiInboxList">
        {data && data.items.filter((it) => !dismissed.has(it.id)).length === 0 ? (
          <div className="wikiEmpty">
            <Inbox size={24} /> 暂无待办
          </div>
        ) : null}
        {data?.items.filter((it) => !dismissed.has(it.id)).map((it) => (
          <article
            key={it.id}
            className={`wikiInboxCard wikiInboxCard--${it.priority}`}
          >
            <div className="wikiInboxCardHead">
              <span className={`wikiArchiveTag wikiInboxPriority--${it.priority}`}>
                {priorityLabel(it.priority)}
              </span>
              <span className="wikiArchiveTag">{originLabel(it.origin)}</span>
              <span className="wikiArchiveTimelineTime">{it.createdAt}</span>
            </div>
            <h4 className="wikiInboxCardTitle">{it.title}</h4>
            <p className="wikiInboxCardSummary">{it.contextSummary}</p>
            <div className="wikiInboxCardActions">
              {it.targetChunkId ? (
                <button type="button" onClick={() => focus(it.targetChunkId as string)}>
                  <ArrowRight size={12} /> 查看知识
                </button>
              ) : null}
              {it.suggestedActions.includes("open_chat") ? (
                <button type="button" onClick={() => handleOpenChat(it.targetChunkId ?? undefined)}>
                  <MessageSquareText size={12} /> 找 AI 协作
                </button>
              ) : null}
              {it.suggestedActions.includes("open_repair") && it.targetChunkId ? (
                <button type="button" onClick={() => focus(it.targetChunkId as string)}>
                  <SquarePen size={12} /> 去修复
                </button>
              ) : null}
              {it.suggestedActions.includes("dismiss") ? (
                <button type="button" onClick={() => handleDismiss(it.id)}>
                  <X size={12} /> 忽略
                </button>
              ) : null}
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}


// ── Phase F · Today Mode：Digest 画布 + 任务侧栏 ──────────────────────────

interface DigestCardView {
  cardId: string;
  kind: string;
  title: string;
  summary: string;
  severity: string;
  suggestedAction: string;
  targetRefs?: Array<Record<string, unknown>>;
  metric?: { name?: string; value?: number; threshold?: number } | null;
}

interface DigestReportView {
  reportId?: string | null;
  workspaceId: string;
  accountId: string;
  reportDate: string;
  status: string;
  errorKind?: string | null;
  cards: DigestCardView[];
  dismissedCardIds: string[];
  generatedAt?: string;
  generatedBy?: string;
}

function severityBadgeClass(sev: string): string {
  return `wikiDigestBadge sev-${sev}`;
}

export function DigestCanvas() {
  const [report, setReport] = useState<DigestReportView | null>(null);
  const [pending, setPending] = useState(false);
  const [regen, setRegen] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [dismissing, setDismissing] = useState<Set<string>>(new Set());

  async function load() {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/knowledge/digest/today");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as DigestReportView;
      setReport(data);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
      setReport(null);
    } finally {
      setPending(false);
    }
  }

  async function regenerate() {
    setRegen(true);
    setError(null);
    try {
      const r = await fetch("/api/knowledge/digest/regenerate", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ force: true })
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as DigestReportView;
      setReport(data);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setRegen(false);
    }
  }

  async function dismiss(cardId: string) {
    setDismissing((s) => new Set(s).add(cardId));
    try {
      const r = await fetch(
        `/api/knowledge/digest/cards/${encodeURIComponent(cardId)}/dismiss`,
        { method: "POST" }
      );
      if (!r.ok) throw await parseApiError(r);
      setReport((prev) =>
        prev ? { ...prev, dismissedCardIds: [...prev.dismissedCardIds, cardId] } : prev
      );
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setDismissing((s) => {
        const next = new Set(s);
        next.delete(cardId);
        return next;
      });
    }
  }

  useEffect(() => {
    void load();
  }, []);

  const visibleCards = useMemo(() => {
    if (!report) return [];
    const dismissed = new Set(report.dismissedCardIds);
    return report.cards.filter((c) => !dismissed.has(c.cardId));
  }, [report]);

  return (
    <div className="wikiDigestCanvas">
      <div className="wikiDigestHead">
        <div>
          <h3>今日 Digest</h3>
          <span className="wikiDigestMeta">
            {report?.reportDate ?? "—"} · {report?.status ?? "—"} · 生成于 {report?.generatedAt ?? "—"}
          </span>
        </div>
        <div className="wikiDigestActions">
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> {pending ? "刷新中…" : "刷新"}
          </button>
          <button type="button" className="primary" onClick={() => void regenerate()} disabled={regen}>
            <Sparkles size={14} /> {regen ? "重算中…" : "强制重算"}
          </button>
        </div>
      </div>
      {error ? <LlmErrorBanner error={error} onRetry={() => void load()} retrying={pending} /> : null}
      {!error && visibleCards.length === 0 && !pending ? (
        <div className="wikiEmpty wikiDigestEmpty">
          <FileBox size={28} /> 今日暂无待办卡片。点击「强制重算」可立即合成。
        </div>
      ) : null}
      <div className="wikiDigestGrid">
        {visibleCards.map((card) => (
          <article className={`wikiDigestCard sev-${card.severity}`} key={card.cardId}>
            <div className="wikiDigestCardHead">
              <span className={severityBadgeClass(card.severity)}>{severityLabel(card.severity)}</span>
              <span className="wikiDigestKind">{card.kind}</span>
            </div>
            <h4 className="wikiDigestTitle">{card.title}</h4>
            <p className="wikiDigestSummary">{card.summary}</p>
            {card.metric && card.metric.name ? (
              <div className="wikiDigestMetric">
                {card.metric.name}：{card.metric.value ?? "—"}
                {card.metric.threshold !== undefined ? ` / 阈值 ${card.metric.threshold}` : ""}
              </div>
            ) : null}
            <div className="wikiDigestCardFoot">
              <span className="wikiDigestAction">建议：{card.suggestedAction}</span>
              <button
                type="button"
                className="wikiDigestDismiss"
                onClick={() => void dismiss(card.cardId)}
                disabled={dismissing.has(card.cardId)}
              >
                <X size={12} /> 忽略
              </button>
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}


interface ChatTaskView {
  taskId: string;
  sessionId: string;
  status: string;
  errorKind?: string | null;
  totalSteps: number;
  completedSteps: unknown[];
  cards: DigestCardView[];
  createdAt?: string;
  startedAt?: string | null;
  finishedAt?: string | null;
}

export function TaskRail() {
  const [sessionId, setSessionId] = useState("");
  const [task, setTask] = useState<ChatTaskView | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [liveTurns, setLiveTurns] = useState<number[]>([]);
  const esRef = useRef<EventSource | null>(null);

  function closeStream() {
    if (esRef.current) {
      esRef.current.close();
      esRef.current = null;
    }
  }

  function attachStream(sid: string) {
    closeStream();
    if (!sid || typeof window === "undefined" || typeof window.EventSource === "undefined") {
      return;
    }
    const es = new EventSource(
      `/api/knowledge/chat/sessions/${encodeURIComponent(sid)}/stream`
    );
    esRef.current = es;
    es.addEventListener("turn", (ev) => {
      const v = Number((ev as MessageEvent).data);
      if (!Number.isNaN(v)) setLiveTurns((prev) => [...prev, v]);
    });
    es.addEventListener("close", () => closeStream());
    es.addEventListener("error", () => closeStream());
  }

  useEffect(() => () => closeStream(), []);

  async function loadTask(taskId: string) {
    setPending(true);
    setError(null);
    try {
      const r = await fetch(`/api/knowledge/chat/tasks/${encodeURIComponent(taskId)}`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as ChatTaskView;
      setTask(data);
      attachStream(data.sessionId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  async function cancelTask() {
    if (!task) return;
    setPending(true);
    setError(null);
    try {
      const r = await fetch(
        `/api/knowledge/chat/tasks/${encodeURIComponent(task.taskId)}/cancel`,
        { method: "POST" }
      );
      if (!r.ok) throw await parseApiError(r);
      await loadTask(task.taskId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <aside className="wikiTaskRail">
      <div className="wikiTaskRailHead">
        <h3>派工跟踪</h3>
        <span className="wikiTaskRailHint">输入 taskId 查看长任务执行进度</span>
      </div>
      <div className="wikiTaskRailForm">
        <input
          type="text"
          className="wikiInput"
          placeholder="taskId（24 位 ObjectId）"
          value={sessionId}
          onChange={(e) => setSessionId(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && sessionId.trim()) void loadTask(sessionId.trim());
          }}
        />
        <button
          type="button"
          className="primary"
          disabled={pending || !sessionId.trim()}
          onClick={() => void loadTask(sessionId.trim())}
        >
          <Search size={14} /> 拉取
        </button>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {task ? (
        <div className="wikiTaskRailBody">
          <div className="wikiTaskCard">
            <div className="wikiTaskCardHead">
              <span className={`wikiTaskStatus s-${task.status}`}>{task.status}</span>
              <span className="wikiTaskMeta">
                {task.completedSteps.length}/{task.totalSteps} 步
              </span>
            </div>
            <div className="wikiTaskMeta wikiTaskMeta--small">session: {task.sessionId}</div>
            <div className="wikiTaskMeta wikiTaskMeta--small">
              开始：{task.startedAt ?? "—"} · 结束：{task.finishedAt ?? "—"}
            </div>
            {task.errorKind ? (
              <div className="wikiAlert error">errorKind: {task.errorKind}</div>
            ) : null}
            {task.cards.length > 0 ? (
              <div className="wikiTaskCardList">
                {task.cards.map((c) => (
                  <div className="wikiTaskCardEntry" key={c.cardId}>
                    <span className={severityBadgeClass(c.severity)}>{c.severity}</span>
                    <span className="wikiTaskCardTitle">{c.title}</span>
                  </div>
                ))}
              </div>
            ) : null}
            {task.status === "running" || task.status === "pending" ? (
              <button
                type="button"
                className="wikiTaskCancel"
                onClick={() => void cancelTask()}
                disabled={pending}
              >
                <X size={12} /> 取消
              </button>
            ) : null}
          </div>
          {liveTurns.length > 0 ? (
            <div className="wikiTaskLive">
              <div className="wikiTaskLiveHead">
                <Loader2 size={12} className="wikiTaskSpin" />
                实时 turn
              </div>
              <ol className="wikiTaskLiveList">
                {liveTurns.slice(-12).map((t, i) => (
                  <li key={`${t}-${i}`}>turn #{t}</li>
                ))}
              </ol>
            </div>
          ) : null}
        </div>
      ) : (
        <div className="wikiEmpty wikiTaskRailEmpty">
          暂无任务。在「探索」对话中派工后，可在此输入 taskId 跟踪。
        </div>
      )}
    </aside>
  );
}
