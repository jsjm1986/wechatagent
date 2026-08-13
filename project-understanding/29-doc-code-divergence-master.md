# 文档-代码偏差权威总表（核证日期 2026-08-13）

> **用途**：全仓"文档声称 X、代码实际 Y"类偏差的合并去重权威底单——后续任何人（含 AI 会话）读本仓任何文档时的防误导对照表。
>
> **来源**：19 份深读记录（`project-understanding/01`–`19`）各自"偏差与疑点"节 + `PROJECT_UNDERSTANDING_LEDGER.md` 偏差表，合并去重后共 **71 条编号条目**（DIV-01…DIV-71）。
>
> **核证纪律**：每条均于 2026-08-13 当场亲验两侧（Read/Grep 读文档原文段落 + 读代码确认差异仍存在），非转抄旧记录。行号为核证时点值，工作区含未提交改动（47 文件），引用前仍须重验。
>
> **范围界定**：只收"文档/注释/spec/测试头/文案 声称 X、代码实际 Y"。纯行为缺陷（毒丸行、知识窗口错位、deferred_wake 取消等）不在本表，见总台账第五部分。
>
> **严重度**：**高** = 按文档做会犯错（写错代码/做错运维操作/得出错误结论并据此行动）；**中** = 概念理解偏差（对机制的理解会系统性走偏）；**低** = 数字/行号/命名/文案过时（就地即可识破）。

---

## 1. 总表

### 1.1 高严重度（4 条）——按文档做会犯错

| # | 文档出处 | 文档声称 | 代码实际 | 证据（两侧均亲验） | 来源记录 |
|---|---|---|---|---|---|
| DIV-01 | `CLAUDE.md:148`（Hard rules 节） | "current rules: `FactRisk ≥ 6` block, `PressureRisk ≥ 7` block, `HumanLikeScore < 6` rewrite once, `EmotionalValue < 6` rewrite once, `ProductAccuracyScore < 7` block"——5 闸字符串守卫式表述 | 销售域字符串守卫 2026-05-25 已删除；现为 Review **分数闸**体系：`hallucination_block_at`（wire 别名 factRiskBlockAt）硬拦、`knowledge_grounding_block_below`（别名 productAccuracy）硬拦、**pressureRisk ≥ 阈值是软闸触发 single-shot revision 而非 block**、humanLike/emotionalValue 同为软闸 | 文档侧 `CLAUDE.md:148`；代码侧 `guards.rs:1-7`（删除声明）、`runtime.rs:414-418`（别名映射 `fact_risk_block_at: typed.hallucination_block_at` / `product_accuracy_block_below: typed.knowledge_grounding_block_below`）、`review/gates.rs:37-44,152-219`（pressure_risk 入 `soft_risks` 走 revision） | LEDGER#1、04、09、17、18 |
| DIV-02 | `.kiro/specs/agent-autonomy-loop/requirements.md:123-137`（R1"自治协议 9 字段"）；`docs/superpowers/specs/2026-07-13-reply-prompt-slimming-design.md:9,95`（以 `user.reply.task` 为生产主 prompt 做 A/B 瘦身）；全部 165 篇 superpowers specs 零退役记录 | Reply Agent 每轮输出 9 个思考链字段（userUnderstanding/relationshipRead/…/riskSelfCheck）；`user.reply.task` 是生产主 prompt | **完整版已退役、生产零消费**：三站点（首发/rewrite/revision）统一 `user.reply.fast.task`（紧凑 schema，思考字段仅 conversationModeReason/riskSelfCheck/whyShouldReply 3 个）；`user.reply.task` 处于"种子仍种入 DB、治理面仍覆盖、测试仍钉内容、运行时零调用"的退役态；specs 目录 `user.reply.fast.task` 零命中（换代未落任何 spec） | 文档侧 requirements.md:123-137、slimming spec:9,95、`rg -l 'user.reply.fast.task' docs/superpowers/specs/`=0 命中；代码侧 `decision.rs:460,1321`（生产 key）、`prompts.rs:1268-1275`（fast 3 字段 schema）vs `prompts.rs:1329-1344`（退役 9 字段 schema 仅存种子）、`rg -l '"user.reply.task"' src/`=7 文件全为治理面/种子/测试 fixture | LEDGER#2、17-Q1、18-② |
| DIV-03 | 5 处 spec 头部 sunset notice：`.kiro/specs/agent-autonomy-loop/requirements.md:9`、`agent-autonomy-loop/design.md:9`、`user-ops-agent-hardening/requirements.md:8`、`user-ops-agent-hardening/design.md:9`、`agent-self-evolution/tasks.md:8` | "运行时收敛为 3 闸（`enforce_knowledge_grounding / enforce_hallucination / enforce_run_budget`，详见 `src/agent/guards.rs`）" | 该"3 闸 enforce_*"是**中间态**：`enforce_knowledge_grounding`/`enforce_hallucination` 函数全库不存在（`rg 'fn enforce_' src/agent/` 仅 enforce_state_action_policy / enforce_task_send_authorization）；在线入口是 `review/gates.rs` 的 review_passed / classify_dual_gate / finalize_review_for_send 分数闸 + 三软闸；`docs/agent-policy.md:3-7` 顶注是正确版本 | 文档侧 5 处行号亲验；代码侧 `rg 'fn enforce_'` 全量、`guards.rs:1-7`、`agent-policy.md:3-7` | 17-Q2 |
| DIV-04 | `src/agent/outbox_dispatcher.rs:2730-2734` 注释 | "`manual_send`（admin UI 主动发）不受此门约束——admin 已显式确认发送意图"（发送前 contact 状态门豁免 manual_send） | 该豁免**不可达**：上游 `second_safety_gate` 的豁免清单只有三种内部通知（`outbox_dispatcher.rs:922-930`：PRINCIPAL_ESCALATION/CLARIFICATION/SYSTEM_INCIDENT，无 MANUAL_SEND），其纯函数第 0 条 `!is_managed → not_managed_at_send`（`outbox.rs:640-655`）先取消——admin 对 normal/paused 客户的手动发送在二次门即被取消，永远到不了下游为它设计的豁免 | `outbox_dispatcher.rs:922-930`（二次门豁免清单）+ `outbox.rs:640-655`（not_managed 拦截）+ `outbox_dispatcher.rs:2737-2749`（下游豁免与注释） | LEDGER五#5、05#1 |

### 1.2 中严重度（32 条）——概念理解偏差

| # | 文档出处 | 文档声称 | 代码实际 | 证据 | 来源 |
|---|---|---|---|---|---|
| DIV-05 | `CLAUDE.md:127-136`（Webhook → Agent flow 图） | 流程图把 `run_user_operation_gateway(...)` 画在 webhook 箭头链上，暗示 webhook 同步跑 Agent | webhook 只落库 + 物化 durable `inbound_reply` 任务（quiet 时 defer 到 wake），非 quiet 时 `tokio::spawn` 低延迟唤醒 `run_due_task_by_id`——决策执行在 task worker 层 | `CLAUDE.md:130-134`；`webhooks.rs:1602-1642`（quiet materialize / durable task + spawn 亲读） | LEDGER#7、03 |
| DIV-06 | `CLAUDE.md:160`（Prompt + knowledge conventions） | "Knowledge is progressive-disclosure: catalog → list_chunks → open_slice via tool-calling"（暗示 user-ops 主链路多轮工具检索） | user-ops 单发决策路径**不支持 tool_calling 中间轮**——LLM 误输出该相位会被强制转 final + 清空 tool_calls + 记 degraded；知识由 gateway 预先路由（knowledge_agent 多轮在路由内部）；catalog→search→open_slice 工具循环服务管理台 chat（chat_tool_loop"永不写库、永不进 outbox"） | `gateway.rs:2912-2937`（强制转 final 亲读）；`chat_tool_loop.rs:12-16` | LEDGER#8、01、07 |
| DIV-07 | `CLAUDE.md:60`（Common commands 节） | "The shell here is bash on Windows…the project root contains non-ASCII characters (`工作项目`)" | 当前环境 macOS（darwin），项目根目录为 `开发项目`（非 `工作项目`）；按文档假设 Windows 会做出错误的 shell/路径决策 | `CLAUDE.md:60` 原文亲读；`<user_info>` OS darwin 25.5.0 + Workspace Path `/Users/a1234/Desktop/开发项目/wechat` | LEDGER#5（部分） |
| DIV-08 | `.kiro/specs/agent-autonomy-loop/requirements.md:32,411,438,590` | 四基线 PBT 含 `string_fact_risk_guard` | 现行第四个基线 PBT 是 `wiki_chunk_revision_pbt`（string_fact_risk_guard 随 2026-05-25 清理换出） | requirements.md 四处行号亲验；`scripts/check-baseline.sh:8,84` | LEDGER#6、17-Q3 |
| DIV-09 | `.kiro/specs/agent-autonomy-loop/requirements.md:133-137`（R1.4/R1.5） | `should_reply=true` 时 `whyShouldReply` SHALL ≥10 unicode 字符（≥6 汉字），违反追加 `missing_required_field:whyShouldReply`；不足降级 insufficient_detail | fast 契约 `validate_reply_critical` 只查 reply_text 非空 + risk_self_check + why_skip_reply，**不校验 why_should_reply 长度**；finalize 的 insufficient_detail 降级分支（gates.rs:688 附近）在 fast 主链路因 promote_risks 不含该前缀而实际不可达 | requirements.md:133-137；`types.rs:855-872` 亲读（无 why_should_reply 校验） | 04#4 |
| DIV-10 | `.kiro/specs/user-ops-agent-hardening/requirements.md:146,152`（R8.1 + R8.7） | coreFacts cap=6 **且** R8.7 要求 PBT 验证"任意初始集合 S（不在 discarded）最终 coreFacts ⊇ S"——两不变量在 \|S\|>6 时数学不可满足 | spec 矛盾措辞仍在未修；实现已从"静默 truncate(6)"升级为**容量淘汰迁 recentFacts + `extra.coreFactEvictions` 有界审计**（不再无痕丢弃）；PBT 对 >6 场景弱化断言（"仅断言未 discarded 不会被…"） | requirements.md:146,152；`memory.rs:435-530`（SR-182 注释 + evict→recent+audit 亲读）；`tests/memory_card_invariants.rs:716` | 17-Q8（行为侧已改善，见 §4） |
| DIV-11 | `.kiro/specs/agent-self-evolution/requirements.md:254`（R9.6） | "本期 SHALL **不**引入'自动发布'开关——release 永远需要人审 + 二次确认串；放宽须独立 M5+ spec" | 代码已存在 auto_release 机制（`auto_release_eligible_thresholds`，EVOLUTION_AUTO_RELEASE_ENABLED env + workspace flag 双闸），仅被代码内政策硬闸 `CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 恒压制（刻意死代码）；"永远人工"目前行为上成立但机制已引入，spec 未修订（SR-180 记录的 HC 决策未回写） | requirements.md:254；`auto_release.rs:3,40-44` 亲读 | 17-Q4 |
| DIV-12 | `.kiro/specs/knowledge-digest-workstation/requirements.md:102`（R7.1） | "baseline 不能跌：`cargo test --lib` ≥ 78 / 0" | 现行基线 LIB_BASELINE=**350**（78 是 autonomy 时代旧值；按 78 设门会放水） | requirements.md:102；`check-baseline.sh:28` | 17-Q3 |
| DIV-13 | `.kiro/specs/{agent-autonomy-loop,agent-self-evolution,user-ops-agent-hardening}/tasks.md` 正文历史勾选 | 任务表 `[x]`/`[~]` 标记暗示已交付 | 历史勾选与真实交付不符（SR-179：未接线/已删除/验收不等价的任务曾标 [x]）；唯一状态权威是 `task-status-manifest.json`（"statusAuthority: This manifest, not historical markdown checkboxes"）；三 spec tasks.md 顶部已加权威注记；**注意** manifest 只覆盖三 spec 且 asOf=2026-07-24 | `task-status-manifest.json:1-10` 亲读；`agent-autonomy-loop/tasks.md:1-7` 顶注亲读 | 17-Q5 |
| DIV-14 | `.kiro/specs/universal-test-coverage/deepread-verify-result-2026-06-30.json:3` | `"completed": 47`——47 业务域全部深读验证完成（300 verified_gaps） | 同目录 `audit-status-manifest.json` 权威改判全部 47 域 **inconclusive**（classification=legacy_inconclusive：无冻结 commit/模型指纹/输入哈希）；引用 47 域结论只能当线索（research_leads）不能当事实 | 两 JSON 亲读（completed:47 vs inconclusive:47） | 17-Q6 |
| DIV-15 | `docs/DEPLOYMENT-STEPS.md:320,332` | 部署验证命令用 `db.agent_runs.find(...)` | 集合实名 `agent_run_logs`（`db/mod.rs:218-219` typed accessor）；按文档查询会得到空集合、误判"系统没跑" | DEPLOYMENT-STEPS.md:320,332；`db/mod.rs:218-219` | 17-Q12 |
| DIV-16 | `src/agent/review/mod.rs:4390` 注释 | "第二 provider 调用失败仅 warn 不阻塞——双脑是增益机制，不应成为新故障源" | second reviewer **调用成功但 parse 失败**时 `return Ok(hold_for_review_schema_failure(...))` 拉闸整个 run（发送被拦）——一个输出不规范的次级模型可持续压制发送；仅 LLM 调用失败路径符合注释 | `review/mod.rs:4390`（注释）vs `4409-4415`（行为）亲读 | LEDGER五#3、04#3 |
| DIV-17 | `src/agent/quiet_hours.rs:118-131` doc 注释 | G04 三级解析链：contact override → profile.per_relationship → profile 默认范式取 `quiet_hours.enabled_override`，缺省回落 global | 函数体只认 workspace 开关：`workspace_enabled` 直接返回（"Workspace policy is authoritative. Contact/profile overrides…no longer alter scheduling behavior"）——doc 描述的三级链已废 | `quiet_hours.rs:118-131` vs `:132-140` 亲读 | 05#4 |
| DIV-18 | `src/agent/escalation/mod.rs:232` 注释 | "骚扰门关：跳过推卡（pending 台账可由 admin 在收件箱处置）" | 骚扰门不过时 `return Ok(())` 直接返回，`insert_pending_escalation`（:262 附近）根本未执行——被拦的 hold 请示**不留任何台账痕迹**，admin 收件箱无从处置 | `escalation/mod.rs:225-268` 亲读（return 在 insert 之前） | 05#3 |
| DIV-19 | `src/agent/knowledge_agent.rs:6-8`（模块 doc）+ `:41-42`（MAX_ROUNDS 注释） | "round 1 额外注入**文档级目录**（catalogSummary/routingMap 导航卡片），让 agent 先选文档再下钻原子（#619）" | `DocEntry` 结构体（:133-145）全库零构造点（rg 仅定义处 1 命中）——文档级目录从未注入，"先文档后原子"分层召回链路不可达，open_document 因 agent 无从得知 documentId 近乎不可用 | `knowledge_agent.rs:6-8,41-42,133-145`；`rg 'DocEntry' src/`=1 命中 | 07#1 |
| DIV-20 | `src/agent/knowledge_agent.rs:436` doc 注释 | AnswerStreamer "只认顶层 answer 键，忽略嵌套对象里的同名键（用 depth 计大括号层级）" | `locate_answer_value_start`（:497+）是朴素子串定位：找 `"answer"` 子串+冒号+引号，**无任何 depth 计数**；顶层 answer 之前若出现嵌套同名键，token 流会提前把嵌套值当正文下发 | `knowledge_agent.rs:433-440` vs `:495-532` 亲读 | 07#2 |
| DIV-21 | `src/evolution/post_release.rs:3`（模块头）+ `:74-75` | "每次 release 后由 release.rs 调 `schedule_post_release_review` 插一条文档"；"本函数**不参与** release transaction" | `schedule_post_release_review` 全库零调用方（rg 仅定义+注释自引）；生产路径是 release.rs 在 **release 事务内**直接 `insert_one_with_session(post_release_review_document(...))`——两句注释描述的都是旧兼容路径 | `post_release.rs:3,74-76`；`rg 'schedule_post_release_review' src/`=2 命中（定义+自引）；`release.rs:115-117` 亲读 | 10#1 |
| DIV-22 | `src/evolution/runtime_flag.rs:46-48` 注释 | 灰度桶号"足以在 worker / webhook / shadow 三路调用方拿到一致的桶号" | `is_evolution_enabled_for` 无生产调用方（rg 命中仅 re-export/注释/自身）；webhook/gateway 侧没有接灰度判定——灰度实际语义只是"哪些 contact 的 run 进演化 cohort"，不影响生产回复行为 | `runtime_flag.rs:44-51,78-84`；`rg 'is_evolution_enabled_for' src/`=4 命中全非调用 | 10#2 |
| DIV-23 | `src/evolution/auto_release.rs:287-289` 注释 | "与 `threshold::generate` 内的口径一致：…rewrite 类用 revision_applied=true 给 human_like/emotional_value 各 +1 命中" | threshold::generate 实际是各 **+0.5**（"暂按两侧各 0.5 分摊"）——auto_release 的 rewrite 命中率是生成侧口径的**两倍**，注释声称一致不实（政策硬闸恒关暂无实害） | `auto_release.rs:283-338` vs `threshold.rs:110-126` 亲读 | 10#3 |
| DIV-24 | `src/evolution/mod.rs:229-231` 注释 | "replay 现阶段 threshold 不调 LLM、prompt 走 placeholder failed，所以这里不会再触发 BudgetExceeded" | replay.rs 已接**真实** `shadow_replay_prompt_one`（跑 decide_reply + review_decision 演练）；shadow LLM 消耗不计入 EvolutionBudget（tick 级预算对 W3 是占位检查） | `evolution/mod.rs:225-235` vs `replay.rs:9,242-245` 亲读 | 10#4 |
| DIV-25 | `src/evolution/post_release.rs:55-58` 注释 | "blocked_by_safety_guard 来自产品声明 fail-closed/relay，**与 fact/pressure 无关**；pressure_risk 是软闸命中率改走 revision_failed" | `threshold.rs`/`auto_release.rs:328-330`（significance 同源权威）把 `blocked_by_safety_guard` 记为 **pressure_risk_block 命中**——同一 gate 在生成/评估侧与发布后观测侧口径相互矛盾，无法从代码读出哪侧反映生产真实 | `post_release.rs:55-70` vs `auto_release.rs:325-340` 亲读 | 10#5 |
| DIV-26 | `src/routes/shared.rs:1090-1098`（guide preview prompt） | prompt 要求 LLM 输出 `readableChanges`（业务话术："将更新用户画像"等） | preview 响应的 readableChanges 实际取 `frozen_plan.authoritative_changes` 的 `"{target} / {label}"` **机器拼接**，LLM 产出的文案被丢弃——前端显示的是键名拼接不是业务话术 | `shared.rs:1090-1098` vs `guides.rs:595-601` 亲读 | 11#11 |
| DIV-27 | `src/routes/admin_ops_versions.rs:1235`、`src/routes/admin_taxonomy_candidates.rs:191-192` 注释 | "TaxonomyEntry 无 workspace_id、只有 scope"；"TaxonomyCandidate/TaxonomyEntry 无 workspace_id 字段，隔离边界是 scope" | 两模型**均有** `workspace_id`（serde default，m032 已物理回填）且相关 filter 恒带它——注释残留自字段引入前，据此推断租户隔离边界会得出错误结论 | 两处注释亲读；`models.rs:3644,3753`（`#[serde(default = "default_taxonomy_workspace_id")]`） | 12#1 |
| DIV-28 | `src/routes/observability.rs:1342` 行内注释 + `src/routes/mod.rs:974` 挂载注释 | "auto_resolved/applied/dismissed 之比是 sweep 命中率" | 实际输出 `historicalResolvedShare`——保留历史中已解决状态占比；集合无 run/cohort 标识**无法**反推单轮 sweep 命中率（同文件 :1409-1413 的权威注释与输出键名已说明）——三处注释相互矛盾，以后者为准 | `observability.rs:1342` vs `:1405-1413,1438`；`routes/mod.rs:974` 亲读 | 12#4 |
| DIV-29 | `tests/revision_recheck_action_gate.rs`（GATE-1）、`tests/memory_card_write_occ.rs`（CONC-1）文件名与头注 | 测试名/注释声称守护"revision 后动作闸复检"与"memory_card OCC 并发写"两条安全不变量 | **两个空壳测试永远绿**：前者函数体纯注释零断言零代码；后者仅 `TestApp::start()` + `let _ = &app;`——两条主链路安全不变量无可执行守护（覆盖幻觉） | 两文件全文亲读（revision_recheck:20-38 全注释；memory_card_write_occ 全文 23 行） | LEDGER五#8、15#1 |
| DIV-30 | `tests/worker_reclaim.rs` 测试名 `stale_running_task_is_recovered_to_retry` | 声称验证 stale running 任务被回收为 retry | 实际只 insert 后断言 `status=="running"`（未驱动任何 reclaim；注释自认 worker tick 私有、退而求其次）——HP-1 stale 回收端到端行为未被真正测到 | `worker_reclaim.rs:55-72` 亲读 | 15#2 |
| DIV-31 | `tests/ingest_worker_smoke.rs:1-12` 文件头 | "RSS 源 → feed-rs 解析 → 落 ≥1 chunk…拉取成功后 last_fetched_at 刷新、failure_streak 归零" | 四个测试函数全为拒绝/跳过路径（rejects_loopback/rejects_metadata/skips_not_due/rejects_due_private）——**零正向成功用例**；RSS/HTML 真实拉取成功路径在 tests/ 全集疑似无集成覆盖 | 文件头 + `rg 'async fn' tests/ingest_worker_smoke.rs` 亲验（4 测试名） | LEDGER五#11、16#1 |
| DIV-32 | `tests/domain_profile_e2e.rs:454-479` 测试名 `e2e_delete_forbidden_on_active` | 声称测"删除 active profile 被禁止" | 实际未发任何删除请求——尾注自认"DB 本身不阻止删除 active 行——业务规则由 handler 层强制。本测试验证 active 行确实存在（前置条件）"；删除被拒路径无覆盖 | `domain_profile_e2e.rs:454-479` 全函数亲读 | 16#5 |
| DIV-33 | `docs/superpowers/plans/2026-07-11-full-system-test-remediation.md:5,20` | 引用台账 `docs/superpowers/specs/2026-07-10-full-system-test-findings.md` 作为设计依据 | 该文件不存在（specs 目录 2026-07-10 前缀共 8 文件，无 full-system-test-findings）——疑似未落盘或被并入 deep-logic-audit | plan:5,20 亲读；`ls docs/superpowers/specs/ | rg '2026-07-10'` 全列 | 18#2 |
| DIV-34 | `deploy.sh:46,115,132-136` | 部署引导：8080 端口直连、`117.72.54.28:3001` MCP、交互式合并 main 流程（2026-07-07 冻结） | 现行部署工具链在 `scripts/deploy/`（candidate_smoke.py:280 把 3003/8080 列为保留端口拒用；audit_hc001 的健康检查 URL 是 `127.0.0.1:3003`）——deploy.sh 是过时快照，按它操作与现行产线不符 | `deploy.sh` 三处亲读；`scripts/deploy/candidate_smoke.py:280`、`audit_hc001_server_carriers.py:34` | 19#6 |
| DIV-35 | `.env.example`（自称配置事实源） | 未收录 POST_DECISION_*/SILENCE_SIGNAL_* 两族变量 | `config.rs` 存在这两族约 10 个变量（post_decision_worker_concurrency、SILENCE_SIGNAL_WORKER_ENABLED/INTERVAL/DAILY_CAP 等）；digital_twin 验收恰依赖 post-decision worker，运维排障缺文档入口 | `rg -c 'POST_DECISION|SILENCE_SIGNAL' .env.example`=0；`config.rs:195,508,655-661` | 19#12 |
| DIV-36 | `src/routes/mod.rs:1080+`（死路由 tripwire 测试 `no_orphan_pub_async_route_handlers`） | include_str! 名单暗示覆盖全部路由文件、新增 handler 忘挂载会被抓 | 名单缺约 11 个文件（campaigns.rs、ask_human_inbox.rs、principal_escalations.rs、domain_profiles.rs、guide_profile.rs、media_assets.rs、referral_cards.rs、send_ledger.rs、operation_view.rs、worker_controls.rs、management_prompt_edit.rs）——这些文件新增 `pub async fn` 忘挂载不会被该测试抓到 | `routes/mod.rs:1080-1096` 段亲读 + `rg '"campaigns.rs"' src/routes/mod.rs`=0 命中 | 12#2 |

### 1.3 低严重度（35 条）——数字/行号/命名/文案过时

| # | 文档出处 | 文档声称 → 代码实际 | 证据 | 来源 |
|---|---|---|---|---|
| DIV-37 | `README.md:326` | "231 条路由" → 实际 235（`rg -c '\.route\(' src/routes/mod.rs`） | 亲跑计数 =235 | LEDGER#3 |
| DIV-38 | `README.md:185,226` | "56 个有序迁移" → 实际 58（m001–m058） | `ls src/db/migrations/`=58 个 m 文件，尾三个 m056/m057/m058 | LEDGER#4 |
| DIV-39 | `CLAUDE.md:135`（Test baseline 节） | "`scripts/check-baseline.{sh:25,ps1:17}` LIB_BASELINE=350" → 实际 sh:**28** / ps1:**20** | `check-baseline.sh:28`、`.ps1:20` 亲读 | **本次新发现** |
| DIV-40 | `src/agent/gateway.rs:2915` 防御注释 | "根因：`user.reply.task` prompt 提供了 tool_calling 形态…" → 生产 prompt 是 `user.reply.fast.task`（注释沿用退役 key 名） | `gateway.rs:2912-2916` vs `decision.rs:460` | **本次新发现**（DIV-02 关联） |
| DIV-41 | `src/agent/gateway.rs:3922-3924` 去抖注释 | "必须在 apply_agent_updates 之前——后者无条件把 last_agent_run_at 推到 now" → inner 主路径已无内联 `apply_agent_updates` 调用（仅 post_decision 投影 worker :935/:1109 调用）；检查本身仍必要，但注释因果对象已迁移 | `gateway.rs:3915-3924` + `rg 'apply_agent_updates\(' src/agent/`（仅 post_decision 两处调用） | 01#1 |
| DIV-42 | `src/webhooks.rs:1463` 注释 | "db/indexes.rs:55-63 的 partial unique index" → 实际位于 `indexes.rs:813-817` | 两侧亲读 | 03-D1 |
| DIV-43 | 多处注释内嵌 file:line（系统性） | 例：`m016:200` 写 "LlmProviderConfig(models.rs:4732)"（02 号实测 6026）；`m030:3` 写 "models.rs:451"（实测 460）；`knowledge_digest/labels.rs:2` 写 "mod.rs:277-282"（4 状态扫描实际在 :355+）；大量测试注释引用的生产行号为写作时快照 | m016:200、m030:3、labels.rs:1-3 亲读 + digest mod.rs:355 | 02#5、08#9、15#6 |
| DIV-44 | `src/models.rs:1802` 注释 | 引用迁移 `2026_05_W1_001_chunks_wiki_type_default` → 该 id 不在 MIGRATIONS 注册表（幽灵引用）；wiki_type 缺省实际靠读取侧兜底 | models.rs:1800-1804 亲读；`rg '2026_05_W1_001' src/db/migrations/mod.rs`=0 | LEDGER五#7、02#12 |
| DIV-45 | `src/models.rs:909-917` doc 注释 + `:948-963` 单测 | "历史值清单"与单测均只列 8 个 status → 闭集 `ALLOWED_AGENT_TASK_STATUS` 实为 9 值（含 `committing`，:920） | models.rs:905-965 亲读 | 02#3 |
| DIV-46 | `src/models.rs:1993` 注释 | "`source` ∈ {ai, human, rule, imported}" → 存在第五个合法值 `lesson_promotion`（m055 定义 + 唯一索引使用） | models.rs:1991-1994；`m055:19`、`indexes.rs:164-169` | 02#6 |
| DIV-47 | `src/db/migrations/m028:110` | 日志 `migration_id = "m028_seed_conversation_mode"` → 注册 id 是 `2026_06_Y2_001_seed_conversation_mode`（mod.rs:415）；仅影响日志检索 | 两侧亲读 | 02#4 |
| DIV-48 | `src/agent/decision.rs:1-9` 模块头注 | "构造 `user.reply.task` prompt…其它子模块通过 pub(crate) 调用 `decide_reply`" → 生产 key 是 fast.task；`decide_reply` 函数不存在（只有 `decide_reply_with_promote`，:590） | decision.rs:1-9 亲读；`rg 'fn decide_reply' src/agent/`=1 命中（with_promote） | 04#1 |
| DIV-49 | `src/supervisor.rs:3-5` 头注 | "main.rs 用 tokio::spawn 拉起 8 个长驻 worker"（列举 9 个名字） → `SUPERVISED_WORKERS` 实际 **16** 个 | supervisor.rs:1-51 亲读（数组 16 项） | 09#1 |
| DIV-50 | `src/llm.rs:288-289` vs `:986` | 解析层数口径混乱："前两层（快路径 + repair_loose_json + extract_embedded_json）"（3 个东西叫两层）vs "三层确定性解析…第四层 LLM-repair" → 实际结构 = 3 层确定性 + 1 层回喂，代码一致文档口径乱 | 两处亲读 | 09#4 |
| DIV-51 | `src/agent/memory.rs:575-579` 注释 | "历史镜像 cap…与 typed 三数组固定 cap 6/10/20 的 wire 兼容形态" → 镜像给 deprecatedFacts 的 cap 是 **6**（:582）而 typed 层 truncate(**20**)（:531），6≠20 无解释 | memory.rs:575-582 + :531 亲读 | 06#2 |
| DIV-52 | `src/agent/knowledge_router.rs:1126-1128` 注释 | record_chunk_hit "fire-and-forget——不阻塞 gateway 决策" → 实现是循环内**顺序 await** 每次 update（`let _=` 只吞错），N 个 chunk = N 次串行 DB 往返，仍在 gateway 请求路径上 | knowledge_router.rs:1123-1150 亲读 | 07#6 |
| DIV-53 | `src/knowledge_wiki/block_parser.rs:21-23` doc | "行内出现 token 但不在行首 → 当作普通正文" → 判定是 `trim_start` 后整行相等（:93,128）——**左侧缩进**的 `---END CHUNK---` 也生效，"行首"实为"去左空白后行首" | block_parser.rs:18-26 vs :88-132 亲读 | 07#7 |
| DIV-54 | `src/knowledge_wiki/gap_signals.rs:11` 模块 doc | "8 类 signal kind" → 结构 lint 实产 9 类（dangling_anchor 后加；:198 注释自称"第 9 类"） | gap_signals.rs:8-14 + :141,198 亲读 | 07#8 |
| DIV-55 | `src/db/indexes.rs:2618` 函数名 | `ensure_evolution_indexes` 暗示只管 evolution → 函数体含 admin_users、lessons_learned 等大量非 evolution 集合的索引（知识日报/wiki/auth/ingest/请示台账），按名找索引会漏一大半 | indexes.rs:2618 起函数体 awk 扫描（admin_users/lessons_learned 命中） | 02#1 |
| DIV-56 | `src/evolution/cohort.rs:113-115` 注释 | "空 contact_wxid 视为'无 contact'分组下的一个自然 contact 组" → `contact_in_runtime_cohort`（:138-142）要求 `!contact.is_empty()`，空 contact 进不了池，该分支实际不可达（仅单测直调 dedup 可达） | cohort.rs:108-118,138-144 亲读 | 10#6 |
| DIV-57 | `src/evolution/mod.rs:62-64` 启动日志 | "evolution worker starting (M4 W1 skeleton — empty tick by design)" → tick 已是 W4 全功能（六段编排） | mod.rs:60-68 亲读 | 10#12 |
| DIV-58 | `src/evolution/threshold.rs:178` 字段名 | `proposed_raw` 暗示存原始值 → 实存 `decide_candidate` 返回的 **clamp 后**值；:208 `cohort_notes.clamped_to_value` 记录同一值——"raw"名与语义相反 | threshold.rs:160-182,204-212 亲读 | 10#9 |
| DIV-59 | `src/agent/escalation/ledger.rs:792-794` 注释 | derive_sediment_title "目前尚无生产调用点（Task 4 接线时才启用），暂 allow(dead_code)" → 已被 `emit_knowledge_gap_proposal` 调用（:681）——dead_code 注记过期 | ledger.rs:758-800 + :681 亲读 | 05#11 |
| DIV-60 | `src/tasks.rs:722` 事件文案 | "已安排第 {attempt_count}/{max_attempts} 次重试" → attempt_count 因 claim 用 ReturnDocument::After 已含本次执行，实义"第 n 次执行失败"；判定逻辑正确仅文案歧义 | tasks.rs:718-726 亲读 | 03-D6 |
| DIV-61 | `src/agent/knowledge_router.rs:771-780` | fallback_rank 六行说明注释原样重复两遍（复制粘贴残留） | 亲读确认两遍 | 07#10 |
| DIV-62 | `src/agent/runtime.rs:459+`（as_document）关联 reviewer "硬运行参数"注入 | as_document 回写 wire doc 不含 `allowed_conversation_modes` 等 4 组新字段（字段 :339 存在）——注入 reviewer 的"硬运行参数"与结构体字段集已漂移 | runtime.rs:459-492 doc! 键集 + `rg 'allowed_conversation_modes' runtime.rs`（as_document 内 0 命中） | 04#7 |
| DIV-63 | `src/agent/outbox_dispatcher.rs:967` 参数名 | `decision_created_ms` 暗示 decision review 创建时刻 → 实取 `entry.created_at.timestamp_millis()`（outbox entry 创建时刻）；正常链路毫秒级近似，窄窗口下 stop 判定语义与名不符 | outbox_dispatcher.rs:963-970 亲读 | 05#2 |
| DIV-64 | `frontend/src/main.tsx:10-12` 注释 | "开关用 sessionStorage（重启 tab 也能复现）" → `wa.authed` 只写不读（全库零 getItem），登录判定完全依赖 `/api/auth/me`，该键无消费者 | main.tsx:8-22 亲读 + rg getItem=0 | 13#1 |
| DIV-65 | `frontend/src/lib/api.ts:117-118` 头注 | openEventSource "替代散落的裸 EventSource" → dead code（全库仅定义处命中）；SSE 实际走 createSseReconnector 与裸 EventSource | api.ts:115-124 + rg=1 命中 | 13#2 |
| DIV-66 | `frontend/src/features/knowledge/steward.tsx:1206` | TryRecall 的 accountId 输入框 placeholder 写"**客户** ID（可选，默认 default）" → 字段实为**账号** ID（与旁边"联系人 ID"并列易误导） | steward.tsx:1206 亲读（绑定 value={accountId}） | 14#5 |
| DIV-67 | `frontend/src/features/user-ops/cockpit/JudgmentBar.tsx:72-74,91-92` | 断言读取 `lastConversationMode`/`inQuietHours`/`nextWakeAt`——TS 类型（OperationHealth/Contact）未声明这些键（注释自认），类型定义落后于后端实际下发 | JudgmentBar.tsx:72-92 亲读（`as` 断言） | 14#4 |
| DIV-68 | `frontend/src/features/knowledge/labels.ts:62-72` | `REVIEW_CATEGORY_LABELS`/`reviewCategoryLabel` 无任何消费者（rg 仅自身），且与 steward 实际使用的 REVIEW_CATEGORIES 同键不同文——若未来接线会出现两套分类文案 | labels.ts:58-73 + rg 全 frontend=仅自引 | 14#1 |
| DIV-69 | `.kiro/specs/user-ops-agent-hardening/requirements.md:308`（R19.2） | 指标名 `human_handoff_success_rate` + `human_handoff` kind → 含 no-human-takeover lint 禁词词根（lint 只扫 git diff 新增行且不扫 .kiro，历史档案未清理不炸 CI，但按此 spec 实现指标名必红） | requirements.md:308 亲读；`check-no-human-takeover.sh:35` 词表（19 号亲验） | 17-Q12 |
| DIV-70 | `docs/real-task-runbook.md:452,462,469,569` | 大量以 `user.reply.task` 为对象的修复叙述（14 字段、R3 契约等） → 该 key 已退役（runbook 本身已 sunset 不再更新，读时须知对象已不存在） | runbook 四处行号亲读 | 17-Q10 关联、18-② |
| DIV-71 | `src/routes/knowledge/digest_inbox.rs:724-731`（禁词防御测试） | candidates 数组锁的文案 "缺 sourceQuote"/"sourceAnchors 为空" → 实现实际文案是"缺原文出处"（:522）/"原文定位锚点为空"（:538）——测试锁旧文案副本，禁词防御未覆盖现行文案（现行恰好也无禁词，防御失效但未爆雷） | digest_inbox.rs:714-740 vs :522,538 亲读 | 08#5 |

---

## 2. 按误导源文件的反向索引

> 读以下任一文件前，先扫对应失真点清单。未列出的文件不代表零失真，代表 19 份深读记录未报告"文档声称类"偏差。

### 2.1 根文档

| 文件 | 已知失真点 |
|---|---|
| `CLAUDE.md` | DIV-01（:148 五闸表述→分数闸）、DIV-05（:127-136 webhook 流程图暗示同步）、DIV-06（:160 tool-calling 知识检索归属）、DIV-07（:60 "bash on Windows"+"工作项目"→macOS+"开发项目"）、DIV-39（:135 baseline 脚本行号 sh:25/ps1:17→28/20）。另：其引用的 `docs/agent-policy.md` 顶注是**正确**的（可信对照源） |
| `README.md` | DIV-37（:326 路由 231→235）、DIV-38（:185,226 迁移 56→58） |
| `deploy.sh` | DIV-34（8080/117 直连流程整体过时，以 `scripts/deploy/` 为准） |
| `.env.example` | DIV-35（缺 POST_DECISION_*/SILENCE_SIGNAL_* 两族变量） |

### 2.2 `.kiro/specs/`

| 文件 | 已知失真点 |
|---|---|
| `agent-autonomy-loop/requirements.md` | DIV-03（:9 sunset notice 3 闸 enforce_*）、DIV-02（:123-137 R1 九字段协议已退役）、DIV-08（:32,411,438,590 基线 PBT 名单含 string_fact_risk_guard）、DIV-09（:133-137 whyShouldReply 校验/insufficient_detail 在 fast 主链路不生效） |
| `agent-autonomy-loop/design.md` | DIV-03（:9 sunset notice 同款） |
| `agent-autonomy-loop/tasks.md` | DIV-13（历史 [x]/[~] 非交付状态；顶注已声明 manifest 权威） |
| `user-ops-agent-hardening/requirements.md` | DIV-03（:8 sunset notice）、DIV-10（:146,152 R8 双不变量数学矛盾）、DIV-69（:308 R19.2 禁词指标名） |
| `user-ops-agent-hardening/design.md` | DIV-03（:9 sunset notice） |
| `user-ops-agent-hardening/tasks.md` | DIV-13（同上） |
| `agent-self-evolution/requirements.md` | DIV-11（:254 R9.6 "不引入自动发布开关" vs auto_release 机制已存在） |
| `agent-self-evolution/tasks.md` | DIV-03（:8 sunset notice）、DIV-13（同上） |
| `knowledge-digest-workstation/requirements.md` | DIV-12（:102 R7.1 基线写 78→现行 350）。注：R5.4（:86）的"随时可撤销"承诺**已兑现**（见 §4），不再是失真点 |
| `universal-test-coverage/deepread-verify-result-2026-06-30.json` | DIV-14（47 域 completed 被 audit-status-manifest 权威改判 inconclusive） |

### 2.3 `docs/`

| 文件 | 已知失真点 |
|---|---|
| `docs/DEPLOYMENT-STEPS.md` | DIV-15（:320,332 `db.agent_runs`→`agent_run_logs`） |
| `docs/real-task-runbook.md` | DIV-70（user.reply.task 相关叙述；runbook 已 sunset）。ISSUE-012 终态悬置（未终裁，见 §5 未收录清单） |
| `docs/superpowers/specs/2026-07-13-reply-prompt-slimming-design.md` | DIV-02（:9,95 诊断/方案对象 user.reply.task 已退役；批次 2/3 灰度是否走完文档不可考） |
| `docs/superpowers/plans/2026-07-11-full-system-test-remediation.md` | DIV-33（:5,20 引用的 2026-07-10-full-system-test-findings.md 不存在） |
| `docs/agent-policy.md` | 顶注（:3-7）正确描述 enforce_* 已移除——**当前无已知失真点**，是分数闸现状的可信文档 |

### 2.4 后端代码注释（按文件）

| 文件 | 已知失真注释 |
|---|---|
| `src/agent/gateway.rs` | DIV-40（:2915 仍称 user.reply.task）、DIV-41（:3922-3924 apply_agent_updates 因果对象已迁移） |
| `src/agent/decision.rs` | DIV-48（:1-9 头注 decide_reply 函数名 + user.reply.task key 双过时） |
| `src/agent/review/mod.rs` | DIV-16（:4390 "双脑失败仅 warn" vs parse 失败拉闸） |
| `src/agent/outbox_dispatcher.rs` | DIV-04（:2730-2734 manual_send 豁免注释不可达）、DIV-63（:967 decision_created_ms 名不副实） |
| `src/agent/escalation/mod.rs` | DIV-18（:232 "pending 台账可处置"实际未建） |
| `src/agent/escalation/ledger.rs` | DIV-59（:792-794 dead_code 注记过期） |
| `src/agent/quiet_hours.rs` | DIV-17（:118-131 G04 三级链 doc vs workspace-only 实现） |
| `src/agent/memory.rs` | DIV-51（:575-582 镜像 cap 6 vs typed 20） |
| `src/agent/knowledge_agent.rs` | DIV-19（:6-8,41-42 文档级目录未落地）、DIV-20（:436 depth 计数不存在） |
| `src/agent/knowledge_router.rs` | DIV-52（:1126-1128 fire-and-forget 不实）、DIV-61（:771-780 注释重复） |
| `src/agent/runtime.rs` | DIV-62（as_document 缺 4 组新字段） |
| `src/agent/types.rs` | （DIV-09 的代码侧：validate_reply_critical 与 spec R1.4 的差） |
| `src/webhooks.rs` | DIV-42（:1463 索引行号 55-63→813-817） |
| `src/tasks.rs` | DIV-60（:722 "第 n 次重试"文案） |
| `src/supervisor.rs` | DIV-49（:3-5 "8 个 worker"→16） |
| `src/llm.rs` | DIV-50（:288-289 vs :986 层数口径） |
| `src/models.rs` | DIV-44（:1802 幽灵迁移）、DIV-45（:909-917 闭集注释/单测缺 committing）、DIV-46（:1993 provenance 闭集缺 lesson_promotion） |
| `src/db/indexes.rs` | DIV-55（:2618 ensure_evolution_indexes 名不副实） |
| `src/db/migrations/`（m016/m028/m030） | DIV-43（注释行号漂移）、DIV-47（m028 日志 id） |
| `src/knowledge_wiki/block_parser.rs` | DIV-53（:21-23 行首语义） |
| `src/knowledge_wiki/gap_signals.rs` | DIV-54（:11 "8 类"→9 类） |
| `src/knowledge_digest/labels.rs` | DIV-43（:2 行号漂移） |
| `src/evolution/post_release.rs` | DIV-21（:3,74-75 调用方与事务性均过时）、DIV-25（:55-58 blocked_by_safety_guard 口径与 threshold/auto_release 矛盾） |
| `src/evolution/runtime_flag.rs` | DIV-22（:46-48 三路调用方设想） |
| `src/evolution/auto_release.rs` | DIV-23（:287-289 "口径一致"实为 2 倍差） |
| `src/evolution/mod.rs` | DIV-24（:229-231 "replay 不调 LLM"过时）、DIV-57（:62-64 skeleton 日志） |
| `src/evolution/cohort.rs` | DIV-56（:113-115 不可达分支描述） |
| `src/evolution/threshold.rs` | DIV-58（:178 proposed_raw 命名） |
| `src/routes/shared.rs` | DIV-26（:1090-1098 prompt 要求 readableChanges 实被丢弃） |
| `src/routes/admin_ops_versions.rs` | DIV-27（:1235 "无 workspace_id"失实） |
| `src/routes/admin_taxonomy_candidates.rs` | DIV-27（:191-192 同款失实） |
| `src/routes/observability.rs` | DIV-28（:1342 sweep 命中率旧口径；:1037 "workspace_id 强制 default"→实际 admin.current_workspace，12#5 亲验） |
| `src/routes/mod.rs` | DIV-28（:974 挂载注释旧口径）、DIV-36（:1080+ tripwire 名单缺 11 文件） |
| `src/routes/chunk_locks.rs` | （12#6：:398-410 `broadcast_chunk_revised` 死函数 + "调用方覆盖"注释不可能兑现——亲验仍在，归 DIV-36 同类护栏/死代码注释，严重度低） |

### 2.5 前端

| 文件 | 已知失真点 |
|---|---|
| `frontend/src/main.tsx` | DIV-64（:10-12 wa.authed 注释） |
| `frontend/src/lib/api.ts` | DIV-65（:117-118 openEventSource 头注） |
| `frontend/src/features/knowledge/steward.tsx` | DIV-66（:1206 placeholder"客户 ID"实为账号 ID） |
| `frontend/src/features/user-ops/cockpit/JudgmentBar.tsx` | DIV-67（:72-92 类型未声明键断言读取，注释自认） |
| `frontend/src/features/knowledge/labels.ts` | DIV-68（:62-72 REVIEW_CATEGORY_LABELS 死代码+同键不同文）。注：:281-283 关于 targetRefs kind 双口径的注释描述的问题已被后端修正（见 §4），该注释本身略滞后但并集策略无害 |

### 2.6 测试

| 文件 | 已知失真点 |
|---|---|
| `tests/revision_recheck_action_gate.rs` | DIV-29（空壳，GATE-1 无可执行守护） |
| `tests/memory_card_write_occ.rs` | DIV-29（空壳，CONC-1 无可执行守护） |
| `tests/worker_reclaim.rs` | DIV-30（测试名与断言不符） |
| `tests/ingest_worker_smoke.rs` | DIV-31（文件头正向断言 vs 全拒绝路径） |
| `tests/domain_profile_e2e.rs` | DIV-32（:454-479 delete_forbidden 未测删除） |
| `src/routes/knowledge/digest_inbox.rs`（内嵌测试） | DIV-71（:724-731 禁词防御锁旧文案） |
| 泛化警示 | 测试注释内嵌的生产行号均为写作时快照（DIV-43）；约 6 处"复刻式"测试锁的是生产逻辑副本（15#3，作者多有声明），生产改动不会自动变红 |

---

## 3. 高严重度条目详解（按文档做会犯什么错）

### DIV-01 CLAUDE.md 五闸表述（+DIV-03 sunset notice，同一真相的两代错误描述）

**会犯的错**：
1. 按 CLAUDE.md:148 去找 `FactRisk`/`ProductAccuracyScore` 字符串守卫或按 sunset notice 去 `guards.rs` 找 `enforce_knowledge_grounding`/`enforce_hallucination` 函数——**都不存在**，会误判"闸门被人删了"或在错误位置加新闸。
2. 把 `PressureRisk ≥ 7` 当硬 block 去调阈值/写测试——实际它是**软闸**（触发 single-shot revision，生产不产 block 终态）；evolution 侧曾因同款误解把 pressure_risk 映射到它永不产生的 block status（10 号 remediation 史实）。
3. 调阈值时不知道 wire 字段名带历史别名：`factRiskBlockAt` 实际承载 hallucination 阈值、`product_accuracy_block_below` 实际承载 knowledge_grounding 阈值（`runtime.rs:414-418`）——按字面名理解会调错参数。

**正确读法**：以 `docs/agent-policy.md` 顶注 + `review/gates.rs`（review_passed / classify_dual_gate / finalize_review_for_send）+ `runtime.rs` 别名映射为准；阈值现状见总台账第二部分 #7。

### DIV-02 user.reply.task 九字段协议已退役

**会犯的错**：
1. 按 autonomy spec R1 的 9 字段协议开发/调试/评审 prompt 输出——生产 fast.task 只有 3 个思考字段，画像/标签/记忆已移到发送后投影（post_decision worker），你会在错误的 schema 上做功。
2. 在 DB `prompt_templates` 或守护测试里看到 `user.reply.task` 就认为它在生产使用——它处于"种子仍种入、治理面仍覆盖、运行时零消费"的退役态，**判断在用与否必须验证调用点**（17 号记录曾因此误报，已裁决）。
3. 按 2026-07-13 reply-prompt-slimming spec 去继续它的批次 2/3 A/B 瘦身——对象已不存在；其止血措施（runTokenBudget 热更等）是否回调文档不可考。
4. 在 165 篇 superpowers specs 里找这次换代的记录——**没有**（07-15~08-05 spec 空窗期内发生，零记录），这是全仓最大的 spec-代码漂移点。

### DIV-04 manual_send 两道门矛盾

**会犯的错**：
1. 读 `check_contact_status_pure` 的注释（"manual_send 不受此门约束——admin 已显式确认发送意图"）会认为 admin 可对 normal/paused 客户手动发送——实际上游 `second_safety_gate` 无此豁免，撤管竞态时 admin 已确认的发送会在二次门被**取消**，下游豁免永不可达。
2. 修 bug 时若只改下游豁免清单（以为那是唯一门），行为不会变；若照抄下游注释语义去改上游，则实质放宽了安全门——**两个门的注释意图相反，改动前必须对照 admin_outbox 路由/产品预期先裁决哪侧是对的**（05 号记录判定：疑似设计缺口或有意收紧未同步注释，未终裁）。

---

## 4. 亲验中发现的新偏差与已消失的偏差

### 4.1 新发现（本次核证新增，旧记录未收）

1. **DIV-39**：CLAUDE.md:135 写 "`check-baseline.{sh:25,ps1:17}`"，实际 `LIB_BASELINE=350` 在 sh:**28** / ps1:**20**——CLAUDE.md 自身的行号引用也漂了。
2. **DIV-40**：`gateway.rs:2915` 的 tool_calling 防御注释仍写"`user.reply.task` prompt 提供了 tool_calling 形态"——生产是 fast.task，退役 key 名残留在生产注释里。

### 4.2 已消失的偏差（旧记录报告过、当前工作区已不成立——引用旧记录时注意）

1. **CLAUDE.md "lib 基线 2359 passed"**（LEDGER 偏差表 #5 前半）：当前 CLAUDE.md 已无 "2359" 字样（`rg '2359' CLAUDE.md`=0），只保留 ≥350 口径——该半边已被修正；"bash on Windows/工作项目"半边仍在（DIV-07）。
2. **SR-181"operator memory 随时可撤销承诺无撤销路径"**（17-Q9）：已修复——完整撤销链路现已存在：`agent::revoke_operator_memory`（`memory.rs:3423`）+ HTTP 端点（`routes/mod.rs:633` POST + `sources_meta.rs:896`）+ chat intent 分支（`chat.rs:1624,2093`，intent 闭集含 `revoke_operator_memory`，:2209）+ 软撤销审计字段 `revoked_at`/`revocation_reason`（`models.rs:6012-6019`）+ 专项测试 `tests/sr181_operator_memory_revocation.rs`。digest spec R5.4 的承诺已兑现。
3. **SR-182"coreFacts 超 cap 静默 truncate 不留痕"**（17-Q8 的行为侧）：已修复——`memory.rs:435-530` 现为"统一排序 → 容量淘汰项迁入 recentFacts + `extra.coreFactEvictions` 有界审计（cap 20）"，注释明确"不再静默消失"。**但 spec R8.7 与 cap=6 的数学矛盾措辞仍未修**（DIV-10 保留主表）。
4. **07-15 audit [S-02]"EVOLUTION_ENABLED 默认值注释自相矛盾"**（18#6）：已修复——`config.rs:5-7` 现为 `EVOLUTION_ENABLED_DEFAULT = "false"` 且注释一致（"默认关闭"），旧的 true/false 张力注释已不存在。
5. **07-15 audit [S-01]"runtime_flag=None 语义注释与实现相反"**（18#7）：已修复——`evolution/mod.rs:125-126` 现注释"`enabled=false` 或文档不存在 → 全员排除"，与 `cohort.rs` 实现（None→false）及单测 `missing_runtime_flag_excludes_contact_from_cohort`（cohort.rs:208）一致。
6. **models.rs targetRefs kind 注释与 prompts.rs 枚举口径不一致**（14#4 前半）：已修正——`models.rs:5859-5862` 现明确"kind 取值以 prompt 给 LLM 的枚举为准：chunk/pack/proposal；历史上本注释还列过 item/run/evolution_proposal"并说明前端并集策略。前端 `labels.ts:281-283` 注释仍称"两处口径不一致(已知问题)"，略滞后但其并集字典无害。
7. **product-modules "一级列表 9 项 vs '8 模块'表述不一致"**（17-Q12）：未能复现——当前 `docs/product-modules.md` 无 "八/8 个/8 大" 表述（rg 无命中），一级模块列表 9 项属实；原表述可能已修正或原指认不准。

### 4.3 亲验排除的疑点（旧记录存疑、本次核证为不成立）

1. **crud.rs PUT 响应硬编码 draft/needs_review 疑与库内不一致**（08#1，原标"待读 harness 确认"）：**不成立**——该 PUT 的 patch 无条件含 `title`（`crud.rs:794`），而 `title` 在 `REVIEW_SENSITIVE_PATCH_FIELDS`（`chunk_revisions.rs:174-187`），`apply_server_owned_lifecycle`（:201-214）对 Human 来源的敏感字段 patch 同样强制 `draft + needs_review + confidence 0`——响应体硬编码与实际行为一致，不误导。
2. **DomainProfileDraft 不回传 generated_state_machine 会丢 AI 状态机草稿**（13#4）：不成立（主会话 2026-08-13 已裁决）——后端 PUT 是剥离管理键的部分 `$set`（`domain_profiles.rs:1149-1153`），未编辑字段原值保持。

---

## 5. 覆盖自证

### 5.1 输入面（通读）

- `PROJECT_UNDERSTANDING_LEDGER.md` 全文（偏差表 8 条 + 第五部分 11 条缺陷清单 + 待重验清单）。
- `project-understanding/01`–`19` 全部 19 份记录的"偏差与疑点"节逐条读完（01:§5 共 13 条、02:§6 共 15 条、03:§5 D1-D11、04:§5 共 10 条、05:§5 共 12 条、06:§5 共 12 条、07:§5 共 13 条、08:§5 共 12 条、09:§5 共 12 条、10:§5 共 15 条、11:§5 共 15 条、12:§5 共 15 条、13:§5 共 16 条、14:§5 共 6 组、15:§5 共 10 条、16:§5 共 12 条、17:§5 Q1-Q12、18:§5 对照点+12 条、19:§6 共 12 条）+ `README.md`（索引）。

### 5.2 筛选口径

- **收录**：产品文档 vs 代码 / 代码注释 vs 代码行为 / spec vs 代码 / README 数字过时 / 测试文件头（名）vs 测试内容 / 前端注释（类型）vs 后端行为 / prompt 文案 vs 处理行为。
- **不收录**（非"文档声称"类，见总台账第五部分）：纯行为缺陷（毒丸行 D2、知识窗口错位、deferred_wake 被取消、settle 竞态等）、性能观察（reviews N+1、planner N+1、指纹全扫）、风格不一致（错误码风格、BSON 命名三分、写防护强度不一）、诚实自认的设计取舍（A-03/A-04/A-05、B-01、strip_known_tags 边界、real_llm 软观测分层——文件头自我声明的不算失真）。
- **收录但未逐条展开**的泛化条目：DIV-43（注释行号系统性漂移，给 4 个已亲验实例代表全类）。

### 5.3 每条亲验记录（本次会话当场执行）

- 文档侧：CLAUDE.md（:60,127-136,135,148,160）、README.md（:185,226,326）、5 处 sunset notice 行号、autonomy req（:32,123-137,133-137,411,438,590）、hardening req（:146,152,308）、evolution req（:254）、digest req（:86,102）、tasks.md 顶注、task-status-manifest.json 头部、audit-status-manifest.json / deepread-verify-result.json 摘要、DEPLOYMENT-STEPS.md（:320,332）、product-modules.md、reply-prompt-slimming（:9,95）、full-system-test-remediation plan（:5,20）、real-task-runbook（:452-569）、deploy.sh、.env.example（rg 差集）。
- 代码侧（全部 Read/Grep 当场确认）：guards.rs:1-7 与 `rg 'fn enforce_'`、runtime.rs:284-292,339,414-418,459-492、gates.rs:37-44,88-105,152-219,244-266、gateway.rs:2912-2937,3905-3934,6166 与 apply_agent_updates 调用点、decision.rs:1-12,460,590,1321、prompts.rs:1268-1275,1329-1344、webhooks.rs:1455-1469,1585-1650、indexes.rs（dedupe_key 全命中、:2618、:164-169）、models.rs:905-965,1790-1807,1985-2000,3644,3753,5855-5872,6012-6019、m028:110 与 mod.rs:415、review/mod.rs:4385-4420、outbox_dispatcher.rs:915-930,963-970,2725-2752、outbox.rs:640-660、escalation/mod.rs:225-268、escalation/ledger.rs:681,758-800、quiet_hours.rs:112-146、memory.rs:356-530,570-585,3423 与 revoke 链路、knowledge_agent.rs:1-12,38-45,130-148,433-532、knowledge_router.rs:771-784,1123-1150、block_parser.rs:18-26,88-132、gap_signals.rs:8-14,141,198、supervisor.rs:1-55、llm.rs:285-292,983-990、post_release.rs:1-8,55-78、release.rs:115-117、runtime_flag.rs:44-84、auto_release.rs:3,40-44,283-340、evolution/mod.rs:60-68,125-130,225-235、replay.rs:9,242-245、cohort.rs:108-144,208、threshold.rs:110-126,160-212、shared.rs:1090-1098、guides.rs:590-604、admin_ops_versions.rs:1232-1240、admin_taxonomy_candidates.rs:188-196、observability.rs:1033-1048,1338-1346,1405-1440、routes/mod.rs:974,1080-1096 与 include 名单 rg、chunk_locks.rs:395-413 与调用点 rg、chunk_revisions.rs:174-244、crud.rs:790-838、tasks.rs:718-726、types.rs:855-872、chat_tool_loop.rs:12-18、chat.rs revoke 分支 rg、db/mod.rs:218-219、config.rs:5-7,195,508,655-661、check-baseline.sh:8,28,84 / .ps1:20、scripts/deploy/candidate_smoke.py:280 等；前端 main.tsx:8-22、api.ts:115-124、labels.ts:58-73,275-295、steward.tsx:1206、JudgmentBar.tsx:72-92（含 wa.authed/openEventSource/REVIEW_CATEGORY_LABELS 的全库 rg）；测试 revision_recheck_action_gate.rs、memory_card_write_occ.rs 全文、worker_reclaim.rs:55-72、ingest_worker_smoke.rs 头部+测试名清单、domain_profile_e2e.rs:450-482、memory_card_invariants.rs:716、digest_inbox.rs:714-740 与 :522,538。

### 5.4 诚实边界（未收录/待重验声明）

1. **17-Q7（SR-178 真模型红线硬门允许零样本绿）**：涉及 6 个 nightly 文件的 skip/产物语义逐一对码，本次未亲验，不入主表；引用前须专项核验修复态。
2. **17-Q10（runbook ISSUE-012 知识红线三层联动失效的终态）**：三选一修复方案具体采纳哪条无文档直接回答、需对码专项终裁，本次未做，不入主表。
3. **19 号"biz-test 旧 cleanup 从未成功清理"与"management 二次 confirm 幂等已实现"**：均为推断级（原记录已标注），未亲验，不入主表。
4. 本表行号基于 2026-08-13 含未提交改动（47 文件）的工作区；提交/继续开发后行号会漂移——**引用任何 file:line 前仍须当场重验**（CLAUDE.md 红线，本表自身同样适用）。
5. 各源记录中"性能/风格/行为缺陷"类条目未消失，只是不属于本表范围；全局行为缺陷以 `PROJECT_UNDERSTANDING_LEDGER.md` 第五部分为准。

---

## 6. 终态核销表（2026-08-13 优化工程后 · 线 G 授权追加）

> **性质**：本节为 2026-08-13 文档准确性收敛波（线 G / Task G4）授权追加的终态账本，§1–§5 正文一字未动。
> **核证方式**：每条终态于线 G 会话在 `fix/dependency-security-remediation @ 3db6cf6` 基线（已含优化线 A–E + S5 全部合并）上重验——凡标"仍存"的条目，其锚点均经当场 Grep/Read 确认仍在（DIV-26 的机械拼接侧、DIV-31/32 的测试体、DIV-56 的可达性、DIV-62 的 as_document 键集四处为"§1 当日亲验 + 波次未触及对应文件"推定，未二次逐行重读）；凡标"已修/失效"的条目，均当场确认原锚点已消失或已更正。
> **"线 F"条目说明**：CLAUDE.md / README.md 归同波并行的线 F 文件集（其任务书 F1/F2 明确逐条覆盖），本表按计划归属预记，**最终以线 F commit 为准**。
>
> **终态分布：已修 16（优化波次 9 + 线 F 7）／ 文档已标注 10 ／ 仍存在 43 ／ 已过时失效 2。高严重度 4 条全部收口（已修 2 + 已标注 2），无"仍存"高严重度条目。**

### 6.1 已修（16 条）

| # | 处置 |
|---|---|
| DIV-01 | 线 F（F1：CLAUDE.md 五闸表述→分数闸；本波并行，以线 F commit 为准） |
| DIV-04 | 线 A commit bfc0395：manual_send 保守语义定案，删除不可达豁免死代码、两处注释改为如实描述 |
| DIV-05 / DIV-06 / DIV-07 | 线 F（F1：webhook 流程图 / tool-calling 知识归属 / macOS 环境表述） |
| DIV-16 | 线 C commit 66dd451：双脑第二路 parse 失败改回退主 review（`review/mod.rs:4627` 注释与行为已一致） |
| DIV-21 | 线 C 清理（终裁 10-1）：`schedule_post_release_review` 死函数已删除，`post_release.rs` 头注改为如实描述事务内直插 |
| DIV-22 | 线 C 清理（终裁 10-2）：`is_evolution_enabled_for` 死包装已删除，`runtime_flag.rs:19` 注释留档说明 |
| DIV-25 | 线 C：`post_release.rs:53-58` 与 threshold/significance 口径统一（`blocked_by_safety_guard` 不归因任何 gate、pressure 走 revision 口径，`five_gate_mapping_tests` 钉死） |
| DIV-29 | 线 C：两个空壳测试落实——`revision_recheck_action_gate.rs`（326 行，mock 六段编排驱动真 gateway）、`memory_card_write_occ.rs`（197 行，4 路并发 OCC 经公共 API） |
| DIV-37 / DIV-38 / DIV-39 | 线 F（F2/F1：README 路由与迁移数、CLAUDE 基线脚本行号——F 侧另行实测现值） |
| DIV-40 | 线 A：`gateway.rs:2945` 注释改为"旧单发 prompt `user.reply.task` **曾**提供 tool_calling 形态"（历史语态，退役 key 残留清除） |
| DIV-41 | 线 A：去抖注释因果对象已更正（`gateway.rs:3969` 现明确 `apply_agent_updates` 是"本 run 落库后才会调度的异步"投影侧调用） |
| DIV-59 | 注释已重写：`ledger.rs:763-765` 的 dead_code 注记现挂在 `derive_sediment_title_fallback`（真实的待接线函数）上，`derive_sediment_title` 本体在 :681 的生产调用不再与注释矛盾 |

### 6.2 已过时失效（2 条 · 偏差本体消失）

| # | 说明 |
|---|---|
| DIV-11 | `evolution::auto_release` 模块被线 C 整体物理删除（tick 无自动发布路径、`routes/evolution.rs:759-762` 拒写子闸 true）——spec R9.6 与代码现状**重新一致**，契约冲突（SR-180）关闭；evolution requirements 头部已加 2026-08-13 终态注记（线 G / G1） |
| DIV-23 | 被指认的 `auto_release.rs:287-289` 注释随文件整体删除而不复存在 |

### 6.3 文档已标注（10 条 · 历史存档不改写正文，防误导指针已就位）

| # | 标注位置 |
|---|---|
| DIV-02 | 代码侧已收口（线 B B7：种子不再种入，`prompts.rs:2652` 守卫测试钉住；生产恒 fast.task）；autonomy requirements 头部 2026-08-13 现状注记（线 G / G1）声明九字段协议退役；`agents.html:116` 展示 key 已改 fast.task（线 G / G3）。spec 正文与 2026-07-13 slimming spec 按历史存档原则保留原文 |
| DIV-03 | 五处 sunset notice 所在文件全部追加"2026-08-13 现状注记"（autonomy req/design、hardening req/design、evolution tasks；另 evolution req、digest req 同批加注——线 G / G1，7 文件） |
| DIV-08 | autonomy requirements 头注声明第四 PBT 现为 `wiki_chunk_revision_pbt`（G1）；正文四处不改 |
| DIV-09 | autonomy requirements 头注总括"正文不再作对照源、逐条偏差见本表 §2.2"（G1）；R1.4/R1.5 正文不改 |
| DIV-10 | hardening requirements 头注②：spec 数学矛盾措辞保留作历史、实现已升级为淘汰迁移 + `coreFactEvictions` 审计（G1） |
| DIV-12 | digest requirements 头注：基线 78 为旧值、现行 350/33 以 check-baseline.sh 为准（G1） |
| DIV-13 | 既有标注充分：三 spec tasks.md 顶部 SR-179 权威注记 + `task-status-manifest.json` 自声明，维持 |
| DIV-14 | 既有标注充分：同目录 `audit-status-manifest.json` 权威改判 inconclusive，维持（47 域 v2 重跑仍未发生，归属见 20b-G4） |
| DIV-69 | hardening requirements 头注③：`human_handoff_success_rate` 属禁词前史措辞、按字面实现必被 CI lint 拦截（G1） |
| DIV-70 | 既有标注充分：runbook 顶部 sunset notice（不再随主线更新）+ 本表 §2.3 反向索引，维持 |

### 6.4 仍存在（43 条 · 全部为中/低严重度，含理由与归属）

**A. 后端注释漂移 / 命名失真（26 条，低风险清理类）**——本波两线文件集均不含 src/**（历史存档原则之外的代码注释修正属代码改动，非文档波职权）。锚点均于线 G 当场复验仍在：
DIV-17（`quiet_hours.rs:140-153` doc 描述三级链、函数体 :154-163 仍 workspace-only）、DIV-18（`escalation/mod.rs:233` "pending 台账可处置"实际未建）、DIV-19（`DocEntry` 仍零构造）、DIV-20（`knowledge_agent.rs:436` depth 计数不存在）、DIV-24（`evolution/mod.rs:225-228` "prompt 走 placeholder"已过时——prompt 影子重放已真实接线，**波次引入的新漂移面**）、DIV-26（guide readableChanges 机械拼接）、DIV-27（`admin_taxonomy_candidates.rs:191` "无 workspace_id"失实）、DIV-28（`observability.rs:1342` sweep 命中率旧口径）、DIV-42（`webhooks.rs:1586` 索引行号 55-63→813+）、DIV-43（注释行号系统性漂移——结构性长期项，治本是"注释不写行号"约定）、DIV-44（幽灵迁移引用）、DIV-45（task status 注释清单缺 `committing`）、DIV-46（provenance 注释缺 `lesson_promotion`）、DIV-47（m028 日志 id）、DIV-48（`decision.rs:1-9` 头注仍称 user.reply.task/decide_reply）、DIV-49（`supervisor.rs:3` "8 个 worker"→实 16）、DIV-50（llm.rs 层数口径）、DIV-51（memory 镜像 cap 6≠20）、DIV-52（`knowledge_router.rs:1195` fire-and-forget 注释 vs 顺序 await）、DIV-53（block_parser "行首"语义）、DIV-54（gap_signals "8 类"→12）、DIV-55（`ensure_evolution_indexes` 名不副实）、DIV-56（cohort 空 contact 注释分支不可达）、DIV-57（`evolution/mod.rs:61` skeleton 日志）、DIV-58（`proposed_raw` 命名）、DIV-60（tasks.rs:722 "第 n 次重试"文案）、DIV-63（`decision_created_ms` 名实不符）。
**归属**：建议合并为一次 `chore(comments)` 清理波（后端 owner），DIV-24 优先（波次新引入）。
> **处置记录（2026-08-14 chore 尾波 `9d22429`）**：A 组 26 条全部关闭——漂移注释逐条对齐实际行为，被触及文件顺势采纳"注释不写行号"约定（DIV-43 治本方向的首批落地）。合并验证 lib 2562/0。

**B. 测试覆盖幻觉（4 条）**：DIV-30（worker_reclaim 测试名与断言不符，HP-1 回收仍无端到端守护）、DIV-31（ingest 正向成功链零覆盖——worker 默认关）、DIV-32（delete_forbidden 未发删除请求）、DIV-71（digest_inbox 禁词防御锁旧文案副本）。**归属**：测试波 owner；DIV-30 与 28 号 §4 无守护清单同源，启用 ingest 前必须补 DIV-31。
> **处置记录（2026-08-14 chore 尾波，4 条全闭）**：DIV-30 `fac9fe9`（真 stale-task 回收端到端，3/0 Docker 亲跑绿）；DIV-31 `a0ca8c7`（经 `claim_due_source_for_redline`/`finalize_claimed_content_for_redline` 现成 redline 入口驱动真实 claim→finalize，断言 draft+needs_review 恒成立、last_fetched_at/failure_streak/ingest_count 全链，5/0 绿）；DIV-32 `6cff3ce`（直调 handler 发真删除请求，BadRequest 拒 active + draft 对照放行，1/0 绿）；DIV-71 `17b45bc`（治本：9 条 inbox 文案抽 `COPY_*` 常量，生产与禁词测试同源，另补测试原漏的 3 条文案）。

**C. 前端死代码 / 文案（5 条）**：DIV-64（wa.authed 只写不读）、DIV-65（openEventSource 死代码）、DIV-66（steward "客户 ID"实为账号 ID）、DIV-67（JudgmentBar 类型断言缺字段声明）、DIV-68（REVIEW_CATEGORY_LABELS 死代码同键不同文）。**归属**：前端 chore 波（均为一行级修正/删除）。
> **处置记录（2026-08-14 chore 尾波 `0551343`，5 条全闭）**：DIV-64 删 `wa.authed` 全部 5 处写入（登录态在 http-only cookie，顺修注释中不存在的 forceLogout）；DIV-65/68 死代码净删（全仓零消费亲验）；DIV-66 placeholder 与表单说明两处"客户 ID"→"账号 ID"；DIV-67 **半翻案**：`OperationHealth` 三键补声明（后端 `shared.rs` 实锤下发）成立，但 `Contact.lastConversationMode` 经亲验后端 ApiContact **当前不下发**（幽灵键）——本表"类型定义落后于后端实际下发"的描述对该键不成立，前端已补声明并如实标注幽灵状态，删除两处 `as` 断言。前端 750/750 + build 绿。

**D. 护栏名单缺口（1 条）**：DIV-36（死路由 tripwire include 名单缺约 11 文件；另线 B 登记 `KNOWN_NON_ROUTE_HANDLERS` 的 `apply_update_chunk` 滞留条目待仲裁）。**归属**：后端 owner，与 A 组同批。

**E. 本波未授权文件（4 条，中严重度居多，建议单独小 PR 尽快处置）**：
DIV-15（`docs/DEPLOYMENT-STEPS.md:320,332` `db.agent_runs`→`agent_run_logs`——按文档排障会误判"系统没跑"，**运维文档 owner**）、DIV-33（plan 引用不存在的 findings 文件——superpowers 历史存档，接受现状或补一行勘误，档案 owner）、DIV-34（`deploy.sh` 整体过时，以 `scripts/deploy/` 为准——建议头部加弃用横幅或删除，**部署 owner**）、DIV-35（`.env.example` 缺 POST_DECISION_*/SILENCE_SIGNAL_* 两族变量——运维排障缺文档入口，**后端 owner**）。

### 6.5 本节核销后的使用约定

- 读 §1 主表任一条目前，先查本节终态；标"已修/失效/已标注"的条目不再构成防误导风险，标"仍存"的条目继续按 §2 反向索引对待。
- 本节由线 G 一次性写入（2026-08-13）；后续波次关闭"仍存"条目时在对应行追加处置记录，不改写本节既有文字。
