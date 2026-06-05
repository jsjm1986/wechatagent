// LP-frontend-refactor：演化中心 Tab 已迁入 features/evolution/，落地 CSS Modules + 新视觉。
// 此处保留 re-export，使既有单测 import 路径 `../EvolutionCenterTab` 与外部引用不受影响。
export {
  EvolutionCenterTab,
  ConfirmModal,
  StatusBadge,
  statusLabel,
  statusTone,
  formatNumber,
  formatPercent,
  aggregateLast7Days,
  type ProposalStatus,
  type ProposalKind,
  type ExperimentEnvelope,
  type ProposalSummary,
  type ExperimentItem,
  type ExperimentsResponse,
  type ShadowReplaySample,
  type ShadowReplaysSummary,
  type ProposalDetail,
  type ProposalDetailResponse,
} from "./features/evolution/EvolutionCenterTab";
