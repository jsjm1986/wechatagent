# 客观购买事实增强 G2/G3/G4 设计（数据模型 + spec）

> 状态：**设计评审稿**（2026-06-15）。本轮范围**只定数据模型与 spec**，不写消费侧实现、不写迁移、不实现支付闭环。先评审形态，通过后再分阶段落码。
>
> **落码追记（2026-06-16，commit `fa1215f`）**：本 spec 的 §2-§5 **已落码**（评审稿已转实现）——G2 产品目录 CRUD（`/api/products`、`agent/entitlements.rs`）、G3 成交关联 `product_id` + `event_kind`(deal|reversal)、G4 持有投影 `project_entitlements` + decision.rs 持有段注入、§5.5 疑似线索通道（agentGeneratedSignals→admin 核实）均已实现并进 lib 门。**未落**：§4 DB 迁移脚本独立化、§6 支付闭环、G5 售后/续费时间。下文「不写实现 / 仅预留」等措辞为评审时点表述，按上述追记理解当前状态。
>
> 上游依据：`2026-06-11-universal-domain-adaptation-design.md` §1.6（CRM 客观业务事实缺口）。本文是该节 G2/G3/G4 的落地专题。

## 1. 背景与范围

### 1.1 缺口复述（§1.6 结论二）

现有三个状态字段（`customer_stage` / `operation_state` / `agent_status`）**全是「对话推进状态」**，且都由 LLM 从聊天推断，**没有任何字段回答"买没买 / 买了什么 / 在不在售后期"**。唯一成交锚点 `OutcomeEvent`（旧 `DealEvent`）：

- **无 `product_id`**，只有金额；
- **全代码库只写不读**（`models.rs:201-208` 自述"只采集、不参与任何评分"），是个**假锚点**（H10）。

> **本专题第一性问题不是"怎么建表"，而是"谁读、读了改变什么 AI 行为"。** 若建完仍无消费者，就是再造一个 H10 假锚点。破局点见 §6（G4↔G1 客观锚纠偏）。

### 1.2 本轮范围（用户 2026-06-15 决定）

| 做 | 不做（后续追加） |
| --- | --- |
| 三实体 schema 定稿（Rust 结构草案） | 消费侧实现（决策层读取、prompt 注入） |
| verification 可信度分级模型 | DB 迁移脚本 |
| 与 G1/C2 的咬合设计 | 支付闭环实现（仅预留接口形态） |
| 支付闭环**预留接口**形态 | 前端录入 UI |
| 通用化零扰动论证 + 命名红线 | 索引/路由落码 |

### 1.3 现实约束（决定可行性的硬事实）

- **MCP 通道目前只有 `message_send_text`**（发文本）。支付链接本质是 URL，可经文本发出；原生微信支付小程序卡片需 MCP 侧新增工具 + 商户资质。
- **代码库无任何订单/支付/商户结构**。`OutcomeEvent` 至今"只写不读"、纯 admin 手动标记。
- **`OutcomeEvent` 字面量构造点全代码库只有 2 处**：唯一业务写入 `routes/contacts.rs:583`（`add_deal_event`）+ 测试 `tests/behavior_signal_smoke.rs:127`。其余出现 `outcome_events` 的地方（`mod.rs:630`、`planner/mod.rs:1365` 等 10+ 处）都是 **`Contact { .. , outcome_events: Vec::new() , .. }` 这类把字段初始化为空 Vec 的构造**，不是 `OutcomeEvent` 字面量，不受新增字段影响。
- 这决定了落码工作量边界：新增字段即便加 `#[serde(default)]`，**结构体字面量仍需在那 2 处显式补字段**（serde default 只管反序列化，见 §4.1），否则 `E0063 missing field`。旧库文档与所有 `Vec::new()` 构造点零破坏。

## 2. 核心架构原则：成交真相源可信度分级

成交是 T0 硬事实，**但系统观测不到**——微信私聊入站只有文字，看不到支付/下单。因此「知道成交」有三条路径，可信度递增，**正好镜像项目已有的"AI 永不自证"红线**（知识库 needs_review、观察/解释分层 Iron Law ③）：

| 来源 `verification` | 可信度 | 产生方 | 能否驱动持有状态（G4 投影） |
| --- | --- | --- | --- |
| `conversation_inferred` | 最低 | AI 从聊天推断"疑似买了" | **绝不**。只能当"疑似线索"，触发 AI 去**求证**或运营去**核实** |
| `staff_confirmed` | 高 | 运营后台核实登记 | 可以。**永远删不掉的兜底来源** |
| `payment_verified` | 最高 | 支付回调闭环自动写入 | 可以。带 product_id + 金额，零人力 |

### 2.1 红线：AI 永不自断成交

**AI 最多产出一条 `conversation_inferred` 的"疑似成交"线索，必须经运营核实（`staff_confirmed`）或支付回调（`payment_verified`）才落为"真实成交"。** 这与项目"AI 永不自动 verify 知识"是同一条红线的镜像，与既有方法论严丝合缝：

- `conversation_inferred` 事件**不计入** G4 持有状态投影，只作为"待核实疑似"在后台高亮 + 可触发 AI 主动求证话术。
- 只有 `staff_confirmed` / `payment_verified` 进入 G4 投影。

> **澄清「正例池」措辞（避免与自学习正向循环混淆）**：早前稿写"进入 G4 投影与正例池"，但核对 `reaction.rs:148-153/334-351` 后确认——自学习的**正向 outcome 信号来自 `reaction_outcome_status`**，它读 LLM `reaction_analysis.buyingSignal` flag（**对话推断的反应信号**），经 `active_profile.outcome_polarity.positive` 映射成 token，与客观 `outcome_events` 是**两套完全独立的数据，当前 outcome_events 根本不喂正向循环**。故本专题**不直接**把 outcome_events 接进正例池——那是 H11（outcome_polarity 自学习）的范畴，列为 §9 待办的独立 H11-linkage 项，不在本轮 schema 范围。真要接，也必须**过 `active_profile.outcome_polarity`**：销售域正极是 `user_replied_buying_signal`、情感陪伴域正极是 `user_emotion_opened_up` 这类——**情感域根本没有"成交"语义**，把 G4 成交硬塞进情感域正例池是语义错位。

### 2.2 命名红线（check-no-human-takeover lint）

`verification` 取值与所有标签**不得含** `人工` / `接管` / `takeover` / `hand-off` 等被 `check-no-human-takeover.{sh,ps1}` 扫描的禁词（扫 `src/agent/` `src/routes/` `src/evolution/` `frontend/src/` 新增行）。故用 AI 中性命名：`staff_confirmed`（不是"人工确认"）、`verified`、`pending_verification`。展示层文案同理。

## 3. G2 · 产品目录实体（workspace 级 collection）

### 3.1 为什么是独立 collection，不塞 domain_attributes

产品目录是 **workspace 级共享实体**（一个工作区所有 contact 共享同一份产品表），塞进每个 `Contact.domain_attributes` 是反范式：无法查"谁买了 X"、无引用完整性、改一次价要遍历所有 contact。故新建 `products` collection，遵循项目"稳定字段 + `attributes: Document` 可变容器"哲学（同 DomainSchema）。

### 3.2 结构草案

```rust
/// 客观购买事实增强 G2：workspace 级产品目录实体。admin 录入，agent 报价从此读，
/// 不再靠知识 chunk 的非结构化描述。OutcomeEvent.product_ref 以快照方式引用本表
/// （见 §4），故 product 改名/下架不污染历史成交记录。
///
/// 通用化：无产品概念的行业（情感陪伴/朋友陪伴）该 workspace 产品表为空 →
/// 决策层零注入、零扰动（同 H17 memory_dimensions 空集套路）。
///
/// 命名约定：**不加 `#[serde(rename_all="camelCase")]`**。存储键须与 §3.3 索引
/// （`workspace_id` / `product_id` / `status`）逐字一致；camelCase 会让索引建在
/// 不存在的字段上。沿用 BehaviorSignal 的纯 snake_case 约定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Product {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    /// 业务可读稳定标识（workspace 内唯一，admin 录入或自动生成）。
    /// OutcomeEvent.product_ref.product_id 软引用此值。
    pub product_id: String,
    pub name: String,
    /// 单价（可选——无定价/一口价以外的行业，如定制报价，可留空）。
    /// **最小币种单位整数（分，19900=¥199.00，#6 金额整数化）**——金额全程整数防浮点误差。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,        // ISO-4217，如 CNY（写入校验 3 大写字母形态）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    /// `active` / `archived`。archived 不再出现在 agent 可报价集合，但历史成交仍可解引用。
    #[serde(default = "default_product_status")]
    pub status: String,
    /// 简短描述（agent 报价时可引用；区别于知识库长文 chunk）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 行业可变字段容器（规格/疗程数/有效期天数/续费周期…）。
    /// G4 售后期/有效期投影规则可读此处的 `entitlement_days` 等键（见 §5）。
    #[serde(default)]
    pub attributes: Document,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

fn default_product_status() -> String { "active".to_string() }
```

### 3.3 索引

- `{ workspace_id: 1, product_id: 1 }` **unique**（workspace 内 product_id 唯一）。
- `{ workspace_id: 1, status: 1 }`（列 active 产品供报价/前端）。

`db/mod.rs` 加 `pub fn products(&self) -> Collection<Product>`；`db/indexes.rs` 加上述索引；`db/migrations` 本轮**不写**（范围所限），落码阶段补。

### 3.4 通用化零扰动

- 空产品表 = 决策层无产品上下文可注入，行为与改造前等价。
- 是否注入产品目录由"该 workspace 有无 active product"**隐式**决定（初版设计）——但落码后审查发现隐式开关不足以防"非交易域 admin 误配产品表 + 登记成交 → '已购买X'裸注入情感对话"。**G4 #5 收口（2026-06-17）改为显式交易域闸** `DomainProfile.transaction_facts_enabled`：仅交易型域（销售/电商/课程）置 `true` 才注入三段交易事实（产品目录 / 持有投影 / 疑似成交指引）；非交易域（情感陪伴/朋友）置 `false`，即便误配产品表也跳过加载、一律空串。默认 `false`（失败方向安全：宁可漏注不可错注），`default_domain_profile` 销售兜底显式置 `true` 保历史等价。该开关已纳入 `RISKY_FIELD_NAMES`（手改已生效血缘时不即时生效，落旁路稿二次确认）。
- **不变量：`transaction_facts_enabled` 是交易事实"所有消费路径"的统一总开关**，不止决策注入一处。交叉审查（2026-06-17）发现 gateway 还有两条旁路曾按旧的隐式信号工作，已一并纳入同一闸：① **G1 `purchase_lifecycle` 纠偏**（gateway 用 G4 持有投影纠正 LLM 推断的购买阶段标签）——此前只看 `purchase_lifecycle` 维度是否「参与决策」，与闸解耦，会造成"非交易域关了注入、却仍用持有事实改写客户阶段"的行为分裂；现追加 `&& transaction_facts_enabled`。② **R5.4 `priced_from_catalog` 报价背书豁免**（命中 active 产品 → 绕过 `blocked_unverified_product_claim`）——此前无条件加载，闸关时强制 `false`（方向更严格、安全）。**新增任何"消费 active 产品 / G4 投影 / 成交事实"的路径，都必须先过此闸**，否则就是新开的分裂口子。注意 `project_entitlements` 的持有投影不只看 active_products、还 fold `outcome_events`（成交快照），故"产品表空→投影恒空"的零扰动论证**不充分**（误录 outcome_events 即可触发），必须靠显式闸而非依赖产品表为空的巧合。

### 3.4.1 金额整数化（#6 财务地基，2026-06-17）

所有表示钱的字段（`Product.price` / `OutcomeEvent.amount` / `OutcomeProductRef.unit_price`）是 **`Option<i64>` 最小币种单位整数（分，19900=¥199.00）**，不用 `f64`——浮点不适合表示钱，未来 LTV/业绩聚合相加零误差。约定：

- **边界（方案 A：分贯穿 API）**：后端存储/计算/API 请求响应全程整数分。**只在两个最终展示点 ÷100 转「元」**：① 前端 `fmtPrice`（CNY 加 `¥` 前缀，固定两位小数）；② AI 决策 prompt 文本（`entitlements::fmt_minor_as_major`，**命门**——若把分值原样喂 agent 会报 100 倍错价，有单测守护）。
- **录入**：前端 input 仍用「元」（`step=0.01`），提交前 `yuanToCents` ×100 + `Math.round`（防 `1.1*100` 浮点）。
- **小数位**：固定 ÷100（分），**不按币种驱动小数位**（不支持日元 0 位/第纳尔 3 位），保持简单。
- **校验**：金额非负（i64 无 NaN/Inf，去掉 f64 时代的 `is_finite`）经 `models::is_valid_minor_amount`；currency 加 **ISO-4217 形态校验**（3 大写字母）经 `models::is_valid_currency_code`，所有写入点（products CRUD + add_deal_event）复用。
- 全新项目无存量 double 文档，无需迁移。


### 3.5 多租户隔离不变量（IDOR 红线，所有新读写点强制）

`Product` 是 workspace 级实体、`outcome_events` 挂在 contact（contact 归属 account 归属 workspace）。项目历史做过 IDOR 扫荡（admin handler 一律按 `current_workspace` 过滤），本专题所有新增读写点**必须延续同一不变量**，否则就是新开的越权口子：

- **G2 CRUD 路由**（`/api/products` GET/POST/PUT/归档）：每个 handler 的 Mongo filter 必须含 `workspace_id: &admin.current_workspace`，**写入时 `workspace_id` 由 admin 会话注入、绝不信前端请求体传入的 workspace 字段**。`product_id` 唯一性是 **workspace 内**唯一（§3.3 复合 unique），跨 workspace 同名 product_id 合法且互不可见。
- **G4 投影 read 端点**：加载产品必须 `products.find({ workspace_id: <当前>, status: "active" })`，**投影函数只 fold 当前 contact 自己的 outcome_events**，解引用 product_ref 只在同 workspace 产品表内查——跨 workspace 解引用必须落空而非串号。
- **决策层注入（§5.2）**：gateway/decision 已按 contact 取数，注入产品目录时同样按 contact 所属 workspace 取 active products，不得全局加载。
- **测试**：新路由必须进 IDOR 隔离测试套件（构造 workspace A 的 admin 读 workspace B 的 product / outcome → 期望空/403），与历史 IDOR sweep 测试同形态。

> 此条是 §9 待办 #2（G2 CRUD）/ #4（注入）/ #6（投影端点）的**横切验收项**，不是单独一步——每个落地后端点都要勾这条。

## 4. G3 · 成交关联产品（OutcomeEvent 演进）

### 4.1 演进策略：新增字段，不改旧字段

`OutcomeEvent` 现有 7 字段（marked_at/occurred_at/amount/currency/source/marked_by/note）**全部保留语义**。新增 2 个带 `#[serde(default)]` 的字段。

**`#[serde(default)]` 只作用于反序列化，不作用于结构体字面量**：`OutcomeEvent` 未派生 `Default`，`#[serde(default)]` 仅让旧库 JSON（无这两个键）反序列化时填缺省值——它**不会**让 Rust 代码里的 `OutcomeEvent { .. }` 字面量自动补字段。故影响分两面：

- **旧库文档 / serde 路径**：零破坏，缺字段反序列化为 `verification="staff_confirmed"`、`product_ref=None`。
- **结构体字面量构造点（§1.3 的 2 处：`routes/contacts.rs:583` + `tests/behavior_signal_smoke.rs:127`）**：落码阶段**必须显式补 `verification` + `product_ref` 两字段**，否则 `E0063 missing field`。`add_deal_event` 写入侧据此把 verification 显式设为 `staff_confirmed`（admin 登记即高可信，见 §4.4）。

### 4.2 结构草案（新增部分）

```rust
pub struct OutcomeEvent {
    // ── 现有 7 字段不变 ──
    pub marked_at: DateTime,
    pub occurred_at: Option<DateTime>,
    pub amount: Option<i64>,             // 最小币种单位整数（分，#6 金额整数化）
    pub currency: Option<String>,
    pub source: String,
    pub marked_by: String,
    pub note: Option<String>,

    // ── G3 新增 ──
    /// 成交真相源可信度（见 §2）。缺省 `staff_confirmed`：保持旧库语义——
    /// 历史 outcome_events 全是 admin 手动登记的高可信成交，缺字段即视为已核实。
    /// 新写入的 conversation_inferred 必须显式标注，绝不缺省成"已核实"。
    #[serde(default = "default_outcome_verification")]
    pub verification: String,    // conversation_inferred | staff_confirmed | payment_verified
    /// 关联产品的**订单式快照**（成交当时拷贝 product 名/价/sku），而非活引用。
    /// product 后续改名/下架不污染历史成交正确性（订单系统标准做法）。
    /// `None` = 无产品语义的成交（无产品行业）或未指明产品的旧记录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_ref: Option<OutcomeProductRef>,
}

fn default_outcome_verification() -> String { "staff_confirmed".to_string() }

/// G3：成交事件上的产品快照（不是活引用——见 §4.3）。
/// 注意与 §3.2 Product 的区别：本结构是**嵌入** `OutcomeEvent`（已 camelCase）的子文档、
/// **无独立索引**，故保留 `camelCase` 与容器一致即可，不存在"索引建在错误字段名"的风险
/// （G4 投影是运行时对反序列化后的 outcome_events 做内存 fold，不按裸 key 查 Mongo）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeProductRef {
    /// 软引用 Product.product_id；product 被删也保留，仅无法再解引用到活实体。
    pub product_id: String,
    /// 成交当时的产品名快照。
    pub name: String,
    /// 成交当时单价快照（与 OutcomeEvent.amount 可不等：折扣/多件）。
    /// 最小币种单位整数（分，#6 金额整数化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    /// 件数（默认 1）。
    #[serde(default = "default_quantity")]
    pub quantity: u32,
    /// G4 #4（2026-06-17 收口）：成交当时冻结的售后/有效期天数快照（来自
    /// `Product.attributes.entitlement_days`）。投影 §5.1 优先读它、仅缺失时回落活产品表，
    /// 故产品 archived 后售后期内的已购客户仍被正确判 in_aftercare。None=无时效/未登记。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entitlement_days: Option<i64>,
}

fn default_quantity() -> u32 { 1 }
```

### 4.3 为什么是快照而非活引用

成交是历史事实。若 `product_ref` 存活引用（仅 product_id，渲染时 join products），则 product 改名/调价/下架会**回溯篡改历史成交记录**——客户半年前买的"基础版 ¥99"会显示成今天的"基础版 ¥199"。订单系统标准做法是**成交即冻结快照**。product_id 仍保留用于"该产品总销量"类聚合查询。

**G4 #4 补充（2026-06-17）**：`entitlement_days`（售后/有效期天数）同属"成交当时口径"，故也纳入冻结快照。初版只快照了 name/price，`entitlement_days` 仍实时读活产品表——产品 archived 后解引用落空 → `in_aftercare=None`，售后期内的已购客户被误判"无时效"（AI 可能拒绝售后/重新推销）。修复后每笔成交各自冻结 days 快照、仅缺失时回落活表，且改产品配置不再回溯篡改历史客户的售后判定（与"成交即冻结"哲学一致）。

**G4 #4-A 续费续窗（2026-06-17 交叉审查补）**：售后到期**不锁死最早一笔**。`owned_since`（取最早正向成交）只承载"客户资历展示"语义；**售后到期 = 各正向成交锚 `max(occurred + 该笔 days)`**——续费/复购同一产品时每笔各续一段窗、取最晚到期。否则刚续费的客户会按首购时刻判过期（如首购 Day0 续费 Day25、各 30 天，错误算法到期 Day30，正确应到 Day55），与本 feature 要消灭的失败模式同形。`reversal` 不是购买时刻 → 不贡献到期锚（只抵消净件数）。该缺陷先于 #4 存在（owned_since 一字段兼顾资历+到期锚），#4-A 把到期锚与资历解耦根治。

### 4.4 `verification` 缺省取值的安全性论证

缺省 `staff_confirmed`（而非最低档）是**唯一安全选择**：现有 outcome_events 全部由 `add_deal_event`（admin 后台手动登记）产生，本就是高可信成交。若缺省成 `conversation_inferred`，会把历史真实成交降级成"疑似"、踢出 G4 投影——破坏既有数据语义。**新写入的低可信线索必须由产生方显式标注 `conversation_inferred`，永不依赖缺省。**

### 4.5 退款/逆转

成交非单调（退款/撤单）。逆转**不删 OutcomeEvent**（审计完整性红线），而是 append 一条
`event_kind="reversal"` 的反向事件（带 `product_ref` 指明抵消标的）。G4 投影按 `product_id`
抵消**净件数**：全额退款（净 ≤ 0）→ 退出持有投影；部分退款（净 > 0）→ 保留剩余件数。

**已定稿结构（落码完成）**：
- `OutcomeEvent.event_kind: String`（`#[serde(default="deal")]`）——`deal`（正向，缺省）|
  `reversal`（退款）。旧文档无此键 → 缺省 `deal`，存量成交语义零变。
- `amount` 在 reversal 下表示退款金额的**正向量级**（方向由 `event_kind` 承载，仍走非负校验）。
- `owned_since` / 快照名只跟随正向 `deal`（reversal 不是购买时刻，不刷新）。
- admin 直登：`POST /contacts/:id/deal-events` 接 `eventKind` 字段；reversal **必须**带
  `product_id`（无标的的退款无可抵消对象）；reversal 放宽到任意 `status` 产品（要能抵消
  成交后才下架的产品），正向成交仍只认 `active`。
- 投影实现见 `agent/entitlements.rs::project_entitlements`；零扰动（无 reversal → 行为等价）。

## 5. G4 · 当前持有状态（派生视图，不独立落库）

### 5.1 为什么派生而非存储

G4 持有状态（entitlement）的**真相源就是已核实的 outcome_events**。独立落一份 `Contact.entitlements` 就会与 outcome_events **drift**——这正是 C2（`operation_state` 派生自 `customer_stage` 防 drift）已验证踩过的坑。故 G4 是**运行时投影函数**，不是存储字段：

```
entitlements(contact, products, profile)  =  fold over
    contact.outcome_events
      .filter(verification ∈ {staff_confirmed, payment_verified})   // §2.1 红线（conversation_inferred 不进投影）
      .filter(has product_ref)
    → 按 product_id 聚合持有 + 抵消退款（§4.5）
    → 售后到期 = 各正向成交锚 max(occurred + 该笔 days)；days 优先成交快照 §4.3、缺失回落 product.attributes（续费续窗，§4.3 #4-A）
```

输出形如：`[{ product_id, name, owned_since, in_aftercare: bool, expires_at: Option }]`，注入决策 prompt。

**投影上限（防撑爆 RunBudget）**：`entitlements_text` 注入决策 prompt，直接吃 token 预算（`RunBudget`，CLAUDE.md 硬规则）。重度复购客户的 outcome_events 可能积累几十上百条，全量注入会顶爆预算 / 挤掉其他上下文。故投影输出**必须设上限**，沿用项目既有节流惯例（intent_trajectory `take(5)`、deprecated_facts `take(5)`、memoryCard cap 表）：

- 按 product_id 聚合**去重后**，只注入当前仍持有 / 售后期内的条目（已退款抵消、已过期的不进 prompt）。
- 仍超量时按 `owned_since` 倒序 `take(N)`（N 落码阶段定，量级同 5–10），并在段尾标注"等共 M 项"让 agent 知道有省略。
- 这是软上限、只影响 prompt 注入；G4 投影 read 端点（§9 #6，给前端「客户持有」Tab）可返回全量，不受此 N 限制。

### 5.2 投影时机与落点（本轮只定形态）

注入分**两段职责**，别混为一谈（核对 `decision.rs:299-346`）：

- **DB 读取 + 投影计算**发生在 gateway 装载上下文阶段（与加载 contact / knowledge_chunks 同处）。`build_decision_prompt` 是纯 prompt 拼接函数、**自身不查 Mongo**，所以 `products` 必须由调用方先 `state.db.products().find({workspace_id, status:"active"})` 取好、对 `contact.outcome_events` 跑 §5.1 投影函数，把结果**作为新入参**传进 `build_decision_prompt`。
- **prompt 段拼接**落在 `build_decision_prompt` 内，与 `intent_trajectory_text`（`decision.rs:346`）**同位置**追加一个 `entitlements_text` 段。空投影 → 空串 → 与改造前字节等价（同 intent_trajectory 老文档向前兼容路径）。

> 修正：早前稿误写"投影发生在 gateway 装 prompt 时（与 intent_trajectory 拼接同位置）"——把 DB 读取和 prompt 拼接两件事混在了 gateway。实际 intent_trajectory 的**拼接**在 `decision.rs:346`（函数内），gateway 只负责把数据喂进来。G4 同构：gateway 读 + 投影，`build_decision_prompt` 拼。

- "售后期/有效期"判定规则读 `entitlement_days`（G4 #4 起**优先读 `OutcomeProductRef` 成交快照**，缺失才回落 `Product.attributes`；#4-A 起售后到期取**各正向成交 `max(occurred+days)`** 而非锁死首购）+ profile；无规则时只输出"已购买 product X"不带时效。
- 具体投影函数签名 / `build_decision_prompt` 新入参 / prompt 文案**落码阶段定**，本 spec 只锁定"派生不存储 + 只认高可信 verification + DB 读在 gateway / 拼接在 decision"三条不变量。

### 5.3 做到这步 AI 行为立刻变（破"只写不读"诅咒）

一旦决策层读到 G4 投影：

- 不再向**已购客户推首单**（识别 owned）；
- 能识别**售后场景**（in_aftercare → 切关怀/续费话术而非拉新）；
- 报价从 G2 产品目录读**准确价格**，不再靠知识 chunk 模糊描述编价。

这是整个专题的 ROI 所在——G2/G3 是数据底座，G4 投影是**第一个真实消费者**，让这套数据脱离 H10 假锚点命运。

### 5.4 G2 定价与 `blocked_unverified_product_claim` 红线的交互（关键边界）

§5.3 第三条"报价从 G2 读准确价格"**直接撞上**项目最硬的一条产品声明红线，必须先把交互定清楚，否则落码会二选一地踩坑：要么 G2 价格被红线拦死（报不出来），要么绕过红线（破坏 verified 背书原则）。

**红线现状（核对 `review/gates.rs:607-671` + `guards.rs:304` `compute_verified_chunks`）**：

- R5.4 强约束：当 reviewer `claim_analysis.requiresProductKnowledge=true`，`compute_verified_chunks(used_knowledge_ids, knowledge_chunks)` 为空 → `blocked_unverified_product_claim`。
- `compute_verified_chunks` 的语料**只有 `operation_knowledge_chunks`**，按 `used_knowledge_ids` 取交集再过 `is_verified`。**G2 `products` 是独立 collection，当前完全不在这条计算里。**

**结论：G2 active product 必须被认定为"结构化 verified 背书"的一种，但走的是独立判定，不混入 `compute_verified_chunks`。** 理由与落地约束：

1. **G2 product 的可信度本就 ≥ verified chunk**：admin 在「产品与成交」频道**显式录入**的 product_id/价格/SKU，是结构化、有引用完整性的硬数据，可信度高于人工撰写的非结构化知识 chunk。把"agent 报了 G2 在售产品的准确价格"判成"未验证产品声明"是错杀。
2. **但不能把 product_id 塞进 `compute_verified_chunks`**：该函数的类型与语义是 `&[OperationKnowledgeChunk]`，product 不是 chunk；混入会污染 grounding 语料、破坏"知识切片 verified"的单一语义。正确形态是**在 R5.4 判定处并联一个独立条件**——`reply 引用的报价能在本 workspace active products 里解引用到 → 视为已背书`，与 verified_chunks 取**或**。
3. **落点（落码阶段，本轮只声明形态）**：R5.4 块里，`verified_chunks.is_empty()` 之外再加一个 `priced_from_catalog` 判定（决策引用的 product_id ∈ workspace active products）。两者皆空才 `blocked_unverified_product_claim`。
4. **快照价 vs 活价**：G4 已购客户的历史价来自 `OutcomeProductRef` 快照（§4.3），**新报价**必须读 G2 活表（`status=active`）——别拿快照价当现价报。archived 产品不进可报价集合（§3.2），引用 archived 报新价应触发红线。
5. **零扰动**：无产品行业（情感域）产品表空 → `priced_from_catalog` 恒假 → R5.4 行为与改造前**字节等价**（纯情感回复 `requiresProductKnowledge` 本就为假，根本不进这个块）。

> 这条是 G2 数据"被读"的**第二个消费者**（第一个是 G4 投影），也是 §1.6"破只写不读诅咒"在报价路径上的兑现。落码顺序见 §9 待办 #4（接通消费者）必须连带改 R5.4 判定，否则 G2 价格读了也发不出去。

### 5.5 `conversation_inferred` 疑似线索的 agent 侧落点（当前缺口）

§2.1 红线允许 AI 产出"疑似成交"线索，但核对 `types.rs:82-191` 后确认：**`AgentDecision` 当前没有任何成交 / deal-lead 字段，也没有把数据写进 `outcome_events` 的通道**——`add_deal_event`（`routes/contacts.rs:583`）是唯一写入点，且只能 admin 后台手动触发。所以"AI 产出疑似成交线索"这条**目前在 agent 侧无处落地**，必须在落码阶段补一个落点。本轮只定形态、不实现：

- **不**直接让 AI 写 `outcome_events`（哪怕标 `conversation_inferred`）——那会让 AI 的推断混进客观成交表，与 §2.1"AI 永不自断成交"红线抵触，也违反"画像更新须保守、写侧严谨"。
- 正确形态是**走已有的弱信号通道**：复用 `agent_generated_signals` / `taxonomy_candidates`（自由信号 → 后台审核，不阻断 run）或 `AgentDecision.domain_signals`（`types.rs:110`）发一条"疑似成交·待核实"信号 → 后台「成交记录」Tab 高亮 → 运营点确认后才由 `add_deal_event` 落成 `staff_confirmed` 的真 outcome_event。
- AI 侧另一动作是**主动求证话术**（"方便确认下您是已经入手了吗？"），由决策 prompt 引导，不写库。
- 落码归属：§9 待办 #5（G3 写入升级）的子项——"新增 conversation_inferred 疑似线索通道"，本节明确该通道**终点是后台待核实队列，不是 outcome_events 直写**。

## 6. 与通用化内核的咬合：G4 当 G1 的客观锚

§1.6 把 G1（生命周期：未购买/已购买/售后期/复购期）定为 **profile 维度、由 LLM 从聊天推断**。但 G4 是**客观硬事实**。两者是同一件事的主客观两面。

复用 **C2 已验证的模式**（让客观事实约束主观标签，防 drift）：

- G4 投影显示"已购买 + 售后期内" → G1 的 LLM 生命周期标签**不应**飘到"未购买咨询期"；若 LLM 推断与 G4 客观态冲突，**以 G4 为准**（客观锚优先），并可 emit 一条审计事件（类比 `operation_state_transition_rejected` 的 fail-soft 观测）。
- 这给 G1 落地提供客观输入，两个专题在此咬合：**G1 不是纯 LLM 推断，而是"LLM 推断 + G4 客观纠偏"**。

> 本轮只声明咬合**设计意图**；G1 profile 维度本身在内核就绪后单独落地，G4→G1 纠偏逻辑随 G1 一起实现。

## 7. 支付闭环（预留接口，本轮不实现）

### 7.1 现实代价（已与用户对齐，故后置）

1. **资质门槛**：真实收款需微信支付商户号/支付宝商户，签约 + 主体资质 + 对账。
2. **微信生态限制**：私聊发第三方支付外链易被拦截/风险提示；稳妥要走微信原生小程序/公众号 H5，回到资质问题。
3. **覆盖不全**：大量成交走对公/线下/转账，**不经链接**——支付回调取代不了 `staff_confirmed` 兜底，**两条来源必须并存**。
4. **逆转**：退款/撤单，G4 投影须容忍非单调（§4.5）。
5. **资金合规责任**：碰真实资金即背退款/对账/资金安全责任面。

### 7.2 预留形态（落点已在上面 schema 里）

支付闭环将来作为**可插拔的"高可信来源 #3"** 接入，**无需改 G2/G3/G4 schema**：

- agent 发支付链接：复用现有 `message_send_text`（URL 即文本），无需新 MCP 工具即可起步；原生小程序卡片待 MCP 侧扩工具。
- 支付回调：新增 `POST /webhooks/payment`（类比 `/webhooks/wechat`），收到成功回调 → 写一条 `OutcomeEvent { verification: "payment_verified", product_ref, amount }`。**落点就是 §4 已定的 OutcomeEvent**，零 schema 变更。
- 优先轻量形态（固定收款链接/码 + 订单号回填），不自研全套收银台。

### 7.3 支付回调安全三件套（落码硬约束，本轮先定形态）

支付回调**直接写最高可信度 `payment_verified` 成交**，是整条链里资金语义最重的入站点。任何人能 POST 伪造回调就能凭空写真实成交、污染 G4 投影。必须**复刻 `/webhooks/wechat` 已验证的入站防线**（核对 `webhooks.rs:292-300`）：

1. **回调签名校验（防伪造）**：照搬 `verify_hmac_sha256` 常时间比对模式（`webhooks.rs:1015`）——支付平台回调按各自规范（微信支付 V3 是平台证书/SHA256-RSA，支付宝是 RSA2）验签，**不是**简单 HMAC，故需按所选支付平台的官方验签算法实现，但沿用"raw body 验签 + 常时间比对 + 失败不泄露原因 + `PAYMENT_VERIFY_SIGNATURE` 灰度开关默认开"的同一形态。验签失败直接 401/拒绝，不落任何成交。
2. **订单级幂等键（防重复入账）**：支付平台会重试回调（网络抖动/超时），同一笔订单可能多次到达。必须用平台订单号（out_trade_no / transaction_id）作幂等键，复用 `agent_send_outbox` 已验证的 `idempotency_key` unique 索引模式（`indexes.rs:612`）——新建 `payment_orders` collection 或在写 OutcomeEvent 前查重，**同一订单号只写一条 `payment_verified` 事件**。重复回调返回成功（幂等）而非再写一条。
3. **workspace_id 归属解析（防越权 / 多租户隔离）**：回调本身不带 `wa_session`，无法从 admin 上下文取 `current_workspace`。必须在**发起支付时**把 `(workspace_id, contact_id, product_id)` 编入订单元数据（attach/passback 字段或自管订单表），回调时据订单号反查归属，再把 OutcomeEvent 写到正确 contact 名下。**绝不**信任回调请求体里客户端可控的 workspace 字段（IDOR）。

> 三件套与本轮 schema **零冲突**——它们全在 `/webhooks/payment` 处理器与新订单表里，OutcomeEvent 落点不变。本节只声明形态，具体支付平台 SDK / 验签算法 / 订单表结构落码阶段（§9 待办 #9）随资质先行一起定稿。

## 8. 通用化零扰动总账（与 H10–H17 一脉相承）

| 场景 | G2 | G3 | G4 |
| --- | --- | --- | --- |
| DEFAULT 销售域、老库 | 产品表可空可填；老 outcome_events 缺 verification → 缺省 `staff_confirmed`（语义不变） | 新字段全 `#[serde(default)]`，旧文档零破坏 | 无 product_ref 的旧成交 → 投影空，行为等价 |
| 情感陪伴/朋友域（无产品） | workspace 产品表为空 → 决策层零注入 | 无 product_ref 成交（或无成交） | 投影恒空 → 零扰动 |

**与 H17 同构**：空集 = 回落/零扰动，非空 = 声明本域配置。不引入新硬 flag，靠"有无数据"隐式开关。

## 8.5 前端频道设计（独立频道，本轮只定设计）

### 8.5.1 归属：新建独立频道「产品与成交」

用户 2026-06-15 决定新建独立顶级频道，**不并入「内容资产」**。理由——两者都叫"产品"但分属不同层，混在一个频道会让运营困惑：

| | 「内容资产」频道（现有） | 「产品与成交」频道（新） |
| --- | --- | --- |
| 数据 | **非结构化**产品知识 chunk（话术/FAQ/效果描述/品牌语气） | **结构化**产品实体（product_id/价格/SKU）+ 成交事实 |
| 后端 | `operation_knowledge_chunks` | G2 `products` + G3 `outcome_events` 投影 |
| 性质 | agent 报价的"怎么说" | agent 报价的"卖什么/多少钱/谁买了" |

### 8.5.2 频道注册（落码阶段三处改动，本轮只记）

延续现有前端架构（CHANNELS 单一事实来源 + lazy + Zustand 导航，`frontend/src/app/channels.ts`），加频道很轻：

1. `frontend/src/types/index.ts` — `Channel` 联合类型加 `"productsDeals"`。
2. `frontend/src/app/channels.ts` — `CHANNELS` 数组加一条：
   ```ts
   {
     id: "productsDeals",
     group: "运营",
     label: "产品与成交",
     caption: "Products & Deals",
     icon: PackageSearch,          // lucide，与现有图标族一致
     eyebrow: "Products & Deals",
     title: "产品与成交",
     subtitle: "维护产品目录与价格，登记核实成交，查看客户当前持有与售后状态。",
     Component: ProductsDealsFeature,
   }
   ```
3. 新建 `frontend/src/features/products-deals/index.tsx`（feature 入口，大页头由 Shell 依 channels.ts 渲染）。

### 8.5.3 三个 Tab + 各自后端依赖（决定落码先后）

| Tab | 展示 | 依赖后端 API（落码顺序的前置） |
| --- | --- | --- |
| **产品目录** | G2 产品列表 + 录入/编辑/归档 | `GET/POST/PUT /api/products`（G2 CRUD，待建） |
| **成交记录** | G3 outcome_events 列表，带 verification 徽标（疑似/已核实/支付核实）、product_ref、金额 | `GET /api/contacts/.../outcome-events`（部分已有，需扩 product_ref + verification 字段） |
| **客户持有** | G4 投影：哪些 contact 当前持有哪些产品、是否售后期 | G4 投影 API（待建，派生不落库 → 需新 read 端点） |

### 8.5.4 死页面风险（为什么本轮不写 React）

频道是这条数据链的**最末端消费者**，它依赖的 G2 collection、CRUD 路由、G4 投影 API **一个都还没落地**。现在写 React 只能做假数据空壳——正是项目反复踩的坑（前端体检"后端新逻辑前端未显形"、CSS tree-shake 死页面）。故频道实现**绑定在后端依赖之后**（见 §9 待办顺序），本轮只定设计。

### 8.5.5 verification 徽标文案（命名红线复核）

成交记录 Tab 的 verification 徽标用 AI 中性词，避开 `check-no-human-takeover` lint 禁词（前端 `frontend/src/` 在扫描范围内）：
- `conversation_inferred` → 「疑似成交·待核实」
- `staff_confirmed` → 「已核实」
- `payment_verified` → 「支付核实」

**不得**用"人工确认/人工核实/接管"等词。

## 9. 本轮交付物 vs 后续待办

### 本轮（设计评审稿，已完成）

- [x] 成交真相源三级可信度模型（§2）+ AI 永不自断成交红线
- [x] G2 `Product` 结构草案 + 索引 + 独立 collection 论证（§3）
- [x] G3 `OutcomeEvent` 演进（+verification +product_ref 快照）+ 缺省安全性论证（§4）
- [x] G4 派生视图形态 + 防 drift 论证（§5）
- [x] G4↔G1 客观锚咬合设计意图（§6）
- [x] 支付闭环预留接口形态（§7）
- [x] 通用化零扰动总账 + 命名红线（§2.2 / §8）
- [x] 独立前端频道「产品与成交」设计 + 三 Tab + 后端依赖映射（§8.5）

### 后续待办（评审通过后分阶段落码）

> 频道是数据链最末端消费者，**实现必须排在它依赖的后端之后**（§8.5.4 死页面风险）。下面顺序已按依赖排好。

1. **落 schema**：models.rs 加 `Product` / `OutcomeProductRef` / OutcomeEvent 两字段；`db/mod.rs` 加 `products()` 访问器；`db/indexes.rs` 加索引；写迁移（products collection 初始化、outcome_events 无需回填——靠 serde 缺省）。
2. **G2 后端**：admin 产品 CRUD 路由（`/api/products`）。**强制 §3.5 workspace 过滤不变量**（filter 含 `workspace_id`、写入由会话注入、进 IDOR 隔离测试）。
3. **G2 频道·产品目录 Tab**：新建「产品与成交」频道（§8.5.2 三处改动）+ 产品目录 Tab（依赖 #2）。
4. **接通消费者**（ROI 核心）：gateway 装 prompt 时投影 G4 + 注入产品目录；决策层识别已购/售后/准确报价。
5. **G3 写入升级**：`add_deal_event` 支持 product_ref + verification；新增 conversation_inferred 疑似线索通道（AI 产出 → 后台高亮待核实 → 不进投影/正例池）。
6. **G4 投影 read 端点** + **频道·客户持有 Tab / 成交记录 Tab**（依赖 #4、#5）。**端点强制 §3.5 workspace 过滤不变量**（产品按当前 workspace + active 加载，只 fold 当前 contact 的 outcome_events，进 IDOR 隔离测试）。
7. **退款/逆转事件**结构定稿（§4.5）。**[已完成]** `OutcomeEvent.event_kind`（deal|reversal，serde 缺省 deal）+ G4 投影按 product_id 抵消净件数（净 ≤ 0 退出持有）+ `add_deal_event` 接 `eventKind`（reversal 必须带 product_id、放宽到任意 status）+ 前端成交记录 Tab 显示退款标记。
8. **G4→G1 纠偏**：随 G1 profile 维度落地一起做（§6）。
9. **（独立，H11-linkage）成交事实接入自学习正向循环**。**[已完成]** 当前正向循环只读 LLM `buyingSignal` 反应信号（`reaction.rs`），与客观 outcome_events 脱节。实现：回路①（`gap_signals::refresh_usage_stats_and_confidence`）统计循环新增「成交追认」旁路——已核实成交（`staff_confirmed`/`payment_verified`，经 `entitlements::confirmed_deal_timestamps` 排除 `conversation_inferred`）回溯其发生前最近 N=3 轮 `knowledge_usage_logs`（`attributed_log_indices` 滑窗），把那些 chunk 额外计为正向 Hit。gating：仅 `real_outcome_enabled` 且 `active_profile.outcome_polarity.positive` 非空时启用（销售域接、情感域无成交事件天然不接）；不改 `decision_reviews.outcome_status`（保 reaction 单一写者）、不开新表/worker（30d 全量重算天然幂等）。`UsageStatsReport.deal_attributed_hits` 供审计观测。
10. **支付闭环**（§7，可行性 + 资质先行）：`/webhooks/payment` + 支付链接发送 + payment_verified 自动成交。**强制 §7.3 安全三件套**（验签 / 订单幂等 / workspace 归属解析）。

### 测试预案（落码阶段，遵循"只增量叠加"）

- serde 向后兼容：旧 OutcomeEvent JSON（无 verification/product_ref）反序列化 → verification=staff_confirmed、product_ref=None。
- 快照不漂移：product 改名后，历史 OutcomeProductRef.name 不变。
- G4 投影只认高可信：conversation_inferred 事件不进 entitlement 投影。
- 通用化零扰动：空产品表 workspace 投影恒空、prompt 无产品段。
- 退款非单调（§4.5）：全额退款净件数 0 → 退出持有；部分退款保留剩余件数；超额退款 clamp 不出现负件数；旧文档无 event_kind → 缺省 deal。
