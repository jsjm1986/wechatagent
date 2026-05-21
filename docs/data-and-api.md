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
operation_playbooks
operating_memories
operation_knowledge_documents
operation_knowledge_items
operation_knowledge_chunks
knowledge_usage_logs
agent_decision_reviews
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
source_channel
```

已落地的用户运营扩展字段：

```text
playbook_id
playbook_version
tags
customer_stage
intent_level
last_commitment
follow_up_policy
profile_attributes
profile_updated_at
```

标签、阶段和意向不使用固定枚举。它们由账号级 `operation_playbooks` 约束方法论，由 Agent 基于单个好友上下文自由生成和持续更新。

运营大脑 V2 新增长期认知对象：

```text
operating_memories: 每个 managed 好友一份，保存用户理解、关系状态、产品匹配和下一步行动。
operation_knowledge_documents: 运营知识文档入口。保存导入来源、AI 目录摘要、routing_map、risk_notes 和原文。
operation_knowledge_items: 运营知识主题包。保留可编辑知识包，但知识类型、业务上下文和适用场景由 AI 自由生成，不使用固定枚举。
operation_knowledge_chunks: Agent 运行时真正按需打开的知识切片。保存 routing_card、正文、安全事实、禁止承诺、证据和原文引用。
knowledge_usage_logs: Agent 运行时知识工具调用和引用审计日志，记录 selectedKnowledgeIds、selectedChunkIds、toolTrace、routeResult、回复文本和 Review 结果。
agent_decision_reviews: 独立评审 Agent 的评分、风险、拦截和改写记录。
```

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
operation_playbooks
```

AI Agent 系统：

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

自我演化（M4 / agent-self-evolution）：

```text
experiments              # 一次 tick 的信封：experiment_id / cohort_summary / budget / status
proposals                # 候选：threshold + prompt 共用，status pending_eval/evaluating/eligible_for_release/rejected_below_threshold/released/rolled_back
shadow_replays           # 单次 source_run 在新阈值/新 prompt 下的重判结果
threshold_overrides      # 已发布的阈值覆盖：(workspace, account, gate_key)，rolled_back_at=null & released_at 最新者生效
```

各 collection 关键字段（详见 `src/models.rs` Proposal / Experiment / ShadowReplay / ThresholdOverride）：

```text
experiments:
  experiment_id (unique) / workspace_id / account_id / started_at desc
  cohort_summary { thresholdCount, promptCount }
  budget { tokensUsed, llmCalls }
  status: running | finished | aborted

proposals:
  experiment_id / workspace_id / account_id / proposal_kind (threshold|prompt)
  status / created_at desc
  threshold path:    gate_key / current_value / proposed_value
  prompt path:       proposed_template_key / proposed_section / diff_snippet / critic_reasoning
  eval: replays_completed / replays_failed / significance_passed / eval_metrics
  released_at / released_by / rolled_back_at / rolled_back_by

shadow_replays:
  proposal_id / source_run_id / new_review.scores.* / new_review.final_status
  outcome_delta { sendSuccess, selfCritique, fiveGateHits }

threshold_overrides:
  workspace_id / account_id / gate_key
  proposed_value / released_by / released_at desc / rolled_back_at (null=生效)
  对应 proposal_id 用于审计
```

prompt_templates 同步升级为多版本形态（M4 W0 一次性迁移）：`(prompt_key, version)` 唯一 + `(prompt_key, current_version=true)` 至多一行；`seeded_by` 新增枚举 `evolution_release`。

## API Design Principles

API 按产品模块组织，而不是按 MCP 工具组织。

当前：

```text
/api/accounts
/api/contacts
/api/conversations
/api/events
/api/tasks
/api/outcomes/autonomy
/api/evolution/experiments        GET     # admin 看 experiment 信封列表
/api/evolution/proposals          GET     # admin 看 proposal 列表（按 status 桶）
/api/evolution/proposals/:id/release    POST  # admin 触发 release（threshold|prompt 自动分派）
/api/evolution/proposals/:id/rollback   POST  # admin 触发 rollback
/api/evolution/rollback_all       POST    # admin 二次确认（输入 ROLLBACK_ALL）一次性回滚
```

未来建议：

```text
/api/users
/api/groups
/api/moments
/api/content-assets
/api/agent-policies
/api/operations
/api/management-agent
/api/agent-souls
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

## Management Agent Contract

Management Agent 也必须输出结构化计划，不能直接把自然语言当作执行结果。

建议计划结构：

```json
{
  "intent": "send_contact_message",
  "riskLevel": "act",
  "requiresConfirmation": true,
  "target": {
    "type": "contact",
    "id": "contact_id"
  },
  "steps": [
    {
      "action": "resolve_contact",
      "status": "pending"
    },
    {
      "action": "send_message",
      "status": "pending"
    }
  ],
  "operatorSummary": "准备给指定好友发送消息，等待确认。"
}
```

建议 API：

```text
POST /api/management-agent/sessions
POST /api/management-agent/sessions/:id/messages
GET  /api/management-agent/commands/:id
GET  /api/management-agent/tool-catalog
POST /api/management-agent/confirmations/:id/approve
POST /api/management-agent/confirmations/:id/reject

GET  /api/agent-souls
POST /api/agent-souls
GET  /api/operation-playbooks
POST /api/operation-playbooks
POST /api/operation-playbooks/generate
PUT  /api/operation-playbooks/:id
POST /api/operation-playbooks/:id/set-default
PUT  /api/contacts/:id/operation-profile
POST /api/contacts/:id/analyze-profile
GET  /api/contacts/:id/operating-memory
PUT  /api/contacts/:id/operating-memory
GET  /api/operation-knowledge
POST /api/operation-knowledge
PUT  /api/operation-knowledge/:id
DELETE /api/operation-knowledge/:id
GET  /api/operation-knowledge/documents
POST /api/operation-knowledge/documents
GET  /api/operation-knowledge/documents/:id
PUT  /api/operation-knowledge/documents/:id
DELETE /api/operation-knowledge/documents/:id
GET  /api/operation-knowledge/documents/:id/chunks
GET  /api/operation-knowledge/chunks
POST /api/operation-knowledge/chunks
PUT  /api/operation-knowledge/chunks/:id
DELETE /api/operation-knowledge/chunks/:id
POST /api/operation-knowledge/import-preview
POST /api/operation-knowledge/import-apply
GET  /api/operation-knowledge/catalog
POST /api/operation-knowledge/tools/search
POST /api/operation-knowledge/tools/open-slice
POST /api/operation-knowledge/tools/open-evidence
POST /api/operation-knowledge/test-match
GET  /api/operation-knowledge/usage
GET  /api/decision-reviews
GET  /api/decision-reviews/:id
PUT  /api/agent-souls/:id
POST /api/agent-souls/:id/publish
GET  /api/prompt-templates
POST /api/prompt-templates
PUT  /api/prompt-templates/:id
POST /api/prompt-templates/:id/publish
POST /api/prompt-templates/reset-system-pack
GET  /api/operation-domains
GET  /api/operation-domains/:domain
PUT  /api/operation-domains/:domain
GET  /api/operation-domains/:domain/state-machine
PUT  /api/operation-domains/:domain/state-machine
POST /api/operation-domains/:domain/reset
POST /api/agent-tasks/:id/review-now
POST /api/agent-tasks/:id/cancel
```

第一阶段 Management Agent API 通过后端代理暴露 MCP 工具执行能力。请求必须绑定 `accountId`，后端按账号读取 MCP Key，执行结果写入 `agent_tool_calls` 和 `mcp_call_logs`。

`prompt_templates` 当前用于 Prompt Stack v2，核心字段：

```text
prompt_key
agent_kind
layer
title
description
content
status
version
prompt_pack_version
created_by
```

`operation_domain_configs` 用于把不同运营域拆开管理。当前用户运营频道已使用 `user_operations` 配置长期目标、方法论、工作流、工具边界、自动化策略、复盘规则、运行参数和状态机。运行参数不是提示词参考，而是发送网关的硬规则。

用户运营自动发送统一经过发送网关。私聊自动回复和 follow-up worker 都必须执行 managed 检查、冷却检查、频控、每日触达上限、上下文刷新、Review Agent 和审计记录。`agent_tasks` 的 follow-up 不允许直接调用微信发送工具。
