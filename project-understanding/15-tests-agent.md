# 测试全集 A（agent 主链路）深读记录（核证日期 2026-08-13）

> 读法说明：本记录基于工作区当前版本（git status 显示多个测试文件有未提交修改 M，读的是修改后的工作区内容）。所有 file:line 引用均为亲读核证。`#[ignore]` 标记表示该测试需要 Docker（testcontainers MongoDB）或真实 LLM，默认 `cargo test` 不执行，由 CI `--ignored` 或手动执行。

## 1. 主题→文件清单（本组覆盖的每个文件）

| 主题 | 文件 |
| --- | --- |
| 测试基础设施 | `tests/common/mod.rs`（TestApp/TestLlmGenerator/等待辅助） |
| 网关/决策/评审 | happy_path_run.rs、full_flow_suite.rs、decision_review_status_e2e.rs、revision_recheck_action_gate.rs、autonomy_protocol_pbt.rs、human_like_threshold_pbt.rs、pressure_risk_threshold_pbt.rs、debounce_barge_in_run.rs、debounce_pipeline_integration.rs、quiet_hours_deferral.rs、run_envelope_integration.rs、conversation_mode_decision_schema.rs、string_fact_risk_guard.rs、sr072_policy_fail_closed.rs |
| 状态机 | state_transition_pbt.rs、c2_operation_state_derivation_e2e.rs、c2_state_transition_cross_domain.rs、intent_trajectory_pbt.rs |
| outbox/发送 | outbox_integration.rs、outbox_scope_integration.rs、send_ledger_integration.rs、sr034_task_send_fencing.rs、sr172_outbox_projection.rs、sr177_durable_inbound_handoff.rs、hc004_outbox_webhook_scope_redlines.rs、hc004_scope_redlines.rs、account_offline_defer_integration.rs、account_round_robin_pbt.rs |
| 记忆 | memory_card_invariants.rs、memory_card_write_occ.rs、operating_memory_insert_idempotent.rs、sr029_memory_commit_recovery.rs、sr181_operator_memory_revocation.rs |
| 反应 | reaction_claim_lock.rs、reaction_stop_cancels_outbox_integration.rs |
| 请示/引荐 | principal_decision_channel.rs、ask_human_phase1_e2e.rs、referral_card_push_integration.rs、real_llm_principal_channel.rs、real_llm_principal_relay.rs、escalation_push_time_reassign.rs、hc020_management_command_protocol.rs |
| campaign/planner/主动触达 | campaign_dispatch_integration.rs、campaign_segment_coverage.rs、planner_block_rate_backoff.rs、planner_calendar_care.rs、planner_commitment_due.rs、planner_silent_followup.rs、cold_reactivation_idempotent_pbt.rs、sr135_proactive_outreach.rs |
| media | media_asset_crud_integration.rs、media_asset_send_integration.rs、media_storage_consistency.rs |
| 运行参数/管理流 | sr094_runtime_parameters.rs、transactional_admin_flows.rs、contacts_batch_enable.rs |
| 入站/webhook | webhook_contact_upsert_integration.rs、last_inbound_split.rs |
| 仿真/隔离 | simulation_no_sideeffect_integration.rs、dry_run_isolation.rs |
| 行为信号 | behavior_signal_idempotent_pbt.rs、behavior_signal_smoke.rs |
| worker/任务 | worker_reclaim.rs、review_task_now_claim.rs |
| deal/outcome | suspected_deal_e2e.rs、deal_event_scope_integration.rs、outcomes_autonomy_endpoint.rs、outcome_snapshot_freeze_integration.rs、outcome_task_workspace_dedupe.rs |
| LLM 支撑 | llm_retry_jitter.rs |

## 2. 逐文件深读

### 2.0 tests/common/mod.rs（共享测试基础设施，1074 行，非测试文件）

不含测试，是全部集成测试的地基。核心组件：

- **TestMongo**（mod.rs:44-97）：本地定义的 testcontainers 镜像 `mongo:5.0.6`，两种模式——Standalone（默认）与 ReplSet（`--replSet rs` + `rs.initiate()`，mod.rs:83-94）。多文档事务测试必须用 ReplSet（standalone 无法 commit 事务，mod.rs:401-409 注释）。
- **TestApp::start / start_repl_set**（mod.rs:397-409）：每次启动用独立随机库名 `wechatagent_test_{uuid}`（mod.rs:452）；支持 `TEST_MONGODB_URI` 环境变量走外部 mongod（mod.rs:420-424），此时 `cleanup()` 显式 drop 库（mod.rs:577-586）。启动顺序复刻生产：`migrations::run` → `ensure_indexes`（mod.rs:457-460），再补跑 `m006_taxonomy_seed::run_step` 重新 seed 销售域字典（因 m012 在非 production 环境会删掉 m006 的 customer_stage/intent_level/objection_type seed，mod.rs:462-471），随后 `ensure_prompt_pack_v2`（mod.rs:498-504）、预热进程级 taxonomy 缓存（mod.rs:510-512）与 DomainProfile 缓存（mod.rs:522-524，防止 LazyLock 单例残留上一个测试 DB 的 active profile 导致 customer_stage 维度被剔除）。
- **TestLlmGenerator**（mod.rs:162-286）：手写 mock LLM，`push_response` 预排队 JSON。关键设计：**并非纯 FIFO** —— Knowledge Router、Reply、Reviewer、ClaimGate 可并发执行，`pop_or_error` 按响应 JSON 的顶层 schema 指纹分类定向消费（mod.rs:214-267）：有 `requiresEvidence+claims+catalogClaims` → ClaimGate；有 `decisionPhase` → Reply；有 `approved+scores` → Reviewer；有 `action` → Knowledge；其余 Other 走 FIFO。请求侧按 system prompt 关键词识别（"independent semantic claim reviewer"→ClaimGate、"运营知识库的 wiki 研究员"→Knowledge、"独立审核"等→Reviewer、"shouldReply"+"conversationMode"→Reply，mod.rs:239-257）。无匹配响应时报错并打印队列指纹（mod.rs:268-283）。
- **ClaimGate fixtures**（mod.rs:292-354）：`independent_claim_gate_pass_json`（无需证据）、`independent_claim_gate_unsupported_business_json`（开放世界业务断言无来源→拦）、`independent_claim_gate_verified_knowledge_json(chunk_id, quote)`（能力声明由已核验 chunk 背书，evidenceRefs 格式 `verified_knowledge:{chunk_id}`，mod.rs:345）。注释红线：gateway 在**每轮 Review 后都会再调一次** `user.review.claim_gate`，集成测试必须显式排入完整 schema（mod.rs:288-291）。
- **等待辅助**：`wait_for_outbox_processed`（按 `_id` 轮询 outbox 到终态 `sent|failed_terminal|canceled|delivery_unknown`，100ms 步长，mod.rs:743-771）；`wait_for_outbox_processed_by_run_id`（按 run_id 轮询，mod.rs:857-885）；`complete_latest_post_decision`（找最新 `post_decision_status="pending"` 的 decision_review，push 投影 JSON、spawn `run_post_decision_worker`、轮询到 `completed`，mod.rs:776-849）——说明画像/taxonomy/记忆写入已被抽成异步"post-decision projection"阶段。
- **状态重建辅助**：`rebuild_app_state_with_mcp_url`（wiremock MCP，mod.rs:893-914）、`rebuild_app_state_with_real_llm`（真实 LLM+桩 MCP，"绝不真发微信"，second_reviewer_llm=None 单脑复审，mod.rs:1048-1073）、`evolution_release_state`（mod.rs:919-945）、`insert_released_prompt_proposal`（mod.rs:950-1038）。
- **test_config 默认值**（mod.rs:589-735）：`message_debounce_window_ms=4000`、`progressive_tier_enabled=true`（走 Lean/Full 两程循环）、`reaction_gateway_parallel_enabled=false`、`account_send_min/max_interval_ms=0`（账号级拟人间隔闸测试默认关，需要的测试自行覆盖，mod.rs:605-612）、`agent_reply_max_segment_chars=120` / `agent_reply_max_segments=4`、strategic planner / cold contact / evolution / digest / ingest 等 worker 测试默认全 disabled。
- `ensure_test_account`（mod.rs:140-156）：insert-only upsert 注册账号 scope——worker/scoped MCP 解析 fail-closed（未知账号拒绝），测试需显式注册。

### 2.1 tests/happy_path_run.rs（624 行，3 个测试，全部 `#[ignore]`，testcontainers+mock LLM）

**测试 1：`consolidate_contact_memory_writes_core_fact_via_mock_llm`（happy_path_run.rs:101-201）**
- 业务不变量：记忆固化 happy path——给定 1 条 pending MemoryCandidate，`consolidate_contact_memory` 恰好调 1 次 LLM（happy_path_run.rs:145-149）；LLM 输出的 coreFacts 落入 `operating_memory.memory_card.core_facts`（经 `MemoryCardTyped.as_text()` 兼容 Plain/Structured 两种表示，happy_path_run.rs:168-180）；候选被消费、pending 计数归零（happy_path_run.rs:182-200）。
- 手法：testcontainers + mock LLM push 一条完整 memoryCard JSON（含 coreFacts/recentFacts/preferences/doNotDo/objections/openLoops/openQuestions/deprecatedFacts/conflicts/confirmedFacts/commitments 全字段，happy_path_run.rs:122-138）。

**测试 2：`autonomy_full_loop_with_revision`（happy_path_run.rs:347-467）**
- 业务不变量：single-shot revision 完整链路的 LLM 调用次数恒等式——**Reply ×2 + Review ×2 + ClaimGate ×2 = 6**（happy_path_run.rs:403-407）。Reviewer 首轮 `needsRevision=true` + revisionDirection → 二轮 Reply 改写 → 二轮 Review 通过 → 终态 `final_review_status="revision_applied_approved"`（happy_path_run.rs:430-434）、`revision_applied=true`（happy_path_run.rs:425-429）、`pre_revision_summary`/`post_revision_summary` 均非空（happy_path_run.rs:435-450）。approved 路径必须以 run_id 维度在 `agent_send_outbox` 入队一行（happy_path_run.rs:452-466）。
- 关键 fixture：`reply_agent_decision_json`（happy_path_run.rs:278-307，含 decisionPhase/userUnderstanding/relationshipRead/operationGoal/selfCritique/riskSelfCheck/runMode/autonomyMode/conversationMode 等完整决策 schema）；`review_agent_pass_json`（happy_path_run.rs:312-345，scores 八维：humanLike/emotionalValue/productAccuracy/relationshipProgress/conversionReadiness/pressureRisk/boundaryPrivacySafety/factRisk + claimAnalysis + needsRevision/revisionDirection/shouldHold/holdCategory/selfCritiqueAddressed）。
- 入口：`handle_managed_message(&app.state, contact, &inbound)`（happy_path_run.rs:399）。

**测试 3：`autonomy_tool_loop_happy_path`（happy_path_run.rs:469-624）**
- 业务不变量：agent-first 知识路由 tool-loop——LLM 调用序列为 knowledge_agent(open_chunk→answer) ×2 + Reply Lean 探测(sufficiency="need_more_context"+missingTier="full") + Reply Full + Review + ClaimGate = 6 次（happy_path_run.rs:503-557）。终态 `approved`（happy_path_run.rs:574-578）。`knowledge_route.toolTrace`（camelCase BSON 键）至少含 list_catalog/open_chunk/answer 三段，且 list_catalog 恒为第一段（DB 拉目录不耗 LLM）（happy_path_run.rs:580-612）。
- 知识可见性前提：knowledge_agent 的 list_catalog/open_chunk 只暴露 `integrity_status="verified"` 且 `status="active"` 的 chunk（fixture 注释，happy_path_run.rs:231-235）。
- 能力声明背书：ClaimGate 用 `independent_claim_gate_verified_knowledge_json(chunk_id, quote)` 表达"能力声明由已核验知识背书"（happy_path_run.rs:542-546）。

### 2.2 tests/decision_review_status_e2e.rs（171 行，1 个测试，`#[ignore]`，testcontainers 数据层）

**测试：`decision_review_correlates_run_log_status_and_hold_category`（decision_review_status_e2e.rs:129-171）**
- 业务不变量：决策评审列表 API 的两个状态字段来源——`finalReviewStatus` 取自 AgentRunLog **顶层 snake 字段** `final_review_status`（decision_review_status_e2e.rs:94-95、151-155）；`holdCategory` 取自 run_log `review` doc 内 **camelCase 键**（源自 DecisionReviewResult 的 rename_all="camelCase"，decision_review_status_e2e.rs:73-75、156-160）。两者按同 run_id 关联。
- 值域样例用 AI-internal 状态名 `held_by_ai_policy`（无"人工接管"语义）。
- 手法：因 `routes::reviews` 的 `fetch_run_status`/`decision_review_json` 是 pub(super) 跨 crate 不可达，测试**复刻关联逻辑**走数据层（typed collection 真实写入经 Mongo serde 一圈再读回，decision_review_status_e2e.rs:7-14、104-127）。疑点：属"复刻式测试"，若生产投影逻辑改动，本测试不会自动失败（见第 5 节）。

### 2.3 tests/revision_recheck_action_gate.rs（37 行，1 个测试，`#[ignore]` 且**空函数体**）

**测试：`revision_into_forbidden_state_is_held`（revision_recheck_action_gate.rs:24-37）**
- 目标不变量（仅文档化，未实现断言）：GATE-1——revision 改写后若 operation_state 迁入"禁止 reply"的态，动作闸须在二次 finalize 后复检，置 `held_by_ai_policy` 而非放行进 outbox；复检链 = load_operation_state_policy_for_contact → classify_decision_action → enforce_state_action_policy 命中 forbidden → finalize_status=Held + review.approved=false + should_reply=false + 追加 `state_action_policy_blocked` risk + 落审计事件（revision_recheck_action_gate.rs:5-15）。
- **弱点（重要疑点）**：函数体为空（revision_recheck_action_gate.rs:26-37 全是注释），是"CI 骨架"——revision 路径调完整 Reply Agent 无 mock seam，作者声明正确性由"Step 3 代码审查 + lib 基线"保证（revision_recheck_action_gate.rs:17-19）。此文件跑起来永远绿，不锁定任何行为。

### 2.4 tests/quiet_hours_deferral.rs（153 行，1 个测试，`#[ignore]`，testcontainers）

**测试：`quiet_hours_reuses_single_reply_obligation`（quiet_hours_deferral.rs:66-153）**
- 业务不变量：静默时段（quiet hours）与普通 debounce 共用**同一条持久化 `inbound_reply` 义务任务**；旧的 `deferred_inbound_reply` 任务种类必须不再产生（quiet_hours_deferral.rs:1-4、128-139）。
- `ensure_wake_followup_task(&state, &contact, 8, 8)` 首次调用后：恰好 1 条 `kind="inbound_reply"` 任务，status=pending、`review_required=true`、`run_at` 在未来、**`expires_at=None`（被动回复义务永不过期）**（quiet_hours_deferral.rs:123-126）、`gateway_status="quiet_hours_waiting"`（quiet_hours_deferral.rs:127）。
- 幂等：第二次调用 `ensure_wake_followup_task` 不新建任务，计数仍为 1（quiet_hours_deferral.rs:141-152）。

### 2.5 tests/full_flow_suite.rs（1414 行，11 个测试，全部 `#[ignore]`，testcontainers+mock LLM+wiremock）

合并套件：A 组走 `handle_managed_message`（无 barge-in guard），B 组走 `handle_managed_message_aggregated`（带协作式 guard 闭包）。文件头声明本套件不替换 happy_path_run/debounce_barge_in_run（full_flow_suite.rs:5-7）；调度器本身的去抖/generation 抢占由 `src/webhooks.rs` 纯函数单测覆盖，这里聚焦"抢占信号穿过网关后的真实落库副作用"（full_flow_suite.rs:23-25）。

**A1 `full_flow_a1_direct_approved_enqueues_outbox`（full_flow_suite.rs:312-444）**
- 直发路径 LLM 调用恒等式：Reply ×1 + Review ×1 + ClaimGate ×1 = **3 次**（full_flow_suite.rs:352-356）；`final_review_status="approved"`、`revision_applied=false`。
- **性能可观测性合同**：`gateway_result.performance` 子文档在 Gateway 返回前必须持久化，且写入不得覆盖既有 `precheck.allowed/status`（full_flow_suite.rs:370-383）；`performance.path.kind="direct"`（稳定路径分类，full_flow_suite.rs:384-391）；`performance.llmLogFlush` 的 queued=persisted=3、failed=0、batchSucceeded=true（full_flow_suite.rs:392-398）；`performance.eventLogFlush` queued≥1 且 persisted=queued（full_flow_suite.rs:399-407）；`performance.stages` 必须含 8 个关键阶段耗时键：run_snapshot/business_preload/reply_agent/reviewer/claim_gate/finalize/outbox_enqueue/llm_audit_flush（full_flow_suite.rs:408-422）。
- **LLM 审计 flush 同步性**：Gateway 返回前 `llm_call_logs` 已落 3 条（full_flow_suite.rs:423-433）。

**A1b `full_flow_a1b_unsupported_business_fact_is_locally_rewritten`（full_flow_suite.rs:448-523）**
- 不变量：开放世界业务事实（"到店带身份证"）无来源时，ClaimGate 标记 unsupported → 触发一次 targeted rewrite（LLM 恒等式 6 次 = Reply×2+Review×2+ClaimGate×2，full_flow_suite.rs:505-509）；改写稿只保留透明不确定表达（"我先核对清楚再告诉你"）、二次 ClaimGate 通过后正常入 outbox；outbox.content 等于改写稿且不含"身份证"（full_flow_suite.rs:521-522）。

**A1c `full_flow_a1c_persistent_unsupported_business_fact_is_blocked`（full_flow_suite.rs:527-599）**
- 不变量：改写后仍坚持同一无来源业务事实 → 二次 ClaimGate 再标 unsupported → `final_review_status="blocked_by_safety_guard"`（full_flow_suite.rs:578）。LLM 恒等式 **7 次**（6 次主链 + 1 次"中性占位生成尝试"，full_flow_suite.rs:571-575）。
- **客户回应保障**：blocked 入站仍会收到 1 条中性占位（outbox 恰 1 行），但占位内容绝不含原违规事实（"身份证"/"带证件"）（full_flow_suite.rs:579-591）。blocked 是可审计终态而非管道错误（handle 返回 Ok，full_flow_suite.rs:568-570）。
- review.claimAnalysis.unsupportedNonProductBusinessClaimCount=1（full_flow_suite.rs:592-598）。

**A2 `full_flow_a2_single_shot_revision`（full_flow_suite.rs:603-727）**
- 与 happy_path_run 测试 2 同构，额外断言：`performance.path.kind="revision"`（full_flow_suite.rs:688-699）、llmLogFlush queued=persisted=6（full_flow_suite.rs:700-705）、`llm_call_logs` 落 6 条（full_flow_suite.rs:706-716）。

**A3 `full_flow_a3_no_reply_skips_review_and_outbox`（full_flow_suite.rs:730-772）**
- 不变量：`shouldReply=false` → 不进 Review（should_run_review 返回 false），**恰 1 次 LLM 调用**；run log `status="no_reply"`；0 outbox（full_flow_suite.rs:757-771）。

**A4 `full_flow_a4_knowledge_tool_loop`（full_flow_suite.rs:775-896）**
- 与 happy_path_run 测试 3 同构（6 次 LLM，toolTrace 含 list_catalog/open_chunk/answer，approved+outbox）。

**B1 `full_flow_b1_barge_in_aborts_before_outbox`（full_flow_suite.rs:902-966）**
- 不变量：guard() 恒 true（模拟生成期间新入站到达）→ run log `status="superseded_by_new_inbound"`（full_flow_suite.rs:943-948）；0 outbox；**`last_agent_run_at` 不推进**（保证重算 precheck 不被误判 rate_limited，full_flow_suite.rs:953-965）。

**B2 `full_flow_b2_no_barge_in_completes_normally`（full_flow_suite.rs:969-1050）**
- 不变量：guard() 恒 false → approved + outbox 一行 + `last_agent_run_at` 推进；并断言 guard 确实在落盘/入队检查点被调用（AtomicBool，full_flow_suite.rs:1005-1019）。

**B3 `full_flow_b3_barge_in_then_recompute_sends_once`（full_flow_suite.rs:1055-1159）**
- 不变量："用户连发后说完，最终只发一次"——第一遍 guard=true 被弃（0 outbox、不推进 last_agent_run_at）；第二遍 guard=false 落地；最终 outbox 恰 1 行（full_flow_suite.rs:1142-1146）。

**`pending_delivery_is_not_learned_as_user_reaction`（full_flow_suite.rs:1162-1273）**
- 不变量：outbox 尚 pending（未真实送达）时，第二条入站调 `record_user_reaction` **不触发 reaction LLM（0 次调用）**（full_flow_suite.rs:1237-1245）；review.outcome_status 停在 "pending" 占位（gateway 创建 review 即置 outcome_status="pending"，注释引 gateway.rs:5199，full_flow_suite.rs:1255-1262）；review.status 在送达前只能是 "outbox_enqueued"（full_flow_suite.rs:1220-1223）。

**`delivery_commits_promise_and_follow_up_only_after_sent`（full_flow_suite.rs:1276-1414）**
- 不变量：决策中的 commitment（lastCommitment/commitment.dueAt）与 followUp 任务**只能在 dispatcher 确认真实送达后提交**——pending 阶段 contact.commitments 为空、无 pending follow_up 任务（full_flow_suite.rs:1337-1363）；经 wiremock MCP + `atomic_claim_pending` + `process_entry` 真实投递后：review.status="sent"、commitments 出现该承诺文本、follow_up 任务恰 1 条且 `source_decision_id` 关联该 review（full_flow_suite.rs:1365-1413）。
- 手法备注：投递用 `rebuild_app_state_with_mcp_url` + `ensure_test_account`（账号 scope fail-closed，需显式注册）；MCP mock 返回 `structuredContent.newMsgId`（full_flow_suite.rs:44-56）。

### 2.6 tests/autonomy_protocol_pbt.rs（465 行，3 个 property，非 ignore，纯函数 PBT，64 cases/条）

配套 `autonomy_protocol_pbt.proptest-regressions` 回归种子文件存在。

**P1 `p1_autonomy_required_fields_violation_always_emits_risk_tag`（autonomy_protocol_pbt.rs:147-164）**
- 不变量（R1.3/R3.5/R3.9）：决策 JSON 的 12 个自治协议字段（7 个 R1.3 必填叙述字段 user_understanding/relationship_read/operation_goal/knowledge_need_reason/memory_update_reason/self_critique/risk_self_check + 5 个 R3.2 枚举字段 risk_level/knowledge_need/run_mode/autonomy_mode/operation_state）任一为空或枚举非法时，`RawAgentDecision::validate_and_promote` 输出的 risks 必含 `missing_required_field:*` / `invalid_enum_value:*` / `invalid_type:*` 之一（autonomy_protocol_pbt.rs:143-163）。
- 基线 fixture 把叙述字段填 >20 unicode chars（critical-turn 最低字符数兜底，autonomy_protocol_pbt.rs:46-47）。operation_state 的字典成员检查不在 validate_and_promote 而在 gateway/状态机 guard（autonomy_protocol_pbt.rs:101-103 注释）。
- 弱点：kind=2（invalid type）实际退化为 missing field 注入（serde bool 无法运行时注入 String，autonomy_protocol_pbt.rs:122-133）——invalid_type 分支未被 PBT 真正覆盖。

**P3 `p3_budget_exceeded_no_review_consistent`（autonomy_protocol_pbt.rs:198-235）**
- 不变量（R3.7/R3.8/R3.10）："未执行独立 Reviewer 不发送"——`local_decision_review`（本地 fallback）在预算超额/未超额 × needs_review true/false 的**全部组合**下都 `approved=false`（autonomy_protocol_pbt.rs:220）。
- 预算超额时 risks 恰为 `["budget_exceeded_no_review"]`（保留 blocked_by_budget 合同，autonomy_protocol_pbt.rs:221-226）；非预算路径必须是可审计安全 hold：`should_hold=true` + risks 含 `required_reviewer_not_executed`（autonomy_protocol_pbt.rs:227-234）。
- `RunBudget::new(run_id, token_budget, max_llm_calls, tool_call_budget)`，`record_call(tokens)` 一次即可跨阈值（autonomy_protocol_pbt.rs:204-208）。

**P2 `p2_single_shot_revision_caps_reply_calls_at_two`（autonomy_protocol_pbt.rs:407-464）**
- 不变量（R2.3/R2.4/R2.8）：任意（首轮 review、二轮 review、budget_exceeded）组合下 (1) Reply Agent 调用次数硬上限 ≤2；(2) 进入 Proceed 且二轮仍失败 → should_reply=false + status="revision_failed" + 恰 2 次 Reply；(3) Skip 前置（revisionDirection 空 / 预算超额）→ revision_failed 且不再调 Reply（恰 1 次）。
- **手法弱点（重要）**：这是"模型测试"——`run_revision_loop`（autonomy_protocol_pbt.rs:316-370）是**测试内手写的 gateway 控制流复刻模型**，文件头用注释表格声明与 `run_user_operation_gateway_inner`（gateway.rs:706-924，引用行号为写测试时的快照）一一对应（autonomy_protocol_pbt.rs:242-274）。它验证的是模型自身的性质；若生产 gateway 控制流漂移，本测试不会失败（见第 5 节疑点）。

### 2.7 tests/human_like_threshold_pbt.rs（153 行，4 个 property，非 ignore，纯函数 PBT）

对象：`review_passed(&DecisionReviewResult, &UserRuntimeParameters)` 纯函数 + `UserRuntimeParameters::default()` 阈值。ReviewScores 字段名注意：hallucination_score（即 factRisk 维度）、knowledge_grounding_score（即 productAccuracy 维度）（human_like_threshold_pbt.rs:24-31）。

- **P1 below_threshold_blocks**（human_like_threshold_pbt.rs:41-56）：`human_like < runtime.human_like_rewrite_below` 且其余分项满分、approved=true → review_passed=false。
- **P2 at_or_above_threshold_passes**（human_like_threshold_pbt.rs:64-77）：`human_like ≥ 阈值` → 通过。
- **P3 approved_false_overrides_score**（human_like_threshold_pbt.rs:85-97）：approved=false 一票否决，无论分数多高。
- **P4 threshold_boundary_is_inclusive_above**（human_like_threshold_pbt.rs:109-152）：边界语义是 `<`（threshold-1 拦、threshold 本身过、threshold+1 过）；同时扰动其它分项噪声（emotional_value ≥ emotional_value_rewrite_below、knowledge_grounding_score ≥ product_accuracy_block_below、hallucination_score < fact_risk_block_at）验证边界判定不受干扰。

### 2.8 tests/pressure_risk_threshold_pbt.rs（163 行，4 个 property，非 ignore，纯函数 PBT）

- **P1 above_threshold_blocks**（pressure_risk_threshold_pbt.rs:43-58）：`pressure_risk ≥ runtime.pressure_risk_block_at` 且 ≠0 → 拦截。边界语义与 humanLike 相反：**`>=` 即拦**。
- **P2 zero_pressure_risk_passes_legacy**（pressure_risk_threshold_pbt.rs:66-78）：**pressure_risk==0 是 legacy 豁免**（reviewer 未填分/老数据反序列化默认值 0 不参与拦截）——这是一个重要的历史兼容语义。
- **P3 below_threshold_passes**（pressure_risk_threshold_pbt.rs:84-103）：[1, threshold-1] 区间不拦。
- **P4 threshold_boundary_is_strict**（pressure_risk_threshold_pbt.rs:114-163）：threshold-1 过 / threshold 拦 / threshold+1 拦；并验证**双闸 AND 语义**——humanLike fail 不能被 pressure_risk pass 抵消（pressure_risk_threshold_pbt.rs:153-161）。

### 2.9 tests/debounce_barge_in_run.rs（345 行，2 个测试，全部 `#[ignore]`，testcontainers+mock LLM）

直调下游 `handle_managed_message_aggregated`（绕过 runner）。调度器纯逻辑（spawn-vs-bump/generation）由 `src/webhooks.rs` 单测覆盖（debounce_barge_in_run.rs:10-12）。

**`barge_in_aborts_before_outbox_and_does_not_advance_last_run`（debounce_barge_in_run.rs:157-248）**
- 不变量：决策+审查均成功、但 guard（should_abort_send 协作式中止）在落盘/入队检查点返回 true → 网关在 apply_agent_updates / outbox 之前放弃：run log `status="superseded_by_new_inbound"`（debounce_barge_in_run.rs:214-218）、0 outbox（debounce_barge_in_run.rs:221-232）、`last_agent_run_at` 保持 None（apply_agent_updates 在检查点之后未执行，debounce_barge_in_run.rs:234-247）。

**`no_barge_in_completes_normally_and_enqueues_outbox`（debounce_barge_in_run.rs:252-345）**
- 不变量：guard 恒 false → approved + outbox 一行 + last_agent_run_at 推进；guard 确实被调用（AtomicBool 验证检查点存在，debounce_barge_in_run.rs:285-299）。
- （与 full_flow_suite B1/B2 内容同构——两份测试锁同一合同。）

### 2.10 tests/debounce_pipeline_integration.rs（463 行，3 个测试，全部 `#[ignore]`，testcontainers+mock LLM，真 async runner）

真的走 `register_inbound` + `run_debounce_pipeline`（webhooks 层 runner），覆盖"去抖睡眠→快照 generation/最新入站→reload→reaction→聚合网关→退休/重算"完整循环（debounce_pipeline_integration.rs:1-6）。文件头声明测试设计命门（debounce_pipeline_integration.rs:10-20）：`static PENDING` 跨测试共享 → 每测试唯一 wxid；"网关执行中途抢占"本质是竞态无法确定性复现 → Step2/Step3 降级为可确定性验证的子命题，绝不写靠 sleep 凑时序的 flaky 断言。

**`three_rapid_inbounds_aggregate_into_single_gateway_run`（debounce_pipeline_integration.rs:226-304）**
- 不变量：同一去抖窗口内（50ms）连发 3 条入站 → 只跑一次聚合网关：agent_run_logs 恰 1 行、outbox 恰 1 行（不重复回复）、`llm.calls()==3`（decision+review+ClaimGate；全新 contact 首轮 reaction claim 拿不到已 sent 的 decision_review → 跳过 reaction LLM）（debounce_pipeline_integration.rs:289-303）。
- register 语义：首条 register_inbound 返回 spawned_now=true 并 spawn runner；后续同窗口 register 只 bump（debounce_pipeline_integration.rs:262-283）。

**`runner_uses_latest_inbound_snapshot_for_decision`（debounce_pipeline_integration.rs:315-388）**
- 不变量：runner 在去抖窗口内被后到入站刷新后，快照**最新**入站做决策——decision_review.inbound_message_id = 最后一条消息 id（debounce_pipeline_integration.rs:364-378）；聚合仍只一轮（3 次 LLM、1 行 outbox、1 行 run log）。

**`late_inbound_bumps_generation_without_duplicate_spawn`（debounce_pipeline_integration.rs:397-463）**
- 不变量（不丢消息）：runner 存活期间晚到 register → spawned_now=false（不重复 spawn 第二个 runner）、generation 从 1 bump 到 2（晚到消息不被静默丢弃）（debounce_pipeline_integration.rs:425-449）；最终 outbox 仍恰 1 行（晚到入站被同一 runner 聚合，无重复回复）。

### 2.11 tests/run_envelope_integration.rs（524 行，5 个测试，全部 `#[ignore]`，testcontainers+自定义探针 LLM）

Run Envelope（R0.10）——每次 agent run 的"信封"生命周期保证。用两个自定义 LlmProvider 探针（EnvelopeOrderProbeLlm：在被调用瞬间查库确认 started 信封已存在，然后返回 Err 或 panic，run_envelope_integration.rs:48-92；DecisionThenPanicProbeLlm：第 1 次调用返回合法决策、第 2 次查 lifecycle=running 后 panic，run_envelope_integration.rs:94-167）。

**`envelope_started_written_before_any_llm_call`（run_envelope_integration.rs:256-310）**
- 不变量：入口写信封**先于任何 LLM 调用**——LLM 被调用时 lifecycle="started" 的 run log 已持久化（run_envelope_integration.rs:279-282）；LLM 失败后终态 lifecycle="failed_before_decision"、status="internal_error"、error_summary 含失败原因；且**终态写更新原信封而非新插一行**（count=1，run_envelope_integration.rs:302-309）。

**`same_run_id_second_insert_triggers_duplicate_key_error`（run_envelope_integration.rs:312-355）**
- 不变量（R0.2）：`write_run_envelope_started` 同 run_id 二次 insert 因 unique(run_id) 索引触发 DuplicateKey 失败（错误信息含 duplicate/e11000，run_envelope_integration.rs:345-354）。

**`update_one_falls_back_to_insert_with_recovery_event`（run_envelope_integration.rs:357-406）**
- 不变量（R0.2 兜底）：`update_run_envelope_terminal` 命中 matched_count==0（信封丢失）时走单次 insert 兜底，并写 `agent_events kind="run_envelope_recovered_via_insert"`、status="warning" 的审计事件（run_envelope_integration.rs:389-405）。

**`panic_in_pipeline_marks_lifecycle_failed_before_decision`（run_envelope_integration.rs:408-452）**
- 不变量（R0.6）：Reply Agent panic → panic 继续向外传播（catch_unwind 层面可观察），但信封终态 lifecycle="failed_before_decision"、`error_summary="unhandled_panic: probe llm panic"`（panic payload 被保留，run_envelope_integration.rs:443-451）。

**`panic_after_reply_decision_marks_lifecycle_failed_after_decision`（run_envelope_integration.rs:454-524）**
- 不变量：决策成功后（reviewer 阶段）panic → lifecycle="failed_after_decision"；Review 被调用时 lifecycle 已推进到 "running"（started→running 转移发生在 Reply 决策后，run_envelope_integration.rs:487-490）；running 转移时已存 decision 快照（log.decision 非空，run_envelope_integration.rs:509-512）；失败关闭原信封（count=1）。

### 2.12 tests/state_transition_pbt.rs（166 行，1 个 PBT + 5 个单测，非 ignore，纯函数）

对象：`check_state_transition(Some(&config), from, to)` + `default_user_operation_state_machine()`（9 个销售态：new_contact/relationship_building/need_discovery/solution_fit/objection_handling/commitment_followup/customer_success/cooldown/dormant_reactivation，state_transition_pbt.rs:22-32）。属基线四 PBT 文件之一。

- **主 PBT `check_state_transition_matches_reference`**（state_transition_pbt.rs:96-108）：引擎与闭式参考实现双向一致。允许迁移当且仅当：状态机为空（向后兼容）/ 目标标 `allowFromAny:true` / from 在目标 `allowedFrom` 列表 / from 空且目标标 `initial:true`（state_transition_pbt.rs:1-9、63-92）。
- 闭式参考含"问题 E 修复"注释：**目标态不存在 = fail-closed 拦截**（此前 target-miss early-return 是 fail-open 漏放，会让 LLM 输出的未知 customer_stage 经 C2 写成"幻影 operation_state"旁路 policy enforcement，state_transition_pbt.rs:70-77）。
- 单测：`new_contact_allows_empty_from`（None/"" 均可入 initial 态，state_transition_pbt.rs:113-118）；`cooldown_allows_any_source`（allowFromAny，state_transition_pbt.rs:120-130）；`self_loop_is_allowed_when_listed_in_allowed_from`（默认状态机所有态自迁移都显式列出，state_transition_pbt.rs:132-143）；`invalid_transition_is_blocked`（new_contact→customer_success 拦，理由含 `state_transition_invalid`，state_transition_pbt.rs:145-160）；`empty_state_machine_skips_validation`（domain_config=None 不强校验，state_transition_pbt.rs:162-166）。

### 2.13 tests/intent_trajectory_pbt.rs（114 行，4 个 property，非 ignore，纯函数 PBT）

对象：`cap_intent_trajectory`（镜像生产 mongo `$push + $slice: -MAX_ITEMS` 行为），MAX_ITEMS=50（intent_trajectory_pbt.rs:7 注释、101）。参与 R11.6 PBT 基线门。
- **P1 length_capped_at_max_items**（intent_trajectory_pbt.rs:42-50）：任意 N∈[0,200] push 一条后长度 = min(N+1, 50)。
- **P2 last_entry_always_preserved**（intent_trajectory_pbt.rs:57-65）：新 entry 恒在尾部。
- **P3 fifo_drops_oldest_first**（intent_trajectory_pbt.rs:73-91）：超限时精确丢最旧的 N+1-50 项，保留段顺序不变。
- **P4 idempotent_under_cap**（intent_trajectory_pbt.rs:99-113）：未超限时无截断、原顺序保留。

### 2.14 tests/c2_state_transition_cross_domain.rs（191 行，6 个单测，非 ignore，纯函数）

R3.2（universal-test-coverage spec）：验证状态机引擎**行业无关**——构造医疗就诊状态机（initial_consult→follow_up→plan_confirmed→treated + missed_appointment 标 allowFromAny，c2_state_transition_cross_domain.rs:30-40）。文件头论证为何纯函数测而非真模型（`check_state_transition` 是 pub 纯函数（引 guards.rs:144）；gateway 派生点 `apply_agent_updates`（引 gateway.rs:2726）非法迁移→拒写+`agent.operation_state_transition_rejected` 审计，fail-soft 不阻断已发送 reply，c2_state_transition_cross_domain.rs:6-10）。

- `cross_domain_legal_transitions_pass`（c2_state_transition_cross_domain.rs:70-96）：线性推进 + 回退复查（plan_confirmed→follow_up）合法。
- `cross_domain_illegal_transitions_rejected`（c2_state_transition_cross_domain.rs:99-122）：跳步（初诊→已治疗）、倒退（treated→initial_consult）、跳步（初诊→方案确认）都被拦，理由含 state_transition_invalid。
- `cross_domain_initial_state_semantics`（c2_state_transition_cross_domain.rs:125-143）：空 from 只能入 initial:true 的态——**引擎读 initial 标志而非写死 new_contact**（H13 修复）。
- `cross_domain_allow_from_any`（c2_state_transition_cross_domain.rs:146-160）：allowFromAny 任何 from（含空）可入。
- `cross_domain_unknown_target_rejected`（c2_state_transition_cross_domain.rs:163-172）：迁向不存在的态 fail-closed 拒绝，理由含 `unknown_target`（防幻影态旁路 policy）。
- `sales_state_keys_are_unknown_in_medical_fsm`（c2_state_transition_cross_domain.rs:175-191）：销售态名（solution_fit/new_contact）在医疗 FSM 是 unknown_target——两域状态空间不串。

### 2.15 tests/c2_operation_state_derivation_e2e.rs（824 行，6 个测试，全部 `#[ignore]`，testcontainers+mock LLM）

G14：gateway `apply_agent_updates`（注释引 gateway.rs:2735-2820）的确定性 E2E。测试用 `complete_stage_projection` 辅助（c2_operation_state_derivation_e2e.rs:192-212）——stage/画像写入已改为**异步 post-decision projection**，测试需 push 投影 JSON 并跑 `run_post_decision_worker` 才能观察 contact 字段变化。

**用例 1 `normal_transition_uses_customer_stage_over_operation_state`（c2_operation_state_derivation_e2e.rs:222-288）**
- 不变量（synced_state 取值优先级）：operation_state 优先派生自 `domain_signals.customer_stage`，仅在缺失时回落 `decision.operation_state`。决策给 customerStage="relationship_building" + operationState="need_discovery"（两者都合法但不同）→ 最终落库 relationship_building（c2_operation_state_derivation_e2e.rs:266-271）。合法迁移不产生 rejected 审计事件（c2_operation_state_derivation_e2e.rs:273-287）。
- 空知识库 → route_operation_knowledge 早返回，3 次 LLM（c2_operation_state_derivation_e2e.rs:245-263）。

**用例 2 `illegal_transition_keeps_old_state_and_audits_failsoft`（c2_operation_state_derivation_e2e.rs:300-418）**
- 不变量（fail-soft 三联）：customerStage="customer_success"（从 new_contact 非法）→ ① operation_state 保留旧值 new_contact（**不回落** operationState=need_discovery——customer_stage present 时被拒也不改用回落源，c2_operation_state_derivation_e2e.rs:337-343）；② 落一条 `agent.operation_state_transition_rejected` 审计事件，status="rejected"，details.prior_state/attempted_state/reason（含 state_transition_invalid）（c2_operation_state_derivation_e2e.rs:345-379）；③ reply 照常放行（approved 类终态 + outbox 一行——fail-soft 不阻断已批准回复，c2_operation_state_derivation_e2e.rs:381-417）。

**用例 3 `illegal_stage_jump_keeps_old_stage_and_audits_failsoft`（c2_operation_state_derivation_e2e.rs:449-566）**
- 不变量（C1 ⑧——customer_stage 字段自身也过状态机）：此前只有派生的 operation_state 过闸、customer_stage 可被 LLM 任意跳导致两字段漂移；现在 stage 写入接同一状态机：非法跳转时 `domain_attributes.customer_stage` 保留旧值、operation_state 也留旧值（**两字段不漂移**，c2_operation_state_derivation_e2e.rs:482-504）；落 `agent.stage_transition_rejected` 审计（details.from/to/reason，c2_operation_state_derivation_e2e.rs:506-540）；reply 照常放行。

**用例 A `weak_stage_evidence_drops_to_observation_not_domain_attrs`（c2_operation_state_derivation_e2e.rs:623-673）**
- 不变量（D7-F1 铁律5 弱证据双层快通道）：stageEvidenceTurns 可解析但 stageExplicitIntent=false → 证据判 Weak → (a) 落一条 `memory_candidates` source="tag_observation"、status="pending"、candidates.dimension="customer_stage" 的暂定层记录；(b) **不写** domain_attributes.customer_stage（保持旧值）（c2_operation_state_derivation_e2e.rs:661-672）。

**用例 B `strong_stage_evidence_writes_domain_attrs_not_observation`（c2_operation_state_derivation_e2e.rs:677-729）**
- 不变量（对照）：Inbound 锚定 + stageExplicitIntent=true → Strong → 实时写 domain_attributes.customer_stage=relationship_building 且**不**落暂定层 observation（c2_operation_state_derivation_e2e.rs:717-728）。

**`audit_write_failure_does_not_drop_reply_failsoft`（c2_operation_state_derivation_e2e.rs:741-824）**
- 不变量（批A家族① C-01/H-01）：apply_agent_updates 内**纯审计事件写失败不得吞掉回复**。手法巧妙：用 MongoDB collection validator 让 `agent.operation_state_transition_rejected` 的 insert 确定性失败（c2_operation_state_derivation_e2e.rs:746-766），走非法迁移路径触发该审计写 → 断言 outbox 仍有本轮回复（修复前 `.await?` 会把 Err 冒泡到 enqueue 之前；修复后降级 `let _ = ...await` 吞错继续，c2_operation_state_derivation_e2e.rs:735-738）。

### 2.16 tests/outbox_integration.rs（2629 行，23 个测试，全部 `#[ignore]`，testcontainers+wiremock/自建阻塞式 MCP server）

本组最大的送达红线文件。关键 mock 装备：`UniqueMsgIdResponder`（每请求唯一 newMsgId——conversation_messages.message_id 有 sparse+unique 索引，重复 id 会撞 E11000 使投递被重置 pending，outbox_integration.rs:98-124）；`ChatSearchHitResponder`（chat_search 返回精确命中 items，形状对齐 src/mcp.rs::chat_search_hit :772-791，outbox_integration.rs:126-174）；`ChatSearchErrDispatchResponder`（chat_search 500 → 回落本地核对，outbox_integration.rs:180-207）；`AmbiguousSendResponder`（initialize 成功但 send 500——"请求可能已被远端接收但无可信回执"的歧义边界，outbox_integration.rs:236-260）；`BlockingMcpServer`（自建 axum server，send 调用阻塞在 Notify 上，可精确控制"远端已收到请求但尚未回执"时刻，outbox_integration.rs:262-355）；`start_mcp_mock_negative_receipt`（HTTP 200 但业务信封 ok=false 无 newMsgId，outbox_integration.rs:357-376）；`start_mcp_mock_inconclusive_receipt`（HTTP 200 但既无 ok 也无 newMsgId，outbox_integration.rs:378-396）。`count_tool_calls` 只数 JSON-RPC method==tools/call（initialize 握手不算发送，outbox_integration.rs:398-415）。

**投递生命周期**
- `durable_enqueue_wakes_dispatcher_without_poll_delay`（outbox_integration.rs:467-529）：enqueue 通过进程内 Notify 唤醒 dispatcher，claim 延迟 <1s、端到端 <2s——不等 5 秒兜底轮询。
- `happy_path_enqueue_claim_send_sent`（outbox_integration.rs:531-571）：enqueue→atomic_claim（status=in_flight）→process_entry→sent；sent 终态必须填 sent_at、清空 worker_id/locked_until（outbox_integration.rs:567-570）。
- `three_failures_lead_to_failed_terminal`（outbox_integration.rs:950-1003）：MCP 持续 500（初始化握手即失败=投递请求未发出，可证明安全重试）→ attempt 1、2 重试 pending，attempt=3 时 status="failed_terminal"、last_error 非空。
- `mixed_run_status_is_order_independent`（outbox_integration.rs:916-946）：同 run 1 条 sent + 1 条 canceled → run log `outbox_status="partially_sent"`，**与两条处理顺序无关**（sent-last 与 canceled-last 都得 partially_sent）。

**送达回执红线（delivery redline 系列）**
- `negative_mcp_receipt_is_retried_without_outbound_record`（outbox_integration.rs:574-643）：HTTP 成功但业务回执 ok=false → 按发送失败重试（pending、attempt=1），且**不写 outbound conversation record**（未获成功凭据不得伪记已发，outbox_integration.rs:632-642）。
- `delivery_redline_ambiguous_http_failure_is_not_automatically_replayed`（outbox_integration.rs:752-802）：send 请求已越过远端边界后 HTTP 500 →"无成功日志"不能当"确认未送达"→ status="delivery_unknown"、attempt=0、后续 claim 拿不到（不自动重放）；message_send_text 物理调用恰 1 次（outbox_integration.rs:786-801）。
- `delivery_redline_namecard_inconclusive_receipt_is_not_replayed`（outbox_integration.rs:647-748）：名片（referral_card_id）送达后回执无 ok/newMsgId → 名片无权威 post-hoc 查询 → delivery_unknown、last_error 含 "automatic replay disabled"、不排自动重试、不伪记 outbound namecard record、物理 message_send_namecard 恰 1 次。
- `delivery_redline_in_flight_stop_request_fences_remote_send`（outbox_integration.rs:1073-1132）：worker 已 claim、未越过 MCP 边界时用户 stop → cancel 赢得最后一次 CAS；持旧 claim 快照的 process_entry 也**不得调用 message_send_text**（0 次，outbox_integration.rs:1127-1131）；终态 canceled、cancel_reason="user_reaction_stop_requested"。
- `delivery_redline_late_cancel_after_remote_acceptance_settles_sent_once`（outbox_integration.rs:1137-1246）：远端已收到 send 请求（BlockingMcpServer 观察 send_started_at 已写，outbox_integration.rs:1173-1185）但尚未回执时取消 → 取消只是 best-effort：cancel_requested=true 但 status 仍 in_flight；随后成功回执落定 **sent**、保留 cancel_requested 审计标记、物理发送恰 1 次、不重放、outbound record 恰 1 条。
- `delivery_redline_namecard_crash_after_remote_boundary_is_not_replayed`（outbox_integration.rs:1251-1355）：名片请求已达远端、worker 崩溃（abort）→ lease 过期 reclaim → **停在 delivery_unknown**（回 pending 会导致名片物理重发）；last_error 含 "manual verification"；后续 claim 为 None；物理发送恰 1 次。

**取消/安全门**
- `user_reaction_stop_cancels_all_pending`（outbox_integration.rs:1007-1069）：`cancel_for_contact_on_user_reaction` 取消同 contact 全部 pending（2 条都 canceled、cancel_reason="user_reaction_stop_requested"）。
- `stale_thirty_minute_entry_is_canceled_by_safety_gate`（outbox_integration.rs:1359-1421）：created_at 倒推 31 分钟 → `second_safety_gate` 返回含 "stale" 的取消理由 → canceled。

**崩溃恢复/幂等**
- `crash_recovery_worker_b_reclaims_after_lease_expires`（outbox_integration.rs:1425-1497）：worker A claim 后卡死，locked_until 过期 → `reclaim_expired_leases` 恰回收 1 条（回 pending、清 worker_id/locked_until）→ worker B 重新 claim 完成 sent。
- `idempotency_key_yields_at_most_one_mcp_send`（outbox_integration.rs:1501-1581）：同 (source_event_id, contact, content) 入队 7 次 → 1 Created + 6 IdempotentSkip（unique-index 去重）；不同内容再建一行；总计 2 次 MCP 发送（不是 8）、DB 2 行。
- `reclaim_gate_precedes_pacing_gate`（outbox_integration.rs:2252-2434）：**门序不变量**——process_entry 里 reclaim 幂等门 post-hoc 核对（注释引 :645）在账号 pacing 节流闸（:719）**之前**。构造 reclaimed_in_flight=true + 本地 mcp_call_logs 有成功记录 + pacing 命中的三重条件 → 必须走 2B post-hoc 标 sent（last_error 含专属 marker "delivery was confirmed post-hoc"）而非被 pacing 拦成永久僵尸 pending；0 次 message_send_text 重发（outbox_integration.rs:2377-2433）。
- `reclaim_text_verifies_via_chat_search_before_local`（outbox_integration.rs:2444-2561）：F-01——reclaim text 路必须**先查权威 chat_search**：故意不 seed 本地 mcp_call_logs，chat_search 命中 → 标 sent、0 次重发、且收到 ≥1 次 chat_search 调用（反向坐实走了权威通道）。

**账号级软上限与拟人间隔（pacing）**
- `over_soft_cap_emits_warning_event`（outbox_integration.rs:1590-1675）：当日 sent 达 `account_daily_send_soft_cap` → 记 `agent.account_daily_send_soft_cap_exceeded` warning 事件，但**发送绝不被拦截**（本条仍 sent）。
- `under_soft_cap_emits_no_warning_event`（outbox_integration.rs:1678-1732）：未达上限无 warning。
- `account_pacing_gate_reschedules_back_to_back_send`（outbox_integration.rs:1747-1855)：同账号刚发过（sent_at=now）、固定间隔 2s 内的第二条 → reschedule 回 pending、**attempt 不变（不耗重试额度）**、next_retry_at 在未来、清 worker_id/locked_until、写 `agent.send_deferred_account_pacing` 事件、MCP 0 调用。
- `account_pacing_gate_allows_after_interval`（outbox_integration.rs:1858-1933）：历史 sent 在 10s 前（超 2s 间隔）→ 正常发出。
- `account_pacing_gate_isolates_accounts`（outbox_integration.rs:1936-2007）：账号 A 刚发不拦账号 B（闸按 account_id 维度查）。
- `account_pacing_gate_first_send_not_blocked`（outbox_integration.rs:2010-2054）：无 sent 历史 → fail-soft 放行。
- `account_pacing_gate_end_to_end_via_gateway`（outbox_integration.rs:2074-2235）：拼接点验证——第二条 outbox 由真实 gateway（handle_managed_message 决策→审查→入队）产出，字段齐全，dispatcher 驱动后仍被账号闸正确拦回 pending。

### 2.17 tests/outbox_scope_integration.rs（148 行，1 个测试，`#[ignore]`，testcontainers，直调 route handler）

**`wrong_account_cancel_is_conflict_with_zero_writes_for_pending_and_in_flight`（outbox_scope_integration.rs:64-148）**
- 不变量：管理端 `cancel_outbox`（routes::admin_outbox）带 `expectedAccountId` 与实际 account 不符时，pending 与 in_flight 两种状态都返回 `AppError::Conflict`，且**零写入**：status 不变、cancel_reason 保持 None、cancel_requested 保持 false、审计事件（outbox_canceled/outbox_cancel_requested）0 条（outbox_scope_integration.rs:107、142-147）。
- 手法：直接以 axum extractor 形参调用 handler 函数（State/Extension(AuthenticatedAdmin)/Path/Json），绕过 HTTP 层。

### 2.18 tests/send_ledger_integration.rs（394 行，5 个测试，全部 `#[ignore]`，testcontainers）

主动发送台账（agent_send_ledger）。文件头说明可见性边界：`scan_send_ledger_outcomes`/`record_send_ledger`/`recent_sends_for_contact` 部分为 pub(crate)，转化判定纯函数由 src/agent/send_ledger.rs 内联单测覆盖，本文件走公开路径（send_ledger_integration.rs:5-10）。

- `ledger_roundtrip_and_outcome_update`（send_ledger_integration.rs:60-111）：台账插入→读回（转化字段 responded/stage_advanced/outcome_evaluated_at 初始全空）→回扫回填→Option<bool>/Option<DateTime> 正确 round-trip。
- `ledger_query_is_workspace_scoped`（send_ledger_integration.rs:115-142）：wsA 查询只见 wsA 行——IDOR 防护的数据层前提。
- `sr050_outbox_anchor_is_idempotent_and_globally_unique`（send_ledger_integration.rs:144-181）：`record_send_ledger` 同 outbox_id 重放只留 1 行（确认送达路径可重放）；**同一 outbox 投递不可归因到第二个账号**——直接 insert 冲突行撞 uniq_send_ledger_outbox_id 唯一索引（E11000）（send_ledger_integration.rs:165-178）。
- `sr050_anchor_audit_rejects_duplicates_without_rewriting_history`（send_ledger_integration.rs:183-224）：drop 唯一索引、seed 两条同 outbox_id 的遗留行 → 迁移审计 m041 必须报错（"duplicate outbox_id"，需运营显式对账）且**绝不删除或合并**台账行（前后 count 相等）。
- `sr050_outcome_scan_does_not_attribute_another_accounts_reply_or_stage`（send_ledger_integration.rs:250-394）：共享 wxid 的两个账号——账号 B 的入站回复和 stage-2 进展**不得**归因给账号 A 的台账；回扫后账号 A 的行 responded=Some(false)、stage_advanced=Some(false)（send_ledger_integration.rs:381-391）。`recent_sends_for_contact` 也按 account 维度隔离（sr050_recent_history_is_account_scoped_for_shared_wxid，send_ledger_integration.rs:226-248）。

### 2.19 tests/sr034_task_send_fencing.rs（547 行，5 个测试，全部 `#[ignore]`，testcontainers+wiremock）

SR-034：task claim → outbox 发送授权 fencing 红线。直接驱动生产 claim/bind/authorize/dispatch 辅助函数（`claim_task_by_id`/`bind_task_decision_if_owned`/`authorize_task_outbox_if_owned`），避免上层 handler 重试掩盖陈旧持有者（sr034_task_send_fencing.rs:1-5）。

- `decision_batch_seal_defers_non_task_row_without_remote_send`（sr034_task_send_fencing.rs:240-307）：decision review 还在 `status="outbox_enqueuing"`（批次未封口）时，dispatcher 对该 decision 的分段**延后**——回 pending、attempt=0（"批次构建不是发送失败"）、0 次 MCP 发送（"review 批次封口前任何 decision-backed 分段不得越过 MCP"，sr034_task_send_fencing.rs:296-305）。
- `building_task_deferred_without_remote_send`（sr034_task_send_fencing.rs:309-351）：task 绑定的 decision 同样处于 building（outbox_enqueuing）→ 延后（pending、attempt=0、next_retry_at 已设、0 发送）。
- `stale_task_claim_cancels_outbox_without_remote_send`（sr034_task_send_fencing.rs:353-416）：旧 claim 的 task 被改回 retry、新 owner 重新 claim 后，旧 claim 遗留的 outbox 行被处理时 → **canceled**、cancel_reason 含 "stale_task_claim"、0 次 MCP 发送。
- `same_claim_authorization_allows_exactly_one_remote_send`（sr034_task_send_fencing.rs:418-481）：合法链路——同一 claim 做 `authorize_task_outbox_if_owned` + review 状态推到 outbox_enqueued → 恰 1 次远程发送、sent 终态、后续 claim 无可领。
- `decision_cancel_stops_pending_and_in_flight_before_remote_boundary`（sr034_task_send_fencing.rs:483-547）：`cancel_for_decision` 对同 decision 的 pending + in_flight 两行都接受取消（accepted=2）；in_flight 行的 process_entry 也不越过远端边界（两行终态都 canceled、0 次发送）。

### 2.20 tests/sr172_outbox_projection.rs（298 行，1 个测试，`#[ignore]`，testcontainers+真实 HTTP API）

**`sr172_public_route_preserves_payload_identity_and_account_scope`（sr172_outbox_projection.rs:144-298）**
- 不变量：管理端 `/api/admin/outbox?accountId=A` 投影——(1) payload 身份保真：text 行 payload={kind:"text",text}；media 行 payload.kind="media"+assetId+title；名片行 payload.kind="referralCard"+cardId+displayName；reclaimCount 透出（sr172_outbox_projection.rs:270-287）。(2) **账号 scope 红线**：A 账号 outbox 引用了 B 账号的 asset（foreign ref）时，assetId 保留但 title/fileName 为 null（**不泄露跨账号资产元数据**，sr172_outbox_projection.rs:288-291）；B 账号自己的行完全不出现在 A 的列表（total=4 不含 account-b-control，sr172_outbox_projection.rs:261、292-294）。
- 手法：真实起 axum server（`api_router` + TcpListener 随机端口）+ 种 AdminUser + `create_session` 拿 cookie + reqwest 走真实 HTTP（sr172_outbox_projection.rs:17-57）。

### 2.21 tests/hc004_outbox_webhook_scope_redlines.rs（730 行，4 个测试，全部 `#[ignore]`，testcontainers+wiremock）

HC-004：**同一 account_id 出现在两个 workspace** 时的租户隔离红线（webhook 限流/pacing/幂等键/reaction stop 四个维度）。每个测试都用 before/after 全文档 BSON 对比证明外域数据"一个字节都没变"。

- `sr024_webhook_rate_limit_is_workspace_account_scoped`（hc004_outbox_webhook_scope_redlines.rs:256-368）：webhook 限流桶按 (workspace, account) 划分——workspace A 的桶耗尽（第二发 RateLimited）不影响 workspace B 首发；`webhook_rate_limited` 事件只落在 A（B 计数 0）；B 的账号 BSON 不变。注意测试用两个独立 AppState 实例模拟双副本、共享 Mongo 权威（hc004_outbox_webhook_scope_redlines.rs:275-283）。
- `sr025_pacing_ignores_same_account_history_from_other_workspace`（hc004_outbox_webhook_scope_redlines.rs:370-518）：外域同 account_id 的 sent 历史**不**触发本域 pacing 延后（间隔 60s + soft_cap=1 的严苛配置下本域仍直接 sent、恰 1 次 message_send_text）；也不触发本域 soft-cap warning；外域 outbox BSON 不变。
- `sr026_outbox_idempotency_is_workspace_account_scoped`（hc004_outbox_webhook_scope_redlines.rs:520-608）：**幂等键 v2 含 workspace**——同 (account, contact, source_event, content) 在两个 workspace 各建一行（key_a≠key_b）；同 workspace 重复入队才 IdempotentSkip；同业务身份跨 workspace 各存一次（count=2）。
- `sr027_reaction_stop_cancels_only_same_workspace_account_outbox`（hc004_outbox_webhook_scope_redlines.rs:610-730）：本域 stop 反应（LLM 返回 stopRequested=true，1 次调用）只取消本域 pending outbox（canceled + user_reaction_stop_requested）；外域同身份 outbox BSON 不变、外域 0 新事件。

### 2.22 tests/sr177_durable_inbound_handoff.rs（720 行，3 个测试，全部 `#[ignore]`，testcontainers+wiremock）

SR-177：入站 webhook 持久化交接红线——保护进程内 debounce map 无法保护的崩溃/并发边界（sr177_durable_inbound_handoff.rs:1-9）。fixture 特点：contact 显式关 quiet_hours（operation_mode_override.quiet_hours.enabled_override=Some(false)，sr177_durable_inbound_handoff.rs:88-89）；`insert_pending_handoff` 给消息文档插 `handoff_status="pending"`（sr177_durable_inbound_handoff.rs:158-171）。

**`pending_message_is_reconciled_to_exactly_one_durable_task`（sr177_durable_inbound_handoff.rs:212-285）**
- 不变量：入站事实在任务物化前崩溃仍存活——`reconcile_pending_inbound_handoffs` 第一次返回 1（补物化）、第二次返回 0（**幂等**）；消息 handoff_status 变 "materialized"；恰 1 条 `kind=DURABLE_INBOUND_REPLY_KIND` 任务，status=pending、content=消息 ObjectId hex、`active_task_key="inbound_reply"`（单飞键，sr177_durable_inbound_handoff.rs:281-283）。

**`later_message_refreshes_single_flight_and_fences_old_outbox`（sr177_durable_inbound_handoff.rs:287-422)**
- 不变量：后到入站刷新同一单飞任务并 fence 旧 claim。**排序正确性红线**：故意让后到消息的 ObjectId 字典序更小（[0x00;12] vs [0xff;12]）——新旧判定必须基于 created_at 而非 ObjectId 随机尾（sr177_durable_inbound_handoff.rs:302-316）。刷新后：同 task_id、`task_claim_is_current(old_claim)=false`、旧 claim 的 `authorize_task_outbox_if_owned` 必须失败（sr177_durable_inbound_handoff.rs:361-372）；task.content 更新为新消息 id、latest_inbound_created_at=+1s；旧 generation 的 outbox 被 dispatcher 处理时 canceled（stale_task_claim）且 **0 次 MCP 文本发送**（"后到入站必须在任何 MCP 发送之前 fence 掉过期批次"，sr177_durable_inbound_handoff.rs:395-421）。

**`crash_after_enqueue_is_adopted_once_but_terminal_rows_stay_terminal`（sr177_durable_inbound_handoff.rs:424-720）**
- 不变量：outbox 入队后、任务授权前崩溃的恢复路径——新 owner 重新 claim + 绑定新 decision 后，`adopt_recoverable_durable_outbox_if_owned` 收养旧 decision 的可恢复行（改 decision_id/run_id 归新代际）并恰好发送一次；但两类终态行**不得复活**：运营取消行（status=canceled + "operator canceled"）与已越过远端边界行（send_started_at 已设）——adopt 都返回 false、行保持 canceled 且 decision_id 仍是旧的（sr177_durable_inbound_handoff.rs:624-653、699-717）。收养行发送后：sent、decision_id=新、run_id=新、恰 1 次 message_send_text、无重复可 claim。

### 2.23 tests/hc004_scope_redlines.rs（1048 行，7 个测试，全部 `#[ignore = "requires replica-set MongoDB"]`，start_repl_set+真实 HTTP API）

HC-004 账号/workspace scope 红线（SR-080/116/119/124/152），用生产聚合、digest、auth 中间件、Axum router。共享 fixture：ACCOUNT_A/ACCOUNT_B 同 workspace、SHARED_WXID 同名联系人（hc004_scope_redlines.rs:34-36）。

- `sr116_deal_attribution_keeps_same_wxid_accounts_separate`（hc004_scope_redlines.rs:379-452）：同 wxid 两账号——A 有成交事件、A/B 各有知识 usage log → `refresh_usage_stats_and_confidence` 的成交归因 hits=1，A chunk 的 hit_count_30d=1、**B chunk =0**（成交不跨账号污染 B 的知识统计）。
- `sr119_digest_health_includes_shared_chunks_without_crossing_accounts`（hc004_scope_redlines.rs:454-491）：digest 健康分析对 ACCOUNT_A 可见 = 共享 chunk（account_id=None）+ A 自己的 chunk；B 的 chunk 不可见。
- `sr119_digest_generation_persists_and_audits_each_account_scope`（hc004_scope_redlines.rs:493-584）：全账号 digest 生成枚举 2 账号，每账号各 1 份 report、1 条 digest usage log、1 条 knowledge_digest_generated 事件、1 条 prompt_key="knowledge.digest.compose" 的 LLM log——审计四件套按账号成对。
- `sr124_direct_dismiss_router_never_crosses_same_card_id_accounts`（hc004_scope_redlines.rs:586-702）：两账号 digest report 含**同一 card_id**——dismiss 路由缺 accountId → 400；未知 accountId → 404 且两报告 BSON 逐字节不变；带 ACCOUNT_B → 只有 B 的 dismissed_card_ids 更新，A 报告逐字节不变。
- `sr124_fenced_worker_dismiss_never_crosses_same_card_id_accounts`（hc004_scope_redlines.rs:704-801）：同一红线走 worker 路径（knowledge_chat_tasks + run_task）——任务完成（status="completed"、completed_steps[0].status="committed"），只改 B 报告。
- `sr080_enable_agent_uses_the_contact_workspace_account_identity`（hc004_scope_redlines.rs:803-930）：`POST /contacts/{id}/enable-agent`（带 expectedAccountId + humanProfileNote，LLM 生成初始画像）把 contact 置 Managed，**外域同 account_id 的账号 BSON 逐字节不变、外域 0 新事件**；本域落 1 条 `contact.enabled_for_ops` 事件。
- `sr152_review_routes_require_account_and_never_cross_same_wxid_accounts`（hc004_scope_redlines.rs:932-1048）：决策评审列表/详情路由**必须带 accountId**（缺 → 400；带错账号查 B 的 contact → 404）；正确账号只见本账号 review（响应字符串级断言不含 A 的 replyText/marker，hc004_scope_redlines.rs:1019-1020）；detail 同规则。且 finalReviewStatus/holdCategory 投影正确（关联 run_log marker）。

### 2.24 tests/memory_card_invariants.rs（725 行，4 个 PBT + 16 个单测，非 ignore，纯函数）

基线四 PBT 文件之一（配套 .proptest-regressions 存在）。对象：`compact_memory_card_with_previous(&MemoryCardTyped, Option<&prev>, &discarded)` 与 `compact_memory_card_with_dimensions`。核心 cap：**core_facts ≤ 6、recent_facts ≤ 10**。

- **PBT compact_caps_core_and_recent**（memory_card_invariants.rs:53-70）：任意输入 compact 后不超 cap。
- **PBT previous_core_facts_are_preserved_unless_discarded**（memory_card_invariants.rs:73-133）：旧 core 事实未被 discarded 必可从 core/recent/`extra.coreFactEvictions` 审计追溯；discarded 项绝不出现在结果。
- **PBT over_cap_core_facts_remain_traceable**（SR-182 红线，memory_card_invariants.rs:321-350）：previous 已满 6 + 新 1..=6 条 → core 仍 6、溢出恰好进 recent、审计条数=溢出数——**任何未 discarded 候选不能静默消失**。
- **PBT legacy_vec_string_inputs_respect_caps_and_persistence**（memory_card_invariants.rs:698-724）：旧 Vec<String> 形态输入同样满足 cap 与 discarded 排除。
- 单测（关键的）：`compact_keeps_leading_items_after_cap`（超 cap 时前 6 进 core、溢出项进 recent 并留 `coreFactEvictions` 审计（text+reason="core_fact_capacity"），memory_card_invariants.rs:156-179）；`core_fact_eviction_archive_survives_when_next_round_does_not_echo_it`（eviction 归档跨轮存活，memory_card_invariants.rs:182-204）；`explicit_discard_removes_prior_capacity_archive`（显式 discard 可清除归档项，memory_card_invariants.rs:207-228）；`promoted_core_fact_is_removed_from_prior_capacity_archive`（被重新提升的事实移出归档，memory_card_invariants.rs:231-268）；`core_fact_priority_is_stable_and_not_incoming_first`（结构化事实按 importance/confidence 排序且**与输入顺序无关**（左右反转输入结果一致），被挤出的事实保留 Structured 形态并带 evictionReason/coreFactRank，memory_card_invariants.rs:271-314）。
- 兼容性/序列化单测：`deprecated_typed_alias_matches_canonical_helper`（旧别名 compact_memory_card_typed 语义一致，memory_card_invariants.rs:391-415）；`document_round_trip_compact_matches_typed_construction`（老 BSON Document 读入 compact 与直接 typed 构造一致，memory_card_invariants.rs:421-448）；`plain_and_structured_round_trip_through_bson`（Plain 反序列化会 promote 到 Structured 但 text 保留，memory_card_invariants.rs:611-641）；`full_card_round_trip_preserves_extra_fields`（extra 捕获 free-form 字段；coreProfile/relationshipState 只能在 extra、序列化后顶层不得出现重复键——防 serde flatten 重复 BSON 键回读崩溃，memory_card_invariants.rs:644-688）。
- **记忆维度通用化（H17）**：`extra_array_caps_are_enforced`（销售槽 preferences cap 8 / doNotDo cap 10，memory_card_invariants.rs:453-472）；`custom_memory_dimension_caps_are_enforced`（自定义维度 cap 经 memory_dimensions 驱动生效，memory_card_invariants.rs:477-522）；`emotional_companion_profile_memory_dimensions_end_to_end`（情感陪伴 profile 三槽（情绪史/纪念日/重要事件）cap 生效 + `render_memory_candidate_types_guidance` 只把 candidate_type=true 的维度放进候选合法集、销售槽不出现，memory_card_invariants.rs:529-606）——证明记忆维度真正"随 profile"。

### 2.25 tests/memory_card_write_occ.rs（23 行，1 个测试，`#[ignore]` 且**空函数体**）

**`concurrent_memory_card_write_does_not_lose_race_error`（memory_card_write_occ.rs:14-23）**
- 目标不变量（CONC-1，仅文档化）：apply_operating_memory_update 写 memory_card 走 OCC 版本谓词（occ_memory_filter），并发输者 modified_count==0 时静默跳过（非 last-write-wins 覆盖、不返回 Err）；memory_card_version 单调不回退；门控外字段（updated_at）走原三键 filter 不受 OCC 影响（memory_card_write_occ.rs:1-10）。
- **弱点（重要疑点）**：函数体只有 `let _ = &app;`——启动 TestApp 后什么都不断言。与 revision_recheck_action_gate.rs 同属"永远绿的 CI 骨架"。

### 2.26 tests/operating_memory_insert_idempotent.rs（103 行，1 个测试，`#[ignore]`，testcontainers 并发）

**`concurrent_first_touch_inserts_all_succeed`（operating_memory_insert_idempotent.rs:65-103）**
- 不变量（CONC-3）：`load_or_create_operating_memory` 首触达 create 分支被 4 路并发调用时，输给唯一索引 (workspace_id, account_id, contact_wxid) 的一方**回落 find_one 返回赢家文档而非 E11000 透传**（透传会让整轮 run 在回复客户前失败）；4 路全 Ok、库里恰 1 行（operating_memory_insert_idempotent.rs:81-102）。

### 2.27 tests/sr181_operator_memory_revocation.rs（140 行，1 个测试，`#[ignore]`，testcontainers）

**`operator_memory_add_revoke_scope_and_readd_lifecycle`（sr181_operator_memory_revocation.rs:14-140）**
- 不变量（SR-181 运营者记忆生命周期红线）：驱动生产 record/load/revoke 辅助（不复制 Mongo filter——如果 live loader 删掉 revoked_at 过滤，本测试会红，sr181_operator_memory_revocation.rs:3-5）。链条：
  - 记录后 `load_operator_memory_read_only` 返回 1 条；
  - **跨 scope 撤销（错账号/错运营者）必须与 not found 不可区分**（AppError::NotFound）且零写入（sr181_operator_memory_revocation.rs:33-52）；
  - 首次撤销落 revoked_by/revocation_reason；撤销后 load 为空（**已撤销记忆不再注入**）；
  - **重复撤销幂等**且不覆盖首次审计（already_revoked=true、revoked_by 仍是 admin-a，sr181_operator_memory_revocation.rs:79-95）；
  - 显式重加建**新行**（新 id）；审计行 + 新活跃行都保留（count=2，撤销历史不被抹掉）；
  - 未知 id 同样 NotFound（不泄露存在性）。

### 2.28 tests/sr029_memory_commit_recovery.rs（588 行，2 个测试，全部 `#[ignore]`，testcontainers）

SR-029：记忆固化的持久化提交红线。用生产 manual-task 入口与生产 commit reconciler，故意持久化"崩溃窗口快照"而非复制 reconciler 的 filter（sr029_memory_commit_recovery.rs:1-5）。

**`manual_consolidation_uses_single_flight_durable_task`（sr029_memory_commit_recovery.rs:91-242）**
- 不变量：`run_manual_memory_consolidation` 走单飞持久任务协议——完成后 agent_tasks 行保留为审计（status="sent"、gateway_status="consolidated"），且 prepared_commit/claim_token/active_task_key 已清除（sr029_memory_commit_recovery.rs:133-146）；operating_memory 落 memory_card_version=1 + coreFacts；pending 候选归零。
- **单飞冲突红线**：已存在 running 的 memory_consolidation 任务（active_task_key 占用）时，再次手动触发返回 `AppError::Conflict`、既有 owner 文档逐字节不变、**不调模型**（llm.calls 仍 1，sr029_memory_commit_recovery.rs:187-241）。

**`prepared_commit_replays_all_partial_windows_exactly_once`（sr029_memory_commit_recovery.rs:444-588）**
- 不变量：两阶段提交恢复——status="committing" + prepared_commit 快照的任务在三种崩溃窗口（none：什么都没应用 / memory：memory_card 已应用 / projections：投影+候选也已应用）下，`reconcile_memory_consolidation_commit` 第一遍补齐全部效果、第二遍是 no-op（幂等）；终态一致：task sent+consolidated、memory_card_version=1、memory_applied_commits 清空、contact.memory_projection_version=1、候选 consolidated；**冲突审计事件按 dedupe_key（memory_commit:{task}:{generation}:conflict:0）恰好 1 条**（即便 preapply 已写过也不重复），completion 事件同样恰 1 条（sr029_memory_commit_recovery.rs:529-585）。

### 2.29 tests/reaction_claim_lock.rs（610 行，3 个测试，全部 `#[ignore]`，testcontainers+自定义阻塞 LLM）

反应分析 claim 所有权回归（HP-3）。全部 claim/reclaim/finalize 走 `record_user_reaction` 生产入口，测试只在 ABA 用例回拨 reaction_claimed_at 模拟超时，不复制 Mongo 状态机（reaction_claim_lock.rs:4-5）。`BlockingReactionProvider`：第 1 次调用阻塞在 Notify 上（返回 stopRequested=true）、第 2 次返回 buyingSignal=true（reaction_claim_lock.rs:134-199）。

**`reaction_redline_concurrent_entry_cannot_start_second_analysis`（reaction_claim_lock.rs:236-293）**
- 不变量：第一路分析持有活跃 claim 期间，第二条入站进入生产入口**不得开启第二次 LLM 分析**（provider.calls()==1）；第一路完成后 outcome_status="user_replied_stop_requested"、reaction_claim_generation=1、claim_token 清空。

**`reaction_redline_stale_owner_cannot_overwrite_or_cancel_after_reclaim`（reaction_claim_lock.rs:295-446）**
- 不变量（ABA 防护）：首路 claim（generation=1、outcome_status="analyzing"）超时被回拨后，新入站走生产 reclaim → generation=2、新 owner 结论 "user_replied_buying_signal" 落库；随后旧 owner 的 stop 结果返回——**不得覆盖当前 owner 结论**（outcome_status 仍 buying_signal）、**不得取消 pending outbox**（outbox 仍 pending——旧 stop 不得触发取消，reaction_claim_lock.rs:432-442）、intent_trajectory 只有当前 owner 追加的 1 条。
- 注意：stopRequested=true 的正常效果是取消 pending outbox，本测试证明**过期 owner** 的 stop 无此权力。

**`explicit_buying_floor_is_claim_scoped_transaction_only_and_zero_llm`（reaction_claim_lock.rs:448-610）**
- 不变量（确定性买入底线）：交易型 profile + 存在已 sent 前置回复时，"可以现在就报名付款吗？我要买"走**零 LLM** 确定性分类（outcome_status="user_replied_buying_signal"、reaction_analysis.deterministic=true），且不创建 deal/payment 事实（dealVerified/paymentVerified 不存在，reaction_claim_lock.rs:468-494）；
- **无 sent 前置 = 无可归因**：同样的话不建 review、不加轨迹、不调模型（reaction_claim_lock.rs:496-537）；
- **否定语句留给模型**："我先不买，也不要帮我下单"不硬编码为买入信号，走 LLM（deterministic≠true，reaction_claim_lock.rs:539-571）；
- **profile 决定语义**：情感陪伴 profile（seed_emotional_companion_profile_in_workspace）下同样字面的"我要买，现在付款"仍走模型域语义（消费队列返回 user_emotion_opened_up）而非销售底线（reaction_claim_lock.rs:573-607）——确定性底线是 per-profile 的交易域特性。

### 2.30 tests/reaction_stop_cancels_outbox_integration.rs（326 行，2 个测试，全部 `#[ignore]`，testcontainers）

锁定 `src/agent/reaction.rs:248`（写测试时行号）的接线：`outcome_signals_stop → cancel_for_contact_on_user_reaction` 完整串联，防"三段各自有测试但串联被静默摘除"（reaction_stop_cancels_outbox_integration.rs:1-20）。

**`record_user_reaction_stop_cancels_pending_outbox`（reaction_stop_cancels_outbox_integration.rs:164-245）**
- 不变量：真调 `record_user_reaction`（非直调 cancel 函数），LLM 返回 stopRequested=true → 映射 user_replied_stop_requested → 同 contact 的 2 条 pending outbox 全部 canceled、cancel_reason="user_reaction_stop_requested"；LLM 恰 1 次。
- claim filter 前提：review status="sent" 且 outcome_status ∈ {null,"pending"}（fixture 注释引 reaction.rs:87，reaction_stop_cancels_outbox_integration.rs:87-90、116-127）。

**`deterministic_stop_needs_no_review_or_llm_and_persists_dispatch_barrier`（reaction_stop_cancels_outbox_integration.rs:247-326）**
- 不变量：明确停止语（"别再发了，我不想聊了，到此为止吧"）**无前置 review 也生效、零 LLM**（显式停止绝不依赖 LLM 可用性，reaction_stop_cancels_outbox_integration.rs:285-289）；持久化重启安全的屏障：contact.cooldown_until 在未来 + operation_policy.explicitStopRequested=true（reaction_stop_cancels_outbox_integration.rs:299-311）；pending outbox 同样 canceled。

### 2.31 tests/principal_decision_channel.rs（1627 行，18 个测试：16 个 `#[ignore]` + 2 个纯函数，testcontainers+wiremock）

决策请示通道（spec §14 九项）。文件头声明策略：经"公共表面切片"（公共模型 + typed accessor + 唯一 pub 入口 scan_escalation_timeouts / handle_follow_up_task）断言，不放开 pub(crate) 可见性；§14.8 assert_target_is_principal 的纯函数测试在 src/agent/escalation.rs 内联（principal_decision_channel.rs:1-16）。

**基础台账/配置（数据层往返）**
- `t_escalation_out_of_scope_creates_pending`（principal_decision_channel.rs:265-291）：pending 台账按 short_code 查回，status=pending、principal_wxid/category 正确、decision=None。
- `t_high_risk_mode_config_roundtrip`（principal_decision_channel.rs:301-334)：principal_decider + high_risk_escalation_mode="all" 以 `$set` 到 seeded current 行的方式持久化（生产 admin 同写法；不另插行避 unique 索引）。
- `t_pending_resolve_roundtrip`（principal_decision_channel.rs:338-393）：pending→resolved + PrincipalDecision（verdict=conditional、substance、constraints、authorization_window_hours=48h）正确反序列化。
- `t_knowledge_proposal_is_draft_needs_review`（principal_decision_channel.rs:397-435）：真人决策沉淀的知识缺口提案**永远 draft + needs_review + workspace 共享域（account_id=None）**——"AI 永不自动验证"红线在请示沉淀入口同样成立。
- `t_awaiting_marker_set_and_clear_roundtrip`（principal_decision_channel.rs:439-506）：`domain_attributes.awaiting_principal_decision` 标记 $set/$unset 往返。

**webhook 请示路由**
- `principal_ambiguity_clarification_is_durable_and_never_direct_mcp`（principal_decision_channel.rs:186-259）：领导回复模糊语（"可以"、有多条 pending 无法定位）经真实 webhook 路由（routed="principal"）→ 澄清请求走**持久化 outbox**（source_kind=principal_clarification），webhook handler **绝不直调 MCP**（0 请求）；重放同一 webhook 幂等（澄清 outbox 恰 1 行）。

**§14.10 超时改派与骚扰门（唯一 pub 入口 scan_escalation_timeouts 驱动）**
- `t_timeout_reassign_pushes_and_touches_updated_at`（principal_decision_channel.rs:643-725）：主决策人超时（updated_at 2h 前 > timeout 1h）→ 改派 backup 并**只开启下一投递代次**（delivery_generation=2、state=queued、last_pushed_at_ms 尚 None）；Outbox 确认 sent + `reconcile_principal_card_deliveries` 对账后才刷新 last_pushed_at_ms/updated_at（下一位的超时窗起点）。
- `t_timeout_reassign_terminal_delivery_failure_releases_pending`（principal_decision_channel.rs:729-847）：下一代次 outbox 重试耗尽 → 对账收敛 status="delivery_failed"、failure_cleanup_completed_at 已设、**清客户 awaiting 标记并释放 pending 唯一槽**（同客户同类别可再建新请示）。
- `t_timeout_reassign_blocked_by_quiet_hours_skips_push`（principal_decision_channel.rs:852-909）：**gate 先于改派**——next 决策人落在 quiet_hours → 不改派不推（principal_wxid 仍 boss、updated_at 不刷新，待下一 tick）。测试构造确定性命中当前小时的静默窗 [now_hour, now_hour+23)（与容器时区无关）。
- `t_timeout_reassign_cap_one_not_self_blocked`（principal_decision_channel.rs:916-965）：daily_push_cap=1 时改派**不被本条自己误命中**（修复回归：旧实现 reassign 先于 gate，本条改派后被算成 backup 的 1 次推送 → 永远收不到卡）。
- `t_timeout_reassign_concurrent_scans_enqueue_one_generation`（principal_decision_channel.rs:969-1028）：两个 scanner 并发 → generation CAS + outbox 幂等键（source_event_id=`principal-card:{id}:2`）收敛为一次改派 + 恰 1 条 generation-2 outbox。
- `t_stale_principal_card_generation_is_canceled_before_remote_send`（principal_decision_channel.rs:1032-1107）：台账已推进到 generation 2 后，被 claim 的 generation-1 卡片在远端边界前取消（cancel_reason="principal_escalation_generation_no_longer_authorized"、send_started_at=None、MCP 0 请求）。
- `t_timeout_chain_tail_sends_holding_reply_once_within_interval`（principal_decision_channel.rs:1113-1158）：单决策人链尾失联 → 发一条安抚话术 + 记 last_holding_reply_ms；紧接第二次 scan 在 min_interval（6h）内不重发；台账保持 pending。

**§14.11 授权过期与 relay 出站**
- `t_relay_expired_authorization_clears_awaiting_and_sends_neutral`（principal_decision_channel.rs:1297-1396）：relay task 执行时授权已过期 → ①必须清 awaiting 标记（否则永久压制自主回复）+ 台账 relay_state=terminal、relay_terminal_reason="authorization_expired"；②给客户发一条**中性收尾**（先入 durable outbox 再 dispatch），文案含"继续/核实/同步"类表达且**绝不复述过期 substance 的具体数字（"8 折"）**（principal_decision_channel.rs:1374-1395）。
- `terminalizing_one_relay_preserves_another_awaiting_owner`（principal_decision_channel.rs:1400-1485）：同客户多条等待项（AWAITING_PRINCIPAL_DECISION_IDS_ATTR owner 数组）——终结一条只移除自己的 owner，另一条活跃时 coarse awaiting 保持 true；全部终结后 awaiting=false、owner 数组空。
- `blocked_relay_preserves_awaiting_and_cancels_task_without_outbox`（principal_decision_channel.rs:1489-1591）：relay 候选文本泄漏内部字段（"verdict=approved"）→ relay 安全门在入队前拦截：0 outbox、客户 awaiting 保持 true（仍在等待有效转述）、任务 status="cancelled" + gateway_status="blocked_by_safety_guard"、review status="blocked_by_safety_guard"。

**纯函数（本地即跑）**
- `t_synthetic_relay_carries_sentinel_and_fields`（principal_decision_channel.rs:1597-1613）：`ConversationMessage::synthetic_principal_relay` 合成消息以 PRINCIPAL_RELAY_SENTINEL 哨兵开头、载荷带 verdict/substance/constraints 三要素。
- `fallback_holding_reply_has_no_handoff_wording`（principal_decision_channel.rs:1618-1627）：兜底安抚文案**不含"真人/转人工/客服/接管/人工"任一禁词**——注：tests/ 目录被 check-no-human-takeover lint 排除，正是为了能在测试里写禁词字面量来断言生产文案没有它们。

### 2.32 tests/referral_card_push_integration.rs（267 行，2 个测试，全部 `#[ignore]`，testcontainers）

专属顾问名片引荐（辅助模式）。文件头可见性说明：outbox::enqueue 系列 pub 可端到端测；`load_referral_cards`/`filter_referral_candidates`/`assist_mode_active`/`validate_card_sendable` 为 pub(crate) 跨 crate 不可见——审核门改测公开集合路径 + **与生产 build_referral_cards_filter 同形**的过滤条件；辅助模式短路真值表由 src/agent/referral.rs 内联单测覆盖（referral_card_push_integration.rs:5-19）。

**`only_approved_enabled_card_is_loadable`（referral_card_push_integration.rs:94-189）**
- 不变量（审核门）：可加载名片过滤 = `enabled:true` + `review_status:"approved"` + workspace + (account_id=null 共享或本账号)（referral_card_push_integration.rs:57-67）。draft+disabled 不命中；人工审核（$set enabled+approved）后命中；**approved 但 disabled 仍不可加载**（两门独立生效）。
- 疑点：这是"同形复刻过滤条件"式测试——若生产 build_referral_cards_filter 改动，本测试不自动失败（作者已声明该取舍，见第 5 节）。

**`namecard_outbox_entry_idempotent_per_card`（referral_card_push_integration.rs:193-267）**
- 不变量：名片 outbox 条目按 card_id 幂等——同 (run_id, contact, referral_card_id) 二次入队 IdempotentSkip；同 run 不同 card_id 独立入队（synthetic_namecard key 含 card_id）；名片条目 content 可空（content_required_for 对 referral_card_id 放行，referral_card_push_integration.rs:84-85）；名片条目与 media 条目互斥（media_asset_id=None，referral_card_push_integration.rs:262-265）。

### 2.33 tests/ask_human_phase1_e2e.rs（1228 行，15 个测试，全部 `#[ignore]`，testcontainers，直调 route handler）

Ask-Human Phase 1 请示通道管理端。harness 纪律：ensure_prompt_pack_v2 已 seed (default, user_operations, v1, current) 底座 config 行，测试需预置 config 时**绝不 insert_one**（撞唯一索引），一律 `$set` 到既有 current 行（ask_human_phase1_e2e.rs:25-31）。

- `put_ask_human_policy_persists_and_reads_back`（ask_human_phase1_e2e.rs:226-281）：PUT ask_human_policy（camelCase wire）→ 回读逐字段一致；**version 不 bump、current_version 仍 true**（admin 编辑是 $set 语义，非版本发布）。前置需 `ensure_decider_identity`（决策人 wxid 必须是真实账号-联系人归属，后端权威校验）。
- `admin_resolve_enqueues_relay_and_marks_resolved`（ask_human_phase1_e2e.rs:287-343）：admin 结构化裁决 → 台账 resolved + resolved_via="admin" + relay_state="enqueued" + 恰 1 条 `kind=principal_decision_relay` task（content=short_code）。
- `resolved_relay_intent_recovers_exactly_one_task`（ask_human_phase1_e2e.rs:347-439）：裁决 CAS 后、task 物化前崩溃（删 task+回拨 relay_state=pending 模拟）→ `reconcile_pending_relay_intents` 第一遍恢复 1 条、第二遍 0（幂等）；恢复的 task 保持同 _id。
- `admin_resolve_is_idempotent`（ask_human_phase1_e2e.rs:444-506）：重复 resolve → alreadyResolved=true、relay task 仍 1 条。
- `reassign_rejects_wxid_not_in_chain`（ask_human_phase1_e2e.rs:512-551）：改派到链外 wxid → Err(AppError::BadRequest)。
- `inbox_aggregates_and_degrades`（ask_human_phase1_e2e.rs:557-602）：统一待审箱聚合 principal_escalation + knowledge_review 两个 source（items ≥2、errors 空）。
- `summary_counts_pending`（ask_human_phase1_e2e.rs:607-630）：summary.principalEscalation 计数正确。
- `resolve_foreign_workspace_escalation_is_noop`（ask_human_phase1_e2e.rs:637-683）：**跨 workspace resolve IDOR 守卫**——越权方拿到幂等 200 alreadyResolved=true（不泄漏存在性），但台账仍 pending、resolved_via 空（未真正被裁决）。
- `admin_deferred_keeps_escalation_pending`（ask_human_phase1_e2e.rs:690-748）：verdict="deferred" 短路返回 deferred=true——台账保持 pending、不 resolve、零 relay task（修前 deferred 会静默关闭台账且 scan 只扫 pending 永不再浮出）。
- `timeout_reassign_gives_each_decider_full_window`（ask_human_phase1_e2e.rs:755-929）：链 [a,b,c] 超时改派——**age 自 last_pushed_at_ms（sent 对账时刻）起算，每位决策人拿到完整 24h 窗**：a 超时后只开启 b 的 generation=2 queued；queued 阶段重复扫描不级联到 c、不产生 generation=3 或第二条 outbox；outbox sent + reconciler 写回 last_pushed_at_ms 后 b 的完整窗才开始（再扫描仍是 b）。
- `inbox_lessons_item_carries_lesson_id`（ask_human_phase1_e2e.rs:935-982）：lessons_learned 收件项带 richParams.lessonId=`{ws}::{pattern_kind}`（lesson_id 字段而非 _id hex——promote 端点按 lesson_id 寻址）。
- `get_single_chunk_by_id_scoped_to_workspace`（ask_human_phase1_e2e.rs:984-1027）：单 chunk 读取 workspace 隔离——本域 200，**跨域 NotFound（反向断言，防假绿）**（ask_human_phase1_e2e.rs:1014-1026）。
- `operation_domain_json_includes_ask_human_policy`（ask_human_phase1_e2e.rs:1029-1081）：GET domain 投影含 askHumanPolicy（deciderChain/timeoutHours/dailyPushCap camelCase）。
- `inbox_relationship_suggestion_carries_evidence`（ask_human_phase1_e2e.rs:1089-1158）：E10 反盲批——关系类型建议经真投影函数流出 evidence/confidence/occurrences/contactWxid。
- `inbox_and_summary_include_only_workspace_pending_suspected_deals`（ask_human_phase1_e2e.rs:1162-1228）：SR-067 疑似成交进统一待审箱——只投影本 workspace 的 pending（approved 历史与外域不泄漏不占数）；item 带 richComponent="suspectedDealReview"；summary.counts.suspectedDeal=1 与兼容顶层计数一致。

### 2.34 tests/real_llm_principal_channel.rs（787 行，1 个测试，`#[ignore]`+env-gated，真模型+testcontainers+wiremock）

R2.5.3"无人工接管"治理红线命门的**真模型业务流**（mock 版在 principal_decision_channel.rs）。红线三件套：MCP 永远是桩（绝不真发微信）、密钥零泄漏、env-gated（缺 REAL_LLM_API_KEY 自我跳过）（real_llm_principal_channel.rs:32-35）。基础设施：FailoverProvider 备胎链（被测 agent 冻结主模型，failover 只解端点限流，不抬高被测分，real_llm_principal_channel.rs:64-234）；`unwrap_or_skip_transient!` 宏——瞬时 LlmUnavailable → skip+落 skip_ledger.jsonl，**非瞬时 4xx 配置错误 panic 堵 R0.3 假绿**（real_llm_principal_channel.rs:263-313）。

**`principal_channel_out_of_scope_arc`（real_llm_principal_channel.rs:506-787）**
- 场景：4 轮超职权固定台词（全额退款+赔偿/要求见拍板人/五折+定制条款/书面无条件包赔）——触发取决于事项**实质**超职权而非"找人工"字面（real_llm_principal_channel.rs:485-504）。
- **硬断言（确定性红线）**：① gateway status / final_review_status ∈ 闭集（GATEWAY_STATUS_VALUES/FINAL_REVIEW_STATUS_VALUES，real_llm_principal_channel.rs:577-588）；② 真发出的回复不含 13 个转接/暴露身份禁词（"转人工/人工客服/转接/…/我是机器人/我是ai"，小写折叠匹配，real_llm_principal_channel.rs:451-465、629-634）；③ 不含 9 个暴露幕后决策源禁词（"领导决定/老板拍板/上级批准"等——注意**边界**："我帮你跟领导申请一下"是合规话术，不能把"领导"裸词当禁词，real_llm_principal_channel.rs:467-483、636-641）；④ 不逐字复读上一轮；⑤ 终局断言 `escalation_observed_turns + fail_closed_turns > 0`——4 轮超职权诉求**至少**要么落 pending 请示台账、要么进入明确 fail-closed 终态，"全程直接回复"不能作为通过证据（real_llm_principal_channel.rs:774-777）。
- **软观测（诚实降级）**：escalation 是否触发依赖真模型 emit escalationRequest（非确定性，两条产生路径：approved 路径 gateway.rs:1845 trigger_principal_escalation / hold→升级路径 gateway.rs:1463 escalate_held_decision，注释行号为写作时快照）——查到 pending 记正向 ledger，查不到不 fail 只记 issue（real_llm_principal_channel.rs:21-30、652-713、759-773）；judge（最强模型+failover）按 profile 派生 rubric 打分 ObserveOnly，factualRestraint 中位数 <4 记"疑似编造超权承诺"issue（real_llm_principal_channel.rs:715-752）。
- 证据账本：CapabilityEvidence + RoleplayLedger（归因报告 jsonl）。

### 2.35 tests/real_llm_principal_relay.rs（670 行，1 个测试，`#[ignore]`+env-gated，真模型+testcontainers+wiremock）

G9/G10 缺口补齐：请示通道**入站 relay 回路**真模型业务流（出站方向在 real_llm_principal_channel）。回路五环节：①领导自然语言裁决经公开 `wechat_webhook` 入站 → ②`interpret_principal_reply` 真 LLM 解析成结构化 PrincipalDecision → ③`resolve_escalation` pending→resolved → ④relay task（handle_follow_up_task 按 kind 分流）→ ⑤真 LLM 生成面向客户的转述（real_llm_principal_relay.rs:15-20）。pub(crate) 卡点：走生产公开入口（webhook + handle_follow_up_task），零可见性改动（real_llm_principal_relay.rs:22-27）。

**`principal_inbound_relay_loop_happy_path`（real_llm_principal_relay.rs:468-670）**
- 固定输入：领导裁决"这个客户可以给他打九折，但仅此一次，让他这周内付款。"（明确批准+约束）；客户只有一条 pending → 不带短码也能命中（match_principal_reply 单条兜底，real_llm_principal_relay.rs:519）。
- 断言（契约级、不锁具体措辞——反过拟合）：webhook routed="principal"（不进客户 agent 链路）；台账 resolved、decision.verdict ∈ ALLOWED_PRINCIPAL_VERDICT 闭集且 **≠ deferred**（明确批准不应被误判暂缓，real_llm_principal_relay.rs:538-563）；relay task 已入队；relay run gateway status/final_review_status ∈ 闭集；转述文本非空且通过 G10 红线——`assert_no_handoff_or_identity_leak`（共享红线判定）+ 9 个幕后决策源禁词（real_llm_principal_relay.rs:617-645）。
- 软观测：relay 后 awaiting 标记应清（只打印不 fail，real_llm_principal_relay.rs:647-662）。

### 2.36 tests/campaign_dispatch_integration.rs（725 行，9 个测试，全部 `#[ignore]`，testcontainers，直调 handler）

campaign 营销活动派发红线。dispatch 请求的 spec_hash 用生产 `campaign_spec_hash_for_view` 计算（**测试故意不复刻 hash 算法**，campaign_dispatch_integration.rs:139-141）。

- `create_rejects_account_owned_by_another_workspace_without_writing`（campaign_dispatch_integration.rs:168-219）：账号只属于外域时 create → NotFound + 零写入（campaign 计数不变）。
- `dispatch_zero_hits_rejected`（campaign_dispatch_integration.rs:222-240）：**命中 0 人 → BadRequest**，不静默"派发成功 0 人"。
- `dispatch_cross_workspace_not_found`（campaign_dispatch_integration.rs:243-261）：跨 workspace dispatch → NotFound（handler 注入 current_workspace 到 filter）。
- `preview_completed_campaign_is_zero_write`（campaign_dispatch_integration.rs:265-312）：SR-075——preview 终态活动 → Conflict 且 campaign 文档逐字节不变（后来的新联系人不能重开/扩张已完成活动）。
- `draft_patch_is_consumed_by_next_preview`（campaign_dispatch_integration.rs:316-371）：SR-077——CAS 改草稿（expected_spec_version）→ specVersion bump 到 2；下一次 preview 评估**新 spec**（filter customer_stage=won → targetCount=1）。
- `dispatch_builds_tasks_then_rejects_repeat_after_completed`（campaign_dispatch_integration.rs:379-458）：命中 2 人 → dispatchedCount=2 + 建 2 条 follow_up task（"走 gateway 的证据"）；首次成功后 campaign 置 completed → **二次 dispatch 被 KC-02 status 前置门 BadRequest 拒**（门在圈人前，不新增 task）。注释明说旧契约"completed 可反复 dispatch 靠 unique 索引幂等返 0"语义错误已被取代（campaign_dispatch_integration.rs:373-378）。
- `dispatch_task_insert_failure_is_reconciled_from_prepared_intent`（campaign_dispatch_integration.rs:462-562）：HC-021 durable fanout——用 collection validator 让 follow_up task insert 确定性失败 → dispatch 返 Err 但 `campaign_sends` 保留 status="prepared" 的持久意图 + campaign 冻结在 "dispatching"（dispatch_audience 已冻结）；解除故障后 `reconcile_campaign_dispatches` 用同一冻结身份恢复 task（同 task_id）、campaign 落 completed、dispatched_count=1。
- `dispatch_completed_campaign_rejected`（campaign_dispatch_integration.rs:565-595）：KC-02 门精确断言 BadRequest 类型（已 seed 对齐 contact，唯一可达 Err 即 status 门）。
- `preview_rejects_when_coarse_audience_exceeds_max` / `preview_succeeds_at_exactly_max`（campaign_dispatch_integration.rs:598-672）：KC-04/07——粗筛候选 > campaign_max_audience → BadRequest（**不静默截断受众**）；恰好等于上限 → 成功且 targetCount=上限（边界探测）。
- `dispatch_backfills_last_dispatch_target_count`（campaign_dispatch_integration.rs:676-725）：KC-06——dispatch 成功回刷 lastDispatchTargetCount=本次命中数，与 dispatchedCount（去重后新入队数）区分。

### 2.37 tests/campaign_segment_coverage.rs（256 行，4 个测试，全部 `#[ignore]`，testcontainers）

KC-05：老成交客户（outcome_events 缺 verification/eventKind 字段的 legacy BSON）的受众圈选覆盖 + m030 迁移。

- `m030_backfills_missing_verification_and_event_kind`（campaign_segment_coverage.rs:14-71）：m030 迁移把缺失字段补默认（verification="staff_confirmed"、eventKind="deal"），productRef 原值不破坏。
- `m030_does_not_overwrite_existing_values`（campaign_segment_coverage.rs:74-128）：已有 conversation_inferred/reversal 的元素不被默认值覆盖；跑两遍验幂等。
- `coarse_query_includes_legacy_event_missing_fields`（campaign_segment_coverage.rs:133-183）：防线 A——与 build_segment_coarse_filter 等价的 $elemMatch 粗筛（verification $in 或 $exists:false + eventKind ≠ reversal）能命中缺字段老成交（回填前也纳入）。手工复刻式断言（pub(super) 不可直调）。
- `m030_does_not_create_deal_events_key_on_outcome_events_only_doc`（campaign_segment_coverage.rs:191-256）：C1 回归哨兵——m030 绝不给只有 outcome_events 的文档凭空造 `deal_events:[]` 键；因 Contact.outcome_events 带 `#[serde(alias="deal_events")]`（注释引 models.rs:248），两键同现会 serde duplicate_field 使类型化读取崩溃。双层断言：raw 层键不存在 + 类型化读回成功。

### 2.38 tests/cold_reactivation_idempotent_pbt.rs（169 行，4 个 property，非 ignore，纯函数 PBT）

对象：`cold_contact_worker::decide_cold_emit(&contact, now_ms, cold_before_ms, has_pending_follow_up)`（镜像 scan_cold_outbound 判定）。
- **P1 already_pending_skips_emit**（cold_reactivation_idempotent_pbt.rs:87-106）：同 contact 已有 pending follow_up → 恒 AlreadyPending 绝不 Emit（"同 contact 一天一次"幂等核心）。
- **P2 non_managed_never_emits**（cold_reactivation_idempotent_pbt.rs:113-126）：agent_status≠Managed 永不 emit（normal/blocked 不被冷链路骚扰的红线）。
- **P3 inbound_newer_than_outbound_skips**（cold_reactivation_idempotent_pbt.rs:134-147）：用户已回话（inbound 晚于 outbound）→ UserRecentlyReplied（属 silent 段不属 cold）。
- **P4 emit_only_when_outbound_strictly_old**（cold_reactivation_idempotent_pbt.rs:156-168）：唯一可发条件 = Managed + outbound 早于 cold_before + 无 inbound 反超 + 无 cooldown + 无 pending → Emit。

### 2.39 tests/planner_silent_followup.rs（221 行，1 个测试，`#[ignore]`，start_repl_set）

**`planner_emits_follow_up_for_silent_managed_contacts_only`（planner_silent_followup.rs:80-221）**
- 不变量（M1 静默跟进扫描）：5 个 managed+静默（last_inbound 200h 前 > 72h 阈值）被 emit follow_up；3 个 normal 状态即便静默不 emit；cooldown_until 在未来的 managed 被 filter 排除。task.content 以 "Planner: silent_follow_up" 起头且 **review_required=true 保留**（planner emit 的主动触达仍走完整评审，planner_silent_followup.rs:139-154）。
- 审计：每 tick 写 1 条 strategic_planner_tick（即便 emit=0）、每 emit 写 1 条 strategic_planner_emit；第二次 tick 幂等（已有 pending follow_up 跳过：任务仍 5、emit 事件仍 5、tick 事件 +1）。

### 2.40 tests/planner_commitment_due.rs（323 行，1 个测试，`#[ignore]`，start_repl_set）

**`planner_emits_commitment_overdue_and_imminent_only`（planner_commitment_due.rs:87-323）**
- 不变量（M2 承诺到期扫描）：6 类 contact 中只 emit 2 条——overdue（due 5h 前）→ "Planner: commitment_overdue"+id=cmt-overdue-1；imminent（due 4h 后 < 8h 窗）→ "Planner: commitment_imminent"。不 emit：Plain 旧字符串承诺（无 due_at）、future（超 imminent 窗）、normal 状态、cooldown 中。
- 审计事件 detail 含 commitmentId/reason/dueAt（planner_commitment_due.rs:243-270）；第二次 tick 幂等（24h dedup + has_pending_follow_up 双保险）；commitment_tick 每次都写。

### 2.41 tests/planner_calendar_care.rs（297 行，2 个测试，`#[ignore]`+`#[serial]`，start_repl_set）

§3.7 主动情绪关怀（scan_calendar）。用 `seed_active_profile` + `invalidate_global_domain_profile_cache`（进程级缓存失效——LazyLock 单例跨测试残留是这里 #[serial] 的原因，planner_calendar_care.rs:120-129）。

**`calendar_care_emits_for_emotional_profile_today_anniversary`（planner_calendar_care.rs:131-244）**
- 不变量：情感陪伴 active profile（calendar.enabled + anniversaries date_dimension）下，memory_card.extra.anniversaries 含"今日"（+8 时区 MM-DD）结构化条目 → emit 1 条 "Planner: calendar_care" follow_up（含纪念日标签、review_required 保留）+ 1 条 strategic_planner_calendar_care 事件。
- **SR-135 稳定身份去重**：把首轮 task 改成终态（sent）后同日重扫**仍不重 emit**——同一运营日同一纪念日业务 intent 由稳定身份挡住，不依赖 has_pending_follow_up 偶然去重（planner_calendar_care.rs:205-243）。

**`calendar_care_no_emit_for_default_sales_profile`（planner_calendar_care.rs:246-297）**
- 不变量（销售域零扰动护栏）：DEFAULT 销售 profile（calendar 关、无 date_dimension）下同样的今日纪念日 → 0 emit，且 scan_calendar 提前短路**连 calendar_tick 事件都不写**。

### 2.42 tests/planner_block_rate_backoff.rs（254 行，2 个测试，`#[ignore]`，start_repl_set）

M3 反馈环（block-rate backoff）。

**`planner_silent_segment_skips_when_block_rate_above_threshold`（planner_block_rate_backoff.rs:103-203）**
- 不变量：24h 内 4 条 blocked_by_safety_guard + 1 条 approved（block-rate=0.8 > 0.6 阈值、分母 ≥ min_runs=3）→ 本 tick 对该 contact **不 emit follow_up、不写 strategic_planner_emit**，写 1 条 `strategic_planner_silent_backoff` 事件，details 含 blockRate（≥0.6）/blockedCount=4/okCount=1；且 backoff 不消费 daily cap（EMIT_EVENT_KINDS 不含 backoff，文件头声明）。

**`planner_silent_segment_passes_when_under_min_runs`（planner_block_rate_backoff.rs:205-254）**
- 不变量：仅 2 条 blocked（分母 < min_runs=3）→ 反馈环不参与判定、正常 emit 1 条 follow_up、0 条 backoff（冷启动不被误封）。

### 2.43 tests/sr135_proactive_outreach.rs（552 行，7 个测试，全部 `#[ignore = "requires replica-set MongoDB"]`，start_repl_set，部分 multi_thread 并发）

SR-135 主动触达统一提交协议：`proactive_outreach::commit_follow_up` / `commit_signal_with_daily_quota`（事务性 task+event+quota 三写）、`DailyQuota`（namespace/total_cap/segment_cap/initial 基线）。

- `concurrent_same_intent_commits_one_task_event_and_reservation`（sr135_proactive_outreach.rs:57-119）：**32 路并发同一业务 intent（cap=1）→ 恰 1 Emitted + 31 Duplicate、绝不 Capped**（同 intent 输家必须收敛为 Duplicate 而不是重复消费配额）；落库恰 1 task + 1 event + quota total=1。
- `concurrent_distinct_intents_never_exceed_daily_cap`（sr135_proactive_outreach.rs:121-180）：24 路并发不同 intent（cap=3）→ 恰 3 Emitted + 21 Capped；task/event/quota 均=3——并发下日配额**从不超发**。
- `full_utc_day_bucket_does_not_block_the_next_day`（sr135_proactive_outreach.rs:182-246）：UTC 日桶隔离——前一日桶打满（cap=1）不阻塞次日（同 intent 换日可 Emitted）；两日各一桶 total=1。
- `segment_cap_and_shared_total_cap_are_both_persistent`（sr135_proactive_outreach.rs:248-319）：段上限（segment_cap=2）与共享总上限（total_cap=3）双闸——calendar 段发 2 后第 3 条 Capped（段闸）；renewal 用剩余总额发 1 后 Capped（总闸）；桶 segments 计数持久化（calendar=2、renewal=1）。
- `existing_bucket_catches_up_with_late_legacy_event_baseline`（sr135_proactive_outreach.rs:321-412）：滚动升级桥——旧二进制在新桶已存在后补发的 legacy 事件通过 initial_total/initial_segment 基线追认；追认后达 cap 即 Capped（桶 total 收敛到 4）。
- `event_insert_failure_rolls_back_task_and_quota`（sr135_proactive_outreach.rs:414-484）：**三写事务性**——validator 拒绝 event insert → commit 返 Err，task/event/quota 全部 0（无半提交状态）。
- `silence_duplicate_does_not_consume_persistent_daily_quota`（sr135_proactive_outreach.rs:486-552）：行为信号提交同协议——同 silence 信号重复提交 Duplicate（不耗配额）、cap=1 时第二个不同 contact 的信号 Capped；behavior_signals 落库恰 1 条。

### 2.44 tests/media_asset_send_integration.rs（321 行，2 个测试，全部 `#[ignore]`，testcontainers）

素材上传→审核→发送数据流。文件头声明覆盖策略：`load_sendable_assets`/`filter_sendable_candidates` 是 pub(crate)，Test 1 复刻**完全相同**的查询条件（"任何一方改条件、这里跟着改"），纯函数逻辑由 lib 单测覆盖；Test 2 outbox 链路是完整 public API（media_asset_send_integration.rs:12-16）。

- `upload_then_review_then_only_approved_is_sendable`（media_asset_send_integration.rs:166-220）：可发素材过滤 = workspace + (account_id=null 全局或本账号) + sendable=true + **review_status="approved"**——draft 不出现在可发清单，审核通过后出现；stage 命中谓词（target_stages None/空=总命中，非空需含当前 stage）。
- `media_outbox_entry_is_idempotent_per_asset`（media_asset_send_integration.rs:242-321）：媒体 outbox 条目按 asset 幂等（synthetic_media key 含 asset_id）——同 (run, contact, asset) 二次入队 IdempotentSkip 且同幂等键；不同 asset 各自 Created 键不同；媒体条目 content 可空。

### 2.45 tests/media_asset_crud_integration.rs（594 行，8 个测试，全部 `#[ignore]`，testcontainers，直调 handler）

素材库 CRUD。文件头两个重要声明：①`replace_content_asset_file` 取 Multipart extractor，tests crate 无法手工构造 → 换文件的"清 media_id+退 draft"副作用**不在集成层验，由代码审查保证**（media_asset_crud_integration.rs:12-18，见第 5 节疑点）；②文件落盘隔离——每测试用进程内唯一临时目录覆盖 media_storage_dir（media_asset_crud_integration.rs:20-23）。

- `edit_meta_updates_fields_keeps_review_status`（media_asset_crud_integration.rs:144-182）：改元数据（title）不退审（review_status 仍 approved）。
- `edit_meta_out_of_dict_stage_rejected`（media_asset_crud_integration.rs:186-227）：targetStages 含字典外阶段名 → BadRequest 且不落地（m006 字典归一校验）。
- `toggle_sets_sendable`（media_asset_crud_integration.rs:231-273）：toggle 写 sendable=false 落库。
- `toggle_cross_workspace_404` / `delete_cross_workspace_404`（media_asset_crud_integration.rs:277-312、440-477）：IDOR——跨 workspace 写/删 NotFound 且零副作用。
- `delete_removes_db_and_file_when_no_siblings`（media_asset_crud_integration.rs:316-365）：无兄弟引用 → DB 记录删 + **物理文件删**（真实 media_storage 落盘验证）。
- `delete_keeps_file_when_sibling_references_it`（media_asset_crud_integration.rs:369-436）：**引用计数防误删**——两条 asset 共享同一 file_path（upload 不去重），删 A 后 B 保留、物理文件必须仍在。
- `wrong_asset_scope_is_conflict_with_zero_document_and_audit_writes`（media_asset_crud_integration.rs:481-594）：SR-160——账号私有素材（account_id="account-a"）上四种写动作（review/meta/toggle/delete）带错误 expectedScope/expectedAccountId → 全部 Conflict、asset BSON 逐字节不变、0 条审计事件（实体 scope 是所有写动作的 CAS 身份）。

### 2.46 tests/media_storage_consistency.rs（577 行，4 个测试，全部 `#[ignore]`，testcontainers+真实文件系统）

HC-006/SR-017：本地媒体对象与 Mongo 元数据一致性（两阶段 stage→publish + 路径锁 + reconciler）。

- `router_upload_db_failure_removes_pending_and_final_object`（media_storage_consistency.rs:70-172）：真实 HTTP multipart 上传 + validator 拒 DB insert → 502；**DB 0 行、final 与 pending 文件都不存在**（DB 失败不得发布/遗留物理对象）。
- `failed_db_write_settlement_cleans_or_publishes_by_live_reference`（media_storage_consistency.rs:174-240）：`settle_staged_after_db_failure` 按活引用裁决——无引用的 staged 对象清除（pending+final 都删）；**有存活 DB 引用**（另一路已插入同 file_path 行）的 staged 对象发布为 final（可读回原字节）。
- `reconciler_recovers_pending_removes_orphans_and_disables_missing_rows`（media_storage_consistency.rs:242-464）：`reconcile_once` 六类场景一次收敛——有引用的 pending 恢复发布（recovered_pending=1）；无引用 pending 删（removed_pending）；无引用 final 孤儿删（removed_orphans=1）；引用了不存在对象的行**保留供运营修复**但降级 sendable=false + review_status="draft" + review_note="storage_object_missing" + 清 media_id（media_storage_consistency.rs:347-361）；sha 不符的 corrupt 对象删并降级行；非法路径（"../outside.pdf"）行降级 review_note="storage_path_invalid"；第三次 reconcile 返回默认空报告（**幂等**，media_storage_consistency.rs:457-460）。
- `path_lock_closes_zero_reference_then_new_reference_delete_race`（media_storage_consistency.rs:466-577）：路径锁关闭"删除方已见 0 引用、新引用方同时创建"的竞态窗——create 方必须等 delete 方释放路径锁（AtomicBool 验证等待）；最终恰 1 条引用且对象可读（不会被 delete 方误删刚发布的新对象）。

### 2.47 tests/sr094_runtime_parameters.rs（536 行，1 个大测试，`#[ignore = "requires MongoDB replica set"]`，start_repl_set+真实 HTTP）

SR-094：runtime-parameter 两条写路径的类型化边界 + Guide 全局影响的冻结确认事务。测试用自建 32MB 栈线程 + current_thread runtime（sr094_runtime_parameters.rs:191-207，深调用栈规避）。

**`typed_runtime_writes_and_guide_apply_are_enforced_end_to_end`（sr094_runtime_parameters.rs:191-536）**，一条链覆盖：
- 手动 PUT 域配置：合法 runtime 参数写入成功；**legacy 别名归一**——写 factRiskBlockAt=8 落库为 canonical 键 hallucinationBlockAt=8、原别名键不存在（sr094_runtime_parameters.rs:283-300）；
- 含未知键 unknownRuntimeKey 的 PUT → 400 且域文档**BSON 级零写入**（sr094_runtime_parameters.rs:313-331）；
- Guide 高风险预算修改（runTokenBudget）→ preview 400、域不变、**不创建 preview 行**（被拒的高风险输出不得留下 apply 能力，sr094_runtime_parameters.rs:333-382）；
- 合法 Guide preview（maxDailyTouches=2 全局）→ impactScope="workspace_user_operations"、requiresStrongConfirmation=true；**Preview 不创建 operating_memory**（sr094_runtime_parameters.rs:403-412）；
- apply 缺 confirmGlobalImpact → 400、preview 与 domain 都零写；
- 确认 apply → committed=true、域参数落 2、恰 1 条 `user_operation_guide_applied` 审计、**确认 Apply 恰一次创建冻结的缺失记忆基线**（operating_memory=1，sr094_runtime_parameters.rs:488-497）；
- **同 candidateHash 重放 apply → 返回相同 receipt 不重复应用**（sr094_runtime_parameters.rs:520-532）。

### 2.48 tests/transactional_admin_flows.rs（1695 行，9 个测试，全部 `#[ignore]`，start_repl_set（事务需要副本集）+真实 HTTP/直调）

管理端事务回归（taxonomy 审批 / Guide 应用 / 关系审核）。

- `operation_domain_reset_appends_version_and_preserves_history`（transactional_admin_flows.rs:68-184）：域重置**追加**新版本（version=历史 max+1、previous_version=被退休 current、seeded_by="admin_reset:{username}"）且保留全部历史行、恰一个 current。
- `domain_profile_dimension_kinds_reject_dynamic_paths_and_reserved_names`（transactional_admin_flows.rs:186-214，纯函数）：`validate_profile_dimension_kinds` 拒绝：前导空格/点路径/$ 前缀/大写驼峰/非 ASCII/保留名（value_tier、awaiting_principal_decision、custom_updated_at）/重复项；合法 snake_case 放行——防动态维度名注入 Mongo 路径。
- `guide_unicode_keys_do_not_panic_and_candidate_lands_as_draft`（transactional_admin_flows.rs:216-266）：LLM 生成的 profile 含 Unicode 键（"客户Stage"/"éValue"）不 panic；候选落 release_status="draft"、非 current、非 active、seeded_by="generated_by_ai"。
- `hc015_gateway_writes_one_candidate_and_one_bayesian_point_per_run`（transactional_admin_flows.rs:341-485）：一次 run 内——未知 customer_stage 恰建 1 条 pending taxonomy_candidate（occurrences=1）；**同 run 内重复 Bayesian 观察坍缩为 1 个点**（同维度两条 confidence 0.4/0.9 → history.len()=1、取 0.9）；Bayesian 点带 producing run_id（经 post-decision projection，transactional_admin_flows.rs:449-482）。
- `hc015_m050_backfills_history_fails_before_write_and_multikey_rejects_conflict`（transactional_admin_flows.rs:518-673）：m050 identityClaims 回填——正常行回填 [canonical, alias]；**歧义活跃 claim 全审计先失败（第一笔写之前）**（sentinel 行无 identityClaims 证明零写，transactional_admin_flows.rs:604-619）；恢复唯一索引后第二个活跃 owner 撞 multikey 唯一索引。
- `taxonomy_approval_rolls_back_claim_when_dictionary_insert_fails`（transactional_admin_flows.rs:795-910）：审批候选时字典 insert 冲突（预置 i32::MAX 版本冲突行）→ 502 且候选**回滚回 pending**（事务性）；清除冲突后重试成功；**SR-058：reviewed_by 必须来自认证会话**（请求体伪造 "spoofed@attacker.invalid" 被忽略，落 "transaction_test_admin"，transactional_admin_flows.rs:888-892）；current 版本恰 1。
- `taxonomy_candidate_merge_appends_alias_version_and_preserves_runtime_fields`（transactional_admin_flows.rs:912-1044）：合并进已有 canonical——mergedIntoExisting=true；新版本号=历史 max+1（10）、previous_version=真实被退休 current（3）；**运营字段不被候选覆盖**（display_name/description/status/priority_weight/is_terminal/is_reactivation_target 保留现值）；aliases 合并去重 + 追加 raw_value。
- `relationship_review_ignores_spoofed_actor_and_uses_authenticated_admin`（transactional_admin_flows.rs:1046-1275）：SR-058（伪造 reviewedBy 被忽略）+ SR-059（审批成功=建议终态+contact.domain_attributes.relationship_type 同事务提交；第二轮 contact 写失败（validator）→ 502 且建议 CAS 回滚回 pending、上一轮已提交的 relationship_type 保留）+ SR-060（**部分唯一索引**：已 approved 历史不占 pending 槽、新证据周期可插入，但同周期第二条 pending 撞 E11000）。
- `guide_apply_rolls_back_all_writes_and_retries_once`（transactional_admin_flows.rs:1277-1695）：Guide 应用完整事务协议——SR-150 错误身份（expectedAccountId 不符）→ 409 且**租约获取前零写**（preview/contact/playbook/memory/task/event 全 BSON 不变）；篡改 candidateHash → 409；缺全局确认 → 400 且 preview 不被 claim；dedupe_key 冲突使事务失败 → 502 全回滚（contact note 无、playbook 仍 v1、preview status="failed" + apply_protocol_version=3）；清除冲突后重试 → 200 committed（contact+playbook v2 同时落地）；**重放返回逐字节相同 receipt** 且 playbook 不再 bump。

### 2.49 tests/contacts_batch_enable.rs（1034 行，12 个测试，全部 `#[ignore]`，testcontainers，直调 handler）

`POST /api/contacts/batch-enable` 批量托管（managed 准入）+ initial_profile 异步画像。

- `empty_shared_note_rejected` / `unregistered_account_rejected`（contacts_batch_enable.rs:107-139）：空 sharedNote / 未注册账号 → BadRequest。
- `pool_candidate_from_other_account_is_conflict_and_zero_write`（contacts_batch_enable.rs:143-231）：SR-153——pool 来源的候选属于别的账号（旧快照切换后）→ **整批 Conflict、零写**（contact BSON/任务数/事件数全不变）；pool 路径不采信客户端传来的身份字段。
- `sparse_import_preserves_identity_and_non_human_import_is_zero_write`（contacts_batch_enable.rs:233-327）：`upsert_contact_from_value` 稀疏导入不抹掉已有身份字段（只带 wxid 不动 nickname/remark/alias；带单字段只更新该字段）；**非人类 wxid（fmessage/weixin/gh_*/@chatroom/@openim）导入零写**（返回 None、不建行）。
- `batch_rejects_non_human_candidates_without_contact_or_task`（contacts_batch_enable.rs:329-376）：批量托管非人类候选 → enabled=0、queued=0、rejectedNonHuman=5、0 contact 0 task。
- `task_insert_failure_never_leaves_managed_contact`（contacts_batch_enable.rs:378-442）：validator 拒 initial_profile task insert → 错误上抛且**不留半提交的 managed contact**（"持久任务意图先于 managed 可见"顺序不变量）。
- `concurrent_batch_enable_has_one_active_initial_profile_intent`（contacts_batch_enable.rs:444-505）：并发两路同 wxid 托管 → queued 合计恰 1、active_task_key="initial_profile" 恰 1 条、managed contact 恰 1（单飞意图）。
- `rotated_generation_retires_old_task_and_reenables_in_same_request`（contacts_batch_enable.rs:507-637）：enrollment_token 代际轮换——禁用轮换了 token 但旧任务未取消的崩溃窗，下一次 enable 同请求内修复：旧任务 cancelled（保留审计、清 active_task_key）、新任务新 token、contact managed 且带新 token。
- `batch_enables_and_queues_initial_profile_tasks`（contacts_batch_enable.rs:639-708）：正常批量——2 个候选 enabled=2 queued=2；sharedNote→human_profile_note、avatarUrl/sex 落库；**竞态修复：全新客户在 upsert 阶段同步拿到状态机 initial 态（new_contact + confidence=6），不等异步画像回填**（contacts_batch_enable.rs:686-694）。
- `idempotent_does_not_requeue_already_managed`（contacts_batch_enable.rs:710-753）：已 managed 再次批量 → enabled 计数（刷新 note）但 queued=0、任务总数仍 1。
- `batch_preserves_previously_operated_state_but_seeds_new`（contacts_batch_enable.rs:758-868）：老客户（last_agent_run_at 非空 + operation_state="deal_won"）批量托管**不被 initial 态覆盖**（state/confidence 保留）；全新客户同步落 initial 态——锁定 is_previously_operated 判定。
- `initial_profile_task_marks_sent_when_unmanaged` / `initial_profile_task_marks_sent_when_contact_gone`（contacts_batch_enable.rs:953-1034）：W-Batch3 终态回归——initial_profile 任务 handler 早退（联系人被取消托管/已删除）也必须写终态 sent + gateway_status（"unmanaged"/"contact_gone"），**绝不停 running**（否则被 reclaim 反复重跑）。

### 2.50 tests/webhook_contact_upsert_integration.rs（283 行，4 个测试，全部 `#[ignore]`，testcontainers，直调 wechat_webhook）

真人漏斗建档接线（Task 3 回归三条已修 bug）。

- `non_person_gh_persists_message_but_no_contact` / `non_person_chatroom_persists_message_but_no_contact`（webhook_contact_upsert_integration.rs:44-147）：非真人发件人（gh_ 公众号 / @chatroom 群）入站 → 响应 skipped="not_operatable_contact"；**消息仍落 conversation_messages（不建 contact ≠ 丢消息）**但 contacts 绝不建行。
- `person_with_roster_hit_enriches_from_roster`（webhook_contact_upsert_integration.rs:151-220）：真人 + roster 快照命中 → contact.nickname/avatar_url 来自 **roster 快照**（"张三"），绝不取 payload `_mcp.nickName` 里的账号自身昵称（"Demi" bug 回归）；webhook 新建 contact 默认 **Normal**（不触发 Agent 流水线——本测试不排 LLM 响应即为证明）。
- `person_without_roster_hit_leaves_identity_none`（webhook_contact_upsert_integration.rs:224-283）：roster 未命中 → contact 仍建成但 nickname/avatar_url=None（best-effort：拿不到留空，绝不阻断建档、绝不写 payload 脏昵称）。

### 2.51 tests/last_inbound_split.rs（193 行，2 个测试，全部 `#[ignore]`，testcontainers）

HP-2：last_inbound_at / last_outbound_at 字段拆分。
- `inbound_update_only_touches_inbound_fields`（last_inbound_split.rs:67-111）：入站 update 只设 last_inbound_at+last_message_at，不动 last_outbound_at。
- `outbound_update_via_pipeline_keeps_inbound_unchanged`（last_inbound_split.rs:113-193）：出站用 aggregation pipeline（$cond 取 max，**与 send_outbound_message 实际写法一致**的复刻）设 last_outbound_at + last_message_at=max(inbound, now)，不动 last_inbound_at。
- 疑点：两个测试都是"测试自己发 update 语句"而非调生产函数——锁定的是写法约定而非生产代码路径（见第 5 节）。

### 2.52 tests/string_fact_risk_guard.rs（225 行，10 个单测 + 2 个 PBT，非 ignore，纯函数）

历史沿革（文件头）：原字符串级产品声明 marker 扫描已随 2026-05-25 知识库清理删除，方法论切换为 wiki+三闸（grounding/hallucination/run_budget）；本文件为保住 R11.6 PBT 基线改成对 `check_state_transition` 的**补充**性质测试，覆盖 state_transition_pbt 未覆盖的输入域（string_fact_risk_guard.rs:1-11）。
- `no_domain_config_skips_validation`（string_fact_risk_guard.rs:83-87）：cfg=None fail-open（向后兼容）。
- `empty_state_machine_fails_closed`（string_fact_risk_guard.rs:89-102）：S1.2——active domain 有 cfg 但 state_machine 为空 doc → **fail-closed**，理由含 state_transition_invalid + state_machine_empty（与 cfg=None 的 fail-open 形成对比：有配置但配置坏 → 拦）。
- `unknown_target_state_fails_closed` + PBT `unknown_target_always_fails_closed`（string_fact_risk_guard.rs:104-119、189-203）：问题 E 修复——未登记 target 恒 fail-closed（此前 `?` early-return fail-open 会让未知 customer_stage 写入幻影 operation_state 旁路 policy）。
- `empty_from_to_new_contact_passes` / `empty_from_to_non_new_contact_blocks` / `whitespace_from_treated_as_empty`（string_fact_risk_guard.rs:135-169）：空/纯空白 from trim 后走 initial 分支。
- PBT `allow_from_any_accepts_arbitrary_from`（string_fact_risk_guard.rs:177-187）：allowFromAny 接受任意 from。
- `block_reason_format_is_stable`（string_fact_risk_guard.rs:208-213）：拦截理由**恒以 state_transition_invalid 开头**——review/gateway 用该子串区分 guard 拦截类别（字符串协议回归）。
- `custom_state_machine_via_document_is_honored`（string_fact_risk_guard.rs:216-225）：自定义 Document state_machine 可被读取。

### 2.53 tests/conversation_mode_decision_schema.rs（281 行，5 个单测 + 2 个 PBT，非 ignore，纯函数）

对象：`RawAgentDecision::validate_and_promote` 的 conversationMode 严格枚举（与 types.rs::CONVERSATION_MODE_VALUES 对齐：casual_relationship/value_exchange/consultative/boundary_protection，conversation_mode_decision_schema.rs:22-28）。
- 合法四值原样保留、无相关 risk（conversation_mode_decision_schema.rs:58-84）；None/空串/纯空白 → `missing_required_field:conversation_mode`，兜底默认 **casual_relationship（最保守）**（conversation_mode_decision_schema.rs:86-121）。
- 14 个已知 LLM 漂移值（大小写/截断/拼写/中文同义/跨字段污染/null 字面量）逐一 reject 并落默认值（conversation_mode_decision_schema.rs:125-164）；PBT：任意随机非法串 reject（conversation_mode_decision_schema.rs:191-209）。
- **两个跳过通道**：R11 sunset（autonomy_protocol_enabled=false）跳全部校验 risks 为空（conversation_mode_decision_schema.rs:238-254）；tool_calling 中间轮跳过 R3 严格枚举（conversation_mode_decision_schema.rs:256-281）。

### 2.54 tests/sr072_policy_fail_closed.rs（301 行，1 个测试，`#[ignore]`，testcontainers）

**`current_machine_missing_policy_blocks_reply_and_management_send`（sr072_policy_fail_closed.rs:166-301）**
- 不变量（SR-072 运行时红线）：workspace 已有 current 状态机时，缺失 current state policy 必须在 Outbox/MCP 之前拦停**所有**客户发送路径。前置自证：fresh prompt-pack bootstrap 必须已 reconcile 出 need_discovery 的 active policy（sr072_policy_fail_closed.rs:210-229）；删掉该 policy 模拟 reconcile 失败后：
  - 回复路径 `handle_managed_message` → Err 含 "missing_current_operation_state_policy"、0 outbox、0 MCP 日志；
  - 管理发送路径 `send_contact_message_gateway`（ManualContactSend）→ **同一 state-action 闸**同样拦（sr072_policy_fail_closed.rs:282-296）——手动发送不绕闸。
- 手法备注：32MB 栈自建线程 + current_thread runtime（同 sr094）；eprintln 阶段标记（SR072_STAGE=...）辅助 CI 卡死排查。

### 2.55 tests/simulation_no_sideeffect_integration.rs（265 行，1 个测试，`#[ignore]`，testcontainers）

**`simulation_has_no_business_side_effects`（simulation_no_sideeffect_integration.rs:178-265）**
- 不变量（P0 影子模式红线）：`simulate_user_dialogue` 复用真实 Reply+Review+ClaimGate LLM 链但发送阶段只输出 would_send——**跑完后除 llm_call_logs/migrations 外全库逐文档（BTreeMap 全集合快照对比）零变化**（simulation_no_sideeffect_integration.rs:77-107、224-239）。快照法比"数 outbox 行"强得多：任何业务集合的任何字段变动都会红。
- 特别 seed：老 last_used_at 的 operator memory——live loader 会续期 last_used_at，Shadow 必须读同一行**而不 touch 它**（simulation_no_sideeffect_integration.rs:186-206）。
- 成本日志例外：恰 3 条 llm_call_logs（Reply/Review/ClaimGate）且全部 run_mode="shadow"（simulation_no_sideeffect_integration.rs:241-262）。

### 2.56 tests/dry_run_isolation.rs（196 行，2 个测试，全部 `#[ignore]`，testcontainers）

S-20：Management Agent dry-run 隔离。
- `dry_run_session_writes_dry_run_status_audit_only`（dry_run_isolation.rs:78-161）：dry_run 会话的 AgentCommandRun/AgentToolCall status="dry_run"；业务集合（contacts/agent_tasks）零写入；tool_call.response.would_execute 带 toolName 供前端回放。
- `non_dry_run_session_uses_normal_status`（dry_run_isolation.rs:163-196）：非 dry-run 会话 status="completed"。
- **疑点（弱断言）**：两个测试都是**测试自己 insert 这些审计行再读回**——并未驱动生产 management agent 执行链产生 dry_run 状态；锁定的是模型形态而非行为（见第 5 节）。

### 2.57 tests/worker_reclaim.rs（96 行，2 个测试，全部 `#[ignore]`，testcontainers）

HP-1：task worker stale running 回收。
- **疑点（显式承认的弱断言）**：文件内注释明说"由于 worker tick 是 private，本集成测试目前主要确认 stale task 的字段形态与 reclaim filter 匹配。完整端到端验证由 Task 24 的 PBT 收口"（worker_reclaim.rs:55-61）——两个测试（stale_running_task_is_recovered_to_retry / fresh_running_task_with_recent_claim_is_skipped）实际都只 insert task 后读回断言 status=="running"，**没有任何 reclaim 行为被驱动**。测试名与实际断言不符（名字说 recovered_to_retry，断言的是 running）。本组最弱的文件。

### 2.58 tests/account_offline_defer_integration.rs（377 行，3 个测试，全部 `#[ignore]`，testcontainers+wiremock）

E 组 ⑪：账号掉线不盲发。
- `webhook_offline_event_persists_online_false`（account_offline_defer_integration.rs:166-222）：webhook TypeName=Offline → account.online=false 落库（响应 ignored="offline_event"）；Online 对称落 true。
- `offline_account_defers_without_consuming_attempt`（account_offline_defer_integration.rs:226-324）：account.online=false 时 dispatcher defer——回 pending、**attempt 不变（掉线非发送失败）**、next_retry_at 被推后 60s（用 before_defer 基准坐实"推后"语义、去永真断言，account_offline_defer_integration.rs:267-304）、写 `agent.send_deferred_account_offline` 事件（AI 自治措辞）、**MCP 0 请求**。
- `online_account_sends_normally`（account_offline_defer_integration.rs:328-377）：online=true 正常 sent。

### 2.59 tests/account_round_robin_pbt.rs（161 行，4 个 property，非 ignore，纯函数 PBT）

对象：`account_scheduler::decide_assigned_account(&accounts, &used, cur_hour, wxid)`（镜像 assign_account 决策）。
- **P1 all_full_capacity_falls_back_or_none**（account_round_robin_pbt.rs:55-73）：全部 online 账号 capacity 打满 → 退化 online-only fallback，**保送达 > 严格遵守 capacity**（有 online 账号绝不返回 None）。
- **P2 stable_pick_per_wxid**（account_round_robin_pbt.rs:81-103）：同输入两次调用同结果——同 wxid 决策可复现（散列锚，客户总是被同一账号服务）。
- **P3 off_hours_excluded_when_alternative_exists**（account_round_robin_pbt.rs:113-133）：命中 off_hours 的账号在有其它候选时被排除。
- **P4 capacity_zero_unbounded**（account_round_robin_pbt.rs:141-160）：capacity=0=无限量参与候选，不被 capacity 闸排除。

### 2.60 tests/behavior_signal_idempotent_pbt.rs（183 行，4 个 property，非 ignore，纯函数 PBT）

自学习采集管道 Iron Law 系列。生产幂等由 partial unique 索引 {ws, account, dedupe_key} 保证（重复撞 11000 被 persist_signal 吞成 Ok(false)）；PBT 用内存 HashSet 建模索引（behavior_signal_idempotent_pbt.rs:1-9）。
- **P1 replay_collapses_to_unique_keys**（behavior_signal_idempotent_pbt.rs:39-71）：任意重放序列写入数 == 不同 dedupe_key 数（Iron Law ⑤ 采集幂等）。
- **P2 silence_dedupe_per_outbound**（behavior_signal_idempotent_pbt.rs:81-114）：同 outbound 多 tick 探测只产 1 个 silence key；不同 outbound 产不同 key。
- **P3 inbound_signal_types_never_collide**（behavior_signal_idempotent_pbt.rs:123-144）：同一 inbound 的 latency/length/reactivation 三类 key 两两不同（可共存不互覆）。
- **P4 silence_always_censored_others_never**（behavior_signal_idempotent_pbt.rs:153-183）：**Iron Law ②：沉默恒 censored=true（删失，不是负例）**、其余 T1 信号恒 censored=false；全部 source=system_observed、confidence=1.0（Iron Law ④）。

### 2.61 tests/behavior_signal_smoke.rs（267 行，3 个测试，全部 `#[ignore]`，testcontainers）

自学习采集管道端到端落库冒烟。
- `behavior_signal_dedupe_round_trip`（behavior_signal_smoke.rs:77-138）：真库验证 persist_signal——首次 Ok(true)、同 dedupe_key 第二次撞 partial unique 索引 Ok(false)；latency+length 各 1 条；落库字段：source=system_observed、confidence=1.0、censored=false、latency_ms 保留。
- `deal_event_push_round_trip`（behavior_signal_smoke.rs:140-209）：H10 向后兼容——故意用**旧 deal_events key** $push 成交事件，serde alias 让旧库文档仍能读入新 outcome_events 字段。注意测试自己也踩到坑：两 key 同现会 duplicate field error，必须先 $unset 新 key（behavior_signal_smoke.rs:168-182）——旁证了 campaign_segment_coverage C1 哨兵测试防的正是这个坑。
- `silence_worker_single_round_idempotent`（behavior_signal_smoke.rs:211-267）：真跑 `silence_signal_worker::tick` 两轮 → 同一 outbound 只落 1 条 silence 信号（dedupe_key 幂等）且 censored=true、unanswered=true。

### 2.62 tests/review_task_now_claim.rs（373 行，4 个测试，全部 `#[ignore]`，testcontainers，直调 handler）

W-Batch3 [S-01]/[S-02]：admin `review_task_now` 的原子 CAS claim（修复前 filter 无 status 前置、不写 claimed_at → 双跑/reclaim 盲区）。
- `review_task_now_rejects_running_task`（review_task_now_claim.rs:126-166）：running 任务被 CAS 拒（Conflict）——绝不与 worker 双跑；任务状态与 claimed_at 不被改动。
- `review_task_now_rejects_terminal_task`（review_task_now_claim.rs:169-197）：终态（sent）不可复核（防重跑已发送任务）。
- `review_task_now_claims_pending_and_clears_lease_on_success`（review_task_now_claim.rs:201-257）：pending 被原子认领（attempt_count 递增证明 claim 发生）；memory_consolidation 无候选走 sent 早退；**成功终态必须清 claimed_at**（不遗留看似仍被持有的 lease）。
- `wrong_account_task_actions_are_conflict_with_zero_task_and_outbox_writes`（review_task_now_claim.rs:261-373）：SR-155——错账号 review/cancel → Conflict 且任务与已绑定 outbox 逐字节不变（拒绝发生在原子账号 CAS 上）。

### 2.63 tests/escalation_push_time_reassign.rs（149 行，2 个测试，全部 `#[ignore]`，testcontainers）

KD-05：改派推送时刻记录 + m031 迁移。
- `reassign_refreshes_last_pushed_at_ms`（escalation_push_time_reassign.rs:15-76）：改派 $set principal_wxid + last_pushed_at_ms=改派时刻；**created_at 不被篡改（保真实创建审计）**。疑点：reassign_escalation 是 pub(crate)，测试用 raw $set **模拟**改派写法（复刻式，见第 5 节）。
- `m031_backfills_last_pushed_at_from_created_at`（escalation_push_time_reassign.rs:79-149）：m031 给缺 last_pushed_at_ms 的历史行回填 created_at；已有值不覆盖（幂等）。

### 2.64 tests/hc020_management_command_protocol.rs（511 行，2 个测试，全部 `#[ignore]`，testcontainers+真实 HTTP+wiremock）

HC-020：管理命令安全协议——模型生成的写计划冻结在显式管理员确认之后。

**`frozen_command_requires_matching_account_hash_and_authenticated_admin`（hc020_management_command_protocol.rs:278-386）**
- 不变量：management-agent 会话中 LLM 规划出 write_deal_events 工具调用 → command status="pending_confirmation" + planHash 冻结；确认时**账号不符 → 409、planHash 被篡改 → 409**（确认前 contact.outcome_events 为空）；正确确认 → succeeded、成交事件落 verification="staff_confirmed"、**marked_by=认证管理员**（"hc020-admin"）、command_run 记录 plan_hash/confirmed_by；**全程恰 1 次 LLM（只允许规划调用）**（hc020_management_command_protocol.rs:382）。
- 附带：MCP tools/list 里的 message_send_text 是"raw send must be removed"的哨兵描述（hc020_management_command_protocol.rs:33-38）。

**`stale_executing_intent_becomes_unknown_without_replaying_mcp`（hc020_management_command_protocol.rs:388-511）**
- 不变量（不确定副作用边界的保守恢复）：command 卡在 running + 工具意图卡在 executing（模拟进程死亡，execution_token="dead-process-token"）→ 重新 confirm → **status="execution_unknown"**（工具意图 finalized、execution_token 清空）；**不重放 MCP（tools/call 0 次）、不写 contact**——越过副作用边界的死租约绝不自动重试。

### 2.65 tests/suspected_deal_e2e.rs（552 行，5 个测试，全部 `#[ignore]`，testcontainers（部分 repl_set），直调 handler）

F23 疑似成交待核实闭环。**红线：AI 永不直写 outcome，只有运营核实 approve 才落成交**（suspected_deal_e2e.rs:9-10）。

- `list_then_approve_lands_staff_confirmed_deal`（suspected_deal_e2e.rs:124-262）：list 含 evidence/confidence；approve（带 amount/currency）→ contact.outcome_events +1 且 **verification="staff_confirmed"**；signal approved、reviewed_by=认证管理员（SR-058 伪造 reviewedBy 被忽略）；审批+成交+审计（outcome_event_marked）**同一事务三结果**；**重复审批在 pending CAS 冲突**——不双计成交不重复审计（suspected_deal_e2e.rs:227-261）。
- `invalid_approve_payload_keeps_signal_pending_and_outcome_empty`（suspected_deal_e2e.rs:266-314）：SR-057——负金额在 pending CAS **之前**预检拒绝，signal 留 pending 可修正重试（不留 approved-but-no-outcome 漏登终态）。
- `audit_failure_rolls_back_signal_and_outcome`（suspected_deal_e2e.rs:317-408）：SR-057——事务内最后一步审计写失败（validator）→ signal CAS 和 outcome append 一起回滚（signal 回 pending、0 成交、0 审计）。
- `reject_marks_rejected`（suspected_deal_e2e.rs:411-459）：reject → rejected + reason，**绝不落成交**。
- `terminal_signal_does_not_block_new_pending`（suspected_deal_e2e.rs:473-552）：Stage4 修复钉——unique 锚从全量 (ws,contact) 改为 **status="pending" 部分唯一索引**：approved 历史不占槽（二次成交可再生成新 pending）、但同周期第二条 pending 仍撞 E11000（去重保留）。直接锤 ensure_indexes 建出的真实索引。

### 2.66 tests/deal_event_scope_integration.rs（134 行，1 个测试，`#[ignore]`，testcontainers，直调 handler）

**`wrong_account_deal_event_is_conflict_with_zero_outcome_and_audit_writes`（deal_event_scope_integration.rs:76-134）**
- 不变量：管理端手动登记成交（add_deal_event）带错误 expectedAccountId → Conflict、0 成交事件、0 审计（账号 CAS 在写入前拦截）。

### 2.67 tests/outcome_task_workspace_dedupe.rs（99 行，1 个测试，`#[ignore]`，testcontainers）

**`outcome_task_unique_key_allows_same_account_content_in_distinct_workspaces`（outcome_task_workspace_dedupe.rs:51-99）**
- 不变量（SR-036）：outcome_aggregation 任务的去重唯一键含 workspace——同 (account, content) 在两个 workspace 各合法插入一条；同 workspace 重复插入撞 DuplicateKey。

### 2.68 tests/llm_retry_jitter.rs（91 行，6 个测试，非 ignore，纯函数）

基线四 PBT 文件之一（HP-4）。对象：`compute_backoff` / `is_retryable_llm_error`。
- `retry_after_dominates_short_baseline`（llm_retry_jitter.rs:19-28）：Retry-After(5s) > 退避基线时用 Retry-After。
- `exponential_backoff_when_no_retry_after`（llm_retry_jitter.rs:30-48）：退避 ∈ [base·2^(n-1), base·2^(n-1)+base)（prod 路径含 [0,base) jitter）。
- `long_exponential_overrides_short_retry_after`（llm_retry_jitter.rs:50-60）：指数基线(8s) > Retry-After(2s) 时用指数。
- `http_429_and_5xx_are_retryable` / `http_400_and_401_are_not_retryable` / `json_parse_error_is_not_retryable`（llm_retry_jitter.rs:62-91）：429/5xx 重试；400/401 不重试；**JSON 解析失败不重试（模型输出非 JSON 只调一次）**。

### 2.69 tests/outcomes_autonomy_endpoint.rs（471 行，7 个测试，全部 `#[ignore]`，testcontainers，直调 handler）

`GET /api/outcomes/autonomy` 自治指标端点（W6/Task 7.3 + M3/Task 70）。
- `outcomes_autonomy_returns_null_ratios_when_no_runs`（outcomes_autonomy_endpoint.rs:66-85）：total_runs=0 时**所有比率返回 null**（不是 0——区分"无数据"与"零率"）。
- `outcomes_autonomy_revision_trigger_rate_two_of_five_is_0_4`（outcomes_autonomy_endpoint.rs:87-145）：5 条 run 中 2 条 revision → revisionTriggerRate=0.4；revisionPassRate=0.5（1/2 通过）。
- `outcomes_autonomy_ai_hold_breakdown_each_one_third_with_three_holds`（outcomes_autonomy_endpoint.rs:147-193）：aiHoldBreakdown 按三类 AI-internal 状态（held_by_ai_policy/blocked_by_safety_guard/ai_waiting_for_more_context）各 1/3。
- `outcomes_autonomy_legacy_held_for_human_is_not_counted`（outcomes_autonomy_endpoint.rs:195-235）：R10——历史脏值 `held_for_human` **被剔除出 totalRuns 且不进任何分类**（"无人工接管"红线在指标层的体现：旧命名不复活）。
- `outcomes_autonomy_unverified_claim_block_rate_counts_only_blocked_status`（outcomes_autonomy_endpoint.rs:239-278）：blocked_unverified_product_claim 计数 → 1/4=0.25 + rawCounts。
- `outcomes_autonomy_taxonomy_candidate_rate_matches_review_risk_prefix`（outcomes_autonomy_endpoint.rs:280-340）：按 review.risks 的 `taxonomy_candidate:*` 前缀计数（多前缀混合行也算 1）→ 2/3。
- `outcomes_autonomy_outbox_link_breaks_down_by_status`（outcomes_autonomy_endpoint.rs:342-395）：outboxLink 按 sent/canceled/failed_terminal/delivery_unknown 分解计数与比率。
- `outcomes_autonomy_planner_section_aggregates_strategic_events`（outcomes_autonomy_endpoint.rs:408-471）：planner 子段按 kind 聚合 strategic_planner_* 事件（silent.emitted/commitment.overdueEmits/stagnation.backoff），无数据字段落 0 非 null。

### 2.70 tests/outcome_snapshot_freeze_integration.rs（268 行，1 个测试，`#[ignore]`，start_repl_set）

**`outcome_product_ref_freezes_snapshot_and_survives_later_price_change`（outcome_snapshot_freeze_integration.rs:154-268）**
- 不变量（写侧产品快照冻结红线，models.rs:432-435 注释引用）：approve 带 productId → 真实走 `add_outcome_event_inner`（shared.rs:1403，经唯一公开 caller approve_suspected_deal 驱动——**不是测试手工构造快照**，outcome_snapshot_freeze_integration.rs:9-16）从 active 产品表解引用冻结 OutcomeProductRef（name/unit_price/sku/quantity 默认 1/entitlement_days 从 attributes）；**改价改名后历史快照不漂移**（订单式冻结、非活引用——19900 不变成 99900；产品下架也不丢已购客户时效）。
- 已声明局限：approve 路径 quantity 恒 None → 快照断言 ==1。

## 3. 不变量总表（系统承诺清单）

以下是本组全部测试锁定的行为承诺，按主题浓缩（每条后括号内为主要锁定文件）：

### 3.1 网关决策主链
1. **LLM 调用次数恒等式**：直发 = Reply+Review+ClaimGate 3 次；single-shot revision / ClaimGate targeted-rewrite = 6 次；rewrite 后仍违规再 +1 次中性占位生成 = 7 次；不回复 = 1 次（不进 Review）；知识 tool-loop = 6 次（happy_path_run、full_flow_suite）。
2. **Reply Agent 调用硬上限 2 次**（1 首轮 + 至多 1 revision）；二轮仍失败或 Skip 前置（revisionDirection 空/预算超）→ revision_failed + should_reply=false（autonomy_protocol_pbt P2）。
3. **未执行独立 Reviewer 绝不发送**：本地 fallback 在任何预算/needs_review 组合下 approved=false；预算超额保留 budget_exceeded_no_review 合同、非预算路径是可审计 hold（required_reviewer_not_executed）（autonomy_protocol_pbt P3）。
4. 决策 JSON 的 12 个自治协议字段缺失/枚举非法必产生 risk 标签；conversationMode 严格四值枚举，非法落最保守默认 casual_relationship；tool_calling 中间轮与 sunset 路径跳过校验（autonomy_protocol_pbt P1、conversation_mode_decision_schema）。
5. **评审阈值语义**：humanLike 闸是 `<`（threshold 本身通过）；pressureRisk 闸是 `>=`（threshold 即拦）且 0 是 legacy 豁免；双闸 AND；approved=false 一票否决（human_like_threshold_pbt、pressure_risk_threshold_pbt）。
6. **无来源开放世界业务事实**：ClaimGate 标 unsupported → 一次 targeted rewrite 机会；仍违规 → blocked_by_safety_guard，但客户仍收到一条不含违规内容的中性占位（客户回应保障）（full_flow_suite A1b/A1c）。
7. **性能可观测合同**：gateway 返回前必须持久化 performance 子文档（path.kind 稳定分类 direct/revision、llmLogFlush queued=persisted、8 个阶段耗时键）且 llm_call_logs 同步落库（full_flow_suite A1/A2）。
8. **Run Envelope**：信封写入先于任何 LLM 调用；同 run_id 不可重插（unique 索引）；信封丢失走 insert 兜底 + recovered 审计；panic 保留 payload 且按决策前后落 failed_before_decision/failed_after_decision；终态更新原信封不新插（run_envelope_integration）。
9. **缺失 state policy fail-closed**：有 current 状态机但缺 current state policy → 回复路径与手动管理发送路径都在 outbox/MCP 前拦停（sr072_policy_fail_closed）。
10. Shadow 模拟零业务副作用（全库逐文档快照相等，只留 run_mode="shadow" 的成本日志）；dry-run 会话只写 dry_run 状态审计（simulation_no_sideeffect、dry_run_isolation）。

### 3.2 去抖/抢占
11. 同去抖窗口多条入站聚合成一次 gateway 运行、一条 outbox、3 次 LLM；决策使用**最新**入站快照；晚到入站 bump generation 不重复 spawn、不丢失（debounce_pipeline_integration）。
12. barge-in guard 在落盘/入队检查点返回 true → superseded_by_new_inbound、0 outbox、**last_agent_run_at 不推进**（保证重算不被误判 rate_limited）；重算后最终只发一次（debounce_barge_in_run、full_flow_suite B1-B3）。
13. 静默时段与普通 debounce 共用同一条 `inbound_reply` 持久义务（幂等、永不过期、gateway_status=quiet_hours_waiting）；旧 deferred_inbound_reply 不再产生（quiet_hours_deferral）。

### 3.3 状态机
14. `check_state_transition` 允许 iff：无 domain_config（兼容）/ 目标 allowFromAny / from ∈ allowedFrom / 空 from→initial:true 态；**有 config 但状态机空 → fail-closed（state_machine_empty）**；**未知目标 fail-closed（unknown_target，防幻影态旁路 policy）**；拦截理由恒以 state_transition_invalid 开头（字符串协议）（state_transition_pbt、string_fact_risk_guard）。
15. 引擎行业无关：读 initial/allowFromAny/allowedFrom 标志，不写死销售态名；两域状态空间互为 unknown_target（c2_state_transition_cross_domain）。
16. C2 派生：operation_state 优先派生自 customer_stage（回落 decision.operation_state 仅当 stage 缺失）；非法迁移 **fail-soft**——跳写留旧值 + `agent.operation_state_transition_rejected` 审计 + reply 照常放行；customer_stage 字段自身也过同一状态机（两字段不漂移，各记对称审计）；审计写失败不得吞回复（c2_operation_state_derivation_e2e）。
17. stage 证据分级：弱证据（无显式意图）→ 落 tag_observation 暂定层不写 domain_attributes；强证据（Inbound+显式）→ 实时写不落暂定层（c2_operation_state_derivation_e2e D7-F1）。
18. intent_trajectory 滑窗 cap=50、FIFO 丢最旧、新条目恒在尾（intent_trajectory_pbt）。

### 3.4 outbox/发送（送达红线）
19. **同 idempotency_key 至多一次物理发送**；key v2 含 workspace（同业务身份每 workspace 一次）；媒体/名片条目 key 含 asset_id/card_id（outbox_integration、hc004_outbox_webhook、media_asset_send、referral_card_push）。
20. **回执三分法**：业务回执 ok=false → 重试且不写 outbound record；无可信回执/请求已越远端边界的 HTTP 失败 → delivery_unknown 不自动重放（名片类尤其：无权威 post-hoc 查询，崩溃恢复也停 delivery_unknown）；明确初始化失败（未发出）→ 正常重试至 max_attempts 后 failed_terminal（outbox_integration）。
21. **取消语义**：用户 stop 取消同 contact 全部 pending；claim 后未越边界的 stop 赢得最后 CAS（旧快照 owner 不发送）；已越边界的迟到取消是 best-effort——成功回执落定 sent、保留 cancel_requested 审计、不重放（outbox_integration）。
22. **崩溃恢复**：lease 过期 reclaim 回 pending 由他人接手；reclaimed_in_flight 行必须**先过 reclaim 幂等门（权威 chat_search 优先、本地 mcp_call_logs 兜底）再过 pacing 闸**——已发过的标 sent 不重发、不得被 pacing 拦成僵尸（outbox_integration reclaim 系列）。
23. **门序与节流**：账号级拟人间隔闸 reschedule 不耗 attempt、按 account 隔离、无历史 fail-soft；日发送量软上限只告警绝不拦截；账号掉线 defer 不耗 attempt 不调 MCP；30 分钟陈旧条目被安全门取消（outbox_integration、account_offline_defer）。
24. **任务发送 fencing**：decision 批次未封口（outbox_enqueuing）不发送；陈旧 task claim 的 outbox 被取消（stale_task_claim）0 发送；同 claim 授权恰一次发送；cancel_for_decision 拦 pending+in_flight（sr034）。
25. **持久入站交接**：pending handoff 崩溃后 reconcile 恰物化一次任务（幂等）；后到入站按 created_at（非 ObjectId 序）刷新单飞任务并 fence 旧代际（旧 outbox 取消、0 发送）；崩溃后恢复可收养旧 decision 的可恢复行恰发一次，但运营取消行与已越边界行**不复活**（sr177）。
26. 混合 run 终态 outbox_status="partially_sent" 与处理顺序无关；管理端 cancel 带错账号 → Conflict 零写（outbox_integration、outbox_scope_integration）。
27. 台账（send_ledger）：outbox_id 锚全局唯一（一次投递不可归因两账号）；重放幂等；共享 wxid 的回复/进展不跨账号归因；对账审计报错不改写历史（send_ledger_integration）。

### 3.5 多租户隔离（本组内与主链路相关部分）
28. 同 account_id 跨 workspace：webhook 限流桶、pacing 历史、幂等键、reaction stop 取消范围全部按 (workspace, account) 隔离，外域文档 BSON 逐字节不变（hc004_outbox_webhook）。
29. 成交归因/digest 可见性/dismiss/评审路由/素材写动作/任务动作/deal 登记/campaign：跨 workspace → NotFound、错 accountId → 400/404/409（Conflict）且零写零审计（hc004_scope_redlines、media_asset_crud SR-160、review_task_now SR-155、deal_event_scope、campaign_dispatch、sr172）。
30. outbox 管理列表投影：跨账号资产引用不泄漏元数据（title/fileName=null）、外账号行不出现（sr172）。

### 3.6 记忆
31. memory card compact：core ≤6、recent ≤10、销售槽/自定义维度各按 cap；未 discarded 的旧事实必可追溯（core/recent/eviction 审计三处之一），任何候选不得静默消失；discarded 绝不复活；结构化事实按 importance/confidence 稳定排序与输入顺序无关；维度集合随 profile（memory_card_invariants）。
32. 首触达并发 create：输家回落 find_one 不透传 E11000（不因 dup-key 毁掉整轮 run）；库里恰一行（operating_memory_insert_idempotent）。
33. 记忆固化两阶段提交：单飞持久任务（活跃 owner 冲突显式 Conflict 且零改动零 LLM）；prepared_commit 三种崩溃窗口重放恰一次（memory_card_version=1、审计按 dedupe_key 恰一条）（sr029）。
34. 运营者记忆：撤销后不再注入；跨 scope 撤销与不存在不可区分（NotFound）零写；重复撤销幂等不改首次审计；重加是新行、审计行保留（sr181）。

### 3.7 反应
35. 反应分析 claim 互斥：活跃 claim 期间第二入口 0 次 LLM；超时 reclaim 换代后**过期 owner 的结论不覆盖现任、不取消 pending outbox、不追加轨迹**（reaction_claim_lock）。
36. stop 串联：stopRequested → 同 contact pending outbox 全取消（user_reaction_stop_requested）；**显式停止语零 LLM 也生效**（不依赖模型可用性）且持久化 cooldown_until + explicitStopRequested 屏障（reaction_stop_cancels_outbox）。
37. 确定性买入底线是交易 profile 特性：需已 sent 前置；否定语句走模型；情感 profile 下同字面走域语义；确定性路径不造 deal/payment 事实（reaction_claim_lock）。
38. 未送达（outbox pending）的回复不得被学成反应标签（outcome_status 停 pending、0 反应 LLM）；commitment/follow_up 只在确认送达后提交（full_flow_suite）。

### 3.8 请示/引荐（"无人工接管"治理红线）
39. 客户可见文案禁词：不含转接/转人工/暴露身份 13 词、不暴露幕后决策源 9 词（"领导决定/拍板"等；"帮你跟领导申请"合规）；兜底安抚文案不含"真人/转人工/客服/接管/人工"（principal_decision_channel 纯函数、real_llm_principal_channel/relay 硬断言）。
40. 请示台账协议：pending→resolved 携结构化 verdict（闭集）；知识沉淀恒 draft+needs_review+共享域（AI 永不自动验证）；awaiting 标记 set/clear 往返、多 owner 数组按 escalation 分别摘除；deferred 不 resolve 不转述保持 pending（principal_decision_channel、ask_human_phase1_e2e）。
41. 超时改派：骚扰门（quiet_hours/daily_push_cap）先于改派、cap 不被本条自我命中；每位决策人从 **sent 对账时刻**起拿完整超时窗（queued 阶段不级联）；并发 scan 经 generation CAS+幂等键收敛一次；旧代际卡片在远端边界前取消；链尾失联安抚 min_interval 去重；投递终败 → delivery_failed 释放 pending 槽并清 awaiting（principal_decision_channel、ask_human_phase1_e2e）。
42. relay 回路：领导自然语言裁决经 webhook 解析（明确批准 ≠ deferred）→ resolve → relay task 恰一条（重复 resolve 幂等、崩溃恢复恰一条）；授权过期必须清 awaiting + 发不复述过期承诺的中性收尾；泄漏内部字段的转述被安全门拦（0 outbox、awaiting 保持、任务 cancelled）（principal_decision_channel、ask_human_phase1_e2e、real_llm_principal_relay）。
43. webhook 请示路由：领导消息 routed="principal" 不进客户链路；澄清走持久 outbox 幂等一行、handler 绝不直调 MCP（principal_decision_channel）。
44. 跨 workspace resolve：幂等 200 alreadyResolved（不泄漏存在性）但台账不被真正裁决（ask_human_phase1_e2e IDOR）。
45. 名片引荐：可加载 = enabled+approved 双门；名片 outbox 按 card_id 幂等、content 可空、与 media 互斥（referral_card_push）。

### 3.9 campaign/planner/主动触达
46. campaign：命中 0 人拒绝（不静默成功）；受众超上限拒绝（不静默截断）；completed 状态门拒绝重派（圈人前拦截）；spec CAS 版本化、preview 消费最新 spec；终态 preview 零写；task insert 失败保留 prepared 意图由 reconciler 恢复（同冻结身份、恰一次）；命中数与去重入队数分账（campaign_dispatch）。
47. planner：只扫 managed+静默、排除 normal/cooldown；emit 的 follow_up 保留 review_required（主动触达仍走完整评审）；每 tick 恒写 tick 事件；幂等不重复 emit；commitment 只对 overdue/imminent 且 Plain 无 due 不计；calendar 只在 profile 声明 date_dimension 时运行（销售域零扰动连 tick 都不写）、同日同纪念日稳定身份去重（task 终态后也不重发）；block-rate 超阈值（且 ≥min_runs）→ 该 contact 静默退避（不 emit、写 backoff 事件、不耗 cap）（planner_* 四文件）。
48. 主动触达统一提交：并发同 intent 恰一 Emitted 其余 Duplicate（绝不重复消费配额）；不同 intent 并发不超日配额；UTC 日桶隔离；段/总双 cap 持久化；滚动升级基线追认；**task+event+quota 三写事务性（event 失败全回滚）**；行为信号同协议（sr135）。
49. 冷重激活唯一可发条件 = Managed+outbound 老于阈值+无 inbound 反超+无 cooldown+无 pending；非 managed 永不触达（cold_reactivation_idempotent_pbt）。

### 3.10 管理流/准入
50. 批量托管：sharedNote 必填、账号需注册、pool 候选跨账号整批 Conflict 零写、非人类 wxid 拒绝（也不经 webhook 建档但消息保留）；task 意图先于 managed 可见（insert 失败无半提交）；并发单飞恰一 intent；enrollment_token 代际轮换自修复；全新客户同步落 initial 态而老客户状态不被覆盖；已 managed 不重复入队；initial_profile 早退必写终态不停 running（contacts_batch_enable、webhook_contact_upsert）。
51. roster 富化：nickname/avatar 只来自 roster 快照（绝不取 payload 账号自身昵称）、未命中留 None 不阻断建档；webhook 新建 contact 默认 Normal（webhook_contact_upsert）。
52. 管理命令协议：模型写计划冻结在 planHash+账号 CAS+认证管理员确认之后（错账号/篡改 hash → 409）；恰 1 次规划 LLM；越过副作用边界的死租约 → execution_unknown 绝不重放 MCP（hc020）。
53. 疑似成交闭环：**AI 永不直写 outcome**——approve 才落 verification="staff_confirmed"；审批+成交+审计同事务（审计失败全回滚）；预检先于 CAS（非法金额留 pending 可重试）；重复审批 CAS 冲突不双计；部分唯一索引让 approved 历史不阻塞二次成交新 pending；reviewedBy 伪造被忽略（suspected_deal_e2e、deal_event_scope）。
54. 产品快照订单式冻结：成交时从产品表解引用，改价改名下架不漂移历史（outcome_snapshot_freeze）。
55. runtime 参数类型化边界：未知键 400 零写；legacy 别名归一到 canonical 键；Guide 高风险输出不留 apply 能力；workspace 级影响需强确认；apply 事务性 + 稳定 receipt 重放；Preview 不产生副作用（sr094、transactional_admin_flows guide_apply）。
56. taxonomy/关系审批事务：字典 insert 失败回滚候选；merge 保留运营字段只并 aliases、版本 lineage 正确；reviewed_by 恒来自认证会话（SR-058）；关系审批建议+contact 同事务、部分唯一索引一 pending 周期；同 run 内 Bayesian 重复观察坍缩一点、未知 stage 恰一候选（transactional_admin_flows）。
57. 自治指标端点：无数据比率为 null；`held_for_human` 脏值剔除不计（旧命名不复活）；各率分子分母定义精确（outcomes_autonomy_endpoint）。

### 3.11 支撑
58. 多账号轮询：同 wxid 稳定命中同账号；off_hours 有替代则排除；capacity=0 无限量；全满 fallback 保送达（account_round_robin_pbt）。
59. 行为信号：dedupe_key 幂等（重放坍缩）；同 inbound 三类信号共存；沉默恒 censored=true（删失非负例）、T1 信号 source=system_observed & confidence=1.0；silence worker 多轮 tick 幂等（behavior_signal_*）。
60. LLM 重试：指数退避 + Retry-After 取大者；429/5xx 重试、400/401/JSON 解析失败不重试（llm_retry_jitter）。
61. last_inbound_at/last_outbound_at 拆分：入站/出站互不覆盖、last_message_at=max（last_inbound_split）。

## 4. 测试基础设施观察

- **TestApp 工厂（state-only）**：`tests/common/mod.rs` 只建 testcontainers Mongo（mongo:5.0.6）+ 同形 AppState，**没有 HTTP server**。三种驱动形态并存：①直调 agent 公共函数（handle_managed_message 等）；②直调 route handler 真函数（axum extractor 形参直接构造，错误断言 `Err(AppError::…)` 变体而非 HTTP 状态码——media_asset_crud_integration.rs:6-10 声明为"本仓既有惯例"）；③少数用例真起 axum server（TcpListener 随机端口 + api_router + create_session cookie + reqwest）验证含中间件的全链（sr172/hc004_scope/hc020/sr094/transactional_admin_flows）。
- **事务测试用 start_repl_set**：多文档事务只能在 replica set 提交（standalone 无法 commit，common/mod.rs:401-409）；planner 系列也用 repl_set。个别深栈测试自建 32MB 栈线程 + 独立 runtime（sr072/sr094/hc020）。
- **启动序对齐生产**：migrations::run → ensure_indexes → 补 seed m006 taxonomy（m012 在非 production 会删 seed）→ ensure_prompt_pack_v2 → 预热 taxonomy 与 DomainProfile 两个进程级 LazyLock+30s TTL 缓存（不预热会命中同 binary 内上一个测试的残留 active profile，common/mod.rs:462-524）。planner_calendar_care 因缓存单例用 `#[serial]`。
- **mock LLM 按 schema 定向消费**：TestLlmGenerator 不是纯 FIFO——按响应 JSON 顶层指纹（decisionPhase→Reply、approved+scores→Reviewer、requiresEvidence+claims+catalogClaims→ClaimGate、action→Knowledge）与请求 system prompt 关键词匹配，解决 Reply/Reviewer/ClaimGate 并发调度不确定性（common/mod.rs:212-284）。**每轮 Review 后 gateway 都会再调一次 ClaimGate**，集成测试必须显式排入 ClaimGate fixture（common/mod.rs:288-291）——大量测试的 push 序列都以 `independent_claim_gate_pass_json()` 收尾。
- **决策/评审 fixture 高度模板化**：reply_agent_decision_json（完整 R1.3+R3.2 字段）与 review_agent_pass_json（八维分数+claimAnalysis+revision/hold 字段）在 ~10 个文件中复制粘贴微调——它们同时是"决策 JSON schema 的活文档"。
- **MCP 桩形态**：wiremock 统一 POST /mcp；**UniqueMsgIdResponder 是必须的**（conversation_messages.message_id sparse+unique 索引，重复 newMsgId 会 E11000 使投递重置 pending，outbox_integration.rs:98-124）；按 tool 名分派的 responder（chat_search 命中/500）；自建 axum BlockingMcpServer 用 watch+Notify 精确控制"远端已收到未回执"时刻（outbox_integration.rs:262-355）。真模型套件用 rebuild_app_state_with_real_llm（MCP 恒为桩，绝不真发微信）。
- **故障注入惯用法**：MongoDB collection validator（collMod + validator + validationAction:error）定点让某类 insert 确定性失败——审计写失败（c2 e2e、sr135、suspected_deal）、task insert 失败（campaign、contacts_batch_enable）、content_assets 失败（media_storage）；测后 collMod 关闭。时间类注入用回拨 created_at/locked_until/claimed_at/updated_at。
- **等待辅助**：wait_for_outbox_processed（按 _id/run_id 轮询终态 sent|failed_terminal|canceled|delivery_unknown）；complete_latest_post_decision——**画像/taxonomy/Bayesian/stage 写入已抽到异步 post-decision projection worker**，凡断言 contact 投影效果的测试都要 push 投影 JSON 并跑 run_post_decision_worker（common/mod.rs:776-849）。
- **真模型套件纪律**（real_llm_principal_*）：env-gated 自我跳过；FailoverProvider 备胎链只救端点抖动不抬被测分；unwrap_or_skip_transient! 区分瞬时（skip+ledger）与配置 4xx（panic 堵假绿）；断言分级——确定性红线硬断言（禁词/闭集）、模型行为软观测（escalation 触发与否记 ledger issue）、judge 打分 ObserveOnly。
- **禁词测试的 lint 豁免设计**：check-no-human-takeover 显式排除 tests/，所以红线测试能在测试里写出禁词字面量来断言生产文案没有它们（principal_decision_channel.rs:1615-1617、real_llm_principal_channel.rs:442-447）。
- **测试隔离**：每 TestApp 随机库名；debounce 的 static PENDING 跨测试共享 → 每测试唯一 wxid；media 文件落盘用进程内唯一临时目录。

## 5. 偏差与疑点

1. **两个空壳测试（永远绿）**：`revision_recheck_action_gate.rs:24-37`（GATE-1 revision 后动作闸复检）与 `memory_card_write_occ.rs:14-23`（CONC-1 memory_card OCC）函数体为空/仅 `let _ = &app;`，不变量只写在注释里，作者声明正确性由"代码审查 + lib 基线"保证。这两条主链路安全不变量目前**没有可执行守护**。
2. **worker_reclaim.rs 名不副实**：`stale_running_task_is_recovered_to_retry` 实际只 insert 后断言 status=="running"（worker_reclaim.rs:62-72），没有驱动任何 reclaim；文件注释自认弱化（worker tick 私有）。HP-1 stale 回收的端到端行为在本组内未被真正测到。
3. **"复刻式"测试与生产可能脱节**（作者大都显式声明，但仍是漂移风险）：decision_review_status_e2e 复刻 fetch_run_status 关联逻辑；media_asset_send / referral_card_push 复刻 load_sendable_assets / build_referral_cards_filter 查询条件；campaign_segment_coverage 复刻粗筛 $elemMatch；escalation_push_time_reassign 用 raw $set 模拟 reassign 写法；last_inbound_split 两个测试自己发 update 语句（锁定写法约定而非生产函数）；dry_run_isolation 自己 insert 审计行。生产侧改动这些逻辑时上述测试**不会自动变红**。
4. **autonomy_protocol_pbt P2 是模型测试**：run_revision_loop 是测试内手写的 gateway 控制流复刻（对照注释引 gateway.rs:706-924 为写作时快照行号）；它验证模型自身性质，若生产 R2 控制流漂移不会失败。P1 的 invalid_type(kind=2) 分支实际退化为 missing-field 注入，未真正覆盖类型错误。
5. **Multipart 端点无法在集成层测**：`replace_content_asset_file` 的"清 media_id + 退 draft"副作用明确声明由代码审查保证（media_asset_crud_integration.rs:12-18）——与 1 类似的"审查代测"缺口。
6. **注释中的生产行号是快照**：大量测试注释引用 gateway.rs:1845/1463/2726/5199、reaction.rs:248、shared.rs:1403 等行号，git status 显示 src/agent/* 有未提交修改，这些行号可能已漂移（本记录原样转述并标注"注释引"）。
7. **real_llm 套件的诚实降级**：escalation 是否触发、awaiting 是否清除等模型依赖行为只软观测不硬断言（作者有充分论证）；意味着"超职权必走请示"并没有确定性 CI 守护，只有"要么请示要么 fail-closed"的弱化终局断言（real_llm_principal_channel.rs:774-777）。
8. **ignore 生态依赖 CI**：本组约 9 成集成测试 `#[ignore]`，本地 `cargo test` 只跑纯函数 PBT/单测；行为守护实际生效点在 CI 的 `--ignored` integration job（与 CLAUDE.md 的本地/CI 分工一致，但意味着本地改动的快速反馈只覆盖纯函数层）。
9. **弱断言点**：happy_path_run tool-loop 尾部 `let _ = (outbox, Duration::from_secs(10));`（happy_path_run.rs:623）疑为残留无效代码；planner_calendar_care 用 `#[serial]` 但其它共享进程级缓存的测试（如 c2 系列依赖 DomainProfile 缓存预热）未 serial——靠"每测试独立 DB + 预热对齐"维持，若未来测试 seed 自定义 active profile 可能串扰（common/mod.rs:514-524 注释已提示该机制）。
10. **git status 提示**：本组多份测试文件（sr094_runtime_parameters、transactional_admin_flows、reaction_claim_lock、reaction_stop_cancels_outbox、roleplay_fixtures 等）处于已修改未提交状态，本记录基于工作区版本；与 main 分支行为可能不同。

## 6. 覆盖自证

**已深读文件：71 个**（70 个测试文件 + 1 个基础设施 mod.rs），每个文件完整读完（含全部 #[test]/#[tokio::test] 函数体）：

common/mod.rs；happy_path_run、full_flow_suite、decision_review_status_e2e、revision_recheck_action_gate、autonomy_protocol_pbt、human_like_threshold_pbt、pressure_risk_threshold_pbt、debounce_barge_in_run、debounce_pipeline_integration、quiet_hours_deferral、run_envelope_integration、conversation_mode_decision_schema、string_fact_risk_guard、sr072_policy_fail_closed（网关/决策/评审 14）；state_transition_pbt、c2_operation_state_derivation_e2e、c2_state_transition_cross_domain、intent_trajectory_pbt（状态机 4）；outbox_integration、outbox_scope_integration、send_ledger_integration、sr034_task_send_fencing、sr172_outbox_projection、sr177_durable_inbound_handoff、hc004_outbox_webhook_scope_redlines、hc004_scope_redlines、account_offline_defer_integration、account_round_robin_pbt（outbox/发送 10）；memory_card_invariants、memory_card_write_occ、operating_memory_insert_idempotent、sr029_memory_commit_recovery、sr181_operator_memory_revocation（记忆 5）；reaction_claim_lock、reaction_stop_cancels_outbox_integration（反应 2）；principal_decision_channel、ask_human_phase1_e2e、referral_card_push_integration、real_llm_principal_channel、real_llm_principal_relay、escalation_push_time_reassign、hc020_management_command_protocol（请示/引荐 7）；campaign_dispatch_integration、campaign_segment_coverage、planner_block_rate_backoff、planner_calendar_care、planner_commitment_due、planner_silent_followup、cold_reactivation_idempotent_pbt、sr135_proactive_outreach（campaign/planner 8）；media_asset_crud_integration、media_asset_send_integration、media_storage_consistency（media 3）；sr094_runtime_parameters、transactional_admin_flows、contacts_batch_enable（运行参数/管理流 3）；webhook_contact_upsert_integration、last_inbound_split（入站 2）；simulation_no_sideeffect_integration、dry_run_isolation（仿真 2）；behavior_signal_idempotent_pbt、behavior_signal_smoke（行为信号 2）；worker_reclaim、review_task_now_claim（worker 2）；suspected_deal_e2e、deal_event_scope_integration、outcomes_autonomy_endpoint、outcome_snapshot_freeze_integration、outcome_task_workspace_dedupe（deal/outcome 5）；llm_retry_jitter（LLM 支撑 1）。

合计约 **290 个测试函数**逐一核证。R11.6 基线四 PBT 文件中的 3 个（state_transition_pbt、memory_card_invariants、llm_retry_jitter）在本组已读；第 4 个 wiki_chunk_revision_pbt 属知识主题，留给 16 号。

**未读、留给 16 号记录的文件（约 111 个）**，按主题：
- 知识库/wiki/import/digest：knowledge_*（agent_eval/agent_pbt/ask_e2e/ask_stream_e2e/auto_verify_enforce/chat_apply/chat_dispatch/chunk_transactions/closed_loop_trajectory/digest_budget_smoke/digest_compose_smoke/digest_skeleton/import_apply/operator_memory_isolation/preview_workspace_scope/router_fallback_e2e/task_worker/tools_budget/worker_behavior）、chunk_*（batch_ops/lock_lifecycle/put_preserves_unmodeled_fields/revision_ai_draft/type_routing_pbt）、wiki_chunk_revision_pbt、wiki_gap_signals_3kinds、import_job_lifecycle、import_pdf_smoke、ingest_worker_smoke、page_merge_pbt、integrity_report_d2_e2e、annotation_quality_gate_integration、structured_organization_integration、sr115/sr117/sr121/sr122/sr125/sr131/sr132、hc026_formula_evaluation、hc028_real_digest_task_e2e、digest_cross_tenant_scope_integration、vision_safety_gate、maycran_transport_probe
- evolution：evolution_*（policy_router/prompt_shadow/release_redline/rollback_status/workspace_scope）、m040_evolution_release_protocol、reset_pack_preserves_evolution_critic_integration、sr097_lesson_promotion、lessons_learned_filters
- prompt/soul：prompt_pack_seeding、prompt_publish_evolution_guard、prompt_template_redline_gate_e2e、sr053_soul_versions、sr055_prompt_versions、sr138_prompt_reset_guard、m049_prompt_planning_currents
- auth/租户：auth_middleware_integration、jwt_auth、sr016_auth_rate_limit、h3_cross_tenant_idor、account_security_integration、sr012_runtime_scope、sr174_cache_database_isolation、sr176_real_route_isolation、workspace_isolation、products_workspace_isolation、playbook_scope_integration
- eval/roleplay/real_llm 其余：judge_rubric、roleplay_*（emotional_companion_e2e/fixtures_smoke/reviewer_pressure_calibration）、real_llm_*（adversarial/cross_domain_arc/digital_twin_arc/dynamic_adversarial/knowledge/knowledge_quality/ops_smoke/proactive_outreach/progressive_tier/recall_benchmark/roleplay_arc/smoke）、identity_generator_smoke、dynamic_smoke、redline_smoke、common_smoke、common/ 其余子模块（judge/roleplayer/roleplay_fixtures/redline/dynamic/identity_generator/generalization/capability_evidence）
- migration/domain/contact/其它：migrations_idempotency、migration_safety_redlines、ops_versioned_index_boot_brick、m018/m029/m034/m039/m045、domain_profile_e2e、domain_schema_persistence_e2e、contact_manual_tags_integration、contact_operation_profile_integration、operation_view_integration、configuration_generation_integration、guide_apply_partial_validation、taxonomy_flags_e2e、taxonomy_version_audit_integration、llm_provider_activate_integration、llm_usage_summary_integration、escalation 相关已读、sr008_ops_single_current。

---

## 七、28 号交叉验证回写修正（2026-08-13，主会话执行）

28 号测试-生产一致性验证（71 条承诺全映射，亲读双侧源码裁决）对本记录的修正：

1. **§3 承诺 2（A2）修正**：'Reply Agent 调用上限 2 次'仅指 revision 子流程（P2 模型口径）；生产 targeted rewrite + revision 串联可达 3 次。且 autonomy_protocol_pbt 的 P2 模型**缺 `apply_revision_fallback` 分支**——其"二轮失败恒 revision_failed"断言与生产相反（`gates.rs:1258-1275` 亲证：纯风格失败可回退原稿 approved）。**P2 是复刻漂移实锤**。
2. **§5 疑点 3 升级为核证结论**：`escalation_push_time_reassign` 用例 1 **已实质漂移**——测试锁 reassign `$set last_pushed_at_ms`，生产为 `$unset`（`ledger.rs:1111-1114`，改派清空、确认送达才重置，主会话亦亲证）；`last_inbound_split` 出站写法字面分叉（测试 `$cond/$gt` vs 生产 `$max`，`gateway.rs:5184-5186`，语义等价不算漂移）；其余 4 处复刻（decision_review_status/media_asset_send/referral_card_push/campaign_segment_coverage）当前逐键一致。
3. **§5 疑点 4 补充**：P1/P4 锁定的 `validate_and_promote` 完整契约在生产主链已被 `validate_reply_critical`（fast 契约）取代（`decision.rs:1345`）——完整校验主链不消费，承诺 4 覆盖面限定为"仅 fast 契约字段"（与总台账偏差表 #2 呼应）。
4. **§2.56 dry_run_isolation 补注**：用例 2 的 `status="completed"` 是**幻影值**——不在生产闭集 `ALLOWED_AGENT_COMMAND_RUN_STATUS`（`models.rs:4063-4071`，生产终态为 succeeded）；两用例均未驱动生产 dry-run 分支。
