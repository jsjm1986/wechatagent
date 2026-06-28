# ⑨记忆冲突治上游 + ④reviewer 让位下沉 — 设计

**日期**：2026-06-28
**分支**：fix/structured-field-gap（v2 基线 commit eb206d3）
**前置**：本设计是「四域结构化字段缺失修复 v2」(`2026-06-27-structured-field-gap-design.md`) 的续作。v2 在**决策点**修复了 ③④⑥ 的结构化字段产出（assetsToSend/namecardToSend/escalationRequest），Task9 server 117 真模型测验证 ③④⑥ 决策层均真生效。本设计修复剩下的**第二道关卡未同步**问题：④ reviewer 不知 assist 豁免、⑨ consolidator 不产 dimension。

## 1. 背景与根因（已全代码 + DB + 确定性单测核实）

### 1.1 审计修正了 Task9 三处幻觉（诚实记录）

Task9 真测结论经本轮逐条代码核对，修正以下三处与代码不符/夸大处：

| 域 | Task9 原结论 | 代码真相 |
|---|---|---|
| ③ | held_by_ai_policy 由 "grounding/R5.4 verified 硬闸" 写 | **错**。`gates.rs:679` R5.4 闸写的是 `blocked_unverified_product_claim`；`held_by_ai_policy` 来自 `gates.rs:844` 末尾 else（软闸 `knowledge_grounding_score < 阈值` → HardGateFailure）。两机制被混叙 |
| ④ | "让位下沉 reviewer 即放行" | **夸大**。`gates.rs:115` `hallucination_score >= fact_risk_block_at(6)` 是独立于红线措辞的数值硬闸。hold 有两条路径 |
| ⑥ | "resolve 没真更新 = 下游基建 pre-existing bug" | **定性偏差**。`routes/principal_escalations.rs:75-113` handler 逻辑正确（真更新 status + enqueue relay）。`:83` IDOR 静默幂等 / `:94` deferred 短路——"仍 pending" 几乎确定是测试调用问题（short_code/workspace 不匹配 或 verdict=deferred），非代码 bug |

③⑥ 经核实**无需代码修复**：③ 是测试 fixture 缺配套 verified 知识背书（绝不为过测试削 grounding 闸——红线）；⑥ 是测试调用问题，resolve/relay 基建代码正确。本设计只修 ④⑨。

### 1.2 ⑨ 根因链（每环有代码证据）

| 环节 | 代码位置 | 现状 | 问题 |
|---|---|---|---|
| 注入给 LLM 的当前卡 | `memory.rs:1253` | `effective_memory_card(&memory).to_document()` 直接序列化 DB 里的 Plain 字符串 | LLM 收到**无 id 的裸字符串数组**，`prompts.rs:1463` 要求"用 id 弃用"前提不成立 |
| LLM 输出 schema | `prompts.rs:1432` | `"coreFacts": []` 空数组，无 per-item 对象示例 | LLM 倾向吐字符串/粗粒度 summary |
| dimension 指令 | `prompts.rs:1474` | 埋在"限制："第 6 条、"**可选**带 dimension" | A/B 已证无效（同 ⑥ 原"可选字段"教训） |
| 升级 | `models.rs:4001` | `auto_upgrade_plain_facts` 升 Structured 但 dimension=None（`from_plain_text` 恒置 None，`models.rs:3882`） | 裁决条件不满足 |
| 裁决 | `memory.rs:489` | 只处理"Structured 且 dimension 非空" | 全 bypass，**裁决从未 engage** |

**DB 实证**（biztest_c9，version 3，source=memory_consolidator_agent）：coreFacts 是 4 条纯字符串，其中 coreFacts[0] 是一条累积巨型 summary，8岁/10岁/预算碎片全揉在**同一条 fact 内部**。这说明 fact 不原子时，任何 id/dimension 级裁决都无从切入。

**确定性验证**（审计期临时单测，已删）：`Structured(带 dimension)` → `to_document` → `from_document` round-trip **完整存活**，dimension 不丢、不塌回 Plain。故根因是"结构化 fact 从未真正产出/落库"，**不是 round-trip 损坏**。治上游成立。

### 1.3 ④ 根因（两条 hold 路径同源）

assist 开 + 引荐场景，Decision Agent 真 emit namecardToSend（v2 决策层让位生效，`decision.rs:426-430` + 注入点 `:932`），但 final=held_by_ai_policy：

- **路径①红线措辞**：reviewer system prompt（`prompts.rs:1576`）"承诺安排真人 / 引入第三方角色就是失约"→ needs_revision。这条不读 assist 状态。
- **路径② factRisk 硬闸**：`gates.rs:115` `hallucination_score >= 6` 数值硬闸。引荐承诺若被 reviewer 误判成"无 verified 背书的产品承诺"→ 打高 hallucination → 拦。

两条路径**同源**：都因 reviewer 对"assist 模式引荐是合法受控动作"零认知（grep `review/` 目录 assist/引荐/namecard/referral 全 0 命中）。

**hold 连带效应**（`gates.rs:844` + `gateway.rs:2245-2247`）：held → `outbox_eligible=false` → namecard **一并不入 outbox**，所以测试 `has_card` 断言（检查 referral_card_id 是否入 outbox）正确 FAIL，非假绿。

## 2. 设计目标与红线

1. **⑨ 治上游**（用户选定强度 ⑨-C 最重）：保 id 注入 + 强制结构化产出 + dimension 改口必填 + 跨轮命名稳定化。
2. **④ reviewer 让位下沉**：注入 assist 感知让位措辞，同时消解两条 hold 路径；**不碰 gates.rs 硬闸阈值**。
3. **红线（全程守）**：
   - 不为过测试改业务逻辑/prompt/guards/阈值（过拟合是红线中的红线）。
   - agent-first：dimension 是 LLM 语义归类，非关键词匹配；裁决纯函数零关键词零 LLM。
   - DEFAULT 字节等价：dimension=None 退回按 text 去重旧行为；assist 关账号让位段空串、reviewer 红线一字不动。
   - check-no-human-takeover lint：④ 让位措辞用「专属顾问/增配/我仍在场辅助」，绝不出现"转人工/接管/第三方真人接手"。

## 3. ⑨ 实现方案（强度 C）

### 3.1 保 id 注入（让 LLM 有 id 可弃用）

**问题**：`memory.rs:1253` 注入的 `effective_memory_card(&memory)` 读的是上一版 DB（Plain 字符串无 id）。`auto_upgrade_plain_facts` 当前只在**读 LLM 输出之后**（`memory.rs:1300`）调用，注入前不升级。

**改动**：在 `memory.rs:1253` 注入前，对 `effective_memory_card(&memory)` 的结果做一次 `auto_upgrade_plain_facts()`，使注入给 LLM 的 coreFacts/recentFacts 都带稳定 id（fresh UUID）。

- 这些 id 必须与 `previous_card`（`memory.rs:1312` 的 `effective_memory_card(&memory)`）**同源**——即同一份升级后的 card 既用于注入、又用于 prev-merge，否则 LLM 引用的 id 在合并时匹配不上。
- 实现要点：把"升级后的 effective card"算一次，注入与 previous 复用同一份。

**幂等性注意**：`from_plain_text` 每次生成 fresh UUID。若注入升级与 previous 合并用不同的 card 实例，会产生不同 id。设计要求二者共用同一实例（见上）。一旦某轮成功写回 Structured（带稳定 id），后续轮次读回就是 Structured，id 稳定，不再重新生成。

### 3.2 强制结构化产出（schema + dimension 改口必填 + 原子化）

`prompts.rs` consolidator task 模板（`:1417-1456`）三处改动：

1. **coreFacts/recentFacts schema 给 per-item 对象示例**（`:1432-1433`）：
   从 `"coreFacts": []` 改为带一条对象示例，如：
   ```json
   "coreFacts": [
     { "id": "沿用当前卡中该 fact 的 id；新 fact 留空由系统生成", "text": "一条只讲一个事实的原子陈述", "dimension": "该事实的语义维度名", "importance": 8 }
   ]
   ```
2. **fact 原子化要求**（限制段）：明确"一条 fact 只讲一个事实（一个属性/一个数值/一个角色），不要把多个事实揉进一条 summary 式长句"。直接针对 c9 巨型 summary 根因。
3. **dimension 改口必填**（镜像 ⑥ 决策墙手法，把 `:1474` 的"可选"升级）：
   "当本轮出现对某属性的**改口/更正**（典型：年龄、预算、决策角色变化）时，新 fact **必须**带 dimension 字段标注该属性维度，且与被更正的旧 fact 用**同一 dimension 名**——系统据此自动让该维度旧值退出生效层。"

### 3.3 跨轮命名稳定化（C 增量）

注入 prompt 时，把当前卡里**已存在的 dimension 名清单**一并告知 LLM，引导复用：
- 在 `memory.rs` 组装 user prompt（`:1227-1248`）处，从升级后的 effective card 提取所有非空 dimension 名，去重后作为一行注入，如："已有维度名（同一属性请沿用，不要新造同义名）：[孩子年龄, 预算, 决策角色]"。
- 空清单（新联系人/无 dimension）→ 不注入该行（DEFAULT 字节等价）。
- **冷启动语义**：3.1 的升级是从 Plain 字符串升 Structured，dimension 恒 None（`from_plain_text`）。故改造**首轮**该清单为空、不注入；只有当某轮 LLM 真产出带 dimension 的 fact 并写回后，后续轮次读回的 Structured fact 才带非空 dimension，此清单才生效。即跨轮稳定化是"第二轮起"的引导，首轮靠 3.2 的 schema/必填措辞冷启。
- 这是**引导**非强制——A/B 教训：靠 prompt 一致性不绝对可靠，故 3.4 兜底仍在。

### 3.4 机制侧兜底（已存在，确认 engage 条件满足）

`deprecate_same_dimension_conflicts`（`memory.rs:480`，Task6 已实现 + 已接入 `:1328`）：同 dimension 的多条 Structured fact，保留 updated_at 最新一条，其余移 deprecated_facts（带 supersededBy + cap20）。零关键词零 LLM。

- 3.2 让 LLM 真产出带 dimension 的 Structured fact 后，此兜底**才真正 engage**（此前因 dimension 恒 None 全 bypass）。
- dimension=None 的 fact 仍完全不参与（退回 text 去重旧行为，字节等价）。

### 3.5 主路径 vs 兜底的关系

- **主路径**（LLM 用 id 在 deprecatedFacts/discarded 显式弃用）：3.1 保 id 注入激活。`apply_consolidator_deprecations`（`memory.rs:556`）按 id 在 previous 查原 fact 弃用。
- **兜底**（LLM 标了 dimension 但漏用 id 弃用）：3.2 + 3.4 激活。
- **残留风险（诚实标注）**：dimension 跨轮命名漂移会让兜底匹配不上（3.3 缓解但不绝对）。但主路径不受影响，是"兜底的兜底失效"，非回到原点。

## 4. ④ 实现方案（reviewer 让位下沉）

### 4.1 注入点与条件

- **注入点**：`review/mod.rs:287` 加载 reviewer system prompt 后、`:302` `apply_review_system_prompt_overrides` 前后追加 assist 让位段。
  - 注意：`apply_review_system_prompt_overrides`（`domain_profile.rs:640`）只吃 `profile`，不含 contact/assist 状态。故让位段在 `review/mod.rs` 调用处注入（能拿到 contact + domain_config），**不塞进**那个纯 profile helper。
- **条件**：复用 `referral::assist_mode_active(domain_config.assist_mode_enabled, contact override)`（`referral.rs:10`，纯函数）。`review_decision`（`review/mod.rs:253`）已有 `contact` + `domain_config` 入参。
- **assist 关账号**（默认全自治）：让位段空串、reviewer system prompt 字节等价、`prompts.rs:1576` 红线一字不动。

### 4.2 让位措辞（同时消解两条 hold 路径）

注入段需说明两点（镜像 decision.rs `assist_redline_yield` `:427` 措辞与 lint 红线）：
1. **解路径①**：辅助模式下"为客户增配一位专属顾问（namecardToSend）"是本账号受控业务动作，不属于「除『我』外不得出现人类角色」红线所禁——该红线在引荐这一动作上让位，不应因守红线判 needs_revision。
2. **解路径②**：引荐专属顾问**不是产品能力声明**（不涉及产品功能/价格/效果），不应计入 hallucination/产品准确度评分。

### 4.3 不碰硬闸（取舍）

**只注入措辞、不碰 `gates.rs:115` 硬闸阈值**，理由：
1. 硬闸是产品声明 grounding 核心红线（CLAUDE.md），为引荐场景在代码层开后门风险大。
2. 引荐本不该被判成产品承诺，正解是让 reviewer 正确分类（措辞引导），非改阈值。
3. 改阈值 = 为过测试削红线，踩过拟合红线。

**残留风险（诚实标注）**：注入措辞后 reviewer 仍可能偶发把引荐打成高 hallucination（LLM 不确定性）。与 ⑨ 同理——prompt 引导是正解，真模型测验证泛化，不靠改硬闸兜底。

## 5. 测试

遵循「新增测试只增量叠加」「动态测试反过拟合四铁律」：

### 5.1 纯函数单测（本地 cargo test --lib）
- ⑨ 保 id 注入：构造 Plain 字符串 card → 升级 → 断言注入用 card 与 previous 同 id（同源）。
- ⑨ dimension 兜底 engage：已有 PBT `pbt_same_dimension_at_most_one_live`（`memory.rs:2243`）覆盖"同维生效层 ≤1"，确认 3.2 产出 dimension 后此 PBT 路径真被走到（可加一条单测：Structured 带同 dimension 两条 → 裁决后生效层 1 条 + deprecated 1 条）。
- ④ 让位注入纯函数：assist_on → 让位段非空 + 含合法措辞；assist 关 → 空串（字节等价）。
- ④ lint：让位措辞过 check-no-human-takeover（无禁词）。

### 5.2 prompt 模板断言（本地）
- consolidator task 模板含 per-item 对象示例 + dimension 改口必填措辞 + 原子化要求。
- PROMPT_PACK_VERSION bump（prompts.rs 改动必须 bump，否则 DB 不重 seed）。

### 5.3 真模型回归（server 117，多 seed 变体防过拟合）
- ⑨：复跑 batch_a_domain9.py（8岁→10岁改口），断言 8岁退出生效层 + 进 deprecated/conflict 事件。换 seed 变体（如 预算 3万→5万、地址变更）验证泛化，非点对点调单条。
- ④：复跑 batch_a_domain4.py（assist 开 + 签约意向），断言 namecard 入 outbox。
- 端点污染（llm_tool_use_instead_of_json glitch）→ reset + 单发隔离重测，看 decision 实体非只看 status。

### 5.4 基线门（不回归）
- `cargo test --lib` ≥ 350/0；4 PBT 累计 ≥ 33/0。
- check-baseline + check-no-human-takeover + check-evolution-isolation 三 lint 绿。

## 6. 变更文件清单

| 文件 | 改动 |
|---|---|
| `src/agent/memory.rs` | 3.1 注入前升级 + 注入与 previous 同源；3.3 已有 dimension 名注入；5.1 单测 |
| `src/prompts.rs` | 3.2 schema 对象示例 + dimension 改口必填 + 原子化；PROMPT_PACK_VERSION bump；5.2 断言 |
| `src/agent/review/mod.rs` | 4.1 让位段注入（assist_on 条件）；5.1 单测 |
| `src/agent/decision.rs` | （可能）抽出让位措辞为共享常量，供 reply + review 复用，避免两处漂移 |

## 7. 非目标（YAGNI）

- 不重构 MemoryFactRepr untagged enum（round-trip 已验证可用）。
- 不改 gates.rs 硬闸阈值（4.3）。
- 不修 ③⑥（审计证实无需代码修复）。
- 不做 dimension 受控枚举取值空间（保持 LLM 语义自命名 + 跨轮复用引导，守 agent-first；强枚举是另一个量级的工程，当前 YAGNI）。
