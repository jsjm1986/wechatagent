# Agent Policy

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
NextBestActionScore = RelationshipGain + UserValue + ConversionProgress + ProductFit + Timing - DisturbanceCost - PressureRisk - FactRisk
```

自动发送约束：

```text
FactRisk >= 6              禁止发送
PressureRisk >= 7          禁止发送
HumanLikeScore < 6         改写一次
EmotionalValue < 5         改写一次
ProductAccuracyScore < 7   禁止发送涉及产品承诺的内容
```

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
