# Design Document — agent-self-evolution

> 中文标题：用户运营 Agent 自我演化 — 技术设计文档
> 关联需求文档：`.kiro/specs/agent-self-evolution/requirements.md`（R1–R10）
> 关联上游 spec：`.kiro/specs/agent-autonomy-loop/`（已落地）+ `.kiro/specs/user-ops-agent-hardening/`（已落地）+ M3 Strategic Planner（已落地）
> 工作流：requirements-first / specType=feature

## 1. Overview

本设计把 requirements.md 的 R1–R10 映射到既有代码库（Rust + Axum + MongoDB + LLM + 现 React 前端），按"先骨架、后逻辑、先 shadow、后 release"的顺序拆成 5 波（W0–W4 + 收口）。设计原则与上游 spec 对齐：

- **不绕过 gateway / outbox / mcp**：演化器物理上是独立模块 `src/evolution/`，编译期 SHALL NOT 依赖 `src/agent/gateway / outbox / src/mcp`；CI lint 通过 `scripts/check-no-human-takeover.sh` + 新增 `scripts/check-evolution-isolation.sh` 双重保护。
- **只读生产，写自己 4 张表**：演化器读 `agent_run_logs / agent_outcome_metrics / agent_events / conversation_messages / agent_decision_reviews / agent_send_outbox / contacts`，写 `experiments / proposals / shadow_replays / threshold_overrides` 与 update `prompt_templates`（仅 release 时）。
- **演化是离线决策、生效是同步配置**：演化器产出"提案"是离线工作（异步 worker），生效是 admin 点 release 时一次原子操作（threshold 写一行 / prompt bump version + 失效 cache）；生产链路读阈值 / prompt 时**总是**通过统一 resolver。
- **shadow eval 是发布前置**：任何 release 前 shadow_replays 必须通过 R5 的显著性测试 + cohort 数下限。
- **AI-自治定位不可妥协**：Critic LLM、前端文案、新事件 kind SHALL 全程过禁词 lint。

## 2. Architecture

### 2.1 演化链路（高层）

```
[每 6h] evolutionary_worker_tick
        │
        ▼
[W1] insert experiment(envelope, status=collecting)
        │
        ▼
[W1] cohort selection
   ├─ threshold cohort: success+failure 混合 ≥ 30 条
   └─ prompt cohort:    failure-only ≥ 30 条
        │
        ▼
[W2] threshold proposals (纯统计，不调 LLM)
   └─ 6 个 gate × 命中率偏离 → 候选 ±step
        │
        ▼
[W2] prompt proposals (Critic LLM)
   └─ 失败 cohort 喂 Critic → 严格 JSON diff[]
        │
        ▼ status=evaluating
[W3] shadow_replays (并行, ≤ EVOLUTION_REPLAY_CONCURRENCY)
   └─ 每条候选 × 每条 cohort run
       └─ src/evolution/replay.rs::run_shadow_replay
           ├─ 读原 run 上下文 (短路 outbox/mcp)
           ├─ 用候选阈值 / 候选 prompt 调 Reply Agent → review
           └─ 写 shadow_replays 记录
        │
        ▼
[W3] aggregate + significance test
   ├─ pass  → proposal_status="eligible_for_release"
   └─ fail  → "rejected_below_threshold"
        │
        ▼ status=awaiting_admin
[W4] /api/evolution/* + EvolutionCenterTab
   └─ admin 点 RELEASE → 写 threshold_overrides 或 bump prompt_templates
        │
        ▼ status=released
[W4] +24h 自动 post_release_review (对比窗口评测)
        │
        ▼ (可选) admin 点 ROLLBACK → rolled_back
```

生产链路只在两处读演化产出：

1. `src/agent/runtime.rs::resolve_thresholds(state, contact)` —— 5 闸 + PlannerBlockRate 取值前先读 `threshold_overrides`。
2. `src/agent::generate_agent_json` —— 通过新增的 `prompt_pack_version` 复合 cache key 读最新 `current_version=true` 的 prompt。

### 2.2 模块边界

```
src/
├── evolution/                     # 本期新增，独立模块
│   ├── mod.rs                     # run_evolutionary_worker 入口 + tick 主循环
│   ├── budget.rs                  # EvolutionBudget（与 RunBudget 隔离）
│   ├── cohort.rs                  # cohort 选择 + 去重 + retention 检查
│   ├── threshold.rs               # 阈值候选生成 + 合理区间常量
│   ├── prompt_critic.rs           # Critic LLM 调用 + JSON schema 校验
│   ├── replay.rs                  # 模拟 gateway: run_shadow_replay
│   ├── significance.rs            # 显著性测试纯函数
│   ├── release.rs                 # release / rollback 原子操作
│   ├── post_release.rs            # +24h 对比窗口评测
│   └── lint.rs                    # 写入前禁词 lint（运行时）
├── routes/
│   └── evolution.rs               # 本期新增：/api/evolution/* 4 个只读 + 2 个写
├── agent/
│   └── runtime.rs                 # 扩 resolve_thresholds（读 threshold_overrides）
├── prompts.rs                     # 扩 ensure_evolution_prompt_pack_v1（seed Critic prompt）
├── db/
│   └── indexes.rs                 # 扩 4 张新 collection 的索引
└── main.rs                        # tokio::spawn(run_evolutionary_worker) 在 ensure_prompt_pack 之后
```

`src/evolution/` 顶部 `mod.rs` 加注释 anchor：

```rust
//! FORBIDDEN dependencies (CI-enforced):
//! - crate::agent::gateway / outbox / mcp
//! - crate::routes::* 中除 evolution.rs 外
//! - crate::tasks  (worker loop)
//! - crate::webhooks
```

### 2.3 实施波次（W0–W4 + 收口）

| 波次     | 名称                  | 主要内容                                                                                                                                                | 前置依赖     |
| ------ | ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| **W0** | 基础设施波               | 新建 4 张 collection accessor (`experiments / proposals / shadow_replays / threshold_overrides`) + 索引；新增 env / config 字段；CI lint `check-evolution-isolation.sh` | —        |
| **W1** | 骨架波                 | `src/evolution/mod.rs` worker 主循环 + EvolutionBudget + experiment 信封 + cohort 选择；空跑产 0 candidate 的 tick 也能写完整 envelope                                  | W0       |
| **W2** | 候选生成波               | threshold.rs（纯统计）+ prompt_critic.rs（Critic LLM）+ 写 `proposals` + `evolution_prompt_pack_v1` seed                                                         | W1       |
| **W3** | Shadow eval + 显著性波  | replay.rs（短路 outbox/mcp 的模拟 gateway）+ significance.rs + 聚合到 proposal                                                                                   | W2       |
| **W4** | Release + 前端 + 回滚波  | release.rs / post_release.rs / `routes/evolution.rs` / EvolutionCenterTab + tsc + vitest                                                              | W3       |
| **收口** | 监控、PBT、文档、CI 收口     | 4 PBT 不动 baseline；新增单测 / 集成测试；`docs/agent-policy.md` 增"自我演化"章节；CI lint 收紧                                                                              | W0–W4    |

## 3. 数据模型

### 3.1 新增 collection

#### 3.1.1 `experiments`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    #[serde(rename = "_id", default)]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub experiment_id: String,        // uuid v7 字符串，业务主键
    pub status: String,               // collecting / proposed / evaluating / awaiting_admin / released / aborted
    pub window_hours: u32,
    pub started_at: DateTime,
    pub updated_at: DateTime,
    pub finished_at: Option<DateTime>,
    pub cohort_run_ids_threshold: Vec<ObjectId>,
    pub cohort_run_ids_prompt: Vec<ObjectId>,
    pub budget_used_tokens: i64,
    pub budget_used_calls: i32,
    pub abort_reason: Option<String>,
}
```

索引：
- `(workspace_id, account_id, started_at desc)` —— 前端列表
- `experiment_id unique` —— 业务主键去重

#### 3.1.2 `proposals`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(rename = "_id", default)]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub experiment_id: String,
    pub proposal_kind: String,        // "threshold" | "prompt_diff"
    pub status: String,               // pending_eval / evaluating / eligible_for_release / rejected_below_threshold / released / discarded_by_admin / rolled_back / aborted
    pub created_at: DateTime,
    pub updated_at: DateTime,

    // threshold 类
    pub gate_key: Option<String>,     // FactRisk / PressureRisk / HumanLikeScore / EmotionalValue / ProductAccuracyScore / PlannerBlockRate
    pub current_value: Option<f64>,
    pub proposed_value: Option<f64>,
    pub hit_rate_observed: Option<f64>,
    pub hit_rate_target_band: Option<(f64, f64)>,

    // prompt 类
    pub prompt_template_key: Option<String>,
    pub section: Option<String>,      // soul / system_contract / policy / operator_instruction
    pub current_section_text: Option<String>,
    pub proposed_section_text: Option<String>,
    pub diff_summary: Option<String>,
    pub critic_reasoning: Option<String>,
    pub expected_improvement_on: Vec<String>,  // finalReviewStatus 枚举子集

    // shadow eval 聚合
    pub eval_replays_completed: i32,
    pub eval_replays_failed: i32,
    pub eval_metrics: Document,       // 灵活 schema：threshold 类 vs prompt 类字段不同
    pub significance_passed: Option<bool>,

    // release 元数据
    pub released_at: Option<DateTime>,
    pub released_by: Option<String>,
    pub rollback_at: Option<DateTime>,
    pub rolled_back_by: Option<String>,
    pub previous_threshold_value: Option<f64>,    // rollback 用
    pub previous_prompt_version: Option<String>,  // rollback 用

    pub failure_reason: Option<String>,
    pub cohort_notes: Option<String>,
}
```

索引：
- `(workspace_id, account_id, status, created_at desc)` —— EvolutionCenterTab 列表
- `experiment_id` —— 反查 experiment 下的 proposals

#### 3.1.3 `shadow_replays`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReplay {
    #[serde(rename = "_id", default)]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub experiment_id: String,
    pub proposal_id: ObjectId,
    pub source_run_id: ObjectId,           // 原 agent_run_logs._id
    pub status: String,                    // pending / running / completed / failed
    pub started_at: DateTime,
    pub finished_at: Option<DateTime>,

    pub original_final_review_status: String,
    pub new_final_review_status: Option<String>,

    pub original_review_risks: Vec<String>,
    pub new_review_risks: Vec<String>,

    pub original_token_cost: i64,
    pub new_token_cost: i64,

    pub original_self_critique_addressed: Option<bool>,
    pub new_self_critique_addressed: Option<bool>,

    pub original_5gate_hit: Document,      // { factRisk: bool, pressureRisk: bool, ... }
    pub new_5gate_hit: Document,

    pub similarity_to_original_text: f64,  // 本期允许常 0
    pub failure_reason: Option<String>,
}
```

索引：
- `proposal_id` —— 聚合时按 proposal 取
- `(workspace_id, account_id, started_at desc)` —— 前端列表

#### 3.1.4 `threshold_overrides`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdOverride {
    #[serde(rename = "_id", default)]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub gate_key: String,
    pub value: f64,
    pub released_proposal_id: ObjectId,
    pub released_at: DateTime,
    pub released_by: Option<String>,
    pub rolled_back_at: Option<DateTime>,
    pub rolled_back_by: Option<String>,
}
```

索引：
- `(workspace_id, account_id, gate_key, released_at desc)` —— resolve_thresholds 取最新

### 3.2 既有 collection 扩字段

#### 3.2.1 `prompt_templates`（最小破坏改造）

现状：单文档存单 prompt template。本期把 `prompt_templates` 升级为"多版本"形态：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    #[serde(rename = "_id", default)]
    pub id: Option<ObjectId>,
    pub key: String,                    // reply_agent_main / review_agent / ...
    pub version: String,                // v1 / v2 / v3 ...
    pub current_version: bool,          // 同 key 仅一条 true
    pub sections: Document,             // { soul, system_contract, policy, operator_instruction }
    pub seeded_by: String,              // "ensure_prompt_pack_v2" | "evolution_release"
    pub previous_version: Option<String>,
    pub released_proposal_id: Option<ObjectId>,
    pub created_at: DateTime,
}
```

迁移：`prompt_templates` 现有文档 SHALL 用一次性迁移脚本 `2026_05_M4_prompt_template_versioned` 加 `version="v_legacy" / current_version=true`；`ensure_prompt_pack_v2` 写入时 SHALL upsert by `(key, version)` 而非 by `(key)`。

索引：
- `(key, current_version)` —— 读路径只关心 `current_version=true`
- `(key, version) unique` —— 防重写

#### 3.2.2 `agent_events`

无 schema 变化，仅追加新 `kind` 值（详见 R8.1）。

### 3.3 prompt_pack_version 失效机制

LRU prompt cache 现有按 `(template_key)` 缓存。本期改为 `(template_key, prompt_pack_version)`：

```rust
// src/lib.rs 或 AppState 顶层
pub struct AppState {
    // ...
    pub prompt_pack_version: Arc<AtomicU64>,
}
```

`evolution::release::release_prompt_proposal` 在 commit 后 `prompt_pack_version.fetch_add(1, Ordering::SeqCst)`；下次 `generate_agent_json` 读 cache 时 key 自然失效，从 Mongo 重读。

`ensure_prompt_pack_v2` / `ensure_evolution_prompt_pack_v1` 在启动期也 fetch_add 一次（保证启动后第一个 run 读到最新 seed）。

## 4. 模块详细设计

### 4.1 `evolution::mod`（Worker 主循环）

```rust
pub async fn run_evolutionary_worker(state: AppState) {
    if !state.config.evolution_enabled {
        tracing::info!("evolution worker disabled by config; skipping spawn");
        return;
    }
    let interval = Duration::from_secs(state.config.evolution_tick_seconds);
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        if let Err(err) = run_one_tick(&state).await {
            tracing::error!(?err, "evolution_tick_failed");
            let _ = write_event(
                &state, "evolution_tick_failed", "failed",
                doc! { "error": err.to_string() },
            ).await;
            // 不 break，继续下一 tick
        }
    }
}

async fn run_one_tick(state: &AppState) -> AppResult<()> {
    let exp_id = uuid::Uuid::now_v7().to_string();
    let mut budget = EvolutionBudget::from_config(&state.config);
    insert_experiment_envelope(state, &exp_id).await?;          // status=collecting
    let cohorts = cohort::select_cohorts(state).await?;
    update_experiment_cohort(state, &exp_id, &cohorts).await?;

    // 阈值候选 (纯统计，不消耗 LLM 预算)
    let threshold_proposals = threshold::generate(state, &cohorts).await?;
    write_proposals(state, &exp_id, &threshold_proposals).await?;

    // prompt 候选 (Critic LLM)
    let prompt_proposals = match prompt_critic::generate(state, &cohorts, &mut budget).await {
        Ok(ps) => ps,
        Err(EvolutionError::BudgetExceeded) => {
            write_event(state, "evolution_budget_exceeded", "degraded", doc! { "phase": "prompt_critic" }).await?;
            vec![]
        }
        Err(e) => return Err(e.into()),
    };
    write_proposals(state, &exp_id, &prompt_proposals).await?;

    update_experiment_status(state, &exp_id, "evaluating").await?;

    // shadow eval 并行
    replay::eval_all(state, &exp_id, &mut budget).await?;
    significance::aggregate_and_grade(state, &exp_id).await?;

    update_experiment_status(state, &exp_id, "awaiting_admin").await?;
    write_event(state, "evolution_tick_completed", "ok", doc! {
        "experiment_id": &exp_id,
        "budget_used_tokens": budget.tokens_used(),
        "budget_used_calls": budget.calls_used(),
    }).await?;
    Ok(())
}
```

### 4.2 `evolution::cohort`

```rust
pub struct Cohorts {
    pub threshold: Vec<ObjectId>,
    pub prompt: Vec<ObjectId>,
}

pub async fn select_cohorts(state: &AppState) -> AppResult<Cohorts> {
    let since = DateTime::from_chrono(
        Utc::now() - chrono::Duration::hours(state.config.evolution_eval_window_hours as i64),
    );
    let coll = state.db.agent_run_logs();
    // threshold cohort: success+failure 混合
    let mixed = coll
        .find(doc! {
            "workspace_id": &state.config.default_workspace_id,
            "account_id": &state.config.default_account_id,
            "lifecycle": "completed",
            "started_at": { "$gte": since },
            "gateway_status": { "$nin": ["legacy_mode_unchecked", "blocked_by_required_field", "tool_loop_timeout", "mcp_error"] },
        }, /* limit + sort */)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let threshold_ids = dedupe_by_contact(mixed, /* per-contact cap = */ 3);
    // prompt cohort: failure-only
    let failure = coll.find(doc! {
        // 同上 + finalReviewStatus ∈ failure 集
    }, ...).await?.try_collect().await?;
    let prompt_ids = dedupe_by_contact(failure, 3);
    Ok(Cohorts { threshold: threshold_ids, prompt: prompt_ids })
}
```

### 4.3 `evolution::threshold`

纯统计，6 个 gate 并行算命中率，与合理区间常量表对比，产 ≤ 4 条 proposal。代码骨架略，复杂度全在常量表 + clamp 边界。

### 4.4 `evolution::prompt_critic`

```rust
pub async fn generate(
    state: &AppState,
    cohorts: &Cohorts,
    budget: &mut EvolutionBudget,
) -> Result<Vec<Proposal>, EvolutionError> {
    if cohorts.prompt.len() < state.config.evolution_min_replays {
        return Ok(vec![]);
    }
    let sampled = sample_per_failure_bucket(state, &cohorts.prompt, 10).await?;
    let critic_prompt = build_critic_prompt(&sampled, state).await?;  // 现行 prompt 全文 + cohort 失败摘要
    budget.check_or_fail()?;
    let json = state
        .agent
        .generate_agent_json("evolution_critic_v1", &critic_prompt, budget)
        .await?;
    let parsed = parse_critic_json(json)?;       // schema 校验
    let proposals = parsed.diffs.into_iter()
        .filter(|d| lint::passes_forbidden_words(&d.diff_snippet))
        .take(4)
        .map(to_proposal)
        .collect();
    Ok(proposals)
}
```

Critic JSON schema 在 `prompt_critic.rs` 内用 serde 反序列化结构体 + 字段长度上限校验；任何不符 SHALL 让 Critic 输出整批 drop。

### 4.5 `evolution::replay`（模拟 gateway，关键模块）

```rust
pub async fn eval_all(
    state: &AppState,
    exp_id: &str,
    budget: &mut EvolutionBudget,
) -> AppResult<()> {
    let proposals = load_pending_eval(state, exp_id).await?;
    let sem = Arc::new(Semaphore::new(state.config.evolution_replay_concurrency));
    let mut handles = vec![];
    for p in proposals {
        for run_id in p.cohort_run_ids() {
            if budget.exhausted() { break; }
            let permit = sem.clone().acquire_owned().await?;
            let state = state.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                run_shadow_replay(&state, &p, run_id).await
            }));
        }
    }
    for h in handles { let _ = h.await; }
    Ok(())
}

pub async fn run_shadow_replay(
    state: &AppState,
    proposal: &Proposal,
    source_run_id: ObjectId,
) -> AppResult<()> {
    // 1. 读原 run + contact + memoryCard 快照
    let original_run = state.db.agent_run_logs().find_one(doc! { "_id": source_run_id }, None).await?;
    let Some(original) = original_run else {
        return write_replay_failed(state, proposal, source_run_id, "source_run_not_found").await;
    };
    let inbound = match find_source_inbound(state, &original).await? {
        Some(m) => m,
        None => return write_replay_failed(state, proposal, source_run_id, "source_message_unavailable").await,
    };
    // 2. 决定本次 replay 用什么阈值 / prompt
    let (overrides_for_this_run, prompt_overrides_for_this_run) = match proposal.proposal_kind.as_str() {
        "threshold" => (Some(build_override(proposal)), None),
        "prompt_diff" => (None, Some(build_prompt_override(proposal))),
        _ => unreachable!(),
    };
    // 3. 调"短路 gateway"
    let outcome = run_simulated_gateway(
        state,
        &inbound,
        &original.contact_wxid,
        overrides_for_this_run,
        prompt_overrides_for_this_run,
    ).await?;
    // 4. 写 shadow_replays
    insert_shadow_replay(state, proposal, source_run_id, &original, &outcome).await
}
```

`run_simulated_gateway` 是 R5.2 的关键短路实现：

- 入口可访问 LLM（用候选 prompt 算决策与 review）；
- 出口**不**写 `agent_send_outbox`、**不**调 `mcp::send_message`、**不**写 `conversation_messages` outbound；
- `agent_run_logs` 在 shadow 路径**不**写（只写 `shadow_replays`），避免污染 outcomes；
- 仍然消费 EvolutionBudget（每次 LLM 调用计 token + calls）。

实现上推荐用现有 `agent::run_user_operation_gateway` 的内部 helper（如 `agent::reply::reply_with_tools_loop / agent::review::review_decision`）但**不**调最外层 `run_user_operation_gateway` 本体（那是写库 + outbox 的复合入口）。这意味着 `agent/` 内的可复用函数 SHALL 是 `pub(crate)` 而非 `pub(super)`——本期会在 W1 评估并最小化暴露面。

### 4.6 `evolution::significance`

纯函数，输入 `Vec<ShadowReplay>`，输出 `(passed: bool, eval_metrics: Document)`：

```rust
pub fn grade_threshold(replays: &[ShadowReplay], cfg: &SignificanceCfg) -> (bool, Document) {
    if replays.iter().filter(|r| r.status == "completed").count() < cfg.min_replays {
        return (false, doc! { "reason": "insufficient_completed_replays" });
    }
    let original_send_success = replays.iter()
        .filter(|r| r.original_final_review_status == "approved" || r.original_final_review_status == "revision_applied_approved")
        .count() as f64 / replays.len() as f64;
    let new_send_success = replays.iter()
        .filter(|r| r.new_final_review_status.as_deref() == Some("approved")
                  || r.new_final_review_status.as_deref() == Some("revision_applied_approved"))
        .count() as f64 / replays.len() as f64;
    let delta = new_send_success - original_send_success;
    let passed = delta >= cfg.min_send_success_rate_delta;
    (passed, doc! {
        "original_send_success_rate": original_send_success,
        "new_send_success_rate": new_send_success,
        "delta": delta,
    })
}

pub fn grade_prompt(replays: &[ShadowReplay], cfg: &SignificanceCfg) -> (bool, Document) {
    // self_critique_addressed_delta + 5gate_hit_delta_per_gate + token_cost_delta
    // ...
}
```

无 LLM 调用，决定性纯函数 → 易写单测。

### 4.7 `evolution::release`

```rust
pub async fn release_threshold(state: &AppState, proposal_id: ObjectId, admin: &str) -> AppResult<()> {
    let proposal = load_proposal(state, proposal_id).await?;
    require_status(&proposal, "eligible_for_release")?;
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    // a) insert threshold_overrides
    state.db.threshold_overrides()
        .insert_one_with_session(ThresholdOverride { /* ... */ }, None, &mut session).await?;
    // b) update proposals
    state.db.proposals()
        .update_one_with_session(doc! { "_id": proposal_id },
            doc! { "$set": { "status": "released", "released_at": now(), "released_by": admin } },
            None, &mut session).await?;
    session.commit_transaction().await?;
    write_event(state, "evolution_threshold_released", "ok", doc! {
        "proposal_id": proposal_id, "gate_key": proposal.gate_key, "value": proposal.proposed_value,
    }).await?;
    Ok(())
}

pub async fn release_prompt(state: &AppState, proposal_id: ObjectId, admin: &str) -> AppResult<()> {
    let proposal = load_proposal(state, proposal_id).await?;
    require_status(&proposal, "eligible_for_release")?;
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let key = proposal.prompt_template_key.as_deref().unwrap();
    let current = state.db.prompt_templates().find_one_with_session(
        doc! { "key": key, "current_version": true }, None, &mut session,
    ).await?.ok_or(EvolutionError::CurrentPromptNotFound)?;
    let new_version = bump_version(&current.version);  // v2 → v3
    let new_sections = apply_diff(&current.sections, &proposal.section, &proposal.proposed_section_text);
    state.db.prompt_templates()
        .update_one_with_session(doc! { "_id": current.id }, doc! { "$set": { "current_version": false } }, None, &mut session).await?;
    state.db.prompt_templates()
        .insert_one_with_session(PromptTemplate {
            key: key.to_string(),
            version: new_version.clone(),
            current_version: true,
            sections: new_sections,
            previous_version: Some(current.version.clone()),
            released_proposal_id: Some(proposal_id),
            seeded_by: "evolution_release".to_string(),
            created_at: now(),
            id: None,
        }, None, &mut session).await?;
    state.db.proposals()
        .update_one_with_session(doc! { "_id": proposal_id }, doc! { "$set": {
            "status": "released",
            "released_at": now(),
            "released_by": admin,
            "previous_prompt_version": current.version,
        }}, None, &mut session).await?;
    session.commit_transaction().await?;
    state.prompt_pack_version.fetch_add(1, Ordering::SeqCst);
    write_event(state, "evolution_prompt_released", "ok", doc! { "proposal_id": proposal_id, "key": key, "new_version": new_version }).await?;
    Ok(())
}
```

`rollback_*` 对称：threshold 类把 `threshold_overrides.rolled_back_at=now`；prompt 类把 `current_version` 指针指回 `previous_version`，`prompt_pack_version.fetch_add(1)`。

### 4.8 `evolution::post_release`

每次 release 都 insert 一条 `post_release_review` 文档（`scheduled_at = released_at + 24h`）；演化器主循环每 tick 顺手扫一次 `scheduled_at <= now AND completed=false`，对每条做"对比窗口评测"：

- 算 `released_at - 24h ~ released_at` 的 outcomes 切片；
- 算 `released_at ~ released_at + 24h` 的 outcomes 切片；
- 写差值到 `post_release_review.actual_*_delta`；
- 不自动回滚，仅 `agent_events kind="evolution_post_release_review"`。

### 4.9 `routes/evolution.rs`

```rust
pub fn evolution_routes() -> Router<AppState> {
    Router::new()
        .route("/api/evolution/experiments", get(list_experiments))
        .route("/api/evolution/proposals/:id", get(get_proposal))
        .route("/api/evolution/proposals/:id/release", post(release_proposal))
        .route("/api/evolution/proposals/:id/rollback", post(rollback_proposal))
}
```

`release_proposal` / `rollback_proposal` SHALL 复用 admin auth middleware（与现有 `routes/contacts.rs` 同款）。

请求 body 形如 `{ "confirmation": "RELEASE" }`，处理函数 SHALL 校验文本完全匹配——前端 R7.5 的二次确认 modal 也是同款字符串。

### 4.10 前端 `EvolutionCenterTab`

新增 `frontend/src/EvolutionCenterTab.tsx`，由 `App.tsx` 顶层 channel/tab 状态机切换。组件结构：

```
EvolutionCenterTab
├── 顶部聚合卡（最近 7 天 experiments / proposals / released / rolled_back / 显著性通过率）
├── proposal 列表（按 created_at desc）
│   ├─ 行：gate_key 或 promptTemplateKey + section / status 徽章 / shadow eval 摘要
│   └─ 行内"展开"按钮 → 展示 ProposalDetail
└── ProposalDetail
    ├── threshold 类：current_value vs proposed_value bar + hit_rate_observed
    ├── prompt 类: 双栏 diff（current_section_text | proposed_section_text）+ Critic reasoning
    ├── shadow eval 报告卡
    ├── [发布] 按钮（仅 eligible_for_release 启用）→ ReleaseModal
    └── [回滚] 按钮（仅 released 启用）→ RollbackModal
```

ReleaseModal 要求文本框输入 `RELEASE` 才启用确认按钮；RollbackModal 同款 `ROLLBACK`。

vitest 单测覆盖 R7.7 的 (a)(b)(c) 三个场景。

## 5. Configuration & Env

`.env.example` 新增：

```sh
EVOLUTION_ENABLED=false
EVOLUTION_TICK_SECONDS=21600
EVOLUTION_RUN_TOKEN_BUDGET=60000
EVOLUTION_RUN_MAX_LLM_CALLS=30
EVOLUTION_EVAL_WINDOW_HOURS=72
EVOLUTION_MIN_REPLAYS=30
EVOLUTION_MIN_SEND_SUCCESS_DELTA=0.05
EVOLUTION_MIN_SELF_CRITIQUE_DELTA=0.10
EVOLUTION_MAX_5GATE_HIT_INCREASE=0.10
EVOLUTION_REPLAY_CONCURRENCY=4
EVOLUTION_REPLAY_MAX_FAIL_RATE=0.30
EVOLUTION_THRESHOLD_RELEASE_COOLDOWN_HOURS=24
EVOLUTION_COHORT_PER_CONTACT_CAP=3
```

`src/config.rs` 在 `AppConfig` 加同名字段 + getter；`tests/common/mod.rs::test_config()` 同步默认值。

## 6. 测试策略

### 6.1 单测（`#[cfg(test)]` in `src/evolution/*.rs`，无 Docker）

| # | 模块                 | 用例                                                           |
| - | ------------------ | ------------------------------------------------------------ |
| 1 | `cohort`           | 窗口外 run 不进 cohort                                              |
| 2 | `cohort`           | 同 contact 超 cap 被去重                                            |
| 3 | `cohort`           | conversation_messages 缺失 → shadow_replays.failed                 |
| 4 | `threshold`        | 命中率 < 区间下限 → 候选 +step                                          |
| 5 | `threshold`        | 命中率 > 区间上限 → 候选 -step                                          |
| 6 | `threshold`        | cooldown 内同 gate 跳过                                            |
| 7 | `threshold`        | clamp 边界（FactRisk = 10 + 1 时 propose=10）                       |
| 8 | `prompt_critic`    | Critic 输出 schema invalid 整批 drop                                |
| 9 | `prompt_critic`    | Critic 输出含禁词 → drop                                             |
| 10 | `prompt_critic`    | budget exhausted 时 generate 返回空 vec                              |
| 11 | `significance`     | 30 条 replay，原 0.6 / 新 0.7 → passed=true                         |
| 12 | `significance`     | replay 成功率 < 70% → reject                                        |
| 13 | `release`          | release_threshold 写 threshold_overrides + 改 status              |
| 14 | `release`          | release_prompt 把老版本 current_version=false、新版本=true              |
| 15 | `release`          | rollback_threshold 设 rolled_back_at                              |
| 16 | `release`          | rollback_prompt 把 current_version 指针指回 previous              |
| 17 | `replay`           | 短路：调 replay 不写 outbox / 不写 conversation_messages outbound       |
| 18 | `lint`             | proposed_value / diff_snippet 含禁词被拦                             |

合计 ≥ 18 单测，加上 baseline 313 → ≥ 331。

### 6.2 集成测试（`tests/`，testcontainers，`#[ignore]` 默认）

- `tests/evolution_threshold_e2e.rs` —— mock 5 闸命中率分布 → tick 产 ≥ 1 条 threshold proposal → shadow eval → admin release → resolve_thresholds 读到新值
- `tests/evolution_prompt_e2e.rs` —— failure cohort → Critic（mock LLM 返回 fixture diff）→ shadow eval → release → prompt_pack_version bump → 下次 generate_agent_json 从 Mongo 重读
- `tests/evolution_rollback.rs` —— release 后 rollback → resolve_thresholds 读回老值
- `tests/evolution_isolation.rs` —— shadow replay 跑 100 次后 `agent_send_outbox` collection size 不变（确保短路）

### 6.3 PBT

本期 SHALL 不动 4 PBT 的 ≥ 33 baseline，但建议给 `significance.rs` 的纯函数加 PBT：阈值候选 delta 在合理区间内永不 panic、prompt grade 在 replay vec empty 时永远 reject，写到 `tests/evolution_significance_pbt.rs`（≥ 6 case）。这是新增 PBT 文件，不计入 R11.6 的 4 文件 baseline 但计入"4 PBT 累计 ≥ 33"的精神（合并门只看 4 文件本身）。

### 6.4 前端 vitest

- `EvolutionCenterTab.test.tsx`
  - 列表渲染 4 种 status 徽章
  - 发布按钮只在 `eligible_for_release` 启用
  - ReleaseModal 输入串错时不发请求
  - prompt diff 双栏渲染不混淆 section text

合计 ≥ 4 case，加上现有 baseline 8 → ≥ 12。

### 6.5 CI lint

- `scripts/check-no-human-takeover.sh` 扫描目录加 `src/evolution/`
- 新增 `scripts/check-evolution-isolation.sh`：grep `src/evolution/` 是否引用 `crate::agent::gateway / outbox / mcp`，命中即 fail
- `scripts/check-baseline.sh`：cargo test --lib 0 failed；4 PBT 累计 ≥ 33 不变；新增 evolution 集成测试不参与 baseline gate 但 PR 必须能编译过

## 7. 风险与回滚

### 7.1 风险矩阵

| 风险                                                | 概率   | 影响                  | 缓解                                                                        |
| ------------------------------------------------- | ---- | ------------------- | ------------------------------------------------------------------------- |
| Critic LLM 输出"绕过 5 闸"的 prompt diff                  | 中    | 灾难（绕安全门）             | shadow eval 必显著降低 5 闸命中率才 pass，但不可能"零命中"——R5.5 增 `max_5gate_hit_increase` 上限 + `evolution::lint` 写入前禁词扫描 + admin 二次确认 + R5.6 的 5 闸不允许"任意一项"涨超 +0.10 |
| 演化器拖慢主进程                                            | 低    | 中                   | 独立 tokio task + EvolutionBudget 限流 + 默认 6h tick                            |
| shadow eval cohort 偏斜                              | 中    | 中（误判显著性）            | per-contact dedupe + min_replays + replay_max_fail_rate                   |
| release 与正在进行 run 竞争                                | 低    | 低                   | resolve_thresholds 在 run 入口取一次，run 中途不重读（R6.7）                              |
| prompt_templates 多版本迁移破坏既有 ensure_prompt_pack_v2     | 中    | 中                   | W0 迁移脚本 + 单测验证迁移幂等 + ensure_prompt_pack_v2 升级为 upsert by (key, version)      |
| Critic prompt 自身需要演化（自我引用悖论）                        | 低    | 低                   | R9.3 显式禁止：Critic prompt 是常量，需要修改时走代码 PR                                    |

### 7.2 回滚路径

- **演化器主开关**：`EVOLUTION_ENABLED=false` → spawn 不发生，已发布的 threshold_overrides / prompt_templates 不动；`resolve_thresholds` 仍读 overrides（保持已发布的演化结果）。
- **单条 release 回滚**：admin 在 `EvolutionCenterTab` 点回滚 → `release.rs::rollback_*` → 阈值恢复、prompt 版本指针回退、`prompt_pack_version.fetch_add(1)` 让 cache 失效。
- **整批清空**：极端场景下 admin 想"全部回滚到 M3 状态"，提供 `POST /api/evolution/rollback_all`（需 admin 输入 `ROLLBACK_ALL`），把所有 `threshold_overrides` 一次性 `rolled_back_at=now`、所有 `prompt_templates(current_version=true, seeded_by="evolution_release")` 回退到 `previous_version`。该路由 SHALL 写一条强警示 `agent_events kind="evolution_rollback_all"`。
- **代码层 sunset**：本期不引入"双轨"（不存在 evolution 与 non-evolution 两套生效路径），只引入"覆盖"（`threshold_overrides` 覆盖 `runtime_parameters`）；rollback 后行为完全等于 M3 末状态。

## 8. Sunset & 后续 spec 锚点

- M5 候选：自动发布开关（R9.6）、跨 workspace / 多 account 演化共享、引入 embedding 相似度评测（替换 R5.2 的 0 占位）。
- M5 候选：演化器调用 Reply Agent 的内部 helper 进入"prompt 元 evolution"——目前 R9.3 显式禁止，未来若放宽 SHALL 在新 spec 立项。
- M5 候选：把 `agent-self-evolution` 的 EvolutionCenterTab 与 AutonomyOutcomesTab 的 Planner section 合并为统一"运营智能中心"。

## 9. 不做的事（与 requirements R10 镜像）

设计层面 SHALL NOT：

1. 在 `src/agent/` 引入对 `src/evolution/` 的依赖（避免循环）。
2. 在 `routes/evolution.rs` 内调 `agent::run_user_operation_gateway`。
3. 让 `src/evolution::replay` 写 `agent_send_outbox` / `conversation_messages` outbound。
4. 让 `EvolutionCenterTab` 直接读 Mongo（必须走 routes/evolution）。
5. 让 release 路径绕过 admin auth middleware。
6. 引入 BSON 字段类型变更（仅新增 collection / 新增字段）。
7. 引入新 LLM provider（沿用 OpenAI-compatible）。
8. 引入新 PBT baseline（4 PBT 文件继续 ≥ 33）。
