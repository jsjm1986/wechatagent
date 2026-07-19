# 全系统审查阅读笔记

所有笔记都以 [`baseline.json`](baseline.json) 冻结的 PR #223 head 为准。`FACT` 只表示当前代码或配置直接证明；文档声明不自动升级为运行事实。

## B01：规则、启动骨架、总路由与 CI

覆盖文件：23 个。逐文件状态见 `file-ledger.csv` 中 `batch_id=B01` 的行。

### 进程与启动（FACT）

- `src/main.rs:25-43` 在 32 MiB 专用线程中创建 Tokio 多线程 runtime，以规避 Windows 启动期深调用栈溢出。
- `src/main.rs:55-87` 的启动顺序是：读配置 → 记录进程启动时间 → 连接 Mongo → 运行迁移 → 建索引 → 可选 bootstrap admin → active 状态机 sanity check → taxonomy 与 DomainProfile cache 预热。
- `src/main.rs:88-112,417-477` 只从 `default_workspace_id` 选择/种入 active LLM provider，并构造一个进程级 `LlmRegistry`；多 workspace 的实际运行隔离需在 LLM provider 路由和调用点阶段继续核实。
- `src/main.rs:113-165` 的第二 Reviewer 与 JWT 都在启动期 fail-closed：开关启用但必要配置缺失或密钥不可解码会拒绝启动。
- `src/main.rs:166-200` 组装单个 `AppState`，并只对默认 workspace/account 种入默认 prompt、演化 prompt 和示例评测场景。
- `src/main.rs:202-312` 最多注册 12 个后台循环：task、import、outbox、strategic planner、cold contact、silence signal、evolution、knowledge digest、knowledge task、catalog rebuild、knowledge feedback、ingest。strategic planner 与 ingest 在 `main` 外层条件启动；其余部分开关在 worker 内部处理，具体停启语义待对应 worker 全文复核。
- `src/main.rs:314-342` 同一进程托管 `/api`、公开微信 webhook、SPA 静态文件，并允许任意 CORS origin/method/header；监听默认来自 `APP_HOST:APP_PORT`。
- `src/routes/mod.rs:301-345` 证明 DB、MCP、主 LLM、第二 Reviewer、JWT keys、进程内软锁/事件总线/缓存均共享在同一个 `AppState`。

### 路由与鉴权装配（FACT）

- `src/routes/mod.rs:347-1083` 是 API 总装配，覆盖账号、联系人、知识、评测、策略、管理 Agent、taxonomy、请示、outbox、LLM provider、DomainProfile 与 Evolution。
- `src/routes/mod.rs:1076-1081` 把 Session/JWT middleware 包在整个 `/api` Router 上；哪些路径被 middleware 白名单豁免，必须以后以 `auth/middleware.rs` 为准，不能只凭这里的注释。
- `src/routes/mod.rs:1085-1244` 有静态 tripwire：扫描已列入的 route 文件中的 `pub async fn`，若既未挂载也未列入 helper 白名单则测试失败。它只覆盖硬编码文件清单和文本模式，不等价于运行时路由完整性证明。

### 配置事实（FACT）

- `src/config.rs:424-736` 表明启动必需 env 只有 `MCP_API_KEY` 与 `OPENAI_API_KEY`；其余均有默认值或可空。
- `src/config.rs:378-421` 为 `AppConfig` 手写脱敏 `Debug`，覆盖 MCP/LLM/Reviewer/bootstrap/JWT 密钥字段。
- 当前默认值包括：Evolution 运维硬锁允许=true、Runtime flag 缺失时是否实际运行待 Evolution 模块核实；Strategic Planner=false；Cold Contact=false；Silence Signal=false；Digest=false；Ingest=false；Dual Reviewer=false；JWT=false；Webhook 签名=true；渐进提示档=true。
- `.env.example:10-14` 明确警告部分旧迁移会依据 `APP_ENV` 走破坏性开发分支，这一风险需在迁移阶段逐个核实。

### 数据层入口（FACT）

- `src/db/mod.rs:38-53` 的 `Database::connect` 无迁移/索引副作用；生产启动显式先迁移再建索引。
- `src/db/mod.rs:64-405` 暴露约 50 个 typed collection accessor，外加 `raw()`；集合的租户键、唯一性和生命周期仍需以模型、索引、迁移及所有调用点四向核验。
- `src/db/mod.rs:56-62` 暴露 Mongo `Client` 供跨 collection transaction 使用。

### CI 与交付（FACT）

- `.github/workflows/ci.yml:87-143` 的 backend hard gate 跑 lib 测试、baseline/PBT、两类文字红线和 evolution 隔离检查。
- `.github/workflows/ci.yml:145-195` 的全量 ignored Docker integration job 标记 `continue-on-error: true`，不阻断合并。
- `.github/workflows/ci.yml:197-219` 的前端 TypeScript + Vitest 是硬门，但只在 path filter 判定 frontend 变化时运行。
- `.github/workflows/ci.yml:221-1210` 的真模型能力、对抗、红线和 skip 门主要仅 nightly schedule 运行；PR 阶段不执行真模型行为门。
- `.github/workflows/nightly-dynamic.yml` 是独立、发现性、skip-friendly 的真模型角色扮演夜跑。
- `.github/workflows/probe_llm_endpoint.yml:5-57` 接受 `workflow_dispatch` 的普通字符串 input 作为 API key，并在 shell 中使用。GitHub workflow inputs 不是 secrets；是否可能在 UI/事件元数据或调试输出暴露需按平台行为确认，当前标为安全设计风险而非已证实泄漏。

### 文档与代码漂移（FACT）

- `README.md:122` 与 `docs/architecture.md:140` 声称 `EVOLUTION_ENABLED` 默认 false；`src/config.rs:5-7,617` 与 `.env.example:146-150` 当前默认 true。文档已过期。
- `README.md:160` 声称 lib baseline ≥78；`CLAUDE.md:64-70` 声称 ≥350。实际门值需在 `scripts/check-baseline.*` 全文阅读后裁决。
- `docs/architecture.md:390-410` 只列 9 条 worker；`src/main.rs:202-312` 当前最多注册 12 条，文档已过期。
- `docs/architecture.md:99-113` 的旧 webhook flow 描述为决策后直接 MCP；同文档 `348-385` 的新图改为 outbox。现状必须以后端实现为准。
- `docs/ai-agent-system.md` 同时包含当前实现与“建议/方向”，本批只把它作为产品意图，不将建议 API/模型视为已上线事实。

### 本批待后续核实

- 每个 API 的真实白名单、workspace 授权、字段过滤与副作用。
- `LlmRegistry` 是否真正支持 workspace 级隔离，及管理后台激活 provider 对其它 workspace 的影响。
- Worker 内部开关、claim/lease、幂等、多副本和恢复语义。
- 文档中所有 Agent 阈值、outbox 状态和知识红线与生产实现的一致性。

## B02：数据模型、索引与迁移

覆盖文件：36 个，合计 13,050 行。包括 `src/models.rs`、`src/db/indexes.rs`、迁移总装配、共享 helper 与 m001–m032；均按连续区间读到 EOF。集合目录与索引/迁移对应关系见 `data-model.md`。

### 模型与存储契约（FACT）

- `src/models.rs` 同时承担持久化模型、API 投影、请求 DTO、状态闭集和大量 BSON 兼容测试；部分核心结构仍以 `Document` 承载可变 schema，typed wrapper 只覆盖边界或渐进迁移。
- 主体集合多数使用 snake_case BSON；`LlmProviderConfig`、`Campaign`、`CampaignSend` 及若干嵌套值使用 camelCase。索引必须逐模型匹配真实序列化键，`indexes.rs` 已显式修复过错误 snake_case 索引的历史遗留。
- 租户键主要是 `workspace_id`，少数 camelCase 集合为 `workspaceId`。`m016` 列出 53 个 snake_case 与 3 个 camelCase 回填目标；taxonomy 两表由后续 `m032` 单独回填。
- 明文秘密仍作为业务字段持久化：`WechatAccount.mcp_api_key/webhook_secret` 与 `LlmProviderConfig.api_key`。模型手写 `Debug` 做掩码，但静态数据加密、API 投影和写路径必须在外部边界阶段继续核实。
- 状态闭集存在于 campaign、agent task、import job、tool call、knowledge task、principal escalation 等模型中；多数断言在 release 仅记录错误，是否真正拒绝写入取决于调用方控制流，需反查全部写点。

### 索引与生命周期（FACT）

- `ensure_all` 集中创建索引，并在迁移之后、HTTP 启动之前执行。业务事实表有唯一键/部分唯一键，诊断日志和若干临时集合有 TTL；TTL 默认值受环境变量影响。
- outbox 具备 `idempotency_key` 唯一索引、claim/retry/lease、发送节流和 delivery-finalize 扫描索引；其状态机与 crash recovery 仍需阶段 4/7 核实。
- ops 三表采用 `(workspace, logical key, version)` 唯一索引；`current_version=true` 的辅助 partial index不是 unique，数据库本身不保证单 current，详见 SR-008。
- taxonomy、candidate、relationship suggestion、suspected deal、campaign send、principal escalation 等均有租户/状态/幂等索引；各查询是否完整命中这些索引需在读写方阶段以真实 filter/sort 反查。

### 迁移执行语义（FACT）

- `migrations::run_with` 串行检查 marker、执行 step、再插入 marker；没有跨 step 事务，也没有执行中 lease。多进程同时首次启动时存在重复执行窗口，具体部署是否允许多副本并发启动待运行架构核实。
- m001–m032 覆盖字段拆分、typed memory、taxonomy seed、prompt/ops 版本化、多租户回填、状态机标志、产品生命周期、联系人身份清理与 outcome/escalation/taxonomy 修复。
- m011/m012/m014/m016 在 `APP_ENV=production` 时返回 `Ok(())`，runner 随后仍写 marker；风险见 SR-010。
- m009 的 current prompt 选择算法与注释不一致；m029 的 roster 身份映射丢失租户/账号维度；分别见 SR-007、SR-009。

### 本批待后续核实

- 每个集合的全部生产读写方、租户 filter 是否完备、状态断言是否在写入前真正拒绝。
- ops/prompt publish 与 rollback 的事务边界，是否能补偿非唯一 current 索引留下的并发窗口。
- 多副本同时执行 migration/ensure_indexes 的实际部署约束与故障恢复。
- TTL 清理是否与审计、统计和训练窗口契约一致。

## B03：鉴权、Webhook、MCP、LLM、账号与媒体外部边界

覆盖文件：18 个，合计 11,712 行。包括 `src/auth/**`、`src/webhooks.rs`、`src/mcp.rs`、`src/llm.rs`、账号调度与账号/鉴权/LLM provider/媒体路由、媒体存储与发送、共享路由 helper 和统一错误映射；均按连续区间读到 EOF，冻结 SHA-256 零漂移。

### Session、JWT 与 workspace 授权（FACT）

- `src/auth/password.rs:6-51` 用 Argon2id PHC、随机盐和进程级合法 dummy hash，用户名不存在时仍执行 verify，降低用户名枚举时序差。
- `src/auth/session.rs:42-184` 的 bootstrap 仅在集合空且用户名/密码均配置时创建 admin；session 是 Mongo 行，UUID v4 标识，固定到期、不滚动续期，默认 workspace 写入 session。
- `src/auth/middleware.rs:31-99` 只公开 `/health`、`/auth/login`、`/auth/token`；其余 `/api` 先尝试 cookie session，再在开关开启时尝试 RS256 Bearer。cookie/JWT 成功后直接注入其保存的 `current_workspace`，不在 middleware 中重读 AdminUser ACL。
- `src/routes/auth.rs:48-241` 设置 HttpOnly、SameSite=Strict、Path=/ 的 session cookie；Secure 由配置决定且默认 false。JWT token 只包含 user/username/current workspace，默认 60 分钟，无 refresh/revocation 记录。
- `src/routes/shared.rs:1577-1616` 提供按请求重读 ACL 的 `resolve_authorized_workspace`，但只被显式调用它的路由覆盖；账号、媒体等大量处理器直接使用 middleware 注入的 `admin.current_workspace`。撤权与初始 workspace 不一致风险见 SR-014。
- 登录和 token 签发都在 middleware 白名单内，代码中没有独立的 IP/用户名速率限制、失败计数或锁定；外围网关是否补偿未知，见 SR-016。

### Webhook 入口（FACT）

- `src/webhooks.rs:290-672` 的顺序为：JSON 解析 → 无副作用 `testMsg` 直接 ack → appId 解析账号 → 可选 HMAC+时间戳验签 → Online/Offline 状态落库 → 进程内 per-account 限流 → principal 分流 → Mongo 原子 dedupe/消息落库 → contact 更新 → quiet-hours 延迟或进程内 debounce runner。
- 签名开启时要求账号行存在非空 `webhook_secret`，验 `HMAC-SHA256("timestamp." + raw body)` 和默认 ±300 秒窗口；AddMsg 的 message-id unique、Online/Offline `$set` 与 principal 幂等承担窗口内重放防线。
- appId 的数据库索引是 sparse 但非 unique；解析和在线状态更新均只按 appId，重复值会产生不确定账号归属，见 SR-015。
- debounce key 含 `(workspace, account, wxid)`，但 runner 参数和 reload 查询丢失 workspace，只按 `(account_id, wxid)` 取 contact，见 SR-012。限流器、pending map 均为进程内状态，多副本不共享。
- 无 msgId 时 dedupe 回退完整 payload 的 FNV-1a hash；相同无 ID 内容连发会被当重复。代码将其明确标为非生产 GeWe payload 的已知边界。

### MCP 与账号同步（FACT）

- `src/mcp.rs:46-239` 实现 MCP initialize + Streamable HTTP session，缓存按 `(base_url, api_key)` 隔离，404 时清缓存并重握手一次；每次 HTTP 硬超时 60 秒，兼容 JSON/SSE，HTTP 200 的 `result.isError=true` 也按失败处理。
- `src/mcp.rs:304-425` 的普通调用固定记录默认 workspace/account；per-account 调用虽接收 account_id，却固定从 default workspace 查账号凭证，找不到再回落全局 MCP 凭证，日志也固定写 default workspace。所有生产调用签名只传 account_id，见 SR-011。
- `src/routes/accounts.rs:32-326` 列表/更新按当前 workspace 过滤；但账号同步本身使用全局默认 MCP 调用，并把全局 MCP URL/key 作为当前 workspace 新账号的初始凭证。手工凭证后续 sync 不覆盖。
- roster 支持快照、24h stale 判定、后台 single-flight 与重试；single-flight key 只有 account_id，进程内共享。MCP 调用日志对 base64 做大小占位脱敏，但保留 recipient/content/mediaId 供崩溃恢复精确核对。

### LLM provider 边界（FACT）

- `src/llm.rs:13-1471` 支持 chat-completions 与 messages 两种协议、HTTP 超时/重试/Retry-After、SSE、视觉输入、JSON 容错与最多两次模型修复；`LlmRegistry` 只持有一个无 workspace 键的当前 client/meta。
- `src/main.rs:88-112,417-477` 启动仅选择默认 workspace 的 active provider；`src/routes/llm_providers.rs:101-600` 的 CRUD/DB active 标记按授权 workspace 隔离，但更新或激活任意 workspace 的 active provider 都 swap 同一个进程级 registry。跨 workspace 串扰见 SR-013。
- provider API 列表只回 masked key；更新 payload 含 mask 占位时沿用旧 key；test 可用未落库 inline 配置发一次真实请求。数据库中的 provider/account API key 仍是明文业务字段。
- 最终 LLM 错误被分类为 503 结构化响应，其中 `detail` 保留原始错误字符串；对外隐藏 DB/MCP 原始错误的统一规则不适用于该专门分支，后续需结合前端与日志审查敏感数据暴露面。

### 媒体与错误边界（FACT）

- 媒体上传/换文件路由有按配置设置的 Axum body limit；handler 再校验大小、media type、扩展名与声明 MIME。磁盘相对路径只由 `(workspace, sha256, ext)` 生成，不使用原文件名，防路径穿越。
- 上传先写本地磁盘再插 DB；换文件先写新文件再更新 DB。中间 DB 失败会留下无引用文件，且本地磁盘不天然跨副本共享，见 SR-017。
- 素材默认 draft；发送前再次要求 `sendable=true`、`review_status=approved` 和合法媒体工具。换文件清 `media_id` 并退回 draft；发送成功后的消息落库失败只记录错误，不返回失败以避免重复发送。
- `src/error.rs:63-140` 将内部 DB/HTTP/序列化错误原文只写日志、HTTP 只给稳定分类码；限流返回 429+Retry-After，LLM unavailable 返回 503，普通 external/MCP 错误返回 502。

### 本批待后续核实

- outbox/dispatcher 对 MCP 日志 best-effort 写入失败、timeout 和进程崩溃的完整去重闭环。
- 所有调用 `logged_call_for_account` 的发送、请示、推荐和管理路径受 SR-011 的实际爆炸半径。
- Session/JWT 管理 UI 是否存在 admin ACL 编辑/删除路径，以及权限收缩后的预期撤销 SLA。
- 本地媒体目录在目标部署中是否挂载共享持久卷，备份、垃圾回收与恶意文件内容检测策略。

## B04A：Agent 契约、运行参数、预算与 Run Envelope 骨架

覆盖文件：8 个，合计 7,460 行、333,092 字节。包括 `src/agent/mod.rs`、`types.rs`、`runtime.rs`、`budget.rs`、`run_envelope.rs`、`pacing.rs`、`sufficiency.rs` 与 `review/style.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。B04A 只完成阶段 4 的契约与运行骨架；Decision、Review、Guards、Gateway、Outbox/Dispatcher、Reaction、Memory、Knowledge Router 与 Escalation 仍待后续子批次全文阅读。

### Agent LLM 公共入口与预算（FACT）

- `src/agent/mod.rs:203-505` 把主 Agent LLM 调用、usage 计费、失败分类、调用日志与 4 类精确缓存统一在 `generate_agent_json`。主决策 prompt 不进精确缓存；缓存仅覆盖 knowledge import preview、playbook generator/optimizer 和 user guide preview。
- cache key 只有 `(prompt_key, prompt_pack_version, hash(system), hash(user))`，不含 workspace、provider id、format、model 或 endpoint。provider 激活/swap 不递增 `prompt_pack_version`；相同 prompt 在切换 provider 后仍可能命中旧 provider 产物，见 SR-020。
- cache hit、success 与 failure 三条 `llm_call_logs` 写入路径都把 `workspace_id` 固定为 `state.config.default_workspace_id`；公共调用签名也没有 workspace 参数。非默认 workspace 的 LLM 使用审计会归错租户，见 SR-018。
- `src/agent/budget.rs:54-202` 用 task-local `RunBudget` 记录 token、LLM call 与 tool call 三维预算；tool call 在执行前原子检查次数和 token，失败不污染计数。LLM call 是调用完成后累计，再由后续降级点检查，不是发起前硬拒绝。
- progressive tier 可通过 `grant_escalated_ceiling` 抬高有效 token 上限但不改真实累计；`is_exceeded` 在任一维度达到上限时为 true。迁移 ID 唯一性和顺序测试已纠正过滤参数并真实各执行 1 条通过。

### 运行参数、决策契约与充分性（FACT）

- `src/agent/runtime.rs:18-549` 把 domain runtime Document 转为 typed 参数，统一 clamp tool/search/outbox/quiet-hours 参数，并按 `(workspace_id, account_id)` 读取最新未回滚 threshold override；若存在多个 current domain config，会按 contact hash 分桶，而不是拒绝重复 current。
- Profile threshold overrides 在 `apply_active_profile` 路径 clamp 到 1..=10；但 evolution collection 的 `ResolvedThresholds::apply_override` 将 f64 直接转换为 i32、不在此处再次 clamp。其写侧合理区间与发布事务将在 Evolution 阶段核实。
- `src/agent/types.rs:80-1130` 使用 `RawAgentDecision -> validate_and_promote` 区分字段缺失与显式 false/空值。final 轮聚合必填、枚举和关键轮长度风险；tool_calling 中间轮只校验 tool 名；自治协议开关关闭时跳过全部校验并走 legacy minimal decision。
- 决策边界对大量评分、数组和可选字段做宽容反序列化；协议风险在 promote 后由 Gateway/Review 如何 fail-closed 仍需 B04B 核实。`ReviewScores.boundary_privacy_safety` 对旧 JSON 缺失默认 0（最保守），而 sufficiency 未识别值由 `decide_tier_escalation` 回落 Enough，同时另有 telemetry 谓词标识畸形值。
- `src/agent/sufficiency.rs:32-109` 只在 Full 档记录路由知识 ID；非 Full 档清空 LLM 自报 ID，防止未实际读取的 verified chunk 架空 grounding gate。missing coverage + 需要知识会强升 Full；weak 只记录乐观偏差。
- pacing 和 style 是纯函数：账号发送间隔把随机值 clamp 后线性映射；风格指纹以长度、emoji、问号、感叹号、句尾和换行组成，至少 3 个轴变化才判漂移。

### Run Envelope（FACT）

- `src/agent/run_envelope.rs:1-38` 明确声明 started/terminal/panic-hook 三个 R0 原语已实现且有 ignored 集成测试，但生产未接线。全仓 `src/` 调用点反查也只命中定义、注释和模型引用，没有生产调用。
- 因 Gateway 仍在决策产出后一次性插入 run log，LLM 调用前 panic、timeout 或 JSON/网络失败不会留下 `lifecycle=started` 的信封记录；已产出决策后的记录不受此缺口影响，见 SR-019。
- 模块定义 lifecycle、gateway status 与 final review status 闭集，以及终态吸收的纯状态机；未接线意味着这些 started→terminal 原语与 recovery event 目前不能被当作生产恢复保证。`update_run_envelope_terminal` 的恢复行/事件在缺上下文时还会以空 workspace/account 占位，但因入口未接线暂不是当前主路径。

### 本批待后续核实

- Gateway 是否把 `validate_and_promote` 返回的每一种 risks 都 fail-closed，以及 autonomy protocol disabled 的真实灰度入口。
- Run log 当前一次性 insert 的所有 precheck、decision、review、enqueue 与异常分支，及 DuplicateKey/重试行为。
- 预算达到/越过上限时 Review、rewrite、knowledge tool loop 和 follow-up 分支的实际降级状态。
- threshold override 的发布写侧是否保证合理区间、单 current 与回滚一致性。

## B04B：Decision、Review 与确定性 Guards

覆盖文件：4 个，合计 6,673 行、311,648 字节。包括 `src/agent/decision.rs`、`review/mod.rs`、`review/gates.rs` 与 `guards.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。B04B 完成决策生成、评审与纯安全汇总层；Gateway 对这些结果的真实接线、revision fallback、状态落库与 outbox 分流仍留给 B04C。

### Decision prompt 与租户上下文（FACT）

- `decide_reply_with_promote` 按 prompt tier 切分恒注入、关系与业务三组上下文；Lean 仍单独注入 doNotDo/commitments，Full 才加载知识、产品、状态机、素材完整清单等业务槽位。DomainProfile、taxonomy、资产、知识、联系人画像和 reaction hint 都按 `contact.workspace_id`/account 读取。
- Reply 的三层 prompt（system/policy/task）与 published user Soul 却固定查询 `default_workspace_id`；Reviewer light/full system prompt 也固定默认 workspace。非默认联系人因而把本租户业务上下文与默认租户的控制 prompt 混合，见 SR-021。初始画像的两条 prompt loader 正确使用传入 workspace，不属于该缺口。
- `RawAgentDecision` 在 LLM 返回后立即 validate/promote；taxonomy alias/candidate 归一发生在 Reviewer 前。conversation mode 枚举由 active profile 覆盖 runtime 默认，避免非销售 profile 被销售四模式拒绝。
- Prompt 隔离对历史和最新入站文本剥离伪造标签/relay sentinel；合法 synthetic relay 才保留哨兵。可引用资产与禁语查询都固定 workspace/account，其中禁语不受 tier 限制且不设数量上限。

### Reviewer、双闸与最终汇总（FACT）

- `should_run_review` 只对 should_reply=true 且自报需审、高风险、知识必需、低状态置信度或 profile 声明 distrust 时调用 LLM；其它路径使用本地 review。高敏 profile 的本地兜底把 pressure risk 保守置到 block threshold。
- Reviewer 只看到候选回复事实面，不接收 Reply Agent 的九个自我推理字段；主/第二 Reviewer 可并行执行，第二路失败或 JSON 错误只告警并回落主 Reviewer。
- `classify_dual_gate` 将 hallucination/grounding 归为硬闸，将 human-like、pressure、emotional-value 和 boundary/privacy 归为可 single-shot revision 的软闸。`finalize_review_for_send` 再按协议违规、预算、verified product claim、hold 等顺序短路；结构性字段违规硬阻断，关键轮推理偏短改走 revision。
- verified product claim 硬门只在 Reviewer 自报 `requiresProductKnowledge=true` 时执行。漏报/缺字段时，效果与语气承诺词探针均只写 observe 事件，不阻断；测试明确锁定“保证按时回款、无 verified 背书仍 Approved”，见 SR-022。
- 状态迁移在提供 domain config 时对空状态机和 unknown target fail-closed；无 domain config（simulation/老路径）则 fail-open。operation-state action policy 缺失或 inactive 同样 fail-open，以兼容老部署；真实 Gateway 是否始终加载并执行 policy 待 B04C 核实。

### 本批待后续核实

- Gateway 是否对所有生产 Reply 路径使用 `decide_reply_with_promote` 而非丢弃 risks 的兼容入口，以及 finalize 是否在所有发送前必经。
- revision LLM 失败、超时与二审失败是否都错误共用“恢复发送原稿”的 fallback；需以 Gateway 控制流裁决，不能仅凭纯 helper 注释下结论。
- 管理发送、simulation、follow-up 与 tool-loop 是否复用相同 prompt workspace、review/finalize 和 state-action policy 约束。
- Reviewer 漏判 telemetry 是否有告警、统计阈值或自动升级机制；当前本批只证明写入 pending event，不证明有人消费。

## B04C：Gateway 主控制流、revision 与发送前落地

覆盖文件：1 个，合计 6,838 行、296,128 字节，即 `src/agent/gateway.rs`；已按连续区间读到 EOF，冻结 SHA-256 零漂移。B04C 完成生产 Reply/FollowUp、管理发送、review/finalize/revision、画像/记忆写入与 outbox 入队前控制流；Outbox/Dispatcher 自身的 claim、lease、发送重试和 delivery-finalize 留给下一子批。

### 生产 Gateway 与安全门（FACT）

- 入站与 FollowUp 共用 `run_user_operation_gateway`：precheck 后先知识路由，再按 Lean→Relational/Full 的充分性结果最多重生成一次；Raw decision 的 promote risks 会进入 `finalize_review_for_send`。预算超额时知识路由或 LLM Review 可降级，但最终 finalize 仍执行。
- 首轮 finalize 为 Approved 后，Gateway 再执行 operation-state action policy；revision 成功并通过二审后也复检该动作门。管理发送走 full Review + finalize，并额外要求 `review_passed`，因此没有 revision 通道时不会把软闸失败稿直接发出。
- 主链在 decision/review 之后先更新画像与记忆、写 decision/run 的 `outbox_enqueuing` 过程态，再逐段 enqueue 文本，随后追加素材/名片。分段入队不是事务：部分段失败时已入队段仍会发送，run 标记 partial failure。进程在过程态写入与 enqueue 之间崩溃时没有统一 Run Envelope 恢复；这进一步印证 SR-019，Outbox 侧是否另有补偿待后续全文核实。
- 产品声明漏判路径已由生产接线闭环：Reviewer 把 `requiresProductKnowledge` 漏报为 false 时，finalize 仅写 observe 事件并保持 Approved；Gateway 的 `outbox_eligible` 只检查 should_reply、非空正文和 approved 终态，不另做产品声明识别，故 SR-022 可达 outbox。

### Revision 回退与事件租户边界（FACT）

- soft gate 包含 human-like、pressure、emotional-value 与 boundary/privacy；后者低分含“可能泄露内部画像、AI 身份或幕后领导信息”的语义。revision LLM 错误、30 秒超时和改写稿二审失败三路都调用同一 fallback，把原稿恢复成 `revision_applied_approved`。之后无 `review_passed` 复检，直接满足 outbox 终态白名单，见 SR-023。
- `write_event_for_account` 被 Gateway、Webhook、任务、Planner 与多类 Worker 广泛复用，但签名没有 workspace 并固定写 default workspace；事件 API 又按当前 workspace 读取。非默认租户事件因此错账并可能向默认租户暴露，见 SR-024。
- 账号日发送软上限统计只按 account_id 聚合 Outbox，缺 workspace。账号主键实际是 `(workspace_id, account_id)`；复用 account_id 时会跨租户合并计数。当前仅告警、不拦截，定为 P2，见 SR-025。
- 真实文本发送仍通过只接收 account_id 的 MCP 公共入口；请示卡同样如此，属于既有 SR-011 的生产爆炸半径。MCP 成功后的消息/联系人落库失败被降级为日志/事件而不返回错误，以避免 Dispatcher 重试造成重复发送。

### 本批待后续核实

- Outbox/Dispatcher 是否能恢复 Gateway 留下的 `outbox_enqueuing` 过程态，以及文本多段、媒体和名片的聚合终态是否在崩溃/部分失败下收敛。
- MCP 发送 timeout、lease 过期与 post-hoc 成功核对能否覆盖“上游已送达、进程未标 sent”的重复发送窗口。
- FollowUp task 的 claim/reclaim 与 Gateway cancel/reschedule/outbox_enqueued 更新是否具备完整 CAS/lease 所有权约束。
- Simulation 与独立 tool-loop 路径是否绕过本 Gateway 的 finalize/state-action/outbox 约束，留给对应文件全文批次。

## B04D：Outbox 与 Dispatcher 可靠投递闭环

覆盖文件：2 个，合计 3,657 行、142,587 字节，包括 `src/agent/outbox.rs` 与 `src/agent/outbox_dispatcher.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。B04D 完成 enqueue 幂等、claim/lease、发送前二次安全门、重试/终止、崩溃与 timeout 后核对、Gateway 过程态恢复及 delivery-finalize 副作用闭环。

### 幂等、claim 与发送状态机（FACT）

- enqueue 校验文本/媒体/名片形态并把 max attempts 收敛到 1..=10；Mongo 单字段 unique `idempotency_key` 是最终原子门。DuplicateKey 会回读既有 Outbox 并返回 skip，而非再次发送。
- Dispatcher 用 `findOneAndUpdate` 把 pending 原子切到 in_flight，lease 为 180 秒，整条 send timeout 为 150 秒；领取按 `(created_at, _id)` FIFO。发送前会重新读取当前 workspace/account/contact，复核 managed、cooldown、用户 stop、30 分钟陈旧度和账号在线状态。
- 发送失败按指数退避回 pending，达到 max attempts 后进入 failed_terminal；账号离线和 pacing 只延后 next_retry，不消耗 attempt。run 级 outbox 聚合用 generation CAS，避免旧查询快照把新状态倒退。
- Gateway 留下超过 60 秒的 `outbox_enqueuing` 会按实际文本段数恢复为 enqueued、partial failure 或 failed；review/task 先幂等补偿，run log 最后 CAS 作为提交标记。分段 enqueue 本身仍不是事务：已有段可发送、缺失段不会自动补齐。

### 崩溃核对与投递后副作用（FACT）

- 过期 in-flight lease 会回到 pending 并标记 reclaimed；重发前文本优先经 MCP `chat_search_outbound` 核对真实已发记录，失败再查本地 MCP log；媒体按素材发送日志核对。命中则直接标 sent，不再次调用发送工具。连续 reclaim 超过 5 次转 failed_terminal 止损。
- 名片条目没有权威或本地 post-hoc 核对，reclaim/timeout 时固定视为未发送并重试；代码明确接受边缘场景下重复名片的代价。这不是文本/媒体“至多一次”保证，后续若提升名片可靠性要求应单独补发送回执锚。
- 全部文本段 sent 后，delivery finalizer 才提交承诺、创建 follow-up、清 principal awaiting 状态并把关联 task/review 推到 sent。review 短 lease、Outbox pending marker、commitment 去重和 task upsert 使副作用可在进程崩溃后重跑，而不会把已 sent Outbox 退回 pending。
- 实际 MCP 发送与 post-hoc 核对仍只有 account_id 上下文，继承 SR-011；Outbox 事件复制写入器固定 default workspace，已并入 SR-024；账号 pacing 只按 account_id 查 sent 历史，已并入 SR-025。

### 租户隔离缺口（FACT）

- 所有 Outbox 幂等原文都缺 workspace，且 unique 索引是全集合单字段约束；跨 workspace（部分形态也跨 account）相同输入会被当成同一消息，后入队租户被静默 skip，见 SR-026。
- Reaction 正确在 contact workspace 内认领 review，但 stop/cooldown 批量取消 API 不接 workspace、固定操作 default workspace。单 entry 二次门只看自己的 decision outcome，不能取消同联系人其它在途 decision，因此非默认租户 stop 后仍可能继续发送，并可能误取消默认租户同标识条目，见 SR-027。

### 本批待后续核实

- `tasks.rs` worker 对 FollowUp claim/reclaim、Gateway 返回错误与 outbox_enqueued/sent 终态的所有权及重试闭环。
- `reaction.rs` 其余分析、反馈与 trajectory 写入链，以及其 prompt/workspace/预算边界。
- `media_send.rs`、`referral.rs` 与 `send_ledger.rs` 的发送回执、跨租户查询和主动发送台账语义。

## B04E：Reaction 分析、反馈学习与轨迹写入

覆盖文件：1 个，合计 1,296 行、59,087 字节，即 `src/agent/reaction.rs`；已按连续区间读到 EOF，冻结 SHA-256 零漂移。B04E 完成上一轮已发送 decision 的 Reaction claim、LLM 分类、outcome 回填、Reviewer 误判信号、负例候选、intent trajectory 与 stop 取消接线。

### Reaction 主链与学习副作用（FACT）

- Webhook 去抖 runner 在生成本轮回复前旁路执行 Reaction：只认领同 workspace/account/contact 最新一条 `status=sent` 且 outcome pending/null 的 review。没有可认领 review 时不调用 LLM；Reaction 失败只写告警，不阻断本轮 Gateway。
- 每次调用建立独立 RunBudget；预算已超时直接降级 `user_replied_unclassified`。正常分析注入当前 contact 的 operating memory、active profile outcome polarity 与 trajectory dimensions，并剥离外部入站中的伪造 relay sentinel。
- outcome 显式字符串优先；否则 stopRequested、buyingSignal、objection 依次映射。正/负极随 active DomainProfile，未分类保持 censored，不被当负样本；stopRequested 是跨域固定红线。
- review 回填后，负向 outcome 可标 `approved_but_user_negative` 并生成 `draft + needs_review` 的 negative_example 候选；AI 不自动 verify。intent trajectory 按 workspace/account/contact 写入，维度值先过 taxonomy/dimension registry，Mongo `$push + $slice:-50` 保留最近 50 项。

### 租户与并发边界（FACT）

- Domain config、profile、memory、review claim、message count、dimension validation、contact trajectory 与负例实体均使用 contact workspace；但 Reaction system/task prompt 固定从 default workspace 加载。该缺口已并入 SR-021，且错误分类会向 outcome、trajectory 和负例学习扩散。
- stop/cooldown 结果接到 Outbox 批量取消时丢失 workspace，属于已定级的 SR-027。
- stale reaction claim 默认 60 秒即重置，但单次 LLM 默认可 45 秒、最多 5 次尝试并退避；claim 没有 owner/generation fencing，最终写回仅按 `_id`。多副本可让旧、新分析都提交并重复轨迹/负例/取消副作用，见 SR-028。
- negative_example 注释宣称按 `(workspace_id, source_review_id)` 幂等，实际 filter 只查 source_review_id，且是 count-then-insert、无本路径可见唯一约束；在正常 ObjectId 全局唯一前提下不会跨租户碰撞，但不能抵御 SR-028 的并发重入。

### 本批待后续核实

- `memory.rs` 对 Reaction/Gateway 并发写、版本推进、压缩与回滚的 CAS 语义。
- `knowledge_router.rs` 如何消费 negative_example、reaction hint 与 intent trajectory，以及候选完整性/verified 门。
- 多副本 Webhook 去抖不共享的更大爆炸半径已在 SR-012 与 SR-028 部分显现，后续结合部署和 Gateway 并发写统一裁决是否需要独立发现。

## B04F：Memory Card、候选整理与持久化闭环

覆盖文件：1 个，合计 3,485 行、152,047 字节，即 `src/agent/memory.rs`；已按连续区间读到 EOF，冻结 SHA-256 零漂移。B04F 完成 OperatingMemory 首建、memoryCard typed 合并、候选生成与 consolidation、标签/人格附属写回、运营偏好记忆及任务调度链。

### Memory Card 结构与并发控制（FACT）

- OperatingMemory 以 `(workspace_id, account_id, contact_wxid)` 唯一索引兜底并发首建；DuplicateKey 输家重读赢家文档。种子 card 从人工画像、manual/confirmed tags、承诺与关系状态构建，滚动 `memory_summary` 只进 recent episode，不进入权威 core facts。
- memoryCard 的 Gateway seed 与 consolidator 写入都使用 `(workspace, account, contact, memory_card_version)` OCC；同旧版本并发 writer 只能有一个命中。压缩对 core/recent/deprecated facts 和 profile memory dimensions 设 cap，并以 stable id、显式 discarded/deprecated 及同 dimension 最新值裁决冲突。
- 对话窗口按时间升序并做 prompt isolation；confirmed tags 和人格维度必须能把 LLM evidence turn 锚到真实消息，否则标签丢弃、人格 confidence 归零。manual tags 与这些 AI 派生字段物理分开。

### Consolidator、租户与恢复边界（FACT）

- consolidator 按 contact workspace 加载 domain config、profile、pending candidates、消息和 OperatingMemory，但 system/task prompt 固定从 default workspace 读取，已并入 SR-021。
- 每次 consolidation 自建 run_id 并传给 LLM 日志，却不创建 AgentRunLog；warnings 只尝试更新不存在的 run 行且忽略 matched_count。该审计缺口已并入 SR-019。
- memoryCard OCC 提交后，confirmed tags/personality、candidate consolidated、事件和 task sent 分布在不同集合且无 durable phase。崩溃可让同批候选重放并推进新版本；附属 Contact 写失败仍会消费候选，见 SR-029。
- memory consolidation task 调度采用 find-then-insert，现有索引不保证同 contact 活跃任务唯一；并发任务虽由 memoryCard OCC 防止同版本覆盖，但会放大 LLM 重跑与跨集合提交窗口。事件仍通过固定 default workspace 的公共 helper，属于 SR-024。
- operator memory 的读写 filter 正确包含 workspace/account/operator；但“同 kind/content 去重”也是先查后插且没有对应唯一索引，并发时可产生重复偏好条目。当前影响主要是 prompt 重复与存储噪声，作为本批次边界记录，不另立高优先级发现。

### 本批待后续核实

- `knowledge_router.rs` 与 `knowledge_tools.rs` 对 verified、negative_example、完整性、动态置信和 tool-loop 的真实召回/打开边界。
- `tasks.rs` 心跳、claim owner 与 handler 自写终态之间是否存在旧执行者覆盖新任务状态的 fencing 缺口。
- 管理端手工修改 OperatingMemory 是否使用 memory_card_version OCC，以及与后台 consolidation 的冲突呈现。

## B04G：生产 Knowledge Router 与渐进式 Knowledge Agent

覆盖文件：3 个，合计 3,945 行、168,715 字节，包括 `src/agent/knowledge_router.rs`、`src/agent/knowledge_agent.rs` 与 `src/agent/knowledge_agent/cache.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。B04G 完成生产私聊知识加载、Agent-first 多轮探索、catalog/open/relation/version 路径、引用收口、弱兜底、召回信号和答案缓存；知识工作台的 `knowledge_tools.rs + chat_tool_loop.rs` 留给 B04H。

### 生产召回与 verified 边界（FACT）

- Router 初始 runtime 只加载当前 workspace、共享/当前 account、active、verified 的 user_operations chunks；Knowledge Agent 的 catalog/open_document 也遵守这一可见域。最多 4 轮按相关度 catalog → open_document/open_chunk → follow relations → answer；cited 最终必须属于 opened，Router 再把 cited 与初始 verified corpus 求交。
- Agent 未给 cited 时，Router 在初始 verified corpus 内按同一 rank key回填最多 5 条弱证据，并显式把 coverage/risk 标为 weak/medium；探索 flag 只改变该 fallback 的抽样并记录 propensity，不扩大 corpus。Reply prompt 注入 selected chunks 全文，但会剔除 Knowledge Agent 的自然语言 reason、tool trace、自报 evidence excerpts 与 ranking 调试字段。
- open/version/relation 下钻没有继承 account/domain/status 可见域；同 workspace 的跨账号私有 chunk 可被 ObjectId 或 relation 打开。生产 Router 的最终 corpus 交集只能挡住直接 selected id，不能撤回已发送给 Knowledge LLM 的正文，也不保护独立 ask API，见 SR-030。

### 引用、关系与缓存边界（FACT）

- verified-only 在 DB 查询层真实执行；superseded 链最多 8 跳并防环，目标必须同 workspace+verified。过期/superseded 在 catalog 只降权不剔除，产品声明 gate 另行排除过期 chunk。
- contradiction 关系目标会被载入 opened 并标记 relationRole，禁止引用仅写在 prompt；服务端只检查 id 已 opened，伪造 quote/anchor 也不做内容锚定，truncated 兜底还引用全部 opened，见 SR-031。
- 答案缓存包含 workspace/account/query/max_rounds，但所谓 corpus signature 实际只签 top-30 的 id、dynamic confidence 与关系数量，不含 updated_at/正文/引用/关系内容。知识编辑后可复用 5 分钟旧答案，见 SR-032；provider 不入 key同时延伸 SR-020。
- recall miss/low-yield 会异步写 gap signal；miss 先确定性落原 query，再 best-effort LLM 生成追问，low-yield 可提 split proposal。该异步链不阻塞召回；运行日志仍经无 workspace 的公共 Agent LLM 入口，归属问题已在 SR-018。

### 本批待后续核实

- Knowledge 写侧 revision、verify、relation 与 corpus generation/invalidation 是否存在统一提交信号；B04G 已证明当前 answer cache 未消费此类信号。
- `knowledge_wiki` feedback/gap/structural proposal worker 是否会把 account 私有信号聚合成 workspace 共享修订。

## B04H：Knowledge Chat 工具派发与多轮收敛

覆盖文件：2 个，合计 2,208 行、87,828 字节，包括 `src/agent/knowledge_tools.rs` 与 `src/agent/chat_tool_loop.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。另定向读取 `src/routes/knowledge/chat.rs` 的 chat tool-loop 组装与下游消费区间作为可达性证据，但该大文件未在本批标记全文完成。专项单测 `agent::knowledge_tools::tests` 22/22、`agent::chat_tool_loop::tests` 5/5 通过。

### 工具能力、verified 与账号域（FACT）

- 内存三件套 `list_catalog/search/open_slice` 消费调用方预载的 KnowledgeRuntime；search 会对未核验正文 redact，open_slice 对未知 id 整批失败并限制 K，superseded 最多 8 跳且防环。chat 路由当前只预载 active documents 与 verified chunks，所以这三件套的正文完整性边界成立。
- 六个 chat-only 工具直接查 Mongo，并统一有白名单、RunBudget tool-call 计数和单次 5 秒 timeout；错误作为 JSON Value 回喂模型，不触达 outbox/MCP，也不直接修改知识。`propose_repair/audit_completeness` 只给诊断，`verify_anchor` 复用写侧模糊锚算法，AI 仍不能自动 verify。
- account 可见域没有贯穿工具 capability：dispatch 只传 workspace，chat 路由虽接收 account_id，却按 workspace 全量预载；direct DB 工具也只筛 workspace。`search_chunks/open_document/audit/propose/verify_anchor` 可读其它账号对象，`analyze_logs` 还能由模型自填其它 account 或省略后读取整个 workspace。该爆炸半径并入 SR-030。

### 循环预算与终态收敛（FACT）

- 循环最多 4 轮，每轮最多派发 6 个调用，连续 3 个错误或 `budget_exceeded` 强制停止；工具结果上下文 keep-tail 截到 8,000 字，trace 最多 32 条。final 轮携带多余 toolCalls 会被清空并记录 risk。
- 强制停止只修改 `ChatToolLoopOutcome.decision`；路由调用方却丢弃 outcome，直接回传最后一轮原始 JSON。若最后 raw 是 protocol 要求不含业务字段的 `tool_calling` 中间态，上层不验证 decisionPhase，最终会把默认“AI 未给出回复”、空 patch 的成功 turn 落库。见 SR-033。
- 30 秒“总硬超时”只是每轮开头的 elapsed 采样：单次 LLM 没有剩余 deadline，同轮最多 6 个工具各自可跑 5 秒。第四轮或强制停止后也不会再检查总耗时，所以该限制不是 whole-future deadline，慢请求可显著越界，亦见 SR-033。

### 本批待后续核实

- `routes/knowledge/chat.rs` 全文中的 session/account 归属、turn index、apply/update 写边界与后台 KnowledgeChatTask worker 将在知识路由大文件批次统一结算；本批只以定向区间证明工具循环可达性。
- `knowledge_wiki` 的 revision/verify/feedback/gap/structural worker 是否维持 account 可见域，以及其修改能否使 answer cache 失效。

## B04I：AgentTask Worker、管理重跑与租约收敛

覆盖文件：2 个，合计 1,107 行、42,311 字节，包括 `src/tasks.rs` 与 `src/routes/tasks.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。另定向读取 Gateway task 终态、Outbox 幂等、principal relay、task 索引与 m017 迁移区间作为可达性证据，不提前结算这些文件。专项单测 `tasks::tests` 6/6、Gateway segment idempotency 2/2 通过；`tests/worker_reclaim.rs` 虽名为集成回归，实际只插入并回读字段，没有调用私有 tick 或验证真实 recovery。

### Claim、心跳与管理入口（FACT）

- Worker 每 tick 先扫描 stale running，再读取最多 20 条 pending/retry；claim 用 `_id + 旧 status` CAS，写 running/claimed_at 并增加 attempt。正常 Worker handler 期间每 `timeout/2`（夹 5..60 秒）续租，错误按 60 秒起、指数退避加 ±20% jitter，最多由 task.max_attempts 控制。
- lease 只有时间没有 owner/generation。stale scanner 的读取与更新分离，更新条件仅 `_id + running`，不比较读到的 claimed_at；扫描后成功心跳不能阻止旧 scanner 回收。管理员 review-now 有 workspace + 可复核状态 CAS，但没有 heartbeat；cancel 有 workspace 过滤，却可直接覆盖任何状态且不通知执行中的 future。
- handler 和 Worker wrapper 的 task 提交大多只按 `_id`。FollowUp 没有 `should_abort_send`，取消或失去 lease 后仍可先创建 Outbox；不同重生成文案因 content hash 不同不会被 task-id 幂等键互斥。该客户触达级 fencing 缺口见 SR-034。

### Handler 终态与 relay 特例（FACT）

- 普通 FollowUp 由 Gateway 自写 cancelled/outbox_enqueued，Dispatcher 在真实送达后将关联 task 推进 sent；memory consolidation、outcome aggregation 与 initial profile 也由各自 handler 自写成功终态。Worker 的 Ok 分支只写事件，不统一收口状态。
- principal_decision_relay 正常路径把 task id 传入 Gateway；但“授权已过期”分支直接裸调 MCP 发中性收尾并 Ok 返回，既不走 Outbox 也不写 task 终态。任务保持 running，心跳停止后会被回收重跑并再次发送，见 SR-035。entry/decision/contact 缺失的早退也会留下 running，但没有直接发送影响。
- 现有 relay 过期测试未给 task `_id`、未插 tasks 集合，只证明一次调用会清 awaiting 并发一条话术，无法发现 Worker 后续回收重发。review-now 的测试覆盖 claim 冲突与 claimed_at，但没有长执行 heartbeat/fencing 测试。

### Outcome 聚合与租户键（FACT）

- scheduler 遍历全部账号并为 7d/30d 各插一条当日任务，handler 按 task.workspace/account 聚合消息、联系人、review 与 run log，再以含 workspace 的 metric id 幂等 upsert。
- 调度去重索引和 m017 历史清理都遗漏 workspace，只按 kind/account/content 判重；不同 workspace 复用 account_id 时会吞掉后者任务或迁移删除合法行，见 SR-036。

### 本批待后续核实

- escalation 模块全文批次需继续核对 relay 的裸 MCP 分支、short_code 全局唯一与 task/source_decision_id 关联是否还有租户或恢复缺口；本批只以定向区间证明 SR-035 可达。
- 阶段 7 各专用 Worker（import、knowledge task、catalog rebuild、cold contact、silence、evolution）是否重复采用无 fencing 的时间 lease，不能由通用 AgentTask 结论直接外推。

## B04J：决策请示、领导裁决与超时升级闭环

覆盖文件：6 个，合计 2,992 行、130,370 字节，包括 `src/agent/escalation/holding_reply.rs`、`labels.rs`、`ledger.rs`、`logic.rs`、`mod.rs` 与 `policy.rs`；均按连续区间读到 EOF，冻结 SHA-256 零漂移。另定向反查 Webhook 领导入口、Gateway 首推/relay/知识沉淀、AgentTask tick、escalation 索引、模型结构、admin resolve/reassign 与相关测试；这些反查文件未因本批定向读取重复结算全文覆盖。

### 创建、匹配与裁决（FACT）

- 两条创建路径都先检查策略/骚扰门和同 contact/category pending，再插台账并裸 MCP 推卡；partial unique 能兜住并发重复插入，但 `last_pushed_at_ms` 在真实发送前已登记。首次发送失败不会回滚或进入可重试投递状态，后续去重反而永久阻断重推，见 SR-038。
- Webhook 先按入站 workspace 判断 from_wxid 是否属于任一 current domain 的 decider chain，再将 workspace/account 交给 handler；handler 查询 pending 时却丢掉 account。一个业务号收到的领导回复可匹配同 workspace 其它账号的 pending，pending 去重键也缺 account，见 SR-040。
- 自然语言裁决经 LLM 生成 verdict/substance/constraints/window/exemption；代码只清洗 verdict。授权 verdict + 模型自报 knowledge 会把模型生成的 substance 自锚定后直接 Verify 为 workspace 共享 product_fact，而非锚定领导原文，见 SR-037。admin 结构化 resolve 虽跳过 interpret，但同样只清洗 verdict，后续应与微信入口共享完整字段校验。

### Relay、holding reply 与任务终态（FACT）

- 正常 relay 以 synthetic source marker 进入 Gateway；内部字段泄漏和授权外数字有出站守卫，客户级 exemption 在 relay 前写入，知识沉淀在 Gateway 成功后进行。awaiting 要等 Dispatcher 确认真实送达后才清除。
- 过期 relay 仍是裸 MCP 并漏写 task 终态，延伸并确认 SR-035；entry/decision/contact 缺失早退同样留下 running，但没有当次发送副作用。
- holding reply 的运行期数字守卫只对 ExpiredAuthorization + Some(substance) 生效；三处生产调用分别为 GateHold/None、ExpiredAuthorization/None、ChainTail/None，因此没有一个真实调用执行数字检查。它们不经过 Reply Reviewer；“不承诺数字/结果/时间”只靠 prompt，见 SR-042。

### 超时扫描、策略归属与并发（FACT）

- timeout scanner 在每个 AgentTask tick 的任务 claim 之前全库运行，没有每条 escalation claim/lease。next 卡与链尾客户安抚都先裸 MCP、后更新台账；多副本或发送后更新失败会重复副作用，发送失败的链尾分支仍 touch 又会抑制重试，见 SR-039。
- scanner 遍历所有 current `OperationDomainConfig`，每次却读取该 workspace 的全部 pending；台账没有 domain 字段。多域 workspace 下同一条会被其它域的 timeout、decider chain、quiet hours 和 cap 重复处理，见 SR-041。
- `reassign_escalation` 有 workspace+short_code+pending 过滤，但没有 expected principal/updated_at/generation；它可防跨 workspace IDOR，却不能为扫描时快照提供并发 fencing。单实例测试覆盖正常顺序、推送失败和链尾间隔，不覆盖多 scanner 竞争。

### 本批待后续核实

- `routes/principal_escalations.rs` 全文批次需核对 admin 请求字段闭集、授权范围呈现及列表分页；本批仅定向读取以证明下游共用裁决链。
- 专用 Outbox/通知模型是否可统一承接领导卡与链尾安抚，将在后续通知/管理路由与前端批次结合产品交互评估；当前生产事实是两者均裸 MCP。

## B04K：动态画像维度、Taxonomy 与贝叶斯旁路

覆盖文件：8 个，合计 5,294 行、255,396 字节，包括 `src/agent/bayesian_slots.rs`、`decision_taxonomy.rs`、`dimension_registry.rs`、`domain.rs`、`domain_profile.rs`、`domain_signals.rs`、`tag_evidence.rs` 与 `taxonomy.rs`；均按连续区间读到 EOF，并以冻结台账 SHA-256 核对。另定向反查 Decision/Gateway 双阶段接线、DomainProfile 管理路由与索引、taxonomy 管理/审批、画像 dotted-path 写入和相关测试；这些反查文件不因本批定向读取重复结算全文覆盖。8 组对应 `--lib` 专项测试共 155/155 通过。

### DomainProfile 与开放维度写入（FACT）

- active profile 缓存按 workspace 隔离并有 30 秒 TTL/显式失效；无 active 或 DB 错误回落内置 DEFAULT。运行时查询只认 `is_active=true AND current_version=true`，但多行异常态用无排序游标“最后一行赢”。profile 发布、回滚、激活又以多次非事务更新维护 current/active，索引不唯一；并发或中间失败会让运行时随机选错或静默回落 DEFAULT，见 SR-043。
- 决策维度由 active profile 动态声明，typed 销售两维与开放 `domainSignals` 双向同步；落库前会剔除 profile 未声明键，并按 taxonomy 做 value 归一/丢弃。这一白名单只约束“是否声明”，不验证 kind 可安全拼进 Mongo dotted path。
- `ProfileDimension.kind` 可含保留名、点号或 `$`，最终直接形成 `domain_attributes.{kind}`。配置错误可覆盖 value tier、awaiting/principal exemption 等系统状态，或制造嵌套/非法更新路径，见 SR-044。

### Taxonomy 严格字典与候选通道（FACT）

- TaxonomyCache 以 `(workspace,scope,kind)` 分组，account scope 优先于 global；canonical active、canonical deprecated、active alias、deprecated alias 四路归一。候选审批在事务内 claim pending，并把正式条目与候选状态一起提交；缓存写后失效，候选不阻塞回复。
- 同一未知值却在 Decision 解析阶段 detached upsert 一次，Gateway 最终阶段又同步 upsert 一次。pending 每调用一次就增加 occurrences；首次插入竞态输家只吞 E11000、不补增量，故单轮可能计 1 或 2，见 SR-045。
- 正式字典只约束 canonical/version 唯一，没有 alias claim；create、patch 与 candidate approve 均不做跨条目冲突检查。缓存对重复 alias 取无序首项，alias 与 canonical 冲突又由 canonical 固定抢占，见 SR-046。

### 证据锚与贝叶斯纯观测旁路（FACT）

- `tag_evidence` 把 LLM 的窗口序位 fail-closed 映射到真实 message id；越界/负数被丢弃。Gateway 只把 Inbound 锚计为贝叶斯强证据，LLM confidence 不参与占槽强证据判定。
- 贝叶斯槽位最多锁 6 个、history 封顶 100，当前只写 `Contact.bayesian_signals` 供趋势展示，不驱动决策、筛选、状态机或发送。并发写明确接受 last-write-wins。
- “hit=跨轮命中”的实现按 observation 数组项逐个 push，调用前只截 6 项、不按 dimension 去重。同一轮三条同名维度可用同一 Inbound 锚累计 hits/strong 并达到默认 3/2 占槽阈值，见 SR-047；因当前纯观测边界，严重度为 P2。

### 本批待后续核实

- DomainProfile 管理前端是否允许直接构造任意 kind、是否向运营呈现 current/active 异常，以及生成向导的 suggested taxonomy 写入在并发下如何收敛，留给对应路由/前端全文批次。
- Planner、operation view 与其它消费者对 taxonomy 冲突、deprecated 值和 profile 维度变化的展示/降级语义，将在各自全文批次继续映射。

## B04L：Agent 剩余模块、影子评测与发送台账

覆盖文件：9 个，合计 3,525 行、156,087 字节，包括 `src/agent/consolidation_window.rs`、`entitlements.rs`、`multimodal.rs`、`prompt_isolation.rs`、`prompt_shadow.rs`、`quiet_hours.rs`、`referral.rs`、`send_ledger.rs` 与 `simulation.rs`；均按冻结提交连续读取到 EOF，九个 blob 与当前 HEAD 相同，SHA-256 与冻结台账一致。另定向反查 Gateway/Decision/Review、Dispatcher、Evolution replay/significance/release、消息与台账索引、Simulation/Send Ledger 路由及相关测试；这些反查文件不因本批定向读取重复结算全文覆盖。

### Shadow、Simulation 与 Prompt replay（FACT）

- Simulation 和 Prompt Shadow 都复用真实 `load_or_create_operating_memory`、Knowledge Agent 与 Reply/Review 链，但没有只读运行模式。首次演练会创建生产 operating memory；operator memory 命中会续期；知识召回 miss/low-yield 会异步写 gap signal、生成追问并可能建 structural proposal。现有“零副作用”测试只比较 Outbox 与 outbound message，漏掉这些集合，见 SR-048。
- Simulation 只用 `review_passed` 把回复标为 `would_send`，没有执行生产 `finalize_review_for_send`、state-action policy 和 revision 二审；路由却宣称这是 prod 同源终态。Prompt Shadow 同样只跑 Decide + Review 并比较分数，不跑生产终态硬门，见 SR-048。
- Prompt replay 的 source message probe 与实际加载都只按裸 `message_id`，不消费消息表唯一键里的 workspace/account；合法跨租户同 id 时会任取消息。新侧还用当前 contact、memory、knowledge、profile/playbook、阈值、provider 与 active prompts 对比历史 run 分数，并非只改变候选片段。grader 只要求一条 completed 即进入 `eligible_for_release`，见 SR-049。

### 交易事实与产品硬门（FACT）

- Entitlement 只消费 `staff_confirmed/payment_verified`，按 product_id 聚合正向成交与 reversal，快照售后期优先于活产品，续费取最晚到期；交易事实段受 active profile 开关，空产品/非交易域保持空注入。这些纯投影边界有较完整单测。
- 产品目录以分转元并把 id、名称、价格、币种、SKU 注入 prompt；但发送前背书只检查 LLM 自报的 `quoted_product_ids` 是否任一命中 active id，不把最终回复中的价格或产品说法与目录逐项比对。任一真实 id 可为错误价格/串价提供目录背书，见 SR-051。

### 发送台账、名片与多模态（FACT）

- Send Ledger 行保存 account_id，但近期历史、响应消息、当前 contact、索引和 contact history API 多处只按 workspace+wxid；同 workspace 多账号复用 wxid 时会串历史、响应和阶段。三条送达确认分支又对同一 Outbox 普通 insert，模型没有 outbox_id/唯一约束，崩溃重放可重复计数，见 SR-050。
- 名片候选与发送前均校验 workspace、account scope、enabled 与 approved；MCP 成功后的本地消息/已引荐状态失败会 fail-soft，避免因后置写失败而重发。可惜 Dispatcher 的 post-hoc verifier 对名片固定 `Ok(false)`，远端成功后 timeout/reclaim 必定再次物理发送，见 SR-052。
- 入站多模态下载当前明确打桩 `Ok(None)`；所有已知非文本类型走确定性过渡话术并经 Outbox，不把空串/XML送入主决策。图片 vision 封装已存在但在下载接通前不可达，因此本批不把“未接通”误报为实现回归。
- Quiet Hours 使用固定时区偏移、确定性 jitter 与 start-inclusive/end-exclusive 纯函数；profile/contact/global 三级开关已接 Gateway/Webhook。Consolidation window 从最新消息回溯，按字符预算和条数双上限返回正序窗口。Prompt isolation 会剥已知伪造边界与客户 relay sentinel，但它是提示隔离而非完备安全边界。

### 本批待后续核实

- Evolution 全文批次仍需核对 experiment/proposal 的 workspace/account 授权、cohort 构造、自动发布与 rollback；本批只证明 Prompt Shadow 证据可进入 eligible/release 链。
- Send Ledger 路由与前端的账号选择语义、历史数据去重迁移，以及名片 MCP 是否提供可查询 delivery id，需要在对应路由、MCP server 契约与前端批次闭合。
- 多模态真实下载、文件内容检测、ASR、共享持久卷与媒体生命周期仍是未实现/部署契约，不由当前打桩模块的 fail-soft 行为代替。

## B05A：管理/业务小路由、Simulation API 与 Soul/请示提交边界

覆盖文件：15 个，合计 2,340 行、84,885 字节，包括 `src/routes/management_prompt_edit.rs`、`health.rs`、`conversations.rs`、`events.rs`、`behavior_signal_metrics.rs`、`outcome_metrics.rs`、`operation_view.rs`、`souls.rs`、`assets.rs`、`principal_escalations.rs`、`reviews.rs`、`send_ledger.rs`、`simulations.rs`、`referral_cards.rs` 与 `products.rs`；均按冻结提交连续读取到 EOF，SHA-256 与冻结台账一致。另定向反查 Soul 运行时读取/启动对齐/索引、领导裁决两入口与 relay task 索引、账户校验 helper 和现有集成测试；这些反查不重复结算全文覆盖。

### 只读路由、账号与投影视图（FACT）

- 会话历史先以 workspace-scoped contact id 反查联系人，再以 workspace/account/wxid 三键取消息；事件、Outcome Metric 与 Review 列表均至少固定 workspace/account。Review 详情按 `_id+workspace`，但关联 run log 只按全局 run_id；run_id 当前有 unique 索引，故不形成独立租户绕过。
- Behavior Signal Metric、产品目录、Operation View 与健康检查均为 workspace 只读或公开探活；日期/limit 有钳制。Operation View 输出 active profile 维度和 global taxonomy，但传空 account scope，因而不展示 account 私有字典——这是当前 API 产品语义，后续以前端消费核对是否符合预期。
- Content Asset/Referral Card 创建允许可选 account_id，却未调用 `validate_account`；这可制造指向不存在/其它 workspace 同名账号的孤儿配置，但读取和 Agent 消费仍带当前 workspace，当前影响偏配置完整性，未单列高严重度 finding。
- Send Ledger API 只按 workspace/wxid 与 workspace 聚合，确认并延伸 SR-050；Simulation API 正确校验 account 属于 workspace 且 contact 属于 account，但底层影子副作用、终态缺失与 `no_reply` 假通过仍见 SR-048。

### Soul 版本生命周期（FACT）

- Soul 列表会先确保 prompt pack，但非空 prompt 库的对齐只处理 PromptTemplate，不补缺失 Soul。运行时只查默认 workspace 最新 published Soul，查无时静默使用短内置人格。
- 创建用“读最大 version + 1”且无版本唯一索引；更新可原地改任意状态行；发布先物理删除同 kind 其它全部版本，再单独把目标置 published。中间失败、并发发布或误改 published 会立即改变人格、删掉历史或长期降级，见 SR-053。
- workspace 过滤阻止跨租户按 id 发布/更新，现有 isolation 测试只锁这一点，没有覆盖版本不变量、事务或失败恢复。

### 产品/名片与领导裁决提交（FACT）

- Product CRUD 全部按 current workspace，金额要求非负最小货币单位、币种要求三位大写；归档不物理删除。它保障目录数据形态，但无法弥补 Reply 对目录声明只做 id 存在性自证（SR-051）。
- Referral Card 创建固定 draft+disabled，审核状态闭集，目标阶段经 taxonomy 归一；toggle 本身不强制 approved，但 Agent 候选/发送前仍双重要求 enabled+approved，因此未形成准入绕过。审核/启停/删除事件继承公共事件写入器的默认 workspace 问题（SR-024）。
- Admin resolve 与微信领导回复都先把 escalation 从 pending CAS 成 resolved，再独立 insert relay task。两步无事务、无 relay unique intent、无 reconciliation；第二步失败后重试只看到 resolved，不会补任务，见 SR-054。Admin 对 exemption/window 的闭集缺失延伸 SR-037，但根因仍是裁决字段信任边界，不另拆。

### 本批待后续核实

- `management.rs` 全文需核对工具代理是否可绕过这些 handler 的字段校验、是否正确注入当前 workspace/account，以及 dry-run/execute 审计闭环。
- Assets、Products、Referral Cards 与 Send Ledger 的前端 account 选择器和错误展示留给前端全量批次；本批只确认后端 wire/query 契约。
- Soul 的 prompt guard、管理端编辑体验与发布回滚应结合 PromptTemplate/Management 全文批次统一设计；当前事实是 Soul 不复用 PromptTemplate 的事务/version guard。

## B05B：Prompt/Ops 版本、Taxonomy 审核与 DomainSchema 生命周期

覆盖文件：8 个，合计 4,376 行、169,741 字节，包括 `src/routes/prompt_templates.rs`、`admin_ops_versions.rs`、`admin_state_policies.rs`、`admin_taxonomies.rs`、`admin_taxonomy_candidates.rs`、`admin_relationship_suggestions.rs`、`admin_suspected_deals.rs` 与 `domain_schemas.rs`；均按冻结提交连续读取到 EOF，blob 与冻结台账一致。另定向反查 Prompt 运行时 loader/Evolution release 与 rollback、ops/prompt/domain schema 索引、Gateway 建议写入、成交落库 helper、Management 工具代理及现有集成测试；这些反查不重复结算全文覆盖。

### Prompt 与 Ops 版本生命周期（FACT）

- 手动 Prompt create 写 `current_version=false`，publish 正常完成却只改 `status=active`，并先物删所有非 evolution 兄弟。生产 loader 按 status 读，Evolution/启动对齐按 current 读，同一 key 因而产生两套“当前版本”；失败还会删掉旧 current 与历史，见 SR-055。
- `admin_ops_versions` 的三表 publish/rollout/rollback 都用多次独立写切 current；`current_version=true` partial index 只是普通索引。文件自身已准确记录不同状态机并发 publish 可交错成持久 0-current；state policy/taxonomy 的同构切换也允许零/多 current，补强 SR-008。
- State Policy 列表默认只返 current，Taxonomy CRUD 正确限制更新/软删到 current 行并写后失效缓存；这些正常路径不消除 current 标志异常态，也不会检测重复 current。

### Taxonomy 与审核状态机（FACT）

- Taxonomy candidate approve 使用 Mongo transaction claim `pending→approving→approved`，新 canonical 的字典写与候选终态同事务提交；现有 replica-set 集成测试覆盖插入失败回滚。若目标 canonical 已存在，代码虽构造含 raw 的 aliases，却跳过所有字典写并仍提交 approved，形成永久无法归一的假成功，见 SR-061。
- Relationship suggestion approve 不使用事务或状态 CAS：先写 contact 关系类型，再无条件把 suggestion 改 approved。中间失败、approve/reject 竞态或建议值刷新可让审核记录与生产画像相反，见 SR-059。
- Relationship suggestion 对终态也使用 `(workspace,contact)` 全量 unique，而 Gateway 只 upsert pending；首次 approve/reject 后后续关系变化必撞 E11000 并被 fail-soft 吞掉，见 SR-060。
- Taxonomy、relationship、suspected deal 三组 approve/reject 请求都接受客户端 `reviewedBy`。疑似成交还把它写进正式 OutcomeEvent.marked_by，实际认证主体无法从审计中恢复，见 SR-058。
- Suspected deal 为避免 append-only 双计采用 CAS-first，但 CAS 后才校验 contact、金额、币种与产品并落成交。任何后续失败都会留下不可重试 approved；成交 append 后审计失败又会返回错误，见 SR-057。

### DomainSchema 生产约束（FACT）

- 同 schema_id 支持多版本，但 update/delete/activate 路由只接 schema_id 并总选最新版本。active v1 + draft v2 时，DELETE 检查 v2 非 active 后会 delete_many 整条血缘，确定性删除生产约束；PUT 也可能原地改 active，见 SR-056。
- activate 先 demote 全 workspace active，再 promote 目标，两步无事务、CAS 或单 active unique；失败留下零 active，并发可留下多 active。chunk 写侧 `find_one(is_active=true)` 无排序：零行时约束静默 no-op，多行时任选 required/enum/alias 规则，见 SR-056。

### 本批测试与后续核实

- 现有 Prompt 测试只验证 evolution 行保留与目标 status=active，不断言 current/previous/history；DomainSchema 测试只模拟顺序成功切换；suspected deal 只测正常成功；relationship suggestion 无失败/竞态集成测试；taxonomy transaction 测试只覆盖新 canonical。上述缺口均未被当前测试锁住。
- Management 工具代理复用 relationship/taxonomy inner，并向 LLM 宣告 reviewedBy 可选，确认 SR-058 不仅存在于 REST；Management 全文的真实 admin 委托、确认门和审计归属仍在后续批次统一核对。

## B05C：Management 命令、Outbox 管理与统一待审箱

覆盖文件：3 个，合计 4,397 行、189,843 字节，包括 `src/routes/management.rs`、`admin_outbox.rs` 与 `ask_human_inbox.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，并与冻结台账 SHA-256 一致。另定向反查 Management prompts、MCP 凭证与日志、Gateway/Outbox Dispatcher、成交 handler/校验/投影、统一收件箱前端及相关测试；这些反查不重复结算全文覆盖。

### Management 工具代理、确认门与身份（FACT）

- session/message 入口会按 current workspace 校验 account 与 session，产品工具的 contact 解析也收窄 workspace/account；但 tools/list 和执行期 MCP 调用只传 account_id，继承 SR-011 的默认 workspace 凭证与日志归属问题。
- 动态 MCP 目录中的 advertised 工具可由兜底分支直接透传。裸 `message_send_text` 被显式列为 Low，prompt 还把它作为发送候选，因此可绕过 typed `wechatagent.send_contact_message` 的 Gateway、Review、Outbox、二次安全门和可靠投递，见 SR-062。
- 静态 `ToolRisk::Dangerous` 覆盖客户发送、provider 激活、Prompt/状态机与生产 rollout/rollback，但调用方固定把 dangerous 开关传 false；除 irreversible、verify、campaign 与未分类工具外，是否确认退化为 LLM 自报字段，见 SR-063。
- Management 复用 REST handler 时统一合成空 user_id + `management-agent` username，命令/工具模型又没有 actor 字段；真实发起/确认 admin 无法从业务审计恢复，作为 SR-058 的新增证据。

### 命令提交、成交与 Outbox 取消（FACT）

- command 与每个 tool call 都是先写 running、执行任意副作用、再普通 update 终态；confirm 先把 pending 原子改 running，再拉目录和执行。没有 lease、attempt、resume cursor、稳定幂等 intent 或 reconciliation，任一中间错误/崩溃都会留下不可安全恢复的状态，见 SR-064。
- `write_deal_events` 是 Low 且无需确认，直接复用 admin 成交 handler；verification 缺省被解释成 staff_confirmed，并进入 entitlement/交易事实投影。具体高可信事实由 LLM 填参而非人确认，见 SR-065。
- admin outbox 路由正确以 `_id+workspace+status∈{pending,in_flight}` 原子取消，但 in-flight worker 没有 cancellation token/generation。若取消发生在二次门之后，worker 仍发送；sent CAS 不命中后代码仍写 sent 事件并 finalize，见 SR-066。

### 统一待审箱与故障可见性（FACT）

- inbox 列表按 source 独立降级并返回 `errors[]`，八类 collector 均带 workspace；但已有的 suspected deal pending 审核队列未接入 collector、filter、summary 或前端 source，见 SR-067。
- summary 对八个 count 查询逐一 `unwrap_or(0)`，数据库错误与真实零待办完全同形，且没有 errors/stale/log 标记，见 SR-068。

### 本批测试与后续核实

- 现有 Management 测试锁定 Dangerous 默认放行、工具状态闭集和 confirm filter，却没有发送旁路、崩溃恢复、真实 actor 或成交确认边界测试；dry-run integration 主要手工插行，不驱动真实 handler。
- Outbox 测试覆盖发送前取消、陈旧取消和 lease reclaim，不覆盖“worker 已越过二次门后并发取消”；统一 inbox 测试沿用固定八源契约，因而不会暴露 suspected deal 缺失或 summary 故障伪零。
- Management 其余大量工具包装的字段/作用域正确性已在本文件全文阅读中抽样并结合对应 handler 扫描；后续相应业务路由全文批次仍需逐工具闭合副作用与权限语义，不能用本批三文件替代。

## B05D：Operation Domain、DomainProfile 与 Playbook 运行配置闭环

覆盖文件：3 个，合计 2,532 行；Git 冻结 blob 合计 107,793 字节，台账 Windows/CRLF 内容口径合计 110,325 字节（差额为每行一个 CR）。包括 `src/routes/domains.rs`、`domain_profiles.rs` 与 `playbooks.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `692ee9e653120fdcc9e90590a79d4cea3a351df9`、`a33d26c5657ff546cfd13e94e93c633eb5169336`、`191eb998fa8c5b65c9fa75383677e9164cadd0bd`，台账 SHA-256 保持不变。另定向反查 Playbook/DomainProfile 运行时 loader、Ops 索引与发布 helper、联系人绑定、Management 工具、前端 store/确认卡及相关测试；这些反查不重复结算全文覆盖。

### Playbook 契约与生产生命周期（FACT）

- 正式前端生成发送 `{accountId,prompt}`，后端要求 `{accountId,description}`；优化发送 `{prompt}`，后端要求 `{instruction}`。生成响应又是 `{id}` 对前端预期 `{item}`，两条 UI 路径确定性不可用，见 SR-069。
- generate 在账号无 Playbook 时把 AI 结果直接设为 default；optimize/update 原地覆盖生产文档。运行时只按同一 `_id` 或 default 读取，联系人保存的 `playbook_version` 不参与冻结加载。Management 所称“草稿、不放量”没有状态模型支持，见 SR-070。
- default 由“先清空再设目标”或“先查不存在再插入”的无事务流程维护，索引不唯一；失败可产生零 default，并发可产生多 default，见 SR-071。

### DomainProfile 发布、确认与激活（FACT）

- publish/rollout/activate 延续多行 current/active 布尔指针，版本号也以 max+1 计算；并发、失败与无序 runtime 读取继续补强 SR-043。
- activate 先切画像，再 best-effort 发布生成状态机、派生 policy 和逐联系人迁移；后续任何失败只告警，接口仍返回成功。人格/阈值、状态机、保护 policy 与存量联系人可处于不同发布代际，见 SR-072。
- 高风险 publish 只克隆可继续编辑的旁路稿；rollout 仅凭 profile id，不验证冻结 hash/version、确认 actor、TTL 或一次性 token。第一次确认后可替换内容，也可绕过 publish 直接 rollout，见 SR-073。

### Operation Domain 直编与重置（FACT）

- domain/runtime 与 state-machine 更新原地修改 current；policy reconcile 失败仅告警且仍返回成功，update 也不检查 matched_count。配置提交与保护策略仍不是同一原子 bundle。
- reset 先物理删除该 domain 全部版本，再独立插入默认；无论成功与否都会丢失历史，插入失败则留下零配置。正式前端直接暴露该操作，见 SR-074。

### 本批测试与后续核实

- Playbook 前端测试以手写 mock 重复错误契约，没有驱动真实 Rust DTO/响应；后端测试主要覆盖顺序 CRUD，不覆盖生成/优化端到端契约、并发 default 或中间写失败。
- DomainProfile 测试覆盖顺序发布/激活与部分迁移 helper，但没有冻结确认内容、直接 rollout 绕过、跨集合故障注入或 policy fail-closed；Operation Domain 测试也没有 delete 后 insert 失败与历史可恢复性。
- 本批证明三个配置面的运行指针、确认与发布协议存在共同架构缺口：配置版本不是不可变事实，生效指针不是单一原子对象，跨集合派生产物也没有 durable rollout intent。后续配置类路由应继续按这一统一原则审查，而非逐接口局部打补丁。

## B05E：Campaign 活动提交与自治监控读模型

覆盖文件：2 个，合计 1,726 行；Git 冻结 blob 合计 68,777 字节，台账 Windows/CRLF 内容口径合计 70,480 字节。包括 `src/routes/campaigns.rs` 与 `outcomes_autonomy.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `5e8377c62c824f35013fb1e0db7ea1765d0bc7aa`、`478f53631a841da9920abbd8531dfe145c569cd0`，台账 SHA-256 保持不变。另定向反查 Campaign/Task 模型与索引、Task Worker claim、Management 工具与 dry-run、活动前端/store、自治前端及对应集成测试；这些反查不重复结算全文覆盖。

### Campaign 圈选、确认与扇出（FACT）

- 两阶段圈选先以 workspace/account/managed、阶段和可信正向成交做 Mongo 粗筛，再用 entitlement 投影精筛净持有、售后与价值层；候选超过配置上限会拒绝而非静默截断，dispatch 会重新圈选。这部分正常路径与边界测试较完整。
- preview 实际无条件写 `status=previewed` 与 `targetCount`，不校验旧状态；completed 活动可借此重开。Management 却把其归为 Readonly，dry-run 会真实创建并修改活动，见 SR-075。
- dispatch 对每个联系人顺序写 CampaignSend、可立即 claim 的 AgentTask、taskId，再独立写 campaign completed；补偿只覆盖返回错误且自身 best-effort，没有 crash recovery、generation 或 reconciler，并发 dispatch 还会各自覆盖局部计数，见 SR-076。
- 正式前端首次预览后复用 draft id，但任何后续表单编辑都没有 update API；再次 preview 与最终 dispatch 仍消费第一次保存的条件和意图，见 SR-077。
- CampaignSend 唯一键能阻止同一 campaign/contact 的正常重复建位，但不能替代跨 CampaignSend、Task 与 Campaign 的提交协议；报表按 taskId 关联最新 run，关联丢失时只能永久显示 pending/not_yet_run。

### 自治指标与读模型（FACT）

- 指标接口按 workspace/account/horizon 统计升级后 run、revision、AI hold、taxonomy、自治模式、Outbox 与 Planner；零分母返回 null，历史 legacy 状态独立计数。revisions 返回最近最多 200 条改写及联系人显示名。
- 同一响应由 17 次串行 count 加一次 aggregation 拼接，没有统一 cutoff/snapshot；活跃写入可使后算分子超过先算分母或分布互相矛盾，见 SR-078。
- 真实 filter 以 workspace+account 开头，但注释依赖的 run 索引以 account 开头且缺 workspace；Outbox horizon、revisionApplied 排序也缺对应形状。指标串行查询，revisions 又逐行查 contact，形成 N+1，见 SR-079。

### 本批测试与后续核实

- Campaign ignored 集成测试覆盖零命中、workspace 隔离、正常扇出、task insert 返回错误后的补偿、completed 直接 dispatch 拒绝和受众上限；未覆盖 completed→preview→dispatch、dry-run 写入、进程退出、补偿失败、worker 抢占或双 dispatch。
- 自治集成测试覆盖静态小数据集数值、legacy 排除和 Planner 聚合，未使用 snapshot 并发写、Mongo explain、查询次数或 100k/多 workspace 延迟门。
- 本批显示“写模型提交”和“读模型观测”必须分别有明确协议：前者以 durable intent/幂等 item 收敛真实副作用，后者以固定 asOf/单次聚合提供可复核快照；UI 状态标签不能替代其中任何一个。

## B05F：联系人纳管、画像任务与运营池读模型

覆盖文件：1 个，共 2,087 行；Git 冻结 blob 87,568 字节，台账 Windows/CRLF 内容口径 89,609 字节。`src/routes/contacts.rs` 已按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 为 `85cbdbdbd6fd4d2aa278b288416e5f4b7d8453c3`，台账 SHA-256 `0fe9d8959c32ddbaa1b61833f18429ed2698cd1f2486c7d682e1175227aa2fad` 保持不变。另定向反查 shared contact/account/playbook helper、Task Worker 与索引、Gateway/Outbox 的 managed 门、联系人前端和批量启用测试；这些反查不重复结算全文覆盖。

### 联系人导入、账号归属与画像写入（FACT）

- 联系人绝大多数读写都先按 `_id+workspace` 取实体，搜索、导入、roster 和批量启用也通过 workspace+account 校验；但单联系人 enable 的账号查询只带 account_id，跨 workspace 同名账号可替当前租户错误通过并提供错误 self wxid，见 SR-080。
- REST、deprecated search-import 和 Management import 共用的 upsert helper 会把候选缺失的 nickname/remark/alias 作为 null 无条件 `$set`，重复导入会抹掉已有身份；批量启用相邻路径已经按字段存在性写入，见 SR-081。
- 手工画像字段会经 taxonomy alias/canonical 校验；manual tags 有数量/长度上限，operation profile 对空 profileAttributes 保留旧值。成交事件仍复用此前已审查的 append helper与 SR-058/SR-065 身份边界，本批不重复编号。

### 纳管提交与异步画像竞态（FACT）

- 批量启用先把 contact 切 managed，再独立创建可 claim 的 initial_profile task。task insert 失败或两步间崩溃后，重试因 already_managed 而跳过入队；并发请求又可创建重复任务，因为该 kind 没有 active unique，见 SR-082。
- initial_profile worker 只在耗时 LLM 前检查一次 managed，最终画像 helper 按裸 `_id` 无条件回写并再次设置 managed。管理员在生成期间 disable/hide 后，晚到任务可把联系人复活；hidden 行更因列表过滤而难以发现，见 SR-083。通用 Task lease/终态 fencing 继续归 SR-034，本项记录联系人授权 generation 缺失造成的独立业务后果。
- disable/hide 后的发送链仍有 Dispatcher 二次 managed 门；已经越过门的发送取消问题归 SR-066。本批不把所有在途副作用重复归到画像竞态。

### 运营池读模型与测试缺口（FACT）

- list_contacts 正确按 workspace/account/hidden 筛选并从 roster snapshot 批量补空身份，但随后为最多 500 个联系人串行查询最新入站消息。正式前端固定请求 limit=500，形成最多 501 次 Mongo 往返，见 SR-084。
- 现有批量启用测试覆盖正常入队、顺序重入幂等、老客户状态保留，以及任务启动时已 unmanaged/contact gone 的早退；没有 task insert 故障、双请求、managed 后崩溃，或“worker 检查后再 disable/hide”的 barrier 测试。
- 联系人纳管需要一个独立于展示状态的 durable enrollment generation：contact desired state、画像 item 与 task claim 必须绑定同代 CAS；列表则应消费联系人上冗余的最近入站预览或单次批量聚合，而不是把逐行查询藏在序列化循环中。

## B05G：Evolution 管理面、版本发布与回滚闭环

覆盖文件：1 个，共 1,255 行；Git 冻结 blob 52,926 字节，台账 Windows/CRLF 内容口径 54,182 字节。`src/routes/evolution.rs` 已按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 为 `dac5ca64de0a7e1ecd55c8b748bb09ef6eac64c8`，台账 SHA-256 `598afdff6502d5d5493f697ccb0ee7c795680c4efbf26b6b15a14fc8e9791216` 保持不变。另定向反查 Evolution release/rollback、cohort/runtime flag、Prompt Shadow、significance、post-release worker、索引、正式前端及相关集成测试；这些反查不重复结算全文覆盖。

### 管理面作用域、灰度开关与审计身份（FACT）

- proposal detail/release/rollback 的第一跳均按 `_id+admin.current_workspace` 限定，release/rollback 也把 `admin.username` 传给内核；experiment id 又有全局唯一索引，因此详情中的二级 experiment/proposal/replay 关联目前不会形成可达 IDOR。
- runtime flag 的缺失、关闭、0% 或 Mongo 读取失败均由 cohort/runtime helper 明确解释为不纳入演化，灰度门方向 fail-closed；release/rollback 还要求精确确认串，Prompt release 复用字面禁词、锚点与 LLM 语义三闸。
- runtime-flag PUT 却允许请求体 `updatedBy` 覆盖真实 admin，正式 UI 不传时还回落固定 `admin`；该审计主体伪造与审核端点同根，已并入 SR-058。
- 更大的作用域裂缝不是越权，而是功能黑洞：唯一 worker、proposal 生成、auto-release 都固定 default workspace/account；路由则按当前 workspace 加默认 account 展示，非默认 workspace 可保存永远无人消费的 flag，见 SR-085。

### 候选证据、发布基线与回滚所有权（FACT）

- threshold/prompt release 的核心生产写与 proposal 状态使用 Mongo transaction，threshold audit 也在同事务内；但事务只保证当前操作原子，不保证它仍针对评估过的基线。
- Prompt proposal 不保存 critic/shadow 时的 base version/hash，release 会把旧 snippet 追加到发布时最新 current；Threshold proposal虽保存旧值，也不校验当前 override 仍等于该值。候选进入 eligible 后若基线变化，旧证据仍可发布到新对象，见 SR-086；Shadow 本身混用当前运行态的证据污染继续归 SR-049。
- Prompt rollback 只保存发布前版本，不保存本 proposal 实际创建的版本。任何旧 released proposal 都会先翻掉该 key 的当前版本，再恢复自己的历史 parent；后续人工或 Evolution 发布没有所有权保护，见 SR-087。

### 发布后观测提交与测试缺口（FACT）

- release/rollback transaction commit 后才写 AgentEvent；事件失败通过 `?` 返回错误，生产状态却已提交。release 的 post-release review 又排在事件之后，事件失败时连观测 intent 都不会创建；review insert 自身失败也只 warn 且无 reconciler，见 SR-088。
- post-release scanner 无 claim/CAS；它先把 review 标 completed，再插事件。事件失败后外层虽记录“下 tick 重试”，但 completed 行不会再次被扫描；多实例还能并发计算与重复写事件，同属 SR-088。
- 现有测试覆盖 workspace 首跳 filter、Prompt 红线、恢复 archived previous 以及 previous 行缺失时事务中止；没有候选后基线变化、P1→P2→rollback P1、commit 后事件失败、review intent 插入失败或多 scanner 竞态。
- Evolution 需要把 candidate base hash、release generation、rollback ownership 与 post-release intent 建模成同一不可变版本协议；管理面还必须显式选择真实 workspace/account scope，而不是让可写 flag 与唯一 default worker形成两套互不相交的控制面。

## B05H：行业画像 AI 候选生成与人审可达性

覆盖文件：1 个，共 946 行；Git 冻结 blob 46,276 字节，台账 Windows/CRLF 内容口径 47,218 字节。`src/routes/guide_profile.rs` 已按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 为 `e63c71568aed9c5e0ccc546698974565628f7294`，台账 SHA-256 `7edcf5b0ce1a66e31e86bd04460d8adb5f9c88a410a00077e18e209230292c95` 保持不变。另定向反查 DomainProfile CRUD/发布、人审收件箱、Strategy store、发布卡及相关测试；这些反查不重复结算全文覆盖。

### 生成边界与正向控制（FACT）

- generate 入口通过管理员会话取得 current workspace，读取与写入 DomainProfile 时均绑定该 workspace；生成结果固定 `current_version=false,is_active=false`，不会直接切换生产画像。
- AI 返回值先单独提取 `stateMachine` 并按目标结构反序列化校验，再对其余对象执行 snake_case 键规范化；状态机不会因统一键转换而破坏内部约定。
- 建议的 taxonomy values 只进入候选字典，不会在本路由内直接成为 approved canonical；失败也以 best-effort 告警处理，不反向改变画像草稿提交。

### JSON 键规范化与请求稳定性（FACT）

- `to_snake_case` 以 `chars().enumerate()` 产生的字符序号切片原 UTF-8 字符串；遇到非 ASCII 前缀后的大写字母时，序号可能落在多字节码点内部并触发 panic。该 helper 递归处理不受信任的模型 JSON 键，一个本地化或额外键即可在写入草稿前展开请求栈，见 SR-089。
- 现有测试只覆盖 ASCII camelCase/snake_case 键，没有多语言、随机 Unicode 或未知键测试；因此测试不能证明递归规范化对真实模型漂移安全。

### 草稿生命周期与正式人审可达性（FACT）

- AI 生成与手工创建都固定写 `current_version=false`；默认画像列表只返回 `current_version=true`。Strategy store 使用默认列表，生成完成后只展示成功文本，不保留可导航到草稿的 review id。
- Ask-Human collector/summary 只收 `current_version=true,is_active=false`，raw draft 不进入统一待审箱。`ProfilePublishCard` 又调用同一默认列表后按 id 查找，隐藏稿会永久停在“加载中…”。其单测手工 mock 了默认列表返回 `current_version=false` 行，掩盖真实后端契约，见 SR-090。
- 发布卡本身能够展示 states、goals、signals 与 risk rules；问题不是生成状态机不可审，而是草稿没有任何正式路径到达该卡。需要用显式 draft 状态/query 与按 id 读取闭合创建→审核→发布生命周期。

## B05I：用户运营 Guide 候选、确认与原子应用

覆盖文件：1 个，共 593 行；Git 冻结 blob 21,990 字节，台账 Windows/CRLF 内容口径 22,160 字节。`src/routes/guides.rs` 已按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 为 `acf60e7a018fc0ff0ed3a5d95fc0350fe3f8fac9`，台账 SHA-256 `4f4eea1b38c873382960a27810d837fba4412bf8e4c8fb90a2d902087f7647d7` 保持不变。另定向反查 Guide shared helpers、Contact/Preview/Runtime 模型、索引、Playbook/Domain 运行时 loader、正式前端/store、契约 fixture 与相关集成测试；这些反查不重复结算全文覆盖。

### 事务、租约与作用域正向控制（FACT）

- preview 入口先按 current workspace 校验 account 与 contact，并验证 contact 属于请求 account；preview 行保存 workspace/account/contact，apply claim 也带 workspace，跨 workspace id 不泄漏存在性。
- apply 使用 pending/failed/stale-applying 原子 claim、不可复用 apply token 和五分钟 lease；业务写、AgentEvent 与 preview=`applied` 在同一 Mongo 事务中提交。contact/memory 使用 updated_at CAS，Playbook 使用 version CAS，Domain 使用 current+updated_at CAS，旧 lease 的 finalize 失败会使整个事务回滚。
- transaction commit 对 `UnknownTransactionCommitResult` 重试；现有副本集集成测试覆盖事件唯一冲突导致的整组回滚、failed 后重试成功及 applied 后重复调用 409。枚举值与状态迁移另有部分应用测试，非法字段进入 skipped，不拖累同候选内合法字段。

### 候选基线、字段契约与全局授权（FACT）

- preview 模型不保存生成时 contact、memory、Playbook 或 Domain 的 version/hash；apply 在确认时重新读取最新对象并把旧候选覆盖/merge 到新基线。提交期 CAS 只能防重新读取后的竞态，不能识别 preview→confirm 间漂移，见 SR-091。
- Prompt 允许 `suggestedChanges.tags`，apply 却写已从 Contact 模型废弃的裸 `tags`。正式标签读侧只消费 `manual_tags+confirmed_tags`，标准人工路径还有 trim/去重、32 条/64 字符和 actor 约束；Guide 会报告已处理但运行时永远不可见，见 SR-092。
- `impactScope`、风险、可读摘要和是否携带 Playbook/Domain patch 都来自同一次模型输出。后端不计算真实影响范围，前端又优先显示模型摘要而不是规范化字段 diff；同一确认按钮可改共享 Playbook 或 workspace Domain，见 SR-093。
- `domainRuntimeParameters` 接受任意 Document 浅合并，不做已知键、类型、单位、范围或跨字段校验。运行时 BSON 反序列化失败会整份回默认，部分关键参数也没有统一 clamp，见 SR-094。
- preview、apply 事件和终态均未保存真实生成/确认 admin；这一审计主体缺失已并入 SR-058，不另设重复编号。

### 提交结果与测试缺口（FACT）

- 事务成功后 handler 仍串行重读 contact/memory/review/domain/profile 来构造响应。任一读失败会返回 502/404，但 preview 已 applied；前端保留旧预览，重试又固定 409，见 SR-095。
- 现有测试证明事务原子性、lease filter 与枚举部分应用；没有 preview/apply 基线漂移、模型伪报 current_contact 但携带全局 patch、runtime 恶意类型/范围、孤儿 tags 端到端不可见、真实 actor 或 post-commit read failure 测试。
- Guide 的系统级协议应是：冻结 base identities + 规范化 typed candidate + 服务端计算的真实 diff/scope + 绑定 actor 的 candidate hash + 幂等 committed outcome。事务是必要底座，但不能替代预览内容完整性与确定提交结果。

## B05J：顶层路由收尾——软锁、Lesson、评测与可观测聚合

覆盖文件：5 个，按冻结源码逻辑行合计 2,667 行（冻结身份脚本含 `contract_snapshot.rs` 末尾空行时计 2,668）；Git 冻结 blob 合计 99,138 字节，台账 Windows/CRLF 内容口径合计 101,436 字节。包括 `src/routes/contract_snapshot.rs`、`chunk_locks.rs`、`lessons_learned.rs`、`evaluations.rs` 与 `observability.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `7c2ef144b805a4f2d032a5feb36d149d3628a061`、`0bf34dfa34a88e0dada19807e10ff5a40ee20e20`、`4b5490ae2b21111f6c9a71922b0d241f64e9e810`、`c09c4e908253f77b46507f212b8635afb8e2371f`、`5efcc0ee1a93a6b4910dfbb629a7b2fdb8e59299`，台账 SHA-256 保持不变。另定向反查 Chunk revision/OCC、知识编辑前端、Lesson 聚合与索引、Simulation/LLM budget、Observability 写端及对应测试；这些反查不重复结算全文覆盖。本批关闭剩余 5 个顶层 `src/routes/*.rs` 文件，不代表 `src/routes/knowledge/*.rs` 已完成。

### 契约快照与 Chunk 协作编辑（FACT）

- `contract_snapshot.rs` 只在测试构建中启用，提供 JSON canonicalize、fixture 对账和投影函数防腐烂扫描；本批未发现需要单列的生产路径缺陷。其扫描是基于源码文本窗口的近似护栏，不能替代真实前后端集成测试。
- Chunk lock 是进程内 `DashMap<chunk_id,...>`：入口不验证 chunk workspace、key 不含 workspace，get→insert/get→remove 也不是原子 CAS。所有实际编辑路由都绕过 lock 直接进入 revision 内核；前端却据此禁用写按钮并显示“暂只读”。revision 内核自身有 updated_at OCC，但不能兑现跨租户、owner lease 或多副本互斥，见 SR-096。
- WebSocket 只在当前进程广播并按事件内 workspace 过滤；正式编辑调用均使用显式 workspace 的 broadcast helper，无 workspace 的旧 helper 未找到生产调用。广播丢失可由 reload 自愈，不单列为一致性提交缺陷。

### Lesson 晋升与评测契约（FACT）

- Lesson 列表按 current workspace 过滤，聚合端用 `$setOnInsert` 保留已晋升状态；晋升产出的 peer_case 固定 draft + needs_review，不会直接进入 verified 召回。
- 晋升却先 insert chunk，再无 status/workspace CAS 地回写 lesson，集合也没有 promotion 唯一键。并发或中间失败会留下多个候选，只有最后一条被 lesson 指向；事件失败被吞，见 SR-097。
- Evaluation CRUD 只在 create 时检查 scenarioId 非空，未校验 status、account、消息、动态公式或 ground truth。正式 UI 创建 active 场景时不发送 groundTruth，runner 又把缺失/非数值 truth 当 0 并跨 account 消费场景，见 SR-098。
- 单场 Simulation 的 token 只在 task-local `RunBudget` 与 `llm_call_logs` 中记录，不写 `agent_run_logs`；批次 runner 却扫描评测开始后的生产 `agent_run_logs.tokens_used`。无生产流量时预算恒显示 0，有并发生产流量时反而被污染并可能提前终止，见 SR-099。Simulation 本身写生产 memory/知识副作用的既有问题继续归 SR-048，不重复编号。

### Observability 口径与测试缺口（FACT）

- phase-rollup 与 worker-health 都按 current workspace 读，数据库错误会向上返回而不是静默伪装 0；闭集外状态也会透出，方向正确。
- `sweepHitRate` 实际是全历史状态库存中 resolved/total 的比例，分母还包含 pending、dismissed 与闭集外值，没有 sweep generation 或时间窗；前端却显示“扫描命中率”。两个响应的顶层 24h 标签还混合全量库存、14d reviewer/lessons 与 30d attribution，见 SR-100。
- 现有锁测试主要验证常量、序列化和直接 DashMap 状态，不覆盖 HTTP workspace 归属、双 acquire barrier、旧 owner release、绕过 UI 写或多副本。Evaluation 只有数值 helper/fixture 测试，没有正式 UI payload、ground truth schema、真实 token 计费或并发生产隔离。Observability 测试只锁常量/闭集和年龄桶，没有多轮口径、freshness 或固定 asOf。
- 本批的系统级结论是：协作锁必须与真实写权共享 owner/generation；人工晋升必须是可恢复的唯一 intent；评测必须冻结有效 schema 与本批 usage；监控必须为每个数字暴露独立 window/asOf/source。UI 标签不能替代这些服务端协议。

## B05K：知识核心 CRUD、审核、修复与多对象编辑

覆盖文件：6 个，共 5,841 行；Git 冻结 blob 合计 236,003 字节，台账 Windows/CRLF 内容口径合计 241,822 字节。包括 `src/routes/knowledge/mod.rs`、`crud.rs`、`verify.rs`、`repair.rs`、`catalog.rs` 与 `wiki_edit.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `5a64cf33b1ec32767fa5737fa477fe45d08ea846`、`8d60f703e2354946f5e0223eddc58164571049d0`、`d060e460a8d8d8c4306074ef368980d2dd062223`、`170c66a0b314f482d683929cadd2c33c78712cd3`、`5fd4d88cb419c0434ef5283693527cfe840a0f76` 与 `d50731ad50c71b0dced32c4e13c3402387a51a95`，台账 SHA-256 保持不变。另定向反查 Chunk revision/page-merge 内核、模型与索引、正式 Knowledge 前端、AI 修复 helper 及对应集成测试；这些反查不重复结算全文覆盖。`src/routes/knowledge/import.rs`、`sources_meta.rs`、`chat.rs` 与 `digest_inbox.rs` 仍待后续批次。

### CRUD、人工核验与来源边界（FACT）

- 单条 verify/reject 与批量 verify 已统一走 `apply_chunk_revision`，verify 前检查 sourceQuote 与 sourceAnchors；auto-verify 最终把所有 `verified` 强制降为 `needs_human_audit`，方向正确。
- 通用 Chunk PUT 却先用父文档调用 `apply_chunk_integrity`；只要 sourceQuote 能在原文定位，该 helper 就主动写 `integrity_status=verified`。随后 D2 coercion 因 quote/anchor 齐全不降级，且 PUT 直接替换生产行，不经过 `/verify`，AI 修复正式前端正使用这条 PUT，见 SR-101。
- create/PUT 请求只校验 title，status、integrity、account、document 等字段主要信任调用方；正式手工创建 UI 自行写死 draft/needs_review，但后端并不把这条安全姿态作为统一不变量。SR-101 记录其已证实的生产越过路径。
- 文档更新用整行 replace 且重置 version/created_at/catalog persisted 字段；文档删除与 Chunk 删除物理删除主行，绕开 revision、引用清理和 catalog intent。文档删除还先删 document 再删 chunks，第二步失败会留下无父 Chunk，见 SR-108。

### Split/Merge、Rollback 与正式操作栏（FACT）

- split 在新 Document 中先注入当前 workspace、draft 与 needs_review，随后把 caller `newChunks` 顶层字段逐项覆盖；请求可改回任意 workspace、active/verified，直接插入且不走 D2/DomainSchema/revision 写门，见 SR-102。
- split 先归档原 Chunk 再循环建 N 个新 Chunk；merge 先归档一个或两个源，再改目标或创建新 Chunk。全部是无事务多写，后续校验、反序列化或 DB 写失败不会补偿，且 create revision 失败被吞，见 SR-103。
- revision 只保存本次 patch 与 before/after hash，没有前态/后态快照。rollback 却把目标 patch 的 key 从“时间上前一条 revision 的 patch”取值；前一条可能修改完全不同字段，无法由 hash 还原内容，正式 UI 仍承诺“回滚到该版本”，见 SR-104。
- 正式操作栏与后端 schema 系统性漂移：patch 把字段放在顶层而后端要求 `{patch:{...}}`；split 发送 offset/regex 而后端要求 `newChunks`；merge 发送 `target_id` 而后端要求 `mergeTargetId`；relate 提供后端闭集外的 `supports`。四类操作从官方 UI 确定性 4xx，见 SR-105。

### 修复审计、批处理与测试缺口（FACT）

- AI repair proposal/follow-up 本身只读 Chunk/文档并生成候选，正式前端从原 Chunk 组装 PUT，且 `thenVerify=false`；但 PUT 的自动 integrity helper 反向击穿该红线，归入 SR-101。
- `/repair/applied` 不验证 target 存在、session/turn/proposal、实际 revision/hash 或 accepted fields；任意管理员可自报“已应用/已确认”。事件写 helper 又吞掉插入错误，端点仍返回 ok，见 SR-106。
- auto-verify 在调用 revision 写之前增加 processed 与各状态计数，随后以 `let _ =` 吞掉写失败并继续写 usage log；结果和完成事件可报告全部已分诊，而数据库仍保持原状态，见 SR-107。
- catalog/completeness 查询均按 current workspace/account 过滤，完整度结果只用于治理看板；进程内五分钟缓存造成的短暂陈旧未单列。现有测试覆盖 D2 gate、auto-verify 降级、PUT OCC/字段保留和部分批量动作，但没有 PUT 锚定即 verified、split 字段覆盖/事务故障、真实 rollback、正式操作栏契约或伪 repair applied 测试。
- 本批的系统级结论是：Chunk 的所有写入口必须共享后端强制的 draft→human verify 状态机与不可变 revision envelope；split/merge/rollback 需要真实快照、事务和 generation；审计结果必须由服务端绑定已提交 revision，而不是由前端事后声明。

## B05L：知识导入、对话应用、日报收件箱与摄取源

覆盖文件：4 个，共 5,761 行；Git 冻结 blob 合计 225,050 字节，台账 Windows/CRLF 内容口径合计 230,741 字节。包括 `src/routes/knowledge/import.rs`、`sources_meta.rs`、`chat.rs` 与 `digest_inbox.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `b9e07df252550ebaad33e439a46a603b30992102`、`938b5f0d23f63dc511952a0321c0160680862cc1`、`6309e33dc739b51c1c1f1a37fb5e9815f68abe1e` 与 `6b2935a73f810da53d36d257f8f74477b85be418`，台账 SHA-256 保持不变。另定向反查 ingest worker、路由 session middleware、模型/索引、正式 Knowledge 前端与相关集成测试；这些反查不重复结算全文覆盖。本批关闭冻结树中的全部 `src/routes/**/*.rs` 文件，但不代表阶段 6 的 Agent、worker、前端和测试已全部阅读。

### 导入提交与 AI 人审边界（FACT）

- 长文 preview 会按标题/段落分块并发抽取，单段失败允许部分结果，全部失败才报错；异步 job 的 GET/list 按 current workspace 过滤。导入 apply、PDF、图片与 worker 最终都把 AI/Imported Chunk 强制为 `draft+needs_review`，未发现新的自动 verify 旁路。
- JSON apply 的准入条件却只看已废弃的 `items` 或 `chunkedText`，不看真正处理的 `chunks`；随后又直接丢弃 items。正式前端仍发送 preview.items，因此只有 chunks 而 items 为空的合法结果会 400，见 SR-112。
- apply 与 `ingest_chunked_text` 都先 insert 文档，再逐条直写 Chunk；没有 transaction、candidate id、幂等键或 committed outcome。旧 JSON/fallback blob 还不写 create revision，fence revision 失败被吞。中途失败和重试会留下 active 空文档、部分行或重复知识，同属 SR-112。
- 图片入口按 current workspace 选择支持视觉的 provider，PDF/图片结果同样进入待审池；视觉候选切换与“AI 永不自动 verify”方向正确。图片/base64 与 PDF 请求体积的更广泛边界留给阶段 8/9 的前端、配置和部署核验，不在本批重复推断。

### Chat 会话、应用与长任务（FACT）

- chat turn 按 workspace/account/session 读历史并用原子 sequence 分配两个 turn index，LLM 有 token/call/tool-loop 上限；create/update apply 都强制 draft+needs_review，update 走 revision 内核并重算 quote/anchors。
- apply 却故意以 account=`*` 读取同 workspace/session 的所有账号历史，在内存挑最后 pending patch，先执行业务写、后无条件标 turn applied；没有原子 claim、apply token、唯一 source-turn 或事务。并发/响应丢失会重复建 Chunk，同 sessionId 跨账号还会混用候选，见 SR-111。
- session sequence、history、discard 与 SSE key 也只绑定 workspace/session；前端本地 loading 无法提供多标签、多设备或网络重试幂等。长任务 create 先建 task 再写 progress turn，同样存在展示写失败后的不确定响应，但其任务本体已由 worker claim；本批没有把该较窄窗口拆成独立发现。
- task action 在入库前有六值闭集、步数上限和 workspace/account 日报反查；task list/get/cancel 按 current workspace 过滤。SSE session stream 本身不带 session middleware 参数或 workspace key的更大授权问题需结合外层路由与前端阶段继续核验，本批不在证据不足时升级。

### 摄取源、元数据与日报收件箱（FACT）

- ingest source CRUD 按 current workspace 读写，但 URL 只做非空校验。worker 跨 workspace 扫 active/failing 行，用默认 reqwest 策略直接 GET 并跟随重定向，没有 scheme/IP/逐跳校验、出站 allowlist或响应体上限，形成持久化 SSRF，见 SR-109。
- metadata 的 Chunk facet 正确先按 current workspace 过滤；revision facet 因模型无 workspace_id，直接聚合全集合 topEditors 与 7d activity，并由正式 Atlas 展示，见 SR-110。
- ask JSON/SSE 忽略客户端 workspaceId，使用 session current workspace，且显式 `include_unverified=false`；此前已确认的同 workspace account 可见域泄漏继续归 SR-029，不重复编号。
- digest today/regenerate 按 workspace+account+date 读取；inbox 的日报与 Chunk 查询也绑定 workspace/account 可见域。dismiss card 只按 workspace+today+cardId、不含 account；ObjectId 随机且需同 workspace 碰撞/获知，当前证据不足以证明独立越权，不单列。
- 本批系统级结论是：导入和 Chat apply 都需要不可变 candidate/turn identity、原子 claim、事务提交与幂等 outcome；外部摄取必须走统一 SSRF-safe egress；revision 必须自带 tenant key，不能靠仍存在的主 Chunk 推导授权。

## B06A：Knowledge Wiki 修订、信号、反馈与摄取内核

覆盖文件：11 个，共 5,604 行；Git 冻结 blob 合计 221,243 字节，台账 Windows/CRLF 内容口径合计 226,177 字节。包括 `src/knowledge_wiki/mod.rs`、`block_parser.rs`、`chunk_revisions.rs`、`page_merge.rs`、`gap_signals.rs`、`structural_proposals.rs`、`reviewer_stats.rs`、`feedback_worker.rs`、`catalog_rebuild.rs`、`ingest_worker.rs` 与 `lessons_learned.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `08f9fdb36166db8cb515c325cb8e039972101502`、`802cade9e42abd11508e79e03c25843ae4fba487`、`bf53328394bde95b017ee85de607d17c5483aaab`、`f7ebdfb10cf375f5cf5b46a8442c98a2eaee1750`、`d427039838338fd4e68c39273c1ac2f39fd6dc5b`、`e3dc08ee60f2882073363c248f9bf7308ef06755`、`0389947d4faff02b30192929edfd8a38d9935dc4`、`0692b8f6b36d26ced7257e59e4f48ef76ee256f0`、`717a3867baf89671373c073038814384546196c2`、`0e88f4fb5e83fe4b23e3126cc2c0bd8df81f7e81` 与 `b94c303d8932c8605587c3dfcbbdbd147ca42727`，台账 SHA-256 保持不变。另定向反查知识路由、模型/索引、Knowledge Agent 调用点、正式 Steward 前端与 ingest/revision 集成测试；这些反查不重复结算全文覆盖。

### Revision 写门与目录投影（FACT）

- `page_merge` 的数组 union、默认/运行时锁字段恢复、70% 文本截断门和 canonical hash 均有纯函数测试；AI source 在 revision 内核中会强制 `draft+needs_review`，updated_at CAS 能拒绝同时替换同一 Chunk，方向正确。
- 通用 patch 的请求体却是自由 BSON，默认锁集遗漏 workspace/account/document/domain/status/integrity 等身份与生命周期字段；human source 可把当前租户 Chunk 移入其它 workspace 并直接标 active+verified，见 SR-113。
- revision 内核先覆盖 provenance，再计算未排除 provenance 的 hash，因此相同业务 patch 通常也不是 no-op；它先插 revision、后反序列化并 CAS replace 主行，失败补删也可能失败，历史不等于已提交事实，见 SR-114。revision 缺 tenant key 的既有问题继续归 SR-110，缺 snapshot 的 rollback 问题继续归 SR-104。
- catalog job 领取是 queued→processing 原子 CAS，但没有 lease、worker token、stale reclaim、失败重试或 desired/applied generation；enqueue 还是主写后的 best-effort，持久目录可永久落后，见 SR-115。
- 正式诊断页又把 persisted `{documents}` 和 live `{item}` 错当 `{total,items}` 与顶层 `{total}`，两次 2xx 最终稳定显示目录覆盖 0/0，见 SR-118。

### Gap、反馈统计与账号归因（FACT）

- structural lint、pending signal 的部分唯一索引/upsert、在线 recall signal 的只增不消解语义，以及 outcome 的 Hit/Block/Censored 三态均有明确实现；沉默不被当负例，规则 signal 与 recall trace 来源分离。
- 30 天置信度重算按 workspace 加载 usage logs；真实 outcome join 仍按 run_id+workspace。但“成交追认”随后按裸 contact_wxid 分组并查询 contacts，丢弃 usage log 已有的 account_id，导致同 workspace 多账号复用 wxid 时成交样本串扰，见 SR-116。
- feedback worker 逐 workspace 顺序执行 usage refresh、lint、sweep、lesson 与 reviewer stats；每个子步骤失败只 warn 并继续。它没有 durable round/generation，阶段 5 的 Observability 口径问题已归 SR-100，本批不重复编号。
- `structural_proposals` 从类型层锁死 `pending_review` 且无 apply/commit/delete 字段，守住 AI 不自动执行结构写的红线；但全仓只有 Knowledge Agent 生产者，没有人审 UI 或 apply consumer，属于代码已明确标注的就绪债，本批记录但不按安全缺陷另编号。

### 自动摄取、解析与测试缺口（FACT）

- block parser 对 fence、unsafe id、截断、重复 id、非法 JSON 与 stray text 做纯函数处理；RSS/HTML renderer 的输出能被它重新解析。摄取后的 Chunk 仍经 `ingest_chunked_text` 强制待审，未发现自动 verify 旁路。
- worker 对 source 没有 claim/lease/generation；多副本可同时看到相同 checkpoint 并各自调用非幂等导入，抓取期间改 URL/status 时旧结果还会用仅含 source_id 的 filter 覆盖新 checkpoint/status，见 SR-117。URL 本身的 SSRF 与无 body cap 继续归 SR-109，导入多写非原子继续归 SR-112。
- ingest 测试覆盖单 worker 的 due/not-due、HTTP 失败、ETag/内容 hash 串行去重与 parser 兼容，但没有双 worker barrier、慢请求期间改 URL、旧 generation finalize 或 lease 回收。revision 测试也没有相同 patch no-op、history insert 后 replace 失败、补删失败和跨 workspace patch。
- 本批系统级结论是：知识写入必须由 typed mutation command 固定租户/审核边界，并以事务产生 committed revision 与 catalog generation；所有周期 worker 必须有可回收 lease/generation；归因键必须保留 workspace/account/contact 全维度；前后端目录契约必须由共享 schema 生成并拒绝未知包络。

## B06B：知识日报与长任务 Worker

覆盖文件：3 个，共 2,050 行；Git 冻结 blob 合计 76,778 字节，台账 Windows/CRLF 内容口径合计 78,828 字节。包括 `src/knowledge_digest/labels.rs`、`src/knowledge_digest/mod.rs` 与 `src/knowledge_task/mod.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `2928bd938dc7c19b3e5da247641185f0a51c0949`、`5f0238a9c2ceb9e577494dca7b2198a72057e883` 与 `fa4247511419c0c38631fa1d05bc604ad5564b62`，台账 SHA-256 保持不变。另定向反查 Digest/Task 路由、模型与索引、配置、Prompt loader、公共 LLM helper、Knowledge Router 可见域、设计规范及专项测试；这些反查不重复结算全文覆盖。三个 Knowledge Agent 主文件已在 B04G/B04H 全文结算，本批不重复计数。

### 日报作用域、预算与失败快照（FACT）

- 定时 worker 每天只为配置中的 default workspace/default account 生成日报；其它租户和同 workspace 非默认账号只能在打开 today 或手工 regenerate 时同步触发。更关键的是 Chunk health 严格匹配 `account_id=当前账号`，遗漏生产召回与 Inbox 都可见的 workspace 共享 Chunk；compose/summarize 两个 Prompt 又固定读取 default workspace，见 SR-119。公共 LLM 日志固定归默认 workspace 的既有问题继续归 SR-018。
- 配置层公开 `KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET` 与 `...MAX_LLM_CALLS`，但生成入口直接写死 `24_000/8`，测试也只验证硬编码默认值；部署调小不能限流、调大不能扩容，见 SR-120。
- 四路分析和 compose 在内存中全部完成后才写报告；任一路失败或预算超限都会 upsert 同一 `(workspace,account,date)` 行并把 `cards` 覆盖为空。所谓 `partial` 没有任何部分卡片，失败重算还会抹掉当天原成功快照，见 SR-121。
- 正向控制包括日报三元唯一索引、稳定 cardId、`dismissed_card_ids` 的 `$setOnInsert` 保留、卡片枚举/长度上限与 severity 排序。现有测试主要覆盖纯解析、BSON round-trip 和跨 workspace 报告落库，不覆盖共享 Chunk、非默认 Prompt、配置预算、成功后失败重算或真实 partial 保留。

### 长任务所有权、结果真实性与取消（FACT）

- worker 先普通读取最早 pending，再靠 `_id+status=pending` 把任务置 running；进程内 session mutex 只能串行当前副本。模型没有 claim owner/token、lease、heartbeat、attempt 或 next-step generation，worker也不回收 stale running。任一异常退出都会永久卡住；若人工改回 pending，已发生的 add/retag/dismiss 副作用又没有 step 幂等键，可能重复，见 SR-122。
- 每个 step 的业务副作用、`completed_steps`、progress turn 与任务终态是独立写。取消只改 status；正在运行的 step 不受 fencing，最终 running→terminal 更新的 matched count 也不检查，随后仍按本地 `final_status` 写 summary/event。取消、崩溃和响应丢失因此都可能让数据库状态、展示和真实副作用分裂，同属 SR-122。
- `fix_chunk` 缺/非法 id、LLM 失败，`add_chunk` 空摘要/空 patch/生成或落库失败，`retag` 不存在/抽取失败，以及 `dismiss` 无效 id/写失败，大多都返回 `Ok(StepOutcome)`；调度层统一记 `status=ok`，最终可报告“全部成功”，见 SR-123。action 闭集、最多八步、AI 写入强制 draft+needs_review 和原子 turn sequence 是局部正向控制，不能修复 outcome 语义。
- stable cardId 只由日期、kind、target refs 和 title 派生，不含 workspace/account；HTTP dismiss 与 Task dismiss 又只按 workspace/date/card 或 workspace/card 更新。两个账号产出同一卡片时，一方可隐藏另一方卡片，且 worker 吞掉 matched/write 结果仍报告成功，见 SR-124。
- 本批系统级结论是：日报生成必须显式携带 `(workspace,account)` 可见域、租户 Prompt 与配置预算，并以 immutable generation 保存 last-success/attempt；长任务必须采用 lease+token+step outcome 的可恢复协议，副作用、幂等 outcome、进度与终态需绑定同一 generation，取消必须 fencing 已运行执行者。

## B06C：Digest/Task 专项回归测试

覆盖文件：7 个，共 950 行；Git 冻结 blob 与台账内容口径均合计 41,884 字节。包括 `tests/digest_cross_tenant_scope_integration.rs`、`knowledge_chat_dispatch.rs`、`knowledge_digest_budget_smoke.rs`、`knowledge_digest_compose_smoke.rs`、`knowledge_digest_skeleton.rs`、`knowledge_operator_memory_isolation.rs` 与 `knowledge_task_worker.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `d14f358140e8fdbc3cb8abfd493fc9310c69da43`、`8f84d178591a50fe524058eb514b7888088a580d`、`ea54f313250bd8f0a19ef3469468f7075a59e588`、`6f2f2eb03b58c677bc43d3e74bc885694c602615`、`db6694a81f4e959fbccb2dc64671161c74a43f97`、`59f66ff7156ce58275122cedabde854fa3ad012e` 与 `52422a0cda46ad57e40dc8993e9698d6388eaf68`，台账 SHA-256 保持不变。另定向反查生产 Chat dispatch/create 接线、正式 Knowledge 前端与其组件测试；这些反查不重复结算全文覆盖。

### 测试覆盖真实性（FACT）

- 唯一直接调用真实 handler 的是需 Docker、默认 `#[ignore]` 的跨租户 today 测试；它只证明非默认 workspace 按需生成后报告写回该 workspace，不覆盖定时枚举、共享 Chunk、租户 Prompt 或账号作用域。
- budget 测试直接构造 `RunBudget(24000,8)`，没有从 AppConfig 调生成入口；compose/skeleton/task 测试主要是 BSON round-trip、手写枚举与 watch bus 行为。它们不会运行 Digest analyzer/upsert、Task claim/execute/finalize，因此 SR-119–SR-124 的配置失接、失败覆盖、stale running、虚假成功与跨账号 dismiss 均未被真实回归保护。
- `knowledge_task_worker` 仍把历史值 `finished` 放进“合法状态”fixture；Serde 对 String 不做闭集校验，所以测试通过也不能证明生产闭集。正式模型与 worker 已改用 `completed`，这是测试漂移证据，不另立生产缺陷。
- `knowledge_chat_dispatch` 声称每个 step.cardId 必须来自 selectedCards，但只对同一测试内手工构造的两个 JSON 数组做包含判断。生产 Chat 请求不携带 cardIds、dispatch 输入是日报前 20 张全量候选、确认请求固定 `cardIds=[]`，create 端也允许空/未知 cardId 与 target override，见 SR-125。
- 六个默认可执行目标已发起 `cargo test`。`knowledge_chat_dispatch` 完成并通过 4/4；其余五个目标在 MSVC 并行链接阶段持续占用约 18 GB、近乎零 CPU 且输出文件长期零增长，未取得可执行终态。确认挂起后仅结束本次命令创建的 13 个 Cargo/rustc/link 进程，零残留；因此这五个目标明确记为“链接挂起、未验证”，不伪报通过。冻结源码审查结论不依赖该运行结果。

### 系统级结论

- 单测中复刻闭集、构造合规 fixture 或验证 serde 往返，只能证明样例可表达，不能证明生产 handler/worker 强制不变量。关键回归必须穿过真实请求 DTO、租户选择、数据库过滤、worker 状态迁移与 committed outcome。
- 派工的确认对象必须是服务端冻结的候选，而不是模型输出加前端按钮。选择集合、card refs、action 与 target 需在创建任务时重新做服务端关联校验，并由任务保存可审计 candidate hash。

## B06D：Knowledge Agent 评测、PBT、闭环与 Worker 回归

覆盖文件：7 个，按既有非空逻辑行口径共 1,534 行（物理行 1,684）；Git 冻结 blob 合计 68,263 字节，台账 Windows/CRLF 内容口径合计 68,576 字节。包括 `tests/knowledge_agent_eval.rs`、`knowledge_agent_pbt.proptest-regressions`、`knowledge_agent_pbt.rs`、`knowledge_closed_loop_trajectory.rs`、`knowledge_router_fallback_e2e.rs`、`knowledge_tools_budget.rs` 与 `knowledge_worker_behavior_integration.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `31928286a70bf6e866941c755185edbedf3bc8e1`、`062abdc91a87001e94f8428e7c074790548205d0`、`32f2b401ac6f43f1c90bb8779d3aab4d7d7187a6`、`27954a232270fc603bba055b0a7e066128734fc8`、`da92d86d6a7fe37f26280d6fb560190b2bbe20f5`、`116217f3f9580442cce7d93f5f6646b4a54c1615` 与 `7b887d80379e4fb6d28be8cba9f73a5fa41e371f`，台账 SHA-256 保持不变。另定向反查 Knowledge Agent/Router 生产实现、Chunk BSON 字段形态、CI ignored job 与既有发现；这些反查不重复结算全文覆盖。

### 有效局部不变量与真实接线（FACT）

- PBT 直接调用生产纯函数，覆盖 cited/quote 只能来自 opened、catalog merge 去重、wiki type 排序、CJK 安全截断、prefetch 不丢项、rank key 全序/时效/取代降权、recall signal 分类与 structural proposal 恒 pending_review；回归种子保留了曾出现的重复 chunk id case。这些是有效的局部性质测试。
- `knowledge_router_fallback_e2e` 穿过生产 Router，覆盖 agent 零引用/OOB 引用后的 weak/medium fallback 和空 corpus missing；`knowledge_tools_budget` 覆盖公共 RunBudget 的 token/LLM/tool 三维与失败不消费配额。它们证明相应局部接线，不证明账号下钻、引用内容锚定、缓存失效或任务提交协议；这些生产缺口继续归 SR-030–033、SR-122–125。
- `knowledge_closed_loop_trajectory` 的最后一个用例确实调用生产 verify handler，能证明 needs_review 在 verify 前不进默认 catalog、核验后可见；但新增、取代与关系三条所谓维护闭环均绕过维护 Agent 和 revision，不能外推到完整写链。
- 本机以 `CARGO_BUILD_JOBS=1` 顺序运行两个无 Docker 目标：`knowledge_agent_pbt` 17/17、`knowledge_tools_budget` 8/8 通过。结果证明上述纯函数与公共预算计数器性质；其余依赖 testcontainers 的 ignored 用例未在本机运行，不能据此外推真实 Router、维护写链或 Worker outcome 已通过。

### 假质量门与覆盖缺口（FACT）

- `knowledge_agent_eval` 把期望 Chunk id 直接喂给 mock LLM 的 open/answer，再用同一 id 计算命中率；80% 阈值不测检索能力。清理还错误使用 camelCase `workspaceId`，实际 Chunk 持久字段是 `workspace_id`，见 SR-126。
- 闭环测试直接插 verified Chunk、直接 `$set superseded_by`、直接构造关系；Worker behavior 测试仍宣称 action 是占位桩，并把非法目标、空动作等返回 `Ok` 固化为期望，反向保护 SR-123 的虚假成功，均见 SR-126。
- 这些 ignored 用例会进入 CI 的 Docker integration job，但该 job `continue-on-error` 的既有门禁问题已归 SR-004；本批不重复编号。即使 job 运行，脚本自证与绕过生产写链仍会给出误导性绿色结果。

### 系统级结论

- 测试 ground truth 必须独立于被测输出：expected ids 不能被注入模型动作，闭环 mutation 不能由测试直接写成期望终态，业务失败也不能仅因函数返回 `Ok` 就计成功。
- Knowledge 的关键门应从真实 Router/maintenance command/task worker 入口出发，冻结 query/candidate/actor，验证 committed revision、审核状态、关系和 per-step outcome，再以独立相关集衡量召回。纯函数 PBT继续保留，但只声明它实际证明的局部性质。

## B06E：Knowledge Ask、流式、自动核验与 Chat apply 集成测试

覆盖文件：5 个，按既有非空逻辑行口径共 1,090 行（物理行 1,226）；Git 冻结 blob 合计 48,400 字节，台账 Windows/CRLF 内容口径合计 48,583 字节。包括 `tests/knowledge_ask_e2e.rs`、`knowledge_ask_stream_e2e.rs`、`knowledge_auto_verify_enforce_integration.rs`、`knowledge_chat_apply_integration.rs` 与 `knowledge_preview_workspace_scope.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 分别为 `e7281dca793a39a73ee9deb72e3eb2e03827f9d9`、`c0b24d5b9d62e15bf2daf241a57627880c0701be`、`bc0869513bd622f7695bf54856ce431a0dad9c23`、`573449adbdfb868aac8375893261f9f3f98629bd` 与 `3d16922a145e114c55b36cea7cefedf9d59c745e`，台账 SHA-256 保持不变。另定向反查正式 Ask JSON/SSE handler、AskView 与前端 SSE 测试；这些反查不重复结算全文覆盖。

### 有效覆盖（FACT）

- `knowledge_ask_e2e` 直接调用生产 Agent 内核并使用真实 Mongo，覆盖正常 open→answer、空 corpus、四轮不收敛、verified-only、contradiction 标记、superseded 多跳/未审新版/自环与 redirect 后 cite 对齐；这些是 Agent 内核的有效集成性质。
- auto-verify 测试直调真实 handler 并复查数据库，能锁住 LLM 自称 verified 后仍强制 `needs_human_audit`；Chat apply 测试直调真实 create helper并证明新 Chunk 落库瞬间固定 `draft+needs_review`。preview 测试则穿过正式 Router helper，证明无 contact 时使用传入 workspace，不回落默认租户。
- 上述 19 个测试全部 `#[ignore]` 且依赖 testcontainers；本机本批未运行 Docker 用例。它们进入 CI soft integration job，但不阻断合并的问题继续归 SR-004。

### SSE 契约断裂与测试边界（FACT）

- 名为 `knowledge_ask_stream_e2e` 的五个用例全部直调 `answer_streaming` 与内存 unbounded channel；没有调用 `ask_knowledge_stream`，因而不覆盖 HTTP query/schema、session workspace、SSE event 名、序列化、断连 Drop 或 handler 的 Agent Err 分支。
- 正式 handler 把 Agent Err 包装成普通 `trace(tool=error)`，再正常发 `close`；AskView 只在原生 EventSource `error` 时显示横幅，普通 trace 不改变错误状态，close 又隐藏 pending-only 时间线，形成确定的静默失败，见 SR-127。前端现有测试只模拟原生 `error`，同样绕开真实后端错误帧。
- AskView 显示 `roundsUsed/3`，而生产上限与本批测试均是 4；这是同一未共享响应契约的次级漂移，归入 SR-127。

### 系统级结论

- Agent 内核测试、HTTP/SSE adapter 测试和前端逐帧消费测试必须分层命名并分别存在。内存 channel 的 Step/Token/Final 顺序可以证明内核事件，但不能证明浏览器看见的 event 类型或错误终态。
- 流式 RPC 必须以可机器识别的唯一终态结束；`close` 只能表示传输结束，不能同时承载 success、cancel 与 failure。前端若在 close 前未收到终态，应主动报错，而不是把空白页面当成功。

## B06F：Knowledge 真模型全能力回归

覆盖文件：1 个，按既有非空逻辑行口径共 1,439 行（物理行 1,546）；Git 冻结 blob 68,755 字节，台账 Windows/CRLF 内容口径 70,301 字节。`tests/real_llm_knowledge.rs` 已按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 为 `5694329c344c0bc46d30bbe5fd3ee8f8048973b8`，台账 SHA-256 `6210622f3427f99e330d5f58be403e06652c06e222d99cb52ac9a2761ce3fed2` 保持不变。另定向反查生产 catalog/rank、auto-verify/chat/vision 分支、nightly job 与 skip ledger；这些反查不重复结算全文覆盖。

### 有效红线与运行边界（FACT）

- 13 个用例均使用真实 provider、真实 Mongo 与生产 Agent/handler；K1 硬断真实 `open_chunk`，K4/R4.2b 的 verified-only、K5/K8/K11 的不自动改写，以及 create/repair 后数据库状态复查，仍是有价值的否定式红线。
- 顶层 `unwrap_or_skip_transient!` 会把结构化 `LlmUnavailable` 写入 skip ledger，配置型 4xx 则 panic；nightly 还汇总 ledger，超过 12 条才失败。这只能观测冒泡到测试宏的显式 transient，不能观测 handler 内吞错或业务成功响应中的空/错误分支。
- 本批未调用外部真模型，也未运行 13 个 ignored test：验证这些测试会消耗真实密钥、触网并依赖 Docker，不是源码审查所需的安全本地检查。结论来自冻结源码、生产排序算法与 fixture 的确定性复算。

### 隐式未验证与 fixture 漂移（FACT）

- K2 声称低置信 B 被 30 条填充项挤出 catalog，只能沿 A→B 关系触达；当前生产从 400 候选按 query 相关度重排再截 30，且正文参与 haystack。按同一 `text_signals/relevance_score/rank_key` 复算，B 约 0.42 排第 1，A 约 0.23，填充项约 0.19；测试又接受直接 `open_chunk(B)`，因此不走关系也能绿，见 SR-128。
- K3/R4.2a 对无覆盖问题只约束 cited id 属于 seed；引用无关 seed 仅 warn，并跳过 cited 为空时才执行的 gap 闭环。K7 在 auto-verify 单条 LLM 失败被 handler 吞掉、`processed=0` 时仍绿，revision 断言随之跳过。K10 允许 `freeform`、无 patch、`canApply=false`；K6 允许 vision 返回空 fence、零 `chunkIds` 后空循环通过，均见 SR-128。
- skip ledger 只由顶层 transient 宏写入；上述 200+空结果、handler `continue`、错误 intent 和 fixture 前提失效都显示 0 skip。由此“13/13 passed”不等于 13 个声明能力均发生。

### 系统级结论

- 真模型测试必须同时证明“模型被调用”和“目标能力发生”。每个 case 应产生 typed evaluation outcome，至少记录 branch、LLM calls、目标 artifacts、实际执行的 assertions 与 skip/failure reason；任何目标见证缺失都不能算 pass。
- 能力 fixture 依赖生产排序/过滤时，先用生产 API 硬断前置条件。关系遍历测试必须先证明目标不在初始 catalog，再要求关系 trace；无覆盖测试应使用独立 relevant set 判断引用相关性，不能只验证 ObjectId 来自 seed。

## B06G：Knowledge 真模型内容质量回归

覆盖文件：1 个，按既有非空逻辑行口径共 2,735 行（物理行 2,918）；Git 冻结 blob 150,653 字节，台账 Windows/CRLF 内容口径 153,571 字节。`tests/real_llm_knowledge_quality.rs` 已按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，Git blob 为 `7896698e0100d07228ea49a25478b64eb89c4670`，台账 SHA-256 `a1b8c90efe87d700dc41a09631199b24ce200d2eb3cbdc56d48e630a2e8a9db0` 保持不变。另定向反查 nightly quality matrix、公共 skip gate 与 B06F/SR-128；这些反查不重复结算全文覆盖。

### 有效确定性门（FACT）

- Q2 对 16 类文档建立 train/holdout 语料，每条都要求至少产出一个 chunk/item、所有 preview Chunk 维持 draft 且不进入 verified，并用手写原文 token 单元计算确定性 recall。末尾分别硬断 train/holdout 平均 recall ≥0.6、两 split 非空且泛化差距 ≤0.18；这些断言不依赖 LLM judge，裁判不可用也照常执行，是本文件最可信的质量证据。
- `reference_recall`、median/spread、语料 split/类型唯一性、三态裁决、裁判自身极差剔除、三裁判泛化与校准维度一致性均有默认执行的纯函数回归。它们能证明仪器纯逻辑按代码定义工作，但不证明某次 ignored 真模型运行取得了有效质量 verdict。
- Q1 仍硬断 answer 非空、引用 id 属于 seed 且至少命中一个价格异议关键词；Q3–Q7 对“不自动写 verified/只读/不落库/返回数组”等否定式状态也有局部硬断。这些只证明对应断言，不能外推为内容质量已经判定通过。

### 无结论被表示为绿色（FACT）

- 质量 verdict 有 `Pass|Fail|SkipDivergent|SkipInsufficientJudges|SkipCalib` 五态；后三态由 `handle_verdict` 只打印后正常返回。裁判全失败时顶层 `LlmUnavailable` 会写公共 skip ledger，但校准拉不开、跨裁判分歧过大或有效裁判不足只写 `quality.jsonl`，不进入 `skip_ledger.jsonl`。nightly 的公共 skip-rate 脚本只数后者，因此可在所有质量 verdict 都 inconclusive 时报告 0 skip。
- Q3 vision 返回空 fence/零 `chunkIds` 时直接 return；Q4 命中固定“AI 工具循环超时”回退文案时直接 return。两者都没有 typed outcome 或公共 skip 记录，矩阵仍显示测试通过。Q4 的硬断只要求 intent 属于七值闭集，`freeform` 也合法，判分若再无结论便没有证据证明 create intent/起草能力发生。Q8 对无覆盖问题仍只硬断 cite id 属于 seed，诚实弃答完全依赖可能 inconclusive 的 judge，均归 SR-128。
- Q2 每文档 judge 的 `None`、分歧/不足/校准 skip 同样不阻断，但其确定性 recall/泛化门独立存在，故本批不把整个 Q2 判为假门；应把“确定性召回通过”和“内容 judge 有结论”分成两个 outcome，而不是一个绿色测试名。
- 本批未调用外部真模型，也未运行 8 个 ignored Q 测试；它们会消耗真实密钥、触网并依赖 Docker。冻结源码 SHA/blob 与工作树源码零漂移已复核。

### 系统级结论

- 质量评测的终态应是 `pass|fail|inconclusive|infra_skip`，其中 inconclusive 不是 pass。每个矩阵项必须上报目标 artifact、硬断言覆盖、有效裁判数、校准结果和最终 verdict；聚合门按有效 verdict 覆盖率及 inconclusive 比例失败，而不是只数顶层 transient。
- 保留 Q2 的确定性原子事实 recall 与 train/holdout 泛化门，并将 judge 作为附加证据。Q1/Q3–Q8 也应增加独立、可复现的目标能力见证，避免内容质量完全依赖可无结论退出的裁判团。

## B06H：Knowledge 前端契约与 Cockpit

覆盖文件：21 个，按既有非空逻辑行口径共 1,895 行（物理行 2,084）；Git 冻结 blob 合计 58,070 字节，台账 Windows/CRLF 内容口径合计 60,154 字节。包括 5 个知识契约 fixture、3 个契约键集声明、2 个契约测试，以及 `frontend/src/features/knowledge/cockpit/` 下 11 个 TSX/CSS 文件；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，21 个冻结 blob 与台账 SHA-256 均保持不变。另定向反查 auto-verify/chat/gap-signals 正式后端 DTO、响应投影与 Cockpit 专项测试；这些反查不重复结算全文覆盖。

### 有效契约与展示控制（FACT）

- Chunk 列表、Chunk 详情顶层、Document 列表与 usage log 的 fixture/CANONICAL_KEYS 做双向键集对账；后端投影增删键后若 fixture 被正确 re-bless，这些测试能强制声明同步。详情 fixture 也明确暴露了列表 camelCase 与裸模型 snake_case+ExtJSON 的形状差异，而没有假装二者一致。
- Cockpit 对 completeness/integrity/gap-signals 的加载会区分请求失败与真实零值；gap-signals 正式响应确为 `{signals}`，该处没有发现包络漂移。`useGoLive` 对 apply→verify 顺序、4xx gate 与 5xx/网络错误有显式状态，局部流程清晰。
- 上述键集测试只证明静态 fixture 的顶层键名，不覆盖 mutation request DTO、字段命名、嵌套 attachment 或 Chat 实际响应；B06H 的两个生产断裂正发生在这些未覆盖边界。

### Cockpit 运行时契约漂移（FACT）

- AutoVerifyPanel 发送 `confidence_threshold` 与 `human_audit_sample_rate`，而后端 `#[serde(rename_all="camelCase")]` 只接 `confidenceThreshold` 与 `humanAuditSampleRate`。未知键被静默忽略，运营选择的 5/7/9 阈值及 30%/5% 抽审比例均回落默认值；现有测试只 mock 200 与结果数字，不检查 request body，见 SR-129。
- ReviewChat 文案承诺“只动这条”，请求却发 `attachments:[{chunk_id}]`，后端只接 `{chunkId}`，目标绑定被静默丢失。正式响应把 patch 放在顶层 `draftPreview`，组件只读 `turn.patch/data.patch`；真实改动不会显示 diff，但 sessionId 仍可驱动 apply→verify。专项测试手写 `{turn:{patch}}` 假响应并不检查请求体，双向绕过生产契约，见 SR-130。
- 本批未运行前端测试；源码与冻结 blob 零漂移已核验。两项结论由 TypeScript 正式请求、Rust serde DTO、正式 JSON 响应和测试 mock 的确定性对账得出，不依赖运行时网络。

### 系统级结论

- 前端—后端共享契约必须覆盖查询、mutation 请求、响应与嵌套对象，不能只对账少数列表 fixture 顶层键。服务端对控制参数应拒绝未知字段或回显 canonical effective input，避免 200 成功掩盖参数失效。
- 审核 Chat 的可应用对象应是服务端签发的 typed candidate，绑定 workspace/account/session/source turn/target chunk/patch hash。UI 只有在目标与 diff 均和 candidate 一致时才能启用 apply/verify；自然语言回复或 sessionId 本身不构成运营已确认改动的证据。

## B06I：Knowledge 前端 Ask、共享治理、修复与 Schema

覆盖文件：16 个，按既有非空逻辑行口径共 3,839 行（物理行 4,096）；Git 冻结 blob 合计 154,805 字节，台账 Windows/CRLF 内容口径合计 158,901 字节。包括 `ChunkRepairPanel.tsx`、`DocumentRepairPanel.tsx`、`DomainSchemaEditor.tsx`、`explore.tsx`、`index.tsx`、`labels.ts`、`shared.tsx`、`trustTypes.ts` 与 8 个对应专项测试；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，16 个冻结 blob 与台账 SHA-256 均保持不变。另定向反查通用 API client、Document/Chunk 正式 handler、软锁响应、repair helper、DomainSchema DTO 与既有 SR-104–106、SR-127、SR-131；这些反查不重复结算全文覆盖。

### 有效局部接线（FACT）

- AskView 已移除误导性的租户输入框，并用 `resultRef` 修复“上一轮成功后下一轮 SSE error 被旧 result 抑制”的 stale closure；对应测试对这两个局部行为有直接覆盖。后端 error 被编码为普通 trace+close、close-before-final 静默终止以及 `/3` 写死仍归 SR-127，本批不重复编号。
- Inspector 的软锁 acquire/409 响应与 WebSocket event 都是 snake_case，前端读取形态一致；unrelate 测试也直接断言源/目标 URL。DomainSchema editor 的 camelCase `schemaId/allowedValues/aliasDict` 与后端 serde DTO 对齐，后端继续负责字段闭集、enum 和 alias target 校验。
- 手工新建 Chunk 测试硬断 `draft+needs_review`；repair 面板对 propose/followup/字段勾选/失败态有局部覆盖。其最终 PUT 与 applied event 分离、客户端自报审计的提交缺口继续归 SR-106；Inspector 的 patch/split/merge/relate DTO 漂移继续归 SR-105，rollback 不可恢复继续归 SR-104。

### 文档编辑测试以假响应保护坏接线（FACT）

- 正式 `GET /operation-knowledge/documents/:id` 返回 `{item:{camelCase document}}`，通用 `api.get` 不解包；DocumentsView 却把整个包当扁平 `DocumentDetail`。表单标题可由列表项回退而正常打开，但保存 URL 使用 `detail.id`，因此固定请求 `/documents/undefined`，且 rawContent/hash/索引均已回落为 `null/[]`。后端当前在 parse ObjectId 时 400，见 SR-131。
- `DocumentEdit.test.tsx` 直接把 `api.get` mock 为扁平 `FULL_DETAIL`，没有使用正式响应包络，所以它能断言 rawContent 被回传，却没有覆盖生产中 `detail.item` 才是正文的事实。该测试是有价值的“整替换不能漏字段”意图，但当前证明的是一个不存在的适配层。
- 本批未运行前端测试；结论来自冻结 TypeScript、Rust handler 和测试 mock 的确定性对账。运行现有测试即使全绿，也不会推翻 SR-131，因为 mock 已预先删除了出错包络。

### 系统级结论

- 列表项、详情包络与 mutation body 应由同一生成契约约束；类型断言不能承担运行时解包。对整替换写路径，客户端回传未编辑大字段是危险设计：服务端应以 PATCH/OCC 保留原文、hash、索引和租户身份。
- 前端专项测试必须复用真实 handler fixture 或共享 response schema。若测试为了方便把 `{item:T}` 改写成 `T`，它验证的是测试适配器而非生产 RPC，且会把确定性不可用路径伪装成防回归门。

## B06J：Knowledge 前端 Today、Atlas、Steward 与综合样式

覆盖文件：6 个，按既有非空逻辑行口径共 9,146 行（物理行 9,414）；Git 冻结 blob 合计 311,257 字节，台账 Windows/CRLF 内容口径合计 320,671 字节。包括 `today.tsx`、`atlas.tsx`、`steward.tsx`、`Knowledge.css` 与两份 Knowledge 综合测试；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，6 个冻结 blob 与台账 SHA-256 均保持不变。另定向反查 Chunk 列表过滤、Chat Task DTO/SSE、CoverageVerdict 下钻及既有 SR-100、SR-118、SR-123–125、SR-131；这些反查不重复结算全文覆盖。

### 有效接线与既有缺陷补证（FACT）

- Today 的 Chat attachment 使用正式 camelCase `{chunkId}`，任务详情/list 响应和 `event:turn` 的数字 data 与后端一致；TaskRail 专项测试覆盖列表计数和点选详情。任务的业务成功真实性仍受 SR-122–123 约束，Chat 派工测试只断言 `sessionId/plannedSteps/action`，明确未要求 `cardIds` 或目标绑定，继续补强 SR-125。
- Digest 画布直派会发送选中 cardIds，Chat 对话派工仍固定 `cardIds:[]`；后端允许空/未知卡片和客户端 target override 的候选身份问题仍归 SR-125。Observability 前端正确消费当前聚合响应，但“扫描命中率”与混合时间窗的指标语义继续归 SR-100；persisted/live catalog 形状漂移继续归 SR-118。
- Atlas 的 Domain Schema、元信息、发布/回退、治理列表和运营记忆请求形状与已审后端一致；本批未发现新的 RPC 漂移。`Knowledge.css` 是全局副作用样式，覆盖响应式布局、状态徽章、Inspector、Today/Atlas/Steward 面板；未发现由 CSS 新造的业务状态或请求语义。

### 评审队列的派生视图失真（FACT）

- `classifyChunk` 从不返回类型声明中的 `needs_review`，而 ReviewView 默认正选该类别，所以首次进入固定显示空列表；待审行实际被拆到 `source_orphan` 或 `pending_verification`。页面无 `status=active` 查询且分类器不检查 status，归档缺来源行会混入队列，与“仅展示活跃知识条目”文案冲突，见 SR-132。
- Cockpit 覆盖维度点击只把 dimKey 传到 ReviewView 横幅；加载、分类和 visible 结果完全不使用它。现有 CoverageVerdict 测试只证明 callback 收到 key，没有覆盖下游过滤，因此“定价/效果数据”等五个下钻实际同结果，亦见 SR-132。
- 本批未运行前端测试；结论来自冻结 TypeScript、Rust handler/query 与现有测试边界的确定性对账，不依赖运行时网络。现有测试即使全绿，也没有 ReviewView/classifyChunk 集成断言可反驳该结论。

### 阶段 6 结论

- 阶段 6 的全部 Knowledge 批次 B05K/B05L、B06A–B06J 已完成全文阅读；知识来源从导入/外部抓取到 Document/Chunk、revision、核验、Catalog、Answer Agent、Digest/Task、Gap/Feedback、前端治理与测试证据均已建立双向映射。本阶段发现截至 SR-132；这只表示阶段 6 阅读与证据结算完成，不表示全系统审查完成。
- Knowledge 的主风险集中于：租户/账号作用域不一致、审核状态可绕过、跨对象非原子提交、revision 不可恢复、候选/任务/评测缺少 immutable identity、RPC 契约漂移，以及 UI/测试把空结果或错误派生视图表示为成功。后续阶段 7–10 仍需把 Worker 舰队、前端其余频道、全量测试/脚本/部署与最终交叉验证闭环。

## B07A：Evolution 生产内核

覆盖文件：14 个，按物理行口径共 6,179 行（非空行 5,760）；Git 冻结 blob 合计 239,194 字节，台账 Windows/CRLF 内容口径合计 245,122 字节。包括 `src/evolution/` 下 `auto_release.rs`、`budget.rs`、`cohort.rs`、`envelope.rs`、`error.rs`、`lint.rs`、`mod.rs`、`post_release.rs`、`prompt_critic.rs`、`release.rs`、`replay.rs`、`runtime_flag.rs`、`significance.rs` 与 `threshold.rs`；均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，14 个冻结 blob 与台账 SHA-256 均保持不变。另定向反查 Evolution 模型/索引、管理路由、现有专项测试及 SR-049、SR-085–088；这些反查不重复结算全文覆盖。

### 运行链与有效边界（FACT）

- env `EVOLUTION_ENABLED` 是最外层熔断，Mongo runtime flag 再按 workspace/contact hash 灰度；缺 flag、关闭或读取失败均 fail-closed。唯一 worker 仍固定 default workspace/account，非默认租户功能黑洞继续归 SR-085。
- threshold 候选从 cohort 统计六类 gate，读取当前 active override、执行 cooldown/quota 和硬边界；prompt critic 有独立预算、schema/长度/禁词/自指校验。Shadow threshold 路径纯重判，prompt 路径走真模型短路链，不写发送/Outbox/MCP；其输入非冻结与 completed 即 eligible 的证据失真继续归 SR-049/SR-086。
- release 的生产写与 proposal 状态置于 Mongo transaction；threshold audit 已进入同一事务，Prompt 发布前还有禁词、锚点与 LLM 语义三闸。发布基线未冻结、旧 Prompt rollback 可越过后续版本，以及事件/review intent 位于事务外的缺口继续归 SR-086–088。

### Worker 与提交所有权（FACT）

- replay 并发 task 会吞掉单条执行错误和 JoinError；threshold grader 至少要求配置的 completed 数，少写通常会因样本不足拒绝。Prompt grader明确只要求 `completed>=1`，但这已是 SR-049 中“证据不具可比性仍可发布”的组成，不重复编号。`shadow_replays` 没有 `(proposal_id,source_run_id)` 唯一键，重跑的重复样本风险留待阶段 9 测试链交叉验证。
- 新发现 SR-133：threshold release 在事务外读 eligible，事务内 proposal update 不带旧状态 CAS，override 的 `source_proposal_id` 又无唯一约束。两个并发 release 可为同一 proposal 写两条 active override；rollback 的 `update_one` 只失效一条，却把 proposal 标成 rolled_back，残留覆盖继续影响生产。
- post-release review 仅是发布后的观测，不是“通过后才生效”的门禁；schedule/event 故障、多 scanner 与先 completed 后 event 的恢复缺口已由 SR-088 完整覆盖，本批不重复编号。

### 测试与系统结论

- 现有专项测试覆盖 Prompt 红线拒绝/正常发布、rollback 恢复历史状态与 workspace 过滤；没有 threshold 双 release、响应丢失重试、唯一 override 或 rollback 后 active=0 的并发集成断言。B07A 未运行依赖 Mongo/LLM 的集成测试；结论来自冻结生产代码、索引和测试边界的确定性对账。机械复核时 `cohort.rs` 的工作树原始 SHA 因 LF→CRLF 转换与台账不同，但规范化文本逐字相同，`git hash-object` 与冻结 blob `84291ced18c0c251da77e71c359c1170bb5f88ee` 一致；其余 13 文件原始 SHA 与台账一致，14/14 冻结 blob 均可解析，因此不构成基线语义漂移。
- Evolution 的候选、评估、发布和回滚应共享 immutable candidate/release identity：绑定 scope、base generation/hash、唯一 release outcome 与生成物 id；所有状态推进以旧状态/generation CAS。生产变更、审计 outbox 与 post-release intent 应形成同一 durable commit，后台评估再以 claim token/generation 收敛。

## B07B：Strategic Planner 生产扫描器

覆盖文件：1 个，物理行 3,733 行（非空行 3,498），即 `src/planner/mod.rs`；按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，冻结 blob 为 `74cf97048a05f62a2d44269c87d583c9eaef0b97`。工作树仅有换行/过滤后的原始字节差异，`git hash-object` 与冻结 blob相同且 `git diff` 为空，故内容身份成立。另定向反查 `main.rs` 启动、Planner 配置、AgentTask/AgentEvent 模型与索引，以及四份 Planner 专项测试的覆盖边界；这些测试文件仍保留待读，不在本批结算全文覆盖。

### 扫描链与有效规则（FACT）

- 唯一 loop 按 silent、commitment、stage_stagnation、calendar、renewal、reactivation 串行扫描；每段独立捕获错误。所有动作只创建 `review_required=true` 的 follow-up task，最终仍经 task worker、Gateway、Review 与 Outbox，不直接调用 MCP。
- silent 排除“Agent 已发但用户未回”和 cooldown；commitment 选择最早 overdue/imminent 条目并对无 due_at 的结构化承诺提供 fallback due；stage 停滞按 taxonomy 权重、价值层和停滞时长排序，并用 block-rate 回退。calendar 从结构化 memory date dimension 读取纪念日，renewal 从已核实 outcome 投影 entitlement，reactivation 按休眠时长、cadence 与 churn reason 生成任务。
- OperationMode 的有效解析顺序确为 contact override → relationship mode → profile default。renewal/reactivation 的扫描器级粗过滤却只查看 profile default/per-relationship；若未来正式写入“仅此 contact 开启”的 override，整段会在加载联系人前 return。当前仓库没有正式写入口，故本批把它记录为契约能力不可达限制，不单独编号；模型/设计若继续声明单客户全字段覆盖，应让粗过滤包含可索引的 override 候选或明确禁止这两个字段。

### Scope、幂等与配额（FACT）

- 新发现 SR-134：六段都固定使用 default workspace/account，进程开关没有租户枚举；非默认 scope 永远不产生 Planner 主动任务。
- 新发现 SR-135：pending/event/cap 都是读取后再插 task/event，Planner follow-up 没有唯一 intent。多实例可重复建任务并超发；calendar/renewal 在任务完成后同日下一 tick 可串行重发，三个独立 `daily_cap` 计数器又每 tick 从 0 开始，实际不是每日上限。
- Planner 复用的事件 helper仍固定写默认 workspace，属于已编号 SR-024；B07B 扫描本身也只跑默认 scope，暂时掩盖了错账的跨租户表现，但不能消除公共 helper 缺陷。

### 测试边界与系统结论

- 文件内单测对日期、阈值、排序、taxonomy 动态维度、OperationMode回落与默认关闭有细致纯函数覆盖。四份 Mongo 专项测试只串行调用 tick；calendar/commitment/silent 的“第二 tick不重复”依赖首条 task仍 pending或 commitment event，未消费任务后再重扫，也没有双实例、原子 quota、非默认租户或 contact-only renewal/reactivation override 测试。
- B07B 将运行 `cargo test planner::tests` 做局部验证；Mongo ignored 集成测试留到阶段 9统一执行。Planner 的生产发射应从“扫描后直接插普通 task”升级为 scope-aware、content-addressed intent + 原子日配额；通用 Task worker 的 claim/recovery正确性不等于上游任务创建幂等。

## B07C：Import、Cold Contact 与 Silence Signal Worker

覆盖文件：3 个，按冻结 blob 共 1,198 个物理行、1,119 个非空行、44,982 字节；分别为 `src/import_worker.rs`（300 行，blob `1300f4021ce3dbba1f65016d0351702b32e47472`）、`src/cold_contact_worker.rs`（581 行，blob `7c16dd8ab4dde3c0f470c2c441a38d3988ac1944`）与 `src/silence_signal_worker.rs`（317 行，blob `21773b286374162f0c73b18fc67beeb1b2fbcad6`）。三文件均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，工作树 `git hash-object` 与冻结 blob 相同。另定向反查 ImportJob/BehaviorSignal 模型、索引、导入路由、Webhook builder 与 `tests/import_job_lifecycle.rs`；这些反查文件不在本批结算全文覆盖。这里的对象 ID 已由 `git rev-parse <commit>:<path>` 机械复核；早期工作摘要中的三串 ID 有转写缺字，未据其判断内容身份。

### Import claim 与结果所有权（FACT）

- Import worker 只 claim `pending`，以 heartbeat 刷新 `claimed_at`，超时恢复把 `running` 改回 `pending`。模型没有 owner/token/generation；所有进度和终态写只验证 `_id + status=running`。
- 新发现 SR-136：A 超时后 B 重领并恢复为 `running` 时，A 的旧 heartbeat、progress、completed/failed 会再次命中同一状态，可替 B 续租或覆盖结果。现有 lifecycle 测试只覆盖状态处于 `pending` 的短窗口，没有覆盖 B 重领后的关键 ABA 窗口。
- 当前执行只生成导入预览，不直接提交 Document/Chunk；因此影响是重复 LLM 消耗和预览结果/错误非确定覆盖。导入 apply 的跨集合原子性仍归 SR-112，不在这里重复编号。

### Cold Contact、Silence 与租户边界（FACT）

- Cold Contact 只固定扫描默认 workspace，但会枚举该 workspace 的全部账号；Silence Signal 同样只扫描默认 workspace。它们与 Planner 的非默认 workspace 功能黑洞合并进 SR-134，而非为相同租户枚举根因另建发现。
- Cold Contact 在每个候选前实时重数当日事件，但“count→普通 follow-up task→event”仍是分步写，task 没有业务 intent key；并发副本可重复建任务和超配额。该证据扩展 SR-135。其 peer-case hook 只按 workspace 读取 verified knowledge，没有 account 可见域，扩展 SR-030。
- Silence Signal 的 `silence_signal_daily_cap` 是每次 tick 从零开始的局部计数，不是每日持久上限，扩展 SR-135。单条 signal 的唯一 dedupe key 可挡相同 key 重复插入，但不能把 per-tick cap 变成 daily quota。

### BehaviorSignal 身份与测试边界（FACT）

- 新发现 SR-137：Contact 身份是 `(workspace_id,account_id,wxid)`，BehaviorSignal 却只保存 workspace/contact；builder、dedupe key 与唯一索引均遗漏 account。同 workspace 两个账号共享 wxid/message id 或时间时会碰撞，未碰撞的历史样本也无法按账号归因。
- 当前信号消费者只做 workspace 级健康聚合，尚无学习链读取，因此没有把“未来可能训练串扰”写成既成事实；但持久身份降维与唯一键碰撞已经由代码确定。
- B07C 只运行三组无外部依赖的模块单测。`tests/import_job_lifecycle.rs` 全部 ignored 且依赖 Docker/Mongo，将留到阶段 9 ignored 集成测试；本批不会因定向阅读而把它在逐文件台账中标记完成。需要补充 A 超时、B 重领后旧 A 全部写失效的 barrier 测试，以及双账号同 wxid 的信号隔离测试。

## B07D：Behavior Signal 采集内核与 Worker Supervisor

覆盖文件：2 个，共 689 个物理行、638 个非空行、28,002 个冻结 blob 字节：`src/behavior_signals.rs`（504 行，blob `3345d2311408ccb3af33f378998c778ebee6127f`）与 `src/supervisor.rs`（185 行，blob `3e6b068aebf04857e497fd18e7cf26a60500bcd4`）。两文件均按冻结提交连续读取到 EOF，工作树 `git hash-object` 与冻结 blob 相同。另定向反查 `main.rs` 的全部 supervisor 接线、各长驻 worker 入口、事件 helper、模型/索引和行为信号调用方；反查文件不在本批重复结算。

### 行为信号采集契约（FACT）

- 四类 builder 分别持久化 reply latency、字符长度、reactivation 与 censored silence；event time 与 ingest time 分离，负延迟降为 None，首条 inbound 不臆造 reactivation，silence 恒为删失。`persist_signal` 只把 Mongo 11000/11001 解释为幂等命中，其余错误透传；健康指标按 workspace/UTC day 记录 persisted、dedupe_skipped、errors 三态。
- 全文确认 builder、dedupe key、持久模型和指标均没有 account_id；唯一键为 workspace+dedupe_key。这不是新的独立根因，而是 SR-137 的完整采集内核证据。现有单测锁定 snake_case、时间与 dedupe 稳定性，却没有同 workspace 两账号共享 wxid/message id 的隔离场景。

### Supervisor 存活语义（FACT）

- `spawn_supervised` 对 panic 使用 `catch_unwind`，记录日志和 `background_worker_panic` 事件后按 1→2→4→8→16→30 秒退避重建 future；运行满 60 秒后再次 panic 会把退避重置为 1 秒。future 正常返回时不重启。
- 对 `main.rs` 的全部生产接线逐项反查后，启用态 worker 都在自身无限循环内捕获单 tick 错误；可正常返回的分支仅是配置明确关闭或 interval=0。Outbox 的 `AppResult` 入口同样在 loop 内吞 tick 错，main 的外层 Err 转换不会形成已知可达的异常退出。因此“正常返回不重启”符合当前关闭语义，本批不新增发现。
- panic 事件调用公共 `write_event_for_account` 并传 account=`system`；该 helper 仍固定写 default workspace，所以非默认租户 worker 的 panic 无法归到真实 workspace，作为 SR-024 的补充证据。事件写失败被 best-effort 丢弃，至少 tracing error 仍保留；现有生产测试只用缩小版无 AppState loop 验证 panic 后计数重启，不验证真实 factory、事件写入、正常退出或 60 秒退避重置。

### 测试与系统结论

- B07C/B07D 已运行 `cold_contact_worker::tests` 12 项、`silence_signal_worker::tests` 9 项与 `behavior_signals::tests` 18 项，共 39/39 通过；Supervisor 模块测试在本批结算时单独运行。通过项证明纯函数与局部序列化契约，不覆盖跨副本 claim/quota、跨账号身份或真实 supervisor 事件归属。
- Worker 存活、执行所有权和业务身份是三层不同不变量：Supervisor 只能恢复 panic 后的 future，不能替 ImportJob 提供 fencing、替 follow-up 提供 intent/quota，也不能补回 BehaviorSignal 丢失的 account 维度。

## B07E：提示词治理内核与版本审计

覆盖文件：2 个，共 3,202 个物理行、2,985 个非空行、201,371 个冻结 blob 字节：`src/prompt_guard.rs`（409 行，blob `81c6bc950002364e240bdf52cb4075d18d2d3f29`）与 `src/prompts.rs`（2,793 行，blob `f08bee40e6dde2826b66ac4e4d04b14effc892cd`）。两文件均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；两份工作树 `git hash-object` 与冻结 blob 相同。另定向反查 Prompt CRUD/publish、Reply 生产加载与 DecisionReview 审计、Evolution release、模型/索引及既有 SR-007、SR-021、SR-049、SR-053、SR-055、SR-086–088；反查文件不在本批重复结算。

### Prompt pack 生命周期与破坏性兜底（FACT）

- 正常非空库路径先 GC archived，再逐 spec key 对齐；只刷新 `seeded_by=system` 的 current 行，manual/evolution 行保留，Evolution 链存在时整 key 跳过。空库则首次种 Souls、Templates、每账号 Playbook 与 Domain configs；代码默认 prompt 是数据库零 active 时的只读回落。
- 新发现 SR-138：用于判断空/非空的一次 `find_one` 只要报错，就直接进入与显式 reset 相同的四集合 delete/reseed。告警是同库 best-effort 写，整套 reset 无事务；瞬时读错后写恢复可删除整租户人工/Evolution/运营配置，中途失败还会留下混合态。现有 seeding 测试均从可用 Mongo 的确定性状态出发，没有“探测失败后零写入”断言。

### 三层编辑闸的有效边界（FACT）

- 禁词闸覆盖所有可编辑模板；锚闸只保护 `user.reply.policy` 的模式段/两条反接管短语和 `user.review.system` 的 few-shot；`evolution_critic_v1` 禁止编辑。Evolution release 只追加 snippet，天然保留原文锚，再经过同一字面闸和 LLM 语义闸。
- 新发现 SR-139：语义 diff 只收 new-only 行，纯删除固定得到空串并直接 Pass；`user.reply.system/task` 虽名为 constrained 却无锚，其它模板也只保护局部段。语义 judge 自身 `management.prompt_redline_review.system` 还是 freely editable，故可先以纯删除削弱审查器，再影响后续编辑。现有 17 项 guard 单测没有 deletion-only、无锚 constrained key 或 reviewer 自保护场景。

### 运行时选择与审计身份（FACT）

- `load_prompt` 选择最高 active；Reply 专用 `load_prompt_for_contact` 则先按 contact locale（无命中回落 zh-CN），再在同 locale active 集合中按 contact hash 稳定分桶。代码默认 fallback 返回 `version=None`。这些机制与 current pointer 是不同维度；status/current 分裂仍归 SR-055，候选基线/rollback 归 SR-086–087。
- 新发现 SR-140：Reply 三层 loader 已返回实际 version，但调用方以 `_..._version` 丢弃；写 DecisionReview 时重新调用不含 contact/locale 的 `prompt_versions`，统一记录最高 active。API 会展示该字段，因此 A/B/locale 实际处理组与运行审计确定性分裂。现有 bucket 单测只证明 hash 行为，不验证实际版本贯穿到审计。
- `user.reaction.task` 的默认枚举含文案未列出的 `user_replied_unclassified`，运行解析允许任意显式 status，且非默认域会追加 profile polarity 词表；这属于开放输出契约与运行时泛化设计，不在缺乏确定错误的情况下另编号。

### 测试与系统结论

- 顺序运行 `cargo test prompt_guard::tests` 17/17 与 `cargo test prompts::` 22/22，共 39/39 通过。最初并行运行因两个 Cargo 编译进程竞争并在 124 秒超时，已清理对应孤儿进程后单进程复跑成功；最终结论只采用成功复跑结果。测试覆盖锚漂移、禁词、三态 JSON、locale、hash 分桶和真实 pack schema，但没有 SR-138–140 的故障/删除/审计贯穿场景。
- Prompt 内容、发布身份与运行身份必须是同一个不可变 bundle：初始化读失败不能授权写，编辑审查必须看双向 diff 与完整 manifest，运行日志必须携带调用时实际选择快照。事后按“当前最高版本”重查，或用少数 substring 代替完整契约，都不能支持可恢复发布与可信 A/B 归因。

## B08A：前端运行骨架、导航、请求客户端与共享状态

覆盖文件：12 个，共 2,037 个物理行、1,915 个非空行、65,023 个冻结 blob 字节：`frontend/src/main.tsx`、`App.tsx`、`app/Shell.tsx`、`app/channels.ts`、`app/GlobalErrorBanner.tsx`、`lib/api.ts`、`stores/authStore.ts`、`stores/accountStore.ts`、`stores/navigationStore.ts`、`stores/uiStore.ts`、`stores/profileStore.ts` 与 `types/index.ts`。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，12/12 工作树 `git hash-object` 与冻结 blob 相同。另定向反查 auth/workspace/account 路由、Chunk WebSocket、Operations/Event 投影、DELETE 成功包络、共享 store 调用方与入口专项测试；反查文件不在本批重复结算。

### 登录、workspace 与请求边界（FACT）

- 启动先以 `/api/auth/me` 恢复 session，成功后把 admin/workspace 与 logout/switch handler 注入 authStore；切 workspace 由服务端 ACL 校验并更新 session，成功后整页 reload。全局 fetch wrapper 对除 login 外的 `/api/*` 401 广播过期事件，普通 API helper 统一解析 JSON、LLM unavailable 与短纯文本错误。
- 后端账号列表和所有本批反查业务路由均使用 session 的 `current_workspace`；Chunk WebSocket 也在服务端按连接建立时的 workspace 过滤事件。未发现这条前端骨架新增的跨租户越权。`originalFetch` 路径绕过统一错误解析、切换失败静默，以及 `selectedAccountId` 在列表变化后只派生回退而不持久收敛，均记录为韧性/可观测性债务；现有调用方普遍使用 `currentAccountId()` 或同形有效值计算，本批没有把它们外推成确定错租户写入。

### Channel、共享投影与占位边界（FACT）

- Shell 以 CHANNELS 为单一注册表懒加载 20 个一级频道，按运营/知识/系统分组；当前没有频道启用 `visibleWhen`。`groupOps` 与 `momentOps` 明确映射 Overview，并在仓库规划、烟测和代码文案中声明 Phase 2 占位，因此不把尚未实现本身编号为生产回归。
- Account、Event、DecisionReview、PromptTemplate 和 OperationDomain 共享类型与正式 JSON 投影定向对账后，本批检查字段匹配；所有正式 `api.delete` 调用对应后端均返回 JSON 成功包络，不存在通用 helper 因 204 空 body 固定抛错的问题。全局 `uiStore.busy` 是非引用计数布尔值，多动作并发时存在提前清 busy 的设计脆弱性，但本批未找到必须并发且造成错误提交的封闭复现，留到具体页面批次按调用链判断。

### Chunk 事件连续性（FACT）

- 正常 `locked/unlocked/revised` 事件由 App 转成 window CustomEvent；Inspector 只在 revised 的 `chunk_id` 命中当前详情时重拉，锁 hook 则监听 lock 事件并每 60 秒续租。
- 新发现 SR-141：服务端 broadcast 落后时发送 `lagged` 表示事件已不可恢复，前端却直接忽略。Inspector 内容没有轮询或 snapshot generation，故被漏掉的 revision 不会因锁 heartbeat 自愈，可无限期停留旧视图。现有测试只覆盖事件 shape、正常 late subscriber 语义和 Inspector 局部动作，没有 lag→全量 resync。

### 测试与系统结论

- 运行 Shell、accountStore、profileStore 与 apiPatch 四个专项文件，共 14/14 通过；随后 `npm run build`（`tsc && vite build`）成功，1,860 个模块完成转换。通过项证明当前静态类型、导航基本渲染、workspace switch callback、账号有效值回退、taxonomy label 与 PATCH 请求形状，不覆盖 WebSocket lag、登录/切租户失败或共享 busy 并发。
- 前端事件缓存必须显式区分“收到增量”与“增量历史已断裂”。正常事件处理正确不代表缓存最终一致；只要协议提供 `lagged`，客户端就必须将其解释成全局 invalidation 并以 snapshot/generation 收敛。

## B08B：工作台、账号管理与 AI 总控

覆盖文件：15 个，共 1,796 个物理行、1,628 个非空行、78,028 个冻结 blob 字节：Overview、Account Management、Command Center 三条页面实现及样式，`commandStore.ts`、`contactStore.ts`，以及三份对应专项测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，15/15 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 account/contact/management handler、MCP structuredContent 解包、账号切换器、User Ops/Roster 的切号实现与既有 Management 发现；反查文件不在本批重复结算。

### 账号作用域与共享前端状态（FACT）

- Overview 首次进入会按当前 account 拉联系人，但把“全局 contacts 数组非空”当作缓存命中；`contactStore` 没有 owning account、generation 或切号 invalidation。A 已加载后切 B 不会请求 B，A/B 并发请求的迟到结果也可互相覆盖。Command Center 的托管人数直接消费同一数组。新发现 SR-142：工作台统计、实时运营流与总控 scope 会在切号后稳定显示旧账号联系人。
- Command Center 的 assets/tasks 会随 account 重拉，`commandResult` 却保持单个全局对象且类型不含 accountId。A 的 `pending_confirmation` 在切到 B 后仍显示；confirm 只发送 run id，后端按 workspace+id 认领并明确使用持久化 `run.account_id=A` 真执行。新发现 SR-143：确认时可见的新账号 scope 与实际副作用账号分裂。
- `McpKeyForm` 在同一 React 位置跨账号复用，account prop 变化不会清空 key/baseUrl 等本地 state。A 未提交的明文密钥可在 B 页面通过新 URL 保存到 B；后端按 B object id 正常提交，无法恢复草稿来源。新发现 SR-144。

### 账号登录与公开 DTO（FACT）

- 账号列表、同步和 MCP key 更新均由后端按当前 workspace 过滤；列表只暴露 `mcpKeyConfigured` 布尔，不回显密钥。同步仍继承既有 SR-011 的默认 MCP 凭证/日志作用域问题，本批不重复编号。
- 新发现 SR-145：MCP client 从 JSON-RPC 中取 `result.structuredContent` 后原样返回，账号 handler也无 DTO 转换；仓库部署契约声明 `session_id/qr_data_url/login_page_url`，前端却读取 `login_session_id/qr_code_base64/login_page_url`。session id 因此为空，二维码、登录链接、轮询和成功后同步整条链均不启动。
- 登录轮询的 pending/success/expired 分支、取消 timer 与成功后 best-effort sync 在前端局部逻辑上完整；但没有 AccountLogin 测试，现有类型断言不能校验运行时字段。账号管理列表本身按 API items 渲染在线、MCP 配置和状态，未发现新的写动作越权。

### 测试与系统结论

- 运行 Command Center、McpKeyForm 与 Overview 三份专项测试，共 14/14 通过；随后 `npm run build`（`tsc && vite build`）成功，1,860 个模块完成转换。测试只覆盖单账号静态渲染、确认按钮存在、状态标签、单次密钥提交和已有联系人概览；没有账号切换、迟到响应、密钥草稿复用或真实登录 DTO fixture，因此不反证 SR-142–145。
- 账号选择不是页面装饰，而是所有展示缓存、草稿、确认 intent 与秘密输入的身份维度。任何可跨账号存活的前端状态都必须携 scope，并在提交时由服务端复核 expected scope；仅让后续 GET 使用新 accountId，不能使旧状态自动归属于新账号。

## B08C：用户运营数据层、联系人池与通讯录

覆盖文件：11 个，共 3,087 个物理行、2,779 个非空行、122,062 个冻结 blob 字节：`frontend/src/features/user-ops/index.tsx`、`RosterView.tsx`、`RosterView.module.css`、`poolHelpers.ts`、`frontend/src/stores/userOpsStore.ts`、`userOpsDomainHelpers.ts`，以及 User Ops 入口、Roster、pool helper、Store、健康投影五份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，11/11 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Contacts/Operating Memory/Playbook/Domain handler、真人准入 helper、Cockpit 保存调用方与既有 SR-069、SR-070、SR-094、SR-142；反查文件不在本批重复结算。

### 账号切换、列表与详情身份（FACT）

- User Ops 入口正确订阅派生后的 accountId 字符串，切号时清 `selected` 并重拉联系人、计数和 Playbook；Roster 更按 account 缓存、用组件请求序号丢弃迟到响应，并在切号/刷新时清勾选、共享备注与 Playbook 选择。对应 Roster 竞态测试直接覆盖 A 慢/B 快，说明该身份纪律是产品既有要求。
- 主联系人链未复用这套纪律：`refreshContacts` 无 generation/loadedAccount，任一账号迟到结果都写同一个 `contactStore.contacts`；`loadContactCounts` 和 `loadPlaybooks` 也无 scope guard并写单个全局投影。该证据扩展 SR-142：User Ops 虽会发 B 请求，A 响应仍可在随后覆盖 B 的列表、计数或方法列表。
- 新发现 SR-146：联系人详情 `loadMessages(contact)` 并发加载五块数据后无条件写全局详情态。A 响应晚到可把 A 的 `memoryDraft` 放到当前选中 B 的编辑器；保存时重新取 `selected=B` 并把全量四组 A 记忆 PUT 到 B。后端按 URL 中 B 的真实身份正常提交，因此这是可持久错写，不只是旧视图。

### 通讯录准入与 Playbook 草稿（FACT）

- Roster API 已公开 `isNonHuman`，前端将系统号折叠展示；就绪缓存按账号隔离，`syncing=true` 不落缓存并以 10 秒只读轮询收敛，force 只触发后台单飞刷新。分页、性别透传、managed 禁选与同步提示均有直接测试。
- 新发现 SR-147：系统号展开后仍可勾选，提交又丢弃 `isNonHuman`；batch-enable 后端只拦账号自身，不调用现有 `is_operatable_person`。已知 `fmessage/weixin/gh_/@chatroom/@openim` 因而可被写成 managed 并入队 initial_profile。测试只验证默认折叠与展开可见，没有断言拒绝纳管。
- 新发现 SR-148：`editingPlaybookId/playbookDraft` 是全局单份，切账号只替换列表。A 草稿留在 B 页面后，保存/设默认仍以旧 id 操作；后端 update/set-default 只按 `_id+workspace` 找资源并采用资源自身 account，故真实改写 A。Playbook 直接进入生产且无 draft/publish 隔离仍归 SR-070；生成/优化的 `prompt` 对 `description/instruction` 与 `{id}` 对 `{item}` 漂移继续归 SR-069。

### Domain 配置、测试与系统结论

- Domain helper 能稳定往返已有 runtime/state-machine 文本，但解析器允许任意 key、负数、极大值与字符串；非法 JSON 静默回落 `{}`。正式 PUT 的 runtime 仍是裸 Document，后端只校验说明字段和状态机结构。这是所有人工/Guide 写入口缺统一 typed schema 的同一根因，已补强 SR-094，不另编号。
- 运行五份 B08C 专项测试共 40/40 通过。通过项证明入口模式渲染、Roster 的单组件竞态守卫/缓存/分页、pool 时间纯函数、单联系人 Store URL/body 和 Guide health 投影；没有主列表迟到响应、联系人详情交错后保存、系统号提交或 Playbook 切号编辑测试，因此不反证 SR-142、SR-146–148。
- 前端状态的身份必须与数据一起传播：请求参数带 account/contact 只能约束服务器读什么，不能约束响应回来时写进哪个当前视图；同理，保存时重新读取“当前 selected/account”会把无身份草稿重新贴到错误目标。列表 cache、详情 snapshot、编辑 draft 和资源 id 都需要同一 scope/generation，并由服务端以 expected identity/version复核。

## B08D：智能驾驶舱、联系人观测与配置

覆盖文件：25 个，共 2,944 个物理行、2,721 个非空行、116,438 个冻结 blob 字节：Cockpit 外壳、观测/配置/判断条、四个下钻视图，标签可信度、贝叶斯走势、OCEAN 人格组件及四份样式，以及 10 份直接组件测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，25/25 工作树对象与冻结 blob、台账 SHA-256 一致。Cockpit 复用 `legacy.tsx` 的展示 helper；该大文件及直接验证它的 Contacts/Planner/SendHistory 测试留到传统模式批次。另定向反查 User Ops Store、Contacts/Guide/Review handler、Playbook runtime loader 与终审写入链；反查文件不在本批重复结算。

### 驾驶舱结构与只读投影（FACT）

- Cockpit 以“观测/配置”段控组织联系人详情，并提供记忆、会话复盘、发送历史与人格/置信走势下钻；配置区再拆画像、指令、记忆和工具四个 tab。长期记忆下钻兼容结构化事实与旧字符串形态，标签面板物理区分人工权威、AI 确信和贝叶斯旁路，人格低置信维度会显式弱化。
- DecisionReview 列表按 `created_at desc` 返回，常驻判断条取第一条；终审状态、hold category 与九字段自治协议从同 run log 投影。生产 Gateway 在硬门和 revision 失败时会同步把 `review.approved=false`，因此未把“通过/终审拦截”列为新发现。健康度后端已返回 quiet-hours 三字段，前端的兼容断言能消费它们。
- 状态机当前将 `operation_state` 与 `customer_stage` 维持同一 canonical id 空间，Cockpit/复盘按 customer_stage taxonomy 翻译未形成独立错误。图表对 score/confidence 做显示 clamp；本批未发现由 SVG 或样式造成的业务写入风险。

### 联系人草稿与候选确认身份（FACT）

- 新发现 SR-149：`TagTrustPanel` 在同一 React 位置跨联系人复用，A 的 `editing/draft` 不会随 `contact.id` 重置；保存回调只提交无身份 tags，Store 再读取当前 B 并覆盖 B 的权威 `manual_tags`。该链不依赖网络竞态，现有测试只覆盖单联系人编辑。
- 新发现 SR-150：Guide preview 请求虽捕获 A 的账号/联系人，响应却无 generation/selected 校验；A 的迟到候选可在 B 页面出现。确认只提交 preview id，后端正确按候选冻结的 A scope 执行，因此可见确认对象与真实副作用对象分裂。SR-093 继续描述候选内部全局字段/摘要不可信，本项聚焦候选自身跨联系人错位。
- 其它画像、特别指令、辅助模式、运营画像和记忆保存仍复用 B08C 已审 Store。它们的无身份全局草稿与详情迟到响应共同受 SR-146 约束，本批不为每个按钮重复编号。

### Playbook 绑定与复盘账号 scope（FACT）

- 新发现 SR-151：Cockpit 的“运营风格模板”下拉只改 `selectedPlaybookId` 与页面摘要。未托管联系人启用请求不发送后端已支持的 `playbookId`，已托管联系人也没有任何换绑请求；运行时只读持久化 `contact.playbook_id`，故该正式控件确定性不生效。
- 新发现 SR-152：详情 Store 请求 DecisionReview 时只传 contactId；后端却先把缺失 accountId 回退为进程 default account，再将目标联系人的 wxid 加入 filter。非默认账号因此稳定查询错误 scope，通常显示空复盘；默认账号存在相同 wxid 时还会串入其记录。当前 Store 测试反而锁定了这个缺 accountId 的 URL。
- Playbook 选择不落库与传统 Playbook CRUD 的跨账号编辑（SR-148）是不同链；Playbook 原地覆盖、无冻结版本仍归 SR-070。复盘内容本身的 Prompt A/B 审计错记继续归 SR-140。

### 测试与系统结论

- 运行 10 份 B08D 专项测试，共 38/38 通过。通过项证明四级 tab/下钻、健康 tone、终审 tone、记忆溯源、自治协议、标签来源、人格与贝叶斯图表的单对象渲染；没有联系人切换、Guide 迟到确认、Playbook 网络持久化或非默认账号复盘测试，因此不反证 SR-149–152。
- 联系人详情中的本地 state、全局 Store、异步候选和服务端资源必须共享同一个身份包络。控件改变可见字符串不等于配置已持久化；服务端冻结资源身份也不等于用户确认时看见了同一身份。所有响应提交、草稿保存与确认 claim 都应校验 workspace/account/contact/version/generation。

## B08E：传统用户运营视图与运营池

覆盖文件：4 个，共 2,341 个物理行、2,179 个非空行、93,579 个冻结 blob 字节：`frontend/src/features/user-ops/legacy.tsx`，以及 ContactsView、PlannerView、SendHistory 三份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，4/4 工作树对象与冻结 blob、台账 SHA-256 一致。`legacy.tsx` 复用 B08C/B08D 已结算的 Store、标签/人格组件与 pool helper；另定向反查 User Ops 编排、batch-enable handler、Contact 时间投影、领域信号写侧、生产 Planner 和发送台账 API，反查文件不在本批重复结算。

### 运营池选择身份与真实写入（FACT）

- ContactsView 的勾选集合是组件本地 `selectedWxids`，没有账号 prop 或切号清理；父级切号只清详情 selected，并异步重拉列表。新发现 SR-153：A 中已勾选的旧列表在 B 请求完成前仍可操作，提交时组件从 A 列表组装候选，父回调却使用当前 B accountId；后端按 `(workspace,B,A.wxid)` 正常 upsert managed Contact 并创建 initial_profile，形成真实错账号纳管。主列表无 generation 的 SR-142 会延长或重新制造该窗口。
- 单人启用和批量启用共用同一无来源候选形状；候选只携 wxid/昵称/备注/头像/性别，不携 source account 或 roster generation。后端验证目标账号存在、不是账号自身并解析 Playbook，但不证明 wxid 属于目标账号的通讯录。系统号准入缺失继续归 SR-147，任务与 managed 非原子继续归 SR-082，本批不重复编号。

### Planner 与发送历史解释面（FACT）

- 新发现 SR-154：PlannerView 从 `domainAttributes.customer_stage` 显示阶段，却用容器级 `domainAttributesUpdatedAt` 声称“自此未变更”。任意 intent、relationship、value tier、授权或请示标记写入都可刷新容器时间；生产 Planner 实际按 `customer_stage_updated_at` 或可配置 `<stagnation_dimension>_updated_at` 判断停滞，前端解释与真实决策依据分裂。模型已同时投影 `operationStateUpdatedAt`，测试却只手工构造容器时间，因而锁定了错误来源。
- SendHistorySection 能正确区分加载失败与真实空历史，并用 effect cleanup 阻止旧 wxid 响应在切联系人后提交；但 API 和 handler 只按 workspace+wxid 查询，忽略台账已有的 accountId。该 UI 是既有 SR-050 的直接消费面，不另编号。ConversationStream、记忆事实兼容、taxonomy fallback 与状态标签的本地渲染未发现新的可达写入错误。

### 传统配置与测试结论

- Playbook 编辑跨账号身份继续归 SR-148，Cockpit 选择不落库归 SR-151；Domain runtime/state-machine 的开放 Document 与静默 JSON 回落继续补强 SR-094。Prompt/Soul 编辑最终走已审 Strategy Store 和红线发布链，本批没有新增独立结论。
- 运行 ContactsView、PlannerView、SendHistory 三份专项测试，共 15/15 通过。通过项覆盖单账号运营池文案/按钮、taxonomy 多维展示、发送历史失败与真实空态区分；没有账号切换后的勾选提交、列表迟到响应、候选来源校验，或专用阶段时间戳与容器时间分离测试，因此不反证 SR-153/154。
- 列表快照、选择集合和提交目标必须形成同一个不可变 scope；观测解释也必须引用生产决策使用的同一事实字段。仅在父组件读取“当前账号”，或用一个宽泛的容器更新时间配对某个具体维度，都会把不同时间、不同身份的数据拼成看似一致的界面。

## B08F：Operations 运行观测、任务动作与运营域契约

覆盖文件：24 个，共 1,736 个物理行、1,630 个非空行、62,642 个冻结 blob 字节：Operations 页面与样式、`operationsStore.ts`、两份直接专项测试，以及运营域契约对账测试与其引用的 9 组 fixture/contract（Behavior Signal、Outcome、LLM Call、Memory Candidate、Operating Memory、Agent Run、Decision Review、Guide Preview、Operation Health）。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，24/24 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Events/Tasks/Reviews handler、AgentTask 模型与 Gateway task 执行链；反查文件不在本批重复结算。

### Operations 账号身份与任务动作（FACT）

- 页面正确订阅派生后的当前 accountId，并在变化时重拉 events/tasks/reviews/llm/runs；五个读端后端也都按 current workspace 与传入 account 过滤。AgentRun、DecisionReview 和 LLM Call 的顶层投影与冻结 fixture 键集一致，事件 detail 通过 relaxed extjson 下发。
- Store 却只有一份无身份全局快照，切号不清旧数据，任一 `Promise.all` 迟到结果都可覆盖当前账号。新发现 SR-155：A 的旧任务显示在 B 页面时，动作只提交 task id；review-now/cancel 后端仅校验 workspace，不校验当前 account，前者会按任务持久化的 A scope 真运行 Gateway，后者会直接取消 A 任务。运行中取消后的 lease/fencing 缺口继续归 SR-034，不重复编号。
- Event、Task 类型没有 accountId；DecisionReview 虽然后端投影 accountId，前端业务类型也省略该字段。AgentRun 带可选 accountId，LLM 单条日志投影也带 accountId，但列表不展示。契约键集测试能发现投影加减字段，不能证明 Store 响应提交身份或写动作 expected scope。

### LLM 成本窗口与运行解释（FACT）

- 新发现 SR-156：成本页把 summary 标成“调用次数/总 token”，但后端只读取最近默认 100 条日志后在内存求和；没有全量 aggregation、窗口标签、截断标记或分页游标。超过 100 条后累计成本必然低报，且滚动窗口值可能下降。
- Run 页从 decision Document 提取 `sufficiency/missingTier` 展示档位遥测，并通用渲染 planner/context/knowledge/review/gateway 文档；冻结 AgentRun 契约只锁顶层键，不锁这些开放 Document 的子结构。复核页动态遍历 scores，未知维度不丢弃，并从关联 run 补 finalReviewStatus/holdCategory；本批未发现新的确定字段错配。
- Operations 的“立即复核”使用后端 CAS 从 pending/retry/failed 领取，可阻止已运行/终态任务重复 claim；但无 heartbeat、cancel token 和 generation 的并发风险已在 B04F/SR-034 完整记录。本批新增问题是可见账号与被领取任务账号分裂，不依赖并发执行器缺陷。

### 测试与系统结论

- 运行 Operations 页面、Store 与运营域契约三份测试，共 31/31 通过；随后 `npm run build` 成功，1,860 个模块完成转换。测试覆盖单账号加载/错误横幅、任务按钮端点、复核四分支、Run 档位遥测和 9 组顶层键集，不覆盖账号切换、迟到响应、旧 task 写动作或超过 100 条的汇总语义，因此不反证 SR-155/156。
- 运营观测面同时包含只读快照和高影响命令。快照身份不能只存在于发请求时的 query，也必须约束响应提交和后续动作；汇总标签则必须公开统计窗口。否则界面即使字段契约完全匹配，仍会在“操作谁”和“统计多少”两个核心语义上失真。

## B08G：Campaign 活动列表、圈人预览与结果看板

覆盖文件：19 个，共 1,313 个物理行、1,193 个非空行、57,757 个冻结 blob 字节：Campaign 频道入口、列表/创建/结果看板、产品与阶段选择器、桶/CSV helper、样式、`campaignStore.ts`，以及 9 份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，19/19 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Campaign REST/Management handler、Campaign/Send 模型、账号切换骨架与联系人 roster 身份来源；反查文件不在本批重复结算。

### 活动规范、账号作用域与确认边界（FACT）

- 创建页首轮预览会把当前账号、标题、意图和圈选条件保存为 Campaign；后续 preview 与总控 dispatch 都只消费该持久对象。正式前端修改条件后保留旧 draft id、仅发空 preview body；标题/意图变化甚至不清旧预览。该链与 B05E 的 SR-077 完全同根，本批补强其正式页面与测试证据，不重复编号。
- `create.test.tsx` 明确锁定“改条件后复用同一 draft、不再 create”，并用手写连续 mock 直接返回 42→8 人，没有驱动真实后端验证持久条件发生变化；因此 42/42 Campaign 测试通过不反证 SR-077，反而说明当前测试把错误网络协议当成功路径。
- 真发送不在 Campaign 页面暴露按钮，只能由 Management 的 `dispatch_campaign` 工具执行；该工具仍属于少数代码恒确认项。看板跳转只接受 `succeeded/executed_unverified` 且 response 含字符串 campaignId 的真实工具结果，dry-run/待确认不展示死链。
- Campaign 模型本身是 workspace+account 级，创建会读取当前账号；但列表 handler 固定返回 workspace 全量活动，列表 DTO 又省略 accountId，账号切换不会刷新或区分列表。由于同 workspace 管理员本就可读这些活动，且产品是否有意提供跨账号总览不够明确，本批将其记录为解释面作用域事实，不在缺乏明确产品契约时升级为独立发现。

### 报表快照与异步身份（FACT）

- 新发现 SR-157：Store 只有单份 `selectedCampaignId/report/loading/lastAttemptedId`。连续打开 A、B 会并发加载；迟到 A 响应无 generation 或 id 校验，能在 selection=B 后覆盖 report。看板也不比较 `report.campaignId`，因此 A 的标题、统计、逐人 wxid 与原因会稳定显示，CSV 文件名却由 B id 生成。
- 报表后端先按 workspace 找 Campaign，再按 campaign id 取 Sends、按 Campaign 自身 account 补联系人名，并回显 campaignId；单次响应身份是完整的。错位完全发生在前端响应提交层，说明后端回显 identity 若不在 Store commit 与渲染时校验，无法保护最终人机语义。
- `lastAttemptedId` 只防单个失败请求在 effect 中自旋，不是请求所有权；多个 `openReport` 仍可同时在途。分页是 Store 全局值、筛选是组件本地值，打开新活动只重置页码不重置筛选；后者会沿用相同桶过滤，但不会改变数据身份，本批不单独编号。

### CSV 边界、测试与系统结论

- 新发现 SR-158：CSV helper 只做逗号/引号/换行转义，不中和 `= + - @` 公式前缀。客户名来自 Campaign report 的 `remark|nickname`，其中 nickname 由外部微信 roster 提供；下载后在公式型表格软件打开可把外部身份数据解释为公式。纯函数实测四类前缀均原样保留，现有测试只证明 RFC 4180 语法转义。
- 七桶标签、原因映射、50 行内存分页、空态、状态 tone 与无前端 dispatch 红线均有直接测试；运行 9 份 Campaign 测试，共 42/42 通过。没有双报表 deferred response、report id 不一致、真实 draft update 或公式前缀测试，因此不反证 SR-077、SR-157/158。
- 活动的安全边界不只在最终 dispatch 确认门。确认前展示的 audience spec、确认后查看的 report snapshot 和导出的离线文件都必须携同一不可变身份与内容语义；请求参数正确、后端单响应正确或 CSV 语法合法，都不能替代前端提交 generation 与消费端安全编码。

## B08H：自治回路、运营成效与发送成效

覆盖文件：16 个，共 2,521 个物理行、2,345 个非空行、96,948 个冻结 blob 字节：Autonomy 频道与 Outbox 面板、Quality 频道与评测场景面板、Send Analytics 频道及 `sendAnalyticsStore.ts`，以及 Autonomy/Quality 的 5 份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，16/16 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Autonomy/Outcome/Outbox、Evaluation/Auto-verify、Send Ledger 与 Prompt publish handler，以及既有 SR-050、SR-055、SR-066、SR-098、SR-099、SR-107；反查文件不在本批重复结算。

### 自治快照、账号切换与取消动作（FACT）

- 自治指标和改写记录的两个后端端点都按 current workspace、传入 account 与 horizon 过滤并回显 accountId；Outbox 列表也按 workspace+account 过滤，单次响应 scope 正确。前端三份本地快照却均无 owning account、generation 或切号清空，A/B 迟到响应可覆盖当前视图，旧 A 数据在 B 请求完成前也持续可见。
- 新发现 SR-159：Outbox 前端丢弃后端已回显的 accountId，A 的 pending/in_flight 行切到 B 后仍有“取消”按钮。请求只携旧 id，后端取消 CAS 只校验 `_id+workspace+status`，不校验 expected account，因此会真实取消 A。in-flight 取消后仍可能物理送达继续归 SR-066；本项按 pending 行也能闭合的错账号取消定级。
- 自治指标的窗口标签和后端 `created_at>=since` 一致，null 比率也明确显示为“—”；Planner 三段计数按相同 workspace/account/horizon 聚合。未发现新的统计口径错配。现有 Autonomy 测试固定一个 mock 账号，只验证文案、标签、数值和一次 id-only 取消，不能反证账号竞态。

### 运营成效治理链（FACT）

- 长期指标端点按 workspace+account+horizon 过滤，DTO 也回显 accountId；前端仍无 generation，切号期间可显示或迟到提交旧账号行。Auto-verify 与公式评测结果同样是单份本地 state，账号变化不清旧结果；这些只读错贴与 SR-159 属同一前端身份纪律，但没有额外写动作，本批不拆低影响重复编号。
- 知识自动校验正式请求已使用后端 camelCase DTO，并且服务端把所有模型自称 verified 强制降为 `needs_human_audit`；AI 不自动放行红线保持。其 committed 计数先增、revision 写失败被吞的既有问题仍归 SR-107。
- 评测场景 UI 创建只发送 scenarioId/title/description/inboundMessages，后端缺省存 active 且 groundTruth 为空；公式 runner 又不按 scenario.accountId 过滤。这是 SR-098 的正式消费证据，不重复编号。批次 token 从并发生产 run 日志误取的预算问题继续归 SR-099。
- 产品声明标记词编辑实现了 update 与 publish 两段各自的 `needs_human_confirm/reject→逐字确认→force` 三态流程，专项测试覆盖 update 的 200/4xx 两类确认。底层手工 publish 的 active/current 分裂和物删历史仍归 SR-055；该配置页面已明确标注守卫当前未启用，不把“不影响生产”误报为已生效。

### 发送成效口径、测试与系统结论

- Send Analytics 前端完全不读取顶部账号，Store 请求也不带 accountId；后端 overview/stats 固定按整个 workspace 聚合，并按 targetId 合并素材/名片。Send Ledger 行虽保存 accountId，统计、历史、归因和唯一锚多处遗漏该维度的完整根因已由 SR-050 覆盖，本批只补正式排行榜和总览消费面。
- 排行率以 outcome 已评估条目为分母，新发未过窗口条目不拉低响应率/推进率；总发送数则是全量 ledger 行。前端说明没有声明当前账号口径，故 workspace 总览本身不另定产品契约问题；但在多账号系统中无法下钻账号、同 target id 合并和重复 ledger 计数均继续受 SR-050 影响。
- 运行 Autonomy/Quality 5 份直接测试，共 13/13 通过；随后 `npm run build` 成功，1,860 个模块完成转换。测试覆盖单账号静态渲染、状态中文化、Outbox 请求形状、评测场景基础 CRUD 和 Prompt update 确认，不覆盖账号切换、迟到响应、expected-account 取消、真实场景 ground truth、公式预算或 Send Analytics 多账号数据。
- 观测页面里的账号选择不仅约束 GET 参数，也必须约束响应提交和后续动作。服务端回显 accountId 若被前端 DTO 丢弃，或写端只接受资源 id，就无法证明操作者当前看到的 scope 与被取消/确认的资源 scope 相同；workspace 级统计则必须明确其聚合维度并保留 account 下钻能力。

## B08I：内容资产、产品成交与专属顾问

覆盖文件：12 个，共 3,191 个物理行、2,970 个非空行、123,572 个冻结 blob 字节：Content Assets 页面/Store/样式/测试、Products & Deals 页面/样式及两份直接测试，以及 Referral Cards 页面/Store/样式/测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，12/12 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Content Asset/Media Asset、Product、Suspected Deal、Referral Card 与 Deal Event handler，运行时素材/名片准入、Shell 挂载行为及既有 SR-024、SR-050、SR-051、SR-057、SR-058、SR-065、SR-067；反查文件不在本批重复结算。

### 内容资产账号身份与生产写入（FACT）

- 列表后端按 workspace 查询，并在给出 accountId 时合并 workspace 共享行与该账号私有行，且正式响应包含每行 accountId。前端类型却丢弃 accountId，Store 只有单份 assets；切号不清旧列表、无 generation，A/B 迟到响应可互相覆盖。
- 新发现 SR-160：A 私有行在 B 页面继续可编辑、审核、启停、换文件和删除；前端动作只传 id，所谓 accountId 只用于事后 reload。五类后端写 filter 均为 `_id+workspace`，因此会真实修改 A 的 prompt/禁语/销售素材生产状态，删除还可能清理物理文件。
- 上传路径仍固定 draft，换文件固定退 draft 并清 mediaId；Agent 的文本注入、禁语、可发素材加载均按 workspace 加 `account_id=null|当前账号`，发送前准入未被绕过。问题发生在管理员选错生产资源，而非 Agent 忽略资源账号。上传/换文件先落盘后写 DB 的孤儿文件问题继续归既有 SR-017。

### 产品、成交与高可信事实（FACT）

- 产品目录被后端与模型明确设计成 workspace 级，前端不随账号重拉符合现有语义；金额元/分转换、非负校验、币种闭集、active/archived 和 reversal 必须关联产品均对齐。Reply 只靠自报 productId 背书价格的运行时缺口继续归 SR-051。
- 新发现 SR-161：DealsTab 选择的 A 联系人保存在父组件本地，切 B 时 ContactPicker 只换自己的候选列表，不清父 selected；Shell 也不重挂载频道。旧表单继续按 A contact object id 调正式写端，后端只按 workspace 找联系人并向资源自身账号 append `staff_confirmed/payment_verified` 成交，因此 B 上下文可稳定改写 A 的持有、LTV 和运营事实。
- 疑似成交队列是明确的 workspace 级审核池，列表和动作不读取顶部账号，本批不把它误报为切号竞态。其 CAS-first 导致 approved 后落成交失败不可重试仍归 SR-057，客户端 reviewedBy/审计主体问题归 SR-058，Management 可直写 staff_confirmed 归 SR-065，未进入统一收件箱归 SR-067。

### 专属顾问、测试与系统结论

- Referral Card 列表也是明确的 workspace 全量管理面，DTO 保留 accountId、运行时候选和发送前二次准入均要求 `account_id=null|contact.account_id` 且 `enabled+approved`；新建固定 draft+disabled。顶部账号只决定新卡绑定 scope 与好友 roster。页面不显示已有卡 accountId、草稿可跨切号保留，存在误配置/误操作风险，但统一名片库是否有意跨账号管理不够明确，本批不单独编号。
- 运行 4 份直接测试，共 15/15 通过；随后 `npm run build` 成功，1,860 个模块完成转换。测试覆盖单账号资产渲染/编辑/删除、好友选择、疑似成交 approve/reject 端点和顾问回填，不覆盖账号切换、迟到响应、expected-account 写入、父级 selected 清理或资源 scope 展示，因此不反证 SR-160/161。
- 全局账号选择器不能与 workspace 级和 account 级资源混用而不标注。对于账号私有生产配置和联系人事实，资源 id 本身不足以证明当前操作上下文；前端必须保留资源 scope，后端写端必须验证 expected account。对于有意 workspace 共享的目录/审核池，则应明确标签并避免让顶部账号暗示不存在的过滤。

## B08J：统一收件箱与请示通道配置

覆盖文件：18 个，共 2,175 个物理行、2,038 个非空行、83,751 个冻结 blob 字节：Ask-Human 统一收件箱主视图、已裁决历史、两类内联动作、样式及 7 份直接测试，以及 Ask-Human Policy 页面、决策人链编辑器、表单 helper、样式与 2 份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，18/18 经 Git clean-filter 归一后与冻结 blob、台账 SHA-256 一致。另定向反查 Inbox Store/API/ReviewQueue、Inbox/Principal Escalation/Domain handler、AskHumanPolicy 模型与运行时、首推 MCP 账号及既有 SR-037、SR-038、SR-040、SR-041、SR-054、SR-067、SR-068、SR-090；反查文件不在本批重复结算。

### 统一收件箱、裁决动作与既有闭环（FACT）

- 收件箱是 workspace 级八源聚合，前端不读取顶部账号与后端现有产品语义一致；workspace 切换会整页 reload，不存在旧 workspace 快照继续动作。Inbox Store 把单次请求级失败设计成保留旧数据并显错误横幅，测试明确锁定该降级；source 局部失败仍返回 `errors[]`。summary 把来源异常降成 0 的既有冲突继续归 SR-068，疑似成交未纳入继续归 SR-067，DomainProfile 原始草稿不可达继续归 SR-090。
- principal 裁决卡展示客户、具体问题和类别，admin resolve 先在当前 workspace pending 列表中确认短码；改派还校验目标在当前 workspace 的决策链。裁决到 relay 的非原子提交继续归 SR-054，授权字段信任边界归 SR-037，微信领导回复跨账号匹配归 SR-040。本批前端只按短码发动作，没有新增跨 workspace 绕过。
- 已裁决历史使用独立 workspace 端点，只读展示 verdict、substance、约束、授权到期与渠道；组件有卸载取消守卫。它不分页且不显示 accountId，但当前频道本就是 workspace 总览，未发现独立于既有账号/请示模型的新越权链。

### 决策人作用域与生产配置提交（FACT）

- 新发现 SR-162：策略按 workspace+domain 生效，配置候选却来自顶部单账号联系人；保存结构只有裸 wxid。其它账号触发时仍用自己的 MCP 账号向该 wxid 发卡，无法证明好友/可发送关系。失败前已插 pending，因而会落入 SR-038 的幽灵 pending；这是配置作用域与投递账号不一致的独立根因。
- 新发现 SR-163：配置 GET 失败后页面主动换成默认草稿并继续开放保存。管理员补一人即可把未知的生产策略整体 `$set` 成默认开关与空频控，没有 baseline/version/diff。错误横幅虽可见，但不能把盲覆盖生产配置变成安全路径。
- 保存端直接原地修改 current version，立即影响全量在跑 Agent；这符合当前产品设计但没有版本 CAS。Management 同一工具已被列为 Dangerous 并要求确认，正式配置页却只有普通“保存”按钮，进一步说明该页面需要已知基线而非加载失败时的默认替身。

### 表单语义、测试与系统结论

- 新发现 SR-164：后端明确以空决策链停用通道，前端却把空链判错；静默时段三格又用 0 填充未输入项，使首次部分填写产生意外窗口，已有值也无法按文案清空为 None。`timeout/dedupe/cap` 的单字段删除语义则正确。
- 运行 Ask-Human/Config 9 份直接测试，共 38/38 通过；随后 `npm run build` 成功，1,860 个模块完成转换。测试覆盖收件箱排序、单次刷新、失败保留旧数据、来源筛选、标签、折叠、裁决/改派请求、历史展示、决策人链增删排序和纯函数校验，不覆盖跨账号投递能力、配置 GET 失败后的保存、空链停用、静默时段真实输入时序或 CAS。
- workspace 级策略不能用 account 级联系人选择器生成后丢弃来源身份；配置表单也不能把“未知现状”“未填写”和数值 0 合并。最小安全原则是：先证明已知基线，再提交带作用域和版本的 typed candidate；显式开关承载启停，空值保持空值。

## B08K：模型供应商配置

覆盖文件：3 个，共 1,036 个物理行、965 个非空行、41,908 个冻结 blob 字节：模型供应商页面、样式与直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，3/3 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 LLM Provider 模型/索引、admin handler、`LlmRegistry`、启动加载、视觉模型选择、Management 代理、激活集成测试及既有 SR-013、SR-020；反查文件不在本批重复结算。

### 主模型编辑、测试与热切换（FACT）

- 列表按授权 workspace 读取并只返回 masked key；编辑回传 mask 时后端沿用原 key，inline test 的明文 key 仅构造一次性 client。协议中性化、ACL 和密钥展示边界未发现新的前端泄漏。
- 新发现 SR-165：inactive 行激活有“立即影响生产”确认，active 行编辑却以普通保存直接 PUT；后端保存后立即 swap 全进程 Registry。测试按钮不是保存前置门，结果不绑定候选 hash，字段变化也不清测试证明。SR-013 已证明 Registry 为全局单例，因此当前 workspace 的普通保存实际影响所有租户。
- activate 先 swap Registry、再用两次独立 DB 写维护 active；任一 DB 写失败或并发激活都可能让 Registry 与持久 active 分裂，索引也不保证每 workspace 唯一 active。该一致性缺口已在 SR-013 建议中明确记录，本批补正式 UI 与测试证据，不重复编号。现有 ignored 集成测试只查顺序成功后的 DB 恰一 active，明确不覆盖 Registry 真换或中间失败。

### 可选参数与视觉模型生命周期（FACT）

- 新发现 SR-166：三个可选重试字段在 UI 声称空值回落默认；清空时前端省略键，后端只 `$set Some`、从不 `$unset`，因此已有 override 永远无法从正式页面清除。
- 新发现 SR-167：视觉指派只在建立时验证 supportsVision。后续普通编辑可关闭能力而保留指派标记；普通删除也只保护文字 active，不保护专职视觉行。运行时只选择 supportsVision=true，因此 UI 标记、持久状态和真实候选会分裂，唯一视觉模型还可被普通删除。
- 视觉运行时有一项正向容错：文字 active 本身支持图片时复用 Registry，否则按“专职优先、其余支持视觉者按更新时间”组成备用链，并只对瞬时错误切下一候选。这不能修复配置生命周期的矛盾，只会让问题表现为静默换备用或无候选失败。

### 测试与系统结论

- 运行直接前端测试 1 份，共 2/2 通过。测试只覆盖两条静态列表、协议标签、激活/视觉徽章与空态；没有点击编辑、保存、测试、激活、清空参数、关闭视觉能力或删除路径，因此不反证 SR-165–167。
- 模型供应商配置同时是凭证编辑器和生产流量切换器。最小安全设计不是增加复杂发布平台，而是把“候选内容、连通性证明、生产确认、实际 swap”绑定为同一 hash，并用明确 null 表达清除；视觉能力、指派和删除则维护一个简单的服务端不变量。

## B08L：系统策略整体

覆盖文件：17 个，共 4,617 个物理行、4,331 个非空行、191,336 个台账字节：系统策略主页面、Store、样式、6 份直接测试，以及 Operation Domain/State Policy、Taxonomy Entry/Candidate 的 4 组契约声明与 fixture。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，17/17 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Prompt/Soul reset 与发布、Ops 三表版本切换、DomainProfile CRUD/publish/rollout/activate、ProfilePublishCard、Taxonomy handler/cache、Planner 消费及既有 SR-008、SR-043、SR-053、SR-055、SR-072、SR-073、SR-090、SR-138、SR-139；反查文件不在本批重复结算。

### Prompt、Soul 与破坏性重置（FACT）

- Prompt 编辑/发布的三态确认在前端真实消费 `needs_human_confirm/reject`，并在管理员输入“已核对”后带 `force=true` 重提；但纯删除绕过语义闸、active/current 双真相和历史物删仍分别归 SR-139、SR-055。Soul 可原地改 published 与发布非原子物删继续归 SR-053，本批没有重复编号。
- SR-138 补强：正式总控页把跨 Soul、Prompt、Playbook、Operation Domain 四集合的 `reset_prompt_pack_v2` 暴露为普通“一键重置”按钮，点击即 POST，无影响清单、确认短语、快照或 dry-run。既有 finding 原本已覆盖同一后端破坏链与隐式探测触发，本批只补正式 UI 的确定性触发面。
- 页面只展示 management/methodology Prompt，运行时 user Prompt/Soul 的默认 workspace 串租户问题继续归 SR-021；本批 UI 没有提供隔离证明，也没有产生新的独立根因。

### DomainProfile 草稿、发布与确认（FACT）

- active profile 的原地 PUT 会被后端拒绝，危险字段只能先编辑非 active 草稿再 publish；因此“直接改 active 绕确认”候选被反证。普通字段发布会在已生效血缘上即时迁移 current+active，危险字段则克隆旁路稿并要求二次确认。
- SR-073 补强：`ProfilePublishCard` 收到危险 publish 返回的新旁路稿 `id` 后，响应类型不保留它，确认时仍用旧草稿 id 调 rollout。于是正式 UI 会 promote 原草稿而非刚确认的新克隆版本，却 toast“已发布”；Store 中未被该卡使用的另一套 action 才正确回传新 id。候选不冻结、rollout 可绕 publish 的既有根因不变。
- 原始草稿被默认列表与发布卡过滤的不可达链继续归 SR-090；activate 跨画像、状态机、Policy 与联系人迁移的半提交继续归 SR-072；current/active 多写竞态继续归 SR-043。版本动作条本身不能修复这些后端生命周期裂缝。

### Taxonomy 生产语义与灰度（FACT）

- 新发现 SR-168：列表投影省略 `isTerminal/isReactivationTarget`，编辑器把缺失值按 false 回填并在任意保存时无条件 PATCH，直接清掉 current 条目的两个运行时标志。Planner 用它们构造停滞扫描排除集和再激活目标集，因此普通改名可让终态重新被催进、让行业休眠阶段退出唤醒扫描。
- 嵌套契约 fixture 同样没有两个标志；契约测试只对账顶层键并把 `value` 作为一个不展开的容器，所以不会发现生产子字段丢失。组件测试又以不含标志的 fixture 断言 false/false PATCH，把缺陷锁成绿测。
- Taxonomy 编辑、废弃和恢复都直接原地修改 current 行并立即失效缓存，未走同页展示的 publish/rollout 链；并发 current 异常继续归 SR-008，alias 歧义与候选合并问题继续归 SR-046/SR-061。本批新增编号只覆盖“投影丢字段→普通编辑清零→Planner 行为变化”的独立闭环。

### 测试与系统结论

- 运行 6 份直接测试，共 28/28 通过；测试覆盖 tab/空态、字典请求形状与分页、Prompt 三态确认、画像高级字段回填、按关系范式和版本回滚按钮。它们不覆盖高风险 publish 返回 id 与 rollout id 一致、重置取消零写入、字典 true 标志往返、嵌套 value 深度契约或 Planner 副作用，因此不反证 SR-073/SR-138/SR-168。
- 系统策略页把“草稿编辑、版本发布、立即生效、原地生产修改、跨集合重置”五种不同生命周期放在同一视觉层。最小安全原则不是再加一套复杂平台，而是让所有生产动作都遵守同一条可验证链：完整读回已知基线 → 不可变 candidate/version → 显式影响确认 → CAS 切换同一 id/hash → 读回实际生效身份；任何响应投影不得丢失可编辑且被运行时消费的字段。

## B08M：共享 UI、全局视觉基础与弹窗交互

覆盖文件：54 个，共 5,738 个物理行、5,027 个非空行、144,992 个台账字节（冻结 blob 合计 139,260 字节）：favicon、Shell/全局错误样式、全局 `styles.css`、Vite CSS 类型声明，`components/ui` 下 11 组共享组件/样式/导出，以及 11 份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；54/54 以 Git clean-filter/`hash-object` 口径与冻结 blob、台账 SHA-256 一致。另定向反查共享 Provider 挂载点、Chunk 操作栏、FriendPicker 两个正式调用方及既有 SR-090、SR-105、SR-161/162；反查文件不在本批重复结算。

### 全局样式与共享组件（FACT）

- `tokens.css` 提供语义色、焦点环与浮层层级，`reset.css` 极小；遗留 `styles.css` 仍有 4,005 行裸 button/input/main 与大量旧频道类，新 CSS Module 普遍显式重置以避免污染。这是明确的迁移债务，但本批未证明它产生独立于已记录页面行为的生产数据错误，不单独编号。
- `Overlay` 正确提供 portal、dialog 语义、Esc/scrim、Tab 环与滚动锁；Confirm 的危险 tone 禁止点遮罩关闭，requireText 首字段输入即使 effect 重跑仍回到自身。Toast、EmptyState、Status/Metric/Avatar 等纯展示组件未发现新的业务副作用链。
- FriendPicker 的手工 wxid 入口只在专属顾问创建中显式开启；其跨账号/可达性语义已由 SR-162 及对应资源 scope 讨论覆盖。Products/Deals 禁用手工输入，切号后父级 selected 未清继续归 SR-161，本批不重复编号。

### 多字段弹窗真实输入时序（FACT，补强 SR-105）

- `FormDialogProvider` 每次 `setValues` 重渲染都会向 `Overlay` 传一个新的内联 `onClose`；`Overlay` effect 依赖 `onClose`，故每个字符后都执行 cleanup/重新进场并把焦点移回首个可聚焦控件。正式 split 的 cutoff 是第二字段，relate 的 note 是第三字段。
- 临时最小测试用 `@testing-library/user-event` 对第二字段逐键输入 `abc`，冻结代码实际只保留 `a`，焦点随后回到第一字段；测试按验证约定已立即删除，未计入台账或工作树。这个缺陷不另起 SR-169：当前两个多字段入口同时被 SR-105 的 wire schema 漂移确定性阻断，业务影响同属“正式 Chunk 操作不可用”；修复契约后必须同时稳定 callback/限制 Overlay 进场 effect。

### 测试与系统结论

- 运行 11 份直接测试，共 34/34 通过。测试覆盖静态渲染、整值 change、required、取消、Esc/scrim、初始聚焦、Toast 和 FriendPicker 基本筛选；FormDialog 用一次 `fireEvent.change` 写整串值，没有逐键输入、焦点保持或多字段真实时序，因此不反证 SR-105 补证。
- 共享 UI 的最小不变量是：弹窗开闭管理焦点，弹窗内部字段更新不应被误判成再次进场；业务表单还必须以同一 typed wire schema 对接后端。视觉 token 与组件抽取不能替代这两条行为契约。

## B08N：共享 Review / Prompt 组件与审核对象身份

覆盖文件：21 个，共 2,653 个物理行、2,455 个非空行、104,893 个台账字节（冻结 blob 合计 102,240 字节）：共享 Prompt 保存/发布确认 hook，Review 卡片、证据原语、类型与样式，以及 6 份 Review 组件测试和 2 份 Prompt 三态 Store 测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；21/21 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查统一收件箱 rich/inline 分派、Chunk verify/reject、DomainProfile publish/rollout、Lesson 晋升、Taxonomy 审批、Evolution release/rollback 及既有 SR-055、SR-061、SR-073、SR-086/087、SR-090、SR-097；反查文件不在本批重复结算。

### 共享卡片与既有提交边界（FACT）

- Prompt save/publish hook 正确消费 Store 的 `ok/needsConfirm/rejected/error` 三态；两类需确认结果都展示原因/diff、要求输入“已核对”，并对同一 template id 带 `force=true` 重提。现有 Store 测试覆盖三态和 force 请求体；底层手工 Prompt 发布的 active/current 分裂与历史物删继续归 SR-055。
- Chunk 卡的 verify 门与后端一致要求 quote+anchor，空 `{}` 请求可被 `verifiedClaims` 可选 DTO 接受；reject 无 body extractor，也接受前端 `{}`。Taxonomy 409 确实表示候选已被标 approved，但既有 canonical 没合并 raw alias 的语义裂缝继续归 SR-061。Lesson 卡提交 draft+needs_review 的同行案例候选，但服务端 insert→lesson 回写非原子/非幂等继续归 SR-097。
- Profile 高风险 publish 仍丢弃响应中的新 candidate id、用旧 profile id rollout，归 SR-073；原始草稿被默认列表过滤归 SR-090。Evolution 卡要求精确输入 RELEASE/ROLLBACK，但候选不绑定评估基线与旧 proposal 可跨后续发布回滚分别继续归 SR-086/SR-087。本批没有把这些既有后端根因重复编号。

### 审核队列行身份丢失（FACT，SR-169）

- `ReviewQueue` 的 `getId` 只参与 busy 状态，`items.map` 返回的 React 元素没有 key；统一收件箱的 `renderItem` 同样返回无 key `InboxRow`。同类 A 行处置后从列表移除、B 上移时，React 按位置复用 A 的行和子卡 state，而 props 与动作 URL 已切成 B。
- 临时最小测试用真实 `ReviewQueue + TaxonomyCandidateReviewCard` 构造 `[A,B]→[B]`：先把 A 显示名改为 `edited-a`，刷新后点屏幕上的 B“采纳”。冻结代码向 `/B/approve` 发请求，但 canonical body 仍是 `{id:"raw-a",label:"edited-a"}`；React 同时打印缺 key warning。测试按验证约定已立即删除，未计入台账或工作树。
- 影响覆盖所有持 state 的同源审核卡：Taxonomy、Escalation 与 Lesson 可稳定把 A 草稿写到 B；Chunk/Profile 在 id effect 的异步重读窗口可展示 A、动作 B；Proposal load 先显示 loading，风险较低但仍依赖相同错误行身份。该根因独立于 SR-144 的账号切换密钥表单，单列 SR-169。

### 测试与系统结论

- 运行 8 份直接测试，共 37/37 通过。测试覆盖 Chunk 门真值表、证据解析、画像状态机展示、Proposal 元数据/对照表、队列首次加载和成功 refetch、Taxonomy 单项审批，以及 Prompt 三态；没有任何测试在 refetch 后删除首项并验证第二项 state/body/URL 身份一致，因此不反证 SR-169。
- 人审 UI 的最小不变量不是“按钮调用了当前 URL”，而是外层标题、展开状态、表单草稿、确认 diff、提交 id 和服务端 generation 全部绑定同一稳定对象。列表重排必须以持久 id 驱动 reconciliation，异步卡片在新对象完成读取前不得保留旧对象的可操作视图。

## B08O：Evolution 页面、契约与控制面

覆盖文件：19 个，共 1,821 个物理行、1,700 个非空行、72,265 个台账字节：Evolution 频道壳、主页面与 CSS Module、兼容 re-export，runtime flag / experiment / proposal / shadow replay 的 6 组契约声明与 fixture，以及 3 份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；19/19 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Evolution runtime flag handler、worker tick、cohort、auto-release、实验列表和既有 SR-049、SR-058、SR-085–088、SR-133；反查文件不在本批重复结算。

### 总开关与自动发布（FACT）

- 新发现 SR-170：UI 的“演化中心总开关”只控制 cohort。成功关闭后，后端保留 `threshold_auto_release_enabled`；每个 tick 即使 cohort 为空，仍无条件运行 auto-release，而该函数只检查 auto-release env 与子闸，不检查父级 `runtime_flag.enabled`。已有 eligible threshold 候选因此仍可在“总开关关闭”后自动发布。
- 同一控制面还有确定性假关停：checkbox 先乐观切值再 PUT，失败只写普通消息，不回滚或重读。临时最小测试让关闭 PUT 返回 500，冻结页面最终仍显示 checkbox 已关闭；测试按验证约定已删除，未计入台账或工作树。现有成功路径测试和 auto-release 双闸纯函数都没有父总闸/失败回滚断言。
- env `EVOLUTION_ENABLED=false` 仍会在 worker 入口硬停，这是有效的最外层运维熔断；问题发生在页面明确承诺的 Mongo 日常总开关。非默认 workspace 保存无人消费 flag 的作用域黑洞继续归 SR-085，updatedBy 不可信继续归 SR-058，不与本项重复。

### 时间窗聚合与候选治理（FACT）

- 新发现 SR-171：页面先固定取最新 20 个 experiments，再在客户端过滤 7 天并展示为“近 7 天”。默认 tick 为 6 小时，7 天约 28 个；且 tick 在检查 runtime flag 前就写 envelope，所以持续运行一周后至少 8 个仍在窗口内的实验及 proposals 被稳定截断。五张聚合卡没有覆盖起点或 truncated 提示。
- Proposal 列表/详情字段、状态 badge、精确 RELEASE/ROLLBACK 确认串和阈值审计端点与已审后端形状一致。候选证据不冻结、旧 Prompt proposal 跨后续版本回滚、并发 threshold release 复制 override 分别继续归 SR-086、SR-087、SR-133；共享详情卡的字段展示已在 B08N 阅读，本批不重复编号。
- 六组契约测试能发现各 fixture 的顶层增删键，但 `experimentSummary` 明确只核对外层三键，不能证明嵌套 envelope/proposals 与各自 canonical 契约组合一致，也不验证分页时间窗。该限制在 SR-171 的统计截断上直接体现。

### 测试与系统结论

- 运行 3 份直接测试，共 22/22 通过。测试覆盖状态徽章、发布按钮、确认字面串、Prompt diff、runtime flag 成功读写、审计表、硬锁/关闭历史可见、错误加载，以及 8 组 Evolution 域顶层键集；不覆盖关闭 PUT 失败、父总闸对 auto-release 的一票否决、写响应乱序或超过 20 条的 7 天窗口，因此不反证 SR-170/171。
- Evolution 控制面需要两个朴素不变量：父级关闭必须支配所有下游副作用；时间窗口指标必须由时间条件而非列表条数定义。配置 UI 的 checkbox 只能展示服务端已提交事实，聚合卡也只能展示可证明覆盖完整的样本窗。

## B08P：剩余共享契约、fixture 与语义完整性

覆盖文件：24 个，共 450 个物理行、442 个非空行、14,560 个台账字节：Evaluation Scenario、Import Job、Ingest Source、Outbox、Playbook、Prompt Template、Relationship Suggestion、Revision Applied、Suspected Deal 与 Threshold Override/Audit 的 11 组契约声明/fixture，以及配置/Playbook、Taxonomy 两份直接契约测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；24/24 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查各后端投影/快照测试、正式消费者、Outbox 生产 enqueue 与既有 SR-050、SR-052、SR-066、SR-159；反查文件不在本批重复结算。

### Fixture 来源与契约能力边界（FACT）

- 本批核心 fixture 并非仅由前端手工维护：Rust 投影测试会构造完整模型并通过 `assert_contract_fixture` 与同名 JSON 对账，覆盖 Evaluation、Import Job、Ingest Source、Outbox、Playbook、Prompt Template、Relationship/Suspected Deal、Revision 与 Threshold 两组投影。这能在后端投影字段增删且未 re-bless 时真实测红。
- 前端测试仍只比较 fixture 与 `CANONICAL_KEYS` 的顶层键；`contactSeed/groundTruth`、Import `progress/result` 等嵌套对象不展开，值类型、枚举、nullability 和跨字段语义也不验证。Taxonomy 的嵌套字段遗漏已由 SR-168 证明这种限制可直接隐藏生产语义变化。
- Import Job 的 progress/result 与正式轮询解包一致，Ingest Source 的 sourceId/status/failure 字段也被正式页面消费；Relationship/Suspected Deal 的顶层投影与现有审核入口一致。相关跨集合提交、actor 与账号边界继续归既有 SR-057–059、SR-117、SR-136，不重复编号。

### Outbox 对象身份投影缺失（FACT，SR-172）

- Outbox Rust 快照测试用非空 `media_asset_id/referral_card_id` 构造完整模型，却明确把投影“漏发 mediaAssetId/referralCardId/reclaimedInFlight”作为当前契约；fixture 和前端声明随之都不含这些字段。测试因此不是失效，而是在稳定证明一个语义不完整的 DTO。
- 生产 Gateway 对媒体/名片 enqueue 时把 `content` 固定为空，业务对象只能由上述 id 识别。正式 OutboxPanel 只展示状态、联系人、content、时间和取消按钮，故媒体/名片行成为无类型、无名称、无目标的空白行；同联系人多条时只能按时间猜测。`reclaimed_in_flight/reclaim_count` 也不下发，远端可能已送达的恢复风险不可见。
- pending/in_flight 空白行可直接取消且无确认。跨账号旧快照误取消继续归 SR-159，in-flight 取消与真实送达竞态继续归 SR-066；SR-172 只覆盖单账号静态列表中“投影删除对象身份→无法辨认→盲取消”的独立链。

### 测试与系统结论

- 运行 2 份直接契约测试，共 10/10 通过。它们证明声明与当前 fixture 顶层键集一致，但不证明完整模型中的业务判别字段均已投影，也不覆盖正式页面对媒体/名片/恢复态的显示与取消确认。
- 契约快照的最小安全标准不能止于“投影没有意外漂移”：还应从完整领域模型声明每个消费者动作所需的语义字段，并对 tagged union、嵌套键、nullability 与状态不变量做断言。否则快照会忠实固化一次有意但危险的字段删除。

## B08Q：共享前端工具、收件箱 Store 与 SSE 重连

覆盖文件：12 个，共 963 个物理行、894 个非空行、40,811 个台账字节：`applyAiRepairPatch`、格式化、统一收件箱 API、共享中文标签、SSE 重连器、Inbox Store，以及 6 份直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；12/12 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Ask Human 正式数据流、Knowledge Chat/Task SSE、TaskRail、后端进度总线与既有 SR-067、SR-101、SR-106、SR-127；反查文件不在本批重复结算。

### 共享工具与既有边界（FACT）

- `applyAiRepairPatch` 从完整 `originalChunk` 构造 PUT，只覆盖操作员勾选且 patch 中存在的字段，未勾选字段保持旧值；业务写成功后再单独 POST applied 事件，`thenVerify` 固定 false。这个客户端防清空成立，但完整 PUT 可被后端 integrity 重算直接升 verified、两请求审计又不可信，分别继续归 SR-101/SR-106，不重复编号。
- `api.patch` 的 JSON method/body/error 路径有直接测试；`formatRate/formatNumber` 对 null/undefined/NaN 统一回退破折号。共享标签文件共 340 行，闭集状态有原值兜底；测试直接覆盖 final review、hold、gateway 32 值及部分扩展字典，本批未发现标签本身造成新的写侧语义。
- Inbox API 会标准化缺失 items/errors 并按严重度、等待时长排序。Store 在请求失败时保留旧 items 并展示 fatal 横幅，避免把网络故障冒充空队列；summary 失败则保留上次值。正式 Ask Human 只有这一条 fetch 来源，消除了旧双请求路径。

### 收件箱筛选与请求身份（FACT，未单独编号）

- Inbox Store 只有全局 `items/errors/summary/loading/fatalError/activeSource`，`load` 没有 request generation 或 AbortController。切换 source 虽用 React key 卸载旧 ReviewQueue，但旧 load 仍能迟到写全局 Store；若新筛选随后失败，`load` 又不抛错，`fetchItems` 会从 Store 取这份旧来源 items，使当前筛选视图回显不匹配条目。
- 行内动作仍使用每条对象自身 source/id，不会像 SR-169 那样把一条草稿提交给另一条对象；页面也明确显示“加载失败（显示上次数据）”。本批将其保留为较窄的过滤/观测一致性风险，不为同类迟到响应再扩一条 finding。修复时仍应让 Store 按 source+generation 提交结果，失败保留同一 source 的最后成功快照。

### SSE 健康与知识长任务静默停更（FACT，SR-173）

- 新发现 SR-173：共享重连器只在注册业务事件到达时把 attempt 清零，原生 EventSource `open`、服务端 keepalive 或长时间稳定连接都不算成功。每次断线继续消耗历史 attempt，默认累计 6 次后永久停止；这实际限制的是组件生命周期累计断线，而非连续建连失败。
- Knowledge Chat/Task 流只注册 `turn` 和终态 `close`。worker 仅在开始、完整 step 结束和总结时写 turn；单步可执行多次最长 45 秒的 LLM 调用，因此成功重连后长时间无 turn 是正常态。临时纯测试构造每个替代连接均已 open、但尚无 turn 又断开，冻结实现仍在测试上限 2 后触发 `onGaveUp`；测试已删除，未计入台账或工作树。
- TaskRail 不传 `onReconnecting/onGaveUp`，也不轮询 task；turn 回调只追加“第 N 步”，不重读任务详情。重试耗尽后页面无提示，主状态、完成步数、进度条、错误与终态永久停在首次手工拉取快照，直到用户再次点击“拉取”。

### 测试与系统结论

- 运行 6 份直接测试，共 26/26 通过。它们覆盖 PATCH、AI patch 字段保留与两阶段失败、收件箱排序/降级、标签闭集，以及连续 error 退避、业务 turn 重置、terminal close；不覆盖 EventSource open 后重置、keepalive、累计非连续断线、gave-up UI/轮询或筛选请求乱序，因此不反证 SR-173。
- 长连接重试预算必须描述连续失败，而不是页面生命周期内曾发生过多少次断线；连接成功、协议心跳与业务进度是三种不同健康信号。调用方还必须把 reconnect/gave-up 转成可见状态，并用可验证的 task GET 兜底收敛最终事实。

## B08R：剩余前端测试与阶段 8 收口

覆盖文件：15 个，共 947 个物理行、864 个非空行、41,206 个台账字节：自治成效与 Planner、Shell workspace 切换、Knowledge Cockpit/Review/Go-Live、测试 setup，以及 Account/Contact/Navigation/Profile/Strategy Store 的剩余直接测试。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF；15/15 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查 Cockpit 错误态、workspace 切换实现和对应既有 SR-129、SR-130、SR-142、SR-159；反查文件不在本批重复结算。

### 绿测覆盖与既有缺口（FACT）

- 自治成效测试覆盖零分母、revision/hold 比率与 Planner 三段展示，但固定 `accountId=default` 且 mock 请求即时完成；它不覆盖账号切换、乱序响应或旧 Outbox 取消，因此不反证 SR-159。后端指标多查询非快照导致的矛盾比率继续归 SR-078。
- Knowledge Cockpit 的解析、五维展示、D2 前端门和 apply→verify 顺序有直接覆盖；Cockpit 正式实现会在任一必要请求失败时显示可重试错误，不把失败冒充零值。AutoVerify 测试只断言 200 响应数字，未检查 snake_case 请求体；ReviewChat 测试手写 `{turn:{patch}}`，既不匹配正式顶层 `draftPreview`，也不检查 attachment camelCase。它们分别继续作为 SR-129/SR-130 的绿测盲区，而非反证。
- Shell workspace 切换成功后由 `main.tsx` 整页 reload，旧 workspace 的 Zustand/组件快照随页面销毁；本批没有发现新的跨 workspace 留存链。切换失败静默属于既记韧性债务。Account/Contact Store 测试只验证派生计数与本地选择，完全没有账号身份、请求 generation 或切号清理断言，因此不反证 SR-142 及其下游切号问题。
- Profile/Strategy Store 测试覆盖字典标签回退和 DomainProfile create/update 路由分支；发布候选 id、危险 rollout、current/active 不变量和字典生产标志仍由 SR-043、SR-073、SR-168 覆盖。本批没有从这些纯分支测试外推新的生产保障。

### 测试与阶段结论

- 运行 14 份可执行直接测试，共 65/65 通过；第 15 个对象是全局 jest-dom setup，不是独立测试文件。测试证明局部渲染和顺序分支符合当前实现，但没有覆盖上述跨账号、异步身份和真实双端 wire schema。
- B08R 已结算 `frontend/src` 的最后一组测试对象；根级测试配置、手工走查脚本与依赖锁在 B08S 单独按工具/结构对象口径收尾。大量局部绿测有效保护展示与单请求 happy path，但生产风险集中在 scope/generation、候选身份、跨请求提交和契约语义完整性；这些必须以双端契约、乱序响应和真实副作用测试验证，不能由单组件即时 mock 推断。

## B08S：前端测试配置、手工走查与依赖锁

覆盖对象：3 个，共 10,281 个文本字节与 107,333 个结构锁文件字节。`frontend/vitest.config.ts`（19 行）和 `frontend/walkthrough.py`（179 行）均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 连续读取到 EOF，工作树 SHA-256 与台账一致；`frontend/package-lock.json` 按 `dependency_lock_structural` 口径完成格式、根依赖、包条目和消费链验证。

### 自动验证链与手工脚本边界（FACT）

- `vitest.config.ts` 被 `package.json` 的 `vitest run` 自动发现，CI 也在 `frontend` 工作目录执行 `npx vitest run`；配置固定 jsdom、jest-dom setup 与 `src/__tests__/**/*.test.{ts,tsx}` 扫描范围。B08R 的剩余测试和 B08Q 的共享工具测试均通过该配置运行，属于真实自动验证链。
- `walkthrough.py` 依赖已启动的 `localhost:5173`、系统 Chrome 与 Python Playwright，通过 route mock 检查 Knowledge Cockpit 文案、四屏截图、左缘竖色杠和 console error。全仓没有 package script、CI workflow 或其它脚本调用它；其中的 `ALL PASS` 只能是人工按需执行结果，不能当作当前审查或 CI 已运行的证据。本轮未启动额外前端服务，因此不执行该脚本。

### 依赖锁结构验证（VERIFIED_NON_TEXT）

- `package-lock.json` 可由 Node 完整解析，`lockfileVersion=3`，共 213 个 `packages` 键（根项 + 212 个包条目）。根项 dependencies/devDependencies 与 `package.json` 分别逐键逐值一致；所有直接依赖都有 `node_modules/<name>` 条目，212 个非根条目均具版本，普通包均具 resolved 与 integrity，未发现缺失或畸形项。
- 该锁文件由 CI 的 `npm ci` 和 npm cache key 直接消费；本阶段已在现有锁定依赖上成功运行分批 Vitest。锁文件不按源码逐行解释，而按 review plan 的 `verified_non_text` 规则记录消费者、格式工具与一致性证明。

### 阶段 8 结论

- B08A–B08S 已覆盖冻结清单全部 371 个 `frontend/` 对象：370 个文本对象 `read_complete`，1 个依赖锁 `verified_non_text`，无 `pending_read/pending_verify`。阶段发现截至 SR-173 连续编号；B08S 未新增生产 finding。
- 阶段最终执行门运行完整 `npm test -- --run`：114 个测试文件、508/508 用例通过；随后 `npm run build` 的 TypeScript 检查与 Vite 生产构建成功（1,860 modules transformed）。全目录 371/371 对象的内容身份复核通过：367 个工作树原始 SHA-256 直接匹配台账，4 个 Taxonomy 契约对象因 Windows CRLF 与台账原始字节不同，但 Git clean-filter blob 与冻结提交逐一一致。构建与测试通过只证明当前自动门，不消除 SR-001–173 的业务发现。
- 自动测试配置有效，但仓库中的手工浏览器走查不会自动保护回归。若要把视觉/跨屏契约升级为发布门，应将其迁入受 CI 管理的 Playwright 项目，固定浏览器、服务启动、mock schema 与截图/console 产物，而不是依赖单机脚本的历史输出。

## B09A：共享测试基础设施与纯函数宿主

覆盖文件：14 个，共 3,176 个物理行、2,925 个非空行、140,053 个台账字节：`tests/common` 的 TestApp、动态三族/轨迹裁判、泛化报告、身份生成器、profile 化 judge、红线判定、Roleplay fixture/扮演器，以及 6 个对应 smoke/rubric 宿主。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF；14/14 工作树对象与冻结 blob、台账 SHA-256 一致。另定向反查生产 Taxonomy/DomainProfile 全局缓存、CI 默认/ignored/真模型接线、Roleplay/动态对抗调用方及既有 SR-004、SR-126、SR-128；反查文件不在本批重复结算。

### 有效测试原语与能力边界（FACT）

- `TestLlmGenerator` 使用进程内 FIFO 返回预排 JSON/usage，并显式统计调用；`TestApp` 为每次启动创建 UUID Mongo database、跑全部迁移/索引、补 seed taxonomy、种 Prompt pack，并以真实 `AppState` 接入 mock LLM/MCP。事务路径另有 replica-set 启动器，Outbox 轮询 helper 只把 `sent/failed_terminal/canceled` 当终态。
- 泛化报告对 train/holdout 空集、各自 floor 与绝对 gap 分开建模；身份生成器用固定行业骨架和 `seed % len` 锁定大类/funnel 极性，LLM 只丰满语义。红线 helper 补齐“转人工/人工客服”，并以子句内否定识别避免把“无需转人工”误判为承诺；这些纯函数都有直接正反例。
- profile 化 judge 会按 funnel 在 `manipulationRisk` 与 `pressureRisk+关系维` 间翻转。`run_judge_graded` 区分 `QualityGate/ObserveOnly`，但当前真实套件调用方只使用 `ObserveOnly`；`QualityGate` 仅在 `judge_rubric` 自验证宿主出现。因此它目前不能被外推成生产质量硬门。HTTP 成功但缺评分维度仍会计为一次成功并返回稀疏/空 medians，是未接线 helper 的脆弱边界，本批不单独编号。
- 动态 roleplayer 的 API 只接收 persona、场景和对话历史，不接 reviewer/operation state；调用失败会返回带 `Fallback` 标志的固定台词。正式动态对抗/roleplay arc 至少要求一轮 `Generated`，全程 fallback 会显式 skip；身份生成器失败也会让单个域 arc 正常 return。真模型 suite 对“无结论/skip 仍绿色”的系统问题已由 SR-128 覆盖，本批不重复编号。

### 独立数据库与全局缓存串扰（FACT，SR-174）

- 新发现 SR-174：`TestApp` 虽为每个用例创建独立 database，却在每次启动时把该 DB 的 Taxonomy 与 active DomainProfile 全表写进同一进程级 `LazyLock` 缓存；缓存键不含 database identity。后一用例的预热会整体覆盖前一用例快照，且 30 秒内读取因缓存新鲜不会回到调用方传入的 DB。
- 确定性交错为：A 启动并在 `default` 写自定义 active profile；B 以空 DB 完成 `init_global_domain_profile_cache(db_b)`；A 随后调用 `load_active_domain_profile(db_a,"default")`，实际查全局 B 快照并回落 DEFAULT。Taxonomy 同理。单个 test binary 内 Rust 默认并行执行测试；`domain_profile_e2e` 有 28 个 TestApp 用例，多个其它文件也有 5–17 个，CI 没有 `--test-threads=1`。
- 本轮编写并立即删除了只验证上述 A/B 顺序的临时 ignored 测试；本机因 Docker daemon 不可用，在第一个 Mongo 容器启动处失败，故不把它记作运行复现。代码级状态替换、freshness 分支和默认并行接线已足以确定交错机制；触发条件和运行限制均在 SR-174 明示。

### 执行与系统结论

- 运行 5 个无 Docker 宿主（dynamic、identity generator、judge rubric、redline、roleplay fixture）：各二进制分别报告 27、29、35、27、24 个通过，共 142 次报告通过、0 失败，Roleplay 另有 4 个 Docker 用例 ignored。`tests/common` 的内嵌单测会被每个 `mod common` 宿主重复编译/执行，因此 142 是跨二进制重复执行次数，不是 142 个独立测试性质。`common_smoke` 本体只有一个 Docker ignored 用例，本机未运行。
- 测试隔离的最小单位必须覆盖数据库、进程级 cache、环境变量和外部 provider，而不只是 collection 名。对生产单例的测试应注入实例级依赖；否则增加更多独立容器只会制造“数据库隔离、内存状态共享”的隐蔽竞态。

## B09B：PBT、回归种子与测试自证边界

覆盖对象：14 个，共 2,881 个物理行、2,649 个非空行、121,978 个台账字节：12 份 property-based/不变量测试与 2 份 Proptest 回归种子。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF；14/14 工作树对象与冻结 blob、台账 SHA-256 一致。两份 `.proptest-regressions` 均使用 Proptest 标准同名伴随文件格式，会在对应测试生成新 case 前自动重放既有失败种子。

### 直接约束生产纯函数的有效性质（FACT）

- Account scheduler、Chunk type 路由、Cold Reactivation、human-like/pressure 阈值、Intent trajectory、Memory Card、Page Merge 与状态迁移测试均直接调用生产纯函数，能有效约束其过滤、分桶、阈值边界、滑窗 cap、BSON 往返、字段锁定、数组并集、hash 稳定和 allowedFrom 判定。状态迁移、Memory Card 与 Wiki revision 还进入 `scripts/check-baseline.{ps1,sh}` 的阻断 PBT 基线；其余目标参与通常的 `cargo test --tests`，但不在那组四文件计数门内。
- Behavior Signal 测试直接验证生产 builder 的 dedupe key、类型隔离与 censored/source/confidence 字段；但所谓“重放只写一次”由测试内 `HashSet` 模拟 unique index，并未调用 Mongo 或 `persist_signal`。真实 partial unique/persist 路径另有 Docker ignored smoke；本文件不能反证 SR-137 的 account 身份缺失，也不能证明数据库索引已部署。
- 两份回归种子分别锁定 Autonomy 必填字段 case 与 Memory discard case；它们是历史失败输入的稳定重放，不是独立业务覆盖。种子文件与测试源同目录同 basename，符合 Proptest 自动发现约定。

### 测试内重写生产行为的自证性质（FACT）

- `autonomy_protocol_pbt` 的 P1/P3 分别直调 `RawAgentDecision::validate_and_promote` 与 `local_decision_review`，属于有效生产性质；P2 却在测试文件内自建 `ReviewSnapshot` 与 `run_revision_loop`，没有调用 `decide_revision`、Gateway、LLM、Finalize 或 Outbox。它只能证明这份手写模型最多加一次调用，生产控制流漂移不会必然使测试变红。
- `wiki_chunk_revision_pbt` 的 locked field、union、truncation、hash 与 normalize ref key直接调用生产 helper；但 `ai_status_forced` 是测试自己执行 `$set status=draft/needs_review`，`revision_id_unique` 自己拼 UUID，`rollback_idempotent` 用 `enforce_locked_fields` 模拟 rollback，均未调用真实 revision apply/rollback handler、Mongo 写链或审计。因此这些绿色结果不能外推为知识 revision 生命周期已验证。
- Account scheduler PBT 固化“全满/全 off-hours 时退化到任一 online 账号”的 helper 语义。定向反查确认生产 `assign_account` 当前唯一调用方在 Cold Contact 中丢弃返回值，只借其写 assignment 审计，真实 follow-up 仍按 contact 原 account 发送；故本批不把该 helper 策略单列生产 finding。若未来接入新联系人分配，必须重新评估 capacity/off-hours 是否允许 fail-open。

### Reviewer 缺失评分被当作安全（FACT，SR-175）

- 新发现 SR-175：Reviewer 实时 wire DTO 对 `factRisk`、`pressureRisk`、`boundaryPrivacySafety` 使用宽容默认与 `number_i32`；缺字段、null、字符串或其它无法解析值都成为 0。主/第二 Reviewer 反序列化后立即进入双闸，没有字段完整性或范围门。
- `factRisk=0` 直接通过 hallucination 硬闸；pressure 与 boundary/privacy 又明确把 0 定义为 legacy/未填豁免。Finalize 不重新验证 Reviewer schema，故其余分数通过时可进入 Approved/outbox。`pressure_risk_threshold_pbt` 与模块单测主动把零豁免锁成绿色，说明这不是偶然漏测，而是实时输出与历史兼容模型混用造成的 fail-open 契约。
- 该问题与 SR-022 不同：SR-022 是产品声明分类字段漏报后不进入 verified-knowledge 门；SR-175 是三个评分字段本身缺失或畸形后被伪装成最低风险。与 SR-023 也不同：SR-023 已知软闸失败但 revision 失败后恢复原稿；SR-175 在首轮分类时就看不见失败。

### 执行与系统结论

- 顺序运行 12 个无外部依赖目标，共 70/70 通过、0 失败、0 ignored；各目标分别通过 4、3、4、4、4、4、4、15、9、4、6、9 个测试。编译与执行未使用 Docker、Mongo 或真实 LLM。
- PBT 的价值取决于 oracle 是否独立且被测调用是否真实。直接调用生产纯函数的代数/边界性质应保留；在测试里重写 Gateway、数据库索引或 handler 副作用，只能作为设计草图，不能命名或计数成生产闭环保证。关键回归应穿过严格 wire DTO、真实控制函数和 committed side effect，并让 schema 缺失、乱序与失败恢复成为生成维度。

## B09C：账户、认证、JWT 与租户隔离集成测试

覆盖对象：9 个，共 1,890 个物理行、1,701 个非空行、69,012 个台账字节：账号离线 defer、账号密钥安全、认证/session、H3 Provider IDOR、JWT、产品与通用 workspace 隔离共 7 份 Rust 测试，以及 2 份 JWT 测试 PEM。7 份文本均按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF；9/9 工作树对象与冻结 blob、台账 SHA-256 一致。两份 PEM 按 `test_key_structural` 口径验证 PKCS#8 private/public wrapper、行数与尺寸，不输出私钥正文；`jwt_auth` 的 RS256 签发/验签、篡改拒绝与过期拒绝进一步证明该测试密钥对可被正式密码学库消费。

### 真实生产边界的有效覆盖（FACT）

- `account_offline_defer_integration` 直调正式 webhook 与 Outbox `process_entry`：Online/Offline 事件更新账号状态；离线时回 pending、不耗 attempt、推迟 retry、写 defer 事件且 MCP 零调用；在线时真实走 mock MCP 后标 sent。该文件覆盖的是生产控制流与 committed side effect，不是测试内重写。
- `account_security_integration` 直调 `list_accounts` 与 `update_account_mcp_key`，验证响应不回显明文、Debug 掩码和跨 workspace ObjectId 更新被拒；它手工构造 `AuthenticatedAdmin`，因此证明 Handler 的 workspace filter，不证明 middleware 自身。`h3_cross_tenant_idor` 则 seed 真实 AdminUser/ACL 后直调 Provider list/activate，验证 override 越权拒绝、无激活副作用与本租户正向成功。Provider 模型本身使用 camelCase BSON，测试的 `workspaceId/providerId/isActive` 回读键与生产序列化一致。
- `jwt_auth` 是本批唯一默认执行目标，4/4 通过：JWT 开启时双 PEM 必填、RS256 issue→verify claims 往返、篡改映射 `token_invalid`、过期映射 `token_expired`。它验证 JWT 纯密码学 API，不验证 middleware 的 cookie/Bearer 优先级、AdminUser 撤权或 token revocation；后两者的生产缺口继续归 SR-014。
- `auth_middleware_integration` 对用户名不存在/密码错误同码、session workspace 三档选择、登出幂等和 switch workspace ACL 有真实 DB/helper/Handler调用价值。但它同样手工注入 `AuthenticatedAdmin`，没有穿过 Router middleware；名为 expired 的用例实际只删除 session 后断言 `SessionNotFound`，没有覆盖 `expires_at <= now → SessionExpired`。

### 手写 Filter 冒充 Handler 隔离回归（FACT，SR-176）

- 新发现 SR-176：`workspace_isolation` 的联系人 IDOR、两批 admin handler sweep 与 MCP passthrough 用例均未调用其注释列出的生产 Handler；测试自己添加 `{_id, workspace_id}` 或 `{account_id, workspace_id}` 查询 Mongo，再把查询隔离成功当成 Handler 修复证据。生产 Handler 即使删掉该条件，这些测试也不会失败。
- `products_workspace_isolation` 同样只直接查询 collection，能证明 `(workspace_id, product_id)` unique 与等值过滤语义，却没有从 Product CRUD Handler 注入 admin workspace，也没有执行设计要求的 workspace A admin 访问 workspace B product/outcome。当前定向反查的 Product Handler 确实带正确过滤，因此本项不宣称现有生产代码已越权；问题是测试无法保护未来回归。
- 同根的测试声明漂移还包括上述 session 过期分支。该问题与 SR-004 正交：SR-004 是 ignored Docker job 即使失败也不阻断；SR-176 是即使这些用例真的运行并绿色，也没有执行其声称保护的端点或分支。

### 执行与系统结论

- 本批共有 27 个测试，其中 23 个标记 `#[ignore]` 且依赖 Docker/Mongo，本机未执行；它们会被 CI 的全量 `--ignored` job尝试执行，但该 job继续为 soft gate（SR-004）。默认 `jwt_auth` 4/4 通过、0 失败、0 ignored。
- 隔离测试必须让授权身份、路由参数、请求体、目标对象和副作用在同一真实调用链上相遇。直接 collection 查询适合证明索引/迁移或 BSON filter 语义，不能以 Handler 名称对外宣称端点授权已验证。JWT 测试 PEM 属固定公开测试材料，不是生产秘密；其安全价值来自只在测试目标消费且冻结身份稳定，而不是隐藏其内容。

## B09D：联系人运营、Campaign、成交与自治读模型集成测试

覆盖文件：9 个，共 2,867 个物理行、2,663 个非空行、109,409 个台账字节：Campaign 派发与成交粗筛迁移、联系人手工标签/运营画像/批量托管、运营 active-view、成交产品快照、自治指标和疑似成交闭环。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF；9/9 Git clean-filter blob 与冻结提交一致。`operation_view_integration.rs` 工作树为 CRLF，原始字节比台账多 11 字节，但 Git clean-filter blob 与冻结对象完全一致、工作树 clean，按行尾差异豁免计入。

### 真实 Handler 与 committed side effect 覆盖（FACT）

- `campaign_dispatch_integration` 直调正式 preview/dispatch Handler，覆盖零命中拒绝、workspace 过滤、CampaignSend 去重、Follow-up Task 创建、completed 重入拒绝、粗筛上限和可返回的 task insert 失败补偿；去重场景还区分 `lastDispatchTargetCount=命中总数` 与 `dispatchedCount=新入队数`。它能保护正常返回路径，但进程崩溃、补偿失败、worker 抢占和并发 dispatch 仍不在覆盖内，继续归 SR-075–076；前端未保存编辑后 Campaign spec 继续归 SR-077。
- `contact_manual_tags_integration` 直接约束 normalize/条数/字符边界，并由真实 Handler 验证落库与跨 workspace 拒绝；`contact_operation_profile_integration` 真实复现前端不带 `profileAttributes` 时保留 AI 画像，并验证非空显式写入。`contacts_batch_enable` 穿过真实批量托管与 initial-profile task handler，覆盖账号存在性、同步 initial state、老客户状态保留、任务幂等，以及 contact gone/unmanaged 早退仍写 sent 终态。
- `operation_view_integration` 真实预热 DomainProfile/Taxonomy cache 后调用 active-view Handler，验证 profile dimensions、关系类型与对话模式字典投影。该用例仍受 SR-174 的进程级缓存跨测试串扰影响；单独运行时的业务调用链是有效的。
- `suspected_deal_e2e` 与 `outcome_snapshot_freeze_integration` 真实调用 list/approve/reject，验证 `staff_confirmed` 成交、pending partial unique、拒绝零 outcome，以及从 active Product 生成订单式冻结快照、产品后续改名改价不污染历史。它们只覆盖成功提交；CAS 后校验/写入失败导致假 approved 继续归 SR-057，客户端可伪造 actor 继续归 SR-058。
- `outcomes_autonomy_endpoint` 调用真实聚合 Handler，能验证零分母、revision/hold/taxonomy/产品声明阻断、Outbox 与 Planner 的公式和响应形状；输入 run/outbox/event 是测试手工插入，因此不证明 Gateway 会产生正确事实。跨多查询非快照导致读模型内部不一致继续归 SR-078。

### 手写粗筛与既有风险边界（FACT）

- `campaign_segment_coverage` 对 m030 回填、幂等和 outcome/deal 双键兼容使用真实 migration，属于有效持久化覆盖；其中 `coarse_query_includes_legacy_event_missing_fields` 却在测试内重写 `build_segment_coarse_filter` 的 `$elemMatch`。生产 builder 漂移时该用例仍可绿色，属于 SR-176 的同类证据边界，不另编号。
- 批量托管 happy path 的候选均为正常人类账号，未覆盖系统号拒绝（SR-147）或前端切号后把旧候选提交到新账号（SR-153）；真实 Handler 测试不能由此反证上游身份错绑。Campaign 正常补偿用例也不能反证 crash window（SR-076），疑似成交成功用例不能反证两步提交裂缝（SR-057）。本批定向反查未发现独立于这些既有 finding 的新生产根因。

### 执行与系统结论

- 本批共 41 个测试，38 个标记 `#[ignore]` 并依赖 Docker/Mongo。`contact_manual_tags_integration` 默认运行报告 26/26 通过、2 ignored；其中只有 3 个是该文件自己的非 Docker 标签纯函数测试，其余通过数来自每个 `mod common` 宿主重复编译的共享单测。
- 尝试一次联合编译/运行 9 个目标，外部命令在 184 秒超时后留下 MSVC 并行链接进程；9 个测试可执行文件均已生成，但 Windows 链接器仍持有文件，无法取得完整运行结果。本轮启动的 Cargo/rustc/link 孤儿进程已按 PID 全部清理。不得把“可执行文件生成”记作 9 目标测试通过，Docker ignored 用例本机仍未执行。
- 高价值集成测试应同时冻结入口身份、业务状态机和最终副作用，并为每个跨集合提交点加入 crash/barrier 故障。成功 happy path 与可返回错误补偿都不能替代 durable intent、事务或 reconciliation；手写等价 filter 只能作为 BSON/migration 单测，不能冒充生产 Handler 回归门。

## B09E：Outbox、Reaction、发送资产与运行恢复集成测试

覆盖文件：10 个，共 3,684 个物理行、3,411 个非空行、138,994 个台账字节：Outbox 主集成套件、Reaction claim/停止取消、quiet-hours 延迟、媒体与名片入队、Run Envelope、Decision Review 状态、发送台账及 Task reclaim。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF；10/10 Git clean-filter blob 与冻结提交一致。`decision_review_status_e2e.rs`、`media_asset_send_integration.rs`、`outbox_integration.rs`、`reaction_claim_lock.rs`、`reaction_stop_cancels_outbox_integration.rs` 的工作树原始字节因 CRLF 大于台账，Git 工作树 clean 且 blob identity 一致，按行尾差异豁免计入。

### 真实投递与停止链路覆盖（FACT）

- `outbox_integration` 真实调用 `enqueue → atomic_claim_pending → process_entry` 并接 wiremock MCP，覆盖成功凭据后 sent、业务否定回执不写 outbound、三次失败终止、run 级 mixed status 顺序无关、陈旧二次安全门、lease reclaim、唯一键去重、软上限告警、账号 pacing，以及 Gateway 真实决策产物进入 Dispatcher。恢复专项还验证 reclaimed text 先做权威 `chat_search`、必要时回落本地 MCP 日志，已送达时不重发，并锁定恢复核对先于 pacing。它是本批最完整的生产控制流测试。
- `reaction_stop_cancels_outbox_integration` 真实调用 `record_user_reaction`，穿过 claim、LLM reaction 分析、停止结果映射与 Outbox 批量取消，验证两条 pending 行都变为 canceled；`quiet_hours_deferral` 真实调用 `ensure_wake_followup_task`，验证未来任务、review_required、expires_at、观测事件和重复调用幂等。
- 媒体与名片文件的第二个用例都真实调用 Outbox `enqueue`，能证明同一 asset/card 二次入队命中唯一键、不同对象各自建行，且名片/媒体身份互斥。它们只验证入队，不验证远端物理发送、timeout/reclaim 或 post-hoc delivery verification；名片恢复后确定性重发继续归 SR-052。
- 上述用例全部固定 `workspace_id=default`。因此它们不能反证 MCP 凭据/日志错 workspace（SR-011）、事件错归默认 workspace（SR-024）、账号计数与 pacing 跨 workspace 合并（SR-025）、Outbox 幂等键跨租户冲突（SR-026），或非默认 workspace 停止意图取消错误对象（SR-027）。in-flight 取消无法撤回已越过发送门的调用继续归 SR-066。

### 复制私有逻辑、未接线原语与空壳测试（FACT）

- `worker_reclaim` 的测试名声称 stale task 会被恢复、fresh task 会被跳过，正文却只 insert 后原样 find，既不调用私有 `reclaim_stale_running_tasks/tick`，也不等待 worker；两例无论生产回收逻辑如何变化都可保持绿色。该事实已在 B04I 记录，生产 claim/reclaim 所有权缺口继续归 SR-036，不重复编号。
- `reaction_claim_lock` 在测试内重写 `find_one_and_update` filter 与 `$set`，没有调用 `record_user_reaction`；它能证明 Mongo 单文档 CAS 的测试脚本只成功一次，不能保护生产 claim 条件、stale analyzing 恢复或 LLM 调用次数。完整停止串联文件提供了单请求正向覆盖，但没有并发串联。
- 媒体“审核后可发”和名片“enabled+approved 可加载”均直接改 collection，再复制生产 loader filter；媒体的 stage 命中又复制纯谓词。它们是 BSON/fixture 语义测试，不是 upload/review Handler 或生产 candidate loader 回归。`decision_review_status_e2e` 同样复制 `fetch_run_status` 与 JSON 投影，未调用 reviews Handler；`send_ledger_integration` 只做 collection CRUD/手写 outcome 回填，没有调用 `record_send` 或 `scan_send_ledger_outcomes`。
- `run_envelope_integration` 的 started、duplicate key 与 terminal fallback 用例确实调用已实现 helper；但生产 Gateway 未调用这些 helper。所谓 panic 用例也不触发 Reply Agent panic，只由测试直接写 `failed_before_decision`。因此绿色只能证明未接线原语自身可写库，不能证明首个 LLM 前已有信封或 panic 会被生产捕获；该根因已由 SR-019 覆盖。

### 执行与系统结论

- 本批共有 34 个测试，34 个全部标记 `#[ignore]` 并依赖 Docker/Mongo；本机未执行。CI 的全量 ignored job 会尝试运行，但仍是 soft gate（SR-004）。本批结论来自冻结源码、生产调用点与既有 finding 的对账，不把“可编译”或“脚本内直接写出期望状态”计为运行闭环。
- 可靠投递测试必须区分三层：Outbox 行唯一只能证明不重复建 intent；Dispatcher 的远端成功凭据与恢复核对决定是否重复物理发送；业务投影/台账则决定审计是否与送达一致。测试应从真实生产入口驱动每一层，并在远端成功后本地提交前注入 timeout/crash，同时使用两个 workspace 的同名 account/contact 验证身份包络贯穿。复制私有 filter 或直接更新终态只能作为低层持久化测试，不能用“端到端”“恢复”命名外推生产保障。

## B09F：Webhook 去抖、任务领取与 Gateway 主链集成测试

覆盖文件：6 个，共 2,123 个物理行、1,980 个非空行、79,652 个台账字节：去抖 runner 与 Gateway barge-in、Operations 立即复核 claim、入出站时间拆分、Webhook 建档，以及 Memory/Revision/Knowledge tool-loop happy path。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF；6/6 工作树对象、Git clean-filter blob 与台账 SHA-256 一致。

### 单进程真实控制流覆盖（FACT）

- `debounce_pipeline_integration` 真实调用 `register_inbound` 与 `run_debounce_pipeline`：同联系人三条快速入站只产生一次 Gateway run/Outbox、快照使用最后一条 inbound，runner 存活期间晚到入站只 bump generation、不重复 spawn。`debounce_barge_in_run` 则直接给正式聚合 Gateway 注入恒真/恒假 guard，验证 superseded 分支不入 Outbox、不推进 `last_agent_run_at`，正常分支落 approved 并入队。两者合起来有效保护单进程正常聚合和 Gateway 协作式中止，但没有确定性制造“Gateway 执行中真实晚到”、进程退出或双实例竞争。
- `review_task_now_claim` 直调正式 Handler，验证 running/terminal 任务返回 Conflict，pending 任务以状态 CAS 写 `claimed_at` 后进入 memory consolidation 无候选终态。它证明当前入口的原子 claim happy path，但没有让 review-now 长跑超过 lease，也没有启动 heartbeat、stale scanner、cancel 或双执行者；这些所有权/晚到提交缺口继续归 SR-034。
- `webhook_contact_upsert_integration` 直调正式 Webhook：公众号与群消息会持久化但不建 Contact，真人 roster 命中时使用 roster 昵称/头像，未命中时仍建 normal Contact且不信任 payload 脏昵称。用例不带 appId、固定默认账号且新建 Contact 为 normal，因此没有进入 managed debounce，也不能反证 runner reload 丢 workspace 的 SR-012。
- `happy_path_run` 真实调用 Memory consolidator 与 Gateway，分别验证候选消费/Memory Card 写入、single-shot revision 的四次 LLM 与终审/Outbox，以及 Knowledge Agent 的 catalog→open_chunk→answer trace。它们是有 committed side effect 的成功链；Memory 跨集合崩溃恢复继续归 SR-029，revision 已知失败稿回退继续归 SR-023，Knowledge 正文作用域继续归 SR-030，不能由 happy path 反证。

### ACK 后不可恢复的进程内调度（FACT，SR-177）

- 新发现 SR-177：正式 Webhook 先把 inbound 写 Mongo，随后只在静态进程内 `PENDING` 保存待处理 generation/deadline，并裸 spawn runner 后立即 ACK。模型没有 processing marker，启动也没有回扫；进程在去抖、Reaction、LLM 或 Gateway 期间退出后，消息虽在库中却没有 durable job 可恢复。
- 上游重送不能自愈：消息唯一键冲突会直接返回 `duplicate=true`，不会重新注册 runner。runner panic 也只删本地 Map并写 best-effort 事件；没有未来新入站时，本条可无限期无人决策。多副本的 Map 各自独立，同联系人入站分落不同实例时又可各起 runner，重复决策/回复。现有两份 debounce 测试都在同一进程显式持有 runner state/handle，只验证成功执行，不覆盖该 durable ownership 边界。
- `last_inbound_split` 两例完全由测试直接执行与生产同形的 Mongo update/pipeline，没有调用 Webhook 或 `send_outbound_message`；它只能证明手写 BSON 会保持另一方向字段，生产更新字段、filter 或 fail-soft 时序漂移不会让测试变红。

### 执行与系统结论

- 本批共有 17 个测试，17 个全部标记 `#[ignore]` 并依赖 Docker/Mongo；本机未执行。CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结测试、生产调用点、启动接线与既有 finding 的双向对账，不把源码可编译或手写同形 update 计作运行证明。
- 可靠 webhook 应把“已接收”与“已处理”建成两个可恢复事实：ACK 前写 durable inbox/job，worker 以 generation/owner/lease claim，最终 run/outbox 提交后再完成；DuplicateKey 重放应确认对应处理已完成，否则补排。单进程 Map 可作为局部 debounce 优化，不能成为唯一队列、互斥锁或恢复来源。

## B09G：Knowledge Wiki 持久化与治理集成测试

覆盖文件：8 个，共 2,223 个物理行、2,015 个非空行、84,543 个冻结 Git blob 字节：Chunk 批量核验/归档与反向引用、软锁生命周期、Chunk PUT 字段保留、AI revision 红线、Guide 部分应用、Integrity D2 计数、Gap signal 三类规则与 Lessons 聚合。全部按冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 独立连续读取到 EOF。审查时 7/8 工作树 clean-filter blob 与冻结对象一致；`tests/wiki_gap_signals_3kinds.rs` 的当前 clean blob 是冻结提交之后的版本，故另用 `git cat-file` 把冻结 blob `0562323c290aa71dbd47a954cc4b32b590a05e0b`（21,416 字节、540 行）从头读到 EOF。CSV 的 `bytes/sha256` 保留建账时原始工作树字节口径（含行尾），不拿 LF-normalized Git blob 摘要覆盖。

### 真实生产入口与持久化不变量（FACT）

- `chunk_batch_ops` 直调正式 verify/reject/batch verify/archive/referrer Handler，验证 D2 quote+anchor 门、状态落库及每条成功动作的 revision；auto-verify 还真实消费 mock LLM。它证明正常成功链能写审计，但 auto-verify 在 revision 失败前已累计 processed、随后吞错的生产缺口继续归 SR-107，成功 fixture不能反证写失败虚报。
- `chunk_put_preserves_unmodeled_fields` 直调正式 PUT，证明正常路径会保留 provenance/wiki type/locked fields/created_at，执行 locked-field 恢复、updated_at OCC，并在 revision insert 成功时留下 human patch 行。生产顺序仍是主 Chunk replace 成功后 best-effort insert revision，审计失败会 warn 后返回 `{ok:true}`；这是 SR-108 已记录的 CRUD 与不可变 revision 非统一提交边界。该测试未注入 revision 写失败，不能证明“内容提交必有历史”。
- `chunk_revision_ai_draft_integration` 真调 `apply_chunk_revision`，有效锁定 AI patch 强制 `draft+needs_review`、数组 union、updated_at CAS 冲突及成功补删时 revision 数量与成功 writer 对应。它没有覆盖相同 patch 的 provenance hash 失真、revision insert 后 replace 网络失败、CAS 补删失败或 timeline 过滤；这些仍归 SR-114。并发测试证明 OCC 可拒绝部分 stale writer，不等于双写已事务化。
- `wiki_gap_signals_3kinds` 真调 structural lint、sweep 与在线 recall persist，覆盖 missing/suggestion/contradiction 生成和自愈、同 kind 精确合并，以及 16 并发 writer 在 partial unique+upsert 下收敛并保留全部 query。冻结版的 `recall_signal_merges_into_legacy_row_without_persisted_dedup_key` fixture 本身不足 40 字，追加后缀后前 40 字不同，会在调用生产 persist 前的 `assert_eq!` 确定失败，因此不能作为 legacy 兼容已运行的证据；当前 HEAD 已延长 query 并新增长度断言修复该测试，但后续修复不计入冻结覆盖。其余 gap 用例仍是高价值持久化回归，不是测试内重写 filter。
- `lessons_learned_filters` 真调正式聚合器，并分别从 `agent_decision_reviews` 与 `agent_run_logs` seed 权威形状，证明 success/failure/blocked 三类可生成 lesson；它验证的是聚合读取与 upsert，不证明上游 Gateway/Reaction 一定写出这些事实。`integrity_report_d2_e2e` 真调正式 report builder，锁定 active 无 anchor 的计数口径。
- `guide_apply_partial_validation` 真调 `apply_contact_changes`，证明 LLM 越界枚举按字段跳过、合法字段继续提交、全无效不空写；Guide candidate 基线、scope、runtime Document 与 post-commit response 风险继续归 SR-091–095，局部 helper 正向测试不反证完整确认协议。

### 名称大于证据的软锁测试（FACT）

- `chunk_lock_lifecycle` 的文件名和头注释称“生命周期集成测试”，实际默认执行的四例只测 TTL 常量、事件序列化、`is_expired` 与裸 Tokio broadcast；唯一 DashMap smoke 也在测试内自行 insert/remove，不调用 acquire/release Handler、不构造 workspace 归属或真实编辑请求。它无法保护跨 workspace key、双 acquire、旧 owner release、绕过锁直写或多副本行为，均继续归 SR-096；late subscriber 正向投递也不能反证 `lagged` 后客户端不重同步的 SR-141。
- 本批其它文件虽然进入生产函数，但均以顺序成功为主；直接 seed DB 是构造前置事实，不等同验证事实的生产写入来源。尤其 Lessons、Integrity 与 Gap 的绿色结果只能证明当前读取/规则对给定数据有效，不能外推为 Gateway、导入、repair 或 worker 的完整闭环已通过。

### 执行与系统结论

- 本批共有 33 个测试：29 个 `#[ignore]` 且依赖 Docker/Mongo，本机未执行；`chunk_lock_lifecycle` 的 4 个无 Docker 默认测试本轮也未重复运行，结论来自冻结源码与此前生产边界对账。冻结版 legacy gap 用例即使 Docker 可用也会先因 fixture 前提失败；该问题已在当前 HEAD 修复。CI 的 ignored integration job仍是 soft gate（SR-004）。
- 本批没有新增 finding 编号。有效测试确认了 D2、AI draft、OCC、gap dedup和 Guide 部分应用的局部不变量；剩余风险均已由 SR-091–096、SR-101、SR-107–108、SR-114、SR-141覆盖。关键补测应在写链中注入主行/revision 任一侧失败、补删失败与响应丢失，并要求服务端返回同一个 committed mutation identity；对协作锁则必须从真实 Handler 到真实写门验证 workspace+owner+generation，而不是直接操作 DashMap。

## B09H：Prompt / Evolution 发布、回滚与红线集成测试

覆盖文件：8 个，共 1,759 个物理行、1,601 个非空行、70,049 个冻结 Git blob 字节：Prompt shadow replay、Evolution release 红线、Prompt rollback、workspace scope、Prompt pack seed/align、手工 publish 对 Evolution 历史的保护、reset 后 Critic 重种，以及 Prompt create/publish 红线门。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；8/8 当前工作树 clean-filter blob 与冻结对象一致。

### 真实 Evolution 核心路径覆盖（FACT）

- `evolution_prompt_shadow` 真调 `run_shadow_replay`，从源 run、真实 inbound 与 Contact 反查到 mock Reply/Review，再持久化 ShadowReplay；同时覆盖 source message/contact 缺失的业务失败。它有效保护 retention 字段名与单次成功 replay，但源样本/Prompt 在评估期未冻结、候选不绑定发布基线的风险继续归 SR-049/SR-086，正常 fixture 不能反证。
- `evolution_release_redline` 真调事务化 `release_prompt`：禁词在 LLM 前拒绝、mock 语义违规拒绝、干净追加生成 version+1 current 并推进 proposal。它证明追加式候选的正常三闸与事务成功链，不覆盖候选评估后 current 漂移（SR-086）、commit 后事件/review intent 失败（SR-088）或纯删除绕过语义闸（SR-139）。
- `evolution_rollback_status` 真调事务化 `rollback_prompt`，验证 archived previous 恢复时同时转 active，以及 previous 行缺失会中止事务、保留 proposal/current。两个 fixture 的 `_id` 各自独立，能够进入真实 rollback；但没有 P1 release→P2 release→rollback P1 的 lineage 场景，旧 proposal 可翻掉更新 current 的问题继续归 SR-087。
- `prompt_pack_seeding` 真调 `ensure_prompt_pack_v2`，覆盖自定义 active/draft 保留、缺 key 补种、system drift 归档重种、Evolution chain 跳过、幂等、版本相同仍对齐、archived GC 和“是否写入”返回值。它们都是正常读写路径；首次探测 Mongo Err 被错误并入破坏性 reset、跨四集合中断的 P0 风险继续归 SR-138，测试没有故障注入。
- `reset_pack_preserves_evolution_critic_integration` 真调销毁性 reset 并确认 Critic 被重种、loader 可读；它只证明 reset 完整成功后的一个系统 key 存活，不证明人工 Prompt、Soul、Playbook、Domain 配置可恢复，也不覆盖中途失败，因此不能削弱 SR-138。

### 绿色测试的证据边界（FACT）

- `evolution_workspace_scope` 文件自己说明无法调用三个真实 Handler；正文直接重写 `{_id, workspace_id}` 的 find/count filter，再把 Mongo 返回 0 当作 detail/release/rollback 已隔离的证据。它也没有执行声称的 `released_by=admin.username`。生产 Handler 当前过滤经既有源码审查确认，但这组测试不能保护 Handler 漏条件或 actor 回退，属于 SR-176 的同类自证测试，不另编号。
- `prompt_publish_evolution_guard` 确实直调手工 publish Handler，并证明 Evolution 历史行未被删除；但成功断言只要求 draft 的 `status="active"`，完全不要求 `current_version=true`、旧 current 被降级或 previous lineage 正确。于是 SR-055 的核心 status/current 分裂发生时该测试仍可绿色，且它还把非 Evolution system 历史被物删断言成预期行为。
- `prompt_template_redline_gate_e2e` 名称称 create/publish 端到端，但因 Handler 可见性限制，大部分只直接调 `validate_prompt_edit` / `review_prompt_edit`；“publish 拒绝”用例仅在测试里读回 draft 后调用纯门函数，没有调用 publish。六例都覆盖禁词、已有锚或新增文本，未构造 deletion-only、无锚 constrained key 或审查器自删规则，不能发现 SR-139。

### 执行与系统结论

- 本批共 27 个测试，27 个全部标记 `#[ignore]` 并依赖 Docker/Mongo；本机未执行，CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、真实调用点和既有 findings 双向对账，不把 mock LLM 的单次成功或测试内复制 filter 外推为完整发布协议。
- 本批没有新增 finding 编号。有效证据确认了 Prompt shadow 的基本可达性、追加候选的三闸、rollback 缺历史时的事务回滚，以及 seed/align 的正常路径；关键未覆盖项继续由 SR-049、SR-055、SR-086–088、SR-138–139、SR-176承接。后续高价值测试应从真实 Router 进入，冻结 candidate base/released version，注入 commit 后事件故障，并把 `status/current/previous/releasedVersion` 作为同一版本状态机断言。

## B09I：Planner 主动触达、反馈退避与日历关怀集成测试

覆盖文件：4 个，共 1,025 个物理行、946 个非空行、33,573 个冻结 Git blob 字节：静默联系人跟进、承诺到期、日历关怀与 block-rate backoff。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；4/4 当前工作树 clean-filter blob 与冻结对象一致。

### 默认作用域内的真实 Planner 控制流（FACT）

- 四份测试都真实调用 `planner::tick`，不是在测试内重写候选 filter。`planner_silent_followup` 验证 managed、静默阈值、cooldown 与 review-required task；`planner_commitment_due` 验证 overdue/imminent 选择、旧 Plain commitment 跳过、24h commitment event 去重及事件明细；两者都检查任务与观测事件的实际落库。
- `planner_calendar_care` 通过真实 DomainProfile cache 与 OperatingMemory 驱动日历扫描：情感陪伴 profile 的今日纪念日会生成关怀任务，DEFAULT 销售 profile 无 date dimension 时整段短路。该测试有效保护行业范式开关与默认销售零扰动，但固定 `workspace/account=default`，不能反证非默认租户完全不被 worker 扫描的 SR-134。
- `planner_block_rate_backoff` 真实 seed `agent_run_logs` 后运行 tick，验证 4 blocked + 1 approved 达阈值时不建任务并写 backoff、样本少于 `min_runs` 时放行。生产查询按 `(workspace_id,account_id,contact_wxid,created_at)` 收口，并通过运行时 threshold resolver 取阈值；本批未发现新的账号归因串扰。

### “第二次 tick 幂等”与配额证据边界（FACT）

- silent 与 calendar 的第二次 tick 仍有第一轮 `pending` follow-up，因 `has_pending_follow_up` 跳过；测试没有先消费、完成或取消任务再重扫。calendar 本身也不检查同日同纪念日 emit，任务离开 pending/retry/running 后可再次生成。commitment 的第二轮另有近期 event 去重，但同样没有并发执行者或 check-then-insert barrier。
- 所有扫描器仍先 count pending/event/cap，再普通 `insert_one` task、再写 event；follow-up 没有业务 intent 唯一键。四份测试均为单进程串行 tick，未覆盖双实例同时通过检查、task 成功后 event 失败、响应不确定或配额原子 reservation，因此不能反证 SR-135 的重复触达与并发超发。
- calendar 专属 `daily_cap` 在每次函数调用都把 `calendar_emitted_today` 从 0 开始；测试只生成一个候选，也没有跨 tick 填满专属上限。共享日 cap虽然从当日事件重数，但仍是非原子“读取余额→写任务→写事件”。测试名称中的幂等不等于持久每日配额或 exactly-once 触达。

### 执行与系统结论

- 本批共 6 个测试，6 个全部标记 `#[ignore]` 并依赖 Docker/Mongo；本机未执行，CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、真实 `planner::tick` 调用与生产扫描器对账。
- 本批没有新增 finding 编号。有效测试确认默认 scope 的筛选、承诺分类、日历范式与 block-rate 冷启动门；跨租户扫描遗漏继续归 SR-134，持久幂等、任务/事件提交裂缝和每日配额继续归 SR-135。关键补测应加入双 tick/双实例 barrier、任务完成后同日重扫、task 写成功后 event 失败、日 cap 边界与午夜换桶，并对 durable intent/配额 reservation 做唯一性断言。

## B09J：人工请示、Principal 裁决与超时改派集成测试

覆盖文件：3 个，共 2,082 个物理行、1,913 个非空行、82,721 个冻结 Git blob 字节：Ask-Human Phase 1、改派推送时刻迁移，以及 Principal 决策通道。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；3/3 当前工作树 clean-filter blob 与冻结对象一致。

### 真实 Handler、Scanner 与 Relay 覆盖（FACT）

- `ask_human_phase1_e2e` 真实调用策略 PUT、admin resolve/reassign、统一 inbox/summary、跨 workspace resolve、deferred 分支与 timeout scanner。正常 resolve 会把台账置 resolved 并建一条 relay task，重复 resolve 不重复建任务；跨 workspace 请求保持 pending，deferred 也保持 pending 且不建 relay。这些是有效的单请求成功与授权回归。
- 同文件的三决策人超时用例使用 wiremock MCP 后真实运行 `scan_escalation_timeouts`，验证改派刷新 `updated_at` 后下一位获得完整窗口。`principal_decision_channel` 后半进一步覆盖推卡成功后改派、推卡失败不改派、quiet-hours、cap=1 自命中、失败后下一 tick 重试、链尾安抚间隔，以及授权过期与 relay 安全门。
- 正常流程不能证明提交原子性：admin 与微信入口仍先提交 resolved、再独立 insert relay task；测试只覆盖两步都成功和第二次请求已 resolved，未注入两步之间崩溃或 task insert 失败，故 SR-054 继续成立。timeout 用例也全是单 scanner 串行执行，不能反证 SR-039 的多副本重复推卡与晚到覆盖。
- 授权过期用例构造的 relay task 没有 `_id` 且未插入 tasks 集合，只直接调用 handler 并断言一次中性 MCP 与 awaiting 清除；它没有经过 Worker claim、终态和 stale recovery，因此无法发现任务保持 running 后重复裸发的 SR-035。

### 手写数据库切片与证据边界（FACT）

- `principal_decision_channel` 前半多项测试直接 insert/update escalation、Chunk、Contact 或 config，再读回同一集合；它们能锁定模型序列化和目标状态形状，但没有调用 pending 创建、领导解释、知识沉淀或 awaiting 写门，不能证明生产函数产生这些状态。领导回复跨 account 串扰继续归 SR-040，LLM 解释自锚定 verified 知识继续归 SR-037。
- `escalation_push_time_reassign` 的迁移用例真实调用 m031，能证明缺字段回填与已有值保留；所谓 reassign 用例则在测试内手写 `$set principal_wxid + last_pushed_at_ms`，没有调用 `reassign_escalation` 或骚扰门查询，不能保护生产改派逻辑。真实 scanner 用例提供了部分正向补偿，但仍不覆盖多执行者 fencing。
- 初次请示卡投递失败、timeout 多域策略错配、holding reply 数字护栏及过期 relay 终态分别继续由 SR-038、SR-041、SR-042、SR-035 承接；本批没有证据推翻这些生产缺口。

### 执行与系统结论

- 本批共 30 个测试：28 个 `#[ignore]` 且依赖 Docker/Mongo，本机未执行；`principal_decision_channel` 的 2 个默认纯函数测试本轮未重复运行。CI 全量 ignored job仍为 soft gate（SR-004）。
- 本批没有新增 finding 编号。有效证据确认了 admin workspace guard、deferred 保持 pending、串行 timeout 改派/重试和 relay 安全门；关键未覆盖项继续由 SR-035、SR-037–042、SR-054 承接。高价值补测应把裁决与 relay intent 作为同一 committed outcome，并加入双 scanner barrier、真实 running relay 的超时回收、首推失败重试和同 workspace 多 account/domain 隔离。

## B09K：Knowledge 导入、PDF 与自动摄取 Worker 集成测试

覆盖文件：3 个，共 791 个物理行、713 个非空行、28,380 个冻结 Git blob 字节：ImportJob 生命周期、PDF 导入冒烟与自动摄取 Worker。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；3/3 当前工作树 clean-filter blob 与冻结对象一致。

### 真实导入与摄取路径覆盖（FACT）

- `import_pdf_smoke` 真调 `import_pdf_bytes → ingest_chunked_text`，用运行时生成的 PDF 覆盖 fence、无 fence fallback blob 与空文本拒绝，并从数据库确认产物均为 `draft + needs_review`。它有效保护“AI 永不自动 verify”红线和文本型 PDF 的基本可达性，但没有在 document 已插入、部分 Chunk 已写或 revision 失败处注入故障；Document/Chunk 分步提交与重试重复继续归 SR-112。
- `ingest_worker_smoke` 真调 `run_one_round` 并以 wiremock 驱动 RSS：覆盖成功落草稿、HTTP 500 失败计数、not-due 不刷新 checkpoint、due 正常拉取，以及无 ETag 时按 content hash 串行去重。它证明单执行者正常 checkpoint 语义，不覆盖两个 worker 同时读取同一 due source、慢请求期间管理员改 URL、success/failure 竞态或旧结果 finalize，故 source 无 claim/generation 的 SR-117 继续成立。
- smoke 使用管理员可保存的本地 HTTP URL作为 fixture，未调用 source create/update Handler，也没有验证 scheme、目标 IP或重定向逐跳限制；因此不能反证持久化 SSRF 的 SR-109。PDF fixture 只含可提取文本，不覆盖扫描件 OCR、加密文件、资源/页数上限或超大响应压力。

### ImportJob 生命周期测试的 ABA 盲区（FACT）

- `import_job_lifecycle` 五例都直接操作 `import_jobs` collection：验证 BSON 字段、手写 stale filter、手写 workspace filter、手写 terminal CAS 与 TTL 字段，没有启动 `run_import_worker/tick`。workspace 用例把 Handler 应有的 `{_id,workspace_id}` 条件复制到测试，不能保护真实 Handler 漏条件，属于 SR-176 同类证据边界。
- 所谓终态竞态测试只把 job 留在 `pending`，再证明旧 worker 的 `{_id,status:"running"}` 更新不命中；关键 ABA 是 B 已把同一 job 重新 claim 成 `running`，此时 A 的旧 heartbeat、progress 与终态都会再次命中。测试注释声称覆盖“甚至已被另一 worker 认领”，正文却没有构造该状态，正是 SR-136 已记录的缺口。
- 本轮复核确认冻结生产实现的滑动 lease 字段是 `claimed_at`，不是 `locked_until`；已同步纠正 SR-136 与 B07C 的术语，机制、严重度和建议不变。

### 执行与系统结论

- 本批共 13 个测试，13 个全部标记 `#[ignore]` 并依赖 Docker/Mongo；本机未执行，CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、真实调用点和生产状态机对账。
- 本批没有新增 finding 编号。有效证据确认文本 PDF/RSS 的草稿红线、单轮 checkpoint 与串行 content-hash 去重；关键未覆盖项继续由 SR-109、SR-112、SR-117、SR-136、SR-176 承接。高价值补测应加入 A claim→超时→B claim 后旧 A 全写失效的 barrier、双 ingest worker、抓取中改 URL，以及 document/chunk/revision 任一步失败后的幂等恢复。

## B09L：数据库迁移、版本索引、OCC 与事务化管理流集成测试

覆盖文件：8 个，共 1,233 个物理行、1,147 个非空行、44,612 个冻结 Git blob 字节：DomainSchema 持久化、m018/m029 数据迁移、Memory Card OCC、迁移框架幂等、OperatingMemory 并发首建、Ops 多版本索引启动，以及 Taxonomy/Guide 事务回归。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；8/8 当前工作树 clean-filter blob 与冻结对象一致。

### 真实事务、迁移与索引入口（FACT）

- `transactional_admin_flows` 使用 replica-set Mongo、真实 Router、管理员会话与 HTTP 请求。Taxonomy 用例先插入 `version=i32::MAX` 的历史行；生产 `saturating_add(1)` 仍得到同版本，事务内新行触发版本唯一键冲突，候选 claim 回滚为 pending，移除冲突后同请求可成功提交。Guide 用例用 event dedupe 冲突在事务后段制造失败，确认 Contact/Playbook 回滚、preview 留下可重试失败协议，移除冲突后只提交一次且 replay 返回 Conflict。这是有效的跨集合事务失败/重试证据，不是手写终态。
- 事务成功链仍不覆盖候选已存在 canonical 的 alias 合并，因此不能反证 SR-061；Guide 也没有在 preview 与 apply 之间修改基线、伪报影响范围、注入非法 runtime 或制造 commit 后读失败，SR-091、SR-093–095 继续成立。事务只保证被纳入 transaction 的写原子，不证明候选就是管理员预览时确认的对象。
- `m018_backfill_domain_stage` 与 `m029_cleanup_contact_identity` 都直接调用正式 `run_step` 并二次执行，分别验证 legacy 顶层字段只回填不覆盖现有 domain，以及非真人 normal 清理、roster 回填、managed 保留和消息不删。m029 fixture 只有 default workspace/account，未构造同 wxid 跨账号 roster，无法发现 SR-009 的全局 wxid 映射污染。
- `ops_versioned_index_boot_brick` 真调 `ensure_indexes`，确认 Domain Config/State Policy 多版本数据不会因残留旧 unique 索引启动失败，同时四元版本唯一键仍拒绝重复版本。`operating_memory_insert_idempotent` 真调 `load_or_create_operating_memory` 并发四次，证明首次建档 E11000 输者会回读赢家且最终只有一行。

### 绿色测试的证据边界（FACT）

- `domain_schema_persistence_e2e` 证明 snake_case 持久字段可被 loader 读取、required 约束生效；所谓 activate 测试却在正文手写 `update_many(false) → update_one(true)`，没有调用 activate Handler，也只覆盖顺序成功。它无法保护删除 active 旧版、第二步失败、双 activate 或多 active 随机读取，均继续归 SR-056。
- `migrations_idempotency` 只确认启动后 marker 数等于迁移清单且二次 runner 不增加行；它没有检查每条迁移的后置条件。生产守卫分支返回 `Ok(())` 后仍写 marker 的 SR-010 即使存在，这个计数测试仍会绿色。
- `memory_card_write_occ` 是确定性空壳：唯一测试只启动 `TestApp` 后引用变量，未 seed OperatingMemory、未并发调用 `apply_operating_memory_update`，也没有版本、modified_count 或最终内容断言。文件注释列出的 OCC 性质没有一条在正文执行；Memory consolidation 的跨集合恢复缺口继续归 SR-029。
- OperatingMemory 首建并发用例只保护 create 分支的唯一键回读，不覆盖已有 Memory Card 的双 writer、OCC 输者后的 Contact/候选/任务副作用，不能替代真实 consolidation barrier。迁移用例也都在新建单租户测试库运行，不能证明已执行 marker 的历史生产库可安全重跑或修复既有污染。

### 执行与系统结论

- 本批共 13 个测试，13 个全部标记 `#[ignore]` 并依赖 Docker/Mongo（事务用例还要求 replica set）；本机未执行，CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、真实入口与既有 findings 对账。
- 本批没有新增 finding 编号。有效证据确认 Taxonomy/Guide 的事务回滚、两条迁移的单租户正常语义、多版本索引重建和 OperatingMemory 并发首建；关键未覆盖项继续由 SR-009–010、SR-029、SR-056、SR-061、SR-091、SR-093–095 承接。高价值补测应让 Memory OCC 真正双写，加入 m029 跨账号同 wxid、迁移 marker 后置条件审计、DomainSchema 双激活/中间失败，以及 Guide commit 后响应故障。

## B09M：协议校验、安全门与 Taxonomy 审计测试

覆盖文件：7 个，共 1,484 个物理行、1,357 个非空行、58,705 个冻结 Git blob 字节：标注质量门、Conversation Mode 决策 schema、revision 后动作闸复检、状态迁移性质、视觉导入安全门、Taxonomy flags 与版本审计。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；7/7 当前工作树 clean-filter blob 与冻结对象一致。

### 真实 validator、归一器与 Handler 覆盖（FACT）

- `annotation_quality_gate_integration` 真实调用 `normalize_target_stages`、媒体审核 Handler 与联系人 assist override Handler：验证 taxonomy alias 归一/越界拒绝、审核状态与事件正常落库、override 写入/清除/闭集及跨 workspace NotFound。Multipart upload 本身未进入，但 target-stage 判定使用的是端点共享生产函数。
- 媒体审核测试固定 default workspace，且只覆盖业务更新与审计都成功。生产实现先提交素材状态，再 best-effort 调固定 default workspace 的公共事件 helper；非默认租户错账继续归 SR-024，审计失败也不会回滚审核。
- `conversation_mode_decision_schema` 与 `string_fact_risk_guard` 都直接调用生产纯函数。前者锁定 DEFAULT 四模式的合法、缺失、漂移值、sunset 与 tool-calling 语义；生产决策会以 active DomainProfile 的动态模式集合覆盖默认值，本文件未验证该动态接线。后者文件名已与正文漂移，实际测试 `check_state_transition` 的空机器、未知目标、initial、allow-from-any 与理由格式，不再测试产品声明字符串风险。
- `vision_safety_gate` 真实进入图片导入 Handler，覆盖无视觉 provider 拒绝、主模型视觉成功、空输出与 `draft+needs_review`。它没有覆盖专职副模型真实 HTTP、备用切换、能力关闭/删除生命周期（SR-167），成功写仍复用非事务 Document→Chunk 链（SR-112）。
- `taxonomy_version_audit_integration` 直调 publish/rollout/rollback Handler，证明顺序成功时事件带 admin/action/scope/version；版本 current 切换仍是多次独立写且审计 best-effort，零/多 current 继续归 SR-008。正常事件存在不等于业务变更与审计同一提交。

### 空壳与自证测试边界（FACT）

- `revision_recheck_action_gate` 是确定性空壳：唯一 ignored test 只有注释，没有启动 TestApp、构造 revision、调用 Gateway 或任何断言。生产源码确实在初次 finalize 与 revision 二审成功后各调用 `apply_state_action_gate`，但这个测试文件不会在接线删除、Held 状态漂移或 Outbox 误入队时变红。
- `taxonomy_flags_e2e` 不调用 PATCH Handler，而是在测试内复制 `value.isTerminal/value.isReactivationTarget` 的 `$set` 后强类型读回。它只证明 BSON camelCase 键可反序列化，无法保护 Handler、响应投影或前端 round-trip；正式列表投影丢 flags、普通编辑清零的 SR-168 即使存在仍会绿色。
- Taxonomy 版本审计三例均为单请求成功路径，没有注入业务第二步失败、事件 insert 失败、双 rollout/rollback 或响应丢失；媒体审计同样没有验证审计失败后的可观测告警。顺序 happy path 不能替代事务、唯一 current 或 durable audit intent。

### 执行与系统结论

- 本批共有 35 个实际测试案例：15 个 `#[ignore]` 依赖 Docker/Mongo，20 个默认纯函数/PBT 本轮未重复运行；CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、生产调用点与既有 findings 对账。
- 本批没有新增 finding 编号。有效证据确认 DEFAULT 决策枚举、状态迁移 fail-closed、target-stage 归一、assist IDOR、视觉待审红线和 Taxonomy 审计正常路径；关键未覆盖项继续由 SR-008、SR-024、SR-112、SR-167–168 承接。高价值补测应真实驱动 revision→action gate→Outbox、动态 DomainProfile 模式、Taxonomy flag API/UI 往返、审计写失败及 current 切换并发/中断。

## B09N：跨域运营状态与 DomainProfile 主链集成测试

覆盖文件：3 个，共 2,793 个物理行、2,581 个非空行、116,492 个冻结 Git blob 字节：Gateway 运营状态派生、跨域状态迁移纯函数，以及 DomainProfile CRUD/发布/激活与状态机联动。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；3/3 当前工作树 clean-filter blob 与冻结对象一致。

### Gateway 与跨域状态机的有效覆盖（FACT）

- `c2_operation_state_derivation_e2e` 真实调用 `handle_managed_message`，用 mock Reply/Review 驱动完整 Gateway。它验证 DEFAULT 销售域下 `customer_stage` 优先于 decision.operation_state、合法迁移落库、非法 stage/state 跳转保留旧值并写审计、弱证据进入 observation、强证据实时写，以及指定审计事件写失败后回复仍入 Outbox。该文件不是手写 filter；它能保护生产写门和 fail-soft 接线。
- 这些 Gateway 用例固定 default workspace/account 和 DEFAULT 销售 profile，状态名、taxonomy 与 conversation mode 也都是销售值。它们不能证明自定义 profile 的维度、模式和状态机在真实 Gateway 中被同一套动态加载；并且多个 TestApp 默认并行时会共享进程级 Taxonomy/DomainProfile cache，可能跨测试数据库假红或假绿，继续归 SR-174。
- `c2_state_transition_cross_domain` 直接调用生产纯函数 `check_state_transition`，用医疗状态机验证非销售 key、initial、allowFromAny、合法/非法迁移和 unknown target。它有效证明判定引擎本身不写死 `new_contact`，但没有加载数据库 profile/config，也不覆盖 Gateway 派生、审计或持久化。

### DomainProfile 真 Handler 与手写 DB 边界（FACT）

- `domain_profile_e2e` Part C 真实调用 publish/rollout/rollback/update/activate Handler：覆盖已生效血缘的 current+active realign、纯草稿不自动激活、危险字段旁路稿与确认、部分更新字段保留、未知/托管键过滤、生成状态机发布、forbidsProactive policy 派生、相同机器幂等、手工 policy 保留、只刷新 current policy，以及切域后存量联系人幻影态迁到新 initial。直编状态机用例也真调 Domain Handler 并验证 policy 重派生。
- 这些均是顺序成功路径，没有在画像切换后注入状态机发布、policy 派生或联系人迁移失败，也没有双 activate/publish barrier。生产 activate 先切 profile，再把后三类步骤 best-effort 执行并固定返回成功；跨集合半提交继续归 SR-072，单集合 current/active 竞态与随机回落继续归 SR-043。
- 高风险确认测试只证明传入正确旁路稿 id 时后端 rollout 可生效；它不覆盖确认内容被编辑、任意 id 直 rollout、actor/TTL/hash 缺失或正式前端提交错误 id，故 SR-073 继续成立。AI 生成两例还会在缺环境或宽泛瞬时错误时运行期 return，且只确认草稿落库，不证明正式 UI 可找到并审核该草稿；SR-090 不受影响。
- Part A 明确以 helper 手写 create/update/publish/activate/list；其中“禁止删除 active”用例甚至只断言 active 前置存在，没有调用 delete Handler。它们只能锁 BSON 与预期状态形状，不能保护业务守卫、并发或中间失败。Real LLM 生成的正常候选也不能反证 Unicode key panic 等生成边界。

### 执行与系统结论

- 本批共 39 个实际测试：33 个 `#[ignore]` 依赖 Docker/Mongo（其中 2 个还依赖真实 LLM 配置），6 个默认跨域纯函数测试本轮未重复运行；CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、真实调用点和既有 findings 对账。
- 本批没有新增 finding 编号。有效证据确认 DEFAULT Gateway 的 stage/state 写门、非销售 FSM 纯判定，以及 DomainProfile 顺序发布/激活、机器/policy 派生和联系人重置；关键未覆盖项继续由 SR-043、SR-072–073、SR-090、SR-174 承接。高价值补测应加入双 activate/publish、每个跨集合步骤故障、真实自定义 profile Gateway、确认 hash/actor/TTL 与两个 TestApp cache barrier。

## B09O：运行隔离、模型切换与内容资产持久化回归测试

覆盖文件：7 个，共 1,425 个物理行、1,291 个非空行、52,410 个冻结 Git blob 字节：BehaviorSignal 落库、Management dry-run、LLM Provider 激活与重试、媒体资产 CRUD、Simulation 零副作用及素材 tag 组织。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；7/7 当前工作树 clean-filter blob 与冻结对象一致。

### 真实生产入口与局部不变量（FACT）

- `behavior_signal_smoke` 真调 `persist_signal` 与 Silence Worker `tick`，验证 partial unique 下同 dedupe key 收敛、不同类型各落一条，以及两轮扫描只生成一条 censored silence。成交事件用测试内 `$push` 旧 `deal_events` 键，只证明 serde alias 往返，不覆盖正式成交 Handler。全部 fixture 固定 default workspace/account，无法发现 BehaviorSignal 模型、builder、dedupe key 与索引遗漏 account 的 SR-137。
- `llm_provider_activate_integration` 真调激活 Handler并回查顺序成功后 DB 恰一条 active，缺失目标返回错误；但 TestApp 的 Registry 为 None，未验证实际热切。生产有 Registry 时先 swap 运行时，再清旧 active、设新 active，三个步骤没有统一提交或补偿；任一 DB 失败可造成 Registry/DB 分裂，清旧成功而设新失败还留下零 active。该一致性缺口已纳入 SR-013，旧产物缓存不失效继续归 SR-020。
- `llm_retry_jitter` 直接约束生产退避和错误分类纯函数，覆盖 Retry-After、指数基线、429/5xx 与 JSON/400/401；它不发真实请求，不证明每种 transport/provider 错误都被包装成预期文本变体，也不覆盖总预算或取消。
- `media_asset_crud_integration` 真调 metadata、toggle 与 delete Handler，验证 target-stage 越界零写入、workspace IDOR、单引用删除文件及有兄弟引用时保留文件。`structured_organization_integration` 真调 list Handler，证明手工 seed 的 tags 可精确筛选且不跨 workspace；Multipart 上传/换文件未进入，因此不保护上传 tags、换文件退审或新文件失败补偿。

### “隔离/零副作用”测试的证据边界（FACT）

- `dry_run_isolation` 完全没有调用 Management Agent、风险分类或任何 tool dispatcher；它手工插入 `dry_run` 状态的 session/command/tool-call，再观察自己没有写 contacts/tasks。生产中被误标 Readonly 的 `preview_campaign` 仍会在 dry-run 创建和修改 Campaign，SR-075 即使存在此测试仍稳定绿色。
- `simulation_no_sideeffect_integration` 真调 `simulate_user_dialogue`，但明确接受 Ok/Err 任一结果，只比较 Outbox 与 outbound message 两个集合。Simulation 可创建/触碰 OperatingMemory、写知识 gap 与 structural proposal，且未执行生产 finalize/action/revision 终态门；这些既有缺口正是 SR-048，窄快照无法证明“零副作用”。它也没有证明 MCP 未调用，只是测试 MCP 地址未被断言。
- Provider 激活测试只覆盖单请求正常顺序，没有 Registry swap、第一/第二次 DB 写失败、双激活 barrier 或响应丢失。媒体 delete 测试也只覆盖静态 0/1 引用：生产在 count=0 与物理删除之间若并发插入同 workspace+sha 路径的新资产，旧删除者仍会删共享文件，使新记录悬空；该反向 TOCTOU 已补入 SR-017。所有素材 fixture都是 workspace 共享行 `account_id=None`，不反证私有素材跨账号误操作的 SR-160。

### 执行与系统结论

- 本批共 23 个实际测试：17 个 `#[ignore]` 依赖 Docker/Mongo，6 个默认 LLM retry 纯函数测试本轮未重复运行；CI 全量 ignored job仍为 soft gate（SR-004）。结论来自冻结 blob、真实调用点与既有 findings 对账。
- 本批没有新增 finding 编号；扩写 SR-017 以纳入引用计数与物理删除之间的并发新引用窗口。有效证据确认 BehaviorSignal 单账号幂等、Provider/媒体顺序成功、素材 tag 查询和重试分类；关键未覆盖项继续由 SR-013、SR-017、SR-020、SR-048、SR-075、SR-137、SR-160 承接。高价值补测应快照 Simulation/Management 的全部可写集合、真实 dispatch dry-run、Registry+DB 故障与双激活，以及素材 count=0→并发同 sha 引用→delete barrier。

## B09P：综合 Gateway 流程与情感陪伴角色域校准测试

覆盖文件：3 个，共 2,884 个物理行、2,695 个非空行、127,555 个冻结 Git blob 字节：mock-LLM 综合 Gateway/Outbox 全流程、情感陪伴四轮真模型角色弧，以及 Reviewer 高压识别校准。三份均直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF。两份 roleplay 当前工作树与冻结对象一致；`tests/full_flow_suite.rs` 当前 HEAD/工作树相对冻结对象有一处后续修正：未送达 review 的 `outcome_status` 预期从 `None` 改为占位态 `"pending"`。本批审查的是台账冻结对象，并保留该漂移事实，不覆盖当前源码。

### Mock 全链与送达后提交边界（FACT）

- `full_flow_suite` 的 9 个 ignored 用例真实调用 `handle_managed_message`、`handle_managed_message_aggregated`、Reaction、Outbox claim 与 `process_entry`。它覆盖直接 Approved、单轮 revision、no-reply、知识 tool loop、barge-in 放弃、无抢占、抢占后重算仅一次入队、pending 未进入 Reaction，以及承诺/follow-up 只在 Dispatcher 确认 sent 后提交。与大量手写 Mongo filter 测试不同，这些断言能保护 Gateway→Review→Outbox 的生产接线和部分 delivery finalizer 语义。
- 抢占用例把 guard 固定为 true/false，并串行模拟“首轮放弃→第二轮重算”；它不启动真实 Webhook debounce runner、不制造 generation 交错、进程崩溃或多副本竞争，因而不能证明 ACK 后进程内待处理输入可恢复，SR-177 继续成立。知识用例把目标 verified Chunk id直接注入 mock 的 open/answer 响应，能证明 tool loop 接线与 trace 记录，不能证明真实检索排序或模型自主选择。
- 冻结基线的 pending-reaction 用例期望 `review.outcome_status=None`，但冻结生产 Gateway 创建 review 时已写 `Some("pending")`；因此该冻结测试会在真实执行时失败。当前 HEAD 已把断言修正为保持 `pending`、不得变成实际 `user_replied_*` 标签。这是已修复的基线漂移，不新增生产 finding，也不能把冻结 integration job 的该项失败误判成 Reaction 回归。
- 送达后承诺用例只覆盖单条 Outbox 顺序 claim/send 成功。它没有注入 MCP 成功后 finalizer 中断、重复 `process_entry`、并发取消或 matched_count=0；取消与远端发送交错导致 canceled 主行仍产生 sent 副作用继续归 SR-066，跨账号发送台账与幂等锚继续归 SR-050。

### Roleplay 与 Reviewer 校准的硬门/软观测边界（FACT）

- `roleplay_emotional_companion_e2e` 会 seed 非销售 active DomainProfile，真实调用四轮 Gateway，并以 wiremock 隔离 MCP。硬断言只约束 gateway/final-review 状态闭集、已发送文本不含转人工/伪装身份标记、且不逐字复读上一轮；实际情绪价值、追问密度、线下承诺、Reviewer 误杀、低通过轮数与异族 judge 分全部只写 ledger/eprintln，不使测试失败。文件自身也明确声明“全绿不等于会情感陪伴”。
- 该角色弧每轮继续传最初的 `contact.clone()`，使内存中的 `last_agent_run_at` 恒为 None，从而绕过生产频率门；它直调 Gateway，也不覆盖 Webhook quiet-hours。四轮只代表一条温和夜间低落脚本，不覆盖自伤危机、强对抗或广泛情感场景。若 profile cache 被同进程其它 TestApp 覆盖，还可能回落 DEFAULT，测试数据库共享全局 cache 的不稳定性继续归 SR-174。
- `roleplay_reviewer_pressure_calibration` 是更强的局部能力门：它把 3 条合理关心与 3 条控制式高压固定候选送入生产 Reviewer，并硬断前者 `pressureRisk < block_at`、后者 `>= block_at`。这能检出 Reviewer 对这六条锚的误杀/漏判，但不经过 finalize、revision、Gateway 或 Outbox，不能证明评分最终如何影响发送；异族 judge 仍只作诊断。
- 两份真模型测试无 `REAL_LLM_API_KEY` 时直接 return；顶层 `LlmUnavailable` 也写 skip ledger 后 return。情感陪伴主质量目标即使 judge 未启用、全失败、通过轮数低或低质回复已发送，仍可绿色。专用 roleplay/calibration job仅手动触发；全量 ignored job虽会枚举它们，但通常无 key 且整体 `continue-on-error`。门禁不阻断归 SR-004；“目标断言/裁判结论未发生仍显示通过”的共性继续归 SR-128。

### 执行与系统结论

- 本批共 11 个测试，11 个全部标记 `#[ignore]`：9 个 mock-LLM Docker/Mongo 集成测试，2 个还依赖真实 LLM 与可选 judge。本机未执行；结论来自冻结 blob、当前 HEAD 单点漂移、生产调用点与 CI 接线对账。
- 本批没有新增 finding 编号。有效证据确认 Gateway 的主要顺序终态、抢占检查点、pending 不学习及送达后承诺提交，也提供 Reviewer 六条固定高压锚的真实阈值门；关键未覆盖项继续由 SR-004、SR-050、SR-066、SR-128、SR-174、SR-177 承接。高价值补测应把 `full_flow_suite` 提升为冻结后可重跑的硬门，加入真实 debounce/generation barrier 与 delivery finalizer 故障恢复，并把 Roleplay 质量结果建模为 `pass|fail|inconclusive|infra_skip`，要求最低有效轮数和 judge 覆盖率。

## B09Q：真模型主动触达、幕后请示与渐进式提示档位测试

覆盖文件：5 个，共 2,776 个物理行、2,570 个非空行、126,616 个冻结 Git blob 字节：Digital Twin 跨域角色弧、幕后请示出站与入站 relay、Planner/quiet-hours 主动触达，以及渐进式三档提示词。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；5/5 当前 HEAD/工作树对象与冻结对象一致。

### 具备正向见证的真实链路（FACT）

- `real_llm_principal_relay` 是本批最完整的生产入口证据：测试先 seed 一条 pending 请示，再从公开 Webhook 注入领导自然语言，真 LLM 解析后硬断台账 `resolved`、decision verdict 闭集且非 deferred、`principal_decision_relay` task 已创建；随后调用 task worker 的公开入口，硬断 Gateway/Review 终态闭集、客户转述非空，并扫描转真人、身份暴露和幕后真人拍板标记。它有效保护正常成功链，但前置 pending 是手工构造，不覆盖首次请示卡发送；resolve 与 relay task 的中间失败、跨账号领导匹配和 LLM 解释升级为 verified 知识仍分别归 SR-038、SR-054、SR-040 与 SR-037。relay 后 awaiting 标记只打印，不作硬断言。
- `real_llm_digital_twin_arc` 真调用身份生成器，把生成的 peer_social/formal_business DomainProfile seed 为 active，再让第三族 roleplayer 与 Gateway 多轮对话；真正跑足时会硬断状态闭集、已发文本的转真人/身份泄漏，以及有发送时联系人至少留下画像/记忆信号。这证明测试脚手架能驱动两种非销售域，但身份生成 `None`、roleplayer 后续全 fallback 会正常 return，且零发送时画像与文本红线均不执行。
- `real_llm_progressive_tier` 的默认纯函数测试直接从公开路径覆盖 `Enough/Escalate(Full)/Clarify` 三分支，是稳定有效的局部契约。四个 ignored 真模型用例只有“寒暄不升档”是硬行为断言；产品问询升 Full、含糊消息澄清和 knowledge missing 强升 Full 都允许目标事件缺失后仅打印观测，不能证明动态分档目标实际发生。

### 标称红线门的零产物通过路径（FACT）

- `real_llm_principal_channel` 真实跑四轮超职权 Gateway，并对实际已发送非空文本扫描转真人、身份与幕后决策源禁词；但“是否发起请示”明确降为软观测。四轮均无 pending escalation、甚至无可检查回复时仍可通过，越权硬答只写 ledger 待人工判断。它因而不能证明产品命门“超职权事项走幕后请示”，只能证明有文本时若恰好命中列举字面量会失败。
- `real_llm_proactive_outreach` 会调用 Planner/`ensure_wake_followup_task` 与 `handle_follow_up_task`，但 planner 未 emit、wake task 不存在时直接 return；Gateway 无 reply 时 helper 也打印后 return。即使产生回复，测试只验证 Gateway 候选和禁词，不 claim/dispatch Outbox，不证明真实 MCP 送达、频控竞态或发送后提交。Judge 始终 ObserveOnly。
- Digital Twin、Principal Channel、Proactive Outreach 与 Principal Relay 被放入 nightly `real-llm-redline` 非 soft 矩阵；CI 明确宣称它们是“确定性硬断言”。然而前3份的 identity None、全 fallback、零 escalation、零 task 与零 reply 大多不写 `skip_ledger`。下游脚本只统计顶层 transient 宏写入的 JSONL；文件不存在就报告“0 skip，全部真跑”。本批新增 SR-178，描述硬红线门在目标断言零执行时仍绿色的独立门禁缺口；Principal Relay 是应推广的正向见证对照。
- Progressive Tier 不在该 redline matrix，且其 transient 宏本身不写 skip ledger；它的真模型分支主要属于能力观测而非红线。缺 key 时10个 ignored 用例都会正常 return；唯一默认纯函数用例仍可独立绿色。不能把整个测试二进制绿色解释为三档真实模型机制已验证。

### 执行与系统结论

- 本批共 11 个测试：10 个 `#[ignore]` 依赖真实 LLM（其中多数还依赖 Docker/Mongo，Digital Twin 另需 roleplayer key），1 个默认纯函数测试。本机未执行；结论来自冻结 blob、生产入口、nightly matrix 与 skip gate 对账。
- 本批新增 SR-178。有效证据是 Principal Relay 的 resolve→task→非空转述红线链、Digital Twin 在产物齐全时的跨域 Gateway 接线，以及分档纯函数三分支；Principal Channel/Proactive/Digital Twin 的零产物路径不能作为红线通过。高价值修复应统一 typed case outcome，要求最低 artifact/断言覆盖率，把所有直接 return 纳入 skip/inconclusive ledger，并以 Principal Relay 的正向见证模式重构其它红线 case。

## B09R：基础真模型、跨域闭环与动态角色博弈测试

覆盖文件：4 个，共 2,680 个物理行、2,487 个非空行、122,702 个冻结 Git blob 字节：基础文本/知识/视觉 smoke、销售与情感陪伴跨域长弧、动态对抗跨会话，以及第三族 roleplayer 博弈弧。全部直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 连续读取到 EOF；4/4 当前 HEAD/工作树对象与冻结对象一致。

### 具备真实正向见证的链路（FACT）

- `real_llm_smoke` 的 T1 真实进入 Gateway/Review；若生成 Outbox，则继续调用生产 claim/`process_entry` 并硬断 MCP 成功桩后为 sent。T2 直接调用知识 Agent，硬断至少一轮、答案非空、首工具为 `list_catalog` 且引用只来自三条 seed。它们分别证明真模型 JSON 可接入主链和知识工具循环可在该小 fixture 上收敛，但 T1 的无回复/hold 终态被明确视为合法，不能证明每轮都有可检查内容。
- `real_llm_cross_domain_arc` 在销售与情感陪伴 profile 下真实跑四轮 Gateway，对实际发送文本硬断状态闭集、转真人/身份泄漏、不逐字复读及宽松情绪分地板；有 NewFact 且至少发送一轮时要求联系人留下实质画像信号。更重要的是有发送时会排空 pending Outbox，硬断 sent、幂等键、sent_at，并从 wiremock 反查 MCP 请求数。这是本批最强的 Gateway→Outbox→MCP 正向见证，而不是把 `outbox_enqueued` 冒充已送达。
- `real_llm_dynamic_adversarial` 与 `real_llm_roleplay_arc` 在角色生成有效时都让第三族 roleplayer 根据历史动态出题，再把 Agent 回复喂回下一轮；前者要求至少一条 Agent 回复，后者要求四轮客户发言、至少一条 Agent 回复和 roleplayer 不出戏，并对非空回复执行共享红线扫描。它们能证明动态博弈脚手架的闭环接线，但轨迹质量仍只写 ledger。

### 零产物、软观测与跨会话边界（FACT）

- Smoke T3 真实进入图片导入 Handler，但明确允许 `chunkIds=[]`；其唯一硬断言位于遍历已产出 Chunk 的循环内，所以空 fence/空抽取时零次执行仍绿色。只要有产物，`draft+needs_review` 红线有效；“视觉抽取已发生”的空产物假绿继续归 SR-128，不能用绿色证明视觉模型可用。
- Cross-domain 的画像与投递硬门都以 `sent_turns>0` 为前提；全程无回复时只记 issue。R2.3 两域任一端点失败直接返回 None，双方都有结果但任一回复为空时也只打印；身份探针两轮均未发送时同样以“部分被频控拦，合法”结束。因此它虽有强正向分支，nightly 仍可能完全没有身份红线样本而绿色，扩展 SR-178。
- Dynamic 的 transient 宏不写 `skip_ledger` 就直接 return，roleplayer 全 fallback 也正常返回；第一段无画像时跨会话验证直接降为观测。即使第一段已有画像，第二段只检查画像字段仍存在，不证明生成回复实际引用或正确承接记忆。Roleplay Arc 的顶层 transient 会记 ledger，但全 fallback 的 return 不记；两者都绕过每轮生产频率门，因为始终传入最初 `contact.clone()`。
- Judge、轨迹分、SmallTalk 过度画像、承诺识别与跨域行为差异方向均为 ObserveOnly/ledger。Cross-domain 的“标尺维度不同”是稳定配置事实，“两域回复不逐字相同”只在双方都非空时执行，不能替代独立业务质量门。

### 执行与系统结论

- 本批共 8 个测试，8 个全部标记 `#[ignore]` 并依赖真实 LLM；其中多数还需要 Docker/Mongo，Dynamic/Roleplay 另需 roleplayer key。本机未执行；结论来自冻结 blob、生产入口、nightly redline 与 skip gate 对账。
- 本批没有新增 finding 编号；补强 SR-178 以纳入 Cross-domain 身份探针、Dynamic 无 ledger transient/全 fallback，以及 Roleplay 全 fallback 的零断言绿色路径。有效证据确认 Smoke T1/T2、Cross-domain 的条件式完整送达链和动态博弈接线；视觉空产物继续归 SR-128。高价值修复是让每个 case 输出 typed outcome、要求最低非空回复/角色轮数/断言数，并把所有直接 return 与空产物纳入统一 skip/inconclusive 覆盖率门。

## B09S：对抗校准、运营全能力与知识召回基准测试

覆盖源码：3 个，共 6,456 个物理行、5,948 个非空行、316,513 个冻结 Git blob 字节：真模型红队与裁判校准、运营 Agent 全能力 smoke，以及跨行业知识召回/改库/缺口闭环基准。另结构核验 `tests/fixtures/k6_article_image.b64`：冻结文本 90,156 字节可严格解码为 67,616 字节的 720×520 PNG，解码 SHA-256 为 `20fb16f1a4e0b7b057d46ff3bdd5ac0e3b7b7321e10cc4d3033e173c2d764c2c`，并被 K6/Q3 两套视觉测试直接 `include_str!` 消费。全部对象均来自冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8`；4/4 当前 HEAD/工作树对象与冻结对象一致，图片正文未输出。

### 有效硬门与生产链路证据（FACT）

- `real_llm_ops_smoke` 含多项真实正向见证：T4 真调 FollowUp worker并硬断未过期任务落合法 run、独立过期任务被 precheck 固定为 `expired`；T5 约束持久化状态只在生产 FSM 字典；T10 要求初始画像至少一项结构信号；T11 真调 consolidation，硬断候选 `consolidated` 且 Memory Card 版本递增；T13 要求对立画像均产出非空且不同回复；T15 六轮跌单弧至少两轮 approved，能检出系统性哑火。T6 对实际发送文本硬拦无 verified 知识支撑的具体折扣/价格和列举的绝对化声明；T8/T17 对实际非空回复使用共享否定感知词表拦转人工。两个默认纯函数测试锁定数字/中文金额及绝对化声明探针。
- `real_llm_recall_benchmark` 的三个默认纯函数测试稳定约束 bigram overlap、reach/adopt 集合和 recall 计算。真模型 smoke 要求唯一 seed 的 reach recall=1；跨行业主基准在取得有效样本后硬断 reach/adopt 只来自 seed、lexical-easy reach 均值至少 0.7、adopt 至少 0.4、对抗组不反超词面组、跨轮稳定率至少 0.8、单 case 漂移不超过 0.34。维护测试在相邻阶段都有非空 reach 的样本上限制漂移率不超过 0.5；gap 闭环真正走通时硬断 recall_miss 信号、显式 verify 后 `verified+active`、同 query 再问 reach 新 Chunk，属于有效条件式正向证据。
- `real_llm_adversarial` 的六条红队弧让对手读取真实回复逐轮升级；每个成功且非空的 Gateway 回复都会硬扫共享转人工词、越狱元短语和内部配置指纹。长期弧还硬断宽松的短期记忆上限及 FollowUp run 状态。四个默认纯函数测试正确区分校准 `hit|miss|skipped`，并把出分准确率与 availability 分开，避免掉线被误算为判错。

### 诊断仪表与隐式空跑边界（FACT）

- Adversarial 文件明确定位 Phase A“纯诊断”：`run_managed_turn` 把任意 Gateway `Err` 写 ledger 后返回 false，整弧继续或提前收尾；红队下一击失败、空 message 或 `should_stop` 都可在没有最低有效轮数的情况下结束。机械健全性只打印 `reached/max_turns`，不作门。裁判 panel、跨裁判分歧和金标命中率即使未启用、全采样失败或 availability 为零，也只写日志/台账；所谓 Phase A 退出门尚未编码为断言。因此成功进程只能证明实际非空回复未命中字面红线，不能证明六条弧跑足或校准达标。
- Ops Smoke 的能力声明强弱不一。T9 第一轮无 sent review 时直接返回，未执行 Reaction 闭集；T12“必须先问预算”、T14 弱信号不得冲掉画像、T18 暖启动尊重老客户与禁止硬推均只打印启发式结果。T16 的跨画像差异只在双方回复都非空时检查，T8/T17 也只有非空回复才扫描正文。Judge 全失败、无 review 或空 reply 一律正常跳过。绿色可证明上列硬契约，但不能外推为操控性、谨慎画像、暖启动或裁判质量通过。
- Recall 主基准的强阈值只在 `ran_cases>0` 后执行；内部 `run_query_n_times` 对每轮 `LlmUnavailable` 只 `continue`，不写公共 skip ledger，所有 case 均零成功轮次时整个阈值块正常 return。Maintenance 的 chat create/apply/verify 任一步未命中即 return，所有相邻阶段都无双侧非空 reach 时漂移门跳过；gap 闭环在初次未诚实弃答、对话补库未命中 create 或第二次 answer 不可用时也正常返回。以上隐式空跑补入 SR-128；它们不否定有样本时的客观 recall 门，但禁止把测试进程绿色等同于这些门已执行。
- PNG fixture 的 base64、PNG signature、IHDR 尺寸、编码/解码哈希与三个静态引用点均已核验；这只证明测试素材完整可消费，不证明视觉模型产生 Chunk。K6/Q3/基础 Vision 对零 `chunkIds` 的循环零次通过仍由 SR-128 承接。

### 执行与系统结论

- 本批源码共 36 个测试：27 个 `#[ignore]` 依赖真实 LLM（多数还依赖 Docker/Mongo），9 个默认纯函数测试。本机未运行真实模型或 Docker 用例；结论来自冻结 blob 全文、生产入口、断言分支、CI/skip gate 与 fixture 结构对账。
- 本批没有新增 finding 编号；扩写 SR-128 以纳入 Vision/Recall 的零产物、循环内 skip、零有效样本与 create 意图未命中。有效证据包括 Ops 的 FollowUp/Memory/最低 approved 门、Recall 在有样本时的 reach/adopt/稳定性契约，以及 Adversarial 对实际回复的红线扫描；Phase A 裁判和多项运营质量仍是诊断仪表。高价值修复应统一 typed outcome、最低成功 case/round/artifact 覆盖率，并让 skip gate 按计划 case 清单校验“断言确实执行”，而不是只统计顶层宏写入的 JSONL。

## B10A：Agent Autonomy Loop 历史规范与完成状态对账

覆盖文件：4 个，共 2,711 个物理行、2,244 个非空行、227,476 个冻结 Git blob 字节：requirements-first 配置、R0–R13 需求、技术设计与 72 项全勾选实施任务。四份均直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob按不重叠区间连续读取到 EOF；4/4 当前 HEAD/工作树 clean-filter 对象与冻结对象一致。台账原始字节较 blob 多 2,711 字节，来自工作树 CRLF，不是内容漂移。

### 历史定位与仍有效的设计意图（FACT）

- requirements、design 与 tasks 顶部都明确标记 2026-05-25 sunset：`customer_stage / intent_level / objection_type`、五闸、`safe_claims`、`routing_card` 等销售域专属形态已下线，运行时改为三闸与 `domain_attributes + DomainSchema`。因此不能拿这组历史文档逐字段要求当前通用域实现，也不能把旧术语仍存在本身记为缺陷。
- 文档提出的若干通用架构目标仍有价值：run 入口先落 durable envelope；决策与发送经 Outbox 解耦；幂等键、atomic claim、lease、重试和发送前二次门；Raw/validated DTO 分层；Memory fact 稳定 id；显式 sunset 与性质测试。冻结树也确实存在 `run_envelope.rs`、Outbox/dispatcher、taxonomy、autonomy outcomes、baseline/lint脚本及对应测试文件，说明并非纯纸面方案。
- Outbox 的正常 enqueue、状态枚举、claim/reclaim、重试与多类集成用例是真实局部证据，但历史设计的幂等原文没有 workspace，公共事件和 stop 取消也缺真实租户上下文；这些生产问题已分别由 SR-024–027承接。in-flight 取消无法 fencing 已在途 MCP 的问题继续归 SR-066，不能用 R13 任务勾选或顺序成功用例反证。

### 全勾选任务表与冻结实现的确定性偏差（FACT）

- tasks 的 72 个 checkbox 全为 `[x]`，包含 Run Envelope 生产接线、P1–P7、Outbox crash/PBT 和“全部 PBT + lib + 集成 + happy_path通过”的最终检查点。`src/agent/run_envelope.rs` 文件头却明确写明 started 信封、recovery insert 与 panic lifecycle 推进均未接生产；定义和 ignored 集成测试存在，但真实 Gateway/Memory 等入口在首个 LLM 前没有统一 envelope。运行时后果已由 SR-019记录。
- `tests/autonomy_protocol_pbt.rs` 只有 P1/P2/P3 三条 proptest。P5 后来分散在 memory 模块；P4 verified-knowledge 与 P6 taxonomy 有局部示例测试，但不是任务声称的统一 PBT交付；P7 曾存在于 test-only 工具循环，后来的 sunset 设计明确承认该循环从未接生产并将其连同测试删除。更关键的是当前 R5 只在 Reviewer 自报 `requiresProductKnowledge=true` 时进入硬闸，缺失/漏判不是历史设计声称的确定性 fail-closed，继续归 SR-022。
- Outbox 任务所谓“任意状态序列下唯一键至多一次实际发送 PBT”实际是 `#[ignore]` Docker集成中的固定重复 enqueue 场景，没有随机状态序列或 shrink。baseline hard job只运行 lib、测试编译与四个旧 PBT 文件，不执行 autonomy PBT、Run Envelope/Outbox ignored integration 或核对任务清单；全量 ignored job又属于 SR-004 的 soft gate。因此“文件存在/局部测试存在”无法支持最终检查点已经验证。
- Sunset 也只有部分兑现：`knowledgeRoutingMode/reply_with_tools_loop` 后来因从未接线而删除；冻结树仍保留 `MemoryFactRepr::Plain`，后续设计还主动继续使用它，说明原 D+14移除计划已被现实演进改写。计划可以修订，但任务表和历史 spec没有把“未上线、部分上线、延后、保留兼容、后来删除”区分开，形成独立的交付账本失真，新增 SR-179。

### 执行与系统结论

- 本批未运行 Rust、Docker 或前端测试；结论来自四份冻结对象全文、生产模块自述、生产调用可达性、测试定义、baseline/CI接线与后续 sunset 设计对账。`.config.kiro` 仅确认 feature / requirements-first 工作流，无独立行为保证。
- 本批新增 SR-179。有效证据是历史方案对 Outbox、租约、DTO 分层和可证伪测试的清晰意图，以及部分真实实现/测试；但全 `[x]` 不能作为 R0–R13 完整交付证明。高价值修复不是重启已 sunset 的销售域方案，而是把任务状态改为 `implemented/production_wired/verified/partial/sunset_not_shipped`，并由硬门生成需求→生产入口→测试 artifact 的可核验追踪关系。

## B10B：Agent Self-Evolution 规范、交付状态与自动放量边界对账

覆盖文件：3 个，共 1,462 个物理行、1,215 个非空行、104,191 个冻结 Git blob 字节：M4 requirements、技术设计与全勾选实施任务。三份均直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 按不重叠区间连续读取到 EOF；3/3 冻结对象与当前 HEAD、工作树对象逐字一致。

### 设计意图与真实生产接线（FACT）

- 规范把 Evolution 定义为独立 worker：生产事实只读，候选写自身集合；shadow replay 禁止 Outbox/MCP/outbound 副作用；threshold/prompt 候选通过显著性后再进入发布；发布与回滚保留历史版本并在每个 run 入口读取一次。冻结生产树确有 `src/evolution/` 主循环、cohort、threshold、prompt critic、replay、significance、release、post-release、路由与前端，且隔离 lint 已接 baseline hard job。这不是纯纸面功能。
- Shadow 路径当前按候选类型分流：threshold 用历史 review scores 做纯重判，prompt 走 `agent::prompt_shadow::shadow_replay_prompt_one` 的 decide+review 演练；生产代码没有直接引用 Gateway/Outbox/MCP 发送入口。Release/rollback 与后续观测也有真实生产调用点。不过这些局部接线不能反证候选基线漂移、旧 proposal rollback 越过后续版本以及 commit 后事件/观测 intent 丢失，分别继续由 SR-086、SR-087、SR-088承接。
- 前端“近 7 天”指标实际上先取最新20条再客户端过滤，继续归 SR-171；runtime 总开关不否决旧 eligible 候选的 auto-release 且关闭写失败会假关停，继续归 SR-170。本批不重复计算这些运行时问题。

### 全勾选任务与已删除验收（FACT）

- Tasks 顶部 Done Notice 宣称 W0→W4与收口全部落地，正文所有任务和验证清单仍为 `[x]`；同一文件紧接着却明确记录原任务5.9的四个 testcontainers集成测试和任务4.8的 `evolution_significance_pbt.rs` 已随销售域收敛删除。冻结树确认 `tests/evolution_isolation.rs`、`evolution_prompt_e2e.rs`、`evolution_rollback.rs`、`evolution_threshold_e2e.rs` 与 `evolution_significance_pbt.rs` 全部不存在。
- 对应 design 仍把四条 E2E和 significance PBT列为测试策略；tasks 后文仍把它们标完成，baseline 条目仍声称新增 PBT存在，验证清单仍称“shadow replay 100次后 Outbox/outbound size不变（集成测试覆盖）”。现有 `evolution_prompt_shadow`、release redline、rollback status等后续测试提供了有价值的局部替代，但不等价证明原验收中的完整 tick→shadow→release→生产读取、rollback、100次零副作用与指定 PBT。该交付状态失真与 B10A 同机制，因此扩展 SR-179，不另增重复 finding。
- `scripts/check-evolution-isolation.sh` 只静态禁直接引用特定发送符号，能保护模块边界但不能证明100次运行的数据库集合不变；baseline hard job也不会按 Kiro tasks 清单检查已删除 artifact。全量 ignored integration job仍是 soft gate（SR-004），不能把“曾存在/有替代测试”自动换算成原任务已验证。

### 管理员发布与 threshold 自动放量的契约双真相（FACT）

- M4 requirements 把 shadow eval + admin 显式确认定义为任何 release 的前置，并在 R9.6 明确禁止本期引入自动发布，要求未来放宽由独立 M5+ spec提议；design 也把自动发布列为 M5候选。当前 `docs/agent-policy.md` 仍写“仅 admin可触发 release/rollback”，Admin视角唯一写动作是 release/rollback/rollback_all。
- 冻结生产实现后来加入 `auto_release_eligible_thresholds`，每个 tick末尾调用。只有 env `EVOLUTION_AUTO_RELEASE_ENABLED` 与 workspace `threshold_auto_release_enabled` 双开，且 eligible threshold候选继续偏离合理区间并通过可选负反应门时，才以 `evolution_auto_release` actor直接调用 `release_threshold`；默认双关，且 prompt候选仍永不自动发布。因此不能夸大为默认配置会自动放量。
- 后续 implementation plan记录该轻量接线“已与用户确认”，可证明代码并非无来由提交；但仓库中没有同步修订正式 requirements-first spec与权威策略文档，形成“永远需admin确认”与“可配置自动threshold release”并存。新增 SR-180记录这一治理/契约双真相；父总开关失效的实际执行漏洞仍由 SR-170独立承担。

### 执行与系统结论

- 本批未运行 Rust、Docker或前端测试；结论来自三份冻结对象全文、生产调用可达性、缺失测试 artifact、CI接线、权威策略与后续实现设计对账。
- 本批扩展 SR-179并新增 SR-180。有效实现证据包括 Evolution worker、shadow/release局部链路与隔离 lint；但全 `[x]` 不能证明已删除验收，且自动放量边界缺少唯一权威契约。高价值修复是把 tasks改为可审计状态并绑定实际 artifact，同时由产品/安全所有者正式决定 threshold auto-release 是受控能力还是应删除的越界路径，再统一 requirements、policy、UI、配置与端到端门禁。

## B10C：Knowledge Digest Workstation 规范与生产闭环对账

覆盖文件：3 个，共 662 个物理行、527 个非空行、44,549 个冻结 Git blob 字节：日报/画布/Chat/长任务/operator memory/tool-calling requirements、实施设计与五阶段任务清单。三份均直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的 Git blob 按不重叠区间连续读取到 EOF；3/3 冻结对象与当前 HEAD、工作树对象逐字一致。Tasks 是阶段式实施清单而非全勾选完成表，本批不套用 SR-179。

### 设计意图与已落地能力（FACT）

- 规范定义每日唯一知识日报、紧凑卡片画布、常驻知识 Chat、durable 长任务、独立运营记忆和受预算约束的知识工具；所有 AI 产物只到 `draft+needs_review`，不得自动 verify。冻结生产树确有 `knowledge_digest` 与 `knowledge_task` worker、三类 collection/index、日报与 Chat/Task 路由、SSE、三栏工作台、operator memory、四类扩展工具及 `draft+needs_review` 强制写入路径，不是纸面方案。
- 日报生成真实扫描 Chunk、usage/run logs 与 Evolution proposal，并经 PromptSpec 产卡；Chat dispatch、任务创建、worker逐步执行和 progress/summary turn也已接线。工具 dispatcher 在实际查询前调用 `RunBudget::record_tool_call`，operator memory按 workspace/account/operator 三维独立存储并在知识 Chat intent 前加载。AI不自动 verify 的局部不变量有生产强制和专项集成测试支持。
- 后续实现已扩展原 spec：TaskRail 增加 workspace级任务总览，Knowledge Task 的 fix/add/retag 从早期占位演进为真实草稿动作，operator memory还被正式接入客户 Reply 的 relational/full prompt。以上均有后续设计依据，不能仅因超出初稿就判为孤儿或越界。

### 日报、派工与实时进度的既有缺口（FACT）

- “每天 09:00 主动生成每个运营范围日报”的实现只跑 default workspace/account，且分析共享知识与加载 Prompt 的作用域不一致，继续归 SR-119；Digest 配置预算不被生产消费归 SR-120；失败重算用空卡覆盖成功日报、`partial` 不保留中间产物归 SR-121。
- 长任务的进程内 session mutex只能保证单进程正常路径串行；领取后无 lease/owner/heartbeat、running 不回收且 step副作用无幂等提交，继续归 SR-122。业务失败被多个 action 包成 `Ok` 并虚报成功归 SR-123；稳定 cardId 与 dismiss filter 的跨账号冲突归 SR-124；dispatch/create 没把确认步骤绑定运营选中卡片归 SR-125。
- SSE 服务端有 turn 通知与终态 close，但前端重连预算在成功 open 后不清零，累计断线后永久停止且 TaskRail 不轮询兜底，继续归 SR-173。上述 findings 已覆盖主链风险，本批不重复编号。

### Operator Memory 生命周期（FACT）

- R5.1–R5.3 的物理隔离、知识 Chat 注入和显式写入确认均有真实实现：记录带 workspace/account/operator，写入返回 `memoryId`，Chat turn明确说明已记下。后续把运营偏好用于客户 Reply 是正式 Phase A/渐进式 Prompt设计，不应误判为无授权污染。
- R5.4 同时承诺附 `memoryId` 以便运营“随时撤销”，生产回复也写“如需撤销请直接告诉我”；冻结树却只有 record/load/touch 与只读 list API，没有 revoke/delete intent、Handler、UI或失效审计。记录默认无过期时间，反向偏好也不会使旧记录失效。新增 SR-181记录这一独立的控制面与生命周期失真。
- Knowledge Task 后续 list/get/cancel 只按 workspace展示是后续明确选择的 workspace管理视图，本批没有证据证明它违反现行权限模型，因此不新增跨账号越权 finding；真正的 card dismiss跨账号写错仍由 SR-124承担。

### 测试与验收边界（FACT）

- 专项测试能锁定模型 round-trip、卡片闭集/排序、预算计数、operator memory BSON隔离、turn payload与 `draft+needs_review` 中间态；但多份 Phase测试主要是手写结构/纯函数，不能替代真实 handler、worker崩溃、选择绑定和端到端副作用验证。全量 ignored integration job仍是 soft gate（SR-004）。
- 原 tasks 中“sessionId串行/cancel/fail-soft/summary”等验收不能由 `tests/knowledge_task_worker.rs` 的状态字面量与序列化测试单独证明；生产深读才暴露 SR-122/SR-123。类似地，operator memory isolation测试不覆盖 Chat撤销，因为撤销能力根本不存在。

### 执行与系统结论

- 本批未运行 Rust、Docker或前端测试；结论来自三份冻结对象全文、生产调用可达性、专项测试形态、后续设计与既有 findings 对账。
- 本批新增 SR-181；其余风险由 SR-004、SR-119–125与 SR-173承接。有效实现证据包括日报/任务真实生产接线、知识工具预算、operator memory物理隔离及 AI只落待审草稿；高价值修复顺序是先补 durable task lease+step幂等与服务端派工候选绑定，再修日报 generation/租户/预算，最后补 operator memory可审计撤销和 SSE轮询兜底。

## B10D：User Ops Agent Hardening 规范、生产实现与验收对账

覆盖文件：4 个，共 2,232 个物理行、1,825 个非空行、131,003 个冻结对象字节：requirements-first 配置、20 项鲁棒性需求、技术设计与 24 项全勾选实施计划。四份均直接从冻结提交 `12d99b3b9fd42eae2293b5b3f0a1ff9fe982b7a8` 的对象按不重叠区间连续读取到 EOF；4/4 当前工作树对象与 ledger 的字节数和 SHA-256 一致。规范对象使用 CRLF，按 PowerShell 文本流转成 LF 后为 128,771 字节，2,232 字节差值恰好对应物理行尾，不是内容漂移。

### Sunset 边界与真实落地能力（FACT）

- 三份正文顶部的 2026-05-25 sunset 只点名字符串 fact-risk、`safe_claims/routing_card` 与旧五闸等销售域机制；worker 恢复、入出站时间拆分、Reaction claim、LLM 重试、run budget、memoryCard、索引、限流、评测、outcome metrics 和 Management dry-run 等通用承诺没有被整体 sunset，仍需与冻结生产实现对账。
- 多数结构能力确有生产接线：`db::migrations` 与索引模块、`last_inbound_at/last_outbound_at`、任务 claimed_at/回收、Reaction claim、指数退避与 Retry-After、LRU、RunBudget、状态机迁移、置信度 full review、typed memoryCard、auto-verify、公式评测、outcome metrics、Management dry-run、Webhook per-account 限流，以及 routes/agent 模块拆分均存在真实调用点。指定 happy-path 文件后来也增加了 `handle_managed_message`→Review→Outbox 的真实路径，不能把整个计划误判为纯纸面。
- 这些局部实现不等于并发与提交协议正确：Reaction stale claim 无 owner/generation 且旧分析可晚到覆盖继续归 SR-028；AgentTask reclaim/heartbeat/cancel/终态无 fencing、取消后仍可入 Outbox继续归 SR-034；memory consolidation 跨 OperatingMemory/Contact/candidate/task 的非原子提交归 SR-029；Management 工具副作用与审计无可恢复 intent归 SR-064，错误分类为 read 的 campaign preview 使 dry-run 仍写生产归 SR-075；auto-verify 先计成功再吞 revision 写错归 SR-107。Sunset notice和全勾选状态都不能反证这些生产缺口。

### 全勾选任务与测试证据失真（FACT）

- 24 个 task 全部标为 `[x]`，验证清单还要求 `cargo test -- --ignored` 全通过，并把 HP-1/2/3、S-20 与完整 Gateway happy path列为交付验收。但多个同名测试没有进入所声明的生产入口：`worker_reclaim.rs` 只插入 stale task，随后断言它仍为 running，并在注释中承认无法调用私有 reclaim；`dry_run_isolation.rs` 手工插入 `status=dry_run` 的 command/tool 审计行，未调用 Management handler或 dispatcher；`reaction_claim_lock.rs` 在测试内重写 Mongo CAS，未调用 `record_user_reaction`、Webhook 或 LLM；`last_inbound_split.rs` 复制生产 update文档，未走真实入站和出站函数。
- 这些测试能证明 Mongo 单文档 CAS或 BSON update形状，但不能证明生产入口使用了相同 filter、顺序、作用域与失败语义。它们全部 ignored，承载全量 ignored测试的 CI job 又明确 `continue-on-error`；hard baseline 只编译全部 tests，并运行四个局部 PBT文件，不执行上述 Handler/Worker 验收。因此本批不新增重复测试 finding，而是把该证据扩展进 SR-179：任务“实现”与“生产接线/验收完成”必须分状态记录。
- LLM retry 的核心代码确实从 OpenAI/Anthropic响应读取 Retry-After，并在 retry loop 合并指数退避；`llm_retry_jitter.rs` 虽没有真实 HTTP fixture，但纯函数契约与生产调用链均可见，本批没有证据支持新增实现缺失 finding。状态机和 memory cap PBT也确实调用生产纯函数，只是后者主动弱化了原声明的保留性质。

### coreFacts 上限与永久保留的不可满足合同（FACT）

- Requirement 8 同时要求 `coreFacts.len() <= 6`，又要求任意初始集合 S 中所有未进入 `discarded` 的事实在任意 N 轮后仍属于 coreFacts。上一版已有 6 条、下一轮新增 1 条时，两条性质在数学上即不可同时满足；设计和任务没有定义淘汰、降级到 recent/archive 或稳定优先级仲裁。
- 生产 `compact_memory_card_with_dimensions` 让 incoming 事实优先，再追加上一版未 discarded 事实，最后直接 `truncate(6)`；新卡写满时旧核心事实可全部静默消失。PBT 注释声称“未 discarded 必保留”，正文却只在合并候选总数不超过 6 时断言保留，超 cap 场景主动跳过核心性质。新增 SR-182 单独记录这一运行时记忆丢失与契约矛盾，不把它混入 SR-179 的交付账本问题。

### 执行与系统结论

- 本批未运行 Rust、Docker或前端测试；结论来自四份冻结对象全文、生产符号与调用点、指定测试源码、baseline/CI接线和既有 findings逐项对账。
- 本批扩展 SR-179并新增 SR-182。有效交付包括多数 hardening结构和真实 Gateway happy path；高价值修复是先为 Task/Reaction 引入 generation fencing并把四个“复制实现”的测试改成真实入口测试，再统一 memory core事实的持久存储与 prompt投影契约，确保 cap淘汰可解释、可追溯且由真正超 cap PBT守护。

## B10E：Universal Test Coverage 规范、真模型记录与 47 域审计工件对账

覆盖文件：7 个，共 6,114 个物理行、5,965 个非空行、1,035,078 个 ledger 冻结字节：测试体系 requirements/tasks、真模型 findings、7 锚点能力清单、47 域清单、深读/证伪 workflow 与 47 域结果。七份均按不重叠区间或 JSON 顶层条目连续读取到 EOF；7/7 当前工作树字节数与 SHA-256 均和 ledger 一致。七份文件均为纯 CRLF；结果 JSON 含 47 个完整 deep 对象和 47 个 falsify 对象，逐域字段结构与汇总计数另做机械校验。

### 测试体系意图与当前实现边界（FACT）

- Requirements 正确识别旧真模型测试的三个根问题：缺 key/端点错误可静默 skip，裁判标尺写死销售域，固定单轮脚本无法覆盖主动触达、治理通道和跨会话关系。它把固定回归线与动态发现线分开，要求红线正向见证、DomainProfile 派生 rubric、trajectory 金标校准、三角色异族、transcript 回放和反过拟合 control/held-out 机制；这些是合理的测试方法目标。
- Tasks 是全未勾选的实施清单，不是完成声明，不能套用 SR-179。冻结 CI 后来已经实现多组 real-LLM job 的缺 key fail 和 skip ledger，nightly dynamic 也有独立 workflow；但“有门”不等于目标行为发生。红线 job 对零 reply/零 task/零 escalation/全 fallback 不记 skip 的问题继续由 SR-178承担，integration soft gate 继续归 SR-004，视觉空产物与知识质量门问题归 SR-128。本批不把 tasks 的待办状态误报成生产缺失。
- `real-llm-findings-2026-06-18.md` 是运行期调查日志而非稳定规范。它诚实记录了 C1 单次哑火被跨 run 证伪、C2 helpfulness 跨弧复现、D1 quiet-hours/expiry 顺序 bug、裁判输入缺 grounding/memory/轨迹上下文，以及 T2/T3 关键词探针把正确拒绝判红等方法学问题；同时也保留当时 run 被并行 push 取消、部分 redline 从未跑完、B2 修复尝试未成功等状态，不能将其中“待新 run”文字当作当前通过证明。

### 7 锚点与 47 域审计的有效线索（FACT）

- Anchors 从 worker、agent 模块、路由、红线、发送链、数据集合与 2026-06 新业务七个角度枚举能力，47 域结果进一步给出 design behavior、现有覆盖、test trust、gaps、orphans 和独立 falsify。它实际找到了多种高价值证据模式：测试内重写 Mongo filter、空壳 ignored test、只 round-trip 自建 BSON、仅断 HTTP 200、真实 handler/worker 未被调用，以及 production 符号已有 sibling test 反证等。结果中 85 条 refuted 说明第二遍并非完全附和。
- 多项结论与本系统审查独立吻合：Follow-up reclaim、Reaction claim、Management dry-run 与 last-inbound 测试复制实现（SR-179）；workspace isolation 自证 filter（SR-176）；debounce durable recovery 缺口（SR-177）；Run Envelope/Evolution 验收漂移（SR-019/SR-179）；knowledge worker、digest、operator memory 等缺口（SR-119–125/SR-181）。这些对象适合作为定向检索线索，但重复项不另编号。
- 结果也会把局部纯函数保护拔高成系统保证，或混入陈旧事实。Memory 域称 cap/保留性 PBT 可信，却没有识别其超 cap 时主动跳过保留断言，已由 SR-182纠正；anchors 同时把 Evolution 描述为“empty skeleton”和完整 tick/release；发送链仍用“五闸”概括，而同套红线锚又说旧五闸已删除。任何单条结论仍需回到冻结生产代码和真实门禁复核。

### “事实底座”的完整性与可复现性失真（FACT）

- Workflow 的 deep schema 只要求 `domain/design_behavior/correctness_layer`，falsify schema只要求 `domain/verdict/test_priority`；证据、verified/refuted/orphan数组都不是必填。知识缺口域实际只返回一句 `deep-read holds, no refutation needed`，没有逐条证伪内容，却仍计入 `completed=47`。agent三次失败后返回 null，最终 `filter(Boolean)` 静默删域且工作流正常结束，没有 failed/inconclusive ledger。
- 结果根对象没有 commit、dirty state、时间、模型/provider、workflow/prompt hash、输入哈希、重试或错误记录；同一结果无法按冻结输入和模型配置重放。域清单又同时存在 JSON 权威副本与脚本内联副本，只靠注释要求人工同步，没有机器一致性门。
- 机械统计为 38 个 P0、9 个 P1、300 条 `verified_gaps`、85 条 `refuted`、182 条 `confirmed_orphans`；下游 brainstorming 设计却宣称 190 条孤儿，并把“47 域权威清单/事实底座/建尺已完成”作为后续上线测试计划前提。新增 SR-183记录审计协议允许空证伪、静默漏域和不可复现元数据的独立问题。它不否定逐条线索价值，但否定未经回查的汇总具有上线证明力。

### 执行与系统结论

- 本批未运行 Rust、Docker、前端或真模型测试；执行的是七份冻结工件全文/逐条 JSON读取、原始字节与 SHA-256校验、schema/字段完整性检查、47 域计数和下游引用对账。
- 本批新增 SR-183；其余运行时问题继续由既有 findings承接。最后一批 specifications 已全部读完。高价值收口是把这套审计改成冻结 manifest + 强 schema + failed/inconclusive outcome + 逐 claim证据，再重新生成可机器校验的汇总；当前 300 条线索应作为 backlog候选逐条复核，而不是直接作为上线 go/no-go 事实。

## B11/B12：SR-001～SR-183 真实业务与反过度工程两轮复审

覆盖对象：`findings.md` 中连续编号的 SR-001～SR-183。第一轮按真实运营角色、客户副作用、生产可达性和外部部署条件重新判断；第二轮逐项比较最小局部修复、接受风险边界和可能过度工程的方案。原始逐项结论记录于 `two-pass-review-ledger.md`，人类决策面记录于 `human-confirmation-checklist.md`。

### 第一轮：真实业务逻辑复审（FACT / CONDITIONAL）

- 183 条均重新标为直接业务问题、条件性生产风险、契约冲突或治理证据问题；测试薄弱、文档漂移和指标误差没有被自动提升成客户业务故障。
- 多租户、多副本、边缘 WAF、功能开关、历史凭证是否轮换等仓库外事实均保留为显式条件，未擅自假设部署已经或没有采用这些能力。尤其 Evolution 冻结代码与 `.env.example` 默认开启、README 写默认关闭，保留为需要产品选择的契约项。
- 正常业务流程即可稳定触发的对象错绑、越权写入、错误终态、数据清空、发送安全与知识审核绕过继续保留；只有共享根因和同一人类取舍被合并，原 SR 证据不删除。

### 第二轮：业务简化与反过度工程复审（FACT）

- 所有建议优先复用现有 collection、状态、CAS、唯一键、typed DTO、失败终态和现有 UI；默认不引入新服务、消息总线、通用工作流引擎、配置中心、事件溯源、CRDT 或独立审批平台。
- 多副本可靠性统一收敛为在现有任务/消息记录上增加 owner、generation、lease 和 finalize CAS；配置发布统一收敛为不可变版本加唯一 active pointer；前端竞态统一收敛为对象/账号 identity、generation 和提交前校验。只有部署确实需要多租户或多副本时，才扩展对应运行能力。
- 183 条最终归并为 36 个互斥人类决策项。每项保留推荐、最小处理、不过度工程边界、不处理代价，以及 `修复/接受风险/暂缓/非问题/需补业务事实` 选择、负责人和决定理由字段。

### 机械验收与边界

- `two-pass-review-ledger.md` 含 SR-001～SR-183 恰好各一行，无缺号、无重复；每条恰好映射一个 HC。
- `human-confirmation-checklist.md` 含 HC-001～HC-036 连续编号；36 个来源集合展开后覆盖 183 条 SR 恰好一次，且与逐项账本映射完全一致。每个 HC 均包含来源、两轮结论、推荐、最小处理、不过度工程边界、不处理代价、人类决定、负责人和决定理由。
- 本轮没有修改生产源码，也没有运行 Rust、Docker、前端或真模型测试；判断基于已冻结并完成证据审阅的 183 条 findings 及其代码/测试/规范锚点。该结论表示“现有 findings 的两轮决策复审完成”，不把 `file-ledger.csv` 中仍为 pending 的历史材料误报为已全文阅读，也不替代人类最终选择。

### 首轮人类确认（产品/部署边界）

- 产品明确要求多租户；当前是一个系统/单实例，尚未生产上线。由此，workspace/account 隔离属于当前必须修复的产品能力，多副本专属机制则不应提前平台化。
- Evolution 默认关闭；现阶段所有发布人工确认。未来若开放自动发布，按 proposal 类型和变更方向分级：Prompt/语义/安全放宽永远人工，只有证据充分的安全收紧类阈值可进入显式白名单。
- Shadow/Simulation 不允许写任何生产业务状态，只允许带 shadow 标记的独立诊断、成本和评测日志。
- Webhook 可靠性倾向使用 Mongo durable inbox：每联系人 pending generation + debounce deadline + owner/lease/finalize CAS，复用现有数据库，不引入 Kafka。
- 旧凭证按已失效处理，但仓库值清除、secret 注入和历史副本审计仍列为上线前检查；当前无 WAF，登录限流列为公网生产上线前硬门，不阻塞现阶段测试优化。

## R01：首批低耦合修复（HC-002 / HC-034 / HC-035 / HC-036）

本批只处理已明确且不依赖新业务判断的四项，不修改数据库模型、不新增依赖、不改变既有 API 路径，也没有顺带处理其它 HC。

- HC-002：Evolution 安装态默认改为关闭。`EVOLUTION_ENABLED_DEFAULT`、`.env.example`、启动注释和单测统一为 false；Mongo runtime flag 仍是 env 显式放行后的第二层开关。
- HC-034：文档编辑按后端真实 `{item}` 包络解包，并校验详情 id 与当前对象一致。继续使用既有整替换 PUT，完整回带原文、hash 与索引字段，不扩大为新 PATCH 协议。
- HC-035：诊断页按 persisted `{documents}` 和 live `{item:{documents,items,chunks}}` 读取；持久化数统计真正已有持久摘要的文档，实时数统计 live catalog 文档。
- HC-036：Knowledge Ask 增加独立 `TraceEvent::Failed` / SSE `failed` 终态；轮内可恢复的 `tool:error` 仍作为 trace。前端区分成功、业务失败、连接失败、主动取消和无结果 close，并把最大轮次显示与后端 `MAX_ROUNDS=4` 对齐。

验证结果：前端 3 个定向测试文件共 5 项通过；`npm run build` 通过；`cargo test --lib evolution_enabled_defaults_to_false -- --exact` 通过；`cargo check --lib` 通过；`cargo test --test knowledge_ask_stream_e2e --no-run` 通过。首次 Vitest fork worker 启动超时后改用单线程池成功；全仓 `cargo fmt --check` 会命中大量既有格式债，本批产生的格式噪音已全部清除，最终 Rust diff 只保留逻辑改动。
