# gateway.rs 深读记录（核证日期 2026-08-13）

> 对象：`src/agent/gateway.rs`，共 9152 行（`wc -l` 核证）。本记录基于 2026-08-13 当场逐行读取（13 段连续 Read，无跳读），所有 `file:line` 均为当日行号。引用外部文件的少量常量/枚举（`review/gates.rs`、`run_envelope.rs`、`types.rs`）已当场 Grep 核证并注明出处。
>
> 一句话定位：本文件是用户运营 Agent 的**编排大脑**——三类触发（webhook 入站 / 跟进任务 / 管理手动发送）统一在此走完 precheck → 决策（渐进三档）→ 独立评审 + ClaimGate → finalize 硬门 → 状态动作闸 → single-shot revision → taxonomy 软闸 → 去抖/claim 复核 → outbox 入队（文本分段/素材/名片）→ 授权 CAS → 审计落库 的全链路。真正的 MCP 发送由 outbox dispatcher 异步完成（本文件仅保留 `send_outbound_message` 供 dispatcher 调用）。

---

## 1. 文件总地图（按行号区段，全部顶层项）

| 行号区段 | 项 | 一句话职责 |
|---|---|---|
| 1-19 | 模块文档注释 | 声明本模块职责总览（入口、precheck、发送、apply、审计写入） |
| 21-94 | `use` 导入 | 依赖 budget/decision/escalation/guards/knowledge_router/memory/multimodal/outbox/review/run_envelope/runtime/taxonomy/types 等子模块 |
| 96-111 | `fn reply_has_time_commitment_feature` | 弱启发：reply 是否含 8 个时间承诺词（仅⑥观测，不进门） |
| 113-123 | `fn existing_outbox_covers_decision` | 判定既有 outbox 行（同 decision_id 且状态∈{pending,in_flight,sent,delivery_unknown}）是否已覆盖本决策 |
| 125-143 | `pub(crate) fn build_commitment_push_update` | CONC-2：commitments 原子追加 `$push+$each+$slice:-8`（保最新 8 条） |
| 145-151 | `pub async fn handle_managed_message` | webhook 入站外部入口 → `run_user_operation_gateway` |
| 153-172 | `pub async fn handle_managed_message_aggregated` | 同上，多带 `should_abort_send` 去抖守卫（调度器聚合多条入站时用） |
| 174-176 | `pub async fn handle_follow_up_task` | 无 claim 的兼容入口 → `handle_follow_up_task_with_claim(None)` |
| 178-218 | `pub async fn handle_follow_up_task_with_claim` | 任务分派：`principal_decision_relay`→escalation 模块；durable inbound kind→专用处理；否则查 contact 走 gateway |
| 220-359 | `async fn handle_durable_inbound_reply_task` | durable webhook 任务：claim 100ms 监视器 + 并行/串行 reaction + gateway |
| 361-415 | `pub async fn send_contact_message_gateway` | 管理 Agent 手动发送外壳：envelope started → budget scope → inner → settle |
| 417-432 | `type GatewayRunInputs` / `ManualRunInputs` | 并行加载快照的元组类型别名 |
| 434-459 | `fn load_gateway_run_inputs` | BoxFuture 并行加载 6 项主链 run 快照（防 future 尺寸撑爆栈） |
| 461-481 | `fn load_manual_run_inputs` | 管理发送专用并行加载 5 项 |
| 483-489 | `struct GatewayBusinessInputs` | 业务快照集合（产品/soul/素材/名片/回复 prompts） |
| 491-555 | `fn load_gateway_business_inputs` | 并行加载业务快照；产品受 `transaction_facts_enabled` 闸、名片受 assist_on 闸、soul 受 override 短路 |
| 557-596 | `fn route_gateway_knowledge` | 知识路由外壳：预算超额→空路由+降级标记，否则 `route_operation_knowledge` |
| 598-661 | `fn review_and_evaluate_claim_gate` | Reviewer 与独立 ClaimGate 用 `tokio::join!` 并行（输入相同、判定独立） |
| 663-673 | `fn concise_review_detail` | 评审摘要归一化+按字符截断（加 `…`） |
| 675-698 | `fn normalize_manual_send_review_terminal` | 管理发送无 revision 通道：finalize=Approved 但软闸失败 → 强制翻 `held_by_ai_policy` |
| 700-718 | `fn manual_send_block_reason` | 拦截状态→运营可读文案映射 + 拼接 160 字符评审摘要 |
| 720-1295 | `async fn send_contact_message_gateway_inner` | 管理发送主流程（详见 §2.5） |
| 1297-1319 | `fn trigger_envelope_source` | trigger → (source_event_id, source_kind)：Inbound→message_id/`synthetic:`+`inbound_message`；FollowUp→task_id.hex+`follow_up_task` |
| 1321-1343 | `fn non_text_inbound_type` | 非文本入站判定：仅 Inbound、msg_type 存在且非 "text"/空 |
| 1345-1579 | `async fn maybe_handle_non_text_transition` | F2 非文本过渡话术：入 outbox（task 路径先落 minimal review + claim 绑定），返回 true 则调用方 early-return |
| 1581-1636 | `struct ReactionCompletion` / `enum ReactionTaskState` / `struct ParallelReactionTask` | 并行 reaction 任务的一次性 join 封装（Mutex<Running/Complete>） |
| 1638-1726 | `struct ReactionStopBarrier` / `async fn abort_on_reaction_stop` | reaction 安全汇合：stop_requested 时 cancel task + 写事件/run log + 终止 run |
| 1728-1744 | `pub(crate) async fn run_user_operation_gateway` | 主入口薄封装 → `_with_parallel_reaction(None)` |
| 1746-1810 | `async fn run_user_operation_gateway_with_parallel_reaction` | envelope started → runtime/thresholds → RunBudget task-local scope → inner → settle |
| 1812-1915 | `async fn settle_gateway_execution` | RUN_AUDIT_BUFFER scope + catch_unwind；join reaction；错误/panic 关 envelope；flush LLM/事件审计；写 performance；panic resume |
| 1917-1925 | `fn panic_payload_message` | panic payload → String（&str/String/其它） |
| 1927-2024 | `pub(crate) async fn trigger_principal_escalation` | 请示通道：骚扰门+去重+落 pending 台账+推卡（详见 §2.9） |
| 2026-2146 | `pub(crate) async fn relay_principal_decision_to_customer` | 领导裁决转述：A 类豁免先写→合成 relay 消息走 gateway→B 类知识提案（详见 §2.10） |
| 2148-2158 | `enum SendOrder` / `fn media_send_order` | 素材定序纯函数：当前恒 TextThenMedia |
| 2160-2168 | `fn media_send_allowed` | 媒体发送门 = `outbox_eligible && has_assets`（与文本同源） |
| 2170-2175 | `fn should_run_send` | 去抖覆盖判定 = `outbox_eligible || media_pending` |
| 2177-2249 | `async fn apply_state_action_gate` | GATE-1 状态动作闸：forbidden/allowlist 不含本 action → 翻 `held_by_ai_policy` + 事件 |
| 2251-2335 | `async fn ensure_customer_acknowledged` | 客户回应保障：Inbound 且状态不在豁免清单 → 入队确定性安抚占位（fail-soft） |
| 2337-2347 | `fn segment_idempotency_base` | 分段幂等 key base：source_event_id 非空用之，空回落 run_id |
| 2349-2359 | `fn text_send_eligible` | `should_reply && reply_text 非空白` |
| 2361-5021 | `fn run_user_operation_gateway_inner` | **主链路核心**（约 2660 行，详见 §2.4） |
| 5023-5050 | `enum SendReceiptStatus` / `fn classify_send_receipt` | MCP 回执三态：Succeeded / ExplicitlyFailed（可安全重试）/ Inconclusive（禁止重放） |
| 5052-5205 | `pub(crate) async fn send_outbound_message` | 实际 MCP `message_send_text`；成功后落 conversation_messages + contact 时间戳（失败仅审计不返 Err，防重发） |
| 5207-5235 | `pub(crate) fn trigger_message` | trigger → 决策用 ConversationMessage（FollowUp 合成措辞由 `follow_up_trigger_message_text` 决定） |
| 5237-5243 | `pub(crate) fn inbound_marker_for_context_check` | `last_inbound_at` 优先，缺失回落 `last_message_at` |
| 5245-5366 | `pub(crate) async fn precheck_send_gateway` | 发送前门（9 道，详见 §4.2 豁免矩阵） |
| 5368-5420 | `async fn precheck_operation_policy` | 联系人级策略门：policy_cooldown / policy_wait_user_reply / policy_consecutive_limit |
| 5422-5424 | `fn trigger_resets_consecutive_outbounds` | 真实/影子入站（有 message_id 或 dedupe_key）重置连续出站计数；手动发送合成入站不重置 |
| 5426-5450 | `async fn consecutive_outbound_count` | 最近 20 条消息中自最新起连续 outbound 数 |
| 5452-5510 | `pub(crate) fn split_reply_into_segments` | #68 回复拆条：双换行→单换行→句末标点，段数超限尾部合并 |
| 5512-5542 | `fn split_long_segment` | 超长段按句末标点/2×max 硬切兜底 |
| 5544-5559 | `const ACK_PLACEHOLDER_EXCLUDED_STATUSES` | 占位豁免黑名单 8 项 |
| 5561-5571 | `pub(crate) fn should_send_ack_placeholder` | 仅 `trigger_kind=="inbound"` 且状态不在黑名单才补占位 |
| 5573-5605 | `pub(crate) fn build_ack_enqueue_request` | 占位 EnqueueRequest：key=`{source_event_id}#ack-placeholder`、decision_id=None |
| 5607-5616 | `pub(crate) fn blocked` | SendGatewayResult 拦截构造器 |
| 5618-5623 | `pub(crate) fn daily_limit_applies_to` | daily_limit 仅约束 FollowUp |
| 5625-5635 | `const PROACTIVE_TOUCH_SOURCE_KINDS` | 主动触达 source_kind 闭集 = ["follow_up","follow_up_task"] |
| 5637-5683 | `pub(crate) fn proactive_touch_filter` | B1 主动触达计数 filter（status∈{sent,delivery_unknown}、时间 $or 双字段） |
| 5685-5704 | `async fn daily_touch_count` | 滚动 24h `distinct run_id` = 逻辑触达次数 |
| 5706-5730 | `async fn account_daily_sent_count` | ④账号级当日 sent 总量（软上限告警用，不按 contact 过滤） |
| 5732-5737 | `fn utc_today_start_millis` | UTC 当日 0 点毫秒 |
| 5739-5768 | `async fn cancel_task` | task → cancelled + gateway_status + $unset claim 字段 |
| 5770-5809 | `async fn reschedule_task` | #69 静默重排：→pending + 新 run_at + `attempt_count:-1` + $unset claim |
| 5811-5832 | `pub(crate) fn detect_state_transition` | operation_state 有效迁移判定（trim 归一，同值/空 next → None） |
| 5834-5866 | `struct ProfileChurnReport` / `PROFILE_SUMMARY_SOFT_CAP=2000` | 画像写侧抖动纯观测报告 |
| 5868-5944 | `pub(crate) fn compute_profile_churn` / `fn flip_of` | 抖动量化纯函数（丢标签/stage/intent 翻转/summary 增长） |
| 5946-5986 | `MEMORY_SUMMARY_MAX_LINES=12` / `MAX_BYTES=1200` / `pub(crate) fn merge_memory_summary_dedup_capped` | 短期记忆整行去重 + 行数/字节双封顶（保新丢旧） |
| 5988-5998 | `pub(crate) fn stage_realtime_write_allowed` | customer_stage 实时写入仅 Strong 证据放行 |
| 6000-6034 | `fn build_observed_dimensions` | 贝叶斯观察映射：强证据数按 Inbound 消息方向代码侧计算，截断 MAX_BAYESIAN_SLOTS |
| 6036-6054 | `struct ProjectionWriteGuard` / `enum AgentUpdateOutcome` | 投影写 fencing 契约（profile_revision + review_id 单调）；Applied/AlreadyApplied/FencedConflict |
| 6056-6080 | `async fn write_agent_update_event` | 投影事件带 `post_projection:{review_id}:{effect}` dedupe key |
| 6082-6163 | `async fn upsert_pending_projection_observation` | pending 观察行 upsert（并发 dup-key 兜底）+ projection_observations 台账计数 + reconcile |
| 6165-6897 | `pub(crate) async fn apply_agent_updates` | 决策后画像/状态写库（由 post_decision 投影 worker 调用，详见 §2.7） |
| 6899-7005 | `pub(crate) async fn apply_operating_memory_update` | 记忆候选/标签观察/consolidation 调度 + memory_card OCC 写 |
| 7007-7033 | `build_decision_event_details` / `review_event_details` / `simulation_gateway_document` | 事件 details 构造纯函数 |
| 7035-7150 | `pub(crate) async fn write_decision_review` | decision_reviews 落库（prompt_versions 快照 + run-local 覆盖；`expected_text_segments` 仅 outbox_enqueuing 时计算） |
| 7152-7189 | `async fn write_agent_run_log` | 薄封装 → `_with_finalize(FinalizeRunLogFields::default())` |
| 7191-7215 | `struct FinalizeRunLogFields` | run log 终态字段包（finalReviewStatus/autonomy/revision/critique/source envelope） |
| 7217-7298 | `async fn write_agent_run_log_with_finalize` | 终态校验（final_review_status/gateway_status/lifecycle 闭集）+ budget snapshot + `update_run_envelope_terminal` |
| 7300-7326 | `async fn persist_finalize_pending_events` | finalize 纯函数产出的 pending events 逐条写 agent_events |
| 7328-7350 | `pub(crate) fn apply_confidence_override` | operation_state_confidence < 阈值 → review_mode=full |
| 7352-7364 | `fn uncovered_inbound_watermark_filter` | (created_at,_id) 复合水位 $or filter（含/不含边界） |
| 7366-7453 | `pub(crate) async fn load_recent_messages` | 最近消息（created_at,_id 双键降序）+ durable 任务未覆盖 inbound 水位合并去重 |
| 7455-7462 | `pub(crate) async fn load_context_messages` | 管理发送上下文窗：`(recent_message_limit*6).clamp(24,80)` |
| 7464-7489 | `pub(crate) async fn load_pending_tasks` | 该 contact pending 任务（run_at 升序，limit 5） |
| 7491-7504 | `pub(crate) fn llm_signal_apply` | DimValidation → Accept(canonical)=Some；Drop/Reject=None（Reject 兜底当 Drop） |
| 7506-7531 | `fn extract_relationship_type_suggestion` / `fn extract_suspected_deal_signal` | 从 agent_generated_signals 提取第一个对应 kind 信号 |
| 7533-7590 | `pub async fn write_event_for_account` / `pub(crate) async fn write_event_for_account_with_dedupe` | agent_events 写入：优先进 run_audit buffer，直写时 dup-key 视为成功 |
| 7592-7608 | `pub(crate) fn check_context_changed_followup_pure` | last_inbound_ms > task_created_ms（严格大于） |
| 7610-7620 | `pub(crate) fn follow_up_trigger_message_text` | deferred wake→被动应答措辞；普通 follow_up→主动触达措辞 |
| 7622-7659 | `struct TaxonomyGuardOutcome` / `pub(crate) fn pick_dimension_display_name` | taxonomy 软闸输出形状；中文名取值（snake 优先、camelCase 回退） |
| 7661-7708 | `pub(crate) fn compute_taxonomy_guard_outcome` | 4 路命中：Active 静默/AliasActive 改写/Deprecated 记 risk/CandidateNew 写候选+隔离（FSM canonical key 豁免） |
| 7710-7734 | `pub(crate) fn apply_taxonomy_guard_outcome` | 应用改写/risks/隔离（customer_stage 隔离连带清 operation_state 同值） |
| 7736-7771 | `mod send_receipt_tests` | classify_send_receipt 三态单测 |
| 7773-9152 | `mod tests` | 纯函数单测集（占位/分段/幂等 base/churn/memory merge/taxonomy/贝叶斯截断等） |

---

## 2. 逐函数深读

### 2.1 外部入口族

**`handle_managed_message`（gateway.rs:145-151，pub）**
webhook 唯一直调入口。输入 `(state, contact, inbound)`；直接委托 `run_user_operation_gateway(state, contact, Inbound(inbound), None, None)`。无 task_context、无去抖守卫。

**`handle_managed_message_aggregated`（gateway.rs:153-172，pub）**
与上等价，但多传 `should_abort_send: Option<Arc<dyn Fn()->bool>>`。注释（gateway.rs:153-157）：调度器在用户连发多条时只起一个 runner，运行期间若更新入站到达则该判定返回 true，网关在落盘/入队前放弃过时生成。判定是纯查询（读 generation 计数），可多次调用。

**`handle_follow_up_task` / `handle_follow_up_task_with_claim`（gateway.rs:174-218，pub）**
分派逻辑（gateway.rs:184-192）：
- `task.kind == "principal_decision_relay"` → `escalation::handle_principal_decision_relay_with_claim`（领导已裁决，走 relay 转述路径）；
- `task.kind == crate::webhooks::DURABLE_INBOUND_REPLY_KIND` → `handle_durable_inbound_reply_task`；
- 其它：`task.id` 为 None 直接 `Ok(())`（gateway.rs:193-195）；构造 `TaskRunContext::new(task_id, task_claim)`，按 `(workspace_id, account_id, wxid)` 查 contact，查不到返回 `NotFound("follow-up contact not found")`（gateway.rs:196-209），然后走 `run_user_operation_gateway(FollowUp(&task), Some(task_context), None)`。

**`handle_durable_inbound_reply_task`（gateway.rs:220-359，私有）**
durable webhook 投递（入站语义 + task claim 作为发送授权 fence）。
- 输入校验：task.id 必须有（否则 `External`）；`task_claim` 必须有（gateway.rs:234-236）；`task.content.trim()` 必须是合法 ObjectId ——task 快照的 content 存的就是持久化的入站消息 `_id`（gateway.rs:237-239，注释 gateway.rs:221-225：后来的入站会刷新同一 task 行、清旧 claim token，使本协作守卫与最终 Outbox 授权都拒绝过时代际）。
- 加载：`messages` 按 `{_id, workspace_id, account_id, contact_wxid, direction:"inbound"}` 查 inbound（缺→NotFound）；contact 同查（缺→NotFound）（gateway.rs:240-267）。
- **claim 监视器**（gateway.rs:269-293）：`tokio::spawn` 100ms interval 轮询 `task_claim_is_current`；失去所有权 → `claim_lost=true`（Release 序）并退出；查询错误仅 warn。函数尾 `monitor.abort()`（gateway.rs:357）。
- **reaction 并行开关**（gateway.rs:295-342）：`state.config.reaction_gateway_parallel_enabled` 为真 → `tokio::spawn` 跑 `reaction::record_user_reaction_with_outcome`，包成 `ParallelReactionTask::running`；为假 → 先串行跑完，包成 `::complete`。两模式共享同一 stop-signal 安全汇合。
- guard = `claim_lost` 的 Acquire 读（gateway.rs:344-346），作为 `should_abort_send` 传入 `run_user_operation_gateway_with_parallel_reaction`（gateway.rs:348-356）。

**`send_contact_message_gateway`（gateway.rs:361-415，pub）**
管理 Agent 生产发送网关外壳。空 content → `BadRequest`。`run_id=uuid`, `source_event_id="manual:{run_id}"`；`write_run_envelope_started(..., SOURCE_KIND_MANUAL_SEND, SOURCE_KIND_MANUAL_SEND)`（gateway.rs:371-381）。execution 闭包内：加载 domain_config → `UserRuntimeParameters::from_config` → `resolve_thresholds().apply_to_runtime`（阈值热更新）→ 构建 `RunBudget`（run_token_budget / run_max_llm_calls / knowledge_max_tool_calls）→ `RUN_BUDGET.scope(budget, inner)`（gateway.rs:383-413）。最后 `settle_gateway_execution(state, run_id, execution, None)`。

### 2.2 快照加载族

**`load_gateway_run_inputs`（gateway.rs:439-459）**
返回 `BoxFuture`（注释 gateway.rs:434-438：巨型 gateway future 只持一个指针，避免 debug 栈溢出）。`tokio::try_join!` 并行 6 项：`load_recent_messages(limit=runtime.recent_message_limit)`、`load_active_domain_profile`、`load_pending_tasks`、`load_operation_playbook_for_contact`、`load_or_create_operating_memory`、`load_operation_knowledge`。stage_timer="run_snapshot"。

**`load_manual_run_inputs`（gateway.rs:462-481）**
管理发送版并行 5 项：playbook、memory、knowledge、`load_context_messages`（更宽窗口）、active_profile。stage_timer="manual_run_snapshot"。

**`load_gateway_business_inputs`（gateway.rs:491-555）**
`tokio::join!` 并行 5 项：
- products：仅 `active_profile.transaction_facts_enabled` 时加载（否则空 Vec）（gateway.rs:499-505）；
- published_soul：profile 有非空 `soul_override` 时短路 `Ok(None)`，否则 `load_published_soul(workspace,"user")`（gateway.rs:506-519）；
- sendable_assets：`load_sendable_assets` 失败吞掉 `unwrap_or_default`（gateway.rs:520-524）；
- referral_cards：仅 assist_on 加载，失败同样 default（gateway.rs:525-537）；
- reply_prompts：`load_reply_prompt_snapshot`（错误用 `?` 传播，gateway.rs:538/551）。

**`route_gateway_knowledge`（gateway.rs:558-596）**
预算已超（`current_run_budget().is_exceeded()`）→ `mark_degraded("knowledge_route_skipped_budget_exceeded")` + 返回 `empty_knowledge_route(initial_planner)`（reason 改中文说明）；否则 `route_operation_knowledge(...)`。stage_timer="knowledge_route"。

**`review_and_evaluate_claim_gate`（gateway.rs:599-661）**
`tokio::join!(review_decision(...), evaluate_independent_claim_gate(...))`——首稿 Reviewer 与独立 ClaimGate 输入相同、判定独立，并行把墙钟降为较慢者（注释 gateway.rs:2980-2982）。review 的错误 `?` 传播，claim_gate 直接返回其评估对象（本身非 Result）。所有主链/rewrite/revision/管理发送 4 个调用点复用（gateway.rs:848/3002/3128/3398）。

### 2.3 precheck 族

**`precheck_send_gateway`（gateway.rs:5245-5366，pub(crate)）**
门序（**返回第一个命中的 blocked**）：
1. `not_managed`：`contact.agent_status != Managed`（gateway.rs:5251-5253）——**所有触发**（含 relay）都查；
2. `is_relay = escalation::is_principal_relay_trigger(trigger)`（gateway.rs:5259）：relay 豁免 3-8 全部频控门（注释 gateway.rs:5254-5258：占位已把 last_agent_run_at 刷 now，领导秒回必撞 min_reply_interval；relay 是客户期待内被动应答）；
3. `cooldown`：`contact.cooldown_until > now`（gateway.rs:5269-5273）；
4. `precheck_operation_policy`（gateway.rs:5274-5282，详见下）；
5. `rate_limited`：`now - last_agent_run_at < min_reply_interval_seconds*1000`（gateway.rs:5283-5288）；
6. `daily_limit`：仅 `daily_limit_applies_to(trigger)`（=FollowUp）且 `daily_touch_count >= max_daily_touches`（gateway.rs:5292-5296）；
7. `expired`：仅 FollowUp，`task.expires_at < now`（gateway.rs:5302-5308）——注释强调 expired 判定**先于**静默门，过期任务不得被 quiet_hours_deferred 重排复活（gateway.rs:5297-5301）；
8. `quiet_hours_deferred`：仅 FollowUp 且非 deferred_wake，`effective_quiet_hours_enabled(contact, active_profile, runtime.quiet_hours_enabled)` 且 `is_quiet_now(start,end,tz)`（gateway.rs:5319-5340）——Inbound 不受此门（webhook 层已权威处理；边界穿越时放行"刚收到就回"是对的，gateway.rs:5313-5316）；
9. `context_changed`：仅 FollowUp 且非 deferred_wake，`inbound_marker_for_context_check(contact) > task.created_at`（gateway.rs:5342-5357）——此门在 `if !is_relay` 块**外**（relay 是 Inbound 触发，天然不进 FollowUp 分支）。
全过 → `allowed`（gateway.rs:5358-5365）。
`is_deferred_wake` 判定（gateway.rs:5263-5267）：FollowUp 且 `task.kind == quiet_hours::DEFERRED_INBOUND_REPLY_KIND`。

**`precheck_operation_policy`（gateway.rs:5368-5420）**
`contact.operation_policy` 空 → None。三门：
- `policy_cooldown`：`operation_policy.cooldownUntil`（RFC3339 字符串）> now；
- `policy_wait_user_reply`：`requireUserReplyBeforeNextOutbound=true` 且 consecutive_outbounds>0；
- `policy_consecutive_limit`：`maxConsecutiveAgentOutbounds >= 0` 且 consecutive >= 上限。
consecutive 计算：当前触发是"真实/影子客户入站"（`trigger_resets_consecutive_outbounds`，gateway.rs:5422-5424：message_id 或 dedupe_key 非空）→ 0（当前入站本身打断出站连击，且影子消息故意不落库，gateway.rs:5386-5390）；否则 `consecutive_outbound_count`（最近 20 条自最新起连续 outbound 计数，gateway.rs:5426-5450）。手动发送的合成入站 message_id=None → 不重置 → 策略门对管理发送有效。

### 2.4 主链路 `run_user_operation_gateway_inner`（gateway.rs:2361-5021）——重点

外层（gateway.rs:1746-1810）：`run_id=uuid`；`trigger_envelope_source`；`write_run_envelope_started`；execution 内加载 domain_config → runtime（`from_config` + `resolve_thresholds` 热覆盖，gateway.rs:1777-1782）→ `RunBudget::new(run_token_budget, run_max_llm_calls, knowledge_max_tool_calls)` → `RUN_BUDGET.scope(inner)`；`settle_gateway_execution` 收尾。`inbound = trigger_message(&contact,&trigger)`（FollowUp 时合成入站，gateway.rs:1770）。

inner 顺序流（编号即执行序）：

**(1) 第一道 precheck（gateway.rs:2379-2435）**
`!allowed` 时：task 路径下 `quiet_hours_deferred` → `reschedule_task(next_wake_at(quiet_hours_end, tz, wxid, wake_jitter_max_seconds))`（不取消，避免丢承诺，#69，gateway.rs:2382-2390）；其余 → `cancel_task(status, reason)`。写 `agent_skipped` 事件 + `write_agent_run_log(precheck.status)`（context=`{refreshed:false, reason:"precheck_blocked"}`）+ `ensure_customer_acknowledged`。return Ok。

**(2) 非文本提前 reaction 汇合（gateway.rs:2439-2457）**
仅 `non_text_inbound_type(&trigger).is_some()` 时在此提前 `abort_on_reaction_stop(barrier_stage="before_non_text_outbox")`——因为非文本过渡直接创建 Outbox，等不到文本首轮汇合点。命中 stop → return。

**(3) 非文本过渡 `maybe_handle_non_text_transition`（gateway.rs:2464-2476；函数体 1345-1579）**
拦截条件：Inbound 且 msg_type 非 "text"/None/空白（gateway.rs:1333-1343）。`multimodal::fetch_inbound_media` 现打桩恒 None（media_ref 有也只 debug log，仍走过渡话术，gateway.rs:1361-1376）。`multimodal::non_text_transition_reply(msg_type)` 生成话术。
- task 路径（durable inbound）：**先落 minimal decision_review**（status=`outbox_enqueuing`，approved=true，review_summary="非文本入站过渡话术（确定性系统回复）"，expected_text_segments=1，gateway.rs:1383-1436）→ `bind_task_decision_if_owned` CAS，失败 → review 置 `stale_task_claim` + return true（gateway.rs:1437-1448）；
- enqueue：`EnqueueRequest{decision_id, source_event_id, content=reply, max_attempts:3}`。IdempotentSkip 时尝试 `adopt_recoverable_durable_outbox_if_owned`（同 claim 收养旧 decision 的可恢复 outbox）或判 `existing_run_id==run_id && decision 匹配 && 状态∈可交付集`（gateway.rs:1474-1514）；Err → 直接 `?` 上抛；
- task 路径 enqueue 后：`authorize_task_outbox_if_owned` CAS，成功 → review 置 `outbox_enqueued`，失败 → `stale_task_claim` + return true（gateway.rs:1521-1542）；
- 写 `non_text_inbound_transition` 事件 + run log（status=enqueue 结果，decision doc=`{msgType, replyKind:"non_text_transition"}`）→ return true（调用方 early-return，**不进决策 Agent**）。

**(4) 快照加载 + profile 应用（gateway.rs:2479-2524）**
`load_gateway_run_inputs` 并行 6 项 → `runtime.apply_active_profile(&active_profile)`（H14 grounding bypass + reviewer distrust + 五闸阈值覆盖一次性派生，gateway.rs:2481-2485）。`maybe_emit_unverified_warning` fail-soft。`context_pack = effective_memory_card_for_contact(...).to_document()`；`should_refresh_context=false` 硬编码（gateway.rs:2496）；`initial_planner = {risk:medium, review:light, reason:"Reply Agent 内联判断..."}`（gateway.rs:2498-2503）。assist_on 判定（config.assist_mode_enabled + contact 覆盖属性，gateway.rs:2508-2515）→ `load_gateway_business_inputs`。`ReplyContextCache` / `ReviewerPromptCache` 各建一次供全 run 复用。

**(5) 渐进三档首程（gateway.rs:2526-2605）**
`progressive_tier_enabled=true`：知识路由 future 与 **Lean 首程**（空知识、空 chunks）`tokio::try_join!` 并行；
`=false`（kill switch）：先 route → select chunks → **Full 首程** 串行（Full prompt 要消费选中知识）。
两分支都传 `DecisionRunSnapshot{active_profile, active_products, published_soul, sendable_assets, referral_cards, reply_prompts, reply_context}`。产出 `(knowledge_route, decision_first, promote_risks_first)`。
`selected_chunks = select_operation_knowledge_chunks(chunks, route)`；`mark_run_envelope_running(decision_first 快照)`（gateway.rs:2606-2613）。

**(6) 文本主汇合 reaction 屏障（gateway.rs:2617-2634）**
`abort_on_reaction_stop(barrier_stage="after_first_reply")`——reaction 与快照+Lean 首程重叠执行，在任何 escalation/review/mutation/Outbox 之前汇合。stop → cancel task + `user_reaction_stop_requested` 事件 + run log + return（函数体 gateway.rs:1653-1726）。

**(7) 充分性升档（gateway.rs:2636-2910）**
`decide_tier_escalation(&decision_first)` 三态：
- **Enough**：若 `forced_full_reason` 非空（progressive 开启时：有非 fallback 引用知识 / 明确请求引荐且有合格候选 / Lean 自报 knowledge_need，判定入参 gateway.rs:2671-2690）→ `forced_full=true`，写 `ptier_forced_full` 事件，`grant_escalated_ceiling(run_token_budget_escalated)` + `grant_additional_llm_calls(1)`（B-1 放宽判定上限，tokens 仍如实累计，gateway.rs:2721-2729），Full 重生成；否则保留首程，并做两条纯观测：`ptier_coverage_optimism`（自评 enough 但覆盖 weak 且需产品知识）、`ptier_relational_optimism`（enough 停 Lean 但 intent_trajectory 非空）（gateway.rs:2758-2800）。
- **Escalate(target_tier)**：写 `ptier_escalated` 事件 + `grant_escalated_ceiling` → 按 Relational/Full 档重生成（gateway.rs:2803-2852）。
- **Clarify**：第一程已生成澄清向回复，直接用它；写 `ptier_clarify` 事件（含 reply_char_count / has_question_mark 客观量，gateway.rs:2853-2882）。
之后：`tier_used = forced_full?"full" : escalated?"escalated" : "lean"`，`mark_tier` + `ptier_run_tier` 事件（gateway.rs:2884-2910）。另有 `ptier_self_assessment_malformed` 观测（sufficiency 非三态，gateway.rs:2656-2669）。

**(8) tool_calling 防御（gateway.rs:2912-2937）**
单发路径不支持 tool_calling 相位；命中 → 写 `decision_phase_tool_calling_in_single_shot`（degraded）事件，强制 `decision_phase="final"` + 清 tool_calls（should_reply 保持原值，注释说明 build_tool_calling_decision 已清空 reply_text 故 false 安全）。

**(9) 归一化 + planner（gateway.rs:2939-2960）**
`normalize_decision_state`（对 domain_config）→ `normalize_decision_runtime(initial_planner)` → `planner_from_decision`（Reply Agent 单轮内联 planner）→ 路由有选中知识时强制 `knowledge_required=true` 且 review_mode 空则 "full" → `apply_confidence_override`（state 置信 < `operation_state_confidence_full_review_below` → full review，gateway.rs:7328-7350）→ 再 `normalize_decision_runtime(planner)` → `context_pack_version=next_memory_card_version` → **KB-01 口径**：`used_knowledge_ids = resolve_used_knowledge_ids(forced_full, escalated_to_full, route_ids)`——仅 Full 档记路由命中 id，非 Full 一律清空（防 LLM 自报真实 verified ObjectId 架空 grounding 硬闸，gateway.rs:2953-2960）。

**(10) 首稿 review 三分支（gateway.rs:2962-3026）**
`budget_exceeded_for_review = is_llm_or_token_exhausted()`：
- 超额 → `mark_degraded("review_skipped_budget_exceeded")` + `run_budget_exceeded` 事件 + `local_decision_review(decision, budget, runtime)`；
- `should_run_review(decision, planner, runtime)` → `effective_review_mode(...)` → `review_and_evaluate_claim_gate`（并行）存下 `precomputed_claim_gate`；
- 否则 → `local_decision_review`。
budget 为 None 时构造 `RunBudget::new(run_id, i64::MAX, i32::MAX, i32::MAX)` 空预算兜底（gateway.rs:2972-2979）。

**(11) ClaimGate 前置合并 + targeted rewrite（gateway.rs:3027-3152）**
`precomputed_catalog_backed = apply_independent_claim_gate(evaluation, ...)`——在 rewrite 决策前合并独立 manifest，让"开放世界不支持的业务事实"走 targeted rewrite 而非等 finalize 只能整条拦（注释 gateway.rs:3028-3030）。
`should_run_targeted_rewrite`（只接 hallucination/grounding 硬闸；`needs_revision=true` 的软闸失败留给 finalize 后 revision 通道，注释 gateway.rs:3041-3046）：
- 预算：`grant_additional_llm_calls(4 + second_reviewer存在?1:0)`（stage_bundle，gateway.rs:3048-3051）；仍超额 → `run_budget_exceeded`(rewrite) 事件 + 跳过；
- 执行：`mark_rewrite()`；先写一条 decision_review（status=`rewrite_requested`）；`decide_reply_with_promote(Some(&review.rewrite_instruction), PromptTier::Full)`；**namecard 保留**（改写未重新输出引荐则沿用改写前意图，gateway.rs:3117-3123）；归一化 + `used_knowledge_ids=route_ids`（rewrite 恒 Full 档故合理）；重新 `review_and_evaluate_claim_gate(review_mode="full")`；`precomputed_catalog_backed=None`。

**(12) finalize #1（gateway.rs:3154-3212）**
`priced_from_catalog` 三源：precomputed_claim_gate（take）→ precomputed_catalog_backed → 兜底 `ensure_independent_claim_gate`（gateway.rs:3170-3193）。`principal_product_exempted = contact_has_principal_product_exemption(&contact)`（R5.4 第三并联背书）。
`finalize_review_for_send(review, &mut final_decision, runtime, contact, selected_chunks, promote_risks, inbound.content, commitment_markers, priced_from_catalog, principal_product_exempted)` → `FinalizeOutcome{review, status, pending_events}`；`persist_finalize_pending_events` 逐条写事件。任何硬门触发会强制 `should_reply=false + autonomy_mode="blocked"`（注释 gateway.rs:3157-3160）。

**(13) 状态动作闸 GATE-1（gateway.rs:3214-3235；函数体 2177-2249）**
仅 Approved 时执行 `apply_state_action_gate`：`action_policy_state_key(domain_config, contact.operation_state, decision.operation_state)`（提案态仅在合法迁移时采用，防 LLM 臆造态引发 missing policy 错，注释 gateway.rs:2196-2201）；缺省回落 `initial_operation_state_for_contact`。`load_operation_state_policy_for_contact` + `classify_reviewed_decision_action` + `enforce_state_action_policy`；违规 → `review.approved=false`、`final_review_status="held_by_ai_policy"`、`should_reply=false`、`autonomy_mode="blocked"`、追加 risk `state_action_policy_blocked`、finalize_status=Held、写 `state_action_policy_blocked` 事件。老库无 policy 行 → fallthrough 兼容。

**(14) single-shot revision R2（gateway.rs:3237-3585）**
前置：Approved && needs_revision && !should_hold && revision_direction 非空 → `grant_additional_llm_calls(4+second_reviewer)`（gateway.rs:3255-3264）。
D2 风格漂移纯观测：Approved 且无 revision 需求时，`observe_style_continuity(prev_style, new_style)` AuditOnly → `style_consistency_observed` 事件（不改 review，gateway.rs:3269-3310）。
`decide_revision(finalize_status, review, budget_exceeded)` 三态：
- **NotEligible**：跳过；
- **Skip{reason,event}**（方向空/预算超额）：`review.approved=false`、`revision_applied=false`、`final_review_status="revision_failed"`、`should_reply=false`、`derive_revision_failure(reason)` 得 finalize_status、写事件（`revision_skipped_invalid_direction` / `revision_skipped_budget_exceeded`）（gateway.rs:3316-3343）；
- **Proceed**（gateway.rs:3344-3584）：`mark_revision()`；保存 `pre_revision_decision/review` 快照（fallback 只对白名单纯风格 trigger 恢复原稿；安全类 trigger 失败必须 fail closed，注释 gateway.rs:3347-3351）；`pre_revision_summary` 记录。30s `tokio::time::timeout` 包 revision 重生成（Full 档）：
  - **Ok(Ok)**：归一化、`used_knowledge_ids=route_ids`、重新并行 review+ClaimGate（full）、namecard 保留、`apply_independent_claim_gate` + 重算豁免 + **finalize #2** + persist events。`second_passed = (第二轮 finalize==Approved) && review_passed(review, runtime)`：
    - 过 → `revision_applied=true`、`final_review_status="revision_applied_approved"`、finalize_status=Approved、`post_revision_summary`、**再过一次 `apply_state_action_gate`**（revision 整条替换 decision，可能迁入禁 reply 态，gateway.rs:3472-3485）；
    - 不过 → 恢复 `pre_revision_review`，`apply_revision_fallback(...,"revision_post_review_failed")` 得 `(reason, restored)`；restored（纯风格）→ 恢复原稿 + `should_reply=true`；否则 `should_reply=false`（gateway.rs:3486-3509）；
  - **Ok(Err(llm 错误))** / **Err(30s 超时)**：同样恢复首轮 review + `apply_revision_fallback("revision_llm_error:..."/"revision_llm_timeout_30s")`，写 `revision_llm_failure` 事件（restored→info"回退到原稿"；否则 blocked"fail closed"）（gateway.rs:3511-3582）。

**(15) taxonomy 软闸 A3（gateway.rs:3588-3639）**
仅 Approved：`global_taxonomy_cache().find_or_load(workspace)`；`decision_dimension_kinds(active_profile)` + FSM stage keys；`compute_taxonomy_guard_outcome`（纯函数，gateway.rs:7661-7708）：Active 静默 / AliasActive→rewrites+risk / Deprecated→risk / CandidateNew→risk+candidate_writes+quarantines（`customer_stage` 命中 FSM canonical key 时豁免隔离，滚动升级窗口，gateway.rs:7691-7698）。`apply_taxonomy_guard_outcome`（gateway.rs:7713-7734）应用改写与隔离（customer_stage 隔离连带清 `operation_state` 同 raw 值）。逐条 `taxonomy_upsert_candidate`（带中文 display_name，`pick_dimension_display_name` snake→camel 回退取名，gateway.rs:7650-7659），失败仅 warn。放在所有 rewrite/revision 之后只对最终稿执行一次（防 revision 引入未审维度绕过 + 防重复累计 candidate，注释 gateway.rs:3590-3592）。

**(16) `final_review_status` 兜底 + ISSUE-001 context_changed 重算（gateway.rs:3641-3674）**
review.final_review_status 空 → `finalize_status.final_review_status_str()` 兜底。FollowUp（非 deferred wake kind）时用 `inbound_marker_for_context_check(contact) vs task.created_at` 重算 `context_changed_followup_hit`——review 阶段（~3s）用户可能插话，若命中则后续拦截分支的状态被覆盖为 `context_changed`（真实信号不被 finalize_review_blocked 掩盖）。

**(17) !Approved 统一拦截分支（gateway.rs:3676-3819）**
`blocked_status`：context_changed 命中 → `("context_changed", "用户在跟进任务后已有新消息...")` 并追加 risk `follow_up_context_changed`；否则 `(finalize_status.gateway_status_str(), "finalize_review_blocked")`。
顺序：`write_decision_review(status=blocked_status)` → task 路径 `cancel_task` → `blocked_review` 事件 → `write_agent_run_log_with_finalize(blocked_status, FinalizeRunLogFields{...revision 字段...})` → **hold→升级请示** `escalation::escalate_held_decision`（fail-soft warn；context_changed 会被 should_escalate_held 排除，注释 gateway.rs:3758-3761）→ `BlockedUnverifiedProductClaim` 时写 **recall_miss 知识缺口信号**（`GapSignalCandidate::recall_miss_from_product_block(inbound.content)` → `persist_recall_signal`，fail-soft，gateway.rs:3775-3806）→ `ensure_customer_acknowledged` → return Ok。

**(18) 第二道 precheck（gateway.rs:3821-3913）**
`final_precheck = precheck_send_gateway(...)` 重跑一遍；`should_reply && !allowed` 时：task 路径同第一道（quiet_hours_deferred→reschedule，其余 cancel；第二道命中静默仅当边界恰落在决策耗时内，重排丢弃本次决策、醒来重跑，注释 gateway.rs:3824-3827）。写 decision_review(status="gateway_blocked") + `gateway_blocked` 事件 + run log("gateway_blocked") + `ensure_customer_acknowledged` → return。
注意：`should_reply=false` 时**不检查** final_precheck（no_reply 不需要发送资格）。

**(19) 主去抖中止（gateway.rs:3915-3946）**
`should_abort_send()` 为真 → 写 run log `superseded_by_new_inbound` → return。注释（gateway.rs:3922-3924）：必须在推进 `last_agent_run_at` 的写库之前放弃，否则重算 precheck 会 rate_limited 吞掉聚合回复（该注释仍以旧的内联 apply_agent_updates 措辞书写，见 §5 疑点 1）。

**(20) task claim 再复核（gateway.rs:3948-3976）**
持 claim 的 task 路径：`task_claim_is_current` 精确重读 lease；失效 → run log `stale_task_claim` → return（100ms 监视器只是提前中止优化，非所有权权威，注释 gateway.rs:3948-3951）。

**(21) 落 decision_review + claim 绑定 + 投影快照（gateway.rs:3978-4098）**
`ascending_window = recent_messages.rev()`（升序窗口构造一次，与 prompt 顺序一致）；`should_reply=false` → `mark_no_reply()`。
`write_decision_review(status = should_reply?"outbox_enqueuing":"no_reply")` 得 `decision_review_id`。
**SR-034**：task 路径在创建任何 Outbox 之前 `bind_task_decision_if_owned(claim, decision_review_id)` CAS（无 claim 的旧入口按 task_id 兼容关联 `outbox_decision_id`）；失败 → review 置 `stale_task_claim` + `task_claim_fenced` 事件 + `post_decision::discard_projection` + return（gateway.rs:4012-4055）。
`post_decision::persist_projection_snapshot(decision_review_id, final_decision, memory, context_pack, domain_config, active_profile, active_products, ascending_window, contact, run_id)`——失败仅 warn + `mark_preparation_failed`，客户投递继续（gateway.rs:4056-4083）。
`write_knowledge_usage_log`（fail-soft，遥测非授权事实，gateway.rs:4084-4098）。

**(22) 无文本可发的终态（gateway.rs:4099-4121）**
`!text_send_eligible(should_reply, reply_text)`：task → `cancel_task("no_reply", reason)`（区分"判断无需触达"vs"想回但正文空"）；投影：should_reply=true 空文本 → `discard_projection("empty_reply_text")`；should_reply=false → `activate_projection`（no_reply 也激活画像投影）。**不 return**，继续走（媒体门后续依赖 outbox_eligible=false 自然不发）。

**(23) prepared 事件 + 预入队 run log（gateway.rs:4122-4173）**
`agent_reply_prepared` 事件（fail-soft）+ `write_agent_run_log_with_finalize(status = should_reply?"outbox_enqueuing":"no_reply")`（fail-soft，warn"pre-enqueue run telemetry failed"）。

**(24) outbox_eligible 计算 + relay 泄漏守卫（gateway.rs:4175-4216）**
`outbox_eligible = should_reply && reply_text 非空 && final_status ∈ {approved, revision_applied_approved}`。
relay run（`is_principal_relay_trigger`）且 eligible 时：`relay_output_leaks_internal_payload(reply_text)`（检 `__PRINCIPAL_RELAY__`/verdict=/substance=/constraints= 标记）命中 → `outbox_eligible=false` + `delivery_block_status=Some("blocked_by_safety_guard")` + `blocked_review` 事件（fail-closed：宁可这轮收不到也不发内部载荷；数字白名单 backstop 因威胁模型错误已删除 KD-01/03，注释 gateway.rs:4184-4192）。

**(25) 兜底去抖（gateway.rs:4217-4283）**
`media_pending = final_status 终态 && (assets_to_send 非空 || namecard_to_send 存在)`；`should_run_send(outbox_eligible, media_pending)` 时再查一次 `should_abort_send()`：命中 → decision_review 置 `superseded_by_new_inbound`（CAS on status="outbox_enqueuing"）+ run log 置同状态（lifecycle=ABORTED_BY_EXTERNAL_SIGNAL, abort_reason）+ cancel_task + discard_projection + return。**B-01 已知取舍**（注释 gateway.rs:4228-4234）：此 guard 到多段 enqueue 循环间仍有极窄尾窗（每段一次 DB 往返 10-100ms），新入站落在窗内会两批 segment 都发出（幂等 key 不同不互相去重）→ 客户可能收两次；彻底消除需按 run/generation 撤销 pending outbox 的补偿，风险最高，列专项不修。

**(26) 文本分段 enqueue 循环（gateway.rs:4284-4529）**
`source_event_id`：Inbound→message_id / FollowUp→task_id.hex（unwrap_or_default，可空）。`split_reply_into_segments(reply_text, agent_reply_max_segment_chars, agent_reply_max_segments)`；多段时每段 key = `{segment_idempotency_base(source_event_id, run_id)}#seg{idx}`（同内容段防幂等碰撞吞消息 #68；空 source 回落 run_id 防跨 run 撞键，gateway.rs:2337-2347/4306-4313）。每段 `EnqueueRequest{decision_id=Some(review_id), max_attempts:3}`：
- Created → `text_outbox_enqueued=true`；
- IdempotentSkip → 先试 `adopt_recoverable_durable_outbox_if_owned`（task claim 场景收养旧 decision 的 outbox），adopted 或 `existing_outbox_covers_decision`（同 decision 且状态可交付）→ 计为已入队；否则记入 `cross_decision_duplicates`；
- Err → 记入 `enqueue_errors`，**继续下一段**（不中断，最大化发完能发的段，注释 gateway.rs:4300-4304）。
循环后分派：
- **全段都是跨 decision 重复**（无错误、`cross_decision_duplicates.len()==total`、无任何入队）→ decision_review 置 `skipped_duplicate`（记 duplicate_outbox_ids）+ run log `skipped_duplicate`（COMPLETED）+ cancel_task + `outbox_skipped_duplicate` 事件 + discard_projection + return（gateway.rs:4390-4455）；
- **部分失败/部分重复** → `outbox_enqueue_partial_failure` 事件（error 级，"已入队段照常发出，失败段缺失"）+ decision_review/run log 置 `outbox_enqueue_partial_failure`（lifecycle=FAILED_AFTER_DECISION；outbox_status=pending(有入队)/canceled）+ cancel_task + discard_projection + `refresh_run_log_outbox_status`；有真错误则返回第一个 Err，只有跨 decision 重复则 return Ok（gateway.rs:4456-4528）。

**(27) 未入队收尾（gateway.rs:4530-4574）**
task 路径 text_send_eligible 但未入队 → `cancel_task("blocked_by_safety_guard","发送安全门拦截，未创建 outbox")`。`should_reply && !text_outbox_enqueued` → decision_review/run log 置 `delivery_block_status.unwrap_or("blocked_by_safety_guard")`（FAILED_AFTER_DECISION, outbox_status=canceled）+ discard_projection。

**(28) 授权 CAS + 完成态（gateway.rs:4577-4724）**
`text_outbox_enqueued` 时：
- task 有 claim → `authorize_task_outbox_if_owned` CAS（全部文本段 enqueue 后立即提交授权；素材/名片不阻塞文本；dispatcher 之前抢到首段只会识别为 Building 无损 defer，注释 gateway.rs:4575-4576）；无 claim 旧路径 → 直接置 task `status/gateway_status="outbox_enqueued"`；非 task → 恒 authorized；
- CAS 失败 → decision_review 置 `stale_task_claim` + run log（ABORTED_BY_EXTERNAL_SIGNAL）+ `task_claim_fenced` 事件（"Dispatcher 将取消旧 owner 条目"）+ discard_projection + refresh + return；
- 成功 → decision_review 置 `outbox_enqueued` + run log 置 `outbox_enqueued`（LIFECYCLE_COMPLETED）→ **contact `$max last_agent_run_at`**（`authorized_at`；rate-limit 锚在文本批次持久授权后立刻推进，独立于重量级画像投影，防投影期间新入站再起重复回复；matched!=1 或 Err 仅 error log，gateway.rs:4669-4708）→ `post_decision::activate_projection`（用 `?`）→ `outbox_enqueued` 事件（fail-soft）。

**(29) 素材发送段（gateway.rs:4725-4900）**
`media_send_allowed(outbox_eligible, !assets_to_send.is_empty())` 时逐个 directive：
- `ObjectId::parse_str` 失败 → `media_asset_id_invalid` 事件 + continue；
- `content_assets` 按 `{_id, workspace_id}` 双条件查（防跨租户 IDOR）；DB 错误 → `media_asset_lookup_failed` 事件 + continue（不 `?`——否则跳过函数末尾 escalation 推送，注释 gateway.rs:4758-4761）；
- 不存在/`validate_asset_sendable` 不过 → `media_asset_rejected`（防幻觉）+ continue；
- `requires_principal_approval == Some(true)` → 构造 `EscalationRequest{category=OUT_OF_SCOPE,...}` 走 `trigger_principal_escalation`（失败 warn）+ `media_asset_escalated` 事件 + continue（**不入 outbox**）；
- enqueue：`content=String::new()`（媒体条目空文本）+ `media_asset_id`；`media_source_event_id` 不加后缀；Created/Skip/Err（Err → `media_outbox_enqueue_failed` 事件，文本已照常发出）。

**(30) 名片段（gateway.rs:4901-5001）**
`media_send_allowed(outbox_eligible, namecard_to_send.is_some())` 且 assist_on（重算一遍 override）时：`referral_cards` 按 `{_id, workspace_id}` 查 + `validate_card_sendable(card, account_id)`；过 → enqueue（key=`{source_event_id}#namecard`，`referral_card_id`）；不过 → `referral_card_rejected` 事件。名片不走素材的 principal_approval 分支（D9：台前顾问 ≠ 幕后决策源）。追加在素材/文本后=先铺垫话术后名片（D5）。

**(31) 收尾（gateway.rs:5002-5021）**
`refresh_run_log_outbox_status` → `final_decision.escalation_request.needed` 时 `trigger_principal_escalation`（失败仅 warn，占位已正常发出）→ Ok。

### 2.5 管理发送 `send_contact_message_gateway_inner`（gateway.rs:720-1295）

`mark_manual()`；构造 `synthetic_inbound`（content=固定管理控制句"后台管理 Agent 请求发送私聊…"，message_id=None，raw=request.source）；planner 硬编码 `{risk:high, context_needs_refresh:true, memory_change_importance:6, knowledge_required:true, review_mode:full}`（gateway.rs:747-756）。
流程：
1. **precheck #1**：拦 → `send_gateway_blocked` 事件 + run log(precheck.status) + `Err(BadRequest(reason))`（管理 API 同步返回错误，gateway.rs:757-788）；
2. `load_manual_run_inputs` 并行 5 项；context_pack 同主链；**knowledge_inbound**：clone synthetic 后把 content 换成拟发正文（知识相关性按拟发文案算，不按控制句，gateway.rs:801-805）；`route_operation_knowledge_for_existing_candidate` ∥ products 并行（gateway.rs:806-824）；
3. 构造 decision：`should_reply=true, reply_text=content, used_knowledge_ids=route ids, next_best_action={source:"management_agent_send", originalContentLocked}`（gateway.rs:828-838）；`mark_run_envelope_running`；
4. **review + ClaimGate 并行**（review_mode="full"）→ `apply_independent_claim_gate` → **finalize**（promote_risks 恒空——decision 非 LLM raw 输出；`principal_product_exempted` 照算）→ `persist_finalize_pending_events` → Approved 时 `apply_state_action_gate` → **`normalize_manual_send_review_terminal`**：Approved 但 `review_passed=false`（软闸分数不达标）→ 强制翻 `Held(held_by_ai_policy)`（管理发送无 revision 循环，暴露 approved 会让调用方空等 outbox，gateway.rs:675-698/902-914）；
5. **不过** → 写 decision_review(status="blocked") + `blocked_review` 事件 + run log(blocked_status) + 返回 `ContactSendResult{review_approved:false, gateway_status, gateway_reason=manual_send_block_reason(...)}`（gateway.rs:916-988）；
6. **precheck #2** 拦 → decision_review + run log("gateway_blocked") + 返回 `review_approved:true` 但 status=final_precheck.status（gateway.rs:990-1042）；
7. **enqueue**（S5.2：原直调 send_outbound_message 绕过 outbox，已改 enqueue+dispatcher 异步）：先写 decision_review(status="outbox_enqueuing") + run log 同状态；`pause_reply_obligation_for_manual`（冻结当前 inbound 水位 + fence 旧 AI owner；返回 coverage 时把 `reply_coverage_kind:"manual_reply"` 等写回该 review，写失败则先 `settle_manual_reply_obligation(false)` 释放再 Err，gateway.rs:1101-1136）；`EnqueueRequest{decision_id=review_id, source_event_id="", source_kind=manual_send, max_attempts:3}`（synthetic 兜底幂等键=run_id+content_hash，重复点发送不重复发，gateway.rs:1144-1147）；
   - Created → "outbox_enqueued"；
   - IdempotentSkip → 释放 pause（既有 outbox 冻结快照不覆盖后来消息）；`existing_outbox_covers_decision` 判 "outbox_enqueued" 或 "skipped_duplicate"；
   - Err → 释放 pause（先释放：后续审计可能失败但绝不能晾死 contact）→ decision_review 置 `outbox_enqueue_failed` + run log 同（FAILED_AFTER_DECISION + error_summary）→ Err；
8. decision_review/run log 置 enqueue_outcome（LIFECYCLE_COMPLETED）；`refresh_run_log_outbox_status`；`management_send` 事件；返回 `ContactSendResult{message_id:None, gateway_status=enqueue_outcome}`。

### 2.6 发送与回执

**`classify_send_receipt`（gateway.rs:5035-5050）**
`ok:true` → Succeeded；`ok:false` 且无非空 newMsgId → ExplicitlyFailed（可安全重试）；`ok:false` 但有 newMsgId / `ok` 非 bool / 无 `ok` 无 id → Inconclusive（必须停止自动重放）；无 `ok` 但有非空 newMsgId → Succeeded（旧信封）。

**`send_outbound_message`（gateway.rs:5056-5205，pub(crate)）**
仅 outbox_dispatcher（与过渡期遗留内联路径）可调（注释 gateway.rs:5052-5055）。`mcp::logged_send_call_for_account("message_send_text", {recipient, content})` → 回执分类：ExplicitlyFailed → `OutboundSendError::SafeToRetry`；Inconclusive → `DeliveryUncertain`。成功后：
- ④ 账号日发软上限观测：`account_daily_sent_count >= account_daily_send_soft_cap` → `agent.account_daily_send_soft_cap_exceeded` warning 事件（仅告警不拦，fail-soft，gateway.rs:5096-5126）；
- **既成事实红线**（注释 gateway.rs:5127-5130）：MCP 成功后任何 DB 失败不得返 Err（否则 dispatcher 重试造成客户重复收信）。落 `conversation_messages`（direction=Outbound, raw 带 wechatagent 附加）失败 → error log + `outbound_record_persist_failed` 事件；
- contact 时间戳 aggregation pipeline：`last_outbound_at/last_agent_run_at/updated_at=now`，`last_message_at=$max(last_inbound_at, now)`（不动 last_inbound_at），并写 `last_outbound_style` 风格指纹（D2）——失败仅 error log（gateway.rs:5173-5203）。

### 2.7 决策后写库族（由 post_decision 投影 worker 消费）

**`apply_agent_updates`（gateway.rs:6166-6897，pub(crate)）**
输入含 `projection_guard: Option<ProjectionWriteGuard>`；返回 `AgentUpdateOutcome`。stage_timer="profile_updates"。构建 `set_doc` 起始 `{updated_at, last_agent_run_at}`（gateway.rs:6179-6182）。步骤：
1. `profile_update` → `agent_profile` 整体覆盖（gateway.rs:6184-6186）；
2. clone decision → `normalize_domain_signals`（typed 维度镜像进 domain_signals 容器，H1/1D）；
3. **G1 客观纠偏**（gateway.rs:6205-6233）：`transaction_facts_enabled` 且 profile 声明 purchase_lifecycle 参与决策 → `project_entitlements(outcome_events, products, now, CAP)` → `reconcile_g1_with_entitlements(llm_g1, entitlements)` 命中 → 容器覆盖 + 记 `g1_correction`；
4. **写侧白名单**：`retain_declared_dimensions`（剔除 LLM 臆造未声明键）（gateway.rs:6234-6242）；
5. **写侧 value 校验**（gateway.rs:6249-6304）：每个 ValueSource::Taxonomy 维度过 `validate_dimension_value(MachineWrite)`；Accept→归一替换；Drop/Reject→移除 + `agent.dimension_dropped` 审计（fail-soft）；
6. **customer_stage 状态机 + 强弱证据门**（gateway.rs:6305-6428）：
   - `check_state_transition(domain_config, prev_stage, to_stage)` 拒绝 → `agent.stage_transition_rejected` 审计（fail-soft）；**容器不移除 customer_stage**（下游 C2 派生读同容器以拒迁移，仅对 domain_attributes 写入用过滤副本，注释 gateway.rs:6318-6322）；
   - 弱证据（`evidence_strength(resolve_evidence(window, stage_evidence_turns), window, stage_explicit_intent)` 非 Strong）→ 不实时写 stage，未被状态机拒绝时落 `write_stage_observation` 暂定层（fail-soft）；
   - `signals_for_attrs`：拒绝/弱证据时 Cow::Owned 剔除 customer_stage，否则 Cow::Borrowed 零拷贝；
   - C-01 stagnation：按 `active_profile.stagnation_dimension`（默认 customer_stage）判该维度变化 → `insert_domain_signal_values(set_doc, signals, changed, dim)`，写过则 `domain_attributes_updated_at`；
7. **G6 value_tier**（gateway.rs:6429-6444）：transaction_facts_enabled 时客观计算 `compute_customer_value_cents` → `classify_value_tier(mid/high 阈值)` 直写 `domain_attributes.value_tier`（不经容器，避免被白名单剔除）；
8. commitments **不在此写**（须 dispatcher 确认送达后提交，gateway.rs:6448-6449）；⑥观测：reply 有时间承诺特征但 `last_commitment` 空 → `agent.commitment_field_missing` 事件（gateway.rs:6450-6466）；
9. `follow_up_policy` 非空写入；**C2 operation_state 派生**（gateway.rs:6470-6511）：`synced_state = 归一后 customer_stage || decision.operation_state`；`check_state_transition` 通过 → 写 `operation_state(+updated_at)` 并记 `applied_operation_state`；拒绝 → 记 `rejected_state_transition`（fail-soft，不阻断，保旧值）；`operation_state_reason/confidence`、`cooldown_until`（RFC3339）、`profile_attributes` 非空写；任一画像字段更新 → `profile_updated_at`；
10. `memory_update` 非空 → `merge_memory_summary_dedup_capped(existing, update, 12, 1200)` 写 `memory_summary`（gateway.rs:6537-6549）；
11. **fenced contact 写**（gateway.rs:6551-6609）：guard 存在时 filter 加 `profile_revision==baseline`（0 时兼容 exists:false/null）AND `last_projection_review_id` 缺失/null/`$lt guard.review_id`，并 set `last_projection_review_id/last_projection_run_id`。matched!=1：无 guard → FencedConflict；有 guard → 重读 contact 判"同 revision 且同 review_id"→ AlreadyApplied（幂等重放），否则 FencedConflict；
12. **贝叶斯旁路**（仅 Applied 且有观察，gateway.rs:6611-6665）：`build_observed_dimensions`（强证据按 Inbound 方向代码算，截断 MAX_BAYESIAN_SLOTS）→ `apply_bayesian_update` → 整体 `$set bayesian_signals`（**无 OCC，last-write-wins 备案 D3-F1**，纯观测永不驱动，fail-soft）；
13. relationship_type 建议（gateway.rs:6667-6711）：提取信号 → `validate_dimension_value(MachineWrite)` → `upsert_pending_projection_observation("relationship_type_suggestions",...)`（fail-soft）；
14. suspected_deal 弱信号（gateway.rs:6713-6743）：无字典校验，upsert 到 `suspected_deal_signals` 待核实专表（**红线：AI 永不直写 outcome_events**）；
15. G1 纠偏审计 `agent.purchase_lifecycle_corrected_by_objective`（gateway.rs:6745-6773）；
16. **churn 观测**（gateway.rs:6775-6844）：`compute_profile_churn(confirmed_tags值投影, decision.tags, stage/intent 新旧, memory_summary, memory_update)`，notable 时写 `agent.profile_churn_observed`；
17. state 迁移事件互斥（gateway.rs:6846-6893）：rejected → `agent.operation_state_transition_rejected`；否则 `detect_state_transition(contact.operation_state, applied_operation_state)` → `agent.operation_state_transitioned`（按**实际写入值**判，不按 decision 提案）。
follow_up 任务创建也不在此（dispatcher 确认送达后，gateway.rs:6895）。

**`apply_operating_memory_update`（gateway.rs:6899-7005，pub(crate)）**
`write_memory_candidates`（`?` 传播）→ `write_tag_observations`（fail-soft warn；window 为升序，LLM evidence_turns 是升序 0-based 下标）→ `contact_memory_consolidation_due` → 是则 `schedule_memory_consolidation_task`（均 fail-soft）→ `operating_memory_update` 与 context_pack 都空则 return → **CONC-1 memory_card OCC**：`memory_card_has_signal(effective_memory_card(memory))` 为假时，用 `occ_memory_filter(ws,acc,wxid,prev_version)` CAS 写 `{memory_card, memory_card_version=next, memory_card_updated_at, updated_at}`；`modified_count!=1` = 输给并发 writer，静默 debug 跳过——既成事实纪律（gateway.rs:6945-6988）→ 门控外 `updated_at` 走原三键 filter（不 bump 版本，不能套版本谓词否则永久 lost-race，gateway.rs:6989-7004）。

### 2.8 审计写入族

**`write_decision_review`（gateway.rs:7036-7150）**
`prompts::prompt_versions` 取 8 个模板快照（user.reply.system/policy/fast.task、knowledge.router、review.system/light.system、memory_consolidator.system/task），再用 run-local budget 记录覆盖（Shadow override 审计真实模板，gateway.rs:7069-7075）。落 `AgentDecisionReview`：`approved=review_passed(review,runtime)`（软闸口径）、scores/formula_breakdown/risks/rewrite_instruction/review_summary、playbook id+version、used_knowledge_ids（parse 失败的 id 静默丢弃）、prompt_versions、operation_state、next_best_action、context_pack_snapshot（嵌入 knowledgeRoute + runPlanner）、domain_config/runtime 快照、send_gateway_result、outcome_status="pending"、reaction 空占位、`expected_text_segments`（仅 status=="outbox_enqueuing" 时按分段函数计算，否则 0）、status、created_at。返回 inserted ObjectId。

**`write_agent_run_log_with_finalize`（gateway.rs:7218-7298）**
先三重闭集校验：`assert_final_review_status_valid` / `assert_gateway_status_valid` / `derive_lifecycle_from_status(status, error)` + `assert_lifecycle_valid`（脏值 fail-closed 不写库）。从 task-local budget snapshot 取 token_budget/tokens_used/llm_calls_used/unknown_usage_calls/degraded_reasons。`update_run_envelope_terminal(AgentRunLogTerminalFields{...全部终态字段...})`；`abort_reason` 仅 lifecycle==ABORTED_BY_EXTERNAL_SIGNAL 时=status。

**`write_event_for_account(_with_dedupe)`（gateway.rs:7533-7590）**
构造 AgentEvent → 优先 `try_buffer_observability_event`（RUN_AUDIT_BUFFER 内批量，settle 时 flush）；buffer 不可用（返回 Err(event)）→ 直插 `events`；duplicate key 错误视为成功（dedupe_key 幂等）。

**`persist_finalize_pending_events`（gateway.rs:7307-7326）**：finalize 纯函数返回的事件逐条持久化（保持 finalize 无 db 依赖可单测）。

**`settle_gateway_execution`（gateway.rs:1812-1915）**
`RUN_AUDIT_BUFFER.scope(audit, AssertUnwindSafe(execution).catch_unwind())`；join parallel_reaction（record_stage "reaction_analysis"，错误 warn）；`Ok(Err(e))` → `fail_run_envelope_if_open("gateway_error: {e}")`；panic → 同（"unhandled_panic: ..."），payload 保留至审计完成后 `resume_unwind`。并行 flush：`flush_llm_logs`（insert_many 快路径 + 稳定 id upsert 兜底）+ `flush_observability_events`；record_stage 三项 flush 计时；`performance_document` 写入 `agent_run_logs.gateway_result.performance`（按 run_id，无 status 谓词）；flush 失败仅 error log（审计中断不得把已授权发送变成网关错误引发重复投递，注释 gateway.rs:1856-1858）。

### 2.9 请示通道 `trigger_principal_escalation`（gateway.rs:1930-2024）

`!req.needed` → Ok。加载 domain_config（无 = 未配置 → Ok）；`resolve_ask_human_policy` + `freeze_ask_human_policy(account_id)`；`decider_chain.first()` 空 = 未启用 → Ok；decider 缺 account_id → `BadRequest("决策人缺少发送账号")`；`principal_wxid == contact.wxid` → `BadRequest`（拒绝自我请示）。**骚扰门**：`count_pushes_today`（since = now-24h）+ `latest_push_ms` → `push_allowed(policy, today, last_push, now)` 不过 → Ok（静默跳过）。category 缺省 `ESCALATION_CATEGORY_OUT_OF_SCOPE`；`stuck_suppressed(category, policy)` → Ok。`has_pending_for_contact(ws,acc,wxid,category)` → Ok（去重）。`customer_label = remark||nickname||alias||wxid`。`insert_pending_escalation(...)` 返回 None（并发唯一索引兜底）→ Ok；Some(entry) → `materialize_principal_card_delivery`。调用方对本函数错误只 warn 不阻断（注释 gateway.rs:1929-1930）。

### 2.10 relay `relay_principal_decision_to_customer`（gateway.rs:2027-2146）

`verdict_authorizes = verdict ∈ {APPROVED, CONDITIONAL}`。
- **A 类豁免先写**（gateway.rs:2044-2098）：authorizes 且 `exemption_type ∈ {CUSTOMER_ONLY, KNOWLEDGE}` → `$set domain_attributes.{PRINCIPAL_PRODUCT_EXEMPTION_ATTR} = {granted, granted_by, substance, escalation_short_code, granted_at_ms}`（点号子键不整体覆盖；**用 `?`**——授权写失败则本轮转述不得视为完成）→ **同步内存副本**（gateway_inner 不重载 contact，R5.4 产品门读内存值；不同步则当轮照拦，注释 gateway.rs:2076-2079）→ `contact.principal_exemption_granted` 事件（fail-soft）；
- 合成 `ConversationMessage::synthetic_principal_relay(contact, verdict, substance, constraints)` → `run_user_operation_gateway(contact.clone(), Inbound(&synthetic), task_context, None)`；
- **awaiting 只能由 dispatcher 确认真实送达后清除**（gateway 返回 Ok ≠ 客户已收到，注释 gateway.rs:2115-2116）；
- **B 类知识沉淀**（gateway.rs:2119-2144）：authorizes 且未 emit 过 → `exemption_type==KNOWLEDGE || entry.is_generalizable` 时 `emit_knowledge_gap_proposal`（只进 draft+needs_review，**绝不直接 verified**）→ CAS 置 `knowledge_proposal_emitted=true`。

### 2.11 其余支撑纯函数

（判定逻辑已在 §1 表格与上文引用，此处仅列关键语义）
- `split_reply_into_segments`（gateway.rs:5465-5510）：双换行→单换行切段；超 `max_segment_chars`（unicode char 计）按句末标点（。！？!?；;.）就近切、无标点 2×max 硬切（gateway.rs:5514-5542）；段数超 `max_segments` 尾部 `join("\n")` 合并；空文本→空 Vec。
- `merge_memory_summary_dedup_capped`（gateway.rs:5963-5986）：existing+update 逐行 trim 去重（保序），超 12 行/1200 字节从最旧整行丢（至少保 1 行）。
- `compute_profile_churn`（gateway.rs:5877-5932）：new_tags 空=未更新不计；notable = 丢标签 || stage/intent 翻转（old 非空才算翻转）|| summary>2000。
- `load_recent_messages`（gateway.rs:7366-7453）：`{created_at:-1,_id:-1}` limit；再查 durable inbound task 的 `covered_through_*`（不含边界）或 `obligation_started_*`（含边界）水位之后的全部 inbound 合并去重重排——长静默窗积压消息超窗时保证不漏旧问题（注释 gateway.rs:7390-7392）。
- `upsert_pending_projection_observation`（gateway.rs:6082-6163）：`{workspace_id, contact_id, status:"pending"}` findOneAndUpdate upsert（dup-key 竞态回读）；`projection_observations::record_and_count` 严格重放台账 + `reconcile_stages` 回写 occurrences。

---

## 3. 跨机制数据流

### 3.1 一次成功 Inbound run 的完整写集合顺序（happy path）

| # | 集合/目标 | 写入点 | 内容 |
|---|---|---|---|
| 1 | `agent_run_logs` | gateway.rs:1757（`write_run_envelope_started`） | envelope started（source_event_id/kind, lifecycle=started） |
| 2 | `agent_run_logs` | gateway.rs:2608（`mark_run_envelope_running`） | lifecycle=running + 首程 decision 快照 |
| 3 | `agent_events`（多为 buffer 暂存） | ptier_*/style/degraded 等观测点 | 观测事件（settle 时批量 flush） |
| 4 | `decision_reviews` | gateway.rs:3072（仅 rewrite 路径） | status=`rewrite_requested` 中间记录 |
| 5 | `agent_events` | gateway.rs:3212/3453（`persist_finalize_pending_events`） | finalize 硬门事件 |
| 6 | `taxonomy_candidates` | gateway.rs:3624 | CandidateNew upsert（fail-soft） |
| 7 | `decision_reviews` | gateway.rs:3988 | 正式记录 status=`outbox_enqueuing`/`no_reply` |
| 8 | `agent_tasks` | gateway.rs:4013（`bind_task_decision_if_owned`，task 路径） | claim CAS 绑定 decision_id |
| 9 | post_decision 投影快照 | gateway.rs:4056 | 投影物料（fail-soft） |
| 10 | knowledge usage log | gateway.rs:4086 | 遥测（fail-soft） |
| 11 | `agent_events` | gateway.rs:4125 | `agent_reply_prepared`（fail-soft） |
| 12 | `agent_run_logs` | gateway.rs:4143（terminal 字段） | status=`outbox_enqueuing`/`no_reply`（fail-soft） |
| 13 | `agent_send_outbox` | gateway.rs:4327（循环） | 文本 N 段（幂等键 `#seg{idx}`） |
| 14 | `agent_tasks` | gateway.rs:4580（`authorize_task_outbox_if_owned`） | 授权 CAS |
| 15 | `decision_reviews` | gateway.rs:4650 | → `outbox_enqueued` |
| 16 | `agent_run_logs` | gateway.rs:4659 | → `outbox_enqueued` + lifecycle=completed |
| 17 | `contacts` | gateway.rs:4676 | `$max last_agent_run_at`（频控锚提前推进） |
| 18 | post_decision | gateway.rs:4709（`activate_projection`） | 投影激活（`?` 传播） |
| 19 | `agent_events` | gateway.rs:4710 | `outbox_enqueued`（fail-soft） |
| 20 | `agent_send_outbox` | gateway.rs:4863/4958 | 素材条目 / 名片条目（fail-soft） |
| 21 | `agent_run_logs` | gateway.rs:5002（refresh_run_log_outbox_status） | outbox 汇总状态 |
| 22 | `agent_principal_escalations` + 推卡 | gateway.rs:5008 | 若 escalation_request（fail-soft） |
| 23 | `llm_call_logs` + `agent_events` + `agent_run_logs.gateway_result.performance` | gateway.rs:1860-1889（settle） | 审计批量 flush + 性能 |

**画像/记忆更新（`apply_agent_updates` / `apply_operating_memory_update`）不在 inner 主路径内联执行**：主路径只做 `persist_projection_snapshot`（#9）→ `activate_projection`（#18），实际的 contacts 画像写、operating_memories OCC 写、观察表 upsert 由 post_decision 投影 worker 异步回放（fencing 契约见 3.3）。

### 3.2 事务边界

全文件**无 MongoDB 多文档事务**。一致性策略：
- 单文档原子更新（`$set`/`$max`/`$push+$slice`/aggregation pipeline）；
- 状态 CAS：decision_review/run log 的状态推进多带旧状态谓词（如 `{run_id, status:"outbox_enqueuing"}`，gateway.rs:4246/4403/4549/4609）；
- 幂等键：outbox enqueue（source_event_id/synthetic run_id+content_hash 派生，构造在 `outbox::enqueue`，本文件未定义）；事件 dedupe_key（dup-key 视为成功，gateway.rs:7585-7588）；
- 顺序保证靠代码顺序 + fail-soft 分级（见 3.4）。

### 3.3 claim / fencing 全景

1. **task claim（发送授权 fence）**：durable inbound 的 100ms 监视器（提前中止优化，gateway.rs:273-293）→ 主路径 `task_claim_is_current` 精确重读（gateway.rs:3956）→ `bind_task_decision_if_owned`（Outbox 前 CAS，gateway.rs:4013）→ `authorize_task_outbox_if_owned`（enqueue 后 CAS，gateway.rs:4580）→ dispatcher 侧同 token CAS 二次门（注释 gateway.rs:4576）。任何一环失败 → `stale_task_claim` 终态，绝不进/不放行 Outbox。
2. **投影 fencing**（`ProjectionWriteGuard`，gateway.rs:6039-6043/6551-6609）：contact 写 filter 带 `profile_revision==baseline`（权威写者 bump）+ `last_projection_review_id < review_id`（投影单调），三态 Applied/AlreadyApplied（同 revision 同 review 幂等重放）/FencedConflict（仅保留 append-only 证据）。
3. **memory OCC**（CONC-1，gateway.rs:6964-6987）：`occ_memory_filter(prev_version)` CAS，输者静默跳过。
4. **commitments**（CONC-2，gateway.rs:125-143）：`$push+$slice:-8` 原子追加，应用层快照去重（并发重复可接受）。
5. **贝叶斯信号**：显式无 OCC、last-write-wins（备案 D3-F1，gateway.rs:6623-6628）。
6. **去抖**（should_abort_send）：三个检查点——reaction 屏障后主检查（apply/写库前，gateway.rs:3925）、入队前兜底（gateway.rs:4235）、ack 占位前（gateway.rs:2287）；残余 B-01 尾窗见 §5。

### 3.4 失败降级矩阵（哪一步失败会怎样）

| 阶段 | 失败行为 |
|---|---|
| precheck / 快照加载 / 首程决策 / review（LLM 错） | `?` 上抛 → settle 关 envelope（gateway_error），webhook/worker 看到 Err（task 由 reclaim 重试） |
| budget 超额 | 分级降级：知识路由跳过（空知识）→ review 降 local → rewrite 跳过 → revision Skip（revision_failed 终态）；全程记 `run_budget_exceeded` 事件 + degraded_reasons |
| reaction 分析失败 | warn，继续（不视为 stop）（gateway.rs:1672-1674） |
| revision LLM 错/超时 | `apply_revision_fallback`：纯风格 trigger → 恢复原稿继续发；安全类 → fail closed 不发 |
| taxonomy candidate upsert | warn 继续 |
| 投影快照准备失败 | warn + mark_preparation_failed，客户投递继续 |
| 文本某段 enqueue 失败 | 继续其余段；结束后整体 `outbox_enqueue_partial_failure`（已入队段照发，失败段缺失，返回首个 Err） |
| 素材/名片 enqueue/查询失败 | 事件 + continue（绝不 `?`，防跳过尾部 escalation） |
| `activate_projection` 失败 | `?` 上抛（罕见的授权后硬失败点） |
| MCP 发送成功后落库失败 | **绝不返 Err**：审计事件 + error log（防 dispatcher 重试重复发信） |
| 审计事件/flush 失败 | warn/error log，不影响主流程与已授权发送 |
| panic | catch_unwind → envelope fail("unhandled_panic") → 审计 flush 完成后 resume_unwind |

---

## 4. 事实卡速查

### 4.1 gateway_status（`agent_run_logs.status` / decision_review.status / task.gateway_status 出现过的全部字面量）与产生条件

**precheck 族**（`blocked()` 构造，gateway.rs:5245-5366）：
| 值 | 条件 |
|---|---|
| `not_managed` | agent_status != Managed（gateway.rs:5252） |
| `cooldown` | contact.cooldown_until > now（gateway.rs:5271） |
| `policy_cooldown` | operation_policy.cooldownUntil > now（gateway.rs:5380） |
| `policy_wait_user_reply` | requireUserReplyBeforeNextOutbound && consecutive>0（gateway.rs:5396-5405） |
| `policy_consecutive_limit` | consecutive >= maxConsecutiveAgentOutbounds>=0（gateway.rs:5406-5418） |
| `rate_limited` | now-last_agent_run_at < min_reply_interval_seconds（gateway.rs:5283-5288） |
| `daily_limit` | FollowUp 且 24h distinct run 数 >= max_daily_touches（gateway.rs:5292-5296） |
| `expired` | FollowUp 且 task.expires_at < now（gateway.rs:5302-5308） |
| `quiet_hours_deferred` | FollowUp 非 wake，静默时段（gateway.rs:5319-5340）→ task 被 **reschedule** 而非 cancel |
| `context_changed` | FollowUp 非 wake，last_inbound > task.created_at（gateway.rs:5342-5357）；另可由 ISSUE-001 在 finalize 拦截分支覆盖产生（gateway.rs:3679-3692） |
| `allowed` | 全过（非终态） |

**finalize 族**（`GatewayStatusFinal::gateway_status_str`，核证于 review/gates.rs:479-501）：
| 值 | 条件 |
|---|---|
| `approved` | 全部硬门通过 |
| `blocked_by_required_field` | 必填协议字段缺失（finalize 内，R3.5/3.6） |
| `blocked_by_budget` | 预算硬门（R3.7） |
| `blocked_unverified_product_claim` | 产品宣称无 verified 知识/目录/豁免背书（R5.4）；同时触发 recall_miss 缺口信号（gateway.rs:3785-3806） |
| `blocked_by_safety_guard` | 安全门；也是 relay 泄漏守卫（gateway.rs:4198）与"应发未入队"兜底（gateway.rs:4544）的状态 |
| `held_by_ai_policy`（Held 类别之一） | should_hold / 状态动作闸（gateway.rs:2230）/ 管理发送软闸失败翻转（gateway.rs:687）；Held(category) 的 category 取值域 = HOLD_CATEGORY_VALUES（types.rs:1512-1515：held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context） |
| `ai_waiting_for_more_context` | Held 类别之一（上下文不足暂缓） |

**主链终态族**：
| 值 | 条件 |
|---|---|
| `user_reaction_stop_requested` | reaction 汇合检测 stop（gateway.rs:1683-1725） |
| `superseded_by_new_inbound` | 去抖主检查/兜底检查命中（gateway.rs:3932/4247-4264） |
| `stale_task_claim` | claim 复核/bind CAS/authorize CAS 失败（gateway.rs:3962/4037/4610）；非文本 task 路径同（gateway.rs:1443/1528） |
| `no_reply` | should_reply=false（run log gateway.rs:4148；task cancel gateway.rs:4109） |
| `outbox_enqueuing` | 入队前中间态（decision_review/run log） |
| `outbox_enqueued` | 文本批次入队 + 授权 CAS 全部成功（gateway.rs:4648-4668） |
| `skipped_duplicate` | 全部段命中跨 decision 既有 outbox（gateway.rs:4390-4454）；管理发送幂等 skip 不覆盖时同（gateway.rs:1194） |
| `outbox_enqueue_partial_failure` | 部分段失败/跨 decision 重复（gateway.rs:4456-4528） |
| `outbox_enqueue_failed` | 管理发送 enqueue Err（gateway.rs:1215/1225） |
| `gateway_blocked` | 第二道 precheck 拦（gateway.rs:3856/3878；管理发送 gateway.rs:1003/1014） |
| `rewrite_requested` | targeted rewrite 前的中间 decision_review（gateway.rs:3083） |
| `revision_failed` | revision Skip / 二轮不过且不可恢复（final_review_status，gateway.rs:3320；gateway_status 由 `derive_revision_failure` 决定） |
| `revision_applied_approved` | revision 二轮通过（final_review_status，gateway.rs:3464；gateway_status 仍 approved） |
| `agent_skipped`（事件 kind 非 status） | 第一道 precheck 拦时事件名（gateway.rs:2400） |

**lifecycle 闭集**（run_envelope.rs:48-54 核证）：`started / running / completed / failed_before_decision / failed_after_decision / aborted_by_budget / aborted_by_external_signal`。本文件显式写入：COMPLETED（gateway.rs:1237/4418/4664）、FAILED_AFTER_DECISION（gateway.rs:1226/4501/4561）、ABORTED_BY_EXTERNAL_SIGNAL（gateway.rs:4258/4621）；其余由 `derive_lifecycle_from_status` 派生。

**source_kind 闭集**（run_envelope.rs:34-45 核证）：`inbound_message / follow_up_task / manual_send / principal_escalation / principal_clarification / system_incident`。trigger.kind()（types.rs:1852-1857）另有 `inbound / follow_up` 两值——**outbox.source_kind 用 trigger.kind()（gateway.rs:4321），envelope source_kind 用 SOURCE_KIND_*（gateway.rs:1302-1319），两套口径并存**；`PROACTIVE_TOUCH_SOURCE_KINDS=["follow_up","follow_up_task"]` 同时覆盖两口径（gateway.rs:5630-5635）。

### 4.2 precheck 豁免矩阵

| 门 | Inbound（真实客户消息） | FollowUp（普通跟进） | relay（synthetic relay Inbound） | deferred_wake（醒来任务，FollowUp kind=deferred_inbound_reply） | 管理发送（synthetic Inbound 无 message_id） |
|---|---|---|---|---|---|
| not_managed | ✓ | ✓ | ✓（唯一保留门） | ✓ | ✓ |
| cooldown | ✓ | ✓ | 豁免 | ✓ | ✓ |
| policy_cooldown | ✓ | ✓ | 豁免 | ✓ | ✓ |
| policy_wait_user_reply / consecutive_limit | 名义 ✓ 实际不触发（真实入站 message_id/dedupe_key 非空 → consecutive=0，gateway.rs:5391-5395/5422-5424） | ✓ | 豁免 | ✓ | ✓（合成入站不重置计数） |
| rate_limited | ✓ | ✓ | 豁免 | ✓ | ✓ |
| daily_limit | 豁免（gateway.rs:5289-5296） | ✓ | 豁免 | **✓（受限，见 §5 疑点 2）** | 豁免（Inbound） |
| expired | 不适用 | ✓ | 不适用 | ✓ | 不适用 |
| quiet_hours_deferred | 不适用（webhook 层权威；穿越边界放行，gateway.rs:5309-5316） | ✓ | 豁免 | 豁免（gateway.rs:5320） | 不适用 |
| context_changed | 不适用 | ✓ | 不适用（非 FollowUp） | 豁免（gateway.rs:5347） | 不适用 |

relay 豁免依据：`is_relay` 包住整个频控块（gateway.rs:5268-5341）；relay 判定 `escalation::is_principal_relay_trigger`（伪造哨兵 is_synthetic_relay=false 不豁免，测试 gateway.rs:8151-8175）。

### 4.3 ack 占位豁免清单（gateway.rs:5550-5559）

`cooldown / rate_limited / quiet_hours_deferred / expired / superseded_by_new_inbound / not_managed / context_changed / no_reply` 不补占位；其余 Inbound 零回复终态（held/blocked/precheck 类如 policy_*、daily_limit、blocked_*、held_by_ai_policy）都补。FollowUp 一律不补（gateway.rs:5569-5571）。task 路径不补（claim 已失去授权，fail closed，gateway.rs:2278-2286）。占位 key=`{source_event_id}#ack-placeholder`、decision_id=None、文案 `generate_holding_reply(GateHold)`。

### 4.4 硬编码常量 / 超时 / 上限

| 常量 | 值 | 位置 |
|---|---|---|
| 时间承诺弱启发词表 | 明天/后天/下周/下个月/稍后/晚点/回头/马上（8 个） | gateway.rs:100-109 |
| outbox 可覆盖状态集 | pending / in_flight / sent / delivery_unknown | gateway.rs:121（另 1504-1507） |
| commitments 保留条数 | `$slice: -8` | gateway.rs:139 |
| durable claim 监视 tick | 100ms | gateway.rs:274 |
| outbox max_attempts | 3（文本/媒体/名片/占位/过渡/管理发送全部） | gateway.rs:1152/1464/4325/4861/4956/5603 |
| rewrite/revision 预算包 | `4 + (second_reviewer? 1:0)` 次 LLM 调用 | gateway.rs:3049/3261 |
| forced_full 额外调用 | +1 | gateway.rs:2728 |
| revision LLM 超时 | 30s | gateway.rs:3390 |
| 评审摘要截断 | 160 chars | gateway.rs:714-715 |
| 请示骚扰门窗口 | 24h（count_pushes_today since） | gateway.rs:1964-1965 |
| daily_touch 窗口 | 滚动 24h | gateway.rs:5686 |
| consecutive_outbound 检查深度 | 最近 20 条 | gateway.rs:5437-5438 |
| 管理发送上下文窗 | `(recent_message_limit*6).clamp(24,80)` | gateway.rs:7460 |
| pending 任务加载 | limit 5，run_at 升序 | gateway.rs:7478-7481 |
| 句末标点集 | 。！？!?；;. | gateway.rs:5515 |
| 超长段硬切 | 2×max_chars | gateway.rs:5527 |
| PROFILE_SUMMARY_SOFT_CAP | 2000 chars | gateway.rs:5866 |
| MEMORY_SUMMARY_MAX_LINES / BYTES | 12 行 / 1200 字节 | gateway.rs:5948-5950 |
| 贝叶斯单轮观测截断 | MAX_BAYESIAN_SLOTS（外部常量，测试显示=6，gateway.rs:8304） | gateway.rs:6014 |
| 观察 upsert 初始 occurrences | 0 | gateway.rs:6108 |
| taxonomy candidate 初始权重 | 50 | gateway.rs:3631 |
| config 派生（非硬编码，runtime/config 提供） | run_token_budget / run_max_llm_calls / knowledge_max_tool_calls / run_token_budget_escalated / min_reply_interval_seconds / max_daily_touches / quiet_hours_* / wake_jitter_max_seconds / agent_reply_max_segment_chars / agent_reply_max_segments / account_daily_send_soft_cap / value_tier_*_threshold_cents / progressive_tier_enabled / reaction_gateway_parallel_enabled / bayesian_slot_min_hits / bayesian_slot_min_strong / operation_state_confidence_full_review_below | 各引用点 |

### 4.5 关键判定纯函数速查

- `text_send_eligible = should_reply && !reply_text.trim().is_empty()`（gateway.rs:2357-2359）
- `outbox_eligible = text_send_eligible && final_status ∈ {approved, revision_applied_approved} && relay 无泄漏`（gateway.rs:4180-4216）
- `media_send_allowed = outbox_eligible && has_assets`（gateway.rs:2166-2168）——媒体/名片资格与文本同源，杜绝孤立文件/孤立名片
- `should_run_send = outbox_eligible || media_pending`（gateway.rs:2173-2175）
- `daily_limit_applies_to = FollowUp`（gateway.rs:5621-5623）
- `should_send_ack_placeholder = inbound && !豁免清单`（gateway.rs:5569-5571）
- `check_context_changed_followup_pure = last_inbound_ms > task_created_ms`（严格大于，gateway.rs:7600-7608）

---

## 5. 偏差与疑点

1. **【注释过时】gateway.rs:3915-3924**：主去抖检查的注释写"必须在 apply_agent_updates 之前——后者无条件把 last_agent_run_at 推到 now"，但当前 inner 主路径已无内联 `apply_agent_updates` 调用（画像写移交 post_decision 投影 worker；主路径推进 last_agent_run_at 的是 gateway.rs:4669-4708 的授权后 `$max` 写）。检查本身仍必要（在写 decision_review/投影/推进频控锚之前放弃过时生成），但注释描述的因果对象已迁移。同类：gateway.rs:6178 `apply_agent_updates` 开头 set `last_agent_run_at`（投影回放时会再推一次锚，与 4673 的授权锚双写，语义相容但来源注释未更新）。
2. **【疑点·行为存疑】deferred_wake 醒来任务受 daily_limit/rate_limited/cooldown 约束且被拦时走 cancel 而非 reschedule**（gateway.rs:2380-2394 只对 `quiet_hours_deferred` reschedule；醒来任务是 FollowUp → `daily_limit_applies_to=true`）。醒来任务语义是"补欠客户的被动应答"，被 daily_limit 取消后该应答义务是否会由 webhook 层 durable task 重建，本文件不可见。注释 gateway.rs:5659-5662 仅承认**计数侧**（占 1 次额度）的不精确，未讨论**闸门侧**可取消醒来任务这一后果。标记疑点。
3. **【dead-ish code】`should_refresh_context` 恒 false**（gateway.rs:2496），run log 的 `context.refreshed` 永远 false、`context_needs_refresh` 只在管理发送 planner 里硬编码 true（gateway.rs:749）。上下文刷新机制在 planner 内联化后已停用，字段仅存审计形态。
4. **【dead-ish code】`promote_risks` 的空消费**：`let _ = &mut promote_risks;`（gateway.rs:2961）与 `let _ = promote_risks;`（gateway.rs:3586）是为抑制未使用告警——promote_risks 实际只被 finalize（gateway.rs:3200/3441 的 `promote_risks.clone()`）消费。
5. **【口径不对称·已确认合理但值得记录】rewrite/revision 后 `used_knowledge_ids = route_used_knowledge_ids(&knowledge_route)` 直赋**（gateway.rs:3127/3395-3396），未再走 `resolve_used_knowledge_ids` 的 KB-01 清空逻辑；因 rewrite/revision 恒以 `PromptTier::Full` 重生成（gateway.rs:3105/3379），业务知识确实注入，故直赋合理。但若未来改 revision 档位，此处会成为 grounding 口径漏洞。
6. **【双 normalize】gateway.rs:2940 与 2951** 对同一 decision 先后各调一次 `normalize_decision_runtime`（先用 initial_planner，再用 planner_from_decision 产出的 planner）。第一次的结果多数字段会被第二次覆盖；是否有依赖第一次归一的中间读（planner_from_decision 读 decision）未在本文件内完全确证。标记轻微疑点（需读 guards.rs 确认幂等性）。
7. **【已知产品取舍 B-01】**（gateway.rs:4228-4234 注释自认）：兜底去抖 guard 与多段 enqueue 循环之间存在 10-100ms/段的尾窗，极端时客户收两批回复。官方列为专项不修。
8. **【管理发送第二道 precheck 的返回语义】**：`ContactSendResult{review_approved:true, gateway_status=final_precheck.status}`（gateway.rs:1034-1041）——调用方会看到"评审通过但被频控拦"，与主链 `gateway_blocked` 写库状态并存；`review_approved` 字段名与实际 gateway 放行结果有歧义空间（仅记录，不判 bug）。
9. **【事件 kind 与状态串复用】**`blocked_by_safety_guard` 既是 finalize 硬门状态、又是 relay 泄漏守卫状态、又是"应发未入队"兜底状态（gateway.rs:4544 delivery_block_status 缺省值）；排障时不能仅凭该状态推断来源，需结合 `blocked_review` 事件与 outbox 记录。
10. **【非文本过渡的 review 双终态窗口】**：`maybe_handle_non_text_transition` 中 task 路径若 `authorize_task_outbox_if_owned` 失败，review 置 `stale_task_claim` 但 outbox 条目已创建（gateway.rs:1521-1541）——依赖 dispatcher 的 task-token fence 取消该条目（注释 gateway.rs:1381-1383 声明此设计），本文件内无法验证 dispatcher 行为，标记跨文件依赖。
11. **【occurrences 字段语义漂移】**`upsert_pending_projection_observation` 的 `$setOnInsert occurrences:0`（gateway.rs:6108）后由 `reconcile_stages(ledger_count,...)` 回写——occurrences 实际值来自 projection_observations 台账而非本行自增，读侧若直接理解为"本行 upsert 次数"会误读（跨文件语义，仅记录）。
12. **【settle 的 performance 写入无状态谓词】**（gateway.rs:1878-1884）：按 `{run_id}` 直接 `$set gateway_result.performance`，晚到的 settle 可能覆盖已终态 run log 的 gateway_result 子字段？实际只写 `.performance` 子键不动其余字段，安全；但若 run log 尚未创建（envelope started 失败路径）此 update 匹配 0 行静默无效。轻微记录。
13. **【ack 占位的 source_kind 用 trigger.kind()（"inbound"）】**（gateway.rs:5589/5599）而非 envelope 口径 `inbound_message`；与 4.1 所述双口径一致，不影响 proactive 计数（两口径都不在闭集），仅提醒审计查询时两种字符串都要匹配。

---

## 6. 覆盖自证

以 Read 工具分 13 段连续读取全文，段间无缝衔接、无跳读：

| 段 | 行号区间 | 覆盖内容 |
|---|---|---|
| 1 | 1-700 | 模块注释、imports、纯函数、入口族、快照加载、review 并行 helper |
| 2 | 701-1400 | manual_send_block_reason、管理发送 inner 全文、trigger_envelope_source、非文本过渡（前半） |
| 3 | 1401-2100 | 非文本过渡（后半）、ParallelReactionTask、abort_on_reaction_stop、gateway 外层、settle、trigger_principal_escalation、relay（前半） |
| 4 | 2101-2800 | relay（后半）、media 定序、状态动作闸、ack 守卫、inner 开头至 tier=Enough 分支 |
| 5 | 2801-3500 | tier Escalate/Clarify、tool_calling 防御、review 三分支、rewrite、finalize#1、revision（前半） |
| 6 | 3501-4200 | revision（后半）、taxonomy 软闸、context_changed、拦截分支、第二道 precheck、去抖、claim 复核、decision_review/投影、outbox_eligible+relay 守卫 |
| 7 | 4201-4900 | 兜底去抖、文本分段 enqueue 循环、部分失败处理、授权 CAS、频控锚、素材段 |
| 8 | 4901-5600 | 名片段、escalation 收尾、classify_send_receipt、send_outbound_message、trigger_message、precheck_send_gateway、operation_policy、分段函数、ack 常量/函数 |
| 9 | 5601-6300 | blocked、daily_limit/proactive filter/count、cancel/reschedule_task、detect_state_transition、churn、memory merge、贝叶斯映射、ProjectionWriteGuard、upsert 观察、apply_agent_updates（前半） |
| 10 | 6301-7000 | apply_agent_updates（后半：stage 状态机/强弱门/value_tier/C2/记忆/фenced 写/贝叶斯/建议信号/churn 事件/state 迁移事件）、apply_operating_memory_update（大部） |
| 11 | 7001-7700 | memory update 收尾、事件 details、write_decision_review、run log 族、persist events、confidence override、水位 filter、load_recent/context/pending、信号提取、write_event、context_changed 纯函数、taxonomy outcome（前半） |
| 12 | 7701-8400 | taxonomy outcome/apply（后半）、send_receipt_tests、tests（manual send/水位/占位/daily_limit/proactive filter/relay 哨兵/commitment/media 定序/贝叶斯/时间承诺/should_run_send/信号提取 部分） |
| 13 | 8401-9152 | tests 余部（suspected_deal/llm_signal/分段/幂等 base/text_send_eligible/context_changed/wake 措辞/taxonomy 全分支/display_name/state_transition/churn/memory merge），文件结束于 9152 行 |

外部核证（只读）：`review/gates.rs:479-501`（GatewayStatusFinal 变体与字符串映射）、`run_envelope.rs:34-54`（SOURCE_KIND_*、LIFECYCLE_*）、`types.rs:1507-1515/1852-1857`（HOLD_CATEGORY_*、AgentTrigger::kind）。

未在本文件内核证、引用时已注明"跨文件"的机制：outbox::enqueue 的幂等键构造、dispatcher 二次门与 task-token fence 的取消行为、post_decision worker 的回放时序、webhooks 的 durable task 重建、finalize_review_for_send / decide_revision / apply_revision_fallback 内部规则（review 模块）、multimodal 打桩、escalation 各 helper。

---

## 追记：22 号疑点终裁回写（2026-08-13，主会话执行）

- **疑点 2（deferred_wake 被 daily_limit 取消）重大修正**：前提大半过时——`DEFERRED_INBOUND_REPLY_KIND` 全仓无创建点、代码自标 legacy（`webhooks.rs:716-717`）；现行静默唤醒物化 `inbound_reply`（`webhooks.rs:117`）走 Inbound 语义，daily_limit/rate_limited 天然豁免。本记录 §4 豁免矩阵中 deferred_wake 列描述的是**防御历史残留行的死代码分支**，非现行主路径。仅 DB 历史 deferred 行存在被 cancel 的理论风险（reconcile_workspace_reply_obligations 会收敛）。
- 疑点 1（apply_agent_updates 注释过时）终裁成立；其余逐条终裁见 22 号 §2（01 号 13 条：缺陷/设计/不成立分布及证据链）。
