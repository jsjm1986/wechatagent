# Data And API Design

## Current Collections

当前 MongoDB collections：

```text
wechat_accounts
contacts
conversation_messages
agent_tasks
agent_events
mcp_call_logs
```

## Core Identity

所有运营数据必须至少能定位到：

```text
workspace_id
account_id
业务对象 ID
```

私聊对象：

```text
contact_wxid
```

群聊对象未来使用：

```text
chatroom_id
```

朋友圈对象未来使用：

```text
moment_id / sns_id / task_id
```

## Contact Model Principles

联系人不是简单通讯录记录，而是运营对象。

当前关键字段：

```text
wxid
nickname
remark
alias
agent_status: normal | managed
human_profile_note
agent_profile
memory_summary
last_message_at
last_agent_run_at
```

后续用户运营扩展字段建议：

```text
tags
customer_stage
intent_level
last_commitment
follow_up_policy
source_channel
```

这些字段不应一次性全部加入。只有当 UI 和 Agent 逻辑实际使用时再添加。

## Future Collections

微信群运营：

```text
wechat_groups
group_messages
group_profiles
group_insights
```

朋友圈运营：

```text
moment_plans
moment_drafts
moment_posts
moment_interactions
```

内容资产：

```text
content_assets
content_collections
brand_voice_rules
forbidden_expressions
```

Agent 策略：

```text
agent_policies
policy_versions
automation_rules
```

## API Design Principles

API 按产品模块组织，而不是按 MCP 工具组织。

当前：

```text
/api/accounts
/api/contacts
/api/conversations
/api/events
/api/tasks
```

未来建议：

```text
/api/users
/api/groups
/api/moments
/api/content-assets
/api/agent-policies
/api/operations
```

原则：

- API 返回产品对象，不暴露 MCP 原始结构。
- MCP 错误要转换成可理解的业务错误。
- 写操作必须记录事件。
- 自动化行为必须能追踪来源。
- 列表接口必须支持分页和筛选。

## MCP Integration Principles

MCP 是能力层，不是产品边界。

调用规则：

- 所有 MCP 调用集中在 MCP client 或 service 层。
- 不在 React 前端直接调用 MCP。
- 不让 LLM 自由选择任意 MCP 工具。
- 高风险工具必须经过策略层。
- 所有 MCP 调用写入 `mcp_call_logs`。

## LLM Output Contract

Agent 输出必须是结构化 JSON。

当前私聊决策：

```json
{
  "shouldReply": true,
  "replyText": "string",
  "profileUpdate": {
    "summary": "string",
    "interests": [],
    "communicationStyle": "string",
    "operationGoal": "string"
  },
  "memoryUpdate": "string",
  "followUp": {
    "needed": false,
    "runAt": "",
    "content": ""
  }
}
```

解析失败时必须不发送消息，并记录错误事件。

