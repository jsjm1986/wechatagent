//! Phase B / B3 / B6：`format_operation_knowledge_for_prompt` 按 `chunk_type`
//! 分段输出的 property-based 测试。
//!
//! Plan 锁定：单文件 ≥ 64 cases。该文件 4 个 property，每个 default 256 cases，
//! 累计 1024，满足"≥ 64"门。
//!
//! 不依赖 testcontainers / mongodb / mock LLM —— 纯 in-memory 渲染，挂在
//! `cargo test --tests` 默认通道。
//!
//! Property 列表：
//!
//! 1. **section_order_is_stable** — 不论输入顺序，输出 section 顺序恒为
//!    `product_fact → style_template → peer_case → negative_example`。
//! 2. **every_chunk_appears_exactly_once** — 输出中每个 chunk title 出现且仅
//!    出现一次（不会因为分桶丢失，也不会重复）。
//! 3. **unknown_chunk_type_falls_back_to_product_fact** — 任意未知 chunk_type
//!    字符串都应被归入 `product_fact` bucket，输出不会出现新的 section header。
//! 4. **only_present_buckets_emit_headers** — 只有真正有 chunk 的 bucket 才
//!    会出现对应 header；空 bucket 不留空 header 行。

use mongodb::bson::{oid::ObjectId, DateTime};
use proptest::prelude::*;
use wechatagent::agent::format_operation_knowledge_for_prompt;
use wechatagent::models::OperationKnowledgeChunk;

// ── 已知 chunk_type 4 类（与 knowledge_router.rs 内部 order[] 对齐） ──

const KNOWN_TYPES: [&str; 4] = ["product_fact", "style_template", "peer_case", "negative_example"];

const HEADER_PRODUCT_FACT: &str = "【产品事实 product_fact】";
const HEADER_STYLE_TEMPLATE: &str = "【语气模板 style_template】";
const HEADER_PEER_CASE: &str = "【同行案例 peer_case】";
const HEADER_NEGATIVE_EXAMPLE: &str = "【反例 negative_example】";

fn mk_chunk(title: &str, chunk_type: &str) -> OperationKnowledgeChunk {
    let now = DateTime::now();
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: Some("default".to_string()),
        document_id: None,
        item_id: None,
        domain: "user".to_string(),
        knowledge_type: None,
        business_context: None,
        title: title.to_string(),
        summary: None,
        body: None,
        applicable_scenes: Vec::new(),
        not_applicable_scenes: Vec::new(),
        product_tags: Vec::new(),
        business_topics: Vec::new(),
        source_quote: None,
        source_anchors: Vec::new(),
        integrity_status: Some("verified".to_string()),
        confidence_score: Some(80),
        status: "active".to_string(),
        priority: 0,
        created_at: now,
        updated_at: now,
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
        chunk_type: chunk_type.to_string(),
    }
}

// 生成已知 chunk_type
fn arb_known_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("product_fact".to_string()),
        Just("style_template".to_string()),
        Just("peer_case".to_string()),
        Just("negative_example".to_string()),
    ]
}

// 唯一 title：用单调递增 index 拼出去重的 title，避免 property 2 误伤。
// 末尾加 `_END` sentinel，防止 substring 误匹配（如 `title_known_1` 是
// `title_known_10` 的子串）。
fn arb_chunk_with_known_type() -> impl Strategy<Value = (String, String)> {
    (0u32..10_000, arb_known_type())
        .prop_map(|(idx, t)| (format!("title_known_{idx}_END"), t))
}

// 任意（含未知）chunk_type 字符串：纯 ASCII 小写、长度 0..16，覆盖空串。
fn arb_any_type() -> impl Strategy<Value = String> {
    "[a-z_]{0,16}".prop_map(|s| s.to_string())
}

// ── Property 1：section_order_is_stable ──────────────────────────────────

proptest! {
    /// 不论 chunks 输入顺序，输出 section 顺序恒为
    /// product_fact → style_template → peer_case → negative_example。
    #[test]
    fn section_order_is_stable(
        chunks in proptest::collection::vec(arb_chunk_with_known_type(), 1..20)
    ) {
        let inputs: Vec<OperationKnowledgeChunk> = chunks
            .iter()
            .map(|(title, t)| mk_chunk(title, t))
            .collect();
        let s = format_operation_knowledge_for_prompt(&inputs);

        // 收集 *出现的* header 位置（缺失的 header 不参与单调性检查）。
        let header_in_priority_order = [
            HEADER_PRODUCT_FACT,
            HEADER_STYLE_TEMPLATE,
            HEADER_PEER_CASE,
            HEADER_NEGATIVE_EXAMPLE,
        ];
        let positions: Vec<usize> = header_in_priority_order
            .iter()
            .filter_map(|h| s.find(h))
            .collect();

        // 单调严格递增（每个 header 至多出现一次，所以是严格 <）。
        for w in positions.windows(2) {
            prop_assert!(
                w[0] < w[1],
                "section order broken: positions={:?}\n{}",
                positions,
                s
            );
        }
    }
}

// ── Property 2：every_chunk_appears_exactly_once ─────────────────────────

proptest! {
    /// 输出中每个 chunk 的 title 出现且仅出现一次（无丢失、无重复）。
    #[test]
    fn every_chunk_appears_exactly_once(
        chunks in proptest::collection::vec(arb_chunk_with_known_type(), 1..15)
    ) {
        // 先去重 title（同 idx 不同 type 时仍唯一，因为 idx 已唯一）
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for (title, t) in &chunks {
            if seen.insert(title.clone()) {
                deduped.push((title.clone(), t.clone()));
            }
        }
        let inputs: Vec<OperationKnowledgeChunk> = deduped
            .iter()
            .map(|(title, t)| mk_chunk(title, t))
            .collect();
        let s = format_operation_knowledge_for_prompt(&inputs);

        for (title, _t) in &deduped {
            let count = s.matches(title.as_str()).count();
            prop_assert_eq!(
                count,
                1,
                "title {} should appear exactly once, got {} times in:\n{}",
                title,
                count,
                s
            );
        }
    }
}

// ── Property 3：unknown_chunk_type_falls_back_to_product_fact ────────────

proptest! {
    /// 任意未知 chunk_type（含空串、纯 ascii、不在已知 4 类内的字符串）都应
    /// 被归入 `product_fact` bucket：输出至少含 product_fact header；不应
    /// 出现以未知类型命名的新 section header。
    #[test]
    fn unknown_chunk_type_falls_back_to_product_fact(
        title_idx in 0u32..10_000,
        unknown in arb_any_type().prop_filter(
            "must not be a known chunk_type",
            |s| !KNOWN_TYPES.contains(&s.as_str())
        ),
    ) {
        let title = format!("title_unknown_{title_idx}");
        let inputs = vec![mk_chunk(&title, &unknown)];
        let s = format_operation_knowledge_for_prompt(&inputs);

        prop_assert!(
            s.contains(HEADER_PRODUCT_FACT),
            "unknown chunk_type={:?} 必须落到 product_fact bucket，但输出无该 header:\n{}",
            unknown,
            s
        );
        // 不应出现以未知类型命名的新 section header（"【...{unknown}】"形态）。
        // 简化判定：unknown 字符串本身不应出现在 `chunkType=xxx` 之外的位置。
        // 由于 render_chunk 会输出 `chunkType=xxx`，这里只校验"不会出现新 header"。
        if !unknown.is_empty() {
            let header_like = format!("【{}】", unknown);
            prop_assert!(
                !s.contains(&header_like),
                "未知类型不应自创 header {}：\n{}", header_like, s
            );
        }
        // chunk title 仍必须出现一次。
        prop_assert!(s.contains(&title));
    }
}

// ── Property 4：only_present_buckets_emit_headers ────────────────────────

proptest! {
    /// 只有真正有 chunk 的 bucket 才会出现对应 header；空 bucket 不留空 header 行。
    #[test]
    fn only_present_buckets_emit_headers(
        present_mask in 1u8..16, // 1..=15，至少 1 个 bucket
    ) {
        // 用 4 位 mask 选取哪些 bucket 出现：
        // bit0=product_fact, bit1=style_template, bit2=peer_case, bit3=negative_example
        let mut inputs = Vec::new();
        for (i, t) in KNOWN_TYPES.iter().enumerate() {
            if (present_mask >> i) & 1 == 1 {
                inputs.push(mk_chunk(&format!("present_{}_{}", i, t), t));
            }
        }
        let s = format_operation_knowledge_for_prompt(&inputs);

        for (i, t) in KNOWN_TYPES.iter().enumerate() {
            let header = match *t {
                "product_fact" => HEADER_PRODUCT_FACT,
                "style_template" => HEADER_STYLE_TEMPLATE,
                "peer_case" => HEADER_PEER_CASE,
                "negative_example" => HEADER_NEGATIVE_EXAMPLE,
                _ => unreachable!(),
            };
            let should_be_present = (present_mask >> i) & 1 == 1;
            let actually_present = s.contains(header);
            prop_assert_eq!(
                actually_present,
                should_be_present,
                "bucket={} should_be_present={} actually_present={}\nmask={:04b}\n{}",
                t,
                should_be_present,
                actually_present,
                present_mask,
                s
            );
        }
    }
}
