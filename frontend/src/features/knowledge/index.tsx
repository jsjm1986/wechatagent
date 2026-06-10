import { useState, useEffect, useCallback } from "react";
import {
  Activity,
  AlertTriangle,
  BookOpen,
  BrainCircuit,
  ChevronDown,
  ChevronRight,
  Clock3,
  Compass,
  FileBox,
  FileText,
  Inbox,
  LayoutDashboard,
  Library,
  MessageSquareText,
  Network,
  Rss,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  UploadCloud,
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
import { AutoVerifyPanel } from "./cockpit/AutoVerifyPanel";
import { ConfirmProvider } from "../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../components/ui/Toast";
import { FormDialogProvider } from "../../components/ui/FormDialog";
import "./Knowledge.css";

// knowledge-wiki 频道——agent-first 渐进式披露主入口。
//
// 信息架构（IA 重组后）：4 模式 → 3 模式，按"运营意图"而非"后端数据模型"组织。
//   - 工作台 workbench：今日待办与起草（Digest / AI 协作 / 待办收件箱 + 派工 + Inspector）
//   - 知识库 library：问答、浏览与治理（问答 / 知识树 / 质量信号 / 待评审 / 批量校验 / 修订历史 + Inspector）
//   - 控制台 console：录入、Schema 与系统（概览 / 内容录入分组 / Schema·系统配置 / 高级折叠诊断）
//
// 跨模式交互（顶层状态提升，避免事件丢失）：
//   - B1 focusChunk：任意位置聚焦 chunk → 若当前模式无 Inspector(console) 先切到 library，再下发 focusedId
//   - B2 待办→AI协作：收件箱「找 AI 协作」→ 切 workbench/chat 并预填 attachChunkId
//   - B8 概览下钻：CockpitView CoverageVerdict 维度 → 切 library/review 并带 initialDimFilter
type KnowledgeMode = "workbench" | "library" | "console";

interface ModeMeta {
  key: KnowledgeMode;
  label: string;
  caption: string;
  Icon: LucideIcon;
}

const KNOWLEDGE_MODES: ModeMeta[] = [
  { key: "workbench", label: "工作台", caption: "今日待办与起草", Icon: LayoutDashboard },
  { key: "library", label: "知识库", caption: "问答、浏览与治理", Icon: Library },
  { key: "console", label: "控制台", caption: "录入、Schema 与系统", Icon: SlidersHorizontal },
];

type WorkbenchPane = "digest" | "chat" | "inbox";
type LibraryPane = "ask" | "tree" | "lint" | "review" | "autoVerify" | "revisions";
type ConsolePane =
  | "cockpit" | "documents" | "import" | "ingest" | "schema" | "sysconfig"
  | "observability" | "tryRecall" | "metrics" | "memory" | "graph";

export function KnowledgeWikiView() {
  const [mode, setMode] = useState<KnowledgeMode>("workbench");

  // 跨模式共享状态（提升到顶层）
  const [workbenchPane, setWorkbenchPane] = useState<WorkbenchPane>("digest");
  const [libraryPane, setLibraryPane] = useState<LibraryPane>("ask");
  const [consolePane, setConsolePane] = useState<ConsolePane>("cockpit");

  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [inspectorCollapsed, setInspectorCollapsed] = useState(true);
  const [reviewDimFilter, setReviewDimFilter] = useState<string | null>(null);
  const [chatAttach, setChatAttach] = useState<string | null>(null);

  // B1：唯一的全局 focusChunk 监听。Inspector 只挂在 workbench / library，
  // 若当前在 console 收到聚焦事件，先切到 library 再下发，杜绝"死跳转"。
  useEffect(() => {
    function onFocus(e: Event) {
      const ce = e as CustomEvent<{ chunkId?: string }>;
      const id = ce.detail?.chunkId;
      if (typeof id === "string" && id) {
        setMode((m) => (m === "console" ? "library" : m));
        setFocusedId(id);
        setInspectorCollapsed(false);
      }
    }
    function onOpenCockpit() {
      setMode("console");
      setConsolePane("cockpit");
    }
    window.addEventListener("wikiFocusChunk", onFocus as EventListener);
    window.addEventListener("wikiOpenCockpit", onOpenCockpit);
    return () => {
      window.removeEventListener("wikiFocusChunk", onFocus as EventListener);
      window.removeEventListener("wikiOpenCockpit", onOpenCockpit);
    };
  }, []);

  // B2：待办收件箱「找 AI 协作」→ 工作台/AI 协作并预填 chunkId。
  const openChatWith = useCallback((chunkId?: string) => {
    setChatAttach(chunkId ?? null);
    setWorkbenchPane("chat");
    setMode("workbench");
  }, []);

  // B8：概览维度下钻 → 知识库/待评审并带维度上下文。
  const openReviewForDim = useCallback((dimKey?: string) => {
    setReviewDimFilter(dimKey ?? null);
    setLibraryPane("review");
    setMode("library");
  }, []);

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
        {mode === "workbench" && (
          <WorkbenchMode
            pane={workbenchPane}
            setPane={setWorkbenchPane}
            chatAttach={chatAttach}
            onOpenChat={openChatWith}
            focusedId={focusedId}
            inspectorCollapsed={inspectorCollapsed}
            setInspectorCollapsed={setInspectorCollapsed}
            setFocusedId={setFocusedId}
          />
        )}
        {mode === "library" && (
          <LibraryMode
            pane={libraryPane}
            setPane={setLibraryPane}
            reviewDimFilter={reviewDimFilter}
            focusedId={focusedId}
            inspectorCollapsed={inspectorCollapsed}
            setInspectorCollapsed={setInspectorCollapsed}
            setFocusedId={setFocusedId}
          />
        )}
        {mode === "console" && (
          <ConsoleMode
            pane={consolePane}
            setPane={setConsolePane}
            onOpenReview={openReviewForDim}
            onOpenAutoVerify={() => { setLibraryPane("autoVerify"); setMode("library"); }}
          />
        )}
      </div>
    </section>
  );
}

// ── 工作台 workbench：今日待办与起草 ─────────────────────────────────
function WorkbenchMode({
  pane, setPane, chatAttach, onOpenChat,
  focusedId, inspectorCollapsed, setInspectorCollapsed, setFocusedId,
}: {
  pane: WorkbenchPane;
  setPane: (p: WorkbenchPane) => void;
  chatAttach: string | null;
  onOpenChat: (chunkId?: string) => void;
  focusedId: string | null;
  inspectorCollapsed: boolean;
  setInspectorCollapsed: (v: boolean) => void;
  setFocusedId: (v: string | null) => void;
}) {
  return (
    <div className={`wikiModeGrid wikiModeGrid--steward${!inspectorCollapsed ? " has-inspector" : ""}`}>
      <div className="wikiModePane wikiModePane--nav wikiStewardNav">
        <NavBtn active={pane === "digest"} onClick={() => setPane("digest")} Icon={Sparkles} label="今日 Digest" />
        <NavBtn active={pane === "chat"} onClick={() => setPane("chat")} Icon={MessageSquareText} label="AI 协作" />
        <NavBtn active={pane === "inbox"} onClick={() => setPane("inbox")} Icon={Inbox} label="待办收件箱" />
        <div className="wikiNavSpacer" />
        <TaskRail />
      </div>
      <div className="wikiModePane wikiModePane--main">
        {pane === "digest" && <DigestCanvas />}
        {pane === "chat" && <ChatWorkbench initialAttachChunkId={chatAttach} />}
        {pane === "inbox" && <KnowledgeInbox onOpenChat={onOpenChat} />}
      </div>
      {!inspectorCollapsed ? (
        <ChunkInspectorPane
          chunkId={focusedId}
          onClose={() => setInspectorCollapsed(true)}
          onClear={() => setFocusedId(null)}
        />
      ) : null}
    </div>
  );
}

// ── 知识库 library：问答、浏览与治理 ─────────────────────────────────
function LibraryMode({
  pane, setPane, reviewDimFilter,
  focusedId, inspectorCollapsed, setInspectorCollapsed, setFocusedId,
}: {
  pane: LibraryPane;
  setPane: (p: LibraryPane) => void;
  reviewDimFilter: string | null;
  focusedId: string | null;
  inspectorCollapsed: boolean;
  setInspectorCollapsed: (v: boolean) => void;
  setFocusedId: (v: string | null) => void;
}) {
  return (
    <div className={`wikiModeGrid wikiModeGrid--steward${!inspectorCollapsed ? " has-inspector" : ""}`}>
      <div className="wikiModePane wikiModePane--nav wikiStewardNav">
        <NavBtn active={pane === "ask"} onClick={() => setPane("ask")} Icon={Compass} label="知识问答" />
        <NavBtn active={pane === "tree"} onClick={() => setPane("tree")} Icon={BookOpen} label="知识树" />
        <NavBtn active={pane === "lint"} onClick={() => setPane("lint")} Icon={AlertTriangle} label="质量信号" />
        <NavBtn active={pane === "review"} onClick={() => setPane("review")} Icon={ShieldCheck} label="待评审" />
        <NavBtn active={pane === "autoVerify"} onClick={() => setPane("autoVerify")} Icon={Sparkles} label="批量校验" />
        <NavBtn active={pane === "revisions"} onClick={() => setPane("revisions")} Icon={Clock3} label="修订历史" />
      </div>
      <div className="wikiModePane wikiModePane--main">
        {pane === "ask" && <AskView />}
        {pane === "tree" && <KnowledgeTreeView />}
        {pane === "lint" && <LintView />}
        {pane === "review" && <ReviewView initialDimFilter={reviewDimFilter} />}
        {pane === "autoVerify" && <AutoVerifyPanel />}
        {pane === "revisions" && <ChunkRevisionsDrawer />}
      </div>
      {!inspectorCollapsed ? (
        <ChunkInspectorPane
          chunkId={focusedId}
          onClose={() => setInspectorCollapsed(true)}
          onClear={() => setFocusedId(null)}
        />
      ) : null}
    </div>
  );
}

// ── 控制台 console：录入、Schema 与系统 ──────────────────────────────
function ConsoleMode({
  pane, setPane, onOpenReview, onOpenAutoVerify,
}: {
  pane: ConsolePane;
  setPane: (p: ConsolePane) => void;
  onOpenReview: (dimKey?: string) => void;
  onOpenAutoVerify: () => void;
}) {
  // 「高级」诊断分组默认折叠——满足"调试面板保留但不铺给运营"。
  const advancedPanes: ConsolePane[] = ["observability", "tryRecall", "metrics", "memory", "graph"];
  const [advancedOpen, setAdvancedOpen] = useState(advancedPanes.includes(pane));

  return (
    <div className="wikiModeGrid wikiModeGrid--atlas">
      <div className="wikiModePane wikiModePane--nav wikiStewardNav">
        <NavBtn active={pane === "cockpit"} onClick={() => setPane("cockpit")} Icon={ShieldCheck} label="概览" />

        <div className="wikiNavGroupTitle">内容录入</div>
        <NavBtn active={pane === "documents"} onClick={() => setPane("documents")} Icon={FileText} label="文档目录" />
        <NavBtn active={pane === "import"} onClick={() => setPane("import")} Icon={UploadCloud} label="导入向导" />
        <NavBtn active={pane === "ingest"} onClick={() => setPane("ingest")} Icon={Rss} label="外部源" />

        <div className="wikiNavGroupTitle">配置</div>
        <NavBtn active={pane === "schema"} onClick={() => setPane("schema")} Icon={BookOpen} label="行业 Schema" />
        <NavBtn active={pane === "sysconfig"} onClick={() => setPane("sysconfig")} Icon={ShieldCheck} label="系统配置" />

        <button
          type="button"
          className="wikiNavGroupTitle wikiNavGroupToggle"
          onClick={() => setAdvancedOpen((v) => !v)}
        >
          {advancedOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />} 高级
        </button>
        {advancedOpen ? (
          <>
            <NavBtn active={pane === "observability"} onClick={() => setPane("observability")} Icon={Activity} label="诊断仪表" />
            <NavBtn active={pane === "tryRecall"} onClick={() => setPane("tryRecall")} Icon={Search} label="试召诊断" />
            <NavBtn active={pane === "metrics"} onClick={() => setPane("metrics")} Icon={Activity} label="指标总览" />
            <NavBtn active={pane === "memory"} onClick={() => setPane("memory")} Icon={BrainCircuit} label="运营记忆" />
            <NavBtn active={pane === "graph"} onClick={() => setPane("graph")} Icon={Network} label="关系图谱" />
          </>
        ) : null}
      </div>
      <div className="wikiModePane wikiModePane--main">
        {pane === "cockpit" && (
          <CockpitView onOpenReview={onOpenReview} onOpenAutoVerify={onOpenAutoVerify} />
        )}
        {pane === "documents" && <DocumentsView />}
        {pane === "import" && <ImportWizard />}
        {pane === "ingest" && <IngestSourcesView />}
        {pane === "schema" && <DomainSchemaTab />}
        {pane === "sysconfig" && <AdminGovernanceView />}
        {pane === "observability" && <ObservabilityDashboard />}
        {pane === "tryRecall" && <TryRecallView />}
        {pane === "metrics" && <MetricsTab />}
        {pane === "memory" && <MemoryDrawer />}
        {pane === "graph" && <ChunkGraphView />}
      </div>
    </div>
  );
}

function NavBtn({
  active, onClick, Icon, label,
}: {
  active: boolean;
  onClick: () => void;
  Icon: LucideIcon;
  label: string;
}) {
  return (
    <button
      type="button"
      className={active ? "wikiStewardNavBtn active" : "wikiStewardNavBtn"}
      onClick={onClick}
    >
      <Icon size={14} /> {label}
    </button>
  );
}

export default function KnowledgeFeature() {
  return (
    <ConfirmProvider>
      <ToastProvider>
        <FormDialogProvider>
          <KnowledgeWikiView />
        </FormDialogProvider>
      </ToastProvider>
    </ConfirmProvider>
  );
}
