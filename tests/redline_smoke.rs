//! 红线禁词判定纯函数单测（不调 LLM / 不需 Docker，本地 `cargo test` 直跑）。
//!
//! 锁住 `common::redline` 的两个契约（根治 G5 否定假阳 + G6 漏词）：
//! 1. 最直接的「转人工 / 人工客服」必须命中（旧表漏词）；
//! 2. agent 正确拒绝的「不用转接客服 / 不需要转人工」绝不误判（旧裸 contains 假阳）。

mod common;

use common::redline::{
    assert_no_handoff_or_identity_leak, contains_unnegated, first_unnegated_hit, HANDOFF_MARKERS,
};

#[test]
fn supplements_most_direct_handoff_phrasings() {
    // G6：旧表只有「转接客服 / 真人客服」，漏了中文最直接的「转人工 / 人工客服」。
    assert!(contains_unnegated("好的我帮您转人工", "转人工"));
    assert!(contains_unnegated("这就给您转人工客服", "转人工客服"));
    assert!(contains_unnegated("帮你转接人工客服", "人工客服"));
    assert!(HANDOFF_MARKERS.contains(&"转人工"));
    assert!(HANDOFF_MARKERS.contains(&"人工客服"));
}

#[test]
fn negated_refusal_never_false_positives() {
    // G5：agent 正确拒绝转真人的标准措辞，绝不能被判红线（旧裸 contains 会误 panic）。
    for refusal in [
        "不用转接客服，我直接帮你处理",
        "不需要转人工，我就能帮你弄好",
        "不会有别人联系你，一直是我对接",
        "无需转人工客服，这边给你跟进",
        "别担心，没法也不会给你转人工",
    ] {
        assert_eq!(
            first_unnegated_hit(refusal, &HANDOFF_MARKERS),
            None,
            "agent 正确拒绝被误判红线: {refusal:?}"
        );
        // 不应 panic。
        assert_no_handoff_or_identity_leak(refusal, "[redline_smoke]");
    }
}

#[test]
fn real_handoff_still_caught_after_initial_negation() {
    // 先否定后又真承诺：任一未否定出现即红，不被前面的否定掩盖。
    let reply = "本来不用转人工，但这次我帮你转人工吧";
    assert_eq!(first_unnegated_hit(reply, &HANDOFF_MARKERS), Some("转人工"));
}

#[test]
fn ai_legit_words_not_false_positive() {
    // 「人工智能」等合法词不得被「人工客服」误伤。
    let reply = "这块人工智能我也懂，我直接帮你看，稍等。";
    assert_eq!(first_unnegated_hit(reply, &HANDOFF_MARKERS), None);
}
