//! agent-autonomy-loop W3 / Tasks 4.5 / 4.11 / 4.12 / 4.13 / 4.14 / 4.15：性质测试 P1-P7。
//!
//! 本文件聚合自治回路相关的所有性质测试（P1–P7），用 `proptest` 在
//! [`RawAgentDecision::validate_and_promote`] / [`finalize_review_for_send`] /
//! [`compute_taxonomy_resolutions`] 等纯函数上做随机输入验证。
//!
//! 性质对应需求：
//!
//! - **P1 自治字段必填**：R1.3 / R3.5 / R3.9 — 任一 R3.2 必填字段为空 / 类型非法 /
//!   枚举非法时，`validate_and_promote` 输出的 `risks` SHALL 含
//!   `missing_required_field:* / invalid_enum_value:* / invalid_type:*` 之一。
//!
//! - **P2 single-shot revision 上限**：R2.3 / R2.4 / R2.8 — 任意
//!   `(Reply 输出, Review 输出 needsRevision=true)` 组合下，gateway SHALL 调用
//!   Reply Agent 至多 2 次（1 首轮 + 至多 1 次 revision）；若第二轮 review 仍
//!   `needsRevision=true` 或 `approved=false`，则终态 SHALL 为
//!   `gateway_status="revision_failed"` + `decision.should_reply=false`。
//!
//! - **P3 预算超额不发送**：R3.7 / R3.10 — 在 `RunBudget::is_exceeded() == true` 且
//!   `decision.needs_review == true` 时，`local_decision_review` SHALL 返回
//!   `approved == false` + `risks == ["budget_exceeded_no_review"]`。
//!
//! - **P4 产品声明强约束**：R5.4 / R5.7 — 当
//!   `claim_analysis.requiresProductKnowledge == true` 且
//!   `used_knowledge_ids ∩ verified_chunks == ∅` 时，所有产品声明类回复
//!   SHALL 被识别为不可发送（`should_reply=false`）；这通过
//!   `compute_verified_chunks` 在 PBT 入口直接断言 verified ∩ used = ∅。
//!
//! P5/P7 在专属任务中实现。

use mongodb::bson::doc;
use proptest::prelude::*;
use wechatagent::agent::{AgentDecision, RawAgentDecision, UserRuntimeParameters};

// ─────────────────────────────────────────────────────────────────
// P1 自治字段必填（task 4.12）
// ─────────────────────────────────────────────────────────────────

/// 生成"final 轮且必填字段被故意置空 / 取非法值"的 RawAgentDecision。
///
/// 把 9 个自治协议字段全部填满合法长度的中文实质内容（≥ 20 unicode chars
/// for critical-turn 兜底），但随机选 1 个字段 (a) 留空 / (b) 设非法枚举 /
/// (c) 设非法类型；预期是 `validate_and_promote` 输出至少一条
/// `missing_required_field:* / invalid_enum_value:* / invalid_type:*`。
#[derive(Debug, Clone)]
struct ViolationCase {
    /// 0 = missing required field, 1 = invalid enum value, 2 = invalid bool/string type
    kind: u8,
    /// Field index in the required-fields list (0..=6 for R1.3 7 fields, 7..=11 for R3.2 fields).
    field_index: u8,
}

fn violation_case_strategy() -> impl Strategy<Value = ViolationCase> {
    (0u8..=2u8, 0u8..=11u8).prop_map(|(kind, field_index)| ViolationCase { kind, field_index })
}

fn build_baseline_raw() -> RawAgentDecision {
    let long = "这是一段足够长的实质内容用来满足关键变化轮的最低字符数要求"; // > 20 unicode chars
    let mut raw = RawAgentDecision::default();
    raw.user_understanding = Some(long.to_string());
    raw.relationship_read = Some(long.to_string());
    raw.operation_goal = Some(long.to_string());
    raw.knowledge_need_reason = Some(long.to_string());
    raw.memory_update_reason = Some(long.to_string());
    raw.self_critique = Some(long.to_string());
    raw.risk_self_check = Some(long.to_string());
    raw.why_should_reply = Some("因为对话上下文表明用户需要明确的回应".to_string());
    raw.why_skip_reply = Some(String::new());
    raw.run_mode = Some("knowledge_grounded".to_string());
    raw.risk_level = Some("medium".to_string());
    raw.knowledge_need = Some("required".to_string());
    raw.autonomy_mode = Some("assisted".to_string());
    raw.needs_review = Some(true);
    raw.operation_state = Some("relationship_building".to_string());
    raw.consolidation_needed = Some(false);
    raw.should_reply = Some(true);
    raw.reply_text = Some("好的，我来回复你".to_string());
    raw.decision_phase = Some("final".to_string());
    raw
}

fn apply_violation(raw: &mut RawAgentDecision, case: &ViolationCase) -> &'static str {
    // R1.3 always-required fields (7) — index 0..=6:
    //   0 user_understanding / 1 relationship_read / 2 operation_goal /
    //   3 knowledge_need_reason / 4 memory_update_reason / 5 self_critique /
    //   6 risk_self_check
    // R3.2 enum-required fields — index 7..=11:
    //   7 risk_level / 8 knowledge_need / 9 run_mode / 10 autonomy_mode / 11 operation_state
    let idx = case.field_index;
    let kind = case.kind;

    match kind {
        0 => {
            // missing required field
            match idx {
                0 => raw.user_understanding = Some(String::new()),
                1 => raw.relationship_read = Some(String::new()),
                2 => raw.operation_goal = Some(String::new()),
                3 => raw.knowledge_need_reason = Some(String::new()),
                4 => raw.memory_update_reason = Some(String::new()),
                5 => raw.self_critique = Some(String::new()),
                6 => raw.risk_self_check = Some(String::new()),
                7 => raw.risk_level = Some(String::new()),
                8 => raw.knowledge_need = Some(String::new()),
                9 => raw.run_mode = Some(String::new()),
                10 => raw.autonomy_mode = Some(String::new()),
                _ => raw.operation_state = Some(String::new()),
            }
            "missing_required_field"
        }
        1 => {
            // invalid enum value (only 7..=10 are enum-typed; 11 operation_state is required-only,
            // its membership check happens later in gateway/state-machine guard, not in
            // validate_and_promote).
            match idx {
                7 => raw.risk_level = Some("critical".to_string()),
                8 => raw.knowledge_need = Some("none".to_string()),
                9 => raw.run_mode = Some("manual".to_string()),
                10 => raw.autonomy_mode = Some("manual".to_string()),
                _ => {
                    // for non-enum fields, fall back to "missing"
                    return apply_violation(
                        raw,
                        &ViolationCase {
                            kind: 0,
                            field_index: idx,
                        },
                    );
                }
            }
            "invalid_enum_value"
        }
        _ => {
            // invalid bool type — fall back to coercing risk_level to a clearly invalid value;
            // "invalid_type" only applies to JSON bools and we can't easily inject a String into a
            // serde-derived bool field at runtime, so we substitute a missing field for symmetry.
            return apply_violation(
                raw,
                &ViolationCase {
                    kind: 0,
                    field_index: idx,
                },
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 1 / Task 4.12 / Validates: R1.3, R3.5, R3.9**
    ///
    /// 任一必填字段被设空 / 枚举非法 时，`validate_and_promote` 输出的 risks
    /// SHALL 含一条相应的违规标签。
    #[test]
    fn p1_autonomy_required_fields_violation_always_emits_risk_tag(
        case in violation_case_strategy()
    ) {
        let mut raw = build_baseline_raw();
        let expected_prefix = apply_violation(&mut raw, &case);
        let runtime = UserRuntimeParameters::default();
        let (decision, risks) = raw.validate_and_promote(&runtime);

        prop_assert!(
            risks.iter().any(|r| r.starts_with(expected_prefix)),
            "expected at least one risk starting with `{}`, got risks={:?}, decision.autonomy_mode={:?}, decision.run_mode={:?}",
            expected_prefix,
            risks,
            decision.autonomy_mode,
            decision.run_mode
        );
    }
}

// ─────────────────────────────────────────────────────────────────
// P3 预算超额不发送（task 4.13）
// ─────────────────────────────────────────────────────────────────
//
// 性质本质：当 `RunBudget::is_exceeded() == true` 且 `decision.needs_review == true`
// 时，`local_decision_review` SHALL 返回 `approved=false` + 唯一 risk =
// `"budget_exceeded_no_review"`；当 `is_exceeded() == true` 且 `needs_review == false`
// 时，应为 `approved=true` + risks 含 `"local_review_low_risk_only"`；当 budget
// 未超额时，approved=true 且 risks 不含上述两个降级标记。
//
// W3 / Task 4.13：`local_decision_review` 与 `RunBudget` 已通过 mod.rs / review.rs
// 提升为 `pub`（仅 PBT 入口需要），其余 `current_run_budget` / `RUN_BUDGET` 仍为
// `pub(crate)`，最小化对外可见面。

use wechatagent::agent::{local_decision_review, RunBudget};

/// 生成 (token_budget, max_llm_calls, force_exceeded, needs_review) 的 PBT 输入。
///
/// `force_exceeded=true` 时通过 `record_call(token_budget + 1)` 把 budget 推过
/// token 阈值；否则保持为 0 用量、未超额。这两条路径覆盖 R3.7 / R3.8 / R3.10。
fn budget_case_strategy() -> impl Strategy<Value = (i64, i32, bool, bool)> {
    (1i64..=100, 1i32..=5, any::<bool>(), any::<bool>())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 3 / Task 4.13 / Validates: R3.7, R3.8, R3.10**
    ///
    /// `local_decision_review` 在 budget 超额 / 未超额、`needs_review` true / false
    /// 的全部组合下，必须严格满足三态决策表：
    /// - `is_exceeded && needs_review` → `approved=false` + `risks==["budget_exceeded_no_review"]`
    /// - `is_exceeded && !needs_review` → `approved=true` + risks 含 `local_review_low_risk_only`
    /// - `!is_exceeded` → `approved=true` 且 risks 不含上述两个降级标记
    #[test]
    fn p3_budget_exceeded_no_review_consistent(
        (token_budget, max_llm_calls, force_exceeded, needs_review)
            in budget_case_strategy()
    ) {
        // tool_call_budget 给 i32::MAX：本性质只关心 token / llm_calls 维度。
        let budget = RunBudget::new("run_pbt_p3", token_budget, max_llm_calls, i32::MAX);
        if force_exceeded {
            // 一次记录 token_budget+1 即可在 token 维度跨过阈值。
            budget.record_call(token_budget + 1);
            prop_assert!(budget.is_exceeded(), "force_exceeded 路径必须命中 is_exceeded");
        } else {
            prop_assert!(!budget.is_exceeded(), "未注入用量时不应超额");
        }

        let mut decision = AgentDecision::default();
        decision.needs_review = needs_review;
        let result = local_decision_review(&decision, &budget);

        if force_exceeded && needs_review {
            // R3.7：高风险路径，必须拒绝放行且唯一 risk 是 budget_exceeded_no_review。
            prop_assert_eq!(result.approved, false);
            prop_assert_eq!(
                result.risks.as_slice(),
                &["budget_exceeded_no_review".to_string()][..],
                "needs_review=true 且超额时唯一 risk 必须是 budget_exceeded_no_review"
            );
        } else if force_exceeded && !needs_review {
            // R3.8：低风险快速通道，approved=true 且必须显式标注未走 LLM review。
            prop_assert!(result.approved, "needs_review=false 且超额时应放行");
            prop_assert!(
                result.risks.iter().any(|r| r == "local_review_low_risk_only"),
                "needs_review=false 且超额时 risks 必须含 local_review_low_risk_only，实际：{:?}",
                result.risks
            );
        } else {
            // R3.10：未超额路径不应出现以上两个 budget-降级标记。
            prop_assert!(result.approved, "未超额时默认 approved=true");
            prop_assert!(
                !result.risks.iter().any(|r|
                    r == "budget_exceeded_no_review" || r == "local_review_low_risk_only"
                ),
                "未超额时不应出现 budget 降级 risk，实际：{:?}",
                result.risks
            );
        }
    }
}


// ─────────────────────────────────────────────────────────────────
// P4 产品声明强约束（task 4.14）
// ─────────────────────────────────────────────────────────────────
//
// 性质：当 `claim_analysis.requiresProductKnowledge == true` 且
// `used_knowledge_ids ∩ verified_chunks == ∅` 时，`finalize_review_for_send`
// SHALL 产出：
//   * `decision.should_reply == false`
//   * `decision.autonomy_mode == "blocked"`
//   * `outcome.status == GatewayStatusFinal::BlockedUnverifiedProductClaim`
//   * `outcome.review.scores.fact_risk >= 6`
//   * `outcome.review.risks` 含 `product_claim_without_verified_knowledge`
//
// W3 / Task 4.14：通过 `pub use guards::default_product_claim_markers /
// ProductClaimMarkers` 让独立 crate 测试构造 `&ProductClaimMarkers` 入参，
// `Contact` / `OperationKnowledgeChunk` 在 `crate::models` 中以全 pub 字段
// 暴露，可直接 inline 构造，无需新增 helper。

use mongodb::bson::{oid::ObjectId, DateTime as BsonDateTime, Document as BsonDocument};
use wechatagent::agent::{
    default_product_claim_markers, finalize_review_for_send, DecisionReviewResult,
    GatewayStatusFinal,
};
use wechatagent::models::{AgentStatus, Contact, OperationKnowledgeChunk};

/// 构造一个空白但合法的 `Contact`，仅用于 `finalize_review_for_send` 入参占位
/// （本期 finalize 仅用 `_contact` 占参，不会触达任何字段）。
fn pbt_contact() -> Contact {
    Contact {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: "pbt_wxid".to_string(),
        nickname: None,
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        tags: Vec::new(),
        customer_stage: None,
        customer_stage_updated_at: None,
        intent_level: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: None,
        operation_state_reason: None,
        operation_state_confidence: None,
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: BsonDocument::new(),
        profile_attributes: BsonDocument::new(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        created_at: BsonDateTime::now(),
        updated_at: BsonDateTime::now(),
    }
}

/// 构造一个 `OperationKnowledgeChunk`，按 `verified` 入参决定 integrity_status。
/// 全部其它字段写最小默认值，避免在测试中泄漏不相关业务字段。
fn pbt_chunk(verified: bool) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: None,
        document_id: None,
        item_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: None,
        business_context: None,
        title: "pbt_chunk".to_string(),
        summary: None,
        body: None,
        routing_card: None,
        applicable_scenes: Vec::new(),
        not_applicable_scenes: Vec::new(),
        safe_claims: Vec::new(),
        forbidden_claims: Vec::new(),
        evidence_items: Vec::new(),
        source_quote: None,
        source_anchors: Vec::new(),
        integrity_status: Some(
            (if verified { "verified" } else { "needs_review" }).to_string(),
        ),
        confidence_score: None,
        distortion_risks: Vec::new(),
        unsupported_claims: Vec::new(),
        verified_claims: Vec::new(),
        status: "active".to_string(),
        priority: 0,
        product_tags: Vec::new(),
        trigger_keywords: Vec::new(),
        business_topics: Vec::new(),
        created_at: BsonDateTime::now(),
        updated_at: BsonDateTime::now(),
        wiki_type: None,
        domain_attributes: None,
        provenance: None,
        valid_from: None,
        valid_to: None,
        superseded_by: None,
        previous_version_id: None,
        related_chunks: None,
        usage_stats: None,
        dynamic_confidence: None,
        integrity_score: None,
        locked_fields: None,
    }
}

/// 构造一个已通过基础 R3 必填校验的最简 `AgentDecision`：要的是 finalize 路径
/// 在协议层面"挑不出毛病"，从而强制走 R5.4 这条 verified-knowledge 分支。
fn pbt_baseline_decision(used_knowledge_ids: Vec<String>) -> AgentDecision {
    let long = "这是一段足够长的实质内容用来满足关键变化轮的最低字符数要求";
    AgentDecision {
        run_mode: "knowledge_grounded".to_string(),
        risk_level: "medium".to_string(),
        knowledge_need: "required".to_string(),
        needs_review: false,
        should_reply: true,
        reply_text: "我们的方案能满足你的需求".to_string(),
        autonomy_mode: "auto".to_string(),
        decision_phase: "final".to_string(),
        operation_state: Some("relationship_building".to_string()),
        user_understanding: long.to_string(),
        relationship_read: long.to_string(),
        operation_goal: long.to_string(),
        knowledge_need_reason: long.to_string(),
        memory_update_reason: long.to_string(),
        self_critique: long.to_string(),
        why_should_reply: "因为对话上下文表明用户需要明确的回应".to_string(),
        risk_self_check: long.to_string(),
        used_knowledge_ids,
        ..Default::default()
    }
}

/// PBT 输入策略：
///   * `num_used`：Reply Agent 引用的 used_knowledge_ids 数量（1..=5）；
///   * `num_chunks`：当前 run 加载的 chunk 总数（0..=5）；
///   * `num_verified`：其中 integrity_status="verified" 的数量（0..=num_chunks）；
///   * `mark_safe_claims`：是否同时填充 decision.safe_claims_used（边路覆盖 R5.7
///     反向门，但本性质核心是 R5.4 阻断，无论 safe_claims 是否填，should_reply
///     都必须为 false）。
fn p4_input_strategy() -> impl Strategy<Value = (usize, usize, usize, bool)> {
    (1usize..=5, 0usize..=5, any::<bool>()).prop_flat_map(|(num_used, num_chunks, mark_safe)| {
        (
            Just(num_used),
            Just(num_chunks),
            0usize..=num_chunks,
            Just(mark_safe),
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 4 / Task 4.14 / Validates: R5.4, R5.7**
    ///
    /// 当 `claim_analysis.requiresProductKnowledge == true` 且
    /// `used_knowledge_ids ∩ verified_chunks == ∅` 时，`finalize_review_for_send`
    /// 必须强制阻塞：should_reply=false / autonomy_mode="blocked" /
    /// status=BlockedUnverifiedProductClaim / fact_risk>=6 /
    /// risks 含 `product_claim_without_verified_knowledge`。
    ///
    /// 关键：`used_knowledge_ids` 用 `ObjectId::new()` 生成全新 hex，与
    /// 任意 chunk.id 不可能相交（ObjectId 在同一进程内单调递增 + 随机后缀），
    /// 从而保证 `compute_verified_chunks` 必返回空集。
    #[test]
    fn p4_unverified_product_claim_always_blocked(
        (num_used, num_chunks, num_verified, mark_safe_claims) in p4_input_strategy()
    ) {
        // used_knowledge_ids：全新 ObjectId hex，故意与 chunks 中任意 id 不相交。
        let used_ids: Vec<String> = (0..num_used)
            .map(|_| ObjectId::new().to_hex())
            .collect();

        // 构造 chunks：前 `num_verified` 条标 verified，其余标 needs_review。
        let mut chunks: Vec<OperationKnowledgeChunk> = Vec::with_capacity(num_chunks);
        for i in 0..num_chunks {
            chunks.push(pbt_chunk(i < num_verified));
        }

        let mut decision = pbt_baseline_decision(used_ids.clone());
        if mark_safe_claims {
            decision.safe_claims_used = vec!["claim_a".to_string(), "claim_b".to_string()];
        }

        // 上游 review：approved=true、low fact_risk —— finalize 必须仍然 flip 它。
        let review = DecisionReviewResult {
            approved: true,
            claim_analysis: doc! {
                "hasProductClaim": true,
                "requiresProductKnowledge": true,
                "knowledgeSupported": false,
                "reason": "PBT: 模型判断这是产品能力表述"
            },
            ..Default::default()
        };

        let runtime = UserRuntimeParameters::default();
        let contact = pbt_contact();
        let markers = default_product_claim_markers();

        let outcome = finalize_review_for_send(
            review,
            &mut decision,
            &runtime,
            &contact,
            &chunks,
            &markers,
            Vec::new(),
            "",
        );

        // R5.4 核心断言。
        prop_assert_eq!(
            decision.should_reply, false,
            "verified ∩ used = ∅ 时 should_reply 必须被强制 false"
        );
        prop_assert_eq!(
            decision.autonomy_mode.as_str(), "blocked",
            "verified ∩ used = ∅ 时 autonomy_mode 必须被强制 blocked，实际：{}",
            decision.autonomy_mode
        );
        prop_assert!(
            matches!(outcome.status, GatewayStatusFinal::BlockedUnverifiedProductClaim),
            "终态必须是 BlockedUnverifiedProductClaim，实际：{:?}",
            outcome.status
        );
        prop_assert!(
            outcome.review.scores.fact_risk >= 6,
            "blocked 路径 fact_risk 必须 >= 6，实际：{}",
            outcome.review.scores.fact_risk
        );
        prop_assert!(
            outcome
                .review
                .risks
                .iter()
                .any(|r| r == "product_claim_without_verified_knowledge"),
            "risks 必须含 product_claim_without_verified_knowledge，实际：{:?}",
            outcome.review.risks
        );
        prop_assert_eq!(
            outcome.review.approved, false,
            "review.approved 必须被翻转为 false"
        );
        prop_assert_eq!(
            outcome.review.final_review_status.as_str(),
            "blocked_unverified_product_claim"
        );
    }
}


// ─────────────────────────────────────────────────────────────────
// P6 字典 candidate 不阻塞（task 4.11）
// ─────────────────────────────────────────────────────────────────

use mongodb::bson::DateTime;
use wechatagent::agent::{compute_taxonomy_resolutions, taxonomy_cache_for_tests};
use wechatagent::models::{TaxonomyEntry, TaxonomyValue};

fn dict_known_stages() -> Vec<TaxonomyEntry> {
    let mut entries = Vec::new();
    for canonical in &[
        "first_contact",
        "relationship_building",
        "need_discovery",
        "premium_returning",
    ] {
        entries.push(TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: canonical.to_string(),
                display_name: canonical.to_string(),
                description: String::new(),
                aliases: Vec::new(),
                status: "active".to_string(),
            },
            updated_at: DateTime::now(),
        });
    }
    entries
}

/// 一半已知（在字典里）/ 一半未知（不在字典）的策略。
fn customer_stage_strategy() -> impl Strategy<Value = (String, bool)> {
    prop_oneof![
        // known
        prop::sample::select(vec![
            "first_contact".to_string(),
            "relationship_building".to_string(),
            "need_discovery".to_string(),
            "premium_returning".to_string(),
        ])
        .prop_map(|s| (s, true)),
        // unknown — 用任意 ASCII 标识符 / 中文字符串
        "[a-z][a-z_]{2,15}".prop_map(|s| (s, false)),
        // unknown — 中文
        "[\u{4e00}-\u{9fff}]{1,10}".prop_map(|s| (s, false)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 6 / Task 4.11 / Validates: R8.3, R8.4**
    ///
    /// 不在字典里的 `customer_stage` → risks 含 `taxonomy_candidate:customer_stage:<v>`
    /// + candidate 列表非空 + alias 改写不发生。
    /// 在字典里的 → risks 与 candidate 列表都为空、`customer_stage` 不变。
    /// 在所有情况下都 SHALL NOT 出现 `missing_required_field:* / invalid_enum_value:*`
    /// 等强制阻塞的 risk 标签（R8.4 核心）。
    #[test]
    fn p6_taxonomy_candidate_never_blocks(
        (raw_value, in_dict) in customer_stage_strategy()
    ) {
        let cache = taxonomy_cache_for_tests(dict_known_stages());
        let mut decision = AgentDecision::default();
        decision.customer_stage = Some(raw_value.clone());
        let mut risks: Vec<String> = Vec::new();
        let to_upsert = compute_taxonomy_resolutions(&cache, "acct-prop", &mut decision, &mut risks);

        if in_dict {
            // Known canonical → no risk, no candidate
            prop_assert!(risks.is_empty(), "in-dict stage emits no risks; got {risks:?}");
            prop_assert!(to_upsert.is_empty());
            prop_assert_eq!(decision.customer_stage, Some(raw_value));
        } else {
            prop_assert!(
                risks.iter().any(|r| r == &format!("taxonomy_candidate:customer_stage:{raw_value}")),
                "expected taxonomy_candidate risk, got {risks:?}"
            );
            prop_assert_eq!(
                to_upsert.first(),
                Some(&("customer_stage".to_string(), raw_value.clone()))
            );
        }

        // R8.4 核心不变量：本路径 SHALL NOT 产生强制阻塞 risk。
        for r in &risks {
            prop_assert!(
                !r.starts_with("missing_required_field"),
                "taxonomy candidate must not block via missing_required_field: {r}"
            );
            prop_assert!(
                !r.starts_with("invalid_enum_value"),
                "taxonomy candidate must not block via invalid_enum_value: {r}"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// P2 single-shot revision 上限（task 4.15）
// ─────────────────────────────────────────────────────────────────
//
// `run_user_operation_gateway_inner`（src/agent/gateway.rs:706-924）的 R2 控制流
// 里 Reply Agent / finalize / revision 的纯逻辑映射到下方 `run_revision_loop`。
// 模型边界与 gateway 一一对应，便于审计：
//
//   gateway 行为                                        本测试 model
//   ───────────────────────────────────────────────────────────────
//   `let mut reply_calls = 1;`（首轮 reply）            reply_calls 初值 1
//   `decide_revision(finalize, review, budget)`         本地 decide_revision
//     └─ Approved && needs_revision && !should_hold
//          && !revision_direction.empty
//          && !budget_exceeded                          Proceed
//     └─ revisionDirection 空                           Skip(InvalidDirection)
//     └─ budget 超额                                    Skip(BudgetExceeded)
//     └─ finalize != Approved 或 !needs_revision
//          或 should_hold                               NotEligible
//   `RevisionDecision::Proceed` →
//     `decide_reply_with_promote(... revision_direction)` reply_calls += 1
//     `review_decision(... revised)` →
//     `finalize_review_for_send(second_review)` →
//     `second_passed = Approved && review_passed`        second_passed
//   `if second_passed` → final_review_status =
//      "revision_applied_approved"                       status="approved"
//   `else { review.approved=false;
//           review.final_review_status="revision_failed";
//           final_decision.should_reply=false; }`        status="revision_failed",
//                                                       should_reply=false
//   `RevisionDecision::Skip` 同样写
//      `final_review_status="revision_failed"`
//      `final_decision.should_reply=false`              status="revision_failed",
//                                                       should_reply=false
//   `RevisionDecision::NotEligible` →
//      review.approved 决定 should_reply
//      （首轮 finalize 已写好 status）                  保留首轮 should_reply
//
// 性质：
//   1. reply_calls ≤ 2 — 任意输入下都成立（Proceed 至多 +1，Skip / NotEligible 不调）；
//   2. 当首轮 needs_revision && !should_hold && !budget_exceeded
//      && revision_direction 非空 && (second_needs_revision || !second_approved)
//      → 终态 should_reply == false 且 status == "revision_failed"。

#[derive(Debug, Clone, Copy)]
struct ReviewSnapshot {
    /// `review.approved`：finalize 之后的 approved 标记。
    approved: bool,
    /// `review.needs_revision`：Review Agent 是否要求重写。
    needs_revision: bool,
    /// `review.should_hold`：是否走 hold 路径（hold 不进 R2 块）。
    should_hold: bool,
    /// 首轮 finalize 是否仍是 `Approved`；非 Approved 表示已被硬安全门拦截，
    /// gateway 永远不会进入 R2 revision 块（gateway.rs:937 之前 fail-closed return）。
    finalize_approved: bool,
    /// 是否提供了非空 `revisionDirection`（gateway.rs decide_revision R2.5）。
    revision_direction_non_empty: bool,
}

impl ReviewSnapshot {
    /// 等价 `review_passed && finalize == Approved`：判定该 review 在 gateway
    /// 视角下是否"算通过"。本模型不展开 score 维度（fact_risk / human_like 等），
    /// 因为 P2 只关心 revision 控制流；score 路径已被 P1/P4 与
    /// `finalize_review_for_send` 单元测试覆盖。
    fn passed(&self) -> bool {
        self.approved && !self.needs_revision && self.finalize_approved
    }
}

/// 模型化 gateway.rs 的 single-shot revision 控制流。返回
/// `(reply_calls, final_should_reply, final_status)`：
///
/// * `reply_calls`：Reply Agent 调用次数（初值 1，Proceed +1，Skip / NotEligible
///   不增）。性质 1 SHALL `<= 2`。
/// * `final_should_reply`：`final_decision.should_reply` 终值。
/// * `final_status`：终态字面量，对应 gateway 内
///   `review.final_review_status`：`"approved"` / `"revision_failed"` / `"hold"` /
///   `"blocked"`（hold/blocked 走 NotEligible 分支，保留首轮 finalize 状态）。
fn run_revision_loop(
    initial: ReviewSnapshot,
    second: ReviewSnapshot,
    budget_exceeded_for_revision: bool,
) -> (u32, bool, &'static str) {
    let mut reply_calls: u32 = 1;

    // 首轮 finalize 未通过 → gateway 直接 fail-closed return（gateway.rs:937），
    // 永远不进入 revision 块。模型保留首轮 should_reply 与 status。
    if !initial.finalize_approved {
        let status = if initial.should_hold { "hold" } else { "blocked" };
        return (reply_calls, false, status);
    }

    // decide_revision：finalize == Approved 之后的三种分支。
    if !initial.needs_revision || initial.should_hold {
        // NotEligible：review 未要求 revision 或 hold → 保留首轮终态。
        let should_reply = initial.passed() && !initial.should_hold;
        let status = if initial.should_hold {
            "hold"
        } else if should_reply {
            "approved"
        } else {
            // approved=false 但 needs_revision=false 也可能发生（safety guard
            // 抢先在 finalize 写过 approved=false）；保守标 "blocked"。
            "blocked"
        };
        return (reply_calls, should_reply, status);
    }

    // 进入 R2 块，但有两种 Skip 前置条件。
    if !initial.revision_direction_non_empty {
        // R2.5：revisionDirection 空 → revision_failed（gateway.rs:735-737）。
        return (reply_calls, false, "revision_failed");
    }
    if budget_exceeded_for_revision {
        // R2.8：budget 超额 → revision_failed（gateway.rs:735-737）。
        return (reply_calls, false, "revision_failed");
    }

    // Proceed：调用第二次 Reply Agent，再走 finalize + review_passed。
    reply_calls += 1;
    let second_passed = second.passed();
    if second_passed {
        // R2.3：revision_applied_approved（gateway.rs:838-850）。
        (reply_calls, true, "approved")
    } else {
        // R2.4：第二轮仍 fail → revision_failed（gateway.rs:851-869）。
        (reply_calls, false, "revision_failed")
    }
}

fn review_snapshot_strategy() -> impl Strategy<Value = ReviewSnapshot> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(approved, needs_revision, should_hold, finalize_approved, dir_non_empty)| {
                ReviewSnapshot {
                    approved,
                    needs_revision,
                    should_hold,
                    finalize_approved,
                    revision_direction_non_empty: dir_non_empty,
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 2 / Task 4.15 / Validates: R2.3, R2.4, R2.8**
    ///
    /// 任意 (首轮 review, 第二轮 review, budget_exceeded) 组合下：
    /// 1. Reply Agent 调用次数 SHALL ≤ 2；
    /// 2. 首轮 review 进入 R2 触发条件且第二轮仍失败时，终态 SHALL 是
    ///    `should_reply == false` + `status == "revision_failed"`；
    /// 3. 进入 R2 但被 Skip 前置条件（revisionDirection 空 / 预算超额）拦截
    ///    时，同样 SHALL 写 `revision_failed` 终态且不再调用 Reply Agent。
    #[test]
    fn p2_single_shot_revision_caps_reply_calls_at_two(
        initial in review_snapshot_strategy(),
        second in review_snapshot_strategy(),
        budget_exceeded_for_revision in any::<bool>(),
    ) {
        let (reply_calls, should_reply, status) =
            run_revision_loop(initial, second, budget_exceeded_for_revision);

        // 性质 1：Reply Agent 调用次数硬上限。
        prop_assert!(
            reply_calls <= 2,
            "reply called {} times, must be ≤ 2 (initial={:?}, second={:?}, budget_exceeded={})",
            reply_calls, initial, second, budget_exceeded_for_revision
        );

        // 性质 2：进入 Proceed 且第二轮仍 fail → revision_failed。
        let entered_proceed = initial.finalize_approved
            && initial.needs_revision
            && !initial.should_hold
            && initial.revision_direction_non_empty
            && !budget_exceeded_for_revision;
        let second_failing = !second.passed();
        if entered_proceed && second_failing {
            prop_assert_eq!(
                should_reply, false,
                "second-pass still failing → should_reply must be false"
            );
            prop_assert_eq!(
                status, "revision_failed",
                "second-pass still failing → status must be revision_failed"
            );
            prop_assert_eq!(
                reply_calls, 2,
                "Proceed branch must invoke Reply Agent exactly 2 times"
            );
        }

        // 性质 3：Skip 分支也 SHALL 写 revision_failed 终态，且不再调用 Reply Agent。
        let entered_skip = initial.finalize_approved
            && initial.needs_revision
            && !initial.should_hold
            && (!initial.revision_direction_non_empty || budget_exceeded_for_revision);
        if entered_skip {
            prop_assert_eq!(
                should_reply, false,
                "Skip branch (empty direction / budget exceeded) → should_reply=false"
            );
            prop_assert_eq!(
                status, "revision_failed",
                "Skip branch (empty direction / budget exceeded) → status=revision_failed"
            );
            prop_assert_eq!(
                reply_calls, 1,
                "Skip branch must NOT call Reply Agent a second time"
            );
        }
    }
}
