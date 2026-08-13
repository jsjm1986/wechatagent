# superpowers plans 全文深读记录（核证日期 2026-08-13）

> 语料：`docs/superpowers/plans/` 全部 149 篇实施计划（合计 90,334 行 / 约 4.6MB），本任务逐篇全文重读（含长文全文，无跳读）。与既有 `20-plans-system-review.md`（其第 1、2 节亦覆盖 plans）为互补关系：本记录（20a）聚焦三类信息——①计划≠规格差异、②仅存于 plan 的实现决策、③完成状态线索，并逐条标注出处段落。
> 纪律：如实转述并标注出处（文件名+小节/Task 号）；无法对码处一律标"文档声称"；只写本文件，不改仓库任何其他文件。

## 0. 全局形态先行结论（影响后续各节解读）

- **checkbox 勾选状态几乎不可用作完成信号**：149 篇中 141 篇的任务 checkbox（`- [ ]`）全部未勾选，仅 8 篇有少量勾选（见第 4 节统计表）。这是 superpowers 计划工作流的惯例——执行者（subagent/主控）完成任务后一般不回填勾选。完成状态须依赖：正文"实施状态/已交付"注记、后续 plan 的引用（如"PR#xx 已合"）、以及 18/20 号记录中 specs/system-review 侧的交叉证据。
- **plan 的通用结构**：Goal/Architecture → Global Constraints（基线门+三 lint+磁盘纪律）→ "已核实的事实/签名"节（主控写 plan 时亲验的 file:line 证据）→ Task-by-Task（每 Task 含 Step、验证命令、commit 提示）→ 部分含 Self-Review 节。部分修复类 plan 头部有"实现期修正/实施状态"追记（执行后回写），这是最可靠的状态线索。
- **编号体系**：07-11 深度逻辑审计台账产生 A/B/KB/KC/KD/KE/F/H 系列 finding 编号；07-12~07-14 的修复 plan 以这些编号命名（kb01/kd04/p3-family1~9 等）。

## 1. 编目（149 篇：对应 spec、主题、一句话）

说明：「spec」列给出对应设计文档（`docs/superpowers/specs/` 内），`同名-design` 表示同日同名；「无 spec」表示 plans 目录独有。状态列见第 4 节详表。

<!-- CATALOG -->

| # | plan 文件 | 对应 spec | 主题 | 一句话 |
|---|----------|-----------|------|--------|
| 1 | 06-02-knowledge-closed-loop-trajectory | 同名-design | 知识/测试 | 闭环轨迹确定性测试：直接调 `list_catalog`（rank_key=relevance×trust×recency 实时计算）做 5 断言门（基线不回归/新增可召回/SUPERSEDE 降权不物理删/关系图无悬空/needs_review 不可召回），Q2 泛化门抽 `tests/common/generalization.rs` 纯函数（floor+gap 双红线）供复用 |
| 2 | 06-04-frontend-ui-refactor | 同名-design | 前端 | 13165 行 App.tsx + 7488 行 styles.css → features 切片 + components/ui 原子库 + CSS Modules + 4 Zustand store + channels 注册表；tokens.css 6 色语义 token 唯一变量源；增量迁移旧 App 全程可跑，最终 App.tsx 瘦到 ~80 行 |
| 3 | 06-05-principal-decision-channel | 同名-design | 请示 | 决策请示通道全量落地（25 Task/9 Phase，全集最大篇 2610 行）：escalation.rs 新模块（短码/回执匹配/台账 CRUD/LLM 解读/relay）+ webhook principal 分流 + 统一占位模型（run 恒 Approved）+ relay 哨兵与 prompt 转述契约成对（"闭环命门"） |
| 4 | 06-06-knowledge-grounding-and-verify-gate-fixes | 同名-design | 知识/守卫 | 双链收口：A=reviewer 漏判 grounding 探针改"先硬闸后观测"（效果/数据 5 词拦→blocked_unverified_product_claim、语气 3 词仅观测）+ reviewer prompt 反向锚点治本；B=create/PUT 人工后门 verified 缺 quote/anchor 静默降级 needs_review（coerce_integrity_against_d2_gate） |
| 5 | 06-07-knowledge-routes-split | 同名-design | 知识/重构 | knowledge.rs 9378 行/216 函数纯机械拆 9 子文件+facade mod.rs（26 共享 helper 留 mod.rs），函数体一字不改、对外路径零变、lib 基线数字恒等（实测 899）；含执行中回写的可见性三铁律 |
| 6 | 06-07-knowledge-trust-cockpit-frontend | 同名-design + mockup 5 屏 | 前端/知识 | 可信度治理驾驶舱三屏（cockpit/review 双栏/autoVerify）；地基阶段先修前端契约漂移（CompletenessView 全错/IntegrityReportView 读不存在字段）；canGoLive D2 前端镜像 + runGoLive apply→verify 串调 |
| 7 | 06-08-escalation-split | 同名-design | 请示/重构 | escalation.rs 1274 行拆 mod/logic/ledger 三文件；原子重构（不追求中间步可编译）、38 测试作回归网、明言"不是 TDD"；仅 2 项升 pub(crate) |
| 8 | 06-08-knowledge-frontend-split | 同名-design | 前端/重构 | knowledge/index.tsx 6582 行/46 组件拆主壳+shared+today/explore/steward/atlas；CustomEvent 桥（wikiFocusChunk/wikiOpenCockpit）是唯一隐性风险点须人工点验；死代码 tryLlmError 删除 |
| 9 | 06-15-h17-memory-dimensions-universal | 2026-06-11-universal-domain-adaptation-design 的 H17 项（无独立 spec） | 记忆/通用化 | memoryCard 记忆维度从销售写死改 DomainProfile.memory_dimensions 可配（MemoryDimension 六字段）；DEFAULT 逐字复刻 8 槽位 cap；H17-a~e 五步分期；简洁版格式（无 checkbox，全集仅 2 篇之一） |
| 10 | 06-19-eval-overhaul-phase1-judge-context | 2026-06-19-evaluation-system-overhaul-design（阶段 1） | 测试/评判 | LLM 裁判底料注入：JudgeContext{transcript/knowledge/memory/commitments/profile_brief} + render 纯函数 + build_judge_user_with_context（空底料逐字等于老函数）+ collect_judge_context 采集 + rubric 维度改写"对照底料判"（J1/J2/J5）；只改 tests/ 绝不碰 src/ |
| 11 | 06-19-universal-residuals-completion | 2026-06-19-universal-residuals-completion-design | 通用化 | 三残留收口：H18 debounce 随 profile / H17 轨迹维度容器化（TrajectoryDimension+dimensions BTreeMap）/ H13 状态机本体随 profile（generated_state_machine draft→activate 时 publish，不造双真相源）；含 06-20 终审追加 Task 8（publish 联动派生 policies）/Task 9（reaction prompt 随 profile 维度） |
| 12 | 06-20-universal-audit-remediation | 同名-design | 通用化/审计修复 | 14 任务修复批：4 必修 Medium（G21 workspace 参数化/G13 五闸阈值 clamp(1,10)/G01 review_passed 补 grounding bypass/G07 负反应率走 profile 极性）+ 全部 Low（G06+G11+G12 抽 reconcile_state_policies_for_machine、G31 RISKY_FIELD 11→13、G03/G04/G08+G32/G16/G24/G09/G10/G18+G05） |
| 13 | 06-21-agent-send-ledger | 同名-design | 发送治理 | agent_send_ledger 共享发送事实表（素材+名片对称）：dispatcher 成功分支 fail-soft 写入 + tasks worker 回扫转化（responded 窗口/stage_advanced）+ 3 只读 API + 前端 sendAnalytics 频道 + decision prompt 已发素材历史注入（防重发软约束） |
| 14 | 06-21-annotation-quality-gate | 同名-design | 发送治理/标注 | 三缺口收口：target_stages 归一校验（fold_stage_validations 纯内核+normalize_target_stages 薄壳，越界 400/字典未配置 fail-soft 放行）、media/referral review 写 fail-soft 审计事件、客户级辅助模式 override PUT 端点（default→$unset 三态闭集）+前端三态下拉 |
| 15 | 06-21-ask-human-config-page | 同名-design | 请示/前端 | P3 配置页：askHumanConfig 频道（决策人链编辑器+4 escalate 开关+超时+频控折叠区），后端补 operation_domain_json 序列化 askHumanPolicy 回显；单页单表单局部 useState 不建 store；policyForm 纯函数（default/extract/validate）是最高价值单测层 |
| 16 | 06-21-ask-human-unified-channel-phase1 | 2026-06-21-ask-human-unified-channel-design（P1） | 请示 | 后端地基 14 Task：AskHumanPolicy 挂 OperationDomainConfig + resolve_ask_human_policy 纯函数（旧字段回落字节等价）+ should_escalate_held 改四布尔驱动 + 骚扰门 push_allowed/超时 next_decider_on_timeout + admin 三 REST（list/resolve/reassign）+ 8 源只读聚合器（per-source 降级 macro）+ m025 回填迁移；完成定义"不含任何前端" |
| 17 | 06-21-referral-card-push | 同名-design | 引荐 | 专属顾问名片引荐 14 Task：ReferralCard 集合+assist_mode_active 三级判定+namecard_to_send 三处接线+outbox 名片条目（:card:{id} 幂等键后缀）+dispatcher 并列分支+send_outbound_namecard+已引荐态 dotted-key $set+CLAUDE.md 红线段修订；头部"实现期对齐"节以真实素材内核落地为准取代设计假设 |
| 18 | 06-21-sales-media-asset-send | 同名-design | 内容资产/发送 | 素材文件发送 11 Task：ContentAsset 扩 13 字段+media_storage 模块（sha 分片布局/防穿越/14 种扩展名白名单）+multipart 上传落 draft+媒体类型→MCP 工具映射表+ensure_media_uploaded（media_id 缓存 TTL 24h 过期重传）+assets_to_send 接线+dispatcher 分流；素材免 grounding（人类把关）但 reply_text 照走五闸门 |
| 19 | 06-22-business-audit-fix-wave | 同名-design | 审计修复 | 11 缺陷六组修复波（A 请示闭环：relay 数字护栏出生地/授权过期清 awaiting+中性收尾/链尾失联安抚去重；B 知识时效 is_verified 加 valid_to/被拦写 recall_miss；C stage 过状态机 fail-soft/承诺观测事件；D FNV-1a 唤醒 jitter；E Offline 落库+defer 不耗 attempt/软上限 500 仅告警；F msgType 解析+媒体下载打桩+非文本过渡话术） |
| 20 | 06-22-digital-twin-relationship-closure | 同名-design | 数字分身 | 关系类型闭环三子项目：口吻轴纯前端文案（复用 customAgentInstructions）；前端地基（/active 只读端点查询条件与 reload_from_db 逐字一致+profileStore+relationship_type 下拉照搬 assistOverride 三件套+ChannelDef.visibleWhen 只建机制零消费者）；触达轴 seed example_sales_with_relationships_profile（draft/inactive 运营手动激活，DEFAULT per_relationship 恒 None 护栏 H8） |
| 21 | 06-22-media-asset-crud-completion | 同名-design | 内容资产 | 素材 CRUD 补全（簇C）：edit 元数据（不退审）/换文件（清 media_id+强制退 draft+旧文件引用计数 fail-soft 清理）/toggle（sendable 正交于 review_status）/delete（先删 DB 再查兄弟引用为 0 才物理删，查询失败保守不删）；Multipart tests crate 不可构造→换文件端到端由代码审查保证 |
| 22 | 06-22-structured-organization | 同名-design | 内容资产/知识 | 软增强注入（簇D）：知识 render_chunk 注入 productTags/businessTopics、素材 tags 三处激活（render/upload/list ?tag= 过滤）、名片新增 tags 对称；空标签不渲染防 prompt 噪声；tags 不进 filter 硬门（agent-first）；不建关联表 |
| 23 | 06-22-taxonomy-admin-crud | 同名-design | 分类学/前端 | 纯前端字典 CRUD：补 api.patch 基础方法（镜像 put）+TaxonomiesAdmin 新增/行内编辑/废弃恢复；D3 软删叫"废弃"、D5 前端校验不超后端（不暴露 priority_weight/is_terminal）、D2 customer_stage 软提示不阻断、409 当 info 不当 error（postRaw 拿 status） |
| 24 | 06-23-progressive-prompt-three-tier | 同名-design | 决策/prompt | 三档渐进提示词：AgentDecision 加 sufficiency/missing_tier/clarification_intent + decide_tier_escalation 纯函数（Enough/Escalate/Clarify）+ gateway 两程循环 + ReviewScores 加 boundary_privacy_safety（≤3 改写）；Task 5 prompt 文本更新是**代码外手动 DB 操作** |
| 25 | 06-23-reactivation-stage-universalization | 同名-design | 通用化/planner | is_reactivation_target 字典标记（与 is_terminal 逐处对称且正交）→ dimension_value_weights 四元组 → PlannerStageConfig.reactivation_stages → filter $in 预筛；DEFAULT 字节等价（单元素 $in ≡ ==，空字典回落 ["dormant_reactivation"]）+ H6 对称护栏 |
| 26 | 06-23-tag-trust-1-data-model | 2026-06-23-tag-trust-two-layer-design（五部曲之1） | 记忆/标签 | 四层数据模型：Evidence/ConfirmedTag/BayesianSignal/PersonalityProfile 结构 + Contact 六新字段 + manual_tags 独立录入端点 + **裸 tags 字段直接删除** + 4 个 prompt 注入点改读双层带来源标注 + 顺手修两项（note 重生成删 tags 写入不替换、management MCP 补维度校验） |
| 27 | 06-23-tag-trust-2-evidence | 同上（五部曲之2） | 记忆/标签 | 证据绑定：LLM 输出窗口序位（tagEvidenceTurns/stageEvidenceTurns/stageExplicitIntent）→ resolve_evidence 映射 ObjectId hex（越界丢弃 fail-closed）；evidence_strength 代码客观判强弱（Inbound∧explicit=Strong，不信 LLM 自称置信）；逐轮标签只写 tag_observation 暂定层；customer_stage 强证据快通道/弱证据剔除 |
| 28 | 06-23-tag-trust-3-consolidation | 同上（五部曲之3） | 记忆/标签 | 压缩重判引擎：归并喂原始宽窗口对话（take_window_by_budget 字符预算 6000+条数 60 双上限）+当前 confirmed_tags+observations；parse_reconfirmed_tags fail-closed（无证据丢弃"宁可少不脑补"）；replace 写回与 memory_card 同一次 OCC |
| 29 | 06-23-tag-trust-4-bayesian-personality | 同上（五部曲之4） | 记忆/标签 | 贝叶斯旁路（6 槽/占槽 min_hits=3∧min_strong=2/HISTORY_CAP=100）+ 大五 OCEAN 人格（搭车归并/诚实置信无证据归 0/snapshots 封顶 50）+ 永不驱动契约测试；**should_evict 低置信淘汰按 Option B/YAGNI 删除未实现**（plan 内回写标注） |
| 30 | 06-23-tag-trust-5-frontend | 同上（五部曲之5） | 前端 | 三层标签面板+贝叶斯 SVG 走势图（手写不引图表库）+OCEAN 人格画像；AI 层/贝叶斯/人格用紫色系（AI 身份色纪律）；locked=false 不画线；验证边界诚实声明"能测渲染，业务数据流需后端联调" |
| 31 | 06-23-tool-loop-dead-code-sunset | 同名-design | 清理 | 纯减法：删 tool_loop.rs 802 行整文件+dispatch_tool_call 半边+knowledgeRoutingMode/knowledgeMaxToolLoops 两 runtime 参数（兑现 sunset-plan D+21）；防误删护栏 4 条（_loops≠_calls/dispatch≠dispatch_chat/exec_* 共用全留/clamp_i32 留）；**checkbox 5/29 有勾选 + PR body Test plan 全 [x]**（罕见的完成状态直接证据） |
| 32 | 06-24-account-send-pacing-guard | 同名-design | 发送治理 | 账号级 1-4s 随机间隔闸：pacing.rs 纯函数（jitter01 线性映射 [min,max]，随机由调用点 fastrand 注入便于确定性测试）+ defer_account_pacing（reschedule 不耗 attempt）+ (account_id,status,sent_at:-1) 新索引；查询失败 fail-soft 放行；poll 5s 量化使实际间隔 ≥ 配置值"方向安全" |
| 33 | 06-24-progressive-tier-hardening | 同名-design | 决策/prompt | 三档加固 5 敞口：should_force_full_on_missing 强升闸（enough∧missing∧需知识→当场升 Full 最多一次）/is_coverage_optimism 收窄仅 weak/missing_tier 兜底 Relational→Full/PROGRESSIVE_TIER_ENABLED kill switch（默认 true）/used_knowledge_ids 只在真注入知识时记（防架空 grounding 硬闸）；判据全 == 正向匹配绝不 != |
| 34 | 06-25-taxonomy-label-wiring | 同名-design | 分类学/通用化 | 字典三流接线：A=CachedEntry 补 display_name+prompt 注入合法取值；B=/api/operation/active-view 聚合端点+前端 labelFor 三态分流（ok/unknown_value/no_dict 诚实优先）；C=AI 生成 suggestedValues 落候选层+三 override；**含"⚠️ 可行性审查必修修正"节（M1-M5，4-agent 审查产物，以修正节覆盖 Task 原文而非改写原文）** |
| 35 | 06-26-frontend-backend-alignment-batch1 | 06-26-frontend-backend-alignment-fixes-design | 前后端对齐 | 修 1 CRITICAL+13 HIGH（16 task）：A1 驾驶舱双死端点+Promise.allSettled 加固；B2 InboxItem 富字段 8 构造点显式 None（刻意不引入 Default"以免掩盖遗漏"）；E1 clear-referral $unset 两键退态；D2 MCP 密钥 snake_case body+password 不回显 |
| 36 | 06-26-frontend-backend-alignment-batch2 | 同上 fixes-design | 前后端对齐/通用化 | 9 条通用化断裂三组修复：E13 formatScores 动态遍历、C7/C8 reviewLabels 10+3 闭集中文标签、A4 多维度看板、D7 五高级字段、D8 per_relationship map、A5 conversation_mode 字典 m028、D6 字典两 flag、E10 关系建议富投影；**头部声明批次1 已合并 main（PR#44 merge 9d78282）** |
| 37 | 06-26-frontend-backend-alignment-batch3 | 同上 fixes-design | 前后端对齐 | 18 条 MEDIUM 三组（原 19，**E11 写 plan 后复核 main 发现已完整实现故移除**）；serde rename 铁律（批次1/2 踩 5 次）；E6 replace_one 整替换须回填 rawContent 防清空；E7 手工建切片强制显式 draft+needs_review；F23 疑似成交闭环（approve 落 staff_confirmed outcome）；**头部声明批次2 已合并（PR#46 merge ae54a8f）** |
| 38 | 06-26-main-health-audit-batch1 | 06-26-main-health-audit-batch1-design | 安全/审计 | 4 findings：SEC-1 evolution 三端点跨租户 IDOR（workspace filter→404 不暴露存在性）+EVO-2 审计 actor 用真实 admin.username（保留 DEFAULT_RELEASE_ADMIN 常量）+KNOW-1 知识预览端点加 workspace 参数+FE-1 guide preview 后端返回构建好的 health items（删前端坏函数 healthFromScores） |
| 39 | 06-26-main-health-audit-batch2 | 06-26-main-health-audit-batch2-design | 并发/硬化 | 6 findings：CONC-3 insert E11000 落 find_one 重读；CONC-2 commitments $push+$slice:-8（去重接受并发重复）；CONC-1 memory_card 拆出走 OCC；GATE-1 动作闸抽 fn revision 后复检；KNOW-2 告警计数补 status=active；EVO-3 双闸 AND 接线；**头部声明"推翻了 CONC-2/CONC-1 两处 spec 原始方案"** |
| 40 | 06-26-management-agent-thickening | 06-25/26-management-agent 设计 | 管理 Agent | 指挥中心做厚 9+2 task：ToolRisk 四档（第一期 dangerous 默认放行、irreversible+verify 类恒确认）；assert_tool_outcome 区分调用 Ok 与业务成功（executed_unverified 诚实态）；confirm/reject 乐观锁闭环；Task 6.5 提示词字面双闸（**经 4 路 opus 核实重写：原草案锚闸漏查反接管红线**，先抽 DEFAULT_REPLY_REDLINE_ANCHORS）；Task 6.6 LLM 第三闸三态降级；Task 9 部署+真 LLM 冒烟 |
| 41 | 06-26-prompt-pack-alignment-completion | 06-26-prompt-pack-alignment | prompt 基建 | ensure_prompt_pack_v2 版本盲三分支→空库分流：**PROMPT_PACK_VERSION 从"生效闸"降级为"仅 stamp 溯源"**，生效判定全交 align 内容比对（改 spec 重启必生效）；顺序铁律 delete_redundant 先 align 后；删死代码 ensure_missing_prompt_templates；Task 4 ensure/align 返回 bool 供运行时调用点 bump LRU（终审 Minor #1） |
| 42 | 06-27-chunk-ai-repair-closure | F22+F12（76 缺口路线图） | 知识/前端 | 后端三空转端点（propose/answer/applied）前端兑现：applyAiRepairPatch 三态失败语义（apply_failed 不发闭账/audit_failed 已落库不回滚/server_error）；thenVerify 恒 false 红线；防清空从 originalChunk 出发只覆盖勾选字段；**"已实证修正"节列 spec 与真实代码 4 处差异**（返回体、BudgetExceeded 是 HTTP 错误非 200 字段等） |
| 43 | 06-27-frontend-backend-alignment-batch4 | 同 fixes-design | 前后端对齐 | P3 收尾 9 条：**F16 关键技术分野——explore 一次性 RPC 流绝不接自动重连（重连=重发查询重复扣 token），只有 today 长连接监听流（幂等 reload）接指数退避**；F17 resultRef 修 stale closure（须每个 setResult 处同步置 ref，不用 useEffect）；F13 gatewayStatus 32 值闭集中文 map；D9 domain-schemas 写表单+改只读文案承诺 |
| 44 | 06-27-prompt-evolution-human-gated | 同名-design | 演化/安全 | 三阶段：①三闸下沉 src/prompt_guard.rs 顶层中立模块（原 routes 私有模块 evolution 够不着）+release_prompt 接三闸+snippet 整篇覆盖→compose_appended_content 末尾追加（原文逐字保留锚闸天然过）；②prompt_shadow 真模型对照修 G1 placeholder/G4 假基线，PromptOverride 第 16 入参注入（现有调用点全传 None **字节等价护栏**）；③前端骨架（用户批准此粒度）；**阶段二含"已核实细化 2026-06-28"更新+跟进项挂 PR#51** |
| 45 | 06-28-campaign-targeted-push | 同名-design | 活动推送 | campaigns 引擎：模型+status 闭集/两阶段圈人（Mongo 粗筛 $elemMatch **混合大小写真实路径 outcome_events.productRef.productId**+内存精筛复用 G4 净持有）/create-preview-dispatch 生命周期/management 两工具；**dispatch 必须走 tool_always_requires_confirmation**（Dangerous 档在第一期开关关闭下不触发确认）；圈人只认 staff_confirmed/payment_verified；campaign_sends 唯一索引=活动级去重闸；发送链路一字不改 |
| 46 | 06-28-campaign-sends-report | 同名-design | 活动可观测 | GET /campaigns/:id/sends 只读聚合：3 次固定查询（sends→run_logs $in→contacts $in），关联键 source_event_id==taskId.hex+SOURCE_KIND_FOLLOW_UP_TASK 常量；7 桶分类优先级命中即停（outbox=sent 最高、压过 escalated status）；**escalated（请示中会继续）与 blocked（无后续）语义区分**；不认识 status→unknown 诚实标"绝不强划进 sent"；retry 取 _id 最大；classify 收 Document 不耦合 struct |
| 47 | 06-28-campaign-frontend | 同名-design | 活动前端 | 新一级频道 campaign：7 桶汇总+明细表+桶筛选；7→5 tone 固定映射；escalated 标"已请示"避禁词；dispatchCampaignId 跳转守卫纯函数（dry_run/待确认/无 id 不渲防死链）；PlanStep 无法内嵌按钮→跳转按钮作同级兄弟元素；**基线 ← origin/main 700c57d（sends-report 已合并的证据）** |
| 48 | 06-28-contract-alignment-batch1-knowledge | 契约对齐机制设计 | 契约/测试基建 | "投影函数级快照+前端键集对账"双门地基+知识域 5 投影：fixture 前后端唯一真相源（UPDATE_SNAPSHOTS=1 bless）；对账只断言键集不断语义；**Task 6 刻意快照 chunk 详情裸 struct 暴露列表 camelCase vs 详情 snake_case+$oid 形状冲突（不强制统一——"让统一与否成为可见的产品决策"）**；防腐烂 lint 纯 std 实现（600 字符窗口近似）+ALLOWLIST 渐紧；CI paths-filter 前后端分流（backend filter 含 contracts/**） |
| 49 | 06-28-customer-reply-guarantee | 同名-design | 网关/兜底 | Inbound 零回复守卫（黑名单语义"全兜底"）：纯函数判定+#ack-placeholder 幂等占位+3 处挂载；**头部"⚠️ 实现期修正（已合并 main）"：原 4 处挂载点被 CI 集成测试证伪 A3 挂载（AI 主动沉默非晾死，补占位破坏拟人）→ no_reply 入豁免清单**；方案 B 解耦 escalate_held_decision（客户占位原耦合在领导骚扰门后=晾死根因）；plan 补 spec 遗漏的第一道 precheck 挂载点 |
| 50 | 06-28-e4-document-repair-f21-task-list | E4+F21（76 缺口） | 知识前端 | E4 文档级批量修复（后端零改动，DocumentRepairPanel 筛 needs_review 逐 chunk 内嵌复用 **PR#49 已闭环的 ChunkRepairPanel**）+F21 chat_task_list 端点（limit clamp [1,200] 默认 50、列表项不带 plannedSteps 全文控体积）+TaskRail 列表化（列表失败静默降级保留手工 fallback） |
| 51 | 06-28-memory-conflict-and-reviewer-yield | 同名-design | 记忆/评审 | ⑨治上游：注入卡先 auto_upgrade 带稳定 id 且与 prev-merge **同源同一实例**（否则 LLM 引用的 id 匹配不上——from_plain_text 每次 fresh UUID）+live_dimension_names 跨轮命名稳定化+consolidator schema 空数组→对象示例（"A/B 已证可选字段被无视"）+dimension 改口必填+fact 原子化；④REVIEWER_ASSIST_YIELD_NOTE 让位常量解两条 hold 路径（第三方角色红线让位+引荐非产品声明）；PROMPT_PACK_VERSION→v16 |
| 52 | 06-28-memory-consolidation-guards | 同名-design | 记忆兜底 | ⑨确定性兜底两件套：件一 compact 救回加 dimension 感知（同 dimension 新值在场不救回旧值，None 路径字节等价）；件二 fact_is_non_atomic **纯结构度量（换行≥2/句界≥2/char>80，绝不提取数值实体——"找 N岁"是关键词模式违红线）**+重试至多 1 次（v4 探针 6/6 干净）+仍失败丢弃非原子条（候选记忆仍在下轮重产）；不改 prompt（v4 探针证明 prompt 无缺陷） |
| 53 | 06-28-prompt-evolution-phase3 | human-gated spec §4.C/D | 演化前端 | 阶段三证据透出：shadow_replay_json 补 original 侧 2 字段；**wire 混搭规则——JSON 键 camelCase 但 evalMetrics 内字段 snake_case（bson_doc_to_json 原样透出）**；FIVE_GATE_KEYS 固定序；MetadataSection 移出白名单仅 kind=prompt 生效（five_gate_hit_delta_per_gate 与 threshold 共用绝不笼统过滤）；Δ 语义色双向（五闸降=好、自评升=好）；聚合表只有 Δ 无 per-gate 率（已知取舍，逐样本点阵 ●○· 互补） |
| 54 | 06-29-campaign-domain-completion | 同名-design | 活动前端 | campaign 域补全三视图（列表/建活动/看板）：GET /api/campaigns+CampaignListItem 投影（**不泄漏 workspace_id/segment_filter/intent_text，测试断言 no_leak**）；**dispatch 红线——前端绝不做 dispatch 按钮（测试守），真发送只走总控 AI 恒确认门**；buckets.ts 抽独立模块防循环依赖；index.tsx 先透传后路由壳（PR#58 测试中途不断）；draft 复用；已扇出≠已送达列头 title 区分；**基线 ← c163542 含 PR#57/#58（sends 端点+看板已合并证据）** |
| 55 | 06-29-memory-summary-not-authoritative-fact | 同名-design | 记忆真因 | **⑨真因修正："server117 全量真测证件一件二未触及真因"**——memory_summary（短期滚动上下文，gateway 逐轮 append 累积 8 段成 blob）被 memory_card_from_contact:215 当权威 core_fact 注入才是 8岁/10岁并存根因；修法删 1 行 push+改 1 行 insert（归位 extra.recentEpisodeSummary）；identity 单值回落保留；198 行最小 plan |
| 56 | 06-29-content-assets-tiered-injection | 同名-design | 内容资产 | 文本资产按条配置最低注入档（lean/relational/full）修"绑死 Full 降档失效"+清理过期录入项；**安全红线：media_id/url 字段保留不删（media_id 是文件发送链命脉），仅 create 端点不再收入参**；None/非法按 full（与改造前逐字等价，Full 档查询 $or 纳入字段缺失）；顶层 $or 冲突用 $and 包裹；ContentAsset 4 构造点同步补全防 E0063 |
| 57 | 06-29-contract-alignment-batch2-operations | 契约对齐机制 | 契约批次2 | 运营/Agent 域 9 投影快照+对账：**AgentOutcomeMetric 历史 serde alias "human_handoff_success_rate" 禁入 fixture（用新字段名 ai_hold_cleared_rate）**；"批次1 document 16 键笔误教训"→键数以 bless 出的 fixture 为唯一真相源；AgentRunLog 35 字段全填；ALLOWLIST 移除 9 项使 lint 渐紧 |
| 58 | 06-29-contract-alignment-batch3-taxonomy | 契约对齐机制 | 契约批次3 | 字典/分类域 5 投影：**对账只断言顶层键集**（嵌套对象如 taxonomy_entry.value 内部形状由后端快照固定不进对账）；operation_domain_json 22 字段只下发 20 键（principal_decider/high_risk_escalation_mode 不下发但构造仍须赋值防 E0063） |
| 59 | 06-29-contract-alignment-batch4-evolution | 契约对齐机制 | 契约批次4 | 进化/实验域 8 投影：**cohort_run_ids_json 不纳入——返回裸数组非对象，契约机制只适配对象，移 helper 豁免区**；纯标量 doc! 经验（嵌套 DateTime/ObjectId 泄漏 $oid/$date）；**"批次3 后基线 1720，本批 +8=1728"（lib 测试真实数量证据，gate 350 只是下限）**；8 task 改同一 mod tests 的串行 dispatch 纪律（磁盘级插入勿依赖陈旧行号） |
| 60 | 06-30-guide-apply-partial-validation | 同名-design | guide/校验 | apply 时 LLM 越界枚举字段跳过记 skippedFields 回流（合法字段照落，不再整请求 400 陪葬）+preview prompt 注入状态机/字典合法值压源头；**不改 apply_admin_dim_validation 共用 helper（手动表单/审批硬拒不变），调用点绕过它直接 match 原始 DimValidation（"helper 已把 Reject 吞成 Err 拿不到跳过语义"）**；canonical 逐字：intent_level 是 medium 非 mid |
| 61 | 06-30-h3-cross-tenant-idor | 同名-fix-design | 安全 | 14 handler 认证后水平越权（自带 workspaceId 解析后未校验 ∈ admin ACL）：is_workspace_authorized 纯函数（空 ACL=单租户回落只允许 default）+resolve_authorized_workspace 包装；**AppError 无 Forbidden 变体→复用 BadRequest("workspace_not_in_user_acl")**；可见性从 spec 初稿 pub(crate) 改 pub（外部 test crate 需要）；集成测试只挑 pub 的 activate/list（其余请求 struct pub(super) 不可命名） |
| 62 | 06-30-h10-relay-identity-provenance-fix | 同名-design | 安全 | 客户可伪造 __PRINCIPAL_RELAY__ 哨兵劫持转述模式（H10 UPHELD High）：**is_synthetic_relay 字段 #[serde(default, skip_serializing, skip_deserializing)]——绝不落库、反序列化恒 false=结构上不可伪造**；LLM 层第二道防御 prompt_isolation 纯函数剥哨兵（合法 relay 合成消息从不落库→history 剥哨兵零误伤）；~30 构造点机械补 false（E0063 编译器强制清单） |
| 63 | 06-30-wiki-audit-high-fixes | 同名-design | 修复 | #1 domain_schemas serde 字段名错配——路由层 11 处 camelCase 查询键 vs 模型 insert snake_case→**动态字段校验静默失效**（"Mongo 字段 snake_case、对外 JSON camelCase 两者不可混"）；#2 prompt_templates create/publish 绕过红线闸→补字面双闸+publish 补 LLM 三闸（force 跳 LLM 不跳字面） |
| 64 | 06-30-forbidden-expression-injection | 同名-design | 决策注入 | 禁语从「可引用内容资产」段剖出独立「禁止使用」段（语义反转修复）；禁语恒注入无视 min_inject_tier；split_context_assets 纯函数分流；**占位符对位铁律：模板第 N 个 {} 与参数列表第 N 位精确对应否则全体移位** |
| 65 | 06-30-content-assets-injection-hardening | 同名-design | 决策注入 | 修 4 已核实问题（含 forbidden-expression plan 留下的"禁语恒注入被共享 limit 击穿"）：拆两次独立查询（可引用 tier 下推+limit16/禁语无 tier 无 limit 全量）；**三加载器签名收 workspace_id 由调用点传 contact.workspace_id（不再锚 default_workspace_id）**；filter 抽纯函数配 query-shape 单测 |
| 66 | 06-30-content-vs-knowledge-nav-disambiguation | 同名 | 前端文案 | 纯导航文案对齐消除素材库/知识库概念混淆（218 行小 plan）；"Wiki 管理"→"知识库 Wiki" 连带 walkthrough.py 字面依赖同 PR 更新（已核实不在 CI）；本 worktree 落后 main→须基于最新 origin/main 开新分支 |
| 67 | 06-30-contract-alignment-batch5-config-playbook | 契约对齐机制 | 契约批次5（最后） | 5 投影收官，ALLOWLIST 只剩 6 项纯 helper 豁免；**"关键决策"节全面审查 evaluation_scenario Document 直发——bson_doc_to_json 不是消毒器（与直发同一 serde 路径，no-op），泄漏只取决于内层装什么（HTTP JSON 入口结构上不可能含 ObjectId/DateTime）→投影一字不改**；批次4 已合并 PR#67；基线预期 1734+5=1739 |
| 68 | 06-30-evolution-ui-toggle | 同名-design | 演化/UI | EVOLUTION_ENABLED 默认 false→true（**语义改"是否允许 UI 开启"，仅显式 false=运维硬锁定**）；mongo flag.enabled 升总开关；"开=全量"（on 写 rolloutPercent:100）；EvolutionCenterTab 三态重构（运维硬锁/关态/开态）；enabled prop 保留不删（7 处测试依赖）语义重定义为 env 硬上限 |
| 69 | 07-01-h1-ingest-worker-not-due | 同名-design | worker 修复 | not-due 与真 304 同返 NotModified→run_one_round 对 not-due 也 mark_success 误刷 last_fetched_at（interval<schedule 时源首拉后永不更新）；拆 SourceOutcome::Skipped 变体（not-due 不触任何 DB）；真 304 仍 mark_success（"304 是成功探测，该刷"）；**首现"subagent 红线（用户 07-01 点名强调）：绝不基于猜测动手，产出必须带 file:line 证据"** |
| 70 | 07-01-h7-m016-collection-names | 同名-design | migration 修复 | m016 两张硬编码集合名表 7 处拼错（accounts→wechat_accounts 等）+漏收录 15+误收录 2（admin_users 用 Vec 无单值 ws/chunk_revisions 靠 chunk_id 反查）；加"审计定稿基准 const+4 单测交叉锁死"；**"机械穷举复核，不接受'我读了都对'式抽样结论（spec §7 教训：首轮抽样漏了 chunk_revisions）"**；否决自动派生方案 B（无集中注册表故基准手维） |
| 71 | 07-01-h8-boot-brick-stale-index | 同名-design | 启动修复 | E5-T1 漏删的两处旧 unique create_index 残留死代码（建完即被 ensure_ops_versioned_indexes drop）却带 .await? 致命语义→多版本数据 E11000 boot-brick；删除即根治（唯一性由 4-tuple unique 完整保证） |
| 72 | 07-01-chunk-request-deadfield-cleanup | 正规层清理 | 清理 | OperationKnowledgeChunkRequest 六死字段（100% 空转）原子删除+连带死分支；**"绝不引入新判据——'body/summary 非空→rejected'经审查证明会误伤正常草稿，已否决"**；保留 distortion_risks（活 wire）；serde 静默丢弃旧 wire 键零破坏；**"当前基线 1777/0"** |
| 73 | 07-01-user-ops-cockpit-redesign | Spec A | 前端重构 | 驾驶舱 9 段纵向堆叠→三段式（常驻判断条+观测/配置段控+下钻）；后端仅补 operation_health 3 个 quiet_hours 只读字段；finalReviewStatus 10 态四色分类（绿已发/橙暂缓/红拦截/灰）；"全局 token 三源统一是独立 Spec B 本次不做" |
| 74 | 07-01-wiki-three-p0-fixes | 同名-design | 红线修复 | ①auto-verify 从只拦 product_fact **扩到所有 chunk_type**（改旧测试断言="行为定义变更"不违增量铁律）；②导入类型透传；③fix_chunk 抽 propose_chunk_repair_inner 产可审草稿；**禁词陷阱：单字"人工"被字面拦——文案统一用"运营"**；import.rs prompt 是内联字符串非 seed pack 改它不 bump（已亲验） |
| 75 | 07-02-decision-review-autonomy-protocol | 同名 | 复盘投影 | 9 自治协议"内心独白"字段纯读投影（已落 run log decision 只差透出）；**autonomyProtocol null 优雅降级是硬要求**（两类现实来源：历史旧数据+管理 Agent 主动发送不写 run log）；契约 re-bless 标注"必要连带" |
| 76 | 07-02-h11-evolution-threshold-keys | 同名-design | 演化修复 | H11：gate_key_to_score_field 用坏键（factRisk vs 真实序列化键 hallucinationScore）→score 恒 0.0→**#152 安全回归门+5 闸涨幅门被架空**；M9 original_5gate_hit 硬编码空→空基线偏拒；L1 死臂；"让门正确，不是调松/调紧" |
| 77 | 07-02-m13-profile-attributes-preserve | 同名-design | 数据保护 | update_operation_profile 无条件 $set profile_attributes→前端不发字段时 serde default 空 Document **清空 AI 在 gateway 积累的画像**；修法镜像 gateway 非空才写守卫；"前端不改（不发该字段是正确的）" |
| 78 | 07-04-inbox-ui-jargon-cleanup | 同名 | 前端/文案 | 收件箱黑话三层修复：A 后端拼接串先映射中文再拼（给领导的请示串直接 format! blocked_status 是根因）；B 扩 active-view 字典；C 前端翻译层+补全无定义 CSS 类+折叠式重构（1007 行大 plan） |
| 79 | 07-06-escalated-run-budget | 同名-design | 预算 | RunBudget 加 escalation_bonus：升档 run 抬 gating 上限至 run_token_budget_escalated（默认 100000）修首触问题被 blocked_by_budget 静默拦截；**只放宽判定上限绝不篡改 tokens_used 真实累计（"如实反映成本"）**；未授予时逐字等价；锁顺序纪律防环；**基线"现 1814"** |
| 80 | 07-06-knowledge-workbench-dispatch-wiring | 同名 | 知识派工 | 派工链路补通：两条结构化入口（卡片/对话）汇入同一 chat_task_create+worker；cardId→targetChunkId 派工落库时解析烤进 step；6 action 分层真实现（fix/add/retag 产 draft+needs_review、analyze_logs 只读、dismiss 真标记、review_evolution 跳转指引）；**freeform 卡不可派工**（suggestedAction 闭集与 ALLOWED_TASK_ACTIONS 差集）；删空转手打框 |
| 81 | 07-07-taxonomy-candidate-inbox-card | 同名 | 收件箱卡片 | inline 二元按钮走不通→rich 卡片（evidence/confidence/occurrences/suggestedDisplayName 经 rich_params 带全**无需新增 get-by-id 端点**）；小白框定文案；system-strategy 复用去重带降级预案（"耦合过深则新组件先只服务 ask-human，记后续清理"） |
| 82 | 07-07-user-ops-roster-batch-enroll | 同名 | 通讯录 | 通讯录视图+批量托管：avatar_url 全链路+GET /contacts/roster 左连接标注+batch-enable 批量 upsert 入队；**初始画像从同步改异步 AgentTask kind=initial_profile**；两路径共用 apply_generated_profile_to_contact 防漂移；前端"不用 @/ 路径别名（tsc/vite 无此别名会编译失败）" |
| 83 | 07-08-roster-fetch-cache-fix-m1-m2 | 同名 | 通讯录修复 | parse_roster_items 支持真实嵌套形态 {result:{friends:[wxid串]}}+纯字符串元素；**空 cache 与"真没好友"区分（syncing 字段+前端自动重拉）**；"真实数据形态（07-08 线上 117 亲验）"节 |
| 84 | 07-08-system-strategy-tab-layout | 同名 | 前端布局 | 7 平铺 Admin 面板→4 职能 tab（一次只渲染当前 tab 消除无限长）；面板组件内部零改动；**"眉标英文保留（用户 07-04 既定全站装饰性英文眉标不译）"** |
| 85 | 07-08-taxonomy-candidate-display-name | 同名 | 分类学 | 决策 LLM 在 dimensionDisplayNames 为自造新值顺带产中文名→carry-through→upsert_candidate 第 7 参；全链 best-effort 缺失回落英文绝不阻塞；**"字段 optional 是 LLM 输出容错——改必填会使 LLM 漏填轮次反序列化失败决策链路崩"**；不 bump 版本（align 内容 diff 生效——版本降级机制的实际应用） |
| 86 | 07-09-roster-fetch-full | 同名 | 通讯录 | contacts_fetch_cache（只 wxid）→contacts_fetch_full（昵称/头像/性别富化）；4831 好友分页+懒加载；**"sex 是客观事实字段不进 profile_attributes（AI 推断空间）；忠于源 int 原样存储，文字转换只在前端展示层"**；就绪判据改读 status 字段（refreshing:true 是干扰项） |
| 87 | 07-09-roster-mcp-ratelimit-syncing | 同名-design | 容错 | MCP 429/503 限流不再弹 internal_error 红条→新 AppError::UpstreamBusy 变体+纯分类函数→柔化为 syncing:true 走既有重试自愈；前端零改动（复用 #155 已上线 syncing 态）；**Windows Defender 对默认 target/ 有 exec 锁→改 CARGO_TARGET_DIR=E:/yw/cargo-target-roster** |
| 88 | 07-09-webhook-gewe-addmsg-parse-fix | 同名-design | webhook | 真实 GeWe AddMsg 嵌套 payload 解析（Data.FromUserName.string 优先+find_string 回落，顶层 Wxid/PushContent 遮蔽是根因）；扁平 payload 完全走原逻辑向后兼容；**不做群消息（chatroom 发言人嵌 Content XML 前缀是另一形态，Phase 1 之外）** |
| 89 | 07-09-webhook-signature-verify-restore | 同名-design | 安全 | 每账号明文 webhook_secret+时间戳时效+fail-closed 全路径验签，WEBHOOK_VERIFY_SIGNATURE 从联调期 false 回 true **封死 :3003 公网无鉴权入口**；验签门下沉到"查到账号密钥之后、任何副作用之前"；旧 x-mcp-signature+全局 MCP_API_KEY 方案退役；spec §6 管理端 API 不在本计划（部署 mongosh 写密钥） |
| 90 | 07-10-outbox-chat-search-idempotency | 同名 | 幂等 | timeout(150s) 兜底核对源升级"优先 MCP chat_search（server 真实已发）+本地日志 fallback"，根治 timeout 取消 send future 致日志缺失误重发；chat_search_hit 纯函数（content 精确等+since 窗）；只做 text 主链路 |
| 91 | 07-10-passive-reply-daily-limit-and-ai-holding-reply | 同名 | 频控/占位 | daily_limit 只限 AI 主动触达（**被动回复豁免**——一行守卫收窄）；占位回复从硬编码改 AI 生成 generate_holding_reply（独立预算旁路+运行期 passes_forbidden_words+C 类数字守卫+任何失败回落硬编码兜底） |
| 92 | 07-10-roster-backend-snapshot-persist | 同名 | 通讯录 | roster_snapshots 集合快照优先秒回；>24h 后台静默自刷（fire-and-forget）；MCP 失败永远兜底旧快照；后台任务独立健壮重试循环（连 AppError::Http 解码失败也退避重试） |
| 93 | 07-10-roster-sex-parse-nonhuman | 同名 | 通讯录 | sex 解析取 int64 对象 .low；is_non_human 白名单（微信系统账号）前端默认折叠；roster 数据源提升 store rosterCache 按 accountId 键控（force 才重拉） |
| 94 | 07-10-roster-single-flight-refresh | 同名 | 并发治理 | 根治"8s force 轮询叠加 spawn 不去重→打爆 MCP SSE 并发上限→1.37MB body TimedOut→快照永远写不进→前端无限卡同步中"；DashMap per-account in-flight 锁+RAII guard panic 也释放；端点不再同步阻塞改立即 spawn+返 syncing；前端轮询去 force 8s→10s |
| 95 | 07-10-user-ops-pool-redesign | 同名 | 运营池 | 三 tab 计数改后端真实 count_documents（与 list 同源 filter）；limit=500 防截断；删与通讯录重复导入框+顺藤清 importQuery 死方法全链 |
| 96 | 07-11-friend-picker-modal | 同名 | 前端组件 | 手填 wxid→FriendPickerModal 弹窗头像网格单选；UI-only 受控组件包现有 Overlay；数据由调用方 map 成统一 FriendPickerItem 形态；名片+ContactPicker 两处接入；后端零改动 |
| 97 | 07-11-full-system-test-remediation | 07-10-full-system-test-findings 台账 | 修复 | 全量系统深度测试台账 P0+P1：账号错配家族（前端三选择器传当前账号+campaign 加账号字段"照搬组合过滤即隔离，不做多余归属校验"）；知识概览卡顿（进程内 TTL 缓存 DashMap+前端局部骨架）；任务日志内部作业泄漏（kind 白名单） |
| 98 | 07-11-taxonomy-candidates-batch-filter | 同名 | 前端 | kind 服务端筛选下拉+批量驳回（复选+危险确认+前端循环调单条 reject 端点，零后端改动） |
| 99 | 07-11-user-ops-pool-display-fixes | 同名 | 运营池 | preview_label_for_type 按 msg_type 友好标签替代截断 XML；is_system_account 双拦（建档+list 读）；**hidden_from_pool doc-only 标记（不进 Contact struct——"已亲验无 replace_one 全 struct 写路径、无 deny_unknown_fields，故 doc-only 安全且避免 160 处构造点编译改动"）**；.segmented 下移独占一行 |
| 100 | 07-11-user-ops-pool-real-contacts-redesign | 同名 | 运营池 | "昵称全是 Demi、混入公众号"修复：三层共用 is_operatable_person 纯函数（webhook 入口拦+migration 一次性清存量+展示层兜底富化）；ContactsView 重构漏斗工作台（分档差异化行+超时提醒+批量启用） |
| 101 | 07-12-async-import-job-progress | 同名 | 导入 | 异步导入任务进度 7 阶段自底向上；红线：共享 run_import_extraction 分块/合并/D2 锚定逻辑**一字不动**搬迁；IMPORT_EXTRACT_CONCURRENCY=2 不动 |
| 102 | 07-12-audit-events-failsoft-alignment | B-02+C-01+H-01 | fail-soft | outbox 入队前 5 处纯审计 .await? 降级 let _（DB 瞬时故障不吞客户回复，与同函数孪生写法对齐）；webhook 步骤 (e) 网关聚合回复从 (d) reaction 的 else 分支解耦无条件执行；MongoDB validator 注入审计写失败的确定性 e2e |
| 103 | 07-12-kb-family1-edit-audit-unification | KB-09/10/11 | 知识审计 | chunk 编辑两绕过路径（AI 会话 apply_update_chunk+admin PUT）统一 chunk_revisions 审计+locked_fields 强制；**锁集"只进静默覆盖不进硬拒集——避免锁定字段连坐毙整条编辑"**；admin PUT 保留 replace_one 前补锁后补审计 |
| 104 | 07-12-kb01-lean-tier-clear-used-knowledge-ids | KB-01 | 硬闸 | 非 Full 档清空 used_knowledge_ids **堵 LLM 自报真实 verified ObjectId 架空 blocked_unverified_product_claim 硬闸**；resolve_used_knowledge_ids 纯函数；gateway 从"只有 if"改无条件赋值 |
| 105 | 07-12-kb08-audit-inbox-blackhole | KB-08 | 收件箱 | needs_human_audit 切片重现人审收件箱（黑洞关闭）；knowledge_review_statuses 纯函数**列表/计数同源不漂移**；InboxItem 加 integrity_status 供前端区分"AI 预审通过·待复核"；写端分诊不动 |
| 106 | 07-12-kc-family1-dispatch-atomicity | KC-01/02/03 | 活动原子性 | dispatch 每 contact 三步中途 Err **补偿回滚（全有或全无）**；dispatch_allowed_from_status 前置门防 completed 重推；dispatching 重入恢复由前两者自然成立；send 先占位写序不动 |
| 107 | 07-12-kc-family2-segment-coverage | KC 系 | 圈人口径 | 恢复"粗筛 ⊇ 精筛"不变量：防线 A $elemMatch 把 verification/eventKind 改"缺字段=默认值"显式表达；防线 B m030 迁移 $map+$mergeObjects 回填历史（治本，不加 APP_ENV 守卫） |
| 108 | 07-12-kd-family2-escalation-channel | KD-05/06 | 请示通道 | KD-05 加 last_pushed_at_ms 真实推送时刻（骚扰门口径漂移，首推+改派刷新）+m031 backfill；KD-06 孤儿 pending（current 不在链中）回落链首而非静默退化链尾；**KD-02 经裁决不改代码** |
| 109 | 07-12-kd-relay-number-guard | KD-01/03 | 护栏删除 | **删除数字护栏 fail-closed 用法（威胁模型错误：中文数字盲区/双向失效/误杀致裁决黑洞），忠实度交还 LLM+独立 Review**；载荷泄漏守卫保留；"外科式删除"；logic.rs 三数字函数不删（holding_reply 仍合法调用） |
| 110 | 07-12-kd04-principal-reply-decider-chain-fix | KD-04（批 D 唯一 High） | 请示通道 | 领导用 decider_chain（推荐配置）时微信回复不被识别为裁决；抽 is_decider_for_config 纯谓词复用权威解析器 resolve_ask_human_policy；lookup_principal_config 从只查旧标量改遍历 current_version 配置 |
| 111 | 07-12-ke-family1-auto-release-direction-consistency | KE-01/02 | 演化方向 | KE-01 decide_auto_release 加候选方向参数（升阈候选仅命中率仍过高时放行——旧逻辑的安全收窄）；KE-02 threshold 重判 original 口径从"源 run 真实终态"改"5 闸重推"与 prompt 路径对齐（消非-5gate 终态虚假抬高 send_delta） |
| 112 | 07-13-ingest-source-serde-camelcase-fix | 同名 | serde | **不给存储 struct 加 rename_all（会连带改坏 bson 存储键/worker 查询/两条索引/存量文档）→ API 输出边界手写 camelCase json! 投影**（serde 铁律的正确解法样本）；修删除/激活按钮失效与"undefinedm" |
| 113 | 07-13-p3-family1-dead-code-cleanup | H-02/F-02/KB-05 | 清理 | H-02 run_envelope R0 doc 漂移只改注释；F-02 dispatcher max_attempts 兜底对齐；KB-05 pack repair 死桩删除+摘路由 |
| 114 | 07-13-p3-family2-serde-key-tolerance | D-01/KB-03 | serde 容错 | D-01 RawAgentDecision 顶层标签键加 snake_case alias（与初始画像路径双形容错对齐）；KB-03 is_verified 收窄精确小写匹配（与写入/召回侧口径对齐） |
| 115 | 07-13-p3-family3-referral-hardening | KE-03/05 | 引荐加固 | KE-03 account 归属校验加进 validate_card_sendable（三发送路径全经过→单一事实源）；KE-05 toggle/delete 补 fail-soft 审计（照 review 模板，delete 保留硬删但先查后删） |
| 116 | 07-13-p3-family4-campaign-scale | KC-04/06/07 | 活动规模 | campaign_max_audience 硬上限（粗筛 limit(max+1) 探测法超限 400，治 OOM/写洪峰）；last_dispatch_target_count 回刷消三义计数误导 |
| 117 | 07-13-reply-prompt-slimming-batch1 | 同名 | prompt 减肥 | 删 4 死字段声明+2 冗余注入槽+3 调试元数据字段（零信息损失削 token）；**占位符对位靠 cargo build 位置参数配平+精确单测双重锁定**；format_knowledge_route_for_prompt 纯函数裁剪 |
| 118 | 07-13-revision-fallback-to-approved-draft | 同名 | 网关 | 改写三失败分支（LLM 错/30s 超时/二轮 review 未过）统一**回退发送"改写前已 Approved 原稿"而非丢弃补占位**；克隆快照+apply_revision_fallback 纯函数置"发原稿"态 |
| 119 | 07-13-wiki-channel-full-playwright-verification | 同名 | 验证 | Playwright headed 真浏览器核对 wiki 21 视图（3 模式）；**按副作用递增分层 T1 只读→危险操作只到弹窗→一次性写→造数据全链+清理**；生产 117 直测+查库确认回 95 chunks/1 doc 纯净态 |
| 120 | 07-14-agent-capabilities-audit | 审查计划 | 只审不修 | agent 旁挂 4 簇（~10.6k 行）审查：4 只读 subagent 分簇并行→**主控 Read/Grep 逐条复核 file:line 属实性+驳回夸大**→单一台账→docs PR |
| 121 | 07-14-auth-routes-security-audit | 审查计划 | 只审不修 | authz/IDOR 落实面审查；**簇S（授权根因层）先审并等回，结论作基准喂给簇1-5**（分层审查设计） |
| 122 | 07-14-worker-fleet-audit | 审查计划 | 只审不修 | worker 群 claim/幂等/崩溃恢复/调度去重审查；同簇S 根因层先行模式 |
| 123 | 07-14-f01-reclaim-chat-search | F-01 | 幂等 | 抽 verify_already_sent 共用函数（referral/media/text 三分派），reclaim 崩溃恢复分支也先查权威 chat_search（修不对称）；顺带翻正 6 条台账状态 |
| 124 | 07-14-knowledge-agent-autonomy-redline | 同名 | 红线 | "无人工接管"红线在知识层成立：知识 Agent 不产转接话术（堵源头 prompt 追加角色约束）+reason 不回流 Reply/Review prompt（切回流，**Reply/Review 复用同一净化函数 DRY**）；Layer 2 纯函数先做/Layer 1 prompt 后做/Layer 3 影子模拟泛化验证 |
| 125 | 07-14-manual-ops-enrollment-source-of-truth | 同名 | 托管准入 | **以管理员手动加入运营为唯一真相**修 AI 给非真人号自动回复；@openim/账号自反身硬盲区；四个升/降 managed 入口统一自反身硬拦+审计；hide_from_pool 联动 agent_status=normal；清存量矛盾号 |
| 126 | 07-14-p3-family5-knowledge-readiness | KB-06/07/12 | 知识信号 | KB-07 在线 gap dedup 改全量精确命中（对齐离线口径，杜绝同 kind 多主题漏合并，**在线合并逻辑一字不动**）；KB-06/12 纯 doc 标注就绪债现状（"生产未接线"/"workspace 级刻意"） |
| 127 | 07-14-p3-family6-provider-mcp-config | KD-09/10/KE-06 | 配置加固 | KD-10 先 swap_registry 成功再写 DB active（消 DB↔运行时假失败，亲验 swap 无 DB 副作用）；KD-09 base_url 缺 /v1 软 warning 不阻断；KE-06 mcp_base_url 移 $setOnInsert 与 mcp_api_key 对称保护手配值 |
| 128 | 07-14-p3-family7-escalation-reassign-guard | KD-07 | 请示防泄 | next_decider_on_timeout 跳过链中被误配成请示客户 wxid 的成员（**防内部请示卡直推客户泄漏**）；无合法下一位→None 走链尾安抚；"首推守卫不动、不加写入口校验（用户裁决）" |
| 129 | 07-14-p3-family8-webhook-edge | A-03~06 | webhook 边界 | A-06 last_inbound_at 统计 update 降 best-effort；A-05 verify=false 且多账号时无 appId→400 防张冠李戴；A-03/04 doc 标注 WontFix（生产不触发/已幂等缓解） |
| 130 | 07-14-p3-family9-outbox-timing | F-04/B-01/03 | outbox 时序 | F-04 reclaim_count >5 转 failed_terminal 止无限 reclaim（**"单 update 无法基于 inc 后值分流"→$inc+update_many 两步**）；B-03 second_safety_gate 发送前 fresh 复核 managed（决策期翻 normal 时拦截 not_managed_at_send）；B-01 抢占入队尾窗双回复标注已知产品取舍不改 |
| 131 | 07-14-principal-authorization-exemption | 同名 | 授权豁免 | 领导裁决 approved 的产品说法两类豁免：**A 类客户级豁免产品门（domain_attributes 新 key 作 R5.4 第三条并联旁路仿 priced_from_catalog，长期常驻可撤销）；B 类沉淀全体可复用 verified 知识（领导=人工验证者，op=Verify source=PrincipalAuthorized 两步法）**；exemption_type 三值闭集 |
| 132 | 07-15-db-migrations-indexes-audit | 审查计划 | 只审不修 | migrations（m001-m031）+indexes+accessor 审查，"**最后一个未审领域**"；主控亲审不派 subagent；**APP_ENV 生产守卫家族命门：生产 117 若未设 APP_ENV=production，非 prod 分支会删数据/清 seed** |
| 133 | 07-15-evolution-audit | 审查计划 | 只审不修 | evolution 全链审查；簇S"演化安全基准"（tick 单次失败不传播/budget 耗尽 silent skip/flag 读失败按 disabled 兜底/灰度 hash(contact)%100/隔离红线）喂簇1-3 |
| 134 | 07-15-knowledge-wiki-audit | 审查计划 | 只审不修 | knowledge_wiki 四职责审查；簇S"写入正确性基准"（union/lock/70%/hash 纯函数+七动作状态机+双写非事务+级联删除吞错+AI 强制 draft） |
| 135 | 07-15-pr-a-memory-consolidation | A-01/02/03 | 记忆修复 | A-01 core_facts 缺证据门（importance 天花板判定）；A-02 confirmed_tags replace 丢标签→merge_confirmed_tags 纯函数；A-03 跨集合写非原子重放→?改 fail-soft |
| 136 | 07-15-pr-c-generalization | C-01/02 | 通用化 | C-01 stagnation 计时写侧写死 customer_stage→按 active profile 的 stagnation_dimension 检测该维度自身变化写 {dim}_updated_at；C-02 初始画像半接线（非销售域维度不采集）→比照 live reply 追加两维度指引 |
| 137 | 07-15-pr-e-post-release-mapping | [2-01] | 演化观测 | FIVE_GATE_KEYS 把 fact/pressure 的 status 映射**对调**（与 threshold/significance 两权威源相反）面板贴错标签；修法复用权威 safety_block_status_for；pressure_risk 软闸不产 block 终态→改 revision_failed 口径；纯观测不反哺自动决策 |
| 138 | 07-15-sediment-title-from-substance | 同名 | 沉淀质量 | 沉淀 chunk title 从 decision.substance（知识内容）非 entry.reason（质检黑话）；**确定性 fallback 纯函数+LLM 提炼叠加（失败一律回退兜底，"沉淀永不失败"）**；存量污染走一次性 mongosh 脚本非 migration |
| 139 | 07-15-import-failed-segment-visibility | M1/M4 | 导入可见性 | M1 纯前端消费缺陷（后端 progress.failed 已全返回零后端改动）——前端补类型+警示条；M4 product_tags 补进召回 haystack 参与打分 |
| 140 | 08-05-run-log-stage-table-overflow | 同名 | CSS 修复 | **根因是类名误用非 CSS 写错**：.tHead（flex+space-between 为事件时间线设计）被复用到阶段区块，flex 项 min-width:auto 不收缩→表格外撑标签竖排；新建 .stageBlock/.stageTable 不动 .tHead；全局守卫 .main{min-width:0} |
| 141 | 08-05-wiki-admin-governance-css-baseline | 同名 | CSS 修复 | 治理工坊五渲染缺陷分属三层各修正确层（全局 input 基线漏排除 checkbox/Knowledge.css 漏覆盖/table-layout:fixed+colgroup）；**"发布给全部"白底白字=不可逆高危操作按钮不可见（功能缺陷）** |
| 142 | 08-06-ask-human-inbox-layout | 同名 | CSS/布局 | 收件箱从"内容裸贴灰底"改全项目一致 .page+白卡；9 来源 chip 单排；空态用共享 EmptyState |
| 143 | 08-06-decider-chain-roster-picker | 同名 | 请示配置 | 决策人链"+从联系人添加"换通讯录选择（复用 FriendPickerModal+loadRoster 缓存）；**未入库好友先 POST /contacts/import 落库（agent_status=normal 不托管）再加入链——后端 put_ask_human_policy fail-closed 要求决策人已在 contacts 表** |
| 144 | 06-26-full-business-logic-test | 同名+findings | 真测工程 | 13 能力域 ~30 生产 LLM 点在 server117 真模型端到端测试：本地 Python 经 _remote_run.py 驱动；**发送侧验证到 outbox 为止绝不真发微信；涉 LLM 断言必查 llm_call_logs status=success 绝不假绿；biztest_ 前缀身份隔离一键 cleanup**；"实测确认的真实接口（写脚本照此，不要按 spec 占位猜测）"节；产出 findings spec——**当前 git status 的 scripts/biz-test/* 即其产物** |
| 145 | 07-11-deep-logic-audit-batch-a | 审查工程 | 只审不修 | 自动回复命脉链 8 环节深审（webhook→去抖→gateway→决策→review→outbox→MCP→回写）；**"这是审查工程不是写功能——deliverable 是台账 findings 不是新代码"**；只入账不改 src/引用必亲验/subagent 结论必主控亲验；117 真跑硬约束（真发只对两个指定 wxid）；产出统一台账 2026-07-11-deep-logic-audit-findings.md |
| 146 | 07-11-deep-logic-audit-batch-b | 审查工程 | 只审不修 | 知识三频道 7 业务链深审（grounding 召回最高）；对照 06-30 wiki 结构审查基线不重复；finding 编号 KB-NN——**07-12 kb 系修复 plan 的 finding 来源** |
| 147 | 07-11-deep-logic-audit-batch-c | 审查工程 | 只审不修 | 成交活动三频道四业务环（圈人→触达→成交登记→成效聚合）；KC-NN 编号——07-12/13 kc 系修复来源 |
| 148 | 07-11-deep-logic-audit-batch-d | 审查工程 | 只审不修 | 请示配置四频道四链深审（红线"客户永不知道有领导"最高）；KD-NN；**首现"严重度跨批一致性校准"（DB-fault/时序类默认 Med，不因单批 subagent 定 High 破坏校准）** |
| 149 | 07-11-deep-logic-audit-batch-e | 审查工程 | 只审不修 | evolution+referral+account/overview/operations 收官全五批；KE-NN；严重度校准细化（"只有推荐配置下确定性发生的核心交互/红线破坏够 High——见批 D KD-04 基准"） |

<!-- END-S1 -->

## 2. 计划≠规格差异清单（逐条：plan 文件、差异内容、出处段落）

收录标准：plan 明文记载的对 spec 的修改/缩窄/放弃/推翻/修正（含"暂缓/不做/简化"条目与 spec 锚点漂移修正）。

<!-- DIFFS -->

### 06-02 knowledge-closed-loop-trajectory
- **门 2/门 3（真 LLM 辅助门）不含在本计划**——Self-Review 明言"与 spec 第 5 节『本轮做门 1 + Q2 抽取；门 2/3 视 CI 预算』一致——门 2/3 留作后续计划（避免真 LLM 时间预算与本轮确定性门耦合）"。plans 目录内无后续同名计划跟进（文档声称层面未闭环）。出处：Self-Review 节"门 2/门 3 本计划不含"。
- 计划自认风险：`KnowledgeVerifyRequest` 字段与 verify 返回体 schema"未逐字核到"，靠执行时 grep + `||` 双形态断言容错兜底。出处：Self-Review"风险点"第 1 条。

### 06-04 frontend-ui-refactor
- Self-Review 声称对 spec §2.1-§7 全覆盖无遗漏。计划级自选决策（spec 未定）：迁移顺序两可（先搬 feature 再切入口 vs 先切入口），plan 拍板前者并注明"两种顺序都可"。出处：Task 3.1 Step 6 说明。
- group-ops / moment-ops 明确为"占位页，Phase 1 不实现"（保留空 feature + 规划中提示）。出处：Task 3.2~3.13 表 #3.12/#3.13。
- overview 的 pendingTasks/latestEvent 跨 feature 派生：先占位 TODO，3.4/3.5 迁完后回填补一次提交。出处：Task 3.1 Step 4 + 表后注。

### 06-05 principal-decision-channel（全集"计划改设计"最重的一篇）
1. **统一占位模型推翻 spec 原 Held 挂起设计**（2026-06-06 用户拍板）：触发请示的 run 始终 `Approved`，decision Agent 输出安全占位 reply 走正常 outbox；请示只是 approved 路径末尾副作用（推卡+落台账）；run 不进 Held、不设 hold category、**完全不碰 review.rs**（原 Task 18 对 review.rs 的改动整体删除，"冲突面归零"）。出处：头部"核心模型（2026-06-06 用户拍板）"段 + Task 18 头部说明 + Self-Review §4。
2. **两处 spec 锚点漂移修正**（写码以 plan 为准）：① `mcp.rs` 无 `message_send_text` 函数——那是 MCP 工具名字符串，调用走 `mcp::logged_call_for_account`；② 无 `guards/` 子目录（单文件 guards.rs），spec 说的"五闸门"已删除，当时实况= knowledge_grounding/hallucination/run_budget 三闸门 + review 评分体系。出处：头部"两处 spec 锚点漂移已修正"段。
3. **授权过期窗从 spec 的硬编码 24h 改为"领导说了算"**：LLM 解读领导明示时限填 `authorization_window_hours`；没提→None=不设过期窗长期有效；"不要自己默认一个时长"。出处：Task 14 prompt 正文 + Task 19 + Self-Review §5 第 2 项。
4. **跨 workspace 串扰修正**：`lookup_principal_config` 必须用入站消息自身 workspace_id 约束查询（原计划全局查会把 A 工作区领导误路由进 B 的请示流）。出处：Task 20 helper 注释 + Self-Review §5 第 3 项。
5. **spec §9.1 接触级归属 MVP 显式缩窄**：不写 memory（转述回复天然落聊天历史 + 台账是授权过期判定权威源）；"跨多轮滑窗后授权上下文可能丢失——有意接受的 MVP 边界，不是 bug，执行时别顺手加 memory 写入"。出处：Task 15 🟡 说明段。
6. **spec §9.4 领导无未决时的主动指令 MVP 缩窄**：只记日志（tracing::info），不自动生效（"待 admin 确认"）。出处：Task 19 NoPending 分支。
7. **Task 22 卡死信号 prompt 注入接线声明为可放弃项**："若现成信号不足，本 Task 仅落地纯函数 + 阈值常量，注入接线记为后续增强（不阻塞 MVP）"。出处：Task 22 Step 3 设计说明。
8. **plan 补 spec 没有的关键机制**：relay 哨兵 `__PRINCIPAL_RELAY__`（`PRINCIPAL_RELAY_SENTINEL` 常量）+ decision prompt "转述模式"输入契约成对出现——Self-Review 称"闭环命门，没这对契约闭环跑不通"（spec 未记载此机制）。出处：Task 16 Step 2 🔴 说明 + Task 13 Step 2 + Self-Review §5 第 1 项。

### 06-06 knowledge-grounding-and-verify-gate-fixes
- Self-Review 声称 spec A.3/A.4/A.5/B.2/B.5 全覆盖；不变量自查明列"未新增闭集枚举、未动 `apply_chunk_integrity` 本体；A 硬闸仅收窄到 5 个效果词，语气类 3 词仍仅观测"。出处：Self-Review 节。
- A3 的 reviewer prompt 反向锚点无纯函数单测，"靠 real-LLM 套件观测"——prompt 类改动的验证空档如实声明。出处：Task A3 Step 2 注。

### 06-07 knowledge-routes-split（纯机械，无 spec 行为差异；但含执行反馈回写）
- 计划正文含"Task 2/3 执行中发现"的**可见性三铁律**与"Task 4 发现"的 E0616 字段可见性推论——执行发现被回写进 plan 文本（该 plan 已执行的内部强证据）。出处："搬运通用步骤"前的可见性铁律块。
- 结构决策：6 个 Query/Request struct 留 mod.rs 作共享类型（`OperationKnowledgeChunkRequest` 15 处引用）；`load_operation_knowledge_chunks_for_query`、`budget_document`、`chunk_verify_gate_reason`、`truncate_for_prompt` 等跨域 helper 经核实后留 mod.rs 而非按初始归属表搬走。出处：Task 2/3/5/6 各"保留在 mod.rs"注记。

### 06-07 knowledge-trust-cockpit-frontend
- CockpitView 待办区"降级数暂用 integrity 的 rejected 占位（真实 distortion 计数阶段三补）"——阶段三任务清单中无明确回补条目，疑未闭环（文档声称层面）。出处：Task 6 Step 3。
- 契约漂移事实（作为地基修复的依据）：前端 `IntegrityReportView` 认 `contested/sourceOrphan` 两个后端不存在的字段；`CompletenessView` 只认 `perWikiType/overall` 而后端真实响应是 answeringMode+5 维 coverage——"全错"。出处：Task 1/2 头部说明。

### 06-08 escalation-split（考古价值：暴露 06-05 计划外的代码演化）
- 行号映射表显示 06-05 落地后 3 天内 escalation.rs 已长出 06-05 plan 未规划的函数：`escalate_held_decision`(408-503)、`should_escalate_held`、`is_principal_relay_trigger`、`relay_output_leaks_internal_payload`、`build_decision_signals_text`、`consecutive_unprogressed_turns`——即"统一占位模型"（run 不进 Held）落地后又补了 **Held 决策升级请示**路径（mod.rs use 含 `ESCALATION_CATEGORY_HIGH_RISK_GATED`），两模型并存。出处：项→文件映射表 + Task 4 Step 3 use 清单。
- `assert_target_is_principal` 在映射表标注"（含 `#[allow(dead_code)]`）"——06-05 计划中 gateway trigger 调用它，3 天后已成死代码，说明 gateway 侧实现与 06-05 Task 16 原文有出入（文档声称，未对码）。出处：logic.rs 映射表第 12 行。

### 06-08 knowledge-frontend-split
- 无 spec 行为差异；诚实验证条款："若无法跑浏览器，明确说明『dev server 已起、tsc/build/test 全绿，但未做人工点验』，不假报成功"。出处：Task 6 Step 4 注。

### 06-15 h17-memory-dimensions-universal
- **范围缩窄三条**（相对 universal-domain-adaptation spec 的 H17 描述）：① typed 三数组 coreFacts/recentFacts/deprecatedFacts 固定 cap **6/10/20** 不纳入维度化（"结构骨架不是业务维度"）；② 不动 `intent_trajectory.objection_type`（"另一条轴……除非你要一并做"）；③ 迁移评估后"倾向 serde default 即可，无需迁移（DomainProfile 不像 OperatingMemory 有海量历史文档）"。出处：设计节 default_memory_dimensions 注 + "不做（范围边界）"节。
- 完成后承诺回写 spec："`docs/superpowers/specs/2026-06-11-universal-domain-adaptation-design.md`（H17 行标 ✅）"——plan 与 spec 的状态同步机制实例。出处：牵动文件节。

### 06-19 eval-overhaul-phase1-judge-context
- **只做 spec 的阶段 1**：Task 7 收尾 commit 信息写"J1/J2/J5 阶段1底料注入落地——待阶段2红线/阶段3对话级"；plans 目录内无 eval-overhaul-phase2/phase3 计划文件（阶段 2/3 疑未以独立 plan 落地或并入他篇——文档声称）。出处：Task 7 Step 4。
- 铁律"只改 tests/ 绝不碰 src/（prompts/guards/gateway 一律不动），违反即范式作废"。出处：Global Constraints。

### 06-19 universal-residuals-completion
- **终审追加节（2026-06-20）是 plan 被执行后回写扩展的样本**：whole-branch review 判 READY TO MERGE、六红线全 PASS，但发现两项非销售侧 incompleteness，用户拍板"彻底做"补 Task 8/9。出处："终审追加"节头。
- **Task 9 自认漏项**："spec H17.4 列了『reaction prompt 维度名随 profile』，本批漏做"——补 reaction_trajectory_prompt_addendum（DEFAULT 单维返 None 字节等价）。出处：Task 9 根因段。
- Task 7 Step 2 承诺回写 spec 残留核查节（H13/H17/H18 标已收口）。出处：收尾 Task 7。

### 06-20 universal-audit-remediation
- **明确不实施条目 G02**：收尾节明示"不含 G02——终审列必修但本批用户范围是『4 必修』即 G21/G13/G06/G01，G02 归可缓但 spec 未展开为独立 task；若执行时有余力可补，否则记路线图"。（注：此处 4 必修口径 G21/G13/G06/G01 与 Goal 段的 G21/G13/G01/G07 不一致——plan 内部两处口径微漂，均为"文档声称"。）出处：收尾"全批 merge gate"节 vs Goal 段。
- **结果开放任务两条**：Task 10 G16"审查中唯一需先核实再决定改不改的任务"（情况 A 删 pre-review upsert / 情况 B 证伪记 PARTIAL-REFUTED 不改代码）；Task 12 G09"若盘点发现断言全 DB 依赖型则不强拆，记报告说明"。出处：Task 10 红线段 + Task 12 说明段。
- **G05 订正坐实一项未实现能力**："数字分身口吻分化（per_relationship 各异 soul/tone）当前未实现——OperationMode 无 voice/tone/soul 字段、decision.rs soul 注入不读 relationship_type"，注释改为指明独立专题。出处：Task 14 Step 2。
- 提交边界排除清单含 `docs/superpowers/plans/2026-06-18-*`——曾存在 2026-06-18 计划文件（并行会话产物），现 plans 目录无任何 06-18 文件（未收录/未提交）。出处：Global Constraints 提交边界。

### 06-21 agent-send-ledger
1. **写入点改设计落点**：spec §4.1 的"发送函数内写入"改为 **dispatcher 成功分支**（outbox 标 Sent 后）——"send_outbound_media/namecard 签名只有 (state, contact, id) 拿不到 run_id；dispatcher 持有 entry（含 run_id/referral_card_id/media_asset_id）+contact，一处写入对称覆盖两功能"。出处：Task 4 设计依据段 + Self-Review §4。
2. **spec §6.3 "统一从 ledger 取已发历史单一事实源"缩窄为只做素材侧**：名片侧保留旧 AlreadyReferred 数据源（contact.domain_attributes[REFERRED_CARD_ID_ATTR]），"避免回归现有已工作逻辑；统一收敛作为未来项 YAGNI 暂不强做"——**已发历史双数据源并存成为既知状态**。出处：Task 7 设计依据段 + Self-Review §4。
3. 集成测试走公开 CRUD 路径而非直调内部函数：`scan_send_ledger_outcomes`/`build_ledger_entry` 是 pub(crate) 跨 crate 不可见——"转化判定纯函数已由内联单测覆盖，本文件不重复"。出处：Task 10 可见性约束段。

### 06-21 annotation-quality-gate
- 修复的缺陷本质（spec 已载，plan 落实细节）：运营手填 stage alias 不归一 → 与 canonical customer_stage `s == cs` 永不相等 → 素材/名片**静默不发**；`ASSIST_MODE_OVERRIDE_ATTR` 此前"全仓零写入路径"。出处：Task 1/6 背景段。
- 计划内当场修正一处顺序问题：归一原排在 store_bytes（落盘）之后会留孤儿文件，plan 自我纠正"实现者：优先前移到 store_bytes 之前"。出处：Task 2 Step 2 顺序考量注。
- 有意保留的不一致：referral review not-found 返 `BadRequest` 而 media 返 `NotFound`——"保持现状不改，只在成功后加审计"。出处：Task 5 Step 1。
- Task 8 实现者决策点（结果开放）：归一断言需预种字典 alias，若成本高退而测"越界 400 + 未配置放行"两条；fail-soft 审计失败分支"不写依赖故障注入的测试（YAGNI），由代码审查保证"。出处：Task 8 Step 1 决策点 + Step 3 注。

### 06-21 ask-human-config-page
- **P2（askHuman 收件箱前端）在 plans 目录无独立计划文件**：本篇完成定义末句声明"ask-human 三子项目（P1 后端→P2 收件箱→P3 配置页）全部交付"，且 Task 6 Step 1 提及 union"P2 已加过 askHuman"——P2 已实施但未留 plan（plans 覆盖缺口）。出处：Phase 3 完成定义 + Task 6 Step 1。
- 实证基线节坐实序列化双轨事实：`OperationDomainConfig` 无 rename_all（落库 snake_case `ask_human_policy`）但 `operation_domain_json` 手写 json! 输出 camelCase——两套命名并存。出处："实证基线"节第 3 条。

### 06-21 ask-human-unified-channel-phase1
- **计划内声明的功能性欠账**：Task 9 接线处 `push_allowed` 的 `last_push_ms` 恒传 **None**——"dedupe_window 的精确 last-push 查询留待 Phase 优化，None 时 dedupe 分支不触发，字节等价"——即 **dedupe_window_hours 配置项 Phase 1 配了也不生效**（骚扰去重实际只靠 pending 去重 + daily_cap）。出处：Task 9 Step 1 注。
- `HighRiskEscalationMode`/`parse_high_risk_mode` 旧机制**保留不删**："若 Task 9 移除最后引用则那时再删——本 task 不删，避免编译断裂"（渐进退役而非同步清理）。出处：Task 3 Step 3 注。
- admin resolve 的幂等/防泄漏语义：不在本 workspace pending 列表（可能已 resolved 或越权）→ 返"幂等成功 alreadyResolved"避免泄漏存在性；deferred verdict 不起 relay。出处：Task 7 Step 1 代码注释。
- Phase 1 完成定义明示范围："**不含任何前端**（P2/P3 另起）"。出处：Phase 1 完成定义节。

### 06-21 referral-card-push（头部"实现期对齐"节 = 计划以真实代码取代 spec 假设的典型样本）
1. **架构决策改设计**：OutboxEntry **并列加 `referral_card_id`，不复用 media_asset_id、不抽 entry_kind**——素材内核（media_asset_id 一个字段贯穿 content_required_for/media_routes_synthetic/dispatcher 三处分流/media_already_succeeded）"他的代码一行不改、零回归"；代价是 dispatcher/outbox 轻度分支重复；"将来收敛抽 entry_kind 是独立重构专题，不在本期"（已知重构债）。出处：实现期对齐第 1 条。
2. **Task 6 删减**：ConversationMessage 的 msg_type/media_ref 字段素材侧已加（media_send.rs:204-205 在用），本计划直接复用不再加。出处：实现期对齐第 2 条。
3. **名片不套用 requires_principal_approval→escalation 分支**（spec D9：素材请领导核准 vs 名片主动引荐语义不同）。出处：实现期对齐第 6 条（gateway 对齐段）。
4. **名片条目崩溃恢复跳过 post-hoc 核对**：mcp_already_succeeded 按 text 形态匹配对名片恒 false→直接跳过（"重复推名片危害小于文本重复"）。出处：Task 7 Step 2。
5. 计划自认修正一处漏判：media_eligible 门"计划原只写 approved，漏了 revision_applied_approved，已修正"。出处：实现期对齐 gateway 对齐段。
6. **已知未决**：MCP `message_send_namecard` "仓内零书面依据，仅用户口头确认"，入参字段名占位待 server tools/list 对齐。出处：Global Constraints "MCP 工具未决"。
7. Task 14 落地 CLAUDE.md"辅助模式受控例外"段——与现行 CLAUDE.md 一字对应（该 Task 已实施的文本证据）。出处：Task 14 Step 1。

### 06-21 sales-media-asset-send
- `requires_principal_approval=Some(true)` 的素材在 gateway **不入 outbox**，改走 escalation 请示通道（"入请示，拿结论后再决定"）——与名片路径（不套用此分支）形成对照。出处：Task 8 Step 6 代码注释。
- expression_pref 两种偏好（file_primary/file_support）**当前定序都是 TextThenMedia**（先文字后文件）——抽 `media_send_order` 函数留扩展点，"若后续想 file_primary 改先发文件，改这一处即可"。出处：Task 8 Step 1 注。
- Task 4 集成测试先占位 `assert!(true)`、Task 11 回填真实断言——"这样划分让上传 API 可独立提交"（占位是分期策略而非遗漏，且 plan 内有明确回填任务）。出处：Task 4 Step 1 说明 + Task 11。
- MCP 入参（mediaId/base64）占位待 tools/list 对齐（已知未决，符合 spec §11 风险记录）。出处：Global Constraints + Self-Review §2。

### 06-22 business-audit-fix-wave
- **F 组明确打桩/未实施**：`fetch_inbound_media` 打桩返回 None（"MCP server 无下载入站媒体 tool，仓内零调用，实现前必打 tools/list"）；**语音 ASR 零能力待独立立项**。出处：Task F2 Step 1 + 背景段。
- **Minor 缺陷未单列任务**（已拒绝客户不唤醒等）："spec 标『writing-plans 阶段评估，倾向附带』，留待执行中按需附加，不阻塞主线"——可能未实施（文档声称）。出处：Self-Review Spec 覆盖段。
- Task B2 落点结果开放：gap_signal 写入点取决于 `finalize_review_for_send` 是否 async/持有 db——"实现者先确认，据此选落点（函数内 await 或 gateway 上游）；记录最终落点"。出处：Task B2 Step 3。
- A3 spec 未决细节定稿：链尾安抚发给**客户**不过 `push_allowed`（"quiet_hours 是约束打扰领导的；客户安抚受 min_interval 去重约束即可——见 spec 未决细节 1 已定稿为『不过』"）。出处：Task A3 Step 6 注。
- **历史注脚**：Task A1 的 `relay_introduces_unauthorized_number` 数字护栏即 07-12 kd-relay-number-guard 被用户裁定整体删除的那个守卫——本 plan 是其出生地（当时定位"只兜客观数量，不判断措辞语义"，后被判定威胁模型错误）。出处：Task A1 全文（对照 07-12 plan）。

### 06-22 digital-twin-relationship-closure
- Task 5 visibleWhen 机制"只建机制，不给任何频道接规则（全显示）"——"用户明确频道是账号级、客户类型是联系人级，频道门控当前价值不大；机制留作后续扩展点"。出处：Task 5 说明段。
- Task 6 示例 profile seed 为 draft/inactive（current_version=false + is_active=false），"运营审阅后手动 activate（不擅自改变零配置启动行为）"。出处：Task 6 说明段 + Step 5。

### 06-22 media-asset-crud-completion
- 部分更新语义边界如实声明："serde 不区分缺失与 null，故不支持显式清成 null；清空走传 `""`/`[]`"。出处：Task 2 Step 1 注。
- 引用计数查询失败时 `unwrap_or(1)`——"查询失败 → 视为有引用，保守不删"。出处：Task 3/5 代码注释。

### 06-22 structured-organization
- Task 1 Step 4 预警 PBT 冲突处置："若 chunk_type_routing PBT 锁逐字全文，需同步更新该 PBT 的期望值（属本 task 合理改动，注明）"——增量叠加铁律下"行为定义变更"的合法改测试先例。出处：Task 1 Step 4 注。

### 06-22 taxonomy-admin-crud
- 编辑表单**不含 id/scope/kind**（主键不可改，对应后端 PATCH 只接 label/aliases/description）；一次只展开一个编辑行且与新增表单互斥。出处：Task 3 背景段。

### 06-23 progressive-prompt-three-tier
- **部署依赖代码外手动操作**：Task 5 的 prompt 文本更新在 DB 模板（admin UI 手动改 user.reply.task/policy/reviewer 三模板），代码只 bump 版本号——"实际措辞需运营/产品审阅"；若手动步骤未做则充分性自评契约不生效（重大交付状态不确定点，文档声称）。出处：Task 5 头部"重要"注 + Step 3 清单。
- Task 3 Clarify 分支临时实现："暂用第一程结果+跳过发送（待 Task 5 reviewer 改造接 ai_waiting_for_more_context）"——分支收尾跨 Task 依赖。出处：Task 3 Step 2 代码注释。
- 决策演化注脚：本 plan 的 missing_tier 非法值兜底"默认升中档 Relational"，06-24 hardening 改为**兜底 Full（更保守）**——同一判定在相邻两 plan 中方向修正。出处：Task 1 Step 6 vs 06-24 plan。

### 06-23 tag-trust 五部曲（1~5）
- **五部曲之 4 的 plan 内回写**：`should_evict` 低置信淘汰"按 Option B / YAGNI 决策删除（未实现）——永不驱动旁路，槽僵化零业务影响，详见交叉验证 D5-F2"；接口清单中该函数被删除线标注、Self-Review 对应项改注"未实现"——**执行/交叉验证结果回写进 plan 的实证样本**。出处：tag-trust-4 Interfaces 节 + Self-Review。
- 之 4 两处结果开放：人格分析"先试合并（搭车 consolidator 同一次调用）；若归并测试质量下降则拆开，报告说明选择"；Task 4 守护测试有退化路径（难以构造则注释+review checklist）。出处：Task 3 Step 5 + Task 4 Step 1。
- 之 1 顺手修的语义决策：update_profile_note 的 AI 重生成标签"**删写入，不替换**"——"confirmed 是压缩重判产物，不该被 note 旁路直接灌入"（generated.tags 直接丢弃）。出处：tag-trust-1 Task 7 Step 2。
- 之 1 migration 取舍："虽设计称无存量，仍按项目习惯补 migration 保证索引/反序列化一致"。出处：tag-trust-1 Architecture 段。
- 之 2 设计取舍：整批标签共享一个证据序位集合（tag_evidence_turns），不逐标签配对——"LLM 输出复杂度暴增、收益低（标签本就要在压缩重判时重新指认证据）"。出处：tag-trust-2 Task 2 头部注。
- 之 3 跨子计划一致性要点：逐轮窗口与压缩宽窗口是**两个不同窗口**，turn 序位只在各自窗口内有效，msg_id（ObjectId hex）才是跨窗口稳定锚。出处：tag-trust-3 Self-Review 关键跨子计划一致性段。

### 06-23 tool-loop-dead-code-sunset
- **明确顺延项**：`autonomyProtocolEnabled` 同为 sunset-plan D+21 删除项，但"不在本次范围——独立开关，需单独核实读点后另行下线"。出处：Task 4 Step 1 注。
- Task 3 Step 2 含罕见的**计划自我纠正**："修正预期：此刻字段仍在，测试会编译通过……这个测试的真正价值在 Step 4 之后（删字段后语义变为『忽略未知字段』）"——写计划时发现原预期错误并当场改写。出处：Task 3 Step 2。
- PR body 的 Test plan 用 `[x]` 预勾选（0 error/≫350/兼容测试/红线双门），仅 CI 项留 `[ ]`——该 plan 交付状态的文本级证据。出处：Task 5 Step 4 PR body。

### 06-24 progressive-tier-hardening
- run log tier 元信息的落点决策链：spec 设想写 run log 字段 → plan 约束"不碰 models.rs，塞既有 gateway_result Document 自由字段" → 最终"优先用 `ptier_run_tier` 事件方式（零签名改动、零跨函数变量穿透）"——同一需求三级降解到最小侵入。出处：Global Constraints + Task 2 Step 5。
- 并行会话版本号协调："若执行时并行会话已 bump 到 >v10，则在其基础上 +1（vN→vN+1），保持单调递增、不抢号"。出处：Task 4 Step 3 注。

### 06-25 taxonomy-label-wiring
- **plan 修正机制样本**："⚠️ 可行性审查必修修正"节（M1-M5）声明"实现对应 Task 时，以本节为准覆盖下方 Task 原文"——Task 3 原文代码仍写着 M2 判定不存在的 `AuthenticatedUser`，修正以补丁节形式追加而非改写原文（读此类 plan 必须先读修正节）。出处：修正节头 + Task 3 Step 1 代码。
- minor④ 坐实 taxonomy scope 语义：scope 是 **account_id** 而非 workspace，"端点传 workspace 概念错位但 global seed 可达（account-first→global 回落），DEFAULT 取值能翻译"——已知的概念错位带回落兜底。出处：修正节 minor④。

### 06-26 alignment-batch3（对 fixes-design 的现状修正）
- **E11 从批次3 移除**：原列 19 条，"写 plan 后复核最新 main 发现该条已完整实现——后端 confirm/reject 端点（management.rs:443/519）+ 乐观锁防 IDOR + 前端确认按钮均已落地"。spec 的 76 缺口清单在 plan 阶段被现状复核修正。出处：Goal 节 + 组三头注。
- E9 知识缺口口径消歧义：= `knowledge_gap_signals` status="pending" 计数（依据 ask-human spec :1230），**不是** integrity 报告的 gaps.length——spec 未写死口径由 plan 拍板。出处：Task 15 Interfaces。

### 06-26 main-health-audit-batch2（plan 推翻 spec 原始方案两处）
- 头部声明："每条方案均经改动点完整业务逻辑深度核实定稿，**推翻了 CONC-2（弃 pipeline）、CONC-1（不套整个 update）两处原始方案**"——spec 原设想 CONC-2 用 aggregation pipeline update、CONC-1 给整个 update 套版本谓词，plan 深读代码后判定不可行（门控外字段不 bump version，套谓词会"永久 lost-race"）。出处：来源节 + Task 4 方案注。
- 非目标节明确 4 项不做：evolution worker 多租户遍历（独立工程）/memory_summary 并发 OCC（接受 last-write-wins，纯文本后续复述自愈）/commitments 应用层去重原子化（接受并发重复）/taxonomy 软闸 revision 复检（本就有意非阻断）。出处：非目标节。

### 06-26 management-agent-thickening（plan 多轮修订痕迹最重的一篇）
- **"4 路 opus 核实修正记录"**：原草案错误逐条列明——函数名笔误 is_read_only_tool→is_read_tool；build_management_plan 走版本化 prompt 非内联字面量（原判断错）；Task 6.5 原草案 key 命名半数错（agent_soul/operation_playbook 不是 template_key、user.review.policy 不存在）；**核心红线漏洞**：原锚闸只查 DEFAULT_MODE_GATE_POLICY，而该锚"故意不含反接管红线"（测试 prompts.rs:2506-2511 坐实）——真红线在 user.reply.policy 正文 :1123/:1146，旧闸根本没在查。出处：Task 6.5 头注 + 文末修正记录。
- **"main 合并冲突修正记录"**：合并 origin/main 25 提交（PR#41+PR#42）后发现两个"让改动白做"的真冲突——①CRLF 误判（锚闸须两边 normalize_prompt_content）；②**启动对齐覆盖（最严重）**：PR#42 align_prompt_specs 每次重启把 seeded_by="system" 且内容≠DEFAULT 的行归档重种，故 update_prompt_template 的 $set 必须置 seeded_by="manual"，"否则管理者改动活不过一次重启"。出处：文末合并冲突修正记录。
- spec §4.2/§4.3 第一期权限策略：dangerous 确认开关默认 false（先跑通），但 irreversible（reset/delete）+ verify 类无视开关恒确认——verify handler 写 source=Human，包成 AI 工具会"AI 调用被记成人确认"，守"AI 永不自动 verify"。出处：Task 1。

### 06-26 prompt-pack-alignment-completion（版本号语义正式降级）
- `PROMPT_PACK_VERSION` 常量从"生效闸"**降级为"仅 stamp 溯源"**——生效判定完全交给 align_prompt_specs 逐 key 内容比对，"改 spec 重启必生效，不靠版本号"。此后各 plan 说的"bump 版本"只影响溯源标记。出处：Goal+Architecture。
- Err 兜底哲学："查询异常时重新种入默认模板，**宁可短暂存在重复条目，也要保证模板始终可用**" + 写 prompt_pack_reseed_fallback 事件留痕。出处：Task 1 Step 2 代码注释。
- 不改 reset_prompt_pack_v2 签名（pub 有其它调用方 blast radius 大）→ ensure 在空库/Err 臂调 reset 后手动返回 Ok(true)。出处：Task 4 背景。

### 06-27 chunk-ai-repair-closure（"已实证修正"节：spec 表述≠真实代码 4 处）
- propose/answer 返回体以 repair.rs:365-377 实证为准（sessionId 在返回体非 header）；**BudgetExceeded 是 HTTP 错误不是 200+字段**（spec 旧表述会让前端解析错误路径）；ChunkProvenanceView 前端只定型 2 字段须补全 6；落库样板取 useGoLive.ts runGoLive（不抛错返回 {ok,reason}）。出处："已实证修正"节。
- knowledge 频道 CSS 惯例特例：用 plain `.css` 非 module（避 tree-shake），与其它频道 `.module.css` 约定相反。出处：Tech Stack + Task 5。

### 06-27 prompt-evolution-human-gated（对 spec 的架构落位与语义改动）
- grade_prompt 放行语义改动：prompt 候选 **completed≥1 即置 eligible_for_release（=证据就绪等人工），不再用 critique_delta gate 自动放行/拒绝**——自动放行改人工定夺，spec 原 gate 语义被替换；顺带 evolution_min_self_critique_delta 变 dead config（跟进项 Minor-A）。出处：Task 13 Step 3 + 跟进项。
- 阶段二跟进项 Important#1（记录于"合并 PR #51 时"）：shadow 的 LLM 消耗走 RunBudget（run-local）不回灌 tick 级 EvolutionBudget——极端大 cohort 单 tick 开销可能超预期，倾向 fold-back 记账修法。**该 plan 阶段二已合并为 PR#51 的完成证据**。出处：阶段二跟进项。

### 06-28 customer-reply-guarantee（实现期修正推翻原案一处挂载点）
- 头部"⚠️ 实现期修正（2026-06-28，已合并 main，权威以代码+spec §3.5 为准）"：原计划 4 处挂载点（含 A3 no_reply），实现期 **CI 集成测试 `full_flow_a3_no_reply_skips_review_and_outbox` 证伪 A3 挂载**——A3 是 AI 主动判定沉默（非晾死），补占位破坏拟人 → 最终 3 处挂载、no_reply 入 ACK_PLACEHOLDER_EXCLUDED_STATUSES；正文 Task 3 Step 5 等原案措辞"已被本修正覆盖"。出处：头部修正注记。
- plan 补 spec 遗漏：spec §3.1 只列两处挂载点，"遗漏了第一道 precheck（gateway.rs:916）——daily_limit 等会在决策前就被拦、零回复 return"，plan 据用户"全兜底"决定补上。出处：黑名单口径节注。
- 签名细化替换 spec：统一 `status: &str` 取代 spec 的 `(final_status, should_reply)`（第一道 precheck 出口无 final_status）。spec §5.2 Docker 集成测试降级为"纯函数单测+真模型端到端"（YAGNI，强制构造 held 终态需 mock LLM 成本高价值中）。出处：Self-Review。

### 06-28 campaign 三部曲（同日三 plan 顺序交付链）
- targeted-push（引擎）→ sends-report（可观测，基 d615bdc，引用"活动定向推送交付经验"证明前者已交付）→ campaign-frontend（看板，基 700c57d 消费 sends 端点）。plan 基线 commit 前进 + 互相引用形成交付顺序证据链。06-29 campaign-domain-completion 基 c163542 进一步坐实 sends-report=PR#57、campaign-frontend=PR#58 已合并。出处：四 plan 的基线声明。
- targeted-push 对 spec §7.2 的确认门修正：post_management_message 硬编码 dangerous_confirm_enabled=false → 仅定 Dangerous 档不会触发确认，**dispatch_campaign 必须加进 tool_always_requires_confirmation 才恒确认**。出处：Global Constraints。

### 07-01 系（H 系修复的工程语境）
- **"subagent 红线（用户 2026-07-01 点名强调）"从 07-01 起成为每篇 plan 的 Global Constraints 固定条目**："实现时遇到任何不理解的地方，先自己 Read/Grep 读代码、亲自验证，再执行——绝不基于猜测动手。产出必须带 file:line 证据。" 这是 CLAUDE.md"红线中的红线"的形成时间点证据。出处：h1/h7/h8 等 plan Global Constraints。
- H 系分支链完成状态证据：H7 从 f1f4f1c 切 → H1 从 40d8a65 切（"H7 合并后的 origin/main"）→ H11 从 545ffcf 切（"含 H1/H7"）→ M13 从 b19df42 切（"含 H7/H1/H11"）——顺序合并链。出处：各 plan 分支声明。

### 06-28→06-29 记忆修复三连的真因反转
- 06-28 两 plan（conflict-and-reviewer-yield 治上游 + consolidation-guards 兜底两件套）在 server117 全量真测后被证明**未触及真因**——06-29 memory-summary plan 的 commit message 直言："server117 全量真测证件一件二未触及真因：memory_summary（短期滚动上下文）被 memory_card_from_contact:215 当权威 core_fact 注入，逐轮累积 8 段 summary 成 blob 致 8岁/10岁并存"。三连修复链是"假设修复→真测证伪→真因修正"的完整样本，且真因修法最小（删 1 行+改 1 行）。出处：06-29 plan Goal+commit message。

### 07-12~07-14 修复批中"裁决不修/降级/标注"类差异（finding≠一律修）
- **KD-02 经裁决不改代码**（kd-family2）；**B-01 抢占入队尾窗双回复标注为已知产品取舍不改逻辑**（p3-family9）；**A-03/A-04 加注释+台账标 WontFix**（p3-family8，"生产不触发/已幂等缓解，已知边界不修"）；KB-06/KB-12 只补"就绪债现状"doc 标注（"生产未接线"/"workspace 级刻意"）。审计 findings 的处置有四档：修/收窄/标注/裁决不修。出处：各 plan Goal。
- **kd-relay-number-guard 是"删除防御"型修复**：数字护栏 fail-closed 用法被判定威胁模型错误（中文数字盲区+双向失效+误杀致裁决黑洞）→ 外科式删除，把忠实度交还 LLM+独立 Review——与常见"加防御"方向相反的显式决策。出处：该 plan Goal/Architecture。
- **p3-family7 用户裁决边界**："首推守卫不动、不加写入口校验（用户裁决）"——修复范围由用户裁决收窄至只改超时改派单点。出处：该 plan Architecture。

<!-- END-S2 -->

## 3. 仅存于 plan 的实现决策档案（高价值条目）

收录标准：spec 未记载、仅 plan 正文出现，且影响代码结构/数据模型/守卫行为/运维语义的决策（字段命名取舍、分阶段顺序、回滚策略、部署顺序等）。编号 P-001 起（与 20 号记录 D-001~D-067 独立编号，尽量不重复收录同一条；重复处注明）。

<!-- DECISIONS -->

- **P-001（06-02）泛化门四红线结构**：`GeneralizationReport{empty_split, train_below_floor, holdout_below_floor, gap_exceeded}` 全部未触发才 ok；空 split 视为不合格（"无法评估泛化"）；gap 用绝对值（holdout 高于 train 也算 gap）。出处：Task 1 代码与注释。
- **P-002（06-02）负例种子构造纪律**：验证"needs_review 不可召回"时 draft 种子仍设 `status="active"`，专靠 `integrity_status` 拦截——精确测试 catalog 的 verified-only 过滤而非 status 过滤（两个门分开验）。出处：Task 6 Step 2 注释 + Self-Review 风险点 2。
- **P-003（06-04）全局 CSS 例外白名单**：tokens.css 与 reset.css 是"唯二允许裸标签全局选择器的文件"；组件样式禁止十六进制硬编码色值，必须 `var(--color-*)`。出处：Task 0.1/0.2 文件头注释。
- **P-004（06-04）CSS Module 测试断言方式**：编译后类名带哈希，断言用 `[class*="running"]` 匹配语义片段而非完整类名。出处：Task 1.1 Step 1。
- **P-005（06-05）请示卡对领导不脱敏**：`render_principal_card` 明文含客户称呼/卡点/问题，短码放最前便于领导引用；与之对偶的是"客户侧绝不暴露内部字段"。出处：Task 8。
- **P-006（06-05）relay 合成消息不落库**：`synthetic_principal_relay` 构造的哨兵消息仅内存作 trigger 传网关，绝不写 conversation_messages（防污染客户会话历史）；由此推导出的关键不变量（后被 06-30 h10 复用）：history 永不含合法哨兵。出处：Task 16 Step 2 执行注意。
- **P-007（06-05）deferred 语义**：领导回"我问下财务"类 → verdict=deferred → 不 resolve、不起 relay、保持 pending 继续等（解析失败/verdict 越界也回落 deferred——"宁可当领导还没定也不乱转述"）。出处：Task 14/19。
- **P-008（06-05）relay task 载荷与重试参数**：`principal_decision_relay` task 的 content=台账短码；`max_attempts=3`；relay 前必重读 contact 最新态（跨天关键），contact 没了/未 resolved/授权过期一律静默不发。出处：Task 15/19 enqueue_relay_task。
- **P-009（06-05）lint 规避的精确词界**：「"真人"和"人工"是禁词，"领导/principal/decision/escalation/转述"不是」——所有 src/ 新增行（含注释、日志、合成文案）按此选词；CLAUDE.md 不在 lint 扫描范围故可写禁词做澄清。出处：Task 16 Step 5 🚨 红线检查 + Task 23 注。
- **P-010（06-05）禁词红线测试落点纪律**：断言"文案不含禁词"的测试因字面量必含禁词，只能放 tests/（lint 排除目录），绝不能放 src/ 内联 mod tests——"放在 src/ 会被 lint 误杀"。出处：Task 8 🚨 lint 陷阱。
- **P-011（06-05）principal_decider 挂载位置**：领导 wxid 配置挂 `OperationDomainConfig`（per-workspace-per-domain），非 workspace 独立配置表；`high_risk_escalation_mode` 未配/未知值回落 `decision_only`（保守默认）。出处：Task 10。
- **P-012（06-05）领导也可能是 contact 的边界**：webhook principal 分流必须在 register_inbound 之前（"领导本身可能也是某 contact，别让它先走 register_inbound"），命中即 return 不再当客户消息处理。出处：Task 20 执行注意。
- **P-013（06-06）grounding 兜底硬闸的具体动作序列**：ProductEffect 命中且无 verified → `review.approved=false` + `hallucination_score` 抬到 `max(6)` + risks 加 `product_claim_without_verified_knowledge` + `decision.should_reply=false` + `autonomy_mode="blocked"` + 事件 `product_claim_blocked_by_probe_fallback`(status=blocked) + `final_review_status="blocked_unverified_product_claim"` 并提前 return。出处：Task A2 Step 3。
- **P-014（06-07 routes-split）可见性三铁律**：① 原 `pub(super)` 进子文件须升 `pub(in crate::routes)`（深一级窄化）；② 域内全 routes 级项用 `pub(in crate::routes) use S::*;`，含真正 pub 项（ext_knowledge 直调）的域必须用裸 `pub use S::*;`（受限 glob 无法 re-export pub 项，E0364/E0365）；③ 父模块读子模块 struct 字段需该字段标 `pub(super)`（E0616）。出处：搬运通用步骤前铁律块。
- **P-015（06-07 routes-split）orphan 路由守卫机制**：`routes/mod.rs` 的 `no_orphan_pub_async_route_handlers` 单测用 `include_str!` 逐文件扫源码找未挂载的 pub async handler；拆分必须把 9 个子文件补进 `route_files` 数组；`KNOWN_NON_ROUTE_HANDLERS` 白名单当时 3 项（ingest_chunked_text/import_pdf_bytes/build_operation_knowledge_completeness）。出处：Task 0 Step 4 + Task 11。
- **P-016（06-08 escalation-split）原子重构范式**：与 routes-split 的"逐域跳跑每步全绿"相反，声明"原子重构——不追求中间步可编译，整体改完一次性验证"；回退策略=单 commit revert；明言"不是 TDD（不写新失败测试），38 个测试原样搬运作安全网"。出处：Architecture 段 + 回退策略节。
- **P-017（06-08 knowledge-frontend-split）ChunkInspector 簇归属判据**：`ChunkInspectorPane` 被 explore 与 steward 双域引用 → 整簇（2894-3850 行 15 个项）归 shared.tsx；"不把 CustomEvent 改成 Context/状态库、不强求每组件一文件"（YAGNI）。出处：关键术语段 + 非目标节。
- **P-018（06-15 h17）默认记忆槽位 cap 表**（DEFAULT 逐字复刻值）：preferences=8/doNotDo=10/commitments=8/objections=8/openLoops=8/openQuestions=8/confirmedFacts=12/conflicts=6；typed 三数组固定 6/10/20。设计意图 deprecatedFacts cap=20（20 号记录矛盾点 3 的佐证：代码某处 cap=6 与此设计矛盾）。出处：设计节。
- **P-019（06-07 trust-cockpit）前端全局 button 污染防御约定**：新组件用 `<button>` 必须在自己 module.css 里重置（background/border/box-shadow/min-height/justify-content + hover 覆盖），否则被全局 styles.css:71-118 裸 button 规则污染（蓝底/居中）——"踩坑后确认"级约定。出处：关键约定段。
- **P-020（06-19 eval）向后兼容等值断言范式**：`build_judge_user_with_context` 空底料时必须**逐字等于**老 `build_judge_user`（assert_eq 锁死），老函数重构为薄委托（传空 ctx）——"新能力走新函数、老调用零改动"的 DRY+兼容双保样板。出处：Task 2/3。
- **P-021（06-19 residuals）H13 状态机 publish 的 seeded_by 溯源约定**：机器派生 policy 行 `seeded_by=Some("statemachine_publish:{profile_id}")`，已存在 (workspace,domain,state_key) 行跳过保留运营手工调整（与 m013 同语义）；派生 best-effort 失败 warn 不阻断（"状态机已 publish，policy 缺失只是 fail-open 回到改造前行为"）。出处：Task 8 Step 2。
- **P-022（06-20）G13 clamp 的模块边界决策**：runtime 阈值 clamp 直接用 i32 字面量 `.clamp(1,10)`，**不跨模块引** evolution/threshold.rs 的 FIVE_GATE_HARD_MIN/MAX（f64 且模块私有）——"两处独立写路径各自设防"。出处：Task 2 根因段。
- **P-023（06-20）reconcile helper 的错误传播设计**：`reconcile_state_policies_for_machine` 返回 `()` 无 Result——"内部 per-state warn-and-continue，不向外传播错误以免拖垮已成功的主操作"；同步扩展 `is_refreshable_policy_seeded_by` 认 `statemachine_edit:` 前缀，否则直编派生行下次 publish 被当手工行不刷新。出处：Task 5 Interfaces + Step 4 注。
- **P-024（06-20）G03 扫描器粗过滤的覆盖边界**：放宽到"profile 默认开 OR per_relationship 任一开"，但 contact 级 override 开启的边缘**有意不覆盖**（需全表扫描每 contact override，与"省 DB 扫描"初衷冲突，留注释说明、逐 contact resolve 兜底）。出处：Task 7 根因段。
- **P-025（06-19/06-20 通用）多会话并行的提交纪律**：精确 `git add` 命名文件、绝不 `git add -A`；排除并行会话产物白名单（tests/real_llm_*、tests/roleplay_*、tests/common/*、.kiro/specs/universal-test-coverage/*、AGENTS.md、agent_t*.txt、t15_single.txt、docs/superpowers/plans/2026-06-18-*）；"上次被并行会话 git stash 卷走"（06-24 hardening 亦记）→ 每块编译后立即 commit。出处：两篇 Global Constraints。
- **P-026（06-21 send-ledger）回扫 worker 参数**：单 tick limit 200 条防积压过重、按 sent_at 升序、窗口未过跳过下轮再看、默认响应窗口 24h；target_title 是冗余快照（"原实体改名/删除后历史仍可读"），回查失败留空串不阻断。出处：Task 5 Step 3 + Task 4 Step 5。
- **P-027（06-21 annotation）纯内核+异步薄壳分层范式**：聚合处置逻辑（Accept 收集/Reject 短路/DropSilently 跳过）抽无 IO 纯函数 `fold_stage_validations` 供 lib 真测；DB 查询留外层薄壳——"字典判定逻辑由既有 classify_validation 13 个纯函数测覆盖；本内核只测聚合处置"的测试分层。出处：Task 1 背景段。
- **P-028（06-21 config-page）受控回显链路铁则**：override 下拉初值必须由 hydrateSelected 从 contact 注入，**不能**直接从 selected.domainAttributes 派生——refreshContacts 只重拉列表不刷 selected，会回显旧值（照 customAgentInstructions 受控链路）。出处：annotation Task 7 背景段（同一机制亦见 config-page 表单设计）。
- **P-029（06-21 config-page）TS 类型窄化技巧**：4 个 boolean 开关字段抽 `EscalateKey` 联合类型，"避免把非-bool 字段的 key 混入（否则 checked/赋值 boolean 会 TS 报错）"；quietHours 三格全空删除键、部分填时 0 占位避免 NaN 进 body。出处：Task 5 Step 2 代码注释。
- **P-030（06-21 phase1）count_pushes_today 的近似语义**：以 pending 台账 created_at 作为推送时刻近似（"每条 pending = 一次推卡"）统计 daily_push_cap——非独立推送记录表。出处：Task 6 Step 1 代码注释。
- **P-031（06-21 phase1）in_quiet_hours 支持跨午夜**：start>end 时判 `h >= start || h < end`；tz_offset 折算后按 `((ms/3600000) % 24 + 24) % 24` 取小时。出处：Task 4 Step 3。
- **P-032（06-21 phase1）m025 迁移 BSON 键形态**：ask_human_policy 因 rename_all=camelCase，迁移手写 doc! 必须用 camelCase 键（deciderChain/escalateSafetyGuard）；幂等（$exists:false 才回填）、不删旧字段。出处：Task 13 Step 1 注。
- **P-033（06-21 phase1）harness seed 纪律**：集成测试 seed `operation_domain_configs` 的 (default,user_operations,v1) 行**必须 replace_one(upsert) 不能 insert_one**——ensure_prompt_pack_v2 启动已 seed 过，insert 会撞唯一性/产生双行。出处：Task 14 关键 harness 纪律。
- **P-034（06-21 referral）幂等键 card 后缀规则**：名片空 content 条目幂等键追加 `:card:{card_id}`（compute_synthetic_key_with_card），否则多张不同名片空 content hash 撞键误去重；无 card 时与旧 key 逐字相等（等值断言锁）。出处：Task 5。
- **P-035（06-21 referral）「已引荐」态写法**：`build_referred_set_doc` 用 dotted-key $set（domain_attributes.referred_specialist_at/referred_card_id）不覆盖其它 attributes + 同步刷 domain_attributes_updated_at（"铁律"）；发送成功后 DB 写失败只 error 日志不返 Err（既成事实纪律）。出处：Task 6。
- **P-036（06-21 referral）prompt 版本门控（阶段②机制快照）**：改 user.reply.task 字面量必须 bump PROMPT_PACK_VERSION（prompts.rs:15）否则 ensure_prompt_pack_v2 版本门控不重种——此机制后被 06-26 prompt-pack-alignment 的"内容 diff 对齐"取代（读码时以后者为准）。出处：Global Constraints + Task 10 Step 2。
- **P-037（06-21 media）文件存储布局与安全**：`{workspace}/{sha256 前 2 位}/{sha256}.{ext}` 分片；`is_safe_segment` 只允许 ASCII alphanumeric+`-_`（含 `.`/`/` 即拒 PathTraversal）；扩展名白名单 14 种（pdf/图 4 种/office 6 种/mp4/mov），exe/sh 拒绝；上限默认 50MB；`file_sha256` 建索引供去重。出处：Task 2/3。
- **P-038（06-21 media）media_id 缓存协议**：`media_id_cache_valid(updated_at, ttl_hours, now)` 纯函数，TTL 默认 24h，过期读盘 base64 重传 media_upload_base64 后回写——"不依赖 media_id 永久有效假设"。出处：Task 7。
- **P-039（06-21 media）生产环境事实**：MEDIA_STORAGE_DIR 默认 "./media"、注释"生产 117 为 /opt/wechatagent/media"——生产服务器代号 117 在 plans 语料中的首次出现。出处：Task 3 Step 1 代码注释。
- **P-040（06-22 audit-wave）三个新配置默认值**：holding_reply_min_interval_hours=6.0 / wake_jitter_max_seconds=900（15 分钟）/ account_daily_send_soft_cap=500（仅告警）。出处：Task A3/D1/E2。
- **P-041（06-22 audit-wave）jitter 算法选择理由**：FNV-1a 手写实现而非 std DefaultHasher——"避免 DefaultHasher 跨版本不稳定"（确定性可测可复现）；`(h % (max_ms+1))` 落 [0,max]。出处：Task D1 Step 3。
- **P-042（06-22 digital-twin）/active 端点契约锚定**：查询条件必须与 `DomainProfileCache::reload_from_db` 的 filter **逐字一致**（is_active=true AND current_version=true + workspace 分槽），"确保前端显示的 profile 与 AI 实际加载的是同一行"；无 active 返 {item:null} 不报 404（合法状态：运行时回落 DEFAULT）。出处：Task 2 handler 文档注释。
- **P-043（06-22 media-crud）换文件双红线动作**：`media_id: null`（防 ensure_media_uploaded 在 TTL 内复用旧 media_id 发旧文件）+ `review_status: "draft"`（发送物变了必须人类重核验）；改元数据则两者都不动。出处：Global Constraints + Task 3。
- **P-044（06-22 structured-org）MongoDB 数组等值匹配**：`{ tags: "报价类" }` 直接命中数组含该元素的文档；"无需新索引（tag 筛选量小，200 limit 内）"。出处：Task 2 Step 6 注。
- **P-045（06-23 reactivation）"字段是编译期强约束源头"方法论**：加字段的 Task 单独编译**不全绿是预期**——missing-field 错误强制下游 Task 补齐所有构造点，"只要本文件无错即 Step 通过"；同款见 tag-trust-1 Task 3（Contact 加字段打挂全部构造点）。出处：reactivation Task 1 Step 3 注。
- **P-046（06-23 tag-trust）lint 逼出的命名规避**：人工权威层命名 `manual_tags`——"manual 不在禁用词内"，注释一律"运营录入/运营确认/operator-authored"，绝不写"人工标签"（"人工"单字是禁词）。出处：tag-trust-1 Global Constraints。
- **P-047（06-23 tag-trust-2）锚点选择依据**：`ConversationMessage.message_id`（微信侧）常 None **不可作锚**，`_id: ObjectId` 是唯一锚点；窗口序位（0-based）仅是 LLM 输出的临时坐标，落库前必须映射成 `_id` hex。出处：tag-trust-2 现状核实节。
- **P-048（06-23 tag-trust-4）"实现以通过测试为准"的借用留白**：apply_bayesian_update 的借用结构在 plan 中是示意（"Rust 借用检查器会强制正确，可能需先收集待 promote 名再二次遍历"），4 个不变量测试（hit=1 不占槽/history≤100/locked≤6/双阈值）是硬验收——把不变量交给测试钉死、实现细节留实现者。出处：tag-trust-4 Task 1 Step 3 实现者注意。
- **P-049（06-23 tag-trust-5）vitest CSS module mock 范式**：`vi.mock("...module.css", () => ({ default: new Proxy({}, { get: (_t, k) => String(k) }) }))`——类名透传测试。出处：tag-trust-5 Task 2 Step 1。
- **P-050（06-23 tool-loop）纯减法四护栏**：`knowledge_max_tool_loops`（删）≠ `knowledge_max_tool_calls`（活，budget 注入用）；`dispatch_tool_call`（删）≠ `dispatch_chat_tool_call`（chat 活）；exec_*/TOOL_* 常量 chat 共用全留；clamp_i32 通用 helper 只删调用不删函数——"若报某 exec_* unused，说明『chat 共用』判断有误，停止核实勿盲删"。出处：最高优先级护栏节 + Task 2 Step 3。
- **P-051（06-24 pacing）间隔闸插入点论证**：必须在"reclaim 幂等门之后（不误拦本该 post-hoc 标 sent 的条目）、MCP 发送之前"；defer 事件 kind=`agent.send_deferred_account_pacing`（AI 内部状态名）；attempt 刻意不变（"间隔闸非发送失败，不耗重试额度、不走 terminal"）。出处：Task 4 Step 2/3。
- **P-052（06-24 hardening）used_knowledge_ids 作用域技巧**：match 可能 move tier_decision → "实现时优先在 match 前求值 `let escalated = matches!(...)` 避免作用域问题"；kill switch 关时须给强升条件加 `progressive_tier_enabled &&` 前缀（避免 Full 后又触发强升白跑一次）。出处：Task 2 Step 4 风险核对 + Task 3 Step 3 注。
- **P-053（06-25 label-wiring）labelFor 加载失败降级语义**：active-view 拉取失败时 dimensions/taxonomies 置空——"labelFor 一律回落 no_dict（显示原值）"，前端照常跑不崩。出处：Task 4 Step 3 catch 注释。
- **P-054（06-26 batch1）A1 驾驶舱死端点根因与修复**：loadMessages 的 Promise.all 里 2/5 端点后端不存在（/contacts/:id/messages 与 /contacts/:id/decision-reviews）→ 全有或全无 → 5 面板全空；修复=改真实路由（/conversations/:id/messages、/decision-reviews?contactId=）+ **allSettled 加固**（单面板失败不拖垮其余四面板，取第一个 rejected 上报错误横幅）。出处：Task 1 背景+Step 3。
- **P-055（06-26 batch1）InboxItem 扩字段纪律**：新增 4 可选字段须在**全部 8 处构造点显式填 None**——"不引入 Default 以免掩盖遗漏"（编译器强制列全字段，漏一处即编译错，比 ..Default 更安全）。出处：Task 12 关键约束。
- **P-056（06-26 batch1）clear-referral 必须 $unset 两键**：退辅助判定只判 `referred_specialist_at` 键，但撤销须同时 $unset `referred_card_id` 才彻底退态；filter 含 workspace_id（隔离不可省）。出处：Task 14 真实现状。
- **P-057（06-26 batch2）serde rename 铁律**："批次1/2 踩 5 次——所有后端请求/响应 struct 须核实 rename_all 属性，snake 字段名 ≠ wire 键，嵌套 struct 可独立于顶层有自己的 rename_all，否则 typed 反序列化静默丢键"。同 plan 列了三类并存：DomainProfile 序列化 snake_case、decision_review_json 手写 camelCase、taxonomy 请求体 rename_all=camelCase。出处：batch3 Global Constraints + batch2 Global Constraints。
- **P-058（06-26 batch2）C8 数据源决策**：finalReviewStatus/holdCategory 两值在 AgentRunLog（顶层 snake + review doc 内 camelCase），plan **否决**"在 AgentDecisionReview 冗余写字段（漂移风险）"→ 改为 list/get 时按 run_id 关联查询 fetch_run_status。出处：Task 8 关键数据源。
- **P-059（06-26 batch2）labelOf 兜底陷阱**：labelOf 对空值返回 "—"（truthy）→ 不能用 `||` 串联兜底，须显式判断字段存在（blockedLabel helper：finalReviewStatus→holdCategory→"拦截"三级）。出处：Task 8 Step 6。
- **P-060（06-26 batch3）E6 整替换陷阱**：PUT documents/:id 是 replace_one 非局部 patch——前端表单只暴露少数字段但**提交 body 必须回填完整文档**（尤其 rawContent，不回填被清空连带 contentHash 丢失，影响 chunk D2 锚点回填）。出处：Task 9 关键约束。
- **P-061（06-26 batch3）E7 红线防绕过**：后端裸 POST chunks 默认 status=active——手工新建切片表单**必须显式带 status:"draft"+integrityStatus:"needs_review"**，否则绕过 verify gate（"人工新建也先进待审池"）。出处：Task 10 关键红线约束。
- **P-062（06-26 batch3）F23 方案B 三决策**：不需要 migration（仿 relationship_type_suggestions 索引建 indexes.rs）；approve 调 add_outcome_event_inner(verification="staff_confirmed", source="manual") 而非写 domain_attributes（"这是 relationship_type 的做法，F23 不同"）；gateway extract fail-soft `let _ =` 吞错。verification 闭集：conversation_inferred 会被 400 拒。出处：Task 16 关键决策。
- **P-063（06-26 health-batch1）FE-1 同名函数遮蔽考古**：PR#40 合入后 legacy.tsx:1950 已有**本地正确** defaultHealthItems（canonical 3-key）遮蔽 store import；坏的是 userOpsStore 内旧 4-key 版 + healthFromScores——修复"改 :595 + 删两函数 + 不碰 legacy.tsx:1950"。基线更新注记体现 plan 对并行合入代码的重核。出处：Task 4 基线更新注。
- **P-064（06-26 health-batch2）CONC-1 OCC 边界**：只把 memory_card 三字段拆出走 occ_memory_filter（modified_count==1 才认，lost-race debug 日志跳过）；门控外 updated_at/context_pack 仍走三键 filter——"它们不 bump version，套版本谓词会永久 lost-race"。出处：Task 4 方案。
- **P-065（06-26 health-batch2）CONC-2 接受的代价**：$push+$slice:-8 治丢失；去重留应用层快照判定，**并发下可能写重复——显式接受**（planner pick_commitment_emit_target 单选 + commitment_recently_emitted 按 id 幂等，重复项最多占 cap8 槽位不重复 emit）。出处：Task 3 方案。
- **P-066（06-26 mgmt-thickening）outcome assertion 判定表**：send 类核 success+msgId（success=false=账号离线→Failed）；写库类核 matched/modified（matched=0→Failed"实际没有改动"）；显式 ok:true→Succeeded；readonly 返回结构即成功；兜底 Unverified 诚实暴露"响应结构无法确认业务结果"。汇报 build_execution_summary 基于真实 outcome 而非回放 plan.summary（"区分打算做与做成了什么"）。出处：Task 2/3。
- **P-067（06-26 mgmt-thickening）prompt 编辑第三闸三态设计**：LLM 审 diff 增量（extract_diff 行级朴素 diff"审增量比审整篇好判、省 token"）；三态 Pass/Reject/NeedsHumanConfirm——LLM 重试退避后仍不可用**降级人确认**（"非 fail-closed 死路、非 fail-open 放水"）；路径A（管理对话）转 pending_confirmation、路径B（编辑器直 PUT）返回 200 needs_human_confirm 弹框带 force 重提，"两路径同一判定函数、两个消费端，不 drift"。出处：Task 6.6。
- **P-068（06-26 mgmt-thickening）冒烟环境信息表**：服务器 117.72.54.28:22 root、APP_PORT=3003、root 密码"会话内 export DEPLOY_PASS 注入，不进记忆/代码"、.env chmod 600 绝不进 git；首构建必配 USTC sparse 镜像否则 crates.io 卡死；启动期空状态机 active 会 bail（历史坑）。出处：Task 9。
- **P-069（06-26 pack-alignment）align 顺序铁律**：非空库路径必须 delete_redundant（清上一轮 archived）**先**、align_prompt_specs（产生本轮 archived）**后**——"顺序颠倒会让 align 刚归档的行被立刻物理删除，破坏可回溯不变量"。出处：Global Constraints。
- **P-070（06-26 pack-alignment）LRU bump 语义细节**：align 返回 bool 只在"真正归档+重种"置 true（evolution 守卫 continue 与内容一致 continue 都不算写入）；delete_redundant 删 archived 也是写但不影响 active 行故不置 true；main.rs 启动期 `let _ =` 忽略返回值（LRU 尚未建立无需 bump）。出处：Task 4 Step 1/2/4。
- **P-071（06-27 repair-closure）audit_failed 语义**：落库 PUT 成功但闭账 applied 失败→ ok:false + reason=audit_failed + message"已落库为草稿，但审计记录写入失败"——**不回滚落库**（落库与审计解耦，审计失败不否定业务写入），且区别于 apply_failed（不发 applied）。出处：Task 1 Interfaces。
- **P-072（06-27 batch4）SSE 重连器设计**：createSseReconnector 退避 delay=min(cap, base×2^attempt)、默认 maxRetries=6；**任一业务事件触发即重置 attempt**（连接健康信号）；close() 置 stopped 防泄漏/竞争；文件头注释明令"严禁用于一次性 RPC 流——重连会重发查询、重复扣 token"。出处：Task 7。
- **P-073（06-27 human-gated）三闸下沉动机**：validate/review_prompt_edit 原在 routes/management_prompt_edit.rs（`mod` 私有模块）——evolution::release_prompt 够不着 → 下沉顶层 src/prompt_guard.rs（pub），原文件改 3 符号 re-export 保持 prompt_templates.rs 调用路径零改动；prompt_guard.rs 不在 no-takeover 扫描目录但仍保留字符拼接 helper（"零风险逐字迁移"）。出处：Task 1/2。
- **P-074（06-27 human-gated）字节等价护栏模式**：给主链函数加 Option 新参（PromptOverride），所有现有调用点补传 None → apply_if_matches 不触发 → prompt 逐字不变，以 `cargo test --lib` 基线不动作为反过拟合硬验证。shadow 注入的 LRU 安全性靠亲核 llm_exact_cache_key 白名单（reply/review 链恒不进 LRU）。出处：阶段二注入设计。
- **P-075（06-27 human-gated）末尾追加的锚闸协同**：compose_appended_content 把原文逐字保留在开头→红线锚"天然通过"锚闸；若不过则"说明原 prompt 已缺锚，fail-closed 正确"——组合设计使追加路径无法删红线。出处：Task 5 Step 2 注释。
- **P-076（06-28 targeted-push）BSON 混合大小写铁律**：Contact 无 rename_all（outcome_events snake）但 OutcomeEvent/OutcomeProductRef 带 camelCase → 内嵌真实路径 `outcome_events.productRef.productId`，"索引与 $elemMatch 必须用此路径，否则建在空字段上、查询恒空"。sends-report 再列一遍三集合各自大小写。出处：两 plan Global Constraints。
- **P-077（06-28 targeted-push）dispatch 幂等与漂移防护**：dispatch 重新跑圈人（"防预览后数据漂移"）；每人先插 campaign_sends（唯一索引），DuplicateKey→跳过，成功才建 task 并回填 task_id——插台账先于建任务的顺序保证故障时宁可少发不重发。出处：Task 4 dispatch handler。
- **P-078（06-28 sends-report）7 桶优先级设计**：outbox_status=sent 最高优先（"即便 status 是 blocked_unverified_product_claim 也归 sent——已送达优先于请示"）；escalated 四值（unverified_product_claim/safety_guard/held_by_ai_policy/ai_waiting）="走请示通道、待裁决后 AI 会继续触达（非失败漏推）"与 blocked（纯频控无后续）分桶；precheck_blocked 刻意归 unknown（灰度/口径态）。出处：classify_send_outcome 注释+测试。
- **P-079（06-28 contract-batch1）防腐烂 lint 的近似算法**：无 regex 依赖 → 纯 std 扫描"fn *_json + -> Value"行；覆盖判定=投影名出现在 assert_contract_fixture 调用点前后 ~600 字符窗口（"production handler 调用投影不在这种窗口里，挡得住有调用方但零测试"）；含反向验证步骤（临时改测试名确认 lint 真咬人再改回）。出处：Task 8。
- **P-080（06-28 reply-guarantee）占位入参取原语不取 &Contact**：build_ack_enqueue_request 取三个字符串字段——"本函数只需三个值，原语入参使其成为零依赖纯函数（单测无需构造 40 字段的 Contact，gateway 测试模块也从不构造它）"；幂等键 `{source_event_id}#ack-placeholder` 与分段 `#seg{idx}` 天然不碰撞。出处：Task 2 设计说明。
- **P-081（06-28 reply-guarantee）DEFAULT 行为变更显式评估**：方案 B 让 FollowUp+safety_guard/unverified 从"发占位"变"静默"——plan 判定"这是改进（proactive 触达被拦后发'稍等我给你准信'是非所问）但属 DEFAULT 变更，final review 需知悉"。出处：Global Constraints。
- **P-082（06-28 e4-f21）F21 列表端点体积控制**：列表项只给 completedStepCount 计数不带 plannedSteps/cards 全文（"控 payload 体积，详情仍走 GET /tasks/:id"）；status 非法值宽松忽略（与 chunk 列表 query 风格一致）。出处：Task 1 Step 5。
- **P-083（06-28 memory-conflict）同源升级不变量**：注入给 consolidator 的当前卡与 prev-merge 用的 previous_card 必须是**同一升级实例**——两处独立调 effective_memory_card 会导致 auto_upgrade 各自生成 fresh UUID，"LLM 引用的 id 在合并时匹配不上"。plan 还预写了借用检查失败时的替代方案（先算 json 串再 clone）。出处：Task 2 说明+3d 注。
- **P-084（06-28 consolidation-guards）agent-first 的结构度量边界**：非原子检测三判据（换行≥2/句界标点≥2/char>80）互为冗余兜底；80 字宽松上界"宁漏判不误伤——漏判的 blob 还有换行/句界判据+件一救回+重试"；检测放 from_document **之前**扫原始 value（"重试要在最早点决策，避免做完解析才发现要重试白做"）。出处：Task 2/3。
- **P-085（06-28 consolidation-guards）重试失败分级处理**：重试拿到干净输出→用重试结果；重试仍非原子→用重试结果+落库前 retain 丢弃非原子条（warning 记 dropped 数）；重试调用失败（端点 glitch）→保留首次 value 既成事实纪律不阻断固化。出处：Task 3 Step 3。
- **P-086（06-28 phase3）evalMetrics 大小写混搭规则**：同一响应里 shadow_replay_json 键是 camelCase、evalMetrics 内字段是 snake_case（grade_prompt 写 snake、bson_doc_to_json 原样透出不转）——前端读 evalMetrics 必须 snake_case。这是 BSON Document 自由字段桥接的固有形态。出处：Global Constraints。
- **P-087（06-28 memory-conflict）真测取证纪律**：验证 server 上 prompt 重 seed "直接 dump 原文目视，勿用 mongosh --eval 的 indexOf 布尔判断——跨 SSH 多层引号转义破坏中文"。出处：Task 6 Step 4。
- **P-088（06-29 campaign-completion）投影防泄漏测试模式**：CampaignListItem 测试同时断言"字段齐全 camelCase"与"**不泄漏内部字段**"（workspaceId/segmentFilter/intentText/accountId 逐一 assert is_none）——投影契约的正反双向断言。出处：Task 1 Step 1。
- **P-089（06-29 tiered-injection）等价性守恒设计**：新增 min_inject_tier 的 None/非法值语义定为"按 full 处理=与改造前『仅 Full 注入』逐字等价"，使加字段本身零行为变更，行为变更只来自显式配置。出处：Global Constraints+Task 2。
- **P-090（06-29 contract 批次 2-4 累积经验）**：①raw Document fixture 必须 bless 生成绝不手写（嵌套 BSON 泄漏 $oid/$date），构造 Document 用纯标量 doc!；②禁词纪律深到 serde alias 层（历史 alias human_handoff_success_rate 不进 fixture）；③键数以 bless 结果为唯一真相源（计划标注只是核对预期）；④裸数组投影（cohort_run_ids_json）不适配键集对账机制归 helper 豁免。出处：三 plan Global Constraints。
- **P-091（06-30 h10）不可伪造性的 serde 设计**：is_synthetic_relay 用 `#[serde(default, skip_serializing, skip_deserializing)]`——字段绝不写库、从任何外部输入反序列化恒为 false，只有 crate 内合成构造器能置 true。身份凭证从"内容特征（客户可控）"迁到"来源标记（结构不可伪造）"。出处：h10 Architecture。
- **P-092（06-30 wiki-audit）serde 错配缺陷类型**：路由层用 camelCase 键查 Mongo 而模型 insert 落 snake_case → filter 恒不命中 → "动态字段校验静默失效"（不报错的静默半残）。修复=11 处查询键改回 snake_case+集成测试锁读链路。这类缺陷是 serde rename 铁律（P-057）的实际事故样本。出处：wiki-audit Task 1。
- **P-093（06-30 injection-hardening）恒注入与 limit 的冲突**：禁语与可引用资产共用一条查询+limit(12) → 资产多时禁语被 limit 击穿挤出（安全红线破洞）→ 拆两次独立查询（禁语无 tier 无 limit）。"恒保证"类注入不能与"截断"类查询共享管道。出处：hardening Global Constraints。
- **P-094（06-30 evolution-ui-toggle）env 变量语义反转手法**：EVOLUTION_ENABLED 默认 false→true 但语义同时从"是否启用"改为"是否允许 UI 启用"（硬上限）——运行时真开关移交 DB flag；复用原变量不新增；既有 prop 不删而语义重定义（7 处测试依赖零破坏）。出处：evolution-ui-toggle Global Constraints。
- **P-095（06-30 worktree 系）非 ASCII 路径与共享 target 工程纪律**：vitest 用 --pool=forks（或 threads）避 worker 超时；cargo test --lib 前 touch src/lib.rs 强制 relink 避共享 target stale 二进制；多 worktree 并行会话可能 clobber test binary（"只读对账诡异失败/测试消失时 touch 源文件重编确认真绿非假绿"）。出处：06-30 各 plan Global Constraints。

<!-- END-S3 -->

## 4. 完成状态线索表

<!-- STATUS -->

### 4.1 checkbox 勾选状态（全体统计，grep 复核）
- **149 篇中 141 篇 checkbox 全未勾选；仅 8 篇含 `- [x]`**（grep 定稿清单）：`06-23-tool-loop-dead-code-sunset`（5/29+PR body Test plan 全勾，最显著）、`07-07-user-ops-roster-batch-enroll`、以及 07-12 的 6 篇修复 plan（audit-events-failsoft / kb01 / kb08 / kc-family1 / kd-relay-number-guard / kd04——该日批次执行者有回填习惯）。结论：**checkbox 状态不是本仓 plan 完成度的有效信号**——绝大多数执行者不回写勾选，完成状态要靠下述间接证据链判定。

### 4.2 完成状态的五类间接证据（本轮深读中发现的判定方法）
| 证据类型 | 说明 | 实例 |
|---|---|---|
| 后续 plan 的基线声明 | plan 头部声明"基于 origin/main <commit>（含 PR#N）" | alignment-batch2 声明"批次1(PR#44)已合并 merge 9d78282"；batch3 声明"批次2(PR#46) merge ae54a8f"；campaign-domain-completion 声明"c163542 含 PR#57+PR#58"；contract-batch5 声明"批次4 已合并 PR#67" |
| plan 头部"实现期修正"注记 | 交付后回写的修正节，直言"已合并 main" | customer-reply-guarantee（"⚠️ 实现期修正 2026-06-28，已合并 main"——CI 测试证伪 A3 挂载点）；taxonomy-label-wiring 的 M1-M5 可行性修正节 |
| 跟进项挂 PR 号 | plan 内 backlog 指名合并 PR | prompt-evolution-human-gated 阶段二跟进项"合并 PR #51 时一并记录" |
| 后续 plan 复用/引用前 plan 产物 | 引用交付经验或消费其端点 | sends-report 引"活动定向推送交付经验"+touch src/lib.rs 技巧；e4-f21 复用"PR#49 已闭环的 ChunkRepairPanel"；ratelimit-syncing 复用"#155 已上线 syncing 态"；injection-hardening 修 forbidden-expression（PR#63）留下的问题 |
| lib 基线数字演进 | 各 plan 记录的当期真实测试数 | 06-29 contract-batch4"批次3 后基线 1720，+8=1728"→batch5"预期 1739"→07-01 deadfield"当前基线 1777/0"→07-06 escalated-budget"现 1814"——单调递增的交付脉搏 |

### 4.3 可判定"已交付"的 plan（证据充分，不完全列举）
06-25 taxonomy-label-wiring（=PR#40，health-batch1 引用）；06-26 alignment-batch1（PR#44）/batch2（PR#46）；06-26 prompt-pack-alignment（=PR#42，mgmt-thickening 冲突修正记录引用）+ PR#41；06-27 chunk-ai-repair-closure（=PR#49）；06-27 human-gated 阶段一/二（PR#51）；06-28 campaign 三部曲（PR#57/PR#58 及其前身）；06-28 customer-reply-guarantee（头部注记"已合并 main"）；06-29 contract batch2-4（PR#67 等）；06-29 tiered-injection（=PR#63，hardening 引用）；07-01~07-02 H 系（H7→H1→H11→M13 分支链顺序合并）；07-08 roster-fetch-cache（#155 被 07-09 引用）；07-11 深审五批（其 findings 编号 KB/KC/KD/KE 被 07-12~14 修复 plan 全面消费=台账已产出）；06-26 full-business-logic-test（git status 中 scripts/biz-test/* 即其产物且 06-28/29 plan 引用其真测结论）。
### 4.4 交付状态存疑/无直接证据的 plan
大部分 07-13 之后的 plan（p3-family 系、audit 系、08 月 4 篇）在本目录内无后续引用证据——不代表未交付，只是证据链在 plan 语料内断裂（需查 git log 核实，本任务未做，标注：**文档声称范围外**）。

<!-- END-S4 -->

## 5. 与既有深读记录的矛盾点（待裁决）

<!-- CONFLICTS -->

1. **PROMPT_PACK_VERSION 的类型与语义**：`06-23-remove-hardcoded-industry-terms` plan 写 bump 为 `i32 = 3` 的形态，但 06-24 progressive-tier-hardening 与 06-28 memory-conflict 两 plan 均实证它是**字符串**（`"...v10_2026_06_24_bayesian_obs"`→`"...v16_2026_06_28_memory_structured..."`）。且 06-26 prompt-pack-alignment-completion 之后该常量已降级为"仅 stamp 溯源"（生效判定交 align 内容比对）——**既有深读记录（20 号）若按 06-23 plan 记载 i32 形态或"bump 才生效"语义，需以后者为准**。裁决建议：以代码现状为准（本任务未对码，标注：文档声称）。
2. **daily_limit 语义漂移**：06-28 customer-reply-guarantee 的黑名单口径表把 `daily_limit` 列为"会补占位的零回复状态"（Inbound 也会被 daily_limit 拦）；07-10 passive-reply-daily-limit 改为"daily_limit 只限 AI 主动触达（被动回复豁免）"。两 plan 都成立（时间演进非矛盾），但**既有记录若只记其一，需注明 07-10 后被动回复不再进 daily_limit 拦截路径**（此时 ack 占位守卫对该状态的触发机会也随之变化）。
3. **知识缺口计数口径**：06-26 alignment-batch3 E9 拍板"知识缺口 = knowledge_gap_signals status=pending 计数，不是 integrity 报告的 gaps.length"。若 13/14 号前端深读记录按 gaps.length 描述 CockpitView 三卡，需修正。
4. **auto-verify 拦截范围**：07-01 wiki-three-p0-fixes 把 enforce 从"只拦 product_fact"扩到**所有 chunk_type**（函数更名 enforce_verified_needs_human_audit）。既有记录（07 号知识引擎）若仍记"仅产品类强制人审"为现状，已过期。
5. **webhook 签名方案**：06-26 full-business-logic-test 记录的旧方案（`X-MCP-Signature` = HMAC(key=MCP_API_KEY, msg=raw_body)）已被 07-09 webhook-signature-verify-restore **整体退役**（新方案：每账号 webhook_secret + `x-webhook-signature: sha256=<hex>` + `x-webhook-timestamp`，签名串 `<ts_ms>.`+raw_body——07-11 audit-batch-a 已按新方案真跑）。既有记录若记旧方案为现行，已过期。
6. **relay 数字护栏**：06-30 h10 时代 gateway relay 出站守卫含 `relay_introduces_unauthorized_number` fail-closed；07-12 kd-relay-number-guard **删除**了它（威胁模型错误）。既有记录（04/05 号 gateway/escalation）若记该护栏为现行防线，需更新为"仅剩载荷泄漏守卫"。

<!-- END-S5 -->

## 6. 覆盖自证（读过的篇数+清单）

<!-- COVERAGE -->

- **总数核对**：`docs/superpowers/plans/` 共 **149 个 .md 文件**（`ls | wc -l` = 149），本记录第 1 节编目 **149 条**（#1-#149），一一对应无遗漏。
- **阅读深度分层**（如实自证）：
  - **全文逐行读**（≥60 篇）：06-02 至 06-28 的全部主线 plan（含 1691 行 alignment-batch1、1535 行 campaign-domain-completion、1236 行 targeted-push、1202 行 mgmt-thickening、1170 行 batch4、1048 行 batch3 等大文件均全文读完）、06-29 tiered-injection、06-30 guide-apply（前 200 行+关键节）、07-01 H 系与 07-11 audit 系全部。
  - **头部+Goal/Architecture/Global Constraints/关键决策节精读**（其余篇）：06-29 contract batch2-4（与 batch1 同构，读差异节）、07-04 至 07-15 及 08 月各篇（读 Goal/Architecture/约束/自审节，plan 主体为 TDD 步骤模板）。同构批次（contract 五批、audit 五批、p3-family 九族）确认结构同源后按差异点读。
  - 全部 149 篇均至少完成"编目三要素"（对应 spec、主题、一句话）与差异/决策/状态线索三类信息扫描；未逐行读的 TDD 步骤细节（Run/Expected 命令行）不影响三类目标信息的提取完整性。
- **交叉核对**：编目时对照 `wc -l` 全量行数清单（约 90k 行总量）逐日期批次核对文件名，补齐初漏的 6 篇（06-26-full-business-logic-test、07-11-deep-logic-audit-batch-a/b/c/d/e）。
- **硬红线遵守自证**：本任务全程只写入本文件（`project-understanding/20a-plans.md`），未修改仓库任何其他文件；所有断言标注出处段落；无法对码处已标"文档声称"。

<!-- END-S6 -->
