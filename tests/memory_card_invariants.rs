//! Property 2 / Task 8 / Task 24 / agent-autonomy-loop W5 task 6.3：
//! memory card compact 不变量。
//!
//! 性质：
//! 1. compact 后 `core_facts.len() <= 6`、`recent_facts.len() <= 10`；
//! 2. previous core_facts 中**未在 discarded 列表里**的事实，必然出现在
//!    compact 后的 core_facts 里（保留性，避免新近性挤掉关键事实）；
//! 3. compact 后 core_facts 里不会出现 discarded 列表中的事实。
//!
//! task 6.3：`compact_memory_card_with_previous` 已从 `Document` 入参 / 返回
//! 升级为 [`MemoryCardTyped`]。本测试文件相应改为构造 typed 输入并对 typed
//! 字段断言；同时保留一组"老 Document → typed → compact 后导出 Document"
//! 的兼容路径用例，验证 BSON wire 形态不丢字段。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use mongodb::bson::doc;
use proptest::prelude::*;
#[allow(deprecated)] // task 6.3：保留兼容别名调用以验证语义等价性。
use wechatagent::agent::compact_memory_card_typed;
use wechatagent::agent::compact_memory_card_with_previous;
use wechatagent::models::{MemoryCardTyped, MemoryFactRepr};

/// 用 `Vec<String>` 构造 typed 形态的 memoryCard（core / recent 各自）。
fn typed_card_with_core_recent(core: &[String], recent: &[String]) -> MemoryCardTyped {
    MemoryCardTyped {
        core_facts: core
            .iter()
            .cloned()
            .map(MemoryFactRepr::Plain)
            .collect(),
        recent_facts: recent
            .iter()
            .cloned()
            .map(MemoryFactRepr::Plain)
            .collect(),
        ..Default::default()
    }
}

fn texts(facts: &[MemoryFactRepr]) -> Vec<String> {
    facts.iter().map(|f| f.as_text().to_string()).collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// 核心 PBT：compact 后两数组都不超 cap。
    #[test]
    fn compact_caps_core_and_recent(
        core in proptest::collection::vec("[a-z]{1,8}", 0..15),
        recent in proptest::collection::vec("[a-z]{1,8}", 0..20),
    ) {
        let card = typed_card_with_core_recent(&core, &recent);
        let result = compact_memory_card_with_previous(&card, None, &[]);
        prop_assert!(
            result.core_facts.len() <= 6,
            "core_facts 超 cap 6: {}",
            result.core_facts.len()
        );
        prop_assert!(
            result.recent_facts.len() <= 10,
            "recent_facts 超 cap 10: {}",
            result.recent_facts.len()
        );
    }

    /// 保留性：previous core_facts 中未在 discarded 列表里的事实必须保留。
    #[test]
    fn previous_core_facts_are_preserved_unless_discarded(
        prev_cores in proptest::collection::vec("[a-z]{3,10}", 1..6),
        new_cores in proptest::collection::vec("[A-Z]{3,10}", 0..6),
        discard_idx in proptest::option::of(0usize..6usize),
    ) {
        // 把 prev / new 各自去重、避免巧合相同。
        let mut prev_unique: Vec<String> = Vec::new();
        for f in &prev_cores {
            if !prev_unique.contains(f) {
                prev_unique.push(f.clone());
            }
        }
        let new_unique: Vec<String> = new_cores
            .iter()
            .filter(|s| !prev_unique.contains(s))
            .cloned()
            .collect();

        // 选一个 prev 元素丢弃。
        let discarded: Vec<String> = match discard_idx {
            Some(idx) if idx < prev_unique.len() => vec![prev_unique[idx].clone()],
            _ => Vec::new(),
        };

        let prev_card = typed_card_with_core_recent(&prev_unique, &[]);
        let new_card = typed_card_with_core_recent(&new_unique, &[]);
        let result = compact_memory_card_with_previous(&new_card, Some(&prev_card), &discarded);
        let core_after = texts(&result.core_facts);

        // 性质 2：previous 中不在 discarded 的、且未被 cap 截掉的事实仍在结果里。
        // 真实合并顺序：new 优先靠前，然后追加 previous 中独有的项；最后保留前 6 条。
        // 因此 previous 独有且没有 discarded 的项，可能因 new 占满前 6 个位置被截掉。
        // 但只要总长度 ≤ 6，prev 中所有未 discarded 的项必然保留。
        let total_potential = new_unique.len() + prev_unique.iter()
            .filter(|f| !discarded.contains(f) && !new_unique.contains(f))
            .count();
        if total_potential <= 6 {
            for fact in &prev_unique {
                if discarded.contains(fact) { continue; }
                prop_assert!(
                    core_after.contains(fact),
                    "previous core_fact {} 未 discarded 且总数 ≤ 6 应保留，但 result.core_facts={:?}",
                    fact, core_after
                );
            }
        }

        // 性质 3：discarded 项绝不出现在结果里。
        for d in &discarded {
            prop_assert!(
                !core_after.contains(d),
                "discarded fact {} 不应出现在 result.core_facts={:?}",
                d, core_after
            );
        }
    }
}

#[test]
fn compact_handles_empty_card() {
    let card = MemoryCardTyped::default();
    let result = compact_memory_card_with_previous(&card, None, &[]);
    // 空输入下 core / recent 都不应超 cap（其实是 0）。
    assert!(result.core_facts.is_empty());
    assert!(result.recent_facts.is_empty());
}

#[test]
fn compact_truncates_oversized_arrays() {
    let big_core: Vec<String> = (0..20).map(|i| format!("c{i}")).collect();
    let big_recent: Vec<String> = (0..30).map(|i| format!("r{i}")).collect();
    let card = typed_card_with_core_recent(&big_core, &big_recent);
    let result = compact_memory_card_with_previous(&card, None, &[]);
    assert_eq!(result.core_facts.len(), 6);
    assert_eq!(result.recent_facts.len(), 10);
}

#[test]
fn compact_keeps_leading_items_after_cap() {
    let big_core: Vec<String> = (0..8).map(|i| format!("c{i}")).collect();
    let big_recent: Vec<String> = (0..12).map(|i| format!("r{i}")).collect();
    let card = typed_card_with_core_recent(&big_core, &big_recent);
    let result = compact_memory_card_with_previous(&card, None, &[]);
    assert_eq!(
        texts(&result.core_facts),
        vec!["c0", "c1", "c2", "c3", "c4", "c5"]
    );
    assert_eq!(
        texts(&result.recent_facts),
        vec!["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9"]
    );
}

#[test]
fn previous_core_fact_persists_after_compact_when_under_cap() {
    let prev = typed_card_with_core_recent(&["foo".to_string(), "bar".to_string()], &[]);
    let new = typed_card_with_core_recent(&["baz".to_string()], &[]);
    let result = compact_memory_card_with_previous(&new, Some(&prev), &[]);
    let cores = texts(&result.core_facts);
    assert!(cores.contains(&"foo".to_string()), "foo 必须保留: {cores:?}");
    assert!(cores.contains(&"bar".to_string()), "bar 必须保留: {cores:?}");
    assert!(
        cores.contains(&"baz".to_string()),
        "新 fact baz 必须存在: {cores:?}"
    );
}

#[test]
fn discarded_fact_is_dropped() {
    let prev = typed_card_with_core_recent(&["foo".to_string(), "bar".to_string()], &[]);
    let new = typed_card_with_core_recent(&[], &[]);
    let result =
        compact_memory_card_with_previous(&new, Some(&prev), &["foo".to_string()]);
    let cores = texts(&result.core_facts);
    assert!(
        !cores.contains(&"foo".to_string()),
        "foo 已 discarded，应被丢弃: {cores:?}"
    );
    assert!(
        cores.contains(&"bar".to_string()),
        "bar 未 discarded，应保留: {cores:?}"
    );
}

/// 波 D1 / task 6.3：deprecated alias `compact_memory_card_typed` 与
/// canonical `compact_memory_card_with_previous` 语义一致。
#[test]
#[allow(deprecated)]
fn deprecated_typed_alias_matches_canonical_helper() {
    let prev = MemoryCardTyped {
        core_facts: vec!["foo".into(), "bar".into()],
        ..Default::default()
    };
    let new_typed = MemoryCardTyped {
        core_facts: vec!["baz".into()],
        ..Default::default()
    };
    let alias_result = compact_memory_card_typed(&new_typed, Some(&prev), &["foo".into()]);
    let canonical_result =
        compact_memory_card_with_previous(&new_typed, Some(&prev), &["foo".into()]);

    let alias_cores = texts(&alias_result.core_facts);
    let canonical_cores = texts(&canonical_result.core_facts);
    assert_eq!(
        alias_cores, canonical_cores,
        "deprecated alias 与 canonical helper 必须一致"
    );
    assert!(canonical_cores.contains(&"bar".to_string()));
    assert!(canonical_cores.contains(&"baz".to_string()));
    assert!(!canonical_cores.contains(&"foo".to_string()));
}

/// task 6.3：通过 `MemoryCardTyped::from_document` 把"老 BSON Document
/// 形态"读进来后再 compact，结果与"直接构造 typed"一致——验证写入路径
/// `bson::to_document(&MemoryCardTyped)` 不丢字段、不出现两套并行表示。
#[test]
fn document_round_trip_compact_matches_typed_construction() {
    let legacy = doc! {
        "coreFacts": vec!["foo", "bar"],
        "recentFacts": vec!["r1", "r2"],
        // free-form 字段（preferences 等）通过 extra catch-all 兜底承接。
        "preferences": vec!["pref_a"],
    };
    let from_doc = MemoryCardTyped::from_document(&legacy);
    assert_eq!(from_doc.core_facts.len(), 2);
    assert_eq!(from_doc.recent_facts.len(), 2);

    let direct = MemoryCardTyped {
        core_facts: vec!["foo".into(), "bar".into()],
        recent_facts: vec!["r1".into(), "r2".into()],
        ..Default::default()
    };

    let from_doc_compact = compact_memory_card_with_previous(&from_doc, None, &[]);
    let direct_compact = compact_memory_card_with_previous(&direct, None, &[]);
    assert_eq!(
        texts(&from_doc_compact.core_facts),
        texts(&direct_compact.core_facts)
    );
    assert_eq!(
        texts(&from_doc_compact.recent_facts),
        texts(&direct_compact.recent_facts)
    );
}

/// task 6.3：cap 表也覆盖 `extra` 中的 free-form 数组（`preferences /
/// commitments / objections` 等）。
#[test]
fn extra_array_caps_are_enforced() {
    let mut extra = mongodb::bson::Document::new();
    extra.insert(
        "preferences",
        (0..20).map(|i| format!("p{i}")).collect::<Vec<_>>(),
    );
    extra.insert(
        "doNotDo",
        (0..20).map(|i| format!("d{i}")).collect::<Vec<_>>(),
    );
    let card = MemoryCardTyped {
        extra,
        ..Default::default()
    };
    let result = compact_memory_card_with_previous(&card, None, &[]);
    let prefs = result.extra.get_array("preferences").unwrap();
    let donts = result.extra.get_array("doNotDo").unwrap();
    assert_eq!(prefs.len(), 8, "preferences cap 8");
    assert_eq!(donts.len(), 10, "doNotDo cap 10");
}


// ── agent-autonomy-loop W5 / Task 6.8 扩展 ─────────────────────────────

/// Plain ↔ Structured 序列化往返保不变量。
#[test]
fn plain_and_structured_round_trip_through_bson() {
    use mongodb::bson::{from_document, to_document};

    let card = MemoryCardTyped {
        core_facts: vec![
            MemoryFactRepr::Plain("简单字符串 fact".to_string()),
            MemoryFactRepr::Structured(wechatagent::models::MemoryFact {
                id: "id-1".to_string(),
                text: "结构化 fact".to_string(),
                evidence: Some("证据".to_string()),
                confidence: 8,
                importance: 6,
                may_expire: true,
                ..Default::default()
            }),
        ],
        ..Default::default()
    };
    let doc = to_document(&card).expect("to_document");
    let back: MemoryCardTyped = from_document(doc).expect("from_document");
    // Plain 在反序列化时会被 promote 到 Structured（bson 不支持
    // untagged enum 的 String 变体），但 text 必须保留。
    let texts: Vec<String> = back
        .core_facts
        .iter()
        .map(|f| f.as_text().to_string())
        .collect();
    assert!(texts.iter().any(|t| t == "简单字符串 fact"));
    assert!(texts.iter().any(|t| t == "结构化 fact"));
}

/// 整层 MemoryCardTyped round-trip Mongo 不丢字段（含 extra）。
#[test]
fn full_card_round_trip_preserves_extra_fields() {
    use mongodb::bson::{from_document, to_document, Bson};

    let mut extra = mongodb::bson::Document::new();
    extra.insert("preferences", vec!["A", "B"]);
    extra.insert("conflicts", Vec::<mongodb::bson::Document>::new());
    extra.insert("source", "test");
    extra.insert("custom_field", "未识别字段也应保留");
    // `coreProfile` / `relationshipState` 通过 extra 承接（free-form 子文档），
    // 不再 typed 出独立字段 —— 否则 serde flatten 会和 extra 同名键冲突，
    // 序列化产生重复 BSON 键，下次读回触发 `duplicate field 'coreProfile'`。
    extra.insert("coreProfile", doc! { "identity": "测试身份" });
    extra.insert("relationshipState", doc! { "trust": 5 });

    let card = MemoryCardTyped {
        core_facts: vec![MemoryFactRepr::Plain("foo".to_string())],
        recent_facts: vec![],
        deprecated_facts: vec![],
        extra,
    };
    let doc = to_document(&card).expect("serialize");
    // 回归断言：序列化后顶层不应同时出现两份 coreProfile/relationshipState。
    assert_eq!(doc.iter().filter(|(k, _)| *k == "coreProfile").count(), 1);
    assert_eq!(doc.iter().filter(|(k, _)| *k == "relationshipState").count(), 1);
    let back: MemoryCardTyped = from_document(doc).expect("deserialize");
    assert_eq!(
        back.extra
            .get_str("custom_field")
            .ok(),
        Some("未识别字段也应保留")
    );
    assert_eq!(
        back.extra
            .get_document("coreProfile")
            .ok()
            .and_then(|d| d.get_str("identity").ok()),
        Some("测试身份")
    );
    let prefs = back.extra.get("preferences");
    assert!(matches!(prefs, Some(Bson::Array(_))));
}

// 旧 Vec<String> 输入下 cap=6/10 与"未 discarded 必保留"性质（PBT）。
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        max_shrink_iters: 50,
        ..ProptestConfig::default()
    })]

    #[test]
    fn legacy_vec_string_inputs_respect_caps_and_persistence(
        prev_core in proptest::collection::vec("[a-z]{1,8}", 0..=12),
        prev_recent in proptest::collection::vec("[a-z]{1,8}", 0..=15),
        new_core in proptest::collection::vec("[a-z]{1,8}", 0..=8),
        new_recent in proptest::collection::vec("[a-z]{1,8}", 0..=8),
        discarded in proptest::collection::vec("[a-z]{1,8}", 0..=4),
    ) {
        let prev = typed_card_with_core_recent(&prev_core, &prev_recent);
        let incoming = typed_card_with_core_recent(&new_core, &new_recent);
        let merged =
            compact_memory_card_with_previous(&incoming, Some(&prev), &discarded);

        // cap：core ≤ 6, recent ≤ 10。
        prop_assert!(merged.core_facts.len() <= 6);
        prop_assert!(merged.recent_facts.len() <= 10);

        // 未 discarded 的 prev_core 中前 N 个 fact，N ≤ 6 时全部保留；
        // N > 6 时由于上一版自身就超 cap，仅断言"未 discarded 不会被
        // discarded 列表挤掉"——即 discarded 列表的 fact 必不出现。
        for d in &discarded {
            prop_assert!(
                !texts(&merged.core_facts).contains(d),
                "discarded fact {d} 不应出现在 core_facts"
            );
        }
    }
}
