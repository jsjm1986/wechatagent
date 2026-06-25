# 标签可信度改造：人工/AI 分层 + 证据绑定 + 压缩重判 + 贝叶斯评估旁路 设计

> 2026-06-23 brainstorming 产出。
> 关联：[[project_agent_first_no_keyword_filters]]、[[feedback_no_overfitting]]、[[feedback_cautious_profiling]]、[[project_universal_test_coverage]]、`docs/superpowers/specs/2026-06-18-dimension-registry-and-validation-design.md`。

## 问题（用户原话）

> "现在的问题是 AI 对用户的标签我是不太信任的，这个是核心。"

用户不信任 AI 逐轮产出的用户标签。根因经实证核查锁定为三条叠加：

1. **逐轮上下文窄**：Reply Agent 默认只看 `recent_message_limit=12` 条消息（`config.rs:405`），微信私聊碎/短/寒暄密度高，12 条可能没几个字 → 逐轮判断先天偏，是"逐轮不可信"的物理根源。
2. **只增不减**：`merge_tags_union_capped`（`gateway.rs:3113`）union+cap16，**绝不删旧标签** → 早期打错的标签永久粘着，无自我纠正。
3. **无证据、无来源区分**：`Contact.tags` 是裸 `Vec<String>`，无置信度/证据/时间戳；且 admin 人工标签（`contacts.rs:686` 整体覆盖）与 AI 标签**混写同一数组**，无法区分谁是权威。

## 方案总览

把"标签"从"裸字符串、逐轮就地写定、只增不减"升级为**三层分离 + 证据绑定 + 压缩时宽窗口整体重判**，并旁挂一条**贝叶斯评估通道**供用户量化"AI 判断到底稳不稳"。

核心立场（与项目一脉相承）：
- **agent-first**：不引入关键词词表判断语义；强弱证据由"是否锚定客户原话"客观判，不靠 LLM 自称置信。
- **既有范本复用**：三层结构、两阶段归并、OCC 锁、证据锚定、AI-建议→人审，全部已在项目中验证，本设计是"把标签对齐到已验证架构"，非新发明。
- **无存量数据**：全面改造，不做迁移兼容，一步到位。

---

## 第一性约束：人类权威层 AI 永不可改（实证清单）

经两路并行核查，系统中以下人工录入/客观计算事实是**毋庸置疑、实时、AI 不可改不可重判**的。本设计的所有 AI 自动逻辑（逐轮 tally / 压缩 replace / 贝叶斯）**在代码层面就够不着它们**：

| 事实 | 存储 | 现状（已天然隔离） |
|---|---|---|
| 运营起始提示词 | `custom_agent_instructions`（`contacts.rs:603`，≤1000字） | 纯人工，AI 无写路径 |
| 真实成交 | `outcome_events`（`contacts.rs:769`，source="manual"，append-only 逆转不删 `models.rs:269`）→ 派生 `value_tier` | 纯人工硬事实 |
| 用户真实评价 | `/evaluations` ground-truth（`evaluations.rs:38`） | 度量基准，非 AI 判断对象 |
| 字典 | `system_taxonomies`（仅 admin CRUD / candidate approve） | AI 只能产 `taxonomy_candidates` 待人审 |
| 状态机/runtime/公式 | `operation_domain_configs` | 发布后运行时只读 |
| 名片/媒体审核 | `review_status`（强制 draft `referral_cards.rs:82` / `media_assets.rs:181`） | AI 强制 draft，人审才 verified |
| 知识 chunk | `integrity_status`（强制 needs_review，product_fact 永不自动 verified） | AI 永不自我核验 |
| relationship_type | `relationship_type_suggestions`（AI 建议→人审回写 `gateway.rs:3551`） | AI 不直写 contact |

**新增人工标签层**：本设计新增 `manual_tags`，把"人工标签"补齐成与上述同等的权威地位（见下）。

---

## 三层数据模型

### 第一层：人工权威层（AI 代码够不着）

```rust
// Contact 新增
pub manual_tags: Vec<String>,                  // 运营自由文本输入（用户明确选择不强制字典）
pub manual_tags_updated_at: Option<DateTime>,
pub manual_tags_by: Option<String>,            // 审计：哪个 admin
```

- **唯一写入路径**：admin 端点（operation-profile 页新增独立输入区）。
- AI 的任何写路径（gateway / management / 压缩重判 / 贝叶斯）**在代码层面不引用此字段** —— 物理隔离，非纪律约束。
- 自由文本：用户明确选择不走字典校验（取舍：失去跨客户统计能力，换录入灵活性）。

### 第二层：AI 暂定层（逐轮 tally，不驱动行为）

复用 `memory_candidates` collection，新增 `source="tag_observation"`（它本就有 `source` 字段 + `candidates: Vec<Document>`，`models.rs:1092`）：

```
memory_candidates {
  source: "tag_observation",
  candidates: [{
    dimension: "tag" | "customer_stage" | "interest" | ...,
    value: "价格敏感",
    hit_count: i32,                          // tally：跨轮命中次数
    first_seen_turn / last_seen_turn: i32,
    evidences: [{ turn: i32, msg_id: String }],   // 证据存引用
    evidence_strength: "weak" | "strong",
  }],
  status: "pending" | "consolidated" | "ignored_low_score",
}
```

- 逐轮 Reply Agent 的标签判断写这里：**tally 累加 + 追加证据引用**，标 `pending`（暂定）。
- **不进 prompt、不驱动任何行为**（符合"AI 少做标签"的用户意图）。
- 复用现有 `decide_candidate_status`（`memory.rs:1278`）双门留存逻辑：整体分 OR 单条高重要度救援。

### 第三层：AI 确信层（压缩时整体重判 replace）

```rust
// Contact 新增（裸 tags 字段废弃）
pub confirmed_tags: Vec<ConfirmedTag>,

pub struct ConfirmedTag {
    pub value: String,
    pub evidences: Vec<Evidence>,            // 重判时重新指认
    pub confirmed_at: DateTime,
    pub confirmed_by: String,                // "consolidation" | "strong_evidence"
}

pub struct Evidence { pub turn: i32, pub msg_id: String }   // 引用，非摘录
```

- 压缩归并时 LLM 看宽窗口 → **整体重判 replace**（带证据）→ 错的不被保留 = 自动消失。
- 这是纠错主力，根治"只增不减"。

### 裸 `tags` 字段处置

**直接废弃**（无存量数据，不做投影兼容）。下游现有 4 个 prompt 注入点（`decision.rs:711`、`shared.rs:900`、`prompts.rs:799`、`memory.rs:216`）改为读 `manual_tags + confirmed_tags`，注入时**标注来源**：人工标签标"运营确认·权威"，AI 标签标"AI 判断·可能调整"，让 LLM 自行掂量分量。

---

## 证据绑定（存引用，对齐 D2 锚定）

每个 AI 标签/判断必挂证据，**存引用不存摘录**（对齐 `source_anchors` 哲学，`models.rs:1178`）：

```rust
Evidence { turn: i32, msg_id: String }   // 指向 conversation_messages，不拷贝文本
```

> **实证修正（消息结构）**：`ConversationMessage`（`models.rs:485-504`）**无显式 turn 字段**，唯一可靠锚点是 `_id`（ObjectId）—— `message_id`（微信侧）是 `Option`，合成 inbound / 出站消息常为 None，不能当通用锚。故：
> - `msg_id` 存**消息 `_id` 的 hex 字符串**（`ObjectId::to_hex()`）。
> - `turn` 不是 DB 字段，而是该证据消息在"当前喂给 LLM 的窗口"内的**序位**（按 `created_at` 排序后的 0-based 下标），由代码在装 prompt 时编号、回收 LLM 输出时映射回 `_id`。LLM 只认窗口里的序位（更直观），代码侧负责序位↔`_id` 互转。

- **校验 fail-closed**：`msg_id`（hex）必须能 parse 成 ObjectId 且在该 contact 的 `conversation_messages` 查到，否则该标签条目**作废丢弃**（照抄 D2 "锚不上写空、绝不放水"，`knowledge/mod.rs:1046`）。
- **无证据不许写**：照抄 `validated_memory_candidate`（`memory.rs:1288`）强制 evidence 非空 —— 从源头掐掉 LLM 脑补（编不出有效 `_id` 就不能乱贴）。

### 强/弱证据判定（纯函数，可单测，不靠 LLM 自称）

这是 customer_stage 快通道的闸门，必须确定性，避免"LLM 自称很确定"的套娃伪精确：

```rust
fn evidence_strength(evidences: &[Evidence], msgs: &[ConversationMessage], explicit_intent: bool) -> Strength {
    // Strong：证据引用指向客户本人消息(direction=inbound) 且 LLM 标注 explicit_intent=true
    // Weak：仅 AI 从语境推断（指向 outbound 或无 explicit 标志）
}
```

- LLM 只负责：产标签 + 指认证据 msg_id + 标 `explicit_intent: bool`。
- 强弱**在代码侧**按"该 msg 是不是客户发的 + 有无明示标志"算出，不由 LLM 主观给。

---

## customer_stage 双层 + 强证据快通道

`customer_stage` 是唯一驱动硬行为的维度（状态机 `gateway.rs:3470` / 选材 `media_send.rs:45` / 触达 `referral.rs:31` / 再激活预筛 `planner.rs:1964`），按"证据强度门控"分流：

```
逐轮 stage 判断：
  ├─ Strong（客户明示，如"我要签约"原话）
  │    → 立即晋升确信，实时写 domain_attributes.customer_stage
  │    → 走现有 check_state_transition 状态机闸（gateway.rs:3470，非法 fail-soft 跳过）
  │    → 照常驱动状态机/选材/触达
  └─ Weak（AI 推断，无客户明示）
       → 只写暂定层 observations，不动 domain_attributes
       → 不驱动任何行为，等压缩重判确信后才写
```

- 强证据快通道**复用现有 stage 写入链路**（C2 同步 + 状态机校验），零新增硬行为路径——只在它前面加一道"证据强度门"。
- **tags / 自由画像无硬消费**（实证：`Contact.tags` 全仓零 if/match/filter），**无快通道**，一律走压缩慢通道。

---

## 压缩时整体重判引擎

### 复用 memory_candidates 两阶段全套（不新建基建）

逐轮 tally 写入复用 `memory_candidates`（`source="tag_observation"`）。**去重调度门（`memory.rs:1311`）、OCC 版本锁（`occ_memory_filter` `memory.rs:634`）、worker 消费（`tasks.rs:229`）、预算降级（`budget.rs`）全部零改动复用**。

### 与现有 memoryCard 归并合一（搭车）

标签重判搭 `consolidate_contact_memory`（`memory.rs:871`）同一趟车：同一次归并任务里既归并记忆候选、也重判标签候选。同一宽窗口上下文喂一次 LLM 同时产出记忆 + 标签重判，**边际成本近零**；共用 OCC 锁的同一次 contact 写入，无额外并发面。

### 整体重判 prompt（replace 语义）

归并 LLM 输入：**原始宽窗口对话**（见下）+ 当前 `confirmed_tags`（带证据）+ 窗口内全部 tag observations（带 tally/证据）。要求：

```
忘掉旧结论，基于完整窗口重新判定。对每个标签：
  - 保留 / 修正 / 删除（replace 语义，不是 union）
  - 每个保留的标签必须重新指认证据（turn + msg_id）
  - 输出 discarded 列表（被推翻的旧标签 + 原因）
```

- replace 是纠错主力：错标签重判时不被保留 = 消失。
- carry-over 保护：照抄 memoryCard 机制（`memory.rs:336`）防宽窗口冲掉早期关键标签，但标签 carry-over 比记忆宽松（标签本就该随认知更新），具体保留策略实现时定。

### 压缩窗口：喂原始宽窗口对话，按字符预算度量

**实证缺陷**：当前归并 prompt（`memory.rs:976`）只喂"逐轮抽好的候选条目"，**不喂原始对话** → 重判输入质量不比逐轮高，"用宽窗口换稳定"落空。本设计修正：

- 标签重判喂**原始对话**，调用 `load_context_messages`（`gateway.rs:4205` 已存在）。
- **度量改字符预算**（微信碎消息下"条数"不等于"信息量"）：

```
压缩重判窗口 = 取最近消息回溯，累积到 6000 字符 ∧ 60 条，谁先到为准。
  - 垃圾号（全寒暄）→ 60 条上限兜底，不空耗回溯
  - 深聊（长消息）→ 6000 字符上限兜底，不超 token 预算
  - 正常 → ~60 条 ≈ 30 个来回，比现状 12 条深 5 倍
```

- token 留账：归并整次预算 `run_token_budget` 默认 30000（`memory.rs:888`/`models.rs:3303`）；6000 字符 ≈ 4000 token，留足 prompt 骨架 + 当前画像 + 输出空间。
- **运营可配**：两个数加进 `UserRuntimeParameters`（`runtime.rs:19`，从 `OperationDomainConfig.runtime_parameters` 读、前端策略页可改），照现有字段模式带 loader clamp 兜底：
  - `consolidation_window_char_budget`：默认 6000，clamp `[1000, 16000]`（上界与 token 预算留账对齐，防超 `run_token_budget`）。
  - `consolidation_window_max_messages`：默认 60，clamp `[10, 200]`。
  - 取窗口时两者谁先到为准（字符预算保信息量下限，条数防垃圾号空耗回溯）。

### 三条线在归并里的隔离

| 线 | 归并时处理 |
|---|---|
| `manual_tags` | **完全不读不写**（AI 够不着） |
| `confirmed_tags` | 整体重判 replace |
| `bayesian_signals` | **不在归并里动**（逐轮增量旁路，与压缩解耦） |

### 校验落库

- 重判后 `confirmed_tags` 每条证据 msg_id 必须可查（fail-closed，锚不上丢弃）。
- customer_stage 若重判确信 → 走现有 `check_state_transition`（非法 fail-soft 跳过）。
- OCC 冲突 → 候选不消费、retry（照抄 `memory.rs:1171`）。

---

## 贝叶斯评估旁路（纯观测，永不驱动）

一条与生产主路（tally + 压缩重判）**彻底解耦**的评估通道，故意让 AI 逐轮自由更新、可完全重写，全程记录"变动过程"，用置信度走势图量化"AI 判断稳不稳"。

> 与项目铁律"轨迹分只进 ledger 不进门"同构。

### 数据模型（独立字段，不碰主路）

```rust
// Contact 新增
pub bayesian_signals: Vec<BayesianSignal>,    // 最多 6 个槽

pub struct BayesianSignal {
    pub dimension: String,                     // AI 自由发现的维度名
    pub current_value: String,
    pub current_confidence: f64,               // 0~1
    pub locked: bool,                          // 是否已正式占槽
    pub history: Vec<BayesianPoint>,           // 走势图数据源，封顶 100
}

pub struct BayesianPoint {
    pub turn: i32,
    pub value: String,
    pub confidence: f64,
    pub value_changed: bool,                   // 本轮是否重写了标签值
    pub confidence_changed: bool,              // 本轮置信度是否变动
    pub reason: Option<String>,                // AI 可选说明为何改
}
```

### 维度自由发现 + 严谨两阶段占槽

用户强调："超危要严谨，不能因为一句话两句话就判断使用了一个槽位。"

- **6 个槽**，维度由 AI 根据 prompt + 上下文**自由发现**（通用开放，不锚定销售域）。
- **占槽是高门槛动作**（两阶段）：
  1. AI 发现新维度苗头 → **先进暂定观察**（复用 tally 旁路），**不立刻占槽**。
  2. 仅当该维度**跨多轮反复出现 + 证据锚定客户原话（强证据累积）**，命中达门槛（≥N 轮 + ≥M 条强证据）→ 才正式 `locked=true` 占槽，开始画走势线。
  3. 一两句话只是 `hit_count=1` 的暂定观察，远够不到占槽线。
- **槽满处置**：6 槽占满后新维度排队，除非旧槽被淘汰腾位。
- **淘汰也严谨**（对称高门槛）：连续多轮缺席 + 置信持续走低才淘汰（淘汰=该线终止，新维度起新线）。
- **门槛参数可配**：N 轮 / M 条强证据阈值放 `operation_domain_configs`，保守默认（宁慢勿滥）。

### 人格维度不在此旁路（见下节独立结构）

大五人格**不放贝叶斯旁路**。理由：贝叶斯旁路的使命是**观测逐轮抖动**（需逐轮翻动才能画出抖动曲线），而人格是**慢变量、应当稳定**——把"要稳"的东西塞进"专门观测抖动"的通道定位相悖，且逐轮更新人格浪费 token。大五独立成"压缩时人格分析"结构（见下节）。贝叶斯旁路因此回归**纯非人格维度**的自由发现（如"价格敏感度""决策果断度"等行为/态度维度）。

### 逐轮流程

- prompt 注入当前已占槽的贝叶斯标签值 + 置信度。
- LLM 输出更新后的值/置信度，并**自报**"本轮是否改了值、是否改了置信度"（提示词新增项）。
- **AI 可完全重写**（不像主路只能压缩时改）——因不驱动行为，放开纠错才有评估价值。
- 每轮 append 一个 `BayesianPoint`（append-only，history 封顶 100，照抄 cap 纪律）。

### 解耦铁律

`bayesian_signals` **永不进任何 filter / 状态机 / 选材 / 触达门** —— 纯观测。与 `confirmed_tags`（生产）、`manual_tags`（人工）三线互不读写。

---

## 压缩时人格分析（大五 OCEAN，慢变量，严谨方法论）

大五人格是**慢变量**，**只在压缩归并时更新一次**（不进逐轮，省 token），随宽窗口重判一起算。独立于贝叶斯旁路与 tags 主路，自成一条线。

### 数据模型

```rust
// Contact 新增
pub personality_profile: Option<PersonalityProfile>,

pub struct PersonalityProfile {
    pub openness: PersonalityFacet,            // 开放性
    pub conscientiousness: PersonalityFacet,   // 尽责性
    pub extraversion: PersonalityFacet,        // 外向性
    pub agreeableness: PersonalityFacet,       // 宜人性
    pub neuroticism: PersonalityFacet,         // 神经质
    pub updated_at: DateTime,
    pub snapshots: Vec<PersonalitySnapshot>,   // 按压缩周期演化记录，封顶 50
}

pub struct PersonalityFacet {
    pub score: f64,                            // 0~1，该维度倾向强度
    pub confidence: f64,                       // 0~1，证据充分度（样本少必须低）
    pub evidence_refs: Vec<Evidence>,          // 支撑该判断的对话引用
}

pub struct PersonalitySnapshot {              // 每次压缩存一份，供画"人格演化"（粒度=压缩周期，非逐轮）
    pub consolidated_at: DateTime,
    pub scores: [f64; 5],                      // O/C/E/A/N
    pub confidences: [f64; 5],
}
```

### 严谨科学提示词要求（硬约束）

用户强调"大五需要严格科学的提示词输出"。压缩归并的人格 prompt 必须满足：

1. **限定五维 OCEAN**：只输出 openness/conscientiousness/extraversion/agreeableness/neuroticism 五个标准维度，不允许 LLM 自创人格维度（与贝叶斯旁路的"自由发现"相反——人格是严肃量表，维度集是封闭的）。
2. **证据强制**：每个维度的判断必须挂对话引用（`evidence_refs`），无证据的维度 `confidence` 必须为低 / 留空，**不许脑补人格**。
3. **置信度诚实**：明确要求"样本不足时给低 confidence"——从聊天文字推断人格科学上置信本就有限，prompt 必须让 LLM 表达不确定，而非伪装笃定。
4. **行为锚定，非贴标签**：要求 LLM 基于"客户说了什么、怎么说"的具体行为证据推断，禁止从单一事件跳到人格定论。
5. **专属 prompt 模板**：`user.personality_analyzer.system/task`，纳入 `prompt_templates` 版本化（运营可调，AI 不自改），seed 进 prompt pack。

> 科学依据：大五人格（OCEAN）跨文化数十年验证、量表信效度最稳。本设计取其"封闭维度集 + 证据锚定 + 诚实置信"的严谨性，避免把人格判断做成又一个抖动的脑补标签。

### 定位与解耦

- **更新节奏**：仅压缩归并时，随宽窗口重判同批算出（搭车，不额外起 LLM 调用）。
- **不驱动行为**：`personality_profile` 与贝叶斯旁路同属"观测/洞察"性质，**不进任何 filter / 状态机 / 选材 / 触达门**。它供运营理解客户、供 AI 在 prompt 里参考沟通风格（软提示），不做硬决策。
- **演化记录**：每次压缩存一份 `PersonalitySnapshot`，前端可画"人格演化"（粒度=压缩周期）。

---

## 前端：置信度走势图 + 三层标签

1. **置信度走势图**：x 轴=轮次/时间，y 轴=置信度 0~1，每个已占槽维度一条线，消费 `history`。实现前先核实前端有无现成图表组件，无则按 `docs/frontend-design-system.md` 选轻量方案（不自由发挥，遵守设计系统）。
2. **三层标签展示**：人工层（运营确认·可编辑）/ AI 确信层（带证据，只读）/ 贝叶斯评估层（走势图）分区呈现，视觉区分来源。
3. **manual_tags 录入 UI**：operation-profile 页新增独立输入区（自由文本）。
4. **人格画像展示**：大五 OCEAN 五维当前分 + 置信度（雷达图或条形），及"人格演化"（按压缩周期，消费 `snapshots`）。复用上面同一图表方案。

---

## 顺手修（同源治理，用户已批准纳入）

1. **update_profile_note 连带重生成**（`contacts.rs:483`）：admin 写人工备注时当前会触发 AI 连带重新生成 agent_profile/tags/customer_stage —— 与"人工的不可被 AI 改"冲突。修：人工写备注时 AI 只能生成/更新 **AI 层**标签，绝不触及人工层。
2. **management.rs 校验旁路**（`management.rs:902`）：MCP 工具 `update_contact_profile` 写 tags/stage/intent **未过 `validate_dimension_value`**，绕开 AdminWrite/MachineWrite 闸门。修：补维度校验。

---

## 全局约束

- `cargo test --lib` ≥350 passed / 0 failed；四 PBT 累计 ≥33 / 0 不回归。
- 后端编译用 `RUSTFLAGS="-D warnings" cargo check --tests`（磁盘受限，lib 测试断言留 CI 基线门）。
- 新增字段用 `#[serde(default)]` 保证反序列化兼容（虽无存量，习惯保留）。
- **agent-first**：不引入关键词词表判断语义；强弱证据靠"是否锚定客户原话"客观判。
- **no-human-takeover**：新增 UI 文案/状态名避开禁用词（`接管/人工` 等），用 AI-内部语义命名。
- **既成事实纪律**：MCP/业务动作成功后 DB/审计写失败只 `tracing::warn!`，不返 Err（防重发）。
- workspace_id scope 贯穿所有新查询（anti-IDOR）。
- 提交需用户显式批准；精确 `git add` 排除并行产物。

## 风险与回滚

- **最大风险**：压缩重判 replace 误删有效标签。缓解：carry-over 保护 + 证据 fail-closed + bayesian 旁路可对照观测重判质量。
- **占槽滥用风险**：用户已强调严谨。缓解：两阶段高门槛 + 可配阈值 + 默认保守。
- **回滚**：三层为新增字段 + 新增旁路，主路 memory_candidates 复用不改签名；裸 tags 废弃是唯一破坏性改动（但无存量数据，风险低）。
- **窗口成本**：字符预算 + 条数双上限 + token 预算三重兜底，超额走现有 BudgetExceeded 降级。

## 实现分期建议（writing-plans 阶段细化）

考虑到改动横跨数据模型/逐轮/压缩/贝叶斯/前端/顺手修，建议拆成可独立验证的子计划：
1. **数据模型 + 三层字段 + manual_tags 录入**（含顺手修 2 个）
2. **证据绑定 + 强弱证据纯函数 + customer_stage 快通道**
3. **压缩重判引擎 + 宽窗口字符预算**
4. **贝叶斯评估旁路 + 严谨占槽 + 压缩时大五人格分析**（同属压缩/观测线，含 OCEAN 严谨提示词）
5. **前端走势图 + 三层展示 + 人格画像**
