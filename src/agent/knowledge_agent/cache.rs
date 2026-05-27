//! Phase E / E4：knowledge_agent 答案缓存。
//!
//! 设计：
//! - 进程级单例 [`OnceLock`] HashMap，TTL 默认 5 分钟。
//! - Key = `(workspace_id, account_id, query_normalized, corpus_signature, max_rounds)`；
//!   `corpus_signature` 由 `list_catalog` 的 chunk_id+updated_at 集合摘要而来，
//!   chunk 任一改动 → signature 变 → 自动失效。
//! - Value = 完整 [`super::AnswerResult`] 克隆 + insert_at。
//! - 容量上限 [`MAX_ENTRIES`]，溢出时按"最旧 insert_at"驱逐。
//! - 不缓存 `cancelled=true` / `truncated=true` 的结果（结果不稳定）。
//!
//! 隔离：本模块仅依赖 `std` + 已用 crates；零新依赖。
//! 与 [`super::RunBudget`] 互不相关——cache hit 时跳过 LLM 调用，自然省 budget。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use super::AnswerResult;

/// 缓存条目 TTL；超过即视为过期，下一次 lookup 不命中且懒删除。
const TTL: Duration = Duration::from_secs(300);

/// 单租户/单进程缓存条目数量上限；溢出按最旧 insert_at 驱逐。
const MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(super) struct CacheKey {
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub query_norm: String,
    pub corpus_sig: u64,
    pub max_rounds: i32,
}

#[derive(Clone)]
struct CacheEntry {
    result: AnswerResult,
    inserted_at: Instant,
}

struct CacheState {
    map: HashMap<CacheKey, CacheEntry>,
    hits: u64,
    misses: u64,
}

fn cache() -> &'static RwLock<CacheState> {
    static CACHE: OnceLock<RwLock<CacheState>> = OnceLock::new();
    CACHE.get_or_init(|| {
        RwLock::new(CacheState {
            map: HashMap::new(),
            hits: 0,
            misses: 0,
        })
    })
}

/// 标准化 query：trim + 折叠连续空白 + lowercase ASCII（CJK 不变）。
pub(super) fn normalize_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    let mut prev_ws = true;
    for ch in q.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_ws = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// 通过 chunk_id + updated_at_millis 列表计算 corpus 签名；
/// 任一 chunk 改动或集合大小变化 → 签名变。
pub(super) fn corpus_signature(items: &[(String, i64)]) -> u64 {
    let mut h = DefaultHasher::new();
    items.len().hash(&mut h);
    for (id, ts) in items {
        id.hash(&mut h);
        ts.hash(&mut h);
    }
    h.finish()
}

/// Lookup；命中且未过期返回 Some(克隆)。命中过期会被懒删除。
pub(super) fn get(key: &CacheKey) -> Option<AnswerResult> {
    {
        let guard = cache().read().ok()?;
        if let Some(entry) = guard.map.get(key) {
            if entry.inserted_at.elapsed() < TTL {
                drop(guard);
                let mut w = cache().write().ok()?;
                w.hits = w.hits.saturating_add(1);
                return Some(w.map.get(key).map(|e| e.result.clone())).flatten();
            }
        } else {
            drop(guard);
            let mut w = cache().write().ok()?;
            w.misses = w.misses.saturating_add(1);
            return None;
        }
    }
    // 过期分支：懒删除并计 miss。
    let mut w = cache().write().ok()?;
    w.map.remove(key);
    w.misses = w.misses.saturating_add(1);
    None
}

/// Put；不缓存 cancelled / truncated 结果。容量到限按 insert_at 最旧驱逐。
pub(super) fn put(key: CacheKey, result: AnswerResult) {
    if result.cancelled || result.truncated {
        return;
    }
    let Ok(mut w) = cache().write() else { return };
    if w.map.len() >= MAX_ENTRIES {
        // 简单 LRU 替代：扫一遍找最旧 insert_at。N <= 256，O(N) 可接受。
        if let Some(oldest_key) = w
            .map
            .iter()
            .min_by_key(|(_, e)| e.inserted_at)
            .map(|(k, _)| k.clone())
        {
            w.map.remove(&oldest_key);
        }
    }
    w.map.insert(
        key,
        CacheEntry {
            result,
            inserted_at: Instant::now(),
        },
    );
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerCacheStats {
    pub entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub max_entries: usize,
    pub ttl_seconds: u64,
}

pub fn cache_stats() -> AnswerCacheStats {
    let guard = match cache().read() {
        Ok(g) => g,
        Err(_) => {
            return AnswerCacheStats {
                entries: 0,
                hits: 0,
                misses: 0,
                max_entries: MAX_ENTRIES,
                ttl_seconds: TTL.as_secs(),
            };
        }
    };
    AnswerCacheStats {
        entries: guard.map.len(),
        hits: guard.hits,
        misses: guard.misses,
        max_entries: MAX_ENTRIES,
        ttl_seconds: TTL.as_secs(),
    }
}

#[cfg(test)]
pub(super) fn clear_for_test() {
    if let Ok(mut w) = cache().write() {
        w.map.clear();
        w.hits = 0;
        w.misses = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 单元测试串行守门：cache 是进程级单例，多个 #[test] 并发跑会互相污染
    /// hits/misses/entries 计数。用一个进程级 Mutex 强制 cache 相关 test 顺序跑，
    /// 比拉 `serial_test` crate 0 新依赖。
    static SERIAL: Mutex<()> = Mutex::new(());

    fn mk_result(answer: &str) -> AnswerResult {
        AnswerResult {
            answer: answer.to_string(),
            cited_chunk_ids: Vec::new(),
            source_quotes: Vec::new(),
            tool_trace: Vec::new(),
            rounds_used: 1,
            truncated: false,
            cancelled: false,
        }
    }

    fn mk_key(query: &str) -> CacheKey {
        CacheKey {
            workspace_id: "ws".into(),
            account_id: None,
            query_norm: normalize_query(query),
            corpus_sig: 42,
            max_rounds: 3,
        }
    }

    #[test]
    fn normalize_query_collapses_whitespace_and_lowercases_ascii() {
        assert_eq!(normalize_query("  Hello   WORLD  "), "hello world");
        assert_eq!(normalize_query("价格   异议"), "价格 异议");
    }

    #[test]
    fn corpus_signature_stable_for_same_input() {
        let a = corpus_signature(&[("c1".into(), 1), ("c2".into(), 2)]);
        let b = corpus_signature(&[("c1".into(), 1), ("c2".into(), 2)]);
        assert_eq!(a, b);
    }

    #[test]
    fn corpus_signature_changes_when_chunk_updated_at_changes() {
        let a = corpus_signature(&[("c1".into(), 1)]);
        let b = corpus_signature(&[("c1".into(), 2)]);
        assert_ne!(a, b);
    }

    #[test]
    fn get_returns_none_when_empty() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_for_test();
        let k = mk_key("hello");
        assert!(get(&k).is_none());
        assert_eq!(cache_stats().misses, 1);
    }

    #[test]
    fn put_then_get_round_trips() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_for_test();
        let k = mk_key("hello world");
        put(k.clone(), mk_result("answer"));
        let r = get(&k).expect("must hit");
        assert_eq!(r.answer, "answer");
        assert_eq!(cache_stats().entries, 1);
        assert_eq!(cache_stats().hits, 1);
    }

    #[test]
    fn truncated_results_are_not_cached() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_for_test();
        let k = mk_key("trunc");
        let mut r = mk_result("partial");
        r.truncated = true;
        put(k.clone(), r);
        assert!(get(&k).is_none());
    }

    #[test]
    fn cancelled_results_are_not_cached() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_for_test();
        let k = mk_key("cancel");
        let mut r = mk_result("partial");
        r.cancelled = true;
        put(k.clone(), r);
        assert!(get(&k).is_none());
    }

    #[test]
    fn different_corpus_sig_misses() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_for_test();
        let mut k = mk_key("same query");
        put(k.clone(), mk_result("a"));
        k.corpus_sig = 99;
        assert!(get(&k).is_none());
    }

    #[test]
    fn cache_stats_reports_entries_and_hit_miss() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        clear_for_test();
        let k = mk_key("alpha");
        put(k.clone(), mk_result("a"));
        let _ = get(&k);
        let _ = get(&mk_key("beta"));
        let s = cache_stats();
        assert_eq!(s.entries, 1);
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.max_entries, MAX_ENTRIES);
        assert_eq!(s.ttl_seconds, TTL.as_secs());
    }
}
