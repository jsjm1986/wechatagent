import { Fragment, useState, useEffect, useMemo, useRef, useCallback, type FormEvent } from "react";
import {
  Activity,
  AlertTriangle,
  Archive,
  ArrowRight,
  BookOpen,
  BrainCircuit,
  Calendar,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock3,
  Compass,
  FileBox,
  FileText,
  Inbox,
  Loader2,
  Map as MapIcon,
  MessageSquareText,
  Network,
  Plus,
  RefreshCw,
  Rss,
  Search,
  SendHorizonal,
  ShieldCheck,
  Sparkles,
  Trash2,
  Undo2,
  UploadCloud,
  Workflow,
  Wrench,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { parseApiError } from "../../lib/api";
import { parseCompleteness, parseIntegrityReport, type CompletenessView, type IntegrityReportView, type TrustChunkFields } from "./trustTypes";
import {
  LlmErrorBanner,
  ChunkInspectorPane,
  ChunkRevisionsTimeline,
  classifyChunk,
  focusChunk,
  numberOr,
  stringOr,
  type ReviewCategory,
  type ReviewChunkItem,
  type TreeChunkItem,
} from "./shared";
import { ChunkGraphView, DomainSchemaTab, MetricsTab, MemoryDrawer, AdminGovernanceView } from "./atlas";
import { CockpitView } from "./cockpit/CockpitView";
import { ReviewChat, type ReviewChatChunk } from "./cockpit/ReviewChat";
import { AutoVerifyPanel } from "./cockpit/AutoVerifyPanel";
import "./Knowledge.module.css";

// knowledge-wiki Phase G+：Wiki 管理频道——agent-first 渐进式披露主入口。
//
// - AskView：调 /api/knowledge/ask，agent 自驱 list_catalog → open_chunk →
//   follow_relations → answer，渲染答案 + cited 卡片 + tool_trace 时间线
// - LintView：8 类 kind 计数树 + signal 列表（替代旧 GapSignalsTab）
// - ReviewView：5 类待评审处置（needs_review / contested / source_orphan /
//   pending_verification / dependents_pending）
// - TreeView：3 级树（wiki_type → business_topic → chunk title），右侧
//   ChunkDetail 透出 source_quote 黄边块 + source_anchors 锚点 + related_chunks 跳转
// - DomainSchemaTab：列 active / 历史版本，一键切换 active
// - ChunkRevisionsDrawer：输入 chunk_id 拉历史 timeline
type KnowledgeMode = "today" | "explore" | "steward" | "atlas";

interface ModeMeta {
  key: KnowledgeMode;
  label: string;
  caption: string;
  Icon: LucideIcon;
}

const KNOWLEDGE_MODES: ModeMeta[] = [
  { key: "today", label: "今日", caption: "Digest 与待办", Icon: Calendar },
  { key: "explore", label: "探索", caption: "知识问答与浏览", Icon: Compass },
  { key: "steward", label: "治理", caption: "信号、待评审、修订", Icon: Wrench },
  { key: "atlas", label: "全景", caption: "Schema、指标、记忆", Icon: MapIcon }
];

export function KnowledgeWikiView() {
  const [mode, setMode] = useState<KnowledgeMode>("today");
  return (
    <section className="qualityCenter knowledgeWiki knowledgeWorkstation">
      <header className="wikiArchiveHeader" style={{ padding: "16px 20px 12px", marginBottom: 0 }}>
        <span className="wikiArchiveSubtitle">Knowledge Workstation · 知识档案馆</span>
        <h2 style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 22 }}>
          <FileBox size={20} /> 知识库工作站
        </h2>
      </header>
      <div className="wikiModeBar">
        {KNOWLEDGE_MODES.map((m) => {
          const ModeIcon = m.Icon;
          const active = mode === m.key;
          return (
            <button
              key={m.key}
              className={active ? "wikiModeBarBtn active" : "wikiModeBarBtn"}
              onClick={() => setMode(m.key)}
              type="button"
            >
              <ModeIcon size={16} />
              <span className="wikiModeBarLabel">{m.label}</span>
              <span className="wikiModeBarCaption">{m.caption}</span>
            </button>
          );
        })}
      </div>
      <div className="wikiModeStage">
        {mode === "today" && <TodayMode />}
        {mode === "explore" && <ExploreMode />}
        {mode === "steward" && <StewardMode />}
        {mode === "atlas" && <AtlasMode />}
      </div>
    </section>
  );
}

function TodayMode() {
  const [pane, setPane] = useState<"digest" | "chat" | "inbox">("digest");
  return (
    <div className="wikiModeGrid wikiModeGrid--today">
      <div className="wikiModePane wikiModePane--nav wikiStewardNav">
        <button
          type="button"
          className={pane === "digest" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("digest")}
        >
          <Sparkles size={14} /> 今日 Digest
        </button>
        <button
          type="button"
          className={pane === "chat" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("chat")}
        >
          <MessageSquareText size={14} /> AI 协作
        </button>
        <button
          type="button"
          className={pane === "inbox" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("inbox")}
        >
          <Inbox size={14} /> 待办收件箱
        </button>
      </div>
      <div className="wikiModePane wikiModePane--main">
        {pane === "digest" && <DigestCanvas />}
        {pane === "chat" && <ChatWorkbench />}
        {pane === "inbox" && <KnowledgeInbox />}
      </div>
      <div className="wikiModePane wikiModePane--side">
        <TaskRail />
      </div>
    </div>
  );
}

function ExploreMode() {
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    function onFocus(e: Event) {
      const ce = e as CustomEvent<{ chunkId?: string }>;
      const id = ce.detail?.chunkId;
      if (typeof id === "string" && id) {
        setFocusedId(id);
        setCollapsed(false);
      }
    }
    window.addEventListener("wikiFocusChunk", onFocus as EventListener);
    return () => window.removeEventListener("wikiFocusChunk", onFocus as EventListener);
  }, []);

  return (
    <div className={`wikiModeGrid wikiModeGrid--explore${collapsed ? " is-collapsed" : ""}`}>
      <div className="wikiModePane wikiModePane--nav">
        <KnowledgeTreeView />
      </div>
      <div className="wikiModePane wikiModePane--main">
        <AskView />
      </div>
      {!collapsed ? (
        <ChunkInspectorPane
          chunkId={focusedId}
          onClose={() => setCollapsed(true)}
          onClear={() => setFocusedId(null)}
        />
      ) : (
        <button
          type="button"
          className="wikiInspectorClose"
          onClick={() => setCollapsed(false)}
          title="展开 Inspector"
          style={{ position: "absolute", right: 8, top: 64 }}
        >
          <ChevronRight size={16} />
        </button>
      )}
    </div>
  );
}

function StewardMode() {
  const [pane, setPane] = useState<"cockpit" | "lint" | "review" | "autoVerify" | "revisions" | "documents" | "import" | "ingest" | "observability" | "tryRecall">("cockpit");
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(true);

  useEffect(() => {
    function onFocus(e: Event) {
      const ce = e as CustomEvent<{ chunkId?: string }>;
      const id = ce.detail?.chunkId;
      if (typeof id === "string" && id) {
        setFocusedId(id);
        setCollapsed(false);
      }
    }
    function onOpenCockpit() {
      setPane("cockpit");
    }
    window.addEventListener("wikiFocusChunk", onFocus as EventListener);
    window.addEventListener("wikiOpenCockpit", onOpenCockpit);
    return () => {
      window.removeEventListener("wikiFocusChunk", onFocus as EventListener);
      window.removeEventListener("wikiOpenCockpit", onOpenCockpit);
    };
  }, []);

  return (
    <div
      className={`wikiModeGrid wikiModeGrid--steward${
        !collapsed ? " has-inspector" : ""
      }`}
    >
      <div className="wikiModePane wikiModePane--nav wikiStewardNav">
        <button
          type="button"
          className={pane === "cockpit" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("cockpit")}
        >
          <ShieldCheck size={14} /> 治理总览
        </button>
        <button
          type="button"
          className={pane === "lint" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("lint")}
        >
          <AlertTriangle size={14} /> 质量信号
        </button>
        <button
          type="button"
          className={pane === "review" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("review")}
        >
          <ShieldCheck size={14} /> 待评审
        </button>
        <button
          type="button"
          className={pane === "autoVerify" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("autoVerify")}
        >
          <Sparkles size={14} /> 批量校验
        </button>
        <button
          type="button"
          className={pane === "revisions" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("revisions")}
        >
          <Clock3 size={14} /> 修订历史
        </button>
        <button
          type="button"
          className={pane === "documents" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("documents")}
        >
          <FileText size={14} /> 文档目录
        </button>
        <button
          type="button"
          className={pane === "import" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("import")}
        >
          <UploadCloud size={14} /> 导入向导
        </button>
        <button
          type="button"
          className={pane === "ingest" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("ingest")}
        >
          <Rss size={14} /> 外部源
        </button>
        <button
          type="button"
          className={pane === "observability" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("observability")}
        >
          <Activity size={14} /> 诊断仪表
        </button>
        <button
          type="button"
          className={pane === "tryRecall" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("tryRecall")}
        >
          <Search size={14} /> 试召诊断
        </button>
      </div>
      <div className="wikiModePane wikiModePane--main">
        {pane === "cockpit" && (
          <CockpitView
            onOpenReview={() => setPane("review")}
            onOpenAutoVerify={() => setPane("autoVerify")}
          />
        )}
        {pane === "lint" && <LintView />}
        {pane === "review" && <ReviewView />}
        {pane === "autoVerify" && <AutoVerifyPanel />}
        {pane === "revisions" && <ChunkRevisionsDrawer />}
        {pane === "documents" && <DocumentsView />}
        {pane === "import" && <ImportWizard />}
        {pane === "ingest" && <IngestSourcesView />}
        {pane === "observability" && <ObservabilityDashboard />}
        {pane === "tryRecall" && <TryRecallView />}
      </div>
      {!collapsed ? (
        <ChunkInspectorPane
          chunkId={focusedId}
          onClose={() => setCollapsed(true)}
          onClear={() => setFocusedId(null)}
        />
      ) : null}
    </div>
  );
}

function AtlasMode() {
  const [pane, setPane] = useState<"schema" | "metrics" | "memory" | "graph" | "governance">("schema");
  return (
    <div className="wikiModeGrid wikiModeGrid--atlas">
      <div className="wikiModePane wikiModePane--nav wikiStewardNav">
        <button
          type="button"
          className={pane === "schema" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("schema")}
        >
          <BookOpen size={14} /> 行业 Schema
        </button>
        <button
          type="button"
          className={pane === "metrics" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("metrics")}
        >
          <Activity size={14} /> 指标总览
        </button>
        <button
          type="button"
          className={pane === "memory" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("memory")}
        >
          <BrainCircuit size={14} /> 运营记忆
        </button>
        <button
          type="button"
          className={pane === "graph" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("graph")}
        >
          <Network size={14} /> 关系图谱
        </button>
        <button
          type="button"
          className={pane === "governance" ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
          onClick={() => setPane("governance")}
        >
          <ShieldCheck size={14} /> 治理
        </button>
      </div>
      <div className="wikiModePane wikiModePane--main">
        {pane === "schema" && <DomainSchemaTab />}
        {pane === "metrics" && <MetricsTab />}
        {pane === "memory" && <MemoryDrawer />}
        {pane === "graph" && <ChunkGraphView />}
        {pane === "governance" && <AdminGovernanceView />}
      </div>
    </div>
  );
}


// ── G2 · DocumentsView · 知识文档目录 CRUD ─────────────────────────────
interface DocumentItem {
  id: string;
  title: string;
  summary?: string | null;
  domain?: string | null;
  sourceType?: string | null;
  sourceName?: string | null;
  status?: string | null;
  catalogSummary?: string | null;
  updatedAt?: string | null;
  routingMap?: string[] | null;
  productTags?: string[] | null;
  businessTopics?: string[] | null;
}

interface DocumentChunkRow {
  id: string;
  title?: string | null;
  wikiType?: string | null;
  status?: string | null;
  integrityStatus?: string | null;
  summary?: string | null;
  updatedAt?: string | null;
}

function DocumentsView() {
  const [items, setItems] = useState<DocumentItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState({ title: "", summary: "", sourceName: "", sourceType: "imported_markdown" });
  // 行内展开：documentId → 子表状态。后端 GET /documents/:id/chunks
  // 已存在但前端未挂；这里按需 lazy-load，节省默认列表渲染开销。
  const [expandedDoc, setExpandedDoc] = useState<string | null>(null);
  const [docChunks, setDocChunks] = useState<Record<string, DocumentChunkRow[]>>({});
  const [docChunksLoading, setDocChunksLoading] = useState<string | null>(null);
  const [docChunksError, setDocChunksError] = useState<string | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/documents");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items?: DocumentItem[] };
      setItems(data.items ?? []);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void load(); }, []);

  async function handleCreate(ev: FormEvent) {
    ev.preventDefault();
    if (!draft.title.trim()) return;
    setCreating(true);
    try {
      const r = await fetch("/api/operation-knowledge/documents", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          domain: "user_operations",
          title: draft.title.trim(),
          summary: draft.summary.trim() || null,
          sourceName: draft.sourceName.trim() || null,
          sourceType: draft.sourceType,
          status: "draft"
        })
      });
      if (!r.ok) throw await parseApiError(r);
      setDraft({ title: "", summary: "", sourceName: "", sourceType: "imported_markdown" });
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  async function handleDelete(id: string) {
    if (!window.confirm(`删除文档？关联 chunks 不会被删除，但失去文档归属。`)) return;
    try {
      const r = await fetch(`/api/operation-knowledge/documents/${encodeURIComponent(id)}`, { method: "DELETE" });
      if (!r.ok) throw await parseApiError(r);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleDocChunks(docId: string) {
    if (expandedDoc === docId) {
      setExpandedDoc(null);
      return;
    }
    setExpandedDoc(docId);
    if (docChunks[docId]) return; // 已缓存
    setDocChunksLoading(docId);
    setDocChunksError(null);
    try {
      const r = await fetch(`/api/operation-knowledge/documents/${encodeURIComponent(docId)}/chunks`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items?: DocumentChunkRow[] };
      setDocChunks({ ...docChunks, [docId]: data.items ?? [] });
    } catch (e) {
      setDocChunksError(String(e));
    } finally {
      setDocChunksLoading(null);
    }
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Documents · 文档目录</span>
        <h3 style={{ fontSize: 20 }}>知识文档</h3>
      </header>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      <form onSubmit={handleCreate} style={{ display: "grid", gridTemplateColumns: "1fr 1fr 200px auto", gap: 8, marginBottom: 16 }}>
        <input
          type="text"
          placeholder="文档标题（必填）"
          value={draft.title}
          onChange={(e) => setDraft({ ...draft, title: e.target.value })}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="摘要"
          value={draft.summary}
          onChange={(e) => setDraft({ ...draft, summary: e.target.value })}
          className="wikiInput"
        />
        <select
          value={draft.sourceType}
          onChange={(e) => setDraft({ ...draft, sourceType: e.target.value })}
          className="wikiInput"
        >
          <option value="imported_markdown">imported_markdown</option>
          <option value="manual">manual</option>
          <option value="external_url">external_url</option>
          <option value="archived">archived</option>
        </select>
        <button type="submit" className="wikiBtn" disabled={creating || !draft.title.trim()}>
          {creating ? "保存中…" : "新建"}
        </button>
      </form>
      {loading ? <div className="wikiHint">加载中…</div> : items.length === 0 ? (
        <div className="wikiHint">还没有文档。新建第一份，或使用导入向导。</div>
      ) : (
        <table className="wikiTable" style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>标题</th>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>来源</th>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>状态</th>
              <th style={{ textAlign: "left", padding: "8px 6px", borderBottom: "1px solid var(--line)" }}>更新</th>
              <th style={{ padding: "8px 6px", borderBottom: "1px solid var(--line)" }}></th>
            </tr>
          </thead>
          <tbody>
            {items.map((d) => (
              <Fragment key={d.id}>
              <tr style={{ borderBottom: "1px solid var(--line)" }}>
                <td style={{ padding: "10px 6px" }}>
                  <div style={{ fontFamily: "var(--font-display)", fontWeight: 600 }}>{d.title}</div>
                  {d.summary ? <div style={{ color: "var(--muted)", fontSize: 12, marginTop: 2 }}>{d.summary}</div> : null}
                  <div style={{ marginTop: 4 }}>
                    {(d.businessTopics ?? []).map((t, i) => <span key={i} className="wikiArchiveTag">{t}</span>)}
                  </div>
                </td>
                <td style={{ padding: "10px 6px", fontFamily: "var(--font-mono)", fontSize: 12 }}>
                  <div>{d.sourceType ?? "—"}</div>
                  <div style={{ color: "var(--muted)" }}>{d.sourceName ?? ""}</div>
                </td>
                <td style={{ padding: "10px 6px" }}>
                  <span className="wikiBadge">{d.status ?? "—"}</span>
                </td>
                <td style={{ padding: "10px 6px", fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)" }}>
                  {d.updatedAt ? new Date(d.updatedAt).toLocaleString() : "—"}
                </td>
                <td style={{ padding: "10px 6px", textAlign: "right", whiteSpace: "nowrap" }}>
                  <button
                    type="button"
                    className="wikiArchiveRollback"
                    style={{ marginRight: 6 }}
                    onClick={() => void toggleDocChunks(d.id)}
                  >
                    {expandedDoc === d.id ? "收起 chunks" : "查看 chunks"}
                  </button>
                  <button type="button" className="wikiArchiveRollback" onClick={() => handleDelete(d.id)}>删除</button>
                </td>
              </tr>
              {expandedDoc === d.id ? (
                <tr>
                  <td colSpan={5} style={{ background: "var(--surface-2, #f4efe5)", padding: "10px 14px" }}>
                    {docChunksLoading === d.id ? (
                      <div className="wikiHint">正在拉 chunks…</div>
                    ) : docChunksError ? (
                      <div className="wikiAlert error">{docChunksError}</div>
                    ) : (docChunks[d.id]?.length ?? 0) === 0 ? (
                      <div className="wikiHint">该文档下还没有 chunks。可走导入向导或手工新建。</div>
                    ) : (
                      <div style={{ display: "grid", gap: 4 }}>
                        <div style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)", marginBottom: 4 }}>
                          {docChunks[d.id].length} chunks · 点击编号跳到 ChunkInspectorPane
                        </div>
                        {docChunks[d.id].map((c) => (
                          <button
                            key={c.id}
                            type="button"
                            className="wikiSignalChunkBtn"
                            onClick={() => focusChunk(c.id)}
                            style={{ textAlign: "left", display: "grid", gridTemplateColumns: "120px 100px 1fr auto", gap: 8, alignItems: "center" }}
                          >
                            <span className="wikiArchiveTag">{c.wikiType ?? "—"}</span>
                            <span style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>
                              {c.status ?? "—"} / {c.integrityStatus ?? "—"}
                            </span>
                            <span style={{ fontFamily: "var(--font-display)", fontWeight: 600 }}>
                              {c.title ?? "（无标题）"}
                            </span>
                            <code style={{ fontSize: 10, color: "var(--muted)" }}>{c.id.slice(-6)}</code>
                          </button>
                        ))}
                      </div>
                    )}
                  </td>
                </tr>
              ) : null}
              </Fragment>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

// ── G2 · ImportWizard · 三步条粘贴 → 预览 → 应用 ───────────────────────
interface ImportPreviewChunk {
  title?: string | null;
  body?: string | null;
  summary?: string | null;
  wikiType?: string | null;
  businessTopics?: string[] | null;
  productTags?: string[] | null;
  routingCard?: string | null;
}

interface ImportPreviewResult {
  document?: { title?: string; summary?: string; catalogSummary?: string } | null;
  items?: unknown[];
  chunks?: ImportPreviewChunk[];
}

function ImportWizard() {
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [content, setContent] = useState("");
  const [sourceName, setSourceName] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreviewResult | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [edits, setEdits] = useState<Record<number, Partial<ImportPreviewChunk>>>({});
  const [created, setCreated] = useState<string[]>([]);
  // G-后续/4：AI 重抽 tags 按钮单条 loading 标记。
  const [retagging, setRetagging] = useState<number | null>(null);

  async function retagCandidate(i: number) {
    const merged = { ...(preview?.chunks ?? [])[i], ...(edits[i] ?? {}) };
    if (!merged.body || !merged.body.trim()) {
      setError("候选无正文，无法重抽 tags");
      return;
    }
    setRetagging(i);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/extract-tags", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          title: merged.title ?? null,
          body: merged.body,
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { productTags?: string[]; businessTopics?: string[] };
      setEdits({
        ...edits,
        [i]: {
          ...(edits[i] ?? {}),
          productTags: data.productTags ?? [],
          businessTopics: data.businessTopics ?? [],
        },
      });
    } catch (e) {
      setError(`重抽 tags 失败：${e}`);
    } finally {
      setRetagging(null);
    }
  }

  async function runPreview() {
    if (!content.trim()) return;
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/import-preview", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ content, sourceName: sourceName.trim() || null })
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as ImportPreviewResult;
      setPreview(data);
      const all = new Set<number>();
      (data.chunks ?? []).forEach((_, i) => all.add(i));
      setSelected(all);
      setEdits({});
      setStep(2);
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  async function runApply() {
    if (!preview) return;
    setPending(true);
    setError(null);
    try {
      const finalChunks = (preview.chunks ?? [])
        .map((c, i) => ({ ...c, ...(edits[i] ?? {}) }))
        .filter((_, i) => selected.has(i));
      const payload = {
        document: preview.document,
        items: preview.items,
        chunks: finalChunks,
        sourceName: sourceName.trim() || null
      };
      const r = await fetch("/api/operation-knowledge/import-apply", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload)
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { createdChunkIds?: string[]; created_chunk_ids?: string[] };
      const ids = data.createdChunkIds ?? data.created_chunk_ids ?? [];
      setCreated(ids);
      setStep(3);
      if (ids[0]) {
        setTimeout(() => focusChunk(ids[0]), 100);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  function reset() {
    setStep(1); setContent(""); setSourceName(""); setPreview(null);
    setSelected(new Set()); setEdits({}); setCreated([]); setError(null);
  }

  function toggle(i: number) {
    const next = new Set(selected);
    if (next.has(i)) next.delete(i); else next.add(i);
    setSelected(next);
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Import Wizard · 文档导入</span>
        <h3 style={{ fontSize: 20 }}>导入向导</h3>
      </header>
      <div className="wikiImportStepper">
        {[
          { n: 1, label: "粘贴" },
          { n: 2, label: "预览" },
          { n: 3, label: "应用" }
        ].map((s) => (
          <div key={s.n} className={`wikiImportStep${step === s.n ? " active" : ""}${step > s.n ? " done" : ""}`}>
            <span className="wikiImportStepNum">{s.n}</span> {s.label}
          </div>
        ))}
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {step === 1 ? (
        <div style={{ marginTop: 12 }}>
          <input
            type="text"
            placeholder="来源名称（可选）：例如「运营手册 v3」"
            value={sourceName}
            onChange={(e) => setSourceName(e.target.value)}
            className="wikiInput"
            style={{ width: "100%", marginBottom: 8 }}
          />
          <textarea
            placeholder="粘贴 markdown / 长文本…"
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={14}
            className="wikiInput"
            style={{ width: "100%", fontFamily: "var(--font-mono)", fontSize: 12 }}
          />
          <div style={{ marginTop: 10, display: "flex", gap: 8 }}>
            <button type="button" className="wikiBtn" onClick={runPreview} disabled={pending || !content.trim()}>
              {pending ? "解析中…" : "下一步：预览"}
            </button>
            <span style={{ color: "var(--muted)", fontSize: 12, alignSelf: "center" }}>
              将由 AI 拆为候选 chunk，所有 chunk 默认 status=draft + integrity_status=needs_review。
            </span>
          </div>
          <div style={{ marginTop: 14, padding: "10px 12px", border: "1px dashed var(--border)", borderRadius: 6 }}>
            <div style={{ fontSize: 12, color: "var(--muted)", marginBottom: 6 }}>
              · multimodal 入口（绕过 AI preview，直接 fence 解析 / vision 抽取，落 status=draft）
            </div>
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
              <label className="wikiBtn" style={{ cursor: pending ? "not-allowed" : "pointer", margin: 0 }}>
                {pending ? "上传中…" : "上传 PDF"}
                <input
                  type="file"
                  accept="application/pdf,.pdf"
                  hidden
                  disabled={pending}
                  onChange={async (ev) => {
                    const f = ev.target.files?.[0];
                    if (!f) return;
                    setPending(true);
                    setError(null);
                    try {
                      const fd = new FormData();
                      fd.append("file", f);
                      fd.append("sourceName", sourceName.trim() || f.name);
                      const r = await fetch("/api/operation-knowledge/import-apply-pdf", {
                        method: "POST",
                        body: fd,
                      });
                      if (!r.ok) throw await parseApiError(r);
                      const data = (await r.json()) as { chunkIds?: string[]; fallbackBlob?: boolean };
                      setCreated(data.chunkIds ?? []);
                      setStep(3);
                      if ((data.chunkIds ?? [])[0]) {
                        setTimeout(() => focusChunk(data.chunkIds![0]), 100);
                      }
                    } catch (e) {
                      setError(`PDF 导入失败：${e}`);
                    } finally {
                      setPending(false);
                      ev.target.value = "";
                    }
                  }}
                />
              </label>
              <label className="wikiBtn" style={{ cursor: pending ? "not-allowed" : "pointer", margin: 0 }}>
                {pending ? "上传中…" : "上传图片（vision）"}
                <input
                  type="file"
                  accept="image/*"
                  hidden
                  disabled={pending}
                  onChange={async (ev) => {
                    const f = ev.target.files?.[0];
                    if (!f) return;
                    setPending(true);
                    setError(null);
                    try {
                      const buf = await f.arrayBuffer();
                      let bin = "";
                      const u8 = new Uint8Array(buf);
                      for (let i = 0; i < u8.byteLength; i++) bin += String.fromCharCode(u8[i]);
                      const b64 = btoa(bin);
                      const r = await fetch("/api/operation-knowledge/import-apply-image", {
                        method: "POST",
                        headers: { "content-type": "application/json" },
                        body: JSON.stringify({
                          imageBase64: b64,
                          mime: f.type || "image/png",
                          sourceName: sourceName.trim() || f.name,
                        }),
                      });
                      if (!r.ok) throw await parseApiError(r);
                      const data = (await r.json()) as { chunkIds?: string[]; note?: string };
                      setCreated(data.chunkIds ?? []);
                      setStep(3);
                      if (data.note) setError(`vision 提示：${data.note}`);
                      if ((data.chunkIds ?? [])[0]) {
                        setTimeout(() => focusChunk(data.chunkIds![0]), 100);
                      }
                    } catch (e) {
                      setError(`图片导入失败：${e}（需文字主模型支持图片，或在模型设置里指派一个「视觉模型」）`);
                    } finally {
                      setPending(false);
                      ev.target.value = "";
                    }
                  }}
                />
              </label>
            </div>
          </div>
        </div>
      ) : null}
      {step === 2 && preview ? (
        <div style={{ marginTop: 12 }}>
          {preview.document ? (
            <dl className="wikiArchiveMeta">
              <dt>文档标题</dt><dd>{preview.document.title}</dd>
              {preview.document.summary ? (<><dt>摘要</dt><dd>{preview.document.summary}</dd></>) : null}
              {preview.document.catalogSummary ? (<><dt>目录摘要</dt><dd>{preview.document.catalogSummary}</dd></>) : null}
            </dl>
          ) : null}
          <hr className="wikiArchiveRule" />
          <div style={{ marginBottom: 8, fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--muted)" }}>
            候选 chunks · {selected.size}/{(preview.chunks ?? []).length} 已选
          </div>
          <div style={{ display: "grid", gap: 10 }}>
            {(preview.chunks ?? []).map((c, i) => {
              const e = edits[i] ?? {};
              const merged = { ...c, ...e };
              const isOn = selected.has(i);
              return (
                <div key={i} className={`wikiImportCandidate${isOn ? " selected" : ""}`}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <input type="checkbox" checked={isOn} onChange={() => toggle(i)} />
                    <span className="wikiArchiveTag">{merged.wikiType ?? "—"}</span>
                    <input
                      type="text"
                      value={merged.title ?? ""}
                      onChange={(ev) => setEdits({ ...edits, [i]: { ...e, title: ev.target.value } })}
                      className="wikiInput"
                      style={{ flex: 1, fontFamily: "var(--font-display)", fontWeight: 600 }}
                    />
                  </div>
                  {merged.summary ? <p style={{ color: "var(--muted)", fontSize: 12.5, margin: "6px 0 4px" }}>{merged.summary}</p> : null}
                  <div className="wikiImportChips">
                    {(merged.businessTopics ?? []).map((t, j) => <span key={`bt-${j}`} className="wikiArchiveTag">{t}</span>)}
                    {(merged.productTags ?? []).map((t, j) => <span key={`pt-${j}`} className="wikiArchiveTag" style={{ borderStyle: "dashed" }}>{t}</span>)}
                    <button
                      type="button"
                      className="wikiArchiveRollback"
                      style={{ marginLeft: "auto", fontSize: 11, padding: "2px 8px" }}
                      onClick={() => void retagCandidate(i)}
                      disabled={retagging === i || !merged.body}
                      title="调 /extract-tags 重抽 productTags / businessTopics"
                    >
                      {retagging === i ? "AI 抽取中…" : "AI 重抽 tags"}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
          <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
            <button type="button" className="wikiBtn" onClick={() => setStep(1)}>← 返回粘贴</button>
            <button type="button" className="wikiBtn primary" onClick={runApply} disabled={pending || selected.size === 0}>
              {pending ? "应用中…" : `应用 ${selected.size} 条 →`}
            </button>
          </div>
        </div>
      ) : null}
      {step === 3 ? (
        <div style={{ marginTop: 12 }}>
          <div style={{ fontFamily: "var(--font-display)", fontSize: 16, marginBottom: 8 }}>
            ✓ 已存入 {created.length} 条草稿
          </div>
          <p style={{ color: "var(--muted)", fontSize: 12.5 }}>
            这些都是草稿,AI 还<strong>不能</strong>拿去跟客户说。需要你逐条看过、确认没问题后,AI 才会用它们。
          </p>
          <div style={{ marginTop: 8, display: "grid", gap: 4 }}>
            {created.map((id) => (
              <button key={id} type="button" className="wikiSignalChunkBtn" onClick={() => focusChunk(id)}>
                <code>{id}</code>
              </button>
            ))}
          </div>
          <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
            <button
              type="button"
              className="wikiBtn"
              onClick={() => window.dispatchEvent(new CustomEvent("wikiOpenCockpit"))}
            >
              去治理总览逐条处理 →
            </button>
            <button type="button" className="wikiBtn" onClick={reset}>导入更多</button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

// ── G-后续/3 · TryRecallView · "按 catalog 试召" 诊断 ─────────────────────
// 调 POST /tools/search（输入 query → 命中 chunk_ids），再 POST /tools/open-slice
// 拉具体 chunk 详情。开发者诊断 grounding：看哪些 chunk 被检索器选上、为什么。
interface TryRecallTraceStep {
  step?: string | number | null;
  tool?: string | null;
  input?: unknown;
  output?: unknown;
  notes?: string | null;
}
interface TryRecallRouteResult {
  neededCategories?: string[] | null;
  selectedKnowledgeIds?: string[] | null;
  selectedDocumentIds?: string[] | null;
  selectedChunkIds?: string[] | null;
  selectedSliceReasons?: string[] | null;
  riskLevel?: string | null;
  requiresEvidence?: boolean | null;
  knowledgeCoverage?: string | null;
  missingKnowledge?: string[] | null;
  reason?: string | null;
  toolTrace?: TryRecallTraceStep[] | null;
  evidenceExcerpts?: string[] | null;
}
interface TryRecallSliceItem {
  id: string;
  title?: string | null;
  wikiType?: string | null;
  status?: string | null;
  integrityStatus?: string | null;
  summary?: string | null;
  body?: string | null;
}

function TryRecallView() {
  const [accountId, setAccountId] = useState("default");
  const [contactId, setContactId] = useState("");
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [route, setRoute] = useState<TryRecallRouteResult | null>(null);
  const [slices, setSlices] = useState<TryRecallSliceItem[]>([]);
  const [openingSlices, setOpeningSlices] = useState(false);

  async function runSearch() {
    if (!query.trim()) return;
    setPending(true);
    setError(null);
    setRoute(null);
    setSlices([]);
    try {
      const r = await fetch("/api/operation-knowledge/tools/search", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          accountId: accountId.trim() || "default",
          contactId: contactId.trim() || null,
          query: query.trim(),
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { item?: TryRecallRouteResult };
      const item = data.item ?? null;
      setRoute(item);
      const ids = item?.selectedChunkIds ?? [];
      if (ids.length > 0) {
        setOpeningSlices(true);
        try {
          const r2 = await fetch("/api/operation-knowledge/tools/open-slice", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ ids }),
          });
          if (!r2.ok) throw await parseApiError(r2);
          const d2 = (await r2.json()) as { items?: TryRecallSliceItem[] };
          setSlices(d2.items ?? []);
        } catch (e2) {
          setError(`检索结果已返回，但 open-slice 失败：${e2}`);
        } finally {
          setOpeningSlices(false);
        }
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Try Recall · 试召诊断</span>
        <h3 style={{ fontSize: 20 }}>按 catalog 试召</h3>
      </header>
      <p style={{ color: "var(--muted)", fontSize: 12.5, margin: "0 0 12px" }}>
        给定 accountId（可选 contactId）和一句话 query，调用知识路由器看哪些 chunk 被选上，
        以及 tool_trace 里的 catalog → list_chunks → open_slice 决策链。开发者调试 grounding 用。
      </p>
      <form
        onSubmit={(e) => { e.preventDefault(); void runSearch(); }}
        style={{ display: "grid", gridTemplateColumns: "200px 200px 1fr auto", gap: 8, marginBottom: 12 }}
      >
        <input
          type="text"
          placeholder="accountId（默认 default）"
          value={accountId}
          onChange={(e) => setAccountId(e.target.value)}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="contactId（可选）"
          value={contactId}
          onChange={(e) => setContactId(e.target.value)}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="query：例如「这个产品对接 SaaS 还是私有化」"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="wikiInput"
        />
        <button type="submit" className="wikiBtn primary" disabled={pending || !query.trim()}>
          {pending ? "试召中…" : "试召"}
        </button>
      </form>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {route ? (
        <>
          <dl className="wikiArchiveMeta">
            <dt>风险等级</dt><dd>{route.riskLevel || "—"}</dd>
            <dt>需证据</dt><dd>{route.requiresEvidence ? "是" : "否"}</dd>
            <dt>覆盖度</dt><dd>{route.knowledgeCoverage || "—"}</dd>
            {route.reason ? (<><dt>路由原因</dt><dd>{route.reason}</dd></>) : null}
            {(route.neededCategories ?? []).length > 0 ? (
              <>
                <dt>需要类别</dt>
                <dd>{(route.neededCategories ?? []).map((c, i) => <span key={i} className="wikiArchiveTag">{c}</span>)}</dd>
              </>
            ) : null}
            {(route.missingKnowledge ?? []).length > 0 ? (
              <>
                <dt>缺失知识</dt>
                <dd>{(route.missingKnowledge ?? []).map((c, i) => <span key={i} className="wikiArchiveTag">{c}</span>)}</dd>
              </>
            ) : null}
          </dl>
          <hr className="wikiArchiveRule" />
          <div style={{ marginBottom: 6, fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--muted)" }}>
            命中 chunks · {(route.selectedChunkIds ?? []).length} 条
            {openingSlices ? "（正在拉详情…）" : ""}
          </div>
          <div style={{ display: "grid", gap: 8, marginBottom: 12 }}>
            {slices.length > 0
              ? slices.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  className="wikiSignalChunkBtn"
                  onClick={() => focusChunk(s.id)}
                  style={{ textAlign: "left", display: "grid", gap: 4 }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <span className="wikiArchiveTag">{s.wikiType ?? "—"}</span>
                    <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--muted)" }}>
                      {s.status ?? "—"} / {s.integrityStatus ?? "—"}
                    </span>
                    <span style={{ fontFamily: "var(--font-display)", fontWeight: 600, marginLeft: 4 }}>
                      {s.title ?? "（无标题）"}
                    </span>
                  </div>
                  {s.summary ? <div style={{ color: "var(--muted)", fontSize: 12 }}>{s.summary}</div> : null}
                </button>
              ))
              : (route.selectedChunkIds ?? []).length === 0
                ? <div className="wikiHint">本次试召未选中任何 chunk。</div>
                : null}
          </div>
          {(route.selectedSliceReasons ?? []).length > 0 ? (
            <details>
              <summary className="wikiInspectorSectionTitle">slice 选择原因（{(route.selectedSliceReasons ?? []).length}）</summary>
              <ul style={{ margin: "6px 0 0 18px", fontSize: 12.5 }}>
                {(route.selectedSliceReasons ?? []).map((r, i) => <li key={i}>{r}</li>)}
              </ul>
            </details>
          ) : null}
          {(route.toolTrace ?? []).length > 0 ? (
            <details style={{ marginTop: 8 }}>
              <summary className="wikiInspectorSectionTitle">tool_trace（{(route.toolTrace ?? []).length}）</summary>
              <pre style={{ fontFamily: "var(--font-mono)", fontSize: 11, background: "var(--surface-2, #f4efe5)", padding: 10, border: "1px solid var(--line)", maxHeight: 320, overflow: "auto" }}>
                {JSON.stringify(route.toolTrace, null, 2)}
              </pre>
            </details>
          ) : null}
          {(route.evidenceExcerpts ?? []).length > 0 ? (
            <details style={{ marginTop: 8 }}>
              <summary className="wikiInspectorSectionTitle">evidence_excerpts（{(route.evidenceExcerpts ?? []).length}）</summary>
              <ul style={{ margin: "6px 0 0 18px", fontSize: 12.5 }}>
                {(route.evidenceExcerpts ?? []).map((e, i) => <li key={i}><code>{e}</code></li>)}
              </ul>
            </details>
          ) : null}
        </>
      ) : null}
    </div>
  );
}


interface GapSignalItem {
  signalId: string;
  kind: string;
  title: string;
  description: string;
  severity: string;
  source: string;
  status: string;
  affectedChunkIds: string[];
  searchQueries: string[];
  resolutionNote?: string | null;
  createdAt?: string | null;
  resolvedAt?: string | null;
}

// 8 类 gap_signal kind —— 与 src/knowledge_wiki/gap_signals.rs:11-19 对齐。
const GAP_SIGNAL_KINDS: { v: string; label: string }[] = [
  { v: "orphan", label: "孤立 chunk" },
  { v: "broken_link", label: "断链" },
  { v: "no_outlinks", label: "缺出链" },
  { v: "low_confidence", label: "低分被命中" },
  { v: "stale", label: "时效已过" },
  { v: "contradiction", label: "同题异说" },
  { v: "missing_chunk", label: "依赖已归档" },
  { v: "suggestion", label: "建议补完" },
];

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
function AskView() {
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
          placeholder="问知识库一个问题（agent 自驱阅读 catalog → open_chunk → 回答）"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          rows={3}
          disabled={pending}
        />
        <div className="wikiAskActions">
          <span className="wikiHint">最多 3 轮工具调用；超过预算自动收尾。</span>
          <label className="wikiAskWsField">
            workspace
            <input
              type="text"
              value={workspaceId}
              onChange={(e) => setWorkspaceId(e.target.value)}
              placeholder="default"
              disabled={pending}
            />
          </label>
          {supportsEventSource ? (
            <label className="wikiAskModeToggle">
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

function LintView() {
  const [items, setItems] = useState<GapSignalItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [sweeping, setSweeping] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [activeKind, setActiveKind] = useState<string>("");

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams({ status: "pending", limit: "300" });
      const r = await fetch(`/api/knowledge/gap-signals?${params.toString()}`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { signals: GapSignalItem[] };
      setItems(data.signals ?? []);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function sweep() {
    setSweeping(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/gap-signals/sweep", { method: "POST" });
      if (!r.ok) throw await parseApiError(r);
      const data = await r.json();
      setInfo(
        `lint 新增 ${data?.structuralLint?.newSignals ?? 0}，` +
          `stage1 自动消解 ${data?.sweep?.stage1AutoResolved ?? 0}`,
      );
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSweeping(false);
    }
  }

  async function resolve(signalId: string, action: "dismiss" | "apply") {
    setBusyId(signalId);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/knowledge/gap-signals/${encodeURIComponent(signalId)}/${action}`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
        },
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(action === "dismiss" ? "已忽略" : "已标记为已应用");
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const s of items) m.set(s.kind, (m.get(s.kind) ?? 0) + 1);
    return m;
  }, [items]);

  const visible = useMemo(() => {
    if (!activeKind) return items;
    return items.filter((s) => s.kind === activeKind);
  }, [items, activeKind]);

  return (
    <div className="wikiPanelBody wikiLintBody">
      <div className="wikiToolbar">
        <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} />
          {loading ? "加载中…" : "刷新"}
        </button>
        <button type="button" className="primary" onClick={() => void sweep()} disabled={sweeping}>
          {sweeping ? "扫描中…" : "立即扫描"}
        </button>
        <span className="wikiHint">
          仅展示 status=pending；扫描包含 structural lint + stage 1 规则消解。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      <div className="wikiLintLayout">
        <div className="wikiLintTree">
          <button
            type="button"
            className={`wikiLintTreeNode ${activeKind === "" ? "active" : ""}`}
            onClick={() => setActiveKind("")}
          >
            <span>全部</span>
            <span className="wikiLintCount">{items.length}</span>
          </button>
          {GAP_SIGNAL_KINDS.map((k) => {
            const c = counts.get(k.v) ?? 0;
            return (
              <button
                type="button"
                key={k.v}
                className={`wikiLintTreeNode ${activeKind === k.v ? "active" : ""} ${
                  c === 0 ? "empty" : ""
                }`}
                onClick={() => setActiveKind(k.v)}
              >
                <span>
                  <span className={`wikiKind ${k.v}`}>{k.v}</span> {k.label}
                </span>
                <span className="wikiLintCount">{c}</span>
              </button>
            );
          })}
        </div>
        <div className="wikiLintPanel">
          {!loading && visible.length === 0 ? (
            <div className="wikiEmpty">
              {activeKind ? `当前 kind 没有 pending 信号。` : "没有 pending 信号。库结构当前看起来很健康。"}
            </div>
          ) : null}
          <div className="wikiSignalList">
            {visible.map((s) => (
              <div className={`wikiSignalCard sev-${s.severity}`} key={s.signalId}>
                <div className="wikiSignalHead">
                  <div className="wikiSignalTitle">
                    <span className={`wikiKind ${s.kind}`}>{s.kind}</span>
                    <span className={`wikiSev ${s.severity}`}>{s.severity}</span>
                    <strong>{s.title}</strong>
                  </div>
                  <div className="wikiSignalActions">
                    <button
                      type="button"
                      className="ghost"
                      onClick={() => void resolve(s.signalId, "dismiss")}
                      disabled={busyId === s.signalId}
                    >
                      忽略
                    </button>
                    <button
                      type="button"
                      className="primary"
                      onClick={() => void resolve(s.signalId, "apply")}
                      disabled={busyId === s.signalId}
                    >
                      标记已处理
                    </button>
                  </div>
                </div>
                <p className="wikiSignalDesc">{s.description}</p>
                {s.affectedChunkIds.length > 0 ? (
                  <div className="wikiSignalRefs">
                    <span className="wikiSignalLabel">affected：</span>
                    {s.affectedChunkIds.slice(0, 8).map((id) => (
                      <button
                        type="button"
                        key={id}
                        className="wikiSignalChunkBtn"
                        onClick={() => focusChunk(id)}
                        title="在 Inspector 中打开"
                      >
                        <code>{id}</code>
                      </button>
                    ))}
                    {s.affectedChunkIds.length > 8 ? (
                      <span className="wikiHint">+{s.affectedChunkIds.length - 8}</span>
                    ) : null}
                  </div>
                ) : null}
                {s.searchQueries.length > 0 ? (
                  <div className="wikiSignalRefs">
                    <span className="wikiSignalLabel">queries：</span>
                    {s.searchQueries.slice(0, 5).map((q, i) => (
                      <code key={`${s.signalId}-q-${i}`}>{q}</code>
                    ))}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

// ReviewView：把 active chunks 客户端分类成 5 类待评审视图。
//
// 5 类（互斥优先级，从严到宽）：
//   1. contested            integrityStatus=rejected — 被否的需要重新审视
//   2. needs_review         integrityStatus=needs_review — 等待运营初审
//   3. source_orphan        缺 sourceQuote 或 sourceAnchors — 无法定位回源文档
//   4. pending_verification integrityStatus=needs_review 且 已有 sourceQuote — 距离 verify 一步之遥
//   5. dependents_pending   relatedChunks 引用的 chunk 不在当前活跃集合 — 关系链残缺
//
// 处置走现有路由：
//   - Verify  → POST /api/operation-knowledge/chunks/:id/verify
//   - Reject  → POST /api/operation-knowledge/chunks/:id/reject
//   - Patch   → 切到 编辑历史 tab，用户用现有 ChunkRevisionsDrawer 修
//
// AI 永不自动 verify：所有按钮都是显式管理员维护动作，前端不做"批量自动 verify"。
const REVIEW_CATEGORIES: { v: ReviewCategory; label: string; hint: string }[] = [
  { v: "contested", label: "被否决", hint: "integrity_status=rejected — 已被管理员或 AI 否决，等待重新评估" },
  { v: "needs_review", label: "待初审", hint: "integrity_status=needs_review — 等待管理员初审或补完证据" },
  { v: "source_orphan", label: "缺源", hint: "缺 source_quote 或 source_anchors — 无法定位回原文档" },
  { v: "pending_verification", label: "待 verify", hint: "已经有 source_quote，距 verify 一步之遥" },
  { v: "dependents_pending", label: "关系残缺", hint: "related_chunks 引用了不在活跃集合中的 chunk" }
];

function ReviewView() {
  const [items, setItems] = useState<ReviewChunkItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [activeCategory, setActiveCategory] = useState<ReviewCategory>("needs_review");
  const [openBody, setOpenBody] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  // T9：在主区打开「审核+对话」双栏(ReviewChat)；非空时替代列表。
  const [chatChunk, setChatChunk] = useState<ReviewChatChunk | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/chunks");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: ReviewChunkItem[] };
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

  // 按优先级把每个 chunk 归入第一个命中的类别。
  // 注意：active chunks 列表也包含 status=rejected 的（status 与 integrity_status 是两条轴）；
  // contested 走 integrity_status，本视图不再二次过滤 status。
  const classified = useMemo(() => {
    const byId = new Set(items.map((i) => i.id));
    const out = new Map<ReviewCategory, ReviewChunkItem[]>();
    for (const cat of REVIEW_CATEGORIES) out.set(cat.v, []);
    for (const it of items) {
      const cat = classifyChunk(it, byId);
      if (cat) out.get(cat)!.push(it);
    }
    return out;
  }, [items]);

  const counts = useMemo(() => {
    const m = new Map<ReviewCategory, number>();
    for (const cat of REVIEW_CATEGORIES) m.set(cat.v, classified.get(cat.v)?.length ?? 0);
    return m;
  }, [classified]);

  const visible = classified.get(activeCategory) ?? [];

  function toggleBody(id: string) {
    setOpenBody((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function verify(id: string) {
    setBusyId(id);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(id)}/verify`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({}) }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(`已 verify：${id}`);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  async function reject(id: string) {
    setBusyId(id);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(id)}/reject`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({}) }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(`已 reject：${id}`);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function batchAction(action: "verify" | "archive") {
    if (selected.size === 0) return;
    if (action === "archive" && !window.confirm(`批量 archive ${selected.size} 条 chunk？`)) return;
    setBatchBusy(true);
    setError(null);
    setInfo(null);
    const ids = [...selected];
    const path =
      action === "verify"
        ? "/api/operation-knowledge/chunks/batch-verify"
        : "/api/operation-knowledge/chunks/batch-archive";
    const body =
      action === "verify"
        ? { ids, note: "batch verify (admin)" }
        : { ids, reason: "batch archive (admin)", actor: "admin" };
    try {
      const r = await fetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as {
        verified?: string[];
        archived?: string[];
        skipped?: { id: string; reason: string }[];
      };
      const okCount = (data.verified?.length ?? data.archived?.length ?? 0);
      const skippedCount = data.skipped?.length ?? 0;
      setInfo(
        `批量${action === "verify" ? "verify" : "archive"} 完成：成功 ${okCount}，跳过 ${skippedCount}` +
          (skippedCount > 0 && data.skipped
            ? `（${data.skipped.slice(0, 3).map((s) => `${s.id}:${s.reason}`).join("； ")}${skippedCount > 3 ? "…" : ""}）`
            : ""),
      );
      setSelected(new Set());
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBatchBusy(false);
    }
  }

  return (
    <div className="wikiArchiveShell wikiReviewBody">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">explore / review queue</div>
          <h2>评审队列</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <button type="button" className="ghost wikiBtn" onClick={() => void load()} disabled={loading}>
            <RefreshCw size={14} />
            {loading ? "加载中…" : "刷新"}
          </button>
        </div>
      </header>
      <div className="wikiToolbar">
        <span className="wikiHint">
          仅展示活跃 chunks。verify / reject 直接走现有路由，AI 永不自动 verify。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      {selected.size > 0 ? (
        <div className="wikiBatchToolbar">
          <span className="wikiArchiveTag">已选 {selected.size}</span>
          <button
            type="button"
            className="wikiBtn wikiActionBtn--verify"
            disabled={batchBusy}
            onClick={() => void batchAction("verify")}
          >
            <CheckCircle2 size={13} /> 批量 verify
          </button>
          <button
            type="button"
            className="wikiBtn"
            disabled={batchBusy}
            onClick={() => void batchAction("archive")}
          >
            <Archive size={13} /> 批量 archive
          </button>
          <button
            type="button"
            className="wikiBtn"
            disabled={batchBusy}
            onClick={() => setSelected(new Set())}
          >
            清空选中
          </button>
        </div>
      ) : null}
      <div className="wikiLintLayout">
        <div className="wikiReviewFilter">
          {REVIEW_CATEGORIES.map((cat) => {
            const c = counts.get(cat.v) ?? 0;
            return (
              <button
                type="button"
                key={cat.v}
                className={`wikiLintTreeNode ${activeCategory === cat.v ? "active" : ""} ${
                  c === 0 ? "empty" : ""
                }`}
                onClick={() => setActiveCategory(cat.v)}
                title={cat.hint}
              >
                <span>{cat.label}</span>
                <span className="wikiLintCount">{c}</span>
              </button>
            );
          })}
        </div>
        <div className="wikiLintPanel">
          {chatChunk ? (
            <>
              <button
                type="button"
                className="ghost wikiBtn"
                onClick={() => setChatChunk(null)}
              >
                <Undo2 size={14} />
                返回评审列表
              </button>
              <ReviewChat
                chunk={chatChunk}
                onResolved={() => {
                  setChatChunk(null);
                  void load();
                }}
              />
            </>
          ) : (
            <>
          {!loading && visible.length === 0 ? (
            <div className="wikiEmpty">
              当前类别没有待评审 chunk。
            </div>
          ) : null}
          <div className="wikiSignalList">
            {visible.map((c) => {
              const open = openBody.has(c.id);
              const hasQuote = !!c.sourceQuote && c.sourceQuote.trim().length > 0;
              const hasAnchor = (c.sourceAnchors?.length ?? 0) > 0;
              return (
                <div className="wikiReviewChunkCard" key={c.id}>
                  <div className="wikiSignalHead">
                    <input
                      type="checkbox"
                      className="wikiBatchCheckbox"
                      checked={selected.has(c.id)}
                      onChange={() => toggleSelect(c.id)}
                      title="选中以批量 verify / archive"
                    />
                    <div className="wikiSignalTitle">
                      <span className={`wikiKind ${c.wikiType ?? "unknown"}`}>{c.wikiType ?? "—"}</span>
                      <span className={`wikiSev ${c.integrityStatus === "rejected" ? "error" : "info"}`}>
                        {c.integrityStatus ?? "—"}
                      </span>
                      <button
                        type="button"
                        className="wikiReviewTitleBtn"
                        onClick={() => focusChunk(c.id)}
                        title="在 Inspector 中打开"
                      >
                        <strong>{c.title}</strong>
                      </button>
                    </div>
                    <div className="wikiSignalActions">
                      <button
                        type="button"
                        className="wikiReviewActionBtn verify"
                        onClick={() => void verify(c.id)}
                        disabled={busyId === c.id || !hasQuote || !hasAnchor}
                        title={!hasQuote || !hasAnchor ? "verify gate：需 sourceQuote + sourceAnchors 全有" : "标记为 verified"}
                      >
                        <CheckCircle2 size={14} />
                        Verify
                      </button>
                      <button
                        type="button"
                        className="wikiReviewActionBtn reject"
                        onClick={() => void reject(c.id)}
                        disabled={busyId === c.id}
                      >
                        <X size={14} />
                        Reject
                      </button>
                      <button
                        type="button"
                        className="wikiReviewActionBtn"
                        onClick={() => setChatChunk(c)}
                        title="打开审核+对话双栏：让 AI 改这条草稿，改完一键放行"
                      >
                        <MessageSquareText size={14} />
                        审核 / 对话
                      </button>
                    </div>
                  </div>
                  {c.summary ? <p className="wikiSignalDesc">{c.summary}</p> : null}
                  {hasQuote ? (
                    <blockquote className="wikiArchiveCitation">
                      {c.sourceQuote}
                      <span className="wikiArchiveCitationSource">{c.id}</span>
                    </blockquote>
                  ) : (
                    <div className="wikiHint">未配 source_quote — verify gate 将硬挡。</div>
                  )}
                  <div className="wikiReviewMeta">
                    <span>id：<code>{c.id}</code></span>
                    {hasAnchor ? (
                      <span>anchors：{c.sourceAnchors?.length ?? 0}</span>
                    ) : (
                      <span className="wikiBadge warn">无 anchors</span>
                    )}
                    {c.relatedChunks && c.relatedChunks.length > 0 ? (
                      <span>related：{c.relatedChunks.length}</span>
                    ) : null}
                    <button
                      type="button"
                      className="wikiCitedHead"
                      onClick={() => toggleBody(c.id)}
                    >
                      {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      {open ? "收起正文" : "展开正文"}
                    </button>
                  </div>
                  {open && c.body ? <pre className="wikiReviewBodyText">{c.body}</pre> : null}
                </div>
              );
            })}
          </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// KnowledgeTreeView：3 级树（wiki_type → business_topic → chunk title），右侧
// ChunkDetail 透出 source_quote 黄边块 + source_anchors 锚点 + related_chunks 跳转。
//
// 数据全部从 /api/operation-knowledge/chunks 取，纯客户端聚合：
//   l1: 9 类 wiki_type（source / entity / concept / comparison / synthesis /
//        methodology / finding / query / thesis；缺省落 "未分类"）
//   l2: chunk.business_topics[0]（缺省 "通用"）
//   l3: chunk.title（点击进入右侧 detail）
//
// 右侧 ChunkDetail 用同一个 chunk 数据渲染：
//   - title + wikiType + integrityStatus + status badge
//   - summary
//   - source_quote 黄边引用块（与 Review / Ask 视图风格一致）
//   - source_anchors 锚点列表：[startLine-endLine] [hash 短前缀]，hover 显示
//     完整 quoteHash + offsets；点击复制 anchor JSON 到剪贴板
//   - related_chunks 跳转 chip：点击切到目标 chunk
//   - body 默认收起；展开后用 <pre> 渲染原文（white-space pre-wrap）
//
// 全部只读：本视图不修改 chunk，verify / reject 走 ReviewView。
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

function ChatWorkbench() {
  const [sessionId, setSessionId] = useState<string>(() => {
    if (typeof window === "undefined") return "";
    return window.localStorage.getItem("knowledgeChat.sessionId") ?? "";
  });
  const [draft, setDraft] = useState("");
  const [attachChunkId, setAttachChunkId] = useState<string>("");
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
    if (!window.confirm("丢弃本会话的最后一份草稿？")) return;
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}/discard`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo("已丢弃当前草稿");
      await loadHistory(sessionId);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
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
          <span className="wikiArchiveTag">[session]</span>
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
              <span className="wikiArchiveTag">[{t.role === "user" ? "运营" : "AI"}]</span>
              <span className="wikiArchiveTimelineTime">#{t.turnIndex}</span>
              {t.intent ? <span className="wikiArchiveTag">[{t.intent}]</span> : null}
              {t.draftKind ? <span className="wikiArchiveTag">[{t.draftKind}]</span> : null}
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
                  <span key={f} className="wikiArchiveTag">
                    [{f}]
                  </span>
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

function KnowledgeInbox() {
  const [data, setData] = useState<InboxResp | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [priority, setPriority] = useState<"" | "high" | "mid" | "low">("");

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

  return (
    <div className="wikiArchiveShell wikiInbox">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">today / inbox</div>
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
          <span className="wikiArchiveTag">[total {data.stats.total}]</span>
          <span className="wikiArchiveTag">[high {data.stats.high}]</span>
          <span className="wikiArchiveTag">[mid {data.stats.mid}]</span>
          <span className="wikiArchiveTag">[low {data.stats.low}]</span>
        </div>
      ) : null}

      <div className="wikiInboxList">
        {data && data.items.length === 0 ? (
          <div className="wikiEmpty">
            <Inbox size={24} /> 暂无待办
          </div>
        ) : null}
        {data?.items.map((it) => (
          <article
            key={it.id}
            className={`wikiInboxCard wikiInboxCard--${it.priority}`}
          >
            <div className="wikiInboxCardHead">
              <span className={`wikiArchiveTag wikiInboxPriority--${it.priority}`}>
                [{it.priority}]
              </span>
              <span className="wikiArchiveTag">[{it.kind}]</span>
              <span className="wikiArchiveTag">[{it.origin}]</span>
              <span className="wikiArchiveTimelineTime">{it.createdAt}</span>
            </div>
            <h4 className="wikiInboxCardTitle">{it.title}</h4>
            <p className="wikiInboxCardSummary">{it.contextSummary}</p>
            <div className="wikiInboxCardActions">
              {it.targetChunkId ? (
                <button
                  type="button"
                  onClick={() => focusChunk(it.targetChunkId as string)}
                >
                  <ArrowRight size={12} /> 聚焦切片
                </button>
              ) : null}
              {it.suggestedActions.includes("open_chat") ? (
                <span className="wikiArchiveTag">[open_chat]</span>
              ) : null}
              {it.suggestedActions.includes("open_repair") ? (
                <span className="wikiArchiveTag">[open_repair]</span>
              ) : null}
              {it.suggestedActions.includes("dismiss") ? (
                <span className="wikiArchiveTag">[dismiss]</span>
              ) : null}
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}

interface CatalogPersistedView {
  total?: number;
  items?: unknown[];
}

interface LogsAnalyzeView {
  windowHours?: number;
  totalCalls?: number;
  avgTurns?: number;
  truncationRate?: number;
  samples?: unknown[];
}

interface IngestSourceItem {
  sourceId: string;
  workspaceId: string;
  kind: string;
  url: string;
  label?: string | null;
  scheduleMinutes: number;
  lastFetchedAt?: string | null;
  lastEtag?: string | null;
  status: string;
  failureStreak?: number;
  lastError?: string | null;
  ingestCount?: number;
  createdAt?: string | null;
  updatedAt?: string | null;
}

function IngestSourcesView() {
  const [items, setItems] = useState<IngestSourceItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState({ kind: "rss", url: "", label: "", scheduleMinutes: 60 });

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/knowledge/ingest-sources");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items?: IngestSourceItem[] };
      setItems(data.items ?? []);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void load(); }, []);

  async function handleCreate(ev: FormEvent) {
    ev.preventDefault();
    if (!draft.url.trim()) return;
    setCreating(true);
    try {
      const r = await fetch("/api/knowledge/ingest-sources", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          kind: draft.kind,
          url: draft.url.trim(),
          label: draft.label.trim() || null,
          scheduleMinutes: draft.scheduleMinutes,
        }),
      });
      if (!r.ok) throw await parseApiError(r);
      setDraft({ kind: "rss", url: "", label: "", scheduleMinutes: 60 });
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  }

  async function handleReactivate(id: string) {
    try {
      const r = await fetch(`/api/knowledge/ingest-sources/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ status: "active" }),
      });
      if (!r.ok) throw await parseApiError(r);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleDelete(id: string) {
    if (!window.confirm("删除外部源？已 ingest 的 chunks 不会被回收。")) return;
    try {
      const r = await fetch(`/api/knowledge/ingest-sources/${encodeURIComponent(id)}`, { method: "DELETE" });
      if (!r.ok) throw await parseApiError(r);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="wikiArchiveShell" style={{ padding: 18 }}>
      <header className="wikiArchiveHeader">
        <span className="wikiArchiveSubtitle">Ingest Sources · 外部源自动 ingest</span>
        <h3 style={{ fontSize: 20 }}>外部源</h3>
        <p style={{ color: "var(--wiki-muted)", fontSize: 12, marginTop: 6 }}>
          周期性拉取 RSS / HTML 源，落库 chunk 默认 draft + needs_review（AI 永不自动 verify）。
          连续 3 次失败 → status=failing；7 天不可达 → status=disabled。
        </p>
      </header>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      <form
        onSubmit={handleCreate}
        style={{
          display: "grid",
          gridTemplateColumns: "120px 1fr 1fr 140px auto",
          gap: 8,
          marginBottom: 16,
        }}
      >
        <select
          value={draft.kind}
          onChange={(e) => setDraft({ ...draft, kind: e.target.value })}
          className="wikiInput"
        >
          <option value="rss">rss</option>
          <option value="html">html</option>
        </select>
        <input
          type="text"
          placeholder="URL（必填）"
          value={draft.url}
          onChange={(e) => setDraft({ ...draft, url: e.target.value })}
          className="wikiInput"
        />
        <input
          type="text"
          placeholder="标签（可选，便于识别）"
          value={draft.label}
          onChange={(e) => setDraft({ ...draft, label: e.target.value })}
          className="wikiInput"
        />
        <input
          type="number"
          min={1}
          placeholder="间隔（分钟）"
          value={draft.scheduleMinutes}
          onChange={(e) => setDraft({ ...draft, scheduleMinutes: Number(e.target.value) || 60 })}
          className="wikiInput"
        />
        <button type="submit" className="wikiBtn primary" disabled={creating || !draft.url.trim()}>
          {creating ? "创建中…" : "新增"}
        </button>
      </form>
      {loading ? (
        <div className="wikiHint">加载中…</div>
      ) : items.length === 0 ? (
        <div className="wikiHint">暂无外部源。新增后由 worker 周期拉取。</div>
      ) : (
        <table className="wikiTable" style={{ width: "100%", fontSize: 13 }}>
          <thead>
            <tr>
              <th style={{ textAlign: "left" }}>kind</th>
              <th style={{ textAlign: "left" }}>URL / 标签</th>
              <th style={{ textAlign: "right" }}>间隔</th>
              <th style={{ textAlign: "left" }}>最后拉取</th>
              <th style={{ textAlign: "left" }}>状态</th>
              <th style={{ textAlign: "right" }}>失败次数</th>
              <th style={{ textAlign: "right" }}>已 ingest</th>
              <th style={{ textAlign: "left" }}>错误</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {items.map((it) => (
              <tr key={it.sourceId}>
                <td>{it.kind}</td>
                <td style={{ maxWidth: 360, wordBreak: "break-all" }}>
                  <div>{it.url}</div>
                  {it.label ? (
                    <div style={{ color: "var(--wiki-muted)", fontSize: 11 }}>{it.label}</div>
                  ) : null}
                </td>
                <td style={{ textAlign: "right" }}>{it.scheduleMinutes}m</td>
                <td style={{ fontSize: 11, color: "var(--wiki-muted)" }}>
                  {it.lastFetchedAt ? new Date(it.lastFetchedAt).toLocaleString() : "—"}
                </td>
                <td>
                  <span
                    className="wikiBadge"
                    style={{
                      background:
                        it.status === "active"
                          ? "var(--wiki-ok-bg, #d1fadf)"
                          : it.status === "failing"
                          ? "var(--wiki-warn-bg, #fef0c7)"
                          : "var(--wiki-error-bg, #fee4e2)",
                    }}
                  >
                    {it.status}
                  </span>
                </td>
                <td style={{ textAlign: "right" }}>{it.failureStreak ?? 0}</td>
                <td style={{ textAlign: "right" }}>{it.ingestCount ?? 0}</td>
                <td style={{ maxWidth: 220, fontSize: 11, color: "var(--wiki-error, #b42318)" }}>
                  {it.lastError ?? ""}
                </td>
                <td style={{ whiteSpace: "nowrap" }}>
                  {it.status !== "active" ? (
                    <button
                      type="button"
                      className="wikiBtn"
                      onClick={() => handleReactivate(it.sourceId)}
                    >
                      重新激活
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="wikiBtn danger"
                    style={{ marginLeft: 6 }}
                    onClick={() => handleDelete(it.sourceId)}
                  >
                    删除
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ObservabilityDashboard() {
  const [catalog, setCatalog] = useState<CatalogPersistedView | null>(null);
  const [catalogLive, setCatalogLive] = useState<{ total?: number } | null>(null);
  const [completeness, setCompleteness] = useState<CompletenessView | null>(null);
  const [integrity, setIntegrity] = useState<IntegrityReportView | null>(null);
  const [logs, setLogs] = useState<LogsAnalyzeView | null>(null);
  const [cacheStats, setCacheStats] = useState<{
    entries?: number;
    hits?: number;
    misses?: number;
    maxEntries?: number;
    ttlSeconds?: number;
  } | null>(null);
  const [phaseRollup, setPhaseRollup] = useState<{
    windowHours?: number;
    lifecycle?: Array<{ lifecycle: string; count: number; outOfClosedSet?: boolean }>;
    revisionReasons?: Array<{ reason: string; count: number }>;
    reviewerMisjudge?: Array<{ kind: string; count: number }>;
    negativeExamplePending?: number;
  } | null>(null);
  const [workerHealth, setWorkerHealth] = useState<{
    chatTasks?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      errorKindsTop?: Array<{ errorKind: string; count: number }>;
    };
    gapSignals?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      pendingKindsTop?: Array<{ kind: string; count: number }>;
      total?: number;
      pending?: number;
      resolved?: number;
      sweepHitRate?: number;
    };
    lessonsLearned?: {
      windowDays?: number;
      patternTop?: Array<{ pattern: string; count: number; outOfClosedSet?: boolean }>;
      blockedTotal?: number;
    };
  } | null>(null);
  const [pending, setPending] = useState(false);
  const [sweeping, setSweeping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const [a, b, c, d, e, f, g, h] = await Promise.all([
        fetch("/api/operation-knowledge/catalog/persisted").then((r) => r.json()),
        fetch("/api/operation-knowledge/catalog").then((r) => r.json()),
        fetch("/api/operation-knowledge/completeness").then((r) => r.json()),
        fetch("/api/operation-knowledge/integrity-report").then((r) => r.json()),
        fetch("/api/operation-knowledge/logs/analyze").then((r) => r.json()),
        fetch("/api/knowledge/metrics").then((r) => r.json()),
        fetch("/api/admin/observability/phase-rollup").then((r) => r.json()),
        fetch("/api/admin/observability/worker-health").then((r) => r.json())
      ]);
      setCatalog(a as CatalogPersistedView);
      setCatalogLive(b as { total?: number });
      setCompleteness(parseCompleteness(c));
      setIntegrity(parseIntegrityReport(d));
      setLogs(e as LogsAnalyzeView);
      const metrics = f as { answerCache?: typeof cacheStats };
      setCacheStats(metrics?.answerCache ?? null);
      setPhaseRollup(g as typeof phaseRollup);
      setWorkerHealth(h as typeof workerHealth);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function sweep() {
    setSweeping(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/gap-signals/sweep", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{}"
      });
      if (!r.ok) throw await parseApiError(r);
      setInfo("已触发 gap-signals sweep");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSweeping(false);
    }
  }

  const persistedTotal = catalog?.total ?? catalog?.items?.length ?? 0;
  const liveTotal = catalogLive?.total ?? 0;
  const drift = liveTotal - persistedTotal;

  return (
    <div className="wikiArchiveShell wikiObservability">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">steward / diagnostics</div>
          <h2>诊断仪表</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <div className="wikiBannerError">{error}</div> : null}
      {info ? <div className="wikiBannerInfo">{info}</div> : null}

      <div className="wikiObservabilityGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[catalog]</span>
            <h4>目录覆盖</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>持久化</dt>
            <dd>{persistedTotal}</dd>
            <dt>实时</dt>
            <dd>{liveTotal}</dd>
            <dt>偏差</dt>
            <dd className={drift !== 0 ? "wikiObservabilityDrift" : undefined}>
              {drift > 0 ? `+${drift}` : drift}
            </dd>
          </dl>
          <button
            type="button"
            className="wikiObservabilityCta"
            onClick={() => void sweep()}
            disabled={sweeping}
          >
            <Workflow size={12} /> {sweeping ? "扫描中…" : "触发 sweep"}
          </button>
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[completeness]</span>
            <h4>类型完整度</h4>
          </header>
          {completeness ? (
            <dl className="wikiArchiveMeta">
              <dt>应答模式</dt>
              <dd>{completeness.answeringMode}</dd>
              <dt>已验证</dt>
              <dd>
                {completeness.verifiedChunks}/{completeness.totalChunks}
              </dd>
              {completeness.summary ? (
                <>
                  <dt>摘要</dt>
                  <dd>{completeness.summary}</dd>
                </>
              ) : null}
              {/* coverage 5 维裁决渲染见后续 cockpit 任务 */}
            </dl>
          ) : (
            <div className="wikiEmpty">无完整度数据</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[integrity]</span>
            <h4>完整性诊断</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>needs_review</dt>
            <dd>{integrity?.needsReview ?? 0}</dd>
            <dt>verified</dt>
            <dd>{integrity?.verified ?? 0}</dd>
            <dt>rejected</dt>
            <dd>{integrity?.rejected ?? 0}</dd>
            <dt>total</dt>
            <dd>{integrity?.total ?? 0}</dd>
          </dl>
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[logs]</span>
            <h4>检索 trace（24h）</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>窗口</dt>
            <dd>{logs?.windowHours ?? 24}h</dd>
            <dt>调用</dt>
            <dd>{logs?.totalCalls ?? 0}</dd>
            <dt>平均轮数</dt>
            <dd>{logs?.avgTurns?.toFixed?.(1) ?? "—"}</dd>
            <dt>截断率</dt>
            <dd>
              {typeof logs?.truncationRate === "number"
                ? `${(logs.truncationRate * 100).toFixed(1)}%`
                : "—"}
            </dd>
          </dl>
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[answer-cache]</span>
            <h4>问答缓存</h4>
          </header>
          {(() => {
            const hits = cacheStats?.hits ?? 0;
            const misses = cacheStats?.misses ?? 0;
            const total = hits + misses;
            const ratio = total > 0 ? (hits / total) * 100 : null;
            return (
              <dl className="wikiArchiveMeta">
                <dt>条目</dt>
                <dd>{cacheStats?.entries ?? 0} / {cacheStats?.maxEntries ?? "—"}</dd>
                <dt>命中</dt>
                <dd>{hits}</dd>
                <dt>未命中</dt>
                <dd>{misses}</dd>
                <dt>命中率</dt>
                <dd className={ratio !== null && ratio >= 30 ? "wikiObservabilityDrift" : undefined}>
                  {ratio === null ? "—" : `${ratio.toFixed(1)}%`}
                </dd>
                <dt>TTL</dt>
                <dd>{cacheStats?.ttlSeconds ?? "—"}s</dd>
              </dl>
            );
          })()}
        </article>
      </div>

      <PhaseRollupPanel data={phaseRollup} />

      <WorkerHealthPanel data={workerHealth} />

      <TestMatchPanel />
    </div>
  );
}

function PhaseRollupPanel({
  data
}: {
  data: {
    windowHours?: number;
    lifecycle?: Array<{ lifecycle: string; count: number; outOfClosedSet?: boolean }>;
    revisionReasons?: Array<{ reason: string; count: number }>;
    reviewerMisjudge?: Array<{ kind: string; count: number }>;
    negativeExamplePending?: number;
  } | null;
}) {
  if (!data) {
    return null;
  }
  const lifecycle = data.lifecycle ?? [];
  const lifecycleTotal = lifecycle.reduce((sum, row) => sum + (row.count ?? 0), 0);
  const revisionReasons = data.revisionReasons ?? [];
  const reviewerMisjudge = data.reviewerMisjudge ?? [];
  const negativeExamplePending = data.negativeExamplePending ?? 0;
  const windowHours = data.windowHours ?? 24;

  return (
    <section className="wikiObservabilityPhaseRollup">
      <header className="wikiObservabilityCardHead">
        <span className="wikiArchiveTag">[phase-rollup]</span>
        <h4>Phase 0-D 自治信号（{windowHours}h）</h4>
      </header>
      <div className="wikiObservabilityGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[lifecycle]</span>
            <h4>run lifecycle 终态</h4>
          </header>
          {lifecycleTotal === 0 ? (
            <div className="wikiEmpty">窗口内无 run</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {lifecycle.map((row, i) => (
                <Fragment key={i}>
                  <dt>
                    {row.lifecycle}
                    {row.outOfClosedSet ? (
                      <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                    ) : null}
                  </dt>
                  <dd>{row.count}</dd>
                </Fragment>
              ))}
            </dl>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[revision]</span>
            <h4>single-shot revision top</h4>
          </header>
          {revisionReasons.length === 0 ? (
            <div className="wikiEmpty">窗口内无 revision</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {revisionReasons.map((row, i) => (
                <Fragment key={i}>
                  <dt>{row.reason}</dt>
                  <dd>{row.count}</dd>
                </Fragment>
              ))}
            </dl>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[reviewer]</span>
            <h4>reviewer 误判信号</h4>
          </header>
          {reviewerMisjudge.length === 0 ? (
            <div className="wikiEmpty">窗口内无误判信号</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {reviewerMisjudge.map((row, i) => (
                <Fragment key={i}>
                  <dt>{row.kind}</dt>
                  <dd>{row.count}</dd>
                </Fragment>
              ))}
            </dl>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[negative-example]</span>
            <h4>负例候选 needs_review</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>待审核</dt>
            <dd
              className={
                negativeExamplePending > 0 ? "wikiObservabilityDrift" : undefined
              }
            >
              {negativeExamplePending}
            </dd>
          </dl>
        </article>
      </div>
    </section>
  );
}

// G-后续Ⅱ/2 · ObservabilityDashboard 第二波卡片：worker 健康聚合
//   - knowledge_chat_tasks 状态分布 + 失败 error_kind top
//   - knowledge_gap_signals sweep 命中率 + pending kind top
//   - lessons_learned 14d pattern × review_status 矩阵 + blocked_total
function WorkerHealthPanel({
  data
}: {
  data: {
    chatTasks?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      errorKindsTop?: Array<{ errorKind: string; count: number }>;
    };
    gapSignals?: {
      byStatus?: Array<{ status: string; count: number; outOfClosedSet?: boolean }>;
      pendingKindsTop?: Array<{ kind: string; count: number }>;
      total?: number;
      pending?: number;
      resolved?: number;
      sweepHitRate?: number;
    };
    lessonsLearned?: {
      windowDays?: number;
      patternTop?: Array<{ pattern: string; count: number; outOfClosedSet?: boolean }>;
      blockedTotal?: number;
    };
  } | null;
}) {
  if (!data) {
    return null;
  }
  const chatStatuses = data.chatTasks?.byStatus ?? [];
  const chatTotal = chatStatuses.reduce((sum, row) => sum + (row.count ?? 0), 0);
  const chatErrors = data.chatTasks?.errorKindsTop ?? [];
  const gapStatuses = data.gapSignals?.byStatus ?? [];
  const gapKinds = data.gapSignals?.pendingKindsTop ?? [];
  const gapTotal = data.gapSignals?.total ?? 0;
  const gapPending = data.gapSignals?.pending ?? 0;
  const gapResolved = data.gapSignals?.resolved ?? 0;
  const gapHitRate = data.gapSignals?.sweepHitRate ?? 0;
  const lessonsPatterns = data.lessonsLearned?.patternTop ?? [];
  const lessonsBlocked = data.lessonsLearned?.blockedTotal ?? 0;
  const lessonsWindow = data.lessonsLearned?.windowDays ?? 14;

  return (
    <section className="wikiObservabilityPhaseRollup">
      <header className="wikiObservabilityCardHead">
        <span className="wikiArchiveTag">[worker-health]</span>
        <h4>worker 健康聚合</h4>
      </header>
      <div className="wikiObservabilityGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[chat-tasks]</span>
            <h4>chat task 状态</h4>
          </header>
          {chatTotal === 0 ? (
            <div className="wikiEmpty">无任务</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {chatStatuses.map((row, i) => (
                <Fragment key={i}>
                  <dt>
                    {row.status}
                    {row.outOfClosedSet ? (
                      <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                    ) : null}
                  </dt>
                  <dd
                    className={
                      row.status === "failed" && row.count > 0
                        ? "wikiObservabilityDrift"
                        : undefined
                    }
                  >
                    {row.count}
                  </dd>
                </Fragment>
              ))}
            </dl>
          )}
          {chatErrors.length > 0 ? (
            <div className="wikiArchiveCitation">
              <strong>error_kind top</strong>
              <ul style={{ margin: 0, paddingLeft: "1.2em" }}>
                {chatErrors.map((row, i) => (
                  <li key={i}>
                    <span className="wikiArchiveTag">[{row.errorKind}]</span> {row.count}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[gap-signals]</span>
            <h4>gap signals · sweep 命中率</h4>
          </header>
          <dl className="wikiArchiveMeta">
            <dt>总计</dt>
            <dd>{gapTotal}</dd>
            <dt>pending</dt>
            <dd className={gapPending > 0 ? "wikiObservabilityDrift" : undefined}>
              {gapPending}
            </dd>
            <dt>resolved</dt>
            <dd>{gapResolved}</dd>
            <dt>命中率</dt>
            <dd className={gapHitRate >= 0.5 ? "wikiObservabilityDrift" : undefined}>
              {gapTotal === 0 ? "—" : `${(gapHitRate * 100).toFixed(1)}%`}
            </dd>
          </dl>
          {gapKinds.length > 0 ? (
            <div className="wikiArchiveCitation">
              <strong>pending kind top</strong>
              <ul style={{ margin: 0, paddingLeft: "1.2em" }}>
                {gapKinds.map((row, i) => (
                  <li key={i}>
                    <span className="wikiArchiveTag">[{row.kind}]</span> {row.count}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
          {gapStatuses.length > 0 ? (
            <details>
              <summary>状态明细</summary>
              <dl className="wikiArchiveMeta">
                {gapStatuses.map((row, i) => (
                  <Fragment key={i}>
                    <dt>
                      {row.status}
                      {row.outOfClosedSet ? (
                        <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                      ) : null}
                    </dt>
                    <dd>{row.count}</dd>
                  </Fragment>
                ))}
              </dl>
            </details>
          ) : null}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[lessons-learned]</span>
            <h4>lessons_learned ({lessonsWindow}d)</h4>
          </header>
          {lessonsPatterns.length === 0 ? (
            <div className="wikiEmpty">窗口内无产出</div>
          ) : (
            <dl className="wikiArchiveMeta">
              {lessonsPatterns.map((row, i) => (
                <Fragment key={i}>
                  <dt>
                    {row.pattern}
                    {row.outOfClosedSet ? (
                      <span className="wikiObservabilityDrift"> · out-of-closed-set</span>
                    ) : null}
                  </dt>
                  <dd
                    className={
                      row.pattern === "blocked_by_safety_guard" && row.count > 0
                        ? "wikiObservabilityDrift"
                        : undefined
                    }
                  >
                    {row.count}
                  </dd>
                </Fragment>
              ))}
              <dt>blocked_total</dt>
              <dd className={lessonsBlocked > 0 ? "wikiObservabilityDrift" : undefined}>
                {lessonsBlocked}
              </dd>
            </dl>
          )}
        </article>
      </div>
    </section>
  );
}

function TestMatchPanel() {
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    const q = query.trim();
    if (!q) {
      setError("请输入查询");
      return;
    }
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/test-match", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query: q })
      });
      if (!r.ok) throw await parseApiError(r);
      setResult(await r.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="wikiObservabilityCard wikiTestMatch">
      <header className="wikiObservabilityCardHead">
        <span className="wikiArchiveTag">[test-match]</span>
        <h4>检索调试</h4>
      </header>
      <div className="wikiTestMatchRow">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="输入查询，看哪些 chunk 命中 + grounding score"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void run();
            }
          }}
        />
        <button type="button" className="primary" onClick={() => void run()} disabled={pending}>
          <Search size={12} /> {pending ? "查询中…" : "试算"}
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      {result ? (
        <pre className="wikiTestMatchResult">{JSON.stringify(result, null, 2)}</pre>
      ) : null}
    </section>
  );
}


function KnowledgeTreeView() {
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
            <div className="wikiEmpty">从左侧选择一个 chunk 查看详情。</div>
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
          <span className={`wikiKind ${chunk.wikiType ?? "unknown"}`}>{chunk.wikiType ?? "—"}</span>
          <h3>{chunk.title}</h3>
        </div>
        <div className="wikiChunkDetailMeta">
          <span className={`wikiSev ${chunk.integrityStatus === "rejected" ? "error" : "info"}`}>
            {chunk.integrityStatus ?? "—"}
          </span>
          <span className="wikiBadge">{chunk.status ?? "—"}</span>
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

interface ChunkRevisionItem {
  revisionId: string;
  chunkId: string;
  op: string;
  patch: unknown;
  beforeHash: string;
  afterHash: string;
  source: string;
  reason?: string | null;
  createdAt?: string | null;
  createdBy?: string | null;
}

function ChunkRevisionsDrawer() {
  const [chunkId, setChunkId] = useState("");
  const [pending, setPending] = useState(false);
  const [items, setItems] = useState<ChunkRevisionItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  async function load() {
    const id = chunkId.trim();
    if (!id) {
      setError("请输入 chunkId（24 位 ObjectId 十六进制）。");
      return;
    }
    setPending(true);
    setError(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/chunks/${encodeURIComponent(id)}/revisions?limit=100`,
      );
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: ChunkRevisionItem[] };
      setItems(data.items ?? []);
      setExpanded(new Set());
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
      setItems([]);
    } finally {
      setPending(false);
    }
  }

  function toggle(revisionId: string) {
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(revisionId)) next.delete(revisionId);
      else next.add(revisionId);
      return next;
    });
  }

  return (
    <div className="wikiPanelBody">
      <div className="wikiToolbar">
        <input
          type="text"
          className="wikiInput"
          placeholder="输入 chunkId 查看 revision 历史"
          value={chunkId}
          onChange={(e) => setChunkId(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void load();
          }}
        />
        <button type="button" className="primary" onClick={() => void load()} disabled={pending}>
          {pending ? "加载中…" : "拉取历史"}
        </button>
        <span className="wikiHint">timeline 倒序；每行展开查看 patch JSON 与 hash。</span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {!pending && chunkId && items.length === 0 && !error ? (
        <div className="wikiEmpty">该 chunk 暂无 revision 记录。</div>
      ) : null}
      <div className="wikiRevisionList">
        {items.map((r) => {
          const isOpen = expanded.has(r.revisionId);
          return (
            <div className={`wikiRevCard op-${r.op}`} key={r.revisionId}>
              <div className="wikiRevHead" onClick={() => toggle(r.revisionId)}>
                <span className={`wikiOp ${r.op}`}>{r.op}</span>
                <span className={`wikiSource ${r.source}`}>{r.source}</span>
                <span className="wikiRevTime">{r.createdAt ?? ""}</span>
                <span className="wikiRevId">{r.revisionId}</span>
                {r.reason ? <span className="wikiRevReason">{r.reason}</span> : null}
                {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              </div>
              {isOpen ? (
                <div className="wikiRevBody">
                  <div className="wikiRevHash">
                    <span>before: {r.beforeHash || "-"}</span>
                    <span>after: {r.afterHash || "-"}</span>
                    {r.createdBy ? <span>by: {r.createdBy}</span> : null}
                  </div>
                  <pre className="wikiRevPatch">{JSON.stringify(r.patch, null, 2)}</pre>
                </div>
              ) : null}
            </div>
          );
        })}
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

function DigestCanvas() {
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
              <span className={severityBadgeClass(card.severity)}>{card.severity}</span>
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

async function tryLlmError(_resp: Response, fallback: string): Promise<Error> {
  return new Error(fallback);
}
void tryLlmError;

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

function TaskRail() {
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



export default function KnowledgeFeature() {
  return <KnowledgeWikiView />;
}
