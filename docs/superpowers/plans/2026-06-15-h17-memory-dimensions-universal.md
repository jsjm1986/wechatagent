# H17：memoryCard 记忆维度 schema 通用化（彻底通用化路线）

## 目标
把 memoryCard 的"记忆维度"（preferences/objections/commitments/openLoops 等数组槽位 + 它们的 cap + consolidator prompt 骨架 + memoryCandidate type 集）从**销售域写死**改造成**从 DomainProfile 读**，让情感陪伴 profile 能声明"情绪史/纪念日/重要事件"等专属记忆槽。对齐 H5 CoverageDimension / H16 ChunkRole 已建立的"维度随 profile"范式。

**核心护栏**：DEFAULT_PROFILE 逐字复刻当前销售槽位 → 现有所有 PBT/real-LLM 行为零变化（反过拟合硬护栏）。

## 现状（勘察已确认）
- `MemoryCardTyped`（models.rs:2676）：3 个 typed 数组（core/recent/deprecated_facts）+ `extra: Document` flatten 兜底承接所有业务槽位。
- 销售槽位写死在三处：
  1. `compact_memory_card_with_previous` 的 cap 表（memory.rs:424-434）——**无界增长唯一闸口**
  2. consolidator prompt JSON 骨架 + limit 散文（prompts.rs:1194-1232）
  3. Reply Agent memoryCandidates[].type 枚举（prompts.rs:1121-1130，销售化：fact/preference/doNotDo/commitment/objection/openLoop/conflict）
- DomainProfile 已有同款可配字段先例：coverage_dimensions(H5)/chunk_roles(H16)/outcome_polarity(H11)/business_formulas(H15)。

## 设计：新增 `MemoryDimension` + DomainProfile.memory_dimensions

### 数据结构（models.rs）
```rust
pub struct MemoryDimension {
    pub key: String,          // extra 容器里的数组键名，如 "objections" / "emotionHistory"
    pub display_name: String, // consolidator prompt 里的人类标签
    #[serde(default)] pub cap: usize,        // 该槽位数组上限（替代写死的 limit_extra_array）
    #[serde(default)] pub is_core: bool,     // 是否核心维度（注入 prompt 优先级，可选）
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub prompt_hint: Option<String>,         // consolidator prompt 里该维度的填写指引
    #[serde(default)] pub candidate_type: bool, // 是否作为 memoryCandidate.type 合法值
}
```
- DomainProfile 加 `#[serde(default = "default_memory_dimensions")] pub memory_dimensions: Vec<MemoryDimension>`。
- `default_memory_dimensions()`：逐字复刻当前 8 个销售槽位 + 各自现有 cap：
  preferences(8)/doNotDo(10)/commitments(8)/objections(8)/openLoops(8)/openQuestions(8)/confirmedFacts(12)/conflicts(6)。
  （coreFacts/recentFacts/deprecatedFacts 仍走 typed 三数组的固定 cap 6/10/20，**不纳入** memory_dimensions——它们是结构骨架不是业务维度。coreProfile/relationshipState 是固定对象结构也不纳入。）

### 消费侧改造（DEFAULT 逐字等价）
1. **cap 表**（memory.rs:424-434）：保留 coreFacts/recentFacts 固定行，把 8 个业务槽位的 `limit_extra_array` 改为遍历 `profile.memory_dimensions` 按 `dim.cap` 截断。`compact_memory_card_with_previous` 需新增 `memory_dimensions: &[MemoryDimension]` 参数（或经已有 profile 加载传入）。无参 wrapper 委托 DEFAULT 供 PBT。
2. **consolidator prompt**（prompts.rs:1194-1232）：JSON 骨架 + limit 散文里的销售槽位改由 `memory_dimensions` 渲染。沿用 H5/H15 的"渲染函数单一真相源"模式——加 `render_memory_card_skeleton(dims)` / `render_memory_limits(dims)`，DEFAULT 输出与当前 prompt **字节等价**（快照测试锁死）。
3. **memoryCandidate type 枚举**（prompts.rs:1121-1130 + 校验逻辑）：合法 type 集从 `memory_dimensions` 里 `candidate_type=true` 的项 + 固定的 fact/conflict 派生。DEFAULT 复刻当前 7 值。
4. **memory_card_has_signal**（memory.rs:142）：判"有信号"的 extra 数组键列表改读 memory_dimensions。
5. **memory_card_from_contact 种子**（memory.rs:190）：按需对齐（DEFAULT 不变）。

### 向后兼容（红线，勘察已列）
- coreFacts/recentFacts 继续反序列化旧 Vec<String>（MemoryFactRepr untagged 不动）。
- extra flatten 不出现同名键冲突（情感槽走 extra 容器，不 typed 化，天然无冲突）。
- 新字段全部 `#[serde(default)]`，老 DomainProfile 文档无 memory_dimensions → default_memory_dimensions() 回落。
- 新增迁移 mNNN：给现有 domain_profiles 文档回填 default_memory_dimensions（幂等，照 m002/m005 套路）。**或** 纯靠 serde default 回落（无 active profile 时本就走 DEFAULT），评估后定——倾向 serde default 即可，无需迁移（DomainProfile 不像 OperatingMemory 有海量历史文档）。
- DEFAULT 逐字等价：cap 值、prompt 字节、candidate type 集全部锁快照。

## 实施步骤（每步独立 commit + 全基线 + 禁词闸）
- **H17-a**（纯 additive）：models.rs 加 MemoryDimension 结构 + DomainProfile.memory_dimensions 字段 + default_memory_dimensions() 逐字复刻 + `default_profile_memory_dimensions_match_hardcoded_verbatim` 护栏测试。不接消费侧。lib 绿。
- **H17-b**（cap 表接线）：compact_memory_card_with_previous 的 8 槽 cap 改读 memory_dimensions。无参 wrapper 委托 DEFAULT。memory_card_invariants PBT **append** 新断言（情感槽 cap 生效），不删旧维度断言。lib + PBT 绿。
- **H17-c**（consolidator prompt 单一真相源）：render_memory_card_skeleton/render_memory_limits 渲染函数，DEFAULT 字节等价快照测试。consolidator prompt 注入改走渲染。
- **H17-d**（candidate type 集）：memoryCandidate.type 合法集从 profile 派生，DEFAULT 7 值等价。
- **H17-e**（情感 profile 验证）：构造一个情感陪伴 profile（声明 emotionHistory/anniversaries/importantEvents 槽），单测验证：① consolidator prompt 出现情感槽；② cap 生效；③ candidate type 接受情感类型。证明通用化真成立（非销售 E2E 的记忆侧基础）。

## 牵动文件
- `src/models.rs`（MemoryDimension + DomainProfile 字段 + default + round-trip 测试）
- `src/agent/memory.rs`（cap 表 + has_signal + 种子 + compact 签名）
- `src/prompts.rs`（consolidator skeleton/limits 渲染 + candidate type）
- `tests/memory_card_invariants.rs`（merge gate PBT，**只 append**）
- 可能新增 `src/db/migrations/mNNN_*.rs`（评估后定，倾向不需要）
- `docs/superpowers/specs/2026-06-11-universal-domain-adaptation-design.md`（H17 行标 ✅）

## 不做（范围边界）
- 不动 intent_trajectory.objection_type typed 字段（models.rs:2818）——那是另一条轴（H17 描述里提了，但属画像信号轴，可单列；本次聚焦 memoryCard schema）。除非你要一并做。
- 不改 OperatingMemory 海量历史文档结构（extra 兜底已兼容）。
- 不动 typed 三数组（core/recent/deprecated_facts）的固定 cap 与结构。

## 验证基线
lib ≥ 350（当前 1071）/ memory_card_invariants + 3 个 PBT 累计 ≥ 33 / 0 failed；DEFAULT 字节等价快照；check-no-human-takeover 0 违规。
