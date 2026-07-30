# 当前架构（审查中）

状态：阶段 1–2 启动骨架与数据层事实，后续阶段会持续修订。只描述已由 PR #223 代码证明的部分。

```text
React/Vite SPA ───────────────┐
                              │ /api
WeChat/GeWe ─ /webhooks/wechat│
                              ▼
                    单个 Rust/Axum 进程
                    ├─ AppState
                    │  ├─ Mongo Database/Client
                    │  ├─ MCP Client
                    │  ├─ 进程级主 LLM Registry
                    │  ├─ 可选第二 Reviewer LLM
                    │  ├─ JWT keys
                    │  └─ 进程内 cache/lock/event bus
                    ├─ API Router + auth middleware
                    ├─ SPA static fallback
                    └─ 最多 12 类 supervised worker loop
                              │
             ┌────────────────┼────────────────┐
             ▼                ▼                ▼
          MongoDB       external MCP      external LLM
```

启动关键顺序：Mongo connect → migrations → indexes → bootstrap/sanity/cache → provider → AppState → prompt seeds → workers → HTTP listener。

当前已证明的部署约束：API、静态站点和 worker 共进程；进程内缓存/锁/广播不跨副本同步；是否安全多副本部署需逐 worker 与写路径复核。

## 数据层启动与演进

```text
process start
    │
    ├─ Mongo connect（无 schema 副作用）
    ├─ migrations::run
    │    └─ 逐条 marker check → step → marker insert
    ├─ ensure_indexes
    │    ├─ unique / partial unique 幂等门
    │    ├─ query/sort 辅助索引
    │    └─ TTL 生命周期索引
    └─ cache warmup / workers / HTTP
```

模型层是混合 schema：多数顶层集合使用 snake_case BSON，部分集合和嵌套对象使用 camelCase；大量行业可变数据保存在 `Document` 容器中。数据库演进依赖“serde 读侧默认 + migration 写侧回填 + startup index”三层共同成立，任何一层字段名漂移都可能形成“Rust 可读但 Mongo filter/index 不命中”的静默分裂。

当前迁移框架没有全局锁或跨 step 事务。单进程顺序启动时可保证先迁移后索引；多副本并发首启是否安全尚未证明。数据层详细目录见 `data-model.md`。

## 外部边界与身份上下文

```text
/api request
    ├─ public exact paths: /health, /auth/login, /auth/token
    └─ require_session
         ├─ wa_session → Mongo admin_sessions → AuthenticatedAdmin
         └─ optional Bearer RS256 claims → AuthenticatedAdmin
                    │
                    └─ handler 直接用 current_workspace，或显式重读 AdminUser ACL

/webhooks/wechat
    └─ appId → WechatAccount(workspace/account/secret)
         → HMAC timestamp gate → in-process rate limit
         → message unique insert → contact → quiet-hours/debounce → Agent/outbox

MCP call
    └─ account_id → credentials → initialize/session → tool call → best-effort mcp_logs

LLM call
    └─ process-wide LlmRegistry → current single provider client → external endpoint

media
    └─ authenticated multipart → local disk workspace/sha shard + Mongo ContentAsset
         → human approve → upload to MCP → outbox delivery
```

已证明的租户边界不一致：API 数据路由多数使用 session/JWT 的 workspace；Webhook 入口能从 appId 得到 workspace，但 debounce runner reload 时丢失；MCP per-account 凭证解析固定 default workspace；LLM Registry 完全没有 workspace 维度。因而“集合中含 workspace 字段”不等价于运行时已多租户隔离，详见 SR-011～SR-015。

鉴权不是单一路径：`resolve_authorized_workspace` 会重读 AdminUser ACL，但仅保护显式调用它的 handler；直接消费 `AuthenticatedAdmin.current_workspace` 的 handler 信任 session/JWT 快照。Cookie session 固定到期且可通过删除 session 行撤销；JWT 无服务端状态，签出后只能等待到期或轮换签名 key。

媒体文件在本机目录、元数据在 Mongo，二者没有事务；当前单进程部署可读写同一目录，多副本必须额外提供共享持久卷或对象存储才能保持文件可见性。Webhook 限流、debounce、MCP session/roster single-flight、LLM registry 同样是进程内状态。

## Agent 运行骨架（阶段 4 进行中）

```text
trigger
  → typed runtime + RunBudget(task-local)
  → Reply Agent raw JSON
  → validate_and_promote
       ├─ tool_calling：仅校验 knowledge tool 名
       └─ final：必填/枚举/长度风险聚合
  → sufficiency tier → review/finalize → state-action gate → revision
  → outbox / dispatcher                            （后续子批次核实）
```

公共 LLM 入口统一做 usage、日志与有限 prompt 的进程级精确缓存。该入口没有 workspace 参数：日志固定写默认 workspace；缓存只按 prompt 内容与全局 prompt pack version 区分，provider 热切换不会自动失效缓存。

仓库中存在完整的 Run Envelope 原语与生命周期闭集，但生产 Gateway 尚未调用 started/terminal/panic-hook 三个入口。当前实际 run log 仍在决策产出后一次写入，因此“决策前失败也必有 started 记录”只是未接线设计和 ignored 集成测试覆盖的目标，不是当前生产事实。

### Decision → Review 纯逻辑链（B04B）

```text
contact/workspace context
  ├─ DomainProfile / taxonomy / memory / knowledge / assets（contact workspace）
  └─ Reply system+policy+task / Soul / Review system（default workspace）  ← 租户口径分裂
          ↓
RawAgentDecision → validate/promote → taxonomy normalize
          ↓
local review 或 LLM reviewer（可选第二 reviewer）
          ↓
dual gate
  ├─ hard: hallucination / grounding
  └─ soft: human-like / pressure / emotional / boundary → revision candidate
          ↓
finalize_review_for_send
  ├─ protocol / budget / verified-product / hold → block/hold
  └─ approved / active silence / soft revision → Gateway 后续分流（B04C 核实）
```

R5.4 verified-product 门并非独立识别产品声明：它只在 Reviewer 的 `claimAnalysis.requiresProductKnowledge=true` 时启动。Reviewer 漏报或字段缺失时，自有承诺词分类目前仅生成观测事件、不改变发送判定。因此“存在 verified chunk 硬门”不等价于“所有无背书产品承诺都会被硬门捕获”。

### Gateway 生产控制流（B04C）

```text
Inbound / FollowUp
  → precheck
  → knowledge route
  → Lean decision ── insufficient ─→ Relational/Full decision（最多一次升档）
  → Review（LLM 或 local）
  → finalize（protocol / budget / product / hold）
  → operation-state action gate
  → optional single-shot revision
       ├─ 二审通过 → 再过 state-action gate
       └─ error / timeout / 二审失败 → 恢复原稿为 revision_applied_approved  ← SR-023
  → final precheck / superseded guard
  → apply profile + memory
  → decision/run = outbox_enqueuing
  → segmented text enqueue → media/namecard enqueue
  → Dispatcher（后续子批核实）
```

Gateway 的正常生产路径会把 promote risks 交给 finalize，并在首稿及成功 revision 后执行状态动作门。管理发送没有 revision 通道，所以同时要求 finalize Approved 与 `review_passed`。但主链的 revision fallback 把安全含义不同的软闸混为一类：包括 boundary/privacy 低分在内的原稿会在改写故障或二审失败时被重新标为可发送，下游不再复审。

事件写入的租户裂缝已关闭：公共 Gateway/Outbox 事件 API 强制显式 `workspace_id`，Webhook 限流桶按 `(workspace_id, account_id)` 隔离，限流事件写入解析出的真实 scope。账号日发送软上限计数与真实 pacing 历史均按 `(workspace_id, account_id)` 聚合；软上限仍保持告警语义、不升级为发送硬门（SR-024/025 已部署并完成部署后动态验证）。

### Outbox 可靠投递闭环（B04D）

```text
Gateway enqueue
  → unique(workspace_id, account_id, idempotency_key)
  → pending
  → atomic FIFO claim + 180s lease
  → in_flight
  → second safety gate / account online / pacing
  → MCP send（150s timeout）
       ├─ success → sent
       ├─ failure → pending + backoff / failed_terminal
       └─ timeout or crash reclaim
            → chat_search / MCP-log post-hoc verify
                 ├─ hit → sent（不重发）
                 └─ miss → retry
  → all text segments sent
  → delivery finalizer
       → commitments + follow-up + task/review sent
```

Gateway 的 `outbox_enqueuing` 是可恢复过程态：Dispatcher 在宽限期后按实际文本段数幂等修复 review/task，并最后 CAS run log。已 sent 但投递后副作用未完成时，Outbox marker 与 review finalizer lease 支持重跑；不会把发送事实倒退为 pending。名片没有 post-hoc 已发核对，因此 timeout/crash 边缘窗口仍可能重复发送名片。

可靠投递状态机及其外围 SR-024～027 已按完整租户 scope 收口：事件与 Webhook 限流按 workspace+account，pacing 历史按 workspace+account，Outbox v2 幂等键和唯一索引按 workspace+account，Reaction stop 只取消同 workspace+account+contact 的 Outbox。部署后同源真实 Mongo 矩阵 4/4 证明跨 workspace 同名账号互不污染；真正 MCP 发送与 post-hoc 搜索的剩余边界继续由 SR-011 跟踪，不能由本矩阵外推关闭。

### Reaction 反馈回路（B04E）

```text
latest inbound
  → claim latest sent review (outcome pending → analyzing)
  → Reaction LLM
       ├─ contact memory/profile/polarity/dimensions（contact workspace）
       └─ reaction system/task prompt（default workspace）             ← SR-021
  → outcome + reaction_analysis + reviewer_misjudge_signal
  → best-effort side effects
       ├─ intent_trajectory push（cap 50）
       ├─ negative_example draft / needs_review
       └─ stop/cooldown → cancel same workspace/account/contact outbox  ← SR-027 closed/deployed
```

Reaction 是上一轮发送结果的旁路反馈，不是本轮回复的前置成功条件；失败不会阻断 Gateway。其 claim 恢复却不是带 fencing 的 lease：默认 60 秒后任何执行者都可把 analyzing 重置为 pending，而合法 LLM 重试墙钟可超过该阈值；旧 worker 最终仍可只按 `_id` 写回并执行副作用。单进程 Webhook runner 串行能降低概率，多副本不共享 PENDING，故旧、新分析覆盖与重复学习副作用成立，见 SR-028。

### Memory consolidation（B04F）

```text
Gateway / task / admin trigger
  → load-or-create OperatingMemory（tenant triple unique）
  → read pending candidates + conversation window
  → Memory consolidator LLM
       ├─ profile / candidates / messages（contact workspace）
       └─ system/task prompt（default workspace）                 ← SR-021
  → compact / deprecate / same-dimension resolve
  → memoryCard version OCC
       ├─ lost race → candidates remain pending + task retry
       └─ winner
            → confirmed tags / personality（best-effort Contact writes）
            → candidates consolidated
            → event + task sent
```

OperatingMemory 本体有 tenant triple 唯一键和 version OCC，能阻止同一旧版本的并发覆盖；事实 cap、stable id、弃用与同维冲突也有确定性保护。但 winner 后的 Contact、candidate、event 和 task 写入不是事务，且没有 consolidation phase marker：在任一步崩溃会重放候选或留下不可恢复的派生视图不一致，见 SR-029。Consolidator 自建 run_id 却不创建 run envelope，warnings 更新可能无目标行，属于 SR-019 的实际爆炸半径。

### Production Knowledge Agent（B04G）

```text
workspace/account verified catalog (top 30 of 400 candidates)
  → Knowledge Agent ≤4 LLM rounds
       ├─ list/open_document（tenant + account + active + verified）
       ├─ open_chunk（workspace + verified only）                    ← SR-030
       └─ follow/version（workspace + verified，account 丢失）       ← SR-030
  → answer: cited ⊆ opened
       ├─ contradiction / quote validity only instructed in prompt   ← SR-031
       └─ Router cited ∩ initial account corpus
  → selected verified chunks → Reply/Review
  → answer cache (workspace/account/query + partial corpus signature) ← SR-032
```

初始 catalog 与 Router 最终交集都维持 account 可见域，但正文下钻不是同一套 capability：`open_chunk`、superseded redirect 与 relation traversal 只带 workspace。外层交集不能撤回已进入 Knowledge LLM 上下文的其它账号正文，独立 `/knowledge/ask` 也没有该交集。引用完整性同样分裂：服务端保证“id 曾打开”，却不保证“可作支撑、quote 真实、anchor 有效”；contradiction 红线目前只是 prompt 约束。缓存再次缩短了修订闭环：top-30 id/confidence/count 不变时，正文或引用修改不会使 5 分钟旧答案失效。

### Knowledge Chat 工具循环（B04H）

```text
chat(account)
  → workspace-wide active documents + verified chunks             ← SR-030
  → LLM decisionPhase
       ├─ final → raw business payload
       └─ tool_calling → ≤6 tool calls
            ├─ in-memory catalog/search/open
            └─ Mongo audit/search/repair/log/open/anchor
                 （workspace only，account capability 丢失）       ← SR-030
  → failure streak / budget / 4-loop force stop
       ├─ normalized AgentDecision = final
       └─ caller returns last raw tool_calling JSON                 ← SR-033
  → route defaults missing business fields and persists turn
```

每个工具 dispatch 有独立 5 秒 timeout，错误会作为工具结果回喂；但循环的 30 秒限制只在每轮入口检查，既不包住 LLM，也不按剩余 deadline 限制同轮多个工具。因而终态和时限都在循环层与路由层之间断裂：内存 decision 已被强制 final 不代表业务 payload 已收敛，elapsed 超过 30 秒也不一定产生 Timeout。

### AgentTask Worker 与管理重跑（B04I）

```text
pending / retry
  → read batch
  → CAS {_id, old status} → running + claimed_at + attempt
  → heartbeat {_id, running}（仅常驻 Worker）
  → handler 自写终态
       ├─ Gateway → Outbox → sent
       ├─ memory / profile / outcome → sent
       └─ expired principal relay → 裸 MCP → Ok、仍 running          ← SR-035

stale scan 读旧 claimed_at
  → update {_id, running} → retry / failed（不比较 lease 版本）      ← SR-034
admin cancel → cancelled（不终止在途 future / 不撤销随后 Outbox）   ← SR-034

all accounts → daily outcome task insert
  → unique(workspace_id, kind, account_id, content)
  → m017 dedupe also groups by workspace/account/content
  → sibling workspaces retain one legal task each                    ← SR-036 closed/deployed
```

AgentTask 的所有权问题与 Outcome 调度的租户键是两个独立边界。Outcome task 已把 workspace 纳入唯一键与历史去重并完成部署后副本集验证（SR-036）；Task claim、取消和发送授权的剩余状态以对应 SR 的当前结论为准，不能用 Outcome 键修复外推。

### 决策请示与领导授权闭环（B04J）

```text
Gateway escalation request
  → resolve domain policy / harassment gate
  → insert pending + last_pushed_at=now
  → raw MCP card to principal                              ← SR-038

principal webhook(workspace, account A)
  → lookup decider in workspace
  → list pending(workspace, principal; account omitted)    ← SR-040
  → interpret LLM
       → sanitize verdict only
       → resolve + relay task
  → synthetic relay through Gateway
       ├─ customer exemption
       └─ model substance self-anchor → verified knowledge ← SR-037

every task-worker tick / every current domain config
  → list all workspace pending (domain absent)              ← SR-041
  → timeout
       ├─ raw MCP card → reassign without generation        ← SR-039
       └─ raw MCP holding reply → touch after send           ← SR-039/SR-042
```

请示台账把 workspace/account/contact 都存了下来，但查询与唯一约束没有始终消费这些维度；它也没有保存 domain 或不可变领导原文，导致账号作用域、策略归属和授权证据在后续阶段丢失。发送可靠性同样分裂：客户正式 relay 走 Gateway/Outbox，而首张领导卡、超时改派卡、链尾安抚和过期授权收尾仍裸 MCP。数据库 pending、`last_pushed_at_ms` 与 `last_holding_reply_ms` 因而不是“已送达事实”，并发 scanner 也没有可 fencing 的投递 generation。

### 动态画像、Taxonomy 与观测旁路（B04K）

```text
active DomainProfile(workspace)
  → profile_dimensions(participates_in_decision)
  → Reply typed fields + domainSignals
  → taxonomy normalize
       ├─ canonical/alias → canonical value
       ├─ deprecated → risk
       └─ unknown → candidate queue                         ← SR-045
  → retain declared dimensions
  → $set domain_attributes.{dynamic kind}                  ← SR-044

DomainProfile publish/activate
  → multi-step current/active updates, no unique/txn       ← SR-043

taxonomy aliases
  → no cross-entry ownership constraint
  → unordered first alias match                            ← SR-046

bayesianObservations[]
  → inbound evidence strength
  → history.len as cross-turn hits
  → same-turn duplicate dimensions count repeatedly        ← SR-047
```

动态画像链在 workspace 级 profile/cache 和 value 校验上已有隔离，但“配置声明”同时被当作 Mongo 路径安全证明，开放维度因此能与 `domain_attributes` 内系统状态争用物理命名空间。Profile 的生效指针又分散在每行 `current_version/is_active` 标志上，缺少事务和唯一约束，缓存只能消费异常态而不能判定唯一真相。

Taxonomy 的候选通道保持 fail-soft、人审后才进入正式字典；问题在于同一 run 有两个持久化责任点，且正式 alias 没有唯一归属。贝叶斯侧路目前明确不驱动行为，降低了同轮重复计数的即时风险，但若未来将 locked 槽位接入策略，必须先修复“数组项数冒充跨轮数”的时间语义。

### Shadow、交易事实与主动发送台账（B04L）

```text
Simulation / Prompt Shadow
  → load-or-create production memory                         ← SR-048
  → production Knowledge Agent
       └─ miss/low-yield → gap / structural writes           ← SR-048
  → Decide + Review only
       └─ no finalize / state-action / revision               ← SR-048

Prompt replay(source run)
  → message lookup by message_id only                         ← SR-049
  → historical score vs current mutable runtime               ← SR-049
  → completed >= 1 → eligible_for_release

Reply product claim
  → LLM reply_text + LLM quoted_product_ids
  → any quoted id exists in active catalog
  → entire claim treated as catalog-backed                    ← SR-051

Outbox delivered media/namecard
  → insert send ledger without outbox unique anchor            ← SR-050
  → outcome scan by workspace + wxid, account omitted           ← SR-050
  → namecard timeout/reclaim verifier always false              ← SR-052
```

影子链只隔离了客户发送副作用，没有把“读取时写入”和知识整改生产者隔离出来，也没有复用生产发送前的完整终态判定。Prompt replay 进一步把历史原始分数与当前可变运行态直接比较，因此其证据既可能跨租户错取源消息，也不能把差异单独归因于候选 prompt。

交易事实的投影来源和租户加载较严格，但目录背书仍由同一个不可信 LLM 同时提供正文和 product id，缺少服务端 claim—catalog 一致性证明。主动发送台账则在模型里保存 account，却在归因查询、索引与 API 中丢失该维度，并缺少与 Outbox 送达事实一一对应的唯一锚；名片恢复路径甚至明确放弃 post-hoc 核对。

### 管理小路由、Soul 与裁决提交边界（B05A）

```text
Soul draft / published row
  ├─ PUT {_id, workspace} mutates row in place
  └─ publish(target)
       → delete all sibling versions
       → set target published                                 ← SR-053

principal/admin decision
  → CAS escalation pending → resolved
  → insert principal_decision_relay task                      ← SR-054
       (no transaction / unique intent / reconciliation)

Simulation API
  → workspace+account+contact ownership checks
  → shadow engine (production read-side writes + partial gates) ← SR-048
  → evaluator treats no_reply as passed                        ← SR-048
```

这些路由大多正确从 admin session 注入 workspace，资源详情/更新也普遍把 workspace 放进 Mongo filter；主要裂缝不在横向 IDOR，而在跨集合提交与版本生命周期。Soul 用物理删除维护“唯一发布版”，既没有事务也没有数据库唯一约束；裁决台账与 relay task 则把一个业务提交拆成两次不可恢复写。二者在正常顺序测试里都表现正确，但任一中间失败或并发交错都会让持久状态与实际运行行为分裂。

### Prompt、审核队列与行业 Schema 生命周期（B05B）

```text
manual prompt draft(current=false)
  → delete non-evolution siblings
  → set status=active only                              ← SR-055
       ├─ runtime reads status=active
       └─ Evolution reads current=true

ops publish / rollout / rollback
  → promote target
  → demote siblings (separate writes, non-unique index) ← SR-008

taxonomy candidate approve
  → claim in transaction
  → canonical already exists
       ├─ skip taxonomy write
       └─ mark candidate approved                       ← SR-061

relationship suggestion approve
  → write contact.relationship_type
  → update suggestion without status CAS                ← SR-059
  → terminal row keeps full unique slot forever          ← SR-060

suspected deal approve
  → CAS pending→approved
  → validate contact/product/amount + append outcome     ← SR-057

all review payloads
  → client-controlled reviewedBy                         ← SR-058

DomainSchema(schema_id, versions)
  ├─ delete checks latest then deletes all versions       ← SR-056
  └─ activate: demote all → promote target, no txn/unique ← SR-056
```

这批路由普遍有 workspace 过滤，主要风险仍是“一个业务提交被拆成多个不受约束的持久状态”。Prompt 的 `status` 与 `current_version` 已成为两个互相漂移的生效指针；Ops 与 DomainSchema 又靠多次更新维护单 current/active。审核队列中，Taxonomy 的事务只覆盖新 canonical，关系建议没有 claim，疑似成交则把 claim 放在所有业务校验之前。请求体自填 reviewedBy 进一步使这些状态即使正常提交，也无法证明实际操作者。

### Management 命令、Outbox 管理与统一待审箱（B05C）

```text
authenticated admin → Management LLM plan
  ├─ advertised raw message_send_text → direct MCP             ← SR-062
  ├─ static Dangerous + code gate disabled → immediate execute ← SR-063
  ├─ command/tool running → side effect → ordinary final update ← SR-064
  ├─ write_deal_events(default verification)
  │    → staff_confirmed outcome                                ← SR-065
  └─ synthetic management-agent actor                            ← SR-058

Outbox in_flight
  → admin/reaction writes canceled
  → already-running send future still delivers
  → sent CAS misses, but sent event/finalizer still run          ← SR-066

Unified review inbox
  ├─ eight collectors; suspected_deal absent                    ← SR-067
  └─ summary count errors → 0                                    ← SR-068
```

Management 的 workspace/account 入口校验本身较完整，但执行边界随后分裂：动态 MCP 工具可绕开 typed 产品网关，静态 Dangerous 分类又没有形成默认硬门；命令与工具审计只是副作用前后各一次普通写，没有 durable intent、恢复 worker 或幂等游标。复用 REST handler 时还把真实管理员替换成固定代理身份，延伸 SR-058；MCP 调用只携带 account_id，继续继承 SR-011。

Outbox 管理端按 workspace 原子改状态，但 `in_flight` 不是可撤回的所有权协议：取消不会使已经越过二次门的 worker 失效，发送后代码又未检查 sent CAS 的 matched_count。统一待审箱则在产品覆盖与故障语义上各缺一层：正式成交核实队列不在八源聚合内，计数查询失败又与真实零待办不可区分。

### 运行配置、画像与方法论发布闭环（B05D）

```text
Playbook UI
  ├─ generate/optimize wire contract mismatches backend        ← SR-069
  └─ AI generate/optimize mutates runtime row directly          ← SR-070
       └─ default boolean changed by non-atomic multi-write      ← SR-071

DomainProfile risky publish
  → mutable side draft → rollout by bare profile id              ← SR-073

DomainProfile activate
  → switch profile active
  → publish generated state machine (best effort)
  → derive policy (best effort / runtime absence fail-open)
  → migrate contacts individually (best effort)
  → always HTTP success                                          ← SR-072

Operation Domain reset
  → delete every historical version
  → insert default current                                       ← SR-074
```

三个配置面暴露的是同一架构问题：版本行会被原地改写或物理删除，生效状态分散为多行布尔标志，跨集合派生产物靠顺序 best-effort 写维持。因而 `version` 不能作为可重放的历史事实，`is_default/current/active` 也不是受数据库保证的单一指针；API 的成功只证明 handler 走到尾部，不证明运行时配置 bundle 已一致提交。

系统级解法应收敛为不可变配置版本 + 单一 active pointer + durable rollout intent：先对候选内容及全部派生产物做完整校验，以 hash/version 绑定真实 actor 的确认，再用事务或 generation CAS 切换指针；长时间联系人迁移由可恢复 job 完成并暴露进度。Playbook、Profile、Domain、State Machine 与 Policy 不应各自发明一套局部发布语义。

### 活动提交与自治监控读模型（B05E）

```text
Campaign spec
  → preview audience
       └─ hidden status write; completed → previewed            ← SR-075
  → confirmed dispatch
       → CampaignSend unique slot
       → claimable AgentTask
       → backfill taskId
       → campaign completed                                      ← SR-076

Campaign UI edits after first preview
  → local form changes only
  → preview/dispatch consume original stored spec                ← SR-077

Autonomy dashboard request
  → 17 sequential counts + planner aggregation                   ← SR-078/SR-079
  → revisions query
       └─ one contact lookup per row                              ← SR-079
```

活动域把“已确认的不可变活动定义”“逐人派发 intent”“可执行 task”和“真实送达结果”分散在多类行中，却没有 durable generation 将它们绑定为一个可恢复提交。唯一索引只解决同 campaign/contact 的正常重复 insert，不能证明 task 已关联、未被 claim，或 campaign 汇总与真实执行一致。preview 还同时扮演读操作和状态迁移，使 completed 终态与 dry-run 零副作用都失去可信度。

自治监控的核心问题是读模型没有可复核时点：多个独立 count 在持续写入的集合上拼成看似同一份快照，索引又没有对齐 workspace 开头的真实查询形状。系统级改进应让 Campaign 采用不可变 spec + fanout intent/item 状态机，让监控采用固定 asOf 的单次聚合/物化读模型；前端只展示这些协议的真实状态，不以“已完成”或百分比掩盖未收敛事实。

### 联系人纳管与运营池读模型（B05F）

```text
contact import
  → partial candidate fields
  → unconditional identity $set(null)                            ← SR-081

single-contact enable
  → contact scoped by workspace + expected account
  → account/self-wxid lookup uses the same workspace + account    ← SR-080 closed/deployed

batch enrollment
  → contact.agent_status=managed
  → insert claimable initial_profile task                        ← SR-082
       → check managed once
       → long-running profile generation
       → unconditional profile write + managed                   ← SR-083

contacts dashboard
  → fetch up to 500 contacts
  → one latest-message query per contact                         ← SR-084
```

联系人当前把“期望纳管状态”“画像生成进度”和“是否可被生产 Agent 服务”压缩在一个 `agent_status` 中。批量启用先开放生产门再提交画像任务，任务又没有 enrollment generation；因此中间失败无法补齐，晚到任务还可推翻管理员刚写入的禁用状态。通用 AgentTask fencing 缺口由 SR-034 描述，但联系人层仍需要自己的 desired-state generation，确保画像结果只能提交到创建它的那一代授权。

读侧的 roster snapshot 已证明批量富化可行，最近消息却仍在联系人循环中逐行查询。系统级改进应让 Webhook 原子维护最近入站预览，或一次聚合按当前页批量 join；账号与联系人 helper 则必须统一消费不可拆的 `(workspace_id,account_id)` scope，导入字段使用 patch 语义而非把缺失解释为清空。

### Evolution 发布、回滚与观测闭环（B05G）

```text
workspace runtime flag
  → admin.current_workspace row
  → only default workspace/account worker consumes               ← SR-085

eligible proposal
  → evaluated against mutable runtime/base                       ← SR-049
  → no immutable base version/hash
  → release rebases old candidate onto latest current            ← SR-086

released prompt proposal P1
  → later P2/manual version becomes current
  → rollback P1 disables arbitrary current and restores P1 parent ← SR-087

release transaction commits production state
  → event insert may fail and return API error
  → post-release review intent may never be created
  → scanner marks completed before event, without claim/CAS       ← SR-088
```

Evolution 的核心 release transaction 已能保证“生产行与 proposal 状态”一起提交，但版本协议仍没有证明“提交对象就是被评估和确认的对象”。候选需要绑定不可变 base hash/generation；release 只能对该 generation CAS，rollback 也只能撤销当前仍由该 proposal 拥有的发布物。跨越后续版本的撤销应生成新的 revert proposal，而不是直接移动 current 指针。

发布后的事件与 24 小时观测不是可丢弃日志：它们决定管理员能否识别安全回归。应在业务事务内写 durable audit/review intent，由带 owner/generation 的 worker 幂等派发与 finalize。控制面同样必须显式建模 scope：要么 worker 枚举每个启用的 workspace/account 并隔离预算，要么禁止非 default scope 写入不会被消费的开关。

### 行业画像 AI 候选生成与人审可达性（B05H）

```text
untrusted LM JSON keys
  → recursive normalize_json_keys
  → char index used as UTF-8 byte offset
  → localized key can panic before draft insert              ← SR-089

AI generate / manual create
  → DomainProfile(current_version=false, is_active=false)
  ├─ default list requires current_version=true
  ├─ Ask-Human requires current_version=true && inactive
  └─ ProfilePublishCard loads default list then filters by id
       → draft has no official route to review/publish         ← SR-090
```

生成入口的 workspace scope、强制 inactive 和状态机独立校验构成了有效的 fail-safe：模型输出不会在一次调用内直接生效，建议字典也只进入候选层。但“安全地不激活”不能等同于“拥有可用的草稿协议”。当前草稿既不是 current，也没有独立状态与读取入口，导致列表、统一人审和发布卡三条正式路径共同把它隐藏。

系统级修复应统一 DomainProfile 的不可变版本状态机：draft 必须有显式查询与按 id 读取，review/publish 只消费同一 candidate id/hash，current 与 active 只在确认提交时移动。JSON 入口则应先按显式 schema 拒绝未知键，并使用 UTF-8 安全的映射；不能让通用键重写承担 schema validation。

### 用户运营 Guide 候选、确认与原子应用（B05I）

```text
current contact/memory/playbook/domain = base A
  → LM preview(candidate)
  → base changes to B before confirm
  → apply reloads B and overlays old candidate               ← SR-091

suggestedChanges
  ├─ tags → deprecated contact.tags orphan                    ← SR-092
  ├─ playbookPatch/domainRuntimeParameters
  │    ├─ scope/risk/readable text all self-reported by LM     ← SR-093
  │    └─ arbitrary Document shallow merge, no typed validate  ← SR-094
  └─ transaction: contact + memory + playbook + domain
       + event + preview applied (lease/token/CAS fenced)
       → commit
       → response-only reads may fail and return 502/404       ← SR-095
```

这一链路的事务底座明显强于其它多数管理写路径：workspace/account 作用域、claim lease、token fencing、逐目标 CAS、事件与终态同事务都是真实保护。但它只保证“确认时重新计算出的这组写原子提交”，没有保证这组写仍对应用户看到的冻结预览。候选缺 base identity，模型又同时控制变更正文与影响范围说明，使确认从服务端可验证授权退化为对模型摘要的信任。

系统级修复不应继续在各字段旁加判断，而应建立统一 candidate envelope：保存 base versions/hashes、规范化 typed patch、服务端 diff/scope/risk、created/confirmed actor 和 candidate hash；apply 只对冻结 base 做 CAS并持久化 committed outcome。全局 Playbook/Domain 变更使用独立强确认，展示性 health 在提交后异步刷新，不能反向改变已提交结果的 HTTP 语义。

### 顶层路由收尾：协作锁、晋升、评测与监控（B05J）

```text
Chunk inspector
  → POST in-process lock keyed only by chunk_id
       ├─ no chunk/workspace validation
       ├─ get→insert / get→remove, no owner generation CAS
       └─ write routes never check lease token                         ← SR-096

Lesson pending_review
  → insert peer_case draft
  → update lesson promoted without status CAS/transaction
       └─ crash/concurrency leaves duplicate untracked candidates       ← SR-097

Evaluation scenario
  ├─ official UI omits groundTruth; status defaults active
  ├─ missing/invalid truth coerced to 0 and account scope ignored       ← SR-098
  └─ simulation usage lives in RunBudget/llm logs
       but batch budget scans unrelated production run envelopes        ← SR-099

Observability response
  ├─ historical status inventory labeled sweep hit rate
  └─ top-level 24h mixes live inventory, 14d and 30d metrics            ← SR-100
```

Chunk revision 的 updated_at OCC 能拒绝一部分同时提交，但它不是协作 lease：不能证明请求持有锁，也不能把多 chunk merge 变成原子操作。锁若作为写权，必须持久化 `(workspace,chunk,owner,generation)` 并由所有 mutation 在提交时校验；若只是 presence，则产品和 UI 都必须明确其 advisory 性质。

Lesson、Evaluation 与 Observability 的共同问题是“展示对象没有绑定可复核事实”：晋升按钮不绑定唯一提交 intent，评测分数不绑定完整真值与本批 usage，监控百分比不绑定单轮 generation/asOf。系统级改进应分别采用唯一 promotion intent、typed immutable evaluation run/items、durable worker run records；每个响应返回自己的 scope/window/freshness，而不是依靠成功 toast、预算数字或顶层 24h 标签制造一致性。

### 知识核心写入、审核与修订协议（B05K）

```text
AI repair / generic client
  → PUT whole Chunk
  → source quote locates in parent document
  → apply_chunk_integrity sets verified without /verify             ← SR-101

split / merge
  ├─ archive source(s) first
  ├─ caller fields can overwrite split workspace/status/integrity    ← SR-102
  └─ later insert/update failure leaves partial committed graph       ← SR-103

revision timeline
  → stores patch + before/after hashes, no state snapshot
  → rollback reads values from previous revision.patch
  → UI labels result “rollback to this version”                       ← SR-104

official Chunk actions
  → patch/split/merge/relate payloads diverge from backend schemas    ← SR-105

repair/auto-verify observability
  ├─ client self-reports applied fields; event failure swallowed      ← SR-106
  └─ counters advance before revision write; write error swallowed    ← SR-107

document/chunk CRUD
  → replace/delete outside immutable revision/reference lifecycle     ← SR-108
```

当前 `apply_chunk_revision` 提供 updated_at OCC、锁字段恢复、数组 union、AI source 降级和 revision 记录，是可复用底座；但通用 PUT、原始 insert/delete 与多对象动作仍绕过其中部分或全部不变量。真正的系统边界应是单一 mutation service：创建、替换、拆分、合并、审核、删除都在事务中写 immutable before/after snapshot、业务行、引用/catalog intent 与真实 actor，并由状态机决定是否可进入 verified+active。

前端动作、AI repair 和批处理不能各自承担安全不变量。请求 DTO 应由共享契约生成；repair applied 与批处理统计必须从 committed revision outcome 派生；rollback 应创建一个以历史快照为内容的新 revision，并对当前 generation 做 CAS，而不是猜测相邻 patch。

### 知识导入、Chat 应用与外部摄取边界（B05L）

```text
admin-managed ingest source URL
  → background worker default reqwest GET + redirects
  → no scheme/IP/redirect revalidation or body cap                 ← SR-109

metadata(current workspace)
  ├─ chunks facet: workspace scoped
  └─ revisions facet: whole collection, no tenant key
       → global editors/activity exposed in Atlas                  ← SR-110

chat pending assistant turn
  → load by workspace+session across accounts
  → create/update Chunk
  → only afterwards mark turn applied, no claim/idempotency        ← SR-111

import candidate
  ├─ admission checks deprecated items, ignores chunks
  └─ insert document → loop insert chunks → best-effort revisions
       → failure/retry leaves partial or duplicate graph            ← SR-112
```

这些路径都已有局部正确控制：session middleware、workspace 首跳过滤、导入强制待审、Chat 更新走 revision、任务 action 闭集。但局部过滤不能替代提交与出站协议。外部 URL 必须先进入统一的 egress policy；所有导入/对话候选必须绑定不可变 id/hash，由服务端原子 claim，并在事务中同时写业务对象、revision、终态与 outcome。

`ChunkRevision` 缺 tenant key 使历史授权依赖可变主对象，既造成当前 metadata 跨租户聚合，也让物理删除后的历史无法可靠归属。revision、candidate、task 与外部 source 都应把 workspace/account 作为一等持久字段和索引前缀，而不是从调用上下文或存活关联行事后推断。

### Knowledge Wiki 修订、反馈与 Worker 所有权（B06A）

```text
generic Chunk patch
  → free-form BSON + client-selected human source
  → tenant/lifecycle fields are not locked
  → move row across workspace + set active/verified                 ← SR-113

single Chunk revision
  → overwrite provenance (included in content hash)
  → insert immutable revision
  → deserialize + CAS replace current row
       ├─ no-op appears changed
       └─ failure can leave an uncommitted timeline row              ← SR-114

catalog projection
  → best-effort enqueue random job
  → queued → processing without lease/reclaim/retry                  ← SR-115
  → backend {documents}/{item}, UI reads {items,total}/{total}       ← SR-118

feedback attribution
  → workspace usage logs contain account_id
  → regroup by (account_id, contact_wxid) inside workspace
  → Contact lookup requires (workspace, account, wxid)
  → same-wxid accounts remain attribution-isolated                   ← SR-116 closed/deployed

ingest source
  → list due snapshot, no source claim/generation
  → fetch + non-idempotent document/chunk import
  → finalize by source_id only; stale worker can overwrite new URL   ← SR-117
```

当前 revision 内核有可复用的纯校验、AI 降级与 updated_at OCC，但 OCC 只保护“当前行是否仍是刚读到的版本”，不能把历史、主行与 catalog intent 变成同一已提交事实。统一 mutation service 应只接受 typed command，从登录会话绑定 actor/tenant；事务内写 current state、immutable before/after revision 和单调 catalog generation，读侧只展示 committed revision。

周期 worker 的正确抽象不是“每轮扫描后尽量更新”，而是 durable work ownership：claim 必须带 owner/token/lease/generation，崩溃可回收，finalize 必须以 token+generation CAS。统计/归因还必须保留完整业务身份 `(workspace, account, contact)`；前端投影则需共享 schema 和显式 freshness，不能把未知响应形状静默降为 0。

### 知识日报与长任务提交协议（B06B）

```text
daily digest scope
  ├─ enabled cron enumerates every persisted (workspace, account)
  │    └─ one account failure does not stop later accounts
  ├─ chunk health reads account-private + workspace-shared(null) rows
  └─ compose/summarize prompts and LLM audit use the same real scope   ← SR-119 closed/deployed

digest run
  ├─ config exposes token/call limits, generator hard-codes 24000/8   ← SR-120
  └─ any analyzer/LLM failure upserts cards=[] over same daily row
       └─ partial also retains no partial cards                        ← SR-121

knowledge task
  → read pending → process-local session lock → pending→running CAS
  → execute external/business step
  → append completed_steps → write progress → finalize task
       ├─ no lease/reclaim/token/idempotent step; crash sticks running
       ├─ cancel does not fence an in-flight step/final summary        ← SR-122
       └─ many failure branches return Ok and are counted successful   ← SR-123

deterministic digest cardId
  → hash(account_id, report_date, canonical card semantics)
  → direct dismiss filters workspace + account + date + cardId
  → fenced worker filters task workspace + account (+ report date)
  → same cardId in sibling accounts remains isolated                  ← SR-124 closed/deployed
```

日报与任务共享同一个根问题：展示状态不是不可变执行事实。日报唯一行同时承担 last-success 与 latest-attempt，失败重算会抹掉健康快照；任务则把 running、step 副作用、进度 turn、summary 和 event 分散提交。日报应保存独立 run/generation，并让 current report 只指向最近 committed success；失败 attempt 与 partial artifacts 独立可查。

任务执行应以原子 lease claim 取得 `(task_id, owner, token, generation)`，每个 step 持久化唯一 identity、输入 hash、状态和 committed outcome。业务副作用使用同一 step identity 幂等提交，进度与终态从 committed outcomes 派生；cancel 递增 generation 或写 cancel token，使旧执行者无法继续副作用或 finalize。进程内 session mutex 和成功文案只能作为体验层优化，不能承担所有权或审计语义。

### Digest/Task 测试与派工确认边界（B06C）

```text
operator selects digest cards
  → Chat request carries natural language, not selected card ids
  → dispatch LLM sees first 20 undismissed cards
  → Chat confirmation posts plannedSteps + cardIds=[]
  → task create accepts empty/unknown cards and targetChunkId override
       └─ confirmed task is not bound to selected cards               ← SR-125

test suite
  ├─ hand-built JSON checks selectedCards contains plannedSteps.cardId
  ├─ BSON/string round-trips accept arbitrary status strings
  └─ public RunBudget tested with hard-coded defaults
       └─ production wiring and failure protocols remain unexercised
```

确认 UI 不是授权边界；只有服务端冻结且可重放验证的 candidate 才是。Digest dispatch 应签发包含 report generation、selected card ids、规范 steps/targets、actor 与输入 hash 的候选，task create 只能确认该候选，不能接受客户端重新描述执行目标。

测试也必须围绕同一边界：真实 handler fixture 应证明未选 card、空/未知 cardId、客户端 target override 和过期 report generation 被拒绝；真实 worker 测试应覆盖 claim 后崩溃、业务副作用后故障、取消 fencing 与结构化失败 outcome。平行实现的常量数组和 serde round-trip 不能替代这些提交协议测试。

### Knowledge Agent 质量门与闭环证据（B06D）

```text
offline “recall rate”
  → test creates expected chunk
  → expected ObjectId is injected into mock open_chunk + answer
  → same ObjectId set is used as hit ground truth
       └─ retrieval quality cannot affect the score                 ← SR-126

“maintenance closed loop”
  ├─ insert verified chunk directly
  ├─ set superseded_by directly
  └─ construct related_chunks directly
       └─ proposal/apply/revision/commit path is bypassed            ← SR-126

worker “redline”
  → invalid target / empty action returns Rust Ok
  → test treats Ok as expected success
       └─ false-success behavior is protected instead of detected    ← SR-126
```

PBT 对 `cite ⊆ opened`、排序全序、UTF-8 截断、去重和 signal 分类仍是有价值的局部防线；Router fallback 集成测试也确实穿过生产读链。问题在于测试名称和门槛把这些局部性质外推成了召回质量、维护闭环与 Worker 成功语义。

可复核质量门必须把 ground truth 与执行系统分离：独立标注 relevant set，只给被测系统 query/corpus；维护链从真实 command/candidate 开始，最终断言 committed revision 与审核状态；Worker 从持久 step intent 聚合 typed outcome。直接写目标终态只能用于 fixture 准备，不能成为“闭环已验证”的核心动作。

### Knowledge Ask 流式终态契约（B06E）

```text
answer_streaming returns Err
  → SSE adapter emits event:trace {tool:"error"}
  → channel closes and adapter emits normal event:close
  → AskView records trace but only native EventSource error sets error state
  → close clears pending; pending-only trace disappears
       └─ no answer and no visible failure                              ← SR-127

display contract
  → backend MAX_ROUNDS=4 and tests exercise roundsUsed=4
  → AskView renders roundsUsed/3                                       ← SR-127
```

流式 RPC 需要与传输关闭分离的封闭业务终态。`close` 只能说明字节流结束；成功、取消和失败必须在关闭前分别发送可机器识别的 `answer|cancelled|failed` 终帧。前端收到 close 时若尚无终帧，应 fail-closed 显示错误并保留已收到的诊断，而不是把空白结果视作正常结束。

测试也必须分层：Agent 内存 channel 测试锁 Step/Token/Final 内核顺序；Axum adapter 测试锁鉴权 workspace、query/schema、SSE event 名和 Err/Drop 映射；前端测试消费真实帧序列并验证每种终态。共享响应 schema 还应携带 `maxRounds`，避免 UI 再写死 `/3`。

### Knowledge 真模型证据协议（B06F）

```text
K2 fixture
  → DB static order includes 32 chunks in candidate cap 400
  → query relevance reranks B to initial catalog rank 1
  → test accepts open_chunk(B) as relationship traversal
       └─ A→B edge need not be traversed                              ← SR-128

implicit non-execution
  ├─ auto-verify catches item LLM error → processed=0 → HTTP success
  ├─ chat returns freeform/no patch/canApply=false → test accepts
  ├─ vision returns empty fence/chunkIds=[] → zero assertions iterate
  └─ absent-topic answer may cite irrelevant seeded chunks → warning only
       └─ target capability absent, test still passes                  ← SR-128

skip accounting
  → only top-level LlmUnavailable macro appends skip ledger
  → swallowed errors and successful empty/wrong branches append nothing
       └─ skip gate reports zero for implicit skips                    ← SR-128
```

真实 provider 不是证据模型。能力测试需要显式的 evaluation outcome：`attempted` 只说明发起调用，`observed_branch/artifacts` 证明能力发生，`assertions_run` 证明判据执行，`failed/skipped_reason` 说明为什么没有证据。只有目标 branch 与 artifact 均存在且其断言已执行，case 才能进入 pass；否则必须 fail 或形成可汇总的显式 skip。

依赖检索前提的 fixture 还必须在同次运行中调用生产入口证明前提。K2 应先断言 B 不在 initial catalog，再要求从 A 的关系边加载 B；若排序算法升级使 B 直接召回，测试应红并更新 fixture，而不是悄然退化为普通 open 测试。

### Knowledge 真模型质量裁决（B06G）

```text
quality case
  → target model produces output
  → judge calibration + K samples
       ├─ pass/fail → conclusive verdict
       ├─ divergent / insufficient / calib failure → log and return
       └─ empty vision artifact / chat timeout fallback → early return
  → cargo test reports success for every non-fail branch
  → shared skip gate counts only top-level LlmUnavailable ledger
       └─ inconclusive quality is represented as green + zero skip       ← SR-128

Q2 extraction matrix
  → 16 document types split train/holdout
  → deterministic source-token recall per document
  → hard floors + non-empty splits + generalization-gap assertion
       └─ remains valid even when judge is inconclusive
```

质量裁判的“尺子不可信”与“内容达标”是不同事实，不能共享绿色终态。矩阵应持久化或输出封闭 evaluation outcome，至少区分 `pass|fail|inconclusive|infra_skip`；公共门同时约束有效 verdict 覆盖率与 inconclusive 比率。确定性指标（如 Q2 原子事实 recall）应单独成门，judge 只增加语义质量证据，不应通过无结论返回抹平测试状态。

### Knowledge Cockpit 运行时契约（B06H）

```text
AutoVerifyPanel selection
  → JSON {confidence_threshold, human_audit_sample_rate, limit}
  → serde camelCase DTO ignores the first two unknown fields
  → backend executes threshold=7 + default sample rate
  → HTTP 200 and result counters render as if selection applied
       └─ operator policy is silently replaced by defaults             ← SR-129

ReviewChat "only this chunk"
  → JSON attachments:[{chunk_id:A}]
  → serde expects chunkId; attachment target becomes None
  → classifier/tool loop is no longer bound to A
  → response carries draftPreview, UI reads turn.patch/data.patch
  → sessionId can still be applied before verify(A)
       └─ displayed confirmation and committed candidate diverge       ← SR-130
```

Static fixture key equality is useful but is not an RPC contract. Each mutation needs one generated schema covering request and response, including nested objects and unknown-field policy. A review/apply flow additionally needs an immutable candidate identity; the UI must confirm the same target and patch hash that the server commits.

### Knowledge 详情读取与整替换编辑（B06I）

```text
GET /documents/:id
  → backend {item: {id, rawContent, contentHash, lineIndex, ...}}
  → api.get returns the envelope unchanged
  → DocumentsView treats envelope as DocumentDetail
       ├─ title falls back to list row, so form appears healthy
       ├─ detail.id is undefined → PUT /documents/undefined
       └─ rawContent/hash/index become null/[] in prepared replace body
  → focused test mocks flat DocumentDetail and therefore passes
       └─ production adapter failure is hidden by the test fixture          ← SR-131
```

Response envelopes are part of the command protocol, not cosmetic transport wrapping. A destructive replace must never depend on the browser round-tripping hidden source fields; the server should patch an identified/versioned row and preserve immutable or unedited fields itself.

### Knowledge 评审派生视图（B06J）

```text
GET /operation-knowledge/chunks (no status filter)
  → active + archived rows, capped/sorted by generic list API
  → classifyChunk
       ├─ rejected → contested
       ├─ needs_review + missing source → source_orphan
       ├─ needs_review + source → pending_verification
       ├─ broken relation → dependents_pending
       └─ never returns needs_review
  → ReviewView defaults to needs_review
       └─ deterministic empty first view despite real review candidates

Coverage dimension click (pricing/effectClaims/...)
  → stores initialDimFilter
  → renders dimension name in an informational banner
  → fetch/classify/visible ignore the dimension
       └─ every drill-down shows the same unfiltered derived list

archived row without quote/anchor
  → source_orphan
  → rendered under UI claim "only active knowledge"
       └─ navigation, filter and lifecycle scope diverge                 ← SR-132
```

A review queue should be a server-owned projection with an explicit scope and closed category/facet schema. UI navigation state is not a filter unless the same canonical value participates in the query and is echoed as effective input; dead category values and banner-only drill-downs must fail contract tests.

### Evolution 候选、发布与回滚所有权（B07A）

```text
worker tick (default workspace/account only)
  → runtime flag + contact bucket
  → cohort → threshold/prompt proposal
  → shadow replay → significance/evidence
  → eligible_for_release

two concurrent threshold releases for proposal P
  ├─ A reads P.status=eligible
  └─ B reads P.status=eligible
       A transaction: insert override O1 → set P=released → commit
       B transaction: insert override O2 → set P=released → commit
          └─ no status CAS; no unique source_proposal_id             ← SR-133

rollback P
  → update_one(active override for P) marks only O1 rolled back
  → set P.status=rolled_back
  → O2 remains active and runtime-visible
       └─ displayed rollback and effective threshold diverge         ← SR-133
```

Transactions make each request atomic but do not create command identity across requests. Evolution release needs a unique proposal-to-production outcome and an eligible-generation CAS; rollback must target that exact outcome and prove no active artifact remains. Candidate base identity and transaction-external event/review recovery remain the separate SR-086–088 concerns.

### Strategic Planner 发射协议（B07B）

```text
single process flag
  → one planner loop
  → every scanner binds default workspace/account only                 ← SR-134

candidate contact
  → count pending follow_up
  → count emit events for daily cap
  → optional count segment history
  → insert ordinary follow_up task
  → insert emit event
       ├─ no planner intent id / task unique key
       ├─ no atomic quota reservation
       └─ task and event are separate writes                            ← SR-135

calendar / renewal next tick
  → previous task already terminal
  → same date or entitlement remains due
  → per-segment "daily" counter starts from zero
  → another task is inserted until shared regular cap is reached       ← SR-135
```

Downstream task claim, Gateway review and Outbox idempotency cannot deduplicate two distinct upstream task ids. Planner needs a durable business intent before task creation, and day/segment quota must be a committed reservation rather than a retrospective event count.

### Worker 舰队所有权与身份边界（B07C）

```text
Import worker A claims J: pending → running
  → atomically generation=G1, token=T1; A starts LLM preview
  → lease expires; recovery writes running → pending
Import worker B claims J: pending → running
  → atomically generation=G2, token=T2
  ├─ B heartbeat/progress/final require {_id, running, G2, T2}
  ├─ old A writes require {_id, running, G1, T1} → zero-match + cancel
  └─ stale scanner freezes generation/token/claimed_at → cannot reclaim G2
       → SR-136 closed in working tree and real-rs0 redlines; deployment pending

workspace W
  ├─ account A + contact wxid X → signal key (type, X, message/time)
  └─ account B + contact wxid X → same signal key
       → unique (workspace, dedupe_key) collapses observations
       → stored signal has no account_id for later attribution           ← SR-137

Cold Contact candidate
  → count today's events
  → insert ordinary follow_up task
  → insert event
       ├─ no durable business intent / atomic quota reservation          ← SR-135
       └─ peer-case lookup filters workspace, not account                ← SR-030

Silence Signal tick
  → local emitted counter starts at zero
  → at most configured cap this tick, not this day                       ← SR-135
```

The remaining shared architectural gap is loss of durable identity at asynchronous boundaries. Import execution now has a fenced claim generation/token (SR-136); follow-up emission still needs a business intent plus quota reservation, and behavior observations need the full tenant/account/contact identity. A mutable status value, an audit event written afterward, or a workspace-only dedupe key cannot substitute for those identities.

### Prompt pack 初始化与编辑信任边界（B07E）

```text
ensure_prompt_pack_v2(workspace W)
  → probe any prompt_template
       ├─ Some → GC archived + per-key safe align
       ├─ None → first-time bootstrap
       └─ Err  → best-effort warning
                 → delete all W souls
                 → delete all W prompt templates
                 → delete all W playbooks
                 → delete all W domain configs
                 → reseed defaults + rebind contacts
                      ├─ read failure is treated as authorization to destroy
                      └─ no transaction / snapshot / recovery intent            ← SR-138

admin submits prompt old → new
  → forbidden-word scan on new
  → check only key-specific substring anchors
  → diff = non-empty lines present in new but absent from old
       └─ deletion-only change → diff="" → semantic review Pass
            ├─ reply.system / reply.task have no required anchors
            ├─ review/policy protect only selected fragments
            └─ redline-review prompt can delete its own judging rules          ← SR-139
```

Initialization is a state transition, not an error fallback: failure to prove emptiness must produce zero writes. Prompt review likewise needs a bidirectional, version-bound contract; an added-lines-only diff cannot establish that required semantics survived.

### Prompt A/B 选择与审计身份（B07E）

```text
Reply run for contact C, locale L
  → load_prompt_for_contact(key, C, L)
       → choose locale cohort
       → hash(C) % active_count
       → returns content + actual version Vbucket
  → decision builder uses content
  → discards Vbucket in _system/_policy/_task_version

Gateway persists DecisionReview
  → prompt_versions(keys) runs a fresh query
       → ignores C and L
       → sorts all active by version desc
       → records Vmax
  → API exposes promptVersions=Vmax
       └─ executed treatment Vbucket != recorded treatment Vmax              ← SR-140
```

A/B evidence is trustworthy only when selection identity travels with the content through the same call graph. Re-querying mutable active state after execution records what is newest, not what ran.

### 前端 Chunk 增量流与失效恢复（B08A）

```text
backend workspace-scoped ChunkEvent broadcast
  → bounded tokio::broadcast receiver
       ├─ normal locked/unlocked/revised
       │    → WebSocket → App CustomEvent bridge
       │         ├─ lock hook updates/reacquires lock
       │         └─ Inspector reloads only when revised.chunk_id == open chunk
       └─ receiver overflow
            → server cannot replay missing events
            → sends {kind:"lagged"}
                 → App groups lagged with hello and returns
                      ├─ no global cache invalidation
                      ├─ no snapshot reload / generation comparison
                      └─ lock heartbeat refreshes lease, not chunk content
                           → open Inspector can remain stale indefinitely       ← SR-141
```

An incremental UI cache needs a continuity proof. Workspace filtering protects confidentiality, but it does not make a lossy broadcast eventually consistent; once the server reports a gap, every dependent cache must become stale until a canonical snapshot is loaded.

### 前端账号切换与状态身份（B08B）

```text
account selector: A → B
  ├─ contactStore = one global contacts[] without loadedAccountId
  │    ├─ Overview sees non-empty array and skips GET contacts(B)
  │    ├─ late GET(A) may overwrite GET(B)
  │    └─ Command Center managedCount consumes the same A projection          ← SR-142
  │
  ├─ commandStore = one global commandResult without accountId
  │    └─ A pending_confirmation remains visible under B
  │         → POST /commands/runA/confirm with id only
  │              → backend CAS checks workspace + id + status
  │              → executes frozen run.account_id=A                           ← SR-143
  │
  └─ McpKeyForm component instance is reused
       → props change A-object-id → B-object-id
       → local plaintext key/baseUrl remain from A
       → save uses B URL with A secret                                         ← SR-144
```

An account switch changes authorization context. Cached projections, pending confirmations and secret drafts must either be keyed by that context or invalidated atomically; a new prop alone does not retag existing state.

### 微信账号登录 DTO 边界（B08B）

```text
MCP tools/call login_begin
  → result.structuredContent
       { session_id, qr_data_url, login_page_url }
  → MCP client returns object unchanged
  → account handler returns JSON unchanged
  → frontend reads
       { login_session_id, qr_code_base64, login_page_url }
            ├─ sessionId = undefined
            ├─ polling never starts
            └─ render guard hides QR and even the valid login_page_url          ← SR-145
```

An external tool result needs a typed adapter at the service boundary. Repeating an expected shape as a TypeScript interface does not transform or validate runtime JSON.

### User Ops 联系人详情的异步身份（B08C）

```text
open contact A
  → selected=A; hydrate draft(A)
  → loadMessages(A) starts five parallel GETs

switch/open contact B before A resolves
  → selected=B; hydrate draft(B)
  → loadMessages(B) starts

late A response
  → no request generation / selected-id check
  → global messages, memoryDraft, candidates, reviews, health = A
  → UI still targets selected=B
       → Save operating memory
            → reads current selected.id=B
            → reads unscoped memoryDraft=A
            → PUT /contacts/B/operating-memory with A's four groups
                 → backend resolves B from URL and commits normally            ← SR-146
```

Clearing selection on an account switch is not enough when old futures still own write access to global state. The response commit and the later mutation must both prove the same account/contact generation.

### Roster 准入与 Playbook 跨账号草稿（B08C）

```text
Roster snapshot
  → server marks known system row isNonHuman=true
  → UI folds it under “系统账号”
       → expanded card uses normal toggle (disabled only when managed)
       → submit drops isNonHuman
            → batch-enable checks account-self only
            → upsert managed + enqueue initial_profile                       ← SR-147

account A: edit playbook PA
  → global editingPlaybookId=PA; playbookDraft=content(PA)
account selector: A → B
  → reload playbooks(B)
  → editing id/draft remain PA
       ├─ Save under B UI → PUT /playbooks/PA
       └─ Set default under B UI → POST /playbooks/PA/set-default
            → backend resolves PA by workspace + id
            → applies PA.account_id=A                                         ← SR-148
```

Presentation labels such as “system account” or “current account” are not authorization boundaries. Admission must be enforced server-side, while editable resources must bind draft, resource id, account and version into one immutable submission identity.

### Cockpit 联系人草稿与 Guide 确认身份（B08D）

```text
contact selector: A → B (Cockpit component instance is reused)
  ├─ TagTrustPanel local editing/draft remains from A
  │    → Save emits tags[] without source identity
  │         → Store reads current selected=B
  │         → PUT /contacts/B/manual-tags with A draft                         ← SR-149
  │
  └─ guide preview(A) is still in flight
       → hydrateSelected(B) clears current preview
       → late response(A) commits global guidePreview=A without generation check
       → B configuration page renders A candidate without target identity
            → Confirm sends previewId only
            → backend claims frozen preview.contact=A and applies to A         ← SR-150
```

A server-side frozen candidate protects execution integrity only after the candidate is selected. The UI must also prove that the candidate displayed at confirmation time belongs to the currently visible account and contact.

### Cockpit 配置持久化与复盘账号 scope（B08D）

```text
Cockpit “运营风格模板” select
  → selectedPlaybookId changes in Zustand and method summary changes
       ├─ enable-agent body sends humanProfileNote only
       ├─ existing-contact save actions send no playbookId
       └─ runtime continues reading persisted contact.playbook_id              ← SR-151

non-default account B, contact CB
  → GET /decision-reviews?accountId=B&contactId=CB
       ├─ missing accountId → 400
       ├─ account A cannot resolve B/CB → 404
       └─ query {workspace, account=B, contact_wxid=W}
            └─ list/detail join RunLog with the same full scope                ← SR-152 closed/deployed
```

Visible selection, persisted configuration and query scope are separate facts unless one immutable identity is carried through the request. A UI control is not complete until the server echoes the saved binding, and a contact id must determine account scope rather than merely supply a reusable wxid.

### 运营池快照、勾选与提交账号（B08E）

```text
account A, ContactsView remains mounted
  → contacts snapshot = A
  → local selectedWxids = {A.wxid}

account selector: A → B
  → parent effectiveAccountId becomes B
  → selected contact is cleared, GET contacts(B) starts
  → old contacts(A) + local selectedWxids remain interactive
       → click “批量启用” before B snapshot commits
       → child builds candidates from contacts(A)
       → parent builds payload {accountId:B, candidates:[A.wxid]}
            → backend validates B but cannot prove candidate source
            → upsert Contact(workspace,B,A.wxid) + initial_profile             ← SR-153
```

A current account id cannot safely relabel an older list snapshot. Selection state must carry the same account and generation as the rows it selects, and the server must validate candidate membership in that source scope.

### Planner 阶段时间的事实来源（B08E）

```text
frontend PlannerView
  ├─ displayed stage = domainAttributes.customer_stage
  └─ “自此未变更” = domainAttributesUpdatedAt
                          ↑ refreshed by any domain attribute write
                            (intent / relationship / value tier / principal flags / ...)

production Planner stagnation scan
  └─ timing fact = domainAttributes.<stagnation_dimension>_updated_at
       fallback = domainAttributes.customer_stage_updated_at

same visible stage + different timing facts
  → UI can say “just changed” while production sees long stagnation             ← SR-154
```

An explanation panel must project the exact fact used by the decision engine. A container-level modification timestamp is not evidence that any particular field changed.

### Operations 快照与任务动作账号（B08F）

```text
account A, Operations snapshot loaded
  → global store.tasks = tasks(A)

account selector: A → B
  → GET five datasets(B) starts
  → tasks(A) remain visible and actionable
       ├─ “立即复核” → POST /agent-tasks/taskA/review-now
       │    → backend filter = {_id:taskA, workspace}
       │    → handler consumes persisted task.account_id=A
       │    → Gateway/Outbox may operate A customer
       └─ “取消” → POST /agent-tasks/taskA/cancel
            → backend filter = {_id:taskA, workspace}
            → A task becomes cancelled                                      ← SR-155

late response(A) after response(B)
  → no account generation guard
  → the same A snapshot can overwrite B again
```

A read query scoped to an account does not make the resulting UI state account-safe. The response commit and every task mutation must carry and verify the same immutable account identity.

### LLM 成本汇总窗口（B08F）

```text
GET /llm-usage?accountId=A
  → find logs(A), sort created_at desc, limit default 100
  → sum only returned cursor
       ├─ totalCalls = items.len()
       ├─ totalTokens = sum(last 100)
       └─ cacheHitRate = ratio(last 100)

frontend labels
  ├─ “调用次数”
  └─ “总 token”
       → rolling sample presented as account total                           ← SR-156
```

Detail pagination and aggregate truth are separate queries. If an aggregate is bounded to a sample, its window and truncation must be part of the public contract and visible label.

### Campaign 规范、报表快照与离线导出（B08G）

```text
Campaign create form
  → first preview creates persisted spec S1
       {account, title, intent, segmentFilter}
  → operator edits local form to S2
       → preview is cleared only for segment controls
       → same campaignId + empty preview body
       → backend still previews and dispatches S1                         ← SR-077

openReport(A) ─────────────── request A (slow)
openReport(B) → selected=B ── request B (fast) → report=B
request A returns last
  → global report=A without id/generation check
  → board renders A rows under selected B
  → CSV filename uses B while payload uses A                              ← SR-157

Campaign report name
  → Contact.remark || Contact.nickname
       → nickname may come from external roster
  → CSV esc handles comma/quote/newline only
       → = / + / - / @ prefix remains spreadsheet-active                  ← SR-158
```

Campaign 的代码级发送确认门只保护“是否执行 dispatch”，并不证明确认时所见规范、结果页所见活动和离线导出内容属于同一对象。活动规范应冻结为带 hash/version 的候选；报表 Store 应按 campaign id/generation 提交；CSV 则需把外部身份字段视为不可信数据并做公式安全编码。三者共同构成从圈选到复盘的身份链。

### 自治快照、质量治理与发送成效作用域（B08H）

```text
account A, Autonomy page loaded
  ├─ metrics/revisions = A
  └─ outbox rows = A (pending/in_flight, response includes accountId=A)

account selector: A → B
  → requests(B) start
  → component-local snapshots(A) remain visible
       └─ operator clicks “取消” on outboxA
            → POST /admin/outbox/outboxA/cancel
            → backend CAS = {_id:outboxA, workspace, cancelable status}
            → expected account B is never submitted or checked
            → A message becomes canceled                                  ← SR-159

late response(A) after response(B)
  → no request generation / account commit guard
  → metrics, revisions, or outbox(A) can overwrite B again

Quality evaluation scenario created by official UI
  → request omits groundTruth/contactSeed/accountId/status
  → backend defaults status=active and groundTruth={}
  → formula runner accepts workspace-wide active scenario                ← SR-098

Send Analytics
  → frontend sends no accountId
  → backend groups entire workspace by targetId
  → account-bearing ledger rows lose account dimension in projection     ← SR-050
```

账号筛选必须同时约束请求、响应提交和后续写动作；资源响应里携带 `accountId` 但前端丢弃它，与后端从未回显身份具有同样效果。workspace 级治理资源和统计可以是有意设计，但必须显式标注作用域，并保留账号下钻；否则它们会与全局账号选择器形成难以识别的混合语义。

### 内容资产与成交写入账号身份（B08I）

```text
account A, Content Assets loaded
  → assets = shared(workspace) + private(A)

account selector: A → B
  → request assets(B) starts
  → assets(A) remain visible; response accountId was discarded
       └─ edit / approve / toggle / replace / delete assetA
            → request carries asset id only
            → backend filter = {_id:assetA, workspace}
            → A production asset is changed from B context                 ← SR-160

Products & Deals, contact A selected
  → parent DealsTab.selected = contactA

account selector: A → B
  → ContactPicker reloads candidates(B)
  → parent selected contactA and deal form remain mounted
       └─ POST /contacts/contactA/deal-events
            → no expected account in request
            → backend resolves contactA by {_id, workspace}
            → appends staff_confirmed/payment_verified fact to A           ← SR-161
```

资源自身的账号归属能保证数据最终写回原账号，却不能证明操作者在当前界面选择了正确对象。账号私有配置与联系人事实必须把 expected account 从响应、组件状态一路携带到写端 CAS；有意 workspace 共享的产品目录、审核池和名片库则应明确标注共享作用域，避免全局账号选择器制造错误暗示。

### 请示策略作用域与可编辑语义（B08J）

```text
top account selector = A
  → GET /contacts?accountId=A
  → choose leader wxid L
  → PUT workspace/domain policy
       deciderChain=[{wxid:L}]     # source account discarded

customer on account B triggers escalation
  → resolve same workspace policy → L
  → insert pending {accountId:B, principalWxid:L}
  → MCP send through account B
       ├─ B can reach L → card delivered
       └─ B cannot reach L → send fails after pending insert               ← SR-162 / SR-038

GET current policy fails
  → UI draft = defaultPolicy()
  → save remains enabled
  → add one decider and PUT whole draft
  → unknown live policy overwritten without base version                  ← SR-163

documented off states
  ├─ deciderChain=[] → backend disables channel
  │    └─ frontend rejects empty chain                                     ← SR-164
  └─ quietHours omitted → all-day delivery
       └─ controlled inputs replace blanks with 0; existing value cannot be cleared
```

配置 scope、候选来源和最终副作用 scope 必须一致。若策略确属 workspace 级，就应显式建模每位决策人的可用发送账号；若无法保证，则应把策略下沉到 account。编辑器还需区分三种状态：已知生产值、加载失败的未知值、用户明确选择的空/关闭值。

### 模型供应商配置与生产切换（B08K）

```text
edit inactive provider
  → PUT DB only
  → explicit “activate”
       → production-impact confirm
       → Registry swap + two DB active writes

edit current active provider
  → optional connectivity test (not required, not hash-bound)
  → ordinary “save”
       → PUT DB
       → refreshed.isActive=true
       → Registry swap immediately                                      ← SR-165

clear timeout / retries / retry base
  → frontend omits field
  → backend only $set Some; never $unset
  → old override survives                                               ← SR-166

dedicated vision provider V
  ├─ edit supportsVision=false; isVisionActive remains true
  │    → UI says vision model, runtime excludes V
  └─ DELETE allowed when text isActive=false
       → runtime silently falls back or returns visionNotSupported       ← SR-167
```

生产切换不需要新的大型发布子系统，但需要一个不可混淆的最小事务语义：测试的是将要保存的 candidate，确认的是同一 candidate，swap 的仍是同一 candidate；失败时 DB 与 Registry 不能分裂。视觉侧只需维护 `isVisionActive => supportsVision` 和“删除/关闭前先解除或改派”两个不变量。

### 系统策略生产配置生命周期（B08L）

```text
taxonomy current row
  value.isTerminal=true / isReactivationTarget=true
  → list projection drops both nested fields
  → edit draft defaults both to false
  → save any field PATCHes false / false and invalidates cache
       ├─ terminal leaves Planner stagnation exclusion set
       └─ reactivation target leaves wake-up inclusion set                 ← SR-168

DomainProfile risky publish(source draft id=A)
  → clone sideline candidate id=B; response {pendingActivation,id:B}
  → UI confirms riskyFields but discards response id
  → POST /domain-profiles/A/rollout
  → A becomes current; confirmed B remains sideline; UI reports success    ← SR-073

ordinary “reset system prompt pack” button
  → no impact confirmation / snapshot / dry-run
  → delete workspace souls → prompts → playbooks → domain configs
  → reseed + rebind managed contacts                                      ← SR-138
```

版本化配置必须把显示、编辑、确认和实际生效绑定到同一持久身份。顶层契约键集相等不能证明嵌套生产字段完整；凡是前端可编辑且运行时消费的子字段，都需要深度契约或强类型投影往返测试。破坏性 reset 则应是可恢复的 bundle 发布，而不是普通按钮触发的跨集合 delete-before-insert。

### 共享弹窗与知识操作交互（B08M）

```text
FormDialog field #2/#3 keypress
  → setValues → Provider rerender
  → new inline onClose function
  → Overlay effect cleanup + rerun
  → focus first control
  → only first typed character remains in later field

Chunk Inspector
  ├─ split: mode + cutoff
  │    ├─ cutoff typing is interrupted
  │    └─ submitted {offset|regex} != backend {newChunks}                 ← SR-105
  └─ relate: target + kind + note
       ├─ note typing is interrupted
       └─ target_id/supports != backend targetId/closed kind              ← SR-105
```

Overlay 的焦点陷阱应把“弹窗进场”与“弹窗内容重渲染”分开：只在 closed→open 时设置初始焦点和滚动锁，字段 state 更新仅维护当前焦点。该修复须与 Chunk wire schema 对齐一起验收，否则只会把可填写表单接回仍然 4xx 的后端。

### 共享审核队列对象身份（B08N）

```text
ReviewQueue items = [A, B]
  → rows rendered without React key
  → admin opens/edits A; child local state = draft(A)
  → A is resolved and refetch returns [B]
  → React reuses position 0
       ├─ outer row props/title/action id = B
       └─ child local state/form/detail = A
  → submit
       ├─ URL targets B
       └─ body carries A's reviewed content                              ← SR-169

proven impact
  ├─ taxonomy: /B/approve + canonicalValue(A)
  ├─ escalation: /B/resolve + substance/constraints(A)
  ├─ lesson: /B/promote + title/body(A)
  └─ chunk/profile: stale A view + action URL(B) during async reload
```

审核对象身份必须由队列边界统一持有：`getId(item)` 同时作为 React key、busy key 与提交 generation 的来源。子卡不得只在首次 mount 从 props 初始化可写 state；id 变化必须清空旧草稿/详情，并在新 id 数据完成读取前禁用生产动作。服务端再以 expected id/generation/hash 校验，才能防止前端重排或迟到响应把人审内容应用到另一对象。

### Evolution 控制面与统计窗口（B08O）

```text
runtime flag document
  enabled=false                         ← UI calls this the master switch
  thresholdAutoReleaseEnabled=true      ← preserved by ordinary toggle PUT
  → next run_one_tick
       ├─ cohort selection sees enabled=false → empty cohort
       └─ auto_release runs unconditionally
            → checks env auto-release + child flag only
            → scans old eligible threshold proposals
            → may release production override                            ← SR-170

disable checkbox
  → local state changes to false before PUT
  → PUT fails
  → error rendered as ordinary message; no rollback/read-back
  → UI says off while database remains on                                ← SR-170

"last 7 days" cards
  → GET experiments?limit=20
  → server truncates newest 20 first
  → client filters those 20 by seven-day cutoff
  → default 6h tick creates about 28 envelopes in seven days
  → at least 8 in-window experiments and proposals are omitted            ← SR-171
```

The runtime flag must be evaluated as one hierarchical gate: the parent `enabled` bit vetoes cohort generation, evaluation, and every release side effect; child gates can only narrow an enabled parent. UI state must represent the committed server generation, not an optimistic checkbox. Time-window metrics likewise need a server-side `started_at >= windowStart` aggregation or cursor exhaustion through the cutoff—fixed list limits cannot substantiate a seven-day label.

### Outbox 契约与可操作对象身份（B08P）

```text
production OutboxEntry
  ├─ text: content="..."
  ├─ media: content="", mediaAssetId=A
  └─ referral card: content="", referralCardId=C

admin outbox projection
  → drops mediaAssetId / referralCardId
  → drops reclaimedInFlight / reclaimCount
  → frontend receives status + contact + empty content + time
       ├─ media row has no asset identity or type
       ├─ card row has no advisor identity or type
       └─ recovered delivery uncertainty is invisible
  → pending/in-flight row still exposes immediate cancel
  → operator can only guess which production object is canceled           ← SR-172

contract snapshot
  → constructs non-null asset/card ids
  → snapshots the intentionally reduced projection
  → frontend compares only projected top-level keys
  → semantic omission remains green
```

A contract snapshot proves stability, not sufficiency. Every actionable DTO must preserve the durable identity, tagged payload kind, scope, and uncertainty state needed to understand and confirm the action; tests should derive those requirements from the complete domain model and consumer behavior rather than blessing a reduced projection in isolation.

### SSE 连续失败预算与任务事实收敛（B08Q）

```text
knowledge task worker
  → writes turn only at start / completed step / summary
  → one step may spend multiple long LLM calls

browser EventSource
  → disconnect #1 → reconnect succeeds and fires native open
  → no business turn yet; keepalive only
  → disconnect #2 → reconnect succeeds and fires native open
  → ... historical attempt is never reset
  → cumulative disconnect #7
       → onGaveUp (default maxRetries=6)
       → no new EventSource                                              ← SR-173

TaskRail
  ├─ does not subscribe to reconnect/gave-up callbacks
  ├─ turn event only appends “step N”; it does not GET task again
  ├─ no periodic task polling
  └─ status/progress/error/final state remain at the old manual snapshot ← SR-173
```

SSE health should be modeled as a small state machine: connecting → open/healthy → reconnecting → terminal/gave-up. The retry budget applies to consecutive connection failures and resets on a proven healthy connection; business events may additionally refresh data but cannot be the only proof that transport recovery succeeded. After gave-up, bounded polling of the durable task resource must converge the UI to its committed terminal state.
