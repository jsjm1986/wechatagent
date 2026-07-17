//! `wiki_gap_signals_3kinds` —— 三类新 gap_signal kind 的端到端集成测试。
//!
//! 覆盖范围（Plan v3 / Commit 2）：
//!   - `missing_chunk`：related_chunks 引用了已 archived 的 chunk → 应产生
//!     severity=error 的 missing_chunk 信号，且不再产生 broken_link；
//!     依赖恢复（chunk 重新 active）后 sweep 应 auto_resolved with
//!     `resolution_note="rule:dep_restored"`。
//!   - `suggestion`：未 verified 且 30d blocked > 3 → 产生 info suggestion；
//!     一旦 chunk integrity_status 改为 verified，sweep 应 auto_resolved with
//!     `resolution_note="rule:chunk_verified"`。
//!   - `contradiction`：同 normalize_title 多 chunk 且 body 首段 sha256 不一致
//!     → 产生 error contradiction；当其中一条 chunk 被 archived（同题只剩一条）
//!     后 sweep 应 auto_resolved with `resolution_note="rule:contradiction_resolved"`。
//!
//! 同时校验 dedup 不变量：连续两次 `run_structural_lint` 不应使同一 (kind, title)
//! 信号被重复 insert（`new_signals` 计数仅首次 > 0，第二次为 0）。
//!
//! 三类信号都是规则路径，不消耗 LLM；测试不需要为 `TestApp` 入队任何 LLM 响应。
//!
//! `#[ignore]` 守门：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use tokio::sync::Barrier;
use wechatagent::knowledge_wiki::gap_signals;
use wechatagent::models::{KnowledgeGapSignal, OperationKnowledgeChunk, RelatedRef, UsageStats};

use crate::common::TestApp;

const WS: &str = "ws_3kinds";

fn base_chunk(title: &str, wiki_type: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        title: title.to_string(),
        wiki_type: Some(wiki_type.to_string()),
        status: "active".to_string(),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

async fn insert_chunks(app: &TestApp, chunks: &[OperationKnowledgeChunk]) {
    for c in chunks {
        app.state
            .db
            .operation_knowledge_chunks()
            .insert_one(c, None)
            .await
            .expect("insert chunk");
    }
}

async fn list_pending(app: &TestApp, kind: &str) -> Vec<KnowledgeGapSignal> {
    use futures::TryStreamExt;
    let cursor = app
        .state
        .db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": WS, "kind": kind, "status": "pending" },
            None,
        )
        .await
        .expect("query gap_signals");
    cursor.try_collect().await.expect("collect")
}

async fn list_resolved(app: &TestApp, kind: &str) -> Vec<KnowledgeGapSignal> {
    use futures::TryStreamExt;
    let cursor = app
        .state
        .db
        .knowledge_gap_signals()
        .find(
            doc! { "workspace_id": WS, "kind": kind, "status": "auto_resolved" },
            None,
        )
        .await
        .expect("query resolved");
    cursor.try_collect().await.expect("collect")
}

#[tokio::test]
#[ignore]
async fn missing_chunk_emitted_when_target_archived_then_resolved_when_restored() {
    let app = TestApp::start().await;

    // 源 chunk 引用一个 target，target 后续被 archive。
    let target_oid = ObjectId::new();
    let target_hex = target_oid.to_hex();
    let mut target = base_chunk("被回收页", "entity");
    target.id = Some(target_oid);
    target.status = "archived".to_string();

    let mut src = base_chunk("源页", "entity");
    src.related_chunks = Some(vec![RelatedRef {
        chunk_id: target_hex.clone(),
        kind: "references".to_string(),
        note: None,
    }]);

    insert_chunks(&app, &[target.clone(), src.clone()]).await;

    // ── 第一次 lint：应产生 missing_chunk，且不应有 broken_link ──
    let report1 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("structural lint 1");
    assert!(
        report1.new_signals >= 1,
        "首次 lint 必须新增至少一条 signal, got {report1:?}"
    );
    let missing_pending = list_pending(&app, "missing_chunk").await;
    assert_eq!(
        missing_pending.len(),
        1,
        "missing_chunk 必须正好 1 条 pending（target archived）"
    );
    let sig = &missing_pending[0];
    assert_eq!(sig.severity, "error");
    assert_eq!(sig.source, "rule");
    assert!(
        sig.affected_chunk_ids.iter().any(|id| id == &target_hex),
        "affected_chunk_ids 必须包含 target id, got {:?}",
        sig.affected_chunk_ids
    );

    let broken_pending = list_pending(&app, "broken_link").await;
    assert!(
        broken_pending.is_empty(),
        "target 在 archived 时不该再产生 broken_link, got {broken_pending:?}"
    );

    // ── dedup：第二次 lint 不应再 insert 同 kind+title ──
    let report2 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("structural lint 2");
    assert_eq!(
        report2.new_signals, 0,
        "重复 lint 不该再 insert（dedup_key 命中）, got {report2:?}"
    );
    let still_pending = list_pending(&app, "missing_chunk").await;
    assert_eq!(still_pending.len(), 1, "pending 数量不应变化");

    // ── 依赖恢复：把 target 改回 active，再 sweep ──
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": target_oid },
            doc! { "$set": { "status": "active" } },
            None,
        )
        .await
        .expect("restore target");

    let sweep = gap_signals::sweep_stale_signals(&app.state.db, WS)
        .await
        .expect("sweep");
    assert!(
        sweep.stage1_auto_resolved >= 1,
        "依赖恢复后 sweep 应至少消解 1 条, got {sweep:?}"
    );

    let resolved = list_resolved(&app, "missing_chunk").await;
    assert_eq!(resolved.len(), 1, "missing_chunk 应有一条 auto_resolved");
    assert_eq!(
        resolved[0].resolution_note.as_deref(),
        Some("rule:dep_restored"),
        "resolution_note 必须区分自愈原因"
    );
}

#[tokio::test]
#[ignore]
async fn suggestion_emitted_when_unverified_and_blocked_then_resolved_when_verified() {
    let app = TestApp::start().await;

    // 草稿 chunk：integrity_status=needs_review, blocked_count_30d=5
    let oid = ObjectId::new();
    let hex = oid.to_hex();
    let mut chunk = base_chunk("常被 grounding 拦的草稿", "entity");
    chunk.id = Some(oid);
    chunk.integrity_status = Some("needs_review".to_string());
    chunk.usage_stats = Some(UsageStats {
        hit_count_30d: 1,
        blocked_count_30d: 5,
        last_used_at: None,
        last_blocked_reason: Some("missing_source_quote".to_string()),
    });

    insert_chunks(&app, &[chunk]).await;

    // ── lint：suggestion 必须出现 ──
    gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 1");
    let pending = list_pending(&app, "suggestion").await;
    assert_eq!(pending.len(), 1, "suggestion 必须正好 1 条 pending");
    assert_eq!(pending[0].severity, "info");
    assert_eq!(pending[0].source, "rule");
    assert_eq!(pending[0].affected_chunk_ids, vec![hex.clone()]);

    // ── dedup：第二次 lint 不应重复 insert ──
    let report2 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 2");
    assert_eq!(
        report2.new_signals, 0,
        "suggestion dedup 失败，第二次仍 insert"
    );

    // ── verify chunk → sweep 应 auto_resolved ──
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": oid },
            doc! { "$set": { "integrity_status": "verified" } },
            None,
        )
        .await
        .expect("verify chunk");

    let sweep = gap_signals::sweep_stale_signals(&app.state.db, WS)
        .await
        .expect("sweep");
    assert!(
        sweep.stage1_auto_resolved >= 1,
        "verify 后 sweep 必须消解, got {sweep:?}"
    );
    let resolved = list_resolved(&app, "suggestion").await;
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].resolution_note.as_deref(),
        Some("rule:chunk_verified")
    );
}

#[tokio::test]
#[ignore]
async fn contradiction_emitted_when_same_title_diff_first_paragraph_then_resolved_when_archived() {
    let app = TestApp::start().await;

    // 同题两条 chunk，body 首段不同 → contradiction
    let mut a = base_chunk("产品价格策略", "methodology");
    a.body = Some("策略一：阶梯价。\n\n详细说明……".to_string());

    let mut b = base_chunk("产品价格策略", "methodology");
    let b_oid = b.id.expect("oid");
    b.body = Some("策略二：固定价。\n\n详细说明……".to_string());

    insert_chunks(&app, &[a.clone(), b.clone()]).await;

    // ── lint：contradiction 应出现 ──
    gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 1");
    let pending = list_pending(&app, "contradiction").await;
    assert_eq!(pending.len(), 1, "contradiction 必须正好 1 条 pending");
    assert_eq!(pending[0].severity, "error");
    assert_eq!(pending[0].source, "rule");
    // affected_chunk_ids 应包含两条 chunk 的 id（顺序不强制）
    assert_eq!(
        pending[0].affected_chunk_ids.len(),
        2,
        "contradiction affected 必须含两条 chunk id"
    );

    // ── dedup：第二次 lint 不应重复 ──
    let report2 = gap_signals::run_structural_lint(&app.state.db, WS)
        .await
        .expect("lint 2");
    assert_eq!(
        report2.new_signals, 0,
        "contradiction dedup 失败，第二次仍 insert"
    );

    // ── 把 b archive 掉（同题只剩 a）→ sweep 应消解 ──
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": b_oid },
            doc! { "$set": { "status": "archived" } },
            None,
        )
        .await
        .expect("archive b");

    let sweep = gap_signals::sweep_stale_signals(&app.state.db, WS)
        .await
        .expect("sweep");
    assert!(
        sweep.stage1_auto_resolved >= 1,
        "同题去重后 sweep 必须消解 contradiction, got {sweep:?}"
    );
    let resolved = list_resolved(&app, "contradiction").await;
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].resolution_note.as_deref(),
        Some("rule:contradiction_resolved")
    );
}

/// KB-07 真回归哨兵：同 kind(recall_miss) 下多条不同主题的 pending 信号并存时，
/// `persist_recall_signal` 必须精确合并进 dedup_key 匹配的那一条，绝不因原
/// find_one({workspace,status,kind} 无序) 只看到"任意一条"而漏合并、误新建重复条。
///
/// 构造要点（据 gap_signals.rs 亲验）：
///   - recall_miss 的 dedup_key = `recall_miss::{normalize_title(title)}`
///     （affected 为空，走默认分支 gap_signals.rs:471-474）；
///   - title = `产品宣称缺 verified 知识背书：{customer_query.chars().take(40)}`
///     （gap_signals.rs:436-439）→ 两 query 的**前 40 字符**相同即命中同一 dedup_key；
///   - `recall_miss_from_product_block` 把**整串** customer_query push 进 search_queries
///     （gap_signals.rs:451-453）→ 前 40 字符相同但整串不同 = 命中同 dedup_key + 带来新
///     search_query 变体，正是合并分支该累积的场景。
///   - 因此 query_b / query_b2 取 >40 字符、前 40 字符完全一致、仅在第 40 字符之后追加
///     后缀 → dedup_key 命中 B、search_query 是新的。
///
/// **为何 seed 三条、且让 B 居中**（哨兵必红的关键）：`knowledge_gap_signals` 上有
/// 两条都能覆盖 `{workspace,status,kind}` 等值谓词的索引——
/// `gap_signals_status_kind_idx {workspace:1,status:1,kind:1}`（indexes.rs:1399，本
/// 用例三条行的这三键全等 → 同键内按自然/插入序 → 无序 find_one 先返回**最早**的 A）
/// 与 `gap_signals_kind_status_created_idx {...,created_at:-1}`（indexes.rs:1442 → 先
/// 返回**最新**的 C）。若只 seed A、B 两条，B 恰是最新，planner 一旦选中 created_at:-1
/// 索引，回退的无序 find_one 会**恰好**返回 B → 误合并成功 → 哨兵在坏代码下也绿（失效）。
/// 让 B 居中（A 最早、C 最新）→ 两种索引序下无序 find_one 都返回非 B（A 或 C）→ dedup_key
/// 落空 → 走新建分支 → count 变 4 → 红。修复后（全量 find + 精确 find）恒命中 B → 仍 3 条 → 绿。
#[tokio::test]
#[ignore]
async fn recall_signal_merges_correct_topic_among_multiple_pending() {
    let app = TestApp::start().await;

    // 三个不同主题，前 40 字符两两不同 → 三个不同 dedup_key。B 取 >40 字符（其变体的后缀须
    // 落在 take(40) 之外才不改 title 截断）。seed 顺序 A→B→C 使 B 居中（非最早非最新）。
    let query_a = "A产品的整机质保期到底是多少个月这一条我一直没在知识库里查到过对应的条款".to_string();
    let query_b =
        "B旗舰套餐每个月赠送的移动数据流量上限到底是多少这个数字我得给客户一个准确的答复不能含糊".to_string();
    let query_c = "C尊享会员的专属线下沙龙活动每个季度到底安排几场这块我手头没有任何可以引用的资料".to_string();

    for (q, label) in [(&query_a, "seed A"), (&query_b, "seed B"), (&query_c, "seed C")] {
        gap_signals::persist_recall_signal(
            &app.state.db,
            WS,
            gap_signals::GapSignalCandidate::recall_miss_from_product_block(q.clone()),
        )
        .await
        .expect(label);
    }

    let pending = list_pending(&app, "recall_miss").await;
    assert_eq!(pending.len(), 3, "三个不同主题应建 3 条 pending, got {pending:?}");

    // 前提亲验：query_b 与其变体的前 40 字符必须一致（否则 dedup_key 不会命中 B）。
    let prefix40 = |s: &str| s.chars().take(40).collect::<String>();
    // query_b2：前 40 字符与 query_b 完全相同、整串不同（在第 40 字符之后追加后缀）。
    let query_b2 = format!("{query_b} 另外也顺带确认下超出上限之后怎么计费");
    assert_eq!(
        prefix40(&query_b),
        prefix40(&query_b2),
        "变体前 40 字符必须与 B 一致才会命中同 dedup_key（构造前提）"
    );
    assert_ne!(
        query_b, query_b2,
        "变体整串必须不同才能产生新的 search_query"
    );
    // 且 query_b 本身须 >40 字符，否则 take(40) 会把后缀也纳入 title → 变体不再同 dedup_key。
    assert!(
        query_b.chars().count() > 40,
        "锚点 query_b 须 >40 字符（当前 {}）",
        query_b.chars().count()
    );

    // ── 再来一次匹配主题 B、但带新 query 变体的信号 ──
    gap_signals::persist_recall_signal(
        &app.state.db,
        WS,
        gap_signals::GapSignalCandidate::recall_miss_from_product_block(query_b2.clone()),
    )
    .await
    .expect("merge into B");

    // ── 断言：精确合并进 B、绝不误新建第 4 条；B 的 search_queries 累积两个变体 ──
    let pending2 = list_pending(&app, "recall_miss").await;
    assert_eq!(
        pending2.len(),
        3,
        "精确合并须仍 3 条；回退 find_one 无序会命中 A 或 C（非 B）、漏合并 → 误新建变 4 条, got {pending2:?}"
    );
    // signal_dedup_key 是 pub(crate)，tests crate 不可见 → 用 search_queries 包含 query_b 定位 B。
    let b = pending2
        .iter()
        .find(|s| s.search_queries.iter().any(|q| q == &query_b))
        .expect("B 仍在 pending（按 search_queries 含 query_b 定位）");
    assert!(
        b.search_queries.iter().any(|q| q == &query_b2),
        "B 应累积 query_b2 变体, got {:?}",
        b.search_queries
    );
    assert!(
        b.search_queries.len() >= 2,
        "B 应至少累积 query_b 与 query_b2 两个变体, got {:?}",
        b.search_queries
    );
    // A / C 不应被污染：各自 search_queries 只含自己的 query，不含 B 的任何变体。
    for (own, label) in [(&query_a, "A"), (&query_c, "C")] {
        let sig = pending2
            .iter()
            .find(|s| s.search_queries.iter().any(|q| q == own))
            .unwrap_or_else(|| panic!("{label} 仍在 pending"));
        assert!(
            !sig.search_queries.iter().any(|q| q == &query_b || q == &query_b2),
            "{label} 不应被 B 的变体污染, got {:?}",
            sig.search_queries
        );
    }
}

/// Legacy rows created before `dedup_key` existed must still be matched by
/// their derived business key. Enriching the same topic must merge into the
/// legacy pending row instead of creating a second modern row.
#[tokio::test]
#[ignore]
async fn recall_signal_merges_into_legacy_row_without_persisted_dedup_key() {
    let app = TestApp::start().await;
    let original_query =
        "历史套餐每年包含的服务额度和超额计费规则是什么需要一份可以核验的准确说明".to_string();
    let variant_query = format!("{original_query} 另外请补充超额后的计费单位");
    assert_eq!(
        original_query.chars().take(40).collect::<String>(),
        variant_query.chars().take(40).collect::<String>(),
        "fixture variants must derive the same logical key"
    );

    let candidate =
        gap_signals::GapSignalCandidate::recall_miss_from_product_block(original_query.clone());
    let legacy = doc! {
        "signal_id": "legacy_gap_without_dedup_key",
        "workspace_id": WS,
        "kind": "recall_miss",
        "title": candidate.title,
        "description": candidate.description,
        "affected_chunk_ids": candidate.affected_chunk_ids,
        "search_queries": [original_query.clone()],
        "severity": candidate.severity,
        "source": "recall_trace",
        "status": "pending",
        "created_at": BsonDt::now(),
    };
    app.state
        .db
        .knowledge_gap_signals()
        .clone_with_type::<mongodb::bson::Document>()
        .insert_one(legacy, None)
        .await
        .expect("insert legacy gap row");

    gap_signals::persist_recall_signal(
        &app.state.db,
        WS,
        gap_signals::GapSignalCandidate::recall_miss_from_product_block(variant_query.clone()),
    )
    .await
    .expect("merge into legacy row");

    let pending = list_pending(&app, "recall_miss").await;
    assert_eq!(pending.len(), 1, "legacy match must not create a duplicate row");
    assert_eq!(pending[0].signal_id, "legacy_gap_without_dedup_key");
    assert!(pending[0].dedup_key.is_none(), "legacy row remains readable without migration");
    assert!(
        pending[0].search_queries.iter().any(|q| q == &variant_query),
        "new query variant must merge into the legacy row"
    );
}

/// Concurrent writers for one business key must converge to one pending row.
/// Each writer carries a distinct query variant, so the test also verifies
/// that the atomic upsert merges arrays instead of dropping a loser's data.
#[tokio::test]
#[ignore]
async fn concurrent_recall_signals_upsert_one_pending_and_merge_all_queries() {
    const WRITERS: usize = 16;

    let app = TestApp::start().await;
    let prefix = "同一产品套餐的年度服务额度和超额计费规则究竟是什么请给出能够核验的准确资料并说明适用范围";
    assert!(prefix.chars().count() >= 40, "prefix must fill the title cap");

    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut handles = Vec::with_capacity(WRITERS);
    let mut expected_queries = Vec::with_capacity(WRITERS);
    for index in 0..WRITERS {
        let db = app.state.db.clone();
        let barrier = barrier.clone();
        let query = format!("{prefix} writer-{index}");
        expected_queries.push(query.clone());
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            gap_signals::persist_recall_signal(
                &db,
                WS,
                gap_signals::GapSignalCandidate::recall_miss_from_product_block(query),
            )
            .await
        }));
    }

    for handle in handles {
        handle
            .await
            .expect("concurrent writer should not panic")
            .expect("concurrent signal upsert should succeed");
    }

    let pending = list_pending(&app, "recall_miss").await;
    assert_eq!(
        pending.len(),
        1,
        "one business dedup key must occupy exactly one pending row: {pending:?}"
    );
    let signal = &pending[0];
    let persisted_key = signal
        .dedup_key
        .as_deref()
        .expect("modern pending row must persist a dedup key");
    assert_eq!(persisted_key.len(), 64);
    for query in expected_queries {
        assert!(
            signal.search_queries.iter().any(|stored| stored == &query),
            "concurrent query variant must be retained: {query:?}; stored={:?}",
            signal.search_queries
        );
    }
}
