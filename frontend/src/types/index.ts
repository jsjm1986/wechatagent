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
  | "knowledgeWiki"
  | "productsDeals";
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
  msgType?: "text" | "media";
  mediaRef?: string;
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
  // 销售素材文件字段
  mediaType?: "image" | "file" | "video";
  fileName?: string;
  fileSize?: number;
  mimeType?: string;
  sendTriggerHint?: string;
  targetStages?: string[];
  expressionPref?: "file_primary" | "file_support";
  requiresPrincipalApproval?: boolean;
  reviewStatus?: "draft" | "approved";
  reviewNote?: string;
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

// ── DomainProfile（行业配置）────────────────────────────────────────────────
// 后端 DomainProfile 用 serde_json::to_value 序列化 → snake_case JSON。

export type ProfileDimension = {
  kind: string;
  display_name: string;
  participates_in_decision: boolean;
  description: string;
};

export type BusinessFormula = {
  key: string;
  expression: string;
  display_name: string;
  // 后端 Option<String>：映射到 reviewer/evaluations 的 score key（#7 漂移护栏锁此字段）。
  // 编辑现有公式时须保留，避免对象覆盖丢字段。
  eval_score_key?: string | null;
};

export type CommitmentMarkers = {
  product_effect: string[];
  tone_only: string[];
};

export type CoverageDimension = {
  key: string;
  display_name: string;
  required: boolean;
  anchor_hint?: string | null;
};

// H16 知识切片用途角色。对齐后端 ChunkRole。
export type ChunkRole = {
  key: string;
  header: string;
  order: number;
  is_fallback: boolean;
};

// H17 memoryCard 记忆维度。对齐后端 MemoryDimension。
export type MemoryDimension = {
  key: string;
  display_name: string;
  cap: number;
  is_core: boolean;
  prompt_hint?: string | null;
  candidate_type: boolean;
};

// H11 自学习极性。对齐后端 OutcomePolarity。
export type OutcomePolarity = {
  positive: string[];
  negative: string[];
};

// H8/H19 运营范式（三驱动力开关 + 阈值 + 作息门控）。对齐后端 OperationMode。
export type FunnelMode = {
  enabled: boolean;
  stagnation_threshold_days?: number | null;
};
export type SilenceMode = {
  enabled: boolean;
  threshold_hours?: number | null;
};
export type CommitmentMode = {
  enabled: boolean;
  imminent_window_hours?: number | null;
};
export type QuietHoursMode = {
  enabled_override?: boolean | null;
};
export type OperationMode = {
  funnel: FunnelMode;
  silence: SilenceMode;
  commitment: CommitmentMode;
  quiet_hours: QuietHoursMode;
};

// 五闸阈值覆盖。对齐后端 ProfileThresholds（#[serde(rename_all = "camelCase")]）。
// 字段为 undefined/缺省 = 不覆盖该闸，沿用该域默认（销售域 6/7/6/6/7）。
export type ProfileThresholds = {
  factRiskBlockAt?: number | null;
  pressureRiskBlockAt?: number | null;
  humanLikeRewriteBelow?: number | null;
  emotionalValueRewriteBelow?: number | null;
  productAccuracyBlockBelow?: number | null;
};

export type DomainProfile = {
  id: string;
  profile_id: string;
  workspace_id: string;
  display_name: string;
  description: string;
  profile_dimensions: ProfileDimension[];
  prompt_fragment: string;
  conversation_modes: string[];
  business_formulas: BusinessFormula[];
  commitment_markers: CommitmentMarkers;
  coverage_dimensions: CoverageDimension[];
  threshold_overrides?: ProfileThresholds | null;
  // universal-domain-adaptation 增量字段（H12/H14/H16/H17/H11/H8 等）。
  soul_override?: string | null;
  methodology_override?: string | null;
  conversation_mode_policy?: string | null;
  methodology_generator_preamble?: string | null;
  stagnation_dimension?: string | null;
  domain_schema_id?: string | null;
  grounding_gate_bypass_without_claim?: boolean;
  distrust_self_reported_low_risk?: boolean;
  chunk_roles?: ChunkRole[];
  memory_dimensions?: MemoryDimension[];
  outcome_polarity?: OutcomePolarity;
  operation_mode?: OperationMode;
  version: number;
  current_version: boolean;
  previous_version: number | null;
  is_active: boolean;
  seeded_by: string | null;
  created_at?: string;
  updated_at?: string;
};

export type DomainProfileDraft = {
  profile_id?: string;
  display_name?: string;
  description?: string;
  profile_dimensions?: ProfileDimension[];
  prompt_fragment?: string;
  conversation_modes?: string[];
  business_formulas?: BusinessFormula[];
  commitment_markers?: CommitmentMarkers;
  coverage_dimensions?: CoverageDimension[];
  threshold_overrides?: ProfileThresholds;
  methodology_generator_preamble?: string;
  soul_override?: string;
  methodology_override?: string;
  conversation_mode_policy?: string;
  stagnation_dimension?: string;
  domain_schema_id?: string;
  grounding_gate_bypass_without_claim?: boolean;
  distrust_self_reported_low_risk?: boolean;
  chunk_roles?: ChunkRole[];
  memory_dimensions?: MemoryDimension[];
  outcome_polarity?: OutcomePolarity;
  operation_mode?: OperationMode;
};

export type GenerateProfileRequest = {
  businessDescription: string;
  profileId: string;
  displayName?: string;
};

export type GenerateProfileResponse = {
  ok: boolean;
  id: string;
  profileId: string;
  status: string;
  note: string;
};
