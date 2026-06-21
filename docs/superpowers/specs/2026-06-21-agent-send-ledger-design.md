# 主动发送台账（agent_send_ledger）设计

> 簇 A / 8 缺口补全的第 1 个子项目。素材库 + 专属顾问名片引荐两个对称功能共享的"发送事实表"，闭合**缺口 1（效果追踪 / 使用统计）**与**缺口 5（跨 run / 跨天防重发）**。

**Date:** 2026-06-21
**Status:** 设计已获批，待落实现计划（writing-plans）
**Scope:** 仅簇 A。簇 B（标注质量门：override 入口 / 权限分级 / 阶段校验）、簇 C（素材 CRUD 补全）、簇 D（结构化组织：知识库关联 / 标签）各自后续独立 spec。

## 1. 背景与动机

WechatAgent 有两个**形态对称**的主动发送功能：

- **素材库**（content-assets / media-asset）：AI 在私聊中按触发条件主动给客户发图片/文件/视频。
- **专属顾问名片引荐**（referral-card）：AI 在辅助模式下识别高价值客户，主动推真人顾问名片。

两者都是"AI 按触发条件主动发送物 + 人类标注审核"。当前两个功能都**没有任何发送台账**：

- 每次发送只落一条出站 `ConversationMessage`（`msg_type=media|namecard`，`media_ref=资产id`），但全仓无任何"某素材/名片发给过哪些客户、发了几次、发完客户有没有响应"的查询或聚合。运营无法判断哪份素材有效、哪位顾问引荐转化好——是"黑盒投放"。
- 跨 run / 跨天防重发只有 prompt 软约束（名片侧有 `AlreadyReferred` 注入，素材侧连这个都没有），且无统一的"已发历史"数据源。

本设计新建一张**共享发送事实表** `agent_send_ledger`，让两个功能从"黑盒投放"变成"可度量、可回溯、可注入历史"的运营。

## 2. 已锁定的关键决策（brainstorming 产出）

| # | 决策点 | 选择 | 理由 |
|---|---|---|---|
| D1 | 台账数据源 | **新建专用 `agent_send_ledger` 事实表** | 复用 conversation_messages 聚合无法挂转化字段、每次防重发/统计都要扫大集合且慢 |
| D2 | 防重发语义 | **不加硬门，只强化 prompt 软约束** | 对齐项目 agent-first 立场（偏好 LLM 语义判断、厌词表/硬阈值硬匹配）；硬门会误伤"隔两月又问"的合理重发 |
| D3 | 台账价值重心 | **4 项全做**：单客户发送历史可见 / 素材·名片维度聚合统计 / 发送后转化追踪 / 已发历史注入 prompt | 用户全选 |
| D4 | 转化追踪口径 | **响应率（N 小时内是否回复）+ 阶段推进（发送后 customer_stage 是否前进）** | 都不依赖额外 LLM 调用，确定性、可单测、可靠 |
| D5 | 台账写入点 | **MCP 成功后紧贴 ConversationMessage 落库处写**，同样 fail-soft | 与既成事实纪律一致：落库失败不返 Err（返 Err 会让 dispatcher retry 重发，客户收重复——红线） |
| D6 | 前端可见性 | **独立「发送成效」频道（聚合统计）+ 单客户历史嵌客户页 + prompt 注入无 UI** | 聚合是横向跨实体视图，独立频道符合信息架构；单客户历史是纵向单实体上下文，留客户页内更顺手 |

## 3. 数据模型

新建集合 `agent_send_ledger`。每次 AI 主动发送成功后写一条。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSendLedger {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,

    // 发送物：kind 区分两种对称功能；target_id 为 asset_id 或 card_id（hex）
    pub send_kind: String,                 // "media" | "namecard"
    pub target_id: String,
    #[serde(default)]
    pub target_title: String,              // 冗余快照：素材标题 / 顾问名（统计展示不必回表，且原素材改名/删除后历史仍可读）

    // 触发上下文
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_reason: Option<String>,    // AI 输出的 reason（素材 directive.reason / 名片 namecard.reason）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_stage_at_send: Option<String>,  // 发送瞬间客户阶段快照（阶段推进判断的"前值"）

    pub sent_at: DateTime,

    // 转化追踪（发送时留空，由 worker 回扫填充）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responded: Option<bool>,           // sent_at 后 N 小时内是否有入站消息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_window_hours: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_advanced: Option<bool>,      // 发送后 customer_stage 是否前进
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_evaluated_at: Option<DateTime>,
}
```

**要点：**

- `send_kind` 让素材和名片共用一张表（对称功能、统一统计），查询按 kind 过滤。
- `target_title` 是冗余快照——统计"哪份素材发得多"不必回 content_assets/referral_cards，且原实体改名/删除后历史可读。
- 转化字段发送时为 `None`，由异步 worker 回扫后填（见 §4.2）。写入点保持极简、不阻塞发送。
- `customer_stage_at_send` 记发送瞬间阶段，作为"阶段推进"判断的前值基准。

**索引**（`src/db/indexes.rs`，新增 typed accessor `agent_send_ledger()`）：

- `{workspace_id: 1, contact_wxid: 1, sent_at: -1}` — 单客户历史
- `{workspace_id: 1, send_kind: 1, target_id: 1}` — 素材/名片维度聚合
- `{workspace_id: 1, outcome_evaluated_at: 1}` — 回扫待处理（找 None 条目）

无需 migration（新集合，首次 insert 自动建 + ensure_indexes 幂等）。

## 4. 数据流

### 4.1 写入（发送瞬间，极简、fail-soft）

```
send_outbound_media / send_outbound_namecard
  → MCP 调用成功（文件/名片已送达 = 既成事实）
  → 落 ConversationMessage（现有逻辑，不动）
  → 紧贴着 insert agent_send_ledger 一条（新增）：responded / stage_advanced 留 None
  → 两次 insert 都 fail-soft：失败只 tracing::error! 不返 Err
     （返 Err 会让 dispatcher retry 重发，客户收重复文件/名片——红线）
```

写入点仅一行 insert，不查不算，发送路径零额外延迟。两表写入并列、各自独立 fail-soft（一个失败不影响另一个，也都不影响"发送成功"这个既成事实）。

### 4.2 转化回填（异步、与发送解耦）

转化在发送瞬间未知（客户还没回）。复用现有 `tasks.rs` 后台 worker loop（间隔 `TASK_WORKER_INTERVAL_SECONDS`），加一个回扫步骤：

```
每个 worker tick：
  查 agent_send_ledger 中 outcome_evaluated_at == None
     且 sent_at + response_window_hours 已过 的条目（一次限量 ≤ 200，防积压时单 tick 过重）
  对每条：
    responded     = 该 contact 在 [sent_at, sent_at + 窗口] 内是否有入站 ConversationMessage
    stage_advanced = contact 当前 customer_stage 相对 customer_stage_at_send 是否"前进"
    写回 responded / stage_advanced / outcome_evaluated_at = now
```

**要点：**

- 回扫纯查询 + 时间戳比较，**不调 LLM**，确定性、可单测。
- `responded` 判定 = 查入站消息时间戳，复用现有 `ConversationMessage`，不引新依赖。
- `stage_advanced` 判定：借现有状态机（`operation_domain_configs` 的 stateMachine）的顺序。"前进"定义为：**发送后阶段 ≠ 发送时阶段，且新阶段不在旧阶段的回退集里**（粗略推进，非要求严格线性——状态机是图不是线性序列）。
- 回填幂等：`outcome_evaluated_at` 非空即跳过，同条目被扫两次不重复改写。

## 5. API（新增只读端点，全部带 workspace_id scope 防 IDOR）

```
GET /api/contacts/:wxid/send-history
  → 单客户发送历史：该客户被发过的素材/名片列表（按 sent_at 倒序）
    每条带 send_kind / target_title / sent_at / responded / stage_advanced / trigger_reason

GET /api/send-ledger/stats?kind=media|namecard
  → 维度聚合：每个 target_id 的 发送次数 / 覆盖客户数 / 响应率(responded=true 占比) / 阶段推进率
    按发送次数倒序，即"素材/名片效果排行榜"

GET /api/send-ledger/overview
  → 总览：本 workspace 总主动发送数 / 整体响应率 / 阶段推进率 / top 素材 / top 顾问
    供「发送成效」频道首屏
```

实现：走现有 `AuthenticatedAdmin` + `current_workspace`；聚合用 MongoDB aggregation pipeline（`$match` workspace → `$group` by target_id → 算计数/率）；查询加 limit/索引，避免大 workspace 慢查询。

## 6. 前端

遵循 `docs/frontend-design-system.md` 企业白色基调。**1 个新频道 + 1 处嵌入 + 1 处后端机制**。

1. **新增「发送成效」频道**（`features/send-analytics/`，`channels.ts` 加一项，归"系统"组，对齐现有"运营成效/quality"定位）：
   - 顶部总览卡：总主动发送数、整体响应率、阶段推进率
   - 两个 tab：**素材效果** / **名片效果**
   - 每 tab 一张排行榜表：target_title · 已发次数 · 覆盖客户数 · 响应率 · 阶段推进率，按发送次数倒序
   - 是运营/老板看"哪份素材有效、哪位顾问引荐转化好"的主场

2. **单客户发送历史** — 嵌用户运营的客户对话/画像页：一个"AI 已发送"只读小面板，列出发过哪些素材/引荐过哪些顾问 + 时间 + 是否响应。补齐现状素材侧零可见性。属于"理解这个客户"的纵向上下文，留客户页内。

3. **prompt 历史注入**（后端机制，无 UI）：在 `decision.rs` 组装 prompt 处，查该客户近期 ledger 渲染"已发过的素材/已引荐的顾问 + 时间"注入候选清单上下文，支撑防重发软约束（缺口 5）。素材侧此前完全没有，本设计补齐；名片侧已有 `AlreadyReferred`，统一改从 ledger 取（单一事实源）。

频道接线沿用现有模式（`App.tsx` channel state，无路由库）。命名暂记「发送成效」（caption: Send Analytics）。

## 7. 测试策略

遵项目铁律（纯函数确定性为主、不接受 skip 假绿、新增只 append 不删旧维度、不过拟合单条样本）：

| 层 | 测什么 | 方式 |
|---|---|---|
| 纯函数 | `stage_advanced` 判定（前进/持平/回退三态 + 状态机序） | lib 单测 |
| 纯函数 | `responded` 窗口判定（入站时间戳落在 [sent_at, sent_at+窗口] 内/外的边界） | lib 单测 |
| 纯函数 | 聚合率计算（响应率/推进率分母为 0 不 panic、四舍五入口径） | lib 单测 |
| 向后兼容 | `AgentSendLedger` roundtrip；转化字段全 None 的条目能反序列化 | lib 单测 |
| 回填幂等 | 同条目被回扫两次不重复改写（outcome_evaluated_at 非空即跳过） | lib 单测 |
| 集成（CI / `#[ignore]`） | 发送成功→ledger 落一条；worker tick 回填 responded/stage_advanced；API workspace scope 防 IDOR | testcontainers |

## 8. 边界 / 不做（YAGNI）

- **不做** LLM 情绪归因（已定口径 = 响应率 + 阶段推进，纯确定性）。
- **不做** 实时转化（回填是异步 worker，发送后过窗口才算，可接受延迟）。
- **不做** 防重发硬门（已定 = 软约束，ledger 只喂 prompt 历史）。
- **不改** 现有发送决策逻辑——台账是旁路记录 + 只读消费。
- **不动** ConversationMessage 既有结构（台账并列新表，不改老数据）。

## 9. 红线守卫

- 写入点 fail-soft（落库失败不返 Err，防 dispatcher 重发）——与现有 ConversationMessage 落库同纪律（`media_send.rs` send_outbound_media / `referral.rs` send_outbound_namecard 的"既成事实"注释）。
- 全部 API 带 `workspace_id` scope（防跨租户 IDOR）。
- 回填 worker 不调 LLM、不发消息，纯读 + 回写自己表，零外发副作用。
- 新频道/新代码守 no-human-takeover 禁词（命名用"发送成效 / 响应率 / 引荐"等 AI 内部口径中性词）。

## 10. 未决细节（实现期对齐，非占位）

- `response_window_hours` 默认值：建议 24h，可后续 profile 化，本期硬编码默认。
- `stage_advanced` 的"状态机序"读取：复用 `operation_domain_configs` stateMachine 的 allowedFrom 反推顺序，实现时确认是否有可复用的序工具（若无则在回扫逻辑里就地实现"非回退即前进"的粗判）。
- 写入点是否抽 `record_outbound_send` 统一函数（同写 ConversationMessage + ledger）：实现时按 send_outbound_media / send_outbound_namecard 的重复度决定，倾向各自就地 insert 保持改动小、不强行抽象。

## 11. 与后续簇的衔接

- 簇 B（标注质量门）独立：override 写入入口 / 审核权限分级 / target_stages 校验，不依赖本台账。
- 簇 C（素材 CRUD 补全）独立。
- 簇 D（结构化组织：知识库关联 / 标签）独立。
- 本台账的转化数据未来可作为簇 D 或自演化器（evolution）的输入，但本期不耦合。
