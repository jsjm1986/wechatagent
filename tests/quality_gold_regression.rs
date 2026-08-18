//! 金标质量回归环 v1 执行器（`#[ignore]`：真实 LLM + testcontainers MongoDB）。
//!
//! 一键入口：`scripts/quality-regression.sh`（env 检查 fail-fast + 汇总输出）。
//! 手动运行：`cargo test --test quality_gold_regression -- --ignored --nocapture`。
//!
//! ## 链路（每场景）
//! seed contact（内存构造，shadow 不落库）+ verified 知识（`operation_knowledge_chunks`，
//! 场景结束即删）→ `simulate_user_dialogue`（复用真实 Reply+Review+ClaimGate 链，
//! run_mode="shadow"，零真实发送、零业务写库）→ `would_send` 轮红线硬断言
//! （`tests/common/quality_gold.rs` 闭集检查）→ judge 打分（`REAL_LLM_JUDGE=1` 时启用，
//! K 次采样取中位；否则标注 skipped）→ 每场景一行 JSONL ledger。
//!
//! ## 门槛（v1 软门，与设计 §4 C1 对齐）
//! - 红线违规即 fail（唯一硬门）；
//! - judge 分数与 `qualityFloor` 命中数只落 ledger / 汇总表，不 fail——ledger 累积
//!   ≥3 次运行且方差可接受后由主会话决策升硬门。
//!
//! ## 纪律
//! - env-gated：缺 `REAL_LLM_API_KEY` 自跳过（与 real_llm 系列同口径）；
//! - LLM 端点瞬时抖动 → 该场景记 `skipped_transient` 继续；非瞬时错误（账户/配置/解析）
//!   → panic 不得假绿；全部场景都被跳过 → panic；
//! - 本文件与场景 fixture 均不硬编码任何具体模型名（provider 全部由 env 显式给定）；
//! - MCP 保持 TestApp 桩地址（simulation 永不发送，防御性保留桩语义）。

mod common;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::json;

use wechatagent::agent::{default_domain_profile, simulate_user_dialogue};
use wechatagent::llm::{LlmClient, LlmFormat};
use wechatagent::models::{AgentStatus, Contact};

use crate::common::judge::{build_judge_rubric, run_judge_graded, JudgeGate, JudgeRubric};
use crate::common::quality_gold::{
    load_all, redline_violations, GoldScenario, DEFAULT_QUALITY_FLOOR,
};
use crate::common::roleplay_fixtures::seed_verified_chunk;
use crate::common::{rebuild_app_state_with_real_llm, TestApp};

/// 读取必填 env（缺失即 panic，报设置指引——scripts/quality-regression.sh 会前置检查）。
fn required_env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            panic!("环境变量 {name} 未设置。金标回归环的 LLM provider 全部由 env 显式给定（不内置任何模型名），请通过 scripts/quality-regression.sh 运行或参照其 env 清单设置。")
        })
}

fn llm_format_from_env(name: &str) -> LlmFormat {
    match std::env::var(name).ok().as_deref() {
        Some("anthropic") | Some("messages") | Some("claude") => LlmFormat::Anthropic,
        _ => LlmFormat::Openai,
    }
}

/// 被测 agent 的真实 LLM client（env 显式配置；retries=10 对齐 real_llm 系列端点韧性）。
fn agent_client() -> Arc<LlmClient> {
    let api_key = required_env("REAL_LLM_API_KEY");
    let base_url = required_env("REAL_LLM_BASE_URL");
    let model = required_env("REAL_LLM_MODEL");
    let fmt = llm_format_from_env("REAL_LLM_FORMAT");
    Arc::new(
        LlmClient::with_format(base_url, api_key, model, fmt, 180, 10, 2500)
            .expect("构造金标回归 agent LLM client"),
    )
}

/// judge client（异族，REAL_LLM_JUDGE_* 显式配置）。仅 `REAL_LLM_JUDGE=1` 时构造；
/// 开了 judge 却缺配置 → panic（半配置比缺席更危险，不静默降级）。
fn judge_client_if_enabled() -> Option<Arc<LlmClient>> {
    if std::env::var("REAL_LLM_JUDGE").ok().as_deref() != Some("1") {
        return None;
    }
    let api_key = required_env("REAL_LLM_JUDGE_API_KEY");
    let base_url = required_env("REAL_LLM_JUDGE_BASE_URL");
    let model = required_env("REAL_LLM_JUDGE_MODEL");
    let fmt = llm_format_from_env("REAL_LLM_JUDGE_FORMAT");
    Some(Arc::new(
        LlmClient::with_format(base_url, api_key, model, fmt, 180, 6, 2500)
            .expect("构造金标回归 judge LLM client"),
    ))
}

/// 按场景 contactSeed 构造 managed contact（仅内存对象——shadow 链路只读，不落库）。
fn scenario_contact(ws: &str, acc: &str, scenario: &GoldScenario) -> Contact {
    let now = DateTime::now();
    let seed = &scenario.contact_seed;
    let non_empty = |s: &str| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    };
    Contact {
        id: None,
        workspace_id: ws.to_string(),
        account_id: acc.to_string(),
        wxid: format!("qg_{}", scenario.id.replace('-', "_")),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: non_empty(&seed.profile_note),
        custom_agent_instructions: non_empty(&seed.custom_instructions),
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: non_empty(&seed.memory_summary),
        playbook_id: None,
        playbook_version: None,
        manual_tags: seed.manual_tags.clone(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: Some(doc! {
            "customer_stage": seed.customer_stage.clone(),
            "intent_level": seed.intent_level.clone(),
        }),
        domain_attributes_updated_at: Some(now),
        commitments: Vec::new(),
        follow_up_policy: None,
        // C2：operation_state 与 customer_stage 同一 canonical id 空间。
        operation_state: Some(seed.customer_stage.clone()),
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: Some(now),
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

/// 为批量模拟抬高独立 LLM 容量：多轮场景会按消息数扩展单轮链路调用，不能让评测
/// 因模拟预算先耗尽而被误判为对话质量问题。取值仍在运营写侧 schema 允许范围内
/// （runMaxLlmCalls ≤ 20、simulationTokenBudget ≤ 2000000）。
async fn raise_simulation_budget(app: &TestApp, ws: &str) {
    app.state
        .db
        .raw()
        .collection::<Document>("operation_domain_configs")
        .update_one(
            doc! {
                "workspace_id": ws,
                "domain": "user_operations",
                "current_version": true,
            },
            doc! { "$set": {
                "runtime_parameters.runMaxLlmCalls": 20_i32,
                "runtime_parameters.simulationTokenBudget": 1_200_000_i32,
            }},
            None,
        )
        .await
        .expect("raise simulation llm budget for gold regression");
}

struct ScenarioOutcome {
    id: String,
    category: String,
    executed: bool,
    violations: Vec<String>,
    /// 每 would_send 轮的 judge overall 中位（judge 关闭/失败时为空）。
    overall_scores: Vec<i64>,
    floor_hit: bool,
}

fn ledger_path() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from("target").join("quality_gold");
    std::fs::create_dir_all(&dir).expect("create target/quality_gold");
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_secs();
    dir.join(format!("run-{ts}.jsonl"))
}

fn append_ledger(path: &std::path::Path, row: serde_json::Value) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open quality gold ledger");
    writeln!(file, "{row}").expect("append quality gold ledger");
}

/// 场景过滤（本地分钟级子集回归用）：
/// `QUALITY_GOLD_CATEGORY`（单类）/ `QUALITY_GOLD_ID`（单场景）/ `QUALITY_GOLD_LIMIT`（条数上限）。
fn apply_filters(mut scenarios: Vec<GoldScenario>) -> Vec<GoldScenario> {
    if let Ok(cat) = std::env::var("QUALITY_GOLD_CATEGORY") {
        let cat = cat.trim().to_string();
        if !cat.is_empty() {
            scenarios.retain(|s| s.category == cat);
        }
    }
    if let Ok(id) = std::env::var("QUALITY_GOLD_ID") {
        let id = id.trim().to_string();
        if !id.is_empty() {
            scenarios.retain(|s| s.id == id);
        }
    }
    if let Some(limit) = std::env::var("QUALITY_GOLD_LIMIT")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        scenarios.truncate(limit);
    }
    scenarios
}

#[tokio::test]
#[ignore]
async fn quality_gold_regression() {
    // env-gated 自跳过（与 real_llm 系列同口径；一键脚本会前置 fail-fast）。
    if std::env::var("REAL_LLM_API_KEY")
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        eprintln!("skip: 未设置 REAL_LLM_API_KEY——金标回归环需要真实 LLM（scripts/quality-regression.sh 有完整 env 清单）");
        return;
    }
    let agent_llm = agent_client();
    let judge = judge_client_if_enabled();
    let judge_samples = std::env::var("QUALITY_GOLD_JUDGE_SAMPLES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3);
    let global_floor = std::env::var("QUALITY_GOLD_FLOOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(DEFAULT_QUALITY_FLOOR);

    let scenarios = apply_filters(load_all());
    assert!(
        !scenarios.is_empty(),
        "场景过滤后为空（检查 QUALITY_GOLD_CATEGORY / QUALITY_GOLD_ID / QUALITY_GOLD_LIMIT）"
    );

    let app = TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let acc = app.state.config.default_account_id.clone();
    raise_simulation_budget(&app, &ws).await;
    // TestApp 桩 MCP 地址原样保留：simulation 永不调 MCP，这里只是防御性桩语义。
    let state =
        rebuild_app_state_with_real_llm(&app, agent_llm, app.state.config.mcp_base_url.clone());
    // judge 标尺从销售 DEFAULT profile 派生（default workspace 无 active 行时的生效 profile）。
    let rubric: JudgeRubric = build_judge_rubric(&default_domain_profile(&ws));

    let ledger = ledger_path();
    eprintln!(
        "[quality-gold] 场景 {} 条，judge={}，ledger={}",
        scenarios.len(),
        if judge.is_some() {
            "on"
        } else {
            "skipped（未设 REAL_LLM_JUDGE=1）"
        },
        ledger.display()
    );

    let mut outcomes: Vec<ScenarioOutcome> = Vec::new();
    let chunks = app.state.db.operation_knowledge_chunks();

    for scenario in &scenarios {
        let started = Instant::now();
        let started_wall = DateTime::now();

        // seed 本场景 verified 知识（场景结束即删，防跨场景串扰）。
        let mut seeded_chunk_ids: Vec<ObjectId> = Vec::new();
        for seed in &scenario.knowledge_seeds {
            let hex = seed_verified_chunk(&app, &ws, &seed.title, &seed.summary, &seed.body).await;
            seeded_chunk_ids.push(ObjectId::parse_str(&hex).expect("seeded chunk id"));
        }

        let contact = scenario_contact(&ws, &acc, scenario);
        let wxid = contact.wxid.clone();
        let result =
            simulate_user_dialogue(&state, contact, scenario.inbound_messages.clone()).await;

        // 场景级清理（无论成败）。
        if !seeded_chunk_ids.is_empty() {
            chunks
                .delete_many(doc! { "_id": { "$in": &seeded_chunk_ids } }, None)
                .await
                .expect("cleanup seeded gold chunks");
        }

        let turns = match result {
            Ok(turns) => turns,
            Err(wechatagent::error::AppError::LlmUnavailable {
                kind, retry_count, ..
            }) if wechatagent::llm::is_transient_llm_unavailable_kind(&kind) => {
                eprintln!(
                    "[quality-gold][{}] 上游瞬时不可达（kind={kind}, retries={retry_count}），场景记 skipped_transient",
                    scenario.id
                );
                append_ledger(
                    &ledger,
                    json!({
                        "kind": "scenario",
                        "id": scenario.id,
                        "category": scenario.category,
                        "status": "skipped_transient",
                        "llmKind": kind,
                    }),
                );
                outcomes.push(ScenarioOutcome {
                    id: scenario.id.clone(),
                    category: scenario.category.clone(),
                    executed: false,
                    violations: Vec::new(),
                    overall_scores: Vec::new(),
                    floor_hit: false,
                });
                continue;
            }
            Err(other) => panic!(
                "[quality-gold][{}] simulation 非瞬时失败（不得假绿）：{other:?}",
                scenario.id
            ),
        };

        assert_eq!(
            turns.len(),
            scenario.inbound_messages.len(),
            "[quality-gold][{}] 轮数与入站消息数不一致",
            scenario.id
        );
        // 环体检：全轮 gateway_blocked 说明 harness 配置坏（如 contact 未 managed），
        // 不是对话质量信号。
        assert!(
            turns.iter().any(|t| t.status != "gateway_blocked"),
            "[quality-gold][{}] 全部轮次 gateway_blocked——回归环 harness 配置异常：{:?}",
            scenario.id,
            turns.iter().map(|t| t.status.clone()).collect::<Vec<_>>()
        );

        let checks = &scenario.expectations.must_not_violate;
        let floor = scenario.expectations.quality_floor.unwrap_or(global_floor);
        let mut violations: Vec<String> = Vec::new();
        let mut overall_scores: Vec<i64> = Vec::new();
        let mut turn_rows: Vec<serde_json::Value> = Vec::new();

        for turn in &turns {
            let mut judge_medians: Option<BTreeMap<String, i64>> = None;
            let mut turn_violations: Vec<String> = Vec::new();
            if turn.status == "would_send" {
                // 红线硬断言只作用于将触达客户的最终文本。
                turn_violations = redline_violations(&turn.reply_text, checks);
                for v in &turn_violations {
                    violations.push(format!("turn{}:{v}", turn.turn));
                }
                if let Some(judge) = judge.as_deref() {
                    let label = format!("{}#t{}", scenario.id, turn.turn);
                    if let Some(outcome) = run_judge_graded(
                        judge,
                        &rubric,
                        &label,
                        &turn.inbound_text,
                        &turn.reply_text,
                        judge_samples,
                        JudgeGate::ObserveOnly,
                    )
                    .await
                    {
                        if let Some(overall) = outcome.medians.get("overall") {
                            overall_scores.push(*overall);
                        }
                        judge_medians =
                            Some(outcome.medians.into_iter().collect::<BTreeMap<_, _>>());
                    }
                }
            }
            let used_knowledge_ids = turn
                .decision
                .get_array("usedKnowledgeIds")
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            turn_rows.push(json!({
                "turn": turn.turn,
                "status": turn.status,
                "shouldReply": turn.should_reply,
                "replyChars": turn.reply_text.chars().count(),
                "usedKnowledgeIds": used_knowledge_ids,
                "violations": turn_violations,
                "judge": judge_medians,
            }));
        }

        // 成本口径：shadow 链路的 llm_call_logs（按 contact + 起始时刻圈定）。
        let llm_calls = app
            .state
            .db
            .raw()
            .collection::<Document>("llm_call_logs")
            .count_documents(
                doc! { "contact_wxid": &wxid, "created_at": { "$gte": started_wall } },
                None,
            )
            .await
            .unwrap_or(0);

        let floor_hit = overall_scores.iter().any(|s| (*s as f64) < floor);
        let latency_ms = started.elapsed().as_millis();
        append_ledger(
            &ledger,
            json!({
                "kind": "scenario",
                "id": scenario.id,
                "category": scenario.category,
                "status": "executed",
                "turns": turn_rows,
                "violations": violations,
                "overallScores": overall_scores,
                "qualityFloor": floor,
                "floorHit": floor_hit,
                "latencyMs": latency_ms as u64,
                "llmCalls": llm_calls,
            }),
        );
        outcomes.push(ScenarioOutcome {
            id: scenario.id.clone(),
            category: scenario.category.clone(),
            executed: true,
            violations,
            overall_scores,
            floor_hit,
        });
    }

    // ── 汇总（五类分布 + ledger 总结行）────────────────────────────────────
    let executed: Vec<&ScenarioOutcome> = outcomes.iter().filter(|o| o.executed).collect();
    assert!(
        !executed.is_empty(),
        "全部场景因上游抖动被跳过——本次运行没有产生任何质量信号，不得视为通过"
    );
    let mut summary_rows = Vec::new();
    eprintln!("\n[quality-gold] ── 五类汇总 ──");
    for category in crate::common::quality_gold::GOLD_CATEGORIES {
        let of_cat: Vec<&&ScenarioOutcome> =
            executed.iter().filter(|o| o.category == category).collect();
        if of_cat.is_empty() {
            continue;
        }
        let mut scores: Vec<i64> = of_cat
            .iter()
            .flat_map(|o| o.overall_scores.iter().copied())
            .collect();
        scores.sort_unstable();
        let mean = if scores.is_empty() {
            None
        } else {
            Some(scores.iter().sum::<i64>() as f64 / scores.len() as f64)
        };
        let median = if scores.is_empty() {
            None
        } else {
            Some(scores[scores.len() / 2])
        };
        let floor_hits = of_cat.iter().filter(|o| o.floor_hit).count();
        let violation_count: usize = of_cat.iter().map(|o| o.violations.len()).sum();
        eprintln!(
            "  {category:<10} n={:<3} overall mean={} median={} floorHits={floor_hits} violations={violation_count}",
            of_cat.len(),
            mean.map(|m| format!("{m:.2}")).unwrap_or_else(|| "-".into()),
            median.map(|m| m.to_string()).unwrap_or_else(|| "-".into()),
        );
        summary_rows.push(json!({
            "category": category,
            "executed": of_cat.len(),
            "overallMean": mean,
            "overallMedian": median,
            "floorHits": floor_hits,
            "violations": violation_count,
        }));
    }
    let skipped = outcomes.len() - executed.len();
    append_ledger(
        &ledger,
        json!({
            "kind": "summary",
            "totalScenarios": outcomes.len(),
            "executed": executed.len(),
            "skippedTransient": skipped,
            "judgeEnabled": judge.is_some(),
            "categories": summary_rows,
        }),
    );
    eprintln!(
        "[quality-gold] executed={} skippedTransient={skipped} ledger={}",
        executed.len(),
        ledger.display()
    );

    // ── v1 唯一硬门：红线违规 ────────────────────────────────────────────────
    let offenders: Vec<String> = executed
        .iter()
        .filter(|o| !o.violations.is_empty())
        .map(|o| format!("{}: {:?}", o.id, o.violations))
        .collect();
    assert!(
        offenders.is_empty(),
        "金标红线硬门违规 {} 个场景（judge 软门只记录，此断言只含确定性红线）：\n{}",
        offenders.len(),
        offenders.join("\n")
    );

    drop(state);
    app.cleanup().await;
}
