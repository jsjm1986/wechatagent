//! v3 prompt-pack / Task 270：硬关键词快路径 (`compute_keyword_fastpath_hits`)
//! 单元测试。
//!
//! 不依赖 AppState / LLM / Mongo，纯函数 PBT 风格，覆盖三种命中态：
//!
//! 1. **精确命中**：trigger_keywords 中的关键词与 inbound 完全相同；
//! 2. **同义/子串命中**：inbound 是包含 trigger_keyword 的更长句子（中文长尾问法）；
//! 3. **不命中**：inbound 与所有 trigger_keywords 都不相关。
//!
//! 同时验证：
//! - 大小写不敏感（trigger_keyword 小写，inbound 大小写混杂）；
//! - 空 trigger_keywords 数组 / 空 inbound 不命中；
//! - 多个 chunk 同时命中时全部返回，每个 chunk 至多记 1 次（即便有多个关键词命中同一 chunk）；
//! - 无 ObjectId 的 chunk 跳过。

use mongodb::bson::{oid::ObjectId, DateTime};
use wechatagent::agent::compute_keyword_fastpath_hits;
use wechatagent::models::OperationKnowledgeChunk;

fn make_chunk(triggers: Vec<&str>) -> OperationKnowledgeChunk {
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
        title: "测试 chunk".to_string(),
        summary: None,
        body: None,
        routing_card: None,
        applicable_scenes: Vec::new(),
        not_applicable_scenes: Vec::new(),
        safe_claims: Vec::new(),
        forbidden_claims: Vec::new(),
        evidence_items: Vec::new(),
        product_tags: Vec::new(),
        trigger_keywords: triggers.into_iter().map(String::from).collect(),
        business_topics: Vec::new(),
        source_quote: None,
        source_anchors: Vec::new(),
        integrity_status: None,
        confidence_score: None,
        distortion_risks: Vec::new(),
        unsupported_claims: Vec::new(),
        verified_claims: Vec::new(),
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
    }
}

#[test]
fn exact_match_keyword_returns_single_hit() {
    let chunk = make_chunk(vec!["群发工具区别"]);
    let chunk_id = chunk.id.unwrap().to_hex();
    let hits = compute_keyword_fastpath_hits("群发工具区别", &[chunk]);

    assert_eq!(hits.len(), 1, "精确命中只返回 1 条 hit");
    assert_eq!(hits[0].0, chunk_id);
    assert_eq!(hits[0].1, "群发工具区别");
}

#[test]
fn substring_match_keyword_in_longer_chinese_question() {
    // 同义/口语化变体：trigger_keyword 是 "群发工具区别"，但用户可能说
    // "你们这个和群发工具区别在哪？" — 子串匹配命中。
    let chunk = make_chunk(vec!["群发工具区别"]);
    let chunk_id = chunk.id.unwrap().to_hex();
    let hits = compute_keyword_fastpath_hits(
        "你们这个和群发工具区别在哪呀，我之前用过别家的",
        &[chunk],
    );

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, chunk_id);
    assert_eq!(hits[0].1, "群发工具区别");
}

#[test]
fn no_match_returns_empty() {
    let chunk = make_chunk(vec!["群发工具区别", "和群发的不同", "你们和那个有啥不一样"]);
    let hits = compute_keyword_fastpath_hits("你好，今天天气真不错", &[chunk]);
    assert!(hits.is_empty(), "完全不相关的寒暄不应命中, hits={:?}", hits);
}

#[test]
fn case_insensitive_match() {
    // 全英文 trigger 测试 ascii 大小写折叠（中文本就没有大小写概念）。
    let chunk = make_chunk(vec!["wechatagent"]);
    let chunk_id = chunk.id.unwrap().to_hex();
    let hits = compute_keyword_fastpath_hits("WeChatAgent 这个产品多少钱", &[chunk]);

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, chunk_id);
    // 返回的 keyword 是 trigger 数组里那一条原值（也即 lowercase 形态）。
    assert_eq!(hits[0].1, "wechatagent");
}

#[test]
fn empty_inbound_returns_empty() {
    let chunk = make_chunk(vec!["产品价格"]);
    assert!(compute_keyword_fastpath_hits("", &[chunk.clone()]).is_empty());
    assert!(compute_keyword_fastpath_hits("   ", &[chunk.clone()]).is_empty());
    assert!(compute_keyword_fastpath_hits("\t\n  ", &[chunk]).is_empty());
}

#[test]
fn empty_trigger_keywords_does_not_match() {
    let chunk = make_chunk(vec![]);
    let hits = compute_keyword_fastpath_hits("产品价格多少", &[chunk]);
    assert!(hits.is_empty(), "空 trigger_keywords 不应命中, hits={:?}", hits);
}

#[test]
fn whitespace_only_trigger_skipped() {
    // trigger 含纯空白条目时应被跳过，且不影响其它有效条目命中。
    let chunk = make_chunk(vec!["  ", "\t", "群发"]);
    let chunk_id = chunk.id.unwrap().to_hex();
    let hits = compute_keyword_fastpath_hits("你们和群发工具有啥区别", &[chunk]);

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, chunk_id);
    assert_eq!(hits[0].1, "群发");
}

#[test]
fn each_chunk_records_at_most_one_hit_even_if_multiple_keywords_match() {
    // 同一 chunk 有 3 个关键词，inbound 包含其中 2 个 — 只记一条 hit
    // （break 掉内层循环），避免单 chunk 在 selected_chunks 中重复出现。
    let chunk = make_chunk(vec!["群发", "区别", "工具"]);
    let chunk_id = chunk.id.unwrap().to_hex();
    let hits = compute_keyword_fastpath_hits("你们这个和群发工具区别在哪呀", &[chunk]);

    assert_eq!(
        hits.len(),
        1,
        "单 chunk 至多 1 hit 即便多关键词同时命中, hits={:?}",
        hits
    );
    assert_eq!(hits[0].0, chunk_id);
    // 第一个命中 keyword 应为 "群发"（按 trigger_keywords 顺序遍历）
    assert_eq!(hits[0].1, "群发");
}

#[test]
fn multiple_chunks_each_independently_match() {
    // 两个 chunk 分别有不同关键词，inbound 同时包含两边的关键词 → 两条 hit。
    let chunk_a = make_chunk(vec!["产品价格"]);
    let chunk_b = make_chunk(vec!["部署方式"]);
    let id_a = chunk_a.id.unwrap().to_hex();
    let id_b = chunk_b.id.unwrap().to_hex();

    let hits = compute_keyword_fastpath_hits(
        "我想了解一下产品价格和部署方式都是怎样的",
        &[chunk_a, chunk_b],
    );

    assert_eq!(hits.len(), 2, "两个 chunk 各自命中, hits={:?}", hits);
    let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&id_a.as_str()));
    assert!(ids.contains(&id_b.as_str()));
}

#[test]
fn chunk_without_object_id_is_skipped() {
    let mut chunk = make_chunk(vec!["产品价格"]);
    chunk.id = None;
    let hits = compute_keyword_fastpath_hits("产品价格多少", &[chunk]);
    assert!(hits.is_empty(), "无 ObjectId 的 chunk SHALL 跳过, hits={:?}", hits);
}

#[test]
fn empty_chunks_array_returns_empty() {
    let hits = compute_keyword_fastpath_hits("产品价格", &[]);
    assert!(hits.is_empty());
}
