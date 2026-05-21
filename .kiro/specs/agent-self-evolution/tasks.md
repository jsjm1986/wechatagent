# Implementation Plan: 用户运营 Agent 自我演化（agent-self-evolution）

## Overview

本实施计划严格遵循 design.md §2.3 的 5 波（W0 基础设施 → W1 骨架 → W2 候选生成 → W3 Shadow eval + 显著性 → W4 Release + 前端 + 回滚）+ 1 收口顺序。每个任务都是可由代码生成 LLM 增量执行的具体编码步骤，引用 requirements.md 的 R1–R10 子条款。

实现语言：Rust（后端）+ TypeScript/React（前端 Tab）。性质测试基于 `proptest`（沿用 `tests/state_transition_pbt.rs` 等惯例）。

强制顺序（不可乱序）：W0 必须先于 W1（缺 collection / config 后续 worker 起不来）；W1 必须先于 W2（缺主循环 + budget 候选写不进 experiment 信封）；W2 必须先于 W3（无候选无可 replay）；W3 必须先于 W4（无显著性结果不允许 release）。任何"先做前端再做后端"的顺序倒置 SHALL 在 PR review 中阻断。

## Tasks

- [ ] 1. 基础设施波（W0）：collection、索引、配置、CI lint
  - [ ] 1.1 在 `src/db/mod.rs` 新增 4 个 collection accessor
    - 新增 `experiments / proposals / shadow_replays / threshold_overrides` 四个 `pub fn xxx(&self) -> Collection<T>` 入口，类型分别绑定到 `Experiment / Proposal / ShadowReplay / ThresholdOverride`（先在 `src/models.rs` 写 struct 占位，字段最终值在 W1/W2 落定）
    - _Requirements: 1.1, 6.1, 8.1_

  - [ ] 1.2 在 `src/db/indexes.rs` 创建 collection 索引
    - `experiments`：`(workspace_id, account_id, started_at desc)` + 唯一 `(experiment_id)`
    - `proposals`：`(workspace_id, account_id, status, created_at desc)` + `(experiment_id)`
    - `shadow_replays`：`(proposal_id)` + `(workspace_id, account_id, started_at desc)`
    - `threshold_overrides`：`(workspace_id, account_id, gate_key, released_at desc)`
    - `prompt_templates`：补建 `(key, current_version)` + 唯一 `(key, version)`
    - _Requirements: 1.1, 6.1, 6.2_

  - [ ] 1.3 在 `src/config.rs` 扩展 `AppConfig`，新增 14 条演化器字段
    - `evolution_enabled: bool`（默认 false）
    - `evolution_tick_seconds: u64`（默认 21600）
    - `evolution_run_token_budget: i64`（默认 60000）
    - `evolution_run_max_llm_calls: i32`（默认 30）
    - `evolution_eval_window_hours: u32`（默认 72）
    - `evolution_min_replays: usize`（默认 30）
    - `evolution_min_send_success_delta: f64`（默认 0.05）
    - `evolution_min_self_critique_delta: f64`（默认 0.10）
    - `evolution_max_5gate_hit_increase: f64`（默认 0.10）
    - `evolution_replay_concurrency: usize`（默认 4）
    - `evolution_replay_max_fail_rate: f64`（默认 0.30）
    - `evolution_threshold_release_cooldown_hours: u32`（默认 24）
    - `evolution_cohort_per_contact_cap: usize`（默认 3）
    - `evolution_cohort_sample_per_failure_bucket: usize`（默认 10）
    - `.env.example` 同步追加 14 行；`tests/common/mod.rs::test_config()` 同步默认值
    - _Requirements: 1.2, 1.4, 2.2, 2.6, 3.1, 4.1, 5.5_

  - [ ] 1.4 新建 CI lint 脚本 `scripts/check-evolution-isolation.sh` + `.ps1`
    - grep `src/evolution/` 是否引用以下字符串中的任意一条：`crate::agent::gateway`、`crate::agent::outbox`、`crate::mcp::`、`agent_send_outbox.insert`、`mcp_client.send`、`run_user_operation_gateway`；命中即 `exit 1`
    - 在 `scripts/check-no-human-takeover.sh` 的 `SCAN_DIRS` 数组里追加 `src/evolution/`
    - 在 `README.md` 引用方式段落补一行运行说明
    - _Requirements: 1.6, 8.4, 9.4_

  - [ ] 1.5 一次性迁移：`prompt_templates` 升级为多版本形态
    - 在 `src/db/migrations.rs` 注册 `2026_05_M4_001_prompt_template_versioned`：扫所有 `prompt_templates`，缺 `version` 字段者填 `version="v_legacy"`、`current_version=true`、`previous_version=null`、`seeded_by="legacy_migration"`
    - 同 migration_id 二次启动 SHALL skip，不报错
    - 修改 `prompts.rs::ensure_prompt_pack_v2` 写入逻辑：从 upsert by `(key)` 改为 upsert by `(key, version)`，写入时若该 (key, version) 已存在则只 update sections（保留 current_version 状态）；首次 seed 时如果同 key 不存在 v2，写新条 + 把 v_legacy 的 current_version 设为 false
    - _Requirements: 6.4, 6.5_

  - [ ] 1.6 为 W0 新增 collection accessor 写最小 smoke 单元测试
    - 测试启动期 `ensure_indexes` 返回 OK
    - 在内存中 mock 一条 `Experiment / Proposal / ShadowReplay / ThresholdOverride`，能往返 BSON 序列化
    - 验证 prompt_templates 迁移幂等：跑两次只 modify 一次
    - _Requirements: 1.1, 6.1_

- [ ] 2. 骨架波（W1）：worker 主循环 + EvolutionBudget + experiment 信封 + cohort 选择
  - [ ] 2.1 新建 `src/evolution/mod.rs`，实现 `pub async fn run_evolutionary_worker(state: AppState)`
    - 顶部写 anchor 注释：`//! FORBIDDEN dependencies: gateway / outbox / mcp / tasks / webhooks`
    - `evolution_enabled=false` 时打日志后直接 return
    - 循环以 `evolution_tick_seconds` 为间隔 `tokio::time::interval` 触发，每 tick 调 `run_one_tick(&state)`
    - 整个 `run_one_tick` 包在 try/catch 等价（`if let Err(err) = ...`）；任何错都 SHALL `agent_events kind="evolution_tick_failed"` + 继续下 tick
    - 在 `src/main.rs` `prompts::ensure_prompt_pack_v2` 之后、`tasks::worker_loop` spawn 之前 `tokio::spawn(run_evolutionary_worker(state.clone()))`
    - _Requirements: 1.1, 1.2, 1.5_

  - [ ] 2.2 新建 `src/evolution/budget.rs`，实现 `EvolutionBudget`
    - 字段：`token_limit / token_used / call_limit / call_used`
    - API：`from_config(&AppConfig) -> Self` / `check_or_fail()` / `record_call(tokens, calls)` / `exhausted() -> bool`
    - 与 `RunBudget` 结构相似但**不**共享类型（运行期完全隔离）
    - `EvolutionError::BudgetExceeded` 定义在 `src/evolution/error.rs`
    - _Requirements: 1.4, 4.7, 5.7_

  - [ ] 2.3 实现 `experiment` 信封写入与状态推进 helper
    - `src/evolution/envelope.rs::insert_experiment_envelope(state, &exp_id) -> AppResult<()>`：`insert_one` `experiments` 一条 `status="collecting"` 信封，含 `experiment_id / workspace_id / account_id / started_at / window_hours / cohort_*=[] / budget_used_*=0`
    - `update_experiment_status(state, &exp_id, "evaluating" | "awaiting_admin" | "released" | "aborted")`：用 `update_one({experiment_id})` + `$set: { status, updated_at, finished_at? }`，禁止再次 insert
    - 单测：同 experiment_id 二次 insert SHALL DuplicateKey；non-existent experiment_id 的 update SHALL 返回 `matched_count=0`（调用方 SHALL 写 `agent_events kind="evolution_envelope_missing"`）
    - _Requirements: 1.3_

  - [ ] 2.4 新建 `src/evolution/cohort.rs`，实现 cohort 选择
    - `select_cohorts(state) -> AppResult<Cohorts>`：
      - 拉 `agent_run_logs` 窗口内 `lifecycle="completed"` 且 `gateway_status` 不在 `[legacy_mode_unchecked, blocked_by_required_field, tool_loop_timeout, mcp_error]` 集合内的所有 run
      - threshold cohort = success+failure 混合；prompt cohort = `finalReviewStatus ∈ failure 子集` only
      - 按 `contact_wxid` 去重，每 contact 最多 N=3 条（`evolution_cohort_per_contact_cap`）
      - 各自 ≥ `evolution_min_replays` 才返回，否则该路径 cohort 为空
    - `Cohorts { threshold: Vec<ObjectId>, prompt: Vec<ObjectId> }`
    - 单测：窗口外 run 不进 cohort、同 contact 超 cap 被去重、conversation_messages 缺失时该 run 在后续 replay 阶段标 failed（cohort 阶段先收进来）
    - _Requirements: 2.1, 2.2, 2.3, 2.5, 2.6, 2.7_

  - [ ] 2.5 主循环骨架接通：tick 内调用 envelope + cohort，写最小事件
    - `run_one_tick` 顺序：insert envelope → select cohorts → update_experiment_cohort_ids → 当前先不调 W2/W3，直接 update status="awaiting_admin" + 写 `agent_events kind="evolution_tick_completed"`（含 budget_used=0、proposals_count=0）
    - 验证空跑也能写完整 envelope
    - _Requirements: 1.3, 1.7, 2.4_

  - [ ] 2.6 为 W1 写 lib 单测（≥ 6 例）
    - `evolution_disabled` 时 worker 不 spawn（main.rs 路径覆盖）
    - cohort 窗口外 run 不进 cohort
    - cohort 同 contact 超 cap 被去重
    - cohort 数 < min_replays 返回空
    - envelope 二次 insert 同 experiment_id DuplicateKey
    - tick 全空跑也写 `evolution_tick_completed`
    - _Requirements: 1.5, 1.7, 2.2, 2.5, 2.6_

- [ ] 3. 候选生成波（W2）：threshold 候选 + Critic LLM prompt 候选
  - [ ] 3.1 新建 `src/evolution/threshold.rs`，实现纯统计阈值候选
    - `pub async fn generate(state, &cohorts) -> AppResult<Vec<Proposal>>`
    - 6 个 gate 并行算命中率（按 §3.1 design.md 公式）
    - 与 `THRESHOLD_REASONABLE_BANDS` 常量表（design.md R3.2）对比，命中率 < 下限 → +step、> 上限 → -step
    - 候选 `proposed_value` clamp 到硬上下限（5 闸 [1,10]、PlannerBlockRate [0.05, 0.95]），clamp 时在 `cohort_notes` 注 `clamped_to_<value>`
    - cooldown 检查：若同 gate 在 `threshold_overrides` 有 `released_at >= now - cooldown_hours` 且 `rolled_back_at` 为空，跳过该 gate 并写 `proposal_status="rejected_below_threshold"` `failure_reason="cooldown_active"`
    - 单 tick 最多 4 条 proposal，按"距离目标区间最远"优先；若超过 4 条仍要 insert 全部 proposal 文档但只有最远 4 条 status=`pending_eval`，其余 status=`rejected_below_threshold` `failure_reason="exceeded_per_tick_quota"`
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7_

  - [ ] 3.2 在 `src/prompts.rs` 添加 `ensure_evolution_prompt_pack_v1`
    - seed 一条 `prompt_templates(key="evolution_critic_v1")`，sections 含：soul（"你是一个专门审视 Reply Agent prompt 的 critic..."）、system_contract（输出 strict JSON schema 描述）、policy（必须遵循的禁词 / 不得绕 5 闸 / 不得引入"人工接管"等约束）、operator_instruction（接受 cohort 失败摘要 + 现行 prompt 全文 → 输出 diff[]）
    - 在 `main.rs` `ensure_prompt_pack_v2` 之后调用 `ensure_evolution_prompt_pack_v1`
    - 该 prompt SHALL 不进入演化器自身的 prompt evolution 循环（`PROMPT_EVOLUTION_FORBIDDEN_KEYS = ["evolution_critic_v1"]`）；prompt_critic.rs 在产候选时若 `proposed_template_key` 命中此集合 SHALL 整批 drop 并 `failure_reason="self_referential_critic_prompt"`
    - _Requirements: 4.2, 9.3_

  - [ ] 3.3 新建 `src/evolution/prompt_critic.rs`，实现 Critic LLM 调用
    - `pub async fn generate(state, cohorts, budget) -> Result<Vec<Proposal>, EvolutionError>`
    - 按 failure cohort `finalReviewStatus` 分桶，每桶取 `cohort_sample_per_failure_bucket`（默认 10）作为 LLM 输入
    - 调用 `agent::generate_agent_json("evolution_critic_v1", prompt, budget)`（**不**绕 LLM 调用入口）；token / calls 计入 EvolutionBudget
    - JSON schema 用 `serde_json::from_value` 反序列化为 `CriticOutput { diffs: Vec<CriticDiff> }`，字段长度上限 4000；schema invalid 整批 drop（`failure_reason="critic_schema_invalid"`）
    - 单 tick 最多 4 条 prompt proposal
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.7_

  - [ ] 3.4 新建 `src/evolution/lint.rs`，运行时禁词扫描
    - `passes_forbidden_words(snippet: &str) -> bool`：复用 `scripts/check-no-human-takeover.sh` 同款正则（`(human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工)`）
    - prompt_critic.rs 在产候选前调用 `lint::passes_forbidden_words(&d.diff_snippet)` + `&d.diff_summary`，命中 SHALL 整批 drop 并 `failure_reason="forbidden_literal"`
    - threshold.rs 不需要这步（数值类无文本）
    - _Requirements: 9.4_

  - [ ] 3.5 把 W2 接入主循环，update experiment.budget_used
    - `run_one_tick` 顺序更新：insert envelope → cohort → threshold.generate → prompt_critic.generate（catch BudgetExceeded → 写 evolution_budget_exceeded + 跳过）→ write_proposals → update experiment.budget_used / status="evaluating"
    - 单测：budget exhausted 时 prompt 阶段被跳过、threshold 阶段不受影响
    - _Requirements: 1.4, 4.7_

  - [ ] 3.6 为 W2 写 lib 单测（≥ 8 例）
    - 命中率 < 下限 → 候选 +step
    - 命中率 > 上限 → 候选 -step
    - cooldown 内同 gate 跳过
    - clamp 到硬上限
    - prompt critic schema invalid 整批 drop
    - prompt critic 含禁词 → drop
    - prompt critic budget exhausted 返回空 vec
    - 单 tick > 4 条 prompt 候选时按重合度截断
    - _Requirements: 3.2, 3.5, 3.6, 4.3, 4.5, 4.7, 9.4_

- [ ] 4. Shadow eval + 显著性波（W3）
  - [ ] 4.1 新建 `src/evolution/replay.rs`，实现"模拟 gateway"
    - `pub async fn run_shadow_replay(state, proposal, source_run_id) -> AppResult<()>`：
      - 反查原 `agent_run_logs.find_one({_id: source_run_id})`；不存在 → 写 shadow_replays.failed `failure_reason="source_run_not_found"`
      - 反查 `conversation_messages` 找 `inbound_message_id` 对应文档；retention 已清理 → 写 shadow_replays.failed `failure_reason="source_message_unavailable"`
      - 调"短路 gateway" `run_simulated_gateway`：
        - 使用候选阈值（threshold 类，构造一个临时的 `ResolvedThresholds` 覆盖现行）或候选 prompt 文本（prompt 类，构造 `template_overrides: HashMap<String, Document>`）
        - 内部调 `agent::reply::reply_with_tools_loop / agent::review::review_decision`（pub(crate) 暴露后）
        - **不**调 revision、**不**调 outbox.enqueue、**不**调 mcp::send_message、**不**写 conversation_messages outbound、**不**写 agent_run_logs
      - 写 `shadow_replays` 一条 `status="completed"`，记 `new_finalReviewStatus / new_review_risks / new_token_cost / new_5gate_hit / new_self_critique_addressed / similarity_to_original_text=0.0`
    - 失败路径（LLM 超时 / json_error / 预算超额）写 `status="failed"` + `failure_reason`
    - _Requirements: 5.1, 5.2, 5.3, 5.6, 5.7_

  - [ ] 4.2 把 `src/agent/` 内 helper 暴露面最小化扩 `pub(crate)`
    - 评估 `agent::reply::reply_with_tools_loop` 和 `agent::review::review_decision` 当前可见性
    - 仅这两个 helper 改 `pub(crate)`（不改更多）；其它 `pub(super)` 不动
    - 在 `src/agent/mod.rs` re-export 注释明确：这两个仅供 `crate::evolution::replay` 使用
    - 若改动有破坏性（如 `pub(super)` 改成 `pub(crate)` 后某些 `mod.rs` re-export 消失），同步修复 evolution 路径之外的现有调用
    - _Requirements: 5.2_

  - [ ] 4.3 实现并行 replay 调度
    - `pub async fn eval_all(state, exp_id, budget) -> AppResult<()>`：
      - 加载 `proposals` 中本 experiment 下 `status=pending_eval` 的所有候选
      - 用 `tokio::sync::Semaphore::new(evolution_replay_concurrency)` 限并发
      - 每条 `(proposal, source_run_id)` 起 `tokio::spawn(run_shadow_replay)`
      - budget 触顶后未启动的 replay 跳过 + 写 shadow_replays.failed `failure_reason="evolution_budget_exceeded"`
    - join 所有 handle 后返回
    - _Requirements: 5.1, 5.7_

  - [ ] 4.4 新建 `src/evolution/significance.rs`，纯函数显著性测试
    - `grade_threshold(replays: &[ShadowReplay], cfg: &SignificanceCfg) -> (bool, Document)`：实现 design.md §4.6
    - `grade_prompt(replays: &[ShadowReplay], cfg: &SignificanceCfg) -> (bool, Document)`：检查 self_critique_addressed_delta + 5gate_hit_delta_per_gate（任一项 ≤ +0.10）+ token_cost_delta（不强制，仅观测）
    - completed_replay_count < min_replays 或 fail_rate > max_fail_rate → 直接 reject
    - _Requirements: 5.4, 5.5, 5.6_

  - [ ] 4.5 实现聚合到 proposal
    - `aggregate_and_grade(state, exp_id) -> AppResult<()>`：
      - 加载本 experiment 下所有 `proposals`
      - 对每条 proposal，按 `proposal_kind` 调 `grade_threshold` / `grade_prompt`
      - 把 `eval_replays_completed / eval_replays_failed / eval_metrics / significance_passed / status=eligible_for_release | rejected_below_threshold` update 到 proposal
      - 全部完成后 update experiment.status="awaiting_admin"
    - _Requirements: 5.4, 5.5_

  - [ ] 4.6 把 W3 接入主循环
    - `run_one_tick` 顺序：W2 之后 → `replay::eval_all` → `significance::aggregate_and_grade` → update experiment.status="awaiting_admin"
    - 写 `agent_events kind="evolution_tick_completed"` 含 `proposals_eligible_count / proposals_rejected_count`
    - _Requirements: 1.7, 5.5_

  - [ ] 4.7 为 W3 写 lib 单测（≥ 5 例）
    - 30 条 replay，原 0.6 / 新 0.7 → threshold passed=true
    - replay 成功率 < 70% → reject
    - 5gate_hit_delta 任一 > 0.10 → prompt reject
    - source_message_unavailable 不计入 completed
    - 短路：调 replay 不写 outbox / 不写 conversation_messages outbound（mock state 验 collection size 不变）
    - _Requirements: 5.2, 5.4, 5.5, 5.6_

  - [ ] 4.8 为 `significance.rs` 写 PBT（新文件，不计 baseline 4 PBT）
    - `tests/evolution_significance_pbt.rs`：≥ 6 case
    - 阈值候选 delta 在 [0,1] 区间内永不 panic
    - prompt grade 在 replay vec empty 时永远 reject
    - completed_count < min_replays 永远 reject
    - 任意 5gate_hit_delta_per_gate 含 NaN 永远 reject（防御）
    - _Requirements: 5.4, 5.5_

- [ ] 5. Release + 前端 + 回滚波（W4）
  - [ ] 5.1 实现 `resolve_thresholds` 集中读路径
    - 在 `src/agent/runtime.rs` 新增 `pub async fn resolve_thresholds(state, contact) -> ResolvedThresholds`
    - 读取顺序：threshold_overrides（rolled_back_at=null 的最新） → contact.runtime_parameters → AppConfig 默认
    - 返回 `ResolvedThresholds { fact_risk_block, pressure_risk_block, human_like_score_rewrite, emotional_value_rewrite, product_accuracy_score_block, planner_block_rate_threshold, ... }`
    - 重构所有读 5 闸 / PlannerBlockRate 的位置（`agent/review.rs` / `agent/guards.rs` / `planner/mod.rs`）改为通过 `resolve_thresholds` 取值；散点直读 SHALL 在 PR 阻断
    - _Requirements: 6.2, 6.7_

  - [ ] 5.2 新建 `src/evolution/release.rs`，实现 release_threshold / release_prompt
    - `release_threshold(state, proposal_id, admin) -> AppResult<()>`：
      - 用 mongo session transaction：insert threshold_overrides + update proposals.status="released"
      - 写 `agent_events kind="evolution_threshold_released"`
      - 校验 proposal.status="eligible_for_release"，否则返回 `EvolutionError::InvalidStatus`
    - `release_prompt(state, proposal_id, admin) -> AppResult<()>`：
      - mongo session transaction：load current `prompt_templates({key, current_version=true})` → bump 新 version → 把 current 改 false / 新条 current=true → update proposals.status="released" + 记录 previous_prompt_version
      - commit 后 `state.prompt_pack_version.fetch_add(1, Ordering::SeqCst)`
      - 写 `agent_events kind="evolution_prompt_released"`
    - _Requirements: 6.3, 6.4_

  - [ ] 5.3 实现 `prompt_pack_version` cache 失效机制
    - 在 `src/lib.rs::AppState` 加 `pub prompt_pack_version: Arc<AtomicU64>`，初始值 0
    - 在 `agent::generate_agent_json` 的 LRU cache key 改为 `(template_key, prompt_pack_version.load(Ordering::SeqCst))` 复合 key
    - 在 `prompts::ensure_prompt_pack_v2` / `ensure_evolution_prompt_pack_v1` 末尾各 fetch_add 一次
    - `release.rs::release_prompt` commit 后 fetch_add 一次
    - 单测：bump version 后下次 generate_agent_json 从 Mongo 重读
    - _Requirements: 6.5_

  - [ ] 5.4 实现 rollback_threshold / rollback_prompt
    - `rollback_threshold(state, proposal_id, admin) -> AppResult<()>`：
      - 加载 proposal，校验 status="released"
      - mongo session transaction：update threshold_overrides.rolled_back_at=now + update proposals.status="rolled_back"
      - 写 `agent_events kind="evolution_rollback_completed"` { kind: "threshold" }
    - `rollback_prompt(state, proposal_id, admin) -> AppResult<()>`：
      - 加载 proposal，校验 status="released"，从 proposal.previous_prompt_version 取要回退的版本
      - mongo session transaction：把 prompt_templates 当前 current_version=true 那条置 false + 把 previous_version 那条置 true + update proposals.status="rolled_back"
      - commit 后 `prompt_pack_version.fetch_add(1)`
      - 写 `agent_events kind="evolution_rollback_completed"` { kind: "prompt" }
    - _Requirements: 6.6_

  - [ ] 5.5 新建 `src/routes/evolution.rs`，实现 4 个路由
    - `GET /api/evolution/experiments?limit=20`：返回最近 N 个 experiment 信封 + proposals 摘要
    - `GET /api/evolution/proposals/:id`：返回单条详情含 cohort_run_ids、shadow_replays 聚合（按 proposal_id 分组）、Critic reasoning（prompt 类）、当前生效值/版本（用于 diff 对照——threshold 类查 threshold_overrides + AppConfig；prompt 类查 prompt_templates 同 key 的 current_version）
    - `POST /api/evolution/proposals/:id/release` body `{ confirmation: "RELEASE" }`：校验串完全匹配 → 调 release.rs 的对应函数
    - `POST /api/evolution/proposals/:id/rollback` body `{ confirmation: "ROLLBACK" }`：校验串 → 调 rollback.rs
    - 在 `src/routes/mod.rs` 注册路由，复用现有 admin auth middleware
    - 路由处理函数顶部加注释 anchor `// FORBIDDEN: enqueue agent_send_outbox / mcp call`
    - _Requirements: 7.2, 7.5, 9.2_

  - [ ] 5.6 新建 `src/evolution/post_release.rs`，+24h 对比窗口评测
    - 每次 release 后由 `release.rs` insert 一条 `post_release_reviews` 文档（`scheduled_at = released_at + 24h`，`completed=false`）
    - `evolution::mod::run_one_tick` 在末尾扫一次 `scheduled_at <= now AND completed=false` 的待评测条目
    - 对每条算 `released_at - 24h ~ released_at` 与 `released_at ~ released_at + 24h` 的 outcomes 切片差值，写到 `actual_send_success_rate_delta / actual_5gate_hit_delta`，置 `completed=true`
    - **不自动回滚**，仅写 `agent_events kind="evolution_post_release_review"`
    - 这条 collection 不需要在 W0 提前建索引，本任务内一并 add
    - _Requirements: 9.7_

  - [ ] 5.7 新建 `frontend/src/EvolutionCenterTab.tsx`
    - 三层结构：聚合卡（最近 7 天 experiments / proposals / released / rolled_back / 显著性通过率）→ proposal 列表（status 徽章 + shadow eval 摘要）→ ProposalDetail 展开
    - threshold 类详情：current vs proposed 数值条 + hit_rate_observed
    - prompt 类详情：双栏 diff（current_section_text | proposed_section_text）+ Critic reasoning + expectedImprovementOn 标签
    - shadow eval 报告卡：replays_completed / failed / 显著性 metric 摘要
    - [发布] / [回滚] 按钮按 status 启用/置灰
    - ReleaseModal 要求文本框输入 `RELEASE` 才启用确认；RollbackModal 同款 `ROLLBACK`
    - 文案 SHALL 不出现"接管 / 人工接管 / handoff / takeover"
    - _Requirements: 7.1, 7.3, 7.4, 7.5, 7.6_

  - [ ] 5.8 在 `App.tsx` 顶层 channel/tab 状态机注册 EvolutionCenterTab
    - 与 `AutonomyOutcomesTab` 同级
    - tab 文案"演化中心"
    - `evolution_enabled=false` 时 Tab 仍显示但内容是"演化器未启用"占位（仅 admin 可见）
    - _Requirements: 7.1, 9.1_

  - [ ] 5.9 为 W4 写 lib 集成测试（4 例，testcontainers，`#[ignore]`）
    - `tests/evolution_threshold_e2e.rs`：mock 5 闸命中率分布 → tick 产 ≥ 1 条 threshold proposal → shadow eval → admin release → resolve_thresholds 读到新值
    - `tests/evolution_prompt_e2e.rs`：failure cohort + mock LLM 返回 fixture diff → release → prompt_pack_version bump → 下次 generate_agent_json 从 Mongo 重读
    - `tests/evolution_rollback.rs`：release 后 rollback → resolve_thresholds 读回老值
    - `tests/evolution_isolation.rs`：shadow replay 跑 100 次后 `agent_send_outbox` collection size 不变（确保短路）
    - _Requirements: 5.2, 6.2, 6.5, 6.6_

  - [ ] 5.10 为 W4 写前端 vitest（≥ 4 例）
    - 列表渲染 4 种 status 徽章
    - 发布按钮只在 `eligible_for_release` 启用
    - ReleaseModal 输入串错时不发请求
    - prompt diff 双栏渲染 section text 不混淆
    - _Requirements: 7.7_

- [ ] 6. 收口：监控、CI、文档、baseline
  - [ ] 6.1 在 `agent_events.kind` 枚举校验位置加新 kind 白名单（如有）
    - 若 `src/db/mod.rs` 或 `src/agent/event.rs` 有 `kind` 校验逻辑，把 R8.1 的 11 个新 kind 列入合法集合
    - 未列入则不动（agent_events 当前是 free-form `kind: String`）
    - _Requirements: 8.1_

  - [ ] 6.2 更新 `docs/agent-policy.md`
    - 新增"自我演化"章节，描述：演化器行为、阈值/Prompt 演化、shadow eval、release/rollback、安全边界
    - 注明 `EVOLUTION_ENABLED` 默认 false，运维需显式打开
    - 声明 R9.3 / R10.1 的"自我引用悖论"红线
    - _Requirements: 9.1, 9.3, 10.1_

  - [ ] 6.3 更新 `docs/architecture.md` + `docs/data-and-api.md`
    - architecture.md 在"运行链路"段落追加演化器子图（design.md §2.1）
    - data-and-api.md 列 4 张新 collection schema + 4 个新 API endpoint（GET experiments / GET proposals / POST release / POST rollback）
    - _Requirements: design 整体_

  - [ ] 6.4 更新 `README.md` 与 `.env.example`
    - README 增 "Evolution worker" 段落，指向 docs/agent-policy.md
    - `.env.example` 新增 14 条 EVOLUTION_* env（已在 1.3 加，本任务确认完整）
    - _Requirements: 1.2_

  - [ ] 6.5 跑 baseline gate
    - `cargo test --lib`：≥ 313（M3 末状态）+ 本期新增 ≥ 18 单测 = 331+，0 failed
    - 4 PBT 累计 ≥ 37 不变
    - 新增 `tests/evolution_significance_pbt.rs` ≥ 6 case 计入 PBT 总数但不计入 R11.6 的 4 文件 baseline
    - `scripts/check-baseline.sh` / `scripts/check-no-human-takeover.sh` / 新增 `scripts/check-evolution-isolation.sh` 全 pass
    - 前端 vitest ≥ 12（既有 8 + 本期新增 4）
    - tsc 全程无报错
    - _Requirements: 8.5, 8.6_

  - [ ] 6.6 端到端手工烟雾（`EVOLUTION_ENABLED=true` 临时打开）
    - 启动后等 6h 或临时把 `EVOLUTION_TICK_SECONDS=120` 跑 1 个 tick
    - 在 EvolutionCenterTab 看到 ≥ 1 条 experiment 信封
    - 制造 5 闸命中率偏离的真实流量 → 下一个 tick 产生 threshold proposal
    - 点 release → 立刻验证生产 run 读到新阈值
    - 点 rollback → 立刻验证读回老值
    - 复原 `EVOLUTION_ENABLED=false`，验证已发布的 threshold_overrides 仍生效（不回退）
    - _Requirements: 6.2, 6.6, 9.1_

## Notes

### 执行顺序与依赖关系

W0 → W1 → W2 → W3 → W4 → 收口。同一波内子任务原则上可并行（W0 内 1.1/1.2/1.3 三组可并行），但跨波 SHALL 严格串行。

### 测试基线

- `cargo test --lib`：M3 末态 313 + 本期新增 ≥ 18 = ≥ 331 / 0 failed（合并门）
- 4 PBT 文件累计 ≥ 37 不变（本期新增 PBT 文件不计入 4 文件门，但累计期望 ≥ 43）
- `scripts/check-baseline.sh` 与 `scripts/check-no-human-takeover.sh` SHALL pass
- 新增 `scripts/check-evolution-isolation.sh` SHALL pass

### 数据迁移可重入性

`2026_05_M4_001_prompt_template_versioned` 必须幂等：`migration_id` 已存在 → skip；缺字段者填充；已填充者不动。

### 验证清单

- [ ] 演化器关停时不影响主进程（`EVOLUTION_ENABLED=false` 单测覆盖）
- [ ] `src/evolution/` 不引用 `src/agent/gateway / outbox / src/mcp`（CI lint 覆盖）
- [ ] 所有新事件 kind / 前端文案过禁词 lint
- [ ] release 与正在进行 run 不竞争（resolve_thresholds 在 run 入口取一次）
- [ ] rollback 后 resolve_thresholds 读回老值（集成测试覆盖）
- [ ] prompt_pack_version bump 后 LRU cache 失效（集成测试覆盖）
- [ ] shadow replay 100 次后 outbox / conversation_messages outbound size 不变（集成测试覆盖）
- [ ] admin auth middleware 复用，不引入新 token 路径
- [ ] Critic LLM prompt 自身不进入 prompt evolution 循环（W2 任务 3.2 内置黑名单）

### 回滚预案

- 主开关关停：`EVOLUTION_ENABLED=false` → 演化器不再产候选；已发布的 threshold_overrides / prompt_templates **不回退**（保持已发布的演化结果）。
- 单条回滚：admin 在前端点回滚 → release.rs::rollback_*。
- 全部回滚：`POST /api/evolution/rollback_all`（需 admin 输入 `ROLLBACK_ALL`），把所有 threshold_overrides 一次性 rolled_back_at=now、所有 prompt_templates 回退到 previous_version；写一条强警示 `agent_events kind="evolution_rollback_all"`。本任务在 W4 任务 5.5 内一并实现。
- 代码层 sunset：本期不引入"双轨"，rollback 后行为完全等于 M3 末状态。

### 实现分工建议

- W0 / W1：单人 1–2 天（基础设施 + 骨架）
- W2：单人 2–3 天（候选生成 + Critic prompt seed）
- W3：单人 3–4 天（replay + 显著性，最复杂）
- W4：单人 3–4 天（release/rollback + 前端 Tab + 集成测试）
- 收口：单人 1 天（文档 + baseline）

总计估 10–14 天，单人推进；多人并行可按"后端 W0–W3 / 前端 W4 / 文档收口"分配。

## Task Dependency Graph

```
W0 ──▶ W1 ──▶ W2 ──▶ W3 ──▶ W4 ──▶ 收口
                              ▲
                              │
            (admin 手工触发：release/rollback 路径单独走 W4)
```

W0 子任务内：1.1 / 1.2 / 1.3 / 1.4 / 1.5 可并行；1.6 依赖前述。
W1 子任务：2.1 → 2.2 → 2.3 → 2.4 → 2.5 → 2.6（严格串行）。
W2 子任务：3.1（threshold） / 3.2 + 3.3（prompt critic）可并行；3.4（lint）独立；3.5 + 3.6 串行收口。
W3 子任务：4.1 → 4.2 → 4.3 → 4.4 → 4.5 → 4.6 → 4.7 → 4.8（基本串行，4.4 与 4.5 可并行）。
W4 子任务：5.1（resolve_thresholds 重构）→ 5.2 / 5.3 / 5.4 / 5.5 / 5.6（后端）→ 5.7 / 5.8（前端）→ 5.9 / 5.10（测试）。
