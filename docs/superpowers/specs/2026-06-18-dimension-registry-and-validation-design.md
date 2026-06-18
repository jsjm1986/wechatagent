# 维度 Registry + Contact 画像校验 设计

> **日期**：2026-06-18
> **基线**：HEAD = `cff6e88`（B 阶段后端轴）
> **来源**：合并审查路线图 `docs/universal-domain-base-robustness-roadmap.md` 的主题 1（扩展点收拢）+ 主题 2（数据完整性），合并为一个 spec。
> **方案**：方案 C —— registry 先行（纯收敛声明，证字节等价）→ 校验作为 registry 第一个消费者（新行为，独立测）。两步独立可验证可提交。
> **流程**：brainstorming（本文）→ writing-plans → 实施。

---

## Context（为什么做）

两份独立审查（扩展轴 audit + 四维架构审查）一致确认：运行时引擎层是真通用的，但**扩展动作分散、Contact 画像数据裸奔**是没收口的两块。

- **扩展点分散（主题 1）**：系统"加一个维度"时没有单一模式。维度元数据散落 ≥7 处——两份重复的 typed 硬编码列表（`KNOWN_TYPED_DIMS` @ domain_signals.rs:27 + `SALES_TYPED_DIMENSION_KINDS` @ domain_profile.rs:974）、4 段 migration 散文注释（m020/m021/m023/m024 顶部写"这维度走不走 LLM 通道/要不要 profile 声明"）、entitlements kind 常量。判断散落 → 加维度易漏步 drift，`objection_type` 就是现成的 drift 样本（声明字典、实现裸 string）。
- **数据裸奔（主题 2）**：`enforce_domain_attributes`（domain_schemas.rs:544，做 alias rewrite + required + enum 越界 reject）**只在 knowledge chunk 写路径调用**（chunk_revisions.rs:245）。Contact 画像三条写入路径**无一过它**：可写 enum 越界值、key 拼写漂移静默落库（到处 `get_str().ok()` 软读、零编译保护）。`DomainProfile.domain_schema_id` 是死字段，现有 schema 机制不服务 Contact。

**意图结果**：建一个中央 `DimensionRegistry` 作单一真相源收敛元数据；Contact 三条写入路径接上从 registry 派生的校验；key 常量化消除拼写漂移；objection_type 补走字典。让"加一个维度"从"散弹改 4+ 处 + 零校验"变成"registry 声明一处 + 自动校验"。

---

## 架构与核心数据结构

新模块 `src/agent/dimension_registry.rs`，中央 const 表，每个画像维度一条 `DimensionSpec`：

```rust
pub struct DimensionSpec {
    pub kind: &'static str,             // "customer_stage" / "value_tier" / ... 唯一 key（常量来源）
    pub channel: DimensionChannel,      // 写入通道
    pub typed: bool,                    // 是否走 typed JSON 镜像（仅 customer_stage/intent_level）
    pub participates_in_decision: bool, // 是否进五闸/状态机（vs 仅观测）
    pub value_source: ValueSource,      // 取值约束来源
}

pub enum DimensionChannel {
    LlmSignals,      // LLM domainSignals 容器：customer_stage/intent_level/purchase_lifecycle/churn_reason
    AdminDirect,     // admin 直写：relationship_type
    GatewayDerived,  // gateway 规则派生：value_tier
    ReactionDerived, // reaction 分析派生：objection_type（第四条隐性通道，强制显式化）
}

pub enum ValueSource {
    Taxonomy,   // 查 system_taxonomies 字典做 enum 校验（customer_stage/intent_level/purchase_lifecycle/churn_reason/relationship_type/objection_type）
    CodeEnum,   // 值由代码产出、信任（value_tier，来自 classify_value_tier）
    FreeText,   // 直通无约束
}
```

**分工原则（决定可行性）**：registry 描述维度的**结构属性（通道/typed/是否决策/约束类型）——这是编译期代码契约，放 Rust const**；每个维度的**具体合法取值（customer_stage 有哪 9 个值）仍在 system_taxonomies DB 字典**（因行业而异、运营可增删）。registry 不抢字典的活，只声明"这维度的取值要去查字典校验"（`ValueSource::Taxonomy`）。registry = 稳定代码结构，字典 = 动态业务数据。

**为什么 Rust const 而非 DB**：通道/typed/是否参与决策改了要动消费代码，不是运营可配项；const 表让"读侧查 registry"有编译期保证，也是 key 常量化的天然落点。

### 当前 7 维度的 registry 声明（收敛自散落点）

| kind | channel | typed | 参与决策 | value_source |
| --- | --- | --- | --- | --- |
| customer_stage | LlmSignals | ✅ | ✅ | Taxonomy |
| intent_level | LlmSignals | ✅ | ✅ | Taxonomy |
| purchase_lifecycle | LlmSignals | ❌ | ✅(示例 profile 声明) | Taxonomy |
| churn_reason | LlmSignals | ❌ | ✅(示例 profile 声明) | Taxonomy |
| value_tier | GatewayDerived | ❌ | ❌(仅观测) | CodeEnum |
| relationship_type | AdminDirect | ❌ | ❌(走 resolve_operation_mode) | Taxonomy |
| objection_type | ReactionDerived | ❌ | ❌(仅观测) | Taxonomy |

---

## 数据校验（registry 的第一个消费者）

从 registry 派生的纯函数：

```rust
pub enum DimValidation { Accept(String), Reject(String /*reason*/), DropSilently }
pub async fn validate_dimension_value(db, kind, raw, account) -> DimValidation
```

查 registry 拿 `ValueSource` 决策：
- `Taxonomy` → 查 system_taxonomies：alias 命中→归一（复用现有 `normalize_dimension_value` @ taxonomy.rs:554 的逻辑）；canonical 命中→Accept；**越界（字典无）→ 按通道处置（见下）**。
- `CodeEnum`（value_tier）→ 信任，不校验。
- `FreeText` → 直通。

**三通道差异化处置（关键：不一刀切 reject）**：

| 通道 | 越界值处置 | 理由 |
| --- | --- | --- |
| **AdminDirect** | **Reject → BadRequest** | admin 是人，当场报错纠正（信息最全） |
| **LlmSignals** | **DropSilently + 审计事件** | LLM 偶发臆造不该阻断已发出的回复；丢弃越界维度+留审计，符合"fail-soft 不阻断已发送"既有红线 |
| **GatewayDerived** | 不校验（CodeEnum 信任） | 值由 classify_value_tier 算，非外部输入 |
| **ReactionDerived** | 归一（objection_type 从裸 string 补走字典） | 现在连归一都没有 |

**LLM 用 Drop 而非 Reject 是和现有架构一致的关键**：现在 LLM 越界值直接落库（脏画像）；改成"丢弃越界维度+审计"，既堵脏画像又不违反"回复已发出后 DB 写失败要降级不能阻断"的铁律（gateway.rs 既有红线）。绝不因一个维度越界让整条已发送回复链路报错。

**与 normalize_dimension_value 的关系**：保留它（不破现有调用点），`validate_dimension_value` 在它之上加越界处置——validate 是 normalize 的超集消费者。

**key 常量化**：registry 的 `kind` 成为唯一 key 来源，读写两侧引 `DimensionSpec` 常量而非裸字面量，编译期消除拼写漂移。

---

## 分步执行计划

### 步骤一 —— registry 纯收敛声明（证字节等价）
- 建 `dimension_registry.rs`：7 维度 `DimensionSpec` const 表全声明。
- 读侧改为查 registry 派生：`KNOWN_TYPED_DIMS` / `SALES_TYPED_DIMENSION_KINDS` / `decision_dimension_kinds` 等从 registry 派生，不再各自硬编码。
- **验证铁律：纯收敛、零行为变化**。registry 派生的 typed 集合 / 决策维度集合必须与现有硬编码逐元素相等——断言测试锁（`assert_eq!(registry_derived, 现有硬编码)`）。不碰任何写入/校验行为。lib 全绿 + DEFAULT 字节等价即可提交。

### 步骤二 —— 校验作为消费者（新行为，独立测）
- 加 `validate_dimension_value`。
- 接三条写入路径：admin（reject）@ contacts.rs、LLM（drop+审计）@ gateway.rs/domain_signals.rs、reaction（归一）@ reaction.rs。
- 新增行为，用新测试覆盖每条通道处置，不影响步骤一的等价性。

---

## 测试策略

- **步骤一**：registry 派生 == 现有硬编码的等价断言（纯函数、确定性）。
- **步骤二**：`validate_dimension_value` 对每个 `ValueSource` × 每条通道处置（Taxonomy 越界→admin reject / LLM drop；CodeEnum 信任；alias 归一）——纯逻辑单测 + 字典 mock。
- 全程守基线：lib ≥350/0、四 PBT ≥33/0、no-takeover lint clean。
- DEFAULT 销售 profile 经全链后 domain_attributes 与改造前逐字节一致（等价测试）。

## 护栏（本项目铁律）

- registry 声明的是**已存在维度的现状**，不是新规则 → 步骤一必然字节等价（只是把散落事实搬到一处）。
- 越界处置是通道级结构规则，不针对任何单条对话/样本（反过拟合）。
- 重构向改动用 subagent 多维交叉验证后再提交（byte-equiv / 回落安全 / 红线）。
- 提交需用户显式批准；commit 精确 add 排除并行会话产物。

## 范围边界（YAGNI）

**本 spec 只解决**：维度元数据收敛到 registry + Contact 三通道接校验 + key 常量化 + objection_type 走字典。

**不做**：registry 放 DB（维度结构是代码契约，非运营可配）；五闸门数量可配（主题 1 子项，不碰闸结构）；driver 框架抽象（主题 1 子项，留后续）；前端显形（主题 3）；规模/多账号/索引/熔断（主题 4）。

## 验证（端到端）

- 步骤一提交前：`cargo test --lib` 全绿 + registry 等价断言测试通过 + DEFAULT 字节等价。
- 步骤二提交前：三通道校验单测全绿 + 基线 + 四 PBT + no-takeover lint。
- 两步各自用 subagent 交叉验证（步骤一验"零行为变化"、步骤二验"校验正确+不阻断已发送+红线"）后再提交。
