
# Implementation Plan: 用户运营 Agent 自治回路（agent-autonomy-loop）

> **⚠️ Sunset Notice (2026-05-25)**：本任务清单写于销售域知识库时代，其中针对 `customer_stage` /
> `intent_level` / `objection_type` / 5 闸阈值（fact_risk / pressure_risk / product_accuracy
> 等）/ `safe_claims` / `routing_card` 的任务条目对应的代码已在 knowledge-cleanup 中下线，
> 详见 `requirements.md` 顶部 sunset notice。本文件保留作历史档案。

## Overview

本实施计划严格遵循 design.md §2.2 的 6 波 + 1 收口顺序（W0 基础设施 → W1 协议骨架 → W2 校验/安全门 → W3 工具/字典 → W4 Outbox → W5 MemoryCard → W6 监控/PBT 收口）。每个任务都是可由代码生成 LLM 增量执行的具体编码步骤，引用具体的需求条目（granular sub-requirements）；性质测试任务标注对应的 P1–P7 性质编号与所验证的需求子条款。

实现语言：Rust（后端）+ TypeScript/React（前端 Tab）。性质测试基于 `proptest`（沿用现有 `tests/state_transition_pbt.rs` 等惯例）。

## Tasks

- [x] 1. 基础设施波（W0）：新建 collection、索引、迁移脚手架与 runtime 参数
  - [x] 1.1 在 `src/db/mod.rs` 新增 3 个 collection accessor
    - 新增 `agent_send_outbox / system_taxonomies / taxonomy_candidates` 三个 `pub fn collection_<name>(&self) -> Collection<T>` 入口，类型分别绑定到 `OutboxEntry / TaxonomyEntry / TaxonomyCandidate`（先在 `src/models.rs` 写 struct 占位，字段最终值在 W3/W4 落定）
    - _Requirements: 8.1, 8.3, 13.1_

  - [x] 1.2 在 `src/db/indexes.rs` 创建 collection 索引
    - `agent_send_outbox`：复合 `(account_id, status, next_retry_at)` + 唯一 `(idempotency_key)` + 复合 `(status, locked_until)` + 复合 `(source_event_id, contact_wxid)`
    - `system_taxonomies`：唯一 `(scope, kind, value.id)`
    - `taxonomy_candidates`：复合 `(scope, kind, status)` + 唯一 `(scope, kind, raw_value)`
    - `agent_run_logs`：补建 `(account_id, lifecycle, started_at)` / `(account_id, finalReviewStatus, started_at)` / `(account_id, autonomyMode, started_at)`
    - _Requirements: 0.8, 8.1, 9.5, 13.1_

  - [x] 1.3 在 `src/models.rs` 与 `src/config.rs` 扩展 `RuntimeParameters`
    - 新增 `autonomy_protocol_enabled: bool`（默认 true）、`knowledge_routing_mode: String`（默认 `auto_tool_loop`）、`knowledge_max_tool_loops / knowledge_max_tool_calls / knowledge_open_slice_max_k / knowledge_search_top_k / outbox_poll_interval_seconds / outbox_lease_seconds`，并在 `RuntimeParameters::default()` / loader 中给出默认值与 clamp
    - _Requirements: 4.2, 4.3, 4.6, 11.3, 11.4, 13.3_

  - [x] 1.4 在 `src/db/migrations.rs` 注册 3 条迁移脚手架（仅占位，不写实质 logic）
    - `2026_05_005_memory_facts_to_structured`（W5 实现实体逻辑）
    - `2026_05_006_taxonomy_seed`（W3 实现实体逻辑）
    - `2026_05_007_outbox_indexes`（确保索引创建幂等，W0 即可生效）
    - 三条迁移皆 SHALL 幂等：同 `migration_id` 二次启动 SHALL skip，不报错
    - _Requirements: 6.4, 8.8, 11.7, 13.1_

  - [x] 1.5 新建 CI 基线核验脚本
    - 创建 `scripts/check-baseline.ps1`（Windows）+ `scripts/check-baseline.sh`（Linux/CI），实测 `cargo test --lib >= 78` 与 4 个 PBT 文件累计 `>= 33`，任一不达标即 `exit 1`
    - 在 `README.md` 引用方式段落补一行运行说明（不创建新 markdown 文件）
    - _Requirements: 11.6_

  - [x] 1.6 为 W0 新增 collection accessor 写最小 smoke 单元测试
    - 测试启动期 `ensure_indexes` 返回 OK；在内存中 mock 一条 `OutboxEntry / TaxonomyEntry / TaxonomyCandidate`，能往返 BSON 序列化
    - _Requirements: 8.1, 13.1_

- [x] 2. 协议骨架波（W1）：删除 Rust 业务兜底、引入 Raw 双层结构、Run Envelope
  - [x] 2.1 删除 `src/agent/guards.rs::normalize_decision_runtime` 中"补默认值"逻辑
    - 仅保留"白名单枚举校验 + planner 同步"语义；`risk_level / knowledge_need / run_mode / needs_review / consolidation_needed` 任何"未输出 → 填默认值"分支 SHALL 整段删除
    - 同步修复所有依赖该兜底的下游调用方（gateway / review / 测试），改为依赖后续 R3.5 的 review 失败路径
    - _Requirements: 3.6, N1_

  - [x] 2.2 在 `src/agent/types.rs` 新增 `RawAgentDecision` 反序列化结构
    - 字段全部 `Option<T>`，覆盖自治协议 9 字段、`risk_level / knowledge_need / run_mode / autonomy_mode / needs_review / operation_state / consolidation_needed / decision_phase / tool_calls / agent_generated_signals`，以及既有 `reply_text / should_reply / used_knowledge_ids / safe_claims_used / knowledge_route` 等
    - 同步给 `AgentDecision` 业务结构补齐 9 字段 / `autonomy_mode / decision_phase / tool_calls / agent_generated_signals`，全部为非 Option 字段
    - _Requirements: 1.1, 3.1, 4.1, 8.2, N2_

  - [x] 2.3 实现 `RawAgentDecision::validate_and_promote(self, runtime) -> (AgentDecision, Vec<String>)`
    - 解析 `decision_phase`（`tool_calling / final`，未填或非法走默认 `final` + risks `decision_phase_invalid:<v>`）
    - `tool_calling` 中间轮：跳过 R1.3/R1.4/R1.5/R1.6 校验，仅做 toolCalls schema 检查
    - `final` 轮：执行 R1.3 7 字段必填、R1.4 互斥必填、R1.5 条件长度（low_routine `unchanged` 短形式 vs critical_turn ≥ 20 字符）、R1.6 回复理由长度延伸、R3.1/R3.2/R3.3 必填 + 严格枚举
    - 违规聚合为 `Vec<String>`（`missing_required_field:<f>` / `invalid_enum_value:<f>:<v>` / `invalid_type:<f>` / `insufficient_detail_in_critical_turn:<f>`）
    - _Requirements: 1.3, 1.4, 1.5, 1.6, 1.10, 3.1, 3.2, 3.3, 3.5, N2_

  - [x] 2.4 新建 `src/agent/run_envelope.rs` 模块
    - `write_run_envelope_started(db, run_id, account_id, contact_wxid, source_event_id, source_kind)`：`insert_one` lifecycle="started"、gateway_status="pending"、final_review_status=""，必须先于任何 LLM 调用（try/catch 之外）
    - `update_run_envelope_terminal(db, run_id, fields)`：用 `update_one({run_id})` + `$set` 落最终字段；`matched_count == 0` 时走单次 `insert_one` 兜底 + 写 `agent_events kind="run_envelope_recovered_via_insert"`
    - `install_panic_hook_for_envelope`：`std::panic::catch_unwind` 包裹 run pipeline，panic 时尽力 update lifecycle 为 `failed_before_decision / failed_after_decision`，写 `error_summary="unhandled_panic: ..."`，二次失败仅 `tracing::error!`（避免 panic-in-panic）
    - 把 `src/models.rs::AgentRunLog` 扩字段：`lifecycle / source_event_id / source_kind / error_summary / abort_reason / revision_applied / revision_reason / pre_revision_summary / post_revision_summary / self_critique / autonomy_mode / final_review_status / outbox_status / memory_consolidator_warnings`
    - _Requirements: 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 9.1_

  - [x] 2.5 把 `write_agent_run_log` 改写为 `update_one`，禁止 re-insert
    - `src/db/mod.rs`（或对应文件）中现有 `insert_one(agent_run_logs)` SHALL 改为 `update_one({run_id}, $set)`；新增 `assert_final_review_status_valid` 与 `assert_gateway_status_valid` 在写库前校验枚举
    - 在 gateway 入口第一步调用 `write_run_envelope_started`；主流程结束（含错误路径）调用 `update_run_envelope_terminal`
    - _Requirements: 0.1, 0.2, 0.3, 9.2, 9.3, 9.4_

  - [x] 2.6 为 W1 协议骨架写 lib 单元测试（≥ 6 例）
    - 入口写 envelope 先于 LLM 调用（mock LLM 抛异常前 lifecycle 已 = "started"）
    - lifecycle 状态机不接受非法转换（`completed → started` SHALL panic/error）
    - Reply Agent panic 后 lifecycle = `failed_before_decision` 且 `error_summary` 非空
    - 同 `run_id` 二次 `insert_one` 触发 DuplicateKey；`update_one` 不存在时走兜底 insert + `kind="run_envelope_recovered_via_insert"`
    - `decision_phase="tool_calling"` 时即使 9 字段全空也不触发协议违规；`decision_phase="final"` 时按 R1.3/R1.4/R1.5 完整校验
    - `risk_level="critical"` 触发 `invalid_enum_value`；`autonomy_mode="manual"` 触发 `invalid_enum_value`
    - _Requirements: 0.10, 1.11, 3.11, N2_

- [x] 3. 校验 + 最终安全门波（W2）：finalize_review_for_send / local_decision_review 二态 / R5 verified knowledge
  - [x] 3.1 改造 `src/agent/review.rs::local_decision_review` 为二态
    - `budget.is_exceeded() && needs_review == true`：返回 `approved=false`、`risks=["budget_exceeded_no_review"]`（autonomy_mode 在 finalize 阶段强制 blocked）
    - `budget.is_exceeded() && needs_review == false`：返回 `approved=true`、`risks` 追加 `"local_review_low_risk_only"`，autonomy_mode 保持原值
    - 默认（未超额）路径保留 `approved=true`
    - _Requirements: 3.7, 3.8, 3.10_

  - [x] 3.2 实现 `finalize_review_for_send` 最终安全汇总层
    - 在 `src/agent/review.rs` 新增 `finalize_review_for_send(review, decision, runtime, contact, knowledge_runtime, promote_risks) -> (DecisionReviewResult, GatewayStatusFinal)`
    - 顺序执行：R3.5 必填违规 → blocked_by_required_field；R3.7 budget_exceeded_no_review → blocked_by_budget；R5.4 verified knowledge → blocked_unverified_product_claim；R5.3.a/b claim_analysis 缺失 fail-closed 推断；R8 字典 candidate 标记（不阻塞）；R2.6 should_hold + holdCategory 校验
    - 任一硬安全门触发 SHALL 强制 `decision.should_reply=false`、`decision.autonomy_mode="blocked"`，并写对应 `agent_events`（`autonomy_field_violation / budget_exceeded_no_review / product_claim_blocked / claim_analysis_malformed_fail_closed / autonomy_hold_category_invalid`）
    - 任何上游 `approved=true` SHALL NOT 绕过本函数
    - _Requirements: 3.5, 3.7, 3.9, 3.10, 5.3, 5.4, 5.5, 5.6, 5.7, 8.4, 8.6, N3_

  - [x] 3.3 在 `src/agent/types.rs::DecisionReviewResult` 扩字段并校验枚举
    - 新增 `needs_revision / revision_direction / should_hold / hold_reason / hold_category / self_critique_addressed / revision_applied / final_review_status`
    - 实现 `assert_hold_category_valid`：取值仅允许 `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context`，其他值（如 `held_for_human / human_required / waiting_for_human`）SHALL 强制改写为 `held_by_ai_policy` 并写 `agent_events kind="autonomy_hold_category_invalid"`
    - _Requirements: 2.1, 2.2, 2.6, 9.1, 9.8_

  - [x] 3.4 在 gateway 主路径接入 `finalize_review_for_send`
    - 把 `src/agent/gateway.rs` 三分支（budget_exceeded / should_run_review / 默认）的 review 结果统一接入 `finalize_review_for_send`，再决定是否进入 R2 revision 或 outbox enqueue
    - 单 run 内最多触发 1 次 revision；`needs_revision=true && !should_hold && !budget_exceeded && !revision_attempted` 时调用第二次 Reply Agent（30s timeout 控制），二次 review 仍 fail 则 `gateway_status="revision_failed"` + `should_reply=false`
    - 把"是否触发 revision / 触发原因 / pre/postRevisionSummary / selfCritique"写入 `agent_run_logs`
    - _Requirements: 2.3, 2.4, 2.5, 2.6, 2.8, 2.9, 2.10, 2.11, 9.7_

  - [x] 3.5 实现 R5 verified knowledge 强约束细节
    - 复用 `OperationKnowledgeChunk.integrity_status == "verified"` 作为唯一判定，新增 `pub(crate) fn is_verified(chunk: &OperationKnowledgeChunk) -> bool` 内部 helper
    - claim_analysis 缺失或损坏时按 R5.3.a/b 推断（`knowledge_need ∈ {required, insufficient}` / `used_knowledge_ids` 非空 / 触发 `enforce_string_fact_risk_guard` marker hit 任一命中 → 强制视为产品声明 fail-closed → `gateway_status="blocked_by_safety_guard"`；否则视为非产品声明，仅追加 `claim_analysis_malformed` risks）
    - safe_claims 反向门：每个 `safe_claim` 检查命中 verified_chunks 中至少一条 `safe_claims` 集合；不命中追加 `safe_claim_not_verified:<claim>`（≤5 条，超出聚合 `safe_claim_not_verified:and_more:<n>`），不单独 block
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.6, 5.7_

  - [x] 3.6 为 finalize_review_for_send 写 lib 单元测试（≥ 7 例）
    - 必填违规 → `gateway_status="blocked_by_required_field"` + autonomy_mode=blocked
    - budget_exceeded + needs_review=true → blocked_by_budget；needs_review=false → 仍可发送但带 `local_review_low_risk_only` risk
    - claim_analysis 缺失 + knowledge_need="required" → fail-closed `blocked_by_safety_guard`
    - claim_analysis 缺失 + 闲聊 → 仅 risks 不 block
    - requiresProductKnowledge=true 且 verified_chunks=∅ → `blocked_unverified_product_claim`
    - hold_category="held_for_human" → 强制改 `held_by_ai_policy` + 事件
    - _Requirements: 3.11, 5.8, 9.10_

  - [x] 3.7 为 R2 revision 控制流写 lib 单元测试（≥ 5 例）
    - needsRevision=true 触发 1 次 revision（Reply Agent 调用次数 == 2）
    - revision 后第二轮 fail → `revision_failed` + should_reply=false
    - revision 之前 budget 超额 → `revision_skipped_budget_exceeded`
    - revisionDirection 空白 → `revision_skipped_invalid_direction`
    - revision LLM 30s 超时 → `revision_failed` + 事件 `revision_llm_failure`
    - _Requirements: 2.12_

  - [x] 3.8 检查点 — W2 校验/安全门波收尾
    - Ensure all tests pass, ask the user if questions arise.

- [x] 4. 工具循环 + 字典波（W3）：reply_with_tools_loop + MCP knowledge.* + system_taxonomies
  - [x] 4.1 在 `src/agent/budget.rs` 扩展 `RunBudget`
    - 新增 `tool_call_budget / tool_calls_used` 字段；新增 `record_tool_call(tokens_consumed) -> Result<(), BudgetError>` 方法（同时检查 tool_calls 与 tokens 双上限）
    - 在 `RunBudget::new` 中从 `runtime_parameters.knowledge_max_tool_calls` 注入预算（默认 6，clamp 到 [1,16]）
    - _Requirements: 4.3_

  - [x] 4.2 实现 MCP knowledge.* 三个工具的派发函数
    - 在 `src/agent/knowledge_router.rs`（或新建 `src/agent/knowledge_tools.rs`）实现 `dispatch_tool_call(call, ctx, budget) -> Result<ToolResult, ToolError>`
    - `knowledge.list_catalog`：输入 schema `{ kind?, limit? }`，输出 `{ items: [...], truncated }`，单 run 内同 kind 调用次数 ≤ 2
    - `knowledge.search`：输入 `{ query, top_k? }`，输出 `{ hits: [...], query, hit_count }`，integrity_status != verified 时 snippet 空字符串占位 + `redacted=true`
    - `knowledge.open_slice`：输入 `{ chunk_ids: [...] }`（≤ K，K 由 runtime_parameters 控制），输出 `{ slices: [...] }`；未知 chunk_id 全部 fail（不返回部分结果）；非 verified body 替换为 `<redacted_unverified_chunk>` 但 `integrity_status` 字段保留
    - 每个工具单次 5s timeout；预算超额返回 `{"error":"budget_exceeded"}`
    - _Requirements: 4.4, 4.5, 4.6, 4.8_

  - [x] 4.3 实现 `reply_with_tools_loop` 多轮派发循环
    - 在 `src/agent/knowledge_router.rs` 新增 `reply_with_tools_loop(ctx, runtime, budget) -> Result<(AgentDecision, Vec<String>), GatewayError>`
    - `MAX_TOOL_LOOPS` 由 `runtime_parameters.knowledge_max_tool_loops` 控制（默认 3，clamp [1,5]）
    - 每轮：调 Reply Agent → `validate_and_promote` → 按 `decision_phase` 分支
    - tool_calling 中间轮：丢弃 reply_text + should_reply（追加 `tool_calling_phase_with_reply_text`）；toolCalls > 4 截断（追加 `tool_calls_per_turn_truncated`）；执行 toolCalls 累计 result（≤ 8000 chars 累计；超出"丢弃最早" + `tool_result_context_truncated`）
    - final 轮：toolCalls 非空时清空（追加 `final_phase_extra_tool_calls_dropped`）；`knowledgeNeedReason` 非空非 unchanged 但 toolTrace 中无成功 search/open_slice → 追加 `knowledge_need_declared_but_not_consulted`；toolTrace 落 `decision.knowledge_route.tool_trace`（最多 32 条，超出 `tool_trace_overflow`）
    - 失败连击 ≥ 3 次 → 强制结束 + `tool_call_failure_streak`；循环总耗时 > 30s → `GatewayError::ToolLoopTimeout`（gateway_status="tool_loop_timeout" fail-closed）；MAX_TOOL_LOOPS 耗尽 + tool_calls 非空 → 强制 final 一次 + `tool_loop_exhausted`
    - 接入 `runtime_parameters.knowledge_routing_mode == "classic_router"` 灰度回退路径，调用现有 `run_knowledge_router`
    - _Requirements: 4.1, 4.2, 4.7, 4.8, 4.9, 4.10, 4.11, 4.12_

  - [x] 4.4 为工具循环写 lib 单元测试（≥ 6 例）
    - Reply Agent 在 auto_tool_loop 下依次 list_catalog → search → open_slice 完成一轮决策
    - tool_calls_used + tokens_used 计入 RunBudget；预算耗尽时 N+1 次 tool call 返回 `budget_exceeded`
    - MAX_TOOL_LOOPS 上限触发 `tool_loop_exhausted` risk
    - 单 tool 5s 超时只该 call 失败、循环继续；连续 3 次失败强制结束循环
    - decision_phase=tool_calling 时即使有 reply_text 也被丢弃；decision_phase=final 时仍带 toolCalls 被清空 + risk
    - classic_router 模式下行为与现状回归一致
    - _Requirements: 4.13_

  - [x] 4.5 实现 P7 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs`（W3 阶段先创建文件骨架）添加 P7 测试
    - **Property 7: 工具循环不死锁 + 预算不被绕过**
    - **Validates: Requirements 4.2, 4.3, 4.8**
    - 随机生成 `Vec<ToolCall>`（含非法 tool 名 / 超长 query / 超 K open_slice / 模拟超时），断言：循环 ≤ MAX_TOOL_LOOPS 内终止；总 tool 调用 ≤ knowledgeMaxToolCalls；budget 超额后任何后续 tool call 返回 `budget_exceeded` 而非实际执行
    - ≥ 64 用例，单条 ≤ 60s
    - _Requirements: 4.2, 4.3, 4.8, 12.1, 12.2_

  - [x] 4.6 新建 `src/agent/taxonomy.rs` 模块
    - `check_value(kind, value, scope, cache) -> TaxonomyMatch`：返回 `Active / AliasActive(canonical_id) / Deprecated / CandidateNew`
    - `upsert_candidate(db, scope, kind, raw_value, evidence, confidence)`：按 `(scope, kind, raw_value)` upsert，已存在 status=rejected 仅 `last_seen_at` 不递增 occurrences；status=pending 递增 occurrences；不存在则 insert pending
    - `approve(db, candidate_id, by)` / `reject(db, candidate_id, by)`：approve 时把 candidate 写入 `system_taxonomies`、把 status=approved；reject 仅 status=rejected
    - 启动期 `TaxonomyCache` 加载 + 后台 API 写后失效
    - _Requirements: 8.1, 8.3, 8.7_

  - [x] 4.7 在 `enforce_decision_guards` 中接入 taxonomy
    - 对 `decision.customer_stage / intent_level / objection_type` 三字段调 `taxonomy::check_value`，按 match 分支：alias 命中改写为 canonical_id；deprecated 追加 `taxonomy_deprecated_value:<kind>:<value>`；CandidateNew 追加 `taxonomy_candidate:<kind>:<value>` + `upsert_candidate`
    - **关键**：CandidateNew SHALL NOT 强制 `review.approved=false`
    - 在 `AgentDecision` 新增 `agent_generated_signals: Vec<AgentSignal>` 字段（W1 已添加占位，本任务落实业务接收逻辑）
    - _Requirements: 8.2, 8.3, 8.4, 8.5, 8.6_

  - [x] 4.8 实现 R8 后台 API 路由
    - 新建 `src/routes/admin_taxonomies.rs` + `src/routes/admin_taxonomy_candidates.rs`，注册 `GET / POST /api/admin/taxonomies`、`PATCH / DELETE /api/admin/taxonomies/:id`（软删）、`GET /api/admin/taxonomy-candidates?status=pending`、`POST /api/admin/taxonomy-candidates/:id/{approve,reject}`
    - approve 时事务性把 candidate 写入对应 `system_taxonomies` + 把 candidate.status=approved
    - 在 `src/routes/mod.rs` 注册路由
    - _Requirements: 8.7_

  - [x] 4.9 实现 `2026_05_006_taxonomy_seed` 迁移
    - 基于现有 prompt 中硬编码的 `customer_stage / intent_level / objection_type` 默认枚举集合，写入 `scope="global"` 默认字典；幂等（按 `(scope, kind, value.id)` 唯一索引）
    - _Requirements: 8.8, 11.7_

  - [x] 4.10 为字典 / candidate 写 lib 单元测试（≥ 4 例）
    - 不在字典的值 → upsert candidate + `review.risks` 含 `taxonomy_candidate:*`，但 `review.approved` 不被强制 false
    - alias 命中视为合法
    - operation_state 不在状态机走 R3.5 路径而非 R8 路径
    - approve 候选后写入字典且下次同值不再写候选
    - _Requirements: 8.9_

  - [x] 4.11 实现 P6 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs` 添加 P6 测试
    - **Property 6: 字典 candidate 不阻塞**
    - **Validates: Requirements 8.3, 8.4**
    - 随机生成 `decision.customer_stage`（一半在字典 / 一半不在），断言：不在字典时 risks 含 `taxonomy_candidate:customer_stage:<v>` + candidates 集合写入 + `review.approved` 不被强制 false
    - ≥ 64 用例
    - _Requirements: 8.3, 8.4, 12.1_

  - [x] 4.12 实现 P1 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs` 添加 P1 测试
    - **Property 1: 自治字段必填**
    - **Validates: Requirements 1.3, 3.5, 3.9**
    - 随机生成 `RawAgentDecision`（其中至少一个 R3 必填字段被设空 / 类型非法 / 枚举非法），运行 `validate_and_promote + finalize_review_for_send` 后断言：`review.approved=false` + `review.risks` 含 `missing_required_field:* / invalid_enum_value:* / invalid_type:*` 之一 + `decision.autonomy_mode="blocked"`
    - ≥ 64 用例
    - _Requirements: 1.3, 3.5, 3.9, 12.1_

  - [x] 4.13 实现 P3 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs` 添加 P3 测试
    - **Property 3: 预算超额不发送**
    - **Validates: Requirements 3.7, 3.10**
    - 随机生成 `RunBudget`（`is_exceeded()=true`）+ `decision.needs_review=true`，断言：`local_decision_review` 返回 `approved=false` + `gateway_status="blocked_by_budget"` + `autonomy_mode="blocked"` + mock 中 `send_called=0`
    - ≥ 64 用例
    - _Requirements: 3.7, 3.10, 12.1_

  - [x] 4.14 实现 P4 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs` 添加 P4 测试
    - **Property 4: 产品声明强约束**
    - **Validates: Requirements 5.4, 5.7**
    - 随机生成 `(claim_analysis.requiresProductKnowledge=true, used_knowledge_ids, integrity_status_set)` 三元组，当 `used_knowledge_ids ∩ verified_chunk_set == ∅` 时断言：`fact_risk >= 6 && approved=false && autonomy_mode="blocked" && gateway_status="blocked_unverified_product_claim"`
    - ≥ 64 用例
    - _Requirements: 5.4, 5.7, 12.1_

  - [x] 4.15 实现 P2 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs` 添加 P2 测试
    - **Property 2: Single-Shot Revision 上限**
    - **Validates: Requirements 2.3, 2.4, 2.8**
    - 随机生成 (Reply 输出, Review 输出 with `needsRevision=true && revisionDirection 非空`)，断言：Reply Agent 调用次数 ≤ 2；第二轮仍 fail 时 `gateway_status="revision_failed" && should_reply=false`
    - ≥ 64 用例
    - _Requirements: 2.3, 2.4, 2.8, 12.1_

  - [x] 4.16 检查点 — W3 工具/字典波收尾
    - Ensure all tests pass, ask the user if questions arise.

- [x] 5. 发送闭环波（W4）：outbox 模块 + dispatcher worker + 二次安全门 + 取消通道
  - [x] 5.1 新建 `src/agent/outbox.rs` 模块与数据结构
    - 定义 `OutboxEntry / OutboxStatus` enum（`Pending / InFlight / Sent / FailedTerminal / Canceled`，**统一枚举值用 `failed_terminal` 不用 `failed`**）
    - 实现 `enqueue(db, run_id, decision_id, source_event_id, source_kind, contact_wxid, account_id, content) -> Result<EnqueueOutcome, OutboxError>`：计算 `idempotency_key = SHA256(source_event_id + ":" + contact_wxid + ":" + content_hash)`；空 source_event_id 走兜底 `synthetic:` 前缀 + 写 `outbox_synthetic_idempotency_key` 警告事件；DuplicateKey 视为 `IdempotentSkip` + 写 `outbox_idempotent_skip` 事件
    - _Requirements: 13.1, 13.2_

  - [x] 5.2 实现 `OutboxDispatcher` worker（atomic claim + lease）
    - `worker_id = "{hostname}:{pid}:{uuid}"`；`poll_interval` 由 `runtime_parameters.outbox_poll_interval_seconds` 控制（默认 5s）；`lease` 由 `outbox_lease_seconds` 控制（默认 60s）
    - `tick`：先 `reclaim_expired_leases`（`status="in_flight" AND locked_until < now` → atomic 改回 pending + 写 `outbox_lease_expired`），再循环 `atomic_claim_pending`（`findOneAndUpdate({status="pending", $or:[next_retry_at:null, next_retry_at:{$lte:now}]}, {$set:{status="in_flight", worker_id, locked_until=now+lease}})` returnDocument After）
    - 在 `src/main.rs` / 启动期 `tokio::spawn` 启动 dispatcher（参考现有 worker 启动模式）
    - _Requirements: 13.3_

  - [x] 5.3 实现发送前二次安全门
    - 新增 `second_safety_gate(entry, db) -> Option<String>` 在 `process_entry` 中抢占后、调 MCP 之前执行
    - 检查 4 类取消条件：`contact.cooldown_until > now` → `contact_cooldown_active`；`contact.last_inbound_at > decision.created_at && reaction outcome 含 stop_re*` → `user_stop_requested_after_decision`；`agent_decision_reviews` 中 reaction outcome 含 stop_requested → 同上；`created_at > 30min` → `outbox_stale_30min`
    - 任一命中 SHALL 走 `cancel(db, entry, reason)` + 写 `outbox_canceled` 事件 + 清 `worker_id / locked_until`
    - _Requirements: 13.4_

  - [x] 5.4 实现 MCP 发送 + 重试 backoff
    - `process_entry` 在二次安全门通过后调 `mcp::send_message(contact_wxid, content)`；把 `send_outbound_message` 改为 `pub(crate)` 仅 dispatcher 可调用，加 `#[doc = "Only callable from outbox_dispatcher"]`
    - 成功 → status=Sent + sent_at=now + 清 worker_id/locked_until + 写 `outbox_sent`
    - 失败：attempt += 1；< max_attempts → status=Pending + `next_retry_at = now + (2^attempt)*5s + jitter` + last_error + 写 `outbox_retry_scheduled`；>= max_attempts → status=FailedTerminal + 写 `outbox_failed_terminal`
    - 同 `outbox_id` 事件总数 SHALL ≤ 20（防 retry 风暴）
    - _Requirements: 13.5, 13.7, N4_

  - [x] 5.5 改造 gateway 主路径：决策落地改走 outbox enqueue
    - 在 `src/agent/gateway.rs::run_user_operation_gateway` 中删除直接调 `send_outbound_message` 的路径；在 `update_run_envelope_terminal` 之后（`finalReviewStatus ∈ {approved, revision_applied_approved} && should_reply=true`）调用 `outbox::enqueue(...)`
    - dry-run / simulation 路径单独走 `simulate_send` 函数，与 outbox 完全分离（避免污染发送链路）
    - 在 `src/models.rs::AgentRunLog.outbox_status: Option<String>` 字段中由 dispatcher 反向更新（W4 收尾交付反向通知通道，简化为：dispatcher 在状态推进时 `update_one(agent_run_logs, {run_id}, $set: {outbox_status: ...})`）
    - _Requirements: 0.7, 13.2, 13.8, N4_

  - [x] 5.6 接入用户拒绝 / cooldown 取消通道
    - 在 `src/agent/reaction.rs::record_user_reaction` 中检测到 stop / cooldown 类 outcome 时调 `outbox::cancel_for_contact_on_user_reaction(db, account_id, contact_wxid)`，把同 contact 所有 `status ∈ {pending, in_flight}` 改为 `canceled` + `cancel_reason="user_reaction_stop_requested"` + 清 worker_id/locked_until + 每条写 `outbox_canceled` 事件
    - 新增后台 `POST /api/admin/outbox/:id/cancel`（body `{ cancel_reason }`），仅允许取消 pending / in_flight，其它返回 409
    - 新增 `GET /api/admin/outbox?status=...&account_id=...&horizon=...` 列表查询（在 `src/routes/admin_outbox.rs`）
    - _Requirements: 13.6, 13.9_

  - [x] 5.7 为 outbox 写 lib 单元测试（≥ 5 例）
    - 同 `idempotency_key` 二次写入触发 idempotent skip
    - atomic claim + locked_until 防双发：两个 worker 同时 claim 同一 entry，恰好一个成功
    - 二次安全门 4 类取消各触发
    - 重试 backoff 公式正确（attempt=1 ≈ 10s、attempt=2 ≈ 20s、attempt=3 ≈ 40s ± jitter）
    - 同 `source_event_id` 但 `run_id` 不同的多次决策 SHALL 共享同一 idempotency_key 故只发送 1 次
    - _Requirements: 13.10_

  - [x] 5.8 为 outbox 写集成测试（testcontainers，≥ 6 例）
    - 决策通过 → outbox pending → worker 抢占 → MCP mock 成功 → status=sent
    - MCP mock 失败 3 次 → status=failed_terminal（统一枚举值）
    - record_user_reaction stop_requested → 同 contact 所有 pending outbox canceled
    - 30 分钟陈旧 outbox 自动 canceled
    - 崩溃恢复：worker A 抢占后 kill，等 lease_seconds 过期，worker B 通过 reclaim_expired_leases 重新抢占并最终 sent
    - PBT：任意 outbox 状态序列下唯一 idempotency_key 永远 ≤ 1 次 MCP 实际发送
    - _Requirements: 13.10_

  - [x] 5.9 检查点 — W4 发送闭环波收尾
    - Ensure all tests pass, ask the user if questions arise.

- [x] 6. MemoryCard 边界替换波（W5）：整层 typed + stable id + deprecatedFacts
  - [x] 6.1 把 `OperatingMemory.memory_card` 整层从 `Document` 改为 `MemoryCardTyped`
    - 在 `src/models.rs:300` 把字段类型改为 `MemoryCardTyped`，含 `core_facts (cap=6) / recent_facts (cap=10) / deprecated_facts (cap=20) / core_profile / relationship_state / extra: Document`
    - 同步迁移 `agent_decision_reviews / agent_run_logs / contacts` 中所有引用 `memory_card` 字段的位置
    - _Requirements: 6.2, N5_

  - [x] 6.2 实现 `MemoryFact` 强类型与 `MemoryFactRepr` 反序列化兼容
    - 在 `src/agent/types.rs` 把 `MemoryFact` 提升为 OperatingMemory 共享结构，含 `id (UUIDv4 字符串) / text (1..=500) / evidence (≤1000) / confidence (0..=10) / importance (0..=10) / may_expire / deprecated_at / deprecation_reason (≤200) / source_message_ids (≤5) / source_run_id / created_at / updated_at`
    - 实现 `#[serde(untagged)] enum MemoryFactRepr { Plain(String), Structured(MemoryFact) }`，`Plain(text)` → `MemoryFact { id: <new UUIDv4>, text, confidence: 7, importance: 5, ... }`（关键：fresh UUID，避免老数据无 id 后续合并失真）
    - _Requirements: 6.1, 6.3_

  - [x] 6.3 改造 memory 模块所有 helper 改用 typed
    - `compact_memory_card_with_previous / consolidate_contact_memory / default_memory_card / memory_card_from_contact / write_memory_to_db / read_memory_from_db` 全部签名改用 `MemoryCardTyped` 入参与返回
    - 写入路径 `serde_bson::to_document(&MemoryCardTyped)` 一次性序列化，不保留两套并行表示
    - _Requirements: 6.2, N5_

  - [x] 6.4 实现 `apply_consolidator_deprecations` 处理 deprecatedFacts / conflicts
    - 按 `MemoryFact.id` 在上一版 `core_facts / recent_facts` 查找命中 → 移到 `deprecated_facts` 集合（保留原 id/text/evidence/confidence/importance/source_message_ids/source_run_id/created_at + 新填 deprecated_at/deprecation_reason/updated_at）
    - id 找不到 → 不写入 deprecatedFacts + warning `deprecated_fact_id_not_found:<id>`
    - supersededBy 在新版查不到 → warning `superseded_by_id_not_found:<id>:<sup>`，但 deprecated 仍写入
    - 非法 RFC3339 deprecatedAt → 回退 now + warning `invalid_deprecated_at:<id>:<raw>`
    - 同 id 同时 active+deprecated → 仅 deprecated 集合保留 + warning `fact_simultaneously_active_and_deprecated:<id>`
    - cap 20，按 deprecatedAt 升序 + id 字典序丢最旧
    - 写 `agent_run_logs.memory_consolidator_warnings`
    - _Requirements: 6.5, 7.2, 7.3, 7.4, 7.7_

  - [x] 6.5 实现 conflicts 事件与 Reply Agent context 注入
    - `conflicts[].winner != "none"` 时为每条写 `agent_events kind="memory_conflict_resolved"`，detail 含 a_id/b_id/winner/resolution/a_text/b_text
    - 在 reply_context_pack（`src/prompts.rs` 或 `src/agent/runtime.rs`）注入最近 K=5 条 deprecated_facts（仅 id+text+deprecation_reason+deprecated_at，按 deprecated_at 降序）
    - _Requirements: 7.5, 7.6_

  - [x] 6.6 实现 `2026_05_005_memory_facts_to_structured` 迁移
    - 扫描所有 `operating_memories.memory_card.coreFacts / recentFacts` 字符串元素 → 升级为结构化（fresh UUIDv4 + created_at=now + 默认 confidence=7/importance=5）
    - 幂等：用 `id` 字段是否存在 + 类型判断
    - _Requirements: 6.4, 11.7_

  - [x] 6.7 处理写入路径自动转换 + 前端 MemoryCardSummary 展示
    - 老 simulation 种子或 Vec<String> 输入路径自动转 MemoryFact 默认结构 + 响应 body 追加 `"warning": "memory_facts_auto_upgraded"`（sunset 后此路径在 R11 移除返回 400）
    - 前端 `MemoryCardSummary` 组件显示每条 fact 的 text / evidence（折叠）/ confidence / importance（小标签 0-10）/ may_expire（角标）/ deprecated_at（删除线 + tooltip）
    - _Requirements: 6.6, 6.7_

  - [x] 6.8 扩展 `tests/memory_card_invariants.rs` PBT
    - Plain 与 Structured 序列化往返保不变量
    - 旧 Vec<String> 输入下 cap=6/10 与"未 discarded 必保留"性质成立
    - 整层 MemoryCardTyped round-trip Mongo 不丢字段（含 extra）
    - _Requirements: 6.8_

  - [x] 6.9 为 R7 deprecatedFacts 写 lib 单元测试（≥ 4 例）
    - consolidator 输出 `deprecatedFacts: [{id:X, reason:Y, deprecatedAt:T}]` 时新 memoryCard 含 `id==X && deprecation_reason==Some(Y) && deprecated_at==T` 且原 text/source 保留
    - id 找不到 → warning fallback、不写 deprecatedFacts
    - 同 id 同时 active+deprecated → warning + 仅 deprecated 集合保留
    - 改写场景：新 fact text 与上一版 X 不同但 id 相同 → 视为改写直接覆盖、不进 deprecatedFacts
    - _Requirements: 7.8_

  - [x] 6.10 实现 P5 性质测试
    - 在 `tests/autonomy_protocol_pbt.rs` 添加 P5 测试（也可在扩展的 `memory_card_invariants.rs` 中）
    - **Property 5: 记忆冲突可追溯**
    - **Validates: Requirements 6.3, 7.2, 7.4**
    - 随机生成 (previous core_facts, consolidator 输出 with deprecatedFacts) → 断言：新版 deprecated_facts 含被弃用条目（按 id 命中）+ 互斥不变量（弃用 fact 不出现在 active 集合）+ stable id 沿用
    - ≥ 64 用例
    - _Requirements: 6.3, 7.2, 7.4, 12.1_

  - [x] 6.11 检查点 — W5 MemoryCard 边界替换波收尾
    - Ensure all tests pass, ask the user if questions arise.

- [x] 7. 监控 + PBT 收口波（W6）：前端自治 Tab + outcomes API + happy_path 扩展 + sunset 文档 + CI lint
  - [x] 7.1 实现 `GET /api/outcomes/autonomy` 后端聚合接口
    - 新建 `src/routes/outcomes_autonomy.rs`，参数 `horizon=24h|7d|30d&account_id=...`
    - 返回 7 指标 + 分子分母原始计数：`revision_trigger_rate / revision_pass_rate / ai_hold_breakdown / taxonomy_candidate_rate / unverified_claim_block_rate / self_critique_addressed_rate / autonomy_mode_distribution`
    - `total_runs == 0` 时所有比率返回 `null`
    - `legacy_mode_unchecked` 不计入新指标分子分母（独立计数显示）
    - 响应时间 ≤ 2s（在 100k runs 规模下，依赖 W0 的 `(account_id, finalReviewStatus, started_at)` 索引）
    - 同时实现 `GET /api/outcomes/autonomy/revisions?limit=50&horizon=...`
    - _Requirements: 9.5, 10.4, 10.5, 11.2_

  - [x] 7.2 实现前端「自治回路监控」Tab
    - 在 `frontend/src/App.tsx` 顶部 Tabs 中新增「自治回路监控」项，路由 `/outcome/autonomy`
    - 实现 7 个指标卡布局 + horizon 选择器（24h/7d/30d）+ AI 暂缓原因分布饼图（标签严格使用「AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文」，禁止"人工接管"等词）+ 发送链路状态行（`outbox_send_success_rate / canceled_rate / failed_terminal_rate / lease_expired_rate / mean_attempts_to_success`）
    - 「近 50 条 revision 记录」列表展示 contact_name / pre_reply_excerpt / post_reply_excerpt / revisionDirection / finalReviewStatus / holdCategory，点击展开看 pre/postRevisionSummary + selfCritique
    - 复用现有 `tokens.css` 设计语言
    - 决策审计详情面板按字段顺序展示 9 个自治协议字段（中文标签：用户理解/关系判断/运营目标/知识需求理由/记忆更新理由/自我批判/该回复理由/不回复理由/风险自检），`unchanged` 灰色「沿用上轮」、空但属互斥允许空淡灰「—」、空但 R1.3 必填红色「协议违规」
    - _Requirements: 1.9, 10.1, 10.2, 10.3, 10.6, 10.7, 10.8, 13.9_

  - [x] 7.3 为前端自治 Tab 写端到端测试（≥ 4 例）
    - 接口在 `total_runs=0` 时返回 null 比率
    - 构造 5 条 run（其中 2 条触发 revision）后 `revision_trigger_rate == 0.4`
    - 构造 3 条 hold（每个 holdCategory 各 1 条）后 ai_hold_breakdown 三类各 1/total_runs
    - `held_for_human` 历史值不被统计在任何分类内（视为脏数据）
    - _Requirements: 10.9_

  - [x] 7.4 扩展 `tests/happy_path_run.rs` 端到端冒烟
    - 新增 `autonomy_full_loop_with_revision`：模拟 needsRevision → revision 后通过；断言 Reply Agent 调用次数恰好 2、`revisionApplied=true && finalReviewStatus="revision_applied_approved" && pre_revision_summary / post_revision_summary` 都非空
    - 新增 `autonomy_tool_loop_happy_path`：模拟 list_catalog → search → open_slice 后给出回复；断言 `decision.knowledge_route.tool_trace.len() == 3 && tool_calls_used == 3 && finalReviewStatus="approved"`
    - 实现测试 helper `wait_for_outbox_processed(run_id, timeout=10s)`（因 outbox 解耦后必须等 worker tick 才能 assert "已发送"）
    - _Requirements: 12.6, N4_

  - [x] 7.5 实现 R9 审计字段单元测试
    - 一次正常通过 run 写入 `revisionApplied=false / finalReviewStatus="approved" / autonomyMode 由 Agent 输出落库`
    - revision 触发 + 二审通过 → `finalReviewStatus="revision_applied_approved"` 且 pre/postRevisionSummary 都非空
    - revision 触发 + 二审失败 → `finalReviewStatus="revision_failed"`
    - shouldHold + holdCategory 走对应 finalReviewStatus
    - `finalReviewStatus="held_for_human"` 被严格拒收（写库时 panic 或 error）
    - _Requirements: 9.10_

  - [x] 7.6 创建 `docs/sunset-plan.md`
    - 明确 D / D+7 / D+14 / D+21 时间表（升级度量脚本、移除 MemoryFactRepr::Plain、移除 autonomyProtocolEnabled / knowledgeRoutingMode、移除迁移脚本）
    - 列出每个灰度开关在 sunset 前后的行为对照表
    - 列出"双轨长期维护"协议违规的检测项（PR review checklist 强制项）
    - _Requirements: 11.5, 11.9_

  - [x] 7.7 实现 CI 严禁词文本 lint
    - 新建 `scripts/check-no-human-takeover.ps1` + `.sh` 版本，在 git diff 范围内扫描 `src/agent/ src/routes/ frontend/src/` 下新增字符串字面量，禁止包含 `human / 人工 / 接管 / takeover / hand-off`
    - 在 CI workflow 中作为合并门
    - _Requirements: 2.7_

  - [x] 7.8 兜底 lib 单元测试 — autonomy_mode 落库
    - autonomyMode=`auto / assisted / blocked` 三种取值正常落库
    - 缺失或非法时强制写 `blocked` + risk `autonomy_mode_invalid`
    - finalReviewStatus 严格枚举校验（写库前 `assert_final_review_status_valid` 阻断脏值）
    - _Requirements: 9.2, 9.3_

  - [x] 7.9 最终检查点 — 全部 PBT + lib + 集成 + happy_path 通过 + 基线脚本通过
    - Ensure all tests pass, ask the user if questions arise.

## Notes

- 标记 `*` 的子任务为可选（property tests / unit tests / integration tests），可在 MVP 加速时跳过；核心实现子任务从不标 `*`。
- 每个任务都引用具体的需求子条款（granular sub-requirements），不仅指向 user story 层级。
- 性质测试任务统一放在 `tests/autonomy_protocol_pbt.rs`（W3 阶段创建文件骨架，W3/W4/W5 各阶段陆续追加 P1–P7），单条性质 ≥ 64 用例、单条执行 ≤ 60 秒。
- W1–W4 顺序串行；W5 可与 W3/W4 并行（不互相依赖，独立 PR）；W6 依赖 W1–W5 全部完成。
- N1–N7 实现注记体现在波次依赖与具体 Task 描述里：N1 = 1.1/2.1；N2 = 2.2/2.3；N3 = 3.2；N4 = 5.5；N5 = 6.1/6.3；N6 = 整体波次划分；N7 = 1.5。
- 检查点任务（3.8 / 4.16 / 5.9 / 6.11 / 7.9）确保每波收尾有显式回归校验机会，避免下一波在隐患上累积。

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.3", "1.5"] },
    { "id": 1, "tasks": ["1.2", "1.4", "1.6"] },
    { "id": 2, "tasks": ["2.1", "2.2"] },
    { "id": 3, "tasks": ["2.3", "2.4"] },
    { "id": 4, "tasks": ["2.5", "2.6"] },
    { "id": 5, "tasks": ["3.1", "3.3"] },
    { "id": 6, "tasks": ["3.2", "3.5"] },
    { "id": 7, "tasks": ["3.4", "3.6", "3.7"] },
    { "id": 8, "tasks": ["4.1", "4.6", "6.1"] },
    { "id": 9, "tasks": ["4.2", "4.7", "4.9", "6.2"] },
    { "id": 10, "tasks": ["4.3", "4.8", "4.10", "6.3", "6.6"] },
    { "id": 11, "tasks": ["4.4", "4.5", "4.11", "4.12", "4.13", "4.14", "4.15", "6.4", "6.7"] },
    { "id": 12, "tasks": ["5.1", "6.5", "6.8", "6.9"] },
    { "id": 13, "tasks": ["5.2", "5.3", "6.10"] },
    { "id": 14, "tasks": ["5.4", "5.5"] },
    { "id": 15, "tasks": ["5.6", "5.7", "5.8"] },
    { "id": 16, "tasks": ["7.1", "7.6", "7.7"] },
    { "id": 17, "tasks": ["7.2", "7.4", "7.5", "7.8"] },
    { "id": 18, "tasks": ["7.3"] }
  ]
}
```
