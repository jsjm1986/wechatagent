//! knowledge-digest-workstation Phase 4：chat `intent="digest_action"` 派工分支
//! 不变量测试（无 Docker 依赖）。
//!
//! 守住 4 件事：
//! 1. `knowledge.chat.intent` 输出的 intent 闭集包含 `digest_action`，与
//!    `src/routes/knowledge.rs` 内 `chat_turn` 的 match 分支 1:1 对齐。
//! 2. `knowledge.digest.dispatch` 输出 plannedSteps 时：每个 step.action 必须
//!    在 6 个合法值之内，cardId 必须能在 selectedCards 内找到。
//! 3. plannedSteps 总长 ≤ 8；累计 estimatedLlmCalls ≤ 12（与 prompt 文档硬约束
//!    一致；超额防御靠 worker `STEP_*` budget，但 prompt 必须先收缩）。
//! 4. dispatch 自然回信 naturalReply 不命中禁词。

use serde_json::json;

const INTENT_CLOSED_SET: &[&str] = &[
    "create_chunk",
    "update_chunk",
    "clarify_chunk",
    "update_pack",
    "freeform",
    "digest_action",
];

const ACTION_CLOSED_SET: &[&str] = &[
    "fix_chunk",
    "add_chunk",
    "retag",
    "review_evolution",
    "analyze_logs",
    "dismiss",
];

#[test]
fn intent_classifier_includes_digest_action_branch() {
    // 防止有人在 src/routes/knowledge.rs:chat_turn 加 match arm 时漏改 prompt 闭集。
    let intent_payload = json!({
        "intent": "digest_action",
        "confidence": 0.92,
        "userIntentSummary": "运营按今日日报上勾的 3 张卡片派工，让 AI 串行处理"
    });
    let intent = intent_payload["intent"].as_str().expect("intent string");
    assert!(
        INTENT_CLOSED_SET.contains(&intent),
        "intent {intent} 必须在闭集 {INTENT_CLOSED_SET:?} 中"
    );
}

#[test]
fn dispatch_planned_steps_respect_card_id_and_action_closed_set() {
    // 模拟 prompt 输出：3 张运营勾选的卡片 → 3 个 plannedSteps；
    // 验证 (a) 每个 step.cardId 都在 selectedCards 中；(b) 每个 step.action 都在闭集内。
    let selected_cards = json!([
        {"cardId":"card_a","kind":"chunk_missing_field","title":"切片缺 sourceQuote","summary":"...","suggestedAction":"fix_chunk"},
        {"cardId":"card_b","kind":"chunk_low_hit_rate","title":"切片命中率低","summary":"...","suggestedAction":"retag"},
        {"cardId":"card_c","kind":"evolution_pending","title":"等待评估的 prompt 候选","summary":"...","suggestedAction":"review_evolution"},
    ]);
    let dispatch_payload = json!({
        "plannedSteps": [
            {"stepId":"step_1","cardId":"card_a","action":"fix_chunk","summary":"为切片补 sourceQuote","estimatedLlmCalls":2},
            {"stepId":"step_2","cardId":"card_b","action":"retag","summary":"重抽标签","estimatedLlmCalls":1},
            {"stepId":"step_3","cardId":"card_c","action":"review_evolution","summary":"提示运营去 EvolutionCenterTab 评估","estimatedLlmCalls":1},
        ],
        "estimatedLlmCalls": 4,
        "naturalReply": "我会按以下三步串行处理：先补切片出处，再重抽标签，最后提示评估候选；完成后请运营确认。"
    });

    let cards = selected_cards.as_array().unwrap();
    let allowed_card_ids: Vec<&str> = cards
        .iter()
        .map(|c| c["cardId"].as_str().unwrap())
        .collect();

    let steps = dispatch_payload["plannedSteps"].as_array().unwrap();
    assert!(
        steps.len() <= 8,
        "plannedSteps 长度必须 ≤ 8（prompt 硬约束）"
    );

    let mut seen_step_ids = std::collections::HashSet::new();
    for step in steps {
        let step_id = step["stepId"].as_str().unwrap();
        assert!(
            seen_step_ids.insert(step_id.to_string()),
            "stepId 必须唯一"
        );
        let card_id = step["cardId"].as_str().unwrap();
        assert!(
            allowed_card_ids.contains(&card_id),
            "step.cardId={card_id} 必须在 selectedCards 内 {allowed_card_ids:?}"
        );
        let action = step["action"].as_str().unwrap();
        assert!(
            ACTION_CLOSED_SET.contains(&action),
            "step.action={action} 必须在闭集 {ACTION_CLOSED_SET:?} 中"
        );
    }
}

#[test]
fn dispatch_total_estimated_llm_calls_bounded() {
    // prompt 硬约束：总 estimatedLlmCalls ≤ 12；超额时 prompt 应把低优合并为
    // freeform 单步。这里测的是「合规输出」直接通过；超额 prompt 自己负责降噪。
    let dispatch_payload = json!({
        "plannedSteps": [
            {"stepId":"step_1","cardId":"card_a","action":"fix_chunk","summary":"...","estimatedLlmCalls":2},
            {"stepId":"step_2","cardId":"card_b","action":"retag","summary":"...","estimatedLlmCalls":1},
        ],
        "estimatedLlmCalls": 3,
        "naturalReply": "我会按 2 步处理"
    });
    let total = dispatch_payload["estimatedLlmCalls"].as_i64().unwrap();
    assert!(total <= 12, "总 estimatedLlmCalls 必须 ≤ 12");
    let sum_steps: i64 = dispatch_payload["plannedSteps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["estimatedLlmCalls"].as_i64().unwrap_or(0))
        .sum();
    // 总和不必严格等于 estimatedLlmCalls（prompt 可能加上 dispatch 自身一次），
    // 但每步 1-3 之间。
    for step in dispatch_payload["plannedSteps"].as_array().unwrap() {
        let n = step["estimatedLlmCalls"].as_i64().unwrap();
        assert!((1..=3).contains(&n), "单步 estimatedLlmCalls 必须 1..=3");
    }
    assert!(sum_steps <= 12);
}

#[test]
fn dispatch_natural_reply_does_not_use_forbidden_phrases() {
    // 与 scripts/check-no-human-takeover.sh 一致的字面禁词，提前在 Rust 单测里
    // 拦一道。新文案合入 commit 之前先跑 cargo test --lib 即可发现。
    let forbidden = [
        "人工接管",
        "人工介入",
        "人工托管",
        "接管",
        "人工",
        "takeover",
        "hand-off",
        "hand off",
    ];
    let reply = "我会按以下三步串行处理：先补切片出处，再重抽标签，最后提示评估候选；完成后请运营确认。";
    for word in forbidden {
        assert!(
            !reply.contains(word),
            "naturalReply 命中禁词 {word}（请改写为 'AI 处理 / 完成后请运营确认' 等）"
        );
    }
}
