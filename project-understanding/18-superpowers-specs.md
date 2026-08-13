# superpowers specs 全集深读记录（核证日期 2026-08-13）

> 语料：`docs/superpowers/specs/` 全部 165 篇（约 27,382 行 / 2.6MB），逐篇全文深读；`docs/superpowers/plans/` 149 篇读清单并抽读重大 spec 对应计划头部。
> 纪律：本文所有断言均标注出处文件名；无法与代码对照处标"文档声称"。

## 1. 全清单编目（165 篇）

说明：状态列取自文中显式标注（Status/状态字段或正文声明）；未标注者记"未标注"。主题域分类为本记录归纳。

| # | 日期 | 文件 | 主题域 | 状态 | 一句话提要 |
|---|------|------|--------|------|-----------|
| 1 | 2026-05-25 | knowledge-base-cleanup-design | 知识/守卫/清理 | Approved（route A 9-commit big bang） | 旧销售话术 RAG 全面清理→wiki 方法论独占：删 Item struct/销售域 Chunk 字段/customer_stage/三 taxonomy seed/fact_risk·pressure_risk·product_accuracy 三阈值；5 闸→3 闸（grounding/hallucination/budget）；baseline PBT 由 string_fact_risk_guard 换 wiki_chunk_revision_pbt |
| 2 | 2026-06-02 | knowledge-closed-loop-trajectory-design | 知识/测试 | 已批准设计 | 闭环轨迹测试（维护 agent 编辑→再召回→召回维持）；RL 概念映射仅作脚手架不建框架壳；红线（cite⊆opened、draft+needs_review、SUPERSEDE 打标不删）优先于 judge 分 |
| 3 | 2026-06-04 | frontend-ui-refactor-design | 前端 | 未标注 | 13165 行 App.tsx+7488 行 styles.css → 领域切片+CSS Modules+Zustand（4 store）；视觉三支柱：易读优先/功能元素呼吸/6 语义色 token；Zustand 5.0.14 依赖审计通过 |
| 4 | 2026-06-04 | recall-rate-benchmark-design | 知识/测试 | v2 设计（待复核转 plans） | recall@k 基准：reach（触达）/adopt（采纳）两层分开；adversarial 由 bigram 重叠客观划分防作弊；跨轮稳定性⓪先行；6 行业矩阵；第一轮不设硬 floor |
| 5 | 2026-06-05 | principal-decision-channel-design | 请示 | 已批准设计 | 决策请示通道：AI 撞墙分两类（缺知识→KB 闭环；缺决策→请示领导）；幕后领导模式不破无人工接管红线；短码请示卡+自然语言弱匹配回执；relay 豁免四道频控；授权带过期时间 |
| 6 | 2026-06-06 | kb-business-probe-design | 知识/测试 | 设计已批准 | KB 业务仿真探针：文档入库→召回→维护→补全六阶段弧，N 轮累积观测召回随库规模趋势；双轨判据（硬轨红线/软轨 judge 异族 deepseek）；只产报告不自动改（反过拟合三焊缝） |
| 7 | 2026-06-06 | knowledge-grounding-and-verify-gate-fixes-design | 知识/守卫 | 设计待批准 | 两处红线收口：A=reviewer 漏判 grounding 探针抬为「词类型切分」硬闸（效果/数据 5 词拦、语气 3 词仅观测，避免恢复 05-25 删掉的脆弱 string-marker）；B=create/PUT 人工后门必须过 D2 门（verified 缺 quote/anchor 降级 needs_review） |
| 8 | 2026-06-07 | knowledge-routes-split-design | 知识/重构 | 已批准设计 | knowledge.rs 9378 行/216 函数拆 9 子文件（facade re-export，外部路径零改）；纯机械搬运零逻辑改动；54 个尾部单测须随函数分发 |
| 9 | 2026-06-07 | knowledge-trust-cockpit-frontend-design | 前端/知识 | 设计已定稿（mockup 逐屏确认） | 可信度治理驾驶舱：把后端可信度治理逻辑（D2 闸/answeringMode/5 维认知矩阵/auto-verify）在前端"显形"，大白话零行话；apply+verify 合并为运营单一动作"让 AI 可以用这条"；澄清 auto-verify 是"唯一 AI 判定可直达 verified 的路径"（运营授权批处理，主体是运营） |
| 10 | 2026-06-08 | escalation-split-design | 请示/重构 | 未标注（纯搬运） | escalation.rs 1274 行拆 mod/logic/ledger 三文件；38 个纯函数测试随 logic 走零路径调整；唯一可见性变更 is_duplicate_key_error 升 pub(crate) |
| 11 | 2026-06-08 | knowledge-frontend-split-design | 前端/重构 | 未标注（纯搬运） | knowledge/index.tsx 6582 行按 4 mode（today/explore/steward/atlas）+shared 拆分；组件间零 prop 耦合（window CustomEvent 通信）是低风险前提 |
| 12 | 2026-06-11 | universal-domain-adaptation-design | 通用域适配（总纲） | Phase 0/1/1.5/2/3/4 全部完成并合并 main（PR #27，2026-06-19）；残留 2026-06-20 收口 | 通用化总纲：H1–H19 硬编码点全图+分期落地记录；DomainProfile 行业总装配单+DEFAULT 逐字等价护栏；正解=「对话生成稳定配置」拒绝「运行时自由发明维度」；数字分身=关系类型×驱动力组合、100% 全自动发送；scan_calendar/renewal/reactivation/G6 扫描器家族 |
| 13 | 2026-06-15 | objective-purchase-facts-design | CRM 客观事实 | 设计评审稿；§2-§5 已落码（2026-06-16 commit fa1215f 追记） | G2 产品目录/G3 成交关联（OutcomeEvent+verification+product_ref 快照）/G4 持有投影（派生不存储）；成交真相源三级可信度（conversation_inferred/staff_confirmed/payment_verified）；红线「AI 永不自断成交」；金额整数化（分）；transaction_facts_enabled 显式交易域总闸 |
| 14 | 2026-06-15 | roleplay-fuzz-testing-design | 测试 | 设计修订版待评审 | AI 角色扮演模糊测试：fixture/校准→固定场景→roleplayer→judge→场景生成五阶段，每阶段只引一个新变量；失败必须带 suspected_layer 归因；reviewer 是独立变量（销售域偏见证据 file:line 列举）；quiet_hours/预算/prompt 覆写时序坑全记录 |
| 15 | 2026-06-18 | dimension-registry-and-validation-design | 通用域适配/画像 | 已落码（2026-06-18） | 中央 DimensionRegistry（7 维度 const 表：channel/typed/是否决策/取值来源）+ validate_dimension_value 三通道差异处置（Admin=Reject、LLM=DropSilently+审计、Reaction=归一）；实施中新增 WriteIntent{AdminWrite,MachineWrite} 正交轴；lib 1308/0 |
| 16 | 2026-06-18 | universal-business-gaps-completion-design | 通用域适配/prompt/数字分身 | 已落码（2026-06-19），终审 Ready to merge | 三模块：A=mode_gate_policy_override+reviewer_fewshot_override（销售散文可替换、boundary 红线不纳入）；B=relationship_type LLM 识别建议-审核链（保守闭环：LLM 只写建议表、approve 是唯一 contact 写点）；C=prompt override per-chain 收敛约定；实证裁决"五闸可配=过度设计（砍）、driver 框架=高风险低收益（缓）" |
| 17 | 2026-06-19 | evaluation-system-overhaul-design | 评估/测试 | 设计待批 | 评判体系统一重构：J1-J6 评判失真（判 grounding 不给 ground/判一致性不给记忆/roleplayer 无校准/无对话级总评）；统一内核 judge_conversation 全上下文喂养；词表硬门（HANDOFF_MARKERS）彻底下线改多采样跨家族中位数 LLM 硬门；吸收 build_judge_rubric 不另起炉灶 |
| 18 | 2026-06-19 | universal-residuals-completion-design | 通用域适配 | 已落码（2026-06-20） | 通用化最后三残留收口：H13 状态机本体随 profile（路径 B：引导层联动 publish，本体单一存 operation_domain_configs 不造双真相源）；H17 轨迹维度容器化（objection_type 旧字段保留兼容）；H18 debounce 窗口 per-profile |
| 19 | 2026-06-20 | universal-audit-remediation-design | 通用域适配/审计修复 | 已批准（用户拍板范围） | 13 域深审+36 agent 证伪后的修复批：G21 多租户首屏画像取错 workspace、G13 五闸阈值 clamp、G01 grounding bypass 漏加在 review_passed、G06 直编路由不派生 policy、G07 负反应率未 profile 化等 4 必修 Medium+全部 Low；数字分身口吻分化明确单独立项 |
| 20 | 2026-06-21 | agent-send-ledger-design | 发送治理 | 设计已获批 | 簇 A：agent_send_ledger 共享发送事实表（素材+名片对称功能）；防重发=prompt 软约束不设硬门（agent-first）；转化=响应率+阶段推进（无 LLM 确定性回扫）；写入 fail-soft（返 Err 会致 dispatcher 重发——红线） |
| 21 | 2026-06-21 | annotation-quality-gate-design | 发送治理/标注 | 设计已获批 | 簇 B 标注质量门：target_stages 归一校验（alias→canonical，否则素材永不命中静默不发）、审核审计（fail-soft 不回滚）、客户级辅助模式 override 三态入口；不引 RBAC |
| 22 | 2026-06-21 | ask-human-config-page-design | 请示/前端 | 设计已逐段获批 | ask-human P3 配置页：决策人链联系人选择器+可排序、4 escalate 开关、频控折叠区；后端补 operation_domain_json 序列化 askHumanPolicy；局部 useState 不建 store |
| 23 | 2026-06-21 | ask-human-inbox-frontend-design | 请示/前端 | 设计已逐段获批 | ask-human P2 统一收件箱：8 源 InboxItem 聚合、inline/rich 两类处置、4 个 rich 组件中立化到 components/review/（老页薄壳化、统一频道是 canonical 主场）；降级红线"坏源绝不连累好源" |
| 24 | 2026-06-21 | ask-human-unified-channel-design | 请示 | 设计已逐段获批（P1） | ask-human 统一通道总纲：推送型（请示卡→微信）与拉取型（admin 队列）两性质；AskHumanPolicy（决策人链/4 escalate 开关/骚扰频控/超时转备选）取代写死 principal_decider+high_risk_escalation_mode（旧字段保留兜底）；只读聚合器不建物化待办表；D 块执行层接线防死字段 |
| 25 | 2026-06-21 | referral-card-push-design | 引荐 | 设计已分节确认 | 专属顾问名片引荐：辅助模式（账号级开关默认关）在无人工接管红线上开受控例外；触发=人类标注+提示词注入+LLM 语义判断（非硬匹配）；已引荐态=prompt 层行为收敛非硬开关；被推真人与幕后 principal 解耦（D9）；validate_and_promote carry-through 三处接线警告 |
| 26 | 2026-06-21 | sales-media-asset-send-design | 内容资产/发送 | 设计已获用户分节确认 | 素材文件发送：改造 ContentAsset（file_path/sha256/sendable/send_trigger_hint/expression_pref）；双轨并行（话术轨+交付轨）；素材免 grounding（人类把关）但伴随 reply_text 照走五闸；媒体类型→MCP 工具映射表；文字+文件两条独立 outbox 条目 |
| 27 | 2026-06-22 | business-audit-fix-wave-design | 审计修复/请示/发送治理 | 设计（待审阅→plans） | 6 路 opus 业务审查 11 缺陷修复波：授权过期清 awaiting+中性收尾、链尾失联 AI 延期安抚（去重）、转述数字白名单护栏（fail-closed）、过期 verified 知识不背书（valid_to）、R5.4 拦截写 gap_signal、stage 写入过状态机（fail-soft）、唤醒 jitter、账号日发送量软上限告警、掉线不盲发、多模态入站打桩；铁律"不夺 LLM 语义判断" |
| 28 | 2026-06-22 | digital-twin-relationship-closure-design | 数字分身/前端 | 设计已获口头批准 | 关系类型闭环：口吻轴=customAgentInstructions（已存在，最高优先级覆盖 Soul，仅补引导文案）；触达轴=relationship_type→per_relationship_operation_mode；"大工程塌缩成几个精准小改"；不新建 per_relationship_soul（否定组合爆炸方向） |
| 29 | 2026-06-22 | media-asset-crud-completion-design | 内容资产 | 设计已获批 | 簇 C 素材 CRUD：edit（元数据/换文件分离）、toggle（sendable 正交于 review_status）、delete（引用计数保护物理文件）；换文件强制退 draft+清 media_id（AI 不自我核验红线） |
| 30 | 2026-06-22 | structured-organization-design | 内容资产/知识 | 设计已获批 | 簇 D 结构化组织：知识 chunk 注入 productTags/businessTopics、素材/名片 tags 激活+注入+检索；软增强不建硬关联表（agent-first）；8 缺口至此全覆盖 |
| 31 | 2026-06-22 | taxonomy-admin-crud-design | 分类学/前端 | 设计待审批 | 字典条目运营编辑前端：接已就绪的 CRUD 端点；软删叫"废弃/恢复"不叫删除；CRUD 原地操作与版本灰度正交；前端校验不超过后端（D5 踩坑教训） |
| 32 | 2026-06-23 | chat-scorer-relevance-migration-design | 知识 | ✅ 已实现（2026-06-24，853ca82/9903c23） | chat 检索 scorer 从整串 contains 迁移到 relevance_score（CJK bigram 分词）；保留字段加权 3/2/1+verified0.5 骨架；中文召回缺陷根治；lib 1498/0 |
| 33 | 2026-06-23 | progressive-prompt-three-tier-design | 决策/prompt | 待审 | 渐进式三档提示词（lean/relational/full）+ 充分性自评统一循环（最多两程）；安全槽位恒注入铁律（降档失误必须可恢复）；隐私双管（注入侧硬约束+reviewer 边界维度）；不复活 tool_loop |
| 34 | 2026-06-23 | reactivation-stage-universalization-design | 通用域适配/planner | 未标注（修复设计） | scan_reactivation 硬编码 dormant_reactivation 焊死→字典 is_reactivation_target 标记（与 is_terminal 完全对称、正交语义）；DEFAULT 单元素 $in 字节等价 |
| 35 | 2026-06-23 | tag-trust-two-layer-design | 记忆/标签 | brainstorming 产出 | 标签可信度三层改造：manual_tags（人工权威层 AI 物理够不着）/AI 暂定层（tally+证据引用）/AI 确信层（压缩时宽窗口整体重判 replace 根治只增不减）；证据存引用 fail-closed；customer_stage 强证据快通道；贝叶斯评估旁路（6 槽严谨占槽、纯观测永不驱动）+大五 OCEAN 压缩时人格分析 |
| 36 | 2026-06-23 | tool-loop-dead-code-sunset-design | 清理 | 未标注（兑现 sunset-plan D+21） | tool_loop.rs 802 行死代码下线（从未接生产、8 调用点全在 #[cfg(test)]）；knowledge_routing_mode/knowledge_max_tool_loops 参数删除；护栏：勿误删形近的 knowledge_max_tool_calls（活路径） |
| 37 | 2026-06-24 | account-send-pacing-guard-design | 发送治理 | 设计待复审 | 账号级最小发送间隔闸（1-4s 随机）防连珠炮风控特征；立场"不赌魔法数字，从'像不像机器'根源降特征"；reschedule 不 sleep 不耗 attempt；插入点必须在 reclaim 幂等门之后；需新增 (account_id,status,sent_at) 索引 |
| 38 | 2026-06-24 | progressive-tier-hardening-design | 决策/prompt | 待审 | 三档机制加固：强升闸（coverage=missing 且需知识→强制 Full）/观测（weak 灰区+关系档漏判+自评 JSON 失效）/PROGRESSIVE_TIER_ENABLED kill switch 三者正交；修 used_knowledge_ids 口径防架空 grounding 硬闸 |
| 39 | 2026-06-25 | taxonomy-label-wiring-design | 分类学/通用域适配 | 设计待复审 | 取值字典三流接线：A=prompt 注入合法取值（TaxonomyCache 补 display_name）；B=前端 labelFor 三态分流（ok/unknown_value/no_dict，诚实优先于好看）；C=AI 生成 profile 连初始取值集落候选层（label 通路 6 处贯通是 blocker）；typed 维度行业化走 override 整段替换不改销售散文 |
| 40 | 2026-06-26 | frontend-backend-alignment-fixes-design | 前端/审计 | 设计稿（全量路线图） | 76 条前后端对齐缺口（241 信号→76 确认，REFUTED=0）：两大反模式"只读不可写"+"错误态静默吞成空态"；按 5 业务域×4 批次组织（批次 1=14 条 P0/P1 直接走 plan）；A1 loadMessages 双死端点是唯一 CRITICAL |
| 41 | 2026-06-26 | frontend-backend-alignment-batch2-design | 前端/通用域适配 | 设计稿；批次 1 已合 main（PR#44） | 批次 2=9 条通用化断裂：统一走 labelFor 三态（诚实：绝不显示错误销售标签）；闭集枚举（finalReviewStatus 10 值/holdCategory 3 值）用常量 map 不走 labelFor；conversation_mode 补 taxonomy 字典 |
| 42 | 2026-06-26 | frontend-backend-alignment-batch3-design | 前端/审计 | 设计稿；批次 2 已合 main（PR#46） | 批次 3=19 条：按后端 blast radius 分三组（纯前端 16/轻后端 E9/重后端 E11 高危确认流+F23 疑似成交闭环）；F23 建 suspected_deal_signals 专表+approve 走 add_outcome_event_inner（AI 永不直写成交） |
| 43 | 2026-06-26 | full-business-logic-test-design | 测试 | brainstorming 产物 | server 117 真模型全量业务测试 13 域方案：webhook 真进站+发送到 outbox 为止不真发微信；biztest_* 隔离；防假绿总则（llm_call_logs status=success 铁证）；两批执行防 active profile 串扰；区分"红线预期 vs bug" |
| 44 | 2026-06-26 | full-business-logic-test-findings | 测试 | 问题清单（仅 1 条） | 首轮结果：域① import-preview 被上游 LLM 503 阻断（critical，llm_unavailable http_5xx retryCount=2）——测试基建就位但被 LLM 平台侧不可用挡住 |
| 45 | 2026-06-26 | main-health-audit-batch1-design | 安全/审计修复 | 未标注（batch1，PR#41 合入） | main 健康度 4 条修复：SEC-1 evolution 三端点 workspace scope（跨租户→404 不暴露存在性）+EVO-2 真实 actor；KNOW-1 知识预览透传 workspace；FE-1 guide preview 健康分改后端构建 items（前端 healthFromScores 三重错展示伪造健康分） |
| 46 | 2026-06-26 | main-health-audit-batch2-design | 并发/审计修复 | 未标注（batch2） | 剩余 6 条：CONC-3 load_or_create 捕获 E11000（防客户丢回复）、CONC-2 commitments $push+$slice 治并发丢失（深核推翻 pipeline 去重方案）、CONC-1 memory_card 拆出走 OCC（推翻整 update 套谓词）、GATE-1 revision 后复检动作闸、KNOW-2 告警口径补 status=active、EVO-3 threshold_auto_release_enabled 接线 |
| 47 | 2026-06-26 | management-agent-thickening-design | 管理域 | 设计待复审（PR#45 已合 main） | 指挥中心做厚：管理侧"提议→确认→执行"循环与客户侧 principal channel 同构；执行结果核实三层（outcome assertion/真实结果汇报/executed_unverified 诚实）；提示词编辑三层分级+三闸（禁词/锚完整/LLM 语义审 diff 增量，降级人确认）；verify 工具恒强制确认不随放权豁免 |
| 48 | 2026-06-26 | prompt-pack-startup-alignment-design | prompt | 首版已实现+终审追加 2bis | prompt pack 启动对齐"spec 为真相"：版本号降级为非生效闸，逐 key 内容比对（normalize CRLF）+seeded_by 白名单（system 可刷新/manual/evolution 保留）+evolution 灰度链 key 跳过；修 rollback 不恢复 status 缺陷；domain_config/playbook/souls 显式不纳入（admin 原地编辑不翻 seeded_by 不兼容） |
| 49 | 2026-06-27 | chunk-ai-repair-closure-design | 知识/前端 | 设计稿 | F22 复活：chunk AI 修复三端点（propose/answer/applied）后端 alive 但前端空转→补前端闭环；AI 只产 patch 不写库；逐字段勾选落库（防清空：从原值出发只覆盖勾选字段）；thenVerify 恒 false |
| 50 | 2026-06-27 | frontend-backend-alignment-batch4-design | 前端 | 设计稿；批次 3 已合 main（PR#47） | 批次 4=9 条收尾（几乎纯前端）：D10/D11 维度编辑补全、F11 confirmedBy 徽标、F14 移除误导性死控件（后端无条件忽略 workspaceId）、F16 SSE 指数退避重连、D9 有意缺口转做（domain-schemas 写表单+改文案承诺） |
| 51 | 2026-06-27 | prompt-evolution-human-gated-design | 演化 | 设计（brainstorming 产出） | prompt 自优化定位"AI 提议+真模型证据+人工把关发布"（终态设计非过渡）：阈值通道可全自动（纯函数重判），prompt 不能（LLM 非确定+改的是红线 prompt+让 LLM 改约束自己的规则）；G1-G4 缺口（shadow 未实装/release 绕三闸/snippet 语义错配/假基线）；三闸下沉 prompt_guard.rs；snippet 改末尾追加（原文逐字保留→锚点天然过） |
| 52 | 2026-06-28 | campaign-frontend-design | campaign/前端 | 设计评审稿 | 活动结果看板（仅消费 /sends 7 桶聚合）：后端无 GET /campaigns 列表端点→入口靠总控 AI dispatch 后跳转带 id；7 桶→StatusBadge 5 tone 映射；escalated 标"已请示"非"转人工" |
| 53 | 2026-06-28 | campaign-sends-report-design | campaign | 设计评审稿（PR#57 合 main） | 推送结果 7 桶查询端点：campaign_sends 死台账不回写，按需聚合 join agent_run_logs（3 次固定查询无 N+1）；7 桶分类纯函数覆盖 GATEWAY_STATUS_VALUES 全集；escalated 单列（请示在途非失败）；outbox_status=sent 最高优先级 |
| 54 | 2026-06-28 | campaign-targeted-push-design | campaign | 设计评审稿（合 main d615bdc） | 活动定向推送：G2/G3/G4 底座上补"按产品反查人群"segment 两阶段查询（Mongo $elemMatch 粗筛+G4 纯函数精筛净持有）；只认高可信成交红线天然继承；预览→人工确认→扇出（campaign_sends 唯一索引幂等去重）；确认门必须用 tool_always_requires_confirmation（Dangerous 档默认不触发确认——坑）；4 处核实修正记录 |
| 55 | 2026-06-28 | customer-reply-guarantee-design | 决策/红线 | 未标注（含实现期修正） | "客户必有回应"保障：held_by_ai_policy 终态客户零回复=晾死缺陷；根因=安抚客户与请示领导错误耦合（骚扰门连带跳过占位）；方案=gateway 末尾 per-run 回应守卫（黑名单语义+豁免清单）+escalation 解耦只推领导；实现期修正：A3 主动沉默（no_reply）不补占位（CI 证明补了破坏拟人） |
| 56 | 2026-06-28 | e4-document-repair-f21-task-list-design | 知识/前端 | 设计稿 | E4 重定义：pack 修复死桩（items 集合已物理删除）不复活→文档级批量修复=视角聚合复用单 chunk 闭环（零新后端端点）；F21 补 chat_task_list 端点+TaskRail 列表化 |
| 57 | 2026-06-28 | frontend-backend-contract-alignment-design | 前端/契约 | 设计已定（四轮证伪+POC 硬验） | 前后端契约对账机制：投影函数（非 endpoint/资源）是契约锚点；fixture JSON 双门（后端快照测试+前端 CANONICAL_KEYS 键集对账）；已证伪 codegen/ts-rs（投影非纯 serde）；防腐烂 lint 用运行时 glob 扫描（手维护清单必腐烂）；raw Document 三档容差 |
| 58 | 2026-06-28 | memory-conflict-and-reviewer-yield-design | 记忆/引荐 | 未标注（v2 续作） | ⑨记忆冲突治上游：保 id 注入+强制结构化产出（fact 原子化+dimension 改口必填）+跨轮命名稳定化+同 dimension 裁决兜底 engage；④reviewer 让位下沉：assist 模式引荐让位段注入 reviewer（消解红线措辞+factRisk 两条 hold 路径），不碰硬闸阈值；诚实记录 Task9 三处幻觉修正 |
| 59 | 2026-06-28 | memory-consolidation-guards-design | 记忆 | 未标注（合并 main #60） | ⑨确定性兜底两件套：件一 compact 救回加 dimension 感知（同 dimension 新值在场不救回旧值）；件二结构性非原子 blob 检测（换行/句界/80 字三判据 OR，零关键词）+降级重试 1 次+丢弃兜底；4 轮探针证明 blob 是低频降级非 prompt 缺陷（ANTHROPIC_JSON_GUARD 已压住 tool_use 劫持） |
| 60 | 2026-06-28 | prompt-evolution-phase3-evidence-display-design | 演化/前端 | 未标注（阶段一 PR#50/二 PR#51 已合） | prompt 候选新旧对照证据透出：shadow_replay_json 补 original 侧 2 字段；聚合 5 闸涨跌表+逐样本对照表；evalMetrics key 是 snake_case（bson 直转不改名——与其它手写 camelCase 字段不同的边界）；FIVE_GATE_KEYS=fact_risk_block/pressure_risk_block/human_like_score_rewrite/emotional_value_rewrite/product_accuracy_score_block |
| 61 | 2026-06-29 | campaign-domain-completion-design | campaign | 设计评审稿 | campaign 域收口：GET /campaigns 列表端点（CampaignListItem 投影防泄漏 workspace_id/segment_filter）+列表页+建活动表单（四维圈人动态选项、draft 复用）+看板 CSV/翻页；红线=前端绝不做 dispatch 按钮（真发送只走总控 AI 恒确认门） |
| 62 | 2026-06-29 | content-assets-tiered-injection-design | 内容资产/决策 | 未标注 | 文本资产分档注入：修复"绑死 Full 档、降档即失效"缺陷（渐进三档下核心禁语/品牌口吻在日常轮完全不生效）；min_inject_tier 闭集{lean,relational,full}，None=full 字节等价；同时清理过期录入项（素材 URL/MCP Media ID 手填框——media_id 是发送链命脉字段但绝不该运营手填） |
| 63 | 2026-06-29 | memory-summary-not-authoritative-fact-design | 记忆 | 未标注 | ⑨第三轮修复（前两轮未生效的根因误判纠正）：真因=memory_card_from_contact 把短期滚动上下文 memory_summary 当权威 core_fact 注入种子卡；修复=纯字段归位（删 1 行 push+改 1 处 insert 到 extra.recentEpisodeSummary）；诚实记录"件一件二全程没触发" |
| 64 | 2026-06-30 | content-assets-injection-hardening-design | 内容资产/决策 | 未标注 | 三 PR 后加固：A=禁语恒注入被共享 limit(16) 击穿（拆两次独立查询物理隔离）；B=三加载器写死 default_workspace_id 多租户即静默失效（签名加 workspace_id）；filter 抽纯函数+query-shape 单测 |
| 65 | 2026-06-30 | content-vs-knowledge-nav-disambiguation-design | 前端 | 未标注 | 纯导航文案对齐：内容资产（话术/素材）vs 知识库 Wiki（录入/审核/问答）概念混淆消除；"Wiki 管理"名实不符（subtitle 藏住了知识录入口） |
| 66 | 2026-06-30 | evolution-ui-toggle-design | 演化/前端 | 未标注 | 演化中心 UI 开关：mongo runtime flag 当真总开关、EVOLUTION_ENABLED 降级为运维硬上限（默认 false→true）；开=全量 rollout 100；部署注意：生产 .env 显式 false 会变硬锁定 |
| 67 | 2026-06-30 | forbidden-expression-injection-design | 内容资产/决策 | 未标注 | 禁用表达独立段注入：修复语义反转（禁语被罩在"可引用内容资产"标题下）；禁语恒注入无视 tier（D1）；诚实边界：纯 prompt 层修正、无代码硬拦截门（D5 明确不做） |
| 68 | 2026-06-30 | guide-apply-partial-validation-design | 引导层 | 设计已与用户逐节确认 | guide apply 两缺陷：A=prompt 不注入合法值（LLM 必然产越界值如 operationState="active"）；B=单字段越界 400 连坐全部合法字段；修复=LLM 产的值越界跳过+记录 skippedFields（人手填的仍硬拒——语义边界"LLM 不连坐、人是权威"）；绝不改 apply_admin_dim_validation 共用 helper |
| 69 | 2026-06-30 | h10-relay-identity-provenance-fix-design | 请示/安全 | 未标注（UPHELD High 修复） | H10：relay 身份从"内容前缀哨兵"（客户可伪造 __PRINCIPAL_RELAY__ 劫持转述模式+绕过全部频控+污染号码护栏授权源）改为"来源凭证" is_synthetic_relay（skip_serializing+skip_deserializing+default 三属性缺一不可）；LLM 层双补：963 当前 inbound 按标记剥哨兵+751 history 剥哨兵（多轮残口） |
| 70 | 2026-06-30 | h3-cross-tenant-idor-fix-design | 安全/租户 | 未标注 | H3 水平越权：3 文件 14 handler 接受 body/params 自带 workspaceId 无 ACL 校验（最高危 activate_provider 进程级热切换/test_provider 用他租户 api_key 出站）；修复=纯函数 is_workspace_authorized + resolve_authorized_workspace 每请求校验（含回落值）；否决 ACL 进 JWT claims（与既有刻意设计冲突） |
| 71 | 2026-06-30 | wiki-audit-high-fixes-design | 知识/安全 | 未标注 | wiki 审查两 High：#1 domain_schemas serde 错配（filter 用 camelCase 查 snake_case 库→列表恒空/activate 写幽灵字段/动态字段校验静默失效——纯函数测试全过掩盖 IO 层缺陷）；#2 prompt create/publish 绕过红线三闸（update 有闸但 create/publish 零校验）；诚实声明 handler pub(super) 测试盲区 |
| 72 | 2026-06-30 | 上线前全量业务测试方法论-design | 测试（方法论总纲） | brainstorming 产出 | 47 域饱和式审计（96 agent/1077 万 token）：正确性四层（L1 红线否定式=go/no-go 硬门/L2 设计意图/L3 主观质量/L4 孤儿行为不可判定逐条交用户）；测试可信度：可信 2/有缺口 25/假绿 12/缺失 8；假绿四手法（平行实现自证/循环论证/状态码即成功/空壳 #[ignore]）；300 真缺口+190 孤儿 |
| 73 | 2026-07-01 | chunk-request-deadfield-cleanup-design | 知识/清理 | 未标注（两轮 subagent 审查交叉验证） | 2026-05-25"未完成的删除"正规层收尾：请求体六死字段（routing_card/safe_claims 等）经正规路径 100% 空转→删除；apply_chunk_integrity 简化为两态（rejected 分支恒不可达死代码）；明确不碰 prompt/chat 裸 $set 旁路（语义债留专题） |
| 74 | 2026-07-01 | h1-ingest-worker-not-due-fix-design | worker | 未标注（UPHELD High） | H1：not-due 早退与真 304 共用 NotModified 变体→mark_success 误刷 last_fetched_at→节流基准被每 tick 前推→schedule_minutes>worker_interval 的源首拉后永不更新；修复=拆 Skipped 变体（类型层分离两语义）；现有测试 seed 全用 last=None 从未走 not-due 分支 |
| 75 | 2026-07-01 | h7-m016-collection-names-fix-design | 租户/迁移 | 未标注 | H7：m016 回填表 7 集合名拼错+15 漏收录+2 该移除（chunk_revisions/admin_users 无单值 workspace_id）；方法论记录：两轮 Explore subagent 核查均不可靠互相矛盾→主控机械穷举亲验定稿（"清单完整性要用可枚举机械判据交叉锁死"教训） |
| 76 | 2026-07-01 | h8-boot-brick-stale-index-fix-design | 数据库 | 未标注 | H8 启动砖：ensure_indexes 残留旧 2/3-key unique（建完即被同函数 drop 的死代码），admin publish 攒多版本行后重启 E11000 崩溃；反讽铁证=团队在 admin_ops_versions 注释里已知此陷阱却漏了既存两处；测试从未触发因全用空库 |
| 77 | 2026-07-01 | user-ops-cockpit-redesign-design | 前端 | 未标注（Spec A） | 用户运营驾驶舱重设计：主轴=观测/配置分离（23 字段编辑表单混进查看型 tab 是过长根因）；三段式=常驻判断条（6 chips：人格态/最近轮/下一步/风险灯/作息灯/请示灯）+段控+下钻；后端唯一小改=operation-health 补 quiet_hours 3 字段 |
| 78 | 2026-07-01 | wiki-three-p0-fixes-design | 知识/红线 | 未标注 | wiki 三 P0：①auto-verify 对所有 chunk_type 都不自动 verified（降级 needs_human_audit，auto-verify 退化为预审分诊器）+冷启动 peer_case 补 integrity=verified 过滤；②wiki_type/chunk_type 导入透传（初判"富字段全丢弃"半误报——那是 05-25 有意删除不该加回）；③派工 fix_chunk 真产可审草稿（execute_step 占位桩定性反转：不自动改库是守红线的正确设计，缺的是产草稿）；禁词陷阱：单字"人工"也在禁词集，代码文案一律用「运营」 |
| 79 | 2026-07-02 | decision-review-autonomy-protocol-design | 决策/前端 | 未标注 | 决策复盘补 AI 自治协议 9 字段（whyShouldReply/selfCritique 等"AI 内心独白"）：字段已在 agent_run_logs.decision，纯读投影零迁移；管理发送路径 decision 直接构造 9 字段恒空→优雅降级是硬功能要求非兼容 |
| 80 | 2026-07-02 | h11-evolution-threshold-keys-fix-design | 演化 | 未标注（H11+M9+L1 三合一） | H11：evaluate_single_gate 读 factRisk/productAccuracy 旧键而生产序列化键是 hallucinationScore/knowledgeGroundingScore（alias 仅反序列化生效）→score 恒 0.0→fact_risk 恒不命中/product 恒命中；铁证=同文件 read_gate_score 已双键兼容但 threshold 路径没复用；现有测试用生产从不产生的旧键 seed=假绿；M9 空基线偏拒；L1 死 match 臂 |
| 81 | 2026-07-02 | m1-management-send-finalize-fix-design | 管理域/安全 | 未标注 | M1：管理发送路径只调 review_passed（软闸折叠 bool）缺 finalize 四硬门（R5.4 verified 背书/协议/预算/should_hold）→同一危险内容走客户回复被拦、走管理发送被放行；修复=接 finalize+必须带 && review_passed guard（finalize 对软闸失败标 Approved 指望 revision 循环而管理路径没有） |
| 82 | 2026-07-02 | m10-streaming-json-repair-design | LLM | 未标注 | M10：流式 JSON 解析缺第四层 LLM-repair 兜底（非流式已有）；注释"由调用方降级"假设从未实装；关键洞察=修复是对已累积文本发独立请求不 re-stream，注释顾虑不适用 |
| 83 | 2026-07-02 | m12-reset-pack-evolution-critic-design | prompt/演化 | 未标注 | M12：reset-system-pack 全量 delete 但只重种业务 pack→evolution_critic_v1 被删不补→演化循环持续报错到进程重启；根因=delete 覆盖面>reseed 覆盖面 |
| 84 | 2026-07-02 | m13-profile-attributes-preserve-fix-design | 画像 | 未标注（原报告标 High） | M13：前端保存运营画像不带 profileAttributes→serde default 空 Document→无条件 $set 清空 AI 积累的画像属性；修复=镜像 gateway 非空守卫（"没发"不当"要清空"）；否决前端回传方案（只读值当可写值往返有时序覆写风险） |
| 85 | 2026-07-02 | m16-mcp-logs-base64-redact-design | 可观测 | 未标注 | M16：media_upload_base64 整份文件 base64 落 mcp_logs（可达 67MB 超 16MB BSON 上限→insert 静默失败审计落空）；修复=按 key 精确脱敏 base64（否决通用长度截断——会截断超长 content 破坏崩溃恢复精确匹配→重复发送） |
| 86 | 2026-07-02 | m4-contacts-stage-dict-validation-design | 画像/分类学 | 未标注 | M4：contacts 三建档端点 LLM 生成 stage 未经字典校验直接落库（management 同路径已校验——安全门不对称与 M1 同源）；修复=镜像 MachineWrite 校验（越界 drop 不阻断建档，AdminWrite 会把"启用 Agent"按钮打成 400） |
| 87 | 2026-07-03 | boundary-privacy-in-review-passed-design | 守卫 | 未标注 | boundary_privacy_safety 软闸（2026-06-23 引入的隐私/边界量化闸）加入 classify_dual_gate 时漏加 review_passed→revision 没修好或管理路径直接放行泄露内容；修复=补对偶判定（0=老数据豁免镜像 pressure_risk） |
| 88 | 2026-07-03 | seg-empty-source-idempotency-key-design | 发送治理 | 未标注 | 多段回复空 source_event_id 时 "#seg0" 非空→走非 synthetic 幂等键→丢 run_id 隔离→跨 run 雷同分段被静默去重（客户少收一段）；修复=空 source 回落 run_id 作 base；非空 source 绝不能掺 run_id（会破重放去重） |
| 89 | 2026-07-04 | followup-empty-reply-stuck-design | 决策/worker | 未标注 | LLM 输出 should_reply=true+空 reply_text 的退化决策→既不入 outbox 也不 cancel→task 卡 running 反复 reclaim 到 failed；修复=text_send_eligible 纯谓词两处终态判定共用 |
| 90 | 2026-07-04 | inbox-ui-jargon-cleanup-design | 前端/请示 | 设计已获用户批准 | 统一收件箱 UI 重做+前后端全量清黑话：黑话两类（A=结构化枚举加字典即可翻译——A1 seed 已有中文 label 只是 active-view 没下发；B=黑话嵌在拼接串里必须改后端如 escalation 请示串直接拼 blocked_unverified_product_claim）；卡片类名 CSS 0 定义（Task 12 从未落地） |
| 91 | 2026-07-04 | login-timing-side-channel-design | 安全/auth | 未标注 | 登录用户名枚举时序侧信道：不存在的用户名秒回 vs 存在的跑 Argon2 ~30-50ms；修复=假 PHC 哈希 dummy verify 抹平时序；关键不变量=假哈希必须合法 PHC 否则解析失败走快路径重新制造时序差 |
| 92 | 2026-07-06 | escalated-run-budget-design | 决策/预算 | 设计待审 | B-1：progressive-tier 升档 run 两程叠加撑爆 30000 token 预算→主回复从不发送（blocked_by_budget）；关键数据=单程 Full+路由已超 30000（否决"丢弃 Lean 计数"方案——仍超且谎报成本）；修复=分档 gating 上限（升档 run 100000），计数保持诚实 |
| 93 | 2026-07-06 | knowledge-workbench-dispatch-wiring-design | 知识/worker | 未标注 | 派工长任务结构性空转（6 action 里 5 个纯文案桩+唯一真干活的 fix_chunk 永远拿不到 targetChunkId+手打文本框产的 step 无 cardId）；修复=两条结构化入口（卡片驱动+对话驱动）汇入同一执行链，chat_task_create 落库时解析 targetChunkId；6 action 分层实现（真产草稿/只读报告/状态标记）；闭集陷阱=suggestedAction 含 freeform 但 ALLOWED_TASK_ACTIONS 不含 |
| 94 | 2026-07-07 | taxonomy-candidate-inbox-card-design | 分类学/前端 | 设计待批 | 收件箱标签候选卡三层缺陷：裸维度键展示/富字段被硬编码 None 丢弃/"通过"按钮必然 400（approve 需 canonicalValue 而 inline 发空 body）；改为 rich 命名卡复用 system-strategy 已验证表单 |
| 95 | 2026-07-07 | user-ops-roster-batch-enroll-design | 通讯录 | 设计待审 | 全量通讯录+批量托管+头像：核心张力=首次托管需人类录入运营意图 vs 批量无法逐人录入→sharedNote 一次写整批用+异步 initial_profile AgentTask（不等 50×20s LLM）；MCP 工具亲验证伪指南页（contact_list 不存在，唯一正确工具 contacts_fetch_cache） |
| 96 | 2026-07-08 | roster-fetch-cache-shape-design | 通讯录 | 设计已获批 | 通讯录恒空根因：contacts_fetch_cache 真实返回 {result:{friends:[纯 wxid 字符串]}}而解析器候选路径无 /result/friends 且要求元素为 object——"解析器从未跑通过真实数据"；四模块修复（解析器兼容字符串数组/空 cache 短重试+syncing 标志/懒加载 contact_get_detail+持久缓存/429 兜底） |
| 97 | 2026-07-08 | system-strategy-panel-pagination-design | 前端 | 未标注 | playwright 实测候选审核面板 104725px≈116 屏（176 条 pending 全量平铺）；三面板客户端分页复用 CampaignBoard 范式；safePage 渲染期夹取自愈越界 |
| 98 | 2026-07-08 | system-strategy-tab-layout-design | 前端 | 未标注 | 系统策略 7 大面板平铺→4 职能 tab（总控/标签与状态/行业配置/经验教训）；不借 knowledge 的全局 wikiModeBar 类（跨频道复用违反 CSS Module 边界）；5 个测试文件需加 selectTab 前置 |
| 99 | 2026-07-08 | taxonomy-candidate-display-name-design | 分类学 | 设计待批 | 决策路径标签候选补 AI 中文建议名：LLM 自造新值时顺带产 dimensionDisplayNames（agent-first：LLM 有上下文知道 anxious 该叫焦虑还是担忧）；carry-through 是头号坑（漏了字段被静默丢弃）；决策+decision_taxonomy 两路径同改（幂等键先写者赢）；亲验纠正：不 bump PROMPT_PACK_VERSION（启动对齐按内容 diff 生效） |
| 100 | 2026-07-09 | roster-fetch-full-design | 通讯录 | 设计（待写计划） | 切 contacts_fetch_full（117 亲验返回 4831 条富化字段）；陷阱=refreshing:true 却带全量数据（就绪判据必须用 status=="ready"/items 非空）；sex 透传+分页（4831 卡片无分页立刻暴露） |
| 101 | 2026-07-09 | roster-mcp-ratelimit-syncing-design | 通讯录/MCP | 设计已获批 | MCP 429/503 弹红条→柔化：新增 AppError::UpstreamBusy 变体（post_rpc 状态码分类），roster 捕获转 syncing 态自动重拉；真实错误（401/500）照常红条不掩盖 |
| 102 | 2026-07-09 | webhook-gewe-addmsg-parse-fix-design | webhook | 设计已获批 | 真实 GeWe AddMsg 嵌套 payload（Data.FromUserName.{string}/{low} 包裹）被通用 find_string 遮蔽：顶层 Wxid（账号自己）抢先命中→发件人归错、PushContent 抢先→内容取"吴界 : 你好"脏串→真实客户消息永不触发 AI 回复；biz-test 62 PASS 全绿因从未用真实嵌套形态（形态盲区非假绿手法）；修复=GeWe 显式路径优先+回落兼容扁平 |
| 103 | 2026-07-09 | webhook-signature-verify-restore-design | webhook/安全 | 设计已获用户逐节批准 | 联调期 WEBHOOK_VERIFY_SIGNATURE=false 是公网无鉴权入口必须回退；但直接改回 true 不够——两端签名方案已不匹配（旧：x-mcp-signature 裸 hex 全局 MCP_API_KEY 仅 body；新：x-webhook-signature sha256=hex 每账号密钥 timestamp.body）；方案 B=每账号 webhook_secret+时间戳防重放+fail-closed（漏配密钥=400 拒绝非放行）；部署顺序关键（先部署再翻开关否则消息流中断） |
| 104 | 2026-07-10 | full-system-deep-test-design | 测试 | 设计已获用户逐节确认 | 全量系统深度测试：playwright 驱动 117 真实 Chrome 逐页走查（19 频道×L1/L2/L3/UX/P 五维）；发送红线三道护栏（除吴界/AI应用开发外全改 normal）；只发现+记录不修复；红线预期 vs 真 bug 区分纪律 |
| 105 | 2026-07-10 | outbox-chat-search-idempotency-design | 发送治理 | 设计（待写计划） | 重复发送实锤（吴界收到 3 条相同占位）：150s timeout 取消 send future→mcp_call_logs 没写成→本地兜底查空→误判没发过→重发；修复=timeout 兜底核对源升级为 MCP chat_search（server 侧真实已发记录，0.02s 同步落库）+content 精确等于判命中+失败回落本地日志；残留风险偏"重发"而非"漏发"（重发可挽回） |
| 106 | 2026-07-10 | passive-reply-daily-limit-and-ai-holding-reply-design | 决策/请示 | 设计（待评审） | 两个"有意设计但语义不对"缺陷：①daily_limit 无差别拦被动回复（客户主动问也被拦→反复收同一句占位）→收窄为仅 FollowUp；②过渡/占位回复是硬编码死文案破坏拟人→改 AI 生成（三支柱：独立小预算旁路+运行期出站禁词守卫 passes_forbidden_words+硬编码降级兜底）；部分推翻 2026-06-28 customer-reply-guarantee 的"占位一律硬编码"结论 |
| 107 | 2026-07-10 | roster-backend-snapshot-persist-design | 通讯录 | 设计（待写计划，即 PR#162） | 后端持久化快照 roster_snapshots（进频道秒回+24h 过期后台自刷+任何 MCP 失败回旧快照）；亲验事实修正：解码失败是 502 红条非无限重拉；前端 force 参数此前从未拼进 URL |
| 108 | 2026-07-10 | roster-sex-parse-nonhuman-design | 通讯录 | 设计（待写计划） | 性别全显未知根因=sex 真实形态是 int64 序列化对象 {high,low,unsigned} 而 as_i64 取不到→取 .low；非真人账号白名单标记折叠（公众号无法可靠识别不硬猜——福州晚报与真人字段完全同构）；前端 roster 缓存 store 化 |
| 109 | 2026-07-10 | roster-single-flight-refresh-design | 通讯录 | 设计（待写计划） | 根治通讯录卡死死循环：8s force 轮询叠加无去重 spawn→打爆 SSE 20 并发上限→1.37MB 大 body 读 TimedOut→互相中断→快照永远写不进→永远 syncing；修正 PR#162 被低估的假设（"重复 spawn 无害"只看到写快照漏了并发抢名额）；三支柱=DashMap single-flight 锁（RAII guard panic 也释放）+前端只读轮询+同步端点不阻塞 |
| 110 | 2026-07-10 | user-ops-pool-real-contacts-redesign-design | 通讯录/画像 | 设计讨论中（B 档拍板） | 运营池"真人漏斗"重设计：昵称全是"Demi"根因=find_string 深度递归命中 _mcp.nickName（账号 owner 昵称）；gh_/@chatroom 黑名单过滤+roster 富化+m029 一次性清洗（明确不带 APP_ENV 守卫）；"在 roster"="真人"（4832 好友 gh_=0 群=0 亲验） |
| 111 | 2026-07-10 | user-ops-pool-redesign-design | 前端 | 设计已获批 | 运营池计数与文案修正："全部 63"非数据错乱而是文案误导（63=来过消息+主动导入并集）；真缺陷=limit 100 截断+前端数组 filter 计数；改后端 count_documents+文案"已互动/待启用"；实现期 grep 更正初判死方法（contactStore 两方法实为活代码——教训：只在单 feature 范围 grep 会误判） |
| 112 | 2026-07-11 | deep-logic-audit-design | 测试/审计（方法论） | 未标注（用户四决策确认） | 深度逻辑审查设计：上轮 19 频道×5 维广度走查扫不到"没有页面的后端逻辑"→从频道入口穿透到底的全链路审查；5 批（A 自动回复命脉/B 知识/C 成交活动/D 请示配置/E 其余）；五步闭环（测绘→逐环审→subagent 并行→117 真跑→入台账）；subagent 结论必主控亲验才入账 |
| 113 | 2026-07-11 | deep-logic-audit-findings | 测试/审计（台账） | 唯一权威台账（含修复状态更新） | 全五批收官：53 findings=0 Critical/1 High/24 Med/28 Low；唯一 High=KD-04（decider_chain 推荐配置下领导微信回复永不被识别为裁决——lookup_principal_config 只查旧标量字段）；跨批元家族="设计声称的不变量，实现层有旁路/缺口/非原子窗口/新旧不对称"（五层：错误处理/数据写入审计/多步非事务写/新旧字段迁移/保护策略不对称）；五批红线核心防线均亲验成立无一突破；多条已标 Fixed（#180/#193/#194 等） |
| 114 | 2026-07-11 | friend-picker-modal-design | 前端 | 设计讨论中 | 好友选择器弹窗：手填 wxid 痛点→FriendPickerModal 共享单选组件（统一 UI 数据源可配：referral 传 roster/products-deals 传 contacts）；决策链多选不动（为统一而统一引入风险不值得）；手动输入 wxid 折叠兜底 |
| 115 | 2026-07-11 | taxonomy-candidates-batch-filter-design | 分类学/前端 | 未标注（F-007 修复） | 候选批量驳回+按 kind 筛选：只做批量驳回不做批量采纳（采纳需逐条人工填 canonicalValue——字典质量红线）；前端循环调单条 reject 零后端改动 |
| 116 | 2026-07-11 | user-ops-pool-display-fixes-design | 前端/通讯录 | 设计讨论中 | 运营池三显示问题：tab 挤成畸形椭圆（CSS 同行挤压）、预览吐 XML（appmsg/sysmsg content 本身是 XML→按 msg_type 标签化 [链接]/[系统消息]）、系统号混入（is_operatable_person 扩展复用 WECHat_SYSTEM_ACCOUNTS）；媒体号无可靠信号区分→hidden_from_pool 手动移除（不删记录） |
| 117 | 2026-07-12 | async-import-job-progress-design | 知识/worker | 已获用户口头批准 | 长文档导入 9 分钟同步死等→异步 import_jobs 集合+专用 worker+前端轮询；小文档保留同步路径；断线恢复=claimed_at 心跳+孤儿重认领；明确不提速（用户没选那条路） |
| 118 | 2026-07-12 | audit-events-failsoft-alignment-design | 决策/修复 | 未标注（批 A 家族①修复） | B-02+C-01+H-01 统一修复：apply_agent_updates 内 5 处纯审计事件 .await? 改 let _（同函数孪生 let _ 铁证对齐）；webhooks (e) 网关从 (d) reaction 的 else 解耦无条件执行；测试用 MongoDB collection validator 精确注入故障 |
| 119 | 2026-07-12 | kb-family1-edit-audit-unification-design | 知识/修复 | 未标注（KB-09/10/11 修复） | 知识编辑统一：apply_update_chunk 接回 apply_chunk_revision（获审计+union+锁字段）；admin PUT 保留 replace_one+补 revision 行；locked_fields 后端强制（关键修正：existing.locked 只进 enforce_locked_fields 静默覆盖、不进 apply_field_patch 硬拒集——避免锁定字段连坐毙掉整条合法编辑） |
| 120 | 2026-07-12 | kb01-lean-tier-clear-used-knowledge-ids-design | 知识/守卫修复 | 未标注（KB-01 修复） | 非 Full 档清空 used_knowledge_ids：LLM 自报 verified ObjectId 可架空 grounding 硬闸→resolve_used_knowledge_ids 纯函数无条件赋值（Full 档路由 id/非 Full 空 Vec）；清空责任归 gateway 口径点不改通用 carry_through |
| 121 | 2026-07-12 | kb08-audit-inbox-blackhole-design | 知识/修复 | 未标注（KB-08 修复，"最有业务价值 finding"） | needs_human_audit 人审黑洞：分诊动作反使待审切片从收件箱消失只剩计数；修复=收件箱查询 $in 两态+列表计数共用同一纯函数（杜绝 count/list drift 病根）+前端"AI预审通过·待复核"琥珀徽章 |
| 122 | 2026-07-12 | kc-family1-dispatch-atomicity-design | campaign/修复 | 未标注（KC-01/02/03 修复） | dispatch 三步非原子→补偿回滚（失败即删已建 state 保持 all-or-nothing）+status 前置门（dispatching 可重入恢复/completed 拒绝重推）；否决"反向关联+幂等自愈"（需改 15 处构造点而触发前提是 Err-return 非崩溃）；不调换 send/task 写序（send 是去重闸必须先占位） |
| 123 | 2026-07-12 | kc-family2-segment-coverage-design | campaign/修复 | 未标注（KC-05 修复） | KC-05：粗筛口径分裂——serde 默认（verification=staff_confirmed/eventKind=deal）只在反序列化补、Mongo $elemMatch 查询不补→缺字段老成交客户被粗筛静默漏掉，"粗筛⊇精筛"被反转；两防线=查询侧 $or/$exists+$ne:"reversal" 对齐（治标）+ m030 $mergeObjects 回填（治本，明确不加 APP_ENV 守卫——语义保持型回填同 m018 类，误加会在 117 生产静默 SKIP 使防线名存实亡） |
| 124 | 2026-07-12 | kd-family-relay-number-guard-design | 请示/守卫 | P1（用户裁定删除） | KD-01+KD-03 同源根治：relay 字符级数字护栏（extract_number_tokens 仅 ascii digit）是威胁模型错误的 backstop——"转述是否忠于授权"是语义问题（中文"八折"提取为空漏真幻觉/"24小时"误杀正确转述→裁决黑洞）；删 gateway fail-closed 用法、忠实度交还 prompt+独立 Review（reviewer 经 is_synthetic_relay 保留通道同时看到 substance+拟发转述）；载荷泄漏守卫保留（固定标记存在性检测威胁模型正确）；holding_reply 对同函数的调用不动（命中回落兜底文案非 fail-closed）；设计从 3-4 支柱收敛为单一删除动作 |
| 125 | 2026-07-12 | kd-family2-escalation-channel-design | 请示 | 未标注（KD-02 裁决不修/KD-05/KD-06 修） | KD-02 经用户裁决不加"领导泄漏"字符词表（与删数字护栏 #185 一脉：客户自称"李老板"误杀/"上面点头了"漏——语义交 LLM+review）；KD-05 骚扰门口径漂移=reassign 不刷 created_at→加 last_pushed_at_ms 字段+m031 回填（不加 APP_ENV 守卫）；KD-06 孤儿 pending=next_decider_on_timeout 的 position 未命中（admin 改链后 principal 已不在链）与真链尾同为 None 无法区分→未命中回落链首重新入链（真链尾→None 语义锁死不变） |
| 126 | 2026-07-12 | kd04-principal-reply-decider-chain-fix-design | 请示 | 未标注（批 D 唯一 High 修复） | KD-04：领导微信回复识别只查旧标量 principal_decider、不看 ask_human_policy.decider_chain，而唯一策略写路径从不写 principal_decider→推荐配置下领导裁决消息确定性掉进普通客户链路（领导若是 managed contact 甚至被 AI 当客户回复）；方案 A=抽纯谓词 is_decider_for_config 复用权威解析器 resolve_ask_human_policy（自动兼容旧字段回落）+lookup_principal_config 改 find 遍历；否决"同步写 principal_decider"（加深新旧字段漂移——正是元家族根因） |
| 127 | 2026-07-12 | ke-family1-auto-release-direction-consistency-design | 演化 | 未标注（KE-01/KE-02 修复） | KE-01：decide_auto_release 缺方向一致性校验——doc 声称"方向与候选一致才放行"但实现只判 band 外任意一侧→升阈候选在命中率已翻转过低时仍被放量（朝错误方向）；修复加 current/proposed 两参方向门（新逻辑=旧逻辑收窄子集只减误放行）；KE-02：threshold 重判 original 侧用真实终态、new 侧用 5 闸重推→非-5gate 因素（blocked_by_budget）凭空制造 send_delta 提升→改 original 也用 final_status_from_5gate 对齐（prompt 路径本已对称） |
| 128 | 2026-07-12 | long-doc-chunked-import-design | 知识 | 已获用户口头批准 | 长文档导入真实修复：29KB 文档 import-preview HTTP 200 但 chunks=0——三方确认根因是**输出生成瓶颈**非 prompt 体积（28k prefill 并行快、6-7k completion 自回归串行 25-30 tok/s 到一半截断）；方案=后端自动分块（标题优先+字符回退纯函数切分→每段并行度 2 抽取→确定性合并不额外调 LLM），前端契约一字不变；否决"整篇 LLM 预加工再切"（输出更长更慢把刚修的瓶颈请回来）；D2 锚定仍对完整原文跑 |
| 129 | 2026-07-13 | ingest-source-serde-camelcase-fix-design | 知识/前端契约 | 未标注（wiki Playwright 验证唯一真 bug） | 外部源列表 serde 大小写不匹配：存储结构体 IngestSource 无 rename_all、list handler 直接 to_value 输出 snake_case，前端声明 camelCase→删除/重激活按钮打 /undefined 永久无效、间隔列 undefinedm；不能给结构体加 rename_all（worker 查询/两条索引/存量文档全 snake 会一起崩）→ API 边界抽 ingest_source_json 逐字段映射；全仓扫查结论：IngestSource 列表是唯一真漏点（DomainProfile 前后端是自洽 snake 契约非 bug） |
| 130 | 2026-07-13 | p3-family1-dead-code-cleanup-design | 修复/死代码 | 未标注（H-02/F-02/KB-05 修复） | H-02：run_envelope R0 三函数（started/terminal/panic_hook）是"已设计已写集成测但未接线"的安全基建，doc 却写成将来时误导读者→只改 doc 标注未接线（不删函数，用户裁定）；F-02：enqueue 与 dispatcher 的 max_attempts 兜底默认 3 vs 5 分歧（死分支）→对齐 3；KB-05：propose_pack_repair 死桩恒 400 但路由仍注册→删桩摘路由（chunk 级活 handler 严格不碰） |
| 131 | 2026-07-13 | p3-family2-serde-key-tolerance-design | 决策/知识 | 未标注（D-01/KB-03 修复） | D-01：RawAgentDecision 顶层 customerStage/intentLevel 只认 camelCase、LLM 输出 snake 静默 miss→加 serde alias 双形容错（亲验顶层 alias 不下探嵌套 Document，不会误吸 dimensionDisplayNames 内同名中文显示名）；KB-03：is_verified 用 eq_ignore_ascii_case 而全部召回侧精确匹配→收窄为精确 == "verified"（向数据实况收敛而非放宽召回侧——后者要改 5+ 处 Mongo 查询为不可能的数据留后门） |
| 132 | 2026-07-13 | p3-family3-referral-hardening-design | 引荐 | 未标注（KE-03/KE-05 修复） | KE-03：候选加载按账号过滤但两处发送准入只按 workspace→他账号名片理论可经幻觉 ObjectId 推出；最优落点非台账初判的 DB filter 而是纯函数 validate_card_sendable 加 account 参（enabled/approved 同层、一处修改三处生效、单一事实源）；KE-05：toggle/delete 改变 AI 可引荐范围却无审计→补 fail-soft 审计（保留硬删，软删需改全链 5+ 读查询超 Low 范围 YAGNI） |
| 133 | 2026-07-13 | p3-family4-campaign-scale-design | campaign | 未标注（KC-04/06/07 修复） | KC-04/07：圈人粗筛 cursor 无 limit 全量驻内存+dispatch 单 HTTP 内串行千次 DB 写；关键亲验=发送本就异步（worker 消费 pending task），真实痛点是请求超时非发送洪峰→粗筛层 limit(max+1) 探测法+超限 400 拒绝（否决 insert_many——推翻 #183 刚过终审的每-contact 补偿回滚）；KC-06：targetCount 三义（preview 命中/dispatched 去重后/report 台账行）→加 lastDispatchTargetCount 消歧不重命名既有字段 |
| 134 | 2026-07-13 | reply-prompt-slimming-design | 决策/性能 | 分批灰度设计 | 回复慢根因：user.reply.task 单次 LLM 45-149s（prompt 39-43k tokens），近 30 天 inbound run 100% 超 30000 预算（avg 67003）→review 跳过/blocked_by_budget/恶性重算；止血已另行实施（runTokenBudget 30000→300000/escalated 600000/maxLlmCalls 6→10 DB 热更）；本 spec=瘦身三批次：批1 零风险（删 4 死字段/去 context_pack 重复注入/删调试元数据/删运行参数——原第 5 条"跨层去重"写 plan 时逐行核对 5 处全不成立被剔除）；批2 中风险 A/B（task 内双写去重+观测字段注释压缩，6 条红线段一字不动）；批3 高风险逐项单独 A/B（状态机只注入当前态+邻接/playbook 按相关性截断/Soul 样例压缩规则保留/history 单条截断）；复用 prompt_templates 多 active hash 分桶 A/B 设施 |
| 135 | 2026-07-13 | revision-fallback-to-approved-draft-design | 决策/守卫 | 设计待审 | 生产实证：managed 客户连续追问永远收到同一句兜底占位——改写三失败分支（LLM 错/30s 超时/二轮 review 未过）全 should_reply=false+Held→真回复被丢、兜底占位顶上；7 月 revision_failed 9 条中 8 条首评本已 approved、7 条是超时；设计基石=能进改写通道的原稿 finalize 必已 Approved（硬闸失败根本进不了改写、改写只由软闸/style_diverged 触发）→三分支统一回退发送已 Approved 原稿（final_review_status=revision_applied_approved，revision_reason 保留失败原因审计） |
| 136 | 2026-07-13 | wiki-channel-full-playwright-verification-design | 测试 | 未标注 | wiki 频道 3 模式×21 视图全功能 Playwright 真实验证：Tier1 纯只读 10 视图全点/Tier2 写操作 7 视图带 [E2E验证] 前缀数据/Tier3 危险操作 6 类只点到确认弹窗绝不确认+1 条造数据全链（新建→verify D2 闸放行→active 进池→supersede 清理回 95 条纯净态）；前置亲验 6 条安全边界（chat 物理隔离永不发客户/auto-verify 强制降级 needs_human_audit 绝不真进池/单条 verify 才是进池动作且有 D2 硬闸） |
| 137 | 2026-07-14 | agent-capabilities-audit-design | 测试/审计（方法论） | 只审不修 | 第一批新范围审查：agent 旁挂能力子系统（上轮 53 findings 只覆盖主链路 8 环节）；4 簇并行（A 记忆固化 3368 行/B 标签体系 1766/C 通用化底座 3466/D 节流准入 1984）≈10.6k 行；4 subagent 并行+主控逐条亲验驳回夸大；严重度校准 High=推荐配置确定性红线破坏 |
| 138 | 2026-07-14 | agent-capabilities-audit-findings | 测试/审计（台账） | 台账（20 findings） | 0 High/5 Medium/15 Low；核心红线全 HOLDS（bayesian/personality 只写不进决策、影子发送侧零副作用、entitlements fail-closed、8 条标签铁律）；元家族=「证据门/合并保护的层间不对称」——A-01 core_facts 缺证据门（tags/personality 有）与 A-02 confirmed_tags 截断窗口整体 replace 丢标签（core_facts 有保留合并）互为镜像；C-01 stagnation 读侧全动态写侧写死 customer_stage、C-02 初始画像半接线残留销售 schema（一次性 seed 首条入站自愈）；D-08 claim_analysis 缺失 fail-open 主控降级为"2026-05-25 显式接受取舍"WontFix 不重开 |
| 139 | 2026-07-14 | auth-routes-security-audit-design | 安全/审计（方法论） | 只审不修 | 第二批：auth(681 行)+routes(28017 行/46 文件) 安全隔离面；核心命题非认证（middleware 已亲验干净）而是授权隔离——workspace 锁靠每个 handler 自觉；6 簇（S 根因层先审出基准喂簇 1-5）+IDOR 五点检查清单；关键校准=单租户默认部署不可达的隔离缺陷=多租户就绪债不夸大成 High |
| 140 | 2026-07-14 | auth-routes-security-audit-findings | 安全/审计（台账） | 台账（17 findings） | 0 High/2 Medium/15 Low；核心正向结论：指标聚合端点跨 workspace 泄漏零命中、凭证泄漏面干净（api_key 恒 mask）、knowledge 写端点全双端复合锁；Medium 2 条均多租户就绪债（1-01 enable_agent account 存在性校验漏 workspace/4-01 taxonomy 一族 7 handler 零隔离——模型层无 workspace_id 是有意设计两处注释亲证，可写故不 WontFix 留待多租户裁决）；元家族=「写 filter 不复述 workspace_id 的纵深缺口」（7 条同型，读门兜住） |
| 141 | 2026-07-14 | f01-reclaim-chat-search-design | 发送治理 | 修复设计（用户已批准） | F-01：outbox reclaim 崩溃恢复分支 text 路只查本地 mcp_call_logs（best-effort 写、崩溃时恰最不可靠）而 timeout 分支先查权威 chat_search——PR#164 漏同步；崩溃窗口可重发同一句给真实客户；修复=抽 verify_already_sent 共用函数（referral→false/media→media_already_succeeded/text→chat_search 带 15s 超时回落本地）根除未来漂移；同 PR 批量翻正 6 条台账状态（B-02/C-01/H-01/D-01/F-02→Fixed，H-02→WontFix doc 标注） |
| 142 | 2026-07-14 | knowledge-agent-autonomy-redline-design | 知识/红线 | 设计待审（批次 1） | 影子模拟实测发现架构缺口：Reply 最终话术红线守住但知识 Agent 中间答案（knowledgeRoute.reason）满口"转人工"——反接管红线只种在 DB prompt 模板，知识 Agent 用代码内联 const SYSTEM_PROMPT 是红线盲区，忠实复现知识库里传统医美"转人工 SOP"；reason 经两路回流（decision 保留 reason/review 整条序列化）；目前靠 Reply 每轮"持续对抗"掰回来，预算降级时可能泄漏；两层修复=知识 Agent SYSTEM_PROMPT 补角色定位约束（不用关键词黑名单——约束角色认知非禁词）+reason 从下游剔除（结构化字段承载充分度信号，reason 是唯一被污染的自然语言，切除不丢信号） |
| 143 | 2026-07-14 | manual-ops-enrollment-source-of-truth-design | 通讯录/webhook | 设计已获批 | 生产实证：AI 给非真人号（福州晚报 22 条/电台/营销号/账号自己"Demi"）机械回复；根因链=管理员批量框选混入非真人号（human_profile_note="从运营池批量启用"铁证）+hide_from_pool 只写 hidden 不改 agent_status（移出池不停回复矛盾态）+is_operatable_person 判据盲区（wxid_ 前缀营销号天然过闸）；核心决策=**放弃自动判"真人"**（判不准会误伤），以管理员手动加入运营为唯一真相；只保留硬事实拦截（gh_/@chatroom/@openim）+逻辑铁律（is_self_account 账号不能运营自己，与真人判据解耦）；hide_from_pool 联动写 agent_status=normal（写入侧单一真相源，回复门不加读侧过滤）；加入/移出补审计事件；存量 4 号 mongosh 一次性清理 |
| 144 | 2026-07-14 | p3-family5-knowledge-readiness-design | 知识 | 未标注（KB-07 修/KB-06/12 doc 标注） | KB-07：在线 gap 信号 dedup 用 find_one 无序+单条 filter——同 kind 多主题时 find_one 返任意一条不匹配 dedup_key→漏合并产重复条；修复只换"找 existing"一步（全量 find+精确命中），保留在线合并语义一字不动（绝不抽共用函数强并在线/离线——两路径 search_queries/source/auto-resolve 语义不同）；KB-06 structural_proposals 无 apply 消费方→模块头补"生产未接线"标注；KB-12 reviewer_stats 刻意 workspace 级→补 doc |
| 145 | 2026-07-14 | p3-family6-provider-mcp-config-design | 配置/MCP | 未标注（KD-10/KD-09/KE-06 修复） | KD-10：provider 热切换先写 DB 后 swap，swap 失败留"DB 已翻但运行时旧 client"假失败→先 swap 成功再写 DB（swap 纯构造+原子替换无 DB 副作用亲验）；KD-09：openai 形态 base_url 缺 /v1→405，软 warning 不 hard block（Azure/代理网关路径不一会误伤）；KE-06：sync_accounts $set 覆盖手配 mcp_base_url，与 mcp_api_key 的 $setOnInsert 不对称→移 $setOnInsert 对齐 |
| 146 | 2026-07-14 | p3-family7-escalation-reassign-guard-design | 请示 | 未标注（KD-07 修复） | KD-07：首推有"决策人==客户"守卫但超时改派无——admin 误把客户 wxid 配进 decider_chain[1..] 时改派会把内部请示卡直推客户（泄漏 reason/question/标签）；修复方向③=next_decider_on_timeout 加 contact_wxid 参+跳过逻辑（根上正确、天然复用链尾安抚兜底——台账初选①推卡点 continue 会每 tick 重算同一非法 next 永久卡住客户被晾死）；不加写入口校验（拦不住"先配 decider 后客户变 managed"时序窗 YAGNI） |
| 147 | 2026-07-14 | p3-family8-webhook-edge-design | webhook | 未标注（A-06/A-05 修/A-03/A-04 WontFix） | A-06：last_inbound_at 统计 update 用 ? 抛错——Mongo 瞬时错误吞掉本轮客户回复→降 best-effort（与紧邻 behavior_signals 旁路纪律对齐，inbound insert fail-close 不动）；A-05：无 appId 回落 default account 张冠李戴——关键亲验 verify=true 时验签门天然挡死（secret=None→400），真实危害面仅 verify=false+多账号→count 收敛到 !verify 分支内多账号 400（生产零额外查询）；A-03 payload-hash 去重/A-04 无 nonce 重放→doc 标注 WontFix（生产 GeWe 恒带 NewMsgId/幂等已缓解） |
| 148 | 2026-07-14 | p3-family9-outbox-timing-design | 发送治理 | 未标注（F-04/B-03 修/B-01 标注） | F-04：reclaim 不消耗 attempt——worker 同位置反复崩溃→无限 reclaim 永不进终态；修复=独立 reclaim_count>5 转 failed_terminal（不复用 max_attempts——reclaim≠发送 attempt）；B-03：managed→normal 翻转在决策运行期（10-15s）不复核→照发在途回复；落点=second_safety_gate（发送前 fresh 查 contact 已有，加 is_managed 参零额外 DB 读）；B-01 协作式抢占入队尾窗双回复→标注不修（gen 撤销补偿触碰 outbox 幂等核心，产品取舍待专项） |
| 149 | 2026-07-14 | principal-authorization-exemption-design | 请示/知识 | 设计已获批 | 生产实证缺陷：领导 approved 的产品说法被系统自己的产品门二次拦截（relay 走同一 gateway，R5.4 重新判定 verified_chunks 空→blocked；领导口头授权不写知识库）→客户永远收兜底；设计=领导裁决时可选两类豁免：A 类一次性客户级（domain_attributes.principal_product_exemption 长期常驻+admin 可撤销+finalize 加 principal_product_exempted 旁路）/B 类沉淀复用（新 provenance PrincipalAuthorized 归"人类权威"家族直落 verified+active）；关键红线定性（用户拍板）=「领导裁决=人工验证」——验证主体是真人，"AI 永不自动验证"红线本质未破；B 类必须走 apply_chunk_revision 直写绕开 verify.rs 的 LLM 自评降级链；B 是 A 的超集（先写 A 当轮即通再沉淀） |
| 150 | 2026-07-14 | worker-fleet-audit-design | 测试/审计（方法论） | 只审不修 | 第三批：后台 worker 群 ≈3131 行（tasks/cold_contact/behavior_signals/account_scheduler/silence_signal/import_worker/supervisor）；核心命题=claim/CAS 竞态+崩溃回收+幂等+调度去重（outbox 元家族在主动侧 worker 的延伸）；关键校准=单进程默认部署不可达的多副本竞态=水平扩展就绪债不夸大 High；边界排除防与后续批次重叠（outbox_dispatcher/planner/evolution/knowledge_* 各留专批） |
| 151 | 2026-07-14 | worker-fleet-audit-findings | 测试/审计（台账） | 台账（18 findings） | **1 High**/2 Medium/15 Low；唯一 High=[1-01] initial_profile 任务成功后无终态写入（四 kind 里唯一不写 tasks 终态——单进程每条确定性命中：停 running→reclaim 反复重跑 3×LLM+旧初始画像覆写窗口内累积更新→recovery≥3 误判 failed）；Medium=[S-01/S-02] admin 侧门 review_task_now 绕开 claim CAS/不写 claimed_at（本进程内永不回收盲区）；元家族=「统一 claim/终态纪律的层间不对称」（新增 kind 漏隐式契约/旁路绕开 CAS/收尾写非 CAS）；正向 HOLDS：worker claim 原子 CAS/behavior_signals 只写不进决策（全仓零生产读取点）/cold_reactivation_idempotent/supervisor 11 worker 判定安全 |
| 152 | 2026-07-15 | audit-cross-verification | 测试/审计（交叉验证） | 报告（主控整理） | 六批审查台账全部 18 条 High/Medium 的对抗式交叉验证：每条派独立 agent 亲读代码**主动试图证伪**；结论=代码事实 100% 属实零虚报（无捏造/引用错位）、Verdict=CONFIRMED 14/OVERSTATED 4/REFUTED 0；4 条 Medium 校正为 Low（全在观测/桩层，偏差单向宁高不低）；唯一 High（initial_profile 无终态）站得住且**比台账更严重**——claimed_at 是 null-present 使 reclaim 分支 B 恒死分支，重启也不回收；修复优先级不变仍是全局 P0 |
| 153 | 2026-07-15 | audit-medium-remediation-design | 修复（综合） | 设计（分 3 PR） | 六批审计收官后剩余 Medium 修复：范围=Batch1 五条（A-01 弱证据 importance 天花板轻方案/A-02 消费 discardedTags 改"保留 unless 显式弃用"/A-03 confirmed_tags 写改 fail-soft）+Batch5 2-01（post_release 5 闸映射对调修正+pressure 改 revision 口径——pressure 是软闸生产不产 block 终态，光对调不解决根因）+C-01 stagnation 写侧动态化（深度校准发现浅修换 key 名是**假修**——触发信号 stage_changed 只反映 customer_stage 变化，必须检测停滞维度自身值变化）+C-02 初始画像接 guidance；全部遵守"DEFAULT 销售域字节等价"；A-02 取舍留痕（交叉验证曾降 Low、用户裁定保留修） |
| 154 | 2026-07-15 | db-migrations-indexes-audit-design | 测试/审计（方法论） | 只审不修（第六批·末批） | 最后一个未深审领域：migrations（32 文件 3810 行）+indexes（1765）+db/mod（405）≈5980 行；本批**全程主控亲审**（subagent 派发连续 API 失败）；4 组（A 框架根因层/B 数据变形/C seed+drop/D indexes）；关键校准=APP_ENV 守卫迁移的危险性取决于"生产是否已入账该 id"——已入账永不重跑，不凭"非 prod 会删"夸 High |
| 155 | 2026-07-15 | db-migrations-indexes-audit-findings | 测试/审计（台账） | 台账（3 findings） | 0 High/1 Medium/2 Low——六批中最干净的一批；唯一 Medium=[C-01] m011 对 operation_knowledge_chunks（**当前 wiki 存活集合、与"legacy 清理"同名同用**）无条件 delete_many({})，破坏性完全押 APP_ENV 守卫+已入账双条件（未设 env+未入账窗口=清空全部 verified 知识）；修复入口是运维动作非代码（117 显式设 APP_ENV=production）；正向 HOLDS：迁移框架幂等契约/守卫 warn+Ok 形态正确（返 Err 会 boot-brick）/12 条数据变形迁移全幂等/seed 全 $setOnInsert 不覆盖运营编辑/outbox 等幂等键 unique 全覆盖 |
| 156 | 2026-07-15 | evolution-audit-design | 测试/审计（方法论） | 只审不修（第五批） | evolution/ 自优化演化器 6095 行 14 文件；4 簇（S 根因层/1 候选生成/2 shadow 评估/3 放量闭环）；核心红线核查=隔离红线（禁引 gateway/outbox/mcp，CI 静态扫描）+R9.7 禁自动回滚+auto_release 唯一自动放量点；注意 EVOLUTION_ENABLED 默认值张力（真实默认 true 但注释称 false） |
| 157 | 2026-07-15 | evolution-audit-findings | 测试/审计（台账） | 台账（11 findings） | 0 High/2 Medium/9 Low；[S-01] runtime_flag=None 被 cohort 当"全量收"与 is_evolution_enabled_for 的"全员排除"语义分叉（同一安全语义两个门函数实现不一致）——默认部署演化跑全流量而非灰度桶，但产出一律 awaiting_admin+auto_release 双闸默认关故非 High；[2-01] post_release 面板 5 闸映射与 threshold/significance **恰好对调**且都与生产真相不符（三文件三向不一致——同一领域映射散在多文件必然漂移），关键分野=shadow 闭环自造合成 status 同口径判定→自洽零后果（[1-01] Low），post_release 读生产真实 status→贴错标签（Medium）；正向 HOLDS：R9.7 禁自动回滚（evolution/ 内零调用 rollback）/auto_release 双闸默认关/prompt 绝不自动放量/release 三写同事务/隔离红线 CI 已接线 |
| 158 | 2026-07-15 | import-failed-segment-visibility-design | 知识/前端 | 设计已获批 | M1（CONFIRMED 纯前端）：长文档异步导入部分段失败时后端数据链已完全就绪（importReport{totalSegments,succeeded,failed}），前端从不读 failed——进度条照显"已完成 N/N"、用户误以为内容完整；修复=step2 顶部非阻断黄色警示条（以 result.importReport 为唯一权威源）；M4（OVERSTATED 低概率）：chunk_haystack 拼检索文本独漏 product_tags（tag 词面天然取自 body 多数自愈）→补一行对称 business_topics |
| 159 | 2026-07-15 | knowledge-wiki-audit-design | 测试/审计（方法论） | 只审不修（第四批） | knowledge_wiki/ 子系统 5272 行 11 文件（前几轮只审过召回算法与 HTTP 端点层，子系统内部从未深审——覆盖缺口最大）；4 簇（S 写入根因层 page_merge+chunk_revisions/1 信号生成消解/2 摄取源头/3 反馈闭环）；红线核查=AI source 强制 draft/structural 只产 pending_review/模块隔离 |
| 160 | 2026-07-15 | knowledge-wiki-audit-findings | 测试/审计（台账） | 台账（32 findings） | 0 High/5 Medium/27 Low（subagent 自评 3H/10M，主控全部降级——防夸大红线）；Medium：[3-01] lessons_learned 幽灵字段 filter 三重落空（查错集合/子结构缺失/字段名从不存在）恒命中 0+[3-02] blocked 支 filter 与状态机派生规则互斥恒 0——两条联合使整模块 inert；[2-01] ingest 零内容去重幂等全押 ETag 单点（默认关）；[1-01] gap_signals 无业务去重唯一索引；[S-08] apply_chunk_revision 读改写无乐观锁 lost update；元家族=「filter 字段与真实写点数据形状/状态机派生规则不对齐致恒空」+「幂等靠应用层 find-then-insert 非唯一索引」；正向 HOLDS：AI 永不自动 verify 红线跨全摄取入口/信号只观测不进决策/page_merge 纯函数 15 单测 |
| 161 | 2026-07-15 | sediment-title-from-substance-design | 请示/知识 | 设计待实现 | 领导授权 B 类沉淀真实测试（E6PM5）暴露缺陷：沉淀 chunk 的 title 用了 entry.reason（Review Agent 质检点评黑话"emotionalValue 只到 6……"）而非知识标题；三重影响=title 是召回打分权重最高信号（×3）质检黑话对产品问句零命中→verified 知识召不回/黑话进决策 prompt/展示不可读；修复=LLM 提炼 title（新 prompt_key）+确定性兜底纯函数（substance 首句+40 字符限长）+存量 2 条一次性 mongosh 脚本修正；不改 reason 本身语义（问题只在"拿 reason 当标题"） |
| 162 | 2026-08-05 | run-log-stage-table-overflow-design | 前端 | 修复设计 | 运行日志 run envelope 展开区六阶段表格横向溢出+标签逐字竖排：四层根因（.tHead flex 容器误用于嵌套 width:100% 表格——该类是为事件时间线设计/.main 缺 min-width:0 溢出逃逸页面级/thead 5 列 vs 摘要行 6 列/JSON.stringify 长单行无断点）；修复=新增 .stageBlock/.stageTable（table-layout:fixed+key 列 38%+word-break）不动 .tHead；唯一全局改动 .main 加 min-width:0 需全量回归 |
| 163 | 2026-08-05 | wiki-admin-governance-css-baseline-design | 前端 | 修复设计 | 治理工坊五处渲染缺陷（一处功能缺陷：不可逆高危按钮"发布给全部"白底白字不可见）；根因=.wikiAdmin* 按"从零编写"方式写但实际运行在全局元素基线之上，未覆盖属性露出全局值；关键决策=不能在 .wikiPublishBar button 规则补 color（特异性计算(0,2,1)会胜过 --verify/--reject 的(0,2,0) 修一个换两个回归）→给按钮补语义 class wikiActionBtn--neutral；全局改动仅 input width 排除 checkbox/radio（38 处逐一核验 0 处依赖）；诚实记录 jsdom 无布局引擎不假装覆盖视觉断言 |
| 164 | 2026-08-06 | ask-human-inbox-layout-design | 前端/请示 | 设计 | 统一收件箱布局对齐：四现象同一根因=该频道未采用全项目 .page+白卡结构自建外壳（max-width:920 全项目唯一/双重 padding/无一张卡/4 个 reviewQueue* class 全库零 CSS 定义）；宽度测算（9 chip×94px=911px vs 实际可用 864px 必然折行）决定方案并否决"chip 与按钮同排"；命名必须带 askHuman 前缀（plain CSS 全局作用域，裸 .panel 会与用户运营频道互相污染）；AskHuman.css 必须保持 plain css（改 module 会被 Rollup tree-shake 删光） |
| 165 | 2026-08-06 | decider-chain-roster-picker-design | 前端/请示 | 设计（纯前端） | 决策人链"从联系人添加"名不副实（拉本地 contacts 表非微信通讯录）→改用共享 FriendPickerModal+roster 数据源；所有复杂度来源=后端硬约束（put_ask_human_policy fail-closed 校验决策人必须已在 contacts 表）；roster not_imported 好友需先走 POST /api/contacts/import（$setOnInsert agent_status=normal 只导入不托管——绝不可用 batch-enable 把决策者当客户交给 AI 运营）；两个"看起来成功但没生效"的坑=HTTP 200 不代表导入成功（upsert None 静默跳过须查 items.length）+roster 非真人判据比后端宽松（须前端双重过滤）；syncing 态抄 RosterView 不抄 referral-cards（后者未处理 syncing 是同类缺陷不在本次修） |
## 2. 按主题域的设计决策史

### 2.1 知识库（wiki 化 → 治理 → 导入 → 召回 → 修复闭环）

- **2026-05-25 大清理（knowledge-base-cleanup）是全部知识域叙事的起点**：旧"销售话术 RAG"整体退役，wiki 方法论独占。删除 Item struct、35 个销售域字段、3 个销售 taxonomy seed、fact_risk/pressure_risk/product_accuracy 三个字符串级阈值守卫；决策闸从 5 个收敛为 3 个核心闸（knowledge_grounding / hallucination / run_budget）；baseline PBT 从 string_fact_risk_guard 换成 wiki_chunk_revision_pbt。此后所有"审计发现 fail-open"的争论（如 07-14 audit D-08 claim_analysis 缺失放行）都回溯到这一天的显式取舍。
- **2026-06-06 起进入"验证门收口"阶段**：grounding-and-verify-gate-fixes 确立 D2 硬门（verified 必须 source_quote+source_anchors，否则降级 needs_review 而非 400）与"词类型切分"硬闸（效果/数据词拦、语气词仅观测——刻意避免恢复 05-25 删掉的脆弱 string-marker）。06-07 trust-cockpit 把治理逻辑前端"显形"，并澄清 auto-verify 的语义主体是运营授权批处理而非 AI 自验。
- **结构性拆分**（06-07 routes-split 9378 行拆 9 文件、06-08 frontend-split 6582 行拆 4 mode）确立"facade re-export + 纯机械搬运零逻辑改动"的重构范式，后被 escalation-split（06-08）复用。
- **导入链路三级演进**：06-26 full-business-logic-test 发现 import-preview 503 → 07-12 long-doc-chunked-import 三方确认根因是**输出自回归生成瓶颈**（非 prompt 体积），后端自动分块+并行度 2+确定性合并；07-12 async-import-job-progress 把 9 分钟同步死等改异步 import_jobs+worker+轮询；07-15 import-failed-segment-visibility 补最后一环——部分段失败的前端可见性（后端数据链已就绪，前端从不读 failed）。
- **修复闭环**：06-27 chunk-ai-repair-closure 接通 AI 修复建议→前端逐字段接受/拒绝；06-28 e4-document-repair 把已死的"pack repair"（items 集合已删）重定义为文档级批量修复；07-13 p3-family1 删除 propose_pack_repair 死桩。07-15 knowledge-wiki-audit（第四批，32 findings）暴露反馈闭环的"幽灵字段 filter"元家族——lessons_learned 三类模式全 inert（查错集合/字段从不存在/状态机互斥）。
- **知识 Agent 红线盲区**（07-14 knowledge-agent-autonomy-redline）：影子实测发现反接管红线只种在 DB prompt 模板，知识 Agent 用代码内联 SYSTEM_PROMPT 完全没被覆盖，其 answer 满口"转人工"（忠实复现医美知识库的转接 SOP），经 route.reason 回流 Reply/Review——修法是校准角色认知（"你是给内部 Reply Agent 的知识研判"）+ 把 reason 从下游剔除，明确拒绝关键词黑名单。

### 2.2 决策请示通道（principal / ask-human）

- **2026-06-05 principal-decision-channel 是奠基篇**：确立"幕后领导模式"——AI 撞墙分两类（缺知识→KB 闭环；缺决策→请示领导），客户永远只跟 AI 对话，请示不是转人工。核心模型 2026-06-06 拍板"统一走安全占位"：触发请示的 run 照常 Approved、发安全占位句，请示是 approved 路径上的副作用（不进 Held、不碰 review.rs）。6 个业务决策锁定：请示模式可配、超时无限等待永不自动代决、泛化沉淀 agent 自判、真人回复无短码弱匹配、多轮卡死=N 轮未推进+负面反应、等待期非越权部分照常回。
- **2026-06-21 三件套把请示通道产品化**：ask-human-unified-channel（AskHumanPolicy：decider_chain 决策人链/触发器/频控+超时改派 worker）、ask-human-config-page（图形化配置）、ask-human-inbox-frontend（8 源统一收件箱，"坏源不坏好源"降级）。
- **2026-06-30 h10-relay-identity-provenance-fix 是安全里程碑**：客户可伪造 `__PRINCIPAL_RELAY__` 哨兵劫持转述模式——修法把身份判定从"内容前缀"换成不可反序列化的 `is_synthetic_relay` 结构标记（身份基于来源而非可伪造内容）。
- **2026-07-11~12 深度审计暴露请示通道成色**：唯一 High KD-04（推荐配置下领导微信回复永不被识别——lookup_principal_config 只查旧标量 principal_decider 不看 decider_chain，修法复用权威解析器）；KD-05 骚扰门口径漂移（reassign 不刷时间戳）；KD-06 改链孤儿 pending 回落链首；KD-07 改派缺"决策人==客户"守卫（跳过误配成员）。
- **字符级护栏的系统性退潮**（详见 2.6）：KD-01/03 relay 数字护栏被裁定为"威胁模型错误的 backstop"整体删除（07-12）；KD-02"领导泄漏"词表被裁决不加（同理）。
- **2026-07-14 principal-authorization-exemption 补上最后一块闭环**：生产实证领导 approved 的产品说法被系统自己的产品门二次拦截（领导口头授权不写知识库→R5.4 重判 verified 为空→blocked）——引入 A 类客户级豁免（domain_attributes 长期常驻可撤销）与 B 类沉淀复用（新 provenance PrincipalAuthorized 归"人类权威"家族直落 verified，红线定性=「领导裁决=人工验证」，验证主体是真人红线未破）。07-15 sediment-title-from-substance 再修 B 类沉淀 title 误用 reviewer 质检黑话（title 是召回打分 ×3 权重最高信号）。
- **前端收尾**（08-06）：ask-human-inbox-layout 对齐全项目 .page+白卡结构；decider-chain-roster-picker 把决策人链选人从"本地 contacts 表"换成微信通讯录选择器（含"只导入不托管"路径，绝不可用 batch-enable 把决策者当客户交给 AI 运营）。

### 2.3 引荐名片与辅助模式（referral）

- 2026-06-21 referral-card-push 确立"辅助模式受控例外"：账号显式开启+AI 判定契合引荐条件→主动推真人顾问名片，AI 退为辅助答疑；这是管理员显式配置的业务动作，不破全自治红线（被引荐顾问 ≠ 幕后决策源）。CLAUDE.md 红线段为此显式修订。
- 2026-06-28 memory-conflict-and-reviewer-yield 给 reviewer 注入显式语言，防止其把名片推荐误判为转人工红线违规。
- 2026-07-13 p3-family3 加固：账号归属校验落到 validate_card_sendable 纯函数（enabled/approved 同层、一处修改三处生效）；toggle/delete 补 fail-soft 审计（保留硬删，软删需改全链 5+ 查询 YAGNI）。
- 2026-07-11 friend-picker-modal 抽出共享 FriendPickerModal（referral 与 products-deals 复用），08-06 决策人链也迁移到它。

### 2.4 通用域适配（universalization）

- **2026-06-11 universal-domain-adaptation 是总纲**：H1–H19 硬编码点全图；DomainProfile=行业总装配单；正解=「AI 通过对话生成稳定配置+人审」，明确拒绝「运行时自由发明维度」；DEFAULT 逐字（字节）等价是贯穿所有改造的反过拟合护栏。数字分身、扫描器家族（renewal/reactivation/calendar）同期落地。
- 06-15~06-20 密集收口：dimension-registry（中央维度注册表+WriteIntent 正交轴）、universal-business-gaps（prompt override+relationship_type 建议-审核链；实证裁决"五闸可配=过度设计砍掉"）、universal-residuals（状态机本体随 profile/轨迹维度容器化/debounce per-profile）、universal-audit-remediation（13 域深审后修复批）。06-23 reactivation-stage-universalization 用 is_reactivation_target taxonomy 旗标替换硬编码 stage 键。06-25 taxonomy-label-wiring 打通中文显示名链路。
- **07-14 agent-capabilities-audit 复核成色**：引擎层"远比预期健康，无写死销售死字段"（销售字面量全是有字节等价护栏的 default seed）；真残留只有 C-01 stagnation 读写不对称（读侧全动态、写侧写死 customer_stage_updated_at）与 C-02 初始画像半接线。07-15 audit-medium-remediation 修 C-01 时的深度校准值得记录：浅修（只换 key 名）是**假修**——触发信号本身只反映 customer_stage 变化，必须检测停滞维度自身值变化。

### 2.5 记忆与标签可信度

- **记忆固化主线**：06-28 memory-consolidation-guards（compact 按维度解决冲突+非原子事实检测重试）与 memory-conflict-and-reviewer-yield（事实原子化+deprecate_same_dimension_conflicts）同日落地；06-29 memory-summary-not-authoritative-fact 修正架构级错误——memory_summary（暂存便签）曾被当权威 core_fact 注入种子卡。
- **标签可信度改造**（06-23 tag-trust-two-layer，配 5 个子 plan）：三层物理隔离——人工 manual_tags（权威）/AI confirmed_tags（整理确信）/tag_observation（待定），证据 fail-closed，bayesian/personality 只写不进决策。07-14 审计复核"8 条铁律全 HOLDS"，但暴露镜像不对称元家族：A-01 core_facts 缺证据门（tags/personality 有）vs A-02 confirmed_tags 缺保留合并（core_facts 有"未显式弃用即保留"）——A-02 在截断窗口整体 replace 会丢证据滚出窗口的持久标签。07-15 remediation 消费从未被消费的 discardedTags 通道实现"保留 unless 显式弃用"。
- 分类学（taxonomy）侧：06-22 taxonomy-admin-crud、07-07 candidate-inbox-card（修"通过按钮必然 400"）、07-08 candidate-display-name（LLM 自造新值时顺带产中文建议名，agent-first）、07-11 batch-filter（只做批量驳回不做批量采纳——采纳需逐条人工填 canonicalValue 是字典质量红线）。

### 2.6 守卫/闸门哲学的演进（本仓最重要的思想主线）

时间线上是一条清晰的"从字符匹配到语义判断"的单向演进：

1. **2026-05-25**：删除字符串级销售守卫（fact_risk 词表等），5 闸收敛 3 闸。此后"5 闸"一词在文档中改指 review 评分体系的五个分数阈值（factRisk/pressureRisk/humanLike/emotionalValue/productAccuracy，见 evolution 系列 spec 的 five_gate_hit_rate），2026-06-05 plan 的锚点修正段落明说"旧五闸门已删除，当前是三闸门+review 评分体系"。
2. **2026-06-19 evaluation-system-overhaul**：把评测侧的词表硬门（HANDOFF_MARKERS panic）下线，改多采样跨家族中位数 LLM 硬门。
3. **2026-06-20 universal-audit 用户裁定**："转人工红线从词表 panic 换纯 LLM 硬门"。
4. **2026-07-12 kd-family-relay-number-guard**：删除 relay 字符级数字护栏——判定"转述是否忠于授权"是语义问题，字符提数字做白名单差比对是范畴错误（既漏"八折"又误杀"24小时"）；与 grounding 硬闸的本质区别被点破：grounding 可确定性判定因为"有没有 verified chunk"是客观集合运算。
5. **2026-07-12 kd-family2**：KD-02"领导泄漏"词表裁决不加（同一逻辑）。
6. **例外边界同样清晰**：载荷泄漏守卫（固定内部标记存在性检测）保留——威胁模型正确；holding_reply 对数字函数的调用保留——命中回落兜底文案而非 fail-closed。**结论：确定性代码闸只用于客观集合运算，语义判断交 LLM+独立 Review。**
- 与此并行的是**fail-soft 纪律的确立**：纯审计/统计旁路写失败只 warn 不拦主流程（07-12 audit-events-failsoft-alignment、07-14 p3-family8 A-06），但 inbound insert、发送准入等主链保持 fail-close。

### 2.7 发送治理（outbox / 幂等 / 防重发 / campaign）

- 基础设施在更早的 spec 之外（outbox+幂等键+二次安全门是 autonomy-loop spec 的遗产），本目录内的演进集中在**防重发与崩溃恢复**：07-10 outbox-chat-search-idempotency（吴界收到 3 条重复占位实锤——150s timeout 取消 send future→本地日志没写→误判没发过；修法把核对源升级为 MCP chat_search 服务端真实记录）；07-14 f01-reclaim-chat-search（reclaim 崩溃恢复分支漏同样的权威核对，抽 verify_already_sent 共用函数根除漂移）；07-14 p3-family9（reclaim_count>5 转终态止住无限 reclaim；second_safety_gate 发送前 fresh 复核 managed）。
- **节流**：06-24 account-send-pacing-guard（账号级 1-4s 随机间隔防"机关枪"）；07-10 passive-reply-daily-limit（daily_limit 收窄为仅 FollowUp——客户主动问不该被拦）。
- **campaign 主线**：06-28 三件套（sends-report 7 桶归类只读聚合 / targeted-push 两阶段圈人+follow_up 复用主网关+人审确认门 / frontend 看板）→ 06-29 domain-completion（前端红线：无直接 dispatch 按钮）→ 07-12 kc-family1（dispatch 三步非原子→补偿回滚+status 前置门）与 kc-family2（粗筛口径分裂：serde 默认只在反序列化补、Mongo 查询不补→"粗筛⊇精筛"被反转）→ 07-13 p3-family4（受众硬上限粗筛层 limit 探测法）。

### 2.8 webhook 与通讯录（roster）

- **webhook 三连修**（07-09）：gewe-addmsg-parse-fix（真实 GeWe 嵌套 payload 被通用 find_string 遮蔽→发件人归错/内容取脏串→真实客户消息永不触发 AI 回复；62 个 biz-test 全绿是形态盲区非假绿）；signature-verify-restore（联调期关闭验签是公网无鉴权入口，且两端方案已不匹配——方案 B 每账号密钥+时间戳防重放+fail-closed）；roster-mcp-ratelimit-syncing（429/503 从红条柔化为 syncing 态）。07-14 p3-family8 收尾边缘（统计 update 降 best-effort、无 appId 多账号 400）。
- **roster 长战役**（07-07~07-11，共 8 篇）：从 contacts_fetch_cache 解析器"从未跑通过真实数据"（返回纯 wxid 字符串数组而解析器要求 object）开始，逐步演进：fetch-full（4831 条富化字段）→ sex 解析（int64 序列化对象取 .low）→ 后端持久化快照（roster_snapshots 秒回+24h 后台自刷）→ single-flight refresh（根治卡死死循环：8s force 轮询叠加无去重 spawn 打爆 SSE 20 并发上限——DashMap 单飞锁+前端只读轮询）。
- **运营池身份治理**（07-10~07-14）：昵称全是"Demi"（find_string 递归命中账号 owner 昵称）→ gh_/@chatroom 过滤+m029 清洗 → manual-ops-enrollment-source-of-truth 确立核心决策：**放弃自动判"真人"**（判不准会误伤），以管理员手动加入运营为唯一真相；系统只保留硬事实拦截（公众号/群/@openim）+逻辑铁律（账号不能运营自己）；hide_from_pool 联动写 agent_status=normal（写入侧单一真相源）。

### 2.9 前端体系

- 06-04 frontend-ui-refactor 定基调（领域切片+CSS Modules+Zustand+语义色 token）。
- **frontend-backend 对齐战役**（06-26~06-27，4 批 76 项差距，0 误报）：系统性反模式=「后端可写前端只读」「静默吞错」；batch2 专修通用化残留（前端硬编码销售标签→labelFor 动态翻译）；batch4 收尾 SSE 指数退避重连。
- 06-28 frontend-backend-contract-alignment 建立**双门契约测试**（后端 Rust 快照测试产 fixture JSON+前端 Vitest 比对 canonical key 集），治"三处手工维护类型必然漂移"的病根。
- 07-04 inbox-ui-jargon-cleanup 确立黑话两类处置（A=结构化枚举加字典可译；B=黑话嵌拼接串必须改后端）。
- 08-05/08-06 三篇 CSS 修复展现同一元教训：**plain CSS 运行在全局基线之上，只覆盖部分属性就会露出全局值**（白底白字的高危按钮）；特异性计算必须先做（补 color 会修一个换两个回归）；jsdom 无布局引擎，视觉断言诚实标注"需目视"。

### 2.10 演化系统（evolution）

- 06-27 prompt-evolution-human-gated 定架构：AI 提案→shadow replay 产证据→人批准发布（三阶段），机制优先于 LLM 自觉。06-28 phase3 补证据展示（原/新 5 闸命中率 delta+逐样本对比）。06-30 evolution-ui-toggle 把 Mongo runtime flag 定为主开关、env 变量降为急停。
- 07-12 ke-family1 修 auto_release 方向一致性（doc 声称"方向一致才放行"实现只判 band 外任意一侧）与 threshold 重判口径对齐（非-5gate 因素污染 send_delta）。
- 07-15 evolution-audit（第五批，11 findings）：R9.7 禁自动回滚 HOLDS、隔离红线 HOLDS、auto_release 双闸默认关；两条 Medium 都是"同一语义多处实现必然漂移"——[S-01] runtime_flag=None 在两个门函数语义分叉（全量收 vs 全员排除）、[2-01] gate↔status 映射三文件三向不一致且 post_release 读生产真实 status 贴错面板标签。

### 2.11 测试与审计方法论（从脚本到工程）

方法论演进线：**闭环轨迹测试**（06-02，constraints>reward）→ **recall 基准**（06-04，reach/adopt 分层+对抗样本客观划分）→ **业务探针**（06-06，N 轮累积趋势+双轨判据）→ **roleplay fuzz**（06-15，五阶段每阶段只引一个新变量）→ **评判体系重构**（06-19，judge_conversation 全上下文）→ **全量业务逻辑测试**（06-26，生产服务器+真 LLM）→ **上线前全量测试方法论**（06-30，四级正确性+两阶段饱和审计）→ **全系统深度测试**（07-10，playwright 19 频道×5 维+发送红线三护栏）→ **深度逻辑审查**（07-11，53 findings，"从频道入口穿透到底"）→ **六批专项深审**（07-14~07-15：agent 旁挂 20/auth+routes 17/worker 群 18/knowledge_wiki 32/evolution 11/db+migrations 3，共 101 findings）→ **对抗式交叉验证**（07-15，18 条 High/Medium 逐条独立证伪，100% 属实零虚报，4 条单向高估校正）。
- 贯穿的纪律：只审不修、subagent 结论必主控亲验 file:line、两态标注（PLAUSIBLE/CONFIRMED）、严重度校准防夸大（单租户/单进程默认不可达=就绪债不夸 High）、反过拟合（绝不为发现问题改业务逻辑）。
- 六批总元家族："设计声称的不变量，实现层有旁路/缺口/非原子窗口/新旧不对称"——五层分型（错误处理/数据写入审计/多步非事务写/新旧字段迁移/保护策略不对称）。全程唯一全局 P0 是 worker-fleet [1-01] initial_profile 无终态写入。

### 2.12 安全

- 06-30 h3-cross-tenant-idor（14 个 handler 补 resolve_authorized_workspace ACL）与 h10-relay-identity-provenance（身份基于来源非内容）是两个真漏洞修复；07-04 login-timing-side-channel（假 PHC 哈希抹平用户名枚举时序差，关键不变量=假哈希必须合法否则重新制造时序差）；07-09 webhook-signature-verify-restore（fail-closed 验签）。
- 07-14 auth-routes-security-audit 系统性结论：认证链干净、指标聚合零跨租户泄漏、凭证恒 mask；实质缺口集中在"多租户就绪债"（taxonomy 一族无 workspace 字段是有意设计待裁决）与"写 filter 不复述 workspace_id 的纵深缺口"元家族。

### 2.13 prompt 体系

- prompt pack 分层（Soul→System Contract→Policy→Business Context→Operator Instruction）版本化管理；06-26 prompt-pack-startup-alignment 把版本号对齐换成**内容 diff 对齐**（改 prompts.rs 即重启生效，且修复 reset 误删 evolution critic 不再重种的缺陷）。
- 06-23 progressive-prompt-three-tier（Lean/Relational/Full 三档 AI 自评升档）→ 06-24 hardening（知识缺口 force full、used_knowledge_ids 记账修复防 grounding 旁路）→ 07-12 kb01（非 Full 档清空 used_knowledge_ids 防 LLM 自报 id 架空硬闸）→ 07-06 escalated-run-budget（升档 run 两程叠加撑爆预算→分档 gating 上限、计数保持诚实）。
- 06-29/06-30 内容资产注入系列：min_inject_tier 分层注入、禁用表达独立注入区（防被当"可引用素材"）、共享截断上限拆分。
- 07-13 reply-prompt-slimming 是性能收口：生产 39-43k token/45-149s 的根因诊断+三批次瘦身（零风险清理→A/B 中风险→逐项单独 A/B 高风险），复用 prompt_templates 多 active hash 分桶灰度设施；其"写 plan 时逐行核对剔除批次 1 第 5 条"是反过拟合纪律的实操样本。

## 3. 高价值决策档案

以下 33 篇为全集中最具决策价值者，各写一段详细提炼（背景/决策/理由/影响模块）。引荐名片（2026-06-21）与决策请示（2026-06-05）两篇已有独立深读，此处只留索引条目。

**1) 2026-05-25 knowledge-base-cleanup** — 背景：旧销售话术 RAG 与 wiki 方法论并存，销售域字符串守卫脆弱且行业绑死。决策：route A 一次性 big bang（9 commit），删 Item struct/35 销售字段/3 taxonomy seed/3 字符串阈值守卫，5 闸收敛 3 闸（grounding/hallucination/budget），Contact 增 domain_attributes 自由 KV。理由：字符串级守卫既误杀又漏判，行业无关引擎不能背着销售包袱。影响：models.rs、guards.rs、review/、baseline 脚本（LIB_BASELINE=350、PBT 替换）。**这是后续一切"5 闸语义漂移"讨论的原点**——之后文档里的"5 闸"指 review 五个分数阈值。

**2) 2026-06-04 recall-rate-benchmark** — 背景：知识编辑后召回是否退化无客观标尺。决策：recall@k 分 reach（检索触达）/adopt（回复采纳）两层；对抗样本用 bigram 重叠客观划分（防人工挑样作弊）；跨轮稳定性⓪先行（不稳定的基准测不出变更影响）；第一轮不设硬 floor。影响：scripts/biz-test 基准脚本族。

**3) 2026-06-05 principal-decision-channel**（已有深读，略）— 关键补充：其 plan 含两处 spec 锚点漂移修正，其中"旧五闸门已删除，当前是 knowledge_grounding/hallucination/run_budget 三闸门+review 评分体系"是全集内对 5 闸→3 闸+分数体系最明确的文字记录。

**4) 2026-06-11 universal-domain-adaptation** — 背景：19 处硬编码销售假设散布 6 层。决策：DomainProfile 作为行业总装配单；配置由"AI 对话生成+人审发布"产生，拒绝运行时自由发明；DEFAULT 与现行为字节等价作硬护栏。理由：稳定性与可审计性优先于灵活性；运行时发明维度会让行为不可回归测试。影响：domain_profile.rs（2454 行）、domain_signals、dimension_registry、prompts 渲染函数族、状态机配置。

**5) 2026-06-15 objective-purchase-facts** — 背景：系统无客观成交事实，AI 凭对话猜测购买状态。决策：Product 目录+OutcomeEvent（verification 三级：conversation_inferred/staff_confirmed/payment_verified）+持有投影（派生不存储）；红线「AI 永不自断成交」——AI 最多写 conversation_inferred，staff_confirmed 以上必须人工。影响：models.rs、entitlements.rs、routes/products、campaign 圈人的数据底座。

**6) 2026-06-15 roleplay-fuzz-testing** — 背景：固定脚本测不出探索性问题。决策：五阶段架构（fixture 校准→固定场景→LLM roleplayer→外部 judge→场景生成）且每阶段只引入一个新变量；失败必须带 suspected_layer 归因。理由：多变量混引无法归因；roleplayer 本身未校准就先当测试工具是自欺。影响：tests/ real-LLM 测试族、roleplay_fixtures。

**7) 2026-06-19 evaluation-system-overhaul** — 背景：J1-J6 评判失真（判 grounding 不给 ground、判一致性不给记忆、词表 panic 硬门误杀）。决策：统一 judge_conversation 内核全上下文喂养；HANDOFF_MARKERS 词表下线，换多采样跨家族中位数 LLM 硬门。理由：词表无法做语义判断（与 2.6 主线同源）。影响：评测基建、judge rubric。

**8) 2026-06-21 referral-card-push**（已有深读，略）— 全自治红线的唯一受控例外，红线段修订见 CLAUDE.md。

**9) 2026-06-21 ask-human-unified-channel** — 背景：请示通道只有单决策人标量配置。决策：AskHumanPolicy（decider_chain 链式决策人+触发器+频控+超时改派 worker）。影响：models.rs OperationDomainConfig、escalation/policy.rs、scan worker。注意：旧 principal_decider 标量与新 decider_chain 的双轨并存，正是后来 KD-04 High 缺陷（识别只查旧字段）的土壤——「新旧字段不对称」元家族的典型样本。

**10) 2026-06-22 business-audit-fix-wave** — 背景：ask-human 之外积累 11 个业务缺陷。决策：一波修复（请示过期/链尾无人接/数字守卫静默失败/知识过期/知识缺口未记录/customer_stage 写入缺状态机校验/账号级限额/离线账号处理）。价值：首次系统性"审计→修复波"运作模式，为 07-11 之后的大规模审计工程探路。

**11) 2026-06-23 progressive-prompt-three-tier** — 背景：所有对话全量注入 prompt，token 浪费且复杂对话反而"空回复"。决策：Lean/Relational/Full 三档，AI 自评信息充分度决定升档；隐私边界=禁止 LLM 复述 memory 内部事实。影响：decision.rs 注入槽体系、gateway 升档循环、runtime 参数。后续 06-24 hardening、07-06 escalated-run-budget、07-12 kb01 三次加固同一机制——**一个功能三次补丁的完整生命周期样本**。

**12) 2026-06-23 tag-trust-two-layer** — 背景：AI 标签不可信且 append-only 无法纠错。决策：三层物理隔离（manual_tags 人工权威/confirmed_tags AI 整理确信 replace 语义/tag_observation 待定），证据 fail-closed，bayesian/personality 只写不进决策。理由：可信度分层必须物理隔离而非打分混存。影响：models.rs Contact、memory.rs consolidation、gateway 写点、5 个子 plan 分阶段落地。

**13) 2026-06-26 frontend-backend-alignment-fixes** — 背景：前后端能力漂移积累。决策：全量盘点 76 项差距（0 误报）分 4 批修。价值：确认两个系统性反模式（后端可写前端只读/静默吞错）并把"对齐"从随机发现升格为审计工程。

**14) 2026-06-26 management-agent-thickening** — 背景：管理 agent 工具面太薄。决策：工具目录扩到配置/策略/知识端点；"提案→确认→执行"循环，高危操作人工门；执行结果如实回报（不粉饰失败）。影响：management.rs 工具目录、高危确认端点。

**15) 2026-06-26 prompt-pack-startup-alignment** — 背景：版本号对齐使 prompts.rs 改动不生效+reset 误删 critic 不重种。决策：改内容 diff 对齐（启动时按内容比对补齐）。理由：版本号是人肉协议必然漂移；内容才是事实。影响：prompts.rs ensure_prompt_pack_v2。后续 07-08 taxonomy-candidate-display-name 亲验纠正"不 bump PROMPT_PACK_VERSION"即依赖此机制。

**16) 2026-06-27 prompt-evolution-human-gated** — 背景：prompt 候选评估的 shadow replay 未实装、放量无人审。决策：AI 提案→shadow replay 产证据→人批准发布三阶段；机制优先于 LLM 自觉。影响：evolution/ 全模块（threshold/prompt_critic/replay/significance/release）。

**17) 2026-06-28 campaign-targeted-push** — 背景：无法按购买事实定向触达。决策：两阶段圈人（Mongo 粗筛+Rust 精筛）；活动经 follow_up AgentTask 汇入主发送网关（复用全部安全闸）；人审确认门+幂等 dispatch。理由：绝不为 campaign 另起发送旁路。影响：campaigns.rs、tasks.rs、entitlements。

**18) 2026-06-28 customer-reply-guarantee** — 背景：决策被闸拦下时客户零回复。决策：per-run 兜底回复守卫（无任何回复已发时入队通用占位句）；客户确认与请示解耦。**注意被 07-10 passive-reply 部分推翻**：硬编码占位破坏拟人→改 AI 生成（独立小预算+出站禁词守卫+硬编码降级兜底三支柱）。

**19) 2026-06-28 frontend-backend-contract-alignment** — 背景：Rust model→投影函数→TS 类型三处手工维护必然漂移。决策：双门契约测试（后端快照测试产 fixture JSON，前端 Vitest 比对 canonical key 集）。价值：把契约漂移从"人肉发现"变 CI 拦截，07-15 import 系列仍在引用此设施。

**20) 2026-06-29 memory-summary-not-authoritative-fact** — 背景：memory_summary（暂存便签）被当权威 core_fact 注入种子卡，导致错误冲突消解与非原子事实入库。决策：种子卡不再注入 memory_summary 为 core_fact，归位为 recentEpisodeSummary。价值：数据语义（权威层 vs 暂存层）必须在注入点显式区分的教科书案例。

**21) 2026-06-30 h10-relay-identity-provenance-fix** — 背景：客户可在消息内容伪造 `__PRINCIPAL_RELAY__` 哨兵劫持转述模式（绕过发送守卫、让 AI 转述假信息）。决策：身份判定改 is_synthetic_relay 结构标记（非持久化、不可反序列化，只能由可信构造函数设置）；即便伪造哨兵也在进 LLM 前剥离。理由：**身份必须基于来源（provenance）而非可伪造内容**。影响：models.rs ConversationMessage、webhooks 分流、decision prompt 组装。

**22) 2026-06-30 h3-cross-tenant-idor-fix** — 背景：14 个 admin handler 接受请求体 workspaceId 未校验归属。决策：中央 resolve_authorized_workspace（请求 workspaceId 回落 current_workspace 后过 AdminUser.workspaces ACL）。影响：llm_providers/domain_schemas/domain_profiles 等 14 handler。后续 07-14 安全审计确认此收口仍守住。

**23) 2026-06-30 上线前全量业务测试方法论** — 决策：四级正确性层级（红线/设计意图/主观质量/孤儿行为）+两阶段饱和审计（锚点枚举→深读对抗验证）。价值：为 07-10/07-11 两轮全系统测试与六批审计提供方法论底座。

**24) 2026-07-09 webhook-gewe-addmsg-parse-fix** — 背景：真实客户消息永不触发 AI 回复（生产核心链路断裂）。根因：GeWe 嵌套 payload（`Data.FromUserName.{string}` 包裹）被通用 find_string 递归遮蔽——顶层账号自己的 Wxid 抢先命中。决策：GeWe 显式路径优先+回落兼容。深层教训：**62 个 biz-test 全绿因从未用真实嵌套形态——测试语料的形态盲区与假绿不同但同样致命**。影响：webhooks.rs 解析层。

**25) 2026-07-09 webhook-signature-verify-restore** — 背景：联调期关闭验签=公网无鉴权入口；且两端签名方案已不匹配（旧全局 key 裸 hex vs 新每账号密钥 timestamp.body）。决策：方案 B 每账号 webhook_secret+时间戳防重放+fail-closed（漏配密钥=400 拒绝）；部署顺序先部署再翻开关否则消息流中断。影响：webhooks.rs 验签、accounts 密钥管理。

**26) 2026-07-10 outbox-chat-search-idempotency** — 背景：客户真实收到 3 条重复占位。根因：150s timeout 取消 send future→mcp_call_logs 没写成→本地兜底核对查空→误判没发→重发。决策：核对源升级为 MCP chat_search（服务端真实已发记录）+content 精确匹配+失败回落本地。理由：崩溃/超时时本地日志恰最不可靠，必须以对端事实为准；残留风险方向偏"重发"（可挽回）而非"漏发"。影响：outbox_dispatcher.rs、mcp.rs。

**27) 2026-07-10 roster-single-flight-refresh** — 背景：通讯录永远 syncing 卡死。根因链：8s force 轮询×无去重 spawn→打爆 SSE 20 并发上限→1.37MB body 读超时→互相中断→快照永远写不进。决策：DashMap single-flight 锁（RAII guard panic 也释放）+前端只读轮询+同步端点不阻塞。价值：修正前一 PR"重复 spawn 无害"的被低估假设——并发资源抢占也是副作用。

**28) 2026-07-11 deep-logic-audit（design+findings）** — 背景：19 频道×5 维广度走查扫不到"没有页面的后端逻辑"。决策：从频道入口穿透到底的全链路审查，5 批 53 findings（0C/1H/24M/28L），subagent 结论必主控亲验才入账。元家族定型："设计声称的不变量，实现层有旁路/缺口/非原子窗口/新旧不对称"。此台账是后续 P0-P3 修复波（#180~#205 等十余 PR）的唯一权威来源，findings 文档内含修复状态更新。

**29) 2026-07-12 kd-family-relay-number-guard** — 背景：relay 数字护栏既漏真幻觉（中文"八折"提取为空）又误杀正确转述（"24小时"）致裁决黑洞。决策（用户裁定）：整体删除 gateway fail-closed 用法，忠实度交还 prompt+独立 Review（reviewer 本就同时看到授权 substance 与拟发转述）。设计从初版 3-4 支柱收敛为单一删除动作。**全集内"字符 backstop 做不了语义判断"论证最完整的一篇**，其威胁模型分析（grounding 可确定性因为是集合运算）被后续多篇引用。

**30) 2026-07-13 revision-fallback-to-approved-draft** — 背景：生产实证约 20% 本可成功的回复被改写超时毙掉（客户反复收同一句兜底）。设计基石：能进改写通道的原稿 finalize 必已 Approved（硬闸失败根本进不了改写，改写只由软闸触发）——故三个失败分支统一回退发送已批准原稿。理由：改写是锦上添花，失败不应劣化为"什么都不发"。影响：gateway.rs revision 分支、review/gates。

**31) 2026-07-14 manual-ops-enrollment-source-of-truth** — 背景：AI 给公众号/电台/账号自己机械回复（骚扰+死循环风险）。核心决策：**放弃用自动判据识别"真人"**，以管理员手动加入运营为唯一真相；系统只拦硬事实（gh_/@chatroom/@openim）与逻辑铁律（is_self_account，与真人判据解耦）；hide_from_pool 联动改 agent_status（写入侧单一真相源而非读取侧加过滤）。理由：wxid 字符特征判营销号必误伤真人；显式动作可审计。影响：webhooks.rs 判据、contacts.rs 三个 enable 入口、审计事件、存量清理脚本。

**32) 2026-07-14 principal-authorization-exemption** — 背景：领导 approved 的产品说法被产品门二次拦截（生产实证客户永远收兜底）。决策：A 类客户级豁免（长期常驻可撤销）+B 类沉淀 verified 知识（新 provenance PrincipalAuthorized，B 是 A 的超集）；红线定性（用户拍板）「领导裁决=人工验证」——验证主体是真人，"AI 永不自动验证"本质未破；B 类必须走 apply_chunk_revision 直写绕开 LLM 自评降级链。影响：escalation/mod、gateway relay、review/gates R5.4、chunk_revisions provenance 闭集、contacts 撤销端点。

**33) 2026-07-14~15 六批审计工程+交叉验证（agent-capabilities 20 / auth-routes 17 / worker-fleet 18 / knowledge-wiki 32 / evolution 11 / db-migrations 3 = 101 findings；audit-cross-verification）** — 方法论集大成：分簇（根因层先审出基准喂资源域簇）、部署拓扑校准（单租户/单进程不可达=就绪债）、主控亲验驳回夸大（knowledge-wiki 批 subagent 自评 3H/10M 被全部降级）、对抗式交叉验证（18 条 High/Medium 独立证伪，100% 属实、4 条单向高估、唯一 High 复核后反而更严重——claimed_at null-present 使重启也不回收）。全局 P0=initial_profile 无终态写入（批量托管核心路径每条确定性命中：3×LLM 浪费+旧初始画像覆写窗口内累积更新+误判 failed）。修复批 07-15 audit-medium-remediation 分 3 PR 落地并留 A-02 取舍冲突记录（交叉验证降 Low、用户裁定保留修）。

## 4. 事实卡速查

### 4.1 跨 spec 红线条款汇总表

| 红线 | 内容 | 主要出处 | 例外/边界 |
|------|------|----------|-----------|
| 无人工接管 | 客户永远只跟 AI 对话；字符级 CI lint（check-no-human-takeover）扫 src/agent、src/routes、src/evolution、frontend/src 新增行禁词 | CLAUDE.md；几乎每篇修复 spec 的"lint 合规"节 | ①幕后领导模式（请示≠转人工，2026-06-05）；②辅助模式引荐名片（账号级显式开启，2026-06-21）；③tests 目录豁免 |
| 客户永不知道有领导 | relay 转述用 AI 自己口吻，绝不透传内部概念 | 2026-06-05；KD-02（2026-07-12） | KD-02 裁决：不加字符词表，由 prompt+独立 Review 语义保障 |
| AI 永不自动 verify 知识 | 一切 AI/摄取路径落库强制 draft+needs_review；auto-verify 只做预审分诊（verified 强制降级 needs_human_audit） | 2026-05-25 起贯穿；07-13 wiki-playwright 前置亲验；07-15 wiki-audit 正向 HOLDS | PrincipalAuthorized（2026-07-14）：领导裁决=人工验证，走 apply_chunk_revision 直落 verified——验证主体是真人，红线本质未破 |
| 产品声明须 verified 背书 | verified_chunks 为空且非目录报价→blocked_unverified_product_claim（R5.4 硬门） | autonomy-loop spec；gates.rs | ①priced_from_catalog 旁路；②principal_product_exempted 旁路（07-14）；③claim_analysis 缺失按"非产品声明"放行=2026-05-25 显式接受取舍（audit D-08 WontFix） |
| D2 锚定门 | verified 必须 source_quote 非空+source_anchors 可锚定 | 2026-06-06；verify.rs | 手工路径违规降级 needs_review 而非 400 |
| 双层标签铁律 | manual_tags 权威/confirmed_tags AI 确信/tag_observation 待定三层物理隔离；证据 fail-closed；bayesian/personality 只写不进决策；未审候选不阻断运行 | 2026-06-23 tag-trust；07-14 audit 复核 8 铁律 HOLDS | A-02 修复（07-15）：confirmed_tags 增"保留 unless discardedTags 显式弃用"合并 |
| 统计只观测不进决策 | gap_signals/structural_proposals/lessons_learned/reviewer_stats/behavior_signals 全仓决策链零读取 | 07-14/07-15 各审计批正向 HOLDS | — |
| 每次发送必经统一网关 | webhook 回复与 follow_up 同走 run_user_operation_gateway→outbox→MCP；campaign 也经 follow_up task 汇入 | CLAUDE.md；2026-06-28 campaign-targeted-push | 请示卡推给领导直接 logged_call_for_account 不走 outbox（不面向客户，2026-06-05 plan） |
| Outbox 幂等+二次安全门 | approved 决策必先入 outbox 带幂等键；发送前 second_safety_gate fresh 复核（07-14 起含 is_managed） | autonomy-loop；07-14 p3-family9 | 防重发核对权威源=chat_search（07-10/07-14） |
| evolution 隔离红线 | evolution/ 禁引 gateway/outbox/mcp/tasks/webhooks；CI 静态扫描 | evolution/mod.rs；07-15 audit HOLDS | lint 是子串匹配，grouped import 理论可绕（S-03 Low，当前无实例） |
| R9.7 禁自动回滚 | 演化只允许 threshold 自动放量（双闸默认关）；rollback 永远 admin 手工；prompt 绝不自动放量 | 2026-06-27；07-15 evolution-audit HOLDS | — |
| 状态机 fail-soft | 非法 operation_state 转移不拦回复（已发出），只跳过写+审计事件 | CLAUDE.md；m004/m013 | — |
| RunBudget | 超预算返 BudgetExceeded 网关降级（local review/跳过 rewrite），绝不 5xx 给 webhook | CLAUDE.md；07-06 escalated-run-budget | 升档 run 独立更高 gating 上限，token 计数保持诚实 |
| DEFAULT 字节等价 | 一切通用化改造 DEFAULT 销售域行为逐字/字节等价，快照测试锁死 | 2026-06-11 起所有 universalization spec | — |
| 反过拟合 | 绝不为让测试/样本通过而改业务逻辑/prompt/阈值/词表；审计只审不修 | 2026-06-06 probe 三焊缝；此后所有审计/修复 spec | — |
| agent-first | 语义判断交 LLM，不引关键词词表 | tag-trust plan Global Constraints；2.6 全线 | 确定性代码闸只用于客观集合运算（如 grounding 的 verified 集合判定、载荷标记存在性） |
| coreFacts 向后兼容 | 必须继续反序列化 legacy Vec<String>（R11） | CLAUDE.md；m002 | — |
| 磁盘纪律 | 本地只跑 --lib+单 PBT；集成测试交 CI；紧时先删 target/debug/incremental | CLAUDE.md；多篇 plan 验证门 | — |
| no-model-hint | 新增行禁模型/品牌名字面量 | 07-14 系列起频繁出现的第三 lint | — |
| 先读懂再动手 | 100% 读懂+file:line 亲验才改码；subagent 产出必主控亲验 | CLAUDE.md 最高红线；所有审计 spec 方法论节 | — |

### 4.2 spec 间明确的矛盾/取代关系表

| 先 | 后 | 关系与理由 |
|----|----|------------|
| 2026-05-25 cleanup："5 闸→3 闸" | 2026-06-04 起多篇仍说"5 闸/五闸" | **语义换位非矛盾**：删除的是字符串级守卫闸；此后"5 闸"指 review 五个分数阈值（factRisk/pressureRisk/humanLike/emotionalValue/productAccuracy）。2026-06-05 plan 锚点段是权威澄清 |
| 2026-06-05 spec 请示走 Held 挂起（初稿描述） | 2026-06-06 用户拍板"统一走安全占位" | plan 核心模型段显式记录改判：请示 run 照常 Approved+占位句，不进 Held、不碰 review.rs |
| 2026-06-18 universal-business-gaps"五闸可配" | 同篇实证裁决 | 自我否决："五闸可配=过度设计（砍）、driver 框架=高风险低收益（缓）" |
| 2026-06-28 customer-reply-guarantee：占位一律硬编码 | 2026-07-10 passive-reply-daily-limit-and-ai-holding-reply | **部分推翻**：硬编码死文案破坏拟人→改 AI 生成（独立小预算+出站禁词守卫+硬编码仅作降级兜底） |
| daily_limit 无差别拦被动回复（原始设计） | 2026-07-10 同篇 | 收窄为仅 FollowUp——客户主动提问不应被日限拦 |
| escalate_held_decision 硬编码 is_generalizable=false | 2026-07-14 principal-authorization-exemption | 放开：由领导裁决 exemption_type=knowledge 驱动，使最典型 B 类场景（high_risk_gated 产品问题）可沉淀 |
| relay 数字护栏（作为代码 backstop 引入） | 2026-07-12 kd-family-relay-number-guard | **整体删除** gateway fail-closed 用法；holding_reply 调用点保留（语义正确） |
| 台账 KD-02 修复建议：加"领导泄漏"词表 | 2026-07-12 kd-family2 用户裁决 | 不修——字符词表是威胁模型错误 backstop，与删数字护栏同理 |
| pack repair（operation_knowledge_items 时代） | 2026-06-28 e4 重定义→2026-07-13 p3-family1 删桩 | items 集合已删（05-25）→"pack repair"死桩恒 400→重定义为文档级批量修复+摘除死路由 |
| 2026-06-23 tool-loop-dead-code-sunset | — | tool_loop.rs 及未接线 runtime 参数整体退役（从未接生产） |
| run_envelope 模块 doc："W1 task 2.5 会接线" | 2026-07-13 p3-family1（H-02） | doc 改标注"R0 三函数生产未接线/推迟"；保留基建+集成测（用户裁定不删不接线） |
| 词表硬门 HANDOFF_MARKERS（评测侧） | 2026-06-19 evaluation-system-overhaul | 下线，换多采样跨家族中位数 LLM 硬门 |
| 台账 A-02 定级 Medium（偏 High 最值得修） | 2026-07-15 audit-cross-verification 降 Low | **两份产物冲突留痕**；2026-07-15 audit-medium-remediation 用户裁定保留修全六条 |
| 台账 KC-05 初判修法（DB filter 加 $or） | 2026-07-13 p3-family3 KE-03 同型情况 | 多次出现"台账初判修法被设计阶段更优落点取代"（KE-03 最优落点=纯函数而非 filter；KD-07 台账首选①被证会永久卡住改用③） |
| roster 解析器候选路径设计 | 2026-07-08 roster-fetch-cache-shape | "解析器从未跑通过真实数据"——设计时假设的返回形态与真实 {result:{friends:[字符串]}} 不符 |
| PR#162 假设"重复 spawn 无害" | 2026-07-10 roster-single-flight-refresh | 修正被低估假设：并发抢 SSE 名额也是副作用 |
| threshold/significance 的 gate↔status 映射 | 2026-07-15 evolution-audit [1-01]/[2-01] | 三文件三向不一致且都与生产真相不符；分野=合成闭环自洽（Low）vs post_release 读生产真实 status（Medium）；07-15 remediation PR-E 统一权威常量+pressure 改 revision 口径 |
| m011"legacy 集合清理" | 2026-07-15 db-audit [C-01] | 集合名与当前 wiki 存活集合**同名同用**——"legacy 清理"语义已过时，破坏性押 APP_ENV+已入账双条件 |
| EVOLUTION_ENABLED 注释"安装态默认 false" | 07-15 evolution-audit [S-02] | 真实默认 true（config.rs:7+测试锁死），:212 注释 stale 且同段自相矛盾 |
| referral-cards 的 roster 加载（未处理 syncing） | 2026-08-06 decider-chain-roster-picker | 显式标注"不抄 referral-cards 抄 RosterView"；同类缺陷留待另修 |

## 5. 偏差与疑点

本节对照任务给定的三个已知代码事实，并列出深读中发现的其余疑点。本任务未对码亲验，无法对码处一律标"文档声称"。

### 5.1 对照点核对

**① "5 闸已改分数闸"——文档链自洽，与已知事实一致。**
- 2026-05-25 cleanup 记录删字符串守卫、5 闸→3 闸；2026-06-05 plan 锚点段明文"旧五闸门已删除，当前是 knowledge_grounding/hallucination/run_budget 三闸门+review 评分体系"。
- 此后所有出现"5 闸/five_gate"的 spec（06-20 G13 五闸阈值 clamp、06-27/06-28 prompt-evolution 的 5 闸命中率、07-12 ke-family1、07-15 evolution-audit/remediation PR-E）语境全部是**分数阈值体系**（fact_risk_block/pressure_risk_block/human_like_rewrite/emotional_value_rewrite/product_accuracy_block）。无一篇在 05-25 之后仍把"5 闸"当字符串守卫使用。
- 细节补充：分数闸内部再分硬/软——文档声称 fact_risk（hallucination_score）与 product_accuracy 走硬 block、pressure_risk/humanLike/emotionalValue 走软闸 revision（07-13 revision-fallback 与 07-15 evolution-audit [1-01] 的生产真相亲验段一致）。evolution 侧曾因映射漂移把 pressure_risk 映射到它生产永不产生的 block status（07-15 remediation 修正）。

**② "user.reply.task 已退役"——⚠️ 与文档存在时间性矛盾，specs 无退役记录。**
- 2026-07-13 reply-prompt-slimming 仍以 `user.reply.task` 为生产主 prompt（45-149s 慢诊断、39-43k tokens），并设计了以它为对象的三批次 A/B 瘦身（批 2/3 需分批灰度验证）。
- **全部 165 篇 specs（截至 2026-08-06）无任何一篇记录 user.reply.task 退役**。若该 prompt key 现已退役，则退役发生在 2026-07-13 之后且未落 spec（可能在 07-15~08-05 的约 20 天文档空窗期内）；reply-prompt-slimming 的批次 2/3 灰度方案是否走完、或随退役一并作废，文档不可考。此为**文档-代码最大的已知漂移点**，读该 spec 时须知其对象可能已不存在。
- 关联疑点：07-13 spec 内提到的止血措施（runTokenBudget 30000→300000 等 DB 热更）是否随退役回调，同样不可考。

**③ "销售域守卫 2026-05-25 删除"——文档链完全一致。**
- cleanup spec 本身+baseline PBT 替换记录+07-14 audit D-08（gates.rs:654-657 注释逐字引用"2026-05-25 知识库清理删除 chunk.safe_claims/ProductClaimMarkers……claim_analysis 缺失时按非产品声明放行"被主控亲验属实）三方互证。该删除带来的 fail-open 被显式定性为"已裁决接受取舍"而非缺陷（WontFix，不重开）。

### 5.2 其余偏差与疑点（文档内部或文档-现实）

1. **07-15 → 08-05 约 20 天 spec 空窗**：165 篇的时间分布在 2026-07-15 后突然中断，直到 08-05 才恢复且转为纯前端小修。空窗期的工作（若有）未以 spec 形式记录；user.reply.task 退役等重大变更可能发生在此窗口。
2. **full-business-logic-test-findings（06-26）只有一条 finding**（import-preview LLM 503）即止——与 design 篇宏大的 7 域计划不成比例。后续 07-10 full-system-deep-test 才是真正执行完的版本（另有 07-10 full-system-test-findings 台账被 07-11 remediation plan 引用，但**该 findings 文件不在 specs 目录 165 篇之列**——文档声称存在 `2026-07-10-full-system-test-findings.md`，实际目录里没有，疑似未落盘或被并入 deep-logic-audit）。
3. **行号漂移是系统性限制**：07-15 audit-medium-remediation 明说"审计写作时的旧行号多已漂移，本文档用亲验后的当前行号"。全集内所有 file:line 引用都是写作时点快照，今日对码须重新亲验（这也是 CLAUDE.md"引用必亲验"红线的由来）。
4. **plans 与 specs 的非一一对应**：149 plans vs 165 specs。审计 findings 类不需要 plan；反向存在**无 spec 的 plan**（如 2026-06-15-h17-memory-dimensions-universal.md——universal 总纲 H17 项直接落 plan；2026-07-11-full-system-test-remediation.md——对应的是上述缺失的 findings 台账；contract-alignment batch1-5、tag-trust 1-5、deep-logic-audit batch a-e 为一 spec 多 plan）。
5. **文档声称的"已修/Fixed"状态未对码复核**：deep-logic-audit-findings 自称"唯一权威台账（含修复状态更新）"，多篇修复 spec 承诺翻正状态（如 f01 篇批量翻正 6 条）。这些状态流转是文档内部自洽的，但本任务未对码确认每条 Fixed 是否真实落地（PR 号 #178~#238 存在于文档叙述中）。
6. **EVOLUTION_ENABLED 默认值张力**（07-15 [S-02]）：config.rs:7 默认 true、:212 注释称 false、同段 :215 又称 true——文档内部自相矛盾（已定性 stale 注释），提醒读 config 注释不可尽信。
7. **[S-01] runtime_flag=None 语义分叉**（07-15）：mod.rs 注释与 cohort 实现相反（"全员排除" vs "全量收"）——评估"演化默认行为"时文档注释不可作为依据。
8. **m011 与存活集合同名**（07-15 [C-01]）：文档声称其安全性取决于生产 117 的 migrations 入账状态+APP_ENV 真值，台账显式标"待生产实证"——截至 specs 记录终点该实证未见回填。
9. **worker-fleet [1-01] 的修复（PR#216）与 S-01/S-02（PR#217）**在 audit-medium-remediation 开篇被声称已完成——但对应的修复设计没有独立 spec（直接走 plan/PR），specs 目录检索不到，属"文档声称"。
10. **知识 Agent 反接管批次 2/3**（数据清洗、红线注入机制架构统一）在 07-14 spec 里明确"留待后续批次"，其后无任何 spec 跟进——未闭环项。
11. **reply-prompt-slimming 批次 2/3、prompt-evolution phase3 之后的 phase4+、ask-human-inbox 的 Task 12（卡片 CSS）、structural_proposals apply 接线（KB-06）、B-01 gen 撤销补偿专项**——文档内明示"待后续/待专项"且全集内无后续记录的未闭环清单。
12. **基线数字漂移**：CLAUDE.md 与各 plan 说 lib 基线 ≥350，07-14 knowledge-agent-autonomy-redline 提到"当前 1974"——两者是下限 vs 实测的关系，非矛盾，但读文档时易误解 350 为实际规模。

## 6. 覆盖自证

- **specs：165/165 全部逐篇读完**（`ls docs/superpowers/specs/ | wc -l` = 165，含 2 篇非 -design 后缀的 findings/报告：2026-07-15-audit-cross-verification.md 与各 audit-findings）。编目见第 1 节 165 行清单，逐篇均有一句话以上提要；篇幅长者（universal-domain-adaptation、deep-logic-audit-findings、六批审计 findings、alignment-fixes 等）读完全文。
- **plans：149 篇全部列清单**（`ls docs/superpowers/plans/ | wc -l` = 149）；抽读计划头部 5 篇：principal-decision-channel（2571 行，含锚点漂移修正与 6 项拍板决策）、h17-memory-dimensions-universal（无对应 spec 的 plan 样本）、full-system-test-remediation（无对应 spec 在目录的 plan 样本）、tag-trust-1-data-model（一 spec 五 plan 样本）、（另在既往会话中通读过 referral-card-push/campaign 等篇对应计划的引用段）。plans 的通用形态：Goal/Architecture/Global Constraints（基线门+lint 门+磁盘纪律）+主控已亲验的"关键事实"节+task-by-task checkbox。
- **未读完的部分：无**。specs 165 篇无遗漏；plans 按任务要求仅做清单+抽样，未逐篇通读（144 篇 plan 正文未读，属任务定义内的有意省略）。
