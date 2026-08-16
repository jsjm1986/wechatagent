# WechatAgent

WechatAgent 是一个面向微信私域关系运营的 AI Agent 平台。后端使用 Rust/Axum，MongoDB 保存业务事实、队列和审计数据，React 管理端负责配置与运营，通过 MCP Server 接入微信能力，并由 workspace 级 LLM Provider 完成决策、知识检索、独立评审和后台管理。

> 文档同步基线：`3db6cf6`（2026-08-13，优化波次 S0 + 三线 + S5 两线合并后）。该基线的确定性核验为 `cargo test --lib` **2,562 passed / 0 failed**、4 组 PBT **41 passed / 0 failed**、前端 Vitest **750 tests passed**、禁词与 CI 政策 lint 全绿。上一轮全量验收（`e9ba277`，2026-08-04）另含：CI 定义的 Knowledge evidence 5 条与 Tenant isolation 3 条 Docker/testcontainers hard gate 已在本机真实执行并全部通过；全量 ignored soft suite 因首次链接使 `target/` 膨胀并触及本机磁盘安全线而主动中止。真实 LLM、真实 MCP、GitHub Actions 和生产数据迁移仍需在对应环境单独验收。

当前可用闭环聚焦**微信好友私聊运营**：联系人默认处于 `normal`，只有管理员显式加入 `managed` 后，Agent 才会自动消费入站消息、维护画像与长期记忆、生成跟进任务并发送回复。微信群运营和朋友圈运营目前只是前端频道占位，不参与自动运营。

## 能力范围

| 领域 | 当前实现 |
| --- | --- |
| 用户运营 | 联系人同步与导入、纳管、画像、标签证据、运营状态、长期记忆、Playbook、静默时段与主动跟进（显式交易意向可豁免静默延迟） |
| 决策与发送 | Reply Agent、独立 Claim Gate/Review（寒暄低风险首稿可按七条件跳过 Claim Gate 并留审计标记）、单次修订、durable Outbox、投递前二次安全检查、长度加权发送节奏、MCP 发送 |
| 知识 Wiki | 文本/文档/PDF/图片导入、切片、修订、审核、渐进式工具检索、问答、缺口和使用反馈 |
| AI 总控 | Management Agent 将自然语言命令转换为冻结计划；工具白名单、风险分级、确认门和 durable intent 可审计 |
| 业务运营 | 产品与成交、冻结受众的 Campaign、素材、专属顾问名片、Ask-Human 决策通道（含预授权底线 standing order）、发送成效 |
| 配置治理 | Prompt、Soul、Domain Profile、Domain Schema、Taxonomy、状态机和策略的版本化发布与回滚 |
| 自治评估 | 运行审计、业务结果、Shadow replay、阈值候选和可选 Evolution worker（生产发布仍受管理员控制）；105 条合成场景金标回归环（`scripts/quality-regression.sh`，shadow 零发送） |
| 多租户 | 主要业务数据按 `workspace_id + account_id` 隔离；每个微信账号可维护独立 MCP 配置 |

## 设计原则与边界

- **默认不接管**：未纳管联系人只保存消息，不自动回复；退出纳管、停止信号、冷却期和静默时段会在执行侧再次校验。
- **事实与生成分离**：模型输出不是事实。产品声明和知识引用必须经过 grounding；AI 创建或修改的 Wiki 内容只能进入 `draft + needs_review`，不能自行标记为 verified。
- **决策与副作用分离**：Agent 先产生可审计决策，再通过 Outbox 执行文本、媒体或名片发送；Evolution 不能直接调用生产发送链。
- **失败时保守**：无效状态、丢失 claim、过期授权、知识不足和无法确认投递结果时均停止、请示或等待外部确认，不以猜测继续推进。
- **持久状态优先**：MongoDB 中的 task、lease、claim token、generation、幂等键和运行台账承担崩溃恢复；进程内锁、缓存、事件总线只用于协调和降低延迟。
- **后台请示而非人工接管**：Ask-Human 向决策人获取裁决，再由 AI 向客户转述；客户对话不会直接切换为真人会话。

## 系统架构

```text
React 19 Admin (Vite + TypeScript + Zustand)
  | REST / WebSocket / SSE
  v
Rust 2021 / Axum API ------------------------------------+
  | Agent Gateway / Review / Management / Knowledge      |
  | supervised background workers                        | serves frontend/dist
  v                                                       |
MongoDB replica set                                      |
  | business state, queues, leases, versions, audit      |
  +--> workspace-scoped LLM Provider Registry            |
  +--> durable Outbox --> safety recheck --> MCP Server --> WeChat
```

单个进程托管 `/api`、`/webhooks/wechat`、`frontend/dist` 和后台 worker。启动入口在所有平台统一创建具有 32 MB 栈的专用线程和 Tokio runtime（规避 Windows 默认小栈下迁移、Prompt seed、缓存预热等启动期深调用的栈溢出）。

### 启动顺序

`src/main.rs` 按以下顺序启动；前置完整性检查失败时通常拒绝提供服务：

1. 加载 `.env`、日志和 `AppConfig`，记录进程启动时间。
2. 连接 MongoDB，依次执行数据库迁移和索引创建。
3. 在管理员表为空时尝试创建 bootstrap admin。
4. 校验 active domain 状态机，预热 Taxonomy 和 Domain Profile 缓存。
5. 从数据库加载 active LLM Provider；数据库为空时才以环境变量创建初始 provider。
6. 校验可选的双 Reviewer 和 RS256 JWT 配置。
7. 初始化 Prompt pack、Evolution critic prompt 和示例评测数据。
8. 启动受 supervisor 管理的 worker，最后监听 HTTP 端口。

长寿 worker panic 时会写入审计事件并按 1 至 30 秒指数退避；60 秒内连续 5 次快速 panic 会把持久控制状态置为 `open` 并停止重启。仅 `SYSTEM_OPERATOR_USERNAMES` 明确列出的系统操作员可通过 `/api/admin/worker-controls` 查看脱敏状态并请求恢复；多副本中只有一个副本能领取 half-open probe，稳定运行 60 秒后闭合，probe panic 会立即重新熔断。worker 正常返回代表主动关闭，不会被 supervisor 强制重启。

### Worker 矩阵

| Worker | 默认 | 职责与关闭方式 |
| --- | --- | --- |
| `task_worker` | 开启 | claim `agent_tasks`，执行私聊和 follow-up；间隔见 `TASK_WORKER_INTERVAL_SECONDS` |
| `inbound_reply_worker` | 开启 | 固定 250ms 轮询恢复入站积压（durable inbound reply 义务，含毒丸行隔离），并发度 `INBOUND_REPLY_WORKER_CONCURRENCY`（默认 4） |
| `post_decision_worker` | 开启 | 发送后投影（画像/记忆/观察回放），独立 token/call 预算，contact 级租约 |
| `import_worker` | 开启 | 异步导入、heartbeat、孤儿回收；成功分段写 48 小时 checkpoint，reclaim 只补缺失段，终态 CAS 成功后清理 checkpoint |
| `outbox_dispatcher` | 开启 | 租约认领、授权复核、MCP 投递、重试和终态收敛 |
| `media_storage_reconciler` | 开启 | 启动时及每小时修复媒体提交窗口、清理 orphan、故障资产 fail-close |
| `knowledge_task_worker` | 开启 | 串行执行知识长任务；`KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=0` 关闭 |
| `catalog_rebuild_worker` | 开启 | 消费目录重建 job；`CATALOG_REBUILD_WORKER_INTERVAL_SECONDS=0` 关闭 |
| `knowledge_feedback_worker` | 开启 | workspace-scoped Mongo lease + heartbeat 下汇总使用、置信度、结构检查、知识缺口和 lessons；间隔设为 `0` 关闭 |
| `strategic_planner` | 关闭 | 静默、承诺、阶段停滞、日历、续费和再激活扫描；由 `STRATEGIC_PLANNER_ENABLED` 开启 |
| `cold_contact_worker` | 关闭 | 冷联系人低频重激活；由 `COLD_CONTACT_WORKER_ENABLED` 开启 |
| `silence_signal_worker` | 关闭 | 只记录无回复 censored signal，不发送消息；由 `SILENCE_SIGNAL_WORKER_ENABLED` 开启 |
| `evolutionary_worker` | 关闭 | Shadow 评估与候选生成；由 `EVOLUTION_ENABLED` 形成运维硬开关 |
| `knowledge_digest_worker` | 关闭 | 生成知识日报；由 `KNOWLEDGE_DIGEST_ENABLED` 开启 |
| `ingest_worker` | 关闭 | 条件抓取 RSS/HTML 并写入待审核草稿；由 `INGEST_WORKER_ENABLED` 开启 |
| `management_command_sweeper` | 开启 | 回收过期 Management 执行租约，将未知命令/tool intent 原子收敛为 `execution_unknown`，绝不重放外部副作用 |

## 核心业务闭环

### 私聊消息

```text
Webhook 验签、时间窗和账号限流
  -> 持久化 inbound message（无法解码的毒丸行按 _id 隔离为 quarantined 并审计，不阻塞后续行）
  -> managed 联系人物化 durable task（静默时段 defer 到唤醒点；显式交易意向可豁免 defer）
  -> claim token + generation
  -> Gateway 加载消息、画像、记忆、策略和 verified knowledge
  -> Reply Agent
  -> independent Claim Gate
  -> Review Agent
  -> 硬闸阻断，或软闸触发一次 revision
  -> durable Outbox
  -> Dispatcher 复核任务所有权、停止信号、冷却和陈旧度
  -> MCP 投递
  -> 消息、task、run、decision、发送台账和审计事件收敛
```

Review 的硬闸覆盖幻觉、知识 grounding、未验证产品声明等事实风险；human-like、压力风险、情绪价值、边界与隐私等软闸可要求一次改写。Reviewer 只读取生成结果和事实面，不接收 Reply Agent 的自我推理字段。开启双 Reviewer 时，第二 provider 配置不完整会导致启动失败。寒暄低风险首稿在七条件全部满足时跳过独立 Claim Gate（审计标记 `claim_gate_skipped_casual_low_risk`；rewrite/revision 与管理发送恒照跑）。发送间隔按文本长度加权：`base + 字符数 × 35ms`，封顶 `max + 6s`；间隔配置为 0/0（闸关）时恒为 0。

Agent run 生命周期为：

```text
started / running / completed
failed_before_decision / failed_after_decision
aborted_by_budget / aborted_by_external_signal
```

### 记忆与画像

每轮对话先写入 `memory_candidates`，再由 consolidator 在窗口内整理为 typed `MemoryCard` 和 `OperatingMemory`。冲突事实会被废弃或替换，`memory_card_version` 通过乐观并发控制防止覆盖；durable consolidation task 包含 prepared commit 和恢复路径。标签、人格 OCEAN 快照、承诺、关系状态与 operator memory 使用各自的证据和版本协议，不能把单次模型推断直接当作稳定事实。

### 主动运营

Strategic Planner 当前包含六类扫描：长期静默、承诺即将到期/已逾期、阶段停滞、日历/纪念日、续费和流失再激活。扫描器只创建确定性的 `follow_up` task，不直接调用 MCP；任务共享每日 cap，并继续经过 Gateway、Review 和 Outbox。Cold Contact 使用独立开关和配额；Silence Signal 只采集删失样本。

### Campaign

Campaign 状态闭集为 `draft / previewed / confirmed / dispatching / completed / canceled`。圈选先执行 MongoDB 粗筛，再按产品净持有、售后状态、价值层和客户阶段做内存精筛。preview/dispatch 冻结：

```text
specHash + specVersion + audience + intent + dispatchGeneration
```

派发阶段持久化 `campaign_sends` 和确定性 follow-up task；`dispatching` 可在崩溃后恢复，`completed` 不可再次派发。Campaign 不是绕过安全链的直接群发，每个对象最终仍经过 Gateway、Review 和 Outbox。单活动受众硬上限默认 500，可通过 `CAMPAIGN_MAX_AUDIENCE` 调整。

### Ask-Human

当 Agent 遇到折扣、承诺、敏感业务边界等决策墙时，会创建 principal escalation。决策人链、静默时段、每日 cap、去重窗口、超时转下一位等策略在创建时冻结，避免在途流程被配置变化改写。请示卡、裁决 relay 和 holding reply 都走 durable task/Outbox；决策人不可达时系统按策略延期或收口，不会悄悄放行高风险动作。运营可预写**预授权底线（standing order）**：链尾无人应答超过设定时限后，预写口径被当作与领导裁决同形的决议走既有 resolve→relay 通道由 AI 转述（本质是执行人类预授权而非 AI 代决；`standing_order` 与时限双字段成对校验防静默误配）；未配置时链尾只周期安抚、不自动放行。

### Knowledge Wiki

知识检索不依赖向量数据库，而是使用 MongoDB catalog 和 LLM 工具规划。Knowledge Agent 最多执行受限轮次的：

```text
list_catalog -> open_document -> open_chunk -> follow_relations -> answer
```

目录候选先按中英文 token/bigram 相关度重排；生产召回默认只暴露 `active + verified` chunk。Wiki 修订入口统一执行 locked fields、数组 union、schema 校验和异常缩短保护；内容变化会自动退回 `draft + needs_review`，revision 与 chunk 使用 CAS/事务提交。Knowledge Task 采用 session 串行、lease 和 heartbeat；问答支持 SSE token/trace/final/failed 事件。Auto Ingest 抓取的外部内容同样只能进入待审核状态。

知识对话和导入还包含显式的输出契约恢复：

- 每个知识 Chat turn 最多 4 次 LLM 调用、总 token budget 为 24,000；mutation 最多 1 轮工具循环，clarify 最多 3 轮，避免工具观察挤占最终业务输出。
- 新建或更新知识时，若模型返回“没有 patch 且没有有效追问”的矛盾空提案，可在剩余预算内最多做 2 次定向 contract repair；仍不完整时明确标记不可 apply，不伪造草稿。
- 文本导入最多 200,000 字符、64 段，每段硬上限 5,000 字符并最多做 3 次契约尝试；同步与 worker 入口共用 600,000 token 的运行预算。异步 job 会按 `(job_id, segment_index, content_hash)` 保存成功段 checkpoint，故障恢复只抽取缺失段；视觉抽取缺少必填文本字段时采用相同的有限重试思想。
- 批量导入或 auto-verify 若所有 LLM 调用均失败，会保留结构化 `LlmUnavailable` 错误；部分成功仍按部分结果处理，不把基础设施故障伪装成普通空结果。

### Management Agent

Management Agent 先生成带 hash 的冻结计划，再从白名单工具目录执行。每次工具调用先记录 durable intent，状态按 `prepared -> executing -> terminal` 推进；终态包括 `dry_run / succeeded / failed / executed_unverified / execution_unknown`。真实执行持有 token-fenced 命令租约并每 60 秒续约，工具前后均复核 ownership；后台 sweeper 只回收超过 5 分钟未续约的孤儿执行，并在同一事务中把命令和执行中的 tool intent 收敛为 `execution_unknown`，绝不自动重放。Provider 变更等高风险工具必须经过确认门。

## 可靠性协议

### Task 与 Outbox

- Task 认领同时绑定 `worker_id`、`claim_token` 和递增 generation；续租、完成和发送授权都必须匹配当前 claim，陈旧 worker 无权提交结果。
- Outbox 使用 workspace/account scoped 唯一幂等键；文本、文件素材和名片均进入同一持久发送队列。
- Outbox 状态闭集为 `pending / in_flight / sent / failed_terminal / canceled / delivery_unknown`。
- `delivery_unknown` 表示请求已经越过远端副作用边界，但没有可信回执。系统禁止自动重放，以避免客户收到重复消息，必须通过审计或外部核对处置。
- Dispatcher 在真正投递前再次确认联系人仍被纳管、任务与 decision 授权一致、消息没有过期且未收到停止信号。
- MCP 错误被区分为 `SafeToRetry` 与 `DeliveryUncertain`；单次请求硬超时为 60 秒，HTTP 200 中的 `result.isError=true` 仍按失败处理。

### 并发与一致性

系统组合使用 MongoDB 事务、唯一/partial 索引、TTL 索引、CAS、OCC、lease、heartbeat 和确定性 idempotency key。关键不变量包括单一 active/current/default 版本、同 scope 唯一 pending escalation、过期 session/job 自动清理，以及任务/Outbox fencing。不要通过数据库控制台绕过这些状态迁移和索引约束。

## 数据架构

项目使用约 79 个 MongoDB collection（集合-写入方矩阵见 `project-understanding/30-global-fact-cards.md` §8）。常用集合按领域分组如下：

| 领域 | 核心集合 |
| --- | --- |
| 账号与用户 | `wechat_accounts`, `contacts`, `conversation_messages`, `operating_memories`, `memory_candidates` |
| 执行与审计 | `agent_tasks`, `agent_send_outbox`, `agent_send_ledger`, `agent_run_logs`, `agent_decision_reviews`, `agent_events`, `mcp_call_logs`, `llm_call_logs` |
| 知识 | `operation_knowledge_documents`, `operation_knowledge_chunks`, `chunk_revisions`, `knowledge_chat_tasks`, `knowledge_usage_logs`, `knowledge_gap_signals`, `ingest_sources` |
| 配置与领域 | `prompt_templates`, `agent_souls`, `operation_playbooks`, `operation_domain_configs`, `domain_profiles`, `domain_schemas`, `system_taxonomies` |
| 业务 | `products`, `campaigns`, `campaign_sends`, `referral_cards`, `agent_principal_escalations`, `content_assets` |
| 管理与演化 | `management_agent_sessions`, `agent_command_runs`, `agent_tool_calls`, `experiments`, `proposals`, `shadow_replays`, `threshold_overrides` |
| 鉴权 | `admin_users`, `admin_sessions`, `auth_security_events` |

启动时会按编号执行 `src/db/migrations/` 中的 62 个有序迁移（m001–m062），然后创建索引。部分生产数据清理或回填迁移受 `APP_ENV=production` 和 `APPROVED_MIGRATIONS` 保护；执行前应备份并核对迁移源码。

> 完整功能和生产部署要求 MongoDB **replica set**。知识修订、版本发布、Evolution、Guide、Taxonomy、Provider 等路径使用多文档事务；standalone MongoDB 可能允许应用启动和简单浏览，但事务功能会在运行时失败。

## 外部适配

### LLM Provider

- active provider 按 workspace 保存于 `llm_provider_configs`；数据库配置优先，`.env` 仅用于首次 seed。
- 支持 OpenAI Chat Completions 兼容协议和 Anthropic Messages 形态；vision provider 可独立配置。
- Registry 支持进程内热切换；usage 将“上游未返回”与真实 `0` 分开记录。
- JSON 输出先做确定性修复，再进行最多两次受控 LLM repair；知识 Chat、导入和 vision 还会对各自的业务必填字段做有界 contract repair，所有修复都受 token/call budget 约束。
- OpenAI 兼容请求统一携带网关兼容的 `User-Agent`/`Accept`；404 默认不可重试，只有响应同时明确包含 `model_not_found` 和“当前动态账号组没有任何账号支持该模型”证据时，才归类为瞬时 `model_routing_unavailable`。普通 endpoint/model 404 继续 fail closed。
- 管理 API 会掩码展示 key，但当前 provider API key 在 MongoDB 中**没有应用层加密**。生产必须启用 MongoDB 鉴权、最小权限、磁盘和备份加密，并限制数据库与管理 API 的访问面。

### MCP 与微信

MCP 客户端支持 Streamable HTTP session；session 失效返回 404 时会重新握手一次。账号同步、联系人获取、文本/媒体/名片发送及状态日志均通过该边界。Webhook 应配置为：

```text
POST {APP_BASE_URL}/webhooks/wechat
```

生产必须保持 `WEBHOOK_VERIFY_SIGNATURE=true`，为每个账号配置独立 secret，并保留 timestamp 防重放检查。应用只信任直连 TCP peer，不读取 `X-Forwarded-For` 一类可伪造头；反向代理应在边缘独立完成可信客户端识别和全局限流。

### 媒体存储

媒体采用本地 content-addressed 路径：

```text
{MEDIA_STORAGE_DIR}/{workspace}/{shard}/{sha256}.{ext}
```

上传经过扩展名/MIME 白名单、路径穿越防护、pending file、`fsync` 和 atomic rename。reconciler 恢复“数据库已提交但文件尚未 rename”的崩溃窗口，并对缺失或非法对象 fail-close。生产应把 `MEDIA_STORAGE_DIR` 放在持久卷；多副本必须共享一致存储或采用单写部署，否则实例之间无法看到彼此的本地文件。

## 项目结构

```text
src/main.rs             启动、静态托管和 worker 注册
src/agent/              Gateway、Review、Memory、Outbox、发送与升级链
src/auth/               Argon2、Mongo session、ACL、限流和可选 RS256 JWT
src/db/                 typed collection、62 个迁移和索引
src/evolution/          与生产发送副作用隔离的演化器
src/knowledge_wiki/     Wiki、catalog、修订、反馈与自动导入
src/knowledge_task/     可恢复的知识长任务
src/knowledge_digest/   知识日报生成
src/routes/             REST、WebSocket 与 SSE API
src/webhooks.rs         微信回调、验签和 durable inbound handoff
src/tasks.rs            带 claim fencing 的 task worker
tests/                  集成、PBT、E2E、红线和真实模型测试
frontend/src/           React feature、store、contract 和组件测试
scripts/                CI 门禁、业务测试和维护脚本
docs/                   产品、架构、策略、数据和设计文档
```

## 环境要求

- Rust stable（crate 使用 Rust 2021 edition）
- Node.js 20 和 npm
- MongoDB replica set；运行 testcontainers 集成测试时还需要 Docker
- 可访问的 MCP Server、微信账号凭证和 webhook secret
- OpenAI Chat Completions 或 Anthropic Messages 兼容的 LLM endpoint

## 快速启动

1. 创建本地配置：

   ```bash
   cp .env.example .env
   ```

   PowerShell：

   ```powershell
   Copy-Item .env.example .env
   ```

2. 至少设置连接信息和首个管理员：

   ```dotenv
   MONGODB_URI=mongodb://localhost:27017/?replicaSet=rs0
   MONGODB_DATABASE=wechatagent
   MCP_BASE_URL=http://localhost:3001
   MCP_API_KEY=replace-me
   OPENAI_BASE_URL=https://api.example.com
   OPENAI_API_KEY=replace-me
   OPENAI_MODEL=replace-me
   BOOTSTRAP_ADMIN_USERNAME=admin
   BOOTSTRAP_ADMIN_PASSWORD=<use-a-long-unique-password>
   SYSTEM_OPERATOR_USERNAMES=admin
   ```

   `BOOTSTRAP_ADMIN_*` 只在 `admin_users` 为空时创建首个管理员。已有管理员后不会重复写入。`SYSTEM_OPERATOR_USERNAMES` 是区分租户管理员与全局 worker 操作员的精确用户名白名单，默认空值会关闭控制接口。不要沿用 `.env.example` 中的示例 endpoint、模型或凭证值。

3. 启动后端：

   ```bash
   cargo run
   ```

   默认监听 `http://localhost:8080`。首次启动会执行迁移、索引、seed 和缓存预热，然后启动 worker。

4. 启动前端开发服务器：

   ```bash
   cd frontend
   npm ci
   npm run dev
   ```

   打开 `http://localhost:5173`；Vite 将 `/api` 和 `/webhooks` 代理到 `http://localhost:8080`。

5. 登录后同步微信账号、设置账号级 MCP/webhook secret、导入联系人，并只将需要自动运营的联系人加入 `managed`。

### 生产式本地运行

```bash
cd frontend
npm ci
npm run build
cd ..
cargo run --release
```

后端会托管 `frontend/dist`，可直接访问 `http://localhost:8080`。若只启动前端开发服务器，必须同时运行后端。

## 配置与安全

完整字段和默认值以 [.env.example](.env.example) 与 `src/config.rs` 为准。生产上线至少检查：

- 设置 `APP_ENV=production`，显式配置 `MONGODB_URI`、`MONGODB_DATABASE` 和 `APP_BASE_URL`，并在执行受保护迁移前备份。
- 使用 HTTPS，设置 `SESSION_COOKIE_SECURE=true`；管理端 session 为 HttpOnly、SameSite=Strict，TTL 由配置决定。
- 保持 webhook 验签开启；对登录、token、webhook 和公网 API 增加反向代理级限流。
- JWT 默认关闭。启用 `JWT_ENABLED=true` 时必须同时提供可解析的 RS256 私钥和公钥，否则应用拒绝启动。
- 不提交 `.env`、MCP key、LLM key、JWT 私钥或生产数据；定期轮换密钥并限制日志、备份和管理接口访问。
- Evolution、Digest、Strategic Planner、Cold Contact 和 Auto Ingest 默认关闭。环境变量是运维硬开关，UI 不能绕过。

认证中间件只公开 `/api/health`、`/api/auth/login` 和 `/api/auth/token`。JWT 关闭时 `/api/auth/token` 仍可访问，但会返回 `jwt_disabled`。每次 cookie/JWT 请求都会重新读取管理员 ACL，因此移除权限会立即生效。登录和 token 共用进程级 client/target/global 失败窗口。

## API 与管理端

后端当前定义 235 条路由（计数会随开发漂移，完整事实源是 `src/routes/mod.rs`）。主要域如下：

```text
/api/auth                    登录、登出、workspace、session 和 JWT
/api/accounts                微信账号同步、登录状态与 MCP 配置
/api/contacts                联系人、纳管、画像、记忆和运营状态
/api/management-agent        AI 总控会话、计划、命令、确认和工具目录
/api/operation-knowledge     文档、切片、导入、审核、修订和工具检索
/api/knowledge               问答、日报、缺口、长任务和指标
/api/products                产品、成交和持有状态
/api/campaigns               活动预览、确认、派发和追踪
/api/admin                   策略、审核、Outbox、模型和可观测性
/api/evolution               实验、候选、Shadow、发布和回滚
```

`GET /api/health` 只返回 `ok`、`appBaseUrl` 和 `evolutionEnabled`。它是进程存活探针，**不会**检查 MongoDB、MCP 或 LLM 就绪状态；生产 readiness/依赖告警需在外部补充。

管理端共有 20 个频道，定义在 `frontend/src/app/channels.ts`：

| 分组 | 频道 |
| --- | --- |
| 运营 | AI 总控、账号管理、工作台、用户运营、微信群运营、朋友圈运营、统一收件箱、请示通道配置、活动、产品与成交 |
| 知识 | 内容资产、专属顾问、知识库 Wiki |
| 系统 | 系统策略、AI 模型配置、任务日志、自治回路监控、演化中心、运营成效、发送成效 |

微信群运营与朋友圈运营当前都指向 Overview 占位组件；其余频道已有独立 feature。账号选择保存在浏览器 localStorage，workspace 选择保存在服务端 session；切换 workspace 会刷新页面。实时 chunk 事件使用进程内 WebSocket broadcast，客户端 lag 时执行全量失效刷新。

## 开发与验证

```bash
cargo check                              # 后端快速编译检查
cargo fmt --all -- --check              # Rust 格式检查
cargo test --lib                         # 后端单元测试
cargo test --test state_transition_pbt   # 运行指定集成/PBT target

cd frontend
npm run build                            # TypeScript 检查并生成 dist
npm test                                 # Vitest 单次运行
npm run test:watch                       # Vitest watch 模式
```

Linux/CI 合并基线：

```bash
bash scripts/check-baseline.sh
```

Windows PowerShell：

```powershell
./scripts/check-baseline.ps1
```

基线要求 `cargo test --lib` 至少 350 项通过、`RUSTFLAGS="-D warnings" cargo check --tests` 通过，并要求 `state_transition_pbt`、`memory_card_invariants`、`wiki_chunk_revision_pbt` 和 `llm_retry_jitter` 累计至少 33 项通过且无失败。

金标回归环（105 条合成场景 shadow 回归，零真实发送，需真实 LLM key）：

```bash
bash scripts/quality-regression.sh
```

场景库在 `tests/fixtures/quality_gold/`（五类 × 21），断言逻辑在 `tests/quality_gold_regression.rs`（红线硬门 + judge 软门）；CI nightly `quality-gold` job 以软门运行，缺 key 时真实失败而非假绿。

多数 `#[ignore]` 集成测试需要 testcontainers MongoDB，也可通过 `TEST_MONGODB_URI` 使用预置测试实例。CI 另有知识证据、租户隔离、secret、manifest 和 literal lint 等阻断门；完整 ignored suite 当前属于 `continue-on-error` 诊断 job。真实模型、动态能力和红线测试主要由 nightly workflow 执行，不能用单次本地单元测试替代。

仓库还保留三份 **Maycran 临时验证 workflow**：

- `maycran-llm-backfill.yml`：仅 `workflow_dispatch`，可按真实模型测试 target/filter 串行回填证据；依赖仓库 Secrets，不属于普通 PR 的默认合并门。
- `maycran-fix-validation.yml` 与 `maycran-model-probe.yml`：只监听历史测试分支 `test/maycran-llm-backfill-20260802` 的 push；合并到 `main` 后不会因普通 main push 自动执行。
- 这些 workflow 用于供应商路由、备用模型和多 Judge 质量证据补跑，不能替代 baseline、租户隔离、Outbox/Task ownership 等确定性红线。

测试命名应贴近不变量或行为，例如 `*_pbt.rs`、`*_integration.rs`、`*_redline.rs`。修复并发或持久化问题时，优先增加崩溃恢复、重复执行、陈旧 claim 和跨租户用例，而不只覆盖成功路径。

## 生产部署

推荐部署顺序：

1. 准备带认证、备份和监控的 MongoDB replica set。
2. 构建 `frontend/dist` 和 Rust release binary；将 `.env`/secret 由部署系统注入。
3. 为 `MEDIA_STORAGE_DIR` 挂载持久存储，并确认运行用户拥有写权限。
4. 在执行迁移前备份；单实例完成迁移和启动完整性检查后再逐步接流量。
5. 通过 HTTPS 反向代理暴露应用，配置可信边缘限流、body 上限和超时。
6. 配置每个微信账号的 webhook URL 与 secret，验证 inbound、Review、Outbox 和 MCP 回执全链路。
7. 监控 `delivery_unknown`、`failed_terminal`、worker panic、claim 回收、LLM/MCP 延迟、磁盘空间和知识审核积压。

仓库根目录的 `deploy.sh` 是带固定服务器目录、分支、IP、systemd service 且会合并并推送 `main` 的历史专用脚本，**不是通用部署入口**。执行前必须逐行审计并改造；仓库当前没有可直接复用的通用容器或 systemd 发布方案。

### 多副本限制

durable task、Outbox、迁移索引和多数版本协议依靠 MongoDB fencing/CAS，可承受 worker 竞争；以下能力仍是进程级，不能视为跨副本强一致：

- LLM registry 热切换及部分 Prompt/Domain/Taxonomy/completeness 缓存传播；
- Management provider mutation lock、登录失败限流和部分本地协调锁；
- Wiki chunk edit lock、chunk/WebSocket event bus 和实时进度广播；
- 媒体路径锁与默认本地文件系统；
- 部分低延迟唤醒、缓存和运行时配置可见性。

多副本部署应使用共享媒体存储、外部全局限流，并为配置热切换增加消息广播或滚动重启。对要求全局串行的管理操作，应在外部部署层限制为单写实例，或补充分布式 lease/CAS。

## 已知非目标

- 微信群画像、群节奏和群工具工作流尚未实现。
- 朋友圈内容计划、发布队列和互动复盘尚未实现。
- 知识检索目前没有向量数据库，采用 catalog + 工具规划。
- 鉴权使用本地管理员、Mongo session 和可选 JWT，未集成第三方 IdP/SSO。
- `/api/health` 不是依赖 readiness 探针。
- 本地媒体和若干进程内协调能力不支持无条件水平扩展。

## 文档导航

- [开发文档索引](docs/README.md)
- [系统架构](docs/architecture.md)
- [AI Agent 系统](docs/ai-agent-system.md)
- [Agent 策略与自动化边界](docs/agent-policy.md)
- [数据与 API](docs/data-and-api.md)
- [知识 Wiki](docs/knowledge-wiki.md)
- [前端设计系统](docs/frontend-design-system.md)
- [代码审查问题台账](CODE_REVIEW_FINDINGS.md)
- [贡献指南](AGENTS.md)

`docs/` 中部分文件保留了历史设计、规划或建议。发生冲突时，以当前代码、`.env.example`、数据库迁移、索引和 `.github/workflows/` 为运行行为的事实源。
