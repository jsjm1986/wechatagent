//! 决策评审守卫（state machine、字符串级 fact-risk、知识支撑校验）。
//!
//! 这里聚合 [`enforce_decision_guards`] 调用链：状态机迁移合法性
//! (MP-7 / Task 13)、产品事实字符串 guard (MP-6 / Task 12)、产品声明
//! 是否被知识切片支撑等。`ProductClaimMarkers` 标记词与白名单从
//! `prompt_templates` 加载，内置默认值只作为缺失时的兜底。
//!
//! 模块对外只暴露 [`enforce_decision_guards`]、纯计算的
//! [`check_state_transition`]、[`normalize_decision_state`] 等给 review /
//! gateway 使用，避免业务调用方误触低级 helper。

use mongodb::bson::Document;

use crate::db::Database;
use crate::models::{OperationDomainConfig, OperationKnowledgeChunk, OperationKnowledgeItem};
use crate::prompts;
use crate::routes::AppState;

use super::types::{doc_bool, AgentDecision, DecisionReviewResult, RunPlannerResult};

/// 同步守卫入口（兼容 + 测试用）。生产路径改走
/// [`enforce_decision_guards_with_markers`]，可注入运行时加载的标记词。
/// 此版本使用内置默认 markers，便于 `mod tests` 直接断言。
#[allow(dead_code)]
pub(crate) fn enforce_decision_guards(
    review: &mut DecisionReviewResult,
    decision: &AgentDecision,
    domain_config: Option<&OperationDomainConfig>,
    operation_knowledge: &[OperationKnowledgeItem],
    knowledge_chunks: &[OperationKnowledgeChunk],
    current_state: Option<&str>,
) {
    enforce_decision_guards_with_markers(
        review,
        decision,
        domain_config,
        operation_knowledge,
        knowledge_chunks,
        current_state,
        &default_product_claim_markers(),
    );
}

/// 波 C4：`ProductClaimMarkers` 进程内 TTL 缓存。
///
/// 每次 `review_decision` 都会用一次，旧实现在每条 review 都做一次 DB 查询
/// + JSON parse；这里用进程级 TTL（30s）摊开开销，前端 publish 后下一次刷
/// 新即可看到效果，对低频运营变更够用。
struct ProductClaimMarkersCacheEntry {
    workspace_id: String,
    fetched_at: std::time::Instant,
    markers: ProductClaimMarkers,
}

static PRODUCT_CLAIM_MARKERS_CACHE: parking_lot::Mutex<Option<ProductClaimMarkersCacheEntry>> =
    parking_lot::Mutex::new(None);

const PRODUCT_CLAIM_MARKERS_TTL: std::time::Duration = std::time::Duration::from_secs(30);

pub(crate) async fn load_product_claim_markers(state: &AppState) -> ProductClaimMarkers {
    // 命中缓存（30s TTL，相同 workspace）直接返回。
    {
        let cache = PRODUCT_CLAIM_MARKERS_CACHE.lock();
        if let Some(entry) = cache.as_ref() {
            if entry.workspace_id == state.config.default_workspace_id
                && entry.fetched_at.elapsed() < PRODUCT_CLAIM_MARKERS_TTL
            {
                return entry.markers.clone();
            }
        }
    }
    let markers = match prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.review.product_claim_markers",
    )
    .await
    {
        Ok(content) => parse_product_claim_markers(&content).unwrap_or_else(default_product_claim_markers),
        Err(_) => default_product_claim_markers(),
    };
    *PRODUCT_CLAIM_MARKERS_CACHE.lock() = Some(ProductClaimMarkersCacheEntry {
        workspace_id: state.config.default_workspace_id.clone(),
        fetched_at: std::time::Instant::now(),
        markers: markers.clone(),
    });
    markers
}

/// 波 C4：在前端 publish prompt template 后调用以让 marker 缓存立即失效。
/// 暴露给 `routes::prompt_templates::publish_prompt_template` 等场景。
#[allow(dead_code)]
pub(crate) fn invalidate_product_claim_markers_cache() {
    *PRODUCT_CLAIM_MARKERS_CACHE.lock() = None;
}

pub(crate) fn enforce_decision_guards_with_markers(
    review: &mut DecisionReviewResult,
    decision: &AgentDecision,
    domain_config: Option<&OperationDomainConfig>,
    _operation_knowledge: &[OperationKnowledgeItem],
    knowledge_chunks: &[OperationKnowledgeChunk],
    current_state: Option<&str>,
    markers: &ProductClaimMarkers,
) {
    if let Some(state_key) = decision
        .operation_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !operation_state_exists(domain_config, state_key) {
            review.approved = false;
            review.scores.fact_risk = review.scores.fact_risk.max(6);
            review
                .risks
                .push(format!("operationState 不在状态机中: {state_key}"));
        } else if let Some(reason) = check_state_transition(domain_config, current_state, state_key)
        {
            review.approved = false;
            review.scores.fact_risk = review.scores.fact_risk.max(6);
            review.risks.push(reason);
        }
    }
    if decision.should_reply
        && claim_requires_product_knowledge(&review.claim_analysis)
        && (knowledge_chunks.is_empty()
            || (decision.used_knowledge_ids.is_empty() && decision.safe_claims_used.is_empty())
            || !claim_is_knowledge_supported(&review.claim_analysis))
    {
        review.approved = false;
        review.scores.fact_risk = review.scores.fact_risk.max(6);
        review.scores.product_accuracy = review.scores.product_accuracy.min(6);
        review.risks.push(
            "Review Agent 判断候选回复涉及需要产品知识支撑的表述，但知识依据不足".to_string(),
        );
    }

    // MP-6 / Task 12：字符串级 fact-risk 兜底。模型自我声明被绕过时仍能拦下。
    enforce_string_fact_risk_guard(review, decision, markers);
}

/// 单条字符串 marker：可以是 literal 子串，也可以是简单数字模式（避免引入完整 regex 依赖）。
#[derive(Debug, Clone)]
pub(crate) struct ProductClaimMarker {
    /// 用于命中判定的 matcher。
    matcher: ClaimMarkerKind,
    /// 给 review.risks 看的可读理由。
    reason: String,
    /// 可读的 marker 名（`literal:保证` 等），用于日志和断言。
    label: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ClaimMarkerKind {
    /// 命中字符串字面量。
    Literal(String),
    /// 命中"数字 + 百分号/折"，例如 `30%`、`30 %`、`5折`。
    NumericPercentOrDiscount,
    /// 命中价格金额：`¥/￥/RMB/rmb` 后跟数字，或数字后跟 `元/万/亿`。
    PriceAmount,
}

/// 内置标记词与白名单。Task 16 之后可改为从 prompt_templates 动态加载。
///
/// agent-autonomy-loop W3 / Task 4.14：本函数原本 `pub(crate)`；为了让
/// `tests/autonomy_protocol_pbt.rs` 中的 P4 性质测试能在独立 crate 中构造
/// `finalize_review_for_send` 需要的 `&ProductClaimMarkers` 入参，提升为 `pub`。
/// 语义不变，仅可见性变化。
pub fn default_product_claim_markers() -> ProductClaimMarkers {
    ProductClaimMarkers {
        markers: vec![
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("保证".to_string()),
                reason: "绝对化承诺".to_string(),
                label: "literal:保证".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("一定能".to_string()),
                reason: "绝对化承诺".to_string(),
                label: "literal:一定能".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("绝对".to_string()),
                reason: "绝对化承诺".to_string(),
                label: "literal:绝对".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("百分之".to_string()),
                reason: "数字百分比承诺".to_string(),
                label: "literal:百分之".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::NumericPercentOrDiscount,
                reason: "数字百分比/折扣".to_string(),
                label: "regex:数字百分比/折扣".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::PriceAmount,
                reason: "价格金额".to_string(),
                label: "regex:价格金额".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("案例".to_string()),
                reason: "可能引用未支撑案例".to_string(),
                label: "literal:案例".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("成功率".to_string()),
                reason: "效果数据承诺".to_string(),
                label: "literal:成功率".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("见效".to_string()),
                reason: "效果承诺".to_string(),
                label: "literal:见效".to_string(),
            },
            ProductClaimMarker {
                matcher: ClaimMarkerKind::Literal("回款".to_string()),
                reason: "效果承诺".to_string(),
                label: "literal:回款".to_string(),
            },
        ],
        whitelist_phrases: vec![
            "准时".to_string(),
            "按时".to_string(),
            "尊重".to_string(),
            "保护".to_string(),
            "你的".to_string(),
        ],
        whitelist_window_chars: 8,
    }
}

#[derive(Debug, Clone)]
pub struct ProductClaimMarkers {
    markers: Vec<ProductClaimMarker>,
    /// 命中点周围的窗口内若出现这些短语，则视为合理表达，豁免本次命中。
    whitelist_phrases: Vec<String>,
    /// 白名单短语在 marker 命中点左侧检查的最大字符数（按 char 计）。
    whitelist_window_chars: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct MarkerHit {
    label: String,
    reason: String,
    matched_text: String,
    /// 命中起始字符索引（按 char 计）。
    char_index: usize,
}

impl ProductClaimMarkers {
    pub(crate) fn scan(&self, reply_text: &str) -> Vec<MarkerHit> {
        let mut hits = Vec::new();
        for marker in &self.markers {
            match &marker.matcher {
                ClaimMarkerKind::Literal(lit) => {
                    if let Some(byte_idx) = reply_text.find(lit) {
                        let char_idx = reply_text[..byte_idx].chars().count();
                        hits.push(MarkerHit {
                            label: marker.label.clone(),
                            reason: marker.reason.clone(),
                            matched_text: lit.clone(),
                            char_index: char_idx,
                        });
                    }
                }
                ClaimMarkerKind::NumericPercentOrDiscount => {
                    if let Some(hit) = scan_numeric_percent_or_discount(reply_text) {
                        hits.push(MarkerHit {
                            label: marker.label.clone(),
                            reason: marker.reason.clone(),
                            matched_text: hit.0,
                            char_index: hit.1,
                        });
                    }
                }
                ClaimMarkerKind::PriceAmount => {
                    if let Some(hit) = scan_price_amount(reply_text) {
                        hits.push(MarkerHit {
                            label: marker.label.clone(),
                            reason: marker.reason.clone(),
                            matched_text: hit.0,
                            char_index: hit.1,
                        });
                    }
                }
            }
        }
        hits
    }

    pub(crate) fn passes_whitelist(&self, text: &str, hit: &MarkerHit) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let start = hit.char_index.saturating_sub(self.whitelist_window_chars);
        let end = (hit.char_index + hit.matched_text.chars().count() + self.whitelist_window_chars)
            .min(chars.len());
        let window: String = chars[start..end].iter().collect();
        self.whitelist_phrases
            .iter()
            .any(|phrase| window.contains(phrase))
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductClaimMarkersConfig {
    #[serde(default)]
    markers: Vec<ProductClaimMarkerConfig>,
    #[serde(default)]
    whitelist_phrases: Vec<String>,
    #[serde(default)]
    whitelist_window_chars: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProductClaimMarkerConfig {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    matcher: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    label: String,
}

fn parse_product_claim_markers(content: &str) -> Option<ProductClaimMarkers> {
    let config: ProductClaimMarkersConfig = serde_json::from_str(content).ok()?;
    let mut markers = Vec::new();
    for item in config.markers {
        let kind = item.kind.trim();
        let matcher = item.matcher.trim();
        let marker_kind = match kind {
            "numeric_percent_or_discount" | "numericPercentOrDiscount" => {
                ClaimMarkerKind::NumericPercentOrDiscount
            }
            "price_amount" | "priceAmount" => ClaimMarkerKind::PriceAmount,
            _ => {
                if matcher.is_empty() {
                    continue;
                }
                ClaimMarkerKind::Literal(matcher.to_string())
            }
        };
        let label = if item.label.trim().is_empty() {
            match &marker_kind {
                ClaimMarkerKind::Literal(value) => format!("literal:{value}"),
                ClaimMarkerKind::NumericPercentOrDiscount => "regex:数字百分比/折扣".to_string(),
                ClaimMarkerKind::PriceAmount => "regex:价格金额".to_string(),
            }
        } else {
            item.label
        };
        markers.push(ProductClaimMarker {
            matcher: marker_kind,
            reason: if item.reason.trim().is_empty() {
                "产品事实风险".to_string()
            } else {
                item.reason
            },
            label,
        });
    }
    if markers.is_empty() {
        return None;
    }
    Some(ProductClaimMarkers {
        markers,
        whitelist_phrases: config
            .whitelist_phrases
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        whitelist_window_chars: config.whitelist_window_chars.unwrap_or(8).clamp(0, 64),
    })
}

/// 扫描第一个 `\d+\s*(%|％|折)` 形态。
fn scan_numeric_percent_or_discount(text: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            // 跳过可选空白。
            let mut j = i;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == '%' || chars[j] == '％' || chars[j] == '折') {
                let matched: String = chars[start..=j].iter().collect();
                return Some((matched, start));
            }
        } else {
            i += 1;
        }
    }
    None
}

/// 扫描价格金额：`¥/￥/RMB/rmb 数字` 或 `数字 元|万|亿`。
fn scan_price_amount(text: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    // ¥/￥ 直接前缀
    for (idx, ch) in chars.iter().enumerate() {
        if *ch == '¥' || *ch == '￥' {
            // 看后面是否有数字
            let mut j = idx + 1;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && chars[j].is_ascii_digit() {
                let mut end = j;
                while end < len && chars[end].is_ascii_digit() {
                    end += 1;
                }
                let matched: String = chars[idx..end].iter().collect();
                return Some((matched, idx));
            }
        }
    }
    // 数字后跟 `元/万/亿`
    let mut i = 0;
    while i < len {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut j = i;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == '元' || chars[j] == '万' || chars[j] == '亿') {
                let matched: String = chars[start..=j].iter().collect();
                return Some((matched, start));
            }
        } else {
            i += 1;
        }
    }
    // RMB / rmb 前缀
    let lower = text.to_ascii_lowercase();
    if let Some(byte_idx) = lower.find("rmb") {
        let char_idx = text[..byte_idx].chars().count();
        let after = byte_idx + "rmb".len();
        let trimmed = text[after..].trim_start();
        if trimmed
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            let mut digits_end = 0;
            for (idx, c) in trimmed.char_indices() {
                if c.is_ascii_digit() {
                    digits_end = idx + c.len_utf8();
                } else {
                    break;
                }
            }
            let matched: String = format!("RMB{}", &trimmed[..digits_end]);
            return Some((matched, char_idx));
        }
    }
    None
}

/// MP-6 / Task 12：字符串 fact-risk 兜底测试用导出 API。
///
/// 给定 reply_text，返回触发的 marker label 列表（已应用白名单豁免）。
/// 用于在外部 crate 验证 PBT 性质。
pub fn scan_product_claim_marker_labels(reply_text: &str) -> Vec<String> {
    let markers = default_product_claim_markers();
    markers
        .scan(reply_text)
        .into_iter()
        .filter(|hit| !markers.passes_whitelist(reply_text, hit))
        .map(|hit| hit.label)
        .collect()
}

/// MP-6 字符串级兜底：`reply_text` 命中标记词且本次没有 used_knowledge_ids
/// 也没有 safe_claims_used，则直接拉高 fact_risk 并拒绝。
pub(crate) fn enforce_string_fact_risk_guard(
    review: &mut DecisionReviewResult,
    decision: &AgentDecision,
    markers: &ProductClaimMarkers,
) {
    if !decision.should_reply {
        return;
    }
    // 模型已声明无需产品知识，且声明知识被支撑，则跳过字符串扫描。
    let safe_by_model = !claim_requires_product_knowledge(&review.claim_analysis)
        && claim_is_knowledge_supported(&review.claim_analysis);
    if safe_by_model {
        return;
    }
    if !decision.used_knowledge_ids.is_empty() || !decision.safe_claims_used.is_empty() {
        return;
    }
    let hits = markers.scan(&decision.reply_text);
    let real_hits: Vec<MarkerHit> = hits
        .into_iter()
        .filter(|hit| !markers.passes_whitelist(&decision.reply_text, hit))
        .collect();
    if real_hits.is_empty() {
        return;
    }
    review.approved = false;
    review.scores.fact_risk = review.scores.fact_risk.max(6);
    review.scores.product_accuracy = review.scores.product_accuracy.min(6);
    for hit in real_hits {
        review.risks.push(format!(
            "string_guard: 命中标记 [{}]（{}），但本次未引用知识切片或安全声明",
            hit.label, hit.reason
        ));
    }
}

pub(crate) fn claim_requires_product_knowledge(claim_analysis: &Document) -> bool {
    doc_bool(claim_analysis, "requiresProductKnowledge")
        || doc_bool(claim_analysis, "requires_product_knowledge")
}

pub(crate) fn claim_is_knowledge_supported(claim_analysis: &Document) -> bool {
    doc_bool(claim_analysis, "knowledgeSupported")
        || doc_bool(claim_analysis, "knowledge_supported")
}

pub(crate) fn normalize_decision_state(
    decision: &mut AgentDecision,
    domain_config: Option<&OperationDomainConfig>,
) {
    let Some(current) = decision
        .operation_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    if operation_state_exists(domain_config, current) {
        return;
    }
    if let Some(key) = operation_state_key_by_name(domain_config, current) {
        decision.operation_state = Some(key);
    }
}

// W1 / R3.6 / N1: this fn no longer fills defaults; missing fields are caught by
// validate_and_promote (task 2.3) and finalize_review_for_send (W2).
//
// Removed branches (silently filled defaults for fields that must come from the
// agent itself per R3.6):
//   - risk_level   ← planner.risk_level / "medium"
//   - knowledge_need ← planner.knowledge_required ? "required" : "not_required"
//   - run_mode     ← derived from risk_level / planner.knowledge_required / planner.memory_change_importance
//   - needs_review ← (risk_level == "high" || planner.knowledge_required)
//   - consolidation_needed ← (memory_write_score >= 6)
//
// What remains here is the non-enumerated planner-sync semantic for
// `memory_write_score`: when the agent emitted an `operating_memory_update`
// payload but did not assign a write score, we still mirror the planner's
// `memory_change_importance`. This is *not* a default-fill for any of the 5
// autonomy-protocol fields above; it is a legacy compatibility hook used by
// `write_memory_candidates` to bucket pending vs. completed candidates and is
// safe to keep until W5 reworks the memory pipeline.
pub(crate) fn normalize_decision_runtime(decision: &mut AgentDecision, planner: &RunPlannerResult) {
    if decision.memory_write_score == 0 && !decision.operating_memory_update.is_empty() {
        decision.memory_write_score = planner.memory_change_importance;
    }
}

pub(crate) fn planner_from_decision(decision: &AgentDecision, reason: &str) -> RunPlannerResult {
    let risk_level = if decision.risk_level.trim().is_empty() {
        "medium".to_string()
    } else {
        decision.risk_level.clone()
    };
    let knowledge_required = decision_requires_knowledge(decision);
    RunPlannerResult {
        risk_level: risk_level.clone(),
        context_needs_refresh: false,
        memory_change_importance: decision.memory_write_score.clamp(0, 10),
        knowledge_required,
        review_mode: if decision.needs_review || risk_level == "high" || knowledge_required {
            "full".to_string()
        } else {
            "light".to_string()
        },
        reason: reason.to_string(),
        ..Default::default()
    }
}

pub(crate) fn decision_requires_knowledge(decision: &AgentDecision) -> bool {
    matches!(
        decision.knowledge_need.trim(),
        "required" | "insufficient" | "knowledge_required"
    )
}

pub(crate) fn operation_state_exists(
    domain_config: Option<&OperationDomainConfig>,
    key: &str,
) -> bool {
    let states = operation_states(domain_config);
    states.is_empty()
        || states
            .iter()
            .any(|state| state.get_str("key").ok() == Some(key))
}

pub(crate) fn operation_state_key_by_name(
    domain_config: Option<&OperationDomainConfig>,
    name: &str,
) -> Option<String> {
    operation_states(domain_config)
        .into_iter()
        .find(|state| state.get_str("name").ok() == Some(name))
        .and_then(|state| state.get_str("key").ok().map(ToString::to_string))
}

pub(crate) fn operation_states(domain_config: Option<&OperationDomainConfig>) -> Vec<Document> {
    domain_config
        .and_then(|config| config.state_machine.get_array("states").ok())
        .map(|states| {
            states
                .iter()
                .filter_map(|item| item.as_document().cloned())
                .collect()
        })
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────
// agent-autonomy-loop W2 / Task 3.5：R5 Verified Knowledge 强约束 helpers。
//
// requirements.md R5.1 / R5.2 / R5.3 / R5.4 / R5.6 / R5.7。
//
// 设计原则：
//   * **唯一 source-of-truth 是 `OperationKnowledgeChunk.integrity_status == "verified"`**
//     （R5.1）；不引入派生字段 `verified: bool / is_verified`，仅在 Rust 层
//     暴露 `is_verified(&chunk) -> bool` 内部 helper。API / prompt 中暴露的
//     字段名仍是 `integrity_status`。
//   * 这些 helper 由 W2 task 3.2 的 `finalize_review_for_send`（review.rs）
//     调用；本任务范围**只新增 helper + 单元测试**，不动 review/gateway 主
//     流程，与 task 3.2 文件范围解耦从而并行安全（design.md §2.3 的工作分
//     区）。
//   * 调用方一旦走到 R5.4 block，SHALL 把 `gateway_status` 设为
//     `blocked_unverified_product_claim`、`autonomy_mode="blocked"`、
//     `should_reply=false`；这部分由 task 3.2 的 finalize 路径完成，本模
//     块只暴露纯函数判定。
// ─────────────────────────────────────────────────────────────────

/// agent-autonomy-loop W2 / Task 3.5（R5.1）：单条知识切片是否 "verified"。
///
/// 唯一判定条件是 `chunk.integrity_status == "verified"`（trim 后比较，兼容
/// 历史脏数据写入了 `" verified "` 这种带空白的取值；大小写不敏感）。任何
/// 其它取值（`needs_review / rejected / draft / None / 任意空白 / ...`）一
/// 律视为**不可信**。
///
/// 该 helper 仅供本 crate 内部使用（`pub(crate)`），不暴露到 API / prompt /
/// 前端 schema，避免引入第二个 source-of-truth。
pub(crate) fn is_verified(chunk: &OperationKnowledgeChunk) -> bool {
    chunk
        .integrity_status
        .as_deref()
        .map(str::trim)
        .map(|s| s.eq_ignore_ascii_case("verified"))
        .unwrap_or(false)
}

/// agent-autonomy-loop W2 / Task 3.5（R5.2）：计算 verified_chunks 集合。
///
/// 定义为 `decision.used_knowledge_ids ∩ { chunk.id | chunk.integrity_status == "verified" }`。
/// 返回 `chunks` 切片中"既被 Reply Agent 引用、又通过校验"的切片**引用**
/// 列表（避免 clone 大量 body 字段），便于调用方在 R5.4 / R5.7 路径上按
/// 需聚合。
///
/// 实现细节：
/// * `used_knowledge_ids` 是 hex `ObjectId` 字符串（与
///   `select_operation_knowledge_chunks` 的索引方式一致，详见
///   `knowledge_router.rs`）；空 / 不可解析的 id 自动跳过；
/// * 如果同一 chunk 在切片中重复出现（理论不应发生，兜底）只计入 1 条；
/// * 返回顺序按 `chunks` 原始顺序，便于测试断言。
pub(crate) fn compute_verified_chunks<'a>(
    used_knowledge_ids: &[String],
    chunks: &'a [OperationKnowledgeChunk],
) -> Vec<&'a OperationKnowledgeChunk> {
    if used_knowledge_ids.is_empty() {
        return Vec::new();
    }
    let used: std::collections::HashSet<&str> = used_knowledge_ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if used.is_empty() {
        return Vec::new();
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<&'a OperationKnowledgeChunk> = Vec::new();
    for chunk in chunks {
        if !is_verified(chunk) {
            continue;
        }
        let Some(hex) = chunk.id.map(|id| id.to_hex()) else {
            continue;
        };
        if !used.contains(hex.as_str()) {
            continue;
        }
        if seen.insert(hex) {
            out.push(chunk);
        }
    }
    out
}

/// agent-autonomy-loop W2 / Task 3.5（R5.3）：判断 `claim_analysis` 是否
/// "缺失或损坏"。
///
/// 满足任一条件即视为 malformed：
///   * `claim_analysis` 整个 Document 为空（反序列化时上游 Review JSON 没
///     带该字段，由 `#[serde(default)]` 落到 `Document::new()`）；
///   * `requiresProductKnowledge` 与 `requires_product_knowledge`（兼容
///     camelCase / snake_case 两种历史命名）两 key 都不存在；
///   * 上述任一 key 存在但值不是 JSON `bool`（例如字符串 `"true"` / `null`）。
///
/// 返回 true 时调用方 SHALL 进入 R5.3.a / R5.3.b 推断分支：
///   * 先调 [`infer_product_claim_trigger`] 看是否要 fail-closed；
///   * 无论 fail-closed 是否触发，调用方 SHALL 都往 `risks` 追加
///     `"claim_analysis_malformed"` 留痕（详见 R5.3 末段）。
pub(crate) fn claim_analysis_is_malformed(claim_analysis: &Document) -> bool {
    if claim_analysis.is_empty() {
        return true;
    }
    let camel = claim_analysis.get("requiresProductKnowledge");
    let snake = claim_analysis.get("requires_product_knowledge");
    if camel.is_none() && snake.is_none() {
        return true;
    }
    if let Some(value) = camel {
        if value.as_bool().is_none() {
            return true;
        }
    }
    if let Some(value) = snake {
        if value.as_bool().is_none() {
            return true;
        }
    }
    false
}

/// agent-autonomy-loop W2 / Task 3.5（R5.3.a）：在 `claim_analysis` 缺失或
/// 损坏时推断"是否产品声明（fail-closed）"。
///
/// 三个条件命中任一 → 强制视为产品声明 → 调用方 SHALL 走 R5.4 强约束路
/// 径并把 `gateway_status` 设为 `blocked_by_safety_guard`：
///   1. `decision.knowledge_need ∈ {"required", "insufficient"}`；
///   2. `decision.used_knowledge_ids` 非空；
///   3. `decision.reply_text` 命中 [`enforce_string_fact_risk_guard`] 的产
///      品 / 价格 / 案例 / 承诺类 marker（命中且未被白名单豁免才算）。
///
/// 返回值：
///   * `Some(trigger)` — 命中，`trigger ∈ {"knowledge_need", "used_knowledge_ids", "string_marker_hit"}`。
///     调用方 SHALL 把这个字符串写入 `agent_events kind="claim_analysis_malformed_fail_closed"`
///     的 `detail.triggered_by` 字段（R5.3.a 末段）。
///   * `None` — 三条件都不命中，调用方走 R5.3.b 综合判断（仅 risks 标
///     记不 block）。
///
/// 命中按 R5.3.a 文本顺序短路（knowledge_need > used_knowledge_ids >
/// string_marker_hit），便于事件埋点稳定、单元测试可重现。
pub(crate) fn infer_product_claim_trigger(
    decision: &AgentDecision,
    markers: &ProductClaimMarkers,
) -> Option<&'static str> {
    let kn = decision.knowledge_need.trim();
    if matches!(kn, "required" | "insufficient") {
        return Some("knowledge_need");
    }
    if !decision.used_knowledge_ids.is_empty() {
        return Some("used_knowledge_ids");
    }
    let hits = markers.scan(&decision.reply_text);
    let real_hit = hits
        .iter()
        .any(|hit| !markers.passes_whitelist(&decision.reply_text, hit));
    if real_hit {
        return Some("string_marker_hit");
    }
    None
}

/// agent-autonomy-loop W2 / Task 3.5（R5.7）：safe_claims 反向门 — 计算
/// `safe_claims_used` 中没有被任何 verified_chunk 的 `safe_claims` 集合支
/// 撑的 claim 列表。
///
/// 返回值是按 `safe_claims_used` 原始顺序去重后的"未被支撑"列表；调用
/// 方接着用 [`append_unverified_safe_claim_risks`] 把这些 claim 转为
/// `safe_claim_not_verified:<claim>` risks（带 cap 5 + 聚合 overflow）。
///
/// 注意：
///   * 本函数 SHALL NOT 改变 `review.approved`（R5.7 末段："不单独 block"）；
///   * 空字符串 / 仅含空白的 safe_claim 直接跳过，避免无意义噪声；
///   * 同一 claim 重复出现只算一次。
pub(crate) fn compute_unverified_safe_claims(
    safe_claims_used: &[String],
    verified_chunks: &[&OperationKnowledgeChunk],
) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for claim in safe_claims_used {
        if claim.trim().is_empty() {
            continue;
        }
        if !seen.insert(claim.clone()) {
            continue;
        }
        let supported = verified_chunks
            .iter()
            .any(|chunk| chunk.safe_claims.iter().any(|sc| sc == claim));
        if !supported {
            out.push(claim.clone());
        }
    }
    out
}

/// `safe_claim_not_verified:*` risks 的硬上限（R5.7）。
///
/// 超出 cap 的 claim 不再单独追加，统一聚合为
/// `safe_claim_not_verified:and_more:<n>`。
const SAFE_CLAIM_RISK_CAP: usize = 5;

/// agent-autonomy-loop W2 / Task 3.5（R5.7）：把"未被 verified_chunk 支撑
/// 的 safe_claim"列表转为 `safe_claim_not_verified:<claim>` risks 并 push
/// 到 `risks`。
///
/// 行为：
///   * 单条 cap 5；
///   * 超出 cap 的 claim 聚合为 `safe_claim_not_verified:and_more:<n>`，
///     `<n>` 是被聚合的剩余条数；
///   * 即使 `unverified_claims` 为空也不追加 `and_more:0`（避免噪声）。
///
/// 调用方典型用法（在 task 3.2 finalize_review_for_send 内）：
/// ```ignore
/// let verified = compute_verified_chunks(&decision.used_knowledge_ids, knowledge_chunks);
/// let unverified = compute_unverified_safe_claims(&decision.safe_claims_used, &verified);
/// append_unverified_safe_claim_risks(&mut review.risks, &unverified);
/// // SHALL NOT 因为这些 risks 强制 review.approved = false
/// ```
pub(crate) fn append_unverified_safe_claim_risks(
    risks: &mut Vec<String>,
    unverified_claims: &[String],
) {
    for claim in unverified_claims.iter().take(SAFE_CLAIM_RISK_CAP) {
        risks.push(format!("safe_claim_not_verified:{claim}"));
    }
    let overflow = unverified_claims.len().saturating_sub(SAFE_CLAIM_RISK_CAP);
    if overflow > 0 {
        risks.push(format!("safe_claim_not_verified:and_more:{overflow}"));
    }
}

/// MP-7 / Task 13：检查 `from -> to` 是否合法。
///
/// 规则：
/// - 状态机为空（domain_config 缺失）时不做迁移校验，向后兼容老配置；
/// - 目标 state `allowFromAny=true`（如 cooldown）总是合法；
/// - `from` 为空时只有目标 = `new_contact` 合法；
/// - 否则 `from` 必须出现在目标 state 的 `allowedFrom` 列表中。
///
/// 返回 `Some(reason)` 表示拦截理由；返回 `None` 表示通过。
pub fn check_state_transition(
    domain_config: Option<&OperationDomainConfig>,
    from: Option<&str>,
    to: &str,
) -> Option<String> {
    let states = operation_states(domain_config);
    if states.is_empty() {
        return None; // 没有状态机不强校验。
    }
    let target = states
        .iter()
        .find(|state| state.get_str("key").ok() == Some(to))?;
    if target.get_bool("allowFromAny").unwrap_or(false) {
        return None;
    }
    let from = from.map(str::trim).filter(|s| !s.is_empty());
    match from {
        None => {
            if to == "new_contact" {
                None
            } else {
                Some(format!("state_transition_invalid: from=<empty> to={to}"))
            }
        }
        Some(from_key) => {
            let allowed: Vec<&str> = target
                .get_array("allowedFrom")
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if allowed.iter().any(|key| *key == from_key) {
                None
            } else {
                Some(format!("state_transition_invalid: from={from_key} to={to}"))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// agent-autonomy-loop W3 / Task 4.7：在 Reply Agent 决策落库前接入字典守卫。
//
// 设计 §4.2 / R8.2-R8.6：对 `decision.customer_stage / intent_level` 两字段
// 调 `taxonomy::check_value`，按 4 路分支处理：
//
// * `Active` → 通过，无操作；
// * `AliasActive(canonical_id)` → 把 decision 字段改写为 canonical_id（保证
//   下游 review / 写库 / 前端展示用统一的字典 id 而不是 alias 字面量）；
// * `Deprecated` → 在 `review.risks` 追加 `taxonomy_deprecated_value:<kind>:<value>`；
//   合法但提示运营人员考虑迁移；
// * `CandidateNew` → 在 `review.risks` 追加 `taxonomy_candidate:<kind>:<value>`
//   并异步 upsert 到 `taxonomy_candidates` 集合；**不**强制 review fail
//   （R8.4：自由维度信号不阻塞 Reply Agent 运行）。
//
// 注：`objection_type` 不是 `AgentDecision` 的直接字段（前端 / 字典层映射），
// 当前 W3 仅校验 `customer_stage / intent_level` 两个直连字段；若后续把
// objection_type 提升为 decision 字段，可在此处扩展第三路 check_value 调用。
// ─────────────────────────────────────────────────────────────────

/// agent-autonomy-loop W3 / Task 4.7：把 Reply Agent 输出的 customer_stage /
/// intent_level 字段对照 `system_taxonomies` 字典进行 alias 改写 + 候选 upsert。
///
/// 调用方约定：
///
/// * 在 `enforce_decision_guards_with_markers` **之前** 调用，因为 alias
///   改写会更新 `decision.customer_stage` 等字段；下游 review / state-machine
///   guard 应基于改写后的 canonical_id 工作。
/// * 调用方负责保证 [`crate::agent::taxonomy::TaxonomyCache`] 已加载（启动
///   期 `warm_up` 或 `ensure_cache_loaded`）；本函数不阻塞主路径，缓存未
///   加载时所有非空值都会落到 `CandidateNew` 分支（运行正常，只是会触发
///   多余的 upsert）。
/// * `upsert_candidate` 失败被静默（log warning），不阻塞 review；R8.4：
///   字典子系统的 IO 异常 SHALL NOT 影响 Reply Agent 主路径。
///
/// 产出：把 `taxonomy_candidate:<kind>:<value>` 与
/// `taxonomy_deprecated_value:<kind>:<value>` 标记 push 进 `risks_out`，由
/// 调用方合并到 `promote_risks` / `review.risks`（详见 design.md §4.5）。
/// 注意：这些 risk **不**强制 `review.approved=false`（R8.4）。
pub(crate) async fn enforce_decision_taxonomy_guards(
    db: &Database,
    cache: &crate::agent::taxonomy::TaxonomyCache,
    account_id: &str,
    decision: &mut AgentDecision,
    risks_out: &mut Vec<String>,
) {
    use crate::agent::taxonomy::{upsert_candidate, TaxonomyKind};

    // 1) 同步纯函数：alias 改写 + risks 收集 + 返回需要 upsert 的候选列表。
    let candidates = compute_taxonomy_resolutions(cache, account_id, decision, risks_out);

    // 2) 异步：把候选列表写入 `taxonomy_candidates`；失败被静默（R8.4 要求
    //    字典 IO 异常不阻塞 Reply Agent 主路径）。
    for (kind, raw) in candidates {
        let kind_enum = match kind.as_str() {
            "customer_stage" => TaxonomyKind::CustomerStage,
            "intent_level" => TaxonomyKind::IntentLevel,
            "objection_type" => TaxonomyKind::ObjectionType,
            _ => continue,
        };
        if let Err(error) = upsert_candidate(db, account_id, kind_enum, &raw, None, 7).await {
            tracing::warn!(
                ?error,
                kind = kind.as_str(),
                raw = raw.as_str(),
                "taxonomy::upsert_candidate failed; non-blocking"
            );
        }
    }
}

/// agent-autonomy-loop W3 / Task 4.7（纯同步部分）：对照字典 alias / deprecated /
/// candidate 4 路分支，**只**做 in-memory 判定与 alias 改写：
///
/// * 命中 Active：无操作；
/// * 命中 AliasActive：把 `decision.{customer_stage, intent_level}` 改写为 canonical id；
/// * 命中 Deprecated：往 `risks_out` 追加 `taxonomy_deprecated_value:<kind>:<raw>`；
/// * 命中 CandidateNew：往 `risks_out` 追加 `taxonomy_candidate:<kind>:<raw>`，
///   并把 `(kind, raw)` 加入返回列表，让调用方异步 upsert。
///
/// 拆分意义：CandidateNew 的 upsert 走 IO（`taxonomy_candidates` collection），
/// 在没有 Mongo 测试容器的纯 lib 单元测试中无法直接验证；但 alias 改写 / risks
/// 收集 / candidate 列表生成 全是纯函数，单元测试可直接对照断言。
pub fn compute_taxonomy_resolutions(
    cache: &crate::agent::taxonomy::TaxonomyCache,
    account_id: &str,
    decision: &mut AgentDecision,
    risks_out: &mut Vec<String>,
) -> Vec<(String, String)> {
    use crate::agent::taxonomy::{check_value, TaxonomyKind, TaxonomyMatch};

    let mut to_upsert: Vec<(String, String)> = Vec::new();

    // customer_stage：trim 后非空才校验，避免把 "" / "  " 误送入候选。
    if let Some(raw) = decision
        .customer_stage
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        match check_value(TaxonomyKind::CustomerStage, &raw, account_id, cache) {
            TaxonomyMatch::Active => {}
            TaxonomyMatch::AliasActive(canonical) => {
                decision.customer_stage = Some(canonical);
            }
            TaxonomyMatch::Deprecated => {
                risks_out.push(format!("taxonomy_deprecated_value:customer_stage:{raw}"));
            }
            TaxonomyMatch::CandidateNew => {
                risks_out.push(format!("taxonomy_candidate:customer_stage:{raw}"));
                to_upsert.push(("customer_stage".to_string(), raw));
            }
        }
    }

    // intent_level：同上。
    if let Some(raw) = decision
        .intent_level
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        match check_value(TaxonomyKind::IntentLevel, &raw, account_id, cache) {
            TaxonomyMatch::Active => {}
            TaxonomyMatch::AliasActive(canonical) => {
                decision.intent_level = Some(canonical);
            }
            TaxonomyMatch::Deprecated => {
                risks_out.push(format!("taxonomy_deprecated_value:intent_level:{raw}"));
            }
            TaxonomyMatch::CandidateNew => {
                risks_out.push(format!("taxonomy_candidate:intent_level:{raw}"));
                to_upsert.push(("intent_level".to_string(), raw));
            }
        }
    }

    to_upsert
}

#[cfg(test)]
mod r5_verified_knowledge_tests {
    //! agent-autonomy-loop W2 / Task 3.5：R5 verified-knowledge helpers 单元测试。
    //!
    //! 校验范围：
    //!   * `is_verified` — `integrity_status == "verified"` 唯一判定 + 大小写 / 空白容错；
    //!   * `compute_verified_chunks` — used_knowledge_ids ∩ verified_chunks 集合语义；
    //!   * `claim_analysis_is_malformed` — 空 / 缺 key / 类型非 bool 三类情况；
    //!   * `infer_product_claim_trigger` — R5.3.a 三条件短路（knowledge_need >
    //!     used_knowledge_ids > string_marker_hit）；
    //!   * `compute_unverified_safe_claims` + `append_unverified_safe_claim_risks` —
    //!     R5.7 反向门 + cap 5 + `and_more:<n>` 聚合。

    use super::{
        append_unverified_safe_claim_risks, claim_analysis_is_malformed, compute_unverified_safe_claims,
        compute_verified_chunks, default_product_claim_markers, infer_product_claim_trigger,
        is_verified,
    };
    use crate::agent::types::AgentDecision;
    use crate::models::OperationKnowledgeChunk;
    use mongodb::bson::{doc, oid::ObjectId, DateTime};

    fn chunk_with(integrity_status: Option<&str>, safe_claims: Vec<&str>) -> OperationKnowledgeChunk {
        OperationKnowledgeChunk {
            id: Some(ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "user_operations".to_string(),
            knowledge_type: None,
            business_context: None,
            title: "test_chunk".to_string(),
            summary: None,
            body: None,
            routing_card: None,
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            safe_claims: safe_claims.into_iter().map(ToString::to_string).collect(),
            forbidden_claims: vec![],
            evidence_items: vec![],
            source_quote: None,
            source_anchors: vec![],
            integrity_status: integrity_status.map(ToString::to_string),
            confidence_score: None,
            distortion_risks: vec![],
            unsupported_claims: vec![],
            verified_claims: vec![],
            status: "active".to_string(),
            priority: 0,
            product_tags: vec![],
            trigger_keywords: vec![],
            business_topics: vec![],
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    // ────────── is_verified ──────────

    #[test]
    fn is_verified_only_true_for_exact_verified_status() {
        // R5.1：唯一判定条件是 `integrity_status == "verified"`。
        assert!(is_verified(&chunk_with(Some("verified"), vec![])));
        assert!(
            is_verified(&chunk_with(Some(" verified "), vec![])),
            "trim 后比较：兼容老数据写入了带空白的取值"
        );
        assert!(
            is_verified(&chunk_with(Some("VERIFIED"), vec![])),
            "大小写不敏感（保留鲁棒性）"
        );

        for bad in ["needs_review", "rejected", "draft", "", "  ", "verifie", "veriFiedX"] {
            assert!(
                !is_verified(&chunk_with(Some(bad), vec![])),
                "integrity_status={bad:?} 不应视为 verified"
            );
        }
        assert!(!is_verified(&chunk_with(None, vec![])), "缺失字段不可信");
    }

    // ────────── compute_verified_chunks ──────────

    #[test]
    fn compute_verified_chunks_returns_intersection() {
        // R5.2：定义 verified_chunks = used_knowledge_ids ∩ verified chunks。
        let mut a = chunk_with(Some("verified"), vec!["claim_a"]);
        let mut b = chunk_with(Some("needs_review"), vec!["claim_b"]);
        let mut c = chunk_with(Some("verified"), vec!["claim_c"]);
        a.id = Some(ObjectId::new());
        b.id = Some(ObjectId::new());
        c.id = Some(ObjectId::new());
        let a_hex = a.id.unwrap().to_hex();
        let _b_hex = b.id.unwrap().to_hex();
        let c_hex = c.id.unwrap().to_hex();

        let used = vec![a_hex.clone(), c_hex.clone()];
        let chunks = vec![a.clone(), b.clone(), c.clone()];
        let verified = compute_verified_chunks(&used, &chunks);
        let verified_ids: Vec<String> = verified
            .iter()
            .map(|chunk| chunk.id.unwrap().to_hex())
            .collect();
        assert_eq!(verified_ids, vec![a_hex, c_hex]);
    }

    #[test]
    fn compute_verified_chunks_excludes_unverified_even_if_referenced() {
        // R5.2：used_knowledge_ids 命中但 integrity_status 非 verified 时不计入。
        let chunk = chunk_with(Some("needs_review"), vec![]);
        let hex = chunk.id.unwrap().to_hex();
        let verified = compute_verified_chunks(&[hex], std::slice::from_ref(&chunk));
        assert!(
            verified.is_empty(),
            "needs_review 切片即使被 used_knowledge_ids 引用也不应进入 verified_chunks"
        );
    }

    #[test]
    fn compute_verified_chunks_handles_empty_inputs() {
        let chunk = chunk_with(Some("verified"), vec![]);
        assert!(compute_verified_chunks(&[], std::slice::from_ref(&chunk)).is_empty());
        assert!(compute_verified_chunks(&["any".to_string()], &[]).is_empty());
    }

    #[test]
    fn compute_verified_chunks_skips_unknown_or_blank_ids() {
        // 不可解析 / 空白 id 自动跳过，不影响 valid id 命中。
        let chunk = chunk_with(Some("verified"), vec![]);
        let hex = chunk.id.unwrap().to_hex();
        let verified = compute_verified_chunks(
            &[
                "".to_string(),
                "   ".to_string(),
                "not_a_real_id".to_string(),
                hex.clone(),
            ],
            std::slice::from_ref(&chunk),
        );
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].id.unwrap().to_hex(), hex);
    }

    // ────────── claim_analysis_is_malformed ──────────

    #[test]
    fn malformed_when_empty_or_missing_key() {
        // R5.3：claim_analysis 缺失 / 整体为空。
        assert!(claim_analysis_is_malformed(&doc! {}));

        // 没有 requiresProductKnowledge 任何形态。
        assert!(claim_analysis_is_malformed(&doc! {
            "hasProductClaim": true,
            "knowledgeSupported": false
        }));
    }

    #[test]
    fn malformed_when_value_is_not_bool() {
        // 字符串 "true" 不是 JSON bool；视为损坏。
        assert!(claim_analysis_is_malformed(&doc! {
            "requiresProductKnowledge": "true"
        }));
        assert!(claim_analysis_is_malformed(&doc! {
            "requires_product_knowledge": 1_i32
        }));
    }

    #[test]
    fn well_formed_claim_analysis_passes() {
        assert!(!claim_analysis_is_malformed(&doc! {
            "requiresProductKnowledge": true
        }));
        assert!(!claim_analysis_is_malformed(&doc! {
            "requires_product_knowledge": false
        }));
        assert!(!claim_analysis_is_malformed(&doc! {
            "requiresProductKnowledge": false,
            "knowledgeSupported": true
        }));
    }

    // ────────── infer_product_claim_trigger ──────────

    #[test]
    fn trigger_short_circuits_on_knowledge_need_required() {
        let mut decision = AgentDecision::default();
        decision.knowledge_need = "required".to_string();
        decision.reply_text = "纯闲聊".to_string();
        let markers = default_product_claim_markers();
        assert_eq!(infer_product_claim_trigger(&decision, &markers), Some("knowledge_need"));
    }

    #[test]
    fn trigger_short_circuits_on_knowledge_need_insufficient() {
        let mut decision = AgentDecision::default();
        decision.knowledge_need = "insufficient".to_string();
        let markers = default_product_claim_markers();
        assert_eq!(infer_product_claim_trigger(&decision, &markers), Some("knowledge_need"));
    }

    #[test]
    fn trigger_falls_through_to_used_knowledge_ids() {
        let mut decision = AgentDecision::default();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = vec!["chunk_xxx".to_string()];
        let markers = default_product_claim_markers();
        assert_eq!(
            infer_product_claim_trigger(&decision, &markers),
            Some("used_knowledge_ids")
        );
    }

    #[test]
    fn trigger_falls_through_to_string_marker_hit() {
        let mut decision = AgentDecision::default();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = vec![];
        decision.reply_text = "我们能保证你转化提升 30%".to_string();
        let markers = default_product_claim_markers();
        assert_eq!(
            infer_product_claim_trigger(&decision, &markers),
            Some("string_marker_hit")
        );
    }

    #[test]
    fn trigger_returns_none_for_pure_chitchat() {
        // R5.3.b：闲聊场景三条件都不命中。
        let mut decision = AgentDecision::default();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = vec![];
        decision.reply_text = "好的，谢谢你的问候。".to_string();
        let markers = default_product_claim_markers();
        assert_eq!(infer_product_claim_trigger(&decision, &markers), None);
    }

    #[test]
    fn trigger_string_marker_respects_whitelist() {
        // 命中 marker 但被白名单豁免（"准时"+"保证"在 8 字符窗口内）→ 不算 hit。
        let mut decision = AgentDecision::default();
        decision.knowledge_need = "not_required".to_string();
        decision.used_knowledge_ids = vec![];
        decision.reply_text = "我会准时回复你，保证不会让你等太久".to_string();
        let markers = default_product_claim_markers();
        assert_eq!(infer_product_claim_trigger(&decision, &markers), None);
    }

    // ────────── compute_unverified_safe_claims + append_unverified_safe_claim_risks ──────────

    #[test]
    fn unverified_safe_claims_excludes_supported_ones() {
        // R5.7：每个 safe_claim 必须在 verified_chunk.safe_claims 中找到至少一条支撑。
        let chunk = chunk_with(Some("verified"), vec!["claim_a", "claim_b"]);
        let verified: Vec<&OperationKnowledgeChunk> = vec![&chunk];

        let used = vec!["claim_a".to_string(), "claim_c".to_string(), "claim_b".to_string()];
        let unverified = compute_unverified_safe_claims(&used, &verified);
        assert_eq!(unverified, vec!["claim_c".to_string()]);
    }

    #[test]
    fn unverified_safe_claims_dedupes_repeated_input() {
        let chunk = chunk_with(Some("verified"), vec!["claim_a"]);
        let verified: Vec<&OperationKnowledgeChunk> = vec![&chunk];

        let used = vec![
            "claim_unsupported".to_string(),
            "claim_unsupported".to_string(),
            "claim_a".to_string(),
        ];
        let unverified = compute_unverified_safe_claims(&used, &verified);
        assert_eq!(unverified, vec!["claim_unsupported".to_string()]);
    }

    #[test]
    fn unverified_safe_claims_skips_blank_inputs() {
        let chunk = chunk_with(Some("verified"), vec![]);
        let verified: Vec<&OperationKnowledgeChunk> = vec![&chunk];
        let used = vec!["".to_string(), "  ".to_string()];
        assert!(compute_unverified_safe_claims(&used, &verified).is_empty());
    }

    #[test]
    fn append_risks_caps_individual_at_five_and_aggregates_overflow() {
        // 7 条未支撑 → 单独写出 5 条 + `and_more:2` 一条共 6 条 risks。
        let claims: Vec<String> = (1..=7).map(|i| format!("c{i}")).collect();
        let mut risks: Vec<String> = Vec::new();
        append_unverified_safe_claim_risks(&mut risks, &claims);
        assert_eq!(risks.len(), 6);
        for (i, claim) in claims.iter().take(5).enumerate() {
            assert_eq!(risks[i], format!("safe_claim_not_verified:{claim}"));
        }
        assert_eq!(risks[5], "safe_claim_not_verified:and_more:2");
    }

    #[test]
    fn append_risks_no_overflow_marker_when_under_cap() {
        let claims = vec!["c1".to_string(), "c2".to_string()];
        let mut risks: Vec<String> = Vec::new();
        append_unverified_safe_claim_risks(&mut risks, &claims);
        assert_eq!(
            risks,
            vec![
                "safe_claim_not_verified:c1".to_string(),
                "safe_claim_not_verified:c2".to_string(),
            ],
            "≤ cap 时不应出现 and_more"
        );
    }

    #[test]
    fn append_risks_no_op_when_empty() {
        let mut risks: Vec<String> = vec!["pre_existing".to_string()];
        append_unverified_safe_claim_risks(&mut risks, &[]);
        assert_eq!(risks, vec!["pre_existing".to_string()], "空输入不追加任何 risk");
    }
}

#[cfg(test)]
mod r8_taxonomy_guard_tests {
    //! agent-autonomy-loop W3 / Task 4.10：字典 / candidate 守卫单元测试。
    //!
    //! 校验 R8.4 / R8.6 / R8.9 子条款：
    //!   * (a) `customer_stage` 不在字典 → 追加 `taxonomy_candidate:*` risk +
    //!     候选列表非空 + `review.approved` **不**被强制 false（R8.4 核心）；
    //!   * (b) alias 命中 → 改写 canonical_id + 不写候选 + 不追加 risk；
    //!   * (c) deprecated 值 → 仅追加 `taxonomy_deprecated_value:*` 不写候选；
    //!   * (d) approve 候选后再次同值 → 字典已含该值（用模拟 cache 模拟"已 approve"）
    //!     → 走 Active 不再写候选；
    //!   * (e) operation_state 不在状态机走 R3.5 路径而非 R8 路径（已由
    //!     `enforce_decision_guards_with_markers` 中状态机分支测试覆盖；本处通过
    //!     断言 `compute_taxonomy_resolutions` 不接触 `operation_state` 字段验证）；
    //!   * (f) `agent_generated_signals` 字段任意值都不写候选、不影响 risks。

    use super::compute_taxonomy_resolutions;
    use crate::agent::taxonomy::taxonomy_cache_for_tests;
    use crate::agent::types::{AgentDecision, AgentSignal};
    use crate::models::{TaxonomyEntry, TaxonomyValue};
    use mongodb::bson::DateTime;

    fn make_entry(
        scope: &str,
        kind: &str,
        canonical: &str,
        aliases: &[&str],
        status: &str,
    ) -> TaxonomyEntry {
        TaxonomyEntry {
            id: None,
            scope: scope.to_string(),
            kind: kind.to_string(),
            value: TaxonomyValue {
                id: canonical.to_string(),
                display_name: canonical.to_string(),
                description: String::new(),
                aliases: aliases.iter().map(|s| s.to_string()).collect(),
                status: status.to_string(),
            },
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn unknown_customer_stage_yields_candidate_risk_and_does_not_block() {
        // R8.4 / R8.9.a：不在字典的值 → `taxonomy_candidate:*` risk + 候选列表非空。
        let cache = taxonomy_cache_for_tests(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            &["新客"],
            "active",
        )]);
        let mut decision = AgentDecision::default();
        decision.customer_stage = Some("不在字典里的新客阶段".to_string());
        let mut risks: Vec<String> = Vec::new();
        let to_upsert =
            compute_taxonomy_resolutions(&cache, "acct-1", &mut decision, &mut risks);
        assert!(
            risks
                .iter()
                .any(|r| r == "taxonomy_candidate:customer_stage:不在字典里的新客阶段"),
            "expected taxonomy_candidate risk, got {risks:?}"
        );
        assert_eq!(
            to_upsert,
            vec![(
                "customer_stage".to_string(),
                "不在字典里的新客阶段".to_string()
            )],
            "candidate list should contain the unknown raw value for upsert"
        );
        // R8.4：不强制 review.approved=false（finalize_review_for_send 不会因这条 risk 阻断）。
        // 我们只能在 `finalize_review_for_send` 内部判断；此处通过断言 risks 中
        // 没有任何 `missing_required_field:*` / `invalid_enum_value:*` 来验证不走必填路径。
        assert!(
            !risks.iter().any(|r| r.starts_with("missing_required_field")
                || r.starts_with("invalid_enum_value")),
            "taxonomy_candidate must not produce R3.5 violation risks: {risks:?}"
        );
    }

    #[test]
    fn alias_match_rewrites_decision_to_canonical_id_without_risk() {
        // R8.9.b：字典 alias 命中视为合法 → 改写 canonical_id + 不追加 risk + 不写候选。
        let cache = taxonomy_cache_for_tests(vec![make_entry(
            "global",
            "customer_stage",
            "first_contact",
            &["新客", "刚加好友"],
            "active",
        )]);
        let mut decision = AgentDecision::default();
        decision.customer_stage = Some("新客".to_string());
        let mut risks: Vec<String> = Vec::new();
        let to_upsert =
            compute_taxonomy_resolutions(&cache, "acct-1", &mut decision, &mut risks);
        assert_eq!(
            decision.customer_stage,
            Some("first_contact".to_string()),
            "alias '新客' should be rewritten to canonical 'first_contact'"
        );
        assert!(risks.is_empty(), "alias hit must not append any risks: {risks:?}");
        assert!(to_upsert.is_empty(), "alias hit must not enqueue candidate upsert");
    }

    #[test]
    fn deprecated_value_warns_but_does_not_block_or_enqueue_candidate() {
        // R8.6 / R8.9.d：deprecated 值只追加 `taxonomy_deprecated_value:*` 不写候选。
        let cache = taxonomy_cache_for_tests(vec![make_entry(
            "global",
            "intent_level",
            "lukewarm",
            &[],
            "deprecated",
        )]);
        let mut decision = AgentDecision::default();
        decision.intent_level = Some("lukewarm".to_string());
        let mut risks: Vec<String> = Vec::new();
        let to_upsert =
            compute_taxonomy_resolutions(&cache, "acct-1", &mut decision, &mut risks);
        assert_eq!(
            risks,
            vec!["taxonomy_deprecated_value:intent_level:lukewarm".to_string()],
            "deprecated must produce exactly one warning risk"
        );
        assert!(
            to_upsert.is_empty(),
            "deprecated must not enqueue a candidate (it's already a known value)"
        );
        assert_eq!(
            decision.intent_level,
            Some("lukewarm".to_string()),
            "deprecated value remains as-is (no alias rewrite)"
        );
    }

    #[test]
    fn approved_value_is_treated_as_active_and_does_not_re_enqueue_candidate() {
        // R8.9.e：approve 候选后写入字典且下次同值不再写候选。
        // 模拟"已 approve"的状态：cache 中已含该值为 active；同样的 raw value
        // 现在应该走 Active 分支，不再写候选。
        let cache = taxonomy_cache_for_tests(vec![make_entry(
            "global",
            "objection_type",
            "新顾虑类型",
            &[],
            "active",
        )]);
        let mut decision = AgentDecision::default();
        // objection_type 不是 AgentDecision 直连字段；在该函数中 customer_stage / intent_level
        // 是直连字段，因此用 customer_stage 同值替代验证 approve 后行为。
        let cache2 = taxonomy_cache_for_tests(vec![make_entry(
            "global",
            "customer_stage",
            "premium_returning",
            &[],
            "active",
        )]);
        decision.customer_stage = Some("premium_returning".to_string());
        let mut risks: Vec<String> = Vec::new();
        let to_upsert =
            compute_taxonomy_resolutions(&cache2, "acct-1", &mut decision, &mut risks);
        assert!(risks.is_empty(), "approved value must not emit any risk: {risks:?}");
        assert!(
            to_upsert.is_empty(),
            "approved value must not be re-enqueued as candidate"
        );
        // cache 不变量：objection_type 字典存在但不影响 customer_stage 路径。
        let _ = cache;
    }

    #[test]
    fn agent_generated_signals_are_not_validated_against_dict() {
        // R8.9.f：`agentGeneratedSignals` 任意值都被接受、不写候选、不影响聚合。
        // 由于 `compute_taxonomy_resolutions` 只看 customer_stage / intent_level，本测试
        // 通过填充任意 agent_generated_signals + 不填字典字段验证：函数不读取该
        // 字段、不产生候选 / risks。
        let cache = taxonomy_cache_for_tests(vec![]);
        let mut decision = AgentDecision::default();
        decision.agent_generated_signals = vec![
            AgentSignal {
                kind: "free_form_kind_自由维度".to_string(),
                value: "用户对竞品很关注（自由信号）".to_string(),
                evidence: Some("最近三轮提到竞品名".to_string()),
                confidence: 7,
            },
            AgentSignal {
                kind: "another".to_string(),
                value: "另一条任意自由信号".to_string(),
                evidence: None,
                confidence: 5,
            },
        ];
        let mut risks: Vec<String> = Vec::new();
        let to_upsert =
            compute_taxonomy_resolutions(&cache, "acct-1", &mut decision, &mut risks);
        assert!(risks.is_empty());
        assert!(to_upsert.is_empty());
    }
}


// ── P4 性质测试（agent-autonomy-loop W3 / Task 4.14：≥ 64 用例）─────────
//
// **Property 4: 产品声明强约束**
// **Validates: Requirements 5.4, 5.7**
//
// 性质：随机生成 (`claim_analysis.requiresProductKnowledge=true`,
// `used_knowledge_ids`, `integrity_status_set`) 三元组，当
// `used_knowledge_ids ∩ verified_chunk_set == ∅` 时，
// `compute_verified_chunks` 返回空 → 上层 finalize_review_for_send
// SHALL 触发 `blocked_unverified_product_claim`。本 PBT 直接验证
// 集合不变量（核心 helper 层），与 finalize 集成层留给单元测试。

#[cfg(test)]
mod p4_pbt {
    use super::{compute_verified_chunks, is_verified};
    use crate::models::OperationKnowledgeChunk;
    use mongodb::bson::{oid::ObjectId, DateTime};
    use proptest::prelude::*;

    fn arbitrary_status() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            Just(Some("verified".to_string())),
            Just(Some("needs_review".to_string())),
            Just(Some("unverified".to_string())),
            Just(Some("draft".to_string())),
            Just(None),
            Just(Some(" verified ".to_string())), // 边界：空白容错由 is_verified 处理
        ]
    }

    fn arbitrary_chunks() -> impl Strategy<Value = Vec<OperationKnowledgeChunk>> {
        proptest::collection::vec(
            arbitrary_status().prop_map(|status| OperationKnowledgeChunk {
                id: Some(ObjectId::new()),
                workspace_id: "w".to_string(),
                account_id: None,
                document_id: None,
                item_id: None,
                domain: "user_operations".to_string(),
                knowledge_type: None,
                business_context: None,
                title: "t".to_string(),
                summary: None,
                body: None,
                routing_card: None,
                applicable_scenes: vec![],
                not_applicable_scenes: vec![],
                safe_claims: vec![],
                forbidden_claims: vec![],
                evidence_items: vec![],
                source_quote: None,
                source_anchors: vec![],
                integrity_status: status,
                confidence_score: None,
                distortion_risks: vec![],
                unsupported_claims: vec![],
                verified_claims: vec![],
                status: "active".to_string(),
                priority: 0,
                product_tags: vec![],
                trigger_keywords: vec![],
                business_topics: vec![],
                created_at: DateTime::now(),
                updated_at: DateTime::now(),
            }),
            1..=8,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 100,
            ..ProptestConfig::default()
        })]

        /// P4-a：当 `used_knowledge_ids` 全部指向非 verified chunks 时，
        /// `compute_verified_chunks` SHALL 返回空集合。
        #[test]
        fn p4_unverified_used_ids_yield_empty_verified(
            chunks in arbitrary_chunks(),
        ) {
            // 选取所有非 verified chunks 的 id 作为 used_knowledge_ids。
            let unverified_ids: Vec<String> = chunks
                .iter()
                .filter(|c| !is_verified(c))
                .filter_map(|c| c.id.map(|id| id.to_hex()))
                .collect();
            prop_assume!(!unverified_ids.is_empty());
            let verified = compute_verified_chunks(&unverified_ids, &chunks);
            prop_assert!(
                verified.is_empty(),
                "used_ids ∩ verified == ∅ 必然成立，实际 verified.len()={}",
                verified.len()
            );
        }

        /// P4-b：当 `used_knowledge_ids` 含至少一个 verified chunk id 时，
        /// `compute_verified_chunks` 必非空。
        #[test]
        fn p4_one_verified_used_id_yields_non_empty(
            chunks in arbitrary_chunks(),
        ) {
            let verified_ids: Vec<String> = chunks
                .iter()
                .filter(|c| is_verified(c))
                .filter_map(|c| c.id.map(|id| id.to_hex()))
                .collect();
            prop_assume!(!verified_ids.is_empty());
            let verified = compute_verified_chunks(&verified_ids, &chunks);
            prop_assert!(
                !verified.is_empty(),
                "至少一个 verified id 应命中"
            );
            // 全部命中元素的 integrity_status 都应为 verified。
            for c in &verified {
                prop_assert!(is_verified(c));
            }
        }

        /// P4-c：互补性——
        /// `compute_verified_chunks(used, chunks).len() <= used.len()`，
        /// 且不会"创造"chunks 中不存在的元素。
        #[test]
        fn p4_verified_subset_of_chunks(
            chunks in arbitrary_chunks(),
        ) {
            let all_ids: Vec<String> = chunks
                .iter()
                .filter_map(|c| c.id.map(|id| id.to_hex()))
                .collect();
            let verified = compute_verified_chunks(&all_ids, &chunks);
            prop_assert!(verified.len() <= chunks.len());
            for c in &verified {
                prop_assert!(chunks.iter().any(|cc| cc.id == c.id));
            }
        }
    }
}
