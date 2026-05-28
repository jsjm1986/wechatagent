# Agent Policy

> **2026-05-25 重要变更**：knowledge-cleanup 已把销售域 5 闸（`fact_risk` / `pressure_risk`
> / `product_accuracy` / `human_like` / `emotional_value`）收敛到 3 闸：
> `enforce_knowledge_grounding` / `enforce_hallucination` / `enforce_run_budget`，
> 实际实现见 `src/agent/guards.rs`。本文档 §自我演化 部分仍引用旧 5 闸名（gate_key 字符串），
> 是因为 evolution layer 的 `threshold_overrides` 集合按字符串 key 工作，与运行时 guard 解耦；
> 真实在线 guard 行为以代码为准，不再扩散销售域字段。`Contact` / `OperationKnowledgeChunk`
> 的业务字段已统一下沉到 `domain_attributes: bson::Document`，由 `DomainSchema` 定义。

Agent 策略定义哪些对象可以自动化、自动化到什么程度、何时停止、如何记录。

策略分两层：

```text
Operations Agent Policy: 约束好友、群、朋友圈等运营 Agent
Management Agent Policy: 约束后台总控 Agent 能做哪些系统和微信动作
```

## Default Automation Boundary

当前默认：

```text
普通好友 normal：不自动回复
纳管好友 managed：允许私聊自动回复
微信群：暂不自动发言
朋友圈：暂不自动发布
后台管理 Agent：第一阶段允许访问完整 MCP 工具目录，但必须经过后端代理、账号凭证和审计日志
```

## User Operations Policy

managed 好友允许 Agent 执行：

- 读取运营备注
- 读取历史消息
- 读取长期记忆
- 读取运营大脑记忆
- 读取产品知识
- 生成回复
- 独立评审候选回复
- 调用 `message_send_text`
- 更新画像
- 更新记忆
- 创建跟进任务

Agent 必须遵守：

- 不编造成交、价格、承诺、身份等事实。
- 回复要像真人微信，不暴露系统或 AI。
- 如果最新消息无需回复，可以 `shouldReply=false`。
- 输出为空或 JSON 解析失败时不发送。
- 短时间内已回复时跳过，避免重复触达。
- 独立评审未通过时不发送。
- 不得使用虚假稀缺、恐惧营销、编造案例或编造产品承诺。

## Operating Brain V2

用户运营 Agent 使用转化平衡目标，不是强销售目标。系统内置以下方法论公式：

```text
Trust = Credibility + Reliability + Intimacy - SelfOrientation
ConversionReadiness = Motivation × ProductFit × Timing × Trust ÷ Friction
EmotionalValue = Empathy + Validation + Specificity + AutonomySupport - Pressure
HumanLikeScore = ContextRecall + Specificity + Naturalness + Brevity + EmotionalAttunement - TemplateRisk
NextBestActionScore = RelationshipGain + UserValue + ConversionProgress + ProductFit + Timing - DisturbanceCost - HallucinationRisk - GroundingRisk
```

自动发送约束（2026-05-25 收敛后的 3 闸 / 详见 `src/agent/guards.rs`）：

```text
HallucinationScore >= block 阈值     禁止发送 (enforce_hallucination)
KnowledgeGroundingScore < 阈值       涉及产品 / 价格 / 数据 / 政策 / 合同等关键词时禁止发送 (enforce_knowledge_grounding)
RunBudget 超限                        终止本 run，落 fallback (enforce_run_budget)
```

`HumanLikeScore` / `EmotionalValue` / `PressureRisk` 在 Phase B 补回为 review 评分阈值通道的软闸（详见 `src/agent/review.rs::route_dual_gate`）：低于 / 高于阈值时触发一次 `single-shot revision`，二次仍未通过写 `blocked_review`，不进入 `enforce_decision_guards` 三硬闸。

当前实现使用统一发送网关。任何自动发送，包括私聊自动回复和 follow-up 定时任务，都必须重新加载上下文，检查 managed、冷却期、最小间隔、每日触达上限和任务是否过期，再进入独立 Review Agent。候选回复先生成，再评审；评审未通过时改写一次；二次仍未通过则写入 `blocked_review`，不调用微信发送工具。

用户运营状态由 `user_operations` 状态机约束。Agent 每次决策必须输出 `operationState` 和 `nextBestAction`，并写入决策复盘，供后续审计和优化。

## Group Operations Policy

群运营第一阶段只允许：

- 群消息分析
- 话题总结
- 线索识别
- 回复建议

默认禁止：

- 自动群内发言
- 自动邀请成员
- 自动移除成员
- 自动修改公告
- 自动解散/退出群

未来开放自动群发言时，必须同时具备：

- 群白名单
- 触发条件
- 频控规则
- 禁止词规则
- 日志记录

## Moment Operations Policy

朋友圈第一阶段建议：

- AI 生成草稿
- AI 生成内容计划
- 选择内容资产
- 创建待发布任务

默认禁止：

- 无规则自动发布
- 无来源素材发布
- 高频连续发布

未来允许自动发布时，必须具备：

- 发布频率限制
- 内容来源记录
- 发布窗口配置
- 失败回滚/取消机制
- 发布历史审计

## Tool Risk Levels

低风险，可自动：

```text
auth_whoami
account_list
account_get_status
contacts_search
contact_get_detail
schedule_list
```

中风险，可按策略自动：

```text
message_send_text
media_get
schedule_create
schedule_cancel
```

高风险，默认不自动：

```text
moment_post_*
group_* 修改类工具
friend_delete
account_logout
personal_update_*
gewe_execute_raw
```

## Decision Logging

每次 Agent 行为都应记录：

- 输入对象
- 当前策略
- 当前运营大脑记忆摘要
- 使用的产品知识
- 是否回复
- 回复内容
- 评审评分
- 是否改写或拦截
- MCP 工具
- 成功/失败
- 失败原因
- 画像/记忆是否更新

日志是长期运营系统的安全边界，不是可选功能。

## Management Agent Policy

Management Agent 可以把操作员自然语言转换成系统动作，但必须按风险等级执行。

第一阶段默认允许自动执行：

- 查询账号、好友、群、朋友圈计划、任务、日志
- 生成运营备注、用户画像、朋友圈草稿、群运营建议
- 创建低风险内部任务
- 调用当前账号 MCP Server 暴露的完整工具目录

第一阶段已落地的产品闭环：

- 把好友加入或移出 Agent 运营
- 发送私聊消息
- 创建跟进任务

后续按策略增强：

- 修改标签、阶段、意向等级
- 修改 Agent Soul 或策略草稿
- 创建朋友圈发布任务
- 创建微信群或邀请成员

默认禁止自动执行：

- 删除好友
- 退出或解散群
- 账号登出
- 修改个人资料
- 前端直接调用 MCP 或接触 MCP Key

Management Agent 在执行前必须生成结构化计划：

```json
{
  "intent": "enable_contact_agent",
  "riskLevel": "configure",
  "target": "contact",
  "steps": [],
  "requiresConfirmation": false
}
```

如果 `requiresConfirmation=true`，必须等待人工确认后再调用工具。

## Prompt Policy

Agent prompt 必须分层管理：

```text
System Contract
Agent Soul
Policy Context
Business Context
Operator Instruction
```

规则：

- Soul Prompt 表达稳定人格和品牌语气。
- Policy Context 表达自动化边界和工具权限。
- Business Context 表达当前对象画像、历史、内容资产。
- Operator Instruction 表达本次指令或触发事件。
- Prompt 必须版本化，运行日志必须记录版本。

不要把长期人格、临时上下文、工具规则和客户画像混在一个不可维护的大 prompt 中。

当前实现要求：

- `agent_souls` 只保存稳定人格和长期原则。
- `prompt_templates` 保存 System Contract、Policy、Task Template、Review、Methodology Generator。
- `operation_playbooks` 保存账号级运营方法论。
- 用户运营决策日志记录 `promptVersions`，包含 Soul、PromptTemplate、Playbook 版本。
- 后台管理执行日志记录 `promptVersions`，并对 dangerous 或 requiresConfirmation 的计划停止自动执行。
- `reset-system-pack` 会物理删除旧系统提示词并重新生成 v2 默认包；这是显式维护动作，不应在每次启动时反复覆盖用户编辑。

## 自我演化（M4 / agent-self-evolution）

WechatAgent 自第二阶段起内置可选的"自我演化"后台 worker（`src/evolution/`），让运营 Agent 在不动业务链路的前提下持续微调 5 闸阈值与运营 prompt。本章节说明它**做什么、不做什么、如何被 admin 监督**。

### 行为概要

- 默认关停：`EVOLUTION_ENABLED=false`。运维显式打开后才会起 tick；关停后**已发布的 `threshold_overrides` / `prompt_templates` 不回退**（保持已发布的演化结果，由 admin 手工 rollback）。
- tick 周期：`EVOLUTION_TICK_SECONDS`（默认 21600，即 6 小时）。每个 tick 拉一次 cohort，run-local 预算受 `EVOLUTION_RUN_TOKEN_BUDGET` / `EVOLUTION_RUN_MAX_LLM_CALLS` 限制，溢出即终止本 tick 不影响主进程。
- 评估窗口：`EVOLUTION_EVAL_WINDOW_HOURS`（默认 72）。Cohort 抽样上限：`EVOLUTION_COHORT_PER_CONTACT_CAP`（默认 3 / contact）+ `EVOLUTION_COHORT_SAMPLE_PER_FAILURE_BUCKET`（默认 10 / 失败桶）。最少回放数：`EVOLUTION_MIN_REPLAYS`（默认 30）。

### 阈值演化（threshold）

5 闸阈值（`fact_risk_block` / `pressure_risk_block` / `human_like_score_rewrite` / `emotional_value_rewrite` / `product_accuracy_score_block`）按 `THRESHOLD_REASONABLE_BANDS` 守住合理上下限：

- 命中率 > 上限 → 候选 `proposed_value = current + step`（阈值收紧）。
- 命中率 < 下限 → 候选 `proposed_value = current - step`（阈值放松）。
- 落在 band 内 → 不产候选。
- Release 写一条 `threshold_overrides`（`rolled_back_at=null` 且 `released_at` 最新者生效）；run 入口 `resolve_thresholds` 单次读取，正在跑的 run 不受影响。
- Rollback 立即把 `threshold_overrides.rolled_back_at` 置为 now，下一次 run 入口的 `resolve_thresholds` 读回 baseline（来自 contact runtime / `AppConfig`）。

### Prompt 演化（prompt）

- 仅对 `prompt_templates` 中 `evolution_eligible=true` 的 key 启用；Critic LLM 的 prompt 自身**永远不进入** prompt evolution 循环（W2 内置黑名单）—— 这是 R10.1 的"自我引用悖论"红线。
- 候选生成：把失败 cohort（`final_review_status` ∈ block / hold 类）+ 当前模板 → Critic LLM → 产 diff_snippet。`validate_diffs` 在落库前剥离禁词（`scripts/check-no-human-takeover.{sh,ps1}` 同款规则）+ 长度上下界。
- Release 写一条 `prompt_templates`（version + 1，`current_version=true`，旧 version 被切到 `current_version=false`），`prompt_pack_version` 原子 +1 让 LRU cache 立刻失效；下次 `generate_agent_json` 自动从 Mongo 重读。
- Rollback 切回旧 version 并再 +1 pack_version；旧 content 完整恢复。

### Shadow eval + 显著性

- Shadow replay（`src/evolution/replay.rs`）只读 `agent_run_logs` 的快照，对同一 source run 在新阈值 / 新 prompt 下重判，**不**写 `agent_send_outbox`、不调 MCP、不写 `conversation_messages.outbound`。
- 显著性门槛：`EVOLUTION_MIN_SEND_SUCCESS_DELTA`（默认 0.05）+ `EVOLUTION_MIN_SELF_CRITIQUE_DELTA`（默认 0.10）+ `EVOLUTION_MAX_5GATE_HIT_INCREASE`（默认 0.10，即新版本不得让任何闸命中率上升超过 10%）。
- 三项任一不达标 → 候选直接转 `rejected_below_threshold`，不进入 `eligible_for_release`。

### Release / Rollback

- 仅 admin 可触发：前端 EvolutionCenterTab → `POST /api/evolution/proposals/:id/release` / `rollback`。Auth 复用既有 admin middleware，不引入新 token 路径。
- Release/rollback 用 Mongo session transaction（`commit_with_session` retry on `UnknownTransactionCommitResult`），保证多文档原子性。
- `evolution_threshold_release_cooldown_hours`（默认 24）：同一 gate_key 上一次 release 距今 < 24h 时下一次 release 阻止（防止抖动）。
- 全部回滚兜底：`POST /api/evolution/rollback_all`（admin 输入 `ROLLBACK_ALL` 二次确认），把所有 `threshold_overrides` 一次性 `rolled_back_at=now`、所有 `prompt_templates` 回退到 `previous_version`，并写一条强警示 `agent_events kind="evolution_rollback_all"`。

### 安全边界（红线，CI 守门）

- **隔离**（R9.4）：`src/evolution/` SHALL NOT 引用 `crate::agent::gateway` / `crate::agent::outbox` / `crate::mcp::` / `agent_send_outbox.insert` / `mcp_client.send` / `run_user_operation_gateway` 等任何主链路符号。`scripts/check-evolution-isolation.{sh,ps1}` 在 CI 阶段 grep；命中即 `exit 1`。
- **零副作用**（R6.5）：100 次 shadow replay 后 `agent_send_outbox` 与 `conversation_messages` outbound 集合 size 必须不变（`tests/evolution_isolation.rs` 守住）。
- **预算硬上限**（R3.1 / R4.1）：超 `EVOLUTION_RUN_TOKEN_BUDGET` / `EVOLUTION_RUN_MAX_LLM_CALLS` 即终止本 tick；不会因为单 tick 失败影响主进程或下一个 tick。
- **AI 自主表达**（R8.4 / R9.4）：所有新增的 `agent_events.kind` / 前端文案 / 状态名 SHALL 过 `scripts/check-no-human-takeover.{sh,ps1}` lint。演化器路径**不**引入"接管 / 人工 / takeover / hand-off"等表达——held / rolled_back / rejected 全部走 AI 内部状态名。
- **自我引用悖论**（R9.3 / R10.1）：演化器 SHALL NOT 演化自己的代码、配置、Critic prompt 与显著性算法。它只演化"被运营 Agent 用到"的阈值与 prompt；演化器自身的迭代由人写代码 + PR review 推进。
- **冷启动友好**：cohort 不足 `EVOLUTION_MIN_REPLAYS` 时本 tick 不生成候选；新 contact / 新阈值上线初期不会被误演化。
- **回滚优先**：任何指标异常 → admin 一键 rollback；rollback 后行为完全等于演化前状态（`resolve_thresholds` / `prompt_pack_version` 双路径都已集成测试覆盖）。

### Admin 视角（前端 EvolutionCenterTab）

- 只读列表：experiment 信封 / proposal（按 status 分桶）/ shadow replay 摘要 / threshold_overrides / prompt_templates 历史。
- 唯一可写动作：release / rollback / rollback_all。所有写动作在 `agent_events` 留痕，事后可审计。
- 前端不展示"接管"或"介入"等字眼；状态机以 AI 内部语言呈现（pending_eval / evaluating / eligible_for_release / rejected_below_threshold / released / rolled_back）。

### 端到端烟雾 Runbook（首次上线 / 重大变更后）

仅供运维操作。**不要**在生产高峰期执行。每步预期结果失败即停下排查，不要继续。

1. **临时启用演化器**
   - `.env` 改 `EVOLUTION_ENABLED=true` + `EVOLUTION_TICK_SECONDS=120`（2 分钟一个 tick，便于快速观察）。
   - `cargo run` 重启，或重启 systemd / docker。
   - 预期：日志看到 `evolution worker started`；不报错退出。

2. **观察 experiment 信封**
   - 等 ≤ 4 分钟（一个 tick 周期 + 处理时间）。
   - 打开前端 `EvolutionCenterTab` → Experiments 列表。
   - 预期：≥ 1 条 experiment 信封，`status=running` 或 `finished`，`cohort_summary` 显示 cohort 数量。

3. **制造 5 闸命中率偏离的真实流量**
   - 选 1 个测试 contact（managed），webhook 模拟 ≥ 30 条 inbound 触发 Agent。
   - 内容刻意触发 fact_risk（绝对承诺词 / 数字百分比 / 大额数字），使 `fact_risk_block` 命中率 > 0.15。
   - 等 1 个 tick 周期。
   - 预期：proposals 列表新增 ≥ 1 条 `proposal_kind=threshold` + `gate_key=fact_risk_block` + `proposed_value > current_value` 的候选。

4. **Release 阈值候选**
   - 候选状态切到 `eligible_for_release` 后（shadow eval + 显著性自动跑完），admin 在前端点 Release。
   - 立刻让那个测试 contact 再触发一条 inbound。
   - 预期：新 run 的 `resolveThresholds.fact_risk_block` 已读到新值（在 `agent_run_logs.runtime` 字段或前端 Run 详情可见）。

5. **Rollback**
   - admin 在前端点 Rollback 同一条 proposal。
   - 立刻再触发一条 inbound。
   - 预期：`resolveThresholds.fact_risk_block` 立刻回到 baseline 值（与第 1 步前一致）。
   - proposal 列表中该条状态切到 `rolled_back`。

6. **复原**
   - `.env` 改回 `EVOLUTION_ENABLED=false` + `EVOLUTION_TICK_SECONDS=21600`（默认）。
   - 重启服务。
   - 制造一次额外的 inbound 触发 Agent。
   - 预期：worker 不再起 tick；**已发布且未 rollback** 的任何 `threshold_overrides` / `prompt_templates` 仍然生效（验证主开关关停不回退已发布演化结果）。

如任何一步偏离预期，按 `docs/agent-policy.md` 自我演化"回滚优先"原则：admin 一键 rollback 该 proposal；必要时调 `POST /api/evolution/rollback_all`（输入 `ROLLBACK_ALL` 二次确认）一次性回滚全部并写 `agent_events kind="evolution_rollback_all"`。

## 知识库日报工作站（knowledge-digest-workstation）

WechatAgent 把"知识库 = AI agent"落到产品形态：chat 是主入口、画布是当日工作台、目录树是索引。完整 spec 见 `.kiro/specs/knowledge-digest-workstation/`。本节说明它**做什么、不做什么、如何被运营消费**。

### 行为概要

- 默认关停：`KNOWLEDGE_DIGEST_ENABLED=false`。运维显式打开后才会起 worker；关停后已生成的 `knowledge_daily_reports` 不删（保留供回看）。
- 节奏 = 节奏 1（每日一次）：每天 `KNOWLEDGE_DIGEST_RUN_HOUR`（默认 09:00，运营时区）由 `KnowledgeDigestWorker` 主动跑一次，吃 4 个数据源（`operation_knowledge_chunks` / `knowledge_usage_logs` / `agent_run_logs` / `evolution_*`），吐一份当日 `knowledge_daily_reports`。
- 单 tick 预算：`KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET`（默认 24000） / `KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS`（默认 8）；超额即终止本次 tick，已写入的 partial 报告保留并标 `status="partial"`。
- **不做**事件驱动 push（webhook 实时叫醒 chat）。所有 AI 主动消息汇集到当日日报，运营在固定时间消费。这是节奏 1 的明确选择，下一轮再考虑实时通道。

### 三栏布局（取代旧抽屉式 chat）

```
┌─ 25% 目录树 ─┬─ 45% 日报画布 ─┬─ 30% Chat（常驻）─┐
│  知识库索引   │  紧凑卡片列表    │  会话补完入口      │
│              │  ☐ severityChip │                     │
│              │  ☐ ...          │  消息流 + 输入条    │
│              │  [💬 让 AI 处理 │                     │
│              │   选中的 N 条]  │                     │
└──────────────┴────────────────┴────────────────────┘
```

- **画布形态 = 紧凑卡片列表**（节奏 1 单日 ~20 条 issue 需要批量勾选 + 横向对比；叙事周报式不能高吞吐）。
- 卡片 schema：`{kind, title, summary, targetRefs, suggestedAction, severity, metric}`；卡片 ≤ 50（超出截断 + status="partial"）；排序优先级：`severity=critical` > `kind=chunk_caused_block` > `kind=chunk_missing_field` > 其他。
- 画布**不**直接编辑 chunk，是审阅 + 派单面板；编辑入口仍走右侧 chat 或目录树展开后的 `KnowledgeChunkEditor`。

### Long-running task

- 当一次派工 ≥ 3 cards 或预估 LLM call > 6，落库为 `knowledge_chat_tasks` 由后台 `KnowledgeTaskWorker` 串行执行（同 sessionId 同时刻只 1 跑），每完成一步写 `knowledge_chat_turns{kind="task_progress"}`，全部完成写 `kind="task_summary"` 列出 needs_review chunkIds。
- **完成边界 = AI 直接落 `status="draft" + integrityStatus="needs_review"`**（与现有 chat apply 规则一致）；不要求每条 chunk 都在 chat 里点确认（节奏 1 单批 10+ 条会被 chat 流淹没）。运营在 `KnowledgeChunkEditor` 二次审核走 #329 sourceQuote → anchor 模糊匹配 gate 才能进 verified 池。
- task 失败 / 超预算走 `LlmUnavailable` 路径（与上一轮 chat 错误统一）；fail-soft，单步失败不阻塞后续 step；最后 summary 列出失败步。
- task `status="cancelled"` 时 worker 在每步开始前检查并立即停下；不强杀正在跑的步。

### Operator memory（与 chunk / contact / agent memoryCard **物理隔离**）

- 独立 collection `knowledge_operator_memory`（`{kind: "preference"|"rejection"|"context", content, lastUsedAt, ...}`）；**禁止**复用 `contacts.memory_card`（contact 维度）/ `agents.soul.memory`（agent 维度）。运营偏好不能污染对客户/agent 的记忆。
- chat 在 intent 分类前注入运营当日相关的 ≤ 5 条 operator memory。
- 写入路径：`knowledge.chat.intent` 命中 `intent=update_operator_memory` → chat handler 写一条 memory + 一条 assistant turn 显式确认（"我已记下这条偏好"）；AI **不**静默写运营记忆。

### Tool-calling 扩展

chat 内 LLM 启用 4 个新工具（基于现有 `agent::tool_loop` 注入模式，不引入新框架）：
- `audit_completeness({chunkId|packId})` — 调现有 chunk 完整度审计
- `search_chunks({query, topK})` — 走现有 BM25 + tag 检索
- `propose_repair({chunkId, hints})` — 调现有 chunk repair 路径
- `analyze_logs({contactWxid?, hours})` — 新增只读路由 `/api/operation-knowledge/logs/analyze`，反查 24h 内 block/hold runs 关联的 chunk 命中率

per-turn tool call ≤ 6（`RUN_BUDGET` 守门）；超额 fail-fast 写 `kind=tool_budget_exceeded` turn 并停下本 turn。Tool 调用结果 + latency + tokens 写入 `knowledge_chat_turns.attachments.tool_calls`，让运营在 chat 里能展开看到 AI 看到了什么。

### 安全边界（红线，CI 守门）

- **AI 永不自动 verify**：digest worker / chat / task worker 三条路径产出的 chunk 一律 `status="draft" + integrityStatus="needs_review"`；verify gate 仍由 sourceQuote → anchor 把守。
- **不动演化器**：digest 路径**只读** `evolution_*` 集合（用于卡片提示）；SHALL NOT 写 `prompt_templates` / `threshold_overrides`。这是演化器红线（R9.3 自我引用悖论）。
- **不引入"接管 / 人工"语义**：所有新增文案过 `scripts/check-no-human-takeover.{sh,ps1}` lint。统一用语「让 AI 处理选中」「AI 起草草稿」「请去 chunk 编辑器审核」「AI 没能生成今日日报」。
- **LLM 错误统一走 `AppError::LlmUnavailable`**：worker / chat / task 三层任何 LLM 失败都用上一轮已落地的错误分类与前端 `<LlmErrorBanner>`，不允许新增第二套错误样式。
- **预算硬上限**：digest worker tick / chat per-turn / task per-step 三层都被 `RunBudget` 卡死；超额即终止当前层，不会因为单层失败影响主进程或下一次 tick。

### Admin 视角

- 运营在 Knowledge 频道顶部能看到「当日日报」徽章（数字 = 未处理 cards 数）；过期日报不自动删（保留 30 天供回看）。
- 唯一可写动作：勾选派工 / 单卡派工 / 忽略 / 重算今日 / 在 chat 里自由对话；所有写动作在 `agent_events` 留痕（`kind=knowledge_digest_generated` / `knowledge_chat_task_*` / `knowledge_operator_memory_*`），事后可审计。
- 前端不展示"接管 / 介入"等字眼；状态机以 AI 内部语言呈现（pending / running / finished / failed / cancelled）。

### 端到端烟雾 Runbook（首次上线 / 重大变更后）

仅供运维操作。**不要**在生产高峰期执行。每步预期结果失败即停下排查，不要继续。

1. **临时启用 worker**
   - `.env` 改 `KNOWLEDGE_DIGEST_ENABLED=true` + `KNOWLEDGE_DIGEST_RUN_HOUR=$(当前小时+1)`。
   - `cargo run` 重启。
   - 预期：日志看到 `knowledge digest worker started`；不报错退出。

2. **观察当日合成**
   - 等 ≤ 1 小时（或临时把 RUN_HOUR 拨到当前 +5min）。
   - 浏览器进 Knowledge 频道。
   - 预期：画布显示当日日报 N 张卡片；右侧 chat 常驻；左侧目录树正常。

3. **勾选 3 张卡片派工**
   - 勾选任意 3 张 → 点「💬 让 AI 处理选中的 3 条」→ 在 chat 输入区微调话术 → 发送。
   - 预期：chat 显示 plannedSteps 弹窗 → 确认 → 弹出 taskId。

4. **观察 task 进度**
   - 等 ≤ 2 分钟（worker 30s tick）。
   - 预期：chat 实时显示 3 条 `kind=task_progress` turn + 1 条 `kind=task_summary` turn 列出 needs_review chunkIds。

5. **二次审核**
   - 点 task_summary 里的某个 chunkId → 跳转到 KnowledgeChunkEditor → 二次审核 → verify。
   - 预期：chunk 进 verified 池；下次 inbound 检索可用。

6. **operator memory 写入**
   - 在 chat 输入"以后别再起带 100% 回奶这种话术"。
   - 预期：chat 显示 assistant turn"我已记下这条偏好…"；mongosh 查 `knowledge_operator_memory` 出现新条目。

7. **故障演练**
   - 把 `OPENAI_API_KEY` 改成无效值重启。
   - 进 Knowledge 频道点「🔄 重算今日」。
   - 预期：画布显示 `<LlmErrorBanner>`（kind=http_4xx 或 connect_failed）+「AI 重试」按钮；不出 5xx 弹窗。

8. **复原**
   - `.env` 改回 `KNOWLEDGE_DIGEST_ENABLED=false` + 恢复 OPENAI_API_KEY；重启。
   - 预期：worker 不再起 tick；已生成的 `knowledge_daily_reports` / `knowledge_chat_tasks` / `knowledge_operator_memory` 仍在 Mongo（验证关停不删数据）。

如任何一步偏离预期，按"回滚优先"原则：phase 级 `git revert` + `cargo test --lib` + 浏览器手动复测；不要在生产环境直接热修。

## 运营知识库 wiki-style 方法论（knowledge-wiki）

WechatAgent 把"销售话术 RAG"升级为"运营知识 Wiki + 检索面"：知识不是查询时即时拼装，而是**写入时被增量编织进一个持久互联的 chunk 仓**，由 LLM 维护一致性、cross-reference、矛盾标注。**召回算法零改动**（catalog → list_chunks → open_slice），本节说明本轮专心做扎实的四件事：质量 / 可被检索 / 可被修改 / 可被优化。

### 1. 9 类 wiki_type（跨行业稳定）

每个 chunk 写 `wiki_type` 为下列之一：

| wiki_type | 含义 | 销售域典型 chunk | 教培域典型 chunk |
| --- | --- | --- | --- |
| `source` | 原始来源 | 销售口径 v3 PDF | 教研周会纪要 |
| `entity` | 实体 | 产品 SKU / 客户角色 | 课程包 / 师资 |
| `concept` | 概念/规则/政策 | 退款政策 | 招生话术合规清单 |
| `comparison` | 对比 | 我方 vs 竞品 | 班型 A vs B |
| `synthesis` | 综合 | 行业图谱 | 学段升学路径 |
| `methodology` | 方法/SOP | 反对意见处理框架 | 家长砍价 SOP |
| `finding` | 发现/数据点 | 转化率统计 | 续报率 |
| `query` | FAQ | "能否分期" | "孩子基础差能否上 X 班" |
| `thesis` | 带立场的判断 | "X 客户必须 face-to-face" | "Y 学段必须冲刺班" |

**业务可变字段下沉到 `domain_attributes: bson::Document`**，由 `DomainSchema` 定义；销售域配 `customer_stage / objection_type / pressure_level`，教培域配 `parent_emotion_state / age_segment / subject`。chunk 主表字段稳定，业务字段在 JSON 子文档里。

### 2. 写入路径三层保护：apply_chunk_revision

所有写入（import / patch / split / merge / archive / restore / rollback）走同一个函数 [`crate::knowledge_wiki::chunk_revisions::apply_chunk_revision`]，三层保护一律生效：

1. **锁定字段守门**：patch 试图改 `chunk_id / wiki_type / created_at / source_anchor / verified_at / verified_by / approved_at` 任意一项 → 4xx；
2. **数组字段 union**：`tags / related_chunks / sources / search_terms / applicable_scenes` 永远应用层 `existing ∪ patch`，0 风险 0 LLM 成本；
3. **70% body 长度阈值**：patch 改 `answer / explanation` 后正文短于既有 70% → 4xx，识别 LLM 截断 / 偷懒 / 误重写。

写入侧附加规则：

- **AI 写入永不自动 verify**：source=ai 强制 `status="draft" + integrity_status="needs_review"`，verify 仍走现有 `/chunks/:id/verify` + sourceQuote→anchor gate；
- **双写**：先写 `chunk_revisions`（不可变历史，sha256 before/after hash），后写 `operation_knowledge_chunks`（可变最新版）；万一 chunks 写失败 revisions 仍留下"试图但未成功"的痕迹；
- **enqueue catalog rebuild**：写完即推 `catalog_rebuild_jobs` 队列，worker 异步落库，写入路径不阻塞。

### 3. patch-only 协议

LLM 编辑 chunk 不返完整页，只返 `patch: { ...field-level diff... }` JSON。后端拿到直接调 `apply_chunk_revision`，模型不可能"顺手"改它没列在 patch 里的字段。借鉴 LLW `enrich-wikilinks.ts` 的核心洞察："让 LLM 只返替换映射而非整页"。

### 4. 反馈闭环

[`crate::knowledge_wiki::feedback_worker::feedback_worker_loop`] 每 `KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS` 秒（默认 600，0 关停）一轮：

1. 30d 滑窗 hit/blocked 回写 `usage_stats`；
2. `dynamic_confidence = clamp(integrity_score × 0.6 + hit_rate × 0.4 - stale_penalty, 0, 1)`；
3. structural lint 生成/合并 `knowledge_gap_signals`：
   - `orphan` — chunk 无入链且 30d 无命中
   - `broken_link` — `related_chunks.chunk_id` 指向不存在/已 archived 的 chunk
   - `no_outlinks` — synthesis/comparison/methodology 类 chunk 的 `related_chunks` 为空
   - `low_confidence` — `dynamic_confidence < 0.3` 但 30d hit > 0
   - `stale` — `valid_to < now`
4. **stage 1 sweep**：candidate 不再被规则生成的 pending signal → `auto_resolved`；broken_link 的 target 已恢复 / missing_chunk 标题已存在 / stale 的 valid_to 被推到未来 → `auto_resolved`。

stage 2（LLM 批裁决：contradiction / suggestion / 残留信号是否仍适用）暂留接口，本轮不进入热路径。

### 5. 不可暗示模型品牌（硬约束）

本子系统**禁止**在 prompt / schema / UI / docs / 错误信息里硬编码任何具体模型名（GPT-4 / Claude 3 / Gemini / DeepSeek-v3 / Qwen-Max ...）或品牌词（Anthropic / OpenAI / 千问 / 豆包 / 文心一言 / ChatGLM / kimi ...）做"广告提示 / 暗示用户使用什么模型"。LLM provider 由用户在 `LlmProviderConfigs` 里自填，`ChunkProvenance.llm_model_alias` 仅写 `provider_id`（如 `"default"` / `"reviewer"`）。CI 由 `scripts/check-no-model-hint.sh` 自检；每次 PR 必跑。

### 6. 编辑路由清单

| 路由 | 行为 |
| --- | --- |
| `POST /operation-knowledge/chunks/:id/patch` | 字段级 patch；最常用 |
| `POST /operation-knowledge/chunks/:id/split` | 拆分：原 chunk archive + 新建 N 个，`previous_version_id` 指原 |
| `POST /operation-knowledge/chunks/:id/merge` | 合并：原 + target 都 archive + 一个新 chunk |
| `POST /operation-knowledge/chunks/:id/archive` | 软删 + 删除级联清 dangling refs |
| `POST /operation-knowledge/chunks/:id/restore` | 取消 archive |
| `POST /operation-knowledge/chunks/:id/rollback/:revision_id` | 找 revision 的 before-state 重写为 current；写新 revision (op=rollback) |
| `GET /operation-knowledge/chunks/:id/revisions` | 分页 timeline |
| `POST /operation-knowledge/chunks/:id/relate` / `DELETE` | 维护 `related_chunks` |
| `GET /operation-knowledge/catalog/persisted` | 读 `documents.catalog_summary_persisted` 持久化快照（O(1)） |
| `GET /knowledge/gap-signals` | 列 pending / dismissed / auto_resolved 信号 |
| `POST /knowledge/gap-signals/:id/dismiss` / `apply` | 运营手动消解 |
| `POST /knowledge/gap-signals/sweep` | 手动触发一次 lint + stage 1 sweep |

## 演进路线 changelog（Phase 0 → Phase E5-T1）

本轮演进按计划 `Phase 0 → A → B → C → D → E` 分阶段交付，每阶段守住 R11.6 基线门（`cargo test --lib ≥ 78` / 4 PBT 累计 ≥ 33）+ R11 `coreFacts: Vec<String>` 兼容 + AI-autonomous 字面量禁词。

### Phase 0：紧急修复

- `gateway::write_agent_run_log_with_finalize` 改走 `update_run_envelope_terminal`，删除裸 `lifecycle: String::new()`，强制 `assert_final_review_status_valid` / `assert_gateway_status_valid`。
- `guards::check_state_transition` 改 fail-closed：`states.is_empty()` → `Some(GuardBlock::StateMachineEmpty)`；启动序列加 closed-set sanity check。
- `simulations.rs` 5 闸硬编码删除，改调 `enforce_decision_guards`，与 prod 同源；frontend `App.tsx` 5 老评分键收为 `["grounding","hallucination","runBudget"]`。

### Phase A：兑现已有能力

- `decision::decide_reply_with_promote` 装 prompt 阶段读 `decision_reviews.reaction_analysis` 近 3 轮，注入 `format_reaction_hint` 段。
- `load_operator_memory` 在 build_context 阶段调用一次（contact_id + domain_id 双键），结果作为新 prompt 段注入。
- `taxonomy::init_global_taxonomy_cache` 启动期初始化；`enforce_decision_guards` 三闸通过后追加 `check_value` 校验，`customer_stage / intent_level / objection_type` 未命中走 `upsert_candidate`，不阻塞 run（CLAUDE.md 硬规则）。
- `agent::knowledge_tools` / `agent::tool_loop` 仅作为常量与工具分发的支持模块保留（被 `chat_tool_loop` 引用：`dispatch_chat_tool_call` / `AnchorMatchFn` / `ALLOWED_CHAT_TOOL_NAMES` / `TOOL_FAILURE_STREAK_LIMIT` / `TOOL_RESULT_CONTEXT_MAX_CHARS`），user-ops 入口本身仍走 `decide_reply_with_promote → review`，不再 user-side tool-calling。

### Phase B：方法论补完（恢复 5 → 3 闸缺口）

- `guards::human_like_gate` / `pressure_risk_gate` 补回为软闸，走 review 评分阈值通道；`human_like < 阈值` 或 `pressure_risk ≥ 阈值` 触发 `single-shot revision`。阈值默认值进 `models::ThresholdDefaults`，可被 `threshold_overrides` 覆盖。
- `review::review_decision` 拼 reviewer 输入时只暴露 `user_message + draft_reply + selected_chunk_ids`，遮罩 `draft.reasoning` 防自洽幻觉。
- `OperationKnowledgeChunk::chunk_type` 升级为 4 类 enum（`product_fact / style_template / negative_example / peer_case`），R11 兼容：缺省值 `product_fact`。
- `operation_state_policies` collection：每状态挂 `allowed / forbidden / recommended_pace`，`enforce_decision_guards` 读取并拦截违反 forbidden 的 reply。
- 新增 PBT：`human_like_threshold_pbt` / `pressure_risk_threshold_pbt` / `chunk_type_routing_pbt`。

### Phase C：学习闭环

- `reaction::reviewer_misjudge_signal`：reviewer 通过但用户负反应、reviewer 拦截但用户后续正反应；`feedback_worker` 周期把信号汇总到 `reviewer_stats`。
- `negative_example chunk` 自动入 review queue（reviewer 通过但用户负反应时）：integrity_status="pending_review" 由 admin 审核入库。
- `evolution_runtime_flags`：`EVOLUTION_ENABLED` 由 env 切到 mongo flag；按 `contact_id` 哈希分桶 5% → 20% → 50% 三档；`post_release / significance / budget` 全程监控。
- `prompts::load_prompt_for_contact`：`prompt_templates` 多版本同时 active 时按 `hash(contact_id) % active_count` 选；`evolution::release_prompt` 旧版本改为 soft-retire（`current_version=false`），不再物理删历史。
- `evolution::threshold` close-loop：近 14d `hold_rate` 跌破阈值时自动写 `threshold_overrides.gate_key`，经 `post_release` 评估通过才生效；`reset-system-pack` 仍人工。

### Phase D：长程节奏

- `Contact::intent_trajectory: Vec<IntentTrajectoryEntry>`（cap 50）：`reaction` 写入时 `$push + $slice: -50`；`knowledge_router` 装 prompt 时拼接近 5 项。
- `Contact::last_outbound_style: Option<String>`：reviewer 在 approve 时回写结构化风格指纹（长度桶 / emoji / qmark / excl / tail / nl）；新 draft 与本字段比对，差异过大时触发 `single-shot revision`（≥3 / 5 axes 差异判定）。
- `cold_contact_worker`：按 `contacts.last_outbound_at` 阈值（默认 168h）挑选；钩子从 `chunk_type=peer_case` 池 deterministic hash 选；`COLD_CONTACT_WORKER_ENABLED` env 开关；每天每 contact ≤ 1 次。
- `account_scheduler`：`WechatAccount::{capacity, persona_tag, off_hours}`；webhook 入站时按 user cohort 分配（轮休策略，cross-midnight off_hours 已支持），零侵入 `webhooks.rs::resolve_account_context`。
- `lessons_learned` collection（跨用户聚合）：`feedback_worker` 14d 滑窗周期归纳 success / reviewer_misjudge_negative / blocked_by_safety_guard 三类模式；upsert 到 `lessons_learned`，admin 通过 `POST /api/admin/lessons-learned/:lesson_id/promote-to-peer-case` 一键晋升为 `chunk_type=peer_case` 候选 chunk（默认 `integrity_status=needs_review` + `status=draft`，仍走 chunk review queue 二次确认才能 verify；红线：AI 永不自动 promote，admin 手工触发）。

### Phase E：通用性抽象

**已落地：**

- **多 locale**：`Contact::locale: Option<String>` + `PromptTemplate::locale: Option<String>`（BCP-47 短形式）；`load_prompt_for_contact` 优先选同 locale 的 active 模板，未命中 fallback 到 `DEFAULT_LOCALE`（`zh-CN`），再未命中 fallback 到 `default_prompt_content`。R11 兼容：旧 contact / 旧模板缺字段反序列化为 None，由 `contact_locale_or_default` / `template_locale_or_default` 透明回退。
- **E2-T1：LlmProvider trait**：`src/llm.rs::LlmProvider`（`generate_json` / `generate_json_with_usage`）作为 LLM 客户端抽象，现有 OpenAI / DeepSeek 协议客户端 `LlmClient` 为第一实现；`generate_agent_json` 的 LRU 缓存与 `RunBudget` 计费路径不变，未来扩第二/第三 provider 不动主干。
- **E2-T2：reviewer 双模并行 + 分歧触发 single-shot revision**：`review::review_decision` 在 `REVIEWER_DUAL_ENABLED=true` 时把 reviewer 输入并行喂两路 LlmProvider（默认 primary + cross_provider），分歧（评分差 ≥ 阈值或 grounding/hallucination 决策不一致）时触发一次 revision，达成 epistemic diversity；同模无分歧时只走 primary，不增预算。
- **E5-T1：ops 三表 active_versions 灰度**：`operation_domain_configs` / `operation_state_policies` / `system_taxonomies` 三表统一加 4 字段 `version: i32 / current_version: bool / previous_version: Option<i32> / seeded_by: Option<String>`，m015 migration backfill 全量旧 row。读路径按 `current_version=true` 过滤，老库无该字段时 `$ne:false` / `$exists=false` 兜底；运行时按 `hash(contact_id) % active_count` 分桶，同 contact 同桶稳定。`admin_ops_versions` 暴露 publish / rollout / rollback 三动作，publish 写新 row 后 `update_many` 旧版本 `current_version=false`（soft demote，非物理删）。`seeded_by` 三色徽章：`legacy_migration`（m015 回填）/ `system`（默认 seed）/ `manual`（admin REST）。前端 `ActiveVersionsBar + StatePolicyAdmin + TaxonomiesAdmin` 渲染版本流水 + 回滚链 "← v{previous_version}"。

- **E1：trait OpsDomain 形式收口**：`src/agent/domain.rs::OpsDomain` 定义 domain 边界（`id` / `state_machine` / `enforce_decision_guards` / `knowledge_router`），`UserOpsDomain` 为第一实现；CLAUDE.md "Group / Moments 不要折叠到 user-ops" 红线由 trait 边界声明承载。本阶段 user-ops 仍走既有入口，`decision / gateway / knowledge_router` 不强制走 trait 分发；当 group / moments 真实落地、产生第二个 domain 调用方时，再按真实需求把分发点接到 trait 上（避免单实现期签名失真）。

**留待第二个 domain（group / moments）真实落地驱动：**

- MCP 工具动态注册：`McpClient::list_tools` + 白名单审计 + sandbox 审核位。

CLAUDE.md "Don't add features beyond what the task requires" 约束下，单 domain 期不把 `decision / gateway / knowledge_router` 强制接到 `OpsDomain` 分发；trait 仅作为边界声明 + 后续抽象的 anchor。当 group / moments 真实落地时再启动剩余 E 任务。

