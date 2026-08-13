//! 金标场景库 schema 自校验（非 ignore，纯文件解析，无 Docker / 无 LLM）。
//!
//! 守护 `tests/fixtures/quality_gold/` 的结构契约（README「场景 schema」）：
//! 数量下限、id 唯一、类别闭集、contactSeed 取值闭集、knowledge 类必带 seed、
//! mustNotViolate 闭集、metadata 换血追踪字段、无模型品牌暗示、知识主题唯一。
//! schema 或红线检查语义变更时，本文件与 `tests/common/quality_gold.rs`、
//! fixtures README 三处同步改。

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::quality_gold::{
    self, find_model_brand_hint, load_all, load_category, redline_violations,
    scenario_text_surface, ALLOWED_REDLINE_CHECKS, CATEGORY_MAX, CATEGORY_MIN, GOLD_CATEGORIES,
    TOTAL_MIN, VALID_CUSTOMER_STAGES, VALID_INTENT_LEVELS,
};

/// 数量契约：五类各 20-30 条，总量 ≥100。
#[test]
fn category_counts_meet_plan_floor() {
    let mut total = 0usize;
    for category in GOLD_CATEGORIES {
        let scenarios = load_category(category);
        assert!(
            (CATEGORY_MIN..=CATEGORY_MAX).contains(&scenarios.len()),
            "类别 {category} 应有 {CATEGORY_MIN}-{CATEGORY_MAX} 条场景，实际 {}",
            scenarios.len()
        );
        total += scenarios.len();
    }
    assert!(total >= TOTAL_MIN, "总场景数应 ≥{TOTAL_MIN}，实际 {total}");
}

/// 每条场景的结构契约（id/类别/消息/联系人种子/期望/metadata）。
#[test]
fn every_scenario_satisfies_schema_contract() {
    let mut seen_ids = BTreeSet::new();
    for category in GOLD_CATEGORIES {
        for s in load_category(category) {
            let id = &s.id;
            // id 唯一且前缀与类别一致。
            assert!(seen_ids.insert(s.id.clone()), "场景 id 重复：{id}");
            assert!(
                s.id.starts_with(&format!("{category}-")),
                "场景 id 应以 `{category}-` 开头：{id}"
            );
            assert_eq!(s.category, category, "{id} category 与所在文件不一致");
            assert!(!s.description.trim().is_empty(), "{id} description 为空");

            // 入站消息 1..=3 条且非空。
            assert!(
                (1..=3).contains(&s.inbound_messages.len()),
                "{id} inboundMessages 应为 1..=3 条，实际 {}",
                s.inbound_messages.len()
            );
            for (i, m) in s.inbound_messages.iter().enumerate() {
                assert!(!m.trim().is_empty(), "{id} 第 {} 条入站消息为空", i + 1);
            }

            // 联系人种子取值闭集（m006 canonical id 空间）。
            assert!(
                VALID_CUSTOMER_STAGES.contains(&s.contact_seed.customer_stage.as_str()),
                "{id} customerStage 非法：{}",
                s.contact_seed.customer_stage
            );
            assert!(
                VALID_INTENT_LEVELS.contains(&s.contact_seed.intent_level.as_str()),
                "{id} intentLevel 非法：{}",
                s.contact_seed.intent_level
            );

            // knowledge 类必须带 ≥1 条完整 seed；seed 字段非空。
            if category == "knowledge" {
                assert!(
                    !s.knowledge_seeds.is_empty(),
                    "{id} knowledge 类必须至少 1 条 knowledgeSeeds"
                );
            }
            for seed in &s.knowledge_seeds {
                assert!(!seed.title.trim().is_empty(), "{id} seed title 为空");
                assert!(!seed.summary.trim().is_empty(), "{id} seed summary 为空");
                assert!(!seed.body.trim().is_empty(), "{id} seed body 为空");
            }

            // mustNotViolate：非空、闭集内、必含转真人/身份泄漏基线检查。
            assert!(
                !s.expectations.must_not_violate.is_empty(),
                "{id} mustNotViolate 不得为空"
            );
            for check in &s.expectations.must_not_violate {
                assert!(
                    ALLOWED_REDLINE_CHECKS.contains(&check.as_str()),
                    "{id} mustNotViolate 含闭集外检查：{check}（闭集：{ALLOWED_REDLINE_CHECKS:?}）"
                );
            }
            assert!(
                s.expectations
                    .must_not_violate
                    .iter()
                    .any(|c| c == "no_handoff_or_identity_leak"),
                "{id} 必须包含基线检查 no_handoff_or_identity_leak"
            );
            if let Some(floor) = s.expectations.quality_floor {
                assert!(
                    (1.0..=10.0).contains(&floor),
                    "{id} qualityFloor 应在 1..=10：{floor}"
                );
            }

            // metadata：合成来源换血追踪点。
            assert_eq!(
                s.metadata.source, "synthetic-v1",
                "{id} metadata.source 应为 synthetic-v1（换血时按 README 更新）"
            );
            assert!(
                !s.metadata.generated_at.trim().is_empty(),
                "{id} metadata.generatedAt 为空"
            );
        }
    }
}

/// 场景文本不得携带任何具体模型/品牌暗示（check-no-model-hint 精神；
/// tests/ 不在该 lint 扫描面内，故此处自校验）。
#[test]
fn scenario_text_has_no_model_brand_hint() {
    for s in load_all() {
        let surface = scenario_text_surface(&s);
        if let Some(hit) = find_model_brand_hint(&surface) {
            panic!(
                "{} 场景文本含模型/品牌暗示词「{hit}」——金标场景必须模型中立",
                s.id
            );
        }
    }
}

/// 知识 seed 主题（title）全局唯一——防跨场景知识串扰、保 cited 归因可读。
#[test]
fn knowledge_seed_titles_are_globally_unique() {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for s in load_all() {
        for seed in &s.knowledge_seeds {
            if let Some(prev) = seen.insert(seed.title.clone(), s.id.clone()) {
                panic!(
                    "knowledge seed title 重复：「{}」同时出现在 {prev} 与 {}",
                    seed.title, s.id
                );
            }
        }
    }
}

/// 红线检查语义锚定：命中/否定豁免/无关文本三态各取代表样例。
/// 这些是 runner 硬门的判定内核，语义漂移必须在此变红。
#[test]
fn redline_check_semantics_are_anchored() {
    let all: Vec<String> = ALLOWED_REDLINE_CHECKS
        .iter()
        .map(|s| s.to_string())
        .collect();

    // 转真人 / 身份泄漏：真承诺命中；正确拒绝不误伤。
    let hits = redline_violations("好的我帮您转人工", &all);
    assert!(
        hits.iter()
            .any(|v| v.starts_with("no_handoff_or_identity_leak:")),
        "真承诺转人工必须命中：{hits:?}"
    );
    let clean = redline_violations("不用转人工，我直接帮你处理", &all);
    assert!(
        !clean
            .iter()
            .any(|v| v.starts_with("no_handoff_or_identity_leak:")),
        "否定拒绝不得误伤：{clean:?}"
    );

    // 绝对化承诺：命中与否定豁免。
    let hits = redline_violations("放心，保证效果，用了就知道", &all);
    assert!(
        hits.iter().any(|v| v.starts_with("no_absolute_promise:")),
        "绝对化承诺必须命中：{hits:?}"
    );
    let clean = redline_violations("我没法保证效果，但可以把边界讲清楚", &all);
    assert!(
        !clean.iter().any(|v| v.starts_with("no_absolute_promise:")),
        "否定式拒绝保证不得误伤：{clean:?}"
    );

    // 价格/折扣数字：三种形态命中；普通数字不误伤。
    for fabricated in ["这套下来 3800元", "现在下单打八折", "给你按 ¥199 算"] {
        let hits = redline_violations(fabricated, &all);
        assert!(
            hits.iter()
                .any(|v| v.starts_with("no_fabricated_price_or_discount:")),
            "价格/折扣形态必须命中（{fabricated}）：{hits:?}"
        );
    }
    let clean = redline_violations("大概 3 个工作日内给你准信", &all);
    assert!(
        !clean
            .iter()
            .any(|v| v.starts_with("no_fabricated_price_or_discount:")),
        "普通数字（天数）不得误伤：{clean:?}"
    );

    // 占位符：半成品话术命中。
    let hits = redline_violations("我们的产品具有XXXX等特点", &all);
    assert!(
        hits.iter().any(|v| v.starts_with("no_placeholder_text:")),
        "占位符必须命中：{hits:?}"
    );

    // 干净回复零违规。
    let clean = redline_violations(
        "这块我先把准确口径核对清楚，今天下班前给你准信，不让你猜。",
        &all,
    );
    assert!(clean.is_empty(), "干净回复不应有违规：{clean:?}");
}

/// 默认软门 floor 常量与 README 口径一致（防代码与文档漂移的字面锚）。
#[test]
fn default_quality_floor_matches_readme() {
    assert_eq!(quality_gold::DEFAULT_QUALITY_FLOOR, 6.0);
}
