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

## Knowledge Digest Workstation（knowledge-digest-workstation）

完整设计见 `.kiro/specs/knowledge-digest-workstation/` 与 `docs/agent-policy.md` 知识库日报工作站章节。本节列出新增的数据模型与路由。

### 新增 collections

```text
knowledge_daily_reports
  _id, accountId, reportDate (YYYY-MM-DD), generatedAt, generatedBy ("worker"|"manual"),
  status ("ok"|"partial"|"failed"), errorKind?,
  budgetSnapshot { tokensUsed, llmCalls },
  cards: [KnowledgeDigestCard],
  dismissedCardIds: [ObjectId],
  promptVersions { intent, draft, ... }
索引: { accountId: 1, reportDate: -1 } unique compound

knowledge_chat_tasks
  _id, sessionId, accountId, operatorId,
  cards: [KnowledgeDigestCard],            // 任务起源 cards 快照
  plannedSteps: [{cardId, action, targetChunkId?, hint?}],
  completedSteps: [{cardId, action, chunkId?, error?}],
  status ("pending"|"running"|"finished"|"failed"|"cancelled"),
  errorKind?,
  createdAt, startedAt?, finishedAt?
索引: { sessionId: 1, status: 1 }
索引: { status: 1, createdAt: 1 }          // worker 取 pending

knowledge_operator_memory
  _id, accountId, operatorId,
  kind ("preference"|"rejection"|"context"),
  content, createdAt, lastUsedAt, expiresAt?
索引: { accountId: 1, operatorId: 1, lastUsedAt: -1 }
```

`KnowledgeDigestCard` 结构（嵌入 `knowledge_daily_reports.cards` 与 `knowledge_chat_tasks.cards` 快照）：

```text
{
  cardId,                                  // ObjectId 持久 id（前端勾选 / dismiss 用）
  kind ("chunk_missing_field"|"chunk_low_hit_rate"|"chunk_caused_block"|
        "pack_outdated"|"evolution_pending"|"evolution_released"|"freeform"),
  title (≤ 60 字),
  summary (≤ 200 字),
  targetRefs: [{kind ("chunk"|"pack"|"item"|"run"|"evolution_proposal"), id}],
  suggestedAction ("fix_chunk"|"add_chunk"|"retag"|"review_evolution"|"dismiss"|"freeform"),
  severity ("info"|"warn"|"critical"),
  metric? { name, value, threshold }
}
```

排序优先级：`severity=critical` > `kind=chunk_caused_block` > `kind=chunk_missing_field` > 其他；同级内按 `targetRefs[0].id` 稳定排序。

### 扩展现有 collection

```text
knowledge_chat_turns
  追加 kind?: "task_progress"|"task_summary"|"tool_call_log"|null
  追加 attachments.tool_calls?: [{name, params, result, latency_ms, tokens}]
  其余字段不变（向后兼容；旧 turn 缺字段视为 null）
```

### 新增路由

```
GET  /api/knowledge/digest/today                  当日日报；命中即返回，未命中同步触发合成
POST /api/knowledge/digest/regenerate             手动重算（body: { force?: bool }）
POST /api/knowledge/digest/cards/:cardId/dismiss  忽略一张卡片
POST /api/knowledge/chat/tasks                    chat 派工（body: {sessionId, cardIds, plannedSteps}）
GET  /api/knowledge/chat/tasks/:taskId            轮询任务进度
POST /api/knowledge/chat/tasks/:taskId/cancel     取消未开始 / 运行中的任务
GET  /api/knowledge/chat/sessions/:sid/stream     SSE 长连接，推 turn id
GET  /api/operation-knowledge/logs/analyze        24h block/hold runs 反查 chunk（tool-calling 用）
```

`POST /api/operation-knowledge/chat` / `chat/:sid/apply` 等已有路由不变；本轮在 `chat_turn` handler 内增加 `intent="digest_action"` 与 `intent="update_operator_memory"` 两个分支。

### 新增配置（`.env`）

```
KNOWLEDGE_DIGEST_ENABLED=false                # 默认关停；运维显式打开
KNOWLEDGE_DIGEST_RUN_HOUR=9                   # 每天 09:00（运营时区）
KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET=24000       # 单次 worker tick token 上限
KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS=8          # 单次 tick LLM 调用上限
KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=30     # task worker tick 间隔；0 表示停掉
```

### 新增 PromptSpec

```
knowledge.digest.compose          worker 4 数据源摘要 → 卡片数组
knowledge.digest.dispatch         运营选 N 卡 → plannedSteps 拆解
knowledge.digest.summarize_logs   24h block/hold log 聚合 → 1 句话 issue
```

经 `ensure_prompt_pack_v2` seed；版本号挂在 `knowledge_daily_reports.promptVersions` / `llm_call_logs.prompt_version`。

### 新增 AgentEvent kind

```
knowledge_digest_generated         worker 完成一次合成
knowledge_digest_failed            合成失败 / 超预算
knowledge_chat_task_created        派工落库
knowledge_chat_task_finished       task 全部步骤完成（含 fail-soft）
knowledge_chat_task_cancelled      运营取消
knowledge_operator_memory_added    新增运营偏好 / 拒绝项
```

所有 kind 过 `scripts/check-no-human-takeover.{sh,ps1}` lint，不引入"接管 / 人工"语义。

## knowledge-wiki 子系统（Phase A–G）

把"销售话术 RAG"升级为"运营知识 Wiki + 检索面"。本节只列字段 / 路由 / 集合 / 索引；方法论与 lifecycle 见 [`docs/knowledge-wiki.md`](knowledge-wiki.md)。**召回算法零改动**（catalog → list_chunks → open_slice）。

### 新增字段（向后兼容，全 Option）

`OperationKnowledgeChunk`：

```
wiki_type             Option<String>          source / entity / concept / comparison /
                                              synthesis / methodology / finding / query / thesis
domain_attributes     Option<Document>        行业字段 JSON 容器；由 active domain_schemas 校验
provenance            Option<ChunkProvenance> { source: ai|human|rule|imported,
                                              source_doc_id?, source_quote?,
                                              llm_model_alias?(只写 provider_id),
                                              edited_at, edited_by? }
valid_from / valid_to Option<DateTime>        时效性（合同期 / 活动期），feedback worker 标 stale
superseded_by         Option<String>          redirect 到新 chunk_id
previous_version_id   Option<String>          版本链
related_chunks        Option<Vec<RelatedRef>> { chunk_id, kind: superseded_by|references|
                                              requires|contradicts|clarifies|refines, note? }
usage_stats           Option<UsageStats>      { hit_count_30d, blocked_count_30d,
                                              last_used_at?, last_blocked_reason? }
dynamic_confidence    Option<f64>             feedback worker 回写
locked_fields         Option<Vec<String>>     默认 [chunk_id, wiki_type, created_at,
                                              source_anchor, verified_at, verified_by, approved_at]
```

`OperationKnowledgeDocument`：

```
catalog_summary_persisted  Option<String>     catalog 落库 markdown 快照
catalog_version            Option<i64>        每次 rebuild 自增
```

旧文档读出 None 不报错；写入路径自然填新字段。

### 新增集合

```
chunk_revisions
    { _id, chunk_id, revision_id, op, patch: Document,
      before_hash: sha256, after_hash: sha256,
      source: ai|human|rule|imported, reason?, created_at, created_by? }
    op ∈ create | patch | split | merge | rollback | archive | restore | verify | unverify
    索引：(chunk_id, revision_id desc)、(created_at desc)

knowledge_gap_signals
    { _id, signal_id, kind, title, description,
      affected_chunk_ids: [String], search_queries: [String],
      severity: warning|info, source: rule|llm,
      status: pending|auto_resolved|llm_resolved|applied|dismissed,
      resolution_note?, created_at, resolved_at? }
    kind ∈ orphan | broken_link | no_outlinks | low_confidence | stale
           | contradiction | missing_chunk | suggestion
    索引：(workspace_id, status, kind)、(workspace_id, created_at desc)

domain_schemas
    { _id, schema_id, workspace_id, name, version, fields: [DomainField],
      alias_dict: Document, guard_dsl?, is_active, created_at, updated_at }
    DomainField: { name, label, kind: string|enum|number|date|reference,
                   required, allowed_values?, alias_of? }
    索引：(workspace_id, schema_id, version desc)、(workspace_id, is_active)

catalog_rebuild_jobs
    { _id, job_id, workspace_id, document_id, queued_at, status,
      attempts, last_error?, started_at?, finished_at? }
    status ∈ queued | running | done | failed
    索引：(workspace_id, status, queued_at)
```

### 新增 chunk 编辑路由（7 个）

走同一函数 `apply_chunk_revision`：read-existing → 锁定字段守门 → 数组字段 union → 70% body 长度阈值 → AI 写入强制 draft+needs_review → chunk_revisions + chunks 双写（先 revisions 后 chunks）→ enqueue catalog_rebuild_job。

```
POST   /api/operation-knowledge/chunks/:id/patch                     字段级 patch
POST   /api/operation-knowledge/chunks/:id/split                     拆分（原 archive + 新建 N 个）
POST   /api/operation-knowledge/chunks/:id/merge                     合并（原 + target archive + 一个新）
POST   /api/operation-knowledge/chunks/:id/archive                   软删 + 删除级联
POST   /api/operation-knowledge/chunks/:id/restore                   取消 archive
POST   /api/operation-knowledge/chunks/:id/rollback/:revision_id     回滚到指定 revision 的 before-state
GET    /api/operation-knowledge/chunks/:id/revisions                 分页 timeline
POST   /api/operation-knowledge/chunks/:id/relate                    维护 related_chunks（受限 6 关系枚举）
DELETE /api/operation-knowledge/chunks/:id/relate/:target_id         移除单条关系
```

### 新增 catalog 双轨

```
GET /api/operation-knowledge/catalog/persisted                        读 documents.catalog_summary_persisted（O(1)）
GET /api/operation-knowledge/catalog                                  实时聚合 fallback（既有路由保留）
```

### 新增 gap-signals 路由（4 个）

```
GET  /api/knowledge/gap-signals?status=pending&kind=&limit=           列信号
POST /api/knowledge/gap-signals/:id/dismiss                           运营手动消解
POST /api/knowledge/gap-signals/:id/apply                             应用建议（接口预留）
POST /api/knowledge/gap-signals/sweep                                 手动触发 lint + stage 1 sweep
```

### 新增 domain-schemas admin 路由（5 个）

```
GET    /api/admin/domain-schemas?workspaceId=&activeOnly=             列出
POST   /api/admin/domain-schemas                                      新建（自动 version=既有 max+1）
PUT    /api/admin/domain-schemas/:schemaId                            更新（updates latest version in place）
DELETE /api/admin/domain-schemas/:schemaId                            删除（不允许删 active）
POST   /api/admin/domain-schemas/:schemaId/activate                   切换 active（同 workspace 同时只能 1 条）
```

校验红线：`fields.len() <= 64`、`field.name` 不与 chunk 主表既有字段冲突、kind=enum 必须提供非空 allowed_values、`alias_dict` 每个 value 必须存在于 fields[].name。

### 新增配置（`.env`）

```
KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS=600       # feedback worker tick；0 = 关停
CATALOG_REBUILD_WORKER_INTERVAL_SECONDS=3     # catalog rebuild worker tick；0 = 关停
```

两个 worker 启动时按 interval gated；为 0 立即 return，零资源消耗。

### import_apply 协议变更

`POST /api/operation-knowledge/import-apply` 不再让 LLM 返一次大 JSON。改要求 `---CHUNK: chunk_id---...---END CHUNK---` 流式块（fence-aware parse + unsafe path-like rejection + 流截断 warning）。每块独立校验 + 落库 + 写 chunk_revision (op=create, source=imported)。已闭合块照常落库（不全有总比全无好）。

### record_chunk_hit fire-and-forget hook

`agent::knowledge_router::write_knowledge_usage_log` 写 log 后 fire-and-forget 调用 `crate::knowledge_wiki::gap_signals::record_chunk_hit`：命中 `$inc usage_stats.hit_count_30d`，被 block `$inc usage_stats.blocked_count_30d` + `$set last_blocked_reason`。**不阻塞召回路径**；隔离红线：knowledge_wiki 子系统不引用 `crate::agent::gateway / outbox`、`crate::mcp::*`。
