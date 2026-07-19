# 审查发现（持续更新）

严重度与确定性分开。这里不记录任何秘密值。

## SR-001：跟踪的测试环境文件包含明文 API 凭证形态值

- 严重度：P0
- 确定性：FACT（是否仍有效为 RUNTIME_UNVERIFIED）
- 证据：`.env.e2e:7-9`，其中 key 行已在审查输出和本文中脱敏。
- 影响：任何能读取当前仓库或历史提交的人都可能取得该值；删除当前文件不能消除 Git 历史泄漏。
- 建议：立即在上游轮换；撤销旧值；用 secret store 注入测试；清理 Git 历史；检查 fork、artifact、日志与缓存。

## SR-002：跟踪的部署脚本包含明文 MCP 凭证形态值

- 严重度：P0
- 确定性：FACT（是否仍有效为 RUNTIME_UNVERIFIED）
- 证据：`deploy.sh:41-50`，具体值不记录。
- 影响：同 SR-001；脚本还会把该值追加到生产 `.env`，扩大复制面。
- 建议：立即轮换；改为部署环境/secret manager 注入；清理历史与服务器副本。

## SR-003：Evolution 默认值文档互相冲突

- 严重度：P2
- 确定性：FACT
- 证据：`README.md:118-123`、`docs/architecture.md:138-141` 对比 `src/config.rs:5-7,617`、`.env.example:146-150`。
- 影响：运维可能误判演化器硬锁默认状态。实际业务运行还受 Mongo runtime flag 影响，待 Evolution 阶段核实。

## SR-004：完整 Docker 集成测试不阻断合并

- 严重度：P1
- 确定性：FACT
- 证据：`.github/workflows/ci.yml:145-195` 的 `continue-on-error: true`。
- 影响：集成失败不会否决 PR；具体高风险未覆盖范围待测试阶段建立映射。

## SR-005：LLM 探针把 API key 定义为普通 workflow input

- 严重度：P1
- 确定性：CONTRACT/RUNTIME_UNVERIFIED
- 证据：`.github/workflows/probe_llm_endpoint.yml:5-26`。
- 影响：普通 `workflow_dispatch` input 不具备 GitHub Secret 的专门遮蔽与权限语义；是否已泄漏需查 Actions 历史和平台元数据。
- 建议：改为 repository/environment secret；删除一次性探针 workflow；审计历史 run。

## SR-006：架构/README 已明显落后于启动代码

- 严重度：P2
- 确定性：FACT
- 证据：`README.md:160` 的 78 基线与 `CLAUDE.md:64-70` 的 350 冲突；`docs/architecture.md:390-410` 的 9 worker 与 `src/main.rs:202-312` 最多 12 类不一致；旧 webhook 图与新 outbox 图并存。
- 影响：维护者可能依据旧运行图做错误修改或部署判断。

## SR-007：m009 可能把最高版本 draft 选为 current prompt

- 严重度：P1
- 确定性：FACT
- 证据：`src/db/migrations/m009_prompt_template_versioned.rs:34-81`。
- 机制：聚合先按 `version` 降序，再用 `$first($cond(status==active, _id, null))`。若最高版本不是 active、较低版本才是 active，`active_id` 仍为 null，随后回退到最高版本 `fallback_id`，与文件注释“优先最高 active”不符。
- 影响：升级旧库时可能激活尚未发布的 draft prompt；混合新旧字段的组还可能与既有 current 并存。
- 建议：先过滤/排序 active 优先级后再 group，或分两次查询；增加“高版本 draft + 低版本 active”的迁移测试，并清点已执行该 marker 的库。

## SR-008：ops 三表的 current-version 索引不是唯一约束

- 严重度：P1
- 确定性：FACT（并发触发需同一逻辑 scope 的 publish/rollout/rollback 交错；单请求中间写失败也可留下异常态）
- 证据：`src/db/indexes.rs:1002-1135` 对 `operation_domain_configs`、`operation_state_policies`、`system_taxonomies` 创建的 `current_version=true` partial index 均未设置 `unique(true)`；模型注释却声明同逻辑键至多一条 current。真实切换路径见 `src/routes/admin_ops_versions.rs:55-139,420-741,748-945`。
- 机制：三表的 publish/rollout/rollback 都用多个独立写维护 current：先 insert/promote 一行，再 demote 同 scope 其它行，没有事务、CAS 或数据库唯一约束。两个 publish 可同时算出相同 next version 而由版本 unique 让其中一个报错；更危险的是不同目标的 promote/demote 交错，双方都可能把对方 demote，留下持久 0-current，或在中间失败时留下多 current。代码只在状态机 publish 注释中承认 0-current 端态，state policy 与 taxonomy 路径具有同构写序列但没有恢复机制。
- 影响：数据库允许并发或异常路径留下零个或多个 current。operation domain 运行时可能回落 DEFAULT，state policy 缺行时保护门 fail-open，taxonomy cache 可能同时加载冲突版本或失去正式值；管理端读取结果又取决于无排序/排序实现，运营看到的版本未必是实际生效版本。
- 建议：先迁移清理异常 current，再以事务或单文档 active pointer/CAS 原子切换；若部署约束不允许事务，应至少串行化同 scope 变更并提供可恢复 intent/reconciliation。建立与真实业务唯一性一致的 partial unique 约束前先做存量预检，读侧遇到零/多 current 应告警或 fail-closed，不能静默任选/回落。增加同 scope 双 publish、交叉 rollout/rollback、每个中间写失败与自动修复测试。

## SR-009：m029 的 roster 身份回填跨 workspace/account 混用同 wxid

- 严重度：P1
- 确定性：FACT
- 证据：`src/db/migrations/m029_cleanup_contact_identity.rs:43-50,53-95`。
- 机制：所有 roster snapshot 被装入 `HashMap<String, ...>`，键只有 `wxid` 且 `or_insert` 保留首条；随后虽用 `(workspace_id, account_id, wxid)` 更新 contact，写入值却来自全局 wxid map。
- 影响：同一 wxid 在多个 workspace/account 下展示身份不同时，先遍历到的昵称/头像可能污染其它租户或账号。
- 建议：映射键改为 `(workspace_id, account_id, wxid)`；对已执行 m029 的多租户库做差异审计，并增加跨账号同 wxid 集成测试。

## SR-010：生产守卫迁移被“跳过后仍记为已执行”

- 严重度：P1
- 确定性：FACT
- 证据：`src/db/migrations/m011_drop_legacy_sales_collections.rs:19-25`、`m012_drop_legacy_taxonomy_seed.rs:18-24`、`m014_drop_trigger_keywords.rs:14-20`、`m016_backfill_workspace_id_on_legacy_rows.rs:95-101`，以及 `src/db/migrations/mod.rs:288-307`。
- 机制：production 分支返回 `Ok(())`；runner 无法区分“执行”与“有意跳过”，仍插入 migration marker。以后切换环境或完成备份后重启也不会自动再跑。
- 影响：尤其 m016 的 workspace 回填若未按运维步骤手工完成，迁移账本仍显示完成，多租户 filter 可让旧行永久不可见；其它三条会留下文档形态/清理状态与账本不一致。
- 建议：把“需要人工确认”建模为明确 pending/blocked 状态或启动前置检查；手工执行后写可核验 marker，并提供审计查询证明目标数据已满足后置条件。

## SR-011：per-account MCP 凭证解析与日志固定到默认 workspace

- 严重度：P0
- 确定性：FACT（跨租户触发需存在非默认 workspace 调用）
- 证据：`src/mcp.rs:335-425,793-815`；调用点只传 account_id，例如 `src/agent/media_send.rs:155-165,227-233`、`src/agent/gateway.rs:3409-3424`、`src/agent/outbox_dispatcher.rs:1463-1558,1585-1848`、`src/routes/accounts.rs:258-276,306-323`、`src/routes/management.rs:255-310,1327-1361,2411-2426`。
- 机制：`credentials_for_account` 用 `state.config.default_workspace_id` 查询账号；非默认 workspace 即使已解析出自己的 contact/account，也无法把 workspace 传入。查不到账号会回落全局 MCP URL/key。`McpCallLog.workspace_id` 同样无条件写默认 workspace。Dispatcher 虽从 OutboxEntry 读取真实 workspace 来加载 contact/account，实际文本、媒体、名片发送和崩溃/超时后的 `chat_search_outbound` 核对仍只传 account_id；本地 post-hoc 日志查询也只按 account/recipient/content，不筛 workspace。Management 路由虽先用当前 workspace 校验 session/account，但工具目录加载、产品包装内的 MCP 查询和所有动态 raw MCP 透传又退化为只传 account_id，因此同样继承这一错凭证/错日志边界。
- 影响：非默认 workspace 可能使用默认租户的账号凭证调用 MCP，导致消息/媒体从错误微信账号发出或访问错误通讯录；审计日志同时归到错误租户。若不同 workspace 复用 account_id，风险直接可触发；即使不复用，查不到后的全局 fallback 仍会发生。崩溃恢复或发送超时时，另一 workspace 同 account/contact/content 的成功日志还可能被误认作本条已送达，使当前 Outbox 直接标 sent 而实际未发送。
- 建议：把 `(workspace_id, account_id)` 作为所有 MCP account 调用、chat search 与 post-hoc 日志查询的强制上下文；移除跨租户 fallback 或只允许显式单租户模式；日志继承同一 workspace；为两个 workspace 同 account/contact/content 的正常发送、timeout 与 reclaim 建端到端隔离测试。

## SR-012：Webhook debounce runner 重载联系人时丢失 workspace

- 严重度：P0
- 确定性：FACT（跨租户触发需复用 account_id/wxid）
- 证据：`src/webhooks.rs:72-74,130-136,273-286,639-660`；联系人唯一索引实际是 `(workspace_id, account_id, wxid)`，见 `src/db/indexes.rs:77-84`。
- 机制：入口 debounce key 含 workspace，但 spawn 的 `run_debounce_pipeline` 参数没有 workspace；`reload_managed_contact` 只按 `{account_id, wxid}` 执行 `find_one`。
- 影响：两个 workspace 存在相同 account_id/wxid 时，Mongo 可返回另一租户 contact，后续 Agent 读取其画像/知识并按其上下文生成或发送，构成跨租户数据与副作用污染；结果还依赖未排序 `find_one`，不确定。
- 建议：runner 强制携带入口已解析的 workspace_id，所有 reload filter 使用三元组；增加同 account/wxid 跨 workspace 的 webhook 集成测试，并审计现有重复三元组投影。

## SR-013：workspace 级 LLM active 配置映射到单个进程级 Registry

- 严重度：P1
- 确定性：FACT
- 证据：`src/main.rs:88-112,417-477`、`src/routes/mod.rs:301-309`、`src/routes/llm_providers.rs:101-128,213-279,325-368`、`src/llm.rs:1384-1471`。
- 机制：DB CRUD 与 active 标记按 workspace 隔离，但进程只有一个 `LlmRegistry`。启动只加载默认 workspace；任何授权 workspace 更新/激活 active provider 都 swap 这一个全局 client，列表里的 runtime active meta 也来自该全局值。
- 影响：租户 A 激活 provider 后，租户 B 及所有 worker 的后续主 LLM 调用会改用 A 的 endpoint/key/model；这既破坏配置隔离，也可能把 B 的 prompt/业务数据发送给 A 配置的第三方端点。
- 建议：Registry 以 workspace 为键并在每次调用强制传 workspace；或明确系统只支持单 workspace 并拒绝其它 workspace 的 provider 管理。激活 DB 更新与 registry swap 还应具备一致的原子/补偿语义。

## SR-014：Session/JWT 信任 workspace 快照，ACL 收缩不会全局即时生效

- 严重度：P1
- 确定性：FACT
- 证据：`src/auth/session.rs:110-147`、`src/auth/middleware.rs:47-89`、`src/auth/jwt.rs:35-115`、`src/routes/shared.rs:1577-1616`；直接使用快照的示例见 `src/routes/accounts.rs:32-67`、`src/routes/media_assets.rs:34-38`。
- 机制：middleware 校验 session 是否存在/过期或 JWT 签名/exp 后，直接注入其中保存的 current_workspace，不重读 AdminUser/ACL。只有显式调用 `resolve_authorized_workspace` 的部分路由会重新校验。登录/token 选择 default_workspace/首 workspace 时也未再次验证该值属于当前 ACL。
- 影响：管理员被移出 workspace 或 AdminUser 被删除后，既有 cookie 最长仍可在直接信任快照的路由使用到 session TTL（默认 7 天）；JWT 最长到 token TTL（默认 60 分钟），且无服务端撤销。脏的 default_workspace 也可在登录时直接进入未授权 workspace。
- 建议：在统一 middleware 中重读 AdminUser 并校验 current_workspace，删除用户时级联删除 session；JWT 加短 TTL+jti/权限版本或在敏感请求重查 ACL；创建 session/token 前验证初始 workspace。

## SR-015：WechatAccount.app_id 未唯一约束但被当作唯一路由键

- 严重度：P1
- 确定性：FACT
- 证据：`src/db/indexes.rs:58-75`、`src/webhooks.rs:318-358,360-380,990-1009`、`src/routes/accounts.rs:98-116,132-168`。
- 机制：`app_id` 只有 sparse 非 unique 索引；Webhook 用 `{app_id}` 的无排序 `find_one` 决定 workspace/account/签名 secret，Online/Offline 也只按 app_id 更新一条记录。账号同步可把上游 appId 写入任意当前 workspace。
- 影响：重复 appId 会使验签 secret、消息归属和在线状态更新落到不确定租户，可能拒收合法 webhook，或把入站消息归入错误 workspace。
- 建议：先检测并消解重复值，再建立 sparse/partial unique app_id 索引；同步/更新时把 duplicate-key 映射为明确冲突；启动 sanity check 拒绝重复 appId。

## SR-016：公开登录与 token 签发端点没有应用级尝试频控

- 严重度：P1
- 确定性：FACT（外围 WAF/反向代理补偿为 RUNTIME_UNVERIFIED）
- 证据：`src/auth/middleware.rs:31-44`、`src/routes/auth.rs:48-88,208-241`、`src/auth/session.rs:77-104`。
- 机制：`/auth/login` 与 `/auth/token` 精确白名单公开；两条路径都直接执行 Argon2 验证，没有 per-IP/per-username 令牌桶、失败退避、账户锁定或审计阈值。不存在用户也有 dummy Argon2，能降低枚举但同时让每次匿名尝试都消耗昂贵哈希成本。
- 影响：公网部署可被在线猜密或低并发 CPU 消耗攻击；`/auth/token` 在 JWT 开启时形成第二个等价入口。
- 建议：在两端点共享限流器和失败审计，组合 IP、用户名与全局预算；生产由边缘网关再设连接/请求限额；避免永久锁定造成反向 DoS。

## SR-017：媒体文件与 Mongo 元数据写入不是原子操作

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/media_assets.rs:150-191,435-490,523-564`、`src/media_storage.rs:73-99`、`tests/media_asset_crud_integration.rs:264-433`。
- 机制：上传先写本地文件再 insert ContentAsset；换文件先写新文件再 update Mongo。后续 DB 写失败没有删除刚写文件。删除/换文件清旧对象时又先提交 Mongo 变更，再查询同 `file_path` 的剩余引用数，为 0 才物理删除；计数与删除间没有锁、事务、对象 generation 或 tombstone。并发上传同内容（路径由 workspace+sha 决定）可在计数为 0 后、新文件删除前插入新引用，随后旧删除者把共享文件删掉。文件位于进程本地 `MEDIA_STORAGE_DIR`，Mongo 记录只保存相对路径。
- 影响：失败重试会积累无引用文件；反向竞态则会留下指向已删除文件的新 ContentAsset，素材列表仍有记录但发送/下载失败。多副本若不共享同一卷，上传副本之外的 dispatcher 本就读不到文件；DB/文件备份也可能形成不一致快照。
- 建议：写临时对象后以补偿清理完成 DB 提交，并把引用建立/释放与对象生命周期建模为可重试协议：使用对象表的引用计数 CAS、generation/tombstone 或对象存储版本，只有确认同 generation 无新引用后才删除；删除后若发现新引用需恢复或重传。提供定期按 DB 引用扫描的垃圾回收与缺失对象修复；生产改共享持久卷或对象存储。增加“count=0 后并发上传同 sha”的 barrier、DB 写失败清理、删除失败重试及多副本读取测试。

## SR-018：Agent LLM 调用日志固定归入默认 workspace

- 严重度：P1
- 确定性：FACT
- 证据：`src/agent/mod.rs:215-310,376-458`；cache hit、success、failure 三条 `LlmCallLog` 构造分别见 `:244,:394,:438`。
- 机制：`generate_agent_json` / streaming 入口只接收 account/contact/run 上下文，没有 workspace 参数；所有日志无条件使用 `state.config.default_workspace_id`。
- 影响：非默认 workspace 的模型、token、错误、prompt key 与联系人/run 元数据会进入默认租户审计视图；租户级成本统计、故障定位和合规导出失真，同 account/contact 标识复用时还会向默认 workspace 的管理员暴露另一租户的调用元数据。日志不保存完整 prompt，因此本项不是“prompt 正文直接泄漏”的结论。
- 建议：所有 Agent LLM 入口强制接收 workspace context，并让 cache/log/usage 继承同一上下文；禁止调用方只传 account_id；增加两个 workspace 同 account/contact id 的日志隔离测试。

## SR-019：Run Envelope 已实现但生产未接线，部分 Agent run 不可追溯

- 严重度：P1
- 确定性：FACT
- 证据：`src/agent/run_envelope.rs:1-38,352-429,554-760`；全仓 `src/` 对 `write_run_envelope_started`、`update_run_envelope_terminal`、`install_panic_hook_for_envelope` 的检索只有定义、注释和模型引用，无生产调用。Memory consolidator 另见 `src/agent/memory.rs:1212-1247,1404-1453,1532-1547`。
- 机制：Gateway 仍在决策产出后一次性写 `AgentRunLog`，没有在首个 LLM 调用前插入 `lifecycle=started`，也没有统一 catch/panic/terminal 更新。Memory consolidator 会生成自己的 run_id 并传入公共 LLM 日志，但既不创建 `AgentRunLog`，也不接 Run Envelope；其 warnings 最后仅 `update_one({run_id})` 且忽略 matched_count，因此正常情况下也可能没有可更新的 run 行。模块内状态机、recovery insert 和 ignored Mongo 集成测试没有接到这些生产控制流。
- 影响：Reply Agent 在决策前 timeout、panic、网络/JSON 失败时，run 表中可能完全没有记录；Memory consolidation 的运行、预算与告警也可能只散落在 LLM 日志或完全丢失 run 级审计。漏斗分母、失败率和事故追踪会系统性遗漏请求，`memory_consolidator_warnings` 的设计字段可能长期为空。运行手册或测试若把 R0 原语当成已上线保障，会得出错误恢复结论。
- 建议：在所有独立 Agent 入口（Gateway、Reaction、Memory consolidation 等）先写 started envelope，再以 catch-unwind/统一 finally 语义推进 terminal；以 workspace/account/source 为必填恢复上下文；warnings 更新必须检查 matched_count 或走 terminal upsert。把关键集成测试提升为合并硬门，并加“首个 LLM 前失败仍留 run”与“consolidator warnings 可在同 run 查询”测试。

## SR-020：LLM 精确缓存不感知 provider/workspace，热切换后可返回旧 provider 产物

- 严重度：P2
- 确定性：FACT
- 证据：`src/agent/mod.rs:203-280,475-505`、`src/llm.rs:1421-1434`；`prompt_pack_version.fetch_add` 调用点不包含 LLM provider 激活路由。
- 机制：进程级 LRU key 只有 prompt key、全局 prompt-pack version 与 system/user 文本 hash；不含 workspace、provider id、format、model、base URL。热替换 Registry 只 swap client/meta，不清 cache、不递增版本。
- 影响：启用或切换 provider 后，四类白名单请求（知识导入预览、playbook 生成/优化、用户引导预览）若输入完全相同，会直接返回旧 provider 生成结果，绕过新 provider 的模型、策略与数据驻留预期；cache-hit 日志还使用静态 `.env` model，进一步误导排障。因为命中要求 system/user 文本完全相同，本项不声称不同 prompt 内容会互相泄漏。
- 建议：cache key 至少加入 workspace + active provider id/version + model/format；provider swap 成功后递增独立 provider generation 或清理对应 namespace；cache-hit 日志记录产物生成时的 provider/model，而非静态配置值。

## SR-021：Reply/Review/Reaction/Memory prompt 与 Soul 固定读取默认 workspace

- 严重度：P1
- 确定性：FACT（跨租户触发需存在非默认 workspace 的 Agent run）
- 证据：`src/agent/decision.rs:628-667,1383-1403`、`src/agent/review/mod.rs:317-346`、`src/agent/reaction.rs:281-345`、`src/agent/memory.rs:1288-1317`；对照同一主链中的 DomainProfile、taxonomy、知识、素材、记忆与联系人数据均使用 `contact.workspace_id`，例如 `src/agent/decision.rs:343-367,696-706`、`src/agent/review/mod.rs:330-346`、`src/agent/reaction.rs:35-38,137-141,288`、`src/agent/memory.rs:1219-1225,1248-1263,1291-1293`。
- 机制：Reply Agent 的 `user.reply.system`、`user.reply.policy`、`user.reply.task` 三层 prompt 和 published user Soul 均用 `state.config.default_workspace_id` 查询；Reviewer 的 light/full system prompt 同样固定默认 workspace。Reaction 与 Memory consolidator 虽已持有 contact，并按其 workspace 加载 domain config、active profile、memory/candidates 与对话，却仍分别从默认 workspace 加载 `user.reaction.*` 和 `user.memory_consolidator.*`。初始画像路径 `build_initial_operation_profile` 则正确使用传入的 `workspace_id`，形成同一 Agent 内部不一致。
- 影响：非默认租户的联系人会在自己的 DomainProfile、记忆、知识和素材上下文上，套用默认租户发布的回复契约、运营策略、任务 schema、Soul、审核规则、Reaction 分类和 Memory 整理指令。结果可能违反本租户政策、错误放行/拦截、把用户反应归入错误语义，或按默认租户的保留/弃用规则重写长期记忆、标签与人格旁路；错误产物又会进入 outcome、intent trajectory、负例学习和后续 prompt。默认 workspace 的自定义 prompt 内容也会被发送进处理其它租户请求的 LLM 上下文。是否会被模型复述给终端用户取决于 prompt 内容与模型行为，本项不把该可能性表述为必然泄漏。
- 建议：所有 prompt/Soul loader 强制接收当前 run 的 workspace；仅在显式、可审计的“继承全局模板”配置下回落默认 workspace；增加两个 workspace 发布不同 reply/review/reaction/memory prompt 与 Soul 的端到端隔离测试，并断言 outcome、长期记忆和学习副作用归属正确。

## SR-022：产品声明硬门信任 Reviewer 自报，漏报时无背书承诺仍可放行

- 严重度：P1
- 确定性：FACT
- 证据：`src/agent/review/gates.rs:660-717,719-786`；确定性回归测试 `:2244-2277,2342-2376` 明确断言 Reviewer 把 `requiresProductKnowledge` 置 false 时，“一定能”或“保证按时回款”且无 verified chunk 的回复仍为 `Approved`，只产生 `status=observe` 事件。`src/agent/guards.rs:326-331,371-429` 显示硬门只读取该布尔，自有词表只做承诺分类。
- 机制：R5.4 只有在 `claim_analysis.requiresProductKnowledge=true` 时才计算 verified chunk 并 fail-closed。字段缺失、类型不符或 Reviewer 语义漏判都经 `doc_bool` 变成 false；后续 commitment marker 探针即使命中效果/数据类承诺，也只记录 telemetry，不改变 `review.approved`、`decision.should_reply` 或终态。
- 影响：Reviewer 对产品能力、价格、案例、效果或交付承诺漏标时，结构化 verified-knowledge 红线完全不执行；无背书的效果承诺可进入发送候选。生产 Gateway 在 finalize 后仅按 `final_review_status ∈ {approved, revision_applied_approved}`、`should_reply=true` 与正文非空建立 outbox 资格，没有独立重判产品声明，因此该漏判可直接进入 outbox。
- 建议：把 claim-analysis 缺失/畸形建模为显式错误；对产品效果、价格和交付类高置信结构化信号采用独立、确定性的 fail-closed 分类器或第二判定源，而不是只信 Reviewer 自报；至少将 `ProductEffect` 漏判从 observe 提升为阻断或强制独立复审，并保留 ToneOnly 的低误杀策略。

## SR-023：revision 失败会把已知软闸失败原稿重新标为可发送

- 严重度：P0
- 确定性：FACT
- 证据：`src/agent/review/gates.rs:120-205,1063-1114`、`src/agent/gateway.rs:2037-2268,2709-2716`。
- 机制：human-like、pressure、emotional-value 与 boundary/privacy 失败被归为软闸并触发 single-shot revision；其中 boundary/privacy 低分明确表示候选可能泄露内部画像、AI 身份或幕后领导信息。revision LLM 报错、30 秒超时，乃至改写稿二审未通过时，Gateway 都恢复改写前原稿，`apply_revision_fallback` 无条件把 review 改成 `approved=true`、`final_review_status=revision_applied_approved`、Gateway 状态改回 Approved。下游不再调用 `review_passed`，而是直接把该终态列入 `outbox_eligible`。
- 影响：系统已经识别为压迫感过高、情绪价值不足或疑似泄露内部画像/AI/幕后领导信息的原稿，在改写服务故障或二审失败时仍会进入 outbox。尤其 boundary/privacy 分支把潜在内部信息泄漏从安全失败降级成“改写失败就照发”，违反 fail-closed 边界。
- 建议：区分纯风格优化与安全/边界软闸；boundary/privacy、压力红线及双 reviewer 安全分歧在 revision 未成功通过二审时必须 hold。若确需对纯 human-like/style 超时回退原稿，使用显式可回退类别白名单，并在恢复前重新执行 `review_passed`/安全分类；`revision_post_review_failed` 不应与 LLM timeout 共用无条件放行 helper。

## SR-024：公共 Agent/Outbox 事件写入器固定把事件归入默认 workspace

- 严重度：P1
- 确定性：FACT（跨租户触发需存在非默认 workspace 的调用）
- 证据：`src/agent/gateway.rs:5536-5564`、`src/agent/outbox.rs:384-416`；两个 helper 的签名都只有 account/contact，没有 workspace。前者被 Gateway、Webhook、任务、Planner、Memory、Knowledge Router、账号调度和多类 Worker/管理路由广泛调用，例如 `src/webhooks.rs:172-258,742`、`src/tasks.rs:87-308`、`src/agent/gateway.rs:235-5399`、`src/supervisor.rs:80-91`；后者覆盖 outbox created/skip/cancel/retry/sent/terminal/cap 等事件，见 `src/agent/outbox.rs:231-343,643-657`、`src/agent/outbox_dispatcher.rs:1205-1459,1685-1955`。事件读侧按当前 workspace 过滤，见 `src/routes/events.rs:28-54`；模型与索引也把 workspace 作为隔离键，见 `src/models.rs:1041-1057`、`src/db/indexes.rs:217-246`。
- 机制：`write_event_for_account` 与复制实现 `write_outbox_event` 都无条件写 `state.config.default_workspace_id`。调用方即使已经持有真实 workspace 或完整 OutboxEntry 也无法传入；Supervisor 捕获任意长驻 worker panic 后也以 account=`system` 调用同一 helper，因而 panic 审计同样只能进入默认 workspace。相邻的直接事件写入路径能正确使用业务对象或 admin 的 workspace，证明默认 workspace 不是集合级全局语义。
- 影响：非默认 workspace 的决策、拦截、revision、知识、画像、任务、Webhook、Outbox 创建/重试/取消/送达与终止事件在本租户事件页不可见，却落入默认租户。若 account/contact 标识复用或默认租户管理员按该 account 查询，事件 summary/details、run_id、outbox_id、风险与联系人标识会跨租户暴露；告警、漏斗和运营统计也会系统性错账。
- 建议：把 workspace_id 设为公共事件写入 API 的必填参数，优先传完整 tenant context；迁移所有调用点并增加编译期阻断旧签名。对历史事件按可验证的 run/contact/account 关联回填，无法唯一归属的隔离审计；增加两 workspace 同 account/contact 标识的读写隔离测试。

## SR-025：账号发送计数与节奏门跨 workspace 合并同名 account_id

- 严重度：P2
- 确定性：FACT（跨租户触发需复用 account_id）
- 证据：`src/agent/gateway.rs:3439-3462,3963-3985`、`src/agent/outbox_dispatcher.rs:1395-1459,1709-1723`、`src/models.rs:2997-3044`、`src/db/indexes.rs:58-67,904-958`。
- 机制：账号业务唯一键是 `(workspace_id, account_id)`，OutboxEntry 也保存 workspace_id；但日发送 helper 只按 `account_id + status=sent + sent_at` 计数。Dispatcher 的 `account_last_sent_at_ms` 同样只按 account_id 取所有 workspace 中最近一次 sent，并据此推迟当前 entry。支撑两者的 sent_at 索引也以 account_id 开头且没有 workspace。
- 影响：两个 workspace 使用同一 account_id 时，任一租户发送都会抬高另一租户的日发送计数、触发虚假软上限告警，并可能让另一租户的消息被 pacing 门不必要地推迟。日上限当前仅告警，节奏门只延迟且不消耗重试，因此本项不等同于丢消息；但防封号观测、发送时序、租户统计与告警归因均会串租户，事件还会叠加 SR-024 被写入默认 workspace。
- 建议：两个 helper 都强制接收 workspace_id，filter 与索引改为 `(workspace_id, account_id, status, sent_at)`；增加跨 workspace 同 account_id 的计数与 pacing 隔离测试。若日阈值未来升级为硬门，修复应作为前置条件。

## SR-026：Outbox 全局唯一幂等键未纳入 workspace

- 严重度：P1
- 确定性：FACT（跨租户触发需幂等输入重合）
- 证据：`src/agent/outbox.rs:162-229,288-350,421-455,939-962`、`src/db/indexes.rs:904-927`；账号业务唯一键对照见 `src/db/indexes.rs:58-67`。
- 机制：OutboxEntry 保存 workspace_id，但普通文本幂等键只哈希 `source_event_id + contact_wxid + content_hash`；manual synthetic 只含 account/contact/content/day，媒体/名片 synthetic 只含 run/contact/asset-or-card。所有形态均未纳入 workspace。Mongo 唯一索引又只约束单字段 `idempotency_key`，所以冲突是跨整个集合而非 workspace 内生效。DuplicateKey 分支不校验既有行 workspace，直接返回 `IdempotentSkip` 及另一行的 run/decision/status。
- 影响：两个 workspace 出现相同幂等输入时，后入队租户的合法消息会被静默视为重复而不创建 Outbox；Gateway 随后可能标 `skipped_duplicate` 或 partial failure，造成跨租户拒绝发送。manual send 在同 account/contact/content/day 复用时尤为直接；普通 inbound 是否碰撞取决于上游 message id/contact id 的全局唯一性，代码和索引没有建立该保证。
- 建议：把 workspace_id（通常连同 account_id）纳入所有幂等原文，并将唯一索引改为 `(workspace_id, idempotency_key)`；上线前检测/迁移现有 key，避免直接改算法造成旧消息重放。DuplicateKey 回读必须带 workspace 并把跨 workspace 冲突视为不变量错误；增加两个 workspace 同 account/contact/source/content 的隔离测试。

## SR-027：用户 stop 取消固定作用于默认 workspace

- 严重度：P1
- 确定性：FACT（跨租户触发需存在非默认 workspace 的 reaction）
- 证据：`src/agent/reaction.rs:28-196,244-276`、`src/agent/outbox.rs:559-660`、`src/agent/outbox_dispatcher.rs:207-264,1585-1742`。
- 机制：Reaction 前半段正确按 contact.workspace_id 认领并更新实际 decision review，但触发 stop/cooldown 后只把 account_id/contact_wxid 传给取消 helper。该 helper 无 workspace 参数，固定用 `state.config.default_workspace_id` 查询并取消 pending/in_flight。发送前二次门只读取“当前 entry 自己的 decision review outcome”，不能替代“取消该联系人所有其它在途 decision”；而 reaction 只更新本次认领的已发送 review。
- 影响：非默认租户明确表示停止后，该租户其它 pending/in_flight Outbox 不会被批量取消，仍可能经 Dispatcher 发送。若默认 workspace 恰有相同 account/contact 标识，其在途消息反而会被错误取消，形成跨租户干扰。取消失败是 best-effort，仅写 warn，不会阻止 reaction 主路径。
- 建议：取消 API 强制接收 contact.workspace_id，并在初始查询及逐条 CAS 更新中都带 workspace/account/contact；事件继承同一 workspace。增加两个 workspace 复用 account/contact 的串联测试：非默认 stop 只取消本租户全部可取消条目，默认租户保持不变；同时覆盖不同 decision_id 的多条在途消息。

## SR-028：Reaction stale claim 无 fencing，旧分析可覆盖新结果并重复副作用

- 严重度：P2
- 确定性：FACT（并发触发需多副本或其它绕过去抖单 runner 的并行调用）
- 证据：`src/agent/reaction.rs:60-196,198-276,539-619`、`src/config.rs:456-467`、`src/llm.rs:826-855,1055-1070`、`src/webhooks.rs:54-70,127-243`。
- 机制：每次 Reaction 开始都会把超过 `REACTION_ANALYSIS_CLAIM_TIMEOUT_SECONDS`（默认 60 秒）的 `analyzing` review 直接重置为 pending，再重新 claim；claim 只保存时间，没有 owner/generation token。分析结束写回只按 review `_id`，不校验当前仍由本 worker 持有，也不校验 `outcome_status=analyzing` 或 claimed_at。默认单次 LLM HTTP timeout 为 45 秒、最多 5 次尝试并带退避，合法运行时长可明显超过 60 秒。单进程 PENDING 能串行同 contact，但代码明确说明多副本不共享；第二副本可回收仍在运行的第一份 claim，随后新旧 worker 都能提交。轨迹用 `$push` 追加；negative_example 的去重是无唯一索引的 count-then-insert，也不能抵御该并发窗口。
- 影响：较旧入站的分析可能在较新分析之后覆盖同一 review 的 outcome、reaction_analysis 与 reviewer_misjudge_signal；同一反应还可能重复追加 intent trajectory、重复生成待审负例或重复触发 stop 取消。Reaction 是旁路，当前不会直接吞掉本轮 Gateway 回复，但会污染后续 prompt、学习样本、Reviewer 指标和运营审计。
- 建议：claim 时写不可复用的 owner/generation，并让所有提交及副作用以 `(_id, outcome_status=analyzing, owner)` CAS 为前提；运行中续租或把 stale 阈值设为严格覆盖最坏 LLM 重试墙钟，但不能只靠延长时间替代 fencing。负例使用 `(workspace_id, source_review_id)` 唯一索引/upsert；增加“旧 worker 超时后、新 worker 提交、旧 worker 晚到不得覆盖或重复副作用”的多执行者集成测试。

## SR-029：Memory consolidation 多文档提交无持久化阶段标记，崩溃可重放或丢失附属写

- 严重度：P2
- 确定性：FACT
- 证据：`src/agent/memory.rs:1248-1272,1532-1800,2005-2057`、`src/db/indexes.rs:133-179,393-401,606-613`、`src/tasks.rs:188-319`。
- 机制：consolidator 先无 claim 地读取最多 30 条 pending candidate，经过 LLM 后以 memory_card_version OCC 更新 OperatingMemory；赢得 OCC 后，再依次 best-effort 写 confirmed_tags/personality、批量把候选标 consolidated、写事件并把 task 标 sent。OperatingMemory、Contact、MemoryCandidate、AgentEvent 与 AgentTask 之间没有事务，也没有 `consolidation_id/phase` 提交标记。若在 memoryCard 已提交而候选尚未标 consolidated 时崩溃，候选仍会被下一任务重新读取并再次调用 LLM；若 Contact 附属写失败，代码仍消费候选且不会留下可恢复待办。任务调度本身还是 find-then-insert，索引不约束同 contact 的活跃 memory_consolidation 唯一性，允许并发任务放大该窗口；OCC 只防同版本覆盖，不能恢复跨集合副作用。
- 影响：恢复路径可能重复消耗 LLM、用同一批候选再次生成并推进新版本，造成记忆摘要/事实顺序的非确定漂移；confirmed tags 或 personality 写失败时，memoryCard 与 Contact 派生视图可永久不一致，而候选已被标消费，普通重试不会补写。该问题不直接发送消息，且 memoryCard 本体有 OCC，因此定为 P2 而非发送链 P1/P0。
- 建议：为每次 consolidation 建 durable job/commit record，原子 claim candidate 并记录 candidate ids、base version、result hash 与 phase；各副作用按同一 consolidation_id 幂等 upsert，全部完成后再提交 candidates/task 终态。若 Mongo 部署支持事务，可将 winner 后的短 DB 写集合放入事务；否则使用可恢复 saga/outbox。为活跃 memory_consolidation 建 partial unique key，并加“memoryCard 后崩溃”“Contact 写失败”“重复任务并发”恢复测试。

## SR-030：Knowledge Agent 的正文打开路径绕过 account/domain 可见域

- 严重度：P1
- 确定性：FACT（跨账号触发需知道目标 ObjectId，或存在跨账号 relation/version 指针）
- 证据：`src/agent/knowledge_agent.rs:660-667,843-928,1049-1070,1128-1212,1264-1369`、`src/agent/knowledge_tools.rs:600-657,665-813,820-1194`、`src/routes/knowledge/chat.rs:1031-1089,1164-1200,1410-1453`、`src/routes/knowledge/wiki_edit.rs:683-778`、`src/routes/knowledge/sources_meta.rs:518-553,612-668`、`src/cold_contact_worker.rs:378-433,455-523`；生产 Router 的外层二次交集见 `src/agent/knowledge_router.rs:492-517,759-773`。
- 机制：初始 `list_catalog` 与 `open_document` 正确限定 `workspace + domain=user_operations + status=active + (account=null|当前 account)`，但 `open_chunk` 的签名没有 account，只按 `_id + workspace + integrity_status=verified` 取完整正文；`resolve_superseded` 与 `follow_relations` 同样没有 account/domain 可见域。模型可直接请求同 workspace 任意已知 ObjectId；更稳定的路径是 relation 写侧只验证 source/target 同 workspace，允许账号 A 的 chunk 指向账号 B 私有 chunk，随后 A 的 Agent 沿关系载入 B 正文。`/knowledge/ask` 又接受调用者提供的 account_id 并直接返回 Agent answer。知识工作台重复同一缺口：`run_chat_with_tools` 明明接收 account_id，却按 workspace 全量预载 active documents/verified chunks；六个直查 Mongo 的 chat-only 工具签名也只接 workspace，`search_chunks/open_document/audit/propose/verify_anchor` 可读取其它账号对象，`analyze_logs` 还允许模型自填任意 account_id 或省略后读取整个 workspace。Cold Contact worker 的 peer-case hook 也只按 workspace 加载 active document/verified `peer_case` chunk，随后把摘要注入某个具体 account 的 follow-up task，没有 account 可见域过滤。生产私聊 Router 最后会把 cited id 再与 A 的初始 corpus 求交，因此能阻止大部分跨账号 id 直接成为最终 selected chunk，但 B 正文此前已经进入 A 的 Knowledge LLM 上下文并可影响其自然语言 answer；该外层保护不覆盖独立 ask API、知识工作台或 Cold Contact task hook。
- 影响：同一 workspace 内按 account 私有化的运营知识、父文档原文与使用日志可被其它账号的知识探索读取并发送给模型 provider；在知识问答 API/工作台中还可能被归纳到返回正文或草稿。即使生产 Reply 最终 citation 被外层过滤，模型调用的数据最小化与 account 级隔离已经失效，跨账号内容还可能间接改变选路、修订建议与弃答结果。
- 建议：把可见域对象（workspace、account、domain、status policy）作为所有 list/search/open/audit/repair/log 工具以及 `open_chunk/resolve_superseded/follow_relations` 的强制参数并逐跳校验；relation/version 写入时拒绝跨可见域指针，或显式建模可审计共享关系；`/knowledge/ask` 与 chat 的 account_id 必须验证属于当前 workspace/管理员授权范围。`analyze_logs` 不应允许模型扩大到调用上下文之外的账号。增加 A→B 私有 relation、直接 B ObjectId、version redirect、父文档/日志工具与 ask/生产 Router/chat 三路径隔离测试。

## SR-031：Knowledge 引用与 contradiction 禁用只靠模型自律，服务端可接受伪造证据

- 严重度：P1
- 确定性：FACT（错误引用触发依赖模型违反 prompt）
- 证据：`src/agent/knowledge_agent.rs:919-997,1002-1033,1354-1369,1499-1527,1530-1602`、`src/agent/knowledge_router.rs:507-523,605-622`、`src/agent/guards.rs:307-368`。
- 机制：`follow_relations` 把 `contradicts` 目标正文放入 opened，仅附 `relation_role=contradiction` 并在 prompt 中要求“绝不可 cite”。终态 `filter_answer_against_opened` 却只用 chunk id 是否在 `opened_seen` 过滤，不知道 relation role；预算/轮数兜底更直接把所有 opened id（包括 contradiction）列为 cited。`RawSourceQuote` 只要指向任一 opened id 就被原样接受，既不要求该 id 同时在 cited 中，也不检查 quote 是否来自 chunk 的 `source_quote/body`，不校验 `source_anchor_index` 是否存在/匹配。Router 据这些自报 quote 把 coverage 提升为 enough；verified product gate 只验证 cited id 对应 chunk 的 verified/未过期状态，不验证引用文本或 contradiction 角色。
- 影响：模型一次协议偏离即可把明确标为矛盾材料的 chunk 当支撑，或为真实 opened chunk 编造 quote/anchor，使知识覆盖与审查获得虚假背书。该 route 后续可把相应 verified chunk 正文注入 Reply Agent，并参与产品声明硬门，最终错误事实或无依据承诺可能进入发送链。即使模型通常守 prompt，这里声明的“contradiction 不作支撑”和“source quote 证据”都不是服务端不变量。
- 建议：opened 集合保存结构化元数据而非仅 id；服务端剔除 contradiction/过期/不可见条目并让兜底也使用同一 eligible 集。quote 必须属于 cited id、非空、能在规范化 source_quote/body 中锚定，anchor index 必须有效且指向同一证据；失败时降 coverage/清 requires_evidence，而不是保留模型自报。增加恶意 LLM 引用 contradiction、伪造 quote、越界 anchor 与 truncated fallback 测试。

## SR-032：Knowledge answer cache 的 corpus 签名不含内容版本，编辑后可继续返回旧答案

- 严重度：P2
- 确定性：FACT
- 证据：`src/agent/knowledge_agent.rs:690-726,985-997,1049-1112,1372-1405`、`src/agent/knowledge_agent/cache.rs:1-13,23-36,82-143`。
- 机制：缓存注释声称签名由 `chunk_id+updated_at` 组成且任一改动自动失效，实际 key 只对初始 top-30 catalog 的 `(chunk_id, dynamic_confidence bits XOR related_count)` 哈希。title、summary、body、source_quote、source_anchors、relation 目标/类型、priority、validity 和 updated_at 均不在签名；只要编辑未改变 top-30 成员、dynamic_confidence 或关系数量，同 workspace/account/query 会命中最长 5 分钟的旧 `AnswerResult`，跳过全部数据库正文打开与 LLM 推理。Provider/model 也不在 key，热切换继承 SR-020 的旧 provider 产物风险。
- 影响：运营修正错误正文、引用或关系后，知识问答与生产 Router 仍可在 TTL 内复用旧答案、旧 cited id 和旧 quote；事故修订、合规口径切换与引用纠错不能即时生效。隔离维度本身包含 workspace/account，因此本项不是跨租户缓存碰撞。
- 建议：签名纳入确定性的内容版本（至少每个可见候选的 `_id + updated_at/version + status/integrity`，或 workspace/account corpus generation），并让所有知识写路径原子递增 generation；缓存 key 同时纳入 provider/model generation。不要只签 top-30 展示页，需覆盖能经 open_document/relation 打开的有效 corpus；增加 body/source quote/relation 等量编辑、top-30 外目标更新和 provider swap 的失效测试。

## SR-033：Knowledge Chat 工具循环强制终止不作用于返回 payload，且 30 秒限制不是整体 deadline

- 严重度：P2
- 确定性：FACT
- 证据：`src/agent/chat_tool_loop.rs:84-203,209-267`、`src/routes/knowledge/chat.rs:993-1031,1092-1200`；下游消费见 `src/routes/knowledge/chat.rs:143-202,918-928`。
- 机制：失败连击、tool budget 耗尽或达到四轮上限时，`chat_reply_with_tools_loop` 会清空内存中 `AgentDecision.tool_calls` 并把 `decision_phase` 改为 final；但调用方 `run_chat_with_tools` 完全丢弃 `outcome.decision`，无条件返回 `last_raw` 中最后一轮模型原始 JSON。该 raw 正是被协议要求在 `tool_calling` 轮不含 `naturalReply/patch/missingFields` 的中间态，因此“强制结束”没有落到业务 payload。上层不复核 `decisionPhase`，会把它持久化为默认文案“（AI 未给出回复）”、空 patch 与 `canApply=false`。此外所谓 30 秒总超时仅在每轮开头检查；`reply_fn` 自身没有剩余时间 timeout，同一轮最多六个工具又各自有独立 5 秒 timeout，循环可在一次检查后继续运行远超 30 秒。只有下一轮入口再次检查时才返回 Timeout；若第四轮结束则直接 Ok，即使总耗时已超限。
- 影响：模型持续请求工具、连续工具失败或预算刚耗尽时，运营看到的不是明确的截断/重试状态，而是无回复的成功 turn；已取得的工具结果和风险不会进入业务返回，失败仍被记录为普通 pending 对话。慢 LLM 或一轮多个慢工具还可突破接口宣称的 30 秒上限，占用请求、数据库连接与 LLM 配额，客户端超时后服务端仍继续工作。
- 建议：让工具循环返回经过强制收敛的业务 payload，或至少由调用方根据 `outcome.decision/risks` 把 raw 重写成明确的 final fallback，绝不返回 `decisionPhase=tool_calling`；上层在持久化前硬校验 final 与必需业务字段。用一个包住整个 future 的 absolute deadline（或每步按剩余 deadline timeout）覆盖 LLM 与全部工具，而不是只在循环顶部采样 elapsed。增加四轮全 tool_calling、failure streak、budget exhausted、单次慢 LLM 与 6×慢工具的时间/终态测试。

## SR-034：AgentTask 的 lease、取消与终态写入无 fencing，旧执行者仍可入队发送

- 严重度：P1
- 确定性：FACT（并发触发需 stale 扫描与续租交错、管理员取消运行中任务，或无心跳的 review-now 执行超过回收阈值）
- 证据：`src/tasks.rs:34-158,188-319,343-395`、`src/routes/tasks.rs:168-253`、`src/agent/gateway.rs:157-186,1233-1263,2760-3053,3150-3389,3994-4053`、`src/agent/outbox.rs:162-229`；其它 handler 的无条件终态写见 `src/agent/memory.rs:1273-1286,1639-1651,1788-1800`、`src/routes/contacts.rs:664-697`。
- 机制：Worker claim 只把任务改为 `running + claimed_at`，没有不可复用的 owner/generation。心跳和 stale recovery 都只按 `_id + status=running` 更新；recovery 先读出 stale 快照，随后 CAS 不比较读到的 `claimed_at`，因此即使原执行者已在两步之间成功续租，旧扫描仍可把它改回 retry/failed。管理员 review-now 虽原子 claim，却不启动心跳；管理员 cancel 又可无状态限制地把 running 改为 cancelled，但运行中的 handler 没有取消令牌。FollowUp 传给 Gateway 的 `should_abort_send` 固定为 None，Gateway 会先生成并创建 Outbox，最后才尝试把 task 标 `outbox_enqueued`；若任务已被取消，该末尾更新不命中，但已创建的 Outbox 不会撤销。错误重试、memory/initial-profile/outcome 成功终态等多处写回也只按 `_id`，旧执行者可覆盖新执行者状态。FollowUp 文本幂等键包含 task id 与正文 hash；并发重生成若文案不同会产生不同 key，不能防双发。
- 影响：运营取消运行中主动触达后，客户仍可能收到消息；stale 回收与原执行者并行时，同一任务可跑两份 LLM/Gateway 并入队两组不同文案。非发送任务也会重复副作用或让旧执行者把新状态覆盖成 retry/failed/sent，审计中的 cancelled、attempt、recovery 与真实执行事实分裂。心跳降低正常 Worker 的概率，但不能修复无心跳入口、读写竞态或进程暂停/DB 抖动后的所有权丢失。
- 建议：claim 时原子生成 `claim_generation/owner`，心跳、reclaim、handler 提交、错误重试及所有 task 终态都必须以 `(_id, status=running, generation, owner)` CAS；reclaim 还应比较扫描时的 generation/claimed_at。review-now 复用同一 lease/heartbeat 执行器。取消 running 时递增 generation，并让 Gateway 在写业务副作用及每次 enqueue 前检查 lease/cancel token；必要时取消已创建但未发送的该 generation Outbox。增加“扫描后心跳”“取消时 LLM 在途”“旧 worker 晚到”“review-now 超过 timeout”多执行者集成测试。

## SR-035：过期 principal relay 裸发后不写任务终态，Worker 会回收并重复发送

- 严重度：P1
- 确定性：FACT（触发需 relay task 执行时授权已经过期）
- 证据：`src/agent/escalation/mod.rs:159-239`、`src/tasks.rs:188-319`、`src/config.rs:453-459`；正常 relay 的 task 交付对照见 `src/agent/gateway.rs:906-1023,3034-3053`，现有测试缺口见 `tests/principal_decision_channel.rs:922-974`。
- 机制：`handle_principal_decision_relay` 发现授权过期后会清 awaiting 标记，直接调用 `mcp::logged_call_for_account(message_send_text)` 发中性收尾，然后返回 Ok；它不走 Outbox，也不把已 claim 的 task 写成 sent/cancelled/outbox_enqueued。Worker 的 Ok 分支只写 `follow_up_processed` 事件，明确依赖 handler 自写终态，所以任务保持 running；心跳随 handler 返回被 abort。默认 300 秒后 stale recovery 把任务改为 retry，下一次又执行同一裸 MCP 发送，累计第三次 recovery 才强制 failed。MCP 错误还被 `let _` 吞掉，无法让 Worker 进入正常错误重试语义。现有过期授权测试构造的 task 没有 `_id`、也未插入 tasks 集合，只断言一次 MCP 调用与 awaiting 清除，未覆盖生产 Worker 的终态/回收闭环。
- 影响：一次合法的授权过期 relay 可向同一客户重复发送中性收尾，最多随当前 recovery 阈值多次触达，最终任务还会被记成 `claim_recovery_exhausted/failed`，与实际已发送事实相反。该分支绕过 Outbox 的唯一幂等键、二次安全门、投递后核对和发送状态聚合，因此进程崩溃/MCP 超时窗口也没有可靠的去重恢复。
- 建议：过期 relay 也必须走 Outbox，使用 task id + 固定语义作为幂等源事件；只有 Dispatcher 确认送达后再清 awaiting 并提交 task sent。若暂时保留直发，至少在成功后以当前 claim generation CAS 写明确终态，并为 MCP 调用增加持久化发送 marker/post-hoc 核对；失败必须返回 Err。测试应插入真实 running task，执行 handler 后断言终态，并推进超过 claim timeout 验证不会再次调用 MCP。

## SR-036：outcome aggregation 任务唯一键缺 workspace，跨租户账号会互相吞任务

- 严重度：P2
- 确定性：FACT（跨租户触发需复用同一 account_id）
- 证据：`src/tasks.rs:397-451,483-720`、`src/db/indexes.rs:133-179`、`src/db/migrations/m017_dedupe_outcome_aggregation.rs:1-57`；账号租户主键与指标 id 对照见 `src/db/indexes.rs:58-67`、`src/tasks.rs:681-709`。
- 机制：每日调度会遍历所有账号，为每个 `(workspace, account, horizon, date)` 插入 outcome_aggregation task；但 partial unique index 只有 `(kind, account_id, content)`，content 仅含相同的 horizon/date，不含 workspace。两个 workspace 复用 account_id 时，后插入者命中 DuplicateKey，代码将其无条件当作“已有任务”忽略。配套 m017 迁移也只按 `(account_id, content)` 分组并删除其余记录，可能在建索引前把不同 workspace 的合法任务当重复项清掉。最终指标 `_id` 和聚合查询本身都包含 workspace，说明丢失发生在调度层而非指标模型有意共享。
- 影响：冲突租户当天的 7d/30d outcome 指标不会生成或刷新，运营面板、演化评估与长期效果判断出现静默数据缺口；保留下来的另一租户任务不会替它写指标。问题不直接发送消息，因此定为 P2。
- 建议：唯一键和 m017 分组统一加入 `workspace_id`（通常为 `workspace_id + account_id + kind + content`），上线迁移先删除旧索引、按新租户键检查真实重复再重建；DuplicateKey 回读/审计也应确认 workspace 相同。增加两个 workspace 复用 account_id、同日双 horizon 均各生成一条任务并写各自 metric 的集成测试。

## SR-037：LLM 对领导原话的解释可被直接提升为 workspace 级 verified 知识

- 严重度：P1
- 确定性：FACT
- 证据：`src/agent/escalation/mod.rs:241-281,322-348`、`src/agent/escalation/logic.rs:410-423,624-651`、`src/agent/gateway.rs:913-1004`、`src/agent/escalation/ledger.rs:349-438`；结构定义见 `src/models.rs:3693-3696,3722-3747`。
- 机制：领导微信原话不会结构化直存为授权事实，而是先由 `escalation.principal.interpret` LLM 生成 `PrincipalDecision`。服务端 `sanitize_verdict` 只校验 verdict 是否在闭集，原样保留模型生成的 `substance`、`constraints`、`authorization_window_hours` 与 `exemption_type`；连 exemption_type 是否在闭集都不校验。模型只要输出授权 verdict 和 `exemption_type=knowledge`，relay 后就以其改写的 substance 自身作为“来源锚点”，创建 workspace 共享、active、`product_fact` chunk，并以 `PrincipalAuthorized` 直接 Verify 到 confidence=100。锚点证明的只是“模型输出与自身一致”，不是它忠实对应领导原话；代码却把验证者记为领导本人。
- 影响：解释模型的遗漏、扩写或幻觉可从一次个案回复升级为全 workspace 可召回的 verified 产品事实，后续通过 R5.4 产品声明硬门并影响所有账号/客户。模型还可把领导未表达的个案决定判成 knowledge，或把未提时限判成长期有效授权；这突破“AI 永不自动验证”的实际信任边界。
- 建议：持久化不可变的领导原文及消息 id/hash，把授权字段限制为服务端闭集并校验有限、合理的时窗；`knowledge` 必须来自领导明确、可审计的结构化动作（例如带短码的确认语法或 admin 二次确认），不能由解释 LLM 单独决定。知识 source quote/anchor 应指向领导原文，模型只能提供候选摘要；原文与摘要不一致或无法证明 knowledge scope 时降为 draft/needs_review。增加模型改写数字、擅自 knowledge、越界 exemption、未提期限和原文—摘要不一致测试。

## SR-038：首次请示卡发送失败会留下永久去重的“幽灵 pending”

- 严重度：P1
- 确定性：FACT
- 证据：`src/agent/escalation/ledger.rs:17-92,118-139,580-624`、`src/agent/escalation/mod.rs:80-156`、`src/agent/gateway.rs:844-902`、`src/db/indexes.rs:1890-1909`。
- 机制：approved 与 hold 两条升级路径都先插入 pending，再裸调用 MCP 推领导卡；插入时已经把 `last_pushed_at_ms` 设为当前时间。若 MCP 返回错误，函数虽返回 Err，但已写台账不回滚；上层又只 warn、不中断客户 run。后续同 workspace/contact/category 会被 `has_pending_for_contact` 和 partial unique 索引永久挡住，不会重新向原决策人推卡，骚扰门统计还把这次失败计成已推送。默认 `timeout_hours=None` 时没有后台补偿；即使有 timeout，scanner 也只尝试下一决策人，不重送原首卡。
- 影响：客户被告知“正在确认”且联系人进入 awaiting 语义后，领导可能从未收到请示。单人链或无限等待配置会使议题长期 pending；同类后续请求被静默吞掉，管理员看见的台账也错误暗示卡已送达。
- 建议：把投递状态显式建模为 `created/dispatch_pending/delivered`，首卡经可靠 Outbox 或专用幂等发送表投递，只有确认送达后再写 `last_pushed_at_ms`/进入 pending 等待态。若必须先建台账，失败必须保留可重试状态和稳定 idempotency key，去重判断不能把未送达记录当作已在等待。增加首次 MCP 失败后重试成功、单决策人、timeout=None 与进程崩溃窗口测试。

## SR-039：escalation timeout scanner 无 claim/CAS，多副本可重复推卡和安抚客户

- 严重度：P1
- 确定性：FACT（重复副作用需两个 scanner 并发、或发送成功后状态更新失败）
- 证据：`src/tasks.rs:161-210`、`src/agent/escalation/mod.rs:354-477`、`src/agent/escalation/ledger.rs:524-577`；单实例顺序测试见 `tests/principal_decision_channel.rs:360-790`。
- 机制：每个 Worker tick 在 AgentTask claim 之前直接运行全库 scanner；扫描条目没有 owner、lease 或原子 claim。多个实例可同时读到同一 pending，分别先裸 MCP 推给 next，再调用 `reassign_escalation`；后者只过滤 workspace/short_code/pending，不校验扫描时的 principal、updated_at 或 generation，因此两个发送都可能发生，晚到更新还可覆盖较新的改派。链尾分支同样先裸发客户安抚再 touch；MCP 结果被 `let _` 丢弃，即使发送失败也会更新时间抑制后续重试，而并发实例又可在 touch 前各发一次。
- 影响：同一内部请示卡可被重复发送给备选决策人，客户也可收到重复链尾安抚；数据库最终只保存一次时间/改派，无法反映真实副作用。发送成功但更新失败会在下个 tick 重发，发送失败却 touch 成功则反向造成消息丢失。现有测试只串行调用 scanner，不能覆盖多进程竞争。
- 建议：为每条到期 escalation 增加原子 claim/lease/generation，发送与提交均以 generation fencing；更稳妥的是把每次领导卡和客户安抚都写入可靠 Outbox，使用 `(escalation id, recipient, phase/generation)` 幂等键，scanner 只推进持久状态。reassign 必须比较 expected principal/updated_at；失败发送不得 touch。增加两个 AppState/worker 并发扫描、发送后更新失败、MCP timeout 和 lease 过期接管集成测试。

## SR-040：领导回复匹配忽略 account，可从一个业务号裁决另一个账号的 pending

- 严重度：P1
- 确定性：FACT（触发需同 workspace 多账号复用同一 principal wxid）
- 证据：`src/webhooks.rs:443-459`、`src/agent/escalation/mod.rs:284-350`、`src/agent/escalation/ledger.rs:94-139`、`src/db/indexes.rs:1862-1909`；联系人租户键对照见 `src/db/indexes.rs:77-84`。
- 机制：Webhook 已解析当前入站 `account_id` 并传入 handler，但 `list_pending_for_principal` 只按 workspace/principal/status 查询。无短码且全 workspace 恰有一条 pending 时，账号 A 收到的领导自然消息会自动匹配并 resolve 账号 B 的条目；带短码也可跨账号命中。入站 account 仅用于解释 LLM 和歧义回复的 MCP 上下文，实际 relay task 又按被命中 entry 的 account 发给 B 客户。`has_pending_for_contact` 与 pending 唯一索引也缺 account，使同 wxid/contact category 在同 workspace 的不同账号互相抑制。
- 影响：领导在一个业务号上的回复可能裁决另一业务号的客户议题，产生错误授权、错误客户触达或跨账号业务信息串扰；多个账号的 pending 还会制造无关歧义，或让一个账号的 pending 阻断另一个账号正常请示。workspace 过滤阻止了跨 workspace，但没有满足仓库普遍采用的 `(workspace, account, contact)` 业务身份边界。
- 建议：所有 principal pending 查询、去重和唯一索引纳入 account_id；handler 只能匹配当前 webhook account 的条目。若产品确实要求领导跨账号统一收件，应使用显式的全局 inbox/路由 id，并要求短码确认，不能把任意账号上的无短码自然消息自动套到其它账号。迁移前按新键审计现有冲突，并增加同 workspace 两账号同 principal/contact 的带码、无码、歧义和去重测试。

## SR-041：timeout scanner 无法识别 escalation 所属 domain，会套用 workspace 内其它域策略

- 严重度：P1
- 确定性：FACT（触发需同 workspace 存在多个 current domain config，且至少一个配置 timeout）
- 证据：`src/agent/escalation/mod.rs:354-477`、`src/agent/escalation/ledger.rs:504-555`、`src/models.rs:1341-1388,3750-3798`、`src/agent/escalation/policy.rs:20-51,103-131`。
- 机制：`OperationDomainConfig` 的策略按 `(workspace, domain)` 定义，但 `AgentPrincipalEscalation` 没有 domain 字段。scanner 遍历每一条 current config，却在每次循环都用 `list_escalations_by_workspace(workspace,"pending")` 取得该 workspace 全部 pending，并把当前 cfg 的 timeout、decider chain、quiet hours 和 cap 套到所有条目。多个 domain 时，同一 escalation 会被多套互不相关的策略依次判断；哪条 config 先返回、哪套链包含当前 principal 会改变结果。
- 影响：一个域的请示可能被另一域更短的 timeout 提前改派，并把卡点、客户标识和问题推给无关决策人；也可能被错误链尾判断而改发客户安抚，或因其它域的骚扰门被压制。多条 current config 本是正常模型能力，因此不能依赖“当前只有 user_operations 一域”作为持久不变量。
- 建议：创建 escalation 时持久化不可变 `domain`（必要时连同 policy version/snapshot），scanner 按 workspace+domain 查询并只应用对应策略。旧记录迁移需有可验证映射，无法确定时进入 admin 待处理而非任选 current config。增加同 workspace 两域、不同 timeout/decider chain 的隔离测试，并断言每条 pending 每 tick 最多产生一次状态推进。

## SR-042：所有生产 holding-reply 调用都绕过数字护栏，客户承诺仅靠 prompt 约束

- 严重度：P1
- 确定性：FACT（实际误发取决于 LLM 是否偏离 prompt）
- 证据：`src/agent/escalation/holding_reply.rs:9-32,35-64,66-123`、`src/agent/gateway.rs:1139-1175`、`src/agent/escalation/mod.rs:177-218,386-426`；调用点全集见 `src/agent/gateway.rs:1169`、`src/agent/escalation/mod.rs:199,399`。
- 机制：运行期守卫始终检查空文本和禁词，但数字校验只在 `scene=ExpiredAuthorization` 且 `authorized_substance=Some` 时执行。GateHold、ChainTail 两类本就不走该分支；唯一的 ExpiredAuthorization 生产调用又明确传 None，所以三处生产调用实际上都不校验数字。是否“不承诺结果/数字/时间”完全依赖 system prompt；服务端也没有检测承诺语义。生成文本随后进入客户占位 Outbox，或在链尾/过期分支裸 MCP 发送，未经过 Reply/Reviewer 的产品声明语义门。
- 影响：模型偏离 prompt 时可在本应中性的过渡话术中编造折扣、金额、成功率、日期或结果承诺；这些内容没有领导授权或 verified 知识，却会直接触达客户。过期授权场景尤其可能重新说出新的数字，破坏“过期即不可用”的安全语义。
- 建议：holding reply 应优先使用确定性模板；如保留 LLM 润色，建立与场景无关的 fail-closed 数字/日期/金额/百分比检测，并对承诺结果使用确定性分类或受限模板槽位。ExpiredAuthorization 不应以 None 表示“无需检查”，而应表达“允许数字集合为空”；ChainTail/GateHold 同样禁止任何数量事实。三类文本统一走可幂等的出站路径，并增加每个生产调用点的数字、日期、金额、百分比和承诺措辞回归测试。

## SR-043：DomainProfile 的 current/active 切换无事务与唯一约束，运行时可随机选错画像

- 严重度：P1
- 确定性：FACT（并发触发需两个 publish/rollout/activate 操作交错；单请求也可在中间 DB 写失败时触发）
- 证据：`src/agent/domain_profile.rs:1045-1060`、`src/routes/domain_profiles.rs:281-377,380-475,478-515,645-695,804-829`、`src/db/indexes.rs:1626-1650`。
- 机制：运行时只加载 `is_active=true AND current_version=true`，但缓存用无排序 `find` 拉全表，同 workspace 多行命中时由游标最后一行覆盖。写侧却用多个独立操作维护不变量：publish/rollout/rollback 先 promote 新 current、再 demote 旧行、再迁移 active；activate 先把目标置 active、再清其它行；`next_version_for_profile` 也是先读 max 再 insert。`domain_profiles` 的 `(workspace,profile,version)` 与 `(workspace,is_active)` 都只是非唯一索引，也没有“每血缘唯一 current”索引或事务。并发或任一中间步骤失败可留下重复 version、多条 current/active，或 active 与 current 分居两行。
- 影响：同一 workspace 的 Agent 可在缓存 reload 后不确定地套用不同人格、闸值、交易事实开关、对话模式与状态机；若 active/current 分裂则查询零命中并静默回落销售 DEFAULT。管理端 `find_one` 也会展示任意一行，运营看到的生效画像可能与 Agent 实际使用者不同。后续缓存失效不能修复脏状态，只会重新随机选择。
- 建议：先检测并修复重复 version/current/active 数据，再建立 `(workspace_id,profile_id,version)` unique、`current_version=true` 的每血缘 partial unique，以及 `is_active=true` 的每 workspace partial unique；将 publish/rollout/rollback/activate 的标记切换放入事务，或改为 workspace 单文档 active-profile 指针并以 CAS 原子替换。缓存若仍遇到多行应 fail-closed 报不变量错误，不能“最后一行赢”；增加并发 publish/activate、每个中间写失败与 reload 确定性测试。

## SR-044：动态画像维度名未经路径校验，可覆写 domain_attributes 保留字段或制造嵌套路径

- 严重度：P1
- 确定性：FACT（触发需 active DomainProfile 声明恶意或误配置的 `profile_dimensions[].kind`）
- 证据：`src/models.rs:200-203,2523-2531,3662-3677`、`src/agent/domain_profile.rs:1147-1155`、`src/agent/domain_signals.rs:165-190,193-229`、`src/agent/gateway.rs:4349-4367,4503-4555`；profile 写入口对照见 `src/routes/domain_profiles.rs:163-193,199-254`、`src/routes/guide_profile.rs:368-409`。
- 机制：profile dimension 的 `kind` 是无格式约束的字符串；create/update/AI 生成只做 serde 结构校验，不拒绝点号、`$` 前缀、空白、重复项或系统保留名。Gateway 的白名单又直接来自这些 kind，`insert_domain_signal_values` 最终把每个键拼成 Mongo `$set` 路径 `domain_attributes.{kind}`。因此 `kind=value_tier`、`awaiting_principal_decision`、`principal_product_exemption` 会与交易分层和领导授权状态共用物理键；含点号的 kind 会被 Mongo 解释为嵌套路径，`$`/非法路径还可使整个联系人更新失败。白名单只能证明“profile 声明过”，不能证明键名安全。
- 影响：一次错误 profile 激活后，LLM 的普通维度字符串可覆盖系统派生值或把布尔/子文档改成错误 BSON 类型，使等待领导、客户级产品豁免、价值分层及其它下游判断静默失效；点号路径还能改写这些保留子文档的内部字段。非法路径导致 `apply_agent_updates` 失败时，决策/发送与画像副作用可能分裂。由于 profile 需要管理端创建/人审激活，本项不是未经认证的直接注入，但它突破了“开放维度只写自己的命名空间”的配置安全边界。
- 建议：为 dimension kind 建立服务端单一验证器（如 `^[a-z][a-z0-9_]{0,63}$`），拒绝点号、`$`、空白、重复 kind，并维护 domain_attributes 系统保留名/前缀黑名单；create、update、AI candidate publish/activate 和 migration seed 全部复用。更稳妥的是把开放维度放进独立 `domain_attributes.signals.<escaped-key>` 子文档，与系统状态物理隔离。激活前对存量 profile 做审计，增加保留名、点号、美元符号、重复键及正常自定义维度测试。

## SR-045：同一未知 taxonomy 值在 Decision 与 Gateway 各写一次，occurrences 不再代表真实出现次数

- 严重度：P2
- 确定性：FACT
- 证据：`src/agent/decision.rs:990-1032`、`src/agent/decision_taxonomy.rs:130-165`、`src/agent/gateway.rs:1915-1970`、`src/agent/taxonomy.rs:370-488`、`src/db/indexes.rs:1139-1180`。
- 机制：Reply 决策解析阶段遇到 CandidateNew 会调用 `validate_and_normalize_decision`，由 detached task fire-and-forget `upsert_candidate`；同一份 final decision 到 Gateway 后又经过 `compute_taxonomy_guard_outcome`，并同步调用同一个 upsert。候选以 `(workspace,scope,kind,raw_value)` unique，已有 pending 每调用一次就 `$inc occurrences:1`，所以一轮正常未知值通常记成两次。两调用并发首次插入时，又都先 `find_one` 后 `insert_one`；输掉 E11000 的调用直接返回 Ok，不补做 increment，因此调度时序可得到 1 或 2，注释所称 unique+retry 实际没有 retry。
- 影响：后台按 occurrences 判断候选热度、排序或审核优先级时会系统性高估，且同样的一轮在不同调度时序下计数不同；若后续基于阈值自动提示/聚合，结果不可复现。候选仍不阻塞回复、unique 索引也防止重复行，因此本项不等同于客户发送或正式字典被直接污染。
- 建议：只保留一个候选持久化责任点，优先由 Gateway 在最终决策确定后写一次；Decision 层只做纯分类/改写。将 upsert 改成单条原子 update-with-upsert 或在 E11000 后按状态执行同一增量逻辑，并明确 occurrences 是“决策轮次”还是“写入尝试”。若两阶段都必须观测，使用 `(run_id,kind,raw)` 去重事件表。增加单 run 未知值=1、并发首次写=准确累计 N、rejected/approved 状态不误增的 DB 集成测试。

## SR-046：taxonomy alias 无唯一归属约束，缓存可把同一原值不确定改写到不同 canonical id

- 严重度：P1
- 确定性：FACT（触发需同 workspace/scope/kind 的 active 条目存在重复 alias，或 alias 与其它 canonical id 冲突）
- 证据：`src/agent/taxonomy.rs:127-161,205-268`、`src/routes/admin_taxonomies.rs:143-181,205-262`、`src/routes/admin_taxonomy_candidates.rs:314-392`、`src/db/indexes.rs:961-985,1081-1135`。
- 机制：缓存无排序加载 current taxonomy，`check_value` 在一个 scope 内先遍历 canonical id，再对 active aliases 用 `find` 取第一条。索引只保证 `(workspace,scope,kind,value.id,version)` 唯一，不约束 alias 在同 kind 内只能属于一个 canonical，也不禁止 alias 等于另一条 canonical id。直接 create/patch 和候选 approve 只 trim/去空，不做跨条目冲突检查。重复 alias 因 Mongo 游标/缓存向量顺序决定改写目标；alias 与 canonical 冲突时 canonical 固定优先，使另一个条目声明的 alias 永远不可达。
- 影响：同一 LLM 原值在 reload、重启或数据重建后可能被改写成不同画像值，继而影响状态迁移、planner 权重、终态/再激活判断及落库画像；运营界面又允许成功保存这种歧义配置。错误发生在正式 active 字典，不是候选软通道，因此后续决策会持续复现，直到人工清理。
- 建议：把同 `(workspace,scope,kind,current)` 下的 canonical id 与全部 aliases 视为一个唯一命名空间；create/patch/approve/publish 在事务内查询冲突并返回 409，发布版本时验证整组。Mongo 对数组元素跨文档的条件唯一约束不便直接表达，可维护规范化 alias-claim 集合或每个 alias 一行的映射表并设 unique。缓存 reload 遇到历史冲突应记录高优告警并 fail-closed 不改写，而不是首条获胜；增加 alias-alias、alias-canonical、account/global 优先级及 reload 顺序测试。

## SR-047：贝叶斯槽位把同轮重复维度当成多轮命中，可在一轮内越过占槽门

- 严重度：P2
- 确定性：FACT
- 证据：`src/agent/bayesian_slots.rs:43-139`、`src/agent/gateway.rs:4245-4278,4683-4717`、`src/agent/types.rs:108-124,457-474,1006-1021`。
- 机制：设计把 `history.len()` 定义为“跨轮命中数”，但 `build_observed_dimensions` 只截前 6 项，不按 dimension 去重；RawAgentDecision 也直接透传数组。`apply_bayesian_update` 对每个数组项逐一查同 dimension 并 push 一个使用相同 `turn` 的 history 点，随后以 history 长度和强点数量判定占槽。LLM 在同一轮返回三条同名维度（可同值或不同值）时，即可一次累积 hits=3；若都锚到同一 Inbound 证据，strong 也累积到 3，从而满足默认 3/2 阈值并 lock。
- 影响：旁路趋势图可把一轮重复输出展示成已被多轮确认的稳定客户维度，history 还会在同一 turn 出现互相矛盾的值。当前代码明确声明 bayesian_signals 永不驱动决策、筛选、状态机或发送，所以本项不等同于行为门被绕过，定为 P2；若未来消费该槽位驱动策略，严重度需前置提升。
- 建议：在调用 update 前按 `(dimension,turn)` 归一为每轮最多一个观察；同轮重复值可取最高置信/合并唯一证据，冲突值则丢弃或标 ambiguity，不得增加多次 hit。`apply_bayesian_update` 本身也应防御性检查最后 history turn，保证每 dimension 每 turn 至多一点。增加同轮三重复不占槽、同轮冲突、跨三轮正常占槽及六项截断后去重预算测试。

## SR-048：Shadow/Simulation 标称纯演练却会写生产记忆与知识整改队列，且未执行生产终态硬门

- 严重度：P1
- 确定性：FACT（知识 gap/proposal 写入取决于本次召回是否无引用或低产出）
- 证据：`src/agent/simulation.rs:76-216`、`src/agent/prompt_shadow.rs:220-345`、`src/agent/memory.rs:791-969,2061-2122`、`src/agent/decision.rs:606-624`、`src/agent/knowledge_router.rs:443-503`、`src/agent/knowledge_agent.rs:282-355,1688-1780`、`src/routes/simulations.rs:37-69,157-210`；现有零副作用测试只计 outbox/outbound，见 `tests/simulation_no_sideeffect_integration.rs:72-139`。
- 机制：两个影子入口都调用 `load_or_create_operating_memory`，它在记忆不存在时插入生产 `operating_memories`，已有空卡时还会用 OCC 回填；`decide_reply` 加载 operator memory 时又把命中行的 `last_used_at` 更新为当前时间。知识路由最终调用 `knowledge_agent::answer`，零引用会 detached upsert `knowledge_gap_signals` 并额外调用 LLM 生成追问，低产出还会创建 structural proposal。与此同时 Simulation 只用 `review_passed` 生成 `would_send`，没有调用生产 `finalize_review_for_send`、`apply_state_action_gate` 或 revision 二审；Prompt Shadow 同样只比较原始/新 review 分数。路由却把结果描述为“prod 同源 review 终态”，而场景 evaluator 只把 `review_blocked/gateway_blocked` 视为失败，`no_reply` 对所有内置场景（包括明确购买兴趣、产品质疑）都无条件算通过。现有测试仅断言 outbox 与 outbound 消息数不变，也只覆盖 would_send/review_blocked，无法发现上述生产写入、终态门缺失和 no_reply 假通过。
- 影响：管理员反复跑演练或自动 Evolution replay 会创建/续期真实客户记忆、制造知识缺口工单和结构调整候选，污染运营队列与后续生产 prompt；影子结果还可能把生产会因协议风险、无 verified 产品背书、状态动作策略或 revision 失败而拦截的回复标成 `would_send`，或把应答场景的静默 `no_reply` 计为成功。这既破坏“applied=false/纯演练”契约，也使评测结论系统性偏乐观。
- 建议：给所有读取链引入显式 `RunMode::Shadow`/只读 repository：影子模式禁止 create/update memory、operator-memory touch、gap signal、structural proposal、usage/候选等任何持久副作用；或在可回滚事务/隔离数据库快照中运行。抽出与生产完全相同的纯终态判定，Simulation 必须执行 finalize、state-action 与 revision 语义但不持久化事件/outbox。测试应对所有可能写入集合做前后快照，而非只看 outbox/outbound，并断言同一输入的生产 dry-run 与 simulation 终态一致。

## SR-049：Prompt Shadow 用裸 message_id 串租户取源消息，并把当前运行态与历史基线直接比较

- 严重度：P1
- 确定性：FACT（错取消息需不同 workspace/account 复用 message_id；证据混杂在所有随时间变化的配置上恒成立）
- 证据：`src/agent/prompt_shadow.rs:89-189,220-345`、`src/evolution/replay.rs:180-217,220-241`、`src/db/indexes.rs:102-118`、`src/evolution/significance.rs:210-255,469-568`、`src/evolution/release.rs:181-280`。
- 机制：消息表的唯一身份是 `(workspace_id,account_id,message_id)`，但 replay 的 retention probe 和 Prompt Shadow 实际 `find_one` 都只过滤 `message_id`。同一 message id 在不同租户合法存在时，probe 只证明“某处有一条”，实际读取由无排序 `find_one` 任取；随后代码又把该消息与源 run 的 contact 混合。即使没有碰撞，所谓“原 prompt + 候选片段”也不是受控 A/B：original 侧直接读取历史 run 分数，新侧却用当前 contact、当前 playbook/domain profile、当前 memory、当前消息窗口、当前知识库、当前 active prompt、当前阈值和当前 provider 重跑，只在其中某个 prompt key 追加 snippet。任何中间演进都被算进候选差异。Prompt grader 只要至少一条 replay completed 就把 proposal 标为 `eligible_for_release`，不要求改善或样本可比；管理员虽仍需 release，页面收到的核心对照证据已失真。
- 影响：跨租户碰撞时，A 租户的真实客户消息可进入 B 租户联系人/知识/记忆上下文，污染 LLM 输入与 shadow evidence；正常无碰撞时，模型、知识或客户状态变化也可被误归因于 prompt candidate，使有害片段看似改善、有效片段看似退化。由于 completed 即可进入可发布状态，这不是单纯统计噪声，而会直接影响生产 prompt 发布决策。
- 建议：probe 与读取统一使用源 run 的 `(workspace_id,account_id,message_id)`，并校验消息 contact 与 source run 一致。创建 run 时持久化可重放快照/内容寻址引用：prompt 各 key/version、runtime/threshold、profile/playbook、memory/context、知识 chunk/version、recent message ids、provider/model 参数；A/B 两侧必须在同一冻结输入和同一模型设置下各跑一次，唯一变量是候选片段。无法重建完整对照时标 `non_comparable`，不得计 completed；grader 至少要求可比样本和明确人工风险提示。

## SR-050：主动发送台账忽略 account 且无 outbox 唯一锚，响应归因、判重与统计会串账号或重复计数

- 严重度：P1
- 确定性：FACT（串账号需同 workspace 多账号复用 contact wxid；重复计数需台账写后进程崩溃/调用重放）
- 证据：`src/agent/send_ledger.rs:40-125,174-317`、`src/agent/outbox_dispatcher.rs:1653-1705,1744-1790,1795-1846`、`src/db/indexes.rs:330-357`、`src/routes/send_ledger.rs:39-141`；模型明明保存 account，见 `src/models.rs:1305-1338`。
- 机制：台账行存有 `account_id`，但近期发送历史只按 `(workspace,contact_wxid,send_kind)`，response 回扫查询消息只按 workspace/contact/direction/time，当前 contact 也只按 workspace/wxid；因此账号 A 的入站、阶段和历史可归因到账号 B 的素材发送。索引与只读 contact history API 同样省略 account。另一方面，`record_send_for_entry` 在正常成功、reclaim post-hoc 和 timeout post-hoc 三条分支都执行普通 `insert_one`，台账模型没有 `outbox_id`，数据库也没有 outbox/run+target 唯一约束；发送已标 sent 后、台账 insert 后若进程在返回前崩溃，恢复路径或人工重放可为同一 Outbox 事实再插一行。
- 影响：响应率、阶段推进率、素材/名片发送量和 prompt 的“近期已发素材”会跨账号互相污染；Agent 可能因另一个业务号发过素材而错误抑制当前账号，后台则把另一个账号的客户回复算成本次触达转化。重复台账会进一步抬高发送量、改变率的分母并制造虚假历史，而现有 workspace-only 聚合无法识别或纠正。
- 建议：所有 contact 级查询、API 与索引使用 `(workspace_id,account_id,contact_wxid)`；回扫消息/contact 必须消费 row.account_id。给台账增加不可变 `outbox_id`（或 delivery id）并建 unique，使用 upsert `$setOnInsert`，三条确认送达路径共享同一个幂等写；迁移前按 outbox/run/target/time 审计可能重复。测试覆盖同 workspace 两账号同 wxid 的历史、响应和阶段隔离，以及 insert 后崩溃/三种确认路径重复调用仍恰好一行。

## SR-051：产品目录硬门只验证 LLM 自报的 product_id，不校验回复中的价格与目录事实一致

- 严重度：P1
- 确定性：FACT（实际错误放行取决于 Reply/Reviewer 同时输出错误正文并自报任一有效 id）
- 证据：`src/agent/entitlements.rs:249-288,371-388`、`src/agent/types.rs:168-176,442-443,978-979`、`src/agent/gateway.rs:1856-1886,2135-2164`、`src/agent/review/gates.rs:660-716`。
- 机制：目录确实把结构化名称/价格/币种/SKU 注入 prompt，但发送前的 `priced_from_active_catalog` 只检查 `decision.quoted_product_ids` 中是否有任一字符串等于任一 active product id。该字段和 `reply_text` 都来自同一次 LLM 输出，服务端没有从回复中提取价格/产品，也不验证自报 id 对应的名称、金额、币种、SKU或其它声明；多个引用中只要一个合法 id 即令整个产品声明获得目录背书。随后 R5.4 只要 reviewer 自报 `requiresProductKnowledge=true`，`priced_from_catalog=true` 就与 verified chunks 等价，跳过 `blocked_unverified_product_claim`。
- 影响：模型可回复错误价格、把 A 产品价格套到 B、编造折扣/功能，同时附带任一真实 product id，通过本来用于 fail-closed 的产品声明硬门。SR-022 是 reviewer 漏报时不进硬门；本项是在 reviewer 正确识别产品声明后，目录背书本身仍可被同一不可信模型自证，二者为独立根因。
- 建议：不要把自由文本 `quoted_product_ids` 当作背书证明。让模型输出结构化 claim（product_id、claim_type、amount_minor、currency、sku/value），服务端逐项与 active catalog 精确比对，并从最终回复中的金额/产品标记反向校验；任一无法映射或不一致即要求 revision/hold。更强方案是价格句由服务端从目录模板化生成，LLM只选择 product id 和自然语言上下文。增加有效 id+错误价、A/B 串价、多 id 一真一假、币种/SKU错误及 revision 后重校验测试。

## SR-052：名片 Outbox 在 timeout/reclaim 窗口无任何送达核对，会确定性重发已成功名片

- 严重度：P2
- 确定性：FACT（重复触发需 MCP 已送达但本地未及时标 sent：超时、崩溃或响应丢失）
- 证据：`src/agent/referral.rs:115-195`、`src/agent/outbox_dispatcher.rs:98-160,1463-1529,1653-1705,1732-1848,1964-1968`；现有测试只覆盖入队幂等，见 `tests/referral_card_push_integration.rs:187-260`。
- 机制：名片发送在 MCP 返回可验证成功后才写本地 message/已引荐状态，且这些后置写失败会 fail-soft，这是正确的“已送达不重试”纪律。但 Dispatcher 外层 timeout 或进程在远端送达后、本地标 sent 前崩溃时，Outbox 会进入 retry/reclaim。`verify_already_sent` 对 `referral_card_id.is_some()` 固定返回 `Ok(false)`，明确跳过 chat search、MCP log 或专用回执核对，所以恢复执行必然再次调用 `message_send_namecard`。入队 idempotency key 只能防止创建第二条 Outbox，不能阻止同一条 Outbox 的第二次物理发送。
- 影响：客户会收到重复顾问名片，发送台账和本地已引荐状态还可能只记录后一次或产生重复行；重复引荐破坏对话体验，也可能把一次业务动作误计为多次触达。默认 timeout 150 秒、lease 180 秒降低正常路径概率，但无法覆盖进程崩溃、网络回包丢失和远端慢响应。
- 建议：为名片发送建立可查询的稳定 delivery marker（outbox_id/idempotency token），让 MCP 调用日志在请求发出前持久记录 attempt、成功响应后记录 message id；reclaim/timeout 优先按 `(workspace,account,recipient,target_wxid,outbox_id/time)` 做权威或本地核对。若 MCP 无查询能力，采用“发送意图 + 单次不可重放”状态并将不确定结果转人工核验，不能自动重发。增加远端成功后 timeout、成功后进程崩溃、日志写失败与正常 retry 的集成测试。

## SR-053：Agent Soul 发布先物理删除其它版本且允许原地改写已发布行，失败或并发会长期降级人格

- 严重度：P1
- 确定性：FACT（单请求中间失败即可触发；并发发布会放大为互删）
- 证据：`src/routes/souls.rs:63-179`、`src/agent/decision.rs:338-351,1383-1404`、`src/prompts.rs:104-129,180-321,432-451`、`src/db/indexes.rs:359-366`；现有隔离测试只验证 workspace 过滤形状，见 `tests/workspace_isolation.rs:296-430`。
- 机制：Soul 没有不可变版本生命周期。`update_agent_soul` 可按 `_id+workspace` 原地改写任意行，包括当前 `published` 行的 `agent_kind/name/content`，无需重新发布或审计；`create` 又用“查最大 version + 1 后 insert”，没有版本唯一索引。发布时先 `delete_many` 物理删除同 workspace/kind 下除目标外的全部版本，再单独把目标置 `published`。若第二步失败，目标仍是 draft、旧 published 已永久删除；两个目标并发发布时，双方可先后删掉对方，最终零行或只剩非预期行。索引只是普通 `(workspace,kind,status,version)`，不维持每 kind 恰一条 published。`ensure_prompt_pack_v2` 在非空 prompt 库只对齐 prompt templates，不补 Soul；运行时查不到 published Soul 时静默回落一段短内置人格，因此异常会长期存在而不报警。
- 影响：一次数据库抖动、并发管理操作或误编辑即可无审计地立即改变生产人格，或让完整 Soul 永久消失并降级到通用短人格。历史版本被物理删除，无法回滚或解释某次 run 使用了什么人格；多 workspace 场景还叠加既有默认-workspace加载问题（SR-021），但本项即使单租户也可触发。
- 建议：Soul 行一经创建即不可原地改版本内容；编辑产生新 draft，发布在事务中以 CAS/唯一约束完成“新 published + 旧 archived”，绝不物理删除历史。建立 `(workspace_id,agent_kind,version)` unique 和 `status=published` 的 partial unique，并让运行时遇到零条/多条 published 记录高优告警或 fail-closed，而不是静默人格降级。增加发布第二步失败、两个 draft 并发发布、编辑已发布行、版本并发创建与历史回滚集成测试。

## SR-054：领导裁决先提交 resolved 再单独创建 relay task，插入失败会永久吞掉客户转述

- 严重度：P1
- 确定性：FACT（触发需 resolved 更新成功后 task insert 失败或进程崩溃）
- 证据：`src/routes/principal_escalations.rs:82-121`、`src/agent/escalation/mod.rs:284-350`、`src/agent/escalation/ledger.rs:141-172,474-501`、`src/db/indexes.rs:135-179`；正常/重复调用测试见 `tests/ask_human_phase1_e2e.rs:184-303`。
- 机制：Admin 与微信领导回复两条入口都先用 `find_one_and_update` 把 pending 台账提交为 `resolved`，随后才以独立 `insert_one` 创建 `principal_decision_relay` task；两步没有事务、持久 outbox marker 或补偿扫描。若 task insert 失败或进程在两步间崩溃，入口返回错误但裁决已不可逆提交。重试时只查 pending：Admin 返回 `alreadyResolved=true`，微信路径也不再匹配该条，因此不会补建任务。tasks 对 relay 的 `(workspace,short_code)` 也没有唯一键，若第一次 insert 实际成功但调用方只丢失回包，人工补偿反而可产生重复 relay。
- 影响：后台显示“已裁决”，客户却一直停在 awaiting/占位回复状态，真人结论永远不会转述；运营重试看到幂等成功，难以察觉缺失任务。相反，不确定成功后的补单可能触发两次转述。该缺口与 SR-035 的“已有 relay task 执行后不终态”不同，发生在裁决到任务创建的提交边界。
- 建议：在 Mongo transaction 内原子提交裁决与 relay task，或先以 `(workspace_id,short_code,kind)` unique upsert 建 durable relay intent，再 CAS resolved；任何重试都应查询/补齐该 intent，而不是仅看 escalation.status。增加 task validator 拒绝、resolved 后进程崩溃、insert 成功回包丢失与双入口并发测试，并由周期 reconciliation 扫描 `resolved && no relay delivery state` 的异常记录。

## SR-055：手动 Prompt 发布把运行时 active 与版本 current 拆成两套真相，并先物删历史

- 严重度：P1
- 确定性：FACT（单请求正常完成即产生 status/current 分裂；历史丢失与零 current 在目标更新失败时进一步触发）
- 证据：`src/routes/prompt_templates.rs:98-149,240-391`、`src/prompts.rs:454-535`、`src/evolution/release.rs:180-390,540-705`、`src/db/indexes.rs:377-384,1396-1415`；现有回归测试只断言 status 与 evolution 行保留，见 `tests/prompt_publish_evolution_guard.rs:54-123`。
- 机制：create 明确把 draft 写为 `current_version=false`，注释称 publish 会切换 current；实际 publish 却先物理删除同 key 的所有非 evolution 兄弟行，再只把目标 `status` 改为 `active`，既不设 `current_version=true`、不 demote 旧 current，也不记录 `previous_version`。普通运行时 loader 只按 `status=active` 选版本，因此新稿会立即参与生产；Evolution release/rollback 与启动对齐却按 `current_version=true` 找版本，看到的是旧 evolution 行、零行或另一套历史。若 delete 成功而目标 update 失败，旧 current 已永久删除，目标仍 draft。即使成功，非 evolution 历史也被物删，回滚链无法重建。
- 影响：同一个 prompt key 对生产 Reply/管理调用与 Evolution 发布器呈现不同“当前版本”；后续演化可能基于错误旧稿追加、因无 current 拒绝发布，或回滚到与刚手动发布无关的内容。数据库抖动可让有效 prompt 直接消失并回落内置默认，历史物删还使既有 run 难以解释和恢复。该问题与 SR-007 的旧库迁移选错 current 不同，发生在当前在线手动发布路径。
- 建议：统一唯一真相：手动发布与 Evolution 共用事务化的不可变版本切换，插入/选择目标后原子设置唯一 current，旧行只 soft-retire，维护 previous_version，绝不物删历史。运行时 loader 应明确消费同一 current/rollout 语义；异常零/多 current 必须告警。补充 draft 发布后 status+current、目标更新失败、双发布并发、Evolution 接续发布/回滚及历史保留测试。

## SR-056：DomainSchema 按“最新版本”判断删除/更新，且 active 切换可留下零个或多个生产约束

- 严重度：P1
- 确定性：FACT（删除 active 旧版本为确定性路径；零/多 active 需中间失败或并发激活）
- 证据：`src/routes/domain_schemas.rs:195-401,498-576`、`src/knowledge_wiki/chunk_revisions.rs:280-299`、`src/db/indexes.rs:1598-1624`；现有集成测试只模拟顺序成功切换，见 `tests/domain_schema_persistence_e2e.rs:40-100`。
- 机制：同一 schema_id 可有多个版本，但 update/delete/activate 的 URL 只有 schema_id，三者都先按 version 降序取“最新行”。若 v1 正在 active、v2 是未激活草稿，DELETE 只检查 v2.is_active=false，随后 `delete_many` 删除该 id 的所有版本，连生产 v1 一并移除；PUT 则原地改写最新行，若最新恰是 active 就绕过新版本/激活流程直接改变生产约束。activate 又先独立 demote workspace 全部 active，再 promote 目标，没有事务、CAS 或 active partial unique；第二步失败留下零 active，并发激活交错可留下两 active。运行时 `find_one(is_active=true)` 无排序，异常多行时任选。
- 影响：生产 chunk 写入的 required/enum/alias 约束可被一次看似删除“草稿”的操作整体关闭；原地 PUT 可无发布审计立即改变准入规则。失败或并发切换后，零 active 会静默 no-op 放过本应拒绝的数据，多 active 则随机选择 schema，使同类写入时而通过、时而失败或被改写成不同字段。
- 建议：路由使用不可变版本 id（或 schema_id+version），更新始终创建新 draft；删除只删明确版本且若血缘任一 active 必须保护，物删整个血缘需单独确认。用事务/单文档 active pointer/CAS 原子切换，并在清理存量后建立每 workspace 单 active 的 partial unique。loader 遇到多 active 应报不变量错误。增加 active 旧版+新 draft 删除、PUT active、激活第二步失败、双激活交错与运行时异常态测试。

## SR-057：疑似成交在校验联系人和成交参数前先提交 approved，失败后无法重试核实

- 严重度：P1
- 确定性：FACT（触发需 CAS 后的联系人/金额/币种/产品解引用或成交写入失败）
- 证据：`src/routes/admin_suspected_deals.rs:119-216`、`src/routes/shared.rs:1451-1577`、`src/db/indexes.rs:1214-1260`；正常成功测试见 `tests/suspected_deal_e2e.rs:122-197`、`tests/outcome_snapshot_freeze_integration.rs:146-212`，未覆盖 CAS 后失败。
- 机制：approve 为防 append-only 成交双计，先 CAS `pending→approved`，之后才加载 contact，并在 `add_outcome_event_inner` 中校验 amount、currency、product_id/status 和写 contact。任一步失败时 signal 已是 approved；重试因 CAS 只接受 pending 而永久被挡。部分唯一索引又只约束 pending，所以后续 AI 可能另建一条同 contact pending，但原核实动作本身没有 `processing/failed`、补偿或 reconciliation。更晚的审计事件写失败发生在 outcome 已 append 后，也会让请求报错而 signal 保持 approved，调用方无法区分“成交未写”与“成交已写但审计失败”。
- 影响：无效币种、下架/错误产品、联系人刚被删除或瞬时数据库故障都可把待核实事项显示成“已批准”，却没有正式成交；收入/持有投影漏记且运营重试只得到 not pending。反向地，成交已写但尾部审计失败时客户端看到错误，人工补登可能造成双计。代码注释虽接受“漏登优于双计”，但当前状态没有把漏登暴露为可恢复异常。
- 建议：先完成无副作用校验与 contact/product 解析，再以持久 `processing` claim 执行；最佳方案是在副本集事务内原子更新 signal 与 contact outcome。无法事务时应写唯一 outcome intent/idempotency key，使用 `pending→processing→approved|failed`，失败可安全重试并由 reconciliation 扫描；审计失败不得混淆业务提交结果。增加每个校验失败、contact/update/audit 故障、进程崩溃和并发 approve 测试。

## SR-058：审核与生产配置变更缺少可信 actor，可伪造或丢失运营、成交与确认主体

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/admin_taxonomy_candidates.rs:53-81,156-187,397-431`、`src/routes/admin_relationship_suggestions.rs:56-72,126-213,220-252`、`src/routes/admin_suspected_deals.rs:57-81,139-207,220-252`、`src/routes/management.rs:93-214,1693-1704,1975-1997,2325-2370,2430-2438`、`src/routes/evolution.rs:549-622`、`src/models.rs:3246-3271`、`src/routes/guides.rs:165-185,204-298,449-472`；可信身份对照见 `src/routes/media_assets.rs:239`、`src/routes/referral_cards.rs:166`。
- 机制：三组 approve/reject payload 都公开接受 `reviewedBy`，写库时优先使用该客户端字符串而非 `AuthenticatedAdmin.username`；taxonomy/relationship 的核心 inner 甚至只接 workspace 或可选 account，不接真实 admin。疑似成交虽持有 AuthenticatedAdmin，也允许 payload 覆盖，并把同一伪造值继续写进正式 `OutcomeEvent.marked_by` 和 `outcome_event_marked.details.markedBy`。Evolution runtime-flag PUT 同样持有真实 admin，却允许请求体 `updatedBy` 覆盖；正式 UI 不传该值时又固定回落字符串 `admin`。管理 Agent 复用 REST handler 时则把真实 admin 替换为固定 `management-agent`，命令/工具行也没有 actor。Guide 走另一种丢失方式：preview 模型不保存 created_by/confirmed_by，apply 虽提取 `AuthenticatedAdmin`，却只用 current workspace claim；事务内事件仅写 instruction/scope/changes，不写确认 admin，且内核根本不接 admin。于是同 workspace 内谁生成、谁最终点击确认都无法从 preview、终态或事件恢复。
- 影响：有管理权限的操作者可把批准、拒绝、关系画像变更、财务成交和 Evolution 开关归因给任意文本；正常 UI 又可能统一记成 `admin`、`management-agent` 或完全没有 actor。事后审计无法回答“谁实际确认这次生产配置变更”，也会污染 UI 展示、事件导出和责任追踪。认证仍阻止匿名调用，所以这不是权限提升，而是审计主体完整性失效。
- 建议：外部请求移除 reviewedBy/updatedBy，始终从 AuthenticatedAdmin 注入不可伪造的 user_id+username；管理 Agent 委托同时携带真实发起 admin。Guide preview 保存 created_by，apply claim/finalize 与事件保存 confirmed_by，并明确是否允许另一 admin 接管。若需要代他人登记，另存 actor 与 on_behalf_of，并要求显式权限/原因。增加恶意 actor 被忽略、Management 委托主体、Guide 跨 admin 确认及所有业务事件 actor 一致的 HTTP 测试。

## SR-059：关系类型 approve 未用 CAS/事务，审核状态可与已生效联系人画像相反

- 严重度：P1
- 确定性：FACT（单请求中间失败即可分裂；相反终态需 approve/reject 或建议刷新并发）
- 证据：`src/routes/admin_relationship_suggestions.rs:109-252`、`src/agent/gateway.rs:4738-4801`、`src/db/indexes.rs:1184-1211`。
- 机制：approve 先普通读取 pending suggestion，再校验并 `$set` contact.relationship_type，最后用仅 `_id` 的无条件 update 把 suggestion 置 approved；第二步没有 workspace/status CAS，也不检查 matched/modified count，回读同样只按 `_id`。若 contact 写后进程崩溃或 suggestion update 失败，生产画像已变而队列仍 pending。若 reject 在 approve 读取后抢先 CAS 为 rejected，approve 仍会写 contact，并随后无条件把 rejected 覆盖成 approved；Gateway 也可在窗口内刷新 pending 的 suggested_value，使 contact 写入旧值而最终 approved 行展示新值。
- 影响：运营看到的审核结论、证据和值可与 Agent 实际用于运营范式的 contact.relationship_type 不一致；被拒建议也可能在并发下生效并被改回 approved。重试虽常把同值幂等写回，但无法修复“状态与值不同源”或证明哪次人审生效。
- 建议：用 `_id+workspace+status=pending+expected suggested_value/version` 原子 claim，后续更新必须带 claim token；在支持事务的部署中把 contact 与 suggestion 终态同事务提交。否则引入 processing/failed 与 reconciliation，并在写 contact 前后验证 matched_count。拒绝与批准共享同一状态机。增加 approve/reject 竞态、suggestion 刷新、contact 后崩溃和第二步失败测试。

## SR-060：关系类型终态行永久占唯一槽，后续新证据无法重新进入人审

- 严重度：P2
- 确定性：FACT
- 证据：`src/db/indexes.rs:1184-1211`、`src/agent/gateway.rs:4738-4801`、`src/routes/admin_relationship_suggestions.rs:80-107,220-252`。
- 机制：集合对 `(workspace_id,contact_id)` 建全量 unique；Gateway upsert 又只匹配 `status=pending`。一旦行变为 approved 或 rejected，后续同 contact 新建议匹配不到，upsert 尝试 insert 时必撞终态行的 unique，错误被 fail-soft 丢弃。与 suspected deal 已改成仅 pending partial unique 不同，关系类型没有新一轮记录、复开动作或版本历史。
- 影响：一次误拒后，无论后续证据多强都不会再出现待审；已批准的 customer/peer/friend 即使真实关系变化，也无法通过 AI 建议链纠正。后台看不到被吞掉的变化，occurrences/evidence 也停止更新，所谓保守人审闭环实际变成一次性永久裁决。
- 建议：保留终态历史，但唯一约束只覆盖 pending，或以 `(workspace,contact,suggestion_generation)` 版本化并保证至多一条 pending；新建议若与当前 confirmed value 不同应创建新轮次并关联 previous decision。明确冷却/重复证据策略，不能静默吞 E11000。增加 reject 后新证据、approve 后关系变化、同值重复抑制与并发新轮次测试。

## SR-061：Taxonomy 候选“合并到既有 canonical”会标 approved，却没有写入 raw alias

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/admin_taxonomy_candidates.rs:262-445`、`src/agent/taxonomy.rs:205-268,370-488`、`src/agent/decision_taxonomy.rs:37-107`；事务测试只覆盖新 canonical 插入，见 `tests/transactional_admin_flows.rs:153-278`。
- 机制：审批先把 candidate.raw_value 加进待写 entry.aliases；但若同 scope/kind/canonical id 已有 current 行，`duplicate=true`，代码完全跳过 taxonomy insert/update，随后仍把 candidate 提交为 approved，并向 REST 返回 409“已存在；候选已置为 approved”。既有行没有 `$addToSet` raw/请求 aliases，也没有新版本。缓存失效后 raw 仍是 CandidateNew；候选 unique 行却已 approved，后续 upsert 只刷新 last_seen_at+warning，不会重新进入 pending。
- 影响：运营以为未知词已归并，实际每轮仍产生 taxonomy risk、无法 canonical rewrite，并可能继续把原始值写入画像；收件箱又不再显示为待审，形成永久“已解决但未生效”的死记录。它与 SR-046 的 alias 冲突不同：即便没有任何冲突，目标 canonical 已存在这一正常合并场景也必然失败。
- 建议：duplicate canonical 应在同一事务内为 current 条目创建新版本或原子 `$addToSet` 规范化 aliases，并在提交 candidate 前验证 raw 已能被 check_value 映射；alias 冲突按 SR-046 返回 409 且候选保持 pending/failed，不得假 approved。响应成功/冲突语义要与持久状态一致。增加已有 canonical 合并 raw、附加 aliases、冲突回滚和缓存 reload 后实际归一测试。

## SR-062：Management 可把裸 message_send_text 当低风险工具直发，绕过生产发送网关与可靠投递

- 严重度：P1
- 确定性：FACT（触发需 MCP tools/list 公布 `message_send_text` 且管理计划选择该裸工具）
- 证据：`src/routes/management.rs:296-310,1107-1218,1301-1315,2411-2426,2782-2822`、`src/prompts.rs:1645-1685`；受绕过的产品发送链见 `src/routes/management.rs:1580-1600`、`src/agent/gateway.rs:188-609`、`src/agent/outbox.rs:162-350`、`src/agent/outbox_dispatcher.rs:1585-1851`。
- 机制：Management 把 MCP 动态目录原样交给 LLM，并允许任何真实 advertised 工具落入兜底透传。裸 `message_send_text` 被代码显式分为 Low，默认无需确认；policy prompt 还把它列为发送微信文本的优先候选之一。计划选中它后，兜底直接调用 `logged_call_for_account`，不会进入 `wechatagent.send_contact_message` 的 contact 解析、生产 Review/Gateway、Outbox 幂等、二次安全门、账号离线/节奏控制、重试与送达后状态聚合。`assert_tool_outcome` 也只为产品工具名识别 send receipt，裸工具通常只能记为 executed_unverified，不能弥补发送前控制缺失。
- 影响：一个本应由生产发送网关拦截、改写、延迟或可靠排队的客户文本，可由管理计划直接从 MCP 发出；账号离线、客户 stop/cooldown、事实/压迫/隐私 Review 和 Outbox 取消均不起作用。调用成功后进程崩溃或回包不确定时又没有 durable outbox intent，重试命令可能重复发送。该路径仍需已认证管理员发起命令，不是匿名远程发信，但 LLM 工具选择不应获得绕过客户出站不变量的能力。
- 建议：从 Management 动态白名单移除所有客户触达型 raw MCP 工具；`message_send_text` 必须强制重写/代理到 `wechatagent.send_contact_message`，并要求解析到当前 workspace/account 的 contact 后统一走 Gateway + Outbox。领导内部通知等非客户消息应使用独立 typed 通知 Outbox，而不是共享裸透传。增加同一正文经 raw/product 工具名都必须产生 Outbox、Review 拦截、stop/cooldown、离线 defer 和幂等送达的端到端测试。

## SR-063：Dangerous 工具的代码确认门默认关闭，安全性退化为信任 LLM 自报风险

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/management.rs:339-360,1107-1218,1288-1315,2650-2670,2720-2794`、`src/prompts.rs:1645-1685`。
- 机制：后端已有 `ToolRisk::Dangerous` 静态表，覆盖产品发送、发布 Prompt、激活 provider、发布/回滚生产配置等动作；但 `post_management_message` 固定以 `plan_requires_confirmation(..., false)` 调用，测试还明确锁定“Dangerous 默认不确认”。因此除 irreversible、verify、campaign 和未分类工具外，静态 Dangerous 本身不起门控作用。是否进入 pending_confirmation 只剩 `plan.requires_confirmation` 或 `plan.risk_level == dangerous`，两者均来自 LLM 输出；而默认 policy 把“发送消息”定义为 act，不要求 dangerous。一个模型误判、提示注入或 plan 字段缺失就会让高影响工具立即执行。
- 影响：运营可能以为前端的待确认流程保护所有高风险动作，实际发送客户消息、切生产 provider、改全局 Prompt/状态机和多类 rollout/rollback 可在首次自然语言请求中直接生效。静态风险表营造了硬门假象，却只对少数特例生效；LLM 同时负责语义规划和自己的安全裁定，违背代码注释所称“安全裁定交代码”。
- 建议：删除 phase-one bypass，`Dangerous|Irreversible` 一律由代码强制 pending_confirmation；若确需免确认，应是按 actor/工具/作用域显式授权、短时有效且审计化的 capability，而不是全局 false。确认页展示冻结的规范化参数、对象数量与副作用摘要，confirm 时校验计划 hash/version。测试逐个枚举风险表，证明所有 Dangerous 在 LLM 自报 false/read/act 时仍被硬门拦截。

## SR-064：Management 命令与工具副作用没有可恢复提交协议，崩溃后会永久 running 或不确定重放

- 严重度：P1
- 确定性：FACT（触发需工具执行或审计写回之间进程崩溃/数据库错误/网络错误）
- 证据：`src/routes/management.rs:85-214,320-426,439-514`、`src/models.rs:3300-3337`、`src/db/indexes.rs:624-647`；现有测试仅覆盖手工插入状态与 confirm filter，见 `tests/dry_run_isolation.rs:28-183`、`src/routes/management.rs:3073-3089`。
- 机制：普通命令先插入 `AgentCommandRun(status=running)`，每个工具再插入 `AgentToolCall(status=running)`，随后执行任意外部/业务副作用，最后分别普通 update 终态。confirm 更先把 pending_confirmation 原子改 running，之后才重新拉工具目录和执行。任何 `?` 错误或进程退出都会中断尾部写回；系统没有 management worker、lease、attempt、resume cursor、idempotency intent 或 reconciliation 扫描。重试原 HTTP 请求会创建新 command；重试 confirm 又因旧命令已不再 pending 而被拒绝。即使副作用已成功，tool-call 终态写失败也只留下 running，无法判断可否安全重放；多工具计划更没有“已提交到第几个”的 durable cursor。
- 影响：命令中心可永久显示 running，操作者无法区分“未执行、执行中、已执行但审计失败”。人工重发可能重复发消息、重复 append 成交/任务或再次切配置；不重发则可能漏掉后续工具。确认动作尤其会在工具目录加载失败时把命令锁死，既不能再次确认也没有自动恢复，审计状态与真实业务事实分裂。
- 建议：把每个 tool call 变成持久 intent 状态机：`pending→claimed(owner,generation)→executing→succeeded|failed|unverified`，副作用前建立稳定 idempotency key，提交与重试均按 generation CAS；command 保存 next index 并由 worker/reconciler 驱动，HTTP 只创建/确认 intent。外部发送统一 Outbox，append 型写入带 command/tool id unique。对旧 running 增加超时扫描但不得盲重放；先按工具类型核对真实结果。增加副作用前/后、tool audit 前/后、confirm claim 后和多工具中途崩溃测试。

## SR-065：Management 的 LLM 可无确认写入 staff_confirmed 成交，突破“AI 永不自证成交”边界

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/management.rs:743-753,1148-1156,1986-1997,2430-2438`、`src/routes/contacts.rs:72-103,1601-1638`、`src/routes/shared.rs:1405-1416,1451-1574`、`src/agent/entitlements.rs:3-17,50-69`。
- 机制：产品工具目录向规划 LLM 暴露 `wechatagent.write_deal_events`，参数中的 verification 可选；风险表把它定为 Low，默认不需确认。执行时直接复用 admin 手工成交 handler，并以合成的 `management-agent` 身份调用。该 handler 的 verification 缺省/空值会被 `validate_deal_verification` 解释成 `staff_confirmed`，随后 append 到正式 `outcome_events`。下游 entitlement、客户阶段与交易事实注入把 staff_confirmed 当客观购买事实。这里的“人工”只有最初自然语言会话已认证，具体 contact、金额、产品、事件方向和 verification 均是 LLM 生成，后端没有要求确认冻结后的结构化事实。
- 影响：模型选错联系人/产品、误解“疑似成交”或漏填 verification，即可制造高可信正式成交，进而改变持有投影、售后/复购运营、活动圈选、收入统计和后续客户回复。记录又标为 manual/staff_confirmed/management-agent，掩盖了它实际由 LLM 参数化生成，违背代码中“conversation_inferred 只能经运营核实”的红线。
- 建议：Management 不得直接调用 admin 直登成交；默认只能创建 `suspected_deal_signal` 或成交草稿。正式 append 必须进入独立结构化确认页，显示 contact、产品、金额、方向、证据与 verification，并由真实 admin 确认后以其 user_id/username 提交；`payment_verified` 只能来自受信支付回调。若保留命令式录入，至少把工具列为 always-confirm、强制 verification 显式值并保存 actor + planner + confirmer 三方身份。测试证明 LLM 直接调用、缺 verification 和“疑似”措辞都不能产生正式 outcome。

## SR-066：取消 in_flight Outbox 不能撤销在途发送，worker 仍会发出并执行送达副作用

- 严重度：P1
- 确定性：FACT（触发需取消发生在 worker 通过二次门之后、MCP send 返回之前）
- 证据：`src/routes/admin_outbox.rs:116-219`、`src/agent/outbox.rs:559-660`、`src/agent/outbox_dispatcher.rs:1585-1851`；现有集成测试只覆盖发送前批量取消或顺序终态，见 `tests/outbox_integration.rs:600-727`。
- 机制：admin 与用户 reaction 都允许把 `in_flight` 直接改成 canceled，并清除 worker_id/locked_until，但没有取消令牌、generation 或与运行中 future 的通信。worker 在 `process_entry` 开头只做一次二次安全门，随后用已读快照继续 await MCP。发送成功后，它以 `_id+status=in_flight` 尝试标 sent；取消已把状态改走，所以 update 匹配 0，但代码不检查 matched_count，仍写 `outbox_sent`、刷新 run 聚合、执行 delivery finalizer 和 send ledger。timeout post-hoc 命中也有同样问题。
- 影响：后台明确返回“已取消”的消息仍可物理触达客户；数据库主行保持 canceled，却同时出现 sent 事件、客户消息、承诺/follow-up、review/task sent 等送达副作用，形成互相矛盾的事实。运营会把 canceled 当作未发送，可能另发替代文本；用户 stop 后也可能收到已在途消息。对不可撤回的远端发送，当前 API 没有表达“取消请求太晚/结果未知”。
- 建议：claim 写不可复用 generation/worker token，所有发送后提交与 finalizer 必须匹配 token 并检查 matched_count；取消 pending 可直接成功，取消 in_flight 应先写 `cancel_requested` 并通知本进程 cancellation token，在真正调用 MCP 前再 CAS 检查。进入不可取消的远端调用后，API 应返回/保持 `cancel_pending_or_delivery_unknown`，待送达核对后收敛为 sent 或 canceled，不能宣称即时取消。增加 barrier 控制的并发测试：二次门前取消零发送、MCP 调用前取消零发送、调用中取消按真实回执收敛且副作用只执行一次。

## SR-067：“统一收件箱”遗漏 suspected_deal，正式成交核实仍是独立孤岛

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/ask_human_inbox.rs:1-43,491-603`、`src/routes/admin_suspected_deals.rs:84-252`、`src/routes/mod.rs:894-905,943-945`、`frontend/src/app/channels.ts:152-161`；统一收件箱设计仍固定八源，见 `docs/superpowers/specs/2026-06-21-ask-human-inbox-frontend-design.md:7-29`。
- 机制：系统已有 authenticated、workspace-scoped 的 suspected deal pending 列表及 approve/reject 路由，但统一 inbox 的 collector、source filter、summary 和前端 source union 只覆盖 principal、knowledge、taxonomy、relationship、gap、profile、evolution、lessons 八类，没有 suspected_deal。频道文案却宣称“所有需要决策/审核的事项收口在此”。因此最接近财务真相、且代码明确要求运营核实后才能成为 staff_confirmed 的队列没有进入 canonical 待办面。
- 影响：只依赖统一收件箱的运营看不到疑似成交，pending 可长期无人核实；真实成交不会进入 outcome/entitlement/收入与售后运营，误报也不会被及时拒绝。独立 API/旧页面可能仍可访问，所以不是数据丢失，但产品承诺和实际审核覆盖不一致，且会放大 SR-065 直接写正式成交的诱因。
- 建议：把 `suspected_deal` 作为一等 source 加入 inbox collector、summary、前端类型/筛选/计数和 inline approve/reject；卡片展示原始证据、contact、产品候选和时间，确认动作复用修复后的 SR-057 事务状态机及 SR-058 真实 actor。增加“独立列表 pending 集合与统一 inbox/summary 同 workspace 计数一致”的契约测试。

## SR-068：Ask-Human summary 把所有数据库错误伪装成零待办

- 严重度：P2
- 确定性：FACT（触发需任一计数查询失败）
- 证据：`src/routes/ask_human_inbox.rs:497-603`、`frontend/src/features/ask-human/index.tsx`、`frontend/src/stores/inboxStore.ts`。
- 机制：inbox 列表对每个 source 的失败会保留其它结果并返回 `errors[]`；相邻 summary 却对八个 `count_documents` 全部 `.await.unwrap_or(0)`，没有错误字段、日志或 stale 标记。任一集合权限、网络、反序列化或 Mongo 故障都与“确实没有待办”返回相同数字 0；多项查询还是顺序执行，故局部故障可制造局部假清零而 HTTP 仍为 200。
- 影响：数据库异常时仪表盘显示安全的空队列，运营不会进入列表排查，SLA/告警也可能把待审积压误判为已清空。列表即使返回 source error，summary 的零徽标仍与之冲突；若前端只突出总数，真实高风险请示或待核验知识会被隐藏。
- 建议：summary 复用 inbox 的 per-source 结果类型，返回 `{count,status,error,updatedAt}` 或至少顶层 `errors[]`；查询失败用 null/unknown，不得用 0。可并行计数并保留上次成功快照，UI 明确显示“计数不可用”。增加每个 source 单独失败、部分成功、全失败以及 summary/list 一致性的 handler 与前端测试。

## SR-069：Playbook 生成与优化的前后端契约漂移，正式 UI 两条路径确定性失败

- 严重度：P1
- 确定性：FACT
- 证据：`frontend/src/stores/userOpsStore.ts:844-988`、`frontend/src/features/user-ops/legacy.tsx:1037-1217`、`src/routes/playbooks.rs:50-61,236-244,314-331`；B08C 的 Store 专项测试没有 generate/optimize 契约断言，见 `frontend/src/__tests__/stores/userOpsStore.test.ts:1-237`。
- 机制：前端生成请求发送 `{accountId,prompt}`，后端 `GeneratePlaybookRequest` 却要求 `{accountId,description}`；前端优化请求发送 `{prompt}`，后端 `OptimizePlaybookRequest` 要求 `{instruction}`，两者都会在进入 handler 前被 serde 以 422 拒绝。即使只修正生成请求字段，后端成功响应也只有 `{id}`，前端却继续按 `{item}` 解引用并读取 `data.item.name`。这不是可选字段兼容问题，而是请求和响应两端同时漂移。
- 影响：管理端通过正式 UI 生成或优化运营方法论均不可用；生成即使绕过第一处错误，也会在成功响应处理阶段崩溃或写入无效状态。前端 mock/单元测试没有以真实 Rust DTO 和响应做契约验证，因此当前测试无法发现该断裂。
- 建议：为 Playbook API 建立单一 schema 来源或后端导出的生成类型；明确生成结果究竟返回新资源还是仅返回 id，并在前后端统一。增加驱动真实 Axum handler 的契约测试，至少覆盖 generate/optimize 的请求反序列化、成功响应和前端 store 消费，禁止仅用手写 mock 复制错误契约。

## SR-070：Playbook 的“AI 候选/草稿”会直接改写生产方法论，版本号不提供冻结与回滚

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/playbooks.rs:92-197,275-422`、`src/agent/decision.rs:1035-1073`、`src/routes/contacts.rs:594-620`、`src/routes/management.rs:789-802,1157-1162,2096-2141`。
- 机制：账号不存在 Playbook 时，generate 将 AI 结果直接插成 `is_default=true`；optimize 和普通 update 都原地覆盖目标文档并递增 `version`，没有 draft/published 状态、不可变版本行或发布动作。运行时按联系人保存的 playbook `_id` 或账号 default 直接读取同一文档；联系人虽保存 `playbook_version`，loader 并不按该版本取冻结快照。Management 的描述把 edit/generate/optimize 称为“草稿、不放量”，风险表也归 Low，但后端实现不存在这层隔离。
- 影响：一次 AI 生成/优化或低风险 Management 工具调用可立即改变默认账号及所有按 `_id` 绑定联系人的生产决策方法论，无需人审。原内容被覆盖后，旧 `version` 只是数字而非可读取历史，无法证明某次回复使用了什么内容，也不能可靠回滚。
- 建议：把 Playbook 改为不可变版本实体加显式 active pointer：AI 只创建 draft，结构化 diff 经真实 actor 确认后再以事务/CAS 发布；联系人绑定应明确选择“跟随 active”或冻结到不可变 version id。Management 生成/优化必须与 REST/UI 共用同一发布协议并至少列为 always-confirm。迁移前保存现有内容快照，并增加“生成/优化不改变运行时、发布后才切换、旧绑定可重放与回滚”测试。

## SR-071：Playbook 默认指针由无事务多步写维护，零默认与多默认都是合法持久状态

- 严重度：P1
- 确定性：FACT（并发触发需同账号 create/generate/ensure/set-default 交错；单请求中间写失败即可留下零默认）
- 证据：`src/routes/playbooks.rs:92-146,200-234,275-317,470-528`、`src/db/indexes.rs:367-376`、`src/agent/decision.rs:1035-1073`。
- 机制：set-default 先 `update_many` 清空同账号所有 default，再普通 update 目标；create/generate 的“首项即默认”是先查存在性再插入；`ensure_default_playbook` 查不到 default 就直接插入。上述流程都没有事务、CAS 或按 `(workspace,account,is_default=true)` 的 partial unique 约束，现有索引只是普通查询索引。失败可停在零默认，并发检查后插入可形成多默认；运行时遇到多行只按 `updated_at` 取最新，遇到零行则返回 None 或由其它路径另建默认。
- 影响：同一账号实际使用的方法论取决于请求交错、更新时间与哪条路径先补默认，而不是一个受数据库保证的业务指针。管理端可能显示已切换，运行时却无默认或消费另一条；重复系统默认还会继续放大后续更新与删除的不确定性。
- 建议：不要用多行布尔标志模拟指针；优先在账号/独立 pointer 文档上保存 active playbook version id，并以 CAS 原子替换。若保留布尔字段，先清理异常数据，再建立与真实 scope 一致的 partial unique，并在事务内切换。读侧遇到零/多默认应报不变量错误而非静默任选或自动制造；增加每个写步骤故障和并发 create/generate/ensure/set-default 测试。

## SR-072：DomainProfile 激活跨画像、状态机、Policy 与联系人迁移半提交，却始终返回成功

- 严重度：P1
- 确定性：FACT（具体分裂形态取决于画像是否携带状态机及后续写是否失败）
- 证据：`src/routes/domain_profiles.rs:478-642`、`src/routes/admin_ops_versions.rs:44-175,187-338,450-465`、`src/agent/domain_profile.rs:1045-1086`。
- 机制：activate 先把目标画像设为 active 并清其它 active，随后才尝试发布画像携带的 `generated_state_machine`、派生 state policy 和迁移联系人状态。状态机非法、找不到 current domain config、发布失败及逐联系人迁移失败都只记 warn 并继续，最终固定返回 `{ok:true}`。状态机发布内部又把 policy reconcile 作为 best-effort；Gateway 在 policy 缺失时 fail-open。画像 cache 则已被失效并会立即加载新人格/阈值，或在 active/current 异常时静默回落 DEFAULT。
- 影响：一次“激活成功”可只切换人格、阈值和交易模式，却继续运行旧状态机；也可发布新状态机但缺少对应 policy，使动作保护门放开；存量联系人还可能部分迁移、部分停留在旧状态。API/前端没有表达 partial success，运营无法区分完整发布与危险半提交。SR-043 描述的是单集合 current/active 不变量，本条是跨画像、状态机、policy、联系人四类事实的业务提交裂缝。
- 建议：先定义可验证的 activation bundle（profile version、state-machine version、derived policy version、迁移策略），预校验全部内容后创建 durable rollout intent；配置指针在事务/CAS 中一次切换，联系人迁移作为可恢复 job 带 generation、游标和完成/失败统计。任何必要 policy 生成失败都应 fail-closed，不得发布状态机；接口应返回真实 rollout 状态而非提前成功。增加每一步故障注入、部分联系人失败、重试恢复和运行时版本一致性测试。

## SR-073：DomainProfile 高风险二次确认不绑定冻结内容，确认后可替换正文或直接绕过 publish

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/domain_profiles.rs:196-254,281-347,380-418`、`frontend/src/components/review/ProfilePublishCard.tsx:56-115`、`frontend/src/stores/strategyStore.ts:399-488`。
- 机制：高风险 publish 仅克隆一条 `current_version=false,is_active=false` 的旁路行并返回新 `id` 与风险字段名；没有生成绑定内容 hash/version、actor、TTL 的确认 capability。update 只禁止编辑 active 行，并未按注释限制为特定 draft 状态，因此第一次确认后仍可原地改写这条旁路稿。rollout 只凭 `_id+workspace` 选行，不校验它是否由本次 risky publish 产生、源版本/内容是否仍相同或调用者是否拥有确认权；任意历史/旁路行都可直接提交。正式 `ProfilePublishCard` 还有一个确定性接线错误：响应类型丢弃新 `id`，二次确认后仍以原始草稿 `profileId` 调 rollout，而不是刚发布的旁路稿 id。原草稿继续是 `current_version=false`，rollout 会把它重新推成 current，刚克隆且等待确认的新版本反而留在旁路；随后 UI 仍 toast“已发布”。Store 内未被该卡使用的另一套 action 才正确保留 `resp.id`。
- 影响：运营确认的内容与最终生效内容之间没有密码学或数据库绑定；确认窗口内正文可被另一次编辑替换，也可跳过 publish 直接对任意行调用 rollout。更直接地，从正式系统策略页走高风险发布并点击“确认生效”时，确认的克隆版本确定不会被这次 rollout 选中，页面却报告成功，生产可能回到原草稿内容而非确认候选。Management 的 rollout 还受 SR-063“Dangerous 默认不确认”影响，使后端缺口不只是假设中的恶意前端问题。
- 建议：第一次确认应创建不可变 candidate 和一次性 confirmation intent，保存规范化内容 hash、source/version、风险 diff、actor 与过期时间；第二次确认必须以真实 admin CAS 消费 intent，hash 或版本变化即失效。rollout 不应接受任意 profile id，只接受有效 intent/candidate；确认页展示完整 diff 和运行影响。最小止血还必须让卡片读取并提交 publish 返回的新 `id`，并以读回的 current/active id 验证成功后再 toast。增加确认后编辑、复用 token、过期、跨 actor、绕过 publish、返回 id 与 rollout id 一致、旧草稿不得被重新 promote 及 Management 调用测试。

## SR-074：Operation Domain reset 先物理删除全部历史再插默认，失败会留下零配置且不可恢复

- 严重度：P1
- 确定性：FACT（数据丢失为顺序必然；零配置需 insert 失败）
- 证据：`src/routes/domains.rs:89-201,243-270`、`frontend/src/stores/userOpsStore.ts:1036-1047`、`src/routes/admin_ops_versions.rs:187-338`。
- 机制：reset 对 workspace/domain 先 `delete_many` 删除全部 Operation Domain 版本，再以独立 insert 写一条默认配置；没有事务、备份、软删除、恢复 intent 或失败补偿。insert 失败时历史已经永久消失并留下零 current。相邻的 domain/state-machine 直接 update 还在原地修改 current，policy reconcile 失败只记录告警并仍返回成功；两条 update 也不检查 `matched_count`，不存在的目标可能得到 `{ok:true}`。正式前端直接暴露 reset 调用。
- 影响：一次普通重置会无条件销毁审计与回滚历史；数据库瞬时错误、schema/unique 错误或进程退出可让该 domain 完全无配置，运行时只能回落默认或进入与管理面不一致的状态。即使 insert 成功，也无法恢复重置前的生产配置或解释历史回复所用规则。
- 建议：reset 应创建一个新的不可变默认版本并通过与 SR-008 同一套原子 active pointer 发布，旧版本保留为 previous/history；禁止 delete-before-insert。直接编辑 current 也应改成 draft→validate→publish，policy 作为同一 bundle 的必需产物。所有 update 检查 matched_count/version CAS，并增加 insert/policy 中断、并发 reset/update、审计回滚和运行时回退测试。

## SR-075：Campaign “预览”会写状态并重开 completed 活动，读工具和 dry-run 都可产生生产副作用

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/campaigns.rs:265-316,319-371`、`src/routes/management.rs:879-884,1110-1127,1292-1324,2383-2409`、`frontend/src/features/campaign/CampaignCreate.tsx:39-59,120-142`。
- 机制：`preview_campaign` 并非只读：它圈人后无条件把指定活动写成 `status=previewed` 并覆盖 `targetCount`，且不校验原状态。于是已 `completed` 的活动可先调用 preview 退回 previewed，再通过 dispatch 的状态门；原有 campaign_sends 只阻止既有联系人重复，对后来新命中的联系人仍会创建新任务。Management 又把 `wechatagent.preview_campaign` 静态分类为 Readonly，dry-run 对 read 工具不跳过；该工具实际执行 `create_campaign` 再 preview，至少新增并修改一条正式活动。
- 影响：`completed` 不是终态，原本“完成后不可扩张受众”的高风险确认边界可被普通 authenticated REST 调用绕过。管理命令的 dry-run 也会积累真实 draft/previewed 活动，审计所称只读与数据库事实相反；后续误选这些活动可能触发真实触达。
- 建议：把纯圈人计算做成真正无写的 preview；创建/修改草稿使用独立写端点并列为 Low。活动进入 dispatching/completed 后内容与 audience spec 冻结，任何重跑必须复制成新 campaign 并重新确认。Management dry-run 应以事务回滚或隔离存储保证零业务写，而不是信任风险标签。增加 completed→preview 拒绝、dry-run 集合快照和新联系人不得扩张旧活动的测试。

## SR-076：Campaign 扇出把去重台账、可执行 Task 与活动终态拆成多步写，崩溃和并发会丢关联或重复触达

- 严重度：P1
- 确定性：FACT（具体端态需进程退出、补偿写失败、worker 抢占或并发 dispatch 交错）
- 证据：`src/routes/campaigns.rs:326-460,572-686`、`src/tasks.rs:161-239`、`src/db/indexes.rs:875-904`、`tests/campaign_dispatch_integration.rs:143-283,377-459`。
- 机制：dispatch 先无 CAS 地写 `dispatching`，随后每个联系人依次 insert CampaignSend、insert 立即可被 worker claim 的 pending AgentTask、回填 taskId，最后另写 campaign=completed。代码只补偿正常返回的 DB Err，补偿删除本身又是 best-effort；没有事务、durable fanout intent、generation、reconciler 或 task 上的 campaign id。进程在 send 后退出会留下 `taskId=None` 的永久去重占位；在 task insert 后退出会让任务真实执行但报表永远无法关联。回填失败时删除 task 也不能撤销已被 worker claim 的 future，删除 send 后重试可再建第二个任务。两个 dispatch 还能同时通过旧状态检查并瓜分 unique insert，最后各自用局部 `dispatched` 覆盖总数。
- 影响：同一活动可表现为永久 pending/假 completed/下发数偏小，真实触达却已发生；人工重试既可能漏掉被孤儿去重位封死的联系人，也可能对已在途联系人再次生成和发送。现有故障测试只注入可返回的 task insert 错误，不覆盖进程崩溃、补偿失败、claim 竞态或双 dispatch。
- 建议：创建持久 CampaignFanout intent 和逐联系人 item 状态机；在事务内冻结 campaign spec、生成 item 并提交待派发状态，由 worker 按 item id 幂等创建/关联 task。task 必须在可 claim 前已绑定 campaign item，所有推进使用 generation CAS；reconciler 收敛 orphan/processing 项，campaign 状态由 item 聚合而不是 HTTP 循环尾部覆盖。增加每个写点 crash、补偿失败、worker barrier 和双 dispatch 测试。

## SR-077：Campaign 前端复用 draft 时不保存已编辑条件，预览和最终推送继续使用第一次创建的受众与意图

- 严重度：P1
- 确定性：FACT
- 证据：`frontend/src/features/campaign/CampaignCreate.tsx:18-62,75-145`、`src/routes/campaigns.rs:124-135,227-316,350-357`、`frontend/src/__tests__/features/campaign/create.test.tsx:28-60`。
- 机制：首次预览先 POST 创建并把表单的 title、intentText、segmentFilter 存入 campaign。之后编辑圈选条件只清空本地 preview，却保留 `draftCampaignId`；再次点击仅 POST `/campaigns/:id/preview` 空 body，后端仍读取第一次落库的 segment_filter。系统没有对应的 draft update 调用。title/intentText 的 onChange 甚至不清空已有 preview，因此页面可继续展示与当前输入不对应的旧命中结果。dispatch 重新圈人时同样只消费数据库里的第一次内容。
- 影响：运营以为已经缩小受众、改产品/阶段或修正活动话术，实际预览和高风险确认仍基于旧条件，最终可向错误客户发送旧意图。现有前端测试反而锁定“改条件后复用同一 draft 只调 preview”，但没有以真实后端断言条件已更新。
- 建议：提供带 version/CAS 的 draft update，并在每次 preview 前先保存完整规范化 spec；更稳妥的是 preview 接收不可变 candidate spec 并返回 content hash，dispatch 只接受经确认的 hash/version。任一表单字段变化都使预览与确认失效。增加真实前后端契约测试，证明第二次条件/意图会改变数据库、命中集合和最终冻结内容。

## SR-078：自治指标由多次非快照计数拼接，活跃流量下可返回超过 100% 或互相矛盾的比率

- 严重度：P2
- 确定性：FACT（异常数值需统计期间有新 run/outbox 写入或状态推进）
- 证据：`src/routes/outcomes_autonomy.rs:219-440`。
- 机制：接口先后执行 17 次独立 `count_documents`，再执行 planner aggregation，没有 session snapshot 或共同 cutoff。`totalRuns` 最先计算，revision、hold、autonomy 分子随后计算；outbox total 也早于 sent/canceled/failed 分子。新行或状态变化可落在任意两个查询之间，因此后算的分子可以包含先算分母时尚不存在的行，三个 autonomy 占比之和也不再对应同一个样本集合。
- 影响：监控可展示 revisionTriggerRate/sendSuccessRate >100%，或 auto+assisted+blocked 不等于总样本；同一次响应中的 rawCounts 无法作为可复核快照。运营和发布判断可能把采集竞态误认为模型退化或改善，测试在静止小数据集上不会暴露该问题。
- 建议：请求开始时固定 `asOf`，所有集合至少统一使用 `created_at <= asOf`；同集合指标用单个 `$facet`/`$group` aggregation 在一次快照读取中计算。若跨集合必须强一致，使用支持 snapshot read concern 的 session，或明确返回各集合独立 asOf。服务端验证分子≤分母及分布约束，违反时返回 unknown/告警而非发布不可能比率。

## SR-079：自治监控的查询形状与索引声明不匹配，并以串行计数和逐行联系人查询放大延迟

- 严重度：P2
- 确定性：FACT（实际延迟取决于数据量、同 account_id 的跨 workspace 分布与 Mongo 计划）
- 证据：`src/routes/outcomes_autonomy.rs:92-119,219-391,443-523`、`src/db/indexes.rs:518-575,910-957`。
- 机制：注释声称 100k runs 下所有过滤命中 `(account_id,final_review_status,created_at)` / `(account_id,autonomy_mode,created_at)`，但真实 filter 还以 workspace 开头，而这两组索引没有 workspace；部分 revision_applied/selfCritique 查询也没有对应复合后缀。outbox 统计按 workspace+account+created_at 查询，现有索引却是 account+status+next_retry_at/sent_at 等，没有该 horizon 形状。handler 仍串行执行 17 次 count；revisions 查询按 workspace+account+revision_applied+created_at 排序也无对应索引，并为最多 200 行逐条查一次 contact，形成 N+1。
- 影响：监控刷新随日志和租户增长产生大量重复索引扫描、残余过滤、排序及网络往返，可能拖慢 API 和主 Mongo；同 account_id 在多个 workspace 复用时，缺 workspace 前缀还会扫描其它租户数据。现有集成测试验证数值，不做 explain、查询次数或规模延迟门。
- 建议：先用生产形状 explain 建立基线，再以 workspace+account+created_at/status/autonomy/revision_applied 的真实选择性设计最少索引；outbox 增 horizon 统计索引。用单个 `$facet` 聚合替代串行 counts，revisions 批量 `$in` 拉 contacts 或 `$lookup`。加入 100k/多 workspace 数据集的 examinedKeys/docs、query-count 和 P95 预算测试，删除无法由 explain 证明的性能注释。

## SR-080：单联系人启用按裸 account_id 校验账号，跨 workspace 同名账号可替错误租户通过

- 严重度：P1
- 确定性：FACT（触发需不同 workspace 复用同一 account_id，或当前 workspace 的账号缺失而其它 workspace 存在同名账号）
- 证据：`src/routes/contacts.rs:963-1011`、`src/routes/shared.rs:141-160`、`src/db/indexes.rs:58-84`；同文件其它正确调用见 `src/routes/contacts.rs:299-348,512-518,761-796`。
- 机制：`enable_agent` 先按 contact id + current workspace 正确取得联系人，随后验证其账号是否注册时却只执行 `accounts.find_one({account_id})`，未带 `workspace_id`。账号真实唯一键是 `(workspace_id,account_id)`，且共享 helper `validate_account` 明确要求两个维度；批量启用、联系人搜索/导入和 roster 都使用正确复合过滤。若当前 workspace 没有该账号，但另一 workspace 有同名 account_id，单联系人启用仍会把本租户 contact 改成 managed，并拿另一租户账号行的 wxid 做“不能运营自己”判断。
- 影响：接口可为一个无法被当前 workspace Webhook/MCP 正确服务的联系人开启生产 Agent，形成管理面显示 managed、真实入站无法路由或后续凭证落入 SR-011 跨租户 fallback 的状态；错误账号 wxid 还会让 self-account 防线误放行或误拒绝。该问题不直接读取另一租户账号正文，但破坏租户归属校验并放大错误凭证风险。
- 建议：删除局部裸查询，统一调用 `validate_account(state,current_workspace,contact.account_id)`；若需要 wxid，再以同一复合 filter 读取账号。把 workspace/account 封装为不可拆 scope 类型，禁止仅凭 account_id 加载租户实体。增加两个 workspace 同 account_id、当前租户缺账号/有账号及 self wxid 不同的集成测试。

## SR-081：联系人导入用缺失字段覆盖既有身份，重复导入可把昵称、备注和 alias 清成 null

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/contacts.rs:333-427`、`src/routes/shared.rs:187-255`、`src/routes/management.rs:1363-1383`；正确的“仅有值才写”对照见 `src/routes/contacts.rs:847-871`。
- 机制：REST import、deprecated search-import 与 Management import 都复用 `upsert_contact_from_value`。helper 把缺失的 nickname/remark/alias 解析为 `None`，随后无条件放进 `$set`；BSON 将其写为 null。于是任何字段不完整的搜索候选或人工传入 candidate 在重跑时都会覆盖已经由 roster、Webhook 或运营积累的非空身份字段。相邻 batch-enable 已专门按 `Some` 条件构造 `$set`，说明缺失字段应表达“不修改”而非“清空”。helper 也未在写前执行 `is_operatable_person`，但这些导入默认只置 normal，当前主要确定性损害是身份数据覆盖。
- 影响：联系人列表、搜索、称呼与运营判断会在一次看似幂等的导入后退化为空；后续读时 roster 富化只补 API 响应，不修复数据库，且 remark/alias 没有同等兜底。由于 Management 将 import 归为 Low，可由 AI 自动触发，覆盖面不只限手工 REST。
- 建议：按字段存在性构造 `$set`，只有显式非 null 值才更新；若产品需要清空，提供独立显式 clear 字段/操作。导入前统一校验真人、自账号与 candidate shape，并返回 skipped 原因。增加“完整记录→缺字段重复导入保持不变”“显式更新单字段不清其它字段”及 Management 同路径测试。

## SR-082：批量启用先提交 managed 再创建画像任务，插入失败会留下永久无画像的已托管联系人

- 严重度：P1
- 确定性：FACT（半提交需 task insert 返回错误或进程在两步之间退出；并发重复任务可由双请求交错触发）
- 证据：`src/routes/contacts.rs:761-960`、`src/db/indexes.rs:133-180`、`src/tasks.rs:161-239`、`tests/contacts_batch_enable.rs:115-225`。
- 机制：`batch_enable_endpoint` 对每个候选先普通 upsert contact，把 `agent_status` 设为 managed，并写 note/playbook/初始状态；随后才独立 insert `initial_profile` task。两步没有事务、durable onboarding intent 或失败补偿。task insert 失败时 handler 返回 Err，但联系人已可被 Webhook/Gateway 当作 managed；重试又先读到 already_managed=true，因而明确跳过入队，永久失去自动画像任务。反向窗口中 task insert 成功而 HTTP 在返回前失败，两个并发请求也都可能先读到非 managed；`initial_profile` 没有 active partial unique，重试/并发可创建多条画像任务。
- 影响：同一批请求可产生一部分完整纳管、一部分“managed 但永远无初始画像”的半提交联系人，API 只返回整体错误且没有逐项恢复信息。重复任务会并行消耗多轮 LLM，并以晚到结果覆盖早先画像；联系人又在任务创建前已进入生产回复路径，运营无法从 managed 状态判断 onboarding 是否真正完成。
- 建议：把纳管建模为持久 enrollment/onboarding intent：事务内创建/更新联系人 generation 与唯一 active `initial_profile` item，联系人仅在必要初始化已持久化后进入可服务状态；或至少事务化 contact+task，并对 `(workspace,account,contact,kind,generation)` 建唯一约束。worker 按 generation CAS 提交，reconciler 补齐 managed 但无完成 item 的异常。接口返回逐项 accepted/already/failed 状态；增加 task insert 故障、写后崩溃、双请求与重试恢复测试。

## SR-083：在途初始画像任务可把刚禁用或移出池的联系人重新改回 managed

- 严重度：P1
- 确定性：FACT（触发需 disable/hide 发生在 worker 首次检查 managed 之后、画像回写之前）
- 证据：`src/routes/contacts.rs:594-661,708-758,1048-1124`、`src/tasks.rs:188-239`、`tests/contacts_batch_enable.rs:392-494`；通用 task fencing 缺口见 SR-034。
- 机制：`handle_initial_profile_task` 载入 contact 后只在耗时 LLM 生成前检查一次 `agent_status==managed`。之后 `apply_generated_profile_to_contact` 按裸 `_id` 无条件更新，并固定 `$set agent_status=managed`。若管理员在 LLM 运行期间调用 disable 或 hide-from-pool，先写入的 normal 会被晚到画像结果覆盖回 managed；hide 标志虽保留且列表隐藏，但生产回复门明确只看 agent_status，因此隐藏联系人也会重新进入 AI 运营。任务没有 contact enrollment generation，终态写也没有 current claim fencing；现有测试只覆盖任务开始前已经 normal/contact gone 的早退。
- 影响：管理员看到“停止运营/从池移除”成功后，联系人仍可能被后台任务静默复活并继续自动回复或主动触达。隐藏行不再出现在常规列表，反而更难发现；审计同时保留 removed_from_ops 事件与实际 managed 状态，无法解释真实授权顺序。SR-034 解释通用 worker 所有权问题，本条是联系人纳管授权被晚到任务反转的独立业务不变量破坏。
- 建议：联系人维护单调 `enrollment_generation`/desired status；启用任务保存 generation，最终回写必须 CAS `{_id,workspace,agent_status:managed,enrollment_generation:g}`，且不得由画像 helper重新赋予 managed。disable/hide 原子递增 generation并取消同代 pending/running task；worker 在 LLM 前后都检查 generation。增加 barrier 测试覆盖“检查后禁用/隐藏”“旧代任务晚到”“重新启用新代不被旧任务覆盖”。

## SR-084：联系人列表为最多 500 行逐条查询最近消息，正式运营池刷新产生 N+1 热点

- 严重度：P2
- 确定性：FACT（实际延迟取决于联系人数量、Mongo 往返和消息规模）
- 证据：`src/routes/contacts.rs:141-246`、`src/db/indexes.rs:102-108`、`frontend/src/stores/userOpsStore.ts:286-298`。
- 机制：`list_contacts` 先拉最多 500 个 contact，再在 cursor 循环中为每个联系人串行执行一次 `messages.find_one`，按 workspace/account/contact/direction 排序取最新入站。消息复合索引能让单次查询较快，但不能消除最多 501 次数据库往返；正式前端刷新固定传 `limit=500`，搜索和 tab 刷新都会触发该路径。roster 富化已经采用一次快照批量加载，最近消息却没有批量 aggregation、lookup 或 contact 上的冗余预览字段。
- 影响：运营池接近上限时，一次页面刷新会串行占用大量 Mongo 往返并显著抬高 P95/P99；多个管理员或频繁搜索可放大数据库连接压力。接口没有分页总游标，前端又只展示首 500 行，因此系统同时承担高查询成本和不完整列表。
- 建议：优先在 Webhook 持久化 contact.last_inbound_preview/type/time，使列表单查询返回；或以一次 aggregation 按 contact 分组取最新消息并内存 join。补充真正的游标分页，按当前页而非固定 500 富化。增加 query-count 断言、500/多管理员负载基线和 P95 预算；索引 explain 只能验证单查询，不应被当成 N+1 的修复。

## SR-085：Evolution worker 与审计面固定到默认 workspace/account，非默认租户开关永远不驱动演化

- 严重度：P2
- 确定性：FACT
- 证据：`src/evolution/mod.rs:53-117,121-229`、`src/evolution/threshold.rs:79-100,224-251`、`src/evolution/prompt_critic.rs:85-123,250-281`、`src/evolution/auto_release.rs:48-86`、`src/routes/evolution.rs:64-103,638-681`、`frontend/src/features/evolution/EvolutionCenterTab.tsx:177-274`。
- 机制：Evolution runtime flag 是 workspace 级，GET/PUT 都按 `admin.current_workspace` 读写；但唯一常驻 worker 的每个 tick 无条件取 `state.config.default_workspace_id/default_account_id`，cohort、threshold/prompt proposal 与 auto-release 也沿用同一默认 scope。路由列表则使用“当前管理员 workspace + 进程默认 account”，阈值审计同样如此，且前端没有 account selector。非默认 workspace 管理员可成功开启自己的 runtime flag，却没有任何 worker 消费；同一 workspace 的非默认账号也永远不进入 cohort，面板只看默认账号而不说明该限制。
- 影响：多租户/多账号部署中，除进程默认 scope 外的运行数据确定性不产生实验、候选、shadow replay 或自动阈值发布；管理员看到开关保存成功，却长期得到空演化中心，形成无错误、无告警的功能黑洞。反过来，把默认配置改到另一个租户会整体转移唯一演化资源，并使其他租户已有 flag 继续成为死配置。
- 建议：worker 每 tick 枚举显式启用且管理员有权管理的 workspace/account scope，按 scope 独立预算、cohort、experiment id、限流和 auto-release；路由强制选择并验证 account，响应显式返回 scope。若产品只支持单一默认 scope，应禁止其它 workspace 写 flag并在 UI/健康检查明确暴露限制，而不是接受无消费者配置。增加两个 workspace、每个两个账号的开关/实验/审计隔离测试。

## SR-086：Evolution 候选不绑定评估时的生效基线，晚发布会把旧证据应用到新 Prompt 或新阈值上

- 严重度：P1
- 确定性：FACT（触发需候选进入 eligible 后、管理员 release 前，同一 Prompt/阈值已有其它发布或手工变更）
- 证据：`src/evolution/prompt_critic.rs:117-123,250-281`、`src/agent/prompt_shadow.rs:100-123`、`src/evolution/threshold.rs:126-188,224-251`、`src/evolution/release.rs:36-147,198-370`；Shadow 对照本身的非冻结输入见 SR-049。
- 机制：Prompt critic 读取当时 current 模板生成 diff，但 Proposal 不保存 base prompt version/hash，`previous_prompt_version` 在候选与评估阶段恒为空；shadow 只携带 target key + snippet。release 时重新读取“此刻 current”，把旧 snippet 追加到任意后来版本并把该后来版本记为 previous。Threshold proposal 虽保存生成时的 `current_value`，release 只插入绝对 `proposed_value`，不校验当前 active override 仍等于该基线。两类 release 都只检查 proposal status，而不以候选评估时的 immutable base 做 CAS。
- 影响：管理员看到的 diff、shadow 指标和风险说明可能针对版本 A，实际发布物却是“版本 B + A 的补丁”；补丁可与 B 重复、冲突或绕开已修订语义。旧 threshold 候选也可覆盖更晚人工/自动调优结果，使生产值跳回基于过期命中率计算的绝对值。事务只能保证这次错误组合原子落库，不能证明发布的是被评估对象。
- 建议：候选创建时保存 content-addressed base：Prompt 的 base version/content hash/完整候选内容，Threshold 的 base override id/value/generation；shadow 与管理员确认都绑定同一 candidate hash。release 以 current generation/hash CAS，基线漂移则标 stale 并要求重新生成/评估，不得自动 rebase。测试覆盖候选后手工发布、另一候选先发布、阈值先变更以及内容相同但版本不同的 stale 拒绝。

## SR-087：任意旧 Prompt proposal 的 rollback 都会翻掉当前版本，可跨越后续合法发布直接跳回历史基线

- 严重度：P1
- 确定性：FACT（触发需同 prompt key 在该 proposal release 后又有至少一次发布）
- 证据：`src/evolution/release.rs:550-704`、`src/db/indexes.rs:1396-1415`、`tests/evolution_rollback_status.rs:14-139`。
- 机制：Prompt release 只在 proposal 中保存“发布前版本号”，不保存自己创建的新版本 id/version。rollback 对任意 `status=released` proposal，先把同 key 当前任意 `current_version=true` 行置 false，再把该 proposal 的 previous version 置 true；它完全不验证当前行是否就是该 proposal 发布的版本，也不比较 lineage/generation。数据库只保证 `(workspace,prompt_key,version)` 唯一，current 辅助索引不唯一，不能提供这种所有权约束。
- 影响：先发布 P1 产生 v2，再发布 P2/手工变更产生 v3/v4 后，回滚旧 P1 会删除所有后来合法改动并直接恢复 v1；P2 仍显示 released，但其生产效果已被另一旧提案撤销。管理员所理解的“撤销该提案”实际变成“把整个 prompt key 回到该提案之前”，可能恢复已修复的安全缺陷或过期业务规则。
- 建议：每次 release 保存 `released_prompt_version/id` 与 parent generation；rollback 仅允许 CAS 当前指针仍指向该 proposal 的发布物。若需要跨版本撤销，应基于不可变版本图生成一个新的 revert proposal，展示将被覆盖的后续提交并重新确认/评估，而不是直接移动指针。增加 P1→P2→rollback P1 拒绝、rollback 最新成功、手工发布插队和并发 rollback 测试。

## SR-088：Evolution 生产变更提交后才写事件和观测任务，失败会返回错误或永久失去 post-release review

- 严重度：P2
- 确定性：FACT（触发需事务提交后的 event/review insert 失败，或多 worker 并发处理 review）
- 证据：`src/evolution/release.rs:126-178,370-405,509-547,685-704,707-743`、`src/evolution/post_release.rs:68-134,137-258`、`src/db/indexes.rs:1373-1393`、`src/evolution/auto_release.rs:193-204`。
- 机制：threshold/prompt release 与 rollback 的核心变更和 proposal 状态在事务内，但 commit 后 `write_release_event` 用 `?` 冒泡；事件失败时生产已经变更，HTTP/auto-release 却收到 Err，且 release 路径因为在事件之后才 schedule review，根本不会创建观测任务。即使事件成功，review insert 也只是 warn 后放弃，没有 unique/reconciler。到期 review scanner 又不 claim：多个 worker可同时计算；单个 worker先无条件写 `completed=true`，再插事件，事件失败后外层日志声称“下 tick 重试”，但 completed 行已不再被扫描。
- 影响：客户端可能收到发布/回滚失败却面对已生效生产状态；auto-release 日志声称会重试，但 proposal 已 released，下一 tick不会再处理。更重要的是，恰在提交后的故障会永久缺失 24h 安全观测，或保留 completed 指标却缺审计事件；多实例还可重复计算和重复事件。核心事务正确并不能覆盖这些对外结果与监控承诺。
- 建议：把 release outcome、审计 outbox 和唯一 post-release review intent 与业务提交放入同一事务；API 以已提交 outcome 返回，异步 dispatcher/reconciler 重试事件。review 对 `proposal_id` 建唯一并用 owner/generation claim，指标+终态+审计采用事务或 durable finalize marker；事件失败不得伪装为业务提交失败。测试覆盖 commit 后 event 失败、review insert 失败、进程退出、多 scanner 和 completed 后 event 失败恢复。

## SR-089：行业画像 JSON 键转换把字符序号当作 UTF-8 字节下标，非 ASCII 键可使生成请求 panic

- 严重度：P2
- 确定性：FACT（触发需模型返回含非 ASCII 前缀且后续出现大写字母的键）
- 证据：`src/routes/guide_profile.rs:42-77,357-398,516-568`。
- 机制：`to_snake_case` 以 `s.chars().enumerate()` 遍历字符串，但把枚举得到的字符序号 `i` 直接用于 `s[..i]` 与 `s[i..]` 字节切片。Rust 字符串只能在 UTF-8 字符边界切片；例如 `中文Key` 遍历到 `K` 时 `i=2`，字节偏移 2 落在第一个中文字符内部，确定性 panic。`normalize_json_keys` 会递归处理模型返回 JSON 的每个对象键，生产生成路径在反序列化前直接调用，因此一个本地化或额外键即可让请求在草稿插入前 unwind。现有测试只覆盖 ASCII camelCase/snake_case 键，未触达该边界。
- 影响：外部 LLM 的合法 UTF-8 JSON 可让画像生成接口崩溃；取决于服务 panic 策略，请求会至少失败并丢失本次候选，严重时可终止 worker/process。该风险不要求正文异常，只需任一嵌套对象键满足触发形状，因此不能依赖提示词保证规避。
- 建议：使用 `char_indices`、成熟 case-conversion 库或显式 schema 映射，任何切片都必须基于真实字节边界；更稳妥的是只接受已知键并拒绝未知/不合规键。增加中文、emoji、组合字符、任意 Unicode 键和属性测试/fuzz，断言转换永不 panic 且未知键按契约失败。

## SR-090：DomainProfile 草稿被默认列表、统一待审箱和发布卡共同过滤，正式人审工作流不可达

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/guide_profile.rs:391-448`、`src/routes/domain_profiles.rs:54-87,163-193`、`src/routes/ask_human_inbox.rs:349-390,568-576`、`frontend/src/stores/strategyStore.ts:322-342`、`frontend/src/components/review/ProfilePublishCard.tsx:41-47,119-120`、`frontend/src/components/review/__tests__/ProfilePublishCard.test.tsx:54-65`。
- 机制：AI 生成和手工创建都固定写入 `current_version=false,is_active=false` 的原始草稿；DomainProfile 默认列表却只返回 `current_version=true`。正式 strategy store 使用默认列表且生成成功后只显示文本提示，没有保存 id 或提供直达审核链接。Ask-Human collector/summary 又只收集 `current_version=true && is_active=false`，原始草稿不会进入统一待审箱。即使外部拿到 id 打开 `ProfilePublishCard`，组件仍先调用默认列表再按 id 过滤，找不到时永久显示“加载中…”。其单元测试手工 mock 了默认列表返回 `current_version=false` 行，这是后端真实契约不可能产生的响应，因而掩盖了裂缝。
- 影响：生成或创建接口返回成功后，草稿会从正式管理视图消失；管理员无法通过官方 UI 检查画像字段与生成的状态机、修改、发布或激活。生产激活方向仍是 fail-safe，但产品宣称的自助生成→人审→发布链路确定性中断，数据库会积累不可管理草稿。
- 建议：为草稿建立显式状态与查询契约，strategy 列表请求应包含相关非 current 版本；发布卡直接调用已有的按 id GET，而非从默认列表反查；Ask-Human 将原始草稿作为独立 review state 纳入 collector/summary。生成响应返回可导航的 candidate id，并增加真实后端+前端契约测试，覆盖 AI/手工草稿可见、按 id 加载、审核、发布和激活全链路。

## SR-091：Guide 预览不冻结生成基线，旧候选可静默覆盖或合并到后来已变更的生产配置

- 严重度：P1
- 确定性：FACT（触发需预览生成后、确认应用前，contact、memory、Playbook 或 Domain runtime 已被其它请求修改）
- 证据：`src/models.rs:3246-3271`、`src/routes/guides.rs:88-185,292-448`、`src/routes/shared.rs:661-892`。
- 机制：preview 读取 contact、memory、review、Playbook、Domain 与 taxonomy 生成绝对 `suggested_changes`，但持久模型只保存候选正文，不保存任一输入实体的 id/version/updated_at/hash。apply 时重新加载此刻的 contact/memory/Playbook/Domain，再把旧候选合并到最新对象；事务中的 contact/memory updated_at、Playbook version、Domain updated_at CAS 只防“apply 重新读取之后”的并发写，不能发现“preview 生成之后、apply 读取之前”的基线漂移。contact 字段按旧候选绝对覆盖，memory/runtime 在新基线上 merge，Playbook 在当前版本上直接改写，均不是用户预览时看到的状态转换。
- 影响：管理员确认的是基线 A 上的建议，实际提交可能是覆盖基线 B 或把 A 的 patch 自动 rebase 到 B；期间的人工修订、另一个管理员操作或自动更新可被悄悄改回或组成未经预览的新配置。事务保证这组新组合原子写入，却不能证明它就是被确认的组合。
- 建议：preview 保存每个目标的不可变 base identity（contact/memory updated_at、playbook id/version/content hash、domain id/version/hash）和规范化 candidate hash；apply 在事务内对全部 base 做 CAS，任一漂移即标 stale 并要求重新生成预览，不得自动 rebase。确认页展示从冻结 base 到 candidate 的服务端 diff，并增加各目标在 preview/apply 间变更及组合变更测试。

## SR-092：Guide 标签建议写入已废弃的裸 tags 字段，界面报告已处理但运行时永远不消费

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/shared.rs:661-673,953-956,1018-1043`、`src/models.rs:180-190,3546-3609`、`src/routes/contacts.rs:1188-1197,1374-1442`、`frontend/src/features/user-ops/index.tsx:191-206`。
- 机制：Guide prompt 明确要求模型可输出 `suggestedChanges.tags`，apply 白名单随后把它写成 contact 顶层 `tags`。但 Contact 模型已没有该字段，正式标签读模型只合并 `manual_tags` 与 `confirmed_tags`；相邻画像更新注释也明确“裸 tags 已废”。标准人工标签端点写 `manual_tags`，执行 trim/去重、最多 32 条和单条 64 字符校验并记录 actor，Guide 全部绕过。`applied_fields` 仍把顶层 `tags` 计为已应用，前端成功提示不会暴露它是孤儿 BSON 键。
- 影响：运营确认“更新用户标签”并收到成功后，联系人 API、Prompt、记忆和标签 UI 都看不到该值；数据库却积累无类型、无长度限制、无审计主体的孤儿字段。后续排障会同时面对“写成功”的事件和完全未生效的运行行为。
- 建议：Guide 的人工确认标签应复用 `normalize_manual_tags`/`validate_manual_tags` 并写 `manual_tags + updated_at + actor`；若产品意图是 AI 证据标签，则必须走 confirmed/candidate 的证据协议，不能混用。迁移/清理既有孤儿 tags 前先审计内容，并增加 Guide→API→Prompt 端到端可见性、超限拒绝和 actor 测试。

## SR-093：Guide 的全局配置确认依赖模型自报范围与摘要，模型可隐藏实际 Playbook/Domain 副作用

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/guides.rs:151-185,292-448`、`src/routes/shared.rs:837-892,932-1043`、`frontend/src/features/user-ops/cockpit/ConfigureView.tsx:317-335`、`frontend/src/features/user-ops/legacy.tsx:201-215,1779-1804`、`src/agent/decision.rs:1035-1073`。
- 机制：是否输出 `playbookPatch`/`domainRuntimeParameters`、`impactScope`、`scopeReason`、`readableChanges` 和 `riskWarnings` 全由同一次 LLM 响应决定。后端不从原始 instruction 独立判定全局意图，不校验 impactScope 与实际字段一致，也不对全局字段要求更强确认。正式 UI 优先展示 LLM 自报的 `readableChanges`；只要该数组非空，就不渲染真实 `suggestedChanges`，因此模型可声明“只影响当前好友”并给出无害摘要，同时携带 Playbook 或 workspace Domain patch。Playbook 行会影响绑定同一 id 的其它联系人，Domain runtime 按 workspace current config 直接生效；确认按钮仍是同一个“一键应用”。
- 影响：管理员授权的是自然语言摘要而非实际规范化 diff，单次点击可在没有准确范围提示的情况下改变同 Playbook 用户或整个 workspace 的运行策略。调用者本身是已认证管理员，所以这不是权限提升；缺陷在于确认内容与真实副作用没有可信绑定。
- 建议：服务端根据字段和目标实体计算 authoritative scope/risk，忽略模型自报范围作为授权依据；确认页始终展示规范化真实 diff、目标 Playbook 绑定人数和 Domain workspace 影响，global patch 使用独立强确认/capability。preview 保存 candidate hash，apply 只接受该 hash；增加模型伪造 current_contact/空风险/无害摘要但携带全局字段的红线测试。

## SR-094：Domain runtime 写入缺少统一强类型校验，Guide 与手工编辑都可持久化危险取值

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/shared.rs:862-892,979-981`、`src/routes/guides.rs:423-448`、`frontend/src/stores/userOpsDomainHelpers.ts:33-96`、`frontend/src/stores/userOpsStore.ts:1017-1033`、`frontend/src/features/user-ops/legacy.tsx:827-1029,1514-1620`、`src/routes/domains.rs:34-49,89-147,298-317`、`src/models.rs:1341-1375,3814-3859,4686-4699`、`src/agent/runtime.rs:128-180,356-446`、`src/agent/gateway.rs:3621-3627`。
- 机制：Guide 的 `domainRuntimeParameters` 接受任意 BSON Document，仅过滤 null 后浅合并进 current config；没有已知键白名单、BSON 类型校验、typed round-trip 或业务范围验证。正式“运行策略”编辑器也把文本行解析成任意 key 与 `number|boolean|string`，数字输入只是 `inputMode="numeric"`，负数、极大值和非数字字符串仍可提交；`PUT /operation-domains/:domain` 的 `OperationDomainRequest.runtime_parameters` 是裸 Document，服务端只校验七个说明字符串和 state machine，不校验 runtime。运行时虽有 `RuntimeParametersTyped`，但解析失败会对整份文档 `unwrap_or_default`，且 maxDailyTouches、预算、重试等多项字段未统一 clamp。例如 `maxDailyTouches < 0` 时主动触达门的 `count >= max` 恒成立，错误类型还可能令整份 typed 配置退回默认并改变与本次编辑无关的参数。
- 影响：一次看似局部的 AI 建议或人工设置可关闭主动运营、放大 LLM 预算/调用次数、改变可靠发送与安全阈值，或因单字段类型错误让整份运行配置静默回默认；持久层仍保留损坏值，管理面、实际运行和审计摘要可能三者不一致。SR-074 描述 Domain 发布协议，本项聚焦所有写入口共同缺失的配置 schema。
- 建议：所有 runtime 写路径共用一个拒绝未知键的 typed validator，按字段做类型、单位、合理区间和跨字段约束；将完整候选反序列化并规范化后再生成 diff，任何错误整体拒绝，不能默认化。高风险阈值/预算禁止经通用 Guide 修改或要求专门确认。增加负数、超大、错误 BSON 类型、未知键、单字段错误不得重置其它配置及运行时门控测试。

## SR-095：Guide 事务提交后重建响应仍可失败，客户端收到 502 后重试只能得到已应用冲突

- 严重度：P2
- 确定性：FACT（触发需 commit 成功后的 contact/memory/review/domain/profile 读取失败或行被并发删除）
- 证据：`src/routes/guides.rs:477-545`、`src/routes/shared.rs:490-520`、`src/error.rs:63-139`、`frontend/src/stores/userOpsStore.ts:759-788`。
- 机制：事务已经原子提交业务写、事件和 preview=`applied` 后，handler 又依次读取 contact、ensure memory、latest review、Domain config 与 active profile来重建 health；这些操作均可返回错误，且不在已提交事务内。任一失败会映射为 502/404，前端保留原 guidePreview 并显示失败。再次点击会因 preview 已是 applied 在 claim 阶段固定返回 409，接口没有按 preview id 查询已提交 outcome 或幂等返回原结果。
- 影响：管理员看到“应用失败”，实际配置已经生效；重复操作无法确认结果，可能转而手工再改或生成第二个候选，造成重复/相反修改。事务解决了原子性，却没有给调用方确定的提交结果。
- 建议：事务内保存最小 committed outcome/result snapshot，commit 后立即以该确定结果返回；展示性 health 异步刷新或失败时返回 committed=true + degraded view，不能把读模型故障伪装成业务失败。apply 对已 applied 的同一 preview 幂等返回已提交 outcome。增加每个 post-commit 读取点故障、响应丢失和重试测试。

## SR-096：Chunk 软锁既可跨 workspace 抢占又不是写入门，界面的“暂只读”无法提供互斥编辑

- 严重度：P1
- 确定性：FACT（并发覆盖需两个请求交错；跨 workspace 阻断只需知道或猜中同一 chunk id）
- 证据：`src/routes/chunk_locks.rs:41-198,200-305`、`src/routes/knowledge/wiki_edit.rs:90-197,521-679`、`src/knowledge_wiki/chunk_revisions.rs:168-386`、`frontend/src/features/knowledge/shared.tsx:600-733,736-813`、`tests/chunk_lock_lifecycle.rs:1-129`。
- 机制：acquire/release 只操作进程内 `DashMap<chunk_id, lock>`，既不按 workspace 组成 key，也不查询该 chunk 是否属于当前管理员；任一已认证管理员可为任意字符串建锁，命中其它 workspace 的真实 id 时，受害 workspace 的 acquire 会收到包含对方 owner/workspace 的 409。acquire 是“get 快照→insert”，release 是“get 快照→remove”，都不是同一 entry 上的原子 compare-and-set：两个管理员可同时读到空锁后都返回 200，旧 owner 也可在快照后删掉已换主的新锁。更根本的是，所有 patch/archive/restore/merge 等写路由直接进入 revision 内核，从不校验当前 lock owner/token；前端虽据锁态禁用按钮并显示“暂只读”，直接 HTTP 请求、另一个副本或竞态赢家仍可写。revision 的 `updated_at` OCC 只能让部分同时落库的写冲突，不能兑现锁的租户隔离、所有权或多对象 merge 互斥。
- 影响：一个 workspace 的管理员可阻断或窥见另一个 workspace 的编辑锁；同一进程内两个编辑者都可能以为自己持锁，多副本则天然各持一份互不相知的锁。运营看到“我正在编辑/对方暂只读”仍可能遭遇 409、部分 merge 或内容被另一请求抢先改写，锁事件与真实可写权分裂。
- 建议：锁必须是数据库中的 `(workspace_id,chunk_id)` durable lease，acquire/renew/release 以 owner+generation 原子 CAS；入口先验证 chunk 归属，响应不得泄漏其它租户 owner。所有修改（含多 chunk 操作）携带 lease token/generation 并在事务或提交 CAS 中校验；若产品只要 presence 提示，应删除“锁/暂只读”承诺并明确标成 advisory。增加跨 workspace 同 id、双 acquire barrier、旧 owner 晚 release、多副本和绕过 UI 直写测试。

## SR-097：Lesson 晋升先建 Chunk 再无 CAS 标记来源，并发或中间失败会制造重复同行案例候选

- 严重度：P2
- 确定性：FACT（触发需并发 promote、lesson 回写失败或进程在两步间退出）
- 证据：`src/routes/lessons_learned.rs:141-286`、`src/knowledge_wiki/lessons_learned.rs:145-174`、`src/db/indexes.rs:1843-1860`、`frontend/src/components/review/LessonPromoteCard.tsx:20-72`。
- 机制：handler 先普通读取 lesson，仅在读到 `review_status=promoted` 且已有 chunk id 时短路；随后独立 insert 一个新的 draft peer_case，再以只含 `lesson_id`、不含 workspace/status/expected value 的 update 写回 promoted。两步没有事务、processing claim、幂等键或唯一 provenance 约束，lessons 集合本身也只有 workspace+updated_at 普通索引。两个并发请求都可读到 pending 并各插一条候选，最后一次 lesson 回写覆盖 `promoted_chunk_id`；若 insert 后回写失败，客户端重试会再建一条。定时聚合的 `$setOnInsert` 会保留 promoted 状态，不能回收这些未被 lesson 指向的重复 chunk；尾部事件失败还被静默吞掉。
- 影响：一次明确的人工“晋升”可对应多个内容不同的 peer_case 候选，只有最后一条被 lesson/UI 追踪，其余仍进入知识审核队列并可能被分别 verify。审计、来源样本和最终知识无法证明哪条是该次确认产生的对象，重复候选还会增加人工审核与召回冲突。
- 建议：为 `(workspace,lesson_id,promotion_generation)` 建唯一 promotion intent；事务内 CAS `pending_review→processing`、插入带 `source_lesson_id` 唯一键的 chunk、写回终态与审计。失败进入可重试 failed，重复请求返回同一 committed outcome；至少也应先原子 claim 并让 chunk insert 以 lesson id 幂等 upsert。增加双请求、insert 后崩溃、lesson update/event 故障和重试测试。

## SR-098：评测场景没有基准 schema，正式 UI 创建的 active 场景会把缺失 ground truth 当成零分真值

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/evaluations.rs:25-183,186-219,261-407,610-628`、`src/models.rs:3385-3416`、`frontend/src/features/quality/EvaluationScenariosPanel.tsx:19-92`、`frontend/src/features/quality/index.tsx:249-308`。
- 机制：create/update 除 create 时检查 scenarioId 非空外，不校验 title、account、status 闭集、消息数量、contact seed、ground_truth 的公式键/数值类型/0..10 范围；status 缺省直接 active。正式 UI 创建请求只发送 scenarioId/title/description/inboundMessages，根本不提供 groundTruth，因此会持久化 `{}` 的 active 场景。评测时只要模型返回任一公式，就把 ground truth 缺键 `unwrap_or(0.0)`，将“未标注”解释为真实零分并计入 adherence；非数值字符串/对象也由 `bson_to_f64` 静默变 0。runner 还不按 scenario.account_id 过滤，账号专属场景会被其它账号执行。
- 影响：官方自助入口创建的场景会系统性惩罚正常的正分预测，平均 adherence 看似精确却没有人工基准；脏 status、跨账号场景和部分公式缺失进一步让不同运行不可比。该指标若用于质量判断、发布门或调参，会把配置错误误诊成模型退化。
- 建议：场景保存前按 active DomainProfile 的 formula schema 验证完整 ground truth、有限数值和范围；未完整标注只能存 draft，不得进入评测。明确 account scope（全局或指定账号）并在 runner filter 中执行；缺失/非法 truth 将整个场景标 invalid，绝不能回落 0。前端必须编辑并展示全部基准维度。增加正式 UI payload、部分/错误类型/越界 truth、跨账号与动态公式测试。

## SR-099：公式评测的批次 token 预算读取错误数据源，正常时恒为零并会被并发生产 run 污染

- 严重度：P2
- 确定性：FACT（错误提前终止需同 workspace/account 在评测期间产生生产 run；预算失效在无并发生产流量时即成立）
- 证据：`src/routes/evaluations.rs:231-305,326-343,410-484`、`src/agent/simulation.rs:38-89,93-264`、`src/agent/mod.rs:209-310,373-458`、`src/agent/budget.rs:55-185`。
- 机制：每个 simulation 的真实 token 只累计在 task-local `RunBudget`，LLM 调用另写 `llm_call_logs`；`simulate_user_dialogue` 不创建或终结 `agent_run_logs` envelope，也不向 caller 返回 budget snapshot。批次 runner 却在每场后查询“评测开始以来该 workspace/account 的全部 agent_run_logs.tokens_used”，再把它当本批消耗。正常无生产流量时查询永远为 0，所谓 `simulationTokenBudget × scenarios` 总门不会触发；若同时有真实 Gateway/Reaction run，则无关生产 token 被计入评测，可能提前 break。按开始时间而非本批 run ids 还使并发两次评测彼此不可归属。
- 影响：管理员可运行大量场景而批次预算仍显示 `0 / budget`，失去预期的成本上限；反向地，客户高峰期会让同一评测随机降级、少跑场景并把生产成本冒充评测成本。UI 与审计事件中的 totalTokensUsed、processedBeforeBudgetExceeded 均不可复核。
- 建议：simulation 返回自己的 `BudgetSnapshot`/run id，批次在内存中只累加本次子 run 的真实 usage；达到预算前不启动下一场，并为并发评测分配 evaluation_id。若需持久审计，写独立 evaluation run/item，而不是扫描生产 envelope。增加无生产流量仍准确计费、并发生产不影响、双评测隔离和边界场景不超启测试。

## SR-100：Observability 把历史状态比率命名为本轮扫描命中率，并在 24h 信封中混入全量、14d 与 30d 指标

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/observability.rs:51-76,253-411,439-655,657-745`、`src/knowledge_wiki/gap_signals.rs:580-677,1355-1528`、`src/knowledge_wiki/reviewer_stats.rs:47-135`、`src/knowledge_wiki/feedback_worker.rs:43-131,138-169`、`frontend/src/features/knowledge/steward.tsx:2418-2613,2615-2797`。
- 机制：`worker_health.gapSignals.sweepHitRate` 的分母实际是集合中所有 status（包含 pending、dismissed 和闭集外历史值），分子是所有历史 `auto_resolved|llm_resolved|applied`；没有 sweep generation、时间窗口或本轮候选数，和注释声称的“上一轮 sweep 消化率/排除 pending、dismissed”均不一致。前端却展示为“知识缺口·扫描命中率”并按阈值着色。更广泛地，`phase_rollup` 顶层固定宣告 `windowHours=24`，同一信封却混入当前 pending、全量 escalation、14d reviewer stats 与 30d deal attribution；`worker_health` 也宣告 24h，但 chat/gap 状态和错误是全量、lessons 才是 14d。各子查询还没有共同 asOf/freshness，缓存型统计只给 deal attribution 暴露更新时间。
- 影响：一个从未再运行的 worker 也可长期显示高“命中率”，积压 pending 或大量 dismissed 会以与本轮无关的方式改变比例；管理员无法从面板判断 worker 最近是否执行、是否成功或统计是否陈旧。混合窗口下的数字不能相互比较，容易把历史累计、当前库存和近期流量误读为同一 24h 健康快照。
- 建议：worker 每轮写 durable run record（generation、started/finished/status、input/new/resolved/error），命中率只用同一轮或明确窗口的分子分母；响应为每个 metric 返回 `window/asOf/updatedAt/source`，不要用一个顶层 window 覆盖异质指标。库存与流量分面展示，数据库错误不得伪装为空。增加多轮累计、pending/dismissed、陈旧 worker、局部故障和固定 asOf 测试。

## SR-101：通用 Chunk PUT 只要引用能锚定就自动写成 verified，AI 修复可绕过人工确认直接进入生产召回

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/knowledge/crud.rs:219-335`、`src/routes/knowledge/mod.rs:907-947,1037-1076`、`frontend/src/lib/applyAiRepairPatch.ts:1-58`、`frontend/src/features/knowledge/ChunkRepairPanel.tsx:76-108`、`src/agent/knowledge_router.rs:1184`。
- 机制：PUT 在读取父文档后无条件调用 `apply_chunk_integrity`；该 helper 只要 sourceQuote 能定位出任一 anchor，就直接设置 `integrity_status="verified"`，并保留请求体默认的 `status="active"`。后续 D2 coerce 只在 verified 缺 quote/anchor 时降级，证据齐全时不会要求 `/verify`、人工签字或审核 generation。正式 AI 修复前端虽声明 `thenVerify=false`、提示“已落库为草稿”，实际却把列表中的 originalChunk 与 AI patch 拼成完整 PUT；原 chunk 已有可定位 quote/anchor 或 AI 修复补齐二者时，PUT 自己就完成 verified。PUT 产生的 revision 还固定标 `source=human`、`created_by=admin.user_id`，无法表明关键字段来自 AI 候选。
- 影响：AI 生成的修复内容可在运营只点击“落库勾选字段”、未点击“确认放行”时进入 `active+verified`，随后被知识路由当作产品事实背书；界面、审计和真实生产状态三者相互矛盾。该路径绕过 auto-verify 的 `needs_human_audit` 强制降级与显式 verify gate，是“AI 永不自动 verify”红线的直接旁路。
- 建议：通用 create/PUT/patch 永远不得提升 integrity；只允许显式 `/verify` 或绑定真人授权 intent 的单一状态机执行 `needs_review→verified`。AI repair 应提交带 candidate/session/hash 的字段 patch，并在服务端强制 `draft+needs_review`；revision source/actor 从真实来源派生，不能固定伪装 human。增加“锚点已存在的 needs_review 经 repair PUT 仍保持待审”、直接 PUT verified 被拒绝及最终召回不可见测试。

## SR-102：Split 先写服务端作用域再被任意 newChunks 字段覆盖，可向其它 workspace 注入 active verified Chunk

- 严重度：P1
- 确定性：FACT
- 证据：`src/routes/knowledge/wiki_edit.rs:365-500`、`src/models.rs:1583-1671`、`src/knowledge_wiki/chunk_revisions.rs:168-386`。
- 机制：split 对原 chunk 做了 current workspace 校验，但创建子 chunk 时先把 `workspace_id/account_id/document_id/status=draft/integrity_status=needs_review` 等服务端字段放进 BSON Document，随后遍历调用者提供的任意 `new_chunks[i]` 顶层对象并无白名单覆盖同名键。最终直接反序列化并 `insert_one`，没有重新强制 workspace、审核状态、provenance、DomainSchema 或父文档归属，也没有让 create 经过 revision 状态机。已认证管理员可在 newChunks 中写 snake_case `workspace_id`、`status=active`、`integrity_status=verified` 和伪造 anchors，把新行插入任意 workspace；后续 best-effort revision 使用当前 workspace 读取该行会失败，但主 insert 已成功。
- 影响：知道目标 workspace id 即可跨租户污染其知识库，且注入内容可直接成为 verified 产品事实；同租户下也能覆盖 account/document/created_at 等服务端所有权字段。响应仍把 inserted id 当成 split 成功返回，而受害租户没有可信来源或创建审计。
- 建议：请求体必须反序列化为专用、拒绝未知键的 `NewSplitChunkPatch`，仅允许 title/body/summary 等内容字段；workspace/account/document/审核状态/provenance/时间由服务端在最后一步覆盖并不可由输入表达。创建与父归档放同一事务，子项逐条通过统一 typed validator 和 revision intent。增加跨 workspace/status/integrity/created_at 覆盖红线测试。

## SR-103：Split/Merge 用多次独立写提交一个动作，任一后续失败都会留下已归档源和不完整结果

- 严重度：P1
- 确定性：FACT（触发需后续反序列化、insert、目标 revision 或数据库写失败；`new_chunk` 策略缺参数即可确定触发）
- 证据：`src/routes/knowledge/wiki_edit.rs:377-500,521-680`、`src/knowledge_wiki/chunk_revisions.rs:306-376`。
- 机制：split 先 archive 原 chunk，再逐个解析、插入 N 个子 chunk；第 k 项格式或 insert 失败时原行已归档且前 k-1 项已存在。`into_target` merge 先归档 source，再更新 target；target 不存在、OCC 冲突或 schema 失败会留下只有 source 被归档。`new_chunk` merge 更在验证 `new_chunk` 参数之前依次归档 A、B，因此缺参数的 400 请求也会确定性改变两条生产数据；之后新行 insert 失败同样无法恢复。新子 chunk 的 create revision又是 best-effort，成功 insert 也可能没有历史。所有步骤没有 transaction、durable operation id、补偿状态或幂等键。
- 影响：一次失败的拆分/合并可让原知识从召回中消失、只生成部分子项、双源归档却没有合并结果，或产生无 revision 的孤儿草稿。客户端只看到 4xx/5xx，重试会在已变更基线上再次执行，扩大重复与数据丢失；SR-096 的软锁也不能保护这种多对象提交。
- 建议：先完整解析并验证全部目标、权限与闭集，再在 Mongo transaction 内以 expected updated_at/generation 同时提交父/目标/子项及 revisions；为 operation_id 建唯一 intent，重试返回同一 outcome。若事务不可用，应先写 durable plan/processing 状态并提供幂等 step + reconciler，绝不能先归档再验证输入。增加每个步骤故障、缺 newChunk、target 冲突、部分 insert 和响应丢失测试。

## SR-104：Chunk revision 只保存 patch 与哈希，所谓“回滚至此版本”却拿前一条 patch 当快照恢复

- 严重度：P1
- 确定性：FACT
- 证据：`src/models.rs:1817-1839`、`src/knowledge_wiki/chunk_revisions.rs:303-321`、`src/routes/knowledge/wiki_edit.rs:200-295`、`frontend/src/features/knowledge/shared.tsx:1031-1152`。
- 机制：revision 模型只持有本次请求 patch 和 before/after hash，不保存 before image、after image 或可逆 diff。rollback 选定目标 revision 后，却查 `created_at < target.created_at` 的前一条 revision，并把“目标 patch 中的每个 key”设置为“前一条 revision patch 里的同名值”；前一条 patch 记录的是更早动作写入的值，不是目标动作发生前的真实字段值。字段若没恰好出现在相邻 patch 就被列 missing 并完全不恢复；整条 PUT 的 revision patch 固定为空，verify/archive 等也无法重建完整状态。它还允许对任意旧 revision 在当前最新状态上局部套用，不校验后续 lineage。
- 影响：正式时间轴向管理员承诺“回滚到该版本”，实际可能 no-op、恢复成错误值或把历史局部字段覆盖到包含后来合法修改的当前行；接口仍返回 ok 并再写一条 rollback revision，形成看似可信但不可重放的审计链。产品事实、审核状态和关系数组均可能被错误重写。
- 建议：revision 必须保存规范化 before/after snapshot（或真正可逆的 per-field old/new diff）及 parent revision/generation；rollback 先重建并展示目标 after-state，再以 current head CAS 创建新的 revert revision。旧 head 非目标后代或存在 later edits 时要求显式三方 diff/确认。现有不可逆历史只能标“不可精确回滚”，不得继续显示统一回滚按钮。增加多字段非相邻 patch、PUT 空 patch、verify/archive、旧 revision 与并发 head 测试。

## SR-105：正式 Chunk 操作栏发送的 Patch、Split、Merge、Relate 请求均不符合后端契约

- 严重度：P1
- 确定性：FACT
- 证据：`frontend/src/features/knowledge/shared.tsx:736-884`、`frontend/src/components/ui/FormDialog/FormDialog.tsx:34-65,105-138`、`frontend/src/components/ui/Overlay/Overlay.tsx:28-68`、`frontend/src/__tests__/components/ui/FormDialog/FormDialog.test.tsx:30-60`、`src/routes/knowledge/wiki_edit.rs:36-76,90-121,365-375,503-519,682-730`。
- 机制：前端“改写摘要”直接发送 `{summary,actor}`，后端要求必填 `{patch:{...}}`；“拆分”发送 `{offset|regex,actor}`，后端要求 `newChunks: Value[]`；“合并”发送 snake_case `target_id`，后端 serde camelCase 字段是 `mergeTargetId`；“关联”同样发送 `target_id` 而非 `targetId`，且 UI 提供的 `supports` 不在后端六值闭集。没有适配层或兼容 alias，这些请求分别在 JSON extractor 或业务闭集校验处返回 4xx。共享 `FormDialog` 还在每次字段更新时重建内联 `onClose`；`Overlay` effect 依赖该函数并在每次重跑时把焦点移回首个控件。用 `userEvent.type` 对第二字段逐键输入 `abc` 的最小复现最终只保留首字符 `a`，随后焦点跳回第一字段；正式 split 的 cutoff 与 relate 的 note 都位于后续字段。现有测试只用一次 `fireEvent.change` 写入整值，因此 34/34 绿测不会触发该真实输入时序。
- 影响：知识 Inspector 中四类核心治理按钮确定性不可用；运营无法通过官方 UI 改摘要、拆分、合并或建立默认“支持”关系，只能使用 verify/archive/restore 或绕过 UI 手工调用 API。即使先修复 wire schema，多字段 split/relate 表单仍会在逐键输入时丢失第二字段后续字符并抢走焦点。界面继续显示完整能力和确认流程，造成治理工作流大面积假接线。
- 建议：定义并生成共享 wire schema/client，前端按后端结构发送；split 若产品意图是 offset/regex，应由后端接收该高层命令并服务端生成候选，不能让两端各自实现不同协议。关系 kind 使用同一闭集。`Overlay` 的进场聚焦/滚动锁定只应由 `open` 的关闭→打开转换触发，或 Provider 向它传稳定 `onClose`，不得因普通表单 state 更新重跑。增加每个按钮到真实 Axum handler 的契约测试、`userEvent.type` 多字段焦点回归，并让 CI 对请求 fixture 做双端对账。

## SR-106：Repair applied 事件完全信任客户端自报且吞掉写失败，审计不能证明任何修复真正发生

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/knowledge/repair.rs:46-75,576-711`、`src/routes/knowledge/mod.rs:1098-1125`、`frontend/src/lib/applyAiRepairPatch.ts:30-58`。
- 机制：`/repair/applied` 不校验 session/proposal 是否存在、candidate hash、目标 revision、实际 before/after、accepted fields 或 thenVerify 结果；无效/不存在 targetId 也只回落默认 account 后写一条 success 事件。字段、置信度、extras 与“已验证”全由客户端提供。底层 `record_repair_event` 对 insert 结果使用 `let _ =`，事件写失败仍让 handler 固定返回 `{ok:true}`；因此前端设计的 `audit_failed` 分支实际上不可达。真实 PUT 与审计又是两个请求，中间退出会有业务变更无事件，单独重放第二个请求则可有事件无业务变更。
- 影响：`knowledge_repair_applied` 既可伪造，也可在真实修复后静默缺失，无法作为合规审计、采纳率、AI 质量或来源追溯依据。界面会把审计写失败显示成整体成功，运维无法区分“修复已提交”“客户端只声称提交”和“事件丢失”。
- 建议：服务端建立 repair candidate/apply intent，保存 session、target、base revision/hash、规范化 patch 与 actor；在同一事务中应用候选、写 revision 和 durable event/outbox，重复 apply 返回同一 outcome。事件写失败必须可恢复且不得伪报成功；移除客户端可控的 thenVerify/accepted 事实，改由服务端从 committed diff 派生。增加不存在 target、伪 session、事件故障、业务提交后断线和重复上报测试。

## SR-107：Auto-verify 在 revision 写入前累计成功统计并吞掉写错误，可报告已分诊但数据库保持原状态

- 严重度：P2
- 确定性：FACT（触发需单条 revision 因 OCC、schema、序列化或数据库错误失败）
- 证据：`src/routes/knowledge/verify.rs:293-305,338-472,475-515`、`src/knowledge_wiki/chunk_revisions.rs:281-351`。
- 机制：每条 LLM 响应解析后，handler 先增加 `processed` 与 final_status 对应计数，再以 `let _ = apply_chunk_revision(...).await` 完全丢弃实际写入结果，随后无条件写一条 usage log，声称该 chunk 已得到 finalStatus。批次末尾事件和 HTTP 响应使用这些预测计数并固定 success，不包含 writeFailed/skipped；即使 DomainSchema 拒绝、OCC 冲突或 Mongo 故障，`needsHumanAudit/rejected/needsReview` 统计仍递增。事件插入也被吞错。
- 影响：管理员看到“自动校验完成”和待复核数量增长，实际 chunk 可能仍是旧状态、收件箱没有对应项；usage/event 数据则记录了从未提交的裁决。批量重跑会重复花费 token，且观测无法区分模型失败、写冲突和真实成功。
- 建议：只在 revision committed 后递增 committed counters 并写 usage；失败分类进入 `skipped/writeFailed`，批次可部分成功但必须返回每项 outcome。为批次/item 持久化 evaluation generation 与 expected revision，支持幂等重试和 reconciliation；事件由 committed items 聚合。增加 schema failure、OCC barrier、Mongo write/event failure 与重跑测试。

## SR-108：文档与 Chunk CRUD 物理替换/删除绕过不可变 revision 和引用生命周期

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/knowledge/crud.rs:73-166,199-217,297-355`、`src/routes/knowledge/mod.rs:397-510`、`src/models.rs:1540-1579,1583-1656,1817-1839`、`src/knowledge_wiki/chunk_revisions.rs:406-496`、`frontend/src/features/knowledge/steward.tsx:100-104,156-170,210-229`。
- 机制：document PUT 用 `replace_one` 重建整行，固定 `version=1`、`created_at=now` 并清空 worker 维护的 `catalog_summary_persisted/catalog_version`，且不检查 matched_count 或旧 version/CAS；前端只能靠先 GET 后回传所有字段避免更多丢失。document DELETE 先删父再独立 `delete_many` 子 chunk，任一步失败都没有事务/恢复 intent。直接 chunk DELETE 物理移除最新行，不写 delete/archive revision、不清 related ref、不排 catalog rebuild；document 级 delete_many 同样绕过每条 revision 和 dangling-ref cleanup。create 也可直接写 active/任意 integrity（仅缺 D2 时降级），没有统一 provenance/revision。
- 影响：一次普通文档编辑会重置版本与创建时间、丢失持久目录快照；物理删除后 chunk revision 仍残留但因父 chunk 消失无法通过授权路由读取，关系图和目录可保留悬空状态。父删除成功、子删除失败时产生无来源文档的孤儿 chunks；客户端却统一收到成功/失败而没有可恢复提交结果。
- 建议：文档与 chunk 统一采用 immutable version/revision 状态机：编辑用 expected version CAS 生成新版本，删除默认 archive/tombstone；文档级级联在事务内逐条写 delete/archive revision、清关系并提交 catalog intent。确需物理 purge 时使用独立高权限异步 job、保留审计与可恢复进度。所有 update/delete 检查 matched/deleted count，增加中间失败、并发编辑、目录字段保留、引用清理和历史可读测试。

## SR-109：管理员可保存任意摄取 URL，后台 Worker 周期跟随重定向抓取，形成持久化 SSRF

- 严重度：P1
- 确定性：FACT（入口要求已认证管理员；实际可达资产取决于部署网络）
- 证据：`src/routes/knowledge/sources_meta.rs:857-898,940-1047`、`src/knowledge_wiki/ingest_worker.rs:44-89,105-164,274-307`、`src/routes/mod.rs:742-747,1120-1127`。
- 机制：摄取源 create/update 只校验 kind、URL 非空和轮询分钟数，不解析 scheme/host，也不拒绝 loopback、link-local、RFC1918、Unix/特殊地址或 DNS 解析到私网的目标。全局 session middleware 只证明调用者是管理员，不能把任意目标转成安全目标。后台 worker 跨 workspace 扫描 active/failing 源，使用默认 `reqwest::Client` 对保存的字符串直接 `GET`；默认重定向策略仍会跟随跳转，代码没有逐跳重新校验目标 IP，也没有出站 allowlist。响应随后被完整读入内存并解析、落为草稿知识，错误与状态由 worker 持续重试。
- 影响：有知识管理权限的管理员账号一旦被滥用，可让服务端周期访问仅内网可达的管理面、云 metadata、sidecar 或本机服务，并通过 `last_error`、状态、ETag、内容是否成功生成 Chunk 等侧信道探测结果；可解析的响应还会持久化进文档/Chunk。重定向与 DNS 变化使仅在保存时做字符串检查也不足。无响应体上限还允许目标返回巨量内容消耗进程内存。
- 建议：建立统一 outbound fetch policy：仅允许 `https`（必要时显式允许 `http`）、解析 DNS 后拒绝 loopback/link-local/private/multicast/保留网段，连接时绑定已验证 IP，并对每次重定向重新执行同一校验；生产可进一步采用域名 allowlist、隔离 egress proxy 与网络层 deny。限制重定向次数、响应字节数和 content type，禁止携带环境代理/凭据。create/update 与 worker 执行前都校验，增加 IPv4/IPv6、十进制/混合编码、DNS rebinding、私网重定向和超大响应测试。

## SR-110：知识元数据端点对 Chunk 限定当前 workspace，却把全库 revision 编辑者和活动量返回给任一租户

- 严重度：P1
- 确定性：FACT（跨租户暴露需数据库中存在其它 workspace 的 revision）
- 证据：`src/routes/knowledge/sources_meta.rs:153-328`、`src/models.rs:1817-1839`、`src/routes/mod.rs:703-706`、`frontend/src/features/knowledge/atlas.tsx:802-837,933-942`。
- 机制：`knowledge_aggregate_metadata` 的 Chunk facet 正确先 `$match workspace_id=current_workspace`；但 `chunk_revisions` 模型没有 workspace_id，第二个 facet 直接对整个集合聚合 `created_by` 和最近七天 `created_at/op`，没有 `$lookup` 回 chunks、没有目标 id 白名单，也没有任何租户 filter。handler 随后把全局 `topEditors` 和 `recentActivity7d` 与当前租户的 wiki/verified 统计拼成同一响应，正式 Atlas 页面直接展示这些数据。全局 session middleware 只要求登录，不会补上集合级作用域。
- 影响：任一 workspace 的管理员可看到其它租户管理员标识及全系统知识编辑时间/操作量；小部署或低流量时，时间序列可推断某租户何时审核、回滚或归档知识。响应还把跨租户分子与本租户分类统计并列，运营会把全局活动误认成本租户数据。该缺口也说明 revision 缺少自包含 tenant key，删除主 Chunk 后将无法可靠恢复授权范围。
- 建议：revision 写入时持久化 `workspace_id`（通常还应有 account/domain）并建立 `(workspace_id, created_at)`、`(workspace_id, created_by)` 索引；所有 revision 读侧强制按 tenant key 过滤。迁移前可用 `$lookup` 将 `chunk_id` 转 ObjectId 后关联当前 workspace chunks，但已物理删除的历史需独立可信归属字段，不能靠现存主行。增加两个 workspace 不同 editor/op 的端点与 Atlas 契约测试。

## SR-111：Chat apply 未原子认领待应用 Turn，会重复创建 Chunk，并可在共享 sessionId 下混用账号草稿

- 严重度：P2
- 确定性：FACT（重复写触发需并发请求、业务写后 turn 更新前失败或响应丢失；跨账号混用需复用同一 sessionId）
- 证据：`src/routes/knowledge/chat.rs:42-88,328-420,422-509,544-611,1668-1800`、`src/db/indexes.rs:453-477`、`frontend/src/features/knowledge/today.tsx:189-217`、`frontend/src/features/knowledge/cockpit/useGoLive.ts:9-28`。
- 机制：apply 先用 `workspace_id + session_id`、显式 `account_id="*"` 读取全部账号历史，再在内存中选择最后一条 `status=pending && patch!=null`；它没有 `pending→applying` 的 findOneAndUpdate claim、apply token 或唯一 outcome。业务写完成后才以 `_id+workspace` 无状态条件把 turn 改成 applied，且不检查 matched/modified count。两个并发请求可同时选中同一 turn：create 分支各自 `insert_one` 一条新 Chunk；若插入成功后 turn 更新/响应失败，客户端重试也会再次创建。session 序号键只有 `workspace|session`，history/discard 同样不含 account，而 sessionId 可由客户端提供；同 workspace 两账号复用 id 时，apply 可选中另一账号的候选，body.accountId 还可改变 create 的落库作用域。
- 影响：一次“应用为草稿”可生成多个内容相同但 id 不同的待审 Chunk，或对同一目标重复写 revision；界面重试看似是恢复，实际扩大重复。共享 sessionId 时，账号 A 的对话/候选可出现在账号 B 的历史或被以错误 account 作用域提交，破坏知识来源和运营归属。前端本地 loading 只能减少单页面双击，无法防多标签、双设备、网络重试或服务端竞态。
- 建议：把 session identity 定义为服务端生成且至少绑定 `(workspace, account, operator, session_id)`，所有 history/seq/discard/apply filter 使用同一键。apply 用原子 claim 将指定 turn 从 pending 推到 applying，并持久化不可复用 apply token、candidate hash 与 committed outcome；Chunk 创建使用 source turn/apply id 唯一键，业务写、revision、turn 终态和审计同事务提交。重复请求幂等返回原 outcome。增加双并发 apply、insert 后故障、响应丢失重试及同 sessionId 跨账号测试。

## SR-112：导入 Apply 用已废弃 items 作为准入门，却把文档与 Chunk 分步直写，失败和重试都会留下半提交或重复知识

- 严重度：P2
- 确定性：FACT（`items=[] + chunks非空` 可确定触发错误门；半提交需后续验证/写入失败或进程退出）
- 证据：`src/routes/knowledge/import.rs:33-54,634-784,1125-1288`、`frontend/src/features/knowledge/steward.tsx:787-818`、`src/models.rs:1540-1671,1817-1839`。
- 机制：handler 的首个检查只接受 `items` 非空或 `chunkedText` 非空，完全不看真正会处理的 `chunks`；紧接着又明确丢弃 `payload.items`，因为 operation_knowledge_items 已删除。正式向导仍发送 preview.items 与选中 chunks，所以合法的“只有 chunks、items 为空”预览会确定性 400，而一个无用 item 可让同一 chunks 通过。通过后，handler 先 insert active document，再循环校验并直写每个 draft Chunk；没有 transaction、import/candidate id、唯一 provenance 或幂等 outcome。第 k 条失败会保留文档与前 k-1 条，重试再建一整套。旧 JSON chunks 与 fallback blob 不写 create revision；fence 路径虽补 revision，但失败被视为 non-fatal。PDF、图片和 worker 复用的 `ingest_chunked_text` 也采用先文档、后逐条 Chunk 的相同多写协议。
- 影响：正式导入向导会因模型是否顺带生成已废弃 items 而随机可用；中途失败时 UI 只显示整体错误，却已留下 active 空文档或部分待审 Chunk。运营重试会制造重复文档/知识，后续可分别被核验并进入召回；缺失 revision 的导入行又无法从审计链识别、去重或精确清理。虽然 Chunk 被强制 draft+needs_review，避免了直接生产放行，但数据完整性和可恢复性没有保证。
- 建议：删除 items 准入与字段，按 `chunks/chunkedText` 的真实闭集验证；preview 生成 immutable import candidate id/hash，apply 对该 id 原子 claim 并幂等返回 committed outcome。文档、全部 Chunk、create revisions、catalog intent 和 candidate 终态在事务中提交；若允许部分采纳，应在提交前冻结所选 id，并为每项持久化明确 outcome，而不是因循环异常隐式部分成功。PDF/图片/worker 统一复用同一提交服务。增加 items 空而 chunks 非空、无 chunks、第二条失败、revision 失败、崩溃/响应丢失重试和重复 candidate 测试。

## SR-113：通用 Chunk Patch 未锁定租户与审核字段，当前租户管理员可向其它 workspace 注入 active+verified 知识

- 严重度：P1
- 确定性：FACT（需要已认证管理员持有本 workspace 任一 Chunk id；目标 workspace id 可由业务配置、日志或其它界面获知）
- 证据：`src/routes/knowledge/wiki_edit.rs:37-117`、`src/knowledge_wiki/chunk_revisions.rs:168-351`、`src/knowledge_wiki/page_merge.rs:28-44,145-189,213-239`、`src/routes/knowledge/catalog.rs:201-239`。
- 机制：Patch 路由把客户端 `patch` JSON 原样转成 BSON，并允许客户端选择 `source=human`。revision 内核只用当前 workspace 读取原 Chunk；默认锁集仅含 chunk/type/创建与验证时间字段，不含 `workspace_id`、`account_id`、`document_id`、`domain`、`status` 或 `integrity_status`。因此 human patch 可同时写 `workspace_id=其它租户`、`status=active`、`integrity_status=verified`。最终 `replace_one` 的 filter 仍使用旧 workspace，所以会命中本租户原行，但 replacement 已带攻击者指定的新 workspace；一次成功请求即把行移动到目标租户并绕过 `/verify`。AI source 的 draft 强制不能保护 human source，actor 还来自请求体而非登录会话。
- 影响：普通 workspace 管理员可删除自己租户的一条知识并把任意内容作为已核验 active 知识注入另一个 workspace；目标租户的 catalog/召回会把它当正式事实使用。相同旁路还能改变 account/domain/父文档归属，制造跨账号可见性、孤儿文档关系和伪造审计主体。该问题比 SR-101 的“锚点命中自动 verified”更直接：不需要有效引用或父文档，只需通用 patch。
- 建议：禁止 patch DTO 表达任何身份、租户、生命周期和审核字段；服务端以 allowlist 构造 typed patch，并从 `AuthenticatedAdmin` 绑定 actor。`workspace_id/account_id/document_id/domain/status/integrity_status/provenance/locked_fields` 等由专用状态机命令修改，verify 只能走带 source gate 的审核端点。replacement 前后都断言 tenant identity 不变，并在事务/validator 层加防线。增加 workspace A→B、account 改写、human 自升 verified、伪 actor 与未知字段测试。

## SR-114：单 Chunk revision 先写历史后替换主行且 provenance 参与内容 hash，失败动作可进入历史、幂等判断恒失真

- 严重度：P2
- 确定性：FACT（孤儿 revision 需主行 replace/反序列化/网络失败或冲突补删失败；no-op 失真对无 provenance 或不同毫秒的重复 patch 可确定触发）
- 证据：`src/knowledge_wiki/chunk_revisions.rs:196-200,245-351,379-386`、`src/knowledge_wiki/page_merge.rs:241-320`、`src/models.rs:1817-1839`、`src/routes/knowledge/wiki_edit.rs:78-117,326-355`。
- 机制：内核每次先覆盖 `provenance.edited_at/source/edited_by`，而 canonical hash 的 volatile 列表不排除 provenance；即使业务 patch 与现值完全相同，after hash 也通常改变，`unchanged` 失去“内容未变”的语义，主行仍被 replace 并重复 enqueue catalog。随后代码先 `insert_one(chunk_revisions)`，再反序列化 replacement 并用 updated_at CAS 替换主 Chunk；两步不在事务中。反序列化或 Mongo replace 报错会直接返回并留下 revision；CAS 未命中虽尝试 delete revision，但删除失败只 warn，历史仍保留。revision 模型又没有 committed/attempt 状态或主行 generation，读侧会把这些行当正常 timeline/activity。
- 影响：管理员可能收到失败/冲突，却在版本历史和活动统计中看到一条从未生效的修改；rollback 又会基于这条伪历史取 patch，进一步放大 SR-104 的不可恢复性。重复提交相同内容会无意义推进 updated_at、catalog version 和审计量，制造假编辑并增加 OCC 冲突。单对象 revision 因此也不满足“不可变历史等于已提交事实”的基本契约。
- 建议：在同一 Mongo transaction 中校验 generation、写主行、写含 before/after snapshot 的 committed revision 与 catalog intent；或先以 pending intent 写入，再由带 generation 的 finalize 原子变 committed，读侧只消费 committed。hash 只覆盖规范业务内容，排除完整 provenance/审计时间；no-op 不推进主行或 catalog，但可按需记录独立 request audit。增加相同 patch 幂等、序列化失败、replace 网络错误、CAS barrier、补删失败和 timeline 过滤测试。

## SR-115：Catalog rebuild job 只有一次性 processing 状态，没有租约、崩溃回收或失败重试，持久目录可永久陈旧

- 严重度：P2
- 确定性：FACT（永久卡住需 worker 在 claim 后 finalize 前退出，或 enqueue/重建失败且之后无同文档写入）
- 证据：`src/knowledge_wiki/chunk_revisions.rs:354-375`、`src/knowledge_wiki/catalog_rebuild.rs:67-170,238-269`、`src/models.rs:2724-2743`、`src/db/indexes.rs:1652-1678`、`src/routes/knowledge/catalog.rs:58-101`。
- 机制：主写入只 best-effort insert 一个随机 job，失败仅 warn。worker 原子把任意 `queued` 行改为 `processing`，但模型没有 worker id、locked_until 或 generation，claim 也不接受 stale processing；进程在重建或终态更新前退出后，该行永不再被扫描。重建错误直接标 `failed`，后续也没有 retry/backoff/reconciler。文档 update 的 matched_count 未检查，文档已删除时仍可把 job 标 done。job 之间也没有 `(workspace,document,generation)` 合并或顺序护栏，旧 job 可以在新 job 后再次重建并推进一个无意义版本。
- 影响：`catalog_summary_persisted` 是正式端点和诊断页的数据源；一次 enqueue 故障、worker 崩溃或瞬时数据库错误即可让某文档长期保持旧快照/null，而主 Chunk 已成功提交。系统既不会自动恢复，也没有 reliable freshness/generation 告知读侧该快照落后；运营看到的 persisted catalog 与实时召回事实分叉。
- 建议：把 catalog 更新建模为每文档单调 generation 的 durable intent；主 Chunk transaction 同时 upsert desired generation。worker 用 `(queued|expired_processing)→processing` lease claim，携 worker/token/locked_until，finalize 时按 token CAS；失败按上限重试并保留 terminal reason，reconciler 比较 desired/applied generation。检查文档 matched_count，删除走 tombstone。增加 claim 后崩溃、失败重试、多 worker、乱序 generation、enqueue 故障和删除文档测试。

## SR-116：知识成交追认按裸 contact_wxid 合并同 workspace 多账号日志，某账号成交会抬高其它账号知识置信度

- 严重度：P2
- 确定性：FACT（串扰需同 workspace 的两个 account 复用同一 contact wxid，且其中一个存在已核实成交）
- 证据：`src/knowledge_wiki/gap_signals.rs:916-1128,1131-1191`、`src/models.rs:2746-2767`、`src/db/indexes.rs:396-451`、`src/db/indexes.rs:86-112`。
- 机制：30 天 usage logs 只按 workspace 全量读取，虽然每行持久化 `account_id`，成交追认却构造 `HashMap<contact_wxid, logs>`，丢弃 account。随后 Contacts 查询也只有 `workspace_id + wxid`；数据库实际允许并以 `(workspace_id,account_id,wxid)` 唯一保存多行。循环任一 contact 的 confirmed deal 都会把该裸 wxid 下所有账号的最近三条 usage log 标为 Hit，最终再按 chunk id 回写整个 workspace 的 dynamic_confidence。代码注释称按“单个 contact”归因，但实现没有使用持久化的 account identity。
- 影响：账号 A 的 staff/payment verified 成交可以给账号 B 最近使用的知识加正向样本，即使 B 的真实用户反应是负向或删失；多笔成交还会取并集持续强化。置信度参与知识排序，因而跨账号污染会改变后续召回与销售回复，并让 `deal_attributed_hits` 观测无法指出贡献来自哪个账号。这与 SR-050 的发送台账归因漏 account 是不同集合与不同决策回路。
- 建议：所有分组、Contacts 查询和归因键统一使用 `(workspace_id,account_id,contact_wxid)`；按 usage log 自带 account 批量加载对应 contacts，并仅把该 contact 的成交窗口应用于同 key 日志。dynamic confidence 若是 account-specific，应持久化账号维度统计；若知识是 workspace shared，也应先分别算证据再按明确策略聚合。增加同 wxid 双账号、相反 outcome、单账号成交和共享 Chunk 的隔离测试。

## SR-117：自动摄取 Worker 不认领 source 也不校验 generation，多副本和并发更新会重复导入并用旧结果覆盖新 checkpoint

- 严重度：P2
- 确定性：FACT（重复导入需 worker 开启且两个实例/重叠轮次同时处理同一 due source；旧结果覆盖需抓取期间更新 URL 或发生成功/失败竞态）
- 证据：`src/knowledge_wiki/ingest_worker.rs:44-89,105-164,274-397`、`src/routes/knowledge/sources_meta.rs:940-1047`、`src/models.rs:2784-2810`、`tests/ingest_worker_smoke.rs:279-345`。
- 机制：每轮先把所有 active/failing source 读成快照，再逐条判断 due、GET 和导入，没有 `findOneAndUpdate` claim、lease、worker token 或 source generation。两个实例会同时看到相同 last hash，均调用非幂等 `ingest_chunked_text`，之后各自写 checkpoint。success/failure 更新 filter 只有 `source_id`，不含 workspace、旧 URL/updated_at/status 或 token；管理员在请求途中改 URL并清 checkpoint 后，旧请求仍会导入旧内容，再把旧 ETag/hash写回新 URL 行。并发失败用 stale snapshot 计算 `failure_streak+1` 后 `$set`，会丢计数；旧 success 还能把另一个 worker刚标 failing/disabled 的行重新设 active。测试只覆盖串行两轮相同 hash去重，不覆盖重叠执行。
- 影响：水平扩展、手工触发重叠轮次或慢源即可重复创建文档/Chunk；每份草稿之后都可独立被核验并进入召回。URL 修改后 UI 显示新地址，却可能携带旧地址 checkpoint并漏掉首次新内容；成功/失败状态和禁用判断也取决于竞态最后写者。SR-112 描述单次导入的非原子性，本条描述 source 调度层缺失执行所有权与 generation。
- 建议：为 source 增加 generation、next_run_at 与 lease fields；worker 原子 claim due source，并把 `(source_id,generation,content_hash)` 作为不可复用 import candidate/唯一 provenance。抓取、提交和 checkpoint finalize 全部带 token+generation CAS；URL/status 更新递增 generation并使旧 lease finalize 失败。failure 用原子 `$inc` 或 token finalize，禁用采用明确状态机。增加双 worker barrier、慢请求期间改 URL、成功/失败竞态、lease 回收和响应丢失重试测试。

## SR-118：正式知识诊断页误读两个 Catalog 响应包络，目录覆盖卡在有数据时仍稳定显示 0/0

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/knowledge/catalog.rs:34-101`、`frontend/src/features/knowledge/steward.tsx:1833-1836,2117-2179,2210-2245`。
- 机制：persisted 端点返回 `{documents:[...]}`，正式前端却声明 `CatalogPersistedView {total?,items?}` 并以 `catalog.total ?? catalog.items.length ?? 0` 计数。live 端点返回 `{item: catalog}`，前端却按 `{total?}` 读取顶层 `catalogLive.total`。HTTP 均为 2xx，`safe()` 不会报错，两个对象都被视作成功加载，最终持久化数、实时数和偏差分别固定为 0、0、0。类型断言只在前端抹掉真实响应形状，没有运行时解析或共享契约阻止漂移。
- 影响：正式“目录覆盖”卡会把有大量实时知识、缺失持久快照或严重 rebuild backlog 的系统展示为 0/0 且偏差 0，恰好隐藏 SR-115 类故障；用户也不会看到“数据加载失败”横幅，因为请求本身成功。立即扫描后重新 load 仍显示相同假健康结果。
- 建议：用共享生成契约统一两个 endpoint；最小修复是 persisted 读取 `documents.length`，live 读取 `item.items/chunks` 的规范计数（或后端都返回明确 `total`）。前端使用 schema parser拒绝错误包络并把未知形状显示为 unavailable，不能回落 0。增加真实 handler fixture→ObservabilityDashboard 的契约测试，以及非零、persisted lag、空库和错误 shape 测试。

## SR-119：知识日报的自动调度、知识可见域与 Prompt 租户作用域彼此不一致

- 严重度：P2
- 确定性：FACT（定时遗漏非默认租户需显式启用 Digest worker；共享知识遗漏与默认 Prompt 混用在任意按需生成中可达）
- 证据：`src/knowledge_digest/mod.rs:39-60,142-184,369-401,476-565,743-790`、`src/agent/knowledge_router.rs:1047-1068`、`src/prompts.rs:454-473`、`src/routes/knowledge/digest_inbox.rs:33-74,86-121`、`tests/digest_cross_tenant_scope_integration.rs:5-28,60-142`。
- 机制：唯一常驻 worker 每天只调用一次 `generate_today_digest(default_workspace_id, default_account_id)`，不会枚举其它 workspace/account；非默认租户只能等管理员打开页面或手动重算才同步生成。即使调用方正确传入当前租户，Chunk 健康扫描仍用精确 `{workspace_id,account_id}`，而生产召回的真实可见域是该账号加 `account_id=null` 的 workspace 共享知识，因此共享 Chunk 的缺证据、待审和陈旧草稿不会进入任何账号日报。两个 Digest Prompt 又无视函数已有的 workspace 参数，固定从 default workspace 读取 active 模板；`load_prompt` 不会跨租户回退，只在指定 workspace 无模板时回退代码内置默认，所以这是明确把默认租户自定义 Prompt 用于其它租户数据。已有跨租户测试只断言报告最终写入正确 workspace，没有断言调度枚举、共享 Chunk 或 Prompt 来源。
- 影响：非默认租户无法获得设计承诺的每日主动日报；所有租户的账号日报都可能漏掉实际会被联系人召回的共享知识问题。非默认租户按需生成时还会由默认租户的摘要/分类策略决定卡片内容与优先级，默认租户修改 Prompt 可静默改变其它租户治理结果。LLM 调用日志归错 workspace 的通用问题已记录为 SR-018，本条不重复该审计缺口。
- 建议：建立显式的 Digest schedule scope，按全部启用的 `(workspace,account)` 生成并记录每个 scope 的 run；所有分析器复用生产知识可见域 helper（`account_id=null OR current`），同时区分共享知识与账号证据的统计归属。Prompt loader 必须接收当前 workspace，并把真实模板 id/version 写入报告。增加双 workspace/双 account、共享 Chunk、不同租户 Prompt 和定时枚举测试。

## SR-120：Digest 的可配置 token/call 预算是死配置，所有生成固定使用 24000/8

- 严重度：P2
- 确定性：FACT
- 证据：`src/config.rs:291-304,678-695`、`src/knowledge_digest/mod.rs:743-757,794-800`、`tests/knowledge_digest_budget_smoke.rs:1-55`。
- 机制：配置层公开并解析 `KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET` 与 `KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS`，测试配置也会赋不同值；但 `generate_today_digest` 构造 `RunBudget` 时硬编码 `24_000` 和 `8`，全仓没有读取这两个配置字段的生产代码。报告的 `budget_snapshot` 因而忠实记录硬编码值，让运维看不出环境配置从未生效。现有 smoke 测试直接构造同样的常量，只证明 `RunBudget` 自身，不验证配置接线。
- 影响：运营把预算调低以限制成本时，单次日报仍可消耗到 24000 token/8 calls；调高以容纳更多 block 摘要时又会按旧上限提前 partial。配置、运行行为和报告快照三者表面一致但实际脱钩，容量规划与成本闸均不可信。
- 建议：从 `state.config` 构造预算并在启动时校验正数/合理上限；若需要区分定时与手动生成，应显式定义两套配置而不是绕过现有字段。增加非默认小预算触发、较大预算放行以及报告 snapshot 等于有效配置的端到端测试，并删除只锁硬编码常量的假覆盖。

## SR-121：Digest 重算失败会用空卡覆盖当日成功报告，所谓 partial 也从不保留部分结果

- 严重度：P2
- 确定性：FACT（覆盖已有成功报告需同日 `force=true` 重算或后续定时重算失败）
- 证据：`src/knowledge_digest/mod.rs:775-864`、`src/routes/knowledge/digest_inbox.rs:77-121`、`.kiro/specs/knowledge-digest-workstation/requirements.md:19-21`、`.kiro/specs/knowledge-digest-workstation/design.md:332-347`。
- 机制：四路分析与最终 compose 全部在内存串行完成，只有整段结束后才执行一次 report upsert；途中没有持久化 partial cards/checkpoint。任一错误都把 `cards` 直接替换为空数组，BudgetExceeded 虽命名 `status=partial`，实际也固定保存零张卡。upsert 对同 `(workspace,account,date)` 无条件 `$set cards/status/generated_at`，因此强制重算中的瞬时 LLM、Mongo 或预算错误会覆盖当天已有的成功报告；`dismissed_card_ids` 虽被保留，但其目标卡已消失。设计与需求却明确承诺“已写 partial 报告保留”。
- 影响：运营点击重算可能把可用日报变成空白失败页，且无法回看或继续处理刚才的卡片；`partial` 状态不能说明保留了什么。失败重试不再以最后成功版本为基线，瞬时故障转化为当日治理结果的数据丢失。
- 建议：日报使用 immutable generation/run：先写 processing run，成功后原子切 current；失败 generation 保存 error 与中间结果，但不得覆盖 last-success。若承诺 partial，就在各 analyzer/compose 阶段持久化可复核的 partial payload；否则去掉 partial 语义。响应同时暴露 current success 与 latest attempt。增加成功后强制重算失败、预算中断、进程退出和恢复 last-success 测试。

## SR-122：Knowledge Task 没有租约、崩溃回收或幂等 step，执行中断会永久 running 或重复副作用

- 严重度：P2
- 确定性：FACT（永久 running 需 pending→running 后进程退出/任一后续写报错；重复副作用需人工重置或未来回收后重跑）
- 证据：`src/knowledge_task/mod.rs:143-208,225-362,516-665`、`src/models.rs:4973-5009`、`src/db/indexes.rs:1436-1462`、`src/routes/knowledge/chat.rs:2137-2192`。
- 机制：worker 先普通 `find_one(status=pending)`，拿到进程内 session mutex 后才用 `_id+pending→running` CAS；模型只有 started/finished 时间，没有 owner、claim token、heartbeat、locked_until、attempt 或 generation。所有后续 tick 只扫描 pending，running 永不回收。step 的实际副作用（创建 Chunk、写 revision、dismiss report）又先发生，随后才 `$push completed_steps`、写 progress turn和最终状态；这些写中任一失败会让已生效副作用没有 committed outcome。stepId 没有唯一约束或独立状态，若管理员取消/手工修复状态、或未来简单加入 running→pending 回收，重跑会再次创建 Chunk或 revision。`add_chunk` 还固定把账号级任务产物写入 workspace 共享域，扩大重放后的可见范围。
- 影响：一次进程重启或瞬时 Mongo 错误即可让任务永久显示 running，官方 API 只能 cancel，不能安全 resume/retry；运营无法判断某一步是否已经生效。贸然重试则可能产生重复待审 Chunk、重复 revision 或不一致的 dismiss。该缺口与 SR-111 的 Chat pending turn apply 是不同任务集合和 worker 协议。
- 建议：用 `findOneAndUpdate` 原子领取并写 owner/token/lease/attempt；回收 expired lease 时必须按 step outcome 恢复，而不是整任务盲重跑。每个 `(task_id,step_id)` 建 durable intent 和唯一 committed outcome，业务副作用使用该 id 作幂等 provenance，并以 token/generation CAS finalize。账号级任务默认保留 account scope，只有显式共享命令才能写 null。增加 claim 后崩溃、副作用后崩溃、进度写失败、lease 回收和 add_chunk 重放测试。

## SR-123：Knowledge Task 把大量确定失败包装成 Ok，最终会虚报全部成功

- 严重度：P2
- 确定性：FACT
- 证据：`src/knowledge_task/mod.rs:225-342,449-665,712-749`、`src/routes/knowledge/chat.rs:2090-2134`。
- 机制：run loop 只把 `execute_step` 的 Rust `Err` 计入 `failed_steps`；但各 action 为了“fail-soft”把业务失败返回成 `Ok(StepOutcome)`：缺/非法 target、Chunk 不存在、repair/tag/draft LLM 失败、空 patch、Chunk insert 失败、非法 cardId 都属于 Ok。部分分支还返回目标 chunk id，外层会把不存在或根本没修复的 id 加入 `needsReviewChunkIds`。dismiss 的数据库错误与 matched_count 被完全忽略。最终 summary 的成功数直接用 `total-failed_steps.len()`，只要这些软失败未抛 Err，task 就进入 completed 并宣告全部成功；`completed_steps[].status` 也写 `ok`，错误只藏在自然语言 message 中。
- 影响：任务总览、SSE summary 与完成事件会把“零动作成功”显示成成功，运营按 needs-review id 查找时可能得到不存在/未改变对象；自动统计无法从结构化字段识别失败。重试和故障告警也不会触发，因为系统已经持久化 completed。
- 建议：`StepOutcome` 使用封闭状态 `committed|noop|needs_manual|failed` 和 typed error，不以 Rust `Ok` 表示业务成功；只有验证 committed outcome（含真实 object/revision/matched count）才计成功。fail-soft 应表示“记录失败后继续”，不是“把失败改名成功”。summary 从持久化 per-step outcomes 聚合。增加每个缺参、LLM 错、对象不存在、insert/update 零命中与 DB 错误测试。

## SR-124：Digest cardId 未包含 account，而两个 dismiss 写路径也不筛 account，可跨账号隐藏卡片

- 严重度：P2
- 确定性：FACT（跨账号触发需同 workspace 两账号在同日生成相同 kind/title/targetRefs 的卡片；共享 Chunk 会自然满足相同 target）
- 证据：`src/knowledge_digest/mod.rs:580-605,607-716`、`src/routes/knowledge/digest_inbox.rs:124-160`、`src/knowledge_task/mod.rs:712-740`、`src/db/indexes.rs:1419-1434`。
- 机制：稳定 cardId 只哈希 `(report_date,kind,target_refs,title)`，不含 workspace/account；同 workspace 多账号针对同一共享 Chunk 生成相同卡片时 id 必然相同。日报唯一键本身正确包含 workspace/account/date，但直接 dismiss 端点只按 workspace+date+cardId `update_one`，task dismiss 只按 workspace+cardId，reportDate 还可缺省；两者均遗漏 account。Mongo 会修改任意一个匹配报告，worker 路径还吞掉错误/matched_count并固定返回成功。B05L 基于旧“随机 ObjectId 难碰撞”的判断因此不再成立：当前 id 已是可预测、跨账号确定性相同的内容哈希。
- 影响：账号 A 的管理员或派工任务可把账号 B 的卡片加入 dismissed 集合，而 A 自己的卡片仍未隐藏；刷新后表现随机取决于命中的报告。共享知识问题最容易在多个账号同时出现，正是该冲突的常见输入。
- 建议：card identity 至少绑定 `(workspace,account,report_date,canonical card payload)`，所有 dismiss filter 强制使用任务/会话中的 workspace+account+date，并检查 matched_count。更稳妥的是用 report id + card id 的复合定位，task 创建时冻结该 identity。增加双账号同共享 Chunk、直接 dismiss、worker dismiss、缺 reportDate 与 regenerate 稳定性测试。

## SR-125：对话派工没有绑定运营选中的卡片，模型可把日报任意候选或客户端任意目标提交为任务

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/knowledge/chat.rs:25-38,850-889,1537-1623,1854-2007`、`frontend/src/features/knowledge/today.tsx:157-181,251-269,618-650`、`tests/knowledge_chat_dispatch.rs:1-125`、`frontend/src/__tests__/features/knowledge/knowledge.test.tsx:86-166`。
- 机制：需求与测试把 digest dispatch 描述成“运营勾选 cards → plannedSteps”，测试还断言每个 `step.cardId` 必须属于 `selectedCards`；但生产 `ChatTurnRequest` 没有 cardIds 字段，Chat 前端发送对话时也只传自然语言和可选 Chunk attachment。dispatch 后端因此把当日日报前 20 张未 dismiss 卡片全部交给 LLM，让模型自行猜“这几张”；确认派工时正式 Chat 路径固定发送 `cardIds: []`。`chat_task_create` 又把 cards 快照当 best-effort：允许空 cardIds、未知 cardId，且若 step 自带 `targetChunkId` 就直接尊重，不验证它是否来自当前日报卡片或选择集合。唯一严格检查只有 action 闭集和步数上限。现有 Rust 测试用手写 JSON 同时构造 selectedCards 与合规 plannedSteps，没有调用生产 dispatch/create；前端测试只断言 `sessionId/plannedSteps/action` 存在，也不要求 cardIds 或目标绑定。
- 影响：运营以为确认的是所选卡片，实际可能执行同日报其它卡片，或在模型/客户端提供 targetChunkId 时对任意当前 workspace Chunk 生成 repair/retag 动作；任务详情的 cards 快照还可能为空，无法从审计中还原“确认时选了什么”。批量画布直派路径会发送 cardIds，但后端同样不验证 steps 与其一一对应，因此客户端漂移或篡改仍可越过选择边界。
- 建议：把选择集合建模为服务端签发的 immutable dispatch candidate（workspace/account/report id+generation、selected card ids、canonical steps/input hash）；Chat 请求显式传 selectedCardIds，dispatch 只向模型提供该子集。create 时要求非空 cardIds（只有明确的非卡片 action 走独立命令类型），逐步验证 cardId 属于候选、action 与卡片允许动作一致、targetChunkId 只能从服务端卡片 refs 派生，拒绝客户端覆盖。任务持久化完整候选/hash与确认 actor。增加真实 handler 双账号/未知 card/空 cardIds/target override、前端选择三张只派三张及候选过期测试。

## SR-126：Knowledge 的召回率、闭环与 Worker“红线”测试绕过真实行为并形成自证质量门

- 严重度：P2
- 确定性：FACT
- 证据：`tests/knowledge_agent_eval.rs:101-117,128-218,230-246`、`tests/knowledge_closed_loop_trajectory.rs:101-214`、`tests/knowledge_worker_behavior_integration.rs:1-68`、`.github/workflows/ci.yml:145-195`；集成 job 不阻断合并的独立 CI 问题继续归 SR-004。
- 机制：名为离线召回率评测的测试先创建期望 Chunk，再把这些 Chunk 的 ObjectId 直接写进 mock LLM 的 `open_chunk` 与 `answer.citedChunkIds`，最后仍以同一组 id 计算 hit rate；检索/排序是否找到了期望知识并不决定结果，`>=80%` 实际由测试脚本预先保证。其清理 filter 还使用不存在的 camelCase `workspaceId`，而 `OperationKnowledgeChunk` 按 snake_case `workspace_id` 存储，场景间旧数据不会被清除。所谓“维护 agent 编辑 KB→再召回”测试又直接插入 verified Chunk、直接 `$set superseded_by`、直接构造关系，只在最后一个负例调用 verify handler；它没有运行维护 Agent 的 proposal/apply、revision 或结构提交路径。Worker“红线”测试仍把生产 action 描述成占位桩，并要求非法/不存在 target、空 add/dismiss 等路径返回 `Ok`；这恰好把 SR-123 的业务失败伪装成功固化为期望行为。PBT 与预算纯函数测试对局部过滤、排序全序和计数器有价值，但不能补偿上述端到端绕行。
- 影响：CI 可以持续显示召回率、闭环轨迹和 Worker 红线通过，即使真实检索不再命中、维护 Agent 写入链断裂、revision/关系提交失败或任务继续虚报成功。维护者会把脚本自证的 80% 命中、直接数据库写后的可召回和 `Ok` outcome 误当成生产能力证据；SR-030、SR-101–108、SR-122–123 一类缺口不会让这些命名上的质量门变红。
- 建议：召回评测只向模型提供 query 与真实 catalog，不得把 expected ids 注入决策；以独立人工标注 relevant 集计算 opened/cited recall@k，并加入无关高分、同义改写和跨账号语料。闭环测试必须从真实维护入口产生 draft/proposal，经 revision、人工 verify、关系/取代提交后再调用生产 Router，对每个 committed object/hash 做断言。Worker 测试应以结构化 `committed|noop|needs_manual|failed` outcome 验证真实副作用与失败，不再把 Rust `Ok` 等同成功。所有测试先断言 fixture 清理和命中数量，避免错误字段造成污染；关键门应进入阻断合并的独立 job，SR-004 的全量软 job可保留为体检。

## SR-127：Knowledge Ask 流把 Agent 失败编码成普通 trace+close，正式界面会无答案、无错误地静默结束

- 严重度：P2
- 确定性：FACT
- 证据：`src/routes/knowledge/sources_meta.rs:617-749`、`frontend/src/features/knowledge/explore.tsx:100-154,242-287`、`tests/knowledge_ask_stream_e2e.rs:1-413`、`frontend/src/__tests__/features/knowledge/exploreNoTenant.test.tsx:49-117`。
- 机制：SSE handler 在 HTTP 200 建流后若 `answer_streaming` 返回 Mongo、LLM、解析或其它 `Err`，不会发送业务 `event:error` 或含失败状态的终帧，而是把错误包装为普通 `TraceEvent::Step {tool:"error",reason}`。转换层把所有 Step 无差别输出为 `event:trace`，channel 关闭后再输出正常 `event:close`。正式 AskView 的 trace listener 只把 payload 追加到时间线；只有浏览器原生 `error` 事件才设置错误横幅，而自定义 `close` 只关闭 EventSource 并清 `pending`。清 pending 后，实时 trace 列表本身也因渲染条件要求 `pending=true` 而消失；因此该协议会留下 `result=null,error=null` 的空白终态。现有前端测试只手工触发原生 `error`，Rust “stream e2e”五个用例又全部直调 `answer_streaming` 和内存 channel，没有调用正式 handler，故双方都不会发现这一断裂。相邻显示还把轮次写死为 `roundsUsed/3`，而生产 `MAX_ROUNDS` 和集成测试均明确为 4，进一步证明响应契约未由共享模型约束。
- 影响：模型端点、数据库或流式解析失败时，运营只看到“思考中”结束，既没有答案也没有失败原因，容易误认为空结果或重复提交；错误 trace 甚至不会留在收起后的时间线中。重复提交会再次消耗模型预算，且排障无法从 UI 区分正常 close、用户取消和 Agent 失败。4/3 轮次显示则会在合法第 4 轮时呈现超额假象。
- 建议：为流协议定义封闭终态 `answer|cancelled|failed`；Agent Err 应发送 `event:error`（结构化 code/message/retryable）或 `event:final` 的 `status=failed`，随后再 close。前端在 close 时若既无 answer 也无已处理 failure，必须 fail-closed 显示“流未产生终态”；trace 中 `tool=error` 也应立即转错误态并保留诊断。轮次分母由后端返回 `maxRounds` 或共享常量，不得硬编码。增加驱动真实 Axum SSE body 的测试，覆盖 query 解析、session workspace、LLM/Mongo Err、answer、cancel、close-before-final 与前端逐帧消费；现有 Agent 内存 channel 测试保留但改名，不能宣称覆盖 HTTP handler。

## SR-128：Knowledge 真模型能力套件允许关键能力未发生仍绿色，skip 台账也观测不到这些隐式跳过

- 严重度：P2
- 确定性：FACT
- 证据：`tests/real_llm_knowledge.rs:116-179,364-496,505-611,783-880,895-1028,1168-1277`、`tests/real_llm_knowledge_quality.rs:978-1315,1832-2082,2091-2242,2248-2648`、`tests/real_llm_smoke.rs:560-649`、`tests/real_llm_recall_benchmark.rs:760-1118,1122-1571,1607-1819`、`src/agent/knowledge_agent.rs:68-81,1038-1092,1639-1681,1803-1898`、`src/routes/knowledge/verify.rs:335-460`、`.github/workflows/ci.yml:221-249,304-335,621-718,1177-1210`、`scripts/check-skip-ledger.sh:1-49`。
- 机制：套件把“真实大模型被调用”外推成“声明的能力已被验证”，但多条成功路径没有要求目标能力实际发生。K2 声称以 30 条填充项把低置信目标 B 挤出 catalog、只能沿 A→B 关系触达；当前生产却先从 400 条候选按 query 相关度重排再截 30，且 haystack 包含正文。按生产 `text_signals/relevance_score/rank_key` 对该 fixture 复算，B 的相关度约 0.42、排名第 1，A 约 0.23，填充项约 0.19；B 已直接出现在初始 catalog。测试又允许 `open_chunk(B)` 代替 `follow_relations`，所以完全不遍历 A→B 也可通过。K3/R4.2 只要求 cited id 属于 seed，引用与问题无关的 seed 仅打印 warning，诚实弃答与 gap 闭环断言随之跳过。K7 的生产 auto-verify 对单条 LLM 失败直接 `continue` 并返回 `processed=0`；测试仍验证原行未变并绿色结束，revision 断言也只在 `processed>=1` 时执行。K10 接受 `freeform`、无 patch、`canApply=false`，只要 collection 数量不变就把“意图分类+起草”算通过。K6 视觉模型返回空 fence 时 handler 返回 200+空 `chunkIds`，测试循环零次后通过。

  质量套件把同一问题扩展到裁判层：`SkipDivergent`、`SkipInsufficientJudges` 与 `SkipCalib` 都只写 `quality.jsonl`/日志后正常返回，Q3 零 `chunkIds`、Q4 命中工具循环超时回退文案也直接 return；矩阵 job 因而显示测试通过。基础 smoke 的视觉用例同样允许 `chunkIds=[]`，唯一的 `draft+needs_review` 断言位于空集合循环内。召回基准虽然在有样本时具备有效的 reach/adopt 下限、稳定性、漂移与 `cite ⊆ seed` 硬门，但 `run_query_n_times` 的每轮瞬时失败只 `continue` 且不写公共 skip ledger；全部 case 都没有成功轮次时整套契约直接 return。维护稳定性在三阶段均没有“双侧非空”样本时跳过漂移门；gap 闭环在弃答未发生、chat 未命中 create/apply/verify 或补库后第二次 answer 不可用时也正常返回。公共 skip-rate 门只统计 `skip_ledger.jsonl`，而该文件主要由顶层 `LlmUnavailable` 宏写入，裁判无结论、空产物、循环内逐轮 skip、超时回退、handler 内吞错、创建意图未命中和 fixture 漂移都可能记为 0 skip。Q2 的 16 类 train/holdout 原子事实召回与泛化差距，以及召回基准真正取得样本后的客观阈值，是独立确定性硬门，属于有效证据；但不能把同一二进制其余 case 的“未产物/未判分”提升为能力通过。
- 影响：nightly 可在关系遍历、无覆盖诚实弃答、auto-verify 真处理、对话起草、视觉抽取、召回有效样本、知识改库或内容质量裁决完全没有发生时仍报告对应矩阵绿色；skip gate 也可能宣称“全部真跑”。维护者据此误判真实模型能力和质量基线已被验证，尤其会把“召回基准进程成功”误读为 reach/adopt 阈值已经执行，而实际上所有 query 都可能在循环内被跳过。相关 job `continue-on-error` 且仅 schedule 运行的门禁范围已在 SR-004/阶段 1 记录，本项聚焦即使测试进程绿色也没有产生所声明证据。
- 建议：每个真模型 case 持久化/返回 typed outcome（`attempted, llm_calls, branch, artifacts, assertions_run, verdict, skipped_reason`），并要求目标能力的正向见证非空。K2 先调用生产 catalog 并硬断 B 不在初始结果，再要求 `follow_relations(A)` trace；K3/R4.2 用独立相关集判断引用；K7 必须 `processed==1` 且有 committed revision；K10/Q4 必须 `intent=create_chunk` 且 patch 含要求字段；K6/Q3/基础 vision smoke 必须产出可复查 artifact。召回基准应要求最低成功 case/round 覆盖率，循环内 transient 也写统一 outcome；maintenance 至少要有规定数量的双侧非空 case，gap 闭环的 create/apply/verify 未命中应是 `inconclusive` 而非 pass。质量矩阵应区分 `pass|fail|inconclusive|infra_skip`，对每个 Q 设最低有效裁判数/判分覆盖率；公共 skip 门汇总所有未执行目标断言和 inconclusive outcome，而不只捕获顶层 `LlmUnavailable`。保留 Q2 与召回基准已有的确定性阈值门，并把 judge 只作为其附加证据。

## SR-129：Cockpit 批量自动校验发送 snake_case 参数，运营选择的阈值与抽审比例被后端静默忽略

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/knowledge/cockpit/AutoVerifyPanel.tsx:13-72,83-148`、`src/routes/knowledge/verify.rs:50-63,160-180`、`frontend/src/__tests__/features/AutoVerifyPanel.test.tsx:1-20`。
- 机制：后端 `KnowledgeAutoVerifyRequest` 使用 `#[serde(rename_all="camelCase")]`，只接受 `confidenceThreshold`、`humanAuditSampleRate` 与 `accountId`；正式 Cockpit 却发送 `confidence_threshold`、`human_audit_sample_rate`。Serde 对未知字段默认忽略，两个 snake_case 键不会报 4xx，后端因此固定回落阈值 7 和默认抽审率，而 `limit` 恰好同形所以仍生效。界面却继续显示“宽松=5/适中=7/严格=9”和“留 30%/关掉留 5%”，让请求 200 与结果计数看起来像选择已执行。现有测试只 mock 任意 200 响应并检查三堆数字，没有解析 request body，也没有调用真实 DTO。
- 影响：运营选择宽松或严格都不会改变自动核验门槛，抽审开关也不改变抽样比例；尤其“严格”实际仍按 7 分处理，可能比运营明确选择的 9 分门槛放行更多非产品类知识。审计记录只看到后端实际默认值，无法解释 UI 上的选择为何未生效。
- 建议：前端发送共享生成 DTO 的 camelCase 字段，并让响应回显服务端实际采用的 `threshold/sampleRate/limit`；UI 只按回显值描述本次运行。增加 request-body 契约测试和真实 handler 测试，覆盖三档阈值、抽审开关、未知字段拒绝或显式告警，避免 serde 静默回落。

## SR-130：Cockpit 审核对话没有把当前 Chunk 绑定给后端且读取错响应字段，所谓“只动这条”会失效

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/knowledge/cockpit/ReviewChat.tsx:145-188,306-352`、`src/routes/knowledge/chat.rs:20-40,110-116,193-201,308-322,408-454`、`frontend/src/__tests__/features/ReviewChat.test.tsx:62-107`。
- 机制：后端 `ChatAttachment` 同样使用 camelCase，请求应为 `attachments:[{chunkId}]`；ReviewChat 却发送 `{chunk_id}`。未知键被忽略后 attachment 成为空对象，`chunk_attached=None`，明确的修改请求无法可靠进入绑定当前 Chunk 的 `update_chunk` 分支，甚至可能被分类为 freeform/create。即使后端成功起草，正式顶层响应字段是 `draftPreview`，组件却只读取 `turn.patch ?? data.patch`，因此不会显示真实 patch diff；后续 `runGoLive` 只要取得 sessionId 仍会调用 apply，而运营看到的界面可能没有任何改动预览。组件文案“只动这条 · 改完仍由你放行”与实际协议不一致。现有测试手写非生产响应 `{turn:{patch}}`，也不检查发出的 attachment body，恰好同时绕过请求和响应两侧漂移。
- 影响：运营在单条审核面板里要求修改 A，后端可能追问、自由回复、起草新切片或因缺 target 报错；即便产生 A 的待应用 patch，UI 也可能只显示自然语言而隐藏字段级变化。运营随后点击放行时可能应用未被界面展示确认的 session patch，再 verify 当前 Chunk，造成“确认内容”和真实提交分离。
- 建议：使用共享 Chat DTO，发送 `attachments:[{chunkId: chunk.id}]`，只接受响应 `targetChunkId===chunk.id`、`intent=update_chunk`、`draftPreview` 非空后启用 apply/放行；将 candidate/patch hash 和 source turn 传给 apply，服务端再次校验目标。前端测试应消费真实 handler fixture，断言 request body、顶层响应字段、错误 target/空 patch 禁止 apply，以及 diff 与最终 committed revision 一致。

## SR-131：文档编辑把 `{item}` 响应包络当成扁平详情，正式保存固定请求 undefined 并准备清空原文

- 严重度：P1
- 确定性：FACT
- 证据：`frontend/src/features/knowledge/steward.tsx:51-83,173-234`、`frontend/src/lib/api.ts:52-57`、`src/routes/knowledge/crud.rs:92-135`、`src/routes/knowledge/mod.rs:228-248`、`frontend/src/__tests__/features/knowledge/DocumentEdit.test.tsx:1-145`。
- 机制：正式详情端点返回 `{item: operation_knowledge_document_json(...)}`，但通用 `api.get<T>` 只原样返回 `response.json()`，不会解包。`DocumentsView.openEdit` 却把整个响应断言为扁平 `DocumentDetail`；于是 `detail.id/rawContent/contentHash/lineIndex/sectionIndex` 均为 `undefined`。标题可从列表项回退，所以编辑表单仍能正常打开并制造“已加载详情”的假象；保存时 URL 变成 `/api/operation-knowledge/documents/undefined`，未编辑字段也被构造成 `null/[]`。后端会因无效 ObjectId 返回 400，因此当前不会真的 replace 清空；一旦只修 URL 或后端容忍该路径而未同时修包络，整替换 body 已准备好清空原文与索引。专项测试直接把 `api.get` mock 成扁平 `FULL_DETAIL`，恰好跳过真实 `{item}` 包络，并据此错误证明 rawContent 会被保留。
- 影响：正式“编辑文档”确定性保存失败，运营修改标题、摘要、来源或状态都无法提交；界面错误只表现为请求失败，难以看出根因。测试则持续提供相反的绿色保证。该路径又采用 `replace_one`，使局部修补若只处理 id 而未做 canonical 解包，可能把原文、hash 和行/章节索引清空，风险从不可用升级为数据破坏。
- 建议：为文档详情定义共享 `DocumentDetailResponse={item: DocumentDetail}` 并在调用点显式解包、校验 `item.id===listRow.id`；更根本地把元数据编辑改为 PATCH/字段白名单，原文、hash、索引和租户身份由服务端从现有行保留，不由客户端回传。测试必须消费真实响应 fixture，覆盖包络、id 不一致、缺 item、并发版本变化与只改标题不触碰原文；后端 replace 后检查 matched_count/version 或采用 OCC。

## SR-132：评审队列默认类别不可达，所谓覆盖维度下钻不筛数据且会混入归档知识

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/knowledge/shared.tsx:114-159`、`frontend/src/features/knowledge/steward.tsx:1522-1605,1667-1809`、`frontend/src/features/knowledge/index.tsx:79-85,119-125,231-290`、`frontend/src/features/knowledge/cockpit/CoverageVerdict.tsx:39-60`、`frontend/src/__tests__/features/CoverageVerdict.test.tsx:17-33`、`src/routes/knowledge/crud.rs:169-176`、`src/routes/knowledge/mod.rs:949-985`。
- 机制：`ReviewView` 初始选择 `activeCategory="needs_review"`，但唯一分类函数 `classifyChunk` 的所有返回分支只有 `contested|source_orphan|pending_verification|dependents_pending|null`，从不返回 `needs_review`；因此默认标签无论数据库内容如何都固定为空。Cockpit 五维覆盖卡点击后虽把 `dimKey` 传到 `ReviewView.initialDimFilter`，该值只用于渲染一条“下面是该维度草稿”的横幅，列表的加载、分类和 `visible` 计算完全不读取它，任何维度点击都会得到同一批结果。页面还无参数请求 `/chunks`，后端只有显式 `status` query 才过滤；分类函数同样不排除 archived，却在 UI 声称“仅展示活跃知识条目”，于是归档且缺 quote/anchor 的行会重新出现在“缺少来源”评审中。现有 CoverageVerdict 测试只断言按钮调用 `onDrillDown("pricing")`，没有渲染 ReviewView、验证默认分类、维度过滤或 status 范围。
- 影响：运营首次进入评审页会看到“待初审 0”空态，即使待审知识大量存在，必须猜测并手动切换“缺少来源/待确认”才能发现；从“定价/效果数据”等覆盖缺口下钻后看到的并非该维度知识，可能审核无关条目；已归档内容又可能混入队列，被误认为仍需处理。三个问题共同破坏队列计数、导航语义和运营对审核范围的信任。
- 建议：用一个封闭、互斥且全覆盖的服务端 Review projection/query 定义类别，默认项必须对应真实返回集合；若保留 `needs_review` 总类，应先按 integrity 聚合，再将 source 状态作为独立 facet，而不是互斥类别中的死值。下钻请求应把 canonical coverage dimension 映射为服务端可验证的字段/标签过滤并回显 effective filter；列表固定传 `status=active`，服务端也应提供专用待审端点。增加真实列表 fixture 的集成测试，覆盖每个类别可达、默认非空、五维下钻结果不同、归档排除及未知维度 fail-closed。

## SR-133：Threshold proposal 并发发布可为同一候选写入多条生效覆盖，回滚后仍有副本继续生效

- 严重度：P1
- 确定性：FACT（触发需同一 eligible threshold proposal 收到两个并发 release 请求）
- 证据：`src/evolution/release.rs:36-147,422-547`、`src/routes/evolution.rs:141-184`、`src/db/indexes.rs:1296-1354`、`src/models.rs:4741-4795,4834-4849`、`tests/evolution_workspace_scope.rs:28-54,90-128`。
- 机制：`release_threshold` 在事务开始前读取 proposal 并检查 `status="eligible_for_release"`；事务内随后无条件插入一条 `threshold_overrides`，再仅按 `_id` 把 proposal 更新为 released，没有把旧 status 放进 CAS，也不检查 update 的 matched/modified count。两个请求可先同时读到 eligible，A 提交后 B 才开启事务并基于自己的陈旧 proposal 再完成同样写入；`threshold_overrides` 只有 `(workspace_id,account_id,gate_key,released_at)` 普通索引，`source_proposal_id` 没有唯一约束，因此两条 active override 都会提交。`rollback_threshold` 又只按 `source_proposal_id+rolled_back_at=null` 执行 `update_one`，只会失效其中任意一条，随后却把 proposal 标成 rolled_back；另一条仍满足运行时 `rolled_back_at=null` 并继续生效。路由层的预读仅做 workspace 授权，不能提供提交所有权。现有测试只证明跨 workspace 过滤形状，没有并发 release/rollback barrier。
- 影响：管理员双击、请求重试或两个管理端同时确认可让一个 proposal 产生多份生产阈值覆盖与审计行；界面随后显示“已回滚”，真实运行阈值却仍可能取到残留副本。重复覆盖的 `released_at` 极接近，运行时按时间排序取值也可能在不同读中表现不稳定；审计与生产状态无法一一对应。
- 建议：在同一事务内先以 `{_id,status:"eligible_for_release"}` CAS 领取 release intent，matched_count 必须为 1；为 `threshold_overrides.source_proposal_id` 建唯一索引，并把 release outcome/id 持久化回 proposal。rollback 应按唯一 override id 更新且校验 matched_count，只有确认该 proposal 的全部生效产物已失效后才推进状态。增加双 release barrier、响应丢失重试、双 rollback、唯一键冲突与“rolled_back 后 active override 数为 0”的集成测试。

## SR-134：Planner、Cold Contact 与 Silence Signal worker 只扫描默认 workspace，其他租户即使启用进程开关也永远没有对应能力

- 严重度：P2
- 确定性：FACT
- 证据：`src/main.rs:222-225,255-264,283-292`、`src/planner/mod.rs:94-142,306-325,656-676,1317-1351,1577-1614,1785-1813,1986-2010`、`src/cold_contact_worker.rs:34-97,108-194`、`src/silence_signal_worker.rs:32-123`、`src/config.rs:60-71,471-480`；同类但独立的 Evolution 默认租户问题见 SR-085。
- 机制：Planner 只有一个进程级 `STRATEGIC_PLANNER_ENABLED` 开关；开启后 `main` 只启动一个 loop。六个扫描段 `silent/commitment/stage_stagnation/calendar/renewal/reactivation` 每次都直接复制 `state.config.default_workspace_id/default_account_id` 构造联系人、事件、记忆与产品查询，没有枚举 accounts、workspace 级 runtime flag 或按租户创建 scanner。Cold Contact 与 Silence Signal loop 同样从配置固定取得 `default_workspace_id`；前者虽会在该 workspace 内枚举所有账号，后者扫描该 workspace 的消息/联系人，但两者都不枚举其它 workspace。配置注释把 Planner daily cap 描述成“每个 account”，而实际 Planner 只有默认 account 会被访问。三类 worker 都没有可供非默认 workspace 单独启用的持久 flag；部署层一旦开启，也只是给默认 scope 开启。
- 影响：同一部署中的非默认 workspace 即使已有 managed 联系人、消息历史或符合冷启动条件的账号，也确定性不会产生 Planner follow-up、Cold Contact task 或沉默信号；非默认 account 在默认 workspace 可被 Cold Contact 扫到，但 Planner 仍遗漏。运维看到进程开关和 loop 正常、默认租户持续产出时，无法从健康状态发现其它租户的功能黑洞。
- 建议：后台 tick 应从持久租户目录枚举明确启用的 `(workspace_id,account_id)` scope，并按 scope 独立加载 profile、taxonomy、产品、配额和 leader/lease；所有事件、任务与信号携带同一 scope。若产品只支持单 workspace，应在启动时验证数据库只存在该 scope，并在 API/健康检查显式暴露限制。增加两个 workspace、每个两个 account 的候选与零串扰测试，并断言每个启用 scope 都获得独立扫描和 cap。

## SR-135：Planner 发射没有持久幂等 intent，且三个“每日上限”按 tick 重置，可重复触达并突破声明配额

- 严重度：P1
- 确定性：FACT（calendar/renewal 串行重复无需并发；其它重复与总 cap 超发需重叠 tick或多实例）
- 证据：`src/planner/mod.rs:150-181,198-232,257-272,306-405,609-639,656-773,1317-1438,1577-1710,1785-1898,1930-1952,1986-2110`、`src/cold_contact_worker.rs:108-194,260-376,455-523`、`src/silence_signal_worker.rs:59-123,126-237`、`src/db/indexes.rs:77-85,139-180,217-275`、`src/models.rs:660-725,839-857,1041-1057`、`tests/planner_silent_followup.rs:80-199`、`tests/planner_commitment_due.rs:87-318`、`tests/planner_calendar_care.rs:128-238`。
- 机制：Planner 扫描器都先 `count_documents` 检查 pending follow-up/历史事件和当日 event 数，再无条件 `insert_one` 一条普通 `kind=follow_up` 的 `agent_tasks`，随后另写 emit event；task 没有 planner intent id/dedupe key，事件 helper也不传已有的可选 `dedupe_key`。Cold Contact 在每个候选前会重新统计 live 当日事件，但仍按“计数→插 task→插 event”分步执行，task 同样没有业务 intent key，因此多副本可同时通过计数并创建不同 task。`agent_tasks` 的唯一索引只 partial 限定 `kind=outcome_aggregation`，明确不约束 follow-up。因此两个进程或重叠 tick 可同时看到“无 pending/仍有余额”并各插一条任务，daily cap 也是先计数后写，无法原子占位。单实例 Planner 也有确定性重复：calendar 与 renewal 在任务离开 pending/retry/running 后不检查当天同主题 emit；命中条件仍成立时下个 10 分钟 tick 会再次建任务。它们的 `calendar_emitted_today`/`renewal_emitted_today`（reactivation 同形）都从 0 初始化，只统计当前函数调用，所谓独立 `daily_cap=3` 实为“每 tick 3”；calendar/renewal 最终可反复发到共享常规 cap（默认 20）。Silence Signal 的 `silence_signal_daily_cap` 也只是每次 tick 从 0 开始的局部计数器，并非持久每日上限；不过其信号本身由唯一 dedupe key 约束，不会像 follow-up task 那样因同一 key 重复落库。commitment/reactivation 虽有事件时间窗去重，仍受 check-then-insert 竞态与 task/event 分步提交影响。
- 影响：同一客户可收到重复纪念日关怀、续费催促或 Cold Contact 触达；多副本还可能为 silent、commitment、stage 或 reactivation 同时生成重复任务，并突破账号日上限。任务先落库、事件后写的中间失败会让配额/去重事实与真实可执行任务分裂；反过来，事件计数不是原子 reservation，无法阻止并发超发。Silence Signal 的命名配置无法限制整日采样总量，长时间运行可远超配置值。现有串行测试只证明第二 tick 在仍有 pending task或 commitment event 时不重复，没有并发 barrier，也没有先消费任务再跑 calendar/renewal、Cold Contact 多副本或跨 tick 验证独立日 cap。
- 建议：为每次计划动作建立服务端生成的 durable intent key（至少 scope/contact/segment/business subject/time bucket或source generation），在 `agent_tasks` 上以 partial unique 原子插入；任务、emit audit 与 quota reservation 应在同一事务/幂等命令中提交。每日配额使用按 scope/segment/day 唯一的计数文档，以条件 `$inc`/transaction 原子占位，失败或取消再按明确策略释放；不能用事件事后计数或进程内变量代替。calendar/renewal 的 key 应绑定日期/纪念日或 entitlement generation，Cold Contact 应绑定账号/联系人/触达周期，Silence Signal 若确需每日 cap则应使用持久原子 reservation。增加双 tick barrier、双实例、task 完成后同日重扫、午夜换桶、任务插入后事件失败和 cap 边界测试。

## SR-136：ImportJob lease 没有 claim generation，旧 worker 可在重领后续租并覆盖新执行结果

- 严重度：P2
- 确定性：FACT
- 证据：`src/import_worker.rs:42-126,129-299`、`src/models.rs:927-968`、`src/db/indexes.rs:181-214`、`src/routes/knowledge/import.rs:263-385,390-539`、`tests/import_job_lifecycle.rs:1-188`。
- 机制：worker 以 `{status:"pending"}` 把 ImportJob 改为 running，并周期更新 `claimed_at`；超时恢复则把 `{status:"running",claimed_at<stale_before}` 改回 pending。模型没有 owner、claim token 或 generation。A 超时后被恢复为 pending，B 可再次 claim 成 running；此时仍在运行的 A 的 heartbeat、进度及 completed/failed 写都只过滤 `{_id,status:"running"}`，因此会重新命中 B 的执行，替 B 续租、覆盖 progress/result/error，甚至先提交终态。任何一次 status 变化都不足以 fencing 旧执行，因为新执行复用了同一个 running 值。现有 lifecycle 测试只验证旧 terminal write 在状态暂时为 pending 时失败，没有覆盖 B 已把状态重新置为 running 的关键窗口。
- 影响：同一导入可重复调用 LLM、浪费费用，并由旧执行或新执行中任一方非确定地决定进度、错误与预览结果；旧 heartbeat 还可能延长新 claim 的 lease。当前 worker 只生成 import preview，未直接写 Document/Chunk，因此这里不是知识库半提交；真正 apply 的跨集合问题仍由 SR-112 覆盖。
- 建议：每次 claim 生成不可复用的 owner/token 或递增 generation，并让 heartbeat、progress 与所有终态 CAS 都包含该 token；reclaim 必须以旧 token/lease 为条件推进 generation，worker 一旦续租或写入 matched_count=0 就立即停止。增加 A claim→超时→B claim 后 A heartbeat/progress/completed/failed 全部失效的 barrier 测试，并验证只有 B 可提交结果。

## SR-137：BehaviorSignal 丢失 account 身份，workspace 内同 wxid 的不同账号会碰撞并无法归因

- 严重度：P2
- 确定性：FACT
- 证据：`src/models.rs:660-725`、`src/behavior_signals.rs:31-230`、`src/webhooks.rs:557-588,771-827`、`src/silence_signal_worker.rs:59-123`、`src/db/indexes.rs:77-85,249-275`。
- 机制：Contact 的唯一身份是 `(workspace_id,account_id,wxid)`，但 BehaviorSignal 只持久化 `workspace_id + contact_wxid`，所有 builder 和查询也没有 account。inbound/outbound/silence dedupe key 分别由信号类型、contact 与 message id/时间组成，唯一索引又是 `(workspace_id,dedupe_key)`；同一 workspace 两个账号若面对相同 wxid 且 message id 或 outbound 时间相同，第二条 observation 会被唯一键当成重复静默丢弃。即使 key 暂不碰撞，存量行也无法回答信号属于哪个账号。当前消费者只聚合 workspace 级健康指标，尚无学习链消费该身份，因此问题尚未扩散为现有模型训练串扰，但持久样本已经不可逆降维。
- 影响：跨账号样本会欠计或失去归属，后续若按 account 做质量评估、策略学习或联系人回放，无法可靠隔离；运维看到的 workspace 聚合仍可能表面正常，掩盖单账号采集缺口。历史行若同 wxid 在 workspace 内属于多个账号，也不能无歧义回填。
- 建议：把 `account_id` 设为 BehaviorSignal 强制身份字段，所有 builder、调用点、查询、dedupe key 和唯一索引都纳入该维度。迁移时仅对可由消息/联系人唯一证明的行回填，歧义行隔离或标记 unknown，不应任选账号；增加同 workspace 两个 account 共用 wxid/message id/时间的双样本测试和按账号聚合隔离测试。

## SR-138：Prompt pack 探测读失败会触发四集合破坏性重置，一次瞬时错误可抹掉整租户运营配置

- 严重度：P0
- 确定性：FACT（数据破坏需首次 `find_one` 失败后，后续 delete/insert 写恢复到可执行状态；分步重置留下混合态还需中途写失败）
- 证据：`src/prompts.rs:104-162,324-429`、`src/main.rs:185-192`、`src/routes/prompt_templates.rs:58-73,385-401`、`src/routes/souls.rs:181-189`、`frontend/src/features/system-strategy/index.tsx:2684-2720`、`frontend/src/stores/strategyStore.ts:263-278`；现有 seeding 测试只覆盖正常非空/漂移/幂等路径，见 `tests/prompt_pack_seeding.rs:43-456`，系统策略测试只断言重置按钮存在，见 `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx:115-128`。
- 机制：`ensure_prompt_pack_v2` 只用一次 `prompt_templates.find_one(workspace)` 区分空库与非空库；该探测返回任何错误时，不是向上失败或重试读，而是 best-effort 写一条告警后直接调用 `reset_prompt_pack_v2`。reset 随即按顺序 `delete_many` 整个 workspace 的 `agent_souls`、`prompt_templates`、`operation_playbooks` 和 `operation_domain_configs`，再逐集合重种默认值并重绑 managed contacts。整个过程没有事务、快照、CAS、恢复 intent 或“只允许显式 reset”的授权边界。B08L 又确认正式系统策略页把同一个 reset 直接暴露为普通 ghost 按钮，点击后不弹影响确认、不要求输入 workspace/确认短语、不展示将删除的四类资源，也没有 dry-run 或快照；Store 立即 POST 正式端点。告警与破坏性写还使用同一数据库，无法充当可靠前置审计。
- 影响：一次瞬时读取错误若在后续写阶段恢复，或管理员在总控页误点一次普通按钮，就会删除该租户全部人工 Prompt、Evolution 历史/灰度链、Soul、每账号 Playbook 与 Domain 配置，并把生产行为恢复成默认包；若任一 delete/insert/联系人重绑在中途失败，则请求或启动报错但数据库已处于部分旧、部分默认或零配置的混合态。该路径既可在启动时触发、由打开 Prompt/Soul 列表触发，也可由正式 UI 一次点击确定触发。
- 建议：探测失败必须原样 fail-closed，不得与“已确认空库”合流；空库初始化使用显式 bootstrap marker/事务并只允许首次创建。显式 reset 应生成新的不可变 bundle、原子切换 active pointer，保留历史和可回滚快照；跨四集合提交使用事务或 durable saga/reconciler。完成后端原子化前，UI 至少应列出准确影响、要求高摩擦确认并先创建可恢复快照，不能以普通按钮直接提交。增加探测 read error 后四集合零写入、误点/取消零 POST、每个中间步骤失败恢复、只读列表无写副作用、人工/Evolution 数据保留与多账号联系人重绑一致性测试。

## SR-139：Prompt 语义闸看不见纯删除且强约束锚不完整，连审查器自身都可被静默削弱

- 严重度：P1
- 确定性：FACT（实际安全后果取决于管理员提交的删减内容及后续模型行为）
- 证据：`src/prompt_guard.rs:28-93,110-206`、`src/prompts.rs:1110-1387,1530-1572,1687-1712`、`src/routes/prompt_templates.rs:98-237,240-313,422-433`；现有纯函数测试见 `src/prompt_guard.rs:222-408`，集成边界见 `tests/prompt_template_redline_gate_e2e.rs:13-135`。
- 机制：第三闸的 `extract_diff` 只收集“新文中不存在于旧文的非空行”；纯删除、删除整段或只把旧文缩短都会得到空 diff，并在调用 LLM 前直接 `Pass`。代码假设这种删除已由锚闸拦住，但 `required_anchors` 只为 `user.reply.policy` 保留模式段和两条反接管短语、为 `user.review.system` 保留 few-shot；被标成 `ConstrainedEditable` 的 `user.reply.system` 与 `user.reply.task` 根本没有锚，其余安全/JSON/schema/grounding 段也不在锚集合。更关键的是第三闸自身的 `management.prompt_redline_review.system` 被归为 freely editable；只要保留任意非空内容并避开禁词，就可用纯删除跳过语义审查、削弱以后所有编辑的 judge。update/publish 都接受该 `Pass`，而现有测试只覆盖新增行判定、已列锚和模糊 JSON，不覆盖 deletion-only 或 reviewer self-protection。
- 影响：管理员无需 `force=true` 即可删除 Reply JSON 契约、单发决策约束、Reviewer grounding/隐私/反接管规则，或先删弱语义审查器再让后续危险新增获得 `violation=false`。所谓“强约束层”和“三闸 fail-closed”因此只保护少量字面锚，不保护完整安全契约；生产风险会表现为模型协议漂移、漏审、越过知识边界或承诺不存在的转交，且审计响应仍显示正常通过。
- 建议：对 old/new 做结构化双向 diff，删除和修改必须把被删旧段及新增新段一起送审；任何非等价删除不得因 diff 为空自动 Pass。为所有受保护模板建立版本化 manifest（必需 section/schema/安全 invariant 的 hash），审查器 key 自身设为 Forbidden 或走独立、不可自审的签名发布通道。服务端按 prompt key 校验封闭 schema，而非靠零散 substring；增加纯删除、行内改写、无锚 constrained key、删除 reviewer 规则、改 key 绕层级与 update/publish 全链测试。

## SR-140：Reply A/B 实际选中版本被丢弃，决策审计统一记录最高 active 版本

- 严重度：P2
- 确定性：FACT（审计错记在同 key/locale 有多条 active 且联系人落入非最高版本桶时触发）
- 证据：`src/prompts.rs:476-611`、`src/agent/decision.rs:625-667`、`src/agent/gateway.rs:5128-5187`、`src/routes/shared.rs:1191-1201`、`src/models.rs:2828-2845`；现有路由测试只覆盖桶确定性/分布，见 `src/prompts.rs:2387-2462`。
- 机制：Reply 构建时，`load_prompt_for_contact` 先按 locale 选 active 集合，再以 `hash(contact_id)%count` 返回实际 `(content,version)`；但 `decision.rs` 把三层 `user.reply.system/policy/task` 的版本分别绑定到 `_system_version/_policy_version/_task_version` 后立即丢弃。Gateway 创建 `AgentDecisionReview` 时另调用 `prompt_versions`，该函数不接 contact/locale，不复用本轮选择结果，只按 `version desc,updated_at desc` 取每个 key 的最高 active 版本并写入 `prompt_versions`。该字段随后由管理 API 原样对外展示，尽管源码注释明确声称 contact loader 返回的 version 应写入运行审计。
- 影响：处于旧版本 A/B 桶或 locale fallback 的联系人会被记录成使用最高版本，导致单次事故无法还原真实 prompt，A/B 版本效果、回归定位和发布后观测无法按真实处理组归因；内置 fallback 实际返回 `version=None` 时也可能被旁路查询伪装成数据库中的 active 版本。内容本身仍按 hash 选取，本项不是“灰度未执行”，而是执行事实与审计身份分裂。
- 建议：把本轮所有 prompt selection 作为不可变 `PromptSelectionSnapshot` 随决策链传递，至少含 workspace、key、template id/version、locale、content hash、bucket count/index 与 fallback 原因；Gateway 只能持久化该快照，禁止事后重查推断。Reviewer、Reaction、Memory 等后续 LLM 同样记录各自实际加载结果。增加两个 active 版本下两个联系人分别命中不同桶、locale fallback、代码默认 fallback、revision/review 与 API 回读的端到端审计测试。

## SR-141：Chunk WebSocket 明示事件已丢失时前端吞掉 `lagged`，已打开详情可无限期停留在旧版本

- 严重度：P2
- 确定性：FACT（触发需该 WebSocket subscriber 落后于进程内 broadcast 容量并收到 `lagged`）
- 证据：`src/routes/chunk_locks.rs:200-253,268-273`、`src/main.rs:178-181`、`frontend/src/App.tsx:23-89`、`frontend/src/features/knowledge/shared.tsx:161-192,221-232,628-730`；现有事件测试只锁定单条序列化与正常投递，见 `src/routes/chunk_locks.rs:311-352`、`tests/chunk_lock_lifecycle.rs:17-98`。
- 机制：Chunk 事件使用有界 `tokio::broadcast`。接收端一旦落后，服务端不能重放缺失的 `locked/unlocked/revised`，只发送 `{"kind":"lagged"}`，代码注释明确要求前端 reload 自愈。顶层 `useChunkEventStream` 虽把 `lagged` 纳入联合类型，却与 `hello` 一样直接 `return`，既不发全局失效事件，也不重连或拉取快照。`ChunkInspectorPane` 仅在收到带当前 `chunk_id` 的 `wikiChunkRevised` 时重新请求全量 chunks；内容没有周期轮询。锁 hook 的 60 秒 heartbeat 只重新 acquire lock，不刷新 chunk 内容，因此不能补回被丢掉的 revision。
- 影响：高事件量、浏览器挂起或客户端消费变慢后，运营打开的 Chunk 可继续显示旧正文、状态、来源与关系，且界面没有“数据可能过期”提示；后续基于旧视图发起 patch、verify、relation 等动作时，可能覆盖或误判他人已经提交的修订。服务端 workspace 过滤仍有效，本项不是跨租户泄漏；正常未丢事件路径也能刷新，本项聚焦协议已明确不可连续时缺少 resync。
- 建议：给事件流增加 workspace 单调 sequence/generation；hello 和每条事件携 current generation，客户端发现 `lagged` 或序号断裂后立即把所有 Chunk cache 标 stale，并执行一次服务端 snapshot/reload，成功后记录新 generation。最低限度也应把 `lagged` 转成 `wikiChunksInvalidated`，让 Inspector、树、评审队列和锁视图各自重拉；重同步失败必须显示持续错误而非静默恢复。增加小容量 broadcast 强制 lag、前端消费 `lagged` 后只触发一次全量刷新、刷新失败/重试和多 workspace 不串扰测试。

## SR-142：联系人共享 store 没有账号身份，切号后工作台与总控会继续展示旧账号联系人

- 严重度：P2
- 确定性：FACT（工作台已有联系人时切号即可稳定触发；迟到响应覆盖需 A/B 请求交错）
- 证据：`frontend/src/stores/contactStore.ts:4-23`、`frontend/src/features/overview/index.tsx:31-64,73-128`、`frontend/src/features/command-center/index.tsx:115-141,163-174`、`frontend/src/features/user-ops/index.tsx:153-161,209-244`、`frontend/src/stores/userOpsStore.ts:286-301,454-472`、`frontend/src/stores/accountStore.ts:16-34`、`frontend/src/__tests__/features/user-ops/roster.test.tsx:118-181`、`src/routes/contacts.rs:141-151,249-281`；现有 Overview/User Ops 测试见 `frontend/src/__tests__/features/overview/overview.test.tsx:8-43`、`frontend/src/__tests__/features/user-ops/userOps.test.tsx:169-236`。
- 机制：`contactStore` 只保存一个全局 `contacts[]/selected`，没有 owning account、per-account map、request generation 或 invalidate 动作。Overview 在 `currentAccountId` 变化时虽会重跑 effect，却只要全局数组非空就立即 return；从 A 切到 B 后，A 的联系人、托管数、覆盖率和运营流因此原样留在标为 B 的应用上下文。User Ops 虽在切号时明确重拉联系人和计数，但 `refreshContacts` 与 `loadContactCounts` 的响应同样无 account generation 校验，A/B 任一迟到响应都可覆盖当前 B 的全局列表/计数。Command Center 又直接消费同一个 `managedCount()`。只有独立 Roster 视图实现了请求序号守卫，现有测试也只覆盖该局部路径。
- 影响：管理员切号后会把 A 的联系人规模、托管覆盖、联系人摘要或运营池计数误认为 B 的实时态势；总控左栏也把旧账号运营人数与新账号、任务、素材并列成一套虚假 scope。本项聚焦列表/计数投影失真；详情异步结果进一步导致实际错写联系人的独立链见 SR-146。
- 建议：共享数据必须以 `(workspace,account)` cache key 保存，或至少同时保存 `loadedAccountId`；切号先原子清空联系人和 selected，再发带 generation/AbortController 的新请求，只有响应 scope 仍等于当前 scope 才可提交。Overview 不应以“数组非空”代替 cache identity，Command Center 也应从同账号投影读取 count。增加 A 已加载→切 B、A 慢/B 快及 B 慢/A 快、请求失败后不回显 A、切回 A 命中正确缓存的测试。

## SR-143：切换账号不会清除待确认命令，B 账号界面可确认并真实执行 A 账号计划

- 严重度：P1
- 确定性：FACT（触发需 A 账号产生 `pending_confirmation` 后切到同 workspace 的 B 并点击确认）
- 证据：`frontend/src/features/command-center/index.tsx:115-147,230-292`、`frontend/src/stores/commandStore.ts:6-23,33-65,67-132`、`frontend/src/types/index.ts:261-275`、`src/routes/management.rs:217-224,255-367,439-478`；现有前端测试只覆盖单账号按钮可见性，见 `frontend/src/__tests__/features/command-center/commandCenter.test.tsx:9-48,77-91`。
- 机制：`commandStore.commandResult` 是单个全局对象，`loadCommandData(currentAccountId)` 切号时只替换 assets/souls/pendingTasks，不清除或按账号分区 commandResult。`CommandResult` 类型和 `post_management_message` 的前端投影还不携带 `accountId`，界面无法展示或校验归属。于是 A 生成待确认计划后切到 B，页面左栏已显示 B，但旧确认条和工具计划仍存在，点击只向 `/commands/<runA>/confirm` 发送 id。后端 confirm filter 仅含 `_id+workspace+pending_confirmation`，随后明确用持久化的 `run.account_id=A` 拉工具并执行；当前 UI 账号 B 不参与校验。
- 影响：管理员以为正在 B 的作用域确认操作，实际可向 A 的联系人发消息、建任务、改配置或调用 A 的 MCP 工具；确认页没有账号标签可帮助识别。后端按原账号执行保持了数据内部一致性，却放大了人机确认语义错配：被确认的可见 scope 与真实副作用 scope 不同。
- 建议：命令结果必须携不可变 workspace/account/session/plan hash，并按账号分区；切号立即隐藏或清除旧结果、busy 与确认能力。confirm/reject 请求应提交 expected account/plan generation，后端 CAS 同时校验它们并在响应中回显实际 scope；确认 UI 显著展示账号与冻结参数。增加 A pending→切 B 后无确认按钮、伪造 expected B 被 409、切回 A 可继续，以及切号期间旧 run/load 响应不得覆盖 B 的测试。

## SR-144：MCP 密钥表单跨账号复用本地明文，切号后可把 A 的密钥保存到 B

- 严重度：P1
- 确定性：FACT（触发需在 A 表单输入未提交密钥后切到 B，再点击保存）
- 证据：`frontend/src/features/command-center/McpKeyForm.tsx:9-37,39-83`、`frontend/src/features/command-center/index.tsx:115-181`、`frontend/src/stores/accountStore.ts:16-34`、`src/routes/accounts.rs:174-202`；现有测试只覆盖单次挂载提交与提交后清空，见 `frontend/src/__tests__/features/command-center/McpKeyForm.test.tsx:8-26`。
- 机制：Command Center 始终在同一 React 位置渲染同类型 `McpKeyForm`，没有以 account id 设 `key`；账号变化只更新 `accountId/configured` props，组件内的 `key/baseUrl/saved/error` state 不会重置。管理员在 A 输入的明文因此会在切到 B 后继续显示，随后 `save()` 使用新 prop 组成 `/api/accounts/<B-object-id>/mcp-key`，却发送旧 state 中 A 的 key/base URL。后端按 URL object id 与当前 workspace 正常更新 B，无法知道该密钥草稿源自 A。
- 影响：A 的敏感凭证会被错误持久化到 B，B 后续 Management/MCP 操作可能以 A 的权限执行，A 密钥也被无意复制到另一账号记录；界面仍显示“已保存”。这不是服务端跨 workspace IDOR，而是前端把秘密草稿与目标账号身份拆开后制造的授权错配。
- 建议：以 `key={account.id}` 强制每账号独立挂载，或在 `accountId` effect 中同步清空 key/baseUrl/saved/error；保存时捕获 account id generation，响应前若 scope 改变则不更新新账号 UI。更稳妥的是在独立账号详情页编辑，并在提交确认中显示目标别名。增加输入 A→切 B 输入为空、A 保存 in-flight 时切 B、响应迟到不标 B 已保存，以及 URL/body 始终绑定同一草稿 scope 的测试。

## SR-145：扫码登录前端读取不存在的字段名，后端按文档原样返回时既不展示二维码也不启动轮询

- 严重度：P1
- 确定性：FACT（针对仓库明确声明的 MCP `login_begin` 返回契约）
- 证据：`frontend/src/features/account-management/AccountLogin.tsx:6-17,19-90,103-181`、`src/routes/accounts.rs:241-279,282-325`、`src/mcp.rs:160-212,304-375`、`docs/DEPLOYMENT-STEPS.md:178-186`；B08B 没有 AccountLogin 专项测试。
- 机制：后端注释和部署检查都定义 `login_begin` 的 structuredContent 为 `qr_data_url + login_page_url + session_id`；MCP client 只取 `result.structuredContent`，`logged_call*` 和账号 handler又原样返回，不做字段转换。前端接口却要求 `qr_code_base64 + login_page_url + login_session_id`，随后只以 `data.login_session_id` 设置 session 和启动 poll。按正式契约返回时该值为 undefined，组件把 session 置空、永不调用 poll；虽然 `login_page_url` 名字碰巧一致，渲染登录区仍受 `sessionId` truthy 守卫控制，因此链接和二维码都不可见。`qr_data_url` 也被错误读取成 `qr_code_base64`。
- 影响：“开始登录”请求可以 2xx 成功，但页面无错误地回到初始表单，管理员看不到二维码/登录页且永远不会轮询成功或触发账号同步；核心账号接入闭环确定性中断。类型断言只约束前端想象的形状，不能校验运行时 JSON。
- 建议：在后端建立 typed `LoginBeginResponse` 并把 MCP 字段规范化为稳定公开 DTO（或前端严格采用 `session_id/qr_data_url`，二选一形成单一契约）；缺 session id 必须显示协议错误，不能静默回到表单。增加真实 structuredContent fixture 的 handler→前端契约测试，覆盖二维码、login URL、pending→success、expired/canceled、缺字段 fail-closed 与同步失败提示。

## SR-146：联系人详情异步响应没有身份守卫，旧 A 的运营记忆可覆盖草稿并被保存到当前 B

- 严重度：P1
- 确定性：FACT（触发需打开 A 后在其详情请求完成前切到 B，且 A 响应晚于 B/切换落地后再保存记忆）
- 证据：`frontend/src/features/user-ops/index.tsx:230-260,305-350`、`frontend/src/stores/userOpsStore.ts:34-62,367-429,654-674`、`frontend/src/stores/contactStore.ts:4-23`、`src/routes/contacts.rs:1717-1768`；现有 Store 测试均串行等待单一联系人，见 `frontend/src/__tests__/stores/userOpsStore.test.ts:42-91,93-123,173-224`。
- 机制：打开联系人时先把全局 `selected`/草稿同步成该联系人，再调用 `loadMessages(contact)` 并并发拉消息、运营记忆、候选、复盘和健康度。该异步方法既不保存 request generation，也不在提交结果前核对 `useContactStore.selected.id` 仍等于入参 contact id；任一旧请求完成都会无条件覆盖全局 `operatingMemory/memoryDraft/...`。因此 A 请求未完成时切到 B，A 晚到结果可把 A 的四组记忆填入当前显示为 B 的表单。随后 `saveOperatingMemory` 重新读取当前 `selected=B`，却读取这份全局 `memoryDraft=A`，并向 `/contacts/B/operating-memory` 全量 PUT 四组。后端只按 URL 中 B 的 id 解析真实身份并正常写 B，没有也不可能从正文识别它源自 A。
- 影响：运营快速切联系人或切账号后，可在没有任何错误提示的情况下把 A 的身份、痛点、关系状态、产品意向和下一步策略覆盖到 B；这些持久记忆会继续进入 B 的 Reply/Planner/Review 上下文，造成错误个性化、触达与审计。相同迟到响应还会让消息、候选、复盘和健康度显示错人，但本项严重度基于可闭合的持久错写链。
- 建议：把详情加载结果封装为带 `(workspace,account,contact,requestGeneration)` 的 snapshot；切联系人先清空详情态，只有 generation 与当前 selection 同时匹配才提交。保存时正文也必须携 source contact/version，后端以 expected account/contact/memory version 做 CAS，禁止仅靠当前全局草稿与 URL 重新拼身份。增加 A 慢/B 快、B 慢/A 快、切账号、卸载后响应、A 迟到后点击保存以及服务端 expected identity/version 冲突测试。

## SR-147：通讯录把已识别的系统账号保留为可勾选候选，batch-enable 后端也不执行真人准入校验

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/user-ops/RosterView.tsx:100-121,267-299,305-337`、`frontend/src/__tests__/features/user-ops/roster.test.tsx:248-266`、`src/routes/contacts.rs:514-586,761-960`、`src/models.rs:3427-3447`、`src/webhooks.rs:1062-1089`。
- 机制：Roster API 已把微信系统号/公众号/群等判定结果下发为 `isNonHuman=true`。前端仅将它们折叠到“系统账号”区域，展开后的卡片仍沿用与真人完全相同的 `toggle`，唯一禁用条件是 `agentStatus=managed`；提交候选时又丢弃 `isNonHuman`。后端 `BatchEnableCandidate` 没有该字段，循环只用 `is_self_account` 拦当前账号自身，从不调用已有且供 Webhook、联系人列表共用的 `is_operatable_person`。因此 `fmessage`、`weixin` 等已明确识别的系统号可以被 upsert 为 managed，并创建 `initial_profile` 任务；现有测试只断言它们默认折叠、展开可见，没有断言不可选择。
- 影响：系统账号会污染 managed 联系人、运营池计数、画像任务和 LLM 成本，并成为后续按 managed 扫描的 Planner/任务候选；即使某些系统号最终无法发送，运营仍会看到“已加入、画像生成中”的成功假象。前端识别事实与后端准入事实分裂，也允许任意客户端绕过折叠 UI 直接提交已知非真人 wxid。
- 建议：服务端必须以 `is_operatable_person` 和账号自身检查作为唯一强制准入门，逐项返回 `enabled/already/rejected_non_human/rejected_self`；不得信任客户端布尔值。前端系统账号卡应不可勾选并解释原因，或完全不进入可操作集合。增加 `fmessage/weixin/gh_/@chatroom/@openim` 的 handler 测试，断言零 Contact、零 task、明确 rejected 结果，并覆盖混合批次与伪造 `isNonHuman=false`。

## SR-148：Playbook 编辑身份不随账号切换，B 页面可继续修改或设默认 A 的生产方法论

- 严重度：P1
- 确定性：FACT（触发需在 A 选择一条 Playbook 进入编辑后切到 B，再保存或设为默认）
- 证据：`frontend/src/features/user-ops/index.tsx:236-244,356-383`、`frontend/src/stores/userOpsStore.ts:62-69,443-451,872-893,972-1015`、`frontend/src/features/user-ops/legacy.tsx:1037-1217`、`src/routes/playbooks.rs:148-234`；现有 User Ops 测试把 Playbook 面板整体 mock，Store 测试也不覆盖切号/编辑，见 `frontend/src/__tests__/features/user-ops/userOps.test.tsx:26-44,247-293`、`frontend/src/__tests__/stores/userOpsStore.test.ts:1-237`。
- 机制：切账号 effect 只重拉新账号的 `playbooks`，不清除或分区 `editingPlaybookId/playbookDraft`。A 中点击编辑后，这两个全局值会跨切号保留；B 页面右侧仍显示 A 的正文和“保存修改/设为默认”。保存请求虽然在 body 填入当前 accountId=B，但后端 update 先按 `_id+workspace` 找 A 的资源，完全忽略 payload.account_id，并按 A 的 existing account 维护默认集合，因此会直接原地改写 A。`set-default` 更只发送旧 id，后端明确按该 Playbook 自身的 `account_id=A` 清默认并激活 A。新账号列表的迟到响应还可能把 A/B 列表互相覆盖，但即使请求严格有序，旧编辑 identity 仍稳定可触发。
- 影响：管理员在视觉上已切到 B，却可无提示修改 A 的生产 Playbook 或切换 A 的默认方法论；该文档被运行时直接消费且没有 draft/publish 隔离（SR-070），所以保存立即影响 A 的客户决策。body 中显示的 B accountId 不能提供保护，审计也无法解释操作者为何在 B 上下文改了 A。
- 建议：Playbook 列表、编辑 id、草稿、生成/优化文本与请求 generation 全部按 account 分区；切号原子清空编辑态，提交必须携 expected account + immutable resource version。后端 update/set-default/optimize filter 同时校验 `_id+workspace+account_id+version`，不匹配返回 409；确认 UI 显示目标账号。增加 A 编辑→切 B 后空白草稿、旧 id 保存/设默认均 409、迟到列表不覆盖、切回 A 恢复正确草稿及并发版本冲突测试。

## SR-149：人工标签编辑草稿不绑定联系人，切换后可用 A 的草稿覆盖 B 的权威标签

- 严重度：P1
- 确定性：FACT（触发需在 A 打开人工标签编辑、修改但未保存，随后切到 B 并点击保存）
- 证据：`frontend/src/features/user-ops/TagTrustPanel.tsx:21-48,65-79`、`frontend/src/features/user-ops/cockpit/ObserveView.tsx:47-50,106-109`、`frontend/src/features/user-ops/cockpit/CockpitPanel.tsx:86-89,134-159`、`frontend/src/stores/userOpsStore.ts:691-708`、`src/routes/contacts.rs:1374-1442`；现有组件测试只覆盖同一联系人内编辑保存，见 `frontend/src/__tests__/features/user-ops/tagTrustPanel.test.tsx:28-35`。
- 机制：`TagTrustPanel` 的 `editing/draft/expandedTag` 都是组件本地 state，没有以 `contact.id` 设 key，也没有在 contact prop 变化时重置。Cockpit 在同一 React 位置持续渲染该组件，故从 A 切到 B 后，界面联系人已变成 B，但编辑框仍保留 A 的草稿。点击保存只把无身份的 `tags[]` 交给回调；`saveManualTags` 此时重新读取全局当前 `selected=B`，向 `/contacts/B/manual-tags` 发送 A 草稿。后端按 URL 中 B 的 id 正常执行权威覆盖、trim/去重和审计，无法知道草稿源自 A。
- 影响：B 的人工权威标签可被整组替换成 A 的标签，进而改变 B 的运营列表、Prompt 标签上下文和人工判断依据；界面没有目标身份确认或版本冲突提示。与 SR-146 的全局异步记忆污染不同，本项不依赖网络响应交错，只依赖组件本地编辑态跨联系人存活。
- 建议：以 `key={contact.id}` 重挂标签编辑器，或在 contact id effect 中同步取消编辑并清空 draft/展开态；保存回调必须捕获并提交 source workspace/account/contact/version，服务端以这些 expected identity 与 `manual_tags_updated_at` 做 CAS，不能在点击时重新读取当前 selected 拼接目标。增加 A 编辑→切 B 后编辑器关闭/草稿清空、强行提交旧 identity 返回 409、切回 A 恢复策略及保存中切换联系人测试。

## SR-150：Guide 预览异步结果不绑定当前联系人，B 页面可确认并实际应用 A 的候选

- 严重度：P1
- 确定性：FACT（触发需为 A 发起预览后在响应前切到 B，A 响应随后返回并由管理员点击确认）
- 证据：`frontend/src/stores/userOpsStore.ts:58-59,367-396,728-789`、`frontend/src/features/user-ops/cockpit/ConfigureView.tsx:291-337`、`frontend/src/features/user-ops/index.tsx:192-206,305-350`、`src/routes/guides.rs:204-315,516-536`、`src/routes/shared.rs:1063-1081`；现有健康投影测试只串行验证单联系人预览，见 `frontend/src/__tests__/stores/userOpsHealth.test.ts:42-69`。
- 机制：发起预览时 Store 捕获 A 的 account/contact 并发送请求，但响应回来后不校验 request generation、当前 selected 或返回体的 `accountId/contactId`，直接把 `guidePreview=A` 和其 health 写入全局状态。切到 B 时 `hydrateSelected(B)` 会先清 preview；然而若 A 的旧响应在其后到达，又会把 A 候选放进 B 的配置页。页面虽然拿得到 preview 自带的 contact identity，却只展示模型自报范围/摘要，不显示或核对目标联系人。点击“确认应用”时 Store只提交 `previewId`；后端正确按 preview 持久化的 workspace/account/contact=A claim并执行，完全不使用前端当前 selected=B。
- 影响：管理员在 B 的姓名、画像和配置上下文中确认一份看似属于 B 的修改，真实副作用却落到 A；候选可修改 A 的联系人、运营记忆、Playbook 或 workspace Domain runtime（后两类影响见 SR-093），响应返回后还会把 A 的 memory/health 写进当前 B 视图。后端忠实执行冻结 preview 保证了内部一致性，却没有保证人机确认时可见身份与执行身份一致。
- 建议：预览请求和结果必须携不可变 `(workspace,account,contact,requestGeneration,candidateHash)`；响应只有在当前 scope/generation 相同才可展示。确认 UI 显著显示目标联系人/账号，提交 expected scope/hash，后端 claim filter 同时校验并在冲突时返回 409；切联系人立即取消或隐藏旧候选。增加 A 慢/B 快、A preview 在 B 页不可见、伪造 expected B 拒绝、切回 A 可确认、确认中切换及响应回填不污染 B 的端到端测试。

## SR-151：Cockpit 的“运营风格模板”下拉只改前端字符串，启用与保存都不提交所选 Playbook

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/user-ops/cockpit/ConfigureView.tsx:95-98,131-143,254-287`、`frontend/src/features/user-ops/index.tsx:324,328-348`、`frontend/src/stores/userOpsStore.ts:356,367-395,524-538,564-578,630-646`、`src/models.rs:3418-3423`、`src/routes/contacts.rs:963-1029`、`src/agent/decision.rs:1035-1073`；现有 ConfigureView 测试不选择 Playbook 或断言网络请求，见 `frontend/src/__tests__/features/user-ops/configureView.test.tsx:34-130`。
- 机制：画像页把下拉标为“运营风格模板”，选择时只调用 `setSelectedPlaybookId` 改一份 Zustand 字符串，并据此切换页面底部的方法摘要。对于未托管联系人，“加入 Agent 运营”调用 `enableAgent` 时 body 只含 `humanProfileNote`，没有发送后端已支持的 `playbookId`；对于已托管联系人，页面提供的画像、备注和特别指令保存请求同样都不携 Playbook，也没有独立绑定端点。后端启用逻辑只有收到 `playbookId` 才解析所选方法，否则使用账号默认；运行时随后只读联系人已持久化的 `playbook_id`，不会读取前端状态。
- 影响：管理员可在正式 Cockpit 看到下拉与所选方法摘要均已变化，却无论点击加入运营还是任何保存按钮都不会让该方法生效；新联系人继续绑定账号默认，已托管联系人继续使用旧绑定。界面没有“尚未保存”提示，刷新或重新选中联系人后又会从旧 `contact.playbookId` 回填，形成无错误的配置黑洞。
- 建议：明确绑定动作：启用请求必须提交当前所选 `playbookId`，已托管联系人使用独立 `PUT /contacts/:id/playbook` 或统一画像保存 DTO，并由后端校验 workspace/account、保存不可变版本和返回更新后的 Contact。下拉应显示 dirty/saved 状态，成功后以服务端响应回填；增加非默认 Playbook 启用、已托管换绑、跨账号 id 拒绝、刷新后保持和运行时实际加载所选版本的端到端测试。

## SR-152：联系人复盘请求省略账号参数，非默认账号固定按默认账号查询并可串入同 wxid 的他人记录

- 严重度：P2
- 确定性：FACT（非默认账号联系人稳定得到错误查询 scope；串入记录还需默认账号存在相同 wxid）
- 证据：`frontend/src/stores/userOpsStore.ts:400-419`、`src/routes/reviews.rs:23-73`、`frontend/src/features/user-ops/cockpit/CockpitPanel.tsx:121-125,170-188`、`frontend/src/features/user-ops/cockpit/drilldowns/ConversationReviewView.tsx:49-70,119-147`；现有 Store 测试反而固定断言无 accountId 的 URL，见 `frontend/src/__tests__/stores/userOpsStore.test.ts:61-68`。
- 机制：联系人详情加载复盘时请求 `/api/decision-reviews?contactId=<id>&limit=20`，不带当前账号。后端先把缺失的 `accountId` 固定回退为进程 `default_account_id` 并写入查询 filter，之后才按 contact id 查出目标联系人的 wxid，再以该 wxid 过滤复盘；它没有把联系人自身的 `account_id` 覆盖进 filter，也不校验 query account 与联系人归属。于是非默认账号 B 的联系人会在 `{workspace,account=default,contact_wxid=B.wxid}` 下查询：通常返回空；若默认账号 A 中恰有相同 wxid，则返回 A 的复盘。结果直接进入常驻判断条与会话复盘下钻。
- 影响：非默认账号管理员会稳定看到“尚无决策记录”或缺失复盘，判断条、风险与自治依据失真；同一微信身份在多个运营账号出现时，还会把默认账号的回复文本、评分、风险和内心独白显示在 B 联系人下，造成跨账号运营上下文混淆。workspace 过滤仍有效，本项不是跨租户泄漏，但账号级数据隔离和审计解释均被破坏。
- 建议：前端始终提交当前 `accountId`；更稳妥的是后端在存在 `contactId` 时以该联系人持久化的 account 为唯一 scope，并在同时给出 query account 且不一致时返回 400/409。复盘实体若有 contact object id 应直接按它过滤，避免用可跨账号复用的 wxid 反查。增加 default/non-default 双账号、相同/不同 wxid、伪造 accountId、判断条与下钻一致性的 handler→Store 测试，并修正当前锁定错误 URL 的单测。

## SR-153：运营池勾选态不随账号切换清空，B 账号可把 A 列表中的 wxid 纳入自己的 Agent

- 严重度：P1
- 确定性：FACT（触发需在 A 的待启用列表勾选候选后切到 B，并在 B 列表响应替换旧列表前点击批量启用；无 generation 的迟到 A 响应可延长该窗口）
- 证据：`frontend/src/features/user-ops/legacy.tsx:481-551,590-648`、`frontend/src/features/user-ops/index.tsx:153-160,236-244,268-294`、`frontend/src/stores/contactStore.ts:4-23`、`frontend/src/stores/userOpsStore.ts:286-301,454-472,474-500`、`src/routes/contacts.rs:761-960`；现有 ContactsView 测试仅覆盖单账号内按钮与展示，见 `frontend/src/__tests__/features/user-ops/contactsView.test.tsx:5-150`。
- 机制：`ContactsView` 把勾选集合保存为组件本地 `selectedWxids`，没有 account prop、scope key 或账号变化时的 reset；User Ops 切号只清全局 `selected`，保留旧 `contactStore.contacts` 并异步请求 B。于是 A 中勾选 wxid 后切到 B，在 B 请求完成前同一组件仍渲染 A 列表和旧勾选条，批量按钮继续可点。`runBatch` 从当前仍为 A 的 `contacts` 组装候选，但父回调在点击时读取新的 `effectiveAccountId=B`，形成 `{accountId:B,candidates:[A.wxid...]}`。后端只验证 B 账号存在、自身 wxid 与可选 Playbook，然后按 `(workspace,B,A.wxid)` upsert managed Contact 并创建 initial_profile 任务；它无法知道候选来自 A 的列表快照。主列表请求本身无 generation（SR-142），A 的迟到响应还可在 B 上下文重新恢复 A 列表并制造同样提交。
- 影响：运营可在 B 页面把仅属于 A 通讯关系的身份写成 B 的 managed 联系人，创建画像任务、污染 B 的联系人池与运营计数，并使后续 Planner/Agent 尝试以 B 账号运营该 wxid；若 B 实际没有该好友，系统仍会先显示启用成功并持续积累无效任务/画像，若两账号都认识同一 wxid，则会在错误账号下无提示启动自动运营。
- 建议：联系人列表与勾选集合必须绑定 `(workspace,account,listGeneration)`；账号变化时原子清空列表、selected 与勾选，并取消/丢弃旧请求。批量提交携带 source account/snapshot generation，后端逐项确认候选属于该账号最新 roster 或已有同账号 Contact，再执行 upsert；仅凭客户端 wxid 不应建立账号归属。增加 A 勾选→切 B 后按钮消失、B 请求前点击不可提交、A 迟到响应丢弃、伪造 A wxid 给 B 被拒绝，以及同 wxid 双账号仍按明确来源隔离的测试。

## SR-154：Planner 看板用领域容器更新时间冒充阶段变更时间，任意画像写入都会重置“阶段未变”展示

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/user-ops/legacy.tsx:1838-1903`、`frontend/src/types/index.ts:86-123`、`src/models.rs:200-220,3544-3560,3613-3624`、`src/routes/shared.rs:73-110,783-788`、`src/agent/domain_signals.rs:113-155`、`src/agent/gateway.rs:4531-4573,4617-4627`、`src/planner/mod.rs:59-81,1000-1044`；现有 PlannerView 测试手工只给 `domainAttributesUpdatedAt`，没有校验专用时间字段，见 `frontend/src/__tests__/features/user-ops/plannerView.test.tsx:11-83`。
- 机制：看板从 `domainAttributes.customer_stage` 取阶段值，却把 `contact.domainAttributesUpdatedAt` 当作“自何时起未变更”的时间。数据模型同时公开了更精确的 `operationStateUpdatedAt`，领域容器内还维护 `customer_stage_updated_at` 或可配置的 `<stagnation_dimension>_updated_at`，生产 Planner 正是按后者判断停滞。容器级时间戳的语义只是“任一 domain attribute 最近写入”：管理员更新 intent/relationship，Gateway 重算 value tier、写领导授权或等待请示标记，都会刷新它而不改变阶段。因此界面显示的阶段值与时间来自不同事实源；甚至真正的阶段变化若只更新专用时间戳而容器时间未同步，也可能继续显示旧时间。
- 影响：运营会把一个长期未推进的客户误认为阶段刚刚更新，或把真实阶段变化显示成更早时间，进而错误判断 Planner 为什么触发或没有触发主动跟进。后端 Planner 的实际停滞判定仍使用专用时间戳，本项不会改变任务调度，但会让解释面板与真实决策依据分裂，削弱人工复盘与告警定位。
- 建议：公开并消费与当前停滞维度同源的时间字段；默认销售域读取 `domainAttributes.customer_stage_updated_at`，通用域由后端直接投影 `stagnationDimension/stagnationValue/stagnationUpdatedAt`，前端不得从容器时间推断。若展示的是 `operationState`，则配对 `operationStateUpdatedAt`。增加“只改 intent/value tier/relationship 不改变阶段时间”、阶段真实变化、可配置停滞维度、旧数据无专用时间戳的明确降级测试，并断言 UI 时间与 Planner 候选判定使用同一来源。

## SR-155：Operations 数据不绑定账号，B 页面可立即执行或取消 A 的跟进任务

- 严重度：P1
- 确定性：FACT（A 已加载任务后切到 B，在 B 响应替换前点击即可稳定触发；A 迟到响应覆盖 B 则可重新制造窗口）
- 证据：`frontend/src/features/operations/index.tsx:181-202,241-288`、`frontend/src/stores/operationsStore.ts:6-16,19-67`、`frontend/src/types/index.ts:203-226`、`src/routes/tasks.rs:40-85,168-253`；现有专项测试只覆盖单账号挂载和端点名称，见 `frontend/src/__tests__/features/operations/operations.test.tsx:20-135,193-214`、`frontend/src/__tests__/stores/operationsStore.test.ts:12-50`。
- 机制：Operations Store 只保存一份全局 `events/tasks/decisionReviews/llmUsage/agentRuns`，没有 owning account、请求 generation 或切号清空。账号变化虽会发起 `loadOperationsData(B)`，但旧 A 数据在五路请求完成前仍可操作，A/B 任一较晚的 `Promise.all` 还会无条件覆盖当前投影。任务 DTO 本身不含 `accountId`，页面无法显示或校验归属；点击“立即复核/取消”只提交旧任务 ObjectId，当前 B accountId 仅用于动作完成后的刷新。后端两个写 handler 都只按 `_id + current workspace` 查改，不校验账号：review-now 会读取任务持久化的 `account_id=A` 并真正运行 follow-up Gateway，cancel 则直接把 A 任务写成 cancelled。因此新账号页面的可见 scope 与真实副作用账号稳定分裂。
- 影响：管理员以为正在处理 B 的待办，却可能取消 A 的合法跟进，或立即运行 A 的任务并触发 LLM、复核、Outbox 与客户发送；界面没有账号列或确认提示可识别错位。若取消命中已运行任务，旧执行继续发送的 fencing 风险仍归既有 SR-034；本项聚焦切号后错误选择了哪一个账号的任务，即使任务执行器本身完全串行也可发生。
- 建议：Operations snapshot 与 loading/error 必须按 `(workspace,account,generation)` 分区；切号原子隐藏旧数据，只有响应 generation 与当前账号一致才提交。Task/Event/Review/Run/LLM DTO 都应携并显示 accountId；写请求提交 expected account，后端以 `_id+workspace+account_id` 原子校验，不匹配返回 409。增加 A 已加载→切 B 后按钮立即不可用、A 慢/B 快与 B 慢/A 快、旧 A task 在 B scope 的 review/cancel 均拒绝、切回 A 恢复正确快照的端到端测试。

## SR-156：LLM 成本页把最近 100 条样本汇总标成总调用与总 token

- 严重度：P2
- 确定性：FACT（账号累计日志超过 100 条后必然低报）
- 证据：`frontend/src/features/operations/index.tsx:215-216,394-443`、`frontend/src/stores/operationsStore.ts:30-50`、`src/routes/tasks.rs:122-165`、`frontend/src/types/index.ts:277-301`；契约测试仅对账单条日志顶层键，不验证汇总窗口语义，见 `frontend/src/__tests__/contracts/operationsDomain.contract.test.ts:21-57`。
- 机制：前端请求 `/api/llm-usage?accountId=...` 时不传 limit 或时间窗，并把响应 `summary.totalCalls/totalTokens` 直接展示为“调用次数/总 token”。后端却先按 `created_at desc` 取默认最多 100 条（limit 仅允许 1..300），再在这批游标上累加 token、命中量和 `items.len()`；没有独立 aggregation/count，也不返回 `window/truncated/nextCursor`。因此 summary 不是账号总量，而是最近 100 条样本的局部统计，命中率同样只代表该样本。日志继续增长后，页面数值会停留在滚动窗口规模，并可能随新旧高成本调用进出窗口反向下降。
- 影响：运营会系统性低估账号累计 LLM 调用与 token 成本，无法用该面板做预算核对、趋势解释或异常检测；“总 token”甚至可能在新增调用后下降，造成成本已回落的假象。账号过滤本身正确，本项是聚合口径与产品标签不一致，不是跨账号泄漏。
- 建议：把明细分页与汇总拆开：summary 用 Mongo aggregation 在明确时间窗内全量计算，并返回 `windowStart/windowEnd/isPartial`；若产品只想显示最近 100 条，应改名并明确“最近 100 次”，同时提供可审计的日/周/月趋势。增加 101/301 条边界、分页不改变 summary、新日志进入后累计值单调、缓存命中率分母和空窗口测试；契约应锁定窗口元数据与语义而非只锁键名。

## SR-157：Campaign 报表响应不校验当前活动，迟到的 A 明细会永久显示并导出为 B

- 严重度：P2
- 确定性：FACT（触发需先打开活动 A、随后在 A 响应返回前打开 B，且 A 最后返回）
- 证据：`frontend/src/stores/campaignStore.ts:42-84`、`frontend/src/features/campaign/CampaignBoard.tsx:26-38,52-57,64-72,97-103,132-138`、`src/routes/campaigns.rs:572-686`；现有 Store/看板测试仅覆盖单请求顺序成功、失败与分页，见 `frontend/src/__tests__/features/campaign/store.test.ts:31-68`、`frontend/src/__tests__/features/campaign/campaign.test.tsx:49-109`、`frontend/src/__tests__/features/campaign/board-paging.test.tsx:24-46`。
- 机制：`openReport(id)` 立即替换全局 `selectedCampaignId`、清空 `report` 并异步调用 `loadReport(id)`；Store 只有单份 `report/loading/lastAttemptedId`，没有 request generation、AbortController 或响应 id 校验。若 A 请求未完成时打开 B，B 响应先写入后，A 的迟到响应仍会无条件 `set({report:A})`，而 selection 保持 B。看板只在 `report==null` 时按 selection 补请求，不校验后端已回显的 `report.campaignId`，所以错位不会自愈；标题、汇总、逐人 wxid/原因全部来自 A，但导出文件名由当前 `selectedCampaignId=B` 生成。
- 影响：运营会在 B 活动上下文查看 A 的客户名单、送达/拦截原因与统计，并把 A 明细下载成 `campaign-B-sends.csv`，后续复盘、对账或对外传递都可能错误归因。后端每次响应本身正确限定活动与 workspace，本项是前端把两个合法快照拼成错误身份，不构成跨 workspace 越权。
- 建议：把报表缓存按 campaign id 分区，或为每次 load 生成递增 generation；仅当 `selectedCampaignId===requestedId===response.campaignId` 且 generation 仍为当前值时提交。`loading/error/lastAttempted/page/filter` 也应绑定活动；渲染与导出前硬校验 report id，不一致时隐藏数据并重拉。增加 A 慢/B 快、B 慢/A 快、A 失败/B 成功、重复打开同活动和错位时禁止导出的 deferred-promise 测试。

## SR-158：Campaign CSV 未中和公式前缀，外部联系人昵称可在表格软件中变成活动公式

- 严重度：P2
- 确定性：FACT（利用需联系人昵称或备注以公式前缀开头，且管理员下载后用会解释 CSV 公式的表格软件打开）
- 证据：`frontend/src/features/campaign/csv.ts:5-15`、`frontend/src/features/campaign/CampaignBoard.tsx:16-23,97-103`、`src/routes/campaigns.rs:634-676`、`src/mcp.rs:603-612`、`src/webhooks.rs:1090-1147`；现有 CSV 测试只覆盖分隔符/引号/换行的 RFC 4180 转义，见 `frontend/src/__tests__/features/campaign/csv.test.ts:5-31`。
- 机制：报表客户名直接取联系人 `remark` 或 `nickname`；nickname 来自微信 roster，属于外部身份数据。`toCsv` 的 `esc` 只在值含逗号、引号或换行时加双引号并转义引号，对以 `=、+、-、@`（及制表/控制字符变体）开头的单元格不做中和。双引号只是 CSV 语法边界，常见公式型表格软件仍会把其中的公式前缀解释为表达式；实测生成结果保留 `=HYPERLINK(...)`、`+SUM(...)`、`-1+2` 与 `@SUM(...)` 的首字符。
- 影响：恶意或被污染的联系人名称可在运营打开导出文件时注入公式，伪造表格内容、诱导点击外链，并在具体客户端安全策略允许时触发外部数据访问。风险发生在导出/打开链，不会在 Web 页面渲染时执行；管理员备注也是输入源，但外部联系人可控 nickname 已足以形成可达链。
- 建议：所有导出单元格先规范化控制字符，再对去除前导空白后以 `= + - @` 开头的值前置单引号或采用明确关闭公式解释的安全导出策略；不能只依赖 CSV 引号。若需保留原文，可同时提供 JSON 导出。增加四类公式前缀、前导空白/制表、逗号包裹公式、正常负数策略、Unicode 变体和电子表格导入 smoke test。

## SR-159：自治页快照不绑定账号，切到 B 后可继续取消 A 的待发消息

- 严重度：P1
- 确定性：FACT（A 已加载可取消 Outbox 条目后切到 B，在 B 响应替换前点击即可稳定触发；A 的迟到响应还可重新制造窗口）
- 证据：`frontend/src/features/autonomy/index.tsx:39-121,391-410`、`frontend/src/features/autonomy/OutboxPanel.tsx:13-73,89-125`、`src/routes/admin_outbox.rs:60-113,116-220,250-274`；现有测试只固定单账号并断言 id-only 取消请求，见 `frontend/src/__tests__/features/autonomy/autonomy.test.tsx:10-19,97-173`、`frontend/src/__tests__/features/autonomy/OutboxPanel.test.tsx:24-59`。
- 机制：自治指标、改写列表和发件箱各自保存一份组件本地快照，没有 owning account、请求 generation、AbortController 或切号时同步清空。账号 A 数据已显示后切到 B，页面会立即发起 B 请求，却在响应完成前继续展示 A；A/B 任一迟到响应也会无条件提交。Outbox DTO 虽然后端回显 `accountId`，前端类型和表格将其丢弃，取消按钮只向 `/api/admin/outbox/<id>/cancel` 发送固定 reason。后端原子更新 filter 只有 `_id + current workspace + cancelable status`，不接受或校验当前 UI 的 expected account，因此 B 页面上的旧 A 行会被真实改成 canceled。若该行已是 `in_flight`，物理发送仍可能完成的独立 fencing 缺口继续归 SR-066；本项即使只取消 pending 行也成立。
- 影响：管理员以为正在清理 B 的待发队列，实际可撤销 A 的合法客户消息；界面既不显示条目账号，也没有二次确认帮助识别错位。自治指标、改写记录和长期成效表也可把 A 数据贴到 B 上下文，造成观测误判，但本项按可闭合的真实取消副作用定为 P1。
- 建议：自治 metrics/revisions/outbox snapshot 与 loading/error 都按 `(workspace,account,generation)` 分区；账号变化时原子隐藏旧数据，只有请求账号、响应 `accountId` 与当前账号一致时才提交。Outbox 前端必须保留并显示 accountId，取消请求提交 expected account/generation；后端 CAS 同时过滤 `_id+workspace+account_id+status`，不匹配返回 409。增加 A 已加载→切 B 后按钮立即消失、A 慢/B 快与 B 慢/A 快、旧 A id 携 expected B 被拒绝、切回 A 恢复正确快照，以及 pending/in_flight 分支测试。

## SR-160：内容资产快照不绑定账号，B 页面可审核、启停、换文件或删除 A 的生产素材

- 严重度：P1
- 确定性：FACT（A 已加载账号私有资产后切到 B，在 B 响应替换前操作即可稳定触发；A 的迟到响应还可重新制造窗口）
- 证据：`frontend/src/features/content-assets/index.tsx:64-124,179-223,412-539,541-759`、`frontend/src/stores/contentStore.ts:6-37,40-65,119-188`、`frontend/src/types/index.ts:228-250`、`src/routes/assets.rs:55-121`、`src/routes/media_assets.rs:201-255,276-351,373-490,499-564`；现有测试只固定单账号，并明确断言旧资源 id 与当前 accountId 仅一同传给 Store，见 `frontend/src/__tests__/features/content-assets/contentAssets.test.tsx:12-69,123-149`。
- 机制：内容资产列表按当前账号查询“workspace 共享资产 + 该账号私有资产”，但 Zustand Store 只有一份无 owning account/generation 的 `assets`；Shell 切号不会重挂载频道，旧 A 列表会保留到 B 请求完成，任一迟到响应也会无条件覆盖。后端列表明明回显 `accountId`，前端 `ContentAsset` 类型却将其丢弃，行上也不展示归属。编辑、审核、启停、换文件和删除动作只提交 asset id；调用时附带的当前 accountId 仅用于动作后的重新加载，不进入写请求。对应后端写 filter 全部只有 `_id+current workspace`，不校验资源账号或 expected account。因此在 B 页面点击 A 行，可真实改写 A 的正文/注入档、把草稿批准为可发送、启停、替换文件并强制重审，或物理删除记录与最后一份文件。
- 影响：运营以为在维护 B 的素材库，实际可改变 A 的 Reply prompt、禁用表达、销售文件和发送候选；错误批准/启用会让 A Agent 在后续会话中引用或发送该内容，错误删除则立即撤掉 A 的生产事实与素材。共享资产与账号私有资产混排且不显示 scope，使即使没有竞态也难以区分一次操作会影响单账号还是整个 workspace。
- 建议：Store 按 `(workspace,account,generation)` 分区，切号时原子隐藏旧快照并丢弃迟到响应；前端保留、显示 `accountId`，把共享项明确标为“全账号”。所有写请求提交 expected scope，后端以 `_id+workspace` 读取后校验 `asset.account_id ∈ {null,current account}`，对私有资源要求精确 account 匹配，并在 CAS filter 中带 account_id；共享资源应走独立的 workspace 级管理入口与确认文案。增加 A→B 切号、双请求乱序、旧 A id 在 B 下的五类写操作均被拒、共享资产显式操作，以及换文件被拒时零孤儿文件测试。

## SR-161：成交页选择不随账号切换失效，B 上下文可继续给 A 联系人追加高可信成交

- 严重度：P1
- 确定性：FACT（A 中选定联系人后切到 B，无需等待竞态即可稳定触发）
- 证据：`frontend/src/features/products-deals/index.tsx:352-410,435-535,537-718`、`frontend/src/app/Shell.tsx:196-203,243-272`、`src/routes/contacts.rs:72-103,1601-1639`、`src/routes/shared.rs:1442-1574`；现有 ContactPicker 测试只验证单账号点选，不覆盖账号变化或父级 selected 清理，见 `frontend/src/__tests__/features/products-deals/ContactPicker.test.tsx:11-27`。
- 机制：DealsTab 把选中联系人保存在父组件本地 `selected`；ContactPicker 在账号变化时只重拉自己的 `contacts`，既不通知父级清空 selected，也不校验 selected.accountId 与当前账号。Shell 切号不以 accountId 作为频道 key，因此 A 联系人的“登记成交/退款”表单持续存在。提交只向 `/contacts/<A object id>/deal-events` 发送事件内容，不携 expected account。后端 `find_contact_by_id` 只校验 `_id+current workspace`，随后按该联系人自身 account 写入；正式落库把前端可选的 `staff_confirmed/payment_verified` 事件 append 到 A 联系人，更新持有、LTV、活动圈选与后续运营依据。B 的联系人请求是否已经返回不影响这条旧选择。
- 影响：管理员在顶部已切到 B 后仍可能把成交、退款、金额、产品快照或“支付核实”写入 A 客户；这是追加式高可信业务事实，界面没有撤销入口，后续售后、价值分层、Campaign 人群和知识正向归因都会消费它。写端使用资源自身账号避免了数据落到 B，但也让可见账号与真实副作用账号静默分裂。
- 建议：选中联系人必须绑定 accountId；账号变化时父级同步清空 selected、事件、持有和表单草稿，并取消/丢弃旧请求。提交 expected account，后端按 `_id+workspace+account_id` 校验，不匹配返回 409；响应与表单显式显示账号。对 append-only 成交增加 idempotency key 和可审计 reversal/纠错流程。增加 A 选人→切 B 后表单立即消失、旧 A id 携 expected B 被拒、A/B 同 wxid 仍按 object id+account 隔离、迟到联系人/事件响应不恢复旧选择，以及 staff/payment 两档测试。

## SR-162：workspace 级决策链从单账号联系人选人却不保存发送账号，其它账号请示可落成幽灵 pending

- 严重度：P1
- 确定性：FACT（确定存在作用域错配；实际首卡失败取决于该决策人 wxid 是否也是触发请示账号的可发送好友）
- 证据：`frontend/src/features/ask-human-config/DeciderChainEditor.tsx:18-47,62-114`、`frontend/src/types/index.ts:772-794`、`src/models.rs:1245-1273`、`src/routes/domains.rs:204-240`、`src/agent/escalation/policy.rs:20-40`、`src/agent/gateway.rs:825-901`、`src/agent/escalation/mod.rs:43-139`；现有组件测试只在未设置账号的单一 mock 联系人列表中验证增删排序，见 `frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx:19-67`。
- 机制：请示策略按 `(workspace,domain)` 保存并对 workspace 内所有账号生效，但配置页的候选联系人只来自顶部当前 account。选中后 `DeciderRef` 仅保存裸 `wxid/displayName`，模型、PUT 和运行时都没有“该决策人可从哪些业务账号触达”或发送账号字段。任意账号的客户触发请示时，Gateway 直接取同一 workspace 链首，却固定用该客户自己的 `contact.account_id` 调 MCP 向该 wxid 推卡。于是从账号 A 好友列表配置的领导会被账号 B 尝试发送；若 B 不认识该 wxid，代码已先插入 B 的 pending，再因 MCP 失败返回，进入 SR-038 所述永久去重的幽灵 pending。配置页和收件箱均不展示这条发送能力约束。
- 影响：多账号 workspace 看似只需配置一次决策链，实际只有部分账号能把内部请示送达；其它账号的客户已收到“正在确认”类占位、后台也显示 pending，决策人却可能从未收到卡，同类后续请示还被去重挡住。若 wxid 恰好跨账号可达则问题暂不显现，造成上线前难以发现的账号依赖。
- 建议：先明确产品语义并选最小模型：若决策人链应 workspace 共享，则每个成员保存并验证可用发送账号集合，触发时选择已验证可达的账号且台账记录 `delivery_account_id`；若各业务号独立运营，则把策略直接下沉为 `(workspace,account,domain)`。配置保存前按所有受影响账号检查好友/发送能力并明确列出缺口；首卡仍应按 SR-038 改为可靠投递状态，未送达不得进入永久 pending 去重。增加 A 选人后 B 触发、B 无好友/有同 wxid、发送账号下线和策略迁移测试。

## SR-163：请示策略读取失败时页面用默认草稿继续开放保存，可整体覆盖正在生效的生产策略

- 严重度：P1
- 确定性：FACT（触发需 GET 失败后管理员忽略错误横幅、补一名决策人并点击保存）
- 证据：`frontend/src/features/ask-human-config/index.tsx:22-45,68-85,101-115`、`frontend/src/features/ask-human-config/policyForm.ts:3-12,21-61,64-85`、`src/routes/domains.rs:204-240`、`frontend/src/app/channels.ts:164-172`；正式设计明确把“失败后默认值可编辑可存”作为路径，见 `docs/superpowers/specs/2026-06-21-ask-human-config-page-design.md:115-119`，直接测试只覆盖纯函数默认值与联系人加载错误，不覆盖配置 GET 失败后的保存。
- 机制：页面 mount 的 GET 一旦因网络、权限或服务端暂态失败，catch 会把未知生产状态替换为 `defaultPolicy()`，同时错误横幅明确写“已展示默认值，可编辑保存”；保存按钮仍启用。管理员只需从联系人补入一人通过空链校验，PUT 就以整份默认草稿原地 `$set` 当前生产行，没有 expected version、读取基线或 diff 确认。原决策链、四个升级开关、超时、去重、每日上限和静默时段因此可在读不到旧值时被整体替换；保存后的重读失败也不改变写已提交的事实。
- 影响：一次只读故障会把普通“修配置”操作转化为盲覆盖，立即改变 workspace 全量在跑 Agent 的请示对象、升级范围与骚扰频率。运营看到“保存失败/读取失败”附近的界面状态无法证明生产实际值，旧策略也没有由该页面提供的版本或回滚入口。
- 建议：加载失败必须 fail-closed：保留不可编辑错误态和显式重试，绝不能把默认值冒充现状；“首次确实未配置”只能由成功 GET 且 `askHumanPolicy=null` 判定。保存携带 config id/version/updatedAt 或 ETag 做 CAS，展示从已知基线到候选的最小 diff；若确需灾备重建，使用单独高风险“以默认值重建”动作和明确确认。增加 GET 失败零 PUT、旧策略存在时盲保存被禁、加载后并发变更 409、PUT 成功后重读失败仍回显 committed 状态测试。

## SR-164：请示配置页禁止保存空决策链且无法清除静默时段，与后端“空即关闭”契约相反

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/ask-human-config/index.tsx:47-65,68-76,112-176`、`frontend/src/features/ask-human-config/policyForm.ts:64-85`、`frontend/src/__tests__/features/ask-human-config/policyForm.test.ts:51-74`、`src/models.rs:1245-1273`、`src/routes/domains.rs:204-240`、`src/agent/gateway.rs:834-842`、`src/agent/escalation/policy.rs:20-33`；设计契约也明确 `deciderChain=[]` 为未启用、`quietHours` 省略为全天可推，见 `docs/superpowers/specs/2026-06-21-ask-human-config-page-design.md:31-45,162-167`。
- 机制：后端模型和运行时都把空 `decider_chain` 解释为“请示通道未启用”，PUT 也允许空链；前端 `validatePolicy` 却固定报“至少配置一个决策人”，使正式 UI 无法停用通道。静默时段的三格编辑更把每个暂时空值立即写成 `0`：首次只填起点会提交 `{startHour:22,endHour:0,tzOffsetHours:0}`，已有三项逐个清空时先前清空项会回弹为 0，最终得到 `{0,0,0}` 而非删除 `quietHours`。页面文案仍宣称“三项留空=全天可推”，现有测试没有驱动该受控表单时序。
- 影响：运营无法通过官方页面关闭请示通道或恢复“全天可推”，只能改库/调 API；部分填写静默时段还会静默生成与输入意图不同的时区和窗口。管理员可能以为已停用或清空，生产却继续使用旧链/错误频控，增加漏请示或不合时段打扰决策人的风险。
- 建议：用显式“启用请示通道”和“启用静默时段”开关表达状态，关闭时分别提交空链与 `quietHours` 省略；三格保留独立字符串草稿，三项完整且合法时才构造 typed 对象，不要用 0 代替未填。后端继续作为权威并补时区合理范围。增加启用→停用、已有 quietHours 三格清空、逐格编辑不提前提交、跨午夜与时区边界的组件→请求体测试。

## SR-165：编辑当前激活模型会绕过激活确认与连通性验证，普通保存立即热切全进程生产流量

- 严重度：P1
- 确定性：FACT（真实故障后果取决于管理员提交的 endpoint、key、协议或 model 是否可用）
- 证据：`frontend/src/features/llm-providers/index.tsx:147-208,230-301,461-690`、`src/routes/llm_providers.rs:213-290,575-600`、`src/routes/mod.rs:301-309`；现有前端测试只覆盖列表、徽章和空态，见 `frontend/src/__tests__/features/llm-providers/llm-providers.test.tsx:53-93`。
- 机制：对未激活供应商，页面“激活”会明确确认“立即对所有生产对话生效”；但编辑当前 active 行时，同一表单允许直接改 format/baseUrl/apiKey/model/超时重试，底部“测试连通性”和“保存”彼此独立，保存既不要求测试成功，也不显示生产影响确认。PUT 落库后发现该行仍 active，立即用新值构造 client 并 swap 单例 `LlmRegistry`。因此看似普通配置保存实际等价于一次全进程热发布；受 SR-013 影响，副作用还不限于当前 workspace。
- 影响：一次 URL、协议、模型名或密钥误填可立即让全部主 Agent LLM 调用失败，或把所有租户后续 prompt/业务上下文送往错误端点；页面返回保存成功只能证明 client 对象可构造，不能证明远端鉴权、模型与 JSON 能力可用。独立测试按钮即使先成功，修改任一字段后测试结果也不会失效或绑定到保存内容。
- 建议：保持现有单页即可，不必另建发布系统：active 行编辑先保存为候选或至少要求对规范化 candidate hash 执行成功连通性测试，再显示 endpoint/model/diff 与影响范围确认后原子 apply；任一字段变化使测试证明失效。后端以 candidate hash/expected updatedAt 校验确认，不能只靠前端按钮顺序。增加 active 编辑未测试不可提交、测试后改字段失效、确认内容与实际 swap 一致、失败不改 DB/Registry，以及多 workspace 影响提示测试。

## SR-166：模型超时与重试字段清空不会恢复默认，页面留空提示与持久化语义相反

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/llm-providers/index.tsx:97-110,157-179,608-646`、`src/routes/llm_providers.rs:131-146,213-264,575-588`；直接测试没有保存或清空字段用例，见 `frontend/src/__tests__/features/llm-providers/llm-providers.test.tsx:53-93`。
- 机制：编辑器把 `timeoutSeconds/maxRetries/retryBaseMs=null` 显示为空，并以“默认沿用 .env / 默认 3 / 默认 1500”说明空值语义。已有自定义值被清空时，`buildUpsertBody` 却直接省略该键；后端 PUT 又只在请求反序列化为 `Some` 时 `$set`，从不 `$unset`。所以数据库继续保留旧覆盖值，重读后输入框恢复原数值；管理员无法通过正式 UI 回到运行时默认。
- 影响：为事故临时加大的超时或重试预算无法撤销，后续模型故障仍可能长时间占用 worker、扩大退避延迟与请求成本；运营会把“留空保存”误认为已经恢复默认，排障时看到的意图与真实运行参数不一致。
- 建议：用明确 nullable patch 契约区分“未修改”和“清除覆盖”：例如 `timeoutSeconds:null` 触发 `$unset`，数字触发 `$set`；前端清空必须发送 null，响应回显 effective value 与来源（provider override / env default）。增加三个字段分别设置→清空→重读、0 的合法/非法边界、active 行清空后 Registry 使用 env 值的端到端测试。

## SR-167：专职视觉模型指派未与能力和删除生命周期绑定，关闭能力或删除可静默撤掉图片处理能力

- 严重度：P2
- 确定性：FACT
- 证据：`frontend/src/features/llm-providers/index.tsx:210-260,361-452,648-662`、`src/routes/llm_providers.rs:213-321,384-447`、`src/routes/knowledge/import.rs:912-975`。
- 机制：`set_vision_active` 只在建立指派时要求 `supportsVision=true`。之后普通 PUT 可把同一行的 `supportsVision` 改为 false，却不清 `isVisionActive`；列表会同时显示“不支持图片”与“视觉模型”的矛盾状态，运行时查询又只取 `supportsVision=true`，所以该指派实际上被忽略。更直接的是 DELETE 只禁止删除文字 `isActive` 行，非文字 active 的专职视觉模型仍可通过普通删除确认被移除。两条路径都不检查是否还有可用视觉候选，也不提示图片导入/入站理解将降级。
- 影响：管理员以为只是修改能力标签或删除未激活文字供应商，实际可能让知识图片导入和运营图片理解立即改用未明确指派的备用模型，或在没有其它候选时固定失败 `visionNotSupported`；后台徽章在能力关闭后还会误报已有视觉模型。
- 建议：维持单一不变量即可：`isVisionActive => supportsVision` 且被指派行删除/关闭能力前必须显式解除或原子改派。PUT 关闭能力时可自动清指派但须在响应/UI 明示影响；删除专职视觉行应要求确认并展示剩余候选，不能把文字 active 当作唯一保护条件。建立同 workspace 至多一条视觉指派的 partial unique 或服务端 CAS，并增加关闭能力、删除唯一视觉模型、原子改派、备用回落和列表徽章一致性测试。

## SR-168：字典投影丢失终态与再激活标志，普通编辑会把两项生产语义静默清零

- 严重度：P1
- 确定性：FACT（条目原本任一标志为 true，管理员打开编辑并保存即可稳定触发）
- 证据：`src/models.rs:3092-3109`、`src/routes/admin_taxonomies.rs:90-126,201-274,319-339,438-467`、`frontend/src/features/system-strategy/index.tsx:40-72,752-768,903-990`、`frontend/src/contracts/taxonomy_entry.fixture.json:1-21`、`frontend/src/__tests__/contracts/taxonomyDomain.contract.test.ts:13-41`、`frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx:173-203`、`src/agent/taxonomy.rs:127-161,270-308`、`src/planner/mod.rs:810-947,1001-1051`。
- 机制：Mongo 模型与 PATCH 都支持 `value.isTerminal`、`value.isReactivationTarget`，运行时 taxonomy cache 也读取两值；但唯一列表/写后投影 `taxonomy_entry_json` 只下发 id、label、description、aliases、status，明确省略这两个字段。前端类型把它们声明为可选，点击“编辑”时用 `?? false` 构造草稿，保存又无条件发送两个布尔值，于是任何原本为 true 的 current 条目即使管理员只改显示名，也会被 PATCH 成 false 并立即失效缓存。契约 fixture 同样删掉了嵌套字段，而契约测试只比较顶层键，明确把 `value` 当一个整体不展开；组件测试的 active fixture也不带标志，反而断言 PATCH 应发送 false/false，把错误行为锁成绿测。该 CRUD 还直接原地改 current 行，不经过同页面提供的 publish/rollout 版本链。
- 影响：Planner 每 tick 从字典把 `isTerminal=true` 构造成停滞扫描的 `$nin` 排除集，把 `isReactivationTarget=true` 构造成唤醒扫描的 `$in` 目标集。标志被清零后，自定义终态可能重新进入漏斗停滞催进，已成交维护、冷却或行业终态联系人收到不应有的主动跟进；自定义沉默/休眠阶段又会退出再激活候选。若该 workspace 的集合因此变空，Planner 还会回落销售域写死的三终态与 `dormant_reactivation`，在非销售行业产生更隐蔽的错误扫描。操作成功响应与列表均无法让管理员看出语义已丢失，也没有可用版本回滚这次原地修改。
- 建议：`taxonomy_entry_json` 与嵌套契约必须完整下发 priority/terminal/reactivation 等所有可编辑且被运行时消费的字段；前端只在已知基线上编辑，缺字段应 fail-closed 而非默认 false，并按 dirty field 构造 PATCH。生产语义修改应创建新版本并经同一 publish/CAS 切换，至少要携 expected version 防止盲覆盖。增加 true/false 列表往返、只改 label 保持标志、显式取消才清零、嵌套键集深度契约、Planner 自定义终态/再激活端到端与历史回滚测试。

## SR-169：统一审核队列未给行绑定 React key，条目移除后会把上一条的人审草稿提交给下一条对象

- 严重度：P1
- 确定性：FACT（同类待办至少两条，前一条处置或刷新后从列表移除即可稳定触发）
- 证据：`frontend/src/components/review/ReviewQueue.tsx:18-66`、`frontend/src/features/ask-human/index.tsx:110-134,215-238`、`frontend/src/components/review/TaxonomyCandidateReviewCard.tsx:29-93`、`frontend/src/features/ask-human/inline/EscalationInline.tsx:16-44`、`frontend/src/components/review/LessonPromoteCard.tsx:20-72`、`frontend/src/components/review/ChunkReviewCard.tsx:46-87`、`frontend/src/components/review/ProfilePublishCard.tsx:28-115`；现有队列测试只覆盖首次加载与成功后 refetch，见 `frontend/src/__tests__/components/review/ReviewQueue.test.tsx:8-39`。
- 机制：`ReviewQueue` 接收 `getId`，但只用其返回值计算 `busyId`，最终 `items.map(...)` 直接返回 `renderItem(...)`，没有给每行元素设置 React `key`。统一收件箱的 `renderItem` 也返回无 key 的 `InboxRow`。因此 A 被处置后 refetch 得到 `[B,...]` 时，React 按数组位置复用第一行的 `InboxRow` 和同类型子组件；新 props/动作 URL 已是 B，本地 `useState` 却仍是 A 的表单草稿、展开态或已加载对象。临时回归测试以两条 Taxonomy 候选复现：编辑 A 显示名后让 A 消失、B 上移，再点采纳；请求 URL 已是 `/B/approve`，body 的 canonical id/label 却仍是 A（含编辑值），React 同时输出缺少 key 的警告。Taxonomy、领导裁决和 Lesson 晋升都持有可写草稿；Chunk/Profile 还会在新 id 的异步 reload 完成前展示旧对象并允许按新 id 动作。
- 影响：管理员看到 B 的外层标题时，可能把 A 的标签 canonical 内容写入 B、把 A 的领导裁决正文/约束提交给 B，或把 A 编写的同行案例标题和正文晋升到 B；这些都是通过合法 B URL 落库的成功请求，后端无法从请求恢复草稿来源。Chunk/Profile 的竞态窗口还可能让“核验/发布的是屏幕上旧对象，实际动作目标是新 id”。结果不是单纯 UI 错位，而是人审证据与被审批对象分离，可污染生产字典、客户裁决和知识候选。
- 建议：`ReviewQueue` 必须自己以 `getId(item)` 包装每行并设置稳定 key，不能把身份责任隐式交给任意 `renderItem`；`busyId` 也应与同一 key 绑定。所有持有本地草稿或异步详情的审核卡再做防御：id 变化时重置草稿/旧对象，提交时绑定打开时的 id/generation，详情未加载到同 id 前禁用动作。增加 `[A,B]→[B]` 的真实组件重排测试，分别断言 Taxonomy canonical、Escalation substance、Lesson 正文、Chunk/Profile 展示与 URL 始终属于同一对象，并让 React key warning 在测试中失败。

## SR-170：Evolution 总开关不能阻止自动发布，写失败后页面还会假装已经关停

- 严重度：P1
- 确定性：FACT（自动发布需 env 自动发布总闸、workspace 子闸已开启且存在满足条件的 eligible threshold 候选；前端假关停在关闭 PUT 失败时确定触发）
- 证据：`frontend/src/features/evolution/EvolutionCenterTab.tsx:177-240,350-390`、`frontend/src/__tests__/EvolutionCenterTab.test.tsx:492-558`、`src/routes/evolution.rs:549-635`、`src/evolution/mod.rs:82-117,224-240`、`src/evolution/auto_release.rs:36-64,69-210,506-517`、`src/evolution/cohort.rs:61-77`。
- 机制：页面把 Mongo `runtime_flag.enabled` 命名为“演化中心总开关”。关闭成功时 PUT 只写 `enabled=false`，请求不带 `thresholdAutoReleaseEnabled`，后端又明确保留既有子闸。下一次 `run_one_tick` 虽因 disabled flag 得到空 cohort，仍在末尾无条件调用 `auto_release_eligible_thresholds`；后者重新读取同一文档时只提取 `threshold_auto_release_enabled`，其 gate 仅检查 env `evolution_auto_release_enabled && 子闸`，完全不检查 `flag.enabled`，随后扫描所有旧 `eligible_for_release` threshold proposals 并可真实 release。关闭请求失败时还有第二条同义裂缝：checkbox `onChange` 先把本地 `flagEnabled` 改为 false，再发 PUT；catch 只把错误塞进普通 `flagMsg`，不回滚、不重读，也不用 alert。临时组件测试实证 500 `write failed` 后 checkbox 仍为关闭。现有测试只覆盖成功打开和成功保存，auto-release 纯函数也只测 env+子闸，恰好没有把所谓总开关纳入真值表。
- 影响：管理员为止损关闭演化中心后，旧的可发布阈值候选仍可能被后台自动写入生产 override；若关闭写本身失败，数据库甚至继续完整运行，而页面仍显示已关。新 cohort 停止并不能抵消旧候选发布，错误又没有危险态反馈，使运营把一个非权威 UI 状态误当成 kill switch。阈值变化会直接改变生产复核闸门，且与页面“关闭期间不再产生新实验”的提示共同制造错误安全感。
- 建议：定义并复用唯一总闸：任何 cohort、评测、post-release 副作用与 auto-release 都必须先满足 `env evolution enabled && runtime_flag.enabled`；auto-release 再叠加自己的 env/子闸，不能跳过父闸。可保留子闸偏好，但父闸关闭时必须一票否决，并在 tick 内用同一份 flag/generation，避免中途读到组合状态。前端使用已提交值与草稿值分离，PUT 成功读回后才改变权威开关；失败则回滚/重读并以 alert 显示。PUT 增加 expected updatedAt/generation CAS。增加 `enabled=false + 子闸=true + eligible proposal` 零 release、关闭 PUT 失败 UI 保持开启、双写乱序和关闭期间旧候选不变的端到端测试。

## SR-171：Evolution“近 7 天”聚合只统计最新 20 个实验，默认频率下稳定少算

- 严重度：P2
- 确定性：FACT（worker 按默认 6 小时频率持续运行满 7 天时确定触发；更高频率下截断更严重）
- 证据：`frontend/src/features/evolution/EvolutionCenterTab.tsx:131-166,265-299,338-347`、`src/routes/evolution.rs:42-43,64-103`、`src/evolution/mod.rs:58-66,82-117`、`src/config.rs:618`、`.env.example:150`；现有聚合测试通过手工数组验证纯函数，但页面测试只返回少量 items，见 `frontend/src/__tests__/EvolutionCenterTab.test.tsx:47-68,139-169`。
- 机制：页面请求固定 `/api/evolution/experiments?limit=20`，后端按 `started_at desc` 先截最新 20 个；客户端随后才以 `Date.now()-7d` 过滤，并把结果标成“近 7 天实验/候选总数/已发布/已回滚/显著性通过率”。默认 `EVOLUTION_TICK_SECONDS=21600`，即每天 4 个、7 天约 28 个；worker 每 tick 在读取 runtime flag 前就插 experiment envelope，因此即使 workspace flag 关闭也持续生成这些信封。运行一周后接口稳定丢掉至少较早 8 个仍在 7 天窗内的实验及其 proposals，聚合函数无法从被截断的输入恢复。契约测试只校验单个 envelope/summary 顶层键，不能证明时间窗完整。
- 影响：管理面把约 5 天的最新 20 条样本冒充 7 天指标，实验数、候选数和发布/回滚数系统性偏低，显著性通过率还会因被截掉样本的分布而偏高或偏低。运营可能据此误判演化活跃度、发布质量或关闭效果；提高 tick 频率会进一步缩短实际覆盖窗口，却没有 truncated/coverageStart 提示。
- 建议：把 7 天聚合下沉为按 `started_at >= since` 的服务端聚合，独立于列表分页，并返回 `windowStart/windowEnd/sampleCount`；候选状态和显著性应在同一窗口内聚合。若暂时客户端分页，必须按 cursor 拉到跨过 cutoff，且响应显式标记截断，不能用固定 N 冒充时间窗。增加默认 28 个信封、超过 100 个高频信封、边界时间、无效时间与截断提示测试。

## SR-172：Outbox 投影删除媒体与名片身份，发件箱把真实待发对象显示成可盲取消的空白行

- 严重度：P2
- 确定性：FACT（账号存在 pending/in_flight 的媒体或名片 Outbox 即稳定触发）
- 证据：`src/models.rs:2998-3040`、`src/routes/admin_outbox.rs:60-113,116-220,250-274,450-486`、`src/agent/gateway.rs:3103-3234,3303-3324`、`frontend/src/contracts/outbox_entry.fixture.json:1-24`、`frontend/src/contracts/outboxEntry.contract.ts:1-25`、`frontend/src/features/autonomy/OutboxPanel.tsx:13-23,37-73,95-127`、`frontend/src/__tests__/contracts/configPlaybookDomain.contract.test.ts:13-41`；现有面板测试只构造有正文的文本行，见 `frontend/src/__tests__/features/autonomy/OutboxPanel.test.tsx:34-59`。
- 机制：媒体和名片发送分别以 `media_asset_id`、`referral_card_id` 作为 Outbox 中唯一的业务对象身份，生产 enqueue 又把两类行的 `content` 固定为空。管理列表的 `outbox_entry_json` 却主动省略这两个 id；同一模型的 `reclaimed_in_flight/reclaim_count` 也不投影。正式表格类型只保留 status/content/contact/time，因而媒体或名片行显示为同一联系人的空白“内容”，没有类型、素材名、名片目标、run 或恢复风险说明。pending/in_flight 行仍直接展示“取消”，无详情或确认。Rust 快照测试特意用非空 asset/card 构造模型，却把删掉字段后的 JSON bless 为 fixture；前端键集测试再只证明 fixture 与声明一致，所以该遗漏会稳定保持绿色。
- 影响：同一客户有多条待发素材/名片时，运营无法判断哪一行对应哪份文件或哪位顾问，只能按时间猜测并可直接取消真实生产投递；恢复过的条目还可能处于“远端可能已送达、正在核对”的不确定状态，但页面看不出。错误取消会撤掉本应发送的销售素材或顾问引荐；若取消的是 in_flight，取消与真实送达不一致的既有 fencing 风险继续归 SR-066。本项与 SR-159 的跨账号旧快照不同，即使单账号、无并发也稳定发生。
- 建议：Outbox 公共 DTO 必须投影互斥的 typed payload（`text | media | referralCard`）及业务对象 id，并由服务端批量解析可读名称/目标；同时公开 `reclaimedInFlight/reclaimCount` 或更清晰的 delivery-uncertain 状态。面板按类型显示正文、素材名或顾问名片，取消确认中展示目标、联系人、账号和恢复风险。契约测试应从完整模型断言语义必需字段，而不是把投影删字段后的 fixture 当作全部真相；增加空正文媒体/名片、多条同联系人、reclaimed 行和取消确认测试。

## SR-173：SSE 成功重连不恢复重试预算，知识长任务会在累计断线后无提示永久停更

- 严重度：P2
- 确定性：FACT（同一页面生命周期内，在两个 `turn` 事件之间累计发生超过重试上限的连接断开即可稳定触发；连接每次已成功 `open` 也不改变结果）
- 证据：`frontend/src/lib/useSseReconnect.ts:23-68`、`frontend/src/__tests__/lib/useSseReconnect.test.ts:22-80`、`frontend/src/features/knowledge/today.tsx:818-1018`、`src/routes/knowledge/chat.rs:2195-2229`、`src/knowledge_task/mod.rs:210-332,450-560,783-806`、`src/config.rs:456-459`。
- 机制：共享重连器只在注册的业务事件回调触发时把 `attempt` 清零；原生 EventSource `open`、Axum keepalive 和一段已稳定存活的连接都不算成功。每次 `error` 都继续消耗上一次留下的累计 attempt，达到默认 6 次后调用 `onGaveUp` 并永久停止创建连接。知识长任务的 SSE 只注册 `turn` 与终态 `close`；worker 又只在开始、每个完整 step 结束和总结时写 turn，单步可包含最长 45 秒且最多多次的 LLM 调用，所以“连接已成功恢复但业务暂时无新 turn”是正常窗口。临时纯组件测试连续构造“replacement EventSource 已 open、尚无 turn、再次断线”，冻结实现仍在第三次（测试上限 2）调用 `onGaveUp`，证明预算按历史断线累计而非连续失败计算；测试已删除。正式 TaskRail 没有传 `onReconnecting/onGaveUp`，也没有周期 GET task；SSE 的 turn 回调只向 `liveTurns` 追加数字，不重读任务详情。
- 影响：网络切换、代理空闲回收或多次短抖动可让仍在执行的知识派工从页面永久停止自动更新，且无错误、重连中或已放弃提示。主状态卡、完成步数、进度条、errorKind 与终态继续停留在首次手工拉取的快照；任务后端可能已经完成或失败，运营仍看到 pending/running，并可能误以为卡死、重复派工或执行不必要的取消。重新点“拉取”会临时恢复事实，但页面没有提示用户必须这么做。
- 建议：在 EventSource `open` 后重置连续失败计数，或以“连接稳定超过阈值/收到任何协议心跳”作为健康证明；重试上限应约束连续建连失败，而不是整个组件生命周期的累计断线。调用方必须消费 `onReconnecting/onGaveUp` 并显示状态，gave-up 后以有界轮询 GET task 兜底，收到每个 `turn` 也应重读任务详情而非只追加序号。增加 open→error 多轮仍不耗尽、连续建连失败才 gave-up、keepalive/长 step、gave-up 轮询收敛 completed/failed 和终态 close 零重连测试。

## SR-174：集成测试的独立 Mongo 仍共享进程级缓存，并行用例会互相读取错误数据库快照

- 严重度：P2
- 确定性：FACT（触发需同一集成测试二进制中的两个 `TestApp` 生命周期交错；Rust 测试默认并行执行用例）
- 证据：`tests/common/mod.rs:126-219`、`src/agent/taxonomy.rs:119-189,492-543`、`src/agent/domain_profile.rs:978-1086,1118-1139`、`tests/domain_profile_e2e.rs:1143-1753`、`tests/c2_operation_state_derivation_e2e.rs:199-258`；ignored Docker job 不阻断合并的独立问题继续归 SR-004。
- 机制：`TestApp::start()` 为每个用例创建 UUID 数据库，并明确宣称“互不干扰”，但启动末尾会把该数据库传给两个进程级 `LazyLock` 单例执行 `warm_up`。Taxonomy 与 DomainProfile 的 `reload_from_db` 都从单个传入数据库读取全表，再整体替换全局缓存并把 `fetched_at` 设为当前时间；缓存键只有 workspace/scope/kind 或 workspace，不含 Mongo database identity。于是 A 启动并写入/读取测试画像后，只要 B 用空数据库完成预热，A 随后的 `load_active_domain_profile(&db_a,"default")` 会看到“缓存尚新鲜”而不再查 `db_a`，直接使用 B 的空快照回落 DEFAULT；反向也可把 B 的 profile/taxonomy 读成 A 的。单个集成测试文件包含多个默认并行的 `#[tokio::test]`，例如 `domain_profile_e2e` 有 28 个、C2 文件有 6 个，CI 没有设置 `--test-threads=1`。夹具内注释已经承认前一测试 DB 的 active profile 可残留，但每次启动预热只能改变最后写入者，不能隔离并行生命周期。
- 影响：依赖 active profile、声明维度、taxonomy alias/终态或状态机的 Mongo 集成测试会时序性假红或假绿：合法字段可能被错误剔除后回落，非默认 profile 可能被 DEFAULT 替代，也可能读取另一用例刚写的自定义语义。失败通常表现为业务断言偶发漂移，重跑或串行执行又恢复；更危险的是本应验证自定义域的用例可能在默认销售域上通过。由于全量 ignored job本就 soft-fail，维护者更难区分生产回归与夹具串扰。
- 建议：测试夹具不要调用生产进程级单例；为 `AppState` 注入按实例持有的 Taxonomy/DomainProfile cache，或让 cache key 至少包含稳定 database identity，并确保所有生产调用从 state 取同一实例。短期可把使用 `TestApp` 且触达全局缓存的测试二进制设为串行，但这只是隔离补丁，不应替代依赖注入。增加两个数据库 A/B 的 barrier 测试：A/B 同 workspace 各自写不同 profile/taxonomy，交错 warm-up 与读取后仍各见自身；同时验证 invalidate 只影响对应实例。

## SR-175：Reviewer 危险评分缺失或畸形会归零并被当作安全，三道发送闸可一起失效

- 严重度：P1
- 确定性：FACT（实际危险发送需 Reviewer 对其它分项给出通过值并省略或畸形化相应危险评分）
- 证据：`src/agent/types.rs:1157-1181,1459-1464`、`src/agent/review/mod.rs:287-323,394-442,500-570`、`src/agent/review/gates.rs:20-45,115-206,563-629,824-880`、`tests/pressure_risk_threshold_pbt.rs:59-75`；产品声明自报漏判的独立问题继续归 SR-022，已知软闸失败后 revision 回退原稿继续归 SR-023。
- 机制：Reviewer prompt 明确要求 `factRisk`、`pressureRisk` 与 `boundaryPrivacySafety`，但 `ReviewScores` 三字段都带 `serde(default)`，共用的 `number_i32` 还把字符串、null 或其它不可解析值静默变成 0。主 Reviewer 与第二 Reviewer 都只做 `serde_json::from_value`，随后立即进入 `route_dual_gate`，没有字段存在性、类型或 0–10 范围校验。`factRisk=0` 天然满足 `hallucination_score < threshold`；另外两项更把 0 明确定义为“未填/老数据豁免”，所以 `pressureRisk=0` 不触发高压改写，`boundaryPrivacySafety=0` 也不触发内部画像、AI 身份或幕后领导信息泄露改写。只要 approved、humanLike、emotionalValue、productAccuracy 等其余字段给出通过值，分类就是 AllPass；Finalize 只消费这些已归零的分数和既有协议风险，不补 Reviewer schema 完整性门，最终可进入 Approved/outbox。正式 PBT 与模块单测还主动断言 pressure/boundary 的 0 必须放行，把 fail-open 兼容策略固化成绿色。
- 影响：一次部分 JSON、键名漂移或类型错误即可同时撤掉事实风险、压迫感与隐私边界三道独立检查。候选回复即使包含编造事实、催促逼单、复述内部客户画像、确认 AI 身份或暴露领导决策来源，也可能仅因 Reviewer 没产出可解析分数而被当作低风险；双 Reviewer 开启时，第二路同样宽容解析，且第二路解析失败只回落主路，不能形成可靠冗余。审计中会持久化分数 0，看起来像明确的最低风险评分，而不是“评分缺失/无效”。
- 建议：把当前调用的 Reviewer 输出定义为严格 wire DTO：所有发送闸字段必须存在、为整数且落在 0–10；缺失、null、字符串或越界均返回显式 schema failure，并对危险方向采用 fail-closed 分数或 hold/retry，不能复用历史文档兼容默认。旧持久化数据兼容应放在只读迁移/展示层，与实时 LLM 输出解析分离。审计保存 `scoreStatus=valid|missing|invalid` 和原始字段摘要，双 Reviewer 分别校验后再比较。增加每个字段缺失、错名、字符串/null/数组、越界、三字段同时缺失，以及真实 Gateway 不入 Outbox 的端到端测试；把现有“0 必须豁免”PBT改为仅针对带明确 legacy provenance 的读取路径。

## SR-176：多租户“隔离回归”大量重写预期 Mongo filter，生产 Handler 漏掉租户条件仍会绿

- 严重度：P2
- 确定性：FACT（当前抽查的生产 Handler 多数确实带租户过滤；本项评价测试证据失效，不宣称这些 Handler 当前已越权）
- 证据：`tests/workspace_isolation.rs:39-138,209-294,296-443,445-502,504-628`、`tests/products_workspace_isolation.rs:1-137`、`tests/auth_middleware_integration.rs:105-134`；真实 Handler 边界测试的对照见 `tests/h3_cross_tenant_idor.rs:66-179`、`tests/account_security_integration.rs:58-135`。全量 ignored job 不阻断合并的独立问题继续归 SR-004。
- 机制：`workspace_isolation` 把多项用例命名为联系人 IDOR、admin mutation handler、MCP passthrough 和“终极审查”，注释还逐一列出 Accounts、Tasks、Souls、Management、Guides、Outbox、State Policy、Ops Version、Evaluation 等生产 Handler；测试体却不调用任何这些 Handler 或其共享授权 helper，只在测试内手写 `{_id, workspace_id}` / `{account_id, workspace_id}` 后直接查询 Mongo，再断言这个由测试自己加入的条件隔离了数据。`products_workspace_isolation` 同样只直接查询 `products` collection，虽然设计验收要求 workspace A admin 访问 workspace B product/outcome 并得到空或拒绝。生产代码若把任一 Handler 的 `workspace_id` 条件删掉、改错字段名、先做副作用后校验，以上测试仍会保持绿色。认证文件还有同根的声明漂移：用例名与注释声称验证过期 session 返回 `SessionExpired`，正文因 `create_session` 强制 TTL 至少一小时，最终只删除 session 并断言 `SessionNotFound`，过期分支从未执行。与之相对，H3 Provider、账号 MCP key 和 JWT 测试确实直调真实 Handler/解析器，因此不属于本项。
- 影响：维护者和审计文档会把这些绿色用例当作广泛 IDOR 与 session 过期回归门，但它们只证明 Mongo 等值过滤本身有效。新增或重构路由时，最危险的错误恰恰是没有使用该过滤、使用了错误 BSON 键、信任请求体 workspace 或校验时序过晚；当前测试无法感知。即使 Docker integration job未来从 soft gate 升为 hard gate并全部通过，这部分权限保证仍是脚本自证，可能掩盖跨租户读取、改写、取消、发布或 MCP 透传回归。
- 建议：每个高风险资源域至少从真实 Axum Router 或真实 Handler 进入，使用有效认证身份/ACL，分别断言跨租户请求返回 403/404、目标行与下游副作用未变化、本租户正向路径成功；共享 helper 只能补充、不能替代端点覆盖。若可见性阻碍测试，应通过 `oneshot` 测完整 Router，而不是复制内部 filter。为 session 直接插入 `expires_at <= now` 的真实 `AdminSession`，同时覆盖 middleware cookie 拒绝与错误码。建立路由清单到测试 case 的机器可读映射，禁止测试名称声称覆盖未调用的 Handler，并让关键 IDOR/认证用例进入阻断合并的安全 job。

## SR-177：Webhook 已 ACK 的待回复只存在进程内，崩溃或 runner panic 后没有持久恢复入口

- 严重度：P1
- 确定性：FACT（丢处理触发需消息落库并 ACK 后、Gateway 完成前进程退出或 debounce runner panic；多副本重复处理触发需同一联系人的不同入站落到不同实例）
- 证据：`src/webhooks.rs:54-85,101-137,147-268,477-525,595-672`、`src/main.rs:202-320`、`tests/debounce_pipeline_integration.rs:1-20,218-453`、`tests/debounce_barge_in_run.rs:151-341`。
- 机制：Webhook 先把入站写入 `conversation_messages`，随后把唯一待处理状态放进进程内静态 `PENDING`，以裸 `tokio::spawn` 启动 debounce runner，并立即返回 `queued=true`。消息模型没有 `processing_status/claimed_at/generation`，启动流程也没有扫描“已入站但未产生 run”的恢复 worker。进程在去抖睡眠、Reaction、LLM 或 Gateway 期间退出时，Mongo 中只剩消息，`PENDING` 与 future 一起消失；同一 webhook 即使由上游重送，也会在消息唯一键冲突分支直接返回 `duplicate=true`，不会重新注册 runner。runner panic 的 catch 分支同样只删 `PENDING`、写 best-effort 事件，不重试或落 durable task。另一方面，`PENDING` 不跨实例共享；同一联系人的不同消息若落到两个副本，会各自看到本地 vacant 并各起一条流水线，单联系人串行与 barge-in generation 均失效。
- 影响：客户消息可以已经成功入库且上游收到 200，却在没有后续新入站时无限期不触发 Reaction、决策、Review 或 Outbox，运营只能看到消息而没有对应 run/失败任务；上游重放也无法自愈。横向扩容时，同一轮连发还可能产生并发决策、重复回复以及画像/记忆写竞态。现有集成测试真实覆盖单进程 runner 的聚合、最新快照与正常退休，但测试自己持有并等待 runner handle，没有覆盖进程退出、panic 后重领、启动回扫或双实例 claim，因此不反证该链。
- 建议：把 webhook 接收与 Agent 处理解耦为 durable inbox/job：消息落库时同事务或可恢复幂等写入 `(workspace,account,contact,generation)` 待处理记录，worker 用 owner/token/lease 原子 claim；成功提交 run/outbox 后标 completed，失败/过期可重领，并以 generation fencing 阻止旧执行者提交。DuplicateKey 重放必须检查对应 durable job 是否已完成，未完成则确保重新排队。若继续按联系人合并，合并窗口和最新 generation 也应存 Mongo/队列而非进程内 Map。增加 ACK 后进程退出→重启恢复、runner panic→自动重试、重复 webhook 修复未完成 job、双实例同联系人连发只产生一个有效 generation，以及旧 worker 晚到不得入队的 barrier 集成测试。

## SR-178：真模型红线“硬门”允许目标行为零发生且不记 skip，nightly 可在未执行红线断言时绿色

- 严重度：P1
- 确定性：FACT
- 证据：`.github/workflows/ci.yml:1079-1213`、`scripts/check-skip-ledger.sh:1-46`、`tests/real_llm_digital_twin_arc.rs:214-226,301-380,393-410`、`tests/real_llm_principal_channel.rs:515-714`、`tests/real_llm_proactive_outreach.rs:252-377`、`tests/real_llm_cross_domain_arc.rs:1026-1272`、`tests/real_llm_dynamic_adversarial.rs:61-88,261-373`、`tests/real_llm_roleplay_arc.rs:267-386`；有效正向对照见 `tests/real_llm_principal_relay.rs:421-621` 与 `tests/real_llm_cross_domain_arc.rs:877-961`。
- 机制：nightly 的 `real-llm-redline` 不设 `continue-on-error`，并宣称矩阵主体是确定性红线 panic；但多个矩阵文件把“没有产生可检查产物”编码成普通成功返回。Digital Twin 的身份生成返回 `None`、后续 roleplayer 全 fallback 时直接 `return`，且只有 `sent_turns>0` 才检查画像，只有实际非空回复才扫描转真人/身份泄漏；Principal Channel 把四轮超职权诉求是否产生 pending escalation 全部降为软观测，零 escalation 只写 issue，禁词同样只在 `sent_like && reply_text 非空` 时检查；Proactive Outreach 在 planner 未建 task、wake task 缺失或 Gateway 没有 reply 时直接返回/结束，未证明任何主动内容经过红线扫描。Cross-domain 的身份探针在两轮都未发送时只打印“合法”，R2.3 任一域端点失败或两域回复为空时也正常结束；Dynamic Adversarial 的瞬时错误宏直接 return 且不写 ledger，roleplayer 全 fallback 也正常退出，跨会话画像只在第一段已经有画像时检查“不丢失”；Roleplay Arc 同样在全 fallback 时无记录返回。只有部分套件的顶层 `unwrap_or_skip_transient!` 错误分支会写 `skip_ledger.jsonl`；上述 `None`、fallback、零 task、零 reply、零 escalation，以及 Dynamic 的 transient return 都可能不记 skip。汇总脚本只按该文件行数计数，文件不存在时还输出“0 skip，全部真跑”。因此 matrix test process 可以 exit 0、skip gate 也 exit 0，而目标红线断言一次都没有执行。Principal Relay 是反例：它硬断领导消息被 resolve、relay task 存在、转述文本非空，再扫描转真人与幕后来源禁词；Cross-domain 在确有发送时也会真实 claim Outbox、调用 `process_entry` 并反查 MCP 请求，二者都具备目标行为的正向见证。
- 影响：nightly 会把“没有生成身份/回复、没有触发请示、没有排出主动任务”显示为 Digital Twin、治理通道或主动触达红线通过；最需要保护的越权承诺、真人接管、身份泄漏和幕后决策源暴露实际上没有样本可检查。维护者会把 job 的绿色结论当作硬红线未击穿，skip gate 又明确误报“全部真跑”，使端点/fixture/模型行为退化长期隐藏。该问题不同于 SR-004 的 soft integration job：这里的 job刻意设计为非 `continue-on-error` 的硬门；也不同于 SR-128 的知识质量目标，本条聚焦红线矩阵缺少正向见证与 skip 记账。
- 建议：每个 redline case 必须产出 typed outcome（至少 `attempted,target_artifact_count,redline_assertions_run,verdict,skip_reason`），只有目标 artifact 非空且对应红线断言实际执行才允许 `pass`；零回复、零 escalation、零 task、identity `None`、roleplayer 全 fallback 与所有 transient return 应统一记 `infra_skip|inconclusive`，进入同一 ledger。对 Principal Channel 应至少硬断四轮中存在一次请示或明确的 fail-closed/合规拒绝终态，不能把“直接回复但是否越权待人工判断”算红线通过；Proactive 必须硬断任务存在且产生非空候选，Digital Twin/Dynamic/Roleplay 必须硬断最低有效 roleplayer/agent 轮数，身份探针必须要求至少一条可检查回复。skip gate 应按计划 case 数核对 outcome 覆盖率，缺 ledger、缺 case 或 `assertions_run=0` 必须失败，而不是解释为零 skip；保留 Principal Relay 的“resolve→task→非空转述→禁词扫描”和 Cross-domain 的“非空回复→Outbox→MCP”正向见证模板。

## SR-179：Kiro 任务表把未接线、已删除或未按验收形态交付的工作统一标为完成

- 严重度：P2
- 确定性：FACT
- 证据：`.kiro/specs/agent-autonomy-loop/tasks.md:17-431`，尤其 `:67-74,172-178,210-216,234-240,298-304,431`；`src/agent/run_envelope.rs:5-28`；`tests/autonomy_protocol_pbt.rs:138-424`；`tests/outbox_integration.rs:11-12,808-879`；`.kiro/specs/agent-self-evolution/tasks.md:9-21,232-235,312-317,348-353,383-393`、对应 `design.md:676-724`；冻结树中不存在 `tests/evolution_{isolation,prompt_e2e,rollback,threshold_e2e}.rs` 与 `tests/evolution_significance_pbt.rs`；`.kiro/specs/user-ops-agent-hardening/tasks.md:13-267,425-439`、`requirements.md:251-263,372-380`；`tests/worker_reclaim.rs:1-91`、`tests/dry_run_isolation.rs:1-183`、`tests/reaction_claim_lock.rs:1-165`、`tests/last_inbound_split.rs:1-185`；`scripts/check-baseline.sh:53-134`、`.github/workflows/ci.yml:110-196`；`docs/superpowers/specs/2026-06-23-tool-loop-dead-code-sunset-design.md:10-27,73`。Autonomy 与 Hardening spec 顶部的 2026-05-25 sunset notice 只覆盖明确列出的销售域专属机制；本项评价任务完成记录与验收证据的真实性，不主张已 sunset 功能仍应上线。
- 机制：Autonomy 任务表 72 个 checkbox 全部标为 `[x]`，最终检查点还宣称“全部 PBT + lib + 集成 + happy_path 通过”。冻结生产源码却直接注明 Run Envelope 的 started 信封、recovery insert 与 panic lifecycle 推进均“未接线”；后来的工具循环下线设计也明确记录 `reply_with_tools_loop` 从未被任何生产入口触达，只有测试调用。测试交付同样与勾选描述不一致：`autonomy_protocol_pbt.rs` 只有 P1/P2/P3 三条 proptest，P5 分散在 memory 模块，P4/P6主要是示例测试，P7 随从未接线的工具循环后来删除；任务 5.8 所称“任意状态序列 PBT”实际是一个 `#[ignore]` Docker 集成用例，固定重复 enqueue 后计数 MCP 调用，并非随机状态序列性质测试。Self-Evolution 任务页又在顶部 Done Notice 宣称 W0–W4 与收口全部落地，同时明确承认任务 5.9 的四个端到端集成测试和任务 4.8 的 significance PBT 已删除；正文中的对应任务、baseline 条目以及“shadow replay 100 次零副作用（集成测试覆盖）”验证项却仍保持 `[x]`。冻结树确认五个声明文件均不存在，现有替代测试不能等价证明完整 tick→shadow→release→生产读取、rollback、100 次零副作用与所声明 PBT。Hardening 的 24 个 task 也全部标 `[x]`，并宣称 HP-1/2/3 与 S-20 有真实回归/隔离测试；但 `worker_reclaim` 只插入 stale 行再断言仍是 running，明确承认没有调用私有 reclaim；`dry_run_isolation` 手工插入 status=dry_run 的 command/tool 行，没有调用 management handler 或任何工具 dispatcher；`reaction_claim_lock` 在测试中重写 Mongo CAS，不调用 `record_user_reaction`、Webhook 或 LLM；`last_inbound_split` 同样复制 update 文档而非走入站/出站生产入口。这些 ignored 用例所在 integration job 还是 `continue-on-error`。文件、模块或局部测试存在，因而被当作任务完成，但没有机器可核验的“生产可达 + 指定验收已执行”证据。
- 影响：维护者、后续 sunset 方案和审计人员会把全勾选任务表误读为 Autonomy R0–R13、Self-Evolution W0–W4 与 Hardening 24 项均完整上线并通过规定门禁，进而在错误基线上删除兼容代码、评估回归覆盖或接受迁移完成。Run Envelope 未接线的实际可追溯性缺口由 SR-019 记录，Reaction/Task 所有权缺口由 SR-028/SR-034记录，产品声明漏判由 SR-022、Outbox 多租户/取消问题由 SR-024–027 与 SR-066记录，Evolution 的发布基线、rollback 与观测缺口由 SR-086–088记录；本项的独立影响是交付账本失真，使这些生产缺口在“全部完成”的历史记录下更难被发现和追责。
- 建议：把 spec task 状态改为可审计枚举 `planned|implemented|production_wired|verified|partial|sunset_not_shipped`，每个 `verified` 条目必须绑定冻结 commit、生产调用点和实际阻断门中的测试 case/artifact；由 CI 生成需求→符号→生产入口→测试→job 的 traceability manifest，并拒绝“引用符号无生产可达路径”“测试复制生产 filter/update”“声明的 PBT/case 不存在或未被硬 job执行”却标完成。对三份任务体系做追溯修订：Autonomy 的 Run Envelope、工具循环、P1–P7、Outbox crash/PBT；Self-Evolution 的四个 E2E、significance PBT、100 次隔离断言与替代覆盖；Hardening 的 worker reclaim、reaction claim、入出站时间与 management dry-run验收，逐条标出实际生产入口、断言与门禁状态。历史档案可保留，但不得用全 `[x]` 表示未经验证的完成。

## SR-180：Evolution 正式需求仍要求管理员发布，生产却存在可开启的阈值自动放量

- 严重度：P2
- 确定性：FACT（契约冲突确定存在；真实自动放量需 env 总闸与 workspace 子闸均开启，并存在满足方向与风险门的 eligible threshold proposal）
- 证据：`.kiro/specs/agent-self-evolution/requirements.md:8-25,245-267`、`design.md:730-756`；`docs/agent-policy.md:330-390`；`src/evolution/mod.rs:224-253`、`src/evolution/auto_release.rs:1-24,36-210`、`src/routes/evolution.rs:549-611,689-705`、`.env.example:174-184`；后续实现依据仅见 `docs/superpowers/specs/2026-06-26-main-health-audit-batch2-design.md:79-91` 与对应 plan，其中记录“已与用户确认轻量版”，但没有同步修订 requirements-first 产品需求或权威策略文档。总开关关闭后仍可能放量的实现漏洞另见 SR-170。
- 机制：M4 requirements 把“shadow eval 通过 + admin 显式确认”定义为任何 release 的前置条件，并在 R9.6 明确禁止本期引入自动发布开关，要求未来放宽时由独立 M5+ spec 提议；design 也只把自动发布列为 M5 候选。当前 `docs/agent-policy.md` 仍声明 Evolution release/rollback 仅 admin 可触发，Admin 视角的唯一写动作是 release/rollback/rollback_all。冻结生产代码却在每个 tick 末尾调用 `auto_release_eligible_thresholds`：当 `EVOLUTION_AUTO_RELEASE_ENABLED=true` 且 workspace 的 `threshold_auto_release_enabled=true` 时，它扫描 `eligible_for_release` threshold proposals，通过方向、band 与可选负反应门后，以 synthetic actor `evolution_auto_release` 直接调用 `release_threshold` 写入生产 override。该能力默认双关且仅作用于 threshold，prompt 仍需 admin，因此本项不声称默认部署一定自动发布；问题是同一仓库同时把“永远需要 admin”与“可配置自动 release”都当成权威真相。
- 影响：运营、审计、测试与事故响应无法从正式契约判断阈值是否允许无人确认进入生产：按 requirements/policy 编写的测试会把任何自动 release 视为红线，按实现计划和代码编写的测试则会保护自动放量。权限模型、变更审批、风险接受人与 kill-switch 语义因此没有唯一依据；SR-170 所述父开关失效也更难被识别，因为一套文档根本否认自动发布能力存在。默认关闭降低了即时触发概率，但不能消除显式开启后的治理与审计歧义。
- 建议：先由产品/安全所有者选择唯一契约并版本化。如果 threshold auto-release 被正式接受，应新增或修订 requirements-first spec，明确适用 proposal_kind、双闸层级、审批豁免依据、风险门、actor、审计、总开关和紧急停用语义，并同步 `docs/agent-policy.md`、UI 和端到端测试；若未被正式接受，则移除生产调用与配置字段。CI 应校验安全边界声明与公开配置/生产调用点的对应关系，禁止只在实现 plan 中改变“必须管理员确认”这类产品红线。

## SR-181：运营记忆承诺可随时撤销，但系统只有新增与读取路径

- 严重度：P2
- 确定性：FACT
- 证据：`.kiro/specs/knowledge-digest-workstation/requirements.md:76-86`；`src/routes/knowledge/chat.rs:889-900,1219-1294`、`src/agent/memory.rs:2073-2207`、`src/routes/knowledge/sources_meta.rs:765-852`、`src/routes/mod.rs:648`、`frontend/src/features/knowledge/atlas.tsx:1383-1420`；现有测试只覆盖模型、隔离与预算等局部不变量，见 `tests/knowledge_operator_memory_isolation.rs:1-160`。
- 机制：R5.4 要求每次写入在 Chat 中附 `memoryId`，使运营“随时可以撤销”。生产 `update_operator_memory_for_chat` 确实写入独立 collection、返回 id，并明确回复“如需撤销请直接告诉我”；但意图闭集只有 `update_operator_memory`，没有 revoke/delete 分支。`record_operator_memory` 只能插入或按同内容 bump `last_used_at`，读取路径还会再次 touch；公开 API 只有 `GET /api/knowledge/operator-memory`，前端也只列出记录。仓库不存在按 memoryId 删除、失效、设置 `expires_at` 或写撤销审计的 Handler。新记录默认 `expires_at=None`，所以运营在 Chat 里说“撤销”最多会落到 freeform 或再次写入路径，无法使原记录停止被后续 prompt 注入。
- 影响：误记、过时或包含不当禁令的运营偏好会无限期参与知识 Chat 的意图与草稿生成；在默认账号等 operator/account 标识重合的部署中，后续正式接线还可能把同一记录注入客户 Reply Agent。界面给出可撤销承诺和稳定 memoryId，却没有可执行控制面，运营无法确认旧规则是否仍生效，只能直接改 Mongo 或等待并不存在的自动过期。新增相反偏好也不能可靠撤销旧值，且按 `last_used_at` 排序可能让冲突记录一起进入前五条。
- 建议：增加按 `(workspace,account,operator,memoryId)` 授权的显式 revoke/delete 路径，并为 Chat 增加 `revoke_operator_memory` 意图；优先采用带 `revoked_at/revoked_by/reason` 的可审计软失效，所有加载与列表投影统一排除或标识 revoked。确认 turn 应回显被撤销的 id/content，未知或跨 scope id 必须拒绝，前端列表提供同一动作。增加“新增→注入→撤销→不再注入”、重复撤销幂等、跨账号/跨 operator 拒绝、冲突偏好与审计事件测试。

## SR-182：memoryCard 同时要求最多 6 条和所有未废弃 coreFact 永久保留，生产会静默挤掉旧事实

- 严重度：P2
- 确定性：FACT（旧卡已有未废弃事实且本轮 incoming 新事实使合并集合超过 6 条时触发）
- 证据：`.kiro/specs/user-ops-agent-hardening/requirements.md:140-152,372-380`、`design.md:508-558,1227,1331-1345`、`tasks.md:92-101,256-260`；`src/agent/memory.rs:334-422,1511-1520`；`tests/memory_card_invariants.rs:48-124,488-507`。
- 机制：Requirement 8 一方面把 `coreFacts` cap 固定为 6，另一方面要求任意初始集合 S 在后续 N 轮中只要未被 `discarded`，最终 `coreFacts ⊇ S`。即使上一版恰有 6 条，这一轮新增 1 条也不可能同时满足两个不变量。生产实现以 incoming card 为前缀，再追加 previous 中未 discarded 的独有事实，最后直接 `truncate(6)`；因此 incoming 有 6 条时，上一版 6 条可全部被静默挤掉，并未按文档声称的 importance 排序或迁入 recent/deprecated。PBT 注释仍宣称未 discarded 必保留，正文却只在 `total_potential <= 6` 时断言保留；超 cap 时主动跳过核心性质，另一条 legacy PBT 也只检查 discarded 不回流，因此绿色测试掩盖了合同不可满足与实际丢失路径。
- 影响：客户身份、预算、禁忌或长期承诺等已确认核心事实可仅因一次 consolidator 输出较多新事实而从 active memoryCard 消失，后续 Reply/Planner 不再看到它；没有 deprecated 记录、淘汰理由或 UI 提示，运营会把消失误认为模型从未记住。cap 防止 prompt 无界增长是合理目标，但当前“新值优先截断旧值”的隐藏仲裁与“旧核心事实不被新近性挤出”的产品承诺相反。
- 建议：先定义可满足的淘汰契约：例如 `coreFacts` 仍 cap 6，但按结构化 importance、权威来源、更新时间和稳定 tie-break 对新旧全集统一排序；被挤出的未废弃事实转入有上限的 recent/archive 并记录 eviction reason，而不是静默删除。若真正要求永久保留，则必须取消固定 cap 或把持久事实库与 prompt 投影窗口分层。PBT 应生成 previous=6、incoming=1..6 的超 cap场景，断言明确的排序/归档规则；禁止用条件分支跳过所声明性质，并增加 N 轮 consolidation 后可追溯性测试。

## SR-183：47 域“事实底座”允许空证伪和静默漏域，结果又缺少可复现元数据

- 严重度：P2
- 确定性：FACT
- 证据：`.kiro/specs/universal-test-coverage/deepread-verify-workflow.mjs:1-6,70-96,98-125,144-176`、`deepread-verify-result-2026-06-30.json:1-4,1492-1531`、`audit-2026-06-30-anchors.json:38,1611`、`biz-domains-2026-06-30.json:1-6`；`docs/superpowers/specs/2026-06-30-上线前全量业务测试方法论-design.md:4,12,33-40,63-73,132-135`。
- 机制：该 workflow 宣称每个域都经过“深读 + 独立对抗证伪”，但 `FALSIFY_SCHEMA` 只把 `domain/verdict/test_priority` 设为必填，`verified_gaps/refuted/confirmed_orphans` 全是可选字段；知识缺口域实际只返回 `verdict="deep-read holds, no refutation needed"`，没有任何逐条证伪数组，仍被计入 `completed=47`。任一 agent 连续三次失败时，`agentRetry` 返回 `null`，deep 失败直接丢弃该域，最终又用 `findings.filter(Boolean)` 生成结果；没有失败清单、inconclusive 状态或 completeness gate，少域也会正常返回。结果根对象只含 `total/completed/findings`，没有冻结 commit、生成时间、workflow/prompt 版本、模型/provider 指纹、输入对象哈希、重试/失败记录或证据文件哈希，无法重放同一审计。聚合内容也没有一致性门：anchors 一处仍称 Evolution 是 `M4 W1 skeleton(empty tick by design)`，另一处则描述完整 tick/release 链。下游设计却把这份输出直接称为“事实底座”和“47 域权威行为清单”，宣称每域都经证伪，并据此发布 `300` 条真缺口、`190` 条孤儿；机械统计结果确有 300 条 `verified_gaps`，但只有 182 条 `confirmed_orphans`，第 25 域还是零证伪明细。
- 影响：这批结果包含大量有价值的 grep 线索和真实反证，但其汇总数字、域完成度和“已独立证伪”标签不能作为可复现的上线证据。agent 空答、schema 最小响应、重试耗尽或输入清单漂移可以在不显式降级结论的情况下消失；陈旧锚点和互相冲突的代码理解也可同时进入后续测试优先级。维护者若据此把 20 个域判为不可上线、按 300/190 清单分配修复或把未列项当作已排除，会把非确定 AI 审阅结果误当冻结事实。本项不同于 SR-179 的任务完成账本失真，也不同于 SR-178 的真模型红线零样本假绿：这里是审计生成协议本身允许不完整输出却提升为权威基线。
- 建议：把每域结果改为强类型 outcome：`complete|inconclusive|failed`，只有 deep 与 falsify 均返回必填的逐条 claim、证据路径/行号、反证查询和 verdict 才计入 completed；重试耗尽必须保留错误并使全量审计失败或明确降级，禁止 `filter(Boolean)` 静默删除。根 manifest 固化 commit、dirty-state、workflow/prompt hash、模型/provider 指纹、生成时间、输入文件 SHA-256、每域尝试次数和 token/错误账本；对 domain 清单与内联副本做哈希一致性检查，对同一事实的跨 anchor 冲突设人工仲裁队列。所有 300/182 等汇总应由机器从 manifest 生成并校验附录，不能手填近似数。重新在冻结 commit 上运行后，逐条把高风险线索回查生产代码与硬门测试；在此之前把产物标为 `research_leads`，不得称为上线事实底座。
