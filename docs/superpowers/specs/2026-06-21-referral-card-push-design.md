# 专属顾问名片引荐能力 — 设计文档

- 日期：2026-06-21
- 范围：Phase 1 用户（私聊）运营域
- 状态：设计已分节确认，待最终审阅
- 关联：发送内核形状对齐 `docs/superpowers/specs/2026-06-21-sales-media-asset-send-design.md`（素材发送，目前 0 行代码落地）；红线沿革见 `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md`（决策请示通道）

## 1. 背景与问题

当前系统定位是**全 AI 自主运营**，CLAUDE.md 红线："无人工接管的精确含义=客户永远只跟 AI 对话、永不直接面对真人"。

现在引入一种**全新的第二运营模式（辅助模式）**：在销售场景下，运营方希望 AI 不只是全程替身，还能在识别出**真正高价值的客户**（如明确要签约成交、要来公司参观）时，**把真人专属顾问的微信名片直接推送给客户**，让客户与真人对接完成临门一脚。此时 AI 退为"辅助助手"角色。

这要求把真人从"幕后"摆到"台前"，与现有红线正面冲突，因此**本设计为辅助模式在红线上开一个受控例外**（详见 §7）。

### 1.1 能力底座核查（已实证）

- **MCP 工具**：MCP server（GeWe，自有服务器）私聊侧提供 `message_send_namecard`（向联系人/群发送名片消息）。这是名片推送的发送原语。本仓 `src/mcp.rs` 是通用 JSON-RPC client（工具名是字符串参数），目前只封装使用了 `message_send_text` / `contacts_search` / `account_list`，**尚未使用 `message_send_namecard`**——这是本设计要新增的发送动作。
- **MCP 精确入参字段名**以 server 侧 `tools/list` 实际 schema 为准（用户负责 MCP 侧）；本设计用占位形态，集成时对齐。
- **高价值识别底座**：decision 阶段已输出 `ConversionReadiness` / `NextBestActionScore` 等公式分，reaction 模块已分析购买信号/反对/停止。"谁是高价值客户"无需从零造——但本设计的触发**不直接读这些分数**，而是走"人类标注 + 提示词注入 + LLM 语义判断"（见 §5）。

## 2. 设计决策（澄清结论）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 定位边界 | 红线需调整。这是全新模式：账号选择启用辅助模式后进入；命中时推真人名片给客户。**仅辅助模式开口子，全自治模式红线一字不动** |
| D2 | 触发机制 | **提示词注入**（非独立语义匹配器）：人类给名片标注自然语言触发提示+适用阶段→系统按结构化条件过滤出候选名片清单注入 decision prompt→AI 在主决策里结合上下文选。让运营 agent 更简单：不新增判断分支、不多跑 LLM 轮次 |
| D3 | 推送后 AI 角色 | 转「已引荐」态 + 被动答疑：AI 不再主动推进成交，客户再问仍答疑（不冷场），临门一脚交真人 |
| D4 | 防重推 | **不设硬上限**，结合上下文的语义判断：把"已推过几次/推了谁/何时推"作为上下文注入，AI 自主决定这轮该不该再推（同一客户多场景可重推） |
| D5 | 名片话术 | 先发一句 AI 口吻铺垫话术（融进 reply_text）+ 名片，再推名片卡 |
| D6 | 推哪个真人 | 多真人 + 语义路由：管理员配多张名片各带标注，AI 按触发提示选合适的人 |
| D7 | 开关粒度 | 账号级开关 + 客户级覆盖（force_on/force_off） |
| D8 | 与素材发送协同 | 名片独立先做、独立建模（不塞进 ContentAsset、跳过文件存储链路）；但发送内核形状（outbox 媒体条目 / dispatcher 分流 / 决策注入模式）严格对齐素材计划，将来自然收敛 |
| D9 | 被推真人 vs 幕后 principal | 不同角色：referral card 的真人要摆到台前给客户加；escalation 的 principal_decider 是幕后决策源客户看不到。两者解耦，可同人可不同人 |

## 3. 总体架构与职责边界

名片引荐是"AI 在对话中除了发文本，还发一个富媒体对象（名片）给客户"——与素材发送（发文件）本质同构，但名片的"内容"是一个真人 wxid，不是磁盘文件，因此**更轻**（无文件存储/上传链路）。

### 3.1 与素材发送的复用边界

| | 复用素材计划（对齐形状，名片先建） | 名片独立（素材不碰） |
|---|---|---|
| 发送内核 | outbox 媒体条目（`media_asset_id` 同款字段语义 + 条目类型标识）、dispatcher 分流点、媒体类型→MCP 工具名映射表 | — |
| 决策注入 | `load_*` + `filter` + `render_candidate_lines` 注入候选清单模式、`AgentDecision` 加 Option 字段、gateway 转 outbox 条目 | — |
| 数据模型 | — | 独立 `referral_cards` 集合（不动 `ContentAsset`） |
| 文件存储 | — | 跳过整个链路（无 media_storage / MEDIA_* 配置 / multipart 上传 / media_upload_base64）——名片无文件 |
| 模式与红线 | — | 辅助模式开关、「已引荐」态、CLAUDE.md 红线调整（名片独有，素材发送无此概念） |

**收敛接缝（将来两者共用）**：outbox 的"非文本条目"抽象 + dispatcher 的分流 match。名片先建这套形状，素材落地时往同一 match 加分支即可，零冲突。

### 3.2 触发模型：提示词注入（D2）

每轮 Reply Agent 主决策本就在读注入的上下文做判断，名片只是多一类可注入候选 + 一个输出字段：

```
人类标注名片(target_stages + send_trigger_hint 自然语言)
  → load_referral_cards 按 workspace/account + enabled + 阶段过滤候选
  → render 成候选清单注入 decision prompt(连同"已引荐历史"上下文)
  → AI 在正常那一次决策里输出 namecard_to_send: Option<{card_id, reason}>
```

判断依据全在人类标注里，改触发策略=改标注（不改代码、不改 prompt 主体）。天然 agent-first（结合上下文+提示自主选，非关键词硬匹配）。

## 4. 数据模型

### 4.1 新建集合 `referral_cards`

```rust
pub struct ReferralCard {
    id: Option<ObjectId>,
    workspace_id: String,
    account_id: Option<String>,        // None=workspace 通用；Some=限本账号
    target_wxid: String,               // 被推真人的 wxid（message_send_namecard 目标）
    display_name: String,              // 管理员可读名/头衔，如"销售总监-老王"

    // ── 注入 prompt 的选择依据（人类标注）──
    send_trigger_hint: String,         // 自然语言触发提示，如"客户明确要签约或要来公司参观时引荐"
    target_stages: Vec<String>,        // 适用客户阶段(来自 system_taxonomies)；空=不限阶段

    // ── 把关 + 启停 ──
    enabled: bool,                     // 管理员启停这张名片
    review_status: String,             // "draft"|"approved"，仅 approved 才被 AI 选(AI 不自我核验红线)
    review_note: Option<String>,

    created_at: DateTime,
    updated_at: DateTime,
}
```

要点：
1. 独立集合，不碰 `ContentAsset`（名片无 file_path/sha256/mime 等，塞进去一半字段无意义）。
2. `target_wxid` 是被推真人，与 `OperationDomainConfig.principal_decider`（幕后决策源）是不同角色（D9），互不耦合。
3. `review_status="draft"` 默认；人类标 `approved` 才允许 AI 选——沿用知识库/素材库"AI 不自我核验"红线。
4. `send_trigger_hint` 即 D2 的"人类自然语言描述"，注入 prompt 给 AI 当选择依据。

### 4.2 辅助模式开关（账号级 + 客户级覆盖，D7）

复用现有挂载点，不新建配置集合：

- **账号级**：`OperationDomainConfig`（`src/models.rs:764`，已有 `principal_decider`/`high_risk_escalation_mode` 先例）新增：
  ```rust
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub assist_mode_enabled: Option<bool>,   // None/false=纯全自治(默认)
  ```
- **客户级覆盖**：`Contact.domain_attributes`（`src/models.rs:164`，已是 `Option<Document>`，dotted-key `$set` 不覆盖其它键），新增常量键：
  - `assist_mode_override`：`"force_on"|"force_off"`（单客户强制开/关）
  - `referred_specialist_at`：已引荐时间戳（§6.3「已引荐」态标记）
  - `referred_card_id`：已引荐推了哪张名片（防重推上下文 + 可观测）

**判定优先级**：客户级 override > 账号级 enabled > 默认关。抽成纯函数 `assist_mode_active(account_cfg, contact_attrs) -> bool` 便于单测。

### 4.3 决策输出扩展 `AgentDecision`

`src/agent/types.rs:80`，Option+default，向后兼容（仿 `escalation_request` 模式）：

```rust
/// AI 决定本轮引荐某专属顾问名片；None=不引荐。
#[serde(default)]
pub namecard_to_send: Option<NamecardDirective>,

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamecardDirective {
    pub card_id: String,             // 从注入清单里选的名片 id
    #[serde(default)]
    pub reason: Option<String>,      // AI 为何这轮引荐(审计用)
    // 铺垫话术融进 reply_text（D5），不单列
}
```

AI 在主决策那一次同时输出 `reply_text`（含铺垫话术）+ `namecard_to_send`，不多跑 LLM 轮次。

### 4.4 outbox 条目扩展（对齐素材计划 Task 6）

`OutboxEntry`（`src/models.rs:2370`）与 `EnqueueRequest`（`src/agent/outbox.rs:126`）：

- 复用素材计划设计的"非文本条目"抽象。名片条目带：
  ```rust
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub referral_card_id: Option<String>,   // 非空=这条 outbox 发的是名片
  ```
  与素材的 `media_asset_id` 是**两个语义独立的可选字段**（名片发 wxid、素材发文件，二者互斥不同时有值）。dispatcher 分流按"哪个字段有值"判定（见 §6.1）。名片先建本字段；素材落地时新增 `media_asset_id`，两字段共存、各管一类，零冲突。**收敛说明**：若将来发送类型变多，再抽 `entry_kind: text|media|namecard` 枚举统一收口（本期两字段足够，不提前抽象——YAGNI）。
- **空 content 放宽**：名片条目 content 可空（名片不带正文）。`enqueue` 校验改为"仅纯文本条目要求 content 非空"，抽纯函数 `content_required_for(referral_card_id) -> bool`（素材落地时扩参为 `(referral_card_id, media_asset_id)`）。
- **幂等键含 card_id**：名片条目 idempotency_key 走 `synthetic:run_id:contact_wxid:referral_card_id` 形态（参照 `outbox.rs:186` synthetic 兜底），否则同 run content 都空会 hash 撞键误去重。

## 5. AI 决策选材（提示词注入落地）

### 5.1 候选名片注入 prompt

新增加载器（仿 `load_context_assets`，`src/agent/decision.rs:1025`）：

```rust
pub(crate) async fn load_referral_cards(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<ReferralCard>> {
    // 过滤: workspace + (account_id=null 或 =account_id) + enabled=true + review_status="approved"
}
```

纯函数过滤 + 渲染（仿素材 `filter_sendable_candidates` / `render_candidate_lines`）：

```rust
pub(crate) fn filter_referral_candidates<'a>(
    cards: &'a [ReferralCard], customer_stage: Option<&str>,
) -> Vec<&'a ReferralCard>;  // enabled+approved，且 target_stages 空或命中 stage

pub(crate) fn render_referral_lines(
    candidates: &[&ReferralCard], already_referred: Option<&AlreadyReferred>,
) -> String;
```

注入形态：

```
可引荐的专属顾问（仅在客户契合触发提示时引荐，没有契合的就不引荐）：
- [card:c1] 销售总监-老王 | 阶段:意向,已成交意向 | 触发提示:客户明确要签约或要来公司参观时引荐
- [card:c2] 技术顾问-李工 | 阶段:方案评估 | 触发提示:客户深入问技术方案/集成细节时引荐
（本客户引荐历史：尚未引荐 / 已于 X 引荐给老王[card:c1]——除非客户出现新的、与上次不同的需求场景，否则不要重复引荐）
```

`AlreadyReferred` 上下文实现 D4 防重推（语义判断，非硬上限）。

### 5.2 prompt 选材指引（agent-first 柔性，无禁词）

`src/prompts.rs` Reply Agent operator/policy 层加（确保不含 no-human-takeover 禁词）：

```
【专属顾问引荐】仅当本账号启用辅助模式时，你可在候选「可引荐的专属顾问」中按需选择引荐给客户，输出到 namecardToSend（{cardId, reason}）。规则：
- 只在客户真正契合某顾问的触发提示时引荐（如明确要签约/要到店/深入技术细节），没有契合的就不引荐（namecardToSend 留空），不要为引荐而引荐。
- 引荐时 replyText 先用你自己的口吻做一句自然铺垫（如"这块我请我们负责人直接跟您对接更高效"），再附名片。
- 只能选候选清单里列出的 cardId，不要编造。
- 已引荐过的客户：除非出现与上次不同的新需求场景，否则不重复引荐；客户再问就正常答疑，不再主动推进成交。
```

## 6. 发送链路与闸门

### 6.1 端到端流程

```
客户消息 → webhook → run_user_operation_gateway
  → 前置检查(managed/cooldown/min-interval/daily-cap)[复用现有，名片不豁免]
  → Reply Agent 主决策(注入了"可引荐顾问清单"+引荐历史)
       ↳ reply_text(含 AI 铺垫话术) + namecard_to_send: Option<{card_id, reason}>
  → 独立 Review(只审 reply_text；名片本身免审——人类已把关)
  → approved:
       1. 文本回复 enqueue(gateway.rs:1768 现有分段循环) ── 先发铺垫话术
       2. namecard_to_send 转 outbox 名片条目(追加在文本循环后) ── 后发名片
  → dispatcher(outbox_dispatcher.rs:556) 按条目类型分流:
       text → send_outbound_message / namecard → send_outbound_namecard
  → send_outbound_namecard 调 MCP message_send_namecard({recipient:客户wxid, 目标真人:target_wxid})
  → 成功后: 落 conversation_messages(msg_type=namecard, media_ref=card_id) + 置「已引荐」态
```

### 6.2 发送前闸门（治"该不该推"）

| 闸门 | 作用 | 复用 |
|---|---|---|
| 辅助模式开关 | 账号级未开 + 客户级无 force_on → 跳过，名片字段视为不存在 | 新增（名片独有）`assist_mode_active` |
| 准入二次校验 | card_id 必须真实存在、`enabled=true`、`review_status=approved`——防 AI 幻觉出不存在/未审名片 | 仿 `validate_asset_sendable` |
| 防重推（上下文判断）| 把引荐历史注入 prompt，AI 结合对话自主决定是否再推（非硬上限）| 注入侧实现，agent-first（D4）|
| 频控/冷却/日上限 | 名片条目和文本一样过 gateway 既有频控，不豁免 | 现有 gateway |

名片**不需要** PressureRisk 等内容闸门（它不是话术、无事实风险）。伴随的 `reply_text`（铺垫话术）照常走完整 Review——堵住"借推名片绕过话术审查"的后门。

### 6.3 「已引荐」态（D3：转态 + 被动答疑）

- 名片发送成功后，gateway 给 `Contact.domain_attributes` dotted-key `$set` `referred_specialist_at`(时间戳) + `referred_card_id`(推了谁)，手法同现有 `AWAITING_PRINCIPAL_DECISION_ATTR`（不覆盖其它属性）。
- 该态下注入 prompt 指引变为"客户已引荐给专属顾问 X，你退为辅助——客户再问答疑，不再主动推进/不重复引荐"。是 **prompt 层行为收敛，非硬开关**（客户真有新问题 AI 照常答，仍 agent-first）。
- 态可观测、可撤销（管理员后台清除标记让客户回正常运营）。

## 7. 红线、命名与定位

### 7.1 红线调整（落到文档）

- **改 CLAUDE.md**：在"无人工接管=客户永远只跟 AI 对话、永不直接面对真人"补受控例外：默认仍 AI 全程；**仅当账号显式开启辅助模式、且 AI 判定命中人类标注的引荐条件时**，AI 才主动把专属顾问引荐给客户。引荐后客户与顾问对接，但这是**管理员显式配置的业务动作、AI 仍是发起方与辅助方**，不是 AI 失控、也不是"接管"。
- **改 `docs/agent-policy.md`**：新增"辅助模式/专属顾问引荐"小节，写清触发依据（人类标注注入）、边界（账号级开关默认关）、与全自治模式关系、「已引荐」态语义。
- **全自治模式红线一字不动**——只为辅助模式开口子。

### 7.2 CI 禁词 lint（不改 lint，从命名绕开）

`scripts/check-no-human-takeover.sh` 把 `人工接管|takeover|hand-off|人工介入|人工托管|接管|人工` 当裸词扫 `src/agent/ src/routes/ src/evolution/ frontend/src/`。名片功能所有标识符/状态名/prompt 文案用 **AI 内部口径**：

- 字段/集合：`referral_cards`、`referred_specialist_at`、`assist_mode_enabled`、`namecard_to_send`、`send_outbound_namecard` —— 不含禁词。
- 状态/文案：用"引荐专属顾问 / 已引荐 / 专属顾问对接"，**绝不**用"转人工/人工对接/接管/handoff"。
- lint 仍是有效防线；新功能从命名天然合规。每个改 `src/agent`/`src/prompts.rs`/`frontend` 的 Task 加禁词自检步骤。

## 8. 前端

遵循 `docs/frontend-design-system.md` 企业白色基调，新增/改造：

1. **名片库管理页**（新建）：
   - 录入表单：display_name、target_wxid（可结合 `contacts_search` 选好友）、send_trigger_hint(自然语言)、target_stages、是否启用；
   - 列表展示 review_status(draft/approved) + enabled 状态；
   - draft 行"标记为可引荐(approved)"按钮——人类把关入口。
2. **辅助模式开关**：账号配置页加 `assist_mode_enabled` 开关 + 说明文案（"开启后，AI 会在客户契合引荐条件时主动推送专属顾问名片"）。
3. **对话消息渲染**：`Message` 类型加 `msgType`（复用素材计划同款字段）；名片消息渲染成"已引荐专属顾问 X"卡片，让运营看到 AI 给客户推了谁。

## 9. 配置与数据库

- **配置**：无新增 env（名片不需要文件存储配置）。
- **数据库**：新建 `referral_cards` 集合 + typed accessor（`src/db/mod.rs`，仿 `agent_principal_escalations` 硬编码字符串）。索引（`ensure_indexes`）：`{workspace_id, account_id, enabled, review_status}`（选材查询）。无需 migration（无 seed 数据；首次访问自动建集合 + ensure_indexes 幂等）。

## 10. 测试策略

遵循项目铁律（纯函数确定性为主、不接受 skip 假绿、新增只 append 不删旧维度、不过拟合单条样本）：

| 层 | 测什么 | 方式 |
|---|---|---|
| 纯函数 | 工具名映射含 namecard、候选过滤(enabled+approved+阶段)、准入校验、`assist_mode_active` 优先级(客户级>账号级>默认关) | lib 单测 |
| 向后兼容 | 旧 Contact(无新 domain_attributes 键)反序列化正常、未开辅助模式时名片字段被忽略、`ReferralCard` roundtrip | lib 单测 |
| prompt 注入 | 引荐候选清单形状、「已引荐」态指引、防重推上下文渲染 | 纯函数测 |
| outbox | 名片条目幂等键(含 card_id)、空 content 放宽、`content_required_for` | lib 单测（对齐素材 Task 6）|
| 发送链路 | enqueue→dispatcher 分流→message_send_namecard 数据流、置「已引荐」态 | 集成测（CI，testcontainers）|
| 真实 LLM | 给定"客户明确要签约/到店"对话 AI 选对引荐；不该推时不推；已引荐后不重复推 | CI real-llm |

baseline 不回归（`cargo test --lib` ≥350/0；4 个 PBT 累计 ≥33/0）；新增测试只 append。触发靠人类标注+LLM 语义，**不写关键词词表**（反过拟合红线）。

## 11. 不做（YAGNI / 范围外）

- 不接入文件存储/上传（名片无文件，与素材发送的本质区别）。
- 不把名片塞进 `ContentAsset`（独立 `referral_cards`）。
- 不在本期合并素材发送与名片为统一"富媒体发送"计划（D8：名片独立先做，内核形状对齐，后续收敛）。
- 不改全自治模式红线（只为辅助模式开口子）。
- 不为引荐而引荐——AI 无契合客户即 namecardToSend 留空。
- 不改 Phase 1 之外的群/朋友圈运营域。
- 名片"被推真人"不复用 escalation 的幕后 principal 角色（D9）。

## 12. 风险与未决

- **MCP `message_send_namecard` 精确入参字段名**（recipient 字段名、目标真人字段名是 `targetWxid` / `cardWxid` / 其它）：以 server 侧 `tools/list` schema 为准，实现时对齐（用户负责 MCP 侧）。本设计用占位形态，不阻塞。
- **被推真人需是业务号好友**：`message_send_namecard` 通常要求 target_wxid 是发送账号的联系人。录入名片时应校验/提示（可借 `contacts_search`）。
- **客户加真人后的"不承认 AI"连带暴露**：客户与真人对接后可能反向确认"之前是 AI 吧"。这是辅助模式的次生暴露面，属真人侧话术问题，本期不做系统强制（真人如何接待超出 agent 代码范围）；在 agent-policy.md 记为已知边界。
- **发送内核与素材计划的收敛**：名片先建 outbox"非文本条目"抽象，素材落地时复用。若素材计划字段命名最终调整，以先落地者为准、后者对齐。
