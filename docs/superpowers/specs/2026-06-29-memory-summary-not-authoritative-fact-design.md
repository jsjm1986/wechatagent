# ⑨记忆固化"memory_summary 不当权威事实"——设计

**日期**：2026-06-29
**分支**：将新建（fix/memory-summary-not-fact）
**触发**：server117 全量真模型测试（部署最新 main 265aa49，含上一波件一/件二修复）域⑨重测仍 FAILED——客户改口 8岁→10岁后，旧值"8岁"不退场，8岁/10岁双值并存。

## 1. 背景与真因（systematic-debugging 逐层查 server 真实数据，铁证）

### 1.1 上一波修复为何没生效（根因误判）
上一波"确定性兜底两件套"（件一 compact 救回加 dimension 感知 + 件二结构性 blob 检测重试丢弃，已合并 main #60）作用在 `consolidate_contact_memory_inner` 的 **`value.coreFacts` 层**（consolidator 单次 LLM 输出）。真测后 `memory_consolidator_warnings` 全空——**件一件二全程没触发**。

### 1.2 真因（三个独立事实，每条经 server mongo 实查）
1. `operating_memories.memory_card.core_facts` = **0 条**——真正的结构化事实层是空的。
2. `contact.memory_summary` = len 421 / 7 换行 = **那条 blob**：8 段历史 summary 累积（"孩子8岁…"/"更新为10岁…"/"确认为8岁…"/"最终确认10岁…"）。
3. `memory.rs:215` `memory_card_from_contact` 把整段 `contact.memory_summary` 当**一条 core_fact** push 进种子卡的 `core_facts[0]` → 展示/读取路径（`effective_memory_card_for_contact` 在 memory_card 空时回落种子卡）把这条 blob 当权威事实显示 → 测试 `_fact_texts` 看到 8岁/10岁并存。

### 1.3 blob 怎么累积出来的（杠杆2，本轮不改）
`gateway.rs:4044` 每轮把 consolidator 的 summary 经 `merge_memory_summary_dedup_capped`（gateway.rs:3581，行级去重 + cap 12 行/1200 字节）**append** 进 `contact.memory_summary`。各版本 summary 措辞不同（"8岁零基础"vs"年龄确认为8岁"），行级去重留不住 → 逐轮累积成多行 blob。

**这是 memory_summary 的设计行为**：gateway.rs:3565 注释明说"短期 memory_summary 是滚动上下文（旧行已被 consolidation 吸收进 memoryCard，保新更有信息量）"。即 memory_summary **本就是短期滚动上下文，不是权威事实层**。错的不是它累积，而是 `memory.rs:215` 把这个短期滚动上下文**当成了权威 core_fact**。

## 2. 设计目标与红线

### 2.1 目标
让 `memory_summary`（短期滚动上下文）不再被 `memory_card_from_contact` 当**权威 core_fact** 注入种子卡。core_facts 只保留真正的结构化权威事实（运营手写的 human_profile_note + manual_tags/confirmed_tags）。memory_summary 内容归位到语义贴切的 `extra.recentEpisodeSummary`（近期摘要层），不丢信息、只改层级。

### 2.2 红线（全程守）
- **agent-first**：纯字段归位（删一行 push + 改一处 insert），**零关键词、零数值实体提取、零语义裁决**。
- **不改 memory_summary 累积逻辑**（gateway.rs:4044/3581 杠杆2）——那是短期滚动上下文该有的行为，不动。
- **不改 consolidator prompt**、不改结构化写入路径、不碰硬闸阈值。
- **不为过测试改业务**：本修复治的是"短期上下文被误当权威事实展示"这个真实分层错误，普适（任何 contact 的 memory_summary 都不该当权威事实），非对单条测试打补丁。
- 既成事实 / cap / OCC 写入语义不变。

## 3. 实现方案（杠杆1：展示注入分层修正）

**落点**：`memory_card_from_contact`（memory.rs:191-292）。

### 3.1 改动
1. **删除 line 215**：`push_unique_text(&mut core_facts, contact.memory_summary.as_deref());`
   —— memory_summary 不再进权威 core_facts 事实层。
2. **改写 line 279**：`extra.insert("recentEpisodeSummary", "");`
   → `extra.insert("recentEpisodeSummary", contact.memory_summary.clone().unwrap_or_default());`
   —— memory_summary 内容归位到语义贴切的"近期摘要"层（当前恒为空串）。

### 3.2 不动的（边界）
- **line 216** `push_unique_text(&mut core_facts, contact.human_profile_note.as_deref());` 保留——human_profile_note 是运营手写的权威画像注记，本就该是权威事实。
- **line 200** identity 的 `.or_else(|| contact.memory_summary.clone())` 回落保留——identity 是 `coreProfile` 的**单值画像字段**（不是事实清单），memory_summary 作为冷启动 identity 兜底不产生"多值并存"问题（单值字段后者覆盖前者）。
- **manual_tags / confirmed_tags**（line 218-229）进 core_facts 保留——按三层信任设计，这两层是权威/AI 确信层，本就该作为初始事实（见 `docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md`）。

### 3.3 改动后的数据流
```
memory_card_from_contact(contact, memory, initial_state):
  core_facts = [human_profile_note?, manual_tags…, confirmed_tags…]   ← 只留权威结构化事实
  extra.recentEpisodeSummary = contact.memory_summary                 ← 短期滚动摘要归位
  extra.coreProfile.identity = human_profile_note ?? memory_summary ?? profile.summary  ← 不变
```

## 4. 本轮非目标（YAGNI / 待后续专题）
- **不改 memory_summary 累积逻辑**（杠杆2，gateway.rs:4044）——短期滚动上下文该滚动，cap 已在。
- **不修"consolidator 把事实写进 summary 字段而非结构化 coreFacts"**——这是更深一层（触 consolidator prompt + 与上轮 A/B 结论"prompt 约束位置决定生不生效"相关），另开专题。
- **件一/件二不回滚**——它们对 consolidator 单次 coreFacts 层偶发 blob 仍有效（虽非域⑨主因），是正交的防御层，保留。
- 全量测试其余发现（⑩管理 agent 疑回归 / ③④⑥统一结构化字段缺失模式）——本轮不碰，各自专题。

## 5. 测试

遵循「新增测试只增量叠加」。

### 5.1 纯函数单测（本地 cargo test --lib）
`memory_card_from_contact` 是 `pub(crate)` 纯函数，可直接单测：
- **改口累积场景**：构造 contact.memory_summary = "孩子8岁\n更新为10岁\n确认8岁"（多行 blob），调 `memory_card_from_contact` → 断言 `core_facts` **不含** memory_summary 那条 blob（不再当权威事实）；断言 `extra.recentEpisodeSummary` == 该 blob 文本（归位成功）。
- **权威事实保留**：构造 contact.human_profile_note = "VIP 客户" + manual_tags=["家长"] → 断言 core_facts 含 "VIP 客户" 和 "家长"（权威层不受影响）。
- **identity 回落不变**：human_profile_note 为空、memory_summary="张三老板" → 断言 `extra.coreProfile.identity` == "张三老板"（单值画像回落保留）。
- **空 memory_summary 不炸**：contact.memory_summary=None → recentEpisodeSummary == ""（字节等价原行为）。

### 5.2 基线门（不回归）
- cargo test --lib ≥ 350/0；4 PBT 累计 ≥ 33/0。
- check-baseline + check-no-human-takeover + check-evolution-isolation 三 lint 绿。
- RUSTFLAGS=-D warnings cargo check --tests 通过。

### 5.3 真模型回归（server117，部署后串行）
- 复跑 batch_a_domain9：A 固化 8岁 → B 改口 10岁 → 断言种子卡 core_facts 不再含累积 summary blob（8岁/10岁不再以"权威事实"形式并存于 core_facts）。
- 注意端点 2 线程限制：真测串行，不并行其它脚本。

## 6. 变更文件清单

| 文件 | 改动 |
|---|---|
| `src/agent/memory.rs` | `memory_card_from_contact`：删 line 215 memory_summary→core_facts push；改 line 279 recentEpisodeSummary 注入 memory_summary；5.1 单测增量 append |
