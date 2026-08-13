# 优化第五波 · 演化器结果信号换血 实施计划（线 H）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 演化器的显著性判定从"评审放行率"（`SEND_SUCCESS_STATUSES`，过程指标——演化器在优化"让自己的审查者点头"）换血为**真实用户反应结果**，对齐动态置信度已走通的信号标准（买入信号=正、负向集=负、无反应=删失）。

**Architecture:** 换标签不换估计器（与置信度换血同哲学）：cohort/replay/release 流水线结构零变化，只把 significance 的"成功"定义从终态字符串集合改为按 `run_id` join `AgentDecisionReview.outcome_status` 的三态分类；shadow 侧（replay 无法产生未来的用户反应）用"源 run 的真实反应"作为标签、新旧配置的差异体现在"新配置是否会放行/拦截该 run"与"该 run 的真实结果"的交叉矩阵上。安全反向门（#152）语义保持。

**背景锚点（实施前逐一重验）：**
- `src/evolution/significance.rs`：`SEND_SUCCESS_STATUSES = ["approved", "revision_applied_approved"]`（缺陷 #16 波已修 pressure 归因，本波动成功定义本身）。
- `src/knowledge_wiki/gap_signals.rs` 的 `classify_outcome_label`：Hit=`user_replied_buying_signal`、Block=五类负向（复用 `reaction::is_negative_outcome` 单一真相源）、其余=Censored——**本波的分类器直接复用/搬迁此逻辑，不重写**。
- `AgentDecisionReview.outcome_status` 由 reaction 在下一轮 inbound 写入（删失=客户没再说话）。
- `post_release.rs` 已有 `actual_negative_reaction_rate_delta` 观测（仅观测不判定）——本波把同族信号提升进 promote/reject 判定。
- 23 号终裁 10-5 与 C2 修复：pressure gate 已无终态统计源、候选停产——换血后 pressure 仍不恢复候选（真实反应信号也无法归因到单一闸），保持停产。

## Global Constraints

- 模型：仅 Fable 5 1M max（不派生 subagent）。
- 红线：evolution 隔离（不触 gateway/outbox/MCP，收尾跑隔离 lint）；动手前全文读懂 significance/replay/threshold/post_release 与 gap_signals 分类器；禁词 lint；不加 feature flag（新语义即默认——未上客户+演化器本身默认关，风险受控）。
- 文件边界：`src/evolution/**`、`tests/**`；`gap_signals.rs` 的分类器若需共享，**抽到 `src/agent/reaction.rs` 或独立纯函数模块由双方引用**（gap_signals 是知识域文件——若抽取涉及它，仅限"删本地实现改为引用共享"的机械替换）。
- 每任务独立 commit；收尾 `cargo test --lib`（≥2562）+ 四 PBT + `-D warnings` + evolution 隔离 lint + 禁词 lint。

---

### Task H1: 共享结果分类器

**Files:**
- Create/Modify: 结果三态分类纯函数的单一真相源（评估现状后选址：`reaction.rs` 已有 `is_negative_outcome`——倾向把 `classify_outcome_label` 的三态逻辑上移到 reaction 域或 `src/agent/outcome_label.rs` 新纯函数模块）
- Modify: `src/knowledge_wiki/gap_signals.rs`（改为引用共享实现，行为逐字节等价——用现有单测锁）
- Test: 分类器单测搬迁/补齐（Hit/Block/Censored 三态矩阵）

**Steps:**
- [ ] 读 `classify_outcome_label` 与 `is_negative_outcome` 全文及全部调用方 → 选址并抽取 → gap_signals 现有测试全绿（行为零变化）→ Commit：`refactor(outcome): extract shared three-state outcome classifier`

### Task H2: significance 换血

**Files:**
- Modify: `src/evolution/significance.rs`（核心）、`src/evolution/replay.rs`（如需在 ReplayOutcome 上携带源 run 的 outcome 标签）、`src/evolution/threshold.rs`/`envelope.rs`（口径注释与字段名同步）
- Test: significance 单测重写（交叉矩阵语义）

**行为契约:**
- 每条 completed replay 关联源 run 的真实结果标签（按 run_id → decision_review.outcome_status → 三态分类；review 缺失或 Censored 的 replay **不进判定分母**，与置信度删失语义一致）。
- 新判定指标（替换 send_success_delta）：**结果加权放行差** `outcome_weighted_delta` = 新配置下「放行∧Hit」占比 −「放行∧Block」占比，与旧配置同口径求 delta；promote 门槛沿用 `EVOLUTION_MIN_SEND_SUCCESS_DELTA` 的 env 值语义（更名为语义中立读取，env 名不改保持部署兼容，注释说明新含义）。
- **样本量硬门**：非删失样本 < `EVOLUTION_MIN_REPLAYS`（复用现有 env）→ 候选直接 `rejected_below_threshold`（reason=`insufficient_outcome_samples`）——冷启动/低互动期演化器自然静默，这是特性不是缺陷（写注释）。
- #152 安全反向门保持原语义（拦截状态集合不变），叠加在新指标之上。
- 5 闸命中率涨幅门（`EVOLUTION_MAX_5GATE_HIT_INCREASE`）保持不变（它防的是"改配置让闸空转"，与结果信号正交）。
- `experiments`/`proposals` 的评估摘要字段增记三态分布（BSON 加法字段 serde default，向后兼容）。

**Steps:**
- [ ] 全文读 significance/replay 现实现 → 失败单测（构造正/负/删失混合 cohort 的交叉矩阵场景）→ 实现 → evolution 全部测试绿 → Commit：`feat(evolution): grade proposals by real user-outcome deltas instead of review pass-rate`

### Task H3: post_release 口径统一

**Files:**
- Modify: `src/evolution/post_release.rs`（+24h 对比指标与新判定同口径：三态分布 delta 为主、负反应率保留）
- Test: post_release 单测同步

**Steps:**
- [ ] 读现实现 → 对齐指标口径（观测性质不变，仍不自动回滚）→ Commit：`feat(evolution): align post-release review metrics with outcome-based grading`

### Task H4: 档案与事实卡同步材料

- 产出交付报告中的"档案回写要点"（10 号记录 significance 节、30 号事实卡演化参数行、台账缺陷 #16 关联段、agent-policy.md 演化章节的 delta 描述——由主会话执行回写与文档波级修正）。

### 收尾

- [ ] `cargo test --lib` ≥2562/0、四 PBT、`-D warnings`、evolution 隔离 lint、禁词 lint。
- [ ] 交付报告（≤15 行）：H1 选址结论、H2 交叉矩阵语义与样本门行为、与 #152/5 闸门的叠加关系亲验结论、测试结果、commit hashes、档案回写要点。
