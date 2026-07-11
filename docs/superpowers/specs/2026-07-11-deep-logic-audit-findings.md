# 核心业务逻辑全链路深度审查 · 台账（批 A：自动回复命脉链）

- 日期起始：2026-07-11
- **single source of truth**：本文件是批 A（及后续批次）深度审查所有 findings 的唯一权威台账。任何 finding 只入此账，不散落到别处；修复进度也在此逐条更新状态。
- 审查方式：逐行读码（PLAUSIBLE）+ 可行时 117 生产真跑复现（CONFIRMED）。
- 审查阶段**只入账、先不修**；审完一批按用户定优先级进入修复（走各自 PR）。

## 关联文档

- 设计：[`2026-07-11-deep-logic-audit-design.md`](./2026-07-11-deep-logic-audit-design.md)（分批地图 / 五步闭环 / 117 真跑硬约束）
- 上轮台账：`2026-07-10-full-system-test-findings.md`（本轮补其后端深度盲区，不重复前端 UI/UX 面）
- 权威依据：`docs/agent-policy.md`、`.kiro/specs/agent-autonomy-loop/requirements.md`（红线与闸阈值）

## Finding 字段格式（固定结构）

每条 finding 用以下结构，无问题的审查点也明确写"✅ 亲验通过"：

```
### [编号] 一句话标题
- 入口频道: command / userOps / autonomy / ...
- 链路环节: ①…⑧
- 类型: 逻辑正确性 | 竞态 | 幂等 | 错误处理 | 红线 | 越权 | 一致性
- 严重度: Critical | High | Med | Low
- 现象/风险: …
- 根因: file:line（亲验）+ 简述
- 复现设想: 输入/前置状态；可 117 复现的标注"可 117 复现"
- 验证状态: CONFIRMED（真跑）| PLAUSIBLE（读码）
- 修复建议: …
- 状态: Open | Fixed | WontFix
```

严重度初判由审查 subagent 给出，主控亲验后可调整。

---

## 环节① webhook 入口 / 签名 / 账号解析 / 领导分流 / managed 门 / inbound 落库

审查文件：`src/webhooks.rs`（`wechat_webhook`:287 起、`verify_webhook_signature`:1763、`resolve_account_context`:977、领导分流 :443、managed 门 :590、inbound 落库 :512）。审查依据：`.kiro/specs/agent-autonomy-loop/requirements.md`（R13 幂等/outbox、产品定位红线 :18/:159/:164/:362）、`config.rs`（webhook 相关默认值）、`db/indexes.rs`（dedupe 唯一索引）。

### ✅ 亲验通过总览

以下审查点亲验后判定行为正确，逐条列证：

- **签名失败返回 400 而非 500**：`verify_webhook_signature` 失败 → handler `return Err(AppError::BadRequest("invalid signature".into()))`（`webhooks.rs:353`），BadRequest 映射 400。✅ 正确。
- **时间戳 skew 比较符 + 边界**：`(now_ms - ts_ms).abs() > skew_seconds.saturating_mul(1000)`（`webhooks.rs:1784`），用 `>` 故恰好 300s 不超窗（单测 `accepts_timestamp_at_window_edge` :1878 覆盖），过去/未来对称（`.abs()`），`saturating_mul` 防溢出。默认 `WEBHOOK_TIMESTAMP_SKEW_SECONDS=300`（`config.rs:705`）。✅ 正确。
- **验签 fail-closed**：`webhook_verify_signature` 默认 `true`（`config.rs:704`）；开启时若账号 `webhook_secret=None`/空 → `SecretNotConfigured` → 400（`webhooks.rs:1771-1774`）。验签打开时无密钥不会被放行。✅ 正确的 fail-closed。
- **未知 appId 不再静默回退 default**：`resolve_account_context` 中 appId 提供但 `wechat_accounts` 无匹配 → 明确 `BadRequest`（`webhooks.rs:994-996`），handler 侧再写一条 `webhook_unknown_app_id` admin 事件后返 400（:322-327）。这修掉了 memory 记录的"账号错配家族"里"inbound 落错 account 致 managed 永远 lookup 不到"的旧回退路径。✅ 正确。
- **同 wxid 跨 account 的 managed 错配 fail-safe**：`upsert_webhook_contact` 检测到同 `(workspace_id, wxid)` 已在另一 `account_id` 下 managed 时，写 `webhook_managed_contact_account_mismatch` 警示事件（`webhooks.rs:1063-1104`），本次仍建 `normal` 影子记录 → **AI 不会误用错账号自动回复**（managed 门在 :590 判 normal → 不触发）。失败方向安全（宁可不回，不误发）。✅ 正确（但见 [A-01] 的产品可见性讨论）。
- **managed 门读的是最新态 + 运行期二次复核**：`webhooks.rs:590` 的 `managed` 读的是本次 webhook 刚 find_one/upsert 的 contact（:520-536），仅用于决定"是否注册去抖 + spawn"。真正发送前，去抖 runner 会 `reload_managed_contact` 重新查库并 `filter(agent_status == Managed)`（:165、:270-284）；窗口期转 unmanaged → `Ok(None)` → runner 退休不发。因此**不存在读旧态误发**。✅ 正确。
- **inbound 落库先于 spawn + 非 dup 失败 fail-close**：`insert_one`（:512）在 spawn 去抖 runner（:639）之前；重复 dedupe_key → 命中 `is_duplicate_key_error` → 返 `duplicate:true` 幂等短路（:514-515）；其它写错误 `return Err`（:517）→ 不触发 agent（落库失败不静默吞、不盲发）。✅ 正确。
- **msgId 幂等靠原子唯一索引**：dedupe_key 优先 `message:{msgId|_mcp.sourceMsgId}`，缺失回落 `payload:{hash}`（:483-486）；依赖 `db/indexes.rs:71-84` 的 partial-unique index（`workspace_id+account_id+dedupe_key`，`$type:string`）在写入时原子去重（消除旧 check-then-insert 的 TOCTOU，见 :488-492 注释与 P0-19）。✅ 正确。
- **领导分流在落库前短路，防领导消息被当客户入站**：`lookup_principal_config` 命中 → `handle_principal_reply` → `consumed` 为真即 `return`（`webhooks.rs:443-457`），发生在 inbound `insert_one`（:512）之前 → 领导回复不进客户会话、不触发客户链路。✅ 顺序正确（但见 [A-02]）。

### [A-01] 同 wxid 跨 account 的 managed 错配只写事件、不建影子副本，客户会被静默晾着
- 入口频道: userOps
- 链路环节: ① webhook 入口
- 类型: 一致性
- 严重度: Low
- 现象/风险: 同一真人 wxid 在 account A 下被标 managed，但某次 inbound 经 account B 的 webhook 进来（账号迁移/多号加同一人）。当前只在 account B 下建 `normal` 影子记录 + 写一条 `webhook_managed_contact_account_mismatch` 警示事件，**不会自动回复**，客户这条消息事实上被晾着，只有运维主动看事件流才发现。代码注释自己也写了"不创建影子副本会更激进，留给后续 PR"。
- 根因: `webhooks.rs:1060-1104` — 检测到错配仅 `events().insert_one(...)`，随后照常 upsert 为 `normal`（:1124-1131 `$setOnInsert agent_status:"normal"`）；managed 门 :590 判 normal → 不触发回复。
- 复现设想: 造一个 wxid 在 account102 下 managed，再从另一 appId（account 别的号）灌一条该 wxid 的 AddMsg。观察：落库到 account B、建 normal、写警示事件、无回复。可 117 复现（需两个已注册账号，谨慎，勿碰真人吴界）。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 产品决策项——是否要在错配时把 inbound 重路由到 managed 所在 account，或至少给运营一个"一键合并/转移"入口。属产品意图裁决，非纯 bug；建议列 Open 待用户拍板。
- 状态: Open

### [A-02] `handle_principal_reply` 恒返回 `consumed=true`，令"既是领导又是客户"的 wxid 永远收不到客户回复
- 入口频道: userOps / askHuman
- 链路环节: ① webhook 入口（领导分流 :443）
- 类型: 逻辑正确性
- 严重度: Low
- 现象/风险: `wechat_webhook` 用 `if consumed { return ... }`（:454-456）预留了 consumed=false 时继续走客户链路的分支，注释也写"领导可能同时也是某 contact，consumed=true 时短路返回"。但 `handle_principal_reply` 的三个 match 臂（`NoPending` / `Ambiguous` / `Matched`）**全部 `Ok(true)`**（`escalation/mod.rs:300 / 319 / 330 / 345 / 348`）——consumed=false 是死分支。后果：若同一 wxid 既被登记为 `principal_decider` 又是某 managed 客户，其**所有**消息都被吞进请示通道（哪怕当前根本无 pending 请示，走 `NoPending` 也返 true），永远不会作为客户入站被回复。
- 根因: `src/agent/escalation/mod.rs:286-351` — 所有返回路径 `Ok(true)`；`NoPending` 臂（:295-301）在"无未决请示"时仍 `Ok(true)` 短路，未回落客户链路。
- 复现设想: 把某 wxid 同时配成 workspace 的 principal_decider + account102 下 managed 客户，让其发一条普通客户消息（无 pending 请示）。观察：webhook 返 `{"routed":"principal"}`，消息不落 conversation_messages、无客户回复。可 117 复现（需配 principal_decider，谨慎）。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 两选一，需产品裁决：(a) 明确"领导与客户身份互斥"为产品约束，则删掉 webhook 侧的 `if consumed` 死分支+注释以免误导；(b) 若允许同一人双身份，则 `NoPending` 应返 `Ok(false)` 让消息回落客户链路。当前实现落在两者之间，语义含混。属产品意图裁决。
- 状态: Open

### [A-03] 无 msgId 时 payload-hash 去重会误杀"内容相同的合法连发"
- 入口频道: command / userOps
- 链路环节: ① webhook 入口（dedupe :483-486）
- 类型: 幂等
- 严重度: Low
- 现象/风险: 当入站既无任何 msgId 键、`_mcp.sourceMsgId` 也缺失时，dedupe_key 回落到 `payload:{stable_payload_hash(payload)}`（:485-486）。若同一客户在极短时间内连发两条**内容与 payload 完全相同**的消息（如两次"在吗"），第二条 hash 相同 → 命中唯一索引 → 被当 duplicate 静默丢弃（:514-515），客户实际发了两条只落一条。
- 根因: `webhooks.rs:483-486` + `stable_payload_hash`（:746-754）对整个 payload 做 FNV hash 当 dedupe key；无 msgId 时无法区分"重放"与"合法重复内容"。
- 复现设想: 灌两条不含任何 *MsgId / _mcp.sourceMsgId 且 body 逐字节相同的 payload。第二条返 `duplicate:true`。**生产近乎无影响**——真实 GeWe AddMsg 恒带 `NewMsgId`（见 :1388 真实样例 + `real_gewe_addmsg` 测试），effective_message_id 必有值，走 `message:{id}` 而非 payload-hash 分支。风险仅限自测/手工无 ID payload。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 生产无影响，可 WontFix；若要严谨，payload-hash 分支可掺入接收时刻毫秒/随机 nonce 降低误杀（但会削弱重放防护，需权衡）。建议标注为"已知边界，生产不触发"。
- 状态: Open

### [A-04] 验签通过后 300s skew 内的重放窗口（已被 dedupe/幂等大幅缓解）
- 入口频道: command
- 链路环节: ① webhook 入口（签名门 :333）
- 类型: 红线（入口鉴权）
- 严重度: Low
- 现象/风险: 验签只校验"签名正确 + 时间戳在 ±300s 内"，不含 nonce/一次性校验。攻击者若截获一条合法签名请求，可在 300s 内原样重放。
- 根因: `webhooks.rs:1763-1797` 无 nonce/已用签名记录；重放窗口 = skew（默认 300s）。
- 复现设想: 截获一条带合法 `x-webhook-signature` + `x-webhook-timestamp` 的 AddMsg，300s 内重发。**实际影响很小**：AddMsg 重放命中 message-id dedupe → `duplicate:true` 幂等短路（:512-515）；Offline/Online 重放只是重复 `$set online`（幂等）；领导回复重放经 `resolve_escalation` 幂等（`escalation/mod.rs:344-345`）与 `NoPending`。故重放不产生重复发送/重复副作用。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 当前缓解已足够，无需加 nonce（会引入状态存储成本）。仅记录为"已知、已缓解"的入口特性。建议 WontFix。
- 状态: Open

### [A-05] 缺失 appId（None）在关闭验签时回退 default account，多账号下可能张冠李戴
- 入口频道: command
- 链路环节: ① webhook 入口（`resolve_account_context` :977）
- 类型: 一致性
- 严重度: Low
- 现象/风险: 当 payload 完全没有 appId 键时，`resolve_account_context` 返回 `(default_workspace_id, default_account_id, None)`（:998-1002）。若此时 `WEBHOOK_VERIFY_SIGNATURE=false`，请求会被当作 default account 的入站处理；在多账号部署里，一条本属账号 B 但漏带 appId 的消息会落到 default account。
- 根因: `webhooks.rs:998-1002` 无 appId 时的 default 回退；secret=None，仅在验签开启时才被 `SecretNotConfigured`（:1771-1774）挡住。
- 复现设想: `WEBHOOK_VERIFY_SIGNATURE=false` 下灌一条无 appId 的 payload。观察落到 default_account_id。**生产默认 verify=true（`config.rs:704`）**，None appId → secret=None → 400，天然挡住。风险仅存于显式关闭验签的部署。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 生产默认配置已 fail-closed。可考虑：无 appId 时无条件 400（不回退 default），彻底消除该路径。属加固项，非现网 bug。
- 状态: Open

### [A-06] `last_inbound_at` 更新或 inbound 写入的瞬时 DB 错误使本条消息在下一条到来前不被回复
- 入口频道: userOps
- 链路环节: ① webhook 入口（:512 / :552）
- 类型: 错误处理
- 严重度: Low
- 现象/风险: inbound `insert_one`（:512）非 dup 错误、或 contact `last_inbound_at` `update_one`（:552）出错，都用 `?` 向上抛 → handler 返 500。MCP 侧 5s timeout 且**失败不重试**（:582-584 注释），故这条 webhook 不会被重推，本条入站在**下一条消息到来前**得不到回复（下一条到来时去抖 runner 的 `load_recent_messages` 会聚合补回）。
- 根因: `webhooks.rs:552-566` 的 `.await?` 传播 + MCP 无重试语义（外部约束）。insert 失败 fail-close 是对的（避免盲发），但 `last_inbound_at` 更新纯属统计/信号旁路，却也能拦掉 spawn。
- 复现设想: 注入 Mongo 瞬时错误于 contacts.update_one。难在生产安全构造，不建议真跑。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 可考虑把 `last_inbound_at` 时间戳更新降级为 best-effort（失败只 warn，不拦 spawn），与 `collect_inbound_behavior_signals`（:570 已是 best-effort）对齐——时间戳落库失败不应连累应答。属小加固。
- 状态: Open

### 已知非本轮 bug 的架构约束（记录留痕，不入 finding）

- **去抖 PENDING 进程内 DashMap，单副本才成立**：`webhooks.rs:69-70` 注释明确"若 webhook 摄入横向扩多副本需改 DB 原子 claim + 心跳"。当前单副本部署 → 串行/抢占语义成立，非现网 bug。属就绪债（同 memory `project_multitenant_isolation_debt`）。

---

## 环节② 去抖 pipeline（register_inbound / deadline / generation 抢占 / barge_in / 窗口聚合）

审查文件：`src/webhooks.rs`（去抖调度 :54-284）+ `src/agent/gateway.rs`（抢占 guard 落点）+ `src/agent/reaction.rs`（步骤 d）。主控已亲验 B-02 结构（webhooks.rs:188 else 包裹 (e)）+ reaction.rs:110 `?` 上抛，因果链坐实。

### ✅ 亲验通过总览
- **deadline 顺延 + 静默才决策**：`register_inbound` 每条入站刷 `deadline_ms.store`（`webhooks.rs:110/121`），runner 内层 loop 每轮重读 `deadline_ms.load`，`now>=dl` 才 break（:150-158）——窗口内又来消息就继续睡。`next_deadline_ms` 饱和加防溢出（:92-94，单测 :1473）。✅
- **generation 抢占无双 runner 竞态**：spawn-vs-bump 原子决策在 `PENDING.entry(key)` DashMap shard 写锁内（:111-124），N 并发恰一个 `spawned_now=true`（单测 `concurrent_register_same_key_spawns_exactly_once` :1636）；退休 `remove_if` 谓词在 shard 锁内复核 generation（:233-237），晚到 bump 与 remove_if 串行化——两种交错各有单测（`retire_blocked_when_late_inbound_bumped_generation` :1570 / `retire_then_new_inbound_respawns` :1606）。任意交错不会同时有两个活 runner、不丢边界消息。✅ 机制严密。
- **reload 早退三分支全覆盖**：`Ok(Some)`→继续 / `Ok(None)`→`PENDING.remove`+return（:167-170）/ `Err`→写事件+remove+return（:171-184），每轮 loop 起点 fresh DB 读（:165）挡住窗口期转 unmanaged/删除。✅
- **load_recent_messages 边界**：按 `{created_at:-1}` 取最近 N 条（`gateway.rs:5010-5036`，N=recent_limit*6 clamp[24,80]），非时间窗过滤；inbound 在 spawn 前已落库（webhooks.rs:512 早于 :639），睡眠结束后连发消息全在库一次性聚合。"窗口边缘"不构成 bug。✅（仅单次突发>N 条时最旧被挤出上下文，属上下文上限非去抖 bug）

### [B-01] 抢占 guard 的入队尾窗 TOCTOU：末次检查后到达的新入站拦不住，致"过时回复照发+重算再回"双回复（补 F-021 深层）
- 入口频道: userOps
- 链路环节: ② 去抖 pipeline（抢占 guard）
- 类型: 竞态
- 严重度: Low
- 现象/风险: 协作式抢占最后一道 `should_abort_send()` 在 outbox 入队之前（`gateway.rs:2531-2542`）。新入站若落在该 guard 返 false 之后、多段 enqueue 循环（:2543-2642，多次 DB 往返）进行中，本轮过时回复会全部 enqueue→dispatcher 照发；同时 runner (f) 步（`webhooks.rs:227`）检测 generation 变化→continue 重算→再 enqueue 一批。两批 segment 不同幂等 key，不互相去重，客户收两次回复。
- 根因: `gateway.rs:2531-2542` 是入队前最后 guard；`:2543-2642` 多段入队循环内无 guard 复查。唯一撤销通道 `outbox::cancel_for_contact_on_user_reaction`（`outbox.rs:545`）仅下一轮 reaction 判 `outcome_signals_stop` 才取消，普通追问不触发。
- 复现设想: 精确时序——过 :2542 guard 后、多段 enqueue 期间灌新入站（窗口约 10-100ms）。观察 outbox 两批 segment（source_event_id 不同）。可 117 复现但时序窗极窄、需注入延迟，生产自然触发概率低。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: 协作式抢占固有尾窗；彻底消除需"入队后按 generation 撤销 pending outbox"补偿（重算前先 cancel 上一 gen 的 pending，用 gen/run_id 标记 outbox 归属）。属产品取舍，Open 待裁决；生产影响小。
- 状态: Open

### [B-02] reaction 分析脚手架的瞬时 DB 错误经 `?` 上抛，吞掉本轮聚合回复（补 F-021 深层）
- 入口频道: userOps
- 链路环节: ② 去抖 pipeline（步骤 d record_user_reaction → 步骤 e gateway）
- 类型: 错误处理
- 严重度: Med
- 现象/风险: runner 步骤 (e) 网关聚合回复被包在步骤 (d) `record_user_reaction` 的 **`else` 分支**里（`webhooks.rs:188-224` 主控亲验）——reaction 返 `Err` 时只写 `agent_error` 事件，(e) 网关**根本不执行**，本条客户消息本轮不回复。reaction 的 LLM 失败虽被 `unwrap_or_else` 兜底（`reaction.rs:162`），但其 **DB 脚手架多处 `?` 会真上抛**：claim `find_one_and_update`（`reaction.rs:110` 主控亲验 `.await?`）、`load_..domain_config`（:36）、stuck 重置 `update_many`（:83）、写回（:196）。任一瞬时 Mongo 错误→整轮回复被侧路分析失败连累吞掉。因 MCP 5s 不重试、webhook 不重推，本条入站要等**下一条消息到来**触发 respawn 才被 `load_recent_messages` 补回；客户不再发则永久静默。
- 根因: `webhooks.rs:199-224` 把步骤 (e) 放进步骤 (d) 的 `else`，使"侧路反应分析成功"成为"生成本轮回复"的前置；`reaction.rs` DB 操作用 `?` 传播而非 best-effort。
- 复现设想: 对 `decision_reviews` claim/update 注入瞬时错误（前置：该 contact 有 `status=sent` 且 `outcome_status∈{null,pending}` 的 decision_review 才进 claim→LLM 路径；无则 :113 提前 `Ok(())` 不触发）。生产安全构造困难，不建议真跑。
- 验证状态: PLAUSIBLE（读码，主控亲验因果链坐实）
- 修复建议: 把步骤 (d)(e) 解耦——reaction 失败只 warn，仍继续跑 (e)（reaction 是旁路分析不该阻断本轮应答）；或把 `reaction.rs` DB 脚手架降级 best-effort（与其 LLM 侧 `unwrap_or_else` 同纪律）。属错误处理加固。建议 Open。
- 状态: Open

### [B-03] managed 门只在 pipeline 起点复核一次，决策运行期（~10-15s）的 managed→unmanaged 翻转不复核，照发在途回复（补 F-021 深层）
- 入口频道: userOps
- 链路环节: ② 去抖 pipeline（reload → gateway）
- 类型: 逻辑正确性
- 严重度: Low
- 现象/风险: `reload_managed_contact` 只在 loop 起点执行一次（`webhooks.rs:165`），gateway `precheck_send_gateway` 用这份快照判 `contact.agent_status`（`gateway.rs:3127`），全程不再重查。抢占 guard 只感知新入站（generation），不感知 agent_status 翻转。管理员在 reload 后、发送前的决策窗口（~10-15s）把 contact 改 normal（想立即止住 AI），本轮回复仍会 enqueue 并发出。
- 根因: `webhooks.rs:165` 单次 reload；`gateway.rs:2308/2531` guard 仅基于 generation；`precheck` 读入参 contact 非 fresh DB。
- 复现设想: contact managed，客户发消息触发 runner；决策期间（借慢 LLM 拉长窗口）管理员改 normal。观察回复仍发。可 117 复现（谨慎，勿碰真人吴界）。
- 验证状态: PLAUSIBLE（读码）
- 修复建议: outbox 入队前兜底 guard（:2531 附近）追加一次 `agent_status==Managed` fresh 复核（或让抢占 guard 一并检测状态翻转）。多一次 DB 读换"止发即时生效"。属加固项，生产窗口小。建议 Open。
- 状态: Open

### 已知非本轮 bug 的架构约束（不重复入 finding）
- reload 查询缺 `workspace_id`（`webhooks.rs:279` 只 `{account_id,wxid}`，入口是三键），单副本单 workspace 无害（account 绑定唯一 workspace）；理论多 workspace 撞 account+wxid 才取错。极低风险 nit，与多租户就绪债同族。

## 环节③ gateway 巨型闸函数（managed / cooldown / min-interval / 日上限 / 过期闸）

审查文件：`src/agent/gateway.rs`——`run_user_operation_gateway`:616 → `run_user_operation_gateway_inner`:999（~1294 行、无单测）+ `precheck_send_gateway`:3121 + C2 派生块 :4144-4526。subagent 逐行走查 + 1 opus 复核，主控亲验 C-01 铁证（:4503 `.await?` vs :3988 `let _` 对称孪生不对称）。**本环节是验证重心，findings 少但每条读透。**

### ✅ 亲验通过总览（三大验证点）
- **闸序 + 临界值全对**（`precheck_send_gateway` :3127-3236）：顺序 = not_managed → (relay 豁免以下) cooldown → operation_policy → rate_limited/min-interval → daily_limit → expired → quiet_hours → context_changed。逐一核比较符临界：cooldown `until>now`（到点放行）、min-interval `elapsed<interval`（`<` 非 `<=`，刚到点即可发）、daily `>=max`（达上限即拦，仅约束 FollowUp，Inbound 豁免 :3477）、**expired 刻意排在 quiet_hours 之前**（:3167 注释：确保过期死任务作废而非被重排次日发）。not_managed 对 relay 不豁免（退运营不转述）正确。✅ 无该拦没拦/该放没放。
- **operation_state 派生一致 + fail-soft**（:4144-4182）：唯一写点，`synced_state`=归一后 customer_stage，缺失回落 decision.operation_state（与 CLAUDE.md 逐字一致）；非法转移只记 rejected、不写、保留旧值（:4177-4180）；rejected/transitioned 事件 if/else 互斥。✅（唯 C-01 的 `?` 缺陷）
- **RunBudget 降级四处全兜住**：知识路由（:1132 mark_degraded+empty_route 不 Err）、review（:1466 local_decision_review 兜底）、rewrite（:1527 跳过）、revision（:1748 Skip→Held）。全仓 grep 确认 `AppError::BudgetExceeded` 主链路**从不构造**（budget.rs 返 BudgetError 非 AppError），不泄 5xx 给 webhook。降级三分支无漏兜致 no_reply。✅

### [C-01] operation_state 非法迁移的 rejected 审计事件用 `?` 阻断未入队回复（违 fail-soft 红线）
- 入口频道: webhook Inbound（主要受害）/ follow-up（有 worker 重试兜底）
- 链路环节: ③ gateway（apply_agent_updates 内 C2 派生块）
- 类型: 错误处理 / 红线（fail-soft 语义）
- 严重度: Medium（主控裁定：subagent 初判 Low-Med；升 Med 因双重铁证违红线 + Inbound 真丢一轮回复；触发前提 DB 瞬时故障压住上限）
- 现象/风险: CLAUDE.md + 代码自身注释（:4486「reply 已照常下发」、:4166 fail-soft）要求非法 state 迁移只跳过写 + 发审计事件、**绝不阻断已发/将发回复**。但 :4489-4503 rejected 事件写用 `.await?`（`gateway.rs:4503` 主控亲验），`write_event_for_account` 的 Mongo insert 失败即 return Err，沿 apply_agent_updates(:2356 `?`)→inner→gateway 冒泡。此时回复**尚未入 outbox**（enqueue 在 :2543 更后）→ 本轮回复丢。webhook Inbound Err 只写 agent_error、**不重发**（webhooks.rs:205-219）→ 该轮回复彻底丢；follow-up 被 worker Err 重试（tasks.rs:254-307）影响较小。
- 根因（亲验 file:line）: `gateway.rs:4503` rejected 事件 `.await?`。**铁证对比**：同函数孪生事件 `:3988 stage_transition_rejected` 用 `let _ = ...await`（fail-soft），且 `:3987` 注释明写「审计写失败不阻断主流程，与 dimension_dropped 同风格」、`:3966` 注释明写「与 operation_state_transition_rejected **对称**……校验/审计失败均不阻断」——它声称对称的孪生反而用了 `?`。:3942 `dimension_dropped` 亦 `let _`。
- 复现设想: rejected 分支命中（LLM 给非法 state 跳转）+ 该次 `write_event_for_account` Mongo insert 恰好失败。难在 117 稳定复现（依赖 DB 瞬时故障注入），标 PLAUSIBLE，非"可 117 复现"候选。
- 验证状态: PLAUSIBLE（主控亲验代码路径 + 对称铁证成立；唯"DB insert 会失败"触发前提无法正常环境构造，保守不标 CONFIRMED）
- 修复建议: :4503 改 `let _ = ...await;`（或 `.ok()`），与同函数 `stage_transition_rejected`/`dimension_dropped` fail-soft 纪律对齐。**连带评估**：:4525 `operation_state_transitioned`（成功迁移审计，同 `?`）同属"回复已定纯观测事件"，审计写失败同样会连带丢回复，是否一并改需产品裁决（不带拒绝迁移红线注释）。
- 状态: Open

### 主控确认排除的假 finding（留痕防漏报/误报）
- **[排除] follow-up 提前标 `outbox_enqueued` 致 DB 故障窗口静默丢消息**：inner :2337 确在 enqueue(:2543) 前置 task `outbox_enqueued`，中间一串 `?` 若 Err 会在 enqueue 前 return。但 `tasks.rs:254-307` worker Err 分支用 `doc!{"_id":task_id}`（无 status CAS）**无条件**重写 task 为 retry/failed，覆盖提前写的 outbox_enqueued → 会重试非静默丢。不成立。（连带观察：该 Err 分支无 status CAS 是隐性耦合就绪债，当前恰好兜住，非当前 bug。）

## 环节④ 决策 + 渐进式知识路由（decision.rs / knowledge_router.rs）

审查文件：`src/agent/decision.rs` + `src/agent/knowledge_router.rs` + `src/agent/types.rs`（RawAgentDecision 契约）。主控亲验 D-01（types.rs:405 `rename_all=camelCase` + decision.rs:150-153 双形兜底不对称）。

### ✅ 亲验通过总览（历史已修现码仍正确 + 红线成立）
- **PR#107 tool_calling 静默 no_reply**：prompt 恒 final + 守卫测试 + gateway 兜底闸三重，首发/rewrite/revision 三站点全覆盖。现码仍正确。✅
- **双层标签红线**：taxonomy 候选 fire-and-forget 异步 upsert，硬门 `is_protocol_violation_tag` 不含 candidate 标签 → 不阻断 run。✅
- **知识路由拿不到知识→拦截非幻觉**：verified-only 语料 + fallback 同池排序 + `blocked_unverified_product_claim` 终局硬门。✅
- **PR#143 升档预算**：两升档分支升档前授 escalated ceiling、非升档不授、used_knowledge_ids 只在升 Full 记，接线正确。现码仍正确。✅

### [D-01] reply 主路径顶层 customerStage/intentLevel 只认 camelCase，无 snake 双形容错（与初始画像路径不对称）
- 入口频道: userOps
- 链路环节: ④ 决策（RawAgentDecision 顶层 serde 键）
- 类型: 一致性 / 错误处理（静默降级）
- 严重度: Low（主控认同 subagent 初判：LLM 主流漂 camelCase、rename_all 即照其设计；漂 snake 是少数；丢失只是标签没打上=画像不精准，非丢回复/绕红线）
- 现象/风险: `RawAgentDecision` 有 `#[serde(rename_all="camelCase")]`（`types.rs:405` 主控亲验），顶层 `customer_stage`/`intent_level`（:461-462）序列化只认 camelCase 键 `customerStage`/`intentLevel`。LLM 若顶层输出 snake_case，serde 静默 miss → None（字段 `default` 不报错、不报 risk）。对比初始画像路径 `decision.rs:150-153` 有手动双形兜底 `optional_string("customerStage").or_else(|| ...("customer_stage"))` ——**两路径不对称**：画像容双形，reply 主路径只容 camel。历史 PR#151 修的是 Document 内层键双形，未覆盖此顶层 serde 键。
- 根因（亲验）: `types.rs:405` rename_all=camelCase + :461-462 无 `#[serde(alias="customer_stage")]`；`decision.rs:150-153` 对照有双形兜底。
- 复现设想: 构造 LLM 响应顶层用 snake `customer_stage` → 解析后该字段 None，标签丢失。是否生产实际触发依赖 LLM 顶层键漂移率（未知，subagent 诚实标"需 117 日志统计"）。可 117 日志统计辅证，非直接真跑复现。
- 验证状态: PLAUSIBLE（主控亲验代码路径 + 不对称确凿）
- 修复建议: 给 :461-462 加 `#[serde(alias = "customer_stage")]` / `alias="intent_level"`（serde alias 让 camel 主名 + snake 别名双认），与初始画像路径的双形容错对齐。低成本加固。
- 状态: Open

## 环节⑤ 独立 review + 阈值闸 + revision（review/gates.rs）

审查文件：`src/agent/review/gates.rs`（review_passed:20 / classify_dual_gate:115 / finalize_review_for_send / decide_revision）+ 默认值 `models.rs:3767-3781` + revision 控制流 `gateway.rs:1800` + 状态闭集 `run_envelope.rs`。主控亲验默认值(models.rs:3767-3781=6/7/7/6/6)+比较符对偶(gates.rs:20-45)。**验证重心环节 —— 结论：命脉核心闸设计扎实、干净。**

### ✅ 亲验通过总览（这道命脉防线整体健康）
- **阈值默认值 = CLAUDE.md 声明一字不差**（主控亲验 `models.rs:3767-3781`）：hallucination_block_at=6 / pressure_risk_block_at=7 / knowledge_grounding_block_below=7 / human_like_rewrite_below=6 / emotional_value_rewrite_below=6。runtime 可配 + admin 覆盖走 `clamp(1,10)` 护栏（runtime.rs:242）防误配禁用硬闸。✅
- **五闸比较符精确对偶**（主控亲验 `gates.rs:20-45` review_passed 放行侧 ↔ `:115-207` classify_dual_gate 拦截侧）：FactRisk `<`放行/`>=`拦(临界6拦)、ProductAccuracy `>=`放行/`<`拦(临界7放)、HumanLike/EmotionalValue `>=`放行/`<`改写(临界6放)、PressureRisk `>=7`拦。无符号反置。✅
- **硬闸优先软闸**：classify 先判 hard，非空即 return（:142），软闸绝不绕过硬闸（测试 :1644）。✅
- **revision 单次硬保证**：`gateway.rs:1800` 是单个 `match`（非 loop），Proceed 只调一次；revision 后经 finalize+review_passed 双重复检，无循环回边；二次不达标→revision_failed 兜 Held。✅
- **状态闭集 fail-closed**：`run_envelope.rs` 闭集完备，写库前 `assert_*_status_valid`（:556-566）不在闭集→返 AppError::External 不静默 coerce；人工接管禁词 `FORBIDDEN_HUMAN_HANDOFF_VALUES` 优先硬阻断；gates.rs 产出字面量全在闭集内。✅
- **R5.4 verified-knowledge 产品声明硬门独立于 grounding 软闸**（:658-691），reviewer 自评高分也拦。✅

### [E-01] pressure_risk / boundary_privacy_safety 的「0 值豁免」双义可被 reviewer 填 0 绕过软闸（观测项）
- 入口频道: userOps / command（同一 gateway）
- 链路环节: ⑤ review 软闸
- 类型: 设计权衡（已知 tradeoff）
- 严重度: Low（主控认同 subagent：观测项非 bug）
- 现象/风险: `gates.rs:38-39`（pressure_risk）与 `:43-44`（boundary_privacy_safety）采「值=0 即豁免、不参与拦截」。0 同时承担"reviewer 未评分哨兵"（R11 老数据反序列化默认）与"合法最低分"两义。若 reviewer LLM 某轮把 pressure_risk 输出为 0（而非真实低分），该软闸静默失效、不触发 revision。
- 根因（亲验）: `gates.rs:37-39` 注释明确 0 是"reviewer 未给分/老数据反序列化默认，不参与拦截"，属 R11 向后兼容显式取舍。
- 为何 Low/观测非 bug: reviewer 是本系统自有 LLM（非对抗输入）；两个 0 豁免的都是**软闸→改写**非硬 block，误放行代价有限；硬闸（hallucination `>=` / grounding `<`）+ 两个无 0 豁免软闸（humanLike/emotionalValue）全程生效，质量兜底不空。
- 复现设想: 可 117 复现——构造 reviewer 对高压迫话术回评 `pressureRisk=0` 观察是否跳过 revision。但需干预 LLM 输出，非自然高频路径。
- 验证状态: PLAUSIBLE（主控亲验 :38-39 逻辑 + 注释意图）
- 修复建议: 是否引入 `Option<i32>`/-1 哨兵区分"未评分"与"评 0 分"，涉 R11 反序列化基线，留用户裁决，勿擅改。
- 状态: Open

## 环节⑥ outbox 幂等 / claim / second-pass safety gate / retry

审查文件：`src/agent/outbox.rs` + `src/agent/outbox_dispatcher.rs`（辅证 gateway/reaction/media_send/mcp/indexes）。主控亲验 F-01 铁证（reclaim :689 直接 mcp_already_succeeded vs timeout :851-878 先 chat_search 再回落，两分支不对称）。**验证重心环节。**

### ✅ 亲验通过总览（含历史三处已修基线仍在位）
- **历史已修基线全部现码仍正确**：①FIFO sort `atomic_claim_pending` `.sort({created_at:1,_id:1})`（`outbox_dispatcher.rs:165`，PR#136）②`MCP_CLIENT_TIMEOUT=60 < SEND_TIMEOUT=150 < LEASE=180`（守护测试 `send_timeout_covers_worst_case...`:1159，PR#164）③second-pass 每次 MCP 前真重查（`process_entry`:600→609 现查非缓存）。✅
- **幂等键维度充分**：文本 `{source_event_id}:{contact}:{content_hash}`（:215）+ 多段 `#seg{idx}`（:2565）；媒体/名片 synthetic 键含 asset_id/card_id（:411）避空 content 撞键；manual_send 摘 run_id + day_bucket。unique 索引存在（indexes.rs:772），DuplicateKey→IdempotentSkip。✅
- **先入 outbox 再 MCP 无先发后记窗口**：MCP 只从 dispatcher process_entry 发起，gateway 侧全是 enqueue 后返回，S5.2 已删直连。✅
- **claim 原子无双 claim**：find_one_and_update 单文档原子 + filter `status=pending`，第二 worker 跳下一条；worker_id=hostname:pid:uuid。✅
- **cancel 彻底**：`cancel_for_contact_on_user_reaction`（outbox.rs:545）把同 contact pending/in_flight 全置 canceled + unset worker_id/locked_until。✅
- **retry backoff 有界**：`2^attempt×5s` clamp[0,10] + jitter±20%；状态机 pending→in_flight→(sent|failed_terminal|canceled) 无卡死；掉线/pacing defer 刻意不耗 attempt。✅

### [F-01] 崩溃恢复(reclaim)分支文本 post-hoc 只查本地 mcp_call_logs 不查权威 chat_search → 崩溃后可能重发文本（补 PR#164 未覆盖分支）
- 入口频道: userOps / command（凡走 outbox 的文本发送）
- 链路环节: ⑥ outbox 崩溃恢复
- 类型: 幂等 / 竞态（防重发缺口）
- 严重度: Medium（主控认同：后果=重发给真实客户顶下限；触发需"MCP已送达+mcp_logs未落+此刻崩溃+lease过期"多条件叠加+单worker崩溃罕见压上限）
- 现象/风险: reclaim 分支（`outbox_dispatcher.rs:689` 主控亲验）文本条目**直接** `mcp_already_succeeded`（只查本地 `mcp_call_logs`）。对比 timeout 分支（`:851-878` 主控亲验）文本**先查 MCP `chat_search_outbound`**（server 真实已发记录，同步落库、不受本地取消影响），失败才回落本地日志。`mcp_call_logs` 写入 best-effort（`mcp.rs:358` `let _`）且在 MCP 响应**之后**。worker 在"MCP 已送达微信、mcp_logs 未落库"窗口崩溃 → lease 过期被 reclaim → 下一 worker `mcp_already_succeeded` 本地查不到 → 判"没发过" → 重发同一句给客户。崩溃恰是本地日志最不可靠时刻，此分支却唯独不查权威 chat_search。**reclaim 分支注释(:670)"与 timeout 分支同一核对函数"是过期/错误的**——PR#164 给 timeout 加 chat_search 时漏同步给 reclaim。
- 根因（亲验）: `outbox_dispatcher.rs:689`（reclaim 文本走本地）vs `:851-878`（timeout 文本先 chat_search）不对称；`mcp.rs:358` 日志 best-effort 且在响应后。
- 复现设想: 可 117 复现——managed 联系人触发回复入队 → dispatcher claim 后在 MCP 成功返回与 mcp_logs 写入之间 kill 进程（或删该 mcp_call_log 行模拟丢日志）→ 等 lease 过期(>180s) → 观察下一 tick 是否重发同内容。谨慎，勿碰真人吴界；须串行不与套件并发。
- 验证状态: PLAUSIBLE（主控亲验两分支不对称确凿；触发窗口宽度需 117 时序实验估计）
- 修复建议: reclaim 文本分支比照 timeout 先 `chat_search_outbound` 权威核对、失败再回落 `mcp_already_succeeded`；把 timeout 分支核对逻辑抽共用函数两处复用，消除非对称。
- 状态: Open

### [F-02] enqueue 与 dispatcher 的 max_attempts 默认值分歧（3 vs 5，死代码分支）
- 链路环节: ⑥ retry 状态机 / 类型: 一致性 / 严重度: Low
- 现象/根因（亲验）: `outbox.rs:244-248` enqueue 兜底 `<=0→3`（落库恒≥1）；`outbox_dispatcher.rs:322-326` schedule_retry_or_terminal 用 `<=0→5`。`<=0` 分支对 enqueue 产出 entry 是死代码（永不触发），仅手工/历史脏文档走到。当前无生产影响。
- 验证状态: PLAUSIBLE（非活跃 bug）
- 修复建议: dispatcher 侧默认改 3 对齐或删该分支。
- 状态: Open

### [F-03] 成功 update 忽略 modified_count，cancel 竞态下审计不一致（不致重发）
- 链路环节: ⑥ cancel×send 竞态 / 类型: 一致性 / 严重度: Low
- 现象/根因（亲验）: send 成功置 sent 的 update_one filter 含 `status=in_flight`（`outbox_dispatcher.rs:784-802`）不查 modified_count。发送在途时用户 stop→cancel 改 canceled，则 sent-update 匹配 0 行静默无效。消息已 MCP 发出（不可撤），DB 停 canceled → 审计"已取消却已送达"不一致。**不重发**（canceled 终态，reclaim 只碰 in_flight）。属固有取消延迟。
- 验证状态: PLAUSIBLE（非双发风险，仅审计观感）
- 修复建议: 可选——命中 0 行补 warn 事件提升审计可解释性。
- 状态: Open

### [F-04] reclaim 不消耗 attempt → 反复崩溃理论无界重发不入 failed_terminal
- 链路环节: ⑥ 崩溃恢复 / 类型: 错误处理 / 严重度: Low
- 现象/根因（亲验）: `reclaim_expired_leases`(:98-128) 仅置 pending+reclaimed_in_flight，attempt 不变；被 reclaim 条目若判未发出走全新实发（非 schedule_retry_or_terminal），不累加 attempt、不受 max_attempts 约束。若 worker 每次同位置崩溃→无限 reclaim→重试永不进 failed_terminal。生产单 worker 崩溃罕见，影响小。
- 验证状态: PLAUSIBLE（纯边缘）
- 修复建议: 可选——加独立 `reclaim_count` 上限，超限转 failed_terminal 交 admin。
- 状态: Open

## 环节⑦ MCP 发送（message_send_text / result.isError / 超时）

审查文件：`src/mcp.rs`（call_tool_with_key:160 / isError:195）。**0 finding —— 本环节亲验干净。** 主控亲验 isError 检查（:195-207）现码仍正确。

### ✅ 亲验通过总览（0 finding）
- **三层失败识别齐全**（主控亲验 mcp.rs:188-207）：HTTP 状态 + JSON-RPC 顶层 `error`（:188）+ `result.isError`（:195-207）。isError=true→返 `AppError::External`（带 content detail），early-return 在 structuredContent 提取（:208）之前；`unwrap_or(false)` 保证 server 不发 isError 时不误触发（no-op 兼容）。历史 finding③（PR 5779c33，联系人拒收类"HTTP200 但失败"）现码仍在且逻辑正确。✅
- **超时层级**：`MCP_CLIENT_TIMEOUT=60`（reqwest client 上）配合外层 `SEND_TIMEOUT=150`，不变式 `60×2≤150<lease180` 有编译进 dispatcher 的守卫测试锁死。✅
- **API key 安全**：只作 Bearer header，redact_request_for_log 不打印 key。✅
- **5xx/网络错误**：转 upstream_error 不 panic；response.json() 用 `?` 不 unwrap。✅
- **structuredContent 提取**：isError 分支加入后既有提取路径不变（no-op 兼容）。✅

### 备案（非缺陷）
- `MCP_CLIENT_TIMEOUT_SECONDS`/`MCP_SEND_TIMEOUT_SECONDS`/`MAX_SEQUENTIAL_MCP_CALLS_PER_SEND` 是编译期常量而非 env 可调项（memory/brief 称"默认值"略有语义出入），但值正确且有守卫测试，不构成问题。

## 环节⑧ 回写（events / outcome metrics / decision review / run log / operation_state 派生）

审查文件：`src/agent/gateway.rs` 回写段 + `run_envelope.rs` + outcome_metrics/tasks/outcomes_autonomy。**主控去重/证伪**：F-⑧-01 与 C-01 部分重叠（去重后保留新增 3 处）、F-⑧-02 与 Task3 已排除的假 finding 同条（主控亲验 tasks.rs:254-307 后驳回）。

### 关键架构澄清（修正 brief 假设）
**gateway 不做同步发送**——物理 MCP 发送推迟到 outbox dispatcher 异步（gateway 只 `outbox_enqueue`:2586，`message_send_text` 由 dispatcher:768 后台发）。故 gateway 所有回写（events/decision_review/run_log/operation_state/metrics）都在 enqueue **之前**：gateway 回写用 `?` 抛错时消息**尚未入队**，后果是"本轮不回复"（**非重发**）。C-01 关于"送达后回写致重发"在 gateway 侧不成立——真正送达后回写全在 dispatcher，已亲验受 lease-reclaim + post-hoc 保护（见✅）。

### ✅ 亲验通过总览（9 点）
- **dispatcher 送达后 sent 回写受保护**：MCP 成功后置 sent 用 `?`（outbox_dispatcher.rs:802），失败则停 in_flight→lease 过期 reclaim→下轮 post-hoc 核对命中标 sent 不重发。✅（唯 reclaim 文本分支非对称见 F-01）
- **update_run_log_outbox_status fail-soft**（outbox_dispatcher.rs:233-255 `let res;if Err{warn}`）✅
- **enqueue 后回写 outbox_status=pending fail-soft**（gateway.rs:2596 `let _`）✅
- **多段部分入队失败正确**：单段失败不中断续尝试其余，循环后写 partial_failure 事件再返 Err；每段 `#seg{idx}` 独立幂等 key 重跑 skip 已入队段。✅
- **run log token usage 完整**（从 RunBudget 快照取 token_budget/tokens_used/llm_calls_used/degraded_reasons，gateway.rs:4897）✅
- **promptVersions 未断链**：落 decision_reviews.prompt_versions（gateway.rs:4725，8 key）与 run_log 同 run_id 关联可 join。✅
- **operation_state 派生落库口径与决策侧一致**（synced_state 优先 canonical customer_stage 缺失回落 decision.operation_state，applied_operation_state 据实际写入判迁移事件，与环节③同源无漂移）✅
- **outcome_metrics 写/读 workspace_id 口径一致**（写 tasks.rs:681 / 读 outcome_metrics.rs:38 均带 ws+account；单租户一致，多租户就绪债）✅
- **planner 聚合读侧口径一致**（outcomes_autonomy.rs:145 match ws+account+kind 白名单，与写侧一致，$ifNull 归 0 无断层）✅

### [H-01] apply_agent_updates 内另有 3 处审计事件误用 `?`（C-01 同族扩展，enqueue 前→丢回复）
- 入口频道: userOps / command
- 链路环节: ⑧ 画像/状态回写侧（apply_agent_updates 内）
- 类型: 错误处理 / 文档-代码漂移（C-01 同族）
- 严重度: Medium（同 C-01 家族：审计事件误 `?`、enqueue 前 DB 抖动吞本轮回复；单租户单 worker 低发压上限）
- 现象/风险: `apply_agent_updates`（gateway.rs:2356 调用，**早于** enqueue:2586）内注释多处声称"回复已异步发出，写失败绝不阻断"（:4168/4232/4285/4345），据此对纯观测旁路正确用了 fail-soft（bayesian warn:4266 / relationship `let _`:4333 / suspected_deal `let _`:4383）。**但同函数另有 5 处审计事件用 `?`**：其中 :4503 operation_state_transition_rejected、:4525 operation_state_transitioned **已在环节③ C-01 入账**；本条新增 **3 处**——`:4411 g1_correction`、`:4480 profile_churn_observed`、`:4551 follow_up_run_at_degraded`（主控亲验三处均 `.await?`）。注释"reply 已下发"前提为假（此刻尚未 enqueue），DB 抖动使这些纯审计写 return Err→本轮回复永不入队→丢回复。
- 根因（亲验）: gateway.rs:4411/4480/4551 `.await?`；apply_agent_updates 调用点 :2356 早于 enqueue :2586。与 C-01 同一代码气味，构成"同函数 5 处审计事件系统性误用 `?`"family。
- 复现设想: 令 events 集合在这些事件写时刻注入 Mongo 瞬时错误，观察本轮 inbound 无回复、无 outbox 条目。生产安全构造困难，标 PLAUSIBLE。
- 验证状态: PLAUSIBLE（主控亲验 3 处 `?` + ordering 属实）
- 修复建议: 与 C-01 合并修复——apply_agent_updates 内**全部纯审计事件**（含 C-01 的 :4503/:4525 + 本条 :4411/:4480/:4551）统一降级 fail-soft（`let _`/`if Err{warn}`），与同函数 bayesian/relationship/suspected_deal 口径一致。C-01 + H-01 应作为一个"审计事件 fail-soft 对齐"修复项一并处理。
- 状态: Open

### [H-02] run_envelope.rs R0「LLM 前先写信封」三函数是生产死代码，pre-LLM 追溯不变量未生效
- 入口频道: —（可观测性）
- 链路环节: ⑧ run log 生命周期
- 类型: 文档-代码漂移 / 就绪债
- 严重度: Low
- 现象/风险: `run_envelope.rs` 头声称 R0.1「LLM 调用前 insert lifecycle=started 信封，确保超时/panic/JSON 失败也有可追溯条目」，提供 `write_run_envelope_started`/`update_run_envelope_terminal`/`install_panic_hook_for_envelope`。但全仓 Grep 这三符号**除定义+doc 外无生产调用点**（main.rs 无引用）。gateway 仍走单次 insert 的 `write_agent_run_log_with_finalize`（gateway.rs:4908）。即决策前 panic/超时的 run **不留** started 信封，R0 追溯在生产未生效。
- 根因（亲验）: 模块 doc 自述"W1 task 2.5 会把 gateway 入口改为先调 write_run_envelope_started"为将来时（run_envelope.rs:23-27/705），接线从未落地。Grep 仅内部定义 + models.rs:2730 一处 doc 引用。
- 复现设想: 读码确认无调用点，无需真跑。
- 验证状态: PLAUSIBLE（主控可复核 Grep 无调用点）
- 修复建议: 要么按原设计接上 started 信封+终态 update+panic hook，要么文档标注 R0 为"未接线/推迟"避免误以为已有 pre-LLM 追溯。当前单次 insert 对"决策已产出"的 run 追溯完整，缺口仅"决策前 panic/超时"极端 run。
- 状态: Open

### 主控驳回的候选（证伪留痕）
- **[驳回] F-⑧-02 task 先置 outbox_enqueued 再 enqueue，中间 `?` 失败留孤儿任务丢消息**：subagent 自标"未能确认（未核 tasks.rs worker）"。**主控亲验 tasks.rs:254-307**：gateway 返 Err → worker `Err` 分支 `update_one(doc!{"_id":task_id}, $set status=retry/failed)`（:261/:292）filter **仅 `_id` 无 status 条件、无条件覆盖** outbox_enqueued → task 会被打回 retry 重跑（enqueue 幂等重跑安全），**不卡孤儿、不静默丢**。与环节③ Task3 已排除的假 finding 同条。驳回，不入账。
