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
import { randomUuid } from "../../lib/uuid";
import { createSseReconnector, type SseHandle } from "../../lib/useSseReconnect";
import { LlmErrorBanner, focusChunk, loadChunkOptions } from "./shared";
import { ChunkPicker } from "../../components/ui/ChunkRef";
import { useConfirm } from "../../components/ui/ConfirmDialog";
import { useToast } from "../../components/ui/Toast";
import { EmptyState } from "../../components/ui/EmptyState";
import { useAccountStore } from "../../stores/accountStore";
import { severityLabel, priorityLabel, originLabel, draftKindLabel, taskStatusLabel, reportStatusLabel, digestCardKindLabel, digestSuggestedActionLabel, digestMetricNameLabel, digestTargetRefKindLabel, chatIntentLabel } from "./labels";

function withAccountScope(path: string, accountId: string): string {
  if (!accountId) return path;
  const separator = path.includes("?") ? "&" : "?";
  return `${path}${separator}${new URLSearchParams({ accountId }).toString()}`;
}

function chatSessionStorageKey(accountId: string): string {
  return `knowledgeChat.sessionId.${encodeURIComponent(accountId || "__default__")}`;
}

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
  plannedSteps?: DispatchStep[] | null;
  digestSelection?: DigestSelectionBinding | null;
  candidateHash?: string | null;
  missingFields?: string[];
  followupQuestions?: string[];
  canApply?: boolean;
  targetChunkId?: string | null;
  targetPackId?: string | null;
}

interface DispatchStep {
  stepId?: string;
  cardId?: string;
  action: string;
  summary?: string;
  targetChunkId?: string;
}

interface DigestSelectionBinding {
  accountId: string;
  reportId: string;
  reportDate: string;
  reportGeneration: number;
  reportHash: string;
  selectedCards: Array<{ cardId: string; cardHash: string }>;
}

interface PendingDispatchCandidate {
  plannedSteps: DispatchStep[];
  digestSelection: DigestSelectionBinding;
  candidateHash: string;
  sourceTurnIndex: number;
}

export function ChatWorkbench({ initialAttachChunkId }: { initialAttachChunkId?: string | null } = {}) {
  const confirm = useConfirm();
  const toast = useToast();
  const accountId = useAccountStore((state) => state.currentAccountId());
  const accountRef = useRef(accountId);
  const [sessionId, setSessionId] = useState("");
  const [draft, setDraft] = useState("");
  const [attachChunkId, setAttachChunkId] = useState<string>("");
  const [dispatching, setDispatching] = useState(false);
  const [pendingDispatch, setPendingDispatch] = useState<PendingDispatchCandidate | null>(null);

  // B2：从待办收件箱「找 AI 协作」跳转过来时预填 chunkId。
  useEffect(() => {
    if (initialAttachChunkId) setAttachChunkId(initialAttachChunkId);
  }, [initialAttachChunkId]);
  const [turns, setTurns] = useState<ChatTurnView[]>([]);
  const [pending, setPending] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    accountRef.current = accountId;
    if (typeof window === "undefined") return;
    const key = chatSessionStorageKey(accountId);
    let sid = window.localStorage.getItem(key) ?? "";
    if (!sid) {
      const legacySid = window.localStorage.getItem("knowledgeChat.sessionId") ?? "";
      if (legacySid) {
        sid = legacySid;
        window.localStorage.setItem(key, legacySid);
        window.localStorage.removeItem("knowledgeChat.sessionId");
      }
    }
    setSessionId(sid);
    setTurns([]);
    setPendingDispatch(null);
    setError(null);
    setInfo(null);
  }, [accountId]);

  const persistSession = useCallback((sid: string) => {
    if (typeof window === "undefined") return;
    const key = chatSessionStorageKey(accountId);
    if (sid) window.localStorage.setItem(key, sid);
    else window.localStorage.removeItem(key);
  }, [accountId]);

  const loadHistory = useCallback(async (sid: string) => {
    if (!sid) {
      setTurns([]);
      return;
    }
    const requestedAccountId = accountId;
    try {
      const r = await fetch(withAccountScope(
        `/api/operation-knowledge/chat/${encodeURIComponent(sid)}`,
        requestedAccountId,
      ));
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
      if (accountRef.current === requestedAccountId) setTurns(list);
    } catch (e) {
      if (accountRef.current === requestedAccountId) {
        setError(e instanceof Error ? e.message : String(e));
      }
    }
  }, [accountId]);

  useEffect(() => {
    if (sessionId) void loadHistory(sessionId);
  }, [sessionId, loadHistory]);

  useEffect(() => {
    if (!sessionId || typeof window === "undefined" || typeof window.EventSource === "undefined") return;
    const handle = createSseReconnector(
      withAccountScope(
        `/api/knowledge/chat/sessions/${encodeURIComponent(sessionId)}/stream`,
        accountId,
      ),
      { onEvent: { turn: () => { void loadHistory(sessionId); } }, terminalEvents: ["close"] },
    );
    return () => handle.close();
  }, [accountId, sessionId, loadHistory]);

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
    setPendingDispatch(null);
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
      const requestedAccountId = accountId;
      const body: Record<string, unknown> = { content, accountId: requestedAccountId };
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
      if (accountRef.current !== requestedAccountId) return;
      if (resp.sessionId !== sessionId) {
        setSessionId(resp.sessionId);
        persistSession(resp.sessionId);
      }
      if (Array.isArray(resp.plannedSteps) && resp.plannedSteps.length > 0) {
        if (resp.digestSelection && resp.candidateHash) {
          setPendingDispatch({
            plannedSteps: resp.plannedSteps,
            digestSelection: resp.digestSelection,
            candidateHash: resp.candidateHash,
            sourceTurnIndex: resp.turnIndex,
          });
        } else {
          setPendingDispatch(null);
          setError("派工候选缺少服务端快照绑定，请重新生成后再确认");
        }
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
          body: JSON.stringify({ accountId })
        }
      );
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as {
        result?: { createdChunkId?: string; updatedChunkId?: string; status?: string };
      };
      const result = data.result ?? {};
      const chunkId = result.createdChunkId || result.updatedChunkId;
      setInfo(`已应用为草稿（${result.status ?? "draft"}）${chunkId ? `：${chunkId}` : ""}`);
      if (chunkId) {
        window.dispatchEvent(
          new CustomEvent("wikiFocusChunk", { detail: { chunkId } })
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
        withAccountScope(
          `/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}/discard`,
          accountId,
        ),
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

  // E14：把当前会话的 plannedSteps 派工为长任务（后端串行 worker 执行）。
  async function confirmDispatch() {
    if (!sessionId || !pendingDispatch) return;
    setDispatching(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/chat/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          accountId,
          sessionId,
          digestSelection: pendingDispatch.digestSelection,
          sourceTurnIndex: pendingDispatch.sourceTurnIndex,
          candidateHash: pendingDispatch.candidateHash,
          plannedSteps: pendingDispatch.plannedSteps,
          cardIds: pendingDispatch.digestSelection.selectedCards.map((card) => card.cardId),
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { taskId?: string };
      setPendingDispatch(null);
      setInfo(`已派工长任务${data.taskId ? `：${data.taskId}` : ""}，可在右侧「派工跟踪」查看进度`);
      if (data.taskId) {
        window.dispatchEvent(new CustomEvent("wikiTrackTask", {
          detail: { taskId: data.taskId, accountId },
        }));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setDispatching(false);
    }
  }

  return (
    <div className="wikiArchiveShell wikiChatWorkbench">
      <header className="wikiArchiveHeader">
        <div>
          <h2>AI 协作工坊</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <span className="wikiArchiveTag">会话</span>
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
          <EmptyState
            icon={<MessageSquareText size={28} />}
            title="与 AI 协作起草知识"
            hint="让 AI 帮你起草或修订知识条目。AI 起草不会自动生效，需要你点击「应用为草稿」后再确认。"
          />
        ) : null}
        {turns.map((t) => (
          <article
            key={`${t.role}-${t.turnIndex}`}
            className={`wikiChatTurn wikiChatTurn--${t.role}`}
          >
            <div className="wikiChatTurnHead">
              <span className="wikiArchiveTag">{t.role === "user" ? "运营" : "AI"}</span>
              <span className="wikiArchiveTimelineTime">#{t.turnIndex}</span>
              {t.intent ? <span className="wikiArchiveTag">{chatIntentLabel(t.intent)}</span> : null}
              {t.draftKind ? <span className="wikiArchiveTag">{draftKindLabel(t.draftKind)}</span> : null}
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
          placeholder="向 AI 描述要起草 / 修订 / 拆分的知识，可在下方选择要引用的现有条目"
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
          <div className="wikiChatAttachPicker">
            <ChunkPicker
              value={attachChunkId}
              onChange={setAttachChunkId}
              loadChunks={loadChunkOptions}
              placeholder="可选：引用一条现有知识"
            />
          </div>
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
            title={lastAssistant?.canApply ? "把 AI 当前草稿保存为待确认草稿，需你确认后 AI 才会使用" : "当前没有可应用的草稿"}
          >
            <CheckCircle2 size={14} /> {applying ? "应用中…" : "应用为草稿"}
          </button>
          <button type="button" onClick={() => void discard()} disabled={!sessionId}>
            <Trash2 size={14} /> 丢弃草稿
          </button>
        </div>
        {pendingDispatch ? (
          <div className="wikiChatDispatch">
            <div className="wikiChatDispatchHead">
              <span className="wikiArchiveTag">待确认派工</span>
              <span className="wikiArchiveTimelineTime">AI 拆出 {pendingDispatch.plannedSteps.length} 步，确认后交后台执行</span>
            </div>
            <ul className="wikiChatFollowups">
              {pendingDispatch.plannedSteps.map((s, i) => (
                <li key={s.stepId ?? i}>{s.action} · {s.summary ?? ""}</li>
              ))}
            </ul>
            <div className="wikiChatFooterRow">
              <button type="button" className="primary" onClick={() => void confirmDispatch()} disabled={dispatching || !sessionId}>
                {dispatching ? "派工中…" : "确认派工"}
              </button>
              <button type="button" onClick={() => setPendingDispatch(null)} disabled={dispatching}>
                取消
              </button>
            </div>
          </div>
        ) : null}
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
    toast.success("已暂时忽略（刷新后恢复）");
  }

  return (
    <div className="wikiArchiveShell wikiInbox">
      <header className="wikiArchiveHeader">
        <div>
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
          <EmptyState
            icon={<Inbox size={28} />}
            title="暂无待办"
            hint="知识缺口、今日要点和质量信号会自动汇集到这里。"
          />
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
              {it.suggestedActions.includes("open_chat") ? (
                <button type="button" className="primary" onClick={() => handleOpenChat(it.targetChunkId ?? undefined)}>
                  <MessageSquareText size={12} /> 找 AI 协作
                </button>
              ) : it.targetChunkId ? (
                <button type="button" className="primary" onClick={() => focus(it.targetChunkId as string)}>
                  <ArrowRight size={12} /> 查看知识
                </button>
              ) : null}
              {it.suggestedActions.includes("open_chat") && it.targetChunkId ? (
                <button type="button" onClick={() => focus(it.targetChunkId as string)}>
                  <ArrowRight size={12} /> 查看知识
                </button>
              ) : null}
              {it.suggestedActions.includes("open_repair") && it.targetChunkId ? (
                <button type="button" onClick={() => focus(it.targetChunkId as string)}>
                  <SquarePen size={12} /> 去修复
                </button>
              ) : null}
              {it.suggestedActions.includes("dismiss") ? (
                <button type="button" className="wikiInboxDismiss" onClick={() => handleDismiss(it.id)}>
                  <X size={12} /> 暂时忽略
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
  cardHash?: string;
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
  reportHash?: string;
  workspaceId: string;
  accountId: string;
  reportDate: string;
  status: string;
  errorKind?: string | null;
  attemptGeneration?: number;
  currentGeneration?: number;
  latestAttemptStatus?: string | null;
  latestAttemptErrorKind?: string | null;
  latestAttemptAt?: string | null;
  lastSuccessAt?: string | null;
  cards: DigestCardView[];
  dismissedCardIds: string[];
  generatedAt?: string;
  generatedBy?: string;
}

function severityBadgeClass(sev: string): string {
  return `wikiDigestBadge sev-${sev}`;
}

/** 从后端 generatedAt 里裁出 HH:MM。
 *
 *  后端给的是 `2026-08-10 9:02:11.81 +00:00:00` 这类原始串，整串直插界面会
 *  重复日期（前面已有 reportDate）、还把毫秒和 `+00:00:00` 时区后缀暴露给运营。
 *  这里只做字符串裁剪、不 new Date()：该串不是合法 ISO8601（空格分隔 + 三段时区），
 *  Safari 下 Date 解析会得到 Invalid Date。裁不出来就返回 null，由调用方回退显示原值，
 *  宁可难看也不显示错误时间。 */
export function digestGeneratedClock(raw?: string | null): string | null {
  if (!raw) return null;
  const m = /(\d{1,2}):(\d{2})/.exec(raw);
  if (!m) return null;
  return `${m[1].padStart(2, "0")}:${m[2]}`;
}

export interface DigestTargetRefChip {
  /** 原始 kind，仅用于 React key，不上屏。 */
  kind: string;
  /** 完整 id，放进 title 属性供悬停查看/复制。 */
  id: string;
  /** 中文类型名，如「切片」。 */
  kindLabel: string;
  /** 尾 6 位缩写，如 `…34567`。 */
  shortId: string;
}

/** 卡片的 targetRefs → 去重后的渲染用短标签（最多 3 条，超出的丢弃）。
 *
 *  **为什么必须渲染**：`cardId` 由
 *  `(account_id, report_date, kind, target_refs, title)` 派生（后端
 *  `knowledge_digest/mod.rs::stable_card_id`）。两张 kind/title 完全相同的卡片能
 *  同时存在，恰恰证明它们的 `target_refs` 不同——指向不同切片。此前前端只在 TS
 *  类型里声明 `targetRefs` 却从不上屏，运营看到两张一模一样的卡，无从判断该勾哪
 *  张、勾了会动哪条知识。
 *
 *  只取 id 尾 6 位：ObjectId 是 24 位 hex，前 8 位是秒级时间戳，同一批生成的卡片
 *  高度重复，差异集中在尾部的计数器段。尾部足以区分，又不会把卡片撑爆；完整 id
 *  仍通过 `title` 属性可查。 */
export function digestTargetRefLabels(
  refs?: Array<Record<string, unknown>>,
): DigestTargetRefChip[] {
  if (!refs || refs.length === 0) return [];
  const seen = new Set<string>();
  const chips: DigestTargetRefChip[] = [];
  for (const ref of refs) {
    const id = typeof ref.id === "string" ? ref.id.trim() : "";
    if (!id) continue;
    const kind = typeof ref.kind === "string" ? ref.kind : "";
    const dedupeKey = `${kind}:${id}`;
    if (seen.has(dedupeKey)) continue;
    seen.add(dedupeKey);
    const kindLabel = digestTargetRefKindLabel(kind);
    chips.push({
      kind,
      id,
      // kind 缺失时 digestTargetRefKindLabel 返回 "—"，那个破折号单独挂在 id 前
      // 只是噪声，置空让 CSS 的 gap 自然收拢。
      kindLabel: kindLabel === "—" ? "" : kindLabel,
      shortId: id.length > 6 ? `…${id.slice(-6)}` : id,
    });
    if (chips.length >= 3) break;
  }
  return chips;
}

export function DigestCanvas() {
  const accountId = useAccountStore((state) => state.currentAccountId());
  const [report, setReport] = useState<DigestReportView | null>(null);
  const [pending, setPending] = useState(false);
  const [regen, setRegen] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [dismissing, setDismissing] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [dispatchingBatch, setDispatchingBatch] = useState(false);

  function toggleSelect(cardId: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(cardId)) next.delete(cardId);
      else next.add(cardId);
      return next;
    });
  }

  async function dispatchSelected() {
    if (selected.size === 0) return;
    const selectedCards = visibleCards.filter(
      (card) => selected.has(card.cardId) && card.suggestedAction !== "freeform",
    );
    if (selectedCards.length === 0) {
      setError(new Error("选中的卡片都不可派工（仅查看类卡片无执行动作）"));
      return;
    }
    if (
      !report?.reportId ||
      !report.reportHash ||
      report.currentGeneration === undefined ||
      selectedCards.some((card) => !card.cardHash)
    ) {
      setError(new Error("当前日报缺少服务端快照绑定，请刷新后重新选择"));
      return;
    }
    const digestSelection: DigestSelectionBinding = {
      accountId: report.accountId,
      reportId: report.reportId,
      reportDate: report.reportDate,
      reportGeneration: report.currentGeneration,
      reportHash: report.reportHash,
      selectedCards: selectedCards.map((card) => ({
        cardId: card.cardId,
        cardHash: card.cardHash as string,
      })),
    };
    setDispatchingBatch(true);
    setError(null);
    try {
      // 必须走 randomUuid()：裸 crypto.randomUUID 在非安全上下文（生产是
      // HTTP + IP）是 undefined，此处会抛 TypeError，派工请求根本发不出去。
      const sessionId = randomUuid();
      const r = await fetch("/api/knowledge/chat/tasks", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          accountId: report.accountId,
          sessionId,
          digestSelection,
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { taskId?: string };
      setSelected(new Set());
      if (data.taskId) {
        window.dispatchEvent(new CustomEvent("wikiTrackTask", {
          detail: { taskId: data.taskId, accountId },
        }));
      }
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setDispatchingBatch(false);
    }
  }

  async function load() {
    const requestedAccountId = accountId;
    setPending(true);
    setError(null);
    try {
      const r = await fetch(withAccountScope("/api/knowledge/digest/today", requestedAccountId));
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as DigestReportView;
      if (useAccountStore.getState().currentAccountId() !== requestedAccountId) return;
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
        body: JSON.stringify({ accountId, force: true })
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
    if (!report?.accountId) {
      setError(new Error("当前日报缺少账号标识，无法忽略卡片"));
      return;
    }
    setDismissing((s) => new Set(s).add(cardId));
    try {
      const query = new URLSearchParams({ accountId: report.accountId });
      const r = await fetch(
        `/api/knowledge/digest/cards/${encodeURIComponent(cardId)}/dismiss?${query.toString()}`,
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
    setReport(null);
    setSelected(new Set());
    void load();
    // load intentionally follows the account snapshot for this render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId]);

  const visibleCards = useMemo(() => {
    if (!report) return [];
    const dismissed = new Set(report.dismissedCardIds);
    return report.cards.filter((c) => !dismissed.has(c.cardId));
  }, [report]);

  return (
    <div className="wikiDigestCanvas">
      <div className="wikiDigestHead">
        <div className="wikiDigestHeadText">
          <h3>今日摘要</h3>
          {/* 日期只出现一次：generatedAt 已裁到 HH:MM，与前面的 reportDate 互补
              而不重复。裁不出时钟段时才回退显示原值。 */}
          <span className="wikiDigestMeta">
            {report?.reportDate ?? "—"}
            <span className="wikiDigestMetaDot" aria-hidden="true">·</span>
            {reportStatusLabel(report?.status)}
            {report?.generatedAt ? (
              <>
                <span className="wikiDigestMetaDot" aria-hidden="true">·</span>
                {digestGeneratedClock(report.generatedAt)
                  ? `${digestGeneratedClock(report.generatedAt)} 生成`
                  : `生成于 ${report.generatedAt}`}
              </>
            ) : null}
            {visibleCards.length > 0 ? (
              <>
                <span className="wikiDigestMetaDot" aria-hidden="true">·</span>
                {`${visibleCards.length} 张待办`}
              </>
            ) : null}
          </span>
        </div>
        {/* 按钮层级：只有「批量派工」是主操作（蓝底）。原先它与「强制重算」都是
            primary，两个蓝底并排互相争抢，而重算是低频的补救动作、刷新更是次要。
            现在重算降为次级描边按钮，与刷新同级。 */}
        <div className="wikiDigestActions">
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> {pending ? "刷新中…" : "刷新"}
          </button>
          <button type="button" onClick={() => void regenerate()} disabled={regen}>
            <Sparkles size={14} /> {regen ? "重算中…" : "强制重算"}
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => void dispatchSelected()}
            disabled={dispatchingBatch || selected.size === 0}
          >
            {dispatchingBatch ? "派工中…" : `批量派工（${selected.size}）`}
          </button>
        </div>
      </div>
      {/* onRetry 绑的是 load()——重新拉取今日摘要，**不是**重发派工。所以本地故障
          （kind=client_error，如派工前的浏览器端异常）时按钮必须写「重新加载」而非
          「AI 重试」：后者会让运营以为派工已经又发了一遍，而实际上什么都没发出去。 */}
      {error ? (
        <LlmErrorBanner
          error={error}
          onRetry={() => void load()}
          retrying={pending}
          retryActionLabel="重新加载"
        />
      ) : null}
      {!error && report?.latestAttemptStatus && report.latestAttemptStatus !== "ok" ? (
        <div className="wikiBannerError" role="alert">
          最近重算{report.latestAttemptStatus === "running" ? "仍在进行" : "未成功"}
          {report.latestAttemptErrorKind ? `（${report.latestAttemptErrorKind}）` : ""}；
          {report.status === "ok" ? "当前继续展示上次成功结果。" : "当前没有可用的成功结果。"}
        </div>
      ) : null}
      {!error && visibleCards.length === 0 && !pending ? (
        <EmptyState
          icon={<FileBox size={28} />}
          title="今日暂无待办卡片"
          hint="点击右上角「强制重算」可立即重新生成今日要点。"
        />
      ) : null}
      <div className="wikiDigestGrid">
        {visibleCards.map((card) => {
          const refs = digestTargetRefLabels(card.targetRefs);
          return (
          <article
            className={`wikiDigestCard sev-${card.severity}${selected.has(card.cardId) ? " is-selected" : ""}`}
            key={card.cardId}
          >
            <div className="wikiDigestCardHead">
              {/* 复选框包在 label 里：原先是裸 input，只有 13px 的方块可点，
                  且与右侧徽章基线对不齐。包起来后整个「勾选区」都是热区。
                  aria-label 必须保留在 input 上（批量派工用例按无障碍名定位它）。 */}
              <label className="wikiDigestPick">
                <input
                  type="checkbox"
                  checked={selected.has(card.cardId)}
                  onChange={() => toggleSelect(card.cardId)}
                  disabled={card.suggestedAction === "freeform"}
                  aria-label={`选择卡片 ${card.title}`}
                />
              </label>
              <span className={severityBadgeClass(card.severity)}>{severityLabel(card.severity)}</span>
              <span className="wikiDigestKind">{digestCardKindLabel(card.kind)}</span>
            </div>
            <h4 className="wikiDigestTitle">{card.title}</h4>
            <p className="wikiDigestSummary">{card.summary}</p>
            {/* 目标对象必须显式渲染。cardId 由 (kind, target_refs, title) 派生，
                所以两张 kind/title 相同的卡片一定指向**不同**切片；此前 targetRefs
                只在 TS 类型里声明、从不上屏，运营看到两张完全一样的卡，无从判断
                该勾哪张、勾了会动哪条知识。 */}
            {refs.length > 0 ? (
              <div className="wikiDigestRefs">
                {refs.map((ref) => (
                  <span className="wikiDigestRef" key={`${ref.kind}:${ref.id}`} title={`${ref.kindLabel} ${ref.id}`}>
                    <span className="wikiDigestRefKind">{ref.kindLabel}</span>
                    <span className="wikiDigestRefId">{ref.shortId}</span>
                  </span>
                ))}
              </div>
            ) : null}
            {card.metric && card.metric.name ? (
              <div className="wikiDigestMetric">
                <span className="wikiDigestMetricName">{digestMetricNameLabel(card.metric.name)}</span>
                <span className="wikiDigestMetricValue">{card.metric.value ?? "—"}</span>
                {/* 阈值 0 不渲染：`threshold !== undefined` 会让「阈值 0」上屏，而
                    「缺字段数 2 · 阈值 0」对运营零信息量——阈值 0 的语义是「只要有就算
                    问题」，此时根本不存在需要对比的门线。同理排除 null。 */}
                {typeof card.metric.threshold === "number" && card.metric.threshold !== 0 ? (
                  <span className="wikiDigestMetricThreshold">阈值 {card.metric.threshold}</span>
                ) : null}
              </div>
            ) : null}
            <div className="wikiDigestCardFoot">
              <span className="wikiDigestAction">
                {digestSuggestedActionLabel(card.suggestedAction)}
              </span>
              <button
                type="button"
                className="wikiDigestDismiss"
                onClick={() => void dismiss(card.cardId)}
                disabled={dismissing.has(card.cardId)}
              >
                {dismissing.has(card.cardId) ? "忽略中…" : "忽略"}
              </button>
            </div>
          </article>
          );
        })}
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

interface ChatTaskListItem {
  taskId: string;
  sessionId: string;
  status: string;
  errorKind?: string | null;
  totalSteps: number;
  completedStepCount: number;
  createdAt?: string;
  startedAt?: string | null;
  finishedAt?: string | null;
}

const CHAT_TASK_TERMINAL_STATUSES = new Set(["completed", "failed", "cancelled"]);
const TASK_FALLBACK_POLL_INTERVAL_MS = 5_000;
const TASK_FALLBACK_POLL_MAX_ATTEMPTS = 12;

export function TaskRail() {
  const toast = useToast();
  const accountId = useAccountStore((state) => state.currentAccountId());
  const [sessionId, setSessionId] = useState("");
  const [task, setTask] = useState<ChatTaskView | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [liveTurns, setLiveTurns] = useState<number[]>([]);
  const [taskList, setTaskList] = useState<ChatTaskListItem[]>([]);
  const [streamNotice, setStreamNotice] = useState("");
  const sseRef = useRef<SseHandle | null>(null);
  const trackedTaskIdRef = useRef("");
  const snapshotGenerationRef = useRef(0);
  const fallbackPollTimerRef = useRef<number | null>(null);
  const fallbackPollGenerationRef = useRef(0);

  async function loadTaskList() {
    try {
      const r = await fetch(withAccountScope("/api/knowledge/chat/tasks", accountId));
      if (!r.ok) {
        // 列表失败不阻塞手工跟踪，静默降级 + 轻量提示
        toast.error("任务列表加载失败，可手动输入任务 ID");
        return;
      }
      const data = (await r.json()) as { items?: ChatTaskListItem[] };
      setTaskList(data.items ?? []);
    } catch {
      // 列表拉取失败：保留手工输入 fallback + 轻量提示
      toast.error("任务列表加载失败，可手动输入任务 ID");
    }
  }

  useEffect(() => {
    trackedTaskIdRef.current = "";
    snapshotGenerationRef.current += 1;
    closeStream();
    stopFallbackPolling();
    setSessionId("");
    setTask(null);
    setTaskList([]);
    setLiveTurns([]);
    setStreamNotice("");
    setError(null);
    void loadTaskList();
    // Reload and clear all task state whenever the selected account changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId]);

  function closeStream() {
    sseRef.current?.close();
    sseRef.current = null;
  }

  function stopFallbackPolling() {
    fallbackPollGenerationRef.current += 1;
    if (fallbackPollTimerRef.current !== null) {
      window.clearTimeout(fallbackPollTimerRef.current);
      fallbackPollTimerRef.current = null;
    }
  }

  function settleTerminalTask(data: ChatTaskView) {
    if (!CHAT_TASK_TERMINAL_STATUSES.has(data.status)) return false;
    closeStream();
    stopFallbackPolling();
    setStreamNotice("");
    void loadTaskList();
    return true;
  }

  async function fetchTaskSnapshot(taskId: string, showPending = false) {
    const snapshotGeneration = ++snapshotGenerationRef.current;
    if (showPending) setPending(true);
    if (showPending) setError(null);
    try {
      const r = await fetch(withAccountScope(
        `/api/knowledge/chat/tasks/${encodeURIComponent(taskId)}`,
        accountId,
      ));
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as ChatTaskView;
      if (
        trackedTaskIdRef.current !== taskId ||
        snapshotGeneration !== snapshotGenerationRef.current
      ) return null;
      setError(null);
      setTask(data);
      settleTerminalTask(data);
      return data;
    } catch (e) {
      if (
        trackedTaskIdRef.current === taskId &&
        snapshotGeneration === snapshotGenerationRef.current
      ) {
        setError(e instanceof Error ? e.message : String(e));
      }
      return null;
    } finally {
      if (
        showPending &&
        trackedTaskIdRef.current === taskId &&
        snapshotGeneration === snapshotGenerationRef.current
      ) setPending(false);
    }
  }

  function startFallbackPolling(taskId: string) {
    stopFallbackPolling();
    const generation = fallbackPollGenerationRef.current;
    let attempts = 0;
    setStreamNotice("实时连接已中断，正在通过任务接口核对最新状态…");

    const poll = async () => {
      if (
        generation !== fallbackPollGenerationRef.current ||
        trackedTaskIdRef.current !== taskId
      ) return;
      attempts += 1;
      const data = await fetchTaskSnapshot(taskId);
      if (
        generation !== fallbackPollGenerationRef.current ||
        trackedTaskIdRef.current !== taskId ||
        (data && CHAT_TASK_TERMINAL_STATUSES.has(data.status))
      ) return;
      if (attempts >= TASK_FALLBACK_POLL_MAX_ATTEMPTS) {
        fallbackPollTimerRef.current = null;
        setStreamNotice("实时连接与自动核对均已停止，请点击“拉取”获取最新状态。");
        return;
      }
      fallbackPollTimerRef.current = window.setTimeout(
        () => void poll(),
        TASK_FALLBACK_POLL_INTERVAL_MS,
      );
    };
    void poll();
  }

  function attachStream(sid: string, taskId: string) {
    closeStream();
    if (!sid || typeof window === "undefined") return;
    if (typeof window.EventSource === "undefined") {
      startFallbackPolling(taskId);
      return;
    }
    setStreamNotice("正在连接实时进度…");
    sseRef.current = createSseReconnector(
      withAccountScope(`/api/knowledge/chat/sessions/${encodeURIComponent(sid)}/stream`, accountId),
      {
        onEvent: {
          turn: (ev) => {
            const v = Number(ev.data);
            if (!Number.isNaN(v)) setLiveTurns((prev) => [...prev, v]);
            void fetchTaskSnapshot(taskId);
          },
          close: () => { void fetchTaskSnapshot(taskId); },
        },
        terminalEvents: ["close"],
        onOpen: () => { setStreamNotice(""); },
        onReconnecting: (attempt) => {
          setStreamNotice(`实时连接中断，正在第 ${attempt} 次重连…`);
        },
        onGaveUp: () => { startFallbackPolling(taskId); },
      },
    );
  }

  useEffect(() => () => {
    closeStream();
    stopFallbackPolling();
  }, []);

  // E14：ChatWorkbench 派工成功后广播 wikiTrackTask，自动填入并跟踪新任务。
  useEffect(() => {
    function onTrack(ev: Event) {
      const detail = (ev as CustomEvent<{ taskId?: string; accountId?: string }>).detail;
      const taskId = detail?.taskId;
      if (taskId && (!detail.accountId || detail.accountId === accountId)) {
        setSessionId(taskId);
        void loadTask(taskId);
        void loadTaskList();
      }
    }
    window.addEventListener("wikiTrackTask", onTrack as EventListener);
    return () => window.removeEventListener("wikiTrackTask", onTrack as EventListener);
    // loadTask 为稳定闭包（仅引用 setter）；空依赖避免重复绑定。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId]);

  async function loadTask(taskId: string) {
    const normalized = taskId.trim();
    if (!normalized) return;
    trackedTaskIdRef.current = normalized;
    snapshotGenerationRef.current += 1;
    closeStream();
    stopFallbackPolling();
    setStreamNotice("");
    setLiveTurns([]);
    const data = await fetchTaskSnapshot(normalized, true);
    if (data && !CHAT_TASK_TERMINAL_STATUSES.has(data.status)) {
      attachStream(data.sessionId, normalized);
    }
  }

  async function cancelTask() {
    if (!task) return;
    setPending(true);
    setError(null);
    try {
      const r = await fetch(
        withAccountScope(
          `/api/knowledge/chat/tasks/${encodeURIComponent(task.taskId)}/cancel`,
          accountId,
        ),
        { method: "POST" }
      );
      if (!r.ok) throw await parseApiError(r);
      await loadTask(task.taskId);
      void loadTaskList();
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
        <span className="wikiTaskRailHint">输入任务编号查看长任务执行进度</span>
      </div>
      <div className="wikiTaskRailForm">
        <input
          type="text"
          className="wikiInput"
          placeholder="粘贴任务编号"
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
      {taskList.length > 0 ? (
        <ul className="wikiTaskRailList">
          {taskList.map((t) => (
            <li key={t.taskId}>
              <button
                type="button"
                className={`wikiTaskRailListItem${task?.taskId === t.taskId ? " active" : ""}`}
                onClick={() => { setSessionId(t.taskId); void loadTask(t.taskId); }}
              >
                <span className={`wikiTaskStatus s-${t.status}`}>{taskStatusLabel(t.status)}</span>
                <span className="wikiTaskRailListSess">{t.sessionId}</span>
                <span className="wikiTaskRailListSteps">{t.completedStepCount}/{t.totalSteps}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {task ? (
        <div className="wikiTaskRailBody">
          <div className="wikiTaskCard">
            <div className="wikiTaskCardHead">
              <span className={`wikiTaskStatus s-${task.status}`}>{taskStatusLabel(task.status)}</span>
              <span className="wikiTaskMeta">
                {task.completedSteps.length}/{task.totalSteps} 步
              </span>
            </div>
            {task.totalSteps > 0 ? (
              <div className="wikiTaskProgress" role="progressbar" aria-valuenow={task.completedSteps.length} aria-valuemin={0} aria-valuemax={task.totalSteps}>
                <div
                  className="wikiTaskProgressFill"
                  style={{ width: `${Math.min(100, Math.round((task.completedSteps.length / task.totalSteps) * 100))}%` }}
                />
              </div>
            ) : null}
            <div className="wikiTaskMeta wikiTaskMeta--small">会话：{task.sessionId}</div>
            <div className="wikiTaskMeta wikiTaskMeta--small">
              开始：{task.startedAt ?? "—"} · 结束：{task.finishedAt ?? "—"}
            </div>
            {streamNotice ? (
              <div className="wikiTaskMeta wikiTaskMeta--small" role="status">
                {streamNotice}
              </div>
            ) : null}
            {task.errorKind ? (
              <div className="wikiAlert error">执行出错：{task.errorKind}</div>
            ) : null}
            {task.cards.length > 0 ? (
              <div className="wikiTaskCardList">
                {task.cards.map((c) => (
                  <div className="wikiTaskCardEntry" key={c.cardId}>
                    <span className={severityBadgeClass(c.severity)}>{severityLabel(c.severity)}</span>
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
                实时进度
              </div>
              <ol className="wikiTaskLiveList">
                {liveTurns.slice(-12).map((t, i) => (
                  <li key={`${t}-${i}`}>第 {t} 步</li>
                ))}
              </ol>
            </div>
          ) : null}
        </div>
      ) : (
        /* 不用共享 <EmptyState/>：它是为主区设计的（28px 图标 + 虚线框 + 28px 内边距），
           塞进 200px 的左栏只剩 ~184px 可用宽，图标和虚线框把两行提示挤成四行，
           比「没有内容」这件事本身还显眼。这里用一行小字说明即可。 */
        <p className="wikiTaskRailEmpty">
          在「AI 协作」里派发长任务后，可在此跟踪进度。
        </p>
      )}
    </aside>
  );
}
