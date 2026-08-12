# outbox/请示/引荐/发送 深读记录（核证日期 2026-08-13）

> 覆盖范围：`src/agent/outbox.rs`、`src/agent/outbox_dispatcher.rs`、`src/agent/escalation/`（mod/policy/labels/ledger/logic/holding_reply 共 6 文件）、`src/agent/referral.rs`、`src/agent/media_send.rs`、`src/agent/send_ledger.rs`、`src/agent/pacing.rs`、`src/agent/quiet_hours.rs`。全部逐行读完（合计 11672 行），所有断言均附 `file:line`（行号为核证日期当天工作区版本，含未提交改动）。跨文件引用（models.rs / config.rs / mcp.rs / types.rs / tasks.rs / gateway.rs / webhooks.rs）均已用 Grep/Read 当场核验。

---

## 1. 模块地图

```
                              ┌────────────────────────────────────────────────┐
                              │  写入侧（业务）                                  │
   gateway.rs（决策批次） ────┤  outbox::enqueue()  唯一入队入口                │
   escalation/（请示卡/安抚） │   outbox.rs:221                                 │
   routes（manual_send）      └────────────────┬───────────────────────────────┘
                                               │ agent_send_outbox（Mongo, tenant-scoped unique idempotency_key）
                                               ▼
   ┌────────────────────────────────────────────────────────────────────────────┐
   │ outbox_dispatcher.rs  后台单例 worker（main.rs:242 spawn）                  │
   │  tick: reclaim → 3 个 reconciler → (claim → process_entry)×≤16             │
   │  process_entry: 授权门(请示卡/事故/SR-034 task)→二次安全门→contact 门       │
   │   →账号在线门→崩溃恢复核对→pacing 闸→最后可取消点 CAS→MCP 分流             │
   │   →成功/重试/终态/delivery_unknown → finalize（承诺/跟进任务/relay 终结）    │
   └──────┬──────────────────────┬──────────────────────┬──────────────────────┘
          │文本                   │媒体                  │名片
          ▼                      ▼                      ▼
   gateway::send_outbound  media_send::send_       referral::send_
   _message                outbound_media          outbound_namecard
          │（三路共用 mcp.rs::logged_send_call_for_account + classify_send_receipt）
          ▼
        MCP server（message_send_text / media_upload_base64+message_send_* / message_send_namecard）

   escalation/ 请示通道（幕后领导裁决）：
     决策墙触发（gateway.rs:1930 trigger_principal_escalation approved 路 /
                 gateway.rs:3762 → mod.rs:196 escalate_held_decision hold 路）
       → ledger::insert_pending_escalation（台账+冻结 policy+短码）
       → ledger::materialize_principal_card_delivery（请示卡走 outbox，source_kind=principal_escalation）
       → 领导微信回复（webhooks.rs:1412 → mod.rs:462 handle_principal_reply）
       → mod.rs:417 interpret_principal_reply（LLM 解读→PrincipalDecision）
       → ledger::resolve_escalation → ledger::materialize_relay_task（task _id=escalation _id）
       → tasks.rs 领 relay task → mod.rs:288 handle_principal_decision_relay_with_claim
       → gateway.rs:2027 relay_principal_decision_to_customer（AI 口吻转述，仍走 outbox）
       → dispatcher finalize → escalation/ledger.rs:495 terminalize_principal_relay_for_task("delivered")
     旁路：scan_escalation_timeouts（超时改派/链尾安抚, tasks.rs:1138 每 tick）
           reconcile_pending_relay_intents / reconcile_principal_card_deliveries（tasks.rs:1124-1125）
           emit_knowledge_gap_proposal（可泛化裁决→draft 知识提案）

   辅助纯函数层：
     pacing.rs        账号级发送间隔（拟人节奏）
     quiet_hours.rs   作息门控（22–8 静默、醒来重排、per-contact jitter）
     send_ledger.rs   素材/名片主动发送台账 + 转化回扫
```

关键集合：`agent_send_outbox`（发送队列）、`agent_principal_escalations`（请示台账）、`agent_decision_reviews`（决策批次封印）、`agent_tasks`（follow_up / principal_decision_relay / inbound_reply）、`agent_send_ledger`（发送台账）、`mcp_call_logs`（post-hoc 核对证据）、`agent_events`（审计）。

---

## 2. 逐文件深读

### 2.1 `src/agent/outbox.rs`（1686 行 = 966 生产 + 720 测试）

模块头（outbox.rs:1-22）声明三条核心不变量：①强幂等（业务 hash 经 `(workspace_id, account_id)` 包装成 v2 key，tenant-scoped unique 索引兜底）；②空 source_event_id 走 synthetic 兜底并写 warning 事件；③状态枚举严格闭集，禁用旧值 `"failed"`。

#### OutboxStatus（outbox.rs:43-85）
六值闭集：`pending / in_flight / sent / failed_terminal / canceled / delivery_unknown`。`as_str`（:62-71）是唯一落库字面量来源；`from_str`（:74-84）逆解析，未知/历史脏值→`None`。`delivery_unknown` 的语义（:55-56）：**已跨过远端发送边界但本地无可验证回执；禁止自动重发，等待离线核验**。测试锁死旧值 `"failed"` 不被接受（:987-991）。

#### OutboxError / EnqueueOutcome / EnqueueRequest
- `OutboxError`（:91-102）三分支：`Db`（透传 mongo 错）、`Invalid`（入参非法）、`Invariant`（幂等键冲突却读不到既有行=事实链损坏）；映射到 `AppError::Db/BadRequest/External`（:104-112）。
- `EnqueueOutcome`（:117-136）：`Created{outbox_id, idempotency_key}`；`IdempotentSkip{idempotency_key, existing_outbox_id, existing_run_id, existing_decision_id, existing_status}`——回读既有行五元组，供调用方区分"本 decision 重试"与"跨 run 去重"。
- `EnqueueRequest`（:141-165）：`workspace_id / account_id / contact_wxid / run_id / decision_id(Option) / source_event_id / source_kind / content / media_asset_id(Option) / referral_card_id(Option) / max_attempts`。media 与 referral 字段互斥（:162 注释），非空即分别表示素材条目/名片条目，两者都允许空 content。

#### 调度元数据纯函数
- `delivery_priority_for`（outbox.rs:169-185）**服务端派生，不信任调用方**：媒体或名片条目一律 **20**（最低）；否则按 source_kind：`manual_send`→**100**、`inbound|inbound_message`→**90**、`principal_escalation`→**80**、`follow_up|follow_up_task`→**60**、`system_incident`→**40**、其它→**50**。
- `run_sequence_for`（:188-204）：同 decision 内稳定序。名片→**20000**，媒体→**10000**，文本从 `source_event_id` 的 `#segN` 后缀解析 N（≥0，解析失败→0）。即同 decision 内"文本分段 < 媒体 < 名片"。

#### enqueue（outbox.rs:221-441）——唯一业务入队入口
1. **入参校验**（:224-240）：workspace/account/contact/run_id 非空；content 是否必填由 `content_required_for` 决定（纯文本必填，媒体/名片可空）。
2. `content_hash = sha256(content)`（:243）；`day_bucket = now_ms / 86_400_000`（:256）。
3. **幂等键构造全规则**（:262-286）——先判 `media_routes_synthetic(media, referral, source_event_id)`（:586-592）：
   - 媒体条目（media_asset_id 有值）**无论 source_event_id 是否为空**一律走 synthetic（media-asset Task 8 硬伤③方案甲，:257-261 注释：否则非空事件路径 key 不含 asset_id、媒体 content 空→sha256("") 全同→同一入站发两个不同文件撞键漏发第二个）；
   - 名片条目同理一律 synthetic；
   - 纯文本仅当 `source_event_id.trim().is_empty()` 才 synthetic。
   - synthetic 形态由 `compute_synthetic_key`（:543-568）给出（按优先级）：
     - 名片：`synthetic_namecard:{run_id}:{contact_wxid}:{card_id}`（:555-557）
     - 媒体：`synthetic_media:{run_id}:{contact_wxid}:{asset_id}`（:560-562）
     - `manual_send`：`synthetic_manual:{account_id}:{contact_wxid}:{content_hash}:{day_bucket}`（:563-564）——P1-4：摘掉 run_id，改按"内容级幂等"，admin 当天双击同内容被拦、次日重发放行；
     - 其余：`synthetic:{run_id}:{contact_wxid}:{content_hash}`（:566）。
   - 非 synthetic（普通文本+非空事件）：`{source_event_id}:{contact_wxid}:{content_hash}`（:279-283）——**不含 run_id**，相同入站事件+同内容跨 run 共享 key（R13.10 item 5，测试 :1271-1289）。
   - 两分支产物先 `sha256_hex` 成 64-hex legacy key，再经 `scoped_outbox_idempotency_key(workspace, account, legacy)`（:458-475）二次哈希：`sha256("outbox-idempotency-v2" ‖ len-prefix(workspace) ‖ len-prefix(account) ‖ len-prefix(legacy))` → `"v2:{hex64}"`。长度前缀防分隔符歧义；`v2:` 标记使 m038 迁移可重启（:456-457 注释）。校验函数 `is_scoped_outbox_idempotency_key`（:477-481）。
4. synthetic 路径写 `outbox_synthetic_idempotency_key` **warning** 事件（:288-309，运维监控频率用）。
5. `max_attempts` 兜底（:311-315）：`<=0 → 3`，上限 `min(10)`。
6. 组装 `OutboxEntry`（:317-360）：`attempt=0, status=pending, claim_generation=0, cancel_requested=false, reclaimed_in_flight=false, reclaim_count=0`，delivery_priority / run_sequence 由上述纯函数派生。
7. **insert + DuplicateKey 容错**（:362-440）：
   - 成功 → 写 `outbox_created` info 事件（:370-385）→ `notify_outbox_work()` 唤醒本进程 dispatcher（:389，注释：Mongo 行已 durable，周期扫描仍是跨进程/丢信号兜底）→ 返回 `Created`。
   - `is_duplicate_key_error`（:484-496，code 11000/11001，含 BulkWrite 形态）→ 回读既有行（读不到 → `Invariant` :406-415）→ 写 `outbox_idempotent_skip` warning 事件 → 返回 `IdempotentSkip`。
   - 其它 → `OutboxError::Db`。

#### 辅助纯函数
- `write_outbox_event`（outbox.rs:502-532）：直接 insert `agent_events`（不复用 gateway 的同类函数以避免 outbox→gateway→outbox 循环依赖，:500-501）。
- `backoff_with_jitter_seeded(attempt, jitter01)`（:599-607）：`exp=clamp(attempt,0,10)`；`base=(1<<exp)*5` 秒；jitter=±20%（`(j-0.5)*2*0.2`）。attempt=1→10s、2→20s、3→40s（jitter=0.5 时精确命中，测试 :1311-1322）；attempt>10 clamp 防 i64 溢出（:1341-1345）。
- `outcome_signals_stop`（:610-615）：outcome 字符串 contains `stop_requested` 或 `cooldown_requested`。
- `outbox_status_is_user_cancelable`（:623-628）：仅 `pending/in_flight`；`from_str` 不识别的脏值一律不可取消。
- `check_second_safety_gate_pure`（:640-670）——**二次门四条件**，顺序即优先级：
  0. `!is_managed` → `"not_managed_at_send"`（B-03：发送前 fresh 复核，决策运行期 admin 把 contact 改 normal / contact 被删都在此拦住，:650-654）；
  1. `cooldown_until > now` → `"contact_cooldown_active"`；
  2. `last_inbound > decision_created && outcome_signals_stop(outcome)` → `"user_stop_requested_after_decision"`；
  3. `now - entry_created > stale_threshold`（30min）→ `"outbox_stale_30min"`。

#### 取消通道（用户/决策驱动）
- `cancel_for_decision(state, workspace_id, decision_id, reason)`（outbox.rs:679-806）：
  - find `workspace+decision_id+status∈{pending,in_flight}`（:693-705），逐条用**update pipeline**（`$cond` 按 DB 当前 status 分支，:724-769）做 `find_one_and_update`（filter 带 `$or:[pending, {in_flight, cancel_requested≠true}]` :716-722，ReturnDocument::Before）：
    - DB 当前是 pending → `status=canceled` + `$$REMOVE` worker_id/locked_until/claim_token；
    - DB 当前是 in_flight → 状态不变，`cancel_requested=true` + `cancel_requested_at=now`（保留 claim token，由 dispatcher 在最后可取消点或真实回执后收敛，:676-678 doc）；
    - 已 `cancel_requested=true` 的 in_flight 不再匹配（不重复写事件）。
  - pipeline 的意义（:857-861 同款注释）：分支按**数据库当前状态**评估而非 cursor 快照——find 与 update 之间 worker 抢占了 pending 行时，原子转为登记 in-flight 取消而不是丢失 stop。
  - 事件：pending→`outbox_canceled`；in_flight→`outbox_cancel_requested`（"绝不提前宣称已取消"，:817）。返回实际改动条数。
- `cancel_for_contact_on_user_reaction(state, ws, account, contact)`（:820-964）：同款 pipeline，过滤维度换成 `(workspace, account, contact_wxid)`，`cancel_reason="user_reaction_stop_requested"`；并发已被其它路径推进的行跳过且不写事件（:927-931）。调用方：reaction.rs:316/568（用户 stop/cooldown 反应，best-effort，失败只 log）。
- 测试区（:968-1686）：状态往返、脏值拒绝、synthetic key 各形态（manual 摘 run_id :1056-1082、day_bucket 分日 :1086-1108、非 manual 保留 run_id :1113-1139、媒体按 asset_id 分键 :1223-1266、同事件不同 asset 不撞键 :1172-1217、名片按 card_id 分键 :1659-1685）、backoff、二次门全分支、可取消集合闭集。

### 2.2 `src/agent/outbox_dispatcher.rs`（3771 行 = 3285 生产 + 486 测试）

模块头（:1-22）：worker 周期扫描四步（reclaim→claim→二次门→MCP）；设计原则：单 tick 抢占循环、每 entry 事件 ≤20 条、lease 严格大于单条 send 外层 timeout 故无需续约。

#### 常量（全部核证）
| 常量 | 值 | 位置 | 说明 |
|---|---|---|---|
| `MCP_SEND_TIMEOUT_SECONDS` | 150s | :154 | 整条 send（可含多次顺序 MCP 调用）外层 timeout。取值约束（finding ①，:139-153）：`MCP_CLIENT_TIMEOUT_SECONDS(60, mcp.rs:25) × 最坏 2 次 = 120 ≤ 150 < lease 180`。历史 5s/30s 太短会取消 in-flight future→丢 mcp_logs→post-hoc 查不到→误重发 |
| `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS` | 15s | :158 | timeout 兜底里 chat_search 核对的独立短超时 |
| `MAX_SEQUENTIAL_MCP_CALLS_PER_SEND` | 2（cfg(test)） | :165-166 | 媒体=上传+发送 2 次；文本/名片 1 次 |
| `STALE_THRESHOLD_MILLIS` | 30min | :169 | 二次门陈旧阈值 |
| `PER_ENTRY_EVENT_CAP` | 20 | :172 | 单 entry 事件上限（R13.7） |
| `PER_TICK_PROCESS_CAP` | 16 | :175 | 单 tick 处理上限 |
| `ACCOUNT_OFFLINE_DEFER_SECONDS` | 60s | :181 | 账号掉线 defer 间隔（不耗 attempt） |
| `OUTBOX_MAX_RECLAIMS` | 5 | :185 | F-04 reclaim 上限，超过转 failed_terminal |
| `DELIVERY_FINALIZE_LEASE_SECONDS` | 60s | :1157 | finalize 短租约 |
| `DELIVERY_FINALIZE_RECONCILE_BATCH` | 20 | :1158 | |
| `OUTBOX_ENQUEUE_RECONCILE_GRACE_SECONDS` | 60s | :1159 | 入队恢复宽限 |
| `OUTBOX_ENQUEUE_RECONCILE_BATCH` | 20 | :1160 | |
| `AGING_CLAIM_EVERY` | 10 | :3269 | 每 10 次 claim 一次 FIFO 抗饿死 |
| `DEFAULT_POLL_INTERVAL_SECONDS` | 5s | :3225 | 兜底轮询 |
| `DEFAULT_LEASE_SECONDS` | 180s | :3232 | 必须 **严格 >** 150s send timeout（否则发送中被 reclaim→并发重发） |

`worker_id()`（:188-195）：`hostname:pid:uuid`。

#### 请示卡发送授权 `principal_card_send_is_authorized`（:62-137）
仅对 `source_kind=principal_escalation` 生效（其余恒 true）。`principal_card_source_identity`（:41-52）严格解析 `principal-card:{escalation_oid}:{generation}`（generation≥1、无多余段；解析失败→**不授权**）。三步核验（base filter = escalation `_id+workspace+status=pending+principal_wxid=收件人+protocol.principal_account_id+delivery_generation+delivery_content=本条 content`，:75-83）：
1. 已确认态：`delivery_state=queued && delivery_outbox_id==本 outbox` → true（:84-98）。
2. 待确认态 CAS：`delivery_state=pending_enqueue && 无 outbox_id` → 置 `queued+outbox_id`（:100-120）——**dispatcher 赢了 enqueue↔ack 竞态时自己补完同代 CAS**，而不是错杀合法卡（:59-61 doc）。
3. CAS 0 改动 → 重读"已被并发 ack 成同 outbox"（:122-136），仍不命中才判 stale。
作用：admin resolve/改派后，旧代队列中的卡在 claim 后与最后可取消点前两处复查（process_entry :2769/:2936），失权即 cancel（reason `principal_escalation_generation_no_longer_authorized`）。

#### reclaim（崩溃恢复）`reclaim_expired_leases`（:205-318）
对 `in_flight && locked_until < now` 分三段收敛 + 一段止损：
1. **安全完成取消**（:209-234）：`cancel_requested=true && send_started_at 空` → `canceled`（reason `cancel request recovered before remote send`），$unset worker/locked/claim。取消在不可逆边界前获胜，可安全完成。
2. **晚到取消/名片跨界 → delivery_unknown**（:238-263）：`send_started_at≠null && (referral_card_id≠null || cancel_requested=true)` → `delivery_unknown`（名片无权威 post-hoc 查询 API；跨边界后的取消也不能谎报 canceled 或重放，:235-237）。
3. **通用回收**（:264-286）：剩余全部 → `pending` + `reclaimed_in_flight=true` + `$inc reclaim_count` + $unset worker/locked/claim/send_started_at。该标记告诉 process_entry：上一 worker 可能已送达，重发前必须 post-hoc 核对（:197-201）。
4. **F-04 止损**（:293-310）：`pending && reclaim_count > 5` → `failed_terminal`（last_error "reclaim 超限"）。单独一遍 update 因 `$inc` 后的新值无法在同一 update 分流。
返回三段修改数之和（:317）。

#### 原子抢占 `atomic_claim_pending[_with_policy]`（:325-388）
filter（:347-358）：`status=pending && cancel_requested≠true &&（next_retry_at 不存在/null/≤now）`。**cancel_requested≠true 是防御不变量**：持久化的取消意图绝不能被新 claim 抹掉（:349-352）。update（:359-373）：`in_flight + worker_id + claim_token(uuid) + locked_until=now+lease + cancel_requested=false` + `$inc claim_generation` + `$unset cancel_requested_at, send_started_at`。sort（:374-380）：正常 `delivery_priority:-1, created_at:1, run_sequence:1, _id:1`（客户/手动文本全局优先，等优先级内按时间和段序）；aging pass（prefer_oldest，每 10 次 claim 一次）纯 FIFO `created_at:1, run_sequence:1, _id:1` 防低优先级媒体/运营行饿死（:333-334, :374-376）。`ReturnDocument::After` 保证多 worker 并发恰一人成功。

#### claim 归属与边界 CAS 工具
- `active_claim_filter`（:390-399）：`_id + in_flight + worker_id + claim_token`——所有后续状态推进都以"我仍持有 claim"为前置。
- `send_not_started_filter`（:401-408）：`send_started_at 不存在或 null`。
- `remote_send_start_filter`（:690-708）：active claim + `cancel_requested≠true` + task 授权 marker（若有）+ send 未开始。测试 :3392-3415 锁死该 filter 逐字段形态。
- `begin_remote_send`（:710-747）：**最后可取消点**。CAS 置 `send_started_at=now`；matched=0（取消赢了/lease 被回收/claim 被替换）→ 尝试 `complete_requested_cancel_before_send` 后返回 false，worker 必须在 MCP 前停下（:2957-2959）。
- `complete_requested_cancel_before_send`（:630-686）：active claim + `cancel_requested=true` + send 未开始 → `canceled`；写 `outbox_canceled` 事件（reason 取 entry.cancel_reason 或默认）。仅在远端边界未跨越时才真实。
- `commit_sent_if_owned`（:749-784）：active claim → `sent + sent_at + delivery_finalize_pending=(decision_id.is_some && 纯文本)`；note（post-hoc 场景）写入 last_error 保留诊断（`sent_unset_fields` :786-797：普通成功清 last_error，post-hoc 保留）。
- `mark_delivery_unknown_if_owned`（:799-851）：active claim → `delivery_unknown + last_error=reason`，写 `outbox_delivery_unknown` warning 事件。
- `settle_late_cancel_as_delivery_unknown`（:856-911）：active claim + `cancel_requested=true` + `send_started_at≠null` → `delivery_unknown`。理由（:853-855）：这种 entry 绝不能回 pending——下一次 claim 会清 cancel_requested 并可能重放一次实际已成功的投递。

#### SR-034 Task 发送授权（:412-626）
Task 产生的 outbox 在触达 MCP 前必须证明发送意图已由同一 task claim 提交。
- `TaskSendAuthorization`（:412-420）：`Authorized(Option<token>)`（None=非 Task/历史 outbox）/ `Building`（gateway 还在建同 decision 的后续分段）/ `Stale(reason)`。
- `classify_task_send_authorization`（:422-452）纯函数：task 当前 claim_token ≠ 绑定 token 或 decision 不匹配 → Stale；outbox 单文档 marker 与 token 不一致 → Stale；task `running` → Building；`outbox_enqueued|sent` → 有 marker 才 Authorized（无 marker → Building）；其它 status → Stale。注（:441-443）：task `sent` 只表示全部文本段送达，同 decision 的媒体/名片行仍由同一不可变 token+decision 对授权。
- `task_send_authorization`（:454-559）DB 版：无 decision_id → Authorized(None)；review 不存在 → Authorized(None)（历史/手工 outbox）；review.status：`outbox_enqueuing`→building 标志、`outbox_enqueued|sent`→非 building、其它→Stale（decision 批次失权）；review 无 task 绑定 → building?Building:Authorized(None)；绑定不完整 → Stale；task 不存在 → Stale；**marker 修复**（:518-551）：行缺 marker 且 task 已提交（outbox_enqueued|sent）且 token/decision 匹配 → CAS 把 `task_send_authorization_token` 写到本 in_flight 行（媒体/名片行可能晚于文本授权物化，仅可从已提交的同 task/token/decision 继承）；最后 classify，review 仍 building 则除 Stale 外一律降级 Building（:552-558）。
- `defer_until_task_authorized`（:563-602）：无损退回 pending + `next_retry_at=now+1s`（**不是失败**：不加 attempt/reclaim_count、不置 reclaimed_in_flight）；CAS 失败 → 尝试完成取消。tasks.rs:533 配套 `notify_outbox_work_after(1050ms)` 定时二次唤醒。
- `enforce_task_send_authorization`（:604-626）：Authorized→Some(token)；Building→defer→None；Stale→`cancel_entry("stale_task_claim: {reason}")`→None。

#### 二次安全门 `second_safety_gate`（:918-978）
**豁免**：`principal_escalation / principal_clarification / system_incident` 三种内部通知直接放行（:922-929）——收件人是领导/管理员而非客户，contact 门与 stop 语义不适用。其余条目：查 contact（cooldown_until、last_inbound、`is_managed = agent_status==Managed`，contact 不存在 → is_managed=false）+ 若有 decision_id 查 review.outcome_status；`decision_created_ms` 取 **entry.created_at**（:967，见 §5 疑点 3）；委托 `check_second_safety_gate_pure`。任一命中 → process_entry 走 `cancel_entry(reason)`。

#### run 级聚合 `aggregate_run_outbox_status` / `refresh_run_log_outbox_status`（:980-1155）
聚合规则（:980-1023）：任一 in_flight→`in_flight`；任一 pending→`pending`；全 sent→`sent`；部分 sent→`partially_sent`；否则有 failed_terminal→`failed_terminal`；有 delivery_unknown→`delivery_unknown`；剩下→`canceled`。刷新（:1042-1155）：先在 run log 上原子 `$inc outbox_refresh_generation` 领取代数（:1046-1071），查询完再以 `run_id+generation(+status)` CAS 写回（:1122-1136）——旧快照即使后完成也因代数不匹配放弃，防 sent 倒退成 pending；run status=`outbox_enqueuing` 恒写 `pending`，`outbox_enqueue_partial_failure` 把全 sent 钉成 `partially_sent`（:1100-1120）。

#### 送达终结 finalize（:1288-1733）+ 两个 reconciler
`finalize_delivered_text_decision`（:1288-1733）——一条纯文本 entry 确认送达后，检查同 decision 全部文本分段是否均已送达，然后做"送达后副作用"并把 review 推到 `sent`：
1. 仅纯文本+有 decision_id（:1293-1298）。读 review 快照；`send_gateway_result.deliveryKind=="expired_principal_authorization_holding"` 的过期安抚回复跳过 decision 解码（:1313-1341，其 run log 无正常决策体）。
2. 文本段完整性：`expected_segments = review.expected_text_segments(>0) else 现存文本行数`（:1359-1366，新记录入队前固化段数，配置热更新不影响本 decision 口径；历史记录以现存行数为最佳事实）；现存 < 期望 或 存在未 sent 文本行 → return（:1367-1387）。
3. review 短租约 CAS（:1389-1442）：`outbox_enqueued` 或 `delivery_finalizing+锁过期` → `delivery_finalizing + worker(uuid) + locked_until(+60s)`；抢不到且 review 已 `sent` → 上一 finalizer 在清 marker 前崩溃 → 只清 outbox 侧 marker（`clear_delivery_finalize_markers` :1267-1283，置 delivery_finalized_at、$unset delivery_finalize_pending）。
4. 副作用块（:1463-1682，全部幂等）：
   - 承诺：decision.last_commitment 非空 → contacts `$push` commitments（filter `commitments.text $ne` 防重，:1478-1487）；
   - **follow_up 任务创建**（:1489-1565）：decision.follow_up.needed 且 run_at 可解析 → 读 runtime 快照 `maxPendingFollowUps`/`followUpExpiresHours` → pending follow_up 计数未达上限 → 以 **`_id = decision_id`** upsert `$setOnInsert` AgentTask{kind=follow_up, review_required=true, max_attempts=3, gateway_status="scheduled_after_delivery", expires_at=run_at+expires_hours}（同 decision 天然幂等）；run_at 非法只 warn 跳过（:1557-1564）；
   - 回复义务结算（:1568-1598）：coverage_kind=`manual_reply`→`webhooks::settle_manual_reply_obligation`；`passive_reply`→按冻结水位 `covers_through_inbound_id/created_at` 调 `settle_ai_reply_obligation`（防误覆盖更晚 inbound）；
   - relay 终结（:1600-1629）：source_task 或按 decision 反查 `kind=principal_decision_relay` 的 task → `terminalize_principal_relay_for_task(..., "delivered")`；
   - task 终态（:1631-1679）：新协议按 `(task_id, claim_token, outbox_decision_id)` 精确 CAS `outbox_enqueued→sent`；旧协议按 source_decision_id update_many `running|outbox_enqueued→sent`。
5. 副作用失败 → 解锁 review（保持 delivery_finalizing + outbox marker，后续 tick 重试；**绝不回 pending 故不会重发**，:1461-1462, :1684-1703）；成功 → review CAS `delivery_finalizing→sent` → 清 outbox marker（:1705-1732）。

`reconcile_delivered_decision_finalizations`（:1736-1790）：扫 `sent+delivery_finalize_pending=true+decision_id≠null+纯文本`（batch 20），按 review status 分诊 `delivery_finalize_reconcile_action`（:1255-1265）：`outbox_enqueued|delivery_finalizing`→Finalize、`outbox_enqueuing`→Wait、`sent|其它|无 review`→Clear；contact 已删 → 清 marker。

`reconcile_stale_outbox_enqueues`（:1843-2183）：恢复 gateway 写入 `outbox_enqueuing` run log 后崩溃的窗口（run log 是恢复标记、**最后 CAS 提交**，:1838-1842, :2159-2160）。扫 `status=outbox_enqueuing && created_at ≤ now-60s`（batch 20）：无 review → run 直接判 Failed（:1865-1882）；最新文本行仍在宽限期内 → 等下轮（:1898-1918）；`stale_enqueue_effective_action`（:1187-1204，review 已到终态则投影既定结论；`outbox_enqueuing` 时按 `expected vs actual` 三分 Enqueued/PartialFailure/Failed，actual=0→Failed，expected≤0→Enqueued 兼容旧记录 :1169-1185）。SR-034 task 绑定核验（:1934-2068）：绑定不完整/task 消失/token 或 decision 被替换 → `reconcile_stale_task_claim`（:1792-1836：cancel_for_decision("stale_task_claim") + review→`stale_task_claim` + run→aborted）；`(Enqueued, running)` → **skip，绝不代 worker 授权**（:1997-2001）；`(Enqueued, outbox_enqueued|sent)` → 给全 decision 行补 token（:2002-2015）；`(Partial|Failed, ...)` → task cancelled + cancel_for_decision("outbox_enqueue_interrupted")（:2016-2054）。之后 review CAS `outbox_enqueuing→{outbox_enqueued|partial|failed}`（失败则核对现状兼容性 `stale_enqueue_review_status_compatible` :1227-1246）；旧协议 task 修复（:2113-2158）；最后 run CAS 提交 + refresh（:2161-2172）。

#### 重试/终态/defer
- `cancel_entry`（:2189-2242）：active claim + **send 未开始**（:2200-2202：安全门取消只在不可逆边界前真实，stale 调用者不得把已开始投递改写成 canceled）→ `canceled+cancel_reason`，事件 `outbox_canceled`，refresh。
- `effective_max_attempts`（:2247-2253）：`<=0→3`，与 enqueue 侧同口径（F-02，对正常入队是死代码，防历史脏文档口径漂移）。
- `schedule_retry_or_terminal`（:2260-2377）：先 `settle_late_cancel_as_delivery_unknown`（跨界晚到取消优先收敛，:2266-2268）；filter=active claim+`cancel_requested≠true`（:2276-2278：取消优先于重试/终态清算，否则取消可能在预检后落地又被翻回 pending）；`next_attempt=attempt+1`：
  - `< max` → `pending + attempt + next_retry_at=now+backoff(fastrand jitter) + last_error`，事件 `outbox_retry_scheduled`（warn）；
  - `≥ max` → `failed_terminal + attempt + last_error`，事件 `outbox_failed_terminal`（error）。
  - 两分支 matched=0 → 依次尝试完成边界前取消/跨界取消收敛（:2305-2308, :2351-2354）。
- `defer_account_offline`（:2384-2440）：⑪ 账号掉线 ≠ 发送失败：`pending + next_retry_at=now+60s`，**attempt 不变**、不走 terminal；事件 `agent.send_deferred_account_offline`（status="deferred"）。
- `defer_account_pacing`（:2470-2525）：同构，`next_retry_at = last_sent_at + interval`；事件 `agent.send_deferred_account_pacing`。
- `account_last_sent_at_ms`（:2444-2466）：查 `(workspace, account, status=sent)` 的最大 sent_at（sort sent_at:-1 limit 1）。

#### post-hoc 核对（防重发的证据学）
- `mcp_already_succeeded`（:2534-2555）+ `mcp_success_filter`（:2557-2580）：本地 `mcp_call_logs` 中查 `tool_name=message_send_text + request.recipient + request.content + error=null + (response.ok==true 或 无 ok 字段但 newMsgId 为非空 string) + created_at ≥ entry.created_at-5min`。时间下界回看 5 分钟容差，避免历史同内容误判（:2532-2533）。
- `verify_delivery`（:2594-2647）三路分流（F-01 统一崩溃恢复与 timeout 两个窗口的不对称）：
  - 名片：无权威查询 API → 恒 `Inconclusive`（:2599-2600）；
  - 媒体：`media_send::media_delivery_verification`（按 media_id 定位，见 §2.9）；
  - 文本：**先查权威 `chat_search_outbound`**（MCP server 真实已发记录，15s 独立超时）：`Ok(true)`→Delivered、`Ok(false)`→**NotDelivered**（权威空结果才可证明未送达）、出错/超时→回落本地 `mcp_already_succeeded`：命中→Delivered、未命中→`Inconclusive`（本地日志只能证成功、不能以"无日志"证失败，:2626-2645）。
- `DeliveryVerification`（types.rs:35-39）三态语义（types.rs:31-33）：缺证据不是 NotDelivered 而是 Inconclusive。
- `commit_verified_delivery`（:2651-2686）：commit_sent_if_owned(note)→事件 `outbox_sent_post_hoc`（warn）→refresh→finalize+send_ledger（与正常成功同副作用）；claim 已易主 → false 只 warn。
- `settle_ambiguous_send`（:2688-2728）：Delivered→commit_verified_delivery；NotDelivered→schedule_retry_or_terminal；Inconclusive/核对 Err→mark_delivery_unknown_if_owned（"automatic replay disabled"）。

#### contact 状态门 `check_contact_status_pure`（:2737-2754）
豁免：`manual_send / principal_escalation / principal_clarification / system_incident`（manual_send=admin 已显式确认发送意图，P1-6 :2730-2735）。其余：`Managed`→放行；否则→`"contact_status_changed_unmanaged"`（撤管即停）。

#### `process_entry` 全流程（:2760-3079）——单条已抢占 entry
按序 15 步（任一步终结即 return）：
1. 请示卡代数授权（:2769-2778）失败→cancel `principal_escalation_generation_no_longer_authorized`。
2. 系统事故通知授权 `system_incident::send_is_authorized`（system_incident.rs:587）失败→cancel（:2779-2788）。
3. SR-034 第一检查点 `enforce_task_send_authorization`（:2790-2797）：Building→defer、Stale→cancel。
4. 二次安全门（:2799-2802）命中→cancel(reason)。
5. 加载 contact（:2804-2815）；**contact 不存在且非内部三类** → `schedule_retry_or_terminal("contact not found at dispatch time")`（:2816-2827，消耗 attempt）。
6. contact 状态门（:2829-2834）命中→cancel。
7. 账号在线门（:2836-2854）：`accounts.online==false` → `defer_account_offline`；account 行不存在 → 保守放行。
8. **崩溃恢复幂等门**（:2856-2901）：`reclaimed_in_flight=true` → `verify_delivery`：Delivered→`commit_verified_delivery`（note "reclaimed after crash but delivery was confirmed post-hoc"）后 return；NotDelivered→继续本次发送；Inconclusive/Err→`mark_delivery_unknown_if_owned` 后 return。**绝不把"无本地成功日志"当"确认未送达"**（:2857-2858）。
9. **账号级 pacing 闸**（:2903-2920）：位置在 reclaim 门之后（不误拦本该 post-hoc 标 sent 的条目）、发送之前；查询失败 fail-soft 放行（宁可漏限一次不丢消息）。`interval = pacing::account_send_interval_ms(fastrand, config.account_send_min/max_interval_ms)`（默认 1000/4000ms，config.rs:482-483）；`now-last_sent < interval` → `defer_account_pacing(last_sent+interval)`。
10. SR-034 第二检查点（:2928-2934）：覆盖第一检查后到远端边界前的 task 状态变化。
11. 请示卡 + 系统事故授权二次复查（:2936-2955）。
12. `begin_remote_send`（:2960-2962）——最后可取消点 CAS；失败即 return。
13. **MCP 分流 send_fut**（:2964-3021）：
    - 内部三类（principal_escalation/clarification/system_incident）：`mcp::logged_send_call_for_account("message_send_text", {recipient, content})` + `gateway::classify_send_receipt`：Succeeded→Ok、ExplicitlyFailed→`SafeToRetry`、Inconclusive→`DeliveryUncertain`（:2965-2997）；
    - 名片：`referral::send_outbound_namecard`（:2998-3004）；
    - 媒体：`media_send::send_outbound_media`（:3005-3011）；
    - 文本：`gateway::send_outbound_message`（:3012-3020，extra_raw 带 outbox_id/run_id/attempt）。
14. 外层 `tokio::time::timeout(150s)`（:3022-3023）。
15. 结果矩阵（:3025-3077）：
    - `Ok(Ok)` → `commit_sent_if_owned`（失败=claim 易主→**抑制重复副作用**只 warn）→事件 `outbox_sent`→refresh→`finalize_delivered_text_decision`+`send_ledger::record_send_for_entry`（素材/名片记台账，fail-soft）；
    - `Ok(Err(SafeToRetry(reason)))` → `schedule_retry_or_terminal`；
    - `Ok(Err(DeliveryUncertain(reason)))` → `settle_ambiguous_send`；
    - `Err(timeout)` → `settle_ambiguous_send("send timed out after the remote boundary")`。

`OutboundSendError`（types.rs:24-29）两值：`SafeToRetry`（可证明未进入不可逆投递）/ `DeliveryUncertain`。`From<AppError>`/`From<mongodb::Error>` 均映射 SafeToRetry（types.rs:41-51，前置条件失败未跨界）；`McpSendError` 保留原分类（types.rs:53-58）。

#### 事件 cap（P2-3，:3081-3191）
`decide_cap_action(count, sentinel_already)`（:3095-3103）：`count<20`→WriteNormal；已写 sentinel→Silent；否则→WriteSentinel。`write_event_with_cap`（:3109-3191）：按 `details.outbox_id` 计数；达 cap 首次补写一条 `outbox.event_cap_reached`（details.kind="event_cap_reached"、cap、observed_count、suppressed_kind/status）后永久静音。

#### worker 循环（:3193-3284）
- 进程内快速唤醒：`OUTBOX_WORK_NOTIFY`（LazyLock<Notify>，:3201）；`notify_outbox_work`（:3203-3205，enqueue 后调 outbox.rs:389、tasks.rs:532）；`notify_outbox_work_after(delay)`（:3211-3216，spawn 定时唤醒，tasks.rs:533 用 1050ms 接住 task 授权 Building 的 1s defer）；`wait_for_outbox_work`（:3218-3223）=notified 或 5s fallback（Notify 在无等待者时保留一个 permit，扫描空档入队不丢，:3251-3252）。
- `run_outbox_dispatcher`（:3236-3259）：main.rs:242 spawn；loop { tick; wait }。
- `tick`（:3262-3284）：`reclaim_expired_leases` → `reconcile_stale_outbox_enqueues` → `reconcile_delivered_decision_finalizations` → `webhooks::reconcile_manual_reply_obligations`（:3268，manual 发送在本 worker 外被取消/终态化时释放暂停；delivery_unknown 刻意保持暂停）→ 至多 16 次 `claim(每第 10 次 FIFO)+process_entry`（错误只 log 不中断 tick）。

测试区（:3286-3771）：Notify 唤醒/兜底、sent unset 字段、task 授权分类矩阵、边界 filter 全 fence、聚合序无关、stale-enqueue 三分、finalize 分诊、cap=20、contact 门、pacing/delivery filter 租户 scope、worker_id、30min 阈值、timeout-lease 时序不变量（:3645-3664）、cap 决策、旧文档 reclaimed_in_flight 默认 false、max_attempts 口径、principal-card 身份严格解析（:3757-3771）。

### 2.3 `src/agent/escalation/mod.rs`（661 行）

模块 doc（:1-5）：决策请示通道——Agent 撞决策墙向幕后真人请示，拿裁决后 AI 口吻转述；客户永远只跟 Agent 对话。子模块（:7-11）与 re-export（:13-20，`fallback_holding_reply` 单独 pub 还原可见性供 crate 外红线测试）。

- `reconcile_pending_relay_intents` / `reconcile_principal_card_deliveries`（:25-31）：包装 ledger 的两个恢复扫描，task worker 每 tick 调（tasks.rs:1124-1125）。
- `enqueue_holding_reply`（:46-73）：安抚话术入 outbox（`source_kind=follow_up_task`，max_attempts=3）。
- `enqueue_expired_relay_holding_reply`（:77-186）：**过期授权安抚走完整 fenced task 协议**——新建 `AgentDecisionReview`（approved=true、status=`outbox_enqueuing`、expected_text_segments=1、`send_gateway_result.deliveryKind="expired_principal_authorization_holding"`、run_id=`holding-expired-{task_id}`，:85-136）→ `tasks::bind_task_decision_if_owned`（tasks.rs:289）失败→review 置 `stale_task_claim` 返 false（:138-149）→ 入队（source_event_id=`principal-expired:{short_code}:{claim_token}`，:151-161）→ `tasks::authorize_task_outbox_if_owned`（tasks.rs:468）失败→review 置 stale（:163-174）→ review `outbox_enqueuing→outbox_enqueued`（:175-183）。
- `escalate_held_decision`（:196-284）——**hold→升级请示**（与 approved 路的 `trigger_principal_escalation`（gateway.rs:1930）区别：本函数只推领导卡+落台账+写 awaiting 标记，**不向客户发任何消息**；客户侧安抚由网关 `ensure_customer_acknowledged` 统一负责，:188-193）。流程：无 domain_config→跳过；`resolve_ask_human_policy`+`should_escalate_held(blocked_status, policy)` 不升级→跳过；`freeze_ask_human_policy`（补 account_id）；链空=未启用→跳过（:212-213）；decider 缺 account_id→`BadRequest("决策人缺少发送账号")`（:216-219）；**principal==客户 wxid → 拒绝**（:221-225）；骚扰门（`count_pushes_today`(24h 窗)+`latest_push_ms`+`push_allowed`，:226-233，不过门→直接 return，见 §5 疑点 4）；`has_pending_for_contact(HIGH_RISK_GATED)` 去重（:234-245）；reason=hold_reason 或 review_summary；question 用 `labels::blocked_status_zh`/`risk_level_zh` 拼中文（:251-255）；customer_label=remark>nickname>alias>wxid（:256-261）；`insert_pending_escalation`（返 None=并发已插同类 pending→不重复推卡，:262-281）；`materialize_principal_card_delivery`（:282）。调用点 gateway.rs:3762，错误只记 warn 不阻断 run（:195）。
- `handle_principal_decision_relay_with_claim`（:288-413）——**relay task 处理**（tasks/gateway.rs:185 调）：claim 现势校验（:293-297，`tasks::task_claim_is_current`）；按 short_code（=task.content）查台账；无 entry/无 decision→静默 Ok；**授权过期分支**（:311-378）：`relay_substance_if_usable`=None → `terminalize_principal_relay(entry,"authorization_expired")`（false=另一终态路径已占有结果→不再发第二条矛盾话术，:317-321）→ 查 contact → `generate_holding_reply(ExpiredAuthorization)` → 有 claim 走 `enqueue_expired_relay_holding_reply`（fenced 协议）、无 claim（测试/工具直调兼容）走裸 `enqueue_holding_reply`（:346-375）。语义（:313-316 注释）：不拿过期授权乱承诺，但必须清 awaiting 标记（否则永久压制自主回复）+发中性收尾（否则客户被晾死）。**有效授权分支**（:380-412）：查 contact→claim 再校验→`gateway::relay_principal_decision_to_customer(contact, entry, decision, TaskRunContext)`。
- `interpret_principal_reply`（:417-457）：prompt `escalation.principal.interpret` + `generate_agent_json`；JSON 反序列化失败→回落 `deferred` 空 decision（保守：宁当"领导还没定"也不乱转述，:444-455）；成功→`sanitize_verdict`（verdict 越闭集也回落 deferred）。注意 LLM 调用本身 Err 会向上传播（:443 `?`）。
- `handle_principal_reply`（:462-554）——**领导微信回复消费**（webhooks.rs:1412 调；返回 true=已消费，不进客户 agent 链路）：`list_pending_for_principal`+`match_principal_reply`：
  - `NoPending`（:472-478）：领导主动消息不自动生效（待 admin 确认），Ok(true)；
  - `Ambiguous(codes)`（:479-524）：**反问澄清也走 durable outbox**（:487-491 注释：内部澄清也是真实出站副作用，须享受重试/取消/pacing/回执分类/delivery_unknown 全套）——`source_kind=principal_clarification`，`source_event_id=principal-clarification:{principal_wxid}:{排序去重codes join "-"}`（幂等：同一批未决集合只问一次），run_id=同 source_event_id；
  - `Matched(code)`（:525-552）：`interpret_principal_reply`→verdict=deferred→保持 pending 继续等（:532-535）；`authorization_window_hours>0` 才换算 expires（领导没提=None=不设过期窗，**不再硬编码默认窗**，:536-546）→`resolve_escalation(..., "wechat")`（None=并发已 resolve，幂等）。
- `scan_escalation_timeouts`（:559-661）——**超时改派/链尾安抚**（tasks.rs:1138 每 tick）：只扫 `list_timeout_eligible_escalations`（新协议+当前卡已确认送达+冻结 policy 有 timeout）；用**创建时冻结**的 policy 快照（`resolve_ask_human_policy_snapshot`，后续改配置不影响在途请示，:556-558）；`age_hours=(now-last_pushed_at_ms)/3600s`；`next_decider_on_timeout`：
  - **None**（未超时/真链尾/无合法下一位）：若确实超时（age≥timeout）→ **链尾安抚**（:576-627）：`holding_reply_min_interval_hours`（默认 6.0h，config.rs:648）去重（last_holding_reply_ms 检查+`window=now.div_euclid(window_ms)` 分桶幂等键 `principal-chain-tail:{code}:{window}`）→`generate_holding_reply(ChainTail)`→enqueue→`touch_last_holding_reply_ms`（失败仅 warn，幂等键仍兜底）；
  - **Some(next)**（:629-658）：next 缺 account_id→error 日志+跳过（拒绝改派）；对 next 过骚扰门（count/latest/push_allowed）；`reassign_escalation`（单文档 CAS 开下一 delivery generation）→`materialize_principal_card_delivery`（入队确认失败保留 pending_enqueue，下轮 reconciler 按代数幂等补偿，:656）。**本函数不直接跨 MCP 远端边界**（:558）。

### 2.4 `src/agent/escalation/policy.rs`（614 行 = 193 生产 + 421 测试）

- `ResolvedAskHumanPolicy`（:8-18）：decider_chain + 4 个逐类别升级开关（safety_guard / unverified_product / ai_policy_hold / stuck）+ 骚扰门三参数（dedupe_window_hours / daily_push_cap / quiet_hours）+ timeout_hours。全部 Option 字段 None=不启用该门。
- `resolve_ask_human_policy`（:21-58）：`ask_human_policy` 存在则逐字段复制；否则**旧字段回落**（字节等价红线④）：`all_mode = high_risk_escalation_mode=="all"`；链=principal_decider 单人（display_name/account_id=None）或空；默认 `escalate_safety_guard=true, unverified=true, ai_policy_hold=all_mode, stuck=true`，其余 None。
- `resolve_ask_human_policy_snapshot`（:63-75）：从**冻结在台账上的** AskHumanPolicy 解析，绝不回看 config——后续配置编辑不影响在途请示。
- `freeze_ask_human_policy`（:80-109）：冻结时给缺 account_id 的旧配置 decider 绑定触发客户所属账号（新 admin 写入拒绝缺 account；此回落仅为 pre-account-binding 兼容）。
- `is_decider_for_config`（:115-120，cfg(test)）：KD-04 谓词（生产版在 ledger.rs:854 `lookup_principal_config`）。
- `in_quiet_hours`（:123-132）：`hour = ((now_ms + tz*3600s) / 3600s) % 24`（手工取模含负数修正），`[start,end)` 支持跨午夜（start>end）。**注意与 `agent/quiet_hours.rs` 是两套独立实现**：本处无 `start==end` 退化分支（start==end 时恒 false 由 `h>=s && h<e` 自然给出——`s==e` 时 `s<=h<e` 恒假、跨午夜分支不会进入，行为等价但代码路径不同）。
- `push_allowed`（:136-159）——**推卡骚扰门**三条 AND：daily_push_cap（today_count≥cap→拦）；dedupe_window_hours（距上次推卡不足窗→拦）；quiet_hours（窗内→拦）。全 None→true（字节等价全放行）。
- `next_decider_on_timeout`（:166-193）——**超时改派选人**：`timeout_hours=None`→None（无限等待）；`age<timeout`→None；起点 = current 在链中→idx+1，**不在链（KD-06 admin 改链孤儿）→0 回落链首重新入链**；从起点起 `find(|d| d.wxid != contact_wxid)`——**KD-07 跳过被误配成客户 wxid 的链成员**，防内部请示卡直推客户；真链尾（idx+1 越界空切片）/剩余全是客户/空链→None→scan 走链尾安抚。测试覆盖：超时转下位（:422-446）、无 timeout 永不转（:449-465）、孤儿回落链首（:468-491）、真链尾仍 None（:494-515）、空链 None（:518-527）、跳过客户成员（:530-558）、跳过后无人→None（:561-582）、quiet hours 跨午夜+时区（:585-613）。

### 2.5 `src/agent/escalation/labels.rs`（70 行）

内部状态码→领导可读中文（嵌在请示卡自然语言里，前端无法字典翻译，后端拼串前转换；未知值回落原字面量不吞信息，:1-3）。`blocked_status_zh`（:7-15）：`blocked_unverified_product_claim`→"产品说法未经核实"、`blocked_by_safety_guard`→"安全门拦截"、`held_by_ai_policy`→"AI 策略主动暂缓"、`ai_waiting_for_more_context`→"AI 等待更多上下文"。`risk_level_zh`（:18-25）：low/medium/high→低/中/高。

### 2.6 `src/agent/escalation/ledger.rs`（1279 行 = 1221 生产 + 58 测试）

台账 CRUD 层，全 async+DB。

- `insert_pending_escalation`（:26-122）：category 必须在闭集（debug_assert :41-44）；至多 5 次尝试，`seed = now_ms.wrapping_add(attempt × 2_654_435_761)`（Knuth 乘散列常数）→`short_code_from_seed`；`render_principal_card` 预渲染 `delivery_content` 冻结在 protocol 上；台账初始：status=pending、protocol{policy_version=config.version、policy 冻结、principal_account_id、delivery_generation=**1**、delivery_state=**pending_enqueue**、delivery_outbox_id=None}、`last_pushed_at_ms=None`（**只有 Outbox 确认送达才写**，:83-84）。insert 错误二分（:106-116）：`is_pending_dedupe_conflict`（同客户同类别 pending 唯一索引）→Ok(None) 静默"已存在"；`is_duplicate_key_error`（短码撞）→换种子重试；其它→Err。5 次耗尽→`AppError::External`（:119-121）。
- `materialize_principal_card_delivery`（:127-187）——**冻结意图→outbox 物化**：前置 `status=pending && delivery_state=pending_enqueue`（否则幂等 no-op）；`activate_awaiting_principal_owner`（先立客户 awaiting 标记）；`source_event_id = principal-card:{escalation_id}:{generation}`（**每代确定性**：outbox 插入后、ack 更新前崩溃会收敛到同一行，:124-126）；enqueue（source_kind=principal_escalation、收件人=principal_wxid、账号=protocol.principal_account_id、内容=冻结 delivery_content）；outbox_id 取 Created 或 IdempotentSkip 的 existing；CAS `pending_enqueue→queued + delivery_outbox_id`（:169-185，filter 锁代数）。
- `reconcile_principal_card_deliveries_once`（:191-291）——**outbox 事实回灌台账**（limit 100）：扫 `pending+state∈{pending_enqueue,queued}` ∪ `delivery_failed 且未 cleanup`；pending 行先补 awaiting owner（:218-220）；delivery_failed→`complete_failed_delivery_cleanup`；pending_enqueue→重新物化（:225-229）；queued→读 outbox 状态映射（:241-258）：`sent`→(SENT, **last_pushed_at_ms=送达时刻**, 仍 pending)；`failed_terminal|canceled`→(FAILED_TERMINAL, 台账状态→**delivery_failed**)；`delivery_unknown`→(UNKNOWN, 仍 pending)；其它（pending/in_flight）→continue。CAS 锁 `pending+generation+queued+outbox_id`（:268-282）；转 delivery_failed 成功即刻做失败清理（:284-288）。
- awaiting 标记管道（:293-424）：`domain_attributes.awaiting_principal_decision`（粗布尔）+ `awaiting_principal_decision_ids`（每请示 id 的 owner 数组，models.rs:4545-4549）。`activate_awaiting_owner_pipeline`（:304-331）：`$setUnion(owners, [id])` + awaiting=true（防御 domain_attributes 非 object/owners 非数组）；`remove_awaiting_owner_pipeline`（:333-371）：`$filter` 掉本 id，awaiting=`size(remaining)>0`——**并发创建/终结不会互踩对方标记**。`activate_awaiting_principal_owner`（:373-400）matched≠1→`Conflict("principal_escalation_contact_missing")`；remove（:402-424）无此校验（容忍 contact 已删）。
- `terminalize_principal_relay`（:430-490）：CAS `resolved + relay_state∈{pending,enqueued} 或缺失（仅显式 task-bound 路径接受，滚动升级兼容）→ terminal + relay_terminal_at + relay_terminal_reason`；抢不到→必须已是 terminal（否则 `Conflict("principal_relay_terminal_state_changed")`）；然后 `remove_awaiting_principal_owner`；返回 `terminal_reason == 本次 reason`（false=别的终态先到，调用方据此不再发第二条收尾话术，mod.rs:317-321）。
- `terminalize_principal_relay_for_task`（:495-527）：按 task 身份定位（ws/account/contact/short_code/resolved + `$or[_id==task_id | relay_task_id==task_id | 两字段都缺(legacy)]`）→ 委托上函数。dispatcher finalize 用它标 "delivered"（outbox_dispatcher.rs:1626-1629）。
- `complete_failed_delivery_cleanup`（:531-552）：先 remove awaiting owner，后 CAS 写 `failure_cleanup_completed_at`（**清理确认最后写**，中断可重试，:529-530）。
- `list_pending_for_principal`（:555-582）：`pending + principal_wxid + protocol.principal_account_id=收信账号 + delivery_state∈{sent, delivery_unknown}` 升序——**卡送达状态 unknown 时领导回复仍可被消费**。
- `has_pending_for_contact`（:585-607）：同客户同类别 pending 计数去重。
- `resolve_escalation`（:610-668）：$set `resolved + decision + resolved_via + relay_state=pending + relay_task_id=escalation_id`（+authorization_expires_at 若有）；CAS filter：`_id+ws+short_code+status=pending+principal_wxid`，有 protocol 再锁 `delivery_generation`，`resolved_via=="wechat"` 还要求 `delivery_state∈{sent,unknown}`（微信回复只能裁决已送达代的卡；admin 后台 resolve 不受限，:641-652）；成功即 `materialize_relay_task`（:664-666）。
- `emit_knowledge_gap_proposal`（:674-757）——**知识沉淀**：title=`derive_sediment_title`（LLM 提炼，失败回落首句 40 字符兜底）加"待审核："前缀；body=客户/短码/裁决/约束；chunk `status=draft + integrity_status=needs_review + account_id=None`（workspace 共享域）；**chunk 插入与 create revision 在 Mongo 事务里原子落地**（:716-756），revision 来源 `ProvenanceSource::PrincipalAuthorized`；**绝不自动 verify**（AI 永不自动验证红线，:670-671, :714-715）。
- `derive_sediment_title_fallback`（:766-788）：首句截断（。！？!?\n）+40 chars 限长+省略号；空→"领导授权沉淀"。`derive_sediment_title`（:795-844）：prompt `escalation.sediment.title`，任何失败/空回落兜底；LLM 超长同款 40 chars 收口。
- `lookup_principal_config`（:854-908）——**入站是否领导**（KD-04 生产版）：🔒 必须用入站消息自己的 workspace 约束（否则 A 工作区领导恰是 B 工作区业务号好友时跨域串扰，:852-853）。两级：先查该 wxid 有无 `pending+delivery_state∈{sent,unknown}+protocol.principal_account_id=account` 的台账（:861-880，返 protocol.domain）；否则遍历 current_version 域配置，resolve 后链成员匹配（decider.account_id=None 或 ==account，:881-907）。
- `materialize_relay_task`（:913-1016）：前置 `resolved+relay_state=pending`（否则 `Conflict("principal_relay_intent_not_pending")`）；**task_id 必须 == escalation_id**（:930-934，新协议确定性身份，崩溃/并发 reconciler 重试 upsert 不会造出第二个 relay）；upsert `$setOnInsert`（task_doc 移除 _id，filter 携带 _id=task_id+身份五元组）task{kind=principal_decision_relay, content=short_code, status=pending, review_required=false, max_attempts=3}；CAS `relay_state pending→enqueued + relay_enqueued_at`；0 改动→验证已是 enqueued+同 task 否则 `Conflict("principal_relay_intent_changed")`（:996-1014）。
- `reconcile_pending_relay_intents_once`（:1021-1052）：扫 `resolved+relay_state=pending`（limit 100，resolved_at 升序）逐条 materialize（失败 warn 继续）；**legacy resolved 行（无 relay_state）刻意忽略**（:1018-1020，后台恢复绝不猜测/重放旧行）。
- `list_escalations_by_workspace`（:1055-1072）：admin 收件箱/SLA 看板。
- `reassign_escalation`（:1077-1122）——**改派开新代**：CAS `ws+short_code+pending+principal_wxid=期望+generation=期望+delivery_state∈{sent,failed_terminal,unknown}`（**上一代必须已终**，无仍可跑的卡能与改派竞速，:1074-1076）→ 新 principal/account、`delivery_state=pending_enqueue`、`$inc generation`、`$unset delivery_outbox_id + last_pushed_at_ms`。
- `list_timeout_eligible_escalations`（:1126-1147）：`pending + delivery_state=sent + protocol.policy.timeoutHours 为数字 + last_pushed_at_ms 为数字`（limit 500，last_pushed_at_ms 升序）；legacy 行刻意不猜。
- `touch_last_holding_reply_ms`（:1150-1170）：仅 pending 可更新。
- `count_pushes_today`（:1174-1194）/ `latest_push_ms`（:1199-1220）：骚扰门数据源，均以 `last_pushed_at_ms`（首推+改派后由 outbox 确认刷新）为口径（KD-05：不用 created_at，改派不刷 created_at 会漏计）。

### 2.7 `src/agent/escalation/logic.rs`（1247 行 = 452 生产 + 795 测试）

纯函数层（无 IO，:1-2）。

- 短码：字符集 base32 去易混（无 0/O/1/I/L，31 字符，:12）+4 位 body；`short_code_from_seed`（:17-27）：`E` 前缀 + 逐位 `seed % 31`。确定性（测试 :479-481）。
- `ReplyMatch`（:31-38）：Matched(code) / Ambiguous(codes) / NoPending。
- `extract_short_code`（:42-51）：大小写不敏感 contains（允许带/不带 #），返回规范化码。
- `match_principal_reply`（:54-69）：无 pending→NoPending；带码→Matched；不带码但仅一条未决→Matched（回落）；多条→Ambiguous。
- `render_principal_card`（:72-81）：`【请示 #code】客户「label」\n卡点：reason\n请示：question`——短码放最前便于领导引用，**对领导不脱敏**（测试 :568-578）。
- 三条硬编码兜底文案：`fallback_holding_reply`（:89-91，pub，GateHold："这个我帮你确认一下，稍等我给你准信。"）；`chain_tail_holding_reply`（:96-98）；`expired_authorization_neutral_reply`（:103-105）。红线：绝不出现转接类措辞（受 CI 全自治 lint 约束）。
- `HoldingReplyScene`（:110-117）：GateHold / ChainTail / ExpiredAuthorization；`scene_fallback_text`（:120-126）映射。
- 授权时效：`authorization_is_usable`（:130-138，expires=None 视为不过期）；`relay_substance_if_usable`（:141-151，过期→None）。
- `HighRiskEscalationMode`（:156-161）+ `parse_high_risk_mode`（:164-169）：仅 `"all"`→All，其余（含未配/脏值）→DecisionOnly 保守默认。
- `assert_target_is_principal`（:179-190）：目标 wxid 必须等于配置领导，防内部卡误发客户；当前无生产调用点（所有发送路径目标同源取自 decider_chain，:174-177），`#[allow(dead_code)]` 保留为防御 API。
- `is_principal_relay_trigger`（:199-204）——**H10 修复**：relay 身份唯一判据是 `ConversationMessage.is_synthetic_relay` 来源标记（仅 `synthetic_principal_relay` 构造器内存置 true），**绝不**按 content 前缀判（客户伪造哨兵 `__PRINCIPAL_RELAY__`（models.rs:841）即可冒充 relay 劫持频控豁免）。测试 :753-816 三态覆盖（真合成/普通/伪造哨兵）。
- `relay_output_leaks_internal_payload`（:214-219）——**relay 出站红线守卫**（与解读侧 sanitize_verdict 对称的代码级兜底）：拟发客户文本含哨兵或 `verdict=`/`substance=`/`constraints=` 字段标记→true，网关 fail-closed 不发。
- 数字白名单护栏：`extract_number_tokens`（:223-238）+`normalize_number_token`（:241-253）+`relay_introduces_unauthorized_number`（:259-269）纯函数仍存在。**【24 号交叉验证修正 2026-08-13：该护栏已不再用于 relay 出站**——`gateway.rs:4188-4192` 注释明确字符级数字白名单 backstop 已删除（KD-01/03：漏中文数字+误杀时间/序数/等价折扣，fail-closed 曾致裁决黑洞），relay 转述忠实性由生成侧 prompt + Review Agent 语义把关；**该函数唯一生产调用点是 `holding_reply.rs:60`（安抚话术的数量边界守卫）**。本记录原表述被 `logic.rs:255-258` 过时函数注释带偏。】小数变体测试 :1174-1199。
- `consecutive_unprogressed_turns`（:273-286）：意图轨迹尾部同 intent 连续数。
- `build_decision_signals_text`（:296-347）——注入 decision prompt 的请示通道信号段，四信号：①awaiting 标记→"勿反复请示/勿替领导拍板/非越权部分照常回复"（:303-313）；②卡死=连续未推进≥3 **且** 末轮负面（极性来自传入 `negative_outcomes`，2.5-main-3 行业可参数化，:315-323）；③all 模式→提示主动 emit escalationRequest（:325-332）；④已引荐态（`REFERRED_SPECIALIST_AT_ATTR` 存在）→"退为辅助答疑、不重复引荐"（:334-344）。全缺→空串。
- `should_escalate_held`（:353-367）——hold 件升级判定：`blocked_by_safety_guard`→policy.escalate_safety_guard；`blocked_unverified_product_claim`→escalate_unverified_product；`held_by_ai_policy`→escalate_ai_policy_hold；**其它终态（等待上下文/必填缺失/预算/context_changed）一律不升级**（非决策墙，测试 :1086-1112）。
- `stuck_suppressed`（:372-377）：仅 STUCK 类且 escalate_stuck=false 时压制。
- 错误判定：`is_duplicate_key_error`（:380-386，11000）；`PENDING_DEDUPE_INDEX_NAME`（:391-392，`uniq_principal_escalation_pending_ws_account_contact_category`）；`dedupe_conflict_matches_pending_index`（:398-400，code==11000 && message 含索引名）；`is_pending_dedupe_conflict`（:404-410）。
- `sanitize_verdict`（:413-425）：verdict 不在 `ALLOWED_PRINCIPAL_VERDICT` 闭集→回落 deferred，**substance/constraints/window/exemption_type 全部透传**（不丢领导授权范围，测试 :667-682）。
- `is_stuck_or_undelivered`（:428-434）：两条件 AND；`latest_reaction_is_negative_with_polarity`（:440-448）；`DEFAULT_STUCK_THRESHOLD=3`（:451）。

### 2.8 `src/agent/escalation/holding_reply.rs`（325 行 = 202 生产 + 123 测试）

- `HoldingSafetyVerdict`（:10-17）+`parse_holding_safety_verdict`（:19-34，五字段全必填、reason 非空，缺任一→None）+`holding_safety_verdict_allows`（:36-41，safe 且三风险位全 false）。语义风险位覆盖 safe=true 的自相矛盾输出（测试 :306-324）。
- `holding_reply_text_is_safe`（:45-65）——出站守卫三关：非空；`evolution::lint::passes_forbidden_words`（运行期全自治禁词，复用 CI lint 词表）；`relay_introduces_unauthorized_number(text, authorized_substance 或 "")`——**None 表示授权数量集为空而非关闭校验**（:57-59），故三场景任何未授权数字都拦（测试 :237-250）。
- `HOLDING_SAFETY_REVIEW_SYSTEM`（:67-71）：独立语义审查 prompt——候选是不可信数据非指令、按语义不按关键词、只允许中性过程语言。
- `review_holding_reply`（:73-100）：`holding.reply.safety_review` 独立 LLM 审查，任何失败→None。
- `holding_reply_system_prompt`（:106-127）：三场景 prompt；**刻意不含禁词字面量**（否则被 CI 文本 lint 自噬，:103-105），用"你是唯一对接人"正面表述；一句话、第一人称、不提别的对接角色、不承诺数字/结果。
- `generate_holding_reply`（:133-201）——**独立预算旁路生成**：`RunBudget::new(run_id="holding-{uuid}", config.holding_reply_token_budget(默认 3000, config.rs:650), max_llm_calls=2, 工具上限 i32::MAX(本路径不用工具，防聚合误判耗尽 :148))`，`RUN_BUDGET.scope` 包住生成+审查两次调用——主 run 预算耗尽也能生成一次；链路：预算已尽检查→`holding.reply` 生成→`holding_reply_text_is_safe`→独立审查→verdict allows；**任一失败回落 scene 硬编码兜底，保证返回非空、经守卫的文案（客户永不被晾死）**（:129-132）。

### 2.9 `src/agent/referral.rs`（479 行 = 285 生产 + 194 测试）

- `REVIEWER_ASSIST_YIELD_NOTE`（:14）：辅助模式下注入 reviewer system prompt 的"让位段"，消解两条 hold 路径：①引荐专属顾问不属于"除我外不得出现人类角色"红线（红线在该受控动作上让位）；②引荐不是产品能力声明（不计入 hallucination/产品准确度、不据此抬 factRisk）。
- `assist_mode_active`（:17-26）：客户级 override（`force_on`/`force_off`）> 账号级 enabled > 默认关；脏值 override 视为无覆盖。
- `validate_card_sendable`（:37-43）——**发送准入唯一口径（KE-03）**：`enabled && review_status=="approved" && account 归属`（card.account_id=None 全局可用；Some(bound) 仅 bound==account）。三条路径（候选加载/gateway 二次准入/send 前二次校验）全走本函数，与 DB filter `$or:[{account_id:null},{account_id:==}]` 口径一致（:29-36）。
- `filter_referral_candidates`（:45-60）：sendable + target_stages 空=总命中/非空须含当前 stage（stage 为 None 且 stages 非空→不命中）。
- `AlreadyReferred`（:63-66）；`render_referral_overview`（:73-88）：Lean 档概览**只露 display_name+引荐时机 hint，绝不露 card id/收件人元数据**——概览不能授权发送，只有 Full prompt 拿 id 且 Gateway 物化前再校验（:68-72）。
- `explicitly_requests_referral_context`（:95-125）：客户显式求引荐的窄确定性信号（仅允许 Gateway 加载 Full 候选，不决定发卡）：归一化去空白/标点；引用/示例标记（如果/比如/例如/假设/示例/文案/怎么回复）→false；7 个请求词 × 7 个否定前缀（"不用/不要/不需要/无需/先别/暂不/别再"紧邻前缀检查）。
- `render_referral_lines`（:127-158）：Full 档候选清单 `[card:{id}] 名字 | 阶段 | 标签(非空才渲染) | 触发提示` + 引荐历史行（已引荐→"除非全新需求场景否则不重复引荐"）。
- `build_referred_set_doc`（:161-168）：dotted-key `$set`（`domain_attributes.referred_specialist_at`=now、`.referred_card_id`=card_id，不覆盖其它 domain_attributes）。
- `send_outbound_namecard`（:174-284）——dispatcher 名片分支（调用方已保证 outbox 幂等）：
  1. parse card_id（坏 id→`AppError::External`→SafeToRetry）；`_id+workspace_id` 双条件查卡（**防跨租户 IDOR**，与 media 对齐，:180）；
  2. 发送前 `validate_card_sendable` 二次校验（防 AI 幻觉/已撤下的卡漏到发送，:191-197）；
  3. MCP `message_send_namecard {recipient, targetWxid}`（:199-208，字段名待 server tools/list 确认的占位注记）；
  4. `classify_send_receipt`：ExplicitlyFailed→SafeToRetry、Inconclusive→DeliveryUncertain（:210-222）；
  5. **MCP 成功=名片已送达=既成事实：此后落库/置态失败绝不返 Err**（否则 dispatcher retry→客户收重复名片，:224-225）：出站 `ConversationMessage{msg_type="namecard", media_ref=card_id, content=display_name, raw 带 referralCardId}` fail-soft（:233-262）；`build_referred_set_doc` 置已引荐态 fail-soft（:264-281）。

### 2.10 `src/agent/media_send.rs`（625 行 = 411 生产 + 214 测试）

- `mcp_tool_for_media_type`（:12-19）：image/file/video → message_send_image/file/video；未知→None（链接卡片/小程序留位）。
- `validate_asset_sendable`（:23-31）：`sendable==Some(true) && review_status==Some("approved") && media_type 可映射`——老朋友圈行（media_type=None）与草稿绝不放行。
- `filter_sendable_candidates`（:35-49）：准入硬门 + target_stages（None/空=总命中；非空必须含 stage；stage=None 且 stages 非空→不命中）。
- `render_candidate_lines`（:52-78）：Full 档 `[id:{assetId}] 标题 | 阶段 | 表达偏好 | 标签 | 触发提示`。
- `render_candidate_overview`（:88-112）：Lean 档只露标题+发送时机；**末尾附显式升档引导**（③升档盲区修复 2026-06-27：A/B 证明纯客观罗列不足以驱动升档——补一句"契合即判 sufficiency=need_more_context、missingTier=full"，只描述语义条件不列关键词，:102-110）。
- `media_id_cache_valid`（:117-120）：`0 ≤ now-updated_at < ttl_hours`；**未来时间戳（时钟回拨/脏数据）→无效强制重传**。
- `MEDIA_SEND_TOOLS`（:123-127）：三工具集合，崩溃恢复核对圈定用。
- `media_asset_available_to_account`（:132-137）：asset.account_id=None 共享/Some 须匹配。
- `ensure_media_uploaded`（:139-209）：账号归属检查；**缓存仅账号私有 asset 可复用**（`cache_is_account_bound = asset.account_id==Some(account)`，:147-156——mediaId 属于上传它的 MCP 账号，共享 asset 的标量缓存无法辨归属账号，绝不复用）+TTL（`media_id_cache_ttl_hours` 默认 24h，config.rs:811）；未命中→读盘（`media_storage::read_bytes_recovering`）→base64→MCP `media_upload_base64 {fileName, mediaType, base64}`→取 mediaId；**回写缓存失败→传播错误中止发送**（:187-207）——崩溃恢复依赖不变式"`asset.media_id==None ⇒ 从未发出 ⇒ 可放行重发`"，若吞掉回写失败会出现"已发未存"→recovery 误判没发过→重发重复文件。宁可这次不发（下次重传+回写）。
- `send_outbound_media`（:214-309）——dispatcher 媒体分支：parse asset_id；`_id+workspace_id` 查（防 IDOR，:222-230）；`validate_asset_sendable` 二次校验；映射工具名；账号归属；`ensure_media_uploaded`；MCP `{recipient, mediaId}`；`classify_send_receipt` 三分同名片；**MCP 成功后落库失败绝不返 Err**（防 retry 重发，:269-270）：出站 `ConversationMessage{msg_type="media", media_ref=asset_id, content=title, raw 带 mediaAssetId}` fail-soft。
- `mcp_media_delivery_verification`（:319-365）——**媒体崩溃恢复核对（硬伤④）**：按 media_id（非 content）定位。证据链（:311-318）：发送必先 `ensure_media_uploaded` 且它在 send 前回写 media_id、send 请求体带 `request.mediaId`，故：坏 asset_id→NotDelivered；asset 无/media_id=None→**NotDelivered（客户投递不可能已发生，安全放行重发）**；`media_success_filter`（:367-390，`tool∈三工具+recipient+mediaId+error null+(ok 或 newMsgId)+created_at≥entry-5min`）命中→Delivered；未命中→**Inconclusive**（本地日志非权威，禁把缺证据当未送达）。
- `media_delivery_verification`（:393-410）：公开包装，dispatcher 崩溃恢复/timeout/歧义分支调用（outbox_dispatcher.rs:2601-2610）。

### 2.11 `src/agent/send_ledger.rs`（523 行 = 393 生产 + 130 测试）

- `build_ledger_entry`（:10-40）：台账行构造，转化字段（responded/response_window_hours/stage_advanced/outcome_evaluated_at）一律留空回扫填。
- `record_send`（:44-88）：**fail-soft 写台账**（发送已成，台账缺一条不影响结果更不能诱发重发，:42-43）；无 outbox_id 锚→拒写（error log）；upsert `$setOnInsert` 以 `(workspace, account, outbox_id)` 为键——同 outbox 重复调用幂等。
- `record_send_for_entry`（:93-136）：实时成功/崩溃 reclaim post-hoc/timeout post-hoc 三处复用（outbox_dispatcher.rs:3057/2683）；send_kind 分流 `referral_card_id→"namecard"` 优先于 `media_asset_id→"media"`，**纯文本不记**（:107-114）；title 冗余快照 `lookup_target_title`（:139-169，namecard→referral_cards.display_name，其它→content_assets.title，查不到空串不阻断）；stage 快照取 `contact.domain_attributes.customer_stage`（:117-122）。
- `response_window_end_ms`（:173-175）：`sent + max(window,0)h`（负窗钳到 sent）。
- `stage_advanced`（:179-192）：两 stage 都在 ordered_stages 且 to 严格靠后→true；任一缺失/不在表→保守 false。
- `response_rate`（:195-201）：total=0→0.0；否则 4 位小数。
- `ordered_stages_from_machine`（:204-216）：从状态机 states 数组按出现顺序抽 key 作粗略阶段序。
- `scan_send_ledger_outcomes`（:220-320）——**转化回扫**（tasks.rs:1140 每 tick）：扫 `outcome_evaluated_at 不存在`（limit 200，sent_at 升序）；窗口未过→跳过下轮再看；`responded` = `(sent, sent+窗口]` 内该 contact inbound 计数 >0（**查询瞬时失败→跳过不落 evaluated，防 responded 假阴性永久化**，:265-273）；`stage_advanced` = 当前 `customer_stage` vs 发送时快照按 `load_user_ops_stage_order`（:323-336，**硬编码 domain="user_operations"** 的 current_version 状态机）判定；回填四字段。纯读+回写自己表，不调 LLM 不发消息（无副作用红线，:218-219）。默认窗口 24h（:224）。
- `recent_sends_for_contact`（:339-375）：按 (ws,account,contact,send_kind) 倒序取近期发送，best-effort 故障返空。
- `render_recent_media_lines`（:379-393）：注入 prompt 的"近期已发素材"判重段（软约束非硬门）。

### 2.12 `src/agent/pacing.rs`（51 行 = 19 生产 + 32 测试）

`account_send_interval_ms(jitter01, min_ms, max_ms)`（:15-19）：jitter clamp [0,1] 线性映射 `[min, max]`；`span=(max-min).max(0)`——max<min 退化恒返 min。调用点注入 `fastrand::f64()`（与 backoff 同款"随机在调用点、函数确定性可测"模式，:3-5）。配置默认 min=1000ms/max=4000ms（config.rs:482-483）。

### 2.13 `src/agent/quiet_hours.rs`（341 行 = 141 生产 + 200 测试）

- 产品语义（:1-14）：静默时段客户消息不立即回，`inbound_reply` 义务排到醒来时段一次性回复；主动发送到点则**重排**不取消；时区用运营参数固定偏移不依赖宿主时区；判定全部 epoch 毫秒纯整数运算。
- `DEFERRED_INBOUND_REPLY_KIND`（:20）：旧 kind `deferred_inbound_reply` 仅滚动升级兼容读。
- `in_quiet_hours(now_hour, start, end)`（:28-40）：`[start,end)` 含头不含尾；start<end 当日区间；start>end 跨午夜；**start==end 退化为永不静默**（防误配全天禁言，:27）。
- `hour_in_offset`（:46-49）：`div_euclid/rem_euclid` 防负数取模坑。
- `jitter_ms_for_seed`（:56-67）：FNV-1a（固定常量算法，跨 Rust 版本可复现；DefaultHasher 不保证）→`[0, max_seconds*1000]`；max=0→恒 0。把同 workspace 多客户整点唤醒打散。
- `next_wake_utc_ms`（:73-89）：本地日序推"下一个 end:00"，**恰命中 end:00 也取次日**（严格未来）；`end.min(23)` 防越界；回 UTC 后叠加 jitter。
- 薄包装 `is_quiet_now`（:92-98）/`next_wake_at`（:103-116，jitter_seed 通常传 contact.wxid，max 来自 `config.wake_jitter_max_seconds` 默认 900s，config.rs:812）。调用方：webhooks.rs:231/878/1603/1694、gateway.rs:2384/3828/5330、routes/shared.rs:618。
- `effective_quiet_hours_enabled`（:132-140）：**函数体只返回 workspace_enabled**——"Workspace policy is authoritative"，contact/profile override 仅保留读取兼容不再影响调度（:137-139）；但其 doc 注释（:118-131）仍描述 G04 三级解析链（见 §5 疑点 6）。运行参数默认：`quiet_hours_enabled=true`、`start=22`、`end=8`、`tz_offset_hours=8`（models.rs:4796-4811, 4952-4963）。

---

## 3. 跨文件机制

### 3.1 一条 approved 决策从入队到送达确认的完整时序

以"webhook 入站→决策 approved→2 段文本+1 个素材"为例：

1. **入队（gateway 侧）**：gateway 把 review 置 `outbox_enqueuing` 并固化 `expected_text_segments`，逐段 enqueue 文本（source_event_id 带 `#segN` 后缀→run_sequence=N，outbox.rs:199-203），再 enqueue 媒体（synthetic_media key，run_sequence=10000）；task 绑定 `bind_task_decision_if_owned`→全部入队后 `authorize_task_outbox_if_owned`（写 task=outbox_enqueued + outbox 行 marker）→review→`outbox_enqueued`。每次 enqueue 成功都 `notify_outbox_work()`（outbox.rs:389）。
2. **claim**：dispatcher 被唤醒（或 5s 轮询），`atomic_claim_pending` 按 `priority desc, created_at asc, run_sequence asc` 抢占——文本 seg0(90) → seg1(90) → 媒体(20)。
3. **process_entry 门链**（对每条）：请示卡授权（非请示卡恒过）→SR-034 第一检查（review 还在 enqueuing→defer 1s 无损退回；token 被新 inbound 替换→cancel stale_task_claim）→二次安全门（not_managed/cooldown/stop/stale 30min）→contact 门（撤管 cancel）→账号在线（离线 defer 60s 不耗 attempt）→崩溃恢复门（reclaimed 才核对）→pacing 闸（距上次实发 <1–4s 随机间隔→defer 到点）→SR-034 第二检查→最后可取消点 `begin_remote_send`（CAS send_started_at；取消/易主在此前全部拦截）。
4. **发送与结果**：文本走 `gateway::send_outbound_message`、媒体走 `send_outbound_media`（可能 2 次 MCP：上传+发送），外层 150s timeout：
   - 成功→`commit_sent_if_owned`（sent+sent_at；文本行带 `delivery_finalize_pending=true`）→`outbox_sent` 事件→run 聚合刷新→finalize+台账；
   - `SafeToRetry`→`schedule_retry_or_terminal`：attempt+1 <max→pending+backoff((2^n)*5s±20%)；≥max→failed_terminal；
   - `DeliveryUncertain`/timeout→`settle_ambiguous_send`→`verify_delivery`（文本：权威 chat_search 15s→本地 mcp_logs 兜底；媒体：media_id 定位；名片：恒 Inconclusive）→Delivered=post-hoc 补 sent（事件 `outbox_sent_post_hoc`）/NotDelivered=正常重试/Inconclusive=**delivery_unknown 禁自动重放**。
5. **finalize（最后一段文本 sent 后触发）**：`finalize_delivered_text_decision`——期望段数校验→review 短租约 `delivery_finalizing`→承诺 push+follow_up task upsert(_id=decision_id)+回复义务结算+relay 终结+task→sent→review→sent→清 marker。素材行独立发送不参与 finalize（但记 send_ledger 台账，转化由 `scan_send_ledger_outcomes` 24h 窗回扫）。
6. **崩溃恢复矩阵**（任一点崩溃）：
   - enqueue 中途崩（review 停在 outbox_enqueuing）→`reconcile_stale_outbox_enqueues`（60s 宽限后按 expected vs actual 三分并撤/续 task 授权）；
   - claim 后发送前崩→lease 180s 到期→reclaim→pending+reclaimed_in_flight→下次 claim 先 post-hoc 核对；
   - 发送后写 sent 前崩→同上，核对命中→post-hoc sent（不重发）；名片核对不了→delivery_unknown；
   - finalize 副作用中途崩→review 保持 delivery_finalizing+outbox marker→`reconcile_delivered_decision_finalizations` 续跑（副作用全幂等）；
   - worker 反复崩同一条→reclaim_count>5→failed_terminal 止损。
7. **取消矩阵**：用户 stop/cooldown（reaction.rs:316/568）或决策级取消（webhooks.rs:343/761、routes/tasks.rs:391）→`cancel_for_*`：pending 立即 canceled；in_flight 只登记 cancel_requested→dispatcher 在 claim filter（拒抢）、defer/retry filter（拒回 pending）、begin_remote_send（最后拦截）、reclaim（边界前完成取消/边界后 delivery_unknown）四处收敛。**跨界后的取消永远收敛为 delivery_unknown，绝不谎报 canceled 也绝不重放**（outbox_dispatcher.rs:853-855）。

### 3.2 失败矩阵（按"远端边界是否已跨"二分）

| 场景 | 边界前 | 边界后 |
|---|---|---|
| MCP 明确失败回执 | SafeToRetry→退避重试→耗尽 failed_terminal | 同左（ExplicitlyFailed=远端明确否定投递） |
| 回执无法验证 | — | DeliveryUncertain→post-hoc 三态 |
| 外层 150s timeout | —（begin_remote_send 前不会调 MCP） | settle_ambiguous_send→post-hoc 三态 |
| worker 崩溃/lease 过期 | reclaim→pending 重投（cancel_requested 则直接 canceled） | reclaim→pending+reclaimed_in_flight→核对；名片/带取消→direct delivery_unknown |
| 用户 stop | pending→canceled；in_flight→登记→begin_remote_send 前完成取消 | 登记→settle_late_cancel→delivery_unknown |
| 账号离线 | defer 60s（不耗 attempt） | 不适用（发送前检查） |
| pacing 命中 | defer 至 last_sent+interval（不耗 attempt） | 不适用 |
| contact 删除/撤管 | 撤管 cancel；删除 retry→terminal（非内部类） | 不适用（发送前检查） |
| task 授权 Building/Stale | defer 1s / cancel stale_task_claim | begin_remote_send filter 含 marker，失配即不开始 |

### 3.3 escalation 与 gateway/tasks 的接口（核证调用点）

- **触发**：approved 路 `gateway.rs:1930 trigger_principal_escalation`（gateway.rs:4825/5008 调）；hold 路 `gateway.rs:3762 → escalation::escalate_held_decision`。两路错误均 fail-soft 只 warn。
- **推卡**：台账 `pending_enqueue`→`materialize_principal_card_delivery`→outbox（source_kind=principal_escalation、source_event=`principal-card:{id}:{gen}` 每代确定性幂等）→dispatcher 发送前后 `principal_card_send_is_authorized` 双检代数→sent 后由 `reconcile_principal_card_deliveries_once` 回灌 `delivery_state=sent + last_pushed_at_ms`（tasks.rs:1125 驱动）。
- **领导回复**：webhooks.rs:1412（入站判定 `lookup_principal_config` 后分流）→`handle_principal_reply`→澄清（outbox principal_clarification）或 `resolve_escalation`→`materialize_relay_task`（task._id=escalation._id）。
- **relay**：task worker 领 `principal_decision_relay`→gateway.rs:185 分流→`handle_principal_decision_relay_with_claim`→有效授权走 `gateway.rs:2027 relay_principal_decision_to_customer`（出站前守卫**仅** `relay_output_leaks_internal_payload`——**24 号修正：数字白名单已从 relay 出站删除**（gateway.rs:4188-4192，KD-01/03），仍走 outbox）；relay 送达由 dispatcher finalize 调 `terminalize_principal_relay_for_task(..., "delivered")`（outbox_dispatcher.rs:1626-1629）终结并清 awaiting 标记。
- **超时/恢复**：tasks.rs:1124-1138 每 tick 依次 `reconcile_pending_relay_intents` / `reconcile_principal_card_deliveries` / `scan_escalation_timeouts`。
- **信号回注**：decision prompt 经 `build_decision_signals_text`（awaiting/卡死/all 模式/已引荐）感知通道状态；`is_principal_relay_trigger` 让 relay 合成消息豁免频控 precheck。
- **知识沉淀**：resolve 后（is_generalizable）`emit_knowledge_gap_proposal` 落 draft+needs_review（AI 永不自动 verify）。

---

## 4. 事实卡速查

### 4.1 OutboxStatus 转移矩阵（写入点核证）

| from → to | 触发 | 位置 |
|---|---|---|
| (无)→pending | enqueue | outbox.rs:343,364 |
| pending→in_flight | atomic claim（+worker/token/lease，$inc claim_generation，清 cancel_requested_at/send_started_at） | outbox_dispatcher.rs:359-373 |
| pending→canceled | cancel_for_decision / cancel_for_contact（用户/决策取消） | outbox.rs:724-731, 876-883 |
| pending→failed_terminal | reclaim_count>5 止损 | outbox_dispatcher.rs:295-310 |
| in_flight→(登记 cancel_requested) | 取消通道（状态不变） | outbox.rs:732-745, 884-897 |
| in_flight→canceled | 二次门/contact 门/授权 stale（cancel_entry，send 未开始）；complete_requested_cancel_before_send；reclaim 边界前取消 | outbox_dispatcher.rs:2189-2242, 630-686, 209-234 |
| in_flight→pending | defer_until_task_authorized(+1s)；schedule_retry(退避)；defer_account_offline(+60s)；defer_account_pacing；reclaim(+reclaimed_in_flight,$inc reclaim_count) | outbox_dispatcher.rs:563-602, 2280-2309, 2384-2440, 2470-2525, 264-286 |
| in_flight→sent | commit_sent_if_owned（正常成功/post-hoc 确认） | outbox_dispatcher.rs:749-784, 2651-2686 |
| in_flight→failed_terminal | attempt 耗尽 | outbox_dispatcher.rs:2330-2375 |
| in_flight→delivery_unknown | 核对 Inconclusive/出错；跨界晚到取消；名片 lease 过期跨界 | outbox_dispatcher.rs:799-851, 856-911, 238-263 |
| sent / canceled / failed_terminal / delivery_unknown | 终态，无出边（delivery_unknown 禁自动重放，等离线核验） | outbox.rs:55-56 |

run 级聚合：in_flight > pending > 全 sent=sent / 部分 sent=partially_sent > failed_terminal > delivery_unknown > canceled（outbox_dispatcher.rs:980-1023）。

### 4.2 二次门四条件与豁免

纯函数 `check_second_safety_gate_pure`（outbox.rs:640-670），顺序：
1. `!is_managed` → `not_managed_at_send`（含 contact 被删）
2. `cooldown_until > now` → `contact_cooldown_active`
3. `last_inbound > decision_created && outcome ⊇ stop_requested|cooldown_requested` → `user_stop_requested_after_decision`
4. `now - entry_created > 30min` → `outbox_stale_30min`

豁免（整门跳过）：`principal_escalation / principal_clarification / system_incident`（outbox_dispatcher.rs:922-929）。**manual_send 不豁免本门**（仅豁免 contact 状态门，见 §5 疑点 2）。另一独立门 `check_contact_status_pure`（:2737-2754）豁免四类：manual_send + 上述三类。

### 4.3 重试退避公式与上限

- 公式：`base=(2^clamp(attempt,0,10))×5s`，jitter ±20%（`backoff_with_jitter_seeded`，outbox.rs:599-607）；attempt=1/2/3 → 10/20/40s。
- max_attempts：enqueue clamp `<=0→3, min(10)`（outbox.rs:311-315）；dispatcher 兜底 `<=0→3`（outbox_dispatcher.rs:2247-2253）。
- 不耗 attempt 的 defer：task 授权 Building(+1s)、账号离线(+60s)、pacing(到 last_sent+interval)。
- reclaim 上限 5 次（超→failed_terminal）；每 entry 事件 cap 20（超→sentinel 一条后静音）。
- 时序不变量：`60s(MCP client)×2 ≤ 150s(send timeout) < 180s(lease)`；poll 5s；tick 处理 cap 16；aging FIFO 每 10 次 claim 一次。

### 4.4 escalation 闭集（models.rs 核证）

- 台账 status：`pending / resolved / delivery_failed`（models.rs:4491-4498）。
- category：`out_of_scope_decision / high_risk_gated / stuck_or_undelivered`（models.rs:4533-4540）。
- verdict：`approved / rejected / conditional / deferred / delegated_back`（models.rs:4564-4575）；越界→sanitize 回落 deferred（logic.rs:413-425）。
- 卡投递态：`pending_enqueue → queued → sent | failed_terminal | delivery_unknown`（models.rs:4504-4508；转移：materialize :169-185、reconcile :241-282、reassign 回 pending_enqueue+代数+1 :1103-1115）。
- relay_state：`pending → enqueued → terminal`（models.rs:4500-4502；resolve :628、materialize_relay_task :988-992、terminalize :457-462）。
- exemption_type：`none / customer_only / knowledge`（models.rs:4578-4580）。
- 豁免/标记 attrs：`awaiting_principal_decision(+_ids)`（models.rs:4545-4549）、`referred_specialist_at / referred_card_id / assist_mode_override`（models.rs:4557-4561）。
- relay 哨兵：`__PRINCIPAL_RELAY__`（models.rs:841）；出站泄漏守卫另拦 `verdict=/substance=/constraints=`（logic.rs:214-219）。

### 4.5 AskHumanPolicy 全字段与默认

字段（policy.rs:8-18）：`decider_chain: Vec<DeciderRef{wxid, display_name, account_id}>`、`escalate_safety_guard`、`escalate_unverified_product`、`escalate_ai_policy_hold`、`escalate_stuck`、`dedupe_window_hours: Option<f64>`、`daily_push_cap: Option<u32>`、`quiet_hours: Option<AskHumanQuietHours{start_hour, end_hour, tz_offset_hours}>`、`timeout_hours: Option<f64>`。

旧字段回落默认（policy.rs:35-57）：safety_guard=**true**、unverified_product=**true**、ai_policy_hold=**仅 high_risk_escalation_mode=="all"**、stuck=**true**、三骚扰门+timeout 全 **None**（=全放行+无限等待）、链=principal_decider 单人或空。冻结时缺 account_id 的 decider 绑定触发账号（policy.rs:80-109）。授权过期窗：领导说了算，没提=None=不过期（mod.rs:536-546）。

### 4.6 pacing / quiet_hours / 其它常量

- pacing：`account_send_interval_ms` 线性映射；配置 `ACCOUNT_SEND_MIN/MAX_INTERVAL_MS` 默认 **1000/4000ms**（config.rs:482-483）；只对"距该账号上次 status=sent 的 sent_at"生效；查询失败放行。
- quiet_hours（运营域 runtime 参数，models.rs:4796-4811 + defaults :4952-4963）：`enabled=true`、`start=22`（含）、`end=8`（不含）、`tz_offset_hours=+8`；start==end→永不静默；醒来时刻=下一个本地 end:00（严格未来）+FNV-1a per-contact jitter ∈[0, `WAKE_JITTER_MAX_SECONDS`=900s]（config.rs:812）；`effective_quiet_hours_enabled` 现只认 workspace 开关。
- holding reply：独立预算 `HOLDING_REPLY_TOKEN_BUDGET`=3000 tokens / 2 次 LLM 调用（config.rs:650；holding_reply.rs:144-149）；链尾安抚最小间隔 `HOLDING_REPLY_MIN_INTERVAL_HOURS`=6.0h（config.rs:648）。
- 媒体：`MEDIA_ID_CACHE_TTL_HOURS`=24（config.rs:811）；仅账号私有 asset 复用缓存。
- send_ledger 回扫：默认响应窗 24h、单轮 limit 200（send_ledger.rs:224,237）；stage 序取 user_operations 域状态机。
- 短码：`E`+4 位 base31（无 0/O/1/I/L）；碰撞重试 5 次。
- 事件名清单（outbox 链）：`outbox_created / outbox_idempotent_skip / outbox_synthetic_idempotency_key / outbox_canceled / outbox_cancel_requested / outbox_retry_scheduled / outbox_failed_terminal / outbox_sent / outbox_sent_post_hoc / outbox_delivery_unknown / outbox.event_cap_reached / agent.send_deferred_account_offline / agent.send_deferred_account_pacing`。

---

## 5. 偏差与疑点

1. **manual_send 会被二次门的 `not_managed_at_send` 拦截，与 contact 状态门的豁免语义相互矛盾**。`second_safety_gate` 的豁免清单只有三种内部通知（outbox_dispatcher.rs:922-929），manual_send 不在其中；contact 非 managed（或已删）时纯函数第 0 条即返 `not_managed_at_send`（outbox.rs:650-654）→ process_entry :2799-2802 取消。但下游 `check_contact_status_pure` 明确豁免 manual_send（"admin 已显式确认发送意图"，:2730-2748）。结果是：**admin 对 normal/paused 客户的手动发送会在二次门被取消，永远到不了那条为它设计的豁免**。同理 manual_send 也受 cooldown 与 30min stale 拦截（后两者或是有意保守）。两个门的注释意图相反，疑似设计缺口或有意收紧未同步注释——需对照 admin_outbox 路由/产品预期确认。
2. **`second_safety_gate` 传入纯函数的 `decision_created_ms` 实为 outbox entry 的 created_at**（outbox_dispatcher.rs:967：`let decision_created_ms = entry.created_at.timestamp_millis()`），并非 decision review 的创建时刻。正常链路 entry 紧随 decision 创建（毫秒级偏差）语义近似成立；但"用户 stop 发生在 decision 之后、enqueue 之前"的窄窗口内，`last_inbound > entry.created_at` 为假会漏判 stop（该窗口极窄且 enqueue 前有 gateway 自己的检查，风险低）。参数命名与实际取值不一致，记录为偏差。
3. **`escalate_held_decision` 骚扰门拦截时连 pending 台账都不建，但注释称"pending 台账可由 admin 在收件箱处置"**（mod.rs:231-233）。实际代码顺序是 push_allowed 不过→`return Ok(())`，`insert_pending_escalation` 在其后（:262）根本未执行——被骚扰门拦下的 hold 请示不留任何痕迹（下次同类 hold 才再触发）。注释与行为不符：либо 应先落台账再决定是否推卡（scan/reconciler 稍后补推），либо 注释该改。对照 `scan_escalation_timeouts` 改派路径：骚扰门不过只是 continue（台账已存在，稍后重试），两处语义不一致。
4. **`quiet_hours::effective_quiet_hours_enabled` 的 doc 注释（G04 三级解析链）与函数体（只认 workspace 开关）不一致**（quiet_hours.rs:118-131 vs :132-140）。函数体内注释已说明"Workspace policy is authoritative...no longer alter scheduling behavior"，测试 :330-340 也按新行为断言，但外层 doc 仍详述 contact override→profile→global 链与"DEFAULT 字节等价"论证。文档滞后于实现，读者易被误导。
5. **两套 quiet-hours 实现并存**：`escalation/policy.rs:123-132`（领导骚扰门用，`AskHumanQuietHours`，手工 `%24` 修正）与 `agent/quiet_hours.rs:28-49`（客户作息门用，`div_euclid/rem_euclid`，带 start==end 退化）。行为上对合法输入等价（policy 版 start==end 时两分支同样恒 false），但边界处理风格不同、无共享代码——将来只改一处易漂移。记录为结构性偏差（非 bug）。
6. **媒体/名片 delivery_priority=20 低于一切文本与系统事故**（outbox.rs:174-184），仅靠每 10 次 claim 一次的 FIFO aging（outbox_dispatcher.rs:3269-3272）保证最终服务。持续高入站负载下素材发送延迟可达分钟级（16 条/tick×5s 间隔下 aging 槽稀疏）。设计上是有意为之（客户文本优先），但"承诺发资料却迟迟不到"的体验风险依赖 aging 参数，未见 SLA 测算，记为观察点。
7. **`delivery_unknown` 的请示卡不进入超时改派扫描**：`list_timeout_eligible_escalations` 只收 `delivery_state=sent`（ledger.rs:1136），且 unknown 不写 `last_pushed_at_ms`（reconcile :264-267 仅 SENT 写）。若卡送达状态永远无法核验（如 MCP 日志缺失），该请示会长期滞留 pending：领导若实际收到卡仍可回复（list_pending_for_principal 含 unknown :571-575），但若确实没收到，无自动改派/安抚，只能靠 admin 收件箱。保守设计（不確認送达就不启动 SLA 时钟）可以理解，但滞留无告警，记为疑点。
8. **`interpret_principal_reply` 的 LLM 调用错误直接向上传播**（mod.rs:433-443 `?`），只有"JSON 解析失败/verdict 越界"才回落 deferred。웹hook 消费路径上 LLM 网络抖动会让该次领导回复处理返回 Err（webhooks 层如何兜底在本次范围外未核）。若 webhook 侧只 log 不重试，领导这句话会被丢（须再回一次）。记为跨界疑点。
9. **`send_outbound_namecard` 的 MCP 入参是占位形态**（referral.rs:199-205 `⚠️ message_send_namecard 入参字段名待 server tools/list 确认`；media_send.rs:168 同款注记）。上线前需与 MCP server 实测对齐，否则名片/上传路径首次运行即 SafeToRetry 循环→failed_terminal。
10. **`mcp_success_filter` 对 `response.ok` 缺失时要求 newMsgId 为非空 string**（outbox_dispatcher.rs:2571-2577）——若 MCP 返回数值型 msgId（serde 成 number），post-hoc 会漏判成功→Inconclusive→delivery_unknown（不会重发，安全侧偏差）。同款约束出现在 media_success_filter（media_send.rs:381-387）。依赖 MCP 返回形态稳定，记为观察点。
11. `assert_target_is_principal`（logic.rs:179-190）与 `derive_sediment_title[_fallback]` 的 `#[allow(dead_code)]`（ledger.rs:763-765, 792-794）注释声称"暂无生产调用点"，但 `derive_sediment_title` 实际已被 `emit_knowledge_gap_proposal` 调用（ledger.rs:681-688）——dead_code 注记过期（无害，属注释腐化）。
12. **pacing 闸对内部通知同样生效**：principal 请示卡/澄清/系统事故没有 pacing 豁免（outbox_dispatcher.rs:2903-2920 无 source_kind 分支），领导卡会被 1–4s 拟人间隔推迟。影响极小（都走同一微信账号确实该限频），仅记录语义：内部通知也被"拟人节奏"覆盖。

---

## 6. 覆盖自证

| 文件 | 总行数 | 读取方式 | 覆盖 |
|---|---|---|---|
| `src/agent/outbox.rs` | 1686 | Read 全文一次（1-1686） | 100% |
| `src/agent/outbox_dispatcher.rs` | 3771 | Read 分 4 段（1-950 / 950-1899 / 1899-2858 / 2858-3771） | 100% |
| `src/agent/escalation/mod.rs` | 661 | Read 全文一次 | 100% |
| `src/agent/escalation/policy.rs` | 614 | Read 全文一次 | 100% |
| `src/agent/escalation/labels.rs` | 70 | Read 全文一次 | 100% |
| `src/agent/escalation/ledger.rs` | 1279 | Read 分 2 段（1-700 / 700-1279） | 100% |
| `src/agent/escalation/logic.rs` | 1247 | Read 分 2 段（1-700 / 700-1247） | 100% |
| `src/agent/escalation/holding_reply.rs` | 325 | Read 全文一次 | 100% |
| `src/agent/referral.rs` | 479 | Read 全文一次 | 100% |
| `src/agent/media_send.rs` | 625 | Read 全文一次 | 100% |
| `src/agent/send_ledger.rs` | 523 | Read 全文一次 | 100% |
| `src/agent/pacing.rs` | 51 | Read 全文一次 | 100% |
| `src/agent/quiet_hours.rs` | 341 | Read 全文一次 | 100% |
| **合计** | **11672** | | **100%（无跳读）** |

跨文件核验（Grep/Read 当场确认，非记忆）：`models.rs`（闭集常量 :841, 4491-4581, 4796-4811, 4952-4963）、`run_envelope.rs`（source_kind :34-45）、`config.rs`（:482-483, 648-650, 811-812）、`mcp.rs`（:25 MCP_CLIENT_TIMEOUT_SECONDS=60）、`types.rs`（:24-58 OutboundSendError/DeliveryVerification）、`tasks.rs`（:15, 279, 289, 468, 532-533, 1124-1140）、`gateway.rs`（:185, 1930, 2027, 3762, 4825, 5008, 5029-5035）、`webhooks.rs`（:343, 458, 589, 761, 1412）、`reaction.rs`（:316, 568）、`main.rs`（:242）、`routes/tasks.rs`（:391）、`system_incident.rs`（:587）。
