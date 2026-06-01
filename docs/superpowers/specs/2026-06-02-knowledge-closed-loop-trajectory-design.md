# 知识库闭环轨迹测试设计（维护 agent 编辑 → 再召回 → 召回维持）

- **日期**：2026-06-02
- **状态**：已批准设计，待写实施计划
- **范围**：评估驱动的「优化-测评」能力的**第一个具体交付物**——闭环轨迹测试。**不建通用框架壳**（YAGNI，当前只有一个消费者）。

---

## 1. 背景与动机

### 1.1 用户意图

用户要**借鉴大模型微调 / 强化学习的方法论**（注意：**不是**微调 MiMo 的权重，provider 的 fine-tuning API 与本设计无关），来同时**优化**和**评估** agent。经澄清与脑暴，落地为一个**闭环轨迹测试**：

> 维护 agent 编辑知识库 → 重新召回 → 召回率得到维持。

并覆盖用户明确点出的三件事：(a) 渐进式披露召回；(b) 确定性排序；(c) agent 发现召回不对时**能返回目录继续查找或换其他召回方法**（回溯/重搜）。

### 1.2 RL 方法论 → 测试设施的映射（仅概念脚手架，不落地为抽象层）

| RL 概念 | 本系统对应 | 优先级 |
| --- | --- | --- |
| Reward model（奖励模型） | Phase 1 双裁判团 + 校准锚 | 辅助信号 |
| Constraint（KL/安全惩罚） | 硬红线断言（cite⊆opened、draft+needs_review、verified-only、关系图完整） | **高于 reward** |
| Policy（策略） | prompt / rubric / 阈值（人读分后更新，非自动） | 人工更新 |
| Train / Holdout | Q2 泛化门（抽成可复用件） | 抗过拟合主门 |
| Trajectory（轨迹） | 召回-trace 闭环（**本设计要补的缺口**） | 本轮交付 |

**关键纪律**：约束（红线）**优先于** reward。任何时候硬红线断言为真理，judge 分只是辅助。这与既有质量套件「judge 恒判分但红线不可放水」一致。

### 1.3 为什么是这个交付物（而非通用框架）

脑暴中我主动泼冷水：通用「优化-测评框架壳」是**过早抽象**（YAGNI，只有一个消费者），且 judge/reward 是**最不稳定**的信号。用户认同，范围收窄为：**具体的闭环轨迹测试**，确定性召回门为**主**、judge 为**辅**；外加把 Q2 泛化门抽成可复用件。框架的 RL 角色映射只作概念脚手架，不建抽象层。

---

## 2. 已核实的生产现状（设计据此，不重写生产代码）

### 2.1 召回是查询时实时计算的——没有可重建的物化索引（硬事实）

`src/agent/knowledge_agent.rs`：

- `rank_key`（:1444）= `effective_relevance_micros` × trust × recency；trust 乘子 `if superseded {0.1} else {1.0} * if expired {0.5} else {1.0}`（:1456）；`live = !superseded && !expired`（:1460）。
- `relevance_score`（:1590）= query↔chunk **bigram 覆盖**打分，对中文友好。
- 设计注释（:77-81）明确：**故意不用向量库、不用 `$text`**（MongoDB `$text` 不分词 CJK；向量库 = 新依赖 + 部署拓扑变更）。

→ **不存在物化的「原子/多视图索引」需要在写入后 rebuild**。写进 `operation_knowledge_chunks` 本身就是更新；「再召回」只是拿同一 query 对**已更新的集合**再跑一次实时打分。

> **设计修正**：早期草案里的「重建原子索引」步骤**删除**，改为「应用写入 → 直接 live 再召回」。好处：没有索引重建的时间窗 / 一致性问题要测，闭环更干净。

### 2.2 多轮回溯是内建的，但未被测过

`knowledge_agent.rs` 多轮循环（:660-948）：`for round in 1..=max_rounds`（:671，`MAX_ROUNDS=4`），每轮 `build_prompt`（:700）重建提示，LLM 自由 emit 5 个 action 之一：`list_catalog` / `open_document` / `open_chunk` / `follow_relations` / `answer`（match :780-915）。**没有状态机强制路径**——agent 可重新 `list_catalog`（回溯）或切到 `follow_relations`。状态跨轮累积（`merge_catalog` :811/:862），`opened_seen` 去重（:827/:867）。循环耗尽无 Answer 时回退（:918-948）。

→ **现有测试只覆盖 happy path**（脚本化 mock）。回溯/重搜从未被断言过。

### 2.3 维护 agent 工具全是只读/提案——从不直接写库

`knowledge_tools.rs` chat 工具白名单（:649-674）共 10 个：`list_catalog` / `search` / `open_slice` / `audit_completeness` / `search_chunks` / `propose_repair` / `analyze_logs` / `open_document` / `inspect_pack` / `verify_anchor`。**无一个工具物理写入或审定**。真实 KB 变更走 operator-workbench 审批 → 生产写函数。

### 2.4 生产写入口（闭环 apply 必须走这些，不能让 agent 直写）

`src/routes/knowledge.rs`：`apply_create_chunk`（:5631）、`apply_update_chunk`（:5666）、`apply_update_pack`（:5737）、`verify_operation_knowledge_chunk`（:569）。

### 2.5 可复用的现成件

- **确定性召回门模板**：`tests/knowledge_agent_eval.rs` `knowledge_agent_eval_set_meets_thresholds`（:128）——testcontainers MongoDB + mock LLM，`#[ignore]`，seed chunk + 脚本 LLM step，断言 `cited_hit_rate() >= 0.80`（:231）。
- **Q2 泛化门**（待抽取）：`tests/real_llm_knowledge_quality.rs`（:1331-1458）——`train_recalls`/`holdout_recalls`，`gap = (train_mean - holdout_mean).abs()`，断言两 mean ≥ `MIN_RECALL_FLOOR` 且 `gap <= MAX_GENERALIZATION_GAP`（=0.18）。
- **Phase 1 双裁判 + 校准 + 分歧门**：`tests/real_llm_knowledge_quality.rs` 已落地（双 checkpoint 裁判、校准锚 `CALIB_MIN_GAP=2.0`、`DIVERGENCE_SKIP_THRESHOLD=3.0`）。

---

## 3. 设计——一条轨迹 + 三门

### 3.1 轨迹（一次变更的完整生命周期）

```
seed 语料（已 verified 的 chunk 集合）
  → 测基线召回（对固定 query 集跑 answer，记 cited 命中 / 排序）
  → 维护 agent 提案（propose_repair / 加标签 / 补证据 / 用新版取代旧版）
  → 经审批走生产写入：
       apply_create_chunk / apply_update_chunk  → 落 draft + needs_review
       verify_operation_knowledge_chunk         → draft → verified
  → 再召回（live，无索引重建，对同一 query 集重跑）
  → 断言（见三门）
```

### 3.2 门 1：召回排序保持门（**主门**，确定性，testcontainers + mock LLM）

确定性、可在 CI 无真 LLM 跑，是闭环的**地基**。断言：

1. **不回归**：基线命中的 query，写入后仍命中（命中率不降）。
2. **新内容可召回**：新增/补强的 chunk 对其目标 query 可被召回。
3. **SUPERSEDE 旧降新升**（确定性可验）：旧 chunk 被 `superseded_by` 打标 → `rank_key` ×0.1 降权 → 新 chunk 必须排到旧的前面。
4. **关系图完整**：写入触碰 `related_chunks` / `superseded_by` 后，被指向 chunk 仍可达，`follow_relations` 无悬空引用。
5. **负例——未批准 draft 不可召回**：agent 提案在**未走审批**前是 `draft + needs_review`，此时**不得**被服务/召回（cite⊆verified-only，K4 红线）。

复用 `knowledge_agent_eval.rs` 的 `cited_hit_rate` 形态与 mock LLM 脚本机制。

### 3.3 门 2：渐进式披露 + 回溯召回门（辅助，真 LLM，分歧/不可达则 skip）

测 2.2 节那条**从未被测的回溯路径**。手法：**故意让首跳 miss**（query 与首个 catalog 命中错位，或目标内容在 `follow_relations` 一跳之外）。断言：

- trace 出现**回溯/换方法**步骤（重新 `list_catalog` 或切 `follow_relations`）。
- 终引（最终 cite）命中目标 chunk。
- cite⊆opened 全程成立。

真 LLM 驱动 orchestration（选哪个工具、query 词）；缺 key / 瞬时不可达 → skip 不 panic。

### 3.4 门 3：答案质量奖励门（辅助，真 LLM，复用 Phase 1）

复用 Phase 1 双裁判 + 校准锚 + 分歧门：断言编辑后答案质量**不变差**（overall 不低于编辑前 floor）。分歧大 → skip（裁判飘，不算被测对象 fail）。

### 3.5 抗过拟合：抽取 Q2 泛化门为可复用件

把 `real_llm_knowledge_quality.rs:1331-1458` 的 train/holdout gap 逻辑抽成可复用函数（纯函数 + 单测），套到闭环召回上：对**单条被编辑 chunk** 的召回提升不得以牺牲 holdout query 召回为代价（`gap <= MAX_GENERALIZATION_GAP=0.18`）。防止「为让这一条 chunk 召回好看而过拟合」。

---

## 4. 红线（全程不破）

1. 提案恒 `draft + needs_review`；agent **永不自动落库 / 自动审定**——闭环 apply 必须走 operator-审批生产写入（`apply_*` / `verify_*`），不是 agent 直写工具。
2. `cite⊆opened`；`verified-only`（draft 永不被服务）。
3. **结构化写永不物理删除**——SUPERSEDE 是打标降权，不是 delete；门 1 第 3、4 条验证。
4. judge 恒判分但**绝不放水**——红线（约束）优先于 reward；修生产代码绝不降断言阈值。
5. 抗过拟合：改 prompt/rubric/阈值只沉淀**可复现抽象方法论**，不对单条对话/单次 CI 样本点对点修补。
6. MCP 永远 wiremock 桩；缺 key / 瞬时不可达 skip 不 panic。
7. 安全门只更严不更松。
8. lib 基线 ≥350（当前 824/0）；PBT 累计 ≥33/0（当前 34/0）不回归；新纯函数单测挂测试文件不进 baseline 门。
9. 禁词 lint（check-no-human-takeover / check-no-model-hint）措辞守住。

## 5. 范围边界

**本轮做**：门 1（确定性主门，含 SUPERSEDE + 关系图 + 负例）+ Q2 泛化门抽取。门 2 / 门 3（真 LLM 辅助门）按 CI 时间预算评估是否同轮或紧随。

**本轮不做**（YAGNI / 留后续）：通用「优化-测评框架壳」抽象层；真异族裁判家族；前端知识库 UI 重设计；提门槛（floor 6.0→8.0）。

## 6. 文件改动面

| 文件 | 改动 |
| --- | --- |
| `tests/knowledge_closed_loop_trajectory.rs`（新增，确定性主门，可 `#[ignore]` 走 testcontainers） | 主改：轨迹 + 门 1；复用 `knowledge_agent_eval.rs` mock 机制 + `cited_hit_rate` 形态 |
| `tests/real_llm_closed_loop.rs`（新增，门 2/门 3 真 LLM，视 CI 预算同轮或紧随） | 渐进式披露+回溯门、答案质量奖励门；复用 Phase 1 双裁判设施 |
| `tests/real_llm_knowledge_quality.rs`（或抽到共享处） | 抽取 Q2 泛化门为可复用件 + 纯函数单测 |
| `.github/workflows/ci.yml` | 视门 2/3 落地情况加 job / 台账 |

**只读依据不改**：`knowledge_agent.rs` / `knowledge_tools.rs` / `routes/knowledge.rs`（生产写函数）/ `real_llm_adversarial.rs`（他人维护）。

## 7. 验证

1. `CARGO_TARGET_DIR=target-check cargo test --test <new>`：确定性主门 + 抽取的泛化门纯函数单测 pass；含 `#[ignore]` 真测的二进制编译过。
2. `cargo test --lib`：lib 基线不回归（≥350/0）。
3. `scripts/check-baseline.sh` 本地过；禁词 lint 过。
4. push → 读 CI：确定性主门绿（含 SUPERSEDE / 关系图 / 负例）；真 LLM 辅助门按 skip 纪律产出。

## 8. 回滚

纯增量加测试文件 + 可选 CI job；无 schema / 无 lib / 无生产代码改动。`git revert` 即可。
