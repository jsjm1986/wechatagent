# 前端测试全集与 website 深读记录（核证日期 2026-08-13）

> 深读范围：`frontend/src/__tests__/` 全部 138 个文件（137 个测试 + setup.ts，合计约 16,180 行）逐个全文读完；`website/` 全部 11 个 HTML 页面 + `assets/shared.js` + `assets/i18n.js`（`assets/*.css` 为样式未逐行；`website/docs/` 下 5 个 md 为 obs 频道 spec/plan 搬运件，结构级扫过）。
> 交叉验证基准：`project-understanding/13-frontend-core.md`、`14-frontend-features.md`（两篇断言在引用处均已按需亲验）。「实际能力」对照全部亲验源码，行号为本日现值。
> 断言强度评级口径：**强**＝锁定不变量/红线/wire 契约/竞态时序，含反向断言；**中**＝真实行为断言但覆盖面窄或单路径；**弱**＝冒烟级（渲染出文字/类名即过）。

---

## 1. __tests__ 全清单编目（文件 → 测什么 → 断言强度）

### 1.0 setup

| 文件 | 测什么 / 作用 | 强度 |
|---|---|---|
| setup.ts (35) | 引入 jest-dom；修复 Node 25 `--localstorage-file` 注入的伪 localStorage（`getItem` 非函数时换成 Map 实现，setup.ts:9-35） | —（基建） |

### 1.1 contracts/（7 个，全部同构「双向键集对账」）

| 文件 | 对账内容 | mock | 强度 |
|---|---|---|---|
| operationsDomain.contract.test.ts (62) | 10 个 fixture（behavior_signal/outcome/llm_call_log/memory_candidate/operating_memory/agent_run/decision_review/guide_preview/operation_health/guide_apply_receipt）↔ CANONICAL_KEYS | 无（直接 import fixture JSON） | **强**：`missingInFrontend`/`deadInFrontend` 双向必须同时为空 + fixture 非空防 bless 写空（:25-39） |
| knowledgeDomain.contract.test.ts (46) | 6 个知识域 fixture（document/usage/revision/chunk_detail/import_job/ingest_source） | 无 | **强**（同款对账函数） |
| configPlaybookDomain.contract.test.ts (58) | 7 个 fixture（playbook/prompt_template/evaluation_scenario/suspected_deal/outbox_entry/outbox_payload/tool_call）；额外断言 outbox payload 保留 typed 业务身份 `{kind:"media",assetId}`（:44-55） | 无 | **强** |
| evolutionDomain.contract.test.ts (54) | 8 个演化域 fixture（runtime_flag/threshold_override(+audit)/experiment_envelope(+summary)/proposal_summary(+detail)/shadow_replay） | 无 | **强** |
| taxonomyDomain.contract.test.ts (42) | 5 个字典域 fixture（state_policy/taxonomy_candidate/relationship_suggestion/taxonomy_entry/operation_domain） | 无 | **强** |
| operationKnowledgeChunk.contract.test.ts (25) | chunk 列表投影 33 键单独对账 + 防空 fixture | 无 | **强** |
| operationStateAction.contract.test.ts (14) | **值闭集**（非键集）：动作 5 值与后端 fixture 全等 + 每值有中文标签（:10-12） | 无 | **强** |

合计消费 38 个 .contract.ts + `gateway_status_values`（由 lib/reviewLabels.test 消费），与 13 号记录 §2.7 的机制完全吻合；**无一空壳**。

### 1.2 lib/（7 个）

| 文件 | 测什么 | mock | 强度 |
|---|---|---|---|
| apiPatch.test.ts (39) | `api.patch` 发 PATCH+JSON header+body、非 2xx 抛 parseApiError 文案 | stubGlobal fetch | 中 |
| applyAiRepairPatch.test.ts (51) | 红线：body `thenVerify:false` 恒定 + **serde 命门反断言** `not.toHaveProperty("then_verify")`（:33-34）；acceptedFields/skippedFields 分拣；失败归一 apply_failed/server_error 不抛 | stubGlobal fetch | **强** |
| clipboard.test.ts (83) | 两级降级（clipboard→execCommand）、writeText 抛错降级、双不可用返回 false 不抛、临时 textarea 清理（:73-82）；显式摘 clipboard 复现生产 HTTP 形态 | defineProperty navigator.clipboard / document.execCommand | **强** |
| inboxApi.test.ts (95) | severityRank 排序 + 未知值=0 不抛；sortItems 不变输入数组；summary 保留 null 计数（不可用≠0）；inbox/summary URL 编码 accountId（`acc /一` → `acc+%2F%E4%B8%80`，:73-94） | spyOn api.get | **强** |
| reviewLabels.test.ts (100) | FINAL_REVIEW 10 值闭集、HOLD 3 值、REVIEW_PHASE 15 值全中文；**GATEWAY_STATUS_LABELS 用后端 fixture 全遍历**（39 值全覆盖+已翻译）+ 与 finalReview 交集措辞一致 + `delivery_finalizing` 不混入 gateway 闭集（:80-99）；labelOf 三态 | 无（import fixture） | **强** |
| useSseReconnect.test.ts (147) | 指数退避精确到 ms（999 不重连/1000 重连）、maxRetries 停、业务事件重置 attempt、**open 重置连续失败预算**（累计短断线不 gave-up，:57-79）、旧 ES 迟到事件防串扰、close 后不重连、terminal 事件停 | FakeES 类 + fake timers | **强** |
| uuid.test.ts (75) | 三级降级逐层摘除复现（randomUUID 缺失/抛错→getRandomValues→Math.random），全部输出合法 v4 正则；200 次不重复 | defineProperty globalThis.crypto | **强**（线上事故复现，头注 :5-7） |

### 1.3 stores/（14 个）

| 文件 | 测什么 | mock | 强度 |
|---|---|---|---|
| accountStore.test.ts (27) | currentAccountId 回退首个、onlineCount、selectAccount 写 localStorage | 无 | 中 |
| commandStore.test.ts (99) | confirm/reject 带 `{accountId, planHash}` 冻结；切号后确认零请求 + 报错文案；缺 planHash 拒发（:80-98） | vi.mock api | **强** |
| contactStore.test.ts (66) | managed/normal 计数；**A 慢 B 快竞态**（deferred 双 promise，迟到 A 不覆盖 B，:47-65） | vi.mock api + uiStore | **强** |
| contentStore.test.ts (135) | 写请求冻结 scope（account 级带 expectedAccountId / workspace 级 expectedScope）；A 慢 B 快；切号后旧资产删除零请求 | vi.mock api | **强** |
| inboxStore.test.ts (172) | summary 旧响应被 generation 丢弃；源级错误不清好 items；换 source/换账号迟到响应丢弃；**请求级失败保留旧 items 只置 fatalError**（:149-171） | vi.mock inboxApi（sortItems 恒等） | **强** |
| navigationStore.test.ts (13) | 仅默认 channel="command" + setChannel | 无 | **弱**（见 §2-B1） |
| operationsStore.test.ts (161) | 失败上报全局横幅；五端点 A 慢 B 快竞态（按 resolver 索引喂 tasks/llm 槽位）；任务动作 expectedAccountId 冻结 + 切号零请求；loadAgentRuns 成功/失败 | vi.mock api + uiStore | **强** |
| profileStore.test.ts (43) | labelFor 三态（ok/unknown_value/no_dict）；**BIZ-1 键名契约**：domainAttributes 读 `customer_stage` 而非裸 `stage`（camelCase rename 不递归进 Document 的回归锁，:23-42） | 无 | **强** |
| promptSaveThreeState.test.ts (107) | savePromptTemplate 三态：ok 切新 id + reload；needsConfirm(200) 不 reload 不当成功；rejected(4xx 红线文案)；force:true 透传；普通错误 setError；**成功缺新 id fail-closed 清发布目标**（:97-106） | vi.mock api；loadStrategyData spy | **强** |
| publishThreeState.test.ts (60) | publishPromptTemplate 同款三态 + force | 同上 | **强** |
| soulVersionIdentity.test.ts (73) | saveSoul 后发布目标切新 id（publish 打新 id URL）；缺新 id / PUT 失败都清空旧发布目标（防误发旧版本） | vi.mock api | **强** |
| strategyStore.test.ts (60) | saveDomainProfile create/update 双路（POST 顶层补 camelCase profileId / PUT :id）+ 锁新草稿 id；SR-138 reset 带精确认短语 | vi.mock api + uiStore | **强** |
| userOpsHealth.test.ts (89) | FE-1：guide preview 用后端 `health.items`（7 canonical 项、Risk 类 danger），**反断言不含旧 4-key 占位**（trust_level/engagement…，:77-87） | vi.mock api | **强** |
| userOpsStore.test.ts (644) | saveManualTags 谓词早退/URL body；loadMessages 端点面（含**死端点反断言** `/api/contacts/:id/messages` 不再调，:148-150）+ allSettled 单面板失败不拖垮 + A/B 详情竞态；旧草稿跨号保存零请求；guide preview 竞态 + 陈旧 preview 不 apply + apply 冻结五件套；operating-memory 四 Document 归组/回填/null 清脏；operationProfile body 不含 AI 派生字段（customerStage 反断言）；clearReferral（无谓词，见 §3）；playbook A/B 竞态 + 切号清编辑态零写；enableAgent 冻结 + 失败复位 busy；saveOperationDomain 作息参数对象化 + override draft | vi.mock api + uiStore | **强**（store 层最全样本） |

### 1.4 顶层 + app/（3 个）

| 文件 | 测什么 | mock | 强度 |
|---|---|---|---|
| AutonomyOutcomesTab.test.tsx (292) | 与后端 `tests/outcomes_autonomy_endpoint.rs` 镜像的 6 case：totalRuns=0 全 "—"（≥11 个独立节点+3 条 hold bar）；40.0%/50.0% + hint 2/5、1/2；三类 hold 33.3%（1 条）×3 + 三固定中文标签；held_for_human 脏值不进分类；planner 三 column 数字/hint（testid 级）；无 planner 段不渲染 | stubGlobal fetch（URL 分流） | **强** |
| EvolutionCenterTab.test.tsx (659) | 4 状态徽章文案+data-tone；发布钮仅 eligible 可用/回滚仅 released；**ConfirmModal 逐字 RELEASE**（小写/错串/尾空格全 disabled 且零请求）；prompt diff 双栏互不渗透 + critic/expected 渲染；runtime-flag PUT body camelCase + 从 `.flag` 内层回读；审计按钮加载+空态；env 硬锁/flag 关仍拉历史（F-008）+ dormant 提示；开总开关 PUT enabled+100；coverage 不完整不伪造聚合；PUT 失败保原值 role=alert；flag 拉取失败显重试不卡加载 | stubGlobal fetch 顺序队列 | **强** |
| app/Shell.test.tsx (234) | 分组展开/独立折叠（非手风琴）/幂等开合/收起打点唯一/计数/DEFAULT_COLLAPSED 三组；aria-current 唯一；comingSoon 灰显非按钮；单列结构无 tablist；**GROUP_ORDER 完整性**（每频道 group 必在 GROUP_ORDER，否则静默不渲染，:182-186）；workspace 切换器多/单 workspace 行为 | 直接 setState store | **强** |

### 1.5 components/（8 个 review 族 + 10 个 ui 族 + 2 个独立）

| 文件 | 测什么 | 强度 |
|---|---|---|
| FriendPickerModal.test.tsx (101) | 渲染优先级 remark>nickname>wxid、三字段过滤、onSelect、loading/error/空态/emptyText、手动 wxid 开关双态、720px 宽、计数「共 N 位/匹配 N 位/空不显」 | **强**（组件级全行为） |
| LlmErrorBanner.test.tsx (95) | 普通 Error→「客户端错误」不冒充上游（线上事故复现）；client_error 按钮用调用方动作名≠「AI 重试」；retrying 态；LlmUnavailableError 显上游分类+自动重试次数+「AI 重试」；缺 kind 回落 unknown；无 onRetry 无按钮 | **强** |
| review/ChunkReviewCard.test.tsx (114) | **verify-gate 红线真值表**：hasQuote&&hasAnchor 四分支 + 空白 quote trim + camelCase 拼写等价放行；verify 提交 expectedUpdatedAt；reject 恒可点（:31-113） | **强**（红线唯一承载卡回归锁） |
| review/ProfilePublishCard.test.tsx (144) | generated_state_machine 渲染（states/goal/advanceSignals/riskRules 内层 camelCase）；无状态机不崩；发布只调 publish 不调 rollout/activate（反断言）；partial 激活提示+重试附属同步入口；历史 published 无发布钮 | **强** |
| review/ProposalReleaseCard.test.tsx (267) | E12 五字段元数据渲染/全空不渲染；prompt 类聚合表（五闸 GATE_LABELS 全出现）+ 样本对照表（点阵 `●○○○○`/`○○○○○` 字形回归、自评已解决/未解决、状态中文化）；空 original 不崩；**白名单 key 移出通用表**（prompt 类）vs threshold 类全保留 | **强** |
| review/ReviewQueue.test.tsx (155) | 挂载 fetch+渲染；runAction 成功后 refetch；**[A,B]→[B] 刷新后草稿按对象 id 绑定**（编辑值不串行、approve 打 B 的 URL、无 React key 告警）；**旧 generation 动作闭包被拒**并提示「列表已刷新」（:118-154） | **强** |
| review/TaxonomyCandidateReviewCard.test.tsx (106) | 预填/中文维度名（反断言无 slug/canonical/rawValue 黑话）；采纳 body canonicalValue；空 id/名拦截零请求；409→「已存在」提示非错误；mergedIntoExisting 文案；驳回 reason 必填 | **强** |
| review/evidenceMetrics.test.ts (66) | FIVE_GATE_KEYS 固定 5 键有序；gateHit 布尔窄化（缺失/非布尔→null）；readAggregateEvidence snake_case 聚合读取/无聚合返回 null；PROMPT_AGG_METRIC_KEYS 白名单 | **强** |
| ui/Avatar (16) / EmptyState (16) / MetricCard (15) / PlanStep (12) / StatusLine (12) | 渲染文字 + tone/status class 存在 | **弱**（冒烟） |
| ui/StatusBadge (16) | 渲染 + tone class；**held 不带 running 呼吸类**（反断言） | 中 |
| ui/ConfirmDialog (64) | promise 化 confirm/cancel/Esc 三态；requireText 未匹配禁用 | **强** |
| ui/FormDialog (77) | 提交返回字段值、required 禁用、取消 null、**逐键输入保持焦点与完整值**（受控输入回归） | **强** |
| ui/Overlay (71) | open 双态、role=dialog+aria-modal、Esc、scrim 可配、进场焦点 | 中 |
| ui/Toast (38) | success 进 role=status、点关闭移除 | 中 |

### 1.6 features/ 顶层（8 个，知识驾驶舱 + 信任类型）

| 文件 | 测什么 | 强度 |
|---|---|---|
| AnsweringModeGauge.test.tsx (25) | 三档标签/档位、有草稿解读、**红线文案反断言**：不得承诺「审掉即解锁」（clamp 只解除封顶，:15-20) | **强** |
| AutoVerifyPanel.test.tsx (28) | 默认三档+留复查开关；开始筛提交 `{confidenceThreshold:7, humanAuditSampleRate:0.3, limit:50}` + 结果三堆 | 中（0.05 硬下限未测，见 §3-7） |
| CockpitView.test.tsx (55) | answeringMode 仪表+5 维+缺口明细；三计数卡口径（needsReview/anchorsMissing/gap-signals.length）等待并行请求全落 | **强** |
| CoverageVerdict.test.tsx (34) | 5 维中文行、effectClaims=missing 显「拦」、下钻回调带维度 key | 中 |
| ReviewChat.test.tsx (133) | 双检查过→生效键可用；缺锚点禁用+大白话；「只动这条」标题；富字段（用量/降级/字段锁）；退回成功关面板/失败不关；patch diff 中文 label+新值+attachments 三元组；snake_case/未知键兜底；**目标或版本不匹配不展示 patch** | **强** |
| UserOperationCockpit.memory.test.tsx (120) | ConfigureView 记忆表单：身份 patch 回调、保存回调、lastCommitment/followUpPolicy patch | 中 |
| trustTypes.test.ts (138) | parseCompleteness 真实响应/缺字段安全默认/dimensionList 固定序+后端优先（M4 三分支）/answeringModeLabels 逐档回落（I）；parseIntegrityReport；TrustChunkFields 旧数据合法；canGoLive D2 镜像三分支 | **强** |
| useGoLive.test.ts (92) | apply→verify 串调**用回执新 updatedAt**（非管理员旧快照）；apply 缺新版本 fail-closed 不调 verify；409 归 gate_blocked；无 session 直接 verify；**空白版本令牌零请求**；5xx/网络归 server_error | **强** |

### 1.7 features/ 各频道（按目录）

**account-management/AccountLogin.test.tsx (84)**：wire 契约（session_id/qr_data_url/login_page_url snake_case）+ poll URL；**过时字段 login_session_id 拒收**报「missing session_id」零 poll；取消后迟到 poll success 丢弃（generation）。**强**。

**ask-human/（9 个）**
| 文件 | 测什么 | 强度 |
|---|---|---|
| AskHumanLayout.test.tsx (262) | 白卡结构（不占全局 .panel 防污染 user-ops）、panelHead 计数、**total=null 不显「待处理 0 项」**、chip 单 toolbar 容器（jsdom 局限注释明示）、计数 0 的源不渲染 chip、activeSource 计数 0 仍保留、不可用源显「不可用」、空态用共享 EmptyState、grid gap 直接子元素前提锁定 | **强** |
| AskHumanView.dataSource.test.tsx (287) | 单数据源链路（真实 sortItems，high 冒顶）；单次刷新 fetch 恰 1 次（mount1+刷新1）；刷新失败保旧列表+fatal 横幅（不走 ReviewQueue 短路）；切源 chip 换数据（fi 带 source）；账号筛选双刷新 + 账号归属 tag（一号业务/全局）；needs_human_audit 徽章接线 + held 色类；summary 源错误显「不可用」非 0 | **强** |
| EscalationInline.labels.test.tsx (28) | category/verdict 中文（反断言英文 id 不现身） | 中 |
| InboxRow.collapse.test.tsx (33) | 默认折叠/点开渲染 children；tag pill 传/不传 | 中 |
| ResolvedEscalations.test.tsx (106) | resolved 端点+verdict 共享字典+授权到期/渠道渲染；null 到期显「本次转述不设期限」；空占位；从频道切「已裁决历史」联通 | **强** |
| gapSignalLabels.test.tsx (32) | gap kind/severity 中文（反断言 orphan/warning 不裸露） | 中 |
| inline/EscalationInline.test.tsx (92) | resolve 提交 verdict+substance；conditional 显授权窗并提交 constraints+authorizationWindowHours+exemptionType；富展示；改派 reassign toWxid | **强** |
| inline/SimpleApproveReject.test.tsx (65) | 富字段展示；**confidence=0 仍展示**（0≠缺省）；无富字段只显摘要（标题不重复渲染反断言） | **强** |
| inline/SuspectedDealReviewCard.test.tsx (82) | 元→分换算+币种大写+**signalId URL 编码**（deal%2Fsig-1）；非法金额客户端拦截零请求 + yuanToCents 纯函数；驳回 reason 必填+trim | **强** |

**ask-human-config/（3 个）**
| 文件 | 测什么 | 强度 |
|---|---|---|
| DeciderChainEditor.test.tsx (256) | 已入库直接入链零 import；未入库先 `POST /api/contacts/import` 再入链（body 契约）；import 成功不 force 重拉（§4.4 防打断连选）；**200 空 items=静默失败显式报错**；失败走 toast（z-index 遮罩层级注释）+ 弹窗保持开；非真人四类过滤（gh_/@chatroom/isNonHuman）；链中已有排除；**无手动 wxid 入口**（fail-closed 反断言）；删除/上移；同步中/加载失败态；**连点重入守卫恰 1 次 import**；**import 期间父级改 chain 用最新值非 stale 闭包**（:232-255） | **强**（编辑器类最全） |
| deciderCandidates.test.ts (31) | isPickableDecider 六分支（真人/isNonHuman/gh_ 前缀/群/@openim/gh 中缀不算） | **强** |
| policyForm.test.ts (78) | defaultPolicy 保守默认（aiPolicyHold=false 其余 true）；extractPolicy 完整/缺失/垃圾回落；validatePolicy 七规则（空链=合法关闭态、决策人必须绑账号、quietHours 越界、负数、cap≥1） | **强** |

**autonomy/（2 个）**：autonomy.test.tsx (176)——一体化壳小标题+accountStore 驱动；三 hold 33.3%×3；revision 行 finalReviewStatus/holdCategory 中文化（原始枚举反断言）。OutboxPanel.test.tsx (228)——拉取渲染+accountId 入查询串；取消确认框显账号/客户/目标、确认后才 POST（expectedAccountId+cancelReason）；**确认框开着切号即隐藏旧快照且不取消旧账号条目**；A 慢 B 快；in_flight+cancelRequested/delivery_unknown 显状态且**无取消按钮**；同客户双素材身份区分+确认框只引用选中项+恢复 2 次风险文案。两者皆**强**。

**campaign/（9 个）**：store.test.ts (175)——loadReport 成功/失败/lastAttemptedId；openReport 联动；**A 慢 B 快 + 响应 campaignId 不符拒收**；列表 loadCampaigns 失败也置 listLoaded（防循环）；view/page/clear。create.test.tsx (97)——空字段禁用；create+preview 双 POST 断言；**改条件→PATCH 带 expectedSpecVersion CAS→再 preview**（create 仅 1 次）；命中 0 提示；**红线：无任何 dispatch/确认推送控件**（反断言）。campaign.test.tsx (136)——7 桶 tone/label（含兜底）；汇总+明细+escalated reason 中文；空明细；桶过滤；**report 身份错位隐藏明细禁导出**。board-paging (47)——50/页+翻页器有无。list.test.tsx (58)——空态/行数/「已下发」列头文案/openReport/未加载才 loadCampaigns。no-refetch-loop (40)——**失败后 effect 不循环重发**（50ms 后仍 1 次）。campaignStatus (23)——6 状态 tone/label+兜底。commandJump (29)——dispatchCampaignId 六分支守卫（dry_run/null response/非字符串 id → null）。csv.test.ts (50)——表头/中文状态/转义/**公式注入中和**（=+-@ 前缀 + 前导空白/制表变体）。整组**强**。

**command-center/（2 个）**：McpKeyForm.test.tsx (71)——camelCase PUT+不回显+提交后清空；**切号 render 期即销毁草稿**（passive effect 前提交也零请求）；A 保存迟到不在 B 显「已保存」。commandCenter.test.tsx (174)——前 5 个 it 冒烟（标题/计数文案）；pending_confirmation 渲确认/否决、succeeded 不渲；executed_unverified 显「待核实」；F13 gatewayStatus 中文化+未知回落。**强/中混合**。

**content-assets/contentAssets.test.tsx (177)**：前 5 个 it 冒烟；kind 中文非裸英文（faq 反断言）；注入档中文；编辑提交含 minInjectTier；删除经 confirm；**禁语行「恒注入」不显档位**（误导反断言）。中→强。

**evolution/evolution.test.tsx (76)**：开启态渲染聚合卡+空候选；env 硬锁占位。中。

**knowledge/（20 个）**
| 文件 | 测什么 | 强度 |
|---|---|---|
| ChunkInspectorRepairProvenance.test.tsx (118) | needs_review 显「AI 修复建议」/verified 不显；provenance 渲染/null 不崩；**他人 presence 只提示不禁编辑**（按钮仍 enabled） | **强** |
| ChunkInspectorUnrelate.test.tsx (162) | 解除关联 DELETE `/chunks/:src/relate/:target`（confirm 后）；**dead 关联仍可解除** | **强** |
| ChunkRepairPanel.test.tsx (93) | propose→patch 逐字段复选+中文 label+confidenceHint；失败横幅；followup 问答（answer body sessionId/turn/answers/previousPatch）；落库调 applyAiRepairPatch（acceptedFieldNames）；失败不 onApplied | **强** |
| DigestCanvasDismiss.test.tsx (155) | dismiss 带 report accountId（URL 编码）；**重算失败保留上次成功卡片**+alert 文案；SR-125 批量派工提交 reportHash/cardHash 封印（**反断言不带 plannedSteps/cardIds/candidateHash**） | **强** |
| DigestCanvasInsecureContext.test.tsx (121) | **线上事故复现**：摘 randomUUID 后批量派工仍发出（sessionId 非空）+无 alert；本地异常显「客户端错误」+「重新加载」≠「AI 重试」 | **强** |
| DigestCardRendering.test.tsx (142) | digestTargetRefLabels 六规则（尾 6 位/丢空 id/去重限 3/kind 缺省与未知）；**同标题双卡靠 targetRefs 区分**；metric 中文化+阈值 0 不渲染/非零渲染 | **强** |
| DocumentEdit.test.tsx (190) | E6 编辑先 GET 详情冻结 version；**PATCH 只发 version+脏字段**（rawContent/contentHash 等反断言）；详情不匹配/无版本拒绝编辑；无改动关闭零 PATCH；双文档迟到详情丢弃 | **强** |
| DocumentRepairPanel.test.tsx (62) | 只渲 needs_review；空态；加载失败显错；onApplied→onRepaired 透传 | 中 |
| ImportWizardApply.test.tsx (116) | **封印契约**：apply 只发 previewId+previewHash+勾选 candidate 的 patch（document/items/accountId/sourceName 反断言） | **强** |
| ManualNewChunk.test.tsx (102) | **E7 红线**：手工新建 POST 写死 `status:"draft"+integrityStatus:"needs_review"`（头注直言后端 create 缺省落 active 的命门） | **强** |
| MemoryDrawer.test.tsx (111) | 撤销冻结 scope（body accountId/operatorId/reason）+只移除该行；失败保留行 | **强** |
| ObservabilityDashboard.test.tsx (223) | catalog 包络（持久化/实时/差值）；**空目录显 0 vs 错误包络显「不可用」**；性能卡真实比率/轮次/降级原因；无样本显「—」非 0.0%；phase-rollup/worker-health 逐指标口径标签（flow_window/current_inventory/…）+ 75.0%/7.0 分钟/512.0 KiB 格式化 | **强** |
| ReviewQueueInvalidation.test.tsx (144) | lagged 事件→invalidateChunks 派发；dimension 下钻 URL+服务端 facet 不重分类；**突发失效合并为一次尾随 reload**（maxInFlight=1、过期响应不落地、横幅出现/消失） | **强** |
| TaskRailList.test.tsx (279) | 列表渲染（completedStepCount 数字非 .length）；点选拉详情；turn 事件回读权威详情；**SSE 连续失败→有限轮询→终态收敛停**（fake timers 逐延迟推进）；无 EventSource 12 次封顶+提示手工拉取 | **强** |
| adminGovernance.test.tsx (186) | PublishBar 三按钮兜色 class（白底白字回归）；**colgroup 列数=表头列数**（9/6/5，加列漏改即红）；ISO 时间本地化渲染（toLocaleString 现算防时区脆断）+缺失显 — | **强** |
| chunkActionContracts.test.ts (22) | patch/split/merge/relate 四请求体 camelCase 契约 | 中 |
| digestLabels.test.ts (109) | kind/action 字典与后端白名单**双向**对账（缺失+死键都测）；metric 名 camel/snake/空格归一；targetRef kind 并集+proposal 同义；未知回落 | **强** |
| domainSchemaEditor.test.tsx (52) | create body camelCase（allowed_values 反断言）；必填拦截；edit 回填+schemaId 只读+expectedVersion | **强** |
| exploreNoTenant.test.tsx (139) | **F14 死控件移除**（租户输入框/文案不渲染）；**F17 stale closure**：先成功再失败第二轮错误横幅必须出现；failed 终态文案+ES closed | **强** |
| knowledge.test.tsx (347) | 一体化壳三模式按钮+active 转移；SR-125 Chat 派工提交 sourceTurnIndex+candidateHash+digestSelection（兼容 cardIds 仍发）；D9 字段表 create camelCase/文案不再承诺只读/SR-056 多版本 edit/activate/delete 全绑 expectedVersion | **强** |

**llm-providers/llm-providers.test.tsx (310)**：列表渲染中性协议文案+徽章+effective 值来源；空态；**激活配置未测试禁存**；测试通过+confirm 后 PUT 带 expectedUpdatedAt/activeUpdateConfirmed/activeUpdateTestToken 三件套；**任何编辑作废已获批测试**；清空重试字段测试与保存都发显式 null；视觉指派不可删/取消勾选被 alert 阻止；原子改派 confirm+POST body。**强**。

**operations/operations.test.tsx (496)**：渲染/挂载 load/空态/取消与立即复核传冻结实体；F15 loading 态 + 真实 store loading 生命周期（importActual）；C10 事件 detail 可展开/无 detail 不渲染；formatScores 动态遍历（隐私维度不丢、未知 key 回落）；C8 拦截四分支（finalReviewStatus>holdCategory>「拦截」>通过）；C6/C9 runs 档位遥测（missingTier 派生+展开三字段）；**表头列数=行单元格数**；阶段区块不复用 flex 容器类+stageTable 类（布局回归）。**强**（store/accountStore 为手写 mock 形状，见 §2-C1）。

**overview/overview.test.tsx (43)**：托管计数/运营流/空态三冒烟。**弱-中**。

**products-deals/（2 个）**：ContactPicker.test.tsx (59)——选好友回调 Contact；**切号同步清除选中**（表单消失+零 POST）。SuspectedDeals.test.tsx (78)——F23 approve POST；驳回内嵌原因必填+reject body。**强**。

**quality/（3 个）**：EvaluationScenariosPanel.test.tsx (125)——列表/新建 POST（默认四公式金标）/多行拆 inboundMessages/删除/active profile 动态公式金标（relationshipHealth: 8.5）。promptConfirm.test.tsx (82)——markers tab 独立内联 save 的三态（needsConfirm 200 不发布→「已核对」→force 重提后才 publish；rejected→强制保存）。quality.test.tsx (82)——壳+四 tab；outcome null 显「—」。前两个**强**，第三个中。

**referral-cards/ReferralCards.test.tsx (54)**：从好友选择回填 wxid+空名联动带入+已选态展示。中。

**system-strategy/（7 个）**：systemStrategy.test.tsx (618)——壳冒烟+各 tab 面板小标题/空态；SR-138 reset 取消零调用+精确短语解锁；taxonomy 新增 body（别名中英文逗号 split）/普通改名只 PATCH label/废弃/恢复/**历史版本行无编辑废弃按钮**；**409 显 info 非 error**（class 定位 .inlineError 缺席消半永真，:286-295）；本地校验拦截；customer_stage 软提示随 kind 消失；D10 维度 participates_in_decision 勾选写回不误删其它字段；D11 coverage 三字段写回；候选分页（20/页 heading 精确匹配防子串）/kind 过滤重请求/批量驳回 2 次 reject+禁用条件/非 pending 无复选框。domainProfileVersions (85)——ActiveVersionsBar 渲染+回滚 POST（ObjectId hex URL）。perRelationshipMode (28) / profileAdvancedFields (28)——editDomainProfile 回填 D8/D7 字段。profileDateWireFormat (77)——**BSON $date 对象渲染不崩**（线上白屏复现，头注直言既有字符串 mock 永远抓不到）。promptConfirm (101)——真实 store 三态+「已核对」解锁+force 重提。taxonomyFlags (129)——D6 两 flag 勾选 create body camelCase true/不勾 false/编辑回显+**只 PATCH 变化的 flag**。整组**强**。

**user-ops/（16 个）**：autonomyProtocolView (42)——三组渲染/空字段不渲染/整组空不渲染。bayesianTrendChart (40)——locked 画线/未占槽不画/空态。configureView (214)——4-tab 切换互斥；记忆 4 分组手风琴默认开第一组；保存运营风格回调；**guide 预览只渲 authoritativeChanges**（MODEL SUGGESTED/READABLE 反断言）；**强确认取消不 apply**。contactsView (156)——三 tab 文案+计数（旧文案反断言）；导入框已移除；启用冻结 contactId+wxid；无 onBatchEnable 降级只读；从池移除 confirm；**预览 [链接] 非 XML**；阶段徽章 labelFor 回落。judgmentBar (33)——finalReviewTone 四组闭集映射+escalationCountLabel 0/null 区分。memoryDetail (89)——溯源徽标（置信/重要）/已弃用显性/字符串旧形态兼容（Vec<String> R11）/coreFactEvictions 归档文案/返回。observeView (84)——健康度 tone class 三色+三下钻回调。personalityPanel (52)——五维+conf=0「证据不足」+snapshots≥2 画 5 线。plannerView (128)——多维度 labelFor 中文/no_dict 灰显原值/无值跳过/仅额外维度仍渲染（守卫上提回归）/conversation_mode 字典/**阶段时间只读停滞时间**（容器时间反断言）+缺失显「时间未知」。poolHelpers (48)——overdueHours 四分支/formatRelativeTime 七档+非法 null+未来时间→刚刚。quietHoursSettings (140)——紧凑按钮/弹窗回填/取消丢弃/保存写全四作息字段且保留其它参数/失败保留输入/关门控禁时间字段/**起止相同报错禁存**/无 draft 显重新加载。referralState (85)——已引荐指示双态。roster (501)——渲染+managed 禁选；批量 body 契约（accountId/source:"roster"/candidates/sharedNote）；**切号竞态**；syncing 显同步中≠暂无好友（双向）；性别文字+sex 透传；分页 60；**非真人折叠区展开只读不可勾**；playbook 下拉滤 AI 草稿；loadRoster 缓存命中零请求/force 重拉/URL force=true；**syncing 每 10s 自动重拉不闪加载中**（fake timers）；本账号身份段有 wxid 才渲；不重复外层标题；共 N 位；**刷新轮询到 serverFetchedAt 变化才收敛**（断缓存不断 toast DOM 防脆）；**刷新轮询不清勾选**。sendHistory (59)——**失败≠空态**（EMPTY_CLAIM 反断言）+成功空态照常+accountId 入参。tagTrustPanel (65)——三层分区+证据条数+编辑保存+confirmedBy 徽标三态（strong_evidence/consolidation/未知不崩不显）。userOps.test.tsx (312)——全 mock 子组件与五 store，仅测三模式路由与 tab 分发（编排层）。除 userOps.test 为**中**（路由级）外整组**强**。

**useGoLive.test.ts / trustTypes.test.ts**：见 §1.6。

---

## 2. 值得注意的测试（逐个说明）

### A. 强不变量 / 红线回归锁（价值最高的一批）
1. **ChunkReviewCard verify-gate 真值表**（components/review/ChunkReviewCard.test.tsx:31-113）——「AI 永不自动 verify」红线在 UI 侧的唯一布尔承载，四分支+trim+双拼写逐一锁死；头注明言「同目录唯一承载红线却原本零测试的卡片」。
2. **ManualNewChunk**（features/knowledge/ManualNewChunk.test.tsx:8-14,93-100)——头注直接点出后端命门：create handler 不传 status 默认落 active、D2 闸不管 status，故前端必须写死 draft+needs_review。这是"前端测试守后端缺口"的样本。
3. **applyAiRepairPatch**：`thenVerify:false` 恒定 + `not.toHaveProperty("then_verify")` serde 命门反断言（lib/applyAiRepairPatch.test.ts:33-34）。
4. **useGoLive**：apply→verify 必须用回执的新 updatedAt；apply 回执缺版本 fail-closed 零 verify；空白令牌零请求（features/useGoLive.test.ts:12-73）。
5. **EvolutionCenterTab ConfirmModal**：逐字 RELEASE，小写/错串/尾空格三变体 disabled 且 fetch 零调用（EvolutionCenterTab.test.tsx:241-278）；coverage 不完整时**拒绝从可见列表伪造聚合指标**（:597-614）。
6. **CampaignCreate 红线**：整组件反断言无「确认推送/立即推送/dispatch」控件（features/campaign/create.test.tsx:83-96），与"真实 dispatch 只在 AI 总控"的产品红线对齐。
7. **DeciderChainEditor**：不提供手动输 wxid（fail-closed）、import 200 空 items 显式报错、stale 闭包用最新 chain（features/ask-human-config/DeciderChainEditor.test.tsx:108-119,172-178,232-255）。
8. **csv 公式注入**：`=+-@` 与前导空白/制表变体全部中和（features/campaign/csv.test.ts:33-49）。
9. **禁词红线在测试断言层的体现**：autonomy.test 断言原始枚举 `held_by_ai_policy`/`blocked_by_safety_guard` 不出现在 DOM、必须是「AI 策略主动暂缓/安全门拦截」（features/autonomy/autonomy.test.tsx:170-174）。

### B. 弱断言 / 覆盖缺口（无空壳，但有薄弱点）
1. **navigationStore.test.ts 仅 13 行**：只测默认值与 setChannel。13 号记录 §2.3 描述的核心复杂度——localStorage `wa.nav.collapsed.v4` 键轮换、LEGACY_KEYS 清理、VALID_GROUPS 白名单清洗、`raw===null` 与 `"[]"` 的语义区分（navigationStore.ts:31-99）——**零测试覆盖**。Shell.test 虽消费 DEFAULT_COLLAPSED 但只测渲染结果，不测持久化读写路径。这是 store 层最明显的缺口。
2. **sendAnalyticsStore 零测试**：14 个 store 测试文件不含它；13 号记录 §5-10 指出的"无 generation 防护"现状既无防护也无测试。
3. **referralCardStore 无直接单测**：仅 ReferralCards.test 经组件间接触达 loadCards/draft；review/toggle/delete 动作零覆盖。
4. **ui 冒烟组**：Avatar/EmptyState/MetricCard/PlanStep/StatusLine 每个 1-2 个断言（渲染出文字+class 片段），protection 价值低但无害。
5. **AutoVerifyPanel 抽审硬下限未测**：14 号记录宣称「取消勾选仍 0.05 硬下限——红线姿态不可被关掉」（AutoVerifyPanel.tsx:55-58），测试只覆盖勾选态 0.3（features/AutoVerifyPanel.test.tsx:20-24），0.05 分支无回归锁。
6. **quality/AutoVerifyTab（自由参数版）零组件测试**：与 knowledge 版打同一端点但参数口径不一致（14 号 §5-2），两者差异无测试守护。
7. **commandCenter/contentAssets/overview 的前几条 it 为纯文案冒烟**——文案改动即红，但不守任何行为。

### C. 与生产可能脱节的测试（mock 形状手写、类型绕过）
1. **operations.test.tsx / userOps.test.tsx / campaign board 系列用 `vi.mock` 整个 store 模块**，state 形状手写 + `as any`。组件真实、store 假——若 store 真实形状漂移（字段改名），这些测试不会红（TS 检查被 any 绕过）。风险由 store 自身的单测（operationsStore.test/userOpsStore.test 用真 store）部分对冲。
2. **AutonomyOutcomesTab / EvolutionCenterTab / knowledge 系列 stubGlobal fetch 手写响应体**：响应形状是测试作者对后端的转述。AutonomyOutcomesTab 头注声明与后端 `tests/outcomes_autonomy_endpoint.rs` 一一镜像（:2-9）、EvolutionCenterTab 的 runtime-flag `.flag` 内层结构注明「复刻后端真实结构」——有对齐意识，但无 fixture 机制强制同步（对比 contracts/ 的 bless 机制）。**前端组件级测试与后端之间没有 39 值 gateway fixture 那样的自动对账**，是体系性弱点。
3. **knowledge.test.tsx SR-125 断言 `body.cardIds` 仍在发送**（:191）——这是兼容字段，测试锁定了「新旧双发」过渡态；后端删除兼容读取时此断言需同步改。
4. **jsdom 局限被诚实标注**：AskHumanLayout（chip 折行/间距需目视）、adminGovernance（colgroup 代 CSS 列宽）、DeciderChainEditor（toast z-index 遮挡需目视）——测试自知只能锁结构前提，不锁视觉结果。

### D. 事故复现类（测试当回归史书写）
- DigestCanvasInsecureContext（crypto.randomUUID 缺失，头注：「既有的批量派工用例全绿也挡不住」）、uuid.test、clipboard.test（HTTP+IP 宿主）、profileDateWireFormat（BSON $date 白屏，头注点名既有测试用字符串 mock 永远抓不到）、exploreNoTenant F17（stale closure 抑制错误横幅）、LlmErrorBanner（TypeError 冒充上游故障）。六个文件共同特征：显式摘除 jsdom 能力复现生产宿主。

---

## 3. 测试与 13/14 号记录的交叉验证结果

**总体结论：未发现测试断言与 13/14 号记录的直接矛盾；以下为逐项一致性核对与三处补充。**

| # | 13/14 号记录断言 | 测试侧证据 | 结论 |
|---|---|---|---|
| 1 | 13 §2.7：38 契约 + fixture 双向对账，7 个契约测试覆盖面无遗漏 | 本次实读 7 个契约测试 import 面与对账函数，与记录逐一吻合 | **一致** |
| 2 | 13 §2.7/§5-3：`domain_profile`/`knowledge_chat_turn`/`worker_control` 三个 fixture 前端无对账 | 137 个测试中无任何文件 import 这三个 fixture | **一致**（缺口仍在） |
| 3 | 13 §5-1：`wa.authed` 只写不读；§5-2：`openEventSource` dead code | 两者均零测试（dead code 无测试属自然） | **一致** |
| 4 | 13 §5-5：`disableAgent`/`analyzeProfile`/`runMemoryConsolidation` 仅判 selected、`clearReferral` 无谓词 | userOpsStore.test.ts:464-474 恰好锁定现状：beforeEach 清空 selected 后 `clearReferral("C1")` 仍直接 POST 成功——测试**固化**了"无谓词"行为而非纠正它 | **一致**（且测试把疑点行为写成了预期） |
| 5 | 13 §5-6：sendAnalyticsStore / loadCampaigns / strategyStore 无 generation | sendAnalyticsStore 零测试；campaign store.test 只给 loadReport 写竞态测试、loadCampaigns 无；strategyStore 测试不含竞态 | **一致**（测试分布精确对应防护分布） |
| 6 | 13 §3.5：竞态防护四层梯度 | 梯度第 2/3 层的每个 store（contact/content/operations/inbox/userOps/campaign.loadReport）都有 deferred 双 promise 竞态测试；第 1 层（无防护的 workspace 级 store）无竞态测试 | **一致**（测试密度与防护梯度同构） |
| 7 | 14 §2.11：AutoVerifyPanel「取消勾选仍 0.05 硬下限」 | 测试只覆盖 0.3 勾选态，0.05 分支无测试 | **记录宣称行为存在但无回归锁**（补充缺口，见 §2-B5） |
| 8 | 14 §2.15：evolution 门控三态（env 硬锁/flag 关仍拉历史/开=100%） | EvolutionCenterTab.test:521-595 三态全覆盖 + F-008 注释同源 | **一致** |
| 9 | 14 §2.4：JudgmentBar finalReviewTone 三色分组 | judgmentBar.test:7-24 分组逐值与记录相同 | **一致** |
| 10 | 14 §2.18：OutboxPanel cancelReason 后端强制非空、CANCELABLE_STATUSES 外藏按钮 | OutboxPanel.test:86-95 断言 cancelReason 存在；:131-168 断言 in_flight+cancelRequested/delivery_unknown 无取消按钮 | **一致** |
| 11 | 14 §5-1：labels.ts `REVIEW_CATEGORY_LABELS` dead code 且与 steward 同键不同文 | digestLabels.test 只测 digest 标签；REVIEW_CATEGORY_LABELS 零测试 | **一致**（dead code 无消费也无测试） |
| 12 | 14 §5-2：`yuanToCents` 双份实现行为略异 | SuspectedDealReviewCard.test:55-56 测 isSafeInteger 版（"1.10"→110、" "→null）；products-deals 版无独立单测 | **一致**（双份漂移风险仍无对账测试） |
| 13 | 14 §3.1：ReviewQueue「generation+acceptedIds 双守卫」 | ReviewQueue.test:118-154 直接构造旧 generation 闭包被拒 +「列表已刷新」文案 | **一致** |
| 14 | 14 §2.7：ask-human 9 源 SOURCE_META、chip 三保留规则、total=null 不显 0 | AskHumanLayout/dataSource 两文件全覆盖（含 9 源计数键） | **一致** |
| 15 | 13 §2.3：commandStore confirm/reject 五条前置校验 | commandStore.test 锁定其中两条（账号漂移、缺 planHash）；其余三条（result 存在/id 匹配/updater 内二次校验）无独立测试 | **部分覆盖，无矛盾** |
| 16 | 13 §2.6：usePromptSaveConfirm requireText「已核对」 | system-strategy/promptConfirm + quality/promptConfirm 两处真实 store 三态 + placeholder「已核对」断言 | **一致** |
| 17 | 14 §2.4：guide preview `requiresStrongConfirmation` → confirm 后带 confirmGlobalImpact | configureView.test:181-213 锁定取消分支（confirm 返回 false 不 apply）；userOpsStore.test:294-321 锁定 body confirmGlobalImpact:false 默认 | **一致** |
| 18 | 13 §2.5/契约：`OperationHealth` 7 canonical items、Risk 反量纲 | userOpsHealth.test 全锁（含旧 4-key 反断言） | **一致** |

补充发现（记录未提及、测试揭示）：
- **profileStore BIZ-1 键名契约测试**（stores/profileStore.test.ts:23-42）为 13 号未记录的回归锁：后端 `domain_signals.rs` 写 `domain_attributes.customer_stage`（snake 内层键），曾有裸 `stage` 读取 bug。
- **operations.test.tsx F15**：用 `vi.importActual` 混真 store 测 loading 生命周期——同文件内 mock 与真实 store 并存的少见写法。
- **roster.test「刷新轮询不清勾选」**（:476-500）与 14 号 §2.4 RosterView 注释里"不能走 refresh 否则清空勾选草稿"的实测坑一一对应，注释里的坑都有测试。

---

## 4. website 页面编目与能力宣称清单

**站点概况**：纯静态双语站（`data-lang-zh/en` 双块就地切换，shared.js:9-27），品牌 WeAgent，11 个页面 + 2 个 JS + 12 个 CSS + sitemap/robots。页脚自述「构建于 2026-06-28 · 由 Claude Code 根据项目代码生成 · by opus-4.8 · 11 万行 Rust · 比例 100%」（index.html:678）。CTA 全站为复制微信号 `agimeme`（shared.js:137-148）。`website/docs/` 下 5 个 md 为 observability 频道的 superpowers spec/plan 搬运副本（非对外页面）。

### 4.1 页面编目

| 页面 | 行数 | 主体内容 |
|---|---|---|
| index.html | 688 | 英雄区（一个 Agent 替你经营 + 决策卡动画）、四能力横条（自运营/自运行/自治理/自进化）、人 vs AI 对比、"比人更细"4 卡（知识沉淀/记忆卡/贝叶斯大脑/大五+关系分化）、痛点速览 5 卡、无人工接管 3 卡、能力总览 6 卡、四支柱、认知内核 4 卡+诚实置信横幅、一条消息旅程 6 步、真实大模型测试（10 域 chips + 350+ 基线 + CI 双门）、行业预览、CTA |
| solutions.html | 990 | 人肉承载→系统承载论点、5 痛点大卡（记忆卡/购买生命周期/承诺跟进/冷客户重激活/离职带走）、千人千面、经验知识资产化 4 卡+飞轮、6 价值卡（情绪价值公式/画像/拟真/低压/驾驶舱/模拟验证）、微信聊天节奏 4 点（等你说完~4 秒/分条回/重想/分寸）+多模态与去重注、自运转 4 步、掌控面板 4 卡、辅助模式横幅、主动维系 3 卡（纪念日/复购节奏/沉默唤回）+防骚扰护栏、生命周期 5 节点+到期提醒、防闯祸 4 卡+防封号诚实声明、数据归属 4 卡（私有部署/多租户隔离/手标权威/成本封顶）、目标人群、诚实边界横幅 |
| product.html | 442 | **18 个管理频道**按运营 8（含群/朋友圈两个「下一阶段」占位）/知识 3/系统 7 分组逐卡展开、用户运营双模视角、统一收件箱 **8 流**详解 |
| agents.html | 679 | **20+ 生产 LLM Agent** 6 编队逐卡（prompt key 级：回复/评审/初始画像/反应分析/记忆固化/知识问答/缺口追问/对话补库/修复提案/自动核验辅助/导入标签/日报×2/playbook 生成/画像草案/管理编排/Prompt 红线评审/请示解读/Prompt 批判）、3 确定性调度器（战略规划器/任务 worker/发件箱派发器）诚实区分"调度不是 LLM"、行业通用化（一个引擎+配置）、完整 relay、CI 质保 Agent 一句带过 |
| technology.html | 699 | 单进程架构、agent 9 模块卡、**网关 10 步流水线**、渐进三档加载+恒注入铁律（doNotDo/commitments 任何档不丢）、认知隔离评审（9 自我推理字段剥离）+可选双模、三道闸门+**默认阈值表**（FactRisk≥6 拦/Pressure≥7 拦/ProductAccuracy<7 拦/HumanLike<6 改写/EmotionalValue<6 改写）、方法论公式 4 条（LLM 读非 Rust 算）、认知科学 4 卡（各带"坦诚说明"）、真实大模型测试 10 域、演化 4 步、知识双写（chunk_revisions sha256）、规模数字（18 频道/1 卡每人/4 自治/7×24） |
| engineering.html | 448 | 技术采购三关切（私有部署 OPENAI_BASE_URL/CI 三门/成本记账）、5 组工程卡共 23 张，**每张带 file:line 引用**（llm.rs 三层 JSON 修复/tool_use 劫持/传输硬调优/退避/可重试分类/预算；outbox 合成幂等键/落库失败不重发/崩溃先核对/最小间隔抖动/四节制门/任务 CAS；知识降权不删/召回缺口转整改/置信随成交换血/结构化只提案/摄取熔断/bigram 重排；记忆 OCC/无证据归零/压缩保核心/reaction 抢锁/无标签联合升级；run envelope 先落记录/闭集生命周期/40 维日志/逐调用记账/评审快照） |
| evolution.html | 571 | 物理隔离（check-evolution-isolation CI 扫描 + 四禁引用清单）、九步流水线（默认 6h tick）、两类候选+**闸门基线与目标区间表**、影子回放（阈值已实装 / **提示词 `prompt_replay_not_implemented_w3` 坦诚未完**）、显著性四门（30 样本/+0.05/0.30 失败率/**0.0 零容忍安全回归**）、发布(Mongo 事务原子)/自动放量(双闸默认关、仅阈值、336h 回看、1条/tick)/回滚永远手动（Req 9.7）、9 类 agent_events；页头 pill 明示「默认全部关闭」 |
| observability.html | 471 | 四层透视：单次决策回放（6 步时间线+为何发没发/引用溯源/评审分+改写前后/自省+成本）、看懂客户（三层可信度：人定/有据/参考只记录 + 大五 + 走势 + 记忆卡 + 0-100 健康分 7 格 +「AI 永不自宣成交」）、系统态势 6 卡、AI 分寸层（请示 6 步回放/裁决类型/名片引荐可观测/总控 dry-run） |
| trust.html | 502 | 第一红线 3 卡（永远面对 AI/幕后请示/辅助模式受控例外）+ AI 内部状态名 3 chips、**五道发送闸门**（硬拦 2 + 先改写 3——注意此页把 PressureRisk 归入"先改写一次"列）、grounding 双卡（无据拦截/AI 永不自动核验）、信任三层（manual_tags 物理隔离/confirmed 挂证据/观测层永不驱动 + 两条铁律含后端测试名 `bayesian_and_personality_do_not_affect_planner_filters`）、4 类审计留痕、fail-soft 双卡（默认 30000 token/6 调用）、双键隔离、**CI 门禁 2 道**（措辞门+基线门 ≥350/≥33） |
| scenarios.html | 515 | DomainProfile 通用化（`__default__` 与销售域字节等价+护栏测试）、8 个可配置维度格、行业示例 6 卡（`sales_with_relationships`/`emotional_companion_minimal` 标"示例画像"、其余标"可配置"）、数字分身两正交轴（触达轴 per_relationship 默认 None 护栏/口吻轴 custom_agent_instructions 末位最高优先）、关系识别 4 步链（识别≠生效）、辅助模式 4 步+**实现状态坦诚**（名片调用已落地、MCP 入参契约待 tools/list 确认）、请示通道 3 类+转述链、多租户双键 |
| 404.html | 125 | 品牌化 404 |
| assets/shared.js | 161 | 双语引擎（localStorage `weagent-lang`）、导航、IntersectionObserver 入场、data-count 数字滚动、微信号复制（两级降级） |
| assets/i18n.js | 23 | 规模数字字典（自称"经代码核实"）：250+ 端点/57 集合/28 迁移/11 worker/66 配置项/100+ 模型/11 万行 Rust/105 组件/107 测试文件 |

### 4.2 能力宣称清单（合并去重后 32 项，标注实证状态）

**✅ = 亲验代码属实且默认可用；⚙️ = 属实但默认关/需配置；⚠️ = 与代码有出入；⏳ = 占位/未完（站点多已自标）**

1. ✅ 客户只跟 AI 对话、请示非接管（`relay_principal_decision_to_customer`，trust/scenarios 多页；escalation 通道 09/05 号记录亲验）
2. ✅ 统一发送网关 10 步、绕过即 bug（`run_user_operation_gateway`；与 CLAUDE.md/01 号记录一致）
3. ✅ 独立评审认知隔离 + 五维阈值（Fact≥6/Pressure≥7/HumanLike<6/Emotional<6/ProductAccuracy<7，与 CLAUDE.md 及 `evolution/threshold.rs:296-299` 基线一致）
4. ✅ 幂等发件箱 + 合成键按天去重 + 崩溃先核对再补发（`outbox.rs:543 compute_synthetic_key`、`outbox_dispatcher.rs:2534 mcp_already_succeeded`）
5. ✅ 渐进三档加载 + 恒注入 doNotDo/commitments（04 号记录亲验）
6. ✅ 长期记忆卡 candidate→consolidate→compact + OCC + 无证据归零（memory.rs；06 号记录）
7. ✅ 贝叶斯信号/大五人格 观测旁路永不驱动行为（trust 页引用的后端测试名真实存在）
8. ✅ 意图轨迹驱动换策略（50 项滑窗；06 号记录）
9. ✅ 知识 Wiki 双写 chunk_revisions(sha256)+AI 永不自动核验（07 号记录；前端 6+ 处落实见 14 号 §4.3）
10. ✅ 渐进式召回 catalog→slice→cite、无向量库、bigram 重排（knowledge_agent.rs）
11. ✅ 多模态入站识别（webhooks.rs:1889-1907 图片/语音/视频/名片/链接分类 + media_ref 提取）
12. ✅ 消息去抖合并 + 分条回复 + 新消息重想（config `message_debounce_window_ms`、`agent_reply_max_segments=4`、gateway should_abort_send）
13. ✅ 微信平台重推原子去重（webhook 唯一索引；03 号记录）
14. ✅ 最小发送间隔+抖动+四节制门（pacing.rs:15 仍准、gateway precheck）
15. ✅ RunBudget 双维硬上限超额降级（30000 token/6 调用与 trust.html 一致=runtime 默认）
16. ✅ 350+ 单测基线 + 禁词 CI + 演化隔离 CI（scripts/check-baseline.sh:28 `LIB_BASELINE=350`、check-no-human-takeover、check-evolution-isolation 均在）
17. ✅ 统一收件箱聚合待办（实际 9 流 > 宣称 8 流）
18. ✅ AI 总控自然语言操作 + planHash 冻结 + dry-run + 高危确认（11 号记录 + commandStore 测试）
19. ✅ LLM Provider 热切换 + 激活配置强制测试门 + 视觉指派原子替换（llm-providers 测试全锁）
20. ✅ 多租户 workspace+account 双键隔离（12 号记录 auth/middleware）
21. ✅ 手标 manual_tags AI 物理写不到（TagTrustPanel 三层 + 后端字段分家）
22. ✅ 私有部署/OpenAI 兼容端点/模型可换（config.rs OPENAI_BASE_URL）
23. ✅ 行业画像 DomainProfile 通用化 + `__default__` 字节等价护栏（10/17 号记录）
24. ✅ 影子模拟对话（simulate_user_dialogue）
25. ⚙️ **自我演化全链默认关**：`EVOLUTION_ENABLED_DEFAULT="false"`（config.rs:7,890 锁常量测试）+ workspace flag 默认关 + 阈值自动放量双闸默认关——evolution.html 页头自标「默认全部关闭」，但 index/product 页的演化宣称未带此注
26. ⚙️ **冷客户重激活/复购到期/纪念日关怀**：planner `scan_reactivation`/`scan_renewal`/`scan_calendar` 已实装（planner/mod.rs:1674-2304），但 `ReactivationMode/RenewalMode/CalendarMode::default().enabled=false`（mod.rs:3904,3916,4010 根护栏测试），DEFAULT 销售画像零扰动短路——solutions 页对"沉默唤回/情感关怀"标了默认关，**index 痛点速览与 solutions 生命周期到期提醒段未标**
27. ⚙️ 辅助模式名片引荐默认关（多页自标 ✓）；**MCP 入参契约占位待确认**（referral.rs:199 `⚠️ 待 server tools/list 确认`，scenarios.html:351-352 自标 ✓）
28. ⚙️ 知识日报 Agent：`KNOWLEDGE_DIGEST_ENABLED` 默认 false（config.rs:741）——agents.html 卡片自标「默认关」✓
29. ⚙️ 双模评审：`REVIEWER_DUAL_ENABLED` 默认 false（config.rs:773）——technology.html 说「可选」✓
30. ⚙️ 自动摄取 ingest worker：`INGEST_WORKER_ENABLED` 默认关（CLAUDE.md/09 号记录）——engineering.html 摄取卡未标默认关
31. ⏳ 提示词候选完整影子重放未实装（`prompt_replay_not_implemented_w3`）——evolution.html:295-296 诚实自标 ✓
32. ⏳ 群运营/朋友圈运营占位（channels.ts:171,183 comingSoon）——product.html 标「下一阶段」✓

---

## 5. 宣称 vs 实际差距表

### 5.1 事实性错误（营销站说错了，需改站）

| # | 宣称（出处） | 实际（代码证据） | 性质 |
|---|---|---|---|
| 1 | 演化闸门基线表 `emotional_value_rewrite` 基线 **"< 5.0"**（evolution.html:266） | 基线 **6.0**（`src/evolution/threshold.rs:299`；与 CLAUDE.md、trust.html:184 `EmotionalValue < 6` 一致） | **数字错误**（站内自相矛盾：trust 页写 6、evolution 页写 5） |
| 2 | 连发合并等待窗口「**默认约 4 秒**、可按行业调」（solutions.html:485-486） | `MESSAGE_DEBOUNCE_WINDOW_MS` 默认 **2000ms**，clamp [1000,10000]（`src/config.rs:36-37,490-492`） | **数字错误**（宣称约 4s 实为 2s；另 DomainProfile 有 debounce_window_ms_override，但默认值就是 2s） |
| 3 | trust.html 页头「**2 道 CI 门禁**」（trust.html:83,410-411） vs engineering.html「三道 CI 门禁」（engineering.html:111-113） | scripts/ 实有 **≥3 道正式门**（check-baseline、check-no-human-takeover、check-evolution-isolation）+ 多个辅助 lint（check-no-model-hint、check-secrets、check-ci-gate-policy 等） | **站内互相矛盾**，trust 页低估 |
| 4 | trust.html 把 `PressureRisk ≥ 7` 归入「先改写一次」列（trust.html:188-193） | CLAUDE.md 红线与 technology.html 阈值表均为 **PressureRisk ≥ 7 直接拦截**（technology.html:393 也标 Block） | **站内互相矛盾**（trust 页把硬拦闸描述成软闸） |

### 5.2 营销站落后于代码（低估/过时——站点 2026-06-28 生成后代码持续演进）

| # | 宣称 | 实际 | 备注 |
|---|---|---|---|
| 5 | 「18 个管理频道」（product.html:75-84、technology.html:622） | **20 个频道**（`frontend/src/app/channels.ts` 20 项，含 2 占位）；营销站漏了真实存在的「活动 campaign」与「账号管理 accountManagement」 | 少宣称 2 个真频道 |
| 6 | 统一收件箱「8 类待办」（index.html:377-378、product.html:139,353-368、solutions.html:594-596） | **9 源**（`features/ask-human/index.tsx` SOURCE_META 含 suspectedDeal；后端 `ask_human_inbox.rs:357` suspected_deal）——「疑似成交」流缺失于宣称 | F23 后加 |
| 7 | 「11 万行 Rust · 比例 100%」（index.html:678 等全站页脚；i18n.js backendLoc=11万） | `src/` 实测 **192,582 行**（≈19.3 万） | 过时约 75% |
| 8 | i18n.js 规模数字：250+ 端点 / 57 集合 / 28 迁移 / 107 测试文件 | 路由注册 470 处（`.route(` 计数）；typed collection accessor 65 个；迁移 **58** 个；仅前端 `__tests__` 即 **137** 文件 | 全面过时（i18n.js 自称"经代码核实"已失效） |
| 9 | engineering.html「真实 file:line 可核对」的行号 | 抽验 8 处：`pacing.rs:15` ✓ 仍准；其余全部漂移——llm.rs:318→实际 :395（parse_or_repair）、llm.rs:1153→:1487（detect_tool_use_hijack）、llm.rs:1049→:1356、llm.rs:1026→:1327、outbox.rs:399→:543、gateway.rs:2752→:5056、outbox_dispatcher.rs:645→:2534 | **函数全部真实存在**，但"逐行核对"承诺因代码演进失准 |

### 5.3 宣称能力"已实装但默认关闭/需显式配置"（对外承诺与开箱体验的差距）

| # | 能力 | 默认状态（代码证据） | 站点是否自标默认关 |
|---|---|---|---|
| 10 | 自我演化全链（experiments/候选/发布） | `EVOLUTION_ENABLED_DEFAULT="false"`（config.rs:7,890）+ workspace flag 默认关 + 阈值自动放量需双闸（evolution.html 自述默认都关） | evolution.html ✓ 页头明标；**index.html 四支柱/能力总览、product.html 演化中心卡未标** |
| 11 | 冷客户重激活（index 痛点卡「解：冷客户 AI 主动重激活」） | `ReactivationMode::default().enabled=false`（planner/mod.rs:4007-4011 根护栏测试「销售 DEFAULT 域绝不默认开」） | solutions 沉默唤回卡标「默认需开启」✓、诚实边界段总括 ✓；**index.html 痛点速览未标** |
| 12 | 服务到期续费提醒/过期挽留（solutions 生命周期段） | `RenewalMode::default().enabled=false`（planner/mod.rs:3913-3922），需行业画像或 per_relationship 显式开 | **该段落未标默认关**（页尾诚实边界仅泛称"主动重激活等能力默认需要显式开启"） |
| 13 | 纪念日/生日关怀 | 已实装（scan_calendar + AnniversaryEntry 结构化解析，planner/mod.rs:1674-1957），`CalendarMode::default().enabled=false`（:3901-3909）且需 memory_dimensions 配 date_dimension 槽 | solutions 护栏段「情感关怀类触达默认关闭」✓ |
| 14 | 名片引荐（辅助模式） | 调用已落地但 **MCP 入参字段名占位待 server tools/list 确认**（`src/agent/referral.rs:199-204`） | scenarios.html:351-352 ✓ 坦诚（唯一标注了"待确认"的页面；index/product/trust 的辅助模式宣称未提） |
| 15 | 提示词候选影子重放 | 未实装（`prompt_replay_not_implemented_w3`），提示词只走管理员确认路径 | evolution.html:293-299 ✓ 坦诚 |
| 16 | 知识日报（digest）定时生成 | `KNOWLEDGE_DIGEST_ENABLED` 默认 false（config.rs:741） | agents.html ✓ 卡片标「默认关」 |
| 17 | 双模型交叉评审 | `REVIEWER_DUAL_ENABLED` 默认 false（config.rs:773） | technology.html「可选」✓ |
| 18 | RSS/HTML 自动摄取 | `INGEST_WORKER_ENABLED` 默认关（CLAUDE.md；09 号记录） | engineering.html 摄取卡**未标** |
| 19 | 群/朋友圈运营 | comingSoon 占位（channels.ts:171,183），无代码路径 | product.html「下一阶段」✓ |

### 5.4 小结

- 营销站整体**诚实度较高**：评审阈值、红线机制、演化约束、"AI 永不自动核验"等核心宣称全部与代码一致；对未完成项（prompt 影子重放、名片 MCP 契约、群/朋友圈）多有主动坦白，且方向罕见地是**低估**（频道数/收件箱流数/代码量/CI 门数都少报了）。
- 真正需要修的对外错误共 4 处（§5.1）：emotional_value 基线 5.0↔6.0 站内矛盾、debounce 4s↔实际 2s、CI 门数 2↔3 站内矛盾、trust 页 PressureRisk 硬拦被写成先改写。
- 对外承诺与开箱体验的最大落差在 §5.3-10/11/12：**「自进化」「冷客户重激活」「到期续费提醒」在默认安装（销售 DEFAULT 画像 + 默认 env）下全部不产生任何行为**——首页与产品页的呈现会让读者以为开箱即用，只有内页（evolution/solutions 局部）做了限定。

---

## 6. 覆盖自证

**__tests__（138/138 全文逐行）**：
- 根：setup.ts 35；AutonomyOutcomesTab 292；EvolutionCenterTab 659。app/：Shell 234。
- components/：FriendPickerModal 101；LlmErrorBanner 95；review/ ChunkReviewCard 114 + ProfilePublishCard 144 + ProposalReleaseCard 267 + ReviewQueue 155 + TaxonomyCandidateReviewCard 106 + evidenceMetrics 66；ui/ Avatar 16 + ConfirmDialog 64 + EmptyState 16 + FormDialog 77 + MetricCard 15 + Overlay 71 + PlanStep 12 + StatusBadge 16 + StatusLine 12 + Toast 38。
- contracts/：configPlaybookDomain 58 + evolutionDomain 54 + knowledgeDomain 46 + operationKnowledgeChunk 25 + operationStateAction 14 + operationsDomain 62 + taxonomyDomain 42。
- lib/：apiPatch 39 + applyAiRepairPatch 51 + clipboard 83 + inboxApi 95 + reviewLabels 100 + useSseReconnect 147 + uuid 75。
- stores/：accountStore 27 + commandStore 99 + contactStore 66 + contentStore 135 + inboxStore 172 + navigationStore 13 + operationsStore 161 + profileStore 43 + promptSaveThreeState 107 + publishThreeState 60 + soulVersionIdentity 73 + strategyStore 60 + userOpsHealth 89 + userOpsStore 644。
- features/ 顶层：AnsweringModeGauge 25 + AutoVerifyPanel 28 + CockpitView 55 + CoverageVerdict 34 + ReviewChat 133 + UserOperationCockpit.memory 120 + trustTypes 138 + useGoLive 92。
- features/account-management：AccountLogin 84。ask-human：AskHumanLayout 262 + AskHumanView.dataSource 287 + EscalationInline.labels 28 + InboxRow.collapse 33 + ResolvedEscalations 106 + gapSignalLabels 32 + inline/EscalationInline 92 + inline/SimpleApproveReject 65 + inline/SuspectedDealReviewCard 82。ask-human-config：DeciderChainEditor 256 + deciderCandidates 31 + policyForm 78。autonomy：OutboxPanel 228 + autonomy 176。campaign：board-paging 47 + campaign 136 + campaignStatus 23 + commandJump 29 + create 97 + csv 50 + list 58 + no-refetch-loop 40 + store 175。command-center：McpKeyForm 71 + commandCenter 174。content-assets：contentAssets 177。evolution：evolution 76。knowledge：ChunkInspectorRepairProvenance 118 + ChunkInspectorUnrelate 162 + ChunkRepairPanel 93 + DigestCanvasDismiss 155 + DigestCanvasInsecureContext 121 + DigestCardRendering 142 + DocumentEdit 190 + DocumentRepairPanel 62 + ImportWizardApply 116 + ManualNewChunk 102 + MemoryDrawer 111 + ObservabilityDashboard 223 + ReviewQueueInvalidation 144 + TaskRailList 279 + adminGovernance 186 + chunkActionContracts 22 + digestLabels 109 + domainSchemaEditor 52 + exploreNoTenant 139 + knowledge 347。llm-providers：310。operations：496。overview：43。products-deals：ContactPicker 59 + SuspectedDeals 78。quality：EvaluationScenariosPanel 125 + promptConfirm 82 + quality 82。referral-cards：54。system-strategy：domainProfileVersions 85 + perRelationshipMode 28 + profileAdvancedFields 28 + profileDateWireFormat 77 + promptConfirm 101 + systemStrategy 618 + taxonomyFlags 129。user-ops：autonomyProtocolView 42 + bayesianTrendChart 40 + configureView 214 + contactsView 156 + judgmentBar 33 + memoryDetail 89 + observeView 84 + personalityPanel 52 + plannerView 128 + poolHelpers 48 + quietHoursSettings 140 + referralState 85 + roster 501 + sendHistory 59 + tagTrustPanel 65 + userOps 312。features/trustTypes、useGoLive 已计入顶层。
- **合计 138 文件 / 16,179 行（5,245 + 10,934，wc -l 亲测）全部读完，无抽样。**

**website（页面全读）**：index 688、solutions 990、product 442、agents 679、technology 699、engineering 448、evolution 571、observability 471、trust 502、scenarios 515、404 125 —— 11 页全文；assets/shared.js 161、assets/i18n.js 23 全文；assets/*.css 12 个样式文件未逐行（非宣称载体）；website/docs/ 5 个 md（obs spec/plan 搬运件）结构级扫过；sitemap.xml/robots.txt 非内容。

**源码核验点（§5 引用全部本日亲验）**：channels.ts 频道计数（rg 20）与 comingSoon 行号；SOURCE_META 9 源；ask_human_inbox.rs suspected_deal；config.rs（debounce 2000/EVOLUTION false/DIGEST false/REVIEWER_DUAL false/AGENT_REPLY_MAX_SEGMENTS 4）；planner/mod.rs（scan_calendar/renewal/reactivation + 三 Mode 默认关根护栏 + anniversary 测试群）；evolution/threshold.rs:296-299 基线；referral.rs:199-204 占位；webhooks.rs:1889-1930 多模态；scripts/（三门脚本 + LIB_BASELINE=350 + biz-test 29 个 py）；`src/` Rust 行数 192,582；迁移 58；collection accessor 65；route 注册 470；engineering.html 行号抽验 8 处（llm.rs×4、outbox.rs、gateway.rs、outbox_dispatcher.rs、pacing.rs）。
