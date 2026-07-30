// 用户运营驾驶舱三段式外壳（Task 2 脚手架 + 行为等价迁移）。
// 段控替代原 SmartOpsTabs：观测（只读判断）/ 配置（可编辑）+ 下钻视图（会话/发送历史/记忆）。
// 本 Task 只做"结构搬家 + 段控骨架"，把现 6 个 activeTab 块的 JSX 原样搬进
// ObserveContent / ConfigureContent / DrilldownHost 三个同文件临时函数，不改数据流。
// 复用的 helper/子组件全部从 ./legacy 导入（那里是它们的 canonical 定义）。
import { useState } from "react";
import { UserRoundCheck } from "lucide-react";
import type {
  Contact,
  DecisionReview,
  MemoryCandidateItem,
  OperatingMemory,
  OperatingMemoryDraft,
  OperationHealth,
  OperationPlaybook,
  UserOperationGuidePreview,
  Message,
  SimulationTurn
} from "../../../types";
import { useProfileStore } from "../../../stores/profileStore";
import { useUserOpsStore } from "../../../stores/userOpsStore";
import { useNavigationStore } from "../../../stores/navigationStore";
import { JudgmentBar } from "./JudgmentBar";
import { ObserveView } from "./ObserveView";
import { ConfigureView } from "./ConfigureView";
import { MemoryDetailView } from "./drilldowns/MemoryDetailView";
import { ConversationReviewView } from "./drilldowns/ConversationReviewView";
import { SendHistoryView } from "./drilldowns/SendHistoryView";
import { TrendsDetailView } from "./drilldowns/TrendsDetailView";
import { PlanStep } from "../legacy";
import styles from "./cockpit.module.css";

type ViewMode = "observe" | "configure";
type Drilldown = null | "memory" | "conversation" | "sendHistory" | "trends";

// CockpitPanel 接收与原 UserOperationCockpit 完全相同的 props，唯独去掉
// activeTab / onTab（段控 + 下钻自管本地 state，不再由外部 tab 驱动）。
// export 供 ConfigureView / DrilldownHost 复用同一 props 契约（Task 6）。
export type CockpitPanelProps = {
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
  assistOverride: string;
  relationshipType: string;
  referredSpecialistAt?: string;
  profileEditDraft: { lastCommitment?: string; followUpPolicy?: string };
  selected: Contact | null;
  selectedPlaybookId: string;
  simulationBusy: boolean;
  simulationInput: string;
  simulationTurns: SimulationTurn[];
  onAnalyzeProfile: () => void;
  onApplyGuidePreview: (confirmGlobalImpact?: boolean) => void;
  onDisableAgent: () => void;
  onEnableAgent: () => void;
  onGuideInstruction: (value: string) => void;
  onPreviewGuide: (instruction: string) => void;
  onProfileNote: (value: string) => void;
  onCustomAgentInstructions: (value: string) => void;
  onAssistOverride: (mode: string) => void;
  onRelationshipType: (value: string) => void;
  onProfileEditDraftChange: (patch: Partial<{ lastCommitment: string; followUpPolicy: string }>) => void;
  onRunMemoryConsolidation: () => void;
  onRunSimulation: () => void;
  onSaveProfileNote: () => void;
  onSaveCustomAgentInstructions: () => void;
  onSaveAssistOverride: () => void;
  onSaveRelationshipType: () => void;
  onSaveManualTags: (contact: Contact, tags: string[]) => void;
  onMemoryDraftChange: (patch: Partial<OperatingMemoryDraft>) => void;
  onSaveOperatingMemory: () => void;
  onSelectedPlaybook: (value: string) => void;
  onSimulationInput: (value: string) => void;
};

export function CockpitPanel(props: CockpitPanelProps) {
  const { selected } = props;
  const [viewMode, setViewMode] = useState<ViewMode>("observe");
  const [drilldown, setDrilldown] = useState<Drilldown>(null);
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const escalationPendingCount = useUserOpsStore((s) => s.escalationPendingCount);
  const setChannel = useNavigationStore((s) => s.setChannel);

  // 空态：搬自原 UserOperationCockpit（legacy.tsx:282-292）。
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

  return (
    <section className={`smartWorkspace panel ${styles.cockpitPanel}`}>
      {/* panelHead：搬自 legacy.tsx:305-314。JudgmentBar 常驻判断条（Task 3）挂在头部之下。 */}
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

      <JudgmentBar
        contact={selected}
        latestReview={props.decisionReviews[0]}
        health={props.health}
        escalationCount={escalationPendingCount}
        taxonomies={taxonomies}
        onRiskClick={() => {
          setDrilldown(null);
          setViewMode("observe");
        }}
        onEscalationClick={() => setChannel("askHuman")}
      />

      {drilldown === null ? (
        <>
          <div className={styles.segmented} role="tablist" aria-label="驾驶舱视图">
            <button
              role="tab"
              aria-selected={viewMode === "observe"}
              className={viewMode === "observe" ? styles.segActive : styles.seg}
              onClick={() => setViewMode("observe")}
            >
              观测
            </button>
            <button
              role="tab"
              aria-selected={viewMode === "configure"}
              className={viewMode === "configure" ? styles.segActive : styles.seg}
              onClick={() => setViewMode("configure")}
            >
              配置
            </button>
          </div>
          {viewMode === "observe" && <ObserveView {...props} onDrilldown={setDrilldown} />}
          {viewMode === "configure" && <ConfigureView {...props} />}
        </>
      ) : (
        <DrilldownHost drilldown={drilldown} onBack={() => setDrilldown(null)} {...props} />
      )}
    </section>
  );
}

// 观测段：抽出为独立文件 ./ObserveView（Task 4）。健康度 tone 三色改用 cockpit.module.css
// token 化 class；长期记忆卡加下钻入口。CockpitPanel 只保留调用点。

// 下钻视图：conversation → ConversationReviewView（会话流 + 复盘展开自治判断依据）；
// sendHistory → SendHistoryView（复用 SendHistorySection）；memory → MemoryDetailView（记忆溯源全景）。
// 三视图各自带下钻头部 + 返回按钮，onBack 统一回到 drilldown=null。
function DrilldownHost(props: CockpitPanelProps & { drilldown: Exclude<Drilldown, null>; onBack: () => void }) {
  const { drilldown, onBack, selected, messages, decisionReviews, operatingMemory } = props;
  if (!selected) return null;

  if (drilldown === "conversation") {
    return (
      <ConversationReviewView messages={messages} decisionReviews={decisionReviews} onBack={onBack} />
    );
  }

  if (drilldown === "sendHistory") {
    return <SendHistoryView wxid={selected.wxid} onBack={onBack} />;
  }

  if (drilldown === "trends") {
    return <TrendsDetailView contact={selected} onBack={onBack} />;
  }

  return <MemoryDetailView memoryCard={operatingMemory?.memoryCard} onBack={onBack} />;
}
