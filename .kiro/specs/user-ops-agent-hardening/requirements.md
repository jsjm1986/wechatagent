# Requirements Document

> 中文标题：用户运营 Agent 鲁棒性强化 — 需求文档

## Introduction

本次改造针对 WechatAgent 现有用户运营 Agent（`agent.rs` 中以 `run_user_operation_gateway` 为核心的链路）做一次全面修复和优化。基于代码审查发现的 20 项问题，按优先级分为 **高优先级（产线 bug）/ 中优先级（设计层）/ 低优先级（代码质量）/ 战略级（长期能力）** 四档。

目标：

- 修复一切会在长跑中确定性出问题的产线 bug
- 给 LLM 调用链加成本上限，避免单条消息烧掉数万 token
- 把"产品事实安全"从模型自我声明升级为多层兜底
- 引入状态机迁移路径校验、memoryCard 分层、知识库未验证告警等设计修正
- 拆分 mega 文件、强类型化核心字段、补齐索引、补齐测试
- 为长期效果验证（公式遵守度、horizon metrics、dry-run）留好脚手架

## Requirements

### 高优先级需求

### Requirement 1: HP-1 跟进任务 worker 加入 running 超时回收

**User Story:** 作为产品运维者，我希望即使 worker 进程崩溃或某次 LLM 调用挂死，已 claim 的跟进/记忆整理任务也能在合理时间内被自动恢复，以便所有任务不会静默消失。

#### Acceptance Criteria

1. WHEN `tasks::tick` 启动一次轮询，THEN 系统 SHALL 先扫描所有 `status="running" AND claimed_at < (now - claim_timeout_seconds)` 的任务，并将其重置为 `status="retry"`、`gateway_status="claim_timeout_recovered"`、`next_retry_at=now`，且不增加 `attempt_count`。
2. WHEN 一条任务被 claim 进入 `running`，THEN 系统 SHALL 把 `claimed_at` 写为当前时间。
3. WHEN `claimed_at` 字段在老任务中缺失，THEN 系统 SHALL 视其等同于 `claim_timeout_seconds` 之前的任意时刻（即首次 tick 立即可回收），但 IF 进程自身的启动时间晚于该任务的 `updated_at`，THEN 系统 SHALL 跳过该任务回收一次以避免与正在跑的真实 worker 冲突。
4. WHERE `claim_timeout_seconds` 来自 AppConfig（默认 300 秒，可通过 `TASK_CLAIM_TIMEOUT_SECONDS` 环境变量覆盖），THE 系统 SHALL 使用该配置值。
5. WHEN 一条任务被回收，THEN 系统 SHALL 写入 `agent_events kind="task_claim_recovered" status="recovered"`，详情包含 `task_id`、`kind`、`previous_attempt_count`、`stuck_seconds`。
6. IF 同一任务在 24 小时内被回收次数 ≥ 3，THEN 系统 SHALL 把它标记为 `status="failed"`、`gateway_status="claim_recovery_exhausted"`，并写一条 `follow_up_failed` 事件，避免无限回收循环。

### Requirement 2: HP-2 拆分 last_message_at 为 inbound/outbound 双字段

**User Story:** 作为运营策略维护者，我希望系统能精准区分"用户最后一次说话"和"Agent 最后一次出站"，以便跟进任务的 `context_changed` 检查不会被 Agent 自己的发送误触发。

#### Acceptance Criteria

1. THE Contact 模型 SHALL 新增字段 `last_inbound_at: Option<DateTime>` 和 `last_outbound_at: Option<DateTime>`，并保留 `last_message_at` 作为兼容字段（每次更新时取两者较大者）。
2. WHEN 一条入站消息（webhook 或 simulation 不计）被写入 `conversation_messages`，THEN 系统 SHALL 同时更新 contact 的 `last_inbound_at = now` 与 `last_message_at = now`。
3. WHEN `agent::send_outbound_message` 完成出站，THEN 系统 SHALL 更新 `last_outbound_at = now` 和 `last_agent_run_at = now`，且同步更新 `last_message_at = max(last_inbound_at, last_outbound_at)`，但 SHALL NOT 把 `last_inbound_at` 改成 now。
4. WHEN `precheck_send_gateway` 检查 follow-up 任务的 context 是否变化，THEN 系统 SHALL 比较 `last_inbound_at > task.created_at` 而不是 `last_message_at > task.created_at`。
5. WHEN 服务首次启动且检测到存量 contact 中存在 `last_inbound_at` 缺失但 `last_message_at` 存在，THEN 系统 SHALL 一次性把 `last_inbound_at` 回填为 `last_message_at`（仅一次，幂等），并在日志中记录回填条数。
6. THE 数据迁移 SHALL 通过一个 `db::migrations` 模块实现，在 `Database::connect` 后、`ensure_indexes` 之前执行；迁移要写一条 metadata 文档（如 `migrations` 集合）记录已执行版本，避免重复回填。

### Requirement 3: HP-3 record_user_reaction 加入 claim 锁

**User Story:** 作为成本控制者，我希望用户连发多条消息时反应分析不会重复执行，以便不浪费 LLM 调用且数据不被互相覆盖。

#### Acceptance Criteria

1. WHEN webhook 处理一条入站消息且 contact 是 managed，THEN 系统在调用 `analyze_user_reaction` 之前 SHALL 先尝试 `update_one(filter: { ..., outcome_status: "pending" }, $set: { outcome_status: "analyzing", reaction_claimed_at: now })`。
2. IF claim 操作的 `modified_count == 0`，THEN 系统 SHALL 跳过本次 reaction analysis，不再调用 LLM，也不写 `agent_events`。
3. WHEN reaction analysis 完成（无论成功失败），THEN 系统 SHALL 把 `outcome_status` 从 `analyzing` 更新为模型输出的最终值（如 `user_replied_buying_signal`）或失败时回退为 `user_replied_unclassified`。
4. IF 一条 review 卡在 `outcome_status="analyzing"` 超过 60 秒（可配置 `REACTION_ANALYSIS_CLAIM_TIMEOUT_SECONDS`），THEN 系统下次 tick 或下次 webhook 触发时 SHALL 把它视为可重新 claim（防止分析进程崩溃后永远卡死）。
5. THE 字段 `reaction_claimed_at` SHALL 加入 `agent_decision_reviews` 模型，仅在 claim 与回写时使用，不暴露在前端默认列表中。
6. WHEN 单元测试模拟 N=10 个并发 webhook 同时到达同一 contact，THEN 系统 SHALL 保证至多 1 次 LLM `analyze_user_reaction` 调用被发起，其余 9 次直接跳过。

### Requirement 4: HP-4 LLM 重试改为指数退避并修正 retryable 判定

**User Story:** 作为成本控制者，我希望 LLM 客户端的重试策略既能扛住真实的 429/5xx 抖动，又不会在模型确定性输出非 JSON 时浪费 token。

#### Acceptance Criteria

1. WHEN LLM 调用失败且错误属于 `is_retryable_llm_error`，THEN 系统 SHALL 用指数退避：`delay_ms = retry_base_ms * 2^(attempt-1) + jitter`，其中 `jitter` ∈ `[0, retry_base_ms)`。
2. WHEN HTTP 响应包含 `Retry-After` header，THEN 系统 SHALL 用 `max(指数退避值, Retry-After 值)` 作为实际等待时间。
3. THE `is_retryable_llm_error` SHALL NOT 把 `AppError::Json(_)` 判为可重试；JSON 解析错误一律 fail-fast，由调用方决定降级（如使用 `local_decision_review` 或 `fast_chat`）。
4. WHEN LLM 调用经过 N 次重试，THEN `llm_call_logs` 单条记录 SHALL 新增字段 `retry_count: i32`、`final_status: "success"|"failed"|"json_error"`，便于事后统计 429 比例。
5. THE `retry_base_ms` 默认 SHALL 改为 `1000ms`，`max_retries` 仍为 3；总最坏退避时间不超过 8 秒。
6. WHEN `is_retryable_llm_error` 返回 true 但调用最终失败，THEN 系统 SHALL 把最后一次错误信息写入 `llm_call_logs.error`，并触发上层降级路径（不抛 panic、不阻断主流程）。


## 中优先级需求

### Requirement 5: MP-5 单次 run 引入 LLM 成本上限和降级链

**User Story:** 作为成本控制者，我希望任何单条入站消息或跟进任务触发的一次 run 都有 token 预算和调用次数上限，超额时自动降级而不是无限烧 token。

#### Acceptance Criteria

1. THE `agent_run_logs` SHALL 新增字段 `token_budget: i64`、`tokens_used: i64`、`llm_calls_used: i32`、`degraded_reasons: Vec<String>`。
2. WHEN 一次 run 启动，THEN 系统 SHALL 从 domain config 的 `runtime_parameters.runTokenBudget`（默认 30000）和 `runtime_parameters.runMaxLlmCalls`（默认 6）读取上限，并把当前累计写回 `agent_run_logs` 对应记录（每次 LLM 调用后追加）。
3. WHEN 一次 LLM 调用完成，THEN 系统 SHALL 把 `total_tokens` 加到当前 run 的 `tokens_used`，把 `llm_calls_used += 1`。
4. IF 当前累计 `tokens_used >= runTokenBudget` 或 `llm_calls_used >= runMaxLlmCalls`，THEN 后续阶段 SHALL 走降级路径：
   - WHEN 仍未进入 review 阶段，系统 SHALL 直接用 `local_decision_review` 替代 LLM review；
   - WHEN 仍未进入 rewrite 阶段，系统 SHALL 跳过 rewrite，直接用第一次 review 结果决定是否拦截；
   - WHEN 处于 knowledge router 二次决策前，系统 SHALL 跳过二次 `decide_reply` 改用第一次决策；
   - 每个降级动作都 SHALL 在 `degraded_reasons` 里追加一条可读理由。
5. WHEN 触发降级，THEN 系统 SHALL 写一条 `agent_events kind="run_budget_exceeded" status="degraded"`，详情包含触发的具体上限、`run_id` 和已用 token/calls。
6. WHEN 模拟（simulation）和评测（evaluation）路径触发 run，THEN 它们 SHALL 同样受预算约束，但可独立配置 `simulationTokenBudget`（默认 60000）。
7. WHERE 操作员通过后台修改 domain config，THE 系统 SHALL 立即对新 run 生效，无需重启。

### Requirement 6: MP-6 增加 Rust 端字符串级 fact-risk 兜底 guard

**User Story:** 作为产品安全责任人，我希望 Agent 的"产品事实"安全网不只依赖模型自我声明，以便即使模型被绕过或 claim_analysis 输出错误也能拦下高风险表达。

#### Acceptance Criteria

1. WHEN `enforce_decision_guards` 运行且 `decision.should_reply == true`，THEN 系统 SHALL 在 reply_text 上扫描"产品事实风险标记词"列表（默认含：`保证`、`一定能`、`绝对`、`百分之`、正则 `\d+\s*(%|％|折)`、正则 `[¥￥]\s*\d+|\d+\s*(元|万|亿|RMB|rmb)`、`案例`、`成功率`、`见效`、`回款`）。
2. IF 扫描命中任意标记词且 `decision.used_knowledge_ids.is_empty() && decision.safe_claims_used.is_empty()`，THEN 系统 SHALL 把 `review.scores.fact_risk = max(review.scores.fact_risk, 6)`、`review.scores.product_accuracy = min(review.scores.product_accuracy, 6)`、并在 `review.risks` 追加可读理由（如 `"reply_text 含高风险表达 [保证]，但本次未引用任何知识切片或安全声明"`）。
3. THE 标记词列表和白名单 SHALL 存储在 `prompt_templates`（key=`user.review.product_claim_markers`），运行时从 DB 读取，模型/运营可后台编辑。
4. THE 白名单 SHALL 支持"前置短语豁免"机制：当标记词左侧 N 字符内出现白名单短语（如 `准时回复你`、`隐私`、`你的判断`），不触发兜底。默认白名单含：`准时|按时|尊重|保护|你的`。
5. IF `claim_analysis.requiresProductKnowledge == false` 且 `claim_analysis.knowledgeSupported == true`，THEN 系统 SHALL 跳过字符串扫描（信任模型语义判断）。
6. WHEN 字符串 guard 触发拦截，THEN `decision_reviews.risks` SHALL 含一条以 `"string_guard:"` 前缀的可读理由，便于事后查询。
7. WHEN 单元测试输入"我会准时回复你"、"保证不会泄露你的隐私"，THEN guard SHALL NOT 触发（命中白名单豁免）。
8. WHEN 单元测试输入"我们能保证你转化提升 30%"且 `used_knowledge_ids` 为空，THEN guard SHALL 触发，fact_risk 至少 6。

### Requirement 7: MP-7 状态机迁移路径合法性校验

**User Story:** 作为运营方法论守护者，我希望模型不能从任意状态跳到任意状态，以便所有状态迁移都有可追溯的业务证据。

#### Acceptance Criteria

1. THE 状态机 schema SHALL 新增字段：每个 state 增 `allowedFrom: Vec<String>`（可为空）、可选 `allowFromAny: bool`（默认 false）。
2. THE `prompts.rs::default_user_operation_state_machine` SHALL 写入合理默认值：
   - `new_contact.allowedFrom = []`（仅作为初始态）
   - `relationship_building.allowedFrom = ["new_contact", "need_discovery", "objection_handling"]`
   - `need_discovery.allowedFrom = ["new_contact", "relationship_building", "solution_fit", "objection_handling"]`
   - `solution_fit.allowedFrom = ["need_discovery", "objection_handling"]`
   - `objection_handling.allowedFrom = ["solution_fit", "need_discovery", "commitment_followup"]`
   - `commitment_followup.allowedFrom = ["solution_fit", "objection_handling", "need_discovery"]`
   - `customer_success.allowedFrom = ["commitment_followup"]`
   - `cooldown.allowFromAny = true`
   - `dormant_reactivation.allowedFrom = ["cooldown"]`
3. WHEN `enforce_decision_guards` 运行且 `decision.operation_state` 非空，THEN 系统 SHALL 校验从 `contact.operation_state` 到 `decision.operation_state` 的迁移是否合法（`allowedFrom` 包含当前 state 或目标 state `allowFromAny == true`）。
4. IF 迁移不合法，THEN 系统 SHALL 把 `review.scores.fact_risk = max(_, 6)`、`review.approved = false`、`review.risks` 追加可读理由，并在 `decision_reviews.risks` 中记录 `"state_transition_invalid: from=<a> to=<b>"`。
5. WHEN `contact.operation_state` 缺失（首次互动），THEN 系统 SHALL 视目标为 `new_contact` 才合法；其它目标走非法路径。
6. THE 后台 UI（系统策略 → 用户运营域 → 状态机编辑） SHALL 始终暴露 allowedFrom 编辑能力（不依赖任何上下文条件），保存时校验目标 state key 存在；对设置了 `allowFromAny=true` 的 state，UI SHALL 把 allowedFrom 数组渲染为只读并显示提示"该状态允许从任意状态迁入"。
7. THE PBT SHALL 验证：对随机生成的 (from, to) 对，guard 行为与 `allowedFrom`/`allowFromAny` 的定义完全一致。

### Requirement 8: MP-8 memoryCard 拆分 coreFacts/recentFacts，引入重要度排序

**User Story:** 作为长期记忆守护者，我希望关键早期事实（如客户身份、预算）不会被后续寒暄信息挤出 memoryCard，以便 Agent 能持续基于真正重要的事实做判断。

#### Acceptance Criteria

1. THE memoryCard schema SHALL 把 `activeFacts: Vec<String>` 替换为 `coreFacts: Vec<String>`（cap 6）和 `recentFacts: Vec<String>`（cap 10）。
2. THE `compact_memory_card` SHALL 对 `coreFacts` 按 importance 倒序保留前 6 条，对 `recentFacts` 按 recency 保留尾部 10 条；其它数组（preferences/doNotDo/commitments/objections/openLoops 等）按 importance 倒序保留 cap 内项。
3. THE consolidator prompt（`user.memory_consolidator.task`） SHALL 显式要求模型按 importance 倒序输出每个数组，且区分 coreFacts vs recentFacts；模型可在 `discarded` 数组中说明显式 deprecate 的 coreFacts。
4. WHEN consolidator 输出未包含某条已存在的 coreFact 且未在 `discarded` 中显式 deprecate，THEN `compact_memory_card` SHALL 保留该 fact 在 coreFacts 中（合并语义，而非覆盖语义）。
5. THE 数据迁移 SHALL 一次性把存量 `activeFacts` 复制到 `coreFacts`，超出 6 条的尾部转入 `recentFacts`，迁移记录写入 `migrations` 集合。
6. THE 后台 contact 详情面板 SHALL 区分展示 coreFacts 和 recentFacts，并允许人工把某条 fact 在两者之间移动。
7. THE PBT SHALL 验证：对任意一组初始 coreFacts S 和 N 轮 consolidation 输入（其中 S 中的 fact 不在 `discarded` 里），最终 coreFacts ⊇ S。

### Requirement 9: MP-9 知识库未验证冷启动告警与批量自动校验

**User Story:** 作为新接手的运营人员，我希望导入了知识库但运行时没生效时能立即看到提示，以便不会困惑"为什么 Agent 不会说产品信息"。

#### Acceptance Criteria

1. WHEN 一次 agent run 进入 `load_operation_knowledge` 阶段且检测到 `total_chunks > 0 && verified_chunks == 0`，THEN 系统 SHALL 写一条 `agent_events kind="knowledge_unverified_warning" status="warn"`，详情含 `total_chunks`、`needs_review_chunks`、`rejected_chunks`、可操作的提示文案。
2. THE 同一 contact 同一日 SHALL 至多写一条该告警事件（去重通过当日 + contact_wxid 检测）。
3. THE 后台 UI（运营知识库面板） SHALL 在顶部明显展示 `verified` 占比和未验证条数；当未验证占比 > 50% 时显示醒目橙色提示。
4. THE 新接口 `POST /api/operation-knowledge/auto-verify` SHALL 接受 `accountId`、`confidenceThreshold`（默认 7，1-10）、`humanAuditSampleRate`（默认 0.1，1/N 抽样）作为参数，对当前 `needs_review` 的 chunks 批量调 LLM（prompt key `knowledge.auto_verify`），按 `confidenceScore >= threshold` 自动标 `verified`，否则标 `needs_review`；按 sampleRate 随机标 `needs_human_audit`。
5. THE 自动校验过程 SHALL 串行调用以避免并发烧 token，每条结果写 `knowledge_usage_logs`（route_result 字段记录原始 confidence 和判定）。
6. THE 自动校验 SHALL 受 MP-5 的全局 LLM 预算约束，超额时停止并返回已处理条数。
7. WHEN 一次 auto-verify 完成，THEN 系统 SHALL 写一条 `agent_events kind="knowledge_auto_verify_done"`，含 verified/needs_review/needs_human_audit 各计数。

### Requirement 10: MP-10 消费 operation_state_confidence 触发 full review

**User Story:** 作为评审策略维护者，我希望模型对状态判断不确定时，评审能强制走 full 模式，以便低置信度决策不会用 light review 一笔带过。

#### Acceptance Criteria

1. THE domain config `runtime_parameters` SHALL 新增 `operationStateConfidenceFullReviewBelow`（默认 4，范围 0-10）。
2. WHEN `effective_review_mode` 计算，THEN 系统 SHALL 在原有条件之外加入：IF `decision.operation_state_confidence.unwrap_or(10) < threshold`，THEN review_mode 强制为 `"full"`，并把 `agent_run_logs.planner.confidence_override_triggered` 标记为 `true`，便于审计区分该次 full review 的来源。
3. WHEN 强制走 full 是因为 confidence 低，THEN `agent_run_logs.planner` SHALL 在 reason 中追加一条可读说明（如 `"operation_state_confidence=3 below threshold 4"`）。
4. WHEN `decision.operation_state_confidence` 缺失（模型未输出），THEN 系统 SHALL 视为 10（最高置信），不强制 full。
5. THE 单元测试 SHALL 验证：confidence=3 + planner.knowledge_required=false 仍能触发 full review；confidence=8 走原 light/full 判定。
6. THE 后台 UI（domain config 编辑页） SHALL 暴露该阈值字段。


## 低优先级需求

### Requirement 11: LP-11 拆分 mega 文件 routes.rs 与 agent.rs

**User Story:** 作为后续维护者，我希望关键文件不再是 5000+ 行单文件，以便 PR 评审、IDE 跳转和编译增量都能落地。

#### Acceptance Criteria

1. THE `src/routes.rs` SHALL 被拆分为 `src/routes/mod.rs` 加子模块：`accounts.rs`、`contacts.rs`、`conversations.rs`、`tasks.rs`、`events.rs`、`assets.rs`、`knowledge.rs`、`playbooks.rs`、`domains.rs`、`prompt_templates.rs`、`souls.rs`、`reviews.rs`、`evaluations.rs`、`simulations.rs`、`management.rs`、`guides.rs`、`health.rs`、`shared.rs`（共享 helpers）。
2. THE `src/agent.rs` SHALL 被拆分为 `src/agent/mod.rs` 加子模块：`gateway.rs`、`decision.rs`、`review.rs`、`knowledge_router.rs`、`memory.rs`、`reaction.rs`、`simulation.rs`、`guards.rs`、`types.rs`、`runtime.rs`。
3. THE 拆分 SHALL 是机械重构：所有公开 API 行为完全不变；既有路由的 URL、HTTP method、请求/响应 JSON shape、错误码全部保留。
4. WHEN 拆分完成，THEN `cargo check` 与 `cargo test` SHALL 全部通过（含 LP-16 新增的集成测试和 PBT）。
5. THE 拆分 SHALL 不引入新的 public 类型或字段；只做模块层级的重新组织和 use 路径调整。
6. THE 拆分 SHALL 在每个子模块顶部加一段 `//!` 注释说明该模块职责（中文），以便后来者理解。
7. WHEN 拆分进行中，THE 工作 SHALL 分多个独立 commit（一个文件一个 commit），便于 git history 追踪。

### Requirement 12: LP-12 核心 Document 字段强类型化

**User Story:** 作为代码安全性维护者，我希望关键的 `Document`-typed 字段被换成强类型 struct，以便重命名字段时编译器能直接报错。

#### Acceptance Criteria

1. THE 系统 SHALL 引入强类型：`RuntimeParameters`（覆盖 user_operations 域的 11 个字段）、`MemoryCardCoreProfile`、`MemoryCardRelationshipState`、`MemoryCard`（含 coreFacts/recentFacts）、`UserUnderstanding`、`ProductFit`、`NextActionMemory`。
2. THE 强类型 SHALL 用 `#[serde(rename_all = "camelCase")]` 保留 wire 格式不变；MongoDB 序列化形状不变；既有数据可直接反序列化。
3. THE `OperatingMemory` 模型字段 SHALL 改为强类型；`OperationDomainConfig.runtime_parameters` SHALL 改为强类型；其它 Document（如 `operation_policy`、`profile_attributes`）保持 Document 直到下次迭代。
4. THE 新类型 SHALL 提供 `From<NewType> for Document` 的转换实现，便于既有调用点（如 `prompts::default_domain_configs` 用 `doc!` 构造）渐进迁移。
5. THE 现有调用点 SHALL 全部迁移到强类型，不再用 `doc.get_str("xxx")` 这样的字符串查询；`agent.rs` 中的 `doc_string`、`doc_i32`、`doc_i64` helper 仅保留给真正动态的 Document 使用。
6. WHEN 测试旧数据反序列化，THEN 强类型 SHALL 兼容缺失字段（用 `#[serde(default)]`）。

### Requirement 13: LP-13 补齐缺失索引

**User Story:** 作为性能维护者，我希望高频查询路径有合适索引，避免线上随数据量增长出现退化。

#### Acceptance Criteria

1. THE `db.rs::ensure_indexes` SHALL 创建 `wechat_accounts.{app_id}` 索引，`sparse=true`（webhook resolve_account_context 高频查）。
2. THE `agent_tasks` SHALL 增加复合索引 `{workspace_id, account_id, contact_wxid, kind, status}`。
3. THE `agent_decision_reviews` SHALL 增加 partial index `{workspace_id, account_id, contact_wxid, status, outcome_status}`，partialFilterExpression 限定 `outcome_status: { $in: ["pending", "analyzing"] }`。
4. THE `agent_events` SHALL 增加复合索引 `{workspace_id, account_id, contact_wxid, created_at: -1}`。
5. WHEN 索引创建已存在则跳过；MongoDB driver 的 `create_index` 默认是幂等行为，本需求不引入额外去重逻辑。
6. THE 新索引 SHALL 仅通过 `ensure_indexes` 流程创建；运行时代码 SHALL NOT 在其它路径中创建或确保索引（即所有索引创建都集中在该函数）。
7. THE 新索引 SHALL 在启动时自动创建（沿用现有 `ensure_indexes` 流程），无需运维手动操作。

### Requirement 14: LP-14 webhook 引入按账号限流

**User Story:** 作为成本与稳定性守护者，我希望 webhook 入口能扛住上游异常或攻击，不会无限烧 LLM 配额。

#### Acceptance Criteria

1. THE webhook 入口 SHALL 实现 per-`account_id` 的令牌桶限流，默认窗口 60 秒，容量 30 个请求。
2. THE 限流参数 SHALL 来自 AppConfig：`WEBHOOK_RATE_LIMIT_WINDOW_SECONDS`（默认 60）、`WEBHOOK_RATE_LIMIT_CAPACITY`（默认 30）。
3. WHEN 同一 `account_id` 在窗口内累计请求数超过容量，THEN webhook SHALL 永远返回 HTTP 429（不返回 HTTP 200 加 body 错误），响应 header 含 `Retry-After: <seconds>`，body 含 `{"error": "rate_limited", "account_id": "..."}`。
4. THE 限流计数 SHALL 在内存中维护（无需持久化）；单实例部署即可；多实例部署时本次不做集群级限流（标注为已知限制）。
5. WHEN webhook 解析不出 `account_id`（即走默认 account），THEN 限流 SHALL 应用到 `default` 这个 account。
6. THE 限流命中 SHALL 写一条 `agent_events kind="webhook_rate_limited" status="blocked"`（同样按 account 当日去重，避免事件爆量）。

### Requirement 15: LP-15 LLM_EXACT_CACHE 替换为 LRU

**User Story:** 作为成本与体验守护者，我希望 LLM 精确缓存命中率不会因为整体清空而出现断崖。

#### Acceptance Criteria

1. THE `LLM_EXACT_CACHE` SHALL 替换为基于 `lru` crate 或等价实现的 LRU 缓存，容量 256，移除现有的"超 256 整体 clear"逻辑。
2. THE 锁实现 SHALL 改为 `parking_lot::Mutex`（不跨 await 持有）或 `tokio::sync::Mutex`（如需异步访问）。
3. WHEN 缓存命中，THEN `llm_call_logs.status = "cache_hit"` 行为保持不变。
4. THE 缓存 key 生成（基于 prompt_key + system + user 的 FNV hash）SHALL 不变。
5. THE 缓存适用 prompt key 列表 SHALL 保持当前的 4 个：`knowledge.import.preview`、`playbook.generator`、`playbook.optimizer`、`user.guide.preview`；不扩到运行时回复链路。
6. THE LRU 命中率 SHALL 通过现有的 `/api/llm-usage` 接口可观测（cache_hit 状态的占比）。

### Requirement 16: LP-16 补齐测试覆盖

**User Story:** 作为长期维护者，我希望关键链路有集成测试和属性测试守护，以便重构和新功能能放心推进。

#### Acceptance Criteria

1. THE 系统 SHALL 引入 `mockall` 或等价方式给 `LlmClient` 加可 mock 抽象；引入 `testcontainers` 或 `mongodb-memory-server` 等价方式提供集成测试用 MongoDB。
2. THE 系统 SHALL 至少新增一个集成测试覆盖 `run_user_operation_gateway` happy path：managed contact + 入站消息 + decide → review pass → send，验证 `conversation_messages` 出站记录、`decision_reviews` 一条 sent 记录、`contacts.last_outbound_at` 已更新。
3. THE 系统 SHALL 用 `proptest` 加 ≥ 3 个属性测试覆盖 Correctness Properties：状态机迁移合法性、memoryCard 不变量、任务 claim 幂等性。
4. THE HP-1 / HP-2 / HP-3 / HP-4 修复 SHALL 各自配回归测试（一条任务被 stale-recover；last_inbound_at vs last_outbound_at 在 follow-up 检查中行为正确；并发 reaction claim 至多一条成功；JSON 错误不重试）。
5. THE MP-6 字符串 fact-risk fallback SHALL 配 ≥ 4 条测试：触发场景、白名单豁免场景、claim_analysis 已声明无需知识时跳过场景、used_knowledge_ids 非空时跳过场景。
6. THE MP-7 状态机迁移 SHALL 配 ≥ 3 条测试：合法迁移通过、非法迁移拦截、cooldown allowFromAny。
7. THE 测试运行时间 SHALL 控制在合理范围（单次 `cargo test` 不超过 90 秒），耗时长的集成测试用 `#[ignore]` 或 feature flag 隔离。

### Requirement 17: LP-17 group/moment 种子状态改为 draft

**User Story:** 作为新接手的运营人员，我希望前端展示的"已发布"配置都是真正运行时可用的，不会被未实现的 group/moment 模板误导。

#### Acceptance Criteria

1. THE `prompts.rs::soul_specs` 中 group/moment 两个 SoulSpec SHALL 在 `reset_prompt_pack_v2` 写入时使用 `status="draft"` 而不是 `published`。
2. THE `prompts.rs::prompt_specs` 中 `group.policy`、`moment.policy` 两条 PromptSpec SHALL 写入 `status="draft"` 而不是 `active`。
3. THE `ensure_prompt_pack_v2` 的"已存在"检测 SHALL 把 `draft` 也视为已存在（不重新种），避免每次启动都重置。
4. IF "已存在"检测逻辑因任何原因（如查询异常、模板字段错乱）未识别到现有 draft，THEN 系统 SHALL 重新种入默认 group/moment 模板，宁可短暂存在重复条目也要保证模板始终可用；运营人员可后续通过 UI 清理重复项。
4. THE `prompts::load_prompt` 仅查找 `status="active"` 的模板的当前行为保持不变；group/moment 现在不会被运行时误用。
5. THE 后台 UI（系统策略 → Agent souls / prompt 模板列表） SHALL 对 `status="draft"` 行加明显标签（如灰色"草稿"徽章），并把这些行排序到列表底部。
6. THE 前端 `groupOps` 和 `momentOps` 频道 SHALL 在 NextPhasePanel 中明确说明"对应 prompt 已存在但运行时未实现"。
7. WHEN 未来某个 domain 的运行时实现完成，THE 运营人员 SHALL 能在后台一键把对应 soul/prompt 改为 published/active；本次不实现该一键切换的具体 UI，但要保留接口（已有 `POST /api/agent-souls/:id/publish` 和 `POST /api/prompt-templates/:id/publish`）。


## 战略级需求

### Requirement 18: S-18 公式遵守度评测脚手架

**User Story:** 作为方法论守护者，我希望能持续验证模型是否真按 Trust/ConversionReadiness/EmotionalValue/NextBestActionScore 公式打分，以便 prompt 调整后能用数据决策而不是凭感觉。

#### Acceptance Criteria

1. THE 系统 SHALL 新增 MongoDB 集合 `evaluation_scenarios`，每条文档含 `scenario_id`、`title`、`description`、`workspace_id`、`account_id`（可选）、`contact_seed`（用于构造一个临时的 Contact 状态）、`inbound_messages: Vec<String>`、`ground_truth: { trust: i32, conversion_readiness: i32, emotional_value: i32, next_best_action_score: i32, notes: String }`、`tags: Vec<String>`、`status`、`created_at`、`updated_at`。
2. THE 系统 SHALL 新增接口 `POST /api/user-operations/evaluations/formula-adherence`，body 含 `accountId`、可选 `scenarioIds: Vec<String>`、可选 `tags: Vec<String>`，跑完后返回每个场景的 `{ scenario_id, predicted: { trust, conversion_readiness, emotional_value, next_best_action_score }, ground_truth: {...}, deviations: { trust: i32, ... }, adherence_score: 0..1 }`，以及汇总 `summary: { mean_adherence, by_formula: {...} }`。
3. THE 跑评测 SHALL 复用 `simulate_user_dialogue` 路径但每条 turn 抓取 `decision.formula_breakdown` 和 `review.scores`，对比 ground_truth 计算偏差（绝对差和方向一致性）。
4. THE 评测 SHALL 受 MP-5 全局 LLM 预算约束；超额时返回部分结果并标记 `degraded`。
5. THE 系统 SHALL 同时提供 `evaluation_scenarios` 的 CRUD 接口（`GET/POST /api/evaluation-scenarios`、`PUT/DELETE /api/evaluation-scenarios/:id`），以便后台导入和编辑场景。
6. THE 本次实现 SHALL 内置至少 1 个示例场景（`scenario_id="example_high_intent_user"`）作为脚手架；不构建大规模标注数据集。
7. IF `evaluation_scenarios` 集合在评测发起时为空（包括示例场景被人工删除的情况），THEN 系统 SHALL 以降级模式继续运行评测：返回 `200 OK` 含 `summary: { degraded: true, reason: "no_scenarios" }` 与空 `items`，且不抛出错误，以便 CI 流水线和 UI 自检不会因数据不全而中断。
8. THE 评测结果 SHALL 通过 `agent_events kind="formula_adherence_evaluated"` 留痕，便于事后查询历史评测。

### Requirement 19: S-19 长 horizon 用户运营 outcome 指标

**User Story:** 作为业务观测者，我希望能看到"Agent 接管后客户的真实长期表现"，以便判断方法论和提示词的真正效果。

#### Acceptance Criteria

1. THE 系统 SHALL 新增 task kind `outcome_aggregation`，由 `tasks::tick` 路由到一个新 handler；该任务每天 per-workspace per-account 跑一次，由系统自动调度（启动时检查并补建当日任务，避免遗漏）。
2. THE 聚合 handler SHALL 计算 per-account 维度的指标并写入新集合 `agent_outcome_metrics`（一条文档对应一个 `account_id` × `horizon` × `date`）：
   - `reply_rate_7d`、`reply_rate_30d`：发出消息后 7/30 天内有用户回复的比例
   - `conversation_depth`：每个 managed contact 当日内入站消息平均数
   - `human_handoff_success_rate`：当 events 中存在 `human_handoff` kind 时，该 contact 后来是否到达 `customer_success` 的比例
   - `agent_block_rate`：`decision_reviews.status="blocked"` 占总决策的比例
   - `daily_run_count`、`daily_run_token_total`：每日 run 次数与 token 累计
3. THE `agent_outcome_metrics` 集合 SHALL 配 TTL 索引（默认 90 天，可配置 `OUTCOME_METRICS_TTL_DAYS`）。
4. THE 系统 SHALL 新增接口 `GET /api/agent-outcome-metrics?accountId=&horizon=7d|30d&fromDate=&toDate=`，返回时间序列数组，便于前端画趋势图。
5. THE 聚合任务 SHALL 是幂等的：同 `account_id × horizon × date` 重跑只会更新已有文档，不会产生重复。
6. THE 聚合任务 SHALL 受 worker timeout 回收（HP-1）保护；单次执行超时 10 分钟也能被回收重试。
7. THE 本次实现 SHALL NOT 包含前端图表 UI（仅交付接口和数据），前端面板留作后续迭代。

### Requirement 20: S-20 Management Agent dry-run 模式

**User Story:** 作为后台操作员，我希望可以让 management agent 把"它打算做什么"先说清楚再执行，以便高风险操作可以二次确认。

#### Acceptance Criteria

1. THE `ManagementAgentSession` 模型 SHALL 新增 `dry_run: bool`（默认 false）。
2. THE `POST /api/management-agent/sessions` SHALL 接受 `dryRun: bool`，写入 session。
3. THE `ManagementMessageRequest` SHALL 新增可选 `dryRun: Option<bool>`，单次请求级别覆盖 session 的 `dry_run`。
4. WHEN 一次 management message 处理 `effective_dry_run = request.dryRun.unwrap_or(session.dry_run) == true`，THEN 系统 SHALL 对所有"非 read 类"工具调用替换为返回 `{ "dry_run": true, "would_execute": { "toolName": "...", "arguments": {...} } }` 而不实际执行。
5. THE "read 类"工具豁免列表 SHALL 含：`account_list`、`contacts_search`、所有 `knowledge.search` / `knowledge.open*` / `knowledge.list_catalog`，以及 `wechatagent.search_contacts`（只查询不写库）；`wechatagent.import_contacts` SHALL 归类为写工具，dry-run 时只返回 would_execute。
6. WHEN dry-run 模式触发，THEN `agent_command_runs.status = "dry_run"`、`agent_tool_calls.status = "dry_run"`、`response` 字段含 `would_execute`。
7. THE 后台 UI SHALL 始终显示"Dry-run 模式"复选框（无论 session 当前是否处于 dry-run），创建 session 时可勾选默认值；session 详情页固定展示当前模式徽章；单条消息发送也可临时勾选覆盖，便于操作员随时切换。
8. WHEN dry-run 模式下模型计划包含 `wechatagent.send_contact_message`，THEN 即使 `apply_locked_send_content` 提取了锁定内容，也仍只 dry-run 不发；锁定内容回放在 `would_execute.arguments.content` 中以便确认；IF 锁定内容提取失败或解析异常，THEN 系统 SHALL 仍以 dry-run 形式返回，把失败原因写入 `would_execute.arguments.content`（如 `"<extraction_failed: ...>"`）和 `would_execute.error` 字段，让操作员能看见问题再决定。
9. THE PBT/集成测试 SHALL 验证：dry-run 模式下任意非 read 工具的调用不会修改 MongoDB 任何业务集合（仅写 `agent_tool_calls` 和 `agent_command_runs` 审计记录）。

## Glossary

- **Agent run**：一次入站消息或一条到期跟进任务触发的完整决策链路；通过 `run_id`（UUID）贯穿 `agent_run_logs`、`decision_reviews`、`llm_call_logs`。
- **决策评审 (Decision Review)**：`AgentDecisionReview`，每次 run 生成的一条审计快照，含决策、评分、命中知识、风险、网关结果、用户反应。
- **memoryCard**：`OperatingMemory.memory_card` 中的紧凑长期记忆卡片，由 `memory_consolidator` Agent 异步整理后供运行时注入 prompt。
- **coreFacts**：本次新引入的 memoryCard 子结构，存放重要度高、不应被新近性挤出的事实（cap 6）。
- **recentFacts**：本次新引入的 memoryCard 子结构，存放滚动窗口的次要事实（cap 10）。
- **knowledge chunk**：`OperationKnowledgeChunk`，知识库三件套（document/item/chunk）中的最小可注入单元。
- **verified chunk**：`integrity_status="verified"` 的知识切片，是当前运行时 prompt 唯一可见的产品事实来源。
- **claim**：候选回复中关于我方产品/服务的事实性表述（能力、价格、案例、效果、交付、承诺等）。
- **fact risk**：评审打分中"事实风险"维度（0-10），≥6 默认禁发。
- **pressure risk**：评审打分中"销售压迫感风险"维度（0-10），≥7 默认禁发。
- **状态机 (State Machine)**：`operation_domain_configs.user_operations.state_machine`，9 个用户运营状态及其迁移规则。
- **playbook**：`OperationPlaybook`，账号级运营方法论文档，被注入 prompt 影响决策风格。
- **soul**：`AgentSoul`，每个 Agent 类型（user/management/group/moment）的人格层提示词。
- **domain config**：`OperationDomainConfig`，按 `domain` 划分的运行参数、工作流、自动化和评审策略。
- **dry-run**：管理 Agent 的"只规划不执行"模式，所有非读类工具返回模拟结果。
- **公式遵守度**：模型在决策中按 prompt 中给定公式（Trust/ConversionReadiness/EmotionalValue/NextBestActionScore）打分的一致性程度。

## Out of Scope

本次不做的事：

1. **group/moment Agent 的运行时实现** — 仅修正其种子状态为 `draft`，UI 区分展示
2. **知识库三件套（document/item/chunk）的导入解析逻辑重写** — 仅引入未验证告警和批量自动校验接口
3. **分布式调度器** — worker 仍是单进程多 tick，仅引入 running 超时回收
4. **LLM provider 抽象层重构** — 仍直接对接 OpenAI 兼容接口
5. **公式遵守度评测的 ground-truth 数据集构建** — 仅交付脚手架和一个示例场景
6. **管理 Agent 工具风险分级体系重构** — 仅新增 dry-run 模式

## Trade-offs

- **`last_inbound_at` 引入需要数据迁移**：对存量 contact 用现有 `last_message_at` 一次性回填到 `last_inbound_at`；保留 `last_message_at` 为兼容字段一段时间（取 last_inbound_at 与 last_outbound_at 较大者）。
- **`claimed_at` 字段对老任务兼容**：缺失视为 stale，第一次 tick 即可重置，但要避免误回收正在跑的老任务（用部署时间戳作为下界）。
- **字符串级 fact-risk fallback 可能误伤合理表达**：例如"我会保证准时回复你"中的"保证"。需提供白名单（同存在 prompt_templates）覆盖；模型在 claim_analysis 已声明 `requiresProductKnowledge=false` 时跳过字符串扫描。
- **长 horizon metrics 增加 Mongo 写压**：每天每 workspace 一次聚合，按账号和联系人粒度写 `agent_outcome_metrics`。建议给该集合加 TTL（90 天）。
- **状态机 `allowedFrom` 限制**：可能挡住合理但少见的状态迁移。需要 `cooldown` 这种"任意状态可进入"的特殊标记，并给运营人员保留 UI 编辑 allowedFrom 的能力。
- **LP-11 mega 文件拆分**：纯机械重构但 PR 体积大，依赖现有测试（要先补齐 LP-16 才安全推进）。

## Correctness Properties (Property-Based Tests)

本 spec 必须由以下 3 条以上 PBT 属性背书：

1. **状态机迁移合法性**：对任意 contact 当前 state `s_from`，模型决策的 `s_to` 必须满足 `s_to.allowedFrom ⊇ {s_from}` 或 `s_to.allowFromAny == true`（仅 cooldown）；否则 review 必拦截，fact_risk ≥ 6。
2. **memoryCard 数组不变量**：任意 consolidator 输出后 `compact_memory_card` 的结果满足 `coreFacts.length ≤ 6 ∧ recentFacts.length ≤ 10`；且对于一个明确写入 coreFacts 的 fact，在没有 consolidator 显式 deprecate 它的前提下，N 轮后续 consolidation 后它仍在 coreFacts 中。
3. **任务 claim 幂等性**：对任意 task，N 个并发 worker 同时调用 claim（`update_one(status:cur→running)`），至多 1 个 modified_count=1；同时 N 次 reaction analysis claim（`outcome_status:pending→analyzing`）也至多 1 个成功。
