# ⑨记忆固化"确定性兜底两件套" — 设计

**日期**：2026-06-28
**分支**：将新建（fix/memory-consolidation-guards）
**触发**：⑨改口冲突裁决真测（batch_a_domain9）FAILED——客户孩子年龄 8岁→改口10岁，固化后 8岁/10岁双值并存，旧值不退场。

## 1. 背景与根因（4 轮真模型探针 + 全代码审 + 三层信任设计核实）

### 1.1 现象
真测 server117（f1f7b98）：A 阶段固化"孩子8岁"进 coreFacts，B 阶段改口"10岁"再固化，期望旧"8岁"退出生效层（进 deprecatedFacts 或被 discarded 替换）。实际：coreFacts[0] 是 411 字 **blob**——把 6 句 summary（"孩子8岁…"/"更新为10岁…"/"确认为8岁…"）揉进**一条** fact 的 text，且该条 `dimension=None`。下游纯函数 `deprecate_same_dimension_conflicts`（memory.rs:480，只对"≥2 条同 dimension 的 Structured fact"裁决）空转 → 8岁/10岁双值并存。

### 1.2 四轮探针定位真因（blob 是低频偶发，非 prompt 结构缺陷）

| 探针 | 配置 | 结果 |
|---|---|---|
| v1 | 简化 prompt，单一职责 vs 多职责 | 都完美，无 blob |
| v2 | 简化 prompt + 注入脏 blob 卡 | LLM 主动清理 blob，无 |
| v3 | 生产真实 3843 字 prompt，**漏 ANTHROPIC_JSON_GUARD** | 86% tool_use 劫持（假象） |
| **v4** | **完全复刻生产**（guarded_system + max_tokens 8192） | **6/6 全部原子化 + 带 dimension + 改口裁决正确，0 blob** |

**结论**：生产的 `ANTHROPIC_JSON_GUARD`（llm.rs:616 禁工具声明）已完全压住 tool_use 劫持。v4 复刻生产串行 6 次全干净。blob 只在**极低频降级**时出现（真测那次的真实超长输入——完整对话窗口 + 候选记忆 JSON + 标签观察——偶发把 LLM 推到降级临界）。**不是 prompt 结构缺陷，不需拆分 consolidator、不需改 prompt。**

### 1.3 撤销的伪缺陷（标签"污染"）
曾怀疑 `memory_card_from_contact`（memory.rs:218-229）把 manual_tags/confirmed_tags 灌进 core_facts 是污染。核实三层信任设计（`docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md`:105）后**撤销**：manual_tags（人工权威层）+ confirmed_tags（AI 确信层=压缩宽窗口整体重判 replace 产物，**纠错后可信**）按设计就该作为初始事实种入。不可信的是**第二层 tag_observation**（memory_candidates，不进 prompt、不驱动行为），它本就不进事实层。标签碎片（"家长"/"编程课咨询"）是可信初始事实，**无害**（分类词不参与数值改口裁决）。

### 1.4 正常路径的三重冗余（为何 blob 才是命门）
探针证实 LLM 正常处理改口有**三条独立正确路径**，任一条即可让旧值退场：
1. 填 `discarded` → compact 全局黑名单剔除（memory.rs:380-384）
2. 填 `deprecatedFacts` → `apply_consolidator_deprecations`（memory.rs:556）
3. 新旧值各成独立条 + 相同 dimension 名 → `deprecate_same_dimension_conflicts`（memory.rs:480）

真测 FAILED 是偶发降级出 blob 时**三条全断**（揉一条：无 discarded、无 deprecatedFacts、无法同 dimension）。

## 2. 设计目标与红线

### 2.1 目标
确定性兜底接住**低频偶发降级**，让偶发 blob/缺 dimension 不再导致双值并存——而非和畸形产物搏斗（拆/丢猜测性强）。姿态：**拒绝降级产物、重试拿干净的**，与项目已有 tool_use 劫持处理（llm.rs:662 检测降级→抛错→重试）一致。

### 2.2 红线（全程守）
- **不碰硬闸阈值 / 不为过测试改业务 / 不改 consolidator prompt**（v4 证明 prompt 正常无缺陷）。
- **agent-first**：非原子检测用**通用结构度量**（换行数/句界数/长度），**绝不提取数值实体或关键词**（"找 N岁"是关键词模式，踩红线）。裁决继续用确定性纯函数 + LLM 主动标注，机器不猜语义。
- **不拆分 consolidator**（已论证：多一路 LLM 调用、prompt 无结构缺陷）。
- 既成事实纪律 / cap 纪律 / OCC 写入语义不变。

## 3. 实现方案（两件套）

### 3.1 件一：compact 救回逻辑加 dimension 感知（病一兜底）

**落点**：`compact_memory_card_with_dimensions`（memory.rs:386-399 的 previous 救回循环）。

**现状**：救回 previous 未 discarded 的 core_facts 时，**仅按 `as_text()` 字符串相等**去重（memory.rs:392-395）。改口场景"孩子8岁"≠"孩子10岁" text 不等 → 旧值被救回 → 与新值并存。

**改动**：救回前增加 dimension 感知判定——若 previous 的某条 fact 是 Structured 且带非空 dimension，而 incoming（compact.core_facts）已存在**同 dimension** 的 Structured fact，则**不救回**该旧值（新值已覆盖该维度）。纯函数、确定性、零关键词、零 LLM。dimension=None 的 fact 维持原 text 去重行为（字节等价，不回归）。

**接住的场景**：LLM 把新旧值打成两条带相同 dimension 的 fact 但**漏填 deprecatedFacts/discarded**（即只断了路径1/2、路径3 的 dimension 信息在）。

**契约**：
```rust
// 在 memory.rs:386 的 `for fact in &prev.core_facts` 循环内，
// 现有 discarded 跳过 + text 去重判定之外，增加：
// 若 fact 是 Structured 且 dimension=Some(非空)，
// 且 compact.core_facts 已有同 dimension 的 Structured fact → continue（不救回）。
```

### 3.2 件二：结构性非原子检测 + 降级重试（blob 防御，方案 X）

**落点**：`consolidate_contact_memory_inner`（memory.rs:1286 拿到 `generate_agent_json` 返回的 `value` 之后，`from_document` 之前）。

**关键前提（已核实）**：`user.memory_consolidator.task` **不在** LLM_EXACT_CACHE 白名单（mod.rs:480-486，仅 4 个 preview/playbook key 走缓存；测试 mod.rs:1115 印证非白名单返回 None）。故重新调 `generate_agent_json` 就是一次**全新 LLM 调用**，无需任何"绕缓存"特殊处理。

**纯函数检测 `fact_is_non_atomic(text: &str) -> bool`**（通用结构度量，零关键词）：
- text 含 ≥2 个换行（`\n`），**或**
- text 含 ≥2 个句界标点（`。`/`！`/`？`/`;`），**或**
- char 数 > 80。

> **80 的依据（非魔数）**：v4 探针实测正常原子 fact 最长 ~18 字（"客户更正孩子年龄：原说8岁，实际10岁"），blob 是 411 字——中间有 ~22 倍安全区。80 取在"正常 fact 上界（~20 字）"与"blob 下界（数百字）"之间的宽松位，**宁可漏判（偏大）也不误伤正常 fact**：漏判的稍长 blob 还有换行/句界两条判据兜底 + 件一 dimension 救回 + 重试，误伤正常 fact 则丢有效信息。三条判据是 OR 关系，互为冗余。

判定"本次输出含非原子 blob"= coreFacts/recentFacts 任一条 `fact_is_non_atomic`。

**重试逻辑**（复用降级重试范式）：
- 检测到非原子 blob → 记 warning `non_atomic_fact_detected` → **重新调一次** `generate_agent_json`（同 system/user，全新调用）。
- 重试输出再检测：仍非原子 → **丢弃那几条非原子 fact**（不落事实层）+ warning `non_atomic_fact_dropped_after_retry`，其余正常 fact 照常落库。重试至多 1 次（v4 证明 6/6 干净，1 次足够；不无限重试避免 token 失控）。
- 重试成功（原子化）→ 用重试结果走正常落库链。

**为何丢弃而非拆分**：拆分需猜"哪句是新值"（启发式易错、近语义判断违 agent-first）；丢弃是确定性的，且重试已大概率拿到完整原子输出，丢弃只是重试也失败的极端兜底（概率极低）。丢弃的 fact 信息会在下一轮固化时由 LLM 重新产出（候选记忆仍在）。

### 3.3 两件套的互补关系
- 件二（方案 X）治**源头**：保证拿到原子化 + 带 dimension 的干净输出（重试）。
- 件一（dimension 兜底）治**残余**：即使 LLM 漏填 deprecatedFacts/discarded，只要新旧值带相同 dimension，纯函数救回收口仍让旧值退场。
- 两者叠加：方案 X 拿到带 dimension 的原子输出 → 件一 + 现有 `deprecate_same_dimension_conflicts` 裁决旧值退场。

## 4. 数据流

```
consolidate_contact_memory_inner:
  value = generate_agent_json(consolidator)            [LLM 调用]
  → [件二] fact_is_non_atomic 扫 value.coreFacts/recentFacts
       ├─ 含非原子 → 重试 generate_agent_json 一次(全新调用,非白名单不缓存)
       │    ├─ 重试原子 → 用重试 value
       │    └─ 重试仍非原子 → 丢弃非原子条 + warning
       └─ 原子 → 直接用
  → from_document → auto_upgrade_plain_facts
  → [件一] compact_memory_card_with_dimensions(救回逻辑加 dimension 感知)
  → apply_consolidator_deprecations(LLM 主动填的 deprecatedFacts)
  → deprecate_same_dimension_conflicts(纯函数同 dimension 裁决)
  → OCC 写入
```

## 5. 测试

遵循「新增测试只增量叠加」「动态测试反过拟合四铁律」。

### 5.1 纯函数单测（本地 cargo test --lib）
**件一 dimension 感知救回**：
- previous 有 `{text:"孩子8岁", dimension:"孩子年龄"}`，incoming 有 `{text:"孩子10岁", dimension:"孩子年龄"}` → 救回后 core_facts **不含**"孩子8岁"（同 dimension 新值在场不救回）。
- previous 有 `{text:"孩子8岁", dimension:"孩子年龄"}`，incoming 无同 dimension fact → 旧值**正常救回**（不误删）。
- previous/incoming 均 dimension=None → 维持 text 去重行为（字节等价回归保护）。
- 不同 dimension（previous "预算3万"/incoming "孩子10岁"）→ 都保留（不误删跨维度）。

**件二 fact_is_non_atomic**：
- "孩子10岁" → false（正常原子）。
- "预算5000左右" → false。
- "孩子8岁\n更新为10岁\n确认8岁" → true（多换行）。
- "孩子8岁。预算5000。男孩。" → true（多句界）。
- 90 字长句 → true（超长）。
- 边界："孩子10岁，零基础想报编程课"（含 1 逗号、~13 字）→ false（不误伤正常稍长 fact）。

### 5.2 集成测试（CI，Docker）
- 构造 consolidator 返回含 blob 的 mock value（注入 fixture）→ 验证触发重试路径 + warning 落 agent_run_logs.memory_consolidator_warnings。
- 重试仍 blob → 验证非原子条被丢弃、正常条保留。

### 5.3 真模型回归（server117，端点恢复且串行后）
- 复跑 batch_a_domain9：A 固化8岁 → B 改口10岁 → 断言 8岁退出生效层（进 deprecatedFacts 或被替换）、无双值。
- 注意端点 2 线程限制（[[reference_llm_endpoint_2thread_limit]]）：真测须串行，不并行其它脚本。

### 5.4 基线门（不回归）
- cargo test --lib ≥ 350/0；4 PBT 累计 ≥ 33/0。
- check-baseline + check-no-human-takeover + check-evolution-isolation 三 lint 绿。

## 6. 变更文件清单

| 文件 | 改动 |
|---|---|
| `src/agent/memory.rs` | 件一：`compact_memory_card_with_dimensions` 救回循环加 dimension 感知（3.1）；件二：新增纯函数 `fact_is_non_atomic` + `consolidate_contact_memory_inner` 检测重试逻辑（3.2）；5.1 单测 |
| `tests/` | 5.2 集成测试（mock blob value → 重试/丢弃路径） |

## 7. 非目标（YAGNI）
- 不拆分 consolidator（v4 证明 prompt 无结构缺陷，拆分多一路争用）。
- 不改 consolidator prompt / 硬闸阈值。
- 不动标签 seed 逻辑（病三已撤销，标签按三层信任设计正常种入）。
- 不引入向量检索/LLM 判 ADD-UPDATE-DELETE（Mem0 范式，超 ⑨ 当前需要，YAGNI）。
- 不做无限重试（至多 1 次，v4 证明足够）。
