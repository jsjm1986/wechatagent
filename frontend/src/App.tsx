import {
  Activity,
  AlertTriangle,
  Bot,
  BrainCircuit,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock3,
  Copy,
  Eye,
  EyeOff,
  FileBox,
  FileText,
  FlaskConical,
  Inbox,
  LibraryBig,
  LayoutDashboard,
  MessageSquareText,
  Package,
  RefreshCw,
  Search,
  SendHorizonal,
  Settings2,
  ShieldCheck,
  Sparkles,
  SquarePen,
  Trash2,
  UploadCloud,
  UserRoundCheck,
  User2,
  UsersRound,
  Workflow,
  X
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { FormEvent, useEffect, useMemo, useRef, useState, KeyboardEvent, ClipboardEvent } from "react";
import type * as React from "react";

import { EvolutionCenterTab } from "./EvolutionCenterTab";

type AgentStatus = "normal" | "managed";
type Channel = "command" | "overview" | "userOps" | "groupOps" | "momentOps" | "knowledge" | "content" | "systemStrategy" | "operations" | "autonomy" | "evolution" | "quality" | "llmProviders" | "knowledgeWiki";
type ContactTab = "all" | "managed" | "normal";
type SmartOpsTab = "cockpit" | "adjust" | "profile" | "memory" | "simulation" | "conversation";
type TraditionalOpsTab = "playbooks" | "prompts" | "settings" | "audit";
type UserOpsMode = "smart" | "traditional";
type OpsTab = "tasks" | "events" | "reviews" | "llm";

type Account = {
  id: string;
  accountId: string;
  alias: string;
  displayName: string;
  appId?: string;
  wxid?: string;
  nickName?: string;
  mcpKeyConfigured?: boolean;
  online: boolean;
};

type AgentProfile = {
  summary: string;
  interests: string[];
  communicationStyle: string;
  operationGoal: string;
};

type Contact = {
  id: string;
  accountId: string;
  wxid: string;
  nickname?: string;
  remark?: string;
  alias?: string;
  agentStatus: AgentStatus;
  humanProfileNote?: string;
  customAgentInstructions?: string | null;
  agentProfile?: AgentProfile;
  memorySummary?: string;
  playbookId?: string;
  playbookVersion?: number;
  tags: string[];
  customerStage?: string;
  /** M3 / Task 80：customer_stage 最近一次变更的时间，给 cockpit Planner 视角展示停滞情况。 */
  customerStageUpdatedAt?: string | null;
  intentLevel?: string;
  /** M3 / Task 80：承诺数组（M2 之后由 dialog → contact 同步），cockpit 侧只读展示。 */
  commitments?: ContactCommitment[];
  lastCommitment?: string;
  followUpPolicy?: string;
  operationState?: string;
  operationStateReason?: string;
  operationStateConfidence?: number;
  operationStateUpdatedAt?: string;
  cooldownUntil?: string;
  operationPolicy: Record<string, unknown>;
  profileAttributes: Record<string, unknown>;
  profileUpdatedAt?: string;
  /** 波 A2 / B2：最近一条入站消息时间（不含 outbound）。 */
  lastInboundAt?: string;
  /** 波 A2 / B2：最近一次 Agent 主动出站时间。 */
  lastOutboundAt?: string;
  /** 兼容字段：max(lastInboundAt, lastOutboundAt)。 */
  lastMessageAt?: string;
  updatedAt: string;
};

/** M3 / Task 80：与后端 `ApiCommitment` 对齐的承诺条目结构。 */
type ContactCommitment = {
  id?: string;
  text: string;
  dueAt?: string | null;
  createdAt?: string | null;
};

type Message = {
  id: string;
  direction: "inbound" | "outbound";
  content: string;
  createdAt?: string;
};

type EventItem = {
  id: string;
  contactWxid?: string;
  kind: string;
  status: string;
  summary: string;
  createdAt?: string;
};

type TaskItem = {
  id: string;
  contactWxid: string;
  kind: string;
  runAt?: string;
  expiresAt?: string;
  content: string;
  status: string;
  sourceDecisionId?: string;
  reviewRequired?: boolean;
  gatewayStatus?: string;
  cancelReason?: string;
  error?: string;
};

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

type OperationKnowledgeItem = {
  id: string;
  category: string;
  businessType: string;
  knowledgeType?: string;
  businessContext?: string;
  title: string;
  summary?: string;
  body?: string;
  routingCard?: string;
  applicableScenes: string[];
  notApplicableScenes: string[];
  suitableFor: string[];
  notSuitableFor: string[];
  customerStages: string[];
  operationStates: string[];
  intentLevels: string[];
  commonQuestions: string[];
  commonObjections: string[];
  safeClaims: string[];
  forbiddenClaims: string[];
  evidenceItems: string[];
  productTags?: string[];
  triggerKeywords?: string[];
  businessTopics?: string[];
  sourceType: string;
  sourceName?: string;
  status: string;
  priority: number;
  updatedAt?: string;
};

type OperationKnowledgeDraft = {
  category: string;
  businessType: string;
  knowledgeType: string;
  businessContext: string;
  title: string;
  summary: string;
  body: string;
  routingCard: string;
  applicableScenes: string;
  notApplicableScenes: string;
  suitableFor: string;
  notSuitableFor: string;
  customerStages: string;
  operationStates: string;
  intentLevels: string;
  commonQuestions: string;
  commonObjections: string;
  safeClaims: string;
  forbiddenClaims: string;
  evidenceItems: string;
  productTags: string;
  triggerKeywords: string;
  businessTopics: string;
  sourceName: string;
  status: string;
  priority: string;
};

type OperationKnowledgeDocument = {
  id: string;
  title: string;
  domain?: string;
  sourceType: string;
  sourceName?: string;
  summary?: string;
  catalogSummary?: string;
  routingMap: string[];
  riskNotes: string[];
  productTags?: string[];
  triggerKeywords?: string[];
  businessTopics?: string[];
  rawContent?: string;
  contentHash?: string;
  lineIndex: Record<string, unknown>[];
  sectionIndex: Record<string, unknown>[];
  status: string;
  updatedAt?: string;
};

type OperationKnowledgeChunk = {
  id: string;
  documentId?: string;
  itemId?: string;
  domain?: string;
  knowledgeType?: string;
  businessContext?: string;
  title: string;
  summary?: string;
  body?: string;
  routingCard?: string;
  applicableScenes: string[];
  notApplicableScenes: string[];
  safeClaims: string[];
  forbiddenClaims: string[];
  evidenceItems: string[];
  sourceQuote?: string;
  sourceAnchors: Record<string, unknown>[];
  integrityStatus?: string;
  confidenceScore?: number;
  distortionRisks: string[];
  unsupportedClaims: string[];
  verifiedClaims: string[];
  status: string;
  priority: number;
  productTags?: string[];
  triggerKeywords?: string[];
  businessTopics?: string[];
  interpretation?: Record<string, unknown>;
  updatedAt?: string;
};

type OperationKnowledgeChunkDraft = {
  documentId: string;
  itemId: string;
  knowledgeType: string;
  businessContext: string;
  title: string;
  summary: string;
  body: string;
  routingCard: string;
  applicableScenes: string;
  notApplicableScenes: string;
  safeClaims: string;
  forbiddenClaims: string;
  evidenceItems: string;
  sourceQuote: string;
  integrityStatus: string;
  confidenceScore: string;
  distortionRisks: string;
  unsupportedClaims: string;
  verifiedClaims: string;
  status: string;
  priority: string;
  productTags: string;
  triggerKeywords: string;
  businessTopics: string;
};

type KnowledgeChatTurnView = {
  turnIndex: number;
  role: "user" | "assistant" | "system";
  intent?: string | null;
  content: string;
  attachments?: Array<{ chunk_id?: string; item_id?: string; taskId?: string; phase?: string }>;
  patch?: Record<string, unknown> | null;
  missingFields?: string[];
  followupQuestions?: Array<{ id?: string; field?: string; question?: string }>;
  status?: string;
  tokensUsed?: number;
  promptKey?: string | null;
  /// knowledge-digest-workstation Phase 4：worker 写的进度 turn。
  /// `task_progress` / `task_summary` / `tool_call_log` / null
  kind?: string | null;
  toolCalls?: Array<Record<string, unknown>>;
  createdAt?: string;
};

type KnowledgeChatTurnResponse = {
  sessionId: string;
  turnIndex: number;
  intent: string;
  naturalReply: string;
  draftKind?: string | null;
  draftPreview?: Record<string, unknown> | null;
  /// digest_action intent 命中时返回的 plannedSteps（每条含 stepId/cardId/action/summary/estimatedLlmCalls）。
  plannedSteps?: Array<Record<string, unknown>> | null;
  estimatedLlmCalls?: number | null;
  missingFields: string[];
  followupQuestions: Array<{ id?: string; field?: string; question?: string }>;
  canApply: boolean;
  targetChunkId?: string | null;
  targetPackId?: string | null;
  promptKey?: string | null;
  tokensUsed: number;
  budget?: Record<string, unknown>;
};

type KnowledgeDigestCardView = {
  cardId: string;
  kind: string;
  title: string;
  summary: string;
  targetRefs?: Array<{ kind?: string; id?: string }>;
  suggestedAction: string;
  severity: "critical" | "warn" | "info" | string;
  metric?: { name?: string; value?: number; threshold?: number } | null;
};

type KnowledgeDailyReportView = {
  reportId?: string | null;
  workspaceId: string;
  accountId: string;
  reportDate: string;
  generatedAt?: string;
  generatedBy?: string;
  status: "ok" | "partial" | "failed" | string;
  errorKind?: string | null;
  budgetSnapshot?: Record<string, unknown>;
  cards: KnowledgeDigestCardView[];
  dismissedCardIds?: string[];
  promptVersions?: Record<string, unknown>;
};

type OperationKnowledgeImportPreview = {
  document: OperationKnowledgeDocument | null;
  items: OperationKnowledgeItem[];
  chunks: OperationKnowledgeChunk[];
  integrityReport?: Record<string, unknown>;
};

type KnowledgeUsageItem = {
  id: string;
  contactWxid?: string;
  runId: string;
  knowledgeIds: string[];
  routeResult: Record<string, unknown>;
  replyText?: string;
  reviewApproved: boolean;
  blockedReason?: string;
  toolTrace?: Record<string, unknown>[];
  createdAt?: string;
};

type KnowledgeIntegrityReport = {
  total: number;
  verified: number;
  needsReview: number;
  rejected: number;
  items: Array<Record<string, unknown>>;
};

type KnowledgeCompletenessReport = {
  totalChunks: number;
  verifiedChunks: number;
  anchoredChunks: number;
  evidenceChunks: number;
  answeringMode: "relationship_only" | "product_safe" | "fully_supported" | string;
  summary: string;
  coverage: Record<string, boolean>;
  gaps: string[];
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

function todayLocalDate(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
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
  { id: "knowledge", label: "运营知识库", caption: "知识路由", icon: LibraryBig },
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
  { key: "factRiskBlockAt", label: "事实风险拦截线", detail: "事实风险达到该分值则禁止发送", kind: "number", defaultValue: 6 },
  { key: "pressureRiskBlockAt", label: "压迫感拦截线", detail: "销售压迫感达到该分值则禁止发送", kind: "number", defaultValue: 7 },
  { key: "humanLikeRewriteBelow", label: "真人感重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 6 },
  { key: "emotionalValueRewriteBelow", label: "情绪价值重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 5 },
  { key: "productAccuracyBlockBelow", label: "产品准确性拦截线", detail: "低于该分值则禁止发送", kind: "number", defaultValue: 7 },
  { key: "operationStateConfidenceFullReviewBelow", label: "状态置信 Review 线", detail: "低于该分值强制完整 Review", kind: "number", defaultValue: 4 },
  { key: "runTokenBudget", label: "单次 Token 预算", detail: "单次用户运营运行的最大 token", kind: "number", defaultValue: 30000 },
  { key: "runMaxLlmCalls", label: "单次模型调用上限", detail: "单次用户运营最多 LLM 调用次数", kind: "number", defaultValue: 6 },
  { key: "simulationTokenBudget", label: "模拟评测预算", detail: "单次模拟/评测可用 token", kind: "number", defaultValue: 60000 },
  { key: "reactionTokenBudget", label: "反应分析预算", detail: "用户回应分析单次最多 token", kind: "number", defaultValue: 8000 },
  { key: "reactionMaxLlmCalls", label: "反应分析调用上限", detail: "用户回应分析最多 LLM 调用次数", kind: "number", defaultValue: 2 }
];

// P1-8：知识库 chat sessionId 必须按 account 隔离——切到另一个微信号时不应该
// 把上一个号的 session 续上去（不同 account 的知识库隔离边界）。把
// `wechatagent.knowledgeChatSessionId` 升级为
// `wechatagent.knowledgeChatSessionId.{accountId}`；空 accountId 时退化为
// 不持久化（避免老的 legacy 全局键继续被无脑读到）。
const KNOWLEDGE_CHAT_SESSION_KEY_PREFIX = "wechatagent.knowledgeChatSessionId";
const LEGACY_KNOWLEDGE_CHAT_SESSION_KEY = "wechatagent.knowledgeChatSessionId";
function chatSessionStorageKey(accountId: string | undefined): string | null {
  if (!accountId) return null;
  return `${KNOWLEDGE_CHAT_SESSION_KEY_PREFIX}.${accountId}`;
}
function readPersistedChatSession(accountId: string | undefined): string | undefined {
  try {
    const key = chatSessionStorageKey(accountId);
    if (!key) return undefined;
    const v = window.localStorage.getItem(key);
    if (v) return v;
    // legacy 兼容：上一版本是全局键；如果新键空、legacy 有值，搬过来一次。
    const legacy = window.localStorage.getItem(LEGACY_KNOWLEDGE_CHAT_SESSION_KEY);
    if (legacy) {
      window.localStorage.setItem(key, legacy);
      window.localStorage.removeItem(LEGACY_KNOWLEDGE_CHAT_SESSION_KEY);
      return legacy;
    }
    return undefined;
  } catch {
    return undefined;
  }
}
function writePersistedChatSession(accountId: string | undefined, sessionId: string) {
  try {
    const key = chatSessionStorageKey(accountId);
    if (!key) return;
    window.localStorage.setItem(key, sessionId);
  } catch {
    /* ignore */
  }
}
function clearPersistedChatSession(accountId: string | undefined) {
  try {
    const key = chatSessionStorageKey(accountId);
    if (key) window.localStorage.removeItem(key);
  } catch {
    /* ignore */
  }
}

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
  const [operationKnowledge, setOperationKnowledge] = useState<OperationKnowledgeItem[]>([]);
  const [knowledgeDocuments, setKnowledgeDocuments] = useState<OperationKnowledgeDocument[]>([]);
  const [knowledgeChunks, setKnowledgeChunks] = useState<OperationKnowledgeChunk[]>([]);
  const [knowledgeCatalog, setKnowledgeCatalog] = useState<Record<string, unknown> | null>(null);
  const [knowledgeIntegrity, setKnowledgeIntegrity] = useState<KnowledgeIntegrityReport | null>(null);
  const [knowledgeCompleteness, setKnowledgeCompleteness] = useState<KnowledgeCompletenessReport | null>(null);
  const [chunkSource, setChunkSource] = useState<Record<string, unknown> | null>(null);
  const [aiRepairTarget, setAiRepairTarget] = useState<{ kind: "chunk" | "pack"; id: string; label: string } | null>(null);
  const [knowledgeChatOpen, setKnowledgeChatOpen] = useState(false);
  // 运营知识库频道双模：AI 协作（inbox + chat 主导）/ 手动（旧三栏入口）。
  // 持久化到 localStorage `wechatagent.knowledgeMode`，默认 "ai"。
  const [knowledgeMode, setKnowledgeMode] = useState<"ai" | "manual">(() => {
    try {
      const v = localStorage.getItem("wechatagent.knowledgeMode");
      return v === "manual" ? "manual" : "ai";
    } catch {
      return "ai";
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("wechatagent.knowledgeMode", knowledgeMode);
    } catch {
      /* localStorage 不可用时静默 */
    }
  }, [knowledgeMode]);
  // P1-8：sessionId 按 account 隔离——初值不再从全局 localStorage 读，而是等
  // currentAccountId 解析后用 useEffect 从 `wechatagent.knowledgeChatSessionId.{accountId}`
  // 读。account 切换时同样在 useEffect 里换号读取。
  const [knowledgeChatSessionId, setKnowledgeChatSessionId] = useState<string | undefined>(undefined);
  const [digestReport, setDigestReport] = useState<KnowledgeDailyReportView | null>(null);
  const [digestPhase, setDigestPhase] = useState<"idle" | "loading" | "regenerating" | "error">("idle");
  const [digestError, setDigestError] = useState<Error | null>(null);
  const [digestSelectedCardIds, setDigestSelectedCardIds] = useState<string[]>([]);
  const [digestPendingChatInjection, setDigestPendingChatInjection] = useState<string | null>(null);
  const [knowledgeUsage, setKnowledgeUsage] = useState<KnowledgeUsageItem[]>([]);
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
  const [knowledgeDraft, setKnowledgeDraft] = useState<OperationKnowledgeDraft>(emptyKnowledgeDraft());
  const [editingChunkId, setEditingChunkId] = useState("");
  const [chunkDraft, setChunkDraft] = useState<OperationKnowledgeChunkDraft>(emptyChunkDraft());
  const [editingKnowledgeId, setEditingKnowledgeId] = useState("");
  const [knowledgeSelectedNodeId, setKnowledgeSelectedNodeId] = useState<string | null>(null);
  const [knowledgeExpandedNodes, setKnowledgeExpandedNodes] = useState<Set<string>>(new Set());
  const [knowledgeTreeSearch, setKnowledgeTreeSearch] = useState("");
  const [knowledgePackSubview, setKnowledgePackSubview] = useState<"overview" | "metadata" | "chunks">("overview");
  const [knowledgeDebugDrawerOpen, setKnowledgeDebugDrawerOpen] = useState(false);
  const [knowledgeWorkspaceMode, setKnowledgeWorkspaceMode] = useState<"selection" | "import">("selection");
  const [knowledgeDocModal, setKnowledgeDocModal] = useState<
    | { mode: "create"; title: string; sourceName: string; summary: string }
    | { mode: "edit"; id: string; title: string; sourceName: string; summary: string }
    | null
  >(null);
  const [knowledgeImportSource, setKnowledgeImportSource] = useState("运营知识导入");
  const [knowledgeImportText, setKnowledgeImportText] = useState("");
  const [knowledgeImportPreview, setKnowledgeImportPreview] = useState<OperationKnowledgeImportPreview>({
    document: null,
    items: [],
    chunks: []
  });
  const [knowledgeTestMessage, setKnowledgeTestMessage] = useState("客户问：你们能不能保证转化提升？有没有真实案例？");
  const [knowledgeTestResult, setKnowledgeTestResult] = useState<Record<string, unknown> | null>(null);
  const knowledgeTree = useMemo(
    () => buildKnowledgeTree(knowledgeDocuments, operationKnowledge, knowledgeChunks),
    [knowledgeDocuments, operationKnowledge, knowledgeChunks]
  );

  useEffect(() => {
    if (!knowledgeSelectedNodeId) return;
    const ancestors = findAncestors(knowledgeTree, knowledgeSelectedNodeId);
    if (!ancestors || !ancestors.length) return;
    setKnowledgeExpandedNodes((prev) => {
      let changed = false;
      const next = new Set(prev);
      for (const id of ancestors) {
        if (!next.has(id)) {
          next.add(id);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [knowledgeSelectedNodeId, knowledgeTree]);
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
      knowledgeData,
      documentData,
      chunkData,
      catalogData,
      completenessData,
      integrityData,
      usageData,
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
      api.get<{ items: OperationKnowledgeItem[] }>(`/api/operation-knowledge${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: OperationKnowledgeDocument[] }>(`/api/operation-knowledge/documents${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: OperationKnowledgeChunk[] }>(`/api/operation-knowledge/chunks${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ item: Record<string, unknown> }>(`/api/operation-knowledge/catalog${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ item: KnowledgeCompletenessReport }>(`/api/operation-knowledge/completeness${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ item: KnowledgeIntegrityReport }>(`/api/operation-knowledge/integrity-report${accountParam ? `?${accountParam}` : ""}`),
      api.get<{ items: KnowledgeUsageItem[] }>(`/api/operation-knowledge/usage${accountParam ? `?${accountParam}` : ""}`),
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
    setOperationKnowledge(knowledgeData.items);
    setKnowledgeDocuments(documentData.items);
    setKnowledgeChunks(chunkData.items);
    setKnowledgeCatalog(catalogData.item);
    setKnowledgeCompleteness(completenessData.item);
    setKnowledgeIntegrity(integrityData.item);
    setKnowledgeUsage(usageData.items);
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

  async function createKnowledge(event: FormEvent) {
    event.preventDefault();
    if (!knowledgeDraft.title.trim()) return;
    await run(async () => {
      await api.post("/api/operation-knowledge", {
        accountId: currentAccountId || undefined,
        ...knowledgePayload(knowledgeDraft)
      });
      newKnowledgeDraft();
      await loadAll();
    });
  }

  async function saveKnowledge(event: FormEvent) {
    event.preventDefault();
    if (!editingKnowledgeId || !knowledgeDraft.title.trim()) return;
    await run(async () => {
      await api.put(`/api/operation-knowledge/${editingKnowledgeId}`, {
        accountId: currentAccountId || undefined,
        ...knowledgePayload(knowledgeDraft)
      });
      await loadAll();
    });
  }

  async function deleteKnowledge(id: string) {
    await run(async () => {
      await api.delete(`/api/operation-knowledge/${id}`);
      if (editingKnowledgeId === id) newKnowledgeDraft();
      await loadAll();
    });
  }

  async function previewKnowledgeImport() {
    if (!knowledgeImportText.trim()) return;
    await run(async () => {
      const data = await api.post<OperationKnowledgeImportPreview>("/api/operation-knowledge/import-preview", {
        accountId: currentAccountId || undefined,
        sourceName: knowledgeImportSource || undefined,
        content: knowledgeImportText
      });
      setKnowledgeImportPreview({
        document: data.document || null,
        items: data.items || [],
        chunks: data.chunks || [],
        integrityReport: data.integrityReport
      });
      setKnowledgeWorkspaceMode("import");
    });
  }

  async function applyKnowledgeImport() {
    if (!knowledgeImportPreview.document && !knowledgeImportPreview.items.length && !knowledgeImportPreview.chunks.length) return;
    await run(async () => {
      await api.post("/api/operation-knowledge/import-apply", {
        accountId: currentAccountId || undefined,
        sourceName: knowledgeImportSource || undefined,
        document: knowledgeImportPreview.document ? knowledgeDocumentPayload(knowledgeImportPreview.document) : undefined,
        items: knowledgeImportPreview.items.map(knowledgeItemPayload),
        chunks: knowledgeImportPreview.chunks.map(knowledgeChunkPayload)
      });
      setKnowledgeImportPreview({ document: null, items: [], chunks: [] });
      setKnowledgeImportText("");
      await loadAll();
      setKnowledgeWorkspaceMode("selection");
    });
  }

  async function createKnowledgeChunk(event: FormEvent) {
    event.preventDefault();
    if (!chunkDraft.title.trim()) return;
    await run(async () => {
      await api.post("/api/operation-knowledge/chunks", {
        accountId: currentAccountId || undefined,
        ...chunkPayload(chunkDraft)
      });
      newChunkDraft();
      await loadAll();
    });
  }

  async function saveKnowledgeChunk(event: FormEvent) {
    event.preventDefault();
    if (!editingChunkId || !chunkDraft.title.trim()) return;
    await run(async () => {
      await api.put(`/api/operation-knowledge/chunks/${editingChunkId}`, {
        accountId: currentAccountId || undefined,
        ...chunkPayload(chunkDraft)
      });
      await loadAll();
    });
  }

  async function deleteKnowledgeChunk(id: string) {
    await run(async () => {
      await api.delete(`/api/operation-knowledge/chunks/${id}`);
      if (editingChunkId === id) newChunkDraft();
      await loadAll();
    });
  }

  async function deleteKnowledgeDocument(id: string) {
    await run(async () => {
      await api.delete(`/api/operation-knowledge/documents/${id}`);
      await loadAll();
    });
  }

  async function createKnowledgeDocumentManual(body: {
    title: string;
    sourceName?: string;
    summary?: string;
    accountId?: string;
  }) {
    await run(async () => {
      await api.post(`/api/operation-knowledge/documents`, {
        title: body.title,
        sourceName: body.sourceName,
        summary: body.summary,
        accountId: body.accountId ?? currentAccountId ?? undefined,
        sourceType: "manual"
      });
      await loadAll();
    });
  }

  async function updateKnowledgeDocumentMeta(
    id: string,
    body: {
      title: string;
      sourceName?: string;
      summary?: string;
      accountId?: string;
    }
  ) {
    await run(async () => {
      await api.put(`/api/operation-knowledge/documents/${id}`, {
        title: body.title,
        sourceName: body.sourceName,
        summary: body.summary,
        accountId: body.accountId ?? currentAccountId ?? undefined,
        sourceType: "manual"
      });
      await loadAll();
    });
  }

  async function listChunksByDocument(documentId: string): Promise<OperationKnowledgeChunk[]> {
    const res = await api.get<{ items: OperationKnowledgeChunk[] }>(
      `/api/operation-knowledge/documents/${documentId}/chunks`
    );
    return res?.items ?? [];
  }

  async function verifyKnowledgeChunk(id: string) {
    await run(async () => {
      await api.post(`/api/operation-knowledge/chunks/${id}/verify`, {});
      await loadAll();
    });
  }

  async function postKnowledgeChatTurn(body: {
    sessionId?: string;
    accountId?: string;
    content: string;
    attachments?: Array<{ chunkId?: string; itemId?: string }>;
  }): Promise<KnowledgeChatTurnResponse> {
    return api.post<KnowledgeChatTurnResponse>("/api/operation-knowledge/chat", body);
  }

  async function getKnowledgeChatHistory(
    sessionId: string
  ): Promise<{ sessionId: string; items: KnowledgeChatTurnView[]; total: number }> {
    return api.get<{ sessionId: string; items: KnowledgeChatTurnView[]; total: number }>(
      `/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}`
    );
  }

  async function applyKnowledgeChat(
    sessionId: string,
    accountId?: string
  ): Promise<{ ok: boolean; sessionId: string; intent: string; result: Record<string, unknown> }> {
    return api.post(`/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}/apply`, {
      accountId
    });
  }

  async function discardKnowledgeChat(
    sessionId: string
  ): Promise<{ ok: boolean; sessionId: string; discardedCount: number }> {
    return api.post(`/api/operation-knowledge/chat/${encodeURIComponent(sessionId)}/discard`, {});
  }

  // knowledge-digest-workstation Phase 4 / P4.4：派工长任务 + SSE 进度回调。
  async function postChatTask(body: {
    sessionId: string;
    accountId?: string;
    operatorId?: string;
    cardIds?: string[];
    plannedSteps: Array<Record<string, unknown>>;
  }): Promise<{ taskId: string; sessionId: string; status: string; totalSteps: number }> {
    return api.post(`/api/knowledge/chat/tasks`, body);
  }

  async function getDigestToday(): Promise<KnowledgeDailyReportView> {
    return api.get<KnowledgeDailyReportView>(`/api/knowledge/digest/today`);
  }

  async function regenerateDigest(
    accountId?: string,
    force = true
  ): Promise<KnowledgeDailyReportView> {
    return api.post<KnowledgeDailyReportView>(`/api/knowledge/digest/regenerate`, {
      accountId,
      force
    });
  }

  async function dismissDigestCard(cardId: string): Promise<{ ok: boolean; cardId: string }> {
    return api.post<{ ok: boolean; cardId: string }>(
      `/api/knowledge/digest/cards/${encodeURIComponent(cardId)}/dismiss`,
      {}
    );
  }

  function openKnowledgeChat(sessionId?: string) {
    if (sessionId) {
      setKnowledgeChatSessionId(sessionId);
      writePersistedChatSession(currentAccountId, sessionId);
    }
    setKnowledgeChatOpen(true);
  }

  function closeKnowledgeChat(persistedSessionId?: string) {
    setKnowledgeChatOpen(false);
    if (persistedSessionId) {
      setKnowledgeChatSessionId(persistedSessionId);
      writePersistedChatSession(currentAccountId, persistedSessionId);
    }
  }

  function toggleDigestCardSelected(cardId: string) {
    setDigestSelectedCardIds((prev) =>
      prev.includes(cardId) ? prev.filter((id) => id !== cardId) : [...prev, cardId]
    );
  }

  function digestSelectAll() {
    if (!digestReport) return;
    const dismissed = new Set(digestReport.dismissedCardIds || []);
    setDigestSelectedCardIds(
      digestReport.cards
        .filter((c) => !dismissed.has(c.cardId))
        .map((c) => c.cardId)
    );
  }

  function digestInvertSelect() {
    if (!digestReport) return;
    const dismissed = new Set(digestReport.dismissedCardIds || []);
    const all = digestReport.cards
      .filter((c) => !dismissed.has(c.cardId))
      .map((c) => c.cardId);
    const selected = new Set(digestSelectedCardIds);
    setDigestSelectedCardIds(all.filter((id) => !selected.has(id)));
  }

  async function digestIgnoreInfoCards() {
    if (!digestReport) return;
    const dismissed = new Set(digestReport.dismissedCardIds || []);
    const infoCards = digestReport.cards.filter(
      (c) => c.severity === "info" && !dismissed.has(c.cardId)
    );
    for (const card of infoCards) {
      try {
        await dismissDigestCard(card.cardId);
      } catch (err) {
        setDigestError(err instanceof Error ? err : new Error(String(err)));
      }
    }
    try {
      const fresh = await getDigestToday();
      setDigestReport(fresh);
    } catch {
      /* ignore refresh failure here; banner already shown */
    }
  }

  async function regenerateDigestNow() {
    setDigestPhase("regenerating");
    setDigestError(null);
    try {
      const fresh = await regenerateDigest(currentAccountId || undefined, true);
      setDigestReport(fresh);
      setDigestSelectedCardIds([]);
      setDigestPhase("idle");
    } catch (err) {
      setDigestError(err instanceof Error ? err : new Error(String(err)));
      setDigestPhase("error");
    }
  }

  async function dismissDigestCardAndRefresh(cardId: string) {
    try {
      await dismissDigestCard(cardId);
      setDigestSelectedCardIds((prev) => prev.filter((id) => id !== cardId));
      const fresh = await getDigestToday();
      setDigestReport(fresh);
    } catch (err) {
      setDigestError(err instanceof Error ? err : new Error(String(err)));
    }
  }

  function serializeDigestCardForChat(card: KnowledgeDigestCardView): string {
    const refs = (card.targetRefs || [])
      .map((r) => `${r.kind || "?"}:${r.id || "?"}`)
      .join(", ");
    const metric = card.metric
      ? `（${card.metric.name || "metric"}=${card.metric.value ?? "?"}/${card.metric.threshold ?? "?"}）`
      : "";
    return `[日报派工] severity=${card.severity} kind=${card.kind} ${card.title}${metric}\n摘要：${card.summary}\n引用：${refs}\n建议动作：${card.suggestedAction}`;
  }

  function dispatchSelectedCardsToChat() {
    if (!digestReport || digestSelectedCardIds.length === 0) return;
    const dismissed = new Set(digestReport.dismissedCardIds || []);
    const picked = digestReport.cards.filter(
      (c) => digestSelectedCardIds.includes(c.cardId) && !dismissed.has(c.cardId)
    );
    if (picked.length === 0) return;
    const blob =
      `请帮我处理以下 ${picked.length} 条日报 issue（按 severity 优先级，每条单独跑一轮 chat）：\n\n` +
      picked.map((c, i) => `${i + 1}. ${serializeDigestCardForChat(c)}`).join("\n\n");
    setDigestPendingChatInjection(blob);
    setKnowledgeChatOpen(true);
  }

  function dispatchOneCardToChat(cardId: string) {
    if (!digestReport) return;
    const card = digestReport.cards.find((c) => c.cardId === cardId);
    if (!card) return;
    const blob =
      `请帮我处理这条日报 issue：\n\n${serializeDigestCardForChat(card)}`;
    setDigestPendingChatInjection(blob);
    setKnowledgeChatOpen(true);
  }

  function openCardTarget(card: KnowledgeDigestCardView) {
    const ref = (card.targetRefs || [])[0];
    if (!ref || !ref.kind || !ref.id) return;
    if (ref.kind === "chunk") {
      setKnowledgeSelectedNodeId(`chunk:${ref.id}`);
      const found = knowledgeChunks.find((c) => c.id === ref.id);
      if (found) {
        setEditingChunkId(found.id);
        setChunkDraft(draftFromChunk(found));
        setKnowledgeWorkspaceMode("selection");
      }
    } else if (ref.kind === "pack") {
      setKnowledgeSelectedNodeId(`pack:${ref.id}`);
    }
  }

  function openKnowledgeDocCreateModal() {
    setKnowledgeDocModal({ mode: "create", title: "", sourceName: "", summary: "" });
  }

  function openKnowledgeDocEditModal(doc: OperationKnowledgeDocument) {
    setKnowledgeDocModal({
      mode: "edit",
      id: doc.id,
      title: doc.title ?? "",
      sourceName: doc.sourceName ?? "",
      summary: doc.summary ?? ""
    });
  }

  function closeKnowledgeDocModal() {
    setKnowledgeDocModal(null);
  }

  async function submitKnowledgeDocModal() {
    if (!knowledgeDocModal) return;
    const title = knowledgeDocModal.title.trim();
    if (!title) return;
    const payload = {
      title,
      sourceName: knowledgeDocModal.sourceName.trim() || undefined,
      summary: knowledgeDocModal.summary.trim() || undefined
    };
    if (knowledgeDocModal.mode === "create") {
      await createKnowledgeDocumentManual(payload);
    } else {
      await updateKnowledgeDocumentMeta(knowledgeDocModal.id, payload);
    }
    setKnowledgeDocModal(null);
  }

  async function refreshKnowledgeCompleteness() {
    await run(async () => {
      const accountParam = currentAccountId ? `accountId=${encodeURIComponent(currentAccountId)}` : "";
      await api.post(
        `/api/operation-knowledge/completeness${accountParam ? `?${accountParam}` : ""}`,
        {}
      );
      await loadAll();
    });
  }

  async function extractTagsForChunk(chunkId: string) {
    await run(async () => {
      const result = await api.post<Record<string, unknown>>(
        "/api/operation-knowledge/extract-tags",
        {
          accountId: currentAccountId || undefined,
          chunkId
        }
      );
      const merged = result?.["item"] as OperationKnowledgeChunk | undefined;
      if (merged) {
        setKnowledgeChunks((prev) =>
          prev.map((c) => (c.id === merged.id ? { ...c, ...merged } : c))
        );
      }
      await loadAll();
    });
  }

  async function proposeChunkRepair(chunkId: string): Promise<Record<string, unknown>> {
    return api.post<Record<string, unknown>>(
      `/api/operation-knowledge/chunks/${chunkId}/repair`,
      {}
    );
  }

  async function answerChunkRepair(
    chunkId: string,
    body: {
      sessionId?: string;
      previousPatch?: Record<string, unknown> | null;
      answers: Array<{ id: string; field?: string; text: string }>;
      turn: number;
    }
  ): Promise<Record<string, unknown>> {
    return api.post<Record<string, unknown>>(
      `/api/operation-knowledge/chunks/${chunkId}/repair/answer`,
      body
    );
  }

  async function proposePackRepair(packId: string): Promise<Record<string, unknown>> {
    return api.post<Record<string, unknown>>(
      `/api/operation-knowledge/items/${packId}/repair`,
      {}
    );
  }

  async function applyAiRepairPatch(
    target: { kind: "chunk" | "pack"; id: string },
    patch: Record<string, unknown>,
    options: { thenVerify?: boolean } = {},
    auditMeta: AiRepairApplyAuditMeta = { acceptedFields: [], skippedFields: [] }
  ) {
    await run(async () => {
      // 1) 把 patch 中 schema 没有容器的 `extras` 摘出来，仅作为审计用，不进 PUT。
      const { extras, ...patchForPut } = (patch as Record<string, unknown>) ?? {};
      if (target.kind === "chunk") {
        const chunk = knowledgeChunks.find((c) => c.id === target.id);
        if (!chunk) {
          throw new Error("找不到目标切片");
        }
        const merged = mergeChunkPatch(chunk, patchForPut);
        await api.put(`/api/operation-knowledge/chunks/${target.id}`, {
          accountId: currentAccountId || undefined,
          ...merged
        });
        if (options.thenVerify) {
          await api.post(`/api/operation-knowledge/chunks/${target.id}/verify`, {});
        }
      } else {
        const pack = operationKnowledge.find((p) => p.id === target.id);
        if (!pack) {
          throw new Error("找不到目标知识包");
        }
        const merged = mergePackPatch(pack, patchForPut);
        await api.put(`/api/operation-knowledge/${target.id}`, merged);
      }
      // 2) 写一条"AI 修复落库"审计事件——闭合 propose → answer → applied 链路，
      //    并把 extras（schema 暂无容器、本轮未持久化进业务字段的领域专属建议）
      //    带进事件 details，便于后续审计回放与 extras 持久化方案落地。
      try {
        await api.post(`/api/operation-knowledge/repair/applied`, {
          targetKind: target.kind,
          targetId: target.id,
          sessionId: auditMeta.sessionId ?? null,
          turn: auditMeta.turn ?? null,
          acceptedFields: auditMeta.acceptedFields ?? [],
          skippedFields: auditMeta.skippedFields ?? [],
          confidenceHint: auditMeta.confidenceHint ?? null,
          extras: extras ?? null,
          thenVerify: !!options.thenVerify
        });
      } catch (err) {
        // 审计上报失败不影响业务落库；只在控制台留痕，不打断 UX。
        console.warn("[ai-repair] applied event upload failed", err);
      }
      await loadAll();
    });
  }

  async function rejectKnowledgeChunk(id: string) {
    await run(async () => {
      await api.post(`/api/operation-knowledge/chunks/${id}/reject`, {});
      if (editingChunkId === id) newChunkDraft();
      setChunkSource(null);
      await loadAll();
    });
  }

  async function loadKnowledgeChunkSource(id: string) {
    await run(async () => {
      const data = await api.get<{ chunk: OperationKnowledgeChunk; document?: OperationKnowledgeDocument }>(
        `/api/operation-knowledge/chunks/${id}/source`
      );
      setChunkSource(data as unknown as Record<string, unknown>);
    });
  }

  async function runKnowledgeTest() {
    if (!knowledgeTestMessage.trim()) return;
    await run(async () => {
      const data = await api.post<{ item: Record<string, unknown> }>("/api/operation-knowledge/test-match", {
        accountId: currentAccountId,
        contactId: selected?.id,
        message: knowledgeTestMessage
      });
      setKnowledgeTestResult(data.item);
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

  function editKnowledge(item: OperationKnowledgeItem) {
    setEditingKnowledgeId(item.id);
    setKnowledgeDraft({
      category: item.category,
      businessType: item.businessType,
      knowledgeType: item.knowledgeType ?? "",
      businessContext: item.businessContext ?? "",
      title: item.title,
      summary: item.summary ?? "",
      body: item.body ?? "",
      routingCard: item.routingCard ?? "",
      applicableScenes: (item.applicableScenes || []).join(", "),
      notApplicableScenes: (item.notApplicableScenes || []).join(", "),
      suitableFor: (item.suitableFor || []).join(", "),
      notSuitableFor: (item.notSuitableFor || []).join(", "),
      customerStages: (item.customerStages || []).join(", "),
      operationStates: (item.operationStates || []).join(", "),
      intentLevels: (item.intentLevels || []).join(", "),
      commonQuestions: (item.commonQuestions || []).join("\n"),
      commonObjections: (item.commonObjections || []).join(", "),
      safeClaims: (item.safeClaims || []).join("\n"),
      forbiddenClaims: (item.forbiddenClaims || []).join("\n"),
      evidenceItems: (item.evidenceItems || []).join("\n"),
      productTags: (item.productTags || []).join(", "),
      triggerKeywords: (item.triggerKeywords || []).join(", "),
      businessTopics: (item.businessTopics || []).join(", "),
      sourceName: item.sourceName ?? "",
      status: item.status,
      priority: String(item.priority ?? 0)
    });
    setKnowledgeSelectedNodeId(`pack:${item.id}`);
    setKnowledgePackSubview("metadata");
    setKnowledgeWorkspaceMode("selection");
    setActiveChannel("knowledge");
  }

  function newKnowledgeDraft() {
    setEditingKnowledgeId("");
    setKnowledgeDraft(emptyKnowledgeDraft());
  }

  function editChunk(item: OperationKnowledgeChunk) {
    setEditingChunkId(item.id);
    setChunkDraft(draftFromChunk(item));
    setKnowledgeSelectedNodeId(`chunk:${item.id}`);
    setKnowledgeWorkspaceMode("selection");
    setActiveChannel("knowledge");
  }

  function newChunkDraft() {
    setEditingChunkId("");
    setChunkDraft(emptyChunkDraft());
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

  // P1-8：currentAccountId 一旦解析或切换，从 per-account localStorage 读 sessionId。
  // - 首次进入：从 `wechatagent.knowledgeChatSessionId.{accountId}` 读历史 session（无则 undefined）。
  // - 切号：丢掉前一账号的 sessionId state，避免「号 A 的 chat 历史显示在号 B」。
  useEffect(() => {
    if (!currentAccountId) {
      setKnowledgeChatSessionId(undefined);
      return;
    }
    setKnowledgeChatSessionId(readPersistedChatSession(currentAccountId));
  }, [currentAccountId]);

  useEffect(() => {
    if (activeChannel !== "knowledge") return;
    if (digestPhase === "loading" || digestPhase === "regenerating") return;
    if (digestReport && digestReport.reportDate === todayLocalDate()) return;
    setDigestPhase("loading");
    setDigestError(null);
    void getDigestToday()
      .then((data) => {
        setDigestReport(data);
        setDigestPhase("idle");
      })
      .catch((err) => {
        setDigestError(err instanceof Error ? err : new Error(String(err)));
        setDigestPhase("error");
      });
  }, [activeChannel, currentAccountId]);

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
                  knowledge={operationKnowledge}
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
                  />
                )}

                {traditionalOpsTab === "audit" && (
                  <OperationsView decisionReviews={decisionReviews} events={events} llmUsage={llmUsage} opsTab={opsTab} tasks={tasks} onOpsTab={setOpsTab} />
                )}
              </>
            )}
          </section>
        )}

        {activeChannel === "knowledge" && (
          <div className="knowledgeShell">
            <header className="knowledgeShell__topbar">
              <div className="knowledgeShell__title">运营知识库 · AI 自主补完工作站</div>
              <div className="knowledgeShell__seg" role="tablist" aria-label="知识库工作模式">
                <button
                  type="button"
                  role="tab"
                  aria-selected={knowledgeMode === "ai"}
                  className="knowledgeShell__segBtn"
                  onClick={() => setKnowledgeMode("ai")}
                >
                  <Sparkles size={14} /> AI 协作
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={knowledgeMode === "manual"}
                  className="knowledgeShell__segBtn"
                  onClick={() => setKnowledgeMode("manual")}
                >
                  <SquarePen size={14} /> 手动
                </button>
              </div>
            </header>

            {knowledgeMode === "ai" ? (
              <div className="knowledgeShell__body">
                <div className="knowledgeShell__main">
                  <AiInboxStrip
                    onOpenChat={(injection) => {
                      if (injection) setDigestPendingChatInjection(injection);
                    }}
                    onOpenManual={(target) => {
                      setKnowledgeMode("manual");
                      if (target?.kind === "chunk" && target.id) {
                        setKnowledgeSelectedNodeId(`chunk:${target.id}`);
                        const found = knowledgeChunks.find((c) => c.id === target.id);
                        if (found) {
                          setEditingChunkId(found.id);
                          setChunkDraft(draftFromChunk(found));
                          setKnowledgeWorkspaceMode("selection");
                        }
                      } else if (target?.kind === "pack" && target.id) {
                        setKnowledgeSelectedNodeId(`pack:${target.id}`);
                      }
                    }}
                    onDismissDigest={(cardId) => void dismissDigestCardAndRefresh(cardId)}
                  />
                  <KnowledgeDigestCanvas
                    report={digestReport}
                    phase={digestPhase}
                    error={digestError}
                    selectedCardIds={digestSelectedCardIds}
                    onToggleSelect={toggleDigestCardSelected}
                    onSelectAll={digestSelectAll}
                    onInvertSelect={digestInvertSelect}
                    onIgnoreInfo={digestIgnoreInfoCards}
                    onRegenerate={regenerateDigestNow}
                    onDismissCard={dismissDigestCardAndRefresh}
                    onDispatchSelected={dispatchSelectedCardsToChat}
                    onDispatchOne={dispatchOneCardToChat}
                    onOpenCard={openCardTarget}
                  />
                  <div className="chatCanvas">
                    <KnowledgeChatPanel
                      open={true}
                      mode="docked"
                      initialSessionId={knowledgeChatSessionId}
                      accountId={currentAccountId || undefined}
                      pendingInjection={digestPendingChatInjection}
                      onInjectionConsumed={() => setDigestPendingChatInjection(null)}
                      onClose={(sid) => {
                        if (sid) {
                          setKnowledgeChatSessionId(sid);
                          writePersistedChatSession(currentAccountId, sid);
                        }
                      }}
                      onApplied={() => {
                        void loadAll();
                      }}
                      postTurn={postKnowledgeChatTurn}
                      getHistory={getKnowledgeChatHistory}
                      apply={applyKnowledgeChat}
                      discard={discardKnowledgeChat}
                      postChatTask={postChatTask}
                    />
                  </div>
                </div>
                <aside className="statusRail" aria-label="库健康与当前草稿">
                  <div className="statusRail__card">
                    <div className="statusRail__cardTitle">库健康</div>
                    <div className="statusRail__big">
                      {knowledgeCompleteness && (knowledgeCompleteness.totalChunks ?? 0) > 0
                        ? `${Math.round(
                            (knowledgeCompleteness.verifiedChunks / knowledgeCompleteness.totalChunks) * 100
                          )}%`
                        : "—"}
                    </div>
                    <div className="statusRail__row">
                      <span className="statusRail__rowMuted">待审 chunk</span>
                      <span>{knowledgeIntegrity?.needsReview ?? 0}</span>
                    </div>
                    <div className="statusRail__row">
                      <span className="statusRail__rowMuted">已核验</span>
                      <span>{knowledgeIntegrity?.verified ?? 0}</span>
                    </div>
                    <div className="statusRail__row">
                      <span className="statusRail__rowMuted">已驳回</span>
                      <span>{knowledgeIntegrity?.rejected ?? 0}</span>
                    </div>
                    <button
                      type="button"
                      className="ghostButton"
                      onClick={() => void refreshKnowledgeCompleteness()}
                      disabled={busy}
                    >
                      <RefreshCw size={12} /> 一键重算
                    </button>
                  </div>
                  <div className="statusRail__card">
                    <div className="statusRail__cardTitle">今日日报</div>
                    <div className="statusRail__row">
                      <span className="statusRail__rowMuted">待处理</span>
                      <span>
                        {(digestReport?.cards || []).filter(
                          (c) => !(digestReport?.dismissedCardIds || []).includes(c.cardId)
                        ).length}
                      </span>
                    </div>
                    <div className="statusRail__row">
                      <span className="statusRail__rowMuted">已选派</span>
                      <span>{digestSelectedCardIds.length}</span>
                    </div>
                    <button
                      type="button"
                      className="ghostButton"
                      onClick={() => void regenerateDigestNow()}
                      disabled={digestPhase === "regenerating" || digestPhase === "loading"}
                    >
                      <Sparkles size={12} /> 让 AI 重算
                    </button>
                  </div>
                </aside>
              </div>
            ) : (
              <div className="knowledgeShell__manual">
                <div className="knowledgeShell__manualHint">
                  <span>手动浏览模式：使用左侧树查阅与编辑切片，需要 AI 协作时点右上角段控件切回。</span>
                  <button
                    type="button"
                    className="ghostButton"
                    onClick={() => setKnowledgeMode("ai")}
                  >
                    <Sparkles size={12} /> 改用 AI 协作
                  </button>
                </div>
                <OperationKnowledgeView
                  busy={busy}
                  catalog={knowledgeCatalog}
                  completeness={knowledgeCompleteness}
                  chunkDraft={chunkDraft}
                  chunkSource={chunkSource}
                  chunks={knowledgeChunks}
                  documents={knowledgeDocuments}
                  editingChunkId={editingChunkId}
                  editingKnowledgeId={editingKnowledgeId}
                  integrityReport={knowledgeIntegrity}
                  importPreview={knowledgeImportPreview}
                  importSource={knowledgeImportSource}
                  importText={knowledgeImportText}
                  knowledge={operationKnowledge}
                  knowledgeDraft={knowledgeDraft}
                  testMessage={knowledgeTestMessage}
                  testResult={knowledgeTestResult}
                  usage={knowledgeUsage}
                  tree={knowledgeTree}
                  selectedNodeId={knowledgeSelectedNodeId}
                  expandedNodes={knowledgeExpandedNodes}
                  treeSearch={knowledgeTreeSearch}
                  packSubview={knowledgePackSubview}
                  workspaceMode={knowledgeWorkspaceMode}
                  debugDrawerOpen={knowledgeDebugDrawerOpen}
                  onSelectNode={(id) => {
                    setKnowledgeSelectedNodeId(id);
                    if (id === null) {
                      return;
                    }
                    if (id.startsWith("chunk:")) {
                      const chunkId = id.slice("chunk:".length);
                      const found = knowledgeChunks.find((c) => c.id === chunkId);
                      if (found) {
                        setEditingChunkId(found.id);
                        setChunkDraft(draftFromChunk(found));
                        setKnowledgeWorkspaceMode("selection");
                      }
                    } else if (id.startsWith("pack:")) {
                      if (editingChunkId) {
                        setEditingChunkId("");
                        setChunkDraft(emptyChunkDraft());
                      }
                    } else if (id.startsWith("doc:")) {
                      if (editingChunkId) {
                        setEditingChunkId("");
                        setChunkDraft(emptyChunkDraft());
                      }
                      if (editingKnowledgeId) {
                        setEditingKnowledgeId("");
                        setKnowledgeDraft(emptyKnowledgeDraft());
                      }
                    }
                  }}
                  onToggleNode={(id) => {
                    setKnowledgeExpandedNodes((prev) => {
                      const next = new Set(prev);
                      if (next.has(id)) next.delete(id);
                      else next.add(id);
                      return next;
                    });
                    if (id.startsWith("doc:")) {
                      const docId = id.slice(4);
                      void listChunksByDocument(docId)
                        .then((chunks) => {
                          if (!chunks.length) return;
                          setKnowledgeChunks((prev) => {
                            const byId = new Map(prev.map((c) => [c.id, c]));
                            for (const c of chunks) byId.set(c.id, c);
                            return Array.from(byId.values());
                          });
                        })
                        .catch(() => {
                          /* per-doc 拉取失败不阻塞 UI */
                        });
                    }
                  }}
                  onCollapseAll={() => setKnowledgeExpandedNodes(new Set())}
                  onTreeSearch={setKnowledgeTreeSearch}
                  onPackSubview={setKnowledgePackSubview}
                  onWorkspaceMode={setKnowledgeWorkspaceMode}
                  onOpenDebugDrawer={() => setKnowledgeDebugDrawerOpen(true)}
                  onCloseDebugDrawer={() => setKnowledgeDebugDrawerOpen(false)}
                  onOpenChat={() => {
                    setKnowledgeMode("ai");
                    openKnowledgeChat();
                  }}
                  onOpenCreateDocModal={openKnowledgeDocCreateModal}
                  onOpenEditDocModal={openKnowledgeDocEditModal}
                  onRefreshCompleteness={() => void refreshKnowledgeCompleteness()}
                  onApplyImport={() => void applyKnowledgeImport()}
                  onChunkDraft={setChunkDraft}
                  onCreateKnowledge={createKnowledge}
                  onCreateKnowledgeChunk={createKnowledgeChunk}
                  onDeleteDocument={(id) => void deleteKnowledgeDocument(id)}
                  onDeleteKnowledgeChunk={(id) => void deleteKnowledgeChunk(id)}
                  onDeleteKnowledge={(id) => void deleteKnowledge(id)}
                  onEditChunk={editChunk}
                  onEditKnowledge={editKnowledge}
                  onImportSource={setKnowledgeImportSource}
                  onImportText={setKnowledgeImportText}
                  onKnowledgeDraft={setKnowledgeDraft}
                  onLoadChunkSource={(id) => void loadKnowledgeChunkSource(id)}
                  onNewChunk={newChunkDraft}
                  onNewKnowledge={newKnowledgeDraft}
                  onPreviewImport={() => void previewKnowledgeImport()}
                  onRejectChunk={(id) => void rejectKnowledgeChunk(id)}
                  onRunTest={() => void runKnowledgeTest()}
                  onSaveKnowledgeChunk={saveKnowledgeChunk}
                  onSaveKnowledge={saveKnowledge}
                  onTestMessage={setKnowledgeTestMessage}
                  onVerifyChunk={(id) => void verifyKnowledgeChunk(id)}
                  onAiRepair={(target) => setAiRepairTarget(target)}
                  onExtractTags={(id) => void extractTagsForChunk(id)}
                />
              </div>
            )}
          </div>
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
      {aiRepairTarget && (
        <AiRepairPanel
          target={aiRepairTarget}
          chunks={knowledgeChunks}
          packs={operationKnowledge}
          onClose={() => setAiRepairTarget(null)}
          proposeChunk={proposeChunkRepair}
          answerChunk={answerChunkRepair}
          proposePack={proposePackRepair}
          onApply={applyAiRepairPatch}
        />
      )}
      {activeChannel !== "knowledge" && (
        <KnowledgeChatPanel
          open={knowledgeChatOpen}
          mode="drawer"
          initialSessionId={knowledgeChatSessionId}
          accountId={currentAccountId || undefined}
          onClose={(sid) => closeKnowledgeChat(sid)}
          onApplied={() => {
            void loadAll();
          }}
          postTurn={postKnowledgeChatTurn}
          getHistory={getKnowledgeChatHistory}
          apply={applyKnowledgeChat}
          discard={discardKnowledgeChat}
          postChatTask={postChatTask}
        />
      )}
      {knowledgeDocModal && (
        <KnowledgeDocumentModal
          state={knowledgeDocModal}
          onChange={setKnowledgeDocModal}
          onClose={closeKnowledgeDocModal}
          onSubmit={() => void submitKnowledgeDocModal()}
        />
      )}
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
  knowledge,
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
  knowledge: OperationKnowledgeItem[];
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
  const activeKnowledge = knowledge.filter((item) => item.status === "active");
  const evidenceBackedKnowledge = activeKnowledge.filter((item) => item.evidenceItems.length > 0).length;
  const riskBoundaries = activeKnowledge.filter((item) => item.forbiddenClaims.length > 0).length;
  const examples = [
    "更像朋友一点，不要太销售",
    "这个客户比较忙，降低主动打扰频率",
    "他已经有明确需求，可以更积极推进下一步",
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
            <div className="sectionCaption">知识库状态</div>
            <div className="profileGrid compactGrid">
              <div>
                <span>可用知识包</span>
                <p>{activeKnowledge.length} 个 active 知识包</p>
              </div>
              <div>
                <span>证据支撑</span>
                <p>{evidenceBackedKnowledge ? `${evidenceBackedKnowledge} 个知识包含证据` : "缺失，涉及事实承诺时应保守回应"}</p>
              </div>
              <div>
                <span>风险边界</span>
                <p>{riskBoundaries ? `${riskBoundaries} 个知识包配置了禁止承诺` : "建议补充禁止承诺与不可说法"}</p>
              </div>
            </div>
          </section>

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
                <span>关系阶段</span>
                <p>{selected.customerStage || selected.operationState || "待判断"}</p>
              </div>
              <div>
                <span>产品匹配</span>
                <p>{selected.intentLevel || memoryDraft.fitReason || "未知，需要继续通过对话确认。"}</p>
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
              <span>事实风险：{reviewScores.factRisk ?? "-"}</span>
              <span>压迫风险：{reviewScores.pressureRisk ?? "-"}</span>
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

function OperationKnowledgeView({
  busy,
  catalog,
  completeness,
  chunkDraft,
  chunkSource,
  chunks,
  documents,
  editingChunkId,
  editingKnowledgeId,
  integrityReport,
  importPreview,
  importSource,
  importText,
  knowledge,
  knowledgeDraft,
  testMessage,
  testResult,
  usage,
  tree,
  selectedNodeId,
  expandedNodes,
  treeSearch,
  packSubview,
  workspaceMode,
  debugDrawerOpen,
  onSelectNode,
  onToggleNode,
  onCollapseAll,
  onTreeSearch,
  onPackSubview,
  onWorkspaceMode,
  onOpenDebugDrawer,
  onCloseDebugDrawer,
  onOpenChat,
  onOpenCreateDocModal,
  onOpenEditDocModal,
  onRefreshCompleteness,
  onApplyImport,
  onChunkDraft,
  onCreateKnowledge,
  onCreateKnowledgeChunk,
  onDeleteDocument,
  onDeleteKnowledge,
  onDeleteKnowledgeChunk,
  onEditChunk,
  onEditKnowledge,
  onImportSource,
  onImportText,
  onKnowledgeDraft,
  onLoadChunkSource,
  onNewChunk,
  onNewKnowledge,
  onPreviewImport,
  onRejectChunk,
  onRunTest,
  onSaveKnowledgeChunk,
  onSaveKnowledge,
  onTestMessage,
  onVerifyChunk,
  onAiRepair,
  onExtractTags
}: {
  busy: boolean;
  catalog: Record<string, unknown> | null;
  completeness: KnowledgeCompletenessReport | null;
  chunkDraft: OperationKnowledgeChunkDraft;
  chunkSource: Record<string, unknown> | null;
  chunks: OperationKnowledgeChunk[];
  documents: OperationKnowledgeDocument[];
  editingChunkId: string;
  editingKnowledgeId: string;
  integrityReport: KnowledgeIntegrityReport | null;
  importPreview: OperationKnowledgeImportPreview;
  importSource: string;
  importText: string;
  knowledge: OperationKnowledgeItem[];
  knowledgeDraft: OperationKnowledgeDraft;
  testMessage: string;
  testResult: Record<string, unknown> | null;
  usage: KnowledgeUsageItem[];
  tree: TreeNode[];
  selectedNodeId: string | null;
  expandedNodes: Set<string>;
  treeSearch: string;
  packSubview: "overview" | "metadata" | "chunks";
  workspaceMode: "selection" | "import";
  debugDrawerOpen: boolean;
  onSelectNode: (id: string | null) => void;
  onToggleNode: (id: string) => void;
  onCollapseAll: () => void;
  onTreeSearch: (value: string) => void;
  onPackSubview: (sub: "overview" | "metadata" | "chunks") => void;
  onWorkspaceMode: (mode: "selection" | "import") => void;
  onOpenDebugDrawer: () => void;
  onCloseDebugDrawer: () => void;
  onOpenChat: () => void;
  onOpenCreateDocModal: () => void;
  onOpenEditDocModal: (doc: OperationKnowledgeDocument) => void;
  onRefreshCompleteness: () => void;
  onApplyImport: () => void;
  onChunkDraft: (draft: OperationKnowledgeChunkDraft) => void;
  onCreateKnowledge: (event: FormEvent) => void;
  onCreateKnowledgeChunk: (event: FormEvent) => void;
  onDeleteDocument: (id: string) => void;
  onDeleteKnowledge: (id: string) => void;
  onDeleteKnowledgeChunk: (id: string) => void;
  onEditChunk: (item: OperationKnowledgeChunk) => void;
  onEditKnowledge: (item: OperationKnowledgeItem) => void;
  onImportSource: (value: string) => void;
  onImportText: (value: string) => void;
  onKnowledgeDraft: (draft: OperationKnowledgeDraft) => void;
  onLoadChunkSource: (id: string) => void;
  onNewChunk: () => void;
  onNewKnowledge: () => void;
  onPreviewImport: () => void;
  onRejectChunk: (id: string) => void;
  onRunTest: () => void;
  onSaveKnowledgeChunk: (event: FormEvent) => void;
  onSaveKnowledge: (event: FormEvent) => void;
  onTestMessage: (value: string) => void;
  onVerifyChunk: (id: string) => void;
  onAiRepair: (target: { kind: "chunk" | "pack"; id: string; label: string }) => void;
  onExtractTags: (chunkId: string) => void;
}) {
  const activeChunks = chunks.filter((item) => item.status === "active").length;
  const verifiedChunks = chunks.filter((item) => item.integrityStatus === "verified").length;
  const evidenceItems = chunks.filter((item) => item.evidenceItems.length > 0).length;

  return (
    <section className="knowledgeWorkspace">
      <section className="knowledgeHeader panel">
        <div>
          <span>Operation Knowledge</span>
          <h2>Agent 自主查询知识系统</h2>
          <p>文档先变成目录，目录再拆成切片；用户运营 Agent 每轮自主选择要打开的知识，而不是把整库塞进上下文。</p>
        </div>
        <div className="knowledgeStats">
          <div><strong>{documents.length}</strong><span>文档</span></div>
          <div><strong>{knowledge.length}</strong><span>知识包</span></div>
          <div><strong>{activeChunks}</strong><span>切片启用</span></div>
          <div><strong>{verifiedChunks}/{evidenceItems}</strong><span>已验证/证据</span></div>
        </div>
        <div className="knowledgeWorkspaceHeadActions">
          <button type="button" className="primary" onClick={onOpenChat}>
            <MessageSquareText size={14} /> 与 AI 对话补完
          </button>
          <button type="button" className="secondary" onClick={onOpenCreateDocModal}>
            <FileText size={14} /> 手动新建文档
          </button>
          <button type="button" className="secondary" onClick={onOpenDebugDrawer}>
            <FlaskConical size={14} /> 运行命中测试
          </button>
        </div>
      </section>

      {completeness && (
        <section className={`knowledgeReadiness ${completeness.answeringMode}`}>
          <div>
            <span>{knowledgeAnsweringModeLabel(completeness.answeringMode)}</span>
            <strong>{completeness.summary || "知识完整度等待评估"}</strong>
            {completeness.gaps.length > 0 && <p>{completeness.gaps.join(" / ")}</p>}
          </div>
          <div className="readinessMetrics">
            <span>verified {completeness.verifiedChunks}</span>
            <span>anchors {completeness.anchoredChunks}</span>
            <span>evidence {completeness.evidenceChunks}</span>
            <button
              type="button"
              className="iconButton"
              aria-label="重算完整度"
              title="重算完整度"
              onClick={onRefreshCompleteness}
            >
              <RefreshCw size={14} />
            </button>
          </div>
        </section>
      )}

      <div className="knowledgeMaster">
        <KnowledgeTreeSidebar
          tree={tree}
          selectedNodeId={selectedNodeId}
          expandedNodes={expandedNodes}
          search={treeSearch}
          workspaceMode={workspaceMode}
          documents={documents}
          onSelect={(id) => {
            onSelectNode(id);
            onWorkspaceMode("selection");
          }}
          onToggle={onToggleNode}
          onCollapseAll={onCollapseAll}
          onSearch={onTreeSearch}
          onImportClick={() => {
            onSelectNode(null);
            onWorkspaceMode("import");
          }}
          onEditDocument={onOpenEditDocModal}
        />
        <KnowledgeWorkspaceRouter
          tree={tree}
          selectedNodeId={selectedNodeId}
          workspaceMode={workspaceMode}
          packSubview={packSubview}
          documents={documents}
          knowledge={knowledge}
          chunks={chunks}
          catalog={catalog}
          integrityReport={integrityReport}
          importPreview={importPreview}
          importSource={importSource}
          importText={importText}
          knowledgeDraft={knowledgeDraft}
          chunkDraft={chunkDraft}
          chunkSource={chunkSource}
          editingKnowledgeId={editingKnowledgeId}
          editingChunkId={editingChunkId}
          busy={busy}
          onPackSubview={onPackSubview}
          onSelectNode={onSelectNode}
          onWorkspaceMode={onWorkspaceMode}
          onEditKnowledge={onEditKnowledge}
          onEditChunk={onEditChunk}
          onDeleteDocument={onDeleteDocument}
          onDeleteKnowledge={onDeleteKnowledge}
          onDeleteKnowledgeChunk={onDeleteKnowledgeChunk}
          onCreateKnowledge={onCreateKnowledge}
          onSaveKnowledge={onSaveKnowledge}
          onCreateKnowledgeChunk={onCreateKnowledgeChunk}
          onSaveKnowledgeChunk={onSaveKnowledgeChunk}
          onChunkDraft={onChunkDraft}
          onKnowledgeDraft={onKnowledgeDraft}
          onLoadChunkSource={onLoadChunkSource}
          onNewKnowledge={onNewKnowledge}
          onNewChunk={onNewChunk}
          onRejectChunk={onRejectChunk}
          onVerifyChunk={onVerifyChunk}
          onAiRepair={onAiRepair}
          onImportSource={onImportSource}
          onImportText={onImportText}
          onPreviewImport={onPreviewImport}
          onApplyImport={onApplyImport}
        />
      </div>

      <KnowledgeDebugDrawer
        open={debugDrawerOpen}
        onClose={onCloseDebugDrawer}
        busy={busy}
        testMessage={testMessage}
        testResult={testResult}
        usage={usage}
        onTestMessage={onTestMessage}
        onRunTest={onRunTest}
      />
    </section>
  );
}

function KnowledgeTreeSidebar({
  tree,
  selectedNodeId,
  expandedNodes,
  search,
  workspaceMode,
  documents,
  onSelect,
  onToggle,
  onCollapseAll,
  onSearch,
  onImportClick,
  onEditDocument
}: {
  tree: TreeNode[];
  selectedNodeId: string | null;
  expandedNodes: Set<string>;
  search: string;
  workspaceMode: "selection" | "import";
  documents: OperationKnowledgeDocument[];
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
  onCollapseAll: () => void;
  onSearch: (value: string) => void;
  onImportClick: () => void;
  onEditDocument: (doc: OperationKnowledgeDocument) => void;
}) {
  const filterMatch = (node: TreeNode): boolean => {
    if (!search.trim()) return true;
    const q = search.trim().toLowerCase();
    if (node.label.toLowerCase().includes(q)) return true;
    if (node.meta && node.meta.toLowerCase().includes(q)) return true;
    if (node.badges && node.badges.some((b) => b.text.toLowerCase().includes(q))) return true;
    if (node.children) return node.children.some(filterMatch);
    return false;
  };

  const visibleRoots = tree.filter(filterMatch);
  const isSearching = search.trim().length > 0;

  return (
    <section className="knowledgeTree">
      <div className="knowledgeTree__toolbar">
        <div className="knowledgeTree__searchWrap">
          <Search size={14} />
          <input
            type="text"
            className="knowledgeTree__search"
            placeholder="搜索文档 / 知识包 / 切片"
            value={search}
            onChange={(e) => onSearch(e.target.value)}
          />
        </div>
        <button
          type="button"
          className={`knowledgeTree__iconBtn primary${workspaceMode === "import" ? " active" : ""}`}
          aria-label="导入文档"
          onClick={onImportClick}
        >
          <UploadCloud size={14} />
        </button>
        <button
          type="button"
          className="knowledgeTree__iconBtn"
          aria-label="折叠全部"
          onClick={onCollapseAll}
        >
          <ChevronRight size={14} />
        </button>
      </div>
      <div className="knowledgeTree__list">
        {!visibleRoots.length ? (
          <div className="knowledgeTree__empty">
            {isSearching ? "无匹配项" : "暂无知识资产 · 点右上 ⤴ 导入第一份文档"}
          </div>
        ) : (
          visibleRoots.map((node) => (
            <KnowledgeTreeNodeRow
              key={node.id}
              node={node}
              depth={0}
              selectedNodeId={selectedNodeId}
              expandedNodes={expandedNodes}
              search={search}
              documents={documents}
              onSelect={onSelect}
              onToggle={onToggle}
              onEditDocument={onEditDocument}
            />
          ))
        )}
      </div>
    </section>
  );
}

function KnowledgeTreeNodeRow({
  node,
  depth,
  selectedNodeId,
  expandedNodes,
  search,
  documents,
  onSelect,
  onToggle,
  onEditDocument
}: {
  node: TreeNode;
  depth: number;
  selectedNodeId: string | null;
  expandedNodes: Set<string>;
  search: string;
  documents: OperationKnowledgeDocument[];
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
  onEditDocument: (doc: OperationKnowledgeDocument) => void;
}) {
  const hasChildren = !!node.children && node.children.length > 0;
  const isSearching = search.trim().length > 0;
  const expanded = isSearching ? true : expandedNodes.has(node.id);
  const isSelected = selectedNodeId === node.id;
  const iconCls =
    node.kind === "document"
      ? "doc"
      : node.kind === "pack"
      ? "pack"
      : node.kind === "chunk"
      ? "chunk"
      : "warn";
  const Icon =
    node.kind === "document"
      ? FileText
      : node.kind === "pack"
      ? Package
      : node.kind === "chunk"
      ? FileBox
      : AlertTriangle;

  function handleNodeClick(e: React.MouseEvent) {
    e.stopPropagation();
    onSelect(node.id);
    if (hasChildren && !expanded && !isSearching) onToggle(node.id);
  }

  function handleCaretClick(e: React.MouseEvent) {
    e.stopPropagation();
    if (hasChildren) onToggle(node.id);
  }

  return (
    <>
      <div
        className={`knowledgeTree__node${isSelected ? " selected" : ""}`}
        style={{ paddingLeft: 12 + depth * 16 }}
        onClick={handleNodeClick}
        role="button"
        tabIndex={0}
        title={node.label}
      >
        <span className={`knowledgeTree__caret${hasChildren ? "" : " placeholder"}`} onClick={handleCaretClick}>
          {hasChildren && (expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />)}
        </span>
        <span className={`knowledgeTree__icon ${iconCls}`}>
          <Icon size={14} />
        </span>
        <span className="knowledgeTree__body">
          <span className="knowledgeTree__labelRow">
            <span className="knowledgeTree__label">{node.label}</span>
            {node.badges?.map((b, i) => (
              <span key={i} className={`knowledgeTree__badge tone-${b.tone}`}>{b.text}</span>
            ))}
          </span>
          {node.meta && <span className="knowledgeTree__meta">{node.meta}</span>}
        </span>
        {node.kind === "document" && node.refId && (
          <button
            type="button"
            className="knowledgeTree__nodeAction"
            aria-label="编辑文档元数据"
            title="编辑文档元数据"
            onClick={(e) => {
              e.stopPropagation();
              const doc = documents.find((d) => d.id === node.refId);
              if (doc) onEditDocument(doc);
            }}
          >
            <SquarePen size={12} />
          </button>
        )}
      </div>
      {hasChildren && expanded && node.children!.map((child) => (
        <KnowledgeTreeNodeRow
          key={child.id}
          node={child}
          depth={depth + 1}
          selectedNodeId={selectedNodeId}
          expandedNodes={expandedNodes}
          search={search}
          documents={documents}
          onSelect={onSelect}
          onToggle={onToggle}
          onEditDocument={onEditDocument}
        />
      ))}
    </>
  );
}

function KnowledgeWorkspaceRouter({
  tree,
  selectedNodeId,
  workspaceMode,
  packSubview,
  documents,
  knowledge,
  chunks,
  catalog,
  integrityReport,
  importPreview,
  importSource,
  importText,
  knowledgeDraft,
  chunkDraft,
  chunkSource,
  editingKnowledgeId,
  editingChunkId,
  busy,
  onPackSubview,
  onSelectNode,
  onWorkspaceMode,
  onEditKnowledge,
  onEditChunk,
  onDeleteDocument,
  onDeleteKnowledge,
  onDeleteKnowledgeChunk,
  onCreateKnowledge,
  onSaveKnowledge,
  onCreateKnowledgeChunk,
  onSaveKnowledgeChunk,
  onChunkDraft,
  onKnowledgeDraft,
  onLoadChunkSource,
  onNewKnowledge,
  onNewChunk,
  onRejectChunk,
  onVerifyChunk,
  onAiRepair,
  onExtractTags,
  onImportSource,
  onImportText,
  onPreviewImport,
  onApplyImport
}: {
  tree: TreeNode[];
  selectedNodeId: string | null;
  workspaceMode: "selection" | "import";
  packSubview: "overview" | "metadata" | "chunks";
  documents: OperationKnowledgeDocument[];
  knowledge: OperationKnowledgeItem[];
  chunks: OperationKnowledgeChunk[];
  catalog: Record<string, unknown> | null;
  integrityReport: KnowledgeIntegrityReport | null;
  importPreview: OperationKnowledgeImportPreview;
  importSource: string;
  importText: string;
  knowledgeDraft: OperationKnowledgeDraft;
  chunkDraft: OperationKnowledgeChunkDraft;
  chunkSource: Record<string, unknown> | null;
  editingKnowledgeId: string;
  editingChunkId: string;
  busy: boolean;
  onPackSubview: (s: "overview" | "metadata" | "chunks") => void;
  onSelectNode: (id: string | null) => void;
  onWorkspaceMode: (mode: "selection" | "import") => void;
  onEditKnowledge: (item: OperationKnowledgeItem) => void;
  onEditChunk: (item: OperationKnowledgeChunk) => void;
  onDeleteDocument: (id: string) => void;
  onDeleteKnowledge: (id: string) => void;
  onDeleteKnowledgeChunk: (id: string) => void;
  onCreateKnowledge: (e: FormEvent) => void;
  onSaveKnowledge: (e: FormEvent) => void;
  onCreateKnowledgeChunk: (e: FormEvent) => void;
  onSaveKnowledgeChunk: (e: FormEvent) => void;
  onChunkDraft: (d: OperationKnowledgeChunkDraft) => void;
  onKnowledgeDraft: (d: OperationKnowledgeDraft) => void;
  onLoadChunkSource: (id: string) => void;
  onNewKnowledge: () => void;
  onNewChunk: () => void;
  onRejectChunk: (id: string) => void;
  onVerifyChunk: (id: string) => void;
  onAiRepair: (target: { kind: "chunk" | "pack"; id: string; label: string }) => void;
  onExtractTags?: (chunkId: string) => void;
  onImportSource: (v: string) => void;
  onImportText: (v: string) => void;
  onPreviewImport: () => void;
  onApplyImport: () => void;
}) {
  const previewCount = (importPreview.document ? 1 : 0) + importPreview.items.length + importPreview.chunks.length;
  const selectedNode = selectedNodeId ? findTreeNode(tree, selectedNodeId) : null;

  if (workspaceMode === "import") {
    return (
      <section className="knowledgeWorkspaceRouter">
        <div className="knowledgeWorkspaceHead">
          <div>
            <span style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>Import</span>
            <h2>导入文档并生成渐进式目录</h2>
            <p>粘贴产品说明、服务边界、FAQ、案例证据或运营 SOP；AI 会自动拆出文档目录、知识包与切片，确认入库后即可在左侧树中查看。</p>
          </div>
          <div className="knowledgeWorkspaceHeadActions">
            <button type="button" className="secondary" onClick={() => onWorkspaceMode("selection")}>
              <X size={14} /> 取消
            </button>
          </div>
        </div>
        <div className="formGrid">
          <label>
            <span>来源名称</span>
            <input value={importSource} onChange={(e) => onImportSource(e.target.value)} />
          </label>
          <label className="fullSpan">
            <span>文档内容</span>
            <textarea
              className="largeTextArea"
              value={importText}
              onChange={(e) => onImportText(e.target.value)}
              placeholder="粘贴产品说明、服务边界、FAQ、案例证据或运营 SOP。第一版支持文本和 Markdown。"
            />
          </label>
        </div>
        <div className="buttonRow">
          <button type="button" onClick={onPreviewImport} disabled={busy || !importText.trim()}>
            <Sparkles size={14} /> AI 生成目录和切片
          </button>
          <button type="button" className="secondary" onClick={onApplyImport} disabled={busy || !previewCount}>
            确认入库
          </button>
        </div>
        <div className="knowledgeDrawer__section">
          <h4>AI 结构化预览</h4>
          <div className="assetList">
            {importPreview.document && (
              <div className="assetRow">
                <strong>{importPreview.document.title}</strong>
                <span>文档目录 · {importPreview.document.status}</span>
                <p>{importPreview.document.catalogSummary || importPreview.document.summary || "已生成文档入口"}</p>
              </div>
            )}
            {importPreview.integrityReport && (
              <div className="assetRow integritySummary">
                <strong>完整性校验</strong>
                <span>verified {String(importPreview.integrityReport.verified ?? 0)} / needs review {String(importPreview.integrityReport.needsReview ?? 0)}</span>
                <p>切片必须能追溯原文。未通过校验的切片入库后默认不会作为事实依据自动启用。</p>
              </div>
            )}
            {importPreview.items.map((item, index) => (
              <div key={`${item.title}-${index}`} className="assetRow">
                <strong>{item.title}</strong>
                <span>知识包 · {item.knowledgeType || item.category} · {item.businessContext || item.businessType}</span>
                <p>{item.routingCard || item.summary || item.safeClaims.join(" / ")}</p>
                <KnowledgeChipBlock label="触发关键词" values={item.triggerKeywords} />
                <KnowledgeChipBlock label="业务主题" values={item.businessTopics} />
                <KnowledgeChipBlock label="操作状态" values={item.operationStates} />
                <KnowledgeChipBlock label="意图层级" values={item.intentLevels} />
                <KnowledgeChipBlock label="适用场景" values={item.applicableScenes} />
                <KnowledgeChipBlock label="安全表达" values={item.safeClaims} />
                <KnowledgeChipBlock label="禁止表达" values={item.forbiddenClaims} muted />
              </div>
            ))}
            {importPreview.chunks.map((item, index) => (
              <div key={`${item.title}-chunk-${index}`} className="assetRow">
                <strong>{item.title}</strong>
                <span>
                  切片 · {item.knowledgeType || "AI 生成"} ·{" "}
                  {item.integrityStatus
                    ? `完整性 ${item.integrityStatus}`
                    : item.evidenceItems.length
                    ? "含证据"
                    : "无证据"}
                  {typeof item.confidenceScore === "number" ? ` · 信心 ${item.confidenceScore}` : ""}
                </span>
                <p>{item.routingCard || item.summary || item.body || "等待确认"}</p>
                <KnowledgeChipBlock label="触发关键词" values={item.triggerKeywords} />
                <KnowledgeChipBlock label="业务主题" values={item.businessTopics} />
                <KnowledgeChipBlock label="适用场景" values={item.applicableScenes} />
                <KnowledgeChipBlock label="安全表达" values={item.safeClaims} />
                <KnowledgeChipBlock label="禁止表达" values={item.forbiddenClaims} muted />
                {item.sourceQuote && (
                  <p className="knowledgeSourceQuote" title="原文锚点">
                    “{item.sourceQuote}”
                  </p>
                )}
              </div>
            ))}
            {!previewCount && <EmptyInline text="等待导入预览，输入文档内容后点 AI 生成" />}
          </div>
        </div>
      </section>
    );
  }

  if (!selectedNode) {
    return (
      <section className="knowledgeWorkspaceRouter">
        <EmptyState
          icon={<Inbox size={28} />}
          title="选择左侧资源开始"
          hint="点击文档查看 catalog；点击知识包编辑元数据 / 切片；点击切片编辑详情。也可以点左上 ⤴ 导入新文档。"
          action={
            <button type="button" onClick={() => onWorkspaceMode("import")}>
              <UploadCloud size={14} /> 导入文档
            </button>
          }
        />
      </section>
    );
  }

  if (selectedNode.kind === "document") {
    const doc = documents.find((d) => d.id === selectedNode.refId);
    if (!doc) {
      return (
        <section className="knowledgeWorkspaceRouter">
          <EmptyInline text="文档已被移除，请在左树重新选择。" />
        </section>
      );
    }
    const docPacks = (selectedNode.children || []).filter((c) => c.kind === "pack");
    const allDocChunks = chunks.filter(
      (c) =>
        c.documentId === doc.id ||
        knowledge.some((k) => docPacks.some((p) => p.refId === k.id) && k.id === c.itemId)
    );
    const verified = allDocChunks.filter((c) => c.integrityStatus === "verified").length;
    const wordEstimate = allDocChunks.reduce((n, c) => n + ((c.body?.length || 0) + (c.summary?.length || 0)), 0);
    return (
      <section className="knowledgeWorkspaceRouter">
        <div className="knowledgeBreadcrumb">
          <FileText size={12} /> <span className="crumbCurrent">{doc.title}</span>
        </div>
        <div className="knowledgeWorkspaceHead">
          <div>
            <span style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>Document</span>
            <h2>{doc.title}</h2>
            <p>{doc.catalogSummary || doc.summary || doc.sourceName || "暂无摘要"}</p>
          </div>
          <div className="knowledgeWorkspaceHeadActions">
            <button type="button" className="secondary danger" onClick={() => onDeleteDocument(doc.id)} disabled={busy}>
              <Trash2 size={14} /> 删除文档
            </button>
          </div>
        </div>
        <div className="knowledgeMetricRow">
          <div className="knowledgeMetricCard"><strong>{docPacks.length}</strong><span>知识包</span></div>
          <div className="knowledgeMetricCard"><strong>{allDocChunks.length}</strong><span>切片</span></div>
          <div className="knowledgeMetricCard"><strong>{verified}</strong><span>已验证</span></div>
          <div className="knowledgeMetricCard"><strong>{Math.round(wordEstimate / 1000)}k</strong><span>字数估算</span></div>
        </div>
        {doc.routingMap.length > 0 && (
          <div className="knowledgeDrawer__section">
            <h4>Routing Map</h4>
            <div className="metricChipRow">
              {doc.routingMap.map((r, i) => (
                <span key={i}>{r}</span>
              ))}
            </div>
          </div>
        )}
        <div className="knowledgeDrawer__section knowledgeMetaGrid">
          <KnowledgeMetaItem label="领域" value={doc.domain} />
          <KnowledgeMetaItem label="来源类型" value={doc.sourceType} />
          <KnowledgeMetaItem label="来源名" value={doc.sourceName} />
          <KnowledgeMetaItem label="状态" value={doc.status} />
        </div>
        <KnowledgeChipBlock label="触发关键词" values={doc.triggerKeywords} block />
        <KnowledgeChipBlock label="业务主题" values={doc.businessTopics} block />
        <KnowledgeChipBlock label="产品标签" values={doc.productTags} block />
        {doc.riskNotes && doc.riskNotes.length > 0 && (
          <div className="knowledgeDrawer__section">
            <h4>风险备注</h4>
            <ul className="knowledgeRiskList">
              {doc.riskNotes.map((note, i) => (
                <li key={i}>{note}</li>
              ))}
            </ul>
          </div>
        )}
        <div className="knowledgeDrawer__section">
          <h4>Catalog</h4>
          {catalog ? <StructuredJson data={catalog} defaultExpanded={2} /> : <EmptyInline text="catalog 尚未生成" />}
        </div>
      </section>
    );
  }

  if (selectedNode.kind === "pack") {
    const pack = knowledge.find((k) => k.id === selectedNode.refId);
    if (!pack) {
      return (
        <section className="knowledgeWorkspaceRouter">
          <EmptyInline text="知识包已被移除。" />
        </section>
      );
    }
    const packChunks = chunks.filter((c) => c.itemId === pack.id);
    const verified = packChunks.filter((c) => c.integrityStatus === "verified").length;
    const evidenceN = packChunks.filter((c) => c.evidenceItems.length > 0).length;
    const ownerDoc = documents.find((d) =>
      packChunks.some((c) => c.documentId === d.id)
    );
    return (
      <section className="knowledgeWorkspaceRouter">
        <div className="knowledgeBreadcrumb">
          {ownerDoc ? (
            <>
              <button type="button" onClick={() => onSelectNode(`doc:${ownerDoc.id}`)}>
                <FileText size={12} /> {ownerDoc.title}
              </button>
              <ChevronRight size={11} className="crumbSep" />
            </>
          ) : null}
          <span className="crumbCurrent"><Package size={12} /> {pack.title}</span>
        </div>
        <div className="knowledgeWorkspaceHead">
          <div>
            <span style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>Knowledge Pack</span>
            <h2>{pack.title}</h2>
            <p>{pack.routingCard || pack.summary || pack.knowledgeType || pack.category}</p>
          </div>
          <div className="knowledgeWorkspaceHeadActions">
            {editingKnowledgeId !== pack.id && (
              <button type="button" className="secondary" onClick={() => onEditKnowledge(pack)} disabled={busy}>
                <SquarePen size={14} /> 编辑元数据
              </button>
            )}
          </div>
        </div>

        <div className="knowledgePackSegmented">
          <button type="button" className={packSubview === "overview" ? "active" : ""} onClick={() => onPackSubview("overview")}>
            <LayoutDashboard size={13} /> 概览
          </button>
          <button type="button" className={packSubview === "metadata" ? "active" : ""} onClick={() => onPackSubview("metadata")}>
            <SquarePen size={13} /> 元数据
          </button>
          <button type="button" className={packSubview === "chunks" ? "active" : ""} onClick={() => onPackSubview("chunks")}>
            <FileBox size={13} /> 切片 ({packChunks.length})
          </button>
        </div>

        {packSubview === "overview" && (
          <>
            <div className="knowledgeMetricRow">
              <div className="knowledgeMetricCard"><strong>{packChunks.length}</strong><span>切片总数</span></div>
              <div className="knowledgeMetricCard"><strong>{verified}</strong><span>已验证</span></div>
              <div className="knowledgeMetricCard"><strong>{evidenceN}</strong><span>含证据</span></div>
              <div className="knowledgeMetricCard"><strong>{pack.priority}</strong><span>优先级</span></div>
            </div>
            <div className="knowledgeDrawer__section">
              <h4>简介 / 路由卡</h4>
              <p style={{ margin: 0, color: "var(--ink)", fontSize: 13, lineHeight: 1.6 }}>
                {pack.routingCard || pack.summary || "暂无路由卡。点【元数据】tab 完善"}
              </p>
            </div>
            {pack.safeClaims.length > 0 && (
              <div className="knowledgeDrawer__section">
                <h4>安全主张</h4>
                <div className="metricChipRow">
                  {pack.safeClaims.map((c, i) => (<span key={i}>{c}</span>))}
                </div>
              </div>
            )}
            {pack.forbiddenClaims.length > 0 && (
              <div className="knowledgeDrawer__section">
                <h4>禁用主张</h4>
                <div className="metricChipRow">
                  {pack.forbiddenClaims.map((c, i) => (<span key={i}>{c}</span>))}
                </div>
              </div>
            )}
            <KnowledgeChipBlock label="触发关键词" values={pack.triggerKeywords} block />
            <KnowledgeChipBlock label="业务主题" values={pack.businessTopics} block />
            <KnowledgeChipBlock label="操作状态" values={pack.operationStates} block />
            <KnowledgeChipBlock label="意图层级" values={pack.intentLevels} block />
            <KnowledgeChipBlock label="适用场景" values={pack.applicableScenes} block />
            <KnowledgeChipBlock label="不适用场景" values={pack.notApplicableScenes} block muted />
            <KnowledgeChipBlock label="常见问题" values={pack.commonQuestions} block />
            <KnowledgeChipBlock label="常见异议" values={pack.commonObjections} block />
            <KnowledgeChipBlock label="证据条目" values={pack.evidenceItems} block />
            <KnowledgeChipBlock label="产品标签" values={pack.productTags} block />
            <div className="knowledgeDrawer__section knowledgeMetaGrid">
              <KnowledgeMetaItem label="分类" value={pack.category} />
              <KnowledgeMetaItem label="业务类型" value={pack.businessType} />
              <KnowledgeMetaItem label="知识类型" value={pack.knowledgeType} />
              <KnowledgeMetaItem label="业务上下文" value={pack.businessContext} />
              <KnowledgeMetaItem label="状态" value={pack.status} />
            </div>
          </>
        )}

        {packSubview === "metadata" && editingKnowledgeId === pack.id && (
          <KnowledgeEditor
            busy={busy}
            draft={knowledgeDraft}
            editingId={editingKnowledgeId}
            onCreate={onCreateKnowledge}
            onDelete={onDeleteKnowledge}
            onDraft={onKnowledgeDraft}
            onNew={onNewKnowledge}
            onSave={onSaveKnowledge}
            onAiRepair={() =>
              onAiRepair({ kind: "pack", id: pack.id, label: pack.title || "未命名知识包" })
            }
          />
        )}
        {packSubview === "metadata" && editingKnowledgeId !== pack.id && (
          <EmptyState
            icon={<SquarePen size={26} />}
            title="点击编辑元数据"
            hint="进入编辑模式后会展示完整的字段表单。"
            action={
              <button type="button" onClick={() => onEditKnowledge(pack)} disabled={busy}>
                <SquarePen size={14} /> 编辑元数据
              </button>
            }
          />
        )}

        {packSubview === "chunks" && (
          <div className="knowledgeDrawer__section">
            <div className="knowledgeWorkspaceHead">
              <h4>切片</h4>
              <button type="button" className="secondary compactButton" onClick={onNewChunk} disabled={busy}>
                + 新建切片
              </button>
            </div>
            {packChunks.length ? (
              <div className="knowledgePackChunksGrid">
                {packChunks.map((c) => (
                  <button key={c.id} className="knowledgeChunkCard" onClick={() => onEditChunk(c)}>
                    <strong>{c.title}</strong>
                    <div className="chunkCardMeta">
                      <span>{integrityStatusLabel(c.integrityStatus)}</span>
                      <span>{c.status}</span>
                      <span>{c.evidenceItems.length} 证据</span>
                      {typeof c.confidenceScore === "number" && <span>信心 {c.confidenceScore}</span>}
                    </div>
                    {c.routingCard && <p className="chunkCardRouting">{c.routingCard}</p>}
                    <KnowledgeChipBlock label="触发关键词" values={c.triggerKeywords} />
                    <KnowledgeChipBlock label="业务主题" values={c.businessTopics} />
                    <KnowledgeChipBlock label="适用场景" values={c.applicableScenes} />
                    {c.sourceQuote && (
                      <p className="knowledgeSourceQuote" title="原文锚点">
                        “{c.sourceQuote}”
                      </p>
                    )}
                  </button>
                ))}
              </div>
            ) : (
              <EmptyInline text="此包暂无切片" />
            )}
          </div>
        )}
      </section>
    );
  }

  if (selectedNode.kind === "chunk") {
    const chunk = chunks.find((c) => c.id === selectedNode.refId);
    if (!chunk) {
      return (
        <section className="knowledgeWorkspaceRouter">
          <EmptyInline text="切片已被移除。" />
        </section>
      );
    }
    const ownerPack = chunk.itemId ? knowledge.find((k) => k.id === chunk.itemId) : null;
    const ownerDoc = chunk.documentId ? documents.find((d) => d.id === chunk.documentId) : null;
    return (
      <section className="knowledgeWorkspaceRouter">
        <div className="knowledgeBreadcrumb">
          {ownerDoc && (
            <>
              <button type="button" onClick={() => onSelectNode(`doc:${ownerDoc.id}`)}>
                <FileText size={12} /> {ownerDoc.title}
              </button>
              <ChevronRight size={11} className="crumbSep" />
            </>
          )}
          {ownerPack && (
            <>
              <button type="button" onClick={() => onSelectNode(`pack:${ownerPack.id}`)}>
                <Package size={12} /> {ownerPack.title}
              </button>
              <ChevronRight size={11} className="crumbSep" />
            </>
          )}
          <span className="crumbCurrent"><FileBox size={12} /> {chunk.title}</span>
        </div>
        <KnowledgeChunkEditor
          busy={busy}
          draft={chunkDraft}
          editingId={editingChunkId}
          onCreate={onCreateKnowledgeChunk}
          onDelete={onDeleteKnowledgeChunk}
          onDraft={onChunkDraft}
          onLoadSource={onLoadChunkSource}
          onNew={onNewChunk}
          onReject={onRejectChunk}
          onSave={onSaveKnowledgeChunk}
          onVerify={onVerifyChunk}
          onAiRepair={() =>
            onAiRepair({ kind: "chunk", id: chunk.id, label: chunk.title || "未命名切片" })
          }
          onExtractTags={onExtractTags ? () => onExtractTags(chunk.id) : undefined}
          source={chunkSource}
        />
      </section>
    );
  }

  if (selectedNode.kind === "group-orphan-packs") {
    return (
      <section className="knowledgeWorkspaceRouter">
        <div className="knowledgeWorkspaceHead">
          <div>
            <span style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>Orphan Packs</span>
            <h2>未关联文档的知识包</h2>
            <p>这些包没有与任何文档建立关联，可能是历史导入或手动新建。建议补充来源文档以便 AI 路由。</p>
          </div>
        </div>
        <div className="knowledgeOrphanList">
          {(selectedNode.children || []).map((child) => {
            const pack = knowledge.find((k) => k.id === child.refId);
            if (!pack) return null;
            return (
              <button key={pack.id} className="knowledgeChunkCard" onClick={() => onSelectNode(`pack:${pack.id}`)}>
                <strong>{pack.title}</strong>
                <div className="chunkCardMeta">
                  <span>{pack.knowledgeType || pack.category}</span>
                  <span>{pack.status}</span>
                  {pack.priority ? <span>P{pack.priority}</span> : null}
                </div>
                {pack.routingCard && <p className="chunkCardRouting">{pack.routingCard}</p>}
                <KnowledgeChipBlock label="触发" values={pack.triggerKeywords} />
                <KnowledgeChipBlock label="主题" values={pack.businessTopics} />
                <KnowledgeChipBlock label="状态机" values={pack.operationStates} />
              </button>
            );
          })}
        </div>
      </section>
    );
  }

  if (selectedNode.kind === "group-orphan-chunks") {
    return (
      <section className="knowledgeWorkspaceRouter">
        <div className="knowledgeWorkspaceHead">
          <div>
            <span style={{ fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>Orphan Chunks</span>
            <h2>未关联包的切片</h2>
            <p>这些切片没有归属到任何知识包。建议关联到知识包让 AI 检索时更精准。</p>
          </div>
        </div>
        <div className="knowledgeOrphanList">
          {(selectedNode.children || []).map((child) => {
            const c = chunks.find((cc) => cc.id === child.refId);
            if (!c) return null;
            return (
              <button key={c.id} className="knowledgeChunkCard" onClick={() => onSelectNode(`chunk:${c.id}`)}>
                <strong>{c.title}</strong>
                <div className="chunkCardMeta">
                  <span>{integrityStatusLabel(c.integrityStatus)}</span>
                  <span>{c.status}</span>
                  {typeof c.confidenceScore === "number" && <span>信心 {c.confidenceScore}</span>}
                </div>
                {c.routingCard && <p className="chunkCardRouting">{c.routingCard}</p>}
                <KnowledgeChipBlock label="触发" values={c.triggerKeywords} />
                <KnowledgeChipBlock label="主题" values={c.businessTopics} />
                {c.sourceQuote && (
                  <p className="knowledgeSourceQuote" title="原文锚点">
                    “{c.sourceQuote}”
                  </p>
                )}
              </button>
            );
          })}
        </div>
        {integrityReport && (
          <div className="knowledgeMetricRow">
            <div className="knowledgeMetricCard"><strong>{integrityReport.total}</strong><span>全部切片</span></div>
            <div className="knowledgeMetricCard"><strong>{integrityReport.verified}</strong><span>已验证</span></div>
            <div className="knowledgeMetricCard"><strong>{integrityReport.needsReview}</strong><span>AI 需复核</span></div>
            <div className="knowledgeMetricCard"><strong>{integrityReport.rejected}</strong><span>已拒绝</span></div>
          </div>
        )}
      </section>
    );
  }

  return (
    <section className="knowledgeWorkspaceRouter">
      <EmptyInline text="未知节点" />
    </section>
  );
}

function KnowledgeDebugDrawer({
  open,
  onClose,
  busy,
  testMessage,
  testResult,
  usage,
  onTestMessage,
  onRunTest
}: {
  open: boolean;
  onClose: () => void;
  busy: boolean;
  testMessage: string;
  testResult: Record<string, unknown> | null;
  usage: KnowledgeUsageItem[];
  onTestMessage: (v: string) => void;
  onRunTest: () => void;
}) {
  useEffect(() => {
    if (!open) return;
    const handler = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <>
      <div className="knowledgeDrawer__scrim" onClick={onClose} />
      <aside className="knowledgeDrawer" role="dialog" aria-label="命中测试">
        <div className="knowledgeDrawer__head">
          <h3><FlaskConical size={16} /> 知识命中测试</h3>
          <button type="button" className="knowledgeDrawer__close" aria-label="关闭" onClick={onClose}>
            <X size={16} />
          </button>
        </div>
        <div className="knowledgeDrawer__body">
          <section className="knowledgeDrawer__section">
            <h4>输入用户消息</h4>
            <textarea
              value={testMessage}
              onChange={(e) => onTestMessage(e.target.value)}
              style={{ minHeight: 96 }}
            />
            <div className="buttonRow">
              <button type="button" onClick={onRunTest} disabled={busy || !testMessage.trim()}>
                <Sparkles size={14} /> 运行知识路由
              </button>
            </div>
            {testResult && <RouteTraceMetrics result={testResult} />}
          </section>
          <section className="knowledgeDrawer__section">
            <h4>路由轨迹</h4>
            {testResult ? (
              <RouteTraceView result={testResult} />
            ) : (
              <EmptyState
                icon={<Search size={26} />}
                title="尚未运行"
                hint="输入用户消息后点击运行知识路由，这里会展示 catalog/list_chunks/open_slice 的工具调用轨迹。"
              />
            )}
          </section>
          <section className="knowledgeDrawer__section">
            <h4>使用日志</h4>
            <UsageLogList items={usage} />
          </section>
        </div>
      </aside>
    </>
  );
}

function RouteTraceMetrics({ result }: { result: Record<string, unknown> }) {
  const candidates = (result.candidates as unknown[]) || (result.candidateChunks as unknown[]) || [];
  const selected = (result.selectedChunks as unknown[]) || (result.selected as unknown[]) || [];
  const verified = Array.isArray(selected)
    ? selected.filter((it) => (it as Record<string, unknown>)?.integrityStatus === "verified").length
    : 0;
  const tokens = Number((result.totalTokens as number) ?? (result.tokens as number) ?? 0);
  return (
    <div className="metricChipRow">
      <span><strong>{Array.isArray(candidates) ? candidates.length : 0}</strong>候选</span>
      <span><strong>{Array.isArray(selected) ? selected.length : 0}</strong>选中</span>
      <span><strong>{verified}</strong>已验证</span>
      <span><strong>{tokens}</strong>token</span>
    </div>
  );
}

function RouteTraceView({ result }: { result: Record<string, unknown> }) {
  const trace = (result.toolTrace as unknown[]) || (result.trace as unknown[]) || [];
  const selected = (result.selectedChunks as unknown[]) || (result.selected as unknown[]) || [];
  return (
    <div className="routeTraceView">
      {Array.isArray(trace) && trace.length > 0 && (
        <section>
          <div className="sectionCaption">工具调用轨迹</div>
          <ol className="eventTimeline">
            {trace.map((rawCall, index) => {
              const call = (rawCall || {}) as Record<string, unknown>;
              const tool = String(call.tool || call.name || `step-${index + 1}`);
              const summary = String(call.summary || call.outcome || "");
              const argsObj = call.arguments || call.args || {};
              const argChips = typeof argsObj === "object" && argsObj
                ? Object.entries(argsObj as Record<string, unknown>).slice(0, 3).map(([k, v]) =>
                    `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`.slice(0, 40)
                  )
                : [];
              return (
                <li key={index} className="timelineItem tone-ai">
                  <span className="timelineDot" />
                  <div className="timelineCard">
                    <div className="timelineHead">
                      <strong>{tool}</strong>
                    </div>
                    {summary && <p>{summary}</p>}
                    {argChips.length > 0 && (
                      <div className="timelineChips">
                        {argChips.map((chip, idx) => <span key={idx}>{chip}</span>)}
                      </div>
                    )}
                  </div>
                </li>
              );
            })}
          </ol>
        </section>
      )}
      {Array.isArray(selected) && selected.length > 0 && (
        <section>
          <div className="sectionCaption">选中切片</div>
          <div className="assetList">
            {selected.map((rawChunk, idx) => {
              const chunk = (rawChunk || {}) as Record<string, unknown>;
              return (
                <div key={String(chunk.id || idx)} className="assetRow">
                  <strong>{String(chunk.title || `切片 ${idx + 1}`)}</strong>
                  <span>{integrityStatusLabel(String(chunk.integrityStatus || ""))}</span>
                  <p>{String(chunk.summary || chunk.body || "").slice(0, 200)}</p>
                </div>
              );
            })}
          </div>
        </section>
      )}
      <section>
        <div className="sectionCaption">原始 JSON</div>
        <StructuredJson data={result} defaultExpanded={1} />
      </section>
    </div>
  );
}

function UsageLogList({ items }: { items: KnowledgeUsageItem[] }) {
  if (!items.length) {
    return <EmptyState icon={<ShieldCheck size={26} />} title="暂无知识使用日志" hint="一旦 Agent 在真实对话中调用知识库，调用记录与 Review 结果会按时间在这里出现。" />;
  }
  return (
    <ol className="eventTimeline">
      {items.map((item) => {
        const tone: "good" | "warn" = item.reviewApproved ? "good" : "warn";
        const headTitle = item.reviewApproved ? "Review 通过" : "Review 拦截";
        const route = (item.routeResult || {}) as Record<string, unknown>;
        const trace = (route.toolTrace as unknown[]) || [];
        const traceCount = Array.isArray(trace) ? trace.length : 0;
        const chips = [
          item.contactWxid ? `wxid · ${item.contactWxid}` : "未绑定联系人",
          `${item.knowledgeIds.length} 个知识包`,
          traceCount ? `${traceCount} 次工具调用` : ""
        ].filter(Boolean) as string[];
        const subtitle = item.replyText || item.blockedReason || "—";
        return (
          <li key={item.id} className={`timelineItem tone-${tone}`}>
            <span className="timelineDot" />
            <div className="timelineCard">
              <div className="timelineHead">
                <strong>{headTitle}</strong>
                <span>{formatTime(item.createdAt)}</span>
              </div>
              <p>{subtitle}</p>
              {chips.length > 0 && (
                <div className="timelineChips">
                  {chips.map((chip, idx) => <span key={idx}>{chip}</span>)}
                </div>
              )}
              {Object.keys(route).length > 0 && (
                <details className="timelineFold">
                  <summary>查看完整路由</summary>
                  <StructuredJson data={route} defaultExpanded={1} copyable={false} />
                </details>
              )}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

function KnowledgeEditor({
  busy,
  draft,
  editingId,
  onCreate,
  onDelete,
  onDraft,
  onNew,
  onSave,
  onAiRepair
}: {
  busy: boolean;
  draft: OperationKnowledgeDraft;
  editingId: string;
  onCreate: (event: FormEvent) => void;
  onDelete: (id: string) => void;
  onDraft: (draft: OperationKnowledgeDraft) => void;
  onNew: () => void;
  onSave: (event: FormEvent) => void;
  onAiRepair?: () => void;
}) {
  return (
    <form className="assetForm promptEditor" onSubmit={editingId ? onSave : onCreate}>
      <div className="panelHead">
        <div>
          <span>{editingId ? "Edit Knowledge" : "Create Knowledge"}</span>
          <h2>{editingId ? "编辑知识包" : "新增知识包"}</h2>
        </div>
        <div className="panelHeadActions">
          {editingId && onAiRepair && (
            <button
              type="button"
              className="aiPrimary compactButton"
              onClick={onAiRepair}
              disabled={busy}
              title="AI 先归纳整个知识包再补完元数据"
            >
              <Sparkles size={14} /> AI 自主修复
            </button>
          )}
          {editingId && (
            <button type="button" className="secondary compactButton" onClick={onNew}>
              新建
            </button>
          )}
        </div>
      </div>
      <div className="formGrid">
        <label>
          <span>知识类型</span>
          <input value={draft.knowledgeType} onChange={(event) => onDraft({ ...draft, knowledgeType: event.target.value })} placeholder="由 AI 生成，可自然语言修改" />
        </label>
        <label>
          <span>业务上下文</span>
          <input value={draft.businessContext} onChange={(event) => onDraft({ ...draft, businessContext: event.target.value })} placeholder="如：售前解释、交付边界、长期关系维护" />
        </label>
      </div>
      <label>
        <span>标题</span>
        <input value={draft.title} onChange={(event) => onDraft({ ...draft, title: event.target.value })} />
      </label>
      <label>
        <span>知识路由卡片</span>
        <textarea value={draft.routingCard} onChange={(event) => onDraft({ ...draft, routingCard: event.target.value })} />
      </label>
      <label>
        <span>摘要</span>
        <input value={draft.summary} onChange={(event) => onDraft({ ...draft, summary: event.target.value })} />
      </label>
      <label>
        <span>正文</span>
        <textarea value={draft.body} onChange={(event) => onDraft({ ...draft, body: event.target.value })} />
      </label>
      <div className="formGrid">
        <label>
          <span>适用场景</span>
          <textarea value={draft.applicableScenes} onChange={(event) => onDraft({ ...draft, applicableScenes: event.target.value })} />
        </label>
        <label>
          <span>不适用场景</span>
          <textarea value={draft.notApplicableScenes} onChange={(event) => onDraft({ ...draft, notApplicableScenes: event.target.value })} />
        </label>
      </div>
      <div className="formGrid">
        <label>
          <span>操作状态（state machine）</span>
          <textarea value={draft.operationStates} onChange={(event) => onDraft({ ...draft, operationStates: event.target.value })} placeholder="逗号分隔。如 triaged, acknowledged, mitigating" />
        </label>
        <label>
          <span>意图层级</span>
          <textarea value={draft.intentLevels} onChange={(event) => onDraft({ ...draft, intentLevels: event.target.value })} placeholder="逗号分隔。如 紧急, 常规" />
        </label>
      </div>
      <div className="formGrid">
        <label>
          <span>触发关键词</span>
          <textarea value={draft.triggerKeywords} onChange={(event) => onDraft({ ...draft, triggerKeywords: event.target.value })} placeholder="逗号分隔。AI 路由命中关键词" />
        </label>
        <label>
          <span>业务主题</span>
          <textarea value={draft.businessTopics} onChange={(event) => onDraft({ ...draft, businessTopics: event.target.value })} placeholder="逗号分隔。如 值班 SOP, 数据库切换" />
        </label>
      </div>
      <label>
        <span>产品标签</span>
        <input value={draft.productTags} onChange={(event) => onDraft({ ...draft, productTags: event.target.value })} placeholder="逗号分隔。涉及的产品/系统名" />
      </label>
      <label>
        <span>安全可说事实</span>
        <textarea value={draft.safeClaims} onChange={(event) => onDraft({ ...draft, safeClaims: event.target.value })} />
      </label>
      <label>
        <span>禁止承诺</span>
        <textarea value={draft.forbiddenClaims} onChange={(event) => onDraft({ ...draft, forbiddenClaims: event.target.value })} />
      </label>
      <div className="formGrid">
        <label>
          <span>常见问题</span>
          <textarea value={draft.commonQuestions} onChange={(event) => onDraft({ ...draft, commonQuestions: event.target.value })} />
        </label>
        <label>
          <span>常见异议</span>
          <textarea value={draft.commonObjections} onChange={(event) => onDraft({ ...draft, commonObjections: event.target.value })} />
        </label>
      </div>
      <label>
        <span>证据来源</span>
        <textarea value={draft.evidenceItems} onChange={(event) => onDraft({ ...draft, evidenceItems: event.target.value })} />
      </label>
      <div className="buttonRow">
        <button type="submit" disabled={busy || !draft.title.trim()}>
          {editingId ? "保存修改" : "保存知识"}
        </button>
        {editingId && (
          <button type="button" className="secondary" onClick={() => onDelete(editingId)} disabled={busy}>
            删除
          </button>
        )}
      </div>
    </form>
  );
}

function KnowledgeChunkEditor({
  busy,
  draft,
  editingId,
  onCreate,
  onDelete,
  onDraft,
  onLoadSource,
  onNew,
  onReject,
  onSave,
  onVerify,
  onAiRepair,
  onExtractTags,
  source
}: {
  busy: boolean;
  draft: OperationKnowledgeChunkDraft;
  editingId: string;
  onCreate: (event: FormEvent) => void;
  onDelete: (id: string) => void;
  onDraft: (draft: OperationKnowledgeChunkDraft) => void;
  onLoadSource: (id: string) => void;
  onNew: () => void;
  onReject: (id: string) => void;
  onSave: (event: FormEvent) => void;
  onVerify: (id: string) => void;
  onAiRepair?: () => void;
  onExtractTags?: () => void;
  source: Record<string, unknown> | null;
}) {
  const sourceDocument = (source?.document || {}) as Record<string, unknown>;
  const rawContent = String(sourceDocument.rawContent || "");
  return (
    <form className="assetForm promptEditor" onSubmit={editingId ? onSave : onCreate}>
      <div className="panelHead">
        <div>
          <span>{editingId ? "Edit Slice" : "Create Slice"}</span>
          <h2>{editingId ? "编辑知识切片" : "新增知识切片"}</h2>
        </div>
        <div className="panelHeadActions">
          {editingId && onAiRepair && (
            <button
              type="button"
              className="aiPrimary compactButton"
              onClick={onAiRepair}
              disabled={busy}
              title="让 AI 自主补完字段；缺资料时由 AI 主动追问"
            >
              <Sparkles size={14} /> AI 自主修复
            </button>
          )}
          {editingId && onExtractTags && (
            <button
              type="button"
              className="secondary compactButton"
              onClick={onExtractTags}
              disabled={busy}
              title="一键重抽产品标签 / 触发关键词 / 业务主题"
            >
              <Sparkles size={14} /> 一键重抽标签
            </button>
          )}
          {editingId && (
            <button type="button" className="secondary compactButton" onClick={onNew}>
              新建
            </button>
          )}
        </div>
      </div>
      <div className="formGrid">
        <label>
          <span>知识类型</span>
          <input value={draft.knowledgeType} onChange={(event) => onDraft({ ...draft, knowledgeType: event.target.value })} />
        </label>
        <label>
          <span>业务上下文</span>
          <input value={draft.businessContext} onChange={(event) => onDraft({ ...draft, businessContext: event.target.value })} />
        </label>
      </div>
      <label>
        <span>标题</span>
        <input value={draft.title} onChange={(event) => onDraft({ ...draft, title: event.target.value })} />
      </label>
      <label>
        <span>路由卡片</span>
        <textarea value={draft.routingCard} onChange={(event) => onDraft({ ...draft, routingCard: event.target.value })} />
      </label>
      <label>
        <span>摘要</span>
        <input value={draft.summary} onChange={(event) => onDraft({ ...draft, summary: event.target.value })} />
      </label>
      <label>
        <span>切片正文</span>
        <textarea className="largeTextArea" value={draft.body} onChange={(event) => onDraft({ ...draft, body: event.target.value })} />
      </label>
      <div className="formGrid">
        <label>
          <span>适用场景</span>
          <TagChipInput
            value={splitTags(draft.applicableScenes)}
            placeholder="新人首次咨询 / 价格敏感 / 已签约"
            onChange={(next) => onDraft({ ...draft, applicableScenes: next.join(", ") })}
          />
        </label>
        <label>
          <span>不适用场景</span>
          <TagChipInput
            value={splitTags(draft.notApplicableScenes)}
            placeholder="售后投诉 / 法务质询"
            onChange={(next) => onDraft({ ...draft, notApplicableScenes: next.join(", ") })}
          />
        </label>
      </div>
      <div className="formGrid">
        <label>
          <span>安全事实</span>
          <TagChipInput
            value={splitTags(draft.safeClaims)}
            placeholder="提供 7 天试用 / 技术对接 1v1"
            onChange={(next) => onDraft({ ...draft, safeClaims: next.join(", ") })}
          />
        </label>
        <label>
          <span>禁止承诺</span>
          <TagChipInput
            value={splitTags(draft.forbiddenClaims)}
            placeholder="承诺销量翻倍 / 永久免费"
            onChange={(next) => onDraft({ ...draft, forbiddenClaims: next.join(", ") })}
          />
        </label>
      </div>
      <label>
        <span>证据</span>
        <TagChipInput
          value={splitTags(draft.evidenceItems)}
          placeholder="案例 A · 销量提升 30% / 内部测试报告 2025-Q1"
          onChange={(next) => onDraft({ ...draft, evidenceItems: next.join(", ") })}
        />
      </label>
      <div className="formGrid">
        <label>
          <span>产品标签 (productTags, 最多 5)</span>
          <TagChipInput
            value={splitTags(draft.productTags)}
            max={5}
            placeholder="WechatAgent / AI 私域销售助手"
            onChange={(next) => onDraft({ ...draft, productTags: next.join(", ") })}
          />
        </label>
        <label>
          <span>触发关键词 (triggerKeywords, 最多 8, 含口语化变体)</span>
          <TagChipInput
            value={splitTags(draft.triggerKeywords)}
            max={8}
            placeholder="群发工具区别 / 价格"
            onChange={(next) => onDraft({ ...draft, triggerKeywords: next.join(", ") })}
          />
        </label>
      </div>
      <label>
        <span>业务主题 (businessTopics, 最多 3)</span>
        <TagChipInput
          value={splitTags(draft.businessTopics)}
          max={3}
          placeholder="产品定位差异 / 竞品对比"
          onChange={(next) => onDraft({ ...draft, businessTopics: next.join(", ") })}
        />
      </label>
      <label>
        <span>原文引用</span>
        <textarea value={draft.sourceQuote} onChange={(event) => onDraft({ ...draft, sourceQuote: event.target.value })} />
      </label>
      <div className="formGrid">
        <label>
          <span>完整性状态</span>
          <select
            value={draft.integrityStatus || "needs_review"}
            onChange={(event) => onDraft({ ...draft, integrityStatus: event.target.value })}
          >
            <option value="needs_review">待 AI 复核 (needs_review)</option>
            <option value="verified">已运营确认 (verified)</option>
            <option value="rejected">驳回 (rejected)</option>
          </select>
        </label>
        <label>
          <span>置信分（0-100）</span>
          <input
            type="number"
            min={0}
            max={100}
            step={1}
            value={draft.confidenceScore}
            onChange={(event) => onDraft({ ...draft, confidenceScore: event.target.value })}
          />
        </label>
      </div>
      <div className="formGrid">
        <label>
          <span>已验证事实</span>
          <TagChipInput
            value={splitLines(draft.verifiedClaims)}
            placeholder="已被运营确认的事实条目"
            onChange={(next) => onDraft({ ...draft, verifiedClaims: next.join("\n") })}
          />
        </label>
        <label>
          <span>无依据声明</span>
          <TagChipInput
            value={splitLines(draft.unsupportedClaims)}
            placeholder="缺少证据的描述"
            onChange={(next) => onDraft({ ...draft, unsupportedClaims: next.join("\n") })}
          />
        </label>
      </div>
      <label>
        <span>失真风险</span>
        <TagChipInput
          value={splitLines(draft.distortionRisks)}
          placeholder="可能被夸大或误解的点"
          onChange={(next) => onDraft({ ...draft, distortionRisks: next.join("\n") })}
        />
      </label>
      {editingId && (
        <section className="sourceCompare">
          <div className="buttonRow">
            <button type="button" className="secondary" onClick={() => onLoadSource(editingId)} disabled={busy}>
              原文对照
            </button>
            <button type="button" className="secondary" onClick={() => onVerify(editingId)} disabled={busy}>
              运营确认
            </button>
            <button type="button" className="secondary" onClick={() => onReject(editingId)} disabled={busy}>
              驳回切片
            </button>
          </div>
          {rawContent && (
            <div className="sourceQuoteBox">
              <strong>{String(sourceDocument.title || "原文")}</strong>
              <p>{rawContent}</p>
            </div>
          )}
        </section>
      )}
      <div className="buttonRow">
        <button type="submit" disabled={busy || !draft.title.trim()}>
          {editingId ? "保存切片" : "保存切片"}
        </button>
        {editingId && (
          <button type="button" className="secondary" onClick={() => onDelete(editingId)} disabled={busy}>
            删除
          </button>
        )}
      </div>
    </form>
  );
}

function DomainConfigEditor({
  busy,
  config,
  draft,
  mode: _mode,
  onDraft,
  onReset,
  onSave
}: {
  busy: boolean;
  config?: OperationDomainConfig;
  draft: OperationDomainDraft;
  mode?: "primary" | "standard";
  onDraft: (draft: OperationDomainDraft) => void;
  onReset: () => void;
  onSave: () => void;
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
    retryBaseMs: ""
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
    retryBaseMs: item.retryBaseMs == null ? "" : String(item.retryBaseMs)
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
      model: d.model.trim()
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

function KnowledgeChipBlock({
  label,
  values,
  muted = false,
  block = false,
}: {
  label: string;
  values?: string[];
  muted?: boolean;
  block?: boolean;
}) {
  if (!values || values.length === 0) return null;
  if (block) {
    return (
      <div className="knowledgeDrawer__section">
        <h4>{label}</h4>
        <div className={`knowledgeChipRow${muted ? " muted" : ""}`}>
          {values.map((v, i) => (
            <span key={i}>{v}</span>
          ))}
        </div>
      </div>
    );
  }
  return (
    <div className={`knowledgeChipInline${muted ? " muted" : ""}`}>
      <em>{label}</em>
      {values.map((v, i) => (
        <span key={i}>{v}</span>
      ))}
    </div>
  );
}

function KnowledgeMetaItem({
  label,
  value,
}: {
  label: string;
  value?: string | number | null;
}) {
  if (value === undefined || value === null || value === "") return null;
  return (
    <div className="knowledgeMetaItem">
      <em>{label}</em>
      <span>{String(value)}</span>
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
    case "knowledge":
      return "运营知识库";
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
    case "knowledge":
      return "Knowledge Router";
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
    case "knowledge":
      return "导入运营资料，拆分知识包，并让用户运营 Agent 按场景渐进加载。";
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

function emptyKnowledgeDraft(): OperationKnowledgeDraft {
  return {
    category: "",
    businessType: "",
    knowledgeType: "",
    businessContext: "",
    title: "",
    summary: "",
    body: "",
    routingCard: "",
    applicableScenes: "",
    notApplicableScenes: "",
    suitableFor: "",
    notSuitableFor: "",
    customerStages: "",
    operationStates: "",
    intentLevels: "",
    commonQuestions: "",
    commonObjections: "",
    safeClaims: "",
    forbiddenClaims: "",
    evidenceItems: "",
    productTags: "",
    triggerKeywords: "",
    businessTopics: "",
    sourceName: "",
    status: "active",
    priority: "0"
  };
}

function emptyChunkDraft(): OperationKnowledgeChunkDraft {
  return {
    documentId: "",
    itemId: "",
    knowledgeType: "",
    businessContext: "",
    title: "",
    summary: "",
    body: "",
    routingCard: "",
    applicableScenes: "",
    notApplicableScenes: "",
    safeClaims: "",
    forbiddenClaims: "",
    evidenceItems: "",
    sourceQuote: "",
    integrityStatus: "needs_review",
    confidenceScore: "0",
    distortionRisks: "",
    unsupportedClaims: "",
    verifiedClaims: "",
    status: "active",
    priority: "0",
    productTags: "",
    triggerKeywords: "",
    businessTopics: ""
  };
}

function draftFromKnowledge(item: OperationKnowledgeItem): OperationKnowledgeDraft {
  return {
    category: item.category,
    businessType: item.businessType,
    knowledgeType: item.knowledgeType ?? "",
    businessContext: item.businessContext ?? "",
    title: item.title,
    summary: item.summary ?? "",
    body: item.body ?? "",
    routingCard: item.routingCard ?? "",
    applicableScenes: (item.applicableScenes || []).join(", "),
    notApplicableScenes: (item.notApplicableScenes || []).join(", "),
    suitableFor: (item.suitableFor || []).join(", "),
    notSuitableFor: (item.notSuitableFor || []).join(", "),
    customerStages: (item.customerStages || []).join(", "),
    operationStates: (item.operationStates || []).join(", "),
    intentLevels: (item.intentLevels || []).join(", "),
    commonQuestions: (item.commonQuestions || []).join("\n"),
    commonObjections: (item.commonObjections || []).join("\n"),
    safeClaims: (item.safeClaims || []).join("\n"),
    forbiddenClaims: (item.forbiddenClaims || []).join("\n"),
    evidenceItems: (item.evidenceItems || []).join("\n"),
    productTags: (item.productTags || []).join(", "),
    triggerKeywords: (item.triggerKeywords || []).join(", "),
    businessTopics: (item.businessTopics || []).join(", "),
    sourceName: item.sourceName ?? "",
    status: item.status || "active",
    priority: String(item.priority ?? 0)
  };
}

function draftFromChunk(item: OperationKnowledgeChunk): OperationKnowledgeChunkDraft {
  return {
    documentId: item.documentId ?? "",
    itemId: item.itemId ?? "",
    knowledgeType: item.knowledgeType ?? "",
    businessContext: item.businessContext ?? "",
    title: item.title,
    summary: item.summary ?? "",
    body: item.body ?? "",
    routingCard: item.routingCard ?? "",
    applicableScenes: (item.applicableScenes || []).join(", "),
    notApplicableScenes: (item.notApplicableScenes || []).join(", "),
    safeClaims: (item.safeClaims || []).join("\n"),
    forbiddenClaims: (item.forbiddenClaims || []).join("\n"),
    evidenceItems: (item.evidenceItems || []).join("\n"),
    sourceQuote: item.sourceQuote ?? "",
    integrityStatus: item.integrityStatus ?? "needs_review",
    confidenceScore: String(item.confidenceScore ?? 0),
    distortionRisks: (item.distortionRisks || []).join("\n"),
    unsupportedClaims: (item.unsupportedClaims || []).join("\n"),
    verifiedClaims: (item.verifiedClaims || []).join("\n"),
    status: item.status || "active",
    priority: String(item.priority ?? 0),
    productTags: (item.productTags || []).join(", "),
    triggerKeywords: (item.triggerKeywords || []).join(", "),
    businessTopics: (item.businessTopics || []).join(", ")
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

function knowledgePayload(draft: OperationKnowledgeDraft) {
  return {
    domain: "user_operations",
    category: draft.category || draft.knowledgeType,
    businessType: draft.businessType || draft.businessContext,
    knowledgeType: draft.knowledgeType || undefined,
    businessContext: draft.businessContext || undefined,
    title: draft.title,
    summary: draft.summary || undefined,
    body: draft.body || undefined,
    routingCard: draft.routingCard || undefined,
    applicableScenes: splitTags(draft.applicableScenes),
    notApplicableScenes: splitTags(draft.notApplicableScenes),
    suitableFor: splitTags(draft.suitableFor),
    notSuitableFor: splitTags(draft.notSuitableFor),
    customerStages: splitTags(draft.customerStages),
    operationStates: splitTags(draft.operationStates),
    intentLevels: splitTags(draft.intentLevels),
    commonQuestions: splitLines(draft.commonQuestions),
    commonObjections: splitLines(draft.commonObjections),
    safeClaims: splitLines(draft.safeClaims),
    forbiddenClaims: splitLines(draft.forbiddenClaims),
    evidenceItems: splitLines(draft.evidenceItems),
    productTags: splitTags(draft.productTags),
    triggerKeywords: splitTags(draft.triggerKeywords),
    businessTopics: splitTags(draft.businessTopics),
    sourceType: "manual",
    sourceName: draft.sourceName || undefined,
    status: draft.status || "active",
    priority: Number(draft.priority || 0)
  };
}

function knowledgeItemPayload(item: OperationKnowledgeItem) {
  return {
    domain: "user_operations",
    category: item.category || item.knowledgeType,
    businessType: item.businessType || item.businessContext,
    knowledgeType: item.knowledgeType || undefined,
    businessContext: item.businessContext || undefined,
    title: item.title,
    summary: item.summary || undefined,
    body: item.body || undefined,
    routingCard: item.routingCard || undefined,
    applicableScenes: item.applicableScenes || [],
    notApplicableScenes: item.notApplicableScenes || [],
    suitableFor: item.suitableFor,
    notSuitableFor: item.notSuitableFor,
    customerStages: item.customerStages,
    operationStates: item.operationStates,
    intentLevels: item.intentLevels,
    commonQuestions: item.commonQuestions,
    commonObjections: item.commonObjections,
    safeClaims: item.safeClaims,
    forbiddenClaims: item.forbiddenClaims,
    evidenceItems: item.evidenceItems,
    sourceType: item.sourceType || "imported_markdown",
    sourceName: item.sourceName || undefined,
    status: item.status || "active",
    priority: item.priority || 0
  };
}

function knowledgeDocumentPayload(item: OperationKnowledgeDocument) {
  return {
    domain: "user_operations",
    sourceType: item.sourceType || "imported_markdown",
    sourceName: item.sourceName || undefined,
    title: item.title,
    summary: item.summary || undefined,
    catalogSummary: item.catalogSummary || undefined,
    routingMap: item.routingMap || [],
    riskNotes: item.riskNotes || [],
    rawContent: item.rawContent || undefined,
    contentHash: item.contentHash || undefined,
    lineIndex: item.lineIndex || [],
    sectionIndex: item.sectionIndex || [],
    status: item.status || "active"
  };
}

function knowledgeChunkPayload(item: OperationKnowledgeChunk) {
  return {
    documentId: item.documentId || undefined,
    itemId: item.itemId || undefined,
    domain: "user_operations",
    knowledgeType: item.knowledgeType || undefined,
    businessContext: item.businessContext || undefined,
    title: item.title,
    summary: item.summary || undefined,
    body: item.body || undefined,
    routingCard: item.routingCard || undefined,
    applicableScenes: item.applicableScenes || [],
    notApplicableScenes: item.notApplicableScenes || [],
    safeClaims: item.safeClaims || [],
    forbiddenClaims: item.forbiddenClaims || [],
    evidenceItems: item.evidenceItems || [],
    sourceQuote: item.sourceQuote || undefined,
    sourceAnchors: item.sourceAnchors || [],
    integrityStatus: item.integrityStatus || undefined,
    confidenceScore: item.confidenceScore ?? undefined,
    distortionRisks: item.distortionRisks || [],
    unsupportedClaims: item.unsupportedClaims || [],
    verifiedClaims: item.verifiedClaims || [],
    status: item.status || "active",
    priority: item.priority || 0
  };
}

function chunkPayload(draft: OperationKnowledgeChunkDraft) {
  return {
    documentId: draft.documentId || undefined,
    itemId: draft.itemId || undefined,
    domain: "user_operations",
    knowledgeType: draft.knowledgeType || undefined,
    businessContext: draft.businessContext || undefined,
    title: draft.title,
    summary: draft.summary || undefined,
    body: draft.body || undefined,
    routingCard: draft.routingCard || undefined,
    applicableScenes: splitTags(draft.applicableScenes),
    notApplicableScenes: splitTags(draft.notApplicableScenes),
    safeClaims: splitLines(draft.safeClaims),
    forbiddenClaims: splitLines(draft.forbiddenClaims),
    evidenceItems: splitLines(draft.evidenceItems),
    sourceQuote: draft.sourceQuote || undefined,
    sourceAnchors: [],
    integrityStatus: draft.integrityStatus || undefined,
    confidenceScore: Number(draft.confidenceScore || 0),
    distortionRisks: splitLines(draft.distortionRisks),
    unsupportedClaims: splitLines(draft.unsupportedClaims),
    verifiedClaims: splitLines(draft.verifiedClaims),
    status: draft.status || "active",
    priority: Number(draft.priority || 0),
    productTags: splitTags(draft.productTags),
    triggerKeywords: splitTags(draft.triggerKeywords),
    businessTopics: splitTags(draft.businessTopics)
  };
}

const AI_REPAIR_CHUNK_FIELDS = [
  "knowledgeType",
  "businessContext",
  "title",
  "summary",
  "body",
  "routingCard",
  "applicableScenes",
  "notApplicableScenes",
  "safeClaims",
  "forbiddenClaims",
  "evidenceItems",
  "sourceQuote",
  "productTags",
  "triggerKeywords",
  "businessTopics"
] as const;

const AI_REPAIR_PACK_FIELDS = [
  "knowledgeType",
  "businessContext",
  "title",
  "summary",
  "body",
  "routingCard",
  "applicableScenes",
  "notApplicableScenes",
  "suitableFor",
  "notSuitableFor",
  "customerStages",
  "operationStates",
  "intentLevels",
  "safeClaims",
  "forbiddenClaims",
  "commonQuestions",
  "commonObjections",
  "evidenceItems",
  "productTags",
  "triggerKeywords",
  "businessTopics"
] as const;

function mergeChunkPatch(
  chunk: OperationKnowledgeChunk,
  patch: Record<string, unknown>
): Record<string, unknown> {
  const result: Record<string, unknown> = {
    documentId: chunk.documentId,
    itemId: chunk.itemId,
    domain: "user_operations",
    knowledgeType: chunk.knowledgeType,
    businessContext: chunk.businessContext,
    title: chunk.title,
    summary: chunk.summary,
    body: chunk.body,
    routingCard: chunk.routingCard,
    applicableScenes: chunk.applicableScenes ?? [],
    notApplicableScenes: chunk.notApplicableScenes ?? [],
    safeClaims: chunk.safeClaims ?? [],
    forbiddenClaims: chunk.forbiddenClaims ?? [],
    evidenceItems: chunk.evidenceItems ?? [],
    sourceQuote: chunk.sourceQuote,
    sourceAnchors: chunk.sourceAnchors ?? [],
    integrityStatus: chunk.integrityStatus,
    confidenceScore: chunk.confidenceScore ?? 0,
    distortionRisks: chunk.distortionRisks ?? [],
    unsupportedClaims: chunk.unsupportedClaims ?? [],
    verifiedClaims: chunk.verifiedClaims ?? [],
    status: chunk.status || "active",
    priority: chunk.priority ?? 0,
    productTags: chunk.productTags ?? [],
    triggerKeywords: chunk.triggerKeywords ?? [],
    businessTopics: chunk.businessTopics ?? []
  };
  for (const key of AI_REPAIR_CHUNK_FIELDS) {
    if (Object.prototype.hasOwnProperty.call(patch, key)) {
      const value = patch[key];
      if (value !== undefined && value !== null) {
        result[key] = value;
      }
    }
  }
  return result;
}

function mergePackPatch(
  pack: OperationKnowledgeItem,
  patch: Record<string, unknown>
): Record<string, unknown> {
  const result: Record<string, unknown> = {
    domain: "user_operations",
    category: pack.category,
    businessType: pack.businessType,
    knowledgeType: pack.knowledgeType,
    businessContext: pack.businessContext,
    title: pack.title,
    summary: pack.summary,
    body: pack.body,
    routingCard: pack.routingCard,
    applicableScenes: pack.applicableScenes ?? [],
    notApplicableScenes: pack.notApplicableScenes ?? [],
    suitableFor: pack.suitableFor ?? [],
    notSuitableFor: pack.notSuitableFor ?? [],
    customerStages: pack.customerStages ?? [],
    operationStates: pack.operationStates ?? [],
    intentLevels: pack.intentLevels ?? [],
    safeClaims: pack.safeClaims ?? [],
    forbiddenClaims: pack.forbiddenClaims ?? [],
    commonQuestions: pack.commonQuestions ?? [],
    commonObjections: pack.commonObjections ?? [],
    evidenceItems: pack.evidenceItems ?? [],
    productTags: pack.productTags ?? [],
    triggerKeywords: pack.triggerKeywords ?? [],
    businessTopics: pack.businessTopics ?? [],
    status: pack.status || "active",
    priority: pack.priority ?? 0
  };
  for (const key of AI_REPAIR_PACK_FIELDS) {
    if (Object.prototype.hasOwnProperty.call(patch, key)) {
      const value = patch[key];
      if (value !== undefined && value !== null) {
        result[key] = value;
      }
    }
  }
  return result;
}

function integrityStatusLabel(status?: string) {
  if (status === "verified") return "已验证";
  if (status === "rejected") return "已拒绝";
  return "需复核";
}

function knowledgeAnsweringModeLabel(mode: string) {
  if (mode === "fully_supported") return "可完整回答事实问题";
  if (mode === "product_safe") return "可有限回答产品事实";
  return "仅关系维护模式";
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

type TreeNodeKind =
  | "document"
  | "pack"
  | "chunk"
  | "group-orphan-packs"
  | "group-orphan-chunks";

type TreeNodeBadge = { tone: "good" | "warn" | "error" | "neutral"; text: string };

type TreeNode = {
  id: string;
  kind: TreeNodeKind;
  refId?: string;
  label: string;
  meta?: string;
  badges?: TreeNodeBadge[];
  children?: TreeNode[];
};

function chunkBadge(chunk: OperationKnowledgeChunk): TreeNodeBadge | null {
  const status = (chunk.integrityStatus || "").toLowerCase();
  if (!status) return null;
  if (status === "verified") return { tone: "good", text: "已验证" };
  if (status === "rejected") return { tone: "error", text: "已拒绝" };
  if (status === "needs_review" || status === "needsreview") return { tone: "warn", text: "AI 需复核" };
  return { tone: "neutral", text: status };
}

function buildKnowledgeTree(
  documents: OperationKnowledgeDocument[],
  items: OperationKnowledgeItem[],
  chunks: OperationKnowledgeChunk[]
): TreeNode[] {
  const roots: TreeNode[] = [];

  const chunksByItem = new Map<string, OperationKnowledgeChunk[]>();
  const chunksByDoc = new Map<string, OperationKnowledgeChunk[]>();
  const chunksOrphan: OperationKnowledgeChunk[] = [];
  for (const c of chunks) {
    if (c.itemId) {
      const arr = chunksByItem.get(c.itemId) || [];
      arr.push(c);
      chunksByItem.set(c.itemId, arr);
    } else if (c.documentId) {
      const arr = chunksByDoc.get(c.documentId) || [];
      arr.push(c);
      chunksByDoc.set(c.documentId, arr);
    } else {
      chunksOrphan.push(c);
    }
  }

  const docIdByItem = new Map<string, string>();
  for (const item of items) {
    const owning = chunks.find((c) => c.itemId === item.id && c.documentId);
    if (owning?.documentId) docIdByItem.set(item.id, owning.documentId);
  }

  const itemsByDoc = new Map<string, OperationKnowledgeItem[]>();
  const orphanItems: OperationKnowledgeItem[] = [];
  for (const item of items) {
    const docId = docIdByItem.get(item.id);
    if (docId) {
      const arr = itemsByDoc.get(docId) || [];
      arr.push(item);
      itemsByDoc.set(docId, arr);
    } else {
      const matchedDoc = documents.find(
        (d) => item.sourceName && (d.title === item.sourceName || d.sourceName === item.sourceName)
      );
      if (matchedDoc) {
        const arr = itemsByDoc.get(matchedDoc.id) || [];
        arr.push(item);
        itemsByDoc.set(matchedDoc.id, arr);
      } else {
        orphanItems.push(item);
      }
    }
  }

  for (const doc of documents) {
    const docPacks = itemsByDoc.get(doc.id) || [];
    const looseChunks = chunksByDoc.get(doc.id) || [];
    const packNodes: TreeNode[] = docPacks.map((pack) => {
      const packChunks = chunksByItem.get(pack.id) || [];
      return {
        id: `pack:${pack.id}`,
        kind: "pack",
        refId: pack.id,
        label: pack.title,
        meta: packChunks.length ? `${packChunks.length} 切片` : "0 切片",
        children: packChunks.map((c) => ({
          id: `chunk:${c.id}`,
          kind: "chunk",
          refId: c.id,
          label: c.title,
          badges: [chunkBadge(c)].filter(Boolean) as TreeNodeBadge[]
        }))
      };
    });
    const looseChunkNodes: TreeNode[] = looseChunks.map((c) => ({
      id: `chunk:${c.id}`,
      kind: "chunk",
      refId: c.id,
      label: c.title,
      badges: [chunkBadge(c)].filter(Boolean) as TreeNodeBadge[]
    }));
    roots.push({
      id: `doc:${doc.id}`,
      kind: "document",
      refId: doc.id,
      label: doc.title || doc.sourceName || "未命名文档",
      meta: `${packNodes.length} 包 · ${packNodes.reduce((n, p) => n + (p.children?.length || 0), 0) + looseChunkNodes.length} 切片`,
      children: [...packNodes, ...looseChunkNodes]
    });
  }

  if (orphanItems.length) {
    roots.push({
      id: "group:orphan-packs",
      kind: "group-orphan-packs",
      label: "未关联文档的知识包",
      meta: `${orphanItems.length} 个`,
      children: orphanItems.map((pack) => {
        const packChunks = chunksByItem.get(pack.id) || [];
        return {
          id: `pack:${pack.id}`,
          kind: "pack",
          refId: pack.id,
          label: pack.title,
          meta: packChunks.length ? `${packChunks.length} 切片` : "0 切片",
          children: packChunks.map((c) => ({
            id: `chunk:${c.id}`,
            kind: "chunk",
            refId: c.id,
            label: c.title,
            badges: [chunkBadge(c)].filter(Boolean) as TreeNodeBadge[]
          }))
        };
      })
    });
  }

  if (chunksOrphan.length) {
    roots.push({
      id: "group:orphan-chunks",
      kind: "group-orphan-chunks",
      label: "未关联包的切片",
      meta: `${chunksOrphan.length} 个`,
      children: chunksOrphan.map((c) => ({
        id: `chunk:${c.id}`,
        kind: "chunk",
        refId: c.id,
        label: c.title,
        badges: [chunkBadge(c)].filter(Boolean) as TreeNodeBadge[]
      }))
    });
  }

  return roots;
}

function findTreeNode(roots: TreeNode[], id: string): TreeNode | null {
  for (const root of roots) {
    if (root.id === id) return root;
    if (root.children) {
      const found = findTreeNode(root.children, id);
      if (found) return found;
    }
  }
  return null;
}

function findAncestors(roots: TreeNode[], id: string, trail: string[] = []): string[] | null {
  for (const root of roots) {
    if (root.id === id) return trail;
    if (root.children) {
      const next = findAncestors(root.children, id, [...trail, root.id]);
      if (next) return next;
    }
  }
  return null;
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
  const keys = ["humanLike", "emotionalValue", "productAccuracy", "pressureRisk", "factRisk"];
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
    pressureRisk: "销售压迫感风险",
    factRisk: "事实风险"
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
    customerStage: "客户阶段",
    intentLevel: "意向等级",
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

// knowledge-wiki Phase G：Wiki 管理频道——3 个最小可用 admin 视图。
//
// - DomainSchemaTab：列 active / 历史版本，一键切换 active；fields 只读展示
//   （新建 / 删除暂走后端 API + curl，UI 不做表单避免和 chunks 主表互锁）
// - GapSignalsTab：列 pending 信号 + 一键 sweep + dismiss / apply
// - ChunkRevisionsDrawer：输入 chunk_id 拉历史 timeline（drawer 形态）
type KnowledgeWikiTab = "domainSchemas" | "gapSignals" | "revisions";

function KnowledgeWikiView() {
  const [tab, setTab] = useState<KnowledgeWikiTab>("domainSchemas");
  return (
    <section className="qualityCenter knowledgeWiki">
      <div className="panelHead compact">
        <div>
          <span>Knowledge Wiki Admin</span>
          <h2>Wiki 管理</h2>
        </div>
        <FileBox size={18} />
      </div>
      <div className="qualityTabs">
        <button
          className={tab === "domainSchemas" ? "tab active" : "tab"}
          onClick={() => setTab("domainSchemas")}
        >
          行业 Schema
        </button>
        <button
          className={tab === "gapSignals" ? "tab active" : "tab"}
          onClick={() => setTab("gapSignals")}
        >
          质量信号
        </button>
        <button
          className={tab === "revisions" ? "tab active" : "tab"}
          onClick={() => setTab("revisions")}
        >
          编辑历史
        </button>
      </div>
      {tab === "domainSchemas" && <DomainSchemaTab />}
      {tab === "gapSignals" && <GapSignalsTab />}
      {tab === "revisions" && <ChunkRevisionsDrawer />}
    </section>
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
      if (!r.ok) throw new Error(await r.text());
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
      if (!r.ok) throw new Error(await r.text());
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

function GapSignalsTab() {
  const [items, setItems] = useState<GapSignalItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [sweeping, setSweeping] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [kindFilter, setKindFilter] = useState<string>("");

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams({ status: "pending", limit: "200" });
      if (kindFilter) params.set("kind", kindFilter);
      const r = await fetch(`/api/knowledge/gap-signals?${params.toString()}`);
      if (!r.ok) throw new Error(await r.text());
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kindFilter]);

  async function sweep() {
    setSweeping(true);
    setError(null);
    setInfo(null);
    try {
      const r = await fetch("/api/knowledge/gap-signals/sweep", { method: "POST" });
      if (!r.ok) throw new Error(await r.text());
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
      const r = await fetch(`/api/knowledge/gap-signals/${encodeURIComponent(signalId)}/${action}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      if (!r.ok) throw new Error(await r.text());
      setInfo(action === "dismiss" ? "已忽略" : "已标记为已应用");
      await load();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  const KIND_OPTIONS = [
    { v: "", label: "全部 kind" },
    { v: "orphan", label: "orphan" },
    { v: "broken_link", label: "broken_link" },
    { v: "no_outlinks", label: "no_outlinks" },
    { v: "low_confidence", label: "low_confidence" },
    { v: "stale", label: "stale" },
  ];

  return (
    <div className="wikiPanelBody">
      <div className="wikiToolbar">
        <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} />
          {loading ? "加载中…" : "刷新"}
        </button>
        <button type="button" className="primary" onClick={() => void sweep()} disabled={sweeping}>
          {sweeping ? "扫描中…" : "立即扫描"}
        </button>
        <select
          className="wikiSelect"
          value={kindFilter}
          onChange={(e) => setKindFilter(e.target.value)}
        >
          {KIND_OPTIONS.map((o) => (
            <option key={o.v} value={o.v}>
              {o.label}
            </option>
          ))}
        </select>
        <span className="wikiHint">仅展示 status=pending；扫描包含结构 lint + stage1 规则消解。</span>
      </div>
      {error ? <div className="wikiAlert error">{error}</div> : null}
      {info ? <div className="wikiAlert info">{info}</div> : null}
      {!loading && items.length === 0 ? (
        <div className="wikiEmpty">没有 pending 信号。库结构当前看起来很健康。</div>
      ) : null}
      <div className="wikiSignalList">
        {items.map((s) => (
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
                  <code key={id}>{id}</code>
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
      if (!r.ok) throw new Error(await r.text());
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
 * M3 / Task 79：Cockpit 内 Planner 视角。展示 customer_stage 最近变更时间与
 * commitments 列表，给运营快速理解 AI 主动跟进的信号面。只读卡片，符合
 * "全自治、无中转"产品定位。
 */
export function PlannerViewSection({ contact }: { contact: Contact | null }) {
  if (!contact) {
    return null;
  }
  const stageUpdatedAt = contact.customerStageUpdatedAt;
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
          客户阶段 <strong>{contact.customerStage || "未分层"}</strong>
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

type AiRepairTarget = { kind: "chunk" | "pack"; id: string; label: string };

/// 前端在调用现有 PUT 落库 + 可选 verify 之后，再 POST `/repair/applied`
/// 写入审计事件时携带的元数据。所有字段都是只读快照，纯审计用，不影响业务字段。
type AiRepairApplyAuditMeta = {
  sessionId?: string;
  turn?: number;
  confidenceHint?: number;
  acceptedFields: string[];
  skippedFields: string[];
};

type AiRepairProposal = {
  chunkId?: string;
  packId?: string;
  sessionId?: string;
  turn?: number;
  promptKey?: string;
  interpretation?: Record<string, unknown> | null;
  patch?: Record<string, unknown> | null;
  missingFields?: Array<{ field: string; reason?: string | null } | string>;
  followupQuestions?: Array<{ id: string; field?: string; question: string }>;
  stillMissing?: Array<{ field: string; reason?: string | null } | string>;
  confidenceHint?: number;
  isFinalTurn?: boolean;
};

const AI_REPAIR_FIELD_LABELS: Record<string, string> = {
  routingCard: "路由卡片",
  summary: "摘要",
  body: "正文",
  knowledgeType: "知识类型",
  businessContext: "业务上下文",
  applicableScenes: "适用场景",
  notApplicableScenes: "不适用场景",
  safeClaims: "安全事实",
  forbiddenClaims: "禁止承诺",
  evidenceItems: "证据",
  sourceQuote: "源文锚定",
  customerStages: "客户阶段（按领域重解读）",
  intentLevels: "意图等级（按领域重解读）",
  commonQuestions: "常见问题（按领域重解读）",
  commonObjections: "常见异议（按领域重解读）",
  suitableFor: "适用对象",
  notSuitableFor: "不适用对象",
  productTags: "产品标签",
  triggerKeywords: "触发关键词",
  businessTopics: "业务主题",
  extras: "领域专属字段"
};

function aiRepairFieldLabel(key: string): string {
  return AI_REPAIR_FIELD_LABELS[key] ?? key;
}

const AI_INTERPRETATION_LABELS: Record<string, string> = {
  domain: "领域",
  audience: "读者",
  purpose: "用途",
  openConditions: "何时打开",
  catalogContext: "目录脉络",
  riskNotes: "风险提示",
  businessTopics: "业务主题",
  triggerKeywords: "触发关键词",
  notes: "其他说明"
};

function aiInterpretationLabel(key: string): string {
  return AI_INTERPRETATION_LABELS[key] ?? key;
}

function aiRepairFormatValue(value: unknown): string {
  if (value === null || value === undefined) return "（空）";
  if (Array.isArray(value)) {
    if (value.length === 0) return "（空）";
    return value
      .map((v) => (typeof v === "string" ? v : JSON.stringify(v)))
      .join(" / ");
  }
  if (typeof value === "object") {
    return JSON.stringify(value, null, 2);
  }
  const s = String(value);
  return s.trim() === "" ? "（空）" : s;
}

function aiRepairCurrentValue(
  target: AiRepairTarget,
  field: string,
  chunks: OperationKnowledgeChunk[],
  packs: OperationKnowledgeItem[]
): unknown {
  if (target.kind === "chunk") {
    const c = chunks.find((x) => x.id === target.id);
    if (!c) return undefined;
    return (c as unknown as Record<string, unknown>)[field];
  }
  const p = packs.find((x) => x.id === target.id);
  if (!p) return undefined;
  return (p as unknown as Record<string, unknown>)[field];
}

function AiRepairPanel(props: {
  target: AiRepairTarget;
  chunks: OperationKnowledgeChunk[];
  packs: OperationKnowledgeItem[];
  onClose: () => void;
  proposeChunk: (chunkId: string) => Promise<Record<string, unknown>>;
  answerChunk: (
    chunkId: string,
    body: {
      sessionId?: string;
      previousPatch?: Record<string, unknown> | null;
      answers: Array<{ id: string; field?: string; text: string }>;
      turn: number;
    }
  ) => Promise<Record<string, unknown>>;
  proposePack: (packId: string) => Promise<Record<string, unknown>>;
  onApply: (
    target: AiRepairTarget,
    patch: Record<string, unknown>,
    options: { thenVerify?: boolean },
    auditMeta: AiRepairApplyAuditMeta
  ) => Promise<void>;
}) {
  const { target, onClose } = props;
  const [phase, setPhase] = useState<
    "proposing" | "reviewing" | "answering" | "applying" | "error"
  >("proposing");
  const [proposal, setProposal] = useState<AiRepairProposal | null>(null);
  const [skippedFields, setSkippedFields] = useState<Set<string>>(new Set());
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [errorObj, setErrorObj] = useState<Error | null>(null);
  // proposeNonce 让「AI 重试」按钮通过 +1 来重新触发首轮 propose effect。
  // 不直接调 props.proposeChunk 是为了保留 effect 内的 cancelled 守卫。
  const [proposeNonce, setProposeNonce] = useState(0);
  const errorMsg = errorObj?.message ?? null;

  useEffect(() => {
    let cancelled = false;
    setPhase("proposing");
    setProposal(null);
    setSkippedFields(new Set());
    setAnswers({});
    setErrorObj(null);
    const propose =
      target.kind === "chunk"
        ? props.proposeChunk(target.id)
        : props.proposePack(target.id);
    propose
      .then((data) => {
        if (cancelled) return;
        setProposal(data as AiRepairProposal);
        setPhase("reviewing");
      })
      .catch((err: Error) => {
        if (cancelled) return;
        setErrorObj(err instanceof Error ? err : new Error(String(err)));
        setPhase("error");
      });
    return () => {
      cancelled = true;
    };
  }, [target, proposeNonce]); // eslint-disable-line react-hooks/exhaustive-deps

  function retryPropose() {
    setProposeNonce((n) => n + 1);
  }

  const patch = (proposal?.patch ?? {}) as Record<string, unknown>;
  const patchEntries = Object.entries(patch);
  const followup = proposal?.followupQuestions ?? [];
  const missing = proposal?.missingFields ?? [];
  const stillMissing = proposal?.stillMissing ?? [];
  const interpretation = (proposal?.interpretation ?? {}) as Record<string, unknown>;

  function toggleSkip(field: string) {
    const next = new Set(skippedFields);
    if (next.has(field)) {
      next.delete(field);
    } else {
      next.add(field);
    }
    setSkippedFields(next);
  }

  function buildAcceptedPatch(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const [k, v] of patchEntries) {
      if (skippedFields.has(k)) continue;
      out[k] = v;
    }
    return out;
  }

  async function handleSubmitAnswers() {
    if (target.kind !== "chunk") return;
    const turn = (proposal?.turn ?? 1) + 1;
    const allAnswered = followup.every((q) => (answers[q.id] ?? "").trim().length > 0);
    if (!allAnswered) return;
    setPhase("answering");
    setErrorObj(null);
    try {
      const data = await props.answerChunk(target.id, {
        sessionId: proposal?.sessionId,
        previousPatch: proposal?.patch ?? null,
        answers: followup.map((q) => ({
          id: q.id,
          field: q.field,
          text: answers[q.id] ?? ""
        })),
        turn
      });
      setProposal(data as AiRepairProposal);
      setSkippedFields(new Set());
      setAnswers({});
      setPhase("reviewing");
    } catch (err) {
      setErrorObj(err instanceof Error ? err : new Error(String(err)));
      setPhase("error");
    }
  }

  async function handleApply(thenVerify: boolean) {
    setPhase("applying");
    setErrorObj(null);
    try {
      const acceptedFields = patchEntries
        .map(([k]) => k)
        .filter((k) => !skippedFields.has(k) && k !== "extras");
      const skipped = Array.from(skippedFields);
      await props.onApply(
        target,
        buildAcceptedPatch(),
        { thenVerify },
        {
          sessionId: proposal?.sessionId,
          turn: proposal?.turn,
          confidenceHint: proposal?.confidenceHint,
          acceptedFields,
          skippedFields: skipped
        }
      );
      onClose();
    } catch (err) {
      setErrorObj(err instanceof Error ? err : new Error(String(err)));
      setPhase("error");
    }
  }

  const acceptedCount = patchEntries.length - skippedFields.size;
  const finalTurnReached = (proposal?.turn ?? 1) >= 3 || proposal?.isFinalTurn === true;
  const stillMissingDisplay = stillMissing.length > 0;

  return (
    <div className="aiRepairScrim" role="dialog" aria-modal="true" aria-label="AI 自主修复">
      <div className="aiRepairPanel">
        <div className="aiRepairPanel__head">
          <div className="title">
            <Sparkles size={16} className="ai" />
            <span>AI 自主修复</span>
            <span className="aiRepairPanel__subject">
              · {target.kind === "chunk" ? "切片" : "知识包"}：{target.label}
            </span>
          </div>
          <button
            type="button"
            className="iconButton"
            onClick={onClose}
            aria-label="关闭"
          >
            <X size={16} />
          </button>
        </div>

        <div className="aiRepairPanel__body">
          {phase === "proposing" && (
            <div className="aiRepairSection">
              <div className="aiRepairSection__title">AI 正在阅读知识，请稍候</div>
              <div className="aiRepairLoading">分析切片所在领域、原文证据、父知识包……</div>
            </div>
          )}

          {phase === "error" && (
            <div className="aiRepairSection">
              <div className="aiRepairSection__title">AI 提案出错</div>
              {errorObj ? (
                <LlmErrorBanner error={errorObj} onRetry={retryPropose} />
              ) : (
                <div className="error">{errorMsg ?? "未知错误"}</div>
              )}
            </div>
          )}

          {(phase === "reviewing" || phase === "answering" || phase === "applying") && proposal && (
            <>
              {Object.keys(interpretation).length > 0 && (
                <div className="aiRepairSection">
                  <div className="aiRepairSection__title">AI 对该条知识的理解</div>
                  <div className="aiInterpretation">
                    {Object.entries(interpretation).map(([k, v]) => {
                      if (v === null || v === undefined || v === "") return null;
                      const label = aiInterpretationLabel(k);
                      const text =
                        typeof v === "string"
                          ? v
                          : Array.isArray(v)
                          ? (v as unknown[]).map(String).join(" / ")
                          : JSON.stringify(v);
                      return (
                        <div key={k}>
                          <span>{label}</span>
                          <strong>{text}</strong>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              <div className="aiRepairSection">
                <div className="aiRepairSection__title">
                  AI 提案 · 共 {patchEntries.length} 项 · 已接受 {acceptedCount} 项 · 自评可信度{" "}
                  {proposal.confidenceHint ?? 0}/100
                </div>
                {patchEntries.length === 0 ? (
                  <div className="aiRepairEmpty">AI 未生成任何字段提案；可在下方追问中补充信息。</div>
                ) : (
                  patchEntries.map(([field, value]) => {
                    const before = aiRepairCurrentValue(
                      target,
                      field,
                      props.chunks,
                      props.packs
                    );
                    const skipped = skippedFields.has(field);
                    return (
                      <div
                        key={field}
                        className={`aiPatchRow${skipped ? " skipped" : ""}`}
                      >
                        <div className="aiPatchRow__field">{aiRepairFieldLabel(field)}</div>
                        <div className="aiPatchRow__before">
                          <span>现值：</span>
                          {aiRepairFormatValue(before)}
                        </div>
                        <div className="aiPatchRow__after">
                          <span>AI 建议：</span>
                          {aiRepairFormatValue(value)}
                        </div>
                        <div className="aiPatchRow__actions">
                          <button
                            type="button"
                            className="ghost compactButton"
                            onClick={() => toggleSkip(field)}
                          >
                            {skipped ? "接受此项" : "跳过此项"}
                          </button>
                        </div>
                      </div>
                    );
                  })
                )}
              </div>

              {missing.length > 0 && (
                <div className="aiRepairSection">
                  <div className="aiRepairSection__title">
                    AI 暂无法补完的字段（{missing.length}）
                  </div>
                  <ul className="aiMissingList">
                    {missing.map((m, idx) => {
                      const field = typeof m === "string" ? m : m.field;
                      const reason = typeof m === "string" ? null : m.reason;
                      return (
                        <li key={idx}>
                          <strong>{aiRepairFieldLabel(field)}</strong>
                          {reason ? <span>· {String(reason)}</span> : null}
                        </li>
                      );
                    })}
                  </ul>
                </div>
              )}

              {target.kind === "chunk" && followup.length > 0 && !finalTurnReached && (
                <div className="aiRepairSection">
                  <div className="aiRepairSection__title">
                    AI 还需要 {followup.length} 项信息确认
                  </div>
                  {followup.map((q) => (
                    <div key={q.id} className="aiQaBubble">
                      <div className="aiQaBubble__q">
                        <Bot size={14} className="ai" />
                        <span>
                          AI：{q.question}
                          {q.field ? (
                            <span className="aiQaBubble__field">
                              · 关联字段：{aiRepairFieldLabel(q.field)}
                            </span>
                          ) : null}
                        </span>
                      </div>
                      <div className="aiQaBubble__a">
                        <textarea
                          value={answers[q.id] ?? ""}
                          onChange={(e) =>
                            setAnswers({ ...answers, [q.id]: e.target.value })
                          }
                          placeholder="把你知道的事实写在这里，AI 只摘取与字段直接相关的部分。"
                        />
                      </div>
                    </div>
                  ))}
                  <div className="aiRepairSection__inlineFoot">
                    <button
                      type="button"
                      className="primary"
                      onClick={() => void handleSubmitAnswers()}
                      disabled={
                        phase === "answering" ||
                        followup.some((q) => (answers[q.id] ?? "").trim().length === 0)
                      }
                    >
                      {phase === "answering" ? "AI 合并中…" : "提交回答继续修复"}
                    </button>
                  </div>
                </div>
              )}

              {target.kind === "pack" && followup.length > 0 && (
                <div className="aiRepairSection">
                  <div className="aiRepairSection__title">
                    AI 提示需要的额外信息（{followup.length}）
                  </div>
                  <ul className="aiMissingList">
                    {followup.map((q) => (
                      <li key={q.id}>
                        <strong>{q.field ? aiRepairFieldLabel(q.field) : "提示"}</strong>
                        <span>· {q.question}</span>
                      </li>
                    ))}
                  </ul>
                  <div className="aiRepairWarn">
                    知识包修复一轮即结束。请在保存前手动在编辑器里补充上述字段。
                  </div>
                </div>
              )}

              {patch.extras && typeof patch.extras === "object" && (
                <div className="aiRepairSection">
                  <div className="aiRepairSection__title">领域专属附加字段（extras）</div>
                  <div className="aiExtrasGrid">
                    {Object.entries(patch.extras as Record<string, unknown>).map(([k, v]) => (
                      <div key={k} className="aiExtrasItem">
                        <em>{k}</em>
                        <span>{aiRepairFormatValue(v)}</span>
                      </div>
                    ))}
                  </div>
                  <div className="aiRepairWarn aiRepairWarn--quiet">
                    extras 不写入主字段，仅审计记录。如果你希望持久化，请把它收敛到对应主字段。
                  </div>
                </div>
              )}

              {stillMissingDisplay && (target.kind === "pack" || finalTurnReached) && (
                <div className="aiRepairSection">
                  <div className="aiRepairSection__title">仍信息不足</div>
                  <div className="aiRepairWarn">
                    以下字段需要运营手动补完：
                    {stillMissing
                      .map((m) => (typeof m === "string" ? m : m.field))
                      .map(aiRepairFieldLabel)
                      .join(" / ")}
                  </div>
                </div>
              )}
            </>
          )}
        </div>

        <div className="aiRepairPanel__foot">
          {phase === "error" ? (
            <button type="button" className="secondary" onClick={onClose}>
              关闭
            </button>
          ) : (
            <>
              <button type="button" className="secondary" onClick={onClose}>
                全部驳回
              </button>
              {target.kind === "chunk" ? (
                <>
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void handleApply(false)}
                    disabled={
                      phase !== "reviewing" || acceptedCount === 0
                    }
                  >
                    应用所有接受字段
                  </button>
                  <button
                    type="button"
                    className="success"
                    onClick={() => void handleApply(true)}
                    disabled={
                      phase !== "reviewing" || acceptedCount === 0
                    }
                  >
                    应用并立即运营确认
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="primary"
                  onClick={() => void handleApply(false)}
                  disabled={phase !== "reviewing" || acceptedCount === 0}
                >
                  应用所有接受字段
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

const INTENT_LABELS: Record<string, string> = {
  create_chunk: "新建",
  update_chunk: "修改",
  update_pack: "修改 pack",
  clarify_chunk: "澄清",
  digest_action: "派工",
  freeform: "自由对话"
};

const DIGEST_SUGGESTED_ACTION_LABELS: Record<string, string> = {
  fix_chunk: "修补 chunk",
  archive_chunk: "归档 chunk",
  fix_pack: "修补 pack",
  review_evolution: "走 evolution 评审",
  rebuild_index: "重建索引",
  ignore: "忽略"
};

const DIGEST_SEVERITY_LABELS: Record<string, string> = {
  critical: "critical",
  warn: "warn",
  info: "info"
};

const DIGEST_KIND_LABELS: Record<string, string> = {
  chunk_caused_block: "Chunk 触发拦截",
  chunk_missing_field: "Chunk 字段不全",
  chunk_outdated: "Chunk 过期",
  pack_outdated: "Pack 过期",
  evolution_pending: "Evolution 待处理",
  trend_pattern: "趋势异常",
  freeform: "自由形态"
};

// ── AiInboxStrip：顶部 AI 待办流（GET /operation-knowledge/inbox 聚合） ───
type InboxCardView = {
  id: string;
  priority: "high" | "mid" | "low" | string;
  kind: string;
  title: string;
  contextSummary: string;
  targetChunkId?: string | null;
  targetPackId?: string | null;
  suggestedActions: string[];
  origin: string;
  createdAt?: string;
};

type InboxStatsView = { total: number; high: number; mid: number; low: number };

type InboxResponseView = { items: InboxCardView[]; stats: InboxStatsView };

const INBOX_PRIORITY_LABEL: Record<string, string> = {
  high: "高",
  mid: "中",
  low: "低"
};

const INBOX_KIND_LABEL: Record<string, string> = {
  create_chunk: "新建切片",
  repair_chunk: "修复切片",
  fill_field: "补字段",
  extract_tags: "抽标签",
  merge_chunks: "整合切片"
};

const INBOX_ORIGIN_LABEL: Record<string, string> = {
  digest_card: "今日日报",
  quote_missing: "缺原文",
  anchors_missing: "缺锚点",
  pending_review: "待审"
};

function AiInboxStrip(props: {
  onOpenChat: (injection: string) => void;
  onOpenManual: (target: { kind: "chunk" | "pack"; id: string } | null) => void;
  onDismissDigest: (cardId: string) => void;
}) {
  const [items, setItems] = useState<InboxCardView[]>([]);
  const [stats, setStats] = useState<InboxStatsView>({ total: 0, high: 0, mid: 0, low: 0 });
  const [phase, setPhase] = useState<"idle" | "loading" | "error">("idle");
  const [errorMsg, setErrorMsg] = useState("");
  const [expanded, setExpanded] = useState(false);

  const reload = async () => {
    setPhase("loading");
    setErrorMsg("");
    try {
      const data = await api.get<InboxResponseView>("/api/operation-knowledge/inbox?limit=24");
      setItems(Array.isArray(data.items) ? data.items : []);
      setStats(data.stats || { total: 0, high: 0, mid: 0, low: 0 });
      setPhase("idle");
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : String(err));
      setPhase("error");
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const visible = expanded ? items : items.slice(0, 5);
  const hiddenCount = items.length - visible.length;

  return (
    <section className="aiInbox" aria-label="今日 AI 待办">
      <header className="aiInbox__head">
        <div>
          <span className="aiInbox__title">今日 AI 待办</span>
          <span className="aiInbox__meta">
            {phase === "loading"
              ? "加载中…"
              : phase === "error"
              ? `读取失败：${errorMsg || "未知错误"}`
              : `${stats.total} 条 · 高 ${stats.high} / 中 ${stats.mid} / 低 ${stats.low}`}
          </span>
        </div>
        <button
          type="button"
          className="ghostButton"
          onClick={() => void reload()}
          disabled={phase === "loading"}
        >
          <RefreshCw size={12} /> 刷新
        </button>
      </header>

      {phase === "idle" && items.length === 0 ? (
        <div className="aiInbox__empty">
          AI 暂无待办。可以直接在下方与 AI 对话，或让 AI 重算今日日报。
        </div>
      ) : null}

      {visible.map((card) => {
        const priorityCls = `aiInbox__priority aiInbox__priority--${card.priority}`;
        const offerChat = card.suggestedActions.includes("open_chat");
        const offerDismiss = card.suggestedActions.includes("dismiss");
        const titleSafe = stripHtml(card.title);
        const ctxSafe = stripHtml(card.contextSummary);
        return (
          <div key={card.id} className="aiInbox__row">
            <span className={priorityCls}>{INBOX_PRIORITY_LABEL[card.priority] || card.priority}</span>
            <span className="aiInbox__kind">{INBOX_KIND_LABEL[card.kind] || card.kind}</span>
            <div className="aiInbox__main">
              <div className="aiInbox__title2" title={titleSafe}>
                {titleSafe}
              </div>
              {ctxSafe ? <div className="aiInbox__sub">{ctxSafe}</div> : null}
              <div className="aiInbox__origin">
                {INBOX_ORIGIN_LABEL[card.origin] || card.origin}
              </div>
            </div>
            <div className="aiInbox__actions">
              {offerChat ? (
                <button
                  type="button"
                  className="primaryButton"
                  onClick={() => {
                    const target = card.targetChunkId
                      ? `chunk:${card.targetChunkId}`
                      : card.targetPackId
                      ? `pack:${card.targetPackId}`
                      : "—";
                    const blob =
                      `请帮我处理这条 AI 待办：\n` +
                      `优先级：${INBOX_PRIORITY_LABEL[card.priority] || card.priority}\n` +
                      `类型：${INBOX_KIND_LABEL[card.kind] || card.kind}\n` +
                      `标题：${titleSafe}\n` +
                      `上下文：${ctxSafe}\n` +
                      `目标：${target}\n` +
                      `来源：${INBOX_ORIGIN_LABEL[card.origin] || card.origin}`;
                    props.onOpenChat(blob);
                  }}
                >
                  谈谈
                </button>
              ) : null}
              {(card.targetChunkId || card.targetPackId) ? (
                <button
                  type="button"
                  className="ghostButton"
                  onClick={() =>
                    props.onOpenManual(
                      card.targetChunkId
                        ? { kind: "chunk", id: card.targetChunkId }
                        : card.targetPackId
                        ? { kind: "pack", id: card.targetPackId }
                        : null
                    )
                  }
                >
                  打开
                </button>
              ) : null}
              {offerDismiss && card.origin === "digest_card" ? (
                <button
                  type="button"
                  className="ghostButton"
                  onClick={() => {
                    const cardId = card.id.startsWith("digest:")
                      ? card.id.slice("digest:".length)
                      : card.id;
                    props.onDismissDigest(cardId);
                    setItems((prev) => prev.filter((c) => c.id !== card.id));
                  }}
                >
                  不采纳
                </button>
              ) : null}
            </div>
          </div>
        );
      })}

      {hiddenCount > 0 ? (
        <button
          type="button"
          className="aiInbox__expand"
          onClick={() => setExpanded(true)}
        >
          展开 {hiddenCount} 条更多 ▾
        </button>
      ) : null}
    </section>
  );
}

function KnowledgeDigestCanvas(props: {
  report: KnowledgeDailyReportView | null;
  phase: "idle" | "loading" | "regenerating" | "error";
  error: Error | null;
  selectedCardIds: string[];
  onToggleSelect: (cardId: string) => void;
  onSelectAll: () => void;
  onInvertSelect: () => void;
  onIgnoreInfo: () => Promise<void>;
  onRegenerate: () => Promise<void>;
  onDismissCard: (cardId: string) => Promise<void>;
  onDispatchSelected: () => void;
  onDispatchOne: (cardId: string) => void;
  onOpenCard: (card: KnowledgeDigestCardView) => void;
}) {
  const {
    report,
    phase,
    error,
    selectedCardIds,
    onToggleSelect,
    onSelectAll,
    onInvertSelect,
    onIgnoreInfo,
    onRegenerate,
    onDismissCard,
    onDispatchSelected,
    onDispatchOne,
    onOpenCard
  } = props;
  const dismissed = new Set(report?.dismissedCardIds || []);
  const cards = (report?.cards || []).filter((c) => !dismissed.has(c.cardId));
  const selectedCount = selectedCardIds.length;

  return (
    <section className="knowledgeDigestCanvas" aria-label="今日知识库日报画布">
      <header className="knowledgeDigestCanvas__head">
        <div className="knowledgeDigestCanvas__title">
          <strong>📅 {report?.reportDate || todayLocalDate()} 日报</strong>
          <span className="muted small">
            {report
              ? `生成于 ${report.generatedAt?.slice(0, 19) || ""} · ${cards.length} 条待处理`
              : phase === "loading"
              ? "AI 正在合成日报..."
              : "暂无日报"}
          </span>
        </div>
        <div className="knowledgeDigestCanvas__toolbar">
          <button
            type="button"
            className="secondaryButton"
            onClick={() => void onRegenerate()}
            disabled={phase === "regenerating" || phase === "loading"}
          >
            <RefreshCw size={14} /> 重算今日
          </button>
          <button
            type="button"
            className="ghostButton"
            onClick={onSelectAll}
            disabled={cards.length === 0}
          >
            全选
          </button>
          <button
            type="button"
            className="ghostButton"
            onClick={onInvertSelect}
            disabled={cards.length === 0}
          >
            反选
          </button>
          <button
            type="button"
            className="ghostButton"
            onClick={() => void onIgnoreInfo()}
            disabled={cards.length === 0}
          >
            一键忽略 info
          </button>
        </div>
      </header>

      {phase === "error" && error ? (
        <LlmErrorBanner error={error} onRetry={() => void onRegenerate()} retrying={false} />
      ) : null}

      {report?.status === "failed" ? (
        <div className="knowledgeDigestCanvas__statusBanner knowledgeDigestCanvas__statusBanner--failed">
          AI 合成失败（{report.errorKind || "未知"}）。可点「重算今日」让 AI 再试。
        </div>
      ) : null}
      {report?.status === "partial" ? (
        <div className="knowledgeDigestCanvas__statusBanner knowledgeDigestCanvas__statusBanner--partial">
          AI 在预算上限内只完成了部分分析（{report.errorKind || "budget_exceeded"}）。
        </div>
      ) : null}

      <div className="knowledgeDigestCanvas__list">
        {phase === "loading" && !report ? (
          <div className="knowledgeDigestCanvas__empty">AI 正在合成日报，请稍候...</div>
        ) : cards.length === 0 ? (
          <div className="knowledgeDigestCanvas__empty">
            {phase === "idle" && report
              ? "今日无待处理 issue（AI 已巡检过 24h 内 chunk 健康度 / usage / runs / evolution）。"
              : "暂无卡片。"}
          </div>
        ) : (
          cards.map((card) => {
            const checked = selectedCardIds.includes(card.cardId);
            const severity = card.severity || "info";
            return (
              <article
                key={card.cardId}
                className={`cardSurface knowledgeDigestCard knowledgeDigestCard--${severity}`}
              >
                <div className="knowledgeDigestCard__head">
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => onToggleSelect(card.cardId)}
                    aria-label={`选中卡片 ${card.title}`}
                  />
                  <span className={`severityChip severityChip--${severity}`}>
                    {DIGEST_SEVERITY_LABELS[severity] || severity}
                  </span>
                  <span className="knowledgeDigestCard__kind">
                    {DIGEST_KIND_LABELS[card.kind] || card.kind}
                  </span>
                  <strong className="knowledgeDigestCard__title">{card.title}</strong>
                </div>
                <p className="knowledgeDigestCard__summary">{stripHtml(card.summary)}</p>
                <div className="knowledgeDigestCard__meta">
                  {card.metric ? (
                    <span className="metricChip">
                      {card.metric.name || "metric"}: {card.metric.value ?? "—"}
                      {card.metric.threshold !== undefined ? ` / 阈值 ${card.metric.threshold}` : ""}
                    </span>
                  ) : null}
                  {(card.targetRefs || []).slice(0, 4).map((ref, idx) => (
                    <span key={idx} className="metricChip metricChip--ref">
                      {ref.kind}: {(ref.id || "").slice(0, 10)}
                    </span>
                  ))}
                  <span className="muted small">
                    建议：{DIGEST_SUGGESTED_ACTION_LABELS[card.suggestedAction] || card.suggestedAction}
                  </span>
                </div>
                <div className="knowledgeDigestCard__actions">
                  <button
                    type="button"
                    className="primaryButton"
                    onClick={() => onDispatchOne(card.cardId)}
                  >
                    让 AI 单独处理
                  </button>
                  <button
                    type="button"
                    className="ghostButton"
                    onClick={() => void onDismissCard(card.cardId)}
                  >
                    忽略
                  </button>
                  <button
                    type="button"
                    className="ghostButton"
                    onClick={() => onOpenCard(card)}
                  >
                    打开
                  </button>
                </div>
              </article>
            );
          })
        )}
      </div>

      <footer className="knowledgeDigestCanvas__foot">
        <button
          type="button"
          className="primaryButton"
          onClick={onDispatchSelected}
          disabled={selectedCount === 0}
        >
          💬 让 AI 处理选中的 {selectedCount} 条
        </button>
      </footer>
    </section>
  );
}

function KnowledgeChatPanel(props: {
  open: boolean;
  initialSessionId?: string;
  accountId?: string;
  mode?: "drawer" | "docked";
  pendingInjection?: string | null;
  onInjectionConsumed?: () => void;
  onClose: (persistedSessionId?: string) => void;
  onApplied: () => void;
  postTurn: (body: {
    sessionId?: string;
    accountId?: string;
    content: string;
    attachments?: Array<{ chunkId?: string; itemId?: string }>;
  }) => Promise<KnowledgeChatTurnResponse>;
  getHistory: (
    sessionId: string
  ) => Promise<{ sessionId: string; items: KnowledgeChatTurnView[]; total: number }>;
  apply: (
    sessionId: string,
    accountId?: string
  ) => Promise<{ ok: boolean; sessionId: string; intent: string; result: Record<string, unknown> }>;
  discard: (sessionId: string) => Promise<{ ok: boolean; sessionId: string; discardedCount: number }>;
  /// knowledge-digest-workstation Phase 4 / P4.4：派工长任务。
  /// 当 chat_turn 返回 intent="digest_action" + plannedSteps 时，
  /// panel 渲染「派工确认」小卡，运营点确认后调用本函数落 task。
  postChatTask?: (body: {
    sessionId: string;
    accountId?: string;
    operatorId?: string;
    cardIds?: string[];
    plannedSteps: Array<Record<string, unknown>>;
  }) => Promise<{ taskId: string; sessionId: string; status: string; totalSteps: number }>;
}) {
  const { open, accountId, onClose, onApplied } = props;
  const mode = props.mode || "drawer";
  const [sessionId, setSessionId] = useState<string | undefined>(props.initialSessionId);
  const [turns, setTurns] = useState<KnowledgeChatTurnView[]>([]);
  const [draftKind, setDraftKind] = useState<string | null>(null);
  const [draftPatch, setDraftPatch] = useState<Record<string, unknown> | null>(null);
  const [missingFields, setMissingFields] = useState<string[]>([]);
  const [followups, setFollowups] = useState<
    Array<{ id?: string; field?: string; question?: string }>
  >([]);
  const [canApply, setCanApply] = useState(false);
  const [phase, setPhase] = useState<"idle" | "sending" | "applying" | "discarding" | "error">("idle");
  const [error, setError] = useState<Error | null>(null);
  // 失败时记录"刚才在做什么"，让 LlmErrorBanner 的「AI 重试」按钮能够精准重放
  // 触发失败的那一步，而不是盲目刷新整个 panel。
  const [lastFailedAction, setLastFailedAction] =
    useState<"sendTurn" | "apply" | "discard" | "loadHistory" | null>(null);
  const [pendingInput, setPendingInput] = useState<string>("");
  const [input, setInput] = useState("");
  const streamRef = useRef<HTMLDivElement | null>(null);
  // P4.4：digest_action intent 命中后由 chat_turn 返回的 plannedSteps + 概要。
  // 运营点「确认派工」即调 postChatTask 把它们落 KnowledgeChatTask；
  // 取消则直接清空，不影响其它草稿状态。
  const [pendingPlannedSteps, setPendingPlannedSteps] = useState<
    Array<Record<string, unknown>> | null
  >(null);
  const [estimatedLlmCalls, setEstimatedLlmCalls] = useState<number | null>(null);
  const [dispatchPhase, setDispatchPhase] = useState<"idle" | "submitting" | "error">("idle");
  const [dispatchError, setDispatchError] = useState<Error | null>(null);

  useEffect(() => {
    if (props.pendingInjection && props.pendingInjection.length > 0) {
      setInput((prev) => (prev ? `${prev}\n${props.pendingInjection}` : props.pendingInjection!));
      props.onInjectionConsumed?.();
    }
  }, [props.pendingInjection]);

  useEffect(() => {
    if (!open) return;
    setError(null);
    if (!props.initialSessionId) {
      setSessionId(undefined);
      setTurns([]);
      setDraftKind(null);
      setDraftPatch(null);
      setMissingFields([]);
      setFollowups([]);
      setCanApply(false);
      return;
    }
    setSessionId(props.initialSessionId);
    let cancelled = false;
    void props
      .getHistory(props.initialSessionId)
      .then((data) => {
        if (cancelled) return;
        setTurns(data.items || []);
        const lastAssistant = [...(data.items || [])]
          .reverse()
          .find((t) => t.role === "assistant" && t.status === "pending");
        if (lastAssistant) {
          setDraftPatch((lastAssistant.patch as Record<string, unknown> | null) || null);
          setMissingFields(lastAssistant.missingFields || []);
          setFollowups(lastAssistant.followupQuestions || []);
          const intent = lastAssistant.intent || "freeform";
          const dk =
            intent === "update_pack"
              ? "pack"
              : intent === "create_chunk" || intent === "update_chunk"
              ? "chunk"
              : null;
          setDraftKind(dk);
          setCanApply(
            !!lastAssistant.patch &&
              (lastAssistant.missingFields?.length || 0) === 0 &&
              !!dk
          );
        }
      })
      .catch((err) => {
        if (cancelled) return;
        setError(err instanceof Error ? err : new Error(String(err)));
        setLastFailedAction("loadHistory");
        setPhase("error");
      });
    return () => {
      cancelled = true;
    };
  }, [open, props.initialSessionId]);

  useEffect(() => {
    if (!streamRef.current) return;
    streamRef.current.scrollTop = streamRef.current.scrollHeight;
  }, [turns.length]);

  // knowledge-digest-workstation Phase 4 / P4.4：SSE 拉 worker 写的 task_progress
  // / task_summary turn。`/api/knowledge/chat/sessions/:sid/stream` 在每次 bump
  // 时推一条 `event: turn`；客户端拿到后 GET history 拿增量。
  // 注意：开 panel 但还没建 session 的阶段不订阅；session 建好或回放历史触发后才订阅。
  useEffect(() => {
    if (!open || !sessionId) return;
    let closed = false;
    const url = `/api/knowledge/chat/sessions/${encodeURIComponent(sessionId)}/stream`;
    const es = new EventSource(url);
    const refetch = () => {
      if (closed) return;
      void props
        .getHistory(sessionId)
        .then((data) => {
          if (closed) return;
          setTurns(data.items || []);
        })
        .catch(() => {
          /* SSE 触发失败不弹错 banner，避免 worker 长任务噪声打扰 chat 主流程 */
        });
    };
    es.addEventListener("turn", refetch);
    // P1-6：worker 写完 summary 后服务端会推一条 `event: close`，标志本 session
    // 已终态——前端拉一次最终 history 后主动 close EventSource，不再重连。
    es.addEventListener("close", () => {
      refetch();
      closed = true;
      es.close();
    });
    es.onerror = () => {
      // EventSource 默认会自动重连；只在 close 阶段彻底放弃。
      if (closed) {
        es.close();
      }
    };
    return () => {
      closed = true;
      es.close();
    };
  }, [open, sessionId]);

  // P2-14：Esc 关闭抽屉。docked 模式下不抢键（运营在嵌入面板里编辑文档时
  // 会大量按 Esc 取消下拉/选区），仅在 drawer 模式下生效。
  useEffect(() => {
    if (!open) return;
    if (props.mode !== "drawer") return;
    const handleEsc = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // 输入法 composing 中不关：避免运营输入中文时误触。
      if (event.isComposing) return;
      onClose(sessionId);
    };
    window.addEventListener("keydown", handleEsc);
    return () => {
      window.removeEventListener("keydown", handleEsc);
    };
  }, [open, props.mode, onClose, sessionId]);

  const sessionTurnCount = turns.filter((t) => t.role === "assistant").length;
  const turnLimitReached = sessionTurnCount >= 8;

  async function handleSend(retryText?: string) {
    const trimmed = (retryText ?? input).trim();
    if (!trimmed || phase === "sending" || phase === "applying") return;
    setPhase("sending");
    setError(null);
    setLastFailedAction(null);
    setPendingInput(trimmed);
    try {
      const res = await props.postTurn({
        sessionId,
        accountId,
        content: trimmed
      });
      setSessionId(res.sessionId);
      writePersistedChatSession(accountId, res.sessionId);
      setTurns((prev) => [
        ...prev,
        {
          turnIndex: res.turnIndex - 1,
          role: "user",
          content: trimmed,
          status: "pending",
          createdAt: new Date().toISOString()
        },
        {
          turnIndex: res.turnIndex,
          role: "assistant",
          intent: res.intent,
          content: res.naturalReply,
          patch: res.draftPreview ?? null,
          missingFields: res.missingFields,
          followupQuestions: res.followupQuestions,
          status: "pending",
          tokensUsed: res.tokensUsed,
          promptKey: res.promptKey,
          createdAt: new Date().toISOString()
        }
      ]);
      setDraftKind(res.draftKind ?? null);
      setDraftPatch(res.draftPreview ?? null);
      setMissingFields(res.missingFields);
      setFollowups(res.followupQuestions);
      setCanApply(res.canApply);
      // P4.4：digest_action 命中 → 缓存 plannedSteps，等运营点「确认派工」。
      if (res.intent === "digest_action" && res.plannedSteps && res.plannedSteps.length > 0) {
        setPendingPlannedSteps(res.plannedSteps);
        setEstimatedLlmCalls(res.estimatedLlmCalls ?? null);
      } else {
        setPendingPlannedSteps(null);
        setEstimatedLlmCalls(null);
      }
      setInput("");
      setPendingInput("");
      setPhase("idle");
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setLastFailedAction("sendTurn");
      setPhase("error");
    }
  }

  async function handleApply() {
    if (!sessionId || !canApply) return;
    setPhase("applying");
    setError(null);
    setLastFailedAction(null);
    try {
      await props.apply(sessionId, accountId);
      onApplied();
      setCanApply(false);
      setPhase("idle");
      onClose(sessionId);
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setLastFailedAction("apply");
      setPhase("error");
    }
  }

  async function handleDiscard() {
    if (!sessionId) {
      onClose();
      return;
    }
    setPhase("discarding");
    setError(null);
    setLastFailedAction(null);
    try {
      await props.discard(sessionId);
      clearPersistedChatSession(accountId);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setLastFailedAction("discard");
      setPhase("error");
    }
  }

  // P4.4：把 LLM 出的 plannedSteps 落 KnowledgeChatTask{status="pending"}；
  // worker 30s 内串行执行，期间通过 SSE 推 task_progress / task_summary turn。
  async function handleDispatch() {
    if (!sessionId || !pendingPlannedSteps || pendingPlannedSteps.length === 0) return;
    if (!props.postChatTask) {
      setDispatchError(new Error("当前面板未注入派工接口"));
      setDispatchPhase("error");
      return;
    }
    const cardIds: string[] = pendingPlannedSteps
      .map((s) => (typeof s.cardId === "string" ? (s.cardId as string) : ""))
      .filter((x) => x.length > 0);
    setDispatchPhase("submitting");
    setDispatchError(null);
    try {
      await props.postChatTask({
        sessionId,
        accountId: props.accountId,
        cardIds: cardIds.length > 0 ? cardIds : undefined,
        plannedSteps: pendingPlannedSteps
      });
      setPendingPlannedSteps(null);
      setEstimatedLlmCalls(null);
      setDispatchPhase("idle");
    } catch (err) {
      setDispatchError(err instanceof Error ? err : new Error(String(err)));
      setDispatchPhase("error");
    }
  }

  function handleCancelDispatch() {
    setPendingPlannedSteps(null);
    setEstimatedLlmCalls(null);
    setDispatchError(null);
    setDispatchPhase("idle");
  }

  function retryLastAction() {
    if (lastFailedAction === "sendTurn" && pendingInput) {
      void handleSend(pendingInput);
    } else if (lastFailedAction === "apply") {
      void handleApply();
    } else if (lastFailedAction === "discard") {
      void handleDiscard();
    } else if (lastFailedAction === "loadHistory" && props.initialSessionId) {
      // 重新触发 effect：清空 turns 让 effect 重跑
      setError(null);
      setTurns([]);
      props
        .getHistory(props.initialSessionId)
        .then((data) => {
          setTurns(data.items || []);
          setPhase("idle");
        })
        .catch((err) => {
          setError(err instanceof Error ? err : new Error(String(err)));
          setLastFailedAction("loadHistory");
          setPhase("error");
        });
    }
  }

  if (!open) return null;

  const draftFields = draftPatch
    ? Object.entries(draftPatch).filter(([k]) => k !== "extras")
    : [];

  const isDocked = mode === "docked";
  const containerClass = isDocked ? "knowledgeChatDocked" : "knowledgeChatDrawer";
  const containerHeadClass = isDocked
    ? "knowledgeChatDocked__head"
    : "knowledgeChatDrawer__head";
  const containerBodyClass = isDocked
    ? "knowledgeChatDocked__body"
    : "knowledgeChatDrawer__body";

  return (
    <>
      {!isDocked && (
        <div className="knowledgeChatScrim" onClick={() => onClose(sessionId)} />
      )}
      <aside className={containerClass} role="dialog" aria-label="AI 对话补完知识库">
        <header className={containerHeadClass}>
          <div>
            <strong>AI 对话补完知识库</strong>
            <p className="muted small">
              {sessionId
                ? `session: ${sessionId.slice(0, 8)} · 第 ${sessionTurnCount} 轮`
                : "新会话 · 第一条消息发出后自动建会话"}
            </p>
          </div>
          {!isDocked && (
            <button
              type="button"
              className="iconButton"
              aria-label="关闭"
              onClick={() => onClose(sessionId)}
            >
              ×
            </button>
          )}
        </header>
        <div className={containerBodyClass}>
          <div className="knowledgeChatStream" ref={streamRef}>
            {turns.length === 0 ? (
              <div className="knowledgeChatEmpty">
                <p>告诉 AI 您想做什么，例如：</p>
                <ul>
                  <li>"再加一条针对宝妈用户的反对话术"</li>
                  <li>"这条只对个人号生效，企业号不适用，帮我修一下"</li>
                  <li>"销售口径里关于价格的部分需要再细化"</li>
                </ul>
                <p className="muted small">
                  AI 会起草 / 修改后落库为 <strong>草稿</strong>，需运营在切片编辑器二次审核后才进入检索池。
                </p>
              </div>
            ) : (
              turns.map((turn, idx) => (
                <div
                  key={`${turn.turnIndex}-${idx}`}
                  className={`bubbleRow ${turn.role === "assistant" ? "ai" : "user"}`}
                >
                  <div className="bubbleAvatar" aria-hidden="true">
                    {turn.role === "assistant" ? <Bot size={14} /> : <User2 size={14} />}
                  </div>
                  <div className="bubble">
                    {turn.role === "assistant" && turn.intent && (
                      <span className="knowledgeChatIntentBadge">
                        {INTENT_LABELS[turn.intent] || turn.intent}
                      </span>
                    )}
                    <p>{turn.content}</p>
                    {turn.role === "assistant" &&
                      (turn.followupQuestions?.length || 0) > 0 && (
                        <ul className="knowledgeChatFollowups">
                          {turn.followupQuestions!.map((q, i) => (
                            <li key={q.id || `${i}`}>
                              <strong>{q.field || "追问"}：</strong>
                              {q.question || ""}
                            </li>
                          ))}
                        </ul>
                      )}
                  </div>
                </div>
              ))
            )}
            {phase === "sending" && (
              <div className="bubbleRow ai">
                <div className="bubbleAvatar"><Bot size={14} /></div>
                <div className="bubble"><span className="muted small">AI 思考中...</span></div>
              </div>
            )}
            {phase === "applying" && (
              <div className="bubbleRow ai">
                <div className="bubbleAvatar"><Bot size={14} /></div>
                <div className="bubble"><span className="muted small">AI 落库为草稿中...</span></div>
              </div>
            )}
          </div>
          <aside className="knowledgeChatDraft">
            <div className="knowledgeChatDraft__title">当前草稿预览</div>
            {pendingPlannedSteps && pendingPlannedSteps.length > 0 ? (
              <div className="knowledgeChatDispatch">
                <p className="small">
                  AI 已拆出 <strong>{pendingPlannedSteps.length}</strong> 步派工
                  {estimatedLlmCalls != null && (
                    <span> · 预估 {estimatedLlmCalls} 次 LLM 调用</span>
                  )}
                </p>
                <ol className="knowledgeChatDispatch__steps">
                  {pendingPlannedSteps.map((s, idx) => {
                    const stepId =
                      typeof s.stepId === "string" ? s.stepId : `step_${idx + 1}`;
                    const action = typeof s.action === "string" ? s.action : "freeform";
                    const summary =
                      typeof s.summary === "string"
                        ? s.summary
                        : typeof s.naturalReply === "string"
                        ? (s.naturalReply as string)
                        : "";
                    return (
                      <li key={stepId}>
                        <span className="knowledgeChatDispatch__action">{action}</span>
                        <span>{summary}</span>
                      </li>
                    );
                  })}
                </ol>
                <p className="muted small">
                  确认后 AI 会按上述顺序串行处理；任意一步失败会跳过并继续。处理结果会以
                  <strong> task_progress / task_summary </strong>
                  形态实时回传到本对话流。
                </p>
                {dispatchError && (
                  <p className="small" style={{ color: "var(--danger)" }}>
                    派工失败：{dispatchError.message}
                  </p>
                )}
                <div className="knowledgeChatDraft__actions">
                  <button
                    type="button"
                    className="primary"
                    disabled={dispatchPhase === "submitting"}
                    onClick={() => void handleDispatch()}
                  >
                    {dispatchPhase === "submitting" ? "派工中..." : "确认派工"}
                  </button>
                  <button
                    type="button"
                    className="secondary"
                    disabled={dispatchPhase === "submitting"}
                    onClick={handleCancelDispatch}
                  >
                    取消
                  </button>
                </div>
              </div>
            ) : !draftPatch ? (
              <p className="muted small">尚无草稿。先在左侧告诉 AI 想做什么。</p>
            ) : (
              <>
                <p className="small muted">
                  类型：<strong>{draftKind === "pack" ? "知识包" : "切片"}</strong>
                  {missingFields.length > 0 && (
                    <span> · 缺 {missingFields.length} 项</span>
                  )}
                </p>
                {draftFields.map(([k, v]) => {
                  const isMissing = missingFields.includes(k);
                  const value =
                    typeof v === "string"
                      ? v
                      : v == null
                      ? ""
                      : JSON.stringify(v);
                  return (
                    <div
                      key={k}
                      className={`knowledgeChatDraft__field ${isMissing ? "missing" : "filled"}`}
                    >
                      <span className="knowledgeChatDraft__fieldLabel">{k}</span>
                      <span>{value || "（空）"}</span>
                    </div>
                  );
                })}
                {missingFields.length > 0 && (
                  <p className="small" style={{ color: "var(--danger)" }}>
                    缺失字段：{missingFields.join(" / ")} —— 请在左侧继续回答。
                  </p>
                )}
              </>
            )}
            <div className="knowledgeChatDraft__actions">
              <button
                type="button"
                className="primary"
                disabled={!canApply || phase === "applying"}
                onClick={() => void handleApply()}
              >
                应用为草稿
              </button>
              <button
                type="button"
                className="secondary"
                disabled={phase === "discarding"}
                onClick={() => void handleDiscard()}
              >
                丢弃此 session
              </button>
            </div>
          </aside>
        </div>
        {error && (
          <div className="knowledgeChatError">
            <LlmErrorBanner
              error={error}
              onRetry={lastFailedAction ? retryLastAction : undefined}
              retrying={phase === "sending" || phase === "applying" || phase === "discarding"}
            />
          </div>
        )}
        <div className="knowledgeChatInput">
          <textarea
            value={input}
            placeholder={
              turnLimitReached
                ? "本会话已达 8 轮上限，请『应用为草稿』或开启新会话"
                : "告诉 AI 您想做什么..."
            }
            disabled={turnLimitReached || phase === "sending" || phase === "applying"}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void handleSend();
              }
            }}
          />
          <button
            type="button"
            className="primary"
            disabled={
              !input.trim() ||
              turnLimitReached ||
              phase === "sending" ||
              phase === "applying"
            }
            onClick={() => void handleSend()}
          >
            <SendHorizonal size={14} /> 发送
          </button>
        </div>
      </aside>
    </>
  );
}

type KnowledgeDocModalState =
  | { mode: "create"; title: string; sourceName: string; summary: string }
  | { mode: "edit"; id: string; title: string; sourceName: string; summary: string };

function KnowledgeDocumentModal({
  state,
  onChange,
  onClose,
  onSubmit
}: {
  state: KnowledgeDocModalState;
  onChange: (next: KnowledgeDocModalState) => void;
  onClose: () => void;
  onSubmit: () => void;
}) {
  const heading = state.mode === "create" ? "手动新建知识文档" : "编辑文档元数据";
  const submitLabel = state.mode === "create" ? "创建文档" : "保存修改";
  const canSubmit = state.title.trim().length > 0;
  return (
    <>
      <div className="knowledgeChatScrim" onClick={onClose} />
      <div
        className="knowledgeDocModal"
        role="dialog"
        aria-modal="true"
        aria-label={heading}
      >
        <div className="knowledgeDocModal__head">
          <div>
            <strong>{heading}</strong>
            <span>仅维护文档元数据；切片仍由 AI 对话补完或手动新增</span>
          </div>
          <button
            type="button"
            className="knowledgeDocModal__close"
            aria-label="关闭"
            onClick={onClose}
          >
            <X size={14} />
          </button>
        </div>
        <form
          className="knowledgeDocModal__body"
          onSubmit={(e) => {
            e.preventDefault();
            if (canSubmit) onSubmit();
          }}
        >
          <label className="knowledgeDocModal__field">
            <span>文档标题（必填）</span>
            <input
              type="text"
              value={state.title}
              maxLength={120}
              autoFocus
              onChange={(e) => onChange({ ...state, title: e.target.value })}
            />
          </label>
          <label className="knowledgeDocModal__field">
            <span>来源名称（可选）</span>
            <input
              type="text"
              value={state.sourceName}
              maxLength={120}
              placeholder="如：销售口径文档 v3 / 客服 FAQ"
              onChange={(e) => onChange({ ...state, sourceName: e.target.value })}
            />
          </label>
          <label className="knowledgeDocModal__field">
            <span>摘要（可选）</span>
            <textarea
              rows={4}
              value={state.summary}
              maxLength={800}
              placeholder="一句话说明这份文档的内容范围，便于检索时定位"
              onChange={(e) => onChange({ ...state, summary: e.target.value })}
            />
          </label>
          <div className="knowledgeDocModal__foot">
            <button type="button" className="secondary" onClick={onClose}>
              取消
            </button>
            <button type="submit" className="primary" disabled={!canSubmit}>
              {submitLabel}
            </button>
          </div>
        </form>
      </div>
    </>
  );
}
