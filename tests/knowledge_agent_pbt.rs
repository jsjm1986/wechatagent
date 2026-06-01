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

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use proptest::prelude::*;
use wechatagent::agent::knowledge_agent::{
    classify_recall_outcome, filter_answer_against_opened, merge_catalog_pure, rank_key,
    split_prefetch, truncate_chars, wiki_type_priority, AnswerResult, CatalogEntry, RawSourceQuote,
};
use wechatagent::models::OperationKnowledgeChunk;
use wechatagent::knowledge_wiki::structural_proposals::{
    StructuralKind, StructuralProposal, STATUS_PENDING_REVIEW,
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

// ── Property 7：split_prefetch_preserves_input ───────────────────────

proptest! {
    /// follow_relations 把按发现顺序收集的关联目标切成「前 cap 个载正文 / 其余
    /// 转摘要」两段（split_prefetch）。锁死三条不变量：
    /// 1. prefetch.len() ≤ cap；
    /// 2. prefetch ⧺ rest 顺序拼接 == 原输入（不丢、不乱序、不重复——证明
    ///    follow_relations 不会因切分丢掉任何已 find_one 出的关联 chunk）。
    #[test]
    fn split_prefetch_preserves_input(
        items in proptest::collection::vec(0i32..1000, 0..20),
        cap in 0usize..10,
    ) {
        let original = items.clone();
        let (prefetch, rest) = split_prefetch(items, cap);
        prop_assert!(prefetch.len() <= cap, "prefetch len {} exceeds cap {}", prefetch.len(), cap);
        let mut roundtrip = prefetch;
        roundtrip.extend(rest);
        prop_assert_eq!(roundtrip, original, "prefetch ⧺ rest must equal input");
    }
}

// ── rank_key 生成器（Gap2：trust/recency + relevance 排序层） ─────────────

/// 构造一条最小可用 chunk，只参数化 rank_key 实际读到的字段。
fn mk_rank_chunk(
    title: &str,
    body: &str,
    wiki_type: &str,
    confidence: f64,
    priority: i32,
    superseded: bool,
    valid_to: Option<DateTime>,
) -> OperationKnowledgeChunk {
    let now = DateTime::now();
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: None,
        document_id: None,
        item_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: None,
        business_context: None,
        title: title.to_string(),
        summary: None,
        body: Some(body.to_string()),
        applicable_scenes: Vec::new(),
        not_applicable_scenes: Vec::new(),
        product_tags: Vec::new(),
        business_topics: Vec::new(),
        source_quote: None,
        source_anchors: Vec::new(),
        integrity_status: Some("verified".to_string()),
        confidence_score: None,
        status: "active".to_string(),
        priority,
        created_at: now,
        updated_at: now,
        wiki_type: Some(wiki_type.to_string()),
        domain_attributes: None,
        provenance: None,
        valid_from: None,
        valid_to,
        superseded_by: if superseded {
            Some("newer-id".to_string())
        } else {
            None
        },
        previous_version_id: None,
        related_chunks: None,
        usage_stats: None,
        dynamic_confidence: Some(confidence),
        integrity_score: None,
        locked_fields: None,
        chunk_type: "product_fact".to_string(),
    }
}

fn arb_wiki_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("thesis".to_string()),
        Just("methodology".to_string()),
        Just("entity".to_string()),
        Just("source".to_string()),
    ]
}

fn arb_rank_chunk() -> impl Strategy<Value = OperationKnowledgeChunk> {
    (
        "[a-z ]{0,12}",
        "[a-z ]{0,20}",
        arb_wiki_type(),
        0.0f64..1.0,
        0i32..100,
        any::<bool>(),
        proptest::option::of(-5i64..5i64),
    )
        .prop_map(|(title, body, wt, conf, pri, sup, days)| {
            let valid_to = days.map(|d| {
                let now_ms = DateTime::now().timestamp_millis();
                DateTime::from_millis(now_ms + d * 86_400_000)
            });
            mk_rank_chunk(&title, &body, &wt, conf, pri, sup, valid_to)
        })
}

// ── Property 8：rank_key_is_total_order ──────────────────────────────

proptest! {
    /// RankKey 派生 Ord，必须是全序：自反、反对称、可传递。这里锁「任意三元组
    /// 上比较自洽」——a≤b ∧ b≤c ⇒ a≤c，且 a 与自身恒等。保证 list_catalog 的
    /// sort_by 不会 panic（Rust 要求比较器全序，否则结果未定义/可能 panic）。
    #[test]
    fn rank_key_is_total_order(
        ca in arb_rank_chunk(),
        cb in arb_rank_chunk(),
        cc in arb_rank_chunk(),
        query in "[a-z ]{0,10}",
    ) {
        let now = DateTime::now();
        let ka = rank_key(&query, &ca, now);
        let kb = rank_key(&query, &cb, now);
        let kc = rank_key(&query, &cc, now);
        // 自反
        prop_assert_eq!(ka.cmp(&ka), std::cmp::Ordering::Equal);
        // 反对称：a<b ⇒ b>a
        if ka < kb {
            prop_assert!(kb > ka);
        }
        // 传递：a≤b ∧ b≤c ⇒ a≤c
        if ka <= kb && kb <= kc {
            prop_assert!(ka <= kc, "transitivity violated");
        }
    }
}

// ── Property 9：rank_key_superseded_demoted ──────────────────────────

proptest! {
    /// 「除 superseded 惩罚外全等的 live 同行」恒排在 superseded 之前
    ///（方法论点 4：superseded 绝不与现行版同列竞争）。构造同字段的一对 chunk，
    /// 仅 superseded_by 不同，断言 live > superseded。
    #[test]
    fn rank_key_superseded_demoted(
        title in "[a-z ]{1,12}",
        body in "[a-z ]{1,20}",
        wt in arb_wiki_type(),
        conf in 0.0f64..1.0,
        pri in 0i32..100,
        query in "[a-z ]{0,10}",
    ) {
        let now = DateTime::now();
        let live = mk_rank_chunk(&title, &body, &wt, conf, pri, false, None);
        let superseded = mk_rank_chunk(&title, &body, &wt, conf, pri, true, None);
        let kl = rank_key(&query, &live, now);
        let ks = rank_key(&query, &superseded, now);
        prop_assert!(kl > ks, "live peer must outrank superseded");
        prop_assert!(kl.live && !ks.live);
    }
}

// ── Property 10：rank_key_now_monotonic ──────────────────────────────

proptest! {
    /// now 单调：把比较时刻往后推（now 增大）只可能令更多 chunk 过期 → 同一 chunk
    /// 的排名键单调不升（never 升）。锁 valid_to 固定、now 前后两个时刻：
    /// rank_key(now_late) ≤ rank_key(now_early)。
    #[test]
    fn rank_key_now_monotonic(
        title in "[a-z ]{1,12}",
        body in "[a-z ]{1,20}",
        wt in arb_wiki_type(),
        conf in 0.0f64..1.0,
        pri in 0i32..100,
        valid_offset_days in -3i64..3,
        query in "[a-z ]{0,10}",
    ) {
        let base_ms = DateTime::now().timestamp_millis();
        let valid_to = Some(DateTime::from_millis(base_ms + valid_offset_days * 86_400_000));
        let chunk = mk_rank_chunk(&title, &body, &wt, conf, pri, false, valid_to);
        let now_early = DateTime::from_millis(base_ms - 10 * 86_400_000);
        let now_late = DateTime::from_millis(base_ms + 10 * 86_400_000);
        let k_early = rank_key(&query, &chunk, now_early);
        let k_late = rank_key(&query, &chunk, now_late);
        prop_assert!(
            k_late <= k_early,
            "advancing now must not raise rank: early={:?} late={:?}",
            k_early, k_late,
        );
    }
}

// ── classify_recall_outcome 生成器（Gap1：在线召回-trace 闭环） ─────────────

/// 构造一条 AnswerResult，trace 里按 opened_chunks / opened_bodies 塞
/// open_chunk / follow_relations 步——镜像 answer_inner 真实写法。
fn mk_answer_result(
    cited: Vec<String>,
    truncated: bool,
    cancelled: bool,
    opened_chunks: Vec<String>,
    opened_bodies: Vec<String>,
) -> AnswerResult {
    let mut trace: Vec<Document> = Vec::new();
    if !opened_chunks.is_empty() {
        trace.push(doc! { "tool": "open_chunk", "opened": opened_chunks });
    }
    if !opened_bodies.is_empty() {
        trace.push(doc! { "tool": "follow_relations", "openedBodies": opened_bodies });
    }
    AnswerResult {
        answer: String::new(),
        cited_chunk_ids: cited,
        source_quotes: Vec::new(),
        tool_trace: trace,
        rounds_used: 1,
        truncated,
        cancelled,
    }
}

fn arb_id_vec(max: usize) -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(arb_chunk_id(), 0..max)
}

// ── Property 11：classify_never_panics_and_affected_subset_opened ──────

proptest! {
    /// classify_recall_outcome 的两条核心不变量：
    /// 1. 永不 panic（容错读 trace，缺字段当空集）；
    /// 2. 若产出候选信号，其 affected_chunk_ids ⊆ 本次 opened（open_chunk.opened ∪
    ///    follow_relations.openedBodies）——绝不凭空引用未 open 的 chunk，落库时
    ///    尊重「signal 只指向真实召回过的原子」。
    #[test]
    fn classify_affected_subset_opened(
        cited in arb_id_vec(6),
        truncated in any::<bool>(),
        cancelled in any::<bool>(),
        opened_chunks in arb_id_vec(8),
        opened_bodies in arb_id_vec(6),
    ) {
        let opened: HashSet<String> = opened_chunks
            .iter()
            .chain(opened_bodies.iter())
            .cloned()
            .collect();
        let r = mk_answer_result(
            cited, truncated, cancelled,
            opened_chunks, opened_bodies,
        );
        if let Some(cand) = classify_recall_outcome(&r) {
            for id in &cand.affected_chunk_ids {
                prop_assert!(
                    opened.contains(id),
                    "affected id {} not in opened {:?}", id, opened,
                );
            }
            // kind 恒为两类签名之一
            prop_assert!(
                cand.kind == "recall_miss" || cand.kind == "recall_low_yield",
                "unexpected kind {}", cand.kind,
            );
        }
    }
}

// ── Property 12：classify_cancelled_always_none ──────────────────────

proptest! {
    /// 用户主动取消（cancelled=true）恒不产信号——取消不是召回质量问题。
    #[test]
    fn classify_cancelled_always_none(
        cited in arb_id_vec(6),
        truncated in any::<bool>(),
        opened_chunks in arb_id_vec(8),
        opened_bodies in arb_id_vec(6),
    ) {
        let r = mk_answer_result(
            cited, truncated, /*cancelled=*/ true,
            opened_chunks, opened_bodies,
        );
        prop_assert!(classify_recall_outcome(&r).is_none());
    }
}

// ── Property 13：classify_healthy_recall_none ────────────────────────

proptest! {
    /// 健康召回（未取消、未截断、cite ≥ 2 且 opened 与 cited 一致量级）恒不产
    /// 信号——避免对正常召回刷队列。构造 cited 全部来自 opened、cited≥2、
    /// opened==cited 集合的场景。
    #[test]
    fn classify_healthy_recall_none(
        ids in proptest::collection::hash_set(arb_chunk_id(), 2..6),
    ) {
        let v: Vec<String> = ids.into_iter().collect();
        // opened == cited，cite 数 ≥ 2 → 既非 miss（cited>0）也非 low_yield
        //（cited > LOW_YIELD_CITED_MAX=1）。
        let r = mk_answer_result(
            v.clone(), /*truncated=*/ false, /*cancelled=*/ false,
            v, Vec::new(),
        );
        prop_assert!(classify_recall_outcome(&r).is_none());
    }
}

// ── Property 14：structural_proposal_always_pending_review ────────────

fn arb_structural_kind() -> impl Strategy<Value = StructuralKind> {
    prop_oneof![
        Just(StructuralKind::Split),
        Just(StructuralKind::Merge),
        Just(StructuralKind::Reclassify),
        Just(StructuralKind::MarkSuperseded),
        Just(StructuralKind::RewriteDirectoryIntent),
    ]
}

proptest! {
    /// 安全语义红线（方法论点 6）：任意 kind / 任意 target / 任意 rationale 构造出的
    /// StructuralProposal，status 恒 pending_review，且序列化后绝无 apply/commit/delete
    /// 字段——结构化写物理上无法表达「已应用」，杜绝 auto-commit。
    #[test]
    fn structural_proposal_always_pending_review(
        kind in arb_structural_kind(),
        targets in proptest::collection::vec(arb_chunk_id(), 0..6),
        rationale in "[a-z ]{0,30}",
        source in prop_oneof![Just("rule"), Just("recall_trace"), Just("human")],
    ) {
        let p = StructuralProposal::new(
            "ws-pbt",
            kind,
            targets.clone(),
            rationale,
            source,
            None,
        );
        prop_assert_eq!(&p.status, STATUS_PENDING_REVIEW);
        prop_assert_eq!(p.kind.as_str(), kind.as_str());
        prop_assert_eq!(&p.target_chunk_ids, &targets);
        prop_assert!(!p.proposal_id.is_empty());
        let bson = mongodb::bson::to_document(&p).expect("serialize");
        for forbidden in ["apply", "applied", "commit", "committed", "delete", "deleted"] {
            prop_assert!(
                !bson.contains_key(forbidden),
                "proposal must not carry field {}", forbidden,
            );
        }
    }
}

// ── #620：rank_key 离线召回度量（recall@k / 命中率，确定性，无 LLM/Docker） ──
//
// Gap2 排序层的**质量**回归锁：前两个 rank_key property 锁的是「全序 / now 单调 /
// superseded 降格」这些代数不变量，但没回答「相关的 chunk 真的会被排进 top-k 吗」。
// 这里用一组确定性语料 + 已知 relevant 集，按 rank_key 排序后算 recall@k / hit@1，
// 锁住 #619 的核心承诺：query 相关度主导排序、static confidence/priority 不能把
// 强相关 chunk 挤出 top-k、superseded/expired 不毒化召回。

/// 对一组 chunk 按 rank_key(query) 降序排，返回排序后的 chunk 引用（top 在前）。
fn rank_sorted<'a>(
    query: &str,
    chunks: &'a [OperationKnowledgeChunk],
    now: DateTime,
) -> Vec<&'a OperationKnowledgeChunk> {
    let mut idx: Vec<&OperationKnowledgeChunk> = chunks.iter().collect();
    // rank_key 大者更优（effective_relevance 在前），故降序。
    idx.sort_by(|a, b| rank_key(query, b, now).cmp(&rank_key(query, a, now)));
    idx
}

/// recall@k：top-k 命中的 relevant 标题数 / relevant 总数。
fn recall_at_k(ranked: &[&OperationKnowledgeChunk], relevant: &HashSet<&str>, k: usize) -> f64 {
    if relevant.is_empty() {
        return 1.0;
    }
    let hit = ranked
        .iter()
        .take(k)
        .filter(|c| relevant.contains(c.title.as_str()))
        .count();
    hit as f64 / relevant.len() as f64
}

#[test]
fn metric_relevant_chunks_rank_into_top_k() {
    // 语料：3 条与 query「报价 折扣」强相关（标题/正文含信号），7 条噪声。
    // 关键反作弊：把强相关的 3 条 dynamic_confidence/priority 全压到最低、
    // 噪声条全拉到最高静态分——若排序只看 static（#619 的硬伤），相关条会被挤出
    // top-3。rank_key 必须靠 query 相关度把它们拉回头部。
    let now = DateTime::now();
    let mut corpus = Vec::new();
    // 强相关，故意 static 最差（conf=0.01, priority=0）。
    corpus.push(mk_rank_chunk("报价规则", "标准报价与折扣阶梯说明", "methodology", 0.01, 0, false, None));
    corpus.push(mk_rank_chunk("折扣政策", "大客户折扣报价审批", "methodology", 0.02, 0, false, None));
    corpus.push(mk_rank_chunk("报价折扣常见问答", "报价 折扣 如何组合", "finding", 0.01, 0, false, None));
    // 噪声，故意 static 最优（conf=0.99, priority=99）。
    for t in ["售后流程", "物流时效", "退换货", "会员等级", "门店地址", "营业时间", "联系方式"] {
        corpus.push(mk_rank_chunk(t, "与报价无关的运营资料", "entity", 0.99, 99, false, None));
    }

    let relevant: HashSet<&str> = ["报价规则", "折扣政策", "报价折扣常见问答"].into_iter().collect();
    let ranked = rank_sorted("报价 折扣", &corpus, now);

    // recall@3 必须满分：3 条强相关全部进 top-3，static 噪声压不住相关度。
    let r3 = recall_at_k(&ranked, &relevant, 3);
    assert!(
        (r3 - 1.0).abs() < f64::EPSILON,
        "recall@3 应为 1.0（相关度主导排序），实得 {}；排序后 top5={:?}",
        r3,
        ranked.iter().take(5).map(|c| c.title.as_str()).collect::<Vec<_>>(),
    );
    // hit@1：排第一的必是 relevant。
    assert!(
        relevant.contains(ranked[0].title.as_str()),
        "top-1 应命中 relevant，实得 {}",
        ranked[0].title,
    );
}

#[test]
fn metric_superseded_expired_do_not_poison_topk() {
    // 同一强相关内容存在 3 个版本：live / superseded / expired。三者 query 相关度
    // 全等，但 trust/recency 必须把 live 排到 superseded、expired 之前——即「永不删」
    // 不毒化召回（方法论点 4）。再混入噪声，断言 top-1 恒为 live 版。
    let now = DateTime::now();
    let past = DateTime::from_millis(now.timestamp_millis() - 7 * 86_400_000);
    let mut corpus = Vec::new();
    // 三版本同标题前缀内容，relevance 全等；只 trust/recency 不同。
    corpus.push(mk_rank_chunk("报价规则-现行", "报价 折扣 阶梯", "methodology", 0.5, 10, false, None));
    corpus.push(mk_rank_chunk("报价规则-旧版", "报价 折扣 阶梯", "methodology", 0.5, 10, true, None));
    corpus.push(mk_rank_chunk("报价规则-过期", "报价 折扣 阶梯", "methodology", 0.5, 10, false, Some(past)));
    // 噪声。
    for t in ["售后流程", "物流时效", "退换货"] {
        corpus.push(mk_rank_chunk(t, "与报价无关", "entity", 0.99, 99, false, None));
    }

    let ranked = rank_sorted("报价 折扣 阶梯", &corpus, now);
    assert_eq!(
        ranked[0].title, "报价规则-现行",
        "live 版必须排在 superseded/expired 同内容之前；实得 top3={:?}",
        ranked.iter().take(3).map(|c| c.title.as_str()).collect::<Vec<_>>(),
    );
    // live 版排名必须严格优于它的 superseded / expired 同内容版本。
    let pos = |title: &str| ranked.iter().position(|c| c.title == title).unwrap();
    assert!(pos("报价规则-现行") < pos("报价规则-旧版"), "live 必先于 superseded");
    assert!(pos("报价规则-现行") < pos("报价规则-过期"), "live 必先于 expired");
}

#[test]
fn metric_empty_query_degrades_to_static_order() {
    // query 为空（无检索意图）→ effective_relevance 恒 0，rank_key 退化为
    // 「live 优先 + wiki_priority + confidence + priority」静态序。锁这条退化路径：
    // 高 confidence 的 live chunk 排在低 confidence 之前，不 panic。
    let now = DateTime::now();
    let corpus = vec![
        mk_rank_chunk("低分", "x", "entity", 0.1, 1, false, None),
        mk_rank_chunk("高分", "y", "entity", 0.9, 1, false, None),
    ];
    let ranked = rank_sorted("", &corpus, now);
    assert_eq!(ranked[0].title, "高分", "空 query 应退化为高 confidence 优先");
}

