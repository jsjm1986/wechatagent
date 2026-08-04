//! `apply_chunk_revision` 红线集成测试:source=Ai 的写入**强制**把 chunk 打回
//! `status="draft"` + `integrity_status="needs_review"`(AI 永不自动 verify)。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test chunk_revision_ai_draft_integration -- --ignored`。
//!
//! ## 红线意义(P0):AI 起草的知识写入**必须**降级为 draft + needs_review
//! (chunk_revisions.rs:207-212 强制)。chat 的 `apply_create_chunk` 创建路径已由
//! `knowledge_chat_apply_integration.rs` 覆盖;但 `apply_chunk_revision` 的
//! **source=Ai patch 这一支**此前仅有纯函数 PBT 模拟(`wiki_chunk_revision_pbt.rs`
//! 断言测试自己复制的逻辑,删掉生产 :207-212 也不变红)。本测试 seed 一条 active +
//! verified 的既有 chunk,真调 `apply_chunk_revision`(source=Ai, op=Patch)驱动生产
//! 降级逻辑,落库后查 DB 断言被打回 draft + needs_review。一旦 :207-212 被删,本测试
//! 立刻红(verified 不会被打回)。
#![cfg(test)]

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use tokio::sync::Barrier;
use wechatagent::error::AppError;
use wechatagent::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, ProvenanceSource, RevisionOp, RevisionRequest,
};
use wechatagent::models::{OperationKnowledgeChunk, OperationKnowledgeDocument};

use crate::common::TestApp;

/// 红线:source=Ai 的 patch 把 active+verified 的 chunk 强制打回 draft+needs_review。
#[tokio::test]
#[ignore]
async fn apply_chunk_revision_ai_source_forces_draft_needs_review() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed 一条既有 chunk,故意置为 active + verified 作为反例初值:
    // 若 :207-212 不生效,patch 后仍是 active/verified,断言立刻红。
    // body 给足长度(远超摘要),避免 patch summary 触发 70% 截断闸
    // (text_payload_len 取 body/summary 较长者;body 不变 → new_len==old_len,不截断)。
    let chunk_oid = ObjectId::new();
    let seeded = OperationKnowledgeChunk {
        id: Some(chunk_oid),
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        title: "退款政策".to_string(),
        summary: Some("原始摘要:七天无理由退款规则概述。".to_string()),
        body: Some(
            "支持七天无理由退款,商品需保持完好、配件齐全、不影响二次销售。\
             超过七天或商品已拆封使用的,按具体情况人工评估处理。运费由买方承担,\
             质量问题除外。以上为退款政策正文,内容较长以确保过 70% body 长度阈值。"
                .to_string(),
        ),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("insert seeded chunk");

    // 真调 apply_chunk_revision,source=Ai + op=Patch,只改 summary。
    // 注意:第一个参数是 &Database(app.state.db),不是 &AppState。
    let applied = apply_chunk_revision(
        &app.state.db,
        &ws,
        chunk_oid,
        RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Ai,
            patch: doc! { "summary": "AI 改写后的摘要:七天无理由退款,需商品完好。" },
            reason: Some("test: ai patch forces draft".to_string()),
            actor: None,
        },
    )
    .await
    .expect("apply_chunk_revision 应成功");

    // status active→draft + integrity verified→needs_review 会改变 canonical hash,
    // 因此本次写入必落库(unchanged=false),replace_one 生效。
    assert!(
        !applied.unchanged,
        "AI source 打回 draft+needs_review 改变了 status/integrity,不应 unchanged: {applied:?}"
    );

    // 落库后立即查 DB,断言 source=Ai 的写入把 chunk 打回 draft + needs_review。
    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_oid, "workspace_id": &ws }, None)
        .await
        .expect("查 chunk")
        .expect("chunk 应存在");

    assert_eq!(
        stored.status, "draft",
        "source=Ai 写入必须把 status 强制打回 draft(AI 永不自动 verify),实得 {:?}",
        stored.status
    );
    assert_eq!(
        stored.integrity_status.as_deref(),
        Some("needs_review"),
        "source=Ai 写入必须把 integrity_status 强制打回 needs_review,实得 {:?}",
        stored.integrity_status
    );
}

/// KB-09 红线:source=Ai 的 patch 对数组字段(product_tags)做 **existing ∪ patch**,
/// 既有 tag 不因 LLM"只列出这一项"而丢失;同时审计行落库 + status 打回 draft。
///
/// 此前 `apply_chunk_revision` 的数组 union 只有 `wiki_chunk_revision_pbt` 纯函数覆盖,
/// 没测真实落库的 union 既有源。KB-09 修复(chunk_revisions.rs:193-198)把数组既有源
/// 从被 patch clobber 过的 `after_patch` 改回**原始 existing_bson**,union 才不会退化成
/// `patch∪patch`(既有 tag 丢失)。本测试 seed 一条 `product_tags=["A"]` 的既有 chunk,
/// 用 source=Ai 的 patch 送 `product_tags=["B"]`,断言落库后是 union `{A,B}` 非替换 `[B]`。
/// 一旦 KB-09 修复被回退(数组既有源退回 after_patch),union 只剩 `[B]`,本测试立刻红。
#[tokio::test]
#[ignore]
async fn apply_chunk_revision_ai_patch_unions_product_tags_and_forces_draft() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();

    // seed:既有 chunk 带 product_tags=["A"],status=active(AI patch 应把它打回 draft)。
    // body 给足长度;本 patch 只改 product_tags(非 body/summary/answer),不触发 70% 截断闸。
    let chunk_oid = ObjectId::new();
    let seeded = OperationKnowledgeChunk {
        id: Some(chunk_oid),
        workspace_id: ws.clone(),
        domain: "user_operations".to_string(),
        title: "产品价格政策".to_string(),
        summary: Some("原始摘要:标准套餐与企业套餐价格概述。".to_string()),
        body: Some(
            "标准套餐月费 99 元,包含基础功能与 5 个坐席;企业套餐按坐席数阶梯计价,\
             含高级分析与专属客户成功经理。以上为产品价格政策正文,内容较长以确保稳定。"
                .to_string(),
        ),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        product_tags: vec!["A".to_string()],
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("insert seeded chunk");

    // 真调 apply_chunk_revision:source=Ai + op=Patch,patch 只送 product_tags=["B"]。
    // product_tags 在 DEFAULT_UNION_ARRAY_KEYS(page_merge.rs:58)→ 应 union 而非替换。
    apply_chunk_revision(
        &app.state.db,
        &ws,
        chunk_oid,
        RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Ai,
            patch: doc! { "product_tags": ["B"] },
            reason: Some("test: ai patch unions product_tags".to_string()),
            actor: None,
        },
    )
    .await
    .expect("apply_chunk_revision 应成功");

    // (1) 审计行落库:chunk_revisions 至少 1 行(chunk_id 存的是 hex 字符串)。
    let revision_count = app
        .state
        .db
        .chunk_revisions()
        .count_documents(doc! { "chunk_id": chunk_oid.to_hex() }, None)
        .await
        .expect("count chunk_revisions");
    assert!(
        revision_count >= 1,
        "apply_chunk_revision 必须写审计行(chunk_revisions),实得 {revision_count}"
    );

    // reload 落库结果。
    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_oid, "workspace_id": &ws }, None)
        .await
        .expect("查 chunk")
        .expect("chunk 应存在");

    // (2) product_tags 是 union {A,B} 非替换 [B](KB-09 治既有 tag 丢失)。
    // 用集合含判定,非顺序敏感(union_array_fields 用 BTreeSet 去重)。
    assert!(
        stored.product_tags.contains(&"A".to_string()),
        "既有 tag 'A' 必须保留(union 而非替换),实得 {:?}",
        stored.product_tags
    );
    assert!(
        stored.product_tags.contains(&"B".to_string()),
        "patch 新增 tag 'B' 必须写入,实得 {:?}",
        stored.product_tags
    );

    // (3) source=Ai 把 status 强制打回 draft(AI 永不自动 verify)。
    assert_eq!(
        stored.status, "draft",
        "source=Ai 写入必须把 status 强制打回 draft,实得 {:?}",
        stored.status
    );
}

/// S-08 regression: concurrent full-document replacements must not silently
/// overwrite a successful patch. Contenders that read a stale updated_at token
/// return Conflict, and their provisional revision rows are removed.
#[tokio::test]
#[ignore]
async fn concurrent_chunk_patches_conflict_without_lost_update_or_orphan_revision() {
    const CONTENDERS: usize = 16;

    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    let chunk_oid = ObjectId::new();
    let document_oid = ObjectId::new();
    let now = BsonDt::now();
    app.state
        .db
        .operation_knowledge_documents()
        .insert_one(
            OperationKnowledgeDocument {
                id: Some(document_oid),
                workspace_id: ws.clone(),
                account_id: None,
                domain: "user_operations".to_string(),
                source_type: "test".to_string(),
                source_name: None,
                title: "concurrent patch parent".to_string(),
                summary: None,
                catalog_summary: None,
                routing_map: Vec::new(),
                risk_notes: Vec::new(),
                product_tags: Vec::new(),
                business_topics: Vec::new(),
                raw_content: None,
                content_hash: None,
                line_index: Vec::new(),
                section_index: Vec::new(),
                status: "active".to_string(),
                version: 1,
                created_at: now,
                updated_at: now,
                catalog_summary_persisted: None,
                catalog_version: None,
                catalog_desired_generation: 0,
                catalog_applied_generation: 0,
            },
            None,
        )
        .await
        .expect("insert concurrent patch parent document");
    let seeded = OperationKnowledgeChunk {
        id: Some(chunk_oid),
        workspace_id: ws.clone(),
        document_id: Some(document_oid),
        domain: "user_operations".to_string(),
        title: "concurrent patch target".to_string(),
        summary: Some("baseline summary".to_string()),
        body: Some(
            "This body remains unchanged and is deliberately long enough that summary edits do not trigger the truncation guard."
                .to_string(),
        ),
        status: "active".to_string(),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("insert concurrent patch target");

    let barrier = Arc::new(Barrier::new(CONTENDERS));
    let mut handles = Vec::with_capacity(CONTENDERS);
    for index in 0..CONTENDERS {
        let db = app.state.db.clone();
        let ws = ws.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            let summary = format!("contender-{index}-summary");
            barrier.wait().await;
            let result = apply_chunk_revision(
                &db,
                &ws,
                chunk_oid,
                RevisionRequest {
                    op: RevisionOp::Patch,
                    source: ProvenanceSource::Human,
                    patch: doc! { "summary": &summary },
                    reason: Some("test: concurrent CAS".to_string()),
                    actor: Some(format!("admin-{index}")),
                },
            )
            .await;
            (summary, result)
        }));
    }

    let mut successful_summaries = HashSet::new();
    let mut conflicts = 0usize;
    for handle in handles {
        let (summary, result) = handle.await.expect("contender task should not panic");
        match result {
            Ok(applied) => {
                assert!(
                    !applied.unchanged,
                    "each unique summary must change the chunk"
                );
                successful_summaries.insert(summary);
            }
            Err(AppError::Conflict(code)) => {
                assert_eq!(code, "chunk_revision_conflict");
                conflicts += 1;
            }
            Err(other) => panic!("unexpected concurrent patch error: {other:?}"),
        }
    }

    assert!(
        !successful_summaries.is_empty(),
        "at least one patch must succeed"
    );
    assert!(
        conflicts > 0,
        "simultaneous contenders must expose stale-write conflicts"
    );

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_oid, "workspace_id": &ws }, None)
        .await
        .expect("reload concurrent patch target")
        .expect("concurrent patch target should exist");
    let stored_summary = stored.summary.expect("winner summary should be stored");
    assert!(
        successful_summaries.contains(&stored_summary),
        "final content must come from a successful writer, got {stored_summary:?}"
    );

    let revision_count = app
        .state
        .db
        .chunk_revisions()
        .count_documents(doc! { "chunk_id": chunk_oid.to_hex() }, None)
        .await
        .expect("count concurrent revisions");
    assert_eq!(
        revision_count,
        successful_summaries.len() as u64,
        "conflicted writes must not leave orphan revision rows"
    );
    let catalog_job_count = app
        .state
        .db
        .catalog_rebuild_jobs()
        .count_documents(doc! { "workspace_id": &ws }, None)
        .await
        .expect("count concurrent catalog jobs");
    assert_eq!(
        catalog_job_count,
        successful_summaries.len() as u64,
        "only committed writers may enqueue catalog rebuild jobs"
    );
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn chunk_replace_failure_rolls_back_revision_and_catalog_job() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    let chunk_oid = ObjectId::new();
    let seeded = OperationKnowledgeChunk {
        id: Some(chunk_oid),
        workspace_id: ws.clone(),
        document_id: Some(ObjectId::new()),
        domain: "user_operations".to_string(),
        title: "validator baseline".to_string(),
        body: Some("stable body for transaction rollback validation".to_string()),
        status: "active".to_string(),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("insert validator target");
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "operation_knowledge_chunks",
                "validator": { "title": { "$ne": "validator-rejected" } },
                "validationLevel": "strict",
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install chunk replacement validator");

    let result = apply_chunk_revision(
        &app.state.db,
        &ws,
        chunk_oid,
        RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Human,
            patch: doc! { "title": "validator-rejected" },
            reason: Some("force main-row write failure".to_string()),
            actor: Some("transaction-test".to_string()),
        },
    )
    .await;
    assert!(
        result.is_err(),
        "validator must reject the main-row replacement"
    );

    let stored = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": chunk_oid, "workspace_id": &ws }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title, "validator baseline");
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &ws, "chunk_id": chunk_oid.to_hex() },
                None
            )
            .await
            .unwrap(),
        0,
        "failed main-row write must roll back the provisional revision"
    );
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &ws }, None)
            .await
            .unwrap(),
        0,
        "failed main-row write must not enqueue catalog work"
    );
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn audit_only_noop_keeps_main_row_exact_and_skips_catalog_job() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    let chunk_oid = ObjectId::new();
    let seeded = OperationKnowledgeChunk {
        id: Some(chunk_oid),
        workspace_id: ws.clone(),
        document_id: Some(ObjectId::new()),
        domain: "user_operations".to_string(),
        title: "no-op target".to_string(),
        body: Some("content that must remain byte-for-byte stable".to_string()),
        status: "draft".to_string(),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    };
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&seeded, None)
        .await
        .expect("insert no-op target");
    let before = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("operation_knowledge_chunks")
        .find_one(doc! { "_id": chunk_oid }, None)
        .await
        .unwrap()
        .unwrap();

    let applied = apply_chunk_revision(
        &app.state.db,
        &ws,
        chunk_oid,
        RevisionRequest {
            op: RevisionOp::Patch,
            source: ProvenanceSource::Human,
            patch: doc! {},
            reason: Some("audit-only no-op".to_string()),
            actor: Some("transaction-test".to_string()),
        },
    )
    .await
    .expect("no-op revision should commit");
    assert!(applied.unchanged);

    let after = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("operation_knowledge_chunks")
        .find_one(doc! { "_id": chunk_oid }, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after, before,
        "audit-only no-op must not rewrite the main row"
    );
    let revision = app
        .state
        .db
        .chunk_revisions()
        .find_one(
            doc! { "workspace_id": &ws, "revision_id": &applied.revision_id },
            None,
        )
        .await
        .unwrap()
        .expect("no-op audit revision");
    assert_eq!(revision.before_snapshot, revision.after_snapshot);
    assert_eq!(
        app.state
            .db
            .catalog_rebuild_jobs()
            .count_documents(doc! { "workspace_id": &ws }, None)
            .await
            .unwrap(),
        0,
        "audit-only no-op must not rebuild catalog content"
    );
}
