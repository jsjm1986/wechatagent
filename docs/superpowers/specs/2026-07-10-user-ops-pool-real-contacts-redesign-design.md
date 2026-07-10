# 用户运营池「真人漏斗」重设计

**日期**：2026-07-10
**状态**：设计讨论中，待用户复审
**范围**：webhook 建档过滤/富化 + 存量数据一次性治理 + list_contacts 读时富化 + 前端 ContactsView 漏斗工作台。跨后端+migration+前端三层，但都围绕同一目标：把运营池变成「主动来找过你的私聊真人」的可理解漏斗。

## 背景与问题（117 生产亲验）

「用户运营池」（`smart` 模式 `ContactsView`，数据源 `contacts` 集合）当前不可用，四个问题：

1. **昵称全是「Demi」**：所有 normal 联系人 nickname = 登录账号自己的昵称。根因（webhooks.rs:1037 亲验 + 117 真实 payload 亲验）：`upsert_webhook_contact` 用 `find_string(payload, ["nickName","nickname","fromNickName"])` 深度递归整个 payload，命中 `raw._mcp.nickName`——这是 gewe-agent 转发信封里的**账号 owner 昵称**（恒 "Demi"），不是发件人昵称。与已修的 FromUserName/Content 遮蔽 bug 同类，nickname 漏修。
2. **全部无头像**：`upsert_webhook_contact`（webhooks.rs:1093-1096）只 `$set` nickname，从不写 avatar_url。
3. **混入 gh_ 公众号 + @chatroom 群**：webhook 建档（webhooks.rs:300-536 全链路亲验）对 `msg_type` 无任何过滤——公众号推文、群消息、任意类型消息，只要通过入口门（非测试/非上下线/非领导/未超限/验签过）且 contacts 查不到，就无脑建成 normal。Phase1 只做私聊，这些不该进池。
4. **认知层不可理解**：三个 tab「已互动/待启用/Agent」+ 每行只有昵称+wxid，新手无法理解这是什么、要他做什么、与通讯录什么区别。

## 数据来源真相（决定设计的前提，全部亲验）

`contacts` 集合只有两个写入源：

| | 来源 | 干净吗 | 字段 |
| --- | --- | --- | --- |
| **normal（待启用）** | webhook 自动建档（webhooks.rs:1030），谁发消息谁进，无过滤 | 脏（Demi/无头像/含 gh_/群） | 仅 nickname(错)+agent_status+时间戳；**无 operation_state/画像/头像** |
| **managed（Agent）** | batch-enable 主动托管（contacts.rs:658-732），从通讯录挑 | 干净（从 roster 候选带 nickname/remark/avatar_url） | 含 operation_state=initial + 画像回填 |

关键事实（117 roster snapshot 亲验）：
- roster 好友项结构 = `{wxid, nickname, remark, avatar_url, sex, is_non_human}`。
- **roster 全量 4832 好友里 gh_ = 0 条、@chatroom = 0 条**。→ 「在不在 roster 里」等价于「是不是能运营的私聊真人」。gh_/群天然不在好友名册。
- 真实 GeWe AddMsg payload 的 `Data.FromUserName.string` 只是 wxid，**发件人昵称/头像根本不在入站消息里**——只在 roster。故 webhook 建档时正确昵称头像唯一来源 = roster。

「记录」边界（webhooks.rs 亲验，供设计参考）：入站消息**完整落库** `conversation_messages`（含 gh_/群/各类型，去重后每条唯一消息记一次；例外：超限流的入站会丢）。normal 联系人**入站不调 LLM**（webhooks.rs:586 `managed` 判断后 normal 直接 return；行为信号 collect_inbound_behavior_signals 是纯算术零 LLM）。只有 managed 才进 gateway 决策/Review（多次 LLM）。→ 待启用档能展示的「消息摘要」是**原始消息文本截断**，非 LLM 智能摘要（符合「未投入 AI 理解」的档位语义）。

## 已定决策（用户逐项拍板）

1. **池子定位**：保留「待启用→Agent」漏斗语义，它是「主动来撩的私聊真人」候选池，有独立价值（非纯 managed 工作台）。
2. **数据源处理**：修好自动建档——只留真人 + 按 roster 富化。
3. **真人判据**：只拦硬特征黑名单（`gh_` 前缀 / `@chatroom` 后缀），其余都建档；能在 roster 查到就富化 nickname/avatar，查不到先存 wxid 后续自愈。**不采用「必须 roster 命中才建档」**（roster 是 >24h 异步快照，会漏掉刚加的新好友）。
4. **存量**：一次性 migration 回填 + 清理已有 66 条脏记录。
5. **认知层**：漏斗工作台（顶部定位说明 + 每档人话副标题 + 空态引导），非仅加提示。
6. **每行显示**：分档差异化——待启用行显示原文截断摘要（不调 LLM），Agent 行显示运营阶段徽章。

## 架构与四层改动

定位：**运营池 = 主动来找过你的私聊真人漏斗**。待启用（评估是否开 AI）→ Agent（AI 正在自动运营）。区别于通讯录（全量好友名册）。

### 改动 1：入口层 —— webhook 建档过滤 + 富化

改 `upsert_webhook_contact`（`src/webhooks.rs:1030-1121`）：

1. **立「主动联系」标准（黑名单过滤）**：函数入口判断 `wxid`——`wxid.starts_with("gh_") || wxid.contains("@chatroom")` → 直接 `return Ok(None)`，不建档。调用点（webhooks.rs:533-540）已有 `let Some(contact) = contact else { return Err(...) }`——需改成 None 时**优雅跳过**（消息已落库，只是不进运营池、不触发 managed 流水线，因为这类 wxid 本就不是 managed）。**注意**：入站消息仍在 :512 落库，本改动只拦「建 contact」，不拦「记消息」。
2. **昵称改对**：删除 `find_string(payload, ["nickName","nickname","fromNickName"])`（webhooks.rs:1037）。真实 payload 无发件人昵称，nickname 不再从 payload 取。
3. **roster 富化**：拿 wxid 查 `roster_snapshots`（复用 `mcp::read_roster_snapshot` 或直接查集合），命中 → 用 roster 的 nickname + avatar_url 建档；查不到 → nickname/avatar 留空（None），只 `$setOnInsert` wxid + agent_status:normal。
   - `$set` 仅在富化命中时写 nickname/avatar_url（**不无条件 $set None**，避免覆盖已有数据——与 batch-enable contacts.rs:678-698 同一保守原则）。

**为什么对**：roster gh_=0/群=0 亲验，「在 roster」=「真人」；富化与过滤在同一次 roster 读完成，无额外链路。

### 改动 2：存量层 —— 一次性 migration（m029）

新增 `src/db/migrations/m029_cleanup_contact_identity.rs`，`run_step` 语义，追加到 `MIGRATIONS` 列表（`src/db/migrations/mod.rs:77`）。对现有 contacts 一次性治理：

1. **删非真人**：`agent_status == "normal"` 且（wxid 以 `gh_` 开头 或 含 `@chatroom`）→ 删除。
   - **managed 一律保留**（保守：理论上不该有 gh_/群 managed，但若有，只清昵称不删，避免误删运营中数据）。
2. **回填真人**：剩余 contacts 按 wxid 查 `roster_snapshots`，命中 → 写正确 nickname + avatar_url。
3. **清 Demi 污染**：`nickname == "Demi"` 且 roster 未命中 → nickname 置 None（前端回落 wxid，好过错显 Demi）。

**安全边界（硬性）**：
- **只碰 contacts 集合的 nickname / avatar_url / 删非真人 normal 行**。绝不动 agent_status、operation_state、agent_profile、memory_summary、custom_agent_instructions、commitments 等运营数据。
- **只删 contacts 行，不删 conversation_messages**：被删的 gh_/群 contact 在 `conversation_messages` 里的历史消息保留（记录边界不变，只是这些 wxid 不再作为运营池联系人存在）。
- **绝不带 `APP_ENV=production` 守卫**（m011/m012 那种守卫会在非 prod 删数据；本 migration 是数据清洗，必须无条件对所有环境的存量生效）。
- migration 幂等：重复运行结果一致（删已删的无操作、回填同值、清已清的无操作）。

### 改动 3：展示层后端 —— 读时富化 + API 摘要字段

`src/routes/contacts.rs`：

1. **list_contacts 读时兜底富化**（contacts.rs:102-160）：对 nickname 为空 / avatar_url 为空的 contact，按 wxid 左连 roster snapshot 补上（自愈——建档时 roster 未覆盖的新好友，下次列表就显示对了）。同时**读时过滤 gh_/@chatroom**（双保险，防历史残留/migration 遗漏）。
2. **ApiContact 新增最近入站消息摘要字段**（`src/models.rs:3327` ApiContact）：新增 `last_inbound_preview: Option<String>`（最近一条 inbound 的 content 截断，如前 N 字）。
   - **实现方式留 writing-plans 定**：候选 A = list_contacts 为每个 contact 查最近一条 inbound（N+1，66 条可接受）；候选 B = 建档/收消息时把最近消息片段冗余到 contact 上（避免 N+1，但加写路径字段）。设计不锁定，实现时按性能/复杂度权衡。

### 改动 4：前端漏斗工作台 —— ContactsView

`frontend/src/features/user-ops/legacy.tsx:465-521`，遵守 `docs/frontend-design-system.md`（四级层级、muted 灰 #64748b 副标题、teal #0f766e 仅 AI/managed 状态、row-h 62px）：

1. **顶部**（复用现有 `panelHead` 结构，不新建嵌套 panel）：标题「运营池」+ 定位副标题「主动来找过你的人 → 挑价值高的交 AI 接管」+ 一行小字「区别于通讯录（全部好友）：这里只收主动来消息的人」（muted 灰）。
2. **三档 tab**（复用 `segmented`）：待启用 / Agent / 全部，下方一句人话副标题「待你评估是否开 AI 自动回复」。
3. **分档差异化行**：
   - **待启用（normal）**：头像（roster 富化，回落首字母）+ 昵称/备注 + 最近来消息时间（`last_inbound_at` 相对时间「3 小时前」）+ 最近消息摘要（`last_inbound_preview` 原文截断）。
   - **Agent（managed）**：头像 + 昵称/备注 + 运营阶段徽章（`operation_state` 经字典转中文）+ 最近互动时间。
4. **空态引导**：池空时显示「还没有人主动来找你，去通讯录主动开启 Agent 运营」。

**取值优先级**（沿用现有 legacy.tsx:509,513）：`remark || nickname || wxid`。

## 测试

### 后端
- **纯函数单测**（进 lib）：真人判据 `is_operatable_person(wxid)`（黑名单：gh_ 前缀 / @chatroom 后缀）——抽纯函数，断言 gh_/群返 false、wxid_xxx 返 true。webhook 建档与 migration 共用这一个判据函数（杜绝两处漂移）。
- **migration 单测/集成**：构造含 gh_/群/Demi 脏记录 + roster snapshot，跑 m029 后断言：非真人删除、真人回填 nickname/avatar、Demi 未命中清空、managed 不动、operation_state 等运营字段零改动、幂等（跑两次结果一致）。
- **webhook 建档测试**：gh_/群 payload → 不建 contact（但消息落库）；真人 payload + roster 命中 → 建档带正确 nickname/avatar；真人 payload + roster 未命中 → 建档仅 wxid。

### 前端
- ContactsView 契约测试（`frontend/src/__tests__/features/user-ops/`）：三档人话文案渲染；待启用行显示时间+摘要、Agent 行显示阶段徽章；空态引导文案；导入框已移除（延续 PR#166）。

### 验证门（CLAUDE.md 基线，硬性）
1. `cargo test --lib` ≥ 350 passed / 0 failed。
2. `cargo check --tests`（复刻 CI baseline step2）。
3. `cd frontend && npm run build`（TS 编译无悬空引用）。
4. `cd frontend && npx vitest run`（前端契约门）。
5. `scripts/check-no-human-takeover.{sh,ps1}`（新增前端文案不得含禁用词——「待启用/Agent/运营池」均安全）。

## 不做的事（YAGNI）

- 不给 normal 联系人调 LLM 生成智能摘要（保持「未托管不调 LLM」现有语义；摘要=原文截断）。
- 不给 normal 落 operation_state（未运营的人无状态机初态，语义正确）。
- 不改 webhook 入站消息落库逻辑（gh_/群消息仍照常记 conversation_messages，只是不建 contact）。
- 不改 principal 请示通道 / quiet-hours / gateway 决策链路 / batch-enable 托管路径。
- 不改通讯录 RosterView（已正确）。
- 不加分页（运营池规模小）。

## 关联

- 根因 memory：`bug-webhook-contact-nickname-mcp-account-self`
- roster 快照：`project-roster-backend-snapshot-deployed`（roster_snapshots 结构 + read_roster_snapshot）
- 前一轮计数/文案修正：PR#166（已上线，本设计在其基础上继续）
- migration APP_ENV 守卫坑：`prod-app-env-guard-migrations-risk`（本 migration 明确不带守卫）
