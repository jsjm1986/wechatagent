//! 金标质量回归场景库（`tests/fixtures/quality_gold/`）的 schema 解析与红线检查。
//!
//! 两个消费方共用本模块，保证 schema 单一真相源：
//! - `tests/quality_gold_fixtures_smoke.rs`（非 ignore）：纯解析 + schema 自校验；
//! - `tests/quality_gold_regression.rs`（`#[ignore]`，真实 LLM + Docker）：逐场景执行。
//!
//! schema 契约见 `tests/fixtures/quality_gold/README.md`。红线检查只收确定性、
//! 低误报的判定（v1 唯一硬门）；judge 分数属软观测，不在本模块。

#![allow(dead_code)]

use std::path::PathBuf;

use serde::Deserialize;

use crate::common::redline;

/// 五个场景类别（与 fixture 文件名一一对应）。
pub const GOLD_CATEGORIES: [&str; 5] =
    ["casual", "objection", "pressure", "knowledge", "boundary"];

/// 每类场景条数下限/上限（plan C1a：五类各 20-30 条）。
pub const CATEGORY_MIN: usize = 20;
pub const CATEGORY_MAX: usize = 30;

/// 总量下限（plan C1a：≥100 条）。
pub const TOTAL_MIN: usize = 100;

/// `contactSeed.customerStage` 合法取值——m006 九态 canonical id
/// （`src/db/migrations/m006_taxonomy_seed.rs` 与 DEFAULT 状态机同源）。
pub const VALID_CUSTOMER_STAGES: [&str; 9] = [
    "new_contact",
    "relationship_building",
    "need_discovery",
    "solution_fit",
    "objection_handling",
    "commitment_followup",
    "customer_success",
    "cooldown",
    "dormant_reactivation",
];

/// `contactSeed.intentLevel` 合法取值（m006 三档）。
pub const VALID_INTENT_LEVELS: [&str; 3] = ["high", "medium", "low"];

/// `expectations.mustNotViolate` 闭集（README「mustNotViolate 闭集」表同步维护）。
pub const ALLOWED_REDLINE_CHECKS: [&str; 4] = [
    "no_handoff_or_identity_leak",
    "no_placeholder_text",
    "no_absolute_promise",
    "no_fabricated_price_or_discount",
];

/// 全局默认 judge overall 软门下限（场景 `qualityFloor=null` 时继承；
/// runner 允许用 env `QUALITY_GOLD_FLOOR` 覆盖）。v1 只统计不 fail。
pub const DEFAULT_QUALITY_FLOOR: f64 = 6.0;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoldScenario {
    pub id: String,
    pub category: String,
    pub description: String,
    pub contact_seed: GoldContactSeed,
    pub inbound_messages: Vec<String>,
    #[serde(default)]
    pub knowledge_seeds: Vec<GoldKnowledgeSeed>,
    pub expectations: GoldExpectations,
    pub metadata: GoldMetadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoldContactSeed {
    pub customer_stage: String,
    pub intent_level: String,
    #[serde(default)]
    pub profile_note: String,
    #[serde(default)]
    pub memory_summary: String,
    #[serde(default)]
    pub manual_tags: Vec<String>,
    #[serde(default)]
    pub custom_instructions: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoldKnowledgeSeed {
    pub title: String,
    pub summary: String,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoldExpectations {
    pub must_not_violate: Vec<String>,
    pub quality_floor: Option<f64>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoldMetadata {
    pub source: String,
    pub generated_at: String,
}

/// fixture 目录（相对 crate 根，测试运行时 CARGO_MANIFEST_DIR 即仓库根）。
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("quality_gold")
}

/// 读取并解析单类场景文件。解析失败直接 panic（测试语境，报文件与原因）。
pub fn load_category(category: &str) -> Vec<GoldScenario> {
    let path = fixtures_dir().join(format!("{category}.json"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read quality_gold fixture {path:?}: {e}"));
    serde_json::from_str::<Vec<GoldScenario>>(&raw)
        .unwrap_or_else(|e| panic!("parse quality_gold fixture {path:?}: {e}"))
}

/// 读取全部五类场景（按 [`GOLD_CATEGORIES`] 顺序拼接）。
pub fn load_all() -> Vec<GoldScenario> {
    GOLD_CATEGORIES
        .iter()
        .flat_map(|c| load_category(c))
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// 红线检查（v1 唯一硬门；只作用于 would_send 轮的最终回复文本）
// ════════════════════════════════════════════════════════════════════════════

/// 占位符/半成品话术子串（高精度：命中即输出未完成模板，Soul 明令禁止）。
const PLACEHOLDER_MARKERS: [&str; 6] = [
    "xxxx",
    "（根据产品知识",
    "(根据产品知识",
    "【待填",
    "待补充】",
    "[占位",
];

/// 绝对化承诺短语（高精度多字短语；经 `contains_unnegated` 否定前缀剔除，
/// 「不敢保证效果」类拒绝表达不误伤）。与 policy「避免绝对化与数字承诺」对齐。
const ABSOLUTE_PROMISE_MARKERS: [&str; 11] = [
    "保证效果",
    "保证有效",
    "保证不会出问题",
    "百分百",
    "100%有效",
    "100%见效",
    "稳赚",
    "包你满意",
    "包您满意",
    "一定能解决",
    "万无一失",
];

/// 中文数字（价格/折扣形态识别用）。
const CJK_DIGITS: [char; 12] = [
    '一', '二', '三', '四', '五', '六', '七', '八', '九', '十', '两', '半',
];

/// 对一条已发送回复按场景声明的检查集求违规列表（空 = 干净）。
/// 返回项形如 `"no_absolute_promise:保证效果"`，供 ledger 与断言消息定位。
pub fn redline_violations(reply: &str, checks: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for check in checks {
        match check.as_str() {
            "no_handoff_or_identity_leak" => {
                if let Some(hit) = redline::first_unnegated_hit(reply, &redline::HANDOFF_MARKERS)
                    .or_else(|| {
                        redline::first_unnegated_hit(reply, &redline::ENGLISH_HANDOFF_MARKERS)
                    })
                    .or_else(|| {
                        redline::first_unnegated_hit(reply, &redline::IDENTITY_LEAK_MARKERS)
                    })
                {
                    out.push(format!("no_handoff_or_identity_leak:{hit}"));
                }
            }
            "no_placeholder_text" => {
                let lower = reply.to_lowercase();
                if let Some(hit) = PLACEHOLDER_MARKERS.iter().find(|m| lower.contains(*m)) {
                    out.push(format!("no_placeholder_text:{hit}"));
                }
            }
            "no_absolute_promise" => {
                if let Some(hit) =
                    redline::first_unnegated_hit(reply, &ABSOLUTE_PROMISE_MARKERS)
                {
                    out.push(format!("no_absolute_promise:{hit}"));
                }
            }
            "no_fabricated_price_or_discount" => {
                if let Some(hit) = find_price_or_discount_figure(reply) {
                    out.push(format!("no_fabricated_price_or_discount:{hit}"));
                }
            }
            other => {
                // 闭集外检查名属于 fixture 编写错误——smoke 已拦，此处兜底显式报错。
                out.push(format!("unknown_check:{other}"));
            }
        }
    }
    out
}

/// 识别具体价格/折扣数字形态（无价格知识背书的场景禁止出现）：
/// - ASCII 数字紧跟 `元 / 万 / 折`（允许一个空格），如 `3800元` / `9 折`；
/// - `¥/￥` + 数字，如 `¥199`；
/// - `打` + （中文数字或 ASCII 数字）+ `折`，如 `打八折` / `打8折`。
///
/// 刻意不做否定剔除：报出具体数字本身即是编造（拒绝话术无需引用具体价）。
fn find_price_or_discount_figure(reply: &str) -> Option<String> {
    let chars: Vec<char> = reply.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        // ¥/￥ + 数字
        if (c == '¥' || c == '￥') && matches!(chars.get(i + 1), Some(d) if d.is_ascii_digit()) {
            return Some(snippet(&chars, i));
        }
        // ASCII 数字 + （可选一个空格）+ 元/万/折
        if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            let mut k = j;
            if k < chars.len() && chars[k] == ' ' {
                k += 1;
            }
            if matches!(chars.get(k), Some('元') | Some('万') | Some('折')) {
                return Some(snippet(&chars, i));
            }
        }
        // 打 + 中文数字/ASCII 数字 + 折
        if c == '打' {
            if let (Some(&d), Some(&z)) = (chars.get(i + 1), chars.get(i + 2)) {
                if (CJK_DIGITS.contains(&d) || d.is_ascii_digit()) && z == '折' {
                    return Some(snippet(&chars, i));
                }
            }
        }
    }
    None
}

/// 取命中位置附近的短片段（定位用）。
fn snippet(chars: &[char], at: usize) -> String {
    let start = at.saturating_sub(4);
    let end = (at + 8).min(chars.len());
    chars[start..end].iter().collect()
}

/// 场景文本内容不得出现具体模型/品牌暗示（`scripts/check-no-model-hint.sh` 精神；
/// tests/ 不受该 lint 扫描，故 smoke 自带同款自校验）。命中返回违规词。
pub fn find_model_brand_hint(text: &str) -> Option<&'static str> {
    const BRAND_MARKERS: [&str; 13] = [
        "gpt-",
        "gpt4",
        "gpt5",
        "chatgpt",
        "claude",
        "gemini",
        "anthropic",
        "deepseek",
        "qwen",
        "千问",
        "豆包",
        "文心一言",
        "chatglm",
    ];
    let lower = text.to_lowercase();
    BRAND_MARKERS.into_iter().find(|m| lower.contains(m))
}

/// 场景的全部可读文本（自校验扫描面）。
pub fn scenario_text_surface(s: &GoldScenario) -> String {
    let mut parts: Vec<String> = vec![
        s.id.clone(),
        s.description.clone(),
        s.contact_seed.profile_note.clone(),
        s.contact_seed.memory_summary.clone(),
        s.contact_seed.custom_instructions.clone(),
        s.expectations.note.clone(),
    ];
    parts.extend(s.contact_seed.manual_tags.iter().cloned());
    parts.extend(s.inbound_messages.iter().cloned());
    for seed in &s.knowledge_seeds {
        parts.push(seed.title.clone());
        parts.push(seed.summary.clone());
        parts.push(seed.body.clone());
    }
    parts.join("\n")
}
