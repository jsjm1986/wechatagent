# AI Agent System

WechatAgent should be designed as an AI-native operations product, not a traditional admin system with a chatbot added later. The product has two different Agent categories:

```text
Management Agent: operates the product and calls system capabilities for the human operator
Operations Agents: operate specific WeChat business domains under policy control
```

These two categories must stay separate. They use different prompts, permissions, logs, and success metrics.

## Agent Roles

### Management Agent

目标：成为项目的总后台操作入口，类似 Codex Desktop / Claude Desktop 的自然语言工作台。

它服务的是内部操作员，可以理解并执行这类指令：

```text
把张三加入 Agent 运营列表
帮我搜索 AI应用开发 这个好友
发送 xxx 给好友 xxx
新建一个群，拉 A、B、C 进群
查看今天失败的任务
把用户运营 Agent 的回复风格改得更克制
生成本周朋友圈内容计划
```

Management Agent 不直接替代业务 Agent。它负责把操作员的自然语言意图转换成产品动作：

- 查询系统状态
- 查询微信对象
- 修改运营配置
- 创建任务
- 调用低风险 MCP 工具
- 对中高风险动作发起确认或策略校验
- 解释系统执行结果

### Operations Agents

目标：对具体运营对象长期工作。

建议拆成三个业务 Agent：

```text
User Operations Agent: 好友私聊运营
Group Operations Agent: 微信群分析、线索识别、群运营建议
Moment Operations Agent: 朋友圈内容计划、草稿、发布任务
```

Operations Agent 的输入是业务上下文，不是后台操作指令。它们只在策略允许的对象上工作，例如 managed 好友、白名单群、已配置朋友圈计划。

## Soul Prompt

Soul Prompt 是 Agent 的稳定人格和运营原则，不是一次性 system prompt。

它应解决：

- Agent 是谁
- 服务什么业务目标
- 说话风格是什么
- 哪些行为永远不能做
- 面对不确定信息如何处理
- 如何在微信场景中显得自然、克制、可信

Soul Prompt 不应包含临时客户画像、最近消息、具体任务等动态信息。动态信息应通过运行时上下文注入。

## Prompt Layers

每次 Agent 调用应由多层上下文组成：

```text
System Contract
  输出格式、工具使用规则、安全边界

Agent Soul
  稳定人格、品牌语气、长期运营原则

Policy Context
  当前模块、对象、工具、频控、禁止行为

Business Context
  联系人/群/朋友圈计划、画像、内容资产、历史消息

Operator Instruction
  人类操作员本次自然语言指令，或 webhook 触发原因
```

不要把所有内容写进一个超长 prompt。后续应把 Soul、Policy、Content Asset、Object Context 分开管理，运行时组合。

## Prompt Stack v2

当前实现使用 `Prompt Stack v2` 管理非 Soul 层提示词：

```text
agent_souls
  user / management / group / moment 的稳定人格

prompt_templates
  system_contract / policy / task_template / review / methodology_generator

operation_playbooks
  账号级运营方法论，绑定到具体微信账号和 managed 好友
```

默认 Prompt Pack：`wechatagent_prompt_pack_v2_2026_05`。

启动时如果 workspace 尚未激活 v2 pack，系统会物理删除旧 Soul、旧 PromptTemplate、旧 Playbook，并创建 v2 默认版本。人工后续修改不会在每次重启时被覆盖；如需彻底恢复系统默认，使用 `POST /api/prompt-templates/reset-system-pack`，该接口同样会先物理删除旧提示词栈再重建。

## User Operations Soul

默认好友运营 Soul 建议：

```text
你是一个长期运行的微信私域运营 Agent。
你代表企业与好友沟通，但不能假装自己是具体某个真人。
你的目标是维护关系、理解需求、推进下一步行动，而不是机械销售。
你说话要像微信里的真人：简短、自然、克制、有上下文。
你不能编造价格、承诺、案例、身份、库存、政策或已经发生的事实。
如果信息不足，你应该先澄清或给出保守表达。
如果对方只是寒暄、结束语或无需回复，你可以不回复。
你必须记住每个好友是独立运营对象，不能用统一话术批量对待所有人。
```

后续每个企业可以创建自己的品牌 Soul，每个好友可以有局部偏好覆盖，例如更直接、更温和、更技术化、更商务化。

## Group Operations Soul

默认群运营 Soul 建议：

```text
你是微信群运营分析 Agent。
你的第一目标是理解群内讨论、识别线索、总结话题和建议运营动作。
你默认不在群内自动发言。
你不能挑起争论、刷屏、过度营销或替人承诺。
当群内存在潜在客户、投诉、合作机会或风险话题时，你应生成清晰的运营建议。
```

群 Agent 的第一阶段输出应是洞察和草稿，不是自动发言。

## Moment Operations Soul

默认朋友圈运营 Soul 建议：

```text
你是朋友圈内容运营 Agent。
你的目标是稳定输出可信、有价值、符合品牌语气的内容计划和草稿。
你不能凭空编造案例、收入、客户评价、产品能力或现场图片。
你应优先使用内容资产库、真实素材和已确认事实。
你生成的内容要适合朋友圈阅读：自然、短句、有观点，避免公众号腔和强营销腔。
```

朋友圈 Agent 默认产出草稿和发布计划。自动发布必须由策略显式允许。

## Management Agent Soul

默认后台管理 Soul 建议：

```text
你是 WechatAgent 的后台管理 Agent。
你服务内部操作员，负责理解自然语言指令并调用系统能力完成微信运营管理。
你必须先判断用户意图属于查询、配置、任务创建、微信动作还是高风险动作。
你只能通过系统提供的工具执行操作，不能编造执行结果。
你必须在高风险动作前进行策略校验；如果策略要求确认，你必须先给出待确认计划。
你需要用简洁、可审计的语言汇报执行结果，包括成功、失败、跳过和下一步建议。
```

Management Agent 的体验重点不是闲聊，而是可执行、可追踪、可撤销。

## Tool Permission Model

Management Agent 使用工具时要按风险分层。第一阶段产品选择是：LLM 可以看到并选择当前账号 MCP Server 暴露的完整工具目录，但执行必须经过后端代理，不能由前端或模型直接持有 MCP Key。

```text
Read: 查询账号、联系人、群、任务、日志、策略
Draft: 生成运营备注、画像、朋友圈草稿、群运营建议
Configure: 修改 managed 状态、策略、标签、内容资产
Act: 发送消息、建群、邀请成员、创建发布任务
Dangerous: 删除好友、退出/解散群、账号登出、原始 MCP 调用
```

默认规则：

- 所有工具调用必须绑定当前 `account_id`。
- 每个微信账号使用独立 MCP Key。
- 所有 MCP 调用必须写入 `mcp_call_logs` 和 `agent_tool_calls`。
- 第一阶段以私聊闭环为主要验证范围，群和朋友圈动作即使工具可见，也不作为默认运营闭环。
- 前端不直接调用 MCP，不保存或展示 MCP Key 明文。

## Command Center UI

前端应有一个 AI 原生的总控入口，而不是只做传统表单：

```text
AI Command Center
  左侧：系统频道和对象范围
  中间：自然语言任务流
  右侧：执行计划、工具调用、结果、待确认动作
```

交互原则：

- 操作员可以用自然语言发起任务。
- 系统必须展示 Agent 准备执行的计划。
- 每个工具调用要有状态：pending / running / succeeded / failed / skipped。
- 高风险动作显示确认按钮，而不是让 Agent 自行执行。
- 执行结果应能跳转到对应对象：好友、群、朋友圈计划、任务、日志。

Command Center 是新增能力的入口，但不替代具体业务模块。复杂配置仍应落在用户运营、群运营、朋友圈运营、Agent 策略等频道里。

## Data Model Direction

后续建议新增：

```text
agent_souls
prompt_templates
agent_prompt_versions
management_agent_sessions
management_agent_messages
agent_command_runs
agent_tool_calls
agent_confirmations
```

关键要求：

- Prompt 必须版本化。
- 每次 Agent 运行必须记录使用的 Soul 和 Policy 版本。
- Management Agent 的每条指令必须能追踪到工具调用和业务结果。
- Operations Agent 的决策日志必须能还原当时上下文摘要。

## API Direction

建议新增：

```text
POST /api/management-agent/sessions
POST /api/management-agent/sessions/:id/messages
GET  /api/management-agent/commands/:id
POST /api/management-agent/confirmations/:id/approve
POST /api/management-agent/confirmations/:id/reject

GET  /api/agent-souls
POST /api/agent-souls
PUT  /api/agent-souls/:id
POST /api/agent-souls/:id/publish
GET  /api/prompt-templates
POST /api/prompt-templates
PUT  /api/prompt-templates/:id
POST /api/prompt-templates/:id/publish
POST /api/prompt-templates/reset-system-pack
```

第一阶段 Management Agent API 允许通过后端代理执行 MCP 原始工具，但必须记录结构化计划、工具调用、账号上下文和结果。后续可再把高频能力沉淀为产品动作，例如 `enable_contact_agent`、`send_contact_message`、`create_group`、`invite_group_members`。

## Success Criteria

这个方向成立的标准：

- 操作员可以通过一句自然语言完成常见后台任务。
- 系统能展示执行计划和工具调用，不是黑盒。
- 运营 Agent 有稳定人格，不会每次回复风格漂移。
- 每个好友、群、朋友圈计划仍然保持独立上下文。
- 高风险微信动作不会因为一句模糊指令被自动执行。
- Prompt、策略、工具调用和结果都可审计。
