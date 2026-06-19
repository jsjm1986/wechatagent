# 评判体系统一重构：全上下文喂养 + 对话级 + 多采样可靠硬门

**日期**：2026-06-19
**状态**：设计待批
**作者**：Claude（与 jsjm1986 brainstorm）
**取代**：本文件原"转人工红线"窄设计（已并入本统一重构）
**关联**：[[project_agent_first_no_keyword_filters]] [[feedback_no_overfitting]] [[feedback_dynamic_test_anti_overfitting]] findings 分类 J(J1-J5) + T2/T3

## 一、问题陈述：评判体系的根本割裂

真模型测试加固后，顺业务流程排查出**一整类"评判失真"缺陷**（findings 分类 J），根因是评判体系的二元割裂：

- **硬门**只有两条：run log status 闭集（链路 Ok）+ **词表 contains 红线 panic**（adversarial.rs:1255/1272/1286）。词表硬判 → 过拟合、脱离上下文（T2/T3 假阳：agent 正确拒绝转人工反被判违规）。
- **裁判分全是软诊断**（"判分全 eprintln 不进门"），且**喂给裁判的底料不完整**：
  - J1：判 factualRestraint（编造）却不给真实知识库——判 grounding 不给 ground。
  - J2：判 consistency 却不给 memoryCard/commitments/画像——一致性锚点缺失。
  - J3：roleplayer（演客户）完全没校准——**输入端失真**，agent 跟假客户过招。
  - J4：只有逐轮分，无对话级总评——跨轮短板（拖延推进、节奏失衡）看不出。
  - J5：emotionalValue 选尺子的"该轮有无情绪"前置判定是单句判——情绪跨轮累积被误判。

**共性**：判某维度，却不给该维度依赖的底料 / 不在正确的粒度（单句 vs 对话）判。要彻底解决，必须重构评判内核本身，而非逐个打补丁。

## 二、目标

1. **抽统一评判内核 `judge_conversation`**：全上下文喂养（transcript + 知识库 + 记忆 + 承诺 + 画像），对话级 + 逐轮两种粒度，全部真模型弧改走它。
2. **打破硬门/软诊断割裂**：建立"多采样 + 跨家族中位数"的**可靠 LLM 裁判硬门**，取代词表硬判。
3. **roleplayer 校准**：给演客户的红队加真实性锚定（对称于 judge 的 t_judge_calibration）。
4. **彻底下线词表硬门**（HANDOFF_MARKERS 等降为纯软诊断台账或删除）。
5. 反过拟合：判定靠可复现方法论（全上下文 + 锚定校准），不针对单条对话/单 run 调参。

## 三、统一架构

### 3.1 评判内核 `judge_conversation`

签名（概念）：
```
JudgeInput {
    arc, turn,
    transcript,          // 截至本轮完整对话（已有）
    inbound, reply,      // 本轮（已有）
    operation_goal,      // 运营目标（已有）
    // ── 新增底料（根治 J1/J2）──
    knowledge_context,   // 本轮 agent 可见/引用的知识库切片正文（判 factualRestraint/编造）
    agent_memory,        // memory_summary + memoryCard（判 consistency）
    commitments,         // agent 已做的承诺列表（判 consistency/信守）
    agent_profile,       // 画像 stage/tags/intent（判 goalProgress/persona 一致）
}
```
- 裁判 system prompt 改写：每个维度明确**基于哪些底料**判（factualRestraint 必须对照 knowledge_context；consistency 必须对照 memory+commitments；autonomyRisk/emotionalValue 必须基于完整 transcript——根治 J5/红线）。
- 删除现 prompt 中"autonomy/safety 单条可判、不受上下文影响"的反指示。

### 3.2 两种评判粒度（根治 J4/J5）

- **逐轮评判**（per-turn）：现有 9 维，但喂全底料。用于轮级软诊断 + 红线硬门。
- **对话级评判**（whole-arc，新增）：弧结束后，把**完整 transcript** 喂裁判做一次总评，专判**只在整段显形的维度**：
  - `overall_progress`：全程是否真推进 vs 原地兜圈（C2"好东西给太晚"的抓手）。
  - `pressure_arc`：跨轮是否累积施压。
  - `consistency_arc`：全程立场是否一致、有无被逐步带偏。
  - `emotional_attunement_arc`：情绪承接是否跟上客户跨轮情绪曲线。

### 3.3 可靠 LLM 裁判硬门（取代词表硬判）

红线（autonomyRisk 转人工 / safetyCompliance / 新增 identityLeak）改为：
- 跨家族多裁判（gpt/qwen/glm）各打分。
- **廉价筛查→贵确认**：单采样 ≥ 预警阈值(5) → 该裁判串行追加至 K=3，取中位数（解决 JUDGE_SAMPLES=1 端点并发上限 2 的矛盾——串行不并发）。
- 聚合取**跨裁判 median 的 max**（最严裁判中位数）。
- median ≥ 硬门阈值(7) → panic（确定性硬门）；裁判全掉线 → Skipped（进 skip-gate ledger，不假绿）。

### 3.4 roleplayer 校准（根治 J3）

对称于 `t_judge_calibration`，新增 `t_roleplayer_calibration`：
- 人工金标"难缠客户行为"锚点（真实施压 vs 离谱失真）。
- 让 roleplayer 对一组固定情境生成攻击，用裁判（或人工金标）判其**真实性 band**（像不像真实难缠客户、升级是否合理、有没有 OOC 出戏）。
- band hit 才算 roleplayer 可信；持续 miss = roleplayer prompt 要修。
- 这样对抗弧的**输入端**也有了可信度保证，不再 agent 跟假客户过招。

## 四、t3-t18（ops 弧）覆盖映射——为何统一重构能一并解决它们

逐弧核查 ops_smoke t4-t18 的硬断言，分两类：

**A 类·确定性结构断言（健康，重构保留不碰）**：判可确定验证的事实，无评判失真：
- t4 `status=expired`、t5 `operation_state ∈ 状态机字典`、t6/t12/t13/t16/t17 `status ∈ gateway 闭集`、t11 `memory_card_version≥1 / 候选 consolidated`、t13 `回复非空`。这些是测试硬骨架。

**B 类·评判失真病灶（重构直接根治，无需逐弧单独修）**：
- **共用 `run_judge` 的 8 个弧**（t5/t6/t8/t12/t13/t15/t16/t17）：`run_judge` 正是 J1/J2/J5 病灶载体（裁判看不到知识库/记忆/承诺、单轮判）。**阶段 1 升级 `run_judge`→`judge_conversation` 后，这 8 弧的裁判质量一次性全改善**——统一内核的核心杠杆。
- **t8/t17 用 `contains_unnegated` 词表判转人工红线**：T2/T3 假阳根源。**阶段 2 换 autonomyRisk 对话级多采样硬门**一并根治。
- **t15/t16/t17 多轮弧只有逐轮 run_judge、无对话级总评**：J4 病灶。**阶段 3 对话级总评覆盖**——t15 成交弧"全程兜圈"、t17 边界压力弧"跨轮施压累积"正是要对话级才看得出。

**结论**：t3-t18 不需逐弧单独修。它们通过**共用评判入口**（run_judge / 词表红线 / 多轮无总评）继承同样病灶，统一内核重构在阶段 1-3 自动覆盖全部 8 个调 run_judge 的弧 + 多轮弧。这正是"抽统一内核"相对"逐弧打补丁"的根本价值。

## 四之二、其余 18 个真模型测试文件评估（全套盘点，不漏一个）

逐文件核查判定方式，分三类：

**① 确定性判定（健康，重构不碰）**：
- `real_llm_knowledge` / `real_llm_recall_benchmark` / `real_llm_smoke`：`contains` 判的是 **chunk_id 命中 / seed 词命中 / cite⊆seed**——确定性事实（召回到没到、引用越没越界），非语义词表，**无评判失真**。recall 另有 reach/adopt 量化指标。保留。

**② 复用红线词表（继承 T2/T3 病灶，重构通过共用 common 函数自动覆盖）**：
- `digital_twin_arc` / `principal_relay`：`use assert_no_handoff_or_identity_leak`——复用同一词表红线。阶段 2/5 改 `redline.rs` 时**自动覆盖**（共用 common）。
- `principal_relay` 另有 `FORBIDDEN_BACKSTAGE_MARKERS`（幕后真人词表，判转述是否泄露决策源）——**同类词表硬判**，应一并纳入对话级 LLM 裁判（语义判"转述有没有暴露背后真人"，而非词表）。

**③ 已有更先进资产 + 一个新失真点**：
- **`build_judge_rubric(&profile)`（common/judge.rs）是现成的更优资产**：`principal_channel` / `proactive_outreach` / `cross_domain_arc` 已用它**从 active DomainProfile 派生裁判标尺**（销售域出销售 rubric、情感域出陪伴 rubric、极性自动翻转）。比 adversarial 硬编码 `JUDGE_SYSTEM` 更先进、可跨域。**统一内核 `judge_conversation` 必须站在它肩上**——复用 `build_judge_rubric` 出"判什么维度"，再叠加本 spec 的"喂什么底料(J1/J2)+什么粒度(J4)"。**不另起炉灶**。它现管标尺不管底料，正是统一内核要补的缺口。
- **J6【新失真点】轨迹裁判已有雏形但未校准**：`dynamic_adversarial` 已有 **R5.2 轨迹裁判评整段对话**（J4"对话级总评"的雏形！），但**校准未达标、只 ledger 不进门**。与 J3 同根——轨迹裁判和 roleplayer 都缺校准锚定。阶段 3（对话级）应吸收 R5.2 轨迹裁判、阶段 4（roleplayer 校准）应同时给轨迹裁判补校准。

**全套结论**：18 个文件无一漏网。健康的（①）保留；词表病灶（②）随 common 函数重构自动覆盖；更优资产（`build_judge_rubric`）被统一内核吸收复用而非重造；新失真点 J6（轨迹裁判未校准）并入阶段 3/4。**统一内核重构覆盖全部牵扯 LLM 评判的测试，不只 adversarial+ops。**

## 五、影响面与边界

- **测试 only**：不碰 src/ 生产（prompts/guards/gateway 一律不动）。
- 改动文件：`tests/real_llm_adversarial.rs`（评判内核 + 对话级 + 红线硬门 + roleplayer 校准）、`tests/common/judge.rs`/`roleplayer.rs`（若内核下沉到 common 复用）、`tests/common/redline.rs`（词表降级/删除）、`tests/real_llm_ops_smoke.rs`（改走统一内核）、cross_domain_arc/dynamic_adversarial（同步）。
- **风险**：(1) 评判依赖裁判可用性——Skipped 不假绿兜底。(2) 喂更多底料 → 单次 judge prompt 更长、token 涨——**关键成本约束**：adversarial CI 已在 45min 墙 + 端点并发上限 2 下，逐轮全量喂底料可能撞墙/触 429。缓解=**分层喂**：逐轮裁判只喂该轮真正需要的底料（红线轮喂 transcript、产品轮喂 knowledge、其余精简）；全量底料只在对话级总评（每弧 1 次）喂。(3) 改动面大——分阶段落地（见五）。
- **不做**：不改生产运行期红线守卫；不改被测 agent 的 prompts（那是 C2 等 agent 优化的范畴，与本"评判体系"重构正交）。

## 六、分阶段落地（大改拆小步，每步可验证）

1. **阶段 1 - 评判内核 + 底料注入（J1/J2/J5）**：抽 `judge_conversation`，给逐轮裁判喂 knowledge/memory/commitments/profile，改写 prompt 维度定义对照底料。验证：J1 编造样本（说知识库没有的）被 factualRestraint 抓到。
2. **阶段 2 - 红线对话级硬门（T2/T3/红线）**：autonomyRisk 多采样硬门取代词表 panic，必传 transcript。验证：T3 复现样本（拒绝转人工）Clean、真承诺 Breach、同句两 transcript 结果相反。
3. **阶段 3 - 对话级总评（J4）**：弧末 whole-arc 评判，加 overall_progress 等维度。验证：C2 兜圈样本被 overall_progress 抓到。
4. **阶段 4 - roleplayer 校准（J3）**：`t_roleplayer_calibration` + 真实性 band。验证：离谱失真攻击被判 miss。
5. **阶段 5 - 全弧迁移 + 词表下线**：ops/cross_domain/dynamic 改走统一内核，删词表硬门。验证：全套真模型 run 绿、无假阳、真红线仍拦。

每阶段独立 commit + CI 验证，不一次性大爆炸。

## 七、验证（贯穿各阶段）

- 纯函数单测：评判聚合（median-of-max、阈值三态、对话级维度提取）。
- **底料依赖验证**：同一 reply 喂"有知识库 vs 无知识库"→ factualRestraint 应不同（证明 J1 真用上底料）。
- **上下文依赖验证**（核心，承原 spec）：同句放两种 transcript → autonomyRisk 结果相反。
- **对话级验证**：每轮 7 分但全程兜圈的 transcript → overall_progress 低分。
- **roleplayer 校准验证**：固定情境，离谱攻击判 miss、真实升级判 hit。
- CI：分阶段跑真模型弧，确认 T2/T3 不再假阳、J1-J5 各有抓手、真红线仍拦。
- **反过拟合守护**：所有阈值/锚点一次定，多 seed 变体验证泛化；裁判/roleplayer 靠人工金标锚定，不朝结果迎合调（[[feedback_dynamic_test_anti_overfitting]] 四铁律）。
