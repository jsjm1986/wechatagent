//! SR-126：Knowledge 离线召回门必须由生产 catalog 排序决定候选。
//!
//! 金标只用于执行后的 recall@1 评分，绝不注入 mock LLM。测试先把相关条目与
//! 高静态分无关干扰项写入真实 Mongo，再调用生产 `list_catalog`；mock 只负责
//! 打开生产排序的第一项并引用其证据。排序若退化，引用会合法但金标 recall 会下降。

mod common;

use std::collections::HashMap;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use serde_json::json;
use wechatagent::agent::knowledge_agent::{answer, list_catalog, AnswerRequest, CatalogFilter};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

const WS: &str = "ws_eval";

struct EvalScenario {
    query: &'static str,
    expected_title: &'static str,
}

const SCENARIOS: &[EvalScenario] = &[
    EvalScenario {
        query: "客户嫌价格贵怎么处理",
        expected_title: "三步价格异议处理",
    },
    EvalScenario {
        query: "新客户首次跟进的开场白",
        expected_title: "新客开场白模板",
    },
    EvalScenario {
        query: "客户已读不回如何唤回",
        expected_title: "已读不回唤回三阶段",
    },
    EvalScenario {
        query: "复购客户如何升级套餐",
        expected_title: "复购升级路径",
    },
    EvalScenario {
        query: "竞品对比怎么客观陈述",
        expected_title: "竞品对比方法论",
    },
];

const DISTRACTORS: &[&str] = &[
    "仓库盘点与库存校准",
    "员工入职设备领取流程",
    "发票抬头修改规范",
];

#[derive(Debug, Default)]
struct EvalReport {
    total: usize,
    cited_hits: usize,
    top_rank_hits: usize,
    rounds_sum: i32,
    truncated: usize,
    cancelled: usize,
    llm_calls_sum: usize,
}

impl EvalReport {
    fn cited_hit_rate(&self) -> f64 {
        self.cited_hits as f64 / self.total.max(1) as f64
    }

    fn top_rank_hit_rate(&self) -> f64 {
        self.top_rank_hits as f64 / self.total.max(1) as f64
    }

    fn avg_rounds(&self) -> f64 {
        self.rounds_sum as f64 / self.total.max(1) as f64
    }
}

fn verified_chunk(title: &str, query_terms: &str, confidence: f64) -> OperationKnowledgeChunk {
    let source_quote = format!("证据：{title}；适用问题：{query_terms}");
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("{title} {query_terms}")),
        body: Some(format!("{source_quote}。正文说明。")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: Some(source_quote.clone()),
        source_anchors: vec![doc! { "sourceQuote": &source_quote }],
        dynamic_confidence: Some(confidence),
        priority: if confidence > 0.9 { 100 } else { 0 },
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

async fn exercise_eval(app: &TestApp) -> anyhow::Result<EvalReport> {
    let chunks = app.state.db.operation_knowledge_chunks();
    let mut report = EvalReport::default();
    let mut previous_calls = app.llm.calls();

    for scenario in SCENARIOS {
        chunks
            .delete_many(doc! { "workspace_id": WS }, None)
            .await?;
        let remaining = chunks
            .count_documents(doc! { "workspace_id": WS }, None)
            .await?;
        anyhow::ensure!(remaining == 0, "scenario reset left {remaining} chunks");

        let relevant = verified_chunk(scenario.expected_title, scenario.query, 0.35);
        let gold_id = relevant.id.expect("relevant oid").to_hex();
        let mut evidence_by_id = HashMap::new();
        evidence_by_id.insert(
            gold_id.clone(),
            relevant
                .source_quote
                .clone()
                .expect("relevant source quote"),
        );
        chunks.insert_one(&relevant, None).await?;

        for title in DISTRACTORS {
            let distractor = verified_chunk(title, "内部行政流程，与客户咨询无关", 0.99);
            let id = distractor.id.expect("distractor oid").to_hex();
            evidence_by_id.insert(
                id,
                distractor
                    .source_quote
                    .clone()
                    .expect("distractor source quote"),
            );
            chunks.insert_one(&distractor, None).await?;
        }

        let catalog = list_catalog(
            &app.state,
            WS,
            None,
            &CatalogFilter::default(),
            Some(scenario.query),
        )
        .await?;
        let selected_id = catalog
            .first()
            .map(|entry| entry.chunk_id.clone())
            .ok_or_else(|| anyhow::anyhow!("production catalog returned no candidates"))?;
        let selected_quote = evidence_by_id
            .get(&selected_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("catalog selected unknown chunk {selected_id}"))?;

        // mock 不知道 gold_id，只照生产排序第一项执行 open + answer。
        app.llm.push_response(json!({
            "action": "open_chunk",
            "ids": [selected_id.clone()],
        }));
        app.llm.push_response(json!({
            "action": "answer",
            "answer": "基于已打开的首候选给出方案。",
            "citedChunkIds": [selected_id.clone()],
            "sourceQuotes": [{
                "chunkId": selected_id.clone(),
                "quote": selected_quote,
                "sourceAnchorIndex": 0,
            }],
        }));

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
        .await?;

        report.total += 1;
        report.rounds_sum += result.rounds_used;
        report.truncated += usize::from(result.truncated);
        report.cancelled += usize::from(result.cancelled);
        let calls_now = app.llm.calls();
        report.llm_calls_sum += calls_now - previous_calls;
        previous_calls = calls_now;
        report.top_rank_hits += usize::from(selected_id == gold_id);
        report.cited_hits += usize::from(result.cited_chunk_ids.iter().any(|id| id == &gold_id));
    }

    Ok(report)
}

#[tokio::test]
#[ignore]
async fn knowledge_agent_eval_set_meets_thresholds() {
    let app = TestApp::start().await;
    let result = exercise_eval(&app).await;
    app.cleanup().await;

    let report = result.expect("exercise production-ranked knowledge eval");
    eprintln!(
        "[eval] total={} top_rank_hit={:.2} cited_hit={:.2} avg_rounds={:.2} truncated={} cancelled={} llm_calls={}",
        report.total,
        report.top_rank_hit_rate(),
        report.cited_hit_rate(),
        report.avg_rounds(),
        report.truncated,
        report.cancelled,
        report.llm_calls_sum,
    );

    assert_eq!(report.total, SCENARIOS.len(), "all scenarios must execute");
    assert!(
        report.top_rank_hit_rate() >= 0.80,
        "production catalog recall@1 below floor: {:.2}",
        report.top_rank_hit_rate()
    );
    assert!(
        report.cited_hit_rate() >= 0.80,
        "grounded cited recall below floor: {:.2}",
        report.cited_hit_rate()
    );
    assert!(report.avg_rounds() <= 3.0);
    assert_eq!(report.truncated, 0);
    assert_eq!(report.cancelled, 0);
    assert_eq!(report.llm_calls_sum, SCENARIOS.len() * 2);
}
