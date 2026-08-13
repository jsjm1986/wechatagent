# 演化器与独立 workers 深读记录（核证日期 2026-08-13）

> 覆盖范围：`src/evolution/` 全部 15 个文件（8,203 行）、`src/planner/mod.rs`（4,301 行）、`src/proactive_outreach.rs`（742 行）、`src/cold_contact_worker.rs`（640 行）、`src/silence_signal_worker.rs`（427 行）、`src/management_worker.rs`（186 行）、`src/account_scheduler.rs`（507 行）、`src/behavior_signals.rs`（580 行）、`src/bin/migrate_only.rs`（41 行），共 23 个文件 14,827 行，全部逐行读完。所有 file:line 均为当场亲验。

---

## 1. 模块地图

```
main.rs（spawn_supervised，src/main.rs:215-289）
 ├── management_command_sweeper（无条件 spawn，main.rs:233-239）→ management_worker.rs
 ├── strategic_planner（仅 strategic_planner_enabled=true 才 spawn，main.rs:263-267）→ planner/mod.rs
 ├── cold_contact_worker（无条件 spawn、函数内检 flag，main.rs:272-274）→ cold_contact_worker.rs
 ├── silence_signal_worker（无条件 spawn、函数内检 flag，main.rs:280-282）→ silence_signal_worker.rs
 └── evolutionary_worker（无条件 spawn、函数内检 EVOLUTION_ENABLED，main.rs:287-289）→ evolution/mod.rs
```

- **evolution/**（演化器，与发送链物理隔离）：
  - `mod.rs` 主循环 + 单 tick 编排（tick→cohort→threshold/prompt 候选→replay→显著性→awaiting_admin→post_release 扫描→auto_release 接点→tick 事件）
  - `error.rs` 独立错误枚举（与 AppError 解耦）；`envelope.rs` experiments 信封；`budget.rs` tick 级 LLM 预算；`runtime_flag.rs` mongo 灰度开关 + 分桶；`cohort.rs` cohort 选择；`threshold.rs` 阈值候选（纯统计）；`prompt_critic.rs` Critic LLM prompt 候选；`lint.rs` 运行时禁词；`replay.rs` shadow 重放调度；`significance.rs` 显著性分级纯函数 + 聚合写回；`revision.rs` 不可变 revision token；`release.rs` release/rollback 事务；`auto_release.rs` 历史自动放行（政策硬闸恒关）；`post_release.rs` +24h 复盘
  - 隔离红线由 `mod.rs` 内嵌 isolation 测试（mod.rs:467-627）+ CI 脚本 `scripts/check-evolution-isolation.sh` 双重锁定：禁 `crate::agent::gateway/outbox`、`crate::mcp`、`crate::tasks`、`crate::webhooks`、`run_user_operation_gateway`、`handle_managed_message`、`handle_follow_up_task`、`agent_send_outbox`（mod.rs:528-538）；允许的 agent 桥仅 `domain_profile / prompt_shadow / run_envelope / runtime` 四个（mod.rs:553-558）；replay.rs 只允许写 `shadow_replays`（mod.rs:585-606）
- **planner/mod.rs**（战略规划器）：六段扫描（silent / commitment / stage_stagnation / calendar / renewal / reactivation），只 emit `follow_up` 任务，发送仍走 task worker→gateway（planner/mod.rs:1-14）
- **proactive_outreach.rs**：主动触达的持久提交边界——确定性任务 + 审计事件 + 当日配额三者单事务提交；planner / cold worker / silence worker 三个调用方
- **cold_contact_worker.rs**：冷链路重激活（`last_outbound_at` 太旧 → emit follow_up，带 peer_case 钩子）
- **silence_signal_worker.rs**：沉默删失探测（outbound 后无回 → 落 `censored=true` 信号，绝不发消息）
- **management_worker.rs**：管理命令执行租约崩溃恢复（stale running → `execution_unknown`，绝不重放副作用）
- **account_scheduler.rs**：多账号调度（scope 枚举供各 worker 用 + persona 池内稳定散列分配）
- **behavior_signals.rs**：T1 行为信号采集底座（4 类信号构造器 + 幂等落库 + 采集健康度计数）；webhooks.rs:1743-1776 为入站侧调用方
- **bin/migrate_only.rs**：独立迁移二进制（不起 HTTP/worker）

---

## 2. 逐文件深读

### 2.1 `src/evolution/mod.rs`（627 行）

**模块头注释**（mod.rs:1-16）：隔离红线声明 + `EVOLUTION_ENABLED` 是硬上限（false=运维硬锁定进函数即 return；true=进常驻 tick，每 tick 由 mongo runtime flag 决定是否真选 cohort）+ W1~W4 波次说明。

**`run_evolutionary_worker(state)`**（mod.rs:54-96）
- 职责：演化器常驻主循环。
- 行为：`!config.evolution_enabled` → log + return（mod.rs:55-60）；`tick_seconds = evolution_tick_seconds.max(60)`（mod.rs:61）；`tokio::time::interval` 循环，每 tick 调 `account_scheduler::list_registered_account_scopes`（mod.rs:69）枚举全部 (workspace, account)，逐 scope 调 `run_one_tick`。
- 错误路径：单 scope 失败 → warn + best-effort 写 `evolution_tick_failed` 事件（mod.rs:75-88，`write_tick_failed_event` 的 Err 被 `let _ =` 吞掉）；scope 枚举失败 → warn 后等下一轮（mod.rs:91-93）。单 scope 失败不影响其它租户。

**`run_one_tick(state, workspace_id, account_id)`**（mod.rs:106-330）——tick 主流程 9 步：
1. `exp_id = "exp_" + ObjectId::new().to_hex()`（全局唯一索引防碰撞，mod.rs:111-113）→ `insert_experiment_envelope`（窗口=`evolution_eval_window_hours`，mod.rs:116-123）。
2. `load_runtime_flag`：读失败按 None 处理（"避免 mongo 抖动让灰度门误开"，mod.rs:125-139）。
3. `select_cohorts_filtered`（mod.rs:142-143）→ update envelope 的 `cohort_threshold_run_ids` / `cohort_prompt_run_ids`（mod.rs:148-167）。
4. `threshold::generate`（纯统计不消预算，mod.rs:169-172）→ `insert_proposals`。
5. `EvolutionBudget::from_config` → `prompt_critic::generate`；`Err(BudgetExceeded)` 被捕获 → 写 `evolution_budget_exceeded` 事件 + 返回空 vec，**不向上传播**（mod.rs:174-203）；其它 Err 上传。
6. 把 `budget.token_used / call_used` 写回 envelope（mod.rs:206-226）。
7. 若存在 status=`pending_eval` 的候选（mod.rs:232-236）：`replay::eval_all`（BudgetExceeded 同样只写事件，mod.rs:238-255）→ `significance::aggregate_and_grade` 返回 `(eligible_count, rejected_after_eval)`（mod.rs:256）。无 pending → `(0,0)`。
8. `update_experiment_status(..., "awaiting_admin")`（W3 后无论有无候选都直达 awaiting_admin，mod.rs:261-263）；envelope 写 `proposals_count / proposals_eligible_count`（mod.rs:266-285）。
9. `post_release::run_due_reviews`（失败 `unwrap_or_else` → warn + 0，mod.rs:289-297）；`auto_release::auto_release_eligible_thresholds`（HC-017 政策硬闸恒关 → 立即 0；保留调用只为未来复用；"rollback 永远由 admin 手工"，mod.rs:299-311）；`write_tick_completed_event`（13 个计数字段，mod.rs:313-328）。

**辅助函数**：`insert_proposals` 空集短路（mod.rs:332-346）；`write_budget_exceeded_event`（kind=`evolution_budget_exceeded`, status=warning，mod.rs:348-381）；`write_tick_completed_event`（kind=`evolution_tick_completed`，details 含 cohort/候选/预算/eligible/rejected/post_release/auto_release 计数，mod.rs:383-430）；`write_tick_failed_event`（kind=`evolution_tick_failed`, status=error, summary 截 1024 字符，mod.rs:432-457）；`truncate` 按 chars 截断（mod.rs:459-465）。

**isolation_contract_tests**（mod.rs:467-627）：
- `EXPECTED_MODULES` 15 文件名单——新文件必须进单子否则测试红（mod.rs:473-489, 507-524）。
- `production_dependencies_exclude_side_effect_entrypoints`：9 个禁止符号逐文件扫描（剔除注释与 `#[cfg(test)]` 之后的文本，mod.rs:495-505, 527-549）。
- `agent_bridge_dependencies_are_closed_and_reviewed`：`crate::agent::` 引用白名单 4 项（mod.rs:551-583）。
- `replay_persists_only_shadow_replay_rows`：replay.rs 的 `insert_one` 前一个 accessor 必须是 `shadow_replays`，且禁 `update_one` / `delete_`（mod.rs:585-606）。
- `prompt_shadow_bridge_has_no_send_or_write_dependency`：`src/agent/prompt_shadow.rs` 禁 outbox/mcp/任何写库调用（mod.rs:608-626）。

### 2.2 `src/evolution/error.rs`（25 行）

`EvolutionError` 6 变体（error.rs:9-25）：`BudgetExceeded{tokens_used, calls_used}`、`InvalidStatus(String)`、`Mongo(#[from])`、`Bson(#[from])`、`RedlineGateRejected(String)`（release 路径过红线闸被拒，与状态机错误区分，前端需单独展示"已拒绝发布"，error.rs:19-22）、`Internal(String)`。设计意图：与 `crate::error::AppError` 解耦，演化器异常不污染主链路 HTTP 响应；预算耗尽不映射 webhook 5xx（error.rs:1-5）。

### 2.3 `src/evolution/envelope.rs`（117 行）

- **`insert_experiment_envelope`**（envelope.rs:14-45）：写一条 `experiments`，初始 `status="collecting"`、cohort 空、预算/候选计数 0。`experiment_id` 唯一索引，重复 insert 触发 DuplicateKey（调用方应避免，envelope.rs:12-13）。
- **`update_experiment_status`**（envelope.rs:53-95）：status 闭集 `collecting | evaluating | awaiting_admin | released | aborted`（envelope.rs:60-63），非法值 → `InvalidStatus`；`released|aborted` 额外写 `finished_at`（envelope.rs:72-74）；`matched_count==0`（envelope 不存在）也报 `InvalidStatus`——注释说明调用方 SHALL 写 `evolution_envelope_missing` 事件，本 helper 只报错（envelope.rs:50-52, 89-93）。
- 测试仅静态断言闭集 5 值（envelope.rs:97-117）。

### 2.4 `src/evolution/budget.rs`（237 行）

- `EvolutionBudget{token_limit, token_used, call_limit, call_used}`（budget.rs:13-18）；与 `agent::budget::RunBudget` **故意不共享类型**——运行期完全隔离（budget.rs:3-4）。
- `from_config`：取 `evolution_run_token_budget` / `evolution_run_max_llm_calls`（budget.rs:22-29）。
- `check_or_fail`：exhausted → `Err(BudgetExceeded)`（budget.rs:32-40）。
- `record_call(tokens, calls)`：`saturating_add` + `max(0)` 负值钳制（LLM usage 偶发负值不回退预算，budget.rs:44-47, 230-236）。
- `exhausted`：**任一维度** `used >= limit` 即耗尽（budget.rs:50-52）。
- 测试用整份占位 AppConfig（budget.rs:59-197）——顺带可见全部 evolution 配置默认样值（如 `evolution_min_replays: 30`、`evolution_min_send_success_delta: 0.05`、`evolution_max_5gate_hit_increase: 0.10`、`evolution_max_safety_regression_rate: 0.0`、`evolution_replay_max_fail_rate: 0.30`、`evolution_cohort_per_contact_cap: 3`、`evolution_cohort_sample_per_failure_bucket: 10`，budget.rs:148-162；此处为测试构造值，语义默认值以 `src/config.rs` 为准——本次未读 config.rs，不作断言）。

### 2.5 `src/evolution/runtime_flag.rs`（195 行）

- **`load_runtime_flag`**（runtime_flag.rs:32-42）：按 workspace 读 `evolution_runtime_flags` 单文档；`None` 应视为"灰度未开"（C3 后 mongo flag 主导，runtime_flag.rs:29-31）。
- **`rollout_bucket_index`**（runtime_flag.rs:51-55）：`DefaultHasher(contact_id) % 100`；跨版本不保证稳定，但灰度不依赖跨版本稳定（runtime_flag.rs:46-50）。
- **`bucket_for_contact`**（runtime_flag.rs:63-68）：`flag.enabled && bucket < rollout_percent_clamped()`；enabled=false 一票否决；percent=0 全排除（"灰度门关但 worker 仍跑空 tick"态，runtime_flag.rs:60-62）。
- **`is_evolution_enabled_for`**（runtime_flag.rs:80-101）：四步双闸——env kill switch → mongo 文档缺失 false → flag.enabled false → 分桶；任何读错 → false + warn（"数据库抖动不得把自己升级成全量启用"，runtime_flag.rs:78-79）。**亲验：该函数当前无任何生产调用方**（仅 mod.rs:46 re-export 与 db/mod.rs:353 注释提及）——见 §5 疑点 2。
- 桶单调性测试：rollout 5%→20%→50%→100% 时已命中 contact 永不退出（runtime_flag.rs:176-186）；`rollout_percent_clamped` 把脏数据 200 钳到 100（runtime_flag.rs:189-194；实现在 models.rs:1661-1666）。

### 2.6 `src/evolution/cohort.rs`（270 行）

- **`FAILURE_FINAL_REVIEW_STATUSES`**（cohort.rs:43-51）7 个失败终态：`blocked_unverified_product_claim / held_by_ai_policy / blocked_by_safety_guard / ai_waiting_for_more_context / blocked_by_budget / blocked_by_required_field / revision_failed`。R14 修订：与 `src/agent/run_envelope.rs` 的 `FINAL_REVIEW_STATUS_VALUES` 真实枚举对齐（旧集合含 phantom 值 `budget_exceeded` 且漏 `blocked_by_required_field`，cohort.rs:39-42）；测试锁定子集关系（cohort.rs:243-253）与成功态排除（approved / revision_applied_approved / legacy_mode_unchecked 不得进失败集，cohort.rs:256-269）。
- **`select_cohorts_filtered`**（cohort.rs:68-136）：
  - 灰度短路：`runtime_flag` 为 None、`enabled=false` 或 `rollout_percent_clamped()==0` → 空 Cohorts，**不查询 run 日志**（cohort.rs:74-76）。
  - 查询：`agent_run_logs` 中 `lifecycle="completed"` + `created_at >= now - window_hours`，按 `created_at` 倒序（cohort.rs:85-101）。
  - 逐 run：`contact_in_runtime_cohort`（要求 contact 非空 **且** 命中灰度桶，cohort.rs:138-143）过滤后进 `threshold_pool`。
  - `threshold` cohort = pool 全量经 `dedup_per_contact`（每 contact 最多 `evolution_cohort_per_contact_cap`，保留最近的——cursor 已倒序，cohort.rs:113-116, 145-157）；`prompt` cohort = pool 中 final_review_status ∈ FAILURE 集者再去重（cohort.rs:117-122）。
  - 两组各自 `< evolution_min_replays` 时**置空 vec**（Requirements 2.5，调用方据此跳过候选生成，cohort.rs:124-135）。
- `select_cohorts` 是无灰度参数的旧签名包装（cohort.rs:53-59）。

### 2.7 `src/evolution/threshold.rs`（508 行）

- **`THRESHOLD_REASONABLE_BANDS`**（threshold.rs:43-50）6 gate 目标命中率区间：`fact_risk_block (0.05,0.15)`、`pressure_risk_block (0.05,0.15)`、`human_like_score_rewrite (0.08,0.18)`、`emotional_value_rewrite (0.08,0.18)`、`product_accuracy_score_block (0.05,0.15)`、`planner_block_rate_threshold (0.10,0.30)`。
- 硬边界与步长：5 闸 [1.0,10.0] 步长 1.0（整数阈值）；planner [0.05,0.95] 步长 0.05（threshold.rs:52-61）。
- **`classify_gate_hit`**（threshold.rs:65-74）：`blocked_unverified_product_claim→product_accuracy_score_block`、`held_by_ai_policy→fact_risk_block`、`blocked_by_safety_guard→pressure_risk_block`；rewrite 类不在此映射（final 多为 revision_applied_approved），由 `revision_applied` 字段补判。
- **`generate`**（threshold.rs:80-253）：
  1. cohort 空 → 空 vec（threshold.rs:87-89）。拉 cohort run 统计命中：block 三态 +1；`revision_applied=true` → human_like 与 emotional_value **各 +0.5**（run log 未记录具体是哪个 rewrite 闸触发，两侧分摊，threshold.rs:107-123）。`total_runs==0` → 空 vec。
  2. `load_gate_cooldowns`（threshold.rs:256-289）：`threshold_overrides` 中 `released_at >= now - cooldown_hours` 且 `current_version=true` 且 `rolled_back_at=null` 的 gate 集合。
  3. `load_active_threshold_overrides`（#155 修复，threshold.rs:314-344）：per gate 最新（released_at desc 首见）未回滚 override → `current_value` 基于真实生效值而非硬编码占位（threshold.rs:130-133 注释：旧实现导致候选从过期 baseline 起步且 audit previous_value 错误，并与 #152 反向门配套）。
  4. 逐 gate：`hit_counts` 只含 5 个 review gate（`review_gate_keys()`，threshold.rs:392-400），**planner gate 取不到样本 → `continue`，永不生成 planner 候选**（"缺样本不是 0 命中，接入真实观测源前不生成"，threshold.rs:150-155）。区间内 → 不产候选；区间外 → `decide_candidate` 得 (值, clamped)，`distance_from_band` 记录偏离幅度（threshold.rs:156-188），`base_revision = threshold_revision(override_id?, current_value)`（threshold.rs:183-186）。
  5. 按 distance 倒序（threshold.rs:192-196）；cooldown 中 → `rejected_below_threshold` + `failure_reason="cooldown_active"`；超 quota（>4）→ `exceeded_per_tick_quota`；否则 `pending_eval`（threshold.rs:210-217）。**超额候选仍 insert 留审计痕迹**（threshold.rs:14-16）。cohort_notes 记 `hit_rate_observed / target_lower / target_upper / total_runs_in_cohort`，clamp 过则加 `clamped_to_value`（threshold.rs:201-209）。
- **`decide_candidate`**（纯函数，threshold.rs:357-390）：方向 = `gate_hits_below_threshold(gate)`（human_like / emotional / product 三个 `<` 命中闸为 true，threshold.rs:402-407）。`>=` 命中闸（fact/pressure/planner）：hit 低 → 减阈值（更易命中）、hit 高 → 加阈值；`<` 命中闸反向。clamp 后与原值差 > EPSILON 记 `clamped=true`。
- `default_threshold_value`（threshold.rs:294-304）：fact 6.0 / pressure 7.0 / human 6.0 / emotional 6.0 / product 7.0 / planner=config.strategic_planner_block_rate_threshold——与 CLAUDE.md 五闸硬规则同源。
- `MAX_THRESHOLD_PROPOSALS_PER_TICK = 4`（threshold.rs:34）。

### 2.8 `src/evolution/prompt_critic.rs`（700 行）

- 调用路径声明（prompt_critic.rs:6-12）：生产走 `state.llm_registry` immutable snapshot，**不经 `agent::generate_agent_json`**（后者读 task-local RunBudget，worker 里没有；且需要 usage 数值计入 EvolutionBudget）；测试注入态（registry=None）回退 `state.llm` mock。
- **`EVOLVABLE_PROMPT_TARGETS`**（prompt_critic.rs:55-61）5 个 key 闭集：`user.reply.system / user.reply.policy / user.reply.fast.task / user.review.system / user.review.light.system`——其它 key 无法形成 shadow 对照证据，proposal 生成前拒绝。
- **`ALLOWED_EXPECTED_IMPROVEMENTS`** 10 个 metric key（prompt_critic.rs:65-76）；越界 key **静默过滤不 drop 整批**（prompt_critic.rs:63-64, 299-303）。
- **`generate`**（prompt_critic.rs:99-348）：
  1. `cohorts.prompt` 空或 budget 已耗尽 → 空 vec（prompt_critic.rs:107-113）。
  2. `sample_failure_buckets`（prompt_critic.rs:353-397）：按 `final_review_status` 分桶，每桶 ≤ `evolution_cohort_sample_per_failure_bucket`；样本字段 = contact_wxid / self_critique / revision_reason / pre_revision_summary；桶序按首见顺序。
  3. `load_prompt(evolution_critic_v1)` 为 Critic system（失败 → `Internal`）；`load_reply_agent_template_text` 读 `user.reply.policy` 原文供参考（**故意选 shadow 可注入的 key**，缺失时空串继续，prompt_critic.rs:438-445）。
  4. `build_user_payload`（prompt_critic.rs:407-428）：模板原文截 6000 chars；样本字段截 800/400/400。
  5. LLM 调用：`LlmPriority::Background` 过 `llm_concurrency.acquire` 闸（prompt_critic.rs:148-150）；registry 有则 `snapshot_synced(...).generate_json_with_usage`，无则 `state.llm`（prompt_critic.rs:152-175）。
  6. 记账：成功 → `budget.record_call(usage.total_tokens, 1)` + 写 `llm_call_logs`（run_mode="evolution"、run_id=experiment_id、status="success"，prompt_critic.rs:179-214）；失败 → `record_call(0, 1)` + status="failed" 日志 + 返回**单条 drop 占位 proposal** `failure_reason="critic_llm_call_failed"`（prompt_critic.rs:215-257）。
  7. 反序列化 `CriticOutput{diffs}` 失败 → drop 占位 `critic_schema_invalid`（prompt_critic.rs:260-272）；`diffs` 空 → 空 vec。
  8. **`validate_diffs` 四道闸，任一命中整批 drop**（prompt_critic.rs:459-479）：字段超长（template_key/section/snippet > 4000 chars 或 summary > 200 chars → `critic_schema_invalid`）；snippet/summary 命禁词（`lint::passes_forbidden_words` → `forbidden_literal`）；`template_key ∈ PROMPT_EVOLUTION_FORBIDDEN_KEYS`（=`["evolution_critic_v1"]`，prompts.rs:2508 亲验 → `self_referential_critic_prompt`）；不在 EVOLVABLE_PROMPT_TARGETS（→ `unsupported_prompt_target`）。全或无由测试锁定（prompt_critic.rs:678-690）。
  9. 逐 diff 落 Proposal：`idx < 4` → `pending_eval`，否则 `exceeded_per_tick_quota`（prompt_critic.rs:290-298）；`load_prompt_base_revision`（current_version=true 模板 → `prompt_revision(id, version, content)`，prompt_critic.rs:481-504）缺失 → 改判 `rejected_below_threshold` + `prompt_base_missing`（prompt_critic.rs:304-309）；cohort_notes 带桶统计 + `diff_index_in_critic_output`。
- `mk_drop_proposal`（prompt_critic.rs:506-546）：kind=prompt、status=`rejected_below_threshold`、cohort_notes.drop_reason。

### 2.9 `src/evolution/lint.rs`（83 行）

- `FORBIDDEN_LITERALS_LOWER` 13 条（lint.rs:13-28）：英文 8 变体（human takeover / human-takeover / human_takeover / hand off / hand-off / hand_off / handoff / takeover）+ 中文 5 条（人工接管 / 人工介入 / 人工托管 / 接管 / 人工）。与 `scripts/check-no-human-takeover.{sh,ps1}` 同款词典（lint.rs:3-4）。
- `passes_forbidden_words`（lint.rs:33-41）：`to_ascii_lowercase` 后 `contains` 逐条匹配；true=干净。纯字符串实现不引入 regex（lint.rs:9-11）。threshold 数值类无文本不需此闸（lint.rs:7）。

### 2.10 `src/evolution/replay.rs`（967 行）

- 头注释（replay.rs:1-27）：threshold 候选=纯重判（读源 run `review.scores` 与候选阈值对比，不调 LLM 不写业务表）；prompt 候选=调 `agent::prompt_shadow::shadow_replay_prompt_one` 用「原 prompt + critic 追加片段」跑真实 Reply+Review 演练（永不触发送链）；严格隔离清单；并发 Semaphore；预算超额未启动的 replay 写 failed。
- 方向常量：`BLOCK_DIRECTION_GTE = [fact_risk_block, pressure_risk_block]`（score>=阈值触发）；`REWRITE_DIRECTION_LT = [human_like, emotional_value, product_accuracy_score_block]`（score<阈值触发；product 走 `<` 与 `review_passed` 对偶，replay.rs:42-52）。
- **`eval_all`**（replay.rs:57-150）：拉本 experiment 的 `pending_eval` proposals；空 → Ok。读 envelope 拿 `cohort_threshold_run_ids / cohort_prompt_run_ids`（缺 envelope → `InvalidStatus`，replay.rs:88-105）。每 (proposal × source_run) 一个 tokio task，`Semaphore::new(evolution_replay_concurrency.max(1))` 限流（replay.rs:110-143）；threshold 候选用 threshold cohort、prompt 用 prompt cohort、未知 kind 跳过（replay.rs:119-123）。**budget 是 &mut 不能跨 task**：起 task 前 `budget.exhausted()` 预检，超额 → `insert_replay_failed("evolution_budget_exceeded")` 不起 task（replay.rs:107-133）。task 内错误被 `let _ =` 吞（replay.rs:140）。
- **`run_shadow_replay`**（replay.rs:153-260）：
  1. 反查源 run，缺 → `failed("source_run_not_found")`（replay.rs:161-186）。
  2. **retention 探针只对 prompt 候选**（threshold 纯读 scores 不需原文，探针会错杀被 retention 清理的源 run，replay.rs:190-195）：要求 `source_kind == SOURCE_KIND_INBOUND_MESSAGE`、`source_event_id` 非空且非 `synthetic:` 前缀、contact 非空，然后按 snake_case `message_id` 查 `messages` 表（**必须 snake_case**，否则全部 happy path 错杀——replay.rs:204-209 注释 + `prompt_shadow_retention_message_filter` replay.rs:262-274）；count==0 → `failed("source_message_unavailable")`。
  3. 分派：threshold → `evaluate_threshold`；prompt → `shadow_replay_prompt_one`，Ok(sample) → `prompt_sample_to_outcome`，Err → `failed_with("prompt_shadow_error:{e}")` 不向上抛（单条不拖垮整批，replay.rs:239-257）。
  4. `persist_replay` 写 `shadow_replays`（唯一落点，replay.rs:565-613）。
- **`evaluate_threshold`**（纯函数，replay.rs:277-347）：缺 gate_key/proposed_value/review.scores → 对应 failed 原因。对 5 gate 逐一算 original/new 命中向量：被改 gate 用 current_value（缺则 default）vs proposed_value，其余 4 gate 两侧同用 default 阈值（delta 恒 0，replay.rs:293-318）。**KE-02**：original 终态也用 5 闸重推（`final_status_from_5gate(original_5gate_hit)`），不用源 run 真实终态——否则非-5gate 终态（blocked_by_budget 等）会让 original 算失败、new 算成功，凭空 +send_delta 翻越显著性门（replay.rs:320-346 + 测试 replay.rs:764-791）。
- **`evaluate_single_gate`**（replay.rs:351-362）+ **`read_gate_score`**（replay.rs:395-413）：双键兼容（`factRisk|hallucinationScore`、`productAccuracy|knowledgeGroundingScore` 等），i32/f64 都接；缺分按 0.0 保守（block 类不命中、rewrite 类命中，replay.rs:353-354, 415-418）。H11 护栏测试锁定真实序列化键可读（replay.rs:848-889）。
- **`default_gate_threshold`**（replay.rs:380-389）：5 闸硬常量 6/7/6/6/7；prompt shadow 两侧固定同一组默认阈值，唯一变量是 prompt 片段（干净归因，replay.rs:364-379）。
- **`final_status_from_5gate`**（replay.rs:445-466）：block 命中（优先级 fact > pressure > product）→ `held_by_ai_policy` / `blocked_by_safety_guard` / `blocked_unverified_product_claim`；仅 rewrite 命中 → `revision_applied_approved`；全不中 → `approved`。threshold/prompt 两路共用同一口径。
- **`prompt_sample_to_outcome`**（replay.rs:472-514）：sample.status != completed → failed（进 significance 的 failed 分母）；新旧 scores 各推 5 闸向量（`scores_to_5gate_hit`，replay.rs:419-440）；final 状态优先用 sample 自带，缺则由向量推；selfCritique addressed 两侧透传。
- `ReplayOutcome`（replay.rs:516-563）：completed / failure_reason / original 侧（final、5gate、self_critique——G4 真实基线）/ new 侧（final、risks、token_cost、self_critique、5gate）。`persist_replay`（replay.rs:565-613）：status = completed|failed；`insert_replay_failed`（replay.rs:615-650）为预算超额未启动路径。

### 2.11 `src/evolution/significance.rs`（1039 行）

> **线 H 换血追记（2026-08-14，b7caec9/08f6069，本节以下为换血前快照、行号已漂移）**：threshold 判定主指标从评审放行率（send_success delta）换血为 **`outcome_weighted_delta`**——按 `source_run_id → AgentRunLog.run_id → AgentDecisionReview.outcome_status` 只读 join 真实用户反应，经共享三态分类器（真相源 `src/agent/outcome_label.rs`，H1 自 gap_signals 抽出、re-export 逐字节等价；Hit=买入信号/Block=负向集/Censored=删失）算新旧配置「放行∧Hit − 放行∧Block」占比差（分母=非删失；"放行"仍认 SEND_SUCCESS_STATUSES，故 159 行常量仍在但不再是判定正例定义）。新增样本硬门：非删失 < `EVOLUTION_MIN_REPLAYS`（env 双语义）→ `insufficient_outcome_samples` 直接拒。`min_send_success_delta` cfg 字段现承载 outcome 门槛（env 名 `EVOLUTION_MIN_SEND_SUCCESS_DELTA` 保持部署兼容）。评审放行率降为 `_observed` 仅观测。#152 安全回归门与 5 闸涨幅门语义与拦截状态集**零变化**（161-163 行相应描述仍准确）。三态分布记入 `eval_metrics`。H3：post_release 主观测指标换 `actual_outcome_weighted_score_delta` + 恒写 `before/after_outcome_distribution`（仍仅观测不自动回滚）。

- **`FIVE_GATE_KEYS`** 5 个（不含 planner——不在 shadow 内观测，significance.rs:32-39）。
- **`SEND_SUCCESS_STATUSES = ["approved", "revision_applied_approved"]`**（significance.rs:41-42）——"成功送出"语义唯一定义，design.md §4.6 同源。`success_rate` 以此为正例、None/其它为负（significance.rs:360-377）。
- **`SAFETY_GATE_BLOCK_STATUS`**（significance.rs:44-59）：`fact_risk_block→held_by_ai_policy`、`pressure_risk_block→blocked_by_safety_guard`、`product_accuracy_score_block→blocked_unverified_product_claim`；rewrite 两闸 + planner **不是**安全闸（放松不构成"漏过风险消息"）；#152 反向门只对本表 gate 生效。`safety_block_status_for`（significance.rs:62-68）。
- **`SignificanceCfg`**（significance.rs:71-93）：min_replays / min_send_success_delta / max_5gate_hit_increase / max_fail_rate / max_safety_regression_rate（默认 0.0 零容忍——任一条风险消息 blocked→sent 即否决，significance.rs:77-80）。
- **`grade_safety_regression`**（significance.rs:101-128）：completed 中 `original_final == 该闸拦截态 && new_final ∈ SEND_SUCCESS` 的占比（分母=全部 completed）；`rate <= max` 过（`<=` 而非 `<`：max=0 时 count=0 → 0.0<=0.0 pass，任一回归 fail，significance.rs:124-127）；非安全闸/None → 恒过 (true, 0, 0)。
- **`grade_threshold`**（significance.rs:139-209）：`early_reject`（completed < min_replays → `insufficient_completed_replays`；fail_rate > max_fail_rate → `replay_fail_rate_above_threshold`，significance.rs:326-358）→ send_delta（NaN → `nan_in_metrics`）→ `compute_5gate_deltas`（new_rate - original_rate；original 侧经 `original_5gate_hit_or_default` 读真实值，缺 gate 回落 false——G4 假基线已修，significance.rs:398-424, 451-469）→ 三门合取：`send_delta >= min && max_increase <= max && safety_passed`（gate 边界含等号，测试 significance.rs:860-882）；失败 reason 优先级 **safety > send > gate**（significance.rs:198-207）；metrics 全量落 `eval_metrics`（含 per-gate delta doc、safety 率/计数/拦截态）。
- **`grade_prompt`**（significance.rs:229-308）：**阶段二语义——不自动放行/拒绝**。`completed==0` → `(false, reason="no_completed_replays")`；`completed>=1` → `(true, …)` 即 eligible（"证据就绪待管理员把关"，`eligibility_basis="completed_ge_1_pending_human_review"`）。self_critique 新旧率/delta、5 闸涨幅、token_cost 全部 `_observed` 后缀仅观测；`per_sample_evidence` 数组逐条带新旧 final/5gate/critique/token（significance.rs:270-304）。**prompt 不走 early_reject**（min_replays/fail_rate 门不适用，`cfg` 仅保留签名，significance.rs:230）。
- **`aggregate_and_grade`**（significance.rs:482-604）：拉本 experiment 全部 proposals，仅处理 `status=="pending_eval"`（其它视为已被 quota/校验拒绝，不再变更，significance.rs:514-516）；逐 proposal 拉其 shadow_replays → 按 kind 调 grader（未知 kind → reject `unknown_proposal_kind`）→ passed → `eligible_for_release`，否则 `rejected_below_threshold` + failure_reason=metrics.reason（缺省 `significance_failed`）；update 写 `eval_replays_completed/failed`、`eval_metrics`、`significance_passed`；返回 `(eligible_count, rejected_count)`。
- 关键测试：#152 放松安全闸单条翻转即拒（significance.rs:921-954）；收紧方向不算回归（significance.rs:956-976）；非安全闸跳过反向门（significance.rs:978-1007）。

### 2.12 `src/evolution/revision.rs`（142 行）

- 不可变 revision token，绑定评估证据与被读的确切工件（revision.rs:1-4）。
- `threshold_revision(source_id, value)` = `"threshold-v1:{source_hex|baseline}:{f64.to_bits():016x}"`（revision.rs:40-48）；`parse_threshold_revision` 严格（前缀/段数/hex 校验，revision.rs:50-69）。
- `prompt_revision(template_id, version, content)` = `"prompt-v1:{id}:{version}:{sha256(content)}"`（revision.rs:71-77）；parse 要求 sha 长度 64 且无多余段（revision.rs:79-95）。
- `EVOLVABLE_PROMPT_KEYS`（revision.rs:26-32）与 prompt_critic 的 `EVOLVABLE_PROMPT_TARGETS` 同 5 个 key（两处重复定义，见 §5 疑点 8）。

### 2.13 `src/evolution/release.rs`（1323 行）

- **`ensure_release_gate_open`**（release.rs:30-48）：env `evolution_enabled` + mongo flag.enabled（缺文档=false）双闸；任一关 → `InvalidStatus`。release 前置；**rollback 两函数均不调它**（admin 永远可回滚，亲验 rollback_threshold/rollback_prompt 无此调用）。
- **`release_threshold`**（release.rs:175-436）：
  - 事务前校验：proposal 存在、kind=threshold、status=`eligible_for_release`、有 gate_key/proposed_value；`threshold_value_is_representable(gate, proposed)`（5 闸只接受可表示值——agent/runtime.rs:775 亲验，6.5 之类被拒，release.rs:222-226）；base_revision 可解析、`current_value.to_bits() == parsed_base.value.to_bits()`（bit 级一致，release.rs:237-241）、base 值同样可表示。
  - 事务内（`start_session + start_transaction`）：
    1. **cooldown 闸**：同 scope+gate 在 `evolution_threshold_release_cooldown_hours` 内已有 `released_at >= since` 的 override → 拒（注意该查询**不**过滤 rolled_back_at，被回滚的 release 仍占 cooldown——保守方向，release.rs:265-290，见 §5 疑点 7）。
    2. **OCC 基线校验**：base 有 source_id → 把那条 override（_id+scope+gate+value+current_version=true+未回滚全匹配）置 `current_version=false`，matched!=1 → "base revision changed before release"（release.rs:292-316）；base 是 baseline（None）→ 要求当前**无任何** current 未回滚 override，否则 "baseline changed"（release.rs:317-338）。
    3. insert 新 override（`_id=override_id`、value、source_proposal_id、base/released_revision、current_version=true、released_at/by，release.rs:340-358）。
    4. proposals 推进（filter 含 status=eligible_for_release + base_revision——双 OCC；set status=released + released_at/by/revision，matched!=1 → 拒，release.rs:360-390）。
    5. **audit 行同事务**（#155：旧实现 commit 后 best-effort 仅 warn，阈值变更可能无审计行生效；现 `threshold_overrides_audit` insert 在 commit 前，release.rs:392-411 + `build_threshold_override_audit` release.rs:1229-1255）。
    6. `insert_release_observability_with_session`：`agent_events`（kind=`evolution_threshold_released`，dedupe_key=`evolution:{kind}:{proposal_id}`，release.rs:50-79）+ **`post_release_reviews` 文档同事务插入**（调 `post_release::post_release_review_document`，release.rs:81-130）。
    7. `commit_with_session`：`UnknownTransactionCommitResult` label 无限重试，其它错误上抛（release.rs:711-721）。
- **`release_prompt`**（release.rs:455-707）：
  - 校验：kind=prompt、eligible、有 proposed_template_key/diff_snippet/base_revision；加载 current 模板（_id=parsed_base.template_id + key + version + current_version=true）；`content_sha256(current.content) != parsed_base.content_sha256` → "base content changed"（release.rs:513-537）。
  - **红线三闸（事务外，与管理员手动编辑路径同源 `prompt_guard`）**：`compose_appended_content`（末尾追加，原正文逐字保留，prompt_guard.rs:132 亲验存在）→ 闸1+2 `validate_prompt_edit`（禁词+锚点完整性；失败 → `RedlineGateRejected`）→ 闸3 `review_prompt_edit`（LLM 语义审查追加增量：变相真人转介/削弱 grounding 等语义绕过）；`Reject(reason)` 与 `NeedsHumanConfirm`（LLM 不可用）都 → `RedlineGateRejected`——**不 fail-open 放水也不 fail-closed 死路，本次 release 中止请管理员逐字核对**（release.rs:539-569）。
  - 事务内：旧 current 置 `current_version=false + status=archived`（OCC 含 content 逐字匹配，release.rs:587-616）→ insert 新版本（version+1、current_version=true、previous_version、`seeded_by="evolution_release"`、`source_proposal_id`，release.rs:618-645）→ proposals released（记 `previous_prompt_version=old_version.to_string()`，release.rs:647-678）→ observability（kind=`evolution_prompt_released`）→ commit。
  - **commit 后**才 `state.prompt_pack_version.fetch_add(1, SeqCst)` 让 LRU prompt 缓存失效（commit 失败不错误标脏，release.rs:701-706）。
- **`rollback_threshold`**（release.rs:729-944）：status 必须 `released`；解析 released_revision 得 artifact id（无 id → 拒）；事务内：① override 置 `current_version=false + rolled_back_at/by`（OCC：_id+source_proposal_id+scope+gate+value+released_revision+current_version=true+未回滚全匹配；matched!=1 → "artifact no longer current or owned"，release.rs:807-839）；② base 有 source_id → 恢复前任 override 为 current（OCC 匹配 base_revision 与 value；matched!=1 → "predecessor unavailable"，release.rs:841-868）——base 为 baseline 时无此步（回滚后无 current override，读端自然回落 baseline）；③ proposal → `rolled_back`（OCC status=released+released_revision）；④ audit 行同事务（previous=被回滚值、new=None，release.rs:901-921）；⑤ `evolution_rollback_completed` 事件；commit。生效机制：`resolve_thresholds` 读 override 时过滤 `rolled_back_at=null`，回滚后下一 run 立即回读上一档（release.rs:723-728）。
- **`rollback_prompt`**（release.rs:951-1211）：status=released；事务内：① find 当前 current 模板且 `source_proposal_id == proposal_id`（该 proposal 必须仍拥有 current 产物），并验证 `prompt_revision(current) == released_revision`（内容漂移拒，release.rs:1025-1063）；② 冻结基线行必须仍存在且 sha 一致（release.rs:1065-1093）；③ current 置 archived（OCC）；④ **只恢复 proposal 冻结的基线行**（按 _id+version 不按可碰撞 version 猜，release.rs:1120-1146）；⑤ proposal → rolled_back；⑥ 事件；commit 后 bump prompt_pack_version（release.rs:1206-1208）。
- 调用方亲验：`routes/evolution.rs:271-345` 按 kind 分派 release/rollback 四函数。

### 2.14 `src/evolution/auto_release.rs`（595 行）

- **政策硬闸**：`CURRENT_AUTO_RELEASE_POLICY_ENABLED: bool = false`（编译期常量，auto_release.rs:36-40）——"当前产品边界是全部 proposal 由管理员显式发布"（HC-017）；配置闸即使误开也在任何查询/写入前返回零（auto_release.rs:1-6）。`auto_release_gate_open` = 政策常量 && env `evolution_auto_release_enabled` && workspace 子闸 `flag.threshold_auto_release_enabled`（三重 AND，auto_release.rs:42-45；测试锁死 auto_release.rs:582-594）。
- **`auto_release_eligible_thresholds`**（auto_release.rs:52-226）：双重快速短路（政策常量或 env 关 → Ok(0)，auto_release.rs:57-59）；读 workspace 子闸（读失败 `.ok().flatten()` 视作未开，auto_release.rs:60-71）；扫 `proposal_kind=threshold AND status=eligible_for_release`；`compute_window_gate_hit_rates` 一次扫描复用给所有候选（窗口 `evolution_auto_release_window_hours`，auto_release.rs:98-105）；**负反应强制门**（2.5-main-4）：`evolution_auto_release_negative_reaction_gate_enabled=true` 才算当前窗口**绝对**负反应率（复用 `post_release::compute_negative_reaction_rate` 同口径同极性源；门关零开销，auto_release.rs:107-125）；逐候选：cap（`evolution_auto_release_per_tick_cap`）命中即 break；缺 gate_key / gate 不在 BANDS → warn skip；`decide_auto_release` + `decide_negative_reaction_block` → `final_decision`；**决策事件先写**（无论 release 成败留审计，kind=`evolution_auto_release_decision`，status=release|skip，forced_skip 落 `negative_reaction_forced_skip` 标记，auto_release.rs:183-201, 346-411）；放行 → `release::release_threshold(..., admin="evolution_auto_release")`，失败 warn 下 tick 重试（auto_release.rs:203-224）。
- **`decide_auto_release`**（纯函数，auto_release.rs:240-260）——KE-01 方向门：观测 None → false（无信号不盲动）；current/proposed 任一缺 → false；`proposed > current`（升阈）仅 `rate > upper` 放行；`proposed < current`（降阈）仅 `rate < lower` 放行；相等 → false。旧实现只判 band 外任意一侧会反向放量（升阈候选在命中率已过低时仍放行），本修是**安全收窄**（auto_release.rs:228-239）。
- **`decide_negative_reaction_block`**（auto_release.rs:274-282）：enabled=false → 永 false（字节等价）；observed=None → false（无信号不强制 skip）；`rate > max` → true=强制 SKIP 退回 admin（**非回滚**，不触 Req 9.7）。阈值是**绝对值**而非 delta——release 前决策没有"后窗口"可比（auto_release.rs:270-273）。
- **`compute_window_gate_hit_rates`**（auto_release.rs:290-344）：窗口内 agent_run_logs 全扫；block 三态各 +1；`revision_applied=true` → human_like 与 emotional_value **各 +1**（注意与 threshold.rs 的 +0.5 分摊不同，见 §5 疑点 3）；planner gate 无计数源 → map 中缺失 → decide 收到 None 保守拒（auto_release.rs:338-339）。total=0 → 空 map。

### 2.15 `src/evolution/post_release.rs`（575 行）

- `REVIEW_WINDOW_HOURS = 24`（post_release.rs:32）。
- **`APPROVED_LIKE_STATUSES`** = approved + revision_applied_approved（分子，post_release.rs:36）；**`UPGRADED_STATUSES`** 9 态（分母；legacy 脏值天然剔除，post_release.rs:40-50）。
- **`FIVE_GATE_KEYS`（post_release 版）**（post_release.rs:60-69）：`fact_risk_block→held_by_ai_policy`、`pressure_risk_block→revision_failed`（**[2-01] 有意偏离**：pressure 在生产是软闸走 revision 不产 block 终态，命中率走 revision_failed 口径；fact/product 与 significance 权威一致——测试钉死 post_release.rs:503-532）、`human_like_score_rewrite→revision_failed`、`emotional_value_rewrite→revision_failed`、`product_accuracy_score_block→blocked_unverified_product_claim`。
- **`schedule_post_release_review`**（post_release.rs:76-99）：独立 insert（**不参与 release 事务**、失败仅 warn 的兼容路径）——**亲验当前无任何调用方**（生产路径是 release.rs:112-128 在事务内直接 insert `post_release_review_document`；见 §5 疑点 1）。`post_release_review_document`（post_release.rs:104-125）：scheduled_at=released_at+24h、completed=false、protocol_version=1。
- **`run_due_reviews`**（post_release.rs:132-167）：扫 `scheduled_at <= now && completed=false`（scope 过滤），逐条 `process_one_review`，单条失败 warn 继续（下 tick 重试）；返回完成数。
- **`process_one_review`**（post_release.rs:169-294）：解析文档字段（缺任一 → `Internal`）；BEFORE=[released-24h, released) 与 AFTER=[released, released+24h) 两窗 `compute_window_metrics`；`delta_send_success = after.zip(before).map(a-b)`（任一窗无样本 → None 不写）；2.5-pre-3 `delta_negative_reaction` 同款 zip（**仅观测不参与判决**，post_release.rs:207-212）；`delta_5gate` 两窗都有值的 gate 才写；update `completed=true + completed_at + actual_*` 三 delta；写 `evolution_post_release_review` 事件——details 含 before/after 总数、delta、负反应升幅超 `evolution_max_negative_reaction_increase` 时的 `negative_reaction_increase_breached_observed` 观测 flag（**不判决**，强制门留 main-4，post_release.rs:256-264）。**不自动回滚**（Requirements 9.7，post_release.rs:16-17）。
- **`compute_window_metrics`**（post_release.rs:311-376）：total=UPGRADED 分母 count；approved_like count；send_success_rate=None（total=0 时）；每 gate 按映射终态 count / total；+ `compute_negative_reaction_rate`。
- **`compute_negative_reaction_rate`**（pub(crate) 供 auto_release 复用，post_release.rs:396-444）：`agent_decision_reviews` 按 `outcome_status` group-count（`$exists && $ne null`）；G07：加载 active domain profile 的 `outcome_polarity` 经 `resolve_effective_polarity` 解析，`classify_outcome_label_with_polarity` 三态判定（Hit/Block/Censored；沉默/pending/未分类=Censored 删失，不进分子分母）；`negative_reaction_rate_from_counts` = Block/(Hit+Block)，0 分类 → None（防 0/0 NaN，post_release.rs:446-458）。
- `released_at_plus_hours`：ms 算术 + saturating，负值即向前回看（post_release.rs:460-468）。

### 2.16 `src/planner/mod.rs`（4301 行）

**顶层结构**：`run_strategic_planner`（planner/mod.rs:127-156）——per-tick 枚举 scope，`run_scope_scans_isolated` 六段独立 try（macro 包裹，单段失败仅 error log 不阻断其它段，planner/mod.rs:183-224）；`tick()` 测试入口为短路语义（planner/mod.rs:158-181）。

**共享设施**：
- domain_attributes 读 helper：`contact_customer_stage / contact_intent_level / contact_value_tier / contact_relationship_type / contact_customer_stage_updated_at`（planner/mod.rs:31-68）；`contact_stagnation_updated_at(dim)`（1G-c：`<dim>_updated_at` 缺失回落旧 `customer_stage_updated_at`——防换维度后存量 contact 主动触达静默冻结；DEFAULT 同 key 逐字等价，planner/mod.rs:70-86）；`contact_stagnation_value`（BSON String 取原文防 JSON 引号污染 subject，planner/mod.rs:88-101）。
- `canonical_subject(prefix, facts)`：`"{prefix}:{serde_json compact}"`——业务代（generation）事实编码，无分隔符歧义（planner/mod.rs:103-107）。
- **daily cap**：`EMIT_EVENT_KINDS` 7 个 emit kind（emit/commitment_overdue/commitment_imminent/stage_stagnation/calendar_care/renewal_reminder/reactivation，planner/mod.rs:231-239）；`count_today_emit_events` 跨段汇总当日计数（planner/mod.rs:242-263）+ `count_today_segment_events` 段内计数（planner/mod.rs:265-285）；`day_start_before` 按 UTC epoch 整天粗近似（planner/mod.rs:287-294）。backoff / capped kind **不in** EMIT_EVENT_KINDS 不耗 cap（测试锁定 planner/mod.rs:3542-3549）。
- `commit_planner_follow_up`（planner/mod.rs:296-333）：包装 `proactive_outreach::commit_follow_up`，quota namespace=`"strategic_planner"`、account_scope=Some(account)。
- `has_pending_follow_up`：同 contact 存在 pending|retry|running 的 follow_up 即 true（planner/mod.rs:359-375）。
- `write_capped_event`（kind=`strategic_planner_capped`，planner/mod.rs:377-404）。
- **运营范式解析 `resolve_operation_mode`**（planner/mod.rs:1134-1149）：三级整组替换——`contact.operation_mode_override ?? profile.per_relationship_operation_mode[relationship_type] ?? profile.operation_mode`；不做逐驱动力 merge。测试锁定 override 优先于关系类型（planner/mod.rs:3670-3703）、DEFAULT 零扰动（planner/mod.rs:3705-3717）。
- **M3 block-rate 反馈环**：`classify_review_status`（planner/mod.rs:1386-1401）blocked-like 7 态（+1,0）/ ok-like 3 态 approved|revision_applied_approved|local_decision_review（0,+1）/ 其它 (0,0) 不参与；`should_skip_for_block_rate`（planner/mod.rs:1406-1461）：窗口 `strategic_planner_block_rate_window_hours`、`min_runs`、阈值经 **`agent::runtime::resolve_thresholds(contact).planner_block_rate_threshold`**（M4 W4 Task 5.1：threshold_overrides release 下一 tick 立即生效，planner/mod.rs:1413-1418）；任一配置 <=0 → 不启用；`total < min_runs` → None；`rate >= threshold` → Some(detail)；`write_backoff_event` 写 `strategic_planner_<segment>_backoff`，status=skipped（planner/mod.rs:1467-1494）。

**段 1 silent**（planner/mod.rs:410-600）：
- DB 粗筛 `silent_candidate_filter`：managed + `last_inbound_at < now - silent_threshold_hours` + 非冷却 $or（planner/mod.rs:549-565）；全局阈值是所有 contact override 的上界（override 只会收紧，粗筛绝不漏，planner/mod.rs:416-419）。
- 内存 `silent_candidate_passes_in_memory`（planner/mod.rs:569-587）：managed、必须有 last_inbound、**`last_outbound >= last_inbound` 排除**（"Agent 刚发出去用户没回不算静默，否则 Planner 帮 Agent 自言自语堆消息"，planner/mod.rs:567-568）、冷却排除。
- H8 范式：`mode.silence.enabled` 关 → skip；有效阈值 `threshold_hours.unwrap_or(global)`（planner/mod.rs:444-457）。
- 流程：pending 幂等 → 余额 `remaining<=0` → capped 事件 + break → block-rate backoff（continue 不耗 cap）→ `commit_planner_follow_up`（subject=`last-inbound:{ms|unknown}`、kind=`strategic_planner_emit`）→ Emitted 扣减 / Duplicate continue / Capped break（planner/mod.rs:458-521）。段尾写 `strategic_planner_tick` 汇总事件（planner/mod.rs:523-543）。
- `silent_hours_for`：saturating_sub + `.max(0)` 防时钟回退产生负值（R14，planner/mod.rs:589-600）。

**段 2 commitment**（planner/mod.rs:604-972）：
- 筛选三阶段：**扫描收集 → 优先级排序 → 消费 cap**（与 silent 的单遍不同）。
- DB 粗筛 `commitment_candidate_filter`：managed + commitments 非空 + 非冷却（planner/mod.rs:641-654）。
- **`pick_commitment_emit_target`**（planner/mod.rs:656-732）：只看 `CommitmentRepr::Structured` 且 id 非空；due_at 缺失时 `fallback_due_hours > 0` → 用 `created_at + fallback` 合成兜底 due（标记 `is_fallback_due=true`；=0 保留旧行为跳过，planner/mod.rs:660-695）；`due < now` → Overdue；`now <= due <= now+imminent_window` → Imminent；否则跳过；多条取（Overdue 优先，同 reason 内取最早 due）——score 元组比较（planner/mod.rs:704-731）。
- dedup：`latest_commitment_emit` 查该 commitmentId 最近一次 overdue/imminent 事件（sort created_at desc + _id desc，planner/mod.rs:738-767）；`dedup_hours` 内已 emit → skip（planner/mod.rs:846-853）；窗口过期后**事件 _id 变成下一代 intent 的 predecessor**（并发扫描器在时间窗边界也推导出同一代，planner/mod.rs:734-737）。
- subject=`commitment:[commitment_id, due_ms, predecessor_hex|genesis]`（planner/mod.rs:912-915）。
- 排序键 `commitment_priority_key`（planner/mod.rs:1337-1356）：`(reason_ord[Overdue=0<Imminent=1], -stage_w, -value_tier_w, -intent_w, due_ms)` 升序，值小者先 emit；`priority_enabled=false` 退回 cursor 自然序（planner/mod.rs:860-869）。
- H8：`mode.commitment.enabled` 关 → skip；`imminent_window_hours` override（planner/mod.rs:829-839）。event kind 按 reason：`strategic_planner_commitment_overdue|_imminent`（planner/mod.rs:626-639）。

**段 3 stage_stagnation**（planner/mod.rs:974-1671）：
- `TERMINAL_STAGES` 写死 3 个：customer_success / cooldown / dormant_reactivation（对齐 m006 种子，planner/mod.rs:978-983）。
- **`PlannerStageConfig`**（planner/mod.rs:985-1073）：stage/intent 权重 map、终态集、再激活目标集、停滞维度——每 tick 由 `build_planner_stage_config` 从 active DomainProfile + taxonomy 缓存构造一次（避免 N+1，planner/mod.rs:1075-1123）；空字典回落写死函数（DEFAULT 逐字等价，测试 planner/mod.rs:3286-3322）；非空字典整组覆盖（planner/mod.rs:3324-3349）。
- DB 粗筛 `stage_stagnation_candidate_filter`（planner/mod.rs:1191-1226）：managed + `domain_attributes.customer_stage` 存在非 null 且 `$nin` 有效终态集 + **计时维度 dotted-key 动态化**（`domain_attributes.{dim}_updated_at < before` **$or** 新字段缺失且旧 `customer_stage_updated_at < before`——镜像内存回落救存量 contact，问题 B 修复，planner/mod.rs:1178-1210）+ 非冷却 + `last_inbound_at < inbound_before`（避开 silent 段重叠）。
- 内存 `stage_stagnation_passes_in_memory`（planner/mod.rs:1229-1259）：managed/非冷却、有 stage、非终态（config.is_terminal_stage）、有停滞计时戳、`last_outbound >= last_inbound` 排除（Agent 已 ping 未回不叠加催）。
- H8：funnel.enabled 关 → skip（"关 funnel = 对该 contact 纯减法"）；有效停滞阈值 override（planner/mod.rs:1551-1567）。
- 排序键 `stage_stagnation_priority_key`：`(-stage_w, -value_tier_w, -stagnation_ms)`（停滞越久越优先，planner/mod.rs:1358-1377）。
- subject=`stage:[dim, value, updated_at_ms|null]`（planner/mod.rs:109-115）——维度/取值/计时代变化都开新代（测试 planner/mod.rs:3165-3205）。
- 权重函数：`stage_priority_weight`（commitment_followup=100 > objection_handling|solution_fit=80 > need_discovery=60 > relationship_building=40 > new_contact=20 > 终态=10，缺省 20，planner/mod.rs:1294-1304）；`intent_level_weight`（high 80/medium 50/low 20/其它 10，planner/mod.rs:1307-1314）；`value_tier_weight`（high 80/mid 50/low 20/无 10——G6 插在 stage 之后 intent 之前，存量无 tier 零扰动，planner/mod.rs:1319-1326）。
- **永不驱动铁律**：`bayesian_signals` / `personality_profile` 是旁路观察字段，绝不进候选谓词/排序键——契约测试构造仅这两字段不同的 contact 断言所有硬行为输出逐字相同（planner/mod.rs:3408-3514）。

**段 4 calendar**（planner/mod.rs:1673-2008）：
- 数据源：profile `memory_dimensions` 中 `date_dimension=true` 的槽 key；**date_dims 空 → 整段 no-op**（销售 DEFAULT 命中此短路，零 DB 扫描，planner/mod.rs:1835-1844）。
- 粗筛 `managed_active_candidate_filter`（managed+非冷却；calendar/renewal 共用，planner/mod.rs:2013-2024）。
- 逐 contact：`mode.calendar.enabled` 关 → skip；只读 `operating_memories` → `agent::effective_memory_card`（亲验 re-export agent/mod.rs:124）→ `anniversaries_from_extra` 解析结构化 `AnniversaryEntry`（旧字符串条目逐个跳过不报错，planner/mod.rs:1807-1817）。
- **`anniversary_occurrence`**（planner/mod.rs:1698-1725）：recurring=true（"MM-DD"）→ 今日起 lookahead+1 天逐日推进（`civil_from_offset` 用 chrono NaiveDate，跨月/跨年/闰年 02-29 自然覆盖，planner/mod.rs:1782-1793）比对月日；recurring=false（"YYYY-MM-DD"）→ 完整年月日比对；解析失败 → None（向后兼容）。返回**锚定到具体年份的 occurrence**——提前 1 天 emit 与纪念日当天扫描共享同一业务代（planner/mod.rs:1698-1701，测试 planner/mod.rs:4157-4180）。
- subject=`occurrences:[[dim,date,recurring,"YYYY-MM-DD"]...]`（排序去重，planner/mod.rs:1727-1749）。
- **双 cap**：calendar 专属 `effective_cap`（mode override ?? `strategic_planner_calendar_daily_cap`）+ 跨段总 cap 取更紧（`calendar_emitted_today >= effective_cap || already + calendar_emitted >= regular_cap` → `strategic_planner_calendar_capped` + break，planner/mod.rs:1919-1942）；commit 时 `segment_cap=Some(effective_cap)` 由 quota 事务硬保证（planner/mod.rs:1971-1973）。
- 时区：`today_in_offset(now_ms, strategic_planner_calendar_tz_offset_hours)`——固定偏移整数运算不依赖宿主时区（planner/mod.rs:1795-1805）。

**段 5 renewal**（planner/mod.rs:2026-2228）：
- **扫描器级粗过滤** `renewal_scan_should_run`：profile 默认开 **或** per_relationship 任一关系开 → 才扫表；DEFAULT（默认关+无 per_relationship）→ return 零 DB 扫描字节等价（planner/mod.rs:1151-1163, 2070-2077）；contact 级 override 单独开的边缘**不在**此层覆盖（与省扫描初衷冲突，注释明示，planner/mod.rs:2070-2074）。
- `load_active_products` 每 tick 一次（避免 N+1，planner/mod.rs:2083-2085）；逐 contact `project_entitlements(outcome_events, products, now, usize::MAX)`（G4 投影：仅已核实成交派生持有）。
- **`renewal_due_soon`**（planner/mod.rs:2038-2053）：`expires_at ∈ [now - grace_days, now + lookahead_days]`（含两端）；None（永久授权）永不；过期超 grace 自然收口不再骚扰（planner/mod.rs:2036-2037）。
- subject=`entitlements:[[product_id, expires_ms|null]...]`（排序去重，planner/mod.rs:2163-2178）；content 内嵌"续费=最高优先级销售，挽留优先；若客户犹豫则诊断顾虑"指令（planner/mod.rs:2179-2181）；双 cap 同 calendar（planner/mod.rs:2135-2156）。

**段 6 reactivation**（planner/mod.rs:2230-2484）：
- 粗过滤 `reactivation_scan_should_run` 同 renewal（planner/mod.rs:1165-1173, 2299-2304）。
- DB 粗筛 `reactivation_candidate_filter`：managed + `customer_stage $in` 再激活目标集（字典缺省回落 `["dormant_reactivation"]`，与原 `==` 查询字节等价）+ 非冷却（planner/mod.rs:2260-2281）。
- 门控 1 dormant：`customer_stage_updated_at` 距今 ≥ `dormant_days`（**无计时戳的存量休眠客 → 视为已休眠足够久纳入唤醒**——"绝不放任"，planner/mod.rs:2347-2361）；门控 2 cadence：`latest_reactivation_emit`（planner/mod.rs:2234-2258）距今 < `cadence_days` → skip（定期低频不刷屏，planner/mod.rs:2362-2371）。
- 文案两层兜底：`domain_attributes.churn_reason` 有 → 按原因精准再营销；无 → 定期价值唤醒（planner/mod.rs:2402-2416）。
- subject=`dormant:[stage_updated_ms|legacy, predecessor_hex|genesis]`（planner/mod.rs:2425-2438）；与 silent 段边界：silent 通用沉默唤醒，reactivation 专扫被 TERMINAL_STAGES 排除出 stage 段的休眠老客（planner/mod.rs:2286-2288）。

### 2.17 `src/proactive_outreach.rs`（742 行）

**定位**（proactive_outreach.rs:1-6）：候选挑选与文案归 Planner/Agent；本模块只把"已接受的业务意图"在数据库侧线性化——**一个确定性任务 + 一条审计事件 + 一次当日配额预留，三者单 MongoDB 事务提交**。

- 常量：`MAX_TRANSACTION_ATTEMPTS=12`、`INTENT_HASH_FIELD="proactive_intent_hash"`、`QUOTA_RETENTION_DAYS=8`（proactive_outreach.rs:20-22）。
- `CommitOutcome`：Emitted / Duplicate / Capped（proactive_outreach.rs:24-29）。
- `DailyQuota`（proactive_outreach.rs:31-45）：namespace、account_scope（Some=账号桶 / None=workspace 桶）、total_cap、segment_cap、initial_total/initial_segment（**滚动部署基线**：预留时把桶单调抬到 legacy 事件日志观测值，旧进程后续 emit 不会让持久配额低计，proactive_outreach.rs:39-44）。
- 身份构造：`intent_identity` = sha256("proactive-follow-up:v1", ws, acct, wxid, segment, subject) → **ObjectId 取哈希前 12 字节** + hex 全串（proactive_outreach.rs:61-78, 120-130）——task 与 event **共用同一个确定性 _id**（build_task/build_event 均传 task_id，proactive_outreach.rs:143-184, 282-311）。
- 校验：`validate_identity` 拒空/首尾空白/NUL；`validate_token`（segment、namespace）额外拒 `.` 与 `$` 前缀（segment 拼进动态字段路径 `segments.<segment>`，防字段路径注入；workspace/account 保留含 `.`/`$` 的既有能力，proactive_outreach.rs:85-118）。
- **`ensure_and_reserve_quota`**（proactive_outreach.rs:186-266）：`total_cap<=0 || segment_cap<=0` → false；`quota_id`=sha256(namespace, ws, account|"*", utc_day)（proactive_outreach.rs:132-141）；三步：① `$setOnInsert` 建桶（含基线、`expires_at`=+8 天）；② **`$max` 单调对齐基线**（每次预留都做，不只 insert 时；也为共享桶补上首见 segment；新协议事件已同时反映在事件日志与桶中，取 max 不会重复计数，proactive_outreach.rs:229-244）；③ 条件 `$inc`（filter：`total < total_cap` 且 `segments.<seg> < segment_cap`），matched==1 即预留成功。
- **`commit_follow_up_once`**（proactive_outreach.rs:268-330）事务内顺序（注释 277-281 说明关键性）：
  1. **先 insert task（确定性 _id + intent hash 字段）**——身份先于配额：另一扫描器已拥有该 intent 时 dup key 中止本事务且**不消耗预留**；同 intent 竞争败者在胜者占满最后一格配额后仍被归类 Duplicate 而非 Capped。
  2. `ensure_and_reserve_quota` 失败 → 返回 Capped → **abort 事务**（task insert 一并回滚，proactive_outreach.rs:316-319）。
  3. insert event（同 _id）→ commit（`UnknownTransactionCommitResult` 循环重试；其它错误 abort，proactive_outreach.rs:426-437）。
- **`committed_identity_matches`**（proactive_outreach.rs:332-386）：**snapshot read concern 单事务**同时读 agent_tasks + agent_events 两表同 _id（两文档单事务提交，独立读可能跨在胜者 commit 两侧制造假 1/2 观测）；`classify_committed_identity_presence`：0=未提交、2=已提交、1/3=报错 partial commit（fail-closed，proactive_outreach.rs:380-386）；hash 不匹配 → 报 ObjectId collision。
- **`commit_follow_up`**（proactive_outreach.rs:439-469）主循环：校验 → 预查 Duplicate → 12 次尝试；每次 Err 后**先重查 committed identity**（胜者已提交 → Duplicate），再判 `retryable`（TransientTransactionError 或 dup key 11000/11001——含 mongodb 2.8.x 把事务内身份碰撞浮为 Command error 的形态，proactive_outreach.rs:388-417）+ `retry_delay` 5ms<<n 封顶 320ms（proactive_outreach.rs:419-424）。
- **signal 版本**：`signal_identity`=sha256("proactive-signal:v1", ws, acct, dedupe_key)（proactive_outreach.rs:471-479）；`signal_exists` 用 `$or(_id, 业务三元组)` 查重，业务身份同且 hash 兼容 → true，否则报 collision（proactive_outreach.rs:481-518）；`commit_signal_once` 同款"身份先于配额"事务（proactive_outreach.rs:520-573）；`commit_signal_with_daily_quota` 同款 12 次循环（proactive_outreach.rs:575-606）。

### 2.18 `src/cold_contact_worker.rs`（640 行）

- 语义边界（cold_contact_worker.rs:1-20）：silent 段=用户来过但停说话（last_inbound 旧）；**cold 段=agent 自己很久没出站**（last_outbound 旧）。不绕 gateway：只选 contact + 写任务，发送走 tasks worker → `handle_follow_up_task` → outbox → MCP。
- **`run_cold_contact_worker`**（cold_contact_worker.rs:36-70）：flag 关 → return；周期复用 `strategic_planner_interval_seconds.max(60)`；`representative_workspace_scopes` 每 workspace 一个代表 scope（audit 锚点）。
- **`scan_cold_outbound`**（cold_contact_worker.rs:81-222)：
  - S4：workspace 级扫描（旧实现锁 default_account_id，多账号下其它 account 的 cold contact 永不被激活）；emit 按 contact 自带 account_id 粘性绑定（cold_contact_worker.rs:94-98）。
  - `cold_candidate_filter_workspace`：workspace + managed + `last_outbound_at < now - cold_contact_threshold_hours` + 非冷却（cold_contact_worker.rs:244-258）。
  - 内存 `cold_candidate_passes_in_memory`（cold_contact_worker.rs:262-280）：managed、必须有 last_outbound、`last_inbound > last_outbound` 排除（属 silent 语义）、冷却排除。
  - pending 检查失败 → failed+1 continue（不中断扫描，cold_contact_worker.rs:115-127）。
  - 每候选调一次 `account_scheduler::assign_account`（结果丢弃 `let _ =`，纯审计流，cold_contact_worker.rs:128-137；见 §5 疑点 10）。
  - **钩子选择**：`load_peer_case_hooks` 按 account 惰性缓存（HashMap，cold_contact_worker.rs:104, 139-144）——`operation_knowledge_chunks` 中 `chunk_type="peer_case"` + `status ∈ [active, approved]` + **`integrity_status="verified"`（①-b 红线：对客推送文案必须内容已核实，启用门+内容门双门，cold_contact_worker.rs:366-390）** + account_id null 或匹配，取 summary 非空文本；`pick_hook` 按 contact_wxid DefaultHasher 稳定散列挑一条（同 contact 同池恒同 hook，池变自然轮换，不依赖 rand，cold_contact_worker.rs:403-415）。
  - content：有 hook → `"Planner: cold_reactivation since {ts} | hook={text}"`；无 → 退化基础文案（cold_contact_worker.rs:152-157）。
  - `commit_follow_up`：segment="cold_contact"、subject=`last-outbound:{ms|never}`、kind=`cold_contact_emit`、quota namespace="cold_contact" **account_scope=None（workspace 级桶）** cap=`cold_contact_daily_emit_cap`、initial 基线=当日 `cold_contact_emit` 事件数（cold_contact_worker.rs:158-187, 296-317）。
  - Emitted → emitted+1；Duplicate → continue；**Capped → break**；Err → failed+1 warn continue（cold_contact_worker.rs:188-200）。段尾 `cold_contact_tick` 事件（cold_contact_worker.rs:203-221）。
- **`decide_cold_emit`** 纯函数 7 态枚举（PBT 用：NotManaged/NeverOutbound/NotCold/UserRecentlyReplied/OnCooldown/AlreadyPending/Emit，判定顺序即列举顺序，cold_contact_worker.rs:417-476）。

### 2.19 `src/silence_signal_worker.rs`（427 行）

- 语义（silence_signal_worker.rs:1-19）：与 cold worker 互补——cold **发**重激活 follow_up；本 worker **只落 `censored=true` 沉默信号，绝不发消息**。Iron Law ②：沉默=删失不是负例；本阶段无下游消费，只铺删失形状。
- **`run_silence_signal_worker`**（silence_signal_worker.rs:33-69）：flag 关**或 interval==0** → return；周期 `silence_signal_interval_seconds.max(60)`；workspace 代表 scope。
- **`scan_silence`**（silence_signal_worker.rs:80-178）：
  - 粗筛 `silence_candidate_filter`：managed + `last_outbound_at < now - silence_threshold_seconds`（**无冷却条件**——只观察不打扰，与 cold filter 不同，silence_signal_worker.rs:222-228）。
  - **`decide_silence_signal`**（纯函数，silence_signal_worker.rs:239-258）：managed、有 outbound 且距 now ≥ threshold、**outbound 后无更新 inbound**（`last_inbound > outbound` → 不沉默 false；相等或更早仍算沉默——测试 silence_signal_worker.rs:398-405）。
  - `behavior_signals::build_silence` → `commit_signal_with_daily_quota`（namespace="silence_signal"、workspace 桶、cap=`silence_signal_daily_cap`、initial=当日 signal_type="silence" 的 behavior_signals 计数——注意基线数据源是 behavior_signals 表而非事件表，silence_signal_worker.rs:107-130, 198-217）。
  - 三态回报 `record_signal_metric`：Emitted→Ok(true)、Duplicate→Ok(false)、Err→Err（P3 健康度）；**Capped → break**（silence_signal_worker.rs:131-155）。段尾 `silence_signal_tick` 事件。
  - 幂等根基：`dedupe_key="silence:{wxid}:{last_outbound_ms}"` + partial unique 索引——同一条 outbound 只产一次删失，与 tick 节奏解耦（silence_signal_worker.rs:16-17；key 构造在 behavior_signals.rs:48-50）。

### 2.20 `src/management_worker.rs`（186 行）

- 语义（management_worker.rs:1-5）：管理命令执行的崩溃恢复。进程可能在取得命令租约后、工具结果最终化前死亡；**外部副作用重放不安全 → stale 执行收敛到 `execution_unknown`**（不重试不重放）。
- 常量：租约 5 分钟、扫描间隔 60s、批量 100（management_worker.rs:16-18）。
- `management_command_sweeper_loop`：无条件循环（main.rs:233-239 无 gate flag 直接 spawn），失败仅 error log（management_worker.rs:20-27）。
- **`sweep_stale_management_commands`**（management_worker.rs:29-56）：filter=`status="running" && execution_started_at <= now-5min && execution_token $type string`（management_worker.rs:62-68），按 started_at 升序限 100，逐条恢复；recovered>0 时 warn 汇总。
- **`recover_one_stale_command`**（management_worker.rs:70-149）：run 缺 id/token/started_at 任一 → Ok(false)；事务内：① `command_runs` OCC 全匹配（_id+scope+status=running+token+started_at）置 `status="execution_unknown"` + 中文 summary（"执行租约过期，结果未知；为避免重复副作用，系统不会自动重放"）+ error=`management_execution_lease_expired` + `$unset` token/started_at；matched!=1（持有者已自行完成或另一 sweeper 赢了）→ abort 返回 false；② 同事务把该 run 下 `tool_calls` 中 `status="executing"` 的行 update_many 为 `execution_unknown` + finalized_at；③ commit。错误 → abort + 上抛。

### 2.21 `src/account_scheduler.rs`（507 行）

- 原则（account_scheduler.rs:1-21）：粘性优先（已绑定 account 直接复用）、persona 内按 wxid 稳定散列轮询、off_hours 跳过（全命中时 fallback 到任一 online——保送达 > 严格遵守）、capacity 上限（0=不参与、永远未满）；零侵入 webhook、不绕 outbox、每次调度写审计事件。
- **`list_registered_account_scopes`**（account_scheduler.rs:59-69）：全量 accounts 表 → `normalize_account_scopes`（拒空 id、排序、去重，account_scheduler.rs:39-52）。**故意不过滤 online**：离线账号仍可能拥有 managed contacts 与持久任务，投递侧再管连接性（account_scheduler.rs:54-58）。演化器/planner/cold/silence 四个 worker 的 scope 枚举都走这里。
- **`representative_workspace_scopes`**（account_scheduler.rs:74-85）：已排序 scopes 压缩为每 workspace 首个（workspace 级 worker 只拿它当审计锚点，候选工作仍按 contact 自身 account 走）。
- **`assign_account`**（account_scheduler.rs:91-171）：`load_persona_pool`（有 tag 过滤，无则全 workspace，account_scheduler.rs:173-187）；空 → None；`count_today_assignments`（agent_events kind=`account_scheduler_assignment` 当日 $group 聚合，account_scheduler.rs:189-219）；**off_hours 按运营方时区小时**（`quiet_hours::hour_in_offset(now, strategic_planner_calendar_tz_offset_hours)`——容器 UTC 会偏 8 小时的旧 bug 已修，回归测试锁定 account_scheduler.rs:390-417）；eligible 过滤 online && !off_hours && (capacity==0 || used<capacity)；空 → 退化 online-only；仍空 → None；`stable_pick` DefaultHasher(wxid)%len；写审计事件（best-effort `let _ =`）。
- `hour_in_range`（account_scheduler.rs:225-238）：start==end 恒 false；start<end 同日 `[start,end)`；start>end 跨午夜 `[start,24)∪[0,end)`。
- **`decide_assigned_account`** 纯函数版（account_scheduler.rs:248-305）：与 assign_account 同源决策不碰 mongo，4 不变量注释（capacity 全满仍保送达绝不 None、同输入决策稳定、capacity=0 视为不限、off_hours 严格池排除仅逼不得已才退化）。

### 2.22 `src/behavior_signals.rs`（580 行）

- 定位（behavior_signals.rs:1-17）：T1 行为信号采集底座，只落"系统客观观察到的量"，**不解释不评分不喂学习公式**（解释层在 reaction_analysis，物理隔离——Iron Law ③）；元数据 `source="system_observed"` / `confidence=1.0` / dedupe_key（Iron Law ④）；dedupe_key + partial unique 索引幂等（Iron Law ⑤）。
- 4 类信号与 dedupe key（behavior_signals.rs:25-50）：`reply_latency:{wxid}:{msg_id}`、`reply_length:{wxid}:{msg_id}`、`reactivation:{wxid}:{msg_id}`、`silence:{wxid}:{outbound_ms}`；`REACTIVATION_THRESHOLD_MS` = 7 天（behavior_signals.rs:29）。
- **`build_reply_latency`**（behavior_signals.rs:57-92）：无 last_outbound 基准 → `latency_ms=None`（不臆造 0）；负 delta（时钟/乱序）→ None；0 合法（同毫秒极速回复）。
- **`build_reply_length`**（behavior_signals.rs:96-123)：`chars().count()` 多字节安全（中文/emoji 按字符不按字节，测试 behavior_signals.rs:401-406）。
- **`is_reactivation`**（behavior_signals.rs:127-136）：上条 inbound 距本条 **>= 阈值**（边界含）；首条 inbound（None）不算——属"新建首次触达"。`build_reactivation`（behavior_signals.rs:139-165）。
- **`build_silence`**（behavior_signals.rs:170-197）：**恒 `censored=true`** + `unanswered=Some(true)`；`silence_ms` clamp 非负。
- **`persist_signal`**（behavior_signals.rs:204-210）：insert_one；dup key 11000/11001 → `Ok(false)` 幂等跳过；其它透传。调用方 best-effort 仅 warn 绝不影响主应答链路（behavior_signals.rs:201-203）。**落库必须 snake_case**——partial 索引按 `dedupe_key` 匹配，camelCase 会让 unique 失效（回归测试锁死，behavior_signals.rs:303-340）。
- **`record_signal_metric`**（behavior_signals.rs:219-260）：flag `behavior_signal_metrics_enabled` 关（默认）→ return；`_id="{ws}:{YYYY-MM-DD}"` upsert `$inc`（三态 → persisted/dedupe_skipped/errors，behavior_signals.rs:276-282）；仅 Ok(true) 刷 `last_success_at` 新鲜度；自身失败仅 warn。
- 双时间戳：`observed_at`=event_time（调用方传入不被篡改）+ `ingest_time`=落库时刻（四 builder 都填；旧文档缺字段反序列化回落 None——R11，behavior_signals.rs:491-562）。
- 调用方亲验：webhooks.rs:1743-1776（入站侧 build_reply_latency/length/reactivation + persist_signal）；silence_signal_worker.rs:107（build_silence）。

### 2.23 `src/bin/migrate_only.rs`（41 行）

- 显式迁移维护入口：**从不加载 .env、不起 HTTP、不启 worker**（migrate_only.rs:1-5）。
- 必填参数 `--uri= / --database= / --confirm=`（`required_arg` 拒空值，migrate_only.rs:13-20）；拒系统库 admin/config/local（migrate_only.rs:11, 28-30）；confirm 必须逐字等于 `migrate-only:{database}`（migrate_only.rs:31-34）。
- 执行序与 main 一致：`Database::connect` → `migrations::run` → `ensure_indexes`（迁移先于索引——部分迁移重建集合，migrate_only.rs:36-38）。

---

## 3. 跨文件机制

### 3.1 演化候选：从生成到发布/回滚的完整旅程

1. **入口**：main.rs:287-289 无条件 spawn `run_evolutionary_worker`；env `EVOLUTION_ENABLED=false` 进函数即 return（mod.rs:55-60）。
2. **tick 准入**：每 tick per-scope 写 envelope（status=collecting）→ 读 mongo runtime flag；flag 缺失/关/0% → cohort 恒空（cohort.rs:74-76），worker 跑"空 tick"仅留观察痕迹。灰度分桶 `hash(contact)%100 < percent` 单调稳定（runtime_flag.rs:51-68）。
3. **候选生成**：threshold 纯统计（cohort 命中率 vs BANDS 区间外 → ±1 步候选，≤4 条 pending_eval，cooldown/超额留痕拒绝）；prompt 走 Critic LLM（Background 优先级 + EvolutionBudget 记账 + 四道闸全或无 drop + 目标 key 白名单 + base_revision 冻结）。两路 proposal 均带 `base_revision` 不可变 token（threshold=source_id+f64 bits；prompt=template_id+version+sha256），把评估证据绑到确切基线工件（revision.rs:1-4）。
4. **shadow 评估**：`replay::eval_all` 对每 (pending_eval proposal × cohort run) 起受限并发 task；threshold=纯重判（KE-02 两侧同用 5 闸口径推终态）；prompt=经 `agent::prompt_shadow::shadow_replay_prompt_one` 跑真实 Reply+Review 演练（隔离桥，白名单允许，mod.rs:553-558）——**shadow 的 LLM 消耗用 per-replay 的 RunBudget（simulation 预算，prompt_shadow.rs:369-375 亲验），不回填 tick 级 EvolutionBudget**（见 §5 疑点 4）。所有结果只落 `shadow_replays`。
5. **显著性分级**：`aggregate_and_grade` 逐 proposal 聚合 replays → threshold 走三门合取（send_delta ≥ 0.05 && 5 闸涨幅 ≤ 0.10 && 安全回归率 ≤ 0.0）+ 两道 early_reject（completed ≥ 30、fail_rate ≤ 0.30）；prompt 走"completed ≥ 1 即 eligible"证据制。passed → `eligible_for_release`，否则 `rejected_below_threshold`。envelope 推 `awaiting_admin`。
6. **发布（admin）**：routes/evolution.rs:271-291 → `release_threshold` / `release_prompt`。共同骨架：release 闸（env+mongo flag）→ 前置校验（kind/status/base_revision 解析+bit 级/sha 级一致）→ **mongo 事务**内 OCC 退役旧工件 + insert 新工件 + proposal→released + audit 行（threshold）+ 观察事件 + `post_release_reviews` 排期，全部 commit 前完成 → commit（UnknownTransactionCommitResult 重试）。prompt 额外过红线三闸（禁词+锚点 `validate_prompt_edit`、LLM 语义 `review_prompt_edit`，LLM 不可用即中止不放水）且 commit 后 bump `prompt_pack_version` 失效 LRU。
7. **发布后**：+24h 到期由下一个 tick 的 `post_release::run_due_reviews` 计算 BEFORE/AFTER 两窗 delta（send_success / 5gate / 负反应）——**纯观测不判决不回滚**（Req 9.7）。
8. **自动放行接点（休眠）**：tick 末尾 `auto_release_eligible_thresholds`，`CURRENT_AUTO_RELEASE_POLICY_ENABLED=false` 编译期恒关；若未来开启需同过 env 闸 + workspace 子闸 + KE-01 方向门 + 负反应强制门，且 admin id 固定 `"evolution_auto_release"`、只覆盖 threshold、rollback 永远人工。
9. **回滚（admin）**：routes/evolution.rs:329-345 → `rollback_threshold` / `rollback_prompt`。不过 release 闸；事务内 OCC 校验"该 proposal 仍拥有 current 产物且 revision 未漂移"→ 退役产物 + 恢复冻结基线行 + proposal→rolled_back（+audit/事件）。生效即时：threshold 读端过滤 `rolled_back_at=null`；prompt bump pack version。
10. **生效面**：threshold override 经 `agent::runtime::resolve_thresholds` 进 5 闸与 planner block-rate（planner/mod.rs:1413-1418 亲验 planner 侧消费）；prompt 新版本经 `current_version=true` + pack version bump 被 `load_prompt` 读到。

**proposal 状态机（代码中实际出现的闭集）**：`pending_eval`（threshold.rs:216 / prompt_critic.rs:291）→ `eligible_for_release` | `rejected_below_threshold`（significance.rs:558-564；生成期直接拒绝也用后者：cooldown/quota/drop/base 缺失）→ `released`（release.rs:374, 661）→ `rolled_back`（release.rs:884, 1163）。

### 3.2 主动触达任务：从扫描到 gateway 的旅程

1. **扫描**（三个来源）：planner 六段（emit `follow_up`，subject 是业务代事实）；cold worker（emit `follow_up`，subject=`last-outbound:{ms}`）；silence worker（**不产任务**，只落删失信号）。
2. **过滤漏斗**（以 planner 段为模板）：DB 粗筛（managed/时间阈值/冷却/终态）→ 内存精筛（last_outbound vs last_inbound 语义、范式 enabled、override 阈值）→ pending follow_up 幂等预筛 → block-rate 反馈环（backoff 不耗 cap）→ 优先级排序（commitment/stage 段）→ cap 余额判定。
3. **提交边界**（proactive_outreach）：`commit_follow_up` 把 (确定性 task + 审计事件 + 配额预留) 绑成单事务；确定性 _id 由 (ws, acct, wxid, segment, subject) 哈希导出——**同一业务代永远同一 _id**，跨进程/跨 tick/并发扫描全部幂等；Duplicate/Capped/Emitted 三态回传扫描器（Duplicate continue、Capped break）。
4. **执行**：落库的 `agent_tasks`（kind=follow_up、status=pending、expires_at=+48h、review_required=true、max_attempts=3，proactive_outreach.rs:143-168）由 tasks.rs 的 task worker 拉起 → `agent::handle_follow_up_task` → 标准 gateway（决策→独立 Review→outbox→MCP）。planner/cold 模块自身绝不直接调 MCP（planner/mod.rs:4-6；cold_contact_worker.rs:19-20）。
5. **配额一致性**：段内 remaining 计数只是近似预筛；**硬保证在 quota 桶的条件 $inc**（total/segment 双维、$max 基线对齐滚动部署）；calendar/renewal/reactivation 双 cap（段 cap 经 segment_cap 进事务）。
6. **审计闭环**：每次 emit 一条 kind 专属事件（进 EMIT_EVENT_KINDS 供次日 cap 反查）；capped/backoff/tick 汇总事件不进 cap 计数。

---

## 4. 事实卡速查

**Proposal.status 闭集（写入侧亲验）**
| status | 写入点 |
|---|---|
| `pending_eval` | threshold.rs:216、prompt_critic.rs:291 |
| `rejected_below_threshold` | threshold.rs:211,213（cooldown/quota）、prompt_critic.rs:295,307,519（quota/base 缺失/drop）、significance.rs:563（评估拒） |
| `eligible_for_release` | significance.rs:560 |
| `released` | release.rs:374（threshold）、release.rs:661（prompt） |
| `rolled_back` | release.rs:884（threshold）、release.rs:1163（prompt） |

**experiments.status 闭集**：`collecting / evaluating / awaiting_admin / released / aborted`（envelope.rs:60-63）；生产 tick 实际只走 collecting → awaiting_admin（mod.rs:263；`evaluating` 合法但当前不被 tick 使用）。

**显著性三门数值（threshold 候选，SignificanceCfg 从 config，significance.rs:84-92）**
| 门 | 条件 | reject reason |
|---|---|---|
| 前置 1 | completed ≥ `evolution_min_replays`（基线 30） | `insufficient_completed_replays` |
| 前置 2 | failed/total ≤ `evolution_replay_max_fail_rate`（0.30） | `replay_fail_rate_above_threshold` |
| 门 1 send | send_delta ≥ `evolution_min_send_success_delta`（0.05） | `send_success_delta_below_threshold` |
| 门 2 gate | max 5 闸涨幅 ≤ `evolution_max_5gate_hit_increase`（0.10，边界含） | `gate_hit_increase_above_threshold` |
| 门 3 safety | 安全回归率 ≤ `evolution_max_safety_regression_rate`（0.0 零容忍；仅安全闸 gate_key 生效） | `safety_gate_regression_above_threshold`（reason 优先级最高） |
| NaN 防御 | 任意 NaN | `nan_in_metrics` |

prompt 候选：completed==0 → `no_completed_replays`；completed≥1 → eligible（一切数值仅 `_observed` 证据，significance.rs:229-308）。

**`SEND_SUCCESS_STATUSES`** = `["approved", "revision_applied_approved"]`（significance.rs:42）——send-success 口径唯一源；threshold replay 的 original/new 终态都由 5 闸重推后按此判成功（KE-02，replay.rs:325-346）；post_release 的 APPROVED_LIKE 同两值（post_release.rs:36）。

**安全闸映射（#152 权威，significance.rs:52-59）**：fact→held_by_ai_policy、pressure→blocked_by_safety_guard、product→blocked_unverified_product_claim；rewrite 两闸与 planner 非安全闸。

**各 worker gate flag / 间隔 / 配额**（默认值以 budget.rs:59-197 测试构造样值 + .env 注释为参考，权威在 src/config.rs）
| worker | gate | 间隔 | 配额/上限 | spawn 方式 |
|---|---|---|---|---|
| evolution | env `EVOLUTION_ENABLED`（硬）+ mongo flag（enabled+rollout_percent） | `evolution_tick_seconds.max(60)`（样值 600） | EvolutionBudget（token+calls）；threshold/prompt 各 ≤4 条/tick；cohort per-contact ≤3；auto_release ≤1/tick（政策恒关） | 无条件 spawn、函数内检（main.rs:287-289） |
| strategic_planner | `strategic_planner_enabled` | `strategic_planner_interval_seconds`（样值 600） | 跨段共享 `strategic_planner_daily_emit_cap`（样值 20）；calendar/renewal/reactivation 各自段 cap（样值 3）；quota namespace=`strategic_planner`（account 桶） | **条件 spawn**（main.rs:263-267） |
| cold_contact | `cold_contact_worker_enabled` | 复用 planner 间隔 max(60) | `cold_contact_daily_emit_cap`（样值 5）；阈值 `cold_contact_threshold_hours`（样值 168）；namespace=`cold_contact`（workspace 桶） | 无条件 spawn、函数内检（main.rs:272-274） |
| silence_signal | `silence_signal_worker_enabled` 且 interval≠0 | `silence_signal_interval_seconds.max(60)` | `silence_signal_daily_cap`（样值 500）；阈值 `silence_threshold_seconds`（样值 86400）；namespace=`silence_signal`（workspace 桶） | 无条件 spawn、函数内检（main.rs:280-282） |
| management_sweeper | 无 flag（常开） | 60s 硬编码 | 租约 5min、批量 100/次 | 无条件 spawn（main.rs:233-239） |

**runtime_flag 分桶规则**（runtime_flag.rs:51-68 + models.rs:1661-1666）：`DefaultHasher(contact_id) % 100 < rollout_percent.min(100)`；enabled=false 一票否决；文档缺失=不灰度；同 contact 跨 percent 单调稳定（5%→20% 不退出）；跨进程一致（同进程内 BuildHasher 一致性），跨版本不保证（灰度不依赖）。

**其它高频事实**
- 演化器 5 闸默认阈值（两处硬常量同值）：fact 6 / pressure 7 / human 6 / emotional 6 / product 7（threshold.rs:294-304、replay.rs:380-389）。
- gate 命中方向：fact/pressure=GTE（分高危险）；human/emotional/product=LT（分低触发）（replay.rs:42-52）。
- prompt 红线闸链（release_prompt）：compose 末尾追加 → validate（禁词+锚点）→ LLM 语义审查（Reject 与 NeedsHumanConfirm 均中止）（release.rs:539-569）。
- 主动触达 intent 幂等键：sha256("proactive-follow-up:v1", ws, acct, wxid, segment, subject) 前 12 字节作 _id（proactive_outreach.rs:120-130）。
- 沉默删失恒 censored=true、silence dedupe 按 outbound 毫秒（behavior_signals.rs:170-197）。
- silent vs cold vs silence 三语义分界：silent=inbound 旧且 outbound<inbound（用户停说话）；cold=outbound 旧且 inbound≤outbound（AI 冷链路）；silence=outbound 旧且其后无 inbound（只记删失不发）。

---

## 5. 偏差与疑点

1. **`schedule_post_release_review` 成为死代码 + 模块注释过时**：post_release.rs:3 声称"每次 release 后由 release.rs 调 schedule_post_release_review"，但亲验全仓无任何调用方（grep 全 *.rs 仅定义与注释）；生产路径是 release.rs:112-128 在 release 事务内直接 insert `post_release_review_document`（事务化是 #155 后的改进，函数注释 post_release.rs:74-75"不参与 release transaction"描述的是旧兼容路径）。
2. **`is_evolution_enabled_for`（runtime_flag.rs:80-101）无生产调用方**：仅 mod.rs:46 re-export、db/mod.rs:353 注释提及。注释设想的"worker / webhook / shadow 三路调用方"（runtime_flag.rs:48）中，目前只有 cohort 过滤（经 `bucket_for_contact`）真实生效；webhook/gateway 侧没有接灰度判定。灰度的实际语义=“只影响哪些 contact 的 run 进演化 cohort”，不影响生产回复行为，符合隔离定位但与注释的三路设想有差距。
3. **rewrite 闸命中口径两处不一致**：threshold::generate 对 `revision_applied=true` 给 human_like/emotional 各 **+0.5**（threshold.rs:114-123，理由是 run log 未记录具体哪个闸触发）；auto_release::compute_window_gate_hit_rates 给两闸各 **+1**（auto_release.rs:333-336），而其注释声称"与 threshold::generate 内的口径一致"（auto_release.rs:287-289）。auto_release 的 rewrite 命中率因此是生成侧口径的两倍。当前政策硬闸恒关无实际影响，但若未来开启会造成 band 判定不对齐。
4. **prompt shadow 的 LLM 消耗不计入 EvolutionBudget**：mod.rs:229-231 注释仍说"replay 现阶段 threshold 不调 LLM、prompt 走 placeholder failed，所以这里不会再触发 BudgetExceeded"，但 replay.rs:9-13, 243-255 已接真实 `shadow_replay_prompt_one`。replay 的 tokio task 无法携带 `&mut EvolutionBudget`（replay.rs:107-109 自述），实际 shadow LLM 开销由 per-replay 的 `RunBudget`（`runtime.simulation_token_budget` / `run_max_llm_calls`，prompt_shadow.rs:369-375 亲验）+ `llm_concurrency` Background 优先级 + replay 并发信号量约束。后果：envelope 的 `budget_used_tokens` 不含 shadow 消耗，`evolution_run_token_budget` 实际只约束 Critic 一次调用；tick 级预算对 W3 是"占位检查"（replay.rs:125-128 注释承认占位）。mod.rs 注释过时。
5. **post_release 三 gate 同值观测**：FIVE_GATE_KEYS 里 pressure/human_like/emotional 三个 gate 都映射 `revision_failed`（post_release.rs:60-69），compute_window_metrics 对它们跑的是同一查询——三条 delta 恒相等，观测分辨率有限（[2-01] 注释解释 pressure 是有意偏离，但 human/emotional 与 pressure 的三重合并未被注释直接说明）。另有跨模块语义分歧：threshold.rs:69 与 auto_release.rs:328-330 都把 `blocked_by_safety_guard` 记为 pressure 命中（significance 权威同），而 post_release [2-01] 注释断言"blocked_by_safety_guard 来自产品声明 fail-closed/relay，与 fact/pressure 无关"——同一 gate 在生成/评估侧与发布后观测侧口径不同，读不出哪侧才反映生产真实（记疑点）。
6. **cohort.rs 注释描述不可达分支**：cohort.rs:113-115 说"空 contact_wxid 视为'无 contact'分组下的一个自然 contact 组"，但 `contact_in_runtime_cohort`（cohort.rs:141）已要求 contact 非空——空 contact run 进不了 threshold_pool，dedup 的空 contact 分支实际到不了（仅单测直接调 dedup 时可达，cohort.rs:196-205）。
7. **release cooldown 查询不排除已回滚 release**：release.rs:273-285 的 cooldown count 按 `released_at >= since` 全算，不带 `rolled_back_at: null`——与 threshold.rs:256-289 生成侧 cooldown（带 `current_version: true + rolled_back_at: null`）口径不同。后果：release 后立即 rollback，同 gate 在 cooldown 窗内无法再次 release（生成侧却可能继续产 pending 候选）。偏保守方向，疑似有意（防抖动），但两处口径未对齐、无注释说明。
8. **EVOLVABLE 白名单双定义**：`prompt_critic.rs:55-61 EVOLVABLE_PROMPT_TARGETS` 与 `revision.rs:26-32 EVOLVABLE_PROMPT_KEYS` 内容相同的两份常量，无编译期同步护栏（若只改一处会漂移）。
9. **`threshold.rs` 字段命名误导**：Candidate.`proposed_raw`（threshold.rs:179）存的是 `decide_candidate` 返回的 **clamp 后**值（threshold.rs:165-167），`cohort_notes.clamped_to_value`（threshold.rs:207-209）记录的也是同一 clamp 后值——行为正确（proposal.proposed_value 就该是最终值），但"raw"名与"clamped_to_value"语义反着读。
10. **cold worker 每 tick 重复写 assignment 审计**：scan_cold_outbound 对每个通过内存筛的 cold candidate 每 tick 调一次 `assign_account`（结果弃用，cold_contact_worker.rs:131-137）。assignment 事件会被 `count_today_assignments`（account_scheduler.rs:189-219）计入 capacity 消耗——冷扫描的审计流会虚耗调度器的当日 capacity 计数（capacity 语义本就是"当日新分配数"而非总持有，account_scheduler.rs:107 注释，但 cold 审计与真实新分配混在同一计数里）。
11. **planner N+1 形态**：六段各自独立 `count_today_emit_events`、每候选一次 `has_pending_follow_up` + `should_skip_for_block_rate`（后者含 `resolve_thresholds` + 扫窗口 run logs）——contact 量大时 tick 内查询量线性放大。硬一致性由 quota 事务兜底，性能是已知取舍（未见注释明示）。
12. **mod.rs:64 日志文案过时**："M4 W1 skeleton — empty tick by design" 仍在 worker 启动 info 里，实际 tick 已是 W4 全功能。
13. **planner 排序键 i32 取负的理论边界**：`-stage_w` 等若权重为 `i32::MIN` 会溢出（planner/mod.rs:1346-1348, 1370-1371）；权重来源是字典 i32 或写死常量，实际不可能，纯理论备注。
14. **`stage_stagnation_passes_in_memory` 的 `let _ = now;`**（planner/mod.rs:1253）：入参 now 仅在该分支显式弃用，函数签名保留 now 但真实冷却判定用 `DateTime::now()`（managed_and_not_in_cooldown，planner/mod.rs:1261-1271）——测试注入固定 now 时冷却分支仍读真实时钟，可测性弱点。
15. **grade_prompt 的证据门槛与 threshold 差异巨大**（1 条 completed vs 30 条 + 失败率门）：阶段二有意设计（significance.rs:212-228 注释明示"管理员把关"），不是 bug，但 release 决策质量完全依赖管理员阅读 per_sample_evidence。

---

## 6. 覆盖自证

以下 23 个文件全部逐行读完（分段 Read 全文，无跳读）；行数与 `wc -l` 输出一致：

| # | 文件 | 行数 | 读取方式 |
|---|---|---|---|
| 1 | src/evolution/mod.rs | 627 | 全文一次 |
| 2 | src/evolution/error.rs | 25 | 全文一次 |
| 3 | src/evolution/envelope.rs | 117 | 全文一次 |
| 4 | src/evolution/lint.rs | 83 | 全文一次 |
| 5 | src/evolution/budget.rs | 237 | 全文一次 |
| 6 | src/evolution/cohort.rs | 270 | 全文一次 |
| 7 | src/evolution/runtime_flag.rs | 195 | 全文一次 |
| 8 | src/evolution/threshold.rs | 508 | 全文一次 |
| 9 | src/evolution/prompt_critic.rs | 700 | 全文一次 |
| 10 | src/evolution/replay.rs | 967 | 全文一次 |
| 11 | src/evolution/significance.rs | 1039 | 全文一次 |
| 12 | src/evolution/revision.rs | 142 | 全文一次 |
| 13 | src/evolution/release.rs | 1323 | 分 2 段（1-700, 700-1323） |
| 14 | src/evolution/auto_release.rs | 595 | 全文一次 |
| 15 | src/evolution/post_release.rs | 575 | 全文一次 |
| 16 | src/planner/mod.rs | 4301 | 分 5 段（1-800, 800-1600, 1600-2400, 2400-3199, 3199-4301） |
| 17 | src/proactive_outreach.rs | 742 | 全文一次 |
| 18 | src/cold_contact_worker.rs | 640 | 全文一次 |
| 19 | src/silence_signal_worker.rs | 427 | 全文一次 |
| 20 | src/management_worker.rs | 186 | 全文一次 |
| 21 | src/account_scheduler.rs | 507 | 全文一次 |
| 22 | src/behavior_signals.rs | 580 | 全文一次 |
| 23 | src/bin/migrate_only.rs | 41 | 全文一次 |

合计 14,827 行。另为核证跨文件断言，定点 grep/Read 了以下未列入必读清单的位置（仅验证，未通读）：main.rs:215-289（spawn 点）、routes/evolution.rs:34,271-345（release/rollback 调用方）、models.rs:1631-1666（EvolutionRuntimeFlag + rollout_percent_clamped）、agent/prompt_shadow.rs:101,294-375（shadow_replay_prompt_one 存在性与独立 RunBudget）、agent/runtime.rs:775（threshold_value_is_representable）、prompts.rs:2508（PROMPT_EVOLUTION_FORBIDDEN_KEYS）、prompt_guard.rs:75,132,142（三闸函数存在性）、agent/gateway.rs:7533 + agent/mod.rs:94,124（write_event_for_account / effective_memory_card re-export）、agent/quiet_hours.rs:46（hour_in_offset）、webhooks.rs:1743-1776（behavior_signals 调用方）、db/indexes 相关断言以 replay.rs:204-209 注释与 behavior_signals.rs 测试为据。

---

## 追记：28 号交叉验证回写（2026-08-13，主会话执行）

- **§2.13 补充**：`rollback_prompt` 恢复基线行时同时 `$set status:"active"`（`release.rs:1131-1134`），且 matched≠1 即中止事务——与 `evolution_rollback_status` 测试承诺一致（28 号裁决：测试与生产一致，原文遗漏 status 恢复细节）。
