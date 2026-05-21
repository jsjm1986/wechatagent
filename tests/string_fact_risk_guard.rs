//! MP-6 / Task 12 / Task 24：字符串级 fact-risk 兜底 guard 正反测试。
//!
//! 性质：
//! 1. 命中 marker（如"保证"、"绝对"、"30%"、"¥50"）→ `scan_product_claim_marker_labels`
//!    返回非空（每个 hit 一个 label）。
//! 2. 白名单短语在窗口内出现 → 该 marker 被豁免。
//! 3. 数字百分比/折扣、价格金额这类正则 marker 与字面量 marker 都能命中。
//! 4. 不命中的中性表达 → 返回空 vec。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use proptest::prelude::*;
use wechatagent::agent::scan_product_claim_marker_labels;

#[test]
fn literal_promise_is_hit() {
    let labels = scan_product_claim_marker_labels("我保证你会满意");
    assert!(
        labels.iter().any(|l| l == "literal:保证"),
        "应命中 literal:保证 marker，实际：{:?}",
        labels
    );
}

#[test]
fn absolute_word_is_hit() {
    let labels = scan_product_claim_marker_labels("这个绝对没问题");
    assert!(
        labels.iter().any(|l| l == "literal:绝对"),
        "应命中 literal:绝对 marker，实际：{:?}",
        labels
    );
}

#[test]
fn numeric_percent_is_hit() {
    let labels = scan_product_claim_marker_labels("成本可降低 30%");
    assert!(
        labels.iter().any(|l| l == "regex:数字百分比/折扣"),
        "应命中数字百分比 marker，实际：{:?}",
        labels
    );
}

#[test]
fn discount_pattern_is_hit() {
    let labels = scan_product_claim_marker_labels("现在下单可以打 5折");
    assert!(
        labels.iter().any(|l| l == "regex:数字百分比/折扣"),
        "应命中折扣 marker，实际：{:?}",
        labels
    );
}

#[test]
fn price_yuan_amount_is_hit() {
    let labels = scan_product_claim_marker_labels("套餐 999元 起");
    assert!(
        labels.iter().any(|l| l == "regex:价格金额"),
        "应命中价格金额 marker，实际：{:?}",
        labels
    );
}

#[test]
fn price_yen_amount_is_hit() {
    let labels = scan_product_claim_marker_labels("总计 ¥500");
    assert!(
        labels.iter().any(|l| l == "regex:价格金额"),
        "应命中 ¥ 价格金额 marker，实际：{:?}",
        labels
    );
}

#[test]
fn whitelist_phrase_within_window_exempts_hit() {
    // "保证" 窗口（左右 8 个字符）含白名单短语 "准时" 时应豁免。
    let labels = scan_product_claim_marker_labels("会准时保证按时交付");
    assert!(
        !labels.contains(&"literal:保证".to_string()),
        "白名单短语 准时 应豁免 literal:保证 marker，实际：{:?}",
        labels
    );
}

#[test]
fn whitelist_phrase_far_away_does_not_exempt() {
    // 白名单短语距 marker 超过 8 个字符则不豁免。
    // 注意避开白名单短语 [准时, 按时, 尊重, 保护, 你的]——文本中绝不能含这些。
    let labels = scan_product_claim_marker_labels(
        "现在情况比较复杂，要做的事很多，整体上保证产品质量没问题",
    );
    assert!(
        labels.contains(&"literal:保证".to_string()),
        "白名单短语距离过远应不豁免，labels={:?}",
        labels
    );
}

#[test]
fn neutral_text_has_no_hits() {
    let labels = scan_product_claim_marker_labels("好的，我了解一下情况后给你回复");
    assert!(
        labels.is_empty(),
        "中性回复不应命中任何 marker，实际：{:?}",
        labels
    );
}

#[test]
fn empty_string_has_no_hits() {
    assert!(scan_product_claim_marker_labels("").is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        ..ProptestConfig::default()
    })]

    /// PBT：任意纯英文字母小写文本都不应命中任何中文 marker。
    #[test]
    fn ascii_lowercase_letters_never_hit_chinese_markers(
        text in "[a-z ]{0,40}",
    ) {
        let labels = scan_product_claim_marker_labels(&text);
        // 这些 ascii 字母不会命中任何 literal（保证/绝对/百分之/案例/见效/回款/成功率/一定能）
        // 也不会命中数字百分比 / 价格金额（无数字）。
        prop_assert!(labels.is_empty(),
            "纯小写字母文本意外命中 marker：text={text:?} labels={labels:?}");
    }

    /// PBT：任意 5-15 位数字 + "%" 必然命中数字百分比 marker。
    #[test]
    fn any_numeric_percent_always_hits(
        n in 1u32..=999_999u32,
    ) {
        let text = format!("数据是 {n}% 这样");
        let labels = scan_product_claim_marker_labels(&text);
        prop_assert!(
            labels.iter().any(|l| l == "regex:数字百分比/折扣"),
            "{n}% 应命中，但 labels={labels:?}"
        );
    }
}
