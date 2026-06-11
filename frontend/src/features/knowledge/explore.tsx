import { useState, useEffect, useRef, useMemo, type FormEvent } from "react";
import { ChevronDown, ChevronRight, Clock3, RefreshCw, Sparkles } from "lucide-react";
import { parseApiError } from "../../lib/api";
import { numberOr, stringOr, type TreeChunkItem } from "./shared";
import { EmptyState } from "../../components/ui/EmptyState";
import { wikiTypeLabel, statusLabel, integrityStatusLabel } from "./labels";

interface AskSourceQuote {
  chunkId: string;
  quote: string;
  sourceAnchorIndex?: number | null;
}
interface AskToolTraceStep {
  tool: string;
  [key: string]: unknown;
}
interface AskResult {
  answer: string;
  citedChunkIds: string[];
  sourceQuotes: AskSourceQuote[];
  toolTrace: AskToolTraceStep[];
  roundsUsed: number;
  truncated: boolean;
  tookMs: number;
}

// AskView：把 /api/knowledge/ask 包装成"输入 → answer + cited 卡片 + tool_trace 时间线"。
//
// 设计要点：
//   - 默认实时模式（streamMode）走 SSE: trace → trace → ... → answer → close；
//     时间线渐进出现，运营可看到 agent 在哪一步、为什么没收敛
//   - 浏览器无 EventSource 或显式关掉实时模式 → 走原一次性 fetch
//   - cited 卡片折叠展开，展开后显示 source_quote 黄边引用块（与 Review 视图对齐）
//   - tool_trace 实时模式默认展开（让运营看到进度）；非实时模式下默认收起
export function AskView() {
  const supportsEventSource = typeof window !== "undefined" && typeof window.EventSource !== "undefined";
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<AskResult | null>(null);
  const [liveTrace, setLiveTrace] = useState<AskToolTraceStep[]>([]);
  const [streamText, setStreamText] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [streamMode, setStreamMode] = useState(supportsEventSource);
  const [showTrace, setShowTrace] = useState(false);
  const [openCited, setOpenCited] = useState<Set<string>>(new Set());
  // E6：workspace 显式覆盖。空字符串 → 后端用 default_workspace_id；
  // localStorage 持久化，方便多租户切换后保留选择。
  const [workspaceId, setWorkspaceId] = useState<string>(() => {
    if (typeof window === "undefined") return "";
    return window.localStorage.getItem("knowledgeAsk.workspaceId") ?? "";
  });
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (workspaceId) {
      window.localStorage.setItem("knowledgeAsk.workspaceId", workspaceId);
    } else {
      window.localStorage.removeItem("knowledgeAsk.workspaceId");
    }
  }, [workspaceId]);

  // 组件卸载/重新提交时关掉旧 EventSource，避免连接泄漏。
  useEffect(() => () => {
    esRef.current?.close();
    esRef.current = null;
  }, []);

  function resetForSubmit() {
    setError(null);
    setResult(null);
    setLiveTrace([]);
    setStreamText("");
    setOpenCited(new Set());
    esRef.current?.close();
    esRef.current = null;
  }

  async function submit(e?: FormEvent<HTMLFormElement>) {
    e?.preventDefault();
    const q = query.trim();
    if (!q) {
      setError("请输入问题。");
      return;
    }
    if (streamMode && supportsEventSource) {
      submitStream(q);
      return;
    }
    setPending(true);
    resetForSubmit();
    setShowTrace(false);
    try {
      const r = await fetch("/api/knowledge/ask", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(workspaceId ? { query: q, workspaceId } : { query: q }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as AskResult;
      setResult(data);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setPending(false);
    }
  }

  // 实时路径：EventSource 监听 trace / answer / error / close 四类事件。
  // 后端 ask_knowledge_stream 的 event:trace data 直接是 payload JSON（含 tool 字段）。
  function submitStream(q: string) {
    setPending(true);
    resetForSubmit();
    setShowTrace(true);
    const startedAt = Date.now();
    const params = new URLSearchParams({ query: q });
    if (workspaceId) params.set("workspaceId", workspaceId);
    const url = `/api/knowledge/ask/stream?${params.toString()}`;
    const es = new EventSource(url);
    esRef.current = es;

    es.addEventListener("trace", (ev) => {
      try {
        const payload = JSON.parse((ev as MessageEvent).data) as AskToolTraceStep;
        setLiveTrace((prev) => [...prev, payload]);
      } catch {
        // 单帧坏 JSON 不致命，忽略后续依赖 close 兜底
      }
    });
    es.addEventListener("token", (ev) => {
      try {
        const data = JSON.parse((ev as MessageEvent).data) as { delta?: string };
        if (typeof data.delta === "string") {
          setStreamText((prev) => prev + data.delta);
        }
      } catch {
        // 单帧坏 JSON 不致命；最终 answer 帧会兜底
      }
    });
    es.addEventListener("answer", (ev) => {
      try {
        const data = JSON.parse((ev as MessageEvent).data) as Omit<AskResult, "tookMs"> & {
          tookMs?: number;
        };
        setResult({ ...data, tookMs: data.tookMs ?? Date.now() - startedAt });
      } catch (err) {
        setError(err instanceof Error ? err.message : "解析 answer 帧失败");
      }
    });
    es.addEventListener("error", () => {
      // 浏览器在 close 后也会触发 error；只在还没拿到 answer 时报警，避免误报。
      if (!result) {
        setError("流式连接错误（请关闭实时模式或重试）");
      }
      es.close();
      esRef.current = null;
      setPending(false);
    });
    es.addEventListener("close", () => {
      es.close();
      esRef.current = null;
      setPending(false);
    });
  }

  // 用户中断：关闭 EventSource 即让后端 SSE body drop → 取消信号置位，
  // agent 在下一个 cancel checkpoint 自行收尾并发出 cancelled answer 帧。
  // 此处前端不等 answer 帧，直接把 pending 置 false，UI 立即解锁。
  function cancelStream() {
    esRef.current?.close();
    esRef.current = null;
    setPending(false);
    setError(null);
  }

  function toggleCited(id: string) {
    if (typeof window !== "undefined") {
      window.dispatchEvent(new CustomEvent("wikiFocusChunk", { detail: { chunkId: id } }));
    }
    setOpenCited((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  // chunk_id → quote 索引，方便在 cited 卡片里直接渲染引用。
  const quoteByChunk = useMemo(() => {
    const m = new Map<string, AskSourceQuote>();
    if (result) {
      for (const q of result.sourceQuotes) m.set(q.chunkId, q);
    }
    return m;
  }, [result]);

  // 时间线源：实时模式跑过 trace 就用 liveTrace，否则用 result.toolTrace。
  const traceSteps: AskToolTraceStep[] = result
    ? streamMode && liveTrace.length > 0
      ? liveTrace
      : result.toolTrace
    : liveTrace;

  return (
    <div className="wikiPanelBody">
      <form className="wikiAskForm" onSubmit={submit}>
        <textarea
          className="wikiAskInput"
          placeholder="向知识库提一个问题（例如：产品保修政策是怎么规定的？）"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          rows={3}
          disabled={pending}
        />
        <div className="wikiAskActions">
          <span className="wikiHint">AI 会自动检索知识库后作答。</span>
          <label className="wikiAskWsField">
            租户（可选）
            <input
              type="text"
              value={workspaceId}
              onChange={(e) => setWorkspaceId(e.target.value)}
              placeholder="default"
              disabled={pending}
            />
          </label>
          {supportsEventSource ? (
            <label className="wikiAskModeToggle" title="开启后可实时看到 AI 检索和作答的过程">
              <input
                type="checkbox"
                checked={streamMode}
                onChange={(e) => setStreamMode(e.target.checked)}
                disabled={pending}
              />
              实时模式
            </label>
          ) : null}
          <button type="submit" className="primary" disabled={pending || !query.trim()}>
            <Sparkles size={14} />
            {pending ? "思考中…" : "提问"}
          </button>
          {pending && streamMode ? (
            <button
              type="button"
              className="wikiAskCancelBtn"
              onClick={cancelStream}
            >
              中断
            </button>
          ) : null}
        </div>
      </form>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {pending && streamMode && traceSteps.length > 0 ? (
        <ol className="wikiToolTraceList">
          {traceSteps.map((step, i) => (
            <li key={i} className="wikiToolTraceStep wikiToolTraceStep--live">
              <span className={`wikiToolTraceTool tool-${step.tool}`}>{step.tool}</span>
              <code>{JSON.stringify(stripTool(step))}</code>
            </li>
          ))}
        </ol>
      ) : null}
      {pending && streamMode && streamText ? (
        <div className="wikiAskStreamingAnswer" aria-live="polite">
          {streamText}
          <span className="wikiAskStreamingCaret" aria-hidden="true" />
        </div>
      ) : null}
      {result ? (
        <div className="wikiAskResult">
          <div className="wikiAskMeta">
            <span>
              <Clock3 size={12} /> {result.tookMs} ms
            </span>
            <span>轮次：{result.roundsUsed}/3</span>
            {result.truncated ? (
              <span className="wikiBadge warn">已截断</span>
            ) : null}
            <span>引用：{result.citedChunkIds.length}</span>
          </div>
          <div className="wikiAskAnswer">{result.answer || "（agent 未给出文本回答）"}</div>
          {result.citedChunkIds.length > 0 ? (
            <div className="wikiCitedList">
              <div className="wikiCitedTitle">引用 chunks</div>
              {result.citedChunkIds.map((cid) => {
                const q = quoteByChunk.get(cid);
                const open = openCited.has(cid);
                return (
                  <div key={cid} className="wikiCitedCard">
                    <button
                      type="button"
                      className="wikiCitedHead"
                      onClick={() => toggleCited(cid)}
                    >
                      {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      <code className="wikiCitedId">{cid}</code>
                      {q ? (
                        <span className="wikiCitedHint">含 source_quote</span>
                      ) : (
                        <span className="wikiCitedHint muted">无 source_quote</span>
                      )}
                    </button>
                    {open && q ? (
                      <blockquote className="wikiCitedQuote">{q.quote}</blockquote>
                    ) : null}
                    {open && !q ? (
                      <p className="wikiHint">该引用未配 source_quote；请在 Review 视图补齐。</p>
                    ) : null}
                  </div>
                );
              })}
            </div>
          ) : null}
          <div className="wikiToolTrace">
            <button
              type="button"
              className="wikiToolTraceToggle"
              onClick={() => setShowTrace((v) => !v)}
            >
              {showTrace ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              工具调用时间线（{traceSteps.length} 步）
            </button>
            {showTrace ? (
              <ol className="wikiToolTraceList">
                {traceSteps.map((step, i) => (
                  <li key={i} className="wikiToolTraceStep">
                    <span className={`wikiToolTraceTool tool-${step.tool}`}>{step.tool}</span>
                    <code>{JSON.stringify(stripTool(step))}</code>
                  </li>
                ))}
              </ol>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function stripTool(step: AskToolTraceStep): Record<string, unknown> {
  const { tool: _tool, ...rest } = step;
  void _tool;
  return rest;
}


const WIKI_TYPES_ORDER: { v: string; label: string }[] = [
  { v: "source", label: "原始资料 source" },
  { v: "entity", label: "实体 entity" },
  { v: "concept", label: "概念 concept" },
  { v: "comparison", label: "对比 comparison" },
  { v: "synthesis", label: "综合 synthesis" },
  { v: "methodology", label: "方法论 methodology" },
  { v: "finding", label: "结论 finding" },
  { v: "query", label: "查询 query" },
  { v: "thesis", label: "命题 thesis" },
  { v: "unknown", label: "未分类" }
];

// ──────────────────────────────────────────────────────────────────────
// G4 · ChatWorkbench / KnowledgeInbox / ObservabilityDashboard / TestMatchPanel
// ──────────────────────────────────────────────────────────────────────


export function KnowledgeTreeView() {
  const [items, setItems] = useState<TreeChunkItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [expandL1, setExpandL1] = useState<Set<string>>(new Set());
  const [expandL2, setExpandL2] = useState<Set<string>>(new Set()); // key = `${l1}|${l2}`
  const [showBody, setShowBody] = useState(false);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/chunks");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: TreeChunkItem[] };
      setItems(data.items ?? []);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  const tree = useMemo(() => {
    // l1Key -> l2Key -> chunk[]
    const t = new Map<string, Map<string, TreeChunkItem[]>>();
    for (const it of items) {
      const l1 = it.wikiType ?? "unknown";
      const l2 = (it.businessTopics && it.businessTopics[0]) || "通用";
      if (!t.has(l1)) t.set(l1, new Map());
      const lvl2 = t.get(l1)!;
      if (!lvl2.has(l2)) lvl2.set(l2, []);
      lvl2.get(l2)!.push(it);
    }
    return t;
  }, [items]);

  const indexById = useMemo(() => {
    const m = new Map<string, TreeChunkItem>();
    for (const it of items) m.set(it.id, it);
    return m;
  }, [items]);

  const active = activeId ? indexById.get(activeId) ?? null : null;

  function toggleL1(k: string) {
    setExpandL1((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }
  function toggleL2(k: string) {
    setExpandL2((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  function selectChunk(id: string) {
    setActiveId(id);
    setShowBody(false);
    setInfo(null);
    if (typeof window !== "undefined") {
      window.dispatchEvent(new CustomEvent("wikiFocusChunk", { detail: { chunkId: id } }));
    }
    // 自动展开它所在路径
    const it = indexById.get(id);
    if (it) {
      const l1 = it.wikiType ?? "unknown";
      const l2 = (it.businessTopics && it.businessTopics[0]) || "通用";
      setExpandL1((prev) => new Set(prev).add(l1));
      setExpandL2((prev) => new Set(prev).add(`${l1}|${l2}`));
    }
  }

  async function copyAnchor(anchor: Record<string, unknown>) {
    try {
      await navigator.clipboard.writeText(JSON.stringify(anchor, null, 2));
      setInfo("已复制 anchor JSON 到剪贴板");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="wikiPanelBody wikiTreeBody">
      <div className="wikiToolbar">
        <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} />
          {loading ? "加载中…" : "刷新"}
        </button>
        <span className="wikiHint">
          只读视图。verify / reject 请去"待评审"，编辑请去"编辑历史"。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      <div className="wikiLintLayout wikiTreeLayout">
        <div className="wikiTreePane">
          {WIKI_TYPES_ORDER.map((t) => {
            const lvl2 = tree.get(t.v);
            const total = lvl2
              ? Array.from(lvl2.values()).reduce((acc, arr) => acc + arr.length, 0)
              : 0;
            const expanded = expandL1.has(t.v);
            return (
              <div key={t.v} className="wikiTreeBlock">
                <button
                  type="button"
                  className={`wikiTreeNode l1 ${total === 0 ? "empty" : ""}`}
                  onClick={() => toggleL1(t.v)}
                >
                  {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                  <span className={`wikiKind ${t.v}`}>{t.v}</span>
                  <span className="wikiTreeLabel">{t.label}</span>
                  <span className="wikiLintCount">{total}</span>
                </button>
                {expanded && lvl2 ? (
                  <div className="wikiTreeChildren">
                    {Array.from(lvl2.entries())
                      .sort((a, b) => a[0].localeCompare(b[0]))
                      .map(([topic, chunks]) => {
                        const k = `${t.v}|${topic}`;
                        const open2 = expandL2.has(k);
                        return (
                          <div key={k} className="wikiTreeBlock">
                            <button
                              type="button"
                              className="wikiTreeNode l2"
                              onClick={() => toggleL2(k)}
                            >
                              {open2 ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                              <span className="wikiTreeLabel">{topic}</span>
                              <span className="wikiLintCount">{chunks.length}</span>
                            </button>
                            {open2 ? (
                              <div className="wikiTreeChildren">
                                {chunks
                                  .slice()
                                  .sort((a, b) => a.title.localeCompare(b.title))
                                  .map((c) => (
                                    <button
                                      type="button"
                                      key={c.id}
                                      className={`wikiTreeNode l3 ${
                                        activeId === c.id ? "active" : ""
                                      }`}
                                      onClick={() => selectChunk(c.id)}
                                      title={c.title}
                                    >
                                      <span className="wikiTreeLabel">{c.title}</span>
                                    </button>
                                  ))}
                              </div>
                            ) : null}
                          </div>
                        );
                      })}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
        <div className="wikiTreeDetail">
          {!active ? (
            <EmptyState title="未选择知识条目" hint="从左侧知识树中选择一条，查看详情。" />
          ) : (
            <ChunkDetail
              chunk={active}
              showBody={showBody}
              onToggleBody={() => setShowBody((v) => !v)}
              onJump={selectChunk}
              onCopyAnchor={(a) => void copyAnchor(a)}
              indexById={indexById}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function ChunkDetail(props: {
  chunk: TreeChunkItem;
  showBody: boolean;
  onToggleBody: () => void;
  onJump: (id: string) => void;
  onCopyAnchor: (anchor: Record<string, unknown>) => void;
  indexById: Map<string, TreeChunkItem>;
}) {
  const { chunk, showBody, onToggleBody, onJump, onCopyAnchor, indexById } = props;
  const hasQuote = !!chunk.sourceQuote && chunk.sourceQuote.trim().length > 0;
  const anchors = (chunk.sourceAnchors as Record<string, unknown>[] | null) ?? [];
  const related = chunk.relatedChunks ?? [];

  return (
    <article className="wikiChunkDetail">
      <header className="wikiChunkDetailHead">
        <div className="wikiChunkDetailTitle">
          <span className={`wikiKind ${chunk.wikiType ?? "unknown"}`}>{wikiTypeLabel(chunk.wikiType ?? undefined)}</span>
          <h3>{chunk.title}</h3>
        </div>
        <div className="wikiChunkDetailMeta">
          <span className={`wikiSev ${chunk.integrityStatus === "rejected" ? "error" : "info"}`}>
            {integrityStatusLabel(chunk.integrityStatus ?? undefined)}
          </span>
          <span className="wikiBadge">{statusLabel(chunk.status ?? undefined)}</span>
          <code>{chunk.id}</code>
        </div>
      </header>
      {chunk.summary ? <p className="wikiChunkSummary">{chunk.summary}</p> : null}
      {hasQuote ? (
        <blockquote className="wikiArchiveCitation">
          {chunk.sourceQuote}
          <span className="wikiArchiveCitationSource">{chunk.id}</span>
        </blockquote>
      ) : (
        <div className="wikiHint">无 source_quote — 该 chunk 不可被 verify。</div>
      )}
      {anchors.length > 0 ? (
        <section className="wikiSourceAnchorsSection">
          <div className="wikiSectionTitle">source_anchors（{anchors.length}）</div>
          <div className="wikiSourceAnchorList">
            {anchors.map((a, i) => {
              const sl = numberOr(a["startLine"]);
              const el = numberOr(a["endLine"]);
              const so = numberOr(a["startOffset"]);
              const eo = numberOr(a["endOffset"]);
              const hash = stringOr(a["quoteHash"]);
              const docId = stringOr(a["documentId"]);
              return (
                <button
                  type="button"
                  key={`${chunk.id}-anchor-${i}`}
                  className="wikiSourceAnchor"
                  onClick={() => onCopyAnchor(a)}
                  title={`复制 anchor JSON\nhash=${hash}\noffset=${so}-${eo}${
                    docId ? `\ndoc=${docId}` : ""
                  }`}
                >
                  <span className="wikiSourceAnchorRange">L{sl}-L{el}</span>
                  {hash ? (
                    <code className="wikiSourceAnchorHash">{hash.slice(0, 12)}…</code>
                  ) : null}
                  {docId ? <span className="wikiBadge">doc</span> : null}
                </button>
              );
            })}
          </div>
        </section>
      ) : null}
      {related.length > 0 ? (
        <section>
          <div className="wikiSectionTitle">related_chunks（{related.length}）</div>
          <div className="wikiRelatedList">
            {related.map((r, i) => {
              const target = indexById.get(r.chunk_id);
              const dead = !target;
              return (
                <button
                  type="button"
                  key={`${chunk.id}-rel-${i}`}
                  className={`wikiRelatedChip ${dead ? "dead" : ""}`}
                  disabled={dead}
                  onClick={() => onJump(r.chunk_id)}
                  title={dead ? "目标 chunk 不在活跃集合（已 archived 或不存在）" : r.note ?? ""}
                >
                  <span className="wikiRelatedKind">{r.kind}</span>
                  <span className="wikiRelatedTitle">{target ? target.title : r.chunk_id}</span>
                </button>
              );
            })}
          </div>
        </section>
      ) : null}
      <section>
        <button type="button" className="wikiCitedHead" onClick={onToggleBody}>
          {showBody ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          {showBody ? "收起正文" : "展开正文"}
        </button>
        {showBody && chunk.body ? <pre className="wikiReviewBodyText">{chunk.body}</pre> : null}
      </section>
    </article>
  );
}

