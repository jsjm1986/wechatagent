# 测试-生产一致性验证（核证日期 2026-08-13）

> 任务：把 15 号（tests/ 前半，61 条系统承诺总表）与 16 号（tests/ 后半，11 类系统承诺）合并为全局承诺清单，逐条映射到 01-10 号生产记录（routes 面辅以 11/12 号，同为生产记录），判定一致性；对疑似矛盾**亲读双侧源码**裁决；对 15 号发现的复刻式测试逐个核验当前是否漂移；结合两个空壳测试与总台账 11 条已核证缺陷，产出"生产行为无测试守护清单"。
>
> 判定四值：【一致】测试锁的行为与生产记录吻合；【弱于】生产有该行为但测试只锁部分分支；【矛盾】两者描述冲突（本记录已亲读双侧源码裁决）；【已变】测试锁的行为在生产已演进/退役。
>
> 本记录亲读的源码（测试侧 8 文件全文/关键段 + 生产侧 7 处）清单见 §6。所有 file:line 为 2026-08-13 工作区版本（含未提交改动）。

---

## 1. 全局承诺清单与映射判定表

合并口径：15 号 §3 不变量总表 61 条（编号 A1-A61，对应原 1-61）+ 16 号 §4 不变量总表 11 条（编号 B1-B11）= **72 条**。其中 B11（真模型测试方法论纪律）是测试自身纪律非生产行为承诺，不参与映射，实际映射 **71 条**。

### 1.1 网关决策主链（A1-A10）

| # | 承诺（浓缩） | 生产锚点（记录号 + 位置） | 判定 | 备注 |
|---|---|---|---|---|
| A1 | LLM 调用次数恒等式（直发 3 / revision 或 rewrite 6 / rewrite 后占位 7 / 不回复 1 / tool-loop 6） | 01号 §2.4(5)(10)(11)(14)：首程+Review+ClaimGate 并行、rewrite/revision 预算包 `4+dual`（gateway.rs:3049/3261） | 一致 | 恒等式按"单一路径"分别锁定成立；rewrite+revision 串联场景（见 A2）未被任何次数恒等式测试覆盖 |
| A2 | Reply Agent 调用硬上限 2 次（1 首轮 + ≤1 revision） | 01号 §2.4(11)(14)：targeted rewrite（gateway.rs:3105）与 revision（:3379）都会重调 Reply Agent；rewrite 条件=硬闸失败无 revision 需求、revision 条件=软闸 needs_revision，rewrite 后重新 review 可再触发 revision → **生产可达 3 次** | **矛盾** | 亲读裁决见 §2.2：P2 是模型测试且模型缺 `apply_revision_fallback` 分支；"≤2"仅对 revision 子流程成立，作为全局承诺表述过强 |
| A3 | 未执行 Reviewer 绝不发送（local fallback 恒 approved=false；预算超额 `budget_exceeded_no_review`；非预算 `required_reviewer_not_executed` hold） | 04号 §2.2 `local_decision_review`（review/mod.rs:3300-3365）三分支逐条吻合 | 一致 | P3 直调生产函数（非复刻） |
| A4 | 决策 JSON 12 自治协议字段缺失/枚举非法必产 risk；conversationMode 四值；tool_calling 中间轮跳过校验 | 04号 §2.6：完整契约 `validate_and_promote`（types.rs:894-1128）行为与测试锁定一致；**但生产主链只调 fast 契约 `validate_reply_critical`**（decision.rs:1345，types.rs:799-889）——不查 R1.3 七字段、不查 should_reply 侧 why_should_reply、无 R1.5 长度门（04号疑点4：finalize 的 insufficient_detail 降级分支主链不可达） | **已变（部分）** | 函数本身未变（P1 仍锁得住 `validate_and_promote`），但主链消费方已换为 fast 契约（台账偏差表#2 的 retired full task 同源）。"缺失必产 risk"承诺在生产主链仅对 fast 契约字段成立 |
| A5 | 评审阈值语义：humanLike `<` 拦 / pressureRisk `>=` 拦且 0 是 legacy 豁免 / 双闸 AND / approved 一票否决 | 亲读 gates.rs:20-47（§2.3）：pressure 0 豁免仅当 `!live_scores_are_valid`；live 有效评分中 0 → 不通过（`pressure_risk > 0` 要求 + classify 侧 `pressure_risk_0_unscored` risk） | 一致（弱于） | 测试 fixture 用 Default claim_analysis（无 reviewScoreStatus）恰落 legacy 分支，与生产一致；live_valid 分支（0 分不豁免）在 tests/ 层无 PBT 覆盖（lib 内嵌测试有） |
| A6 | 无来源开放世界业务事实：ClaimGate 标 unsupported → 一次 targeted rewrite；仍违规 → blocked_by_safety_guard + 客户收中性占位 | 04号 §2.2 `merge_independent_claim_verdict`（mod.rs:1083-1106）+ finalize ⑥（gates.rs:787-824）+ 01号 `ensure_customer_acknowledged`（gateway.rs:2251-2335） | 一致 | |
| A7 | performance 子文档 + llm_call_logs 同步落库合同 | 01号 §2.8 settle（gateway.rs:1856-1889）flush 报告 + performance 写入 | 一致 | 01号疑点12：run log 未创建时 performance 写 matched=0 静默——测试只锁 happy path |
| A8 | Run Envelope：写入先于 LLM / run_id 唯一 / 丢失 insert 兜底+recovered 审计 / panic 保留 payload / 终态更新不新插 | 06号 §2.7（run_envelope.rs:384-435, 657-812）+ 01号 settle catch_unwind | 一致 | 06号疑点1：`update_run_envelope_terminal` 无 lifecycle 转移 CAS，终态吸收性靠调用纪律——该弱点测试与生产同盲（见 §4#14） |
| A9 | 缺失 state policy fail-closed（回复与手动发送都在 outbox/MCP 前拦停） | 04号 `load_operation_state_policy_for_contact`（decision.rs:1501-1569）state 无版本但 config 存在 → Conflict | 一致 | |
| A10 | Shadow 模拟零业务副作用；dry-run 只写审计 | 06号 §2.10 simulation.rs 全程零写库 + run_mode="shadow"；11号 §2.3 management dry-run 分支 | 前半一致 / dry-run **无有效测试** | `simulation_no_sideeffect` 真跑全库快照比对；`dry_run_isolation` 是自插自读（§3.7），生产 dry-run 分支无行为级守护 |

### 1.2 去抖/抢占（A11-A13）

| # | 承诺 | 生产锚点 | 判定 | 备注 |
|---|---|---|---|---|
| A11 | 去抖窗口聚合一次运行/一条 outbox/最新快照/晚到 bump 不重复 spawn | 03号：生产 webhook 已改 durable 任务模型（materialize + refresh 分支水位单调，webhooks.rs:99-223）；legacy 进程内去抖（PENDING/run_debounce_pipeline）**已无生产调用方**（03号 D9 grep 亲证） | 弱于/部分已变 | debounce_barge_in_run / debounce_pipeline_integration 驱动的是 legacy runner + `handle_managed_message_aggregated`（gateway.rs:153-172 仅剩 legacy 用途）；聚合语义在生产由 durable 任务 + `load_recent_messages` 水位合并承载（另有 sr177 锁定）。gateway 侧 `should_abort_send` 检查点两代共用，该部分不变量仍被守护 |
| A12 | barge-in guard 返回 true → superseded、0 outbox、last_agent_run_at 不推进 | 01号 §2.4(19)(25)：主去抖检查在写库/推进频控锚之前（gateway.rs:3915-3946, 4217-4283） | 一致 | B-01 尾窗（gateway.rs:4228-4234 官方不修）测试不覆盖（§4#13） |
| A13 | 静默与普通 debounce 共用同一条 inbound_reply 持久义务（幂等、永不过期、quiet_hours_waiting） | 03号：webhook quiet 分支 `materialize_durable_inbound_task_at(next_wake_at, "quiet_hours_waiting")`（webhooks.rs:1592-1623），同一确定性 _id 行 | 一致 | |

### 1.3 状态机（A14-A18）

| # | 承诺 | 生产锚点 | 判定 |
|---|---|---|---|
| A14 | check_state_transition 判定表 + 空状态机 fail-closed + unknown_target fail-closed + 拦截理由前缀协议 | 04号 guards.rs:187-254 逐分支吻合 | 一致 |
| A15 | 引擎行业无关（读 initial/allowFromAny/allowedFrom 标志） | 04号 guards.rs + 医疗 FSM 交叉单测（G09） | 一致 |
| A16 | C2 派生：operation_state 优先派生自 customer_stage；非法迁移 fail-soft 跳写留旧值 + 对称审计；审计失败不吞回复 | 01号 §2.7 apply_agent_updates 步骤 6/9（gateway.rs:6305-6511） | 一致 |
| A17 | stage 证据分级：弱证据落暂定层不写 domain_attributes；强证据实时写 | 01号 gateway.rs:5988-5998 + 6367-6428；06号 tag_evidence.rs:33-52（客观强弱判定） | 一致 |
| A18 | intent_trajectory 滑窗 cap=50、FIFO、新条目在尾 | 06号 reaction.rs:1160-1179（`$push+$each+$slice:-50`）+ models MAX_ITEMS=50 | 一致 |

### 1.4 outbox/发送（A19-A27）

| # | 承诺 | 生产锚点 | 判定 |
|---|---|---|---|
| A19 | 同 idempotency_key 至多一次物理发送；key v2 含 workspace；媒体/名片 key 含 asset/card id | 05号 §2.1 enqueue 幂等键全规则（outbox.rs:262-286, 543-568）+ scoped v2（:458-475） | 一致 |
| A20 | 回执三分法（ok=false 重试 / 无可信回执或越界 HTTP 失败 → delivery_unknown 禁自动重放 / 名片恒 Inconclusive / 明确初始化失败重试至 failed_terminal） | 01号 classify_send_receipt（gateway.rs:5035-5050）+ 05号 verify_delivery 三路（outbox_dispatcher.rs:2594-2647）+ settle_ambiguous_send | 一致 |
| A21 | 取消语义（stop 取消全部 pending；claim 后未越界 stop 赢 CAS；越界迟到取消 best-effort 落 sent 保留审计） | 05号 cancel pipeline（outbox.rs:679-964）+ begin_remote_send（:710-747）+ settle_late_cancel（:856-911） | 一致 |
| A22 | 崩溃恢复：reclaim 回 pending；reclaimed_in_flight 先过 post-hoc 幂等门再过 pacing | 05号 process_entry 步骤 8（恢复核对）在步骤 9（pacing）之前（outbox_dispatcher.rs:2856-2920） | 一致 |
| A23 | 门序与节流（拟人间隔 reschedule 不耗 attempt、按 account 隔离、无历史 fail-soft；日发送量软上限只告警；掉线 defer；30min stale 取消） | 05号 defer_account_pacing/offline（:2384-2525）+ 01号 账号日发软上限观测（gateway.rs:5096-5126）+ 二次门 stale | 一致 |
| A24 | 任务发送 fencing（批次未封口不发送；陈旧 claim 取消 0 发送；同 claim 恰一次授权；cancel_for_decision 拦 pending+in_flight） | 05号 SR-034 授权链（outbox_dispatcher.rs:412-626）+ 03号 bind/authorize CAS（tasks.rs:289-535） | 一致 |
| A25 | 持久入站交接（handoff 崩溃 reconcile 恰一次物化；按 created_at 刷新单飞 fence 旧代；收养可恢复行恰发一次；取消行/越界行不复活） | 03号 §2.1（webhooks.rs:99-223, 803-908）+ tasks.rs adopt 六条件（:349-465） | 一致 |
| A26 | 混合 run partially_sent 与顺序无关；管理端 cancel 错账号 Conflict 零写 | 05号 aggregate_run_outbox_status（:980-1023）+ 12号 admin_outbox | 一致 |
| A27 | send_ledger：outbox_id 锚唯一、重放幂等、不跨账号归因、对账不改写历史 | 05号 §2.11 record_send upsert `$setOnInsert` 按 (ws,acct,outbox_id)（send_ledger.rs:44-136）+ scan 只回填自己表 | 一致 |

### 1.5 多租户隔离-主链（A28-A30）

| # | 承诺 | 生产锚点 | 判定 |
|---|---|---|---|
| A28 | webhook 限流桶/pacing/幂等键/reaction stop 取消范围按 (workspace, account) 隔离 | 03号 shared_webhook_rate_limit 桶 id（webhooks.rs:911-923）+ 05号 account_last_sent_at_ms、cancel_for_contact 三键 filter | 一致 |
| A29 | 管理写动作跨 workspace NotFound / 错 account 400/404/409 零写零审计 | 11号 find_contact_by_id(_for_account)（shared.rs:218-262）+ 各写端点；08号 knowledge 各拒绝零写 | 一致 |
| A30 | outbox 管理列表投影不泄漏跨账号资产元数据 | 12号 admin_outbox（生产记录覆盖粒度粗） | 一致（弱佐证） |

### 1.6 记忆（A31-A34）

| # | 承诺 | 生产锚点 | 判定 |
|---|---|---|---|
| A31 | memory card compact：core≤6/recent≤10/维度 cap；未 discarded 旧事实必可追溯；discarded 不复活；稳定排序 | 06号 §2.1 compact 全规则（memory.rs:387-587）逐条吻合 | 一致 |
| A32 | 首触达并发 create：输家回落 find_one 不透传 E11000 | 06号 load_or_create_operating_memory（memory.rs:1231-1257） | 一致 |
| A33 | 记忆固化两阶段提交（单飞 Conflict 零改动；prepared_commit 三窗口重放恰一次） | 06号 prepared commit 协议（memory.rs:2400-2648）+ run_manual_memory_consolidation Conflict（:3134-3211） | 一致 |
| A34 | 运营者记忆：撤销后不注入；跨 scope 不可区分 NotFound；重复撤销幂等 | 06号 operator memory（memory.rs:3225-3495） | 一致 |

### 1.7 反应（A35-A38）

| # | 承诺 | 生产锚点 | 判定 |
|---|---|---|---|
| A35 | claim 互斥：活跃 claim 期间第二入口 0 LLM；过期 owner 结论不覆盖现任 | 06号 reaction claim（reaction.rs:368-400, 492-515 token CAS stale 丢弃） | 一致 |
| A36 | stop 串联：取消全部 pending；显式停止语零 LLM 生效 + 持久化屏障 | 06号 explicit_stop_intent 在一切加载前（reaction.rs:37-79）+ apply_deterministic_stop（:242-332，DB 失败仍返回 stop） | 一致 |
| A37 | 确定性买入是交易 profile 特性；不造 deal/payment 事实 | 06号 reaction.rs:428-429（transaction_facts_enabled 门）+ :164-169 注释 | 一致 |
| A38 | 未送达回复不学成反应标签（outcome 停 pending） | 06号 claim filter 仅 status="sent" review（reaction.rs:368-400）——outbox 未送达时 review 停在 outbox_enqueued 不可 claim | 一致 |

### 1.8 请示/引荐（A39-A45）

| # | 承诺 | 生产锚点 | 判定 | 备注 |
|---|---|---|---|---|
| A39 | 客户可见文案禁词 + 不暴露幕后决策源 + 兜底安抚不含禁词 | 05号 labels/logic 兜底文案（logic.rs:89-126）+ relay 泄漏守卫（:214-219）+ holding_reply 三关守卫（holding_reply.rs:45-65） | 一致 | |
| A40 | 请示台账协议（verdict 闭集；知识沉淀恒 draft+needs_review；awaiting set/clear；deferred 保持 pending） | 05号 sanitize_verdict（logic.rs:413-425）+ emit_knowledge_gap_proposal（ledger.rs:674-757）+ awaiting 管道（:293-424）+ mod.rs:532-535 | 一致 | |
| A41 | 超时改派：骚扰门先于改派；每位决策人从 **sent 对账时刻**起拿完整超时窗；generation CAS 收敛；旧代取消；链尾安抚去重；投递终败释放 | 05号 scan_escalation_timeouts（mod.rs:559-661）+ 亲读 reassign_escalation（ledger.rs:1077-1122：`$unset last_pushed_at_ms`，送达对账后由 reconcile 写入）+ principal_card_send_is_authorized 双检 | 一致 | 真守护来自 ask_human_phase1_e2e/principal_decision_channel（调生产函数）；escalation_push_time_reassign 复刻已过期（§3.5）。hold 侧骚扰门拦截零台账（05号疑点3）无测试（§4#7） |
| A42 | relay 回路（明确批准≠deferred；relay task 恰一条；授权过期清 awaiting+中性收尾；泄漏转述被拦） | 05号 handle_principal_reply/interpret（mod.rs:417-554）+ terminalize（ledger.rs:430-527）+ 01号 relay 泄漏守卫（gateway.rs:4175-4216） | 一致 | |
| A43 | webhook 请示路由（领导消息 routed=principal 不进客户链路；澄清走持久 outbox 幂等） | 03号 主 handler 步骤 9（principal 分流先于落库）+ 05号 Ambiguous 澄清（source_event_id 幂等） | 一致 | |
| A44 | 跨 workspace resolve 幂等 200 但不真裁决 | 05号 resolve_escalation filter 带 workspace（ledger.rs:610-668） | 一致 | |
| A45 | 名片引荐 enabled+approved 双门 + card_id 幂等 + 与 media 互斥 | 亲读 build_referral_cards_filter（decision.rs:1787-1801）+ 05号 validate_card_sendable 三处复用（referral.rs:37-43）+ outbox synthetic_namecard key | 一致 | |

### 1.9 campaign/planner/主动触达（A46-A49）

| # | 承诺 | 生产锚点 | 判定 |
|---|---|---|---|
| A46 | campaign：命中 0 拒绝/超上限拒绝/completed 拒重派/spec CAS/终态 preview 零写/task 失败 reconciler 恢复/命中与入队分账 | 11号 §2.5 dispatch 全链（campaigns.rs:454-944）逐条吻合 | 一致 |
| A47 | planner：只扫 managed+静默；emit 保留 review_required；tick 事件恒写；commitment overdue/imminent；calendar 仅声明 date_dimension 时运行；同日同纪念日稳定去重；block-rate backoff 不耗 cap | 10号 §2.16 六段扫描逐条吻合（planner/mod.rs）+ proactive_outreach build_task review_required:true | 一致 |
| A48 | 主动触达统一提交：并发同 intent 恰一 Emitted；UTC 日桶；段/总双 cap；滚动基线追认；task+event+quota 三写事务 | 10号 §2.17 proactive_outreach（身份先于配额、$max 基线、Capped abort 回滚） | 一致 |
| A49 | 冷重激活唯一可发条件 = Managed+outbound 老于阈值+无 inbound 反超+无 cooldown+无 pending | 10号 §2.18 decide_cold_emit 七态（cold_contact_worker.rs:417-476） | 一致 |

### 1.10 管理流/准入（A50-A57）

| # | 承诺 | 生产锚点 | 判定 | 备注 |
|---|---|---|---|---|
| A50 | 批量托管全套（sharedNote 必填/pool 整批 Conflict/非人类拒/intent 先于 managed/enrollment_token 代际/初始态同步落/已 managed 不重复） | 11号 §2.4 batch_enable + enrollment intent 协议（contacts.rs:1213-1918） | 一致 | |
| A51 | roster 富化只取 roster 快照；未命中留 None；新建默认 Normal | 03号 upsert_webhook_contact（webhooks.rs:2043-2148） | 一致 | |
| A52 | 管理命令协议（planHash 冻结+账号 CAS+确认；恰 1 次规划 LLM；死租约 execution_unknown 不重放） | 11号 §2.3 confirm/租约执行/intent 重放语义（management.rs:926-1135, 336-636） | 一致 | |
| A53 | 疑似成交：AI 永不直写 outcome；approve 才 staff_confirmed；审批+成交+审计同事务；重复审批 CAS 不双计；reviewedBy 伪造被忽略 | 12号 admin_suspected_deals（approve 事务 + CAS + 服务端身份）+ 01号 suspected_deal 弱信号专表（gateway.rs:6713-6743） | 一致 | |
| A54 | 产品快照订单式冻结 | 11号 prepare_outcome_event 冻结 OutcomeProductRef（shared.rs:1705-1828） | 一致 | |
| A55 | runtime 参数类型化边界（未知键 400 零写/legacy 别名归一/Guide 白名单/强确认/apply 事务+receipt/Preview 零副作用） | 04号 runtime.rs:24-274 + 11号 guides.rs 预览冻结/apply 事务/租约 | 一致 | |
| A56 | taxonomy/关系审批事务（insert 失败回滚候选；reviewed_by 恒来自认证会话；一 pending 周期） | 12号 六条审核链共性（pending CAS + ReviewActor 服务端身份） | 一致 | |
| A57 | 自治指标端点（无数据 null；held_for_human 脏值剔除） | 12号 outcomes_autonomy 挂载存在；生产记录未逐行 | 一致（弱佐证） | 测试直调 handler 为真集成；生产深读记录对该文件覆盖为挂载级 |

### 1.11 支撑（A58-A61）

| # | 承诺 | 生产锚点 | 判定 | 备注 |
|---|---|---|---|---|
| A58 | 多账号轮询（同 wxid 稳定同账号/off_hours 排除/capacity=0 不限/全满 fallback 保送达） | 10号 §2.21 decide_assigned_account 四不变量（account_scheduler.rs:248-305） | 一致 | |
| A59 | 行为信号（dedupe_key 幂等/三类共存/沉默恒 censored=true/T1 元数据/worker 幂等） | 10号 §2.22 behavior_signals + silence worker | 一致 | |
| A60 | LLM 重试（指数退避+Retry-After 取大；429/5xx 重试、400/401 与 JSON 解析失败不重试） | 09号 compute_backoff（llm.rs:1356-1371，封顶 60s、Retry-After 更长胜出）+ is_retryable_llm_error（:1327-1353） | 一致 | llm_retry_jitter 直调生产 pub 函数（非复刻） |
| A61 | last_inbound_at/last_outbound_at 拆分互不覆盖；last_message_at=max | 亲读 gateway.rs:5173-5203（出站 pipeline `$max ["$last_inbound_at", now]`，不动 last_inbound_at）+ 03号 webhook 入站 $set | 一致 | 复刻测试字面已分叉（§3.6）：不变量仍真，守护力弱 |

### 1.12 知识/演化/鉴权（B1-B10）

| # | 承诺 | 生产锚点 | 判定 | 备注 |
|---|---|---|---|---|
| B1 | 一切 AI 产出知识恒 draft+needs_review（chat apply/AI revision 打回/各导入口/merge/rollback 重审/worker add_chunk/修复提案不落库） | 07号 apply_server_owned_lifecycle（chunk_revisions.rs:201-245，AI source 强制降级+confidence=0）+ 08号 红线落点 15 处清单（§4.4） | 一致 | PrincipalAuthorized 是记录在案的唯一例外（视同人签） |
| B2 | auto_verify 过闸结果必经 enforce_verified_needs_human_audit → 恒 needs_human_audit、response verified=0 | 08号 verify.rs:490-496 接线 + :681-686 函数 | 一致 | 16号记录引 verify.rs:401/554 为旧行号（工作区 verify.rs 已修改）——行号漂移非行为漂移，见 §5 |
| B3 | verify 人工专属 + D2 证据闸 + OCC（stale → chunk_revision_conflict 零写）+ 每次 verify/reject 留 revision | 08号 verify_chunk_at_version（verify.rs:60-133：事务+版本绑定+D2 闸；唯一 active+verified 写点） | 一致 | 报表侧 4 处裸 `!is_empty()` 锚口径（台账缺陷#9）测试同样漏报（§4#10） |
| B4 | domain profile：AI 候选恒草稿（seeded_by=generated_by_ai）；publish 只动 published-current，runtime-active 只能显式 activate；AI 草稿 playbook 不可绑定 contact | 11号 guide_profile（version=0/draft/is_active=false）+ 12号 三指针解耦（domain_profiles.rs）+ 亲读 resolve_playbook_for_contact（shared.rs:546-575，filter 带 `release_status:"published"` → draft NotFound） | 一致 | |
| B5 | 未审定知识永不上桌（catalog/open_chunk verified-only；redirect 停在未审新版前；cite⊆opened；fallback 标 weak 不授权；空 corpus 恒 missing） | 07号 §2.1/2.3（visible_chunk_filter、resolve_superseded、filter_answer_against_opened_chunks、route_used_knowledge_ids B2 红线、empty corpus 短路） | 一致 | 已知生产缺陷#4（corpus 200 vs catalog 400 窗口错位，合法引用可被降格 fallback）——测试语料均 <200 条，该分支无守护（§4#3） |
| B6 | exactly-once：import-apply/chat-apply/shared ingest/lesson 晋升并发+重放同 receipt；任一步失败（含审计写失败）全量回滚 | 08号 import_apply_in_transaction（hash 双校验+ready→applying→applied CAS+receipt）+ chat_apply 事务 + ingest 重放检测 + 12号 lesson 晋升事务 | 一致 | |
| B7 | 五套 claim/fencing 同构（import_jobs m056 / ingest_sources m053 含配置代 fencing / catalog_rebuild generation 收敛 / knowledge_chat_tasks claim_lost+stepId 重放 / digest attempt-current 两代） | 08号 import_worker + knowledge_task 两阶段 + digest 世代栅栏；07号 catalog_rebuild + ingest claim；02号 m053/m056 | 一致 | |
| B8 | 多租户/鉴权：一切 filter 带 workspace；ACL 撤销即时 401（cookie 与 JWT 一致）；resolve_authorized_workspace 单闸；缓存按 database 隔离 + bump_generation 立即可见；认证不泄漏存在性；登录/token 共享限速；审计 90 天无 PII；MCP key 不回显；app_id 全局唯一 | 12号 middleware 每请求重查 admin_users+ACL（cookie 与 Bearer 双路径同构）、authenticate 抗枚举 dummy、AuthRateLimiter 共享、审计盐化指纹 90 天；02号 app_id partial unique（indexes L715-733 重复即炸启动）+ Debug 掩码；04号 taxonomy/profile 缓存按 cache_identity 分片 + generation 比对 | 一致 | 16号自认多数 workspace 测试是 filter 形状级、真 Router 证据仅 SR-176/h3——handler 级 IDOR 面整体【弱于】（§4#9） |
| B9 | 版本/单指针六套同构（prompt m043/m034/m049；soul m042；threshold m040；ops 三表 m048+4-tuple；profile 双指针；schema activate_exact_version）：append-only 保历史、恰一 current、并发 publish 一胜一 Conflict、坏数据 fail-closed 先于首写 | 02号 §4 迁移逐条（m040/m042/m043/m045/m048/m049 校验先行、歧义炸启动）+ 09号 publish_version 事务 CAS（prompt_template_versions.rs:208-373）+ load_unique_current fail-closed + soul_versions 同构 + 12号 三表 publish/rollout/rollback | 一致 | |
| B10 | 演化闭环安全：release_prompt 三道闸（禁词→锚→LLM 语义，LLM 不可用 NeedsHumanConfirm 不放水）；threshold auto-release 被 human-release policy 拒且不落 flag；rollback 恢复 status=active、缺历史行中止事务；shadow 基线冻结重跑；retention 探针 snake_case；reset pack 补种 critic | 10号 release.rs 三闸（:539-569）+ auto_release 政策编译期常量 false 三重闸（:36-45）+ 亲读 rollback_prompt（release.rs:1121-1146：恢复基线行 `$set {current_version:true, status:"active"}`，matched≠1 → Err 事务中止）+ replay retention 探针（replay.rs:204-209）+ 09号 prompt_guard 三态 | 一致 | rollback 恢复 status=active 经本记录亲读源码确证（10号记录原文未写明该细节，现已闭环） |

B11（真模型方法论：env-gated/瞬时 skip/非瞬时 panic/MCP 恒桩/判分仪器解耦/CapabilityEvidence）：为测试基础设施自身纪律，无生产对应物，不判定。

**统计**：71 条映射中——【一致】64 条（其中 5 条附"弱于"备注：A5/A30/A57/B5/B8）、【弱于/部分已变】2 条（A11、A4）、【矛盾】1 条（A2，裁决见 §2）、【dry-run 无有效守护】1 条（A10 后半）、另 A41/A61 一致但其命名复刻测试已过期（§3）。

---

## 2. 矛盾裁决详情（亲读两侧源码）

### 2.1 裁决方法

对映射中出现的 5 处"疑似矛盾"逐一亲读测试源码与生产源码（非依赖 15/16 号记录转述），裁决结果：2 处实质矛盾/漂移成立、1 处幻影值成立、2 处裁决为一致（测试仅覆盖部分分支）。

### 2.2 【矛盾成立】autonomy_protocol_pbt P2 revision 模型 vs 生产 revision fallback

- **测试侧**（亲读 tests/autonomy_protocol_pbt.rs:238-465）：`run_revision_loop` 模型在 Proceed 且二轮不过时**恒**返回 `(should_reply=false, "revision_failed")`（:366-369），性质 2 断言 `entered_proceed && second_failing → revision_failed`（:430-443）；注释自称与 `gateway.rs:706-924` 一一对应（写作时快照行号，现行 revision 块在 gateway.rs:3237-3585）。
- **生产侧**（亲读 src/agent/review/gates.rs:1228-1275）：`apply_revision_fallback` 在 `revision_fallback_is_safe_style_only`（risks 全为 `human_like_*`/`emotional_value_*`，或 AllPass 且 `style_diverged`）时把 `review.approved=true`、`final_review_status="revision_applied_approved"`、finalize=Approved——**恢复原稿照发**（gateway 接线：01号 §2.4(14) gateway.rs:3486-3509 二轮不过与 LLM 错/超时路径均走此函数）。
- **裁决**：生产为准。P2 模型写作于 fallback 机制引入之前，缺失该分支；其性质 2 的断言（二轮失败恒不发）在生产最常见的 revision 触发场景（纯 humanLike/emotionalValue 软闸）下与生产行为**相反**。且模型不含 targeted rewrite 路径，"Reply Agent ≤2 次"作为全局上限不成立（首轮+rewrite+revision 串联可达 3 次，rewrite 后 review 软闸失败再触发 revision 的路径在 gateway.rs:3027-3152→3237-3585 上可达）。P2 测试永远绿（自足模型），属假信心。生产 fallback 纯函数有 gates.rs 内嵌单测守护（revision_fallback mod），但"gateway 接线级二轮失败发原稿"无集成测试（§4#6）。

### 2.3 【裁决一致】pressure_risk 0 分豁免语义

- **测试侧**（亲读 tests/pressure_risk_threshold_pbt.rs 全文）：P2 `zero_pressure_risk_passes_legacy` 断言 pressure=0 恒豁免；fixture 的 `claim_analysis` 为 Default（无 `reviewScoreStatus` 键）。
- **生产侧**（亲读 src/agent/review/gates.rs:20-47）：`review_passed` 的 pressure 项 = `(!live_scores_are_valid && pressure==0) || (pressure>0 && pressure<block_at)`；`live_scores_are_valid` 仅当 `claim_analysis.reviewScoreStatus=="valid"`（parse_live_review 成功打标）。测试 fixture 恰落 `!live_valid` 分支 → 与生产一致。
- **裁决**：不构成矛盾。15号承诺 A5 的"0 是 legacy 豁免"表述准确；但 live 有效评分中 0 分**不豁免**（classify 侧另打 `pressure_risk_0_unscored` risk，gates.rs:165-171 一带）这一收紧分支在 tests/ 层无覆盖（lib 内嵌测试有）——判定【一致，测试弱于生产】。boundary_privacy_safety 的 0 豁免同构（gates.rs:45-46）。

### 2.4 【漂移成立】escalation reassign 的 last_pushed_at_ms 语义

- **测试侧**（亲读 tests/escalation_push_time_reassign.rs 全文）：用例 1 用 raw `$set {principal_wxid:"B", last_pushed_at_ms: 改派时刻}` 模拟 reassign，断言"改派须刷新 last_pushed_at_ms 为改派时刻"（:48-69）。
- **生产侧**（亲读 src/agent/escalation/ledger.rs:1074-1122）：`reassign_escalation` 的 update 是 `$set {principal_wxid, principal_account_id, delivery_state=pending_enqueue}` + `$inc generation` + **`$unset {delivery_outbox_id, last_pushed_at_ms}`**；doc 注释明言 "Delivery time is written only after Outbox confirms sent"（:1076）——推送时刻由 `reconcile_principal_card_deliveries_once` 在 outbox 确认 sent 后写入（05号 ledger.rs:241-258）。
- **裁决**：复刻已过期。测试锁的 KD-05 时代行为（改派即置推送时刻）已被"sent 对账口径"取代（对应 15号承诺 A41"每位决策人从 sent 对账时刻起拿完整超时窗"，由 ask_human_phase1_e2e 真调生产函数守护）。该测试用例 1 测的是自己发的 update，生产 reassign 若改坏它也不红——假信心实锤；其"created_at 不被改派篡改"断言仍与生产一致（生产不动 created_at）。用例 2（m031 回填）直调真实迁移函数，非复刻，有效。

### 2.5 【幻影值成立】dry_run_isolation 的 status="completed"

- **测试侧**（亲读 tests/dry_run_isolation.rs 全文）：用例 2 直插 `AgentCommandRun{status:"completed"}` 并断言读回 "completed"。
- **生产侧**（grep 亲验 src/models.rs:4063-4081）：`ALLOWED_AGENT_COMMAND_RUN_STATUS = [pending_confirmation, running, dry_run, succeeded, failed, execution_unknown, canceled]`——**无 "completed"**；生产写点均过 `validate_agent_command_run_status` 断言（11号 §4.2），正常终态是 `succeeded`。测试直插 typed 模型绕过写点断言，锁定的是生产永不会写的幻影状态值。
- **裁决**：漂移成立（状态字典级）。同文件用例 1 亦为"自插审计行 + 断言空库无业务写"的自我实现结构——它从未驱动 management 的 `should_dry_run_tool` 分支，dry-run 真隔离行为无测试（§4#8）。

### 2.6 【裁决一致】rollback_prompt 恢复 status=active

- 16号锁定的承诺（evolution_rollback_status.rs）："rollback 把 previous_version 行置回 current 时必须一并恢复 status=active；历史行被物删则中止事务"。10号生产记录只写了"恢复 proposal 冻结的基线行"，未提 status。
- **生产侧**（亲读 src/evolution/release.rs:1095-1146）：步骤 2 归档 current（`$set {current_version:false, status:"archived"}`）；步骤 3 恢复基线行 `$set {current_version:true, status:"active", updated_at}`（:1131-1134），`matched_count != 1` → `Err(InvalidStatus)` 事务中止（:1142-1146）。
- **裁决**：一致；10号记录该细节缺失，已在本记录补全（§5 回写项）。

---

## 3. 复刻式测试漂移核验（逐个：测试逻辑 vs 生产逻辑当前是否一致）

15号疑点 3 列出 6 处复刻式测试 + 疑点 4 的 P2 模型 + 本次新识别的 dry_run_isolation，共 8 处逐个亲读核验：

| # | 测试文件 | 复刻对象（生产） | 当前是否漂移 | 详情 |
|---|---|---|---|---|
| 1 | decision_review_status_e2e.rs（亲读全文） | routes/reviews.rs `fetch_run_status`（亲读 :337-391） | **未漂移（核心）** | 复刻的关联逻辑（按 run_id 查 run log、顶层 `final_review_status` 空串→None、review doc `holdCategory` 空串→None）与生产逐句吻合。差异：生产 filter 多 `workspace_id+account_id` 两键、另投影 autonomy_protocol（9 键 R1.1 字段，:301-325）与 outbox_status——测试不覆盖（生产删掉租户 filter 或投影字段，测试不红） |
| 2 | media_asset_send_integration.rs（亲读 :1-194） | decision.rs `build_sendable_assets_filter`（亲读 :1928-1939）+ `load_sendable_assets`（:1759-1783） | **未漂移** | 复刻 filter 四键（workspace/$or account/sendable:true/review_status:"approved"）+ sort updated_at:-1 limit 30 与生产逐键一致；stage 命中谓词复刻与 media_send.rs `filter_sendable_candidates` 语义一致（lib 单测另有本体覆盖）。Test 2 走真 enqueue 公开入口（非复刻） |
| 3 | referral_card_push_integration.rs（亲读 :1-110） | decision.rs `build_referral_cards_filter`（亲读 :1787-1801） | **未漂移** | 复刻 filter（workspace/$or/enabled:true/review_status:"approved"）与生产逐键一致；outbox 幂等段走真 enqueue |
| 4 | campaign_segment_coverage.rs（亲读全文） | campaigns.rs `build_segment_coarse_filter`（亲读 :33-72） | **未漂移** | 手工 $elemMatch（productId $in + $and[$or[verification 白名单\|$exists:false], eventKind $ne reversal]）与生产逐键一致；m030 两用例直调真迁移函数非复刻。customer_stage 粗筛分支未覆盖（弱于） |
| 5 | escalation_push_time_reassign.rs（亲读全文） | escalation/ledger.rs `reassign_escalation`（亲读 :1074-1122） | **已漂移（实锤）** | 测试模拟 `$set last_pushed_at_ms=改派时刻`；生产是 `$unset last_pushed_at_ms`（送达对账后由 reconcile 写入）。见 §2.4。真行为由 ask_human_phase1_e2e 守护，本文件用例 1 应改写或退役 |
| 6 | last_inbound_split.rs（亲读全文） | webhooks.rs 入站 $set + gateway.rs `send_outbound_message` pipeline（亲读 :5173-5203） | **字面已分叉（语义等价）** | 测试出站用 `$cond [$gt $last_inbound_at now]`；生产用 `$max ["$last_inbound_at", now]`——对 null/missing/存在三态结果等价，但写法已分叉（测试注释还说"与实际写法一致"）。生产另写 last_agent_run_at/updated_at/last_outbound_style 测试不碰。不变量（出站不动 last_inbound_at、last_message_at=max）当前仍真；生产改坏时本测试不红 |
| 7 | dry_run_isolation.rs（亲读全文） | management.rs dry-run 分支（`should_dry_run_tool` 等） | **已漂移（状态字典）+ 自我实现** | 见 §2.5：status="completed" 为闭集外幻影值（生产 succeeded）；两用例均自插自读，从未驱动生产 dry-run 分支 |
| 8 | autonomy_protocol_pbt.rs P2（亲读 :150-465） | gateway.rs revision 控制流 + gates.rs `decide_revision`/`apply_revision_fallback`（亲读 :1228-1275） | **已漂移（实锤）** | 见 §2.2：模型缺 fallback 分支，性质 2 断言与生产行为相反；注释行号引用（706-924）过期。`decide_revision` 前置条件部分（Proceed/Skip 判定）仍与生产一致 |

**结论**：8 处复刻中 3 处实质漂移（#5、#7、#8），1 处字面分叉（#6），4 处当前未漂移（#1-#4，但结构上生产单方面改动均不会使其变红——复刻式测试的固有假信心在全部 8 处成立）。

---

## 4. 生产行为无测试守护清单（按风险排序）

下表 = "改坏了没有任何测试会红"的生产行为。来源：11 条已核证缺陷中无测试覆盖者 + 两个空壳测试 + 本次映射/复刻核验新发现。**这是后续改动最需要人工盯防的区域地图。**

| # | 无守护行为 | 生产锚点 | 风险 | 依据 |
|---|---|---|---|---|
| 1 | **deferred_wake 醒来任务被 daily_limit/rate_limited/cooldown 拦时取消而非重排**——静默时段客户消息可能永远得不到回复 | gateway.rs:2380-2394（仅 quiet_hours_deferred 重排）+5263-5296 | 高 | 台账缺陷#1（[A] 亲证）；quiet_hours_deferral 只锁重排语义，无"醒来任务撞频控"用例 |
| 2 | **毒丸消息行**：pending handoff 行 decode 失败 → `tick`/`tick_inbound_replies` 每轮中止，两 worker 静默瘫痪且熔断不触发 | webhooks.rs:819-822 + tasks.rs:1069,1121 | 高 | 台账缺陷#2；无任何测试构造坏形状行 |
| 3 | **知识窗口错位**：router corpus 200 条静态序 vs agent catalog 400 条相关度序，verified>200 后合法引用被静默降格 fallback，可致 R5.4 误拦 | knowledge_router.rs:74-78,752-762 + knowledge_agent.rs:81 | 高（随规模恶化） | 台账缺陷#4；全部知识测试语料 <200 条，该分支零覆盖 |
| 4 | **HP-1 任务 stale 回收端到端**（精确 token/generation/claimed_at CAS、累计 3 次转 failed、心跳窗口竞态） | tasks.rs:789-977 | 高（崩溃恢复主链） | worker_reclaim.rs 名不副实（15号疑点2：只 insert 后断言 running，未驱动 reclaim）；16号未补 |
| 5 | **GATE-1 revision 后动作闸复检**（revision 整条替换 decision 后再过 apply_state_action_gate） | gateway.rs:3472-3485 | 中-高 | 空壳测试 revision_recheck_action_gate.rs（零断言，台账缺陷#8） |
| 6 | **revision 失败安全回退发原稿**（gateway 接线级：二轮不过/LLM 错/超时 → 纯风格 trigger 恢复原稿照发） | gateway.rs:3486-3582 + gates.rs:1258-1275 | 中-高 | §2.2：P2 模型与之相反；gates.rs 纯函数有 lib 单测，gateway 接线无集成用例（改坏接线如恒 fail-closed 或恒放行，均无测试红） |
| 7 | **manual_send 被二次安全门 not_managed_at_send 拦截**（与 contact 状态门豁免矛盾，撤管竞态取消 admin 已确认发送） | outbox_dispatcher.rs:922-928 vs 2741-2748 | 中 | 台账缺陷#5；outbox 23 个测试无 manual_send+撤管竞态用例 |
| 8 | **双脑 second reviewer parse 失败拉闸整个 run**（与注释"不应成为新故障源"矛盾） | review/mod.rs:4409-4416 | 中（仅双脑开启时） | 台账缺陷#3；REVIEWER_DUAL_ENABLED 无集成测试，gates.rs lib 测试只盖分歧分类 |
| 9 | **memory_card OCC 并发语义**（两 writer 恰一胜、输者静默跳过、门控外 updated_at 不套版本谓词） | gateway.rs:6945-7004 | 中 | 空壳测试 memory_card_write_occ.rs（台账缺陷#8）；lib 层仅 filter 形状单测 |
| 10 | **management dry-run 真隔离**（should_dry_run_tool：非只读工具 dry-run 下不执行、只读照常执行） | management.rs:2028-2057 | 中 | §2.5/§3.7：dry_run_isolation 自插自读；lib 仅 tool_call_status_for_outcome 局部覆盖 |
| 11 | **handler 级跨租户 IDOR 面（大多数管理端点）**：outbox 取消/状态策略/评测场景/guides/souls publish/command_runs 等的真实 Router 级隔离 | routes/* 各 handler | 中 | 16号疑点2 自认：filter 形状测试不得外推；真 Router 证据仅 SR-176（3 端点）+h3（2 handler）；一致性依赖"thin wrapper 注入 workspace"约定 |
| 12 | **审计身份完整性**：evolution 灰度旗 updated_by 取请求体可伪造（手边有 Extension(admin) 未用）；EVO-2 released_by=真实操作者自认"代码审查保证" | evolution.rs:742-746 | 中 | 台账缺陷#6 + 16号疑点4；无测试 |
| 13 | **B-01 兜底去抖尾窗双发**（guard 到多段 enqueue 循环间 10-100ms/段，新入站落窗内客户收两批） | gateway.rs:4228-4234 | 中低（官方列专项不修） | 注释自认；无测试（按设计不修，但改动该区域时无红灯提示） |
| 14 | **run envelope 终态吸收性**：update_run_envelope_terminal 无 lifecycle 转移 CAS，迟到写可把 completed 覆写为 failed_after_decision | run_envelope.rs:657-743 | 中低 | 06号疑点1；is_valid_lifecycle_transition 纯函数有测试但写路径不调用它 |
| 15 | **hold 请示被骚扰门拦时零台账**（与注释"pending 可由 admin 处置"不符）；**delivery_unknown 请示卡不进超时改派**（长期滞留无告警） | escalation/mod.rs:231-233；ledger.rs:1136 | 中低 | 05号疑点3/7（[C] 级）；15号承诺 A41 测试不覆盖这两分支 |
| 16 | **ingest 正向拉取成功链路**（真实 HTTP fetch→解析→落库 draft）零集成覆盖 | ingest_worker.rs 正向分支 | 低（worker 默认关） | 台账缺陷#11；smoke 四测试全为 SSRF 拒绝路径 |
| 17 | **锚点口径报表漏报**：crud/verify/digest_inbox/catalog 四处裸 `!is_empty()` 与 B3 统一函数不一致，畸形锚在报表/队列漏报 | crud.rs:547 等四处 | 低-中 | 台账缺陷#9；16号 sr132 测试按现行（同样口径的）行为锁定，改对反而可能红 |
| 18 | **replace_content_asset_file 换文件三副作用接线**（清 media_id+退 draft+新 file_*） | media_assets.rs:500-709 | 低 | 15号疑点5（Multipart 无法集成构造）；file_replace_effects 纯函数有 lib 测试，接线审查代测 |
| 19 | **fast 契约下的协议完整性收窄**：validate_reply_critical 不查 R1.3 七字段/should 侧理由/R1.5 长度，finalize insufficient_detail 分支主链不可达 | types.rs:799-889 + gates.rs:667-698 | 低-中 | §1.1 A4：P1/P4 锁的完整契约主链已不消费；若有人依赖"critical 轮推理深度拦截"，生产并不发生且无测试提醒 |

另注两处**测试本身给出假信心**（非生产缺陷但属守护失效）：escalation_push_time_reassign 用例 1 与 autonomy_protocol_pbt P2（§3#5/#8），以及 dry_run_isolation 的幻影状态值（§3#7）。

---

## 5. 需回写修正清单

1. **15号 §3 承诺 2（本记录 A2）**：应改为"Reply Agent 调用上限 2 次仅指 revision 子流程（P2 模型口径）；生产 targeted rewrite + revision 串联可达 3 次；且 P2 模型缺 apply_revision_fallback 分支，其'二轮失败恒 revision_failed'断言与生产相反（gates.rs:1258-1275 亲证）"。
2. **15号 §5 疑点 3**：由"可能脱节"升级为核证结论——escalation_push_time_reassign **已实质漂移**（生产 `$unset last_pushed_at_ms`，ledger.rs:1111-1114 亲证）；last_inbound_split 出站写法**字面已分叉**（`$cond/$gt` vs 生产 `$max`，gateway.rs:5184-5186 亲证，语义等价）；其余 4 处（decision_review_status/media_asset_send/referral_card_push/campaign_segment_coverage）当前逐键一致未漂移。
3. **15号 §5 疑点 4**：P1 的补充——`validate_and_promote` 完整契约在生产主链已被 `validate_reply_critical`（fast 契约）取代（decision.rs:1345 亲证调用点），P1/P4 锁定的完整校验主链不消费；承诺 4 覆盖面应加"仅 fast 契约字段"限定（与台账偏差表#2 呼应）。
4. **15号 2.56 / dry_run_isolation**：应补注"用例 2 的 status='completed' 不在生产闭集 ALLOWED_AGENT_COMMAND_RUN_STATUS（models.rs:4063-4071 亲证），生产终态为 succeeded；两用例均未驱动生产 dry-run 分支"。
5. **16号 §4 A2（本记录 B2）**：verify.rs 行号更新——enforce 接线现于 verify.rs:490-496、函数于 :681-686（16号写的 :401/:554 为记录撰写时行号，工作区 verify.rs 在未提交修改中已漂移）；行为无变。
6. **10号 §2.13 release_prompt/rollback_prompt**：补充"rollback 恢复基线行时同时 `$set status:'active'`（release.rs:1131-1134），且 matched≠1 中止事务——与 evolution_rollback_status 测试承诺一致"（10号原文未写 status 恢复细节）。
7. **总台账（PROJECT_UNDERSTANDING_LEDGER.md）五、清单**：可追加两条 [A] 级："复刻式测试实质漂移 2 处（escalation_push_time_reassign 用例1、autonomy_protocol_pbt P2）+ 幻影状态值 1 处（dry_run_isolation 'completed'）"；"生产行为无测试守护清单见 28 号 §4（19 条）"。
8. **建议的测试侧修复方向**（供后续任务，非本任务执行）：escalation_push_time_reassign 用例 1 改为直调 `reassign_escalation`（pub(crate) 需测试可见性方案）或改断言 `$unset` 语义；dry_run_isolation 改走 post_management_message dry-run 会话真路径；P2 模型补 fallback 分支或降格注释为"仅 Skip/NotEligible 路径模型"；两个空壳测试（revision_recheck_action_gate / memory_card_write_occ）落实实现。

---

## 6. 覆盖自证

**输入记录（全文读）**：15号（§3 总表+§5 疑点+§2 相关小节，168K 字符中读取承诺表/偏差/基础设施节）、16号全文、01/03/04/05/06/07/08/10/11 号全文、README、PROJECT_UNDERSTANDING_LEDGER.md 全文；09/02/12 号按承诺映射所需定向提取（grep 命中段：09号 LLM 重试/prompts/prompt_guard/soul_versions/error 映射；02号 迁移 m034-m058/审批闸/app_id 索引/闭集；12号 auth 全链/suspected_deals/ops_versions/domain_profiles 三指针）。

**亲读源码（本记录裁决与核验用）**：
- 测试侧 8 文件：tests/decision_review_status_e2e.rs（全文）、tests/last_inbound_split.rs（全文）、tests/media_asset_send_integration.rs（:1-194）、tests/referral_card_push_integration.rs（:1-110）、tests/campaign_segment_coverage.rs（全文）、tests/escalation_push_time_reassign.rs（全文）、tests/dry_run_isolation.rs（全文）、tests/autonomy_protocol_pbt.rs（:150-465）、tests/pressure_risk_threshold_pbt.rs（全文）。
- 生产侧：src/routes/reviews.rs:295-391（fetch_run_status+autonomy_protocol_from_decision）；src/agent/gateway.rs:5165-5205（send_outbound_message 时间戳 pipeline）；src/agent/decision.rs:1755-1825,1920-1939（load_sendable_assets/build_referral_cards_filter/build_sendable_assets_filter）；src/routes/campaigns.rs:33-124（coarse+精筛）；src/agent/escalation/ledger.rs:1074-1122（reassign_escalation）；src/agent/review/gates.rs:20-47,1228-1275（review_passed/apply_revision_fallback）；src/evolution/release.rs:1095-1146（rollback_prompt 恢复段）；src/routes/shared.rs:546-575（resolve_playbook_for_contact）；src/models.rs:4063-4081（command run status 闭集，grep 上下文）。

**边界与诚实声明**：
- 判定以 01-12 号生产记录（各自声明 100% 逐行覆盖 + 主会话抽查）为生产行为权威，矛盾候选处一律亲读源码，未出现"记录与源码不符"的情况（记录质量佐证）。
- 工作区含 47 个未提交修改文件（含 src/agent/*、tests/* 多个本记录引用的文件），全部行号为工作区版本；与 main 分支可能不同。
- A57（outcomes 端点）、A30（outbox 投影）两条生产记录覆盖粒度为挂载/摘要级，判定标注"弱佐证"。
- B11 不判定（测试自身纪律）。71 条映射全部给出判定，无遗漏。
