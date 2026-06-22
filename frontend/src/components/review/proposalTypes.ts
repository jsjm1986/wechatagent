// Ask-Human Phase 2 Task 6（零跨feature修订）：演化候选领域类型的中立家。
// 原本定义在 features/evolution/EvolutionCenterTab.tsx；为让 ProposalReleaseCard 与统一收件箱
// 频道都能复用而不反向依赖 features/evolution，类型定义物理迁出到本中立模块。
// 老页（EvolutionCenterTab）与卡片都从这里 import；定义逐字保留，镜像后端 src/routes/evolution.rs 返回 schema。

export type ProposalStatus =
  | "pending_eval"
  | "evaluating"
  | "eligible_for_release"
  | "rejected_below_threshold"
  | "released"
  | "rolled_back";

export type ProposalKind = "threshold" | "prompt";

export interface ExperimentEnvelope {
  experimentId: string;
  workspaceId: string;
  accountId: string;
  status: string;
  windowHours: number;
  startedAt: string;
  updatedAt: string;
  finishedAt: string | null;
  cohortThresholdSize: number;
  cohortPromptSize: number;
  budgetUsedTokens: number;
  budgetUsedCalls: number;
  proposalsCount: number;
  proposalsEligibleCount: number;
}

export interface ProposalSummary {
  id: string | null;
  kind: ProposalKind;
  status: ProposalStatus | string;
  gateKey: string | null;
  proposedTemplateKey: string | null;
  proposedSection: string | null;
  currentValue: number | null;
  proposedValue: number | null;
  significancePassed: boolean | null;
  evalReplaysCompleted: number | null;
  evalReplaysFailed: number | null;
  failureReason: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ExperimentItem {
  experiment: ExperimentEnvelope;
  proposalsCounts: Record<string, number>;
  proposals: ProposalSummary[];
}

export interface ExperimentsResponse {
  items: ExperimentItem[];
}

export interface ShadowReplaySample {
  id: string | null;
  sourceRunId: string;
  status: string;
  failureReason: string | null;
  originalFinalReviewStatus: string | null;
  newFinalReviewStatus: string | null;
  newReviewRisks: unknown;
  newTokenCost: number | null;
  new5gateHit: Record<string, unknown>;
  newSelfCritiqueAddressed: boolean | null;
  similarityToOriginalText: number | null;
  startedAt: string;
  finishedAt: string | null;
}

export interface ShadowReplaysSummary {
  totalCompleted: number;
  totalFailed: number;
  samples: ShadowReplaySample[];
}

export interface ProposalDetail {
  id: string | null;
  experimentId: string;
  workspaceId: string;
  accountId: string;
  kind: ProposalKind;
  status: ProposalStatus | string;
  gateKey: string | null;
  currentValue: number | null;
  proposedValue: number | null;
  cohortNotes: Record<string, unknown>;
  proposedTemplateKey: string | null;
  proposedSection: string | null;
  diffSummary: string | null;
  diffSnippet: string | null;
  criticReasoning: string | null;
  expectedImprovementOn: string[] | null;
  riskNote: string | null;
  previousPromptVersion: string | null;
  evalMetrics: Record<string, unknown>;
  evalReplaysCompleted: number | null;
  evalReplaysFailed: number | null;
  significancePassed: boolean | null;
  failureReason: string | null;
  releasedAt: string | null;
  releasedBy: string | null;
  rolledBackAt: string | null;
  rolledBackBy: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProposalDetailResponse {
  proposal: ProposalDetail;
  experiment: ExperimentEnvelope | null;
  cohortRunIds: string[];
  shadowReplays: ShadowReplaysSummary;
  currentState: Record<string, unknown>;
}
