# 业务审查修复波设计

> 状态：设计（待用户审阅 → writing-plans）
> 日期：2026-06-22
> 范围：三个子项目（ask-human P1/P2/P3）之外，从业务逻辑与业务场景角度审查发现的 11 条缺陷（4 Critical + 7 Important + 若干 Minor）。

## 背景与定性

继 ask-human 三子项目 32-agent 交叉验证审查收口后，用户要求"在业务逻辑和业务场景的角度审查一下"。6 路 opus 并行审查 + 主持人逐条 file:line 亲核，产出 4 Critical + 7 Important + Minor。用户决定"全部都修复吧"。

经四组 opus 调查员实证（含对初判的校准），11 条归为六组。**A–E 十条是已审查清楚、性质同质（纯代码修复）、会一起交付的修复波**（与上次交叉验证修复波同模式），作为一个 spec 一次交付；**F（多模态入站）是带外部依赖的新 feature**，本波只做代码地基 + 外部依赖打桩，完整实现另行立项。

### 两条贯穿铁律

1. **不夺 LLM 语义判断**（项目一贯 agent-first）。本波所有"护栏"只兜**客观边界**：状态机字典（配置驱动）、数字事实白名单、知识时效、账号在线状态。凡属语义判断（如"这句话是不是做了承诺"）一律不加确定性硬拦，交还 LLM + prompt。
2. **反过拟合**（红线中的红线）。护栏必须是可复现的抽象方法论，不得针对单条对话/单次 CI 样本点对点修补。

### 红线对齐

- **客户永不被晾死、永不直接面对真人**：A 组（请示通道闭环）的全部话术用 AI 自治措辞（"我再帮您同步/确认"），绝不出现"人工接管/转人工/hand-off"。受 `scripts/check-no-human-takeover.{sh,ps1}` lint 约束。
- **AI 永不自动 verify 知识**：B 组只收紧"过期知识不可背书"，不放宽任何 verify 路径。

---

## 业务取舍（已与用户收敛，逐条决策）

| 缺陷 | 业务取舍（用户已定） |
| --- | --- |
| ③ 领导失联客户兜底 | **AI 自主延期话术，保持等待**（不自答越权点、不升级 admin） |
| ④ 账号级发送总量闸 | **软上限只告警**（不硬拦、不排队） |
| ⑤ 非文本入站 | **配多模态模型**；本波做代码地基 + 外部依赖打桩 |
| ⑥⑧⑩ 决策/转述护栏 | **加轻量护栏，不夺语义判断** |
| ⑥ 承诺兑现（细化） | 确定性护栏做不到（语义判断）→ 降为 **prompt 强化 + 观测事件**，不硬拦 |

---

## A 组 · 请示通道闭环（②③⑩）

核心：客户永不被晾死 + 转述事实安全。三条均已 file:line 亲核。

### ② 授权过期 → 客户零反馈 + awaiting 永久残留

**现状（亲核）**：`src/agent/escalation/mod.rs:182` `handle_principal_decision_relay`：
```rust
if relay_substance_if_usable(&decision, entry.authorization_expires_at, now).is_none() {
    // 授权过期：不拿过期授权乱承诺，结束。
    return Ok(());
}
```
早退发生在 `relay_principal_decision_to_customer`（gateway.rs:203 调用，clear_awaiting 在 gateway.rs:598）之前 → 授权过期时客户什么都没收到，且 `domain_attributes.awaiting_principal_decision` 标记永久残留（下一轮 `build_decision_signals_text` 仍读到"正在等待裁决"，永久压制对该议题的自主回复）。

**修复**：早退分支改为——
1. 调 `clear_awaiting_principal_state`（议题已被领导处理过，标记必须清；该函数现为 gateway 内 relay 流程私有，需提为可在 escalation 早退分支调用——见"接口调整"）。
2. 发一条**不含过期 substance 的中性收尾话术**（AI 自主口吻，措辞如"关于您之前问的那件事，我这边再帮您跟进下最新情况，有结果第一时间同步您"）。绝不使用过期授权里的具体承诺/数字。
3. 之后客户再来消息，awaiting 已清，正常对话接管。

**接口调整**：`clear_awaiting_principal_state`（gateway.rs:598 附近，当前为 relay 流程内私有）提取/暴露为 `pub(crate)`，供 escalation 早退分支与 relay 完成分支共用同一实现（避免 dual-path drift——参照 D2 锚定教训：抽共用纯函数/共用写入，不写第二份）。

### ③ 领导链尾失联 → 客户永久搁置

**现状（亲核）**：`src/agent/escalation/policy.rs:105` `next_decider_on_timeout` 在链尾 `decider_chain.get(idx+1)` 返回 None；`scan_escalation_timeouts`（mod.rs）遇 None `continue`。客户只在最初被 hold 时收到一次 `fallback_holding_reply`（mod.rs:141），此后领导一直不回 → 永久静默。

**修复（对齐"AI 自主延期话术保持等待"）**：在 `scan_escalation_timeouts` 里，当一条 pending 请示 `next_decider_on_timeout` 返回 None（链尾，无更多决策人可改派）且仍 pending 时：
1. 发一条 AI 自主延期安抚话术（自治口吻，不自答越权点）。
2. 台账保持 pending，继续等领导。
3. **去重（关键）**：`AgentPrincipalEscalation` 新增字段 `last_holding_reply_ms: Option<i64>`（`#[serde(default)]` 向后兼容）。仅当 `now - last_holding_reply_ms >= holding_reply_min_interval`（默认值见"配置"）才发，发后更新该字段。否则本 tick 跳过——防止每个 worker tick（间隔 `TASK_WORKER_INTERVAL_SECONDS`）刷屏。
4. 安抚话术发送同样过 `push_allowed` 静默门？——**不过**：这是发给**客户**的安抚（非给领导推卡），quiet_hours 是约束打扰领导的，对客户的安抚走客户侧 min-interval（即第 3 点的去重间隔）即可。（设计澄清点，见"未决细节"——倾向不过 push_allowed，仅受 holding 去重间隔约束。）

**不做**：不升级 admin、不替领导拍板、不自答越权议题。

### ⑩ 转述编造授权外数字 → 无代码护栏

**现状（亲核）**：`src/agent/escalation/logic.rs:174` `relay_output_leaks_internal_payload` 只检测**载荷泄漏**（哨兵 `__PRINCIPAL_RELAY__` / `verdict=` / `substance=` / `constraints=`）。它**不检测**转述文本里出现了授权 substance 之外的数字/金额（领导说"可以给 9 折"，AI 转述成"8 折"或追加"再送一年质保" → 当前无拦）。

**修复（"轻量数字白名单护栏"）**：新增纯函数 `relay_introduces_unauthorized_number(reply_text: &str, authorized_substance: &str) -> bool`：
- 提取 reply_text 中的数字 token（整数/小数/百分比/带千分位金额/中文数字阈值——范围见"未决细节"）。
- 逐个核验是否出现在 `authorized_substance`（领导授权的实质内容）中。
- 出现了 substance 没有的数字 → 返回 true。
- 网关 relay run 入 outbox 前调用（紧邻现有 `relay_output_leaks_internal_payload` 调用点 gateway.rs:1778），命中即 fail-closed（不发该转述，回落"我再跟领导确认下准确信息"类安全话术）。

**边界**：只兜**数字事实**，不碰话术语义、不判断措辞好坏。纯函数，便于单测 + 反过拟合（测多种数字形态变体，不针对单句）。

---

## B 组 · 知识时效与缺口（①⑨）

### ① 过期 verified 知识仍背书产品宣称

**现状（亲核）**：`src/agent/guards.rs:308` `is_verified` 只判 `integrity_status.eq_ignore_ascii_case("verified")`，**忽略 `valid_to`**。`compute_verified_chunks`（:344）是唯一调用方。过期知识仍能通过 grounding 闸背书报价/产品声明。

**修复**：`is_verified` 增加时效判定：
```rust
// 过期判定：valid_to 为 None（永久有效）或 valid_to >= now 才算有效。
let not_expired = chunk.valid_to.map_or(true, |vt| vt >= now);
integrity_status_verified && not_expired
```
`is_verified` 需要 `now` 参数（当前可能无）——签名调整见"接口调整"。`compute_verified_chunks` 透传 now。

**测试**：纯函数真值表——`(verified, no valid_to)` → 有效；`(verified, valid_to 未来)` → 有效；`(verified, valid_to 过去)` → **无效**；`(draft, *)` → 无效。

### ⑨ 知识缺口被拦但不记录 → 运营不知道缺什么

**现状（亲核）**：`src/agent/review/gates.rs:656` 产品宣称被 `blocked_unverified_product_claim` 拦时只发瞬时 `agent_events kind="product_claim_blocked"`（details 仅 used_knowledge_ids + 总数，**不含客户问句**）。基础设施齐全但断链：
- `knowledge_gap_signals` 集合存在（gap_signals.rs），统一收件箱已展示 `source=gap_signal`（ask_human_inbox.rs:172）。
- `recall_miss`（gap_signals.rs:567 `persist_recall_signal`，**携带原始 query**）只由知识 agent 诚实弃答触发（knowledge_agent.rs:291，cited==0），**不覆盖 R5.4 产品拦截**。
- `suggestion` 靠 `blocked_count_30d` 累加，但需当次有 selected chunk；R5.4 典型场景 verified_chunks 为空（无命中 chunk）→ 永不触发。

**修复**：在 R5.4 拦截点（gates.rs:656 附近）复用 `persist_recall_signal` 写一条 gap_signal，携带客户当前问句/产品主题。写库后立即在统一收件箱可见（**零新载体**）。闭环其余环（运营补录 chunk、verify draft→verified、收件箱 resolve/sweep 端点）已存在。

**fail-soft**：gap_signal 写失败不阻断主流程（回复路径已决），仿 `agent.dimension_dropped` 的 `let _ =` 审计写。

---

## C 组 · 决策写入可靠性（⑥⑧）

### ⑧ 画像 customer_stage 写入无状态机合法性门

**现状（亲核）**：`src/agent/gateway.rs:3076` `insert_domain_signal_values` 写 `customer_stage` 时只做 `stage_changed` 判断（:3075 prev != new），**不过状态机校验**。而同源派生的 `operation_state`（:3149 起）已经过 `check_state_transition`。两者同属一套 canonical id 空间（m006 一一对应），stage 却能被 LLM 任意跳转（如 new_contact → closed_won 直跳）→ 与 operation_state 漂移。

**修复（轻量护栏，状态机是配置驱动的客观约束，不违反 agent-first）**：`customer_stage` 写入前过 `check_state_transition`（复用现有状态机字典 `operation_domain_configs`）：
- 合法跳转 → 照常写。
- 非法跳转 → **fail-soft skip**（保持旧 stage，不阻断已发回复）+ 记 `agent.stage_transition_rejected` 事件，与 operation_state 的 `operation_state_transition_rejected`（已存在）对称。

**复用**：派生 operation_state 的 `check_state_transition` 调用（:3149）已加载状态机，stage 校验复用同一字典查询，避免二次加载。

### ⑥ 承诺兑现依赖 LLM 填字段（降级处理）

**现状（亲核）**：`src/agent/gateway.rs:3116` 结构化承诺已有 due_at 兜底（LLM 没填 due_at → 回落 planner created_at）。真缺口是 **LLM 在 reply_text 里口头承诺却没填 `last_commitment` 字段** → 无 follow-up 兑现。

**定性**："reply 里是否做了承诺"是语义判断，确定性代码护栏做不到、强做会违反 agent-first 且引入误判。**降级方案**：
1. **prompt 侧强化**（主）：在 decision prompt 强调"凡向客户做了任何时间相关承诺，必须填 `last_commitment`/`commitment` 结构化字段"。改 prompt 须 bump `PROMPT_PACK_VERSION`（参照 referral-card 教训）。
2. **代码侧观测事件**（辅，不硬拦）：当 reply_text 含时间承诺特征（如出现日期/"明天"/"下周"等且 commitment 字段为空）时记一条 `agent.commitment_field_missing` 观测事件，供运营事后观测 prompt 是否生效。**不阻断、不改写**。

> 注：观测事件的"时间承诺特征"检测本身是弱启发（非红线护栏），仅用于观测覆盖率、不进任何门。若运营观测显示 prompt 强化已足够，此观测事件可后续移除。

---

## D 组 · 跟进节奏（⑦）

### ⑦ 唤醒任务在整点爆发

**现状（亲核）**：静默期延后的主动发送/延后入站全部重排到 `next_wake_at`（`src/agent/quiet_hours.rs:78`，三处调用：gateway.rs:686、gateway.rs:1551、webhooks.rs:661）。同一 workspace 多客户的唤醒时刻全部 = 次日 quiet_hours_end（如 8:00）→ 整点齐发，像机器人、撞发送峰值（与 E 组 ④ 账号限流叠加风险）。

**修复**：`next_wake_at` 加 **per-contact 确定性 jitter**：
- 纯函数 `next_wake_utc_ms`（:55）增加 `jitter_ms: i64` 入参，加到结果上（保持纯函数可测）。
- 包装 `next_wake_at` 增加 `jitter_seed: &str`（contact wxid）入参，内部用稳定 hash（如 wxid 字节和 % 窗口）派生 0..`wake_jitter_max_seconds` 的确定性偏移。
- 三处调用点传入 contact.wxid（已确认三处都能拿到 contact 标识）。

**确定性**：同一 contact 永远算出同一 jitter（可测、可复现），不同 contact 散开。纯函数测试：相同 seed 同输出、不同 seed 落在 [0, max] 内、分布散开。

---

## E 组 · 账号健康防护（④⑪）

### ④ 无账号级发送总量闸（防封号）

**现状（亲核）**：`src/agent/gateway.rs` `daily_touch_count`（:2631 附近）以 `contact_wxid` 过滤 = 仅 per-contact 频控，无账号级当日总发送量限制。

**修复（"软上限只告警"）**：`send_outbound_message`（gateway.rs:2203）发送前查该 account 当日总发送量：
- 计数源复用 `agent_send_outbox` 集合（类型 `OutboxEntry`，models.rs:2476）——已有 `account_id`（:2480）、`status`（:2500，`sent` 为已送达）、`sent_at`（:2515）。当日总量 = `account_id + status=sent + sent_at >= 当日起点` 计数。
- 超 `account_daily_send_soft_cap`（配置，默认值见"配置"）→ 记一条 `agent.account_daily_send_soft_cap_exceeded` warning 事件。
- **不拦、不排队、不降级**——观测先行、零误伤。后续若需硬限再独立评估。

### ⑪ 掉线账号盲发

**现状（亲核，含校准）**：
- `WechatAccount.online: bool`（models.rs:70）字段存在但**无持续数据源**：webhook 收到 `TypeName=Offline` 直接 ack 丢弃（webhooks.rs:330-338），仅 `POST /accounts/sync`（routes/accounts.rs:113，手动、无定时器）刷新。
- 发送主链路 `send_outbound_message`（gateway.rs:2209）发送前**完全不查 online**。
- 重试有 3 次上限 + 几何退避（outbox.rs:457，10s→20s），**不构成风暴**——但掉线期间盲发、且不区分掉线与普通失败。

**修复（"掉线不盲发"）**：
1. **建状态源**：webhook 收到 Offline 事件（webhooks.rs:330）→ 落库 `WechatAccount.online=false`（+ 上线事件落 true，若 payload 有）。不再静默丢弃。
2. **发送前 gate**：`send_outbound_message`（:2209）发送前查 `account.online`；掉线 → defer（reschedule，不盲发）+ 记 `agent.send_deferred_account_offline` 事件。online 恢复后照常发。

> 注：online 字段的"陈旧"问题（手动 sync）由本修复的 webhook 落库部分缓解——Offline 事件实时落 false，比纯手动 sync 可信。完整心跳/SSE 机制超出本波范围。

---

## F 组 · 多模态入站地基（⑤，代码地基 + 外部依赖打桩）

**现状（亲核）**：
- vision 调用底层**已存在**：`llm.rs:525 generate_json_once_openai_vision`（OpenAI vision 消息体 `content:[{type:text},{type:image_url}]` 已正确构造）+ `generate_json_with_image`（:882 带重试封装，仅 OpenAI 格式）。vision 模型配在 DB `llm_provider_configs.supports_vision`/`is_vision_active`（models.rs:4255/4261），非 env。知识库导入 `import_apply_image`（routes/knowledge/import.rs:540）在用。
- 三大缺口：webhook 不解析 msgType/不取媒体内容（webhooks.rs:472 `media_ref:None` 写死）；决策主链路 `generate_agent_json` 纯文本；语音 ASR 仓内零能力；**MCP 当前无"下载入站媒体"tool**（仓内零调用）。

**本波做（代码地基 + 打桩）**：
1. **入站解析**：webhooks.rs:472 解析 msgType（image/voice/link/miniprogram/...），落 `msg_type` + `media_ref` 字段（不再写死 None）。`conversation_messages` 据此可区分非文本消息。
2. **图片理解封装**：复用 `generate_json_with_image` + import.rs:560-608 的 VisionProvider 选择逻辑，封装一个"描述/理解客户图片"的调用（拿到图片 base64 即可调）。
3. **媒体下载留接口打桩**：定义"拉取入站媒体内容"的接口（trait/函数），实现打桩返回"未接通"——等 MCP server `tools/list` 实证确认媒体下载 tool 后接通（参照 referral-card `message_send_namecard`：实现前必打 server tools/list，仓内零书面依据不能凭空实现）。
4. **过渡话术**：媒体下载未接通 / 语音 / 链接等场景，AI 发自然过渡话术（自治口吻请客户文字补充关键信息），**不硬答空串/原始 XML、不崩**。

**本波不做**：语音 ASR（仓内零能力，需全新外部集成）、媒体下载真实接通（待 server 确认）、决策主链路原生接图片。这些待 ⑤ 完整 feature 独立立项。

---

## 接口调整汇总

| 位置 | 调整 | 原因 |
| --- | --- | --- |
| `gateway.rs` `clear_awaiting_principal_state`（:598 附近） | 提为 `pub(crate)`，escalation 早退分支与 relay 完成分支共用 | ② 避免 dual-path drift |
| `models.rs` `AgentPrincipalEscalation`（:3072） | 新增 `last_holding_reply_ms: Option<i64>` `#[serde(default)]` | ③ 安抚话术去重 |
| `guards.rs` `is_verified`（:308）/ `compute_verified_chunks`（:344） | 增加 `now` 参数，加 valid_to 判定 | ① 知识时效 |
| `quiet_hours.rs` `next_wake_utc_ms`（:55）/ `next_wake_at`（:78） | 增加 `jitter_ms` / `jitter_seed` 参数 | ⑦ per-contact 抖动 |
| `escalation/logic.rs` | 新增纯函数 `relay_introduces_unauthorized_number` | ⑩ 数字白名单 |
| webhooks.rs:472 | 解析 msgType + 落 media_ref | ⑤ 入站地基 |

## 配置项（新增，均带默认值，置于 src/config.rs）

| 配置 | 默认 | 用途 |
| --- | --- | --- |
| `holding_reply_min_interval_hours` | 待定（建议 6h） | ③ 链尾安抚话术去重间隔 |
| `account_daily_send_soft_cap` | 待定（建议保守高值，仅告警用） | ④ 账号级软上限 |
| `wake_jitter_max_seconds` | 待定（建议 900s=15min） | ⑦ 唤醒抖动窗口 |

> 默认值在 writing-plans 阶段定稿（需结合 TASK_WORKER_INTERVAL_SECONDS 与业务体感）。

## Minor（本波附带或记录，不展开）

- 冷唤醒可能唤醒已拒绝客户：⑦ 修复后唤醒散开缓解部分；是否加"已拒绝则不唤醒"判断 → writing-plans 阶段评估，倾向附带小修。
- referral 过度引荐（prompt-only）/ 多账号 persona 一致性：本波不做，记录待独立评估。
- 已确认良好覆盖、不动：记忆边界、跨会话连续性、画像读入决策、grounding 误触、namecard 幻觉自陈、workspace 知识隔离。

---

## 测试与基线

- **纯函数优先**（确定性可测、反过拟合）：① valid_to 真值表、⑩ 数字白名单多形态、⑦ jitter 确定性/分布、⑧ stage transition（复用状态机测试模式）。
- **escalation 集成测试**（testcontainers，`#[ignore]`，CI 跑）：② 授权过期清 awaiting + 发收尾话术、③ 链尾去重发安抚（连续两 tick 只发一条）、⑩ 转述数字拦截 fail-closed。
- **E 组集成**：④ 软上限告警事件、⑪ webhook Offline 落 false + 发送前 defer。
- **守基线**：`cargo test --lib ≥350/0`、四 PBT 累计 ≥33/0、`check-no-human-takeover.{sh,ps1}` lint 0（话术全用 AI 自治措辞）。新增测试只增量叠加，不删改旧维度。
- **磁盘纪律**：本地只跑 `cargo test --lib` + 单 PBT 文件；整合套件交 CI。

## 执行方式

writing-plans 生成分任务计划 → subagent-driven-development 逐组执行（A/B/C/D/E/F 大致对应任务边界，组内按 file:line 拆 bite-sized task）。每任务 fresh implementer + task reviewer（opus）+ fix loop + ledger。提交需用户显式批准。

## 未决细节（writing-plans 阶段定稿）

1. ③ 安抚话术是否过 `push_allowed`（倾向**不过**，仅受 holding 去重间隔约束——它是发给客户不是推领导）。
2. ⑩ 数字 token 提取范围（整数/小数/百分比/金额/中文数字阈值的精确正则与边界）。
3. 三个新配置项默认值定稿。
4. ⑤ 媒体下载接口的精确 trait 形态（待与 referral-card 的 MCP 工具探测模式对齐）。
5. Minor「已拒绝客户不唤醒」是否本波附带。
