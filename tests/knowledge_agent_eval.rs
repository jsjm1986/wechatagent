//! Phase E / E3：knowledge_agent 离线评测集 harness。
//!
//! 目的：在 testcontainers MongoDB + mock LLM 下批量跑一组固定 query，统计
//! - 平均 rounds_used（应 ≤ 3）
//! - cited 命中率（每条 query 都标注了"理想 chunk_id"，命中即 +1）
//! - truncated / cancelled 比例
//! - 全程 LLM 调用次数
//!
//! 评测集刻意做小（5 条），方便快速回归；后续加 query 只需在 [`SCENARIOS`]
//! 末尾追加 [`EvalScenario`]。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, AnswerRequest, CatalogFilter};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const WS: &str = "ws_eval";

/// 一条固定评测样本。
struct EvalScenario {
    /// 给运营的自然语言 query。
    query: &'static str,
    /// 期待 agent 引用的 chunk title 集合（命中其一即视为 cited 命中）。
    expected_titles: &'static [&'static str],
    /// mock LLM 在本场景应输出的 action 序列（转 JSON 后入队）。
    /// 空表示用默认"open 第一条 → answer"两步。
    llm_steps: &'static [LlmStep],
}

#[derive(Clone, Copy)]
enum LlmStep {
    /// open 期待 chunk 中第 idx 条（按 expected_titles 顺序）。
    OpenExpected(usize),
    /// 直接 answer，cite 期待 chunk 中第 idx 条。
    AnswerExpected(usize),
}

const SCENARIOS: &[EvalScenario] = &[
    EvalScenario {
        query: "客户嫌价格贵怎么处理",
        expected_titles: &["三步价格异议处理"],
        llm_steps: &[LlmStep::OpenExpected(0), LlmStep::AnswerExpected(0)],
    },
    EvalScenario {
        query: "新客户首次跟进的开场白",
        expected_titles: &["新客开场白模板"],
        llm_steps: &[LlmStep::OpenExpected(0), LlmStep::AnswerExpected(0)],
    },
    EvalScenario {
        query: "客户已读不回如何唤回",
        expected_titles: &["已读不回唤回三阶段"],
        llm_steps: &[LlmStep::OpenExpected(0), LlmStep::AnswerExpected(0)],
    },
    EvalScenario {
        query: "复购客户如何升级套餐",
        expected_titles: &["复购升级路径"],
        llm_steps: &[LlmStep::OpenExpected(0), LlmStep::AnswerExpected(0)],
    },
    EvalScenario {
        query: "竞品对比怎么客观陈述",
        expected_titles: &["竞品对比方法论"],
        llm_steps: &[LlmStep::OpenExpected(0), LlmStep::AnswerExpected(0)],
    },
];

/// 评测集结果聚合。
#[derive(Debug, Default)]
struct EvalReport {
    total: usize,
    cited_hits: usize,
    rounds_sum: i32,
    truncated: usize,
    cancelled: usize,
    llm_calls_sum: usize,
}

impl EvalReport {
    fn cited_hit_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.cited_hits as f64 / self.total as f64
        }
    }

    fn avg_rounds(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.rounds_sum as f64 / self.total as f64
        }
    }
}

fn verified_chunk(title: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(format!("正文：{title}")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        dynamic_confidence: Some(0.9),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

/// E3 入口测试：跑完 SCENARIOS，断言聚合阈值。
///
/// 阈值：
/// - cited 命中率 ≥ 80%
/// - 平均 rounds_used ≤ 3
/// - 0 truncated / 0 cancelled
#[tokio::test]
#[ignore]
async fn knowledge_agent_eval_set_meets_thresholds() {
    let app = TestApp::start().await;
    // 评测集隔离：清表，再按场景顺序写入对应 chunk，把 chunk_id 记下来给 mock LLM。
    app.state
        .db
        .operation_knowledge_chunks()
        .delete_many(mongodb::bson::doc! { "workspaceId": WS }, None)
        .await
        .expect("clean ws_eval chunks");

    let mut report = EvalReport::default();
    let mut prev_calls: usize = 0;

    for scenario in SCENARIOS {
        // 每条 scenario 单独建 chunk，避免互相污染 catalog 排序。
        // 用 delete_many 把上一轮的 chunk 全清，保持 catalog 干净。
        app.state
            .db
            .operation_knowledge_chunks()
            .delete_many(mongodb::bson::doc! { "workspaceId": WS }, None)
            .await
            .expect("reset ws_eval chunks");

        let mut chunk_hexes: Vec<String> = Vec::new();
        for title in scenario.expected_titles {
            let chunk = verified_chunk(title);
            chunk_hexes.push(chunk.id.expect("oid").to_hex());
            app.state
                .db
                .operation_knowledge_chunks()
                .insert_one(&chunk, None)
                .await
                .expect("insert chunk");
        }

        // 把本场景 LLM 步骤入队。
        for step in scenario.llm_steps {
            match step {
                LlmStep::OpenExpected(idx) => {
                    let id = &chunk_hexes[*idx];
                    app.llm.push_response(json!({
                        "action": "open_chunk",
                        "ids": [id],
                    }));
                }
                LlmStep::AnswerExpected(idx) => {
                    let id = &chunk_hexes[*idx];
                    app.llm.push_response(json!({
                        "action": "answer",
                        "answer": format!("基于 {} 给出方案。", id),
                        "citedChunkIds": [id],
                        "sourceQuotes": [],
                    }));
                }
            }
        }

        let result = answer(
            &app.state,
            AnswerRequest {
                workspace_id: WS.to_string(),
                account_id: None,
                query: scenario.query.to_string(),
                filter: CatalogFilter::default(),
                max_rounds: None,
            },
        )
        .await
        .expect("answer must succeed");

        report.total += 1;
        report.rounds_sum += result.rounds_used;
        if result.truncated {
            report.truncated += 1;
        }
        if result.cancelled {
            report.cancelled += 1;
        }
        let calls_now = app.llm.calls();
        report.llm_calls_sum += calls_now - prev_calls;
        prev_calls = calls_now;

        // cited 命中：返回的任一 cited_chunk_id 必须落在本场景准备的 chunk_hexes 集合里。
        if result
            .cited_chunk_ids
            .iter()
            .any(|c| chunk_hexes.contains(c))
        {
            report.cited_hits += 1;
        }
    }

    eprintln!(
        "[eval] total={} hit_rate={:.2} avg_rounds={:.2} truncated={} cancelled={} llm_calls={}",
        report.total,
        report.cited_hit_rate(),
        report.avg_rounds(),
        report.truncated,
        report.cancelled,
        report.llm_calls_sum,
    );

    assert!(
        report.cited_hit_rate() >= 0.80,
        "cited 命中率不达标：{:.2} < 0.80",
        report.cited_hit_rate()
    );
    assert!(
        report.avg_rounds() <= 3.0,
        "平均 rounds 超阈值：{:.2} > 3.0",
        report.avg_rounds()
    );
    assert_eq!(report.truncated, 0, "评测集中不应出现 truncated 结果");
    assert_eq!(report.cancelled, 0, "评测集中不应出现 cancelled 结果");
    // 每条场景应该 LLM 调用 = open + answer = 2，5 条 = 10 次。允许 +/- 0 容差。
    assert_eq!(
        report.llm_calls_sum,
        SCENARIOS.len() * 2,
        "LLM 调用次数与场景预期不符"
    );
}
