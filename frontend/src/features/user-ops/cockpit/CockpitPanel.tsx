// 用户运营驾驶舱三段式外壳（Task 2 脚手架 + 行为等价迁移）。
// 段控替代原 SmartOpsTabs：观测（只读判断）/ 配置（可编辑）+ 下钻视图（会话/发送历史/记忆）。
// 本 Task 只做"结构搬家 + 段控骨架"，把现 6 个 activeTab 块的 JSX 原样搬进
// ObserveContent / ConfigureContent / DrilldownHost 三个同文件临时函数，不改数据流。
// 复用的 helper/子组件全部从 ./legacy 导入（那里是它们的 canonical 定义）。
import { useState } from "react";
import {
  Activity,
  ArrowLeft,
  Bot,
  SendHorizonal,
  Sparkles,
  SquarePen,
  UserRoundCheck
} from "lucide-react";
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
import TagTrustPanel from "../TagTrustPanel";
import PersonalityPanel from "../PersonalityPanel";
import {
  MEMORY_DRAFT_FIELD_GROUPS,
  ChangePreview,
  ConversationStream,
  EmptyInline,
  MemoryCardSummary,
  PlanStep,
  PlannerViewSection,
  SendHistorySection,
  SimulationResult,
  defaultHealthItems,
  formatTime,
  impactScopeLabel,
  memoryCandidateText,
  memoryStatusLabel,
  nextBestActionLabel
} from "../legacy";
import styles from "./cockpit.module.css";

type ViewMode = "observe" | "configure";
type Drilldown = null | "memory" | "conversation" | "sendHistory";

// CockpitPanel 接收与原 UserOperationCockpit 完全相同的 props，唯独去掉
// activeTab / onTab（段控 + 下钻自管本地 state，不再由外部 tab 驱动）。
type CockpitPanelProps = {
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
  onApplyGuidePreview: () => void;
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
  onSaveManualTags: (tags: string[]) => void;
  onMemoryDraftChange: (patch: Partial<OperatingMemoryDraft>) => void;
  onSaveOperatingMemory: () => void;
  onSelectedPlaybook: (value: string) => void;
  onSimulationInput: (value: string) => void;
};

export function CockpitPanel(props: CockpitPanelProps) {
  const { selected } = props;
  const [viewMode, setViewMode] = useState<ViewMode>("observe");
  const [drilldown, setDrilldown] = useState<Drilldown>(null);

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
      {/* panelHead：搬自 legacy.tsx:305-314。JudgmentBar 常驻判断条留待 Task 3 填充。 */}
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
          {viewMode === "observe" && <ObserveContent {...props} onDrilldown={setDrilldown} />}
          {viewMode === "configure" && <ConfigureContent {...props} />}
        </>
      ) : (
        <DrilldownHost drilldown={drilldown} onBack={() => setDrilldown(null)} {...props} />
      )}
    </section>
  );
}

// 观测段：搬自原 cockpit 块（legacy.tsx:318-440），去掉运营记忆编辑器（移入配置段），
// 发送历史改为下钻入口（原 SendHistorySection 内联 → 点击进 sendHistory 下钻）。
function ObserveContent(props: CockpitPanelProps & { onDrilldown: (d: Drilldown) => void }) {
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

      {/* 发送历史改为下钻入口（原内联 SendHistorySection 移入下钻视图）。 */}
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

// 配置段：搬自 adjust（442-488）+ profile（490-639）+ memory 编辑部分（641-671）
// + simulation（673-696）+ 运营记忆编辑器（从 cockpit 块 339-372 移来）。
// 用各原始块自带的小标题分组，不引入第三层 tab。
function ConfigureContent(props: CockpitPanelProps) {
  const {
    busy,
    guideBusy,
    guideInstruction,
    guidePreview,
    memoryCandidates,
    memoryDraft,
    operatingMemory,
    playbooks,
    profileNote,
    customAgentInstructions,
    assistOverride,
    relationshipType,
    referredSpecialistAt,
    profileEditDraft,
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
    onAssistOverride,
    onRelationshipType,
    onProfileEditDraftChange,
    onRunMemoryConsolidation,
    onRunSimulation,
    onSaveProfileNote,
    onSaveCustomAgentInstructions,
    onSaveAssistOverride,
    onSaveRelationshipType,
    onMemoryDraftChange,
    onSaveOperatingMemory,
    onSelectedPlaybook,
    onSimulationInput
  } = props;
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const relationshipOptions = taxonomies.relationship_type ?? [];
  const clearReferral = useUserOpsStore((s) => s.clearReferral);
  if (!selected) return null;
  const currentPlaybook = playbooks.find((playbook) => playbook.id === selectedPlaybookId) || playbooks.find((playbook) => playbook.isDefault);
  const examples = [
    "更像朋友一点，自然一些",
    "这个用户比较忙，降低主动打扰频率",
    "用户已经有明确需求，可以更积极推进下一步",
    "重新分析画像，并补充不能踩的沟通禁忌"
  ];

  return (
    <>
      {/* 运营记忆编辑器：从 cockpit 块（legacy.tsx:339-372）移来。 */}
      <section className="smartTabPanel profileEditor">
        <div className="sectionCaption">运营记忆（运营可编辑，保存后影响 Agent 决策）</div>
        {MEMORY_DRAFT_FIELD_GROUPS.map((group) => (
          <div key={group.caption} className="memoryEditGroup">
            <div className="modeLine editable">{group.caption}</div>
            {group.fields.map((field) => (
              <label key={field.key}>
                <span>{field.label}</span>
                {field.multiline ? (
                  <textarea
                    value={memoryDraft[field.key]}
                    rows={2}
                    placeholder={field.placeholder}
                    onChange={(event) => onMemoryDraftChange({ [field.key]: event.target.value })}
                  />
                ) : (
                  <input
                    type="text"
                    value={memoryDraft[field.key]}
                    placeholder={field.placeholder}
                    onChange={(event) => onMemoryDraftChange({ [field.key]: event.target.value })}
                  />
                )}
              </label>
            ))}
          </div>
        ))}
        <div className="buttonRow">
          <button onClick={onSaveOperatingMemory} disabled={busy} type="button">
            <SquarePen size={16} />
            保存运营记忆
          </button>
        </div>
      </section>

      {/* AI 调整：搬自 adjust 块（legacy.tsx:442-488）。 */}
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

      {/* 用户画像：搬自 profile 块（legacy.tsx:490-639）。 */}
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
          <span>运营人员特别指令（最高优先级，可空 — 也可描述关系与口吻）</span>
          <textarea
            value={customAgentInstructions}
            maxLength={1000}
            rows={5}
            onChange={(event) => onCustomAgentInstructions(event.target.value)}
            placeholder="例①：这个客户已签约老客户，不要主动推销，只服务问题。例②：这是我大学同学，他公司可能采购我们产品，但别推销，先用轻松口吻维护关系。Agent 将在每轮对话最末尾读取这段指令，可覆盖默认人格口吻。"
          />
          <span className="counter">{customAgentInstructions.length} / 1000</span>
          {selected.agentStatus === "managed" && (
            <button className="secondary" onClick={onSaveCustomAgentInstructions} disabled={busy} type="button">
              <SquarePen size={16} />
              保存特别指令
            </button>
          )}
        </label>
        {selected.agentStatus === "managed" && (
          <label>
            <span>辅助模式（本客户）</span>
            <small>覆盖账号级默认：跟随账号 / 强制为本客户引荐专属顾问 / 强制不引荐。</small>
            <select
              value={assistOverride}
              onChange={(event) => onAssistOverride(event.target.value)}
            >
              <option value="default">跟随账号默认</option>
              <option value="force_on">强制开启引荐</option>
              <option value="force_off">强制关闭引荐</option>
            </select>
            {referredSpecialistAt && (
              <div className="modeLine editable">
                已引荐 · AI 已退辅助答疑
                {`（${formatTime(referredSpecialistAt)}）`}
              </div>
            )}
            <button className="secondary" onClick={onSaveAssistOverride} disabled={busy} type="button">
              <SquarePen size={16} />
              保存辅助模式
            </button>
            <button
              className="secondary"
              onClick={() => void clearReferral(selected.id)}
              disabled={busy}
              type="button"
            >
              <UserRoundCheck size={16} />
              撤销引荐 / 恢复主动运营
            </button>
          </label>
        )}
        <label>
          <span>客户类型</span>
          <small>影响 AI 的主动触达策略（如：朋友不主动追单、销售对象继续跟进）。</small>
          <select
            value={relationshipType}
            onChange={(event) => onRelationshipType(event.target.value)}
          >
            <option value="">未分类</option>
            {relationshipOptions.length > 0 ? (
              relationshipOptions.map((opt) => (
                <option key={opt.id} value={opt.id}>{opt.label}</option>
              ))
            ) : (
              // 字典未配回落：保留原写死三项，避免下拉只剩"未分类"不可选（渐进降级）
              <>
                <option value="customer">客户（销售型）</option>
                <option value="peer">同行</option>
                <option value="friend">朋友</option>
              </>
            )}
          </select>
          <button className="secondary" onClick={onSaveRelationshipType} disabled={busy} type="button">
            <SquarePen size={16} />
            保存客户类型
          </button>
        </label>
        <label>
          <span>最近承诺（last_commitment）</span>
          <small>运营可编辑：记录对客户作出的最近一条承诺，影响 AI 跟进话术。</small>
          <textarea
            rows={2}
            value={profileEditDraft.lastCommitment ?? ""}
            onChange={(event) => onProfileEditDraftChange({ lastCommitment: event.target.value })}
            placeholder="例：本周内给到方案报价"
          />
        </label>
        <label>
          <span>跟进策略（follow_up_policy）</span>
          <small>运营可编辑：约定主动跟进的节奏/边界，影响 AI 触达频率。</small>
          <textarea
            rows={2}
            value={profileEditDraft.followUpPolicy ?? ""}
            onChange={(event) => onProfileEditDraftChange({ followUpPolicy: event.target.value })}
            placeholder="例：每周跟进一次，客户明确拒绝则停止"
          />
          <small>客户阶段 / 意向等级由 AI 派生，前端只读，此处不编辑。</small>
          <button className="secondary" onClick={onSaveRelationshipType} disabled={busy} type="button">
            <SquarePen size={16} />
            保存运营画像
          </button>
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

      {/* 长期记忆候选：搬自 memory 块（legacy.tsx:641-671）。 */}
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

      {/* 模拟验证：搬自 simulation 块（legacy.tsx:673-696）。 */}
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
    </>
  );
}

// 下钻视图：conversation 分支搬自 conversation 块（legacy.tsx:698-714）；
// sendHistory 分支复用 SendHistorySection；memory 分支 Task 5 做，先占位 + 返回按钮。
function DrilldownHost(props: CockpitPanelProps & { drilldown: Exclude<Drilldown, null>; onBack: () => void }) {
  const { drilldown, onBack, selected, messages, decisionReviews } = props;
  if (!selected) return null;

  const title = drilldown === "conversation" ? "会话记录" : drilldown === "sendHistory" ? "AI 已发送" : "长期记忆";

  return (
    <section className="smartTabPanel">
      <div className={styles.drilldownHead}>
        <button className={styles.backButton} type="button" onClick={onBack}>
          <ArrowLeft size={15} />
          返回
        </button>
        <strong>{title}</strong>
      </div>

      {drilldown === "conversation" && (
        <section className="conversationGrid">
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

      {drilldown === "sendHistory" && <SendHistorySection wxid={selected.wxid} />}

      {drilldown === "memory" && (
        <section className="cockpitSection">
          <EmptyInline text="长期记忆下钻详情将在后续版本提供。" />
        </section>
      )}
    </section>
  );
}
