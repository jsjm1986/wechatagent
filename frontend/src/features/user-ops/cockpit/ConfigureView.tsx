// 配置段独立视图。配置段内再分 4 个次级 tab：画像 / 指令 / 记忆 / 工具，
// 一次只显示一个 tab 的内容，避免原来 5 个模块纵向堆叠导致页面过长、无层次。
// 各 tab 内的 JSX / 回调名 / 数据引用一字不改（verbatim 迁移），仅重组结构：
//   - 画像：用户画像编辑（风格模板 / 判断 / 特别指令 / 辅助模式 / 客户类型 / 承诺 / 跟进）
//   - 指令：AI 调整（自然语言运营指令 + 修改预览）
//   - 记忆：运营记忆编辑器（4 分组手风琴，单行字段双列 / 多行字段全宽）
//   - 工具：长期记忆候选 + 影子验证
import { useState } from "react";
import {
  Activity,
  Bot,
  ChevronDown,
  ChevronUp,
  SendHorizonal,
  Sparkles,
  SquarePen,
  UserRoundCheck
} from "lucide-react";
import { useProfileStore } from "../../../stores/profileStore";
import { useUserOpsStore } from "../../../stores/userOpsStore";
import {
  MEMORY_DRAFT_FIELD_GROUPS,
  ChangePreview,
  EmptyInline,
  MemoryCardSummary,
  SimulationResult,
  formatTime,
  impactScopeLabel,
  memoryCandidateText,
  memoryStatusLabel
} from "../legacy";
import type { CockpitPanelProps } from "./CockpitPanel";
import styles from "./cockpit.module.css";

const CONFIG_TABS = [
  { key: "profile", label: "画像" },
  { key: "guide", label: "指令" },
  { key: "memory", label: "记忆" },
  { key: "tools", label: "工具" }
] as const;
type ConfigTab = (typeof CONFIG_TABS)[number]["key"];

export function ConfigureView(props: CockpitPanelProps) {
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
  const [activeTab, setActiveTab] = useState<ConfigTab>("profile");
  // 记忆分组手风琴：默认只展开第一组（用户理解），其余折叠，首屏只见 4 个组标题。
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(
    () => new Set(MEMORY_DRAFT_FIELD_GROUPS[0] ? [MEMORY_DRAFT_FIELD_GROUPS[0].caption] : [])
  );
  if (!selected) return null;
  const currentPlaybook =
    playbooks.find((playbook) => playbook.id === selectedPlaybookId) ||
    playbooks.find((playbook) => playbook.isDefault);
  const examples = [
    "更像朋友一点，自然一些",
    "这个用户比较忙，降低主动打扰频率",
    "用户已经有明确需求，可以更积极推进下一步",
    "重新分析画像，并补充不能踩的沟通禁忌"
  ];

  const toggleGroup = (caption: string) =>
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(caption)) next.delete(caption);
      else next.add(caption);
      return next;
    });

  return (
    <>
      <div className={styles.configTabs} role="tablist" aria-label="配置视图">
        {CONFIG_TABS.map((tab) => (
          <button
            key={tab.key}
            role="tab"
            type="button"
            aria-selected={activeTab === tab.key}
            className={activeTab === tab.key ? styles.configTabBtnActive : styles.configTabBtn}
            onClick={() => setActiveTab(tab.key)}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* 画像 tab：用户画像编辑（搬自 profile 块）。 */}
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
            <span>最近承诺</span>
            <small>运营可编辑：记录对客户作出的最近一条承诺，影响 AI 跟进话术。</small>
            <textarea
              rows={2}
              value={profileEditDraft.lastCommitment ?? ""}
              onChange={(event) => onProfileEditDraftChange({ lastCommitment: event.target.value })}
              placeholder="例：本周内给到方案报价"
            />
          </label>
          <label>
            <span>跟进策略</span>
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
      )}

      {/* 指令 tab：AI 调整（搬自 adjust 块）。 */}
      {activeTab === "guide" && (
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

      {/* 记忆 tab：运营记忆编辑器（4 分组手风琴 + 单行双列 / 多行全宽）。 */}
      {activeTab === "memory" && (
        <section className="smartTabPanel profileEditor">
          <div className="sectionCaption">运营记忆（运营可编辑，保存后影响 Agent 决策）</div>
          {MEMORY_DRAFT_FIELD_GROUPS.map((group) => {
            const expanded = expandedGroups.has(group.caption);
            return (
              <div key={group.caption} className={styles.accordionGroup}>
                <button
                  type="button"
                  className={styles.accordionHeader}
                  onClick={() => toggleGroup(group.caption)}
                  aria-expanded={expanded}
                >
                  <span>{group.caption}</span>
                  <span className={styles.accordionCount}>
                    {group.fields.length} 项
                    {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                  </span>
                </button>
                {expanded && (
                  <div className={styles.accordionBody}>
                    <div className={styles.fieldGrid}>
                      {group.fields.map((field) => (
                        <label key={field.key} className={field.multiline ? styles.fieldFull : undefined}>
                          <span>{field.label}</span>
                          {field.multiline ? (
                            <textarea
                              value={memoryDraft[field.key]}
                              rows={3}
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
                  </div>
                )}
              </div>
            );
          })}
          <div className="buttonRow">
            <button onClick={onSaveOperatingMemory} disabled={busy} type="button">
              <SquarePen size={16} />
              保存运营记忆
            </button>
          </div>
        </section>
      )}

      {/* 工具 tab：长期记忆候选 + 影子验证。 */}
      {activeTab === "tools" && (
        <>
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
                    <span>评分 {item.memoryWriteScore} · {formatTime(item.createdAt)}</span>
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
              <span>影子模式只看决策、风险和记忆变化，不写入真实会话。</span>
              <button onClick={onRunSimulation} disabled={simulationBusy || !simulationInput.trim()}>
                <Sparkles size={16} />
                {simulationBusy ? "验证中" : "开始验证"}
              </button>
            </div>
            <SimulationResult turns={simulationTurns} />
          </section>
        </>
      )}
    </>
  );
}
