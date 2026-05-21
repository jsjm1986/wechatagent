import {
  Activity,
  Bot,
  BrainCircuit,
  CheckCircle2,
  Clock3,
  FileText,
  LibraryBig,
  LayoutDashboard,
  MessageSquareText,
  RefreshCw,
  Search,
  SendHorizonal,
  Settings2,
  ShieldCheck,
  Sparkles,
  SquarePen,
  UploadCloud,
  UserRoundCheck,
  UsersRound,
  Workflow
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { FormEvent, useEffect, useMemo, useState } from "react";

type AgentStatus = "normal" | "managed";
type Channel = "command" | "overview" | "userOps" | "groupOps" | "momentOps" | "knowledge" | "content" | "systemStrategy" | "operations" | "autonomy" | "quality";
type ContactTab = "all" | "managed" | "normal";
type SmartOpsTab = "cockpit" | "adjust" | "profile" | "memory" | "simulation" | "conversation";
type TraditionalOpsTab = "playbooks" | "prompts" | "settings" | "audit";
type UserOpsMode = "smart" | "traditional";
type OpsTab = "tasks" | "events" | "reviews" | "llm";
type KnowledgeTab = "documents" | "catalog" | "library" | "chunks" | "test" | "usage";

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
  sourceName: string;
  status: string;
  priority: string;
};

type OperationKnowledgeDocument = {
  id: string;
  title: string;
  sourceType: string;
  sourceName?: string;
  summary?: string;
  catalogSummary?: string;
  routingMap: string[];
  riskNotes: string[];
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

const api = {
  async get<T>(url: string): Promise<T> {
    const response = await fetch(url);
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  },
  async post<T>(url: string, body?: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined
    });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  },
  async put<T>(url: string, body: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  },
  async delete<T>(url: string): Promise<T> {
    const response = await fetch(url, { method: "DELETE" });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  }
};

const channels: Array<{ id: Channel; label: string; caption: string; icon: LucideIcon }> = [
  { id: "command", label: "AI 总控", caption: "Command Center", icon: BrainCircuit },
  { id: "overview", label: "工作台", caption: "运行态势", icon: LayoutDashboard },
  { id: "userOps", label: "用户运营", caption: "私聊关系运营", icon: UserRoundCheck },
  { id: "groupOps", label: "微信群运营", caption: "群分析与线索", icon: UsersRound },
  { id: "momentOps", label: "朋友圈运营", caption: "内容计划", icon: Sparkles },
  { id: "knowledge", label: "运营知识库", caption: "知识路由", icon: LibraryBig },
  { id: "content", label: "内容资产", caption: "素材知识", icon: FileText },
  { id: "systemStrategy", label: "系统策略", caption: "全局与总控", icon: Settings2 },
  { id: "operations", label: "任务日志", caption: "执行审计", icon: Activity },
  // W6 / Task 7.2：自治回路监控提到顶级频道（原 QualityCenterView 子 Tab 已下沉到此）
  { id: "autonomy", label: "自治回路监控", caption: "Autonomy Loop", icon: ShieldCheck },
  // 波 B4-B6：运营成效中心（outcome metrics / auto-verify / formula-adherence / 标记词编辑）
  { id: "quality", label: "运营成效", caption: "指标与质量", icon: Workflow }
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
  const [knowledgeTab, setKnowledgeTab] = useState<KnowledgeTab>("documents");
  const [knowledgeImportSource, setKnowledgeImportSource] = useState("运营知识导入");
  const [knowledgeImportText, setKnowledgeImportText] = useState("");
  const [knowledgeImportPreview, setKnowledgeImportPreview] = useState<OperationKnowledgeImportPreview>({
    document: null,
    items: [],
    chunks: []
  });
  const [knowledgeTestMessage, setKnowledgeTestMessage] = useState("客户问：你们能不能保证转化提升？有没有真实案例？");
  const [knowledgeTestResult, setKnowledgeTestResult] = useState<Record<string, unknown> | null>(null);
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
      setKnowledgeTab("documents");
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
      setKnowledgeTab("catalog");
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

  async function verifyKnowledgeChunk(id: string) {
    await run(async () => {
      await api.post(`/api/operation-knowledge/chunks/${id}/verify`, {});
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
      sourceName: item.sourceName ?? "",
      status: item.status,
      priority: String(item.priority ?? 0)
    });
    setKnowledgeTab("library");
    setActiveChannel("knowledge");
  }

  function newKnowledgeDraft() {
    setEditingKnowledgeId("");
    setKnowledgeDraft(emptyKnowledgeDraft());
  }

  function editChunk(item: OperationKnowledgeChunk) {
    setEditingChunkId(item.id);
    setChunkDraft(draftFromChunk(item));
    setKnowledgeTab("chunks");
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

  useEffect(() => {
    void loadAll().catch((err) => setError(err instanceof Error ? err.message : String(err)));
  }, [currentAccountId]);

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
                  onRunMemoryConsolidation={() => void runMemoryConsolidation()}
                  onRunSimulation={() => void runDialogueSimulation()}
                  onSaveProfileNote={() => void saveProfileNote()}
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
          <OperationKnowledgeView
            busy={busy}
            currentAccountId={currentAccountId}
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
            activeTab={knowledgeTab}
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
            onTab={setKnowledgeTab}
            onTestMessage={setKnowledgeTestMessage}
            onVerifyChunk={(id) => void verifyKnowledgeChunk(id)}
          />
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

        {activeChannel === "quality" && (
          <QualityCenterView accountId={currentAccountId} />
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
  knowledge,
  memoryCandidates,
  memoryDraft,
  messages,
  operatingMemory,
  playbooks,
  profileNote,
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
  onRunMemoryConsolidation,
  onRunSimulation,
  onSaveProfileNote,
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
  onRunMemoryConsolidation: () => void;
  onRunSimulation: () => void;
  onSaveProfileNote: () => void;
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
          <div className="messageList smartMessages">
            {messages.map((message) => (
              <div key={message.id} className={`message ${message.direction}`}>
                <p>{message.content}</p>
                <span>{formatTime(message.createdAt)}</span>
              </div>
            ))}
            {!messages.length && <EmptyInline text="暂无会话记录" />}
          </div>
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
        <table className="dataTable">
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
      )}

      {opsTab === "events" && (
        <table className="dataTable">
          <tbody>
            {events.map((event) => (
              <tr key={event.id}>
                <td>{event.kind}</td>
                <td>{event.summary}</td>
                <td>{event.status}</td>
                <td>{formatTime(event.createdAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {opsTab === "reviews" && (
        <table className="dataTable">
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
          <table className="dataTable">
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
  activeTab,
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
  onTab,
  onTestMessage,
  onVerifyChunk
}: {
  activeTab: KnowledgeTab;
  busy: boolean;
  currentAccountId: string;
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
  onTab: (tab: KnowledgeTab) => void;
  onTestMessage: (value: string) => void;
  onVerifyChunk: (id: string) => void;
}) {
  const activeChunks = chunks.filter((item) => item.status === "active").length;
  const verifiedChunks = chunks.filter((item) => item.integrityStatus === "verified").length;
  const evidenceItems = chunks.filter((item) => item.evidenceItems.length > 0).length;
  const previewCount = (importPreview.document ? 1 : 0) + importPreview.items.length + importPreview.chunks.length;
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
          </div>
        </section>
      )}

      <div className="segmented knowledgeTabs">
        <button className={activeTab === "documents" ? "active" : ""} onClick={() => onTab("documents")}>
          <UploadCloud size={15} /> 文档
        </button>
        <button className={activeTab === "catalog" ? "active" : ""} onClick={() => onTab("catalog")}>
          <Workflow size={15} /> AI 目录
        </button>
        <button className={activeTab === "library" ? "active" : ""} onClick={() => onTab("library")}>
          <LibraryBig size={15} /> 知识包
        </button>
        <button className={activeTab === "chunks" ? "active" : ""} onClick={() => onTab("chunks")}>
          <FileText size={15} /> 切片与证据
        </button>
        <button className={activeTab === "test" ? "active" : ""} onClick={() => onTab("test")}>
          <Search size={15} /> 命中测试
        </button>
        <button className={activeTab === "usage" ? "active" : ""} onClick={() => onTab("usage")}>
          <ShieldCheck size={15} /> 使用日志
        </button>
      </div>

      {activeTab === "documents" && (
        <section className="knowledgeImportGrid">
          <section className="panel assetForm">
            <div className="panelHead">
              <div>
                <span>Text / Markdown</span>
                <h2>导入文档并生成渐进式目录</h2>
              </div>
            </div>
            <label>
              <span>来源名称</span>
              <input value={importSource} onChange={(event) => onImportSource(event.target.value)} />
            </label>
            <label>
              <span>文档内容</span>
              <textarea
                className="largeTextArea"
                value={importText}
                onChange={(event) => onImportText(event.target.value)}
                placeholder="粘贴产品说明、服务边界、FAQ、案例证据或运营 SOP。第一版支持文本和 Markdown。"
              />
            </label>
            <div className="buttonRow">
              <button type="button" onClick={onPreviewImport} disabled={busy || !importText.trim()}>
                AI 生成目录和切片
              </button>
              <button type="button" className="secondary" onClick={onApplyImport} disabled={busy || !previewCount}>
                确认入库
              </button>
            </div>
          </section>
          <section className="panel">
            <div className="panelHead">
              <div>
                <span>Preview</span>
                <h2>AI 结构化预览</h2>
              </div>
            </div>
            <div className="assetList">
              {importPreview.document && (
                <div className="assetRow">
                  <strong>{importPreview.document.title}</strong>
                  <span>文档目录 / {importPreview.document.status}</span>
                  <p>{importPreview.document.catalogSummary || importPreview.document.summary || "已生成文档入口"}</p>
                </div>
              )}
              {importPreview.integrityReport && (
                <div className="assetRow integritySummary">
                  <strong>完整性校验</strong>
                  <span>
                    verified {String(importPreview.integrityReport.verified ?? 0)} / needs review {String(importPreview.integrityReport.needsReview ?? 0)}
                  </span>
                  <p>切片必须能追溯原文。未通过校验的切片入库后默认不会作为事实依据自动启用。</p>
                </div>
              )}
              {importPreview.items.map((item, index) => (
                <button key={`${item.title}-${index}`} className="assetRow selectable" onClick={() => {
                  onNewKnowledge();
                  onKnowledgeDraft(draftFromKnowledge(item));
                  onTab("library");
                }}>
                  <strong>{item.title}</strong>
                  <span>{item.knowledgeType || item.category} / {item.businessContext || item.businessType}</span>
                  <p>{item.routingCard || item.summary || item.safeClaims.join(" / ")}</p>
                </button>
              ))}
              {importPreview.chunks.map((item, index) => (
                <button key={`${item.title}-chunk-${index}`} className="assetRow selectable" onClick={() => {
                  onNewChunk();
                  onChunkDraft(draftFromChunk(item));
                  onTab("chunks");
                }}>
                  <strong>{item.title}</strong>
                  <span>切片 / {item.knowledgeType || "AI 生成"} / {item.evidenceItems.length ? "含证据" : "无证据"}</span>
                  <p>{item.routingCard || item.summary || item.body || "等待确认"}</p>
                </button>
              ))}
              {!previewCount && <EmptyInline text="等待导入预览" />}
            </div>
          </section>
        </section>
      )}

      {activeTab === "catalog" && (
        <section className="knowledgeImportGrid">
          <section className="panel">
            <div className="panelHead">
              <div>
                <span>Documents</span>
                <h2>文档目录</h2>
              </div>
            </div>
            <div className="assetList">
              {documents.map((item) => (
                <div key={item.id} className="assetRow">
                  <strong>{item.title}</strong>
                  <span>{item.sourceName || item.sourceType} / {item.status}</span>
                  <p>{item.catalogSummary || item.summary || item.routingMap.join(" / ")}</p>
                  <div className="buttonRow compactActions">
                    <button type="button" className="secondary compactButton" onClick={() => onDeleteDocument(item.id)} disabled={busy}>删除文档</button>
                  </div>
                </div>
              ))}
              {!documents.length && <EmptyInline text="暂无文档目录" />}
            </div>
          </section>
          <section className="panel">
            <div className="panelHead">
              <div>
                <span>Catalog JSON</span>
                <h2>Agent 可见目录</h2>
              </div>
            </div>
            <pre className="jsonPreview">{JSON.stringify(catalog || {}, null, 2)}</pre>
          </section>
        </section>
      )}

      {activeTab === "library" && (
        <section className="splitWorkspace embedded">
          <section>
            <div className="assetList">
              {knowledge.map((item) => (
                <button
                  key={item.id}
                  className={editingKnowledgeId === item.id ? "assetRow selectable selected" : "assetRow selectable"}
                  onClick={() => onEditKnowledge(item)}
                >
                  <strong>{item.title}</strong>
                  <span>{item.knowledgeType || item.category} / {item.businessContext || item.businessType} / {item.status}</span>
                  <p>{item.routingCard || item.summary || item.safeClaims.join(" / ") || "暂无摘要"}</p>
                </button>
              ))}
              {!knowledge.length && <EmptyInline text="暂无运营知识包" />}
            </div>
          </section>
          <KnowledgeEditor
            busy={busy}
            draft={knowledgeDraft}
            editingId={editingKnowledgeId}
            onCreate={onCreateKnowledge}
            onDelete={onDeleteKnowledge}
            onDraft={onKnowledgeDraft}
            onNew={onNewKnowledge}
            onSave={onSaveKnowledge}
          />
        </section>
      )}

      {activeTab === "chunks" && (
        <section className="splitWorkspace embedded">
          <section>
            <div className="assetList">
              {chunks.map((item) => (
                <button key={item.id} className={editingChunkId === item.id ? "assetRow selectable selected" : "assetRow selectable"} onClick={() => onEditChunk(item)}>
                  <strong>{item.title}</strong>
                  <span>{item.knowledgeType || "切片"} / {integrityStatusLabel(item.integrityStatus)} / {item.status} / {item.evidenceItems.length ? "含证据" : "无证据"}</span>
                  <p>{item.routingCard || item.summary || item.body || "暂无摘要"}</p>
                </button>
              ))}
              {!chunks.length && <EmptyInline text="暂无知识切片" />}
            </div>
          </section>
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
            source={chunkSource}
          />
        </section>
      )}

      {activeTab === "test" && (
        <section className="knowledgeImportGrid">
          <section className="panel assetForm">
            <div className="panelHead">
              <div>
                <span>Knowledge Router</span>
                <h2>命中测试</h2>
              </div>
            </div>
            <label>
              <span>用户消息</span>
              <textarea value={testMessage} onChange={(event) => onTestMessage(event.target.value)} />
            </label>
            <button type="button" onClick={onRunTest} disabled={busy || !testMessage.trim()}>
              运行知识路由
            </button>
          </section>
          <section className="panel">
            <div className="panelHead">
              <div>
                <span>Tool Trace</span>
                <h2>自主查询轨迹</h2>
              </div>
            </div>
            <pre className="jsonPreview">{JSON.stringify(testResult || {}, null, 2)}</pre>
          </section>
        </section>
      )}

      {activeTab === "usage" && (
        <section className="panel">
          <div className="panelHead">
            <div>
              <span>Audit</span>
              <h2>知识使用日志</h2>
            </div>
          </div>
          <div className="assetList">
            {usage.map((item) => (
              <div key={item.id} className="assetRow">
                <strong>{item.reviewApproved ? "Review 通过" : "Review 拦截"} / {formatTime(item.createdAt)}</strong>
                <span>{item.contactWxid || "未绑定联系人"} / {item.knowledgeIds.length} 个知识包</span>
                <p>{item.replyText || item.blockedReason || JSON.stringify(item.routeResult)}</p>
              </div>
            ))}
            {!usage.length && <EmptyInline text="暂无知识使用日志" />}
          </div>
        </section>
      )}

      {integrityReport && (
        <section className="panel integrityDock">
          <div className="panelHead">
            <div>
              <span>Integrity</span>
              <h2>完整性状态</h2>
            </div>
          </div>
          <div className="knowledgeStats compactStats">
            <div><strong>{integrityReport.total}</strong><span>全部切片</span></div>
            <div><strong>{integrityReport.verified}</strong><span>已验证</span></div>
            <div><strong>{integrityReport.needsReview}</strong><span>需复核</span></div>
            <div><strong>{integrityReport.rejected}</strong><span>已拒绝</span></div>
          </div>
        </section>
      )}
    </section>
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
  onSave
}: {
  busy: boolean;
  draft: OperationKnowledgeDraft;
  editingId: string;
  onCreate: (event: FormEvent) => void;
  onDelete: (id: string) => void;
  onDraft: (draft: OperationKnowledgeDraft) => void;
  onNew: () => void;
  onSave: (event: FormEvent) => void;
}) {
  return (
    <form className="assetForm promptEditor" onSubmit={editingId ? onSave : onCreate}>
      <div className="panelHead">
        <div>
          <span>{editingId ? "Edit Knowledge" : "Create Knowledge"}</span>
          <h2>{editingId ? "编辑知识包" : "新增知识包"}</h2>
        </div>
        {editingId && (
          <button type="button" className="secondary compactButton" onClick={onNew}>
            新建
          </button>
        )}
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
          <span>适合客户/阶段</span>
          <textarea value={draft.suitableFor} onChange={(event) => onDraft({ ...draft, suitableFor: event.target.value })} />
        </label>
        <label>
          <span>不适合使用</span>
          <textarea value={draft.notSuitableFor} onChange={(event) => onDraft({ ...draft, notSuitableFor: event.target.value })} />
        </label>
      </div>
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
        {editingId && (
          <button type="button" className="secondary compactButton" onClick={onNew}>
            新建
          </button>
        )}
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
          <textarea value={draft.applicableScenes} onChange={(event) => onDraft({ ...draft, applicableScenes: event.target.value })} />
        </label>
        <label>
          <span>不适用场景</span>
          <textarea value={draft.notApplicableScenes} onChange={(event) => onDraft({ ...draft, notApplicableScenes: event.target.value })} />
        </label>
      </div>
      <div className="formGrid">
        <label>
          <span>安全事实</span>
          <textarea value={draft.safeClaims} onChange={(event) => onDraft({ ...draft, safeClaims: event.target.value })} />
        </label>
        <label>
          <span>禁止承诺</span>
          <textarea value={draft.forbiddenClaims} onChange={(event) => onDraft({ ...draft, forbiddenClaims: event.target.value })} />
        </label>
      </div>
      <label>
        <span>证据</span>
        <textarea value={draft.evidenceItems} onChange={(event) => onDraft({ ...draft, evidenceItems: event.target.value })} />
      </label>
      <label>
        <span>原文引用</span>
        <textarea value={draft.sourceQuote} onChange={(event) => onDraft({ ...draft, sourceQuote: event.target.value })} />
      </label>
      <div className="formGrid">
        <label>
          <span>完整性状态</span>
          <input value={draft.integrityStatus} onChange={(event) => onDraft({ ...draft, integrityStatus: event.target.value })} />
        </label>
        <label>
          <span>置信分</span>
          <input value={draft.confidenceScore} onChange={(event) => onDraft({ ...draft, confidenceScore: event.target.value })} />
        </label>
      </div>
      <div className="formGrid">
        <label>
          <span>已验证事实</span>
          <textarea value={draft.verifiedClaims} onChange={(event) => onDraft({ ...draft, verifiedClaims: event.target.value })} />
        </label>
        <label>
          <span>无依据声明</span>
          <textarea value={draft.unsupportedClaims} onChange={(event) => onDraft({ ...draft, unsupportedClaims: event.target.value })} />
        </label>
      </div>
      <label>
        <span>失真风险</span>
        <textarea value={draft.distortionRisks} onChange={(event) => onDraft({ ...draft, distortionRisks: event.target.value })} />
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
    case "quality":
      return "运营成效";
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
    case "quality":
      return "Outcome & Quality";
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
    case "quality":
      return "用户回复率、对话深度等长期指标，知识切片自动校验，公式遵守度评测，产品声明兜底标记词管理。";
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
    priority: "0"
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
    priority: String(item.priority ?? 0)
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
    priority: Number(draft.priority || 0)
  };
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
  if (!hasStage && !hasCommitments) {
    return null;
  }
  return (
    <section className="cockpitSection" data-testid="planner-view-section">
      <div className="sectionCaption">Planner 视角</div>
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
