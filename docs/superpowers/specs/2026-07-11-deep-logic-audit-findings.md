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

_（待审）_

## 环节③ gateway 巨型闸函数（managed / cooldown / min-interval / 日上限 / 过期闸）

_（待审）_

## 环节④ 决策 + 渐进式知识路由（decision.rs / knowledge_router.rs）

_（待审）_

## 环节⑤ 独立 review + 阈值闸 + revision（review/gates.rs）

_（待审）_

## 环节⑥ outbox 幂等 / claim / second-pass safety gate / retry

_（待审）_

## 环节⑦ MCP 发送（message_send_text / result.isError / 超时）

_（待审）_

## 环节⑧ 回写（events / outcome metrics / decision review / run log / operation_state 派生）

_（待审）_
