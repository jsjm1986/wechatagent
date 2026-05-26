//! 长期记忆与 memoryCard 整理 (MP-8)。
//!
//! 该模块覆盖以下职责：
//! - `default_memory_card` / `default_context_pack` 默认结构；
//! - `compact_memory_card_with_previous`，保证 coreFacts/recentFacts 等数组
//!   截留与上一版合并语义；
//! - `effective_memory_card_for_contact`：从 `OperatingMemory` 与 `Contact`
//!   推出当前 prompt 应注入的 memoryCard；
//! - `memory_card_has_signal`、`memory_card_from_contact` 等辅助；
//! - `consolidate_contact_memory` / `handle_memory_consolidation_task`：
//!   memory_consolidator Agent 入口，负责合并候选记忆并写回；
//! - `write_memory_candidates` 与 `schedule_memory_consolidation_task`
//!   等运行时辅助。
//!
//! agent-autonomy-loop W5 task 6.3：所有 helper 签名（`default_memory_card` /
//! `memory_card_from_contact` / `compact_memory_card_with_previous` /
//! `consolidate_contact_memory`）统一以 [`MemoryCardTyped`] 为入参与返回类型，
//! 写入路径通过 `bson::to_document(&MemoryCardTyped)` 一次性序列化，不再保留
//! Document / typed 两套并行表示。Document 形态仅在 prompt 注入 / 路由 JSON
//! 响应等"对外 wire shape" 边界出现，由 `to_document()` 在最末端一次性转换。

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, to_bson, to_document, Bson, DateTime, Document};
use mongodb::options::FindOptions;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::models::{
    AgentProfile, AgentTask, Contact, MemoryCandidate, MemoryCardTyped, MemoryFact,
    MemoryFactRepr, OperatingMemory,
};
use crate::prompts;
use crate::routes::AppState;

use super::gateway::write_event_for_account;
use super::generate_agent_json;
use super::types::{doc_i32, doc_string, AgentDecision};
use super::{RunBudget, RUN_BUDGET};

pub(crate) fn default_context_pack() -> Document {
    doc! {
        "confirmedFacts": Vec::<String>::new(),
        "preferences": Vec::<String>::new(),
        "painPoints": Vec::<String>::new(),
        "objections": Vec::<String>::new(),
        "commitments": Vec::<String>::new(),
        "doNotDo": Vec::<String>::new(),
        "relationshipTimeline": Vec::<Document>::new(),
        "recentSignals": Vec::<String>::new(),
        "openQuestions": Vec::<String>::new(),
        "importantQuotes": Vec::<String>::new(),
        "stalenessWarnings": Vec::<String>::new(),
        "deprecatedFacts": Vec::<Document>::new(),
        "conflicts": Vec::<Document>::new()
    }
}

/// task 6.3：返回 typed 形态的 memoryCard 默认值。所有 typed 字段空，业务
/// 标量（`coreProfile / relationshipState / source / version`）通过 `extra`
/// 兜底承接历史 schema 形状，便于把 typed 直接 `to_document()` 后落库 / 注入
/// prompt 时与既有 wire shape 保持一致。
pub(crate) fn default_memory_card() -> MemoryCardTyped {
    let mut extra = Document::new();
    extra.insert(
        "coreProfile",
        doc! {
            "identity": "",
            "businessContext": "",
            "communicationStyle": "",
            "operationGoal": ""
        },
    );
    extra.insert(
        "relationshipState",
        doc! {
            "stage": "unknown",
            "trustLevel": "unknown",
            "temperature": "unknown",
            "lastEmotion": ""
        },
    );
    extra.insert("preferences", Vec::<String>::new());
    extra.insert("doNotDo", Vec::<String>::new());
    extra.insert("commitments", Vec::<String>::new());
    extra.insert("objections", Vec::<String>::new());
    extra.insert("openLoops", Vec::<String>::new());
    extra.insert("recentEpisodeSummary", "");
    extra.insert("conflicts", Vec::<Document>::new());
    extra.insert("source", "memory_card");
    extra.insert("version", 0_i32);
    MemoryCardTyped {
        core_facts: Vec::new(),
        recent_facts: Vec::new(),
        deprecated_facts: Vec::new(),
        extra,
    }
}

/// task 6.3：返回当前 OperatingMemory 上"已生效"的 memoryCard（typed 形态）。
/// 当前 typed 字段全空、且 `context_pack` 有内容时退回 `context_pack`（历史
/// 兼容路径），最终一律走一次 `compact_memory_card_with_previous` 拿到 cap
/// 后形态。Document 版本由调用方在 prompt 注入 / 路由 JSON 响应等 wire 边界
/// 通过 `to_document()` 一次性转换。
pub(crate) fn effective_memory_card(memory: &OperatingMemory) -> MemoryCardTyped {
    if !memory.memory_card.is_empty() {
        compact_memory_card_with_previous(&memory.memory_card, None, &[])
    } else if !memory.context_pack.is_empty() {
        let from_pack = MemoryCardTyped::from_document(&memory.context_pack);
        compact_memory_card_with_previous(&from_pack, None, &[])
    } else {
        default_memory_card()
    }
}

/// task 6.3：返回带 `version` 注入的 typed memoryCard，用于 prompt 注入 /
/// 路由 JSON 响应。`memory.memory_card_version` 落到 `extra.version`，与
/// 历史 wire shape 一致。
pub(crate) fn effective_memory_card_for_contact(
    memory: &OperatingMemory,
    contact: &Contact,
) -> MemoryCardTyped {
    let card = effective_memory_card(memory);
    let mut compact = if memory_card_has_signal(&card) {
        compact_memory_card_with_previous(&card, None, &[])
    } else {
        compact_memory_card_with_previous(&memory_card_from_contact(contact, memory), None, &[])
    };
    compact
        .extra
        .insert("version", memory.memory_card_version);
    compact
}

/// task 6.3：判断 typed memoryCard 是否含"业务信号"。判定逻辑覆盖三类：
/// 1. typed 字段（`core_facts / recent_facts / deprecated_facts`）任一非空；
/// 2. `extra` 中数组类字段（`preferences / doNotDo / commitments / objections /
///    openLoops / conflicts`）任一非空；
/// 3. `extra.recentEpisodeSummary` 非空字符串，或 `extra.coreProfile` 任一文本
///    字段（identity / businessContext / communicationStyle / operationGoal）
///    非空。
pub(crate) fn memory_card_has_signal(card: &MemoryCardTyped) -> bool {
    if !card.core_facts.is_empty()
        || !card.recent_facts.is_empty()
        || !card.deprecated_facts.is_empty()
    {
        return true;
    }
    let extra_array_keys = [
        "coreFacts",
        "recentFacts",
        "preferences",
        "doNotDo",
        "commitments",
        "objections",
        "openLoops",
        "deprecatedFacts",
        "conflicts",
    ];
    if extra_array_keys.iter().any(|key| {
        card.extra
            .get_array(key)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
    }) {
        return true;
    }
    if doc_string(&card.extra, "recentEpisodeSummary").is_some() {
        return true;
    }
    let core_profile = card.extra.get_document("coreProfile").ok();
    if let Some(profile) = core_profile {
        return [
            "identity",
            "businessContext",
            "communicationStyle",
            "operationGoal",
        ]
        .iter()
        .any(|key| doc_string(profile, key).is_some());
    }
    false
}

/// task 6.3：从 [`Contact`] / [`OperatingMemory`] 推断"种子 memoryCard"，用于
/// 还没有实质 consolidator 输出的新联系人。返回 typed 形态，所有 free-form 字段
/// （coreProfile / relationshipState / preferences / commitments / openLoops /
/// doNotDo / objections / source / recentEpisodeSummary / conflicts /
/// deprecatedFacts）落 `extra` 兜底，与历史 wire shape 保持一致。
pub(crate) fn memory_card_from_contact(
    contact: &Contact,
    memory: &OperatingMemory,
) -> MemoryCardTyped {
    let profile: Option<&AgentProfile> = contact.agent_profile.as_ref();
    let identity = contact
        .human_profile_note
        .clone()
        .or_else(|| contact.memory_summary.clone())
        .or_else(|| profile.and_then(|item| non_empty_text(&item.summary)))
        .unwrap_or_default();
    let communication_style = profile
        .and_then(|item| non_empty_text(&item.communication_style))
        .or_else(|| doc_string(&contact.profile_attributes, "communicationStyle"))
        .unwrap_or_default();
    let operation_goal = profile
        .and_then(|item| non_empty_text(&item.operation_goal))
        .or_else(|| contact.follow_up_policy.clone())
        .unwrap_or_default();
    let business_context = doc_string(&contact.profile_attributes, "businessContext")
        .or_else(|| doc_string(&memory.user_understanding, "businessContext"))
        .unwrap_or_default();
    let mut core_facts: Vec<String> = Vec::new();
    push_unique_text(&mut core_facts, contact.memory_summary.as_deref());
    push_unique_text(&mut core_facts, contact.human_profile_note.as_deref());
    for tag in &contact.tags {
        if core_facts.len() >= 6 {
            break;
        }
        push_unique_text(&mut core_facts, Some(tag));
    }
    let mut preferences = Vec::new();
    push_unique_text(&mut preferences, Some(&communication_style));
    let mut commitments = Vec::new();
    push_unique_text(
        &mut commitments,
        contact.commitments.last().map(|c| c.text()),
    );
    let mut open_loops = Vec::new();
    push_unique_text(&mut open_loops, contact.follow_up_policy.as_deref());

    let mut extra = Document::new();
    extra.insert(
        "coreProfile",
        doc! {
            "identity": identity,
            "businessContext": business_context,
            "communicationStyle": communication_style,
            "operationGoal": operation_goal,
        },
    );
    extra.insert(
        "relationshipState",
        doc! {
            "stage": contact
                .domain_attributes
                .as_ref()
                .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()))
                .or_else(|| contact.operation_state.clone())
                .unwrap_or_else(|| "new_contact".to_string()),
            "trustLevel": doc_string(&memory.relationship_state, "trustLevel")
                .unwrap_or_else(|| "unknown".to_string()),
            "temperature": doc_string(&memory.relationship_state, "temperature")
                .unwrap_or_else(|| "unknown".to_string()),
            "lastEmotion": doc_string(&memory.relationship_state, "lastEmotion")
                .unwrap_or_default(),
        },
    );
    extra.insert("preferences", preferences);
    extra.insert(
        "doNotDo",
        string_array_from_doc(&memory.relationship_state, "doNotDo"),
    );
    extra.insert("commitments", commitments);
    extra.insert(
        "objections",
        string_array_from_doc(&memory.product_fit, "objections"),
    );
    extra.insert("openLoops", open_loops);
    extra.insert("recentEpisodeSummary", "");
    extra.insert("conflicts", Vec::<Document>::new());
    extra.insert("source", "contact_seed");

    MemoryCardTyped {
        core_facts: core_facts
            .into_iter()
            .map(MemoryFactRepr::Plain)
            .collect(),
        recent_facts: Vec::new(),
        deprecated_facts: Vec::new(),
        extra,
    }
}

fn push_unique_text(items: &mut Vec<String>, value: Option<&str>) {
    let Some(text) = value.map(str::trim).filter(|item| !item.is_empty()) else {
        return;
    };
    if !items.iter().any(|item| item == text) {
        items.push(text.to_string());
    }
}

fn non_empty_text(value: &str) -> Option<String> {
    let text = value
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn string_array_from_doc(doc: &Document, key: &str) -> Vec<String> {
    doc.get_array(key)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn compact_memory_card(card: &MemoryCardTyped) -> MemoryCardTyped {
    compact_memory_card_with_previous(card, None, &[])
}

/// 把 `card` 与可选的 `previous` coreFacts 合并，按 cap 截留各数组。
///
/// agent-autonomy-loop W5 task 6.3：本函数从 Document 入参 / 返回升级为
/// [`MemoryCardTyped`]。计算路径全部基于 typed 字段（`core_facts /
/// recent_facts / deprecated_facts`）+ `extra` 内的 free-form 数组（仍是
/// Document 形态，由 `extra` catch-all 兜底承接），写入路径通过
/// `bson::to_document(&MemoryCardTyped)` 一次性序列化，避免 typed/Document
/// 双轨表示。
///
/// 合并规则（HP-2 / Task 8）：
/// - `previous.core_facts` 中未在 `discarded` 列表里的事实会被保留到结果，
///   即使 `card.core_facts` 没显式列出它（避免新近性挤掉关键早期事实）。
/// - `card.core_facts` 优先靠前；previous 中独有的项追加到末尾再统一截留。
/// - 其它字段直接用 `card` 的值（recent_facts / preferences 等都属于
///   "consolidator 自己负责按重要度排序" 的范畴）。
/// - cap：`core_facts ≤ 6 / recent_facts ≤ 10 / deprecated_facts ≤ 20`，
///   `extra` 中数组类字段（`coreFacts / recentFacts` 历史字段，
///   `confirmedFacts / preferences / doNotDo / commitments / objections /
///   openLoops / openQuestions / conflicts`）也按 cap 截留。
pub fn compact_memory_card_with_previous(
    card: &MemoryCardTyped,
    previous: Option<&MemoryCardTyped>,
    discarded: &[String],
) -> MemoryCardTyped {
    let mut compact = card.clone();

    // discarded 是全局黑名单：无论 fact 来自 incoming card 还是上一版 previous，
    // 出现在 discarded 里就必须被排除。先把 card.core_facts 里命中的剔掉，再
    // 处理 previous 的合并保留（W5 / Task 6.8 PBT 不变量）。
    if !discarded.is_empty() {
        compact
            .core_facts
            .retain(|fact| !discarded.iter().any(|d| d == fact.as_text()));
    }

    if let Some(prev) = previous {
        for fact in &prev.core_facts {
            let fact_text = fact.as_text();
            if discarded.iter().any(|d| d == fact_text) {
                continue;
            }
            if !compact
                .core_facts
                .iter()
                .any(|item| item.as_text() == fact_text)
            {
                compact.core_facts.push(fact.clone());
            }
        }
    }

    // typed 字段 cap。
    compact.core_facts.truncate(6);
    compact.recent_facts.truncate(10);
    compact.deprecated_facts.truncate(20);

    // extra 中的 free-form 数组也按既有 cap 把关。历史 wire shape 保持不变：
    // 老数据可能在 extra.coreFacts / extra.recentFacts 里残留 String 数组（已
    // 通过 typed 字段反序列化吸收），同时 extra.preferences / .doNotDo 等是
    // 业务级数组，由 consolidator 输出后落到这里。task 6.3 把同一份 cap 表
    // 集中放到本函数，避免 typed 与 Document 两边各自维护。
    if previous.is_some() {
        // 处理 extra.coreFacts 历史路径：与 typed core_facts 合并（去重 + 未
        // discarded 保留），保持历史 BSON wire 兼容。
        let prev_extra_cores = previous
            .and_then(|p| p.extra.get_array("coreFacts").ok())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| b.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !prev_extra_cores.is_empty() {
            let mut merged: Vec<String> = compact
                .extra
                .get_array("coreFacts")
                .ok()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|b| b.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            // 同 typed core_facts：discarded 是全局黑名单，incoming 起点也要剔。
            merged.retain(|fact| !discarded.iter().any(|d| d == fact));
            for fact in prev_extra_cores {
                if discarded.iter().any(|d| d == &fact) {
                    continue;
                }
                if !merged.iter().any(|item| item == &fact) {
                    merged.push(fact);
                }
            }
            let merged_bson: Vec<Bson> = merged.into_iter().map(Bson::String).collect();
            compact.extra.insert("coreFacts", Bson::Array(merged_bson));
        }
    }

    limit_extra_array(&mut compact.extra, "coreFacts", 6);
    limit_extra_array(&mut compact.extra, "recentFacts", 10);
    limit_extra_array(&mut compact.extra, "confirmedFacts", 12);
    limit_extra_array(&mut compact.extra, "preferences", 8);
    limit_extra_array(&mut compact.extra, "doNotDo", 10);
    limit_extra_array(&mut compact.extra, "commitments", 8);
    limit_extra_array(&mut compact.extra, "objections", 8);
    limit_extra_array(&mut compact.extra, "openLoops", 8);
    limit_extra_array(&mut compact.extra, "openQuestions", 8);
    limit_extra_array(&mut compact.extra, "deprecatedFacts", 6);
    limit_extra_array(&mut compact.extra, "conflicts", 6);
    compact
}

fn limit_extra_array(doc: &mut Document, key: &str, max_items: usize) {
    if let Some(Bson::Array(items)) = doc.get_mut(key) {
        if items.len() > max_items {
            items.truncate(max_items);
        }
    }
}

/// agent-autonomy-loop W5 / Task 6.4：把 consolidator 输出的
/// `deprecatedFacts` / `conflicts` 应用到合并后的 [`MemoryCardTyped`]。
///
/// 行为对齐 R6.5 / R7.2 / R7.3 / R7.4 / R7.7：
///
/// * `deprecatedFacts`：按 id 在上一版 `core_facts` / `recent_facts` 找到原 fact，
///   保留其原 text / evidence / confidence / importance / source_message_ids /
///   source_run_id / created_at，附加 deprecated_at / deprecation_reason / updated_at；
///   id 找不到 → 不写入 + warning `deprecated_fact_id_not_found:<id>`；
/// * 同 id 同时出现在新 active + deprecated → warning
///   `fact_simultaneously_active_and_deprecated:<id>` + 仅保留 deprecated 集合；
/// * 非法 RFC3339 deprecatedAt → 回退 now + warning `invalid_deprecated_at:<id>:<raw>`；
/// * supersededBy 在新版查不到 → warning `superseded_by_id_not_found:<id>:<sup>`，
///   但 deprecated 仍写入；
/// * cap 20，按 deprecatedAt 升序 + id 字典序丢最旧。
///
/// 返回追加的 `warnings: Vec<String>`，由调用方写入
/// `agent_run_logs.memory_consolidator_warnings`。
pub(crate) fn apply_consolidator_deprecations(
    card: &mut MemoryCardTyped,
    previous: Option<&MemoryCardTyped>,
    consolidator_value: &serde_json::Value,
) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    let now = DateTime::now();

    let deprecated_entries = consolidator_value
        .get("deprecatedFacts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if deprecated_entries.is_empty() {
        return warnings;
    }

    // 索引新版 active facts 的 id 集合，用于检测同时出现在 active+deprecated。
    let active_ids: std::collections::HashSet<String> = card
        .core_facts
        .iter()
        .chain(card.recent_facts.iter())
        .filter_map(|fact_repr| match fact_repr {
            MemoryFactRepr::Structured(f) if !f.id.is_empty() => Some(f.id.clone()),
            _ => None,
        })
        .collect();

    let mut new_deprecated: Vec<MemoryFact> = Vec::new();

    for entry in deprecated_entries {
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            warnings.push("deprecated_fact_id_not_found:<empty>".to_string());
            continue;
        }
        let reason = entry
            .get("reason")
            .or_else(|| entry.get("deprecationReason"))
            .and_then(|v| v.as_str())
            .map(|s| {
                let mut s = s.to_string();
                s.truncate(200);
                s
            });
        let deprecated_at_raw = entry
            .get("deprecatedAt")
            .and_then(|v| v.as_str());
        let deprecated_at = match deprecated_at_raw {
            Some(raw) => match chrono::DateTime::parse_from_rfc3339(raw) {
                Ok(dt) => DateTime::from_millis(dt.timestamp_millis()),
                Err(_) => {
                    warnings.push(format!("invalid_deprecated_at:{id}:{raw}"));
                    now
                }
            },
            None => now,
        };
        // 在上一版查找原 fact。
        let original = previous.and_then(|prev| {
            prev.core_facts
                .iter()
                .chain(prev.recent_facts.iter())
                .find_map(|repr| match repr {
                    MemoryFactRepr::Structured(f) if f.id == id => Some(f.clone()),
                    _ => None,
                })
        });
        let Some(mut fact) = original else {
            warnings.push(format!("deprecated_fact_id_not_found:{id}"));
            continue;
        };
        fact.deprecated_at = Some(deprecated_at);
        fact.deprecation_reason = reason;
        fact.updated_at = now;
        // supersededBy 校验。
        if let Some(sup) = entry.get("supersededBy").and_then(|v| v.as_str()) {
            if !active_ids.contains(sup) {
                warnings.push(format!("superseded_by_id_not_found:{id}:{sup}"));
            }
        }
        if active_ids.contains(&id) {
            warnings.push(format!("fact_simultaneously_active_and_deprecated:{id}"));
            // 仅 deprecated 集合保留：从 active 集合移除同 id。
            card.core_facts.retain(|repr| match repr {
                MemoryFactRepr::Structured(f) => f.id != id,
                _ => true,
            });
            card.recent_facts.retain(|repr| match repr {
                MemoryFactRepr::Structured(f) => f.id != id,
                _ => true,
            });
        }
        new_deprecated.push(fact);
    }

    // 合并到现有 deprecated_facts（保留旧条目），按 deprecated_at 升序 + id 排序，
    // cap=20 丢最旧。
    let mut combined: Vec<MemoryFact> = card
        .deprecated_facts
        .iter()
        .filter_map(|repr| match repr {
            MemoryFactRepr::Structured(f) => Some(f.clone()),
            MemoryFactRepr::Plain(_) => None,
        })
        .collect();
    combined.extend(new_deprecated);
    combined.sort_by(|a, b| {
        let a_at = a.deprecated_at.map(|d| d.timestamp_millis()).unwrap_or(0);
        let b_at = b.deprecated_at.map(|d| d.timestamp_millis()).unwrap_or(0);
        a_at.cmp(&b_at).then_with(|| a.id.cmp(&b.id))
    });
    if combined.len() > 20 {
        let drop = combined.len() - 20;
        combined.drain(0..drop);
    }
    card.deprecated_facts = combined.into_iter().map(MemoryFactRepr::Structured).collect();

    warnings
}

/// task 6.3 deprecated alias：保持 [`compact_memory_card_typed`] 名字以兼容
/// 既有 PBT / 测试调用方；语义即 [`compact_memory_card_with_previous`]。
#[deprecated(
    note = "task 6.3：直接使用 compact_memory_card_with_previous，本函数仅作向后兼容别名。"
)]
pub fn compact_memory_card_typed(
    card: &MemoryCardTyped,
    previous: Option<&MemoryCardTyped>,
    discarded: &[String],
) -> MemoryCardTyped {
    compact_memory_card_with_previous(card, previous, discarded)
}

pub(crate) fn next_memory_card_version(memory: &OperatingMemory) -> i32 {
    memory.memory_card_version.saturating_add(1)
}

pub(crate) async fn load_or_create_operating_memory(
    state: &AppState,
    contact: &Contact,
) -> AppResult<OperatingMemory> {
    if let Some(mut memory) = state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await?
    {
        if !memory_card_has_signal(&effective_memory_card(&memory)) {
            let seeded = memory_card_from_contact(contact, &memory);
            if memory_card_has_signal(&seeded) {
                let updated_at = DateTime::now();
                // task 6.3：typed-only 路径。compact 在 typed 域完成，
                // `extra.version` 注入后通过 `bson::to_document` 一次性序列化
                // 落库；不再保留 typed/Document 双轨表示。
                let mut compact = compact_memory_card(&seeded);
                memory.memory_card_version = next_memory_card_version(&memory);
                compact
                    .extra
                    .insert("version", memory.memory_card_version);
                let compact_doc = to_document(&compact).unwrap_or_default();
                memory.memory_card = compact;
                memory.memory_card_updated_at = Some(updated_at);
                state
                    .db
                    .operating_memories()
                    .update_one(
                        doc! {
                            "workspace_id": &contact.workspace_id,
                            "account_id": &contact.account_id,
                            "contact_wxid": &contact.wxid
                        },
                        doc! {
                            "$set": {
                                "memory_card": compact_doc,
                                "memory_card_version": memory.memory_card_version,
                                "memory_card_updated_at": updated_at,
                                "updated_at": updated_at
                            }
                        },
                        None,
                    )
                    .await?;
            }
        }
        return Ok(memory);
    }
    let mut memory = OperatingMemory {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        user_understanding: doc! {
            "facts": Vec::<String>::new(),
            "signals": Vec::<String>::new(),
            "hypotheses": Vec::<Document>::new(),
            "unknowns": Vec::<String>::new(),
            "changes": Vec::<String>::new(),
            "identity": "",
            "businessContext": "",
            "decisionStyle": "",
            "communicationPreference": "",
            "sensitivePoints": Vec::<String>::new()
        },
        relationship_state: doc! {
            "trustLevel": "unknown",
            "temperature": "unknown",
            "lastEmotion": "",
            "relationshipGoal": "",
            "doNotDo": Vec::<String>::new()
        },
        product_fit: doc! {
            "painPoints": Vec::<String>::new(),
            "interestedProducts": Vec::<String>::new(),
            "fitReasons": Vec::<String>::new(),
            "objections": Vec::<String>::new(),
            "notFitReasons": Vec::<String>::new(),
            "safeClaimsUsed": Vec::<String>::new(),
            "riskPoints": Vec::<String>::new(),
            "unknowns": Vec::<String>::new()
        },
        next_action: doc! {
            "currentState": contact.operation_state.clone().unwrap_or_else(|| "new_contact".to_string()),
            "nextBestAction": "",
            "goal": "",
            "recommendedMove": "",
            "avoid": "",
            "timing": "",
            "reason": ""
        },
        context_pack: default_context_pack(),
        context_pack_version: 0,
        context_pack_updated_at: None,
        // task 6.3：直接以 typed 默认值落入；不再走 `Document → from_document`
        // 的中转兼容路径。
        memory_card: default_memory_card(),
        memory_card_version: 0,
        memory_card_updated_at: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let mut seeded = memory_card_from_contact(contact, &memory);
    let has_signal = memory_card_has_signal(&seeded);
    memory.memory_card_version = if has_signal { 1 } else { 0 };
    seeded.extra.insert("version", memory.memory_card_version);
    memory.memory_card = seeded;
    memory.memory_card_updated_at = if memory.memory_card_version > 0 {
        Some(DateTime::now())
    } else {
        None
    };
    state
        .db
        .operating_memories()
        .insert_one(&memory, None)
        .await?;
    state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::External("operating memory missing after insert".to_string()))
}

pub async fn handle_memory_consolidation_task(state: &AppState, task: AgentTask) -> AppResult<()> {
    let Some(task_id) = task.id else {
        return Ok(());
    };
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "wxid": &task.contact_wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("memory consolidation contact not found".to_string()))?;
    consolidate_contact_memory(state, &contact, Some(task_id)).await
}

pub async fn consolidate_contact_memory(
    state: &AppState,
    contact: &Contact,
    task_id: Option<ObjectId>,
) -> AppResult<()> {
    // 波 C3：从 OperationDomainConfig.runtime_parameters 读 run_token_budget /
    // run_max_llm_calls，避免硬编码 60000/4 让运营策略页的预算控件形同虚设。
    let domain_config =
        super::decision::load_user_operation_domain_config(state, &contact.workspace_id).await?;
    let runtime = super::runtime::UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    let run_id = uuid::Uuid::new_v4().to_string();
    let budget = std::sync::Arc::new(RunBudget::new(
        run_id.clone(),
        runtime.run_token_budget,
        runtime.run_max_llm_calls,
        runtime.knowledge_max_tool_calls,
    ));
    RUN_BUDGET
        .scope(
            budget,
            consolidate_contact_memory_inner(state, contact, task_id, run_id),
        )
        .await
}

async fn consolidate_contact_memory_inner(
    state: &AppState,
    contact: &Contact,
    task_id: Option<ObjectId>,
    run_id: String,
) -> AppResult<()> {
    let memory = load_or_create_operating_memory(state, contact).await?;
    let mut cursor = state
        .db
        .memory_candidates()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "status": "pending"
            },
            FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .limit(30)
                .build(),
        )
        .await?;
    let mut candidate_ids = Vec::new();
    let mut candidates = Vec::new();
    while let Some(candidate) = cursor.try_next().await? {
        if let Some(id) = candidate.id {
            candidate_ids.push(id);
        }
        candidates.push(to_document(&candidate).unwrap_or_default());
    }
    if candidates.is_empty() {
        if let Some(task_id) = task_id {
            state
                .db
                .tasks()
                .update_one(
                    doc! { "_id": task_id },
                    doc! { "$set": { "status": "sent", "gateway_status": "no_candidates", "updated_at": DateTime::now() } },
                    None,
                )
                .await?;
        }
        return Ok(());
    }
    let system = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.memory_consolidator.system",
    )
    .await
    .unwrap_or_else(|_| {
        "你是用户运营长期记忆整理 Agent。只输出严格 JSON，不输出 markdown。".to_string()
    });
    let task_prompt = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.memory_consolidator.task",
    )
    .await
    .unwrap_or_else(|_| {
        r#"请输出 JSON：{ "memoryCard": {}, "summary": "", "discarded": [] }。只保留影响未来运营决策的信息，合并重复，最新明确表达优先，所有数组必须克制。"#.to_string()
    });
    let user = format!(
        r#"{}

当前 memoryCard:
{}

候选记忆:
{}

客户昵称: {}
客户阶段: {}
意向等级: {}
"#,
        task_prompt,
        // task 6.3：prompt wire shape 仍是 Document JSON；在最末端通过
        // `to_document()` 一次性把 typed memoryCard 转成 BSON Document 再
        // 序列化为 JSON，避免 typed 与 Document 双轨并存。
        serde_json::to_string(&effective_memory_card(&memory).to_document()).unwrap_or_default(),
        serde_json::to_string(&candidates).unwrap_or_default(),
        contact.nickname.clone().unwrap_or_default(),
        contact
            .domain_attributes
            .as_ref()
            .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()))
            .unwrap_or_default(),
        contact
            .domain_attributes
            .as_ref()
            .and_then(|d| d.get_str("intent_level").ok().map(|s| s.to_string()))
            .unwrap_or_default()
    );
    let value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        Some(&run_id),
        "user.memory_consolidator.task",
        &system,
        &user,
    )
    .await?;
    // task 6.3：consolidator 输出的 memoryCard 是 JSON Document，先经
    // `MemoryCardTyped::from_document` 解析为 typed，再走 typed compact 合并；
    // 写入路径 `bson::to_document(&MemoryCardTyped)` 一次性序列化，不保留
    // 两套并行表示。
    let card_doc = value
        .get("memoryCard")
        .or_else(|| value.get("memory_card"))
        .and_then(|item| to_document(item).ok())
        .or_else(|| to_document(&value).ok())
        .unwrap_or_default();
    let card_typed = if card_doc.is_empty() {
        default_memory_card()
    } else {
        MemoryCardTyped::from_document(&card_doc)
    };
    // agent-autonomy-loop W5 / Task 6.7：consolidator LLM 偶发只回 `Vec<String>`
    // 形态的 coreFacts / recentFacts；统一在反序列化边界升级为结构化，并把
    // `memory_facts_auto_upgraded` 写入 consolidator_warnings。后续 R11 sunset
    // 后此路径直接返回 400 / 拒收，由 caller 端契约保证 Structured 形态。
    let mut card_typed = card_typed;
    let auto_upgraded = card_typed.auto_upgrade_plain_facts();
    // HP-2 / Task 8：consolidator 输出与上一份 memoryCard 合并，
    // 未被显式 discarded 的 coreFacts 不会因为新近性被挤出。
    let discarded_list: Vec<String> = value
        .get("discarded")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let previous_card = effective_memory_card(&memory);
    let mut compact =
        compact_memory_card_with_previous(&card_typed, Some(&previous_card), &discarded_list);
    // agent-autonomy-loop W5 / Task 6.4：把 consolidator 输出的 deprecatedFacts /
    // conflicts 应用到合并后的 typed card；warnings 写入 agent_run_logs。
    let mut consolidator_warnings =
        apply_consolidator_deprecations(&mut compact, Some(&previous_card), &value);
    if auto_upgraded > 0 {
        // Task 6.7：把"老 Vec<String> 形态被自动升级"作为可观测信号写入审计。
        // 数量也带出来，方便 sunset 灰度期度量曲线。
        consolidator_warnings.push(format!("memory_facts_auto_upgraded:{auto_upgraded}"));
    }
    if !consolidator_warnings.is_empty() {
        // 落审计：把 warnings 写入 agent_run_logs.memory_consolidator_warnings。
        let _ = state
            .db
            .agent_run_logs()
            .clone_with_type::<Document>()
            .update_one(
                doc! { "run_id": &run_id },
                doc! {
                    "$set": {
                        "memory_consolidator_warnings": consolidator_warnings.clone(),
                    }
                },
                None,
            )
            .await;
    }
    // agent-autonomy-loop W5 / Task 6.5：conflicts[].winner != "none" 时
    // 为每条写 agent_events kind="memory_conflict_resolved"。
    if let Some(conflicts) = value.get("conflicts").and_then(|v| v.as_array()) {
        for conflict in conflicts {
            let winner = conflict.get("winner").and_then(|v| v.as_str()).unwrap_or("");
            if winner.is_empty() || winner == "none" {
                continue;
            }
            let a_id = conflict.get("aId").and_then(|v| v.as_str()).unwrap_or("");
            let b_id = conflict.get("bId").and_then(|v| v.as_str()).unwrap_or("");
            let a_text = conflict.get("aText").and_then(|v| v.as_str()).unwrap_or("");
            let b_text = conflict.get("bText").and_then(|v| v.as_str()).unwrap_or("");
            let resolution = conflict
                .get("resolution")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let _ = write_event_for_account(
                state,
                &contact.account_id,
                Some(&contact.wxid),
                "memory_conflict_resolved",
                "info",
                "consolidator 解决了一组事实冲突",
                Some(doc! {
                    "a_id": a_id,
                    "b_id": b_id,
                    "winner": winner,
                    "resolution": resolution,
                    "a_text": a_text,
                    "b_text": b_text,
                }),
            )
            .await;
        }
    }
    let next_version = next_memory_card_version(&memory);
    compact.extra.insert("version", next_version);
    compact
        .extra
        .insert("source", "memory_consolidator_agent");
    let compact_doc = to_document(&compact).unwrap_or_default();
    state
        .db
        .operating_memories()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            doc! {
                "$set": {
                    "memory_card": compact_doc,
                    "memory_card_version": next_version,
                    "memory_card_updated_at": DateTime::now(),
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    if !candidate_ids.is_empty() {
        state
            .db
            .memory_candidates()
            .update_many(
                doc! { "_id": { "$in": candidate_ids } },
                doc! { "$set": { "status": "consolidated", "updated_at": DateTime::now() } },
                None,
            )
            .await?;
    }
    write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "memory_consolidated",
        "success",
        "长期记忆卡片已整理",
        Some(doc! {
            "runId": run_id,
            "summary": value.get("summary").and_then(|item| item.as_str()).unwrap_or_default(),
            "discarded": to_bson(value.get("discarded").unwrap_or(&json!([]))).unwrap_or(Bson::Array(Vec::new())),
            "candidateCount": candidates.len() as i32,
            "memoryCardVersion": next_version,
        }),
    )
    .await?;
    if let Some(task_id) = task_id {
        state
            .db
            .tasks()
            .update_one(
                doc! { "_id": task_id },
                doc! { "$set": { "status": "sent", "gateway_status": "consolidated", "updated_at": DateTime::now() } },
                None,
            )
            .await?;
    }
    Ok(())
}

pub(crate) async fn write_memory_candidates(
    state: &AppState,
    contact: &Contact,
    decision: &AgentDecision,
    run_id: &str,
) -> AppResult<()> {
    if decision.memory_candidates.is_empty() && decision.operating_memory_update.is_empty() {
        return Ok(());
    }
    let raw_candidates = if decision.memory_candidates.is_empty() {
        vec![decision.operating_memory_update.clone()]
    } else {
        decision.memory_candidates.clone()
    };
    let candidates = raw_candidates
        .into_iter()
        .filter_map(validated_memory_candidate)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(());
    }
    state
        .db
        .memory_candidates()
        .insert_one(
            MemoryCandidate {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                run_id: Some(run_id.to_string()),
                source: decision.run_mode.clone(),
                candidates,
                memory_write_score: decision.memory_write_score,
                status: if decision.memory_write_score >= 6 {
                    "pending".to_string()
                } else {
                    "ignored_low_score".to_string()
                },
                reason: Some(decision.memory_update.clone()),
                created_at: DateTime::now(),
                updated_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(())
}

fn validated_memory_candidate(candidate: Document) -> Option<Document> {
    let candidate_type = doc_string(&candidate, "type")?;
    let content = doc_string(&candidate, "content")?;
    let evidence = doc_string(&candidate, "evidence")?;
    let importance = doc_i32(Some(&candidate), "importance", 0).clamp(0, 10);
    let confidence = doc_i32(Some(&candidate), "confidence", 0).clamp(0, 10);
    if importance == 0 || confidence == 0 {
        return None;
    }
    Some(doc! {
        "type": candidate_type,
        "content": content,
        "evidence": evidence,
        "importance": importance,
        "confidence": confidence
    })
}

pub(crate) async fn schedule_memory_consolidation_task(
    state: &AppState,
    contact: &Contact,
    run_id: &str,
) -> AppResult<()> {
    let pending = state
        .db
        .tasks()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": "memory_consolidation",
                "status": { "$in": ["pending", "retry", "running"] }
            },
            None,
        )
        .await?;
    if pending.is_some() {
        return Ok(());
    }
    state
        .db
        .tasks()
        .insert_one(
            AgentTask {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                kind: "memory_consolidation".to_string(),
                run_at: DateTime::now(),
                expires_at: None,
                content: format!("整理候选记忆 runId={run_id}"),
                status: "pending".to_string(),
                source_decision_id: None,
                review_required: false,
                attempt_count: 0,
                max_attempts: 3,
                next_retry_at: None,
                gateway_status: None,
                cancel_reason: None,
                error: None,
                claimed_at: None,
                claim_recovery_count: 0,
                created_at: DateTime::now(),
                updated_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(())
}


/// knowledge-digest-workstation Phase 5：加载运营长期偏好记忆。
///
/// 与 `consolidate_contact_memory` / `compact_memory_card_*` 物理隔离 —
/// 这些函数都只触达 `contacts.memory_card`；本函数只触达
/// `knowledge_operator_memory` collection。两者**禁止**互相读写。
///
/// 行为：按 `accountId + operatorId` 取最近 `top_n` 条非过期记忆，
/// 按 `lastUsedAt desc` 排序；命中时把这些记忆的 `lastUsedAt`
/// 一次性 bump 为 now（运营重新拿出来用过 = 续期）。
///
/// 返回的 Vec 已按 `lastUsedAt desc` 排好，调用方拼 prompt header 时
/// 直接渲染即可。
pub(crate) async fn load_operator_memory(
    db: &crate::db::Database,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    top_n: i64,
) -> AppResult<Vec<crate::models::KnowledgeOperatorMemory>> {
    use futures::TryStreamExt;
    let now = DateTime::now();
    let filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "operator_id": operator_id,
        "$or": [
            { "expires_at": { "$exists": false } },
            { "expires_at": null },
            { "expires_at": { "$gt": now } },
        ],
    };
    let opts = FindOptions::builder()
        .sort(doc! { "last_used_at": -1_i32 })
        .limit(top_n.max(1))
        .build();
    let mut cursor = db
        .knowledge_operator_memory()
        .find(filter, opts)
        .await
        .map_err(|e| AppError::External(format!("加载运营记忆失败：{e}")))?;
    let mut out = Vec::new();
    while let Some(m) = cursor
        .try_next()
        .await
        .map_err(|e| AppError::External(format!("迭代运营记忆失败：{e}")))?
    {
        out.push(m);
    }
    if !out.is_empty() {
        let ids: Vec<ObjectId> = out.iter().filter_map(|m| m.id).collect();
        if !ids.is_empty() {
            let _ = db
                .knowledge_operator_memory()
                .update_many(
                    doc! { "_id": { "$in": ids } },
                    doc! { "$set": { "last_used_at": now } },
                    None,
                )
                .await;
        }
    }
    Ok(out)
}

/// Phase A2：把 `load_operator_memory` 返回的偏好记忆渲染成可注入 reply prompt 的文本段。
///
/// 输出按 `kind`（preference / rejection / context）分组，空输入返回空串。
/// 调用方在 reply Agent 装配 prompt 时拼接。
#[allow(dead_code)] // Phase A helper，生产路径接入待 follow-up
pub(crate) fn format_operator_memory_for_reply_prompt(
    items: &[crate::models::KnowledgeOperatorMemory],
) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut buf = String::from("[运营偏好记忆]\n");
    for m in items {
        buf.push_str(&format!("- ({}) {}\n", m.kind, m.content));
    }
    buf
}

/// knowledge-digest-workstation Phase 5：写入运营长期偏好记忆。
///
/// 同 `(workspace_id, account_id, operator_id, kind, content)` 命中时只
/// bump `lastUsedAt`，不重复插入，避免运营把同一句话说两遍就刷出两条
/// 重复 memory。
pub(crate) async fn record_operator_memory(
    db: &crate::db::Database,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    kind: &str,
    content: &str,
) -> AppResult<crate::models::KnowledgeOperatorMemory> {
    let kind_trim = kind.trim();
    let content_trim = content.trim();
    if !["preference", "rejection", "context"].contains(&kind_trim) {
        return Err(AppError::BadRequest(format!(
            "memoryKind 非法：{kind}（必须在 [preference, rejection, context]）"
        )));
    }
    if content_trim.is_empty() {
        return Err(AppError::BadRequest(
            "memoryContent 为空，无法落库".to_string(),
        ));
    }
    let now = DateTime::now();
    let filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "operator_id": operator_id,
        "kind": kind_trim,
        "content": content_trim,
    };
    if let Some(existing) = db
        .knowledge_operator_memory()
        .find_one(filter.clone(), None)
        .await
        .map_err(|e| AppError::External(format!("查询运营记忆失败：{e}")))?
    {
        let _ = db
            .knowledge_operator_memory()
            .update_one(
                doc! { "_id": existing.id.expect("existing id") },
                doc! { "$set": { "last_used_at": now } },
                None,
            )
            .await;
        let mut bumped = existing;
        bumped.last_used_at = now;
        return Ok(bumped);
    }
    let mem = crate::models::KnowledgeOperatorMemory {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        operator_id: operator_id.to_string(),
        kind: kind_trim.to_string(),
        content: content_trim.to_string(),
        created_at: now,
        last_used_at: now,
        expires_at: None,
    };
    db.knowledge_operator_memory()
        .insert_one(&mem, None)
        .await
        .map_err(|e| AppError::External(format!("写入运营记忆失败：{e}")))?;
    Ok(mem)
}



#[cfg(test)]
mod r7_deprecation_tests {
    //! 覆盖 design.md §3.5 / R7.2 / R7.3 / R7.4 / R7.7 行为：
    //! 1. consolidator 输出 deprecatedFacts 命中上一版 fact → 新版
    //!    deprecated_facts 含 id==X && deprecation_reason==Y && deprecated_at==T；
    //! 2. id 找不到 → warning fallback、不写 deprecatedFacts；
    //! 3. 同 id 同时 active+deprecated → warning + 仅 deprecated 集合保留；
    //! 4. 改写场景：新 fact text 与上一版 X 不同但 id 相同 → 视为改写直接覆盖、
    //!    不进 deprecatedFacts。

    use super::apply_consolidator_deprecations;
    use crate::models::{MemoryCardTyped, MemoryFact, MemoryFactRepr};
    use mongodb::bson::DateTime;
    use serde_json::json;

    fn fact(id: &str, text: &str) -> MemoryFact {
        MemoryFact {
            id: id.to_string(),
            text: text.to_string(),
            evidence: None,
            confidence: 7,
            importance: 5,
            may_expire: false,
            deprecated_at: None,
            deprecation_reason: None,
            source_message_ids: vec![],
            source_run_id: None,
            created_at: DateTime::from_millis(0),
            updated_at: DateTime::from_millis(0),
            extra: Default::default(),
        }
    }

    #[test]
    fn deprecation_id_matches_previous_fact() {
        // R7.2 / R7.3：consolidator 输出 deprecatedFacts: [{id:X, reason:Y, deprecatedAt:T}]
        // → 新版 deprecated_facts 含 id==X && deprecation_reason==Some(Y)。
        let prev = MemoryCardTyped {
            core_facts: vec![MemoryFactRepr::Structured(fact("id-1", "原始 fact"))],
            ..Default::default()
        };
        let mut new_card = MemoryCardTyped::default();
        let consolidator = json!({
            "deprecatedFacts": [
                { "id": "id-1", "reason": "用户已澄清不再需要", "deprecatedAt": "2026-05-01T00:00:00Z" }
            ]
        });
        let warnings = apply_consolidator_deprecations(&mut new_card, Some(&prev), &consolidator);
        assert!(warnings.is_empty(), "正常路径不应产生 warnings: {warnings:?}");
        assert_eq!(new_card.deprecated_facts.len(), 1);
        match &new_card.deprecated_facts[0] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.id, "id-1");
                assert_eq!(f.text, "原始 fact");
                assert_eq!(f.deprecation_reason.as_deref(), Some("用户已澄清不再需要"));
                assert!(f.deprecated_at.is_some());
            }
            _ => panic!("expected Structured"),
        }
    }

    #[test]
    fn deprecation_id_not_found_emits_warning_and_skips() {
        // R7.4：id 找不到 → warning + 不写 deprecatedFacts。
        let prev = MemoryCardTyped {
            core_facts: vec![MemoryFactRepr::Structured(fact("id-known", "known"))],
            ..Default::default()
        };
        let mut new_card = MemoryCardTyped::default();
        let consolidator = json!({
            "deprecatedFacts": [
                { "id": "id-unknown", "reason": "test" }
            ]
        });
        let warnings = apply_consolidator_deprecations(&mut new_card, Some(&prev), &consolidator);
        assert!(warnings
            .iter()
            .any(|w| w == "deprecated_fact_id_not_found:id-unknown"));
        assert!(new_card.deprecated_facts.is_empty(), "id 找不到时不应写入");
    }

    #[test]
    fn fact_simultaneously_active_and_deprecated_emits_warning_and_keeps_only_deprecated() {
        // R7.7：同 id 同时出现在 active + deprecated → warning + 仅 deprecated 保留。
        let prev = MemoryCardTyped {
            core_facts: vec![MemoryFactRepr::Structured(fact("id-2", "原始"))],
            ..Default::default()
        };
        let mut new_card = MemoryCardTyped {
            core_facts: vec![MemoryFactRepr::Structured(fact("id-2", "新版还有它"))],
            ..Default::default()
        };
        let consolidator = json!({
            "deprecatedFacts": [
                { "id": "id-2", "reason": "矛盾测试" }
            ]
        });
        let warnings = apply_consolidator_deprecations(&mut new_card, Some(&prev), &consolidator);
        assert!(warnings
            .iter()
            .any(|w| w == "fact_simultaneously_active_and_deprecated:id-2"));
        // active 集合中 id-2 被移除。
        assert!(new_card
            .core_facts
            .iter()
            .all(|repr| match repr {
                MemoryFactRepr::Structured(f) => f.id != "id-2",
                _ => true,
            }));
        // deprecated 集合中 id-2 存在。
        assert!(new_card.deprecated_facts.iter().any(|repr| match repr {
            MemoryFactRepr::Structured(f) => f.id == "id-2",
            _ => false,
        }));
    }

    #[test]
    fn invalid_deprecated_at_falls_back_to_now_with_warning() {
        // R7.7：非法 RFC3339 deprecatedAt → 回退 now + warning。
        let prev = MemoryCardTyped {
            core_facts: vec![MemoryFactRepr::Structured(fact("id-3", "x"))],
            ..Default::default()
        };
        let mut new_card = MemoryCardTyped::default();
        let consolidator = json!({
            "deprecatedFacts": [
                { "id": "id-3", "reason": "r", "deprecatedAt": "not-a-date" }
            ]
        });
        let warnings = apply_consolidator_deprecations(&mut new_card, Some(&prev), &consolidator);
        assert!(warnings
            .iter()
            .any(|w| w == "invalid_deprecated_at:id-3:not-a-date"));
        // 仍然写入 deprecated（time 用 now 兜底）。
        assert_eq!(new_card.deprecated_facts.len(), 1);
    }

    #[test]
    fn deprecated_facts_capped_at_twenty() {
        // cap=20 + 按 deprecatedAt 升序丢最旧。
        let mut prev = MemoryCardTyped::default();
        for i in 0..30 {
            prev.core_facts
                .push(MemoryFactRepr::Structured(fact(&format!("id-{i}"), "f")));
        }
        let mut new_card = MemoryCardTyped::default();
        let mut deprecated = Vec::new();
        for i in 0..30 {
            deprecated.push(json!({
                "id": format!("id-{i}"),
                "reason": "r",
            }));
        }
        let consolidator = json!({ "deprecatedFacts": deprecated });
        let warnings = apply_consolidator_deprecations(&mut new_card, Some(&prev), &consolidator);
        assert!(warnings.is_empty());
        assert_eq!(
            new_card.deprecated_facts.len(),
            20,
            "deprecated_facts 必须 cap 在 20"
        );
    }
}


// ── P5 性质测试（agent-autonomy-loop W5 / Task 6.10：≥ 64 用例）─────────
//
// **Property 5: 记忆冲突可追溯**
// **Validates: Requirements 6.3, 7.2, 7.4**
//
// 性质：随机生成 (previous core_facts, consolidator deprecatedFacts) →
// 1. 凡是命中前一版的 deprecatedFacts.id 必出现在新版 deprecated_facts；
// 2. 同一 id 不能既出现在 active 又出现在 deprecated；
// 3. stable id 沿用（fact 文本 / id 都从前一版透传）。

#[cfg(test)]
mod p5_pbt {
    use super::apply_consolidator_deprecations;
    use crate::models::{MemoryCardTyped, MemoryFact, MemoryFactRepr};
    use mongodb::bson::DateTime;
    use proptest::prelude::*;
    use serde_json::json;

    fn fact_with(id: &str, text: &str) -> MemoryFact {
        MemoryFact {
            id: id.to_string(),
            text: text.to_string(),
            confidence: 7,
            importance: 5,
            created_at: DateTime::from_millis(0),
            updated_at: DateTime::from_millis(0),
            ..Default::default()
        }
    }

    fn arbitrary_id() -> impl Strategy<Value = String> {
        "[a-z]{1,8}-[0-9]{1,4}".prop_map(String::from)
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 80,
            ..ProptestConfig::default()
        })]

        /// P5：deprecation 集合不变量。
        #[test]
        fn p5_deprecation_invariants(
            prev_ids in proptest::collection::vec(arbitrary_id(), 1..=10),
            depr_count in 0usize..=10usize,
        ) {
            // dedupe + 取前 depr_count 个作为本次要 deprecate 的 id 集合。
            let mut prev_ids = prev_ids.clone();
            prev_ids.sort();
            prev_ids.dedup();
            prop_assume!(!prev_ids.is_empty());
            let to_deprecate: Vec<String> = prev_ids.iter().take(depr_count).cloned().collect();

            let prev = MemoryCardTyped {
                core_facts: prev_ids
                    .iter()
                    .map(|id| MemoryFactRepr::Structured(fact_with(id, &format!("text-{id}"))))
                    .collect(),
                ..Default::default()
            };
            let mut new_card = MemoryCardTyped::default();
            let consolidator = json!({
                "deprecatedFacts": to_deprecate
                    .iter()
                    .map(|id| json!({ "id": id, "reason": "test" }))
                    .collect::<Vec<_>>(),
            });
            let _warnings = apply_consolidator_deprecations(
                &mut new_card,
                Some(&prev),
                &consolidator,
            );

            // 性质 1：所有 to_deprecate id 都在 new_card.deprecated_facts 中。
            for id in &to_deprecate {
                let found = new_card
                    .deprecated_facts
                    .iter()
                    .any(|repr| match repr {
                        MemoryFactRepr::Structured(f) => f.id == *id,
                        _ => false,
                    });
                prop_assert!(found, "deprecated id={id} 未出现在 deprecated_facts");
            }

            // 性质 2：active 集合（new_card.core_facts / recent_facts）不应包含
            //         同时 deprecated 的 id（new_card 起步空，所以这里为零项，
            //         任何"既 active 又 deprecated"会被 apply 函数移除）。
            let active_ids: Vec<String> = new_card
                .core_facts
                .iter()
                .chain(new_card.recent_facts.iter())
                .filter_map(|repr| match repr {
                    MemoryFactRepr::Structured(f) => Some(f.id.clone()),
                    _ => None,
                })
                .collect();
            for id in &to_deprecate {
                prop_assert!(!active_ids.contains(id),
                    "id={id} 不应同时出现在 active 与 deprecated");
            }

            // 性质 3：deprecated_facts 中每个 fact 的 text 沿用前一版（stable id）。
            for repr in &new_card.deprecated_facts {
                if let MemoryFactRepr::Structured(f) = repr {
                    if to_deprecate.contains(&f.id) {
                        prop_assert_eq!(&f.text, &format!("text-{}", f.id),
                            "deprecated fact text 应沿用前一版");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod a6_tests {
    use super::*;
    use crate::models::KnowledgeOperatorMemory;
    use mongodb::bson::DateTime;

    fn mk(kind: &str, content: &str) -> KnowledgeOperatorMemory {
        KnowledgeOperatorMemory {
            id: None,
            workspace_id: "ws_default".to_string(),
            account_id: "acct-1".to_string(),
            operator_id: "op-1".to_string(),
            kind: kind.to_string(),
            content: content.to_string(),
            created_at: DateTime::from_millis(0),
            last_used_at: DateTime::from_millis(0),
            expires_at: None,
        }
    }

    /// Phase A6: `operator_memory_loaded_in_decision`
    /// 验证：当 `load_operator_memory` 返回非空列表时，`format_operator_memory_for_reply_prompt`
    /// 能把它渲染为可拼到 reply prompt 的文本段——即"决策装配 prompt 的边界处会真正吃到记忆"。
    #[test]
    fn operator_memory_loaded_in_decision() {
        let memories = vec![
            mk("preference", "默认用 'xx' 称呼客户"),
            mk("rejection", "不要发优惠券模板"),
            mk("context", "客户偏好下午沟通"),
        ];
        let segment = format_operator_memory_for_reply_prompt(&memories);
        assert!(segment.contains("[运营偏好记忆]"), "应渲染段头");
        assert!(segment.contains("(preference) 默认用 'xx' 称呼客户"));
        assert!(segment.contains("(rejection) 不要发优惠券模板"));
        assert!(segment.contains("(context) 客户偏好下午沟通"));
    }

    #[test]
    fn operator_memory_empty_yields_empty_segment() {
        let segment = format_operator_memory_for_reply_prompt(&[]);
        assert!(segment.is_empty());
    }
}
