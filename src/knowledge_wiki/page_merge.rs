//! `page_merge` —— 写入路径的纯函数预校验层。
//!
//! 借鉴 `nashsu/llm_wiki` 的 `page-merge.ts` / `enrich-wikilinks.ts`，把所有
//! "LLM 倾向于把整页/整 chunk 重写一遍"的失败模式拦在写库之前：
//!
//! 1. **数组字段 union（应用层）** — 永远是 `existing ∪ patch`，不让 LLM 决定
//!    "丢哪几个 tag"；
//! 2. **锁定字段守门** — `_id / workspace_id / account_id / document_id / item_id /
//!    wiki_type / chunk_type / created_at` 等真实身份字段不允许 patch 携带，
//!    携带即拒收；
//! 3. **70% body 长度阈值** — patch 后 body 长度低于既有 70% → 大概率 LLM
//!    截断/偷懒，拒收；
//! 4. **canonical chunk hash** — sha256 已规范化的 BSON（剔除 volatile 字段：
//!    `updated_at` / `usage_stats` / `dynamic_confidence` / `_id`），用于
//!    `chunk_revisions.before_hash / after_hash`。
//!
//! 设计原则：
//! - **0% LLM 参与** —— 5 个导出全是同步纯函数；
//! - **0 副作用** —— 不读写数据库、不发起网络请求、不依赖时间，便于 PBT；
//! - **0 新依赖** —— 复用既有 `mongodb::bson` + `sha2` + `serde_json`。

use std::collections::BTreeSet;

use mongodb::bson::{Bson, Document};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// 默认锁定字段集合 —— `apply_chunk_revision` 的 patch 入参不允许命中其中任一。
///
/// 列表语义：
/// - `_id` / workspace/account/document/item scope：实体身份与租户归属永不由 patch 改写；
/// - `wiki_type` / `chunk_type` / `created_at`：类型与创建时间保持不可变；
/// - `source_anchors` **不在默认锁中**：它由可信正文/quote 变更路径重算，并由 verify gate
///   校验。历史单数字段 `source_anchor` 不是模型字段，不能充当安全策略。
pub const DEFAULT_LOCKED_FIELDS: &[&str] = &[
    "_id",
    "workspace_id",
    "account_id",
    "document_id",
    "item_id",
    "wiki_type",
    "chunk_type",
    "created_at",
];

/// 默认走 union 合并的数组字段集合 —— LLM 给出的 patch 数组**不替换**既有数组，
/// 而是 `BTreeSet` 去重后并集。永远不会因 LLM "只列出这一项"导致历史 tag 丢失。
///
/// 注意：`related_chunks` 不在这里 —— 它是 `RelatedRef` 结构数组，需要按
/// `chunk_id` 去重而非按整 BSON 比较，由 `apply_chunk_revision` 自行处理。
pub const DEFAULT_UNION_ARRAY_KEYS: &[&str] = &[
    "tags",
    "search_terms",
    "sources",
    "applicable_scenes",
    "not_applicable_scenes",
    "business_topics",
    "product_tags",
];

/// 70% body 长度阈值（LLW page-merge.ts:53）。
pub const BODY_TRUNCATION_THRESHOLD: f64 = 0.7;

/// `apply_chunk_revision` 入参校验失败的封闭枚举。
///
/// 每一种都对应 `apply_chunk_revision` 路由层的 4xx 文案；不在这里塞业务字符串
/// （路由层翻译为中文 + 用户可理解的提示）。
#[derive(Debug, Clone, Error, PartialEq)]
pub enum RevisionError {
    /// patch 试图修改受锁定保护的字段。
    #[error("locked field '{field}' is not allowed in patch")]
    LockedFieldInPatch { field: String },
    /// patch 后 body 长度低于既有 70%（疑似 LLM 截断）。
    #[error(
        "patched body length {new_len} below 70% of existing {old_len} (threshold {threshold:.2})"
    )]
    BodyTruncated {
        old_len: usize,
        new_len: usize,
        threshold: f64,
    },
}

// ── 1. 数组字段 union ───────────────────────────────────────────────────

/// 对 `keys` 中列出的字段做应用层 union（去重 + 保序：existing 先，incoming 后）。
///
/// 三方入参（KB-09 修复：数组既有源与标量底解耦）：
/// - `base`：结果底 Document，提供**标量字段**的值（浅拷贝为结果基底）。
/// - `existing_arr_source`：数组字段既有值的来源。**必须是原始既有 chunk**，
///   而非已被 `apply_field_patch` 用 patch 覆盖过的 doc——否则数组既有元素已被
///   patch clobber，union 退化成 `patch ∪ patch`，既有 tag 丢失（KB-09 原缺陷）。
/// - `incoming`：patch，提供数组字段的新增元素。
///
/// 行为契约（PBT 覆盖）：
/// 1. **幂等**：`union(union(a, b), b) == union(a, b)`；
/// 2. **包含性**：`existing_arr_source[k]` 与 `incoming[k]` 中所有字符串元素都在结果里；
/// 3. **保序**：existing 中已有元素相对顺序保持，incoming 新元素按出现顺序追加；
/// 4. **跳过非数组**：`keys[i]` 在两侧都非数组 / 不存在 → 不动；
/// 5. **类型不匹配**：跳过非字符串元素（不同 BSON 类型不混并）。
///
/// 输入是借用，输出是新 `Document`（基于 `base` 浅拷贝 + 覆盖目标数组字段）。
pub fn union_array_fields(
    base: &Document,
    existing_arr_source: &Document,
    incoming: &Document,
    keys: &[&str],
) -> Document {
    let mut merged = base.clone();
    for &key in keys {
        let existing_arr = bson_string_array(existing_arr_source.get(key));
        let incoming_arr = bson_string_array(incoming.get(key));
        if existing_arr.is_none() && incoming_arr.is_none() {
            continue;
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out: Vec<Bson> = Vec::new();
        for s in existing_arr.unwrap_or_default() {
            if seen.insert(s.clone()) {
                out.push(Bson::String(s));
            }
        }
        for s in incoming_arr.unwrap_or_default() {
            if seen.insert(s.clone()) {
                out.push(Bson::String(s));
            }
        }
        merged.insert(key, Bson::Array(out));
    }
    merged
}

fn bson_string_array(b: Option<&Bson>) -> Option<Vec<String>> {
    match b {
        Some(Bson::Array(a)) => Some(
            a.iter()
                .filter_map(|v| match v {
                    Bson::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

/// 生效锁定字段集 = `DEFAULT_LOCKED_FIELDS` ∪ existing 文档上运营配置的
/// `locked_fields` 数组（owned 合并、去重）。
///
/// **单一真相源**：`enforce_locked_fields` 末次防线需要的 owned 锁字段集，
/// `apply_chunk_revision`（LLM/rule 写入路径）与 admin PUT（`update_operation_knowledge_chunk`）
/// 两条写入路径都调本函数构造，杜绝各自内联一份导致的 dual-path 漂移。
///
/// 注意：本函数只并入 `enforce_locked_fields` 用的**末次静默覆盖集**——
/// 运营 per-chunk 锁定字段被改动时静默覆盖回 existing、其余字段正常写；
/// 不并入 `apply_field_patch` 的硬拒集（那会连坐毙掉同一 patch 里的合法字段）。
pub fn effective_locked_fields(existing: &Document) -> Vec<String> {
    let mut out: Vec<String> = DEFAULT_LOCKED_FIELDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut seen: BTreeSet<String> = out.iter().cloned().collect();
    if let Ok(arr) = existing.get_array("locked_fields") {
        for b in arr {
            if let Some(s) = b.as_str() {
                if seen.insert(s.to_string()) {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}

// ── 2. 锁定字段强制覆盖 ────────────────────────────────────────────────

/// 把 `merged` 中位于 `locked` 列表的字段强制覆盖回 `existing` 的值。
///
/// 这是 `apply_field_patch` 已经在前置 reject 锁定字段后的二次保险：即便上游
/// 通过其它路径混入了对锁定字段的修改，这里也会把它们捞回 existing 形态。
/// LLW page-merge.ts:162-176 也有同样的"末次防线"。
pub fn enforce_locked_fields(merged: &Document, existing: &Document, locked: &[&str]) -> Document {
    let mut out = merged.clone();
    for &k in locked {
        match existing.get(k) {
            Some(v) => {
                out.insert(k, v.clone());
            }
            None => {
                out.remove(k);
            }
        }
    }
    out
}

// ── 3. 70% body 长度阈值 ───────────────────────────────────────────────

/// patch 后 body 长度是否被截断（< existing × `threshold`）。
///
/// `threshold` 由调用方传入（默认 0.7），方便 admin override 接口下一轮抬阈值。
/// `incoming_len` 当前未在判断式里使用（保留参数语义清晰：调用方关心
/// "patch 给的 body 长度"和"merged 后实际写盘 body 长度"两个数）。
pub fn is_body_truncated(
    existing_len: usize,
    incoming_len: usize,
    merged_len: usize,
    threshold: f64,
) -> bool {
    let _ = incoming_len; // 保留入参以便未来扩展（如 incoming != merged 时的特例处理）
    if existing_len == 0 {
        return false; // 既有为空 → 任何 patch 都视为新增，不触发截断
    }
    let limit = (existing_len as f64) * threshold;
    (merged_len as f64) < limit
}

// ── 4. field patch 应用 ────────────────────────────────────────────────

/// 字段级 patch 应用：先校验 patch 不携带任何 `locked` 字段，再逐字段覆盖。
///
/// 返回新 `Document`（基于 `existing` 浅拷贝 + 应用 patch）。如果任何 `patch` 顶层
/// key 命中 `locked`，立即返回 [`RevisionError::LockedFieldInPatch`]。
///
/// 注意：本函数**不做** array union，调用方应先调用 [`union_array_fields`]
/// 把数组字段 union 后再传给本函数；本函数只应用标量/对象字段覆盖。
pub fn apply_field_patch(
    existing: &Document,
    patch: &Document,
    locked: &[&str],
) -> Result<Document, RevisionError> {
    for k in patch.keys() {
        if locked.iter().any(|lk| *lk == k.as_str()) {
            return Err(RevisionError::LockedFieldInPatch {
                field: k.to_string(),
            });
        }
    }
    let mut out = existing.clone();
    for (k, v) in patch.iter() {
        out.insert(k, v.clone());
    }
    Ok(out)
}

// ── 5. canonical chunk hash ────────────────────────────────────────────

/// 与版本对比无关的字段集合（hash 时剔除）。
///
/// `updated_at`、`usage_stats`、`dynamic_confidence`、`_id` 在每次写入/反馈
/// worker 都会跳变，参与 hash 会让"内容未变但 hash 变了"成为常态。
const VOLATILE_FIELDS: &[&str] = &[
    "_id",
    "updated_at",
    "provenance",
    "usage_stats",
    "dynamic_confidence",
    "integrity_score",
    "id",
];

/// 计算 chunk 的规范化 sha256 hash —— 用于 `chunk_revisions.before_hash /
/// after_hash` 标识"本次写入到底改变了哪一份内容"。
///
/// 实现：
/// 1. clone 一份 doc，剔除 [`VOLATILE_FIELDS`]；
/// 2. 转 `serde_json::Value` 后用 BTreeMap 重排 key（递归）→ canonical JSON；
/// 3. UTF-8 字节 → sha256 → hex 小写。
///
/// 同一逻辑内容（仅字段顺序不同 / volatile 字段不同）保证产出相同 hash —— PBT
/// `hash_is_field_order_independent` 覆盖。
pub fn compute_chunk_hash(c: &Document) -> String {
    let mut clean = c.clone();
    for &k in VOLATILE_FIELDS {
        clean.remove(k);
    }
    let json = bson_to_canonical_json(&Bson::Document(clean));
    let bytes = serde_json::to_vec(&json).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    hex_lower(&digest)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 把 BSON 转成 canonical JSON：
/// - Document → JSON object，key 按字典序排序（递归）；
/// - Array → 元素递归转换；
/// - 其它标量沿用 `mongodb::bson::Bson::into_canonical_extjson()` 的语义，
///   但只取字符串/数字/布尔/null 的简单形式（避免 ext-json `$oid` 类型差异）；
/// - ObjectId / DateTime / Binary → 转为字符串表示（hash 稳定即可）。
fn bson_to_canonical_json(b: &Bson) -> serde_json::Value {
    use serde_json::{Map, Value};
    match b {
        Bson::Document(d) => {
            let mut keys: Vec<&String> = d.keys().collect();
            keys.sort();
            let mut m = Map::new();
            for k in keys {
                m.insert(k.clone(), bson_to_canonical_json(d.get(k).unwrap()));
            }
            Value::Object(m)
        }
        Bson::Array(a) => Value::Array(a.iter().map(bson_to_canonical_json).collect()),
        Bson::String(s) => Value::String(s.clone()),
        Bson::Int32(i) => Value::Number((*i).into()),
        Bson::Int64(i) => Value::Number((*i).into()),
        Bson::Double(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Bson::Boolean(b) => Value::Bool(*b),
        Bson::Null => Value::Null,
        Bson::ObjectId(oid) => Value::String(oid.to_hex()),
        Bson::DateTime(dt) => Value::String(dt.to_string()),
        Bson::Binary(bin) => Value::String(format!("__bin:{}", hex_lower(&bin.bytes))),
        // 其它类型（Decimal128 / Timestamp / RegEx 等）转 debug 字符串即可，
        // 知识库 chunk 不会用到它们；hash 稳定即可。
        other => Value::String(format!("{:?}", other)),
    }
}

// ── 单测 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn union_appends_new_and_preserves_existing_order() {
        let existing = doc! { "tags": ["a", "b", "c"] };
        let incoming = doc! { "tags": ["b", "d"] };
        let merged = union_array_fields(&existing, &existing, &incoming, &["tags"]);
        assert_eq!(
            merged.get_array("tags").unwrap(),
            &vec![
                Bson::String("a".into()),
                Bson::String("b".into()),
                Bson::String("c".into()),
                Bson::String("d".into()),
            ]
        );
    }

    #[test]
    fn union_is_idempotent_when_incoming_is_subset() {
        let existing = doc! { "tags": ["a", "b"] };
        let incoming = doc! { "tags": ["a"] };
        let merged = union_array_fields(&existing, &existing, &incoming, &["tags"]);
        assert_eq!(
            merged.get_array("tags").unwrap(),
            &vec![Bson::String("a".into()), Bson::String("b".into())]
        );
    }

    #[test]
    fn union_skips_non_array_and_missing_keys() {
        let existing = doc! { "tags": ["x"], "title": "hello" };
        let incoming = doc! { "title": "world" };
        let merged = union_array_fields(&existing, &existing, &incoming, &["tags", "title"]);
        // tags 不变、title 沿用 base（union 不处理标量字段）
        assert_eq!(merged.get_str("title").unwrap(), "hello");
        assert_eq!(
            merged.get_array("tags").unwrap(),
            &vec![Bson::String("x".into())]
        );
    }

    #[test]
    fn union_arr_source_decoupled_from_base_preserves_existing_tags() {
        // KB-09 回归哨兵：模拟生产 apply_chunk_revision 场景——base 的数组字段已被
        // apply_field_patch 用 patch 整体覆盖成 ["B"]（既有 "A" 在 base 里已丢），
        // 但 existing_arr_source 是**原始既有**（含 "A"）。union 须从 existing_arr_source
        // 取数组既有 → 结果含 A 含 B（既有 tag 不丢）。若退回"数组源=base"旧缺陷，
        // 结果会是 ["B"]、A 丢失，本测立刻红。
        let base = doc! { "product_tags": ["B"], "title": "标量patch后的值" };
        let existing = doc! { "product_tags": ["A"], "title": "原始标量" };
        let patch = doc! { "product_tags": ["B"] };
        let merged = union_array_fields(&base, &existing, &patch, &["product_tags"]);
        let tags = merged.get_array("product_tags").unwrap();
        assert!(
            tags.contains(&Bson::String("A".into())),
            "既有 tag A 不得丢失（KB-09）"
        );
        assert!(
            tags.contains(&Bson::String("B".into())),
            "patch 新增 tag B 须在"
        );
        // 标量字段沿用 base（保标量 patch 结果）。
        assert_eq!(merged.get_str("title").unwrap(), "标量patch后的值");
    }

    #[test]
    fn enforce_locked_overrides_when_merged_diverges() {
        let existing = doc! { "workspace_id": "ws1", "wiki_type": "entity", "title": "T" };
        let merged = doc! { "workspace_id": "evil", "wiki_type": "evil", "title": "T2" };
        let out = enforce_locked_fields(&merged, &existing, &["workspace_id", "wiki_type"]);
        assert_eq!(out.get_str("workspace_id").unwrap(), "ws1");
        assert_eq!(out.get_str("wiki_type").unwrap(), "entity");
        assert_eq!(out.get_str("title").unwrap(), "T2"); // 非锁定字段保留 merged 形态
    }

    #[test]
    fn body_truncation_detects_below_70_percent() {
        // existing=100, merged=60 → 60/100=0.6 < 0.7 → true
        assert!(is_body_truncated(100, 60, 60, 0.7));
        // existing=100, merged=70 → 70/100=0.7 NOT < 0.7 → false（边界包含）
        assert!(!is_body_truncated(100, 70, 70, 0.7));
        // existing=100, merged=200 → false
        assert!(!is_body_truncated(100, 200, 200, 0.7));
        // 既有为空 → 永远不截断
        assert!(!is_body_truncated(0, 999, 999, 0.7));
    }

    #[test]
    fn apply_patch_rejects_locked_field() {
        let existing = doc! { "workspace_id": "ws1", "title": "T" };
        let patch = doc! { "workspace_id": "EVIL" };
        let err = apply_field_patch(&existing, &patch, DEFAULT_LOCKED_FIELDS).unwrap_err();
        assert_eq!(
            err,
            RevisionError::LockedFieldInPatch {
                field: "workspace_id".into()
            }
        );
    }

    #[test]
    fn apply_patch_overrides_non_locked_fields() {
        let existing = doc! { "workspace_id": "ws1", "title": "old", "summary": "s" };
        let patch = doc! { "title": "new" };
        let out = apply_field_patch(&existing, &patch, DEFAULT_LOCKED_FIELDS).unwrap();
        assert_eq!(out.get_str("title").unwrap(), "new");
        assert_eq!(out.get_str("summary").unwrap(), "s");
        assert_eq!(out.get_str("workspace_id").unwrap(), "ws1");
    }

    #[test]
    fn hash_is_stable_for_same_content() {
        let a = doc! { "title": "T", "tags": ["a", "b"] };
        let b = doc! { "tags": ["a", "b"], "title": "T" }; // 字段顺序不同
        assert_eq!(compute_chunk_hash(&a), compute_chunk_hash(&b));
    }

    #[test]
    fn hash_changes_on_content_change() {
        let a = doc! { "title": "T", "tags": ["a", "b"] };
        let b = doc! { "title": "T", "tags": ["a", "b", "c"] };
        assert_ne!(compute_chunk_hash(&a), compute_chunk_hash(&b));
    }

    #[test]
    fn hash_ignores_volatile_fields() {
        let now = mongodb::bson::DateTime::now();
        let later = mongodb::bson::DateTime::from_millis(now.timestamp_millis() + 86_400_000);
        let a = doc! {
            "title": "T",
            "updated_at": now,
            "dynamic_confidence": 0.5,
            "provenance": { "source": "human", "edited_by": "a" },
        };
        let b = doc! {
            "title": "T",
            "updated_at": later,
            "dynamic_confidence": 0.9,
            "provenance": { "source": "ai", "edited_by": "b" },
        };
        assert_eq!(compute_chunk_hash(&a), compute_chunk_hash(&b));
    }

    #[test]
    fn effective_locked_unions_default_with_runtime_and_dedups() {
        // 无运营锁 → 恰是 DEFAULT。
        let bare = doc! { "title": "T" };
        assert_eq!(
            effective_locked_fields(&bare).len(),
            DEFAULT_LOCKED_FIELDS.len()
        );
        // 运营锁定 "title"（非 DEFAULT）+ 重复一个 DEFAULT 项 "workspace_id"（应去重）。
        let with_lock = doc! { "locked_fields": ["title", "workspace_id"] };
        let eff = effective_locked_fields(&with_lock);
        assert!(eff.contains(&"title".to_string()), "运营锁定字段须并入");
        assert!(
            eff.contains(&"workspace_id".to_string()),
            "DEFAULT 字段须保留"
        );
        // "workspace_id" 已在 DEFAULT，不应重复计数：len == DEFAULT + 1（仅新增 title）。
        assert_eq!(eff.len(), DEFAULT_LOCKED_FIELDS.len() + 1, "重复项须去重");
    }

    #[test]
    fn enforce_locked_honors_runtime_locked_fields() {
        // KB-11：运营在 chunk 上锁定 "title"（内容字段，非 DEFAULT 集）→ merged 改了 title
        // → enforce_locked_fields 传入 DEFAULT ∪ ["title"] 时，title 被覆盖回 existing 值，
        //   未锁字段 summary 正常保留（静默覆盖，不毙整条）。
        let existing = doc! { "title": "旧标题", "summary": "旧摘要" };
        let merged = doc! { "title": "AI改的新标题", "summary": "AI改的新摘要" };
        let locked: Vec<&str> = vec!["title"]; // 模拟 DEFAULT ∪ existing.locked_fields 里的运营锁
        let out = enforce_locked_fields(&merged, &existing, &locked);
        assert_eq!(
            out.get_str("title").unwrap(),
            "旧标题",
            "锁定字段 title 须覆盖回 existing"
        );
        assert_eq!(
            out.get_str("summary").unwrap(),
            "AI改的新摘要",
            "未锁字段 summary 正常保留"
        );
    }
}
