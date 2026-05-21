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
