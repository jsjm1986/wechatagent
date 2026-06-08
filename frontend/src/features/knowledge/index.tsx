import { useState, useEffect } from "react";
import {
  Activity,
  AlertTriangle,
  BookOpen,
  BrainCircuit,
  Calendar,
  ChevronRight,
  Clock3,
  Compass,
  FileBox,
  FileText,
  Inbox,
  Map as MapIcon,
  MessageSquareText,
  Network,
  Rss,
  Search,
  ShieldCheck,
  Sparkles,
  UploadCloud,
  Wrench,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { ChunkInspectorPane } from "./shared";
import { ChunkGraphView, DomainSchemaTab, MetricsTab, MemoryDrawer, AdminGovernanceView } from "./atlas";
import { AskView, KnowledgeTreeView } from "./explore";
import { DigestCanvas, ChatWorkbench, KnowledgeInbox, TaskRail } from "./today";
import {
  LintView, ReviewView, DocumentsView, ImportWizard,
  IngestSourcesView, ObservabilityDashboard, TryRecallView, ChunkRevisionsDrawer,
} from "./steward";
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




export default function KnowledgeFeature() {
  return <KnowledgeWikiView />;
}
