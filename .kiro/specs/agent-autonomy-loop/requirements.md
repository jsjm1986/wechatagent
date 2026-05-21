# Requirements Document

> 中文标题：用户运营 Agent 自治回路（agent-autonomy-loop）— 需求文档

## Introduction

本次升级在已经稳定运行的 WechatAgent 用户运营 Agent 链路（`run_user_operation_gateway` 为核心，已通过 78 个 lib unit test + 33 个 PBT）之上，把"后端规则驱动 + Agent 辅助"重塑为"Agent 自治驱动 + 后端只做安全边界 + 双层标签 + 可靠发送闭环"。

**产品定位（不可妥协）**：本系统是**全 AI 自治流程**，不引入"人工接管 / 等人工处理"概念。所有"暂缓发送"路径必须用 AI 内部状态语义命名（如 `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context`），后台 UI 可让运营人员**观察**这些状态，但业务语义上 **不存在 "human takeover"**。

升级目标分四层：

1. **自运行 (Self-Run)**：每轮 Reply Agent 主动决定要不要查知识、要不要写记忆、要不要回复，不再由 Rust 控制流写死。知识查询从"路由 LLM 一次性塞 chunks"改为"Reply Agent 通过 MCP tool-calling 协议按需 list_catalog → search → open_slice"。
2. **自治 (Self-Govern)**：业务字段（`risk_level / knowledge_need / run_mode / autonomy_mode / needs_review / operation_state / consolidation_needed`）由 Agent 输出，Rust 只做"必填校验 + 安全边界拦截"，不再替 Agent 兜底默认值。**沿用现有枚举语义**（不改名 `runMode = fast_chat / memory_candidate / knowledge_grounded / high_risk`、`knowledgeNeed = not_required / required / insufficient`），仅**新增** `autonomy_mode = auto / assisted / blocked` 描述自治控制状态。
3. **自优化 (Self-Improve)**：Review Agent 反馈下，Reply Agent 能在单次 run 内自我修正一次（SelfCritique + Single-Shot Revision）；记忆冲突时 Memory Agent 主动判断哪条过期。
4. **可靠发送闭环 (Reliable Outbox)**：决策通过 review 后必须经过持久化 outbox + 幂等 key + 发送前二次安全门 + 失败重试 + 完整 trace；用户拒绝 / cooldown 后旧的 outbox entry 必须被取消，不能继续发送。

强约束（不可放松）：

- **不**让 Agent 自由生成 `operation_state` key（保留状态机字典 + `check_state_transition` 校验）。
- **双层标签**：`customer_stage / intent_level / objection_type` 严格走 `system_taxonomies` 字典（用于聚合 / 报表 / 流程）；同时新增 `agent_generated_signals` 自由维度（带 evidence/confidence），不在字典里的"想法"进入 `taxonomy_candidate` 候选集合，由后台审核后并入字典；**未审核的候选 SHALL NOT 阻塞运行**。
- **保留**生产安全边界：`fact_risk / pressure_risk` 阈值、verified knowledge 强约束、状态机 `allowedFrom` 校验、单 run 预算上限、幂等、限流、审计。
- **保留**当前测试基线（合并前 SHALL 自动核验）：本期升级前已有 78 个 lib unit test + 33 个 PBT（6+9+6+12 across `state_transition_pbt / memory_card_invariants / string_fact_risk_guard / llm_retry_jitter`）；CI 在合并前 SHALL 通过 `cargo test --lib` 与上述 4 个 PBT 文件得到 ≥ 78 + ≥ 33 通过、0 失败的实测基线，否则阻断合并；不破坏向后兼容（老 `Vec<String>` 形态的 `coreFacts` 反序列化必须 OK）。
- **灰度兼容有 sunset**：`autonomyProtocolEnabled / knowledgeRoutingMode=classic_router` 等回退开关 SHALL 在迁移完成且 7 天内全量 run 都满足新协议后被移除（详见 R11）；不做"长期双轨"。

## 状态枚举映射表（gateway_status × finalReviewStatus）

本期定义两层状态，**不混用**：

- **`gateway_status`**：单次 run 进行中的过程状态（运行时分支用），写入 `agent_run_logs.gateway_status`。
- **`finalReviewStatus`**：归档后用于统计聚合的最终状态（前端 horizon 指标用），写入 `agent_run_logs.finalReviewStatus`。

| 触发条件（运行时分支）             | gateway_status                          | finalReviewStatus                  |
| ---------------------------- | --------------------------------------- | --------------------------------- |
| 一次 run 正常通过 review            | `approved`                              | `approved`                        |
| 触发 1 次 revision 后通过 re-review | `approved`                              | `revision_applied_approved`       |
| revision 后 re-review 仍 fail     | `revision_failed`                       | `revision_failed`                 |
| `needsRevision=true` 但 `revisionDirection` 空 | `revision_skipped_invalid_direction` | `revision_failed`                 |
| revision 之前预算超额                | `revision_skipped_budget_exceeded`      | `revision_failed`                 |
| revision LLM 调用超时 / 不可解析       | `revision_llm_failure`                  | `revision_failed`                 |
| Review `shouldHold=true`，holdCategory=`held_by_ai_policy`         | `held_by_ai_policy`                    | `held_by_ai_policy`               |
| Review `shouldHold=true`，holdCategory=`blocked_by_safety_guard`   | `blocked_by_safety_guard`              | `blocked_by_safety_guard`         |
| Review `shouldHold=true`，holdCategory=`ai_waiting_for_more_context` | `ai_waiting_for_more_context`        | `ai_waiting_for_more_context`     |
| 必填字段 missing/invalid             | `blocked_by_required_field`             | `blocked_by_required_field`       |
| 预算超额 + needs_review=true         | `blocked_by_budget`                     | `blocked_by_budget`               |
| 产品声明无 verified knowledge         | `blocked_unverified_product_claim`      | `blocked_unverified_product_claim`|
| 工具循环 30s 总超时                  | `tool_loop_timeout`                     | `blocked_by_safety_guard`         |
| 工具循环耗尽 MAX_TOOL_LOOPS         | `approved` 或本表其它分支（取决于最终轮决策）| 按最终轮决策落                      |
| 灰度 `autonomyProtocolEnabled=false` 下校验失败 | `legacy_mode_unchecked`                 | `legacy_mode_unchecked`           |

**说明**：

- `gateway_status` 多于 `finalReviewStatus`，因为前者是过程态、后者只关心最终归档结果。多个过程态可以聚合到同一个 finalReviewStatus（例如 4 类 revision 失败都归档为 `revision_failed`）。
- `legacy_mode_unchecked` 是合法 finalReviewStatus（详见 R9.2 / R11.4），仅出现在灰度回退路径；该状态在 R10 自治指标聚合中按"未升级"独立计数显示，不计入新指标的分子分母（避免污染 revision_trigger_rate / blocked_by_safety_guard_rate 等）。
- 任何在本表之外出现的 `gateway_status` / `finalReviewStatus` 值 SHALL 视为协议违规并阻断写库（R9.10.e）。
- outbox 链路使用独立的 `outbox.status` 枚举（详见 R13.1），与上述两套状态正交，三者通过 `run_id / decision_id` 关联但不互相覆盖。


## Glossary

- **Reply Agent (UserOpsBrain)**：核心 LLM 决策器，每轮入站消息或跟进任务上下文喂入后输出 `AgentDecision`，负责自治协议字段、文案、记忆候选、状态变更建议、tool calls。
- **Review Agent**：评审 LLM，输出 `DecisionReviewResult`，本期新增 `needsRevision / revisionDirection / shouldHold / holdReason / holdCategory / selfCritiqueAddressed` 字段。
- **Knowledge Agent (MCP tool surface)**：以 MCP tool-calling 协议被 Reply Agent 调用的知识查询器，对外暴露 `knowledge.list_catalog / knowledge.search / knowledge.open_slice` 三个工具；不再是独立 LLM 角色。
- **Memory Agent (Consolidator)**：异步整理 `memoryCard` 的 LLM，本期需要主动产出 `deprecatedFacts` 与 `conflicts` 元数据。
- **AgentDecision**：`src/agent/types.rs::AgentDecision` 中定义的 Reply Agent 输出结构，本期需要扩字段。
- **DecisionReviewResult**：`src/agent/types.rs::DecisionReviewResult` 中定义的 Review Agent 输出结构，本期需要扩字段。
- **MemoryFact**：本期新增的强类型记忆事实结构，含 `text / evidence / confidence / importance / mayExpire / deprecatedAt`，通过 `#[serde(untagged)]` 兼容老 `Vec<String>`（R11 迁移后老格式被物理移除）。
- **MemoryCardTyped**：本期把 `OperatingMemory.memory_card` 整层升级为强类型 struct，不再以 `Document` 为主要表示（R6 全量替换，不止局部 helper 转换）。
- **system_taxonomies**：本期新增的 MongoDB collection，存放 `customer_stage / intent_level / objection_type` 等**严格字典**（不含自由 `tags`），每个字典含 `key / displayName / description / aliases / status`。
- **taxonomy_candidate**：Agent 输出但不在 `system_taxonomies` 中的值落入此集合（含 evidence / confidence / first_seen_at / occurrences），由后台审核 → 通过 → 并入正式字典；候选状态 **SHALL NOT 阻塞 Reply Agent 运行**。
- **agent_generated_signals**：Reply Agent 自由生成的"对真实用户的理解"维度（自由 tag、行为信号、关系判断），保存在 `agent_generated_signals` 字段中，**不参与统计聚合**，仅供 Agent 后续自我引用与人审审计。
- **AutonomyMode**：本期新增的自治控制枚举 `auto / assisted / blocked`，与现有 `runMode` 正交：`runMode` 描述"运行链路类型"（fast_chat / knowledge_grounded / high_risk），`autonomyMode` 描述"本轮 Agent 自主权范围"（auto = 全权决策；assisted = 需 Review 拍板；blocked = 由安全门拦截）。
- **Tool Call Protocol**：本期定义的 Reply Agent → MCP tools 调用协议（基于现有 LLM JSON 决策流之上的"toolCalls"数组 + Rust 多轮派发 + 结果回注），详见 R4。
- **Outbox**：本期新增的发送事务表 `agent_send_outbox`，承担"决策落地 → 实际发送"之间的可靠链路（持久化 / 幂等 / 重试 / 取消），详见 R13。
- **Self-Critique**：Reply Agent 对自身候选回复的内省字段（`selfCritique / whyShouldReply / whySkipReply / riskSelfCheck`）。**长度按风险级别条件化**（详见 R1.4 / R1.5）。
- **Single-Shot Revision**：当 Review Agent 返回 `needsRevision=true` 时，Reply Agent 拿着 `revisionDirection` 重写一次，但同一次 run 内最多重写 1 次；再 fail 即落入 hold/block。
- **Run Budget**：`src/agent/budget.rs::RunBudget`，单次 run 的 token、LLM 调用次数、tool call 次数预算（本期 tool calls 也计入）。
- **Operation State Machine**：`prompts::default_user_operation_state_machine` 返回的状态机 Document，含 `states[].key` 与 `allowedFrom / allowFromAny`。
- **Verified Knowledge Chunk**：`OperationKnowledgeChunk.integrity_status == "verified"` 的知识切片（与现有数据模型一致，**不引入新字段** `verified: bool`），是产品声明类回复的唯一可信源。
- **Local Decision Review**：`src/agent/review.rs::local_decision_review`，预算超额或评审跳过时返回的本地兜底评审结果；本期需要修改其语义（详见 R3）。
- **Hold Category**：替代 "human takeover" 的 AI 内部状态分类，枚举 `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context`，由 Review Agent 输出 + Rust 校验，**不暗示需要人工干预**。
- **Decision Phase**：本期新增的 Reply Agent 输出阶段标识 `decision_phase ∈ {"tool_calling", "final"}`：
  - `tool_calling` 表示这是 tool-loop 中间轮（toolCalls 非空、reply_text 必为空、should_reply 必为 false），Rust **只**校验 tool schema、不校验自治协议 9 字段；
  - `final` 表示这是工具循环结束的最终决策轮，Rust 完整校验 R1 自治协议 9 字段、R3 必填、R5 verified knowledge 等所有 review 规则。
  详见 R1.10、R4.1.b。
- **Run Envelope**：本期新增的 run 生命周期信封，由 `agent_run_logs` 在 run 入口创建（lifecycle="started"）、过程中切到 "running"、最终落 "completed" / "failed_before_decision" / "failed_after_decision"，确保任何 LLM 超时 / JSON 解析失败 / 中间崩溃都有可追溯条目（详见 R0）。
- **Source Event ID**：触发本次 run 的入站消息 / 跟进任务 ID（即 `inbound_message_id` 或 `task_id`），用于 R13 outbox 的强幂等 key 计算，确保即使 run_id 重生成也不会触发重复发送。

## Requirements

### Requirement 0: Agent Run Lifecycle / Run Envelope（生产排障底座）

**User Story:** 作为生产排障人员，我希望任何一次 run（无论后面是否走完 Reply→Review→outbox）从入口的那一刻就在 `agent_run_logs` 留下信封记录；即使 LLM 超时、JSON 解析失败、Rust panic 也能追溯到"哪个 contact、哪条消息、哪一步、几点几分崩了"。这是发送闭环 / 自治审计 / outbox trace 的共同底座，必须最先就位。

#### Acceptance Criteria

1. **入口立即写信封**：WHEN `run_user_operation_gateway` / `handle_managed_message` / `handle_follow_up_task` 任一入口被触发，THE 系统 SHALL 在调用任何 LLM / 知识库 / Reply Agent 之前，先以 `lifecycle="started"` 在 `agent_run_logs` **insert_one** 一条信封记录，含 `run_id / account_id / contact_wxid / source_event_id / source_kind ∈ {"inbound_message", "follow_up_task", "manual_send"} / started_at`，**且**初始化 `gateway_status="pending"` 与 `finalReviewStatus=""`（空字符串占位，后续才落最终值）。
2. **后续阶段必须用 update_one，禁止 re-insert**：THE 现有 `write_agent_run_log` SHALL 从 `insert_one` 改写为 `update_one({run_id})` + `$set` 全部最终字段（`gateway_status / finalReviewStatus / lifecycle / planner / context / decision / review / gateway_result / error / token_budget / tokens_used / llm_calls_used / degraded_reasons / revisionApplied / revisionReason / pre,postRevisionSummary / selfCritique / autonomyMode` 等）；
   - **禁止再次 `insert_one(agent_run_logs)`**：因为 `(run_id)` 已建唯一索引（见 `src/db/indexes.rs:236`），二次 insert 会返回 DuplicateKey 错误；本期 SHALL 视任何"在同一 run 期内对 `agent_run_logs` 再次 insert"为协议违规并阻断 PR；
   - update 失败（matched_count == 0）SHALL 视为信封丢失或 run_id 错位 → 写 `tracing::error!("agent_run_envelope_missing run_id={run_id}")` 并尝试**单次** `insert_one` 兜底（仅在 matched_count==0 这种异常场景，正常路径必须走 update）；该兜底 insert 路径 SHALL 在 `agent_events` 留痕 `kind="run_envelope_recovered_via_insert"`，便于发现哪些上游路径漏写信封。
3. **lifecycle 枚举**：THE `agent_run_logs.lifecycle` 取值 SHALL 严格属于以下集合：`started / running / completed / failed_before_decision / failed_after_decision / aborted_by_budget / aborted_by_external_signal`。
4. **状态推进义务**（用 update_one 推进，不重写 envelope）：
   - IF Reply Agent 输出 LLM 超时 / JSON 解析失败 / Rust panic 发生在 Review 之前 / decision 写出之前，THEN lifecycle SHALL 推进为 `failed_before_decision`，且 `error_summary` 字段（≤ 1024 chars）SHALL 含错误类别（`llm_timeout / json_parse_error / unhandled_panic / mcp_error / budget_pre_check_failed`）与简要 message；
   - IF 失败发生在 decision 已写但 review/outbox 之前，THEN lifecycle SHALL 推进为 `failed_after_decision`，并保留已落字段；
   - WHEN run 在预算硬上限触发时被强制结束（详见 R3.7 / R4.3），THE lifecycle SHALL 推进为 `aborted_by_budget`；
   - WHEN run 因外部信号（用户拒绝 / cooldown / dry-run skip）在中途被取消，THE lifecycle SHALL 推进为 `aborted_by_external_signal`，且 `abort_reason` 字段（≤ 256 chars）含取消原因。
5. **失败也必须写**：THE 系统 SHALL **保证** 任何走到 `lifecycle ∈ {failed_before_decision, failed_after_decision, aborted_by_budget, aborted_by_external_signal}` 的 run 都至少有一条 `agent_run_logs` 记录；这意味着信封写入与"是否成功决策"完全解耦——R0.1 的入口写入 SHALL 在 try/catch 之外，SHALL 不依赖任何其它字段就绪。
6. **panic 兜底**：IF Rust 运行时发生 panic（用 `std::panic::catch_unwind` 或 `tokio::spawn` 上层捕获），THEN 系统 SHALL 在 panic 处理器中尝试把 lifecycle 推进为 `failed_before_decision` 或 `failed_after_decision`（按已落字段判断），写 `error_summary="unhandled_panic: <panic message>"`；IF 写库本身也失败，THEN SHALL 把错误日志输出到 `tracing::error!` 并不再尝试（避免 panic-in-panic）。
7. **trace 关联**：THE `run_id` SHALL 是后续所有 `agent_events / agent_decision_reviews / agent_send_outbox` 记录的关联键；`source_event_id` SHALL 是 R13 outbox 强幂等 key 的核心成分（详见 R13.1）。
8. **索引**：THE `agent_run_logs` SHALL 在 `(account_id, lifecycle, started_at)` 建立复合索引，便于 `lifecycle != "completed"` 的 run 用于运维报警查询。
9. **前端展示**：THE 前端"运营成效中心"的"自治回路监控"Tab SHALL 新增"未完成 run 数"指标卡（`lifecycle != "completed" / total_runs`，按 horizon 聚合），值 > 0 时高亮为橙色；列表 SHALL 支持筛选 `lifecycle ∈ {failed_before_decision, failed_after_decision, aborted_by_budget, aborted_by_external_signal}` 看具体错误。
10. **单元测试** SHALL 覆盖：(a) 入口写信封先于任何 LLM 调用（mock LLM 抛异常前 lifecycle 已 = "started"）；(b) lifecycle 状态机不接受非法转换（`completed → started` 写库 SHALL panic 或 error）；(c) Reply Agent panic 后 lifecycle 终态 = `failed_before_decision` 且 `error_summary` 非空；(d) 预算超额触发 run 取消时 lifecycle = `aborted_by_budget`；(e) 同一 run_id 二次 `insert_one` SHALL 因 unique 索引返回 DuplicateKey 错误（验证禁止 re-insert 约束）；(f) `write_agent_run_log` 用 `update_one` 在不存在 envelope 时走兜底 insert + 写 `kind="run_envelope_recovered_via_insert"` 事件。

### Requirement 1: Reply Agent 自治协议升级（UserOpsBrain 9 字段，按风险条件化长度）

**User Story:** 作为运营策略维护者，我希望 Reply Agent 每轮都把"为什么这样判断"显式写出来（用户理解 / 关系判断 / 运营目标 / 知识需求理由 / 记忆更新理由 / 自我批判 / 该不该回复 / 风险自检），但低风险常规闲聊轮要简短、关键变化轮要完整，这样我能在审计端复核因果链而不被噪音淹没、也不为每条寒暄付昂贵 token。

#### Acceptance Criteria

1. THE AgentDecision SHALL 新增字段 `userUnderstanding: String`、`relationshipRead: String`、`operationGoal: String`、`knowledgeNeedReason: String`、`memoryUpdateReason: String`、`selfCritique: String`、`whyShouldReply: String`、`whySkipReply: String`、`riskSelfCheck: String`，统称"自治协议 9 字段"。每个字段长度 SHALL ≤ 600 字符（utf-8 char 计数）。
2. THE Reply Agent prompt SHALL 在 schema 段强制要求这 9 个字段全部输出；输出 schema 由 prompt_template 中的"自治协议输出契约"段落定义；prompt SHALL 同时给出"低风险简短 / 关键变化完整"的填写指引（详见 R1.5 / R1.6）。
3. **基础必填规则**（按字段拆分，互斥）：
   - 7 个字段（`userUnderstanding / relationshipRead / operationGoal / knowledgeNeedReason / memoryUpdateReason / selfCritique / riskSelfCheck`）**始终必填**：WHEN 任一字段为空字符串或仅含空白字符（U+0020/U+0009/U+000A/U+000D），THE 系统 SHALL 视为协议违规，把该字段名以 `missing_required_field:<fieldName>` 形式追加进 `review.risks`，并将 `review.approved` 置为 `false`、`gateway_status` 设为 `"blocked_by_required_field"`、`finalReviewStatus="blocked_by_required_field"`。
   - 2 个字段（`whyShouldReply / whySkipReply`）走**条件必填**（详见 R1.5 / R1.6），**不**进入 R1.3 的"7 字段任一为空 → 违规"判定，避免正常回复轮被误判。
4. **回复理由互斥必填**：
   - WHEN `decision.should_reply == true`，THE `whyShouldReply` SHALL ≥ 10 个 Unicode 字符（含至少 6 个汉字 U+4E00..U+9FFF）；`whySkipReply` 此时允许为空字符串（合法）。
   - WHEN `decision.should_reply == false`，THE `whySkipReply` SHALL ≥ 10 个 Unicode 字符（含至少 6 个汉字）；`whyShouldReply` 此时允许为空字符串（合法）。
   - 违反 R1.4 SHALL 在 `review.risks` 追加 `missing_required_field:whyShouldReply` 或 `missing_required_field:whySkipReply` 并 `review.approved = false`。
5. **条件化长度规则**（基于现有 `risk_level` 枚举 `low / medium / high`，**不引入** `critical`）：THE 字段最小长度 SHALL 按本轮风险级别判定：
   - WHEN `decision.risk_level == "low" AND decision.knowledge_need == "not_required" AND decision.consolidation_needed == false`（"低风险常规轮"），THE 7 个 R1.3 必填字段中允许 5 个字段（`userUnderstanding / relationshipRead / operationGoal / memoryUpdateReason / riskSelfCheck`）取以下任一**短形式**：(a) `"unchanged"`（≤ 1 个汉字也合法），表示"与上一轮相同"；(b) ≤ 30 个 Unicode 字符的简短陈述；其余 2 个 R1.3 必填字段（`knowledgeNeedReason / selfCritique`）SHALL ≥ 6 个 Unicode 字符的实质内容（防止 Agent 全部偷懒）。
   - IF `decision.risk_level == "high" OR decision.run_mode == "high_risk" OR decision.knowledge_need ∈ {"required", "insufficient"} OR decision.consolidation_needed == true OR 本轮触发 operation_state 迁移`（"关键变化轮"），THEN ALL 7 个 R1.3 必填字段 SHALL 不得使用 `"unchanged"` 短形式，且每个字段 SHALL ≥ 20 个 Unicode 字符的实质性内容；违反 SHALL 追加 `risks` 标记 `insufficient_detail_in_critical_turn:<fieldName>` 并 `review.approved = false`。
   - 关键变化轮的判定 **同时包含** `risk_level == "high"` 与 `run_mode == "high_risk"` 两个独立路径，因为 `risk_level` 与 `run_mode` 在 R3 中是正交字段；任一命中即视为关键变化轮。
6. **回复理由长度延伸**：在 R1.4 必填基础上，关键变化轮（同 R1.5 第二条）下的 `whyShouldReply / whySkipReply`（按 `should_reply` 命中那一个）SHALL ≥ 30 个 Unicode 字符（含至少 12 个汉字）。
7. THE 自治协议 9 字段 SHALL 全部以 `String` 类型落入 `agent_run_logs.decision` 与 `agent_decision_reviews.decision` 中，便于审计端原文读取。
8. WHERE 老版本 Agent 输出未携带 9 字段（例如向后兼容回放历史 run），THE 反序列化 SHALL 兼容缺失（按空字符串落入 `AgentDecision`），但 R1.3 / R1.4 / R1.5 的校验 SHALL 仍然触发（除非 `autonomyProtocolEnabled=false` 灰度，详见 R11）。
9. THE 前端运营成效中心 SHALL 在"决策审计"详情面板中按字段顺序展示这 9 个字段（中文标签：用户理解 / 关系判断 / 运营目标 / 知识需求理由 / 记忆更新理由 / 自我批判 / 该回复理由 / 不回复理由 / 风险自检），值为 `"unchanged"` 时 SHALL 显示为灰色"沿用上轮"占位，值为空但属于 R1.4 的"互斥允许空"情形 SHALL 显示淡灰色"—"占位（不视为协议违规），值为空但属于 R1.3 必填情形 SHALL 显示红色"协议违规"占位。
10. **decision_phase 门控**（与 R4 tool-loop 协作）：THE `AgentDecision` SHALL 新增字段 `decision_phase: String`，允许枚举为 `"tool_calling"` 与 `"final"`：
    - WHEN `decision_phase == "tool_calling"`（tool-loop 中间轮），THE 系统 SHALL **跳过** R1.3 / R1.4 / R1.5 / R1.6 全部校验，仅校验 R4.1 的 toolCalls JSON schema；中间轮的自治协议 9 字段允许全部为空（不视为协议违规）；
    - WHEN `decision_phase == "final"`（工具循环结束的最终决策轮），THE 系统 SHALL 完整执行 R1.3 / R1.4 / R1.5 / R1.6 / R3 / R5 全部 review 校验；
    - IF `decision_phase` 字段缺失或非以上两个枚举值，THEN 默认视为 `"final"`（保守 + 触发完整校验），并在 `risks` 追加 `"decision_phase_invalid:<value>"`。
11. THE 单元测试 SHALL 覆盖：(a) `should_reply=true` 时 `whyShouldReply` 必填、`whySkipReply` 允空（与原 R1 矛盾被修复）；(b) `should_reply=false` 时反之；(c) 低风险常规轮允许 5 字段 `unchanged`；(d) 高风险轮（`risk_level=high` 或 `run_mode=high_risk`）拒绝 `unchanged`；(e) `risk_level="critical"` 触发 `invalid_enum_value`（与 R3.2 一致，本期不引入 critical）；(f) `decision_phase="tool_calling"` 时即使 9 字段全空也不触发协议违规；(g) `decision_phase="final"` 时按 R1.3 / R1.4 / R1.5 完整校验。

### Requirement 2: SelfCritique + Single-Shot Revision

**User Story:** 作为产品经理，我希望 Review Agent 在发现回复有问题时不只是打分，而是直接给"应该往哪个方向改"的反馈让 Reply Agent 改一次再发；如果实在不能发就由 AI 自己暂缓而不是踢给人工，这样能把可发文案率提上去同时不破坏全 AI 自治流程的产品定位。

#### Acceptance Criteria

1. THE DecisionReviewResult SHALL 新增字段 `needsRevision: bool`（默认 false）、`revisionDirection: String`（≤ 1024 chars，默认空）、`shouldHold: bool`（默认 false）、`holdReason: String`（≤ 512 chars，默认空）、`holdCategory: String`（默认空）、`selfCritiqueAddressed: bool`（默认 false）。
2. THE `holdCategory` 允许枚举值 SHALL 严格为以下三选一：`held_by_ai_policy`（AI 策略主动暂缓 / 例：用户明确拒绝刚发生不久）、`blocked_by_safety_guard`（安全门拦截 / 例：产品声明无 verified knowledge）、`ai_waiting_for_more_context`（AI 自评信息不足，等下一轮入站再决策）；**严禁** 出现 `held_for_human / human_required / waiting_for_human` 等暗示人工接管的取值。
3. WHEN Review Agent 输出 `needsRevision == true AND shouldHold == false AND 当前 run 尚未发生过 revision AND revisionDirection 非空非空白`，THE 系统 SHALL 触发 Reply Agent 第二次调用，传入 `revisionDirection` 与原 decision 作为输入，并把第二次调用结果作为最终 decision 进入第二轮 review。
4. THE 系统 SHALL 在单次 run 内最多触发 1 次 revision；IF 第二次 review 仍 `approved == false`，THEN 系统 SHALL 不再继续 revise，把 `decision.should_reply` 强制改为 `false`，把 `gateway_status` 设为 `"revision_failed"`。
5. IF Review Agent 输出 `needsRevision == true` 但 `revisionDirection` 为空或仅含空白，THEN 系统 SHALL 不触发 revision，把 `gateway_status` 设为 `"revision_skipped_invalid_direction"` 并 fail-closed（不发送）。
6. **AI 策略性暂缓**：IF Review Agent 输出 `shouldHold == true`，THEN 系统 SHALL 不发送、不再触发 revision；THE `gateway_status` SHALL 等于 `holdCategory` 取值（`held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context` 之一）；IF `holdCategory` 为空或不在枚举内，THEN 默认填 `held_by_ai_policy`，并在 `agent_events` 写一条 `kind="autonomy_hold_category_invalid"` 事件，detail 含原始 `holdCategory` 值；THE `holdReason` 必须 ≥ 10 个 Unicode 字符的非空理由。
7. **业务语义保护**：THE 系统 SHALL NOT 在任何 `gateway_status / agent_events.kind / 前端文案` 中出现 `human / 人工 / 接管 / takeover / hand-off` 等词；CI 中 SHALL 加一条文本 lint 检查 `src/agent/ src/routes/ frontend/src/` 下所有新增字符串字面量不含上述词（已存在的"人工抽查""人审"在波 D 之前的产品文案保留，仅限制本期新增内容）。
8. IF 触发 revision 之前 `current_run_budget().is_exceeded() == true`，THEN 系统 SHALL 不触发 revision，把 `gateway_status` 设为 `"revision_skipped_budget_exceeded"` 并 fail-closed（不发送）。
9. WHEN revision 被触发，THE 系统 SHALL 把"是否触发 revision / 触发原因 / revision 前 decision 摘要 / revision 后 decision 摘要"四项写入 `agent_run_logs.revisionApplied / revisionReason / preRevisionSummary / postRevisionSummary` 字段（详见 R9）。
10. THE 第二次 Reply Agent 调用 SHALL 在 prompt 中显式包含上一轮的 `selfCritique` 与本轮 Review 的 `revisionDirection`，且要求新输出的 `selfCritique` 字段 SHALL 解释"为什么这次改写已经解决了上一轮的问题"。第二轮 review SHALL 据此显式置 `selfCritiqueAddressed = true / false`。
11. IF 第二次 Reply Agent 调用超时（> 30 秒）或返回不可解析 JSON，THEN 系统 SHALL 把 `gateway_status` 设为 `"revision_failed"`，`decision.should_reply = false`，并写 `agent_events kind="revision_llm_failure"`，event detail 含 `error_summary / latency_ms / attempt_count`。
12. THE 单元测试 SHALL 覆盖：(a) needsRevision=true 触发 1 次 revision、(b) revision 后再 fail 走 block、(c) revision 之前预算超额走 fail-closed、(d) shouldHold=true + holdCategory=held_by_ai_policy 走 hold 不 revise、(e) holdCategory="held_for_human" 视为非法被强制改为 held_by_ai_policy 并写事件、(f) revisionDirection 空走 revision_skipped_invalid_direction、(g) revision LLM 30s 超时走 revision_failed。

### Requirement 3: Rust 不再替 Agent 兜底业务字段（保留现有枚举语义 + 新增 autonomy_mode）

**User Story:** 作为系统架构师，我希望 Rust 控制流不再替 Agent"猜默认值"，但**不要**在迁移过程中破坏已经在生产跑的 `runMode / knowledgeNeed` 语义；改造范围限于"必填校验 + 新增 autonomy_mode 自治控制位"，老枚举一字不改。

#### Acceptance Criteria

1. THE 系统 SHALL 视 `AgentDecision` 中以下 7 个字段为必填字段：`risk_level / knowledge_need / run_mode / autonomy_mode / needs_review / operation_state / consolidation_needed`。
2. **沿用现有枚举不重命名**：
   - `risk_level` 允许枚举 SHALL **严格**为 `low / medium / high`（**保留现状**）；本期 **不引入** `critical` 取值；R1.5 / R5 / 任何下游分支引用 "critical" 视为非法 → 触发 `invalid_enum_value:risk_level:critical`；高风险路径全部走 `risk_level=high` 或 `run_mode=high_risk`。
   - `knowledge_need` 允许枚举 SHALL 为 `not_required / required / insufficient`（**保留现状**，不改名为 `none / lookup / required`）。
   - `run_mode` 允许枚举 SHALL 为 `fast_chat / memory_candidate / knowledge_grounded / high_risk`（**保留现状**，不改名为 `auto / pilot / manual`）。
   - `needs_review / consolidation_needed` SHALL 为 JSON `bool`。
3. **新增 autonomy_mode 自治控制位**（与 `run_mode` 正交）：`autonomy_mode` 允许枚举 SHALL 为 `auto / assisted / blocked`：
   - `auto`：本轮 Reply Agent 全权决策，review 仅打分不强制改写；
   - `assisted`：本轮需 Review Agent 显式 approve 或触发 revision，是大部分高风险路径的默认；
   - `blocked`：本轮被安全门 / 预算 / 字典违规等 Rust 校验拦截，不发送、不 revise。
4. **统一 knowledgeNeed 不再前后两套**：CI SHALL 加一条 lint，禁止本期新增代码与 prompt 出现 `not_required / required / insufficient` 之外的 `knowledge_need` 取值（如 `none / lookup` 等被明确禁用），同时禁止旧别名 `knowledgeRequired: bool` 在新增 schema 中出现。
5. **必填校验**：IF Reply Agent 输出后任一必填字段按"未填"判定（key 缺失、为 JSON `null`、字符串字段空或仅空白），或字符串字段取值不在 R3.2 / R3.3 允许枚举内，或 bool 字段不是 JSON `bool`，THEN 系统 SHALL 把 `review.approved` 置为 `false`，并按违规字段在 `review.risks` 中追加：未填字段追加 `missing_required_field:<fieldName>`，枚举非法字段追加 `invalid_enum_value:<fieldName>:<value>`，类型非法字段追加 `invalid_type:<fieldName>`；每个违规字段最多一条。
6. THE `src/agent/guards.rs::normalize_decision_runtime` SHALL 删除"对空字段填默认值"的逻辑，仅保留"白名单枚举校验 + planner 同步"语义；任何当前依赖 `normalize_decision_runtime` 给字段赋默认值的调用方 SHALL 改为依赖 R3.5 的 review 失败路径。
7. **预算超额二态**：IF `decision.needs_review == true AND current_run_budget().is_exceeded() == true`，THEN `local_decision_review` SHALL 返回 `approved == false`，把 `risks` 设为恰好 `["budget_exceeded_no_review"]`，并把 `decision.autonomy_mode` 强制改为 `"blocked"`。
8. **低风险快速通道**：IF `current_run_budget().is_exceeded() == true AND decision.needs_review == false`，THEN `local_decision_review` SHALL 返回 `approved == true`，在 `risks` 追加 `"local_review_low_risk_only"`，且 `decision.autonomy_mode` 保持原值（不强制 blocked）。
9. WHEN R3.5 触发 `approved == false`，THE `gateway_status` SHALL 等于 `"blocked_by_required_field"`、`decision.should_reply` SHALL 强制 false、`decision.autonomy_mode` SHALL 强制 `"blocked"`、`agent_events` SHALL 写一条 `kind="autonomy_field_violation"` 事件。
10. WHEN R3.7 触发 `approved == false`，THE `gateway_status` SHALL 等于 `"blocked_by_budget"`、`decision.should_reply` SHALL 强制 false、`agent_events` SHALL 写一条 `kind="budget_exceeded_no_review"` 事件。
11. THE 单元测试 SHALL 覆盖：(a) 7 个必填字段任一未填触发 `missing_required_field`、(b) `risk_level="weird"` 触发 `invalid_enum_value`、(c) `knowledge_need="none"`（被禁用别名）触发 `invalid_enum_value`、(d) `autonomy_mode="manual"`（旧字面量）触发 `invalid_enum_value`、(e) needs_review=true + 预算超额走 blocked 不发送、(f) needs_review=false + 预算超额仍能发送但带 `local_review_low_risk_only`。

### Requirement 4: Knowledge Agent 工具化（MCP tool-calling 协议 + 多轮 Reply Agent 派发）

**User Story:** 作为 Reply Agent 的设计者，我希望 Reply Agent 能像人一样"先翻目录、再搜关键词、最后打开切片"地按需检索知识；当前代码不是天然 tool-calling runtime（是普通 LLM JSON 决策流），所以本期 SHALL 显式定义"工具调用 JSON 协议 + 多轮 Reply Agent 派发循环 + 失败降级 + 预算硬上限 + 知识结果如何回注 prompt"全部细节，否则就会变成"文档上有工具，实际还是一次性拼 prompt"。

#### Acceptance Criteria

1. **Tool Call JSON 协议**：THE Reply Agent 输出 schema SHALL 在现有 `AgentDecision` 字段之外新增 `toolCalls: Array<{ tool: String, arguments: Object }>`（默认空数组）字段，**且** 与 R1.10 的 `decision_phase` 字段联动：
   - `tool` 取值 SHALL 严格为 `"knowledge.list_catalog" / "knowledge.search" / "knowledge.open_slice"` 三选一；
   - `arguments` SHALL 是匹配该 tool 入参 schema 的 JSON 对象（详见 R4.4 / R4.5 / R4.6）；
   - **(a) tool-loop 中间轮**：当 Reply Agent 在某轮想要"先调用工具再决定 reply_text"时，SHALL 输出 `decision_phase = "tool_calling"` + 非空 `toolCalls` + `decision.reply_text == ""` + `decision.should_reply == false`；此时 R1.3-R1.6 的自治协议必填校验 SHALL **不触发**（仅校验 toolCalls schema、tool count cap、tool 名 / arguments 格式合法性）；
   - **(b) 最终轮**：当 Reply Agent 在某轮想要"完结决策"时，SHALL 输出 `decision_phase = "final"` + 空 `toolCalls`，`decision.reply_text` 与 `should_reply` 按正常语义填写；此时 R1.3-R1.6 / R3 / R5 全部 review 校验 SHALL 完整执行（详见 R1.10）；
   - IF Reply Agent 在 `decision_phase == "tool_calling"` 时仍然填写非空 `reply_text` 或 `should_reply == true`，THEN 系统 SHALL 在 `risks` 追加 `"tool_calling_phase_with_reply_text"`，丢弃该轮的 reply 字段，仅保留 toolCalls 进入下一轮；
   - IF Reply Agent 在 `decision_phase == "final"` 时仍然输出非空 `toolCalls`（即想"既调工具又给最终回复"），THEN 系统 SHALL 把 `toolCalls` 截断为空数组（**不**继续工具循环）、在 `risks` 追加 `"final_phase_extra_tool_calls_dropped"`、按 final 轮完整 review 校验当前 decision。
2. **多轮 Reply Agent 派发循环**：THE 系统 SHALL 实现一个新的内部循环 `reply_with_tools_loop`，伪代码语义为：
   ```
   loop_count = 0
   while loop_count < MAX_TOOL_LOOPS (default 3):
       decision = call Reply Agent with current prompt + accumulated tool results
       if decision.toolCalls is empty: break  // Agent 自己说完成
       for each call in decision.toolCalls (max R4.7 cap):
           execute call with timeout 5s, append result to accumulated tool results
       loop_count += 1
   if loop_count == MAX_TOOL_LOOPS and decision.toolCalls non-empty:
       force decision.toolCalls = []
       append risks "tool_loop_exhausted"
   ```
   `MAX_TOOL_LOOPS` SHALL 由 `runtime_parameters.knowledgeMaxToolLoops` 覆盖（默认 3，范围 [1, 5]）。
3. **预算硬上限（计入 RunBudget）**：每次 tool call SHALL 计入 `current_run_budget()`：
   - 计 1 次 LLM 调用相当于 0 次 tool call（独立计数）；
   - tool call 计数器存于新增字段 `RunBudget.tool_calls_used`，上限由 `runtime_parameters.knowledgeMaxToolCalls` 覆盖（默认 6，范围 [1, 16]）；
   - tool call 返回的 token 数（snippet / 全文长度）SHALL 累加到 `RunBudget.tokens_used`；
   - IF `tool_calls_used >= knowledgeMaxToolCalls OR tokens_used >= run_token_budget`，THEN 任何后续 tool call SHALL 立即返回错误 `{ "error": "budget_exceeded" }`，不实际执行；
   - IF Reply Agent 第 N 轮还想调用工具但预算已耗尽，THEN 系统 SHALL 强制结束循环并把 `risks` 追加 `tool_budget_exhausted`。
4. **`knowledge.list_catalog`**（输入 / 输出 / 行为）：
   - 输入 schema：`{ kind?: "documents" | "items" | "chunks", limit?: i32 (1..=200, default 50) }`；输入非法 SHALL 返回 `{ "error": "invalid_input", "detail": "..." }`。
   - 输出 schema：`{ items: Array<{ id, title, category, integrity_status, updated_at }>, truncated: bool }`；本工具 SHALL NOT 返回正文。
   - 单 run 内同 `kind` 调用次数 SHALL ≤ 2；超出 SHALL 返回 `{ "error": "tool_call_repeated" }`。
5. **`knowledge.search`**（输入 / 输出 / 行为）：
   - 输入 schema：`{ query: String (1..=200 chars), top_k?: i32 (1..=32) }`；空 query / 超长 query SHALL 返回 `{ "error": "invalid_query" }`。
   - 输出 schema：`{ hits: Array<{ chunk_id, score, snippet (≤ 200 chars), integrity_status }>, query, hit_count }`；
   - 默认 `top_k = runtime_parameters.knowledgeSearchTopK` (default 8)；
   - **integrity_status 过滤**：IF `chunk.integrity_status != "verified"`，THEN snippet SHALL 被空字符串占位且 `hits[i].redacted = true`，其余字段照常返回；客户调用方据此知道"有这条切片但不能直接引用"。
6. **`knowledge.open_slice`**（输入 / 输出 / 行为）：
   - 输入 schema：`{ chunk_ids: Array<String> (1..=K) }`；K 由 `runtime_parameters.knowledgeOpenSliceMaxK` 覆盖（默认 4，范围 [1, 16]）；超 K SHALL 返回 `{ "error": "over_limit" }`；
   - 输出 schema：`{ slices: Array<{ chunk_id, body, integrity_status, source, updated_at }> }`；
   - 未知 / 不可访问的 `chunk_id` SHALL 返回 `{ "error": "unknown_chunk_id", "missing": [...] }`，部分命中也走 error 路径不返回部分结果（避免 Agent 误以为没未命中的）；
   - **integrity_status 强约束**：IF `chunk.integrity_status != "verified"`，THEN `body` SHALL 被替换为 `"<redacted_unverified_chunk>"` 占位，但 `integrity_status` 字段保留原值供 Agent 判定。
7. **每轮 toolCalls 数量上限**：THE 单条 `decision.toolCalls` 数组长度 SHALL ≤ 4；超出 SHALL 截断到前 4 条并在 `risks` 追加 `"tool_calls_per_turn_truncated"`。
8. **失败降级**：
   - 单个 tool call 抛错（参数非法 / 预算超额 / 超时 5s）SHALL 把错误以 `{ "error": "...", "detail": "..." }` 返回给 Reply Agent，**不**直接中止整个 run；
   - WHEN 单 run 内累计 ≥ 3 次 tool call 错误，THE 系统 SHALL 强制结束工具循环（force `toolCalls=[]`）并把 `risks` 追加 `"tool_call_failure_streak"`；
   - WHEN 工具循环超时（loop 总耗时 > 30s），THE 系统 SHALL 强制结束循环，把 `gateway_status` 设为 `"tool_loop_timeout"` 并 fail-closed（不发送）。
9. **知识结果如何进入最终回复**：THE 系统 SHALL 把每轮 tool call 的结果以系统消息形式注入下一轮 Reply Agent prompt，注入格式 SHALL 为：
   ```
   [system tool result]
   tool: knowledge.search
   arguments: {...}
   result: {...}
   ```
   累计注入长度 SHALL ≤ 8000 chars（utf-8）；超过 SHALL 按"丢弃最早"策略截断并在 `risks` 追加 `"tool_result_context_truncated"`。
10. **toolTrace 落库**：每次 tool call SHALL 以 `{ tool, arguments, result_summary, hit_count?, error?, latency_ms, started_at }` 形式 append 到 `decision.knowledgeRoute.toolTrace`；本字段在 `agent_run_logs` 中完整保留，单 run 上限 32 条；超过 SHALL 截断并写 `risks "tool_trace_overflow"`。
11. **声明而未使用的检测**：WHEN Reply Agent 在最终轮（toolCalls 为空那一轮）的 `knowledgeNeedReason != "" AND knowledgeNeedReason != "unchanged"` 但 `decision.knowledgeRoute.toolTrace` 中没有任何成功的 search/open_slice 调用，THE 系统 SHALL 把 `review.risks` 追加 `"knowledge_need_declared_but_not_consulted"`；该 risk 不单独 block，但会让 review 综合分下降。
12. **fast lane 回退（带 sunset）**：WHEN `runtime_parameters.knowledgeRoutingMode == "classic_router"`，THE 系统 SHALL 走原 `run_knowledge_router` 单次拼 prompt 路径，不进入 R4.2 多轮循环；该开关默认 `auto_tool_loop`，并在 R11 中规定 sunset 时间。
13. **单元测试 + PBT** SHALL 覆盖：
   - (a) Reply Agent 在 `auto_tool_loop` 下能依次调用 list_catalog → search → open_slice 完成一轮决策；
   - (b) 工具调用次数 + 总 token 计入 `current_run_budget()`，预算耗尽时第 N+1 次 tool call 返回 `budget_exceeded` 错误；
   - (c) `MAX_TOOL_LOOPS` 上限触发后 `tool_loop_exhausted` 风险写入；
   - (d) 单 tool call 超时 5s 后只该 call 失败、循环继续；
   - (e) `classic_router` 模式下行为与现状完全一致（回归保护）；
   - (f) PBT：随机生成 toolCalls 数组（含非法 tool 名、超长 query、超 K open_slice 等）→ 每条都被映射到正确的 `error` 字符串，**没有任何输入能让循环死锁或绕过预算**。

### Requirement 5: Verified Knowledge 强约束（产品声明类回复，对齐现有 integrity_status）

**User Story:** 作为运营负责人，我希望 Agent 一旦被识别为"在做产品能力声明（价格 / 案例 / 转化承诺 / 交付能力）"就必须命中已通过 `integrity_status == "verified"` 校验的知识切片，否则就被强制拦截，这样我能信任系统转交给客户的话；同时本期 SHALL 直接复用现有 `OperationKnowledgeChunk.integrity_status` 字段，不引入额外 `verified: bool` 派生字段，避免双重 source-of-truth。

#### Acceptance Criteria

1. **复用现有字段**：THE 系统 SHALL 以 `OperationKnowledgeChunk.integrity_status == "verified"` 作为"verified knowledge"的唯一判定条件；SHALL NOT 引入新字段 `verified: bool`、`is_verified` 或类似派生字段。如果代码层为了可读性需要派生 helper，SHALL 命名为 `pub fn is_verified(chunk: &OperationKnowledgeChunk) -> bool` 并仅在内部使用，**API 与 prompt 中暴露的字段名 SHALL 沿用 `integrity_status`**。
2. WHEN Review Agent 的 `claim_analysis.requiresProductKnowledge == true`，THE 系统 SHALL 计算 `verified_chunks` 集合，定义为 `decision.used_knowledge_ids ∩ { chunk.id | chunk.integrity_status == "verified" }`。
3. **claim_analysis 缺失/损坏时的 fail-closed 路径**：IF Review Agent 输出未携带 `claim_analysis` 或 `claim_analysis.requiresProductKnowledge` 字段缺失 / 非 bool（视为损坏），THEN 系统 SHALL **不再保守默认 false**，而是按以下规则推断"是否产品声明"：
   - **3.a 强制视为产品声明**（fail-closed）：IF 满足以下任一条件 — `decision.knowledge_need ∈ {"required", "insufficient"}`、或 `decision.used_knowledge_ids` 非空、或 `decision.reply_text` 经 `enforce_string_fact_risk_guard` 命中产品 / 价格 / 案例 / 承诺类标记词（即触发了 R12 中 string_fact_risk_guard 的非空 hits） — THEN 系统 SHALL 视为 `requiresProductKnowledge = true` 进入 R5.4 强约束；
   - **3.b 综合判断**（非 fail-closed）：IF 上述 3.a 条件**全部不命中**（即明显是不涉及产品的闲聊 / 关系运营），THEN 系统 SHALL 视为 `requiresProductKnowledge = false`，但 SHALL 把 `risks` 追加 `"claim_analysis_malformed"` 让 review 综合判断；
   - 无论 3.a / 3.b 哪条命中，SHALL 同时把 `risks` 追加 `"claim_analysis_malformed"` 留痕；
   - WHEN 3.a 触发并最终走到 R5.4 block，THE `gateway_status` SHALL 等于 `"blocked_by_safety_guard"`（不是 `blocked_unverified_product_claim`，因为根本原因是 review 输出损坏，而非 verified knowledge 真的缺）、`finalReviewStatus="blocked_by_safety_guard"`、`agent_events kind="claim_analysis_malformed_fail_closed"`，detail 含 `triggered_by ∈ {"knowledge_need", "used_knowledge_ids", "string_marker_hit"}`。
4. IF `claim_analysis.requiresProductKnowledge == true AND verified_chunks.is_empty()`，THEN 系统 SHALL 强制 `review.scores.fact_risk >= 6`、强制 `review.approved = false`、并把 `review.risks` 追加 `"product_claim_without_verified_knowledge"`。
5. **执行顺序**：THE R5.4 强制 SHALL 发生在 `enforce_decision_guards` 之后、`local_decision_review` 之外的最终汇总阶段；SHALL NOT 被 `local_decision_review` 的 `approved=true` 路径绕过；当二者结论冲突（local 说 approved=true，R5.4 说 false），SHALL 以 R5.4 为准。
6. WHEN R5.4 触发，THE 系统 SHALL 把 `decision.should_reply` 强制改为 `false`、`decision.autonomy_mode` 强制改为 `"blocked"`、`gateway_status` 设为 `"blocked_unverified_product_claim"`、写 `agent_events kind="product_claim_blocked"`，事件详情含 `claim_excerpt (≤ 120 chars) / requested_knowledge_ids / verified_hits=0`。
7. **safe_claims 反向门**：WHEN `decision.safe_claims_used` 非空，THE 系统 SHALL 对每个 `safe_claim` 检查 ∃ verified_chunk in verified_chunks where `safe_claim ∈ chunk.safe_claims`；IF 任一 safe_claim 不被任何 verified_chunk 支撑，THEN `review.risks` 追加 `"safe_claim_not_verified:<claim>"`（最多 5 条，超出聚合为 `"safe_claim_not_verified:and_more:<n>"`），但 SHALL NOT 单独 block（由综合分判定）。
8. THE 单元测试 SHALL 覆盖：(a) `requiresProductKnowledge=true && verified_chunks=[]` 触发 block + autonomy_mode=blocked + `gateway_status=blocked_unverified_product_claim`，(b) 同样命中但有 1 条 `integrity_status="verified"` chunk 时不 block，(c) `claim_analysis` 完全缺失 + `knowledge_need="not_required"` + `used_knowledge_ids=[]` + 无 marker 命中 → 走 R5.3.b（综合判断）+ risk 标记，(d) `claim_analysis` 完全缺失 + `knowledge_need="required"` → 走 R5.3.a（fail-closed）+ `gateway_status="blocked_by_safety_guard"`，(e) `claim_analysis` 完全缺失 + reply_text 含价格金额 marker → 走 R5.3.a fail-closed，(f) `safe_claims_used` 无 verified 命中时只追加 risks 不 block，(g) 一条 chunk 有 `integrity_status="needs_review"` 不算 verified（不影响判定）。

### Requirement 6: 长期记忆 schema 升级（MemoryFact 强类型，整层替换 Document 而非局部转换）

**User Story:** 作为人审与下游分析师，我希望 `memoryCard` 整层从 `Document` 升级为强类型 struct，每条事实带"证据来源 / 置信度 / 重要度 / 是否会过期"等元数据，且 schema 在编译期受 Rust 类型系统保护，避免运行时再出现"某 helper 转了，某 helper 没转"的不一致；这次 SHALL 是物理替换，不是局部 helper 转换。

#### Acceptance Criteria

1. **MemoryFact 强类型**（含稳定 ID 与来源链）：THE 系统 SHALL 引入 `MemoryFact` 强类型：
   - `id: String`：稳定唯一 ID，**字符串形式的 UUIDv4**；初次创建（consolidator 输出新 fact 或 R6.4 迁移升级）时由 Rust 生成；后续合并 / 改写 / 标 deprecated SHALL 沿用同一 `id`，便于跨 run 跟踪；
   - `text: String (1..=500 chars)`；
   - `evidence: Option<String> (≤ 1000 chars)`；
   - `confidence: i32 (0..=10)`；
   - `importance: i32 (0..=10)`；
   - `mayExpire: bool`；
   - `deprecatedAt: Option<DateTime>`；
   - `deprecationReason: Option<String> (≤ 200 chars)`；
   - `sourceMessageIds: Vec<ObjectId>`：本 fact 的事实依据来自哪些 `conversation_messages._id`（最多 5 条；超出按时间倒序保留最新 5）；
   - `sourceRunId: Option<String>`：首次写入本 fact 的 `agent_run_logs.run_id`，用于 R0 trace 关联；
   - `createdAt: DateTime`：本 fact 首次创建时间；
   - `updatedAt: DateTime`：本 fact 最近一次内容 / 元数据被改写的时间。
2. **整层强类型化（不只是局部 helper）**：THE `OperatingMemory.memory_card` 字段 SHALL 整层从 `Document` 替换为新 struct `MemoryCardTyped`，至少包含 `coreFacts: Vec<MemoryFact> (cap=6)`、`recentFacts: Vec<MemoryFact> (cap=10)`、`deprecatedFacts: Vec<MemoryFact> (cap=20)`、`coreProfile: CoreProfileTyped`、`relationshipState: RelationshipStateTyped`、`extra: Document`（用于承接未识别字段，避免破坏老数据）；
   - `compact_memory_card_with_previous` / `consolidate_contact_memory` / `default_memory_card` / `memory_card_from_contact` 等所有现有 helper SHALL 改用 `MemoryCardTyped` 作为入参与返回值；
   - 涉及 Rust → MongoDB 的写入 SHALL 用一次性 `serde_bson::to_document(&MemoryCardTyped)` 序列化，不再保留两套并行表示；
   - **R7 冲突 / 废弃 / 合并的关键键 SHALL 优先用 `MemoryFact.id`，不依赖 `text` 精确匹配**（详见 R7.2）。
3. **反序列化兼容**（一次性 + sunset）：THE 反序列化 SHALL 通过 `#[serde(untagged)] enum MemoryFactRepr { Plain(String), Structured(MemoryFact) }` 兼容老 `Vec<String>`：`Plain(text)` SHALL 等价于 `MemoryFact { id: <new UUIDv4>, text, evidence: None, confidence: 7, importance: 5, mayExpire: false, deprecatedAt: None, deprecationReason: None, sourceMessageIds: vec![], sourceRunId: None, createdAt: now, updatedAt: now }`（注意 id 是 fresh UUID，避免老数据没有 id 后续合并失真）；该 enum SHALL 在 R6.4 迁移完成且 7 天内全量数据已升级后被物理移除（详见 R11）。
4. **数据迁移**：THE `db::migrations` SHALL 新增一条幂等迁移 `2026_05_005_memory_facts_to_structured`，扫描所有 `operating_memories.memory_card.coreFacts / recentFacts` 中字符串元素并升级为 R6.3 默认值的结构化条目（每条赋予新 UUIDv4 + `createdAt = now`）；迁移后老数据 100% 走 R6.2 强类型路径；同一 fact 第二次执行迁移 SHALL 跳过（用 `id` 字段是否存在 + 类型判断幂等）。
5. **Memory Consolidator prompt 强约束**：THE Memory Consolidator prompt SHALL 要求每条新生成的 fact 强制包含 `evidence`（≥ 10 个 Unicode 字符 + 至少 6 个汉字 / 引用对话片段或来源说明）与 `confidence / importance` 整数（0..=10）；输出不合规 SHALL 在落库前由 Rust 校验后填默认值，并在 `agent_run_logs.memoryConsolidatorWarnings` 追加 `"missing_evidence:<text>" / "invalid_confidence:<text>:<value>"` 等具体警告。
6. **前端展示**：THE 前端 MemoryCardSummary SHALL 显示每条 fact 的 `text`、`evidence`（折叠展示）、`confidence` 与 `importance`（小标签 0-10）、`mayExpire`（角标）、`deprecatedAt`（若非空显示删除线 + 时间 + `deprecationReason` tooltip）。
7. **写入路径自动转换**：IF 调用方仍传入 `Vec<String>`（例如老 simulation 种子），THEN 写入路径 SHALL 自动转为 `MemoryFact` 默认结构（按 R6.3）并保留 200 OK 响应；同时在响应 body 中追加 `"warning": "memory_facts_auto_upgraded"` 提示前端尽快迁移；该自动转换路径 SHALL 在 R11 sunset 时间点被移除，届时传入 `Vec<String>` 直接返回 400。
8. **PBT 测试**：THE PBT 测试 SHALL 扩展现有 `memory_card_invariants.rs`：(a) `Plain` 与 `Structured` 序列化往返保不变量；(b) 旧 `Vec<String>` 输入下 cap=6/10 与"未 discarded 必保留"性质仍成立；(c) 整层 `MemoryCardTyped` round-trip Mongo 不丢字段（含 `extra`）。

### Requirement 7: Memory Agent 冲突处理（deprecatedFacts / conflicts 落库）

**User Story:** 作为运营负责人，我希望 Memory Agent 在整理记忆时主动判断"哪条旧事实已经过期"并把过期原因记下来，这样系统能在多轮对话中越用越准而不是越用越脏。

#### Acceptance Criteria

1. **Memory Consolidator prompt 输出**：THE prompt SHALL 输出 `deprecatedFacts: Vec<{ id: String /* 引用上一版 MemoryFact.id */, reason: String (1..=200 chars), supersededBy: Option<String> /* 新 fact 的 id */, deprecatedAt: String /* RFC3339 */ }>` 与 `conflicts: Vec<{ a_id: String, b_id: String, resolution: String (1..=300), winner: "a"|"b"|"none" }>` 两个新字段。**关键**：所有引用 SHALL 用 `MemoryFact.id`，**不**用 `text` 精确匹配，避免改写后失真。
2. **按 id 标 deprecated**：WHEN Memory Consolidator 输出 `deprecatedFacts: [{ id: X, reason: Y, deprecatedAt: T, supersededBy?: Z }]`，THE 系统 SHALL：
   - 在上一版 `coreFacts / recentFacts` 中按 `id == X` 查找命中条目；命中 SHALL 把它从 `coreFacts / recentFacts` 移除（如果在）；
   - 把命中条目的副本（保留原 `id / text / evidence / confidence / importance / sourceMessageIds / sourceRunId / createdAt`）+ 新填充的 `deprecatedAt = T / deprecationReason = Some(Y) / updatedAt = now` 追加到 `deprecatedFacts` 列表；
   - IF `id == X` 在新版 `coreFacts / recentFacts / deprecatedFacts` 都查不到，THEN 系统 SHALL 在 `agent_run_logs.memoryConsolidatorWarnings` 追加 `"deprecated_fact_id_not_found:<id>"`，**不**写入 `deprecatedFacts`（避免幻觉）；
   - IF `supersededBy = Some(Z)` 但 `Z` 在新版 `coreFacts / recentFacts` 都查不到，THEN 系统 SHALL 在 warnings 追加 `"superseded_by_id_not_found:<deprecated_id>:<superseded_id>"`，但 deprecated 写入仍执行。
3. **时间戳解析**：IF prompt 输出的 `deprecatedAt` 不是合法 RFC3339，THEN 系统 SHALL 回退为当前 `now`，且在 `agent_run_logs.memoryConsolidatorWarnings` 追加 `"invalid_deprecated_at:<id>:<raw>"`。
4. THE `memoryCard.deprecatedFacts` SHALL 至多保留最近 N=20 条；超出 SHALL 按 `deprecatedAt` 升序丢弃最旧的（同 `deprecatedAt` 时按 `id` 字典序作 tiebreak）。
5. **冲突事件**：WHEN 冲突解决发生（`conflicts[].winner != "none"`），THE 系统 SHALL 在 `agent_events` 写一条 `kind="memory_conflict_resolved"` 事件，detail 含 `a_id / b_id / winner / resolution / a_text / b_text`（text 仅作为 audit 辅助显示，不参与匹配）；多条 conflicts 各写一条事件。
6. **Reply Agent context 注入**：THE Reply Agent prompt SHALL 在每轮 context pack 中显式包含最近 K=5 条 `deprecatedFacts`（仅 `id + text + deprecationReason + deprecatedAt`，不含 evidence 全文，按 `deprecatedAt` 降序），便于 Reply Agent 在 selfCritique 中引用"为什么不再使用这条事实（id=X）"。
7. **协议违规检测**：IF 同一 `id` 同时出现在新版 `coreFacts / recentFacts` 与 `deprecatedFacts`，THEN 系统 SHALL 把它视为协议违规、保留在 `deprecatedFacts` 中（不在 active 集合中），并在 `agent_run_logs.memoryConsolidatorWarnings` 记录 `"fact_simultaneously_active_and_deprecated:<id>"`。
8. THE 单元测试 SHALL 覆盖：(a) consolidator 输出 `deprecatedFacts: [{ id: X, reason: Y, deprecatedAt: T }]` 时新 memoryCard 的 `deprecatedFacts` 包含 `id == X && deprecationReason == Some(Y) && deprecatedAt == T`、原 `text / sourceMessageIds / sourceRunId` 保留；(b) `id` 找不到走 R7.2 warning fallback、不写入 deprecatedFacts；(c) 同 id 同时 active+deprecated 走 R7.7 fallback；(d) `deprecatedFacts` 超 cap 20 按 R7.4 排序丢弃；(e) 非法 RFC3339 deprecatedAt 走 R7.3 fallback；(f) 改写场景 — 新 fact 的 `text` 与上一版 X 不同但 `id` 相同时 SHALL 视为改写（直接覆盖原条目，不进 deprecatedFacts）。

### Requirement 8: 双层标签 — 严格字典 + Agent 自由信号 + 候选审

**User Story:** 作为多 domain 运营管理者，我希望聚合用的 `customer_stage / intent_level / objection_type` 走严格字典约束 Agent 必须选已知值；同时不限制 Agent 对真实用户的"自由理解"，让 Agent 自由生成的标签先进入候选池由后台审核，再决定是否升级为字典条目。这样既能聚合统计又不僵化。

#### Acceptance Criteria

1. **system_taxonomies 严格字典**（聚合用）：THE 系统 SHALL 引入新 collection `system_taxonomies`，仅承载**严格字典**类目 `kind ∈ {"customer_stage", "intent_level", "objection_type"}`（**不含 free-form `tags`**）；文档 schema：`{ scope: "global" | account_id, kind: <如上>, value: { id: String, displayName: String, description: String, aliases: Vec<String>, status: "active" | "deprecated" }, updated_at: DateTime }`，在 `(scope, kind, value.id)` 上建唯一索引。
2. **agent_generated_signals 自由层**：THE `AgentDecision` SHALL 新增字段 `agentGeneratedSignals: Vec<AgentSignal>`，其中 `AgentSignal { kind: String (≤ 40 chars), value: String (1..=80 chars), evidence: Option<String> (≤ 500 chars), confidence: i32 (0..=10) }`；本字段 SHALL NOT 受字典约束，Agent 可自由生成；本字段值 SHALL NOT 进入聚合统计（只在审计 / 后续 Agent 自我引用 / `taxonomy_candidate` 候选审中使用）。
3. **taxonomy_candidate 候选审**：THE 系统 SHALL 新增 collection `taxonomy_candidates`：当 Agent 输出 `customer_stage / intent_level / objection_type` 不在对应字典中（含 alias 命中失败）时，SHALL 把该值以 `{ scope, kind, raw_value, evidence, confidence, first_seen_at, occurrences, status: "pending" | "approved" | "rejected" }` upsert 进 candidates 集合（同值递增 `occurrences`）；候选 SHALL 通过新接口 `POST /api/admin/taxonomy-candidates/:id/{approve|reject}` 由后台审核；approve 时 SHALL 自动并入对应字典。
4. **不阻塞运行（核心约束）**：WHEN Agent 输出值不在字典中，THE 系统 SHALL 把 `review.risks` 追加 `"taxonomy_candidate:<kind>:<value>"`（注意：是 `taxonomy_candidate` 不是 `taxonomy_unknown_value`）、把候选写入 `taxonomy_candidates`，但 SHALL NOT 单独 block；review 综合分仍可让它通过；这是与原硬阻塞设计的**关键差别**。
5. **状态机仍走严格校验**：THE `decision.operation_state` SHALL 仍然必须在 `default_user_operation_state_machine().states[*].key` 集合内；非法 SHALL 走 R3.5 必填校验路径（`invalid_enum_value:operation_state:<value>`），与 taxonomies 路径互斥。
6. **deprecated 字典值**：WHEN Agent 输出值命中字典里 `status == "deprecated"` 的条目，THE 系统 SHALL 把 `review.risks` 追加 `"taxonomy_deprecated_value:<kind>:<value>"`，但 SHALL NOT 单独 block。
7. **后台 API**：THE 系统 SHALL 提供：
   - `GET / POST / PATCH / DELETE /api/admin/taxonomies?kind=...`（管理字典本身，软删 = 把 `status` 改为 `deprecated`，不物理删）；
   - `GET /api/admin/taxonomy-candidates?status=pending`（看候选）；
   - `POST /api/admin/taxonomy-candidates/:id/approve`（接受 → 自动写入对应 `system_taxonomies`）；
   - `POST /api/admin/taxonomy-candidates/:id/reject`（拒绝 → 候选状态改 rejected，下次 Agent 输出同值时不再频繁累计 occurrences）。
8. **数据迁移 seed**：THE `db::migrations` SHALL 在新增一条幂等迁移 seed `kind ∈ {customer_stage, intent_level, objection_type}` 的 global 默认字典，初值与现有 prompt 中硬编码的运营术语对齐（保证升级后老 contact 仍能通过校验）。
9. THE 单元测试 SHALL 覆盖：(a) `customer_stage="不在字典里"` → 不 block，但 `taxonomy_candidate` 写入 + `review.risks` 含 `taxonomy_candidate:customer_stage:<value>`；(b) 字典 alias 命中视为合法；(c) `operation_state` 不在状态机走 R3.5 路径而非 R8 路径；(d) deprecated 值只警告不 block；(e) approve 候选后自动写入字典且下次同值不再写候选；(f) `agentGeneratedSignals` 任意值都被接受、不写候选、不影响聚合。

### Requirement 9: 自治回路审计字段（agent_run_logs 升级）

**User Story:** 作为产品负责人，我希望每次 run 的"是否触发 revision / hold / selfCritique 命中 / 字典 candidate / autonomy_mode"都有结构化字段记录，这样我能用一句聚合查询看到某天自治回路的整体表现。

#### Acceptance Criteria

1. THE `AgentRunLog` SHALL 新增字段：`revisionApplied: bool`、`revisionReason: String (≤ 1024)`、`preRevisionSummary: Option<String> (≤ 2048)`、`postRevisionSummary: Option<String> (≤ 2048)`、`selfCritique: Option<String> (≤ 2048)`、`autonomyMode: String`、`finalReviewStatus: String`。
2. **finalReviewStatus 枚举（不含 human）**：THE `finalReviewStatus` 取值 SHALL 严格为以下集合之一：`approved` / `revision_applied_approved` / `revision_failed` / `held_by_ai_policy` / `blocked_by_safety_guard` / `ai_waiting_for_more_context` / `blocked_by_required_field` / `blocked_by_budget` / `blocked_unverified_product_claim` / `legacy_mode_unchecked`；**严禁**取值 `held_for_human / human_required` 等暗示人工接管。
   - `legacy_mode_unchecked` 仅在 `runtime_parameters.autonomyProtocolEnabled == false`（灰度回退）且原本应判 `blocked_by_required_field` 的场景下落入，详见 R11.4；该状态 SHALL 在 R10.2 自治指标聚合中**独立计入分母但不计入任何分子比率**（按 R11.2"未升级"语义处理）。
   - 任何在本枚举之外的取值 SHALL 视为脏数据，写库时由 R9.10.e 的拒收测试阻断。
3. **autonomyMode 落库**：THE `agent_run_logs.autonomyMode` SHALL 等于 `decision.autonomy_mode` 的最终值（详见 R3.3）；缺失或非法 SHALL 写 `"blocked"` 并加 risk `"autonomy_mode_invalid"`。
4. **一次性写入语义**：THE 系统 SHALL 在 `write_agent_run_log` 同一 DB 操作中写入 R9.1 全部字段；缺失语义的场景（如未触发 revision）SHALL 写 `revisionApplied=false / revisionReason="" / preRevisionSummary=None / postRevisionSummary=None`。
5. THE `agent_run_logs` SHALL 在 `(account_id, finalReviewStatus, started_at)` 上建复合索引；同时在 `(account_id, autonomyMode, started_at)` 建复合索引便于 R10 聚合。
6. **事件对齐**：THE 系统 SHALL 在 `agent_events` 中为每个非 `approved` 与非 `revision_applied_approved` 的 `finalReviewStatus` 写**恰好一条** kind 对齐的事件，并通过 `run_id` 关联（详见 R3.9 / R3.10 / R5.6 / R8.4）。
7. WHEN `revisionApplied == true`，THE `preRevisionSummary` SHALL 包含原 `decision.reply_text` 与原 `review.scores` 的 JSON 字符串；THE `postRevisionSummary` SHALL 包含修订后的 `decision.reply_text` 与第二轮 `review.scores`。
8. THE `agent_decision_reviews` SHALL 在原结构基础上新增 `revision_applied: bool` 与 `final_review_status: String`，与 `agent_run_logs` 同 run_id 时取值 SHALL 完全一致。
9. **前端筛选**：THE 前端列表 SHALL 支持按 `final_review_status` 多选过滤（0 选 = 不筛选；全选 = 不筛选；其他 = AND-IN 子集）。
10. THE 单元测试 SHALL 覆盖：(a) 一次正常通过 run 写入 `revisionApplied=false / finalReviewStatus="approved" / autonomyMode 由 Agent 输出落库`；(b) revision 触发 + 二审通过写 `finalReviewStatus="revision_applied_approved"` 且 `pre/postRevisionSummary` 都非空；(c) revision 触发 + 二审失败写 `finalReviewStatus="revision_failed"`；(d) shouldHold + holdCategory 走对应 `finalReviewStatus`；(e) `finalReviewStatus="held_for_human"` 被严格拒收（写库时 SHALL 视为 bug 并 panic 或 error）。

### Requirement 10: 前端自治回路监控 Tab

**User Story:** 作为运营负责人，我希望在"运营成效中心"里有一个"自治回路监控"Tab，能看到 revision 触发率、hold 比例分类（仅 AI 内部状态分类、不暗示人工）、selfCritique 命中率、taxonomy candidate 比例，这样我能判断"自治升级到底有没有让系统更好"。

#### Acceptance Criteria

1. THE 前端 SHALL 在 `运营成效中心` 顶部 Tabs 中新增"自治回路监控"项，路由 `/outcome/autonomy`。
2. THE 该 Tab SHALL 至少展示以下 7 个指标卡（基于 horizon 内 `agent_run_logs` 聚合，仅统计 R11 升级后的 run，老 run 不计分子分母）：
   (a) `revision_trigger_rate = revisionApplied=true / total_runs`
   (b) `revision_pass_rate = (revisionApplied=true && finalReviewStatus="revision_applied_approved") / revisionApplied=true`
   (c) **`ai_hold_breakdown`**（替代旧 `should_hold_rate`）：分别按 `held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context` 三个分类显示比例，**不**显示"等待人工"
   (d) `taxonomy_candidate_rate = `（risks 含 `taxonomy_candidate:*`）`/ total_runs`
   (e) `unverified_claim_block_rate = finalReviewStatus="blocked_unverified_product_claim" / total_runs`
   (f) `self_critique_addressed_rate`（review.selfCritiqueAddressed=true 占 revisionApplied=true 的比例）
   (g) `autonomy_mode_distribution`（auto / assisted / blocked 三类占比）
3. THE 该 Tab SHALL 提供 horizon 选择器（24h / 7d / 30d，默认 24h），所有指标按该 horizon 重新计算。
4. THE 后端 SHALL 提供新接口 `GET /api/outcomes/autonomy?horizon=...&account_id=...` 返回上述指标的数值与分子分母原始计数（便于前端展示 tooltip）；接口响应时间 SHALL ≤ 2 秒（在 100k runs 规模下）。
5. WHEN horizon 内 `total_runs == 0`，THE 接口 SHALL 返回所有比率为 `null`（不为 `0/0` 也不为 `0`），前端 SHALL 显示"暂无数据"。
6. THE 该 Tab SHALL 提供"近 50 条 revision 记录"列表，每条展示 `contact_name / pre_reply_excerpt(≤50 chars) / post_reply_excerpt(≤50 chars) / revisionDirection(≤80 chars) / finalReviewStatus / holdCategory(可选)`，点击展开看完整 `pre/postRevisionSummary` 与 `selfCritique`。
7. **AI 暂缓分类视图**：THE 该 Tab SHALL 在 `ai_hold_breakdown` 下方提供"AI 暂缓原因分布"小图（饼图或条形图），分类标签 SHALL **严格使用** AI 内部状态名（"AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文"），SHALL NOT 出现"人工接管 / 转人工 / 等待运营"等词汇。
8. THE 该 Tab 的 UI SHALL 复用现有 `tokens.css` 的设计语言（按钮、卡片、配色）。
9. THE 端到端测试 SHALL 覆盖：(a) 接口在 `total_runs=0` 时返回 null 比率；(b) 构造 5 条 run（其中 2 条触发 revision）后 `revision_trigger_rate == 0.4`；(c) 构造 3 条 hold（每个 holdCategory 各 1 条）后 `ai_hold_breakdown` 三类各 1/total_runs；(d) `held_for_human` 历史值不被统计在任何分类内（被视为脏数据）。

### Requirement 11: 一次性迁移兼容 + 灰度开关 sunset（不长期维护双轨）

**User Story:** 作为生产环境运维者，我希望本次升级**只**保留一次性迁移兼容与短期灰度开关，新数据全部走新结构、灰度开关有明确移除时间点；不为了旧结构长期污染主逻辑。

#### Acceptance Criteria

1. **一次性反序列化兼容**：THE 系统 SHALL 通过 R6.3 的 `MemoryFactRepr` enum 与 R8.8 的迁移 seed 兼容老 `Vec<String>` 与缺字典数据，**且 SHALL 在 R6.4 / R8.8 迁移完成后立即验证**：通过启动期统计本 workspace 下还有多少 `coreFacts / recentFacts` 含字符串元素 + 还有多少 contact 的 `customer_stage` 不在字典中；当两个数都为 0 持续 7 天，运维 SHALL 启动 sunset 流程（详见 R11.5）。
2. **agent_run_logs 老字段缺失**：THE 系统 SHALL 兼容历史 `agent_run_logs / agent_decision_reviews` 缺失 R9.1 中新增字段：缺失字段反序列化 SHALL 取默认值（`bool=false / String="" / Option=None`）；统计时 SHALL 把"缺 finalReviewStatus 字段或字段为空字符串"的历史记录归为"未升级"，独立计数显示，不计入新指标分子分母。
3. **knowledgeRoutingMode = classic_router 灰度开关**：THE `runtime_parameters.knowledgeRoutingMode` SHALL 接受 `auto_tool_loop / classic_router` 两值（默认 `auto_tool_loop`）；切到 `classic_router` 时 R4 多轮工具循环 SHALL 完全不生效，回退到现 `run_knowledge_router` 单次拼 prompt 路径；该开关 SHALL 在 R11.5 sunset 时点物理移除。
4. **autonomyProtocolEnabled 灰度开关**：THE `runtime_parameters.autonomyProtocolEnabled: bool = true` 开关：当为 `false` 时，R1.3 / R1.4 / R3.5 的"必填校验"SHALL 不触发 review 失败（仍记录 risks 但 `approved` 不被强制 false），且 `agent_run_logs.finalReviewStatus` SHALL 不写 `blocked_by_required_field`，改写 `legacy_mode_unchecked`；该开关 SHALL 在 R11.5 sunset 时点物理移除。
5. **明确 sunset 计划**：THE 项目 SHALL 在 `docs/sunset-plan.md` 文档中明确以下移除时间点（基于代码合并日 D 起算）：
   - **D + 7 天**：跑一次"升级度量"（剩余非结构化 facts 数 / 字典外 stage 数 / `legacy_mode_unchecked` run 占比 / `classic_router` run 占比）；任一 > 0.1% 则推迟 sunset；
   - **D + 14 天**：物理移除 `MemoryFactRepr::Plain` 反序列化兼容 + 物理移除 `autonomyProtocolEnabled` 与 `knowledgeRoutingMode` 字段（默认行为变成永久新行为）；
   - **D + 21 天**：物理移除 R6.4 / R8.8 数据迁移脚本（同一 migration_id 重启不再扫描）。
6. **基线核验（合并门）**：THE 升级合并前 CI SHALL 跑 `cargo test --lib` 与 `cargo test --test state_transition_pbt --test memory_card_invariants --test string_fact_risk_guard --test llm_retry_jitter`，并断言：
   - `cargo test --lib` 总通过数 ≥ Introduction 中记录的"升级前基线"（实测 78），且 0 失败；
   - 4 个 PBT 文件累计通过数 ≥ 升级前基线（实测 33 = 6+9+6+12），且 0 失败；
   - 实际 PBT 总数 SHALL 在合并 PR 描述中以"实测：lib=N1, pbt=N2"形式记录，且 N1 / N2 不得低于 Introduction 中的基线数；
   - 升级新增的测试可以增加分子，但 SHALL NOT 替换或删除老测试以"伪造增长"。
7. THE 数据迁移（R6.4、R8.8、R13）SHALL 幂等且带版本号；同一版本第二次启动 SHALL 跳过执行而不报错。
8. **API 兼容**：THE 后端 API SHALL 不删除现有 `customer_stage / intent_level / tags` 任一字段；前端老调用方在 R11.5 sunset 之前 SHALL 继续可用而无需立即升级。
9. **关键约束**：本期升级 SHALL NOT 引入 R11.5 之外的额外"双轨长期维护"代码（即每个新增的兼容路径都必须有 sunset 计划、相关测试、和"sunset 触发器"测量）；任何 PR 引入新的"灰度开关"但没写进 R11.5 SHALL 视为协议违规并阻断合并。

### Requirement 12: PBT 与回归测试（覆盖 P1-P7）

**User Story:** 作为质量负责人，我希望本次升级以"性质（property）+ 回归测试"双轨保护，所有自治协议核心规则都有 100 次随机驱动的反例搜索，而不是只靠几条人工写的 example。

#### Acceptance Criteria

1. THE 测试套件 SHALL 新增一个 PBT 文件 `tests/autonomy_protocol_pbt.rs`，覆盖 P1-P7 共 7 条性质：

   - **P1 自治字段必填**：FOR ALL 随机生成的 `AgentDecision` (其中至少一个必填字段被设空，含 R3 中 7 个必填字段)，运行 `enforce_decision_guards + review` 后 `review.approved == false` 且 `review.risks` 含 `missing_required_field:*` / `invalid_enum_value:*` / `invalid_type:*` 之一；同时 `decision.autonomy_mode == "blocked"`。
   - **P2 Single-Shot Revision 上限**：FOR ALL 随机生成的 (Reply 输出, Review 输出 with `needsRevision=true && revisionDirection 非空`)，运行完整 run pipeline 后 Reply Agent 调用次数 ≤ 2（首次 + 至多 1 次 revision）；当第二轮 review 仍 fail 时，`final gateway_status == "revision_failed" && decision.should_reply == false`。
   - **P3 预算超额不发送**：FOR ALL 随机生成的 `RunBudget`（含 `is_exceeded() == true` 状态），当 `decision.needs_review == true` 时，`local_decision_review` 返回 `approved == false`、`gateway_status == "blocked_by_budget"`、`autonomy_mode == "blocked"`、mock 中 `send_called == 0`。
   - **P4 产品声明强约束**：FOR ALL 随机生成的 `(claim_analysis.requiresProductKnowledge=true, used_knowledge_ids, integrity_status_set)` 三元组（按 `integrity_status == "verified"` 判定），当 `used_knowledge_ids ∩ verified_chunk_set == ∅` 时，`review.scores.fact_risk >= 6 && review.approved == false && decision.autonomy_mode == "blocked"`。
   - **P5 记忆冲突可追溯**：FOR ALL 随机生成的 (previous coreFacts, consolidator 输出 with `deprecatedFacts: [{ text: X, reason: Y }]`)，新版 `MemoryCardTyped.deprecatedFacts` 列表 SHALL 含 `text == X && deprecationReason == Some(Y) && deprecatedAt != None`。
   - **P6 字典 candidate 不阻塞**：FOR ALL 随机生成的 `decision.customer_stage`（一半在字典 / 一半不在），不在字典且非 alias 命中时，`review.risks` 含 `taxonomy_candidate:customer_stage:<value>` 且 `taxonomy_candidates` collection 写入；**关键**：`review.approved` 不被该字段强制 false（与 R8.4 一致）。
   - **P7 工具循环不死锁 + 预算不被绕过**：FOR ALL 随机生成的 `Vec<ToolCall>`（含非法工具名 / 超长 query / 超 K open_slice / 模拟超时），`reply_with_tools_loop` SHALL 在 ≤ `MAX_TOOL_LOOPS` 轮内终止；总 tool call 次数 SHALL ≤ `knowledgeMaxToolCalls`；当 `RunBudget.is_exceeded()` 后任何后续 tool call 立刻返回 `budget_exceeded` 错误而不实际执行。

2. THE 每条性质 SHALL 跑 ≥ 64 次随机用例（proptest 默认 256 也可），失败时 SHALL 给出最小化反例；单条性质执行时间 SHALL ≤ 60 秒（防死循环）。

3. THE 现有 `tests/state_transition_pbt.rs / memory_card_invariants.rs / string_fact_risk_guard.rs / llm_retry_jitter.rs / last_inbound_split.rs / migrations_idempotency.rs / reaction_claim_lock.rs / dry_run_isolation.rs / common_smoke.rs / worker_reclaim.rs / happy_path_run.rs` 等老测试 SHALL 100% 保持通过；本次升级若导致老 PBT 反例出现，SHALL 视为升级缺陷而不是测试失误，必须修代码。

4. THE 新增 lib unit tests SHALL 至少覆盖：

   - R0.10 Run Envelope 生命周期 + insert/update 边界 + panic / 超时落库（≥ 6 例）
   - R1.3 / R1.4 / R1.5 / R1.10 必填校验 + 条件长度 + decision_phase 门控（≥ 6 例）
   - R2.3 / R2.4 / R2.5 / R2.6 / R2.11 revision 控制流（≥ 5 例）
   - R3.7 / R3.8 / R3.11 local_decision_review 二态 + 枚举严格（≥ 3 例）
   - R4.1 / R4.2 / R4.3 / R4.5 / R4.6 / R4.8 工具调用循环 + decision_phase + 失败降级 + 预算计入（≥ 6 例）
   - R5.3 / R5.4 / R5.7 verified knowledge block + claim_analysis fail-closed + safe_claim risks（≥ 4 例）
   - R6.1 / R6.2 / R6.3 / R6.4 反序列化、整层 typed、迁移、stable id（≥ 4 例）
   - R7.2 / R7.5 / R7.7 / R7.8 deprecatedFacts 落库 + conflict 事件 + id-not-found warning（≥ 4 例）
   - R8.3 / R8.4 / R8.5 / R8.7 字典 candidate 不阻塞 + 状态机分流（≥ 4 例）
   - R9.2 / R9.7 / R9.10 审计字段写入 + 严禁 held_for_human（≥ 3 例）
   - R13 发送闭环 outbox + 强幂等 key + 取消 + locked_until（≥ 5 例，详见 R13.10）

5. THE 升级合并前 SHALL 跑一次完整 `cargo test --lib` 与 `cargo test --tests`，全部通过；升级后新增的 PBT 失败 SHALL 阻断合并（CI exit code 0 才视为通过）。

6. THE 端到端冒烟（happy_path_run）SHALL 至少新增两个 case：
   - `autonomy_full_loop_with_revision`：模拟"Reply Agent 输出引发 needsRevision → revision 后通过 → 写出 `agent_run_logs.revisionApplied=true && finalReviewStatus="revision_applied_approved"`"全链路；Reply Agent 调用次数恰好 2。
   - `autonomy_tool_loop_happy_path`：模拟"Reply Agent 调 list_catalog → search → open_slice 后给出回复"，断言 `decision.knowledgeRoute.toolTrace.len() == 3 && tool_calls_used == 3 && finalReviewStatus="approved"`。

7. THE 性质测试 SHALL NOT 依赖真实 MongoDB / 真实 LLM / 真实 MCP / 真实网络；所有外部依赖 SHALL 用 mock 或纯函数版本（参考现有 `compact_memory_card_with_previous` 风格）。

### Requirement 13: 可靠发送闭环（outbox + 幂等 + 二次安全门 + 重试 + 取消）

**User Story:** 作为生产环境运维者，我希望"决策通过 review"和"实际把消息发出去"是两个解耦的可靠链路：决策落地一次性写持久化 outbox，发送 worker 拉取后做最后一次安全门 + MCP 调用 + 失败重试 + 完整 trace；用户拒绝 / cooldown 后旧 outbox entry 必须被取消，不能继续发送。这样系统能在崩溃 / 网络抖动 / 用户态变化下都不丢消息也不发错消息。

#### Acceptance Criteria

1. **outbox collection**：THE 系统 SHALL 新增 collection `agent_send_outbox`，schema：
   ```
   {
     _id: ObjectId,
     account_id, contact_wxid, run_id, decision_id,
     source_event_id: String,         // 触发 run 的入站消息 / 跟进任务 ID（必填，与 R0.1 对齐）
     source_kind: String,             // "inbound_message" | "follow_up_task" | "manual_send"
     content: String,                 // 最终发送文本（不可变）
     content_hash: String,            // SHA-256 of content（用于幂等 key 计算与防篡改校验）
     idempotency_key: String,         // SHA-256 of (source_event_id + ":" + contact_wxid + ":" + content_hash) — **不**包含 run_id
     attempt: i32 (default 0),
     max_attempts: i32 (default 3),
     status: "pending" | "in_flight" | "sent" | "failed_terminal" | "canceled",
     cancel_reason: Option<String>,
     last_error: Option<String>,
     next_retry_at: Option<DateTime>,
     // R13 新增：worker 抢占崩溃恢复
     worker_id: Option<String>,        // 当前抢占该条 entry 的 worker 标识（hostname:pid:uuid）
     locked_until: Option<DateTime>,  // 抢占租约过期时间，超过则可被其它 worker 重新抢占
     created_at, updated_at, sent_at: Option<DateTime>
   }
   ```
   并在以下位置建立索引：(a) `(account_id, status, next_retry_at)` 复合索引（worker 扫描用）；(b) `(idempotency_key)` **唯一**索引（强幂等保证）；(c) `(status, locked_until)` 复合索引（崩溃恢复用）；(d) `(source_event_id, contact_wxid)` 索引（按入站消息追溯发送链路）。
2. **决策落地 = outbox 写入**：WHEN 一次 run 的 `finalReviewStatus ∈ {"approved", "revision_applied_approved"} AND decision.should_reply == true`，THE 系统 SHALL 在写 `agent_run_logs` 最终字段（详见 R0.2 的 `update_one` 阶段）成功之后、紧接着的同一 await 序列内写一条 outbox `status="pending"`；
   - **顺序约束**：`agent_run_logs` 的 update_one 必须先于 `outbox.insert_one` 执行（保证有 envelope 才有 outbox），但二者**不要求**强一致事务（MongoDB 标准部署不开启事务）；用 `run_id` 关联保证 trace 可追溯；
   - **关于 gateway_status 与 finalReviewStatus 的关系**（详见 Introduction 状态映射表）：触发条件用 `finalReviewStatus`（归档语义）而**不**用 `gateway_status`（过程语义），因为 revision 通过后 `gateway_status="approved"` 但 `finalReviewStatus="revision_applied_approved"` —— 二者都允许写 outbox，统一通过 finalReviewStatus 判定避免漏写；
   - **强幂等 key 公式**：`idempotency_key = SHA256(source_event_id + ":" + contact_wxid + ":" + content_hash)`，**不含** `run_id`，确保同一条入站消息被多次重试 run 时（每次 run_id 不同）也只发送一次；
   - IF 同 `idempotency_key` 已存在（任意 status，包括 `sent / canceled / failed_terminal`），THEN 跳过写入但记录 `agent_events kind="outbox_idempotent_skip"` + detail 含原 outbox `status / sent_at` 让排障可见；
   - IF `source_event_id` 为空字符串（理论不应发生），THEN SHALL 退回为 `idempotency_key = SHA256("synthetic:" + run_id + ":" + contact_wxid + ":" + content_hash)` 并写 `agent_events kind="outbox_synthetic_idempotency_key"` 警告 — 此分支只为兜底，正常路径不应触发。
3. **发送 worker（含崩溃恢复）**：THE 系统 SHALL 新增 worker `outbox_dispatcher`，每 N 秒（默认 5s，由 `runtime_parameters.outboxPollIntervalSeconds` 覆盖）扫描可处理 entries，每个 worker 实例 SHALL 启动时生成自己的 `worker_id = "{hostname}:{pid}:{uuid}"`：
   - **正常抢占**：扫 `status="pending" AND (next_retry_at IS NULL OR next_retry_at <= now)`；用 atomic `findOneAndUpdate({status: "pending", _id: ?}, {$set: {status: "in_flight", worker_id: <self>, locked_until: now + lease_seconds, updated_at: now}})` 抢占；`lease_seconds` 默认 60s（由 `runtime_parameters.outboxLeaseSeconds` 覆盖）；
   - **崩溃恢复**：每个 worker tick **同时**扫 `status="in_flight" AND locked_until < now` 的 entries，用 atomic `findOneAndUpdate` 抢回为 `status="pending" AND worker_id=NULL AND locked_until=NULL`，并写 `agent_events kind="outbox_lease_expired"`，detail 含 `previous_worker_id / locked_until / reclaimed_by`；
   - 抢占成功 SHALL 进入 R13.4 二次安全门；抢占失败 SHALL 跳过该条；
   - 同一 entry 一次 worker tick 内 SHALL 不被两个 worker 同时处理（atomic claim + locked_until 双保险）。
4. **发送前二次安全门**：抢占后、调 MCP 之前，THE worker SHALL 执行二次安全门检查：
   - 检查 `contact.cooldown_until > now`：成立则 status → `canceled` + `cancel_reason="contact_cooldown_active"`；
   - 检查 `contact.last_inbound_at > decision.created_at` 且新入站消息 outcome 是 `user_replied_stop_requested`：成立则 status → `canceled` + `cancel_reason="user_stop_requested_after_decision"`；
   - 检查 `agent_decision_reviews.find_by_decision_id(this.decision_id).reaction_analysis.outcome` 含 `stop_requested`：成立则取消；
   - 检查 outbox entry 创建时间 > 30 分钟：成立则 status → `canceled` + `cancel_reason="outbox_stale_30min"`（避免发出"过时回复"）；
   - 任一取消 SHALL 写 `agent_events kind="outbox_canceled"`，detail 含 `cancel_reason / decision_id / run_id`；同时清 `worker_id = NULL / locked_until = NULL`。
5. **MCP 调用 + 重试（统一 status 枚举）**：二次安全门通过后，THE worker SHALL 调用 MCP 发送（沿用现有 `mcp::send_message` 或等价接口）：
   - 成功：status → `sent`、`sent_at = now`、`worker_id = NULL`、`locked_until = NULL`、写 `agent_events kind="outbox_sent"`；
   - 失败（HTTP 错误 / 超时 / MCP 返回 error）：`attempt += 1`；
     - IF `attempt < max_attempts`，THEN status → `pending` + `next_retry_at = now + (2^attempt)*5s + jitter` + `last_error = error_summary` + `worker_id = NULL` + `locked_until = NULL`，写 `agent_events kind="outbox_retry_scheduled"`；
     - IF `attempt >= max_attempts`，THEN status → `failed_terminal`（**统一枚举值** — R13.1 schema 与 R13.7 trace 与 R13.10 测试全部用 `failed_terminal`，**不**用 `failed`），写 `agent_events kind="outbox_failed_terminal"`。
6. **取消接口**：THE 系统 SHALL 提供两条取消通道：
   - 内部：当 `record_user_reaction` 检测到 stop / cooldown 时，主动把该 contact 所有 `status ∈ {pending, in_flight}` 的 outbox 改为 `canceled` + `cancel_reason="user_reaction_stop_requested"` + 清 `worker_id / locked_until`；
   - 后台：`POST /api/admin/outbox/:id/cancel` 接受手动取消（带 `cancel_reason` body 字段）；该 API 仅允许取消 `pending / in_flight` entry，对 `sent / failed_terminal / canceled` SHALL 返回 409 Conflict。
7. **trace 完整性**：每个 outbox entry 的生命周期 SHALL 至少写以下事件之一：`outbox_created / outbox_idempotent_skip / outbox_canceled / outbox_sent / outbox_failed_terminal / outbox_retry_scheduled / outbox_lease_expired / outbox_synthetic_idempotency_key`；同 `outbox_id` 的事件总数 SHALL ≤ 20（防 retry 风暴写爆）。
8. **gateway_status 对齐**：WHEN 一次 run 写了 outbox 但**没有**实际触发 MCP 发送（例如取消 / 失败），THE `agent_run_logs.gateway_status` SHALL 仍然是 `approved`（因为决策本身通过），但 `agent_run_logs.outbox_status` 字段 SHALL 反映最终结果（取值与 R13.1 status 枚举一致：`pending / in_flight / sent / failed_terminal / canceled`）；这意味着 `agent_run_logs` 与 outbox 是两条独立 trace，前端审计 SHALL 同时显示二者。
9. **前端展示**：THE 前端 `运营成效中心` SHALL 在"自治回路监控"Tab 内新增一行"发送链路状态"指标，含 `outbox_send_success_rate / outbox_canceled_rate / outbox_failed_terminal_rate / outbox_lease_expired_rate / mean_attempts_to_success`；后台 SHALL 提供 `GET /api/admin/outbox?status=...&account_id=...&horizon=...` 列表接口供运营查问题。
10. **测试矩阵**（PBT + 集成）：
    - 单元：(a) 同 `idempotency_key` 二次写入触发 idempotent skip；(b) atomic claim + locked_until 防双发；(c) 二次安全门 4 类取消各触发；(d) 重试 backoff 公式正确（attempt=1 → ~10s、attempt=2 → ~20s、attempt=3 → ~40s ± jitter）；(e) 同 `source_event_id` 但 `run_id` 不同的多次决策 SHALL 共享同一 `idempotency_key` 故只发送 1 次（强幂等核心安全性）。
    - 集成（testcontainers）：(f) 决策通过 → outbox pending → worker 抢占 → MCP mock 成功 → status=`sent`；(g) MCP mock 失败 3 次 → status=`failed_terminal`（**枚举值统一**）；(h) `record_user_reaction` 检测 stop_requested → 同 contact 所有 pending outbox 被取消；(i) 30 分钟陈旧 outbox 自动取消；(j) **崩溃恢复** — worker A 抢占后立即 kill（不更新 status），等 `lease_seconds` 过期，worker B SHALL 通过 R13.3 崩溃恢复路径重新抢占并最终 `status=sent`。
    - PBT：(k) 任意 outbox 状态序列下，唯一 `idempotency_key` 永远不会触发 ≥ 2 次 MCP 实际发送（关键安全性）；(l) 任意 worker 抢占 / 崩溃 / 续租序列下，永远不会出现 2 个 worker 同时持有同一 entry（locked_until + atomic claim 不变量）。


## 实现注记（Design / Tasks 阶段强约束）

本段不是 Acceptance Criteria，是基于当前代码现状给 design.md 与 tasks.md 的**实现序约束**：违反任一条都会导致下一波改动卡死。

### N1（对应反馈 4）：先删 Rust 业务兜底，再加新协议

`src/agent/guards.rs:554` 的 `normalize_decision_runtime` 现在还在替 Agent 补 `risk_level / knowledge_need / run_mode / needs_review / consolidation_needed`。这与 R3.6 直接冲突。Tasks 阶段 SHALL 把"删除默认业务兜底 + 改成协议校验失败"作为**第 1 个 Task**，**先于** R1 / R2 / R4 任何其它改动。否则 R3 / R12.P1 测试根本写不出来 — 现有代码会把 Agent 漏字段补成默认值，违规永远触发不了。

### N2（对应反馈 5）：先升级 AgentDecision struct，再升级 prompt

`src/agent/types.rs:35` 当前的 `AgentDecision` 没有 `autonomy_mode / decision_phase / toolCalls / 9 个自治字段 / agentGeneratedSignals`，且 `needs_review / consolidation_needed` 是 `bool` 默认值（无法区分"模型没输出"与"模型输出 false"）。Tasks 阶段 SHALL 引入两层结构：

- **`RawAgentDecision`**：`#[derive(Deserialize)]`，所有 R3.1 必填字段用 `Option<T>` 表达"模型是否输出"（`Option<bool> / Option<String>`）；本结构仅用于反序列化 Reply Agent 输出。
- **`AgentDecision`**（业务结构）：保留现有非 Option 字段，由 `RawAgentDecision::validate_and_promote(...) -> Result<AgentDecision, Vec<RiskTag>>` 转换；转换时执行 R1.3 / R3.5 全部必填 / 类型 / 枚举校验，违规聚合为 `Vec<RiskTag>` 返回供 review 注入。
- 转换函数 SHALL 在 R0.4 状态推进之前调用，确保校验失败也能落 envelope。

不能**先升级 prompt 让 Agent 输出新字段，再升级 struct** — 那会让现有 serde 反序列化丢字段（默认 default value），R3 校验测试无法落地。

### N3（对应反馈 6）：抽出"最终安全汇总层" + 改造 local_decision_review

`src/agent/review.rs:69` 的 `local_decision_review` 默认 `approved=true`，且 `src/agent/gateway.rs:517` 在预算超额或低风险时直接走它。R5 的 verified knowledge 强约束 / R3.7 的预算超额 fail-closed / R8.4 的 taxonomy candidate 标记，**全部**必须发生在 `local_decision_review` **之后**且不能被它绕过。Tasks 阶段 SHALL：

- 抽出新函数 `finalize_review_for_send(review, decision, runtime, contact, knowledge_runtime) -> DecisionReviewResult`，这是"最终安全汇总层"；
- 它在所有 review 结果（无论来自 LLM full / light review 还是 local_decision_review）之后调用一次，负责执行 R3.7 / R5.4 / R8.3 等"硬安全门"，且其修改 SHALL NOT 被任何上游 `approved=true` 路径绕过；
- gateway 的 `if budget_exceeded { local_decision_review() } else if should_run_review() { review_decision() } else { local_decision_review() }` 三分支 SHALL 全部接到 `finalize_review_for_send` 之前。

### N4（对应反馈 7）：把直接发送改造为 outbox + decoupling

`src/agent/gateway.rs:739` 的 `send_outbound_message` 当前**直接**调 MCP，且发送在 `write_agent_run_log` 之前。R13 的"决策落地 = outbox 写入"要求二者解耦。Tasks 阶段 SHALL：

- **保留** `send_outbound_message` 函数体，但仅供 outbox dispatcher worker 调用（标 `pub(crate)` + 加 `#[doc = "Only callable from outbox_dispatcher"]`）；
- gateway 主路径 SHALL 在 `agent_run_logs.update_one`（R0.2）之后调用 `outbox_dispatcher::enqueue(decision, run_id, source_event_id, content_hash)`，**不再**直接调 `send_outbound_message`；
- 老 simulation / dry-run 路径如果需要"实时不入库"发送，SHALL 走独立 `simulate_send` 函数，与 outbox 完全分开；
- 此改动会让现有 `tests/happy_path_run.rs` 等测试断言"已发送"的 case 必须等 worker tick 后才能 assert；要在测试 helper 中加 `wait_for_outbox_processed(run_id, timeout=10s)`。

### N5（对应反馈 8）：MemoryCard typed 替换是边界工程，不是局部 helper 修改

`src/models.rs:300` 的 `OperatingMemory.memory_card: Document` 与 `src/models.rs:1020` 的 `MemoryCardTyped` 当前是两套并行结构（typed 仅在 helper 内部用）。R6.2 要求**整层物理替换**：

- `OperatingMemory.memory_card` 字段类型 SHALL 从 `Document` 改为 `MemoryCardTyped`（含 `extra: Document` 兜底）；
- `agent_decision_reviews / agent_run_logs / contacts` 中所有引用 `memory_card` 的字段 SHALL 同步迁移；
- `compact_memory_card_with_previous / consolidate_contact_memory / default_memory_card / memory_card_from_contact / write_memory_to_db / read_memory_from_db` 全部签名改用 `MemoryCardTyped`；
- 旧 `Vec<String>` 反序列化通过 R6.3 的 `MemoryFactRepr::Plain` 一次性兼容；
- 这次替换 SHALL 在 tasks 中作为**独立 Task 群**（"MemoryCard 边界替换 + 迁移"），与 prompt 改动 / outbox / tool-loop 完全解耦，避免巨大的合并冲突。

### N6（对应反馈 9）：基础设施 Task 必须独立于业务 Task

`src/db/mod.rs:114` 当前没有 `agent_send_outbox / system_taxonomies / taxonomy_candidates` 三个 collection 的 accessor / 索引 / 迁移。Tasks 阶段 SHALL 把这三个 collection 的定义放在最早的"基础设施波"（与 N1 删兜底并列），先就位再做业务改动：

- **基础设施波**：新建 collection accessor + 唯一索引 + 复合索引（按 R8.1 / R13.1）+ R6.4 / R8.8 数据迁移脚本 framework；不做任何 prompt / Agent 行为变更；
- **业务波 1**：N1（删兜底）+ N2（RawAgentDecision）+ R0（envelope insert/update）；
- **业务波 2**：R3（校验）+ R5 finalize_review（N3）；
- **业务波 3**：R4 tool-loop + R8 taxonomy 接入；
- **业务波 4**：R13 outbox 接入（N4）；
- **业务波 5**：R6 / R7 MemoryCard 整层替换（N5）；
- **业务波 6**：R10 前端 + R12 PBT 收口。

不要让"prompt 改动"与"基础设施新增"放同一个 PR；这是历史教训。

### N7（实现层补充约束）：测试基线核验自动化

R11.6 要求 CI 合并前实测核验。Tasks 阶段 SHALL 增加一个 `scripts/check-baseline.ps1` 脚本（或 Makefile target），跑：

```
cargo test --lib | grep "test result: ok\." | awk '{print $4}' >= 78
cargo test --test state_transition_pbt --test memory_card_invariants --test string_fact_risk_guard --test llm_retry_jitter | awk total >= 33
```

CI workflow SHALL 在 `git push` 时自动运行；任何 PR 让任一数字下跌 SHALL 阻断合并。
