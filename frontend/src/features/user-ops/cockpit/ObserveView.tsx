// 观测段独立视图（Task 4 从 CockpitPanel 的 ObserveContent 抽出）。
// 抽文件是搬迁不是重写：JSX 从 CockpitPanel.tsx:185-287 verbatim 搬出，仅两处增强：
//  1) 运营健康度卡改用 cockpit.module.css 的 tone class（health_good/warn/danger），
//     底色引用 tokens.css 的 --fill-running/held/blocked，不再走全局 `healthItem ${tone}` 硬编码。
//  2) 长期记忆卡片加下钻入口（data-testid + onClick → onDrilldown("memory")）。
// 复用的 helper/子组件全部从 ../legacy 导入（canonical 定义所在），不改数据流。
import { SendHorizonal, TrendingUp } from "lucide-react";
import type {
  Contact,
  DecisionReview,
  OperatingMemory,
  OperatingMemoryDraft,
  OperationHealth
} from "../../../types";
import TagTrustPanel from "../TagTrustPanel";
import PersonalityPanel from "../PersonalityPanel";
import {
  MemoryCardSummary,
  PlannerViewSection,
  defaultHealthItems,
  formatTime,
  nextBestActionLabel
} from "../legacy";
import styles from "./cockpit.module.css";

type Drilldown = "memory" | "conversation" | "sendHistory" | "trends";

// 观测段所需 props 子集（从 CockpitPanelProps 抽出，避免 ObserveView 被无关配置项耦合）。
type ObserveViewProps = {
  selected: Contact | null;
  decisionReviews: DecisionReview[];
  memoryDraft: OperatingMemoryDraft;
  health: OperationHealth | null;
  operatingMemory: OperatingMemory | null;
  onSaveManualTags: (tags: string[]) => void;
  onDrilldown: (d: Drilldown) => void;
};

// 健康度 tone → cockpit.module.css tone class（token 化底色）。
const HEALTH_TONE_CLASS: Record<string, string> = {
  good: styles.health_good,
  warn: styles.health_warn,
  danger: styles.health_danger
};

export function ObserveView(props: ObserveViewProps) {
  const { selected, decisionReviews, memoryDraft, health, operatingMemory, onSaveManualTags, onDrilldown } = props;
  if (!selected) return null;
  const latestReview = decisionReviews[0];

  return (
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
        <div className="sectionCaption">标签可信度</div>
        <TagTrustPanel contact={selected} onSaveManualTags={onSaveManualTags} />
      </section>

      <section className="cockpitSection">
        <div className="sectionCaption">人格画像（OCEAN）</div>
        <PersonalityPanel profile={selected.personalityProfile} />
        <div className="buttonRow">
          <button className="secondary" type="button" onClick={() => onDrilldown("trends")}>
            <TrendingUp size={16} />
            查看走势详情
          </button>
        </div>
      </section>

      <section className="cockpitSection">
        <div className="sectionCaption">运营健康度</div>
        <div className="healthGrid compact">
          {(health?.items || defaultHealthItems()).map((item) => (
            <div key={item.key} className={`healthItem ${HEALTH_TONE_CLASS[item.tone] || ""}`}>
              <div>
                <strong>{item.label}</strong>
                <span>{item.score}</span>
              </div>
              <p>{item.detail}</p>
            </div>
          ))}
        </div>
      </section>

      {/* 长期记忆卡片：加下钻入口，点击进 memory 下钻视图（Task 5 填详情）。 */}
      <section className="cockpitSection">
        <div className="sectionCaption">长期记忆卡片</div>
        <button
          type="button"
          className={styles.memoryDrillCard}
          data-testid="observe-memory-card"
          onClick={() => onDrilldown("memory")}
        >
          <MemoryCardSummary memoryCard={operatingMemory?.memoryCard} />
        </button>
      </section>

      <PlannerViewSection contact={selected} />

      {/* 发送历史下钻入口（原内联 SendHistorySection 移入下钻视图）。 */}
      <section className="cockpitSection">
        <div className="sectionCaption">AI 已发送</div>
        <div className="buttonRow">
          <button className="secondary" type="button" onClick={() => onDrilldown("sendHistory")}>
            <SendHorizonal size={16} />
            查看发送历史
          </button>
        </div>
      </section>
    </section>
  );
}
