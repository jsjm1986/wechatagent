# System Architecture

## Current Architecture

```text
React Admin
  -> Rust Axum API
    -> MongoDB
    -> MCP Server
    -> DeepSeek/OpenAI-compatible API
```

当前系统是一个 Rust 单体服务：

- 托管 React 静态文件
- 暴露后台 API
- 接收微信 webhook
- 调用 MCP 工具
- 调用 LLM
- 执行任务 worker
- 写入 MongoDB

## Layering

系统应保持以下分层：

```text
Product Modules
  用户运营 / 群运营 / 朋友圈 / 内容资产 / 策略 / AI Command Center / 日志

Agent Layer
  Management Agent / Operations Agents / 意图判断 / 回复生成 / 画像更新 / 任务生成 / 策略执行

Application Services
  联系人服务 / 群服务 / 朋友圈服务 / 内容资产服务 / 任务服务

Infrastructure
  MCP Client / LLM Client / MongoDB / Webhook / Worker
```

原则：

- Product Module 不直接裸调 MCP。
- Agent 不直接关心 HTTP 和数据库细节。
- MCP Client 只负责协议和错误包装。
- 自动化边界由 Agent 策略决定，不散落在业务代码里。
- Management Agent 只能调用产品动作和授权工具，不直接裸调任意 MCP 工具。

## Current Backend Modules

```text
src/main.rs       启动、路由、静态文件、worker
src/config.rs     环境变量配置
src/db.rs         MongoDB 连接和索引
src/models.rs     数据结构
src/mcp.rs        MCP JSON-RPC 客户端
src/llm.rs        OpenAI-compatible LLM 客户端
src/agent.rs      私聊 Agent 决策和执行
src/routes.rs     后台 API
src/webhooks.rs   微信消息 webhook
src/tasks.rs      跟进任务 worker
```

## Agent Types

系统应明确区分两类 Agent：

```text
Management Agent
  面向内部操作员，负责自然语言后台操作、跨模块调度、执行计划和确认流。

Operations Agents
  面向具体运营对象，负责好友、微信群、朋友圈等长期业务运营。
```

Management Agent 的输入是操作员指令，例如“把 xx 加入运营列表”。Operations Agent 的输入是业务事件和上下文，例如好友新消息、群消息摘要、朋友圈计划。

两类 Agent 不共用运行日志和权限模型，但可以共用 LLM client、内容资产、策略服务和 MCP client。

## Recommended Evolution

随着模块扩展，后端应逐步拆出 service 层：

```text
src/services/contact_service.rs
src/services/group_service.rs
src/services/moment_service.rs
src/services/content_asset_service.rs
src/services/agent_policy_service.rs
src/services/agent_soul_service.rs
src/services/management_agent_service.rs
src/services/task_service.rs
```

不要为了抽象而抽象。只有当业务逻辑开始跨路由、worker、webhook 复用时再拆。

## Webhook Flow

当前私聊流程：

```text
POST /webhooks/wechat
→ 解析 appId/fromWxid/content/messageId
→ 定位微信账号和联系人
→ 保存 inbound message
→ 如果 contact.agent_status != managed，停止
→ 构建 Agent 上下文
→ 调用 LLM 生成决策
→ 调用 MCP message_send_text
→ 保存 outbound message
→ 更新画像/记忆/任务
→ 写入事件日志
```

后续群聊 webhook 应使用独立流程，不复用私聊自动回复逻辑。

## Worker Flow

当前任务 worker：

```text
定时扫描 pending task
→ 到期任务置为 running
→ 调用 MCP 发送
→ 成功置 sent
→ 失败置 failed
→ 写入事件日志
```

后续应补充：

- 重试次数
- 下次重试时间
- 失败分类
- 任务来源模块
- 幂等键

## Evolution Worker Flow（M4 / agent-self-evolution）

可选后台 tick（`EVOLUTION_ENABLED=true` 才起；默认 false）。完整设计见 `docs/agent-policy.md` 自我演化章节。运行链路：

```text
[evolution::tick] 每 EVOLUTION_TICK_SECONDS 触发一次
  ↓
[evolution::cohort::select_cohorts]
  ↓ 抽 threshold cohort + prompt failure cohort
  │  （per-contact cap=3，最少 EVOLUTION_MIN_REPLAYS=30 才发起）
  ↓
[evolution::threshold::generate]            [evolution::prompt::generate (Critic LLM)]
  │  按 THRESHOLD_REASONABLE_BANDS 决定         │  失败 cohort + 当前模板 → diff_snippet
  │  +step / -step                              │  validate_diffs（剥禁词 / 长度门）
  ↓                                              ↓
[Proposal] status=pending_eval ──────┬──────────┘
                                     ↓
[evolution::replay::run_shadow_replay] 仅读 agent_run_logs
                                     │  ❌ 不写 agent_send_outbox
                                     │  ❌ 不调 mcp_client
                                     │  ❌ 不写 conversation_messages
                                     ↓
[evolution::significance] EVOLUTION_MIN_SEND_SUCCESS_DELTA / *_SELF_CRITIQUE_DELTA
                                     │ + EVOLUTION_MAX_5GATE_HIT_INCREASE
                                     ↓
                          ┌──────── significance_passed? ────────┐
                          ↓                                       ↓
              status=eligible_for_release           status=rejected_below_threshold
                          ↓
              admin 在 EvolutionCenterTab 手工
                          ↓
[evolution::release::release_threshold|release_prompt]
  ↓ Mongo session transaction
  │  - threshold: insert threshold_overrides（rolled_back_at=null）
  │  - prompt:    bump version + current_version 切换 + prompt_pack_version +1（LRU 失效）
  ↓
[agent::resolve_thresholds] / [generate_agent_json] 在下一个生产 run 入口读到新值

回滚：admin 点 rollback → release.rs::rollback_threshold|rollback_prompt
       threshold: rolled_back_at=now → resolve_thresholds 读回 baseline
       prompt:    current_version 切回旧 version + prompt_pack_version 再 +1
```

红线（CI 守门）：

- `src/evolution/` SHALL NOT 引用 `crate::agent::gateway / outbox / mcp::` 任意符号（`scripts/check-evolution-isolation.{sh,ps1}`）。
- 所有新增 `agent_events.kind` / 前端文案过 `scripts/check-no-human-takeover.{sh,ps1}` lint。
- 100 次 shadow replay 后 `agent_send_outbox` 集合 size 不变（`tests/evolution_isolation.rs`）。

## Knowledge Digest Worker Flow（knowledge-digest-workstation）

可选后台 cron-like worker（`KNOWLEDGE_DIGEST_ENABLED=true` 才起；默认 false）。完整设计见 `docs/agent-policy.md` 知识库日报工作站章节与 `.kiro/specs/knowledge-digest-workstation/`。运行链路：

```text
[knowledge_digest::worker_loop] 每天 KNOWLEDGE_DIGEST_RUN_HOUR 触发一次
  ↓
[knowledge_digest::generate_today_digest(account_id)]
  │  RUN_BUDGET.scope(token=24000, calls=8) {
  ↓
  ├─ analyze_chunks_health(db)   读 operation_knowledge_chunks
  ├─ analyze_usage_logs(db, 24h) 读 knowledge_usage_logs
  ├─ analyze_run_logs(db, 24h)   读 agent_run_logs（block/hold）→ summarize_logs LLM
  └─ analyze_evolution(db, 24h)  仅读 proposals
  ↓
  compose_cards(signals) → LLM `knowledge.digest.compose`
  │  ├─ JSON schema 校验
  │  ├─ targetRefs 外键存在性校验
  │  ├─ 排序 R2.5 + 截断 ≤ 50
  │  └─ 失败 → status="failed" / partial
  ↓
  upsert knowledge_daily_reports (accountId + reportDate unique)
  写 KnowledgeUsageLog{kind="digest_compose"}
  写 AgentEvent{kind="knowledge_digest_generated"}
  }

[运营 09:30 进 Knowledge 频道] GET /api/knowledge/digest/today
  ├─ 命中 → 直接渲染
  └─ 未命中 → 同步 generate_today_digest（同入口、同 budget）

[画布勾选 N 卡 → 派工]
  ↓
POST /api/operation-knowledge/chat (intent=digest_action) → plannedSteps
  ↓
POST /api/knowledge/chat/tasks (sessionId, cardIds, plannedSteps)
  ↓ knowledge_chat_tasks{status="pending"}
  ↓
[knowledge_task::worker_loop] 每 30s tick
  ↓ 取 status=pending 按 sessionId 串行
  for step in plannedSteps:
    RUN_BUDGET.scope(token=8000, calls=4) {
      match step.action {
        fix_chunk   → 走现有 chunk repair propose+apply（强制 draft+needs_review）
        add_chunk   → 走现有 chunk_from_request（强制 draft+needs_review）
        retag       → 走现有 /chunks/:id/extract-tags
        review_evo  → 不动 evolution，仅在 chat 提示运营
        dismiss     → 写 dismissedCardIds
      }
      // 每步写一条 knowledge_chat_turns{kind="task_progress"}
      // 失败 fail-soft，不阻塞后续 step
    }
  ↓ 全部完成
  写 knowledge_chat_turns{kind="task_summary", attachments: needs_review chunkIds}
  写 AgentEvent{kind="knowledge_chat_task_finished"}

[前端 SSE GET /api/knowledge/chat/sessions/:sid/stream] 实时推 turn id
  ↓
chat 流追加 progress / summary turn

[运营点 summary 里 chunkId] → KnowledgeChunkEditor 二次审核
  → 现有 #329 sourceQuote → anchor gate → verify
  → chunk 进 verified 池
```

红线（CI 守门）：

- `src/knowledge_digest/` 与 `src/knowledge_task/` SHALL NOT 写 `prompt_templates` / `threshold_overrides`（演化器红线 R9.3）。
- 三层（worker tick / chat per-turn / task per-step）都被 `RunBudget` 卡死；超额即终止当前层。
- 任何 LLM 失败统一走 `AppError::LlmUnavailable` → 前端 `<LlmErrorBanner>`，不允许新增第二套错误样式。
- 所有新增文案 / `agent_events.kind` 过 `scripts/check-no-human-takeover.{sh,ps1}` lint。
- AI 永不自动 verify：worker / chat / task 三条路径产出的 chunk 一律 `status="draft" + integrityStatus="needs_review"`。
- 节奏 1 阶段**不**接事件驱动 push；webhook 实时不会主动叫醒 chat。

## Knowledge Wiki Subsystem（knowledge-wiki Phase A–G）

把"销售话术 RAG"升级为"运营知识 Wiki + 检索面"。**召回算法零改动**（catalog → list_chunks → open_slice 不动），本子系统专心做扎实的四件事：质量 / 可被检索 / 可被修改 / 可被优化。设计原则与 LLW 借鉴对照见 [`docs/knowledge-wiki.md`](knowledge-wiki.md)；字段 / 路由 / 集合见 [`docs/data-and-api.md`](data-and-api.md#knowledge-wiki-子系统phase-a-g)。

### 写入路径（同步）

```text
[POST /chunks/:id/patch | split | merge | archive | restore | rollback | import-apply ...]
  ↓
apply_chunk_revision (src/knowledge_wiki/chunk_revisions.rs)
  ├─ 1. 锁定字段守门：patch 含 chunk_id / wiki_type / created_at / source_anchor /
  │     verified_at / verified_by / approved_at 任意一项 → 400 BadRequest
  ├─ 2. 数组字段 union（src/knowledge_wiki/page_merge.rs）：tags / related_chunks /
  │     sources / search_terms / applicable_scenes 永远 existing ∪ patch（应用层，0 LLM）
  ├─ 3. 70% body 长度阈值：patch 改 answer/explanation 后 new_len < old_len × 0.7 → 400
  ├─ 4. AI 写入强制 status=draft + integrity_status=needs_review
  ├─ 5. 双写：先写 chunk_revisions（不可变历史，sha256 before/after hash），
  │           再写 operation_knowledge_chunks（可变最新版）
  └─ 6. enqueue catalog_rebuild_jobs（异步，写入路径不阻塞）
```

### 异步 worker（两条独立 loop）

```text
[catalog_rebuild_worker]                                  默认每 3s 一轮
  ├─ 取一批 catalog_rebuild_jobs status=queued
  ├─ 按 document 聚合 active chunk → 渲染 markdown
  ├─ 落 documents.catalog_summary_persisted + 自增 catalog_version
  └─ job.status = done / failed (3 次失败标 failed，feedback worker 周期重试一次)

[feedback_worker]                                         默认每 600s 一轮
  ├─ 1. 30d 滑窗聚合 knowledge_usage_logs → 每 chunk usage_stats.{hit,blocked}_count_30d
  ├─ 2. dynamic_confidence = clamp(integrity × 0.6 + hit_rate × 0.4 - stale_penalty, 0, 1)
  ├─ 3. structural lint（纯查询，无 LLM）：5 类规则信号
  │     orphan / broken_link / no_outlinks / low_confidence / stale
  │     → 写入 / 合并 knowledge_gap_signals（按 normalized_title 去重）
  └─ 4. stage 1 sweep：candidate 不再被规则生成 / target 已恢复 / valid_to 已推到未来
        → status=auto_resolved
        stage 2（LLM 批裁决）接口预留，本轮不进入热路径
```

### 召回路径（零改动 + fire-and-forget hook）

```text
[现有] catalog → list_chunks → open_slice → tool-loop reply
  ↓ (write_knowledge_usage_log 写 log 后)
fire-and-forget: knowledge_wiki::gap_signals::record_chunk_hit
  └─ $inc usage_stats.hit_count_30d 或 blocked_count_30d
     $set last_used_at / last_blocked_reason
  注意：不阻塞 reply 返回；失败 ignore（`let _ = ...`）
```

### 隔离红线（CI 守门）

- `src/knowledge_wiki/*` SHALL NOT 引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`。
- `record_chunk_hit` 仅接 `&Database`，不接 `AppState`，避免误用 LLM / outbox。
- `feedback_worker` / `catalog_rebuild_worker` 启动按 `*_INTERVAL_SECONDS == 0` 立即 return（零资源消耗）。
- `apply_chunk_revision` source=ai 强制 draft+needs_review，**AI 永不自动 verify**（红线沿用）。
- 所有新增 prompt / schema / UI / docs / 错误信息过 `scripts/check-no-model-hint.sh`，不暗示具体 LLM 品牌；LLM provider 由运营在 `LlmProviderConfigs` 自填。

## Deployment Shape

第一阶段保持简单：

```text
one Rust process
one MongoDB
external MCP Server
external DeepSeek API
```

当任务量或 webhook 量上升后，再考虑：

- API 和 worker 进程拆分
- 队列系统
- 多实例部署
- webhook 签名校验
- 日志/指标采集

## Phase 0 → E5-T1 时代图（updated）

本节是 `## Webhook Flow` / `## Worker Flow` 之后的补丁——把 Phase 0 → E5-T1 的实际链路落到一处。两份原图保留了"当前最小可运行链路"的语义；本节描绘的是 reaction / outbox / multi-account / multi-locale / reviewer 双模 / ops 三表灰度全部接通后的真实形态。

### 私聊 Webhook Flow（全链路）

```text
POST /webhooks/wechat
→ 解析 appId / fromWxid / content / msgId（参考 webhooks.rs:45-295）
→ account_scheduler::resolve_account_context 选 persona/capacity/off_hours 命中的账号
→ 持久化 inbound message
→ 若 contact.agent_status != managed → 停止（只持久化）
→ run_user_operation_gateway:
  1. reload 联系人 + 历史 + 三类 prompt（locale-aware：load_prompt_for_contact 按
     contact.locale 选 prompt_template 版本，未命中 fallback 到 zh-CN）
     +（Phase E5-T1）operation_domain_configs / operation_state_policies / system_taxonomies
       三表 active_versions 桶选：hash(contact_id) % active_count，同 contact 同桶稳定，
       老库无 current_version 字段时 `$ne:false` / `$exists=false` 兜底
  2. enforce_decision_guards 三闸：grounding / hallucination / run_budget
     +（Phase B）双软闸：human_like / pressure_risk → 触发 single-shot revision
     +（Phase A）taxonomy::check_value 校验 customer_stage / intent_level / objection_type，
       未命中走 taxonomy::upsert_candidate（不阻塞 run）
     +（Phase B）operation_state_policies forbidden 拦截
  3. knowledge_router：catalog → list_chunks → open_slice
     +（Phase B）按 chunk_type 分段拼接（product_fact verified-only / style_template few-shot
       / negative_example don't-do / peer_case reference）
     +（Phase D）拼接 contact.intent_trajectory 近 5 项
     +（Phase A）注入 reaction_analysis 近 3 轮 + load_operator_memory
  4. decide_reply_with_promote → review_decision（reviewer 输入遮蔽 draft.reasoning）
     +（Phase E2）REVIEWER_DUAL_MODEL_ENABLED=true 时 LlmProvider 双模并行 reviewer，
       分歧（评分差≥阈值或 grounding/hallucination 决策不一致）触发 single-shot revision
  5. （Phase D）style_consistency_check：与 contact.last_outbound_style 比对，差异≥3/5 axes
     时强制 single-shot revision
  6. approved → agent_send_outbox enqueue（idempotency key）→ 二次安全门 → MCP message_send_text
→ 写 agent_run_logs（lifecycle 走 update_run_envelope_terminal，闭集校验）
→ reaction_phase 异步：record_user_reaction 写 reaction_analysis +
  reviewer_misjudge_signal + intent_trajectory.push（cap 50） + last_outbound_style 回写
```

### Worker Flow（多 worker 并行）

```text
tokio::spawn 主进程内 8 条 loop（启停由 env / mongo flag 控制）：

1. tasks::worker_loop                  follow-up task 调度，走同一 gateway
2. outbox_dispatcher::run_outbox_dispatcher  五状态机 + idempotency + retry/backoff
3. planner::run_strategic_planner_loop  M3 strategic planner（commitment due / silent followup）
4. evolution::worker（EVOLUTION_ENABLED=true 时启）  threshold / prompt 灰度 + post_release
5. knowledge_wiki::feedback_worker      30d 滑窗 usage_stats / dynamic_confidence + structural lint
                                        + sweep_stale_signals + lessons_learned 14d 聚合
6. catalog_rebuild_worker               documents.catalog_summary_persisted 增量重写
7. knowledge_digest::run_loop           日报工作站（chat-only async tools）
8. cold_contact_worker                  按 last_outbound_at 阈值挑联系人 → peer_case 钩子重激活
                                        （COLD_CONTACT_WORKER_ENABLED env 开关，默认 false）
```

### Phase 0 → E5-T1 新增 collection / 字段速查

| 范畴 | collection / 字段 | 来源 |
| --- | --- | --- |
| FSM 闭集 | `agent_run_logs.lifecycle / final_review_status / gateway_status`（assert_*_valid 守门） | Phase 0 |
| 反馈信号 | `decision_reviews.reaction_analysis` 用于下轮 prompt | Phase A |
| 操作员记忆 | `load_operator_memory` 在 build_context 阶段注入 | Phase A |
| 双层标签 | `system_taxonomies` + `taxonomy_candidates` | Phase A |
| 双闸 | `human_like_gate` / `pressure_risk_gate` 软闸 + 阈值 | Phase B |
| 知识用途 | `OperationKnowledgeChunk.chunk_type` 4 类 | Phase B |
| 状态策略 | `operation_state_policies` collection | Phase B |
| 误判信号 | `decision_reviews.reviewer_misjudge_signal` + `reviewer_stats` | Phase C |
| 演化 flag | `evolution_runtime_flags` collection | Phase C |
| 阈值历史 | `threshold_overrides` + `threshold_overrides_audit` | Phase C |
| 多版本 prompt | `prompt_templates` 多 active + soft-retire `current_version` | Phase C |
| 意图轨迹 | `Contact.intent_trajectory: Vec<IntentTrajectoryEntry>` cap 50 | Phase D |
| 风格指纹 | `Contact.last_outbound_style: Option<String>` | Phase D |
| 多账号 | `WechatAccount.{capacity, persona_tag, off_hours}` | Phase D |
| 跨用户教训 | `lessons_learned` collection（pending_review → peer_case chunk 候选池） | Phase D |
| 多 locale | `Contact.locale` + `PromptTemplate.locale`（BCP-47，默认 zh-CN） | Phase E3 |
| LLM provider 抽象 | `trait LlmProvider` (`src/llm_provider.rs`) + reviewer 双模并行（`REVIEWER_DUAL_MODEL_ENABLED`） | Phase E2 |
| ops 三表灰度 | `operation_domain_configs / operation_state_policies / system_taxonomies` 加 `version / current_version / previous_version / seeded_by`；`hash(contact_id) % active_count` 桶；`admin_ops_versions` 三动作 publish/rollout/rollback | Phase E5-T1 |

### 模块隔离红线（不变）

- `crate::knowledge_wiki::*` 严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`。
- `crate::evolution::*` 严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`。
- group / moments domain 永远不折叠到 user-ops 代码路径（CLAUDE.md 红线）。当落地时通过 `trait OpsDomain`（Phase E1，留待第二个 domain 真实需求驱动）分发，user-ops 为第一实现。

