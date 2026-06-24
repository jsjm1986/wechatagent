# 渐进式三档机制加固 设计

日期：2026-06-24
状态：待审
关联：[[2026-06-23-progressive-prompt-three-tier-design]]（被加固的原机制）、[[project_redline_pure_llm_gate]]、[[project_agent_first_no_keyword_filters]]

## 1. 背景

渐进式三档 + 充分性自评机制（commit 前缀 `[ptier]`）上线后，经 7-agent 多维度交叉审查（实际场景视角），确认 5 个 HIGH 敞口。核心问题不在单条 finding，而在三者叠加形成的「静默劣化无人知」闭环：

- Lean 档读不到画像/记忆/知识 → 幻觉 + 空转敞口；
- 自评乐观偏差（Reply Agent 是利益相关方，倾向自评"够了"）→ 该升档却没升；
- 观测只覆盖约 1/3 路径（仅 Enough 分支 × 仅产品知识维度）→ 关系档漏判、自评 JSON 失效、Clarify/Escalate 全盲；
- run log 无 tier 字段 + 无 feature flag → 出问题既看不见、又止不住、更无法用数据验证设计。

机制在最高频的关系维护轮上悄悄降质，以"关系温度缓慢流失"呈现——私域里最贵、最难归因的损失。

## 2. 设计原则

**强升闸堵确定高危，观测盯不硬堵的灰区，运维开关给止损。三者职责正交、不重叠。**

- 强升（确定性硬动作）：coverage=missing 且需知识、missing_tier 非法 → 当场升 Full，不留给观测。
- 观测（先观测后判罚）：weak 乐观、关系档漏判、自评 JSON 失效 → 只记 telemetry，不改决策。
- 运维开关：一键退回单程 Full，给上线初期止损 + 灰度 + A/B 抓手。

全程 agent-first：判据是 sufficiency / coverage / knowledge_need 的客观字符串匹配 + 结构判断，**不引入任何关键词词表 / 文本启发式**。

## 3. 五块改动

### 块 A — sufficiency.rs 纯谓词层（零依赖，先做）

纯函数，无 DB/state，最易单测、最不易冲突，打头阵。

1. 新增正交谓词 `should_force_full_on_missing(decision, knowledge_coverage) -> bool`：
   `sufficiency=="enough" && knowledge_coverage=="missing" && decision_requires_knowledge(decision)`。
   决定块 B 的②强升。正向精确匹配，绝不用 `!=`。
2. `is_coverage_optimism` 收窄：coverage 集从 `{missing, weak}` 改为**仅 `weak`**。
   强升接管 missing，观测只盯 weak 灰区。两谓词正交、各自单测。
3. ④回落改 Full：`decide_tier_escalation` 里 `missing_tier` 非法值的兜底从
   `_ => PromptTier::Relational` 改为 `_ => PromptTier::Full`（更保守，复合高价值轮不被卡在无知识档）。
   对应单测 `test_need_more_context_invalid_tier_falls_back_to_relational` 改名 + 断言更新为 Full。
4. 新增纯谓词 `is_sufficiency_recognized(decision) -> bool`：sufficiency 是否落在
   `enough / need_more_context / need_clarification` 三态内。供块 B 的 `ptier_self_assessment_malformed`
   观测判据用（false = 落了 `_=>` 兜底 = 静默降级）。单测覆盖三态 + 空/乱值。

**agent-first**：全是字符串客观匹配，无词表。✓

### 块 B — gateway 两程循环接线（依赖 A）

在 `run_user_operation_gateway_inner` 第一程判 `TierDecision::Enough` 后：

1. ②强升：先查 `should_force_full_on_missing`。命中 → 写 `ptier_forced_full` 事件
   + 调 `decide_reply_with_promote(..., PromptTier::Full)` 第二程重生成。
   **最多一次**：Full 第二程结果直接进五闸，不再触发强升检查（Full 已是最高档，无循环）。
2. ①观测：未强升时查收窄后的 `is_coverage_optimism`(仅 weak) → 写 `ptier_coverage_optimism`
   （保留现状逻辑，只是判据变窄）。
3. ①对称观测补全：
   - `ptier_self_assessment_malformed`：第一程 sufficiency 落到 `_=>` 兜底（空/乱值，
     不在 enough/need_more_context/need_clarification 三态内）时写一条，捕捉静默降级。
     判据用一个纯谓词 `is_sufficiency_recognized(decision) -> bool` 在 sufficiency.rs 提供。
   - `ptier_relational_optimism`：sufficiency=enough 但本轮触及关系信号（contact.intent_trajectory
     非空 或 近期 reaction 非空）却停在 Lean（未升档）时写一条，捕捉关系档漏判。
   - 让 `is_coverage_optimism` / 自评观测覆盖 Clarify / Escalate 分支（目前观测只在 Enough 分支跑）。
4. ⑤修 `used_knowledge_ids` 口径：Lean 第一程未实际注入知识（include_business=false）时，
   终决策不记路由命中的 id（避免架空 grounding 确定性硬闸——硬闸取 used∩verified 交集非空即放行，
   Lean 没读切片却记了路由 id 会让"路由命中过"被误当"Agent 读过"）。
   gateway 有 5 处 `route_used_knowledge_ids` 赋值，**只改 Lean-Enough 终决策那一处**（gateway.rs:1066 附近，
   以实际行号为准），升档第二程 / revised / 初始 planner 路径不动（它们要么是 Full、要么本就该带路由 id）。

**风险点**：gateway 是共享热点 + `used_knowledge_ids` 5 处赋值需精准只改 1 处。实现时先 grep 全部赋值点核对。

### 块 C — config 运维开关（独立，可并行）

新增 `PROGRESSIVE_TIER_ENABLED`，默认 `true`（`parse_bool(env_or("PROGRESSIVE_TIER_ENABLED", "true"))`——
注意与现有多数默认 `false` 的 `*_ENABLED` 开关相反）。

gateway 第一程的 tier 选择读这个 flag：开 → `PromptTier::Lean`（三档生效）；关 → `PromptTier::Full`
（第一程直接全量，退回 ptier 前的单程行为，等于 kill switch）。配合块 B 的埋点可做账号级灰度 + A/B 量化。

**agent-first**：纯运维开关，不碰决策语义。✓

### 块 D — run log tier 字段（不碰 models.rs）

审查建议给 `AgentRunLog` 加 `tier_used/sufficiency/escalated` 字段。**改进**：`AgentRunLog` 已有
`gateway_result: Document` 自由字段（models.rs:2463），tier 信息塞进该 Document（键
`tier_used` / `sufficiency` / `escalated` / `forced_full`）即可，**完全不动 models.rs**——
避开和并行会话在 models.rs 的高频冲突。这块改动并入块 B 的 gateway 写 run log 处，不单独成块。

### 块 E — prompt 收紧（最后，bump 版本）

③ `user.reply.task` 模板 sufficiency 字段说明处加一句约束：
"need_clarification 时 replyText 只能是澄清问句本身，不得给推测性答案"。
bump `PROMPT_PACK_VERSION`（当前 v10 → v11）。

放最后做：先确认与并行会话的 prompt schema 改动无文本冲突，再 bump。
ensure_prompt_pack_v2 语义：bump 版本号后，全新 DB / reset 自动重种；已部署环境需 reset-system-pack 激活
（与 v8 教训一致，记入潜在后续）。

**agent-first**：纯 LLM 语义约束。✓

## 4. 实现与提交顺序

主工作区小步快提交，每块编译验证（`cargo test --lib` 相关模块）后立即 commit，不积压未提交改动
（上次 ptier 改动被并行会话 `git stash` 卷走的教训）。全程 `[ptier]` 前缀，只 `git add` 自己文件，
绝不 `-A`，共享文件提交前核对未误纳并行会话改动。

1. **块 A**（谓词 + 单测）：零依赖先锁住，纯 lib 单测可验。
2. **块 B + D**（gateway 接线 + run log Document 字段）：核心，最大爆破面。
3. **块 C**（config 开关）：独立。
4. **块 E**（prompt + bump v11）：最后单独一提交，隔离版本号风险。

## 5. 测试策略（遵循 [[project_universal_test_coverage]]：测试不改生产 prompt/guards，纯函数确定性测 + 真模型测能力）

1. 纯函数：`should_force_full_on_missing` 三条件 AND + 正向匹配陷阱（not_required/unknown 不命中）；
   `is_coverage_optimism` 收窄后仅 weak 命中、missing 不再命中（归强升）；
   `is_sufficiency_recognized` 三态识别 + 兜底；④回落 Full 断言。lib 单测。
2. 向后兼容：观测/run log Document 新键缺省不破坏既有反序列化。
3. 真模型（#[ignore] 留 CI）：扩 `real_llm_progressive_tier.rs`——
   产品问询 coverage=missing 轮应出现 `ptier_forced_full`（强升真生效）；关系维护轮触发 `ptier_relational_optimism` 观测。
4. 基线：lib ≥ 350 不回归（新增测试只增量叠加）。

## 6. 风险与权衡

- **强升成本**：高危轮多一次 Full 第二程（Lean completion 废弃 + 全量 DB 重载 + Full LLM）。
  权衡：只在 coverage=missing 且需知识的确定高危轮付，换来堵住凭空承诺非产品事实。可被 `PROGRESSIVE_TIER_ENABLED` 一键关掉。
- **观测分母**：收窄 is_coverage_optimism 到 weak 后，missing 那部分转为强升（不再进观测）。
  这是有意的——观测只盯不硬堵的灰区，missing 已被强升处理。
- **回落 Full 更保守**：missing_tier 非法值升 Full 比 Relational 多注入业务槽位、成本略增。
  权衡：非法值本就罕见（LLM 输出畸形），保守注入避免复合高价值轮被卡，值得。
- **prompt bump 与并行会话**：v10 是并行会话所 bump，v11 须先核对 schema 文本无冲突再动。
- **Clarify 收紧不落 hold**：本轮用 prompt 约束"只输出澄清问句"，不做 should_hold 静默挂起
  （上轮交叉审查已证伪静默 hold 触发恢复死锁）。仍保留"发澄清问句"方向，只是约束其不硬答。

## 7. 不做（YAGNI）

- 不给 AgentRunLog 加强类型 tier 字段（用既有 Document）。
- 不做强升的多次重试（最多两程铁律）。
- 不动 simulation 影子模式的单程行为（影子不直接面客户，文档注明"影子≠生产 Lean 行为"即可）。
- 不引入任何词表 / 文本启发式判断（agent-first 红线）。
