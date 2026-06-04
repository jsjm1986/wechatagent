// 跨 feature 共享类型。从 App.tsx 抽出，作为单一来源。

export type AgentStatus = "normal" | "managed";
export type Channel =
  | "command"
  | "overview"
  | "userOps"
  | "groupOps"
  | "momentOps"
  | "content"
  | "systemStrategy"
  | "operations"
  | "autonomy"
  | "evolution"
  | "quality"
  | "llmProviders"
  | "knowledgeWiki";
export type ContactTab = "all" | "managed" | "normal";
export type SmartOpsTab = "cockpit" | "adjust" | "profile" | "memory" | "simulation" | "conversation";
export type TraditionalOpsTab = "playbooks" | "prompts" | "settings" | "audit";
export type UserOpsMode = "smart" | "traditional";
export type OpsTab = "tasks" | "events" | "reviews" | "llm";

export type Account = {
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

export type AgentProfile = {
  summary: string;
  interests: string[];
  communicationStyle: string;
  operationGoal: string;
};

export type Contact = {
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
  domainAttributes?: Record<string, unknown>;
  domainAttributesUpdatedAt?: string | null;
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
export type ContactCommitment = {
  id?: string;
  text: string;
  dueAt?: string | null;
  createdAt?: string | null;
};

export type Message = {
  id: string;
  direction: "inbound" | "outbound";
  content: string;
  createdAt?: string;
};

export type EventItem = {
  id: string;
  contactWxid?: string;
  kind: string;
  status: string;
  summary: string;
  createdAt?: string;
};

export type TaskItem = {
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

export type ContentAsset = {
  id: string;
  kind: string;
  title: string;
  body?: string;
  url?: string;
  mediaId?: string;
  usageScene?: string;
};

export type AgentSoul = {
  id: string;
  agentKind: string;
  name: string;
  content: string;
  status: string;
  version: number;
};

export type CommandToolCall = {
  id: string;
  toolName: string;
  arguments?: Record<string, unknown>;
  status: string;
  response?: Record<string, unknown>;
  error?: string;
};

export type CommandResult = {
  id: string;
  status: string;
  summary: string;
  toolCalls: CommandToolCall[];
};

export type LlmUsageItem = {
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

export type LlmUsageResponse = {
  summary: {
    totalCalls: number;
    totalTokens: number;
    promptCacheHitTokens: number;
    promptCacheMissTokens: number;
    promptCacheHitRate: number;
  };
  items: LlmUsageItem[];
};

export type DecisionReview = {
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

export type PromptTemplate = {
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

export type PromptTemplateDraft = {
  promptKey: string;
  agentKind: string;
  layer: string;
  title: string;
  description: string;
  content: string;
};

export type OperationPlaybook = {
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

export type PlaybookDraft = {
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

export type OperatingMemory = {
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

export type MemoryCandidateItem = {
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

export type OperatingMemoryDraft = {
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

export type OperationHealthItem = {
  key: string;
  label: string;
  score: number;
  tone: "good" | "warn" | "danger";
  detail: string;
};

export type OperationHealth = {
  scores: Record<string, number>;
  items: OperationHealthItem[];
};

export type UserOperationGuidePreview = {
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

export type SimulationTurn = {
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

export type DomainKey = "user_operations" | "group_operations" | "moment_operations";

export type OperationDomainConfig = {
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
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  seededBy?: string | null;
};

export type OperationDomainDraft = {
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
