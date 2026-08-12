# 疑点终裁 I（01-06 号记录，核证日期 2026-08-13）

> 对象：`project-understanding/` 01、02、03、04、05、06 六份深读记录"偏差与疑点"节的**全部 73 条**条目（无一条带"主会话已抽查/已核证"标注，故全部纳入）。每条终裁均基于 2026-08-13 当场亲读源码（函数全文 + 调用方），行号为当日工作树行号（与源记录同一工作树，未提交状态一致）。
>
> 结论三分类：**【属实·缺陷】**（附严重度/触发条件）/ **【不成立】**（附反证）/ **【属实·设计】**（= 属实·但为刻意设计或需产品决策；无法从代码内文档确证意图时明确标"仍存疑"）。
>
> **总计：73 条终裁 = 属实·缺陷 20 / 不成立 5 / 属实·设计（含需产品决策/事实确认）48。**

---

## 1. 疑点汇总表

| ID | 来源 | 一句话 | 终裁 |
|---|---|---|---|
| 01-1 | 01 §5.1 | 主去抖注释仍写"apply_agent_updates 之前"，内联调用已迁移 | 属实·缺陷（低，注释过时） |
| 01-2 | 01 §5.2 | deferred_wake 醒来任务受 daily_limit 等约束且被拦走 cancel | 属实·设计（前提大半过时：该 kind 仅 legacy 残留，生产 wake 走 durable/Inbound 天然豁免） |
| 01-3 | 01 §5.3 | `should_refresh_context` 恒 false，上下文刷新机制停用 | 属实·设计（审计形态残留，非行为缺陷） |
| 01-4 | 01 §5.4 | `promote_risks` 两处空消费是抑制告警 | 属实·设计（事实成立；finalize 确有消费，非死值） |
| 01-5 | 01 §5.5 | rewrite/revision 后 used_knowledge_ids 直赋不走 KB-01 清空 | 属实·设计（恒 Full 档故合理；留守护点） |
| 01-6 | 01 §5.6 | 双重 normalize_decision_runtime 是否有依赖第一次的中间读 | 不成立（第一次调用恒 no-op，无行为影响） |
| 01-7 | 01 §5.7 | B-01 兜底去抖尾窗客户可能收两批 | 属实·设计（注释自认的已知产品取舍） |
| 01-8 | 01 §5.8 | 管理发送二道 precheck 返回 review_approved:true 有歧义 | 属实·设计（字段各述其阶段，仅命名歧义） |
| 01-9 | 01 §5.9 | `blocked_by_safety_guard` 状态串三来源复用 | 属实·设计（闭集复用，排障需结合事件） |
| 01-10 | 01 §5.10 | 非文本过渡 authorize 失败后 outbox 条目遗留，依赖 dispatcher fence | 不成立（作为缺陷：dispatcher 侧 Stale→cancel 闭环已亲验） |
| 01-11 | 01 §5.11 | `occurrences` 字段语义漂移（台账 reconcile 而非本行自增） | 属实·设计（幂等台账契约，模块文档已声明） |
| 01-12 | 01 §5.12 | settle 的 performance 写入无状态谓词恐覆盖终态 | 不成立（只写 `.performance` 子键；run log 缺失时静默 no-op） |
| 01-13 | 01 §5.13 | ack 占位 source_kind 用 trigger.kind() 双口径 | 属实·设计（两口径均不入 PROACTIVE 闭集，注释已声明） |
| 02-1 | 02 §6.1 | `ensure_evolution_indexes` 内容远超 evolution | 属实·缺陷（低，纯组织/命名误导） |
| 02-2 | 02 §6.2 | typed 模型外的 raw 动态字段清单 | 属实·设计（事实核证成立） |
| 02-3 | 02 §6.3 | 闭集 9 值 vs 注释/测试 8 值（缺 committing） | 属实·缺陷（低，文档/测试滞后） |
| 02-4 | 02 §6.4 | m028 日志 id 与注册 id 不一致 | 属实·缺陷（低，日志检索） |
| 02-5 | 02 §6.5 | 多处注释行号漂移 | 属实·缺陷（低，注释坐标过期） |
| 02-6 | 02 §6.6 | provenance.source 存在闭集外第五值 lesson_promotion | 属实·缺陷（低，模型注释未更新） |
| 02-7 | 02 §6.7 | operation_state 由 customer_stage 派生（原"待亲验"） | 属实·设计（已亲验 C2 派生逻辑成立，与 CLAUDE.md 一致） |
| 02-8 | 02 §6.8 | taxonomy workspace_id serde default 反序列化时读 env | 属实·设计（隐式环境耦合成立，暴露面已被 m032 收窄） |
| 02-9 | 02 §6.9 | Contact 多个 Option 字段无 `#[serde(default)]` | 不成立（缺陷不存在：bson 对 Option 缺键天然回 None，测试佐证） |
| 02-10 | 02 §6.10 | `operation_knowledge_items` 幽灵集合 | 属实·设计（历史残留，对新代码无影响） |
| 02-11 | 02 §6.11 | m009 与 m034 promote/demote 顺序相反 | 属实·设计（两处注释各自论证，皆有意） |
| 02-12 | 02 §6.12 | models.rs:1802 引用的迁移不在注册表 | 属实·缺陷（低，注释指涉不存在的迁移） |
| 02-13 | 02 §6.13 | 索引/字段大小写陷阱清单 | 属实·设计（历史事故沉淀，抽查成立） |
| 02-14 | 02 §6.14 | lessons_learned 无 typed 模型、review_status 闭集仅迁移侧记录 | 属实·设计（routes 侧为实际 schema 所有者，口径一致） |
| 02-15 | 02 §6.15 | 记录基于未提交工作树 | 属实·设计（事实，git status 佐证） |
| 03-D1 | 03 §5.D1 | dedupe 索引注释坐标 :55-63 过期 | 属实·缺陷（低，实际在 indexes.rs:810-822） |
| 03-D2 | 03 §5.D2 | 无法 decode 的 pending/deferred 行使两个 worker 每轮 tick 中止 | 属实·缺陷（**中**，毒丸行；触发需异常写入） |
| 03-D3 | 03 §5.D3 | quiet-hours 加载粒度分裂（contact 级 vs workspace 级） | 不成立（`_contact_id` 参数被忽略，两加载器完全等价） |
| 03-D4 | 03 §5.D4 | A-03/A-04/A-05 已知边界 | 属实·设计（三处注释自认，亲验存在） |
| 03-D5 | 03 §5.D5 | 乱序消息仍标 materialized | 属实·设计（语义自洽，命名易误读） |
| 03-D6 | 03 §5.D6 | retry 事件文案"第 n 次重试"实为"第 n 次执行" | 属实·缺陷（低，仅文案） |
| 03-D7 | 03 §5.D7 | claim filter status 注入与 $or 的隐式耦合 | 属实·设计（行为正确，守护点） |
| 03-D8 | 03 §5.D8 | reply_rate O(N) 查询 + 每 30s 全账号无条件 insert | 属实·设计（正确性无损的性能观察） |
| 03-D9 | 03 §5.D9 | legacy 去抖层无生产调用方 | 属实·设计（待退役保留兼容，亲验无生产调用） |
| 03-D10 | 03 §5.D10 | webhook 低延迟唤醒为裸 tokio::spawn 无 supervisor | 属实·设计（250ms worker 兜底存在） |
| 03-D11 | 03 §5.D11 | 聚合日期/day_bucket 按 UTC 日截断 | 属实·设计（注释自认粗糙够用） |
| 04-1 | 04 §5.1 | decision.rs 模块 doc 提及不存在的 `decide_reply` | 属实·缺陷（低，文档漂移） |
| 04-2 | 04 §5.2 | render_transaction_facts_sections 第三段恒被丢弃 | 属实·缺陷（低，演进残留浪费渲染） |
| 04-3 | 04 §5.3 | second reviewer parse 失败拉闸整 run | 属实·设计（行为确认；意图无文档，**仍存疑**，需产品决策） |
| 04-4 | 04 §5.4 | fast 契约不校验 why_should_reply、R1.5 主链不可达 | 属实·设计（行为确认；是否刻意精简**仍存疑**；调用点注释漂移属实） |
| 04-5 | 04 §5.5 | operation_state_exists fail-open 与 check fail-closed 并存 | 属实·设计（#155 注释论证成立，隐式契约守护点） |
| 04-6 | 04 §5.6 | quoted_product_ids 双轨语义留存 | 属实·设计（文档已注明，无编译期防护） |
| 04-7 | 04 §5.7 | runtime.as_document 缺 4 组新字段 | 属实·缺陷（低，reviewer 硬参数注入完整性漂移） |
| 04-8 | 04 §5.8 | 开放世界门依赖 merge 写入的计数、缺失按 0 | 属实·设计（语义一致的结构性耦合点） |
| 04-9 | 04 §5.9 | light reviewer 注入 evidenceExcerpts 与 decision 侧剥离口径不一 | 属实·设计（口径不一致确认；意图无注释，**仍存疑**） |
| 04-10 | 04 §5.10 | decision_requires_knowledge 容忍闭集外 "knowledge_required" | 属实·设计（兼容垫；promote 清空后几乎不可达） |
| 05-1 | 05 §5.1 | manual_send 被二次门 not_managed_at_send 取消，下游豁免死代码 | 属实·缺陷（低，两门语义矛盾+死代码；fail-safe 方向；需产品定夺） |
| 05-2 | 05 §5.2 | decision_created_ms 实为 entry.created_at | 属实·缺陷（低，命名与取值不一致+窄窗漏判 stop） |
| 05-3 | 05 §5.3 | 骚扰门拦截时连 pending 台账都不建，注释相反 | 属实·缺陷（低-中，被拦请示无痕） |
| 05-4 | 05 §5.4 | effective_quiet_hours_enabled doc 与函数体不一致 | 属实·缺陷（低，文档漂移） |
| 05-5 | 05 §5.5 | 两套 quiet-hours 实现并存 | 属实·设计（行为等价，结构性重复） |
| 05-6 | 05 §5.6 | 媒体/名片 priority=20 仅靠 aging 保底 | 属实·设计（有意文本优先；饥饿风险观察成立） |
| 05-7 | 05 §5.7 | delivery_unknown 请示卡不进超时改派扫描 | 属实·设计（保守设计；滞留无告警属实，需产品决策） |
| 05-8 | 05 §5.8 | interpret_principal_reply LLM 错误直达 webhook，领导消息可丢 | 属实·缺陷（低-中，领导回复须重发） |
| 05-9 | 05 §5.9 | 名片/媒体 MCP 入参为占位形态 | 属实·设计（注释自认；上线前必须对齐） |
| 05-10 | 05 §5.10 | 成功核验要求 newMsgId 为 string，数值型漏判成功 | 属实·设计（安全侧偏差，观察点成立） |
| 05-11 | 05 §5.11 | derive_sediment_title 的 dead_code 注记过期 | 属实·缺陷（低，注释腐化——已有生产调用点） |
| 05-12 | 05 §5.12 | pacing 闸对内部通知同样生效 | 属实·设计（1-4s 影响极小） |
| 06-1 | 06 §5.1 | update_run_envelope_terminal 不做 lifecycle 转换 CAS | 属实·设计（权衡确认；吸收性依赖调用方纪律，**仍存疑**是否该加 CAS） |
| 06-2 | 06 §5.2 | extra 镜像 deprecatedFacts cap=6 vs typed 20 | 属实·设计（显式写死；6≠20 理由无文档，**仍存疑**） |
| 06-3 | 06 §5.3 | append-only 降级路径用默认 runtime | 属实·设计（不对称属实，影响有限） |
| 06-4 | 06 §5.4 | activate 与 runnable_filter 的 prepared 语义冗余 | 属实·设计（竞态窗口语义安全） |
| 06-5 | 06 §5.5 | strip_known_tags 大小写敏感仅剥 8 子串 | 属实·设计（模块文档声明的既定取舍） |
| 06-6 | 06 §5.6 | reaction turn_index 是全量 inbound count | 属实·设计（best-effort 观测坐标系） |
| 06-7 | 06 §5.7 | shadow 指纹全量段扫描无 limit | 属实·设计（正确性优先的性能观察） |
| 06-8 | 06 §5.8 | 同维冲突 warning 泄漏数组下标而非 id | 属实·缺陷（低，审计回指不稳定） |
| 06-9 | 06 §5.9 | explicit_stop_intent 否定标记先于 DIRECT 判定 | 属实·设计（fail-open 到 LLM 的刻意保守） |
| 06-10 | 06 §5.10 | 无 claim 整理路径候选/事件非事务 | 属实·设计（注释自认兼容路径，弱一致由去重吸收） |
| 06-11 | 06 §5.11 | memory_card_has_signal 把 extra 镜像也算信号 | 属实·设计（防覆盖真数据；清空记忆需连 extra） |
| 06-12 | 06 §5.12 | post_decision 双层锁（review 锁 + contact lease） | 属实·设计（行为自洽已亲验） |

---

## 2. 逐条终裁详情

### 01 号 `gateway.rs`（13 条）

**01-1【属实·缺陷】注释过时（apply_agent_updates 已迁移）**
- 亲读：gateway.rs:3900-3989（主去抖检查与注释全文）、6166-6195（`apply_agent_updates` 函数头）、4660-4719（授权后 `$max last_agent_run_at` 锚）；Grep `apply_agent_updates` 全仓调用方。
- 证据链：gateway.rs:3915/3922 注释仍写"必须在 apply_agent_updates 之前——后者无条件把 last_agent_run_at 推到 now"；但 `src/` 内 `apply_agent_updates` 仅被 post_decision.rs:935、post_decision.rs:1109（投影 worker）调用，inner 主路径无内联调用；主路径推进频控锚的是 gateway.rs:4673-4685 的授权后 `$max` 写。同类：gateway.rs:6179-6182 `apply_agent_updates` 开头仍 `$set last_agent_run_at=now`（投影回放时二次推锚，语义相容——`$set` 晚于授权锚，仅进一步延后 rate-limit 窗口）。检查本身仍必要（在写 decision_review/投影/锚之前放弃过时生成）。
- 严重度：低（纯文档）；触发条件：无（不影响运行）。

**01-2【属实·设计｜前提大半过时】deferred_wake 受 daily_limit 约束且被 cancel**
- 亲读：gateway.rs:5245-5366（precheck 全文）、5618-5622（`daily_limit_applies_to`）、2379-2435（拦截分支）、330-359（durable 任务以 `AgentTrigger::Inbound` 进网关，gateway.rs:351）、5739-5768（cancel_task）；quiet_hours.rs:18-20；webhooks.rs:1586-1642（主入站路径）、803-908（reconcile）、656-799（workspace 策略 reconcile）、30（HANDOFF_DEFERRED 常量注释）；config.rs:480/490；Grep `DEFERRED_INBOUND_REPLY_KIND` 全仓。
- 证据链：(a) 疑点描述的代码路径**属实**——`is_deferred_wake`（gateway.rs:5263-5267）只豁免 quiet_hours 门（5320）与 context_changed（5347），daily_limit（5292-5296）/cooldown（5269-5273）/rate_limited（5283-5288）照拦，被拦走 `cancel_task`（2391-2393，仅 quiet_hours_deferred 走 reschedule）。(b) 但该 kind **已无创建点**：quiet_hours.rs:18-20 注释明言"旧版……新写入统一使用 DURABLE_INBOUND_REPLY_KIND"；全仓 Grep 无插入点；生产静默路径（webhooks.rs:1608-1623）与崩溃恢复（webhooks.rs:883-897）都物化 **durable inbound task**（run_at=醒来时刻），它经 `handle_durable_inbound_reply_task` 以 **Inbound** 触发进网关（gateway.rs:351）→ daily_limit 天然不适用（仅 FollowUp，gateway.rs:5621-5622）。(c) legacy 残留行在 workspace 策略编辑时被并入 durable 义务（webhooks.rs:716-737、770-796"merged_into_reply_obligation"）；若在此之前被 daily_limit cancel，义务不会自动重建（HANDOFF_DEFERRED 是"legacy read compatibility only"，webhooks.rs:30），须等下一条入站刷新 durable 行（webhooks.rs:178-204 的 refresh 不看 status）或策略编辑。
- 结论：对现行生产路径疑点**不成立**；对 legacy 残留行为**属实但影响面极窄**（滚动升级期一次性风险）。归类：属实·设计（过渡兼容状态）。附注：durable 任务（Inbound 触发）被 rate_limited（默认 20s，config.rs:480）/cooldown 拦时同样 cancel 不重排——"短时间内已触达，跳过本次自动发送"（gateway.rs:5286）是既有设计（多个测试显式绕开它，如 tests/real_llm_ops_smoke.rs:1318-1321、tests/roleplay_emotional_companion_e2e.rs:409），义务由下一条入站复活。

**01-3【属实·设计】`should_refresh_context` 恒 false**
- 亲读：gateway.rs:2479-2510（inner 快照段）、720-756（管理发送 planner）；guards.rs:48-68。
- 证据链：gateway.rs:2496 `let should_refresh_context = false;` 硬编码；run log `context.refreshed` 因此恒 false（gateway.rs:3934/3964 等）；唯一 `context_needs_refresh: true` 在管理发送 planner 硬编码（gateway.rs:749）；`planner_from_decision` 恒产 `context_needs_refresh: false`（guards.rs:57）。上下文刷新机制在 planner 内联化后停用、字段仅存审计形态——记录判断准确。非行为缺陷；可作清理项。

**01-4【属实·设计】promote_risks 空消费行**
- 亲读：gateway.rs:2953-2961、3580-3594；Grep `promote_risks` 全文件。
- 证据链：gateway.rs:2961 `let _ = &mut promote_risks;`、3586 `let _ = promote_risks;`（带注释"后续如需进一步审计可再消费"）；真实消费在 finalize：gateway.rs:3200 与 3441 的 `promote_risks.clone()`；rewrite/revision 重赋值在 3119/3421。两行 suppress 是因为末次赋值（3421）后无读取。非缺陷。

**01-5【属实·设计】rewrite/revision 的 used_knowledge_ids 直赋**
- 亲读：gateway.rs:3100-3151（rewrite）、3370-3404（revision）、2953-2960（KB-01 主链清空）；sufficiency.rs:110-120。
- 证据链：rewrite 恒 `PromptTier::Full`（gateway.rs:3105）后直赋 `route_used_knowledge_ids`（3127）；revision 同（3379/3395-3396）；主链走 `resolve_used_knowledge_ids`（2956-2960，仅 Full 档保留 route ids，sufficiency.rs:110-120 亲验）。因 rewrite/revision 恒 Full、知识确已注入，直赋与 KB-01 语义一致。守护点成立：若未来改 revision 档位需同步改此两处。

**01-6【不成立】双重 normalize_decision_runtime**
- 亲读：gateway.rs:2939-2951；guards.rs:42-68（`normalize_decision_runtime` + `planner_from_decision` 全文）；types.rs:1688-1705（RunPlannerResult 结构）；gateway.rs:2498-2503（initial_planner 构造）。
- 反证：`normalize_decision_runtime` 唯一效果是 `decision.memory_write_score==0 && operating_memory_update 非空` 时回填 `planner.memory_change_importance`（guards.rs:43-45）。initial_planner 用 `..Default::default()`（gateway.rs:2502），`memory_change_importance` 默认 0 → 第一次调用即使命中条件也是 0→0 的 no-op。`planner_from_decision` 读 `decision.memory_write_score`（guards.rs:58），不依赖第一次归一结果。第二次调用（2951）以 planner 的 clamp 值回填（仍等于原 score）。结论：无任何中间读依赖、无行为影响，纯冗余（可删第一次调用）。疑虑解除。

**01-7【属实·设计】B-01 兜底去抖尾窗**
- 亲读：gateway.rs:4217-4283（兜底去抖 + B-01 注释全文）。
- 证据链：gateway.rs:4228-4234 注释逐字自认"极窄尾窗……两批 segment 幂等 key 不同不互相去重→客户可能收两次……列为已知产品取舍待专项"。官方备案的取舍，非未知缺陷。

**01-8【属实·设计】管理发送二道 precheck 的返回语义**
- 亲读：gateway.rs:988-1042（二道 precheck 分支全文）；routes/management.rs:2304-2326（唯一调用方）。
- 证据链：gateway.rs:1034-1041 返回 `review_approved: true, gateway_status: final_precheck.status`——review 确实通过、gateway 确实拦了，两字段各述其阶段。唯一调用方 management.rs:2325 `Ok(json!(response))` 整体序列化给管理 Agent 工具结果，consumer 是 LLM/前端展示，不据 `review_approved` 做发送判定。歧义仅在字段命名层面，无行为错误。

**01-9【属实·设计】`blocked_by_safety_guard` 三来源复用**
- 亲读：gateway.rs:4175-4216（relay 泄漏守卫）、4530-4574（"应发未入队"兜底）；review/gates.rs:775-785（finalize BlockedBySafetyGuard 分支之一）。
- 证据链：同一状态串产自 (a) finalize 硬门（gates.rs:779-782 等）、(b) relay 泄漏守卫（gateway.rs:4198）、(c) 兜底缺省 `delivery_block_status.unwrap_or("blocked_by_safety_guard")`（gateway.rs:4544）。闭集设计的必然结果；排障须结合 `blocked_review` 事件与 outbox 记录——记录的提醒成立。

**01-10【不成立（作为缺陷）】非文本过渡 authorize 失败的 outbox 遗留**
- 亲读：gateway.rs:1495-1579（enqueue→authorize→review 状态翻转全文）；outbox_dispatcher.rs:410-626（TaskSendAuthorization 全链）。
- 反证：authorize CAS 失败时 review 置 `stale_task_claim`（gateway.rs:1525-1541），outbox 条目遗留。dispatcher 侧 `task_send_authorization`（outbox_dispatcher.rs:454-559）：review.status 非 {outbox_enqueuing, outbox_enqueued, sent} → Stale（477-484）；即便 review 状态写失败，task 的 claim_token 已被新代际替换 → `task_claim_token != binding_token` → Stale（429-433）；Stale → `cancel_entry`（615-623）。跨文件依赖成立、闭环收敛为 canceled，无发出风险。

**01-11【属实·设计】occurrences 语义**
- 亲读：gateway.rs:6082-6163（upsert 全文）；projection_observations.rs:1-138（全文）。
- 证据链：`$setOnInsert occurrences:0`（gateway.rs:6108）后由 `record_and_count`（projection_observations.rs:25-70，严格 (entity,run) 台账）+ `reconcile_stages`（75-111，`occurrences = $max(现值, baseline+ledger_count)`）回写。模块 doc（1-5 行）明言"aggregate 由台账 reconcile、崩溃自愈"。语义确如记录所述且有文档，读侧照 doc 理解即可。

**01-12【不成立（作为缺陷）】settle 的 performance 写入**
- 亲读：gateway.rs:1845-1915（settle 收尾全文）。
- 反证：gateway.rs:1878-1885 按 `{run_id}` 仅 `$set gateway_result.performance` 子键，不触碰 status/lifecycle 等其余字段，无覆盖终态风险；run log 未创建时 matched 0 静默（仅 error log 于失败分支）。与记录自答一致，缺陷不存在。

**01-13【属实·设计】ack 占位 source_kind 双口径**
- 亲读：gateway.rs:5544-5605（豁免清单+构造函数）、5607-5641（blocked/daily_limit/PROACTIVE 闭集）。
- 证据链：`build_ack_enqueue_request` 的 `source_kind: trigger_kind`（gateway.rs:5599，值为 "inbound"）；`PROACTIVE_TOUCH_SOURCE_KINDS=["follow_up","follow_up_task"]`（5630-5635），其注释明言 `inbound`/`inbound_message` 都不在内。双口径并存但不影响主动触达计数；审计查询需匹配两种字符串——记录提醒成立。

### 02 号 models/db（15 条）

**02-1【属实·缺陷】ensure_evolution_indexes 名不副实**
- 亲读：indexes.rs:2602-2618（函数 doc+签名）；Grep 函数体内集合调用点。
- 证据链：函数体（2618 起）除 evolution 五表外实含 prompt_templates（2808-2851）、knowledge_daily_reports（2863-2866）、knowledge_operator_memory（2909-2930）、admin_users（3171-3179）、ingest_sources（3281-3317）、reviewer_stats（3326-3336）、deal_attribution_stats（3345-3356）、lessons_learned（3365-3388）、agent_principal_escalations（3388-3429）。按名找索引会漏一大半——属实。严重度：低（纯组织）；触发条件：仅影响代码导航。

**02-2【属实·设计】typed 模型外的 raw 动态字段**
- 亲读（抽样亲验写入方）：post_decision.rs:592-599/609-615（`post_decision_status/next_retry_at` 写点）、webhooks.rs:1487-1488（`handoff_status` 与入站同 insert）、webhooks.rs:136（`active_task_key`）、memory.rs:2653-2669（`projection_key`）、indexes.rs:196-210（`delivery_finalize_pending` 部分索引）+ memory.rs:2191-2194（`$unset active_task_key`）。
- 结论：记录所列字段清单成立（本次抽样 5 处全中）；"models.rs 非字段全集"的警示正确。事实确认，非缺陷。

**02-3【属实·缺陷】闭集与注释/测试不同步**
- 亲读：models.rs:905-965。
- 证据链：`ALLOWED_AGENT_TASK_STATUS` 9 值含 `committing`（models.rs:921）；doc 注释历史值清单（913-917）8 值无 committing；单测 `closed_set_covers_all_known_writers`（948-963）仅断言 8 值。运行无影响（committing 在闭集内合法）。严重度：低；触发条件：无（文档滞后）。

**02-4【属实·缺陷】m028 日志 id 不一致**
- 亲读：m028_seed_conversation_mode.rs:100-116；migrations/mod.rs:408-421。
- 证据链：m028:110 `migration_id = "m028_seed_conversation_mode"` vs 注册 id `2026_06_Y2_001_seed_conversation_mode`（mod.rs:415）。仅日志检索受影响。低。

**02-5【属实·缺陷】注释行号漂移**
- 亲读（抽样）：m016_backfill_workspace_id_on_legacy_rows.rs:193-206；Grep `pub struct LlmProviderConfig`。
- 证据链：m016:200 注释写 "LlmProviderConfig(models.rs:4732)"，实际 struct 在 models.rs:6028。抽样即中，记录的"以本记录行号为准"警示成立。低（注释坐标）。

**02-6【属实·缺陷】provenance.source 闭集外值**
- 亲读：models.rs:1991-1999；m055_lesson_promotion_identity.rs:19；indexes.rs:164-175。
- 证据链：模型注释声明 `source ∈ {ai, human, rule, imported}`（models.rs:1993）；`LESSON_PROMOTION_SOURCE="lesson_promotion"`（m055:19）被 indexes.rs:172 部分唯一索引与 routes/lessons_learned.rs:47 生产写路径使用——第五合法值实存，模型注释未更新。低。

**02-7【属实·设计】operation_state 由 customer_stage 派生（原"待亲验"已核证）**
- 亲读：gateway.rs:6470-6514（C2 派生写点全文）。
- 证据链：`synced_state = 归一后 domain_signals.customer_stage || decision.operation_state`（6483-6490）；经 `check_state_transition` fail-soft（6497-6510：通过则写 operation_state+updated_at，拒绝则保旧值记审计元组）。与 CLAUDE.md/源记录描述逐字一致。models.rs 字段旁无注释是事实，但设计承载于 gateway 注释（6470-6480 五行详注），可选改进而非缺陷。

**02-8【属实·设计】default_taxonomy_workspace_id 反序列化读 env**
- 亲读：models.rs:3770-3779；Grep serde default 引用点（models.rs:3644/3753）。
- 证据链：`#[serde(default = "default_taxonomy_workspace_id")]` 两处；函数体 `std::env::var("DEFAULT_WORKSPACE_ID")`（3777-3779）在反序列化时刻求值。隐式环境耦合成立；仅影响缺字段的滚动升级窗口行（m032 已物理回填）——记录评估准确，设计上可接受。

**02-9【不成立】Contact 无 default 的 Option 字段**
- 亲读：models.rs:155-234（Contact 字段全段）。
- 反证：`nickname/remark/alias/agent_profile/memory_summary/playbook_id/…/operation_state*/cooldown_until/last_*_at` 等确为裸 `Option` 无 `#[serde(default)]`（158-233），但 serde+bson 对 `Option<T>` 缺键天然回 None——源记录自己已用 tag_trust 最小文档测试证明兼容成立。风格不一致是观察，兼容性缺陷不存在。

**02-10【属实·设计】operation_knowledge_items 幽灵集合**
- 亲读：m011_drop_legacy_sales_collections.rs 全文（35 行）；Grep 全仓 `operation_knowledge_items`。
- 证据链：m010:35-39、m011:20、m014:17 仍触达该集合；routes/knowledge 各处注释确认"已删除"（crud.rs:956-979 端点恒 400）；typed accessor 无。部分历史版本库可能残留无人维护数据，对新代码无影响——记录准确。

**02-11【属实·设计】m009/m034 顺序相反皆有意**
- 亲读：m009_prompt_template_versioned.rs:70-83；m034_reconcile_review_fixes.rs:48-61。
- 证据链：m009:76-77 "Promote first so interruption cannot leave the scope with zero current rows"；m034:53-55 "Demote the old pointer before promoting the winner…without E11000"。两个方向各有明确论证，非 bug。

**02-12【属实·缺陷】models.rs:1802 引用不存在的迁移**
- 亲读：models.rs:1795-1810；Grep `chunks_wiki_type_default` 全仓；knowledge_agent.rs:1781（`wiki_type_priority`）。
- 证据链：注释声称 "migration `2026_05_W1_001_chunks_wiki_type_default` 把所有缺字段 chunk 默认填 entity"，但全仓仅此注释一处提及该 id，MIGRATIONS 注册表无此迁移；实际机制是读侧 `wiki_type_priority(None)==entity` 兜底（knowledge_agent.rs:1781/2624 测试）。注释描述的迁移从未落地或已移除。低（误导性注释）。

**02-13【属实·设计】大小写陷阱清单**
- 亲读（抽样）：indexes.rs:1963-1980（llm_provider_configs snake 索引事故的 drop 补救 + 注释）、756-768（`outcome_events.productRef.productId` 混合路径注释）。
- 结论：抽样两处与记录一致，历史事故沉淀属实。事实确认。

**02-14【属实·设计】lessons_learned 无 typed 模型**
- 亲读：m055:88-107；routes/lessons_learned.rs（Grep review_status 相关 19/222-266/350-355 行）。
- 证据链：m055:98 强制 `review_status ∈ {pending_review, promoted}`；routes 侧同一闭集语义（lessons_learned.rs:222-232/259-266）且经 raw Document 读写（indexes.rs:3366 注释"无 typed accessor"）。迁移侧记录的形状与实际 schema 所有者一致。事实确认。

**02-15【属实·设计】工作树状态**
- 证据：会话 git status 快照显示 src/、tests/、scripts/ 大量 modified——记录声明成立，无需代码核证。

### 03 号 webhooks/tasks（11 条）

**03-D1【属实·缺陷】dedupe 索引注释坐标过期**
- 亲读：webhooks.rs:1460-1464（注释）；indexes.rs:50-62 与 805-823。
- 证据链：注释称索引在 "db/indexes.rs:55-63"，该处现为 `llm_vision_active_unique_index`（51-62）；messages dedupe 唯一索引实际在 indexes.rs:810-822（`workspace_id+account_id+dedupe_key` partial unique）。低（坐标漂移，语义无误）。

**03-D2【属实·缺陷（中）】毒丸行瘫痪两个 worker tick**
- 亲读：webhooks.rs:803-908（reconcile 全文，819-822 decode `?`）；tasks.rs:1067-1155（两个 tick 的调用序）。
- 证据链：`from_document` 失败 → `AppError::External` 直接 `?` 上抛（webhooks.rs:820-822）；`tick_inbound_replies`（tasks.rs:1069）与 `tick`（tasks.rs:1121）都以 `?` 传播——前者的扫描（1071 起）与后者 1124 行之后的全部步骤（escalation/incident/campaign reconcile、reclaim、outcome 保障、主 claim 扫描）每轮中止。该行 handoff_status 停在 pending/deferred 且无自动退出（decode 失败不会标 ignored），下一轮按 `created_at asc` 排序仍首先命中。
- 严重度：**中**（影响面：整个任务系统两个 worker 停摆，含跟进/请示/事故通道）；触发条件：低概率——正常写入路径不产生（inbound 与 handoff 标记同笔 insert，字段类型受控），需手工改库/半截迁移/外部写坏行。建议降级为 skip+error log+计数事件。

**03-D3【不成立】quiet-hours 加载粒度分裂**
- 亲读：decision.rs:1414-1482（两个加载器全文）；webhooks.rs:487（settle_manual）、667（reconcile_workspace）、866-871（reconcile_pending）、1592-1597（主路径）。
- 反证：`load_user_operation_domain_config` 是 `load_user_operation_domain_config_for_contact(state, workspace_id, "")` 的薄封装（decision.rs:1414-1419）；后者的 `_contact_id` 参数**带下划线且函数体从不使用**（decision.rs:1445，查询仅按 workspace+domain+current_version，1450-1458）。所谓"contact 级"与"workspace 级"两个加载器**完全等价**，不存在 contact 级 override，run_at 不可能因此不一致。疑点前提不成立。

**03-D4【属实·设计】A-03/A-04/A-05 已知边界**
- 亲读：webhooks.rs:1451-1458（A-03）、2845-2848（A-04）、1975-1987（A-05）。
- 证据链：三处注释逐字自认（payload-hash 退化、±300s 无 nonce 重放依赖下游幂等、单账号 default 回落）并各自给出不修理由。代码注释备案的设计决策。

**03-D5【属实·设计】乱序消息仍标 materialized**
- 亲读：webhooks.rs:99-223（materialize_durable_inbound_task_at 全文）。
- 证据链：`newer` 谓词不成立时两个 update matched 0（143-205），但 210 行无条件 `mark_inbound_handoff(HANDOFF_MATERIALIZED)` 且 211-222 返回行内现值。义务由现行行承载、上下文按消息表聚合——语义自洽；"materialized"字面易误读属实。命名观察，非缺陷。

**03-D6【属实·缺陷】retry 事件文案**
- 亲读：tasks.rs:186-240（claim：`$inc attempt_count` + ReturnDocument::After，208-220）、616-729（process_claimed_task：判定 686、文案 721-723）。
- 证据链：claim 时 attempt_count 已 +1 并回读 After 值 → 文案"已安排第 {attempt_count}/{max_attempts} 次重试"中的 attempt_count 实为"已执行次数"。判定 `attempt_count < max_attempts`（686）正确（max=3 ⇒ 最多执行 3 次）。仅文案歧义。低。

**03-D7【属实·设计】status 注入与 $or 的隐式耦合**
- 亲读：tasks.rs:186-193（注入）、592-611（run_due_task_by_id filter）。
- 证据链：filter 顶层无 status → 注入 `status ∈ {pending,retry,failed}`（191-193）；$or 各分支（600-602）要求 pending+run_at 或 retry+next_retry_at → 合取后 failed 不可达。行为正确；"$or 分支须枚举全部可执行态"的隐式耦合确实存在，未来加分支需警惕——守护点成立。

**03-D8【属实·设计】写/查放大**
- 亲读：tasks.rs:1395-1447（reply_rate 循环：每条 outbound 一次 count_documents，1420-1437）、1277-1325（每 tick 全账号×2 无条件 insert 靠 dup-key 弹回）。
- 证据链与记录一致。正确性无损；30d 窗口高发送量账号 O(N) 网络往返属实。性能观察，非缺陷。

**03-D9【属实·设计】legacy 去抖层无生产调用方**
- 亲读：Grep `register_inbound|run_debounce_pipeline|handle_managed_message_aggregated` 全仓。
- 证据链：`register_inbound`（webhooks.rs:1045）/`run_debounce_pipeline`（1070）在 src/ 内仅互相调用与测试引用（2561+）；`handle_managed_message_aggregated` 唯一调用点是 run_debounce_pipeline 内部（webhooks.rs:1158）；生产主路径（1624-1641）走 durable task + `run_due_task_by_id`。"待退役保留兼容"状态确认。

**03-D10【属实·设计】裸 tokio::spawn 唤醒**
- 亲读：webhooks.rs:1633-1641；main.rs Grep（spawn_supervised 17 处，含 inbound_reply_worker，main.rs:221-223）；tasks.rs:552-557（250ms 轮询）。
- 证据链：低延迟唤醒是裸 spawn（1634），错误仅 tracing::error（1638-1640，注释自认"periodic worker will retry"）；250ms 的 `run_inbound_reply_worker`（tasks.rs:557）兜底存在。panic 无 supervisor 痕迹属实但兜底完备。设计取舍。

**03-D11【属实·设计】UTC 日界**
- 亲读：tasks.rs:1345-1354（today_date_string，注释"粗糙但足够幂等用"）；webhooks.rs:2185-2197（day_bucket = now_ms/day_ms）。
- 证据链与记录一致：对 UTC+8 运营者"当日"窗口偏移 8 小时。注释备案的取舍。

### 04 号 decision/review/guards（10 条）

**04-1【属实·缺陷】decision.rs 文档漂移**
- 亲读：decision.rs:1-20（模块 doc）；Grep `fn decide_reply\b` 全仓。
- 证据链：doc 两处提 `decide_reply`（decision.rs:1/9），全仓无该函数定义（仅 `decide_reply_with_promote`）。另发现同类：调用点注释（decision.rs:1326-1332）仍描述"调 validate_and_promote"并列出 `insufficient_detail_in_critical_turn:*`，实际调用是 `validate_reply_critical`（1345，见 04-4）。低（文档）。

**04-2【属实·缺陷】第三段渲染恒被丢弃**
- 亲读：decision.rs:915-940（绑定 `_suspected_deal_text`）、1028-1031（实际注入精简版）；entitlements.rs:290-390（三个渲染函数 + `render_transaction_facts_sections` 全文）；Grep 两个 render 函数全仓调用点。
- 证据链：`render_transaction_facts_sections`（entitlements.rs:358-377）生产调用点唯一（decision.rs:929），其第三返回值恒被 `_` 丢弃（decision.rs:928）；task prompt 注入的是 `render_suspected_deal_reply_guidance`（decision.rs:1030，fast 版，entitlements.rs:328-335 注释"Signal extraction is deferred to the projection worker"）；完整版 `render_suspected_deal_guidance`（entitlements.rs:297-308）除自身单测外无输出消费者。历史演进残留（fast/projection 契约拆分后未收缩签名），浪费一次字符串渲染。低（清理项，无行为影响）。

**04-3【属实·设计｜仍存疑】second reviewer parse 失败拉闸整 run**
- 亲读：review/mod.rs:4380-4435（双脑分支全文）、3069-3094（hold_for_review_schema_failure：approved=false + hallucination=10 + pressure=10 → 必拦）。
- 证据链：second **调用失败** → warn + 回退 primary（4417-4422）；second 调用成功但 **parse 失败** → `return Ok(hold_for_review_schema_failure(...))` 拦发（4409-4415）。与 4390 注释"双脑是增益机制，不应成为新故障源"存在张力；与 primary parse 失败同样拦（4428-4435）构成"给了 verdict 但不可信=必须拦"的对称解释，但代码内无文档确证该意图。行为终裁属实；**意图仍存疑，需产品决策**（若视为缺陷：严重度中——触发条件=启用第二 provider 且其持续输出畸形 JSON，将持续压制发送；建议至少加监控告警或对 second-parse-failure 做降级开关）。

**04-4【属实·设计｜仍存疑】fast 契约缺 why_should_reply 校验、R1.5 主链不可达**
- 亲读：types.rs:796-880（validate_reply_critical 全文）、1005-1050（validate_and_promote 的 R1.4/R1.5 段）、758-780（check_required_enum）；decision.rs:1315-1355（主链调用点）；gates.rs:660-698（finalize 的 insufficient_detail 降级分支）；Grep 两个 validate 函数全部调用方。
- 证据链：主决策链用 `raw.validate_reply_critical`（decision.rs:1345）；该函数 should_reply=true 时只查 reply_text（types.rs:857-859）、不查 why_should_reply（对照 validate_and_promote 的 R1.4，types.rs:1017-1020），且整段缺 R1.5/R1.6 长度判定（对照 types.rs:1025-1049）→ `insufficient_detail_in_critical_turn:*` 在主链 promote_risks 中不可能出现 → gates.rs:688 的 revision 降级分支在主链**不可达**（该分支是为修 t15 跌单弧专门建的，gates.rs:667-677 注释）。`validate_and_promote` 现仅 knowledge chat 路由（routes/knowledge/chat.rs:1917）与测试使用。fast 契约的 doc（types.rs:797-798"compact send-critical contract"）表明精简是有意的，但"精简掉 R1.5 使其修复通道失效"未见论证；调用点注释（decision.rs:1326-1332）还停留在 validate_and_promote 时代。行为属实；**是否刻意仍存疑，需产品决策**；注释漂移部分是确定的文档缺陷（低）。

**04-5【属实·设计】fail-open/fail-closed 分工**
- 亲读：guards.rs:77-134（operation_state_exists #155 注释 + action_policy_state_key 全文）。
- 证据链：`operation_state_exists` 空字典返 true 是注释论证过的局部 fail-open（77-85：真正迁移闸在 `check_state_transition`，空状态机 fail-closed，启动期另有 sanity check）；`action_policy_state_key`（115-134）按"exists && (同值 || check 通过)"选 policy 键，依赖该分工。逻辑自洽、有文档；隐式契约守护点成立。非缺陷。

**04-6【属实·设计】quoted_product_ids 双轨**
- 亲读：types.rs:205-217。
- 证据链：字段 doc（211-215）明言"仅为持久化/提示兼容保留，不能作为目录背书或发送授权证据；R5.4 priced_from_catalog 只由独立 ClaimGate 产生"。历史字段留存+文档标注属实，无编译期防护属实——记录准确，混淆风险备案。

**04-7【属实·缺陷】as_document 字段集漂移**
- 亲读：runtime.rs:440-505（from_config 尾段 + as_document 全文）；review/mod.rs:4150-4170（reviewer 注入点之一，4157）。
- 证据链：as_document（459-492）不含 `allowed_conversation_modes`、`consolidation_window_char_budget/max_messages`、`bayesian_slot_min_hits/min_strong`（均为 runtime 已有字段，440-455 可见后三组）；reviewer full 档把 `runtime.as_document()` 当「硬运行参数」注入（review/mod.rs:4157）。完整性漂移属实；reviewer 不消费这些值故无行为异常。低；触发条件：无（观测完整性）。

**04-8【属实·设计】开放世界门的计数耦合**
- 亲读：gates.rs:787-815。
- 证据链：`unsupportedNonProductBusinessClaimCount` 以 `.unwrap_or(0)` 读取（791-794）——ClaimGate 未跑（should_reply=false 无正文）时缺失按 0，与"无正文无风险"一致。若未来某路径跳过 ClaimGate 而保留正文则此门静默失效——结构性耦合点确认，当前无缺陷。

**04-9【属实·设计｜仍存疑】light reviewer 的 evidenceExcerpts 例外**
- 亲读：review/mod.rs:3613；decision.rs:1627-1642（决策侧剥离）+ 测试 2212-2266。
- 证据链：decision/full-reviewer 侧统一剥离 evidenceExcerpts（decision.rs:1642 map.remove）；light 档 route_summary 注入 `take(3)`（review/mod.rs:3613）。口径不一致属实；无注释说明是否为 light 档补偿上下文的有意设计——**仍存疑**（影响极小：3 条摘录回流 light reviewer 上下文）。

**04-10【属实·设计】"knowledge_required" 兼容垫**
- 亲读：guards.rs:70-75；types.rs:711-712（KNOWLEDGE_NEED_VALUES）、758-780（check_required_enum 越界即清空）。
- 证据链：闭集为 {not_required, required, insufficient}（types.rs:712）；planner 侧额外容忍 "knowledge_required"（guards.rs:73）。但主链上该值经 check_required_enum 已被打 `invalid_enum_value` 并清空为 ""（types.rs:772-774）→ planner 宽容分支仅对未经 promote 的内部构造 decision 可达，影响趋零。遗留兼容垫确认。

### 05 号 outbox/escalation/发送（12 条）

**05-1【属实·缺陷】manual_send 二次门与 contact 状态门语义矛盾**
- 亲读：outbox_dispatcher.rs:913-978（second_safety_gate：豁免仅三种内部通知，922-928）、2730-2754（check_contact_status_pure：manual_send 豁免+注释"admin 已显式确认发送意图"）、2760-2834（process_entry 顺序：二次门 2799-2802 **先于** contact 状态门 2830-2833）；outbox.rs:630-670（纯函数：第 0 条 `!is_managed → not_managed_at_send`，650-655）；gateway.rs:757-788（管理发送 precheck#1 在入队前已拦 not_managed/cooldown）。
- 证据链：manual_send 不在二次门豁免清单 → contact 非 managed 时在 2799 被 `not_managed_at_send` 取消，永远到不了 2830 为它设计的豁免——`check_contact_status_pure` 的 manual_send 豁免是**死代码**（对非 managed 情形），两处注释意图相反（二次门注释 B-03"撤管即停" vs 状态门注释"admin 已显式确认"）。同理 manual_send 也受 cooldown（656-660）与 30min stale（666-668）拦截。可达场景：入队时 managed（上游 gateway.rs:5251-5253 已保证）、派发前被改 normal/paused 或进入 cooldown 的竞态窗。
- 严重度：低（后果方向是 fail-safe——手动发送被静默取消，绝不会多发；cancel_entry 有审计）；触发条件：入队→派发间隔内 contact 状态翻转。**需产品决策**保留哪一侧语义（撤管即停 vs admin 意图优先），并删除死豁免或把 manual_send 加入二次门豁免。

**05-2【属实·缺陷】decision_created_ms 取值失真**
- 亲读：outbox_dispatcher.rs:952-977；outbox.rs:640-669（纯函数签名与判定）。
- 证据链：`let decision_created_ms = entry.created_at.timestamp_millis()`（dispatcher:967）传入名为 decision_created_ms 的参数（outbox.rs:646），用于 `last_inbound > decision_created && outcome 命中 stop`（661-664）。正常链路 entry 紧随 decision（毫秒级）；"stop 落在 decision 后、enqueue 前"的窄窗漏判属实，且 gateway 自身去抖（B-01 主/兜底检查）覆盖大半。低；命名失真建议修正。

**05-3【属实·缺陷】骚扰门拦截时不留台账**
- 亲读：escalation/mod.rs:215-280（escalate_held_decision 骚扰门→去重→insert 顺序全文）。
- 证据链：`push_allowed` 不过 → `return Ok(())`（mod.rs:231-233），注释却说"pending 台账可由 admin 在收件箱处置"；`insert_pending_escalation` 在其后（262-277）未执行——被拦 hold 请示不留任何台账（events/run log 里仍可见 held 状态，但请示收件箱无）。与 `scan_escalation_timeouts` 改派路径（骚扰门不过仅 continue、台账已存在稍后重试）语义不一致属实。
- 严重度：低-中；触发条件：领导 daily_push_cap 命中或静默时段/去重窗内发生新 hold。修复方向二选一：先落台账再决定是否推卡（由 reconciler 补推）、或改注释。

**05-4【属实·缺陷】effective_quiet_hours_enabled 文档漂移**
- 亲读：quiet_hours.rs:118-140（外层 doc + 函数体全文）。
- 证据链：外层 doc（118-131）详述 G04 三级解析链（contact override→profile→global）与"DEFAULT 字节等价"论证；函数体（132-140）参数带下划线、直接 `workspace_enabled`，内注"Workspace policy is authoritative…no longer alter scheduling behavior"。文档滞后于实现。低。

**05-5【属实·设计】两套 quiet-hours 实现**
- 亲读：escalation/policy.rs:122-132（AskHumanQuietHours 版：手工 `%24+24)%24`，`start<=end` 分支含 start==end→恒 false）；quiet_hours.rs:28-49（div_euclid/rem_euclid 版：显式 s==e→false）。
- 证据链：合法输入下行为等价（含 start==end 退化）；无共享代码、边界风格不同——将来单边修改易漂移。结构性偏差确认，非 bug。

**05-6【属实·设计】媒体/名片低优先级仅靠 aging**
- 亲读：outbox.rs:167-199（delivery_priority_for/run_sequence_for：媒体/名片恒 20，低于 manual 100/inbound 90/escalation 80/follow_up 60/incident 40）；outbox_dispatcher.rs:175（PER_TICK_PROCESS_CAP=16）、3261-3284（每 10 次 claim 一次 prefer_oldest）；runtime.rs:431/956（poll 默认 5s）。
- 证据链：记录的"16 条/tick×5s、aging 槽稀疏"数值全部核实。有意的客户文本优先；持续高负载下素材延迟分钟级的体验风险与"无 SLA 测算"观察成立。设计+观察点。

**05-7【属实·设计｜需产品决策】delivery_unknown 请示卡滞留**
- 亲读：ledger.rs:1125-1147（超时扫描 filter：仅 delivery_state=sent 且 last_pushed_at_ms 为数值）、250-279（reconcile：unknown→PENDING 且不写 last_pushed_at_ms，仅 SENT 写 264-267）、556-582（list_pending_for_principal 含 SENT+UNKNOWN，571-574）。
- 证据链：三处判定全部核实——unknown 卡永不进 SLA 扫描；领导实际收到仍可回复；确实没收到则无自动改派/安抚、无告警，只能靠 admin 收件箱。保守设计可解释（不确认送达不启动 SLA 时钟），滞留无告警的缺口属实——建议加观测事件。

**05-8【属实·缺陷】领导回复处理的 LLM 错误直达 webhook**
- 亲读：escalation/mod.rs:417-457（interpret：LLM 调用 `?`，仅 JSON/verdict 越界回落 deferred，444-455）、462+/531（handle_principal_reply 内 `?`）；webhooks.rs:1400-1425（分流调用点：`handle_principal_reply(...).await?`，且**先于**消息落库）。
- 证据链：LLM 网络错误 → webhook 返回 Err（HTTP 错误）；MCP 侧按 webhooks.rs:1443-1444 注释"5s timeout 内不重试"→ 领导这句话丢失（且因分流先于落库，消息本体也未持久化），须重说一次。缓解：generate_agent_json 内部有重试/jitter（llm.rs 契约），仅持续性 LLM 故障触发。
- 严重度：低-中；触发条件：领导回复时 LLM 通道持续不可用。修复方向：interpret 失败也回落 deferred（与 parse 失败同路径）或先落库再解读。

**05-9【属实·设计】MCP 入参占位形态**
- 亲读：referral.rs:190-215（":199 ⚠️ message_send_namecard 入参字段名待 server tools/list 确认"）；media_send.rs:160-179（":168 MCP 入参字段名以 server tools/list 为准；占位形态"）。
- 证据链：两处注释亲验存在；后果链（首次运行 ExplicitlyFailed→SafeToRetry 循环→failed_terminal）与 referral.rs:210-215 的回执分类一致。上线前必须与 MCP server 实测对齐——运维前置项，非代码缺陷。

**05-10【属实·设计】newMsgId 必须为 string 的成功核验**
- 亲读：outbox_dispatcher.rs:2556-2580（mcp_success_filter：`response.newMsgId {$type:"string",$ne:""}`，2575）；media_send.rs:385（同款）。
- 证据链：数值型 msgId 会漏判成功 → post-hoc Inconclusive → delivery_unknown（不重发，安全侧）。依赖 MCP 返回形态稳定——观察点确认。

**05-11【属实·缺陷】dead_code 注记腐化**
- 亲读：escalation/logic.rs:172-190（assert_target_is_principal：注记与现实一致，确无生产调用）；ledger.rs:670-757（emit_knowledge_gap_proposal **调用** derive_sediment_title 于 681-688）、759-800（两个 `#[allow(dead_code)]` + "暂无生产调用点"注释）。
- 证据链：`derive_sediment_title`（ledger.rs:794-795 注释"尚无生产调用点"）实际已被生产函数 emit_knowledge_gap_proposal 调用（681-688）——注记过期属实（无害）；`derive_sediment_title_fallback` 经由前者间接进生产。低（注释腐化）。

**05-12【属实·设计】pacing 闸覆盖内部通知**
- 亲读：outbox_dispatcher.rs:2903-2920（账号级最小发送间隔闸，无 source_kind 分支）。
- 证据链：principal 卡/澄清/系统事故同受 1-4s 拟人间隔（`account_send_interval_ms`，config 默认 1000-4000ms）。同一微信账号限频合理；语义备案成立。

### 06 号 memory/reaction/runtime 支撑（12 条）

**06-1【属实·设计｜仍存疑】update_run_envelope_terminal 无转换 CAS**
- 亲读：run_envelope.rs:296-310（is_valid_lifecycle_transition 纯函数存在）、442-455（mark_run_envelope_running：filter 带 lifecycle=started）、612-648（fail_run_envelope_if_open：filter 带 from-lifecycle）、657-705（update_run_envelope_terminal：仅枚举校验 663-681，filter 只有 `{run_id}`，692）。
- 证据链：终态写路径确实不校验 from→to；纯函数存在但未被该路径消费；一次迟到的 update 理论上可把 completed 覆写为 failed_after_decision。吸收性依赖"每 run 只 finalize 一次"的调用方纪律（gateway 的各终态写点互斥 return，实际成立）。设计权衡确认；**是否该加 CAS 仍存疑**（文档未明说），若视为缺陷：低（触发需要调用方 bug）。

**06-2【属实·设计｜仍存疑】extra 镜像 deprecatedFacts cap 6 vs typed 20**
- 亲读：memory.rs:515-587（compact cap 收口全文：typed `deprecated_facts.truncate(20)` 于 531；extra 镜像 `limit_extra_array(...,"deprecatedFacts",6)` 于 582；H17 注释 575-579 逐字声明镜像 cap "coreFacts 6 / recentFacts 10 / deprecatedFacts 6"）。
- 证据链：6≠20 是注释里写死的显式行为（非笔误可能性高），但差异理由无解释。影响：老数据 extra.deprecatedFacts>6 被截到 6，typed 层保 20——镜像仅为旧读端兼容，实际影响极小。**理由仍存疑**，建议作者补注或对齐。

**06-3【属实·设计】append-only 降级路径的默认 runtime**
- 亲读：post_decision.rs:905-949（stale 降级路径：`UserRuntimeParameters::from_config(None, state)` 于 934，signal_only decision 930-933）、1043-1044（主路径用快照 domain_config + apply_active_profile）。
- 证据链：不对称属实；降级路径只回放 append-only 观察（tag/stage observation + 弱信号 upsert），runtime 阈值在这些函数里几乎不消费——低风险评估成立。

**06-4【属实·设计】activate 与 runnable_filter 的冗余**
- 亲读：post_decision.rs:585-641（activate：prepared→pending；discard：仅匹配 prepared；runnable_filter：prepared/pending/retry 且 review.status ∈ [outbox_enqueued,sent,no_reply]）。
- 证据链：prepared 行可不经 activate 被认领属实——但 runnable 前提是 review.status 已翻三终态之一，即发送已授权/no_reply 已定，此刻投影本就该执行；discard 只在 review.status 仍 outbox_enqueuing 的失败路径调用，彼时行不满足 runnable，被抢占的窗口不存在实质危害。冗余与竞态语义安全的判断确认。

**06-5【属实·设计】strip_known_tags 固定 8 子串**
- 亲读：prompt_isolation.rs:1-25（模块 doc：三原则，明言"不做关键词黑名单（fuzz 化的越狱不可能 enum）；模型策略层才是决定层"）、480-496（函数体：USER_OPEN/CLOSE + 小写 user/system/assistant 三对）。
- 证据链：`<SYSTEM>` 等大小写变体不剥属实；模块文档已声明这是既定取舍。已知边界确认。

**06-6【属实·设计】reaction turn_index 全量计数**
- 亲读：reaction.rs:1085-1110（count_documents 全量 inbound，无窗口）。
- 证据链：与 tag_evidence 的"窗口 0-based 下标"不同坐标系属实；intent_trajectory 仅观测。阅读混淆提醒成立，无一致性风险。

**06-7【属实·设计】shadow 指纹全量扫描**
- 亲读：prompt_shadow.rs:735-768（specs 列表尾部 + 逐行 SHA256 循环，find 无 limit）。
- 证据链：按 tenant filter 拉全部行做指纹属实；大 workspace 代价可观、正确性无虞（宁可 fail shadow_dependencies_changed）。性能标注确认。

**06-8【属实·缺陷】同维冲突 warning 泄漏下标**
- 亲读：memory.rs:775-805（胜者选择 + `same_dimension_conflict_deprecated:{dim}:idx{i}` 于 793，i 为移除前数组下标；随后 801-805 按下标降序 remove）。
- 证据链：warning 用移除前下标，事后无法稳定回指具体 fact（audit events 另有 id 可交叉）。可用性小瑕疵确认。低；触发条件：同维多 fact 冲突整理时审计回溯。

**06-9【属实·设计】否定标记先于 DIRECT**
- 亲读：reaction.rs:88-123（explicit_stop_intent：否定/引用标记 101-115 先 return false，DIRECT 表 116 起）。
- 证据链：含"比如"的真实指令回落 LLM 路径属实；注释（100 行）声明"不把讨论/否定 opt-out 的人误判为指令"——高精度地板的刻意保守。fail-open 到模型侧，语义边界确认。

**06-10【属实·设计】无 claim 整理路径非事务**
- 亲读：memory.rs:2030-2201（prepared-commit 主路径分叉 2040-2051；无 claim 兼容路径：OCC 写 2084-2104、输者处置 2105-2128、contact 投影 fail-soft 2134-2148、候选置 consolidated 2150-2159、事件 2161-2177、task 终态 2178-2199；2053 注释"不参与任务取消协议，保持原 OCC 写入语义"）。
- 证据链：崩溃在 OCC 写与候选更新之间 → 卡已更新但候选仍 pending → 下轮重复喂 LLM，由 compact 去重吸收。生产路径走 prepared-commit（2040-2050），兼容路径弱一致是备案取舍。确认。

**06-11【属实·设计】extra 镜像也算记忆信号**
- 亲读：memory.rs:145-175（memory_card_has_signal：typed 三数组 + extra 9 键 + recentEpisodeSummary + coreProfile）。
- 证据链：typed 全空但 extra.coreFacts 残留 → 有信号 → 不被种子卡覆盖。防覆盖真数据正确；"彻底清空记忆需同时清 extra 镜像"的运维注意点成立。

**06-12【属实·设计】双层锁并存自洽**
- 亲读：post_decision.rs:619-641（runnable_filter 的 processing 分支只看 post_decision_locked_until）、643-645（contact_lease_id）、652-682（acquire_contact_lease：独立集合 CAS，claim_token/locked_until 谓词）。
- 证据链：review 层锁过期但 contact lease 未过期时，新 worker 在 acquire_contact_lease 被挡 → 候选 defer。行为自洽确认。

---

## 3. 需回写源记录的修正清单

主会话回写时，建议在各源记录 §5/§6 对应条目后追加终裁标注：

**01-agent-gateway.md §5**
- 疑点 1 → 标注"终裁：属实·缺陷（低）——apply_agent_updates 仅存于 post_decision.rs:935/1109，注释对象已迁移"。
- 疑点 2 → 标注"终裁：前提大半过时——DEFERRED_INBOUND_REPLY_KIND 无创建点（quiet_hours.rs:18-20 自注 legacy），生产 wake=durable task 以 Inbound 触发、daily_limit 天然豁免；仅 legacy 残留行可触发 cancel 且需下一条入站/策略编辑重建"。
- 疑点 6 → 标注"终裁：不成立——第一次 normalize 恒 no-op（initial_planner.memory_change_importance=0，guards.rs:42-46 只回填该字段），planner_from_decision 无依赖"。
- 疑点 10 → 标注"终裁：不成立（作为缺陷）——dispatcher Stale→cancel_entry 闭环成立（outbox_dispatcher.rs:429-433/477-484/615-623）"。
- 疑点 12 → 标注"终裁：不成立——仅写 .performance 子键"。
- 疑点 3/4/5/7/8/9/11/13 → 标注"终裁：属实·设计（详见 22 号 §2）"。

**02-models-db.md §6**
- 疑点 7 → 撤销"待亲验"标记，改为"已亲验成立（gateway.rs:6470-6511）"。
- 疑点 9 → 标注"终裁：不成立（兼容性缺陷不存在）"。
- 疑点 3/4/6/12 与偏差 1/5 → 标注"终裁：属实·缺陷（低，均为文档/注释/测试滞后类）"。
- 疑点 8/10/14 与事实 2/11/13/15 → 标注"终裁：属实·设计/事实确认"。

**03-webhooks-tasks.md §5**
- D2 → 标注"终裁：属实·缺陷（中）——毒丸行瘫痪两 worker tick 后续步骤；建议降级 skip+error log"。
- D3 → 标注"终裁：不成立——`_contact_id` 参数被忽略（decision.rs:1445），两加载器等价，无 contact 级 override"。
- D1/D6 → 标注"终裁：属实·缺陷（低）"。
- D4/D5/D7/D8/D9/D10/D11 → 标注"终裁：属实·设计"。

**04-decision-review-guards.md §5**
- 疑点 1/2/7 → 标注"终裁：属实·缺陷（低）"。
- 疑点 3 → 标注"终裁：行为属实；意图仍存疑，需产品决策（parse 失败 fail-closed vs 调用失败 fail-open 的不对称）"。
- 疑点 4 → 标注"终裁：行为属实（R1.5 主链不可达、why_should_reply 未校验）；是否刻意精简仍存疑；另 decision.rs:1326-1332 调用点注释停留在 validate_and_promote 时代（文档缺陷）"。
- 疑点 5/6/8/9/10 → 标注"终裁：属实·设计（9 仍存疑）"。

**05-outbox-escalation-send.md §5**
- 疑点 1 → 标注"终裁：属实·缺陷（低，fail-safe 方向）——process_entry 中二次门（:2799）先于状态门（:2830），manual_send 豁免死代码；需产品定夺语义归属"。
- 疑点 2/3/4/8/11 → 标注"终裁：属实·缺陷（2/4/11 低；3/8 低-中）"。
- 疑点 5/6/7/9/10/12 → 标注"终裁：属实·设计（7/9 需产品/上线决策）"。

**06-memory-reaction-runtime.md §5**
- 疑点 8 → 标注"终裁：属实·缺陷（低）"。
- 疑点 1/2 → 标注"终裁：属实·设计权衡，仍存疑（1：是否补 lifecycle CAS；2：镜像 cap 6≠20 的理由）"。
- 疑点 3/4/5/6/7/9/10/11/12 → 标注"终裁：属实·设计"。

---

## 4. 覆盖自证

**终裁条数：73 条**（01 号 13、02 号 15、03 号 11、04 号 10、05 号 12、06 号 12），全部基于当场亲读源码，无一条仅凭记录文本判定。分布：属实·缺陷 20、不成立 5、属实·设计（含需产品决策/事实确认，其中 6 条显式标"仍存疑"）48。

本次终裁亲读的源码文件与行号区段（Read 工具逐段读取；标 (g) 者为 Grep 定位后精读命中行）：

| 文件 | 亲读区段 |
|---|---|
| src/agent/gateway.rs | 330-364, 720-788, 988-1060, 1495-1579, 1845-1919, 2361-2510, 2926-3000, 3100-3159, 3370-3404, 3580-3594, 3900-3989, 4175-4292, 4530-4590, 4660-4719, 5245-5420, 5544-5641, 5739-5809, 6082-6195, 6470-6514 |
| src/webhooks.rs | 60-299, 470-520, 560-908, 1395-1653, 1655-1703, 1968-1992, 2180-2197, 2838-2862 |
| src/tasks.rs | 180-240, 552-557(g), 560-740, 1040-1155, 1270-1355, 1395-1447 |
| src/agent/guards.rs | 30-144 |
| src/agent/decision.rs | 1-20, 915-1040, 1315-1355, 1414-1482 |
| src/agent/types.rs | 205-235, 700-780, 796-880, 1005-1050, 1688-1705, 1837(g) |
| src/agent/review/mod.rs | 3069-3094(g), 3613(g), 4150-4170, 4380-4435 |
| src/agent/review/gates.rs | 660-698, 775-815, 3053(g) |
| src/agent/runtime.rs | 279(g), 402-408(g), 440-505, 934/956(g) |
| src/agent/entitlements.rs | 290-390 |
| src/agent/sufficiency.rs | 110-120(g) |
| src/agent/outbox.rs | 160-199, 630-670 |
| src/agent/outbox_dispatcher.rs | 175(g), 410-640, 900-999, 2556-2580, 2700-2930, 3255-3284 |
| src/agent/escalation/mod.rs | 215-280, 417-465, 531(g) |
| src/agent/escalation/policy.rs | 110-152 |
| src/agent/escalation/ledger.rs | 250-279, 556-582, 670-800, 1125-1147 |
| src/agent/escalation/logic.rs | 172-196 |
| src/agent/referral.rs | 190-215 |
| src/agent/media_send.rs | 160-179, 272/385(g) |
| src/agent/quiet_hours.rs | 15-150 |
| src/agent/run_envelope.rs | 296-310(g), 442-455(g), 612-705 |
| src/agent/memory.rs | 145-175, 515-595, 775-805, 2030-2210, 2653-2669(g) |
| src/agent/post_decision.rs | 585-682, 905-950, 1030-1058 |
| src/agent/projection_observations.rs | 1-138（全文） |
| src/agent/prompt_isolation.rs | 1-25, 480-497 |
| src/agent/prompt_shadow.rs | 735-768 |
| src/agent/reaction.rs | 88-123, 1085-1110 |
| src/agent/knowledge_agent.rs | 1781, 2621-2624(g) |
| src/models.rs | 155-234, 700-720, 758-780, 905-967, 1795-1810, 1988-2000, 3644/3753(g), 3770-3785, 4730-4731/4790-4791/4881/4945(g), 6028(g) |
| src/config.rs | 24(g), 470-499 |
| src/db/indexes.rs | 50-70, 164-175(g), 196-210(g), 756-768, 805-826, 1310-1316(g), 1963-1980, 2602-2618 + 函数体内九组集合索引 Grep 定位（2808-3429） |
| src/db/migrations/mod.rs | 408-421, 523-524(g) |
| src/db/migrations/m009/m011/m016/m028/m034/m055 | m009:70-83；m011 全文（35 行）；m016:193-206；m028:100-116；m034:48-61；m055:19(g), 88-107 |
| src/db/migrations/m010/m014 | 触达行 Grep 亲验（m010:35-39；m014:17） |
| src/routes/management.rs | 2295-2326 |
| src/routes/lessons_learned.rs | 19-47, 171-266, 335-355（Grep 命中行精读） |
| src/main.rs | 215-346 worker spawn 清单（Grep 命中行） |
| tests/（rate_limited 语义佐证） | real_llm_ops_smoke.rs:1318-1321、roleplay_emotional_companion_e2e.rs:409、full_flow_suite.rs:18/1092 等 Grep 命中行 |

合计精读约 6,500+ 行、跨 38 个源码文件；另执行约 30 次全仓/定向 Grep 用于调用方封闭性验证（`apply_agent_updates`、`DEFERRED_INBOUND_REPLY_KIND`、`decide_reply`、`register_inbound`、`validate_and_promote`、`interpret_principal_reply`、`operation_knowledge_items`、`lesson_promotion`、`chunks_wiki_type_default` 等）。所有【不成立】结论均给出当场反证行号；所有"仍存疑"条目（04-3、04-4、04-9、06-1、06-2 及 05-1 的语义归属）均注明存疑原因，未做硬判。
