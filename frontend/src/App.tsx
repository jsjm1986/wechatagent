import {
  Activity,
  AlertTriangle,
  Archive,
  ArrowRight,
  Bot,
  BookOpen,
  BrainCircuit,
  Calendar,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock3,
  Compass,
  Copy,
  Eye,
  EyeOff,
  FileBox,
  FileText,
  FlaskConical,
  GitMerge,
  History,
  Inbox,
  LibraryBig,
  LayoutDashboard,
  Link2,
  Loader2,
  Map as MapIcon,
  MessageSquareText,
  Network,
  Package,
  Plus,
  RefreshCw,
  Rss,
  Scissors,
  Search,
  SendHorizonal,
  Settings2,
  ShieldCheck,
  Sparkles,
  SquarePen,
  Trash2,
  Undo2,
  UploadCloud,
  UserRoundCheck,
  User2,
  UsersRound,
  Wrench,
  Workflow,
  X
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Fragment, FormEvent, useCallback, useEffect, useMemo, useRef, useState, KeyboardEvent, ClipboardEvent } from "react";
import type * as React from "react";

import { EvolutionCenterTab } from "./EvolutionCenterTab";

import type {
  AgentStatus,
  Channel,
  ContactTab,
  SmartOpsTab,
  TraditionalOpsTab,
  UserOpsMode,
  OpsTab,
  Account,
  AgentProfile,
  Contact,
  ContactCommitment,
  Message,
  EventItem,
  TaskItem,
} from "./types";

type ContentAsset = {
  id: string;
  kind: string;
  title: string;
  body?: string;
  url?: string;
  mediaId?: string;
  usageScene?: string;
};

type AgentSoul = {
  id: string;
  agentKind: string;
  name: string;
  content: string;
  status: string;
  version: number;
};

type OperationPlaybook = {
  id: string;
  accountId: string;
  name: string;
  description?: string;
  methodPrompt: string;
  profileMethod?: string;
  tagMethod?: string;
  stageMethod?: string;
  intentMethod?: string;
  followUpMethod?: string;
  replyStyle?: string;
  forbiddenRules?: string;
  successCriteria?: string;
  createdBy: string;
  isDefault: boolean;
  version: number;
  updatedAt?: string;
};

type PlaybookDraft = {
  name: string;
  description: string;
  methodPrompt: string;
  profileMethod: string;
  tagMethod: string;
  stageMethod: string;
  intentMethod: string;
  followUpMethod: string;
  replyStyle: string;
  forbiddenRules: string;
  successCriteria: string;
  isDefault: boolean;
};

type OperatingMemory = {
  id: string;
  userUnderstanding: Record<string, unknown>;
  relationshipState: Record<string, unknown>;
  productFit: Record<string, unknown>;
  nextAction: Record<string, unknown>;
  memoryCard?: Record<string, unknown>;
  memoryCardVersion?: number;
  memoryCardUpdatedAt?: string;
  contextPack?: Record<string, unknown>;
  contextPackVersion?: number;
  contextPackUpdatedAt?: string;
  updatedAt?: string;
};

type LlmUsageItem = {
  id: string;
  promptKey: string;
  model: string;
  status: string;
  latencyMs: number;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  promptCacheHitTokens: number;
  promptCacheMissTokens: number;
  error?: string;
  createdAt?: string;
};

type LlmUsageResponse = {
  summary: {
    totalCalls: number;
    totalTokens: number;
    promptCacheHitTokens: number;
    promptCacheMissTokens: number;
    promptCacheHitRate: number;
  };
  items: LlmUsageItem[];
};

type MemoryCandidateItem = {
  id: string;
  runId?: string;
  source: string;
  candidates: Record<string, unknown>[];
  memoryWriteScore: number;
  status: string;
  reason?: string;
  createdAt?: string;
  updatedAt?: string;
};

type OperatingMemoryDraft = {
  identity: string;
  businessContext: string;
  jobsToBeDone: string;
  painPoints: string;
  motivations: string;
  decisionStyle: string;
  communicationPreference: string;
  sensitivePoints: string;
  trustLevel: string;
  temperature: string;
  lastEmotion: string;
  relationshipGoal: string;
  doNotDo: string;
  interestedProducts: string;
  fitReason: string;
  objections: string;
  riskPoints: string;
  unknowns: string;
  nextGoal: string;
  recommendedMove: string;
  avoid: string;
  timing: string;
  reason: string;
};


type PromptTemplate = {
  id: string;
  promptKey: string;
  agentKind: string;
  layer: string;
  title: string;
  description?: string;
  content: string;
  status: string;
  version: number;
  promptPackVersion: string;
  createdBy: string;
  updatedAt?: string;
};

type PromptTemplateDraft = {
  promptKey: string;
  agentKind: string;
  layer: string;
  title: string;
  description: string;
  content: string;
};

type DomainKey = "user_operations" | "group_operations" | "moment_operations";

type OperationDomainConfig = {
  id: string;
  domain: DomainKey;
  name: string;
  goal: string;
  methodology: string;
  workflow: string;
  toolPolicy: string;
  automationPolicy: string;
  reviewPolicy: string;
  runtimeParameters: Record<string, unknown>;
  stateMachine: Record<string, unknown>;
  status: string;
  updatedAt?: string;
  // Phase E / E5-T1：active_versions 灰度字段。后端在 m015 之后保证非空。
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  seededBy?: string | null;
};

// Phase E / E5-T1：operation_state_policies / system_taxonomies 同款灰度元数据，
// 抽出公共 type 给三个 admin 面板复用。
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

// Phase D / D5：跨用户教训聚合（lessons_learned）。
// 后端 src/knowledge_wiki/lessons_learned.rs 周期 upsert，admin 只读列表。
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

type OperationDomainDraft = {
  name: string;
  goal: string;
  methodology: string;
  workflow: string;
  toolPolicy: string;
  automationPolicy: string;
  reviewPolicy: string;
  runtimeParameters: string;
  stateMachine: string;
};

type OperationStateDraft = {
  raw: Record<string, unknown>;
  key: string;
  name: string;
  goal: string;
  allowedActions: string;
  allowedFrom: string;
  allowFromAny: boolean;
  advanceSignals: string;
  cooldownSignals: string;
  riskRules: string;
  successCriteria: string;
};

type DecisionReview = {
  id: string;
  contactWxid?: string;
  replyText?: string;
  approved: boolean;
  scores: Record<string, number>;
  risks: string[];
  reviewSummary?: string;
  operationState?: string;
  nextBestAction?: Record<string, unknown>;
  sendGatewayResult?: Record<string, unknown>;
  outcomeStatus?: string;
  status: string;
  createdAt?: string;
};

type OperationHealthItem = {
  key: string;
  label: string;
  score: number;
  tone: "good" | "warn" | "danger";
  detail: string;
};

type OperationHealth = {
  scores: Record<string, number>;
  items: OperationHealthItem[];
};

type UserOperationGuidePreview = {
  id: string;
  accountId: string;
  contactId: string;
  contactWxid: string;
  instruction: string;
  mode: string;
  status: string;
  summary: string;
  impactScope: string;
  scopeReason: string;
  readableChanges: string[];
  healthScores: Record<string, unknown>;
  suggestedChanges: Record<string, unknown>;
  riskWarnings: string[];
  createdAt?: string;
  updatedAt?: string;
};

type SimulationTurn = {
  turn: number;
  inboundText: string;
  shouldReply: boolean;
  replyText: string;
  status: string;
  decision: Record<string, unknown>;
  review: Record<string, unknown>;
  gatewayResult: Record<string, unknown>;
  knowledgeRoute: Record<string, unknown>;
  contextPack?: Record<string, unknown>;
  memoryPreview: Record<string, unknown>;
  stateTransition: Record<string, unknown>;
};

type CommandToolCall = {
  id: string;
  toolName: string;
  arguments?: Record<string, unknown>;
  status: string;
  response?: Record<string, unknown>;
  error?: string;
};

type CommandResult = {
  id: string;
  status: string;
  summary: string;
  toolCalls: CommandToolCall[];
};

/**
 * 后端 `AppError::LlmUnavailable` 的前端镜像错误。
 *
 * 后端在 LLM 调用经过完整重试（默认 5 次 + 指数退避 + jitter + 尊重 Retry-After）
 * 后仍失败时，会返回 HTTP 503 + `{"error":"llm_unavailable", "kind", "retryCount", "detail", "hint"}`。
 * 前端把它从 raw fetch 错误中解出来作为 `LlmUnavailableError`，让所有调 LLM 的面板
 * （AiRepairPanel / KnowledgeChatPanel / 其它）都能按 `kind` 渲染一致的中文文案 +
 * 「AI 重试」按钮，而不是把 reqwest 原文 "error sending request for url..." 糊到 UI。
 */
export class LlmUnavailableError extends Error {
  kind: string;
  retryCount: number;
  detail: string;
  hint: string;
  constructor(payload: { kind: string; retryCount: number; detail: string; hint: string }) {
    super(payload.hint || payload.detail || "LLM 暂不可用");
    this.name = "LlmUnavailableError";
    this.kind = payload.kind;
    this.retryCount = payload.retryCount;
    this.detail = payload.detail;
    this.hint = payload.hint;
  }
}

async function parseApiError(response: Response): Promise<Error> {
  const text = await response.text();
  try {
    const json = JSON.parse(text) as Record<string, unknown>;
    if (json && json.error === "llm_unavailable") {
      return new LlmUnavailableError({
        kind: typeof json.kind === "string" ? json.kind : "unknown",
        retryCount: typeof json.retryCount === "number" ? json.retryCount : 0,
        detail: typeof json.detail === "string" ? json.detail : text,
        hint:
          typeof json.hint === "string"
            ? json.hint
            : "调用 LLM 失败，请稍后再试。"
      });
    }
    if (json && typeof json.error === "string") {
      return new Error(json.error);
    }
  } catch {
    /* 不是 JSON：可能是 SPA fallback 返回的 HTML / Axum 默认 405 文本，
       不能把整段 body 当成错误信息渲染（screenshot 里就是因为这个把
       <!doctype html> 整个塞进 UI）。统一脱壳成 HTTP 状态码。 */
  }
  const ct = response.headers.get("content-type") ?? "";
  if (ct.toLowerCase().includes("text/html")) {
    return new Error(`HTTP ${response.status}（服务端未返回 JSON，可能是后端尚未编译该接口）`);
  }
  const trimmed = text.trim();
  if (!trimmed || /^<!doctype|^<html/i.test(trimmed)) {
    return new Error(`HTTP ${response.status}`);
  }
  // 非 JSON 但是是较短的纯文本（如 axum 的 "Method Not Allowed"），保留前 120 字
  const safe = trimmed.length > 120 ? `${trimmed.slice(0, 120)}…` : trimmed;
  return new Error(`HTTP ${response.status}：${safe}`);
}

const api = {
  async get<T>(url: string): Promise<T> {
    const response = await fetch(url);
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  async post<T>(url: string, body?: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined
    });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  async put<T>(url: string, body: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
  async delete<T>(url: string): Promise<T> {
    const response = await fetch(url, { method: "DELETE" });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  }
};

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


function stripHtml(input: string | undefined | null): string {
  if (!input) return "";
  return input
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * 统一渲染 LLM 调用失败的提示横幅，给所有调 LLM 的面板复用。
 *
 * - `error` 是 `LlmUnavailableError` → 显示 kind 标签 + hint + 重试次数 + 「AI 重试」
 * - `error` 是普通 `Error` → 显示 message + 「AI 重试」（走通用错误路径）
 */
function LlmErrorBanner(props: {
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

const channels: Array<{ id: Channel; label: string; caption: string; icon: LucideIcon }> = [
  { id: "command", label: "AI 总控", caption: "Command Center", icon: BrainCircuit },
  { id: "overview", label: "工作台", caption: "运行态势", icon: LayoutDashboard },
  { id: "userOps", label: "用户运营", caption: "私聊关系运营", icon: UserRoundCheck },
  { id: "groupOps", label: "微信群运营", caption: "群分析与线索", icon: UsersRound },
  { id: "momentOps", label: "朋友圈运营", caption: "内容计划", icon: Sparkles },
  { id: "content", label: "内容资产", caption: "素材知识", icon: FileText },
  { id: "systemStrategy", label: "系统策略", caption: "全局与总控", icon: Settings2 },
  { id: "llmProviders", label: "AI 模型配置", caption: "LLM Providers", icon: Bot },
  { id: "operations", label: "任务日志", caption: "执行审计", icon: Activity },
  // W6 / Task 7.2：自治回路监控提到顶级频道（原 QualityCenterView 子 Tab 已下沉到此）
  { id: "autonomy", label: "自治回路监控", caption: "Autonomy Loop", icon: ShieldCheck },
  // M4 W4 / Task 5.8：演化中心（experiments / proposals / release / rollback）
  { id: "evolution", label: "演化中心", caption: "Self Evolution", icon: ShieldCheck },
  // 波 B4-B6：运营成效中心（outcome metrics / auto-verify / formula-adherence / 标记词编辑）
  { id: "quality", label: "运营成效", caption: "指标与质量", icon: Workflow },
  // knowledge-wiki Phase G：Wiki 管理（domain_schemas / gap_signals / chunk_revisions）
  { id: "knowledgeWiki", label: "Wiki 管理", caption: "schema / 信号 / 历史", icon: FileBox }
];

const USER_RUNTIME_PARAMETER_FIELDS: Array<{
  key: string;
  label: string;
  detail: string;
  kind: "number" | "boolean";
  defaultValue: number | boolean;
}> = [
  { key: "recentMessageLimit", label: "上下文消息数", detail: "每次决策读取的最近消息数量", kind: "number", defaultValue: 12 },
  { key: "minReplyIntervalSeconds", label: "最小回复间隔", detail: "避免短时间连续自动回复，单位秒", kind: "number", defaultValue: 20 },
  { key: "maxDailyTouches", label: "每日触达上限", detail: "单个好友每天最多主动触达次数", kind: "number", defaultValue: 3 },
  { key: "maxPendingFollowUps", label: "待跟进上限", detail: "同一好友最多保留的未执行跟进任务", kind: "number", defaultValue: 3 },
  { key: "followUpExpiresHours", label: "跟进过期时间", detail: "超过时间未执行则自动失效，单位小时", kind: "number", defaultValue: 48 },
  { key: "cooldownAfterNoReplyHours", label: "未回复冷却", detail: "用户无回应后的默认冷却时间，单位小时", kind: "number", defaultValue: 24 },
  { key: "hallucinationBlockAt", label: "幻觉风险拦截线", detail: "幻觉风险达到该分值则禁止发送", kind: "number", defaultValue: 6 },
  { key: "knowledgeGroundingBlockBelow", label: "知识落地拦截线", detail: "低于该分值则禁止发送涉及产品/价格/政策的内容", kind: "number", defaultValue: 7 },
  { key: "humanLikeRewriteBelow", label: "真人感重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 6 },
  { key: "emotionalValueRewriteBelow", label: "情绪价值重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 5 },
  { key: "operationStateConfidenceFullReviewBelow", label: "状态置信 Review 线", detail: "低于该分值强制完整 Review", kind: "number", defaultValue: 4 },
  { key: "runTokenBudget", label: "单次 Token 预算", detail: "单次用户运营运行的最大 token", kind: "number", defaultValue: 30000 },
  { key: "runMaxLlmCalls", label: "单次模型调用上限", detail: "单次用户运营最多 LLM 调用次数", kind: "number", defaultValue: 6 },
  { key: "simulationTokenBudget", label: "模拟评测预算", detail: "单次模拟/评测可用 token", kind: "number", defaultValue: 60000 },
  { key: "reactionTokenBudget", label: "反应分析预算", detail: "用户回应分析单次最多 token", kind: "number", defaultValue: 8000 },
  { key: "reactionMaxLlmCalls", label: "反应分析调用上限", detail: "用户回应分析最多 LLM 调用次数", kind: "number", defaultValue: 2 }
];

export function App() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [selected, setSelected] = useState<Contact | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [events, setEvents] = useState<EventItem[]>([]);
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [llmUsage, setLlmUsage] = useState<LlmUsageResponse | null>(null);
  const [assets, setAssets] = useState<ContentAsset[]>([]);
  const [souls, setSouls] = useState<AgentSoul[]>([]);
  const [promptTemplates, setPromptTemplates] = useState<PromptTemplate[]>([]);
  const [operationDomains, setOperationDomains] = useState<OperationDomainConfig[]>([]);
  const [domainDrafts, setDomainDrafts] = useState<Record<string, OperationDomainDraft>>({});
  const [playbooks, setPlaybooks] = useState<OperationPlaybook[]>([]);
  const [decisionReviews, setDecisionReviews] = useState<DecisionReview[]>([]);
  const [operatingMemory, setOperatingMemory] = useState<OperatingMemory | null>(null);
  const [memoryCandidates, setMemoryCandidates] = useState<MemoryCandidateItem[]>([]);
  const [memoryDraft, setMemoryDraft] = useState<OperatingMemoryDraft>(emptyMemoryDraft());
  const [operationHealth, setOperationHealth] = useState<OperationHealth | null>(null);
  const [guideInstruction, setGuideInstruction] = useState("");
  const [guidePreview, setGuidePreview] = useState<UserOperationGuidePreview | null>(null);
  const [guideBusy, setGuideBusy] = useState(false);
  const [simulationInput, setSimulationInput] = useState("我最近在看 AI 运营，想了解你们能做到什么程度。\n我们现在几百个客户，销售经常跟丢，但我不想做机器人群发。\n如果客户三天没回，你们会一直追吗？");
  const [simulationBusy, setSimulationBusy] = useState(false);
  const [simulationTurns, setSimulationTurns] = useState<SimulationTurn[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState(() => localStorage.getItem("wechatagent.accountId") || "");
  const [query, setQuery] = useState("");
  const [importQuery, setImportQuery] = useState("");
  const [profileNote, setProfileNote] = useState("");
  const [customAgentInstructions, setCustomAgentInstructions] = useState("");
  const [selectedPlaybookId, setSelectedPlaybookId] = useState("");
  const [assetDraft, setAssetDraft] = useState({ kind: "text", title: "", body: "", url: "", mediaId: "", usageScene: "" });
  const [soulDraft, setSoulDraft] = useState({ agentKind: "user", name: "", content: "" });
  const [editingSoulId, setEditingSoulId] = useState("");
  const [promptDraft, setPromptDraft] = useState<PromptTemplateDraft>(emptyPromptTemplateDraft());
  const [editingPromptId, setEditingPromptId] = useState("");
  const [playbookDraft, setPlaybookDraft] = useState<PlaybookDraft>(emptyPlaybookDraft());
  const [editingPlaybookId, setEditingPlaybookId] = useState("");
  const [generatePlaybookText, setGeneratePlaybookText] = useState("我们运营 AI 软件定制客户，希望像真实顾问朋友一样长期理解用户，在信任不受损的前提下自然推进需求沟通、方案确认和成交。");
  const [optimizePlaybookText, setOptimizePlaybookText] = useState("让方法更像真人朋友，减少营销感；对高意向用户更自然地主动推进；对沉默客户降低打扰频率。");
  const [commandDraft, setCommandDraft] = useState("把 AI应用开发 加入 Agent 运营列表，并生成一份克制、专业的运营备注");
  const [commandResult, setCommandResult] = useState<CommandResult | null>(null);
  const [commandDryRun, setCommandDryRun] = useState<boolean>(true);
  const [activeChannel, setActiveChannel] = useState<Channel>("command");
  const [contactTab, setContactTab] = useState<ContactTab>("all");
  const [userOpsMode, setUserOpsMode] = useState<UserOpsMode>("smart");
  const [smartOpsTab, setSmartOpsTab] = useState<SmartOpsTab>("cockpit");
  const [traditionalOpsTab, setTraditionalOpsTab] = useState<TraditionalOpsTab>("playbooks");
  const [opsTab, setOpsTab] = useState<OpsTab>("tasks");
  const [busy, setBusy] = useState(false);
  const [commandBusy, setCommandBusy] = useState(false);
  const [error, setError] = useState("");

  // P1-4：登录态后挂一次 WebSocket，进程内所有 ChunkInspectorPane / 锁徽章共享。
  useChunkEventStream();

  const managedCount = useMemo(
    () => contacts.filter((contact) => contact.agentStatus === "managed").length,
    [contacts]
  );
  const normalCount = contacts.length - managedCount;
  const onlineCount = accounts.filter((account) => account.online).length;
  const currentAccountId = accounts.some((account) => account.accountId === selectedAccountId)
    ? selectedAccountId
    : accounts[0]?.accountId || "";
  const currentAccount = accounts.find((account) => account.accountId === currentAccountId);
  const filteredContacts = useMemo(() => {
    if (contactTab === "managed") return contacts.filter((contact) => contact.agentStatus === "managed");
    if (contactTab === "normal") return contacts.filter((contact) => contact.agentStatus === "normal");
    return contacts;
  }, [contacts, contactTab]);
  const latestEvent = events[0];
  const pendingTasks = tasks.filter((task) => task.status === "pending").length;

  async function loadAll() {
    setError("");
    const accountParam = currentAccountId ? `accountId=${encodeURIComponent(currentAccountId)}` : "";
    const separator = query ? "&" : "";
    const [
      accountData,
      contactData,
      eventData,
      taskData,
      assetData,
      soulData,
      promptData,
      domainData,
      playbookData,
      llmUsageData,
      reviewData
    ] = await Promise.all([
      api.get<{ items: Account[] }>("/api/accounts"),
      api.get<{ items: Contact[] }>(
        `/api/contacts?${query ? `q=${encodeURIComponent(query)}${separator}` : ""}${accountParam}`
      ),
      api.get<{ items: EventItem[] }>(`/api/events${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: TaskItem[] }>(`/api/tasks${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: ContentAsset[] }>(`/api/content-assets${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: AgentSoul[] }>("/api/agent-souls"),
      api.get<{ items: PromptTemplate[] }>("/api/prompt-templates"),
      api.get<{ items: OperationDomainConfig[] }>("/api/operation-domains"),
      api.get<{ items: OperationPlaybook[] }>(`/api/operation-playbooks${accountParam ? `?${accountParam}` : ""}`),
      api.get<LlmUsageResponse>(`/api/llm-usage${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: DecisionReview[] }>(`/api/decision-reviews${accountParam ? `?${accountParam}` : ""}`)
    ]);
    setAccounts(accountData.items);
    setContacts(contactData.items);
    setEvents(eventData.items);
    setTasks(taskData.items);
    setAssets(assetData.items);
    setSouls(soulData.items);
    setPromptTemplates(promptData.items);
    setOperationDomains(domainData.items);
    setDomainDrafts(domainDraftsFromConfigs(domainData.items));
    setPlaybooks(playbookData.items);
    setLlmUsage(llmUsageData);
    setDecisionReviews(reviewData.items);
    if (selected) {
      const refreshed = contactData.items.find((item) => item.id === selected.id);
      hydrateSelected(refreshed ?? null, playbookData.items);
    }
  }

  function hydrateSelected(contact: Contact | null, knownPlaybooks = playbooks) {
    setSelected(contact);
    setProfileNote(contact?.humanProfileNote ?? "");
    setCustomAgentInstructions(contact?.customAgentInstructions ?? "");
    setSelectedPlaybookId(contact?.playbookId || knownPlaybooks.find((playbook) => playbook.isDefault)?.id || knownPlaybooks[0]?.id || "");
    setGuidePreview(null);
    setGuideInstruction("");
    if (!contact) {
      setOperatingMemory(null);
      setMemoryCandidates([]);
      setMemoryDraft(emptyMemoryDraft());
      setOperationHealth(null);
    }
  }

  async function loadMessages(contact: Contact) {
    hydrateSelected(contact);
    const [data, memoryData, candidateData, reviewData, healthData] = await Promise.all([
      api.get<{ items: Message[] }>(`/api/conversations/${contact.id}/messages`),
      api.get<{ item: OperatingMemory }>(`/api/contacts/${contact.id}/operating-memory`),
      api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${contact.id}/memory-candidates?limit=30`),
      api.get<{ items: DecisionReview[] }>(`/api/decision-reviews?accountId=${encodeURIComponent(contact.accountId || currentAccountId)}&contactId=${encodeURIComponent(contact.id)}`),
      api.get<OperationHealth>(`/api/contacts/${contact.id}/operation-health`)
    ]);
    setMessages(data.items.reverse());
    setOperatingMemory(memoryData.item);
    setMemoryCandidates(candidateData.items);
    setMemoryDraft(draftFromMemory(memoryData.item));
    setDecisionReviews(reviewData.items);
    setOperationHealth(healthData);
  }

  async function openContact(contact: Contact, channel: Channel = "userOps") {
    await loadMessages(contact);
    setActiveChannel(channel);
  }

  async function runMemoryConsolidation() {
    if (!selected) return;
    await run(async () => {
      const data = await api.post<{ item: OperatingMemory }>(`/api/contacts/${selected.id}/memory-consolidation/run`, {});
      setOperatingMemory(data.item);
      setMemoryDraft(draftFromMemory(data.item));
      const candidateData = await api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${selected.id}/memory-candidates?limit=30`);
      setMemoryCandidates(candidateData.items);
    });
  }

  async function syncAccounts() {
    await run(async () => {
      await api.post("/api/accounts/sync");
      await loadAll();
    });
  }

  async function importContacts(event: FormEvent) {
    event.preventDefault();
    if (!importQuery.trim()) return;
    await run(async () => {
      // 波 A3：拆成 search → import 两步。先用只读 /search 拿候选，
      // 再用 /import 真正写库；保留 search-import 的"一步到位"语义供
      // 老 UI 兼容，但默认走拆分流程，避免误以为搜索就会改库。
      const search = await api.post<{ items: unknown[] }>("/api/contacts/search", {
        query: importQuery,
        accountId: currentAccountId
      });
      const candidates = search.items || [];
      if (!candidates.length) {
        setImportQuery("");
        await loadAll();
        return;
      }
      const data = await api.post<{ items: Contact[] }>("/api/contacts/import", {
        accountId: currentAccountId,
        candidates
      });
      setImportQuery("");
      await loadAll();
      if (data.items[0]) await openContact(data.items[0], "userOps");
    });
  }

  async function enableAgent() {
    if (!selected || !profileNote.trim()) return;
    await run(async () => {
      const data = await api.post<{ item: Contact }>(`/api/contacts/${selected.id}/enable-agent`, {
        humanProfileNote: profileNote,
        playbookId: selectedPlaybookId || undefined
      });
      hydrateSelected(data.item);
      await loadAll();
    });
  }

  async function saveProfileNote() {
    if (!selected) return;
    await run(async () => {
      const data = await api.put<{ item: Contact }>(`/api/contacts/${selected.id}/profile-note`, {
        humanProfileNote: profileNote
      });
      hydrateSelected(data.item);
      await loadAll();
    });
  }

  async function saveCustomAgentInstructions() {
    if (!selected) return;
    await run(async () => {
      const data = await api.put<{ item: Contact }>(
        `/api/contacts/${selected.id}/custom-agent-instructions`,
        { instructions: customAgentInstructions }
      );
      hydrateSelected(data.item);
      await loadAll();
    });
  }

  async function disableAgent() {
    if (!selected) return;
    await run(async () => {
      const data = await api.post<{ item: Contact }>(`/api/contacts/${selected.id}/disable-agent`);
      hydrateSelected(data.item);
      await loadAll();
    });
  }

  async function analyzeProfile() {
    if (!selected) return;
    await run(async () => {
      const data = await api.post<{ item: Contact }>(`/api/contacts/${selected.id}/analyze-profile`);
      hydrateSelected(data.item);
      await loadAll();
    });
  }

  async function previewGuideInstruction(instruction = guideInstruction) {
    if (!selected || !currentAccountId || !instruction.trim()) return;
    setGuideBusy(true);
    setError("");
    try {
      const data = await api.post<{ item: UserOperationGuidePreview }>("/api/user-operations/guide/preview", {
        accountId: currentAccountId,
        contactId: selected.id,
        instruction,
        mode: userOpsMode
      });
      setGuideInstruction(instruction);
      setGuidePreview(data.item);
      if (data.item.healthScores) {
        setOperationHealth(healthFromScores(data.item.healthScores));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setGuideBusy(false);
    }
  }

  async function applyGuidePreview() {
    if (!selected || !guidePreview) return;
    setGuideBusy(true);
    setError("");
    try {
      const data = await api.post<{ item: { contact: Contact; operatingMemory: OperatingMemory; health: OperationHealth } }>(
        "/api/user-operations/guide/apply",
        { previewId: guidePreview.id }
      );
      hydrateSelected(data.item.contact);
      setOperatingMemory(data.item.operatingMemory);
      setMemoryDraft(draftFromMemory(data.item.operatingMemory));
      setOperationHealth(data.item.health);
      setGuidePreview(null);
      await loadAll();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setGuideBusy(false);
    }
  }

  async function runDialogueSimulation() {
    if (!selected || !currentAccountId) return;
    const messages = simulationInput
      .split(/\n+/)
      .map((item) => item.trim())
      .filter(Boolean);
    if (!messages.length) return;
    setSimulationBusy(true);
    setError("");
    try {
      const data = await api.post<{ items: SimulationTurn[]; runMode: string; applied: boolean }>(
        "/api/user-operations/simulations/dialogue",
        {
          accountId: currentAccountId,
          contactId: selected.id,
          messages,
          applyMemory: false
        }
      );
      setSimulationTurns(data.items || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSimulationBusy(false);
    }
  }

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function runManagementCommand() {
    if (!currentAccountId || !commandDraft.trim()) return;
    setCommandBusy(true);
    setError("");
    try {
      // 波 B1：session 创建时把当前 dry-run 状态作为默认；后端写到 session.dry_run
      // 单条消息再把 dryRun 单次覆盖（不冲突），所以"用户改 toggle"立即生效。
      const session = await api.post<{ id: string }>("/api/management-agent/sessions", {
        accountId: currentAccountId,
        title: commandDraft.slice(0, 40),
        dryRun: commandDryRun
      });
      const data = await api.post<{ command: CommandResult }>(
        `/api/management-agent/sessions/${session.id}/messages`,
        {
          accountId: currentAccountId,
          content: commandDraft,
          dryRun: commandDryRun
        }
      );
      setCommandResult(data.command);
      await loadAll();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setCommandBusy(false);
    }
  }

  async function createAsset(event: FormEvent) {
    event.preventDefault();
    if (!assetDraft.title.trim()) return;
    await run(async () => {
      await api.post("/api/content-assets", {
        accountId: currentAccountId || undefined,
        kind: assetDraft.kind,
        title: assetDraft.title,
        body: assetDraft.body || undefined,
        url: assetDraft.url || undefined,
        mediaId: assetDraft.mediaId || undefined,
        usageScene: assetDraft.usageScene || undefined
      });
      setAssetDraft({ kind: assetDraft.kind, title: "", body: "", url: "", mediaId: "", usageScene: "" });
      await loadAll();
    });
  }

  async function createSoul(event: FormEvent) {
    event.preventDefault();
    if (!soulDraft.name.trim() || !soulDraft.content.trim()) return;
    await run(async () => {
      await api.post("/api/agent-souls", soulDraft);
      setSoulDraft({ ...soulDraft, name: "", content: "" });
      await loadAll();
    });
  }

  async function saveSoul(event: FormEvent) {
    event.preventDefault();
    if (!editingSoulId || !soulDraft.name.trim() || !soulDraft.content.trim()) return;
    await run(async () => {
      await api.put(`/api/agent-souls/${editingSoulId}`, soulDraft);
      await loadAll();
    });
  }

  async function publishSoul(id: string) {
    await run(async () => {
      await api.post(`/api/agent-souls/${id}/publish`);
      await loadAll();
    });
  }

  async function createPromptTemplate(event: FormEvent) {
    event.preventDefault();
    if (!promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()) return;
    await run(async () => {
      await api.post("/api/prompt-templates", promptPayload(promptDraft));
      newPromptDraft();
      await loadAll();
    });
  }

  async function savePromptTemplate(event: FormEvent) {
    event.preventDefault();
    if (!editingPromptId || !promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()) return;
    await run(async () => {
      await api.put(`/api/prompt-templates/${editingPromptId}`, promptPayload(promptDraft));
      await loadAll();
    });
  }

  async function publishPromptTemplate(id: string) {
    await run(async () => {
      await api.post(`/api/prompt-templates/${id}/publish`);
      await loadAll();
    });
  }

  async function resetSystemPromptPack() {
    await run(async () => {
      await api.post("/api/prompt-templates/reset-system-pack");
      setEditingPromptId("");
      setPromptDraft(emptyPromptTemplateDraft());
      await loadAll();
    });
  }

  async function saveOperationDomain(domain: DomainKey) {
    const draft = domainDrafts[domain];
    if (!draft?.name.trim()) return;
    await run(async () => {
      await api.put(`/api/operation-domains/${domain}`, domainPayload(draft));
      await loadAll();
    });
  }

  async function resetOperationDomain(domain: DomainKey) {
    await run(async () => {
      await api.post(`/api/operation-domains/${domain}/reset`);
      await loadAll();
    });
  }

  async function createPlaybook(event: FormEvent) {
    event.preventDefault();
    if (!playbookDraft.name.trim() || !playbookDraft.methodPrompt.trim()) return;
    await run(async () => {
      await api.post("/api/operation-playbooks", {
        accountId: currentAccountId,
        ...playbookPayload(playbookDraft)
      });
      newPlaybookDraft();
      await loadAll();
    });
  }

  async function savePlaybook(event: FormEvent) {
    event.preventDefault();
    if (!editingPlaybookId || !playbookDraft.name.trim() || !playbookDraft.methodPrompt.trim()) return;
    await run(async () => {
      await api.put(`/api/operation-playbooks/${editingPlaybookId}`, {
        accountId: currentAccountId,
        ...playbookPayload(playbookDraft)
      });
      await loadAll();
    });
  }

  async function generatePlaybook(event: FormEvent) {
    event.preventDefault();
    if (!currentAccountId || !generatePlaybookText.trim()) return;
    await run(async () => {
      await api.post("/api/operation-playbooks/generate", {
        accountId: currentAccountId,
        description: generatePlaybookText
      });
      await loadAll();
    });
  }

  async function optimizePlaybook() {
    if (!editingPlaybookId || !optimizePlaybookText.trim()) return;
    await run(async () => {
      const data = await api.post<{ item: OperationPlaybook }>(`/api/operation-playbooks/${editingPlaybookId}/optimize`, {
        instruction: optimizePlaybookText
      });
      editPlaybook(data.item);
      await loadAll();
    });
  }

  async function setDefaultPlaybook(id: string) {
    await run(async () => {
      await api.post(`/api/operation-playbooks/${id}/set-default`);
      await loadAll();
    });
  }

  function editSoul(soul: AgentSoul) {
    setEditingSoulId(soul.id);
    setSoulDraft({
      agentKind: soul.agentKind,
      name: soul.name,
      content: soul.content
    });
  }

  function newSoulDraftFor(agentKind: string) {
    setEditingSoulId("");
    setSoulDraft({ agentKind, name: "", content: "" });
  }

  function editPromptTemplate(template: PromptTemplate) {
    setEditingPromptId(template.id);
    setPromptDraft({
      promptKey: template.promptKey,
      agentKind: template.agentKind,
      layer: template.layer,
      title: template.title,
      description: template.description ?? "",
      content: template.content
    });
  }

  function newPromptDraft() {
    setEditingPromptId("");
    setPromptDraft(emptyPromptTemplateDraft());
  }

  function newPromptDraftFor(agentKind: string) {
    setEditingPromptId("");
    setPromptDraft({ ...emptyPromptTemplateDraft(), agentKind });
  }

  function editPlaybook(playbook: OperationPlaybook) {
    setEditingPlaybookId(playbook.id);
    setPlaybookDraft({
      name: playbook.name,
      description: playbook.description ?? "",
      methodPrompt: playbook.methodPrompt,
      profileMethod: playbook.profileMethod ?? "",
      tagMethod: playbook.tagMethod ?? "",
      stageMethod: playbook.stageMethod ?? "",
      intentMethod: playbook.intentMethod ?? "",
      followUpMethod: playbook.followUpMethod ?? "",
      replyStyle: playbook.replyStyle ?? "",
      forbiddenRules: playbook.forbiddenRules ?? "",
      successCriteria: playbook.successCriteria ?? "",
      isDefault: playbook.isDefault
    });
  }

  function newPlaybookDraft() {
    setEditingPlaybookId("");
    setPlaybookDraft(emptyPlaybookDraft());
  }


  function changeAccount(accountId: string) {
    setSelectedAccountId(accountId);
    localStorage.setItem("wechatagent.accountId", accountId);
    setSelected(null);
    setMessages([]);
    setSelectedPlaybookId("");
    setOperatingMemory(null);
    setMemoryDraft(emptyMemoryDraft());
  }

  // 避免"首屏闪一下全量、再跳到账号过滤集"的双拉：accounts 列表还没回来时，
  // currentAccountId 必然为空串，此时若直接 loadAll() 会先拉一份 workspace 级
  // 全量数据写到 state，等 setAccounts 回填后第二轮 effect 才带 accountId 重拉，
  // 表现就是知识库文档树/可答性横幅在 200~600ms 内跳变。
  // 解决方案：第一拉走"无账号"分支拿到 accounts 列表（and only that），剩余面板
  // 数据等 currentAccountId 收敛到真实账号 ID 后再统一加载。
  const accountsBootstrapRef = useRef(false);
  useEffect(() => {
    if (accounts.length === 0) {
      if (accountsBootstrapRef.current) return;
      accountsBootstrapRef.current = true;
      void api
        .get<{ items: Account[] }>("/api/accounts")
        .then((data) => setAccounts(data.items))
        .catch((err) => setError(err instanceof Error ? err.message : String(err)));
      return;
    }
    void loadAll().catch((err) => setError(err instanceof Error ? err.message : String(err)));
  }, [currentAccountId, accounts.length]);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brandMark">
            <Bot size={18} />
          </div>
          <div>
            <strong>WechatAgent</strong>
            <span>Enterprise AI Ops</span>
          </div>
        </div>

        <div className="systemCard">
          <div className="signalLine">
            <span className="liveDot" />
            <strong>Agent Fabric Online</strong>
          </div>
          <div className="systemMetrics">
            <span>{managedCount} Managed</span>
            <span>{onlineCount}/{accounts.length} Online</span>
          </div>
          <label className="accountPicker">
            <span>当前账号</span>
            <select value={currentAccountId} onChange={(event) => changeAccount(event.target.value)}>
              {accounts.map((account) => (
                <option key={account.id} value={account.accountId}>
                  {account.alias || account.displayName || account.accountId}
                </option>
              ))}
            </select>
          </label>
        </div>

        <nav className="channelNav" aria-label="Product channels">
          {channels.map((channel) => {
            const Icon = channel.icon;
            return (
              <button
                key={channel.id}
                className={activeChannel === channel.id ? "channel active" : "channel"}
                onClick={() => setActiveChannel(channel.id)}
              >
                <Icon size={17} />
                <span>
                  <strong>{channel.label}</strong>
                  <small>{channel.caption}</small>
                </span>
              </button>
            );
          })}
        </nav>
      </aside>

      <main>
        <header className="topline">
          <div>
            <p>{channelEyebrow(activeChannel)}</p>
            <h1>{channelTitle(activeChannel)}</h1>
            <span>{channelSubtitle(activeChannel)}</span>
          </div>
          <div className="actions">
            <button onClick={() => void syncAccounts()} disabled={busy}>
              <RefreshCw size={16} />
              同步账号
            </button>
            <button className="secondary" onClick={() => void loadAll()} disabled={busy}>
              <RefreshCw size={16} />
              刷新
            </button>
          </div>
        </header>

        {error && <div className="error">{error}</div>}

        {activeChannel === "command" && (
          <CommandCenterView
            accounts={accounts}
            assets={assets}
            commandDraft={commandDraft}
            commandBusy={commandBusy}
            commandResult={commandResult}
            commandDryRun={commandDryRun}
            setCommandDryRun={setCommandDryRun}
            currentAccount={currentAccount}
            managedCount={managedCount}
            onlineCount={onlineCount}
            pendingTasks={pendingTasks}
            souls={souls}
            onRunCommand={() => void runManagementCommand()}
            setCommandDraft={setCommandDraft}
          />
        )}

        {activeChannel === "overview" && (
          <OverviewView
            contacts={contacts}
            managedCount={managedCount}
            normalCount={normalCount}
            onlineCount={onlineCount}
            pendingTasks={pendingTasks}
            latestEvent={latestEvent}
            onOpenChannel={setActiveChannel}
          />
        )}

        {activeChannel === "userOps" && (
          <section className="userOpsWorkspace">
            <UserOpsModeHeader mode={userOpsMode} onMode={setUserOpsMode} />

            {userOpsMode === "smart" && (
              <section className="userCockpitGrid">
                <ContactsView
                  busy={busy}
                  contactTab={contactTab}
                  contacts={filteredContacts}
                  importQuery={importQuery}
                  query={query}
                  totalCount={contacts.length}
                  managedCount={managedCount}
                  normalCount={normalCount}
                  selected={selected}
                  onContactTab={setContactTab}
                  onImport={importContacts}
                  onImportQuery={setImportQuery}
                  onLoadAll={() => void loadAll()}
                  onOpenContact={(contact) => void openContact(contact, "userOps")}
                  onQuery={setQuery}
                />
                <UserOperationCockpit
                  activeTab={smartOpsTab}
                  busy={busy}
                  decisionReviews={decisionReviews}
                  guideBusy={guideBusy}
                  guideInstruction={guideInstruction}
                  guidePreview={guidePreview}
                  health={operationHealth}
                  memoryCandidates={memoryCandidates}
                  memoryDraft={memoryDraft}
                  messages={messages}
                  operatingMemory={operatingMemory}
                  playbooks={playbooks}
                  profileNote={profileNote}
                  customAgentInstructions={customAgentInstructions}
                  selected={selected}
                  selectedPlaybookId={selectedPlaybookId}
                  simulationBusy={simulationBusy}
                  simulationInput={simulationInput}
                  simulationTurns={simulationTurns}
                  onAnalyzeProfile={() => void analyzeProfile()}
                  onApplyGuidePreview={() => void applyGuidePreview()}
                  onDisableAgent={() => void disableAgent()}
                  onEnableAgent={() => void enableAgent()}
                  onGuideInstruction={setGuideInstruction}
                  onPreviewGuide={(instruction) => void previewGuideInstruction(instruction)}
                  onProfileNote={setProfileNote}
                  onCustomAgentInstructions={setCustomAgentInstructions}
                  onRunMemoryConsolidation={() => void runMemoryConsolidation()}
                  onRunSimulation={() => void runDialogueSimulation()}
                  onSaveProfileNote={() => void saveProfileNote()}
                  onSaveCustomAgentInstructions={() => void saveCustomAgentInstructions()}
                  onSelectedPlaybook={setSelectedPlaybookId}
                  onSimulationInput={setSimulationInput}
                  onTab={setSmartOpsTab}
                />
              </section>
            )}

            {userOpsMode === "traditional" && (
              <>
                <TraditionalOpsTabs
                  active={traditionalOpsTab}
                  managedCount={managedCount}
                  pendingTasks={pendingTasks}
                  onChange={setTraditionalOpsTab}
                />

                {traditionalOpsTab === "playbooks" && (
                  <UserPlaybookPanel
                    busy={busy}
                    editingPlaybookId={editingPlaybookId}
                    generatePlaybookText={generatePlaybookText}
                    optimizePlaybookText={optimizePlaybookText}
                    playbookDraft={playbookDraft}
                    playbooks={playbooks}
                    onCreatePlaybook={createPlaybook}
                    onEditPlaybook={editPlaybook}
                    onGeneratePlaybook={generatePlaybook}
                    onGeneratePlaybookText={setGeneratePlaybookText}
                    onNewPlaybook={newPlaybookDraft}
                    onOptimizePlaybook={() => void optimizePlaybook()}
                    onOptimizePlaybookText={setOptimizePlaybookText}
                    onPlaybookDraft={setPlaybookDraft}
                    onSavePlaybook={savePlaybook}
                    onSetDefaultPlaybook={(id) => void setDefaultPlaybook(id)}
                  />
                )}

                {traditionalOpsTab === "prompts" && (
                  <DomainPromptPanel
                    agentKinds={["user"]}
                    busy={busy}
                    defaultAgentKind="user"
                    editingPromptId={editingPromptId}
                    editingSoulId={editingSoulId}
                    lockAgentKind
                    promptDraft={promptDraft}
                    promptTemplates={promptTemplates}
                    soulDraft={soulDraft}
                    souls={souls}
                    title="用户运营 Agent 提示词"
                    onCreatePromptTemplate={createPromptTemplate}
                    onCreateSoul={createSoul}
                    onEditPromptTemplate={editPromptTemplate}
                    onEditSoul={editSoul}
                    onNewPromptTemplate={() => newPromptDraftFor("user")}
                    onNewSoul={() => newSoulDraftFor("user")}
                    onPromptDraft={setPromptDraft}
                    onPublishPromptTemplate={(id) => void publishPromptTemplate(id)}
                    onPublishSoul={(id) => void publishSoul(id)}
                    onSavePromptTemplate={savePromptTemplate}
                    onSaveSoul={saveSoul}
                    onSoulDraft={setSoulDraft}
                  />
                )}

                {traditionalOpsTab === "settings" && (
                  <DomainConfigEditor
                    busy={busy}
                    config={operationDomainByKey(operationDomains, "user_operations")}
                    draft={domainDrafts.user_operations ?? emptyDomainDraft()}
                    mode="primary"
                    onDraft={(draft) => setDomainDrafts({ ...domainDrafts, user_operations: draft })}
                    onReset={() => void resetOperationDomain("user_operations")}
                    onSave={() => void saveOperationDomain("user_operations")}
                    onAfterVersionAction={() => loadAll()}
                  />
                )}

                {traditionalOpsTab === "audit" && (
                  <OperationsView decisionReviews={decisionReviews} events={events} llmUsage={llmUsage} opsTab={opsTab} tasks={tasks} onOpsTab={setOpsTab} />
                )}
              </>
            )}
          </section>
        )}

        {activeChannel === "content" && (
          <ContentAssetsView
            assetDraft={assetDraft}
            assets={assets}
            busy={busy}
            onAssetDraft={setAssetDraft}
            onCreateAsset={createAsset}
          />
        )}

        {activeChannel === "groupOps" && (
          <NextPhasePanel
            title="微信群运营"
            text="对应 Soul 与 Prompt 已存在但运行时未实现，可在系统策略页查看草稿模板。"
          />
        )}

        {activeChannel === "momentOps" && (
          <NextPhasePanel
            title="朋友圈运营"
            text="对应 Soul 与 Prompt 已存在但运行时未实现，可在系统策略页查看草稿模板。"
          />
        )}

        {activeChannel === "systemStrategy" && (
          <SystemStrategyView
            busy={busy}
            editingPromptId={editingPromptId}
            editingSoulId={editingSoulId}
            promptDraft={promptDraft}
            promptTemplates={promptTemplates}
            soulDraft={soulDraft}
            souls={souls}
            onCreatePromptTemplate={createPromptTemplate}
            onCreateSoul={createSoul}
            onEditPromptTemplate={editPromptTemplate}
            onEditSoul={editSoul}
            onNewPromptTemplate={() => newPromptDraftFor("management")}
            onNewSoul={() => newSoulDraftFor("management")}
            onPromptDraft={setPromptDraft}
            onPublishPromptTemplate={(id) => void publishPromptTemplate(id)}
            onPublishSoul={(id) => void publishSoul(id)}
            onResetPromptPack={() => void resetSystemPromptPack()}
            onSavePromptTemplate={savePromptTemplate}
            onSaveSoul={saveSoul}
            onSoulDraft={setSoulDraft}
          />
        )}

        {activeChannel === "operations" && (
          <OperationsView decisionReviews={decisionReviews} events={events} llmUsage={llmUsage} opsTab={opsTab} tasks={tasks} onOpsTab={setOpsTab} />
        )}

        {activeChannel === "autonomy" && (
          <AutonomyLoopView accountId={currentAccountId} />
        )}

        {activeChannel === "evolution" && (
          <EvolutionCenterView />
        )}

        {activeChannel === "quality" && (
          <QualityCenterView accountId={currentAccountId} />
        )}

        {activeChannel === "llmProviders" && (
          <LlmProvidersView />
        )}

        {activeChannel === "knowledgeWiki" && (
          <KnowledgeWikiView />
        )}
      </main>
    </div>
  );
}

function CommandCenterView({
  accounts,
  assets,
  commandDraft,
  commandBusy,
  commandResult,
  commandDryRun,
  setCommandDryRun,
  currentAccount,
  managedCount,
  onlineCount,
  onRunCommand,
  pendingTasks,
  souls,
  setCommandDraft
}: {
  accounts: Account[];
  assets: ContentAsset[];
  commandDraft: string;
  commandBusy: boolean;
  commandResult: CommandResult | null;
  commandDryRun: boolean;
  setCommandDryRun: (value: boolean) => void;
  currentAccount?: Account;
  managedCount: number;
  onlineCount: number;
  onRunCommand: () => void;
  pendingTasks: number;
  souls: AgentSoul[];
  setCommandDraft: (value: string) => void;
}) {
  const examples = ["把 xx 加入 Agent 运营", "发送 xx 给好友 xx", "查看今天失败任务"];

  return (
    <section className="commandLayout">
      <aside className="scopePanel">
        <div className="panelHead compact">
          <div>
            <span>Scope</span>
            <h2>操作范围</h2>
          </div>
          <ShieldCheck size={18} />
        </div>
        <div className="scopeStack">
          <StatusLine label="微信账号" value={`${onlineCount}/${accounts.length} 在线`} tone="good" />
          <StatusLine
            label="当前账号"
            value={currentAccount?.alias || currentAccount?.displayName || currentAccount?.accountId || "-"}
            tone={currentAccount?.mcpKeyConfigured ? "ai" : "warn"}
          />
          <StatusLine label="运营好友" value={`${managedCount} managed`} tone="ai" />
          <StatusLine label="待执行任务" value={`${pendingTasks} pending`} tone={pendingTasks ? "warn" : "neutral"} />
          <StatusLine label="内容资产" value={`${assets.length} assets`} tone="neutral" />
          <StatusLine label="Agent Soul" value={`${souls.length} versions`} tone="neutral" />
        </div>
        <div className="boundaryBox">
          <strong>执行边界</strong>
          <p>当前版本开放完整 MCP 工具目录给 Management Agent，所有调用通过后端账号凭证代理并写入审计日志。</p>
        </div>
      </aside>

      <section className="commandPanel">
        <div className="commandHeader">
          <BrainCircuit size={20} />
          <div>
            <strong>Management Agent</strong>
            <span>用自然语言管理好友、群、朋友圈和任务。</span>
          </div>
        </div>
        <label className="commandInput">
          <textarea value={commandDraft} onChange={(event) => setCommandDraft(event.target.value)} />
        </label>
        <div className="suggestionRow">
          {examples.map((item) => (
            <button key={item} className="chipButton" onClick={() => setCommandDraft(item)}>
              {item}
            </button>
          ))}
        </div>
        <div className="commandActions">
          <button onClick={onRunCommand} disabled={commandBusy || !commandDraft.trim()}>
            <Workflow size={16} />
            {commandBusy ? "执行中" : "执行指令"}
          </button>
          {/* 波 B1：dry-run toggle。打开后所有写库/发消息工具只回放 would_execute，
              不实际触达 MCP 或业务集合。Read 工具（搜索 / 知识库浏览）仍正常执行。 */}
          <label className="dryRunToggle" style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={commandDryRun}
              onChange={(event) => setCommandDryRun(event.target.checked)}
            />
            <span>Dry-run（不写业务库）</span>
          </label>
          <span className={`modeBadge ${commandDryRun ? "dryRun" : "live"}`}>
            {commandDryRun ? "🧪 演练模式" : "⚡ 真实执行"}
          </span>
          <span>LLM 生成工具计划，后端逐步调用 MCP 并记录结果</span>
        </div>
        {commandResult && (
          <div className={`commandResult ${commandResult.status}`}>
            <strong>{commandResult.status === "dry_run" ? "DRY-RUN 演练" : commandResult.status}</strong>
            <p>{commandResult.summary}</p>
          </div>
        )}
      </section>

      <aside className="planPanel">
        <div className="panelHead compact">
          <div>
            <span>Plan Preview</span>
            <h2>执行计划</h2>
          </div>
          <Workflow size={18} />
        </div>
        {commandResult?.toolCalls.length ? (
          <div className="planSteps">
            {commandResult.toolCalls.map((call) => (
              <PlanStep
                key={call.id || call.toolName}
                status={
                  call.status === "succeeded" || call.status === "dry_run" ? "ready" : "pending"
                }
                title={call.toolName}
                detail={commandCallDetail(call)}
              />
            ))}
          </div>
        ) : (
          <div className="planSteps">
            <PlanStep status="ready" title="加载工具目录" detail="从当前账号 MCP Server 获取完整工具列表" />
            <PlanStep status="pending" title="生成执行计划" detail="LLM 选择工具并输出结构化 JSON" />
            <PlanStep status="pending" title="调用 MCP 工具" detail="后端代理执行并记录日志" />
          </div>
        )}
      </aside>
    </section>
  );
}

function OverviewView({
  contacts,
  managedCount,
  normalCount,
  onlineCount,
  pendingTasks,
  latestEvent,
  onOpenChannel
}: {
  contacts: Contact[];
  managedCount: number;
  normalCount: number;
  onlineCount: number;
  pendingTasks: number;
  latestEvent?: EventItem;
  onOpenChannel: (channel: Channel) => void;
}) {
  return (
    <section className="overviewGrid">
      <MetricCard label="Managed Users" value={managedCount} detail="Agent 运营好友" onClick={() => onOpenChannel("userOps")} />
      <MetricCard label="Contact Base" value={contacts.length} detail={`${normalCount} 普通好友`} onClick={() => onOpenChannel("userOps")} />
      <MetricCard label="Account Online" value={onlineCount} detail="可用微信账号" onClick={() => onOpenChannel("overview")} />
      <MetricCard label="Pending Tasks" value={pendingTasks} detail="待执行任务" onClick={() => onOpenChannel("operations")} />

      <section className="widePanel">
        <div className="panelHead">
          <div>
            <span>Operating Model</span>
            <h2>AI 私域运营系统</h2>
          </div>
          <Sparkles size={18} />
        </div>
        <div className="principleGrid">
          <div>
            <strong>独立用户上下文</strong>
            <p>每个 managed 好友拥有运营备注、画像、记忆和跟进节奏，不使用统一批量话术。</p>
          </div>
          <div>
            <strong>双 Agent 架构</strong>
            <p>管理 Agent 负责后台操作，运营 Agent 负责好友、群和朋友圈的长期业务运营。</p>
          </div>
          <div>
            <strong>审计优先</strong>
            <p>回复、任务、策略、工具调用和失败事件进入日志，保证长期运行可复盘。</p>
          </div>
        </div>
      </section>

      <section className="sidePanel">
        <div className="panelHead">
          <div>
            <span>Last Event</span>
            <h2>最近事件</h2>
          </div>
          <Activity size={18} />
        </div>
        {latestEvent ? (
          <div className="eventPreview">
            <strong>{latestEvent.kind}</strong>
            <p>{latestEvent.summary}</p>
            <small>{formatTime(latestEvent.createdAt)}</small>
          </div>
        ) : (
          <EmptyInline text="暂无运营事件" />
        )}
      </section>
    </section>
  );
}

function UserOpsModeHeader({ mode, onMode }: { mode: UserOpsMode; onMode: (mode: UserOpsMode) => void }) {
  return (
    <section className="userModeHeader">
      <div>
        <span>用户运营驾驶舱</span>
        <h2>{mode === "smart" ? "智能模式" : "传统模式"}</h2>
        <p>{mode === "smart" ? "用自然语言指挥 Agent，查看当前运营判断、风险和修改预览；确认前不会改动数据。" : "用传统后台方式手动维护方法、提示词、运行策略和审计复盘。"}</p>
      </div>
      <div className="modeSwitch" role="tablist" aria-label="用户运营模式">
        <button className={mode === "smart" ? "active" : ""} onClick={() => onMode("smart")}>智能模式</button>
        <button className={mode === "traditional" ? "active" : ""} onClick={() => onMode("traditional")}>传统模式</button>
      </div>
    </section>
  );
}

function UserOperationCockpit({
  activeTab,
  busy,
  decisionReviews,
  guideBusy,
  guideInstruction,
  guidePreview,
  health,
  memoryCandidates,
  memoryDraft,
  messages,
  operatingMemory,
  playbooks,
  profileNote,
  customAgentInstructions,
  selected,
  selectedPlaybookId,
  simulationBusy,
  simulationInput,
  simulationTurns,
  onAnalyzeProfile,
  onApplyGuidePreview,
  onDisableAgent,
  onEnableAgent,
  onGuideInstruction,
  onPreviewGuide,
  onProfileNote,
  onCustomAgentInstructions,
  onRunMemoryConsolidation,
  onRunSimulation,
  onSaveProfileNote,
  onSaveCustomAgentInstructions,
  onSelectedPlaybook,
  onSimulationInput,
  onTab
}: {
  activeTab: SmartOpsTab;
  busy: boolean;
  decisionReviews: DecisionReview[];
  guideBusy: boolean;
  guideInstruction: string;
  guidePreview: UserOperationGuidePreview | null;
  health: OperationHealth | null;
  memoryCandidates: MemoryCandidateItem[];
  memoryDraft: OperatingMemoryDraft;
  messages: Message[];
  operatingMemory: OperatingMemory | null;
  playbooks: OperationPlaybook[];
  profileNote: string;
  customAgentInstructions: string;
  selected: Contact | null;
  selectedPlaybookId: string;
  simulationBusy: boolean;
  simulationInput: string;
  simulationTurns: SimulationTurn[];
  onAnalyzeProfile: () => void;
  onApplyGuidePreview: () => void;
  onDisableAgent: () => void;
  onEnableAgent: () => void;
  onGuideInstruction: (value: string) => void;
  onPreviewGuide: (instruction: string) => void;
  onProfileNote: (value: string) => void;
  onCustomAgentInstructions: (value: string) => void;
  onRunMemoryConsolidation: () => void;
  onRunSimulation: () => void;
  onSaveProfileNote: () => void;
  onSaveCustomAgentInstructions: () => void;
  onSelectedPlaybook: (value: string) => void;
  onSimulationInput: (value: string) => void;
  onTab: (tab: SmartOpsTab) => void;
}) {
  if (!selected) {
    return (
      <section className="cockpitEmpty">
        <div className="onboardingSteps">
          <PlanStep status="ready" title="第一步：导入或选择好友" detail="左侧搜索好友，导入后点击进入运营驾驶舱。" />
          <PlanStep status="pending" title="第二步：写一句运营背景" detail="例如：老客户，喜欢直接沟通，最近在看 AI 私域运营。" />
          <PlanStep status="pending" title="第三步：让 AI 给出调整预览" detail="确认前不会改配置，适合日常运营放心试。" />
        </div>
      </section>
    );
  }

  const latestReview = decisionReviews[0];
  const currentPlaybook = playbooks.find((playbook) => playbook.id === selectedPlaybookId) || playbooks.find((playbook) => playbook.isDefault);
  const examples = [
    "更像朋友一点，自然一些",
    "这个用户比较忙，降低主动打扰频率",
    "用户已经有明确需求，可以更积极推进下一步",
    "重新分析画像，并补充不能踩的沟通禁忌"
  ];

  return (
    <section className="smartWorkspace panel">
      <div className="panelHead">
        <div>
          <span>当前运营对象</span>
          <h2>{selected.remark || selected.nickname || selected.wxid}</h2>
        </div>
        <div className="statusPill">
          <UserRoundCheck size={15} />
          {selected.agentStatus === "managed" ? "Agent 运营中" : "未加入 Agent"}
        </div>
      </div>

      <SmartOpsTabs active={activeTab} onChange={onTab} />

      {activeTab === "cockpit" && (
        <section className="smartTabPanel">
          <div className="agentBehaviorGrid">
            <div>
              <span>语气风格</span>
              <strong>{selected.agentProfile?.communicationStyle || memoryDraft.communicationPreference || "先专业克制，等待更多上下文"}</strong>
            </div>
            <div>
              <span>跟进节奏</span>
              <strong>{selected.followUpPolicy || memoryDraft.timing || "等待用户消息，不主动高频打扰"}</strong>
            </div>
            <div>
              <span>重点话题</span>
              <strong>{memoryDraft.nextGoal || selected.agentProfile?.operationGoal || "先理解需求和真实场景"}</strong>
            </div>
            <div>
              <span>避免事项</span>
              <strong>{memoryDraft.avoid || selected.operationStateReason || "不要在信息不足时强推销售"}</strong>
            </div>
          </div>

          <section className="cockpitSection">
            <div className="sectionCaption">Agent 当前判断</div>
            <div className="profileGrid compactGrid">
              <div>
                <span>用户理解</span>
                <p>{selected.agentProfile?.summary || selected.humanProfileNote || "还没有足够信息，先补充一句运营背景。"}</p>
              </div>
              <div>
                <span>下一步动作</span>
                <p>{nextBestActionLabel(latestReview?.nextBestAction) || memoryDraft.recommendedMove || "等待用户下一次消息"}</p>
              </div>
              <div>
                <span>当前运营状态</span>
                <p>{selected.operationState || "待判断"}</p>
              </div>
              <div>
                <span>领域信号</span>
                <p>{memoryDraft.fitReason || "未知，需要继续通过对话确认。"}</p>
              </div>
              {/* 波 B2：分别展示入站 / 出站时间，运营据此判断"用户主动来"还是
                  "Agent 主动出"。lastMessageAt 仅作兼容字段不在 UI 暴露。 */}
              <div>
                <span>最近用户来访</span>
                <p>{formatTime(selected.lastInboundAt) || "无"}</p>
              </div>
              <div>
                <span>最近 Agent 触达</span>
                <p>{formatTime(selected.lastOutboundAt) || "无"}</p>
              </div>
            </div>
          </section>

          <section className="cockpitSection">
            <div className="sectionCaption">运营健康度</div>
            <div className="healthGrid compact">
              {(health?.items || defaultHealthItems()).map((item) => (
                <div key={item.key} className={`healthItem ${item.tone}`}>
                  <div>
                    <strong>{item.label}</strong>
                    <span>{item.score}</span>
                  </div>
                  <p>{item.detail}</p>
                </div>
              ))}
            </div>
          </section>

          <section className="cockpitSection">
            <div className="sectionCaption">长期记忆卡片</div>
            <MemoryCardSummary memoryCard={operatingMemory?.memoryCard} />
          </section>

          <PlannerViewSection contact={selected} />
        </section>
      )}

      {activeTab === "adjust" && (
        <section className="smartTabPanel guidePanel">
          <div className="panelHead compact unlined">
            <div>
              <span>AI 调整</span>
              <h2>你想怎么运营这个用户？</h2>
            </div>
            <Bot size={18} />
          </div>
          <textarea
            value={guideInstruction}
            onChange={(event) => onGuideInstruction(event.target.value)}
            placeholder="例如：更像朋友一点，少一点销售感；这个客户比较忙，跟进不要太频繁。"
          />
          <div className="suggestionRow">
            {examples.map((item) => (
              <button key={item} className="chipButton" onClick={() => onPreviewGuide(item)} disabled={guideBusy}>
                {item}
              </button>
            ))}
          </div>
          <button onClick={() => onPreviewGuide(guideInstruction)} disabled={guideBusy || !guideInstruction.trim()}>
            <Sparkles size={16} />
            {guideBusy ? "生成中" : "生成修改预览"}
          </button>
          {guidePreview && (
            <div className="guidePreview">
              <div className={`impactScope ${guidePreview.impactScope || "current_contact"}`}>
                <span>影响范围</span>
                <strong>{impactScopeLabel(guidePreview.impactScope)}</strong>
                <p>{guidePreview.scopeReason || "默认只影响当前好友。"}</p>
              </div>
              <strong>修改预览</strong>
              <p>{guidePreview.summary}</p>
              <ChangePreview changes={guidePreview.suggestedChanges} readableChanges={guidePreview.readableChanges} />
              {guidePreview.riskWarnings.length > 0 && (
                <div className="riskList">
                  {guidePreview.riskWarnings.map((warning, index) => <span key={`${warning}-${index}`}>{warning}</span>)}
                </div>
              )}
              <button onClick={onApplyGuidePreview} disabled={guideBusy}>
                确认应用
              </button>
            </div>
          )}
        </section>
      )}

      {activeTab === "profile" && (
        <section className="smartTabPanel profileEditor">
          <div className="modeLine editable">运营可编辑，只影响当前好友</div>
          <label>
            <span>运营风格模板</span>
            <select value={selectedPlaybookId} onChange={(event) => onSelectedPlaybook(event.target.value)}>
              {playbooks.map((playbook) => (
                <option key={playbook.id} value={playbook.id}>
                  {playbook.name}{playbook.isDefault ? " / 默认" : ""}
                </option>
              ))}
            </select>
          </label>
          <label>
            <span>你对这个用户的判断</span>
            <textarea
              value={profileNote}
              onChange={(event) => onProfileNote(event.target.value)}
              placeholder="写这个人是谁、喜欢什么沟通方式、哪些话题不要碰、下一步希望推进什么。"
            />
          </label>
          <label>
            <span>运营人员特别指令（最高优先级，可空）</span>
            <textarea
              value={customAgentInstructions}
              maxLength={1000}
              rows={5}
              onChange={(event) => onCustomAgentInstructions(event.target.value)}
              placeholder="例：这个客户已签约老客户，不要主动推销，只服务问题。Agent 将在每轮对话最末尾读取这段指令。"
            />
            <span className="counter">{customAgentInstructions.length} / 1000</span>
            {selected.agentStatus === "managed" && (
              <button className="secondary" onClick={onSaveCustomAgentInstructions} disabled={busy} type="button">
                <SquarePen size={16} />
                保存特别指令
              </button>
            )}
          </label>
          <div className="buttonRow">
            {selected.agentStatus === "managed" ? (
              <>
                <button onClick={onSaveProfileNote} disabled={busy}>
                  <SquarePen size={16} />
                  保存并重建画像
                </button>
                <button className="secondary" onClick={onAnalyzeProfile} disabled={busy}>
                  <Sparkles size={16} />
                  AI 重新分析
                </button>
                <button className="secondary" onClick={onDisableAgent} disabled={busy}>
                  停止运营
                </button>
              </>
            ) : (
              <button onClick={onEnableAgent} disabled={busy || !profileNote.trim()}>
                <SendHorizonal size={16} />
                加入 Agent 运营
              </button>
            )}
          </div>
          <div className="methodSummary">
            <strong>{currentPlaybook?.name || "默认运营风格"}</strong>
            <p>{currentPlaybook?.description || currentPlaybook?.replyStyle || "传统模式里可以维护完整策略。"}</p>
          </div>
        </section>
      )}

      {activeTab === "memory" && (
        <section className="smartTabPanel memoryPanel">
          <div className="panelHead compact unlined">
            <div>
              <span>长期记忆</span>
              <h2>Agent 已确认和待整理的信息</h2>
            </div>
            <button className="secondary" onClick={onRunMemoryConsolidation} disabled={busy}>
              <Sparkles size={16} />
              整理候选
            </button>
          </div>
          <MemoryCardSummary memoryCard={operatingMemory?.memoryCard} />
          <div className="memoryCandidateList">
            <div className="sectionCaption">候选记忆</div>
            {memoryCandidates.map((item) => (
              <article key={item.id} className="memoryCandidate">
                <header>
                  <strong>{memoryStatusLabel(item.status)} / {item.source || "agent"}</strong>
                  <span>score {item.memoryWriteScore} · {formatTime(item.createdAt)}</span>
                </header>
                {(item.candidates || []).slice(0, 4).map((candidate, index) => (
                  <p key={`${item.id}-${index}`}>{memoryCandidateText(candidate)}</p>
                ))}
                {item.reason && <small>{item.reason}</small>}
              </article>
            ))}
            {!memoryCandidates.length && <EmptyInline text="暂无候选记忆。只有影响长期运营的事实、偏好、禁忌、承诺和异议才会进入这里。" />}
          </div>
        </section>
      )}

      {activeTab === "simulation" && (
        <section className="smartTabPanel simulationPanel">
          <div className="panelHead compact unlined">
            <div>
              <span>影子验证</span>
              <h2>模拟长对话，不触发真实发送</h2>
            </div>
            <Activity size={18} />
          </div>
          <textarea
            value={simulationInput}
            onChange={(event) => onSimulationInput(event.target.value)}
            placeholder="每行一条用户消息，按真实聊天顺序输入。"
          />
          <div className="simulationToolbar">
            <span>Shadow 模式只看决策、风险和记忆变化，不写入真实会话。</span>
            <button onClick={onRunSimulation} disabled={simulationBusy || !simulationInput.trim()}>
              <Sparkles size={16} />
              {simulationBusy ? "验证中" : "开始验证"}
            </button>
          </div>
          <SimulationResult turns={simulationTurns} />
        </section>
      )}

      {activeTab === "conversation" && (
        <section className="smartTabPanel conversationGrid">
          <ConversationStream messages={messages} />

          <div className="reviewList">
            <div className="sectionCaption">最近复盘</div>
            {decisionReviews.slice(0, 4).map((review) => (
              <div key={review.id} className="reviewItem">
                <strong>{review.approved ? "通过" : "拦截"} / {review.operationState || "未记录状态"}</strong>
                <p>{review.reviewSummary || review.replyText || "-"}</p>
                <span>{formatTime(review.createdAt)}</span>
              </div>
            ))}
            {!decisionReviews.length && <EmptyInline text="暂无决策复盘" />}
          </div>
        </section>
      )}
    </section>
  );
}

function ChangePreview({ changes, readableChanges }: { changes: Record<string, unknown>; readableChanges?: string[] }) {
  const items = readableChanges?.length
    ? readableChanges.map((value) => ({ label: "将执行", value }))
    : readableChangeItems(changes);
  if (!items.length) return <EmptyInline text="这次建议不需要直接改配置。" />;
  return (
    <div className="changePreviewList">
      {items.map((item, index) => (
        <div key={`${item.label}-${index}`}>
          <span>{item.label}</span>
          <p>{item.value}</p>
        </div>
      ))}
    </div>
  );
}

function MemoryCardSummary({ memoryCard }: { memoryCard?: Record<string, unknown> }) {
  const factSections = [
    { key: "coreFacts", label: "核心事实" },
    { key: "recentFacts", label: "近期事实" },
    { key: "deprecatedFacts", label: "已过期事实" }
  ];
  const plainSections = [
    { key: "preferences", label: "偏好" },
    { key: "objections", label: "异议" },
    { key: "commitments", label: "承诺" },
    { key: "doNotDo", label: "禁忌" },
    { key: "openLoops", label: "待办" },
    { key: "conflicts", label: "记忆冲突" }
  ];
  const profile = memoryCard?.coreProfile as Record<string, unknown> | undefined;
  const relation = memoryCard?.relationshipState as Record<string, unknown> | undefined;
  const factItems = factSections
    .map((section) => ({ ...section, facts: memoryFactList(memoryCard, section.key) }))
    .filter((section) => section.facts.length > 0);
  const plainItems = plainSections
    .map((section) => ({ ...section, values: contextPackList(memoryCard, section.key) }))
    .filter((section) => section.values.length > 0);
  if (!factItems.length && !plainItems.length && !profile && !relation) {
    return <EmptyInline text="还没有形成长期记忆。下一次真实对话或模拟验证后会生成。" />;
  }
  return (
    <div className="contextPackGrid">
      {(profile || relation) && (
        <div>
          <span>核心画像</span>
          <p>{[
            stringField(profile || {}, "identity"),
            stringField(profile || {}, "businessContext"),
            stringField(profile || {}, "communicationStyle"),
            stringField(relation || {}, "stage")
          ].filter(Boolean).join(" / ") || "待确认"}</p>
        </div>
      )}
      {factItems.map((section) => (
        <div key={section.key}>
          <span>{section.label}</span>
          {section.facts.slice(0, 4).map((fact, index) => (
            <MemoryFactRow key={`${section.key}-${index}`} fact={fact} />
          ))}
        </div>
      ))}
      {plainItems.map((section) => (
        <div key={section.key}>
          <span>{section.label}</span>
          {section.values.slice(0, 4).map((value, index) => <p key={`${section.key}-${index}`}>{value}</p>)}
        </div>
      ))}
    </div>
  );
}

type MemoryFactView = {
  text: string;
  evidence?: string;
  confidence?: number;
  importance?: number;
  mayExpire?: boolean;
  deprecatedAt?: string;
  deprecationReason?: string;
};

function memoryFactList(source: Record<string, unknown> | undefined, key: string): MemoryFactView[] {
  const value = source?.[key];
  if (!Array.isArray(value)) return [];
  return value
    .map((item): MemoryFactView | null => {
      if (typeof item === "string") {
        const text = item.trim();
        return text ? { text } : null;
      }
      if (item && typeof item === "object") {
        const obj = item as Record<string, unknown>;
        const text = stringField(obj, "text").trim();
        if (!text) return null;
        const confidence = typeof obj.confidence === "number" ? obj.confidence : undefined;
        const importance = typeof obj.importance === "number" ? obj.importance : undefined;
        const mayExpire = typeof obj.mayExpire === "boolean" ? obj.mayExpire : undefined;
        const evidence = stringField(obj, "evidence").trim() || undefined;
        const deprecatedAt = stringField(obj, "deprecatedAt").trim() || undefined;
        const deprecationReason = stringField(obj, "deprecationReason").trim() || undefined;
        return { text, evidence, confidence, importance, mayExpire, deprecatedAt, deprecationReason };
      }
      return null;
    })
    .filter((v): v is MemoryFactView => v !== null);
}

function MemoryFactRow({ fact }: { fact: MemoryFactView }) {
  const [expanded, setExpanded] = useState(false);
  const isDeprecated = Boolean(fact.deprecatedAt);
  const tooltip = isDeprecated
    ? `已弃用：${fact.deprecatedAt}${fact.deprecationReason ? `（${fact.deprecationReason}）` : ""}`
    : undefined;
  return (
    <div className="memoryFactRow">
      <p
        className={isDeprecated ? "memoryFactDeprecated" : undefined}
        title={tooltip}
      >
        {fact.text}
      </p>
      <div className="memoryFactMeta">
        {typeof fact.confidence === "number" && (
          <span className="memoryFactChip" title="confidence 0-10">置信 {fact.confidence}</span>
        )}
        {typeof fact.importance === "number" && (
          <span className="memoryFactChip" title="importance 0-10">重要 {fact.importance}</span>
        )}
        {fact.mayExpire && <span className="memoryFactChip memoryFactBadge">易失效</span>}
        {fact.evidence && (
          <button
            type="button"
            className="memoryFactToggle"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? "收起证据" : "展开证据"}
          </button>
        )}
      </div>
      {expanded && fact.evidence && <p className="memoryFactEvidence">{fact.evidence}</p>}
    </div>
  );
}

function SimulationResult({ turns }: { turns: SimulationTurn[] }) {
  if (!turns.length) return <EmptyInline text="还没有验证结果。输入多轮用户消息后开始验证。" />;
  return (
    <div className="simulationResult">
      {turns.map((turn) => {
        const reviewScores = (turn.review?.scores || {}) as Record<string, number>;
        const gatewayAllowed = Boolean(turn.gatewayResult?.allowed);
        const selectedChunks = Array.isArray(turn.knowledgeRoute?.selectedChunkIds)
          ? (turn.knowledgeRoute.selectedChunkIds as string[]).length
          : 0;
        return (
          <article key={turn.turn} className="simulationTurn">
            <header>
              <span>第 {turn.turn} 轮</span>
              <strong className={`simStatus ${turn.status}`}>{simulationStatusLabel(turn.status)}</strong>
            </header>
            <div className="simDialogue">
              <div>
                <span>用户消息</span>
                <p>{turn.inboundText}</p>
              </div>
              <div>
                <span>Agent 候选回复</span>
                <p>{turn.replyText || "本轮判断无需回复"}</p>
              </div>
            </div>
            <div className="simMetrics">
              <span>网关：{gatewayAllowed ? "通过" : String(turn.gatewayResult?.reason || "拦截")}</span>
              <span>幻觉风险：{reviewScores.hallucinationScore ?? "-"}</span>
              <span>知识匹配：{reviewScores.knowledgeGroundingScore ?? "-"}</span>
              <span>真人感：{reviewScores.humanLike ?? "-"}</span>
              <span>知识切片：{selectedChunks}</span>
              <span>状态：{String(turn.stateTransition?.from || "-")} → {String(turn.stateTransition?.to || "-")}</span>
            </div>
            {Array.isArray(turn.review?.risks) && turn.review.risks.length > 0 && (
              <div className="riskList compact">
                {(turn.review.risks as string[]).map((risk, index) => <span key={`${risk}-${index}`}>{risk}</span>)}
              </div>
            )}
            <MemoryCardSummary memoryCard={turn.contextPack} />
          </article>
        );
      })}
    </div>
  );
}

function SmartOpsTabs({ active, onChange }: { active: SmartOpsTab; onChange: (tab: SmartOpsTab) => void }) {
  const tabs: Array<{ id: SmartOpsTab; label: string; meta: string; icon: LucideIcon }> = [
    { id: "cockpit", label: "运营驾驶舱", meta: "看判断和风险", icon: UserRoundCheck },
    { id: "adjust", label: "AI 调整", meta: "自然语言优化", icon: Bot },
    { id: "profile", label: "用户画像", meta: "备注、偏好、禁忌", icon: SquarePen },
    { id: "memory", label: "长期记忆", meta: "确认和候选", icon: BrainCircuit },
    { id: "simulation", label: "模拟验证", meta: "长对话影子运行", icon: Activity },
    { id: "conversation", label: "会话记录", meta: "消息和复盘", icon: MessageSquareText }
  ];

  return (
    <nav className="userOpsTabs smartTabs" aria-label="智能模式功能">
      {tabs.map((tab) => {
        const Icon = tab.icon;
        return (
          <button key={tab.id} className={active === tab.id ? "active" : ""} onClick={() => onChange(tab.id)}>
            <Icon size={16} />
            <span>
              <strong>{tab.label}</strong>
              <small>{tab.meta}</small>
            </span>
          </button>
        );
      })}
    </nav>
  );
}

function TraditionalOpsTabs({
  active,
  managedCount,
  pendingTasks,
  onChange
}: {
  active: TraditionalOpsTab;
  managedCount: number;
  pendingTasks: number;
  onChange: (tab: TraditionalOpsTab) => void;
}) {
  const tabs: Array<{ id: TraditionalOpsTab; label: string; meta: string; icon: LucideIcon }> = [
    { id: "playbooks", label: "运营方法", meta: `${managedCount} 个运营好友`, icon: Workflow },
    { id: "prompts", label: "Agent 提示词", meta: "人格、任务、复盘", icon: Bot },
    { id: "settings", label: "运行策略", meta: "频控、边界、状态机", icon: ShieldCheck },
    { id: "audit", label: "审计复盘", meta: `${pendingTasks} 个待跟进`, icon: Clock3 }
  ];

  return (
    <nav className="userOpsTabs" aria-label="传统模式功能">
      {tabs.map((tab) => {
        const Icon = tab.icon;
        return (
          <button key={tab.id} className={active === tab.id ? "active" : ""} onClick={() => onChange(tab.id)}>
            <Icon size={16} />
            <span>
              <strong>{tab.label}</strong>
              <small>{tab.meta}</small>
            </span>
          </button>
        );
      })}
    </nav>
  );
}

function ContactsView({
  busy,
  contactTab,
  contacts,
  importQuery,
  managedCount,
  normalCount,
  query,
  selected,
  totalCount,
  onContactTab,
  onImport,
  onImportQuery,
  onLoadAll,
  onOpenContact,
  onQuery
}: {
  busy: boolean;
  contactTab: ContactTab;
  contacts: Contact[];
  importQuery: string;
  managedCount: number;
  normalCount: number;
  query: string;
  selected: Contact | null;
  totalCount: number;
  onContactTab: (tab: ContactTab) => void;
  onImport: (event: FormEvent) => void;
  onImportQuery: (value: string) => void;
  onLoadAll: () => void;
  onOpenContact: (contact: Contact) => void;
  onQuery: (value: string) => void;
}) {
  return (
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>好友池</span>
          <h2>用户运营池</h2>
        </div>
        <div className="segmented">
          <button className={contactTab === "all" ? "active" : ""} onClick={() => onContactTab("all")}>
            全部 {totalCount}
          </button>
          <button className={contactTab === "managed" ? "active" : ""} onClick={() => onContactTab("managed")}>
            Agent {managedCount}
          </button>
          <button className={contactTab === "normal" ? "active" : ""} onClick={() => onContactTab("normal")}>
            普通 {normalCount}
          </button>
        </div>
      </div>

      <div className="toolbar">
        <form className="searchRow" onSubmit={onImport}>
          <label>
            <Search size={15} />
            <input
              value={importQuery}
              onChange={(event) => onImportQuery(event.target.value)}
              placeholder="搜索并导入好友，例如 AI应用开发"
            />
          </label>
          <button type="submit" disabled={busy || !importQuery.trim()}>
            导入
          </button>
        </form>

        <label className="filter">
          <Search size={15} />
          <input
            value={query}
            onChange={(event) => onQuery(event.target.value)}
            onBlur={onLoadAll}
            placeholder="过滤已导入好友"
          />
        </label>
      </div>

      <div className="contactList">
        {contacts.map((contact) => (
          <button
            key={contact.id}
            className={selected?.id === contact.id ? "contact selected" : "contact"}
            onClick={() => onOpenContact(contact)}
          >
            <span className={contact.agentStatus === "managed" ? "dot managed" : "dot"} />
            <div>
              <strong>{contact.remark || contact.nickname || contact.wxid}</strong>
              <small>{contact.alias || contact.wxid}</small>
            </div>
            <em>{contact.agentStatus === "managed" ? "Agent" : "普通"}</em>
          </button>
        ))}
      </div>
    </section>
  );
}

function OperationsView({
  decisionReviews,
  events,
  llmUsage,
  opsTab,
  tasks,
  onOpsTab
}: {
  decisionReviews: DecisionReview[];
  events: EventItem[];
  llmUsage: LlmUsageResponse | null;
  opsTab: OpsTab;
  tasks: TaskItem[];
  onOpsTab: (tab: OpsTab) => void;
}) {
  return (
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>Operations</span>
          <h2>任务、事件与 Review</h2>
        </div>
        <Clock3 size={18} />
      </div>

      <div className="subTabs">
        <button className={opsTab === "tasks" ? "active" : ""} onClick={() => onOpsTab("tasks")}>
          跟进任务
        </button>
        <button className={opsTab === "events" ? "active" : ""} onClick={() => onOpsTab("events")}>
          运营事件
        </button>
        <button className={opsTab === "reviews" ? "active" : ""} onClick={() => onOpsTab("reviews")}>
          Review 记录
        </button>
        <button className={opsTab === "llm" ? "active" : ""} onClick={() => onOpsTab("llm")}>
          LLM 成本
        </button>
      </div>

      {opsTab === "tasks" && (
        tasks.length === 0 ? (
          <EmptyInline text="暂无跟进任务" />
        ) : (
          <table className="dataTable">
            <thead>
              <tr>
                <th>状态</th>
                <th>任务内容</th>
                <th>计划执行</th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((task) => (
                <tr key={task.id}>
                  <td>{task.status}</td>
                  <td>{task.content}</td>
                  <td>{formatTime(task.runAt)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )
      )}

      {opsTab === "events" && (
        events.length === 0 ? (
          <EmptyInline text="暂无运营事件" />
        ) : (
          <EventTimeline
            items={events.map((event) => ({
              id: event.id,
              tone: eventTone(event.status),
              title: event.kind,
              subtitle: event.summary,
              meta: formatTime(event.createdAt),
              chips: event.status ? [event.status] : undefined,
            }))}
          />
        )
      )}

      {opsTab === "reviews" && (
        decisionReviews.length === 0 ? (
          <EmptyInline text="暂无 Review 记录" />
        ) : (
          <table className="dataTable">
            <thead>
              <tr>
                <th>结论</th>
                <th>下一步</th>
                <th>结果</th>
                <th>评分</th>
                <th>摘要</th>
                <th>时间</th>
              </tr>
            </thead>
            <tbody>
              {decisionReviews.map((review) => (
                <tr key={review.id}>
                  <td>{review.approved ? "通过" : "拦截"}</td>
                  <td>{nextBestActionLabel(review.nextBestAction)}</td>
                  <td>{review.outcomeStatus || "pending"}</td>
                  <td>{formatScores(review.scores)}</td>
                  <td>{review.reviewSummary || review.replyText || "-"}</td>
                  <td>{formatTime(review.createdAt)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )
      )}

      {opsTab === "llm" && (
        <section className="usagePanel">
          <div className="profileGrid compactGrid">
            <div>
              <span>调用次数</span>
              <p>{llmUsage?.summary.totalCalls ?? 0}</p>
            </div>
            <div>
              <span>总 token</span>
              <p>{llmUsage?.summary.totalTokens ?? 0}</p>
            </div>
            <div>
              <span>缓存命中 token</span>
              <p>{llmUsage?.summary.promptCacheHitTokens ?? 0}</p>
            </div>
            <div>
              <span>缓存命中率</span>
              <p>{Math.round((llmUsage?.summary.promptCacheHitRate ?? 0) * 100)}%</p>
            </div>
          </div>
          {(llmUsage?.items || []).length === 0 ? (
            <EmptyInline text="暂无 LLM 调用记录" />
          ) : (
            <table className="dataTable">
              <thead>
                <tr>
                  <th>Prompt Key</th>
                  <th>状态</th>
                  <th>耗时</th>
                  <th>命中</th>
                  <th>未命中</th>
                  <th>时间</th>
                </tr>
              </thead>
              <tbody>
                {(llmUsage?.items || []).map((item) => (
                  <tr key={item.id}>
                    <td>{item.promptKey}</td>
                    <td>{item.status}</td>
                    <td>{item.latencyMs}ms</td>
                    <td>hit {item.promptCacheHitTokens}</td>
                    <td>miss {item.promptCacheMissTokens}</td>
                    <td>{formatTime(item.createdAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      )}
    </section>
  );
}

function ContentAssetsView({
  assetDraft,
  assets,
  busy,
  onAssetDraft,
  onCreateAsset
}: {
  assetDraft: { kind: string; title: string; body: string; url: string; mediaId: string; usageScene: string };
  assets: ContentAsset[];
  busy: boolean;
  onAssetDraft: (draft: { kind: string; title: string; body: string; url: string; mediaId: string; usageScene: string }) => void;
  onCreateAsset: (event: FormEvent) => void;
}) {
  return (
    <section className="methodWorkbench">
      <section className="panel">
        <div className="panelHead">
          <div>
            <span>Content Assets</span>
            <h2>内容资产库</h2>
          </div>
          <FileText size={18} />
        </div>
        <div className="assetList">
          {assets.map((asset) => (
            <div key={asset.id} className="assetRow">
              <strong>{asset.title}</strong>
              <span>{asset.kind}</span>
              <p>{asset.body || asset.url || asset.mediaId || asset.usageScene || "暂无内容"}</p>
            </div>
          ))}
          {!assets.length && <EmptyInline text="暂无内容资产" />}
        </div>
      </section>

      <form className="panel assetForm" onSubmit={onCreateAsset}>
        <div className="panelHead">
          <div>
            <span>新增</span>
            <h2>新增资产</h2>
          </div>
        </div>
        <label>
          <span>类型</span>
          <select value={assetDraft.kind} onChange={(event) => onAssetDraft({ ...assetDraft, kind: event.target.value })}>
            <option value="text">文本资料</option>
            <option value="faq">FAQ</option>
            <option value="script">话术</option>
            <option value="forbidden_expression">禁用表达</option>
            <option value="brand_voice">品牌语气</option>
            <option value="moment_media">朋友圈素材</option>
          </select>
        </label>
        <label>
          <span>标题</span>
          <input value={assetDraft.title} onChange={(event) => onAssetDraft({ ...assetDraft, title: event.target.value })} />
        </label>
        <label>
          <span>正文</span>
          <textarea value={assetDraft.body} onChange={(event) => onAssetDraft({ ...assetDraft, body: event.target.value })} />
        </label>
        <label>
          <span>素材 URL</span>
          <input value={assetDraft.url} onChange={(event) => onAssetDraft({ ...assetDraft, url: event.target.value })} />
        </label>
        <label>
          <span>MCP Media ID</span>
          <input value={assetDraft.mediaId} onChange={(event) => onAssetDraft({ ...assetDraft, mediaId: event.target.value })} />
        </label>
        <label>
          <span>使用场景</span>
          <input value={assetDraft.usageScene} onChange={(event) => onAssetDraft({ ...assetDraft, usageScene: event.target.value })} />
        </label>
        <button type="submit" disabled={busy || !assetDraft.title.trim()}>
          保存资产
        </button>
      </form>
    </section>
  );
}

// Phase E / E5-T1：ops 三表多版本灰度元数据徽章 + publish/rollout/rollback 三动作。
//
// 同一组件复用于 operation_domain_configs / operation_state_policies / system_taxonomies
// 三个面板，所以只接收 ActiveVersionMeta 与 endpoint 前缀，不耦合资源 schema。
//
// `previousVersion` 不为 null 时渲染 "v3 ← v2" 回滚链；`seededBy` 显示写入来源徽章
// （manual / system / legacy_migration）。`canPublish` 由调用方决定（admin 操作员
// 是否手上有未发布草稿），canRollback 受 previousVersion 是否存在管控。
function ActiveVersionsBar({
  meta,
  endpointPrefix,
  resourceLabel,
  busy,
  canPublish = false,
  onAfterAction
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
    <div className="activeVersionsBar">
      <div className="activeVersionsMeta">
        <span className={`activeVersionsBadge ${isCurrent ? "current" : "shadow"}`}>
          v{version}
          {isCurrent ? " · current" : " · shadow"}
        </span>
        {previousVersion !== null && (
          <span className="activeVersionsChain" title="previous_version 回滚链">
            ← v{previousVersion}
          </span>
        )}
        {seededBy && (
          <span className={`activeVersionsSeeded seeded-${seededBy}`} title="写入来源">
            {seededBy}
          </span>
        )}
        {meta.updatedAt && (
          <span className="activeVersionsTimestamp" title="updated_at">
            {meta.updatedAt}
          </span>
        )}
      </div>
      <div className="activeVersionsActions">
        {canPublish && (
          <button
            type="button"
            className="secondary"
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
            className="secondary"
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
            className="secondary"
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

function DomainConfigEditor({
  busy,
  config,
  draft,
  mode: _mode,
  onDraft,
  onReset,
  onSave,
  onAfterVersionAction
}: {
  busy: boolean;
  config?: OperationDomainConfig;
  draft: OperationDomainDraft;
  mode?: "primary" | "standard";
  onDraft: (draft: OperationDomainDraft) => void;
  onReset: () => void;
  onSave: () => void;
  onAfterVersionAction?: () => void | Promise<void>;
}) {
  const runtimeParams = runtimeParametersFromText(draft.runtimeParameters);
  const states = stateMachineStates(draft.stateMachine);

  function setRuntimeParameter(key: string, value: string | boolean) {
    onDraft({
      ...draft,
      runtimeParameters: runtimeParametersWithValue(draft.runtimeParameters, key, value)
    });
  }

  function setStateValue(key: string, field: keyof OperationStateDraft, value: string | boolean) {
    onDraft({
      ...draft,
      stateMachine: stateMachineWithValue(draft.stateMachine, key, field, value)
    });
  }

  return (
    <section className="panel domainSettingsPanel">
      <div className="panelHead">
        <div>
          <span>运行策略</span>
          <h2>{draft.name || config?.name || "用户运营大脑"}</h2>
        </div>
        <div className="buttonRow">
          <button type="button" className="secondary" onClick={onReset} disabled={busy}>
            恢复默认
          </button>
          <button type="button" onClick={onSave} disabled={busy || !draft.name.trim()}>
            保存策略
          </button>
        </div>
      </div>

      {config && (
        <ActiveVersionsBar
          meta={config}
          endpointPrefix="/api/admin/operation-domains"
          resourceLabel={`Domain ${config.domain}`}
          busy={busy}
          canPublish
          onAfterAction={onAfterVersionAction}
        />
      )}

      <div className="settingsLayout">
        <section className="settingsSection">
          <div className="sectionCaption">基础策略</div>
          <div className="domainConfigGrid compact">
            <label>
              <span>模块名称</span>
              <input value={draft.name} onChange={(event) => onDraft({ ...draft, name: event.target.value })} />
            </label>
            <label>
              <span>长期运营目标</span>
              <textarea value={draft.goal} onChange={(event) => onDraft({ ...draft, goal: event.target.value })} />
            </label>
            <label>
              <span>核心方法论</span>
              <textarea value={draft.methodology} onChange={(event) => onDraft({ ...draft, methodology: event.target.value })} />
            </label>
          </div>
        </section>

        <section className="settingsSection">
          <div className="sectionCaption">执行边界</div>
          <div className="domainConfigGrid compact">
            <label>
              <span>工作流</span>
              <textarea value={draft.workflow} onChange={(event) => onDraft({ ...draft, workflow: event.target.value })} />
            </label>
            <label>
              <span>工具和边界</span>
              <textarea value={draft.toolPolicy} onChange={(event) => onDraft({ ...draft, toolPolicy: event.target.value })} />
            </label>
            <label>
              <span>自动化策略</span>
              <textarea value={draft.automationPolicy} onChange={(event) => onDraft({ ...draft, automationPolicy: event.target.value })} />
            </label>
            <label>
              <span>复盘规则</span>
              <textarea value={draft.reviewPolicy} onChange={(event) => onDraft({ ...draft, reviewPolicy: event.target.value })} />
            </label>
          </div>
        </section>

        <section className="settingsSection">
          <div className="sectionCaption">运行参数</div>
          <div className="runtimeParameterGrid">
            {USER_RUNTIME_PARAMETER_FIELDS.map((field) => (
              <label key={field.key}>
                <span>{field.label}</span>
                <small>{field.detail}</small>
                {field.kind === "boolean" ? (
                  <select value={String(runtimeParams[field.key] ?? field.defaultValue)} onChange={(event) => setRuntimeParameter(field.key, event.target.value === "true")}>
                    <option value="true">启用</option>
                    <option value="false">关闭</option>
                  </select>
                ) : (
                  <input
                    inputMode="numeric"
                    value={String(runtimeParams[field.key] ?? field.defaultValue)}
                    onChange={(event) => setRuntimeParameter(field.key, event.target.value)}
                  />
                )}
              </label>
            ))}
          </div>
        </section>

        <section className="settingsSection">
          <div className="sectionCaption">用户运营状态机</div>
          <div className="stateMachineGrid">
            {states.map((state) => (
              <article key={state.key} className="stateCard">
                <div className="stateCardHead">
                  <strong>{state.name || state.key}</strong>
                  <span>{state.key}</span>
                </div>
                <label>
                  <span>状态名称</span>
                  <input value={state.name} onChange={(event) => setStateValue(state.key, "name", event.target.value)} />
                </label>
                <label>
                  <span>阶段目标</span>
                  <textarea value={state.goal} onChange={(event) => setStateValue(state.key, "goal", event.target.value)} />
                </label>
                <div className="formGrid">
                  <label>
                    <span>允许动作</span>
                    <textarea value={state.allowedActions} onChange={(event) => setStateValue(state.key, "allowedActions", event.target.value)} />
                  </label>
                  <label>
                    <span>允许迁入来源</span>
                    <textarea
                      value={state.allowedFrom}
                      disabled={state.allowFromAny}
                      onChange={(event) => setStateValue(state.key, "allowedFrom", event.target.value)}
                    />
                    {state.allowFromAny && <small>该状态允许从任意状态迁入</small>}
                  </label>
                </div>
                <div className="formGrid">
                  <label>
                    <span>任意状态可迁入</span>
                    <select value={String(state.allowFromAny)} onChange={(event) => setStateValue(state.key, "allowFromAny", event.target.value === "true")}>
                      <option value="false">关闭</option>
                      <option value="true">启用</option>
                    </select>
                  </label>
                  <label>
                    <span>推进信号</span>
                    <textarea value={state.advanceSignals} onChange={(event) => setStateValue(state.key, "advanceSignals", event.target.value)} />
                  </label>
                </div>
                <div className="formGrid">
                  <label>
                    <span>冷却信号</span>
                    <textarea value={state.cooldownSignals} onChange={(event) => setStateValue(state.key, "cooldownSignals", event.target.value)} />
                  </label>
                  <label>
                    <span>风险规则</span>
                    <textarea value={state.riskRules} onChange={(event) => setStateValue(state.key, "riskRules", event.target.value)} />
                  </label>
                </div>
                <label>
                  <span>成功标准</span>
                  <textarea value={state.successCriteria} onChange={(event) => setStateValue(state.key, "successCriteria", event.target.value)} />
                </label>
              </article>
            ))}
            {!states.length && <EmptyInline text="暂无状态机配置" />}
          </div>
        </section>
      </div>
    </section>
  );
}

function UserPlaybookPanel({
  busy,
  editingPlaybookId,
  generatePlaybookText,
  optimizePlaybookText,
  playbookDraft,
  playbooks,
  onCreatePlaybook,
  onEditPlaybook,
  onGeneratePlaybook,
  onGeneratePlaybookText,
  onNewPlaybook,
  onOptimizePlaybook,
  onOptimizePlaybookText,
  onPlaybookDraft,
  onSavePlaybook,
  onSetDefaultPlaybook
}: {
  busy: boolean;
  editingPlaybookId: string;
  generatePlaybookText: string;
  optimizePlaybookText: string;
  playbookDraft: PlaybookDraft;
  playbooks: OperationPlaybook[];
  onCreatePlaybook: (event: FormEvent) => void;
  onEditPlaybook: (playbook: OperationPlaybook) => void;
  onGeneratePlaybook: (event: FormEvent) => void;
  onGeneratePlaybookText: (value: string) => void;
  onNewPlaybook: () => void;
  onOptimizePlaybook: () => void;
  onOptimizePlaybookText: (value: string) => void;
  onPlaybookDraft: (draft: PlaybookDraft) => void;
  onSavePlaybook: (event: FormEvent) => void;
  onSetDefaultPlaybook: (id: string) => void;
}) {
  return (
    <section className="methodWorkbench">
      <section className="panel">
        <div className="panelHead">
          <div>
            <span>运营方法</span>
            <h2>用户运营方法</h2>
          </div>
        </div>
        <form className="generateBar" onSubmit={onGeneratePlaybook}>
          <label>
            <Sparkles size={15} />
            <input value={generatePlaybookText} onChange={(event) => onGeneratePlaybookText(event.target.value)} />
          </label>
          <button type="submit" disabled={busy || !generatePlaybookText.trim()}>
            生成
          </button>
        </form>
        <section className="methodologyPanel">
          <div>
            <span>Formula</span>
            <h3>长期用户运营公式</h3>
          </div>
          <div className="formulaGrid">
            <div>
              <strong>用户理解</strong>
              <p>身份 + 场景 + 痛点 + 动机 + 决策风格 + 沟通偏好 + 禁忌</p>
            </div>
            <div>
              <strong>关系质量</strong>
              <p>信任 + 情绪价值 + 稳定陪伴 - 打扰感 - 销售压迫感</p>
            </div>
            <div>
              <strong>行动优先级</strong>
              <p>关系增益 + 业务推进 + 产品匹配 + 时机成熟度 - 风险</p>
            </div>
          </div>
        </section>
        <div className="assetList">
          {playbooks.map((playbook) => (
            <button
              key={playbook.id}
              className={editingPlaybookId === playbook.id ? "assetRow selectable selected" : "assetRow selectable"}
              onClick={() => onEditPlaybook(playbook)}
            >
              <strong>{playbook.name}</strong>
              <span>v{playbook.version} / {playbook.createdBy}{playbook.isDefault ? " / 默认" : ""}</span>
              <p>{playbook.description || playbook.methodPrompt}</p>
            </button>
          ))}
          {!playbooks.length && <EmptyInline text="暂无用户运营方法" />}
        </div>
      </section>

      <form className="panel assetForm promptEditor methodEditor" onSubmit={editingPlaybookId ? onSavePlaybook : onCreatePlaybook}>
        <div className="panelHead">
          <div>
            <span>{editingPlaybookId ? "编辑" : "新增"}</span>
            <h2>{editingPlaybookId ? "编辑用户方法" : "新增用户方法"}</h2>
          </div>
          <button type="button" className="secondary compactButton" onClick={onNewPlaybook}>
            新建
          </button>
        </div>
        <label>
          <span>名称</span>
          <input value={playbookDraft.name} onChange={(event) => onPlaybookDraft({ ...playbookDraft, name: event.target.value })} />
        </label>
        <label>
          <span>描述</span>
          <input value={playbookDraft.description} onChange={(event) => onPlaybookDraft({ ...playbookDraft, description: event.target.value })} />
        </label>
        <label>
          <span>方法论总纲</span>
          <textarea value={playbookDraft.methodPrompt} onChange={(event) => onPlaybookDraft({ ...playbookDraft, methodPrompt: event.target.value })} />
        </label>
        <div className="formGrid">
          <label>
            <span>如何理解用户</span>
            <textarea value={playbookDraft.profileMethod} onChange={(event) => onPlaybookDraft({ ...playbookDraft, profileMethod: event.target.value })} />
          </label>
          <label>
            <span>标签识别规则</span>
            <textarea value={playbookDraft.tagMethod} onChange={(event) => onPlaybookDraft({ ...playbookDraft, tagMethod: event.target.value })} />
          </label>
        </div>
        <div className="formGrid">
          <label>
            <span>关系阶段判断</span>
            <textarea value={playbookDraft.stageMethod} onChange={(event) => onPlaybookDraft({ ...playbookDraft, stageMethod: event.target.value })} />
          </label>
          <label>
            <span>意向和时机判断</span>
            <textarea value={playbookDraft.intentMethod} onChange={(event) => onPlaybookDraft({ ...playbookDraft, intentMethod: event.target.value })} />
          </label>
        </div>
        <label>
          <span>跟进节奏和下一步动作</span>
          <textarea value={playbookDraft.followUpMethod} onChange={(event) => onPlaybookDraft({ ...playbookDraft, followUpMethod: event.target.value })} />
        </label>
        <label>
          <span>微信表达风格</span>
          <input value={playbookDraft.replyStyle} onChange={(event) => onPlaybookDraft({ ...playbookDraft, replyStyle: event.target.value })} />
        </label>
        <label>
          <span>禁止行为</span>
          <textarea value={playbookDraft.forbiddenRules} onChange={(event) => onPlaybookDraft({ ...playbookDraft, forbiddenRules: event.target.value })} />
        </label>
        <label>
          <span>复盘和优化标准</span>
          <textarea value={playbookDraft.successCriteria} onChange={(event) => onPlaybookDraft({ ...playbookDraft, successCriteria: event.target.value })} />
        </label>
        {editingPlaybookId && (
          <section className="optimizeBox">
            <div>
              <span>AI Optimize</span>
              <h3>优化当前方法</h3>
            </div>
            <textarea value={optimizePlaybookText} onChange={(event) => onOptimizePlaybookText(event.target.value)} />
            <button type="button" className="secondary" onClick={onOptimizePlaybook} disabled={busy || !optimizePlaybookText.trim()}>
              AI 优化当前方法
            </button>
          </section>
        )}
        <label className="checkLine">
          <input
            type="checkbox"
            checked={playbookDraft.isDefault}
            onChange={(event) => onPlaybookDraft({ ...playbookDraft, isDefault: event.target.checked })}
          />
          <span>设为当前账号默认用户运营方法</span>
        </label>
        <div className="buttonRow">
          <button type="submit" disabled={busy || !playbookDraft.name.trim() || !playbookDraft.methodPrompt.trim()}>
            {editingPlaybookId ? "保存修改" : "保存方法"}
          </button>
          {editingPlaybookId && (
            <button type="button" className="secondary" onClick={() => onSetDefaultPlaybook(editingPlaybookId)} disabled={busy}>
              设为默认
            </button>
          )}
        </div>
      </form>
    </section>
  );
}

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
  onSoulDraft
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
  const updateSoulDraft = (patch: Partial<typeof soulDraft>) => onSoulDraft({
    ...soulDraft,
    ...(lockAgentKind ? { agentKind: defaultAgentKind } : {}),
    ...patch
  });
  const updatePromptDraft = (patch: Partial<PromptTemplateDraft>) => onPromptDraft({
    ...promptDraft,
    ...(lockAgentKind ? { agentKind: defaultAgentKind } : {}),
    ...patch
  });

  return (
    <section className="panel domainPromptPanel">
      <div className="panelHead">
        <div>
          <span>Agent 提示词</span>
          <h2>{title}</h2>
        </div>
      </div>
      <section className="promptWorkbenchBlock">
        <section className="assetList promptLibrary">
          <div className="sectionCaption">人格设定</div>
          {visibleSouls.map((soul) => (
            <button key={soul.id} className={editingSoulId === soul.id ? "assetRow selectable selected" : "assetRow selectable"} onClick={() => onEditSoul(soul)}>
              <strong>
                {soul.name}
                {soul.status === "draft" && <span className="statusBadge statusBadgeDraft">草稿</span>}
              </strong>
              <span>{agentKindLabel(soul.agentKind)} / v{soul.version} / {soul.status}</span>
              <p>{soul.content}</p>
            </button>
          ))}
          {!visibleSouls.length && <EmptyInline text="暂无人格设定" />}
        </section>
        <form className="assetForm promptEditor promptEditorStage" onSubmit={editingSoulId ? onSaveSoul : onCreateSoul}>
          <div className="panelHead compact">
            <div>
              <span>{editingSoulId ? "编辑" : "新增"}</span>
              <h2>{editingSoulId ? "编辑人格设定" : "新增人格设定"}</h2>
            </div>
            <button type="button" className="secondary compactButton" onClick={onNewSoul}>
              新建
            </button>
          </div>
          {lockAgentKind ? (
            <div className="staticField">
              <span>适用对象</span>
              <strong>{agentKindLabel(defaultAgentKind)}</strong>
            </div>
          ) : (
            <label>
              <span>Agent 类型</span>
              <select value={soulDraft.agentKind || defaultAgentKind} onChange={(event) => onSoulDraft({ ...soulDraft, agentKind: event.target.value })}>
                {agentKinds.map((kind) => <option key={kind} value={kind}>{agentKindLabel(kind)}</option>)}
              </select>
            </label>
          )}
          <label>
            <span>名称</span>
            <input value={soulDraft.name} onChange={(event) => updateSoulDraft({ name: event.target.value })} />
          </label>
          <label>
            <span>人格提示词</span>
            <textarea value={soulDraft.content} onChange={(event) => updateSoulDraft({ content: event.target.value })} />
          </label>
          <div className="buttonRow">
            <button type="submit" disabled={busy || !soulDraft.name.trim() || !soulDraft.content.trim()}>{editingSoulId ? "保存修改" : "保存草稿"}</button>
            {editingSoulId && <button type="button" className="secondary" onClick={() => onPublishSoul(editingSoulId)} disabled={busy}>发布</button>}
          </div>
        </form>
      </section>

      <section className="promptWorkbenchBlock">
        <section className="assetList promptLibrary">
          <div className="sectionCaption">任务提示词</div>
          {visiblePrompts.map((template) => (
            <button key={template.id} className={editingPromptId === template.id ? "assetRow selectable selected" : "assetRow selectable"} onClick={() => onEditPromptTemplate(template)}>
              <strong>
                {template.title}
                {template.status === "draft" && <span className="statusBadge statusBadgeDraft">草稿</span>}
              </strong>
              <span>{agentKindLabel(template.agentKind)} / {template.layer} / v{template.version} / {template.status}</span>
              <p>{template.description || template.content}</p>
            </button>
          ))}
          {!visiblePrompts.length && <EmptyInline text="暂无任务提示词" />}
        </section>
        <form className="assetForm promptEditor promptEditorStage" onSubmit={editingPromptId ? onSavePromptTemplate : onCreatePromptTemplate}>
          <div className="panelHead compact">
            <div>
              <span>{editingPromptId ? "编辑" : "新增"}</span>
              <h2>{editingPromptId ? "编辑任务提示词" : "新增任务提示词"}</h2>
            </div>
            <button type="button" className="secondary compactButton" onClick={onNewPromptTemplate}>
              新建
            </button>
          </div>
          <div className="formGrid">
            <label>
              <span>层级</span>
              <select value={promptDraft.layer} onChange={(event) => updatePromptDraft({ layer: event.target.value })}>
                <option value="system_contract">系统契约</option>
                <option value="policy">运营规则</option>
                <option value="task_template">任务模板</option>
                <option value="review">复盘审查</option>
                <option value="methodology_generator">方法论生成</option>
              </select>
            </label>
            <label>
              <span>标题</span>
              <input value={promptDraft.title} onChange={(event) => updatePromptDraft({ title: event.target.value })} />
            </label>
          </div>
          <label>
            <span>业务说明</span>
            <input value={promptDraft.description} onChange={(event) => updatePromptDraft({ description: event.target.value })} />
          </label>
          <label>
            <span>Prompt 内容</span>
            <textarea value={promptDraft.content} onChange={(event) => updatePromptDraft({ content: event.target.value })} />
          </label>
          <details className="advancedFields">
            <summary>高级字段</summary>
            <div className="formGrid">
              <label>
                <span>模板标识</span>
                <input value={promptDraft.promptKey} onChange={(event) => updatePromptDraft({ promptKey: event.target.value })} />
              </label>
              {lockAgentKind ? (
                <div className="staticField">
                  <span>适用对象</span>
                  <strong>{agentKindLabel(defaultAgentKind)}</strong>
                </div>
              ) : (
                <label>
                  <span>Agent 类型</span>
                  <select value={promptDraft.agentKind || defaultAgentKind} onChange={(event) => onPromptDraft({ ...promptDraft, agentKind: event.target.value })}>
                    {agentKinds.map((kind) => <option key={kind} value={kind}>{agentKindLabel(kind)}</option>)}
                  </select>
                </label>
              )}
            </div>
          </details>
          <div className="buttonRow">
            <button type="submit" disabled={busy || !promptDraft.promptKey.trim() || !promptDraft.title.trim() || !promptDraft.content.trim()}>{editingPromptId ? "保存修改" : "保存草稿"}</button>
            {editingPromptId && <button type="button" className="secondary" onClick={() => onPublishPromptTemplate(editingPromptId)} disabled={busy}>发布</button>}
          </div>
        </form>
      </section>
    </section>
  );
}

function SystemStrategyView(props: {
  busy: boolean;
  editingPromptId: string;
  editingSoulId: string;
  promptDraft: PromptTemplateDraft;
  promptTemplates: PromptTemplate[];
  soulDraft: { agentKind: string; name: string; content: string };
  souls: AgentSoul[];
  onCreatePromptTemplate: (event: FormEvent) => void;
  onCreateSoul: (event: FormEvent) => void;
  onEditPromptTemplate: (template: PromptTemplate) => void;
  onEditSoul: (soul: AgentSoul) => void;
  onNewPromptTemplate: () => void;
  onNewSoul: () => void;
  onPromptDraft: (draft: PromptTemplateDraft) => void;
  onPublishPromptTemplate: (id: string) => void;
  onPublishSoul: (id: string) => void;
  onResetPromptPack: () => void;
  onSavePromptTemplate: (event: FormEvent) => void;
  onSaveSoul: (event: FormEvent) => void;
  onSoulDraft: (draft: { agentKind: string; name: string; content: string }) => void;
}) {
  return (
    <section className="domainWorkspace">
      <section className="panel">
        <div className="panelHead">
          <div>
            <span>Global Strategy</span>
            <h2>系统总控策略</h2>
          </div>
          <button className="secondary" onClick={props.onResetPromptPack} disabled={props.busy}>
            重置系统 Prompt Pack v2
          </button>
        </div>
        <div className="domainMethodCards">
          <div className="methodCard"><span>后台管理 Agent</span><p>把自然语言指令转成微信工具调用、项目配置和运营管理任务。</p></div>
          <div className="methodCard"><span>方法论生成 Agent</span><p>把业务目标、人群差异和复盘结果生成可读、可编辑、可验证的方法论。</p></div>
          <div className="methodCard"><span>全局边界</span><p>只管理跨模块规则；用户运营的具体长期策略在用户运营频道维护。</p></div>
        </div>
      </section>
      <DomainPromptPanel
        {...props}
        agentKinds={["management", "methodology"]}
        defaultAgentKind="management"
        title="系统总控 Prompt"
      />
      <StatePolicyAdmin busy={props.busy} />
      <TaxonomiesAdmin busy={props.busy} />
      <LessonsLearnedAdmin busy={props.busy} />
    </section>
  );
}

// Phase E / E5-T1：operation_state_policies 灰度面板（admin 只读列表 + 三动作）。
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
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>State Policies</span>
          <h2>状态机动作策略灰度</h2>
        </div>
        <div className="buttonRow">
          <label className="inlineCheckbox">
            <input
              type="checkbox"
              checked={includeAll}
              onChange={(event) => setIncludeAll(event.target.checked)}
            />
            <span>显示历史版本</span>
          </label>
          <button type="button" className="secondary" onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
        </div>
      </div>
      {error && <div className="inlineError">{error}</div>}
      {!loading && items.length === 0 && <EmptyInline text="暂无状态策略" />}
      <div className="versionedList">
        {items.map((item) => (
          <div key={item.id} className="versionedListItem">
            <div className="versionedListHead">
              <div>
                <span className="versionedListScope">{item.domain}</span>
                <h3>{item.stateKey}</h3>
              </div>
              <span className={`badge ${item.status === "active" ? "ok" : "degraded"}`}>{item.status}</span>
            </div>
            <ActiveVersionsBar
              meta={item}
              endpointPrefix="/api/admin/operation-state-policies"
              resourceLabel={`State ${item.domain}/${item.stateKey}`}
              busy={busy}
              canPublish
              onAfterAction={reload}
            />
            <div className="versionedListBody">
              <div className="versionedListChunk">
                <span>allowed</span>
                <p>{item.allowed.join("，") || "—"}</p>
              </div>
              <div className="versionedListChunk">
                <span>forbidden</span>
                <p>{item.forbidden.join("，") || "—"}</p>
              </div>
              <div className="versionedListChunk">
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

// Phase E / E5-T1：system_taxonomies 灰度面板。
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
      const data = await api.get<{ items: TaxonomyEntry[] }>(
        `/api/admin/taxonomies?${params.toString()}`
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
  }, [includeAll, includeDeprecated]);

  return (
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>Taxonomies</span>
          <h2>双层标签字典灰度</h2>
        </div>
        <div className="buttonRow">
          <label className="inlineCheckbox">
            <input
              type="checkbox"
              checked={includeAll}
              onChange={(event) => setIncludeAll(event.target.checked)}
            />
            <span>显示历史版本</span>
          </label>
          <label className="inlineCheckbox">
            <input
              type="checkbox"
              checked={includeDeprecated}
              onChange={(event) => setIncludeDeprecated(event.target.checked)}
            />
            <span>显示已废弃</span>
          </label>
          <button type="button" className="secondary" onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
        </div>
      </div>
      {error && <div className="inlineError">{error}</div>}
      {!loading && items.length === 0 && <EmptyInline text="暂无字典条目" />}
      <div className="versionedList">
        {items.map((item) => (
          <div key={item.id} className="versionedListItem">
            <div className="versionedListHead">
              <div>
                <span className="versionedListScope">{item.scope} · {item.kind}</span>
                <h3>{item.value.label || item.value.id}</h3>
              </div>
              <span className={`badge ${item.value.status === "active" ? "ok" : "degraded"}`}>
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
            <div className="versionedListBody">
              <div className="versionedListChunk">
                <span>id</span>
                <p>{item.value.id}</p>
              </div>
              <div className="versionedListChunk">
                <span>aliases</span>
                <p>{(item.value.aliases ?? []).join("，") || "—"}</p>
              </div>
              {item.value.description && (
                <div className="versionedListChunk">
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
    if (kind === "success") return "badge ok";
    if (kind === "reviewer_misjudge_negative") return "badge degraded";
    if (kind === "blocked_by_safety_guard") return "badge warn";
    return "badge";
  }

  function patternLabel(kind: string): string {
    if (kind === "success") return "成功模式";
    if (kind === "reviewer_misjudge_negative") return "Reviewer 误判（用户负反应）";
    if (kind === "blocked_by_safety_guard") return "安全门拦截";
    return kind || "未识别";
  }

  return (
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>Lessons Learned</span>
          <h2>跨用户教训归纳（14d 滑窗）</h2>
        </div>
        <div className="buttonRow">
          <select
            value={patternKind}
            onChange={(event) => setPatternKind(event.target.value)}
            disabled={busy || loading}
          >
            <option value="">全部模式</option>
            <option value="success">success</option>
            <option value="reviewer_misjudge_negative">reviewer_misjudge_negative</option>
            <option value="blocked_by_safety_guard">blocked_by_safety_guard</option>
          </select>
          <button type="button" className="secondary" onClick={() => void reload()} disabled={busy || loading}>
            刷新
          </button>
        </div>
      </div>
      <p className="panelHint">
        feedback_worker 周期把 agent_run_logs 的胜/败模式压缩成可被下一轮决策检索的颗粒；
        admin 在此抽象为 chunk_type=peer_case 候选 chunk（仍走知识审核队列二次确认才能 verify）。
      </p>
      {error && <div className="inlineError">{error}</div>}
      {!loading && items.length === 0 && <EmptyInline text="暂无教训聚合（窗口内无命中样本）" />}
      <div className="versionedList">
        {items.map((item) => (
          <div key={item.lessonId} className="versionedListItem">
            <div className="versionedListHead">
              <div>
                <span className="versionedListScope">{patternLabel(item.patternKind)}</span>
                <h3>
                  {item.lessonId}
                  <span style={{ marginLeft: 8, fontWeight: 400, opacity: 0.7 }}>×{item.count}</span>
                </h3>
              </div>
              <div className="buttonRow">
                <span className={patternBadgeClass(item.patternKind)}>{item.reviewStatus}</span>
                {item.reviewStatus !== "promoted" && (
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => openPromote(item.lessonId)}
                    disabled={busy || loading || promoting !== null}
                  >
                    晋升为 peer_case
                  </button>
                )}
              </div>
            </div>
            <div className="versionedListBody">
              <div className="versionedListChunk">
                <span>sample run ids ({item.sampleRunIds.length})</span>
                <p>
                  {item.sampleRunIds.length === 0
                    ? "—"
                    : item.sampleRunIds.map((rid) => (
                        <code key={rid} style={{ marginRight: 6, opacity: 0.85 }}>{rid}</code>
                      ))}
                </p>
              </div>
              <div className="versionedListChunk">
                <span>updated</span>
                <p>{item.updatedAt || "—"}</p>
              </div>
              <div className="versionedListChunk">
                <span>created</span>
                <p>{item.createdAt || "—"}</p>
              </div>
              {item.promotedChunkId && (
                <div className="versionedListChunk">
                  <span>promoted chunk</span>
                  <p><code>{item.promotedChunkId}</code></p>
                </div>
              )}
              {promoting === item.lessonId && (
                <div className="versionedListChunk" style={{ gridColumn: "1 / -1" }}>
                  <span>晋升为 peer_case 候选 chunk（仍需 admin 在知识审核队列 verify）</span>
                  <div style={{ display: "grid", gap: 8 }}>
                    <input
                      type="text"
                      placeholder="title（≤ 200 字）"
                      value={draftTitle}
                      onChange={(e) => setDraftTitle(e.target.value)}
                      maxLength={200}
                    />
                    <input
                      type="text"
                      placeholder="summary（一句话，可选）"
                      value={draftSummary}
                      onChange={(e) => setDraftSummary(e.target.value)}
                    />
                    <textarea
                      placeholder="body：案例正文（≤ 4000 字）"
                      value={draftBody}
                      onChange={(e) => setDraftBody(e.target.value)}
                      rows={6}
                      maxLength={4000}
                    />
                    {promoteError && <div className="inlineError">{promoteError}</div>}
                    <div className="buttonRow">
                      <button
                        type="button"
                        className="primary"
                        onClick={() => void submitPromote()}
                        disabled={busy || !draftTitle.trim() || !draftBody.trim()}
                      >
                        提交晋升
                      </button>
                      <button type="button" className="secondary" onClick={closePromote}>
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


function NextPhasePanel({ text, title }: { text: string; title: string }) {
  return (
    <section className="panel emptyPanel">
      <EmptyInline text={text} />
      <h2>{title}</h2>
    </section>
  );
}

type LlmProviderItem = {
  providerId: string;
  name: string;
  format: string;
  baseUrl: string;
  apiKeyMasked: string;
  model: string;
  isActive: boolean;
  timeoutSeconds?: number | null;
  maxRetries?: number | null;
  retryBaseMs?: number | null;
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
};

type LlmProviderDraft = {
  isNew: boolean;
  providerId: string;
  name: string;
  format: "openai" | "anthropic";
  baseUrl: string;
  apiKey: string;
  model: string;
  timeoutSeconds: string;
  maxRetries: string;
  retryBaseMs: string;
  supportsVision: boolean;
};

function emptyLlmProviderDraft(): LlmProviderDraft {
  return {
    isNew: true,
    providerId: "",
    name: "",
    format: "openai",
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
    providerId: item.providerId,
    name: item.name,
    format: item.format === "anthropic" ? "anthropic" : "openai",
    baseUrl: item.baseUrl,
    apiKey: item.apiKeyMasked,
    model: item.model,
    timeoutSeconds: item.timeoutSeconds == null ? "" : String(item.timeoutSeconds),
    maxRetries: item.maxRetries == null ? "" : String(item.maxRetries),
    retryBaseMs: item.retryBaseMs == null ? "" : String(item.retryBaseMs),
    supportsVision: Boolean(item.supportsVision)
  };
}

function LlmProvidersView() {
  const [items, setItems] = useState<LlmProviderItem[]>([]);
  const [active, setActive] = useState<LlmProviderListResponse["active"]>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<LlmProviderDraft | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<LlmProviderTestResponse | null>(null);
  const [showApiKey, setShowApiKey] = useState(false);

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
    setDraft(emptyLlmProviderDraft());
    setTestResult(null);
  }

  function startEdit(item: LlmProviderItem) {
    setDraft(draftFromItem(item));
    setTestResult(null);
  }

  function cancelEdit() {
    setDraft(null);
    setTestResult(null);
  }

  function buildUpsertBody(d: LlmProviderDraft) {
    const body: Record<string, unknown> = {
      providerId: d.providerId.trim(),
      name: d.name.trim() || d.providerId.trim(),
      format: d.format,
      baseUrl: d.baseUrl.trim(),
      apiKey: d.apiKey,
      model: d.model.trim(),
      supportsVision: d.supportsVision
    };
    if (d.timeoutSeconds.trim()) {
      const v = Number(d.timeoutSeconds);
      if (!Number.isNaN(v) && v > 0) body.timeoutSeconds = Math.floor(v);
    }
    if (d.maxRetries.trim()) {
      const v = Number(d.maxRetries);
      if (!Number.isNaN(v) && v >= 0) body.maxRetries = Math.floor(v);
    }
    if (d.retryBaseMs.trim()) {
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
        await api.put(`/api/admin/llm-providers/${encodeURIComponent(draft.providerId)}`, body);
      }
      await refetch();
      setDraft(null);
      setTestResult(null);
    } catch (err) {
      window.alert(`保存失败：${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function deleteItem(item: LlmProviderItem) {
    if (item.isActive) {
      window.alert("当前激活配置不可删除，请先激活其它 provider");
      return;
    }
    if (!window.confirm(`确认删除 provider「${item.name || item.providerId}」？`)) return;
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
  async function setVisionItem(item: LlmProviderItem, active: boolean) {
    if (active && !item.supportsVision) {
      window.alert("该 provider 未勾选「支持图片」，请先在编辑里开启 supportsVision 再指派为视觉模型");
      return;
    }
    setBusy(true);
    try {
      await api.post(`/api/admin/llm-providers/${encodeURIComponent(item.providerId)}/vision`, {
        active
      });
      await refetch();
    } catch (err) {
      window.alert(`设置视觉模型失败：${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function runTest() {
    setTestResult(null);
    setTesting(true);
    try {
      const body: Record<string, unknown> = {};
      if (draft && !draft.isNew) {
        body.providerId = draft.providerId;
        body.format = draft.format;
        body.baseUrl = draft.baseUrl;
        body.model = draft.model;
        if (draft.apiKey && !draft.apiKey.includes("****")) body.apiKey = draft.apiKey;
        if (draft.timeoutSeconds.trim()) {
          const v = Number(draft.timeoutSeconds);
          if (!Number.isNaN(v) && v > 0) body.timeoutSeconds = Math.floor(v);
        }
      } else if (draft) {
        body.format = draft.format;
        body.baseUrl = draft.baseUrl;
        body.apiKey = draft.apiKey;
        body.model = draft.model;
        if (draft.timeoutSeconds.trim()) {
          const v = Number(draft.timeoutSeconds);
          if (!Number.isNaN(v) && v > 0) body.timeoutSeconds = Math.floor(v);
        }
      }
      const result = await api.post<LlmProviderTestResponse>(
        "/api/admin/llm-providers/test",
        body
      );
      setTestResult(result);
    } catch (err) {
      setTestResult({
        ok: false,
        latencyMs: 0,
        error: { kind: "client_error", detail: (err as Error).message }
      });
    } finally {
      setTesting(false);
    }
  }

  return (
    <section className="domainWorkspace llmProvidersView">
      <section className="panel">
        <div className="panelHead">
          <div>
            <span>LLM Providers</span>
            <h2>AI 模型服务商配置</h2>
            <p className="panelHeadHint">
              {active ? (
                <>
                  当前激活：<strong>{active.providerId}</strong> · {active.format} · {active.model}
                </>
              ) : (
                <>尚未加载</>
              )}
            </p>
          </div>
          <div className="panelActions">
            <button className="secondary" onClick={() => void refetch()} disabled={loading}>
              <RefreshCw size={14} /> 刷新
            </button>
            <button onClick={startCreate} disabled={busy}>
              <SquarePen size={14} /> 新增 provider
            </button>
          </div>
        </div>
        {error && <div className="errorBanner">{error}</div>}

        <div className="llmProvidersList">
          {items.length === 0 && !loading && (
            <div className="emptyHint">暂无配置。点击右上「新增 provider」创建第一条。</div>
          )}
          {items.map((item) => (
            <article
              key={item.providerId}
              className={`llmProviderCard${item.isActive ? " active" : ""}`}
            >
              <header>
                <div>
                  <strong>{item.name || item.providerId}</strong>
                  <span className="providerId">{item.providerId}</span>
                </div>
                <div className="badges">
                  <span className={`formatBadge ${item.format}`}>{item.format.toUpperCase()}</span>
                  {item.isActive && <span className="activeBadge">已激活</span>}
                  {item.supportsVision && <span className="visionBadge">支持图片</span>}
                  {item.isVisionActive && <span className="visionActiveBadge">视觉模型</span>}
                </div>
              </header>
              <dl>
                <div><dt>baseUrl</dt><dd className="mono">{item.baseUrl}</dd></div>
                <div><dt>model</dt><dd className="mono">{item.model}</dd></div>
                <div><dt>apiKey</dt><dd className="mono">{item.apiKeyMasked}</dd></div>
                <div>
                  <dt>超时 / 重试</dt>
                  <dd>
                    {item.timeoutSeconds ?? "默认"}s · 重试 {item.maxRetries ?? "默认"} 次 · 退避基线 {item.retryBaseMs ?? "默认"}ms
                  </dd>
                </div>
              </dl>
              <footer>
                {!item.isActive && (
                  <button className="secondary" onClick={() => void activateItem(item)} disabled={busy}>
                    激活
                  </button>
                )}
                {item.supportsVision && !item.isVisionActive && (
                  <button
                    className="secondary"
                    onClick={() => void setVisionItem(item, true)}
                    disabled={busy}
                    title="指派为本 workspace 处理图片的专职视觉模型"
                  >
                    设为视觉模型
                  </button>
                )}
                {item.isVisionActive && (
                  <button
                    className="secondary"
                    onClick={() => void setVisionItem(item, false)}
                    disabled={busy}
                    title="取消视觉模型指派"
                  >
                    取消视觉模型
                  </button>
                )}
                <button className="secondary" onClick={() => startEdit(item)} disabled={busy}>
                  <SquarePen size={13} /> 编辑
                </button>
                <button
                  className="danger"
                  onClick={() => void deleteItem(item)}
                  disabled={busy || item.isActive}
                  title={item.isActive ? "请先激活其它 provider 后再删除" : "删除"}
                >
                  <Trash2 size={13} /> 删除
                </button>
              </footer>
            </article>
          ))}
        </div>
      </section>

      {draft && (
        <section className="panel llmProviderEditor">
          <div className="panelHead">
            <div>
              <span>{draft.isNew ? "新增 provider" : "编辑 provider"}</span>
              <h2>{draft.isNew ? "新增 LLM 服务商" : draft.name || draft.providerId}</h2>
              {!draft.isNew && (
                <small className="panelHeadSub">
                  <code>{draft.providerId}</code> · {draft.format === "anthropic" ? "Anthropic 兼容" : "OpenAI 兼容"}
                </small>
              )}
            </div>
            <button className="secondary" onClick={cancelEdit} disabled={busy}>
              <X size={14} /> 关闭
            </button>
          </div>

          <div className="editorSectionTitle">协议格式</div>
          <div className="formatPickerCards">
            <button
              type="button"
              className={`formatCard${draft.format === "openai" ? " selected" : ""}`}
              onClick={() => setDraft({ ...draft, format: "openai" })}
              disabled={busy}
            >
              <div className="formatCard__title">OpenAI 兼容</div>
              <div className="formatCard__meta">POST /chat/completions · Authorization: Bearer</div>
              <div className="formatCard__hint">兼容 OpenAI 协议形态的服务商或自建网关</div>
            </button>
            <button
              type="button"
              className={`formatCard${draft.format === "anthropic" ? " selected" : ""}`}
              onClick={() => setDraft({ ...draft, format: "anthropic" })}
              disabled={busy}
            >
              <div className="formatCard__title">Anthropic 兼容</div>
              <div className="formatCard__meta">POST /v1/messages · x-api-key</div>
              <div className="formatCard__hint">兼容 Anthropic Messages 协议形态的服务商或自建网关</div>
            </button>
          </div>

          <div className="editorSectionTitle">基本信息</div>
          <div className="formGrid">
            <label>
              <span>provider 标识 (providerId)</span>
              <input
                value={draft.providerId}
                onChange={(e) => setDraft({ ...draft, providerId: e.target.value })}
                disabled={!draft.isNew || busy}
                placeholder="如 my-llm-prod / gateway-a"
              />
              <small>唯一 slug，保存后不可修改</small>
            </label>
            <label>
              <span>展示名称</span>
              <input
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                disabled={busy}
                placeholder="便于识别的展示名"
              />
            </label>
            <label>
              <span>model</span>
              <input
                value={draft.model}
                onChange={(e) => setDraft({ ...draft, model: e.target.value })}
                disabled={busy}
                placeholder={
                  draft.format === "anthropic"
                    ? "请填写 Anthropic 形态的模型 ID"
                    : "请填写 OpenAI 形态的模型 ID"
                }
              />
            </label>
          </div>

          <div className="editorSectionTitle">连接配置</div>
          <div className="formGrid">
            <label className="spanFull">
              <span>baseUrl</span>
              <input
                value={draft.baseUrl}
                onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
                disabled={busy}
                placeholder={
                  draft.format === "anthropic"
                    ? "https://your-host"
                    : "https://your-host/v1"
                }
              />
              <small>OpenAI 形态需含 /v1；Anthropic 形态填到根域即可</small>
            </label>
            <label className="spanFull">
              <span>apiKey</span>
              <div className="inputWithAction">
                <input
                  type={showApiKey ? "text" : "password"}
                  value={draft.apiKey}
                  onChange={(e) => setDraft({ ...draft, apiKey: e.target.value })}
                  disabled={busy}
                  placeholder={draft.isNew ? "请填写 apiKey" : "保留 mask 占位则不更新"}
                  autoComplete="new-password"
                />
                <button
                  type="button"
                  className="inputActionBtn"
                  onClick={() => setShowApiKey((v) => !v)}
                  disabled={busy}
                  aria-label={showApiKey ? "隐藏 apiKey" : "显示 apiKey"}
                  title={showApiKey ? "隐藏" : "显示"}
                >
                  {showApiKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
              <small>编辑模式下若不修改请保持「****」mask 占位，提交时不会覆盖原 key</small>
            </label>
          </div>

          <div className="editorSectionTitle">重试与超时</div>
          <div className="formGrid formGrid--three">
            <label>
              <span>超时秒数</span>
              <input
                type="number"
                min={1}
                value={draft.timeoutSeconds}
                onChange={(e) => setDraft({ ...draft, timeoutSeconds: e.target.value })}
                disabled={busy}
                placeholder="默认沿用 .env"
              />
            </label>
            <label>
              <span>最大重试</span>
              <input
                type="number"
                min={0}
                value={draft.maxRetries}
                onChange={(e) => setDraft({ ...draft, maxRetries: e.target.value })}
                disabled={busy}
                placeholder="默认 3"
              />
            </label>
            <label>
              <span>重试退避基线 (ms)</span>
              <input
                type="number"
                min={1}
                value={draft.retryBaseMs}
                onChange={(e) => setDraft({ ...draft, retryBaseMs: e.target.value })}
                disabled={busy}
                placeholder="默认 1500"
              />
            </label>
          </div>

          <div className="editorSectionTitle">多模态能力</div>
          <div className="formGrid">
            <label className="spanFull checkboxRow">
              <input
                type="checkbox"
                checked={draft.supportsVision}
                onChange={(e) => setDraft({ ...draft, supportsVision: e.target.checked })}
                disabled={busy}
              />
              <span>支持图片输入（multimodal vision）</span>
            </label>
            <small className="spanFull">
              勾选后该模型可识别图片。若文字主模型不支持图片，可单独配置一条支持图片的模型，保存后在卡片上「设为视觉模型」——图片导入会自动路由到该视觉模型；否则图片导入返回 visionNotSupported。
            </small>
          </div>

          <div className="editorFooter">
            <button className="secondary" onClick={() => void runTest()} disabled={testing || busy}>
              <FlaskConical size={14} /> {testing ? "测试中…" : "测试连通性"}
            </button>
            <div className="footerSpacer" />
            <button className="secondary" onClick={cancelEdit} disabled={busy}>
              取消
            </button>
            <button onClick={() => void saveDraft()} disabled={busy}>
              {draft.isNew ? "创建" : "保存"}
            </button>
          </div>

          {testResult && (
            <div className={`testResult ${testResult.ok ? "ok" : "fail"}`}>
              <div className="testResultHead">
                <strong>{testResult.ok ? "测试成功" : "测试失败"}</strong>
                <span>耗时 {testResult.latencyMs} ms</span>
              </div>
              {testResult.ok ? (
                <pre className="testResultBody">
{typeof testResult.preview === "string"
  ? testResult.preview
  : JSON.stringify(testResult.preview, null, 2)}
                </pre>
              ) : (
                <div className="testResultBody">
                  <div>
                    <em>错误类型：</em>
                    {testResult.error?.kind || "unknown"}
                    {testResult.error?.retryCount != null && (
                      <span> · 重试 {testResult.error.retryCount} 次</span>
                    )}
                  </div>
                  {testResult.error?.detail && (
                    <pre>{testResult.error.detail}</pre>
                  )}
                  {testResult.error?.hint && (
                    <div className="hint">建议：{testResult.error.hint}</div>
                  )}
                </div>
              )}
            </div>
          )}
        </section>
      )}
    </section>
  );
}

function MetricCard({
  detail,
  label,
  onClick,
  value
}: {
  detail: string;
  label: string;
  onClick: () => void;
  value: number;
}) {
  return (
    <button className="metricCard" onClick={onClick}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{detail}</small>
    </button>
  );
}

function StatusLine({ label, tone, value }: { label: string; tone: "ai" | "good" | "neutral" | "warn"; value: string }) {
  return (
    <div className={`statusLine ${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function PlanStep({ detail, status, title }: { detail: string; status: "ready" | "pending"; title: string }) {
  return (
    <div className={`planStep ${status}`}>
      <CheckCircle2 size={16} />
      <div>
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
    </div>
  );
}

function EmptyInline({ text }: { text: string }) {
  return (
    <div className="emptyState">
      <MessageSquareText size={30} />
      <p>{text}</p>
    </div>
  );
}

function EmptyState({ icon, title, hint, action }: { icon?: React.ReactNode; title: string; hint?: string; action?: React.ReactNode }) {
  return (
    <div className="emptyStateRich">
      <div className="emptyStateRichIcon">{icon ?? <Inbox size={28} />}</div>
      <strong>{title}</strong>
      {hint && <p>{hint}</p>}
      {action && <div className="emptyStateRichAction">{action}</div>}
    </div>
  );
}

function TagChipInput({
  value,
  onChange,
  placeholder,
  max,
  disabled
}: {
  value: string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
  max?: number;
  disabled?: boolean;
}) {
  const [draft, setDraft] = useState("");
  const atMax = typeof max === "number" && value.length >= max;

  function commit(raw: string) {
    if (!raw.trim()) return;
    const tokens = raw
      .split(/[,，、\n]+/)
      .map((token) => token.trim())
      .filter(Boolean);
    if (!tokens.length) return;
    const next = [...value];
    for (const token of tokens) {
      if (typeof max === "number" && next.length >= max) break;
      if (!next.includes(token)) next.push(token);
    }
    onChange(next);
    setDraft("");
  }

  function handleKey(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter" || event.key === "," || event.key === "，" || event.key === "、") {
      event.preventDefault();
      commit(draft);
    } else if (event.key === "Backspace" && !draft && value.length) {
      onChange(value.slice(0, -1));
    }
  }

  function handlePaste(event: ClipboardEvent<HTMLInputElement>) {
    const text = event.clipboardData.getData("text");
    if (text && /[,，、\n]/.test(text)) {
      event.preventDefault();
      commit(text);
    }
  }

  function removeAt(index: number) {
    const next = value.slice();
    next.splice(index, 1);
    onChange(next);
  }

  return (
    <div className={`tagChipInput${disabled ? " disabled" : ""}${atMax ? " atMax" : ""}`}>
      {value.map((tag, index) => (
        <span key={`${tag}-${index}`} className="tagChip">
          {tag}
          <button
            type="button"
            aria-label={`删除 ${tag}`}
            onClick={() => removeAt(index)}
            disabled={disabled}
          >
            <X size={11} />
          </button>
        </span>
      ))}
      <input
        value={draft}
        placeholder={atMax ? `已达上限 ${max}` : value.length ? "" : placeholder || "回车 / 逗号添加"}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={handleKey}
        onPaste={handlePaste}
        onBlur={() => commit(draft)}
        disabled={disabled || atMax}
      />
    </div>
  );
}

function StructuredJson({
  data,
  defaultExpanded = 1,
  copyable = true
}: {
  data: unknown;
  defaultExpanded?: number;
  copyable?: boolean;
}) {
  const [raw, setRaw] = useState(false);
  if (data === null || data === undefined) {
    return <EmptyInline text="暂无数据" />;
  }
  if (raw) {
    return (
      <div className="structuredJson">
        <div className="structuredJsonToolbar">
          <button type="button" className="secondary compactButton" onClick={() => setRaw(false)}>
            返回结构化视图
          </button>
        </div>
        <pre className="jsonPreview">{JSON.stringify(data, null, 2)}</pre>
      </div>
    );
  }
  return (
    <div className="structuredJson">
      <div className="structuredJsonToolbar">
        {copyable && (
          <button
            type="button"
            className="secondary compactButton"
            onClick={() => {
              try {
                void navigator.clipboard?.writeText(JSON.stringify(data, null, 2));
              } catch {
                /* clipboard unavailable */
              }
            }}
          >
            <Copy size={12} /> 复制 JSON
          </button>
        )}
        <button type="button" className="secondary compactButton" onClick={() => setRaw(true)}>
          原始视图
        </button>
      </div>
      <div className="structuredJsonBody">
        <JsonNode value={data} depth={0} defaultExpanded={defaultExpanded} keyName={null} />
      </div>
    </div>
  );
}

function JsonNode({
  value,
  depth,
  defaultExpanded,
  keyName
}: {
  value: unknown;
  depth: number;
  defaultExpanded: number;
  keyName: string | null;
}) {
  const [open, setOpen] = useState(depth < defaultExpanded);

  if (value === null) {
    return <JsonLeaf keyName={keyName}><span className="jsonNull">null</span></JsonLeaf>;
  }
  if (typeof value === "boolean") {
    return <JsonLeaf keyName={keyName}><span className="jsonBool">{String(value)}</span></JsonLeaf>;
  }
  if (typeof value === "number") {
    return <JsonLeaf keyName={keyName}><span className="jsonNumber">{value}</span></JsonLeaf>;
  }
  if (typeof value === "string") {
    return <JsonStringLeaf keyName={keyName} value={value} />;
  }
  if (Array.isArray(value)) {
    if (!value.length) {
      return <JsonLeaf keyName={keyName}><span className="jsonMuted">[ ]</span></JsonLeaf>;
    }
    return (
      <div className="jsonNode">
        <button type="button" className="jsonNodeToggle" onClick={() => setOpen(!open)}>
          {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          {keyName !== null && <span className="jsonKey">{keyName}</span>}
          <span className="jsonMuted">[</span>
          <span className="jsonBadge">{value.length}</span>
          {!open && <span className="jsonMuted">…]</span>}
        </button>
        {open && (
          <div className="jsonChildren">
            {value.map((child, index) => (
              <JsonNode
                key={index}
                value={child}
                depth={depth + 1}
                defaultExpanded={defaultExpanded}
                keyName={String(index)}
              />
            ))}
            <span className="jsonMuted">]</span>
          </div>
        )}
      </div>
    );
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (!entries.length) {
      return <JsonLeaf keyName={keyName}><span className="jsonMuted">{"{ }"}</span></JsonLeaf>;
    }
    return (
      <div className="jsonNode">
        <button type="button" className="jsonNodeToggle" onClick={() => setOpen(!open)}>
          {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          {keyName !== null && <span className="jsonKey">{keyName}</span>}
          <span className="jsonMuted">{"{"}</span>
          <span className="jsonBadge">{entries.length}</span>
          {!open && <span className="jsonMuted">…{"}"}</span>}
        </button>
        {open && (
          <div className="jsonChildren">
            {entries.map(([k, v]) => (
              <JsonNode
                key={k}
                value={v}
                depth={depth + 1}
                defaultExpanded={defaultExpanded}
                keyName={k}
              />
            ))}
            <span className="jsonMuted">{"}"}</span>
          </div>
        )}
      </div>
    );
  }
  return <JsonLeaf keyName={keyName}><span>{String(value)}</span></JsonLeaf>;
}

function JsonLeaf({ keyName, children }: { keyName: string | null; children: React.ReactNode }) {
  return (
    <div className="jsonLeaf">
      {keyName !== null && <span className="jsonKey">{keyName}</span>}
      {children}
    </div>
  );
}

function JsonStringLeaf({ keyName, value }: { keyName: string | null; value: string }) {
  const [expanded, setExpanded] = useState(false);
  const long = value.length > 200;
  const display = !long || expanded ? value : `${value.slice(0, 200)}…`;
  return (
    <div className="jsonLeaf">
      {keyName !== null && <span className="jsonKey">{keyName}</span>}
      <span className="jsonString">{display}</span>
      {long && (
        <button type="button" className="jsonStringToggle" onClick={() => setExpanded(!expanded)}>
          {expanded ? "收起" : "展开"}
        </button>
      )}
    </div>
  );
}

function ConversationStream({ messages }: { messages: Message[] }) {
  if (!messages.length) {
    return <EmptyState icon={<MessageSquareText size={28} />} title="暂无会话记录" hint="一旦有用户消息或 AI 触达，对话会按时间在这里以左右气泡呈现。" />;
  }
  const items: React.ReactNode[] = [];
  let lastTime: number | null = null;
  for (const message of messages) {
    const ts = message.createdAt ? Date.parse(message.createdAt) : Number.NaN;
    if (!Number.isNaN(ts) && (lastTime === null || ts - lastTime > 30 * 60 * 1000)) {
      items.push(
        <div key={`sep-${message.id}`} className="bubbleSeparator">
          <span>{formatTime(message.createdAt)}</span>
        </div>
      );
    }
    if (!Number.isNaN(ts)) lastTime = ts;
    const isInbound = message.direction === "inbound";
    items.push(
      <div key={message.id} className={`bubbleRow ${isInbound ? "inbound" : "outbound"}`}>
        <div className="bubbleAvatar">
          {isInbound ? <User2 size={13} /> : <Bot size={13} />}
        </div>
        <div className="bubbleBody">
          <div className="bubble">
            <p>{message.content}</p>
          </div>
          <span className="bubbleMeta">{formatTime(message.createdAt)}</span>
        </div>
      </div>
    );
  }
  return <div className="conversationStream">{items}</div>;
}

function EventTimeline({
  items
}: {
  items: { id: string; tone: "ai" | "good" | "warn" | "error" | "neutral"; title: string; subtitle?: string; meta?: string; chips?: string[] }[];
}) {
  if (!items.length) {
    return <EmptyState icon={<Activity size={26} />} title="暂无事件" hint="跟进任务、Agent 决策与拦截会按时间在这里呈现。" />;
  }
  return (
    <ol className="eventTimeline">
      {items.map((item) => (
        <li key={item.id} className={`timelineItem tone-${item.tone}`}>
          <span className="timelineDot" />
          <div className="timelineCard">
            <div className="timelineHead">
              <strong>{item.title}</strong>
              {item.meta && <span>{item.meta}</span>}
            </div>
            {item.subtitle && <p>{item.subtitle}</p>}
            {item.chips && item.chips.length > 0 && (
              <div className="timelineChips">
                {item.chips.map((chip, idx) => (
                  <span key={`${chip}-${idx}`}>{chip}</span>
                ))}
              </div>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}

function channelTitle(channel: Channel) {
  switch (channel) {
    case "command":
      return "AI Command Center";
    case "overview":
      return "运营工作台";
    case "userOps":
      return "用户运营";
    case "groupOps":
      return "微信群运营";
    case "momentOps":
      return "朋友圈运营";
    case "content":
      return "内容资产";
    case "systemStrategy":
      return "系统策略";
    case "operations":
      return "任务与日志";
    case "autonomy":
      return "自治回路监控";
    case "evolution":
      return "演化中心";
    case "quality":
      return "运营成效";
    case "llmProviders":
      return "AI 模型配置";
  }
}

function channelEyebrow(channel: Channel) {
  switch (channel) {
    case "command":
      return "Management Agent";
    case "overview":
      return "System Overview";
    case "userOps":
      return "User Operations";
    case "groupOps":
      return "Group Operations";
    case "momentOps":
      return "Moment Operations";
    case "content":
      return "Knowledge Assets";
    case "systemStrategy":
      return "Global Prompt Policy";
    case "operations":
      return "Execution Audit";
    case "autonomy":
      return "Autonomy Loop";
    case "evolution":
      return "Self Evolution";
    case "quality":
      return "Outcome & Quality";
    case "llmProviders":
      return "LLM Providers";
  }
}

function channelSubtitle(channel: Channel) {
  switch (channel) {
    case "command":
      return "用一个后台管理 Agent 统筹好友、微信群、朋友圈与系统任务。";
    case "overview":
      return "查看微信账号、运营对象、任务和最近事件的整体状态。";
    case "userOps":
      return "围绕单个好友长期运营，维护用户画像、运营记忆、方法论、提示词和执行边界。";
    case "groupOps":
      return "下一阶段独立建设群画像、群节奏和群工具工作流。";
    case "momentOps":
      return "下一阶段独立建设朋友圈内容计划、发布队列和互动复盘。";
    case "content":
      return "维护产品资料、FAQ、话术、禁用表达、品牌语气和朋友圈素材。";
    case "systemStrategy":
      return "管理后台总控 Agent、方法论生成 Agent 和跨模块 Prompt Pack。";
    case "operations":
      return "追踪跟进任务、Agent 决策事件和系统执行结果。";
    case "autonomy":
      return "实时监控自治回路：修订触发率、AI 暂缓三类细分、未验证产品声明拦截、发送链路状态与最近修订记录。";
    case "evolution":
      return "查看自演化器产出的 experiments、阈值与 Prompt 候选、Shadow 评测与显著性结论；管理员二次确认后发布或回滚。";
    case "quality":
      return "用户回复率、对话深度等长期指标，知识切片自动校验，公式遵守度评测，产品声明兜底标记词管理。";
    case "llmProviders":
      return "管理 LLM 服务商：base_url / api_key / model / 协议格式（OpenAI 兼容、Anthropic 兼容）；支持测试连通性与一键热切换激活配置。";
  }
}

function emptyPlaybookDraft(): PlaybookDraft {
  return {
    name: "",
    description: "",
    methodPrompt: "",
    profileMethod: "",
    tagMethod: "",
    stageMethod: "",
    intentMethod: "",
    followUpMethod: "",
    replyStyle: "",
    forbiddenRules: "",
    successCriteria: "",
    isDefault: false
  };
}

function emptyPromptTemplateDraft(): PromptTemplateDraft {
  return {
    promptKey: "",
    agentKind: "user",
    layer: "task_template",
    title: "",
    description: "",
    content: ""
  };
}

function emptyDomainDraft(): OperationDomainDraft {
  return {
    name: "",
    goal: "",
    methodology: "",
    workflow: "",
    toolPolicy: "",
    automationPolicy: "",
    reviewPolicy: "",
    runtimeParameters: "",
    stateMachine: ""
  };
}

function domainDraftsFromConfigs(configs: OperationDomainConfig[]) {
  return Object.fromEntries(configs.map((config) => [config.domain, domainDraftFromConfig(config)]));
}

function domainDraftFromConfig(config: OperationDomainConfig): OperationDomainDraft {
  return {
    name: config.name,
    goal: config.goal,
    methodology: config.methodology,
    workflow: config.workflow,
    toolPolicy: config.toolPolicy,
    automationPolicy: config.automationPolicy,
    reviewPolicy: config.reviewPolicy,
    runtimeParameters: runtimeParametersText(config.runtimeParameters),
    stateMachine: jsonText(config.stateMachine)
  };
}

function operationDomainByKey(configs: OperationDomainConfig[], domain: DomainKey) {
  return configs.find((config) => config.domain === domain);
}

function domainPayload(draft: OperationDomainDraft) {
  return {
    name: draft.name,
    goal: draft.goal,
    methodology: draft.methodology,
    workflow: draft.workflow,
    toolPolicy: draft.toolPolicy,
    automationPolicy: draft.automationPolicy,
    reviewPolicy: draft.reviewPolicy,
    runtimeParameters: runtimeParametersFromText(draft.runtimeParameters),
    stateMachine: jsonFromText(draft.stateMachine)
  };
}

function jsonText(value: Record<string, unknown>) {
  if (!value || !Object.keys(value).length) return "";
  return JSON.stringify(value, null, 2);
}

function jsonFromText(text: string) {
  if (!text.trim()) return {};
  try {
    return JSON.parse(text) as Record<string, unknown>;
  } catch {
    return {};
  }
}

function runtimeParametersText(value: Record<string, unknown>) {
  return orderedRuntimeParameters(value || {})
    .map(([key, item]) => `${key} = ${String(item)}`)
    .join("\n");
}

function runtimeParametersFromText(text: string) {
  return Object.fromEntries(
    text
      .split(/\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const [rawKey, ...rest] = line.split("=");
        const rawValue = rest.join("=").trim();
        return [rawKey.trim(), parseParameterValue(rawValue)];
      })
      .filter(([key]) => key)
  );
}

function parseParameterValue(value: string) {
  if (value === "true") return true;
  if (value === "false") return false;
  if (value && !Number.isNaN(Number(value))) return Number(value);
  return value;
}

function orderedRuntimeParameters(value: Record<string, unknown>) {
  const knownKeys = new Set(USER_RUNTIME_PARAMETER_FIELDS.map((field) => field.key));
  const known = USER_RUNTIME_PARAMETER_FIELDS
    .filter((field) => value[field.key] !== undefined)
    .map((field) => [field.key, value[field.key]] as [string, unknown]);
  const rest = Object.entries(value).filter(([key]) => !knownKeys.has(key));
  return [...known, ...rest];
}

function runtimeParametersWithValue(text: string, key: string, value: string | boolean) {
  const params = runtimeParametersFromText(text);
  params[key] = typeof value === "boolean" ? value : parseParameterValue(value);
  return runtimeParametersText(params);
}

function stateMachineStates(text: string): OperationStateDraft[] {
  const machine = jsonFromText(text);
  const states = Array.isArray(machine.states) ? machine.states : [];
  return states
    .filter((item): item is Record<string, unknown> => Boolean(item) && typeof item === "object" && !Array.isArray(item))
    .map((state) => ({
      raw: state,
      key: stringField(state, "key"),
      name: stringField(state, "name"),
      goal: stringField(state, "goal"),
      allowedActions: listField(state, "allowedActions"),
      allowedFrom: listField(state, "allowedFrom"),
      allowFromAny: Boolean(state.allowFromAny),
      advanceSignals: listField(state, "advanceSignals"),
      cooldownSignals: listField(state, "cooldownSignals"),
      riskRules: listField(state, "riskRules"),
      successCriteria: listField(state, "successCriteria")
    }))
    .filter((state) => state.key);
}

function stateMachineWithValue(text: string, key: string, field: keyof OperationStateDraft, value: string | boolean) {
  const machine = jsonFromText(text);
  const states = stateMachineStates(text).map((state) => {
    if (state.key !== key) return state;
    return { ...state, [field]: value };
  });
  return JSON.stringify(
    {
      ...machine,
      states: states.map((state) => ({
        ...state.raw,
        key: state.key,
        name: state.name,
        goal: state.goal,
        allowedActions: splitTags(state.allowedActions),
        allowedFrom: state.allowFromAny ? [] : splitTags(state.allowedFrom),
        allowFromAny: state.allowFromAny,
        advanceSignals: splitTags(state.advanceSignals),
        cooldownSignals: splitTags(state.cooldownSignals),
        riskRules: splitTags(state.riskRules),
        successCriteria: splitTags(state.successCriteria)
      }))
    },
    null,
    2
  );
}

function agentKindLabel(kind: string) {
  const labels: Record<string, string> = {
    user: "用户运营",
    management: "后台管理",
    methodology: "方法论生成",
    group: "微信群运营",
    moment: "朋友圈运营"
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

function emptyMemoryDraft(): OperatingMemoryDraft {
  return {
    identity: "",
    businessContext: "",
    jobsToBeDone: "",
    painPoints: "",
    motivations: "",
    decisionStyle: "",
    communicationPreference: "",
    sensitivePoints: "",
    trustLevel: "",
    temperature: "",
    lastEmotion: "",
    relationshipGoal: "",
    doNotDo: "",
    interestedProducts: "",
    fitReason: "",
    objections: "",
    riskPoints: "",
    unknowns: "",
    nextGoal: "",
    recommendedMove: "",
    avoid: "",
    timing: "",
    reason: ""
  };
}

function draftFromMemory(memory: OperatingMemory): OperatingMemoryDraft {
  const user = memory.userUnderstanding || {};
  const relation = memory.relationshipState || {};
  const fit = memory.productFit || {};
  const next = memory.nextAction || {};
  return {
    identity: stringField(user, "identity"),
    businessContext: stringField(user, "businessContext"),
    jobsToBeDone: listField(user, "jobsToBeDone"),
    painPoints: listField(user, "painPoints"),
    motivations: listField(user, "motivations"),
    decisionStyle: stringField(user, "decisionStyle"),
    communicationPreference: stringField(user, "communicationPreference"),
    sensitivePoints: listField(user, "sensitivePoints"),
    trustLevel: stringField(relation, "trustLevel"),
    temperature: stringField(relation, "temperature"),
    lastEmotion: stringField(relation, "lastEmotion"),
    relationshipGoal: stringField(relation, "relationshipGoal"),
    doNotDo: listField(relation, "doNotDo"),
    interestedProducts: listField(fit, "interestedProducts"),
    fitReason: stringField(fit, "fitReason"),
    objections: listField(fit, "objections"),
    riskPoints: listField(fit, "riskPoints"),
    unknowns: listField(fit, "unknowns"),
    nextGoal: stringField(next, "goal"),
    recommendedMove: stringField(next, "recommendedMove"),
    avoid: stringField(next, "avoid"),
    timing: stringField(next, "timing"),
    reason: stringField(next, "reason")
  };
}

function playbookPayload(draft: PlaybookDraft) {
  return {
    name: draft.name,
    description: draft.description || undefined,
    methodPrompt: draft.methodPrompt,
    profileMethod: draft.profileMethod || undefined,
    tagMethod: draft.tagMethod || undefined,
    stageMethod: draft.stageMethod || undefined,
    intentMethod: draft.intentMethod || undefined,
    followUpMethod: draft.followUpMethod || undefined,
    replyStyle: draft.replyStyle || undefined,
    forbiddenRules: draft.forbiddenRules || undefined,
    successCriteria: draft.successCriteria || undefined,
    isDefault: draft.isDefault
  };
}

function promptPayload(draft: PromptTemplateDraft) {
  return {
    promptKey: draft.promptKey,
    agentKind: draft.agentKind,
    layer: draft.layer,
    title: draft.title,
    description: draft.description || undefined,
    content: draft.content
  };
}

function stringField(source: Record<string, unknown>, key: string) {
  const value = source[key];
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.join(", ");
  if (value === undefined || value === null) return "";
  return String(value);
}

function listField(source: Record<string, unknown>, key: string) {
  const value = source[key];
  if (Array.isArray(value)) return value.map(String).join(", ");
  if (typeof value === "string") return value;
  return "";
}

function contextPackList(source: Record<string, unknown> | undefined, key: string): string[] {
  const value = source?.[key];
  if (Array.isArray(value)) {
    return value
      .map((item) => {
        if (typeof item === "string") return item;
        if (item && typeof item === "object") return formatChangeValue(item);
        return String(item);
      })
      .filter(Boolean);
  }
  if (typeof value === "string" && value.trim()) return [value.trim()];
  return [];
}

function memoryStatusLabel(status: string) {
  if (status === "pending") return "待整理";
  if (status === "consolidated") return "已入库";
  if (status === "ignored_low_score") return "低价值忽略";
  return status || "未知";
}

function memoryCandidateText(candidate: Record<string, unknown>) {
  const type = stringField(candidate, "type");
  const content = stringField(candidate, "content");
  const evidence = stringField(candidate, "evidence");
  return [type, content, evidence ? `依据：${evidence}` : ""].filter(Boolean).join(" · ");
}

function splitTags(value: string) {
  return value
    .split(/[,，\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function splitLines(value: string) {
  return value
    .split(/\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}


function eventTone(status?: string): "ai" | "good" | "warn" | "error" | "neutral" {
  const s = (status || "").toLowerCase();
  if (!s) return "neutral";
  if (s.includes("success") || s === "ok" || s === "approved" || s.includes("done")) return "good";
  if (s.includes("fail") || s.includes("error") || s.includes("blocked") || s.includes("rejected")) return "error";
  if (s.includes("warn") || s.includes("hold") || s.includes("pending") || s.includes("waiting")) return "warn";
  if (s.includes("ai") || s.includes("agent")) return "ai";
  return "neutral";
}

function formatScores(scores: Record<string, number>) {
  // S1.4 (Phase 0)：与 src/agent/types.rs::ReviewScores 同源——仅展示当前
  // 三闸 + 软闸字段，移除 productAccuracy / factRisk 五老评分键。
  const keys = ["humanLike", "emotionalValue", "hallucinationScore", "knowledgeGroundingScore", "pressureRisk"];
  return keys
    .filter((key) => scores[key] !== undefined)
    .map((key) => `${key}:${scores[key]}`)
    .join(" / ") || "-";
}

function simulationStatusLabel(status: string) {
  const labels: Record<string, string> = {
    would_send: "会发送",
    no_reply: "不回复",
    review_blocked: "复盘拦截",
    gateway_blocked: "网关拦截"
  };
  return labels[status] || status || "未知";
}

function nextBestActionLabel(action?: Record<string, unknown>) {
  if (!action) return "-";
  const type = typeof action.type === "string" ? action.type : "-";
  const score = typeof action.score === "number" ? ` / ${action.score}` : "";
  return `${type}${score}`;
}

function defaultHealthItems(): OperationHealthItem[] {
  return [
    { key: "userUnderstanding", label: "用户理解完整度", score: 0, tone: "warn", detail: "选择好友后自动诊断。" },
    { key: "relationshipQuality", label: "信任关系质量", score: 0, tone: "warn", detail: "选择好友后自动诊断。" },
    { key: "productFit", label: "产品匹配清晰度", score: 0, tone: "warn", detail: "选择好友后自动诊断。" }
  ];
}

function healthFromScores(scores: Record<string, unknown>): OperationHealth {
  const labels: Record<string, string> = {
    userUnderstanding: "用户理解完整度",
    relationshipQuality: "信任关系质量",
    productFit: "产品匹配清晰度",
    rhythmRisk: "跟进节奏风险",
    knowledgeGrounding: "知识匹配度",
    hallucinationRisk: "幻觉风险"
  };
  const items = Object.entries(labels).map(([key, label]) => {
    const score = typeof scores[key] === "number" ? scores[key] : Number(scores[key] || 0);
    const risk = key.endsWith("Risk");
    const tone = risk
      ? score >= 70 ? "danger" : score >= 40 ? "warn" : "good"
      : score >= 75 ? "good" : score >= 45 ? "warn" : "danger";
    return {
      key,
      label,
      score,
      tone,
      detail: risk ? "分数越高风险越高。" : "分数越高代表越充分。"
    } as OperationHealthItem;
  });
  return { scores: Object.fromEntries(items.map((item) => [item.key, item.score])), items };
}

function readableChangeItems(changes: Record<string, unknown>) {
  const labels: Record<string, string> = {
    humanProfileNote: "运营备注",
    tags: "用户标签",
    domainAttributes: "领域属性",
    followUpPolicy: "跟进策略",
    operationState: "运营状态",
    operationStateReason: "状态原因",
    memory: "运营记忆",
    playbookPatch: "整体方法论",
    domainRuntimeParameters: "运行参数"
  };
  return Object.entries(changes || {})
    .filter(([, value]) => value !== undefined && value !== null && String(value).trim() !== "" && !(Array.isArray(value) && value.length === 0))
    .map(([key, value]) => ({
      label: labels[key] || key,
      value: formatChangeValue(value)
    }));
}

function impactScopeLabel(scope?: string) {
  if (scope === "all_user_operations") return "影响所有用户运营";
  if (scope === "agent_personality") return "影响 Agent 整体人格";
  return "只影响当前好友";
}

function formatChangeValue(value: unknown): string {
  if (Array.isArray(value)) return value.map(String).join(" / ");
  if (value && typeof value === "object") {
    return Object.entries(value as Record<string, unknown>)
      .map(([key, item]) => `${key}: ${formatChangeValue(item)}`)
      .join("；");
  }
  return String(value);
}

function commandCallDetail(call: CommandToolCall): string {
  if (call.error) return call.error;
  const response = call.response || {};
  // 波 B1：dry-run 时后端返回 { dry_run: true, would_execute: { toolName, arguments } }；
  // 把 would_execute 摘要打到 detail 中，方便运营在不真实触达的情况下确认计划。
  if (response.dry_run === true || call.status === "dry_run") {
    const would = response.would_execute as Record<string, unknown> | undefined;
    if (would) {
      const args = would.arguments as Record<string, unknown> | undefined;
      const errorField = would.error as string | undefined;
      const content = args && typeof args.content === "string" ? args.content : undefined;
      const tool = (would.toolName as string | undefined) || call.toolName;
      const summary = [
        `演练：${tool}`,
        content ? `content="${content.slice(0, 60)}"` : "",
        errorField ? `error=${errorField}` : ""
      ].filter(Boolean).join(" · ");
      return summary || `演练：${tool}（不真实执行）`;
    }
    return "演练模式：未实际调用工具";
  }
  const sentContent = response.sentContent;
  const messageId = response.messageId;
  const reviewApproved = response.reviewApproved;
  const gatewayStatus = response.gatewayStatus;
  const gatewayReason = response.gatewayReason;
  if (typeof sentContent === "string" && sentContent.trim()) {
    return [
      `实际发送：${sentContent}`,
      gatewayStatus ? `网关：${String(gatewayStatus)}` : "",
      reviewApproved !== undefined ? `Review：${reviewApproved ? "通过" : "未通过"}` : "",
      messageId ? `messageId：${String(messageId)}` : "",
      gatewayReason ? `原因：${String(gatewayReason)}` : ""
    ].filter(Boolean).join(" · ");
  }
  return call.status;
}

function formatTime(value?: string) {
  if (!value) return "-";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}


// =============================================================================
// 波 B4-B6：运营成效中心
// 集中暴露 outcome metrics（B3 风格 null 友好）/ knowledge auto-verify 触发器（B4）/
// formula adherence 评测看板（B5）/ product claim markers JSON 编辑器（B6）。
// 单文件实现，避免拆出新模块导致的工程量膨胀。
// =============================================================================

type OutcomeMetric = {
  id: string;
  accountId: string;
  horizon: string;
  date: string;
  replyRate: number | null;
  conversationDepth: number | null;
  aiHoldClearedRate: number | null;
  agentBlockRate: number | null;
  dailyRunCount: number;
  dailyRunTokenTotal: number;
};

type FormulaItem = {
  scenarioId: string;
  title?: string;
  predicted?: Record<string, number | null>;
  groundTruth?: Record<string, number>;
  deviations?: Record<string, number | string>;
  adherenceScore?: number;
  invalid?: boolean;
  invalidReason?: string;
  missingFormulas?: number;
  skipped?: boolean;
  reason?: string;
  error?: string;
};

type FormulaSummary = {
  degraded: boolean;
  degradedReason?: string | null;
  scenarioCount: number;
  meanAdherence: number;
  totalTokensUsed?: number;
  totalTokenBudget?: number;
  processedBeforeBudgetExceeded?: number;
  reason?: string;
};

type AutoVerifyResult = {
  processed: number;
  verified: number;
  needsReview: number;
  rejected: number;
  needsHumanAudit: number;
  degraded: boolean;
  budget?: Record<string, unknown>;
};

type PromptTemplateLite = {
  id: string;
  promptKey: string;
  status: string;
  version: number;
  content: string;
};

function AutonomyLoopView({ accountId }: { accountId?: string }) {
  return (
    <section className="qualityCenter">
      <div className="panelHead compact">
        <div>
          <span>Autonomy Loop</span>
          <h2>自治回路监控</h2>
        </div>
        <ShieldCheck size={18} />
      </div>
      <AutonomyOutcomesTab accountId={accountId} />
    </section>
  );
}

// M4 W4 / Task 5.8：演化中心顶级频道。读 /api/health 的 evolutionEnabled 决定渲染
// 真正的 EvolutionCenterTab 还是占位文案（演化器未启用时 worker 不 tick，experiments
// 也会一直为空，但占位文案能让运营更明确知道原因）。
function EvolutionCenterView() {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/api/health")
      .then((r) => (r.ok ? r.json() : { evolutionEnabled: false }))
      .then((d: { evolutionEnabled?: boolean }) => {
        if (!cancelled) setEnabled(d.evolutionEnabled === true);
      })
      .catch(() => {
        if (!cancelled) setEnabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="qualityCenter">
      <div className="panelHead compact">
        <div>
          <span>Self Evolution</span>
          <h2>演化中心</h2>
        </div>
        <ShieldCheck size={18} />
      </div>
      {enabled === null ? (
        <div className="evolutionEmpty">加载中…</div>
      ) : (
        <EvolutionCenterTab enabled={enabled} />
      )}
    </section>
  );
}

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

function KnowledgeWikiView() {
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
  const [pane, setPane] = useState<"lint" | "review" | "revisions" | "documents" | "import" | "ingest" | "observability" | "tryRecall">("lint");
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
    window.addEventListener("wikiFocusChunk", onFocus as EventListener);
    return () => window.removeEventListener("wikiFocusChunk", onFocus as EventListener);
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
        {pane === "lint" && <LintView />}
        {pane === "review" && <ReviewView />}
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

// ── P1 · ChunkGraphView · 关系图谱（SVG 原生布局，0 新依赖）─────────────
//
// 数据：GET /api/operation-knowledge/chunks → 每条 chunk 一个节点。
// 边来源：
//   - relatedChunks: { chunk_id, kind } 6 类（references / requires / contradicts / clarifies / refines / superseded_by）
//   - supersededBy:  归档链尾 → 现役新版本（隐式 superseded_by）
//   - previousVersionId: split/merge/rollback 维护的前一版指针
//
// 两种布局模式：
//   1) polar（确定性）：按 wikiType 分扇区，扇区内按 id 哈希分布角度，
//      半径按"被引用次数" 微调（核心 chunk 向中心收）。0 抖动。
//   2) force（力导向）：以 polar 为初始解 → 200 步 spring + 排斥力迭代，
//      时间步逐步降温（dt *= 0.99）。同步算完一次 setLayout，无每帧重排。
//
// 颜色：默认按 wikiType；切到"社区"模式时用并查集找连通分量，按 component
// 索引分配 HSL 等距色环。
//
// 交互：节点 click → focusChunk(id)；hover → 浮窗显示 title + wikiType。
function ChunkGraphView() {
  const [items, setItems] = useState<TreeChunkItem[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>("all"); // wikiType filter
  const [layoutMode, setLayoutMode] = useState<"polar" | "force">("polar");
  const [colorMode, setColorMode] = useState<"wikiType" | "community">("wikiType");

  useEffect(() => {
    setLoading(true);
    fetch("/api/operation-knowledge/chunks")
      .then(async (r) => {
        if (!r.ok) throw await parseApiError(r);
        return r.json() as Promise<{ items: TreeChunkItem[] }>;
      })
      .then((data) => setItems(data.items ?? []))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, []);

  const wikiTypes = useMemo(() => {
    if (!items) return [] as string[];
    const set = new Set<string>();
    for (const it of items) if (it.wikiType) set.add(it.wikiType);
    return Array.from(set).sort();
  }, [items]);

  const visible = useMemo(() => {
    if (!items) return [] as TreeChunkItem[];
    if (filter === "all") return items;
    return items.filter((it) => it.wikiType === filter);
  }, [items, filter]);

  const indexById = useMemo(() => {
    const m = new Map<string, TreeChunkItem>();
    for (const it of visible) m.set(it.id, it);
    return m;
  }, [visible]);

  // FNV-1a 32-bit：deterministic id → 0..1 用作角度噪声。
  const hash01 = (s: string): number => {
    let h = 0x811c9dc5;
    for (let i = 0; i < s.length; i += 1) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    return ((h >>> 0) % 100000) / 100000;
  };

  // 入度（被引用次数）— 半径压缩量。
  const inDegree = useMemo(() => {
    const m = new Map<string, number>();
    for (const it of visible) {
      if (it.relatedChunks) {
        for (const r of it.relatedChunks) {
          m.set(r.chunk_id, (m.get(r.chunk_id) ?? 0) + 1);
        }
      }
      if (it.supersededBy) m.set(it.supersededBy, (m.get(it.supersededBy) ?? 0) + 1);
      if (it.previousVersionId) m.set(it.previousVersionId, (m.get(it.previousVersionId) ?? 0) + 1);
    }
    return m;
  }, [visible]);

  // 边渲染：按 kind 决定线条样式。
  type Edge = { from: string; to: string; kind: string };
  const edges: Edge[] = useMemo(() => {
    const out: Edge[] = [];
    const idSet = new Set(visible.map((it) => it.id));
    for (const it of visible) {
      if (it.relatedChunks) {
        for (const r of it.relatedChunks) {
          if (idSet.has(r.chunk_id)) out.push({ from: it.id, to: r.chunk_id, kind: r.kind });
        }
      }
      if (it.supersededBy && idSet.has(it.supersededBy)) {
        out.push({ from: it.id, to: it.supersededBy, kind: "superseded_by" });
      }
      if (it.previousVersionId && idSet.has(it.previousVersionId)) {
        out.push({ from: it.id, to: it.previousVersionId, kind: "previous_version" });
      }
    }
    return out;
  }, [visible]);

  // 社区检测：把节点 + 边喂并查集，输出 nodeId → componentIdx。
  // 同 component 节点同色（用于"社区染色"模式）。
  const community = useMemo(() => {
    const parent = new Map<string, string>();
    const find = (x: string): string => {
      let p = parent.get(x) ?? x;
      if (p === x) return x;
      const root = find(p);
      parent.set(x, root);
      return root;
    };
    const union = (a: string, b: string) => {
      const ra = find(a);
      const rb = find(b);
      if (ra !== rb) parent.set(ra, rb);
    };
    for (const it of visible) parent.set(it.id, it.id);
    for (const e of edges) {
      if (parent.has(e.from) && parent.has(e.to)) union(e.from, e.to);
    }
    const idxByRoot = new Map<string, number>();
    const result = new Map<string, number>();
    for (const it of visible) {
      const root = find(it.id);
      let idx = idxByRoot.get(root);
      if (idx === undefined) {
        idx = idxByRoot.size;
        idxByRoot.set(root, idx);
      }
      result.set(it.id, idx);
    }
    return { byId: result, count: idxByRoot.size };
  }, [visible, edges]);

  // 计算节点坐标。layoutMode 决定 polar / force。
  const layout = useMemo(() => {
    const W = 720;
    const H = 560;
    const cx = W / 2;
    const cy = H / 2;
    const types = wikiTypes.length ? wikiTypes : ["__none"];
    const sectorByType = new Map<string, number>();
    types.forEach((t, i) => sectorByType.set(t, i));
    const sectorWidth = (Math.PI * 2) / types.length;
    const positions = new Map<string, { x: number; y: number }>();
    for (const it of visible) {
      const t = it.wikiType ?? "__none";
      const sector = sectorByType.get(t) ?? 0;
      const noise = hash01(it.id);
      const angle = sector * sectorWidth + noise * sectorWidth * 0.92 + sectorWidth * 0.04;
      const deg = inDegree.get(it.id) ?? 0;
      const radius = 230 - Math.min(deg, 8) * 18;
      positions.set(it.id, {
        x: cx + Math.cos(angle) * radius,
        y: cy + Math.sin(angle) * radius
      });
    }

    if (layoutMode === "force" && visible.length > 0) {
      // 200 步弹簧 + 排斥力迭代。常量在 100-500 节点规模上调过：
      //   k_spring = 0.06     边 → 拉近
      //   rest_len = 80       边目标长度
      //   k_repel  = 1400     节点间 → 推开
      //   dt0      = 0.5      每步位移系数；逐步退火 dt *= 0.99
      //   bounds   = 边界回拉，避免节点飞出 viewBox
      const ids = visible.map((it) => it.id);
      const adj = new Map<string, Set<string>>();
      for (const id of ids) adj.set(id, new Set());
      for (const e of edges) {
        adj.get(e.from)?.add(e.to);
        adj.get(e.to)?.add(e.from);
      }
      const k_spring = 0.06;
      const rest_len = 80;
      const k_repel = 1400;
      let dt = 0.5;
      const padding = 40;
      for (let step = 0; step < 200; step += 1) {
        const fx = new Map<string, number>();
        const fy = new Map<string, number>();
        for (const id of ids) {
          fx.set(id, 0);
          fy.set(id, 0);
        }
        // 排斥力 O(N²)
        for (let i = 0; i < ids.length; i += 1) {
          const a = positions.get(ids[i])!;
          for (let j = i + 1; j < ids.length; j += 1) {
            const b = positions.get(ids[j])!;
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            let dist2 = dx * dx + dy * dy;
            if (dist2 < 1) dist2 = 1;
            const dist = Math.sqrt(dist2);
            const force = k_repel / dist2;
            dx /= dist;
            dy /= dist;
            fx.set(ids[i], (fx.get(ids[i]) ?? 0) + dx * force);
            fy.set(ids[i], (fy.get(ids[i]) ?? 0) + dy * force);
            fx.set(ids[j], (fx.get(ids[j]) ?? 0) - dx * force);
            fy.set(ids[j], (fy.get(ids[j]) ?? 0) - dy * force);
          }
        }
        // 弹簧力
        for (const e of edges) {
          const a = positions.get(e.from);
          const b = positions.get(e.to);
          if (!a || !b) continue;
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const dist = Math.sqrt(dx * dx + dy * dy) || 1;
          const f = k_spring * (dist - rest_len);
          const ux = dx / dist;
          const uy = dy / dist;
          fx.set(e.from, (fx.get(e.from) ?? 0) + ux * f);
          fy.set(e.from, (fy.get(e.from) ?? 0) + uy * f);
          fx.set(e.to, (fx.get(e.to) ?? 0) - ux * f);
          fy.set(e.to, (fy.get(e.to) ?? 0) - uy * f);
        }
        // 应用位移 + 边界回拉
        for (const id of ids) {
          const p = positions.get(id)!;
          let nx = p.x + (fx.get(id) ?? 0) * dt;
          let ny = p.y + (fy.get(id) ?? 0) * dt;
          if (nx < padding) nx = padding;
          if (nx > W - padding) nx = W - padding;
          if (ny < padding) ny = padding;
          if (ny > H - padding) ny = H - padding;
          positions.set(id, { x: nx, y: ny });
        }
        dt *= 0.99;
      }
    }

    return { positions, W, H, cx, cy };
  }, [visible, wikiTypes, inDegree, edges, layoutMode]);

  // 颜色：wikiType 模式按 token 色板；community 模式按 component 索引等距分布 HSL。
  const colorFor = (it: TreeChunkItem): string => {
    if (colorMode === "community") {
      const idx = community.byId.get(it.id) ?? 0;
      const total = Math.max(community.count, 1);
      const hue = Math.round((idx * 360) / total);
      return `hsl(${hue}, 48%, 42%)`;
    }
    const palette = [
      "#7a4a30",
      "#3d6a52",
      "#5a4d8a",
      "#8a6a3a",
      "#3d6a8a",
      "#8a3a5a",
      "#3a6a3a",
      "#6a3a3a"
    ];
    if (!it.wikiType) return "#888";
    const h = hash01(it.wikiType);
    return palette[Math.floor(h * palette.length)] ?? palette[0];
  };

  // 图例颜色（只取 wikiType；community 模式下只显示组数）。
  const legendColorFor = (wikiType: string): string => {
    const palette = [
      "#7a4a30",
      "#3d6a52",
      "#5a4d8a",
      "#8a6a3a",
      "#3d6a8a",
      "#8a3a5a",
      "#3a6a3a",
      "#6a3a3a"
    ];
    const h = hash01(wikiType);
    return palette[Math.floor(h * palette.length)] ?? palette[0];
  };

  const focused = hovered ? indexById.get(hovered) : null;

  if (loading) return <div className="wikiInspectorEmpty">加载中…</div>;
  if (error) return <div className="wikiAlert error">{error}</div>;
  if (!visible.length) return <div className="wikiInspectorEmpty">无 chunk 可绘图</div>;

  return (
    <div className="wikiGraphPane">
      <header className="wikiArchiveHeader">
        <h2>关系图谱</h2>
        <div className="wikiArchiveSubtitle">
          {visible.length} chunks · {edges.length} edges{filter !== "all" ? ` · 过滤 ${filter}` : ""}
        </div>
      </header>
      <div className="wikiGraphToolbar">
        <label className="wikiGraphFilterLabel">wiki_type：</label>
        <select
          className="wikiGraphFilterSelect"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        >
          <option value="all">全部</option>
          {wikiTypes.map((t) => (
            <option key={t} value={t}>{t}</option>
          ))}
        </select>
        <label className="wikiGraphFilterLabel">布局：</label>
        <select
          className="wikiGraphFilterSelect"
          value={layoutMode}
          onChange={(e) => setLayoutMode(e.target.value as "polar" | "force")}
        >
          <option value="polar">极坐标（确定性）</option>
          <option value="force">力导向（200 步）</option>
        </select>
        <label className="wikiGraphFilterLabel">染色：</label>
        <select
          className="wikiGraphFilterSelect"
          value={colorMode}
          onChange={(e) => setColorMode(e.target.value as "wikiType" | "community")}
        >
          <option value="wikiType">按 wiki_type</option>
          <option value="community">按社区（{community.count} 组）</option>
        </select>
        <span className="wikiGraphLegend">
          {colorMode === "wikiType"
            ? wikiTypes.slice(0, 8).map((t) => (
                <span key={t} className="wikiGraphLegendItem">
                  <span className="wikiGraphLegendDot" style={{ background: legendColorFor(t) }} />
                  {t}
                </span>
              ))
            : (
                <span className="wikiGraphCommunityHint">
                  {community.count} 个连通分量 · 颜色按分量索引等距分布
                </span>
              )}
        </span>
      </div>
      <div className="wikiGraphSvgWrap">
        <svg
          viewBox={`0 0 ${layout.W} ${layout.H}`}
          className="wikiGraphSvg"
          xmlns="http://www.w3.org/2000/svg"
        >
          <defs>
            <marker
              id="wikiGraphArrow"
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="5"
              markerHeight="5"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" fill="#666" />
            </marker>
          </defs>
          <g className="wikiGraphEdges">
            {edges.map((e, i) => {
              const a = layout.positions.get(e.from);
              const b = layout.positions.get(e.to);
              if (!a || !b) return null;
              const stroke = e.kind === "contradicts"
                ? "#a13a3a"
                : e.kind === "superseded_by"
                ? "#7a4a30"
                : e.kind === "previous_version"
                ? "#5a4d8a"
                : "#999";
              const dash = e.kind === "contradicts" || e.kind === "superseded_by" ? "4 3" : undefined;
              const isHovered = hovered === e.from || hovered === e.to;
              return (
                <line
                  key={`${e.from}-${e.to}-${i}`}
                  x1={a.x}
                  y1={a.y}
                  x2={b.x}
                  y2={b.y}
                  stroke={stroke}
                  strokeWidth={isHovered ? 2 : 1}
                  strokeOpacity={isHovered ? 0.85 : 0.35}
                  strokeDasharray={dash}
                  markerEnd="url(#wikiGraphArrow)"
                />
              );
            })}
          </g>
          <g className="wikiGraphNodes">
            {visible.map((it) => {
              const p = layout.positions.get(it.id);
              if (!p) return null;
              const deg = inDegree.get(it.id) ?? 0;
              const r = 5 + Math.min(deg, 6) * 1.5;
              const fill = colorFor(it);
              const isHovered = hovered === it.id;
              const isArchived = it.status === "archived";
              return (
                <g
                  key={it.id}
                  transform={`translate(${p.x},${p.y})`}
                  className="wikiGraphNode"
                  onMouseEnter={() => setHovered(it.id)}
                  onMouseLeave={() => setHovered(null)}
                  onClick={() => focusChunk(it.id)}
                  style={{ cursor: "pointer" }}
                >
                  <circle
                    r={r}
                    fill={isArchived ? "#fff" : fill}
                    stroke={fill}
                    strokeWidth={isHovered ? 2.5 : isArchived ? 1.5 : 1}
                    opacity={isArchived ? 0.7 : 1}
                  />
                  {isHovered ? (
                    <text
                      x={r + 4}
                      y={4}
                      fontFamily="var(--font-mono)"
                      fontSize="11"
                      fill="var(--ink, #222)"
                    >
                      {it.title?.slice(0, 28) || it.id.slice(0, 8)}
                    </text>
                  ) : null}
                </g>
              );
            })}
          </g>
        </svg>
        {focused ? (
          <div className="wikiGraphTooltip">
            <div className="wikiGraphTooltipTitle">{focused.title || "（无标题）"}</div>
            <div className="wikiGraphTooltipMeta">
              <span className="wikiArchiveTag">{focused.wikiType ?? "—"}</span>
              <span className="wikiBadge">{focused.status ?? "—"}</span>
              <span className="wikiGraphTooltipDeg">入度 {inDegree.get(focused.id) ?? 0}</span>
            </div>
          </div>
        ) : null}
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
            ✓ 已写入 {created.length} 条草稿 chunk
          </div>
          <p style={{ color: "var(--muted)", fontSize: 12.5 }}>
            所有 chunk 处于 draft + needs_review 状态，进入「待评审」面板逐条 verify。
          </p>
          <div style={{ marginTop: 8, display: "grid", gap: 4 }}>
            {created.map((id) => (
              <button key={id} type="button" className="wikiSignalChunkBtn" onClick={() => focusChunk(id)}>
                <code>{id}</code>
              </button>
            ))}
          </div>
          <div style={{ marginTop: 14 }}>
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

interface DomainSchemaField {
  name: string;
  label: string;
  kind: string;
  required: boolean;
  allowedValues?: string[] | null;
  aliasOf?: string | null;
}
interface DomainSchemaItem {
  schemaId: string;
  workspaceId: string;
  name: string;
  version: number;
  fields: DomainSchemaField[];
  aliasDict: Record<string, unknown>;
  guardDsl?: string | null;
  isActive: boolean;
  createdAt: number;
  updatedAt: number;
}

function DomainSchemaTab() {
  const [items, setItems] = useState<DomainSchemaItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [activating, setActivating] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/admin/domain-schemas");
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: DomainSchemaItem[] };
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

  async function activate(schemaId: string) {
    setActivating(schemaId);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(`/api/admin/domain-schemas/${encodeURIComponent(schemaId)}/activate`, {
        method: "POST",
      });
      if (!r.ok) throw await parseApiError(r);
      setInfo(`已切换 active：${schemaId}`);
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setActivating(null);
    }
  }

  return (
    <div className="wikiPanelBody">
      <div className="wikiToolbar">
        <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} />
          {loading ? "加载中…" : "刷新"}
        </button>
        <span className="wikiHint">
          新建 / 编辑 schema 走后端 API（POST/PUT /api/admin/domain-schemas）；UI 仅做激活与只读浏览。
        </span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      {!loading && items.length === 0 ? (
        <div className="wikiEmpty">暂无 schema。可通过后端 API 创建一条 fields 数组（≤ 64 项）。</div>
      ) : null}
      <div className="wikiList">
        {items.map((s) => (
          <div className={s.isActive ? "wikiCard active" : "wikiCard"} key={`${s.schemaId}-${s.version}`}>
            <div className="wikiCardHead">
              <div>
                <span className="wikiCardTitle">{s.name}</span>
                <span className="wikiCardMeta">
                  {s.schemaId} · v{s.version} · {s.fields.length} fields
                </span>
              </div>
              <div className="wikiCardActions">
                {s.isActive ? (
                  <span className="wikiBadge active">active</span>
                ) : (
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void activate(s.schemaId)}
                    disabled={activating === s.schemaId}
                  >
                    {activating === s.schemaId ? "切换中…" : "设为 active"}
                  </button>
                )}
              </div>
            </div>
            <div className="wikiCardBody">
              <div className="wikiFieldList">
                {s.fields.map((f) => (
                  <div className="wikiField" key={f.name}>
                    <span className="wikiFieldName">{f.name}</span>
                    <span className="wikiFieldKind">{f.kind}</span>
                    {f.required ? <span className="wikiFieldFlag">required</span> : null}
                    {f.allowedValues && f.allowedValues.length > 0 ? (
                      <span className="wikiFieldFlag">enum({f.allowedValues.length})</span>
                    ) : null}
                  </div>
                ))}
              </div>
              {Object.keys(s.aliasDict ?? {}).length > 0 ? (
                <div className="wikiAlias">
                  <span className="wikiAliasTitle">aliasDict</span>
                  <code>{JSON.stringify(s.aliasDict)}</code>
                </div>
              ) : null}
            </div>
          </div>
        ))}
      </div>
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

// MetricsTab：进程级 knowledge agent 指标透出。当前只显示 answer cache
// 命中率，后续可扩展。
//
// E5：拉 /api/knowledge/metrics → 渲染 cache hits / misses / entries / TTL。
// 5 秒手动刷新一次（不做 SSE，避免 EventSource 资源滥用）。
function MetricsTab() {
  const [data, setData] = useState<{ answerCache?: AnswerCacheMetrics } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch("/api/knowledge/metrics");
      if (!r.ok) throw await parseApiError(r);
      setData(await r.json());
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载指标失败");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  const cache = data?.answerCache;
  const total = cache ? cache.hits + cache.misses : 0;
  const hitRate = total > 0 ? ((cache!.hits / total) * 100).toFixed(1) : "—";

  return (
    <div className="wikiPanelBody">
      <div className="wikiMetricsHead">
        <div className="wikiMetricsTitle">Answer Cache</div>
        <button type="button" className="wikiMetricsRefresh" onClick={refresh} disabled={loading}>
          {loading ? "刷新中…" : "刷新"}
        </button>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {cache ? (
        <div className="wikiMetricsGrid">
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">命中</div>
            <div className="wikiMetricValue">{cache.hits}</div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">未命中</div>
            <div className="wikiMetricValue">{cache.misses}</div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">命中率</div>
            <div className="wikiMetricValue">{hitRate}%</div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">条目</div>
            <div className="wikiMetricValue">
              {cache.entries}
              <span className="wikiMetricSub"> / {cache.maxEntries}</span>
            </div>
          </div>
          <div className="wikiMetricCard">
            <div className="wikiMetricLabel">TTL</div>
            <div className="wikiMetricValue">
              {cache.ttlSeconds}
              <span className="wikiMetricSub"> 秒</span>
            </div>
          </div>
        </div>
      ) : !loading && !error ? (
        <div className="wikiHint">暂无指标数据。</div>
      ) : null}
    </div>
  );
}

interface AnswerCacheMetrics {
  entries: number;
  hits: number;
  misses: number;
  maxEntries: number;
  ttlSeconds: number;
}

// LintView：8 类 kind 树 + signal 列表 + 处置三按钮。替代旧 GapSignalsTab。
//
// 树是计数视图：左侧每行 [kind label] [count]；点击切换右侧 filter。
// 处置：dismiss（忽略）/ apply（标记已处理）；外加 sweep 一键扫描刷新。
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
type ReviewCategory =
  | "contested"
  | "needs_review"
  | "source_orphan"
  | "pending_verification"
  | "dependents_pending";

interface ReviewChunkItem {
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
        </div>
      </div>
    </div>
  );
}

function classifyChunk(
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
function ChunkInspectorPane({
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
function focusChunk(chunkId: string) {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent("wikiFocusChunk", { detail: { chunkId } }));
}

// ── P1-4 · WebSocket 软锁 + 事件总线 ───────────────────────────────────────
//
// `useChunkEventStream` 在 App 顶层挂一次：连 ws://.../api/ws/chunks，把后端
// 推下来的 ChunkEvent 转成两类 window CustomEvent：
//   - `wikiChunkLocked` / `wikiChunkUnlocked`：lock 状态变迁
//   - `wikiChunkRevised`：chunk 被编辑（patch / archive / restore / split / merge / ...）
// ChunkInspectorPane 监听 `wikiChunkRevised` 比对 chunkId 触发 reload，
// 让两个 admin 同步看到对方的写入；锁徽章监听前两个事件实时刷新。
//
// 重连：onclose → 5s 后重试，最长 30s 退避。WebSocket 失败不阻塞业务功能，
// 锁的状态在写入时仍然是真实的（acquire/release 走 HTTP）。
type ChunkEventEnvelope =
  | { kind: "hello"; workspace: string }
  | { kind: "lagged" }
  | {
      kind: "locked";
      chunk_id: string;
      workspace_id: string;
      owner_user_id: string;
      owner_username: string;
      expires_at: string;
    }
  | {
      kind: "unlocked";
      chunk_id: string;
      workspace_id: string;
      owner_user_id: string;
    }
  | {
      kind: "revised";
      chunk_id: string;
      workspace_id: string;
      revision_kind: string;
      actor: string;
    };

function useChunkEventStream() {
  useEffect(() => {
    let socket: WebSocket | null = null;
    let cancelled = false;
    let backoffMs = 1000;
    let timer: number | null = null;

    const connect = () => {
      if (cancelled) return;
      const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
      const url = `${proto}//${window.location.host}/api/ws/chunks`;
      try {
        socket = new WebSocket(url);
      } catch {
        scheduleReconnect();
        return;
      }
      socket.onopen = () => {
        backoffMs = 1000;
      };
      socket.onmessage = (ev) => {
        let parsed: ChunkEventEnvelope | null = null;
        try {
          parsed = JSON.parse(typeof ev.data === "string" ? ev.data : "") as ChunkEventEnvelope;
        } catch {
          return;
        }
        if (!parsed) return;
        switch (parsed.kind) {
          case "hello":
          case "lagged":
            return;
          case "locked":
            window.dispatchEvent(new CustomEvent("wikiChunkLocked", { detail: parsed }));
            return;
          case "unlocked":
            window.dispatchEvent(new CustomEvent("wikiChunkUnlocked", { detail: parsed }));
            return;
          case "revised":
            window.dispatchEvent(new CustomEvent("wikiChunkRevised", { detail: parsed }));
            return;
        }
      };
      socket.onclose = () => {
        scheduleReconnect();
      };
      socket.onerror = () => {
        try {
          socket?.close();
        } catch {
          // ignore
        }
      };
    };

    const scheduleReconnect = () => {
      if (cancelled) return;
      if (timer != null) return;
      timer = window.setTimeout(() => {
        timer = null;
        connect();
      }, backoffMs);
      backoffMs = Math.min(backoffMs * 2, 30000);
    };

    connect();

    return () => {
      cancelled = true;
      if (timer != null) {
        window.clearTimeout(timer);
        timer = null;
      }
      try {
        socket?.close();
      } catch {
        // ignore
      }
    };
  }, []);
}

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

function ChunkRevisionsTimeline({
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

interface TreeChunkItem extends ReviewChunkItem {
  businessTopics?: string[] | null;
}

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

interface CompletenessView {
  perWikiType?: Array<{ wikiType?: string; total?: number; ratio?: number }>;
  overall?: { total?: number; verified?: number; ratio?: number };
}

interface IntegrityReportView {
  needsReview?: number;
  contested?: number;
  sourceOrphan?: number;
  total?: number;
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
      setCompleteness(c as CompletenessView);
      setIntegrity(d as IntegrityReportView);
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
          {completeness?.perWikiType && completeness.perWikiType.length > 0 ? (
            <div className="wikiCoverageBars">
              {completeness.perWikiType.map((row, i) => {
                const ratio = Math.max(0, Math.min(1, Number(row.ratio ?? 0)));
                return (
                  <div className="wikiCoverageBarRow" key={i}>
                    <span className="wikiCoverageBarLabel">{row.wikiType ?? "?"}</span>
                    <div className="wikiCoverageBar">
                      <div
                        className="wikiCoverageBarFill"
                        style={{ width: `${(ratio * 100).toFixed(0)}%` }}
                      />
                    </div>
                    <span className="wikiCoverageBarValue">
                      {(ratio * 100).toFixed(0)}% · {row.total ?? 0}
                    </span>
                  </div>
                );
              })}
            </div>
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
            <dt>contested</dt>
            <dd>{integrity?.contested ?? 0}</dd>
            <dt>source_orphan</dt>
            <dd>{integrity?.sourceOrphan ?? 0}</dd>
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

// ──────────────────────────────────────────────────────────────────────
// G5 · AdminGovernanceView / MetadataDashboard / PublishBar
// ──────────────────────────────────────────────────────────────────────

interface MetadataResp {
  wikiTypeCounts?: Array<{ wikiType?: string; count?: number }>;
  verifiedRatioByType?: Array<{
    wikiType?: string;
    total?: number;
    verified?: number;
    ratio?: number;
  }>;
  topEditors?: Array<{ author?: string; count?: number }>;
  recentActivity7d?: Array<{ date?: string; op?: string; count?: number }>;
}

function MetadataDashboard() {
  const [data, setData] = useState<MetadataResp | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-knowledge/metadata");
      if (!r.ok) throw await parseApiError(r);
      setData((await r.json()) as MetadataResp);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 计算 wikiType 柱状图最大值，做归一化。
  const maxCount = useMemo(() => {
    const arr = data?.wikiTypeCounts ?? [];
    return arr.reduce((m, x) => Math.max(m, Number(x.count ?? 0)), 0);
  }, [data]);

  // 7d 活跃数据按日期归并 + total。
  const activityByDate = useMemo(() => {
    const arr = data?.recentActivity7d ?? [];
    const map: Record<string, number> = {};
    for (const a of arr) {
      const d = a.date ?? "";
      if (!d) continue;
      map[d] = (map[d] ?? 0) + Number(a.count ?? 0);
    }
    return Object.entries(map)
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([date, count]) => ({ date, count }));
  }, [data]);

  const maxActivity = useMemo(
    () => activityByDate.reduce((m, x) => Math.max(m, x.count), 0),
    [activityByDate]
  );

  return (
    <div className="wikiMetadataDashboard">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">atlas / governance</div>
          <h2>元信息总览</h2>
        </div>
        <div className="wikiArchiveHeaderActions">
          <button type="button" onClick={() => void load()} disabled={pending}>
            <RefreshCw size={14} /> 刷新
          </button>
        </div>
      </header>

      {error ? <div className="wikiBannerError">{error}</div> : null}

      <div className="wikiMetadataGrid">
        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[counts]</span>
            <h4>wiki_type 切片分布</h4>
          </header>
          {data?.wikiTypeCounts && data.wikiTypeCounts.length > 0 ? (
            <div className="wikiCoverageBars">
              {data.wikiTypeCounts.map((row, i) => {
                const ratio = maxCount > 0 ? Number(row.count ?? 0) / maxCount : 0;
                return (
                  <div className="wikiCoverageBarRow" key={i}>
                    <span className="wikiCoverageBarLabel">{row.wikiType ?? "?"}</span>
                    <div className="wikiCoverageBar">
                      <div
                        className="wikiCoverageBarFill"
                        style={{ width: `${(ratio * 100).toFixed(0)}%` }}
                      />
                    </div>
                    <span className="wikiCoverageBarValue">{row.count ?? 0}</span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="wikiEmpty">暂无切片</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[ratio]</span>
            <h4>verified 占比</h4>
          </header>
          {data?.verifiedRatioByType && data.verifiedRatioByType.length > 0 ? (
            <div className="wikiCoverageBars">
              {data.verifiedRatioByType.map((row, i) => {
                const ratio = Math.max(0, Math.min(1, Number(row.ratio ?? 0)));
                return (
                  <div className="wikiCoverageBarRow" key={i}>
                    <span className="wikiCoverageBarLabel">{row.wikiType ?? "?"}</span>
                    <div className="wikiCoverageBar">
                      <div
                        className="wikiCoverageBarFill"
                        style={{ width: `${(ratio * 100).toFixed(0)}%` }}
                      />
                    </div>
                    <span className="wikiCoverageBarValue">
                      {(ratio * 100).toFixed(0)}% · {row.verified ?? 0}/{row.total ?? 0}
                    </span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="wikiEmpty">暂无数据</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[editors]</span>
            <h4>近期编辑者</h4>
          </header>
          {data?.topEditors && data.topEditors.length > 0 ? (
            <dl className="wikiArchiveMeta">
              {data.topEditors.map((row, i) => (
                <Fragment key={i}>
                  <dt>{row.author ?? "unknown"}</dt>
                  <dd>{row.count ?? 0}</dd>
                </Fragment>
              ))}
            </dl>
          ) : (
            <div className="wikiEmpty">暂无编辑记录</div>
          )}
        </article>

        <article className="wikiObservabilityCard">
          <header className="wikiObservabilityCardHead">
            <span className="wikiArchiveTag">[activity]</span>
            <h4>7 天活跃</h4>
          </header>
          {activityByDate.length > 0 ? (
            <div className="wikiActivityChart">
              {activityByDate.map((d) => {
                const h = maxActivity > 0 ? (d.count / maxActivity) * 100 : 0;
                return (
                  <div className="wikiActivityBar" key={d.date} title={`${d.date}: ${d.count}`}>
                    <div
                      className="wikiActivityBarFill"
                      style={{ height: `${h.toFixed(0)}%` }}
                    />
                    <span className="wikiActivityBarLabel">{d.date.slice(5)}</span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="wikiEmpty">7d 内无修订</div>
          )}
        </article>
      </div>
    </div>
  );
}

interface PublishBarProps {
  resourceKind: "taxonomies" | "operation-state-policies" | "operation-domains";
  id: string;
  onChange?: () => void;
}

function PublishBar({ resourceKind, id, onChange }: PublishBarProps) {
  const [busy, setBusy] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  async function call(action: "publish" | "rollout" | "rollback") {
    if (action === "rollback" && !window.confirm("回退到上一版本？")) return;
    setBusy(action);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch(
        `/api/admin/${resourceKind}/${encodeURIComponent(id)}/${action}`,
        { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }
      );
      if (!r.ok) throw await parseApiError(r);
      setInfo(`${action} ok`);
      onChange?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy("");
    }
  }

  return (
    <div className="wikiPublishBar">
      <button
        type="button"
        onClick={() => void call("publish")}
        disabled={busy !== ""}
        className="wikiActionBtn--verify"
      >
        <CheckCircle2 size={12} /> {busy === "publish" ? "发布中…" : "发布新版"}
      </button>
      <button type="button" onClick={() => void call("rollout")} disabled={busy !== ""}>
        <ArrowRight size={12} /> {busy === "rollout" ? "灰度中…" : "灰度全量"}
      </button>
      <button
        type="button"
        onClick={() => void call("rollback")}
        disabled={busy !== ""}
        className="wikiActionBtn--reject"
      >
        <Undo2 size={12} /> {busy === "rollback" ? "回退中…" : "回退上版"}
      </button>
      {info ? <span className="wikiPublishBarInfo">{info}</span> : null}
      {error ? <span className="wikiPublishBarError">{error}</span> : null}
    </div>
  );
}

interface TaxonomyEntryView {
  id: string;
  scope?: string;
  kind?: string;
  value?: { id?: string; displayName?: string; status?: string };
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  updatedAt?: string;
}

function AdminGovernanceView() {
  const [tab, setTab] = useState<"meta" | "taxonomies" | "policies" | "domains">("meta");
  return (
    <div className="wikiArchiveShell wikiAdminGovernance">
      <header className="wikiArchiveHeader">
        <div>
          <div className="wikiArchiveEyebrow">atlas / governance</div>
          <h2>治理工坊</h2>
        </div>
      </header>
      <div className="wikiAdminTabs">
        <button
          type="button"
          className={tab === "meta" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("meta")}
        >
          <Activity size={12} /> 元信息
        </button>
        <button
          type="button"
          className={tab === "taxonomies" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("taxonomies")}
        >
          <LibraryBig size={12} /> 分类系统
        </button>
        <button
          type="button"
          className={tab === "policies" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("policies")}
        >
          <ShieldCheck size={12} /> 状态策略
        </button>
        <button
          type="button"
          className={tab === "domains" ? "wikiAdminTab active" : "wikiAdminTab"}
          onClick={() => setTab("domains")}
        >
          <Workflow size={12} /> 域配置
        </button>
      </div>
      <div className="wikiAdminPanel">
        {tab === "meta" && <MetadataDashboard />}
        {tab === "taxonomies" && <TaxonomiesGovernance />}
        {tab === "policies" && <StatePoliciesGovernance />}
        {tab === "domains" && <DomainGovernance />}
      </div>
    </div>
  );
}

function TaxonomiesGovernance() {
  const [items, setItems] = useState<TaxonomyEntryView[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [includeAll, setIncludeAll] = useState(false);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      if (includeAll) params.set("includeAllVersions", "true");
      const r = await fetch(
        `/api/admin/taxonomies${params.toString() ? "?" + params : ""}`
      );
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as { items?: TaxonomyEntryView[] };
      setItems(d.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, [includeAll]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section>
      <div className="wikiAdminToolbar">
        <label className="wikiAdminToolbarLabel">
          <input
            type="checkbox"
            checked={includeAll}
            onChange={(e) => setIncludeAll(e.target.checked)}
          />
          显示历史版本
        </label>
        <button type="button" onClick={() => void load()} disabled={pending}>
          <RefreshCw size={12} /> 刷新
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      <table className="wikiAdminTable">
        <thead>
          <tr>
            <th>scope</th>
            <th>kind</th>
            <th>value</th>
            <th>label</th>
            <th>status</th>
            <th>version</th>
            <th>active</th>
            <th>updated</th>
            <th>actions</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && !pending ? (
            <tr>
              <td colSpan={9}>
                <div className="wikiEmpty">暂无分类</div>
              </td>
            </tr>
          ) : null}
          {items.map((it) => (
            <tr key={it.id} className={it.currentVersion ? "is-active" : ""}>
              <td>{it.scope}</td>
              <td>{it.kind}</td>
              <td className="wikiArchiveTimelineTime">{it.value?.id}</td>
              <td>{it.value?.displayName}</td>
              <td>
                <span className="wikiArchiveTag">[{it.value?.status ?? "?"}]</span>
              </td>
              <td className="wikiArchiveTimelineTime">v{it.version ?? 0}</td>
              <td>{it.currentVersion ? "✓" : ""}</td>
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
              <td>
                <PublishBar
                  resourceKind="taxonomies"
                  id={it.id}
                  onChange={() => void load()}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

interface StatePolicyEntryView {
  id: string;
  domain?: string;
  version?: number;
  currentVersion?: boolean;
  updatedAt?: string;
  states?: unknown[];
}

function StatePoliciesGovernance() {
  const [items, setItems] = useState<StatePolicyEntryView[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/admin/operation-state-policies");
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as { items?: StatePolicyEntryView[] };
      setItems(d.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section>
      <div className="wikiAdminToolbar">
        <button type="button" onClick={() => void load()} disabled={pending}>
          <RefreshCw size={12} /> 刷新
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      <table className="wikiAdminTable">
        <thead>
          <tr>
            <th>domain</th>
            <th>version</th>
            <th>active</th>
            <th>states</th>
            <th>updated</th>
            <th>actions</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && !pending ? (
            <tr>
              <td colSpan={6}>
                <div className="wikiEmpty">暂无状态策略</div>
              </td>
            </tr>
          ) : null}
          {items.map((it) => (
            <tr key={it.id} className={it.currentVersion ? "is-active" : ""}>
              <td>{it.domain}</td>
              <td className="wikiArchiveTimelineTime">v{it.version ?? 0}</td>
              <td>{it.currentVersion ? "✓" : ""}</td>
              <td className="wikiArchiveTimelineTime">{(it.states ?? []).length} 状态</td>
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
              <td>
                <PublishBar
                  resourceKind="operation-state-policies"
                  id={it.id}
                  onChange={() => void load()}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

interface DomainEntryView {
  id: string;
  domain?: string;
  version?: number;
  currentVersion?: boolean;
  updatedAt?: string;
}

function DomainGovernance() {
  const [items, setItems] = useState<DomainEntryView[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setPending(true);
    setError(null);
    try {
      const r = await fetch("/api/operation-domains");
      if (!r.ok) throw await parseApiError(r);
      const d = (await r.json()) as { items?: DomainEntryView[] };
      setItems(d.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section>
      <div className="wikiAdminToolbar">
        <button type="button" onClick={() => void load()} disabled={pending}>
          <RefreshCw size={12} /> 刷新
        </button>
      </div>
      {error ? <div className="wikiBannerError">{error}</div> : null}
      <table className="wikiAdminTable">
        <thead>
          <tr>
            <th>domain</th>
            <th>version</th>
            <th>active</th>
            <th>updated</th>
            <th>actions</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && !pending ? (
            <tr>
              <td colSpan={5}>
                <div className="wikiEmpty">暂无域配置</div>
              </td>
            </tr>
          ) : null}
          {items.map((it) => (
            <tr key={it.id} className={it.currentVersion ? "is-active" : ""}>
              <td>{it.domain}</td>
              <td className="wikiArchiveTimelineTime">v{it.version ?? 0}</td>
              <td>{it.currentVersion ? "✓" : ""}</td>
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
              <td>
                <PublishBar
                  resourceKind="operation-domains"
                  id={it.id}
                  onChange={() => void load()}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
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

function numberOr(v: unknown): number {
  return typeof v === "number" ? v : Number(v ?? 0) || 0;
}
function stringOr(v: unknown): string {
  return typeof v === "string" ? v : "";
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

// ── Phase F · Atlas Mode：运营记忆抽屉 ──────────────────────────────────

interface OperatorMemoryView {
  id: string | null;
  workspaceId: string;
  accountId: string;
  operatorId: string;
  kind: string;
  content: string;
  createdAt?: string | null;
  lastUsedAt?: string | null;
  expiresAt?: string | null;
}

const OPERATOR_MEMORY_KINDS: Array<{ key: string; label: string }> = [
  { key: "", label: "全部" },
  { key: "preference", label: "偏好" },
  { key: "rejection", label: "拒绝" },
  { key: "context", label: "上下文" }
];

function MemoryDrawer() {
  const [items, setItems] = useState<OperatorMemoryView[]>([]);
  const [kind, setKind] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    setPending(true);
    setError(null);
    try {
      const params = new URLSearchParams();
      if (kind) params.set("kind", kind);
      params.set("limit", "100");
      const r = await fetch(`/api/knowledge/operator-memory?${params.toString()}`);
      if (!r.ok) throw await parseApiError(r);
      const data = (await r.json()) as { items: OperatorMemoryView[] };
      setItems(data.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setItems([]);
    } finally {
      setPending(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind]);

  return (
    <div className="wikiMemoryDrawer">
      <div className="wikiMemoryHead">
        <h3>运营记忆</h3>
        <span className="wikiHint">注入到 reply prompt 的长期偏好/拒绝/上下文</span>
      </div>
      <div className="wikiMemoryFilter">
        {OPERATOR_MEMORY_KINDS.map((k) => (
          <button
            key={k.key || "all"}
            type="button"
            className={kind === k.key ? "wikiMemoryKindBtn active" : "wikiMemoryKindBtn"}
            onClick={() => setKind(k.key)}
          >
            {k.label}
          </button>
        ))}
        <button
          type="button"
          className="wikiMemoryKindBtn"
          onClick={() => void load()}
          disabled={pending}
        >
          <RefreshCw size={12} /> {pending ? "刷新中" : "刷新"}
        </button>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {!error && !pending && items.length === 0 ? (
        <div className="wikiEmpty">该筛选下暂无运营记忆。</div>
      ) : null}
      <ul className="wikiMemoryList">
        {items.map((m) => (
          <li className={`wikiMemoryItem kind-${m.kind}`} key={m.id ?? `${m.kind}-${m.createdAt}`}>
            <div className="wikiMemoryItemHead">
              <span className={`wikiMemoryKind kind-${m.kind}`}>{m.kind}</span>
              <span className="wikiMemoryOperator">{m.operatorId}</span>
            </div>
            <div className="wikiMemoryContent">{m.content}</div>
            <div className="wikiMemoryFoot">
              <span>last_used_at: {m.lastUsedAt ?? "—"}</span>
              <span>expires_at: {m.expiresAt ?? "—"}</span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function QualityCenterView({ accountId }: { accountId?: string }) {
  const [tab, setTab] = useState<"outcome" | "autoVerify" | "formula" | "markers">("outcome");
  return (
    <section className="qualityCenter">
      <div className="panelHead compact">
        <div>
          <span>Outcome & Quality</span>
          <h2>运营成效中心</h2>
        </div>
        <Workflow size={18} />
      </div>
      <div className="qualityTabs">
        <button className={tab === "outcome" ? "tab active" : "tab"} onClick={() => setTab("outcome")}>
          长期指标
        </button>
        <button className={tab === "autoVerify" ? "tab active" : "tab"} onClick={() => setTab("autoVerify")}>
          知识自动校验
        </button>
        <button className={tab === "formula" ? "tab active" : "tab"} onClick={() => setTab("formula")}>
          公式遵守度
        </button>
        <button className={tab === "markers" ? "tab active" : "tab"} onClick={() => setTab("markers")}>
          产品声明标记词
        </button>
      </div>
      {tab === "outcome" && <OutcomeMetricsTab accountId={accountId} />}
      {tab === "autoVerify" && <AutoVerifyTab accountId={accountId} />}
      {tab === "formula" && <FormulaAdherenceTab accountId={accountId} />}
      {tab === "markers" && <ProductClaimMarkersTab />}
    </section>
  );
}

function OutcomeMetricsTab({ accountId }: { accountId?: string }) {
  const [horizon, setHorizon] = useState<"7d" | "30d">("7d");
  const [items, setItems] = useState<OutcomeMetric[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string>("");

  async function load() {
    if (!accountId) return;
    setLoading(true);
    setErr("");
    try {
      const data = await api.get<{ items: OutcomeMetric[] }>(
        `/api/agent-outcome-metrics?accountId=${encodeURIComponent(accountId)}&horizon=${horizon}&limit=60`
      );
      setItems(data.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId, horizon]);

  return (
    <div className="qualityPanel">
      <div className="qualityToolbar">
        <select value={horizon} onChange={(e) => setHorizon(e.target.value as "7d" | "30d")}>
          <option value="7d">7 天窗口</option>
          <option value="30d">30 天窗口</option>
        </select>
        <button onClick={() => void load()} disabled={loading || !accountId}>
          {loading ? "加载中" : "刷新"}
        </button>
        <small style={{ color: "#888" }}>
          指标说明：null（"—"）= 该窗口内无样本；不要把它当 0 解读。
        </small>
      </div>
      {err && <div className="error">{err}</div>}
      {!accountId && <p>请先在顶部选择一个微信账号。</p>}
      {accountId && items.length === 0 && !loading && (
        <p>该账号在选定 horizon 内还没有 outcome aggregation 任务跑过。后台 worker 会在每天 tick 时自动生成。</p>
      )}
      {items.length > 0 && (
        <table className="qualityTable">
          <thead>
            <tr>
              <th>日期</th>
              <th>回复率</th>
              <th>对话深度</th>
              <th>AI暂缓澄清率</th>
              <th>Agent 拦截率</th>
              <th>当日 run 数</th>
              <th>当日 token</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr key={item.id}>
                <td>{item.date}</td>
                <td>{formatRate(item.replyRate)}</td>
                <td>{formatNumber(item.conversationDepth)}</td>
                <td>{formatRate(item.aiHoldClearedRate)}</td>
                <td>{formatRate(item.agentBlockRate)}</td>
                <td>{item.dailyRunCount}</td>
                <td>{item.dailyRunTokenTotal.toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

/**
 * M3 / Task 77：Planner 三段事件聚合。后端 `/api/outcomes/autonomy.planner` 返回此 shape。
 *
 * 关键不变量：
 * - silent 段 `tick` 是 `strategic_planner_tick` 事件数；`scanned` 来自 tick details 的累加；
 *   `emitted` 是真实 `strategic_planner_emit` 事件数；`tickDetailEmitted` 是 tick 自报的累加值（兜底）。
 * - commitment 段拆 `overdueEmits / imminentEmits`，与 daily-cap 一致。
 * - stagnation 段单一 emit 维度。
 * - 三段都有独立的 `_backoff` 计数，绝不计入 daily-cap。
 */
type PlannerSection = {
  silent: {
    tick: number;
    scanned: number;
    emitted: number;
    tickDetailEmitted: number;
    capped: number;
    backoff: number;
  };
  commitment: {
    tick: number;
    overdueEmits: number;
    imminentEmits: number;
    backoff: number;
  };
  stagnation: {
    tick: number;
    emitted: number;
    backoff: number;
  };
};

export function AutonomyOutcomesTab({ accountId }: { accountId?: string }) {
  type AutonomyMetrics = {
    horizon: string;
    accountId: string;
    totalRuns: number;
    legacyModeUnchecked: number;
    metrics: {
      revisionTriggerRate: number | null;
      revisionPassRate: number | null;
      aiHoldBreakdown: {
        heldByAiPolicy: number | null;
        blockedBySafetyGuard: number | null;
        aiWaitingForMoreContext: number | null;
      };
      taxonomyCandidateRate: number | null;
      unverifiedClaimBlockRate: number | null;
      selfCritiqueAddressedRate: number | null;
      autonomyModeDistribution: {
        auto: number | null;
        assisted: number | null;
        blocked: number | null;
      };
    };
    rawCounts: Record<string, number>;
    outboxLink: {
      totalEnqueued: number;
      sent: number;
      canceled: number;
      failedTerminal: number;
      sendSuccessRate: number | null;
      canceledRate: number | null;
      failedTerminalRate: number | null;
    };
    /** M3 / Task 70：Planner 三段 tick / emit / capped / backoff 计数。 */
    planner?: PlannerSection;
  };
  type RevisionItem = {
    runId: string;
    contactWxid: string | null;
    contactName: string | null;
    preReplyExcerpt: string;
    postReplyExcerpt: string;
    preRevisionSummary: string;
    postRevisionSummary: string;
    revisionDirection: string;
    finalReviewStatus: string;
    holdCategory: string;
    selfCritique: string | null;
    createdAt: string;
  };

  const [horizon, setHorizon] = useState<"24h" | "7d" | "30d">("24h");
  const [data, setData] = useState<AutonomyMetrics | null>(null);
  const [revisions, setRevisions] = useState<RevisionItem[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string>("");

  async function load() {
    if (!accountId) return;
    setLoading(true);
    setErr("");
    try {
      const qs = `accountId=${encodeURIComponent(accountId)}&horizon=${horizon}`;
      const [metrics, revs] = await Promise.all([
        api.get<AutonomyMetrics>(`/api/outcomes/autonomy?${qs}`),
        api.get<{ items: RevisionItem[] }>(
          `/api/outcomes/autonomy/revisions?${qs}&limit=50`
        ),
      ]);
      setData(metrics);
      setRevisions(revs.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId, horizon]);

  const m = data?.metrics;
  const ob = data?.outboxLink;
  const breakdown = m?.aiHoldBreakdown;
  const dist = m?.autonomyModeDistribution;

  return (
    <div className="qualityPanel">
      <div className="qualityToolbar">
        <select value={horizon} onChange={(e) => setHorizon(e.target.value as "24h" | "7d" | "30d")}>
          <option value="24h">24 小时窗口</option>
          <option value="7d">7 天窗口</option>
          <option value="30d">30 天窗口</option>
        </select>
        <button onClick={() => void load()} disabled={loading || !accountId}>
          {loading ? "加载中" : "刷新"}
        </button>
        <small style={{ color: "#888" }}>
          指标说明：null（"—"）= 该窗口内没有升级后样本；legacy 行单独计数不进任何分子分母。
        </small>
      </div>
      {err && <div className="error">{err}</div>}
      {!accountId && <p>请先在顶部选择一个微信账号。</p>}
      {accountId && data && (
        <>
          <div className="autonomyHeader">
            <span>升级后 run 数：<strong>{data.totalRuns}</strong></span>
            <span>未升级 run（独立计数）：<strong>{data.legacyModeUnchecked}</strong></span>
          </div>
          <div className="autonomyMetricGrid">
            <AutonomyMetricCard
              label="revision 触发率"
              value={m?.revisionTriggerRate ?? null}
              hint={`${data.rawCounts.revisionApplied}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="revision 通过率"
              value={m?.revisionPassRate ?? null}
              hint={`${data.rawCounts.revisionPass}/${data.rawCounts.revisionApplied}`}
            />
            <AutonomyMetricCard
              label="未验证产品声明拦截率"
              value={m?.unverifiedClaimBlockRate ?? null}
              hint={`${data.rawCounts.unverifiedClaimBlock}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="新词候选触发率"
              value={m?.taxonomyCandidateRate ?? null}
              hint={`${data.rawCounts.taxonomyCandidate}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="自我批判已回应率"
              value={m?.selfCritiqueAddressedRate ?? null}
              hint={`${data.rawCounts.selfCritiqueAddressed}/${data.rawCounts.revisionApplied}`}
            />
            <AutonomyMetricCard
              label="自治模式：auto"
              value={dist?.auto ?? null}
              hint={`${data.rawCounts.autonomyAuto}/${data.rawCounts.totalRuns}`}
            />
            <AutonomyMetricCard
              label="自治模式：assisted / blocked"
              value={null}
              hint={`assisted ${data.rawCounts.autonomyAssisted} · blocked ${data.rawCounts.autonomyBlocked}`}
            />
          </div>

          <div className="autonomySection">
            <h3>AI 暂缓原因分布</h3>
            <div className="autonomyHoldBars">
              <HoldBar label="AI 策略主动暂缓" value={breakdown?.heldByAiPolicy ?? null} count={data.rawCounts.heldByAiPolicy} />
              <HoldBar label="安全门拦截" value={breakdown?.blockedBySafetyGuard ?? null} count={data.rawCounts.blockedBySafetyGuard} />
              <HoldBar label="AI 等待更多上下文" value={breakdown?.aiWaitingForMoreContext ?? null} count={data.rawCounts.aiWaitingForMoreContext} />
            </div>
          </div>

          <div className="autonomySection">
            <h3>发送链路状态</h3>
            <table className="qualityTable">
              <thead>
                <tr>
                  <th>入队总数</th>
                  <th>已送达</th>
                  <th>已取消</th>
                  <th>终态失败</th>
                  <th>送达率</th>
                  <th>取消率</th>
                  <th>失败率</th>
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td>{ob?.totalEnqueued ?? 0}</td>
                  <td>{ob?.sent ?? 0}</td>
                  <td>{ob?.canceled ?? 0}</td>
                  <td>{ob?.failedTerminal ?? 0}</td>
                  <td>{formatRate(ob?.sendSuccessRate ?? null)}</td>
                  <td>{formatRate(ob?.canceledRate ?? null)}</td>
                  <td>{formatRate(ob?.failedTerminalRate ?? null)}</td>
                </tr>
              </tbody>
            </table>
          </div>

          {data.planner && <AutonomyPlannerSection planner={data.planner} />}

          <div className="autonomySection">
            <h3>近 50 条 revision 记录</h3>
            {revisions.length === 0 ? (
              <p>该窗口内没有 revision 记录。</p>
            ) : (
              <table className="qualityTable">
                <thead>
                  <tr>
                    <th>联系人</th>
                    <th>修订前摘要</th>
                    <th>修订后摘要</th>
                    <th>修订方向</th>
                    <th>归档状态</th>
                    <th>暂缓分类</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {revisions.map((r) => (
                    <RevisionRow
                      key={r.runId}
                      item={r}
                      expanded={expanded === r.runId}
                      onToggle={() => setExpanded(expanded === r.runId ? null : r.runId)}
                    />
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </>
      )}
    </div>
  );
}

export function AutonomyMetricCard({ label, value, hint }: { label: string; value: number | null; hint: string }) {
  return (
    <div className="autonomyMetricCard">
      <div className="autonomyMetricLabel">{label}</div>
      <div className="autonomyMetricValue">{formatRate(value)}</div>
      <div className="autonomyMetricHint">{hint}</div>
    </div>
  );
}

/**
 * M3 / Task 77：Planner 三段事件可视化。展示 tick / scanned / emitted / capped / backoff
 * 计数；标签使用 AI 自主策略表达，符合"全自治"产品定位。
 */
export function AutonomyPlannerSection({ planner }: { planner: PlannerSection }) {
  return (
    <div className="autonomySection" data-testid="planner-section">
      <h3>Planner 自主调度</h3>
      <small style={{ color: "#888" }}>
        三段扫描器（沉默跟进 / 承诺到期 / 阶段停滞）的 tick、emit、capped、backoff 计数；backoff 表示 AI 因 block-rate 过高自主回退。
      </small>
      <div className="autonomyMetricGrid" style={{ marginTop: 12 }}>
        <div className="autonomyMetricCard" data-testid="planner-silent">
          <div className="autonomyMetricLabel">沉默跟进</div>
          <div className="autonomyMetricValue">{planner.silent.emitted}</div>
          <div className="autonomyMetricHint">
            tick {planner.silent.tick} · scanned {planner.silent.scanned} · capped {planner.silent.capped} · backoff {planner.silent.backoff}
          </div>
        </div>
        <div className="autonomyMetricCard" data-testid="planner-commitment">
          <div className="autonomyMetricLabel">承诺到期</div>
          <div className="autonomyMetricValue">
            {planner.commitment.overdueEmits + planner.commitment.imminentEmits}
          </div>
          <div className="autonomyMetricHint">
            tick {planner.commitment.tick} · overdue {planner.commitment.overdueEmits} · imminent {planner.commitment.imminentEmits} · backoff {planner.commitment.backoff}
          </div>
        </div>
        <div className="autonomyMetricCard" data-testid="planner-stagnation">
          <div className="autonomyMetricLabel">阶段停滞</div>
          <div className="autonomyMetricValue">{planner.stagnation.emitted}</div>
          <div className="autonomyMetricHint">
            tick {planner.stagnation.tick} · backoff {planner.stagnation.backoff}
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * M3 / Task 79：Cockpit 内 Planner 视角。展示运营状态最近变更时间与
 * commitments 列表，给运营快速理解 AI 主动跟进的信号面。只读卡片，符合
 * "全自治、无中转"产品定位。
 */
export function PlannerViewSection({ contact }: { contact: Contact | null }) {
  if (!contact) {
    return null;
  }
  const stageUpdatedAt = contact.domainAttributesUpdatedAt;
  const stageLabel = (() => {
    const attrs = contact.domainAttributes;
    if (!attrs || typeof attrs !== "object") return "";
    const stage = (attrs as Record<string, unknown>).stage;
    return typeof stage === "string" ? stage : "";
  })();
  const commitments = (contact.commitments ?? []).slice(0, 5);
  const hasStage = !!stageUpdatedAt;
  const hasCommitments = commitments.length > 0;
  const lastMode = (contact as { lastConversationMode?: string | null }).lastConversationMode || null;
  const hasMode = !!lastMode;
  if (!hasStage && !hasCommitments && !hasMode) {
    return null;
  }
  return (
    <section className="cockpitSection" data-testid="planner-view-section">
      <div className="sectionCaption">Planner 视角</div>
      {hasMode && (
        <div data-testid="planner-mode-row" style={{ fontSize: 13, color: "#444", marginBottom: 8 }}>
          上轮对话模式 <strong>{conversationModeLabel(lastMode!)}</strong>
        </div>
      )}
      {hasStage && (
        <div data-testid="planner-stage-row" style={{ fontSize: 13, color: "#444", marginBottom: 8 }}>
          运营阶段 <strong>{stageLabel || "未分层"}</strong>
          ：自 <span>{formatStageTimestamp(stageUpdatedAt!)}</span> 起未变更
        </div>
      )}
      {hasCommitments && (
        <ul data-testid="planner-commitments" style={{ listStyle: "disc", paddingLeft: 18, margin: 0 }}>
          {commitments.map((c, idx) => (
            <li key={c.id ?? `${idx}-${c.text}`} style={{ fontSize: 13, color: "#333", lineHeight: 1.6 }}>
              <span>{c.text}</span>
              {c.dueAt && (
                <span style={{ marginLeft: 6, color: "#888" }}>· 计划 {formatStageTimestamp(c.dueAt)}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function conversationModeLabel(mode: string): string {
  switch (mode) {
    case "casual_relationship":
      return "寒暄关系（casual_relationship）";
    case "value_exchange":
      return "价值互换（value_exchange）";
    case "consultative":
      return "顾问/销售（consultative）";
    case "boundary_protection":
      return "边界保护（boundary_protection）";
    default:
      return mode;
  }
}

/** M3 / Task 79：把 ISO 时间格式化成 cockpit 显示用的"YYYY-MM-DD HH:mm"。 */
function formatStageTimestamp(iso: string): string {
  const dt = new Date(iso);
  if (Number.isNaN(dt.getTime())) {
    return iso;
  }
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${dt.getFullYear()}-${pad(dt.getMonth() + 1)}-${pad(dt.getDate())} ${pad(dt.getHours())}:${pad(dt.getMinutes())}`;
}

export function HoldBar({ label, value, count }: { label: string; value: number | null; count: number }) {
  const pct = value === null || value === undefined ? 0 : Math.max(0, Math.min(1, value)) * 100;
  return (
    <div className="autonomyHoldBar">
      <div className="autonomyHoldLabel">
        <span>{label}</span>
        <span>{formatRate(value)}（{count} 条）</span>
      </div>
      <div className="autonomyHoldTrack">
        <div className="autonomyHoldFill" style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function RevisionRow({
  item,
  expanded,
  onToggle,
}: {
  item: {
    runId: string;
    contactName: string | null;
    contactWxid: string | null;
    preReplyExcerpt: string;
    postReplyExcerpt: string;
    preRevisionSummary: string;
    postRevisionSummary: string;
    revisionDirection: string;
    finalReviewStatus: string;
    holdCategory: string;
    selfCritique: string | null;
  };
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr>
        <td>{item.contactName || item.contactWxid || "—"}</td>
        <td>{item.preReplyExcerpt || "—"}</td>
        <td>{item.postReplyExcerpt || "—"}</td>
        <td>{item.revisionDirection || "—"}</td>
        <td>{item.finalReviewStatus}</td>
        <td>{item.holdCategory || "—"}</td>
        <td>
          <button className="linkBtn" onClick={onToggle}>
            {expanded ? "收起" : "展开"}
          </button>
        </td>
      </tr>
      {expanded && (
        <tr className="autonomyRevisionDetail">
          <td colSpan={7}>
            <div>
              <strong>修订前完整摘要：</strong>
              <pre>{item.preRevisionSummary || "—"}</pre>
            </div>
            <div>
              <strong>修订后完整摘要：</strong>
              <pre>{item.postRevisionSummary || "—"}</pre>
            </div>
            <div>
              <strong>自我批判（selfCritique）：</strong>
              <pre>{item.selfCritique || "—"}</pre>
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

export function formatRate(value: number | null): string {
  if (value === null || value === undefined) return "—";
  return `${(value * 100).toFixed(1)}%`;
}

function formatNumber(value: number | null): string {
  if (value === null || value === undefined) return "—";
  return value.toFixed(2);
}

function AutoVerifyTab({ accountId }: { accountId?: string }) {
  const [threshold, setThreshold] = useState(7);
  const [sampleRate, setSampleRate] = useState(0.1);
  const [limit, setLimit] = useState(50);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<AutoVerifyResult | null>(null);
  const [err, setErr] = useState<string>("");

  async function run() {
    if (!accountId) return;
    setBusy(true);
    setErr("");
    setResult(null);
    try {
      const data = await api.post<AutoVerifyResult>("/api/operation-knowledge/auto-verify", {
        accountId,
        confidenceThreshold: threshold,
        humanAuditSampleRate: sampleRate,
        limit
      });
      setResult(data);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="qualityPanel">
      <p style={{ color: "#666", fontSize: 13 }}>
        对 <strong>needs_review</strong> 状态的知识切片做 LLM 自动校验。verified 必须满足：
        切片自带 <code>source_quote</code> 非空 + <code>source_anchors</code> 可定位 +
        模型 <code>integrityStatus="verified"</code> + 置信分 ≥ 阈值。否则降级为 needs_review/rejected。
        随机 <code>sampleRate</code> 比例的 verified 切片改成 needs_human_audit 走运营抽查。
      </p>
      <div className="qualityToolbar">
        <label>
          置信阈值
          <input
            type="number"
            min={0}
            max={10}
            value={threshold}
            onChange={(e) => setThreshold(Number(e.target.value) || 0)}
            style={{ width: 60, marginLeft: 6 }}
          />
        </label>
        <label>
          抽样比例
          <input
            type="number"
            step={0.05}
            min={0}
            max={1}
            value={sampleRate}
            onChange={(e) => setSampleRate(Number(e.target.value) || 0)}
            style={{ width: 60, marginLeft: 6 }}
          />
        </label>
        <label>
          单次上限
          <input
            type="number"
            min={1}
            max={500}
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value) || 1)}
            style={{ width: 60, marginLeft: 6 }}
          />
        </label>
        <button onClick={() => void run()} disabled={busy || !accountId}>
          {busy ? "校验中" : "开始自动校验"}
        </button>
      </div>
      {err && <div className="error">{err}</div>}
      {result && (
        <div className="autoVerifyResult">
          <h3>校验结果（共 {result.processed} 条）{result.degraded && <span className="badge degraded"> 预算超额降级</span>}</h3>
          <ul>
            <li>verified：{result.verified}</li>
            <li>needs_review：{result.needsReview}</li>
            <li>rejected：{result.rejected}</li>
            <li>needs_human_audit：{result.needsHumanAudit}</li>
          </ul>
          {result.budget && (
            <pre style={{ background: "#f7f7f7", padding: 8, fontSize: 12 }}>
              {JSON.stringify(result.budget, null, 2)}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function FormulaAdherenceTab({ accountId }: { accountId?: string }) {
  const [busy, setBusy] = useState(false);
  const [summary, setSummary] = useState<FormulaSummary | null>(null);
  const [items, setItems] = useState<FormulaItem[]>([]);
  const [err, setErr] = useState<string>("");

  async function run() {
    if (!accountId) return;
    setBusy(true);
    setErr("");
    setSummary(null);
    setItems([]);
    try {
      const data = await api.post<{ summary: FormulaSummary; items: FormulaItem[] }>(
        "/api/user-operations/evaluations/formula-adherence",
        { accountId }
      );
      setSummary(data.summary);
      setItems(data.items || []);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="qualityPanel">
      <p style={{ color: "#666", fontSize: 13 }}>
        对所有 <code>active</code> 的 evaluation_scenarios 跑一次 simulate_user_dialogue，
        抓最后一个 turn 的 <code>review.formulaBreakdown</code> 与 <code>scores</code>，
        与场景的 <code>ground_truth</code> 比较计算 adherence。整批共享一个累计 token 预算
        （每场景 simulationTokenBudget × scenarios 数），超额时返回部分结果 + degraded:true。
        缺四个公式的场景标 invalid，不静默按 0 计入平均。
      </p>
      <div className="qualityToolbar">
        <button onClick={() => void run()} disabled={busy || !accountId}>
          {busy ? "评测中" : "开始评测"}
        </button>
      </div>
      {err && <div className="error">{err}</div>}
      {summary && (
        <div className="formulaSummary">
          <h3>
            平均 adherence：{summary.meanAdherence.toFixed(3)}（{summary.scenarioCount} 个有效场景）
            {summary.degraded && (
              <span className="badge degraded">
                降级：{summary.degradedReason || summary.reason || "未知"}
              </span>
            )}
          </h3>
          {summary.totalTokenBudget !== undefined && (
            <small>
              预算使用：{summary.totalTokensUsed?.toLocaleString() || 0} /{" "}
              {summary.totalTokenBudget.toLocaleString()}
              {summary.processedBeforeBudgetExceeded !== undefined &&
                ` · 超额前完成 ${summary.processedBeforeBudgetExceeded} 个`}
            </small>
          )}
        </div>
      )}
      {items.length > 0 && (
        <table className="qualityTable">
          <thead>
            <tr>
              <th>场景</th>
              <th>状态</th>
              <th>adherence</th>
              <th>偏差（预测 - 实际）</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item) => (
              <tr key={item.scenarioId} className={item.invalid ? "invalid" : ""}>
                <td>
                  <strong>{item.title || item.scenarioId}</strong>
                  <br />
                  <small style={{ color: "#888" }}>{item.scenarioId}</small>
                </td>
                <td>
                  {item.error
                    ? `❌ ${item.error}`
                    : item.skipped
                    ? `⏭ ${item.reason || "skipped"}`
                    : item.invalid
                    ? `⚠ ${item.invalidReason || "invalid"}`
                    : "✓ 完成"}
                </td>
                <td>
                  {item.adherenceScore !== undefined
                    ? item.adherenceScore.toFixed(3)
                    : "—"}
                </td>
                <td>
                  {item.deviations ? (
                    <code style={{ fontSize: 11 }}>
                      {Object.entries(item.deviations)
                        .map(([k, v]) => `${k}=${typeof v === "number" ? v.toFixed(2) : v}`)
                        .join(", ")}
                    </code>
                  ) : (
                    "—"
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ProductClaimMarkersTab() {
  const [template, setTemplate] = useState<PromptTemplateLite | null>(null);
  const [draft, setDraft] = useState<string>("");
  const [parseError, setParseError] = useState<string>("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string>("");
  const [statusMsg, setStatusMsg] = useState<string>("");

  async function load() {
    setErr("");
    setStatusMsg("");
    try {
      const data = await api.get<{ items: PromptTemplateLite[] }>("/api/prompt-templates");
      const found = (data.items || []).find(
        (item) => item.promptKey === "user.review.product_claim_markers" && item.status === "active"
      );
      if (!found) {
        setErr("未找到 active 的 user.review.product_claim_markers 模板，可能需要重置 prompt pack。");
        return;
      }
      setTemplate(found);
      setDraft(found.content);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }

  useEffect(() => {
    void load();
  }, []);

  function validateJson(text: string): string {
    try {
      const parsed = JSON.parse(text);
      if (!parsed || typeof parsed !== "object") return "JSON 顶层必须是对象";
      if (!Array.isArray(parsed.markers)) return "缺 markers 数组";
      if (!Array.isArray(parsed.whitelistPhrases)) return "缺 whitelistPhrases 数组";
      if (typeof parsed.whitelistWindowChars !== "number") return "whitelistWindowChars 必须是数字";
      for (const m of parsed.markers) {
        if (!m || typeof m !== "object") return "markers 中含非对象项";
        if (typeof m.kind !== "string") return "marker.kind 必须是字符串";
        if (typeof m.label !== "string") return "marker.label 必须是字符串";
      }
      return "";
    } catch (e) {
      return e instanceof Error ? e.message : String(e);
    }
  }

  function onChangeDraft(value: string) {
    setDraft(value);
    setParseError(validateJson(value));
  }

  async function save() {
    if (!template) return;
    const validation = validateJson(draft);
    if (validation) {
      setParseError(validation);
      return;
    }
    setSaving(true);
    setErr("");
    setStatusMsg("");
    try {
      // 写一个新版本草稿（PUT 现有 active 模板的 content）。
      await api.put(`/api/prompt-templates/${template.id}`, {
        promptKey: template.promptKey,
        agentKind: "user",
        layer: "review_guard",
        title: "产品事实风险兜底标记",
        description: "Rust 字符串兜底 guard 使用的可编辑标记词和白名单。",
        content: draft,
        status: "active"
      });
      // 立即 publish 让 Rust 端 30s 缓存失效，确保新规则即时生效。
      await api.post(`/api/prompt-templates/${template.id}/publish`);
      setStatusMsg("已发布，Rust 端缓存已失效；下一次 review 即生效。");
      await load();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="qualityPanel">
      <p style={{ color: "#666", fontSize: 13 }}>
        Rust 端字符串级 fact-risk 兜底守卫使用的标记词与白名单短语。这是 Review Agent
        判断"看似中性的话术里夹带绝对承诺"的最后一道防线。修改后会立即让进程内
        30s TTL 缓存失效，下一次 review 加载新规则。
      </p>
      {err && <div className="error">{err}</div>}
      {statusMsg && <div className="success">{statusMsg}</div>}
      {template && (
        <>
          <small style={{ color: "#888" }}>
            模板版本 v{template.version} · status={template.status}
          </small>
          <textarea
            value={draft}
            onChange={(e) => onChangeDraft(e.target.value)}
            spellCheck={false}
            style={{
              width: "100%",
              minHeight: 360,
              fontFamily: "monospace",
              fontSize: 12,
              marginTop: 6
            }}
          />
          {parseError && (
            <div className="error" style={{ marginTop: 6 }}>
              JSON 校验：{parseError}
            </div>
          )}
          <div style={{ marginTop: 8 }}>
            <button onClick={() => void save()} disabled={saving || !!parseError}>
              {saving ? "发布中" : "保存并发布"}
            </button>
            <button onClick={() => void load()} disabled={saving} style={{ marginLeft: 8 }}>
              丢弃改动
            </button>
          </div>
        </>
      )}
    </div>
  );
}

