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
