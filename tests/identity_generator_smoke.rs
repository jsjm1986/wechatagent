//! R2.1 身份生成器纯函数单测（不调 LLM，本地 `cargo test` 直跑）。
//!
//! 验证生成器**可复现性的根**——离线候选库 + 确定性选择——不依赖任何外部模型：
//! 1. 候选库覆盖 ≥4 大类；
//! 2. `select_skeleton(seed)` 确定性（同 seed 同骨架，遍历所有 seed 不 panic）；
//! 3. funnel 极性按大类正确（衔接 R1.1 judge 极性翻转）；
//! 4. `apply_category_semantics` 把 default profile 的销售骨架按大类改写自洽。
//!
//! LLM 那一段（`generate_identity`）在 real-LLM 套件里跑，本文件刻意只测纯函数。

mod common;

use common::identity_generator::{
    apply_category_semantics, industry_candidates, select_skeleton, IdentityCategory,
};
use std::collections::HashSet;
use wechatagent::agent::default_domain_profile;

#[test]
fn candidate_library_covers_at_least_four_categories() {
    let cats: HashSet<&str> = industry_candidates()
        .iter()
        .map(|s| s.category.as_str())
        .collect();
    assert!(cats.len() >= 4, "候选库必须覆盖 ≥4 大类，实际={cats:?}");
    // 四大类必须各至少有一个具体行业。
    for expected in ["sales", "companion", "peer_social", "formal_business"] {
        assert!(
            cats.contains(expected),
            "候选库缺少大类「{expected}」，实际={cats:?}"
        );
    }
}

#[test]
fn select_skeleton_is_deterministic_for_same_seed() {
    // 同 seed → 同行业 + 同 category（可复现的根）。
    for seed in [0usize, 1, 7, 42, 1000, usize::MAX] {
        let a = select_skeleton(seed);
        let b = select_skeleton(seed);
        assert_eq!(a.industry, b.industry, "seed={seed} 选中行业必须确定");
        assert_eq!(a.category, b.category, "seed={seed} 选中大类必须确定");
    }
}

#[test]
fn select_skeleton_wraps_modulo_and_covers_all_candidates() {
    let candidates = industry_candidates();
    let len = candidates.len();
    // seed % len 选取：seed 与 seed+len 必选同一条（取模回绕）。
    for seed in 0..len {
        assert_eq!(
            select_skeleton(seed).industry,
            select_skeleton(seed + len).industry,
            "seed={seed} 与 seed+{len} 应回绕到同一行业"
        );
    }
    // 遍历一整个周期应覆盖到全部候选行业（无遗漏、无越界）。
    let covered: HashSet<&str> = (0..len).map(|s| select_skeleton(s).industry).collect();
    assert_eq!(
        covered.len(),
        len,
        "0..len 应覆盖全部 {len} 个候选行业，实际覆盖 {}",
        covered.len()
    );
}

#[test]
fn funnel_polarity_matches_category() {
    // 销售/正式业务 = 漏斗型（true）；情感陪伴/同行社交 = 关系型（false）。
    assert!(IdentityCategory::Sales.is_funnel());
    assert!(IdentityCategory::FormalBusiness.is_funnel());
    assert!(!IdentityCategory::Companion.is_funnel());
    assert!(!IdentityCategory::PeerSocial.is_funnel());

    // 候选库每条骨架的 funnel 极性与其 category 一致。
    for sk in industry_candidates() {
        assert_eq!(
            sk.is_funnel(),
            sk.category.is_funnel(),
            "行业「{}」的 funnel 极性应随 category 走",
            sk.industry
        );
    }
}

#[test]
fn apply_category_semantics_sets_funnel_and_transaction_flags() {
    // 漏斗型（销售）：funnel 开、交易事实开、grounding 不旁路、信任自报低风险。
    let mut sales = default_domain_profile("ws-sales");
    apply_category_semantics(&mut sales, IdentityCategory::Sales);
    assert!(sales.operation_mode.funnel.enabled, "销售域 funnel 应开");
    assert!(sales.transaction_facts_enabled, "销售域应注入交易事实");
    assert!(
        !sales.grounding_gate_bypass_without_claim,
        "销售域 grounding 硬闸不旁路"
    );
    assert!(
        !sales.distrust_self_reported_low_risk,
        "销售域沿用既有 review 判定"
    );

    // 关系型（情感陪伴）：funnel 关、交易事实关、grounding 旁路、强制 LLM review。
    let mut companion = default_domain_profile("ws-comp");
    apply_category_semantics(&mut companion, IdentityCategory::Companion);
    assert!(
        !companion.operation_mode.funnel.enabled,
        "陪伴域 funnel 应关"
    );
    assert!(!companion.transaction_facts_enabled, "陪伴域不注入交易事实");
    assert!(
        companion.grounding_gate_bypass_without_claim,
        "陪伴域 grounding 软闸旁路（纯情感回复不被误拦）"
    );
    assert!(
        companion.distrust_self_reported_low_risk,
        "高敏域强制走 LLM review"
    );
}

#[test]
fn apply_category_semantics_peer_social_is_relational() {
    // 同行社交也是关系型：与情感陪伴同极性（funnel 关）。
    let mut peer = default_domain_profile("ws-peer");
    apply_category_semantics(&mut peer, IdentityCategory::PeerSocial);
    assert!(!peer.operation_mode.funnel.enabled);
    assert!(!peer.transaction_facts_enabled);

    // 正式业务是漏斗型：与销售同极性（funnel 开）。
    let mut formal = default_domain_profile("ws-formal");
    apply_category_semantics(&mut formal, IdentityCategory::FormalBusiness);
    assert!(formal.operation_mode.funnel.enabled);
    assert!(formal.transaction_facts_enabled);
}
