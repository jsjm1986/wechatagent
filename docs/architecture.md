# System Architecture

> Documentation snapshot: checked against commit `d60d3d85f8e193160dca8df185de0daef004a6b6` plus the uncommitted SR-001--SR-183 closure worktree on 2026-07-24. This is not deployment verification; source code and `.env.example` are authoritative if this document drifts. Sections explicitly labelled as recommendations are not shipped topology.

## Current Architecture

```text
React Admin
  -> Rust Axum API
     + supervised background workers (same process)
    -> MongoDB (business state, queues, leases, audit)
    -> workspace-scoped OpenAI-compatible LLM providers
    -> durable Outbox -> second safety gate -> MCP Server
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
- Management Agent 只能调用产品动作和授权工具，不直接裸调任意 MCP 工具。兜底透传分支只放行 `tools/list` 实际公布过的工具名（外加已注册的 `wechatagent.*` 产品工具），LLM 幻觉或提示注入产生的未公布工具名在打到生产 MCP 之前被拒绝（`routes/management.rs` `advertised_tool_names` + `execute_management_tool` 白名单门）。

## Current Backend Modules

```text
src/main.rs          启动、路由、静态文件、受监督 worker 注册
src/config.rs        环境变量配置与默认值
src/db/              MongoDB typed accessors、迁移和索引
src/models.rs        数据结构
src/mcp.rs           workspace/account scoped MCP JSON-RPC 客户端
src/llm.rs           OpenAI-compatible LLM client/provider 抽象
src/agent/           Gateway、决策、Review、Memory、Outbox 与发送调度
src/routes/          后台 API
src/webhooks.rs      微信消息 webhook 与 durable inbound handoff
src/tasks.rs         fenced 跟进任务 worker
src/evolution/       与生产发送链物理隔离的演化器
src/knowledge_*      Knowledge Agent、日报、长任务与 Wiki worker
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
→ 在 ACK 前持久化 inbound message / pending handoff
→ 如果 contact.agent_status != managed，停止
→ 去抖 runner 领取 generation，进入 run_user_operation_gateway
→ reload scope context → knowledge route → Reply → independent Review
→ precheck/finalize/state-action safety gates
→ 写 durable agent_send_outbox（先于任何 MCP 发送）
→ outbox dispatcher claim/lease + second safety gate
→ MCP 发送；timeout/reclaim 先做 post-hoc delivery verification
→ 收敛 outbound/run/task/audit 状态，异步分析反应与记忆
```

后续群聊 webhook 应使用独立流程，不复用私聊自动回复逻辑。

## Worker Flow

跟进任务与发送 worker：

```text
定时扫描 pending/retry task
→ claim token + generation + lease 原子领取（stale 可恢复）
→ 到期任务走同一 run_user_operation_gateway
→ Gateway 提交 task-bound outbox intent
→ outbox dispatcher 独立 claim 并在 MCP 前复核 task owner/generation
→ sent / retry / failed / canceled 以 fenced CAS 收敛
→ 周期恢复器重建 webhook handoff 与过期任务，不依赖进程内唤醒保证正确性
```

上述正确性依赖 Mongo 中的 durable task/outbox、幂等键、lease 和 fencing；进程内即时唤醒只用于降低延迟。

## Evolution Worker Flow（M4 / agent-self-evolution）

可选后台 tick。worker 由主进程注册，但 `EVOLUTION_ENABLED=false`（默认）时立即退出；显式设为 true 后仍需 workspace Mongo runtime flag 放行。当前代码政策 `CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 强制所有 proposal 人工发布，旧自动发布配置不能绕过。完整设计见 `docs/agent-policy.md` 自我演化章节。运行链路：

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
[POST /chunks/:id/patch | split | merge | archive | restore | rollback | verify | reject | auto-verify | batch-verify | import-apply ...]
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
  │     hit/block 标签默认取真实用户反应：按 run_id join decision_reviews.outcome_status
  │     （buying_signal→hit，负向集→block，沉默/pending/unclassified→删失排除，不进分母）。
  │     DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED=false 时退回 review_approved 旧统计。
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

**可选探索注入（KNOWLEDGE_EXPLORATION_ENABLED，默认 false）**：fallback 排序路径（非 agent cited 路径）在 verified 池内、top-k 之上做 softmax(score/temperature) 受控抽样，并把每个 chunk 的 selection_prob 落进 route_result → knowledge_usage_log，为未来 off-policy 纠偏（IPS/DR）留 propensity。探索只重排已过 grounding 的 verified 集合，FactRisk/ProductAccuracy/grounding 硬门在其后照常执行；本阶段只记录不消费。

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
external OpenAI-compatible LLM provider(s)
```

当任务量或 webhook 量上升后，再考虑：

- API 和 worker 进程拆分
- 队列系统
- 多实例部署
- 跨副本 lease/锁与共享媒体存储
- 集中式日志/指标后端

## Phase 0 → E5-T1 时代图（updated）

本节保留 Phase 0 → E5-T1 的详细历史演进说明；顶部 `Current Architecture`、`Webhook Flow` 与源码是当前事实入口。字段或阶段名若与源码冲突，以源码和本页快照标记为准。

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
  2. 三闸（grounding / hallucination / run_budget）— 实际入口是
     review::classify_dual_gate / review::review_passed（评分门）+
     review::finalize_review_for_send（verified 产品声明结构化兜底）；
     历史文档里的 `enforce_decision_guards` 是 2026-05-25 知识库清理前的旧符号，
     现已不存在，遇到请按上述真实符号阅读
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
     +（Phase E2）REVIEWER_DUAL_ENABLED=true 时 LlmProvider 双模并行 reviewer，
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
`spawn_supervised` 在主进程最多注册 13 条 loop（部分由 env/interval/Mongo flag 立即关闭）：

1. task_worker                          follow-up/durable inbound task，走同一 gateway
2. import_worker                        异步知识导入 job
3. outbox_dispatcher                    durable send claim / second gate / MCP / retry
4. media_storage_reconciler             本地内容寻址媒体启动恢复与周期一致性扫描
5. strategic_planner                    commitment due / silent followup（默认关）
6. cold_contact_worker                  冷联系人重激活（默认关）
7. silence_signal_worker                沉默删失信号（默认关）
8. evolutionary_worker                  env + Mongo 双闸（默认关；发布仍人工）
9. knowledge_digest_worker              日报合成（默认关）
10. knowledge_task_worker               chat plannedSteps 长任务（默认 30s）
11. catalog_rebuild_worker              documents.catalog_summary_persisted 增量重写（默认 3s）
12. knowledge_feedback_worker           30d usage_stats / dynamic_confidence + structural lint
                                        + sweep_stale_signals + lessons_learned 14d 聚合
                                        （dynamic_confidence 带最小样本门 DYNAMIC_CONFIDENCE_MIN_SAMPLES；
                                         hit/block 标签默认取真实用户反应 outcome_status，按 run_id join
                                         decision_reviews，沉默删失排除——DYNAMIC_CONFIDENCE_REAL_OUTCOME_ENABLED
                                         默认 true，置 false 退回 reviewer 自评 review_approved 旧统计）
13. ingest_worker                       active ingest source 条件抓取（默认关）
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
| LLM provider 抽象 | `trait LlmProvider` (`src/llm.rs::LlmProvider`，方法 `generate_json` / `generate_json_with_usage`) + reviewer 双模并行（`REVIEWER_DUAL_ENABLED`） | Phase E2 |
| ops 三表灰度 | `operation_domain_configs / operation_state_policies / system_taxonomies` 加 `version / current_version / previous_version / seeded_by`；`hash(contact_id) % active_count` 桶；`admin_ops_versions` 三动作 publish/rollout/rollback | Phase E5-T1 |

### 模块隔离红线（不变）

- `crate::knowledge_wiki::*` 严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、`agent_send_outbox`、`run_user_operation_gateway`。
- `crate::evolution::*` 严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`。
- group / moments domain 永远不折叠到 user-ops 代码路径（CLAUDE.md 红线）。当落地时通过 `trait OpsDomain`（Phase E1，留待第二个 domain 真实需求驱动）分发，user-ops 为第一实现。

