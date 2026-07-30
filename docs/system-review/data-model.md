# 数据模型、索引与迁移台账

状态：阶段 2 初版。依据 PR #223 冻结 head 的 `src/models.rs`、`src/db/mod.rs`、`src/db/indexes.rs` 与 m001–m032；后续阶段会为每个集合补齐全部读写方和真实查询形状。

## 存储约定

- 默认顶层 BSON 键为 snake_case；带 `#[serde(rename_all = "camelCase")]` 的模型或嵌套值使用 camelCase。索引与 `doc!` 查询必须按实际 BSON 键而非 Rust 字段名书写。
- 多租户主键通常是 `workspace_id`；`llm_provider_configs`、`campaigns`、`campaign_sends` 使用 `workspaceId`。部分全局/间接归属集合没有单值 workspace 键。
- `Document` 广泛用于 domain attributes、operation policy/state machine、runtime parameters、prompt snapshots、review scores、tool traces 等演进型结构；typed wrapper 与 serde default 不能替代 Mongo 查询侧的物理回填。
- 时间生命周期由业务状态和 TTL 共同决定。TTL 只适用于诊断日志、过期 session、终态 import job、operator memory、outcome metrics 等明确集合，不覆盖核心业务事实表。

## 集合分组

| 分组 | 主要集合 | 租户/业务键 | 已确认约束与生命周期 |
|---|---|---|---|
| 账号与联系人 | `wechat_accounts`, `contacts`, `roster_snapshots` | `(workspace_id, account_id)`；contact 再加 `wxid` | account/contact/roster 均有复合唯一键；单联系人 enable 已按 workspace+account 校验。导入为字段存在性 patch；批量纳管以 enrollment token、Contact CAS 与 durable initial-profile intent 防半提交/迟到复活；contact 的 outcome product 路径有 multikey 索引。 |
| 消息与任务 | `conversation_messages`, `agent_tasks`, `agent_events`, `import_jobs` | workspace/account/contact；message/task/event 幂等键 | message_id/dedupe partial unique；AgentTask 所有权修复见 SR-034；outcome aggregation 的 partial unique 与迁移去重均含 workspace（SR-036，已部署并完成部署后 `rs0` 验证）；ImportJob claim 使用 generation+token fencing，终态 24h TTL（SR-136，已部署并完成部署后验证）。 |
| 可靠发送 | `agent_send_outbox`, `agent_send_ledger` | workspace/account/contact/run/source event | Outbox v2 幂等身份与唯一索引均含 `(workspace_id, account_id)`，DuplicateKey 回读、账号 pacing 和 Reaction stop 取消也使用完整租户 scope（SR-025/026/027 已部署并完成真实 `rs0` 验证）；含 lease/retry/pacing/finalize 索引。Ledger 的剩余归因/送达锚问题见 SR-050；名片恢复无送达核对见 SR-052；在途取消与远端边界竞态见 SR-066。 |
| Agent 配置 | `agent_souls`, `prompt_templates`, `operation_playbooks`, `operation_domain_configs`, `operation_state_policies` | workspace + logical key + version | ops Domain/Policy 保留多版本历史，版本键 unique、logical scope 的 current 为 partial unique；publish/rollout/rollback 用副本集事务切换，异常零/多 current 读侧 fail-closed。Prompt/Soul/Playbook/Domain reset 的独立生命周期问题见各自 SR。 |
| 记忆与审查 | `operating_memories`, `memory_candidates`, `agent_decision_reviews`, `agent_run_logs`, `llm_call_logs`, `user_operation_guide_previews` | workspace/account/contact/run | operating memory 每 contact unique；run_id 索引；诊断日志默认 30d TTL；Guide preview 只有查询索引，无 base versions/hash、actor 或 committed outcome，旧候选可重放到新基线且提交后读失败会伪装业务失败（SR-058/SR-091/SR-095） |
| 知识核心 | `operation_knowledge_documents`, `operation_knowledge_chunks`, `chunk_revisions`, `knowledge_usage_logs`, `knowledge_gap_signals`, `domain_schemas`, `domain_profiles`, `catalog_rebuild_jobs` | workspace/account/domain/document/chunk | revision/gap/job 幂等或唯一键；usage 35d TTL；chunk 按状态、类型、有效期、置信度索引；catalog intent 与 revision/父文档 desired generation 同事务，worker 以 token/generation/lease fencing 原子推进 applied generation并暴露 freshness；domain schema/profile 与其它剩余风险见对应 SR。 |
| 知识工作站 | `knowledge_chat_turns`, `knowledge_daily_reports`, `knowledge_chat_tasks`, `knowledge_operator_memory` | workspace/account/session/operator/date | 日报每天每账号 unique；task pending scan；operator memory `expires_at` TTL |
| 字典与建议 | `system_taxonomies`, `taxonomy_candidates`, `relationship_type_suggestions`, `suspected_deal_signals` | workspace/scope/kind/value/contact | taxonomy canonical version unique且每 logical value 的 current 为 partial unique，版本切换使用事务；alias 无唯一归属（SR-046），合并既有 canonical 可假 approved（SR-061）；candidate 行唯一但同 run 双写使 occurrences 失真（SR-045）；其它建议/成交闭环见对应 SR。 |
| 产品与活动 | `products`, `campaigns`, `campaign_sends` | workspace(+account)+product/campaign/generation/contact | product id tenant unique；Campaign 草稿以 `specVersion/specHash` CAS 编辑，preview 零写；首次 dispatch 冻结 generation/spec/意图/受众。campaign send `(campaignId, contactWxid)` unique并以 `prepared→enqueued` 绑定确定性 task；task 先 `committing` 后放行为 `pending`，周期 reconciler 收敛中断点；Campaign BSON 为 camelCase。 |
| 管理与请示 | `management_agent_sessions`, `management_agent_messages`, `agent_command_runs`, `agent_tool_calls`, `agent_principal_escalations` | workspace/account/session/contact/domain/policy-version/delivery-generation | short_code 全局 unique；pending escalation 以 `(workspace_id, account_id, contact_wxid, category)` partial unique；请示同行冻结 domain、policy、领导发送账号与卡片正文。每代卡片通过确定性 Outbox source id 投递，只有 `sent` 对账后写 `last_pushed_at_ms`；裁决同行持久化 relay intent/task id，客户转述送达或授权过期后写 `relay_state=terminal`。Contact 以 escalation id 集合维护 awaiting owner，单条终结不会误清其它等待项。Management command 冻结 workspace/account/plan hash 与 execution token；tool call 以 scoped partial-unique intent、`prepared→executing→terminal|execution_unknown` 状态机防止未知副作用自动重放。 |
| 演化 | `experiments`, `proposals`, `shadow_replays`, `threshold_overrides`, `threshold_overrides_audit`, `post_release_reviews`, `evolution_runtime_flags` | workspace/account/experiment/proposal/gate | experiment id unique；proposal/replay/release 查询索引；audit append-only；prompt replay 源消息与历史/当前对照不满足隔离和单变量比较（SR-049）；worker 只消费 default scope（SR-085）；proposal 不冻结 base、rollback 不验证发布所有权（SR-086/SR-087）；post-release review 无唯一 intent/claim/finalize（SR-088） |
| 认证与 provider | `admin_users`, `admin_sessions`, `llm_provider_configs` | username/session/workspaceId/providerId | username/session/provider tenant unique；session TTL；provider BSON camelCase，密钥明文模型字段 |
| 采集与评测 | `behavior_signals`, `behavior_signal_metrics`, `ingest_sources`, `agent_outcome_metrics`, `evaluation_scenarios` | signal 仅 workspace/contact，遗漏 account；其余按 date/source/scenario | signal dedupe partial unique 也遗漏 account（SR-137）；source/scenario unique；outcome metrics TTL 默认 90d |

## 关键状态闭集

| 实体 | 闭集 |
|---|---|
| `AgentTask.status` | `pending`, `running`, `committing`, `retry`, `failed`, `cancelled`, `sent`, `completed`, `outbox_enqueued` |
| `ImportJob.status` | `pending`, `running`, `completed`, `failed` |
| `Campaign.status` | `draft`, `previewed`, `confirmed`, `dispatching`, `completed`, `canceled` |
| `AgentCommandRun.status` | `running`, `pending_confirmation`, `succeeded`, `failed`, `dry_run`, `canceled`（模型无集中闭集断言） |
| `AgentToolCall.status` | `running`, `dry_run`, `succeeded`, `failed`, `executed_unverified` |
| `KnowledgeChatTask.status` | `pending`, `running`, `completed`, `failed`, `cancelled` |
| principal escalation | status=`pending|resolved|delivery_failed`；card delivery=`pending_enqueue|queued|sent|failed_terminal|delivery_unknown`；relay=`pending|enqueued|terminal`；verdict=`approved|rejected|conditional|deferred|delegated_back` |
| outbox（模型注释/索引） | `pending`, `in_flight`, `sent`, `failed_terminal`, `canceled`；写点待阶段 4 反查 |

## 迁移演进摘要

| 波次 | 迁移 | 作用 |
|---|---|---|
| 基础兼容 | m001–m005 | 消息时间拆分、memory activeFacts 拆分/结构化、状态机来源、metric id |
| autonomy/taxonomy | m006–m008 | 默认字典、outbox marker、commitment 结构化 |
| 版本与清理 | m009–m015 | prompt/ops 多版本、知识字段、开发期清理、state policy、active-version 字段 |
| 多租户与状态 | m016–m019 | workspace 回填、task 去重、domain attributes 回填、state flags |
| 行业字典 | m020–m028 | purchase lifecycle、churn、dormant、value tier、relationship、ask-human、trust、conversation mode |
| 可靠性收口 | m029–m049 | 联系人身份、发送/版本协议、review cycle、请示 account-scoped pending 索引、principal awaiting owner 回填、ops 三表单-current收敛，以及旧 m043 marker 数据库的 Prompt planning-current 纠正重跑 |
| 数据修复 | m029–m032 | contact identity、outcome 默认值、escalation push 时间、taxonomy workspace |

## 已确认的契约裂缝

- m009 的 active/current 选择不符合注释，见 SR-007。
- m029 roster 回填映射丢失 tenant/account 维度，见 SR-009。
- production guard 返回成功后仍记 marker，见 SR-010。
- m049 以独立 marker 复用 m043 的 Prompt 全表 validation-before-write；2026-07-25 生产升级后 planning-only `group.policy`/`moment.policy` 保持 draft 且不再是 current，见 [生产发布记录](production-release-2026-07-25.md)。
- principal escalation 的 account/domain/policy/delivery 身份已在 HC-013 收口；旧行由 m046/m047 审计或精确回填，不猜测旧 resolved 历史。
- `last_pushed_at_ms` 现在只由 Outbox `sent` 事实写入；`last_holding_reply_ms` 仍是安抚节流时钟，不代表业务送达凭据。
- `domain_profiles` 的 version/current/active 都没有数据库唯一约束，且生效切换为非事务多步写，见 SR-043。
- `ProfileDimension.kind` 没有 Mongo 路径/保留名约束，开放信号与 `domain_attributes` 系统状态共用命名空间，见 SR-044。
- taxonomy candidate 的 occurrences 被 Decision/Gateway 双写，正式 alias 又没有同 kind 唯一 claim，见 SR-045、SR-046。
- `bayesian_signals.history` 用数组项数代表跨轮 hit，但同轮同 dimension 可重复追加，见 SR-047。
- Shadow/Simulation 会写 production memory、operator-memory 使用时间和知识整改队列，且只跑 Decide/Review、不跑生产 finalize/state-action/revision，见 SR-048。
- Prompt Shadow 按裸 message_id 取源消息，并用当前可变运行态直接对比历史分数；completed 即可成为 eligible 证据，见 SR-049。
- `agent_send_ledger` 保存 account 但归因读侧省略 account，也没有 outbox_id unique，见 SR-050。
- 产品目录背书只验证 LLM 自报 id 存在，不证明最终回复中的价格/声明与目录一致，见 SR-051。
- 名片 Outbox 的 timeout/reclaim 路径固定跳过送达核对，远端成功但本地未知时会重发，见 SR-052。
- `agent_souls` 没有版本/单 published 唯一约束，发布先物理删除历史再更新目标，见 SR-053。
- escalation 的 resolved 提交与 relay task 插入是两个独立写，失败后没有可重试 intent 或 reconciliation，见 SR-054。
- 手动 Prompt 发布只切 status、不切 current/previous，并先物删非 evolution 历史；运行时与 Evolution 消费不同生效指针，见 SR-055。
- `domain_schemas` 只按 workspace/schema/version 与 workspace/active 建普通索引；删除按最新版本判断后删除全血缘，active 切换非事务且运行时无序 find_one，见 SR-056。
- suspected deal 在业务校验前 CAS approved，关系 suggestion 则先写 contact 后无 CAS 提交审核状态；两者均缺可恢复的 processing/intent，见 SR-057、SR-059。
- Taxonomy/relationship/suspected-deal 审核主体来自客户端 reviewedBy，正式成交 marked_by 也继承该值，见 SR-058。
- relationship suggestion 的终态行占用全量 unique 槽，后续新证据无法形成新 pending；taxonomy 合并既有 canonical 则提交 approved 但不写 raw alias，见 SR-060、SR-061。
- Management 原始 `message_send_*` 已从动态目录剔除并在执行兜底拒绝；主动发送只走生产 Gateway/Review/Outbox。所有真实副作用默认要求代码层确认，未知工具 fail-closed；见 SR-062、SR-063 的修复证据。
- `agent_command_runs` 冻结 account/plan hash 并持有 owner lease；`agent_tool_calls` 以 scoped partial-unique intent 和 CAS 状态机提交。崩溃遗留的 `executing` 收敛为 `execution_unknown` 且禁止自动重放，见 SR-064 的修复证据。
- Management 的 `write_deal_events` 只允许显式确认路径传入真实认证管理员，LLM 仅起草 payload；`staff_confirmed.marked_by` 固定为认证用户名，见 SR-065 的修复证据。
- Outbox 允许把 `in_flight` 标 canceled，却没有 generation/cancel token 阻止已运行 worker 发送及 finalize，见 SR-066。
- Ask-Human 聚合遗漏 suspected deal，summary 又把数据库计数错误降为 0，见 SR-067、SR-068。
- `operation_playbooks` 的 version 只是原地覆写计数，联系人绑定不按 version 冻结；default 由无事务多步写维护且没有唯一约束，见 SR-070、SR-071。
- DomainProfile 激活跨 profile、state machine、state policy 与 contacts 顺序 best-effort 写，确认 intent 又没有内容 hash/version/actor/TTL，见 SR-072、SR-073。
- Operation Domain reset 先删除该 logical domain 的所有版本再插默认，历史与恢复锚均不存在，见 SR-074。
- Campaign preview/草稿/dispatch 已按 HC-021 分离：preview 零写，草稿 PATCH 带 version CAS，dispatch 绑定 preview hash 并冻结受众；SR-075、SR-077 已修复，Mongo 副作用红线待 Docker 环境复验。
- `campaign_sends` 已成为逐目标 durable intent，并与 deterministic task、generation 和后台 reconciler 组成可恢复提交协议；SR-076 已完成代码与协议单测，待真实 Mongo/worker 崩溃复验。
- 自治监控以多次非快照计数拼接同一响应，且 run/outbox/revision 查询与现有索引形状不一致并含 contact N+1，见 SR-078、SR-079。
- 单联系人 enable 已按 workspace+account 隔离，账号注册与 self-wxid 判断使用联系人同一租户身份；SR-080 已正式部署，并以跨 workspace 同 account、外域 self-wxid 恰等于目标联系人的真实 Cookie Router 反例完成部署后 `rs0` 验证。共享导入 helper 使用字段存在性 patch，缺失/null 身份字段不再覆盖旧值，见 SR-081 修复证据。
- 批量纳管已改为 `committing intent → Contact CAS → pending task` 的可恢复提交协议；画像写回由 enrollment token + task claim generation fencing，disable/hide 旋转代际并退休旧任务，见 SR-082、SR-083 修复证据。
- 联系人列表已按当前页 wxid 一次聚合最新入站消息，删除逐联系人查询，见 SR-084 修复证据。
- Evolution runtime flag 是 workspace 级，但唯一 worker、proposal 生成与 auto-release 固定 default workspace/account，见 SR-085。
- Evolution proposal 未保存不可变评估基线，Prompt rollback 也不验证当前版本仍由该 proposal 拥有，见 SR-086、SR-087。
- Evolution 生产提交后的 event 与 post-release review intent 不在同一提交协议，review scanner 又无 claim/CAS 且先终态后事件，见 SR-088。
- `threshold_overrides` 允许同一 `source_proposal_id` 出现多条 active 行，Proposal 也不保存唯一 release outcome/generation；并发发布可重复生成覆盖，而 rollback 的 `update_one` 无法证明全部产物已失效，见 SR-133。
- `agent_tasks` 对 Planner `kind=follow_up` 没有 business intent/dedupe key；现有任务唯一索引只覆盖 `outcome_aggregation`。`agent_events.dedupe_key` 虽有 partial unique，但 Planner 不写该字段，且 task/event 分步提交，无法为主动触达提供幂等所有权，见 SR-135。
- Planner 的账号/分段“每日配额”没有独立持久模型；共享 cap靠当日 emit event 事后计数，calendar/renewal/reactivation 的专属 cap只是函数内计数器。模型无法原子证明某日某 scope/segment 已占用多少配额，见 SR-135。
- `import_jobs` 已为每次 claim 持久化单调 `claim_generation` 与不可复用 `claim_token`；heartbeat、progress、终态和 stale scanner 的恢复 CAS 均绑定冻结所有权，旧执行不能覆盖重领后的新代次。m056 负责 legacy generation 的 fail-closed 幂等初始化，见 SR-136（真实副本集红线已通过，待部署）。
- `behavior_signals` 的实体身份和 dedupe key 都没有 `account_id`，而 Contact 以 `(workspace_id,account_id,wxid)` 唯一；同 workspace 跨账号信号可碰撞，未碰撞样本也无法按账号归因，见 SR-137。
- 行业画像生成会递归改写不受信任的 JSON 键，但以字符序号作为 UTF-8 字节偏移切片，非 ASCII 键可在草稿写入前触发 panic，见 SR-089。
- AI 与手工 DomainProfile 草稿都写 `current_version=false`，而默认列表、Ask-Human collector 与发布卡读取链只接受 current，导致正式人审与发布路径不可达，见 SR-090。
- Guide preview 不冻结 contact/memory/Playbook/Domain 基线，确认时会把旧候选覆盖或合并到最新生产对象，见 SR-091。
- Guide 把标签写入已废弃的裸 `tags`，同时允许模型自报范围/摘要控制全局配置确认，并把任意 Domain runtime Document 绕过 typed 校验浅合并，见 SR-092、SR-093、SR-094。
- Guide 事务提交后仍依赖多次展示性读取；失败会返回 502/404，而相同 preview 重试只能得到 applied 冲突，见 SR-095。
- Chunk 编辑锁只存在进程内并以裸 chunk_id 为键，没有 workspace、owner generation 或持久 lease；真实 revision 写不校验锁，见 SR-096。
- `lessons_learned` 只有列表索引，没有唯一 promotion intent/source_lesson 约束；peer_case insert 与 lesson promoted 回写分离，见 SR-097。
- `evaluation_scenarios` 仅保证 `(workspace_id,scenario_id)` 唯一，没有 ground-truth/status/account schema；评测 run/item 与真实 token usage 也没有独立持久模型，见 SR-098、SR-099。
- Observability 直接拼接实时集合库存与 14d/30d 滚动文档，没有统一 worker generation/asOf；历史状态比例被命名为 sweep 命中率，见 SR-100。
- `OperationKnowledgeChunk` 的安全状态可被通用 PUT 在锚点命中时直接提升为 verified；split 原始 Document 又允许 caller 覆盖 workspace/status/integrity，说明持久模型不变量没有由统一写服务强制，见 SR-101、SR-102。
- split/merge 跨多个 Chunk 与 revision 独立提交，没有 transaction 或 operation intent；失败可留下已归档源和不完整目标，见 SR-103。
- `ChunkRevision` 只有 patch 与 before/after hash，没有可恢复的前后快照；现有 rollback 无法重建任一历史状态，见 SR-104。
- 前端 Chunk 操作 DTO 与后端 serde schema 已漂移，repair applied 事件也没有 proposal/revision/hash 外键，无法证明 UI 动作或审计行绑定真实提交，见 SR-105、SR-106。
- auto-verify 没有 per-item committed outcome，批次计数可先于失败写入；文档/Chunk 物理 replace/delete 又绕过 revision、引用与 catalog 生命周期，见 SR-107、SR-108。
- `ingest_sources.url` 是可持久化的后台网络目标，但模型/CRUD 没有 scheme、解析后 IP、重定向或响应上限策略；worker 会周期执行该值，见 SR-109。
- `chunk_revisions` 没有 workspace/account 字段，导致 metadata 只能全库聚合或依赖主 Chunk `$lookup`；当前端点已把全局 editor/activity 返回给单租户，见 SR-110。
- `knowledge_chat_turns` 的普通索引虽包含 workspace/account/session/turn，但 session sequence 与 apply identity 只用 workspace/session，且没有 pending-turn claim、apply token 或 source-turn 唯一 outcome，见 SR-111。
- 导入没有持久化 candidate/apply intent 或文档—Chunk 原子提交边界；准入仍依赖已删除的 items 实体，文档、Chunk 与 revision 可形成半提交和重复集合，见 SR-112。
- `OperationKnowledgeChunk` 的通用 revision patch 仍可改写 `workspace_id/account_id/document_id/domain/status/integrity_status`；replacement filter 用旧 workspace、replacement 本体却可带新 workspace，模型身份和审核状态没有不可变约束，见 SR-113。
- `ChunkRevision` 没有 workspace、committed state、before/after snapshot 或 current-generation 外键；写协议先 insert revision 后 replace 主行，且 provenance 参与 hash，使未提交动作与相同内容重复动作都可能进入正式 timeline，见 SR-114。
- `catalog_rebuild_jobs` 已具备 queued/processing/done/superseded/discarded/failed 闭集、owner/token/claim generation、lease heartbeat、过期回收、有界退避和 desired/applied generation fencing；m052 为历史文档补 reconciliation intent，读侧以 `catalogFresh` 区分陈旧快照。SR-115 已进入正式 ELF，并完成部署后副本集动态验证。
- `KnowledgeUsageLog` 持久化 workspace/account/contact 三维，`Contact` 以 `(workspace_id,account_id,wxid)` 唯一；成交追认现按 `(account_id,contact_wxid)` 分组并以完整三元身份读取 Contact。SR-116 已正式部署，候选与部署后同 wxid 双账号动态红线均证明归因不跨账号。
- `IngestSource` 已具备配置 `source_generation` 与 claim owner/token/generation/lease；管理员更新递增配置代次并撤销旧 claim，Worker 原子领取、heartbeat、过期回收，并以完整配置快照和有效 claim fencing 所有终态。内容变化时知识图与 checkpoint 同事务提交，旧 URL/旧 owner 晚到会整批回滚；m053 幂等补齐历史代次。SR-117 已进入正式 ELF，并完成部署后副本集动态验证。
- Catalog persisted/live API 没有共享响应模型：后端分别返回 `{documents}` 与 `{item}`，正式前端自定义 `{total,items}`/`{total}` 并把未知形状回落为 0，见 SR-118。
- `KnowledgeDailyReport` 以 `(workspace_id,account_id,report_date)` 唯一，但同一行同时充当最近 attempt 与成功快照；失败/超预算 upsert 会把既有 cards 覆盖为空，模型没有 run generation、last-success 指针或可保留的 partial artifacts，见 SR-121。
- Digest 已统一 tenant-scope：启用后的定时 helper 枚举所有持久 `(workspace_id,account_id)` 并逐账号隔离失败；Chunk health 可见域为当前账号私有行加 `account_id=null` 的 workspace 共享行；compose/summarize Prompt、LLM 审计、run 与报告读写继承同一真实 scope。SR-119 已正式部署并完成候选及部署后动态红线；当前环境仍配置 `KNOWLEDGE_DIGEST_ENABLED=false`，代码能力上线不等于运营启用。
- Digest 配置持久契约声明 token/call 上限，执行模型却固定构造 `RunBudget(24000,8)`；报告中的 budget snapshot 记录硬编码限制而非部署配置，见 SR-120。
- `KnowledgeChatTask` 只有 pending/running/terminal 状态与 started/finished 时间，没有 claim owner/token、lease、heartbeat、attempt、generation 或 next-step cursor；`completed_steps` 也没有唯一 step identity/committed outcome 约束，见 SR-122。
- `StepOutcome` 没有 typed success/failure 状态，多个业务失败被编码为 Rust `Ok`，再落成 `completed_steps.status=ok`；模型无法区分“成功”“跳过”“需手工处理”和“副作用失败”，见 SR-123。
- Digest cardId 已由 `account_id + report_date + canonical card semantics` 稳定派生；直接 dismiss 使用 workspace+account+date+cardId，fenced Worker 使用 Task workspace+account（及报告日期）提交。SR-124 已正式部署；部署后同 cardId 双账号 Router/Worker 红线证明非目标日报完整 BSON 不变。
- `KnowledgeChatTask.cards` 只是由请求 `cardIds` best-effort 反查的快照，`planned_steps` 没有 candidate id/hash、report generation 或 card→action→target 外键；Chat 正式路径可提交空 cards，客户端 targetChunkId 仍被直接持久化，任务模型无法证明执行内容来自运营选中的卡片，见 SR-125。
- Knowledge 的测试证据没有独立的 immutable evaluation run/item 或 committed mutation outcome：离线评测把 expected Chunk id直接注入 mock 输出，闭环测试直接写 verified/关系/取代终态，Worker 测试又以 Rust `Ok` 代替业务成功；现有绿色结果不能作为召回率、维护闭环或 step outcome 的持久事实，见 SR-126。
- Knowledge Ask 的 SSE 协议没有封闭业务终态模型：`TraceEvent::Step` 同时承载普通进度与 Agent failure，传输层 `close` 又同时结束成功、取消和失败；前端无法从类型上证明一次流已得到 answer 或 failed outcome。响应也不携带 `maxRounds`，导致 UI 将运行上限另写成 3，见 SR-127。
- Knowledge 真模型测试没有持久或结构化的 per-case evaluation outcome；显式 `LlmUnavailable` 才写公共 skip ledger，handler 内吞错、`processed=0`、空 artifact、错误 intent 和未执行的条件断言都没有状态字段。质量套件虽把 `pass/fail/skip_divergent/skip_insufficient_judges/skip_calib` 写入诊断 JSONL，但后三种无结论状态不会进入公共 skip-rate 门，Q3 空产物与 Q4 超时回退也没有 outcome。测试进程的 pass 因而无法证明目标 branch/artifact/assertion 或有效质量 verdict 实际发生，见 SR-128。Q2 的确定性 train/holdout recall 是独立有效门，不依赖该缺失模型。
- Auto-verify 批次没有持久化或回显“运营请求参数 vs 服务端实际采用参数”；前端 snake_case 控制字段被 serde 忽略后，批次结果只剩计数和 budget，无法从 outcome 证明阈值/抽审比例与 UI 选择一致，见 SR-129。
- Knowledge Chat 的 session/turn/patch 没有一个由服务端签发并在 apply 时校验的 candidate identity（target chunk、source turn、patch hash、actor/scope）。Cockpit 又丢失 attachment 目标并读错 patch 字段，使 UI 展示与最终 apply/verify 对象可分离，见 SR-130。
- Document 详情读取与整替换写入没有共享的 typed envelope/version：后端返回 `{item}`，前端按扁平详情读取并把隐藏原文、hash、索引当作客户端回传状态。当前因 id 丢失而 400；若只修 id，预构造的 null/空数组可能覆盖持久内容，见 SR-131。
- Review queue 没有持久或服务端定义的 projection/category identity；前端从通用 Chunk 列表临时分类，类型闭集含永远不可达的 `needs_review`，coverage dimension 也没有映射到查询字段，且 lifecycle status 未进入派生条件。队列计数、下钻维度和 active 范围因此都不是可复核的数据事实，见 SR-132。

## 尚未宣称完成的反查

阶段 2 证明模型、索引和迁移文件已全文阅读，不代表“集合—全部读写方”已闭环。后续阶段必须继续把路由、Agent、worker、前端调用与测试映射回本台账，最终才可确认租户隔离、索引覆盖、状态机和生命周期完整性。
