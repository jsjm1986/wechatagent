//! Plan v3 Commit 1：knowledge_agent 渐进式披露的 property-based 测试。
//!
//! 6 个 property（Plan §F + §8 锁定）：
//!
//! 1. **cited_subset_of_opened** — `filter_answer_against_opened` 输出的
//!    cited_chunk_ids 必须是 opened_seen 的子集；LLM 不许凭空创造未 open 的 chunk。
//! 2. **source_quotes_align** — 同函数里 quote.chunk_id 必须命中 opened_seen
//!    且非空；否则丢弃。剩余 quote 与 cited 不强制一一映射（允许 cite 但无 quote）。
//! 3. **filter_idempotent** — 对同一 (opened_seen, raw_cited, raw_quotes) 反复
//!    调用 filter_answer_against_opened，结果集合恒等（运营人 retry 同 query 时
//!    answer 行为可重现）。
//! 4. **catalog_merge_dedup_idempotent** — `merge_catalog_pure(target, target.clone())`
//!    后 target 长度不变；多次合并只去重不增长——保证 follow_relations 反复触发
//!    不会污染 catalog。
//! 5. **wiki_type_priority_total_order** — 9 类 wiki_type 形成严格的优先级偏序：
//!    thesis > synthesis > methodology > finding > comparison > concept > entity
//!    > source > query；None 等价 entity；未知字符串映射为 0。该顺序与
//!    knowledge_router::format_operation_knowledge_for_prompt 一致。
//! 6. **truncate_chars_cjk_safe** — 任意 CJK / ASCII 混合字符串截断到 N，
//!    输出 `chars().count()` ≤ N + 1（多出来的 1 是省略号 …）；不会切到 UTF-8
//!    多字节中间。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use std::collections::HashSet;

use proptest::prelude::*;
use wechatagent::agent::knowledge_agent::{
    filter_answer_against_opened, merge_catalog_pure, truncate_chars, wiki_type_priority,
    CatalogEntry, RawSourceQuote,
};

// ── 字符串生成器 ─────────────────────────────────────────────────────

fn arb_chunk_id() -> impl Strategy<Value = String> {
    "[a-z0-9]{1,4}".prop_map(|s| s.to_string())
}

fn arb_chunk_id_set(max: usize) -> impl Strategy<Value = HashSet<String>> {
    proptest::collection::hash_set(arb_chunk_id(), 0..max)
}

fn arb_chunk_id_vec(max: usize) -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(arb_chunk_id(), 0..max)
}

fn arb_raw_quote() -> impl Strategy<Value = RawSourceQuote> {
    (arb_chunk_id(), "[ -~]{0,20}", proptest::option::of(0i32..8))
        .prop_map(|(chunk_id, quote, source_anchor_index)| RawSourceQuote {
            chunk_id,
            quote,
            source_anchor_index,
        })
}

fn arb_raw_quote_vec(max: usize) -> impl Strategy<Value = Vec<RawSourceQuote>> {
    proptest::collection::vec(arb_raw_quote(), 0..max)
}

fn arb_catalog_entry() -> impl Strategy<Value = CatalogEntry> {
    (arb_chunk_id(), "[a-z]{1,5}").prop_map(|(chunk_id, title)| CatalogEntry {
        chunk_id,
        wiki_type: "methodology".to_string(),
        chunk_type: "product_fact".to_string(),
        title: title.clone(),
        summary: title,
        business_topics: Vec::new(),
        verified: true,
        has_source_quote: true,
        dynamic_confidence: 0.9,
        related_count: 0,
    })
}

fn arb_catalog_vec(max: usize) -> impl Strategy<Value = Vec<CatalogEntry>> {
    // 业务前提：catalog 来源于 MongoDB find()，按 _id 唯一；test 层用 chunk_id 去重
    // 模拟同一约束。允许空 vec，但不允许 vec 内有重复 chunk_id。
    proptest::collection::vec(arb_catalog_entry(), 0..max).prop_map(|items| {
        let mut seen: HashSet<String> = HashSet::new();
        items
            .into_iter()
            .filter(|e| seen.insert(e.chunk_id.clone()))
            .collect()
    })
}

// ── Property 1：cited_subset_of_opened ────────────────────────────────

proptest! {
    /// LLM 输出 cited_chunk_ids 经过滤后只剩 opened_seen 子集。
    #[test]
    fn cited_subset_of_opened(
        opened in arb_chunk_id_set(8),
        raw_cited in arb_chunk_id_vec(12),
    ) {
        let (cited, _) = filter_answer_against_opened(
            &opened,
            raw_cited,
            Vec::new(),
        );
        for id in &cited {
            prop_assert!(
                opened.contains(id),
                "cited id {} not in opened {:?}",
                id, opened,
            );
        }
    }
}

// ── Property 2：source_quotes_align ───────────────────────────────────

proptest! {
    /// quote.chunk_id 命中 opened_seen 且非空才保留。
    #[test]
    fn source_quotes_align(
        opened in arb_chunk_id_set(8),
        raw_quotes in arb_raw_quote_vec(10),
    ) {
        let (_, quotes) = filter_answer_against_opened(
            &opened,
            Vec::new(),
            raw_quotes,
        );
        for q in &quotes {
            prop_assert!(!q.chunk_id.is_empty(), "empty chunk_id should be dropped");
            prop_assert!(
                opened.contains(&q.chunk_id),
                "quote chunk_id {} not in opened {:?}",
                q.chunk_id, opened,
            );
        }
    }
}

// ── Property 3：filter_idempotent ────────────────────────────────────

proptest! {
    /// 同一 (opened, cited, quotes) 调 filter 两次结果集合恒等。
    #[test]
    fn filter_idempotent(
        opened in arb_chunk_id_set(8),
        raw_cited in arb_chunk_id_vec(12),
        raw_quotes in arb_raw_quote_vec(10),
    ) {
        let (cited_a, quotes_a) = filter_answer_against_opened(
            &opened,
            raw_cited.clone(),
            raw_quotes.clone(),
        );
        let (cited_b, quotes_b) = filter_answer_against_opened(
            &opened,
            raw_cited,
            raw_quotes,
        );
        let set_a: HashSet<String> = cited_a.into_iter().collect();
        let set_b: HashSet<String> = cited_b.into_iter().collect();
        prop_assert_eq!(set_a, set_b);
        let qids_a: HashSet<String> = quotes_a.into_iter().map(|q| q.chunk_id).collect();
        let qids_b: HashSet<String> = quotes_b.into_iter().map(|q| q.chunk_id).collect();
        prop_assert_eq!(qids_a, qids_b);
    }
}

// ── Property 4：catalog_merge_dedup_idempotent ───────────────────────

proptest! {
    /// merge_catalog_pure(target, target.clone()) 不会让 target 变长；
    /// merge 后再 merge 同一份 incoming 也不会再增长（去重幂等）。
    #[test]
    fn catalog_merge_dedup_idempotent(
        base in arb_catalog_vec(12),
        incoming in arb_catalog_vec(8),
    ) {
        // 第一次合并
        let mut a = base.clone();
        merge_catalog_pure(&mut a, incoming.clone());
        let len_after_first = a.len();
        // 把刚才合并完的拷贝当 incoming 再合一次：去重后不应增长
        let snapshot = a.clone();
        merge_catalog_pure(&mut a, snapshot);
        prop_assert_eq!(
            a.len(), len_after_first,
            "second merge should be idempotent",
        );
        // chunk_id 唯一
        let ids: HashSet<String> = a.iter().map(|e| e.chunk_id.clone()).collect();
        prop_assert_eq!(ids.len(), a.len());
    }
}

// ── Property 5：wiki_type_priority_total_order ───────────────────────

#[test]
fn wiki_type_priority_total_order() {
    let order = [
        "thesis",
        "synthesis",
        "methodology",
        "finding",
        "comparison",
        "concept",
        "entity",
        "source",
        "query",
    ];
    for w in order.windows(2) {
        let (hi, lo) = (w[0], w[1]);
        assert!(
            wiki_type_priority(Some(hi)) > wiki_type_priority(Some(lo)),
            "{hi} should outrank {lo}",
        );
    }
    // None 等价 entity
    assert_eq!(wiki_type_priority(None), wiki_type_priority(Some("entity")));
    // 未知字符串映射 0，严格低于所有 9 类
    let unknown = wiki_type_priority(Some("__bogus__"));
    for t in order {
        assert!(
            unknown < wiki_type_priority(Some(t)),
            "unknown should rank below {t}",
        );
    }
}

// ── Property 6：truncate_chars_cjk_safe ──────────────────────────────

proptest! {
    /// 任意 CJK / ASCII 混合字符串截断到 N，输出 chars().count() ≤ N + 1。
    /// 不会切到 UTF-8 多字节中间（Rust char 边界保证）。
    #[test]
    fn truncate_chars_cjk_safe(
        s in proptest::string::string_regex("[\\x{4e00}-\\x{9fff}a-zA-Z0-9 ]{0,40}").unwrap(),
        max_chars in 0usize..30,
    ) {
        let out = truncate_chars(&s, max_chars);
        let out_chars = out.chars().count();
        let in_chars = s.chars().count();
        if in_chars <= max_chars {
            // 未触发截断：输出 == 输入
            prop_assert_eq!(out, s);
        } else {
            // 触发截断：N + 1 个 char（多出来的 1 个是省略号）
            prop_assert_eq!(out_chars, max_chars + 1);
            prop_assert!(out.ends_with('…'));
        }
    }
}
