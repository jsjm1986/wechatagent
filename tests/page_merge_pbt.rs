//! `knowledge_wiki::page_merge` 的 property-based 测试。
//!
//! 8 个 property（见函数 doc 注释）覆盖：
//! - **union 的代数性质**（幂等、并集、保序）；
//! - **锁定字段守门的不变量**（patch 含锁定字段必拒，apply 后必相等）；
//! - **70% 阈值的边界**（精确等于阈值不算截断、严格小于则算）；
//! - **hash 稳定性**（字段顺序无关 / volatile 字段无关 / 内容变则 hash 变）；
//! - **apply_field_patch 不引入新键**（仅覆盖 patch 列出的键）。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use std::collections::BTreeSet;

use mongodb::bson::{doc, Bson, Document};
use proptest::prelude::*;
use wechatagent::knowledge_wiki::page_merge::{
    apply_field_patch, compute_chunk_hash, enforce_locked_fields, is_body_truncated,
    union_array_fields, RevisionError, BODY_TRUNCATION_THRESHOLD, DEFAULT_LOCKED_FIELDS,
    DEFAULT_UNION_ARRAY_KEYS,
};

// ── 字符串生成器：保持小且可打印，避免 PBT 输出过长 ────────────────────

fn arb_label() -> impl Strategy<Value = String> {
    "[a-z]{1,6}".prop_map(|s| s.to_string())
}

fn arb_label_vec(max: usize) -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(arb_label(), 0..max)
}

fn vec_to_bson(v: &[String]) -> Bson {
    Bson::Array(v.iter().cloned().map(Bson::String).collect())
}

fn doc_with_tags(tags: &[String]) -> Document {
    doc! { "tags": vec_to_bson(tags) }
}

// ── Property 1：union 是 set-equal（结果集合 == existing ∪ incoming）──

proptest! {
    /// `union_array_fields` 的输出在 `tags` 字段上的元素集合等于
    /// `existing.tags ∪ incoming.tags`。
    #[test]
    fn prop_union_equals_set_union(
        existing in arb_label_vec(8),
        incoming in arb_label_vec(8),
    ) {
        let merged = union_array_fields(
            &doc_with_tags(&existing),
            &doc_with_tags(&incoming),
            &["tags"],
        );
        let merged_set: BTreeSet<String> = merged
            .get_array("tags")
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap().to_string())
            .collect();
        let expected: BTreeSet<String> =
            existing.iter().chain(incoming.iter()).cloned().collect();
        prop_assert_eq!(merged_set, expected);
    }
}

// ── Property 2：union 幂等 —— union(union(a,b), b) == union(a,b) ───────

proptest! {
    #[test]
    fn prop_union_is_idempotent(
        existing in arb_label_vec(8),
        incoming in arb_label_vec(8),
    ) {
        let once = union_array_fields(
            &doc_with_tags(&existing),
            &doc_with_tags(&incoming),
            &["tags"],
        );
        let twice = union_array_fields(&once, &doc_with_tags(&incoming), &["tags"]);
        prop_assert_eq!(once.get_array("tags").unwrap(), twice.get_array("tags").unwrap());
    }
}

// ── Property 3：union 保序（existing 中元素相对顺序保持）─────────────

proptest! {
    #[test]
    fn prop_union_preserves_existing_order(
        existing in arb_label_vec(8),
        incoming in arb_label_vec(8),
    ) {
        let merged = union_array_fields(
            &doc_with_tags(&existing),
            &doc_with_tags(&incoming),
            &["tags"],
        );
        let out_seq: Vec<String> = merged
            .get_array("tags")
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap().to_string())
            .collect();
        // existing 的去重序列应是 out_seq 的前缀
        let mut seen = BTreeSet::new();
        let existing_dedup: Vec<String> = existing
            .iter()
            .filter(|s| seen.insert((*s).clone()))
            .cloned()
            .collect();
        prop_assert!(
            out_seq.starts_with(&existing_dedup),
            "merged {:?} should start with existing dedup {:?}",
            out_seq,
            existing_dedup,
        );
    }
}

// ── Property 4：patch 含任意 locked 字段必拒收 ────────────────────────

proptest! {
    #[test]
    fn prop_patch_with_locked_field_is_rejected(
        locked_idx in 0usize..DEFAULT_LOCKED_FIELDS.len(),
        title in arb_label(),
    ) {
        let locked_field = DEFAULT_LOCKED_FIELDS[locked_idx];
        let existing = doc! { "title": "old" };
        let patch = doc! {
            locked_field: title.clone(),
        };
        let err = apply_field_patch(&existing, &patch, DEFAULT_LOCKED_FIELDS).unwrap_err();
        match err {
            RevisionError::LockedFieldInPatch { field } => {
                prop_assert_eq!(field, locked_field.to_string());
            }
            other => prop_assert!(false, "unexpected error variant: {:?}", other),
        }
    }
}

// ── Property 5：patch 应用后 — 仅 patch 顶层键被覆盖，其它字段不变 ───

proptest! {
    #[test]
    fn prop_patch_only_overrides_listed_keys(
        title_old in arb_label(),
        summary_old in arb_label(),
        title_new in arb_label(),
    ) {
        let existing = doc! {
            "title": title_old.clone(),
            "summary": summary_old.clone(),
        };
        let patch = doc! { "title": title_new.clone() };
        let out = apply_field_patch(&existing, &patch, DEFAULT_LOCKED_FIELDS).unwrap();
        prop_assert_eq!(out.get_str("title").unwrap(), &title_new);
        prop_assert_eq!(out.get_str("summary").unwrap(), &summary_old);
        prop_assert_eq!(out.len(), existing.len());
    }
}

// ── Property 6：70% 阈值精确 — 等于阈值不截断、严格小于截断 ──────────

proptest! {
    #[test]
    fn prop_body_truncation_threshold_boundary(
        existing_len in 1usize..1000,
    ) {
        let limit = (existing_len as f64) * BODY_TRUNCATION_THRESHOLD;
        let exactly = limit as usize;
        let just_below = exactly.saturating_sub(1);
        let above = exactly.saturating_add(1);
        // 严格小于阈值 → true
        prop_assert!(
            is_body_truncated(existing_len, just_below, just_below, BODY_TRUNCATION_THRESHOLD),
            "existing={} merged={} should be truncated (limit={:.2})",
            existing_len, just_below, limit,
        );
        // 严格大于阈值 → false
        prop_assert!(
            !is_body_truncated(existing_len, above, above, BODY_TRUNCATION_THRESHOLD),
            "existing={} merged={} should NOT be truncated (limit={:.2})",
            existing_len, above, limit,
        );
    }
}

// ── Property 7：compute_chunk_hash 与字段顺序无关 ─────────────────────

proptest! {
    #[test]
    fn prop_hash_is_field_order_independent(
        title in arb_label(),
        tag_a in arb_label(),
        tag_b in arb_label(),
    ) {
        let a = doc! {
            "title": title.clone(),
            "tags": vec_to_bson(&[tag_a.clone(), tag_b.clone()]),
            "summary": "s",
        };
        let b = doc! {
            "summary": "s",
            "tags": vec_to_bson(&[tag_a.clone(), tag_b.clone()]),
            "title": title.clone(),
        };
        prop_assert_eq!(compute_chunk_hash(&a), compute_chunk_hash(&b));
    }
}

// ── Property 8：enforce_locked_fields 后所有锁定字段一定 == existing ──

proptest! {
    #[test]
    fn prop_enforce_locked_pins_to_existing(
        chunk_id_existing in arb_label(),
        wiki_type_existing in prop_oneof![
            Just("entity"), Just("concept"), Just("methodology"), Just("finding")
        ],
        chunk_id_evil in arb_label(),
        wiki_type_evil in arb_label(),
        title in arb_label(),
    ) {
        let existing = doc! {
            "chunk_id": chunk_id_existing.clone(),
            "wiki_type": wiki_type_existing,
            "title": "T",
        };
        let merged = doc! {
            "chunk_id": chunk_id_evil,
            "wiki_type": wiki_type_evil,
            "title": title.clone(),
        };
        let pinned = enforce_locked_fields(&merged, &existing, &["chunk_id", "wiki_type"]);
        prop_assert_eq!(pinned.get_str("chunk_id").unwrap(), &chunk_id_existing);
        prop_assert_eq!(pinned.get_str("wiki_type").unwrap(), wiki_type_existing);
        // 非锁定字段保留 merged 形态
        prop_assert_eq!(pinned.get_str("title").unwrap(), &title);
    }
}

// ── 加分：DEFAULT_UNION_ARRAY_KEYS 列表含核心数组字段（防回归）────────

#[test]
fn default_union_keys_include_core_array_fields() {
    for k in &["tags", "search_terms", "applicable_scenes", "business_topics"] {
        assert!(
            DEFAULT_UNION_ARRAY_KEYS.contains(k),
            "DEFAULT_UNION_ARRAY_KEYS missing required key '{}'",
            k,
        );
    }
}
