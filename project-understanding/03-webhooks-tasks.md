# webhooks.rs 与 tasks.rs 深读记录（核证日期 2026-08-13）

> 读法：两文件均用 Read 分段逐行读完全文（`src/webhooks.rs` 1–3089 行、`src/tasks.rs` 1–1972 行，无跳读），所有断言均在当日用 Read/Grep 亲验；跨模块引用（config 默认值、status 闭集、gateway 入口、索引、quiet_hours 常量、main.rs worker 启动）另行核验并注明出处。

---

## 1. 文件总地图（按行号列出全部函数/常量，每个一句话）

### 1.1 `src/webhooks.rs`（共 3089 行）

**常量 / 类型**

| 行号 | 名称 | 一句话 |
|---|---|---|
| 26 | `DURABLE_INBOUND_REPLY_KIND = "inbound_reply"` | durable 入站应答任务的 kind，webhook→task worker 的单飞交接契约（pub） |
| 27 | `DURABLE_INBOUND_ACTIVE_KEY = "inbound_reply"` | 写进任务行的 `active_task_key` 固定值 |
| 28–31 | `HANDOFF_PENDING/"pending"`、`HANDOFF_MATERIALIZED/"materialized"`、`HANDOFF_DEFERRED/"deferred"`（仅遗留读兼容）、`HANDOFF_IGNORED/"ignored_not_managed"` | 入站消息 `handoff_status` 的四个取值 |
| 33–37 | `struct DurableInboundTask { task_id, run_at_ms }` | materialize 的返回值：任务 id + 生效的 run_at 毫秒 |
| 248–253 | `struct ManualReplyCoverage { task_id, inbound_id, inbound_created_at }` | 人工回复暂停时冻结的入站水位快照 |
| 1019–1023 | `struct PendingState { generation: AtomicU64, deadline_ms: AtomicI64, latest_inbound: Mutex<_> }` | legacy 进程内去抖状态（generation=抢占信号，deadline=去抖窗口） |
| 1025 | `static PENDING: DashMap<String, Arc<PendingState>>` | legacy 去抖调度器全局表（生产 webhook 已不用，见 989–1010 注释） |
| 2834–2843 | `enum WebhookSigError`（7 变体） | 验签失败原因闭集：SecretNotConfigured / MissingSignature / MissingTimestamp / BadTimestamp / TimestampOutOfWindow / BadSignatureFormat / Mismatch |

**函数**

| 行号 | 函数 | 一句话 |
|---|---|---|
| 39–53 | `durable_inbound_task_id` | SHA256(ws\0acct\0wxid\0kind\0) 取前 12 字节构造确定性 ObjectId——每 (租户,账号,联系人) 恒一行 |
| 55–74 | `mark_inbound_handoff` | 仅当 `handoff_status ∈ {pending, deferred}` 时把消息标为给定状态（终态不可再改） |
| 81–94 | `materialize_durable_inbound_task` | 以 `inbound.created_at + debounce_window` 为 run_at 调 `_at` 版本，schedule_reason="debouncing" |
| 99–223 | `materialize_durable_inbound_task_at` | 核心：insert-or-revive 联系人唯一 durable 任务行，(created_at,_id) 水位单调，尾部标 handoff=materialized |
| 225–246 | `policy_run_at` | 静默时段内→`next_wake_at`（带 jitter），否则 now |
| 257–371 | `pause_reply_obligation_for_manual` | 人工发送前：补建缺行→CAS 打 `manual_reply_run_id` 冻结水位→取消旧 AI outbox（失败回滚暂停） |
| 374–411 | `advance_covered_watermark` | 单调推进 `covered_through_*`（(created_at,_id) 字典序，绝不回退） |
| 417–453 | `settle_ai_reply_obligation` | AI 回复全段送达后：推进水位 + 精确 latest-CAS 置 `sent/agent_reply_delivered`（有更新入站或 manual 占有则不完成） |
| 458–565 | `settle_manual_reply_obligation` | 人工回复结算：delivered 且水位仍最新→sent；否则释放暂停回 pending 按当前策略重排 |
| 567–585 | `manual_outbox_settlement` | 段状态集→结算判定：全 sent=Some(true)；任一 sent/pending/in_flight/delivery_unknown=None（保持暂停）；其余=Some(false) |
| 589–654 | `reconcile_manual_reply_obligations` | 崩溃恢复：扫 manual 暂停行，无 outbox 且孤儿超 5min→按失败释放；有则按 settlement 判定 |
| 661–799 | `reconcile_workspace_reply_obligations` | 策略编辑后：workspace 内全部未完成义务先 fence 旧 claim 再按新策略重排；legacy kind 迁移合并 |
| 803–908 | `reconcile_pending_inbound_handoffs` | 每 tick 开头恢复 `handoff_status ∈ {pending,deferred}` 的入站（含重放 contact 建档），batch 100 |
| 911–923 | `webhook_rate_limit_bucket_id` | SHA256(ws\0acct\0+window_start_ms) → `"webhook:<hex>"` 配额桶 _id |
| 929–987 | `shared_webhook_rate_limit` | 跨副本固定窗口配额：pipeline `$add` 原子计数（超限仍递增），超限返回 retry_after 秒 |
| 1012–1014 | `contact_key` | `"{ws}:{acct}:{wxid}"`（legacy 去抖 key） |
| 1027–1029 | `now_ms` | 当前毫秒 |
| 1032–1034 | `next_deadline_ms` | now+window 饱和加（防溢出） |
| 1037–1039 | `barge_in_triggered` | generation 快照与当前不等 → 期间有新入站 → 抢占 |
| 1045–1065 | `register_inbound` | legacy：DashMap entry shard 锁内 spawn-vs-bump 原子决策，prev_gen==0 才 spawn |
| 1070–1216 | `run_debounce_pipeline` | legacy 去抖 runner：睡到 deadline→快照→reload contact→reaction→聚合网关（带抢占 guard）→重算或原子退休；catch_unwind 包裹 |
| 1220–1235 | `reload_managed_contact` | 重查 contact 并过滤 `agent_status==Managed`，None=应退休 |
| 1237–1243 | `managed_contact_reload_filter` | 三元组全租户过滤器 |
| 1245–1653 | `wechat_webhook` | 主 handler：解析→验签→事件分流→限流→principal 分流→去重落库→contact upsert→信号采集→quiet/debounce 物化任务→200 |
| 1668–1703 | `ensure_wake_followup_task` | 测试/工具兼容入口：取最新 inbound 按 wake 时刻物化 durable 任务 |
| 1705–1713 | `stable_payload_hash` | FNV-1a 64bit hex（无 msgId 时的 dedupe 兜底） |
| 1726–1787 | `collect_inbound_behavior_signals` | T1 行为信号旁路采集（reply_latency / reply_length / reactivation），逐条 best-effort |
| 1789–1801 | `is_duplicate_key_error` | Mongo 11000/11001 判定（本模块私有副本） |
| 1805–1813 | `panic_payload_message` | panic payload → 可读字符串 |
| 1815–1833 | `find_string` | 深度递归按候选键找第一个字符串/数字值 |
| 1835–1841 | `value_to_string` | 非空字符串或数字 → String |
| 1848–1855 | `gewe_data_string` | GeWe `Data.<field>.string` 显式提取（优先于 find_string，防顶层同名键遮蔽） |
| 1859–1866 | `gewe_data_msg_type_code` | GeWe `Data.MsgType.low` 数字码 → 字符串 |
| 1878–1885 | `parse_inbound_msg_type` | 专用键（MsgType/msgType/msg_type）解析消息类型，无键默认 "text"（刻意不收裸键 type/Type） |
| 1898–1913 | `classify_inbound_msg_type` | 微信数字码/别名 → 11 类归一，未知恒 "unknown"（绝不当 text） |
| 1921–1948 | `extract_inbound_media_ref` | 非文本消息从候选键提取一个媒体引用；text 恒 None |
| 1950–1993 | `resolve_account_context` | appId→accounts 查 (ws, acct, webhook_secret)；未注册 400；无 appId 按验签开关/账号数决定回退 default 或 400 |
| 1997–2024 | `emit_unknown_app_id_event` | 未知 appId 写 admin 可见事件（kind=webhook_unknown_app_id） |
| 2029–2034 | `is_operatable_person` | 排除 gh_ 公众号 / @chatroom / @openim / 系统保留号 |
| 2039–2041 | `is_self_account` | wxid == 账号自身 wxid 判定（None 无从判定不拦） |
| 2043–2148 | `upsert_webhook_contact` | roster 富化昵称头像 + 跨账号 managed 错配告警事件 + upsert（$setOnInsert agent_status="normal"） |
| 2155–2157 | `rate_limit_event_dedupe_key` | `"rate_limit:{acct}:{day_bucket}"` |
| 2159–2176 | `build_rate_limit_event` | 构造 webhook_rate_limited 事件（status=blocked，带 dedupe_key） |
| 2185–2199 | `maybe_emit_rate_limit_event` | 原子 insert + dup-key 吞（同账号同 UTC 天最多一条） |
| 2203–2211 | `pick_identity_from_friends` | roster friends 内按 wxid 找 (nickname, avatar)（纯函数） |
| 2215–2226 | `roster_identity_for` | 读 roster 快照拿身份，任何失败→None |
| 2849–2884 | `verify_webhook_signature` | HMAC-SHA256(secret, `"<ts>." + raw_body`) + 时间戳 ±skew 校验（纯函数） |

**测试模块**：`roster_identity_tests` 2228–2263、`inbound_msg_type_tests` 2265–2499、`debounce_tests` 2501–2772、`rate_limit_dedupe_tests` 2774–2825、`webhook_sig_tests` 2886–3050、`reply_obligation_tests` 3052–3088。

### 1.2 `src/tasks.rs`（共 1972 行）

| 行号 | 名称 | 一句话 |
|---|---|---|
| 14–41 | `struct TaskClaim` + `owned_running_filter` + `committing_filter` | claim 所有权凭证：token（不可伪造）+ generation（单调写 fence）；两个 CAS filter |
| 46–72 | `prepare_task_commit_if_owned` | running→committing CAS：写 `prepared_commit_kind/prepared_commit` 持久化载荷，$unset claimed_at（非发送类副作用的线性化点） |
| 74–101 | `finalize_task_commit_if_owned` | committing→sent CAS，清 token/prepared |
| 106–134 | `requeue_task_commit_if_owned` | committing→retry（next_retry_at=now）：乐观前置条件失效且未产生目标侧写时回退重算 |
| 136–145 | `task_outbox_marker_prepare_filter` | outbox 授权标记 prepare filter：decision_id + (无 token/null/同 token) |
| 147–151 | `task_outbox_commit_filter` | owned_running + outbox_decision_id |
| 153–160 | `task_claim_send_terminal_filter` | _id + status∈{outbox_enqueued,sent} + token + generation（发送成功审计判据） |
| 164–184 | `struct TaskRunContext` + `write_filter` | 处理器执行上下文：有 claim 用 owned filter，无则裸 _id（测试/人工工具兼容） |
| 186–244 | `claim_task_with_filter` | claim 核心 CAS：默认注入 status∈{pending,retry,failed}；$set running/claimed_at/token，$inc attempt_count+claim_generation，$unset outbox_decision_id/next_retry_at/rerun_requested；ReturnDocument::After |
| 247–257 | `claim_task_by_id` | _id（+可选 workspace）claim——Admin"立即复核"与集成测试共用 |
| 262–277 | `claim_task_by_id_for_account` | _id+ws+acct 绑定 claim（Admin 动作必须绑账号） |
| 279–286 | `task_claim_is_current` | count(owned_running_filter)==1 |
| 289–336 | `bind_task_decision_if_owned` | 写 outbox 前把 decision 绑到仍属本 token 的 task（水位快照线性化点）；decision_reviews 回写 source_task_id/claim_token/覆盖水位 |
| 349–465 | `adopt_recoverable_durable_outbox_if_owned` | SR-177：接管"Outbox 已写、Task 未授权"崩溃窗留下的旧 Outbox，严格 6 条件 CAS 改绑新 decision/run |
| 468–535 | `authorize_task_outbox_if_owned` | 先 update_many 给全部 outbox 行打授权 token（matched 必须==total），再 task CAS→outbox_enqueued；唤醒 dispatcher（立即 + 1050ms 后各一次） |
| 537–547 | `run_task_worker` | 主 worker 循环：tick() + sleep(task_worker_interval_seconds=默认 30s) |
| 552–559 | `run_inbound_reply_worker` | 入站应答专用 worker：tick_inbound_replies() + sleep(250ms)——恢复/积压永不排在画像/campaign 后面 |
| 563–585 | `execute_claimed_task` | spawn 心跳→按 kind 分发（memory_consolidation / outcome_aggregation / initial_profile / 其余→handle_follow_up_task_with_claim）→abort 心跳 |
| 592–611 | `run_due_task_by_id` | 对指定 due 任务走完整 claim+execute+settle 协议（webhook 低延迟唤醒与周期 worker 共用入口） |
| 616–777 | `process_claimed_task` | 执行+结算：成功→按 send 终态 filter 决定审计事件；失败→LLM 不可用保留预算 5min 重试 / 指数退避 retry / 终态 failed |
| 789–977 | `reclaim_stale_running_tasks` | 每 tick 回收超时 running：精确 (token,generation,claimed_at) 快照 CAS；累计回收 ≥3 次→failed |
| 979–1020 | `reconcile_committing_tasks` | 扫 status=committing（limit 20）按 prepared_commit_kind 分发重放（campaign_fanout 交给专用 reconcile） |
| 1022–1052 | `revive_failed_memory_tasks_with_rerun` | memory_consolidation failed 且 rerun_requested→retry（attempt/recovery 归零） |
| 1054–1065 | `dedupe_inbound_candidates` | 每 contact 只保留扫描序中第一个（最老 due）候选 |
| 1067–1115 | `tick_inbound_replies` | 先 reconcile handoff→扫 due inbound_reply（limit 20，sort next_retry_at,run_at,_id）→按 contact 去重→buffer_unordered(并发默认 4) 执行 |
| 1117–1171 | `tick` | 主 tick：6 个 reconcile→reclaim→revive memory→ensure outcome 任务→escalation 超时扫描→send_ledger 回扫→扫非 inbound_reply due 任务（limit 20）逐个执行 |
| 1179–1181 | `retry_delay_seconds` | seeded(attempt, fastrand) |
| 1183–1185 | `provider_unavailable_retry_delay_seconds` | 恒 300s |
| 1187–1205 | `provider_unavailable_settlement_update` | retry + blocked_provider_unavailable + `$inc attempt_count:-1`（不耗预算）+ next_retry=+5min |
| 1207–1214 | `retry_delay_seconds_seeded` | base=min(60·2^(attempt-1), 900)，±20% jitter |
| 1219–1222 | `claim_heartbeat_interval_seconds` | (timeout/2).clamp(5, 60) |
| 1230–1268 | `spawn_claim_heartbeat` | 后台续约 claimed_at；modified==0（已非 running）即自杀；故意不走 supervisor |
| 1277–1325 | `ensure_today_outcome_aggregation_tasks` | 每 account×{7d,30d} 直接 insert，靠 partial unique index 原子去重（dup-key 忽略） |
| 1330–1343 | `is_duplicate_key_error` | 11000/11001（本模块私有副本） |
| 1345–1354 | `today_date_string` | epoch 毫秒截断到 UTC 日 → YYYY-MM-DD |
| 1358–1630 | `handle_outcome_aggregation_task` | 计算 reply_rate/conversation_depth/agent_block_rate/daily_run_* → 有 claim 走 prepare→reconcile 两段提交；无 claim 直接条件 upsert+task sent |
| 1632–1700 | `reconcile_outcome_aggregation_commit` | 从 committing 行的 prepared_commit 重放 metric 投影（dup-key=更高 generation 已写，容忍）→ finalize |
| 1702–1717 | `outcome_metric_write_filter` | `_id` + (无 source_task_id 或 同 task 且 generation ≤ claim.generation)——投影单调写 fence |
| 1719–1971 | `mod tests` | claim 结构 / filter 形状 / 退避 jitter / 心跳 clamp / schema 回归等 15 个单测 |

---

## 2. 逐函数深读

### 2.1 webhooks.rs — durable 交接层

#### `durable_inbound_task_id`（webhooks.rs:39-53）
- **职责**：把 (workspace_id, account_id, wxid, kind) 哈希成确定性 12 字节 ObjectId。每个值后接 `\0` 分隔（webhooks.rs:45-47），防拼接歧义。
- **输出**：确定性 ObjectId → 同一联系人的 durable inbound 任务在 `agent_tasks`（代码中 `state.db.tasks()`）里**恒为同一行 _id**，这是"单飞 + 复活 + fence"的基石。

#### `mark_inbound_handoff`（webhooks.rs:55-74）
- **职责**：CAS 式推进消息的 `handoff_status`。filter 限定 `handoff_status ∈ {pending, deferred}`（webhooks.rs:65），即只允许从非终态出发；写 `handoff_status` + `handoff_updated_at`。
- **注意**：update_one 未匹配也返回 Ok（幂等重放安全）。`deferred` 是遗留兼容读值（webhooks.rs:30），新代码不再写。

#### `materialize_durable_inbound_task` / `materialize_durable_inbound_task_at`（webhooks.rs:81-94 / 99-223）
- **职责**：物化或刷新联系人**唯一**的被动应答义务行。消息序 = `(created_at, _id)`：created_at 是到达事实主键，_id 只是同毫秒 tie-breaker（webhooks.rs:76-80 doc 注释——不同进程同秒内 ObjectId 随机尾不单调）。
- **输入**：contact、已持久化的 inbound（必须有 `_id`，否则 `AppError::External`，webhooks.rs:106-108）、run_at、schedule_reason。
- **主路径**：
  1. 构造 `AgentTask`：kind=`inbound_reply`、status=`pending`、`content=message_id.to_hex()`（**任务只存消息 _id 的 hex，不存内容**）、review_required=true、max_attempts=3、gateway_status=schedule_reason（webhooks.rs:112-134）。
  2. insert_doc 追加 5 个非 DTO 字段：`active_task_key`、`latest_inbound_id/created_at`（上界水位）、`obligation_started_inbound_id/created_at`（首次成功交付前的上下文扩张稳定下界，webhooks.rs:135-141）。
  3. `newer` 谓词（webhooks.rs:143-152）：仅当本条 inbound 的 (created_at,_id) **严格大于**行内已记录水位才允许刷新——字典序展开成 `$lt` + 相等时 `_id $lt` 三分支。
  4. `insert_one` 成功 → 新行；**dup-key**（行已存在）→ 两段有序 fallback（webhooks.rs:156-206）：
     - **manual 分支**（webhooks.rs:159-176）：行上有 `manual_reply_run_id`（人工回复占有中）→ 只推 `latest_inbound_*`/`run_at`/`content` 水位，**不动 status**——"人工回复未结算前新入站不得启动 AI 回复"（webhooks.rs:157-158 注释）。
     - **refresh 分支**（webhooks.rs:177-204）：无 manual 占有 → `$set` 全量复活（status=pending、attempt_count=0、claim_recovery_count=0、gateway_status=schedule_reason、run_at、content 等）+ `$unset` 清 12 个字段（claim_token/claimed_at/outbox_decision_id/prepared_commit*/manual_* 等）。**filter 不含 status 条件**：`sent`/`failed`/`cancelled` 终态行都能被新入站复活——这正是"同一行跨终态复活并 fence 旧 owner"的设计（webhooks.rs:23-25 doc）。
  5. 无条件 `mark_inbound_handoff(materialized)`（webhooks.rs:210）+ 读回行内实际 run_at 返回（webhooks.rs:211-222，并发下可能是更晚到者写的值）。
- **乱序语义**：若本条比水位旧（`newer` 不成立），两个 update 都 matched 0，函数**仍**标 materialized——语义为"该消息的应答义务已由现行任务行承载"（回复时 gateway 按消息表聚合上下文，见 §3.3），不丢消息。
- **错误路径**：非 dup-key 的 insert 错误向上抛（webhooks.rs:207）；行消失抛 External（webhooks.rs:214）。

#### `policy_run_at`（webhooks.rs:225-246）
- quiet_hours_enabled 且 `is_quiet_now(start,end,tz_offset)` → `next_wake_at(end, tz_offset, wxid, wake_jitter_max_seconds)`（按 wxid 加 jitter，默认上限 900s，config.rs:812）；否则 `DateTime::now()`。

#### `pause_reply_obligation_for_manual`（webhooks.rs:257-371）
- **职责**：人工（运营者）要亲自发消息前，冻结当前入站水位并暂停 AI 被动应答义务；fence 掉旧 AI owner 后才取消其 Outbox。
- **流程**：
  1. 取该联系人最新 inbound（(created_at,_id) 降序，webhooks.rs:262-276）；无 inbound / 无 _id → Ok(None)。
  2. 任务行不存在（历史/崩溃缺口）→ 先 `materialize_durable_inbound_task_at(now, "manual_reply_preparing")` 补一行（webhooks.rs:285-302）。
  3. `find_one_and_update`（ReturnDocument::Before）：filter=_id+kind+(`manual_reply_run_id` 不存在或等于本 run_id，幂等重入)；$set status=pending、gateway_status=`manual_reply_pending`、`manual_reply_run_id`、`manual_reply_started_at`、`manual_covers_through_inbound_id/created_at`；$unset claim_token/claimed_at/outbox_decision_id/next_retry_at/error（webhooks.rs:304-336）。
  4. 未匹配（另一个 run_id 占有中）→ `Conflict("another_manual_reply_is_pending")`（webhooks.rs:337-341）。
  5. Before 快照里有 `outbox_decision_id` → `agent::cancel_for_decision(..., "superseded_by_manual_reply")`；**取消失败则回滚**：调 `settle_manual_reply_obligation(delivered=false)` 释放刚打的暂停（释放再失败仅 error log），返回原错误（webhooks.rs:342-365）。
- **顺序不变量**：先 task 转移（fence 授权）后取消 Outbox（webhooks.rs:255-256 doc）——dispatcher 授权总是回联 task，旧 claim 已失效则即便取消竞争失败也发不出去。

#### `advance_covered_watermark`（webhooks.rs:374-411）
- 单调 CAS：仅当行内 `covered_through_*` 缺失/为 null/严格小于给定 (created_at,_id) 时推进；绝不回退更新的交付水位。

#### `settle_ai_reply_obligation`（webhooks.rs:417-453）
- **职责**：AI 回复的**全部文本分段确认送达后**结算义务。
- **两步**：先无条件 `advance_covered_watermark`（冻结水位总是推进）；再精确 CAS 完成：filter=_id+kind+`latest_inbound_id/created_at` **恰等于**本次覆盖的水位+无 `manual_reply_run_id` → status=`sent`、gateway_status=`agent_reply_delivered`、covered_through 同步、$unset claim/outbox/retry/error（webhooks.rs:424-451）。
- **返回** matched==1：false 表示送达期间有更新入站刷新了水位（义务继续、行已被 refresh 分支重排）或 manual 占有——调用方不得视为完成。

#### `settle_manual_reply_obligation`（webhooks.rs:458-565）
- **输入**：五元组 + `delivered: bool`。按 `_id+manual_reply_run_id` 查行、查 contact，任一缺 → Ok(false)。
- **策略重载**：`load_user_operation_domain_config(workspace)`（**workspace 级**，webhooks.rs:487-489）→ runtime → `policy_run_at`。
- **delivered=true**：advance watermark 后做"精确 latest-watermark CAS"（webhooks.rs:499-531 注释：与竞态入站互斥）→ 成功则 sent/`manual_reply_delivered` 并清全部 manual 字段，返回 true。
- **CAS 失败或 delivered=false**（webhooks.rs:535-564）：按 run_id 释放暂停 → status=pending、run_at=policy、gateway_status ∈ {`manual_reply_delivered_newer_inbound_pending`, `manual_reply_failed_rescheduled`}，清 manual 字段。失败交付**从不推进覆盖水位**（webhooks.rs:455-457 doc）。

#### `manual_outbox_settlement`（webhooks.rs:567-585）
- 纯函数三态：空→None；全 `sent`→Some(true)；任一 ∈ {sent,pending,in_flight,delivery_unknown}→None（**部分送达不可逆**，保持暂停等运营者处理，webhooks.rs:574-575 注释）；否则（全部 canceled/failed_terminal 等确认未交付）→Some(false)。

#### `reconcile_manual_reply_obligations`（webhooks.rs:589-654）
- 扫 `kind=inbound_reply` 且 `manual_reply_run_id` 为 string 的行（limit 100）；对每行查 `agent_send_outbox` 中 `run_id+source_kind="manual_send"` 的全部段状态。
- 无任何 outbox 段（崩在 pause 与 Outbox 创建之间）：以 `manual_reply_started_at`（回落 updated_at → now）判孤儿，超 `ORPHAN_GRACE_MS = 5min`（webhooks.rs:591）→ 按 delivered=false 释放；否则等待。
- 有段 → `manual_outbox_settlement` 判定；`delivery_unknown` 刻意保持暂停防重复回复（webhooks.rs:587-588 doc）。返回结算行数。

#### `reconcile_workspace_reply_obligations`（webhooks.rs:661-799）
- **触发**：workspace 策略（quiet hours 等）编辑后。
- 扫 workspace 下 kind ∈ {`inbound_reply`, `deferred_inbound_reply`(legacy，quiet_hours.rs:20)} 且 status ∈ {pending,retry,failed,running,outbox_enqueued} 的行，全部快照后逐行：
  1. 查 contact、按新策略算 run_at；
  2. **先 fence**（webhooks.rs:719 注释"Invalidate ownership first"）：legacy 行 CAS→`cancelled`/`merged_into_reply_obligation` 并清 active_task_key；现行行 CAS→`pending`+新 run_at+`policy_reconciled`+attempt_count=0+清 claim/outbox/error（webhooks.rs:720-755）；
  3. matched 后才 `cancel_for_decision("quiet_hours_policy_changed")`（webhooks.rs:760-768）；
  4. legacy 行再取最新 inbound `materialize_durable_inbound_task_at(run_at, "policy_reconciled")` 迁移进现行单行模型（webhooks.rs:770-796）。
- 返回变更行数。**注意**：此处 runtime 也是 workspace 级 config（webhooks.rs:667-668）。

#### `reconcile_pending_inbound_handoffs`（webhooks.rs:803-908）
- **职责**：恢复"消息已落库但任务物化前崩溃"的窗口（SR-177）；每个 task-worker tick 开头执行，可重复。
- 扫 `direction=inbound` 且 `handoff_status ∈ {pending,deferred}`，(created_at,_id) 升序 limit 100。
- **decode 失败即整体 Err**（webhooks.rs:820-822，见 §5 疑点 D2）。
- contact 缺失时的再物化：`is_operatable_person` → 重跑 `upsert_webhook_contact`（与正常 webhook 路径同构建档，重建为 normal，webhooks.rs:838-856）；非 operatable / 非 managed → `mark_inbound_handoff(ignored)`（webhooks.rs:857-864）。
- managed → 按**当前**quiet 判定物化：quiet → `_at(next_wake_at, "quiet_hours_waiting")`；否则 profile 决定 debounce 窗口 → 常规 materialize（webhooks.rs:866-905）。**config 用 contact 级** `load_user_operation_domain_config_for_contact`（webhooks.rs:866-871）。

#### `shared_webhook_rate_limit`（webhooks.rs:929-987）
- **模型**：跨副本固定窗口。`window_start = now div_euclid window`；桶 _id = `webhook_rate_limit_bucket_id(ws, acct, window_start_ms)`；`expires_at = window_end + window`（TTL 兜底清理）。
- **原子性**：pipeline update `count = $ifNull(count,0)+1` + upsert；**超限也继续递增**（webhooks.rs:925-928 doc——"every accepted request is counted exactly once"）；并发首次 upsert 撞 `_id` dup-key → loser 以无 upsert 的同 update 重试（webhooks.rs:965-979）。
- **判定**：`count > capacity.max(1)` → `Some(ceil((window_end-now)/1000))` 作为 retry_after 秒（webhooks.rs:981-985）；否则 None 放行。

### 2.2 webhooks.rs — legacy 进程内去抖层（生产 webhook 已不用）

> webhooks.rs:989-1010 注释块明确：生产 ingestion 已改为物化 durable 任务 + 任务租约/CAS 协议做跨副本单飞；`PENDING` 是进程本地的，仅保留给历史集成测试与外部工具，**禁止**重新引入生产 webhook 路径。经 Grep 亲验，`register_inbound`/`run_debounce_pipeline` 在 `src/` 下无生产调用方（仅定义、注释与 `#[cfg(test)]`）。

#### `register_inbound`（webhooks.rs:1045-1065）
- DashMap `entry()` shard 锁内原子 spawn-vs-bump：或插入新 `PendingState`（generation 从 0）或复用；统一 `generation.fetch_add(1)`、刷新 deadline、替换 latest_inbound；`prev_gen==0` → 返回 spawned_now=true（调用方据此 spawn runner，恰好一次——debounce_tests:2692-2742 并发 16 线程断言）。

#### `run_debounce_pipeline`（webhooks.rs:1070-1216）
- 七步循环：(a) 睡到 deadline（可被后到入站反复重置，1093-1101）；(b) 快照 generation+latest_inbound（锁绝不跨 .await，1104-1105）；(c) reload contact，非 managed/出错 → 移除 state 退休（出错先写 `agent_error` 事件，1108-1132）；(d) `record_user_reaction` 旁路（失败仅写事件不阻断，1136-1148）；(e) `handle_managed_message_aggregated` + 抢占 guard（`barge_in_triggered` 闭包，1153-1172）；(f) generation 变 → continue 重算（1175-1177）；(g) `remove_if(generation==gen_at_start)` 原子退休，谓词失败回 loop（1181-1188）。
- panic 兜底：catch_unwind → 移除 state + `tracing::error!` + `webhook_handler_panic` 事件（爆炸半径=丢这一串，1192-1215）。

### 2.3 webhooks.rs — 主 handler `wechat_webhook`（webhooks.rs:1245-1653）

**输入**：`State(AppState)` + `HeaderMap` + 原始 `Bytes` body（验签需原始字节）。**输出**：`AppResult<Json<Value>>`。全流程 18 步：

1. **解析 JSON**（1252-1253）：失败 → 400 `invalid json body: ...`。
2. **testMsg 探活**（1260-1266）：`testMsg/TestMsg` 存在 → 立即 200 `{ok:true, ignored:"callback_test", echo}`。刻意放在验签**前**（无副作用，GeWe 控制台"测试回调"按钮要 5s 内 ack，1255-1259 注释）。
3. **解析 appId + 账号上下文**（1273-1287）：appId 候选 6 键（`appId/app_id/appid/Appid/AppId/APPID`）；`resolve_account_context` 返回 (workspace_id, account_id, webhook_secret)。BadRequest → `emit_unknown_app_id_event` + 400（未知 appId 不再静默回退 default，1280-1285 注释 P1）。
4. **验签门（方案 B，fail-closed）**（1291-1313）：`config.webhook_verify_signature`（默认 true，config.rs:798）时，`verify_webhook_signature(secret, x-webhook-timestamp, x-webhook-signature, raw body, now, skew=300s 默认)` 失败 → 脱敏 warn（只记 reason/account/body_len）+ 400 `invalid signature`。签名在"解析 appId、查到账号密钥之后"进行——每账号密钥（1250-1251 注释）。
5. **Online/Offline 事件**（1317-1342）：`TypeName ∈ {offline, online}`（大小写不敏感）→ `accounts.update_one($set online, last_sync_at)`（**fail-soft**：失败仅 warn 不 5xx，防 MCP 重推，1322-1334）→ 200 `{ok, ignored:"online_event"|"offline_event", type}`。有副作用故在验签**后**（1258-1259 注释）。`online` 供 outbox dispatcher 发送前 gate（掉线 defer 不盲发）。
6. **共享限流**（1345-1359）：`shared_webhook_rate_limit(ws, acct, capacity=30, window=60s 默认)` 超限 → `maybe_emit_rate_limit_event`（当日去重）+ `AppError::RateLimited{retry_after, account_id}`。
7. **from_wxid 提取**（1361-1380）：优先 `gewe_data_string(Data.FromUserName.string)`（真实 GeWe 推送；防被顶层 `Wxid`=账号自己遮蔽——inbound_msg_type_tests:2411-2425 回归留证），回落 `find_string` 11 键；缺失 → 400 `webhook missing sender wxid`。
8. **content 提取**（1381-1399）：优先 `Data.Content.string`（干净正文；防命中 `PushContent` 通知串"吴界 : 你好"——tests:2427-2440），回落 8 键，缺省空串。**注意**：`gewe_data_string` 命中空串返回 Some("")，刻意不回落（1846-1847 注释）。
9. **principal（领导）分流**（1400-1425）：`lookup_principal_config(ws, acct, from_wxid)` 命中 → `handle_principal_reply(content)`；consumed=true → 200 `{ok, routed:"principal"}` 短路。**必须在落库/managed 处理之前**——领导可能同时也是某 contact，防止领导消息被当客户入站（1400-1402 注释）。consumed=false 则继续走客户链路。
10. **message_id + dedupe_key**（1426-1458）：message_id 候选 9 键（newMsgId/msgId/…/NewMsgId/MsgId/MessageId）；`_mcp.sourceMsgId` envelope 兜底 → `effective_message_id`。dedupe_key = `"message:{id}"`，全缺时 `"payload:{fnv1a64hex}"`（A-03 已知边界：无 ID 同内容连发第二条被当 duplicate 丢，生产 GeWe AddMsg 恒带 NewMsgId，1451-1454 注释，不修）。
11. **构造 + 原子落库**（1465-1503）：`ConversationMessage{direction:Inbound, msg_type: parse_inbound_msg_type, media_ref: extract_inbound_media_ref, raw: payload 全量, is_synthetic_relay:false}`；插入文档**同写** `handoff_status:"pending"`（SR-177 同一次 insert，1484-1488）。dup-key（partial unique index `workspace_id+account_id+dedupe_key`，亲验 db/indexes.rs:810-822）→ 200 `{ok:true, duplicate:true}`（P0-19：insert+catch 11000 替代 check-then-insert TOCTOU，1460-1464 注释）。其余错误 → 5xx。
12. **contact 查询/建档**（1505-1521）：find_one 不存在 → `upsert_webhook_contact`。
13. **非私聊真人短路**（1523-1532）：contact=None（gh_/@chatroom 等）→ 消息保留在库、mark handoff `ignored_not_managed` → 200 `{ok, skipped:"not_operatable_contact"}`。
14. **contact 统计字段更新**（1534-1560）：先快照 `prev_last_inbound_ms`/`prev_last_outbound_ms`（供行为信号），再 $set `last_inbound_at/last_message_at/updated_at`。**A-06 best-effort**：失败仅 warn 不影响应答（1540-1543 注释）。
15. **T1 行为信号采集**（1562-1575）：`collect_inbound_behavior_signals`（见下），全程旁路。
16. **managed 分流**（1585-1645）：`managed = contact.agent_status == Managed`。
    - **managed + quiet**（1592-1623）：加载 contact 级 domain config → runtime；`quiet_hours_enabled && is_quiet_now` → `materialize_durable_inbound_task_at(next_wake_at(end,tz,wxid,jitter), "quiet_hours_waiting")`，`deferred=true`。**不 spawn 唤醒**——醒来由 250ms 的 inbound_reply worker 扫描捕获；醒后 gateway `load_recent_messages` 天然聚合整段静默期消息一次性回（1588-1591 注释）。
    - **managed + 非 quiet**（1624-1642）：`resolve_debounce_window_ms(active_profile, 默认 2000ms)` → `materialize_durable_inbound_task`（run_at=created_at+窗口）→ `tokio::spawn` 低延迟唤醒：sleep 到 run_at → `run_due_task_by_id(task_id)`；失败仅 error log，周期 worker 兜底（1633-1641）。
    - **非 managed**（1643-1645）：mark handoff `ignored_not_managed`（只持久化不应答）。
17. **P2 背景**（1577-1584 注释）：MCP 侧 fetch 是 5s AbortController 且失败不重试；决策+审查流水线 10-15s，必须落库后立即 ack、任务后台执行。
18. **返回**（1647-1652）：200 `{ok:true, managed, queued: managed && !deferred, deferred}`。

#### `collect_inbound_behavior_signals`（webhooks.rs:1726-1787）
- dedupe 后缀 = message_id，缺失退化用 `observed_at` 毫秒（幂等精度略降，1721-1723 注释）。
- 恒产出 `reply_latency`（基于 prev_last_outbound_ms）+ `reply_length`；`is_reactivation(prev_last_inbound_ms, now, REACTIVATION_THRESHOLD_MS)` 成立再加 `reactivation`。
- 逐条 `persist_signal` + `record_signal_metric`，任何失败仅 warn（1774-1786）。幂等由 `behavior_signals` 的 partial unique index (ws,acct,dedupe_key) 保证（亲验 db/indexes.rs:527-540）。

#### 解析辅助函数细节
- `find_string`（1815-1833）：对象先按候选键序查本层，再**深度递归**所有子值；数组逐项。首个非空字符串/数字命中即返回。
- `gewe_data_string`（1848-1855）：只走 `Data.<field>.string` 精确路径，取不到 None 交回落；空串命中返回 Some("")。
- `parse_inbound_msg_type`（1878-1885）：`Data.MsgType.low`（数字）优先，回落 find_string 仅 3 个专用键 `msgType/msg_type/MsgType`；**刻意排除裸键 `type`/`Type`**（envelope 里 `{"type":"event"}` 极常见，会把纯文本误标非文本，1868-1877 注释 F1 评审 I1）；无任何类型键默认 `"text"`。
- `classify_inbound_msg_type`（1898-1913）：微信数字码映射——1=text 3=image 34=voice 43=video 42=namecard 47=emoji 48=location 49=appmsg 50=voip 51=statussync 10000/10002=system；字符串别名亦收；未知恒 `"unknown"`（绝不当 text，下游走非文本分支，F2 才做媒体理解）。
- `extract_inbound_media_ref`（1921-1948）：text 恒 None；非文本从 16 个候选键（mediaUrl/cdnUrl/MediaId/FileId 等）尽力取一个引用，取不到 None（不造假）。

#### `resolve_account_context`（webhooks.rs:1950-1993）
- 有 appId：查 `wechat_accounts.app_id` → 命中返回 (ws, acct, webhook_secret)；未命中 → 400（P1：不再静默回退 default 导致 managed contact 永远 lookup 不到，1968-1973）。
- 无 appId：**A-05 防线**（1975-1987）——开验签时无需 count（secret=None 必然 SecretNotConfigured→400，default 回退到不了副作用点）；未开验签时 count>1 → 400"缺 appId 且存在多个账号"；单账号（≤1）回落 (default_workspace_id, default_account_id, None)。

#### `upsert_webhook_contact`（webhooks.rs:2043-2148）
- 非 operatable → Ok(None)（消息已在调用点落库，只是不建 contact，2050-2053）。
- 身份富化只从 **roster 快照**取（payload 的 find_string 会递归命中 `_mcp.nickName`=账号自己昵称，2054-2056 注释）；roster 未命中不写 nickname/avatar（防 $set None 覆盖已有值，2105-2112）。
- **跨账号 managed 错配检测**（2060-2104）：同 (workspace, wxid) 已在**另一** account 下 managed → 写 `webhook_managed_contact_account_mismatch` warning 事件（本次仍创建 normal 影子记录，AI 不自动回复）。
- upsert：`$setOnInsert agent_status:"normal"`（新联系人默认不接管）+ created_at；随后 find_one 读回。

#### `verify_webhook_signature`（webhooks.rs:2849-2884）
- 签名内容 = `"<timestamp_header.trim()>." + raw_body`，HMAC-SHA256(每账号明文 secret)，hex 于 `x-webhook-signature: sha256=<hex>`（前缀可省，大小写 hex 均可——tests:2912-2927）。
- 校验序：secret 空→SecretNotConfigured；签名/时间戳缺失→对应错误；ts 非整数→BadTimestamp；`|now-ts| > skew*1000`→TimestampOutOfWindow（**恰好等于边界通过**，tests:2980-2987）；hex 解码失败→BadSignatureFormat；HMAC 不符→Mismatch。
- **A-04 已知边界**（2845-2848 注释，不修）：无 nonce，skew（默认 300s）内可原样重放；但重放无重复副作用——AddMsg 撞 dedupe、Online/Offline 幂等 $set、领导回复 resolve 幂等。

### 2.4 tasks.rs — claim/lease 协议

#### `TaskClaim`（tasks.rs:14-41）
- `claim_token`：每次 claim 重新生成的 UUID，**不可伪造所有权**；`claim_generation`：每次 claim `$inc`，把晚到业务投影做成**单调写**（旧执行者不能覆盖新执行者结果，tasks.rs:18-20 注释）。正确性只依赖 `_id + status + token (+generation)` CAS（tasks.rs:12-13）。
- `owned_running_filter`：`{_id, status:"running", claim_token, claim_generation}`；`committing_filter` 同构但 status="committing"。

#### `claim_task_with_filter`（tasks.rs:186-244）——所有 claim 的底层
- 未显式给顶层 `status` 键则注入 `status ∈ {pending, retry, failed}`（tasks.rs:191-193；`failed` 在集内 → Admin 按 _id 直接 claim 可重跑 failed 任务）。
- `find_one_and_update`（ReturnDocument::**After**）：`$set {status:"running", updated_at, claimed_at, claim_token}`、`$inc {attempt_count:1, claim_generation:1}`、`$unset {outbox_decision_id, next_retry_at, rerun_requested}`（tasks.rs:194-222）。
- 返回 (decode 后的 AgentTask, TaskClaim)；`claim_generation` 容忍 i32/i64 两种 BSON 编码，缺省 1（tasks.rs:229-232）。**因 ReturnDocument::After，返回的 `task.attempt_count` 已含本次 +1**（影响 process_claimed_task 的判定语义，见下）。

#### `claim_task_by_id` / `claim_task_by_id_for_account`（tasks.rs:247-257 / 262-277）
- 前者 _id（+可选 ws）；后者强制 ws+acct 绑定（Admin 动作必须绑定渲染时的账号；后台 worker 用前者，因为它们从已按账号 scope 的调度查询里拿 id，tasks.rs:259-261 注释）。

#### 两段提交原语（非发送类副作用）
- `prepare_task_commit_if_owned`（46-72）：running→`committing` CAS + 持久化 `prepared_commit_kind/prepared_commit` 载荷 + $unset claimed_at。**线性化点**：取消与该 CAS 竞争同一行；committing 赢了之后载荷持久、崩溃后可幂等重放，取消不再接受该状态（43-45 注释）。
- `finalize_task_commit_if_owned`（74-101）：committing→`sent` + 清 token/prepared。
- `requeue_task_commit_if_owned`（106-134）：committing→`retry`（next_retry_at=now）——乐观前置被取代且**目标侧零写入**时回炉重算，仍需 committing token。

#### 发送授权链（Outbox 协同）
- `bind_task_decision_if_owned`（289-336）：running CAS 写 `outbox_decision_id`（**水位快照线性化点**：更新入站先刷新任务 → 本 claim 不再匹配；晚到 → 授权前 fence 本 decision，294-296 注释）；成功后回写 decision_reviews：`source_task_id`+`source_task_claim_token`，inbound_reply 任务追加 `reply_coverage_kind:"passive_reply"` + `covers_through_inbound_id/created_at`（该 decision 覆盖到的入站水位）。
- `adopt_recoverable_durable_outbox_if_owned`（349-465）：SR-177 崩溃窗（Outbox 已写、Task 未授权）恢复。前置：新旧 decision 不同且 run_id 非空；当前 claim 仍拥有该 inbound_reply 任务且已绑新 decision（361-377）；旧 decision 确系**同一 task 的另一（已失效）claim** 产生（`source_task_claim_token` 存在且 ≠ 当前 token，379-393）。Outbox CAS 六条件（400-424）：未请求取消、未 reclaimed_in_flight、reclaim_count ∈ {0,null}、attempt ∈ {0,null}、无 send_started_at、无授权 token，且 status=pending 或 (canceled 且 cancel_reason 以 `stale_task_claim:` 开头)——**从未跨过远端发送边界**才可接管。成功 → Outbox 复位干净 pending 并改绑新 decision/run；旧 review 标 `superseded_by_task_recovery`（452-463）。接管**不构成**发送授权，调用方仍须走 `authorize_task_outbox_if_owned`（347-348 注释）。
- `authorize_task_outbox_if_owned`（468-535）：
  1. 数 outbox 行；0 → 拒绝授权（warn，484-487）。
  2. `update_many` 给该 decision 全部 outbox 行打 `task_send_authorization_token=claim_token`（filter 容忍无 token/null/同 token）；`matched != total` → 另一 claim 冲突 → 拒绝（498-507）。**顺序论证**（475-479 注释）：标记本身不是授权（dispatcher 还要求本 task ∈ outbox_enqueued|sent），此处崩溃只 defer、后续 reclaim 使旧标记 stale；若倒序先 task CAS，投影失败会留下"已提交却无标记"的永久搁浅。
  3. task CAS（owned_running + outbox_decision_id）→ `outbox_enqueued`（509-527）——**授权线性化点**。
  4. `notify_outbox_work()` 立即唤醒 + `notify_outbox_work_after(1050ms)` 二次唤醒（dispatcher 若在 Task 尚 Building 时抢跑，该 outbox 行有 1s durable 延迟；不清 next_retry_at 以免干扰真实退避/节奏，528-533 注释）。

#### `task_claim_send_terminal_filter`（153-160）
- `_id + status ∈ {outbox_enqueued, sent} + token + generation`：本 claim 确实走到发送授权终态的判据（process_claimed_task 用它决定是否写成功审计）。

### 2.5 tasks.rs — worker 循环

#### `run_task_worker`（537-547）/ `run_inbound_reply_worker`（552-559）
- 均为无限循环 + 错误仅 log；间隔分别 `task_worker_interval_seconds`（默认 30s，config.rs:493）与固定 250ms。
- 两者在 main.rs:217-223 由 `spawn_supervised` 包裹启动（panic 退避重启 + 写 background_worker_panic 事件，main.rs:213-214 注释）。
- inbound 专线存在意义（549-551 注释）：webhook 已做即时唤醒，此 worker 保证**重启/积压恢复**永不排在 profiling/consolidation/campaign/主动跟进后面。

#### `tick`（1117-1171）——主 worker 每 30s 一轮的完整序列
1. `reconcile_committing_tasks`（两段提交重放）；
2. `webhooks::reconcile_pending_inbound_handoffs`（SR-177 崩溃窗）；
3. `escalation::reconcile_pending_relay_intents`（SR-054：请示裁决的 durable relay intent 崩溃恢复）；
4. `escalation::reconcile_principal_card_deliveries`；
5. `system_incident::reconcile_notifications`；
6. `campaigns::reconcile_campaign_dispatches`（HC-021：受众快照→确定性任务物化之间的崩溃续跑）；
7. `reclaim_stale_running_tasks`（HP-1：先回收再认领）；
8. `revive_failed_memory_tasks_with_rerun`；
9. `ensure_today_outcome_aggregation_tasks`（S-19）；
10. `escalation::scan_escalation_timeouts`（超时改派下一位真人）；
11. `send_ledger::scan_send_ledger_outcomes`（主动发送台账回扫转化）；
12. 扫 `status ∈ {pending,retry}` 且 **kind ≠ inbound_reply** 且 due（run_at 或 next_retry_at ≤ now）的任务，limit 20、sort (next_retry_at, run_at)，逐个 `run_due_task_by_id`（1141-1169）。
- 步骤 1-6 的错误用 `?` 传播 → **任一 reconcile Err 会中止本轮 tick**（错误被 run_task_worker log 后 30s 重来）；步骤 9-11 用 `let _ = ....await;` 吞错。claim 内再次校验 status+due（1164-1167 注释：cursor 快照后入站可能已刷新 run_at，靠原子 claim 防提前执行）。

#### `tick_inbound_replies`（1067-1115）
1. 先 `reconcile_pending_inbound_handoffs`（`?` 传播）；
2. 扫 kind=inbound_reply、status ∈ {pending,retry}、due，limit 20、sort (next_retry_at, run_at, _id)；
3. `dedupe_inbound_candidates`：每 contact 只留扫描序第一个（最老 due）——单进程不为同一会话浪费两个前台槽位；跨进程正确性仍归 claim CAS（1089-1092 注释）；
4. `buffer_unordered(inbound_reply_worker_concurrency，默认 4，clamp [1,32]，config.rs:494-496)` 并发执行 `run_due_task_by_id`，逐个 log 错误。

#### `run_due_task_by_id`（592-611）
- claim filter：`_id` + **`manual_reply_run_id` 不存在**（人工占有中的义务不可执行）+ `$or [{status:pending, run_at≤now}, {status:retry, next_retry_at≤now}]`。顶层无 status 键 → claim_task_with_filter 注入 `status ∈ {p,r,f}`，与 $or 交并后等价于 due 的 pending/retry（failed 不满足 $or，不会被周期执行）。claim 不到 → Ok(false)。
- claim 到 → `process_claimed_task`。webhook 低延迟唤醒与周期 worker 共用此入口，崩溃只是把任务留给周期扫描（587-591 注释）。

#### `execute_claimed_task`（563-585）
- `spawn_claim_heartbeat(state, claim, task_claim_timeout_seconds)` 起续约；
- kind 分发：`memory_consolidation` → `agent::handle_memory_consolidation_task_with_claim`；`outcome_aggregation` → 本文件 handler；`initial_profile` → `routes::contacts::handle_initial_profile_task_with_claim`；**其余全部** → `agent::handle_follow_up_task_with_claim`（gateway.rs:178-218，其内部再分流 `principal_decision_relay` 与 `inbound_reply`，见 §3.3）；
- 完成后 `heartbeat.abort()`。

#### `process_claimedtask`（616-777）——统一结算（低延迟唤醒与周期 worker 不许分叉，613-615 注释）
- 快照 task 字段；`max_attempts <= 0` 修正为 3（626-630）。
- **Ok(())**：处理器可能在丢失所有权/选择非发送终态后仍返回 Ok；只有 `task_claim_send_terminal_filter` 恰好命中（本 claim 达到 outbox_enqueued|sent）才写 `follow_up_processed`/success 事件（634-656）。
- **Err(e)** 三分支：
  1. `is_llm_account_unavailable(e)`（659-685）：owned CAS → `provider_unavailable_settlement_update(now)`——status=retry、gateway_status=`blocked_provider_unavailable`、`$inc attempt_count:-1`（**退还本次消耗，无限保留**）、next_retry=+300s、清 claim；写 `follow_up_blocked_provider_unavailable` 事件。
  2. `attempt_count < max_attempts`（686-729）：owned CAS → retry、`retry_scheduled`、error 文本、next_retry=now+`retry_delay_seconds(attempt_count)`、清 claimed_at/claim_token/outbox_decision_id；写 `follow_up_retry_scheduled` 事件（文案"第 {n}/{max} 次重试"里 n 是**已执行次数**——claim 已 +1）。max_attempts=3 ⇒ 总共最多执行 3 次。
  3. 预算耗尽（730-773）：owned CAS → `failed`；$unset 基础三项，且 **kind ∉ {memory_consolidation, inbound_reply} 才清 `active_task_key`/`rerun_requested`**——durable 任务刻意保留稳定 key，下一条入站可复活同一行并 fence 本代（736-739 注释）；写 `follow_up_failed` 事件。
- 所有结算 CAS 都用 `owned_running_filter`：若执行期间被 reclaim/新入站刷新，matched=0，静默放弃（新 owner 已接管）。

#### `reclaim_stale_running_tasks`（789-977）
- 判 stale：`status=running` 且（`claimed_at < now - task_claim_timeout_seconds`（默认 300s，config.rs:529）或 无 claimed_at 且 `updated_at < APP_STARTED_AT`）——缺 claimed_at 的行只回收**本进程启动之前**留下的，进程存活期内的跳过一轮防误伤（783-786 注释；APP_STARTED_AT 于 main.rs:57 set，OnceCell 未填时退化为 stale_before，tasks.rs:793-795）。
- **读原始 BSON**（810-812 注释）：claim_token 不在 AgentTask DTO；两次读取之间旧 lease 可能已被新 owner 接管，二次读 token 会拿新 token 误回收。
- 精确 CAS filter（828-856）：_id + running + 扫描快照的 claim_token（无 token 行匹配 `$exists:false` 兼容部署前遗留）+ 扫描快照的 claimed_at（owner 在 find→update 窗口 heartbeat 过则 CAS 必败，841-843 注释）+ claim_generation（**有 token 无 generation 的畸形行拒绝回收**——宁可不回收也不误伤后来 owner，849-855）。
- `recovery_count = claim_recovery_count + 1`；**≥3 → 直接 failed**：gateway_status=`claim_recovery_exhausted`、error 固定文案、`$inc claim_recovery_count`、终态 unset（同样保留 durable/memory 的 active_task_key）；写 `claim_recovery_exhausted` + `follow_up_failed` 两条事件（865-926）。
- 否则 → retry：gateway_status=`claim_timeout_recovered`、next_retry_at=now、`$inc claim_recovery_count`、清 claim 三项；写 `task_claim_recovered` 事件（928-971）。

#### `spawn_claim_heartbeat`（1230-1268）+ `claim_heartbeat_interval_seconds`（1219-1222）
- 间隔 = (timeout/2).clamp(5,60)：下界防抖动，上界保证 timeout=120s 仍有两次心跳机会（1216-1218 注释）；默认 300s timeout → 60s 心跳。
- interval 首拍先消费（1238-1239），loop 内每拍对 `owned_running_filter` `$set claimed_at=now`；`modified_count==0`（已 committing/终态/被 reclaim）→ 心跳自杀；DB 错误仅 warn 下拍再试。
- 刻意不走 supervisor：心跳 panic 应让其消失、由 reclaim 兜底，而 supervisor 语义是无限重启（1227-1229 注释）。

#### `reconcile_committing_tasks`（979-1020）
- 扫 status=committing limit 20；按 `prepared_commit_kind` 分发：`initial_profile_enrollment`/`initial_profile` → routes::contacts 对应 reconcile；`outcome_aggregation` → 本文件；`memory_consolidation` → agent；`campaign_fanout` → **continue**（campaign 专用 reconcile 紧随本步、拥有 CampaignSend→task 释放的顺序，1007-1008 注释）；未知 kind → error log 跳过。单条失败仅 warn 继续。

#### `revive_failed_memory_tasks_with_rerun`（1022-1052）
- `kind=memory_consolidation ∧ status=failed ∧ active_task_key="memory_consolidation" ∧ rerun_requested=true` → update_many：retry、`memory_candidates_arrived`、next_retry=now、attempt_count=0、claim_recovery_count=0、$unset error/rerun_requested。候选可能在任务 running→failed 过程中到达，调度器把 rerun_requested 留在行上，此处先复活再扫描（1132-1134 注释）。

### 2.6 tasks.rs — outcome 聚合

#### `ensure_today_outcome_aggregation_tasks`（1277-1325）
- 遍历全部 accounts × {7d,30d}：直接 insert AgentTask{kind:"outcome_aggregation", contact_wxid:"_outcome_aggregation", content=`{"horizon":"7d","date":"YYYY-MM-DD"}`, review_required:false}；P1-1：dup-key（`uniq_outcome_aggregation_ws_kind_account_content` partial unique index）= 当日已有，幂等忽略；非 dup 错误向上抛。

#### `handle_outcome_aggregation_task`（1358-1630）
- 解析 content 拿 horizon（默认 7d）/date（默认 "unknown"）；窗口 = now - horizon_days。
- **reply_rate**（1384-1447）：严格按"每条 outbound 后 horizon 窗内该联系人是否有 inbound"——**逐条 outbound 一次 count 查询**（O(outbound) 次查询，性能敏感见 §5）；outbound=0 → None（波 A2：不写 0 误导前端）。
- **conversation_depth**（1449-1482）：窗口内 inbound 总数 / managed contact 数；无 managed → None。
- **agent_block_rate**（1484-1517）：blocked reviews / total reviews；total=0 → None。
- **ai_hold_cleared_rate**（1519-1521）：暂无事件源，恒 None（不以 0 冒充零成功率）。
- **daily_run_count / daily_run_token_total**（1523-1553）：固定 24h 的 agent_run_logs 计数与 token 求和（游标逐行累加）。
- **metric._id** = `"{ws}:{acct}:{horizon}:{date}"`（1555-1559）；记录 `source_task_id` + `source_task_claim_generation`（无 claim 时 0）。
- **有 claim**（1575-1587）：`prepare_task_commit_if_owned("outcome_aggregation", {metric})` 失败（所有权已失）→ Ok 放弃；成功 → 直接调 `reconcile_outcome_aggregation_commit` 完成投影+finalize（崩溃则周期 reconcile 重放）。
- **无 claim**（legacy/测试路径，1589-1629）：条件 upsert `outcome_metrics`（filter=`outcome_metric_write_filter`）；claim 存在时 dup-key = 更高 generation 已写同 _id，是 fencing 正常结果，不得回写 retry/failed（1602-1605 注释）；随后 task → sent/`aggregated`。
- `outcome_metric_write_filter`（1702-1717）：`_id` + `$or [无 source_task_id（首次/legacy 行可覆盖）, {source_task_id==claim.task_id ∧ source_task_claim_generation ≤ claim.generation}]`——**旧 generation 不能覆盖新 generation 的投影**。
- `reconcile_outcome_aggregation_commit`（1632-1700）：从 committing 行读 token/generation/prepared metric（缺任一 → External）→ 条件 upsert（dup-key 容忍）→ `finalize_task_commit_if_owned("aggregated")`。

---

## 3. 跨机制数据流

### 3.1 一条 webhook 从到达至任务执行的完整时序（managed、非静默、text）

```
T0   POST /webhooks/wechat（GeWe → gewe-agent MCP 转发，5s timeout 不重试）
     ├─ 解析 body（1252）→ testMsg? → 否
     ├─ appId → resolve_account_context（1277）→ (ws, acct, secret)
     ├─ 验签门（1291-1313，默认开）→ HMAC(ts.body) + ±300s
     ├─ Online/Offline? → 否
     ├─ shared_webhook_rate_limit（1345，30/60s/账号，Mongo 桶原子计数）
     ├─ from_wxid / content（Data.*.string 优先，1361-1399）
     ├─ principal 分流（1403-1425）→ 非领导，继续
     ├─ dedupe_key = message:{NewMsgId|_mcp.sourceMsgId}（1442-1458）
     ├─ conversation_messages.insert_one（含 handoff_status="pending"，1487-1503）
     │   └─ 撞 partial unique (ws,acct,dedupe_key) → 200 {duplicate:true} 短路
     ├─ contact 查/建（upsert normal，roster 富化，1505-1521）
     ├─ contact.last_inbound_at 等 $set（best-effort，1543-1560）
     ├─ T1 行为信号 ×2~3 条（best-effort，1564-1575）
     ├─ managed → 非 quiet → materialize_durable_inbound_task（1629-1630）
     │   ├─ task _id = SHA256(ws,acct,wxid,"inbound_reply")[..12]（恒同一行）
     │   ├─ run_at = inbound.created_at + debounce_window（profile 或默认 2000ms）
     │   ├─ insert 或 dup-key→refresh/manual 分支（(created_at,_id) 水位单调）
     │   └─ 消息 handoff_status → "materialized"（210）
     ├─ tokio::spawn 低延迟唤醒（1634-1641）：sleep 至 run_at → run_due_task_by_id
     └─ 200 {ok:true, managed:true, queued:true, deferred:false}   ← T0+~几十 ms

T0+2s（去抖窗口到）三条竞争路径之一先到（正确性都归 claim CAS）：
     ① webhook spawn 的唤醒（本进程、恰在 run_at）
     ② inbound_reply worker（250ms 轮询扫 due inbound_reply）
     ③ 主 task worker tick（30s 轮询，但 kind≠inbound_reply 被排除 → 实际不参与）
     └─ run_due_task_by_id（tasks.rs:592）
         ├─ claim CAS：pending&run_at≤now、无 manual_reply_run_id
         │   → status=running、claim_token=UUID、claim_generation+1、attempt_count+1
         ├─ spawn_claim_heartbeat（默认 300s lease / 60s 续约）
         ├─ execute_claimed_task → kind=inbound_reply
         │   → agent::handle_follow_up_task_with_claim（gateway.rs:178）
         │   → handle_durable_inbound_reply_task（gateway.rs:190-191, 226）
         │       ├─ task.content 解析回消息 _id → 读回 inbound + contact（gateway.rs:237-267）
         │       ├─ 100ms claim monitor（丢所有权即置 claim_lost，gateway.rs:269-293）
         │       ├─ （并行）reaction 分析（gateway.rs:299-303）
         │       └─ 决策→审查→（bind_task_decision_if_owned 绑 decision + 覆盖水位）
         │           →（可能 adopt_recoverable_durable_outbox）→ 写 outbox
         │           → authorize_task_outbox_if_owned：outbox 打 token → task=outbox_enqueued
         │           → notify_outbox_work（dispatcher 发 MCP message_send_text）
         └─ process_claimed_task 结算：
             Ok + send 终态命中 → follow_up_processed 事件
             Err → provider_unavailable(5min 保预算) / retry(指数退避) / failed(3 次耗尽)

发送全部段确认送达后（outbox/dispatcher 侧回调）：
     settle_ai_reply_obligation（webhooks.rs:417）
     ├─ advance_covered_watermark（单调）
     └─ latest 精确 CAS → status=sent / agent_reply_delivered
         （送达期间来了新消息 → CAS 失败 → 行已被新 inbound 刷回 pending，义务延续）
```

**静默时段变体**：步骤"materialize"改为 `_at(next_wake_at+jitter, "quiet_hours_waiting")`（1608-1623），不 spawn 唤醒，`deferred=true`、`queued=false`；醒来由 250ms worker 扫到 due 执行，gateway 聚合整段静默期消息一次性回。
**去抖聚合**：窗口内第二条消息走完全相同路径，dup-key → refresh 分支把同一行 run_at 推到"新消息 created_at+窗口"、latest_inbound_* 推进、旧 claim/decision 被 $unset fence；先到的唤醒 claim 不到（run_at 未 due 或行已易主）自然让位。

### 3.2 崩溃恢复语义（崩溃窗口 → 恢复者）

| # | 崩溃窗口 | 持久痕迹 | 恢复者（每 tick） | 恢复动作 |
|---|---|---|---|---|
| 1 | 消息 insert 后、materialize 前 | 消息行 `handoff_status="pending"`（同一次 insert 写入，webhooks.rs:1484-1488） | `reconcile_pending_inbound_handoffs`（tick:1121 与 tick_inbound_replies:1069 双入口） | 重放 contact 建档 + 按当前 quiet/debounce 重新 materialize；非 managed → ignored（webhooks.rs:803-908） |
| 2 | 任务 running 中进程死（心跳停） | status=running、claimed_at 陈旧 | `reclaim_stale_running_tasks`（tick:1131） | 精确 (token,generation,claimed_at) CAS → retry/`claim_timeout_recovered`；累计 3 次 → failed/`claim_recovery_exhausted`（tasks.rs:789-977） |
| 3 | prepare_task_commit 后、finalize 前（非发送副作用） | status=committing + prepared_commit 载荷 | `reconcile_committing_tasks`（tick:1118） | 按 prepared_commit_kind 幂等重放投影 → finalize→sent（tasks.rs:979-1020） |
| 4 | Outbox 已写、Task 未 CAS 到 outbox_enqueued | outbox 行无授权 token；task 仍 running/被 reclaim | 下一代 claim 主动 `adopt_recoverable_durable_outbox_if_owned` | 六条件确认从未跨发送边界 → 改绑新 decision/run 复用旧 outbox；旧 review 标 superseded（tasks.rs:349-465）；dispatcher 侧因 task 不在 outbox_enqueued|sent 拒发 → 只 defer 不重发 |
| 5 | 人工回复 pause 后、manual outbox 创建前 | task 行有 manual_reply_run_id、无对应 outbox | `reconcile_manual_reply_obligations`（由调用方周期执行；扫描本体 webhooks.rs:589-654） | 孤儿超 5min → 按未交付释放回 pending；`delivery_unknown` 刻意保持暂停 |
| 6 | 策略编辑与在途任务竞争 | 旧 run_at/claim 基于旧策略 | `reconcile_workspace_reply_obligations`（策略写路径调用） | 先 fence 旧 claim 再按新策略重排；legacy kind 迁移（webhooks.rs:661-799） |
| 7 | webhook 低延迟唤醒 spawn 丢失（进程死/panic） | durable 任务行 pending & due | inbound_reply worker（250ms）/ 主 tick | 与唤醒共用 `run_due_task_by_id`，语义零分叉（tasks.rs:587-591 注释） |
| 8 | LLM 提供商不可用 | — | process_claimed_task 分支 1 | retry 且 `$inc attempt_count:-1` 不耗预算，5min 后重试，无限保留（tasks.rs:659-685） |
| 9 | memory 任务 failed 后新候选到达 | failed 行留 rerun_requested | `revive_failed_memory_tasks_with_rerun`（tick:1134） | 复活为 retry、预算归零（tasks.rs:1022-1052） |

**总原则**：每个跨系统副作用要么"单文档原子写 +唯一索引去重"，要么"先持久化意图（handoff_status / prepared_commit / manual_reply_run_id / outbox 行）再执行，恢复者幂等重放"；所有权转移一律 (token, generation) CAS，旧 owner 的任何晚到写都 matched 0。

### 3.3 与 gateway 的接口契约（亲验 gateway.rs:150-320）

- **入口**：`execute_claimed_task` 对非专属 kind 统一调 `agent::handle_follow_up_task_with_claim(state, task, Some(claim))`（tasks.rs:581）。gateway 内再分流：`principal_decision_relay` → escalation 专用 relay（gateway.rs:184-189）；`inbound_reply` → `handle_durable_inbound_reply_task`（gateway.rs:190-192）；其余 kind → 查 contact → `run_user_operation_gateway(contact, AgentTrigger::FollowUp(&task), Some(TaskRunContext), None)`（gateway.rs:193-218）。
- **durable inbound 专径**（gateway.rs:226-293）：**必须有 claim**（无 → External 错，gateway.rs:234-236）；`task.content` 必须是可解析的消息 ObjectId hex（gateway.rs:237-239）；消息/contact 查不到 → NotFound（→ process_claimed_task 走 retry→failed）。gateway 起 100ms 间隔的 claim monitor（`task_claim_is_current`）把丢所有权变成协作式中止信号（gateway.rs:269-293），配合 webhook 侧"新入站 refresh 任务行清 token"实现**双向 fence**：新代 fence 旧代（refresh $unset claim_token），旧代自觉退出（monitor + Outbox 授权失败）。
- **tasks 提供给 gateway 的原语**（均以 claim 为参数）：`bind_task_decision_if_owned`（写 outbox 前绑 decision+覆盖水位）→ `adopt_recoverable_durable_outbox_if_owned`（可选恢复）→ `authorize_task_outbox_if_owned`（授权终态 CAS + 唤醒 dispatcher）；非发送副作用用 `prepare/finalize/requeue_task_commit_if_owned` 三件套。
- **webhooks 提供给 gateway/outbox 的结算原语**：AI 送达 → `settle_ai_reply_obligation`；人工送达/失败 → `settle_manual_reply_obligation`；人工起手 → `pause_reply_obligation_for_manual`。
- **legacy 聚合入口**：`handle_managed_message_aggregated(contact, inbound, should_abort_send)`（gateway.rs:158-172）仅剩 legacy 去抖 runner 使用（run_debounce_pipeline (e) 步）；`should_abort_send` 是纯查询闭包（读 generation），gateway 在落盘/入队前调用它放弃过时生成。

---

## 4. 事实卡速查

### 4.1 AgentTask.status 闭集与转移（亲验 models.rs:918-928）

闭集（`ALLOWED_AGENT_TASK_STATUS`，所有写点先过 `assert_agent_task_status_valid`，debug panic / release error log）：
`pending / running / committing / retry / failed / cancelled / sent / completed / outbox_enqueued`

本两文件内出现的转移（→ 后标 gateway_status）：

| 从 | 到 | 站点 | gateway_status |
|---|---|---|---|
| （新建/复活） | pending | materialize insert/refresh（webhooks.rs:121,191）、manual 释放（webhooks.rs:545）、策略重排（webhooks.rs:741） | debouncing / quiet_hours_waiting / manual_reply_preparing / policy_reconciled / manual_reply_* |
| pending/retry/failed | running | claim CAS（tasks.rs:203） | （不改） |
| running | committing | prepare_task_commit（tasks.rs:60） | `{kind}_committing` |
| committing | sent | finalize_task_commit（tasks.rs:87） | 调用方给定（如 aggregated） |
| committing | retry | requeue_task_commit（tasks.rs:119） | 调用方给定 |
| running | outbox_enqueued | authorize_task_outbox（tasks.rs:516） | outbox_enqueued |
| running | retry | 失败退避（tasks.rs:696）、provider 不可用（tasks.rs:1190）、reclaim（tasks.rs:937） | retry_scheduled / blocked_provider_unavailable / claim_timeout_recovered |
| running | failed | 预算耗尽（tasks.rs:751）、回收 3 次（tasks.rs:885） | failed / claim_recovery_exhausted |
| failed | retry | memory revive（tasks.rs:1036）、Admin claim（tasks.rs:192 集内含 failed → running） | memory_candidates_arrived |
| pending 等 5 态 | pending（重排） | 策略 reconcile（webhooks.rs:741） | policy_reconciled |
| pending 等 5 态 | cancelled | legacy 行合并（webhooks.rs:729） | merged_into_reply_obligation |
| （任意含终态） | pending | durable 行被新入站复活（webhooks.rs:178-204 refresh 分支，无 status 前置条件） | 调度原因 |
| running/outbox_enqueued 等 | sent | AI 送达结算（webhooks.rs:438）、manual 送达（webhooks.rs:513）、outcome 无 claim 路径（tasks.rs:1620） | agent_reply_delivered / manual_reply_delivered / aggregated |

（`completed` 在本两文件无写点，闭集保留为 R10 reset alias，models.rs:916。）

### 4.2 全部超时 / 退避 / 窗口参数（默认值亲验 config.rs）

| 参数 | 值 | 出处 |
|---|---|---|
| 消息去抖窗口 | 2000ms，clamp [1000,10000]；profile 可覆盖 | config.rs:490-492；webhooks.rs:1625-1628 |
| 主 task worker tick | 30s | config.rs:493；tasks.rs:542-545 |
| inbound reply worker tick | 固定 250ms | tasks.rs:557 |
| inbound reply 并发 | 4，clamp [1,32] | config.rs:494-496；tasks.rs:1103 |
| task claim lease 超时 | 300s | config.rs:529；tasks.rs:790 |
| claim 心跳间隔 | (timeout/2).clamp(5,60) ⇒ 默认 60s | tasks.rs:1219-1222 |
| claim 回收上限 | 累计 claim_recovery_count ≥ 3 → failed | tasks.rs:865 |
| 任务重试预算 | max_attempts=3（≤0 修正为 3） | webhooks.rs:125；tasks.rs:626-630 |
| 重试退避 | base=min(60·2^(attempt-1),900)s ±20% jitter ⇒ 60/120/240/480/900 封顶 | tasks.rs:1207-1214 |
| LLM 不可用重试 | 恒 300s，`$inc attempt_count:-1` 不耗预算 | tasks.rs:1183-1204 |
| webhook 限流 | 30 次 / 60s / (ws,acct)，超限仍计数 | config.rs:539-541；webhooks.rs:929-987 |
| 限流事件去重 | 每账号每 UTC 天 1 条（day_bucket=epoch_ms/86400000） | webhooks.rs:2150-2199 |
| 验签时间戳窗口 | ±300s（含边界） | config.rs:799；webhooks.rs:2870 |
| 验签开关 | 默认 true | config.rs:798 |
| 静默唤醒 jitter | ≤900s（按 wxid 确定性） | config.rs:812；webhooks.rs:237-242 |
| manual 孤儿宽限 | 5min（ORPHAN_GRACE_MS） | webhooks.rs:591 |
| dispatcher 二次唤醒 | authorize 后 1050ms | tasks.rs:533 |
| 扫描批量 | handoff 100 / manual reconcile 100 / committing 20 / due 任务 20 | webhooks.rs:814,599；tasks.rs:986,1084,1153 |

### 4.3 `handoff_status` 取值（webhooks.rs:28-31）

| 值 | 含义 | 写点 |
|---|---|---|
| `pending` | 消息已落库、任务尚未物化（与 insert 同批写入） | webhooks.rs:1488 |
| `materialized` | durable 任务已承载该消息义务 | webhooks.rs:210 |
| `deferred` | 遗留值，仅作恢复扫描的读兼容，新代码不写 | webhooks.rs:30,65,811 |
| `ignored_not_managed` | 非 operatable 联系人或非 managed，只持久化不应答 | webhooks.rs:858,862,1527,1644 |

状态机：`pending/deferred → materialized | ignored_not_managed`（终态不可再改，mark filter 限定 $in [pending,deferred]，webhooks.rs:65）。

### 4.4 `dedupe_key` 构造规则（webhooks.rs:1442-1458）

1. `effective_message_id` = 顶层 9 键（`newMsgId/new_msg_id/msgId/msg_id/messageId/id/NewMsgId/MsgId/MessageId`，find_string 深度递归）`.or(_mcp.sourceMsgId)`；
2. 有 → `"message:{id}"`；无 → `"payload:{FNV-1a 64bit hex}"`（stable_payload_hash，webhooks.rs:1705-1713）；
3. 唯一性由 `conversation_messages` 的 partial unique index `(workspace_id, account_id, dedupe_key)`（`dedupe_key: {$type:"string"}`）原子保证（亲验 db/indexes.rs:810-822）；
4. 行为信号 dedupe：`bs::build_*` 用 message_id（缺失退化 observed_at 毫秒）为后缀，unique index `(ws, acct, dedupe_key)`（db/indexes.rs:527-540）；
5. 事件 dedupe：可选 `(workspace_id, dedupe_key)` partial unique（db/indexes.rs:930-948），rate_limit 事件用 `"rate_limit:{acct}:{day_bucket}"`。

### 4.5 webhook 返回体格式（全部 200 JSON，除错误）

| 场景 | body | 行号 |
|---|---|---|
| testMsg 探活 | `{"ok":true,"ignored":"callback_test","echo":<testMsg>}` | 1261-1266 |
| Online/Offline | `{"ok":true,"ignored":"online_event"\|"offline_event","type":<TypeName>}` | 1336-1341 |
| 领导消息已消费 | `{"ok":true,"routed":"principal"}` | 1420-1424 |
| 重复消息 | `{"ok":true,"duplicate":true}` | 1500 |
| 非私聊真人 | `{"ok":true,"skipped":"not_operatable_contact"}` | 1529-1531 |
| 正常处理 | `{"ok":true,"managed":<bool>,"queued":managed&&!deferred,"deferred":<bool>}` | 1647-1652 |
| 错误 | 400（bad json / unknown appId / invalid signature / missing sender / 多账号缺 appId）；429 RateLimited{retry_after, account_id}；5xx（DB 等内部错） | 1253,1284,1311,1380,1983;1355-1358 |

---

## 5. 偏差与疑点

**D1（文档层面偏差）**：webhooks.rs:1463 注释称 dedupe unique index 在 `db/indexes.rs:55-63`，实际亲验现位于 db/indexes.rs:810-822（代码演进后注释行号过期）。语义无误，仅坐标漂移。

**D2（尖锐边界）**：`reconcile_pending_inbound_handoffs` 对无法 decode 成 `ConversationMessage` 的行直接返回 Err（webhooks.rs:820-822），而 `tick`（tasks.rs:1121）与 `tick_inbound_replies`（tasks.rs:1069）都以 `?` 传播——一条形状损坏且 handoff_status 停在 pending/deferred 的消息行，会让**两个 worker 的每一轮 tick 都在该步中止**（主 tick 后续的 reclaim/扫描全部执行不到），且该行无自动退出机制（decode 失败不会被标 ignored）。正常写入路径不会产生这种行，但手工改库/半截迁移可触发。属"疑点：是否值得降级为 skip+error log"，未见 spec 论证。

**D3（config 粒度不一致，疑点）**：quiet-hours runtime 的加载粒度分裂——webhook 主路径与 handoff 恢复用 **contact 级** `load_user_operation_domain_config_for_contact`（webhooks.rs:1592-1597, 866-871），而 `settle_manual_reply_obligation`（webhooks.rs:487）与 `reconcile_workspace_reply_obligations`（webhooks.rs:667）用 **workspace 级** `load_user_operation_domain_config`。若存在 contact 级 override，人工结算/策略重排算出的 run_at 可能与入站路径不一致。是否刻意（策略编辑本就是 workspace 事件）未在注释中说明。

**D4（已知边界，代码注释自认，不修）**：
- A-03（webhooks.rs:1451-1454）：payload 完全无 msgId 时 dedupe 退化为 payload-hash，同内容连发第二条会被误判 duplicate 丢弃；生产 GeWe AddMsg 恒带 NewMsgId，仅自测面暴露。
- A-04（webhooks.rs:2845-2848）：验签无 nonce，±300s 窗口内可整包重放；依赖下游幂等（dedupe/幂等 $set/幂等 resolve）消化，无重复副作用。
- A-05（webhooks.rs:1975-1987）：未开验签 + 单账号时无 appId 回落 default 账号，是刻意保留的单账号部署兼容。

**D5（语义细节）**：`materialize_durable_inbound_task_at` 在乱序/晚到消息（`newer` 谓词不成立，两个 update 均 matched 0）时**仍**把该消息标为 materialized 并返回行内现值（webhooks.rs:210-222）。语义自洽（义务由现行行承载、回复上下文按消息表聚合），但"materialized"字面上并非"本消息触发了物化"，读代码时易误解。

**D6（事件文案）**：`follow_up_retry_scheduled` 文案"已安排第 {attempt_count}/{max_attempts} 次重试"（tasks.rs:721-723）中 attempt_count 因 claim 用 ReturnDocument::After 已含本次执行（tasks.rs:219,625），实义是"第 n 次执行失败"，非"第 n 次重试"。仅文案歧义，判定逻辑（`attempt_count < max_attempts`，tasks.rs:686）正确：max_attempts=3 ⇒ 最多执行 3 次。

**D7（微妙交互）**：`run_due_task_by_id` 的 filter 顶层无 `status` 键（due 条件在 `$or` 内），故 `claim_task_with_filter` 会追加注入 `status ∈ {pending,retry,failed}`（tasks.rs:191-193, 596-603）。合取后 failed 不满足 $or 不会被执行——行为正确，但正确性依赖"$or 分支枚举了全部可执行态"这一隐式耦合；若未来 $or 加分支而忘记 status 语义，注入的 $in 可能悄悄放行 failed。

**D8（性能疑点）**：`handle_outcome_aggregation_task` 的 reply_rate 对窗口内**每条 outbound 各发一次** count 查询（tasks.rs:1400-1441），30d 窗口 + 高发送量账号是 O(N) 次网络往返；`ensure_today_outcome_aggregation_tasks` 每 30s 对全部 account×2 无条件 insert（靠 dup-key 弹回，tasks.rs:1277-1325）。均为正确性无损的写/查放大。

**D9（观察）**：legacy 去抖层（PENDING/register_inbound/run_debounce_pipeline，webhooks.rs:989-1216）在 src/ 内已无生产调用方（Grep 亲验仅测试与注释引用），但仍为 `pub` 且被 `handle_managed_message_aggregated`（gateway.rs:158）这一同样仅剩 legacy 用途的入口依赖。属"待退役但保留兼容"状态，注释（webhooks.rs:993-994）已声明不得回流生产。

**D10（疑点）**：webhook 非 quiet 分支的低延迟唤醒是裸 `tokio::spawn`（webhooks.rs:1634-1641），若唤醒执行中 panic 不会有 supervisor 事件（对比 main.rs worker 的 spawn_supervised）；错误路径仅 tracing::error。兜底完备（250ms worker），但 panic 无痕迹（除 tokio runtime 默认输出）。

**D11（时区）**：`today_date_string`（tasks.rs:1345-1354）按 UTC 日截断生成聚合任务 date；`rate_limit_event_dedupe_key` 的 day_bucket 也是 UTC 天（webhooks.rs:2190-2192）。对中国时区运营者，"当日"指标窗口与本地日历日偏移 8 小时。注释自称"粗糙但足够幂等用"。

---

## 6. 覆盖自证（读过的行号区段清单）

### src/webhooks.rs（3089 行，100% 覆盖）
- 1–400（imports、常量、durable_inbound_task_id、mark_inbound_handoff、materialize 双入口、policy_run_at、pause_reply_obligation_for_manual、advance_covered_watermark 起始）
- 400–799（advance 收尾、settle_ai/manual、manual_outbox_settlement、reconcile_manual/workspace）
- 800–1199（reconcile_pending_inbound_handoffs、rate limit、legacy 去抖注释与实现、run_debounce_pipeline 主体）
- 1200–1599（debounce panic 兜底、reload_managed_contact、wechat_webhook 全体、contact 更新与信号采集）
- 1600–1999（quiet/debounce 物化、ensure_wake_followup_task、hash/dup-key/panic helpers、解析函数族、resolve_account_context）
- 2000–2399（emit_unknown_app_id_event、is_operatable_person、upsert_webhook_contact、rate-limit 事件、roster、测试模块前半）
- 2400–2799（inbound_msg_type_tests 后半、debounce_tests）
- 2800–3089（rate_limit_dedupe_tests、WebhookSigError、verify_webhook_signature、webhook_sig_tests、reply_obligation_tests、EOF）

### src/tasks.rs（1972 行，100% 覆盖）
- 1–399（TaskClaim、prepare/finalize/requeue、marker filters、TaskRunContext、claim_task_with_filter、claim_by_id*、task_claim_is_current、bind_task_decision、adopt 前半）
- 400–799（adopt 后半、authorize_task_outbox、两个 worker 循环、execute_claimed_task、run_due_task_by_id、process_claimed_task、reclaim 前半）
- 800–1199（reclaim 后半、reconcile_committing、revive_memory、dedupe_inbound_candidates、tick_inbound_replies、tick、退避函数族、心跳前半）
- 1200–1599（心跳后半、ensure_today_outcome、is_duplicate_key、today_date_string、handle_outcome_aggregation 主体）
- 1600–1971 + 1968–1972 尾部单读（outcome 无 claim 收尾、reconcile_outcome_commit、outcome_metric_write_filter、mod tests 全体、EOF）

### 跨文件核验（支撑断言，非本任务通读对象）
- models.rs:908-941（ALLOWED_AGENT_TASK_STATUS 闭集 + 断言函数）
- config.rs:36-41,66-67,77-80,317-318,369-373,398-399,489-541,754-812（本记录引用的全部默认值）
- db/indexes.rs:527-540, 683-692, 807-822, 930-956（messages/behavior_signals/events 三处 dedupe 索引）
- gateway.rs:150-320（handle_managed_message_aggregated、handle_follow_up_task_with_claim、handle_durable_inbound_reply_task 契约段）
- quiet_hours.rs:20（DEFERRED_INBOUND_REPLY_KIND="deferred_inbound_reply"）
- main.rs:57, 205-234（APP_STARTED_AT set、双 worker spawn_supervised 启动）
- lib.rs:44-50（APP_STARTED_AT 定义）
- Grep 全库：`register_inbound|run_debounce_pipeline` 无生产调用方；`run_task_worker|run_inbound_reply_worker|APP_STARTED_AT` 调用点

---

## 追记：24 号交叉验证回写（2026-08-13，主会话执行）

- 笔误修正：materialize refresh 分支 `$unset` 清 **11** 个字段（非 12，webhooks.rs:196-201 亲数）；文件总行数实测 webhooks.rs **3088**、tasks.rs **1971**（原多报 1 行）。函数级锚点经 75 点抽验无偏移，本记录可信度评估为"可直接作改动依据"级。
