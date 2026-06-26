// 跨 feature 共享类型。从 App.tsx 抽出，作为单一来源。

export type AgentStatus = "normal" | "managed";
export type Channel =
  | "command"
  | "overview"
  | "userOps"
  | "groupOps"
  | "momentOps"
  | "content"
  | "referralCards"
  | "sendAnalytics"
  | "systemStrategy"
  | "operations"
  | "autonomy"
  | "evolution"
  | "quality"
  | "llmProviders"
  | "knowledgeWiki"
  | "productsDeals"
  | "askHuman"
  | "askHumanConfig";
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

/** 标签可信度改造：单条证据，对话引用（不拷贝原文）。后端 `Evidence`（camelCase）。 */
export type Evidence = { turn: number; msgId: string };
/** AI 确信层标签：压缩归并时整体重判写回，每条必带证据。后端 `ConfirmedTag`。 */
export type ConfirmedTag = { value: string; evidences: Evidence[]; confirmedAt: string; confirmedBy: string };
/** 贝叶斯评估旁路：单轮观测点（append-only ledger）。后端 `BayesianPoint`。 */
export type BayesianPoint = {
  turn: number;
  value: string;
  confidence: number;
  valueChanged: boolean;
  confidenceChanged: boolean;
  reason?: string;
};
/** 贝叶斯评估旁路：一个被追踪的维度槽。后端 `BayesianSignal`。 */
export type BayesianSignal = {
  dimension: string;
  currentValue: string;
  currentConfidence: number;
  locked: boolean;
  history: BayesianPoint[];
};
/** 大五人格单维度：分值 + 证据充分度 + 支撑引用。后端 `PersonalityFacet`。 */
export type PersonalityFacet = { score: number; confidence: number; evidenceRefs: Evidence[] };
/** 人格演化快照：每次压缩归并存一份。后端 `PersonalitySnapshot`。 */
export type PersonalitySnapshot = { consolidatedAt: string; scores: number[]; confidences: number[] };
/** 大五 OCEAN 人格画像：只在压缩归并时更新（慢变量）。后端 `PersonalityProfile`。 */
export type PersonalityProfile = {
  openness: PersonalityFacet;
  conscientiousness: PersonalityFacet;
  extraversion: PersonalityFacet;
  agreeableness: PersonalityFacet;
  neuroticism: PersonalityFacet;
  updatedAt: string;
  snapshots: PersonalitySnapshot[];
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
  /** 标签可信度改造 - 运营录入层：原始运营录入标签（与合并后的 `tags` 区分展示）。 */
  manualTags?: string[];
  /** 标签可信度改造 - AI 确信层：每条带证据，压缩归并时重判。 */
  confirmedTags?: ConfirmedTag[];
  /** 标签可信度改造 - 贝叶斯旁路层：维度槽走势（永不驱动行为）。 */
  bayesianSignals?: BayesianSignal[];
  /** 标签可信度改造 - OCEAN 人格画像（慢变量，压缩归并时更新）。 */
  personalityProfile?: PersonalityProfile;
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
  /** 出站消息类型："text"(默认/缺省) | "media" | "namecard"。
   *  对齐后端 ConversationMessage.msg_type（list_messages 以 camelCase 序列化）。 */
  msgType?: string;
  /** 媒体/名片消息引用的资源 id（media=content_assets._id，namecard=referral_cards._id）。 */
  mediaRef?: string;
};

/** 客户画像页「AI 已发送」只读历史项：对齐后端
 *  GET /api/contacts/:wxid/send-history 的 items 序列化（camelCase）。 */
export type SendHistoryItem = {
  sendKind: "media" | "namecard";
  targetId: string;
  targetTitle: string;
  sentAt?: string;
  triggerReason?: string | null;
  responded?: boolean | null;
  stageAdvanced?: boolean | null;
};

/** 专属顾问名片：对齐后端 ReferralCard 的 list 序列化（camelCase）。 */
export type ReferralCard = {
  id: string;
  workspaceId: string;
  accountId?: string | null;
  targetWxid: string;
  displayName: string;
  sendTriggerHint: string;
  targetStages: string[];
  enabled: boolean;
  reviewStatus: "draft" | "approved";
  reviewNote?: string | null;
  tags?: string[];
  createdAt?: string;
  updatedAt?: string;
};

export type ReferralCardDraft = {
  displayName: string;
  targetWxid: string;
  sendTriggerHint: string;
  targetStages: string;
  tags: string;
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
  sendable?: boolean;
  tags?: string[];
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
  // 后端 FE-1 后返回构建好的 health（scores + canonical 7 项 items），
  // 与正常加载路径 /operation-health 同形态；前端直接消费它。
  // 后端兼容期（旧后端/兼容路径）可能缺失，运行时以 data.item.health && ... 守卫兜底，故声明为可选。
  health?: OperationHealth;
  // 旧 healthScores（scores document）保留以兼容尚未迁移读端；前端不再用它重建 items。
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
  assistModeEnabled?: boolean | null;
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
  assistModeEnabled: boolean;
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

// D7：reviewer 评审取向覆盖。对齐后端 ReviewerOrientation(models.rs:1939)。
export type ReviewerOrientation = {
  reviewFocus?: string | null;
  balancePrinciple?: string | null;
  reviewerFewshotOverride?: string | null;
};

// D7/H17：intent 轨迹维度声明。对齐后端 TrajectoryDimension(models.rs:4067)。
export type TrajectoryDimension = {
  kind: string;
  display_name: string;
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

// H13：AI 生成状态机本体（draft 暂存料，激活前供审阅）。后端 DomainProfile.generated_state_machine
// 是 Option<Document>，**外层字段名 snake_case**（serde 无 rename_all），但**内层 key 保留 camelCase**
// （guide_profile.rs:368-413 显式绕过 normalize_json_keys：states/key/name/goal/initial/allowedFrom/
// allowFromAny/forbidsProactive/advanceSignals/riskRules）。goal/advanceSignals/riskRules 是**逐 state**
// 字段（prompts.rs default_user_operation_state_machine 实证），非顶层。
export type GeneratedState = {
  key?: string;
  name?: string;
  goal?: string;
  initial?: boolean;
  advanceSignals?: string[];
  riskRules?: string[];
};

export type GeneratedStateMachine = {
  states?: GeneratedState[];
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
  grounding_gate_bypass_without_claim?: boolean;
  distrust_self_reported_low_risk?: boolean;
  chunk_roles?: ChunkRole[];
  memory_dimensions?: MemoryDimension[];
  outcome_polarity?: OutcomePolarity;
  operation_mode?: OperationMode;
  transaction_facts_enabled?: boolean;
  reviewer_orientation?: ReviewerOrientation | null;
  mode_gate_policy_override?: string | null;
  trajectory_dimensions?: TrajectoryDimension[];
  debounce_window_ms_override?: number | null;
  // H13：AI 生成状态机本体（draft），激活前供审阅；激活后运行时读 operation_domain_configs。
  generated_state_machine?: GeneratedStateMachine | null;
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
  grounding_gate_bypass_without_claim?: boolean;
  distrust_self_reported_low_risk?: boolean;
  chunk_roles?: ChunkRole[];
  memory_dimensions?: MemoryDimension[];
  outcome_polarity?: OutcomePolarity;
  operation_mode?: OperationMode;
  transaction_facts_enabled?: boolean;
  reviewer_orientation?: ReviewerOrientation;
  mode_gate_policy_override?: string;
  trajectory_dimensions?: TrajectoryDimension[];
  debounce_window_ms_override?: number;
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

// 请示通道策略（对齐后端 models.rs AskHumanPolicy，camelCase serde）。P3 配置页 + P2 收件箱共用。
export type DeciderRef = {
  wxid: string;
  displayName?: string;
};

export type AskHumanQuietHours = {
  startHour: number;   // 0-23
  endHour: number;     // 0-23
  tzOffsetHours: number;
};

export type AskHumanPolicy = {
  deciderChain: DeciderRef[];
  escalateSafetyGuard: boolean;
  escalateUnverifiedProduct: boolean;
  escalateAiPolicyHold: boolean;
  escalateStuck: boolean;
  dedupeWindowHours?: number;
  dailyPushCap?: number;
  quietHours?: AskHumanQuietHours;
  timeoutHours?: number;
};
