// 用户运营频道私有的视图/辅助实现，从 App.tsx 整体迁出（LP 前端重构）。
// index.tsx 只编排，这里是 7 个根组件 + 其传递闭包内的本地 helper/type。
// 视觉沿用既有 styles.css 类名；token 化由后续 *.module.css 阶段承接。
import {
  Activity,
  Bot,
  BrainCircuit,
  CheckCircle2,
  Clock3,
  Inbox,
  MessageSquareText,
  Search,
  SendHorizonal,
  ShieldCheck,
  Sparkles,
  SquarePen,
  UserRoundCheck,
  User2,
  Workflow
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { FormEvent, useState } from "react";
import type * as React from "react";
import type {
  ContactTab,
  SmartOpsTab,
  TraditionalOpsTab,
  UserOpsMode,
  Contact,
  Message,
  AgentSoul,
  DecisionReview,
  PromptTemplate,
  PromptTemplateDraft,
  OperationPlaybook,
  PlaybookDraft,
  OperatingMemory,
  MemoryCandidateItem,
  OperatingMemoryDraft,
  OperationHealthItem,
  OperationHealth,
  UserOperationGuidePreview,
  SimulationTurn,
  OperationDomainConfig,
  OperationDomainDraft
} from "../../types";
import { api } from "../../lib/api";

type ActiveVersionMeta = {
  id: string;
  version?: number;
  currentVersion?: boolean;
  previousVersion?: number | null;
  seededBy?: string | null;
  updatedAt?: string;
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
  { key: "hallucinationBlockAt", label: "幻觉风险拦截线", detail: "幻觉风险达到该分值则禁止发送", kind: "number", defaultValue: 6 },
  { key: "knowledgeGroundingBlockBelow", label: "知识落地拦截线", detail: "低于该分值则禁止发送涉及产品/价格/政策的内容", kind: "number", defaultValue: 7 },
  { key: "humanLikeRewriteBelow", label: "真人感重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 6 },
  { key: "emotionalValueRewriteBelow", label: "情绪价值重写线", detail: "低于该分值时要求重写", kind: "number", defaultValue: 5 },
  { key: "operationStateConfidenceFullReviewBelow", label: "状态置信 Review 线", detail: "低于该分值强制完整 Review", kind: "number", defaultValue: 4 },
  { key: "runTokenBudget", label: "单次 Token 预算", detail: "单次用户运营运行的最大 token", kind: "number", defaultValue: 30000 },
  { key: "runMaxLlmCalls", label: "单次模型调用上限", detail: "单次用户运营最多 LLM 调用次数", kind: "number", defaultValue: 6 },
  { key: "simulationTokenBudget", label: "模拟评测预算", detail: "单次模拟/评测可用 token", kind: "number", defaultValue: 60000 },
  { key: "reactionTokenBudget", label: "反应分析预算", detail: "用户回应分析单次最多 token", kind: "number", defaultValue: 8000 },
  { key: "reactionMaxLlmCalls", label: "反应分析调用上限", detail: "用户回应分析最多 LLM 调用次数", kind: "number", defaultValue: 2 },
  { key: "quietHoursEnabled", label: "作息门控", detail: "开启后客户在休息时段来的消息不立即回，等醒来时段一次性回复；主动跟进也顺延到醒来", kind: "boolean", defaultValue: true },
  { key: "quietHoursStart", label: "休息起点(时)", detail: "进入静默的整点小时，运营方本地时区，0-23，含。默认 22", kind: "number", defaultValue: 22 },
  { key: "quietHoursEnd", label: "醒来时间(时)", detail: "结束静默/醒来回复的整点小时，0-23，不含。默认 8；起点>终点表示跨午夜", kind: "number", defaultValue: 8 },
  { key: "quietHoursTzOffsetHours", label: "作息时区偏移", detail: "运营方所在时区相对 UTC 的小时偏移，中国填 8。不依赖服务器时区，跨机房部署也稳定", kind: "number", defaultValue: 8 }
];

export function UserOpsModeHeader({ mode, onMode }: { mode: UserOpsMode; onMode: (mode: UserOpsMode) => void }) {
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


export function UserOperationCockpit({
  activeTab,
  busy,
  decisionReviews,
  guideBusy,
  guideInstruction,
  guidePreview,
  health,
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
  const examples = [
    "更像朋友一点，自然一些",
    "这个用户比较忙，降低主动打扰频率",
    "用户已经有明确需求，可以更积极推进下一步",
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
              <span>幻觉风险：{reviewScores.hallucinationScore ?? "-"}</span>
              <span>知识匹配：{reviewScores.knowledgeGroundingScore ?? "-"}</span>
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


export function TraditionalOpsTabs({
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


export function ContactsView({
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


function ActiveVersionsBar({
  meta,
  endpointPrefix,
  resourceLabel,
  busy,
  canPublish = false,
  onAfterAction
}: {
  meta: ActiveVersionMeta | undefined;
  endpointPrefix: string;
  resourceLabel: string;
  busy: boolean;
  canPublish?: boolean;
  onAfterAction?: () => void | Promise<void>;
}) {
  const [actionBusy, setActionBusy] = useState(false);
  if (!meta || !meta.id) {
    return null;
  }
  const version = meta.version ?? 1;
  const isCurrent = meta.currentVersion !== false;
  const previousVersion = meta.previousVersion ?? null;
  const seededBy = meta.seededBy ?? null;

  async function runAction(action: "publish" | "rollout" | "rollback") {
    if (!meta || !meta.id) return;
    const confirmText =
      action === "publish"
        ? `确认发布 ${resourceLabel} 新版本（version=${version + 1}）？`
        : action === "rollout"
        ? `确认把 ${resourceLabel} v${version} 设为当前生效版本？`
        : `确认回滚 ${resourceLabel} 到上一版本（v${previousVersion ?? "?"}）？`;
    if (!window.confirm(confirmText)) return;
    setActionBusy(true);
    try {
      await api.post(`${endpointPrefix}/${meta.id}/${action}`, {});
      if (onAfterAction) await onAfterAction();
    } catch (error) {
      window.alert(`${resourceLabel} ${action} 失败：${(error as Error).message}`);
    } finally {
      setActionBusy(false);
    }
  }

  const disabled = busy || actionBusy;

  return (
    <div className="activeVersionsBar">
      <div className="activeVersionsMeta">
        <span className={`activeVersionsBadge ${isCurrent ? "current" : "shadow"}`}>
          v{version}
          {isCurrent ? " · current" : " · shadow"}
        </span>
        {previousVersion !== null && (
          <span className="activeVersionsChain" title="previous_version 回滚链">
            ← v{previousVersion}
          </span>
        )}
        {seededBy && (
          <span className={`activeVersionsSeeded seeded-${seededBy}`} title="写入来源">
            {seededBy}
          </span>
        )}
        {meta.updatedAt && (
          <span className="activeVersionsTimestamp" title="updated_at">
            {meta.updatedAt}
          </span>
        )}
      </div>
      <div className="activeVersionsActions">
        {canPublish && (
          <button
            type="button"
            className="secondary"
            onClick={() => void runAction("publish")}
            disabled={disabled}
            title="基于当前 row 发布新版本（version+1，previous_version 自动写入）"
          >
            发布新版本
          </button>
        )}
        {!isCurrent && (
          <button
            type="button"
            className="secondary"
            onClick={() => void runAction("rollout")}
            disabled={disabled}
            title="把这一版本切到当前生效（其他版本 soft demote）"
          >
            切到当前
          </button>
        )}
        {previousVersion !== null && isCurrent && (
          <button
            type="button"
            className="secondary"
            onClick={() => void runAction("rollback")}
            disabled={disabled}
            title="把上一版本重新激活到当前生效"
          >
            回滚到 v{previousVersion}
          </button>
        )}
      </div>
    </div>
  );
}


export function DomainConfigEditor({
  busy,
  config,
  draft,
  mode: _mode,
  onDraft,
  onReset,
  onSave,
  onAfterVersionAction
}: {
  busy: boolean;
  config?: OperationDomainConfig;
  draft: OperationDomainDraft;
  mode?: "primary" | "standard";
  onDraft: (draft: OperationDomainDraft) => void;
  onReset: () => void;
  onSave: () => void;
  onAfterVersionAction?: () => void | Promise<void>;
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

      {config && (
        <ActiveVersionsBar
          meta={config}
          endpointPrefix="/api/admin/operation-domains"
          resourceLabel={`Domain ${config.domain}`}
          busy={busy}
          canPublish
          onAfterAction={onAfterVersionAction}
        />
      )}

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


export function UserPlaybookPanel({
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


export function DomainPromptPanel({
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


function readableChangeItems(changes: Record<string, unknown>) {
  const labels: Record<string, string> = {
    humanProfileNote: "运营备注",
    tags: "用户标签",
    domainAttributes: "领域属性",
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


function formatTime(value?: string) {
  if (!value) return "-";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}





/**
 * M3 / Task 79：Cockpit 内 Planner 视角。展示运营状态最近变更时间与
 * commitments 列表，给运营快速理解 AI 主动跟进的信号面。只读卡片，符合
 * "全自治、无中转"产品定位。
 */

export function PlannerViewSection({ contact }: { contact: Contact | null }) {
  if (!contact) {
    return null;
  }
  const stageUpdatedAt = contact.domainAttributesUpdatedAt;
  const stageLabel = (() => {
    const attrs = contact.domainAttributes;
    if (!attrs || typeof attrs !== "object") return "";
    const stage = (attrs as Record<string, unknown>).stage;
    return typeof stage === "string" ? stage : "";
  })();
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
          运营阶段 <strong>{stageLabel || "未分层"}</strong>
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


