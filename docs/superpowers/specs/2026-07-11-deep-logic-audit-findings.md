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
- 状态: WontFix（已知边界，doc 标注 —— 生产 GeWe 恒带 NewMsgId 走 message-id 分支，payload-hash 仅自测触发；家族⑧ #待补）

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
- 状态: WontFix（已知边界，doc 标注 —— dedupe/幂等已缓解重放无重复副作用，加 nonce 收益不抵成本；家族⑧ #待补）

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
- 状态: Fixed（家族⑧ #待补 —— resolve_account_context 无 appId 分支收敛：仅 verify=false 且多账号时 400，单账号仍回落 default 不打断单账号部署）

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
- 状态: Fixed（家族⑧ #待补 —— last_inbound_at/last_message_at/updated_at 统计 update 降 best-effort，失败仅 warn；inbound insert_one 的 fail-close 保持不动）

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
- 状态: 已知产品取舍待专项（家族⑨ 标注不修 —— 协作式抢占固有尾窗，彻底消除需 gen 撤销补偿触碰 outbox 幂等核心风险最高；#待补）

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
- 状态: Fixed（家族⑨ #待补 —— second_safety_gate 发送前 fresh 复核 managed，非 managed 拦截 not_managed_at_send）

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
- 状态: Fixed（家族⑨ #待补 —— reclaim 累加 reclaim_count，>5 转 failed_terminal 止损）

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

---

## 批 A 总评（自动回复命脉链 · 审查收口）

**审查方式**：8 环节逐行读码（webhook→去抖→gateway 闸→决策→review→outbox→MCP→回写），每环节派 opus subagent 只读审 + **主控逐条亲验 file:line**（多条 subagent 结论经亲验后调级/去重/驳回）。审查阶段只入账不改 src。

### finding 计数（去重后，全部主控亲验）
- **Critical 0 / High 0**
- **Medium 4**：
  - **B-02**（环节②）reaction 分析 DB 失败经 `?` 上抛、吞掉本轮聚合回复（(e) 网关包在 (d) 的 else 里）
  - **C-01**（环节③）operation_state 非法迁移 rejected 审计事件 `:4503 .await?` 阻断未入队回复（违 fail-soft 红线，孪生 :3988 用 `let _` 为铁证）
  - **H-01**（环节⑧）apply_agent_updates 另 3 处审计事件（:4411/:4480/:4551）同样 `?`——C-01 同族，构成"同函数 5 处审计事件系统性误用 `?`"
  - **F-01**（环节⑥）outbox reclaim 崩溃恢复分支文本 post-hoc 只查本地 mcp_call_logs、漏查权威 chat_search（timeout 分支有）→ 崩溃后可能重发文本（PR#164 漏同步此分支）
- **Low 9**：A-01/A-02（wxid 双重身份场景族）、B-01（抢占尾窗 TOCTOU 双回复）、B-03（决策期 managed 翻转不复核）、D-01（reply 顶层 serde 键 camel-only 不对称）、E-01（reviewer 填 0 绕软闸）、F-02/F-03/F-04（outbox 边缘）、H-02（run_envelope R0 死代码）
- **主控驳回 2**：F-⑧-02（=Task3 排除的假 finding，worker 无条件重试兜底）；outbox_enqueued 静默丢（同上，环节③已排除）
- **整环干净 2**：环节⑤ review 阈值闸（默认值/比较符对偶/revision 单次/状态闭集全过）、环节⑦ MCP 发送（三层失败识别/isError/超时/key 安全 0 finding）

### 跨环节根因家族（修复统筹）
1. **审计/旁路事件误用 `?` 连坐吞回复家族**（B-02 + C-01 + H-01，共 6 处 `?`）：本轮**最重、最系统性**的发现，也正是上一轮广度走查扫不到的"错误处理层"。共性 = 发送/回复所必需路径把"旁路观测写（reaction 分析 / operation_state 审计 / 画像 churn 审计）"用 `?` 硬抛，DB 抖动即吞本轮回复；且多处代码注释自认应 fail-soft，实现却 `?`（C-01 有孪生 `let _` 铁证）。**修复统筹**：一个"审计/旁路写 fail-soft 对齐"专项，把 apply_agent_updates 内 5 处审计事件（:4411/4480/4503/4525/4551）+ reaction 步骤 (d)(e) 解耦一并处理。这类改动小、风险低、价值高（消除"DB 一抖就静默丢客户回复"），建议优先。
2. **outbox 崩溃恢复非对称家族**（F-01 + F-04）：reclaim 分支比 timeout 分支少一层权威核对、且不耗 attempt。修复=抽共用 post-hoc 核对函数（先 chat_search 再本地）+ reclaim_count 上限。

### 修复优先级建议（供用户定）
- **P1**：家族①（B-02/C-01/H-01）——改动小、根治"DB 抖动吞回复"，且可写确定性 lib 单测复现（mock write_event/reaction DB 失败，验证回复仍发）。
- **P2**：F-01（reclaim 漏查 chat_search）——重发是发送链路最不该的后果，但触发窗窄；修复=抽共用核对函数。
- **P3**：Low 类批量（D-01 serde alias、B-01/B-03 时序加固、A-01/A-02 wxid 双重身份产品裁决、E-01/F-02/F-04/H-02）——多为加固/产品裁决，择机。

### Task 9（117 真跑复现）定调
用户裁定**不在生产注故障/kill 进程复现**——批 A findings 触发前提多为"Mongo 瞬时故障 / 进程崩溃 / 精确时序 / 干预 LLM 输出"，生产无法安全构造。全部标 **PLAUSIBLE**（主控已亲验代码路径与铁证，确信度高，仅未在生产触发实证）。可复现的 finding（尤其家族① 的 mock 失败注入）**留到修复阶段写成确定性 lib 单测/小型集成测试随修复 PR 上 CI**——这比在生产 kill 进程更严谨可控。

### 审查质量说明（防假绿）
- 每条 finding 的 file:line 均主控当场 Read 亲验；subagent 凭猜/未确认的结论要么驳回（F-⑧-02）、要么降为 PLAUSIBLE。
- 2 条整环 0 finding 是真读透后的正面结论（阈值闸/MCP），非漏审。
- 未真跑，故无 CONFIRMED；这是诚实的确信度标注，非假绿。

---

# 批 B（知识链：knowledgeWiki / content / quality 三频道）

- 审查计划：[`2026-07-11-deep-logic-audit-batch-b.md`](../plans/2026-07-11-deep-logic-audit-batch-b.md)
- 视角：**业务链/命脉红线视角**（结构正确性由 2026-06-30 wiki 全覆盖审查 + PR#74 已覆盖，本批不重复结构面）。
- 7 条业务链：①录入 ②审核 ③grounding 召回 ④修复 ⑤修订 ⑥质量 ⑦catalog。优先级：链3 grounding（最高）> 链4 修复 > 链2 审核 > 链1/5/7 > 链6。
- finding 编号：`KB-NN`。全部主控当场 Read 亲验；未真跑标 PLAUSIBLE（同批 A 定调，复现留修复阶段写确定性测试）。

## 链3 grounding 召回链（最高优先 · 产品声明红线命脉）

审查范围：`routes/knowledge/chat.rs`（chat_turn/chat_apply/run_chat_with_tools/apply_create_chunk）+ `agent/knowledge_router.rs`（生产召回 load_operation_knowledge）+ `agent/knowledge_tools.rs` / `knowledge_agent.rs`（tool-calling 暴露面）+ `agent/review/gates.rs`（blocked_unverified_product_claim 硬闸）+ `agent/sufficiency.rs`（used_knowledge_ids 记录闸）。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（grounding 红线三处协同健康）

1. **✅ 所有喂 LLM 的召回入口只暴露 verified**：生产客户召回 `knowledge_router.rs:71`（exact `"verified"`）、知识对话快照 `chat.rs:1076`、tool-calling `list_catalog`(:1063)/`open_chunk`(:1206)/`open_document`(:1235)/`follow_relations`(:1333)、`resolve_superseded` 只 redirect 到 verified(:1163-1171)——逐个亲验无一漏过滤。
2. **✅ redaction 兜底**：`knowledge_tools.rs` exec_search/exec_open_slice 对非 verified 切片 snippet 置空 / body 换 `<redacted_unverified_chunk>`，即使调用方误传非 verified 集合也不泄漏正文。
3. **✅ 过期知识两侧都排除**：硬闸 `guards.rs:309 is_verified` 排除 `valid_to < now`（含单测 :798）；召回侧过期 verified 降格但因 finalize `compute_verified_chunks(now)` 仍排除，不背书产品声明。
4. **✅ 对话产出落库恒 draft+needs_review**：`apply_create_chunk:1680-1681`（强制 draft+needs_review 在 insert 前）、`apply_update_chunk:1776-1777`、chat_apply 分发(:422-472) 全链无任何路径能让对话直出 verified；溯源经共用 `resolve_quote_anchors`（D2 双路径修复）锚定。
5. **✅ 硬闸主路径依赖真读**：`gates.rs:658-690` 仅当 reviewer 自报 requiresProductKnowledge=true 触发，取 `used_knowledge_ids ∩ selected_chunks(verified)` 非空 OR priced_from_catalog，否则强制 block；`selected_chunks`(gateway.rs:1155) 是 verified-only 语料。rewrite(:1592)/revision(:1865) 两处无条件记 used_knowledge_ids，但均在 `PromptTier::Full` 重跑决策（include_business=true 真读切片），非架空。

### [KB-01] Lean/Relational 档 LLM 自报 used_knowledge_ids 未清空，可架空 blocked_unverified_product_claim 硬闸（防架空守卫不完整于其自述意图）

- 入口频道：webhook 自动回复主链路（run_user_operation_gateway_inner，非知识频道，但根因在知识 grounding 闸）
- 链路环节：③grounding 召回 → 硬闸
- 类型：红线硬闸可被架空（防御不完整）
- 严重度：**Medium**
- 现象：非 Full 档决策（Lean-Enough / Clarify(Lean) / Escalate(Relational)，三者 include_business=false，decision.rs:318 不注入任何切片正文）若 LLM 自己在输出里吐出一个真实存在于 verified 语料的 24 位 ObjectId hex，该 id 会被记进 `decision.used_knowledge_ids` 并令 grounding 硬闸 `used ∩ verified` 非空 → 放行本应 `blocked_unverified_product_claim` 的产品声明。
- 根因（亲验）：
  - `types.rs:973-975 carry_through_fields` **无条件**把 `raw.used_knowledge_ids`（LLM 原始输出）透传进 `decision.used_knowledge_ids`。
  - `gateway.rs:1457-1459` 的 `should_record_used_knowledge_ids(forced_full, escalated_to_full)` 只在 Full 档为真时**覆盖写**路由命中 id；非 Full 档走 else、**不清空**上一步透传的 LLM 自报值。
  - `sufficiency.rs:88-91` 注释明确声明的不变量是"没读切片的决策不得记 id，否则架空硬闸"——该不变量对"路由通道"守住了，对"LLM 自报通道"没守。
  - 硬闸侧 `selected_chunks`(gateway.rs:1155) 是 tier-independent 的 verified 全量语料，故非 Full 档的 `used ∩ selected` 完全可能非空。
- 验证状态：**PLAUSIBLE**（代码缺口 CONFIRMED：自报值确实不被清空，主控亲验 types.rs:973 + gateway.rs:1457 + selected_chunks:1155 三处坐实；实际可利用性依赖 LLM 恰好吐出真实 verified ObjectId——而 Lean 档 prompt 不注入切片 ID，id 不可猜，真实命中率低）。
- 修复建议：在 `gateway.rs:1457` 的 else 分支（非 Full 档）显式 `decision.used_knowledge_ids.clear()`，与 sufficiency.rs:88-91 注释意图对齐；一行改动、风险低、根治"自报通道架空硬闸"。可写确定性 lib 单测（构造 Lean 决策 + 自报一个语料内 verified id，断言硬闸仍 block）。

### [KB-02] admin PUT 经 apply_chunk_integrity 锚点命中即自动置 verified，绕过 /verify 人审端点（人类 admin 路径，非 AI 红线）

- 入口频道：knowledgeWiki 频道 chunk 编辑（PUT /api/knowledge/chunks/:id）
- 链路环节：②审核（旁路）
- 类型：verified 状态非 /verify 旁路
- 严重度：**Low-Medium**（产品裁决项）
- 现象：admin 更新一个带父文档的 chunk 时，若提交的 sourceQuote 能在父文档 raw_content 命中锚点，`apply_chunk_integrity`(mod.rs:918-921) 直接置 `integrity_status="verified"`；后续 `coerce_integrity_against_d2_gate`(mod.rs:1057) **只在缺 quote/anchor 时降级**，锚点命中则保持 verified。即：改个 summary + 附一段能命中的 quote，会把 needs_review 切片作为副作用提升到 verified，绕过专用 `/verify` 人审端点。
- 根因（亲验）：crud.rs:241-245 有父文档即调 apply_chunk_integrity（锚点→verified），coerce 只降级不拦锚点命中。**与 import 路径不对称**：import.rs:316/365/880 在 apply_chunk_integrity 之后**无条件压回** needs_review（显式红线注释），唯独 crud.rs 更新路径没有这层覆写。
- 验证状态：**PLAUSIBLE**（代码路径 CONFIRMED 可达 verified 而未走 /verify）。**边界澄清**：这是 `AuthenticatedAdmin` 鉴权端点=人类 admin 动作，**不属** CLAUDE.md「AI 永不自动 verify」红线的严格破坏（对话/AI 路径 apply_create_chunk/apply_update_chunk/import 全部硬置 needs_review，红线未破）。
- 修复建议：需用户产品裁决——(a) 若"admin 锚点命中即信任"是有意设计则保留、仅补审计；(b) 若要求所有 verified 必走 /verify，则 crud.rs 更新路径比照 import 在 apply_chunk_integrity 后压回 needs_review。倾向 (b) 收口口径一致。

### [KB-03] verified 判定口径大小写不一致（硬闸大小写不敏感 vs 召回精确匹配）——latent，当前不可触发

- 链路环节：③grounding（口径一致性）
- 类型：口径漂移（latent）
- 严重度：**Low**（信息性）
- 现象：硬闸 `guards.rs:314 is_verified` 用 `eq_ignore_ascii_case("verified")`（大小写不敏感），而**所有召回过滤**（knowledge_router.rs:71 / knowledge_agent.rs 各 tool / chat.rs:1076 / catalog）均 `== "verified"` 精确匹配。
- 根因（亲验）：写入侧 verify.rs:112 恒写小写 `"verified"`，当前不可能产生 `"Verified"`。若真出现，召回侧精确匹配失败→切片根本不进语料→不注入、不进 finalize 的 knowledge_chunks，`is_verified` 的宽松判定永远碰不到它——既不误放也不误堵。
- 验证状态：**PLAUSIBLE**（无害 latent，当前不可触发）。
- 修复建议：可选——把 is_verified 改成精确匹配 `== "verified"`，或召回侧统一大小写不敏感，消除口径漂移。不改也安全。

## 链4 修复链（次高优先 · AI 只提议不落库主集合红线）

审查范围：`routes/knowledge/repair.rs`（propose_chunk_repair_inner:218 / propose_pack_repair:576 / record_repair_apply:629）+ `knowledge_task/mod.rs`（worker execute_step 各 Phase）+ `knowledge_wiki/gap_signals.rs`（缺口信号生成/消解）+ `knowledge_wiki/structural_proposals.rs`。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（"AI 提议永不落主集合 verified"红线成立）

逐路径亲验，链4 **无任何一条**直接把 chunk 写 verified 或绕人审改动已生效切片正文：

1. **✅ propose_chunk_repair_inner**（repair.rs:218-377）：只 find_one 读 chunk/document → LLM → 写 `knowledge_usage_logs`（blocked_reason=`..._pending_operator_apply`）+ `knowledge_repair_proposed` 事件 → 返回 JSON。**全程无 operation_knowledge_chunks().update/insert**（repair.rs 三处 chunk 访问 :226/:422/:654 全是 find_one 读）。
2. **✅ propose_pack_repair**（repair.rs:576-586）：死桩返 400（operation_knowledge_items 已删）——见 KB-05。
3. **✅ record_repair_apply**（repair.rs:615-724）：纯审计端点，注释 :628「不写主业务集合」与代码一致（patch 已由前端 PUT 落库=KB-02 路径）。
4. **✅ worker execute_step 各 Phase**：`fix_chunk`(:458) 调 propose_chunk_repair_inner 拿草稿塞 turn details 供人审、不 apply；`add_chunk`(:516) 走 apply_create_chunk（硬置 draft+needs_review）；`review_evolution`(:666) 有意占位（"需人工评估，AI 不自动放量"文案，不动集合）；`analyze_logs`/`dismiss` 只读 events / 写日报 dismissed_card_ids。
5. **✅ gap_signals 只写信号旁路集合**：persist_signals/persist_recall_signal/sweep_stale_signals 只写 `knowledge_gap_signals`；唯一碰主 chunk 的 refresh_usage_stats_and_confidence(:1034)/record_chunk_hit(:1186) 只写 usage_stats/dynamic_confidence/updated_at 统计字段，**从不碰 status/integrity_status/body**（模块头 :8-9 自述不变量成立）。
6. **✅ structural_proposals 只产 pending_review**：propose_structural_change 只 insert `status="pending_review"` 到旁路集合，序列化层物理无 apply/commit 字段——见 KB-06（无消费方=就绪债）。

### [KB-04] worker `retag` 步骤经 apply_update_chunk 把 verified 切片静默降级为 draft/needs_review（人工派工路径，反向红线 UX 意外）

- 入口频道：content/知识对话收件箱「派工」→ retag step
- 链路环节：④修复（worker execute_step）
- 类型：AI 自动改动已生效切片状态（反向：un-verify）/ UX 语义意外
- 严重度：**Low-Medium**
- 现象：worker 的 `retag` step（只想重抽 productTags/businessTopics，不动事实正文）复用 `apply_update_chunk`，而后者对**任何** patch 无条件写 `status="draft"` + `integrity_status="needs_review"`（chat.rs:1776-1777，retag 调用点 knowledge_task/mod.rs:643-648 注释亦自认）。若目标 chunk 当前 verified，一次重抽标签会把它踢回草稿态、退出可召回 verified 集合。
- 根因（亲验）：apply_update_chunk 的"永不 verify"降级本为运营改正文设计（改正文理应重审）；retag 借道它，把"仅改标签元字段"也拖入降级。
- 触发链（亲验，决定严重度）：**非 AI 自主**——retag step 由 admin 在收件箱审阅 AI 建议的 plannedSteps 后显式「派工」（`POST /api/knowledge/chat/tasks` = `chat_task_create` chat.rs:1882 `AuthenticatedAdmin` 门），落 `knowledge_chat_tasks{status=pending}`，再由 worker `tick_once`(mod.rs:165) 串行异步执行。故 AI 从不自主 un-verify，是人工派工触发；**不违反** CLAUDE.md「AI 永不自动 verify」红线（该红线管"不自动转 verified"，此处是反向 un-verify 且有人工闸）。
- 验证状态：**PLAUSIBLE**（代码副作用 CONFIRMED：retag→apply_update_chunk→强制降级三处亲验坐实；实际影响=admin 对 verified 切片派 retag 时，本意"补标签"却导致该切片退出生效池且需重新人审）。
- 修复建议：需用户产品裁决"重抽标签是否应触发重审"——倾向：retag 不走强制降级的 apply_update_chunk，或给 apply_update_chunk 加「仅元字段变更（标签）不降级 integrity」开关；至少 retag 判定 target 已 verified 时仅写候选标签、不动 integrity_status。

### [KB-05] propose_pack_repair 死桩返 400 但路由仍注册，前端调用即失败（就绪债/死代码误导）

- 入口频道：content 频道 pack 级修复（POST /api/operation-knowledge/packs/:id/repair）
- 链路环节：④修复
- 类型：死代码/路由误导
- 严重度：**Low**
- 现象：propose_pack_repair（repair.rs:576-586）已下线（operation_knowledge_items 集合已删），恒返 400 "pack repair temporarily disabled"，但路由仍在 mod.rs:682 注册。前端若调用 pack 修复即得 400。
- 根因（亲验）：注释 :580-581「等 wiki Phase 重新规划包级别 repair」——有意下线但路由未摘。
- 验证状态：**PLAUSIBLE**（死桩 CONFIRMED；是否有前端真调此路由未核，故严重度取决于前端是否暴露入口）。
- 修复建议：要么摘掉路由注册（mod.rs:682）避免误导，要么前端隐藏 pack 修复入口；属就绪债，择机。

### [KB-06] structural_proposals 只产 pending_review 无任何 apply/人审消费方，提案永久躺集合（就绪债）

- 链路环节：④修复（结构化 split/merge 提议）
- 类型：就绪债（功能半实装）
- 严重度：**Low**
- 现象：propose_structural_change 只 insert `status="pending_review"` 到 `structural_proposals` 集合，全仓 grep 无任何 apply worker / 人审 UI 消费方——提案产出后无落库/人审出口，纯躺集合。
- 根因（亲验）：模块注释 structural_proposals.rs:1-14 自述"下一轮 out-of-scope"；序列化层物理无 apply/commit/delete 字段（测试 :173-190 锁死）——这是红线**正确**的一面（AI 绝不自动 apply split/merge），但也意味功能未闭环。
- 验证状态：**PLAUSIBLE**（无消费方 CONFIRMED）。
- 修复建议：要么补人审 apply UI 闭环、要么文档明确标注该功能未接线；不作为 bug，登记就绪债。

### [KB-07] gap_signals 在线 recall 信号 dedup 用无序 find_one 单条 + 内存过滤，同 kind 多主题信号可能漏合并（逻辑不严谨）

- 入口频道：召回热路径（每次 recall_miss 拦截写信号）
- 链路环节：④修复（gap 信号去重）
- 类型：业务逻辑正确性（去重不严谨）
- 严重度：**Low**
- 现象：`persist_recall_signal`（gap_signals.rs:602-610）先 `find_one({workspace_id, status:pending, kind})`（**无 sort、无 dedup_key 过滤**），再对返回的**单条**用 `.filter(dedup_key==key)`。若同一 kind（如 recall_miss）下已有多条不同主题的 pending 信号，find_one 返回"任意一条"很可能不匹配 → filter 掉 → insert 新建，导致**同主题信号漏合并**（无法累积 search_queries 变体、产生重复条目稀释信号）。
- 根因（亲验）：对比离线 `persist_signals`（:499-503）把**全部** pending 载入 map 按 key 精确比对（正确）；在线路径为省一次全量查询走 find_one，牺牲了正确性。
- 验证状态：**PLAUSIBLE**（逻辑不严谨 CONFIRMED：find_one 无序 + 内存过滤坐实；实际漏合并频率依赖 Mongo 返回顺序，需运行时估）。有 signal_id 唯一索引（indexes.rs:1386）兜底，不写坏数据，仅噪音。
- 修复建议：在线 dedup 改为可精确命中的查询键（持久化 dedup_key/normalized_title 字段并加索引直接 find_one 命中），与离线 persist_signals 精确比对对齐。

## 链2 审核链（第三优先 · auto-verify 红线最大对抗面）

审查范围：`routes/knowledge/verify.rs`（auto_verify_inner:253 / decide_auto_verify_status:527 / enforce_verified_needs_human_audit:554 / clamp_sample_rate:38 / 人工 verify:66 / reject:126）+ `wiki_edit.rs` batch_verify_chunks:934 + `knowledge_wiki/chunk_revisions.rs` apply_chunk_revision。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（"AI 永不自动 verify"红线极稳固，无绕过）

1. **✅ auto_verify 写库必经强制降级**：verify.rs:391 算 decide_auto_verify_status → :401 **无条件**套 enforce_verified_needs_human_audit → :418/:426 apply_chunk_revision 用的就是这个 final_status。verified→needs_human_audit 写库前已定死，无旁路；:404 `"verified"=>verified+=1` 是**死分支**（红线正确，非 bug）。
2. **✅ product_fact 无特权也无边界漏洞**：decide_auto_verify_status / enforce_verified_needs_human_audit 完全不读 chunk_type，所有类型一视同仁强制降级（含单测 all_types_verified_forced_to_human_audit）。
3. **✅ LLM 输出 integrityStatus 不透传**：model_status 只作 decide_auto_verify_status 入参（须 =="verified" 且证据+置信齐），LLM 自称 verified 也必降级；未知/脏值回落 needs_review（:544）。
4. **✅ apply_chunk_revision Verify op 不自写 verified**：integrity_status 由调用方 patch 决定；AI source 强制 draft+needs_review，仅 Human source 才允许 verified。
5. **✅ 人工 /verify + batch_verify 鉴权+隔离+D2 闸齐全**：均 `AuthenticatedAdmin` + require_session；先 find_one 带 `workspace_id=current_workspace`（跨 ws 返 NotFound）+ apply_chunk_revision 二次 workspace 守门（chunk_revisions.rs:159-165）；均先过 D2 闸 chunk_verify_gate_reason（无 quote/anchor 不得 verify）。
6. **✅ batch_verify 部分失败正确**：wiki_edit.rs:947-1008 逐条 try，parse/not_found/db/gate/apply 失败各自 push skipped，不静默吞、不整批标成功；空/超100 提前 400。
7. **✅ needs_human_audit 绝不被误召回 verified**：所有召回/背书过滤精确 `=="verified"`（guards.rs:314 / knowledge_router.rs:71 / knowledge_agent 各 tool / knowledge_tools.rs），needs_human_audit 既不满足召回也不吃 +0.5 加分。
8. **✅ clamp_sample_rate 硬下限**：`.clamp(0.05,1.0)`，传 0 也钳 0.05（单测锁死）。
9. **✅ 幂等**：重复 verify 已 verified 切片 → apply_chunk_revision before/after hash 相同跳过 replace_one，仅多一条 revision 审计，无副作用。

### [KB-08] auto_verify 降级出的 needs_human_audit 切片不进任何审核收件箱，人审漏斗黑洞（分诊反使待审切片从收件箱消失）

- 入口频道：content 频道 auto_verify 批处理 → 人审收件箱（quality/知识对话收件箱）
- 链路环节：②审核（人审漏斗衔接）
- 类型：业务流缺陷（状态机新增态无消费方）
- 严重度：**Medium**
- 现象：auto_verify 对过闸切片强制写 `integrity_status="needs_human_audit"`（verify.rs:401/:426），设计意图是"预审分诊，过闸的挑出来等运营重点看"。但**没有任何审核入口查询这个状态**：
  - 统一收件箱 `collect_knowledge_review` 硬编码 `integrity_status:"needs_review"`（ask_human_inbox.rs:124），不认 needs_human_audit。
  - AI digest 收件箱三分支（digest_inbox.rs:431/465/481）：分支1要 needs_review 不匹配；分支2/3要缺 quote/缺 anchor，而 needs_human_audit 必由「verified 判定」降级来（decide_auto_verify_status:534 要求 quote+anchor 都真），故必然同时有 quote+anchor，两分支也不匹配。
  - 全仓 needs_human_audit 只出现在 verify.rs 写入端 + 前端**仅作计数展示**（AutoVerifyPanel.tsx:170 / quality/index.tsx:238「待人工抽查」数字），**无任何过滤视图列出这些切片供审**。
- 根因（亲验）：auto_verify 输入过滤是 `integrity_status ∈ {needs_review, null}`（verify.rs:270）——它把原本以 needs_review 显示在收件箱的切片，改写成 needs_human_audit 后**从收件箱移除**。分诊动作反而让待审切片从人审界面消失，只剩一个计数。
- 验证状态：**PLAUSIBLE**（写端 CONFIRMED verify.rs:270 输入含 needs_review + :426 写 needs_human_audit；读端 CONFIRMED ask_human_inbox.rs:124 只查 needs_review + 前端 grep 仅计数无过滤视图；主控亲验后端+前端两侧坐实）。红线字面未破（切片没被自动 verified、也不会被误召回背书——精确 `=="verified"` 已核），但人审漏斗有洞：needs_human_audit 切片无限期停留、无入口晋升 verified（仅能靠 chunk 全量列表浏览看到，故 Medium 非 High）。
- 修复建议：让审核收件箱查询 `integrity_status ∈ {needs_review, needs_human_audit}`（ask_human_inbox.rs:124 + digest_inbox.rs:431 同步），并给 needs_human_audit 一个"AI 预审通过待人复核"标签/优先级；或前端加 needs_human_audit 专用过滤视图。**这是本批迄今最有业务价值的 finding**——正是上一轮广度走查扫不到的"状态机新增态与消费方脱节"。

## 链1 录入 + 链5 修订 + 链7 catalog（合并 · 结构较稳）

审查范围：链1 `routes/knowledge/import.rs` + `media_assets.rs` + `knowledge_wiki/ingest_worker.rs`；链5 `routes/knowledge/chat.rs`(apply_update_chunk) + `crud.rs` + `knowledge_wiki/chunk_revisions.rs`(apply_chunk_revision) + `page_merge.rs`；链7 `routes/knowledge/catalog.rs`。主控派 opus subagent 复审链5 + 逐条亲验。

### ✅ 亲验通过总览

1. **✅ 链1 录入恒 draft+needs_review**：import 全入口（apply :315-316 / 流式 :362-365 / pdf :824-825 / image :872-880）在 apply_chunk_integrity 之后**无条件压回** draft+needs_review（红线注释在位）；ingest_worker 模块头 :9 自述同红线。**唯一非 /verify 直 verified 旁路是 KB-02**（crud PUT admin 路径，已账）。
2. **✅ 链7 catalog completeness 缓存 key 维度充分**：缓存 key = `(workspace_id, account_id)`（catalog.rs:114/132/150）——含双维度，account A **不会**看到 account B 的 completeness。Explore 提示的"跨账号串味"风险**证伪**。F-013 TTL 缓存本身挡慢查询已确认。
3. **✅ apply_chunk_revision 双写次序正确**：先 chunk_revisions.insert_one（:271）后 chunks.replace_one（:277）；before/after hash 用 compute_chunk_hash 剔 volatile 字段；workspace 隔离 find(:158)/replace(:278) 两侧一致；AI source 强制 draft+needs_review；末次 enforce_locked_fields（:234）。
4. **✅ page_merge 三层函数逻辑正确**：apply_field_patch 前置 reject 锁定字段（:190）、union_array_fields existing∪incoming 保序去重（:94）、is_body_truncated 70% 阈值 existing=0 短路、enforce_locked_fields 强制覆盖回 existing（:140）。
5. **✅ crud delete 无越权**：document delete / 级联 delete_many / chunk delete_one 三处 filter 均带 workspace_id，隔离健全无 IDOR。
6. **✅ crud PUT 未建模字段回填**：preserve_unmodeled_chunk_fields（mod.rs:532-551）正确回填 provenance/wiki_type/locked_fields/created_at（有守卫测试 chunk_put_preserves_unmodeled_fields）。

### [KB-09] apply_update_chunk（AI 会话应用草稿）直改主集合内容+状态，不写 chunk_revisions 审计，且数组字段整体替换非并集（旁路 apply_chunk_revision）

- 入口频道：知识对话「应用草稿」→ update_chunk 分支（chat.rs:465）
- 链路环节：⑤修订
- 类型：审计链断 + 旁路统一编辑入口
- 严重度：**Medium**
- 现象：`apply_update_chunk`（chat.rs:1711-1790）把 title/summary/routing_card/applicable_scenes/source_quote 等内容字段 + status=draft/integrity=needs_review 直接 `$set` update_one 写库（:1779-1790），**全函数无任何 chunk_revisions 写入**（对比 auto_verify 已明确接回 apply_chunk_revision）。
- 根因（亲验）：该路径独立实现 $set patch，未接回 apply_chunk_revision。
- 后果：(1) 无 before/after hash、无 revision 行 → 审计链断（无法回滚/追溯）；(2) applicable_scenes/product_tags/business_topics 是 `$set` **整体替换**而非 union_array_fields 并集 → 运营既有 tag 可能被 AI 补丁悄悄丢弃（apply_chunk_revision 会 union，这里不会）；(3) 不读 locked_fields（见 KB-11）。**注：status 被强制 draft+needs_review，未破"AI 永不自动 verify"红线**；workspace 隔离在位。
- 验证状态：**CONFIRMED**（主控亲验 chat.rs:1779-1790 update_one + 全函数无 revision 调用坐实）。
- 修复建议：apply_update_chunk 落库改构造 `RevisionRequest{op:Patch}` 调 apply_chunk_revision，天然获审计行+截断守卫+数组 union+锁字段守门。

### [KB-10] crud.rs admin PUT 用 replace_one 整条替换主集合，不写 chunk_revisions 审计

- 入口频道：knowledgeWiki 频道 chunk 编辑（PUT /api/knowledge/chunks/:id）
- 链路环节：⑤修订
- 类型：审计链断
- 严重度：**Medium**
- 现象：`update_operation_knowledge_chunk`（crud.rs:212-279）用 replace_one 整条替换 chunk（:269-277），**无 chunk_revisions 写入**。整条 replace 覆盖 title/summary/body 等内容却不留 revision 审计。
- 根因（亲验）：admin 直编端点独立实现，未接回 apply_chunk_revision。
- 后果：审计链断（违"所有主集合内容编辑留不可变审计"）。**workspace 隔离健全**（filter 带 workspace_id find :256 + replace :271 一致，无 IDOR）；preserve_unmodeled_chunk_fields 正确回填元字段。唯一缺口=审计行缺失 + locked_fields 不强制（KB-11）。此路径也是 KB-02（锚点→verified）的同一 replace_one。
- 验证状态：**CONFIRMED**（主控亲验 crud.rs:266-277 replace_one + 全函数无 revision 坐实）。
- 修复建议：PUT 落库改走 apply_chunk_revision（op=Patch, source=Human），或至少 replace_one 前后补写一条 chunk_revisions 记 before/after hash。

### [KB-11] 运营 per-chunk locked_fields 后端从不强制，仅前端禁用 + 静态 DEFAULT 8 字段受保护（字段锁形同虚设于后端）

- 链路环节：⑤修订（字段锁保护）
- 类型：设计红线未兑现（后端无强制）
- 严重度：**Medium**（是否算缺陷取决于 locked_fields 设计定位）
- 现象：设计要求 apply_chunk_revision "尊重 chunk 的 locked_fields（运营锁定字段不被覆盖）"。实际 `enforce_locked_fields(&merged, &existing, DEFAULT_LOCKED_FIELDS)`（chunk_revisions.rs:234）传的是**编译期常量** DEFAULT_LOCKED_FIELDS（page_merge.rs:35-44，仅 8 个身份/时间戳字段 chunk_id/wiki_type/chunk_type/created_at/source_anchor/verified_at/verified_by/approved_at）；对 `existing.locked_fields`（运营在单条 chunk 上标注的字段锁，models.rs:1538）**无任何读取/强制点**。
- 根因（亲验）：全仓 grep locked_fields 非测试消费点——只有 model 定义、preserve_unmodeled_chunk_fields 载体回填（mod.rs:546）、前端序列化（mod.rs:307）、wiki_edit 拒收入参（:39）、domain_schemas 黑名单——**无任何 guard 读 existing.locked_fields 去强制**。运营把 title/body 加进 locked_fields 后，apply_chunk_revision 的 patch 改 title/body 照样通过（DEFAULT 集不含它们），crud PUT replace_one 更直接覆盖。字段锁当前只在前端编辑表单禁用输入（设计文档 knowledge-trust-cockpit-frontend-design.md:71），后端无兜底。
- 验证状态：**CONFIRMED**（主控亲验 chunk_revisions.rs:234 传常量 + 全仓无 existing.locked_fields 强制点坐实）。
- 修复建议：需用户裁决 locked_fields 是"后端强制"还是"纯前端提示"——若前者（按设计描述应是），apply_chunk_revision 里把 existing.locked_fields 与 DEFAULT_LOCKED_FIELDS 合并后再 apply_field_patch + enforce_locked_fields；PUT 路径同理回填 existing 锁定字段值。KB-09/KB-10/KB-11 构成"统一编辑入口未真统一 + 字段锁未兑现"同一根因家族。

## 链6 质量链（outcome 度量 / reviewer 统计 / 成交追认）

审查范围：`routes/outcome_metrics.rs` + `routes/outcomes_autonomy.rs`（get_autonomy_outcomes:219 / list_autonomy_revisions:443）+ `knowledge_wiki/reviewer_stats.rs` + `agent/entitlements.rs`（staff_confirmed 闭集）+ 写侧 `routes/shared.rs`（add_outcome_event）/ `admin_suspected_deals.rs`。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（"AI 永不自证成交"红线干净，无 CONFIRMED bug）

1. **✅ AI 永不自证成交（核心红线）**：`verification_drives_entitlement`（entitlements.rs:51-53）闭集 `{staff_confirmed, payment_verified}`，conversation_inferred 物理排除；三消费点（project_entitlements/confirmed_deal_timestamps/compute_customer_value_cents）全复用该闭集。
2. **✅ 写侧闭集严密**：outcome_events 唯一写入点 add_outcome_event_inner（shared.rs）强制走 `validate_deal_verification`（shared.rs:1410-1419）——**主控亲验**：None/空/staff_confirmed→staff_confirmed，payment_verified→payment_verified，**其它一律 BadRequest 拒绝**（含 conversation_inferred，错误文案明说"疑似线索须经核实"）。AI 疑似成交无法经任何路径写 outcome_events。
3. **✅ 成交追认必须人工触发**：conversation_inferred→staff_confirmed 唯一升级点 approve_suspected_deal（admin_suspected_deals.rs:119）**主控亲验** `Extension<AuthenticatedAdmin>` 门 + verification 硬编码 `"staff_confirmed"`（:203）；无 worker/AI 调用。AI 侧只 upsert suspected_deal_signals（status=pending 独立集合，gateway.rs:4346），从不碰 outcome_events，全程 fail-soft。
4. **✅ misjudge_rate 分子⊆分母**：reviewer_misjudge_signal="approved_but_user_negative" 仅在 reviewer_approved==true 时产出（reaction.rs:173 `&&` 守卫），误判分子恒⊆approved 分母，misjudge_rate≤1 不虚高。
5. **✅ ratio 除零保护**：reviewer_stats.rs:36-45 分母 0 返 0.0；outcomes_autonomy.rs:122 分母 0 返 null；held 类比率分子⊆分母恒≤1。
6. **✅ 度量口径**：outcome_metrics 与 outcomes_autonomy 均 (workspace_id, account_id) 双维过滤；outcomes_autonomy final_review_status 白名单剔 legacy 脏值。

### [KB-12] reviewer_stats 只按 workspace 聚合、缺 account_id 维度，与 outcome_metrics/outcomes_autonomy 双维口径不一致（就绪债）

- 链路环节：⑥质量（reviewer 度量聚合）
- 类型：口径不一致（多租户就绪债）
- 严重度：**Low**（就绪债）
- 现象：`aggregate_reviewer_stats_for_workspace`（reviewer_stats.rs:49-91）三条 count_documents 过滤只含 `workspace_id + created_at`，聚合行 stat_id=`"{workspace_id}::reviewer"`（一 workspace 一行）；而 outcome_metrics.rs:38 与 outcomes_autonomy.rs:101 都按 `(workspace_id, account_id)` 双维。
- 根因（亲验）：reviewer 度量刻意做成 workspace 级（reviewer 的 prompt/model 是 workspace 级），但 AgentDecisionReview 写侧其实带 account_id（models.rs:2635），数据支持按账号切却没切。
- 验证状态：**PLAUSIBLE**（就绪债）。单 workspace 挂多账号时 reviewer pass_rate/misjudge_rate 会混算；多租户默认关、常见"一 workspace 一账号"部署无实际串数据，且 reviewer 准确率作为 workspace 级属性语义基本成立。故记就绪债非 bug。
- 修复建议：若未来一 workspace 多账号成常态，stat_id 与过滤加 account_id 维度对齐另两端点；否则维持现状 + 文档写明"reviewer 度量是 workspace 级"。

---

# 批 B 总评（知识链 · 审查收口）

**审查方式**：7 条业务链按优先级组织（链3 grounding 最高 > 链4 修复 > 链2 审核 > 链1/5/7 > 链6），5 个 task 逐链读码 + 每链派 opus subagent 只读复审 + **主控逐条亲验 file:line**（subagent 结论经亲验后入账，无一凭猜采信）。审查阶段只入账不改 src。

### finding 计数（去重后，全部主控亲验）
- **Critical 0 / High 0**
- **Medium 5**：
  - **KB-01**（链3）Lean/Relational 档 LLM 自报 used_knowledge_ids 未清空，可架空 blocked_unverified_product_claim 硬闸（types.rs:973 透传 + gateway.rs:1457 只 Full 档覆盖不清空 + selected_chunks:1155 verified 全量语料，三处坐实；exploit 需 LLM 猜中真实 ObjectId 故 PLAUSIBLE）
  - **KB-08**（链2）auto_verify 降级出的 needs_human_audit 切片不进任何审核收件箱（verify.rs:270 输入含 needs_review→:426 写 needs_human_audit 从收件箱移除；ask_human_inbox.rs:124 只查 needs_review；前端仅计数无过滤视图）——**本批最有业务价值**，红线字面未破但人审漏斗有洞
  - **KB-09**（链5）apply_update_chunk 直改主集合内容+状态不写 revision 审计，数组 $set 整体替换非 union 可丢运营 tag（chat.rs:1779）
  - **KB-10**（链5）crud PUT replace_one 整条替换不写 revision 审计（crud.rs:269；workspace 隔离健全）
  - **KB-11**（链5）per-chunk locked_fields 后端从不强制，enforce_locked_fields 只传 DEFAULT 常量 8 字段，existing.locked_fields 仅前端禁用无 guard 读（chunk_revisions.rs:234）
- **Low 4**：KB-02（admin PUT 锚点→verified 绕 /verify，人类路径非 AI 红线）、KB-03（is_verified 大小写口径 latent）、KB-05（propose_pack_repair 死桩路由仍注册）、KB-06（structural_proposals 无 apply 消费方就绪债）、KB-07（gap_signals 在线 dedup 无序 find_one 漏合并）、KB-12（reviewer_stats 缺 account 维度就绪债）、KB-04（worker retag 降级 verified，人工派工触发反向 UX 意外）
  - 注：KB-02/KB-04 定 Low-Medium 边界，修复优先级归 P3。

### 跨链根因家族（修复统筹）
1. **知识编辑审计/统一入口家族**（KB-09 + KB-10 + KB-11，链5）：**本批最系统性发现**。apply_chunk_revision 本应是唯一编辑落库入口（留不可变审计 + 数组 union + 锁字段守门），但两条内容编辑路径（会话应用草稿 apply_update_chunk / admin PUT replace_one）**绕过它直改主集合**，且 per-chunk locked_fields 后端从不强制。共性="设计声称统一，实现有旁路"。修复统筹：一个"知识编辑统一接回 apply_chunk_revision + locked_fields 后端强制"专项，把 apply_update_chunk 与 crud PUT 都改走 revision（获审计+union+锁字段），并把 existing.locked_fields 并入 enforce_locked_fields。改动中等、价值高（补审计链 + 兑现字段锁）。
2. **人审漏斗衔接家族**（KB-08，链2 独立但高价值）：auto_verify 引入第三态 needs_human_audit 却无收件箱消费方。修复=收件箱查询纳入 needs_human_audit。
3. **grounding 硬闸防架空补全**（KB-01，链3 独立）：一行 clear() 补上自报通道。

### 修复优先级建议（供用户定）
- **P1**：KB-08（人审漏斗黑洞，收件箱查询加 needs_human_audit，改动小价值高）+ KB-01（grounding 硬闸 else 分支 clear()，一行根治）。
- **P2**：家族①（KB-09/10/11，知识编辑统一接回 apply_chunk_revision + locked_fields 后端强制）——改动中等，补审计链 + 兑现字段锁；需先与用户确认 locked_fields 设计定位（后端强制 vs 前端提示）。
- **P3**：Low 批量（KB-02 产品裁决、KB-04 retag 语义裁决、KB-03/KB-05/KB-06/KB-07/KB-12 就绪债/latent/死代码），择机。

### 与批 A 的关联
- 批 A 最系统性家族=①"审计/旁路事件误用 `?` 吞回复"（错误处理层）；批 B 最系统性家族=①"知识编辑绕过统一 revision 入口 + 字段锁未兑现"（数据写入审计层）。两批共性=**"设计声称的不变量，实现层有旁路/缺口"**——上一轮广度走查（前端点页面 + 抽验）扫不到的"看不见的层"，正是本轮逐行读码 + 主控亲验的价值所在。
- 批 A 红线（自动回复命脉）与批 B 红线（AI 永不自动 verify / 产品声明须 verified 背书 / AI 永不自证成交）**核心防线均亲验成立**——findings 是"防护不完整/衔接有洞/审计链断"，非"红线被突破"。

### 审查质量说明（防假绿）
- 每条 finding 的 file:line 均主控当场 Read/Grep 亲验；subagent 4 次复审共产出的候选，经主控亲验后全部坐实入账（KB-01 补验 selected_chunks tier-independent、KB-04 补验人工派工触发链、KB-08 补验前端仅计数无视图、KB-11 补验全仓无 existing.locked_fields 强制点）。
- 5 条链 red-line 正面结论（grounding 三处协同 / auto-verify 强制降级 / AI 提议不落库 / AI 永不自证成交 / 录入恒 draft）是真读透后的结论，非漏审。
- 未真跑（同批 A 定调），故无 CONFIRMED 运行时复现；KB-09/KB-10/KB-11 是代码路径 CONFIRMED（静态确凿），其余 PLAUSIBLE。复现留修复阶段写确定性 lib 单测随修复 PR 上 CI。

---

# 批 C（成交活动链：campaign / productsDeals / sendAnalytics）

- 审查计划：[`2026-07-11-deep-logic-audit-batch-c.md`](../plans/2026-07-11-deep-logic-audit-batch-c.md)
- 业务流四环：圈人（audience 筛选）→ 触达（批量扇出 follow_up）→ 成交登记（deal 录入）→ 成效聚合（analytics）。
- 与批 B 链6 不重复：add_outcome_event_inner / approve_suspected_deal / entitlements 闭集已在批 B 亲验红线成立，批 C 审**上游入口衔接 + 触达/圈人 + 成效聚合口径**。
- finding 编号 `KC-NN`。全部主控当场 Read/Grep 亲验；未真跑标 PLAUSIBLE（同批 A/B 定调）。

## 触达环 dispatch_campaign（最高优先 · 多步非事务写健壮性）

审查范围：`routes/campaigns.rs`（dispatch_campaign:289 / build_campaign_follow_up_task:127 / is_duplicate_key:170 / classify_send_outcome:395 / campaign_sends_report:492）+ `management.rs`（dispatch_campaign 工具入口）。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览

1. **✅ 触达不直连 MCP、复用统一发送网关**：build_campaign_follow_up_task(:127) 造 `kind="follow_up"` + `review_required=true` + `run_at=now` 标准 task，交 task worker → gateway → outbox → MCP。活动消息受同一批安全闸（cooldown/日上限/managed 门）约束，不绕过（主控亲验 task 形态）。
2. **✅ 活动级去重靠唯一索引**：campaign_sends (campaignId, contactWxid) unique index（indexes.rs:748），DuplicateKey→跳过。
3. **✅ dispatchedCount 语义正确**：只在 send 插入成功+task 建成+回填完成后 +1（:367），去重跳过（:369）/错误（:370）都不计。
4. **✅ classify_send_outcome 无虚报 sent**：outbox_status=="sent" 最高优先（:410），无任何 run_status 分支误落 sent，未识别 status 诚实归 unknown。
5. **✅ MCP 工具入口强制确认**：dispatch_campaign 经 management tool 恒 tool_always_requires_confirmation（高风险须确认）+ AuthenticatedAdmin + workspace 隔离。

### [KC-01] 孤儿 send 永久漏推：先占去重位再建 task，中间失败留下"有 send 无 task"，重发被去重索引挡死

- 入口频道：campaign 频道触达（POST /campaigns/:id/dispatch）
- 链路环节：触达
- 类型：多步非事务写 / 静默永久漏消息
- 严重度：**Medium**（后果严重=客户永久漏推，但触发=循环中途 Mongo 瞬时写错误，与批 A/B DB-fault 触发类 finding 同级；无运行时复现故 PLAUSIBLE）
- 现象：dispatch 循环里 `campaign_sends().insert_one` 成功（:341，占了 (campaignId,contactWxid) 去重位）后，`tasks().insert_one(&task).await?`（:351）的 `?` 若失败 → 整个 handler return Err。留下 `status="enqueued"` + `task_id=None` 的 campaign_send，但**无对应 task**。
- 根因（亲验）：先占去重位、后建 task，两步非原子；中间失败无补偿（无 delete send 回滚、无 worker 兜底建 task——主控 grep tasks.rs/knowledge_task/agent 零命中，确认无对账）。
- 后果（亲验）：重新 dispatch 时该 contact insert 撞 DuplicateKey（:369）被静默跳过 → task 永远建不出来 → 客户永久收不到活动消息。report 侧 `s.task_id=None`（:522 filter_map 丢弃）→ classify_send_outcome 走 run_log=None → 永远归 `("pending","not_yet_run")`，运营看到"待跑"假象，永不暴露为失败。
- 验证状态：**PLAUSIBLE**（孤儿位一旦形成、重入必跳过、永久漏推的逻辑链 CONFIRMED；触发需循环中途非 DuplicateKey 写错误，概率需运行时估）。
- 修复建议：调换顺序（先建 task 再插 send 带 taskId，让 task 成可重放源）；或 report 识别 `enqueued && task_id=None && 超时` 孤儿单列一桶；或提供孤儿 send 补建 task 入口。

### [KC-02] 部分失败中断整批、campaign 永久卡 dispatching 无恢复；且 dispatch 无 status 前置门可重复推送

- 入口频道：campaign 频道触达
- 链路环节：触达
- 类型：批量写无 checkpoint / 状态机悬空
- 严重度：**Medium**
- 现象：循环中任一 contact 非 DuplicateKey 错误 → `Err(e)=>return Err`（:370）立即中断 → 循环后的 completed update（:373-382）不执行 → campaign 永久停 `status="dispatching"`。前面已建 send+task、后面完全没建。**无任何 worker 扫 dispatching 态恢复**（主控亲验）。
- 附加（亲验）：dispatch 前无 status 前置校验（:304-316 只查存在+圈人，不校验当前 status）→ `completed` 活动可被反复 dispatch（每次重新圈人、对新命中的人建 task）。
- 根因（亲验）：批量写无 checkpoint/无幂等重入设计；status 机乐观直推（dispatching→completed），中断态无回收。重入虽可自愈"没建的继续建"，但被 KC-01 孤儿位反噬（中断在"send 已插 task 未建"窗口时，重入误当已推跳过）。
- 验证状态：**PLAUSIBLE**（中断卡 dispatching + 无恢复 + 无 status 门 CONFIRMED；触发需循环中途写错误）。
- 修复建议：dispatching 态可重入 + 孤儿位可自愈（配合 KC-01）；或失败落 status=canceled 而非悬空；补 status 前置门（仅 previewed/confirmed 可 dispatch，防重复推送）。

### [KC-03] taskId 回填失败 → task 会发但成效报表显示 pending（成效统计虚低）

- 入口频道：campaign 频道触达
- 链路环节：触达 / 成效
- 类型：多步非事务写 / 报表失真
- 严重度：**Low-Medium**
- 现象：task 已 insert 成功（:351），但回填 taskId 的 update_one（:365）`?` 失败 → return Err 中断整批（同 KC-02 卡 dispatching），且该 send.task_id 停 None，但 task 实际存在、worker 会正常跑真发消息。
- 根因（亲验）：task_id 是 report join 唯一关联键（:520-523 filter_map），回填与建 task 非原子。
- 后果（亲验）：report join `s.task_id=None` → run_doc=None → 归 pending/not_yet_run。**消息真发了，但活动报表显示"待跑"**，成效统计虚低。比 KC-01 轻（消息没丢）。
- 验证状态：**PLAUSIBLE**。
- 修复建议：建 task 时带上预生成 send 关联，或反向让 task 存 campaignId、report 从 task 侧 join，消除回填这步。

### [KC-04] 触达规模无上限：大受众单 HTTP 请求内串行建数千 task（超时/内存/task 洪峰）

- 入口频道：campaign 频道触达（也经 management tool 被 Agent 触发）
- 链路环节：圈人+触达
- 类型：规模/性能（无上限保护）
- 严重度：**Low-Medium**
- 现象：resolve_segment_contacts 的 cursor 无 limit（campaigns.rs:185），全量 collect 进 Vec（:191）；dispatch 循环对每个 hit 顺序 await 三次 DB 写。几千上万 contact 时单 HTTP 请求内串行几千~上万次往返。
- 根因（亲验）：无分页、无批量写（insert_many）、无受众上限。对比 list_campaigns 不分页是因"活动数量本身有限"，但受众规模无此天然上界。
- 后果：单请求耗时随受众线性增长 → HTTP/反代超时（超时后客户端重试叠加 KC-02 卡死）；contacts 全量驻内存；task worker 洪峰。
- 验证状态：**PLAUSIBLE**（无上限保护 CONFIRMED；实际超时/内存崩溃点需压测）。
- 修复建议：受众硬上限 + 超限拒绝或分批；insert_many 批量建 task；或 dispatch 只落"待扇出"标记由后台 worker 分批推进（同时解决 KC-02 恢复）。

## 圈人环 audience 筛选（两阶段：Mongo 粗筛 + 内存精筛）

审查范围：`routes/campaigns.rs`（build_segment_coarse_filter:31 / contact_matches_segment:61 / resolve_segment_contacts:178 / preview_campaign:236）+ `agent/entitlements.rs`（project_entitlements 净持有）。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览

1. **✅ 净持有退款抵消正确**：project_entitlements 按 product_id 聚合、reversal 反号累减、净件数≤0 剔除（entitlements.rs:86-125）；full/partial/over-reversal 三测覆盖。
2. **✅ 售后窗 + value_tier 与 gateway G6 同源**：in_aftercare 语义（Some(true)期内/Some(false)过期/None 无规则）与精筛一致；value_tier 用同函数同 config 阈值（campaigns.rs:162 ↔ gateway.rs:4070），无漂移。
3. **✅ conversation_inferred 不进筛选**：verification_drives_entitlement 闭集只认 staff_confirmed/payment_verified（entitlements.rs:51），精筛与 LTV 均排除 AI 疑似成交，红线守住。
4. **✅ workspace/account 隔离**：粗筛固定 workspace_id+account_id+managed（:37-39）；account_id 缺省回落 default_account_id 是更严收窄，不串别账号 contact。
5. **✅ 退款场景粗筛/精筛正确方向**：买后全额退款——粗筛靠原 deal 事件仍命中（宽），精筛净持有=0 排除（严），superset→narrow 方向正确（此为既定设计，非 KC-05 的反向问题）。
6. **✅ customer_stage 写读路径一致**：粗筛读 domain_attributes.customer_stage（:44）↔ gateway 状态机门后写同字段（gateway.rs:4040-4054），主控亲验字段路径一致（不因字段名漂移恒筛不到）。

### [KC-05] 粗筛对 verification/eventKind 做 Mongo 精确匹配，漏掉缺字段的旧成交事件 → product 定向活动静默漏老客户（serde 默认与 Mongo 查询口径分裂）

- 入口频道：campaign 频道圈人（带 product_ids 的活动）
- 链路环节：圈人（粗筛）
- 类型：serde 默认 vs Mongo 查询口径分裂 / 假阴漏人
- 严重度：**Medium**
- 现象：带 product_ids 的活动，2026-06-15 §4.5 字段上线前登记的老成交客户会被粗筛漏掉，永进不了精筛 → 本该命中的老客户收不到活动推送。
- 根因链（主控亲验）：
  - 粗筛 `$elemMatch`（campaigns.rs:50-54）对 `verification:{$in:[...]}` + `eventKind:"deal"` 做 **Mongo 存储层精确匹配**。
  - `verification`（models.rs:451 `#[serde(default="default_outcome_verification"]`→staff_confirmed）+ `event_kind`（:464 default→deal）**只在 Rust 反序列化时补默认，Mongo 查询时不补**。models.rs:448-450 注释明说"缺字段即视为已核实；`#[serde(default)]` 只作用于反序列化"。
  - 旧文档 BSON 里根本没这两个字段（无迁移回填——主控亲验 migrations 仅 m011/m012/m014 是 production guard 文案，非回填）。
  - 精筛侧 project_entitlements 读的是**已反序列化的 Contact**（缺字段已补 staff_confirmed/deal）→ 精筛认为该客户持有 → 但粗筛已把它挡在外面。
- 后果：既定"粗筛 superset ⊇ 精筛"被**反转**——粗筛比精筛更严，旧成交事件 $elemMatch 匹配失败被排除，精筛没机会捞回。触发=①活动用 product_ids（空则不加 $elemMatch 无此问题）②库存在缺字段旧 outcome_events。
- 验证状态：**Medium / PLAUSIBLE**（代码层口径分裂 CONFIRMED：serde default 不作用于 Mongo 查询 + 无回填迁移，主控亲验 models.rs:451/464 + migrations；生产实际影响面依赖 117 库是否真有缺字段旧成交，本地无法验，117 活跃系统概率非低）。
- 修复建议：粗筛把"缺失=默认值"显式写进查询——`verification: {$or:[{$in:[...]},{$exists:false}]}` + `eventKind` 同理（或 `{$ne:"reversal"}` 与精筛 event_kind!=reversal 同口径）；或一次性迁移回填旧文档补齐两字段（更彻底，消除 serde 默认与 Mongo 查询的长期口径分裂）。

### [KC-06] preview targetCount / dispatch dispatchedCount / report targetCount 三义相近命名各异，可能误导运营

- 入口频道：campaign 频道 preview→dispatch→报表
- 链路环节：圈人/触达/成效
- 类型：可观测性/UX（语义不对齐）
- 严重度：**Low**
- 现象：preview 存 targetCount（命中总人数，:277）；dispatch 只写 dispatchedCount（本次新入队数，去重后，:379），**不更新 targetCount**；report 的 summary.targetCount（:472）又=campaign_sends 台账总行数。三个相近命名三种含义。首次 dispatch 若受众漂移缩小，dispatchedCount<残留 targetCount，运营会误读"有人没发出去"，实为受众变少。
- 根因（亲验）：受众重算是既定设计，但三处计数语义未对齐、dispatch 不回刷 targetCount。
- 验证状态：**CONFIRMED**（可观测性缺口，非发送正确性 bug）。
- 修复建议：dispatch 时把本次 hits.len() 回刷 campaign（如 lastDispatchTargetCount），或前端/文档明确区分"预览命中数/本次实发数/累计台账数"三义。

### [KC-07] resolve_segment_contacts 全量载入受众无 limit/分页（preview 与 dispatch 共用）——规模就绪债

- 链路环节：圈人
- 类型：规模（无上限）
- 严重度：**Low**（就绪债）
- 现象：resolve_segment_contacts（campaigns.rs:185-196）用 cursor 把粗筛命中的**全部** contact 逐条载入内存跑精筛，无 limit/分页；preview 与 dispatch 共用。product_ids 为空时粗筛退化为 {workspace,account,managed} 会扫本账号全部 managed 联系人。大受众下 preview 就可能超时/占内存。
- 验证状态：**PLAUSIBLE**（无上限 CONFIRMED；当前联系人量有限时无害）。与 KC-04 同源（触达侧也无上限），修复可合并。
- 修复建议：游标分批或规模上限；与 KC-04 一并做"受众规模保护"专项。

### 未能确认的点（供修复阶段核）
- **customer_stage 前端传值口径**：粗筛 domain_attributes.customer_stage 做精确字符串匹配、无归一/大小写容错（campaigns.rs:44）。后端写的是 taxonomy 校验后 canonical id。若前端 filter.customer_stage 传的也是 canonical id 则一致；若传显示名/大小写不符则**恒筛不到人且静默无报错**。前端传值口径未读 frontend 未确认——建议修复阶段核前端 filter 下发值，或后端加一次 canonical 归一兜底。

## 成交登记环 + 成效聚合环（合并 · 上游入口衔接 + 聚合口径）

审查范围：成交登记 `routes/contacts.rs`(add_deal_event:1407) + `management.rs`(write_deal_events:1917) + `routes/products.rs`；成效聚合 `routes/send_ledger.rs`(overview:115/stats:73) + `agent/send_ledger.rs`(response_rate:151/agg_count)。批 B 链6 已审落库核心（add_outcome_event_inner/approve_suspected_deal/entitlements 闭集），此处审上游入口 + 聚合口径。主控亲验。

### ✅ 亲验通过总览（0 finding · AI 永不自证成交红线在活动链上游同样成立）

1. **✅ AI 永不自证成交（活动链上游核验，最关键）**：`write_deal_events`（management.rs:1917）转调 add_deal_event(contacts.rs:1407)，payload.verification 直传 add_outcome_event_inner——**但这是 management-agent（管理台人类 admin 的控制台助手）工具**，路由挂 `/management-agent/*` 全 AuthenticatedAdmin（mod.rs:831-849），`management_admin(workspace_id)` 包的是**已鉴权 admin 的权限**（=admin 亲自填成交表单，本就是人工确认）。**客户端自治回复 agent（gateway/decision/knowledge_agent）无任何 deal-write 工具**——主控亲验 src/agent/ 下 write_deal_events/add_deal_event/add_outcome_event 仅 2 处注释引用，gateway.rs:4343 明说自治 agent 只写 suspected_deal_signals 待核实队列、"运营 approve 才调 add_outcome_event_inner 落正式成交"。故 AI 无法自证 staff_confirmed，红线在活动链上游同样成立。
2. **✅ verification 闭集在写侧兜底**：DealEventRequest.verification 注释（contacts.rs:88-91）声明只接受 staff_confirmed/payment_verified、conversation_inferred 传入即 400；实际拦在 add_outcome_event_inner→validate_deal_verification（shared.rs:1410，批 B 已亲验）。即便管理台 admin 手滑传 conversation_inferred 也被拒。
3. **✅ write_deal_events 风险分级 = Low 可逆写**（management.rs:1137，tool_effect），非只读、非 Irreversible；由管理台 admin 驱动，语义等价 admin 直登。
4. **✅ 成效聚合 workspace 隔离 + 率计算正确**：send_ledger_overview（:120）/stats（build_stats_match:25）/contact_send_history 均固定 workspace_id 过滤（含单测 stats_match_pins_workspace）；`response_rate(total, responded)`（agent/send_ledger.rs:151）分母=evaluated（**已评估**数，非总发送数——避免未评估的 sent 拉低率）、零守卫 total==0→0.0、分子分母正确（responded/total）。
5. **✅ agg_count 防静默清零**：send_ledger.rs:17 i64 优先、回落 i32、负值 clamp 0——防 Mongo $sum 类型漂移读成 0。

---

# 批 C 总评（成交活动链 · 审查收口）

**审查方式**：4 业务环（圈人→触达→成交登记→成效聚合），3 task 组织，每环派 opus subagent 只读复审 + **主控逐条亲验 file:line**。审查阶段只入账不改 src。

### finding 计数（去重后，全部主控亲验）
- **Critical 0 / High 0**
- **Medium 3**：
  - **KC-01**（触达）孤儿 send 永久漏推（campaign_sends 占去重位后 tasks insert `?` 失败→有 send 无 task，重发撞 DuplicateKey 跳过永建不出，report 归 pending 假象，无 worker 对账）
  - **KC-02**（触达）部分失败中断整批、campaign 永久卡 dispatching 无恢复 + dispatch 无 status 前置门可重复推送
  - **KC-05**（圈人）粗筛对 verification/eventKind 做 Mongo 精确匹配，漏掉缺字段的旧成交事件（serde 默认不作用于 Mongo 查询 + 无回填迁移）→ product 定向活动静默漏老客户
- **Low 4**：KC-03（taskId 回填失败→task 真发但 report 显 pending 成效虚低）、KC-04（触达规模无上限）、KC-06（targetCount/dispatchedCount 三义命名误导）、KC-07（圈人受众无 limit 就绪债）
- **整环干净 1**：成交登记+成效聚合环（0 finding——AI 永不自证成交在活动链上游同样成立、聚合口径正确）

### 跨环根因家族（修复统筹）
1. **触达多步非事务写家族**（KC-01 + KC-02 + KC-03，触达环）：**本批最系统性发现**。dispatch_campaign 的"占去重位→建 task→回填 taskId"三步非原子 + 无 checkpoint + 无 worker 对账，任一步失败留下孤儿 send / 卡 dispatching / report 失真。修复统筹：一个"活动触达可重入 + 孤儿自愈"专项——调换 send/task 写序（task 为可重放源）、dispatching 态可恢复、report 识别孤儿单列桶、补 status 前置门。
2. **serde 默认 vs Mongo 查询口径分裂**（KC-05，圈人独立）：粗筛 $elemMatch 精确匹配未处理"缺字段=默认值"。修复=查询显式 `$or {$exists:false}` 或迁移回填。
3. **规模无上限家族**（KC-04 + KC-07）：触达与圈人共用 resolve_segment_contacts 无 limit。修复合并做受众规模保护。

### 修复优先级建议（供用户定）
- **P1**：家族①（KC-01/02/03，触达多步非事务写）——孤儿漏推是营销触达最不该的后果（客户永久收不到、运营看不到失败），改动中等（调写序 + 可重入 + report 孤儿桶），可写确定性测试复现。
- **P2**：KC-05（粗筛口径分裂，product 定向漏老客户）——需先确认 117 库是否有缺字段旧成交定影响面；修复=查询 `$or {$exists:false}` 与 serde 默认对齐（低风险）。
- **P3**：Low 批量（KC-04/KC-07 规模保护、KC-06 命名对齐）+ 未确认项（customer_stage 前端口径核）。

### 与批 A/B 关联
- 三批最系统性家族各不同层：批 A=错误处理层（审计事件 `?` 吞回复）、批 B=数据写入审计层（知识编辑绕统一入口 + 字段锁未兑现）、批 C=多步非事务写健壮性层（触达三步非原子留孤儿）。**共性="设计声称的不变量/闭环，实现层有旁路/缺口/非原子窗口"**——正是上一轮广度走查（前端点页面 + 抽验）扫不到的"看不见的层"。
- 三批红线核心防线**均亲验成立**：批 A 自动回复命脉、批 B AI 永不自动 verify/产品声明须 verified 背书、批 C AI 永不自证成交（客户端自治 agent 无 deal-write 工具，只写 suspected_deal_signals，运营 approve 才落 staff_confirmed）。findings 全是"防护不完整/衔接有洞/审计链断/非原子窗口"，**非红线被突破**。

### 审查质量说明（防假绿）
- 每条 finding 的 file:line 均主控当场 Read/Grep 亲验；3 次 subagent 复审结论经主控亲验后坐实/校准入账（KC-01 补验无 worker 对账 + report 归桶、KC-05 补验 models.rs serde 默认 + 无回填迁移、成交登记环补验自治 agent 无 deal-write 工具）。
- **严重度校准（反过拟合）**：subagent 曾把 KC-01/KC-02 定 High，主控按跨批一致性校准为 Medium——后果虽重（客户永久漏推）但触发=循环中途 Mongo 瞬时写错误，与批 A/B 的 DB-fault 触发类 finding（B-02/C-01/F-01 均 Med）同级，不因单批 subagent 定 High 破坏跨批校准。
- 1 条整环 0 finding（成交登记+成效聚合）是真读透后的正面结论，非漏审。
- 未真跑（同批 A/B 定调），KC-06 CONFIRMED（静态确凿），其余 PLAUSIBLE。复现留修复阶段写确定性测试随修复 PR 上 CI。

---

# 批 D（请示配置链：askHuman / askHumanConfig / llmProviders / systemStrategy）

- 审查计划：[`2026-07-11-deep-logic-audit-batch-d.md`](../plans/2026-07-11-deep-logic-audit-batch-d.md)
- 四链：请示裁决（AI 识别→写单→领导裁决→AI relay 转述）→ 决策人链 → provider 热切换 → prompt pack。
- **红线**：客户永远只跟 AI 对话、**永不知道背后有"领导"存在**；relay 是 AI 转述幕后领导结论、非人工接管（决策请示通道设计 2026-06-05）。
- finding 编号 `KD-NN`。全部主控当场 Read/Grep 亲验；未真跑标 PLAUSIBLE。

## 请示裁决链 + relay 出站守卫（最高优先 · 红线"客户永不知道有领导"）

审查范围：`agent/escalation/logic.rs` + `agent/gateway.rs`（relay 守卫:2480 / relay_principal_decision_to_customer:755）+ `agent/escalation/mod.rs`（interpret/handle/escalate）+ `holding_reply.rs` + `prompt_isolation.rs`。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（红线核心防线成立）

1. **✅ relay 触发不可伪造（红线根基）**：is_principal_relay_trigger（logic.rs:196）只认 `AgentTrigger::Inbound(m) if m.is_synthetic_relay`；is_synthetic_relay 字段 **skip_serializing + skip_deserializing**（models.rs:786，测试 :6790-6818 锁死）——客户消息从 DB/webhook 恒反序列化成 false，**不可伪造**。仅 synthetic_principal_relay（models.rs:822）在 gateway.rs:755 relay 路径置 true。主控亲验。
2. **✅ 哨兵剥离防伪造转述模式**：inbound_prompt_content（prompt_isolation.rs:54）仅对 is_synthetic_relay=true 保留哨兵触发转述模式，客户伪造哨兵一律剥（:58），history 无条件剥——LLM 不会对客户输入进入转述模式。
3. **✅ escalate 客户侧零泄漏**：escalate_held_decision（mod.rs:43）只 message_send_text 给 principal_wxid + 写 awaiting 标记，对 contact.wxid 不发任何消息；principal_wxid==contact.wxid 时拒发（:75）。客户安抚与请示领导完全解耦。
4. **✅ interpret 侧红线**：interpret_principal_reply 解析失败/越界回落 deferred 空 substance（mod.rs:271）；handle 对 deferred 保持 pending、不起 relay（mod.rs:328）；sanitize_verdict 5 值闭集越界回落 deferred（logic.rs:411）。解析失败不误发。
5. **✅ relay 出站守卫 fail-closed 在入 outbox 前**：gateway.rs:2480-2519 relay run 拟发文本命中泄漏载荷/授权外数字即 outbox_eligible=false + 记 blocked_by_safety_guard，非 relay run 不受影响。数字白名单授权源取合成消息完整 content（含 substance+constraints 全部数字）。gateway.rs:2483 FollowUp 分支授权源="" 是**死分支**（relay 恒 Inbound 触发，主控亲验）。

### [KD-01] relay 授权外数字护栏只认 ASCII 数字，中文数字（九折/八折）完全盲区：既可绕过编造折扣、又误杀正确转述

- 入口频道：askHuman relay 转述（客户等待领导裁决后的转述）
- 链路环节：请示裁决（relay 出站数字护栏）
- 类型：红线代码backstop 覆盖盲区（双向：绕过 + 误杀）
- 严重度：**Medium**（relay 主防线是 prompt「AI 口吻重组」，此为**代码 backstop** 的覆盖盲区，非主防线突破；但 backstop 号称兜底却对最常见中文数字零覆盖）
- 现象：extract_number_tokens（logic.rs:220）只用 `ch.is_ascii_digit()` 提取阿拉伯数字。
  - **绕过**：领导授权 substance="9折"（阿拉伯），LLM 转述成"打八折"（中文）→ extract_number_tokens("打八折")=∅ → relay_introduces_unauthorized_number 恒 false → 放行编造折扣。
  - **误杀**：授权"九折"（中文）、转述"9折"（阿拉伯）→ "9"∉空授权集 → 判授权外 → fail-closed 拦掉正确转述。
- 根因（亲验）：数字 token 提取只覆盖 ASCII，中文数字/大写金额（八折/叁仟）既不进白名单也不进被检文本，白名单与被检文本不在同一数字空间。
- 验证状态：**CONFIRMED**（extract_number_tokens :224 仅 is_ascii_digit + relay_introduces_unauthorized_number :260-264 集合差比对，主控亲验；中文数字盲区确定）。
- 修复建议：extract_number_tokens 增中文数字→阿拉伯归一（一~十/百/千 + 折/成量词），白名单与被检文本同空间比较；无法归一时该场景保守 fail-closed。

### [KD-02] "客户永不知道有领导"红线无字符级词表守卫，relay 出站 + holding_reply 对"领导/老板/请示"语义泄漏零代码兜底（仅靠 prompt）

- 链路环节：请示裁决（relay 出站 + 过渡回复）
- 类型：红线代码兜底缺失（诚实标注：prompt-only）
- 严重度：**Medium**（LLM 转述本质靠 prompt 非必然 bug，但需如实标注最敏感泄漏向量无代码兜底 + 函数 doc 造成"已有代码守卫"错觉）
- 现象：
  - relay_output_leaks_internal_payload（logic.rs:211）只检测 4 个固定载荷标记（`__PRINCIPAL_RELAY__`/verdict=/substance=/constraints=）。转述若说"跟**领导**申请下来了""**老板**批准"——不含这 4 token → 放行。
  - gateway relay 分支（:2485）只调此守卫 + 数字护栏，**从不调 passes_forbidden_words**；即便调，evolution::lint FORBIDDEN_LITERALS_LOWER（lint.rs:13-27）只含"人工接管/接管/人工"家族，**不含领导/上级/老板/请示**（主控亲验）。
  - holding_reply_text_is_safe（holding_reply.rs:11）注释称"全自治定位禁词守卫"，但委托的 passes_forbidden_words 同样不含领导类词——"稍等我问下领导"可发出。
- 根因（亲验）：本项目对"无人工接管"红线有字符级 lint（check-no-human-takeover + evolution::lint），但对**平级的"客户永不知道有领导"红线无任何字符级词表**。relay 出站守卫是"载荷标记检测器"非"语义泄漏检测器"，函数 doc（:203-210）自称"出站方向代码守卫……与 sanitize_verdict 对称"易造成"已有代码兜底"错觉。
- 验证状态：**CONFIRMED**（结构事实：代码层无领导类词确定性兜底，主控亲验 lint.rs:13-27 词表 + gateway.rs:2485 只调 2 守卫；实际泄漏频率属 LLM 行为层未验，标结构 CONFIRMED）。
- 修复建议：新增"幕后决策源泄漏"词表（领导/上级/老板/请示了/上面批/汇报了）对 relay reply_text + holding_reply 施加（命中 fail-closed）。接受词表必漏 fuzz 变体、主防线仍是 prompt，但至少堵直白泄漏，与"无人工接管"红线的字符级 lint 对齐。

### [KD-03] relay 数字护栏误杀后清 awaiting + 不重排 relay task → 领导裁决永久丢失（fail-closed 把"这轮"放大成"永远"）

- 链路环节：请示裁决（relay task 生命周期）
- 类型：误杀后无补偿 / 裁决黑洞
- 严重度：**Medium**
- 现象：relay_introduces_unauthorized_number 误杀（KD-01 的误杀向量，或转述含"第2个问题""24小时"等非授权语义普通数字）→ outbox_eligible=false（gateway.rs:2491）→ relay 不入队。而 relay_principal_decision_to_customer 在 gateway 返回后**无条件** clear_awaiting_principal_state（gateway.rs:776），relay task 随即完成、**不重排**。领导已裁决（如 approved 9折）因转述夹了个"24小时"被误拦 → 客户**永远收不到这条裁决**（下轮客户来消息走普通对话、无 substance），awaiting 也清了。
- 根因（亲验）：extract_number_tokens 抓一切数字串无"数量事实 vs 序数/时间/无关数字"语义区分（函数注释自称"只看客观数量"实际抓一切）；误杀后清 awaiting + 无重试 = 裁决永久丢失。
- 验证状态：**PLAUSIBLE**（"裁决永久不达 + awaiting 清除"CONFIRMED：gateway.rs:776 无条件清 + relay task 不重排，主控亲验；误杀后客户是纯静默还是被 ensure_customer_acknowledged 补安抚未读该路径，标 PLAUSIBLE）。
- 修复建议：数字提取加语义降噪（跳过时间/序数上下文），或误拦时不清 awaiting、重排一次 relay 用中性收尾兜底文案（新增"裁决不可安全转述"场景），避免裁决黑洞。KD-01/KD-03 同源（数字护栏），修复可合并。

## 决策人链 + 超时改派 + 骚扰门

审查范围：`agent/escalation/policy.rs`（resolve_ask_human_policy:21 / next_decider_on_timeout:95 / push_allowed:68 / in_quiet_hours:55）+ `agent/escalation/mod.rs`（scan_escalation_timeouts:358）+ `agent/escalation/ledger.rs`（reassign:289 / lookup_principal_config:215 / count_pushes_today:338）+ `routes/principal_escalations.rs` + `routes/domains.rs`（put_ask_human_policy:206）+ `webhooks.rs`（领导回复分流:443）。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览

1. **✅ push_allowed 三条件组合**：cap/dedupe/quiet 任一不满足即 false（policy.rs:68-91），全 None→true 字节等价全放行。
2. **✅ in_quiet_hours 跨午夜正确**：start<=end 用 [start,end)；start>end 用 h>=start||h<end；tz_offset 折算 + 负数安全（policy.rs:55-64，单测覆盖）。
3. **✅ next_decider_on_timeout 常规边界**：timeout=None→None（无限等待）；age<timeout→None；链尾越界→None（policy.rs:100-105，仅"决策人不在链中"退化链尾属 KD-06）。
4. **✅ resolve_ask_human_policy 回落字节等价**：ask_human_policy 存在逐字段拷贝；None 时旧行为等价（policy.rs:21-52，单测覆盖）。
5. **✅ workspace 隔离**：lookup_principal_config（ledger.rs:224）/ reassign_escalation（:299）/ count_pushes_today/latest_push_ms/list_* 全带 workspace_id；两 admin 端点 AuthenticatedAdmin + current_workspace 约束；resolve 对非本 ws 条目返回泛化 alreadyResolved 避存在性泄漏；reassign 校验 to_wxid ∈ decider_chain。
6. **✅ 骚扰门"无自我命中"时序不变量**：gate 检查时台账仍挂原 principal，查 next 的 count/latest 不含本条（mod.rs:432-437）。
7. **✅ 短码唯一 + resolve/reassign 幂等**：唯一索引 + 碰撞换种子重试 5 次；find_one_and_update filter 带 status=pending，并发只一方成功。

### [KD-04] 用 decider_chain（推荐配置）时领导的微信回复永不被识别为裁决，掉进普通客户入站链路（甚至被 AI 当客户自动回复）

- 入口频道：askHumanConfig 配置 decider_chain 后，领导微信回复裁决
- 链路环节：决策人链（领导回复分流）
- 类型：新旧字段迁移不对称 / 核心交互静默失效
- 严重度：**High**（非 DB-fault/时序触发——**推荐配置下确定性发生**，核心交互"领导微信回复裁决"整体失效；admin REST resolve 仍可用故非全断，但 WeChat-reply 模式是设计主交互）
- 现象：领导用微信回复裁决时，webhooks.rs:443 靠 lookup_principal_config 分流。该函数（ledger.rs:224-227）查询条件是 `principal_decider: from_wxid`——**只认旧标量字段 principal_decider，完全不看 ask_human_policy.decider_chain**。
- 根因链（主控亲验）：
  - 唯一策略写路径 put_ask_human_policy（domains.rs:233）只 `$set ask_human_policy`，**从不写 principal_decider**。
  - 默认种子 prompts.rs principal_decider: None；m025 迁移是**正向** backfill（principal_decider→decider_chain，仅对旧有 principal_decider 的 config 生效，一次性）。
  - 用推荐的 decider_chain 配置后 principal_decider 恒 None → lookup_principal_config 恒返 None → handle_principal_reply 永不被调用 → 领导微信裁决消息掉进普通客户入站链路（领导若恰是 managed contact，甚至被 AI 当客户自动回复）。
- 验证状态：**CONFIRMED / High**（lookup_principal_config 只查 principal_decider（ledger.rs:226）+ put_ask_human_policy 不写 principal_decider（domains.rs:233）+ 种子 None，三处主控亲验；decider_chain 反查领导身份全仓无实现）。
- 修复建议：lookup_principal_config 改为解析 resolve_ask_human_policy(cfg) 后判断 from_wxid ∈ decider_chain（兼容旧 principal_decider），覆盖链中全部决策人（含改派后 next 决策人回复）。**批 D 最高优先修复项**。

### [KD-05] 改派目标的骚扰门统计口径漂移：用 created_at 近似推送时刻，改派不改 created_at → 低估 next 的打扰

- 链路环节：决策人链（超时改派骚扰门）
- 类型：骚扰门统计口径漂移
- 严重度：**Medium**
- 现象：count_pushes_today（ledger.rs:351 `created_at>=since`）与 latest_push_ms（:373 `sort created_at:-1`）都以台账 created_at 近似"推送时刻"；reassign_escalation（:304）改派只 `$set principal_wxid+updated_at`，**不改 created_at**。改派后该行 principal_wxid=next 但 created_at 仍是原始创建时刻 → 对 next 算骚扰门时：latest_push_ms(next) 返回陈旧时刻低估最近打扰、可能 dedupe 窗内再推；改派跨天时今天推给 next 的卡不计入 next 当日 cap。
- 根因（亲验）：改派复用 created_at 作推送时刻，未记真实"最近推送时刻"。
- 验证状态：**CONFIRMED**（口径亲验；实际触发需改派 + 同决策人多议题并发）。
- 修复建议：台账增真实"最近推送时刻"字段（改派/首推刷新），骚扰门统计改用它而非 created_at。

### [KD-06] 配置变更后旧 pending 挂"已不在链中"的决策人被永久当链尾晾住（孤儿 pending）

- 链路环节：决策人链（超时改派边界）
- 类型：配置漂移致孤儿 pending
- 严重度：**Medium**
- 现象：next_decider_on_timeout（policy.rs:104）用 `position(|d| d.wxid==current_wxid)?`——当前 principal_wxid 不在链中时 position 返 None → 函数返 None。scan（mod.rs:378）next=None 且 timed_out=true 被当**链尾**：只发客户安抚、台账保持 pending，**永不改派**。decider_chain 被 admin 改过（删/换人）后旧 pending 的 principal_wxid 可能已不在新链 → 永远卡在原（失效）决策人名下，既不改派新链、又（叠加 KD-04）无法微信回复推进，只能 admin REST 手动兜底。
- 根因（亲验）：position 未命中即静默退化为链尾语义。
- 验证状态：**CONFIRMED**（代码路径亲验；触发需运行中改链）。
- 修复建议：next_decider_on_timeout 在 position 未命中时回落链首（或明确"重新入链"策略），而非静默退化链尾。

### [KD-07] 超时改派缺"next==客户 wxid"防护（首推有、改派无），误配下会把请示卡推给客户

- 链路环节：决策人链（改派推卡）
- 类型：守卫不一致
- 严重度：**Low-Medium**
- 现象：首推 escalate_held_decision:75 有 `principal_wxid==contact.wxid→拒绝`；但超时改派 scan_escalation_timeouts:430 取 next.wxid 后直接推卡（:450），**无"next 是否等于客户 wxid"校验**。reassign_principal_escalation 只保证 to_wxid∈链内、不保证链内成员非客户 → admin 误把客户 wxid 配进 decider_chain[1..] 时改派会把含客户标签/卡点的请示卡直接推给该客户，泄漏内部请示内容。
- 根因（亲验）：两条推卡入口对"决策人==客户"守卫不一致。
- 验证状态：**CONFIRMED**（守卫缺失亲验；触发需误配）。
- 修复建议：改派推卡前复用同一 `next_wxid==entry.contact_wxid` 守卫；或 put_ask_human_policy 校验链成员不得等于任何在管客户。

### [KD-08] 推卡成功但 reassign 落库失败 → next 收到重复请示卡（已知设计权衡）

- 链路环节：决策人链（改派时序）
- 类型：多步非事务写（已知权衡）
- 严重度：**Low**
- 现象：scan 先推卡（mod.rs:450）成功才 reassign（:463）。reassign 落库失败 → 下一 tick principal_wxid 仍原值、age 仍超时 → 重算同一 next → 再推同 short_code 卡。DB 层无重复台账，但 next 微信收到两条相同请示卡。
- 根因（亲验）：push-then-reassign 时序落库失败无补偿、靠下 tick 重推兜底。代码注释（:459-461）已承认"幂等、可接受"。
- 验证状态：**CONFIRMED**（行为与注释一致，单实例影响有限）。
- 修复建议：可选——reassign 落库失败补一条"已推待改派"标记避免重推；或接受为已知权衡。

### 备案（观察项，非 live bug）
- **resolve_escalation filter 缺 workspace_id**（ledger.rs:161，仅 {short_code, status=pending}）：因 short_code 全局唯一 + 两调用方上游均已 workspace 预筛，当前不可跨域越权；缺纵深防御，未来新增未预筛调用方即成 IDOR。列观察项。
- **多实例并发双推**：scan_escalation_timeouts 单进程 worker（tasks.rs:167）驱动，当前单实例无害；横向扩展时两实例可能对同一 pending 同时推卡+改派（reassign_escalation 不 CAS principal_wxid）。就绪债，同多租户隔离债性质。

## provider 热切换 + prompt pack 生效闸

审查范围：`routes/llm_providers.rs`（activate_provider:305 / swap_registry:552 / test_provider:439 / base_url trim:178/229 / mask_api_key:43）+ `routes/prompt_templates.rs`（publish:240 / update:155 / reset_system_prompt_pack:385）+ `prompt_guard.rs`（三闸）+ `llm.rs`（with_format:268 / 请求拼接:453）。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（密钥安全 + 三闸不可绕 + 生效闸内容 diff + reset 语义全成立）

1. **✅ api_key 无明文 echo/日志泄漏（红线，若泄漏为 High）**：LlmProviderView（llm_providers.rs:53-68）只出 api_key_masked 无明文字段（主控亲验字段列表）；全 llm.rs + llm_providers.rs **零** api_key 进 tracing/println/format 宏（主控 grep 空）；create 拒 mask 占位/update 传 mask 沿用旧值/test 传 mask 回退 DB 真值三处一致。**CONFIRMED 干净**。
2. **✅ prompt pack 三闸不可绕**：字面双闸 validate_prompt_edit（禁词+锚完整性）在 create/update/publish 三处都过，且 publish 双闸在 force 判断**之外**（force 只跳 LLM 第三闸）；全仓仅 publish（gated）+ evolution/release 会写 status=active，无绕闸直置 active 路径。
3. **✅ 生效闸是内容 diff 非版本号**：review_prompt_edit 传 old_content vs new_content 行级取增量；启动重种 ensure_prompt_pack_v2 按 normalize_prompt_content 内容比对，与版本常量解耦（符合"改 prompt 不必 bump 版本"）。
4. **✅ publish 保护 evolution 行**：delete_many filter 带 `seeded_by:{$ne:"evolution_release"}`（prompt_templates.rs:338），不误删 rollback 链历史行。
5. **✅ reset-system-pack 是显式销毁性 reseed**：reset_prompt_pack_v2 对 4 集合 delete_many+重种，仅 route handler 调（非每启动幂等，启动走 ensure_prompt_pack_v2 不 clobber 运营编辑）；bump prompt_pack_version 失效 LRU cache 正确。
6. **✅ base_url trailing-slash 不发散**：LlmClient::with_format（llm.rs:268）内部 trim_end_matches('/')——test/activate 走同一 client 构造器，尾斜杠被统一吸收，**不因 test 不 trim 而与 activate 发散**（主控亲验，推翻初步"test 不 trim 致发散"担忧）。
7. **✅ activate 读 DB 已 trim 值传 swap**：activate_provider（:334 读 target DB 文档）→ swap_registry，用的是写库已 trim 的值，与运行时一致。
8. **✅ classify_review_verdict 收紧 fail-open**：仅 violation==false 才 Pass，模糊响应降级 NeedsHumanConfirm 不放水。

### [KD-09] base_url 缺 /v1 无任何前置校验/补全，仅错误后文案提示（生产 405 坑无硬防）

- 入口频道：llmProviders 频道 create/update provider
- 链路环节：provider 配置
- 类型：配置校验缺失（405 坑无前置防）
- 严重度：**Low**
- 现象：create/update 只 trim_end_matches('/')（:178/229），全代码无任何地方校验/补全 /v1。OpenAI 形态请求 `{base_url}/chat/completions`（llm.rs:453），管理员填 `https://api.deepseek.com`（漏 /v1）→ 打到 `.../chat/completions` → 404/405（历史踩过的坑）。仅错误后 classify_llm_error_for_user（llm.rs:982）文案提示补 /v1，无前置校验。
- 缓解（降 severity，亲验）：test_provider 与 activate 走同一 LlmClient 拼接，test 能在保存前复现 404 让管理员发现。
- 验证状态：**PLAUSIBLE**（无前置校验 CONFIRMED；触发需管理员漏填 /v1）。
- 修复建议：可选——create/update 对 openai 形态软校验 base_url 是否以 /v1（或已知兼容后缀）结尾，不匹配给 warning 而非 hard block（各家路径不一不宜强制）。

### [KD-10] provider 热切换 swap_registry 失败时 DB 已置新 active、运行时仍旧 client，返回 Err 掩盖"DB 已改"（瞬时不一致，自愈）

- 链路环节：provider 热切换
- 类型：DB↔运行时非事务
- 严重度：**Low**
- 现象：activate_provider 先 update_many 清旧 active（:325）→ update_one 置新（:334）→ swap_registry（:341）。若 swap_registry 返 Err（LlmFormat::parse 失败或 client 构造失败），DB 已提交 isActive=true 但运行时仍旧 client，函数返 Err，管理员以为激活失败实际 DB 已翻。
- 缓解（降 severity，亲验）：①format 在 create/update 已过 LlmFormat::parse，DB 合法值极难触发 parse 失败；②reqwest builder 几乎不失败；③**最终一致**——重启时 ensure_default_llm_provider（main.rs）按 isActive=true 从 DB 重建 registry 用上新 provider。自愈瞬时窗口。
- 验证状态：**PLAUSIBLE**（非事务窗口 CONFIRMED；触发需 swap 构造失败这一极罕见路径）。
- 修复建议：swap 失败时回滚刚写的 active 标记（或先 swap 成功再写 DB），让"返回 Err"与"DB 未改"一致。

### 备案（观察项，非 bug）
- **manual publish 后无 current_version=true 行**：publish 只置 status=active（prompt_templates.rs:377）不设 current_version=true 且 delete_many 删了旧 current_version 行。不影响运行时（load_prompt 按 status=active + version:-1 选，不依赖 current_version），但下次 publish 的 old_content 基线查询（:280 用 current_version:true）查空→全文当增量送 LLM 第三闸。代码注释已声明"查不到则空串"，属设计取舍（manual publish 是单版本清理，rollback 只 evolution 链支持）。

---

# 批 D 总评（请示配置链 · 审查收口）

**审查方式**：4 链（请示裁决→决策人链→provider 热切换→prompt pack），3 task，每链派 opus subagent 只读复审 + **主控逐条亲验 file:line**（含推翻 subagent 初判：base_url trailing-slash 经亲验 with_format 内部 trim 不发散）。审查阶段只入账不改 src。

### finding 计数（去重后，全部主控亲验）
- **Critical 0 / High 1**：
  - **KD-04**（决策人链）用 decider_chain（推荐配置）时领导微信回复永不被识别为裁决——lookup_principal_config 只查旧标量 principal_decider（ledger.rs:226）+ put_ask_human_policy 不写 principal_decider（domains.rs:233），领导裁决掉进普通客户链路甚至被 AI 当客户自动回复。**非 DB-fault/时序触发、推荐配置下确定性发生、核心交互失效**——批 D 唯一 High、跨批唯一确定性核心交互破坏。
- **Medium 4**：KD-01（relay 数字护栏中文数字盲区，绕过编造+误杀）、KD-03（relay 误杀后清 awaiting 不重排致裁决永久丢失）、KD-05（改派骚扰门用 created_at 口径漂移）、KD-06（改链后旧 pending 挂链外决策人被永久当链尾晾住）
- **Low 4**：KD-02（客户永不知道有领导红线无字符级词表守卫，仅 prompt）、KD-07（改派缺 next==客户守卫）、KD-08（推卡成功 reassign 失败重复卡，已知权衡）、KD-09（base_url /v1 无前置校验）、KD-10（provider swap 失败 DB↔运行时瞬时不一致，自愈）
  - 注：KD-02/KD-07 定 Low-Medium 边界，归 Low 计。
- **整块干净 1**：provider 热切换 + prompt pack（密钥安全/三闸不可绕/生效闸内容 diff/reset 语义全 CONFIRMED，仅 2 Low）

### 跨链根因家族（修复统筹）
1. **relay 数字护栏家族**（KD-01 + KD-03，请示裁决）：extract_number_tokens 只认 ASCII 数字→中文数字盲区（绕过+误杀），且误杀后清 awaiting 不重排致裁决黑洞。修复统筹：中文数字归一 + 误杀不清 awaiting/重排兜底文案。
2. **新旧字段迁移不对称家族**（KD-04 独立但最严重）：decider_chain 是推荐新字段，但领导回复分流仍靠旧 principal_decider 标量，写路径不同步。修复=lookup_principal_config 认 decider_chain。**与批 B KB-01（Lean 档 used_knowledge_ids 未清）、KB-05（serde 默认 vs Mongo 查询）同属"新旧路径/口径不对称"元家族**。
3. **红线代码兜底缺失/覆盖不全**（KD-02）：无人工接管红线有字符级 lint，但平级的"客户永不知道有领导"红线无。修复=对齐加词表。

### 修复优先级建议（供用户定）
- **P0/P1**：**KD-04**（领导微信回复识别，改 lookup_principal_config 认 decider_chain）——推荐配置下核心交互确定性失效，改动小（一个函数改查询），**批 D 最高优先，建议置顶**。
- **P1**：KD-01+KD-03（relay 数字护栏中文数字 + 误杀补偿）——涉红线转述正确性，改动中等，可写确定性测试。
- **P2**：KD-05/KD-06（骚扰门口径 + 孤儿 pending）+ KD-02（领导泄漏词表兜底）。
- **P3**：Low 批量（KD-07/08/09/10 + 备案观察项）。

### 与批 A/B/C 关联
- 四批最系统性家族分处四层：批 A 错误处理层（审计事件 `?` 吞回复）、批 B 数据写入审计层（编辑绕统一入口+字段锁未兑现）、批 C 多步非事务写健壮性层（触达三步非原子留孤儿）、批 D **新旧字段迁移不对称层**（KD-04 领导回复分流靠旧字段 + KD-01 数字护栏 + KD-02 红线兜底缺失）。
- **KD-04 是四批唯一 High**——前三批全是 0 High（防护不完整/衔接有洞/审计断/非原子窗口，触发需 DB-fault/时序/误配），KD-04 是**推荐配置下确定性发生的核心交互破坏**，够 High 线。
- 四批红线核心防线**均亲验成立**：relay 触发不可伪造/客户侧零泄漏/密钥不泄漏/三闸不可绕——findings 全是"防护不完整/衔接有洞/迁移不对称/覆盖盲区"，**非红线被突破**。

### 审查质量说明（防假绿）
- 每条 finding 的 file:line 均主控当场 Read/Grep 亲验；3 次 subagent 复审结论经主控亲验后坐实/校准/**推翻**入账（KD-04 补验 put_ask_human_policy 不写 principal_decider + 种子 None、KD-06 补验 position 未命中退化链尾、**base_url trailing-slash 经亲验 with_format 内部 trim 推翻 subagent 初判发散担忧、降为 /v1 校验缺失 Low**）。
- **严重度校准（反过拟合）**：subagent 曾把 KD-01 定 High、KD-04 定 High——KD-01 按"relay 主防线是 prompt、数字护栏是代码 backstop"校准为 Med（backstop 盲区非主防线突破）；KD-04 **确认 High**（非 backstop、是核心交互在推荐配置下确定性失效，与前三批 DB-fault 触发类本质不同）。校准依据是触发确定性 + 是否主防线，非机械压级。
- 1 整块 0 finding（provider+prompt pack，仅 2 Low）是真读透后正面结论。
- 未真跑（同前定调），KD-04/KD-05/KD-06/KD-07/KD-08 代码路径 CONFIRMED，其余 PLAUSIBLE。复现留修复阶段写确定性测试随修复 PR 上 CI。

---

# 批 E（其余频道：evolution / account / overview / operations / referral）

- 审查计划：[`2026-07-11-deep-logic-audit-batch-e.md`](../plans/2026-07-11-deep-logic-audit-batch-e.md)
- 五块：evolution 自优化（AI 提议+人工发布红线）/ account CRUD / overview 首屏 / operations 视图 / referral 名片引荐（辅助模式受控例外）。
- **红线**：AI 提议、人工发布，AI 绝不自动放量 prompt 到生产（evolution）；名片引荐仍是 AI 发起+辅助、对话始终 AI 在说、台前顾问≠幕后领导（referral 受控例外）。
- finding 编号 `KE-NN`。全部主控当场 Read/Grep 亲验；未真跑标 PLAUSIBLE。收官全五批。

## evolution 自优化（最高优先 · auto_release 是唯一 AI 自动写生产路径）

审查范围：`evolution/auto_release.rs`（gate_open:39 / eligible:48 / decide_auto_release:207 / decide_negative_reaction_block:226）+ `evolution/release.rs`（release_threshold:36 / release_prompt:198）+ `evolution/significance.rs` + `evolution/replay.rs` + `evolution/post_release.rs` + `routes/evolution.rs`（release_evolution_proposal:142）+ `config.rs`。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（"AI 提议+人工发布"红线成立，双闸默认双关）

1. **✅ auto_release 双闸默认双关（红线核心）**：auto_release_gate_open（auto_release.rs:39-41）= env `EVOLUTION_AUTO_RELEASE_ENABLED` AND per-workspace `threshold_auto_release_enabled`，缺失/读失败均回落关（:60 `.ok().flatten()` + `unwrap_or(false)`）。**config.rs:637-640 硬编码默认 "false"** + .env.example:170 false + 子闸 default false——**双锁，两闸都须显式开**。主控亲验。
2. **✅ prompt 绝不自动放量**：auto_release query 硬编码 `proposal_kind="threshold"`（auto_release.rs:77）；release_prompt 唯一 caller 是 routes/evolution.rs:169（release_evolution_proposal 内，AuthenticatedAdmin + confirmation=="RELEASE" 精确串校验）——**prompt 只人工 release，无任何自动路径**。主控亲验。
3. **✅ release_prompt 红线三闸不 fail-open**：事务内 compose_appended_content + validate_prompt_edit（字面禁词+锚）+ review_prompt_edit（LLM 语义），LLM 不可用→NeedsHumanConfirm 中止（release.rs:278）不 fail-open。
4. **✅ 无样本保守拒放**：decide_auto_release(None,...)→false（:209）；compute_window_gate_hit_rates total=0 返空 map→gate 缺失保守 SKIP。覆盖所有无样本路径。
5. **✅ significance 方向/样本门正确**：send_delta=new-original≥0.05（方向=改进）；gate_increase≤0.10；completed<min_replays 先拒；fail_rate>max 拒；NaN 一律 reject；#152 安全回归门零容忍拦 blocked→sent 翻转。failed replay 正确排除出 completed。
6. **✅ release_threshold 事务原子**：override insert + proposal status=released + audit 三写全 _with_session 一次 commit；threshold_overrides 写入 gate_key 与 runtime.rs:399 RESOLVED_GATE_KEYS 读取口径一致（写了能读到）。
7. **✅ post_release 只观测不自动回滚**：process_one_review 仅写 post_release_reviews + agent_event，无 release/rollback 调用；rollback_threshold/rollback_prompt 唯一 caller 是 routes/evolution.rs:212/215（admin 端点+confirmation 串校验）。Req 9.7 成立。
8. **✅ 负反应门同窗同源**：decide_negative_reaction_block 复用 post_release::compute_negative_reaction_rate 同 window_start 同极性源，超阈强制 SKIP 退 admin 不回滚。

### [KE-01] decide_auto_release 缺"方向一致性"校验，实现与 doc 声称不符，命中率跨 band 翻转时可反向放量 threshold

- 入口频道：evolution 自优化（auto_release，仅运维显式开启时）
- 链路环节：evolution（自动放量决策）
- 类型：doc-实现漂移 / 放量方向错误
- 严重度：**Medium**（放量方向错误后果重，但触发需 auto_release **双闸显式开启**（默认双关、且是"受控例外"opt-in 面）AND 命中率跨 band 翻转到相反外侧——非默认配置 + 时序依赖，按跨批校准=Med，不同于 KD-04 的"推荐配置确定性发生"）
- 现象：模块 doc（auto_release.rs:11-13）声称放行条件是"命中率仍在 band 之外**（方向与候选方向一致，意味着信号没有自然回正）**"。但 decide_auto_release（:207-211）实现只判落 band 外**任意一侧**：`rate < target_lower || rate > target_upper`，**不接收候选方向**（proposed_value vs current_value），调用点（:121-165）拿到整条 proposal 却从未用来判方向。
- 失败场景（亲验推演）：gate=fact_risk_block band=[0.05,0.15]，候选生成时 hit_rate=0.30>upper→升阈值候选（拦截更少→命中率降）；经 replay+significance 通过→eligible。auto_release tick 用**不同时间窗**重看命中率，若此时 observed 已翻转到 0.02<lower → decide_auto_release(0.02,0.05,0.15)→0.02<0.05→true→**放行升阈值候选**，但命中率此刻已过低、继续升阈只把命中率推更低——朝错误方向放量，与"信号没自然回正才放行"设计相悖。
- 根因（亲验）：设计意图（方向必须一致）没落进实现；decide_auto_release 签名只有 (observed,lower,upper) 天然无法表达方向。
- 验证状态：**CONFIRMED**（实现 :210 无方向参数 + doc :11-13 声称方向一致 + 调用点有 proposal 未用，主控三处亲验；触发窗口窄+默认关故 Med）。
- 修复建议：给 decide_auto_release 传候选方向（proposed_value-current_value 符号），仅当 observed 偏离方向与候选修正方向一致才放行——升阈候选只在 observed>upper 放行、降阈候选只在 observed<lower 放行，相反侧一律 SKIP 退 admin。

### [KE-02] threshold 重判 send_success 口径不对称：original 用真实终态、new 用纯 5gate 重判，非-5gate block 类 run 抬高 send_delta 可虚假过门

- 入口频道：evolution 自优化（threshold 显著性评估）
- 链路环节：evolution（replay→significance）
- 类型：重判口径不对称 / 显著性虚高
- 严重度：**Medium**（PLAUSIBLE）
- 现象：threshold 重判里 original_final_review_status 用源 run **真实**终态（replay.rs:296），而 new_final_review_status = final_status_from_5gate（:406-425，只产 5 闸决定的 5 种状态）。若 cohort 含非-5gate 因素 block 的 run（如 blocked_by_budget / blocked_by_required_field / ai_waiting_for_more_context，post_release.rs:40 UPGRADED_STATUSES 可见的合法终态）且带 review.scores：original 侧按真实终态计为发送失败、new 侧 5 闸重判不命中算 approved=成功 → 凭空制造一次"send_success 提升"，与被改阈值无关，累积到 send_delta 可能把不达标候选虚假抬过 min_send_success_delta(0.05) 门→误放行伪改进。
- 根因（亲验）：final_status_from_5gate 假设"final 完全由 5 闸决定"，但源 run 真实终态可能是非-5gate 因素。
- 验证状态：**PLAUSIBLE**（口径不对称 CONFIRMED；需 cohort 含"非-5gate block 但带 scores 且 5 闸重判 approved"的 run 且占比足以翻越 0.05 门，频率需生产终态分布量化）。
- 修复建议：evaluate_threshold 计 original/new send_success 对齐口径——original 也用 final_status_from_5gate 基于源 scores+current 阈值重推（两侧同口径仅差被改 gate），或重判前剔除非-5gate 决定终态的 run 不计入 send_delta。KE-01/KE-02 均只在 auto_release 开启时影响放量（默认关）。

## referral 名片引荐（红线受控例外 + 三闸让位）

审查范围：`agent/referral.rs`（assist_mode_active:17 / validate_card_sendable:26 / send_outbound_namecard:99）+ `agent/decision.rs`（assist_on:395 / referral_block:399 / build_referral_cards_filter:1405）+ `agent/gateway.rs`（名片入队门:2818 / 准入:2837 / 不走 escalation:2813）+ `agent/review/mod.rs`（reviewer 让位:259/348）+ `routes/referral_cards.rs`（create:51 / review:122 / toggle:185 / delete:212）+ `agent/review/gates.rs`（产品硬闸:658）。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（辅助模式红线受控例外成立·默认关）

1. **✅ 辅助模式默认关 + 三处 assist_on 判定一致**：assist_mode_active（referral.rs:17-23）force_on/force_off override > account_enabled.unwrap_or(false)=默认关，脏值 override 视无覆盖。三处（decision.rs:398 prompt 注入 / gateway.rs:2824 入队 / review/mod.rs:353 reviewer 让位）调**同一纯函数**、同输入、同优先级。**关时全跳**：gateway 二次门 `if assist_on`（:2828）整段包住入队，即便 LLM 幻觉 namecard_to_send 也不入队。主控亲验。
2. **✅ 让位段不架空产品硬闸**：blocked_unverified_product_claim 是 gates.rs:658-684 **确定性结构化闸**，判据=claim_requires_product_knowledge && verified_chunks 空 && !priced_from_catalog，**不读 factRisk/reviewer 自评分**（gates.rs:642 注释"对 reviewer 自评分不信任的兜底"）。即便让位段令 reviewer 降 factRisk，产品声明硬闸独立照拦。让位措辞（referral.rs:14）明确只针对"引荐这一动作"、明说引荐不是产品声明（不计入 hallucination/产品准确度），未泛化到普通产品声明。主控亲验。
3. **✅ 引荐≠转人工（物理隔离）**：gateway.rs:2811-2818 名片独立入队**不走** escalation 分支（不会被标 held_by_ai_policy）；台前顾问存 referral_cards 集合、幕后 principal_decider 存 operation_domain_configs.decider_chain——**物理隔离不同集合**（D9 解耦）。措辞守卫"我仍在场辅助"就位。
4. **✅ 名片门含 relay 泄漏 fail-closed + should_reply + reply 非空**：media_send_allowed(outbox_eligible,...)（:2818）——outbox_eligible 含 relay 泄漏 fail-closed，杜绝 should_reply=false/reply 空发孤立名片、relay 泄漏时仍发名片。
5. **✅ 准入闭合**：gateway.rs:2837 + referral.rs:111 双重 find_one（workspace scope）+ validate_card_sendable（enabled&&approved）；幻觉/不存在/未审/跨 workspace/停用 → 全落 referral_card_rejected 事件。
6. **✅ CRUD 红线**：create 强制 enabled=false+review_status=draft（referral_cards.rs:80，AI 不自我核验，无路径创建即 approved）；approve 不自动 enable（validate 要 enabled&&approved 双门）；review/toggle/delete 均 AuthenticatedAdmin + workspace scope。

### [KE-03] 名片发送准入门只按 workspace_id 不按 account_id，与候选加载器（账号级）不对称，同租户内可跨账号推名片

- 链路环节：referral（发送准入）
- 类型：防御纵深不对称（同租户业务错配）
- 严重度：**Low**
- 现象：候选加载 build_referral_cards_filter（decision.rs:1405-1419）是**账号级**过滤（workspace_id + `$or:[account_id null, ==account_id]` + enabled + approved），注入给 LLM 的是本账号候选。但 gateway 二次准入（:2837）和 send_outbound_namecard（referral.rs:111）的 find_one **只带 workspace_id 不带 account_id**。同 workspace 内绑定账号 A 的顾问名片若 card_id 被选中，会经账号 B 的会话推出去——同租户业务错配。
- 根因（亲验）：准入门对齐了素材侧 workspace IDOR 防御，但未镜像候选加载器的 account 维度。
- 验证状态：**Low / PLAUSIBLE**（口径不对称 CONFIRMED，主控亲验候选账号级 vs 准入 workspace-only；正常 LLM 只看本账号候选，触发需 LLM 幻觉合法他账号 ObjectId，概率极低）。
- 修复建议：gateway 与 send_outbound_namecard 的 find_one 追加 `$or:[account_id null, ==contact.account_id]` 与 build_referral_cards_filter 同口径，命中失败照走 referral_card_rejected。

### [KE-04] 名片防重推是纯 prompt 建议无硬去重门（有意软设计，登记）

- 链路环节：referral（防重推）
- 类型：软约束无硬兜底（有意设计）
- 严重度：**Low**
- 现象：对"已引荐过同一顾问"的防重推，全链路只有 render_referral_lines 往 prompt 注入一句建议（referral.rs:76-79）。gateway 入队块（:2818-2907）不读 REFERRED_CARD_ID_ATTR、无硬门；outbox 幂等键含 run_id（outbox.rs:412），每个新 run 键不同 → 同卡跨 run 可再入队真发。
- 根因（亲验）：去重设计成 LLM 遵从 prompt 的软约束，无确定性兜底。outbox_dispatcher.rs:672 注释"重复推名片危害小…放行重发"——**有意软设计**。
- 验证状态：**Low / PLAUSIBLE**（危害受限=客户多收一张名片，且文档化取舍）。
- 修复建议：若要硬防，gateway 入队前读 REFERRED_CARD_ID_ATTR 同卡已引荐则跳过+落 referral_card_duplicate_skipped（须保留"新需求可重推"例外）；或维持现状 + 设计文档标注"去重 advisory-only"。

### [KE-05] toggle 停用 / delete 删除名片无审计事件，delete 为硬删无留痕

- 链路环节：referral（名片库 CRUD 审计）
- 类型：审计留痕缺失
- 严重度：**Low**
- 现象：review_referral_card 写 referral_card.reviewed 审计（referral_cards.rs:167），但 toggle_referral_card（停用）和 delete_referral_card（delete_one 硬删）**都不写审计事件**。停用/删除已审批顾问名片直接改变 AI 可引荐范围（红线敏感），却无留痕；硬删误删不可恢复。三 handler 均有 AuthenticatedAdmin + workspace scope（越权已防）故 Low。
- 验证状态：**Low / PLAUSIBLE**（审计缺失 CONFIRMED）。
- 修复建议：toggle/delete 补 referral_card.toggled/deleted 审计（同 review 模式 fail-soft）；delete 可改软删（enabled=false + deleted_at）保留可追溯。

## account + overview / operations（合并 · 低风险配置+只读统计）

审查范围：`routes/accounts.rs`（list:32 / sync:64 / update_mcp_key:171 / login:244）+ `routes/operation_view.rs`（active_view:27）+ `routes/send_ledger.rs`（overview:115 / stats:73）+ outcome_metrics/behavior_signal_metrics。主控派 opus subagent 复审 + 逐条亲验。

### ✅ 亲验通过总览（本块整体干净，红线关注点全达标）

1. **✅ 鉴权/workspace 隔离全绿**：list/sync/update_mcp_key/login 全 AuthenticatedAdmin + current_workspace scope；update_account_mcp_key `_id+workspace_id` 双过滤（跨 ws 改不到别人 key）；login alias 查账号带 ws scope。
2. **✅ mcp_api_key 明文不泄漏**：list_accounts 只下发 mcpKeyConfigured 布尔（accounts.rs:56，主控亲验）；全 accounts.rs 响应无明文；key 只进 Authorization Bearer 头，logged_call 落库走 redact_request_for_log；无 tracing 打印 key。
3. **✅ mcp_api_key 不被 sync 抹掉**：$setOnInsert（accounts.rs:158）保护既有手配 key。
4. **✅ 无破坏性删除面**：mod.rs 无 /accounts DELETE 路由、无 delete_account handler。
5. **✅ 统计口径健全**：response_rate 除零守卫返 0.0；agg_count i32/i64 兼容防静默清零；overview 率以 evaluated 为分母；只读端点无写副作用；outcome_metrics 投影不下发 workspaceId。
6. **✅ online 陈旧风险已缓解**：online 不止 sync——webhook Offline/Online 事件实时落库（webhooks.rs:361）；outbox dispatcher 发送前 gate fail-soft（掉线 defer 不盲发），陈旧 online 不致误发。

### [KE-06] sync_accounts 用 $set 覆盖手配 mcp_base_url，与 mcp_api_key 的 $setOnInsert 保护不对称

- 链路环节：account（sync upsert）
- 类型：保护策略不对称（就绪债）
- 严重度：**Low**（就绪债）
- 现象：update_account_mcp_key 允许管理员为单账号写自定义 mcp_base_url；但 sync_accounts upsert 时把 mcp_base_url 放进 $set（accounts.rs:147，值恒为 config.mcp_base_url），下次 sync 会静默把手配 base_url 重置回全局默认。而 mcp_api_key 被刻意保护在 $setOnInsert（:158，注释"避免 sync 抹掉手配 key"）——二者非对称。
- 根因（亲验）：同一 upsert 里 mcp_api_key 有 $setOnInsert 保护、mcp_base_url 没有。
- 验证状态：**Low / CONFIRMED**（$set vs $setOnInsert 不对称，主控亲验 accounts.rs:147/158；单 default account+单 MCP 部署下 base_url 通常与 config 一致，触发面窄）。
- 修复建议：把 mcp_base_url 从 $set 移到 $setOnInsert（与 mcp_api_key 对齐），或 sync 时不写 base_url。

---

# 全五批终评（核心业务逻辑全链路深度审查 · 收官）

**审查方式**：5 批 · 21 条业务链/环/块，每链派 opus subagent 只读复审 + **主控逐条 file:line 亲验**（含调级/去重/驳回/**推翻** subagent 结论）。审查阶段只入账不改 src。逐行读码（PLAUSIBLE），117 真跑留修复阶段写确定性测试（不在生产注故障/kill 进程）。

## 累计 finding 计数（去重后，全部主控亲验；数字由 ledger 头逐条统计核对，非记忆）
- **批 A（自动回复命脉链）**：18 条 = 0C / 0H / 4M（B-02/C-01/H-01/F-01）/ 14L（A-01~06 双身份族+边缘、B-01/03、D-01、E-01、F-02/03/04、H-02）
- **批 B（知识链）**：12 条 = 0C / 0H / 5M（KB-01/08/09/10/11）/ 7L
- **批 C（成交活动链）**：7 条 = 0C / 0H / 3M（KC-01/02/05）/ 4L
- **批 D（请示配置链）**：10 条 = 0C / **1H（KD-04）** / 4M（KD-01/03/05/06）/ 5L
- **批 E（其余频道）**：6 条 = 0C / 0H / 2M（KE-01/02）/ 4L（KE-03/04/05/06）
- **累计：53 findings = 0 Critical / 1 High / 24 Medium / 28 Low**（另 3 整环/块 0 finding：批 A 阈值闸+MCP、批 C 成交登记+成效聚合、批 D provider+prompt pack；及大量 ✅ 亲验通过红线点）。

> 注：唯一 High = **KD-04**（用 decider_chain 推荐配置时领导微信回复永不被识别为裁决）——四批 0 High 的例外，因它是**推荐配置下确定性发生的核心交互破坏**，非其余 findings 的"DB-fault/时序/误配/opt-in"触发。

## 跨批元家族（最有价值的结构性洞察）
本轮深度审查最系统性的发现，是一个贯穿五批的**元家族**：

> **「设计声称的不变量/闭环/口径，实现层有旁路/缺口/非原子窗口/新旧不对称」**

分处五层，每层一个代表：
1. **错误处理层**（批 A 家族①）：审计/旁路事件误用 `?` 连坐吞回复（B-02/C-01/H-01，6 处 `?`）——注释自认应 fail-soft、实现却 fail-closed。
2. **数据写入审计层**（批 B 家族①）：知识编辑绕过"统一 apply_chunk_revision 入口"（KB-09/10/11），且 per-chunk locked_fields 后端从不强制——设计声称统一，实现有旁路。
3. **多步非事务写健壮性层**（批 C 家族①）：dispatch_campaign 三步非原子（KC-01/02/03）——占去重位→建 task→回填，中间失败留孤儿。
4. **新旧字段迁移不对称层**（批 D 家族②）：领导回复分流靠旧 principal_decider、推荐配置用 decider_chain（KD-04）；relay 数字护栏只认 ASCII（KD-01）；批 B KB-01/KB-05 同属此元家族。
5. **保护策略不对称/口径分裂层**（批 E + 批 C）：mcp_base_url vs mcp_api_key 保护不对称（KE-06）；serde 默认 vs Mongo 查询口径分裂（KC-05）；名片准入 workspace vs 候选账号级不对称（KE-03）。

**这正是上一轮"全量系统测试"（前端点页面 + 抽验，偏广度）扫不到的"看不见的层"**——逐行读码 + 主控亲验才能穿透到。

## 红线总结论（五批一致）
**所有核心红线防线均亲验成立，无一被突破**：
- 批 A：自动回复命脉（gateway 闸/阈值闸/outbox 幂等/状态机 fail-soft）。
- 批 B：AI 永不自动 verify（auto_verify 强制降级所有类型）+ 产品声明须 verified 背书（grounding 三处协同 + 硬闸）+ AI 提议不落主集合。
- 批 C：AI 永不自证成交（自治 agent 无 deal-write 工具、只写 suspected_deal_signals、运营 approve 才落 staff_confirmed）。
- 批 D：relay 触发不可伪造（is_synthetic_relay skip_ser/deser）+ 客户侧零泄漏 + 密钥不泄漏 + prompt 三闸不可绕。
- 批 E：AI 提议+人工发布（auto_release 双闸默认双关、prompt 只人工 release）+ 名片引荐受控例外（默认关、引荐≠转人工、物理隔离幕后领导）。

**53 条 findings 全是"防护不完整/衔接有洞/审计断/非原子窗口/迁移不对称/口径分裂"，非红线被突破。**

## 修复路线图（供用户定优先级）
- **P0**：**KD-04**（领导微信回复识别，改 lookup_principal_config 认 decider_chain）——推荐配置下核心交互确定性失效，改动小（一个函数改查询），**置顶**。
- **P1**（改动小/中、价值高、可写确定性 lib 单测）：
  - 批 A 家族①（审计事件 fail-soft 对齐，6 处 `?`）
  - KB-08（收件箱纳入 needs_human_audit）+ KB-01（grounding 硬闸 else clear()）
  - 批 C 家族①（触达可重入+孤儿自愈）
  - KD-01/03（relay 数字护栏中文数字 + 误杀补偿）
- **P2**（需先裁定设计定位或改动中等）：
  - 批 B 家族①（知识编辑统一接回 apply_chunk_revision + locked_fields 后端强制，需裁定字段锁定位）
  - KC-05（粗筛 serde/Mongo 口径对齐）+ KD-05/06（骚扰门口径/孤儿 pending）+ KD-02（领导泄漏词表兜底）
  - KE-01/02（auto_release 方向一致性 + 重判口径，仅 auto_release 开启时影响）
- **P3**：Low 批量（产品裁决 / 就绪债 / latent / 死代码 / 规模保护 / 命名对齐 / 审计留痕 / 保护不对称）。

## 审查质量说明（防假绿·收官）
- 每条 finding 的 file:line 均主控当场 Read/Grep 亲验；跨 5 批 ~15 次 subagent 复审，结论经主控亲验后**调级/去重/驳回/推翻**入账（如批 D base_url trailing-slash 经亲验 with_format 内部 trim 推翻 subagent 发散初判、批 A 驳回 F-⑧-02 假 finding）。
- **严重度跨批一致性校准（反过拟合核心）**：subagent 多次把 DB-fault/opt-in 触发类定 High，主控按统一基准校准为 Med（触发需 DB-fault/时序/非默认配置=Med；只有"推荐配置下确定性发生的核心交互/红线破坏"够 High——全五批仅 KD-04 达此线）。校准依据是触发确定性 + 是否主防线，非机械压级。
- 3 整环/块 0 finding + 大量 ✅ 亲验通过红线点是真读透后的正面结论，非漏审。
- 未真跑故无 CONFIRMED 运行时复现；代码路径 CONFIRMED 的 finding 已如实标注，其余 PLAUSIBLE。复现留修复阶段写确定性 lib 单测/小型集成测试随修复 PR 上 CI（比生产 kill 进程更严谨可控）。
