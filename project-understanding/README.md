# 项目深读记录目录（100% 理解工程）

> 目标：对整个仓库做逐行级深读，每个领域一份带 file:line 证据的记录文件。
> 总台账（全局结论+偏差清单）见根目录 `PROJECT_UNDERSTANDING_LEDGER.md`。
> 约定：所有记录不许猜测——读不懂的标注"疑点"；每个关键断言附 file:line；每份记录末尾附覆盖自证（读过的文件+行数清单）。

## 终局统计（2026-08-13 完成）

- **19/19 份记录全部完成**。覆盖：后端 src 约 19.3 万行全读（12 份记录合计与 src 总行数 192,582 吻合）；tests 182/182 文件全读；前端 core+features+review 37,405 行全读；.kiro specs 5 个三件套全读；docs 顶层 26 篇全读；superpowers specs 165/165 全读；scripts/CI 全读；未提交 47 文件逐 hunk 全读。
- **主会话质检**：15+ 处关键锚点亲读抽查；1 次记录间冲突裁决（17 号误报已回写修正）；1 个前端疑点被后端代码排除（13 号）；11 项缺陷/矛盾经主会话亲证为真（清单见总台账）。
- **残余未逐行区**：frontend/src/__tests__ 部分文件体（结构与覆盖面已录）、docs/system-review 26 文件仅头部+结论、docs/superpowers/plans 149 篇仅清单+抽读、website/ 营销站。以上均为低风险区，已知边界。

## 第二轮终局统计（2026-08-13 完成）

- **12/12 任务全部完成**（20 号拆分为 20a/20b）。
- **疑点终裁**：01-19 号全部 236 条疑点（73+163）逐条对源码终裁——72 条实锤缺陷、22 条不成立、128 条刻意设计、7 条诚实存疑；22/23 号为终裁唯一权威，各记录疑点节原始表述以其为准。
- **交叉验证**：五路（主链路/知识域/数据层/前后端契约/测试-生产）锚点抽验通过率 96.7%-100%（累计 200+ 锚点）；记录间矛盾全部裁决并回写（含对主会话既有结论的一次重大翻案：台账缺陷 #1 降级）。
- **新发现运行时缺陷**：referrers 恒 400、products 过滤静默失效（27 号）、authorizationExpiresAt 前端崩溃（23 号）、演化器 pressure gate 统计源失真（23 号）。
- **汇编产出**：30 号全局事实卡手册（改动前首查）、29 号 71 条偏差总表（读文档前首查）、28 号 §4 十九条无测试守护清单（改动风险地图）。
- **方法论教训（两条，已固化）**：① 判断"是否在生产使用"必须验证调用点，spec/测试/种子存在性不是证据（17 号误报）；② 缺陷判断必须同时验证分支内行为与分支可达性（缺陷 #1 翻案）。

## 第二轮：交叉验证与终裁（2026-08-13 启动）

| # | 文件 | 任务 | 状态 |
|---|---|---|---|
| 20 | 20a-plans.md / 20b-system-review-smoke.md | 补盲区：superpowers plans 149 篇全文 + system-review 26 文件全文 + docs/smoke（原 20 号被平台内容防护误拦后拆分重发） | ✅ 全部完成。20a：149/149 篇（checkbox 不可作完成信号——完成状态靠五类间接证据链；95 条仅存于 plan 的实现决策；6 个矛盾点全部归档——4 个已被前期核证覆盖、PROMPT_PACK_VERSION 主会话亲证为字符串+仅溯源语义、知识缺口计数口径留待前端复查）。20b：31 文件 8,187 行（遗留三态判定：21 已解决/约 24 组开放——HC-001 凭证轮换最重；07-26 全量上线事实厘清） |
| 21 | 21-frontend-tests-website.md | 补盲区：frontend/__tests__ 全部文件体 + website/ | ✅ 已完成（138/138 测试文件 16,179 行 + website 13 文件全读）：契约测试无空壳、与 13/14 号零矛盾；营销站 4 处事实错误（emotional_value 5.0 应为 6.0、debounce 4s 应为 2s 等）+数字全面低估过时；三处前端测试薄弱点（navigationStore/sendAnalyticsStore/AutoVerify 下限） |
| 22 | 22-verdicts-core.md | 疑点终裁 I：01-06 号记录全部疑点逐条对源码终裁（属实/不成立/需产品决策） | ✅ 已完成：73 条全终裁（20 缺陷/5 不成立/48 设计，6 条诚实标"仍存疑"）；**重大翻案：台账缺陷 #1 主路径不成立**（deferred kind 全仓无创建点系 legacy，主会话二次亲证并已修正台账+01 号）；#5 精化为 fail-safe 死代码。**终裁权威声明：各记录疑点节的原始表述一律以 22/23 号终裁为准** |
| 23 | 23-verdicts-extended.md | 疑点终裁 II：07-19 号记录全部疑点逐条终裁 | ✅ 已完成：163 条全终裁（52 实锤缺陷/17 不成立/80 刻意设计/1 存疑/13 已核证跳过）；重量级：演化器 pressure gate 统计源失真（主会话亲验 threshold.rs:69 vs gates.rs）、authorizationExpiresAt 前端崩溃路径、17 号 Q7-Q10 四条历史疑点被后续修复反证不成立；14 条回写待与 22 号合并执行 |
| 24 | 24-crosscheck-mainchain.md | 交叉验证：01/03/04/05/06 主链路记录接口一致性 + 锚点抽验 | ✅ 已完成：75 锚点 98.7% 通过、7 接口 6 一致；1 实质矛盾裁决（05 号"relay 数字白名单双守卫"错——已删除仅剩泄漏守卫，主会话亲证 gateway.rs:4188-4192 并回写）；01/04/06 评为"可直接作改动依据"级 |
| 25 | 25-crosscheck-knowledge.md | 交叉验证：07/08/09 知识域记录一致性 + 锚点抽验 | ✅ 已完成：真矛盾 0、61 锚点 96.7%；**红线落点合并总表 19 处逐处亲验**（★Imported+Create 无 harness 兜底、依赖 import.rs 两处显式赋值）；顺带关闭 08 号 3 个疑点、确认 1 个；回写已执行 |
| 26 | 26-crosscheck-data.md | 交叉验证：02 数据层记录与全部引用方记录一致性 | ✅ 已完成：25/25 锚点通过、raw 字段拼写三方全一致；揪出 02 号 1 处事实错（active_task_key 归属 agent_tasks 非 operating_memories，已原地回写）+3 处注释滞后失真（已追加修正节）+11 号 1 处失真（skipped_duplicate 非值域，已回写）；02 号使用纪律：闭集可直接引用、注释级数值须复核实现 |
| 27 | 27-crosscheck-api-contract.md | 交叉验证：前端(13/14) vs 路由(11/12) vs 源码 端点三方对账 | ✅ 已完成：272 端点 vs 239 前端调用精确闭合（幽灵 0/方法不匹配 0/33 孤儿逐个归类含 9 个零消费功能缺口）；**发现 2 个全档案漏检的运行时缺陷**（referrers 恒 400、products 过滤静默失效——query 参数 camelCase 错配，主会话双侧亲验，已入台账 #14/#15）；11 号计数修正已回写 |
| 28 | 28-crosscheck-tests-vs-prod.md | 交叉验证：测试记录(15/16)不变量 vs 生产记录(01-10)行为 | ✅ 已完成：71 承诺全映射（64 一致/2 弱化/1 矛盾/1 无守护）；2 起复刻漂移实锤+1 幻影值（主会话抽证 reassign $unset 属实）；产出 19 条"无测试守护清单"（28 号 §4，改动前必查）；回写已执行（15/16/10 号+总台账） |
| 29 | 29-doc-code-divergence-master.md | 全局文档-代码偏差总表（合并去重+复核） | ✅ 已完成：71 条编号偏差（高 4/中 32/低 35）每条当日双侧亲验；7 条已消失偏差 +2 条新发现（CLAUDE.md 行号漂移、gateway.rs:2915 退役 key 注释残留）。**读任何本仓文档前先查此表的反向索引** |
| 30 | 30-global-fact-cards.md | 全局事实卡手册（闭集/阈值/幂等键/flag/端点/worker 单一速查） | ✅ 已完成（541 行八类齐备：闭集 75/阈值 160/幂等 17+40/flag 20/prompt 36+4/worker 16/红线 30 落点/集合矩阵 79）；裁决记录间数字口径冲突 8 处（gateway_status=39 非 40、source_kind 分集合口径等）；每类 ≥5 条抽验源码全过。**改动前首查此手册** |

## 记录清单与状态

| # | 文件 | 覆盖范围 | 状态 |
|---|---|---|---|
| 01 | 01-agent-gateway.md | src/agent/gateway.rs（9152 行逐段） | ✅ 已完成+主会话抽查通过（发现真实缺陷：醒来任务被 daily_limit 取消而非重排，锚点 2380-2394/5263-5296/5659-5662 三处亲证） |
| 02 | 02-models-db.md | src/models.rs（8352 行逐字段）、src/db/**（含 58 个迁移逐个） | ✅ 已完成（20,881 行全读）+主会话抽查通过（幽灵迁移引用 models.rs:1802 亲证属实；BSON 命名三分/raw Document 索引字段/迁移两代风格为关键结构性事实） |
| 03 | 03-webhooks-tasks.md | src/webhooks.rs、src/tasks.rs | ✅ 已完成+主会话抽查通过（确认 D2 毒丸缺陷：坏消息行可静默瘫痪两个 worker 的 tick，webhooks.rs:819-822 + tasks.rs:1069,1121 亲证；Err 非 panic，supervisor 熔断不触发） |
| 04 | 04-decision-review-guards.md | decision.rs、review/、guards.rs、types.rs、runtime.rs、taxonomy 双层、domain 系列 | ✅ 已完成（23,814 行全读）+主会话抽查通过（确认双脑 parse 失败拉闸与注释矛盾，review/mod.rs:4390 vs 4409-4415 亲证） |
| 05 | 05-outbox-escalation-send.md | outbox.rs、outbox_dispatcher.rs、escalation/、referral.rs、media_send.rs、send_ledger.rs、pacing.rs、quiet_hours.rs | ✅ 已完成（11,672 行全读）+主会话抽查通过（确认 manual_send 两道门裁决矛盾：dispatcher:922-928 二次门无豁免 vs 2741-2748 下游门有豁免+注释意图相反；另有 hold 请示被骚扰门拦时零台账、delivery_unknown 请示卡不进超时改派两条待重验疑点） |
| 06 | 06-memory-reaction-runtime.md | memory.rs、reaction.rs、sufficiency.rs、post_decision.rs 及运行支撑模块 | ✅ 已完成（15,597 行全读；疑点：run_envelope 终态无 lifecycle CAS、deprecatedFacts 两处 cap 不一致 6 vs 20、discard_projection 窄竞态——未逐条抽查，引用前重验） |
| 07 | 07-knowledge-engine.md | knowledge_agent(.rs+/)、knowledge_router.rs、knowledge_tools.rs、chat_tool_loop.rs、knowledge_wiki/** | ✅ 已完成（15,260 行全读）+主会话抽查通过（确认窗口错位缺陷：router corpus 200 条静态序 vs agent catalog 400 条相关度序，合法引用可被求交过滤丢弃，knowledge_router.rs:74-78,752-762 + knowledge_agent.rs:81 三处亲证；另 DocEntry 未落地、structural_proposals 无消费方） |
| 08 | 08-knowledge-routes-workers.md | routes/knowledge/**、knowledge_task/、knowledge_digest/、import_worker.rs | ✅ 已完成（20,862 行全读）+主会话抽查通过（红线 15 处落点正面核证；确认 4 处裸 !is_empty() 锚点口径与 B3 统一函数不一致，crud.rs:547 亲证——报表漏报级；knowledge_task execute_step 疑似死路径待重验） |
| 09 | 09-llm-mcp-infra-prompts.md | llm.rs、llm_concurrency.rs、mcp.rs、config.rs、prompts.rs 全文、prompt_guard、版本模块、supervisor、secret、error、media_storage、outbound_fetch | ✅ 已完成（12,898 行全读）+主会话解决其疑点 2：ClaimGate prompt 为代码内嵌常量（review/mod.rs:340-354），有意游离于 prompt 治理/演化体系之外 |
| 10 | 10-evolution-workers.md | evolution/** 全部、planner/、proactive_outreach、cold_contact、silence_signal、management_worker、account_scheduler、behavior_signals、bin/ | ✅ 已完成（23 文件 14,827 行全读；疑点：prompt shadow LLM 消耗不计入 EvolutionBudget、rewrite 闸口径两处差 2 倍、schedule_post_release_review 与 is_evolution_enabled_for 无调用方、release cooldown 不排除已回滚——中级未抽查） |
| 11 | 11-routes-business.md | routes 业务面（contacts/management/campaigns/guides/evaluations 等） | ✅ 已完成（26 文件 22,995 行全读；疑点：guide apply OCC 基线漏检 manual_tags 变更、reviews 列表 N+1、guide preview readableChanges 丢弃 LLM 话术——中轻级未抽查） |
| 12 | 12-routes-admin-auth.md | routes 管理面（admin_*/observability/llm_providers 等）+ src/auth/** | ✅ 已完成（19,903 行全读）+主会话抽查通过（确认 evolution.rs:742-746 updated_by 取请求体可伪造审计身份——手边有 Extension(admin) 却未用；另经此份记录+后端亲证排除了 13 号的丢草稿疑点） |
| 13 | 13-frontend-core.md | frontend：app/、stores/、contracts/、lib/、types/、入口 | ✅ 已完成（8,319 行全读；疑点：3 个后端 fixture 前端零消费、openEventSource 与 wa.authed 死代码、DomainProfileDraft 缺 generated_state_machine 疑丢 AI 状态机草稿（需对后端 update 逻辑核实）、部分写操作未走 detailActionIsCurrent 防护——未抽查） |
| 14 | 14-frontend-features.md | frontend：features/** 全部、components/ | ✅ 已完成（79 文件 29,086 行全读；红线在前端 ≥6 处独立落地；4 处 dead code 已 grep 核实；auto-verify 双入口口径不一致、runtime 参数字段表双份漂移风险、多处未接线 UI——中轻级未抽查） |
| 15 | 15-tests-agent.md | tests/：agent/gateway/outbox/memory/reaction/escalation/campaign 主题 | ✅ 已完成（71 文件约 290 测试函数全读，61 条系统承诺总表）+主会话抽查通过（确认两个空壳测试永远绿：revision_recheck_action_gate 零断言、memory_card_write_occ 仅启动容器——两条安全不变量无可执行守护，亲读全文证实；另 worker_reclaim 名不副实、约 6 处复刻式测试与生产脱节风险） |
| 16 | 16-tests-knowledge-infra.md | tests/：knowledge/evolution/auth/迁移/real_llm/roleplay 主题 + common/ | ✅ 已完成（124 文件全读，与 15 号合计覆盖 tests/ 全部 182 文件）+主会话抽查通过（确认 ingest 正向拉取零集成覆盖，四测试全为拒绝路径亲证；workspace 隔离多为 filter 形状级、真 Router 证据仅 SR-176/h3；mock LLM 按 prompt 锚文本路由是隐式契约） |
| 17 | 17-kiro-specs-docs.md | .kiro/specs 5 个全读 + docs/ 顶层文档全读 | ✅ 已完成（约 2.2 万行全读）+主会话裁决其发现 2 为误报并已回写修正（user.reply.task 生产零消费结论维持）；高价值发现：sunset notice 本身已过时、task-status-manifest 是任务状态唯一权威（SR-179）、SR-180 evolution 自动发布契约冲突、47 域审计全判 inconclusive（SR-183） |
| 18 | 18-superpowers-specs.md | docs/superpowers/specs 165 篇全读提炼 | ✅ 已完成（165/165 全读+plans 149 清单；user.reply.task 退役在全部 spec 零记录=最大文档-代码漂移点，退役窗口 07-15→08-05；守卫哲学单向演进"字符匹配→语义判断"；07-10 findings 文档被引用但不存在） |
| 19 | 19-uncommitted-scripts-ci.md | 47 个未提交文件完整 diff + scripts/** + .github/** | ✅ 已完成+主会话抽查通过（改动=6 组后端工作+1 组 biz-test 硬化，src 与 scripts 必须同批合入；**已验证提交阻断风险：import.rs 新增行两处含禁词"人工"，进 PR 必红**；domain8 的 severity="BLOCKED" 误用为死代码级 bug） |
