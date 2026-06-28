# WechatAgent Admin 前端「可观测维度」盘点

> 以 `frontend/src` 实际渲染的代码为准。每项标注：真有展示 / 半成品 / 仅入口。
> 频道定义见 `frontend/src/app/channels.ts`；左侧导航分「运营 / 知识 / 系统」三组。
> 范围说明：本盘点聚焦运营者能在界面**读到**的运行/决策/客户/系统数据；纯写表单（配置类）只在与可观测相关时列出。

---

## 一、单次决策 / Run 维度

### 1. 单次 Run 完整经过（run envelope）— 真有展示
- 文件：`frontend/src/features/operations/index.tsx`（频道「任务日志」，也内嵌在用户运营→传统模式→审计复盘）。
- 看什么：每轮 Agent 决策的运行日志列表（状态、Run ID、触发来源、档位遥测、时间）。点「展开」逐阶段铺开 Planner / Context / 知识路由 / 决策 / Review / 送达网关 各 Document 的 key-value，含错误信息。档位遥测来自 decision 文档（sufficiency / missingTier / 是否升档）。
- 数据来源：operationsStore → `agentRuns`（run envelope）。

### 2. Review 记录（独立评审复盘）— 真有展示
- 文件：`frontend/src/features/operations/index.tsx`「Review 记录」Tab。
- 看什么：每条决策复盘的结论（通过/拦截，拦截时细分 finalReviewStatus / holdCategory 中文标签）、下一步动作、成效状态、六维评分（拟人度/情绪价值/幻觉风险/知识接地/压迫风险/隐私边界）、摘要、时间。
- 数据来源：operationsStore → `decisionReviews`（`/api/...` decision-reviews）。

### 3. 修订记录（pre/post 改写 + 自我批判）— 真有展示
- 文件：`frontend/src/features/autonomy/index.tsx`「近 50 条 revision 记录」。
- 看什么：联系人、修订前/后回复摘要、修订方向、归档状态、暂缓分类；展开看修订前后完整摘要 + **selfCritique（AI 自我批判原文）**。
- 数据来源：`/api/outcomes/autonomy/revisions`。

### 4. 会话内最近复盘 + 会话流 — 真有展示
- 文件：`frontend/src/features/user-ops/legacy.tsx`（智能模式→会话记录 Tab）。
- 看什么：左右气泡会话流（含名片引荐气泡、素材气泡），右侧「最近复盘」列出通过/拦截 + 状态 + 摘要。
- 数据来源：userOpsStore `messages` / `decisionReviews`。

### 5. 仿真 / 影子模式（simulate_user_dialogue）— 真有展示
- 文件：`frontend/src/features/user-ops/legacy.tsx`（智能模式→模拟验证 Tab）。
- 看什么：输入多行用户消息跑影子对话，逐轮展示候选回复、网关通过/拦截、幻觉风险、知识匹配、真人感、选中知识切片数、状态迁移 from→to、风险列表、上下文记忆卡。不触发真实发送。
- 数据来源：`runDialogueSimulation` → 仿真端点。

### 6. AI 总控指令回放（Management Agent 工具计划）— 真有展示
- 文件：`frontend/src/features/command-center/index.tsx`（频道「AI 总控」）。
- 看什么：自然语言指令 → LLM 工具计划逐步执行；每个 tool call 的状态（成功/失败/待核实/演练/进行中）与 detail（演练 would_execute、实际发送内容、网关状态中文标签、Review 是否通过、messageId）。支持 dry-run。高风险计划走「确认/否决」。
- 数据来源：commandStore `commandResult.toolCalls`（management.rs）。

---

## 二、客户维度（单个好友画像）
（全部集中在「用户运营」频道智能模式驾驶舱，`features/user-ops/legacy.tsx` + 子面板）

### 7. Agent 当前判断卡 — 真有展示
- 用户理解、下一步动作、当前运营状态、领域信号、最近用户来访 / 最近 Agent 触达时间。
- 数据：contact.agentProfile / operationState / lastInbound/OutboundAt。

### 8. 三层可信度标签 — 真有展示
- 文件：`features/user-ops/TagTrustPanel.tsx`。
- 运营录入层（权威，可编辑）/ AI 确信层（只读 chip + 证据条数 + 强证据/压缩重判来源）/ 贝叶斯评估层（标注「持续观测，永不驱动行为」）。

### 9. 贝叶斯判断走势 — 真有展示
- 文件：`features/user-ops/BayesianTrendChart.tsx`。
- 手写 SVG 折线，每个已占槽维度一条线，x=历史轮次、y=置信度 0~1；图例显当前值 + 当前置信度。未占槽维度不画线。

### 10. 大五人格 OCEAN — 真有展示
- 文件：`features/user-ops/PersonalityPanel.tsx`。
- 五维横向 bar（开放/尽责/外向/宜人/神经质）+ 各维置信度；低置信灰化、无证据标「证据不足」；snapshots≥2 画人格演化折线。

### 11. 运营健康度 — 真有展示
- 健康分卡片（用户理解完整度 / 信任关系质量 / 产品匹配清晰度），含分值 + tone + 说明。数据：operationHealth。

### 12. 长期记忆卡（memoryCard）— 真有展示
- 核心画像、核心/近期/已过期事实（带置信/重要度 chip + 易失效徽标 + 可展开证据 + 弃用 tooltip）、偏好/异议/承诺/禁忌/待办/记忆冲突。
- 候选记忆（另一 Tab）：待整理/已入库/低价值忽略状态 + memoryWriteScore + 候选条目 + 依据。

### 13. Planner 视角（运营阶段 / 承诺 / 维度）— 真有展示
- 文件：`legacy.tsx` PlannerViewSection。上轮对话模式、运营阶段（字典翻译，未知值/无字典灰化提示）、自某时起未变更、commitments 列表（含计划到期）、其余画像维度。

### 14. AI 已发送历史（素材 / 名片 + 客户反应）— 真有展示
- 文件：`legacy.tsx` SendHistorySection。AI 主动给该客户发过的素材/名片记录 + 发送时间 + **响应信号标记（已响应/未响应/待评估）**。
- 数据：`/api/contacts/:wxid/send-history`。

### 15. 成交 / 持有 / 疑似成交线索 — 真有展示
- 文件：`features/products-deals/index.tsx`（频道「产品与成交」）。
- 成交记录（金额、件数、可信度徽标：疑似待核实/已核实/支付核实）、客户持有（售后有效期）、**疑似成交待核实队列**（判断依据 + 置信度 + 出现次数 + 客户，运营核实才落正式成交）。

> 注：「客户情绪 / reaction」无独立读视图。AI 对客户反应的判断只以派生结果间接出现（memoryCard 的「最近情绪/关系温度」字段、send-history 的响应信号、人格/贝叶斯走势），**没有一个专门的「用户反应分析」面板**——属遗漏候选。

---

## 三、系统 / 自治态势维度

### 16. 运营工作台总览 — 真有展示
- 文件：`features/overview/index.tsx`（频道「工作台」）。
- 托管联系人数、托管覆盖率、在线账号数、实时运营流（每个托管好友的状态徽章：自主回复 / AI 策略暂缓 / 未托管 + 运营备注）。

### 17. 自治回路监控 — 真有展示（强）
- 文件：`features/autonomy/index.tsx`（频道「自治回路监控」，含 24h/7d/30d 窗口）。
- revision 触发率/通过率、未验证产品声明拦截率、新词候选触发率、**自我批判已回应率**、自治模式分布（auto/assisted/blocked）；**AI 暂缓三类细分**（AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文）条形图；发送链路状态（入队/送达/取消/终态失败/各率）；**Planner 自主调度**（沉默跟进/承诺到期/阶段停滞三段扫描器的 tick/emit/capped/backoff）。

### 18. 发件箱（outbox）— 真有展示
- 文件：`features/autonomy/OutboxPanel.tsx`（自治频道内嵌）。持久发件队列状态。

### 19. 长期成效指标 — 真有展示
- 文件：`features/quality/index.tsx`「长期指标」Tab（频道「运营成效」）。
- 按日：回复率、对话深度、AI 暂缓澄清率、Agent 拦截率、当日 run 数、当日 token。null 显「—」不当 0。

### 20. 公式遵守度评测 — 真有展示
- `features/quality/index.tsx`「公式遵守度」Tab。对 active 评测场景跑 simulate，比对 ground_truth 算 adherence；含降级/预算/偏差明细。

### 21. 知识自动校验（批量）— 真有展示
- `features/quality/index.tsx`「知识自动校验」Tab。对 needs_review 切片 LLM 校验，输出 verified/needs_review/rejected/needs_human_audit 计数 + 预算降级标记。

### 22. 产品声明兜底标记词 — 真有展示（含写）
- `features/quality/index.tsx`「产品声明标记词」Tab。可编辑 Rust 字符串守卫的标记词/白名单，保存触发红线语义审查（拒绝/需逐字确认弹框）。

### 23. LLM 成本 / 用量 / 缓存命中 — 真有展示
- 文件：`features/operations/index.tsx`「LLM 成本」Tab。调用次数、总 token、缓存命中 token、缓存命中率；逐条 llm_call_log（promptKey、状态、耗时、hit/miss、时间）。

### 24. 跟进任务 — 真有展示（含操作）
- `features/operations/index.tsx`「跟进任务」Tab。任务状态、内容、计划执行时间；可「立即复核 / 取消」。

### 25. 运营事件流（审计留痕）— 真有展示
- `features/operations/index.tsx`「运营事件」Tab。按时间线渲染事件 kind + 摘要 + 状态着色（含拦截/暂缓）。

### 26. 自我进化 / Evolution 中心 — 真有展示（强）
- 文件：`features/evolution/EvolutionCenterTab.tsx`（频道「演化中心」）。
- 近 7 天实验/候选/已发布/已回滚/显著性通过率聚合卡；候选列表（阈值类 current→proposed，prompt 类 section diff）；ProposalReleaseCard 含 shadow eval / Critic reasoning，发布需输入 RELEASE、回滚输入 ROLLBACK；灰度开关 + 比例；**阈值变更不可变审计日志**（动作/阈值项/值变更/操作者/时间）。

### 27. 发送成效（素材 / 名片 ROI）— 真有展示
- 文件：`features/send-analytics/index.tsx`（频道「发送成效」）。
- 总发送数、响应率、阶段推进率；素材/名片分 Tab 排行（已发次数、覆盖客户数、响应率、阶段推进率）。

### 28. 微信号在线状态 — 真有展示
- 文件：`app/Shell.tsx` AccountSwitcher。账号下拉 + 在线点 + 「N/M 在线」；0 账号时给同步入口。

### 29. AI 模型配置（含连通性测试）— 真有展示（含写）
- `features/llm-providers/index.tsx`。Provider 列表（base_url/model/格式/激活/vision），测试连通性返回 latency/错误，一键热切换。

### 30. 多租户 workspace 切换 — 半成品
- `app/Shell.tsx` WorkspaceSwitcher：仅当 `user.workspaces.length > 1` 才渲染切换器，否则纯文本显示当前 workspace。无独立 workspace 维度看板。

---

## 四、决策请示 / Ask-Human（重点确认）

### 31. 统一收件箱（Ask-Human Inbox）— 真有展示（强，且是营销最可能漏的）
- 文件：`features/ask-human/index.tsx`（频道「统一收件箱」）。
- 八类来源 chip + 计数：**请示裁决（principal_escalation）/ 知识核验 / 标签候选 / 关系建议 / 知识缺口 / 画像发布 / 进化发布 / 经验晋升**。每项 inline 或 rich 卡片处置。
- 数据：inboxStore（汇总 `/inbox` + summary，按 source 过滤）。

### 32. 请示裁决处置（幕后决策源链路）— 真有展示
- 文件：`features/ask-human/inline/EscalationInline.tsx`。
- 客户、AI 向决策人提的具体问题、类别；裁决类型（批准/驳回/有条件/暂缓/退回再议）+ 授权窗 + 约束 + 转述意见；可改派给备选决策人。这正是「AI 遇超职权事项向幕后领导请示、拿结论转述客户」的真实读+处置视图。

### 33. 已裁决请示历史 — 真有展示
- 文件：`features/ask-human/ResolvedEscalations.tsx`。
- 已裁决回顾：短码、裁决结论、转述实质、客户、约束、授权到期、裁决渠道（决策人微信/后台直接/决策人对话）。
- 数据：`/api/admin/principal-escalations?status=resolved`。

### 34. 请示通道配置 — 真有展示（含写）
- 文件：`features/ask-human-config/index.tsx`（频道「请示通道配置」）。
- 决策人链编辑、四类触发请示情形开关、超时转备选、推送频控（去重窗口/每日上限/静默时段）。

### 35. AI 待审新想法（候选标签 / 关系建议 / 疑似成交 / 知识缺口）— 真有展示
- 统一收件箱内 SimpleApproveReject 卡片处置：标签候选、关系类型建议、知识缺口信号（dismiss）；疑似成交另在产品频道（见 #15）。

---

## 五、专属顾问名片引荐 / 辅助模式（重点确认）

### 36. 专属顾问名片库 — 真有展示（含写）
- 文件：`features/referral-cards/index.tsx`（频道「专属顾问」）。
- 顾问名片列表（草稿/可引荐 + 启停徽标）、引荐时机、目标阶段、标签；录入 + 审核（标记可引荐/撤回）+ 启停 + 删除。明示需到运营域开启账号级辅助模式。

### 37. 单客户辅助模式 override + 引荐留痕 — 真有展示
- 文件：`features/user-ops/legacy.tsx`（智能模式→用户画像 Tab，仅 managed 客户）。
- 辅助模式三态（跟随账号/强制开/强制关）、「已引荐 · AI 已退辅助答疑（时间）」留痕、撤销引荐按钮。会话流里名片引荐显专属气泡（#4）。

### 38. 名片成效 — 真有展示
- 见 #27 发送成效「名片效果」Tab。

---

## 六、知识库健康维度（「Wiki 管理」频道，`features/knowledge/`）

### 39. 知识库概览驾驶舱 — 真有展示
- 文件：`features/knowledge/cockpit/CockpitView.tsx`（控制台→概览）。
- 应答模式仪表、知识覆盖判定、治理待办（待审草稿 / **D2 降级（active 但缺原文锚点）** / 知识缺口）、缺口明细、批量自动校验入口。

### 40. 切片修订留痕 — 真有展示
- `features/knowledge/steward.tsx` ChunkRevisionsDrawer（库→修订历史）+ ChunkInspectorPane（WebSocket 实时同步）。

### 41. 待评审 / 质量信号（lint）— 真有展示
- steward.tsx ReviewView + LintView（库→质量中心）。待核实草稿、矛盾/过时/缺锚等质量信号。

### 42. 知识诊断仪表（observability，高级折叠）— 真有展示
- steward.tsx ObservabilityDashboard（控制台→高级→诊断仪表）。
- catalog/completeness/integrity、日志分析、**LLM 缓存命中统计**、phase rollup（lifecycle 计数、修订原因、reviewer 误判率、**请示 escalation 按状态分布 + pending 时长分桶 + 最老 pending 时长**、负例待办）。
- 注：这是工程诊断面板，默认折叠，不铺给普通运营。

### 43. 试召诊断 / 知识树 / 文档目录 / 外部源 / 关系图谱 / Schema — 真有展示
- steward.tsx + atlas.tsx + explore.tsx：TryRecallView（试召 + grounding 选中切片）、KnowledgeTreeView、DocumentsView、IngestSourcesView（RSS/HTML 自动采集源）、ChunkGraphView、DomainSchemaTab。

### 44. 今日 Digest / AI 协作起草 / 知识待办收件箱 — 真有展示
- `features/knowledge/today.tsx`（工作台）：DigestCanvas、ChatWorkbench（AI 协作起草知识，含 draftPreview/missingFields/followupQuestions）、KnowledgeInbox、TaskRail（派工）。

---

## 七、其它系统配置（与可观测弱相关，含写为主）
- 系统策略（`features/system-strategy`）：后台总控 Agent / 方法论 Agent / 跨模块 Prompt Pack、人格设定与任务提示词、版本发布/回滚（ActiveVersionsBar）。
- 用户运营传统模式（`features/user-ops`）：运营方法 playbook、Agent 提示词、运行策略（频控/边界/状态机编辑器 + 辅助模式开关）。
- MCP 密钥表单（`features/command-center/McpKeyForm.tsx`）：账号 MCP 凭证录入。

---

## 缺口 / 弱项小结（前端确未兑现或半成品）
- **客户情绪 / 用户反应（reaction）无专门读视图**：后端有 reaction 分析能力，前端只在 memoryCard 字段、send-history 响应信号、人格/贝叶斯走势间接体现，无独立面板。
- **多租户 workspace**：仅在多 workspace 时显切换器，无 workspace 维度看板（半成品）。
- 知识诊断仪表 / 试召诊断等高级面板刻意折叠，不面向普通运营。

## 维度计数
按上表，面向运营者的**前端可观测/可处置维度共约 44 项**（条目 1–44；其中 #30、#42 为半成品/工程向，reaction 为缺口）。若只算「真有面向业务运营的读视图」核心维度，约 38 项。
