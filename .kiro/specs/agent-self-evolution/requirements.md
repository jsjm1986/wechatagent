# Requirements Document

> 中文标题：用户运营 Agent 自我演化（agent-self-evolution）— 需求文档

## Introduction

本期工作（M4）在 `agent-autonomy-loop`（W0–W6 已落地）+ `user-ops-agent-hardening`（24 任务全部完成）+ M3 Strategic Planner（silent / commitment / stagnation 三段反馈环 + 优先级 + Planner 自我监控）之上，加入"AI 自己学"这一层：让系统基于已经持续生产的 outcomes / Planner / agent_run_logs 数据，**自动**为下一次 run 调整 5 闸阈值与 prompt 文案，但任何变更必须先在 shadow 模式下通过历史回放评测，且最终由 admin 在 UI 上一键发布。

**产品定位（不可妥协 · 与上游 spec 一致）**：本系统是**全 AI 自治流程**。本期不引入任何"人工接管 / 等人工处理"概念。Self-Evolution 产出的所有变更建议（threshold delta / prompt diff）通过的语义都是"AI 自身在历史数据上得出的迭代提案"，运营人员的角色是**审阅 + 一键发布**，不是"接管 AI 的判断"。所有新事件 kind / 状态文案 SHALL 过 `scripts/check-no-human-takeover.{sh,ps1}` lint。

升级目标分两大维度：

1. **阈值自适应（Threshold Adaptation）**：基于 outcomes 主表 + 5 闸命中率 + Planner backoff 计数，在 shadow 期内对 5 闸阈值（FactRisk / PressureRisk / HumanLikeScore / EmotionalValue / ProductAccuracyScore）和 Planner 反馈环阈值（block_rate_threshold）形成"阈值候选 + 预估命中率变化 + 预估 send_success_rate 变化"评测报告；admin 点确认后**应用为生效阈值**。
2. **Prompt / Soul 演化（Prompt Evolution）**：基于失败 run（`finalReviewStatus ∈ blocked-like / revision_failed`）批量回放，由 Critic LLM 生成"prompt diff 候选"，shadow 模式下并行评测 N 条历史 run（同样输入、新 prompt vs 现行 prompt），admin 看 win-rate / token cost / 5 闸命中率 + selfCritiqueAddressed 对比后一键切换 `prompt_templates` 当前版本。

**强约束（不可放松，与既有 spec 衔接）**：

- **不绕过 gateway**：M4 的所有演化都只改"输入到 gateway 的参数"（阈值、prompt 文本），gateway 链路（reload context → 5 闸 → review → revision → outbox → MCP）一行不动。
- **不绕过 5 闸**：演化产出的"新阈值"在生效后仍由现有 review 链路使用；Critic 输出的 prompt diff 在生效后仍要走 R5 verified knowledge 等强约束（自我演化不能"教 Reply Agent 绕安全门"）。
- **不直连 MCP / 不发消息**：evolutionary worker 永远不写 `agent_send_outbox`、不调 MCP、不调 Reply Agent 处理实时消息；只读 `agent_run_logs / agent_outcome_metrics / agent_events / conversation_messages（只读）` 与写自己的 4 张新表。
- **不引入新业务 collection 替代既有数据源**：演化器读的所有事实都来自既有的 7 张 collection（`agent_run_logs / agent_outcome_metrics / agent_events / conversation_messages / agent_decision_reviews / agent_send_outbox / contacts`），不再造一份"演化事实表"。
- **shadow eval 是发布前置条件**：任何 threshold delta 或 prompt diff 在没有 shadow eval 通过 + admin 显式确认前，**不**进入生效配置。
- **回放禁止真实出站**：shadow 评测的 run 走"模拟 gateway"路径——可以调 LLM（用候选 prompt 算决策与 review），但 outbox / MCP 一律 short-circuit；Critic LLM / shadow LLM 调用的 token 计入"演化预算"，与生产 RunBudget 隔离。
- **测试基线（与 R11.6 一致）**：`cargo test --lib` ≥ 78 + 本期新增；4 PBT 累计 ≥ 33。`scripts/check-baseline.sh` / `scripts/check-no-human-takeover.sh` 都 SHALL pass。
- **不破坏向后兼容**：现有 `prompt_templates / system_taxonomies / agent_souls / operation_playbooks / agent_outcome_metrics` 字段集 SHALL NOT 被破坏；新表新增、新字段新增。
- **Self-Evolution 不调阈值的"阈值"**：演化器不能"自我演化自我调阈值的策略"——M4 的 Critic 提示词、shadow eval 阈值、显著性测试参数都是**常量 / env**，不进入演化循环（避免无限自我引用）。

## 状态枚举映射表（experiments × proposals × shadow_replays）

本期定义两条主链路：实验（experiment）周期与候选提案（proposal）发布。两套状态独立、不混用：

- **`experiment_status`**：演化器一轮工作的过程状态。
- **`proposal_status`**：单条候选阈值 / prompt 提案的状态。
- **`shadow_replay_status`**：单条 shadow 回放结果的状态。

| 触发条件                                                                                | experiment_status                  | proposal_status                | shadow_replay_status            |
| ---------------------------------------------------------------------------------- | ---------------------------------- | ------------------------------ | ------------------------------- |
| 演化器 tick 启动并加载 outcomes 数据                                                          | `collecting`                       | -                              | -                               |
| 演化器产出 N 条候选阈值 / prompt diff                                                          | `proposed`                         | `pending_eval`                 | -                               |
| Critic / shadow LLM 已对每条候选发起 ≥ K 条历史 run 的并行回放                                          | `evaluating`                       | `pending_eval`                 | `pending`                       |
| 单条历史 run 在 shadow 中跑完                                                                  | -                                  | -                              | `completed`                     |
| 单条 shadow 回放因 LLM 超时 / JSON 错 / 预算超额而失败                                                    | -                                  | -                              | `failed`                        |
| 候选提案的 shadow eval 全部完成且通过显著性阈值                                                          | `awaiting_admin`                   | `eligible_for_release`         | -                               |
| 候选提案的 shadow eval 全部完成但**未**通过显著性阈值                                                      | `awaiting_admin`                   | `rejected_below_threshold`     | -                               |
| admin 在前端"演化中心"对一条 `eligible_for_release` 候选点确认                                          | `released`                         | `released`                     | -                               |
| admin 在前端对一条 `eligible_for_release` 候选点放弃                                              | `released`                         | `discarded_by_admin`           | -                               |
| 整轮 experiment 因预算超额 / Mongo 不可用等被强制终止                                                    | `aborted`                          | `aborted`                      | -                               |
| 已发布提案被 admin 触发回滚                                                                       | `released`（不变）                 | `rolled_back`                  | -                               |

**说明**：

- `proposal_status` 是单条候选的生命周期，不与 experiment 主键 1:1：一轮 experiment 通常产 ≤ 8 条候选（4 条阈值 + ≤ 4 条 prompt diff），并行评测后归到同一 experiment 下。
- 任何在本表之外出现的状态值 SHALL 视为协议违规并阻断写库（同 R9.10.e 风格）。
- `released` proposals 落到生效配置：阈值类写到 `threshold_overrides`（覆盖 `runtime_parameters`），prompt 类写到 `prompt_templates` 的新版本号 + 把 `current_version` 指向新版本。
- 发布是**单方向**：rollback 不是"反向 release"，而是把 `current_version` 指回上一版（写一条 `proposal_status="rolled_back"` 审计记录，但不撤销 `released` 历史值）。

## Glossary

- **Evolutionary Worker**：本期新增的独立 `tokio::spawn` 后台 loop，间隔 `evolution_tick_seconds`（默认 6 小时）扫一次最近 24/72 小时窗口的 outcomes / run logs / events，产出 `experiment` 与 `proposal` 候选。位置：`src/evolution/mod.rs`（新模块），由 `main.rs` 在 `prompts::ensure_prompt_pack_v2` 之后启动。
- **Threshold Proposal**：阈值自适应候选，`proposal_kind="threshold"`，描述"针对哪个 gate（FactRisk/PressureRisk/HumanLikeScore/EmotionalValue/ProductAccuracyScore/PlannerBlockRate）从 X 调到 Y，预期 send_success_rate 变化为 ΔS"。
- **Prompt Proposal**：prompt 演化候选，`proposal_kind="prompt_diff"`，描述"针对哪个 prompt_template_key（如 `reply_agent_main / review_agent / memory_consolidator`）的哪一段，diff 是什么，预期 5 闸命中率 + token cost + selfCritiqueAddressed 变化为多少"。
- **Critic LLM**：本期新增的演化器内部 LLM 角色，输入 = "失败 run 的 envelope + 现行 prompt 全文 + 失败模式归类"，输出 = "prompt diff 候选 + 改动理由 + 预期改善的 finalReviewStatus 列表"。**Critic 只产文本 diff，不直连 production 链路**。
- **Shadow Replay**：取一条历史 `agent_run_logs.lifecycle="completed"` 的 run（含原始 inbound message / context / 决策时刻可访问的 memoryCard 快照），在隔离的"模拟 gateway"中以候选阈值或候选 prompt 重跑一次，记录新决策的 5 闸命中、token cost、finalReviewStatus、selfCritiqueAddressed、产出文本相对原文的余弦相似度（embedding 占位即可，可不调用）。
- **Significance Threshold**：本期常量化的显著性参数：`min_replays_per_proposal`（默认 30）、`min_send_success_rate_delta`（默认 +0.05）、`max_5gate_hit_increase`（默认 0.10，即新 prompt 不允许把 5 闸命中率加更多）、`min_self_critique_addressed_delta`（prompt 类，默认 +0.10）。
- **Threshold Override**：本期新增 collection `threshold_overrides`，存储已发布的阈值覆盖对，`{ workspace_id, account_id, gate_key, value, released_proposal_id, released_at, rolled_back_at? }`。生效优先级：`threshold_overrides` 覆盖 → `runtime_parameters` → `domain_configs` 默认。
- **Prompt Version**：`prompt_templates` 现行字段已包含 version 概念（M3 时被 `ensure_prompt_pack_v2` 维护到 v2）。本期把单 template 升级为"多版本数组 + current_version 指针"形态，老版本不删，便于回滚；同时保留对 `Vec<String>` 老形态的反序列化兼容（同 R11 风格）。
- **Evolution Budget**：每次 `evolutionary_worker_tick` 的 LLM token 上限（默认 60000）+ LLM 调用次数上限（默认 30），与生产 `RunBudget` 隔离；超额时 SHALL 降级（跳过 prompt eval、只跑 threshold eval）并写 `kind="evolution_budget_exceeded"`。
- **Replay Cohort**：本期定义的回放样本筛选规则——按 `(account_id, finalReviewStatus, started_at >= now - eval_window_hours)` 取 ≥ `min_replays_per_proposal` 条 run；阈值类候选用 success + failure 混合 cohort，prompt 类候选用 failure-only cohort（聚焦失败模式）。
- **Release Gate**：admin 一键发布前，前端 SHALL 二次确认：(a) shadow eval 数 ≥ min_replays_per_proposal、(b) 显著性已通过、(c) 不存在更新的同 gate / 同 prompt_template_key 的 `eligible_for_release` 候选（避免发布一个被新候选取代的旧版）。
- **Rollback Snapshot**：每次 release SHALL 记录"被替换的旧值/旧版本号 + 替换理由"，便于一键回滚。

## Requirements

### Requirement 1: Evolutionary Worker 主循环（演化器骨架与预算）

**User Story:** 作为运维者，我希望演化器是一个独立、可单独关停、不影响生产 webhook / Planner / tasks worker 的后台 loop，且每轮自身有预算上限和事件留痕，这样即使演化器跑挂或被 LLM 拖慢，主链路也安然无恙。

#### Acceptance Criteria

1. THE 系统 SHALL 在 `src/evolution/mod.rs` 暴露 `pub async fn run_evolutionary_worker(state: AppState)` 入口，由 `src/main.rs` 在 `prompts::ensure_prompt_pack_v2` 之后、`tasks::worker_loop` 之前 `tokio::spawn` 启动。
2. THE 演化器主循环 SHALL 以 `evolution_tick_seconds`（默认 21600，env `EVOLUTION_TICK_SECONDS`）为间隔轮询，单次 tick 全程 SHALL 在独立 try/catch 内执行——任何 panic / DB error 都 SHALL 写 `agent_events kind="evolution_tick_failed"` 并继续下一 tick，不影响主进程。
3. WHEN 单次 tick 启动，THE 系统 SHALL 在 `experiments` 集合 insert 一条 `experiment_status="collecting"` 信封，记录 `experiment_id / started_at / window_hours`；后续阶段（proposed / evaluating / awaiting_admin / released / aborted）SHALL 用 `update_one({experiment_id})` 推进，禁止再次 insert（同 R0.2 风格）。
4. THE 单次 tick LLM 用量 SHALL 受 `EvolutionBudget` 约束（默认 60000 token / 30 calls，env `EVOLUTION_RUN_TOKEN_BUDGET / EVOLUTION_RUN_MAX_LLM_CALLS`）；累计触顶后 SHALL 走降级：先停 prompt eval，再停 threshold eval，最后写 `experiment_status="aborted"`。
5. WHEN admin 通过 `EVOLUTION_ENABLED=false` 关停演化器，THE 系统 SHALL 跳过 spawn 但 main 路径正常启动；现有已发布的 threshold_overrides / prompt_templates SHALL 继续按生效优先级被读取，不因此回退。
6. THE 演化器 SHALL NOT 直接或间接调用 `run_user_operation_gateway`、不直接或间接 enqueue `agent_send_outbox`、不直接或间接调 MCP；任何对这三处的引用 SHALL 在 PR review 中被阻断（建议在 `evolution/mod.rs` 顶部加 `// FORBIDDEN: gateway / outbox / mcp` 注释作为 lint anchor）。
7. WHEN 单次 tick 全程结束（无论 released / aborted / awaiting_admin），THE 系统 SHALL 写 `agent_events kind="evolution_tick_completed"`，details 含 `experiment_id / proposals_count / replays_count / budget_used`。

### Requirement 2: 数据源与回放样本筛选（Replay Cohort）

**User Story:** 作为演化器设计者，我需要演化候选基于真实生产分布，且筛选规则透明可复盘，否则 shadow eval 的结论无法外推到 live 流量。

#### Acceptance Criteria

1. THE 演化器 SHALL 从下列既有 collection 读数据，**只读**：`agent_run_logs / agent_outcome_metrics / agent_events / conversation_messages / agent_decision_reviews / agent_send_outbox / contacts`。
2. WHEN 演化器为阈值候选筛选 cohort，THE 系统 SHALL 拉取窗口 `now - eval_window_hours`（默认 72，env `EVOLUTION_EVAL_WINDOW_HOURS`）内 `lifecycle="completed"` 的 run，过滤 `gateway_status` 不在 `legacy_mode_unchecked / blocked_by_required_field / tool_loop_timeout / mcp_error` 集合内的；样本数 < `min_replays_per_proposal`（默认 30，env `EVOLUTION_MIN_REPLAYS`）时 SHALL 跳过该候选并写 `proposal_status="rejected_below_threshold"`。
3. WHEN 演化器为 prompt 候选筛选 cohort，THE 系统 SHALL 拉取同窗口内 `finalReviewStatus ∈ {revision_failed, blocked_unverified_product_claim, held_by_ai_policy, blocked_by_safety_guard, ai_waiting_for_more_context}` 的 failure cohort；同样有 `min_replays_per_proposal` 下限。
4. THE cohort SHALL 在 experiment 信封里以 `cohort_run_ids: [ObjectId]` 数组记录原始 run id 列表，**不拷贝消息内容**，回放时按 id 反查。
5. IF 一条历史 run 的 `conversation_messages` 已经被超出 `MESSAGE_RETENTION_DAYS`（默认 90）清理，THEN 该 run SHALL 被移出 cohort，`shadow_replays.status="failed"`，`failure_reason="source_message_unavailable"`；不参与显著性计算。
6. THE replay cohort 选择 SHALL **按 contact_wxid 去重**，避免某高频对话 contact 的 run 占满 cohort 而扭曲分布；去重规则：同 contact 最多保留最近 N=3 条 run。
7. THE cohort 筛选 SHALL **不跨 workspace_id / account_id**——本期演化器只针对当前 default workspace + default account，与生产配置一致；多账号场景留待后续。

### Requirement 3: 阈值候选生成（Threshold Adaptation）

**User Story:** 作为运营策略维护者，我希望演化器在 outcomes 显示某 gate 命中率长期偏离合理区间时，自动给出"调到多少"的提案与预估改善幅度，不要让我盯指标手算。

#### Acceptance Criteria

1. WHEN 演化器进入 threshold 阶段，THE 系统 SHALL 对 6 个 gate（FactRisk / PressureRisk / HumanLikeScore / EmotionalValue / ProductAccuracyScore / PlannerBlockRate）逐一计算"过去窗口内命中率"：
   - 5 闸：命中率 = `count(被该闸 block 或 rewrite) / count(进入 review 的 run)`；
   - PlannerBlockRate：命中率 = `count(strategic_planner_*_backoff 事件) / count(strategic_planner_emit + strategic_planner_*_backoff)`。
2. THE 演化器 SHALL 与"合理区间常量表"对比：
   | Gate                  | 命中率区间下限 | 命中率区间上限 | 调整步长 |
   |-----------------------|---------------|---------------|---------|
   | FactRisk              | 0.05          | 0.20          | ±1      |
   | PressureRisk          | 0.05          | 0.20          | ±1      |
   | HumanLikeScore        | 0.10          | 0.35          | ±1      |
   | EmotionalValue        | 0.10          | 0.35          | ±1      |
   | ProductAccuracyScore  | 0.05          | 0.25          | ±1      |
   | PlannerBlockRate      | 0.10          | 0.40          | ±0.05   |

   命中率 < 区间下限 → 候选"调严"（threshold +step）；> 区间上限 → 候选"调松"（threshold -step）。
3. THE 候选 proposal 文档 SHALL 含 `proposal_kind="threshold" / gate_key / current_value / proposed_value / hit_rate_observed / hit_rate_target_band / cohort_run_ids / created_at / status="pending_eval"`。
4. THE 系统 SHALL 一次 tick 最多产 4 条 threshold proposal——若 6 个 gate 都偏离，按"距离目标区间最远的优先"排序取前 4，避免单 tick 改动过多。
5. IF 同一 gate 在 `threshold_overrides` 已存在 `released_at >= now - threshold_release_cooldown_hours`（默认 24）的最新已发布提案，THEN 演化器 SHALL **跳过**该 gate 本轮提案（避免短时间反复调阈值），写 `proposal_status="rejected_below_threshold"`，`failure_reason="cooldown_active"`。
6. THE 阈值 proposal 的 `proposed_value` SHALL 落在硬上下限内：5 闸 [1, 10]，PlannerBlockRate [0.05, 0.95]——超出 SHALL 直接 clamp 并在 `cohort_notes` 注明。
7. THE threshold proposal SHALL 不要求 LLM 调用，纯统计计算——从而 threshold 阶段独立于 EvolutionBudget 的 LLM 部分（仅占用 calls=0 / token=0），即使 prompt 阶段 budget 超额，threshold 阶段仍能完整跑完。

### Requirement 4: Prompt 候选生成（Prompt Evolution via Critic LLM）

**User Story:** 作为运营策略维护者，我希望演化器把"被反复 block 的 run 类型"识别出来，请 Critic LLM 提案 prompt diff，且我能在 UI 上肉眼对比新旧 prompt 段落、shadow eval 数据、Critic 给的改动理由后再发布。

#### Acceptance Criteria

1. WHEN 演化器进入 prompt 阶段，THE 系统 SHALL 把 failure cohort 按 `finalReviewStatus` 分桶，每桶取 `cohort_sample_per_bucket`（默认 10）条作为 Critic LLM 输入；输入结构 SHALL 包含：原 inbound text + decision summary + review.risks + review.holdReason + 现行 prompt 全文（去掉绑定字段）。
2. THE Critic LLM 调用 SHALL 走 `agent::generate_agent_json`（**不**绕过现有 LLM 调用入口），prompt 模板 key 为 `evolution_critic_v1`（本期新增，由演化器随主程序首次启动时 `ensure_evolution_prompt_pack` 写入）；调用 token / calls 计入 EvolutionBudget。
3. THE Critic SHALL 输出严格 JSON：
   ```json
   {
     "diffs": [
       {
         "promptTemplateKey": "reply_agent_main",
         "section": "soul" | "system_contract" | "policy" | "operator_instruction",
         "diffSummary": "...",
         "diffSnippet": "...新文案片段...",
         "expectedImprovementOn": ["held_by_ai_policy", "blocked_unverified_product_claim"],
         "reasoning": "...",
         "riskNote": "..."
       }
     ]
   }
   ```
   Rust SHALL 校验：`promptTemplateKey ∈ system_taxonomies("prompt_template_keys")`、`section ∈ {soul, system_contract, policy, operator_instruction}`、`diffSnippet` 长度 ≤ 4000 chars，违反 SHALL 整批 drop 并写 `proposal_status="rejected_below_threshold"`，`failure_reason="critic_schema_invalid"`。
4. THE 候选 proposal SHALL 含 `proposal_kind="prompt_diff" / prompt_template_key / section / current_version / proposed_version_label / diff_snippet / critic_reasoning / cohort_run_ids / status="pending_eval"`；`proposed_version_label` 用 `vN+1` 形态（N 为现行版本号）。
5. THE 系统 SHALL 一次 tick 最多产 4 条 prompt proposal——若 Critic 给更多，按 `expectedImprovementOn` 与 cohort 主要失败桶重合度排序取前 4。
6. THE prompt proposal SHALL **不**直接覆盖 `prompt_templates`——仅写 `prompt_proposals` 集合，等 release 后才生成新版本。
7. IF Critic LLM 单次调用失败（超时 / json_error / 预算超额），THEN 演化器 SHALL 跳过本轮 prompt 阶段（不重试），写 `experiment_status` 仍推进到 `awaiting_admin`（threshold 阶段产出可独立交付）。

### Requirement 5: Shadow Replay 评测

**User Story:** 作为运营策略维护者，我希望任何 threshold / prompt 提案在被发布前都有一份"在 N 条历史 run 上的预估对比报告"，否则我没办法判断"调严 FactRisk 1 档"到底会让多少历史 run 改判。

#### Acceptance Criteria

1. THE 演化器 SHALL 为每条 `proposal_status="pending_eval"` 的候选启动 shadow eval：取该候选的 `cohort_run_ids` 中每条 run，写一条 `shadow_replays` 文档（`status="pending"`），随后并行（不超过 `EVOLUTION_REPLAY_CONCURRENCY`，默认 4）调用"模拟 gateway"重跑。
2. THE "模拟 gateway" SHALL 由 `src/evolution/replay.rs::run_shadow_replay` 实现，行为：
   - 反查原 run 的 inbound text + decision_phase 之前的 context（contact + memoryCard 快照 + verified knowledge slice 列表）；
   - 用候选阈值（threshold 类）或候选 prompt 文本（prompt 类）调一次 Reply Agent → review；
   - **不**调 revision 路径；**不**写 outbox；**不**调 MCP；
   - 输出 `shadow_replays` 字段：`new_finalReviewStatus / new_review_risks / new_token_cost / new_5gate_hit_array / new_self_critique_addressed / similarity_to_original_text`（相似度可置 0，本期不强求 embedding）。
3. WHEN 单条 shadow_replay 完成，THE `status` SHALL 从 `pending` 推进为 `completed`；失败（LLM 超时 / json_error / 预算超额）SHALL 推进为 `failed` 并写 `failure_reason`。
4. THE 演化器 SHALL 在所有 cohort run 完成后聚合：
   - `replays_completed / replays_failed / replays_total`；
   - threshold 类：`hit_rate_delta = new_hit_rate - original_hit_rate`、`send_success_rate_delta`；
   - prompt 类：`5gate_hit_delta_per_gate / token_cost_delta / self_critique_addressed_delta / status_distribution_delta`。
5. WHEN 显著性测试通过（threshold：`send_success_rate_delta >= EVOLUTION_MIN_SEND_SUCCESS_DELTA` 默认 +0.05；prompt：`self_critique_addressed_delta >= EVOLUTION_MIN_SELF_CRITIQUE_DELTA` 默认 +0.10 且 `5gate_hit_delta_per_gate` 任意一项 ≤ +0.10），THE proposal SHALL 推进到 `eligible_for_release`；否则 `rejected_below_threshold`。
6. WHEN 单次 shadow_replay 触发的 LLM 调用失败次数累计 ≥ `EVOLUTION_REPLAY_MAX_FAIL_RATE`（默认 0.30）的 cohort 比例，THE 当前 proposal SHALL 直接判 `rejected_below_threshold`，`failure_reason="replay_quality_below_threshold"`，避免基于不可靠样本下结论。
7. THE shadow_replay 不消耗生产 RunBudget，但每条 replay 的 LLM token 计入演化 EvolutionBudget；超过预算时尚未启动的 replay SHALL 跳过并写 `failure_reason="evolution_budget_exceeded"`。

### Requirement 6: Threshold Override 与 Prompt 版本生效

**User Story:** 作为运营策略维护者，我希望被发布的提案在生效后立即被生产链路读取，且老配置仍保留可一键回滚——不需要重启进程、不需要改代码。

#### Acceptance Criteria

1. THE 系统 SHALL 新增 collection `threshold_overrides` 含字段 `{ _id, workspace_id, account_id, gate_key, value, released_proposal_id, released_at, rolled_back_at?, released_by? }`；建索引 `(workspace_id, account_id, gate_key, released_at desc)`。
2. WHEN gateway / Planner 在 run 时读取 5 闸阈值或 PlannerBlockRate，THE 读取顺序 SHALL 为：`threshold_overrides`（取 `rolled_back_at == null` 的最新条目）→ `runtime_parameters`（来自 domain_configs）→ 代码默认值。读路径 SHALL 在 `src/agent/runtime.rs` 中集中实现一个 `resolve_thresholds(state, contact)`，所有读 5 闸的位置 SHALL 通过它取值（避免散点遗漏）。
3. WHEN admin 在前端"演化中心"对一条 `proposal_kind="threshold"` 的 `eligible_for_release` 候选点确认，THE 系统 SHALL：
   - insert 一条 `threshold_overrides` 文档，`released_proposal_id=候选 _id`、`released_at=now`、`released_by=admin_session_user`；
   - update 候选 `proposal_status="released"`；
   - 写 `agent_events kind="evolution_threshold_released"`。
4. WHEN admin 对一条 `proposal_kind="prompt_diff"` 的 `eligible_for_release` 候选点确认，THE 系统 SHALL：
   - 读现行 `prompt_templates({key=候选.prompt_template_key, current_version=true})`；
   - 在 `prompt_templates` 写新版本（同 key、新 version、把候选 diff 应用到对应 section）；
   - update 现行版本 `current_version=false`、新版本 `current_version=true`；
   - update 候选 `proposal_status="released"`；
   - 写 `agent_events kind="evolution_prompt_released"`。
5. THE prompt 版本生效 SHALL 在 LRU prompt cache 自动失效——`prompt_templates` 写入时 SHALL bump 一个全局 `prompt_pack_version`，`generate_agent_json` 读取时按 `(template_key, prompt_pack_version)` 复合 cache key，避免老 cache 命中老 prompt。
6. WHEN admin 触发回滚（点 `eligible_for_release` 候选下方的 "回滚" 或对已 released 候选的"回滚"按钮），THE 系统 SHALL：
   - threshold 类：把 `threshold_overrides` 的 `rolled_back_at=now`，update proposal `proposal_status="rolled_back"`；
   - prompt 类：把 `prompt_templates` 的 current_version 指针指回上一版本（由 release 时记录的 `previous_version` 字段获得），update proposal `proposal_status="rolled_back"`；
   - 写 `agent_events kind="evolution_rollback_completed"`。
7. THE 发布与回滚 SHALL 不影响正在进行的 run——读路径每个 run 入口读一次最新值，run 中途不重读；run 中已读到的阈值 / prompt SHALL 在该 run 全程一致。

### Requirement 7: 前端"演化中心"Tab（admin 一键发布）

**User Story:** 作为运营策略维护者，我希望在 admin SPA 里有一个独立 Tab 看演化器最近的 experiment、每条 proposal 的 shadow eval 报告、以及一键发布 / 回滚按钮，不要让我去 Mongo 手 query。

#### Acceptance Criteria

1. THE 前端 SHALL 在 `App.tsx` 现有 channel/tab 状态机里新增一个 Tab `EvolutionCenterTab`，与 `AutonomyOutcomesTab` 同级，文案 "演化中心"。
2. THE Tab SHALL 调用以下后端只读路由（本期新增到 `src/routes/evolution.rs`）：
   - `GET /api/evolution/experiments?limit=20` —— 返回最近 N 个 experiment 信封 + 每个含 `proposals: ProposalSummary[]`；
   - `GET /api/evolution/proposals/{proposal_id}` —— 返回单条 proposal 详情，含 cohort_run_ids、shadow_replays 聚合（不含原文）、Critic reasoning（prompt 类）、当前生效值/版本（用于 diff 对照）；
   - `POST /api/evolution/proposals/{proposal_id}/release` —— admin 一键发布；
   - `POST /api/evolution/proposals/{proposal_id}/rollback` —— admin 一键回滚。
3. THE 前端 SHALL 在 proposal 列表里展示：`gate_key / promptTemplateKey + section / status / shadow_eval 摘要（replays_completed / hit_rate_delta or self_critique_addressed_delta） / created_at`；prompt 类 SHALL 提供"展开 diff"按钮显示 `current_section_text` vs `proposed_section_text` 双栏。
4. THE 前端 SHALL 仅在 `proposal_status="eligible_for_release"` 时启用"发布"按钮、在 `proposal_status="released"` 时启用"回滚"按钮；其他状态按钮置灰并 hover 提示状态名。
5. THE 前端 SHALL 在发布前 modal 二次确认，列出 R6.3 / R6.4 的副作用（"该操作 SHALL 立即生效到所有未来 run，不影响进行中的 run"），需 admin 在文本框输入 "RELEASE" 确认（防止误点）。
6. THE 前端文案 SHALL 不出现"接管 / 人工接管 / handoff / takeover" 等字眼；推荐用"AI 自我演化提案 / 演化中心 / 一键发布 / 一键回滚"。
7. THE 前端 SHALL 有最低限度的单测（vitest）：(a) 发布按钮只在 `eligible_for_release` 启用；(b) 二次确认 modal 在确认串错时不发请求；(c) prompt diff 展开渲染两个 section 文本不混淆。

### Requirement 8: 监控、事件留痕、CI Lint

**User Story:** 作为产品安全责任人，我希望演化器的每一步都有事件留痕，并且 CI 阻断"自我演化绕过既有红线"的代码改动。

#### Acceptance Criteria

1. THE 系统 SHALL 在 `agent_events` 写入以下新 kind：
   - `evolution_tick_started / evolution_tick_completed / evolution_tick_failed`
   - `evolution_threshold_proposal_created / evolution_prompt_proposal_created`
   - `evolution_shadow_replay_started / evolution_shadow_replay_completed / evolution_shadow_replay_failed`
   - `evolution_threshold_released / evolution_prompt_released / evolution_rollback_completed`
   - `evolution_budget_exceeded / evolution_proposal_rejected`
2. THE `AutonomyOutcomesTab.planner` 子段 SHALL **不**变（M3 落点保持），但 `EvolutionCenterTab` SHALL 在顶部聚合卡片展示：最近 7 天 experiments 数 / proposals 数 / released 数 / rolled_back 数 / 显著性通过率（released_or_eligible / total_proposals）。
3. THE 后端 SHALL 在 `src/routes/evolution.rs` 路由处理函数顶部加注释 anchor：`// FORBIDDEN: enqueue agent_send_outbox / mcp call`，方便人审。
4. THE CI lint `scripts/check-no-human-takeover.sh` SHALL 把 `src/evolution/` 加入扫描目录（与 `src/agent/ src/routes/ frontend/src/` 并列），并保证既有 forbidden words 表无变化。
5. THE 测试 SHALL 覆盖：
   - 单测：`evolutionary_worker` 一次 tick 在 mock state 下产 ≥ 1 条 threshold proposal；阈值候选超 cooldown 跳过；prompt critic schema invalid 整批 drop；
   - 集成测试（testcontainers）：阈值发布后 gateway 读到新值（同一 contact 下一个 run 的 5 闸阈值变化）；rollback 后 gateway 读回老值；prompt 发布后 LRU cache 失效。
6. THE `scripts/check-baseline.{sh,ps1}` SHALL 在 PR 合并前 pass：`cargo test --lib` ≥ 既有 313 + 本期新增、4 PBT 累计 ≥ 37、0 failed。
7. THE 任何对 `experiments / proposals / shadow_replays / threshold_overrides / prompt_templates` 的写路径 SHALL 走 `src/evolution/` 内的 helper（不允许散点直接 `collection.insert_one(...)`），便于审计与回滚。

### Requirement 9: 安全边界与 Sunset

**User Story:** 作为产品安全责任人，我希望本期任何"AI 自己改 AI"的能力都默认关停 + 多层保险，且未来新功能不会偷偷扩大这个能力面。

#### Acceptance Criteria

1. THE 演化器主开关 SHALL 是 env `EVOLUTION_ENABLED`（默认 `false` 在 `.env.example`、`true` 在生产配置由运维显式打开）。关停时 spawn 不发生，前端"演化中心"Tab SHALL 显示"演化器未启用"占位（仅 admin 可见）。
2. THE 发布 admin 鉴权 SHALL 不放宽——发布 / 回滚路由 SHALL 复用现有 admin auth middleware（与 `/api/contacts/...` 同级）；本期 SHALL NOT 引入"演化专用 token"或"无 auth 的内部接口"。
3. THE Critic LLM prompt 模板 SHALL 在 `prompts.rs` 内被 `ensure_evolution_prompt_pack_v1` 显式 seed，**不**进入演化器自身的 prompt evolution 循环——即"演化 Critic 的 prompt"是常量，需修改时由人审走代码 PR。
4. THE 阈值 proposal 的 `proposed_value` 与 prompt proposal 的 `diff_snippet` SHALL 在写入前过 `scripts/check-no-human-takeover.sh` 同款关键字 lint（运行时拦截而非 CI），出现禁词整批 drop 并写 `proposal_status="rejected_below_threshold"`，`failure_reason="forbidden_literal"`。
5. THE 演化器 SHALL **不**主动给 admin 发邮件 / IM / 推送——只在 `EvolutionCenterTab` 内部展示候选；admin 主动来看才能发现，避免被噪音淹没。
6. THE 本期 SHALL **不**引入"自动发布"开关（即不允许 admin 设置 "shadow eval 通过自动 release"）——release 永远需要人审 + 二次确认串。该约束在未来若放宽 SHALL 由独立 spec 提议（M5+），不在 M4 增量。
7. THE 演化器 SHALL 在每条 release 后 24h 内自动触发一次"对比窗口评测"——把发布后 24h 的 outcomes 数据与发布前 24h 同段 outcomes 对比，写一条 `proposal_post_release_review` 文档（含 `actual_send_success_rate_delta / actual_5gate_hit_delta`）。**不自动回滚**，仅作为下一轮 release 时的 admin 参考。

### Requirement 10: 不做的事（明确边界）

THE 系统 SHALL NOT 在 M4 范围内：

1. 自动调"调阈值的阈值"（即 R3.2 的合理区间表 / R5.5 的显著性常量）——这些由代码 PR 修改。
2. 引入跨 workspace / 多 account 的演化共享。
3. 让 Critic LLM 写代码 / 改 schema / 调用 MCP。
4. 给 `agent_run_logs / agent_outcome_metrics` 增字段（M4 只读它们）。
5. 引入 embedding 服务做相似度评测（R5.2 的 similarity 字段允许置 0）。
6. 给前端加路由库（沿用 `App.tsx` Tab 状态机）。
7. 给"自动发布"开关（R9.6）。
8. 让演化器调用 Reply Agent 处理实时入站消息（R1.6）。
9. 在 release 前要求邮件 / IM 通知（R9.5）。
10. 拓展到群运营 / 朋友圈运营（与现有 Phase 1 范围一致）。
